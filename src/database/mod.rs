// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database (alloc/persistence/journal/snapshot)
//! Database operations on stores
#![allow(dead_code)]

mod allocation;
mod descriptor;
mod format;
mod io;
pub mod journal;
pub mod lazy;
mod search;
pub mod snapshot;
/// @PLN126 step 1 — does ordered insertion leave a finished record contiguous?
/// A measurement, not a gate: see the module header for how to run it.
#[cfg(test)]
mod spans;
pub mod sql_query;
pub mod sql_source;
mod structures;
mod types;

pub use allocation::{CLEAR_KEYED_VIEW, timeline_summary};
pub use descriptor::{BaseKind, Iterated, LayoutDesc, LayoutField, LayoutNode};
pub use journal::Journal;
pub use types::Type;

/// Store index reserved for compile-time constant data (vectors, long strings).
/// Always allocated during `State::new()`, locked before execution begins.
/// See `doc/claude/CONST_STORE.md` for the full design.
pub const CONST_STORE: u16 = 1;

use crate::keys::{Content, DbRef};
use crate::store::{Store, StoreChange};
use std::collections::HashMap;
use std::fmt::{Debug, Formatter, Write as _};
use std::sync::{Arc, Mutex};

// the `--html` build compiles for wasm32-unknown-unknown
// WITHOUT the `wasm` feature (the feature carries wasm-bindgen
// host bridges that `--html`'s hand-rolled JS runtime does not
// provide).  That leaves `std::time::Instant` on a target with no
// time source — calling `Instant::now()` panics, and the panic
// compiles to `(unreachable)` which was the root of every
// `--html loft_start` trap.  Use Instant only on non-wasm32
// targets; wasm32 (with or without the feature) tracks time in
// milliseconds through the host bridge.
#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
use std::time::Instant;

/// Type alias for a native function callable from loft bytecode.
pub type Call = fn(&mut Stores, &mut DbRef);

/// Context injected into `Stores` by `State::execute()` so that native
/// functions such as `n_parallel_for` / `n_parallel_for_light` can access
/// the interpreter's bytecode, text segment, library, and compiled data
/// for spawning workers.
///
/// All raw pointers are valid for the duration of the `execute()` call
/// that set them.
///
/// A worker inherits its parent's context (`clone_for_light_worker_with_scratch`)
/// so that a `par` inside a `par` worker can dispatch in turn — the pointers stay
/// valid for a worker exactly as they do for the thread that set them, because
/// workers are joined before `execute()` returns.
#[derive(Clone, Copy)]
pub struct ParallelCtx {
    pub bytecode: *const Arc<Vec<u8>>,
    pub library: *const Arc<Vec<Call>>,
    pub data: *const crate::data::Data,
    /// Cached library index of `n_stack_trace`; `u16::MAX` = not found.
    /// Copied into worker `State::stack_trace_lib_nr` so workers can snapshot
    /// the call stack when `stack_trace()` is called (fix #92).
    pub stack_trace_lib_nr: u16,
}

// Safety: the pointed-to data lives for the duration of `State::execute()`,
// which is on the main thread and outlives all worker threads it spawns
// (workers are joined before execute() returns).
unsafe impl Send for ParallelCtx {}
unsafe impl Sync for ParallelCtx {}

/// TR1.4: snapshot of one local variable's runtime value, captured by
/// `State::static_call` for inclusion in a `StackFrame.variables` vector.
/// All fields are owned values — no raw pointers — so the snapshot is safe
/// to retain across native function boundaries.
#[derive(Debug, Clone)]
pub struct VarSnapshot {
    pub name: String,
    pub type_name: String,
    pub value: VarValueSnapshot,
}

/// Owned snapshot of a variable's typed runtime value.  Mirrors the loft
/// `ArgValue` enum so the native can populate `VarInfo.value` directly.
#[derive(Debug, Clone)]
pub enum VarValueSnapshot {
    Null,
    Bool(bool),
    Int(i32),
    Long(i64),
    Float(f64),
    Single(f32),
    Char(char),
    Text(String),
    Ref { store: i32, rec: i32, pos: i32 },
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    /// Known-type number of the field's value type — needed by
    /// runtime struct-schema walkers (e.g. `n_struct_from_jsonvalue`)
    /// that iterate `Parts::Struct(_)`.
    pub content: u16,
    pub position: u16,
    /// loft#876 — the field's DECLARED default (`height: float = 1.5`), folded to a
    /// constant, or `None` when the field declares none.
    ///
    /// Only a default that folds to a literal lands here. `= mk()` / `= [1, 2]` and
    /// anything else needing a temporary is lowered parser-side into a function the
    /// CONSTRUCTION site calls, and the store layer has no evaluator to run it — so
    /// those keep applying where the constructor runs, and nowhere else. That split is
    /// the contract, not an omission ([`crate::typedef::fold_declared_default`]).
    ///
    /// Carried, never RENDERED: a default changes no width and no offset, so
    /// `layout_dump` / `LayoutDesc::render_dump` must not see it or adding `= 1.5` to a
    /// field would change the @PLN97 layout identity and refuse an existing store.
    /// Same reasoning as `nullable` below.
    ///
    /// Serialized as its `Content` with `None` written as `Content::Str("")` — the
    /// value every registration site wrote before this field carried anything, so the
    /// snapshot and IR-store formats stay byte-identical for a field with no default.
    pub default: Option<Content>,
    /// @PLN127 arc D — was this field DECLARED nullable?
    ///
    /// Not derivable from anything else here: a narrow scalar registers a
    /// distinct content type per nullability, but `text?` and `integer?` share
    /// their non-null type and spell an absent value with a SENTINEL. So the
    /// fact reaches the store only because the parser deposits it, and it is
    /// carried rather than RENDERED — `layout_dump` and `LayoutDesc::render_dump`
    /// are untouched, so the @PLN97 layout identity is too.
    pub nullable: bool,
    pub(self) other_indexes: Vec<u16>, // For now only fields on the same record
}

impl Field {
    /// @PLN11 D2a read seam — `other_indexes` is module-private; expose it
    /// `pub(crate)` so the schema materializer (`crate::ir_store`) can cache it.
    #[must_use]
    pub(crate) fn other_indexes(&self) -> &[u16] {
        &self.other_indexes
    }

    /// @PLN11 D2a — reconstruct a `Field` from cached store fields (the
    /// `pub(self)` `other_indexes` makes a direct literal impossible outside
    /// this module).
    /// The wire spelling of a declared default: `None` is written as the empty string,
    /// which is what every registration site wrote before defaults were carried, so an
    /// existing snapshot / IR store round-trips byte-identically.  A field declaring
    /// `= ""` collapses onto the same spelling, and reads back as "no declared
    /// default" — harmless, because the absent value of a non-null `text` IS the
    /// interned empty string (loft#875), so both routes answer identically.
    #[must_use]
    pub(crate) fn default_to_wire(default: Option<&Content>) -> Content {
        match default {
            Some(c) => c.clone(),
            None => Content::Str(crate::keys::Str::new("")),
        }
    }

    /// Inverse of [`Self::default_to_wire`].
    #[must_use]
    pub(crate) fn default_from_wire(c: Content) -> Option<Content> {
        match &c {
            Content::Str(s) if s.len == 0 => None,
            _ => Some(c),
        }
    }

    #[must_use]
    pub(crate) fn from_stored(
        name: String,
        content: u16,
        position: u16,
        default: Content,
        nullable: bool,
        other_indexes: Vec<u16>,
    ) -> Field {
        Field {
            name,
            content,
            position,
            default: Self::default_from_wire(default),
            nullable,
            other_indexes,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Parts {
    Base,                              // One of the simple base types or text.
    Struct(Vec<Field>),                // The fields of this record.
    Enum(Vec<(u16, String)>),          // Enumerate type with possible values.
    EnumValue(u8, Vec<Field>),         // Enumerate value with actual value for typed structures.
    Byte(i32, bool),                   // start number and nullable flag
    Short(i32, bool),                  // start number and nullable flag
    Int(i32, bool), // 4-byte integer field (size(4) annotation). Null sentinel: i32::MIN.
    ShortRaw(i32, bool), // P184 Phase 4b: 2-byte narrow vector element. Direct encoding (no +1 shift). Null sentinel: i16::MIN.
    Vector(u16),         // The records are part of the vector
    Array(u16),          // The array holds references for each record
    Sorted(u16, Vec<(u16, bool)>), // Sorted vector on fields with an ascending flag
    Ordered(u16, Vec<(u16, bool)>), // Sorted array on fields with an ascending flag
    Hash(u16, Vec<u16>), // A hash table, listing the field numbers that define its key
    Index(u16, Vec<(u16, bool)>, u16), // An index to a table, listing the key fields and the left field-nr
    Radix(u16, Vec<u16>),              // A spatial index with the listed coordinate fields as a key
    // A trie: a radix tree over ONE `text` key field, answering exact lookup, key
    // order and prefix.  Shares `radix_tree` with `Radix` and nothing above it —
    // `Radix` is geometric (Morton interleave, boxes, near/within/nearest) and none
    // of that means anything for a word, which is why `spatial` is not called `radix`
    // at the surface.  See doc/claude/plans/text-keyed-trie.md.
    //
    // ONE key, held as a `u16` rather than a `Vec<u16>`: a trie over two text fields
    // is not a thing, and encoding that in the type means no site has to check it.
    Trie(u16, u16),
    // Plan-06 phase 4d.C step 2: 12-byte stored DbRef pointer (store_nr
    // u16 padded to u32 + rec u32 + pos u32).  Distinct from `Vector`
    // / `Hash` / etc. which all store a 4-byte rec pointer; this
    // variant preserves the FULL DbRef so the closure half of a
    // fn-ref struct field can round-trip through storage.
    // No element type — DbRef bytes are opaque at this layer.
    DbRef,
    // P213: 4-byte u32 rec-id pointing at a child record (of type
    // `content`) co-located in the SAME Store as the host record.
    // Lifetime is exactly the host's: `copy_claims` / `remove_claims`
    // cascade automatically claim/free the child via the cascade arms
    // for this variant.  Distinct from `Vector(c)` (which holds N
    // elements with a length-prefixed chunk) and `DbRef` (12B opaque
    // pointer with no cascade).  Used by capturing-closure-in-struct-
    // field codegen to co-locate the closure record in host's Store.
    ChildRec(u16),
}

impl PartialEq for Content {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Content::Long(l), Content::Long(r)) => l == r,
            (Content::Float(l), Content::Float(r)) => l == r,
            (Content::Single(l), Content::Single(r)) => l == r,
            (Content::Str(s), Content::Str(o)) => s.str() == o.str(),
            _ => false,
        }
    }
}

impl Debug for Content {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Content::Long(l) => f.write_fmt(format_args!("Long({l})"))?,
            Content::Float(v) => f.write_fmt(format_args!("Float({v})"))?,
            Content::Single(s) => f.write_fmt(format_args!("Single({s})"))?,
            Content::Str(t) => {
                f.write_char('"')?;
                f.write_str(t.str())?;
                f.write_char('"')?;
            }
        }
        Ok(())
    }
}

#[allow(clippy::struct_excessive_bools)]
/// @PLN129 arc D — a collection's lazy source, plus the identity it was PINNED
/// to at bind time.
///
/// A store is a consistent image; a live source is not. Two faults in one
/// traversal reading different worlds breaks that silently — measured: swapping
/// the file between two lookups returned `grace` then `ALAN-v2`, with nothing
/// reporting it.
///
/// A local file cannot be snapshotted cheaply (copying it defeats the point), so
/// the pin is its identity — length and modification time — and drift is
/// DETECTED rather than prevented. Detection is the honest half: the traversal
/// either sees one world or is told it did not, which is the same contract arc C
/// set for unreachability. An `http(s)` source carries no pin (there is nothing
/// cheap to stat), and a database source will pin a transaction instead, which is
/// the one case where consistency can actually be provided rather than checked.
#[derive(Clone, Debug)]
pub struct LazyBinding {
    /// Where to fetch from — a local path or an `http(s)://` URL.
    pub source: String,
    /// `(len, mtime_nanos, inode)` for a local file at bind time; `None` when the
    /// source cannot be stat'ed cheaply.
    ///
    /// All three, because each alone is too coarse and that was MEASURED, not
    /// guessed: the first version pinned `(len, mtime_secs)` and failed to notice
    /// a swap between two images that were both 8192 bytes and written in the
    /// same second. Nanoseconds catch an in-place rewrite; the inode catches a
    /// rename-replace, which does not touch the new file's mtime at all.
    pub pin: Option<(u64, u64, u64)>,
    /// @PLN129 arc B step 7 — whether this source's schema can serve this
    /// collection, decided once.
    pub check: SchemaCheck,
}

/// @PLN129 arc B step 7 — the verdict on a database schema loft does not own.
///
/// Decided on the FIRST fetch rather than at bind, because that is the first
/// moment the collection's TYPE is known — a bind takes a reference, and a
/// reference does not carry one. Still before any answer a program could
/// believe, which is the property the check exists for.
///
/// Re-decided by a rebind: a rebind is a caller saying "this is a different
/// world now", and the schema is part of that world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaCheck {
    Unchecked,
    Ok,
    /// The bind cannot be served, and this says why. Sticky for the life of the
    /// binding: nothing about the schema changes between two lookups, so
    /// re-asking would cost a round trip per fault to get the same answer.
    Refused(String),
}

/// The runtime-type-name prefix reserved for a generic's TYPE VARIABLE.
///
/// A `<T>` type parameter needs a row in the name-keyed type table so a template body
/// can be parsed at all, and that row is registered under this prefix rather than under
/// `T` — a name a user type may legitimately carry, and did: a user `enum T` reused the
/// marker's size-0 entry and divided by zero (`typedef::fill_database`).
///
/// It is a constant rather than a spelling because two sites depend on it agreeing: the
/// one that MINTS the row (`typedef::fill_database`) and the one that REFUSES to allocate
/// a record with it (`Stores::enum_parent_size`, loft#1070). A prefix that drifted between
/// them would leave the guard silently matching nothing.
pub const TYPEVAR_ROW_PREFIX: &str = "__typevar_";

#[allow(clippy::struct_excessive_bools)]
pub struct Stores {
    pub types: Vec<Type>,
    pub names: HashMap<String, u16>,
    pub allocations: Vec<Store>,
    /// #306 — true when slot 0 holds the interpreter's eval-stack store
    /// (set by `State::new`).  `free_named` then refuses a whole-store free
    /// of slot 0: such a ref is always a stack-allocated record
    /// (`OpCreateStack`) wrongly treated as an owned heap store, and the
    /// free would destroy every live frame.  Bare `Stores` (unit tests,
    /// tooling) and the NATIVE runtime keep `false` — there slot 0 is an
    /// ordinary heap store that must stay freeable and leak-checkable
    /// (#490/#491).  Always test via [`Stores::is_stack_store`], never a
    /// bare `store_nr == 0`.
    pub stack_store_at_zero: bool,
    /// @PLN129 arc A — a collection bound to a LAZY source, keyed by the
    /// collection's root `(store_nr, rec, pos)`.
    ///
    /// Per COLLECTION, not per store and not per type: `persons` and `companies`
    /// are different tables, and two collections of one type can be bound
    /// differently. A runtime-only field — configuration, not data — so `clone`
    /// resets it alongside `allocations`, and it never reaches a store image.
    ///
    /// Empty for every program that binds nothing, which is the common case: a
    /// miss consults this map only after the ordinary lookup has already failed,
    /// so an unbound collection pays one hash probe on a path that was about to
    /// return "absent" anyway.
    pub lazy_sources: HashMap<(u16, u32, u32), LazyBinding>,
    /// @PLN129 arc C — why a fetch for this collection could not reach its
    /// source. Absent means healthy.
    ///
    /// A lookup cannot report this: C80 says a value read never raises, and
    /// answering `null` would make "no such person" and "the database is
    /// unreachable" the same answer — and an UNSTABLE one, since it changes with
    /// the network. So the failure lives on a channel the value cannot carry,
    /// asked deliberately (`store_lazy_error`), the way `#errors` and
    /// `store_verify` already work.
    ///
    /// STICKY, and cleared only by `store_lazy_clear`. A later success must NOT
    /// clear it: a traversal whose first lookup could not reach the source and
    /// whose second could is MISSING data, and reporting "healthy" afterwards is
    /// the silent-wrong-answer this channel exists to prevent. Reachability now
    /// says nothing about what an earlier failure already lost — which is why an
    /// absence does not clear it either.
    ///
    /// `(count, first reason)`: the count answers "how incomplete am I", and the
    /// FIRST reason is kept because it names the original cause; later ones are
    /// usually the same failure repeating.
    pub lazy_errors: HashMap<(u16, u32, u32), (u64, String)>,
    /// The reason the paged loader last REFUSED, or `None` when it merely
    /// missed. Set by [`Stores::refuse_paged`], read and cleared by the lazy
    /// fetch (loft#802).
    ///
    /// The loaders answer `false` / `0` for both outcomes, and those are
    /// different facts: a key that is absent is a stable truth about the data, a
    /// refused shape is a permanent truth about the BINDING. Without this the
    /// lazy fetch had only the loader's `false` to go on and reported the
    /// refusal as an absence — so `store_lazy_error` said `""`, whose documented
    /// meaning is "reachable, genuinely no such key". A trie bound to a paged
    /// source then looked healthy and answered null forever.
    pub(crate) paged_refusal: Option<String>,
    /// @PLN133 S8 — the stores created while a lazy DRIVER is running, or
    /// `None` when none is.
    ///
    /// A raise short-circuits the dispatch loop, so the scope-exit frees the
    /// compiler emitted never run. For a program about to exit that is
    /// harmless; for a CONTAINED fault the program continues, and this is what
    /// the containment frees instead of the teardown it could not run.
    pub(crate) lazy_driver_allocs: Option<Vec<u16>>,
    #[cfg(not(feature = "wasm"))]
    pub files: Vec<Option<std::fs::File>>,
    #[cfg(feature = "wasm")]
    pub files: Vec<()>,
    pub max: u16,
    /// Monotonic high-water mark of `max` over the whole run — the **store
    /// watermark** plan-57 tracks (the peak the `LOFT_STORES=log` trace prints
    /// as `max=`).  Unlike `max`, which shrinks when a top slot frees
    /// (`allocation.rs` free path), `peak` only ever grows, so it survives to
    /// the end of execution as a single readable number.  This is what lets a
    /// Rust test assert "confined block-locals free at block exit" without
    /// shelling out and parsing the stderr trace (which has a stdout/stderr
    /// buffering hazard).  Reset to 0 on a fresh runtime.
    pub peak: u16,
    /// @PLN101 Slice 0 — monotonic count of `record_new` events (a logical record: struct
    /// value, nested-struct field, collection entry). NOTE (corrected 2026-07-08): this is a
    /// COARSE proxy — it counts logical records incl. already-inline ones, and MISSED the real
    /// cost (per-construction scratch stores) entirely. Use `stores_allocated` as the true
    /// heap metric. Kept for continuity / logical-record accounting.
    pub records_created: u64,
    /// @PLN101 Slice 0 (the TRUE heap metric) — monotonic count of store-SLOT allocations
    /// (every time a `Store` goes live). A reference struct's real cost is the per-construction
    /// SCRATCH store: a local struct loop x100 → ~102 store allocs; a `vector<P>` inlines its
    /// elements → ~4 (near the scalar baseline). A `value struct` built in place allocates
    /// none. `LOFT_ALLOC_REPORT=1` prints it; the pub field is read in-process for assertions.
    pub stores_allocated: u64,
    /// @PLN105 leak provenance — the bytecode position of the currently-executing op,
    /// republished per-op by the interpreter's dispatch loop. `database_named` stamps it
    /// into each freshly-allocated store's `created_at` so a leaked store's allocation
    /// site (`LOFT_LEAK_SITES` → source line) is populated for EVERY allocation path, not
    /// just `OpDatabase` (which stamps `code_pos` directly in `alloc_record_at`). Zero
    /// outside interpretation (native/tooling), which resolves to "line 0" harmlessly.
    pub alloc_pc: u32,
    /// S29: bitmap of free store slots — bit `i` is set when `allocations[i]`
    /// is free and eligible for reuse.  `database_named` finds the lowest set bit below `max`
    /// and reuses that slot instead of always growing `max`.  This eliminates the LIFO-order
    /// requirement on `free()` that the old cascade-based scan imposed.
    pub free_bits: Vec<u64>,
    /// @PLN10 N2b — destination for the NEXT cdylib FFI text return.  Set by
    /// `n_set_bridge_dest` (emitted right before a dest-passed cdylib text call)
    /// and `take()`n by the bridge's text path (`bridge_push_str` /
    /// `push_loft_str`): when `Some`, the foreign `LoftStr` bytes are written
    /// directly into that store record, and the
    /// bridge pushes nothing (the record IS the result).  Transient — lives only
    /// across the two adjacent `OpStaticCall`s.
    pub bridge_text_dest: Option<crate::keys::DbRef>,
    /// per-definition DbRef into the CONST_STORE for vector
    /// constants (e.g. `pub HEIGHT_STEP_LABELS: vector<text> = […]`).
    /// Indexed by `d_nr`; a null DbRef (store_nr = u16::MAX) means
    /// that definition isn't a constant.  Populated by
    /// `compile::build_const_vectors` (interpreter path) or by the
    /// `init()` function emitted by `src/generation/` (native path).
    /// Mirrors `State.const_refs` so native code's
    /// `stores.const_ref_at_runtime(d_nr)` accessor (called via the
    /// substituted `OpConstRef` template — see
    /// `src/generation/calls.rs`) resolves from any function context
    /// that only has `&mut Stores`.
    pub const_refs: Vec<DbRef>,
    /// Errors from the last `Type.parse()` call, read via `s#errors`.
    pub last_parse_errors: Vec<String>,
    /// errors from the last `json_parse()` call, read via
    /// `json_errors()`.  Cleared on every successful `json_parse`;
    /// populated with `format!("{msg} (byte {pos})")` on parse failure.
    pub last_json_errors: Vec<String>,
    /// Set by `State::execute()` to allow native functions to access the
    /// interpreter's bytecode, library, and compiled data during execution.
    pub parallel_ctx: Option<Box<ParallelCtx>>,
    /// Plan-06 PRIORITY.md spine step 8a — stack of per-call result
    /// buffers populated by `n_parallel_queue` and consumed by
    /// `n_parallel_buf_get` / drained by `n_parallel_buf_drop`.  Each
    /// `Vec<u64>` holds one row's u64-encoded worker return value per
    /// element (in input order).  A stack — not a single buffer — is
    /// needed for nested fused for-par: an inner `par()` inside an
    /// outer par body pushes its own buffer; the outer keeps its
    /// buffer underneath.
    ///
    /// Step 8b is the first parser-side consumer; until then, the
    /// only writer is the `n_parallel_queue` native fn (exercised by
    /// Rust unit tests in `tests/threading.rs`).
    pub par_buffer_stack: Vec<Vec<u64>>,
    /// Plan-06 PRIORITY.md spine step 8c — sibling stack for text-
    /// returning par workers.  `n_parallel_queue_text` populates;
    /// `n_parallel_buf_get_text` reads (cloning into `scratch` for
    /// the standard text-return convention); `n_parallel_buf_drop_text`
    /// pops.  A separate stack from `par_buffer_stack` because text
    /// results are owned `String`s, not u64-encoded primitives —
    /// keeping the per-row read path tight (no enum match per
    /// element) is worth the duplication.
    pub par_text_buffer_stack: Vec<Vec<String>>,
    /// Plan-06 PRIORITY.md spine step 8d.1 — sibling stack for
    /// reference / struct-enum-payload / vector-returning par
    /// workers.  Each entry is `(refs, adopted_store_nrs)`:
    /// - `refs` — rebased `DbRef`s in input-row order, valid in the
    ///   parent's namespace after `Stores::adopt_worker_excess` +
    ///   `rebase_walk_record` (8d.0's `run_parallel_queue_ref`).
    /// - `adopted_store_nrs` — parent-side store_nrs that the queue
    ///   adopted from worker output stores.  `n_parallel_buf_drop_ref`
    ///   frees these at the body-tail to release the worker memory.
    ///
    /// Separate from `par_buffer_stack` and `par_text_buffer_stack`
    /// because ref returns own additional state (the adopted-store
    /// list) — keeping the per-row read path tight (no enum match
    /// per element) is worth the duplication.
    pub par_ref_buffer_stack: Vec<(Vec<DbRef>, Vec<u16>)>,
    /// Plan-06 ARC.md A3 — sibling stack for narrow-primitive
    /// (1, 2, or 4-byte) returning par workers.  `n_parallel_queue_narrow`
    /// populates a flat `Vec<u8>` of `n_rows * stride` bytes (little
    /// endian, packed); `n_parallel_buf_get_narrow` reads one row,
    /// sign-extending if signed.  `n_parallel_buf_drop_narrow` pops.
    /// 8-byte ints + Float keep using `par_buffer_stack` (their bit
    /// pattern fits in `u64`).
    ///
    /// Each entry is `(bytes, stride)`: stride is 1, 2, or 4.
    pub par_narrow_buffer_stack: Vec<(Vec<u8>, u8)>,
    /// Plan-06 ARC.md A6.b — sibling stack for fn-ref-returning par
    /// workers.  Each entry is a packed `Vec<u8>` of `n_rows * 20`
    /// bytes — one fn-ref per row in Rust's reordered `DbRef` layout
    /// (8B i64 d_nr + 12B closure DbRef where DbRef is rec u32 +
    /// pos u32 + store_nr u16 + 2B padding).  Workers write each
    /// row directly via `State::execute_at_raw_to`; readers pull
    /// 20 bytes via `n_parallel_buf_get_fn` and push them onto the
    /// operand stack as a fn-ref blob.
    ///
    /// Separate from `par_buffer_stack` (8-byte rows),
    /// `par_text_buffer_stack` (Vec<String>),
    /// `par_ref_buffer_stack` ((Vec<DbRef>, Vec<u16>)), and
    /// `par_narrow_buffer_stack` ((Vec<u8>, u8)) — fn-ref returns
    /// have a fixed 20-byte stride so no per-call width field is
    /// needed.
    pub par_fn_buffer_stack: Vec<Vec<u8>>,
    /// Native fn-ref-return Queue buffer — each row is the worker's native
    /// fn-ref value `(u32 d_nr, DbRef closure)`.  The native packer
    /// (`n_parallel_queue_fn_native`) and reader (`n_parallel_buf_get_fn_native`)
    /// agree on this typed shape, so the native path needs no 20-byte byte
    /// serialization (distinct from the interpreter's `par_fn_buffer_stack`).
    pub par_fn_native_buffer_stack: Vec<Vec<(u32, DbRef)>>,
    /// Shared runtime logger.  Set by `main.rs` after the State is created.
    /// Cloned (Arc clone) into worker Stores so all threads share a single logger.
    pub logger: Option<Arc<Mutex<crate::logger::Logger>>>,
    /// Set to `true` when a loft `panic()` or failed `assert` fires in production mode
    /// (where the error is logged instead of aborting).  `main.rs` checks this after
    /// execution and exits with code 1 so shell scripts can detect failure.
    pub had_fatal: bool,
    /// Plan-07 phase 4 — typed runtime error captured by the most recent
    /// fault-site opcode or native fn.  Set by callers via
    /// [`crate::runtime_error::RuntimeError`] constructors plus
    /// `had_fatal = true`; the interpreter dispatch loop in
    /// `src/state/mod.rs::execute_argv` checks `runtime_error.is_some()`
    /// after each op and breaks out of execution by setting
    /// `code_pos = u32::MAX`.  `main.rs` then renders the error through
    /// the phase-2 pretty renderer.  Boxed because the slot is rarely
    /// populated and the `RuntimeError` payload (kind enum + Position
    /// String + message String) is otherwise ~96 bytes per `Stores`.
    pub runtime_error: Option<Box<crate::runtime_error::RuntimeError>>,
    /// Directory of the main source file being executed.
    /// Set by `main.rs` after parsing; used by `source_dir()` built-in.
    pub source_dir: String,
    /// #255 / @PLN9: when true, a *relative* program-supplied file path
    /// re-homes against `source_dir` (the program's own directory) instead of
    /// the process cwd.  Absolute paths are never touched.  Parse-time config:
    /// the default (set in `Stores::new`), overridable per-program by the
    /// `#cwd` directive and per-invocation by the `LOFT_PATHS` env var.  Read by
    /// `resolve_path`, the single home every file-op site routes through.
    pub program_relative: bool,
    /// FY.1: When true, the interpreter loop yields back to the caller.
    /// Set by `gl_swap_buffers` in WASM mode; cleared by `resume_frame`.
    pub frame_yield: bool,
    /// When true, `free_named` overwrites the freed store's buffer with a
    /// poison pattern (`0xDEADBEEF` i32 words) so subsequent reads through a
    /// stale DbRef hit recognisable garbage instead of whatever bytes the
    /// allocator leaves.  Enabled by `LOFT_LOG=poison_free` via
    /// `execute_log_impl` (or anywhere else that wires it).
    pub poison_free: bool,
    /// Plan-06 spine 8d.2: when true, `database_named`'s slot allocator
    /// skips `find_free_slot` and always pushes a new store at the end
    /// of `allocations`.  Set on workers spawned by
    /// `run_parallel_queue_ref` so each worker-created Result record
    /// lands at an index ≥ `parent_store_count`, where
    /// `adopt_worker_excess` can move it into the parent's namespace.
    /// Without this flag, workers reuse their own freed slots (S29) —
    /// each `transform()` call's local Result store gets freed at fn
    /// return and the next call picks the same slot, so the per-row
    /// `DbRef`s collected in the batch end up aliasing the LAST call's
    /// data.  Other dispatch paths (run_parallel_direct / _text / _int
    /// / _discard / the legacy run_parallel_ref consumed by
    /// `parallel_execute_and_collect`) leave this `false` because they
    /// don't rely on adoption — `copy_from_worker` reads through the
    /// graft swap regardless of slot reuse.
    pub disable_slot_reuse: bool,
    /// `LOFT_UAF` use-after-free detector: slots freed by the CURRENT op
    /// (`free_named` pushes; the dispatch loop drains + scans after the op).
    /// Always empty when the detector is off.  See `keys::uaf_check_enabled`.
    pub uaf_freed_this_op: Vec<u16>,
    /// Plan-06 ARC.md A2 — shared atomic dispenser of parent-namespace
    /// slot indices for `run_parallel_queue_ref` workers.  When
    /// `disable_slot_reuse == true` AND this is `Some`, every
    /// `database_named` call in worker context dispenses a unique index
    /// from the shared atomic counter, extends the worker's own
    /// `allocations` vec to that index, and records the index in
    /// `worker_allocated_indices`.  After thread join the dispatcher
    /// grows parent's `allocations` to fit and swaps each recorded
    /// slot back into parent at the recorded index.
    ///
    /// Replaces the 8d.3 fixed-range design (`worker_slot_offset` /
    /// `worker_slot_limit` + 16-slot cap) which panicked on workers
    /// that allocated more than `SLOTS_PER_THREAD` stores.  Per-thread
    /// indices remain disjoint because the atomic dispenses globally
    /// unique values; growth is unbounded.
    pub worker_slot_dispenser: Option<Arc<std::sync::atomic::AtomicU16>>,
    /// ARC.md A2 — per-worker list of parent-namespace indices the
    /// worker allocated via the shared dispenser.  The dispatcher
    /// iterates this Vec to swap each worker-owned slot back into
    /// parent at the recorded index after thread join (replacing
    /// 8d.3's `[off, off+SLOTS_PER_THREAD)` linear scan).
    pub worker_allocated_indices: Vec<u16>,
    /// When true, assert() reports results (pass/fail) to `assert_results`
    /// instead of panicking on failure.  Used by the WASM playground.
    pub report_asserts: bool,
    /// Structured assert results: (passed, message, file, line).
    pub assert_results: Vec<(bool, String, String, u32)>,
    /// Script-level arguments (set by the CLI after parsing its own flags).
    /// When non-empty, `os_arguments()` returns these instead of raw `std::env::args`.
    pub user_args: Vec<String>,
    /// Monotonic timestamp captured at `Stores::new()`.  Used by `ticks()` to return
    /// microseconds elapsed since program start; cloned into worker Stores unchanged so
    /// all threads share the same reference point.
    #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
    pub start_time: Instant,
    /// Under any wasm32 target (both the `wasm` feature's host-bridge
    /// build and the `--html` no-feature build): milliseconds since
    /// Unix epoch at program start.  `n_ticks` uses this plus the
    /// host-imported `time_ticks` to compute elapsed time without
    /// `std::time::Instant`.  Instant is unavailable on wasm32 (for
    /// either feature variant), so we snapshot elapsed ms here.
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
    pub start_time_ms: i64,
    /// TR1.3: snapshot of (`fn_name`, file, line) for each call frame.
    /// Populated by `State::static_call` when `n_stack_trace` is invoked.
    pub call_stack_snapshot: Vec<(String, String, u32)>,
    /// TR1.4: per-frame variable snapshot.  Outer Vec is parallel to
    /// `call_stack_snapshot` (one entry per frame); inner Vec is the live
    /// variables in that frame as `(name, type_name, ArgValueSnapshot)`.
    /// Populated alongside `call_stack_snapshot` in `State::static_call`.
    pub variables_snapshot: Vec<Vec<VarSnapshot>>,
    /// Native-code closure store. Maps lambda d_nr → closure DbRef.
    /// Set by `OpStoreClosure` (native) immediately before calling the lambda;
    /// read by `OpGetClosure` in the match-dispatch arm.
    pub closure_map: HashMap<u32, DbRef>,
    /// Shared `JsonValue::JNull` sentinel record for `n_field` / `n_item`
    /// fallback paths.  Lazily allocated on first use (after JsonValue's
    /// `known_type` has been registered), kept for the process lifetime —
    /// its containing store is flagged `free = false` so `check_store_leaks`
    /// ignores it.
    pub jnull_sentinel: Option<DbRef>,
}

impl Default for Stores {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for Stores {
    /// Clone the type-schema portion of a `Stores`.
    /// Runtime-only fields (`allocations`, `files`, `parallel_ctx`)
    /// are reset to empty/None because they are only valid during execution.
    fn clone(&self) -> Self {
        Self {
            types: self.types.clone(),
            names: self.names.clone(),
            allocations: Vec::new(),
            stack_store_at_zero: self.stack_store_at_zero,
            lazy_sources: HashMap::new(),
            lazy_errors: HashMap::new(),
            paged_refusal: None,
            lazy_driver_allocs: None,
            files: Vec::new(),
            max: self.max,
            peak: 0,
            records_created: 0,
            stores_allocated: 0,
            alloc_pc: 0,
            free_bits: Vec::new(),
            bridge_text_dest: None,
            const_refs: Vec::new(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            parallel_ctx: None,
            par_buffer_stack: Vec::new(),
            par_text_buffer_stack: Vec::new(),
            par_ref_buffer_stack: Vec::new(),
            par_narrow_buffer_stack: Vec::new(),
            par_fn_buffer_stack: Vec::new(),
            par_fn_native_buffer_stack: Vec::new(),
            logger: self.logger.clone(),
            had_fatal: false,
            runtime_error: None,
            // #255: `source_dir` is parse-time CONFIG (the main source file's
            // directory), not runtime state — it must survive `clone()` so the
            // `source_dir()` builtin works after the test runner / native paths
            // rebuild a fresh `State` from a cloned schema db.  Resetting it to
            // empty here left source-relative asset resolution broken in those
            // contexts.
            source_dir: self.source_dir.clone(),
            // #255: parse-time config like source_dir — preserve across clone so
            // the path mode survives the per-test-function State rebuild.
            program_relative: self.program_relative,
            frame_yield: false,
            poison_free: self.poison_free,
            disable_slot_reuse: self.disable_slot_reuse,
            uaf_freed_this_op: Vec::new(),
            worker_slot_dispenser: None,
            worker_allocated_indices: Vec::new(),
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: self.user_args.clone(),
            #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
            start_time: self.start_time,
            #[cfg(all(target_arch = "wasm32", not(target_os = "wasi")))]
            start_time_ms: self.start_time_ms,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: HashMap::new(),
            jnull_sentinel: None,
        }
    }
}

// Safety: `Content::Str` raw pointers in type metadata point into parse-time
// source strings that live for the program duration and are never mutated.
// Workers only read this metadata.  `Store` is already `unsafe impl Send`.
// `Sync` is additionally required so that `OnceLock<(Data, Stores)>` can be
// used as a process-wide static; the same invariant (read-only after parse)
// makes concurrent shared access safe.
unsafe impl Send for Stores {}
unsafe impl Sync for Stores {}

/// Type-level proof that a [`Stores`] was produced by
/// [`Stores::clone_for_light_worker`] and belongs to exactly one worker thread.
///
/// `WorkerStores` is `Send` (movable to a worker thread) but intentionally not
/// `Sync` (cannot be shared across threads).  The `PhantomData<*mut ()>` field
/// suppresses the auto-derived `Sync` implementation; the explicit `Send`
/// implementation restores send-ability.  This ensures that passing a worker
/// snapshot to `State::new_worker` at the call site is a compile-time guarantee
/// rather than a runtime convention.
pub struct WorkerStores {
    pub(crate) stores: Stores,
    _not_sync: std::marker::PhantomData<*mut ()>,
}

// SAFETY: each worker thread receives exclusive ownership of its WorkerStores.
// The inner Stores is a locked snapshot of main-thread data; workers never
// access the main thread's mutable state through this value.
unsafe impl Send for WorkerStores {}

impl WorkerStores {
    pub(crate) fn new(stores: Stores) -> Self {
        WorkerStores {
            stores,
            _not_sync: std::marker::PhantomData,
        }
    }
}

impl std::ops::Deref for WorkerStores {
    type Target = Stores;
    fn deref(&self) -> &Stores {
        &self.stores
    }
}

impl std::ops::DerefMut for WorkerStores {
    fn deref_mut(&mut self) -> &mut Stores {
        &mut self.stores
    }
}

/// Plan-06 phase 1 — marker telling the parent which slot in this
/// worker's `WorkerStores.allocations` to extract after join.
///
/// The output slot is a regular `Store` inside the worker's allocations
/// table, written via ordinary `OpSet*` opcodes addressed by a normal
/// `DbRef`.  After the worker thread joins, the parent calls
/// `WorkerStores::take_slot(slot.store_nr)` to extract the inner Store
/// and `Stores::adopt_store(store)` to install it into the parent's
/// allocations.  See plan-06 DESIGN.md D2.1 for the rationale.
///
/// Just a `u16`; no Drop logic.  The worker's `WorkerStores` owns the
/// underlying Store until `take_slot` extracts it.  If the worker
/// panics, the `WorkerStores` is dropped and the slot's Store is
/// freed via `Store::Drop`.
#[derive(Debug, Clone, Copy)]
pub struct WorkerOutputSlot {
    pub store_nr: u16,
}

impl WorkerStores {
    /// Append a fresh empty Store to `allocations` and return the
    /// new slot's index as a `WorkerOutputSlot` marker.
    ///
    /// Called by the parallel dispatcher right after `clone_for_light_worker`,
    /// before handing the `WorkerStores` to the worker thread.  The
    /// worker writes its result into the slot via ordinary `OpSet*`
    /// opcodes addressed by a `DbRef { store_nr: slot.store_nr, .. }`.
    ///
    /// `slot_words` is the requested capacity in 8-byte words; the
    /// minimum is one word so the underlying allocator never sees zero.
    pub fn add_output_slot(&mut self, slot_words: u32) -> WorkerOutputSlot {
        let store_nr = self.stores.allocations.len() as u16;
        let mut store = Store::new(slot_words.max(1));
        // Worker output slots are writable (free=true→false handled by
        // the worker's first claim).  Mark non-free so debug invariants
        // don't think this is a freed slot.
        store.free = false;
        self.stores.allocations.push(store);
        if store_nr >= self.stores.max {
            self.stores.max = store_nr + 1;
        }
        WorkerOutputSlot { store_nr }
    }

    /// Move the inner `Store` out of the slot, replacing it with a
    /// freed sentinel so the worker's `Drop` is a no-op for the
    /// extracted slot.
    ///
    /// Called by the parent thread after the worker joins.  The
    /// returned `Store` retains its bytes — installation into the
    /// parent's allocations table happens via `Stores::adopt_store`.
    ///
    /// # Panics
    /// Panics if `slot_nr` is out of range or has already been taken
    /// (sentinel-replaced) — both indicate dispatcher bugs.
    pub fn take_slot(&mut self, slot_nr: u16) -> Store {
        let pos = slot_nr as usize;
        assert!(
            pos < self.stores.allocations.len(),
            "take_slot: slot {slot_nr} out of range",
        );
        let sentinel = crate::store::Store::new_freed_sentinel();
        std::mem::replace(&mut self.stores.allocations[pos], sentinel)
    }

    /// Plan-06 phase 2 — extract every worker-allocated Store
    /// (those at index ≥ `parent_store_count`) for adoption by the
    /// parent.  Returns `(worker_local_store_nr, Store)` pairs in
    /// ascending store_nr order.
    ///
    /// Each returned slot is sentinel-replaced in the worker's
    /// allocations table, so the worker's `Drop` won't double-free
    /// adopted stores.  Slots that were freed by the worker during
    /// execution (free=true) are skipped.
    ///
    /// `parent_store_count` is the number of stores the parent had
    /// before the worker ran — these are clones of parent stores
    /// (the worker only read them) and must NOT be adopted.
    ///
    /// Used by the parent's stitch logic in conjunction with
    /// `Stores::adopt_store` and the `StoreRebase` rebase map (see
    /// `src/parallel.rs::StoreRebase`).
    pub fn take_all_owned(&mut self, parent_store_count: u16) -> Vec<(u16, Store)> {
        let mut out = Vec::new();
        let total = self.stores.allocations.len();
        for nr in (parent_store_count as usize)..total {
            if self.stores.allocations[nr].free {
                continue;
            }
            let sentinel = crate::store::Store::new_freed_sentinel();
            let s = std::mem::replace(&mut self.stores.allocations[nr], sentinel);
            out.push((nr as u16, s));
        }
        out
    }
}

/// @PLN63 RX1 — a snapshot of the execution heap for the reverse-step ring: a writable deep
/// byte-copy of every store allocation, plus the store-table registers a step can move.  Built
/// by [`Stores::snapshot_heap`] and re-applied by [`Stores::restore_heap`]; correct by
/// construction (a byte copy, restored by copying back).
pub struct HeapSnapshot {
    allocations: Vec<Store>,
    stack_store_at_zero: bool,
    max: u16,
}

impl std::fmt::Debug for HeapSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HeapSnapshot({} stores)", self.allocations.len())
    }
}

impl Stores {
    /// H8 — grow `allocations` to `high_water` slots (the `par` dispenser's
    /// one-past-last index) so every worker-allocated slot has a parent slot to
    /// swap into.  Paired with [`Self::swap_in_worker_slots`]; the two are the
    /// ONE home for the `par` worker-slot swap-back (the memory-safety-critical
    /// "swap dance" — previously inline in `parallel.rs`).
    pub(crate) fn grow_allocations_to(&mut self, high_water: usize) {
        while self.allocations.len() < high_water {
            self.allocations.push(crate::store::Store::new(100));
        }
    }

    /// H8 — swap each store slot `worker` allocated (`worker_allocated_indices`)
    /// into this parent by index, after a `par` worker batch joins.
    ///
    /// Store isolation — the load-bearing `par` memory-safety invariant — holds
    /// for ONE reason expressed HERE: each worker lists ONLY the slots it itself
    /// allocated, and the bounds guard skips out-of-range indices, so no two
    /// threads' slots ever alias through this swap.  Call [`Self::grow_allocations_to`]
    /// the dispenser high-water mark first, so every listed slot has a parent home.
    pub(crate) fn swap_in_worker_slots(&mut self, worker: &mut Stores) {
        for &slot_nr in &worker.worker_allocated_indices {
            let i = slot_nr as usize;
            if i < worker.allocations.len() && i < self.allocations.len() {
                std::mem::swap(&mut self.allocations[i], &mut worker.allocations[i]);
            }
        }
    }

    /// @PLN16.J — begin recording structural changes (claims / frees) on every heap
    /// store, for the debugger's edit journal.  Off by default (one branch on the cold
    /// alloc paths); turn on only for the duration of an edit, then drain with
    /// [`take_journal`](Self::take_journal).
    pub fn start_recording(&mut self) {
        for store in &mut self.allocations {
            store.start_recording();
        }
    }

    /// @PLN16.J — stop recording and drain every store's buffered changes into one
    /// `Journal`, tagging each with its `store_nr`.  An `Insert`'s `after` bytes are
    /// read from the store now (flush); a `Free`'s `before` was snapshotted at delete
    /// time.  The journal is ready to `apply` (cross-store / redo) or `revert` (undo).
    ///
    /// # Errors
    /// Returns the I/O error if writing the journal's blob fails.
    pub fn take_journal(&mut self) -> std::io::Result<Journal> {
        let mut journal = Journal::create()?;
        for sn in 0..self.allocations.len() {
            let Some(changes) = self.allocations[sn].take_recording() else {
                continue;
            };
            for change in changes {
                match change {
                    StoreChange::Insert { pos, size } => {
                        journal.record_insert(self, sn as u16, pos, size)?;
                    }
                    StoreChange::Free { pos, before } => {
                        journal.record_free(sn as u16, pos, &before)?;
                    }
                }
            }
        }
        Ok(journal)
    }

    /// @PLN63 RX1 — snapshot the execution heap for the reverse-step ring: a writable deep
    /// byte-copy of every store allocation plus the store-table registers execution mutates
    /// (`max`, `stack_store_at_zero`).  Returns `None` when a durable (file-backed) store is
    /// live — its on-disk state cannot be reversed, so RX refuses to snapshot it (an honest
    /// boundary; a normal debug session has only in-memory stores).  Compile-time schema
    /// (`types`, `names`) and cosmetic counters (`peak`, `records_created`) are execution-
    /// invariant / non-value, so they are not captured.
    #[must_use]
    pub fn snapshot_heap(&self) -> Option<HeapSnapshot> {
        if self.allocations.iter().any(Store::is_file_backed) {
            return None;
        }
        Some(HeapSnapshot {
            allocations: self.allocations.iter().map(Store::snapshot_copy).collect(),
            stack_store_at_zero: self.stack_store_at_zero,
            max: self.max,
        })
    }

    /// @PLN63 RX1 — restore a [`snapshot_heap`](Self::snapshot_heap): replace the live
    /// allocations with fresh writable copies of the snapshot's bytes (the old ones drop +
    /// free), so the heap is byte-identical to when the snapshot was taken.  The snapshot is
    /// left intact (copied, not moved) so the ring entry survives.
    pub fn restore_heap(&mut self, snap: &HeapSnapshot) {
        self.allocations = snap.allocations.iter().map(Store::snapshot_copy).collect();
        self.stack_store_at_zero = snap.stack_store_at_zero;
        self.max = snap.max;
    }

    /// @PLN14 arc B — **the materialize chokepoint**: give the value rooted at
    /// `value` (of type `tp`) its own home in `dest_store`, and return the stable
    /// [`DbRef`] that store now owns.
    ///
    /// This is the one primitive a store-resident binding environment writes
    /// through: a bind materializes its result here, and every later observe
    /// reads the returned ref.  The copy is a **deep, by-value** copy — nested
    /// structs, vectors, collections and text are re-allocated inside
    /// `dest_store`, so the result shares nothing with `value` and survives the
    /// source being mutated or freed (loft value semantics: `b = a` copies).
    ///
    /// It reuses the deep-copy walk `OpCopyRecord` runs
    /// ([`copy_block`](Self::copy_block) + [`copy_claims`](Self::copy_claims)),
    /// which already handles every container kind and copies **across** stores.
    /// That is why a value spanning several stores needs no store-number
    /// rebasing: `copy_claims` allocates each sub-record freshly in `dest_store`
    /// rather than re-pointing at the source's.
    ///
    /// A null / empty source (`store_nr == u16::MAX`, or record 0) materializes
    /// as null — a faulting bind records no value.
    ///
    /// # Panics
    /// Panics if `dest_store` is not a live store in this table.
    pub fn materialize(&mut self, value: &DbRef, tp: u16, dest_store: u16) -> DbRef {
        if value.store_nr == u16::MAX || value.rec == 0 || tp == u16::MAX {
            return self.null();
        }
        assert!(
            (dest_store as usize) < self.allocations.len()
                && !self.allocations[dest_store as usize].free,
            "materialize: destination store #{dest_store} is not live"
        );
        // Byte size of the record, then the word count a record needs: one
        // header word plus the payload (the `claim(1 + size.div_ceil(8))` shape
        // the collection deep-copies use).
        let size = u32::from(self.size(tp));
        let rec = self.allocations[dest_store as usize].claim(1 + size.div_ceil(8));
        let to = DbRef {
            store_nr: dest_store,
            rec,
            pos: 8,
        };
        // The block copy overwrites the whole payload (so a reused free block
        // leaves no garbage behind), and `copy_claims` then re-homes every
        // nested allocation the raw bytes still point at in the source.
        self.copy_block(value, &to, size);
        self.copy_claims(value, &to, tp);
        to
    }

    /// @PLN14 arc A — move a `Store` OUT of this table, leaving a freed sentinel
    /// behind, so it outlives the `Stores` it was running in.  The inverse of
    /// [`adopt_store`](Self::adopt_store), and the pair that lets the REPL's
    /// session store survive a throwaway per-eval `State`: adopt it for the run,
    /// take it back out afterwards.
    ///
    /// The returned `Store` keeps its bytes — no copy, no claim translation.
    ///
    /// # Panics
    /// Panics if `slot_nr` is out of range.
    pub fn take_store(&mut self, slot_nr: u16) -> Store {
        let pos = slot_nr as usize;
        assert!(
            pos < self.allocations.len(),
            "take_store: slot {slot_nr} out of range"
        );
        let sentinel = crate::store::Store::new_freed_sentinel();
        std::mem::replace(&mut self.allocations[pos], sentinel)
    }

    /// Install an externally-allocated `Store` into this `Stores`'
    /// allocations table.  Returns the parent-side `store_nr`.
    ///
    /// Used by the parent thread after `WorkerStores::take_slot`
    /// extracts a worker's output slot.  The `Store` keeps its bytes
    /// — no memcpy, no claim translation.  Phase 2's rebase walk
    /// rewrites cross-store DbRefs after every worker's slot is
    /// adopted.
    ///
    /// Reuses a free slot if one is available below `max`; otherwise
    /// pushes a new slot at the end.
    pub fn adopt_store(&mut self, mut store: Store) -> u16 {
        // Registration makes the store LIVE in both homes of that fact:
        // the slot's free **bit** (cleared below) and the store's own
        // `free` **flag** — `Store::new`/`Store::open` construct with
        // `free: true` and document that the database layer clears it on
        // registration.  This site forgot the flag half: release runs key
        // on the bitmap and never noticed, while every armed-build
        // `validate()` call on an adopted store (the bundle writer's
        // first push) died on "Using a freed store" (the @P317
        // bitmap-vs-flag dual invariant, F3 in the sweep catalog).
        store.free = false;
        // Inline the free-slot scan rather than calling allocation.rs's
        // private `find_free_slot` — keeping mod.rs from depending on
        // that private helper avoids cross-file plumbing for one
        // 5-line scan.
        let mut chosen: Option<u16> = None;
        for (wi, &word) in self.free_bits.iter().enumerate() {
            if word != 0 {
                let bit = word.trailing_zeros() as u16;
                let slot = wi as u16 * 64 + bit;
                if slot < self.max {
                    chosen = Some(slot);
                    break;
                }
            }
        }
        // With no reusable slot BELOW the watermark, take the one AT it — every
        // index from `max` up is unused by definition, so a slot the table
        // already holds there is free to overwrite.  Pushing instead (what this
        // did) grows `allocations` by one on every adoption that finds nothing
        // below `max`, because the Vec never shrinks while `max` does: a
        // borrow-and-release pair — @PLN119's call arena, once per placed call —
        // then walked the table upward forever, and under `LOFT_STRICT_STORES`
        // (which keeps `max` trimmed by never recycling) it exhausted all 65535
        // slots.  Same rule as `database_named`'s `find_free_slot`: reuse below
        // the watermark, else take the watermark, else grow.
        let store_nr = chosen.unwrap_or(self.max);
        if (store_nr as usize) < self.allocations.len() {
            self.allocations[store_nr as usize] = store;
        } else {
            while self.allocations.len() < store_nr as usize {
                self.allocations.push(Store::new_freed_sentinel());
            }
            self.allocations.push(store);
        }
        if store_nr >= self.max {
            self.max = store_nr + 1;
        }
        // Clear the free bit (slot is now active).
        let wi = store_nr as usize / 64;
        let bi = store_nr as usize % 64;
        if wi < self.free_bits.len() {
            self.free_bits[wi] &= !(1u64 << bi);
        }
        store_nr
    }

    /// Plan-06 phase 2b prereq — adopt all stores a worker allocated
    /// beyond `parent_store_count` and build a `StoreRebase` mapping
    /// worker-local `store_nr` → parent-side `store_nr` for each.
    ///
    /// `clone_for_light_worker` borrows every parent allocation into the
    /// worker (so worker's `allocations[0..parent_store_count]` are
    /// read-only views); any store the worker creates above that
    /// index is genuinely new.  This helper takes those new stores
    /// (replacing each with a freed sentinel so the worker's drop is
    /// a no-op for that slot) and adopts them into `self`.
    ///
    /// Returns the per-worker rebase map.  The caller uses
    /// `rebase.translate(db_ref)` to convert any worker-handed-out
    /// `DbRef` into a parent-side reference, and
    /// `rebase_walk_record(...)` to translate inner DbRef fields
    /// inside an adopted record.
    ///
    /// Currently dead code at the call-site level (no production
    /// caller wires it yet); 2b's `copy_from_worker_rebase` is the
    /// first consumer.  Self-tested in
    /// `tests/parallel_rebase.rs::adopt_worker_excess_*`.
    #[allow(dead_code)]
    pub fn adopt_worker_excess(
        &mut self,
        worker: &mut Stores,
        parent_store_count: u16,
    ) -> crate::parallel::StoreRebase {
        let mut rebase = crate::parallel::StoreRebase::with_parent_count(parent_store_count);
        let total = worker.allocations.len();
        for nr in (parent_store_count as usize)..total {
            if worker.allocations[nr].free {
                continue;
            }
            let sentinel = crate::store::Store::new_freed_sentinel();
            let s = std::mem::replace(&mut worker.allocations[nr], sentinel);
            let parent_nr = self.adopt_store(s);
            rebase.add(nr as u16, parent_nr);
        }
        rebase
    }

    /// ARC.md A2 — create a shared atomic dispenser for
    /// `run_parallel_queue_ref` workers.  The first dispensed index is
    /// `self.allocations.len() + 1` — the `+1` skips over the index
    /// each worker's stack store occupies in its own clone (every
    /// worker's `prog.new_state(ws)` push-at-ends a 1000-byte stack
    /// store at `parent_len`, so dispensing `parent_len` would try to
    /// reinitialise that stack as a user-data slot).
    ///
    /// Workers extend their own `allocations` clone to fit each
    /// dispensed index (filling skipped indices with empty
    /// `Store::new(100)` placeholders) and record the index in
    /// `worker_allocated_indices`; cross-worker collisions are
    /// impossible because the atomic dispenses globally-unique values.
    ///
    /// Replaces 8d.3's `reserve_worker_slots` / `release_worker_slots`
    /// pair (which pre-pushed `n_threads * SLOTS_PER_THREAD` fresh
    /// stores into parent and reclaimed the unused tail post-dispatch).
    /// The new design grows parent only after threads join, so we
    /// pay for exactly the slots workers actually allocated.
    pub fn make_worker_slot_dispenser(&self) -> Arc<std::sync::atomic::AtomicU16> {
        Arc::new(std::sync::atomic::AtomicU16::new(
            (self.allocations.len() as u16).saturating_add(1),
        ))
    }
}

#[cfg(test)]
mod slot_recycling_tests {
    use super::Stores;
    use crate::store::Store;

    /// @PLN123 B2 — `adopt_store` + `take_store` do NOT recycle the slot on
    /// their own, and a caller that borrows a scratch slot per operation has to
    /// know it.  `adopt_store` CLEARS the slot's free bit; `take_store` leaves a
    /// sentinel and leaves the bit clear, because it is written for a store
    /// handed out to outlive the table (the REPL session store), not for
    /// scratch.  `find_free_slot` only returns a slot whose bit is SET, so the
    /// number is burned until someone sets it back.
    ///
    /// `Stores::compact_slot` borrows a scratch slot on every load, so this is
    /// the difference between bounded and unbounded growth there — and it is
    /// invisible from loft, where the store report counts only LIVE stores and a
    /// sentinel is not one.
    #[test]
    fn a_taken_slot_is_reused_only_after_its_free_bit_is_restored() {
        let mut s = Stores::new();
        let first = s.adopt_store(Store::new_in_use(64));
        let _ = s.take_store(first);
        // Not recycled: the bit is still clear, so the next adopt lands elsewhere.
        let second = s.adopt_store(Store::new_in_use(64));
        assert_ne!(
            first, second,
            "take_store alone must not recycle the slot — if it starts to, \
             compact_slot's explicit release becomes a double-free of the slot \
             number and should be removed with it"
        );
        // Released explicitly, the number comes back.
        let _ = s.take_store(second);
        s.release_slot(second);
        let third = s.adopt_store(Store::new_in_use(64));
        assert_eq!(
            second, third,
            "a released scratch slot must be handed out again, or every \
             compaction burns one"
        );
    }
}

#[cfg(test)]
mod worker_output_slot_tests {
    use super::{Stores, WorkerStores};

    #[test]
    fn add_output_slot_returns_next_index() {
        let s = Stores::new();
        let initial = s.allocations.len();
        let mut ws = WorkerStores::new(s);
        let slot = ws.add_output_slot(64);
        assert_eq!(slot.store_nr as usize, initial);
        assert!(ws.stores.allocations[slot.store_nr as usize].capacity_words() >= 64);
    }

    #[test]
    fn add_output_slot_minimum_one_word() {
        let mut ws = WorkerStores::new(Stores::new());
        let slot = ws.add_output_slot(0);
        assert!(ws.stores.allocations[slot.store_nr as usize].capacity_words() >= 1);
    }

    #[test]
    fn take_slot_returns_owned_store_and_leaves_sentinel() {
        let mut ws = WorkerStores::new(Stores::new());
        let slot = ws.add_output_slot(32);
        let pos = slot.store_nr as usize;
        let cap = ws.stores.allocations[pos].capacity_words();
        let taken = ws.take_slot(slot.store_nr);
        assert_eq!(taken.capacity_words(), cap);
        // Sentinel left behind has tiny capacity (Store::new_freed_sentinel = 4 words)
        // and is marked free.
        assert!(ws.stores.allocations[pos].free);
    }

    #[test]
    fn adopt_store_pushes_to_parent_allocations() {
        let mut parent = Stores::new();
        let initial_len = parent.allocations.len();
        let mut donor = WorkerStores::new(Stores::new());
        let slot = donor.add_output_slot(16);
        let store = donor.take_slot(slot.store_nr);
        let nr = parent.adopt_store(store);
        assert!((nr as usize) < parent.allocations.len() || (nr as usize) == initial_len);
        assert!(!parent.allocations[nr as usize].free);
    }

    #[test]
    #[should_panic(expected = "take_slot: slot")]
    fn take_slot_out_of_range_panics() {
        let mut ws = WorkerStores::new(Stores::new());
        let _ = ws.take_slot(9999);
    }

    #[test]
    fn take_all_owned_skips_parent_clone_slots() {
        // Parent has 2 stores; worker will get 2 clones + add 1 output.
        let mut parent = Stores::new();
        parent.allocations.push(crate::store::Store::new(8));
        parent.allocations.push(crate::store::Store::new(8));
        parent.max = 2;
        let parent_count = parent.allocations.len() as u16;

        // Build a synthetic worker view with 2 cloned slots + 1 output.
        let mut ws_inner = Stores::new();
        ws_inner.allocations.push(crate::store::Store::new(8));
        ws_inner.allocations.push(crate::store::Store::new(8));
        ws_inner.max = 2;
        let mut ws = WorkerStores::new(ws_inner);
        let _slot = ws.add_output_slot(16);

        let owned = ws.take_all_owned(parent_count);
        assert_eq!(owned.len(), 1, "only worker-allocated slot is adopted");
        assert_eq!(owned[0].0, 2, "adopted slot is at parent_count");
    }

    #[test]
    fn take_all_owned_returns_multiple_in_order() {
        let mut ws = WorkerStores::new(Stores::new());
        let _s0 = ws.add_output_slot(8);
        let _s1 = ws.add_output_slot(16);
        let _s2 = ws.add_output_slot(32);
        let owned = ws.take_all_owned(0);
        assert_eq!(owned.len(), 3);
        assert_eq!(owned[0].0, 0);
        assert_eq!(owned[1].0, 1);
        assert_eq!(owned[2].0, 2);
    }

    #[test]
    fn take_all_owned_skips_freed_slots() {
        let mut ws = WorkerStores::new(Stores::new());
        let s0 = ws.add_output_slot(8);
        let _s1 = ws.add_output_slot(16);
        // Mark s0 as freed.
        ws.stores.allocations[s0.store_nr as usize].free = true;
        let owned = ws.take_all_owned(0);
        assert_eq!(owned.len(), 1, "freed slot skipped");
        assert_eq!(owned[0].0, 1);
    }
}

#[cfg(test)]
mod store_rebase_tests {
    use super::DbRef;
    use crate::parallel::StoreRebase;

    #[test]
    fn translate_passes_through_unmapped() {
        // PARENT-SHARED pass-through (D11c row 2): unmapped but below the
        // parent store count.  An unmapped ref at/above the count is the
        // CROSS-WORKER row — debug-panic, not pass-through — covered by
        // `translate_cross_worker_is_a_debug_panic` below.
        let r = StoreRebase::with_parent_count(6);
        let db = DbRef {
            store_nr: 5,
            rec: 10,
            pos: 8,
        };
        let out = r.translate(&db);
        assert_eq!(out.store_nr, 5);
        assert_eq!(out.rec, 10);
        assert_eq!(out.pos, 8);
    }

    /// D11c row 3 — a DbRef that is neither mapped nor parent-shared is a
    /// codegen bug: debug builds panic at the fault, release builds log and
    /// pass through (graceful degradation).  Pin BOTH profiles.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "cross-worker DbRef"))]
    fn translate_cross_worker_is_a_debug_panic() {
        let r = StoreRebase::new();
        let db = DbRef {
            store_nr: 5,
            rec: 10,
            pos: 8,
        };
        let out = r.translate(&db);
        // Only reached in release builds (debug_assertions off).
        assert_eq!(out.store_nr, 5, "release path passes through unchanged");
    }

    #[test]
    fn translate_rewrites_mapped() {
        let mut r = StoreRebase::new();
        r.add(2, 7);
        let db = DbRef {
            store_nr: 2,
            rec: 1,
            pos: 4,
        };
        let out = r.translate(&db);
        assert_eq!(out.store_nr, 7, "store_nr translated");
        assert_eq!(out.rec, 1, "rec preserved");
        assert_eq!(out.pos, 4, "pos preserved");
    }

    #[test]
    fn multiple_translations_disjoint() {
        let mut r = StoreRebase::new();
        r.add(2, 7);
        r.add(3, 8);
        r.add(4, 9);
        for (worker_nr, expected_parent) in [(2, 7), (3, 8), (4, 9)] {
            let db = DbRef {
                store_nr: worker_nr,
                rec: 0,
                pos: 0,
            };
            assert_eq!(r.translate(&db).store_nr, expected_parent);
        }
    }
}

#[allow(dead_code)]
/// #333 — armed (process-locally) by the generated native binary's `main`
/// so `raise_runtime` halts at the faulting op with the interpreter's exit
/// contract (render + exit 1).  Stays false inside a host interpreter that
/// loads auto-native cdylibs: each cdylib links its own copy of this static,
/// so a bridged fn's raise keeps the record-only behaviour and the host's
/// dispatch loop reports it.
pub static NATIVE_FAIL_FAST: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

impl Stores {
    #[must_use]
    pub fn new() -> Stores {
        let mut result = Stores {
            types: Vec::new(),
            names: HashMap::new(),
            allocations: Vec::new(),
            stack_store_at_zero: false,
            lazy_sources: HashMap::new(),
            lazy_errors: HashMap::new(),
            paged_refusal: None,
            lazy_driver_allocs: None,
            files: Vec::new(),
            max: 0,
            peak: 0,
            records_created: 0,
            stores_allocated: 0,
            alloc_pc: 0,
            free_bits: Vec::new(),
            bridge_text_dest: None,
            const_refs: Vec::new(),
            last_parse_errors: Vec::new(),
            last_json_errors: Vec::new(),
            parallel_ctx: None,
            par_buffer_stack: Vec::new(),
            par_text_buffer_stack: Vec::new(),
            par_ref_buffer_stack: Vec::new(),
            par_narrow_buffer_stack: Vec::new(),
            par_fn_buffer_stack: Vec::new(),
            par_fn_native_buffer_stack: Vec::new(),
            logger: None,
            had_fatal: false,
            runtime_error: None,
            source_dir: String::new(),
            // #255 / @PLN9: program-relative by default — a relative file path
            // re-homes against the program's own directory, so "program + assets"
            // is a portable bundle.  CLI tools opt back into cwd with `#cwd`.
            program_relative: true,
            frame_yield: false,
            poison_free: false,
            disable_slot_reuse: false,
            uaf_freed_this_op: Vec::new(),
            worker_slot_dispenser: None,
            worker_allocated_indices: Vec::new(),
            report_asserts: false,
            assert_results: Vec::new(),
            user_args: Vec::new(),
            #[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]
            start_time: Instant::now(),
            // `Stores::new()` must not call `Instant::now()` or
            // `SystemTime::now()` on wasm32-unknown-unknown — both
            // trap as `(unreachable)` with no time source.  The
            // `--html` build (wasm32, no `wasm` feature) uses 0 as
            // the epoch stub; the full `wasm` feature build routes
            // through the host bridge.
            #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), feature = "wasm"))]
            start_time_ms: crate::wasm::host_time_now(),
            #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
            start_time_ms: 0,
            call_stack_snapshot: Vec::new(),
            variables_snapshot: Vec::new(),
            closure_map: HashMap::new(),
            jnull_sentinel: None,
        };
        result.base_type("integer", 8); // 0  (Phase 2c: widened from 4)
        result.base_type("long", 8); // 1
        result.base_type("single", 4); // 2
        result.base_type("float", 8); // 3
        result.base_type("boolean", 1); // 4
        result.base_type("text", 4); // 5
        result.base_type("character", 4); // 6
        result
    }

    /// #255 / @PLN9: the single home every file-op site routes a
    /// program-supplied path through.  Absolute paths and the cwd-relative
    /// default pass through unchanged; under `program_relative` a *relative*
    /// path re-homes against `source_dir` — the program's own directory (the
    /// source dir under `--interpret` and under driver-mode native, where the
    /// driver hands it down via `LOFT_SOURCE_DIR`; the executable's dir for a
    /// standalone native bundle), so "program + assets" is a portable bundle
    /// that runs from any cwd.  Empty `source_dir` (no anchor) falls back to
    /// cwd, never to a wrong file.
    ///
    /// There is no path-shape filter here, and that is the point (loft#712).
    /// A lexical `..` refusal used to live on this path: it rejected
    /// `file("../a.txt")` and reported the refusal as a null size — which a
    /// reader doing `if f#size < HEADER` turns into "the file is truncated", a
    /// DATA error for what was a PATH decision.  It was never containment
    /// either: the same bytes by absolute path were served, and a `..` that
    /// normalised back INSIDE the root (`../sub/s.loft`) was refused too, so it
    /// filtered text rather than checking a boundary.  loft has no filesystem
    /// sandbox — admission is decided at load time and carries no runtime
    /// checks (`SANDBOX.md`) — so the resolved path is the whole answer and the
    /// filesystem gives it.
    #[must_use]
    pub fn resolve_path(&self, raw: &str) -> String {
        if !self.program_relative || self.source_dir.is_empty() {
            return raw.to_string();
        }
        let p = std::path::Path::new(raw);
        if p.is_absolute() {
            return raw.to_string();
        }
        std::path::Path::new(&self.source_dir)
            .join(p)
            .to_string_lossy()
            .into_owned()
    }

    /// Plan-07 phase 4c — Stores-side counterpart of `State::raise`.
    /// Used by native codegen, which has `&mut Stores` (via
    /// `unsafe { &mut *cell.get() }`) but no `&mut State`.  The
    /// native template rewriter in `src/generation/calls.rs`
    /// translates `s.raise(...)` → `stores.raise_runtime(...)` so
    /// the same `default/01_code.loft` annotations work in both
    /// contexts.  The `_runtime` suffix is mandatory: a plain
    /// `stores.raise(` would let a second pass of the substring-
    /// based substitution match `s.raise(` inside the just-
    /// produced output and accumulate `stor` prefixes
    /// (observed: `storestorestores.raise(`).  Sibling helpers
    /// follow the same convention (`*_runtime` rename).
    ///
    /// **Position is `None` for the native path today** — native
    /// doesn't have a bytecode pc → Position lookup table.  Phase 4g
    /// (backtrace + polish) will thread codegen-time positions
    /// through.  For now native diagnostics omit `--> file:line:col`
    /// (rendered as just `error: <kind detail>` without the
    /// location header).  Better than today's `cannot find value 's'`
    /// rustc compile error from the s.raise call.
    ///
    /// Production-vs-development split mirrors `State::raise` per
    /// `DESIGN_DECISIONS.md § C66`:
    /// - production logger attached → log via `log_runtime_kind` +
    ///   set `had_fatal` + return without populating `runtime_error`
    /// - development (no logger or non-production logger) → populate
    ///   `runtime_error` so the dispatch loop's short-circuit fires
    ///   and main.rs renders the typed error
    pub fn raise_runtime(&mut self, kind: crate::runtime_error::RuntimeErrorKind) {
        let production = self
            .logger
            .as_ref()
            .and_then(|l| l.lock().ok())
            .is_some_and(|l| l.config.production);
        // Plan-07 phase 4g.3 — `--dev-soft-halt` mirror of the
        // State::raise path.  When the env var is set, demote to
        // log-and-continue regardless of logger.production flag
        // so a single run surfaces every fault site.
        let dev_soft_halt =
            std::env::var("LOFT_DEV_SOFT_HALT").is_ok_and(|v| v == "1" || v == "true");
        if production || dev_soft_halt {
            if let Some(logger) = &self.logger
                && let Ok(mut lg) = logger.lock()
            {
                lg.log_runtime_kind(&kind, None);
            }
            if dev_soft_halt {
                crate::loft_eprintln!("soft-halt: {}", kind.describe());
            }
            self.had_fatal = true;
            return;
        }
        let message = kind.describe();
        let err = crate::runtime_error::RuntimeError {
            kind,
            position: None,
            op_pc: u32::MAX,
            message,
            // Stores-side raise (native codegen path) has no
            // access to call_stack; slice 2 of 4g.1 will thread
            // it through.
            call_chain: Vec::new(),
            crossed_placement: false,
        };
        // #333 — the standalone native binary mirrors the interpreter's
        // halt-at-the-op contract: render the error and exit 1 instead of
        // recording it for a check nobody runs (pre-fix, `5 / 0` printed
        // a wrong value and exited 0 on --native).
        if NATIVE_FAIL_FAST.load(std::sync::atomic::Ordering::Relaxed) {
            crate::loft_eprintln!("error: {}", err.message);
            std::process::exit(1);
        }
        self.runtime_error = Some(Box::new(err));
        self.had_fatal = true;
    }

    /// Remove `cur` from a tree-backed collection MID-ITERATION and answer the cursor that
    /// continues the walk: `None` when the walk is over, `Some(park)` when the caller should
    /// park the cursor at `park` so the next `OpStep` yields whatever followed `cur`.
    ///
    /// One home, because the two backends each had this and each had the same defect. The
    /// walk ends when the node REMOVED was the range's last (`cur == finish`) — not when its
    /// SUCCESSOR is. `finish` names the last node to VISIT: `step` yields it and only then
    /// marks the end. Ending a step early meant the successor was never visited, so removing
    /// consecutive elements skipped one — `for r in c[1..4] { r#remove; }` over 1..5 left
    /// `3` behind. It was invisible on an unbounded walk, where `finish` is `0` and the test
    /// could never fire, and that is the only shape the suite covered (loft#1272).
    ///
    /// The successor is read BEFORE the removal, while the tree pointers still stand, and the
    /// predecessor AFTER it, in the tree the removal left.
    pub fn remove_during_tree_iteration(
        &mut self,
        data: &DbRef,
        tp: u16,
        cur: u32,
        finish: u32,
        reverse: bool,
    ) -> Option<u32> {
        let fields = self.fields(tp);
        let cur_ref = crate::keys::DbRef {
            store_nr: data.store_nr,
            rec: cur,
            pos: u32::from(fields),
        };
        let n_after = {
            let store = crate::keys::store(data, &self.allocations);
            if reverse {
                crate::tree::previous(store, &cur_ref)
            } else {
                crate::tree::next(store, &cur_ref)
            }
        };
        // Read before the removal: `cur` is about to stop being a node.
        let removed_the_last = cur == finish;
        self.remove_owned(data, &cur_ref, tp);
        if n_after == 0 || removed_the_last {
            return None;
        }
        let n_ref = crate::keys::DbRef {
            store_nr: data.store_nr,
            rec: n_after,
            pos: u32::from(fields),
        };
        let store = crate::keys::store(data, &self.allocations);
        Some(if reverse {
            crate::tree::next(store, &n_ref)
        } else {
            crate::tree::previous(store, &n_ref)
        })
    }

    /// Did this run end on a fault?  The exit-code question, asked in one place because
    /// three exits ask it: the interpreter's `main`, and the two generated `fn main()`
    /// templates the native emitter writes.
    ///
    /// `had_fatal` alone does not answer it. An integer overflow surfaced by
    /// `--dev-soft-halt` is reported from `ops`, which has no `Stores` to mark (see
    /// `ops::note_integer_overflow`), so a run whose only fault was an overflow would
    /// print its `soft-halt:` line and still exit 0 — clean, to anything reading the
    /// status rather than the output.
    #[must_use]
    pub fn run_failed(&self) -> bool {
        self.had_fatal || crate::ops::overflow_surfaced()
    }

    /// @P356 — Stores-side counterpart of `State::raise_recoverable`.  Logs a
    /// `Warn` and returns WITHOUT halting, so the native backend continues
    /// with the null sentinel — identical to the interpreter.
    /// `LOFT_DEV_SOFT_HALT` opts into fail-fast (delegates to `raise_runtime`).
    pub fn raise_recoverable_runtime(&mut self, kind: crate::runtime_error::RuntimeErrorKind) {
        let dev_soft_halt =
            std::env::var("LOFT_DEV_SOFT_HALT").is_ok_and(|v| v == "1" || v == "true");
        if dev_soft_halt {
            self.raise_runtime(kind);
            return;
        }
        if let Some(logger) = &self.logger
            && let Ok(mut lg) = logger.lock()
        {
            lg.log_runtime_kind(&kind, None);
        }
    }

    /// Nullable narrow-FIELD store with a dev-only sentinel-collision warning.
    ///
    /// `OpSetByteNullable` reserves the all-ones byte (255) for null, so a
    /// NON-null value that encodes onto it (`val - min == 255`) reads back as
    /// null — silent data loss the compile-time check can only catch for
    /// literals.  This logs a `Warn` through the attached logger, which is
    /// rate-limited + level-filtered: it points the developer at the field
    /// DURING development (interpreter, default `Warn`) and is silent in a
    /// shipped game (no dev logger attached, or a production config).  Behaviour
    /// is unchanged — the value still stores as the sentinel; only the
    /// diagnostic is added.  `val` is `i32::MIN` for an intentional null.
    pub fn set_byte_nullable(&mut self, db: &DbRef, pos: u32, min: i32, val: i32) {
        self.warn_narrow_sentinel(val, min, 0xFF, pos);
        // loft#984 — a value the field cannot represent takes the type's DEFAULT, and for
        // a NULLABLE field that default is `null`, not the bottom of the range: absence is
        // a value this type can hold, and it is the honest one for "this did not fit".
        // (The non-nullable setter defaults to `min` for the same reason — that is the
        // only default IT has.)
        let store = if crate::store::Store::byte_fits(min, val) {
            val
        } else {
            i32::MIN
        };
        self.store_mut(db).set_byte(db.rec, pos, min, store);
    }

    /// 2-byte twin of [`Self::set_byte_nullable`]: the all-ones code is 65535.
    pub fn set_short_nullable(&mut self, db: &DbRef, pos: u32, min: i32, val: i32) {
        self.warn_narrow_sentinel(val, min, 0xFFFF, pos);
        // See [`Self::set_byte_nullable`] — nullable defaults to `null` (loft#984).  The
        // `+1` encoding reserves raw 0, so `min + 65535` is the value that cannot be
        // stored here; it read back as null before by COLLIDING with the sentinel, and now
        // it is written as null deliberately.  Same observable answer, stated rather than
        // stumbled into — `tests/scripts/389-narrow-runtime-collision.loft` pins it.
        let store = if crate::store::Store::short_fits(min, val) {
            val
        } else {
            i32::MIN
        };
        self.store_mut(db).set_short(db.rec, pos, min, store);
    }

    /// Emit the dev-only warning when a non-null `val` encodes onto the all-ones
    /// null sentinel of a nullable narrow field.  The `logger.is_none()` guard
    /// collapses the shipped-game path to a single `Option` check; the rate
    /// limiter keys on the field offset (`pos`), so a loop writing one field
    /// warns once, not per iteration.
    fn warn_narrow_sentinel(&mut self, val: i32, min: i32, all_ones: i32, pos: u32) {
        // The stored byte/short is `(val - min) as uN`; it collides with the
        // all-ones sentinel when its low N bytes are all-ones.  MASK before
        // comparing — for a SIGNED type the sacrificed value is the bottom edge
        // (nullable i8 `-128`, i16 `-32768`), whose `val - min` is `-1`, not
        // `0xFF`/`0xFFFF`; `-1 & all_ones == all_ones` catches it, a bare `==`
        // does not.  (`val == i32::MIN` is the intentional-null short-circuit.)
        if self.logger.is_none()
            || val == i32::MIN
            || (val.wrapping_sub(min) & all_ones) != all_ones
        {
            return;
        }
        if let Some(logger) = &self.logger
            && let Ok(mut lg) = logger.lock()
        {
            let usable_hi = i64::from(min) + i64::from(all_ones) - 1;
            lg.log(
                crate::logger::Severity::Warn,
                "nullable-narrow-field",
                pos,
                &format!(
                    "value {val} written to a nullable narrow field collides with the null \
                     sentinel and reads back as null (usable {min}..={usable_hi}); declare the \
                     field `not null`, or keep values within the usable range"
                ),
            );
        }
    }

    /// The format-fault cause lives in [`crate::ops`], per thread, rather than on `Stores` —
    /// a codegen constraint as much as a design one. The native emitter inlines an op's
    /// `#rust` body into whatever expression contains it, so a body that writes through
    /// `stores` lands inside another `stores.` call's arguments and rustc rejects it with
    /// E0502 (loft#1169). Free functions over thread-local state borrow nothing and compose in
    /// any position. These are thin delegates so existing call sites read the same.
    pub fn set_format_fault(&mut self, kind_id: u8) {
        crate::ops::note_format_fault(kind_id, true);
    }

    /// See [`crate::ops::note_format_fault`].
    pub fn note_format_fault(&mut self, kind_id: u8, faulted: bool) {
        crate::ops::note_format_fault(kind_id, faulted);
    }

    /// See [`crate::ops::arm_format_fault`].
    pub fn arm_format_fault(&mut self) {
        crate::ops::arm_format_fault();
    }

    /// See [`crate::ops::take_format_fault`].
    #[must_use]
    pub fn take_format_fault(&mut self) -> Option<&'static str> {
        crate::ops::take_format_fault()
    }

    /// Plan-07 phase 4c — Stores-side counterpart of
    /// `State::vec_get_or_raise`.  Same body, calls
    /// `self.raise_runtime(...)` on OOB.  Native template rewriter
    /// translates `s.vec_get_or_raise(...)` →
    /// `stores.vec_get_or_raise_runtime(...)`.  The `_runtime`
    /// suffix avoids substring-collision with the substitutor —
    /// see `raise_runtime` for the explanation.
    #[must_use]
    pub fn vec_get_or_raise_runtime(
        &mut self,
        db: &crate::keys::DbRef,
        size: u32,
        index: i64,
    ) -> crate::keys::DbRef {
        let len = crate::vector::length_vector(db, &self.allocations);
        // @FR-H-Index — the native half of the one normalisation: negative counts from the
        // end, and only a STILL-out-of-range index is out of bounds.  Kept byte-for-byte with
        // `State::vec_get_or_raise` because a divergence here is a different program, not a
        // different speed.
        let normalized = if index < 0 {
            index + i64::from(len)
        } else {
            index
        };
        if normalized < 0 {
            self.raise_recoverable_runtime(crate::runtime_error::RuntimeErrorKind::NegativeIndex {
                idx: index,
            });
            // `nullref`, the one value spelling of absence (`DbRef::or_null`,
            // @FR-L-Null) — see `State::vec_get_or_raise`, its interpreter twin.
            return crate::keys::DbRef::NULL;
        }
        if normalized >= i64::from(len) {
            self.raise_recoverable_runtime(
                crate::runtime_error::RuntimeErrorKind::IndexOutOfBounds { idx: index, len },
            );
            return crate::keys::DbRef::NULL;
        }
        crate::vector::get_vector(db, size, index, &self.allocations)
    }

    /// `vec_get_or_raise_runtime` with the store resolution and the length load supplied
    /// by an already-derived [`crate::vector::VecHeader`] (loft#885).
    ///
    /// The in-range fast path is address arithmetic; every other index — negative,
    /// out-of-range, `i64::MIN` — falls through to `vec_get_or_raise_runtime`, so the
    /// raise it reports and the sentinel it answers have only one definition.
    #[must_use]
    #[inline]
    pub fn vec_get_hoisted_or_raise_runtime<const VERIFY: bool>(
        &mut self,
        h: &crate::vector::VecHeader,
        db: &crate::keys::DbRef,
        size: u32,
        index: i64,
    ) -> crate::keys::DbRef {
        if index >= 0 && index < i64::from(h.len) {
            crate::vector::get_vector_hoisted::<VERIFY>(h, db, size, index, &self.allocations)
        } else {
            self.vec_get_or_raise_runtime(db, size, index)
        }
    }

    /// Plan-07 phase 4c — Stores-side counterpart of
    /// `State::vec_ref_or_raise`.  Same body; native rewriter
    /// translates `s.vec_ref_or_raise(...)` →
    /// `stores.vec_ref_or_raise_runtime(...)`.
    #[must_use]
    pub fn vec_ref_or_raise_runtime(
        &mut self,
        db: &crate::keys::DbRef,
        index: i64,
    ) -> crate::keys::DbRef {
        let inner = self.vec_get_or_raise_runtime(db, 4, index);
        // `get_ref` already short-circuits to a null DbRef when
        // `inner.rec == 0`, so no extra guard is needed.
        self.get_ref(&inner, 0)
    }

    /// Plan-07 phase 4c — Stores-side counterpart of
    /// `State::text_char_or_raise`.  Same body; native rewriter
    /// translates `s.text_char_or_raise(...)` →
    /// `stores.text_char_or_raise_runtime(...)`.
    #[must_use]
    pub fn text_char_or_raise_runtime(&mut self, val: &str, index: i64) -> char {
        let len = val.len() as i64;
        let normalized = if index < 0 { index + len } else { index };
        if normalized < 0 {
            self.raise_recoverable_runtime(crate::runtime_error::RuntimeErrorKind::NegativeIndex {
                idx: index,
            });
            return char::from(0);
        }
        if normalized >= len {
            self.raise_recoverable_runtime(
                crate::runtime_error::RuntimeErrorKind::IndexOutOfBounds {
                    idx: index,
                    len: len as u32,
                },
            );
            return char::from(0);
        }
        crate::ops::text_character(val, index)
    }

    /// Initiative 03 Phase 3b: return a `Str` pointing into the
    /// constant store.  Native-mode counterpart to
    /// `State::string_from_const_store`, which pushes the Str onto
    /// the bytecode interpreter's stack.  Native code uses the
    /// value directly via the `#rust"…"` template substitution
    /// `s.string_from_const_store` → `stores.string_from_const_store`.
    #[must_use]
    pub fn string_from_const_store(&self, rec: u32, _pos: u32) -> crate::keys::Str {
        let store = &self.allocations[CONST_STORE as usize];
        let len = store.get_u32_raw(rec, 4);
        let ptr = unsafe { store.ptr.offset(rec as isize * 8 + 8) };
        crate::keys::Str { ptr, len }
    }

    /// `_runtime` peer of [`Self::string_from_const_store`] — the
    /// native codegen substitution rewrites
    /// `s.string_from_const_store(` to `stores.string_from_const_store_runtime(`
    /// to break the substring chain (see `const_ref_at_runtime`).
    /// @P275 fix.
    #[must_use]
    pub fn string_from_const_store_runtime(&self, rec: u32, pos: u32) -> crate::keys::Str {
        self.string_from_const_store(rec, pos)
    }

    /// Native-side accessor for `const_refs[d_nr]` used via the
    /// `#rust"s.const_ref_at(@d_nr as usize)"` template substitution
    /// (see `src/generation/calls.rs` — pattern `s.const_ref_at(`
    /// rewrites to `stores.const_ref_at_runtime(`).  The `_runtime`
    /// suffix breaks the substring chain that would otherwise let the
    /// substitution accumulate `stor` prefixes when `OpConstRef` is
    /// nested inside another opcode template (e.g. `OpGetVector` →
    /// `OpConstRef`).  Same trick used for `raise_runtime`,
    /// `vec_get_or_raise_runtime`, etc.  @P275 fix.
    #[must_use]
    pub fn const_ref_at_runtime(&self, d_nr: usize) -> DbRef {
        self.const_refs[d_nr]
    }

    #[must_use]
    pub fn get<T: 'static>(&mut self, stack: &mut DbRef) -> &T {
        // @PLAN53 cluster 2 / S4: pop the value's STEPPED span (8-rounded in
        // aligned mode) so a native arg occupying a stepped slot is reached
        // correctly; identity (real size_of) when off → flag-OFF unchanged.
        let step = crate::variables::aligned_stack_step(size_of::<T>() as u32);
        debug_assert!(
            stack.pos >= step,
            "Stack underflow in get<{}>: stack.pos={} but need {} bytes",
            std::any::type_name::<T>(),
            stack.pos,
            step,
        );
        stack.pos -= step;
        let r = self.store(stack).addr::<T>(stack.rec, stack.pos);
        #[cfg(debug_assertions)]
        {
            if std::any::TypeId::of::<T>() == std::any::TypeId::of::<DbRef>() {
                let db: &DbRef = unsafe { &*(r as *const T as *const DbRef) };
                debug_assert!(
                    db.store_nr == u16::MAX || (db.store_nr as usize) < self.allocations.len(),
                    "get<DbRef>: OOB store_nr={} (allocations.len()={}) \
                     rec={} pos={} — corrupt DbRef on stack",
                    db.store_nr,
                    self.allocations.len(),
                    db.rec,
                    db.pos,
                );
            }
        }
        r
    }

    pub fn put<T: 'static>(&mut self, stack: &mut DbRef, val: T) {
        #[cfg(debug_assertions)]
        {
            if std::any::TypeId::of::<T>() == std::any::TypeId::of::<DbRef>() {
                let db: &DbRef = unsafe { &*(&val as *const T as *const DbRef) };
                debug_assert!(
                    db.store_nr == u16::MAX || (db.store_nr as usize) < self.allocations.len(),
                    "put<DbRef>: OOB store_nr={} (allocations.len()={}) \
                     rec={} pos={} — corrupt DbRef being pushed",
                    db.store_nr,
                    self.allocations.len(),
                    db.rec,
                    db.pos,
                );
            }
        }
        let m = self.store_mut(stack).addr_mut::<T>(stack.rec, stack.pos);
        *m = val;
        // @PLAN53 cluster 2 / S4: push the value's STEPPED span so a native
        // result lands at the aligned slot the caller's codegen expects;
        // identity when off.
        stack.pos += crate::variables::aligned_stack_step(size_of::<T>() as u32);
    }

    /// Look up a type by index, panicking with a diagnostic if the index is out of range.
    ///
    /// # Panics
    /// Panics if `nr` is out of range for the types table.
    #[must_use]
    pub fn get_type(&self, nr: u16) -> &Type {
        self.types.get(nr as usize).unwrap_or_else(|| {
            panic!(
                "type index {} out of range (total: {})",
                nr,
                self.types.len()
            )
        })
    }
}

/// The single record renderer — one schema walk, four output modes selected by the
/// flags below.  Each mode shares the traversal (the `Parts` dispatch, the null-skipping
/// field loop, the `next()` vector loop) and differs only at the emission points:
///
/// - **loft** — re-parseable native source (`TypeName{field: value}`, `Enum.Variant`,
///   quoted+escaped text, forced-decimal floats).  Backs `Stores::show_loft`, the
///   round-tripping own-format serializer (@PLN12 REPL.X / live data migration).
/// - **json** — RFC 8259 JSON.  Backs `T.to_json()`.
/// - debug (neither flag) — bare `{ field: value }`, the pretty inspector form.
/// - **dump** — the structured trace form with `#store.rec` references, `compact`
///   single-line option, and depth/element **limits** (the old `DumpDb`).  Backs the
///   `LOFT_LOG` execution trace + `tests/dumps/*.txt`.
///
/// `max_depth` / `max_elements` (default `u16::MAX` = unlimited) bound *any* mode — so
/// the loft form can be rendered bounded too (`show_loft_bounded`), the `{clean, bounded}`
/// the debugger's variables panel needs.  `indent` doubles as the nesting depth (it
/// increments exactly once per level), so the limit guards key off it directly.
// The flags are orthogonal output toggles (`pretty` modifies, `json`/`loft`/`dump` select
// the format, `compact` is a dump-only modifier), not a packed state — a mode enum would
// just re-spell the same set while churning every `if self.json/loft/dump` site.
#[allow(clippy::struct_excessive_bools)]
pub struct ShowDb<'a> {
    pub stores: &'a Stores,
    pub store: u16,
    pub rec: u32,
    pub pos: u32,
    pub known_type: u16,
    pub pretty: bool,
    pub json: bool,
    pub loft: bool,
    /// Trace form: `#store.rec` references, `compact`, and the depth/element limits below
    /// are the truncation markers (`{...}` / `...N more`).  Mutually exclusive with
    /// `loft`/`json` (it is the old `DumpDb` mode).
    pub dump: bool,
    /// Single-line dump (spaces instead of newlines) — only meaningful with `dump`.
    pub compact: bool,
    /// Maximum nesting depth before `{...}` / `[N items...]` (`u16::MAX` = unlimited).
    pub max_depth: u16,
    /// Maximum array/vector elements before `...N more` (`u16::MAX` = unlimited).
    pub max_elements: u16,
}

/// `get_type()` with an out-of-range index must panic with a helpful message.
#[test]
#[should_panic(expected = "type index 999 out of range")]
fn get_type_out_of_range_panics() {
    let stores = Stores::new();
    let _ = stores.get_type(999);
}

// These values are for amd64 or arm64 systems.
// It's not possible to test these continuously as these will fail on 32-bit systems.
#[test]
fn sizes() {
    /*
    assert_eq!(size_of::<DbRef>(), 12);
    assert_eq!(size_of::<String>(), 24);
    assert_eq!(size_of::<&str>(), 16);
    */
}
