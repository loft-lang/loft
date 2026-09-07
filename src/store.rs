// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I69 — Word-addressed store

// @PLAN38 phase 01: this module is now `pub mod store` (was `mod store`).
// Promoting the module surfaced clippy::pedantic lints on existing
// public methods (panic/error doc sections, must_use, is_empty, etc.)
// that the codebase has always tolerated as internal — fixing each one
// is a project-wide doc sweep, out of scope here.  Allow them at the
// module level so this PR stays focused on the durable-store API.
#![allow(
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::collapsible_if,
    clippy::not_unsafe_ptr_arg_deref,
    clippy::len_without_is_empty,
    clippy::return_self_not_must_use,
    clippy::unnecessary_debug_formatting
)]

//! Word-addressed heap store with bump allocation and free-block reuse.
//!
//! Each [`Store`] is a contiguous buffer of 8-byte words.  Records are
//! allocated via [`claim`](Store::claim) and freed via
//! [`delete`](Store::delete).
//!
//! ## Memory layout
//!
//! Every record starts with a **signed size header** (one `i32` word):
//! - **Positive** → claimed record; magnitude = size in words (incl. header).
//! - **Negative** → free block; magnitude = size in words.
//!
//! Free blocks are tracked in a `BTreeMap` for gap-based allocation.
//! Adjacent free blocks are merged on `delete` to reduce fragmentation.
//!
//! Record 0 is the store header; record 1 (`PRIMARY`) is the main record
//! describing vectors and indexes with sub-records.  A store may optionally
//! be backed by a memory-mapped file (`mmap` feature).

#[cfg(feature = "mmap")]
use mmap_storage::file::Storage as MmapStorage;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};

#[allow(dead_code)]
static A: System = System;
const SIGNATURE: u32 = 0x53_74_6f_31;
pub const PRIMARY: u32 = 1;
/// Byte offset of a record's PAYLOAD — past the 8-byte size header at word 0.
///
/// A field's `position` in a struct type is an offset from HERE, so any `DbRef`
/// used to walk a record's fields must sit at this position.  Naming it makes
/// the difference visible between such a `DbRef` and one a caller navigated
/// with: an `index`'s tree cursor carries the red-black link's offset instead,
/// and walking fields from that is loft#718.
pub const RECORD_PAYLOAD: u32 = 8;
/// Maximum store size in words; offsets are stored as `i32` so this is the limit.
pub const MAX_STORE_WORDS: u32 = i32::MAX as u32;

/// Minimum free-block size (words) to register in the LLRB free-space tree.
/// A node needs 4 (left) + 4 (right) + 1 (color) bytes after the 4-byte header = 13 bytes.
/// Two 8-byte words (16 bytes) comfortably hold these fields.
const MIN_FREE_TREE: i32 = 2;

/// The kernel's page — the granularity `madvise` drops at, asked of the host rather
/// than assumed to be 4 KB (it is 16 KB on aarch64 macOS and configurable on ppc64).
/// A hard-coded 4096 would mean [`Store::release_resident`] handing `madvise` a length
/// that is not a whole number of pages, which it rounds DOWN — silently dropping less
/// than the caller was told.
#[cfg(all(feature = "mmap", unix))]
fn page_bytes() -> u64 {
    static PAGE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    #[allow(clippy::cast_sign_loss)]
    *PAGE.get_or_init(|| unsafe { libc::sysconf(libc::_SC_PAGESIZE).max(4096) as u64 })
}

/// Smallest size a FILE-BACKED arena is kept at, in words.  [`Store::open`]
/// lifts anything under it back up to 8192 bytes, so an image written below the
/// floor — or a shrink past it ([`Store::shrink_to`]) — only buys an immediate
/// re-grow.  One home for the floor, since three places depend on it.  A heap
/// store has no floor beyond [`Store::new`]'s two words.
pub const MIN_BOUND_WORDS: u32 = 1024;

/// The capacity a store of `live_end` words of content should be given: the
/// high-water mark plus an **eighth**.  One home for a fact two paths need —
/// the image [`crate::database::Stores::bind_path`] writes, and the capacity
/// [`Store::reclaim_tail`] shrinks a live store to.
///
/// Never the bare mark, on either path.  A store that survives the operation
/// keeps allocating, and growth multiplies by 7/3 ([`Store::resize_store`]), so
/// a store trimmed to exactly its content pays a **2.33× resize on its very
/// next claim** — for a bound store, on the file.  The claim that trips it is
/// the most ordinary one there is: iterating a keyed collection claims its
/// key-sorted snapshot inside the store, so merely READING the collection back
/// re-grew the file the caller had just shortened (loft#710, loft#727).
///
/// An eighth, rather than a fixed reserve, because the quantity it has to
/// absorb is a function of the CONTENT: that snapshot is 4 bytes per element.
#[must_use]
pub fn slack_target(live_end: u32) -> u32 {
    live_end.saturating_add(live_end / 8)
}

/// Byte offset of LLRB left-child field within a free block.
const FL_LEFT: u32 = 4;
/// Byte offset of LLRB right-child field within a free block.
const FL_RIGHT: u32 = 8;
/// Byte offset of LLRB color flag within a free block (1 = red, 0 = black).
const FL_COLOR: u32 = 12;

/// Internal space-utilisation snapshot of a single store — actual
/// claimed data vs free space, and how fragmented the free space is.
/// Produced by [`Store::usage`] by walking the block chain.
#[derive(Default, Clone, Copy)]
pub struct StoreUsage {
    /// Total store size in 8-byte words (the allocated buffer).
    pub capacity_words: u32,
    /// Sum of CLAIMED record sizes (the actual live data).
    pub claimed_words: u32,
    /// Number of claimed records.
    pub claimed_count: u32,
    /// Sum of FREE block sizes (reclaimable / fragmentation).
    pub free_words: u32,
    /// Number of distinct free blocks (the "free structure" elements).
    pub free_count: u32,
    /// Largest single free block (words) — a low value with high
    /// `free_words` means the free space is fragmented.
    pub largest_free_words: u32,
    /// Number of ADJACENT free-block pairs found during the walk — two
    /// consecutive free blocks that are physical neighbours and could be
    /// coalesced into one bigger gap.  `delete` is supposed to merge
    /// adjacent free blocks, so a non-zero count flags a missed merge
    /// (coalescing gap / fragmentation that better detection could
    /// reclaim).
    ///
    /// NOTE: this is a FIRST-CUT detector — it only catches free blocks
    /// that are immediately consecutive in the linear block walk.  Finer
    /// coalescing analysis (merge opportunities the LLRB free tree could
    /// realise, near-but-not-adjacent gaps, etc.) is expected to develop
    /// here; the metric and its plumbing are deliberately kept simple so
    /// they can be extended.
    pub mergeable_free_pairs: u32,
    /// The word just past the LAST CLAIMED block — the store's high-water
    /// mark.  Everything above it is free tail.
    ///
    /// This is what decides what a persisted image costs, because the image
    /// ends here (`store_image_live_end`): `capacity - live_end` is the tail a
    /// trim already removes, and `live_end - claimed` is the interior free
    /// space only RELOCATION could recover (loft#713).  Reading the two apart
    /// is the difference between "my store has room to give back" and "my
    /// records are spread out", which look identical in `free_words` alone.
    ///
    /// **Only meaningful when [`walk_complete`](Self::walk_complete).**
    pub live_end_words: u32,
    /// Did the block walk reach the end of the store?
    ///
    /// @PLN123 A0 — the walk stops early on a zero-size header ("malformed /
    /// uninitialised tail"), and a store that is `free` or smaller than one
    /// block is never walked at all.  In both cases `live_end_words` is a LOWER
    /// bound, not the mark: it can sit below a live record the walk never
    /// reached.  Anything that would act on the mark — above all truncating to
    /// it — must refuse unless this is true, because shrinking to a too-low
    /// mark cuts live data rather than free tail.
    ///
    /// The reporting side (`store_memory`) is happy with a lower bound; the
    /// deciding side is not, and that difference is the whole reason this field
    /// exists rather than being assumed.
    pub walk_complete: bool,
}

impl StoreUsage {
    /// Fraction of capacity holding live data, 0..100.
    #[must_use]
    pub fn used_pct(&self) -> f64 {
        if self.capacity_words == 0 {
            return 0.0;
        }
        100.0 * f64::from(self.claimed_words) / f64::from(self.capacity_words)
    }
}

/// A structural change recorded on a [`Store`] while @PLN16.J edit-recording is on
/// (see `crate::database::journal`).  A `Store` method does not know its own
/// `store_nr`, so each store buffers its own changes and `Stores` drains them into the
/// unified `Journal`, tagging the store index.  `Insert` keeps only the position and
/// size — the new record's bytes are read at drain (flush); `Free` snapshots `before`
/// at delete time, since a freed block's body is repurposed instantly (probe 5a).
#[derive(Debug)]
pub enum StoreChange {
    /// A record claimed at `pos` (`size` words); replayed via `claim_at` at flush.
    Insert { pos: u32, size: u32 },
    /// A record freed; `before` is its bytes captured *before* the `delete`.
    Free { pos: u32, before: Box<[u8]> },
}

// A low-level heap store: the several flags (free / read_only /
// free_protected / borrowed) are independent state bits on the same
// allocation, not a bundle that should become an enum.
#[allow(clippy::struct_excessive_bools)]
pub struct Store {
    // format 0 = SIGNATURE, 4 = free_space_index, 8 = record_size, 12 = content
    pub ptr: *mut u8,
    claims: HashSet<u32>,
    size: u32,
    #[cfg(feature = "mmap")]
    file: Option<MmapStorage>,
    /// @PLN126 — how far [`Store::release_resident`] has already flushed and dropped,
    /// in BYTES from the mapping's base.  Residency bookkeeping, not content: it never
    /// reaches an image, and a clone starts at zero because its pages are its own.
    ///
    /// Read only where [`Store::release_resident`] has a body to be — without `mmap`
    /// there is no file to flush to, and off unix there is no `madvise`.
    #[cfg_attr(not(all(feature = "mmap", unix)), allow(dead_code))]
    released_bytes: u64,
    /// @PLN126 — the word past the highest block ever claimed, carried forward by
    /// [`Store::claim_block`].
    ///
    /// A monotone UPPER bound on [`StoreUsage::live_end_words`], not a replacement for
    /// it: freeing the top block lowers the real mark and never lowers this.  That is
    /// the safe direction for its one consumer — flushing and dropping a few free
    /// pages above the live end costs a page fault if the allocator comes back for
    /// them, where a mark that is too LOW would drop nothing and silently buy nothing.
    /// Anything that needs the exact mark (`shrink_to`, `reclaim_tail`, `bind_path`)
    /// still reads the chain, because truncating to an upper bound would cut live data.
    ///
    /// `0` means "not established yet" — a store adopted from an image has claims this
    /// process never made, so the first reader seeds it from one chain walk.
    ///
    /// Maintained on every target and read only where [`Store::release_resident`] has a
    /// body to be: the cost is one `max` per claim, and making the bookkeeping itself
    /// conditional would mean a store whose mark depends on how loft was compiled.
    #[cfg_attr(not(all(feature = "mmap", unix)), allow(dead_code))]
    claimed_end: u32,
    pub(crate) free: bool,
    /// HARD lock: when `true`, the store is immutable.  All `addr_mut`,
    /// `claim`, and `delete` calls panic.  Set by CONST_STORE init
    /// (`compile.rs`), the JSON null sentinel (`native.rs`), and worker
    /// store borrows (`clone_locked` / `borrow_locked_for_light_worker`).
    /// Cleared via `unlock()`.
    pub read_only: bool,
    /// SOFT lock: when `true`, only `delete` is illegal — `addr_mut`
    /// and `claim` are still allowed.  Set by the fn-call deep-copy
    /// bracket (`Stores::lock_store(&r)` from
    /// `n_set_store_lock(r, true)`) to mark a caller's arg as
    /// PROTECTED-FROM-FREE for the duration of the call, so
    /// `OpCopyRecord`'s `0x8000` free-source branch skips the free
    /// when the callee returned a borrowed view of an arg.  Doesn't
    /// block scratch allocations made by the callee (e.g.
    /// `build_hash_sorted_vec` claiming sort scratch in the hash's
    /// own store per C60).  Cleared via `Stores::unlock_store(&r)`
    /// from `n_set_store_lock(r, false)`.  See @P290 for the
    /// rationale; replaced the prior origin-string discriminator.
    /// NESTING DEPTH, not a flag: one call bracket inside another may protect the
    /// SAME store (`b = outer(a)` whose body binds `c = inner(a)`), and a boolean
    /// let the inner `clear` release the outer bracket's protection — after which
    /// the outer copy's source-free would free the caller's own argument.  Depth
    /// makes the brackets nest: protection lifts when the LAST one closes.
    pub free_protect_depth: u32,
    /// Root of the LLRB free-space tree (0 = empty).
    /// Populated lazily: `open()` calls `fl_rebuild()`; `new()` starts empty
    /// and the tree fills as blocks are freed.
    free_root: u32,
    /// P6: set by `delete` whenever a free block is produced; cleared by
    /// `coalesce_free`.  `claim` runs the lazy coalescing sweep only when
    /// this is set (something was freed since the last sweep), so an
    /// alloc-only workload never pays for a fruitless O(n) pass.  A single
    /// flag — NOT an index; it does not grow with the free-block count.
    needs_coalesce: bool,
    /// CO1.9/S28: monotonic counter incremented on every `claim`, `resize`, and `delete`.
    /// Saved into `CoroutineFrame` at yield; compared at resume to detect store mutations
    /// that may have invalidated `DbRef` locals held by the generator.  Always compiled in
    /// (was debug-only before CO1.9) so the guard fires in release builds too.
    pub generation: u32,
    /// @PLN16.J — when `Some`, the structural ops (`claim` via `claim_block`, `delete`)
    /// append a [`StoreChange`] here for the debugger's edit journal.  `None` on every
    /// normal run, so the cold alloc paths pay one branch.  Turned on by
    /// `Stores::start_recording`, drained by `Stores::take_journal`.
    recording: Option<Vec<StoreChange>>,
    /// Plan-57 store-identity gate (verification builds only): the allocation-site
    /// id written by `OpStoreTag` and verified by `OpFreeRefTag`.  `0` = untagged
    /// (no verification emitted for this store).  Catches wrong-store / cross-owner
    /// frees that `free_named` otherwise silently no-ops.  Set/checked only when the
    /// gated IR post-pass emits the tagged ops; inert in normal builds.
    pub tag: u32,
    /// When true, this Store borrows another's buffer — `Drop` must NOT dealloc.
    borrowed: bool,
    /// Which SLOT this store occupies in `Stores` — its `store_nr`, the first field of every
    /// `DbRef` that names it.
    ///
    /// @PLN154 phase 3.  A `Store` method does not otherwise know its own number, which is
    /// why the edit journal buffers changes per store and lets `Stores` tag them at the
    /// drain.  The relocation log cannot do that: a record that MOVES invalidates every
    /// reference that named it, and a reference is `(store_nr, rec, pos)` — so the number
    /// has to travel with the move, not be attached at a later drain.
    ///
    /// `u16::MAX` until the slot goes live.
    pub store_nr: u16,
    /// Which allocation this slot is, on `Stores`' monotonic counter.
    ///
    /// Slot NUMBERS are reused, so `store_nr` alone cannot answer *"is this the same store I
    /// saw earlier?"* — and two questions in the fn-ref return path need exactly that: whether
    /// a returned store was minted DURING a call (`State::release_fnref_bufs`, loft#1185), and
    /// whether a remembered handle still names what it named when it was remembered. The
    /// counter never repeats within a run, so a comparison against a snapshot answers both.
    ///
    /// Stamped where the slot goes live, beside `created_at`, which records WHERE rather than
    /// WHEN.
    pub alloc_serial: u64,
    /// Bytecode position that allocated this store (via OpDatabase).
    /// Used for diagnostics when a store is leaked at program exit.
    pub created_at: u32,
    /// Bytecode position of the last significant operation on this store
    /// (OpCopyRecord, OpFreeRef skip, ref_count change, etc.).
    pub last_op_at: u32,
    /// Plan-57 Phase C: const/global stores are PINNED — `free_named` never
    /// frees them (they live for the whole program).  Replaced the Stores
    /// ref-count (deleted) as the only per-store free gate.
    pub pinned: bool,
    /// Plan-22 02d-vii follow-up — identifier of the call site that
    /// most recently locked this store.  Empty when the store is
    /// unlocked or was locked without an origin (legacy callers).
    /// Surfaced in panic messages from `addr_mut` / `claim` / `delete`
    /// so a "Write to read-only store" failure points directly at the
    /// locker rather than requiring `LOFT_LOG=locks` to re-trace.
    pub lock_origin: String,
    /// P259 — type-id of the loft type whose root record lives at
    /// `(rec=1, pos=8)` of this store.  `u16::MAX` when unknown
    /// (raw stores not allocated through `database_named`).
    /// Read by `Stores::free_named` to gate the cascade-free walk on
    /// closure records (type name starts with `__closure_`) —
    /// see commit 4 of the P259 fix.
    ///
    /// WRITE it through [`Store::set_known_type`], which moves the store's bytes to
    /// the new type in the memory-ceiling accounting.  Assigning the field directly
    /// still compiles and stays sound — the ceiling itself counts bytes, not types —
    /// but it leaves those bytes filed under the previous type in the
    /// [`breakdown`](crate::store_budget::breakdown) a refused growth prints, which
    /// is the one place they have to be right.
    pub known_type: u16,
    /// @PLAN38 phase 01 — when `Some(path)`, this store was opened
    /// via `Store::open_durable` and on clean drop must (1) flush
    /// the mmap, (2) compute the payload CRC, and (3) rewrite the
    /// `<path>.dmeta` sidecar atomically.  Cleared (left `None`) on
    /// every non-durable constructor (`new`, `open`, `clone_locked*`,
    /// `borrow_locked_*`, `new_freed_sentinel`).  See
    /// `doc/claude/plans/43-loft-store-durable/`.
    #[cfg_attr(not(feature = "mmap"), allow(dead_code))]
    durable_meta_path: Option<std::path::PathBuf>,
    /// Where this store's FILE lives, for a store mapped by [`Store::open`].
    ///
    /// Distinct from `durable_meta_path`, which records that the store was
    /// opened through the durable API — a Rust entry point no loft program
    /// reaches.  The loft-level durability surface is path-based
    /// (`store_durable_seal(path)` / `store_durable_check(path)`), so a store
    /// can have a live `.dmeta` sidecar beside it while `durable_meta_path` is
    /// `None`.  Anything that must ask "is a sidecar recording MY bytes" has to
    /// ask about the file, not about how the store was opened
    /// ([`Self::has_durable_sidecar`]).
    #[cfg_attr(not(feature = "mmap"), allow(dead_code))]
    file_path: Option<std::path::PathBuf>,
    /// @PLAN38 phase 01 — durability-mode tier on this store.  Tracks
    /// which durability variant the consumer opted into so the Drop
    /// path can apply tier-specific shutdown logic.  `0` for non-durable
    /// stores; `1` for `IntegrityOnly`.  Tiers `2` (`SnapshotEvery`)
    /// and `3` (`WAL`) are reserved for future phases.
    #[cfg_attr(not(feature = "mmap"), allow(dead_code))]
    durable_tier: u16,
    /// @PLN154 — the stack shadow: one tag word per store byte, `0` meaning nothing has
    /// written it.
    ///
    /// Empty on every store but the interpreter's value stack, and empty there too unless
    /// `LOFT_VERIFY_STACK` armed it — so the hook in [`addr_mut`](Self::addr_mut) is a
    /// length load and a not-taken branch on an ordinary run.  It lives HERE, beside the
    /// bytes it describes, because a shadow kept anywhere else has to be told about every
    /// growth and every move by hand, and phase 0 measured 33 separate routes that write
    /// these bytes.
    init_shadow: Vec<u32>,
}

impl Debug for Store {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("Store[{}]", self.size))
    }
}

impl PartialEq for Store {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl Drop for Store {
    fn drop(&mut self) {
        // borrowed stores share another Store's buffer — do not free.
        if self.borrowed {
            return;
        }
        #[cfg(feature = "mmap")]
        if self.file.is_some() {
            // @PLAN38 phase 01 — for durable stores, flush mmap + rewrite
            // sidecar atomically BEFORE the mmap_storage handle's own Drop
            // runs and closes the file.  Failure to flush is logged to
            // stderr (panicking from Drop would abort the process); a
            // failed sidecar write leaves the on-disk state without a
            // valid clean-close marker → next open detects corruption →
            // callback fires.  By design.
            if self.durable_meta_path.is_some() && !self.read_only {
                if let Err(e) = self.flush_durable_sidecar() {
                    crate::loft_eprintln!(
                        "store_durable: clean-close sidecar write failed: {e}; \
                         next open will treat the store as corrupt"
                    );
                }
            }
            return;
        }
        let l = Layout::from_size_align(self.size as usize * 8, 8).expect("Problem");
        unsafe { A.dealloc(self.ptr, l) };
        crate::store_budget::release(self.known_type, self.size as usize * 8, self.created_at);
    }
}

#[allow(dead_code)]
/// The bytes of a store image, wherever THIS target keeps its files.
///
/// `std::fs` is the whole story on every native target, and the host-FS arm never runs
/// there.  It exists for `wasm32-unknown-unknown`, which has no filesystem at all: a
/// browser page's own tree is reachable only through the `loft_host_fs_*` bridge, so a
/// page reading a pack it carries in `globalThis.loftBaseFS` arrives here with
/// `std::fs` failing and the host FS holding the file.  Without it a `--html` page could
/// not read a store at ALL — `store_load` answered `false` for every path, politely and
/// with nothing to act on (@PLN146 F4).
pub(crate) fn image_bytes(path: &str) -> Option<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(_) => host_image_bytes(path),
    }
}

/// Is there an image at `path` of at least `min` bytes, on either filesystem?
///
/// The cheap existence probe `load_path` gates on.  On a native target it is one
/// `metadata` call and never reads the file; on wasm it costs a read, because the host
/// FS answers bytes rather than sizes — and the caller then reads it again through
/// [`image_bytes`].  Two reads of an embedded pack is worth the single code path, and a
/// page carrying its own assets holds them in memory either way.
pub(crate) fn image_at_least(path: &str, min: u64) -> bool {
    if let Ok(meta) = std::fs::metadata(path) {
        return meta.len() >= min;
    }
    host_image_bytes(path).is_some_and(|b| b.len() as u64 >= min)
}

/// The host filesystem's answer, on the targets that have one.  `None` everywhere else,
/// so the two helpers above read as one path rather than as a pair of cfg forks.
fn host_image_bytes(path: &str) -> Option<Vec<u8>> {
    #[cfg(host_fs)]
    {
        crate::wasm::host_fs_read_binary(path)
    }
    #[cfg(not(host_fs))]
    {
        let _ = path;
        None
    }
}

impl Store {
    /// True when this store's memory IS a memory-mapped file
    /// (`store_persist_bind`) — its bytes are DURABLE state.
    #[must_use]
    pub fn is_file_backed(&self) -> bool {
        #[cfg(feature = "mmap")]
        {
            self.file.is_some()
        }
        #[cfg(not(feature = "mmap"))]
        {
            false
        }
    }

    /// @PLN154 — arm the stack shadow on this store.
    ///
    /// From here on every write through [`addr_mut`](Self::addr_mut) tags the bytes it
    /// covers, a byte-for-byte move carries the tags with the bytes, and
    /// [`shadow_written`](Self::shadow_written) can answer whether anything ever wrote a
    /// span.  Called on the interpreter's value-stack store only, and only when
    /// `LOFT_VERIFY_STACK` is set.
    ///
    /// The store's existing bytes are tagged UNWRITTEN, which is what they are: arming
    /// happens at `State::new`, before the first frame exists.
    pub fn arm_init_shadow(&mut self) {
        self.init_shadow = vec![0u32; self.size as usize * 8];
        crate::stack_verify::note_armed();
    }

    /// Pack a byte's tag: the kind and width the value was written at, and where in that
    /// value this byte sits.
    ///
    /// The index is what makes a read starting INSIDE a value distinguishable from one
    /// starting at its base, which is the difference between *"the slot holds another type"*
    /// and *"the two sides disagree about where the slot begins"*.  Both are findings and
    /// they want different words.
    #[inline]
    const fn pack_tag(kind: u16, idx: usize) -> u32 {
        // Beyond 254 bytes the index saturates: only an opaque raw span is that wide, and
        // an opaque tag is admitted at any offset anyway.
        let idx = if idx > 254 { 254 } else { idx };
        ((kind as u32) << 8) | (idx as u32 + 1)
    }

    /// Is the @PLN154 shadow armed on this store?
    #[inline]
    #[must_use]
    pub fn shadow_armed(&self) -> bool {
        !self.init_shadow.is_empty()
    }

    /// Tag `len` bytes at absolute byte offset `at` as written.
    ///
    /// `at` is the offset from the store's base — the `rec * 8 + fld` that
    /// [`addr_mut`](Self::addr_mut) computes — so a tag and a byte are always in one
    /// coordinate system.
    ///
    /// Under `LOFT_VERIFY_STACK_INJECT` this does nothing, which makes every checked read
    /// report: the positive control that tells a detector that cannot fire from a corpus
    /// that is clean.
    /// Outlined and `cold`: the branch that guards it in [`addr_mut`](Self::addr_mut) is
    /// never taken on an ordinary run, and inlining the body there pushed the hot path's
    /// code apart for nothing — measured at +10 % instructions on a field-write loop before
    /// this attribute, and under 1 % after.
    #[cold]
    #[inline(never)]
    pub fn shadow_write(&mut self, at: usize, len: usize, kind: u16) {
        if crate::stack_verify::inject() {
            return;
        }
        let end = (at + len).min(self.init_shadow.len());
        for (i, slot) in self.init_shadow[at.min(end)..end].iter_mut().enumerate() {
            *slot = Self::pack_tag(kind, i);
        }
    }

    /// Drop the tags on `len` bytes at `at`: the span has left the live frame, and the
    /// next occupant of those offsets inherits nothing from the last.
    #[cold]
    #[inline(never)]
    pub fn shadow_kill(&mut self, at: usize, len: usize) {
        if self.init_shadow.is_empty() {
            return;
        }
        let end = (at + len).min(self.init_shadow.len());
        if at < end {
            self.init_shadow[at..end].fill(0);
        }
    }

    /// What does the shadow say about `at..at + len`?
    ///
    /// The three answers are the states the plan separates.  `Unwritten` — no byte of the
    /// span was ever written — is @PLN154 phase 1's finding, and it is the one that means
    /// the slot has no occupant at all.  `Partial` is a slot whose occupant is NARROWER
    /// than the read: an eval slot is stepped to eight bytes
    /// ([`aligned_stack_step`](crate::variables::aligned_stack_step)) while a `boolean`
    /// writes one and a `character` four, so every sub-word scalar leaves padding nobody
    /// wrote.  Whether the read's width matches the write's is phase 2's question, and
    /// answering it here would report the language's own stepped slots on day one.
    ///
    /// A span outside the shadow answers `Written` and is counted in the summary: that is
    /// a fault in the shadow's own plumbing rather than a finding about the program, and
    /// reporting it as one would bury the real findings.
    #[must_use]
    pub fn shadow_state(&self, at: usize, len: usize, kind: u16) -> crate::stack_verify::SlotState {
        use crate::stack_verify::{OPAQUE, SlotState, kind_width};
        if self.init_shadow.is_empty() {
            return SlotState::Written;
        }
        let Some(span) = self.init_shadow.get(at..at + len) else {
            crate::stack_verify::note_out_of_range();
            return SlotState::Written;
        };
        let base = span[0];
        if base == 0 {
            return SlotState::Unwritten;
        }
        let wrote = (base >> 8) as u16;
        // A handle whose record has moved: the tag kept its width and index and changed only
        // its family, so it lands here rather than in a second lookup.
        if wrote & 0xFF00 == crate::stack_verify::STALE {
            return SlotState::Stale;
        }
        // A raw byte BLOCK on either side is opaque: whoever assembled it knows what is in
        // it, and the shadow's tag does not.  The fn-ref slot is written as
        // `[MaybeUninit<u8>; 20]` and read back as an `i64` and a `DbRef`; a coroutine frame
        // is restored from its own snapshot.  Those are composite reads, not disagreements.
        if wrote == OPAQUE || kind == OPAQUE {
            return SlotState::Written;
        }
        if wrote == kind && base & 0xFF == 1 {
            return SlotState::Written;
        }
        // THE FINDING IS A HANDLE CROSSING A VALUE, in either direction.  A `DbRef` is a
        // position in a store, so reading one as a number answers an address wearing a
        // number's clothes, and reading a number as one indexes a store by data — and
        // neither can be an intended pun, because a slot the compiler typed as a reference
        // is a reference on every path.  That is the monomorph-layout class this phase was
        // opened for: loft#1028 wrote `OpNullRefSentinel`'s twelve bytes into a slot the
        // monomorph read as an `integer`, and loft#1016 defaulted a `__typevar_T` RECORD
        // into one it read as a float.
        //
        // The WIDTH comparison is deliberately not a finding, and that is measured rather
        // than conceded: the interpreter's frame carries composite slots the compiler
        // addresses field by field — the 20-byte fn-ref slot, the iterator state `OpStep`
        // reads as two `u32`s — so a width rule reports the frame's own layout on 43 of the
        // first 180 corpus programs.  What survives is counted as a pun, not reported.
        let crossing = (wrote >> 8 == 1) != (kind >> 8 == 1);
        if crossing {
            return SlotState::Mismatch { wrote };
        }
        // The remaining disagreements are the language's own puns: a scalar narrower than
        // its stepped slot (`OpSizeScalar` reads eight bytes to discard a `boolean`), a null
        // sentinel written as the word it is and read as the float it stands for, and a
        // composite slot read one field at a time.
        let _ = kind_width(wrote);
        SlotState::Partial
    }

    /// Read the tags of `at..at + len`, for a move that has to carry them.
    #[must_use]
    pub fn shadow_tags(&self, at: usize, len: usize) -> Vec<u32> {
        self.init_shadow
            .get(at..at + len)
            .map_or_else(|| vec![0u32; len], <[u32]>::to_vec)
    }

    /// Write `tags` over the span starting at `at`.
    pub fn shadow_set_tags(&mut self, at: usize, tags: &[u32]) {
        let end = (at + tags.len()).min(self.init_shadow.len());
        if at < end {
            self.init_shadow[at..end].copy_from_slice(&tags[..end - at]);
        }
    }

    /// Mark the `len`-byte value based at `at` as a STALE handle: it names a record that has
    /// moved.  Keeps the width and the index, so a later read reports the handle rather than
    /// a width disagreement.
    pub fn shadow_stale(&mut self, at: usize, len: usize) {
        let end = (at + len).min(self.init_shadow.len());
        for slot in &mut self.init_shadow[at.min(end)..end] {
            if *slot != 0 {
                *slot = (*slot & 0xFF) | (u32::from(crate::stack_verify::STALE) << 8);
            }
        }
    }

    /// Does a live HANDLE begin at byte `at`?
    ///
    /// @PLN154 phase 3's scan asks exactly this and nothing else, and it asks it about every
    /// four-byte offset in the frame — so the tag's encoding is decoded HERE, beside
    /// [`pack_tag`](Self::pack_tag) that wrote it, rather than open-coded at the caller where
    /// a change to the layout would leave a silently-never-matching test behind.
    #[must_use]
    pub fn shadow_handle_base_at(&self, at: usize) -> bool {
        let Some(&tag) = self.init_shadow.get(at) else {
            return false;
        };
        // index 0 within the value (stored as 1), and the handle family.
        tag & 0xFF == 1 && (tag >> 8) as u16 >> 8 == 1
    }

    /// Move the tags of a `len`-byte span from `from` to `to`, within this store.
    ///
    /// The SOURCE tags are the real ones — whoever wrote the bytes wrote them there — so
    /// an unwritten source stays unwritten at the destination, which is the property that
    /// lets a callee's missing return value be caught at the caller's read.
    pub fn shadow_move(&mut self, from: usize, to: usize, len: usize) {
        if self.init_shadow.is_empty() {
            return;
        }
        let tags = self.shadow_tags(from, len);
        self.shadow_set_tags(to, &tags);
    }

    /// Total capacity of this store in bytes.
    #[must_use]
    pub fn byte_capacity(&self) -> u64 {
        u64::from(self.size) * 8
    }

    /// Total capacity of this store in 8-byte words.
    #[must_use]
    pub fn capacity_words(&self) -> u32 {
        self.size
    }

    /// @P294: grow this store's backing buffer to at least `words` words,
    /// in place — no record relocation, so any `(store_nr, rec, pos)`
    /// `DbRef` into this store stays valid (only the raw `ptr` moves, and
    /// every access re-derives it).  No-op when already large enough.
    /// Used by the interpreter to extend the value-stack store (#0) on
    /// deep call nesting, where the stack writes bypass `claim` and so
    /// never trigger the normal growth path.
    pub fn grow_words(&mut self, words: u32) {
        self.resize_store(words);
    }

    /// loft#935 — after [`grow_words`], make the PRIMARY record span the store
    /// it grew into.
    ///
    /// The interpreter's value stack lives inside record 1 of store #0, claimed
    /// once at `State::new` and then written directly: the frame slots bypass
    /// `claim`, so `grow_words` is what extends the buffer when a program nests
    /// deeply enough. It extends the BUFFER only, and record 1's header still
    /// claims the size it was born with — so every stack byte above that mark
    /// sits outside the record that is supposed to own it. `Store::valid` says
    /// so under debug assertions (`Fld 8004 is outside of record 1 size 8000`),
    /// and in a release build the same bytes are simply written to a region the
    /// store's own accounting believes nobody holds.
    ///
    /// The invariant this restores is the one `State::new` established and
    /// nothing since maintained: **record 1 of the stack store spans the whole
    /// store**. It is the store's only record — nothing else ever claims there —
    /// so extending the header to the buffer's end cannot overlap anything.
    ///
    /// `Store::resize` is deliberately NOT used: it falls back to
    /// claim-copy-delete, which RELOCATES the record. Every live frame is
    /// addressed as `(0, 1, pos)`, so moving record 1 would move the running
    /// stack out from under the interpreter.
    ///
    /// Grow-only, and a no-op on a store whose primary is already at its
    /// extent, so it is safe to call after every growth.
    pub fn extend_primary_to_store_end(&mut self) {
        let span = self.size - PRIMARY;
        let claimed: i32 = *self.addr::<i32>(PRIMARY, 0);
        // A negative header means record 1 is a FREE block — an uninitialised or
        // reset store, which the stack store never is once `State::new` has
        // claimed it. Leave it alone rather than forging a live header.
        if claimed > 0 && (claimed as u32) < span {
            *self.addr_mut::<i32>(PRIMARY, 0) = span as i32;
        }
    }

    /// Raw base pointer to the store's memory buffer.
    #[must_use]
    pub fn base_ptr(&self) -> *mut u8 {
        self.ptr
    }

    pub fn new(size: u32) -> Store {
        // `init()` writes record 1's header at byte offset 8, so the
        // backing buffer must hold at least 2 words.  Smaller sizes
        // produce an OOB write that Linux's allocator slack tolerates
        // but Windows catches at deallocation as STATUS_HEAP_CORRUPTION
        // (0xc0000374).
        let size = size.max(2);
        let l = Layout::from_size_align(size as usize * 8, 8).expect("Problem");
        let ptr = unsafe { A.alloc_zeroed(l) };
        // A fresh store has no type yet — `set_known_type` moves these bytes across
        // when `database_named` names it.
        crate::store_budget::add(u16::MAX, size as usize * 8, 0);
        let mut store = Store {
            ptr,
            size,
            claims: HashSet::new(),
            #[cfg(feature = "mmap")]
            file: None,
            free: true,
            read_only: false,
            free_protect_depth: 0,
            borrowed: false,
            store_nr: u16::MAX,
            alloc_serial: 0,
            created_at: 0,
            last_op_at: 0,
            free_root: 0,
            needs_coalesce: false,
            released_bytes: 0,
            claimed_end: 0,
            generation: 0,
            recording: None,
            tag: 0,
            pinned: false,
            lock_origin: String::new(),
            known_type: u16::MAX,
            durable_meta_path: None,
            file_path: None,
            durable_tier: 0,
            init_shadow: Vec::new(),
        };
        store.init(); // sets claims = {PRIMARY} and free_root = 0
        store
    }

    /// A standalone, immediately-usable store for tests and fuzzing.
    ///
    /// `new` returns a store flagged `free` (the database layer clears that
    /// when it registers the store into a `Stores`); without a `Stores` the
    /// store's own `validate()` rejects it as "freed".  This mirrors what the
    /// in-crate unit tests do by hand (`store.free = false`) and gives external
    /// callers (the fuzz harness) a clean entry point.
    #[doc(hidden)]
    #[must_use]
    pub fn new_in_use(size: u32) -> Store {
        let mut store = Store::new(size);
        store.free = false;
        store
    }

    /// Do these leading bytes carry the store [`SIGNATURE`]?
    ///
    /// The one place that answers it, because it is asked from two very different
    /// distances: the startup cache holds the whole file, and a PAGED source holds
    /// only the four bytes it range-read.  A source that opens and is not an image
    /// used to reach neither check — `PageSource::open` validated nothing, so an empty
    /// file, a truncated download, a directory or an HTTP 200 serving an error page
    /// bound successfully and answered `null` for every key with the fault channel
    /// reporting healthy (loft#994).
    ///
    /// Fewer than four bytes is not an image either — an empty file is the shortest
    /// way to get here.
    ///
    /// `SIGNATURE` is native-endian: an image is never read on an architecture other
    /// than the one that wrote it (the startup cache is keyed by target triple, and a
    /// paged read is refused by the `.dschema` layout gate).
    #[must_use]
    pub fn has_signature(head: &[u8]) -> bool {
        head.len() >= 4 && u32::from_ne_bytes([head[0], head[1], head[2], head[3]]) == SIGNATURE
    }

    /// @PLN11 arc E — cheap validity pre-check for a store image file: it
    /// exists, is large enough, and starts with the store [`SIGNATURE`].  Lets
    /// the startup cache reject a corrupt / non-store / truncated bundle and
    /// fall back to a cold parse instead of letting [`Store::open`] panic on a
    /// bad signature.  (`SIGNATURE` is written native-endian; a cache is never
    /// shared across architectures — it is keyed by target triple.)
    #[cfg(feature = "mmap")]
    #[must_use]
    pub fn is_store_file(path: &str) -> bool {
        use std::io::Read as _;
        let Ok(mut f) = std::fs::File::open(path) else {
            return false;
        };
        if f.metadata().map_or(0, |m| m.len()) < 16 {
            return false;
        }
        let mut buf = [0u8; 4];
        f.read_exact(&mut buf).is_ok() && Self::has_signature(&buf)
    }

    #[cfg(not(feature = "mmap"))]
    pub fn open(_path: &str) -> Store {
        panic!(
            "mmap feature is not compiled in; enable the `mmap` Cargo feature to use file-backed stores"
        )
    }

    #[cfg(feature = "mmap")]
    pub fn open(path: &str) -> Store {
        let mut file = MmapStorage::open(path).expect("Opening file");
        let init = if (file.capacity() / 8) < MIN_BOUND_WORDS as usize {
            file.resize(8192).unwrap();
            true
        } else {
            false
        };
        // @PLAN38 phase 01 — `size` MUST be read AFTER the resize.  The
        // previous code captured it from pre-resize capacity, so for a
        // freshly-created file (`MmapStorage::open` initialises at 1 byte
        // → resized to 8192) the Store struct received `size = 0` while
        // the buffer was 8192 bytes.  `init()` then wrote the record-1
        // header using the bad size → on re-open, `fl_rebuild()` walked
        // garbage block headers and hit `block_size = 0` → infinite loop
        // in release builds.
        let size = (file.capacity() / 8) as u32;
        let ptr = std::ptr::addr_of!(file.as_slice()[0]).cast_mut();
        let mut store = Store {
            file: Some(file),
            // Recorded so the store can answer "is a `.dmeta` sidecar recording
            // MY bytes" — see `has_durable_sidecar`.
            ptr,
            claims: HashSet::new(),
            size,
            // An opened FILE-BACKED store is in use by definition (it
            // carries real data and `open` itself validates it below,
            // before any `adopt_store` registration could clear the
            // flag) — unlike `Store::new`, whose blank store stays
            // `free` until the database layer registers it.
            free: false,
            read_only: false,
            free_protect_depth: 0,
            free_root: 0,
            needs_coalesce: false,
            released_bytes: 0,
            claimed_end: 0,
            generation: 0,
            recording: None,
            tag: 0,
            borrowed: false,
            store_nr: u16::MAX,
            alloc_serial: 0,
            created_at: 0,
            last_op_at: 0,
            pinned: false,
            lock_origin: String::new(),
            known_type: u16::MAX,
            durable_meta_path: None,
            file_path: Some(std::path::PathBuf::from(path)),
            durable_tier: 0,
            init_shadow: Vec::new(),
        };
        if init {
            store.init();
        } else {
            assert_eq!(
                unsafe { store.ptr.cast::<u32>().read_unaligned() },
                SIGNATURE,
                "Unknown file format"
            );
            #[cfg(debug_assertions)]
            store.validate(0);
            store.fl_rebuild();
            store.claims_rebuild();
        }
        store
    }

    /// Load a persisted store IMAGE FILE fully into a fresh HEAP-backed store —
    /// the portable, always-available counterpart of [`open`](Store::open)
    /// (which mmaps).  The bytes are copied into an owned heap arena
    /// (`file: None`), so the result is a normal self-contained store: it works
    /// on **every** target (wasm has no mmap) and is **not** durable — writes
    /// stay in memory and never touch the file.  @PLN97 arc G Phase 1
    /// ([#522](https://github.com/loft-lang/loft/issues/522)) — the whole-file
    /// load the wasm / browser working-set path builds on.  Panics on a
    /// missing / truncated / wrong-signature file (a `read`, an under-16-byte
    /// image, or a bad `SIGNATURE`); callers that want a clean reject wrap it in
    /// `catch_unwind` (see `Stores::load_path`).
    pub fn load(path: &str) -> Store {
        let bytes = image_bytes(path).expect("store load: cannot read file");
        assert!(
            bytes.len() >= 16,
            "store load: file too small to be a store image"
        );
        let sig = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!(sig, SIGNATURE, "store load: unknown file format");
        // Round the byte length up to a whole 8-byte word; `alloc_zeroed`
        // zero-fills any trailing partial word.
        let words = bytes.len().div_ceil(8).max(2) as u32;
        assert!(
            words <= MAX_STORE_WORDS,
            "store load: image exceeds the {MAX_STORE_WORDS}-word store limit"
        );
        let l = Layout::from_size_align(words as usize * 8, 8).expect("Problem");
        let ptr = unsafe { A.alloc_zeroed(l) };
        crate::store_budget::add(u16::MAX, words as usize * 8, 0);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        }
        let mut store = Store {
            ptr,
            size: words,
            claims: HashSet::new(),
            #[cfg(feature = "mmap")]
            file: None,
            // A loaded store carries real data (like `open`), so it is in use.
            free: false,
            read_only: false,
            free_protect_depth: 0,
            borrowed: false,
            store_nr: u16::MAX,
            alloc_serial: 0,
            created_at: 0,
            last_op_at: 0,
            free_root: 0,
            needs_coalesce: false,
            released_bytes: 0,
            claimed_end: 0,
            generation: 0,
            recording: None,
            tag: 0,
            pinned: false,
            lock_origin: String::new(),
            known_type: u16::MAX,
            durable_meta_path: None,
            file_path: None,
            durable_tier: 0,
            init_shadow: Vec::new(),
        };
        #[cfg(debug_assertions)]
        store.validate(0);
        store.fl_rebuild();
        store.claims_rebuild();
        store
    }

    /// Adopt an in-memory byte buffer as a store — the slice counterpart of
    /// [`Store::load`] (a heap copy, no mmap / no file handle, so it works on
    /// every backend incl. wasm).  Returns `None` (a clean reject, never a
    /// panic) on a buffer too small for the header, a wrong `SIGNATURE`, or an
    /// over-limit size.  Used by the verified HTTP store loader
    /// ([`crate::database::Stores::load_url_verified`], @PLN97 arc G Phase 0):
    /// the bytes are fetched + authenticity-verified in memory and only then
    /// adopted here, so an untrusted body never touches disk.
    ///
    /// Unlike `load` (the trusted local path, which structurally validates only
    /// in debug), `from_bytes` runs [`Store::validate_structure`] **always-on and
    /// BEFORE `fl_rebuild`** (@PLN97 arc G Phase 2): a crafted buffer with a
    /// zero-size block (the `fl_rebuild` release infinite-loop hazard) or a record
    /// claiming a size past the arena (a heap over-read) is rejected here with
    /// `None`, never walked.  Interior `DbRef` soundness remains the caller's
    /// `store_verify` backstop.
    /// @PLN14 arc G — how many records are currently CLAIMED in this store.
    ///
    /// The right instrument for the re-bind growth guard: the arena is pre-sized,
    /// so its byte size stays flat whether or not orphans are released, and
    /// measuring that proves nothing.  `claims` is the live-record set — `claim`
    /// inserts, `delete` removes — so a session that frees its orphans keeps this
    /// flat while one that leaks grows it once per re-bind.
    #[must_use]
    pub(crate) fn claims_count(&self) -> usize {
        self.claims.len()
    }

    /// @PLN14 arc F — this store's whole arena as raw bytes, the counterpart of
    /// [`from_bytes`](Self::from_bytes).  Used to persist the REPL's session store
    /// into a resume image.
    ///
    /// The bytes are a **host-endian raw image** (@PLN97 F9), so they are only
    /// meaningful to a build with the same layout — which is why the image that
    /// carries them is gated on `Stores::layout_algo_hash`.
    #[must_use]
    pub(crate) fn raw_bytes(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size as usize * 8) }
    }

    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Store> {
        if bytes.len() < 16 {
            return None;
        }
        let sig = u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        if sig != SIGNATURE {
            return None;
        }
        let words = bytes.len().div_ceil(8).max(2) as u32;
        if words > MAX_STORE_WORDS {
            return None;
        }
        let l = Layout::from_size_align(words as usize * 8, 8).ok()?;
        let ptr = unsafe { A.alloc_zeroed(l) };
        if ptr.is_null() {
            return None;
        }
        crate::store_budget::add(u16::MAX, words as usize * 8, 0);
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        }
        let mut store = Store {
            ptr,
            size: words,
            claims: HashSet::new(),
            #[cfg(feature = "mmap")]
            file: None,
            free: false,
            read_only: false,
            free_protect_depth: 0,
            borrowed: false,
            store_nr: u16::MAX,
            alloc_serial: 0,
            created_at: 0,
            last_op_at: 0,
            free_root: 0,
            needs_coalesce: false,
            released_bytes: 0,
            claimed_end: 0,
            generation: 0,
            recording: None,
            tag: 0,
            pinned: false,
            lock_origin: String::new(),
            known_type: u16::MAX,
            durable_meta_path: None,
            file_path: None,
            durable_tier: 0,
            init_shadow: Vec::new(),
        };
        // @PLN97 arc G Phase 2 — fail-closed structural validation of an
        // UNTRUSTED buffer, ALWAYS-ON and BEFORE `fl_rebuild`.  A crafted
        // 0-size block or an out-of-bounds record is rejected here (returning
        // `None`; dropping `store` frees the arena), so `fl_rebuild` never walks
        // garbage headers (the release infinite-loop / heap-over-read hazards).
        if store.validate_structure().is_err() {
            return None;
        }
        store.fl_rebuild();
        Some(store)
    }

    pub fn init(&mut self) {
        // The normal routines will not write to rec=0, so we write a signature: StoreV01
        unsafe {
            self.ptr.cast::<u32>().write_unaligned(SIGNATURE);
            // The first empty space
            self.ptr.add(4).cast::<u32>().write_unaligned(1);
        }
        // Indicate the complete store as empty
        *self.addr_mut(1, 0) = -(self.size as i32) + 1;
        // Reset the LLRB free-space tree and claims to match the fresh store layout.
        // Without this, a re-used store's stale tree would cause fl_take_ge to allocate
        // from old split blocks at positions other than 1, breaking the rec=1 invariant
        // relied upon by database-level code.
        self.free_root = 0;
        self.claims.clear();
        self.claims.insert(PRIMARY);
    }

    /// @P317 debug — `LOFT_LOG=zero_claim` (or `LOFT_ZERO_CLAIM=1`) zeroes
    /// every freshly-claimed record's payload, so a read-before-write or a
    /// stale-`DbRef` read picks up a deterministic `0` instead of arena slack
    /// (old record data, or the freed block's LLRB free-list pointers, which
    /// live at offset 4 — exactly where a vector's length word sits).  This
    /// turns NON-deterministic uninitialised-arena bugs — which valgrind
    /// cannot see, because the arena buffer is itself validly allocated — into
    /// deterministic, reproducible failures.  Read once (cached); zero cost
    /// when off.  See `doc/claude/DEBUG.md` § store-ownership debugging.
    fn zero_claim_enabled() -> bool {
        // Default ON: a claimed block's PAYLOAD must read as zero.  `claim` reuses freed blocks
        // WITHOUT clearing them, so a caller that relies on zero-init — e.g. an empty `[]`
        // collection placeholder (`V{a:[]}` / `parts: vector<T> = []`), which assumes the field/
        // var handle is already 0 — instead inherits the freed block's STALE bytes.  That stale
        // collection handle then resolves to a non-claimed record in `remove_claims`/`length_vector`
        // → a use-after-free SIGSEGV (135-vector-u8-concat gate-on; @PLN25).  Zeroing the payload at
        // the single claim chokepoint makes the invariant hold for every caller (interpreter only;
        // native uses Rust ownership and never hits this).  `LOFT_NO_ZERO_CLAIM` disables it for
        // perf benchmarking only.
        static FLAG: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *FLAG.get_or_init(|| std::env::var("LOFT_NO_ZERO_CLAIM").is_err())
    }

    /// Common tail of every `claim` path: zero the claimed payload when
    /// `zero_claim` is enabled (@P317 debugging lever).  No-op otherwise.
    #[inline]
    fn finish_claim(&mut self, pos: u32) -> u32 {
        if Self::zero_claim_enabled() {
            self.zero_fill(pos);
        }
        pos
    }

    /// Claim the space of a record
    /// # Arguments
    /// * `size` - The requested record size in 8 byte words
    pub fn claim(&mut self, size: u32) -> u32 {
        debug_assert!(
            !self.read_only,
            "Claim on read-only store (size={size}) (locked by: {})",
            self.lock_origin
        );
        assert!(
            !self.read_only,
            "Claim on read-only store (size={size}) (locked by: {})",
            self.lock_origin
        );
        assert!(size >= 1, "Incomplete record");
        // CO1.9/S28: increment generation so coroutine_next can detect store mutations
        // that may invalidate DbRef locals held by suspended generators.
        self.generation = self.generation.wrapping_add(1);
        #[cfg(debug_assertions)]
        self.fl_validate();
        // Fast path: find the smallest tracked free block that fits.
        if let Some(pos) = self.fl_take_ge(size as i32) {
            let result = self.claim_block(pos, size);
            #[cfg(debug_assertions)]
            self.fl_validate();
            return self.finish_claim(result);
        }
        // P6: a fast-path miss means no single tracked free block fits.
        // `delete` only merges forward, so adjacent frees that each don't
        // fit (but together would) are left uncoalesced and `claim_scan`
        // below would grow the store.  Coalesce the chain (reusing the one
        // free tree — no new index) and retry once before growing.  Guarded
        // by `needs_coalesce` so an alloc-only workload never sweeps.
        if self.needs_coalesce {
            self.coalesce_free();
            if let Some(pos) = self.fl_take_ge(size as i32) {
                let result = self.claim_block(pos, size);
                #[cfg(debug_assertions)]
                self.fl_validate();
                // This coalesce path reuses a freed block too — zero its payload like the other
                // claim paths (it previously returned WITHOUT `finish_claim`, leaking stale bytes).
                return self.finish_claim(result);
            }
        }
        // Slow path: linear scan (handles size-1 blocks and first-time allocation).
        let result = self.claim_scan(size);
        #[cfg(debug_assertions)]
        self.fl_validate();
        self.finish_claim(result)
    }

    /// Mark `pos` as claimed (splitting if the block is much larger than `size`).
    fn claim_block(&mut self, pos: u32, size: u32) -> u32 {
        let req_size = size as i32;
        let block_size = -(*self.addr::<i32>(pos, 0));
        assert!(block_size >= req_size, "Claimed block too small at {pos}");
        if block_size > req_size * 4 / 3 {
            *self.addr_mut(pos, 0) = req_size;
            let new_free = pos + size;
            *self.addr_mut(new_free, 0) = req_size - block_size; // negative = free
            self.fl_insert(new_free);
        } else {
            *self.addr_mut(pos, 0) = block_size; // positive = claimed
        }
        self.claims.insert(pos);
        // @PLN126 — carry the high-water mark forward here, where the block becomes
        // claimed, rather than deriving it from a chain walk when someone asks.
        // `Store::usage` reads the header of EVERY block, which touches every page of
        // the arena; a release call that asked it for the frontier faulted the whole
        // store back in to decide what to drop, and measured a peak RSS of the entire
        // file where the same build without the call held half of it.
        self.claimed_end = self
            .claimed_end
            .max(pos + (*self.addr::<i32>(pos, 0)) as u32);
        // @PLN16.J: record the claim while edit-recording is on.  Read the *actual*
        // claimed size from the header (claim_block may take the whole block without
        // splitting), so replay's `claim_at` reproduces the exact extent.
        if self.recording.is_some() {
            let size = (*self.addr::<i32>(pos, 0)) as u32;
            if let Some(log) = self.recording.as_mut() {
                log.push(StoreChange::Insert { pos, size });
            }
        }
        pos
    }

    /// Linear-scan fallback for `claim()`: walks from PRIMARY until a free block
    /// of the required size is found, growing the store if necessary.
    fn claim_scan(&mut self, size: u32) -> u32 {
        let req_size = size as i32;
        let mut pos = PRIMARY;
        let mut last = pos;
        let mut claim = *self.addr::<i32>(pos, 0);
        while pos < self.size && (claim >= 0 || -claim < req_size) {
            last = pos;
            pos += i32::abs(claim) as u32;
            if pos >= self.size {
                break;
            }
            debug_assert_ne!(pos, last, "Inconsistent database zero sized block {pos}");
            claim = *self.addr::<i32>(pos, 0);
            if claim == 0 {
                // A block that owns no words cannot be stepped over: `pos` stops
                // advancing and this walk never ends.  That assert above is the
                // invariant, and it is compiled OUT of every release build
                // (`[profile.dev.package.loft]` turns debug assertions off even
                // under `cargo test`), so where it matters the loop simply hung
                // — a `store_load` that never returned, with nothing on stderr
                // and no timeout of its own.
                //
                // Nothing may produce a zero-sized block; when something does,
                // say so and treat the chain as ending here.  The store grows
                // instead, so the words past the break are leaked rather than
                // walked, and the program keeps its answer.
                crate::loft_eprintln!(
                    "loft: block chain is malformed at word {pos} of {} (a block owning no \
                     words); allocating past it and leaking the remainder",
                    self.size
                );
                claim = *self.addr::<i32>(last, 0);
                pos = self.size;
                break;
            }
        }
        if pos >= self.size {
            // If the last block is free and tracked in the LLRB tree, remove it
            // before claim_grow changes its header in place.  Without this step
            // the tree retains a stale node that claim_block would later claim,
            // leaving a positive-header block reachable from free_root.
            if claim < 0 {
                self.fl_remove(last);
            }
            pos = self.claim_grow(size, last, claim);
            #[cfg(debug_assertions)]
            self.validate(0);
        }
        self.claim_block(pos, size)
    }

    /// Grow the store to accommodate `size` words and return the position of the
    /// new free block (either the extended last block or a fresh one).
    fn claim_grow(&mut self, size: u32, last: u32, last_claim: i32) -> u32 {
        let cur = self.size;
        let new_size = if last_claim < 0 {
            (self.size as i32 + size as i32 + last_claim) as u32
        } else {
            self.size.checked_add(size).unwrap_or_else(|| {
                panic!(
                    "store size limit exceeded: {} words ({} bytes)",
                    u64::from(self.size) + u64::from(size),
                    (u64::from(self.size) + u64::from(size)) * 8
                )
            })
        };
        self.resize_store(new_size);
        let increase = (self.size - cur) as i32;
        if last_claim < 0 {
            *self.addr_mut(last, 0) = last_claim - increase;
            last
        } else {
            *self.addr_mut(cur, 0) = -increase;
            cur
        }
    }

    /// Mutate the claimed size of a record
    pub fn resize(&mut self, rec: u32, size: u32) -> u32 {
        // CO1.9/S28: increment generation so coroutine_next can detect resize operations
        // that may invalidate DbRef locals (record relocation) held by suspended generators.
        self.generation = self.generation.wrapping_add(1);
        let req_size = size as i32;
        let claim = *self.addr::<i32>(rec, 0);
        if claim >= req_size {
            return rec;
        }
        let next = rec + claim as u32;
        if next < self.size {
            let next_size = *self.addr::<i32>(next, 0);
            if next_size < 0 && claim - next_size > req_size {
                // The adjacent free block can cover the growth.
                self.fl_remove(next);
                let act = req_size * 7 / 4;
                let new_size = if claim - next_size > act {
                    let new_next = rec + act as u32;
                    let new_free_size = (-next_size) as u32 + next - new_next;
                    *self.addr_mut(rec, 0) = act;
                    *self.addr_mut(new_next, 0) = -(new_free_size as i32);
                    self.fl_insert(new_next);
                    act
                } else {
                    *self.addr_mut(rec, 0) = claim - next_size;
                    claim - next_size
                };
                // The absorbed region (old end `claim` .. new end) held the freed block's
                // STALE bytes.  `claim`/`finish_claim` zero a payload on allocation so a
                // freshly-exposed slot reads as 0 (the invariant `set_default_value` and the
                // vector/text readers rely on); an in-place grow must uphold the SAME
                // invariant or a newly-exposed vector element carries garbage text/vec
                // handles that `remove_claims`/`length_vector` then follow into a UAF
                // (cluster-462, #462 @ sim.loft:3546).  Zero only the grown tail; the old
                // payload (words 1..claim) is preserved.  Same flag as `claim` so
                // `LOFT_NO_ZERO_CLAIM` toggles both together.
                if Self::zero_claim_enabled() {
                    self.zero_range(rec, claim as u32 * 8, (new_size - claim) as u32 * 8);
                }
                return rec;
            }
        }
        let new = self.claim(size);
        self.copy(rec, new);
        self.delete(rec);
        // @PLN154 phase 3 — the record MOVED, so every reference that named it is stale.
        // Recorded here rather than detected later because this is the only moment both
        // record numbers exist: afterwards the old one is a free block like any other, and
        // a frame slot still naming it is indistinguishable from one naming a slot that was
        // simply reused.
        if crate::stack_verify::enabled() {
            crate::stack_verify::note_relocation(self.store_nr, rec, new);
        }
        new
    }

    /// Delete a record, this assumes that all links towards this record are already removed
    pub fn delete(&mut self, rec: u32) {
        // `read_only` is IMMUTABILITY — CONST_STORE, workers, the user-facing
        // `d#lock` tripwire — so nothing in the store may change, deletes included.
        //
        // `free_protected` is NOT that (loft#760). It is the call bracket's "do not
        // FREE my argument", and both places that could — `do_copy_record` and
        // `replace_keyed`, the two `0x8000` source-frees — already refuse on it
        // themselves before calling `database.free`. Blocking `delete` as well was
        // strictly wider than the marker's job, and it caught the wrong thing: a
        // container the callee is legitimately mutating releases its own old block
        // when it regrows, and `AppendVector` on a passed-by-value struct's field
        // then aborted the interpreter. That is a WRITE, which the bracket exists to
        // keep legal — @P290's whole point was to stop using `lock_store` here,
        // because a callee iterating a passed-in hash field must not crash.
        //
        // Native never had this check and was always correct on the same source,
        // which is what made the `&`-redundancy advice right on one backend and
        // wrong on the other.
        let frozen = self.read_only;
        debug_assert!(
            !frozen,
            "Delete on locked store (rec={rec}) (locked by: {})",
            self.lock_origin
        );
        assert!(
            !frozen,
            "Delete on locked store (rec={rec}) (locked by: {})",
            self.lock_origin
        );
        // CO1.9/S28: increment generation so coroutine_next can detect deletions that
        // may free a record still referenced by a suspended generator.
        self.generation = self.generation.wrapping_add(1);
        self.valid(rec, 4);
        // @PLN16.J: snapshot the record *before* delete repurposes its body as a
        // free-tree node (probe 5a), while edit-recording is on.
        if self.recording.is_some() {
            let words = (*self.addr::<i32>(rec, 0)) as u32;
            let before = self.read_span(rec, 0, words * 8);
            if let Some(log) = self.recording.as_mut() {
                log.push(StoreChange::Free { pos: rec, before });
            }
        }
        let mut claim = *self.addr::<i32>(rec, 0);
        // Coalesce with any adjacent free blocks that follow.
        while (rec + claim as u32) < self.size {
            let next_pos = rec + claim as u32;
            let next_header = *self.addr::<i32>(next_pos, 0);
            if next_header >= 0 {
                break;
            }
            // Remove the about-to-be-absorbed block from the tree before merging.
            self.fl_remove(next_pos);
            claim -= next_header;
        }
        *self.addr_mut(rec, 0) = -claim;
        self.claims.remove(&rec);
        // Register the (possibly coalesced) free block in the tree.
        self.fl_insert(rec);
        // P6: a free block now exists.  `delete` only merged FORWARD, so an
        // adjacent free PREDECESSOR (if any) is left uncoalesced; flag it so
        // the next allocation that would otherwise grow the store runs the
        // lazy coalescing sweep first.
        self.needs_coalesce = true;
        #[cfg(debug_assertions)]
        self.fl_validate();
    }

    /// Claim the record at an **exact** position with an **exact** size (words),
    /// carving it out of whatever free block currently covers `pos`.  Unlike
    /// [`claim`](Self::claim) — best-fit, position chosen by the allocator — this
    /// reproduces a *recorded* position.  It is the keystone of the @PLN16.J store
    /// journal's position-addressed replay (`Insert`-apply and `Free`-revert): forward
    /// and reverse replay are exact because positions are forced, not re-derived.
    ///
    /// `[pos, pos + size)` must lie entirely within free space.  The covering free
    /// block `[base, bend)` (with `base <= pos`, found by walking the block chain) is
    /// split three ways — `[base, pos)` free, `[pos, pos + size)` claimed,
    /// `[pos + size, bend)` free.  A remainder below `MIN_FREE_TREE` keeps a valid free
    /// header but is left untracked (coalesced later), the same rule `claim_block`
    /// follows.
    pub(crate) fn claim_at(&mut self, pos: u32, size: u32) -> u32 {
        debug_assert!(!self.read_only, "claim_at on read-only store");
        assert!(size >= 1, "claim_at: zero-size record");
        // S28: a structural op — bump generation like claim/delete/resize.
        self.generation = self.generation.wrapping_add(1);
        // Walk the block chain to find the block physically covering `pos`.
        let mut base = PRIMARY;
        loop {
            assert!(base < self.size, "claim_at: pos {pos} past end of store");
            let bsz = (*self.addr::<i32>(base, 0)).unsigned_abs();
            assert!(bsz > 0, "claim_at: zero-size block at {base}");
            if pos < base + bsz {
                break;
            }
            base += bsz;
        }
        let header = *self.addr::<i32>(base, 0);
        assert!(
            header < 0,
            "claim_at: region at {pos} is not free (covering block {base} is claimed)"
        );
        // The free region may be *fragmented* into several adjacent free blocks:
        // `delete` coalesces only forward, so a freed predecessor stays a separate block
        // until lazy `coalesce_free` runs.  Absorb consecutive free blocks (detaching
        // each from the tree) until they span `[pos, pos + size)`.  A claimed block
        // before then is a genuine "region not free" error.
        self.fl_remove(base);
        let mut bend = base + (-header) as u32;
        while bend < pos + size {
            assert!(
                bend < self.size,
                "claim_at: record [{pos}, {}) runs past the store end",
                pos + size
            );
            let next = *self.addr::<i32>(bend, 0);
            assert!(
                next < 0,
                "claim_at: record [{pos}, {}) hits a claimed block at {bend}",
                pos + size
            );
            self.fl_remove(bend);
            bend += (-next) as u32;
        }
        // [base, pos) prefix stays free.
        if base < pos {
            *self.addr_mut::<i32>(base, 0) = -((pos - base) as i32);
            self.fl_insert(base);
        }
        // [pos, pos + size) becomes the claimed record.
        *self.addr_mut::<i32>(pos, 0) = size as i32;
        self.claims.insert(pos);
        // [pos + size, bend) suffix stays free.
        let tail = pos + size;
        if tail < bend {
            *self.addr_mut::<i32>(tail, 0) = -((bend - tail) as i32);
            self.fl_insert(tail);
        }
        #[cfg(debug_assertions)]
        self.fl_validate();
        pos
    }

    /// @PLN16.J — begin buffering structural changes for the debugger's edit journal.
    /// No-op on a locked / freed store (those are never edit targets).
    pub(crate) fn start_recording(&mut self) {
        if !self.free && !self.read_only {
            self.recording = Some(Vec::new());
        }
    }

    /// @PLN16.J — stop recording and hand back the buffered changes (`None` if this
    /// store was never recording).
    pub(crate) fn take_recording(&mut self) -> Option<Vec<StoreChange>> {
        self.recording.take()
    }

    /// Validate the store
    pub fn validate(&self, recs: u32) {
        if !cfg!(debug_assertions) {
            return;
        }
        assert!(!self.free, "Using a freed store");
        let mut pos = PRIMARY;
        let mut alloc = 0;
        while pos < self.size {
            let claim = *self.addr::<i32>(pos, 0);
            assert!(
                pos + i32::abs(claim) as u32 <= self.size,
                "Incorrect record {pos} size {}",
                i32::abs(claim)
            );
            if claim < 0 {
                // ignore the open spaces for now, later we want to check if they are part of the open tree.
                pos += (-claim) as u32;
            } else {
                // check the claimed records
                alloc += 1;
                pos += claim as u32;
            }
        }
        assert_eq!(pos, self.size, "Incorrect {pos} size {}", self.size);
        assert!(
            recs == 0 || alloc == recs as usize,
            "Inconsistent number of records: claimed {alloc} walk {recs}"
        );
    }

    /// Fail-closed, ALWAYS-ON structural validation of the block chain — the
    /// release-mode, non-panicking counterpart of [`Store::validate`], used to
    /// make adopting an UNTRUSTED buffer safe ([`Store::from_bytes`]).  Walks
    /// every record from `PRIMARY`: each `i32` size word must be **non-zero** and
    /// keep the walk **in-bounds**, and the chain must **partition the store
    /// exactly** (`pos == size` at the end — so no record claims a size past the
    /// arena, no overlap, no gap).  Returns `Err(reason)` on the first violation
    /// instead of panicking or looping.
    ///
    /// Two concrete hazards this closes for a crafted buffer:
    /// - **the 0-size-block release infinite loop** `fl_rebuild` would hit
    ///   (`pos += 0`; the @PLAN38 hazard `Store::open`'s comment describes) —
    ///   guarded by the `span == 0` reject, so it MUST run *before* `fl_rebuild`;
    /// - **the forged-header heap over-read** — a record whose size word claims
    ///   more words than remain would let later field reads run past the arena;
    ///   the in-bounds + exact-partition checks make that impossible.
    ///
    /// It validates the record/block STRUCTURE (bounds + partition).  It does NOT
    /// validate interior `DbRef` pointers (a field aimed at a wrong record) —
    /// that is `verify_graph_ok` / `store_verify`, the reachable-graph backstop
    /// recommended after an untrusted load.  @PLN97 arc G Phase 2.
    ///
    /// # Errors
    /// Returns `Err(reason)` on a too-small store, a zero-size block header, a
    /// record running past the arena, or a chain that does not end exactly at the
    /// store size.
    pub fn validate_structure(&self) -> Result<(), String> {
        if self.size < PRIMARY {
            return Err(format!("store too small: {} words", self.size));
        }
        let mut pos = PRIMARY;
        while pos < self.size {
            // In-bounds by the loop guard (`pos < size`): the `i32` size word at
            // byte `pos*8` lies within the `size*8`-byte arena.
            let claim = *self.addr::<i32>(pos, 0);
            let span = claim.unsigned_abs();
            if span == 0 {
                return Err(format!("zero-size block header at record {pos}"));
            }
            // `u64` arithmetic so an `i32::MIN` span (2^31) can't overflow the
            // bounds test.
            if u64::from(pos) + u64::from(span) > u64::from(self.size) {
                return Err(format!(
                    "record {pos} claims {span} words, past the {}-word store",
                    self.size
                ));
            }
            pos += span;
        }
        if pos != self.size {
            return Err(format!(
                "block chain ends at {pos}, not the store size {}",
                self.size
            ));
        }
        Ok(())
    }

    pub fn len(&self) -> u32 {
        self.size
    }

    /// Walk the block chain and report internal space utilisation.
    /// Same traversal as [`Self::validate`]: a negative size header
    /// (`addr::<i32>(pos, 0)`) is a free block of `-size` words, a
    /// positive header is a claimed record of `size` words.  Word 0 is
    /// the store header and is excluded.  A freed store reports only its
    /// capacity.
    #[must_use]
    pub fn usage(&self) -> StoreUsage {
        let mut u = StoreUsage {
            capacity_words: self.size,
            ..StoreUsage::default()
        };
        if self.free || self.size <= PRIMARY {
            return u;
        }
        let mut pos = PRIMARY;
        let mut prev_was_free = false;
        while pos < self.size {
            let claim = *self.addr::<i32>(pos, 0);
            if claim == 0 {
                break; // malformed / uninitialised tail — stop rather than spin
            }
            let sz = claim.unsigned_abs();
            if claim < 0 {
                u.free_words += sz;
                u.free_count += 1;
                if sz > u.largest_free_words {
                    u.largest_free_words = sz;
                }
                // Two consecutive free blocks are physical neighbours that
                // `delete` should have coalesced — flag the missed merge.
                if prev_was_free {
                    u.mergeable_free_pairs += 1;
                }
                prev_was_free = true;
            } else {
                u.claimed_words += sz;
                u.claimed_count += 1;
                prev_was_free = false;
            }
            pos += sz;
            if claim > 0 {
                u.live_end_words = pos; // a claimed block ends here
            }
        }
        // @PLN123 A0 — calibrate the mark rather than trust it.  The dangerous
        // direction is a mark that is too LOW: it reads as "nothing to
        // reclaim", which is indistinguishable from a healthy store, and arc A
        // truncates to it.
        //
        // Within a COMPLETE walk the mark cannot be too low, and that is worth
        // stating because it rules out a whole family of assertions: the loop
        // above raises the mark at every claimed block it passes, so any check
        // phrased against what this walk saw can only agree with itself.  The
        // mark's trustworthiness is not a property of the arithmetic — it is
        // exactly the question of whether the chain tiled the store, which is
        // what this bit answers and nothing else here can.
        u.walk_complete = pos == self.size;
        // Falsifiable, unlike the above: a final block whose header claims more
        // words than remain drives `pos` past the end, and if that block reads
        // as claimed the mark lands outside the arena.
        debug_assert!(
            u.live_end_words <= u.capacity_words,
            "high-water mark {} past capacity {}",
            u.live_end_words,
            u.capacity_words
        );
        u
    }

    /// Change the store size, do not mutate content
    fn resize_store(&mut self, to_size: u32) {
        if to_size <= self.size {
            return;
        }
        assert!(
            to_size <= MAX_STORE_WORDS,
            "store offset overflow: requested {} words exceeds limit of {} ({} bytes)",
            to_size,
            MAX_STORE_WORDS,
            u64::from(MAX_STORE_WORDS) * 8
        );
        // saturating_mul prevents u32 overflow when the store is very large
        let inc = self.size.saturating_mul(7) / 3;
        let size = if to_size > inc { to_size } else { inc };
        #[cfg(feature = "mmap")]
        if let Some(f) = &mut self.file {
            f.resize(size as usize * 8).expect("Resize");
            self.ptr = std::ptr::addr_of!(f.as_slice()[0]).cast_mut();
            self.size = size;
            return;
        }
        let old_bytes = self.size as usize * 8;
        let bytes = size as usize * 8;
        // Refused BEFORE the realloc, so a store that cannot grow is left exactly as
        // it was and the report describes the growth that was stopped.
        crate::store_budget::grow(self.known_type, old_bytes, bytes, self.created_at);
        let l = Layout::from_size_align(old_bytes, 8).expect("Problem");
        self.ptr = unsafe { A.realloc(self.ptr, l, bytes) };
        if bytes > old_bytes {
            unsafe { self.ptr.add(old_bytes).write_bytes(0, bytes - old_bytes) };
        }
        self.size = size;
        // @PLN154 — the shadow spans the buffer, so it grows with it.  The new bytes are
        // tagged unwritten, which is what they are: nothing has been put there yet.  The
        // tags themselves do not move, because a growth keeps every `rec * 8 + fld` where
        // it was.
        if !self.init_shadow.is_empty() {
            self.init_shadow.resize(bytes, 0);
        }
    }

    /// Give the store's tail back: lower the capacity to `words`, keeping every
    /// claimed record and every position that names one.  Returns whether the
    /// store actually shrank.  @PLN123 A1.
    ///
    /// A sibling of [`Self::resize_store`] rather than a mode of it: that one
    /// refuses to shrink and must keep refusing — it is the growth path, and
    /// every caller relies on grow-only.
    ///
    /// Safe by construction rather than by reference tracking. Everything above
    /// the high-water mark is free, and a `DbRef` is a POSITION — `(store_nr,
    /// rec, pos)`, not a pointer — so no reference can name a word above the
    /// mark. That argument holds only while the mark is read off the block chain
    /// itself ([`Self::usage`]), never a cached count, and only when the walk
    /// reached the store's end (A0: otherwise the mark is a lower bound and a
    /// live record can sit above it).
    ///
    /// Refuses, leaving the store exactly as it was, when:
    /// - the chain walk did not complete, so the mark cannot be trusted;
    /// - `words` is below the mark, or at/above the current size (never a
    ///   growth path — that is `resize_store`'s job);
    /// - the store is read-only, or borrows another store's buffer (freeing
    ///   that buffer's tail is not this store's to do);
    /// - a durable `.dmeta` sidecar is live (F4). The sidecar records the
    ///   file's byte length and CRC, so truncating behind its back turns a
    ///   healthy store into a corrupt one at the next `store_durable_check`.
    ///   Re-sealing instead is possible; refusing is the safe default.
    #[allow(dead_code)] // @PLN123 A1 lands inert: A3 is what calls it.
    pub fn shrink_to(&mut self, words: u32) -> bool {
        if self.read_only || self.borrowed || self.has_durable_sidecar() {
            return false;
        }
        let mark = {
            let u = self.usage();
            if !u.walk_complete {
                return false;
            }
            u.live_end_words
        };
        if words < mark {
            return false;
        }
        // F6 — a file-backed store that lands under the floor is lifted right
        // back up by `Store::open`, so shrinking past it buys an immediate
        // re-grow.  A heap store has no such floor: `Store::new` only insists
        // on two words.
        let floor = if self.is_file_backed() {
            MIN_BOUND_WORDS
        } else {
            PRIMARY + 1
        };
        let words = words.max(floor);
        if words >= self.size {
            return false;
        }
        let bytes = words as usize * 8;
        #[cfg(feature = "mmap")]
        if let Some(f) = &mut self.file {
            if f.resize(bytes).is_err() {
                // F3 — a failed truncation leaves the file and the mapping as
                // they were, so the store simply did not shrink.  Re-derive the
                // pointer regardless: if the REMAP is what failed there is no
                // mapping at all, and this is where that has to surface, rather
                // than in a caller reading through a dangling pointer.
                self.ptr = std::ptr::addr_of!(f.as_slice()[0]).cast_mut();
                return false;
            }
            self.ptr = std::ptr::addr_of!(f.as_slice()[0]).cast_mut();
            self.size = words;
            self.retile_tail(mark);
            return true;
        }
        let l = Layout::from_size_align(self.size as usize * 8, 8).expect("Problem");
        self.ptr = unsafe { A.realloc(self.ptr, l, bytes) };
        crate::store_budget::shrink(
            self.known_type,
            self.size as usize * 8,
            bytes,
            self.created_at,
        );
        self.size = words;
        self.retile_tail(mark);
        true
    }

    /// Re-tile the arena after a truncation: whatever free space is left above
    /// `mark` becomes ONE free block ending exactly at the new size, so the
    /// chain still partitions the store, and the free tree is rebuilt from that
    /// chain — F2, because a stale `free_root` still indexes the words that were
    /// just cut and would hand one of them out.
    fn retile_tail(&mut self, mark: u32) {
        // @PLN154 — the shadow spans the buffer, so a truncation truncates it.  Called
        // from both shrink paths, which is why it sits here rather than beside each.
        if !self.init_shadow.is_empty() {
            self.init_shadow.truncate(self.size as usize * 8);
        }
        let tail = mark.max(PRIMARY);
        if tail < self.size {
            *self.addr_mut(tail, 0) = -((self.size - tail) as i32);
        }
        self.fl_rebuild();
        // The same reason `resize` bumps it: a suspended coroutine holding a
        // DbRef has to be able to notice that the arena moved.
        self.generation = self.generation.wrapping_add(1);
    }

    /// Lock this store against writes. Any subsequent call to `addr_mut` panics.
    /// Sets `lock_origin` to a generic identifier — callers wanting richer
    /// context should use `lock_with_origin`.
    pub fn lock(&mut self) {
        self.lock_with_origin("Store::lock");
    }

    /// Lock this store + record an identifier of the lock-origin call site.
    /// The origin string surfaces in panic messages from `addr_mut` / `claim`
    /// / `delete` so a "Write to read-only store" failure points directly at the
    /// locker rather than requiring `LOFT_LOG=locks` to re-trace.
    pub fn lock_with_origin(&mut self, origin: impl Into<String>) {
        let origin = origin.into();
        // Plan-22 02d-vii follow-up — `LOFT_LOG=locks` trace.
        // Caught here (the lowest-level lock site) so direct
        // callers bypassing `Stores::lock_store(&r)` (e.g.
        // `compile.rs` const-store init, `native.rs` worker
        // store init) are visible too.
        if !self.read_only && crate::log_config::lock_trace_enabled() {
            crate::loft_eprintln!("[locks] LOCK   origin={origin:?}");
        }
        self.read_only = true;
        self.lock_origin = origin;
    }

    /// Unlock this store (only callable from Rust; loft code cannot unlock via d#lock = false
    /// on a const variable).
    pub fn unlock(&mut self) {
        if self.read_only && crate::log_config::lock_trace_enabled() {
            crate::loft_eprintln!("[locks] UNLOCK origin-was={:?}", self.lock_origin);
        }
        self.read_only = false;
        self.lock_origin.clear();
    }

    /// @P290 — mark the store as PROTECTED-FROM-FREE for the duration
    /// of a fn-call deep-copy bracket.  Writes, claims AND deletes stay
    /// legal; the one thing blocked is the `0x8000` source-free, refused
    /// by `do_copy_record` and `replace_keyed` themselves.  Deleting was
    /// blocked here too until loft#760 — wider than the marker's job, and
    /// it aborted on a container releasing its own block as it regrew.
    /// Cleared by `clear_free_protected()`.
    pub fn set_free_protected(&mut self, origin: impl Into<String>) {
        let origin = origin.into();
        if self.free_protect_depth == 0 && crate::log_config::lock_trace_enabled() {
            crate::loft_eprintln!("[locks] FREE_PROTECT origin={origin:?}");
        }
        self.free_protect_depth = self.free_protect_depth.saturating_add(1);
        self.lock_origin = origin;
    }

    /// @P290 — clear the call-bracket free-protection.
    pub fn clear_free_protected(&mut self) {
        self.free_protect_depth = self.free_protect_depth.saturating_sub(1);
        if self.free_protect_depth > 0 {
            // An enclosing bracket still holds this store; keep its origin.
            return;
        }
        if crate::log_config::lock_trace_enabled() {
            crate::loft_eprintln!("[locks] FREE_UNPROTECT origin-was={:?}", self.lock_origin);
        }
        // Clear lock_origin only if the hard read_only lock isn't also
        // holding it (it shouldn't be — but be defensive).
        if !self.read_only {
            self.lock_origin.clear();
        }
    }

    /// Return whether this store is HARD-locked (read-only).
    /// CONST_STORE / worker borrow / JSON null sentinel.  Does NOT
    /// include the call-bracket free-protection.
    #[must_use]
    pub fn is_locked(&self) -> bool {
        self.read_only
    }

    /// Return whether this store is currently protected from frees
    /// by a fn-call deep-copy bracket.  @P290.
    #[must_use]
    pub fn is_free_protected(&self) -> bool {
        self.free_protect_depth > 0
    }

    /// Has this store been freed?
    ///
    /// A freed store keeps its buffer until its slot is reused, so reading a record
    /// out of one still answers the old bytes — which is why "the data is still
    /// there" is not evidence that it is alive, and why a test about store lifetime
    /// has to ask this rather than read a value back.
    #[must_use]
    pub fn is_free(&self) -> bool {
        self.free
    }

    /// Return whether this store is a borrowed view of another store's buffer.
    #[must_use]
    pub fn is_borrowed(&self) -> bool {
        self.borrowed
    }

    /// Does a durable `.dmeta` sidecar record this store's file?
    ///
    /// The sidecar carries the file's byte length and a payload CRC, so any
    /// operation that rewrites or shortens the file behind its back turns a
    /// healthy store into a corrupt one at the next `store_durable_check`.
    /// Both [`Self::shrink_to`] and @PLN123's compaction refuse on it.
    ///
    /// It asks about the FILE, not about how the store was opened, and the
    /// difference is the whole point.  The first version tested
    /// `durable_meta_path.is_some()` — set only by `Store::open_durable`, a Rust
    /// entry point **no loft program can reach**.  Meanwhile the loft-level
    /// surface is path-based (`store_durable_seal(path)`), so the reachable
    /// hazard was entirely unguarded: seal, then `store_reclaim`, and
    /// `store_durable_check` reported a perfectly healthy store as CORRUPT
    /// (measured: 156,344 -> 138,976 bytes, check true -> false).  A guard on
    /// how you got here cannot see a fact about what is on disk.
    #[must_use]
    pub fn has_durable_sidecar(&self) -> bool {
        #[cfg(feature = "mmap")]
        {
            if self.durable_meta_path.is_some() {
                return true;
            }
            self.file_path
                .as_deref()
                .is_some_and(|p| dmeta_path(p).exists())
        }
        #[cfg(not(feature = "mmap"))]
        {
            false
        }
    }

    /// Return whether this store has an empty claims set (worker clones).
    #[must_use]
    pub fn claims_empty(&self) -> bool {
        self.claims.is_empty()
    }

    /// Relabel this store's type, moving its bytes with it in the memory-ceiling
    /// accounting.
    ///
    /// A store is often created before its type is known — `database_named` claims
    /// the store, then names it — so the bytes are first filed under whatever type
    /// it started with.  Left there, the breakdown a refused growth prints would
    /// blame the wrong type, which is exactly the fact that report exists to get
    /// right.
    pub fn set_known_type(&mut self, kt: u16) {
        if self.known_type == kt {
            return;
        }
        if !self.borrowed && !self.is_file_backed() {
            let bytes = self.size as usize * 8;
            crate::store_budget::release(self.known_type, bytes, self.created_at);
            crate::store_budget::add(kt, bytes, self.created_at);
        }
        self.known_type = kt;
    }

    /// Stamp the bytecode position that allocated this store, moving its bytes with
    /// it in the allocation-site ledger (@PLN140 arc A).
    ///
    /// The site is stamped *after* the buffer exists — `Store::new` allocates, then
    /// `database_named` says where from — and store slots are pooled, so a reused
    /// slot keeps its buffer and gets a new site. Writing `created_at` directly still
    /// compiles and is sound (the ceiling counts bytes, not sites), but it leaves
    /// those bytes filed under the previous site, which is the one thing the hot-spot
    /// report has to get right.
    pub fn set_created_at(&mut self, pc: u32) {
        if self.created_at == pc {
            return;
        }
        if !self.borrowed && !self.is_file_backed() {
            crate::store_budget::relabel(
                (self.created_at, self.known_type),
                (pc, self.known_type),
                self.size as usize * 8,
            );
        }
        self.created_at = pc;
    }

    /// Create a locked deep-copy of this store for use in a worker thread.
    /// The clone always has `locked = true`; the mmap file is not shared (data is copied).
    pub fn clone_locked(&self) -> Store {
        let l = Layout::from_size_align(self.size as usize * 8, 8).expect("Problem");
        let ptr = unsafe { A.alloc(l) };
        crate::store_budget::add(self.known_type, self.size as usize * 8, 0);
        unsafe { std::ptr::copy_nonoverlapping(self.ptr, ptr, self.size as usize * 8) };
        Store {
            ptr,
            size: self.size,
            claims: self.claims.clone(),
            #[cfg(feature = "mmap")]
            file: None,
            free: self.free,
            read_only: true,
            free_protect_depth: 0,
            free_root: 0, // workers never claim/delete; no free tree needed
            needs_coalesce: false,
            released_bytes: 0,
            claimed_end: 0,
            generation: self.generation,
            recording: None,
            tag: self.tag,
            borrowed: false,
            store_nr: u16::MAX,
            alloc_serial: 0,
            created_at: 0,
            last_op_at: 0,
            pinned: self.pinned,
            lock_origin: "clone_locked".to_string(),
            known_type: self.known_type,
            durable_meta_path: None,
            file_path: None,
            durable_tier: 0,
            init_shadow: Vec::new(),
        }
    }

    /// @PLN63 RX1 — a **writable** deep byte-copy of this in-memory store, for the reverse-step
    /// checkpoint ring.  Like [`clone_locked`](Self::clone_locked) it allocates a fresh buffer
    /// and copies the bytes, but the copy is writable (`read_only = false`) and keeps the
    /// free-space tree + claims (`free_root`, `needs_coalesce`, `claims`) — because a restored
    /// store resumes *execution* and will `claim`/`delete`, unlike a worker borrow which never
    /// allocates.  The caller guarantees `!self.is_file_backed()`; the copy is heap-owned
    /// (`file = None`, `borrowed = false`) so `Drop` deallocates it correctly.
    #[must_use]
    pub(crate) fn snapshot_copy(&self) -> Store {
        let l = Layout::from_size_align(self.size as usize * 8, 8).expect("snapshot layout");
        let ptr = unsafe { A.alloc(l) };
        crate::store_budget::add(self.known_type, self.size as usize * 8, self.created_at);
        unsafe { std::ptr::copy_nonoverlapping(self.ptr, ptr, self.size as usize * 8) };
        Store {
            ptr,
            size: self.size,
            claims: self.claims.clone(),
            #[cfg(feature = "mmap")]
            file: None,
            free: self.free,
            read_only: false,
            free_protect_depth: self.free_protect_depth,
            borrowed: false,
            store_nr: self.store_nr,
            alloc_serial: self.alloc_serial,
            created_at: self.created_at,
            last_op_at: self.last_op_at,
            free_root: self.free_root,
            needs_coalesce: self.needs_coalesce,
            released_bytes: 0,
            claimed_end: 0,
            generation: self.generation,
            recording: None,
            tag: self.tag,
            pinned: self.pinned,
            lock_origin: String::new(),
            known_type: self.known_type,
            durable_meta_path: None,
            file_path: None,
            durable_tier: 0,
            init_shadow: Vec::new(),
        }
    }

    /// Create a read-only view that shares the original's buffer pointer.
    /// The borrow is `locked = true` (writes panic/discard) and `borrowed = true`
    /// (Drop does NOT free the buffer — the main thread owns it).
    ///
    /// # Safety
    /// The original `Store` must outlive all threads that hold the borrow.
    /// Guaranteed by `thread::scope` in `run_parallel_light`.
    pub unsafe fn borrow_locked_for_light_worker(&self) -> Store {
        Store {
            ptr: self.ptr,
            claims: HashSet::new(),
            size: self.size,
            #[cfg(feature = "mmap")]
            file: None,
            free: false,
            read_only: true,
            free_protect_depth: 0,
            free_root: self.free_root,
            needs_coalesce: false,
            released_bytes: 0,
            claimed_end: 0,
            generation: self.generation,
            recording: None,
            tag: self.tag,
            borrowed: true,
            store_nr: u16::MAX,
            alloc_serial: 0,
            created_at: 0,
            last_op_at: 0,
            pinned: self.pinned,
            lock_origin: "borrow_locked_for_light_worker".to_string(),
            known_type: self.known_type,
            durable_meta_path: None,
            file_path: None,
            durable_tier: 0,
            init_shadow: Vec::new(),
        }
    }

    /// Create a sentinel for a freed main-thread slot.
    /// Tiny allocation, no data — just a placeholder so the worker's slot indices align.
    pub fn new_freed_sentinel() -> Store {
        let mut s = Store::new(4);
        s.free = true;
        s
    }

    // ---- LLRB free-space tree ------------------------------------------------
    // Nodes are stored inside free blocks using fields at FL_LEFT / FL_RIGHT /
    // FL_COLOR.  Key = (positive_block_size, block_position); ties break on pos.
    // Only blocks with size >= MIN_FREE_TREE are tracked.

    fn fl_size(&self, p: u32) -> i32 {
        -*self.addr::<i32>(p, 0)
    }

    fn fl_left(&self, p: u32) -> u32 {
        *self.addr::<u32>(p, FL_LEFT)
    }

    fn fl_right(&self, p: u32) -> u32 {
        *self.addr::<u32>(p, FL_RIGHT)
    }

    fn fl_red(&self, p: u32) -> bool {
        *self.addr::<u8>(p, FL_COLOR) != 0
    }

    fn fl_set_left(&mut self, p: u32, v: u32) {
        *self.addr_mut::<u32>(p, FL_LEFT) = v;
    }

    fn fl_set_right(&mut self, p: u32, v: u32) {
        *self.addr_mut::<u32>(p, FL_RIGHT) = v;
    }

    fn fl_set_red(&mut self, p: u32, v: bool) {
        *self.addr_mut::<u8>(p, FL_COLOR) = u8::from(v);
    }

    fn fl_cmp(&self, a: u32, b: u32) -> Ordering {
        match self.fl_size(a).cmp(&self.fl_size(b)) {
            Ordering::Equal => a.cmp(&b),
            other => other,
        }
    }

    fn fl_rotate_left(&mut self, h: u32) -> u32 {
        let x = self.fl_right(h);
        let x_left = self.fl_left(x);
        self.fl_set_right(h, x_left);
        self.fl_set_left(x, h);
        let h_red = self.fl_red(h);
        self.fl_set_red(x, h_red);
        self.fl_set_red(h, true);
        x
    }

    fn fl_rotate_right(&mut self, h: u32) -> u32 {
        let x = self.fl_left(h);
        let x_right = self.fl_right(x);
        self.fl_set_left(h, x_right);
        self.fl_set_right(x, h);
        let h_red = self.fl_red(h);
        self.fl_set_red(x, h_red);
        self.fl_set_red(h, true);
        x
    }

    fn fl_flip_colors(&mut self, h: u32) {
        let h_red = self.fl_red(h);
        self.fl_set_red(h, !h_red);
        let l = self.fl_left(h);
        if l != 0 {
            self.fl_set_red(l, !self.fl_red(l));
        }
        let r = self.fl_right(h);
        if r != 0 {
            self.fl_set_red(r, !self.fl_red(r));
        }
    }

    fn fl_balance(&mut self, mut h: u32) -> u32 {
        let r = self.fl_right(h);
        if r != 0 && self.fl_red(r) {
            h = self.fl_rotate_left(h);
        }
        let l = self.fl_left(h);
        let ll = if l != 0 { self.fl_left(l) } else { 0 };
        if l != 0 && self.fl_red(l) && ll != 0 && self.fl_red(ll) {
            h = self.fl_rotate_right(h);
        }
        let l2 = self.fl_left(h);
        let r2 = self.fl_right(h);
        if l2 != 0 && self.fl_red(l2) && r2 != 0 && self.fl_red(r2) {
            self.fl_flip_colors(h);
        }
        h
    }

    fn fl_insert_node(&mut self, h: u32, rec: u32) -> u32 {
        if h == 0 {
            self.fl_set_left(rec, 0);
            self.fl_set_right(rec, 0);
            self.fl_set_red(rec, true);
            return rec;
        }
        match self.fl_cmp(rec, h) {
            Ordering::Less => {
                let l = self.fl_left(h);
                let new_l = self.fl_insert_node(l, rec);
                self.fl_set_left(h, new_l);
            }
            Ordering::Greater | Ordering::Equal => {
                let r = self.fl_right(h);
                let new_r = self.fl_insert_node(r, rec);
                self.fl_set_right(h, new_r);
            }
        }
        self.fl_balance(h)
    }

    /// Register a free block in the LLRB free-space tree.
    /// Blocks with fewer than `MIN_FREE_TREE` words are silently ignored.
    fn fl_insert(&mut self, rec: u32) {
        if self.fl_size(rec) < MIN_FREE_TREE {
            return;
        }
        let root = self.free_root;
        self.free_root = self.fl_insert_node(root, rec);
        self.fl_set_red(self.free_root, false);
    }

    fn fl_min_node(&self, h: u32) -> u32 {
        if h == 0 {
            return 0;
        }
        let l = self.fl_left(h);
        if l == 0 { h } else { self.fl_min_node(l) }
    }

    fn fl_move_red_left(&mut self, mut h: u32) -> u32 {
        self.fl_flip_colors(h);
        let r = self.fl_right(h);
        let rl = if r != 0 { self.fl_left(r) } else { 0 };
        if rl != 0 && self.fl_red(rl) {
            let new_r = self.fl_rotate_right(r);
            self.fl_set_right(h, new_r);
            h = self.fl_rotate_left(h);
            self.fl_flip_colors(h);
        }
        h
    }

    fn fl_move_red_right(&mut self, mut h: u32) -> u32 {
        self.fl_flip_colors(h);
        let l = self.fl_left(h);
        let ll = if l != 0 { self.fl_left(l) } else { 0 };
        if ll != 0 && self.fl_red(ll) {
            h = self.fl_rotate_right(h);
            self.fl_flip_colors(h);
        }
        h
    }

    /// Remove the leftmost (minimum) node from the subtree rooted at `h`.
    fn fl_delete_min_node(&mut self, h: u32) -> u32 {
        if self.fl_left(h) == 0 {
            return 0;
        }
        let l = self.fl_left(h);
        let ll = self.fl_left(l);
        let mut cur = h;
        if !self.fl_red(l) && (ll == 0 || !self.fl_red(ll)) {
            cur = self.fl_move_red_left(cur);
        }
        let left = self.fl_left(cur);
        let new_left = self.fl_delete_min_node(left);
        self.fl_set_left(cur, new_left);
        self.fl_balance(cur)
    }

    /// Remove the block at `target_pos` from the subtree rooted at `h`.
    fn fl_delete_node(&mut self, mut h: u32, target_pos: u32) -> u32 {
        if h == 0 {
            return 0; // target not found in this subtree (shouldn't happen normally)
        }
        let target_sz = self.fl_size(target_pos);
        let h_sz = self.fl_size(h);
        if (target_sz, target_pos) < (h_sz, h) {
            let l = self.fl_left(h);
            let ll = if l != 0 { self.fl_left(l) } else { 0 };
            if l != 0 && !self.fl_red(l) && (ll == 0 || !self.fl_red(ll)) {
                h = self.fl_move_red_left(h);
            }
            let left = self.fl_left(h);
            let new_left = self.fl_delete_node(left, target_pos);
            self.fl_set_left(h, new_left);
        } else {
            if self.fl_left(h) != 0 && self.fl_red(self.fl_left(h)) {
                h = self.fl_rotate_right(h);
            }
            if h == target_pos && self.fl_right(h) == 0 {
                return 0;
            }
            let r = self.fl_right(h);
            let rl = if r != 0 { self.fl_left(r) } else { 0 };
            if r != 0 && !self.fl_red(r) && (rl == 0 || !self.fl_red(rl)) {
                h = self.fl_move_red_right(h);
            }
            if h == target_pos {
                let right = self.fl_right(h);
                if right == 0 {
                    // No right subtree after rotations; just return left.
                    return self.fl_left(h);
                }
                let succ = self.fl_min_node(right);
                let h_left = self.fl_left(h);
                let h_red = self.fl_red(h);
                let new_right = self.fl_delete_min_node(right);
                self.fl_set_left(succ, h_left);
                self.fl_set_right(succ, new_right);
                self.fl_set_red(succ, h_red);
                h = succ;
            } else {
                let right = self.fl_right(h);
                let new_right = self.fl_delete_node(right, target_pos);
                self.fl_set_right(h, new_right);
            }
        }
        self.fl_balance(h)
    }

    /// Find the position of the smallest free block with size >= `min_size`.
    /// Returns 0 when no suitable block exists.
    fn fl_find_ge(&self, h: u32, min_size: i32) -> u32 {
        if h == 0 {
            return 0;
        }
        if self.fl_size(h) < min_size {
            return self.fl_find_ge(self.fl_right(h), min_size);
        }
        let left_result = self.fl_find_ge(self.fl_left(h), min_size);
        if left_result != 0 { left_result } else { h }
    }

    /// Remove and return the smallest free block with size >= `min_size`.
    fn fl_take_ge(&mut self, min_size: i32) -> Option<u32> {
        if self.free_root == 0 {
            return None;
        }
        let found = self.fl_find_ge(self.free_root, min_size);
        if found == 0 {
            return None;
        }
        let root = self.free_root;
        self.free_root = self.fl_delete_node(root, found);
        if self.free_root != 0 {
            self.fl_set_red(self.free_root, false);
        }
        Some(found)
    }

    /// Remove `rec` from the free tree if it is currently tracked.
    fn fl_remove(&mut self, rec: u32) {
        if self.free_root == 0 || self.fl_size(rec) < MIN_FREE_TREE {
            return;
        }
        #[cfg(debug_assertions)]
        debug_assert!(
            self.fl_contains(rec),
            "fl_remove: block at {rec} (size={}) not in free tree",
            self.fl_size(rec)
        );
        let root = self.free_root;
        self.free_root = self.fl_delete_node(root, rec);
        if self.free_root != 0 {
            self.fl_set_red(self.free_root, false);
        }
    }

    /// Return `true` if `target` is reachable from the free-tree root.
    #[cfg(debug_assertions)]
    fn fl_contains(&self, target: u32) -> bool {
        self.fl_contains_node(self.free_root, target)
    }

    #[cfg(debug_assertions)]
    fn fl_contains_node(&self, h: u32, target: u32) -> bool {
        if h == 0 {
            return false;
        }
        if h == target {
            return true;
        }
        self.fl_contains_node(self.fl_left(h), target)
            || self.fl_contains_node(self.fl_right(h), target)
    }

    /// Scan the whole store and (re)build the free-space tree from scratch.
    /// Called once after `open()` to populate the tree from persisted data.
    pub fn fl_rebuild(&mut self) {
        self.free_root = 0;
        let mut pos = PRIMARY;
        while pos < self.size {
            let header = *self.addr::<i32>(pos, 0);
            let block_size = i32::abs(header);
            debug_assert!(block_size > 0, "zero-size block at {pos}");
            // A zero-size block ENDS the walk rather than repeating it. The
            // chain is malformed at that point — `validate_structure` is what
            // refuses it — and stepping by zero is an infinite loop, which in a
            // release build is a hang with no output rather than a diagnosis.
            // `claims_rebuild` walks the same chain and has always stopped here;
            // this half had only the debug assertion, so an image with a short
            // tail hung @PLN119's remote transport (arc E) instead of failing.
            if block_size == 0 {
                break;
            }
            if header < 0 && -header >= MIN_FREE_TREE {
                self.fl_insert(pos);
            }
            pos += block_size as u32;
        }
    }

    /// @PLN119 arc E — take on a store image another machine sent.
    ///
    /// An image is the LIVE PREFIX of a store, not a whole one: sending a
    /// store's unused capacity would make every small call pay for the largest
    /// one before it. So the prefix has to be made into a well-formed store
    /// again on arrival, and that is one step — the space past the image becomes
    /// a single free block, exactly as `init` makes the whole store one.
    ///
    /// Without it the tail is whatever this buffer held, and a zero word there
    /// reads as a zero-size block: the chain walk then never terminates.
    pub fn adopt_image(&mut self, bytes: &[u8]) {
        let words = u32::try_from(bytes.len() / 8)
            .unwrap_or(u32::MAX)
            .max(PRIMARY + 1);
        if words > self.size {
            self.resize_store(words);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.ptr,
                bytes.len().min(self.size as usize * 8),
            );
        }
        // Everything past the image is free space, in one block. `init` writes
        // the same negative-header form for a whole fresh store.
        if words < self.size {
            *self.addr_mut(words, 0) = -((self.size - words) as i32);
        }
        self.free_root = 0;
        self.fl_rebuild();
        self.claims_rebuild();
    }

    /// Rebuild `claims` from the arena, for a store whose records arrived as an IMAGE
    /// rather than through `claim`.
    ///
    /// `claims` is in-memory bookkeeping and is not persisted, so a store built by
    /// `from_bytes` came back with LIVE records and an EMPTY claims set — and every
    /// check phrased against claims then read those records as unknown. `store_load`
    /// into a heap-backed hash hit it on the first iteration: `Unknown record 1` out of
    /// `hash::records`, under debug assertions only, so ordinary runs and the release
    /// suite never saw it.
    ///
    /// A reopened mmap store has the same empty set and is exempted at the check
    /// instead. This rebuilds rather than exempts because the information is right
    /// there in the block chain — the same walk `fl_rebuild` does — so claims can
    /// describe reality instead of being a set the reader has to know to distrust.
    fn claims_rebuild(&mut self) {
        self.claims.clear();
        let mut pos = PRIMARY;
        while pos < self.size {
            let header = *self.addr::<i32>(pos, 0);
            let block_size = i32::abs(header);
            if block_size <= 0 {
                break; // a malformed image; `validate_structure` is what refuses it
            }
            if header > 0 {
                self.claims.insert(pos);
            }
            pos += block_size as u32;
        }
    }

    /// @PLN119 arc B — re-derive the allocator's cached state from the store's
    /// own bytes.
    ///
    /// The free tree and the claim set live in the block chain; the `Store`
    /// struct only caches them, and a cache is only ever right for the process
    /// that made the claims. A store shared through a mapping therefore has a
    /// second reader whose cache says nothing happened — and allocating against
    /// that stale cache hands out a block that is already in use.
    ///
    /// This is the same walk `open` does, made callable at the moment the other
    /// side hands the store over. It is not a repair: nothing about the bytes
    /// changes, only this side's opinion of them.
    pub fn resync_allocator(&mut self) {
        self.fl_rebuild();
        self.claims_rebuild();
    }

    /// P6: merge every run of adjacent free blocks in place, then rebuild
    /// the free tree from the coalesced chain.  `delete` only coalesces
    /// FORWARD (it cannot find a freed block's predecessor in the
    /// header-only layout), so adjacent frees accumulate; this single
    /// O(n) pass over the contiguous block chain (the same walk
    /// `claim_scan` / `usage` / `fl_rebuild` already do) catches them all,
    /// reusing the one existing free tree — no extra index, no footer.
    /// Called lazily by `claim` only when an allocation would otherwise
    /// grow the store, so freed space is reused instead.
    fn coalesce_free(&mut self) {
        let mut pos = PRIMARY;
        while pos < self.size {
            let header = *self.addr::<i32>(pos, 0);
            let mut block_size = i32::abs(header);
            debug_assert!(block_size > 0, "zero-size block at {pos}");
            if header < 0 {
                // Absorb following adjacent free blocks into this one.
                let mut next = pos + block_size as u32;
                while next < self.size {
                    let nh = *self.addr::<i32>(next, 0);
                    if nh >= 0 {
                        break;
                    }
                    block_size += i32::abs(nh);
                    next = pos + block_size as u32;
                }
                *self.addr_mut(pos, 0) = -block_size;
            }
            pos += block_size as u32;
        }
        self.fl_rebuild();
        self.needs_coalesce = false;
    }

    /// Sweep, then give the tail back: returns the number of WORDS the store
    /// shrank by, 0 when there was nothing to give.  The whole of arc A behind
    /// one call.  @PLN123 A2.
    ///
    /// **The sweep does not move the mark**, and the plan's own step said
    /// otherwise, so it is worth being exact.  `live_end_words` is the end of
    /// the last CLAIMED block; merging free blocks never moves a claimed one,
    /// so the tail that comes back is the same with or without the sweep.  (The
    /// claim it inherited — "a naive check reclaims almost nothing without
    /// coalescing" — is true of a *is the top block free* test, which is not how
    /// the mark is computed.)
    ///
    /// It is here anyway, and pays for a different thing: a caller reaching for
    /// this has just dropped a lot, `coalesce_free` is lazy (only `claim` runs
    /// it, and only when it would otherwise grow the store), so the INTERIOR is
    /// where those thousands of unmerged free blocks sit — 2,696 mergeable pairs
    /// in this plan's measurement.  Merging them is what decides whether the
    /// next large claim reuses space or grows the store, and this explicit, rare
    /// call is the one moment an O(blocks) sweep is welcome.  Nothing on the
    /// free path ever walks the chain.
    #[allow(dead_code)] // @PLN123 A2 lands inert: A3 is what calls it.
    pub fn reclaim_tail(&mut self) -> u32 {
        if self.needs_coalesce {
            self.coalesce_free();
        }
        let before = self.size;
        let mark = self.usage().live_end_words;
        // Not to the bare mark: the store stays LIVE, so it keeps allocating,
        // and a store with no slack pays 7/3 on its next claim — which for a
        // bound store means the file comes back BIGGER than before the call
        // (loft#727).  `slack_target` is the same eighth the image format
        // already gives a freshly-bound store; both routes to a right-sized
        // store now leave it in the same shape.
        let target = slack_target(mark);
        // `shrink_to` clamps to its floor, so the size it settles on is the
        // only honest source for what came back — never `before - mark`.
        if self.shrink_to(target) {
            before - self.size
        } else {
            0
        }
    }

    /// The word past everything this arena has written — [`Store::claimed_end`],
    /// seeded from the block chain the first time it is asked of a store whose claims
    /// this process did not make.
    ///
    /// Clamped to the capacity, because the seed is the only place a stale value could
    /// come from and its one consumer hands the result to `madvise`.
    #[cfg(all(feature = "mmap", unix))]
    fn write_frontier(&mut self) -> u32 {
        // Once per store, on the FIRST release only: a store bound to an existing
        // image holds claims this process never made, so `claimed_end` has seen none
        // of them and would name a mark far below the content. The walk that reads
        // them costs a touch of every page — which is the whole thing this call
        // avoids — so it happens once, at a moment when a bind-first generator's
        // arena is still nearly empty, and never again.
        //
        // An incomplete walk makes the mark a LOWER bound, which is the safe
        // direction here (fewer pages flushed) and is used rather than refused.
        // `shrink_to` refuses on the same fact, because truncating to a lower bound
        // cuts live data where dropping fewer pages only buys less.
        if self.released_bytes == 0 {
            self.claimed_end = self.claimed_end.max(self.usage().live_end_words);
        }
        self.claimed_end.min(self.size)
    }

    /// @PLN126 — write everything below the arena's high-water mark out to the file
    /// and drop it from this process's resident set.  Answers the BYTES dropped; 0
    /// when there was nothing to drop or this store is not file-backed.
    ///
    /// The whole of the plan behind one call, and the reason it can be one call is a
    /// measurement.  A *per-record* release is impossible — `MADV_DONTNEED` works on
    /// pages, and `database::spans` measured that 0.0% of the pages one record of a
    /// real generator's shape touches hold only that record.  A *frontier* release
    /// needs no per-record contiguity at all: it needs the region below the mark not
    /// to be written again, and 87–99% of its pages are not.
    ///
    /// **Content is unaffected, and so is every reference into it.**  The mapping is
    /// `MAP_SHARED`, so the pages stay in the page cache and an access after this
    /// re-faults them from the file — the same bytes, one page fault later.  Nothing
    /// moves, nothing is freed, and the arena's own bookkeeping is untouched: this
    /// changes where the bytes are RESIDENT, not what they are.
    ///
    /// Two calls in one because either alone is a trap.  `msync` without the drop
    /// leaves the pages dirty-clean in RSS and gives back nothing; the drop without
    /// `msync` hands the kernel a writeback it must do under pressure, at the moment
    /// it is least able to.  Flushing first is what turns unreclaimable dirty page
    /// cache into reclaimable clean page cache, which is the whole mechanism.
    ///
    /// Only WHOLE pages strictly below the mark are dropped, so the page the next
    /// claim writes into is never among them.
    #[cfg(all(feature = "mmap", unix))]
    pub fn release_resident(&mut self) -> u64 {
        if self.file.is_none() || self.read_only {
            return 0;
        }
        let mark = self.write_frontier();
        let page = page_bytes();
        let till = (u64::from(mark) * 8) / page * page;
        // Only the region since the LAST release, and that is not an optimisation —
        // it is what makes the call affordable at all.  Flushing "everything below the
        // frontier" each time re-syncs a region that grows with the run, so a
        // generator that calls this per record pays O(n²) in writeback: measured at
        // 217x and 359x the wall clock of the same build without it, on a call that
        // exists to make a build FASTER under memory pressure.  Bounded to the new
        // bytes it is O(file), which is the writeback the run owed anyway.
        //
        // A page below the mark that is written again afterwards is therefore not
        // re-flushed here.  That is the 1–13% `database::spans` measured, and the
        // kernel's ordinary writeback is exactly the right owner for it: the whole
        // claim of this call is that it knows about the OTHER 87–99%.
        let from = self.released_bytes.min(till);
        let len = till - from;
        if len == 0 {
            return 0;
        }
        // SAFETY: `ptr` is the mmap base (page-aligned by construction), `from` is a
        // whole number of pages, and `from + len` is `till`, which is inside the
        // mapping — `live_end_words` never exceeds the store's capacity, which
        // `usage` itself asserts.
        let at = unsafe { self.ptr.add(from as usize) };
        // `MS_ASYNC`, not `MS_SYNC`, and the gap is the difference between a call a
        // generator can make per record and one it cannot make at all. Both reach the
        // same resident set — 44.3 MB down to 2.2 MB on an 89 MB build — but waiting
        // for the writeback costs ~1.5 ms per call, which is 208x the wall clock of
        // the same build at one call per record. Asynchronous, the identical drop is
        // FREE: 0.8x, slightly faster than not calling it, because the writeback
        // starts early instead of arriving all at once at the end.
        //
        // Nothing is at risk in the difference. The mapping is `MAP_SHARED`, so
        // `MADV_DONTNEED` unmaps the pages and leaves them in the page cache for the
        // kernel to write back; the `msync` only asks for that writeback to START, so
        // the region becomes reclaimable sooner. This call is a residency hint and
        // makes no durability promise — `store_durable_seal` is what does.
        let ok = unsafe {
            libc::msync(at.cast(), len as usize, libc::MS_ASYNC) == 0
                && libc::madvise(at.cast(), len as usize, libc::MADV_DONTNEED) == 0
        };
        if ok {
            self.released_bytes = till;
            len
        } else {
            0
        }
    }

    /// Not compiled without `mmap` (no file to flush to) or off unix (no `madvise`).
    /// A no-op rather than an error: the call is a HINT about residency, and a program
    /// that runs on a target which cannot honour it is not a program that is wrong.
    #[cfg(not(all(feature = "mmap", unix)))]
    pub fn release_resident(&mut self) -> u64 {
        0
    }

    /// Debug-only: walk the LLRB tree and verify its invariants.
    ///
    /// Asserts that:
    /// - Every tree node has a negative fld-0 header (it is truly free).
    /// - No tree node is present in `claims` (freed ≠ claimed).
    #[cfg(debug_assertions)]
    pub fn fl_validate(&self) {
        self.fl_validate_node(self.free_root);
    }

    #[cfg(debug_assertions)]
    fn fl_validate_node(&self, h: u32) {
        if h == 0 {
            return;
        }
        let header: i32 = *self.addr(h, 0);
        debug_assert!(
            header < 0,
            "fl_validate: node at {h} has positive header {header} (should be free)"
        );
        debug_assert!(
            !self.claims.contains(&h),
            "fl_validate: node at {h} is both in the free tree and in claims"
        );
        self.fl_validate_node(self.fl_left(h));
        self.fl_validate_node(self.fl_right(h));
    }

    // ---- End of LLRB free-space tree -----------------------------------------

    /// Byte offset of field `fld` in record `rec`, computed in u64 so the multiply
    /// cannot wrap.
    ///
    /// Only reports the offset.  Whether the offset is *inside this store* is a
    /// question about `self`, which this associated function cannot see — ask
    /// [`Self::offset_in_bounds`], and prefer it wherever a `&self` is at hand.
    ///
    /// **The panic here is reachable only where `isize` is 32 bits** — the wasm
    /// targets.  Both inputs are `u32`, so the largest offset this can produce is
    /// about 3.7e10, which fits an i64 with room to spare: on a 64-bit host
    /// `try_from` never fails and this raise is dead code.  On wasm32 it fires once
    /// `rec` passes ~268 million.
    #[inline]
    fn checked_offset(rec: u32, fld: u32) -> isize {
        let off = u64::from(rec) * 8 + u64::from(fld);
        isize::try_from(off)
            .unwrap_or_else(|_| panic!("Store offset overflow: rec={rec} fld={fld}"))
    }

    /// Byte offset of `width` bytes at field `fld` of record `rec`, **checked
    /// against this store's capacity on every target and in every build**.
    ///
    /// This is the guard that catches a corrupted `DbRef`.  It is deliberately
    /// always-on rather than a `debug_assert!`, and the reason is loft#950.  The
    /// only bound that used to survive a release build was `checked_offset`'s
    /// `isize::try_from`, which can fail solely where `isize` is 32 bits.  So the
    /// same corrupt `rec` trapped in a browser page and, on every 64-bit backend,
    /// computed a representable offset and read whatever lay at it — a silently
    /// wrong scalar, or a wild write through [`Self::addr_mut`].  "The browser
    /// build traps and the interpreter is green" was therefore evidence about
    /// where the guard could speak, not about where the corruption was.
    ///
    /// Reading a report: a `rec` that trips this is not off by one, it is garbage,
    /// so the fault to hunt is whatever produced the reference — a store-lifetime
    /// bug, not an indexing bug.  `LOFT_STRICT_STORES=1` names the free
    /// ([DEBUG.md](../doc/claude/DEBUG.md)).
    ///
    /// The price is one `u64` compare and a not-taken branch per access.  Measured
    /// by instruction count (the box was loaded, so wall clock could not settle) on
    /// a loop that does nothing but read and write struct fields — the worst case by
    /// construction: **+2.5 % on `--native`, +9.4 % on `--interpret`**.  Native is
    /// the default backend and pays least, and loft#885's hoisted element reads
    /// derive their address once per loop and never come through here at all.
    #[inline]
    fn offset_in_bounds(&self, rec: u32, fld: u32, width: usize) -> isize {
        let off = u64::from(rec) * 8 + u64::from(fld);
        if off + width as u64 > u64::from(self.size) * 8 {
            self.raise_out_of_bounds(rec, fld, width);
        }
        // The bound just proved `off` addresses a byte of this store's live
        // allocation, and that allocation's `Layout` was accepted — so `off`
        // fits `isize` on every target, and no second `try_from` is needed.
        off as isize
    }

    /// The message for [`Self::offset_in_bounds`], kept out of line so the guard
    /// itself stays a compare and a not-taken branch.
    #[cold]
    #[inline(never)]
    fn raise_out_of_bounds(&self, rec: u32, fld: u32, width: usize) -> ! {
        panic!(
            "Store access out of bounds: rec={rec} fld={fld} width={width} \
             store_bytes={} type={} — the reference is corrupt, not merely \
             out of range; run with LOFT_STRICT_STORES=1 to name the free",
            u64::from(self.size) * 8,
            self.known_type,
        )
    }

    #[inline]
    pub fn addr<T>(&self, rec: u32, fld: u32) -> &T {
        let at = self.offset_in_bounds(rec, fld, std::mem::size_of::<T>());
        // Validate field offset against record's claimed size (first word).
        // rec=0 and rec=1 are special (store header / primary record).
        #[cfg(debug_assertions)]
        if rec > 1 && fld > 0 {
            let rec_header = unsafe {
                std::ptr::read_unaligned(
                    self.ptr
                        .add(Self::checked_offset(rec, 0) as usize)
                        .cast::<i32>(),
                )
            };
            let rec_size = rec_header.unsigned_abs() as isize * 8;
            debug_assert!(
                (fld as isize + std::mem::size_of::<T>() as isize) <= rec_size,
                "Fld {fld} is outside of record {rec} size {rec_size}",
            );
        }
        unsafe {
            let off = self.ptr.offset(at).cast::<T>();
            off.as_mut().expect("Reference")
        }
    }

    #[inline]
    pub fn addr_mut<T: 'static>(&mut self, rec: u32, fld: u32) -> &mut T {
        // Only hard `read_only` blocks writes.  Call-bracket
        // `free_protected` lets writes through (only frees are blocked).
        //
        // @FR-H-WriteLocked — the write half of the lock, and the last place that can refuse:
        // a locked store never takes a write, so the rule's "never a silent successful write"
        // holds here whatever route reached it.  What this site cannot do is tell WHOSE
        // mistake it is — `read_only` is one flag for the const store, a worker borrow and the
        // author's own `d#lock`, and only the last is a user error that deserves a loft fault
        // with the author's line (loft#1405).  The assert is deliberately not a
        // `debug_assert`: a release build must refuse too.
        debug_assert!(
            !self.read_only,
            "Write to read-only store at rec={rec} fld={fld} (locked by: {})",
            self.lock_origin
        );
        let at = self.offset_in_bounds(rec, fld, std::mem::size_of::<T>());
        #[cfg(debug_assertions)]
        if rec > 1 && fld > 0 {
            let rec_header = unsafe {
                std::ptr::read_unaligned(
                    self.ptr
                        .add(Self::checked_offset(rec, 0) as usize)
                        .cast::<i32>(),
                )
            };
            let rec_size = rec_header.unsigned_abs() as isize * 8;
            debug_assert!(
                (fld as isize + std::mem::size_of::<T>() as isize) <= rec_size,
                "Fld {fld} is outside of record {rec} size {rec_size}",
            );
        }
        assert!(
            !self.read_only,
            "Write to read-only store at rec={rec} fld={fld} (locked by: {})",
            self.lock_origin
        );
        // @PLN154 — the one write hook.  Phase 0 counted 33 sites that write the
        // interpreter stack and 32 of them arrive here, where `T` also names the width;
        // the typed accessor above them carries only 74.5 % of the bytes.  Off, this is
        // the length load and the not-taken branch.
        if !self.init_shadow.is_empty() {
            self.shadow_write(
                at as usize,
                std::mem::size_of::<T>(),
                crate::stack_verify::kind_of::<T>(),
            );
        }
        unsafe {
            let off = self.ptr.offset(at).cast::<T>();
            off.as_mut().expect("Reference")
        }
    }

    /// A raw pointer to `len` bytes at `(rec, fld)`, for a caller that writes the whole
    /// span in one untyped move.
    ///
    /// @PLN154 — the difference from `addr_mut::<u8>` is that the bound and the shadow tag
    /// both cover `len` rather than the one byte the type names.  Seven sites in the
    /// interpreter restore a coroutine frame, overlay a worker's stack or slide a yielded
    /// value this way; each is a route the phase-0 census counted and none of them can be
    /// reached through a typed accessor, so the primitive is where the two facts live
    /// instead of being repeated at every site.
    ///
    /// # Safety
    /// The caller writes at most `len` bytes from the returned pointer.  The span is
    /// bounds-checked here.
    pub fn addr_span_mut(&mut self, rec: u32, fld: u32, len: usize) -> *mut u8 {
        assert!(
            !self.read_only,
            "Write to read-only store at rec={rec} fld={fld} (locked by: {})",
            self.lock_origin
        );
        let at = self.offset_in_bounds(rec, fld, len);
        if !self.init_shadow.is_empty() {
            self.shadow_write(at as usize, len, crate::stack_verify::OPAQUE);
        }
        unsafe { self.ptr.offset(at) }
    }

    pub fn buffer(&mut self, rec: u32) -> &mut [u8] {
        // The header word counts itself: `claim(n)` reserves `n` words at `rec*8`, of
        // which the first IS the size word this reads.  The payload therefore starts one
        // word in and is one word SHORTER than the record — a span of `size` bytes from
        // offset 8 runs exactly 8 bytes past the record's end (loft#970).
        let size = (*self.addr::<u32>(rec, 0) as usize).saturating_sub(1) * 8;
        // The length comes from the record's own header, so a corrupt header sizes
        // the slice — and a FREED record's header is negative, which reading it as
        // `u32` turns into a span of gigabytes.  Bound it before it becomes a slice.
        let at = self.offset_in_bounds(rec, 8, size);
        // @PLN154 — a `&mut [u8]` handed out is a write of the whole span as far as the
        // shadow can tell; the caller need not come back through a typed accessor.
        if !self.init_shadow.is_empty() {
            self.shadow_write(at as usize, size, crate::stack_verify::OPAQUE);
        }
        unsafe {
            let p = self.ptr.offset(at);
            std::slice::from_raw_parts_mut(p, size)
        }
    }

    /// @PLN16.J — read `len` raw bytes of a record starting at byte offset `off`
    /// (the same `(rec, off)` addressing as [`addr`](Self::addr)).  The store
    /// change journal uses this to snapshot a record region for undo / replay.
    #[must_use]
    pub fn read_span(&self, rec: u32, off: u32, len: u32) -> Box<[u8]> {
        debug_assert!(
            Self::checked_offset(rec, off) + len as isize <= self.size as isize * 8,
            "read_span out of bounds: rec={rec} off={off} len={len} store_size={}",
            self.size * 8,
        );
        let at = self.offset_in_bounds(rec, off, len as usize);
        let mut out = vec![0u8; len as usize];
        if len > 0 {
            // SAFETY: the span is bounds-checked above; `out` holds `len` bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(self.ptr.offset(at), out.as_mut_ptr(), len as usize);
            }
        }
        out.into_boxed_slice()
    }

    /// @PLN16.J — write `bytes` into a record at byte offset `off` — the journal's
    /// only mutator (undo restores `before`, replay writes `after`).  A pure byte
    /// restore: no allocator interaction, so it never moves or resizes a record.
    /// Honors the hard `read_only` lock.
    pub fn write_span(&mut self, rec: u32, off: u32, bytes: &[u8]) {
        assert!(
            !self.read_only,
            "write_span on read-only store at rec={rec} (locked by: {})",
            self.lock_origin
        );
        debug_assert!(
            Self::checked_offset(rec, off) + bytes.len() as isize <= self.size as isize * 8,
            "write_span out of bounds: rec={rec} off={off} len={} store_size={}",
            bytes.len(),
            self.size * 8,
        );
        let at = self.offset_in_bounds(rec, off, bytes.len());
        if !bytes.is_empty() {
            // SAFETY: the span is bounds-checked above; `bytes` is `bytes.len()` long.
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.ptr.offset(at), bytes.len());
            }
        }
    }

    /// Fast check whether a value looks like a valid live record.
    /// Used by `get_ref()` to detect inline data that was misinterpreted
    /// as a record pointer.  Cheaper than `HashSet` lookup — just a range
    /// check and one memory read (the record header).
    #[must_use]
    pub fn is_valid_record(&self, rec: u32) -> bool {
        // Record must be within the store's allocated space and have a
        // positive header (live records have size > 0; freed have size < 0).
        rec > 0 && rec < self.size && *self.addr::<i32>(rec, 0) > 0
    }

    /// Try to validate a record reference as much as possible.
    /// Complete validations are only done in 'test' mode.
    pub fn valid(&self, rec: u32, fld: u32) -> bool {
        // S29/P1-R3: locked (worker) stores have empty claims by design — skip the
        // claims check.  Records in worker stores are valid copies of the originals.
        // Likewise a REOPENED file-backed (mmap) store: `claims` is in-memory
        // bookkeeping and is not persisted, so a fresh `Store::open` of an
        // existing image has an empty set while its records are live (the same
        // reason poison skips file-backed stores).
        debug_assert!(
            self.read_only || self.is_file_backed() || self.claims.contains(&rec),
            "Unknown record {rec}"
        );
        // Read size before any multiplication to avoid overflow when fld 0 is negative
        // (a negative header means the block was freed — a bug if still in claims).
        let size: i32 = *self.addr(rec, 0);
        debug_assert!(
            size > 0,
            "Freed record {rec} (size={size}) accessed at fld {fld}"
        );
        debug_assert!(
            fld >= 4 && fld < 8 * size as u32,
            "Fld {fld} is outside of record {rec} size {}",
            8 * size as u32
        );
        debug_assert!(
            rec != 0 && u64::from(rec) * 8 + u64::from(fld) <= u64::from(self.size) * 8,
            "Reading outside store ({rec}.{fld}) > {}",
            self.size
        );
        if fld != 0 {
            // The first 4 positions are reserved for the record size
            debug_assert!(
                rec + size as u32 <= self.size,
                "Inconsistent record {rec} size {size} > {}",
                self.size
            );
            debug_assert!(
                fld >= 4,
                "Field {fld} too low, overlapping with size on ({rec}.{fld})"
            );
            debug_assert!(
                size >= 1 && fld <= size as u32 * 8,
                "Reading fields outside record ({rec}.{fld}) > {size}"
            );
        }
        true
    }

    /// The payload length of `rec` in bytes, read from the size word it claims.
    ///
    /// A record's first word is its size, and every whole-payload walk derives its
    /// length from that word as `size * 8 - 4`.  A size that is not POSITIVE is
    /// already a corrupted record — freed, or never written — and the subtraction
    /// then wraps: `0 * 8 - 4` is ~18 exabytes, so the caller reads or writes off
    /// the end of the store and the process dies inside `memcpy`.  That reports the
    /// copy, which is innocent, and says nothing about the corruption that reached
    /// it; loft#810 is a SIGSEGV in the value channel for exactly this reason, with
    /// a debug build's `attempt to subtract with overflow` pointing at the same
    /// innocent line.
    ///
    /// So refuse here instead, naming the record and its claimed size.  `assert!`
    /// rather than `debug_assert!` deliberately: a debug build already catches the
    /// underflow, and it is the RELEASE build — where the wrap is silent — that
    /// needs the guard.  This does not fix any cause; it converts an unbounded
    /// access into a report at the first read that cannot be satisfied.
    #[inline]
    fn payload_bytes(&self, rec: u32, op: &str) -> usize {
        let size = *self.addr::<i32>(rec, 0);
        assert!(
            size >= 1,
            "{op}: record {rec} claims size {size}, but a record's size word must be \
             positive — it has been freed or was never written, so its payload length \
             cannot be derived (loft#810)"
        );
        size as usize * 8 - 4
    }

    #[inline]
    /// Copy only the content of a record, not the claimed size
    fn copy(&self, rec: u32, into: u32) {
        let bytes = self.payload_bytes(rec, "Store::copy");
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.ptr.offset(rec as isize * 8 + 4),
                self.ptr.offset(into as isize * 8 + 4),
                bytes,
            );
        }
    }

    #[inline]
    pub fn zero_fill(&self, rec: u32) {
        let bytes = self.payload_bytes(rec, "Store::zero_fill");
        unsafe {
            std::ptr::write_bytes(self.ptr.offset(rec as isize * 8 + 4), 0, bytes);
        }
    }

    /// Zero `len` bytes at byte offset `pos` within record `rec`.  `claim`
    /// reuses freed blocks without clearing them, so a freshly-allocated vector
    /// element (or struct record) carries garbage; callers materialising into
    /// it must clear it first so unwritten vector-header sub-fields read as
    /// empty (`vec_rec = 0`) instead of dereferencing a junk record id.
    #[inline]
    pub fn zero_range(&mut self, rec: u32, pos: u32, len: u32) {
        if len == 0 {
            return;
        }
        let ptr: *mut u8 = self.addr_mut::<u8>(rec, pos);
        // SAFETY: callers pass a record/element's own `pos..pos+len`, which lies
        // within its claimed allocation.
        unsafe { std::ptr::write_bytes(ptr, 0, len as usize) }
        // @PLN154 — `addr_mut::<u8>` above tagged one byte; the write covers `len`.
        if !self.init_shadow.is_empty() {
            let at = (rec as usize) * 8 + pos as usize;
            self.shadow_write(at, len as usize, crate::stack_verify::OPAQUE);
        }
    }

    #[inline]
    pub fn copy_block(
        &mut self,
        from_rec: u32,
        from_pos: isize,
        to_rec: u32,
        to_pos: isize,
        size: isize,
    ) {
        #[cfg(debug_assertions)]
        {
            let from_limit = *self.addr::<i32>(from_rec, 0) as isize * 8;
            let to_limit = *self.addr::<i32>(to_rec, 0) as isize * 8;
            debug_assert!(
                from_pos + size <= from_limit,
                "copy_block src OOB: rec={from_rec} [{from_pos}..+{size}] > {from_limit} bytes"
            );
            debug_assert!(
                to_pos + size <= to_limit,
                "copy_block dst OOB: rec={to_rec} [{to_pos}..+{size}] > {to_limit} bytes"
            );
        }
        unsafe {
            std::ptr::copy(
                self.ptr.offset(from_rec as isize * 8 + from_pos),
                self.ptr.offset(to_rec as isize * 8 + to_pos),
                size as usize,
            );
        }
        // @PLN154 — a raw byte move carries its tags, so a span the source never wrote
        // arrives unwritten and is caught at the read rather than at this copy.
        if !self.init_shadow.is_empty() {
            self.shadow_move(
                (from_rec as isize * 8 + from_pos) as usize,
                (to_rec as isize * 8 + to_pos) as usize,
                size as usize,
            );
        }
    }

    #[inline]
    pub fn copy_block_between(
        &self,
        from_rec: u32,
        from_pos: isize,
        to_store: &mut Store,
        to_rec: u32,
        to_pos: isize,
        len: isize,
    ) {
        #[cfg(debug_assertions)]
        {
            let from_limit = *self.addr::<i32>(from_rec, 0) as isize * 8;
            let to_limit = *to_store.addr::<i32>(to_rec, 0) as isize * 8;
            debug_assert!(
                from_pos + len <= from_limit,
                "copy_block_between src OOB: rec={from_rec} [{from_pos}..+{len}] > {from_limit} bytes"
            );
            debug_assert!(
                to_pos + len <= to_limit,
                "copy_block_between dst OOB: rec={to_rec} [{to_pos}..+{len}] > {to_limit} bytes"
            );
        }
        unsafe {
            std::ptr::copy(
                self.ptr.offset(from_rec as isize * 8 + from_pos),
                to_store.ptr.offset(to_rec as isize * 8 + to_pos),
                len as usize,
            );
        }
        // @PLN154 — the tags follow the bytes across stores too.  A source with no shadow
        // is real data, so the destination is tagged WRITTEN; only a shadowed source can
        // hand over an unwritten span.
        if to_store.shadow_armed() {
            let at = (to_rec as isize * 8 + to_pos) as usize;
            if self.shadow_armed() {
                let tags =
                    self.shadow_tags((from_rec as isize * 8 + from_pos) as usize, len as usize);
                to_store.shadow_set_tags(at, &tags);
            } else {
                to_store.shadow_write(at, len as usize, crate::stack_verify::OPAQUE);
            }
        }
    }

    #[inline]
    pub fn get_int(&self, rec: u32, fld: u32) -> i64 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            i64::MIN
        }
    }

    #[inline]
    pub fn set_int(&mut self, rec: u32, fld: u32, val: i64) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    /// Word count of a live record, read from its size header (fld 0).
    /// The header word doubles as DATA for hash bucket records: a
    /// bucket's `room` IS its record size (`hash.rs::add` claims `room`
    /// words), so this is the one legitimate fld-0 read — `valid()`'s
    /// data-field gate (`fld >= 4`) correctly rejects it via the
    /// ordinary accessors.
    #[must_use]
    pub fn record_words(&self, rec: u32) -> u32 {
        // Same tolerance as `valid()`: a FILE-BACKED store does not populate `claims`
        // (its records come from the mapped image, not from this process's allocator),
        // so membership says nothing there.  Measured: the same bucket record is in
        // `claims` for a heap store and absent for the bound one, so without this the
        // header read asserts "Unknown record" for every bound store — which is
        // precisely the store `store_verify` is most often asked about.
        debug_assert!(
            self.read_only || self.is_file_backed() || self.claims.contains(&rec),
            "Unknown record {rec}"
        );
        let size: i32 = *self.addr(rec, 0);
        debug_assert!(
            size > 0,
            "Freed record {rec} (size={size}) read as a record header"
        );
        size as u32
    }

    /// Does `rec` name a CLAIMED block in this arena?  Bounds + block header,
    /// checked for real in release builds — unlike [`Self::valid`], whose
    /// checks are `debug_assert`s.
    ///
    /// For a record number that arrives as **data** rather than from the
    /// allocator: the iteration scratch's header names its rec-nr vector, and a
    /// stale or corrupt value there must be refused, not deleted
    /// ([`crate::database::Stores::free_iteration_scratch`]).  `claims` cannot
    /// answer this — it is in-memory bookkeeping a re-opened file-backed store
    /// does not have.
    #[must_use]
    pub fn is_claimed_record(&self, rec: u32) -> bool {
        rec > PRIMARY && rec < self.size && *self.addr::<i32>(rec, 0) > 0
    }

    /// 4-byte unsigned raw read — for internal collection headers
    /// (vector `[rec:u32][len:u32]`, hash buckets, tree node ptrs,
    /// string-record lengths).  NOT for user `integer` fields.
    #[inline]
    pub fn get_u32_raw(&self, rec: u32, fld: u32) -> u32 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            0
        }
    }

    /// Read a COLLECTION SLOT — the 4-byte record id a vector/keyed field or local holds.
    ///
    /// Identical to [`get_u32_raw`] except that it maps the reserved absent id
    /// ([`DbRef::ABSENT_REC`], loft#917) to `0`. Every consumer that asks "which record
    /// holds this collection's elements?" wants that: absent and empty are the same answer
    /// to that question — no records — and differ only for the null TEST, which reads the
    /// raw slot through `vector::is_absent_collection`.
    ///
    /// This exists so the distinction costs one accessor rather than a decision at each of
    /// the twenty-odd sites that dereference a slot. Reading the raw value there instead
    /// takes `u32::MAX` for a record number: `get_u32_raw(MAX, 4)` is out of bounds, and a
    /// missed site is a SIGSEGV rather than a wrong answer.
    ///
    /// That prediction came true where the accessor had not reached: every call site was in
    /// `vector.rs`, and the KEYED family read its slots raw, so appending to a `hash<E[k]>?`
    /// field left absent dereferenced the marker (loft#1213). The keyed side now asks the same
    /// question through the same accessor — `hash::ensure_table` and `hash::add` on the write
    /// side — and `Stores::find` states it once above its kind dispatch, so a lookup on an
    /// absent collection misses instead of following the marker.
    #[inline]
    pub fn collection_rec(&self, rec: u32, fld: u32) -> u32 {
        let v = self.get_u32_raw(rec, fld);
        if v == crate::keys::DbRef::ABSENT_REC {
            0
        } else {
            v
        }
    }

    /// 4-byte unsigned raw write — counterpart of [`get_u32_raw`].
    #[inline]
    pub fn set_u32_raw(&mut self, rec: u32, fld: u32, val: u32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    /// 4-byte signed raw read — for internal type tags and sentinel
    /// comparisons that must stay `i32::MIN`-relative (e.g. `File.ref`).
    #[inline]
    pub fn get_i32_raw(&self, rec: u32, fld: u32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            i32::MIN
        }
    }

    /// 4-byte signed raw write — counterpart of [`get_i32_raw`].
    #[inline]
    pub fn set_i32_raw(&mut self, rec: u32, fld: u32, val: i32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn get_long(&self, rec: u32, fld: u32) -> i64 {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            i64::MIN
        }
    }

    #[inline]
    pub fn set_long(&mut self, rec: u32, fld: u32, val: i64) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn get_short(&self, rec: u32, fld: u32, min: i32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            let read: u16 = *self.addr(rec, fld);
            if read != 0 {
                i32::from(read) + min - 1
            } else {
                i32::MIN
            }
        } else {
            i32::MIN
        }
    }

    #[inline]
    pub fn set_short(&mut self, rec: u32, fld: u32, min: i32, val: i32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            if val == i32::MIN {
                // The `u16` suffix is load-bearing: a bare `0` infers `i32`, so
                // `addr_mut::<i32>` writes 4 bytes and zeroes the two packed bytes
                // after this 2-byte field (silent sibling corruption on a null store).
                *self.addr_mut(rec, fld) = 0u16;
                true
            } else if Self::short_fits(min, val) {
                *self.addr_mut(rec, fld) = (val - min + 1) as u16;
                true
            } else {
                // loft#984 — OUT OF RANGE: the DEFAULT (lowest in range), encoded.  The
                // old bound was off by TWO: this encoding is `val - min + 1` and reserves
                // raw 0 for null, so `min + 65535` encodes to 0 (reads back as NULL) and
                // `min + 65536` to 1 (reads back as `min`).
                *self.addr_mut(rec, fld) = 1u16;
                false
            }
        } else {
            false
        }
    }

    /// Read a `Parts::ShortRaw` narrow-vector element.  ShortRaw is ALWAYS
    /// non-nullable (`narrow_vector_content` / `vectors.rs` build it with
    /// `nullable = false`), so it reserves NO sentinel — the full 2-byte range,
    /// identical to [`get_short_full`](Self::get_short_full) /
    /// [`get_byte`](Self::get_byte) (`read + min`; stored as `(val - min) as u16`
    /// by `set_i16_raw`, the raw-byte-copy path `vector_add` relies on).
    ///
    /// H6: it used to decode `u16::MAX → null`, which silently nulled a
    /// `vector<u16>`'s `65535` / `vector<i16>`'s `32767` — inconsistent with
    /// `vector<u8>` holding the full `0..=255` via `Byte`.  The write twin
    /// `set_i16_raw` keeps its `i32::MIN → u16::MAX` clamp purely as an underflow
    /// guard; narrow vectors store concrete values, never null (like `vector<u8>`).
    #[inline]
    pub fn get_i16_raw(&self, rec: u32, fld: u32, min: i32) -> i32 {
        self.get_short_full(rec, fld, min)
    }

    /// direct 2-byte write for `Parts::ShortRaw` vector
    /// elements.  Mirrors `set_byte`: stores `(val - min) as u16`.
    /// `val == i32::MIN` stores as `u16::MAX` (null sentinel).
    #[inline]
    pub fn set_i16_raw(&mut self, rec: u32, fld: u32, min: i32, val: i32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            if val == i32::MIN {
                *self.addr_mut(rec, fld) = u16::MAX;
                true
            } else if Self::short_raw_fits(min, val) {
                *self.addr_mut(rec, fld) = (val - min) as u16;
                true
            } else {
                // loft#984 — this had NO range check: `(val - min) as u16` is a
                // TRUNCATING cast, so `70000` into a `limit(0, 65535)` field silently
                // became `4464`.  Out of range now stores the DEFAULT (lowest in range)
                // and reports `false` so the caller warns.
                *self.addr_mut(rec, fld) = 0u16;
                false
            }
        } else {
            false
        }
    }

    /// Direct 2-byte read for a NOT-NULL `u16`/`i16` field (`Parts::ShortFull`):
    /// the full 65536-value range, NO null sentinel — `read + min`
    /// unconditionally, the 2-byte twin of [`get_byte`](Self::get_byte).  Unlike
    /// `get_short` (`+1` encoding, reserves `0`, swallows `65535` as null), this
    /// reserves nothing, so a not-null `u16` can hold `65535`.  The `ShortRaw`
    /// narrow-vector read `get_i16_raw` now delegates here (same full-range
    /// decode).  The write reuses `set_i16_raw` (`OpSetShortRaw`): `(val - min)
    /// as u16`, matching this decode.
    #[inline]
    pub fn get_short_full(&self, rec: u32, fld: u32, min: i32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            let read: u16 = *self.addr(rec, fld);
            i32::from(read) + min
        } else {
            i32::MIN
        }
    }

    #[inline]
    pub fn get_byte(&self, rec: u32, fld: u32, min: i32) -> i32 {
        if rec != 0 && self.valid(rec, fld) {
            let read: u8 = *self.addr(rec, fld);
            i32::from(read) + min
        } else {
            i32::MIN
        }
    }

    /// loft#984 — the ONE range predicate per narrow width, read by BOTH the setter
    /// (which stores the type's default when it fails) and `Stores`' warning wrapper
    /// (which says so).  Two derivations of "does this fit" is how a field came to be
    /// stored three different wrong ways — aliased, wrapped, and dropped.
    ///
    /// `i32::MIN` is the intentional-null write, never an out-of-range one.
    #[must_use]
    pub const fn byte_fits(min: i32, val: i32) -> bool {
        val == i32::MIN || (val >= min && val <= min + 255)
    }

    /// See [`byte_fits`](Self::byte_fits).  The `+1` encoding reserves raw 0 for null,
    /// so the top of the range is `min + 65534`.
    #[must_use]
    pub const fn short_fits(min: i32, val: i32) -> bool {
        val == i32::MIN || (val >= min && val <= min + 65534)
    }

    /// See [`byte_fits`](Self::byte_fits).  The raw encoding reserves nothing in the
    /// non-null form, so it spans the whole 2 bytes.
    #[must_use]
    pub const fn short_raw_fits(min: i32, val: i32) -> bool {
        val == i32::MIN || (val >= min && val <= min + 65535)
    }

    #[inline]
    pub fn set_byte(&mut self, rec: u32, fld: u32, min: i32, val: i32) -> bool {
        if rec != 0 && self.valid(rec, fld) {
            if val == i32::MIN {
                // The `u8` suffix is load-bearing: a bare `255` infers `i32`, so
                // `addr_mut::<i32>` writes 4 bytes and zeroes the three packed
                // fields after this one (silent sibling corruption on a null store).
                *self.addr_mut(rec, fld) = 255u8;
                true
            } else if Self::byte_fits(min, val) {
                *self.addr_mut(rec, fld) = (val - min) as u8;
                true
            } else {
                // loft#984 — OUT OF RANGE: store the type's DEFAULT, the lowest value in
                // range, and report `false` so the caller warns.  Never the old two
                // answers: `min + 256` used to be admitted and stored `256 as u8` = 0,
                // so it ALIASED onto `min`; anything further out was dropped, leaving
                // whatever the field held before.  A byte encodes `val - min`, so the
                // range is `min ..= min + 255` — one value, not 257.
                *self.addr_mut(rec, fld) = 0u8;
                false
            }
        } else {
            false
        }
    }

    /// True when the `text` slot at `(rec, pos)` holds NULL.
    ///
    /// @FR-L-Null-Text — an absent text has TWO byte-level spellings and they are one
    /// absence: an **unset handle** (str_rec 0, the `nullref` sentinel `(L-Null)` names)
    /// and an **allocated record holding the `STRING_NULL` (`"\0"`) bytes**, which is what
    /// a written `null` and every native text-null produce.  Testing the handle alone reads
    /// the second spelling as a present, one-character string — and telling `null` from `""`
    /// is the distinction the whole reading exists for.
    ///
    /// The CONTENT test is the total one: [`Store::get_str`] maps an unset or out-of-range
    /// handle onto `STRING_NULL` too, so it subsumes the pointer test rather than sitting
    /// beside it.  This is the one home every reader asks — `Stores::is_null` (which decides
    /// whether a struct field is omitted), the format walk (which decides how it renders) and
    /// the native reader — so a render and an omission cannot disagree about whether a value
    /// is there.
    ///
    /// An empty string is NOT null: `""` is an allocated record whose content is `""`, a
    /// present value that must survive a save→load round trip (@P375).
    #[inline]
    #[must_use]
    pub fn text_is_null(&self, rec: u32, pos: u32) -> bool {
        self.get_str(self.get_u32_raw(rec, pos)) == crate::state::STRING_NULL
    }

    #[inline]
    pub fn get_str<'a>(&self, rec: u32) -> &'a str {
        if rec == 0 || rec > i32::MAX as u32 {
            return crate::state::STRING_NULL;
        }
        let len = self.get_u32_raw(rec, 4);
        if (len / 8) + rec > self.size {
            return crate::state::STRING_NULL;
        }
        unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                self.ptr.offset(rec as isize * 8 + 8),
                len as usize,
            ))
        }
    }

    #[inline]
    pub fn set_str(&mut self, val: &str) -> u32 {
        let res = self.claim(((val.len() + 15) / 8) as u32);
        self.set_u32_raw(res, 4, val.len() as u32);
        unsafe {
            std::ptr::copy_nonoverlapping(
                val.as_ptr(),
                self.ptr.offset(res as isize * 8 + 8),
                val.len(),
            );
        }
        res
    }

    #[inline]
    pub fn set_str_ptr(&mut self, ptr: *const u8, len: usize) -> u32 {
        let res = self.claim(((len + 15) / 8) as u32);
        self.set_u32_raw(res, 4, len as u32);
        unsafe {
            std::ptr::copy_nonoverlapping(ptr, self.ptr.offset(res as isize * 8 + 8), len);
        }
        res
    }

    #[inline]
    pub fn append_str(&mut self, record: u32, val: &str) -> u32 {
        let prev = self.get_u32_raw(record, 4);
        let result = self.resize(record, (prev as usize + val.len()).div_ceil(8) as u32);
        unsafe {
            std::ptr::copy_nonoverlapping(
                val.as_ptr(),
                self.ptr.offset(result as isize * 8 + 8 + prev as isize),
                val.len(),
            );
        }
        result
    }

    #[inline]
    pub fn get_boolean(&self, rec: u32, fld: u32, mask: u8) -> bool {
        if self.valid(rec, fld) {
            let read: u8 = *self.addr(rec, fld);
            (read & mask) > 0
        } else {
            false
        }
    }

    #[inline]
    pub fn set_boolean(&mut self, rec: u32, fld: u32, mask: u8, val: bool) -> bool {
        if self.valid(rec, fld) {
            let current: u8 = *self.addr(rec, fld);
            let mut write = current & !mask;
            if val {
                write |= mask;
            }
            *self.addr_mut(rec, fld) = write;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn get_float(&self, rec: u32, fld: u32) -> f64 {
        // @P284 — guard `rec != 0` like the integer getters do.  In release
        // mode `valid()` is a no-op (all the inner checks are `debug_assert`)
        // so without this guard a null DbRef (rec=0) reads `*self.addr(0, 0)`
        // — the store's free-list header bytes interpreted as f64.  That
        // garbage value (~2.8e-282 in practice) is finite, so for-loop
        // iteration over `vector<float>` saw a non-null value past the end
        // and looped forever.
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            f64::NAN
        }
    }

    #[inline]
    pub fn set_float(&mut self, rec: u32, fld: u32, val: f64) -> bool {
        if self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }

    #[inline]
    pub fn get_single(&self, rec: u32, fld: u32) -> f32 {
        // @P284 — sibling of `get_float`'s rec=0 guard; needed for
        // `for s in vector<single>` to terminate.
        if rec != 0 && self.valid(rec, fld) {
            *self.addr(rec, fld)
        } else {
            f32::NAN
        }
    }

    #[inline]
    pub fn set_single(&mut self, rec: u32, fld: u32, val: f32) -> bool {
        if self.valid(rec, fld) {
            *self.addr_mut(rec, fld) = val;
            true
        } else {
            false
        }
    }
}

// Safety: worker threads only call `addr()` (read-only) on locked stores.
// `addr_mut()` on a locked store always panics.
unsafe impl Send for Store {}

// =============================================================================
// @PLAN38 — durable-store support (phases 00 + 01 — IntegrityOnly tier).
//
// Design: a 40-byte `.dmeta` sidecar file alongside the main store file holds
// signature + tier + CRCs.  The main store file is bit-for-bit identical to
// a legacy (non-durable) store — durability is a metadata layer, not a
// payload-layout change.  See
// `doc/claude/plans/43-loft-store-durable/00-foundation.md`.
//
// Phase-01 reach: nothing inside the `loft` binary calls these yet (the
// training-port consumer that drives @PLAN38 lives in another repo and
// will reach them via a future loft-language builtin).  Tests under
// `tests/store_durable_*.rs` exercise them.  `#[allow(dead_code)]` markers
// on each top-level item keep the bin-side dead-code lint quiet until a
// runtime caller lands; remove them when the first in-tree caller wires up.
// =============================================================================

/// Sidecar signature bytes — `"DStoreV1"` (no NUL terminator).
#[allow(dead_code)]
const DURABLE_SIGNATURE: [u8; 8] = *b"DStoreV1";

/// Total size of a sidecar file in bytes.  Fixed; never grows.
#[allow(dead_code)]
const DURABLE_SIDECAR_BYTES: usize = 40;

/// Tier IDs.  Tier 0 means "not a durable store" and should never appear in a
/// sidecar; it exists so the field has a meaningful default in non-durable
/// `Store` instances.
#[allow(dead_code)]
const TIER_NONE: u16 = 0;
#[allow(dead_code)]
const TIER_INTEGRITY_ONLY: u16 = 1;
// const TIER_SNAPSHOT_EVERY: u16 = 2; // reserved — phase 02
// const TIER_WAL: u16 = 3;            // reserved — phase 03

/// On-disk format detected for a store path.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreFormat {
    /// Legacy non-durable store — main file has no `.dmeta` sidecar.
    /// `Store::open` is the right entry point.
    Legacy,
    /// Durable store — `.dmeta` sidecar is present.  The `u16` is the
    /// `tier_id` recorded in the sidecar (1 = IntegrityOnly, etc.).
    Durable(u16),
}

/// Integrity verdict for a durable store, returned by [`Store::validate_integrity`].
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreIntegrity {
    /// All checks passed; the main file matches the sidecar exactly.
    Clean,
    /// At least one check failed; the consumer must rebuild.  The
    /// `CorruptReason` says which check tripped first (in priority order).
    Corrupt(CorruptReason),
}

/// Reason a durable store failed integrity validation.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorruptReason {
    /// Sidecar signature bytes were not `"DStoreV1"`.
    SignatureMismatch,
    /// Sidecar's own header_crc did not match a recomputed CRC of bytes 0..12.
    HeaderCrcMismatch,
    /// Sidecar file does not exist.  Semantic: clean-close protocol never
    /// ran, so the on-disk payload state is unknown.  Treat as corruption.
    TailMarkerMissing,
    /// Sidecar's `payload_crc` did not match the recomputed CRC of the
    /// main file's bytes (within the recorded `payload_len`).
    TailCrcMismatch,
    /// Main file's actual on-disk byte length differs from sidecar's
    /// `payload_len`.  Either the file was truncated after the sidecar
    /// was written, or the sidecar predates a resize that never completed.
    TruncatedFile,
    /// @PLN97 Phase D — the bytes are INTACT (integrity clean) but were
    /// written under a DIFFERENT store layout than the running program's:
    /// reading them raw would silently misinterpret them.  Produced by the
    /// `.dschema` schema-sidecar check (`crate::schema_sidecar`), not by
    /// integrity validation.  A consumer routes it through the same
    /// `on_corruption` rebuild path — the store must be migrated or rebuilt,
    /// never read raw.
    SchemaMismatch,
}

/// Durability mode for [`Store::open_durable`].  Tier 1 is the only variant
/// shipped in phase 01; tiers 2 and 3 land in later phases.
#[allow(dead_code)]
pub enum DurabilityMode {
    /// **Tier 1 — IntegrityOnly.**  No msync discipline on the hot write
    /// path; the OS page cache is trusted for in-flight writes.  On open,
    /// the sidecar is validated; on corruption (or when the file/sidecar
    /// is absent), `on_corruption` is invoked and is expected to rebuild
    /// the store from authoritative sources.  After the callback returns
    /// successfully, `open_durable` retries once.  Cap recursion depth at 1
    /// — if validation still fails after the rebuild, the error is returned
    /// to the caller (no infinite loop).
    ///
    /// Fresh-file note: when the main file does not exist yet (brand-new
    /// database), the same callback fires with `Corrupt(TailMarkerMissing)`.
    /// Consumers MUST implement `on_corruption` as a "rebuild OR initialise"
    /// routine, not a "repair existing file" one.
    IntegrityOnly {
        on_corruption: Box<dyn Fn(&std::path::Path) -> std::io::Result<()>>,
    },
}

/// Decoded sidecar contents.  Module-private; consumers see the public
/// [`StoreIntegrity`] verdict only.
#[allow(dead_code)]
struct SidecarHeader {
    tier_id: u16,
    #[allow(dead_code)] // reserved for future tier-specific behaviour
    flags: u16,
    #[allow(dead_code)] // surfaced for diagnostics; not load-bearing yet
    last_clean_ns: u64,
    payload_len: u64,
    payload_crc: u32,
}

/// Compute the CRC32 of the first 12 bytes of a sidecar.  Used both to
/// write a fresh sidecar and to validate one read off disk.
#[allow(dead_code)]
fn compute_header_crc(bytes: &[u8]) -> u32 {
    debug_assert!(bytes.len() >= 12);
    let mut h = crc32fast::Hasher::new();
    h.update(&bytes[..12]);
    h.finalize()
}

/// Read + parse a sidecar.  Returns `Ok(None)` if the file does not exist,
/// `Ok(Some(...))` if it does and the structural read succeeded.  CRC and
/// signature checks are the caller's job (`detect_format` and
/// `validate_integrity` differ on which checks they apply).
#[allow(dead_code)]
fn read_sidecar(meta_path: &std::path::Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(meta_path) {
        Ok(bytes) => {
            if bytes.len() != DURABLE_SIDECAR_BYTES {
                // A short/long sidecar is structurally invalid.  Treat it as
                // "present but unreadable" — let the caller decide the verdict.
                return Ok(Some(bytes));
            }
            Ok(Some(bytes))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Parse a sidecar byte buffer.  Returns `Err(CorruptReason::...)` if any
/// of: signature, header_crc, or structural length checks fail.  Callers
/// add further checks (payload_crc, payload_len) by comparing against the
/// main file.
#[allow(dead_code)]
fn parse_sidecar(bytes: &[u8]) -> Result<SidecarHeader, CorruptReason> {
    if bytes.len() != DURABLE_SIDECAR_BYTES {
        return Err(CorruptReason::SignatureMismatch);
    }
    if bytes[..8] != DURABLE_SIGNATURE {
        return Err(CorruptReason::SignatureMismatch);
    }
    let tier_id = u16::from_le_bytes([bytes[8], bytes[9]]);
    let flags = u16::from_le_bytes([bytes[10], bytes[11]]);
    let stored_header_crc = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    if stored_header_crc != compute_header_crc(bytes) {
        return Err(CorruptReason::HeaderCrcMismatch);
    }
    let last_clean_ns = u64::from_le_bytes([
        bytes[16], bytes[17], bytes[18], bytes[19], bytes[20], bytes[21], bytes[22], bytes[23],
    ]);
    let payload_len = u64::from_le_bytes([
        bytes[24], bytes[25], bytes[26], bytes[27], bytes[28], bytes[29], bytes[30], bytes[31],
    ]);
    let payload_crc = u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]);
    // bytes[36..40] is reserved; not validated.
    Ok(SidecarHeader {
        tier_id,
        flags,
        last_clean_ns,
        payload_len,
        payload_crc,
    })
}

/// Build a fresh sidecar buffer for a clean-close write.  `payload_len` is
/// the byte length of the main file at the moment of capture, and
/// `payload_crc` is the CRC over those bytes.
#[allow(dead_code)]
fn encode_sidecar(
    tier_id: u16,
    last_clean_ns: u64,
    payload_len: u64,
    payload_crc: u32,
) -> [u8; DURABLE_SIDECAR_BYTES] {
    let mut buf = [0u8; DURABLE_SIDECAR_BYTES];
    buf[..8].copy_from_slice(&DURABLE_SIGNATURE);
    buf[8..10].copy_from_slice(&tier_id.to_le_bytes());
    buf[10..12].copy_from_slice(&0u16.to_le_bytes()); // flags
    let header_crc = compute_header_crc(&buf[..12]);
    buf[12..16].copy_from_slice(&header_crc.to_le_bytes());
    buf[16..24].copy_from_slice(&last_clean_ns.to_le_bytes());
    buf[24..32].copy_from_slice(&payload_len.to_le_bytes());
    buf[32..36].copy_from_slice(&payload_crc.to_le_bytes());
    buf[36..40].copy_from_slice(&0u32.to_le_bytes()); // reserved
    buf
}

/// Compute the CRC32 of the main store file's first `len` bytes.  Used both
/// to write a fresh sidecar (`compute_payload_crc(path, len)` against the
/// freshly-flushed main file) and to validate one (recompute, compare to
/// the sidecar's stored value).
#[allow(dead_code)]
fn compute_payload_crc(main_path: &std::path::Path, len: u64) -> std::io::Result<u32> {
    use std::io::Read;
    let mut f = std::fs::File::open(main_path)?;
    let mut h = crc32fast::Hasher::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut remaining = len;
    while remaining > 0 {
        let want = std::cmp::min(remaining as usize, buf.len());
        let n = f.read(&mut buf[..want])?;
        if n == 0 {
            // File shorter than expected — caller's TruncatedFile check
            // will catch this; here we just stop the hash.
            break;
        }
        h.update(&buf[..n]);
        remaining -= n as u64;
    }
    Ok(h.finalize())
}

/// Write a sidecar atomically: write to `<meta_path>.tmp`, fsync, rename.
/// Cross-platform note: POSIX rename is atomic; Windows ReplaceFile (which
/// `std::fs::rename` uses on Windows) is atomic in the same sense for the
/// destination path.
#[allow(dead_code)]
fn write_sidecar_atomic(
    meta_path: &std::path::Path,
    bytes: &[u8; DURABLE_SIDECAR_BYTES],
) -> std::io::Result<()> {
    use std::io::Write;
    let mut tmp = meta_path.to_path_buf();
    let mut name = tmp
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(".tmp");
    tmp.set_file_name(name);
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, meta_path)?;
    Ok(())
}

/// Trace helper for @PLAN38 debugging.  Writes a line to
/// `/tmp/loft-store-durable.trace` with explicit flush — bypasses Rust
/// stderr's pipe-full-buffering behaviour that swallowed eprintln output
/// during the phase-01 verification session.  Gated on env var
/// `LOFT_STORE_DURABLE_TRACE=1`; zero cost when unset (no file open).
#[allow(dead_code)]
fn dur_trace(line: &str) {
    if std::env::var("LOFT_STORE_DURABLE_TRACE").is_err() {
        return;
    }
    // Honour LOFT_STORE_DURABLE_TRACE_FILE if set (helps when /tmp is
    // sandbox-restricted); otherwise use TMPDIR or /tmp.
    let path = std::env::var("LOFT_STORE_DURABLE_TRACE_FILE").unwrap_or_else(|_| {
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
        format!("{base}/loft-store-durable.trace")
    });
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(f, "{line}");
        let _ = f.flush();
    }
}

/// Path of the metadata sidecar for a given main store path.
/// `tags.store` → `tags.store.dmeta`.
#[allow(dead_code)]
fn dmeta_path(main_path: &std::path::Path) -> std::path::PathBuf {
    let mut p = main_path.as_os_str().to_owned();
    p.push(".dmeta");
    std::path::PathBuf::from(p)
}

#[allow(dead_code)]
impl Store {
    /// **Phase 00.**  Detect the on-disk format for a store at `path`.
    /// Reads only the sidecar; the main file is not opened.
    ///
    /// - Sidecar absent → [`StoreFormat::Legacy`].
    /// - Sidecar present and structurally readable →
    ///   [`StoreFormat::Durable`]`(tier_id)`.  No CRC or signature
    ///   validation is performed here; use [`Store::validate_integrity`]
    ///   for that.
    ///
    /// Cheap (one file read of ≤ 40 bytes).  Suitable to call before
    /// deciding which open path to use.
    pub fn detect_format(path: &std::path::Path) -> std::io::Result<StoreFormat> {
        let meta = dmeta_path(path);
        let raw = read_sidecar(&meta)?;
        match raw {
            None => Ok(StoreFormat::Legacy),
            Some(bytes) => {
                if bytes.len() == DURABLE_SIDECAR_BYTES && bytes[..8] == DURABLE_SIGNATURE {
                    let tier_id = u16::from_le_bytes([bytes[8], bytes[9]]);
                    Ok(StoreFormat::Durable(tier_id))
                } else {
                    // A short/long sidecar or wrong-signature one is a
                    // durable store with damaged metadata.  Reporting
                    // `Durable(0)` would lie about the tier; the cleanest
                    // signal is "this is durable in intent but unreadable"
                    // — surface it via the validate path.  `detect_format`
                    // returns Legacy so callers that ONLY ran detect can
                    // still distinguish "no sidecar" from "broken sidecar"
                    // by calling validate_integrity for the verdict.
                    Ok(StoreFormat::Durable(0))
                }
            }
        }
    }

    /// **Phase 00.**  Validate the integrity of a durable store at `path`.
    ///
    /// Performs:
    /// 1. Sidecar exists → else [`CorruptReason::TailMarkerMissing`].
    /// 2. Sidecar signature is `"DStoreV1"` →
    ///    else [`CorruptReason::SignatureMismatch`].
    /// 3. Sidecar's own `header_crc` matches a recomputed CRC of bytes 0..12
    ///    → else [`CorruptReason::HeaderCrcMismatch`].
    /// 4. Main file's actual byte length matches sidecar's `payload_len`
    ///    → else [`CorruptReason::TruncatedFile`].
    /// 5. CRC32 of the main file's `payload_len` bytes matches the sidecar's
    ///    `payload_crc` → else [`CorruptReason::TailCrcMismatch`].
    ///
    /// Checks short-circuit on the first failure.
    pub fn validate_integrity(path: &std::path::Path) -> std::io::Result<StoreIntegrity> {
        let meta = dmeta_path(path);
        dur_trace(&format!("[validate] entry path={path:?} sidecar={meta:?}"));
        let raw = match read_sidecar(&meta)? {
            None => {
                dur_trace("[validate] sidecar missing → TailMarkerMissing");
                return Ok(StoreIntegrity::Corrupt(CorruptReason::TailMarkerMissing));
            }
            Some(bytes) => bytes,
        };
        dur_trace(&format!("[validate] sidecar read {} bytes", raw.len()));
        let hdr = match parse_sidecar(&raw) {
            Ok(h) => h,
            Err(reason) => {
                dur_trace(&format!("[validate] parse_sidecar → {reason:?}"));
                return Ok(StoreIntegrity::Corrupt(reason));
            }
        };
        dur_trace(&format!(
            "[validate] parsed: tier={} flags={} last_clean_ns={} payload_len={} payload_crc={:#x}",
            hdr.tier_id, hdr.flags, hdr.last_clean_ns, hdr.payload_len, hdr.payload_crc
        ));
        let main_meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                dur_trace("[validate] main file missing → TruncatedFile");
                return Ok(StoreIntegrity::Corrupt(CorruptReason::TruncatedFile));
            }
            Err(e) => return Err(e),
        };
        dur_trace(&format!(
            "[validate] main file len={} (sidecar says {})",
            main_meta.len(),
            hdr.payload_len
        ));
        if main_meta.len() != hdr.payload_len {
            dur_trace("[validate] length mismatch → TruncatedFile");
            return Ok(StoreIntegrity::Corrupt(CorruptReason::TruncatedFile));
        }
        dur_trace(&format!(
            "[validate] computing payload CRC over {} bytes …",
            hdr.payload_len
        ));
        let crc = compute_payload_crc(path, hdr.payload_len)?;
        dur_trace(&format!("[validate] computed CRC = {crc:#x}"));
        if crc != hdr.payload_crc {
            dur_trace("[validate] CRC mismatch → TailCrcMismatch");
            return Ok(StoreIntegrity::Corrupt(CorruptReason::TailCrcMismatch));
        }
        if hdr.tier_id == TIER_NONE {
            dur_trace("[validate] tier_id=0 → SignatureMismatch");
            return Ok(StoreIntegrity::Corrupt(CorruptReason::SignatureMismatch));
        }
        dur_trace("[validate] Clean");
        Ok(StoreIntegrity::Clean)
    }

    /// **Phase 01.**  Open a store with durability guarantees.
    ///
    /// On entry, the sidecar at `<path>.dmeta` is validated.  On any
    /// integrity failure (or when either file is missing), `on_corruption`
    /// is invoked once, then validation is retried.  A second failure
    /// returns `io::Error` (kind `InvalidData`) rather than looping.
    ///
    /// On success, the returned `Store` is wired so its `Drop` impl will
    /// flush the mmap and rewrite the sidecar on clean shutdown.  A
    /// `kill -9` (or any other path that skips `Drop`) leaves the sidecar
    /// stale → the next open detects corruption → callback re-fires.  This
    /// is by design — Tier 1 trusts the OS page cache for in-flight writes
    /// and recovers via rebuild.
    ///
    /// **Do not use Tier 1** for data that cannot be re-derived from
    /// authoritative sources; phases 02 (snapshots) and 03 (WAL) cover
    /// stronger guarantees.
    #[cfg(feature = "mmap")]
    pub fn open_durable(path: &std::path::Path, mode: DurabilityMode) -> std::io::Result<Store> {
        Self::open_durable_inner(path, mode, 0)
    }

    #[cfg(feature = "mmap")]
    fn open_durable_inner(
        path: &std::path::Path,
        mode: DurabilityMode,
        depth: u32,
    ) -> std::io::Result<Store> {
        dur_trace(&format!(
            "[open_durable_inner] entry depth={depth} path={path:?}"
        ));
        let verdict = Self::validate_integrity(path)?;
        dur_trace(&format!(
            "[open_durable_inner] validate_integrity → {verdict:?}"
        ));
        match verdict {
            StoreIntegrity::Clean => {
                let path_str = path.to_str().ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "store path is not valid UTF-8",
                    )
                })?;
                dur_trace(&format!(
                    "[open_durable_inner] calling Store::open({path_str:?})"
                ));
                let mut store = Store::open(path_str);
                dur_trace("[open_durable_inner] Store::open returned");
                store.durable_meta_path = Some(path.to_path_buf());
                store.durable_tier = TIER_INTEGRITY_ONLY;
                Ok(store)
            }
            StoreIntegrity::Corrupt(_reason) => {
                if depth >= 1 {
                    dur_trace("[open_durable_inner] recursion cap hit; returning InvalidData");
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "store_durable: rebuild callback ran but store still fails integrity",
                    ));
                }
                let DurabilityMode::IntegrityOnly { ref on_corruption } = mode;
                dur_trace("[open_durable_inner] firing on_corruption callback");
                on_corruption(path)?;
                dur_trace("[open_durable_inner] on_corruption returned ok");
                let has_main = path.exists();
                dur_trace(&format!(
                    "[open_durable_inner] post-callback: main_exists={has_main}"
                ));
                if has_main {
                    // After the callback returns Ok, treat the main file as
                    // the new authoritative state and rewrite the sidecar to
                    // match.  Without this, a corruption that affected ONLY
                    // the sidecar (signature flip, payload_crc tear) would
                    // never reach a Clean state: the callback can't repair a
                    // sidecar from the main file's POV, and the stale
                    // sidecar would survive into the depth-1 recursion and
                    // trigger the recursion cap.  Rewriting unconditionally
                    // covers both "main file rebuilt" and "main file fine,
                    // sidecar damaged" cases with one code path.
                    let meta = dmeta_path(path);
                    if meta.exists() {
                        dur_trace(&format!(
                            "[open_durable_inner] removing stale sidecar {meta:?}"
                        ));
                        std::fs::remove_file(&meta)?;
                    }
                    dur_trace(&format!(
                        "[open_durable_inner] write_initial_sidecar({path:?})"
                    ));
                    Self::write_initial_sidecar(path)?;
                    dur_trace("[open_durable_inner] write_initial_sidecar returned");
                }
                dur_trace(&format!(
                    "[open_durable_inner] recursing to depth={}",
                    depth + 1
                ));
                Self::open_durable_inner(path, mode, depth + 1)
            }
        }
    }

    /// Write a fresh sidecar for a main file that exists but has no
    /// sidecar yet (e.g. the rebuild callback just initialised the file).
    /// **Phase 01b.**  Verdict-bool form of [`Store::validate_integrity`]
    /// for the loft-callable binding.  Returns `true` iff the sidecar
    /// validates cleanly against the main file at `path` (signature,
    /// header CRC, payload length, payload CRC, tier_id all OK).
    ///
    /// Any I/O error or `Corrupt(_)` verdict collapses to `false` —
    /// the loft caller can't act on the distinct `CorruptReason`
    /// variants distinctly (every non-Clean case routes to the same
    /// "rebuild from source" response on their side), so the binding
    /// surfaces a flat bool.
    #[cfg(feature = "mmap")]
    #[must_use]
    pub fn durable_check(path: &std::path::Path) -> bool {
        matches!(Self::validate_integrity(path), Ok(StoreIntegrity::Clean))
    }

    /// Shim when the `mmap` feature is off — the shape `bind_path` already
    /// uses, for the same reason: the stdlib surface is one declaration for
    /// every target, so a target without the sidecar machinery has to answer
    /// rather than fail to link (loft#1063).
    ///
    /// `false` is the honest answer AND the safe one: it means *not verified
    /// clean*, which routes the caller into the same rebuild-from-source
    /// branch a corrupt sidecar does.
    #[cfg(not(feature = "mmap"))]
    #[must_use]
    pub fn durable_check(_path: &std::path::Path) -> bool {
        false
    }

    /// **Phase 01b.**  Write a fresh `.dmeta` sidecar capturing the
    /// current main-file's byte length + CRC32 + a clean-close
    /// timestamp.  Returns `true` on success, `false` on any I/O
    /// error (out-of-space, permission denied, parent dir missing,
    /// main file absent).
    ///
    /// This is the loft equivalent of the Rust API's Drop-driven
    /// sidecar write.  The loft caller invokes it explicitly after
    /// finishing a write session; if the program crashes between
    /// the last write and the seal, the sidecar stays stale and the
    /// next `durable_check` returns `false` → rebuild fires.
    ///
    /// Implementation note: this is `write_initial_sidecar` lifted
    /// into a pub function with the result mapped to bool.
    #[cfg(feature = "mmap")]
    #[must_use]
    pub fn durable_seal(path: &std::path::Path) -> bool {
        Self::write_initial_sidecar(path).is_ok()
    }

    /// Shim when the `mmap` feature is off — see [`Self::durable_check`]
    /// (loft#1063).  `false` says the seal did not happen, which is what the
    /// declared contract already covers ("false on any I/O error") and what
    /// the caller must know before trusting a later `durable_check`.
    #[cfg(not(feature = "mmap"))]
    #[must_use]
    pub fn durable_seal(_path: &std::path::Path) -> bool {
        false
    }

    /// Captures the current main-file length and CRC at the moment of call.
    #[cfg(feature = "mmap")]
    fn write_initial_sidecar(path: &std::path::Path) -> std::io::Result<()> {
        dur_trace(&format!("[init_sidecar] entry path={path:?}"));
        let main_meta = std::fs::metadata(path)?;
        let payload_len = main_meta.len();
        dur_trace(&format!("[init_sidecar] main file len={payload_len}"));
        let payload_crc = compute_payload_crc(path, payload_len)?;
        dur_trace(&format!("[init_sidecar] payload CRC = {payload_crc:#x}"));
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let bytes = encode_sidecar(TIER_INTEGRITY_ONLY, now_ns, payload_len, payload_crc);
        dur_trace(&format!(
            "[init_sidecar] writing {} bytes to {:?}",
            bytes.len(),
            dmeta_path(path)
        ));
        write_sidecar_atomic(&dmeta_path(path), &bytes)?;
        dur_trace("[init_sidecar] done");
        Ok(())
    }

    /// On clean drop of a durable store, capture the current main-file
    /// CRC and rewrite the sidecar atomically.  Returns `Err` if any
    /// step fails; the Drop impl logs (rather than panics) on failure
    /// since panicking during drop would abort the process.
    #[cfg(feature = "mmap")]
    fn flush_durable_sidecar(&mut self) -> std::io::Result<()> {
        // Step 1: flush the mmap to disk.
        if let Some(file) = &self.file {
            file.flush_sync()?;
        }
        // Step 2: compute payload CRC over the now-flushed main file.
        let path = match &self.durable_meta_path {
            Some(p) => p.clone(),
            None => return Ok(()),
        };
        let main_meta = std::fs::metadata(&path)?;
        let payload_len = main_meta.len();
        let payload_crc = compute_payload_crc(&path, payload_len)?;
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        let bytes = encode_sidecar(self.durable_tier, now_ns, payload_len, payload_crc);
        // Step 3: write sidecar atomically (tmp → rename).
        write_sidecar_atomic(&dmeta_path(&path), &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_STORE_WORDS, Store, slack_target};

    /// loft#760 — the call bracket's `free_protected` marker must NOT block a delete.
    ///
    /// It means "do not FREE my argument", and the two `0x8000` source-frees that could
    /// (`do_copy_record`, `replace_keyed`) refuse on it themselves. Blocking `delete` as
    /// well was wider than the marker's job and caught the wrong thing: a container the
    /// callee is legitimately mutating releases its own old block when it regrows, so
    /// appending to a passed-by-value struct's field aborted the interpreter with
    /// "Delete on locked store" while `--native`, which has no such check, was correct
    /// on the same source.
    ///
    /// Written against `Store` rather than as a `.loft` program on purpose: the shape
    /// that reaches this needs a whole consumer package to arise (the reduction attempts
    /// on the issue and here all came out clean), while the invariant itself is one bit.
    #[test]
    fn free_protection_permits_a_delete() {
        let mut store = Store::new(4);
        store.free = false;
        let rec = store.claim(2);
        store.set_free_protected("call_bracket(test)");
        assert!(store.is_free_protected(), "the marker is set");
        store.delete(rec); // must not panic — this is the regression
    }

    /// loft#810 — a record whose size word is not positive is refused, not wrapped.
    ///
    /// Every whole-payload walk derives its length as `size * 8 - 4`, so a size of `0`
    /// wraps to ~18 exabytes and the process dies inside `memcpy` — a SIGSEGV that
    /// names the copy rather than the corruption that reached it. The guard cannot
    /// prevent a record from being corrupted; it makes the first read that cannot be
    /// satisfied say so.
    ///
    /// Written against `Store` for the same reason as the delete tests above: the loft
    /// shape that produces a size-`0` record needs a whole consumer package to arise
    /// (the report's own reduction, and the reconstructions here, all came out clean),
    /// while the invariant being asserted is one bit.
    /// The corruption is written through `addr_mut`, NOT `set_i32_raw`, and that is
    /// the point rather than a detail. `set_i32_raw` routes through
    /// [`valid`](Store::valid), which debug-asserts `fld >= 4` — the size word is a
    /// header, not a field — so with debug assertions ON this test used to die at the
    /// SETUP line with "Fld 0 is outside of record 1 size 24", and `should_panic`
    /// accepted it: a test that never reached its subject read as a passing one
    /// everywhere except the debug-assertions gate. A real corruption does not arrive
    /// through the field API either.
    #[test]
    #[should_panic(expected = "claims size 0")]
    fn a_record_claiming_size_zero_is_refused_not_wrapped() {
        let mut store = Store::new(8);
        store.free = false;
        let rec = store.claim(3);
        *store.addr_mut::<i32>(rec, 0) = 0; // the corruption the report observed
        store.zero_fill(rec);
    }

    /// The positive control for the guard above: a well-formed record still copies its
    /// whole payload. Without this, the `should_panic` test would pass just as well if
    /// the guard refused EVERY record.
    #[test]
    fn a_well_formed_record_still_zero_fills_its_payload() {
        let mut store = Store::new(8);
        store.free = false;
        let rec = store.claim(3);
        store.set_i32_raw(rec, 4, 0x7f7f_7f7f);
        store.zero_fill(rec);
        assert_eq!(
            *store.addr::<i32>(rec, 4),
            0,
            "the payload after the size word is cleared"
        );
        assert_eq!(
            *store.addr::<i32>(rec, 0),
            3,
            "the size word itself is kept"
        );
    }

    /// The other direction, so the test above cannot pass by the guard being gone
    /// entirely: `read_only` is IMMUTABILITY (CONST_STORE, workers, `d#lock`) and still
    /// refuses a delete.
    #[test]
    #[should_panic(expected = "Delete on locked store")]
    fn read_only_still_refuses_a_delete() {
        let mut store = Store::new(4);
        store.free = false;
        let rec = store.claim(2);
        store.read_only = true;
        store.delete(rec);
    }

    /// loft#950 — a corrupt record number is refused on EVERY target, not only where
    /// `isize` is 32 bits.
    ///
    /// The only bound that used to survive a release build was `checked_offset`'s
    /// `isize::try_from`, which can fail solely on wasm32.  So one corrupt `DbRef`
    /// trapped in a browser page and, on every 64-bit backend, addressed whatever lay
    /// at the offset it happened to compute — the browser looked like the buggy target
    /// when it was merely the only one that could speak.  `u32::MAX` here is the shape
    /// that report carried: not an index off by one, a reference that is garbage.
    ///
    /// These are written against `Store` for the same reason as the tests above — the
    /// loft program that corrupts a reference needs a whole consumer package to arise,
    /// while the invariant is one compare.
    #[test]
    #[should_panic(expected = "Store access out of bounds")]
    fn a_corrupt_record_number_is_refused_on_the_read_path() {
        let store = Store::new(4);
        let _ = *store.addr::<i64>(u32::MAX, 0);
    }

    /// The write half, which is the one that matters for soundness: unchecked,
    /// `addr_mut` hands out a `&mut` to arbitrary process memory.
    #[test]
    #[should_panic(expected = "Store access out of bounds")]
    fn a_corrupt_record_number_is_refused_on_the_write_path() {
        let mut store = Store::new(4);
        *store.addr_mut::<i64>(u32::MAX, 0) = 1;
    }

    /// The bound is exact, not merely a smell test for huge numbers: one word past
    /// the end is refused too.
    #[test]
    #[should_panic(expected = "Store access out of bounds")]
    fn one_word_past_the_end_is_refused() {
        let store = Store::new(4); // 4 words = 32 bytes
        let _ = *store.addr::<i64>(4, 0); // bytes 32..40
    }

    /// The positive control for all three: without it they would pass just as well if
    /// the guard refused every access.  The LAST addressable word must still read, so
    /// the bound cannot be off by one in the direction that costs correct programs.
    #[test]
    fn the_last_word_of_a_store_is_still_readable() {
        let mut store = Store::new(4); // 4 words = 32 bytes
        *store.addr_mut::<i64>(3, 0) = 0x0123_4567_89ab_cdef; // bytes 24..32
        assert_eq!(
            *store.addr::<i64>(3, 0),
            0x0123_4567_89ab_cdef,
            "the last word is inside the store and stays writable"
        );
    }

    /// Growing the store through many claims must not wrap or silently fail.
    #[test]
    fn store_grows_without_overflow() {
        let mut store = Store::new(4);
        store.free = false; // mark as in-use so validate() does not reject it
        for _ in 0..200 {
            store.claim(1);
        }
        // Store must have grown to hold 200 single-word claims (≥200 * 8 bytes).
        assert!(store.byte_capacity() >= 200 * 8);
    }

    /// @PLN97 arc G Phase 2 — `from_bytes` must fail-closed on a crafted buffer:
    /// `validate_structure` rejects it BEFORE `fl_rebuild`, so a malicious image
    /// can neither hang (the 0-size-block release infinite loop) nor drive a read
    /// past the arena (a forged oversized record).  A structurally-valid image is
    /// still accepted.
    #[test]
    fn from_bytes_rejects_crafted_buffers_fail_closed() {
        use super::SIGNATURE;
        // A 4-word (32-byte) image: word 0 = signature, record 1's `i32` size
        // word = `rec1`; words 2..3 are payload inside record 1.
        fn img(sig: u32, rec1: i32) -> Vec<u8> {
            let mut b = vec![0u8; 32];
            b[0..4].copy_from_slice(&sig.to_ne_bytes());
            b[8..12].copy_from_slice(&rec1.to_ne_bytes());
            b
        }

        // VALID: one claimed block of size 3 partitions words [1, 4).
        assert!(
            Store::from_bytes(&img(SIGNATURE, 3)).is_some(),
            "a structurally-valid image must be accepted"
        );

        // 0-size block — the @PLAN38 release infinite loop: must reject, not hang.
        assert!(
            Store::from_bytes(&img(SIGNATURE, 0)).is_none(),
            "a zero-size block header must be rejected (DoS guard)"
        );
        // A record/free block claiming more words than the arena holds — the
        // forged-header heap over-read.
        assert!(
            Store::from_bytes(&img(SIGNATURE, 100)).is_none(),
            "an out-of-bounds claimed record must be rejected"
        );
        assert!(
            Store::from_bytes(&img(SIGNATURE, -100)).is_none(),
            "an out-of-bounds free block must be rejected"
        );
        assert!(
            Store::from_bytes(&img(SIGNATURE, i32::MIN)).is_none(),
            "an i32::MIN span (2^31) must be rejected without overflow"
        );
        // A chain that leaves a zero header mid-store (record 1 spans [1,3),
        // record 3's header is 0) — caught as a zero block at record 3.
        assert!(
            Store::from_bytes(&img(SIGNATURE, 2)).is_none(),
            "a chain that leaves a zero header mid-store must be rejected"
        );

        // Pre-structural rejects: wrong signature, too small.
        assert!(
            Store::from_bytes(&img(0xDEAD_BEEF, 3)).is_none(),
            "a wrong signature must be rejected"
        );
        assert!(
            Store::from_bytes(&[0u8; 8]).is_none(),
            "an under-16-byte buffer must be rejected"
        );
        assert!(
            Store::from_bytes(&[]).is_none(),
            "an empty buffer must be rejected"
        );
    }

    /// P6: `delete` only coalesces forward, so freeing two blocks in
    /// FORWARD order leaves them as an uncoalesced adjacent pair.  The lazy
    /// `coalesce_free` sweep must merge them (mergeable-pairs → 0) and the
    /// merged block must be reused by the next allocation instead of
    /// growing the store.
    #[test]
    fn coalesce_free_merges_adjacent_and_reuses_space() {
        let mut store = Store::new(64);
        store.free = false;
        let _a = store.claim(5);
        let b = store.claim(5);
        let c = store.claim(5);
        let d = store.claim(5);
        assert!(b < c && c < d, "A,B,C,D are contiguous and ascending");
        // Free B then C (forward order): delete only merges with the NEXT
        // block, so freeing B (C still claimed) then C (D claimed) leaves
        // B|C adjacent-but-unmerged.
        store.delete(b);
        store.delete(c);
        assert!(
            store.usage().mergeable_free_pairs >= 1,
            "forward-order frees leave an uncoalesced adjacent pair"
        );
        // The lazy sweep merges them.
        store.coalesce_free();
        assert_eq!(
            store.usage().mergeable_free_pairs,
            0,
            "coalesce_free merges every adjacent free pair"
        );
        // The merged B+C block (10 words) is reused for a request that
        // neither B(5) nor C(5) could satisfy alone — no store growth.
        let cap_before = store.byte_capacity();
        let reclaimed = store.claim(9);
        assert_eq!(
            store.byte_capacity(),
            cap_before,
            "reused the merged free block instead of growing the store"
        );
        assert!(
            reclaimed >= b && reclaimed < d,
            "reclaimed from the old B/C region (got {reclaimed}, B={b}, D={d})"
        );
    }

    /// @PLN123 A0 — the high-water mark is the word past the LAST claimed
    /// block, and everything above it is free.  That is the entire safety
    /// argument for shrinking a store's file to the mark, so its value is
    /// pinned here rather than left for arc A's first caller to discover.
    #[test]
    fn high_water_mark_ends_at_the_last_claimed_block() {
        let mut store = Store::new(64);
        store.free = false;
        let a = store.claim(5);
        let b = store.claim(5);
        let c = store.claim(5);
        assert!(a < b && b < c, "A, B, C are contiguous and ascending");
        // Free the two TOP records: the mark falls back to the end of A,
        // which is where B began.
        store.delete(c);
        store.delete(b);
        let u = store.usage();
        assert!(u.walk_complete, "a healthy store's chain tiles it exactly");
        assert_eq!(u.live_end_words, b, "the mark is the word past A");
        assert_eq!(u.claimed_words, 5, "only A is still claimed");
        assert_eq!(
            u.free_words,
            store.len() - u.live_end_words,
            "with no interior gap, the free space IS the tail above the mark"
        );
    }

    /// @PLN123 A0 — when the block chain does not tile the store the walk
    /// stops early, so the mark is a LOWER BOUND and a record can live above
    /// it.  `walk_complete` is what tells that apart from a healthy store, and
    /// it has to: truncating to a lower bound cuts live data instead of free
    /// tail.  The two shapes below differ in nothing else a caller can see.
    #[test]
    fn an_incomplete_walk_leaves_a_live_record_above_the_mark() {
        let mut store = Store::new(64);
        store.free = false;
        let _a = store.claim(5);
        let b = store.claim(5);
        let c = store.claim(5);
        let healthy = store.usage();
        assert!(healthy.walk_complete);
        assert_eq!(healthy.live_end_words, c + 5, "the mark is past C");

        // Corrupt B's header to zero — the "malformed / uninitialised tail"
        // shape the walk breaks on.  C stays claimed above the break.
        *store.addr_mut::<i32>(b, 0) = 0;
        let u = store.usage();
        assert!(!u.walk_complete, "the chain no longer tiles the store");
        assert_eq!(u.live_end_words, b, "the walk got no further than A");
        assert!(
            u.live_end_words < c,
            "record C is above the mark, so shrinking to it would cut C — \
             which is why acting on the mark requires walk_complete"
        );
    }

    /// @PLN123 A1 — `shrink_to` gives the tail back and keeps everything below
    /// the mark: the records still read their values, the chain still tiles the
    /// (smaller) store, and the free tree no longer indexes the words that were
    /// cut.  Shrinking to the mark EXACTLY is the boundary case — no free block
    /// is left at all.
    #[test]
    fn shrink_to_the_mark_keeps_every_record() {
        let mut store = Store::new(64);
        store.free = false;
        let a = store.claim(5);
        let b = store.claim(5);
        let c = store.claim(5);
        *store.addr_mut::<i64>(a, 8) = 0x1111;
        *store.addr_mut::<i64>(b, 8) = 0x2222;
        *store.addr_mut::<i64>(c, 8) = 0x3333;
        store.delete(c);

        let mark = store.usage().live_end_words;
        assert_eq!(mark, c, "the mark falls back to where C began");
        let before = store.len();
        assert!(
            store.shrink_to(mark),
            "the tail above the mark is free by construction"
        );
        assert_eq!(store.len(), mark, "capacity is now the mark");
        assert!(before > mark, "the store really was bigger");

        assert_eq!(
            *store.addr::<i64>(a, 8),
            0x1111,
            "A survives the truncation"
        );
        assert_eq!(
            *store.addr::<i64>(b, 8),
            0x2222,
            "B survives the truncation"
        );
        let after = store.usage();
        assert!(
            after.walk_complete,
            "the chain still tiles the smaller store"
        );
        assert_eq!(after.claimed_words, 10, "A and B, nothing else");
        assert_eq!(
            after.free_words, 0,
            "shrinking to the mark leaves no free space"
        );
        store.validate(0);
        #[cfg(debug_assertions)]
        store.fl_validate();

        // The store is still usable: with no free space left, the next claim
        // grows it again rather than handing out a word that was just cut.
        let grown = store.claim(4);
        assert!(
            grown >= mark,
            "the new record starts at or above the old mark"
        );
        assert_eq!(*store.addr::<i64>(b, 8), 0x2222, "and B is still B");
    }

    /// @PLN123 A1 — shrinking to somewhere ABOVE the mark leaves a free tail,
    /// which has to become one block ending exactly at the new size and be
    /// re-indexed (F2).  A stale `free_root` would still name the cut words; a
    /// missing one would strand the leftover, so the proof is that the next
    /// claim REUSES it instead of growing the store.
    #[test]
    fn shrink_to_above_the_mark_leaves_a_usable_free_tail() {
        let mut store = Store::new(64);
        store.free = false;
        let _a = store.claim(5);
        let b = store.claim(5);
        store.delete(b);

        let mark = store.usage().live_end_words;
        assert!(
            store.shrink_to(mark + 4),
            "4 words of tail is a shrink from 64"
        );
        assert_eq!(store.len(), mark + 4);
        let u = store.usage();
        assert!(
            u.walk_complete,
            "the rewritten tail block ends at the new size"
        );
        assert_eq!(
            u.free_count, 1,
            "the tail is ONE block, however many it was"
        );
        assert_eq!(u.free_words, 4);
        #[cfg(debug_assertions)]
        store.fl_validate();

        let cap = store.len();
        let reused = store.claim(3);
        assert_eq!(store.len(), cap, "the leftover tail is in the free tree");
        assert_eq!(reused, mark, "and the claim came out of it");
    }

    /// @PLN123 A1 — the refusals, each leaving the store byte-identical.  A
    /// shrink below the mark would cut a live record; a "shrink" to more than
    /// the current size is a growth request, which is `resize_store`'s job and
    /// not something this path may quietly perform.
    #[test]
    fn shrink_to_refuses_anything_that_would_cut_or_grow() {
        let mut store = Store::new(64);
        store.free = false;
        let _a = store.claim(5);
        let b = store.claim(5);
        *store.addr_mut::<i64>(b, 8) = 0x2222;
        let before = store.len();
        let mark = store.usage().live_end_words;

        assert!(
            !store.shrink_to(mark - 1),
            "one word below the mark cuts into B"
        );
        assert!(!store.shrink_to(0), "and so does zero");
        assert!(!store.shrink_to(before), "the current size is not a shrink");
        assert!(!store.shrink_to(before + 10), "nor is a bigger one");
        assert_eq!(store.len(), before, "every refusal left the capacity alone");
        assert_eq!(*store.addr::<i64>(b, 8), 0x2222, "and B untouched");

        // A0's gate: a chain the walk cannot follow makes the mark a lower
        // bound, and shrinking to a lower bound is what cuts live data.
        *store.addr_mut::<i32>(b, 0) = 0;
        assert!(
            !store.shrink_to(before - 1),
            "an incomplete walk must refuse, whatever the mark says"
        );
        assert_eq!(store.len(), before);
    }

    /// @PLN123 A2 — `reclaim_tail` on the shape it exists for: a store whose
    /// free space is thousands of unmerged blocks because `coalesce_free` is
    /// lazy.  The tail comes back, the interior is merged, and the store is
    /// immediately usable again.
    ///
    /// It also pins the correction in `reclaim_tail`'s own doc: the sweep does
    /// NOT decide how much comes back.  The same store reclaims the same tail
    /// with the sweep already done, because merging free blocks never moves a
    /// claimed one.
    #[test]
    fn reclaim_tail_returns_the_tail_and_merges_the_interior() {
        // Freed in runs of two, FORWARD order: `delete` merges only with the
        // block after it, and that one is still claimed both times, so each run
        // leaves an adjacent-but-unmerged pair — the shape a lazy sweep leaves
        // behind.  (Freeing every OTHER record would leave nothing mergeable
        // and make the sweep assertion below say nothing.)
        fn fragmented() -> (Store, Vec<u32>) {
            let mut store = Store::new(256);
            store.free = false;
            let recs: Vec<u32> = (0..21).map(|_| store.claim(5)).collect();
            for i in (0..18).step_by(3) {
                store.delete(recs[i]);
                store.delete(recs[i + 1]);
            }
            (store, recs)
        }

        let (mut store, recs) = fragmented();
        let last_live = *recs.last().expect("21 records");
        assert_eq!(
            store.usage().mergeable_free_pairs,
            6,
            "six unmerged pairs to sweep — the precondition this test needs"
        );
        let before = store.len();
        let freed = store.reclaim_tail();
        assert_eq!(
            freed,
            before - store.len(),
            "the words it says it gave back"
        );
        assert!(
            freed > 0,
            "a 256-word store holding 20 five-word records has a tail"
        );
        assert_eq!(
            store.len(),
            slack_target(last_live + 5),
            "capacity is now the end of the last surviving record, plus the \
             eighth every right-sized store keeps — trimming to the bare mark \
             leaves the store one claim away from a 7/3 re-grow (loft#727)"
        );
        let after = store.usage();
        assert!(after.walk_complete);
        assert_eq!(after.mergeable_free_pairs, 0, "the interior was swept");
        #[cfg(debug_assertions)]
        store.fl_validate();
        // Still usable, and the swept interior is what serves the next claim.
        let cap = store.len();
        let reused = store.claim(4);
        assert_eq!(store.len(), cap, "reused interior space instead of growing");
        assert!(reused < last_live, "and it came from below the last record");

        // The sweep is not what decides the tail: pre-swept, same answer.
        let (mut swept, _) = fragmented();
        swept.coalesce_free();
        let before_swept = swept.len();
        assert_eq!(
            swept.reclaim_tail(),
            before_swept - slack_target(last_live + 5),
            "coalescing first changes nothing about how much comes back"
        );
    }

    /// @PLN123 A2 — a dense store has nothing to give, and says so without
    /// touching anything.  The report is what a caller acts on, so a `0` that
    /// actually shrank the store would be worse than no call at all.
    #[test]
    fn reclaim_tail_reports_zero_when_there_is_no_tail() {
        let mut store = Store::new(64);
        store.free = false;
        while store.len() == 64 {
            store.claim(5); // fill until the next claim would grow it
        }
        let mark = store.usage().live_end_words;
        assert!(store.shrink_to(mark), "trim to the mark first");
        let before = store.len();
        assert_eq!(store.reclaim_tail(), 0, "nothing above the mark to give");
        assert_eq!(store.len(), before, "and the store is untouched");
    }

    /// @PLN123 A0 — non-vacuity for the mark-vs-capacity assertion.  It is a
    /// `debug_assert`, and `[profile.dev.package.loft]` strips those from
    /// `cargo test`, so this test compiles away with it: it runs only in the
    /// build that installs the instrument, and proves that build can fail.
    /// A final block claiming more words than remain drives the walk — and
    /// with it the mark — past the end of the arena.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "high-water mark")]
    fn a_mark_past_capacity_is_caught() {
        let mut store = Store::new(64);
        store.free = false;
        let a = store.claim(5);
        *store.addr_mut::<i32>(a + 5, 0) = 100; // claimed, and 100 > 64 - 6
        let _ = store.usage();
    }

    /// `resize_store` must panic when the requested size exceeds `MAX_STORE_WORDS`.
    #[test]
    #[should_panic(expected = "store offset overflow")]
    fn resize_store_exceeds_max_panics() {
        let mut store = Store::new(4);
        store.free = false;
        store.resize_store(MAX_STORE_WORDS + 1);
    }

    /// `addr_mut` on a locked store must panic (not silently discard the write).
    #[test]
    #[should_panic(expected = "Write to read-only store")]
    fn write_to_locked_store_panics() {
        let mut store = Store::new(64);
        let rec = store.claim(8);
        store.lock();
        let _: &mut u8 = store.addr_mut::<u8>(rec, 4);
    }

    /// Borrowed store reads return the same data as the original.
    #[test]
    fn borrow_locked_reads_original_data() {
        let mut store = Store::new(64);
        store.free = false;
        let rec = store.claim(4);
        *store.addr_mut::<i32>(rec, 0) = 42;
        let borrow = unsafe { store.borrow_locked_for_light_worker() };
        assert_eq!(*borrow.addr::<i32>(rec, 0), 42);
        assert!(borrow.read_only);
        assert!(borrow.borrowed);
        // Drop of borrow must NOT free the original's buffer.
        drop(borrow);
        assert_eq!(
            *store.addr::<i32>(rec, 0),
            42,
            "original intact after borrow dropped"
        );
    }

    /// Writing to a borrowed store panics.
    #[test]
    #[should_panic(expected = "Write to read-only store")]
    fn borrow_locked_write_panics() {
        let mut store = Store::new(64);
        store.free = false;
        let rec = store.claim(4);
        let mut borrow = unsafe { store.borrow_locked_for_light_worker() };
        // This must panic because the borrow is locked.
        let _: &mut u8 = borrow.addr_mut::<u8>(rec, 0);
    }

    /// Freed sentinel is a minimal store.
    #[test]
    fn freed_sentinel_is_free() {
        let sentinel = Store::new_freed_sentinel();
        assert!(sentinel.free);
    }

    /// cluster-462 / #462 regression: an in-place `resize` grow that absorbs an adjacent
    /// freed block MUST zero the newly-absorbed region, upholding the same "claimed payload
    /// reads zero" invariant `claim` provides. A freshly-exposed vector element slot that
    /// keeps the freed block's stale bytes (garbage text/vec handles) is followed by
    /// `remove_claims`/`length_vector` into a UAF (the sim.loft:3546 SIGSEGV). Pre-fix this
    /// region kept `0xDEAD_BEEF`; post-fix it reads 0.
    #[test]
    fn resize_in_place_zeroes_absorbed_region() {
        let mut store = Store::new(256);
        store.free = false;
        let a = store.claim(4); // 4-word record
        let b = store.claim(16); // adjacent 16-word record
        // Garbage at HIGH offsets in b, past the free-tree node header `delete` writes into
        // b's first words — so it survives the free and is what `resize` must clear.
        *store.addr_mut::<u32>(b, 80) = 0xDEAD_BEEF;
        *store.addr_mut::<u32>(b, 100) = 0x00CA_FE00;
        store.delete(b); // b becomes a free block adjacent to a
        let a2 = store.resize(a, 12); // grow a in place into b's region
        assert_eq!(
            a2, a,
            "resize should grow a in place (absorb the adjacent free block)"
        );
        // b started at word 4 relative to a; b byte 80/100 -> a byte 4*8+80 / 4*8+100.
        assert_eq!(
            *store.addr::<u32>(a, 32 + 80),
            0,
            "absorbed region must be zeroed (kept 0xDEADBEEF pre-fix)"
        );
        assert_eq!(
            *store.addr::<u32>(a, 32 + 100),
            0,
            "absorbed region must be zeroed (kept 0xCAFE pre-fix)"
        );
    }
}
