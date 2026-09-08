// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I61 — Stack slot allocator

//! Per-function variable table and stack slot assignment.
//!
//! Each function being compiled gets a [`Function`] that tracks every
//! variable (name, type, scope, liveness interval, stack slot) and every
//! iterator/loop in the function body.
//!
//! ## Dependency tracking
//!
//! The `dep` field on [`Type`](crate::data::Type) controls ownership:
//! - **empty** → the variable *owns* its heap value (freed at scope exit).
//! - **non-empty** → the variable *borrows* from a parameter listed by
//!   attribute index (not freed — the caller owns the store).
//!
//! [`Function::depend`] adds a dependency when the parser discovers that a
//! local variable borrows from a parameter (e.g. field access on a reference
//! argument).
//!
//! ## Slot assignment
//!
//! After scope analysis, [`assign_slots`] assigns each variable a byte
//! offset on the stack using a two-zone layout:
//! - **Zone 1** (pre-claimed): small types (≤ 16 bytes) packed per-block.
//! - **Zone 2** (sequential): large types (text, references) allocated in
//!   the order they first appear.
//!
//! See `slots.rs` for the algorithm.

mod intervals;
mod slots_v2;
mod validate;

pub use intervals::compute_intervals;
// @PLAN53 — the aligned V2 allocator is the only allocator; scopes.rs drives
// it directly via `assign_slots_v2` + `apply_v2_result`.
#[allow(unused_imports)]
pub use slots_v2::{AllocatorResult, SlotAssignment, SlotKind, apply_v2_result, assign_slots_v2};
pub use validate::{dump_var_tables, dump_variables};
// Plan-04 Phase 2e: ungate validate_slots so LOFT_SLOT_V2=validate
// shadow mode can invoke it from any build profile (integration
// tests compile against loft without debug_assertions).  The
// call site in state/codegen.rs remains gated on
// `#[cfg(any(debug_assertions, test))]` — validate_slots is
// unconditionally *compiled*, but only *called* automatically
// during unit tests; shadow mode opts in at runtime via env var.
// The profile.dev.package.loft override disables debug_assertions
// in the hot interpreter path; clippy sees no in-crate caller and
// would otherwise flag this re-export.
#[allow(unused_imports)]
pub use validate::{validate_alignment, validate_slots};

use crate::data::{Context, Data, Deps, Type, Value};
use crate::diagnostics::{Level, diagnostic_format};
use crate::keys::DbRef;
use crate::lexer::Lexer;

/// True iff `name` contains at least one uppercase letter and no
/// lowercase letters.  Used by `warn_upper_case_locals` (P246
/// follow-up): `_`, `_foo`, and `123` should NOT trip the warning,
/// but `FOO`, `MAX_SIZE`, `X` should.  Distinct from the parser's
/// `is_upper`, which accepts pure-numeric names like `42` (no
/// lowercase chars at all but also no uppercase).
fn is_upper_case_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let mut has_upper = false;
    for c in name.chars() {
        if c.is_lowercase() {
            return false;
        }
        if c.is_uppercase() {
            has_upper = true;
        }
    }
    has_upper
}
/**
This administrates variables and scopes for a specific function.
- The first scope (0) is for function arguments.
- Variables might exist in multiple scopes but not with different types.
- We allow for variables to move to a higher scope.
*/
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::{Display, Formatter};

// Iterator details on each for loop inside the current function
#[derive(Debug, Clone)]
struct Iterator {
    inside: u16,       // iterator number or MAX when top level loop
    variable: u16,     // variable number
    on: u8,            // structure type and direction
    db_tp: u16,        // database type of this structure
    value: Box<Value>, // code to gain the structure or Value::Null for a range
    /// The original user-written collection variable number being iterated.
    /// For vector loops the iterator works on a unique temp copy; this field
    /// stores the original var so mutation of the original can be detected.
    /// `u16::MAX` when the iterated expression is not a simple variable
    /// (e.g. a struct-field access like `db.map`).
    coll_var: u16,
    counter: u16, // variable number or MAX when it is not used
    /// @PLN102 strict-index lint — for a `for i in 0..len(X)` range, the `VecKey` of `X`
    /// (the vector whose length bounds this loop). `Some` only when the range's upper bound
    /// is a bare `len(<addressable vector>)`; `None` for `0..n`, slices, or collection loops.
    /// Read by the gated `LOFT_LINT_STRICT_INDEX` warning to flag `w[i]` where `w != X`.
    len_bound: Option<crate::parser::operators::VecKey>,
    /// The I64 local holding this loop's packed iterator state (`cur << 32 | finish`), the
    /// one `OpStep` steps and `OpRemove` rewinds. `u16::MAX` when the loop has none (a
    /// range, a vector walk, a custom iterator).
    ///
    /// Recorded by whoever CREATES the cursor, because there are two lowerings of a keyed
    /// iteration and they name it differently: an unbounded walk builds `{loop}#iter_state`
    /// (parser/collections.rs) while a bounded RANGE builds `_iter_N`
    /// (parser/fields.rs::parse_key). `#remove` used to rebuild the first spelling by hand
    /// and fall back to `{loop}#index` when it missed — and for a range that fallback names
    /// a local the lowering ELIDES, so the operand was measured against a slot that does not
    /// exist and the compile ended in an arithmetic overflow (loft#1272).
    state_var: u16,
}

/// @PLAN28 C4 — borrowed view of the codegen-read fields of one `Variable`,
/// produced by [`Function::snapshot_var`] for the snapshot encoder.
#[allow(clippy::struct_excessive_bools)] // mirrors `Variable`'s codegen-read flags
pub(crate) struct VarSnapshot<'a> {
    pub name: &'a str,
    pub type_def: &'a Type,
    pub stack_pos: u16,
    pub uses: u16,
    pub argument: bool,
    pub stack_allocated: bool,
    pub skip_free: bool,
    pub captured: bool,
    pub caller_hidden_buf: bool,
    /// @PLN157 § V-g — the copy from a borrowing call is elided; read by both emitters.
    pub view_elided: bool,
    /// The owner witness of a mixed-ownership local (`@FR-O-Witness`), `u16::MAX` for none.
    pub owner_witness: u16,
}

/// @PLAN28 C4 — owned codegen-read fields of one `Variable`, consumed by
/// [`Function::from_snapshot`] on the decode path.
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct RestoredVar {
    pub name: String,
    pub type_def: Type,
    pub stack_pos: u16,
    pub uses: u16,
    pub argument: bool,
    pub stack_allocated: bool,
    pub skip_free: bool,
    pub captured: bool,
    pub caller_hidden_buf: bool,
    pub view_elided: bool,
    pub owner_witness: u16,
}

// This is created for every variable instance, even if those are of the same name.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct Variable {
    name: String,
    type_def: Type,
    source: (u32, u32),
    scope: u16,
    stack_pos: u16,
    uses: u16,
    uses_at_write: u16,
    write_source: (u32, u32),
    argument: bool,
    defined: bool,
    /// Binding-const (`const` PREFIX): `const x` local, `const p: T` param, and the
    /// field twin `Attribute.const_field`.  The slot is write-once — a rebind (`=`)
    /// is rejected — but the value it holds stays mutable (`+=` / element write are
    /// allowed).  @PLN40 const-model phase 1; see doc/claude/plans/40-const-fields/.
    const_binding: bool,
    /// Value-const (`const` before the TYPE): `p: const T` param, `x: const T` local.
    /// A read-only borrow of the value — every mutation THROUGH this name (`+=`,
    /// element, field, nested) is rejected, but a rebind (`=`) that re-points the
    /// slot is allowed.  The sibling of the `&T` mutable borrow.  @PLN40 phase 1.
    value_const: bool,
    /// @PLN157 § V-g — bound from a call whose return borrows an argument, and only ever
    /// read: the copy `(O-Move)` asks for is unobservable, so the local keeps the VIEW and
    /// releases only the callee's per-execution minted store, by identity at scope exit.
    /// Set by `scopes::scan_set`, read by both backends' copy arms (`is_view_elided`).
    view_elided: bool,
    /// @PLN130 F9 — this binding was spelled with `&` at a STRUCT-typed projection
    /// (`c = &v[0]`, `c = &o.inner`).  Such a projection is already a VIEW under B-View,
    /// so both spellings lower to byte-identical IR and the `&` used to be dropped as
    /// redundant.  It stopped being redundant when F2 made a view MATERIALISE on a
    /// reshape: from then on `&` also means *"and do not silently copy it"*, which is
    /// information the IR no longer carries.  A marker rather than `Type::RefVar` on
    /// purpose — RefVar would re-route every read and write through the double
    /// indirection parameters use, slowing every access to carry a compile-time fact.
    /// See loft#779 / `formal/binding.md` D-bind-8.
    amp_link: bool,
    /// Is this the temp a `for` loop holds its iteration SOURCE in?
    ///
    /// A vector-valued `for … in <expr>` binds the source to a temp and iterates THAT
    /// (`Parser::parse_for` and the comprehension's twin), so the iteration depends on the
    /// temp's identity: give it a store of its own and the loop walks a copy while the body's
    /// `#remove` empties the original, which does not terminate.
    ///
    /// So the view-materialise walk skips it.  A NAME test would not do — the parser renames
    /// author bindings too, and a `match` payload (`_mv_inner_N`) is a view the author wrote
    /// and must still materialise — which is why this is a marker set where the temp is made.
    iteration_source: bool,
    /// Whether this variable's stack storage has been initialised by codegen.
    /// Set to `true` when the first-allocation init opcodes are emitted (A6.3).
    /// Arguments are pre-allocated by the caller, so they start as `true`.
    pub stack_allocated: bool,
    /// When true, `get_free_vars` must not emit `OpFreeRef` for this variable.
    /// Set by `clean_work_refs` for work-ref temporaries that have been re-purposed
    /// and must not be freed at scope exit (A14 replacement for type-mutation hack).
    pub skip_free: bool,
    /// Variable is captured by a closure.  Suppresses the "never read"
    /// warning in `test_used` without affecting the dead-assignment uses counter.
    pub captured: bool,
    /// Sequence number of the first `Value::Set` node for this variable; `u32::MAX` = never defined.
    pub first_def: u32,
    /// Sequence number of the last `Value::Var` (or implicit `OpFreeText`/`OpFreeRef`) for this variable.
    pub last_use: u32,
    /// Slot assigned by `assign_slots` before codegen may override it via `set_stack_pos`.
    /// `u16::MAX` means `assign_slots` has not run yet.  Shown as `pre:` in `validate_slots`
    /// diagnostics when it differs from the final `stack_pos`.
    pub pre_assigned_pos: u16,
    /// If this variable is a shadow local promoted from a text argument,
    /// `promoted_from` holds the var_nr of the original argument. `u16::MAX` = not promoted.
    pub promoted_from: u16,
    /// C61.local: set by `loop_var()` when this variable has served as
    /// the body variable of a `for <id> in …` loop.  Survives slot reuse
    /// across sequential loops in the same function, letting
    /// `parse_for_iter_setup` distinguish a safe sequential reuse from
    /// an outer-local shadow.
    pub was_loop_var: bool,
    /// The loop this variable was FIRST created inside, or `u16::MAX` for one created at
    /// function scope.
    ///
    /// loft#1145 — loft#915 gave a `for` loop's VARIABLE its own binding per loop so two
    /// loops may spell one name at their own element types.  A local declared in the BODY
    /// had no such split, so the second loop's `e = y` re-typed the first loop's slot and
    /// was refused.  Telling the two apart needs the loop a variable BELONGS to, which no
    /// other field carries: `was_loop_var` answers a different question (did it ever serve
    /// as a loop's own variable), and `scope` is the block nesting, which two sibling loops
    /// share.
    pub created_in_loop: u16,
    /// @PLAN51 Cluster IV: set by `add_defaults` when it synthesises a
    /// caller-side work-ref for a callee's hidden return-buffer arg.
    /// These work-refs are allocated by the parser as call-site placeholders
    /// (the caller pre-allocates the buffer, the callee writes into it),
    /// so they need a leading `Set(r, Null)` IR so the slot allocator sees
    /// a `first_def` and assigns a stack slot.  Without the null-init, vars
    /// whose typedef has a non-empty dep list (e.g. `Reference(td, [arg_idx])`
    /// for if-tail / recursion shapes) skip the dep-empty guard in
    /// `parse_code` and end up SKIP'd by `assign_slots` ("no first_def") —
    /// codegen then panics with "Incorrect var __ref_N[65535]" when it tries
    /// to emit the call's arg.
    pub caller_hidden_buf: bool,
}

#[derive(Debug, Clone)]
pub struct Function {
    /// loft#1466 — the locals whose borrow list pass 2 has already rebuilt.
    ///
    /// A CALL RESULT's deps are the CALLEE's answer and pass 1 has not read the callee's body,
    /// so the list it publishes for one is a guess.  Pass 2 re-derives every assignment, so its
    /// union is the whole answer: the binding's list is cleared at its FIRST pass-2 assignment
    /// from a call and rebuilt from there.  This records which locals that has happened to, so
    /// the clear happens once and the later assignments union onto it — clearing at every one
    /// would keep only the last, and a dep list is flow-INsensitive.
    ///
    /// Lives on the `Function` rather than on the parser because a var number is unique per
    /// FUNCTION, and the parser swaps this whole table for a lambda's.
    pass2_rebuilt: std::collections::HashSet<u16>,
    pub name: String,
    pub file: String,
    /// Per-prefix counters for `unique()` temp names (`_<prefix>_<n>`).
    /// Per-PREFIX (not one shared counter) so a temp family created on only
    /// one parser pass (e.g. a second-pass-only lowering temp) cannot shift
    /// the numbering of every OTHER family between passes — a shifted name
    /// re-resolves to a pass-1 var of a different TYPE in `add_variable`
    /// (#320's frame-drift: an integer-typed `__ncc_N` holding a DbRef).
    unique: HashMap<String, u16>,
    /// Per-name counters for [`Function::loop_binding`] — how many `for` loops in this
    /// function have already bound each source name.
    ///
    /// Counting occurrences in PARSE ORDER is what makes the extra bindings pass-stable.
    /// The sequence reads no type and consults no table, so pass 2 regenerates it exactly
    /// and `add_variable` re-finds each loop's slot under the same key; a verdict that
    /// MINTS has to come out identical on both passes or every slot number after it
    /// shifts.  Cleared by `append`, beside `unique`, for that reason.
    loop_binds: HashMap<String, u16>,
    /// loft#1160 — the field access each variant-payload BINDING in this function was
    /// projected from (`_mv_<field> = OpGetField(subject, off, tp)`), with the type that
    /// names the variant, by the binding's variable number.
    ///
    /// A write spelled through the binding has to mean the same as the write spelled through
    /// the FIELD, and resolving it back needs the projection.  It lives HERE, on the
    /// function's own variable table, because a variable NUMBER is only unique within a
    /// function: held on the parser instead, a binding registered in one function made an
    /// unrelated variable with the same number in the NEXT one look like a binding, and the
    /// `i490_enum_field_return_inline` leak case then read a store the substitution had
    /// redirected.  Regenerated by the binding sites on every pass, like `unique`.
    pub mv_field_origin: HashMap<u16, (Value, Type)>,
    /// loft#1145 — a per-PASS ordinal for the loop now being parsed, and the body locals
    /// each loop binds.
    ///
    /// Loop NUMBERS (`current_loop`) are not stable across the two passes: `Function::copy`
    /// carries pass 1's `loops` and pass 2's `start_loop` pushes on top of them.  The
    /// ordinal is, because it is cleared per pass and `start_loop` is called exactly once
    /// per loop in parse order — the same property `loop_binds` relies on.  So the ordinal
    /// is what a body local's cross-pass identity is keyed by, and the plain `names` map,
    /// which `Function::append` clones from pass 1, is re-pointed at each loop's own
    /// bindings when that loop is entered.
    loop_ord: u16,
    loop_ord_of: HashMap<u16, u16>,
    body_binds: HashMap<(u16, String), u16>,
    pub(crate) current_loop: u16,
    loops: Vec<Iterator>,
    variables: Vec<Variable>,
    /// Variables whose type came from an EXPLICIT `: Type` annotation (not inferred
    /// from an assignment).  An annotated narrow integer (`x: u8`) stays constrained;
    /// an INFERRED local widens to the join of its assignments — the `(I-Join)` rule
    /// that closes the #433-residual (see `parse_assign_op`).
    annotated: HashSet<u16>,
    work_text: u16,
    /// Separate counter for the caller-allocated `&text` out-param buffers
    /// `caller_text_buf()` mints — the text twin of `work_vdb` below, and for
    /// the same reason (loft#662).  A call only needs one once the callee's
    /// `&text` ABI exists, which for a self-/forward-recursive callee is not
    /// until pass 1 has promoted it; sharing `work_text` would shift the
    /// `__work_N` names relative to pass 1 and break `text_return`'s name-based
    /// attr matching.
    work_ctext: u16,
    /// loft#665 piece 2 — the PASS-2-ONLY `__work` sequence.  A mint site that can
    /// only fire on pass 2 must not draw from `work_text`: doing so shifts every
    /// later `__work_N` relative to pass 1, and because the variable tables persist
    /// BY NAME, pass 2's buffers then re-find pass 1's variables under the wrong
    /// roles (loft#662).  Pass-availability is a STATIC property of each mint site
    /// (measured: 19 both-pass sites, 15 pass-2-only, none mixed), so the split is
    /// decidable at the call site.
    work_text_p2: u16,
    work_ref: u16,
    /// loft#848 — the PASS-2-ONLY `__ref` sequence, the reference twin of
    /// `work_text_p2`, and for the same reason: the variable tables persist across
    /// passes BY NAME while the counter restarts, so a mint that fires on only one
    /// pass shifts every later name in that function by one.
    ///
    /// What makes the reference sequence's version of that a WRONG VALUE rather than
    /// a mismatched buffer is where the shifted name lands.  `ref_return` promotes
    /// the returned work-ref to an ARGUMENT on pass 1, so the `__ref_N` it leaves
    /// behind denotes the function's RETURN BUFFER — and a shifted pass-2 site is
    /// handed that buffer as scratch.  It then writes a value that must stay live
    /// into the buffer the return re-mints with `OpDatabase`, and the copy at the
    /// return reads the destroyed store: `null`, no diagnostic.  In loft#848 a
    /// value-block's result and an enum variant's payload both died that way.
    ///
    /// Pass-availability is a STATIC property of each mint site (every site routed
    /// here sits under a `!self.first_pass` guard), so the split is decidable at the
    /// call site — `work_refs_p2`.
    ///
    /// Routed here are the pass-2-only sites whose value is bound to something that
    /// OUTLIVES the statement — a block's value, a named local's construction.  A
    /// pass-2-only site whose value is consumed inside its own expression can share
    /// the buffer harmlessly, and the value-position construction that cannot
    /// (`fn h() -> vector<E> { g(Filled { … }) }`, where the subject must outlive the
    /// call) is already defended one layer later, by @PLN90 A1b's promotion-skip in
    /// `classify_ret_promotion`.  Moving that one here too would work and would make
    /// A1b's materialise redundant for the shape — but it also takes away what
    /// `oracle_flags_the_a1b_wrong_plan` proves, so it is a deliberate choice and not
    /// an oversight (loft#848).
    ///
    /// The name stays `__ref_p2_N` and does not become its own stem: every
    /// downstream work-ref rule keys on the `__ref_` prefix (the scope-exit free
    /// emission in `scopes.rs`, `use_analysis`, `emit.rs`), and these ARE ordinary
    /// work-refs — only their numbering is separate.  `sync_work_counters`
    /// self-disambiguates, since `p2_1` does not parse as a number.
    work_ref_p2: u16,
    // Separate counter for vector-db work-refs created by `vector_db()`.
    // `vector_db` only runs on the second pass (first_pass guard), so it cannot
    // use the shared `work_ref` counter: that would shift the counter relative to
    // the first pass and break `ref_return`'s name-based attr matching.
    work_vdb: u16,
    /// loft#703 — separate counter for the work-ref a KEYED collection literal in value
    /// position builds into.  Its own namespace for two reasons: `__ref_N` is reserved
    /// for return buffers, whose name-based `ref_return` match a shared counter would
    /// shift; and `__vdb_N` means "a WRAPPER record holding a vector field", which
    /// `store_confinement` reads as "some other local backs this store, confine both to
    /// that local's block".  A keyed collection has no wrapper — the accumulator IS the
    /// value — so the only local depending on it is an ELEMENT inside it, and confining
    /// the store to the element's block left the block's own result unfreed.
    work_kvb: u16,
    /// @PLN124 — separate counter for the accumulator a format string BUILDS when
    /// its target type implements the interpolation contract (`"…{x}…"` into a
    /// `SqlText` rather than into text).
    ///
    /// Its own namespace for the same reason every counter above has one: the
    /// variable tables persist across passes BY NAME, so a mint that fires on
    /// only some strings must not shift anyone else's numbering (loft#662).  It
    /// is not a `__work_N` because it is not a text buffer, and not a `__ref_N`
    /// because that stem is reserved for return buffers whose `ref_return` match
    /// is name-based.  Like `__kvb_N`, the accumulator IS the value — no wrapper
    /// record backs its store — so it is function-scoped and the exit sweep frees
    /// it.
    work_fmt: u16,
    // Work variables for texts
    work_texts: BTreeSet<u16>,
    // Work variables for stores
    work_refs: BTreeSet<u16>,
    /// Vars the return-delivery materializer CONSUMED inside a branch arm
    /// (free-after-append on every path, incl. cross-arm frees).  Gates
    /// `insert_free`'s reads-filter: a pre-return free is dropped ONLY when
    /// the returned tail still reads the var AND an in-arm free covers it —
    /// dropping on reads alone leaks any read-but-unconsumed buffer (the
    /// `?? call()` hidden __ref, gate-ON elem_accumulate).
    arm_consumed: BTreeSet<u16>,
    // Subset of work_refs: inline-ref temporaries created by parse_part to capture
    // the result of a ref-returning method call that is immediately chained (e.g.
    // `p.shifted(1.0, 0.0).x`).  These need their preamble null-init inserted
    // AFTER the first user statement so they appear after user-scope vars in var_order
    // and are therefore freed BEFORE them — satisfying the database LIFO invariant.
    inline_ref_vars: BTreeSet<u16>,
    /// Locals assigned a BORROW on one path and an owned value on another (loft#1333).
    borrow_arm_vars: BTreeSet<u16>,
    // The names store only the last known instance of this variable in the function.
    names: HashMap<String, u16>,
    // Scope numbers that correspond to loop bodies (Value::Loop), i.e. scopes whose
    // variables are freed by OpFreeStack when the loop exits.  If-block scopes
    // (Value::Block) are NOT in this set; their variables live until function return.
    // Used by assign_slots to compute the physical TOS accurately.
    loop_scopes: HashSet<u16>,
    // Maps each loop-body scope number → (seq_start, seq_end) where seq_start / seq_end
    // are the `compute_intervals` sequence counters immediately before / after the loop
    // body is traversed.  assign_slots uses this to decide whether a dead loop-scope
    // variable j is still physically present at i.first_def:
    //   - If i.first_def < seq_end(j.scope): the loop's FreeStack fires AFTER i.first_def
    //     → j's bytes are still on the physical stack at i.first_def (include in tos_estimate).
    //   - If i.first_def >= seq_end(j.scope): the loop exited before i.first_def
    //     → j's bytes were freed by FreeStack (exclude from tos_estimate).
    loop_seq_ranges: HashMap<u16, (u32, u32)>,
    // Maps each scope number to the source construct that introduced it: "block", "for", "if", etc.
    scope_origins: HashMap<u16, &'static str>,
    pub done: bool,
    pub logging: bool,
    // maps fn_ref_var_nr → closure_var_nr for native codegen.
    closure_var_map: HashMap<u16, u16>,
    /// @PLN87 P2.1 — reassignment-locality.  Maps a user-visible heap PARAMETER
    /// (whole-binding-reassigned in the body) → its `__orig` witness var, a
    /// skip-free work-ref holding the param's caller-supplied DbRef captured at
    /// function entry.  A rebind frees the param's CURRENT store only when it
    /// differs from this witness (`OpFreeRefIfDistinct`), so the caller's
    /// original store is never freed by the callee and a fresh rebind store is.
    /// Parse-time only: on a snapshot load `scopes::check` is skipped (the frees
    /// are already in `code`), so this map is not part of the snapshot.
    rebind_orig: HashMap<u16, u16>,
    /// A heap-record LOCAL whose assignments MIX ownership — one hands it a store of its
    /// own, another a view — maps to its OWNER WITNESS `__own_<name>`: a hidden reference
    /// that names the store the local minted for as long as the local still holds it, and
    /// the null sentinel otherwise.  The local itself is never freed (`skip_free`); every
    /// release goes through the witness, which `scopes` maintains per RUN in the IR so both
    /// backends translate one fact (`@FR-O-Witness`).  Set by `scopes::check`; read by the
    /// two emitters to pick the arm that COPIES into a store the local owns, and by the
    /// native generator to leave such a local to the IR rather than its own tracker.
    owner_witness: HashMap<u16, u16>,
}

/// @PLN104 — swap membership of two indices in a set (for `Function::swap_variables`).
/// An index in the set moves to the other; if both or neither are present, no change.
fn swap_in_bset(s: &mut BTreeSet<u16>, a: u16, b: u16) {
    let (ha, hb) = (s.contains(&a), s.contains(&b));
    if ha != hb {
        if ha {
            s.remove(&a);
            s.insert(b);
        } else {
            s.remove(&b);
            s.insert(a);
        }
    }
}

fn swap_in_hset(s: &mut HashSet<u16>, a: u16, b: u16) {
    let (ha, hb) = (s.contains(&a), s.contains(&b));
    if ha != hb {
        if ha {
            s.remove(&a);
            s.insert(b);
        } else {
            s.remove(&b);
            s.insert(a);
        }
    }
}

/// Swap `a`/`b` in BOTH the keys and the values of a var→var map.
fn swap_map_indices(m: &mut HashMap<u16, u16>, a: u16, b: u16) {
    let swap1 = |x: u16| {
        if x == a {
            b
        } else if x == b {
            a
        } else {
            x
        }
    };
    *m = m.iter().map(|(&k, &v)| (swap1(k), swap1(v))).collect();
}

impl Display for Function {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for v in &self.variables {
            f.write_fmt(format_args!("{v:?}\n"))?;
        }
        Ok(())
    }
}

impl Function {
    pub fn new(name: &str, file: &str) -> Self {
        Function {
            pass2_rebuilt: std::collections::HashSet::new(),
            name: name.to_string(),
            file: file.to_string(),
            unique: HashMap::new(),
            loop_binds: HashMap::new(),
            mv_field_origin: HashMap::new(),
            loop_ord: 0,
            loop_ord_of: HashMap::new(),
            body_binds: HashMap::new(),
            current_loop: u16::MAX,
            loops: Vec::new(),
            work_text: 0,
            work_ctext: 0,
            work_text_p2: 0,
            work_ref: 0,
            work_ref_p2: 0,
            work_vdb: 0,
            work_kvb: 0,
            work_fmt: 0,
            variables: Vec::new(),
            annotated: HashSet::new(),
            work_texts: BTreeSet::new(),
            work_refs: BTreeSet::new(),
            arm_consumed: BTreeSet::new(),
            inline_ref_vars: BTreeSet::new(),
            borrow_arm_vars: BTreeSet::new(),
            names: HashMap::new(),
            loop_scopes: HashSet::new(),
            loop_seq_ranges: HashMap::new(),
            scope_origins: HashMap::new(),
            logging: false,
            done: false,
            closure_var_map: HashMap::new(),
            rebind_orig: HashMap::new(),
            owner_witness: HashMap::new(),
        }
    }

    // ─── @PLAN28 startup-cache snapshot seam (C4) ────────────────────────
    //
    // The variable table is stored in the snapshot as DEBUG SYMBOLS plus the
    // FINAL `stack_pos`.  The codec lives in `src/ir_schema.rs`; these
    // `pub(crate)` accessors give it field access without exposing the private
    // struct internals project-wide.  Only the fields codegen READS are
    // stored (`name`/`type_def`/`stack_pos`/`uses`/`argument`/
    // `stack_allocated`/`skip_free`/`captured`/`caller_hidden_buf`, plus the
    // `names` map and `inline_ref_vars` set).  Fields codegen never reads
    // (`scope`, the slot scratch `pre_assigned_pos`/`first_def`/`last_use`,
    // the parse-time `work_*` counters) are NOT stored — `scopes::check` is
    // skipped on load (it ran before the snapshot, and re-running would
    // double-insert the free-ops already in `code`).

    /// Number of variables (snapshot encode helper).
    #[must_use]
    pub(crate) fn snapshot_len(&self) -> usize {
        self.variables.len()
    }

    /// The ten codegen-read fields of variable `i`, for the snapshot encoder.  A fact the
    /// EMITTERS read belongs here whatever map carries it at parse time: `owner_witness`
    /// was maintained in the IR and restored nowhere, so a warm program-cache run copied
    /// into the record a witnessed local was viewing (`@FR-O-Witness`).
    #[must_use]
    pub(crate) fn snapshot_var(&self, i: usize) -> VarSnapshot<'_> {
        let v = &self.variables[i];
        VarSnapshot {
            name: &v.name,
            type_def: &v.type_def,
            stack_pos: v.stack_pos,
            uses: v.uses,
            argument: v.argument,
            stack_allocated: v.stack_allocated,
            skip_free: v.skip_free,
            captured: v.captured,
            caller_hidden_buf: v.caller_hidden_buf,
            view_elided: v.view_elided,
            owner_witness: self.owner_witness(i as u16).unwrap_or(u16::MAX),
        }
    }

    /// The `names` map entries (name → var_nr), for the snapshot encoder, in a
    /// STABLE total order (by `var_nr`, then name).
    ///
    /// `self.names` is a `HashMap`, so iterating it raw yields a run-to-run
    /// varying order — which made the serialized variable-name list, and thus the
    /// cached `Data` (the store codec) AND the JSON codec, **non-reproducible**:
    /// two names sharing a `var_nr` (a text param and its `__tp_` first-mutation
    /// promotion shadow) flipped order between runs, so a fresh parse and a
    /// store/JSON round-trip of the same program disagreed.  Sorting here is the
    /// chokepoint — both codecs encode from this, so both become deterministic.
    /// The order is non-load-bearing (debug symbols, looked up by name), so the
    /// sort is free of behaviour change.
    #[must_use]
    pub(crate) fn snapshot_names(&self) -> Vec<(&str, u16)> {
        let mut out: Vec<(&str, u16)> = self.names.iter().map(|(k, &v)| (k.as_str(), v)).collect();
        out.sort_unstable_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(b.0)));
        out
    }

    /// The `inline_ref_vars` set, for the snapshot encoder.
    #[must_use]
    pub(crate) fn snapshot_inline_refs(&self) -> Vec<u16> {
        self.inline_ref_vars.iter().copied().collect()
    }

    /// Reconstruct a `Function` from a snapshot (C4 decode).  Every field
    /// codegen does not read is filled with its post-parse default — harmless
    /// because `scopes::check` is not re-run on the load path.
    #[must_use]
    pub(crate) fn from_snapshot(
        name: &str,
        file: &str,
        vars: Vec<RestoredVar>,
        names: Vec<(String, u16)>,
        inline_refs: Vec<u16>,
    ) -> Function {
        let mut f = Function::new(name, file);
        for (i, r) in vars.iter().enumerate() {
            if r.owner_witness != u16::MAX {
                f.owner_witness.insert(i as u16, r.owner_witness);
            }
        }
        f.variables = vars
            .into_iter()
            .map(|r| Variable {
                name: r.name,
                type_def: r.type_def,
                stack_pos: r.stack_pos,
                uses: r.uses,
                argument: r.argument,
                stack_allocated: r.stack_allocated,
                skip_free: r.skip_free,
                captured: r.captured,
                caller_hidden_buf: r.caller_hidden_buf,
                view_elided: r.view_elided,
                // codegen-irrelevant post-parse defaults (not stored):
                source: (0, 0),
                scope: u16::MAX,
                uses_at_write: 0,
                write_source: (0, 0),
                defined: false,
                const_binding: false,
                value_const: false,
                amp_link: false,
                iteration_source: false,
                first_def: u32::MAX,
                last_use: 0,
                pre_assigned_pos: u16::MAX,
                promoted_from: u16::MAX,
                was_loop_var: false,
                created_in_loop: u16::MAX,
            })
            .collect();
        f.names = names.into_iter().collect();
        f.inline_ref_vars = inline_refs.into_iter().collect();
        // A snapshot is taken AFTER `scopes::check` ran (the stored `code`
        // already carries its free-ops); mark the reconstructed function
        // `done` so a load-path `scopes::check` skips it and does not
        // double-insert those frees (@PLN11 arc D / @PLAN28 C4 insight).
        f.done = true;
        f
    }

    pub fn append(&mut self, other: &mut Function) {
        self.current_loop = u16::MAX;
        self.logging = other.logging;
        self.unique.clear();
        other.unique.clear();
        self.loop_binds.clear();
        other.loop_binds.clear();
        self.mv_field_origin.clear();
        other.mv_field_origin.clear();
        self.loop_ord = 0;
        other.loop_ord = 0;
        self.loop_ord_of.clear();
        self.loop_ord_of.clone_from(&other.loop_ord_of);
        // CARRIED, not cleared: this is exactly the map pass 2 reads to hand each loop the
        // binding pass 1 gave it (loft#1145).  The ordinal keys it, and the ordinal restarts
        // per pass, so the entries line up.
        self.body_binds.clear();
        self.body_binds.clone_from(&other.body_binds);
        self.loops.clear();
        self.loops.append(&mut other.loops);
        self.variables.clear();
        self.variables.append(&mut other.variables);
        for v in &mut self.variables {
            v.uses = 0;
        }
        self.work_text = 0;
        self.work_ctext = 0;
        self.work_text_p2 = 0;
        self.work_ref = 0;
        self.work_ref_p2 = 0;
        self.work_vdb = 0;
        self.work_kvb = 0;
        self.work_fmt = 0;
        self.work_texts.clear();
        self.work_refs.clear();
        self.arm_consumed.clear();
        self.arm_consumed.clone_from(&other.arm_consumed);
        self.inline_ref_vars.clear();
        self.inline_ref_vars.clone_from(&other.inline_ref_vars);
        self.names.clear();
        self.names.clone_from(&other.names);
        other.names.clear();
        self.loop_scopes.clear();
        self.loop_scopes.clone_from(&other.loop_scopes);
        self.loop_seq_ranges.clear();
        self.loop_seq_ranges.clone_from(&other.loop_seq_ranges);
        self.scope_origins.clear();
        self.scope_origins.clone_from(&other.scope_origins);
        self.closure_var_map.clear();
        self.closure_var_map.clone_from(&other.closure_var_map);
        other.closure_var_map.clear();
        // @PLN87 P2.1 — carry the parsed pass's rebind-witness map into the
        // stored function so `scopes::check` can emit the function-exit
        // `OpFreeRefIfDistinct`.  Cleared first so a re-parse can't leave a
        // stale param→witness entry.
        self.rebind_orig.clear();
        self.rebind_orig.clone_from(&other.rebind_orig);
        self.owner_witness.clear();
        self.owner_witness.clone_from(&other.owner_witness);
    }

    /// The highest `N` among this table's `<prefix><N>` names — the number a work counter must
    /// start past so a new mint cannot re-claim a name something already depends on.  Matches
    /// `<prefix>` followed by digits ONLY, so `__ref_` does not read `__ref_p2_1` as its own.
    fn highest_minted(table: &Function, prefix: &str) -> u16 {
        table
            .names
            .keys()
            .filter_map(|n| n.strip_prefix(prefix))
            .filter(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
            .filter_map(|rest| rest.parse::<u16>().ok())
            .max()
            .unwrap_or(0)
    }

    pub fn copy(other: &Function) -> Self {
        Function {
            pass2_rebuilt: std::collections::HashSet::new(),
            name: other.name.clone(),
            file: other.file.clone(),
            current_loop: u16::MAX,
            unique: HashMap::new(),
            loop_binds: HashMap::new(),
            mv_field_origin: HashMap::new(),
            // loft#1145 — the ordinal RESTARTS per pass (it is what makes the key stable),
            // but the maps it keys are CARRIED: they are how pass 2 hands each loop the
            // binding pass 1 gave it.  Cleared here, pass 2's third loop found the second
            // loop's binding through the plain `names` map and re-typed it — the failure
            // this pair of clones is the fix for.
            loop_ord: 0,
            loop_ord_of: other.loop_ord_of.clone(),
            body_binds: other.body_binds.clone(),
            loops: other.loops.clone(),
            variables: other.variables.clone(),
            annotated: other.annotated.clone(),
            arm_consumed: other.arm_consumed.clone(),
            // Every work COUNTER starts past the names the table already carries — derived from
            // the NAMES, not copied, because the stored number is not trustworthy at every
            // instantiation (a monomorph built from a pass-1 body, a reset between the passes).
            // The live sets below do not carry.
            // A monomorph is this table plus passes that MINT (a boxed tuple return, a text
            // return buffer, a par lowering), and a name is a SITE only within one pass:
            // `work_refs` reuses an existing `__ref_N` by name when the counter is behind it.
            // Starting at 0 here handed `promote_monomorph_tuple_return` the `__ref_1` the
            // template's tuple-member copy had already claimed, retyping the member's backing
            // into the return record while `t` still depended on it — the caller read zeroes
            // (the join of loft#1361 and the @FR-F-Ret walk, 2026-09-05).  Past the carried
            // names, a monomorph-time mint is always new; the empty sets keep the before/after
            // snapshots the passes take (`work_texts()`) meaning "minted here".
            work_text: Self::highest_minted(other, "__work_"),
            work_ctext: Self::highest_minted(other, "__work_c"),
            work_text_p2: Self::highest_minted(other, "__work_p2_"),
            work_ref: Self::highest_minted(other, "__ref_"),
            work_ref_p2: Self::highest_minted(other, "__ref_p2_"),
            work_vdb: Self::highest_minted(other, "__vdb_"),
            work_kvb: Self::highest_minted(other, "__kvb_"),
            work_fmt: Self::highest_minted(other, "__fmt_"),
            work_texts: BTreeSet::new(),
            work_refs: BTreeSet::new(),
            inline_ref_vars: other.inline_ref_vars.clone(),
            borrow_arm_vars: other.borrow_arm_vars.clone(),
            names: other.names.clone(),
            loop_scopes: other.loop_scopes.clone(),
            loop_seq_ranges: other.loop_seq_ranges.clone(),
            scope_origins: other.scope_origins.clone(),
            logging: other.logging,
            done: other.done,
            closure_var_map: other.closure_var_map.clone(),
            rebind_orig: other.rebind_orig.clone(),
            owner_witness: other.owner_witness.clone(),
        }
    }

    pub fn start_loop(&mut self) -> u16 {
        // loft#1145 — give this loop its per-PASS ordinal, then re-point every name whose
        // body-local binding belongs to it.  On pass 1 the map is empty here and fills as
        // the body parses; on pass 2 the entries are pass 1's, so each loop is handed the
        // binding it had — without this, `names` (cloned from pass 1 by `Function::append`)
        // starts pass 2 pointing at whichever loop bound the name LAST, and the FIRST loop
        // re-typed that slot instead of its own.  The same act `loop_variable` performs for
        // the loop's own variable, at the same moment.
        let ord = self.loop_ord;
        self.loop_ord = self.loop_ord.saturating_add(1);
        let rebind: Vec<(String, u16)> = self
            .body_binds
            .iter()
            .filter(|((o, _), _)| *o == ord)
            .map(|((_, n), v)| (n.clone(), *v))
            .collect();
        for (n, v) in rebind {
            self.names.insert(n, v);
        }
        self.loop_ord_of.insert(self.loops.len() as u16, ord);
        self.loops.push(Iterator {
            inside: self.current_loop,
            variable: u16::MAX,
            on: 0,
            db_tp: u16::MAX,
            value: Box::new(Value::Null),
            coll_var: u16::MAX,
            counter: u16::MAX,
            len_bound: None,
            state_var: u16::MAX,
        });
        self.current_loop = self.loops.len() as u16 - 1;
        self.current_loop
    }

    pub fn loop_var(&mut self, variable: u16) {
        self.loops[self.current_loop as usize].variable = variable;
        // C61.local: track that this variable has served as a for-loop
        // variable at some point in the function.  Used to distinguish
        // a sequential for-loop reuse (safe) from an outer-local shadow
        // (silent clobber) at parse-for time.
        if (variable as usize) < self.variables.len() {
            self.variables[variable as usize].was_loop_var = true;
        }
    }

    /// Has this variable served as a `for` loop's variable anywhere in this function?
    ///
    /// Read by `scopes.rs`'s dep-init prefix (loft#1135): a loop variable is assigned
    /// unconditionally by its own header, inside its own loop, so it never needs a slot
    /// reserved for it somewhere else — and reserving one registers it in the scope of
    /// whatever dep list happened to name it first, which is not where it lives.
    ///
    /// C61.local also reserved this for a future liveness-aware diagnostic that would reject
    /// `x = 5; for x in …` when the outer `x` has a live read after the loop.
    #[must_use]
    pub fn was_loop_var(&self, var_nr: u16) -> bool {
        if var_nr == u16::MAX || (var_nr as usize) >= self.variables.len() {
            return false;
        }
        self.variables[var_nr as usize].was_loop_var
    }

    pub fn set_loop(&mut self, on: u8, db_tp: u16, value: &Value) {
        // D-key-1: a keyed range / partial-key subscript used in a VALUE position (not a
        // `for`/comprehension iterable) reaches `fill_iter` with no active loop.  Skip the
        // loop-slot write instead of indexing `loops[u16::MAX]` and panicking — `parse_key`
        // emits a clean "keyed slice is a for-only iterator" diagnostic that aborts the
        // compile before this never-consumed loop state could matter.
        if self.current_loop == u16::MAX || self.current_loop as usize >= self.loops.len() {
            return;
        }
        let l = &mut self.loops[self.current_loop as usize];
        l.on = on;
        l.db_tp = db_tp;
        *l.value = value.clone();
        // Auto-extract coll_var when the iterated expression is a plain variable.
        // For vector loops this will be overridden by set_coll_var() because the
        // iterator works on a unique temp copy, not the original user variable.
        l.coll_var = if let Value::Var(v) = value {
            *v
        } else {
            u16::MAX
        };
    }

    /// Record the local holding this loop's packed iterator state — see `Iterator::state_var`.
    /// Called from the site that CREATES the cursor, so the two keyed lowerings cannot drift
    /// in what they call it. No-op when there is no active loop, matching `set_loop`.
    pub(crate) fn set_loop_state_var(&mut self, var_nr: u16) {
        if self.current_loop == u16::MAX || self.current_loop as usize >= self.loops.len() {
            return;
        }
        self.loops[self.current_loop as usize].state_var = var_nr;
    }

    /// Override the iterated collection variable after `set_loop`.
    /// Called from `parse_for` for vector loops where a unique temp copy is created:
    /// the iterator runs over the copy, but the user-visible variable is `orig_var`.
    pub fn set_coll_var(&mut self, orig_var: u16) {
        self.loops[self.current_loop as usize].coll_var = orig_var;
    }

    /// Override the iterated collection `value` expression after `set_loop`.
    /// Called from `parse_for` for vector loops so that `is_iterated_value` can compare
    /// the original user-written expression (e.g. `db.items`) instead of the internal
    /// temp-copy variable that `set_loop` records.
    pub fn set_coll_value(&mut self, orig_value: Value) {
        *self.loops[self.current_loop as usize].value = orig_value;
    }

    /// @PLN102 strict-index lint — record the vector whose `len(...)` bounds the current
    /// loop's range (`for i in 0..len(X)` → `X`'s `VecKey`). No-op when there is no active
    /// loop. Set from `parse_in_range_body` right after the range's upper bound is parsed.
    pub(crate) fn set_loop_len_bound(&mut self, vk: crate::parser::operators::VecKey) {
        if self.current_loop != u16::MAX && (self.current_loop as usize) < self.loops.len() {
            self.loops[self.current_loop as usize].len_bound = Some(vk);
        }
    }

    /// @PLN102 strict-index lint — the `len(...)` bound recorded for the active for-loop
    /// whose variable is `var_nr` (walks the active-loop chain like `is_active_loop_var`).
    /// `None` when `var_nr` is not an active loop var or the loop wasn't a `0..len(X)` range.
    pub(crate) fn loop_len_bound(&self, var_nr: u16) -> Option<crate::parser::operators::VecKey> {
        if var_nr == u16::MAX {
            return None;
        }
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].len_bound;
            }
            c = self.loops[c as usize].inside;
        }
        None
    }

    /// C61: returns true when `var_nr` is the loop *variable* (the `<id>`
    /// bound by `for <id> in …`) of any currently active for-loop,
    /// including outer loops.  Used to detect nested same-name loops.
    pub fn is_active_loop_var(&self, var_nr: u16) -> bool {
        if var_nr == u16::MAX {
            return false;
        }
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return true;
            }
            c = self.loops[c as usize].inside;
        }
        false
    }

    /// Returns true when `var_nr` is the collection variable of any currently active
    /// for-loop (including outer loops).  Used to detect unsafe mutation during iteration.
    pub fn is_iterated_var(&self, var_nr: u16) -> bool {
        if var_nr == u16::MAX {
            return false;
        }
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].coll_var == var_nr {
                return true;
            }
            c = self.loops[c as usize].inside;
        }
        false
    }

    /// Returns true when `val` structurally matches the iterated-collection expression of
    /// any currently active for-loop.  Catches field-access cases like `db.items` where
    /// `coll_var` is `u16::MAX` (no single variable covers the expression).
    pub fn is_iterated_value(&self, val: &Value) -> bool {
        if matches!(val, Value::Null) {
            return false;
        }
        // Plan-07 phase 1: compare via unspan() so the iterated-collection
        // expression `db.items` (parsed once for the for-loop) and the
        // mutating expression `db.items` (parsed again at the `+=` site)
        // — each potentially wrapped at a different source position —
        // still compare equal.
        let unspanned = val.unspan();
        let mut c = self.current_loop;
        while c != u16::MAX {
            if *self.loops[c as usize].value.unspan() == *unspanned {
                return true;
            }
            c = self.loops[c as usize].inside;
        }
        false
    }

    /**
    Stop the current loop.
    # Panics
    When this loop is not started.
    */
    pub fn finish_loop(&mut self, loop_nr: u16) {
        assert_eq!(self.current_loop, loop_nr, "Incorrect loop finish");
        self.current_loop = self.loops[self.current_loop as usize].inside;
    }

    /// Register `count_var` as the `#count` of the loop whose iteration variable
    /// is `loop_var` — searched OUTWARD from the current loop, not assumed to be
    /// it.
    ///
    /// loft#794 — `#count` vars are minted on first READ, and a read of an OUTER
    /// loop's `#count` happens while the parser sits in an INNER loop. Stamping
    /// the current loop then re-pointed the inner loop's counter at the outer
    /// loop's variable: with only the outer one read the inner loop silently
    /// incremented the WRONG counter, and with both read in the same body the
    /// inner loop's own count var was left with no init and no stack slot, which
    /// aborted the compiler ("Incorrect var q#count[65535]") on both backends.
    ///
    /// A `loop_var` that names no enclosing loop keeps the current-loop
    /// behaviour — a `#count` on something that is not an enclosing iteration
    /// variable is already diagnosed elsewhere, and this is not the place to
    /// change what it compiles to.
    pub fn loop_count_of(&mut self, loop_var: u16, count_var: u16) {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == loop_var {
                self.loops[c as usize].counter = count_var;
                return;
            }
            c = self.loops[c as usize].inside;
        }
        if self.current_loop != u16::MAX {
            self.loops[self.current_loop as usize].counter = count_var;
        }
    }

    pub fn loop_counter(&mut self) -> u16 {
        self.loops[self.current_loop as usize].counter
    }

    /// How many loops out `variable` names, counting from the innermost — the depth
    /// `#break` / `#continue` jump — or `None` when it names no enclosing loop.
    ///
    /// Matched on the BINDING the name currently denotes, not on the spelling: two `for i`
    /// loops in one function are two variables and only one of them is bound to `i` here
    /// (loft#915).  Comparing names would walk past a loop whose variable is `i#1` and
    /// answer the chain length, which is a jump out of the wrong loop.
    ///
    /// `Option` because "not found" and "the outermost loop" are different answers and
    /// this used to give them the same one: the walk exited on its CONDITION, so falling
    /// off the end returned the chain length — one past the deepest valid level, with no
    /// signal. `Scopes::scan` then indexed `loops.len() - lv - 1` and underflowed a
    /// `usize`, so naming any declared non-loop local in `k#break` was an internal
    /// compiler error reporting index 18446744073709551615 (loft#998). Returning on the
    /// MATCH instead is what makes the missing case impossible to fall out of.
    ///
    /// An unbound name is `None` too — it names no loop, which is the same answer as a
    /// bound one that names no loop.
    pub fn loop_nr(&self, variable: &str) -> Option<u16> {
        let target = self.var(variable);
        if target == u16::MAX {
            return None;
        }
        let mut c = self.current_loop;
        let mut nr = 0;
        while c != u16::MAX {
            if self.loops[c as usize].variable == target {
                return Some(nr);
            }
            c = self.loops[c as usize].inside;
            nr += 1;
        }
        None
    }

    /// The variable names of the enclosing loops, innermost first — what `x#break` may
    /// legally name here. For the diagnostic when it names something else.
    #[must_use]
    pub fn enclosing_loop_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut c = self.current_loop;
        while c != u16::MAX {
            let v = self.loops[c as usize].variable;
            if v != u16::MAX {
                let n = self.name(v);
                // The parser mangles a shadowed loop variable to `i#1`; the author wrote
                // the part before the `#`, and that is what they can type back.
                let n = n.split('#').next().unwrap_or(n);
                if !n.is_empty() && !out.iter().any(|s: &String| s == n) {
                    out.push(n.to_string());
                }
            }
            c = self.loops[c as usize].inside;
        }
        out
    }

    pub fn loop_on(&self, var_nr: u16) -> u8 {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].on;
            }
            c = self.loops[c as usize].inside;
        }
        0
    }

    pub fn loop_value(&self, var_nr: u16) -> &Value {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return &self.loops[c as usize].value;
            }
            c = self.loops[c as usize].inside;
        }
        &Value::Null
    }

    /// The local holding the iterator state of the active loop whose variable is `var_nr`,
    /// or `u16::MAX` when that loop keeps none. Walks the active-loop chain like `loop_on`.
    #[must_use]
    pub fn loop_state_var(&self, var_nr: u16) -> u16 {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].state_var;
            }
            c = self.loops[c as usize].inside;
        }
        u16::MAX
    }

    pub fn loop_db_tp(&self, var_nr: u16) -> u16 {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].db_tp;
            }
            c = self.loops[c as usize].inside;
        }
        u16::MAX
    }

    /// Return the iterated-collection variable for the loop whose index
    /// variable is `var_nr`, or `u16::MAX` when the loop iterates an
    /// expression that isn't a plain variable.  Used by `#remove` to
    /// detect the C60 hash-iteration scratch variable.
    #[must_use]
    pub fn loop_coll_var(&self, var_nr: u16) -> u16 {
        let mut c = self.current_loop;
        while c != u16::MAX {
            if self.loops[c as usize].variable == var_nr {
                return self.loops[c as usize].coll_var;
            }
            c = self.loops[c as usize].inside;
        }
        u16::MAX
    }

    /// Number of variables declared in this function (arguments + locals).
    #[must_use]
    pub fn count(&self) -> u16 {
        self.variables.len() as u16
    }

    /// Plan-04 Phase B.3: frame high-water mark — max slot end across
    /// all non-argument placed locals.  Used by `def_code` (B.3.i) to
    /// emit a single `OpReserveFrame(frame_hwm)` at function entry,
    /// which replaces the N per-block `OpReserveFrame(block.var_size)`
    /// paired with `OpFreeStack`.  Returns 0 if no local is placed.
    ///
    /// Dead code until B.3.i wires the caller — kept `pub` because
    /// `Function` is exported.
    #[must_use]
    #[allow(dead_code)]
    pub fn frame_hwm(&self, context: &Context) -> u16 {
        let mut hwm: u16 = 0;
        for v in &self.variables {
            if v.argument || v.stack_pos == u16::MAX {
                continue;
            }
            let end = v.stack_pos.saturating_add(size(&v.type_def, context));
            if end > hwm {
                hwm = end;
            }
        }
        hwm
    }

    pub fn name(&self, var_nr: u16) -> &str {
        if var_nr as usize >= self.variables.len() {
            return "??";
        }
        &self.variables[var_nr as usize].name
    }

    pub fn set_scope(&mut self, var_nr: u16, scope: u16) {
        assert!((var_nr as usize) < self.variables.len(), "Unknown variable");
        assert_eq!(
            self.variables[var_nr as usize].scope,
            u16::MAX,
            "Variable has a scope"
        );
        self.variables[var_nr as usize].scope = scope;
        self.done = true;
    }

    /// Mark a scope number as corresponding to a loop body (`Value::Loop`).
    /// Variables in loop scopes are freed by `OpFreeStack` when the loop exits;
    /// if-block scopes (`Value::Block`) are NOT marked and live until function return.
    pub fn mark_loop_scope(&mut self, scope: u16) {
        self.loop_scopes.insert(scope);
    }

    /// Returns true if `scope` is a loop-body scope (variables freed by `OpFreeStack`).
    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn is_loop_scope(&self, scope: u16) -> bool {
        self.loop_scopes.contains(&scope)
    }

    /// Record the seq-number range [`seq_start`, `seq_end`) for a loop-body scope.
    /// Called by `compute_intervals` when it finishes traversing a `Value::Loop`.
    pub fn record_loop_range(&mut self, scope: u16, seq_start: u32, seq_end: u32) {
        self.loop_seq_ranges.insert(scope, (seq_start, seq_end));
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn loop_seq_range(&self, scope: u16) -> Option<(u32, u32)> {
        self.loop_seq_ranges.get(&scope).copied()
    }

    pub fn record_scope_origin(&mut self, scope: u16, name: &'static str) {
        let short = match name {
            "For block" => "for",
            "For loop" | "Slice materialise" | "For comprehension" => "loop",
            "Formatted string" => "fmt",
            "" => "if",
            o => o,
        };
        self.scope_origins.entry(scope).or_insert(short);
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn scope_origin(&self, scope: u16) -> &'static str {
        self.scope_origins.get(&scope).copied().unwrap_or("block")
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn first_def(&self, var_nr: u16) -> u32 {
        self.variables[var_nr as usize].first_def
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn last_use(&self, var_nr: u16) -> u32 {
        self.variables[var_nr as usize].last_use
    }

    pub fn scope(&self, var_nr: u16) -> u16 {
        if var_nr as usize >= self.variables.len() {
            return u16::MAX;
        }
        self.variables[var_nr as usize].scope
    }

    #[allow(dead_code)] // used from integration tests (tests/testing.rs)
    pub fn size(&self, var_nr: u16, context: &Context) -> u16 {
        size(&self.variables[var_nr as usize].type_def, context)
    }

    pub fn tp(&self, var_nr: u16) -> &Type {
        if var_nr as usize >= self.variables.len() {
            &Type::Null
        } else {
            &self.variables[var_nr as usize].type_def
        }
    }

    /// Point every `Type::Unknown(stub)` in this function's variable types at `target`.
    ///
    /// Sibling of [`Function::substitute_type`], for adoption rather than instantiation.
    /// The two-pass parser pre-populates the whole variable table in pass 1, so a local
    /// whose declared type named a forward reference is stored as `Unknown(stub)` — and
    /// pass 2, which resolves the name correctly, then meets the pass-1 slot and reports
    /// `cannot change type from (integer, unknown) to (integer, Q)` (loft#944). Run after
    /// pass 1, once the declaration has adopted the stub.
    ///
    /// Returns whether anything changed, so the caller can skip the tracing work.
    pub fn resolve_unknown_stub(&mut self, stub_nr: u32, target: &Type) -> bool {
        let mut changed = false;
        for v in &mut self.variables {
            if let Some(new_tp) = Self::rewrite_unknown(&v.type_def, stub_nr, target) {
                v.type_def = new_tp;
                changed = true;
            }
        }
        changed
    }

    /// `Some(rewritten)` when the subtree named `stub`, `None` when it is unchanged.
    /// Walks children through [`Type::for_each_child`]'s variant list, kept in step with
    /// `Data::rewrite_type_opt` — the same question asked of a variable's type.
    fn rewrite_unknown(t: &Type, stub: u32, target: &Type) -> Option<Type> {
        match t {
            Type::Unknown(n) if *n == stub => Some(target.clone()),
            Type::Vector(inner, deps) => Self::rewrite_unknown(inner, stub, target)
                .map(|new| Type::Vector(Box::new(new), deps.clone())),
            Type::RefVar(inner) => {
                Self::rewrite_unknown(inner, stub, target).map(|new| Type::RefVar(Box::new(new)))
            }
            Type::Rewritten(inner) => {
                Self::rewrite_unknown(inner, stub, target).map(|new| Type::Rewritten(Box::new(new)))
            }
            // The idempotent former, for the reason `Data::rewrite_type_opt` gives (@FR-N-Idem).
            Type::Optional(inner) => Self::rewrite_unknown(inner, stub, target).map(Type::optional),
            Type::Tuple(elems) => {
                let mut changed = false;
                let new_elems: Vec<Type> = elems
                    .iter()
                    .map(|e| match Self::rewrite_unknown(e, stub, target) {
                        Some(new_e) => {
                            changed = true;
                            new_e
                        }
                        None => e.clone(),
                    })
                    .collect();
                changed.then_some(Type::Tuple(new_elems))
            }
            _ => None,
        }
    }

    /// Replace all occurrences of `Type::Reference(tv_nr, _)` with `concrete`
    /// in every variable's type definition.  Used when instantiating a generic template.
    pub fn substitute_type(&mut self, tv_nr: u32, concrete: &Type) {
        let trace_target = crate::log_config::type_timeline_target();
        for (i, v) in self.variables.iter_mut().enumerate() {
            let new_tp = Self::subst_type(v.type_def.clone(), tv_nr, concrete);
            if let Some(target) = &trace_target
                && v.name == *target
                && new_tp != v.type_def
            {
                eprintln!(
                    "[type_timeline] {name} (v_nr={v_nr}) {old:?} -> {new:?}  origin=substitute_type(tv_nr={tv_nr})",
                    name = v.name,
                    v_nr = i,
                    old = v.type_def,
                    new = new_tp,
                );
            }
            v.type_def = new_tp;
        }
    }

    /// `[T ↦ C]` over one type — @FR-G-Mono's *"applied throughout"*, for the variable
    /// table.  Its twin for the signature is `Parser::substitute_type`.
    fn subst_type(tp: Type, tv_nr: u32, concrete: &Type) -> Type {
        match tp {
            Type::Reference(d, deps) if d == tv_nr => {
                // preserve the original deps when substituting T → concrete.
                // The deps carry vector-element borrowing info needed by get_free_vars
                // to suppress FreeRef on loop element variables.
                let mut result = concrete.clone();
                if !deps.is_empty() {
                    for dep in deps {
                        result = result.depending(dep);
                    }
                }
                result
            }
            // The shape is the keystone's to decide; only the LEAF above differs from
            // the `Parser::substitute_type` twin this mirrors (that one drops the deps,
            // this one carries them).  Written as four hand-spelled formers the two
            // twins drifted apart from a third copy that had all seven, so a `T` under
            // a `fn(T) -> T` was rewritten in the type table and left parametric in the
            // signature — the same variable with two types.
            other => other.map_children(&mut |c| Self::subst_type(c.clone(), tv_nr, concrete)),
        }
    }

    pub fn is_independent(&self, var_nr: u16) -> bool {
        // No such variable — e.g. the `u16::MAX` sentinel a file-scope construction
        // carries when there is no destination slot.  Such a var is not an in-place
        // target; report `false` so the caller allocates fresh (mirrors `tp`, which
        // returns `Type::Null` for the same out-of-range index).  Guarding here keeps
        // `P p = P{}` at module scope from indexing an empty var table and panicking.
        if var_nr as usize >= self.variables.len() {
            return false;
        }
        let d = self.variables[var_nr as usize].type_def.depend();
        d.is_empty() || (d.len() == 1 && d[0] == var_nr)
    }

    /// Remove a lifetime dependency for this variable.
    /// Remove a lifetime dependency for this variable.
    ///
    /// Traced under `LOFT_LOG=type_timeline:<var>` like its `depend` sibling, and naming the
    /// caller. Without that, the timeline recorded deps being ADDED and never REMOVED — so a
    /// borrow that gets promoted to an owner looked like a variable that was born owned, and
    /// the promotion site could only be found by reading every `make_independent` call by
    /// hand. That is exactly how @PLN130's F1 had to be tracked down: the container-destroying
    /// free came from a dep strip the instrument could not show.
    #[track_caller]
    pub fn make_independent(&mut self, var_nr: u16, remove: u16) {
        if crate::log_config::type_timeline_target().is_some() {
            let mut after = self.variables[var_nr as usize].type_def.clone();
            if let Some(to) = after.deps_mut()
                && let Some(pos) = to.iter().position(|x| x == &remove)
            {
                to.remove(pos);
            }
            self.trace_type_change(var_nr, &after, "make_independent");
        }
        // `Type::deps_mut` is the one home for the list, and it peels the dep-transparent
        // wrappers the READ side (`Type::depend`) already peels. Spelled inline here, this
        // arm list had drifted behind that one by an `Optional`: a `S?` local's dep could
        // be read and set but never removed, so no nullable heap local could be made an
        // owner and the store it held at scope exit was freed by nobody (loft#1106).
        if let Some(to) = self.variables[var_nr as usize].type_def.deps_mut()
            && let Some(pos) = to.iter().position(|x| x == &remove)
        {
            to.remove(pos);
        }
    }

    /// Is `incoming` the placeholder `declared` with its stubs RESOLVED — the same shape at
    /// every constructor, where a stub in `declared` accepts whatever `incoming` has there?
    ///
    /// The question the declared-placeholder arm of [`Self::change_var_type`] asks: a
    /// declaration still carrying a forward stub must be kept against an ASSIGNMENT that would
    /// overwrite it with an unrelated type (`x: Maybe? = 5` must not become `integer`), and
    /// must be REPLACED by pass 2's re-declaration of the same type with the name resolved
    /// (`c: (integer, Q)` over `(integer, Unknown)`).  Shape, not identity: `Optional(Unknown)`
    /// against `Integer` is not a refinement; `Tuple[Int, Unknown]` against
    /// `Tuple[Int, Reference]` is.  Walks the same variant families
    /// [`crate::data::Type::for_each_child`] names; a leaf compares by `is_equal`.
    fn refines(declared: &Type, incoming: &Type) -> bool {
        use crate::data::Type as T;
        if declared.is_unknown() {
            return true;
        }
        match (declared, incoming) {
            (T::Optional(a), T::Optional(b))
            | (T::RefVar(a), T::RefVar(b))
            | (T::Rewritten(a), T::Rewritten(b))
            | (T::Vector(a, _), T::Vector(b, _)) => Self::refines(a, b),
            (T::Tuple(xs), T::Tuple(ys)) => {
                xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| Self::refines(x, y))
            }
            (T::Function(xa, xr, _), T::Function(ya, yr, _)) => {
                xa.len() == ya.len()
                    && xa.iter().zip(ya).all(|(x, y)| Self::refines(x, y))
                    && Self::refines(xr, yr)
            }
            (T::Iterator(a1, a2), T::Iterator(b1, b2)) => {
                Self::refines(a1, b1) && Self::refines(a2, b2)
            }
            _ => declared.is_equal(incoming),
        }
    }

    /// [`Self::make_independent`] one level down: strip `remove` from the deps of each
    /// ELEMENT of a tuple-typed variable, and say whether anything moved.
    ///
    /// A tuple carries no deps of its own — each element carries its own backing — so
    /// `make_independent` cannot reach them: `deps_mut` on a `Type::Tuple` is `None`, and
    /// asking it would quietly do nothing.  The caller is loft#1365's monomorph pass, which
    /// removes a tuple-member copy the instantiation does not need and has to take the
    /// element's dep on that copy's backing with it; a dep left naming a backing nothing
    /// fills makes the element look owned by an empty variable, and the store the member
    /// really holds is then freed by nobody.
    pub fn make_tuple_members_independent(&mut self, var_nr: u16, remove: &[u16]) -> bool {
        self.retarget_tuple_member_deps(var_nr, remove, None)
    }

    /// [`Self::make_tuple_members_independent`] with a destination: every dep on a member of
    /// `remove` is replaced by a dep on `to` when given, or dropped when not.  The replacement
    /// is what a collapsed copy needs — a member unwrapped to a plain LOCAL is a VIEW of that
    /// local (`(B-View)`), and a tuple whose element depends on nothing is read as OWNING it,
    /// which freed the aliased argument's store at the callee's exit (the keyed tuple-via-local
    /// cell of the @FR-F-Ret guard, both backends, 2026-09-05).
    pub fn retarget_tuple_member_deps(
        &mut self,
        var_nr: u16,
        remove: &[u16],
        to: Option<u16>,
    ) -> bool {
        let mut tp = self.variables[var_nr as usize].type_def.clone();
        let crate::data::Type::Tuple(elems) = &mut tp else {
            return false;
        };
        let mut moved = false;
        for e in elems.iter_mut() {
            let Some(deps) = e.deps_mut() else { continue };
            for d in remove {
                while let Some(pos) = deps.iter().position(|x| x == d) {
                    deps.remove(pos);
                    moved = true;
                }
            }
            if moved
                && let Some(t) = to
                && !deps.contains(&t)
            {
                deps.push(t);
            }
        }
        if moved {
            self.trace_type_change(var_nr, &tp, "retarget_tuple_member_deps");
            self.variables[var_nr as usize].type_def = tp;
        }
        moved
    }

    /// Give `var_nr` EVERY dep the incoming type carries.
    ///
    /// ⚠ Not a loop over [`Self::depend`], and that distinction is the whole point:
    /// `depend` REPLACES the list (`Type::depending` → `with_deps(Deps::frame1(on))`), so
    /// iterating it keeps only the LAST dep.  A value borrowing two sources — the arms of
    /// `pick = if c { a } else { b }` — then records one of them and loses the other, and
    /// the lost source is not known to be borrowed by anything.
    ///
    /// An EMPTY incoming list is a no-op rather than a clear, which is what the loop it
    /// replaces did: "the types agree, adopt the deps" never meant "and drop what you had".
    fn depend_all(&mut self, var_nr: u16, type_def: &Type) {
        self.depend_on_all(var_nr, &type_def.depend());
    }

    /// Give `var_nr` every dep in `on`, for a caller that already holds the list.
    ///
    /// The slice-taking half of [`Self::depend_all`] — same contract, same reason to exist.
    /// Reach for it wherever a loop would otherwise call [`Self::depend`] once per element:
    /// the SAVE side of a save/restore pair loops correctly over the whole list, and a
    /// restore that collapses to the last element is the asymmetry to look for.
    pub(crate) fn depend_on_all(&mut self, var_nr: u16, on: &[u16]) {
        if var_nr == u16::MAX {
            return;
        }
        // `u16::MAX` is a real entry in a dep list — the #328 share-marker — and [`Self::depend`]
        // has always skipped it, so the loops this replaces dropped it too.  Keep dropping it:
        // two downstream decisions read the marker's PRESENCE (`deps.contains(&u16::MAX)` for a
        // struct field's layout, `deps == [u16::MAX]` as its own predicate), so preserving it
        // here would change answers well outside the collapse this fixes.
        let incoming: Vec<u16> = on.iter().copied().filter(|d| *d != u16::MAX).collect();
        match incoming.len() {
            0 => (),
            1 => self.depend(var_nr, incoming[0]),
            _ => {
                let cur = &self.variables[var_nr as usize].type_def;
                let new_tp = cur.with_deps(&crate::data::Deps::frame(incoming));
                self.trace_type_change(var_nr, &new_tp, "depend_on_all");
                self.variables[var_nr as usize].type_def = new_tp;
            }
        }
    }

    #[track_caller]
    pub fn depend(&mut self, var_nr: u16, on: u16) {
        if on != u16::MAX {
            let new_tp = self.variables[var_nr as usize].type_def.depending(on);
            self.trace_type_change(var_nr, &new_tp, "depend");
            self.variables[var_nr as usize].type_def = new_tp;
        }
    }

    pub fn uses(&self, var_nr: u16) -> u16 {
        self.variables[var_nr as usize].uses
    }

    /// Number of variables in this function's table (@PLN107 — for the dead-store walk).
    #[must_use]
    pub fn var_count(&self) -> usize {
        self.variables.len()
    }

    pub fn is_defined(&self, var_nr: u16) -> bool {
        self.variables[var_nr as usize].defined
    }

    pub fn stack(&self, var_nr: u16) -> u16 {
        self.variables[var_nr as usize].stack_pos
    }

    /// Return the lowest byte offset at which a new variable slot can safely be placed —
    /// i.e. the maximum end-byte of all variables that already have an assigned slot.
    ///
    /// Currently unused in production code.  Retained for Step 3 of the stack-slot
    /// assignment plan (`assign_slots` in ASSIGNMENT.md): the linear-scan pass will use
    /// this to find the next free position when no expired slot is available for reuse.
    ///
    /// Note: a naive guard that advances `stack.position` to this value inside
    /// `generate_set` was attempted and reverted — it broke the bridging invariant
    /// (compile-time `stack.position` diverged from the runtime stack pointer).  This
    /// function is correct; the problem was the call site, not the computation.
    pub fn set_stack(&mut self, var_nr: u16, pos: u16) {
        self.variables[var_nr as usize].stack_pos = pos;
    }

    pub fn in_use(&mut self, var_nr: u16, plus: bool) {
        if plus {
            self.variables[var_nr as usize].uses += 1;
        } else {
            self.variables[var_nr as usize].uses -= 1;
        }
    }

    pub fn defined(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].defined = true;
    }

    /// Check for dead assignment (overwritten before read) and update write tracking.
    /// Call this on every `=` assignment to a user variable during the second pass.
    pub fn track_write(&mut self, var_nr: u16, lexer: &mut Lexer) {
        let var = &self.variables[var_nr as usize];
        if var.name.starts_with('_')
            || var.name.contains('#')
            || var.const_binding
            || var.value_const
        {
            return;
        }
        // #625 — the warning below seeks the lexer BACK to the previous write so it
        // reports there, but `to()` moves only the reporting line/pos and never
        // rewinds the read cursor: the tokenizer keeps incrementing that line for
        // every physical line it goes on to pull.  Left unrestored, the seek shifts
        // EVERY later diagnostic in the file back by its own distance — and, because
        // `write_source` below is then captured from the seeked position, each further
        // reassignment stacks another shift (`c = 1; c = 2; c = f();` misreported by
        // two lines).  Hold the true cursor and put it back; the seek is for REPORTING
        // only.  `definitions.rs` does the same around the end-of-function warning
        // passes — this one runs DURING the body parse, which is why it reaches user
        // code that has not been parsed yet.
        let here = lexer.at();
        // A CLOSURE CAPTURE is a read, and it is deliberately not counted in `uses`
        // (the capture site in `parser/vectors.rs` says so, to keep this check from
        // seeing a capture as an ordinary use).  So a captured variable's earlier
        // write can never be shown dead here: `s = 10; f = fn() { s }; s = 20;`
        // hands `f` the 10, and reporting that write as dead advertises a deletion
        // that changes the answer.  Silence on a captured variable is the safe
        // direction for a lint that must never make a program wrong.
        if var.write_source != (0, 0) && var.uses == var.uses_at_write && !var.captured {
            // Variable was written before but not read since — dead assignment
            let name = var.name.clone();
            let prev_source = var.write_source;
            lexer.to(prev_source);
            diagnostic!(
                lexer,
                Level::Warning,
                code = "dead-assignment",
                "Dead assignment — '{}' is overwritten before being read",
                name,
            );
            lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "delete the assignment, or read it before the next one".to_string(),
                condition: Some("nothing between the two writes needs the first value".to_string()),
                edit: None,
                concept: "dead-code lint",
                concept_ref: "@F100",
            });

            lexer.to(here);
        }
        let var = &mut self.variables[var_nr as usize];
        var.uses_at_write = var.uses;
        var.write_source = here;
    }

    /// Save write-tracking state for all variables, then clear pending writes.
    /// Call before entering a branch — the branch should not see pre-branch writes
    /// as "unread" because the branch might not execute.
    pub fn save_and_clear_write_state(&self) -> Vec<(u16, (u32, u32))> {
        self.variables
            .iter()
            .map(|v| (v.uses_at_write, v.write_source))
            .collect()
    }

    /// Restore write-tracking state for all variables (call after leaving a branch).
    pub fn restore_write_state(&mut self, state: &[(u16, (u32, u32))]) {
        for (i, (uses_at_write, write_source)) in state.iter().enumerate() {
            if i < self.variables.len() {
                self.variables[i].uses_at_write = *uses_at_write;
                self.variables[i].write_source = *write_source;
            }
        }
    }

    /// Clear all pending write tracking (no variable has an "unread write").
    pub fn clear_write_state(&mut self) {
        for v in &mut self.variables {
            v.write_source = (0, 0);
        }
    }

    pub fn exists(&self, var_nr: u16) -> bool {
        var_nr < self.variables.len() as u16
    }

    pub fn name_exists(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    pub fn arguments(&self) -> Vec<u16> {
        let mut arg = Vec::new();
        for (v_nr, v) in self.variables.iter().enumerate() {
            if v.argument {
                arg.push(v_nr as u16);
            }
        }
        arg
    }

    pub fn var(&self, name: &str) -> u16 {
        if let Some(nr) = self.names.get(name) {
            return *nr;
        }
        u16::MAX
    }

    /// Return all variable names and their types for capture analysis.
    pub fn all_names_and_types(&self) -> Vec<(String, Type)> {
        self.variables
            .iter()
            .map(|v| (v.name.clone(), v.type_def.clone()))
            .collect()
    }

    pub fn next_var(&self) -> u16 {
        self.variables.len() as u16
    }

    /// Set a name→variable mapping, returning the previous mapping (if any).
    /// Used by match arm bindings (S15) to alias a user-visible field name
    /// to a per-arm unique variable.
    pub fn set_name(&mut self, name: &str, var_nr: u16) -> Option<u16> {
        self.names.insert(name.to_string(), var_nr)
    }

    /// Remove a name→variable mapping.
    pub fn remove_name(&mut self, name: &str) {
        self.names.remove(name);
    }

    /// The name the `for` loop now being parsed binds its variable under (loft#915).
    ///
    /// A loop variable stays a function-scoped local — `i` after the loop still reads the
    /// value the loop left, which programs rely on — but each LOOP gets its own binding,
    /// so a second loop may spell one name at a different element type instead of
    /// re-typing the first loop's slot.  That is also what keeps loft#690 fixed by
    /// construction rather than by diagnostic: the second loop can no longer inherit the
    /// first's type, dep or storage, which is what made it read B's records through A's
    /// layout.
    ///
    /// The FIRST loop to use a name binds the name itself, so a program with no repeat
    /// spells every loop variable exactly as it did before — dumps, the debugger frame and
    /// every diagnostic are untouched.  A second loop over the same name binds `i#1`, a
    /// third `i#2`.  The suffix is on the NAME and not only on a lookup key because the
    /// native backend names a local `var_<name>`, and two locals spelling one name declare
    /// it twice — the same constraint loft#928 hit for a generator's fields.  `#` cannot
    /// occur in a loft identifier, so a suffixed name cannot collide with a variable the
    /// program declared itself.
    ///
    /// `_` and `$` are exempt: both already take a fresh slot per loop through `unique`,
    /// and both must keep working across different element types in one function.
    pub fn loop_binding(&mut self, id: &str) -> String {
        if id == "_" || id == "$" {
            return id.to_string();
        }
        let ctr = self.loop_binds.entry(id.to_string()).or_insert(0);
        let nr = *ctr;
        *ctr += 1;
        if nr == 0 {
            id.to_string()
        } else {
            format!("{id}#{nr}")
        }
    }

    /// Add a `for` loop's variable under `name`, keyed for cross-pass identity by the
    /// LOOP rather than by the name (loft#915).
    ///
    /// `add_variable` reuses by name, and that is what normally gives one variable one slot
    /// across both parser passes.  A loop variable cannot use its own name as that key: the
    /// name is re-pointed at every loop that binds it, so `names["i"]` ends pass 1 holding
    /// the LAST loop's slot, and pass 2's FIRST loop would then be handed it — a text
    /// binding reusing an integer one, which is the shape the whole split exists to stop.
    ///
    /// The key is `<name>#bind`.  `name` is already unique per loop (`loop_binding` gives
    /// the second `for i` the name `i#1`), nothing re-points a `#`-suffixed name, and `#`
    /// cannot occur in a loft identifier — so the key denotes exactly this loop on both
    /// passes.
    pub fn loop_variable(&mut self, name: &str, type_def: &Type, lexer: &mut Lexer) -> u16 {
        // Binding the name is part of creating the loop's variable, on BOTH the create and
        // the reuse path.  Pass 2 opens with the name pointing wherever pass 1 left it —
        // at the LAST loop that bound it — so a reuse that only answered the slot number
        // would leave this loop's body reading the last loop's variable.
        let key = format!("{name}#bind");
        if let Some(nr) = self.names.get(&key) {
            let nr = *nr;
            // loft#950 — adopt pass 2's type when it carries DEPS the stored one lacks, not
            // only when pass 1 left the slot unknown.
            //
            // A binding over a collection that is itself a view must depend on it, or scope
            // exit frees a store the binding does not own.  `for_type` derives that dep from
            // the collection's own — but the collection may only acquire it on PASS 2 (see
            // `collections.rs`: "on the second pass in_type may carry __vdb_N dependencies
            // that were not present on the first pass"), and a known-but-depless pass-1 type
            // is not `is_unknown`, so the better answer was discarded.
            //
            // In moros' client that left `wc` in `for wc in st.tcams` marked OWNS with its
            // DbRef carrying `st`'s store_nr: the loop's exit freed the CLIENT store, whose
            // slot the next allocation reused, and every later `st.field` read float data.
            // A smaller program was green only because pass 1 happened to know the dep
            // already — the rule held by luck, which is why this reproduced in one program
            // and not in a copy of the same function.
            let stored_bare = self.variables[nr as usize]
                .type_def
                .deps_ref()
                .is_none_or(|d| d.is_empty());
            if self.variables[nr as usize].type_def.is_unknown()
                || (stored_bare && type_def.deps_ref().is_some_and(|d| !d.is_empty()))
            {
                self.trace_type_change(nr, type_def, "loop_variable(reuse)");
                self.variables[nr as usize].type_def = type_def.clone();
            }
            self.names.insert(name.to_string(), nr);
            return nr;
        }
        let v = self.new_var(name, type_def, lexer);
        self.names.insert(name.to_string(), v);
        self.names.insert(key, v);
        v
    }

    /// The loop now being parsed, or `u16::MAX` outside any.
    #[must_use]
    pub fn current_loop(&self) -> u16 {
        self.current_loop
    }

    /// The loop a variable was first created inside, or `u16::MAX` at function scope.
    #[must_use]
    pub fn created_in_loop(&self, var_nr: u16) -> u16 {
        self.variables
            .get(var_nr as usize)
            .map_or(u16::MAX, |v| v.created_in_loop)
    }

    /// Would storing `type_def` into `var_nr` be REFUSED as a type change?
    ///
    /// Deliberately CONSERVATIVE: it answers `true` only where the two types stand in
    /// neither direction of `decl_accepts` and neither is a shape some other arm of
    /// [`Self::change_var_type`] accepts.  Everything it is unsure about it calls `false`,
    /// so every case it claims is one that is refused today — which is what lets its only
    /// caller (loft#1145's per-loop rebind) be strictly ADDITIVE: it can turn a refusal
    /// into an acceptance and can never change a program that compiles.
    ///
    /// It is not a second opinion about `change_var_type`'s verdict and must not become
    /// one.  Widening it past that guarantee means extracting the real verdict from
    /// `change_var_type` instead, which is a refactor of a 300-line mutating function and
    /// wants its own before/after IR proof.
    #[must_use]
    pub fn retype_would_be_refused(&self, var_nr: u16, type_def: &Type, data: &Data) -> bool {
        let Some(v) = self.variables.get(var_nr as usize) else {
            return false;
        };
        if self.is_declared(var_nr) {
            // An explicitly ANNOTATED local's type is a contract the author wrote down, and
            // a parameter's belongs to the signature.  Neither is a candidate for a silent
            // rebind, whatever the loop structure says.
            return false;
        }
        let cur = &v.type_def;
        // Unknown is a pass-1 placeholder that pass 2 REFINES, and `Null` is the inferred
        // start of a `(N-Join)` widen.  Both are accepted; neither is a type change.
        if cur.is_unknown()
            || type_def.is_unknown()
            || matches!(cur, Type::Null)
            || matches!(type_def, Type::Null)
        {
            return false;
        }
        // Shapes other arms accept: element-wise tuples, the iterator→vector materialise,
        // and the two spellings of one nullable struct.
        if matches!(cur, Type::Tuple(_))
            || matches!(type_def, Type::Tuple(_))
            || matches!(cur, Type::Vector(_, _) | Type::Iterator(_, _))
            || matches!(type_def, Type::Vector(_, _) | Type::Iterator(_, _))
            || data.same_nullable_struct(cur, type_def).is_some()
        {
            return false;
        }
        !cur.is_equal(type_def)
            && !Self::decl_accepts(cur, type_def)
            && !Self::decl_accepts(type_def, cur)
    }

    /// Give the loop now being parsed its OWN binding for a body local named `name`
    /// (loft#1145) — the body-local twin of [`Self::loop_variable`].
    ///
    /// A body local stays FUNCTION-scoped, exactly as loft#915 left the loop variable:
    /// `for x in xs { e = x } print(e.v)` reads the loop's last value and programs rely on
    /// that.  What each loop gets is its own BINDING, so the second loop cannot inherit the
    /// first's type — which is the whole of loft#690's cure, by construction rather than by
    /// diagnostic.
    ///
    /// The cross-pass key is `(loop ordinal, name)`, NOT a counter over rebinds.
    /// `loop_variable` can key off `loop_binding`'s counter because it runs at EVERY loop
    /// header; a body-local rebind runs only where a retype is refused, so the counter's
    /// sequence differs between the passes and keyed that way pass 2 handed loop 1 loop 2's
    /// binding — measured, and it reported the refusal with its two types the wrong way
    /// round.  The ordinal is stable because `start_loop` runs once per loop in parse order
    /// and the counter is cleared per pass.
    ///
    /// The BINDING NAME carries the ordinal too (`e#b1`), because native codegen names a
    /// local `var_<name>` and two locals spelling one name declare it twice — the same
    /// constraint loft#915 met.  `#` cannot occur in a loft identifier, so it cannot collide
    /// with a name the program wrote.
    /// ⚠ The registry is populated ONLY here, so it is EMPTY for a program in which no loop
    /// ever needs a split — and `start_loop`'s re-point is then a no-op over an empty map.
    /// Registering every body local as it is minted was tried and is much too wide: it
    /// captured loop variables and generated temps (`i#index`, `_elm_2`, `_vector_1`) and
    /// re-pointed their names at every loop entry, which is `loop_variable`'s job and not
    /// this one's.
    pub fn body_local_binding(
        &mut self,
        name: &str,
        old_var: u16,
        type_def: &Type,
        lexer: &mut Lexer,
    ) -> u16 {
        let Some(&ord) = self.loop_ord_of.get(&self.current_loop) else {
            return self.add_variable(name, type_def, lexer);
        };
        // The binding being SPLIT FROM is registered under its own loop's ordinal at the
        // same moment, because that is the first time anything needs to remember it: pass 2
        // has to be able to hand loop 1 its own binding back, and until a split happens
        // there is nothing to distinguish.
        let old_loop = self.created_in_loop(old_var);
        if let Some(&old_ord) = self.loop_ord_of.get(&old_loop) {
            self.body_binds
                .entry((old_ord, name.to_string()))
                .or_insert(old_var);
        }
        let key = (ord, name.to_string());
        if let Some(&nr) = self.body_binds.get(&key) {
            if self.variables[nr as usize].type_def.is_unknown() {
                self.trace_type_change(nr, type_def, "body_local_binding(reuse)");
                self.variables[nr as usize].type_def = type_def.clone();
            }
            self.names.insert(name.to_string(), nr);
            return nr;
        }
        let v = self.new_var(&format!("{name}#b{ord}"), type_def, lexer);
        self.names.insert(name.to_string(), v);
        self.body_binds.insert(key, v);
        v
    }

    pub fn unique(&mut self, name: &str, type_def: &Type, lexer: &mut Lexer) -> u16 {
        let ctr = self.unique.entry(name.to_string()).or_insert(0);
        *ctr += 1;
        let nr = *ctr;
        self.add_variable(&format!("_{name}_{nr}"), type_def, lexer)
    }

    /// Mark a variable as carrying an EXPLICIT `: Type` annotation (vs an inferred type).
    /// An annotated narrow integer stays constrained; an inferred one widens (`widen_int`).
    pub fn set_annotated(&mut self, var_nr: u16) {
        self.annotated.insert(var_nr);
    }

    /// Whether the variable's type came from an explicit annotation.
    #[must_use]
    pub fn is_annotated(&self, var_nr: u16) -> bool {
        self.annotated.contains(&var_nr)
    }

    /// Is this binding's type a COMMITMENT the author wrote down — an explicit `: τ`
    /// annotation, or a parameter, whose type belongs to the signature?
    ///
    /// The one home for the declared / inferred split the storage rules turn on: a declared
    /// slot keeps its type and a wider write is `@FR-N-Decl`'s (and `@FR-N-Store`'s)
    /// question, where an inferred one widens to the join of its writes (`@FR-N-Join`,
    /// `(I-Join)`).  Every site that asks "may this binding's type move?" reads this rather
    /// than spelling `argument || annotated` for itself.
    #[must_use]
    pub fn is_declared(&self, var_nr: u16) -> bool {
        self.variables
            .get(var_nr as usize)
            .is_some_and(|v| v.argument || self.annotated.contains(&var_nr))
    }

    /// Widen an INFERRED integer variable's type directly to `type_def` (the `(I-Join)`
    /// join target).  Bypasses `change_var_type`, which no-ops on integers because
    /// `is_equal` collapses all integer widths to one type.
    pub fn widen_int(&mut self, var_nr: u16, type_def: &Type) {
        if let Some(v) = self.variables.get_mut(var_nr as usize) {
            v.type_def = type_def.clone();
        }
    }

    pub fn add_variable(&mut self, name: &str, type_def: &Type, lexer: &mut Lexer) -> u16 {
        // Due to 2 passes through the code, we will add the same variable a second time.
        if let Some(nr) = self.names.get(name) {
            let nr = *nr;
            let existing = &self.variables[nr as usize].type_def;
            // Refine an unknown; and for GENERATED temps (`__`-prefixed) let PASS 2 WIN
            // on a type CONFLICT: the `__ncc_N`/`__work_N` counters can diverge across
            // the two passes (a `??` that stays trivial in pass 1 materialises a temp in
            // pass 2), so the same NAME can denote a DIFFERENT site per pass. Keeping
            // pass 1's type then hands pass-2 code a contradicting temp — the routing
            // `add_tile` corruption: `txs as integer ?? -1`'s temp kept a pass-1
            // `ref(Img)` type, mis-emitting native (`E0605 as DbRef`) AND mis-reading
            // interp (a silent wrong value). A user variable keeps the old behaviour
            // (its name IS its cross-pass identity; type evolution has its own checks).
            if existing.is_unknown()
                || (name.starts_with("__") && !type_def.is_unknown() && existing != type_def)
            {
                self.trace_type_change(nr, type_def, "add_variable(reuse)");
                self.variables[nr as usize].type_def = type_def.clone();
            }
            return nr;
        }
        self.new_var(name, type_def, lexer)
    }

    /// Create a temporary variable during scope analysis (no Lexer needed).
    /// Reuses an existing variable if the name already exists (two-pass stability).
    /// Used to lift inline struct-returning call arguments.
    pub fn add_temp_var(&mut self, name: &str, type_def: &Type) -> u16 {
        if let Some(nr) = self.names.get(name) {
            let nr = *nr;
            let existing = &self.variables[nr as usize].type_def;
            // Same pass-2-wins rule as `add_variable` (see there): a generated
            // temp's cross-pass identity is name+type, so a conflicting re-add
            // re-types instead of handing back a contradicting temp.
            if existing.is_unknown()
                || (name.starts_with("__") && !type_def.is_unknown() && existing != type_def)
            {
                self.trace_type_change(nr, type_def, "add_temp_var(reuse)");
                self.variables[nr as usize].type_def = type_def.clone();
            }
            return nr;
        }
        let v = self.variables.len() as u16;
        self.names.insert(name.to_string(), v);
        self.variables.push(Variable {
            name: name.to_string(),
            type_def: type_def.clone(),
            source: (0, 0),
            scope: u16::MAX,
            stack_pos: u16::MAX,
            uses: 1,
            uses_at_write: 0,
            write_source: (0, 0),
            argument: false,
            defined: false,
            const_binding: false,
            value_const: false,
            view_elided: false,
            amp_link: false,
            iteration_source: false,
            stack_allocated: false,
            skip_free: false,
            captured: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
            promoted_from: u16::MAX,
            was_loop_var: false,
            created_in_loop: u16::MAX,
            caller_hidden_buf: false,
        });
        v
    }

    /// Create an exact copy of a variable, used to duplicate them when reused in later scopes.
    pub fn copy_variable(&mut self, var: u16) -> u16 {
        let v = self.variables.len() as u16;
        self.variables.push(Variable {
            name: self.variables[var as usize].name.clone(),
            type_def: self.variables[var as usize].type_def.clone(),
            source: self.variables[var as usize].source,
            scope: u16::MAX,
            stack_pos: u16::MAX,
            uses: 1,
            uses_at_write: 0,
            write_source: (0, 0),
            argument: false,
            defined: self.variables[var as usize].defined,
            const_binding: self.variables[var as usize].const_binding,
            value_const: self.variables[var as usize].value_const,
            view_elided: false,
            amp_link: self.variables[var as usize].amp_link,
            iteration_source: self.variables[var as usize].iteration_source,
            stack_allocated: false,
            skip_free: false,
            captured: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
            promoted_from: u16::MAX,
            was_loop_var: false,
            created_in_loop: u16::MAX,
            caller_hidden_buf: false,
        });
        v
    }

    fn new_var(&mut self, name: &str, type_def: &Type, lexer: &mut Lexer) -> u16 {
        let v = self.variables.len() as u16;
        if !self.names.contains_key(name) {
            self.names.insert(name.to_string(), v);
        }
        let in_loop = self.current_loop;
        self.variables.push(Variable {
            name: name.to_string(),
            type_def: type_def.clone(),
            source: lexer.at(),
            scope: u16::MAX,
            stack_pos: u16::MAX,
            uses: 1,
            uses_at_write: 0,
            write_source: (0, 0),
            argument: false,
            defined: false,
            const_binding: false,
            value_const: false,
            view_elided: false,
            amp_link: false,
            iteration_source: false,
            stack_allocated: false,
            skip_free: false,
            captured: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
            promoted_from: u16::MAX,
            was_loop_var: false,
            created_in_loop: in_loop,
            caller_hidden_buf: false,
        });
        v
    }

    #[cfg(test)]
    pub fn add_unique(&mut self, prefix: &str, type_def: &Type, scope: u16) -> u16 {
        let v = self.variables.len() as u16;
        self.variables.push(Variable {
            name: format!("_{prefix}_{v}"),
            type_def: type_def.clone(),
            source: (0, 0),
            scope,
            stack_pos: u16::MAX,
            uses: 1,
            uses_at_write: 0,
            write_source: (0, 0),
            argument: false,
            defined: true,
            const_binding: false,
            value_const: false,
            view_elided: false,
            amp_link: false,
            iteration_source: false,
            stack_allocated: false,
            skip_free: false,
            captured: false,
            first_def: u32::MAX,
            last_use: 0,
            pre_assigned_pos: u16::MAX,
            promoted_from: u16::MAX,
            was_loop_var: false,
            created_in_loop: u16::MAX,
            caller_hidden_buf: false,
        });
        v
    }

    pub fn change_var_type(
        &mut self,
        var_nr: u16,
        type_def: &Type,
        data: &Data,
        lexer: &mut Lexer,
    ) -> bool {
        // A `u16::MAX` / out-of-range `var_nr` is the "no variable" sentinel — nothing to
        // retype (matches the guards in `tp`/`name`/`set_type`). Without it, malformed input
        // whose assignment LHS never resolved to a real variable (`Foo x = 5` with an unknown
        // type `Foo`) panics here instead of being diagnosed.
        if var_nr == u16::MAX || (var_nr as usize) >= self.variables.len() {
            return false;
        }
        let var_tp = &self.variables[var_nr as usize].type_def;
        // `_` is the universal unused variable — allow type changes silently.
        if self.variables[var_nr as usize].name == "_" && !type_def.is_unknown() {
            self.trace_type_change(var_nr, type_def, "change_var_type(_)");
            self.variables[var_nr as usize].type_def = type_def.clone();
            return self.is_new(var_nr);
        }
        // loft#663 — an integer-element vector's element WIDTH is layout-bearing: it
        // IS the store's stride.  `is_equal` deliberately collapses every integer
        // width to one type, so the equality early-return below keeps whichever
        // element type the variable was given FIRST.  When pass 1 could not resolve
        // the callee — a FORWARD-declared or recursive function — that first type
        // carries no declared width; pass 2 resolves the real one, and the collapse
        // then discards it.  The append writes 8-byte elements into a 1-byte-strided
        // store and they read back as 0.  Adopt a declared width over none; the
        // reverse (declared → undeclared) is left to the collapse, so an annotated
        // variable is never widened out from under its declaration.
        let adopt_elem_width = matches!(
            (var_tp, type_def),
            (Type::Vector(cur, _), Type::Vector(new, _))
                if matches!(
                    (&**cur, &**new),
                    (Type::Integer(c), Type::Integer(n))
                        if c.forced_size.is_none() && n.forced_size.is_some()
                )
        );
        if adopt_elem_width {
            self.trace_type_change(var_nr, type_def, "change_var_type(#663 element width)");
            self.variables[var_nr as usize].type_def = type_def.clone();
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // A fn-ref slot ADOPTS a refined RETURN dep, for the reason the element width above
        // is adopted: `is_equal` collapses deps, and the two passes do not know the same
        // thing.  Pass 1 has not parsed the lambda's body, so the type it publishes says the
        // result is owned; pass 2 knows the body hands back what it CAPTURED and says so.
        // Keeping the first answer is what left the call site with an empty dep for a store
        // the outer scope still owns — the bind adopted it and scope exit released it, while
        // `(L-CapHeap)` says a captured heap value is shared (loft#1181).  Same base type
        // either way, so the frame the two passes lay out is unchanged.
        let adopt_fnref_ret = matches!(
            (var_tp, type_def),
            (Type::Function(_, cur, _), Type::Function(_, new, _))
                if cur.is_equal(new) && cur.depend() != new.depend()
        );
        if adopt_fnref_ret {
            self.trace_type_change(var_nr, type_def, "change_var_type(fn-ref return deps)");
            self.variables[var_nr as usize].type_def = type_def.clone();
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // @P376 — assigning the `Never` poison (an errored struct construction,
        // pass 2) to an as-yet-`Unknown` variable must OVERWRITE it to `Never`,
        // NOT take the early-return below.  `is_equal(Unknown, Never)` is true,
        // so without this guard the poison is dropped, the variable stays
        // `Unknown`, and the typo cascades (`p.name` → "Field of unknown
        // variable" → format-string fatal).  Falling through re-types it to
        // `Never`, which field access / format interpolation / the unknown-type
        // sweep all skip — leaving the single `unknown type '…'` diagnostic.
        let var_tp = &self.variables[var_nr as usize].type_def;
        let never_into_unknown = matches!(type_def, Type::Never) && var_tp.is_unknown();
        if !never_into_unknown && (type_def.is_unknown() || var_tp.is_equal(type_def)) {
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // loft#1073 — the same rule as the `type_def.is_unknown()` arm above, one level
        // in.  A bare `Unknown` source is accepted there as "pass 1 has not resolved this
        // yet"; the same fact inside a composite is not, because `is_unknown()` does not
        // see through a `Tuple`.  So `t: (fn(integer) -> integer, integer) = (later, 1)`
        // with `later` declared BELOW measured pass 1's placeholder `(unknown, integer)`
        // against the declared type and rejected a program pass 2 resolves perfectly —
        // while the same forward reference in every other position (a plain local, a call
        // argument, a member ASSIGNMENT) was accepted, because there the placeholder is a
        // bare `Unknown`.
        //
        // The mirror of loft#944, which made the same statement about `var_tp`: a type
        // carrying an unresolved component is not a baseline a change can be measured
        // against, and it is no more a MEASUREMENT than it is a baseline.  Restricted to a
        // resolved current type, so the declared type is KEPT rather than overwritten by
        // the placeholder — pass 2 re-derives the value's type either way, and an
        // unresolvable name has its own diagnostic (`Unknown variable`) to report it.
        if !var_tp.is_unknown()
            && !crate::data::Data::type_has_unresolved(var_tp)
            && crate::data::Data::type_has_unresolved(type_def)
        {
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // The mirror once more, now for the DECLARED side (@PLN153 phase 0): a declared type
        // still carrying a stub under a wrapper — `x: Maybe? = 5` with `type Maybe = integer?`
        // declared below — is a placeholder pass 1 has not resolved, not a baseline the
        // assignment's type may overwrite.  Overwriting it dropped the annotation's `?`: the
        // slot read `integer`, `resolve_unknown_stub` found no stub left to fill, and pass 2's
        // `integer?` was refused as a type change.  Kept instead; the stub rewrite fills it
        // once the declaration adopts the stub, and pass 2 re-derives the value's type.
        // A BARE `Unknown` declaration is not this case — `is_unknown()` — and keeps the
        // pass-1 inference it always had.
        // …unless the incoming type is the placeholder RESOLVED — pass 2 re-declaring
        // `c: (integer, Q)` as `(integer, Reference(Q))` over pass 1's `(integer, Unknown)` is
        // exactly the write this must let through (loft#944's own guard, c1: keeping the
        // placeholder there left the member unresolved for good).  `refines` tells the two
        // apart by SHAPE: same constructors down to the stubs, and a stub accepts anything.
        if !var_tp.is_unknown()
            && crate::data::Data::type_has_unresolved(var_tp)
            && !crate::data::Data::type_has_unresolved(type_def)
            && !Self::refines(var_tp, type_def)
        {
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // @PLN25 (N-Decl): `Optional(τ)` and `τ` share sentinel storage, so storing a
        // non-null `τ` into a nullable `τ?` slot is NOT a type change — accept and KEEP the
        // nullable slot type (do not narrow it to non-null). This is what makes nullable
        // LOCALS usable (`x: integer? = 5`); without it `change_var` rejected the assignment
        // as "cannot change type from integer? to integer". The reverse (an explicit non-null
        // target ← `τ?`) is the `(N-Store)` violation, caught at the store site before here.
        // Gate-OFF inert: the postfix `?` is a no-op so `var_tp` is never `Optional`.
        if let Type::Optional(inner) = var_tp
            // @PLN25 (N-Idem): peel the SOURCE too — `Optional(τ) ← Optional(τ)` (e.g. a
            // `text?` local reassigned from a `text?`-typed if-join whose frame-deps differ)
            // is not a type change. Without `.base()` the source's `Optional` wrapper made
            // `inner.is_equal` fail and change_var wrongly rejected `text? ← text?`.
            && (inner.is_equal(type_def.base()) || matches!(type_def, Type::Null))
        {
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // @PLN25 (N-Decl) composed with a TUPLE — element-wise (loft#1034).
        //
        // The rule above says storing a non-null `τ` into a `τ?` slot is not a type change.
        // It peels ONE `Optional`, at the top, so it never saw `(text?, integer) ← (text,
        // integer)`: the variable's type is a `Tuple`, and `is_equal` compares the elements
        // exactly, where `text?` and `text` differ.  `(text?, integer)` was therefore
        // refused as a declared LOCAL while the identical type was accepted as a RETURN —
        // two sites disagreeing about one type, which is the shape `formal/tuples.md`
        // D-tup-1 collapsed once already.
        //
        // This is the TYPING half only — "is the declaration legal".  Making the VALUE
        // match is a separate step and lives at the assignment site, where a tuple target
        // now reaches `convert` so a `null` element becomes the element type's sentinel.
        // Both are needed and neither subsumes the other: this answers on pass 1, before
        // any lowering has happened, and refusing here stops the declaration outright.
        //
        // Sound on the same premise the scalar rule rests on: `Optional(τ)` shares `τ`'s
        // sentinel storage, and `variables::size` peels `Optional` before measuring, so a
        // `text?` element occupies exactly what a `text` element does and the tuple's
        // layout is unchanged.  The declared type is KEPT, never narrowed to the literal's
        // — the slot stays able to hold null, which is what the annotation asked for.
        //
        // One direction only.  `(text, integer) ← (text?, integer)` is the `(N-Store)`
        // violation and still rejects, because `decl_accepts` widens `τ → τ?` and never
        // the reverse.
        if let (Type::Tuple(want), Type::Tuple(got)) = (var_tp, type_def)
            && want.len() == got.len()
            && want
                .iter()
                .zip(got.iter())
                .all(|(w, g)| Self::decl_accepts(w, g))
        {
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // @FR-N-Join — an inferred local's type is the JOIN of its assignments, made optional
        // when any of them may be null; a declared one is @FR-N-Decl's and never widens.
        // @PLN25 DN6 (N-Join): an INFERRED local first assigned a bare `null`, then a
        // non-null INLINE scalar `τ`, widens to `Null ⊔ τ = τ?` instead of erroring — the
        // ergonomic escape valve for `a = null; a = 5` (a now `integer?`, so a later
        // `b: integer = a` still requires a discharge).  `var_tp == Null` is INHERENTLY the
        // inferred-from-null case: a variable cannot be ANNOTATED `null`, so this never
        // overrides an explicit non-null contract — `a: integer = null` carries
        // `var_tp == integer` and is the case-1 nullable-mix reject below.  Scoped to this
        // ONE direction (the reverse `a = 5; a = null` cannot be told apart from an
        // annotated `a: integer = null` here, so it keeps rejecting).  DN1-gated.
        //
        // SOUNDNESS BOUNDARY — INLINE scalars ONLY (Integer/Boolean/Float/Single/Character).
        // The retroactive widen keeps the slot allocated by the FIRST `= null`; that slot is
        // sound for a τ? only when Null and τ? share it.  Inline scalars carry the null as an
        // in-slot sentinel, so `null`→`τ?` reuses the same inline slot.  `Text` (the only
        // heap-backed scalar here) needs a heap-ref slot with text-position tracking that the
        // Null slot is NOT — widening it corrupts `fn_return`'s discard accounting (interp
        // underflow / native E0308).  A text null-start must annotate `s: text? = null` so the
        // slot is heap from the start; `s = null; s = "hi"` falls through to the case-1
        // nullable-mix error, which already says "declare it `text?`".
        // The source may itself be nullable — `a = null; a = v[i]` — and the join is the
        // same `τ?` over the same inline slot, so the arm reads the source through `base()`.
        if crate::keys::pln25_dn1_enabled()
            && matches!(var_tp, Type::Null)
            && matches!(
                type_def.base(),
                Type::Integer(_) | Type::Boolean | Type::Float | Type::Single | Type::Character
            )
        {
            let widened = Type::optional(type_def.base().clone());
            self.trace_type_change(var_nr, &widened, "change_var_type(N-Join)");
            self.variables[var_nr as usize].type_def = widened;
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // @PLN25 — the SAME nullable struct, spelled two ways, and the two spellings arrive
        // in DIFFERENT PASSES.  A field written `f: S?` reaches the parser as
        // `Optional(Reference(S))`, and `typedef::synth_nullable_struct_fields` rewrites the
        // declared field type to the synthetic `Enum(__nullable<S>, true)` — in `fill_all`,
        // which runs BETWEEN the two parser passes.  So `s = o.f` infers the Optional on pass
        // 1 and the synth on pass 2, and refusing that reported a legal program as a type
        // change between one type and itself ("cannot change type from S? to __nullable<S>"),
        // naming cures — a new name, an `as` cast — that cannot reach it.
        //
        // The SYNTH wins, because the value really is a `__nullable<S>` record: absence needs
        // the discriminant, and a payload sub-reference cannot be null (its record is the
        // holder's — loft#1071).  Both spellings occupy a `DbRef` slot, so the frame the two
        // passes lay out is the same either way.
        if data.same_nullable_struct(var_tp, type_def).is_some() {
            self.trace_type_change(var_nr, type_def, "change_var_type(nullable-synth)");
            self.variables[var_nr as usize].type_def = type_def.clone();
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // @FR-N-Join — the nullable half of the join: an INFERRED local whose next write may
        // be null widens to `τ?` — `a = 2; a = v[i]` makes `a` an `integer?` — exactly as
        // `(I-Join)` widens an inferred narrow integer.  A DECLARED binding never takes this
        // arm: its type is a commitment (`@FR-N-Decl`), so the nullable write is
        // `@FR-N-Store`'s question and the assignment seam asks it through the store face
        // BEFORE the retype ever reaches here (`parse_assign_op_inner`).
        //
        // Sound for the same reason `(N-Decl)`'s arm above is: `Optional(τ)` shares `τ`'s
        // slot, so the binding's frame layout is unchanged and only the type record moves.
        // The join of two integer widths is the WIDER one, so `a = 2 as u8; a = v[i]` widens
        // to `integer?`, not `u8?` — a narrower slot would spend a value on the null.  A
        // `RefVar` binding is a link to another place and keeps its own peel below.
        if !self.is_declared(var_nr)
            && !matches!(var_tp, Type::Optional(_) | Type::Null | Type::RefVar(_))
            && let Type::Optional(inner) = type_def
            && inner.is_equal(var_tp)
        {
            let wider_source = matches!((var_tp, &**inner), (Type::Integer(cur), Type::Integer(new))
                if new.byte_width(true) > cur.byte_width(true));
            let base = if wider_source {
                (**inner).clone()
            } else {
                var_tp.clone()
            };
            let widened = Type::optional(base);
            self.trace_type_change(var_nr, &widened, "change_var_type(N-Join nullable)");
            self.variables[var_nr as usize].type_def = widened;
            self.depend_all(var_nr, type_def);
            return self.is_new(var_nr);
        }
        // Allow assigning an iterator (vector slice) to a vector variable
        // when element types are compatible — the iterator is materialised.
        if let (Type::Vector(_, _), Type::Iterator(_, _)) = (var_tp, type_def) {
            return self.is_new(var_nr);
        }
        if let (Type::Vector(tp, _), Type::Vector(to, _)) = (var_tp, type_def) {
            if to.is_unknown() {
                return self.is_new(var_nr);
            }
            // loft#944 — the element's own unresolved MEMBER counts too, not just a bare
            // unresolved element.  `vector<(integer, unknown)>` is the pass-1 placeholder
            // for `vector<(integer, Q)>`, and rejecting the refinement made pass 2 refuse
            // the literal it had just resolved.
            if !crate::data::Data::type_has_unresolved(tp) {
                self.reject_retype(var_nr, type_def, data, lexer);
            }
        } else if !var_tp.is_unknown()
            // `&unknown` → `&T` (#375): a `&` parameter whose pointee was an
            // unresolved forward / cross-package reference on pass 1 carries the
            // type `RefVar(Unknown)`, which the outer `is_unknown()` does not see
            // through.  Treat it as unknown here so pass 2's resolved `&T`
            // refines it (falling through to the type update below) instead of
            // erroring "cannot change type from &unknown to &T".
            && !matches!(var_tp, Type::RefVar(in_tp) if in_tp.is_unknown())
            // `Never` → `T` (#376): an errored-construction poison (or dead
            // post-divergence code) is the BOTTOM type — re-typeable to anything.
            // Lets pass 2 re-resolve a forward `c = Cell{…}` (poisoned `Never`
            // in pass 1) to its real type instead of erroring "cannot change
            // type from never to Cell".
            && !matches!(var_tp, Type::Never)
            // …and the general form of both of those (loft#944).  A type carrying an
            // unresolved component ANYWHERE is not a baseline a change can be measured
            // against: pass 2 resolving it is a refinement, not a retype.  `is_unknown()`
            // sees through `Vector` alone, which is why `&unknown` and then
            // `(integer, unknown)` each had to be discovered as their own bug —
            // `t: (integer, Q) = (71, q)` with `Q` declared below reported "cannot change
            // type from (integer, unknown) to (integer, unknown)", naming one type twice
            // because both spellings render the unresolved member the same way.
            && !crate::data::Data::type_has_unresolved(var_tp)
        {
            // @PLN25 DN1: peel an `Optional` source — `&text ← text?` is the hoisted
            // work-buffer local (control.rs return-deps hoist) re-assigned from a
            // nullable call result; `Optional(τ)` shares `τ`'s sentinel storage, so the
            // buffer carries the null (`STRING_NULL`) without a type change.
            // loft#1372 — and the other direction, `&τ? ← τ`: the link's SLOT is what a write
            // through it has to fit, and whether that slot may be absent is `@FR-N-Store`'s
            // question, asked where the value lands, not a RETYPE of the link.  Both sides
            // peel, so `&integer? = 7` and `&integer? = null` are admitted exactly as
            // `x: integer? = 7` and `= null` are on the slot itself; unpeeled, the write
            // through a nullable link was refused as *"cannot change type from `&integer?`
            // to `integer`"*, which is @FR-B-Ref-Intro's `&τ` for every τ not holding.
            if let Type::RefVar(in_tp) = var_tp
                && (in_tp.is_equal(type_def.base())
                    || in_tp.base().is_equal(type_def.base())
                    || (matches!(**in_tp, Type::Optional(_)) && matches!(type_def, Type::Null)))
            {
                return self.is_new(var_nr);
            }
            // annotated LHS struct-enum accepts a variant of
            // that enum as RHS.  `let k: Kind = Alpha { x: 1 };` is
            // idiomatic — the struct-literal constructor types the
            // variant as `Reference(variant_d, _)`, but the parent
            // relationship (`def(variant_d).parent == enum_d`)
            // proves subtype compatibility with `Enum(enum_d, true, _)`.
            // loft#1065 — through `base()`, so a NULLABLE struct-enum accepts a variant
            // too.  `s: Shape? = Shape::Circle { r: 7 }` was refused ("cannot change type
            // from Shape? to Circle") while the bare `Shape` beside it was accepted, and
            // a no-payload `Shape::Dot` was accepted either way — because only a variant
            // carrying a RECORD arrives typed as its own `Reference`.  Whether the slot
            // may be absent says nothing about which variants it can hold.
            // loft#1292 — through `RefVar` for the same reason loft#1065 went through
            // `base()`: whether the slot is a live LINK to its source says nothing about
            // which variants it can hold.  `fn f(x: &Shape) { x = Circle { r: 9 }; }` is the
            // write-back `&` exists for, and it was refused ("cannot change type from &Shape
            // to Circle") while the emitter already had an arm for it.
            let lhs_shape = match var_tp {
                Type::RefVar(inner) => inner.base(),
                other => other.base(),
            };
            if let (Type::Enum(parent_d, true, _), Type::Reference(rhs_d, _)) =
                (lhs_shape, type_def)
                && data.def(*rhs_d).parent == *parent_d
            {
                return self.is_new(var_nr);
            }
            // @PLN25 (N-Decl / DN6) — a `null` ↔ non-null-scalar transition is the
            // NULLABILITY case, not a generic type mismatch: `a: integer = null` (the
            // slot is committed non-null) or the inferred `a = null; a = 5` (the slot
            // was `null`). Name the real fix (`τ?`) and NEVER suggest `as` — `x as
            // integer` would LAUNDER the null into the non-null slot (the DN5 hole).
            // (Once `(N-Join)`/DN6 lands, the inferred direction widens silently instead
            // of erroring.) DN1-gated so gate-OFF stays byte-identical: gate-OFF the bare
            // `null` is coerced to the scalar sentinel before here, so `Type::Null` never
            // reaches `change_var` and this branch is unreachable.
            let is_null_scalar = |t: &Type| {
                matches!(
                    t,
                    Type::Integer(_)
                        | Type::Text(_)
                        | Type::Boolean
                        | Type::Float
                        | Type::Single
                        | Type::Character
                )
            };
            let nullable_mix = crate::keys::pln25_dn1_enabled()
                && ((is_null_scalar(var_tp) && matches!(type_def, Type::Null))
                    || (matches!(var_tp, Type::Null) && is_null_scalar(type_def)));
            if nullable_mix {
                let scalar = if is_null_scalar(var_tp) {
                    var_tp
                } else {
                    type_def
                };
                let scalar_name = scalar.source_name(data);
                diagnostic!(
                    lexer,
                    Level::Error,
                    "Variable '{}' cannot hold both `null` and the non-null scalar type `{}` — declare it `{}?` to allow null (do NOT cast with `as`: `null as {}` would store null into a non-null slot)",
                    self.name(var_nr),
                    scalar_name,
                    scalar_name,
                    scalar_name
                );
            } else {
                self.reject_retype(var_nr, type_def, data, lexer);
            }
        }
        self.trace_type_change(var_nr, type_def, "change_var_type");
        self.variables[var_nr as usize].type_def = type_def.clone();
        true
    }

    /// May a value of type `got` be stored into a slot DECLARED `want` without that
    /// counting as a type change?
    ///
    /// This is @PLN25 `(N-Decl)` — "`Optional(τ)` and `τ` share sentinel storage, so
    /// storing a non-null `τ` into a nullable `τ?` slot is NOT a type change" — written so
    /// it can be asked about a nested position instead of only the top one.  A tuple is
    /// checked element by element, so `(text?, integer)` admits `(text, integer)` for the
    /// same reason `text?` admits `text` (loft#1034), and recursively for a tuple inside a
    /// tuple.
    ///
    /// Deliberately asymmetric: it widens `τ → τ?` and never the reverse, so the
    /// `(N-Store)` direction — a non-null slot fed something nullable — keeps rejecting.
    fn decl_accepts(want: &Type, got: &Type) -> bool {
        if want.is_equal(got) {
            return true;
        }
        match (want, got) {
            (Type::Tuple(w), Type::Tuple(g)) => {
                w.len() == g.len()
                    && w.iter()
                        .zip(g.iter())
                        .all(|(a, b)| Self::decl_accepts(a, b))
            }
            // The scalar rule, unchanged: a `τ?` slot takes a non-null `τ` or a bare null.
            (Type::Optional(inner), _) => inner.is_equal(got.base()) || matches!(got, Type::Null),
            _ => false,
        }
    }

    /// Report a rejected re-type of `var_nr`, choosing the advice by WHICH property
    /// of the type changed.
    ///
    /// A `τ` → `τ?` rejection is about nullability, and none of the cures the general
    /// message names get a user out of it: `as τ` is refused by the cast checker for
    /// exactly the reason the store was refused (the value may be null), `as τ?` lands
    /// back on this same rejection, and a fresh variable name only moves the store one
    /// line down. What works is discharging the null at the value — `?` for the type's
    /// default, or `?? <default>` — or widening the variable to `τ?`, and the general
    /// message mentions neither, so following it went in a circle (loft#859).
    ///
    /// Only the ADVICE half differs. The diagnosis — "cannot change type from τ to τ?"
    /// — is right as it stands and stays word for word, so the two messages remain one
    /// diagnostic to anyone reading, grepping or testing for it.
    ///
    /// Everything else keeps the general message: for a genuine type change (`sorted<…>`
    /// → `T`) `as` is the right instrument, which is what it was written for.
    fn reject_retype(&self, var_nr: u16, type_def: &Type, data: &Data, lexer: &mut Lexer) {
        let var_tp = &self.variables[var_nr as usize].type_def;
        // `Never` is the POISON an already-reported expression leaves behind (@P376), not
        // a type the author wrote — there is no source spelling for it, so "cannot change
        // type from integer to never; use a new variable name or cast with 'as'" advertises
        // a cure (`as never`) that cannot be written and buries the root error under it.
        // `y: integer = qqq` earned both lines; only the first one is the author's
        // (loft#934).
        if matches!(type_def, Type::Never) || matches!(var_tp, Type::Never) {
            return;
        }
        let widened_to_nullable = matches!(type_def, Type::Optional(_))
            && !matches!(var_tp, Type::Optional(_))
            && var_tp.is_equal(type_def.base());
        if widened_to_nullable {
            // The SOURCE spelling, not the schema key: this message names a type back to the
            // author four times, and `name` re-spells a keyed payload (`hash<It,["k"]>` for
            // what they wrote as `hash<It[k]>`).
            let base = var_tp.source_name(data);
            diagnostic!(
                lexer,
                Level::Error,
                "Variable '{}' cannot change type from {} to {}; discharge the null where it is produced: `?` (the type's default) or `?? <default>`, or declare it `{}?` to let it hold null (do NOT cast with `as`: `as {}` is refused for the same reason this store is, and `as {}?` returns here)",
                self.name(var_nr),
                base,
                type_def.source_name(data),
                base,
                base,
                base
            );
            return;
        }
        // loft#1146 — `single` ← `float` is the one pairing where "use a new variable name"
        // is not merely incomplete but WRONG: the name is not the problem, and a second
        // variable earns the same rejection.  What a bare `1.5` is missing is the `f`
        // suffix, which `LOFT.md` calls the first cure and names three times — so the
        // refusal knew the answer and offered the other one.  Only the ADVICE half differs,
        // for the same reason the nullable arm above gives: the diagnosis stays word for
        // word, so the three messages remain one diagnostic to anyone grepping for it.
        if matches!(var_tp, Type::Single) && matches!(type_def, Type::Float) {
            diagnostic!(
                lexer,
                Level::Error,
                "Variable '{}' cannot change type from single to float; a bare decimal literal is `float` — write it with the `f` suffix (`1.5f`), or cast the value with `as single`",
                self.name(var_nr)
            );
            return;
        }
        diagnostic!(
            lexer,
            Level::Error,
            "Variable '{}' cannot change type from {} to {}; use a new variable name or cast with 'as'",
            self.name(var_nr),
            var_tp.source_name(data),
            type_def.source_name(data)
        );
    }

    fn is_new(&self, var_nr: u16) -> bool {
        self.variables[var_nr as usize].uses == 0
    }

    pub fn become_argument(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].argument = true;
        self.variables[var_nr as usize].defined = true;
        self.variables[var_nr as usize].stack_allocated = true;
    }

    /// @PLN104 — renumber a FRAME variable `from` → `to` through every variable's
    /// TYPEDEF deps (frame space).  The variable-table companion to the IR walker
    /// (`Parser::renumber_frame_var`) and `swap_variables`: a var swap must move the
    /// deps a typedef holds on OTHER vars too, or the type table desyncs.
    pub fn renumber_frame_in_types(&mut self, from: u16, to: u16) {
        for v in &mut self.variables {
            v.type_def.renumber_frame_deps(from, to);
        }
    }

    /// @PLN104 — swap variable slots `a` and `b`, updating EVERY index-keyed table
    /// that references them (the variable-numbering namespace is a shared medium).
    /// Relocates a late-promoted text retbuf (minted after an inherited body local,
    /// so its variable index exceeds its attribute index) into the slot matching its
    /// attribute index — the `a == v` the returned-type dep needs (loft-lang/loft#568).
    /// SCOPE-keyed tables (`loop_scopes`, `loop_seq_ranges`, `scope_origins`) are left
    /// alone — they key on scope numbers, not variable numbers.  The CALLER must
    /// renumber the IR body and the typedef deps in tandem (`renumber_frame_var` +
    /// `renumber_frame_in_types`), or the code and the table desync.
    pub fn swap_variables(&mut self, a: u16, b: u16) {
        self.variables.swap(a as usize, b as usize);
        // name → index: the two names now resolve to the swapped slots.
        let na = self.variables[a as usize].name.clone();
        let nb = self.variables[b as usize].name.clone();
        self.names.insert(na, a);
        self.names.insert(nb, b);
        swap_in_bset(&mut self.work_texts, a, b);
        swap_in_bset(&mut self.work_refs, a, b);
        swap_in_bset(&mut self.arm_consumed, a, b);
        swap_in_bset(&mut self.inline_ref_vars, a, b);
        swap_in_hset(&mut self.annotated, a, b);
        swap_map_indices(&mut self.closure_var_map, a, b);
        swap_map_indices(&mut self.rebind_orig, a, b);
        swap_map_indices(&mut self.owner_witness, a, b);
    }

    /// @PLAN59 (H1): drop a var from the argument set — used to retire the
    /// signature-time `__retbuf` placeholder when `ref_return` promotes a
    /// real local into the buffer role (the promoted local takes the
    /// placeholder's attribute; `arguments()` then yields the promoted var
    /// in the same last position by number order).
    pub fn retire_argument(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].argument = false;
    }

    /// May the record bind `var = src` carry ABSENCE — is either side typed `τ?`?
    ///
    /// The one question both backends' record-bind emitters ask before choosing the
    /// null-aware bind over the plain allocate-then-copy (`state/codegen.rs`
    /// `gen_set_first_ref_var_copy`, `generation/dispatch.rs` the whole-value record bind).
    /// A copy of an absent value is absent: `OpCopyRecord` from `nullref` reads nothing
    /// and leaves the destination holding the record allocated for it, PRESENT where its
    /// source was absent.  The destination's type is asked as well as the source's because
    /// a source typed non-null can hold `nullref` — a keyed lookup and an element read
    /// trusted by contract are typed bare views, and a miss or an overrun answers the one
    /// value spelling of absence (`DbRef::or_null`, @FR-L-Null); asked of the source alone,
    /// `x = t.h[k]; y: S? = x` on a miss read `y == null` false where `x == null` read
    /// true, on both backends.  A dense destination keeps the plain copy: `(N-Store)` has
    /// already reported that it cannot hold absence.
    #[must_use]
    pub fn bind_admits_absence(&self, var: u16, src: u16) -> bool {
        matches!(self.tp(src), Type::Optional(_)) || matches!(self.tp(var), Type::Optional(_))
    }
    pub fn is_argument(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].argument
    }

    /// Mark `var_nr` binding-const (`const` PREFIX): its slot is write-once.
    pub fn set_const_binding(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].const_binding = true;
    }

    /// Whether `var_nr` is binding-const — a rebind (`=`) is rejected, but the
    /// value it holds stays mutable.
    pub fn is_const_binding(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].const_binding
    }

    /// Mark `var_nr` value-const (`const` before the TYPE): the value is a
    /// read-only borrow — mutation through this name is rejected.
    pub fn set_value_const(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].value_const = true;
    }

    /// Whether `var_nr` is value-const — every mutation THROUGH it (`+=`, element,
    /// field, nested) is rejected; a rebind (`=`) that re-points the slot is allowed.
    pub fn is_value_const(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].value_const
    }

    /// @PLN157 § V-g — mark `var_nr` as a read-only local bound from a borrowing call whose
    /// copy is elided: it keeps its dep (a view) and both backends deliver the call's
    /// result to it directly.  Decided once, in `scopes::scan_set`
    /// (`use_analysis::view_elision_bind`), so the strip and the two copy arms name the
    /// same binds.
    pub fn mark_view_elided(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].view_elided = true;
    }

    /// Whether `var_nr`'s copy from its borrowing call is elided — see
    /// [`Self::mark_view_elided`].
    #[must_use]
    pub fn is_view_elided(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].view_elided
    }

    /// Mark `var_nr` as bound with an explicit `&` at a struct-typed projection —
    /// the author asked for a live link, not a view loft may quietly copy (@PLN130 F9).
    /// Mark `var_nr` as a `for` loop's iteration SOURCE holder — see the field's own doc.
    pub fn set_iteration_source(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].iteration_source = true;
    }

    /// Does the loop iterate THIS variable, so that its identity is load-bearing?
    #[must_use]
    pub fn is_iteration_source(&self, var_nr: u16) -> bool {
        self.variables[var_nr as usize].iteration_source
    }

    pub fn set_amp_link(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].amp_link = true;
    }

    /// Whether `var_nr` was spelled `&` at a struct-typed projection.  The `&` is
    /// otherwise invisible after parsing: `c = &v[0]` and `c = v[0]` emit the same IR.
    pub fn is_amp_link(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].amp_link
    }

    /// Whether `var_nr` carries EITHER const axis — used by the guards that apply to
    /// any const binding (`d#lock` unlock, text-arg auto-promotion, dead-store and
    /// UPPER_CASE lints) regardless of which immutability it is.
    pub fn is_const_any(&self, var_nr: u16) -> bool {
        self.is_const_binding(var_nr) || self.is_value_const(var_nr)
    }

    pub fn is_captured(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len() && self.variables[var_nr as usize].captured
    }

    /// @PLAN51 Cluster IV: mark this variable as a caller-side work-ref
    /// synthesised by `add_defaults` for a callee's hidden return-buffer.
    /// Used by `parse_code`'s preamble null-init loop to ensure these
    /// work-refs receive a `Set(r, Null)` IR regardless of their typedef's
    /// dep list — without it, the slot allocator skips them ("no first_def")
    /// and codegen panics with "Incorrect var __ref_N[65535]".
    /// #319 — add an existing var to the work-ref set so `parse_code`'s
    /// preamble null-init reserves its stack slot.  Used for heap-DbRef
    /// `__ncc_N` temps: their only `Set` lives inside the ncc block (an
    /// operand position the Zone-2 slot scan does not walk), so without a
    /// hoisted `Set(v, Null)` the slot allocator skips them ("no first_def")
    /// and codegen panics with "Incorrect var __ncc_N[65535]".
    pub fn register_work_ref(&mut self, var_nr: u16) {
        self.work_refs.insert(var_nr);
    }

    /// Whether `var_nr` is a registered work-ref temporary (a generated
    /// preamble-allocated buffer such as a vector-literal `_vec_N`) — the
    /// discriminator the return-delivery materializer uses to CONSUME an
    /// owned-fresh arm local: a plain param/user var is also deps-empty but
    /// is caller-owned and must never be freed by the arm.
    #[must_use]
    pub fn is_work_ref(&self, var_nr: u16) -> bool {
        self.work_refs.contains(&var_nr)
    }

    /// Mark a var as consumed in-arm by the return-delivery materializer.
    pub fn set_arm_consumed(&mut self, var_nr: u16) {
        self.arm_consumed.insert(var_nr);
    }

    /// Whether the return-delivery materializer consumed this var in-arm.
    #[must_use]
    pub fn is_arm_consumed(&self, var_nr: u16) -> bool {
        self.arm_consumed.contains(&var_nr)
    }

    /// Retire a work-ref the one-buffer binding substituted out of the IR
    /// (`ref_return`'s chain leg): every use now names the return buffer, so the
    /// variable names no storage at all.  Without this the orphan still gets a
    /// `Set(v, Null)` preamble and a scope-exit free; the presence of FREES then
    /// flips the tail-`If` emission into the discarded-statement + `Return(Null)`
    /// shape that returns the null sentinel on native (the @P378 trap).
    ///
    /// Dropping it from the registry is not the whole retirement, because a
    /// producer other than the bare-call site can have minted it: the `inline ref
    /// copy` projection materialiser marks its ref `inline_ref` as well, and the
    /// null-init for THOSE is inserted from a separate sweep that has already run by
    /// the time a return site substitutes.  `skip_free` is the flag that actually
    /// says "names no storage" — it is what suppresses a free and what tells
    /// `check_ref_leaks` this is a dead declaration rather than a leaked store.
    /// Without it, a `??` over a field reached through a call tripped that assert
    /// under `-C debug-assertions=on` (loft#906): the two parser passes materialise
    /// such a chain at DIFFERENT sites, so pass 2's ref is always the one substituted
    /// out, and it stayed declared, unassigned and unfreed.  The `= null` the earlier
    /// sweep already emitted for it stays behind as a dead store.
    ///
    /// `skip_free` cannot mask a real leak here, and that rests on something the
    /// compiler checks rather than on care: `substitute_work_ref` lists every `Value`
    /// variant explicitly (no wildcard arm), so the rewrite is TOTAL — a surviving use
    /// of `var_nr` is not possible, and a variable with no uses holds no store.
    pub fn unregister_work_ref(&mut self, var_nr: u16) {
        self.work_refs.remove(&var_nr);
        if (var_nr as usize) < self.variables.len() {
            self.set_skip_free(var_nr);
        }
    }

    pub fn mark_caller_hidden_buf(&mut self, var_nr: u16) {
        if (var_nr as usize) < self.variables.len() {
            self.variables[var_nr as usize].caller_hidden_buf = true;
        }
    }

    /// @PLN87 P2.1 — record that visible heap parameter `param` is
    /// whole-binding-reassigned in the body and `orig` is its caller-store
    /// witness (see [`Function::rebind_orig`]).  Idempotent — keyed on `param`.
    pub fn set_rebind_orig(&mut self, param: u16, orig: u16) {
        self.rebind_orig.insert(param, orig);
    }

    /// @PLN87 P2.1 — the witness var for a rebindable heap param, or `None` if
    /// `param` is never wholesale-reassigned (the common case).
    #[must_use]
    pub fn rebind_orig(&self, param: u16) -> Option<u16> {
        self.rebind_orig.get(&param).copied()
    }

    /// @PLN87 P2.1 — every (param, witness) pair, for the entry stash and the
    /// function-exit `OpFreeRefIfDistinct`.
    #[must_use]
    pub fn rebind_params(&self) -> Vec<(u16, u16)> {
        self.rebind_orig.iter().map(|(&p, &o)| (p, o)).collect()
    }

    /// Record that local `v` releases its stores through the owner witness `w`
    /// (see [`Function::owner_witness`]).  Idempotent — keyed on `v`.
    pub fn set_owner_witness(&mut self, v: u16, w: u16) {
        self.owner_witness.insert(v, w);
    }

    /// The owner witness of local `v`, or `None` when `v`'s ownership is static and the
    /// ordinary free placement applies (the common case).
    #[must_use]
    pub fn owner_witness(&self, v: u16) -> Option<u16> {
        self.owner_witness.get(&v).copied()
    }

    pub fn is_caller_hidden_buf(&self, var_nr: u16) -> bool {
        (var_nr as usize) < self.variables.len()
            && self.variables[var_nr as usize].caller_hidden_buf
    }

    /// The variable a const-modification diagnostic should name.  A mutated text
    /// argument is promoted to a `__tp_` local (so a rebind has a slot to write); that
    /// synthetic local carries the const axis but not a user-facing name, so report
    /// against the ORIGINAL parameter it was promoted from.  Non-promoted vars map to
    /// themselves.
    pub fn const_report_var(&self, var_nr: u16) -> u16 {
        let origin = self.variables[var_nr as usize].promoted_from;
        if origin == u16::MAX { var_nr } else { origin }
    }

    /// Returns the appropriate error noun for a const-modification diagnostic.
    /// Parameters say "const parameter"; local variables say "const variable".
    ///
    /// `argument` alone is not the question.  It marks a variable that OCCUPIES an argument
    /// slot, and a heap-typed LOCAL that supplies the function's return value occupies one
    /// too: `ref_return` promotes it to the hidden destination parameter, so
    /// `fn f() -> text { const u: text = "abc"; u = "z"; u }` reported a "const PARAMETER"
    /// about a function that has none (loft#1252).
    ///
    /// `promoted` is the caller's half and has to be passed rather than derived here, because
    /// the fact lives on the DEFINITION: `text_return` / `ref_return` mark the hidden
    /// destination as `attributes[…].hidden` at the same moment they call `become_argument`,
    /// and a variable table cannot see its own definition's attributes.
    /// [`Parser::const_noun`] is the one place that joins the two.
    ///
    /// Do not try to read it off the TYPE.  The promoted local is a `RefVar` and that looked
    /// like a free discriminator — until the shipped suite produced `fn f(a: &const integer)`,
    /// a DECLARED parameter that is also a `RefVar`, and the noun flipped the other way for
    /// it.  (`const &integer` is what does not parse; the `&` goes first.)
    ///
    /// Deliberately NOT a split of the `argument` flag, which is what the issue proposed:
    /// `is_argument` has 141 readers across the parser, codegen, the LSP and introspection,
    /// and `ir_schema` / `ir_store` persist the field — so splitting it is a serialised-format
    /// change, and this question needs no new state to answer.
    pub fn const_kind(&self, var_nr: u16, promoted: bool) -> &'static str {
        if (var_nr as usize) < self.variables.len()
            && self.variables[var_nr as usize].argument
            && !promoted
        {
            "const parameter"
        } else {
            "const variable"
        }
    }

    // text argument auto-promotion helpers

    pub fn set_promoted_from(&mut self, shadow: u16, original: u16) {
        self.variables[shadow as usize].promoted_from = original;
    }

    #[allow(dead_code)]
    pub fn promoted_from(&self, v: u16) -> u16 {
        self.variables[v as usize].promoted_from
    }

    /// Returns (shadow_var_nr, original_arg_var_nr) pairs for promoted text arguments.
    pub fn promoted_text_args(&self) -> Vec<(u16, u16)> {
        self.variables
            .iter()
            .enumerate()
            .filter(|(_, v)| v.promoted_from != u16::MAX)
            .map(|(i, v)| (i as u16, v.promoted_from))
            .collect()
    }

    pub fn remap_name(&mut self, name: &str, new_var: u16) {
        self.names.insert(name.to_string(), new_var);
    }

    #[allow(dead_code)]
    pub fn rename(&mut self, v: u16, new_name: &str) {
        self.variables[v as usize].name = new_name.to_string();
    }

    pub fn mark_used(&mut self, v: u16) {
        self.variables[v as usize].uses += 1;
    }

    pub fn var_source(&self, var_nr: u16) -> (u32, u32) {
        self.variables[var_nr as usize].source
    }

    pub fn test_used(&self, lexer: &mut Lexer, data: &Data, body: &Value, d_nr: u32) {
        for (nr, var) in self.variables.iter().enumerate() {
            if var.name.starts_with('_') || var.name.contains('#') {
                continue;
            }
            // A parameter that another parameter's DEFAULT reads is read — the body is
            // simply not where it happens.  `fn window(rows: integer, height: integer =
            // rows * 10)` counted no use of `rows`, because the default is parsed against
            // temporary variables that are dropped again before the real parameter slots
            // exist (see `definitions.rs`, the `injected` mapping).  So the lint told the
            // author to "drop the parameter `rows` — and its callers' argument", and
            // taking that advice deletes what the default reads: the same shape as a
            // closure capture reading a variable the check could not see.
            if var.argument && Self::read_by_a_parameter_default(data, d_nr, &var.name) {
                continue;
            }
            // A variable the emitted body never even NAMES is a pass-1 leftover, not an
            // unread local (loft#661's class, second half).  Pass 1 could not resolve the
            // match subject — a forward-declared callee is enough — so it bound the arm to
            // a plain variable; pass 2, with the callee resolved, bound the arm to its
            // `_mv_<field>` variable and read THAT.  Variables persist across passes, so
            // the abandoned pass-1 binding survives with `uses == 0` and warns about a
            // name the program does read.  The `Unknown`-type test below catches the
            // leftovers that never got a type; this catches the ones that did.
            //
            // It cannot silence a genuine unused local: `reads_var` counts `Set` TARGETS,
            // so `x = 5` with no read still names `x` and still warns.  Only a LOCAL
            // absent from the body entirely is skipped — a local that is not in the code
            // cannot be one the user failed to read.
            //
            // ARGUMENTS are exempt from the exemption, and that is not a detail: a
            // parameter is declared in the SIGNATURE, so "never appears in the body" is
            // exactly what an unread parameter looks like.  Skipping those silenced
            // `Parameter b is never read` outright — caught by keeping an unused-parameter
            // case in the matrix rather than only unused-local ones.
            if !var.argument && u16::try_from(nr).is_ok_and(|n| !body.reads_var(n)) {
                continue;
            }
            // A variable still typed `Unknown` after pass 2 is a pass-1 LEFTOVER, not
            // something the user wrote and failed to read (loft#661).  Pass 1 parses a
            // `match` whose subject type is not resolvable yet — e.g. a field whose type
            // is declared later in the file — and binds each arm pattern to a plain
            // variable; pass 2, with the enum resolved, binds the arm to its `_mv_<field>`
            // variable instead and reads THAT.  Variables persist across passes, so the
            // abandoned pass-1 binding survives with `uses == 0` and warned about a name
            // the program does read.  Every variable pass 2 actually lowered has a
            // resolved type, so this cannot silence a genuine unused local.
            if matches!(var.type_def, Type::Unknown(_)) {
                continue;
            }
            // @PLN125 arc B — a value held ONLY for its scope-end hook is never read by
            // construction, and that is the idiom the hook exists for:
            //
            //   t = begin(conn);      // nothing reads `t`; the closing brace rolls back
            //
            // So the lint contradicted the feature: the one shape it fires on hardest is
            // the correct one.  A type that declares `OpDrop` says the BINDING is the
            // point, so holding it unread is a use.
            if !var.argument
                && let Type::Reference(d, _) = var.type_def.base()
                && *d < data.definitions()
                && data.drop_hook_nr(*d) != u32::MAX
            {
                continue;
            }
            if var.uses == 0 && !var.captured && data.def_nr(&var.name) == u32::MAX {
                lexer.to(var.source);
                diagnostic!(
                    lexer,
                    Level::Warning,
                    code = "never-read",
                    "{} {} is never read",
                    if var.argument {
                        "Parameter"
                    } else {
                        "Variable"
                    },
                    var.name,
                );
                // A parameter and a local are the same lint but not the same fix: deleting
                // a parameter changes the signature every caller wrote, so that one is the
                // author's call in a way deleting a local is not.
                lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: if var.argument {
                        format!(
                            "drop the parameter `{}` — and its callers' argument",
                            var.name
                        )
                    } else {
                        format!("delete `{}`", var.name)
                    },
                    condition: Some(if var.argument {
                        "the parameter is not part of a signature you must keep".to_string()
                    } else {
                        "computing it has no effect you are relying on".to_string()
                    }),
                    edit: None,
                    concept: "dead-code lint",
                    concept_ref: "@F100",
                });
            }
        }
    }

    /// Does any parameter's stored default read the parameter called `name`?
    ///
    /// A default is held on the SIGNATURE and replayed in the caller's frame, so it
    /// refers to earlier parameters by ARGUMENT INDEX rather than by this function's
    /// var numbering (`definitions.rs` remaps them for exactly that reason). The
    /// lookup therefore goes name → index → every attribute's default, which also
    /// covers a default that was lifted into a function of its own: its arguments are
    /// the same indices.
    fn read_by_a_parameter_default(data: &Data, d_nr: u32, name: &str) -> bool {
        if d_nr == u32::MAX {
            return false;
        }
        let count = data.attributes(d_nr);
        let Some(idx) = (0..count).find(|&a| data.attr_name(d_nr, a) == name) else {
            return false;
        };
        let Ok(idx) = u16::try_from(idx) else {
            return false;
        };
        (0..count).any(|a| data.attr_value(d_nr, a).reads_var(idx))
    }

    /// @PLN107 S1 — observable dump of the dead-store access classification (value-observing
    /// READS vs `OpSet*` WRITE-TARGET bases), gated on `LOFT_DUMP_READS`. Purely diagnostic:
    /// no warning is emitted, so the classifier can be verified against the shape corpus
    /// before S2 wires it into a lint. `uses` is printed alongside to make the read /
    /// write-target split visible against the codegen counter it deliberately does NOT change
    /// (`reads == 0 && write_targets > 0` is the future S2 dead-store signal — see
    /// `doc/claude/plans/107-dead-code-lint/`).
    pub fn debug_dead_store_dump(&self, fn_name: &str, body: &Value, data: &Data) {
        if std::env::var_os("LOFT_DUMP_READS").is_none() {
            return;
        }
        let acc = crate::use_analysis::dead_store_accesses(body, self.variables.len(), data);
        for (i, var) in self.variables.iter().enumerate() {
            if var.name.starts_with('_') || var.name.contains('#') || var.argument {
                continue;
            }
            let (reads, write_targets) = acc.get(i).copied().unwrap_or((0, 0));
            eprintln!(
                "dead-store-dbg: fn={fn_name} var={} uses={} reads={reads} write_targets={write_targets}",
                var.name, var.uses,
            );
        }
    }

    /// Warn on UPPER_CASE non-const locals (P246 follow-up).  The
    /// UPPER_CASE convention is reserved for constants — file-scope
    /// `const NAME = expr;` / `NAME = expr;` and in-fn `const FOO =
    /// expr;`.  A LOCAL written `FOO = …` without the `const` keyword
    /// violates the convention and confuses readers — they expect
    /// UPPER_CASE to mean "compiler-checked immutable" but the
    /// variable can be reassigned.  Emits a Warning telling the user
    /// to either add `const` or rename to lower_case.  Skips
    /// arguments (the `const T` parameter modifier already handles
    /// const-ness on parameters), `_`-prefixed names, and synthetic
    /// names containing `#`.
    pub fn warn_upper_case_locals(&self, lexer: &mut Lexer, body: &Value) {
        for (nr, var) in self.variables.iter().enumerate() {
            if var.argument
                || var.const_binding
                || var.value_const
                || var.name.starts_with('_')
                || var.name.contains('#')
            {
                continue;
            }
            if !is_upper_case_name(&var.name) {
                continue;
            }
            // A variable the emitted body never NAMES is a pass-1 leftover, not a local —
            // the same peel `test_used` takes, and it was missing here (loft#921).  A
            // constant used ABOVE its own `const NAME = …` cannot resolve in pass 1, so
            // the name is parked as a placeholder variable; pass 2 has the declaration
            // and pastes the constant's value, leaving that placeholder unread.  The
            // stale entry then advised that a CONSTANT is a local variable — the one
            // message whose whole job is to say a name is *not* a constant — and it fired
            // only when the declaration sat below the use, so the same constant advised
            // or stayed silent depending on where in the file it was declared.
            //
            // It cannot silence a real UPPER_CASE local: `reads_var` counts `Set` TARGETS
            // too, so `FOO = 1;` names `FOO` whether or not anything reads it back.
            if u16::try_from(nr).is_ok_and(|n| !body.reads_var(n)) {
                continue;
            }
            lexer.to(var.source);
            diagnostic!(
                lexer,
                Level::Advice,
                code = "upper-case-local",
                "Variable '{}' is UPPER_CASE — that style is reserved for constants",
                var.name,
            );
            lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "declare it `const` to make it immutable".to_string(),
                condition: Some("the value never changes after this point".to_string()),
                edit: None,
                concept: "const",
                concept_ref: "@F18",
            });
            lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Mechanical,
                title: "rename it to lower_case".to_string(),
                condition: None,
                edit: None,
                concept: "const",
                concept_ref: "@F18",
            });
        }
    }

    /// Advance EVERY pooled work-name counter past the names already in `names`, so the
    /// next mint yields a genuinely-fresh variable instead of aliasing a live one.
    ///
    /// A mint helper reuses the name it finds in `names`.  That pooling is deliberate
    /// during a parse — it is what lets pass 2 re-find pass 1's buffer in the same role.
    /// Re-entering an ALREADY-PARSED function is where the reuse turns into aliasing: the
    /// counters were reset to 0 when the function was stored (`append`), so the first mint
    /// hands back the buffer the parse already gave to something still live.
    ///
    /// Call this at every such re-entry.  @PLN104 Phase B (`patch_tret_callers`) is the
    /// one today, and the buffer it aliased went to a callee AS ITS OWN ARGUMENT:
    /// `wrap(s())` gave `s`'s result buffer to `wrap` to build its return in, so `wrap`
    /// overwrote the bytes it was reading — `[hi]` came out `[[i]`, and `--native` dropped
    /// the backing `String` while a `&str` still pointed into it, a null-pointer copy
    /// (loft#671).
    ///
    /// **Every sequence belongs in this list.**  Syncing only `__work_N` is what left that
    /// hole: the caller retbuf moved to its own `__work_cN` counter (loft#662) and silently
    /// stopped being covered.  The shared `__work` stem self-disambiguates — `strip_prefix`
    /// leaves `c1` / `p2_1`, which do not parse as a number.
    pub fn sync_work_counters(&mut self) {
        fn index(name: &str, prefix: &str) -> Option<u16> {
            name.strip_prefix(prefix)?.parse::<u16>().ok()
        }
        let (mut text, mut ctext, mut p2, mut refs, mut refs_p2, mut vdb, mut kvb, mut fmt) = (
            self.work_text,
            self.work_ctext,
            self.work_text_p2,
            self.work_ref,
            self.work_ref_p2,
            self.work_vdb,
            self.work_kvb,
            self.work_fmt,
        );
        for name in self.names.keys() {
            if let Some(k) = index(name, "__work_") {
                text = text.max(k);
            }
            if let Some(k) = index(name, "__work_c") {
                ctext = ctext.max(k);
            }
            if let Some(k) = index(name, "__work_p2_") {
                p2 = p2.max(k);
            }
            if let Some(k) = index(name, "__ref_") {
                refs = refs.max(k);
            }
            if let Some(k) = index(name, "__ref_p2_") {
                refs_p2 = refs_p2.max(k);
            }
            if let Some(k) = index(name, "__vdb_") {
                vdb = vdb.max(k);
            }
            if let Some(k) = index(name, "__kvb_") {
                kvb = kvb.max(k);
            }
            if let Some(k) = index(name, "__fmt_") {
                fmt = fmt.max(k);
            }
        }
        self.work_text = text;
        self.work_ctext = ctext;
        self.work_text_p2 = p2;
        self.work_ref = refs;
        self.work_ref_p2 = refs_p2;
        self.work_vdb = vdb;
        self.work_kvb = kvb;
        self.work_fmt = fmt;
    }
    #[track_caller]
    pub fn work_text(&mut self, lexer: &mut Lexer) -> u16 {
        let n = format!("__work_{}", self.work_text + 1);
        self.work_text += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            *nr
        } else {
            self.add_variable(&n, &Type::Text(Deps::none()), lexer)
        };
        self.work_texts.insert(v);
        v
    }

    /// The pass-2-only twin of [`Function::work_text`] — same buffer, own sequence.
    ///
    /// Use it at any mint site that cannot fire on pass 1 (typically one gated
    /// `!first_pass`, or one that needs a callee signature pass 1 has not promoted
    /// yet).  Such a site drawing from `work_text` shifts every later `__work_N`
    /// relative to pass 1; since the variable tables persist BY NAME, pass 2 then
    /// re-finds pass 1's variables under the wrong roles — loft#662.  Drawing from
    /// its own sequence, a pass-2-only mint cannot perturb anyone else's numbering,
    /// and its own names simply have no pass-1 counterpart to collide with.
    ///
    /// The name keeps the `__work` prefix so the free/scope/coroutine/introspect
    /// passes that key on it are unaffected; only `sync_work_text_counter`'s
    /// `__work_<N>` parse skips it, as it does for `__work_c<N>`.
    #[track_caller]
    pub fn work_text_p2(&mut self, lexer: &mut Lexer) -> u16 {
        let n = format!("__work_p2_{}", self.work_text_p2 + 1);
        self.work_text_p2 += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            *nr
        } else {
            self.add_variable(&n, &Type::Text(Deps::none()), lexer)
        };
        self.work_texts.insert(v);
        v
    }

    /// A buffer the CALLER allocates for a callee's hidden `&text` out-param —
    /// the text twin of `work_refs`' `__ref_N` retbuf for a vector/enum callee.
    ///
    /// It gets its own `__work_c<N>` counter rather than sharing `work_text`'s
    /// `__work_<N>` because the two are minted on different schedules, and the
    /// variable tables persist across passes BY NAME (loft#662).  A call to a
    /// text-returning function only needs this buffer once the callee's `&text`
    /// ABI exists — which for a SELF- or forward-recursive callee is not until
    /// pass 1 has promoted it.  Sharing one counter therefore let a pass-2-only
    /// mint shift every later `__work_N` by one, so pass 2's format buffers
    /// re-found pass 1's variables under the wrong roles: the return buffer
    /// landed on a fresh name (growing the signature — "Too few parameters") and
    /// a plain local landed on the variable pass 1 had promoted to a `&text`
    /// parameter.  Separate counters keep `__work_N` driven only by the body's
    /// format sites, which ARE pass-stable.
    ///
    /// The name keeps the `__work` prefix: the free/scope/coroutine/introspect
    /// passes that key on it treat this buffer identically, and only
    /// `sync_work_text_counter`'s `__work_<N>` parse is deliberately skipped.
    #[track_caller]
    pub fn caller_text_buf(&mut self, lexer: &mut Lexer) -> u16 {
        let n = format!("__work_c{}", self.work_ctext + 1);
        self.work_ctext += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            *nr
        } else {
            self.add_variable(&n, &Type::Text(Deps::none()), lexer)
        };
        self.work_texts.insert(v);
        v
    }

    pub fn work_ref(&self) -> u16 {
        self.work_ref
    }

    pub fn work_ref_p2(&self) -> u16 {
        self.work_ref_p2
    }

    pub fn clean_work_refs(&mut self, work_ref: u16) {
        for w in work_ref..self.work_ref {
            let n = format!("__ref_{}", w + 1);
            self.mark_skip_free_by_name(&n);
        }
    }

    /// `clean_work_refs` for the pass-2-only sequence — the caller saves both marks
    /// (`work_ref()` and `work_ref_p2()`) and cleans both, since one abandoned
    /// construction can have minted from either.
    pub fn clean_work_refs_p2(&mut self, work_ref_p2: u16) {
        for w in work_ref_p2..self.work_ref_p2 {
            let n = format!("__ref_p2_{}", w + 1);
            self.mark_skip_free_by_name(&n);
        }
    }

    fn mark_skip_free_by_name(&mut self, n: &str) {
        let v_nr = self.var(n);
        // A name inside the abandoned range can belong to the return-buffer ARGUMENT
        // that `work_refs` stepped over rather than to a work-ref of this construction.
        // The caller owns that buffer, so its free discipline is not ours to change.
        if v_nr == u16::MAX || self.is_argument(v_nr) {
            return;
        }
        // Mark skip_free so get_free_vars does not emit OpFreeRef for this variable.
        // with this explicit flag, keeping the type_def intact for downstream passes.
        // Through the one setter, so `LOFT_SKIPFREE_TRACE` sees this writer like every other.
        self.set_skip_free(v_nr);
    }

    /// A fresh function-scoped work-ref (`__ref_N`), reusing the same variable across
    /// passes when the name is already taken.
    ///
    /// **A work-ref is function SCRATCH, and the return buffer is not scratch.**  On
    /// pass 1 `ref_return` promotes the returned work-ref to an ARGUMENT — so the
    /// `__ref_N` name it leaves behind now denotes the function's return buffer.  The
    /// variable tables persist across passes BY NAME while the counter restarts, and
    /// the sequence is NOT pass-stable (many mint sites can only fire on pass 2, once
    /// types and layouts are known — that is why `__vdb_N` / `__kvb_N` / `__work_p2_N`
    /// have their own counters).  A shifted pass-2 site was therefore handed the
    /// return buffer, and wrote a live value into the buffer the return re-mints with
    /// `OpDatabase`: in loft#848 a value-block's result and an enum variant's payload
    /// both died that way, and the copy at the return read the destroyed store as
    /// `null`.
    ///
    /// So a name that resolves to an argument is STEPPED OVER rather than reused.
    /// Nothing needs that reuse: `__retbuf` is reserved at signature time, so
    /// `ref_return` no longer re-finds the buffer by work-ref name (@PLAN59 H1), and a
    /// return site handed a fresh local is delivered into the buffer by the same
    /// `Bind`/substitute leg that already serves every site whose numbering shifted.
    #[track_caller]
    pub fn work_refs(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let v = loop {
            let n = format!("__ref_{}", self.work_ref + 1);
            self.work_ref += 1;
            let Some(&nr) = self.names.get(&n) else {
                break self.add_variable(&n, tp, lexer);
            };
            if self.retypes_argument(nr, tp) {
                continue;
            }
            self.set_type(nr, tp.clone());
            self.variables[nr as usize].source = lexer.at();
            break nr;
        };
        // A `__ref_N` name identifies a SITE, and only within one pass.  The two passes
        // claim the names in different orders — a callee declared later in the file has
        // no known return type on pass 1, so the call mints no buffer, and on pass 2 it
        // does — so the same name is a different site each time.  `add_variable` states
        // the same thing for the TYPE and lets pass 2 win; the per-site FLAGS follow the
        // same rule, and this mint is where the new site claims them.
        //
        // `skip_free` is the one that carries: `unregister_work_ref` sets it to record
        // that a ref has no uses left, and a freshly minted buffer is by definition a ref
        // that has one.  Leaving it set says "borrows, needs no store" to
        // `gen_set_first_vector_null`, and the callee then receives `DbRef::NULL` where
        // its return buffer belongs (loft#1082).
        self.variables[v as usize].skip_free = false;
        self.trace_work_ref(v, tp);
        self.work_refs.insert(v);
        v
    }

    /// `LOFT_TRACE_WORKREF=1` — one line per work-ref mint: the function, the variable
    /// it resolved to, its type, and the SITE that asked for it (`#[track_caller]`).
    ///
    /// Reach for it when a buffer holds the wrong thing and the var table
    /// (`LOFT_VAR_TABLE`) shows the right names in the wrong roles: the table is the
    /// end state, and what a collision needs is the ORDER the names were claimed in,
    /// which differs between the two parser passes.  That is what showed `__ref_1`
    /// being minted for a call's out-param on pass 2 after pass 1 had promoted the
    /// same name to the return-buffer argument (loft#872) — the table alone said only
    /// that one variable was both.
    #[track_caller]
    fn trace_work_ref(&self, v: u16, tp: &Type) {
        if std::env::var_os("LOFT_TRACE_WORKREF").is_none() {
            return;
        }
        // `arg=` is the half that decides whether a reuse is harmless or a collision, and
        // the trace could not say it — including for its own headline example (loft#872's
        // out-param landing on a promoted return buffer).  A work-ref name is scratch, so
        // pass 2 re-resolving it to the same scratch slot is the intended reuse; the same
        // name resolving to an ARGUMENT means `ref_return` promoted it to the return buffer
        // on pass 1 and a different role is now being handed the buffer.  Filtering a corpus
        // sweep on `arg=yes` is what separates the two: 138 same-name-two-sites hits across
        // `tests/scripts` are almost all the benign kind (loft#1078).
        eprintln!(
            "[workref] fn={} -> v{} {} arg={} tp={tp:?} at {}",
            self.name,
            v,
            self.variables[v as usize].name,
            if self.is_argument(v) { "yes" } else { "no" },
            std::panic::Location::caller()
        );
    }

    /// A work-ref for a mint site that can fire ONLY on pass 2 (every caller sits
    /// under a `!self.first_pass` guard) — see [`work_ref_p2`](Self::work_ref_p2) for
    /// why such a site must not draw from the shared `__ref_N` sequence (loft#848).
    /// Otherwise identical to [`work_refs`](Self::work_refs): the variable is an
    /// ordinary work-ref, so it gets the null-init preamble and the scope-exit free.
    #[track_caller]
    pub fn work_refs_p2(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let v = loop {
            let n = format!("__ref_p2_{}", self.work_ref_p2 + 1);
            self.work_ref_p2 += 1;
            let Some(&nr) = self.names.get(&n) else {
                break self.add_variable(&n, tp, lexer);
            };
            if self.retypes_argument(nr, tp) {
                continue;
            }
            self.set_type(nr, tp.clone());
            self.variables[nr as usize].source = lexer.at();
            break nr;
        };
        self.trace_work_ref(v, tp);
        self.work_refs.insert(v);
        v
    }

    /// Would handing `v` to a mint asking for `tp` CHANGE the type of an argument?
    ///
    /// An argument's type is frozen at the signature: the caller allocates by it and
    /// passes what it allocated.  A work-ref name that resolves to one belongs to the
    /// return buffer `ref_return` promoted on pass 1, and pass 2 re-minting the SAME
    /// name for the SAME role is how the buffer is re-found — that reuse is required
    /// (a lambda's return site grows its attribute from it, and stepping over grows a
    /// second one: "grew a pass-2-only attribute").
    ///
    /// What must not happen is a mint for a DIFFERENT role landing on that name, since
    /// the only signal it has is the type it asked for.  In loft#872 a call's out-param
    /// buffer asked for `vector<StoredHex>` and got the record buffer of the function it
    /// was inside: the callee then cleared the caller's record as a vector and built into
    /// it, so the value arrived empty and the write that followed landed out of bounds —
    /// silently on the interpreter, as a `store_nr == 65535` panic on native.  Deps are
    /// not part of the question (`without_deps`): they say where a value came from, not
    /// what storage it is.
    fn retypes_argument(&self, v: u16, tp: &Type) -> bool {
        crate::keys::work_ref_stepover_enabled()
            && self.is_argument(v)
            && self.tp(v).without_deps() != tp.without_deps()
    }

    /// Work-ref for `vector_db()` — uses a separate `__vdb_N` counter/namespace.
    /// `vector_db` only runs on the second pass (it is guarded by `!first_pass`),
    /// so it must NOT share the `work_ref` / `__ref_N` counter with `add_defaults`.
    /// Using a distinct counter prevents the name-shift that would cause
    /// `ref_return` to fail its name-based attr match and add a spurious attr.
    /// These variables are inserted into `work_refs` so they receive null-inits.
    /// loft#703 — a function-scoped work-ref that OWNS a keyed collection's store, for a
    /// keyed literal standing in VALUE position (a return, a call argument).  See the
    /// `work_kvb` field for why it is neither a `__ref_N` nor a `__vdb_N`.
    #[track_caller]
    pub fn work_keyed(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        // The accumulator OWNS the store it builds — that is the whole reason this
        // namespace is function-scoped, so the exit sweep frees it.  It therefore takes
        // the target's SHAPE and none of its borrows: `tp` is an expected type read off
        // the context, and in a `??` default that context is the JOIN, whose deps name the
        // holder the other arm reads.  Adopting them made the accumulator read as a
        // borrow, so no free leg claimed it and every keyed `h ?? […]` retained one store
        // per evaluation — unbounded in a loop, while the `vector` twin, whose default is
        // a dep-free function-scoped temp, was clean.
        let tp = &tp.without_deps();
        let n = format!("__kvb_{}", self.work_kvb + 1);
        self.work_kvb += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            let nr = *nr;
            self.set_type(nr, tp.clone());
            nr
        } else {
            self.add_variable(&n, tp, lexer)
        };
        self.work_refs.insert(v);
        v
    }

    /// @PLN124 — the accumulator a format string builds when its target type
    /// implements the interpolation contract, in its own `__fmt_N` namespace.
    ///
    /// Function-scoped, like [`Function::work_keyed`] and for the same reason:
    /// the accumulator IS the value the expression produces, so no wrapper
    /// record backs its store, and a block-local temp would leave one in an
    /// argument position unfreed.
    #[track_caller]
    pub fn work_format(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let n = format!("__fmt_{}", self.work_fmt + 1);
        self.work_fmt += 1;
        let v = if let Some(nr) = self.names.get(&n) {
            let nr = *nr;
            self.set_type(nr, tp.clone());
            nr
        } else {
            self.add_variable(&n, tp, lexer)
        };
        self.work_refs.insert(v);
        v
    }

    #[track_caller]
    pub fn work_vec_db(&mut self, tp: &Type, lexer: &mut Lexer) -> u16 {
        let n = format!("__vdb_{}", self.work_vdb + 1);
        self.work_vdb += 1;
        let v = self.add_variable(&n, tp, lexer);
        self.work_refs.insert(v);
        v
    }

    /// Mark `v` as an inline-ref temporary (created by `parse_part` for chained
    /// ref-returning calls).  These get their null-init inserted AFTER the first
    /// user statement in `parse_code` so they appear in `var_order` after user-scope
    /// reference variables, giving the correct LIFO-reversed free order.
    pub fn mark_inline_ref(&mut self, v: u16) {
        self.inline_ref_vars.insert(v);
    }

    /// Record that `v` is assigned a BORROW on at least one path and an owned value on
    /// another — the MIXED binding whose single ownership fact cannot be right for both.
    ///
    /// `deps` is empty for such a local (the owned arm contributed none and the join dropped
    /// the borrow's), so @FR-O-Proxy answers "owned" and a displacement free releases the
    /// store the borrow arm only borrowed.  @FR-O-Complete is the rule — the fact is per
    /// BINDING and per PATH — and where one static site cannot separate the paths, erring
    /// toward NOT owned is the direction it names: a leak is recoverable, a premature free is
    /// not.
    pub fn mark_borrow_arm(&mut self, v: u16) {
        self.borrow_arm_vars.insert(v);
    }

    /// Is `v` a mixed binding, borrowed on some path?  Read by ONE site — the fn-ref
    /// collection-delivery strip in `scopes.rs` (loft#1333), which then leaves the binding's
    /// dep in place.  Neither backend's displacement free reads this flag: both read the DEPS
    /// (`Self::owns_displaced_store`), and a dep the strip no longer empties is what keeps them
    /// from freeing on the proxy — agreeing, as @FR-O-NoDiverge requires, through the one fact
    /// they share rather than through this one.  A record-typed mixed local is covered by a
    /// different route again, its owner witness (`owner_witness_locals`, @FR-O-Witness).
    #[must_use]
    pub fn has_borrow_arm(&self, v: u16) -> bool {
        self.borrow_arm_vars.contains(&v)
    }

    /// Does `v`, a NULLABLE heap local, borrow exactly ONE argument of this function — the
    /// D-own-16 residual `d: S? = p`, whose single dep names a parameter and nothing else?
    ///
    /// Such a local's dep reads as a permanent borrow though a later minting call may hand it
    /// a store of its own; the displaced free it licenses is the GUARDED one, decided at run
    /// time by store identity against the argument the dep names (`free_displaced` declines a
    /// free-protected store, so the caller's argument survives the first round).  The ONE
    /// spelling of a test three sites carried by hand — the interpreter's pre-`Set` free
    /// (`state/codegen.rs`), the native reassignment gate (`generation/dispatch.rs`) and the
    /// scope-exit sweep's borrow witness (`scopes.rs`) — so the two backends cannot drift on it
    /// (@FR-O-NoDiverge).
    #[must_use]
    pub fn borrows_one_argument(&self, v: u16) -> bool {
        let d = self.tp(v).depend();
        d.len() == 1
            && d[0] != v
            && matches!(self.tp(v), Type::Optional(_))
            && self.is_argument(d[0])
            && !self.is_argument(v)
    }

    /// May the store `v` DISPLACES at `v = value` be freed — the fact-reading half of the
    /// displacement free, asked identically by both backends?
    ///
    /// `v` names a store-backed kind (a record, a record enum, a vector or a keyed collection,
    /// read through `base()` so the nullable spelling is the same binding); its dep list says it
    /// OWNS — empty (@FR-O-Proxy) or the one-argument borrow above; it is not never-free
    /// (@FR-O-Override, the veto the proxy needs) and not captured (a closure holds its
    /// capture-time `DbRef`, @FR-L-CapHeap); and the assignment is not a DETACH, which
    /// displaces a store without claiming to have owned it (`is_null_sentinel_detach`).
    ///
    /// Enforces @FR-O-NoDiverge: this was two predicates, `state/codegen.rs`'s `owned_ref` and
    /// `generation/dispatch.rs`'s `owned_ref_reassign`, each carrying the other's list
    /// "verbatim" by hand — and each time one gained a kind or a veto the other had to be found
    /// and taught it (the keyed kinds, the vector destination, the override veto, the detach:
    /// four such rounds are in their history).  What stays per backend is only what IS per
    /// backend: the interpreter excludes the hidden buffer ARGUMENT (the caller owns that
    /// store), native asks that the Rust local be already declared, that the right-hand side
    /// produce a store, and that a retbuf-attr local carry an entry-buffer witness.
    #[must_use]
    pub fn owns_displaced_store(&self, v: u16, value: &Value, data: &Data) -> bool {
        (matches!(
            self.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
        ) || crate::parser::vectors::is_keyed(self.tp(v).base()))
            // @FR-O-Proxy asks free — the displacement free follows on this answer, so the
            // @FR-O-Override veto is consulted right after it, as every free on the proxy must.
            && self.proxy_says_owned_or_arg(v)
            && !self.is_captured(v)
            && !crate::data::is_null_sentinel_detach(v, value, data, self)
    }

    pub fn is_inline_ref(&self, v: u16) -> bool {
        self.inline_ref_vars.contains(&v)
    }

    /// Must no ownership-derived free ever be emitted for `v` — in ANY of a free's five
    /// spellings (`OpSets::frees`), from the scope-exit sweep, a transition free, a pre-`Set`
    /// free or a move alike?  Its contract is that sentence, and nothing weaker; the one
    /// admissible free of a marked binding is the release its marking pass PLACES itself, on
    /// a consumption fact ([`Self::is_staged_text_temp`]), and `ownership_cfg`'s Check D
    /// reports every other.
    ///
    /// Enforces @FR-O-Override — the never-free veto.
    ///
    /// ⚠ It exists because @FR-O-Proxy is unsound alone.  The way sites read `deps` for
    /// ownership is `tp.depend().is_empty()` — which is only a PROXY: it
    /// answers "owned" for a borrow whose dep list was never populated (loft#723).  A `??`
    /// subject of `Reference` type is exactly that shape — the parser materialises it into
    /// `__ncc_N` and marks it here, while its type keeps empty deps.  So this flag has to
    /// VETO the proxy at every site that frees on it, not only at the scope-exit sweep in
    /// `Scopes::get_free_vars`.
    ///
    /// Consulting it only at scope exit is what left an unconditional pre-Set free
    /// reachable in a loop body: the free landed on the NEXT iteration's store — stale
    /// bytes without `LOFT_POISON`, SIGSEGV with it.
    ///
    /// Set by `clean_work_refs` for work-ref temporaries re-purposed after use, and by the
    /// `??` lowering for a borrowed subject.
    /// Is `v` a BORROWED VIEW the parser mints and OVERWRITES before any read — a match /
    /// `is` payload binding (`_mv_<field>_N`) or a `??` coalesce subject (`__ncc_N`)?
    ///
    /// Distinct from [`Self::is_skip_free`], whose only contract is *"emit no `OpFreeRef`"* — a
    /// free-time fact that an OWNED keyed return-local also carries.  Reading `skip_free` as
    /// this question is what @PLN85 A.1 de-conflated at `gen_keyed_null`, correctly: a keyed
    /// return-local that owns its store was left at the `u16::MAX` sentinel and OOB-panicked
    /// the next record op.
    ///
    /// ⚠ But the de-conflation dropped this half rather than renaming it, and the KEYED site
    /// then allocated a store for every match binding and orphaned it the moment the arm
    /// overwrote the slot with its projection — one store per call, unbounded in a loop
    /// (loft#1155).  The vector twin kept the broad gate and stayed clean, which is exactly
    /// why `vector` was the control in that issue's own measurements.
    ///
    /// Both facts are required: the PREFIX says which kind of temp this is, and `skip_free`
    /// confirms the parser marked it as a view rather than an owner.
    #[must_use]
    pub fn is_overwritten_view(&self, v: u16) -> bool {
        if !self.is_skip_free(v) {
            return false;
        }
        let n = self.name(v);
        n.starts_with("_mv_") || n.starts_with("__ncc_")
    }

    pub fn is_skip_free(&self, v: u16) -> bool {
        self.variables[v as usize].skip_free
    }

    /// Does the deps PROXY say `v` owns its store, with the never-free veto discharged?
    ///
    /// The two obligations @FR-O-Proxy names, travelling together — which is the only way
    /// that rule permits the proxy to be read at a site that frees. `tp(v).depend().is_empty()`
    /// is the cheap stand-in for *"this binding owns its store"* and is unsound alone: a
    /// borrow whose dep list was never populated reads empty too, and answers "owner" for a
    /// borrower (loft#723). [`Self::is_skip_free`] is the veto that makes it safe, and its
    /// contract is *no ownership-derived free, in any spelling, for this binding*.
    ///
    /// **One home because the conjunction was written out at six free sites** — the arm's
    /// backing-store release in `parser/control.rs`, three ownership-TRANSITION frees and the
    /// drop-cascade hook in `scopes.rs`, and the move-elision shortcut in `state/codegen.rs`
    /// — each of which had to be found and taught the veto separately. That is how the
    /// pre-`Set` free in a loop body came to read the proxy without it, landing on the next
    /// iteration's store (@PLN155 phase 1).
    ///
    /// ⚠ This is the PROXY, not the oracle. @FR-O-Oracle's independent derivation is
    /// `use_analysis::ownership_of`, which never consults `deps` — so the two can disagree,
    /// and 8.0 % of emitted frees rest on this predicate with the oracle having nothing to
    /// say (`make licence-census`, @PLN155 phase 0).
    ///
    /// ⚠ Three sites that free on `deps` do NOT ask this question, and each is marked at its
    /// own site. `Scopes::tuple_owned_elem_frees` reads the proxy off a tuple ELEMENT's type
    /// and the veto off the CONTAINER binding — two subjects, which no predicate over one
    /// `v` can express. The two dep-STRIPPING sites in `Scopes::scan_set` read
    /// `!depend().is_empty()` as *"is there a dep list to strip"* rather than as an
    /// ownership answer; only their veto is this rule's obligation. Merging those onto this
    /// predicate would couple three questions that must stay free to differ.
    /// Is `v` a VECTOR the parser marked never-free that still carries deps — a borrowed view
    /// it holds rather than a store it owns?
    ///
    /// The complement of [`Self::proxy_says_owned`], and a notion in its own right: the veto
    /// says nobody frees this binding on an ownership derivation, and the non-empty dep list
    /// says there is something it still views.  A `_mv_` match-field binding is the shape.
    ///
    /// One home because two sites asked it independently — `Parser::ref_return`, deciding
    /// which arms need their borrow copied before the return, and
    /// `Parser::jo_copy_borrowed_arm_yield`, deciding whether to build the owned `mvcopy` —
    /// and both had to name three conditions to say one thing.
    #[must_use]
    pub fn is_marked_vector_borrow(&self, v: u16) -> bool {
        // @FR-O-Proxy asks free — read as the COMPLEMENT: a true answer says this binding is
        // a marked borrow, so no ownership-derived free is emitted for it.
        self.is_skip_free(v)
            // @FR-N-Shape — a SHAPE question answers alike for `τ` and `τ?`, and "is this a
            // vector" is one.  Both folded sites matched bare; peeling is measured
            // byte-identical over the corpus, so the fold keeps its meaning and the rule gets
            // its answer in the same step.
            && matches!(self.tp(v).base(), Type::Vector(_, _))
            && !self.tp(v).depend().is_empty()
    }

    /// [`Self::proxy_says_owned`] WIDENED: a binding whose dep list names exactly one
    /// ARGUMENT still counts as owning what it displaces.
    ///
    /// The displacement question is not the sweep's.  A local that borrows one argument still
    /// has a store of its own to release when a reassignment displaces it, and the dep naming
    /// that argument is what makes the release decidable at runtime — so the proxy is read
    /// wider here on purpose.  Routed through the pair rather than respelling it, because the
    /// VETO is the same obligation either way: @FR-O-Override does not soften because the
    /// proxy did.
    #[must_use]
    pub fn proxy_says_owned_or_arg(&self, v: u16) -> bool {
        // @FR-O-Proxy asks free — the displacement release follows on this answer, and the
        // @FR-O-Override veto rides on both halves: through the pair on the left, spelled on
        // the right because the widening does not exempt it.
        self.proxy_says_owned(v) || (self.borrows_one_argument(v) && !self.is_skip_free(v))
    }

    #[must_use]
    pub fn proxy_says_owned(&self, v: u16) -> bool {
        // @FR-O-Proxy asks free — this IS the free question, and @FR-O-Override is the
        // conjunct beside it, which is the whole point of the predicate.
        self.tp(v).depend().is_empty() && !self.is_skip_free(v)
    }

    /// Is `v` a text temp that STAGES a value across the statement or the return that reads
    /// it — a `??` coalesce subject (`__ncc_N`) or a return-delivery stage (`__ret_N`,
    /// `__ret_text_N`)?
    ///
    /// Such a temp is never-free for the scope-exit sweep — its value outlives the block the
    /// sweep would free it in — and is released by the pass that staged it, at the site that
    /// pass chose: the ncc-orphan pass right after the consuming statement, the return delivery
    /// once the bytes have moved into the caller's buffer.  @FR-O-Override names this as the ONE
    /// admissible free of a never-free binding — a release placed on a CONSUMPTION fact, never
    /// on the ownership proxy — and `ownership_cfg`'s Check D admits exactly this predicate.
    /// Both facts are required, as in [`Self::is_overwritten_view`]: the PREFIX says which pass
    /// staged it, and `skip_free` that the pass did mark it.
    #[must_use]
    pub fn is_staged_text_temp(&self, v: u16) -> bool {
        if !self.is_skip_free(v) || !matches!(self.tp(v).base(), Type::Text(_)) {
            return false;
        }
        let n = self.name(v);
        n.starts_with("__ncc_") || n.starts_with("__ret_")
    }

    /// Mark a variable so that `get_free_vars` will not emit `OpFreeRef` for it.
    /// Used for borrowed references (e.g. par-loop result variables that point
    /// into the result vector store).
    /// Lift the never-free mark — the binding has stopped being the borrow it was marked for.
    ///
    /// Two callers, and they are the same fact in two passes: a view that is given a store of
    /// ITS OWN stops being a view, and a binding that owns a store must free it.
    /// `@FR-O-Override`'s contract is that a MARKED binding is never freed; this retires the
    /// marking rather than freeing around it.
    ///
    /// * `(B-View)`'s materialise — a view live across a disturbance of its container.
    ///   Stripping the deps without lifting the mark left a materialised `_mv_<field>_N`
    ///   holding a record nothing released: one leaked record per call on `--native`.
    /// * @PLN101's value-struct copy (`value_struct_copy`) — a view into a TAINTED local is
    ///   materialised for the same reason, and it inherited the same omission: the parser marks
    ///   the `??` temp `skip_free` because its subject is a borrowed place read, that pass makes
    ///   it an owner, and the stale mark suppressed the free.  `(v[0] ?? d)` on a value struct
    ///   leaked one record per evaluation, both backends (loft#1472).
    ///
    /// ⚠ The DEP LIST and this MARK are two spellings of one question — *does this variable own
    /// its store?* — so a site that changes the first owes the second.  Two passes have now
    /// learned that separately; a third that strips deps should call this rather than rediscover
    /// it.
    pub fn clear_skip_free(&mut self, v: u16) {
        self.variables[v as usize].skip_free = false;
    }

    #[track_caller]
    pub fn set_skip_free(&mut self, v: u16) {
        if let Ok(want) = std::env::var("LOFT_SKIPFREE_TRACE")
            && (want == "*" || self.variables[v as usize].name == want)
        {
            eprintln!(
                "[skip_free] {} (var={v}) in {} @ {}",
                want,
                self.name,
                std::panic::Location::caller()
            );
        }
        self.variables[v as usize].skip_free = true;
    }

    /// Mark a variable as captured by a closure.
    /// Suppresses the "never read" warning without affecting dead-assignment tracking.
    ///
    /// ⚠ This flag is LOAD-BEARING FOR CODEGEN as well as for diagnostics:
    /// [`rebind_must_mint`](Self::rebind_must_mint) reads it to deny in-place store reuse
    /// on a rebind (loft#1447).  Narrowing where it is set to quiet a warning would
    /// silently restore a wrong VALUE, not just a lint.
    pub fn set_captured(&mut self, v: u16) {
        self.variables[v as usize].captured = true;
    }

    /// Register an existing variable as a work-reference so that `parse_code`
    /// inserts `Set(v, Null)` at the function body start.  This pre-reserves v
    /// in the outer scope, ensuring its frame slot survives inner-block FreeStack.
    pub fn add_to_work_refs(&mut self, v: u16) {
        self.work_refs.insert(v);
    }

    /// Return true if `v` is a compiler-generated temporary — its
    /// name starts with `_`, which `Function::unique` reserves for
    /// the `_<kind>_<counter>` prefix (e.g. `_elm_N`, `_for_result_N`,
    /// `_vector_N`, `__ref_N`, `__vdb_N`).  User-declared loft
    /// variables cannot start with `_` (parser rejects such names),
    /// so this reliably distinguishes owned user locals from aliases
    /// and internal scratch slots that borrow storage from an
    /// enclosing container.
    #[must_use]
    pub fn is_compiler_generated(&self, v: u16) -> bool {
        self.variables[v as usize].name.starts_with('_')
    }

    /// Does `v` own the store it points at — i.e. may a site allocate into its
    /// slot and free it independently?
    ///
    /// ONE home for a fact three different derivations used to answer separately
    /// (loft#664): an empty dep list, the `_elm` NAME prefix loft#660 had to match
    /// on, and the structural "defined by `OpNewRecord`" scan.  Deps alone cannot
    /// carry it — a dep names the borrow SOURCE, so a borrow with no source
    /// VARIABLE (a vector inside an enum payload is addressed by a field DbRef)
    /// came back with an empty list and read as OWNING, which is a wrong answer
    /// rather than an unknown one.  So the two markers are read first and the deps
    /// only decide what they do not cover:
    ///
    /// - `inline_ref` — "borrow, don't allocate", set at every producer of a
    ///   non-owning slot (a vector-literal element, a lift temp, a rebind backing);
    /// - `skip_free` — the free-time half of the same fact;
    /// - otherwise a non-empty dep list means the value is a view of something
    ///   else, and a lone SELF dep is the @P302 owned-keyed-local marker.
    #[must_use]
    pub fn owns_store(&self, v: u16) -> bool {
        // `u16::MAX` reaches here from a file-scope construction with no destination
        // slot; report "does not own" so the caller allocates fresh, as `is_independent`
        // does for the same sentinel.
        if v as usize >= self.variables.len() {
            return false;
        }
        !self.is_inline_ref(v) && !self.is_skip_free(v) && self.is_independent(v)
    }

    /// @FR-L-CapHeap — must a whole-value REBIND of this local mint a FRESH store,
    /// rather than re-mint the one its slot already holds?
    ///
    /// Deliberately a separate predicate from [`owns_store`](Self::owns_store) rather
    /// than a widening of it.  `owns_store` answers who owes the FREE and is shared with
    /// `generation::dispatch` so parser and codegen cannot drift (loft#664); changing what
    /// it means changes the free path.  This one asks only whether in-place REUSE is
    /// licensed, and the two answers differ for exactly one population: a captured heap
    /// local still owns its store, so `owns_store` is true, while a closure record built
    /// over it is a SECOND holder and in-place reuse is not licensed.
    ///
    /// `(L-CapHeap)` says the closure answers the value it was BUILT with, so re-minting
    /// the same store in place makes it answer the rebind instead — loft#1447, where a
    /// captured `d: C = C{5}` read 9 after `d = C{9}` while its nullable twin read 5.
    ///
    /// Every rebind, not only one after the build: `set_captured` runs when the closure
    /// BODY is parsed, so on pass 2 this is known for the whole function, and minting for
    /// a rebind that precedes the build is semantically identical to re-minting (nothing
    /// observes the store yet) at the cost of one allocation on a path that is, by
    /// construction, already building a closure.
    pub fn rebind_must_mint(&self, v: u16) -> bool {
        (v as usize) < self.variables.len()
            && self.is_captured(v)
            && crate::data::is_dbref(self.tp(v).base())
    }

    /// Record that fn_ref variable `fn_ref` has its closure stored in `clos`.
    pub fn set_closure_var_of(&mut self, fn_ref: u16, clos: u16) {
        self.closure_var_map.insert(fn_ref, clos);
    }

    /// Return the closure variable number for a fn_ref variable, if any.
    pub fn closure_var_of(&self, fn_ref: u16) -> Option<u16> {
        self.closure_var_map.get(&fn_ref).copied()
    }

    pub fn inline_ref_references(&self) -> Vec<u16> {
        self.inline_ref_vars.iter().copied().collect()
    }

    pub fn work_texts(&self) -> Vec<u16> {
        let mut res = Vec::new();
        for v in &self.work_texts {
            res.push(*v);
        }
        res
    }

    pub fn work_references(&self) -> Vec<u16> {
        let mut res = Vec::new();
        for v in &self.work_refs {
            res.push(*v);
        }
        res
    }

    /// Set the pre-assigned stack position for `var`.  Called once per argument during
    /// argument layout in `def_code`; the caller advances `stack.position` separately.
    pub fn set_stack_pos(&mut self, var: u16, pos: u16) {
        // After assign_slots has run (pre_assigned_pos != u16::MAX),
        // interpreter codegen should not move variables to a different slot.
        // Native codegen has its own slot management and may legitimately adjust.
        // This assertion is a diagnostic — it logs but does not block.
        #[cfg(debug_assertions)]
        {
            let v = &self.variables[var as usize];
            if v.pre_assigned_pos != u16::MAX
                && v.pre_assigned_pos != pos
                && !v.argument
                && std::env::var("LOFT_SLOT_LOG").is_ok()
            {
                eprintln!(
                    "[set_stack_pos] '{}' scope={}: assign_slots placed at {} but \
                     codegen is moving to {}",
                    v.name, v.scope, v.pre_assigned_pos, pos,
                );
            }
        }
        self.variables[var as usize].stack_pos = pos;
    }

    /// Plan-22 02d-vii follow-up — `LOFT_LOG=type_timeline:<varname>`
    /// trace.  Called from every site that mutates a variable's
    /// `type_def` field; logs to stderr when the env var matches the
    /// variable's name.  No-op fast path when the env var is unset
    /// or has a different value (one `env::var` read per call;
    /// type-mutations are rare).
    /// `#[track_caller]` so the timeline names the SOURCE LINE that rewrote the type.
    /// A dep list is overwritten, not merged (`Type::depending`), so "who wrote this
    /// dep last" is the whole question when a borrow points at the wrong variable
    /// (loft#666) — and the origin word alone ("depend") cannot answer it.
    #[track_caller]
    fn trace_type_change(&self, var_nr: u16, new_tp: &Type, origin: &str) {
        let Some(target) = crate::log_config::type_timeline_target() else {
            return;
        };
        if (var_nr as usize) >= self.variables.len() {
            return;
        }
        let v = &self.variables[var_nr as usize];
        if v.name != target {
            return;
        }
        eprintln!(
            "[type_timeline] {name} (v_nr={v_nr}) {old:?} -> {new:?}  origin={origin} at {site}",
            name = v.name,
            v_nr = var_nr,
            old = v.type_def,
            new = new_tp,
            site = std::panic::Location::caller(),
        );
        // `LOFT_TIMELINE_BT=1` adds the stack behind that line.  The immediate caller is
        // often a shared helper (`change_var_type`'s adopt-deps branch rewrites a dep list
        // on behalf of whoever assigned), and the question is always which PARSE site is
        // behind it.
        if std::env::var_os("LOFT_TIMELINE_BT").is_some() {
            eprintln!("{}", std::backtrace::Backtrace::force_capture());
        }
    }

    /// Record that pass 2 has rebuilt `var_nr`'s borrow list — `true` the FIRST time only.
    ///
    /// loft#1466: see [`Self::pass2_rebuilt`].
    pub(crate) fn mark_pass2_rebuilt(&mut self, var_nr: u16) -> bool {
        self.pass2_rebuilt.insert(var_nr)
    }

    #[track_caller]
    pub fn set_type(&mut self, var_nr: u16, tp: Type) {
        self.trace_type_change(var_nr, &tp, "set_type");
        self.variables[var_nr as usize].type_def = tp;
    }

    /// Reset every non-argument variable's `stack_pos` and
    /// `pre_assigned_pos` to `u16::MAX`.  Called by V1's
    /// `assign_slots` inline; V2's caller calls this explicitly
    /// before invoking `assign_slots_v2` so a stale state from
    /// an earlier pass does not leak.
    #[allow(dead_code)]
    pub fn reset_local_slots(&mut self) {
        for v in &mut self.variables {
            if !v.argument {
                v.stack_pos = u16::MAX;
                v.pre_assigned_pos = u16::MAX;
            }
        }
    }

    pub fn var_type(&self, var_nr: u16) -> &Type {
        &self.variables[var_nr as usize].type_def
    }

    /// Returns `true` when codegen has already emitted the first-allocation init opcodes
    /// for this variable (e.g. `OpText`, `OpConvRefFromNull`).  Used by A6.3 to replace
    /// the `stack_pos == u16::MAX` first-assignment guard in `generate_set`.
    pub fn is_stack_allocated(&self, var_nr: u16) -> bool {
        self.variables[var_nr as usize].stack_allocated
    }

    /// Mark `var_nr` as having been allocated on the stack (call once per variable,
    /// when the first-allocation init opcodes are emitted in `generate_set`).
    pub fn set_stack_allocated(&mut self, var_nr: u16) {
        self.variables[var_nr as usize].stack_allocated = true;
    }
}

pub fn size(tp: &Type, context: &Context) -> u16 {
    match tp {
        // @PLN25 slice (b): `Optional(τ)` shares its base's sentinel storage — same size.
        Type::Optional(inner) => size(inner, context),
        // A declared `size(N)` on an integer alias wins over the range heuristic
        // below, which only knows a 1 / 2 / 8 ladder and so has no way to express
        // 4.  For every alias that existed before this arm the two agree (`u8` /
        // `i8` force 1 and range to 1; `u16` / `i16` force 2 and range to 2; plain
        // `integer` forces nothing), so reading the declaration changes no
        // constant that was already being emitted — verified by a byte-identical
        // `loft introspect` over a corpus of all of them.  It is what makes a
        // 4-byte constant expressible at all, which the jump displacement needs.
        Type::Integer(s) if context == &Context::Constant && s.forced_size.is_some() => {
            u16::from(s.forced_size.expect("checked by the guard").get())
        }
        Type::Integer(s) if context == &Context::Constant && s.range() - 1 <= 256 => 1,
        Type::Integer(s) if context == &Context::Constant && s.range() - 1 <= 65536 => 2,
        Type::Boolean | Type::Enum(_, false, _) => 1,
        Type::Single | Type::Character => 4,
        Type::Integer(_) | Type::Float => 8,
        Type::Function(_, _, _) => 20, // Phase 2c: 8B d_nr (i64) + 12B closure DbRef
        Type::Text(_) if context == &Context::Variable => size_of::<String>() as u16,
        Type::Text(_) => size_of::<&str>() as u16,
        Type::RefVar(_)
        | Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Sorted(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _)
        | Type::Iterator(_, _) => size_of::<DbRef>() as u16,
        Type::Tuple(elems) => crate::data::element_stack_size(&Type::Tuple(elems.clone())) as u16,
        _ => 0,
    }
}

/// Stack alignment (in bytes) a value of type `tp` requires so that an
/// `addr`/`addr_mut::<T>` into the eval stack / frame slot forms a sound,
/// aligned reference (@PLAN53 cluster 2).
///
/// Unlike [`size`], this is **not** context-dependent: stack `text` is
/// either an align-8 `String` (Variable context) or an align-8 `Str`
/// (otherwise) — both contain a raw pointer, so 8 either way.  This is
/// deliberately STRONGER than `data::element_align` (which aligns the
/// record-stored `Str` to 4); records keep their own weaker layout on
/// the `element_align` path and are unaffected.  Not meaningful for
/// `Context::Constant` (byte-packed bytecode operands) — callers only
/// use it for stack/frame layout.
// S3: called by the aligned V2 allocator (`slots_v2::assign_slots_v2`).
pub fn align(tp: &Type) -> u8 {
    match tp {
        // @PLN25: `Optional(τ)` shares its base's sentinel storage — same align as the
        // base (mirrors `size`, which already peels). Missing this aligned an `integer?`
        // stack slot to 1 instead of 8 → misaligned i64 reference → UB/SIGSEGV.
        Type::Optional(inner) => align(inner),
        Type::Boolean | Type::Enum(_, false, _) => 1,
        Type::Single | Type::Character => 4,
        Type::Integer(_) | Type::Float | Type::Function(_, _, _) => 8,
        // String (Variable) and Str (otherwise) both hold a raw pointer → align 8.
        Type::Text(_) => 8,
        Type::RefVar(_)
        | Type::Reference(_, _)
        | Type::Vector(_, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Sorted(_, _, _)
        | Type::Enum(_, true, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _)
        | Type::Iterator(_, _) => 4, // DbRef = u16 + u32 + u32 → align 4
        // @PLN114 — a stack tuple's alignment is the strongest alignment ITS OWN
        // elements need on the stack, so recurse through THIS function rather than
        // the record table.  `data::element_stack_align` gives `Text` 4 (the record's
        // weaker `Str` rule); on the stack a `Str` holds a raw pointer and needs 8, as
        // the doc above says.  Taking the record answer left `(P, text)` locals
        // 4-aligned and landed the `Str` on a 4-mod-8 address — real UB, caught by
        // `stack_align_guard` once the matrices joined its sweep.
        Type::Tuple(elems) => elems.iter().map(align).max().unwrap_or(1),
        _ => 1,
    }
}

/// @PLAN53 cluster 2 / S4 — one eval-TOS / frame-reserve advance step.
///
/// When `aligned`, round `size` up to 8 (the max alignment on the
/// stack) so successive pushes stay 8-aligned and every typed write
/// lands on its required boundary; the LIFO pop reverses the same
/// rounded step (the design's § 3 "uniform-8 step" choice).  When not
/// `aligned`, returns `size` unchanged — V1's tight, real-size step.
///
/// This is the single seam the S4 work toggles: route every
/// `stack_pos += size` / `-= size` and `stack.position += size` site
/// through it so codegen and runtime advance in lockstep (S1).
#[must_use]
#[inline]
pub fn aligned_stack_step(size: u32) -> u32 {
    size.next_multiple_of(8)
}

#[cfg(test)]
mod align_tests {
    use super::*;

    // S2 (@PLAN53 cluster 2): the stack-alignment table.  Text is align-8
    // (both `String` and `Str` hold a raw pointer); small types stay tight
    // (align 1/4) so they pack into the holes alignment leaves.
    #[test]
    fn align_values_for_stack_layout() {
        assert_eq!(align(&Type::Text(Deps::none())), 8);
        assert_eq!(align(&Type::Boolean), 1);
        assert_eq!(align(&Type::Character), 4);
        assert_eq!(align(&Type::Single), 4);
        assert_eq!(align(&Type::Float), 8);
        assert_eq!(align(&crate::data::I64), 8);
    }

    // S4 (@PLAN53 cluster 2): the eval-TOS step — every advance rounds up to
    // the 8-byte max-alignment so typed writes always land on their boundary.
    #[test]
    fn aligned_stack_step_contract() {
        assert_eq!(aligned_stack_step(1), 8);
        assert_eq!(aligned_stack_step(4), 8);
        assert_eq!(aligned_stack_step(8), 8);
        assert_eq!(aligned_stack_step(12), 16);
        assert_eq!(aligned_stack_step(16), 16);
        assert_eq!(aligned_stack_step(0), 0);
    }
}

#[cfg(test)]
mod loop_binding_dep_tests {
    use super::Function;
    use crate::data::{Deps, Type};
    use crate::lexer::Lexer;

    /// loft#950 — a `for` binding whose type pass 1 got RIGHT-SHAPED but DEPLESS must adopt
    /// pass 2's dep-carrying answer.
    ///
    /// The binding is a view into the collection it iterates, so the dep is what stops scope
    /// exit freeing a store it does not own.  `loop_variable` used to adopt only when pass 1
    /// left the slot `Unknown`, and a known-but-depless type is not unknown — so the right
    /// answer was computed on pass 2 and discarded, and moros' client freed its `Client`
    /// store at the exit of `for wc in st.tcams`.
    ///
    /// ⚠ Asserted on the PREDICATE rather than through a program, deliberately.  Whether
    /// pass 1 knows the dep is a property of the whole program — every reduction of the
    /// reported function stayed green because pass 1 happened to know it there — so a
    /// script-level guard would be pinning the luck, not the rule.
    #[test]
    fn a_depless_pass1_binding_adopts_pass2s_dep() {
        let mut lexer = Lexer::default();
        let mut f = Function::new("t", "t.loft");
        let bare = Type::Reference(7, Deps::none());
        let with_dep = Type::Reference(7, Deps::frame1(3));

        let first = f.loop_variable("wc", &bare, &mut lexer);
        let again = f.loop_variable("wc", &with_dep, &mut lexer);
        assert_eq!(first, again, "the same loop must reuse its binding slot");
        assert!(
            f.tp(again).deps_ref().is_some_and(|d| !d.is_empty()),
            "a binding over a view must carry the collection's dep, or its scope exit frees              a store it only borrows (loft#950)"
        );
    }

    /// The other direction, so the adoption cannot be written as "always take the newer
    /// type": a binding that already carries a dep keeps it when a depless type arrives.
    #[test]
    fn a_binding_that_has_a_dep_does_not_lose_it() {
        let mut lexer = Lexer::default();
        let mut f = Function::new("t", "t.loft");
        let with_dep = Type::Reference(7, Deps::frame1(3));
        let bare = Type::Reference(7, Deps::none());

        let v = f.loop_variable("wc", &with_dep, &mut lexer);
        let again = f.loop_variable("wc", &bare, &mut lexer);
        assert_eq!(v, again);
        assert!(
            f.tp(again).deps_ref().is_some_and(|d| !d.is_empty()),
            "a dep already established must not be dropped by a later depless type"
        );
    }

    /// D-own-8 — a value that borrows TWO sources must record BOTH.
    ///
    /// `pick = if c { a } else { b }` aliases whichever arm runs, so the binding's dep list
    /// is the union.  `Function::change_var_type` used to adopt it with
    /// `for on in type_def.depend() { self.depend(var_nr, on); }`, and `depend` REPLACES
    /// the list (`Type::depending` → `with_deps(Deps::frame1(on))`) — so the loop kept only
    /// the LAST dep and the other source was recorded nowhere.
    ///
    /// ⚠ Asserted on the predicate, not through a program, for the reason the two tests
    /// above give and one more of its own: the collapse has no observable symptom.  Probed
    /// with the dropped source going out of scope first, under `LOFT_POISON` and
    /// `LOFT_STRICT_STORES`, the answer is correct — something downstream keeps it alive.
    /// A script-level guard would therefore assert nothing, while the FACT is plainly
    /// wrong, and the fact is what every free-placement decision reads.
    #[test]
    fn a_binding_that_borrows_two_sources_records_both() {
        let mut lexer = Lexer::default();
        let mut f = Function::new("t", "t.loft");
        let one = Type::Vector(Box::new(Type::Float), Deps::frame1(3));
        let v = f.loop_variable("pick", &one, &mut lexer);

        // The join's union: this value borrows BOTH 3 and 4.
        let both = Type::Vector(Box::new(Type::Float), Deps::frame(vec![3, 4]));
        f.change_var_type(v, &both, &crate::data::Data::new(), &mut lexer);

        let deps = f.tp(v).depend();
        assert!(
            deps.contains(&3) && deps.contains(&4),
            "a two-source borrow must keep both deps, got {deps:?} — the lost one is a \
             store nothing records as borrowed"
        );
    }

    /// The complement, so the fix cannot be written as "always widen": an EMPTY incoming
    /// dep list leaves the established deps alone.  The loop this replaced did nothing for
    /// an empty list, and "the types agree, adopt the deps" never meant "and drop what you
    /// had" — clearing here would free a store the binding still views.
    #[test]
    fn an_empty_incoming_dep_list_does_not_clear_the_established_ones() {
        let mut lexer = Lexer::default();
        let mut f = Function::new("t", "t.loft");
        let with_dep = Type::Vector(Box::new(Type::Float), Deps::frame1(3));
        let v = f.loop_variable("held", &with_dep, &mut lexer);

        let bare = Type::Vector(Box::new(Type::Float), Deps::none());
        f.change_var_type(v, &bare, &crate::data::Data::new(), &mut lexer);

        assert!(
            f.tp(v).depend().contains(&3),
            "a depless incoming type must not clear an established dep"
        );
    }
}

/// Does this compiler-internal local OWN the store of the literal it accumulates?
///
/// `__vdb_N` (a vector literal's backing record) and `__kvb_N` (loft#703's keyed twin) are
/// both FUNCTION-scoped for the same reason: the accumulator owns the store it builds, so
/// the function-exit sweep is what frees it.  Every other `__` local either owns nothing or
/// has its own emission path.
///
/// One home because a coroutine asks it TWICE — which locals become struct fields that
/// survive a resume, and which are pre-declared at method scope — and both sites spelled it
/// as a bare `starts_with("__vdb")`.  A keyed literal could not reach a generator at all
/// until loft#1130 gave `yield` its expected type, and the moment it could, the local it
/// mints was declared inside one `match` arm and read from another (rustc E0425).
#[must_use]
pub fn owns_literal_backing_store(name: &str) -> bool {
    name.starts_with("__vdb") || name.starts_with("__kvb")
}
