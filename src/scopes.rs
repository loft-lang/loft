// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Scope analysis and dependency-based freeing.
//!
//! After parsing, every function is walked by [`check`] which:
//! 1. Assigns each variable to a scope (block nesting level).
//! 2. Inserts `OpFreeText` / `OpFreeRef` at scope exits to free owned values.
//! 3. Handles variable shadowing across sibling scopes via `var_mapping`.
//! 4. Calls [`assign_slots`] and [`compute_intervals`] for stack layout.
//!
//! ## Dependency-based freeing
//!
//! Whether a heap value is freed at scope exit depends on the `dep` field
//! on its [`Type`]:
//!
//! - **`dep` empty** → the variable *owns* the value → emit `OpFreeRef`.
//! - **`dep` non-empty** → the variable *borrows* from a parameter → skip free
//!   (the caller owns the store; freeing here would corrupt it).
//!
//! **Text exception:** `OpFreeText` is always emitted for `Type::Text` regardless
//! of deps, because text lives as a `String` on the stack frame — it must be
//! dropped when the frame exits, even if borrowed.  The `Str` slice that was
//! passed as an argument is a view, not an allocation.
//!
//! **Return-value exemption:** the variable holding the function's return value
//! (`ret_var`) is never freed — its value is consumed by the caller.

use crate::data::{Block, Context, Data, DefType, Deps, Type, Value, v_if, v_set};
use crate::variables::{Function, compute_intervals, size};
use std::collections::{BTreeMap, HashMap, HashSet};

struct Scopes<'s> {
    /// The store-type registry — read for the element type a vector COPY op names
    /// (`OpReplaceVector`'s third argument), which only the registry can answer for a
    /// narrow or nested element.
    database: &'s mut crate::database::Stores,
    /// The definition number of the current analyzed function.
    d_nr: u32,
    /// The next scope number that will be created.
    max_scope: u16,
    /// The current scope during traversal of the code. 0 is the scope of the function arguments.
    scope: u16,
    /// The currently open scopes.
    stack: Vec<u16>,
    /// Per encountered variable the scope where it was created. Later copied into the definition.
    var_scope: BTreeMap<u16, u16>,
    /// Insertion order of variables into `var_scope` (excluding scope-0 arguments).
    /// Used by `variables()` to emit `OpFreeRef` in reverse-allocation order so that
    /// `database::free()` LIFO invariant is satisfied.
    var_order: Vec<u16>,
    /// Variables that are redefined after running out-of-scope get copied with this mapping.
    var_mapping: HashMap<u16, u16>,
    /// Plan-57 cluster-I two-phase scan: confined `__vdb`/local var → the block
    /// scope it should register at instead of function scope, so the block-exit
    /// `free_vars` sweep frees the store there.  Empty on phase 1; populated from
    /// `store_confinement` for the gated phase-2 re-scan (`put_scope` consults it).
    confined: HashMap<u16, u16>,
    /// The scopes of the currently traversed loops.
    loops: Vec<u16>,
    /// Recursion depth counter for `scan`; reset to 0 when scope analysis starts.
    scan_depth: usize,
    /// Counter for `__lift_N` temporary variables created to own inline struct
    /// arguments.
    lift_counter: u16,
    /// Variables added by `scan_args` for inline struct-returning call arguments.
    /// These are conditionally assigned inside if-chains / match arms, so the
    /// outer block needs a `Set(v, Null)` at function entry to reserve their
    /// slot in codegen's stack.position — otherwise the function-level
    /// `OpFreeRef(__lift_N)` at function exit reads a slot that was never
    /// allocated along every execution path.
    lift_vars: Vec<u16>,
    /// Text temps minted mid-scan (the block-VALUE `__blk_N` hoists) whose
    /// `String` must be DECLARED at function scope on native — a block-local
    /// `String` behind the block's `Str` value is E0597 ("dropped while
    /// still borrowed").  Each gets a `Set(tmp, Text(""))` prepended at the
    /// function root (the `lift_vars` mechanism, text-typed).
    lift_texts: Vec<u16>,
    /// Counter for `__ret_N` temporaries used by `free_vars` to hold a
    /// non-trivial tail expression's value while free ops run (B5-L3 fix).
    ret_temp_counter: u16,
    /// `__ref_N` work_ref → witness variable whose call-return
    /// value might alias `__ref_N`'s store at runtime.  Populated by
    /// `scan_set` when the work_ref is passed as an arg to a user-fn
    /// call whose Reference result is assigned to the witness.
    /// Consulted by `get_free_vars` to emit `OpFreeRefIfDistinct` (a
    /// runtime store-nr check) instead of the unconditional `OpFreeRef`
    /// — see the comment block around `scan_set`'s witness-pairing branch.
    paired_witness: HashMap<u16, u16>,
    /// loft#1317 — the `__ref_N` an inline record LITERAL minted, mapped to the local it was
    /// then aliased into.  Separate from [`Scopes::paired_witness`] because the free it
    /// governs is conditional on a fact only `get_free_vars` holds: whether that local is
    /// RETURNED.  Where it is not, the buffer's plain free is the store's only release and
    /// has to stay plain.
    literal_buffer: HashMap<u16, u16>,
    /// loft#1257 — a `__lift_N` holding a COLLECTION `??` return whose `Own::Join` base the
    /// oracle can NAME.  The temp may hold the caller's own store (the discharge arm ran) or
    /// one the closure minted, and which of the two is decidable at run time by store IDENTITY
    /// against that base — `ownership.md` D-own-16's route, no witness slot.  Read by
    /// `get_free_vars`, which frees the temp only where it is NOT the caller's store.
    lift_join_witness: HashMap<u16, u16>,
    /// Set by `callref_owned_return` on that path, consumed by the next `new_lift_var`.
    /// `u16::MAX` = none.
    pending_join_witness: std::cell::Cell<u16>,
    /// Variables assigned at MORE THAN ONE site in this function.  The identity route
    /// (`lift_join_witness`) compares a local's store against the variable its dep names at
    /// scope exit; a base reassigned while the local is live could by then name a store that
    /// is already gone, so such a base is not offered as a witness.  Conservative in the safe
    /// direction: declining keeps today's leak, never frees a store twice.
    multi_assigned: HashSet<u16>,
    /// The backing local each CAPTURE named at the closure build — see
    /// [`capture_build_backings`].  Computed once off the raw body, because the answer is
    /// positional (@FR-O-Latest) and the variable table carries only the LAST assignment.
    capture_build_backing: CaptureBuilds,
    /// The loop depth at which each `__lift_N` temp was created.  A temp created INSIDE the
    /// innermost loop that re-runs its Set has its scope exited — and its slot freed — every
    /// iteration, so a transition free there would free twice; one created OUTSIDE that loop
    /// keeps its slot live across iterations and needs one.
    lift_decl_depth: HashMap<u16, usize>,
    /// For every local bound from a fn-ref call anywhere in this function, the set of `Join`
    /// bases those Sets name.  A local with ONE base takes the identity route; one with two
    /// would compare the store one site handed it against the other site's base, and free a
    /// caller's store.  Read off the raw body before the scan, because a conflict found at
    /// the second Set could not retract the free already emitted at the first.
    callref_join_bases: HashMap<u16, HashSet<u16>>,
    /// The witness slot ([`Self::snapshot_witness_for`]) of each collection local whose base
    /// cannot stand witness itself.
    snapshot_witness: HashMap<u16, u16>,
    /// @P378(a) — INVERSE of `paired_witness` for the case where the
    /// witness `v` is INNER-scoped relative to the `__ref_N` buffer
    /// `av` (e.g. `bs = alloc_bag(ci, __ref_1)` inside a `for` loop,
    /// where `bs` lives in the loop body and `__ref_1` is the
    /// function-scoped return buffer).  Here the buffer must stay
    /// reserved across iterations; freeing the witness each iteration
    /// (it adopts the buffer's store) would recycle that store to a
    /// callee temp next iteration and collide (SIGSEGV at the keyed
    /// insert).  Maps witness `v` → buffer `av`; consulted by
    /// `get_free_vars` to emit `OpFreeRefIfDistinct(v, av)` for the
    /// witness's free — a no-op in the adoption case (store stays
    /// reserved, freed once via the buffer's function-exit OpFreeRef),
    /// a real free in the fresh-store case.  Scope-safe for native:
    /// `av` (outer/function) outlives `v` (inner), so `av`'s Rust
    /// `let` is still live where `v`'s free fires.
    ///
    /// One witness can adopt SEVERAL buffers — the arms of a value branch each mint one
    /// (`y = if c { S { … } } else { S { … } }`) and the witness holds whichever arm ran —
    /// so the free declines against every one of them.
    witness_buffer: HashMap<u16, Vec<u16>>,
    /// Reference vars whose LATEST scanned assignment gave them an OWNED store (a call
    /// whose filtered return deps are empty, a deep-copied var, …), mapped to the loop
    /// depth (`loops.len()`) at that assignment.
    ///
    /// Enforces @FR-O-Latest — a memo of @FR-O-Oracle's answer plus the loop depth at
    /// which the assignment was taken.
    ///
    /// ⚠ Not redundant with `deps` or with @FR-O-Override: it carries a TEMPORAL fact (the
    /// *latest* assignment) and a LOOP-DEPTH fact, neither of which a type-level dep list
    /// can express.  `scan_set`'s ownership-TRANSITION free is gated on this and not on
    /// `deps` for exactly that reason — freeing at the wrong loop depth would release the
    /// previous iteration's viewed store.
    /// When such a var is reassigned with a BORROW, its merged static type
    /// already carries deps, so codegen's dep-empty pre-Set free never fires —
    /// `scan_set` emits an explicit `OpFreeRef(v)` for the orphaned store
    /// instead (only at the same loop depth: emitting inside a deeper loop
    /// would re-free a viewed store on iterations 2+).
    owned_refs: HashMap<u16, usize>,
    /// loft#1128 — the RUNTIME ownership witness for the hidden return-buffer parameter:
    /// `(buffer var, boolean flag var)`.  `owned_refs` above is the same fact answered
    /// STATICALLY, and it is intersect-merged at every join (@FR-O-Complete), so a prior
    /// assignment inside one `if` arm correctly answers *"not owned on every path"* and no
    /// free is emitted — sound, and incomplete.  The flag mirrors the same fact per RUN, the
    /// way `--native` already does with its entry-buffer witness `_rb_w_<name>`.
    ///
    /// `None` unless the body actually reaches a displacing site (`displaces_return_buffer`),
    /// so a function that cannot leak pays no slot.
    rbuf_witness: Option<(u16, u16)>,
    /// loft#1200 — the per-LOCAL ownership witness: a nullable heap-record local that is
    /// reassigned from a minting call, mapped to the boolean that records whether the store
    /// it currently holds is this frame's SOLE property.  The static answer is not available
    /// (see `nullable_locals_that_displace`), so the displaced-store free reads this instead.
    local_owns: HashMap<u16, u16>,
    /// loft#1336 / @FR-O-Witness — a heap-record LOCAL whose assignments MIX ownership (one
    /// hands it a store of its own, another a view) → its OWNER WITNESS `__own_<name>`, the
    /// hidden reference that names the store the local minted for as long as the local
    /// still holds it.  Every release of such a local's stores goes through the witness —
    /// at the `Set` that makes the local stop naming it, or at scope exit — and the local
    /// itself is never freed.  Keyed on every id the local is known by (the original and
    /// any scope copy `scan_set` makes of it).  [`owner_witness_locals`] picks them.
    owner_witness: HashMap<u16, u16>,
    /// @PLN85 `local_source` over-free fix (gated by `LOFT_JOIN_OWN`): heap slots
    /// that hold an OWNED store displaced by a later `Borrowed`/`Join` reassignment
    /// (`use_analysis::displaced_owned_slots`). For these, `scan_set` strips the
    /// declared deps so the OWNED path deep-copies + frees the slot — otherwise the
    /// displaced owned store is orphaned and leaks. Empty when the flag is off.
    displaced_owned: HashSet<u16>,
    /// @PLN130 F2/F8 — view bindings live across a disturbance of their container, and which
    /// disturbance it was.  See [`collect_views_to_materialise`].
    views_to_materialise: HashMap<u16, Disturbance>,
    /// loft#721 — fn-ref variable -> the definition it was assigned, or
    /// `u32::MAX` when more than one definition reaches it.  A `CallRef`'s callee
    /// is a runtime value, so this local fact is what lets the lift ask the
    /// callee's own `returns_borrowed_view()` instead of guessing from the type.
    fnref_target: HashMap<u16, u32>,
    /// loft#849 / @PLN139 — vars that no longer OWN what they hold, so their scope end must
    /// not drop it.  See [`collect_drop_transferred`].
    drop_transferred: HashSet<u16>,
    /// loft#890 — the lifted temps whose STORE a consuming op already freed, so
    /// `get_free_vars` must not free it again.  Scope-local on purpose: `skip_free` is a
    /// VARIABLE flag both backends read at ALLOCATION time too, so stamping it here made
    /// the lift borrow instead of own and the append wrote into its own source.
    free_transferred: HashSet<u16>,
    /// loft#854 — the whole-function half of the ownership oracle, computed once
    /// for `d_nr` instead of once per question.
    ///
    /// `ownership_of` walks the entire function body (and clones each defining
    /// right-hand side) to answer about ONE value. `scan_set` asks about every
    /// assignment, so a function with n of them paid n whole-function walks: a
    /// vector literal is one `Set` per element, and 86 400 elements took over 13
    /// minutes at 99 % CPU.
    ///
    /// Safe to memo for exactly as long as this `Scopes` lives: the body it
    /// summarises is `data.def(d_nr).code`, `data` is borrowed `&Data` for the
    /// whole traversal, and `run_scan_phase` installs the rewritten body only
    /// after the scan returns — so the borrow checker, not a convention, is what
    /// keeps this from going stale. A `Scopes` is built per scan phase, so a
    /// second phase re-derives it.
    fn_defs: Option<crate::use_analysis::Defs>,
}

/// Perform scope analysis on all currently known functions.
/// One scan pass for [`check`]: scan `orig_code`/`orig_vars`, prepend the
/// lift-var null-inits, apply the result to `def`, run the debug ref/leak checks,
/// and set each variable's scope.  Runs once normally; plan-57 cluster I re-runs
/// it with a non-empty `confined` map (`__vdb`/local → block scope) so a confined
/// store registers — and therefore frees — at its block exit (`put_scope`).
/// loft#721 — map each fn-ref VARIABLE to the definition assigned to it.
///
/// A non-capturing lambda is stored as a bare definition number, a capturing one
/// as `FnRef(d_nr, closure)`; both forms are searched.  A variable that receives
/// two different definitions maps to `u32::MAX` (ambiguous), and a caller that
/// cannot name one definition must not lift — see `callref_owned_return`.
/// @PLN130 F2 — the container variable an element/field read ultimately reads OUT of.
///
/// One line, because the derivation is [`crate::use_analysis::projection_container_var`]'s:
/// the EMITTER that copies a materialised view reads the same home
/// (`generation::container_element_base`), and a walk that named a container the emitter
/// cannot is a copy the compiler reports and does not make.
fn base_container_var(value: &Value, data: &Data) -> Option<u16> {
    crate::use_analysis::projection_container_var(data, value)
}

/// The PLACE an element/field read ultimately reads out of — the one home is
/// [`crate::use_analysis::projection_container_place`], which carries both the variant-check
/// peel a struct-enum payload projection needs and the field OFFSET that tells `w.a` from
/// `w.b`.  This wrapper exists so the view walk reads it by the name the walk talks about.
fn base_container_place(value: &Value, data: &Data) -> Option<(u16, u32)> {
    crate::use_analysis::projection_container_place(data, value)
}

/// The offset of a place that is a whole VARIABLE rather than one of its fields — and the
/// wildcard on both sides of a match, since disturbing a variable ends every place in it.
use crate::use_analysis::ANY_FIELD;

/// Do a view's place and a disturbance's place name the same storage?  Equal offsets, or
/// either side naming the whole variable.
fn same_place(view: (u16, u32), disturbed: (u16, u32)) -> bool {
    view.0 == disturbed.0
        && (view.1 == disturbed.1 || view.1 == ANY_FIELD || disturbed.1 == ANY_FIELD)
}

/// The PLACE the value of a `Set` VIEWS — the container variable and the field offset inside
/// it — for a plain projection ([`base_container_place`]), through a BRANCH to the place its
/// arms project from, and through a value BLOCK to the place its tail names.
///
/// `x = if k > 0 { h.inner } else { mk(0) }` is a view of `h` on the arm that projects and a
/// fresh value on the other, and asked only of the whole `If` it named no container at all — so
/// a later `h = …` disturbed nothing, and the binding kept reading the container it no longer
/// belongs to (measured on both backends, on a struct tail and a collection one).
///
/// Arms that MINT are ignored rather than disqualifying: one arm viewing is enough for the
/// binding to be a view on some run, and this is a per-binding fact.  Two arms viewing
/// DIFFERENT containers name none — there is no single place to be disturbed.  Two arms viewing
/// different FIELDS of one container name the whole variable, since either place can be the
/// one a disturbance ends.
///
/// **A block's tail may NAME a temp the block itself bound**, and that name views nothing on
/// its own.  A `??` discharge is the shape that matters: it hoists a non-trivial subject into a
/// temp and its tail `if` hands that temp back on the present path, so `c = v[1] ?? Box{n:0}`
/// reached this as an `if` whose arms are a bare `Var` and a fresh mint — two names, neither a
/// projection, so no container at all.  The binding stayed a live alias of position 1 across a
/// `remove` that renumbered it, reading another element's value and writing back into the
/// container, where the plain spelling materialises and says so (loft#1401, both backends).
/// Resolving a tail name through the block's OWN bindings covers `??`, `?? return` and a
/// `match` subject in one step, because it matches the notion — a name standing for a value
/// computed here — rather than any one lowering's spelling of it.
///
/// Only a binding the block MAKES is resolved, never a bare variable the block merely mentions:
/// a discharge whose subject is already a variable lowers to a plain `if` with a `Var` arm and
/// no hoist, and reading that as a projection base is the misreading
/// [`crate::use_analysis::variant_check_subject`] documents at length.
///
/// ⚠ Read ONLY by the walk that NAMES the views to materialise, never by the deps strip.  The
/// strip makes a binding an owner, and for a branch- or block-valued right-hand side the
/// emitters have no copy to pair with that — `container_element_base` answers `None` for an
/// `If` — so a binding stripped here would own a store it only views and free the CONTAINER's
/// at scope exit (loft#778's class, measured; and measured again for the discharge block, where
/// the advice then asserts a guarantee the emitters do not deliver).  What supplies the copy
/// instead is per ARM: [`Scopes::arm_bind`] gives a projecting arm — and a discharge hoist the
/// arm hands back — its own temp once this has named the binding, which is `(O-Complete)`'s
/// per-path fact rather than one verdict for the whole `Set`.
///
/// Deliberately NOT folded into [`crate::use_analysis::projection_container_place`], which the
/// ownership oracle and both emitters read: peeling an arbitrary `if` there would claim `a?` on
/// a nullable parameter, whose lowering is an `if` with a `Var` arm, and answer `Borrowed` for
/// a value the callee minted.
fn value_view_places(value: &Value, data: &Data, function: &Function) -> Vec<(u16, u32)> {
    let mut out = Vec::new();
    view_place_in(value, data, function, &[], 0, &mut out);
    out
}

/// [`value_view_place`] with the bindings a surrounding value block made in scope, and a depth
/// bound so a self-referential binding (`Set(x, Var(x))`) cannot walk forever.
///
/// The fallback is [`crate::use_analysis::view_source_place`] — [`base_container_place`]
/// counting a NULLABLE element read as the projection it is, which is what a discharged `v[i]`
/// arrives as.  Its own `None` means *"this value is not read out of a place a disturbance can
/// name"* — a literal, a mint, a call.  That is the safe answer here in both directions: an
/// unnamed value is not materialised, so a shape this cannot read keeps the aliasing it has
/// today rather than gaining a copy nothing asked for.
fn view_place_in<'a>(
    value: &'a Value,
    data: &Data,
    function: &Function,
    env: &[(u16, &'a Value)],
    depth: u32,
    out: &mut Vec<(u16, u32)>,
) {
    // Bounded on both axes: the depth stops a self-referential binding (`Set(x, Var(x))`) from
    // walking forever, and the width stops a deeply nested branch from making the open-view
    // frame grow with the number of arms rather than with the number of bindings.
    if depth > 16 || out.len() >= 8 {
        return;
    }
    match value.unspan() {
        // BOTH arms, not their intersection.  A binding whose arms project from DIFFERENT
        // containers is a view of each on the path that takes it, and either being disturbed
        // ends it — asked for one answer this arm said `None`, so `c = if k { w[0] } else
        // { v[1] } ; v.remove(0)` kept reading the container it no longer belongs to, on both
        // backends and in silence (loft#1401's matrix).  A view is recorded once per place it
        // can name, which the open-view frame already holds as one entry per pair, so nothing
        // downstream had to learn a new shape.
        //
        // Two arms naming the same container at DIFFERENT fields stay two places rather than
        // collapsing to `ANY_FIELD`: `(B-Disturb)` ends a place, and a disturbance of a third
        // field of that container ends neither of them.
        Value::If(_, t, e) => {
            view_place_in(t, data, function, env, depth + 1, out);
            view_place_in(e, data, function, env, depth + 1, out);
        }
        Value::Block(b) => block_tail_place(&b.operators, data, function, env, depth, out),
        Value::Insert(ops) => block_tail_place(ops, data, function, env, depth, out),
        Value::Var(x) => {
            if let Some((_, bound)) = env.iter().rev().find(|(v, _)| v == x) {
                view_place_in(bound, data, function, env, depth + 1, out);
            }
        }
        // A place inside a COMPILER-GENERATED container is not a place any disturbance can
        // name, so it is a MINT for this question rather than a view.  A vector literal is
        // the shape that matters: it lowers to a hidden `__vdb_N` backing local and reads its
        // own store back out of it (`vv = OpGetField(__vdb_2, 0, …)`), which is a projection
        // by every structural test.  Counted as a view it made the `[]` arm of
        // `b = if … { d.tiles.proto } else { [] }` name a SECOND container, and two arms
        // naming different containers name none — so the whole binding stopped being a view
        // and loft#1399 came back.  `(B-Disturb)` is about places the author can disturb, and
        // nothing in the program can reassign a `__vdb_N`; `Self::resolve_view_root` stops at
        // one for the same reason and says so.
        other => {
            if let Some(place) = crate::use_analysis::view_source_place(data, other)
                .filter(|(c, _)| !function.is_compiler_generated(*c))
                && !out.contains(&place)
            {
                out.push(place);
            }
        }
    }
}

/// The place a value block's TAIL views, with the block's own `Set`s added to `env` so a tail
/// that names one of them resolves to the value it was bound to.
fn block_tail_place<'a>(
    ops: &'a [Value],
    data: &Data,
    function: &Function,
    env: &[(u16, &'a Value)],
    depth: u32,
    out: &mut Vec<(u16, u32)>,
) {
    let Some(tail) = ops
        .iter()
        .rev()
        .find(|o| !matches!(o.unspan(), Value::Line(_)))
    else {
        return;
    };
    let mut inner: Vec<(u16, &'a Value)> = env.to_vec();
    for op in ops {
        if let Value::Set(v, val) = op.unspan() {
            inner.push((*v, val.as_ref()));
        }
    }
    view_place_in(tail, data, function, &inner, depth + 1, out);
}

/// Tell the author that a view was COPIED out of its container, in the sentence its cause
/// earns.
///
/// One home for the three sentences, because the copy itself has two mechanisms and the report
/// must not differ between them: the deps STRIP materialises a plain projection bind, and the
/// per-ARM lift materialises a branch- or discharge-valued one.  A reader cannot tell which
/// route their binding took, and `(H-Materialise)`'s promise — "the author is told" — is about
/// the copy, not about how it was arranged.
fn report_materialised_view(cause: ViewCause, vname: &str, cname: &str, fname: &str) {
    match cause {
        ViewCause::Reshaped => {
            crate::copy_manifest::note_materialised_view(vname, cname, fname);
        }
        // loft#1373 — the fourth invalidator: the container GREW, so the elements may
        // have moved to a larger record. Same materialise, different sentence: a
        // reader told "removing an element renumbers the others" goes looking for a
        // `remove` that is not in the function.
        ViewCause::Grown => {
            crate::copy_manifest::note_grown_view(vname, cname, fname);
        }
        // @PLN130 F8 — the third invalidator: the container VARIABLE is reassigned,
        // so the dep still names `bx` while the store it named is gone. Different
        // cause, different way out, so a distinct advice line.
        ViewCause::Reassigned => {
            crate::copy_manifest::note_reassigned_view(vname, cname, fname);
        }
    }
}

/// Is `v` the temp a null DISCHARGE hoists its SUBJECT into?
///
/// `e ?? d` and `e ?? return` bind a non-trivial subject to a temp of their own and hand that
/// temp back from the tail `if`'s present arm, so it is the subject's value wearing a compiler
/// name.  Two questions read it — what the join BORROWS on that path
/// ([`Scopes::lift_arm_tails_into`]), and whether that arm needs a store of its own
/// ([`Scopes::arm_bind`]) — and one home keeps them from drifting apart.
fn is_discharge_hoist(function: &Function, v: u16) -> bool {
    let name = function.name(v);
    name.starts_with("__ncc_") || name.starts_with("_ncr_")
}

/// What an argument LIFTED into a `__lift_N` temp borrows — the deps its type must carry.
///
/// A lift temp holds a value the caller reads out of something it does not own: an element
/// of a container, a field of a record, the surviving arm of a `??`.  Typed without deps it
/// reads as the OWNER of that store and `get_free_vars` emits a scope-exit `OpFreeRef` for
/// it — releasing a container the caller still names.  Where the container is a local of the
/// same frame the bogus free lands on a store that was dying anyway and nothing reports it;
/// where it OUTLIVES the frame (a parameter, a global) the next allocation recycles the
/// record and the container reads back as another type's bytes.
///
/// So the temp borrows what the value borrows.  The walk bottoms out at:
/// * a plain `Var` — the chain reads out of that local's store;
/// * a `TupleGet` — out of the tuple's;
/// * a BLOCK, whose `result` type already carries the deps the parser derived for it (a
///   `??` lowers to one, and its type names the container the surviving arm reads);
/// * an `If`, where either arm can be the value, so the deps are the union.
///
/// `None` where none of those is reached — a value whose source cannot be named must not be
/// bound at all.  The caller then leaves the argument as it was, which costs the leak that
/// is already there; a temp typed as an owner of somebody else's store costs a
/// use-after-free, and a leak is the better of those two.
fn lift_view_deps(arg: &Value, data: &Data) -> Option<Vec<u16>> {
    match arg.unspan() {
        Value::Var(v) => Some(vec![*v]),
        Value::TupleGet(base, _) => Some(vec![*base]),
        Value::Block(bl) => {
            let d = bl.result.depend();
            if d.is_empty() { None } else { Some(d) }
        }
        Value::If(_, then_v, else_v) => {
            let mut d = lift_view_deps(then_v, data)?;
            for x in lift_view_deps(else_v, data)? {
                if !d.contains(&x) {
                    d.push(x);
                }
            }
            Some(d)
        }
        Value::Call(d_nr, cargs) if crate::use_analysis::is_projection_op(data, *d_nr) => {
            lift_view_deps(cargs.first()?, data)
        }
        _ => None,
    }
}

/// Every container variable `code` RESHAPES — removes from, or grows.
///
/// Detects two of @FR-B-Disturb's four place-ending events: REMOVING from a container and
/// GROWING one.  (The other two are RE-KEYING an element and REASSIGNING the container
/// itself, which [`established_stores`] answers.  OVERWRITING a place does not disturb it —
/// the write lands in the place the view already points at, and `v[i] = x` lowers to
/// `OpCopyRecord`, which is named nowhere here.)
///
/// `v.remove(i)` lowers to `OpRemoveVector(v, size, index)` and the in-loop `e#remove` to
/// `OpRemove(index, container, …)` — the one op naming its container at arg 1, where every
/// other names it at arg 0.  Both renumber the positions inside the container's store, which
/// is what invalidates an element view.
///
/// GROWING is the fourth event, and the one this list was short by (loft#1373).  A vector
/// that outgrows its allocation is copied into a larger record — `Store::resize` answers a
/// NEW record number when it cannot absorb the free block beside it, `vector_append` repoints
/// the container's handle at it, and the old record is freed — so every element moves and a
/// view bound before the growth names an address the elements have left.  Measured: `d: S =
/// v[0]` followed by two hundred appends read `4294967296` on both backends with strict
/// stores silent, while the same code with TWO appends read `1`, because nothing had
/// reallocated yet.  One shape, two answers, decided by an allocation the author cannot see —
/// which is why ANY growth disturbs rather than only one that provably crosses the capacity.
///
/// The five growth spellings, each naming its container at arg 0: `OpNewRecord` (a
/// single-element append and a keyed add — it calls `vector_append`), `OpPreAllocVector` (the
/// capacity request a literal and a sized append make), `OpAppendVector` (`v += w`),
/// `OpInsertVector` (an insert, which also shifts every later element) and `OpHashAdd` (which
/// can rehash and move records).  A container's own CONSTRUCTION uses these too and needs no
/// separate test: the walk runs in ORDER and only shakes views that are already open, so an
/// op running before any view of that container exists reaches nothing.
///
/// Only a container named by a plain `Var` is collected; a reshape reached through some
/// other expression is not recognised, so the answer is a lower bound and a missed case
/// keeps today's behaviour rather than inventing a new one.
/// **Deliberately NOT extended across a call.** A callee that removes from a `&vector`
/// parameter reshapes the CALLER's container, and a view the caller holds then goes stale with
/// no diagnostic — measured, the write is silently lost (@PLN130 probe 38 cell A1, both
/// backends, and it reproduces on mainline). Propagating the reshape to the call site was tried
/// and reverted: it did not fix A1 (the view's dep does not name a `&vector` PARAMETER, so the
/// materialise arm never fires) and it broke cell C1, where the viewed element does not move
/// and the write legitimately lands. It would also make a `&` argument silently become a copy
/// whenever the callee removes from the container, which changes what `&` means (@PLN87) rather
/// than fixing a bug. Filed as [loft#779](https://github.com/loft-lang/loft/issues/779); the
/// decided answer is to REFUSE that program, not to copy behind the author's back.
fn reshaped_containers(code: &Value, data: &Data, function: &Function) -> HashSet<(u16, u32)> {
    let mut out = places_named_by(code, data, &|name| match name {
        "OpRemove" => Some(1),
        "OpRemoveVector" => Some(0),
        _ => None,
    });
    // @FR-Col-RemoveDense — a KEYED removal reaches all five keyed kinds through ONE op, and
    // only one of them renumbers, so this has to be keyed on the KIND and not on the op.
    //
    // `@FR-Col-RemoveKeyed`: `hash`, `index`, `spatial` and `trie` give each element a record
    // of its own, so removing one leaves every other key reachable AT THE SAME ADDRESS —
    // measured, a view of another element reads correctly after the removal and a write
    // through it still lands, which is why collecting them here would materialise a binding
    // whose write is fine today.  A `sorted` is the INLINE keyed kind: its elements sit in key
    // order in one dense array, so a removal shifts every later position exactly as a
    // vector's does, and `@FR-Col-RemoveDense` names the two by-value kinds together.
    //
    // The same split `Stores::remove_vector_at`'s `is_linked` gate makes for the LEAK half of
    // this rule (loft#1402): one `sorted` leaked through `#remove` and not through
    // `[key] = null`, and here it goes stale through `[key] = null` where `hash` does not.
    // Two symptoms, one boundary.
    //
    // Read off the container VARIABLE's type, so a `sorted` reached through a FIELD is not
    // collected — a lower bound, kept because the projection carries its element type rather
    // than its collection kind and guessing there would shake the dense kinds' siblings.
    for place in places_named_by(code, data, &|name| match name {
        "OpHashRemove" => Some(0),
        _ => None,
    }) {
        if place.1 == ANY_FIELD && matches!(function.tp(place.0).base(), Type::Sorted(_, _, _)) {
            out.insert(place);
        }
    }
    out
}

/// Every container variable `code` GROWS — @FR-B-Disturb's fourth place-ending event.
///
/// Apart from [`reshaped_containers`] because the ADVICE differs, and only because of that:
/// a removal renumbers the elements after the one removed, while a growth can move ALL of
/// them to a larger record, so a reader told the wrong one goes looking for a `remove` that
/// is not there.  Both shake the same views for the same reason.
///
/// The five spellings, each naming its container at arg 0: `OpNewRecord` (a single-element
/// append and a keyed add — it calls `vector_append`), `OpPreAllocVector` (the capacity
/// request a literal and a sized append make), `OpAppendVector` (`v += w`), `OpInsertVector`
/// (an insert, which also shifts every later element) and `OpHashAdd` (which can rehash and
/// move records).
fn grown_containers(
    code: &Value,
    data: &Data,
    function: &Function,
    database: Option<&crate::database::Stores>,
    cleared: &HashSet<(u16, u32)>,
) -> HashSet<(u16, u32)> {
    let mut out: HashSet<(u16, u32)> = HashSet::new();
    code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let name = data.def(*d).name();
        if !matches!(
            name,
            "OpNewRecord" | "OpPreAllocVector" | "OpAppendVector" | "OpInsertVector" | "OpHashAdd"
        ) {
            return;
        }
        // `OpNewRecord(parent, tp, fld)` names its container in TWO parts, and only the
        // whole-variable form belongs here: `fld == u16::MAX` is an append to the variable
        // itself (`v += [x]` emits `OpNewRecord(v, tp, 65535)`), while any other `fld` is an
        // append to that variable's FIELD (`s.us_redo += [x]` emits `OpNewRecord(s, tp, 1)`).
        //
        // Collecting the parent for the second shape shakes every view rooted at the same
        // variable whichever field it names.  Measured on `moros_editor`'s `undo_pop`:
        // `e = s.us_entries[idx]` was materialised because the SIBLING field `s.us_redo` grew,
        // so each undo entry was read out of a copy, the stack silently stopped recording, and
        // `undo_depth` answered 0 where 3 was due.  A field-qualified growth is left
        // UNCOLLECTED rather than compared field-wise, which keeps this function the lower
        // bound its sibling's doc claims: a missed disturbance costs a materialise, a spurious
        // one costs a program its meaning.
        let Some(Value::Var(c)) = args.first().map(Value::unspan) else {
            return;
        };
        // `OpNewRecord(parent, tp, fld)` names its container in TWO parts: `fld == u16::MAX`
        // is an append to the variable itself (`v += [x]`), any other `fld` an append to that
        // variable's FIELD (`w.a += [x]`).  The field is a NUMBER and a view carries a byte
        // OFFSET, so the two are converted here — `Stores::field_position` against the
        // parent's own struct type — and the place is `(w, offset)`.
        //
        // Without the store the conversion cannot be made, and the field-qualified growth is
        // left UNCOLLECTED rather than widened to the parent: reading the parent alone shakes
        // every view rooted at it whichever field it names, which silently emptied
        // `moros_editor`'s undo stack when loft#1373 first shipped.  A missed disturbance
        // costs a materialise; a spurious one costs a program its meaning.
        let mut place = ANY_FIELD;
        if name == "OpNewRecord" {
            let fld = match args.get(2).map(Value::unspan) {
                Some(Value::Int(f)) => *f,
                _ => return,
            };
            if fld != i32::from(u16::MAX) {
                let Some(db) = database else { return };
                let parent = data.type_def_nr(function.tp(*c).base());
                if parent == u32::MAX {
                    return;
                }
                let off = db.field_position(data.def(parent).known_type(), fld as u16);
                if off == u16::MAX {
                    return;
                }
                place = u32::from(off);
                if cleared.contains(&(*c, place)) {
                    return;
                }
            }
        }
        out.insert((*c, place));
    });
    out
}

/// The PLACES `code` disturbs at the argument `which` picks, for the ops it picks.
///
/// A whole VARIABLE ends every place inside it (`ANY_FIELD`); a FIELD of one ends the places
/// inside THAT field only.  The distinction is the whole reason this answers places rather
/// than variables: `p.va.remove(0)` and `p.vb.remove(0)` both name `p`, and treating either as
/// "everything in `p`" materialises a view whose write lands today.  `grown_containers` records
/// the measurement — collecting the PARENT for a field-qualified growth shook every view rooted
/// at the same variable, and `moros_editor`'s undo stack silently stopped recording.
///
/// Before loft#1401's matrix this collected ONLY a plain `Var`, so a removal reached through a
/// field was not a disturbance at all: `c = p.va[1]; p.va.remove(0)` read the element that
/// shifted in, on both backends and in silence, where the same code with `va` in a local
/// materialises and says so.  The VIEW side already named the place `(p, off_va)` —
/// [`value_view_place`] resolves a projection chain to its outermost field — so only this half
/// was short and the two never met.
///
/// A container reached through anything else — a call result, an element of an element — is
/// still uncollected, and that stays the lower bound it always was: a missed disturbance costs
/// a materialise, a spurious one costs a program its meaning.
fn places_named_by(
    code: &Value,
    data: &Data,
    which: &dyn Fn(&str) -> Option<usize>,
) -> HashSet<(u16, u32)> {
    let mut out: HashSet<(u16, u32)> = HashSet::new();
    code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let Some(at) = which(data.def(*d).name()) else {
            return;
        };
        let Some(arg) = args.get(at) else { return };
        match arg.unspan() {
            Value::Var(c) => {
                out.insert((*c, ANY_FIELD));
            }
            other => {
                if let Some(place) = base_container_place(other, data) {
                    out.insert(place);
                }
            }
        }
    });
    out
}

/// @PLN130 F8 — which `&` parameters of `d_nr` are REASSIGNED wholesale by its body.
///
/// A `&` param's slot is a double indirection into the caller's variable, and @PLN87 P2.2
/// lowers `p = T{…}` on one to *"build a fresh store, write it through, free the store the
/// caller was holding"*. So a callee doing that destroys a store the CALLER may have views
/// into — measured on `--native` as a read of `703`, a `Wide` field belonging to an unrelated
/// later allocation. Nothing at the call site says so, which is why the caller needs this
/// fact about the callee rather than a guess from the argument's shape.
///
/// Answered from the callee's IR: a `Set` targeting an argument slot whose declared type is
/// `RefVar`. Argument slots lead the variable numbering, so the slot number indexes the
/// attribute list. A shape this does not recognise yields no fact and keeps today's
/// behaviour — the same lower-bound stance as [`collect_reshaped_containers`].
fn reassigned_ref_params(data: &Data, d_nr: u32) -> HashSet<u16> {
    let def = data.def(d_nr);
    let mut out: HashSet<u16> = HashSet::new();
    def.code.walk(&mut |v| {
        let Value::Set(slot, rhs) = v else { return };
        if matches!(rhs.unspan(), Value::Null) {
            return;
        }
        if let Some(a) = def.attributes.get(usize::from(*slot))
            && matches!(a.typedef, Type::RefVar(_))
        {
            out.insert(*slot);
        }
    });
    out
}

/// See through the `OpCreateStack` wrapper an argument passed to a `&` parameter carries.
///
/// A bare `Var` is returned unchanged, so a lowering change that stops wrapping cannot
/// silently lose the fact this is asked for.
fn peel_stack_ref<'a>(arg: &'a Value, data: &Data) -> &'a Value {
    let inner = arg.unspan();
    if let Value::Call(cs, cargs) = inner
        && data.def(*cs).name() == "OpCreateStack"
        && let Some(first) = cargs.first()
    {
        return first.unspan();
    }
    inner
}

/// @PLN130 F9 — which `&` parameters of `d_nr` its body REMOVES from.
///
/// The mirror of [`reassigned_ref_params`], and needed for the same reason: a `&` parameter
/// is a double indirection into the CALLER's variable, so `all.remove(0)` in the callee
/// renumbers the caller's container. Nothing at the call site says so, which is why the
/// caller needs this fact about the callee rather than a guess from the argument's shape
/// ([loft#779](https://github.com/loft-lang/loft/issues/779)).
///
/// Answered from the callee's IR: an `OpRemoveVector` / `OpRemove` whose container argument
/// is an argument slot declared `RefVar`. Argument slots lead the variable numbering, so the
/// slot number indexes the attribute list. This is the DIRECT answer only; a removal further
/// down reaches the caller through [`removed_params_map`], which closes it over the call graph.
fn removed_ref_params(data: &Data, d_nr: u32) -> HashSet<u16> {
    let def = data.def(d_nr);
    let mut out: HashSet<u16> = HashSet::new();
    if def.attributes.is_empty() {
        return out;
    }
    def.code.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let arg = match data.def(*d).name() {
            "OpRemoveVector" => args.first(),
            "OpRemove" => args.get(1),
            _ => return,
        };
        if let Some(Value::Var(slot)) = arg.map(Value::unspan)
            && let Some(a) = def.attributes.get(usize::from(*slot))
            && matches!(a.typedef, Type::RefVar(_))
        {
            out.insert(*slot);
        }
    });
    out
}

/// @PLN130 F9 — every container variable `stmt` reshapes THROUGH A CALL: passed as a `&`
/// argument to a callee that removes from that parameter.
///
/// Deliberately separate from [`reshaped_containers`] rather than folded into it, because the
/// two answers feed different decisions and folding them was measured to break a cell. F2's
/// materialise is conservative — *any* reshape copies the view — so counting a callee's removal
/// there would silently COPY a plain view whose element never moves and whose write
/// legitimately lands, which is the regression the earlier attempt at this fix measured
/// (probe 38 cell C1). The REFUSAL wants the wider answer, because a rejected program is not
/// silently anything; the materialise wants the narrower one.
fn reshaped_via_call(stmt: &Value, data: &Data, removed: &RemovedParams) -> HashMap<u16, u32> {
    let mut out: HashMap<u16, u32> = HashMap::new();
    stmt.walk(&mut |v| {
        let Value::Call(d, args) = v else { return };
        let Some(params) = removed.get(d) else { return };
        for k in params {
            if let Some(Value::Var(c)) = args.get(usize::from(*k)).map(|a| peel_stack_ref(a, data))
            {
                out.insert(*c, *d);
            }
        }
    });
    out
}

/// Every user definition that removes from at least one of its `&` parameters, and which.
///
/// Built once per program rather than re-derived at each call site: the question is asked once
/// per CALL, and a callee body would otherwise be re-walked once per call to it.
type RemovedParams = HashMap<u32, HashSet<u16>>;

/// [`RemovedParams`], CLOSED OVER THE CALL GRAPH: a function that forwards its own `&`
/// parameter to something that removes from it removes from it too.
///
/// Without the closure the answer is one frame deep, and the hole is one an author would trip
/// over by refactoring: extracting `all.remove(0)` into a helper makes the refusal disappear and
/// the silent lost write come back (probe 40 cell X7). Closed with a worklist over
/// *"caller `c` passes its own `&` parameter `s` as callee `e`'s parameter `k`"* edges, built in
/// the same pass as the direct removals, so the cost stays one walk of each body plus the
/// propagation.
///
/// A callee reached only through a runtime fn-ref has no edge here and keeps today's behaviour —
/// a lower bound in the safe direction, since the refusal simply does not fire.
fn removed_params_map(data: &Data) -> RemovedParams {
    let mut out = RemovedParams::new();
    let mut work: Vec<(u32, u16)> = Vec::new();
    // (callee, its param) -> every (caller, caller's own `&` param) that feeds it.
    let mut forwards: HashMap<(u32, u16), Vec<(u32, u16)>> = HashMap::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if !def.name.starts_with("n_") {
            continue;
        }
        for k in removed_ref_params(data, d_nr) {
            if out.entry(d_nr).or_default().insert(k) {
                work.push((d_nr, k));
            }
        }
        if def.attributes.is_empty() {
            continue;
        }
        def.code.walk(&mut |v| {
            let Value::Call(callee, args) = v else { return };
            if !data.def(*callee).name.starts_with("n_") {
                return;
            }
            for (i, arg) in args.iter().enumerate() {
                let Ok(i) = u16::try_from(i) else { continue };
                // Only the caller's OWN `&` parameter forwards a reshape upwards; a local
                // container passed down is reshaped inside this frame, not the caller's.
                if let Value::Var(slot) = peel_stack_ref(arg, data)
                    && def
                        .attributes
                        .get(usize::from(*slot))
                        .is_some_and(|a| matches!(a.typedef, Type::RefVar(_)))
                {
                    forwards
                        .entry((*callee, i))
                        .or_default()
                        .push((d_nr, *slot));
                }
            }
        });
    }
    while let Some(key) = work.pop() {
        let Some(ups) = forwards.get(&key) else {
            continue;
        };
        for (caller, slot) in ups {
            if out.entry(*caller).or_default().insert(*slot) {
                work.push((*caller, *slot));
            }
        }
    }
    out
}

/// @PLN130 F2/F8 — why a view had to give up its alias.
///
/// The two causes read differently to an author and have different remedies, so the walk
/// carries which one fired. A view reached by both reports the reshape: that is the cause
/// with something to act on at the container.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViewCause {
    /// The container is RESHAPED — `v.remove(i)` / `e#remove` renumbers its positions (F2).
    Reshaped,
    /// The container GROWS — an append, an insert or a keyed add can move every element to a
    /// larger record (@FR-B-Disturb's fourth event, loft#1373).
    Grown,
    /// The container VARIABLE is re-established, so the name stops meaning the store (F8).
    Reassigned,
}

/// One disturbance of a container, as the walk saw it.
///
/// The cause decides the advice line; `line` and `via` exist only so the F9 refusal can point
/// its caret at the statement responsible instead of at the whole function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Disturbance {
    cause: ViewCause,
    /// Source line of the statement that disturbed the container.
    line: u32,
    /// The callee that did the removal, when it was not this frame's own statement.
    via: Option<u32>,
    /// The container the view names — carried so a diagnostic can name it without re-deriving
    /// the view→container mapping from a frame that has since closed.
    container: u16,
}

/// Record `d` for `view`, keeping [`ViewCause::Reshaped`] when both apply.
fn record_cause(map: &mut HashMap<u16, Disturbance>, view: u16, d: Disturbance) {
    let slot = map.entry(view).or_insert(d);
    if d.cause == ViewCause::Reshaped {
        *slot = d;
    }
}

/// @PLN130 F2 + F8 — every VIEW binding that is live across a disturbance of its container.
///
/// This is @FR-B-View's materialise clause: a struct-typed projection aliases without `&`, and
/// where the container is DISTURBED (@FR-B-Disturb) while the view is still LIVE the binding
/// gives up the alias, takes its own copy at the bind, and the author is told.  Answering it
/// is this walk's whole job; the two users of the verdict — `scan_set`'s dep strip and the
/// interpreter's materialising emitters — act on it.
///
/// Two disturbances, one question. A RESHAPE (`v.remove(i)`, `e#remove`) renumbers the
/// positions inside the container's store, and a view is a `DbRef` pinned to one — so a view
/// live across it silently starts naming a different element (probes 03-07, 29: a pure READ
/// answered `44/444` where its element held `33/333`, and a write tore a live record). A
/// REASSIGNMENT of the container variable is the other:
///
/// A dep names a VARIABLE, not a store instance, so a view bound from `bx.v[0]` keeps
/// reading "wherever `bx` points" — and `bx = <other value>` re-points it. The view then
/// answers the REPLACEMENT's value (measured 22 where its element held 11, on both
/// backends), and on `--native` the displaced store is freed, so the read is a genuine
/// use-after-free into whatever now occupies the space (measured 703, a `Wide` field).
///
/// A vector local is immune and that is what hid this: a vector literal allocates through a
/// hidden `__vdb_N` owner, so `a = [...]` twice mints TWO stores and the first keeps its
/// identity. A struct local has no such indirection — `bx` IS the owner.
///
/// The question this answers is deliberately about the VIEW, not the container. *"Is the
/// container re-established anywhere in this function"* is a per-FUNCTION proxy for a
/// per-BINDING fact, and it is wrong in the direction that costs correctness elsewhere: an
/// unrolled `for pf in fields(p)` re-establishes `pf` once per field, and the `is`-pattern
/// subject bound from it dies inside its own arm long before the next one — yet the proxy
/// stripped that subject's deps, which put an `OpFreeRef` in a scope where the declaration
/// was not visible and stopped `tests/scripts/45-field-iter.loft` compiling under `--native`.
///
/// So a view is at risk only where a disturbance of its container can be reached WHILE THE
/// VIEW IS STILL LIVE. A disturbance sitting in a block the view's own block has already
/// closed cannot reach it, which is exactly the unrolled-iteration case above.
///
/// Four forms establish a store, matching the four emitters measured to break:
///
/// - `OpDatabase(v, …)` — a struct literal built into `v`'s own store;
/// - `Set(v, Var(src))` — the C86 whole-value bind, which codegen lowers to
///   `OpDatabase` + `OpCopyRecord` into a fresh store (the `bx = other` swap);
/// - `Set(v, Call(f, …))` — a bind from a user function's return (`bx = mk(22)`);
/// - passing `v` as a `&` argument to a callee that reassigns that parameter wholesale
///   ([`reassigned_ref_params`]) — the callee frees the caller's store, so from the view's
///   point of view this is a reassignment that happens to be spelled as a call.
///
/// A container's FIRST establishment is never a risk: it necessarily runs before any view of
/// it can exist, so no separate "is this the definition" test is needed.
///
/// A vector-typed container can never be REASSIGNED into danger — it reaches its store
/// through a hidden `__vdb_N` owner, so re-pointing the local leaves the old store's identity
/// intact — and [`established_stores`] never reports one. It very much can be RESHAPED, which
/// is the F2 half.
///
/// **A disturbance alone is not enough: the view must still be USED afterwards.** Keying on
/// order alone is what F2 shipped (a flat per-function `reshaped_containers` set), and it
/// costs both halves of this plan's closure bar — a write that mainline lands is LOST, and the
/// advice says *"`v` is modified while `c` is in use"* of a `c` that is not in use (probe 39
/// cells L1, L5, L8, L10). So a disturbance only SHAKES the open views of that container;
/// a later read or write of a shaken view is what condemns it. A view whose last use precedes
/// the disturbance keeps its alias and writes through, which is the rustc rule.
///
/// A block frame ends the views the block OWNS, which is not the same as the views bound in
/// it. Re-binding an outer local inside a nested block gives a view that outlives the block,
/// and dropping it at the close is what let loft#1184 through — `a = w.inner` in a loop body,
/// `w = Outer{inner: a}` on the next turn, every heap field of `a` empty from the second
/// iteration on. So a view goes into the frame that owns its VARIABLE (the block it was first
/// bound in), and a LOOP is walked twice: the first pass could only shake views that already
/// existed, and the second supplies the use that condemns one the body itself bound. Two
/// passes reach the fixpoint, because the second binds exactly the views the first did.
///
/// Known lower bound: only a `Var` names a container, so a disturbance reached through some
/// other expression is not recognised — [`reshaped_containers`] and [`established_stores`] are
/// both lower bounds, and a missed case keeps today's behaviour rather than inventing a new
/// one.
fn collect_views_to_materialise(
    code: &Value,
    function: &Function,
    data: &Data,
    database: &crate::database::Stores,
) -> HashMap<u16, Disturbance> {
    let out = ViewWalk::run(code, function, data, None, Some(database), 0);
    if !out.is_empty() && std::env::var_os("LOFT_DEBUG_F8").is_some() {
        let mut names: Vec<String> = out
            .iter()
            .map(|(v, d)| format!("{}({:?})", function.name(*v), d.cause))
            .collect();
        names.sort();
        eprintln!(
            "[f8] views live across a container disturbance: {}",
            names.join(" ")
        );
    }
    // The whole `Disturbance` is kept, not just its cause: it already carries the CONTAINER
    // that was disturbed, and the two report sites used to re-derive one from the right-hand
    // side instead.  That was a restatement — and once a binding can view more than one
    // container it is also a wrong answer, because the RHS names both and only one of them was
    // disturbed.  One home for the fact, named where it was observed.
    out
}

/// The state of [`collect_views_to_materialise`]'s in-order walk.
struct ViewWalk<'a> {
    function: &'a Function,
    data: &'a Data,
    /// One frame per open block: the views the block OWNS, and the container each one views.
    /// A view dies when the block that owns its VARIABLE closes — which is where the variable
    /// was first bound, not necessarily where this binding was written (see `bound_at`).
    open: Vec<Vec<(u16, u16, u32)>>,
    /// The frame depth each variable was first BOUND at, which is the block that owns it.
    ///
    /// A view goes into the frame that owns its VARIABLE, not the block the binding statement
    /// happens to sit in — the two differ whenever an outer local is re-bound inside a nested
    /// block, and the variable then outlives that block. A hoisted `Set(v, Null)` declaration
    /// is skipped: it is emitted at function scope for every ref- and text-typed local, so
    /// counting it would put EVERY view at function scope and undo the frame model.
    bound_at: HashMap<u16, usize>,
    /// Views whose container has been disturbed since the bind, and by what. Being shaken is
    /// not yet a verdict — it becomes one at the next use.
    shaken: HashMap<u16, Disturbance>,
    /// The answer: views USED after their container was disturbed.
    out: HashMap<u16, Disturbance>,
    /// `Some` also counts a CALLEE's removal from a `&` parameter as a reshape (F9's refusal);
    /// `None` stays inside this frame (F2's materialise). See [`reshaped_via_call`] for why the
    /// two questions do not share an answer.
    cross_frame: Option<&'a RemovedParams>,
    /// Every place this function REBUILDS in whole — `x.a = [9, 9]` emits an
    /// `OpClearVector` on the field and then exactly the `OpNewRecord`s an append emits, and
    /// the two are SEPARATE statements, so the pairing cannot be seen one statement at a time.
    ///
    /// `(B-Disturb)` is explicit that OVERWRITING a place is not disturbing it — *"`o.inner =
    /// Box{…}` writes INTO the place `o.inner` already occupies, so a view of it survives"* —
    /// and without this subtraction `c = b.vecf; b.vecf = [9, 9]` materialised `c`, which
    /// `bind-copies-or-views-the-whole-boundary` caught on its `(B-View-Base)` cell: the
    /// seventeen-cell guard that exists to pin exactly this line.
    ///
    /// Accumulated for the whole walk rather than per block, which errs the safe way: a place
    /// cleared once and genuinely GROWN later is a MISSED disturbance, costing a materialise,
    /// where the other direction costs a program its meaning.
    cleared: HashSet<(u16, u32)>,
    /// The store, where the caller has one.  Only the field-NUMBER to byte-OFFSET conversion
    /// needs it (`grown_containers`); the refusal path runs without and keeps the conservative
    /// answer, which for a REFUSAL is the safe direction — refusing less, never more.
    database: Option<&'a crate::database::Stores>,
    /// The source line of the statement being walked, tracked from the `Value::Line` markers
    /// a block interleaves with its operators — the only line information the IR carries.
    line: u32,
}

impl ViewWalk<'_> {
    /// Walk `code` in source order and answer which views were used after their container was
    /// disturbed.
    ///
    /// `start_line` seeds the line tracking with the definition's own: a block emits a
    /// `Value::Line` marker only where the line CHANGES, so a body whose first statement is on
    /// the signature's line carries no marker at all and would otherwise report line 0.
    fn run<'a>(
        code: &Value,
        function: &'a Function,
        data: &'a Data,
        cross_frame: Option<&'a RemovedParams>,
        database: Option<&'a crate::database::Stores>,
        start_line: u32,
    ) -> HashMap<u16, Disturbance> {
        let mut walk = ViewWalk {
            function,
            data,
            open: vec![Vec::new()],
            bound_at: HashMap::new(),
            shaken: HashMap::new(),
            out: HashMap::new(),
            cross_frame,
            database,
            cleared: HashSet::new(),
            line: start_line,
        };
        walk.walk_block(std::slice::from_ref(code));
        walk.out
    }

    fn walk_block(&mut self, stmts: &[Value]) {
        for stmt in stmts {
            if let Value::Line(n) = stmt.unspan() {
                self.line = *n;
            }
            self.walk_stmt(stmt);
        }
    }

    /// Descend in SOURCE ORDER, because the whole point is which came first.
    ///
    /// Only the block forms listed here are descended into; anything else — a `match`, say —
    /// is handled whole by [`Self::leaf`], which both shakes and reads uses over the entire
    /// statement. That is deliberately coarse in both directions for a form we cannot order
    /// internally, and it is the safety net that keeps an unrecognised construct from hiding
    /// a removal.
    fn walk_stmt(&mut self, stmt: &Value) {
        match stmt.unspan() {
            // `Insert` is spliced into the enclosing block rather than forming its own, so
            // its statements are siblings and a view bound there stays live here.
            Value::Insert(ops) => self.walk_block(ops),
            Value::Block(b) => self.scoped(&b.operators),
            Value::Loop(b) => {
                // A loop body runs again, so a disturbance ANYWHERE inside it precedes every
                // use inside it on the next iteration. Shaking before the body is walked is
                // what makes a view held from OUTSIDE the loop and used at the top of the
                // body come out live across a removal at the bottom of it.
                self.disturb(stmt);
                self.scoped(&b.operators);
                // The BACK EDGE. The shake above could only reach views that already existed;
                // a view the body itself binds is disturbed by the same statements one turn
                // later, and nothing had seen it yet. So shake again and re-walk — the second
                // pass is what supplies the USE that condemns it (loft#1184). One extra pass
                // reaches the fixpoint: it binds exactly the views the first pass bound, so a
                // third would read the same state.
                let before: HashSet<u16> = self.shaken.keys().copied().collect();
                self.disturb(stmt);
                if self.shaken.keys().any(|v| !before.contains(v)) {
                    self.scoped(&b.operators);
                }
            }
            Value::If(cond, t, e) => {
                // The condition is evaluated before either branch and is not part of one.
                self.leaf(cond);
                for branch in [t.unspan(), e.unspan()] {
                    match branch {
                        Value::Block(b) => self.scoped(&b.operators),
                        Value::Insert(ops) => self.scoped(ops),
                        other => self.leaf(other),
                    }
                }
            }
            // A `Set` whose VALUE carries statements — a value `if` or `match`, a block —
            // runs them BEFORE the target is written, so they are walked in that order and
            // the target is recorded after.  Read whole by `leaf` instead, a view bound
            // inside one arm and the reassignment of its container in the SAME arm were
            // never separated in time: `got = match sh { Holder{inner} => { sh = Empty{…};
            // inner.a }, … }` was shaken and used in one indivisible step, so nothing
            // materialised and nothing was said, on both backends (loft#1394).  `leaf`'s own
            // doc calls that coarseness deliberate in both directions, and it is — for a
            // form whose internal order is unknown.  A `Set`'s is not: the value first, the
            // target after.
            Value::Set(_, rhs)
                if matches!(
                    rhs.unspan(),
                    Value::If(_, _, _) | Value::Block(_) | Value::Insert(_)
                ) =>
            {
                self.note_binding_depth(stmt);
                self.walk_stmt(rhs);
                // The TARGET's own establishment comes after its value, because that is when
                // the slot is written — and it is read off the whole statement, since a
                // struct-enum literal's mint names a work-ref and only the `Set` says which
                // variable took it (`established_stores`).  Skipping it here left an enum
                // subject reassigned inside a branch arm disturbing nothing at all.
                self.disturb(stmt);
                self.record_target(stmt);
            }
            other => self.leaf(other),
        }
    }

    /// Walk a nested block in its own frame: a view bound inside one dies with it.
    fn scoped(&mut self, stmts: &[Value]) {
        self.open.push(Vec::new());
        self.walk_block(stmts);
        self.open.pop();
    }

    /// Shake for everything `stmt` disturbs, at any depth inside it.
    fn disturb(&mut self, stmt: &Value) {
        self.shake_places(
            &reshaped_containers(stmt, self.data, self.function),
            ViewCause::Reshaped,
            None,
        );
        stmt.walk(&mut |v| {
            let Value::Call(d, args) = v else { return };
            if self.data.def(*d).name() != "OpClearVector" {
                return;
            }
            if let Some(place) = args
                .first()
                .and_then(|a| base_container_place(a, self.data))
            {
                self.cleared.insert(place);
            }
        });
        self.shake_places(
            &grown_containers(stmt, self.data, self.function, self.database, &self.cleared),
            ViewCause::Grown,
            None,
        );
        if let Some(removed) = self.cross_frame {
            for (container, callee) in reshaped_via_call(stmt, self.data, removed) {
                self.shake(
                    &HashSet::from([container]),
                    ViewCause::Reshaped,
                    Some(callee),
                );
            }
        }
        let established = established_stores(stmt, self.function, self.data);
        self.shake(&established, ViewCause::Reassigned, None);
    }

    /// One statement, in the order its parts take effect: what it disturbs, then what it
    /// uses, then what it (re)binds.
    fn leaf(&mut self, stmt: &Value) {
        self.note_binding_depth(stmt);
        self.disturb(stmt);
        // Reading or writing a shaken view is what makes the disturbance matter.
        self.note_uses(stmt);
        self.record_target(stmt);
    }

    /// What a `Set` does to the walk's state once its VALUE has been accounted for — the last
    /// of [`Self::leaf`]'s three steps, and the only one a value-branch `Set` still owes after
    /// its arms have been walked in their own order.
    ///
    /// A `Set` REPLACES whatever the slot held, so the old binding's troubles end here and a
    /// view bound by this statement is live from here on.  Recorded LAST, so a statement that
    /// re-establishes a container and binds a view of the NEW value does not mark the fresh
    /// view against its own establishment.
    fn record_target(&mut self, stmt: &Value) {
        if let Value::Set(v, rhs) = stmt.unspan() {
            self.shaken.remove(v);
            for frame in &mut self.open {
                frame.retain(|(view, _, _)| view != v);
            }
            // Through `base()`: a nullable `S?` view is the same storage behind a
            // nullability marker (@FR-L-Null), so it is at risk exactly as its dense twin is.
            //
            // A COLLECTION-typed view is here too, because `(B-View-Depth)` makes an index
            // read a view "whatever the element type" and the materialise arm now has a
            // collection case (loft#1377): the record path strips the container dep and lets
            // the BIND copy, which for a collection is decided at PARSE time and cannot hear
            // a scope-pass strip, so the copy is emitted here instead.
            //
            // A bare `&` link to a whole container is not recorded either, and that is
            // `base_container_var`'s doing rather than this list's: it answers `None` unless
            // the right-hand side is a PROJECTION, so `pe = &e` names no container while
            // `pw = &w[0]` names `w`.  That is the in-versus-to distinction `(B-Ref-Alias)`
            // needs, and it lives in one place.
            // ⚠ THESE TWO TESTS ANSWER DIFFERENT QUESTIONS, AND BOTH ARE LOAD-BEARING.  The
            // type list says WHICH BINDINGS CAN BE VIEWS AT ALL; `value_view_place` says
            // WHICH PLACE a value views.  They were widened for different defects, on
            // different branches, and met here at a cherry-pick — so the pairing reads as an
            // accident of adjacency in the history and is not one.  Narrowing either silently
            // un-fixes a shipped defect, and none of them fails loudly:
            //
            //   * drop `Type::Vector` (or the `.base()` that reaches it through a `τ?`) and a
            //     COLLECTION view is never named, which is loft#1377 and loft#1399 — the
            //     latter answers correctly on `--native` either way, so the interpreter goes
            //     quietly wrong on one backend only;
            //   * drop `value_view_place` back to `base_container_place` and a BRANCH-valued
            //     binding names no container — which costs loft#1396 AND loft#1399, since the
            //     latter's binding is branch-valued too, and a `??`-DISCHARGED projection names
            //     none either, which is loft#1401.
            //
            // Both narrowings were MEASURED rather than reasoned, by making each one and
            // running the guards: dropping `Type::Vector` fails
            // `1377-a-collection-typed-element-view-materialises-too` and
            // `a-collection-projection-arm-of-a-branch-materialises` while loft#1396's guard
            // stays green; dropping the namer fails loft#1396's guard AND loft#1399's, because
            // a branch-valued binding is not named at all without it.
            //
            // Naming and copy have to land together — a named binding whose deps are stripped
            // with no emitter copy owns a store it only views (loft#778's class), which is why
            // widening the type list ALONE was measured unsound.  The guards for those three
            // issues are this line's regression net; nothing names the pairing itself, so it
            // is named here.  `binding-history.md` D-bind-23 carries the history.
            // A binding the loop ITERATES is not materialised: the iteration depends on its
            // identity, so a store of its own makes the loop walk a COPY while the body's
            // `#remove` empties the original — which does not terminate.  Measured: once a
            // removal reached through a FIELD became a disturbance (`D-bind-26`),
            // `for e in d.items { e#remove; }` shook the loop's own source temp — a view of
            // `(d, off_items)` by every test this walk applies — and `903-loop-remove` went
            // from 0.06s to a 300s corpus timeout.
            //
            // ⚠ The obvious wider rule is WRONG here, and was measured wrong: *"a view the
            // author cannot name"* excludes a `match` PAYLOAD binding too, which the parser
            // renames to `_mv_<field>_N` and which the author very much wrote — so
            // `a-payload-binding-warns-when-its-subject-is-given-another-variant` read its
            // subject's new variant.  The fact belongs on the variable the lowering created,
            // not on the shape of its name.
            if !self.function.is_iteration_source(*v)
                && matches!(
                    self.function.tp(*v).base(),
                    Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
                )
            {
                // The view belongs to the frame that owns its VARIABLE. Re-binding an outer
                // local inside a nested block gives a view that outlives the block, and
                // dropping it at the block's close is what let loft#1184 through: `a =
                // w.inner` in a loop body, `w = Outer{inner: a}` on the next turn.
                let depth = self.bound_at.get(v).copied().unwrap_or(self.open.len());
                let idx = depth.min(self.open.len()).saturating_sub(1);
                // `(B-Disturb)` ends the place a view names when its CONTAINER is disturbed,
                // and a chain of views names one place however many statements it is spelled
                // over.  `base_container_place` resolves a chain inside ONE expression
                // (`dv.tiles.proto` names `dv`); split through a local it does not —
                // `t = dv.tiles; prev = t.proto` recorded `prev` as a view of `t`, so
                // reassigning `dv` shook `t` and left `prev` reading the new value, on both
                // backends and with nothing said, where the one-expression spelling
                // materialises and says so (loft#1393).  Resolve through the views already
                // open, which is the same walk one level out.
                // One entry per PLACE the value can name.  The frame already holds a
                // `(view, container, field)` triple per pair, so a binding that views two
                // containers is two entries and `shake_places` matches either — no new shape,
                // and `record_target`'s own `retain` clears them all when the slot is rebound.
                for (container, field) in value_view_places(rhs, self.data, self.function) {
                    let (container, field) = self.resolve_view_root(container, field);
                    if !self.open[idx].contains(&(*v, container, field)) {
                        self.open[idx].push((*v, container, field));
                    }
                }
            }
        }
    }

    /// The container a view ultimately names, following views this walk has already opened.
    ///
    /// A view whose container is itself a view names a place inside the OUTER container, so a
    /// disturbance of that outer one ends it: `t = dv.tiles; prev = t.proto` makes `prev` a
    /// place inside `dv`, exactly as the single-expression `prev = dv.tiles.proto` does.  The
    /// FIELD that survives is the outermost one — the one a disturbance of the root can name —
    /// which is the rule [`base_container_place`] states for a chain inside one expression.
    ///
    /// Bounded, and it stops at the first container that is not an open view: a view rebound
    /// to name a chain that leads back to itself would otherwise walk forever, and a lower
    /// bound is what this walk answers everywhere else.
    fn resolve_view_root(&self, mut container: u16, mut field: u32) -> (u16, u32) {
        for _ in 0..16 {
            let Some(&(_, outer, outer_field)) = self
                .open
                .iter()
                .flatten()
                .find(|(view, _, _)| *view == container)
            else {
                return (container, field);
            };
            // Stop at a COMPILER-GENERATED container.  A vector local is bound to its own
            // backing store — `v = OpGetField(__vdb_1, 0, …)` — so it is an open "view" of a
            // local nothing in the program can disturb, and following it moved `c = &v[0]`
            // off `v` and onto `__vdb_1`: the `(B-Ref-Reshape)` refusal for `v.remove(2)`
            // under a live link then had no container to match and stopped firing.  A place
            // is only inside a container the author can reassign.
            if outer == container || self.function.is_compiler_generated(outer) {
                return (container, field);
            }
            container = outer;
            field = outer_field;
        }
        (container, field)
    }

    /// Note where each variable is first BOUND, which is [`Self::leaf`]'s frame for a view of it.
    ///
    /// A `Set(v, Null)` is the hoisted declaration every ref- and text-typed local gets at
    /// function scope, not a binding, so it is not what owns the variable.
    fn note_binding_depth(&mut self, stmt: &Value) {
        if let Value::Set(v, rhs) = stmt.unspan()
            && !matches!(rhs.unspan(), Value::Null)
        {
            self.bound_at.entry(*v).or_insert(self.open.len());
        }
    }

    /// Mark every open view of one of `containers` as disturbed. Not a verdict yet — only a
    /// use after this point makes the view wrong, which is what separates one that is live
    /// across the disturbance from one that is already dead.
    fn shake(&mut self, containers: &HashSet<u16>, cause: ViewCause, via: Option<u32>) {
        if containers.is_empty() {
            return;
        }
        let places: HashSet<(u16, u32)> = containers.iter().map(|&c| (c, ANY_FIELD)).collect();
        self.shake_places(&places, cause, via);
    }

    /// [`Self::shake`] where the disturbance names a PLACE rather than a whole variable — a
    /// growth of one FIELD does not end the places inside its siblings.  A view and a
    /// disturbance match when [`same_place`] says they name the same storage.
    fn shake_places(&mut self, places: &HashSet<(u16, u32)>, cause: ViewCause, via: Option<u32>) {
        if places.is_empty() {
            return;
        }
        let hit: Vec<(u16, u16, u32)> = self
            .open
            .iter()
            .flatten()
            .filter(|(_, container, field)| {
                places.iter().any(|&p| same_place((*container, *field), p))
            })
            .copied()
            .collect();
        for (view, container, _) in hit {
            let d = Disturbance {
                cause,
                line: self.line,
                via,
                container,
            };
            record_cause(&mut self.shaken, view, d);
        }
    }

    /// Condemn every shaken view this statement reads or writes.
    ///
    /// A `Set`'s target is a `u16` slot rather than a `Value::Var`, so a walk for `Var` nodes
    /// already counts only genuine reads — a rebind does not read the binding it replaces.
    fn note_uses(&mut self, stmt: &Value) {
        if self.shaken.is_empty() {
            return;
        }
        let mut used: Vec<u16> = Vec::new();
        stmt.walk(&mut |v| {
            if let Value::Var(x) = v {
                used.push(*x);
            }
        });
        for v in used {
            if let Some(cause) = self.shaken.get(&v).copied() {
                record_cause(&mut self.out, v, cause);
            }
        }
    }
}

/// Visit every node of `code`, telling `f` which source LINE is in effect for it.
///
/// A statement's line is not on the statement: a block interleaves `Value::Line(n)` markers
/// with its operators, so the line has to be carried down the walk. `Block`, `Loop` and
/// `Insert` therefore re-read it per operator; every other form inherits its parent's.
fn walk_lined(code: &Value, line: u32, f: &mut impl FnMut(&Value, u32)) {
    let node = code.unspan();
    let stmts: Option<&[Value]> = match node {
        Value::Block(b) | Value::Loop(b) => Some(&b.operators),
        Value::Insert(ops) => Some(ops),
        _ => None,
    };
    f(node, line);
    if let Some(stmts) = stmts {
        let mut cur = line;
        for s in stmts {
            if let Value::Line(n) = s.unspan() {
                cur = *n;
            }
            walk_lined(s, cur, f);
        }
    } else {
        node.for_each_child(&mut |c| walk_lined(c, line, f));
    }
}

/// @PLN130 F9 — does argument `arg` name an ELEMENT of container `c`?
///
/// Both ways an author can write it, because the two reach the check in different shapes:
///
/// - **bound earlier** (`t = v[2]; f(t, v)`) — `t` arrives as a plain `Var` and carries `v` in
///   its type deps, which is the borrow relation itself;
/// - **written into the call** (`f(v[2], v)`) — the parser does not leave that inline. It lifts
///   the projection into a temp first, so the argument is
///   `Insert([Set(t, OpGetVector(v, …)), OpCreateStack(t)])` and the alias is a `Set` INSIDE the
///   argument expression. Reading only the argument's value misses it, which is how the issue's
///   own repro (`shift(v[2], v)`) went unreported while `t = v[2]; shift(t, v)` did not.
///
/// The lifted `Set` is looked up by the temp the argument actually passes rather than by
/// searching the expression for any projection of `c`: `f(w[v[0].n], v)` mentions `v[0]` but
/// passes an element of `w`, and a search would refuse it.
fn arg_references_element_of(arg: &Value, c: u16, function: &Function, data: &Data) -> bool {
    let Value::Var(t) = arg_target(arg, data) else {
        return base_container_var(arg_target(arg, data), data) == Some(c);
    };
    if *t == c {
        return false;
    }
    if function.tp(*t).depend().contains(&c) {
        return true;
    }
    let mut lifted = false;
    arg.walk(&mut |n| {
        if let Value::Set(s, rhs) = n
            && *s == *t
            && base_container_var(rhs.unspan(), data) == Some(c)
        {
            lifted = true;
        }
    });
    lifted
}

/// The value an argument ultimately passes: the tail of any lifting preamble, with the
/// `OpCreateStack` wrapper a `&` parameter adds peeled off.
///
/// Deliberately NOT folded into [`peel_stack_ref`]: that one answers which variable a `&`
/// CONTAINER argument names, where no lift is involved, and widening it would quietly change
/// what @PLN130 F8's `established_stores` treats as a reassignment.
fn arg_target<'a>(arg: &'a Value, data: &Data) -> &'a Value {
    let inner = arg.tail().unspan();
    if let Value::Call(cs, cargs) = inner
        && data.def(*cs).name() == "OpCreateStack"
        && let Some(first) = cargs.first()
    {
        return first.tail().unspan();
    }
    inner
}

/// @PLN130 F9 — one program shape the compiler REFUSES, ready to be reported.
///
/// Carries a position rather than being emitted here, because the analysis runs over `Data`
/// (where every callee's body is available) while the diagnostics collector lives on the
/// parser's lexer.
pub struct ReshapeRefusal {
    pub file: String,
    pub line: u32,
    pub message: String,
}

/// @PLN130 F9 / [loft#779](https://github.com/loft-lang/loft/issues/779) — the shapes where a
/// container is reshaped while a reference into it is still live, which loft REFUSES.
///
/// **B-Ref-Alias is unconditional** — a `&` binding is a live link to the source, so every
/// write through it reaches the source. There is exactly one program shape where it cannot:
/// `remove` renumbers the positions inside a container's store, and a reference is pinned to
/// one, so a write through it lands on the wrong element or on a vacated slot and is lost.
/// Rather than carry runtime machinery to re-point the link, that shape is rejected before it
/// runs (maker, 2026-08-05). It is the rustc bargain in loft's spelling: where rustc refuses
/// the mutation while a borrow is live, loft refuses the removal.
///
/// Two producers, because a reference into a container reaches a removal two ways:
///
/// 1. a **`&` LINK in this frame** (`c = &v[0]`) that is live across a removal from `v` — the
///    removal being either this frame's own or one a callee does through a `&` parameter;
/// 2. a **CALL that is handed both a container and a reference into it** (`shift(v[2], v)`),
///    where the callee removes from the container parameter. Checked at the CALL SITE, which is
///    the only place the two arguments are known to name the same store: inside the callee they
///    are two unrelated parameters, and refusing there would reject sound programs.
///
/// **Liveness is the condition, not existence** — the rustc rule, and the same walk F2 uses.
/// `c = &v[0]; c.n = 1; v.remove(0);` keeps compiling: the link is dead before the removal, so
/// there is no conflict and the write lands.
///
/// Producer 2 does **not** ask whether the argument was spelled `&`, and that is measured, not
/// an oversight: a plain struct parameter aliases the caller's element exactly as a `&` one
/// does (`fn w(t: Box) { t.n = 99 }` called as `w(v[2])` writes 99 into `v` — and loft's own
/// `warn_redundant_amp` advice tells authors so). Refusing only the `&` spelling would mean an
/// author who takes that advice and drops the `&` trades a compile error for a silent lost
/// write. Producer 1 is `&`-only for the opposite and equally measured reason: a PLAIN local
/// bind does not alias across a reshape, because @PLN130 F2 materialises it and says so.
///
/// Known lower bound, in the safe direction (the refusal simply does not fire): a callee
/// reached only through a runtime fn-ref has no static call edge, so [`removed_params_map`]'s
/// closure cannot follow it. Declaration order does NOT matter — the check runs once the whole
/// world is parsed, so a callee written below its caller is answered the same way.
///
/// Every definition is checked, the stdlib's included. Filtering by `source` was tried and is
/// wrong: `Parser::parse_str` — the whole Rust test harness — never leaves `STD_SOURCE`, so the
/// filter silently made the check a no-op there while it still fired on a file. A pass over
/// definitions that cannot possibly trip it is the cheaper mistake.
#[must_use]
pub fn reshape_refusals(data: &Data) -> Vec<ReshapeRefusal> {
    let removed = removed_params_map(data);
    let mut out: Vec<ReshapeRefusal> = Vec::new();
    for d_nr in 0..data.definitions() {
        out.extend(def_reshape_refusals(data, d_nr, &removed));
    }
    out
}

fn def_reshape_refusals(data: &Data, d_nr: u32, removed: &RemovedParams) -> Vec<ReshapeRefusal> {
    let def = data.def(d_nr);
    if !matches!(def.def_type, DefType::Function) || matches!(def.code, Value::Null) {
        return Vec::new();
    }
    let function = &def.variables;
    let file = def.position.file.clone();
    let mut out: Vec<ReshapeRefusal> = Vec::new();
    // (1) — a `&` link this frame holds, still live where its container is disturbed. Every
    // cause the walk reports is refused: each one ends the place the reference names, and a
    // reference that cannot reach its source is not what `&` asked for.  That includes the
    // GROWTH the walk learned in loft#1373 — `c = &v[0]; v += [x]; c.n` names an element the
    // growth may have moved, which is the same reason the other three are refused.
    for (view, d) in ViewWalk::run(
        &def.code,
        function,
        data,
        Some(removed),
        None,
        def.position.line,
    ) {
        if !function.is_amp_link(view) {
            continue;
        }
        let view_name = function.name(view);
        let container = function.name(d.container);
        // The two causes destroy the place differently, so they read differently and have
        // different ways out — but the verdict is the same.
        let (what, why) = match d.cause {
            ViewCause::Reshaped => (
                format!("remove from `{container}`"),
                format!(
                    "a removal renumbers the remaining elements, so a write through \
                     `{view_name}` would no longer reach the element it names"
                ),
            ),
            ViewCause::Grown => (
                format!("grow `{container}`"),
                format!(
                    "a container that outgrows its allocation moves every element, so a \
                     write through `{view_name}` would no longer reach the element it names"
                ),
            ),
            ViewCause::Reassigned => (
                format!("give `{container}` a new value"),
                format!(
                    "`{view_name}` names a place inside `{container}`, and replacing \
                     `{container}` leaves that place with nothing to point at"
                ),
            ),
        };
        let message = match d.via {
            Some(callee) => format!(
                "cannot call `{callee_name}` while `{view_name}` references a place inside \
                 `{container}` — `{callee_name}` would {what}, and {why}. Move the call after \
                 the last use of `{view_name}`, or bind without `&` to work on a copy",
                callee_name = data.def(callee).original_name()
            ),
            None => format!(
                "cannot {what} while `{view_name}` references a place inside it — {why}. Move \
                 it after the last use of `{view_name}`, or bind without `&` to work on a copy"
            ),
        };
        out.push(ReshapeRefusal {
            file: file.clone(),
            line: d.line,
            message,
        });
    }
    // (2) — a call handed both a container and a reference into it.
    walk_lined(&def.code, def.position.line, &mut |node, line| {
        let Value::Call(callee, args) = node else {
            return;
        };
        let cdef = data.def(*callee);
        let Some(params) = removed.get(callee) else {
            return;
        };
        for k in params {
            let k = usize::from(*k);
            let Some(Value::Var(c)) = args.get(k).map(|a| peel_stack_ref(a, data)) else {
                continue;
            };
            for (j, arg) in args.iter().enumerate() {
                if j == k {
                    continue;
                }
                // Only a parameter that can NAME an element is a hazard; a scalar or a text
                // copies, so there is nothing pinned to a position.
                let Some(attr) = cdef.attributes.get(j) else {
                    continue;
                };
                let ptp = match &attr.typedef {
                    Type::RefVar(inner) => inner.as_ref(),
                    other => other,
                };
                if !matches!(ptp, Type::Reference(_, _) | Type::Enum(_, true, _)) {
                    continue;
                }
                if !arg_references_element_of(arg, *c, function, data) {
                    continue;
                }
                out.push(ReshapeRefusal {
                    file: file.clone(),
                    line,
                    message: format!(
                        "cannot pass both `{cname}` and a reference into it to `{fname}` — \
                         `{fname}` removes from `{cparam}`, which renumbers the remaining \
                         elements while `{vparam}` still references one, so a write through \
                         `{vparam}` would be lost. Pass the INDEX instead and read the element \
                         again after the removal",
                        cname = function.name(*c),
                        fname = cdef.original_name(),
                        cparam = cdef.attributes[k].name,
                        vparam = attr.name,
                    ),
                });
            }
        }
    });
    out.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.message.cmp(&b.message)));
    out.dedup_by(|a, b| a.line == b.line && a.message == b.message);
    out
}

/// Every variable whose store `stmt` establishes, at any depth.
///
/// Nested blocks are included deliberately: `if flag { bx = T{…} }` establishes `bx` as far
/// as a view bound outside the `if` is concerned.
fn established_stores(stmt: &Value, function: &Function, data: &Data) -> HashSet<u16> {
    let mut out: HashSet<u16> = HashSet::new();
    let note = |v: u16, out: &mut HashSet<u16>| {
        if matches!(
            function.tp(v),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && !function.is_compiler_generated(v)
        {
            out.insert(v);
        }
    };
    stmt.walk(&mut |v| match v {
        Value::Call(d, args) if data.def(*d).name() == "OpDatabase" => {
            if let Some(Value::Var(t)) = args.first().map(Value::unspan) {
                note(*t, &mut out);
            }
        }
        // A call that reassigns one of its `&` parameters displaces the caller's store.
        Value::Call(d, args) if data.def(*d).name.starts_with("n_") => {
            let reassigned = reassigned_ref_params(data, *d);
            for (i, arg) in args.iter().enumerate() {
                if !u16::try_from(i).is_ok_and(|i| reassigned.contains(&i)) {
                    continue;
                }
                if let Value::Var(t) = peel_stack_ref(arg, data) {
                    note(*t, &mut out);
                }
            }
        }
        Value::Set(t, rhs) => {
            // `Set(v, Null)` is the in-place re-init prelude that PRECEDES an `OpDatabase`
            // for the same var, not an establishment of its own.
            let establishes = match rhs.unspan() {
                Value::Var(src) => matches!(
                    function.tp(*src),
                    Type::Reference(_, _) | Type::Enum(_, true, _)
                ),
                Value::Call(f, _) => data.def(*f).name.starts_with("n_"),
                // A struct-ENUM literal builds into a work-ref and hands the variable that
                // block: `sh = { OpDatabase(__ref_p2_2, …); …; __ref_p2_2 }`.  The tail is
                // the same `Var` establishment the first arm names, one wrapper out — and
                // read only through the wrapper's own `OpDatabase`, which names the COMPILER
                // work-ref and is filtered out, no reassignment of a struct-enum local
                // established anything.  A payload view of it then survived the subject's
                // reassignment with no materialise and no warning, where the plain-struct
                // twin (which builds in place, `OpDatabase(h)`) materialises and says so.
                // `Value::tail` stops at an `If`, so a `??` or a branch-valued right-hand
                // side still answers "no" here.
                Value::Block(_) | Value::Insert(_) => matches!(
                    rhs.tail().unspan(),
                    Value::Var(src) if matches!(
                        function.tp(*src),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                ),
                _ => false,
            };
            if establishes {
                note(*t, &mut out);
            }
        }
        _ => {}
    });
    out
}

/// The variable whose scope-end DROP a plain whole-value copy `v = src` takes away, or
/// `None` where the copy moves nothing.  `@FR-H-Drop`: responsibility moves with a copy.
///
/// `t = s` deep-copies (`@FR-B-Copy`) and leaves two records holding one resource — the
/// failure C111 names for a container and answers with a MOVE — so the copy owns the
/// resource and the SOURCE stops dropping.  Only the drop moves; both stores are still freed
/// by the ordinary sweep.  A copy off a PARAMETER runs the other way: the caller owns the
/// resource (calls.md F-ParamHeap — the parameter aliases it), so it is the callee's copy,
/// `v`, that never drops.
///
/// The fact belongs to the ASSIGNMENT, not the variable (`@FR-O-Latest`): a source rebound
/// after the copy releases the record it displaces through [`Scopes::displaced_drop`] — no,
/// it does not: that record's resource is the copy's now, so the rebind SKIPS it and only
/// retires the hand-off, and the source's NEW record is its own again.  A copy rebound later
/// releases the record it displaces the same way.  The scan keeps that order
/// (`drop_transferred` is re-armed at every hand-off a statement makes and retired at an
/// unconditional reassignment), which is what lets this predicate ignore how often either
/// side is assigned.  A captured variable is left alone — the closure record shares its
/// slot.
///
/// A COMPILER BUFFER destination (`buffer_dst`: a `__ref_N` return buffer, the `__ref_p2_N`
/// a materialised branch arm is copied into) is exempt from the not-an-argument test — a
/// return buffer IS an argument (the caller's, adopted at the return), and its record is
/// released with the cascade at its own free or by the caller that adopts it.  That is how
/// `t = s; return t` releases once, in the caller.
///
/// One home for the three sites that see a whole-value copy: [`collect_drop_transferred`]
/// (the parser's `Set(v, Var(src))` and its `OpCopyRecord` into a buffer), the branch-arm
/// lift (`__lift_N = a`, built after the collector ran) and the double-move lint, so none
/// of them can disagree about which copies move the drop.
pub(crate) fn copy_moves_drop_from(
    function: &Function,
    data: &Data,
    v: u16,
    src: u16,
    buffer_dst: bool,
) -> Option<u16> {
    if v == src
        || (!buffer_dst && function.is_argument(v))
        || function.is_captured(v)
        || function.is_captured(src)
    {
        return None;
    }
    let d = function.tp(v).base().heap_def_nr()?;
    let sd = function.tp(src).base().heap_def_nr()?;
    if !data.copies_as(d, sd) || data.drop_cascade_nr(d) == u32::MAX {
        return None;
    }
    Some(if function.is_argument(src) { v } else { src })
}

/// @PLN139 stage C — the vars that HANDED OFF what they hold, so their scope end must not
/// drop it.  Two ways a value stops being its variable's to release, both an `OpCopyRecord`:
///
/// - **the source-free bit (`0x8000`)** — "deep-copy me, then FREE my source store", the
///   move a collection element-append does for a value it knows is dead after the copy.
///   The store is gone but the variable still names it, so the scope-exit drop ran the
///   author's release on a freed record — and once the slot was recycled by the next
///   allocation, on somebody else's LIVE one. Two elements closed the second's resource
///   twice and the first's never (loft#849); `LOFT_STRICT_STORES` called it a use-after-free.
/// - **a FIELD destination (`OpGetField`)** — construction copies a droppable into a
///   container, and @PLN139 makes that a MOVE: the container's copy is the owner now, and
///   the container's death releases it through the cascade. Without this the resource is
///   released twice — once by the source at its own scope end (early, while the container
///   still holds it, which is the use-after-free @PLN138 met) and once by the cascade.
///
/// Only the DROP is suppressed. The two cases differ in what happens to the store — the
/// first has already been freed, the second keeps its own copy — so the free is left to the
/// ordinary sweep, which is null-tolerant either way.
///
/// Only a plain `Var` source can be marked: any other expression names no slot that could
/// carry a scope-exit drop.
fn collect_drop_transferred(code: &Value, function: &Function, data: &Data) -> HashSet<u16> {
    let mut out: HashSet<u16> = HashSet::new();
    code.walk(&mut |n| drop_handoff_node(n, function, data, &mut out));
    out
}

/// The hand-offs ONE node makes, added to `out` — the body of [`collect_drop_transferred`],
/// which the scan re-applies statement by statement so a variable handed off AFTER a
/// reassignment retired it is armed again in scan order (the fact belongs to the
/// assignment, `@FR-O-Latest`).
fn drop_handoff_node(n: &Value, function: &Function, data: &Data, out: &mut HashSet<u16>) {
    let copy_d = data.def_nr("OpCopyRecord");
    if copy_d == u32::MAX {
        return;
    }
    {
        match n {
            Value::Call(d, args) if *d == copy_d && args.len() >= 3 => {
                // A whole-value copy into a compiler BUFFER — the per-arm `__ref_p2_N` a
                // materialised branch arm is copied into, the `__ref_N` a return delivers
                // through — is the same move as `t = s`: the buffer is freed with its cascade
                // (or adopted by a caller who runs it), so the source stops dropping.  The
                // spelling is a parser `OpCopyRecord` rather than a `Set`, which is why the
                // arm below does not see it.
                if let Some(src) = drop_bearing_source(&args[0], function)
                    && let Value::Var(dst) = args[1].unspan()
                    && function.name(*dst).starts_with("__ref")
                    && let Some(moved) = copy_moves_drop_from(function, data, *dst, src, true)
                {
                    out.insert(moved);
                    return;
                }
                let moved = matches!(args[2].unspan(), Value::Int(tp) if tp & 0x8000 != 0);
                if !moved
                    && !copy_hands_off(&args[1], function, data)
                    && !appends_to_element(&args[1], function, data)
                {
                    return;
                }
                if let Some(src) = drop_bearing_source(&args[0], function) {
                    out.insert(src);
                }
            }
            // A CONSTRUCTION block delivers its work-ref's record to the binding rather than
            // copying it, so the two then name ONE record and only the binding owns it.
            // Without this both released it: a struct-enum literal always takes the work-ref
            // path (its declared type is the enum, the constructed one the variant, so the
            // record cannot be built in place) and `w: W = WH { h: c }` cascaded twice.
            Value::Set(v, rhs) => {
                if let Some(w) = construction_work_ref(rhs, function)
                    && w != *v
                {
                    out.insert(w);
                }
                // A plain WHOLE-VALUE copy between two locals — `t = s`, `h2 = h` — moves the
                // drop to the copy; see [`copy_moves_drop_from`], which the branch-arm lift
                // reads for its `__lift_N = a` too.
                if let Value::Var(src) = rhs.unspan()
                    && let Some(moved) = copy_moves_drop_from(function, data, *v, *src, false)
                {
                    out.insert(moved);
                }
            }
            _ => {}
        }
    }
}

/// The work-ref a CONSTRUCTION block hands to its target, if that is what `rhs` is.
///
/// Deliberately not a bare `Var`: `x = y` between two locals deep-copies, so both keep their
/// own store and both must release. Only a block/insert whose tail is a work-ref delivers
/// the record itself.
fn construction_work_ref(rhs: &Value, function: &Function) -> Option<u16> {
    match rhs.unspan() {
        Value::Block(_) | Value::Insert(_) => {
            let v = drop_bearing_source(rhs, function)?;
            let n = function.name(v);
            (n.starts_with("__ref_") || n.starts_with("__rref_")).then_some(v)
        }
        _ => None,
    }
}

/// loft#890 — the argument index whose STORE `outer_call` frees WHOLE for itself, via
/// the `0x8000` source-free bit on its `const u16` type parameter.
///
/// Only `OpReplaceKeyed` answers.  Every op carrying that bit frees SOMETHING, but only
/// the keyed whole-collection replace frees a store that a `__lift_N` also owns: its
/// source is a keyed collection minted by a call, which is a store of its own.
/// `OpCopyRecord`'s move releases a RECORD inside a store whose life the append site
/// already governs (@PLN85's Join-return machinery reads that site), so answering for it
/// here would take the free away from the analysis that owns it.
fn moved_source_arg(outer_call: u32, args: &[Value], data: &Data) -> Option<usize> {
    if outer_call == u32::MAX || outer_call != data.def_nr("OpReplaceKeyed") {
        return None;
    }
    matches!(args.get(2).map(Value::unspan), Some(Value::Int(tp)) if tp & 0x8000 != 0).then_some(0)
}

/// Does a copy into `dest` hand the source's OWNERSHIP over — i.e. will something else
/// release it?  True for a PLACE reached from a root variable through field reads and
/// vector element reads, at any depth (`o.h`, `o.s.h`, `v[0].h`, `o.items[i]`), when the
/// root's type owns a droppable anywhere: its cascade recurses through fields and
/// elements, so it reaches that place.  Read one level only, `o.s = S {…}` copied into
/// the nested `o.s.h` and `v[0] = S {…}` into an element were not hand-offs, and the
/// literal's work-ref released the resource a second time beside the container's cascade.
///
/// A path through a KEYED read is never a hand-off: a keyed collection does not release
/// its records (`@FR-H-Drop-Not`), so the source keeps dropping there.
pub(crate) fn copy_hands_off(dest: &Value, function: &Function, data: &Data) -> bool {
    let get_field_d = data.def_nr("OpGetField");
    let get_vector_d = data.def_nr("OpGetVector");
    let vector_ref_d = data.def_nr("OpVectorRef");
    if get_field_d == u32::MAX {
        return false;
    }
    let mut cur = dest;
    loop {
        let Value::Call(d, args) = cur.unspan() else {
            return false;
        };
        if *d != get_field_d && *d != get_vector_d && *d != vector_ref_d {
            return false;
        }
        match args.first().map(Value::unspan) {
            Some(Value::Var(cv)) => {
                return data.type_owns_droppable_anywhere(function.tp(*cv).base());
            }
            Some(inner) => cur = inner,
            None => return false,
        }
    }
}

/// Does a copy into `dest` hand ownership to a COLLECTION element?
///
/// `_elm_N` is the element `OpNewRecord` hands back, so a copy into it is the element-append.
/// The releaser is not the element's own type but the COLLECTION's cascade, which walks every
/// element — and that loop is emitted exactly when the element type owns a droppable, so that
/// is the condition to test.
///
/// Needed beside the `0x8000` case, which only fires when the source is dead after the copy.
/// A NAMED local appended to a collection (`v: vector<H> = [h1, h2]`) stays live, so no move
/// bit is set — and once the collection releases its elements, leaving the local dropping too
/// means one resource released twice.
pub(crate) fn appends_to_element(dest: &Value, function: &Function, data: &Data) -> bool {
    let Value::Var(dv) = dest.unspan() else {
        return false;
    };
    if !function.name(*dv).starts_with("_elm_") {
        return false;
    }
    match function.tp(*dv).base() {
        Type::Reference(ed, _) | Type::Enum(ed, true, _) => data.owns_droppable(*ed),
        _ => false,
    }
}

/// The variable a copy SOURCE ultimately names, or `None` when it names no slot.
///
/// A plain `Var` is the named-local case. An `Object` construction reaches here as the block
/// that BUILDS it, whose tail is the work-ref holding the finished record — `Nest { s: S { … } }`
/// copies such a block into `Nest`'s field, and without peeling it the inner `S` temp kept a
/// scope-exit drop and released the payload a second time.
///
/// A tuple MEMBER read names a slot too, and it is the third spelling of a copy source rather
/// than a fourth kind of thing: `layout.md (L-Tuple)` makes a tuple a synthetic struct, and a
/// heap member's stack word is the handle of a work-ref the tuple's own type names
/// (`(ref(S)["__ref_1"], integer)`). So `u = t` — lowered onto the per-member copy since
/// loft#1361 — copies `t`'s member record into `u`'s, and the source it displaces is that
/// work-ref. Without this arm the copy named no slot, both members kept a scope-exit drop,
/// and one resource was released TWICE while `(B-Copy)` and `heap.md (H-Drop)` between them
/// say a copy MOVES the single release to the copy.
pub(crate) fn drop_bearing_source(src: &Value, function: &Function) -> Option<u16> {
    match src.unspan() {
        Value::Var(v) => Some(*v),
        Value::TupleGet(base, i) => tuple_member_backing(*base, *i, function),
        Value::Block(bl) => bl
            .operators
            .last()
            .and_then(|v| drop_bearing_source(v, function)),
        Value::Insert(ops) => ops.last().and_then(|v| drop_bearing_source(v, function)),
        _ => None,
    }
}

/// The work-ref backing member `i` of the tuple in `base`, or `None` when that member is not
/// a heap record — a scalar member is stored inline and has no slot of its own to release.
///
/// The tuple's TYPE is where the pairing lives, and reading it takes one step of care: the
/// dep lists are UNIONED across the tuple's heap members, so every heap element carries the
/// same list and `(WS, integer, WT)` prints as
/// `(ref(WS)["__ref_1", "__ref_2"], integer, ref(WT)["__ref_1", "__ref_2"])`. The list is in
/// member order and a scalar member contributes nothing, so the backing of member `i` is the
/// dep at the number of HEAP members before it — `__ref_2` for the `WT` above, not the
/// `__ref_1` that `first()` answers.
///
/// The count is what makes that positional read safe rather than a convention this function
/// hopes for: if the list is not exactly as long as the tuple's heap members, the order it
/// would be indexed by is not established, so this DECLINES instead of naming a work-ref it
/// guessed. Declining costs the hand-off (the pre-loft#1361 double release) and never
/// suppresses the release of a member that is still live.
fn tuple_member_backing(base: u16, i: u16, function: &Function) -> Option<u16> {
    // A PARAMETER's members are the CALLER's, and its deps are not frame variables of this
    // function at all — reading one as a local's number would suppress the release of
    // whatever local happens to wear that number.  The parameter rule is the one that
    // applies here anyway: a copy off an argument leaves the caller as the owner
    // (`copy_moves_drop_from`), which is a decision about the argument, not its member.
    if function.is_argument(base) {
        return None;
    }
    let Type::Tuple(elems) = function.tp(base).base() else {
        return None;
    };
    let elem = elems.get(i as usize)?;
    if !crate::data::is_dbref(elem.base()) {
        return None;
    }
    let deps = match elem.base() {
        Type::Reference(_, deps) | Type::Enum(_, true, deps) => deps,
        _ => return None,
    };
    let heap = |e: &Type| crate::data::is_dbref(e.base());
    if deps.len() != elems.iter().filter(|e| heap(e)).count() {
        return None;
    }
    let dep = *deps.get(elems.iter().take(i as usize).filter(|e| heap(e)).count())?;
    (dep != u16::MAX).then_some(dep)
}

/// Which DEFINITION each fn-ref variable in `code` was assigned, `u32::MAX` where the
/// assignments disagree or the target cannot be named.
///
/// `pub(crate)` because two readers need the same answer and a second spelling of it could
/// only agree by accident: the scope pass decides here whether a `CallRef` result may be
/// lifted, and the ownership oracle needs the same target to resolve that call through the
/// callee's return summary (`@FR-O-Oracle`).
pub(crate) fn collect_fnref_targets(code: &Value, function: &Function) -> HashMap<u16, u32> {
    let mut out: HashMap<u16, u32> = HashMap::new();
    code.walk(&mut |v| {
        let Value::Set(var, rhs) = v else { return };
        if !matches!(function.tp(*var).base(), Type::Function(_, _, _)) {
            return;
        }
        // Which definition this right-hand side names is read by
        // `use_analysis::fnref_target_in` — the one home, shared with
        // `Ownership::classify`'s `CallRef` arm, which resolves the same question off
        // its own `Defs` table.  `Some(u32::MAX)` is its "names two" answer and joins
        // the disagreement below rather than reading as a target.
        if let Some(d) = crate::use_analysis::fnref_target_in(rhs) {
            let slot = out.entry(*var).or_insert(d);
            if d == u32::MAX || *slot != d {
                *slot = u32::MAX;
            }
        }
    });
    out
}

/// The CALLER variables written into each fn-ref's closure record, in capture-slot order.
///
/// A capturing lambda's assignment is a BLOCK that mints the record, writes each captured
/// value into it and then yields the `FnRef` — so `OpSetDbRef(___clos_N, <slot>, <var>)`
/// already says which caller variable a capture slot holds, and nothing else has to be
/// derived to know it.
///
/// Only the `DbRef` writes are collected, and that is the question rather than a shortcut:
/// this exists to answer *"which caller store might the closure hand back?"*, and a capture
/// that is not a store cannot be handed back as one.  A scalar capture is written with
/// `OpSetInt` and correctly contributes nothing.
///
/// `pub(crate)` for the same reason as [`collect_fnref_targets`] beside it: the ownership
/// oracle needs the same answer, and a second spelling of it could only agree by accident.
pub(crate) fn collect_fnref_captures(
    code: &Value,
    function: &Function,
    data: &Data,
) -> HashMap<u16, Vec<(i32, u16)>> {
    let set_dbref = data.def_nr("OpSetDbRef");
    let mut out: HashMap<u16, Vec<(i32, u16)>> = HashMap::new();
    code.walk(&mut |v| {
        let Value::Set(var, rhs) = v else { return };
        if !matches!(function.tp(*var).base(), Type::Function(_, _, _)) {
            return;
        }
        // The closure variable this assignment builds — named by the `FnRef` it yields, so a
        // block that happens to touch another record contributes nothing.
        let mut clos: Option<u16> = None;
        rhs.walk(&mut |inner| {
            if let Value::FnRef(_, c, _) = inner {
                clos = Some(*c);
            }
        });
        let Some(clos) = clos else { return };
        let mut slots: Vec<(i32, u16)> = Vec::new();
        rhs.walk(&mut |inner| {
            let Value::Call(d, args) = inner else { return };
            if *d != set_dbref || args.len() < 3 {
                return;
            }
            let (Some(Value::Var(target)), Some(Value::Int(slot)), Some(Value::Var(src))) = (
                args.first().map(Value::unspan),
                args.get(1).map(Value::unspan),
                args.get(2).map(Value::unspan),
            ) else {
                return;
            };
            if *target == clos {
                slots.push((*slot, *src));
            }
        });
        slots.sort_by_key(|(slot, _)| *slot);
        out.insert(*var, slots);
    });
    out
}

/// The scope-exit / pre-reassignment frees for a TUPLE local's OWNED elements, in
/// reverse index order.
///
/// `Value::TupleGet(v, idx)` reads one element as a plain `DbRef` / `Str`, so each
/// free is the ordinary op on that read and needs no per-element stack-offset
/// machinery of its own.
///
/// Only an element the tuple OWNS. `owned_elements` answers on the type KIND — is
/// it heap-shaped — which a BORROWED element passes exactly as an owned one does,
/// and freeing that releases the source's store.  The empty-dep test is the
/// ownership half, the same one the scalar branch of `get_free_vars` uses.
///
/// STORE-backed elements only.  A tuple's `text` element is a `Str` VIEW — `put_text`
/// stores the pointer+length pair, never an owning `String` — so there is nothing to
/// release, and `OpFreeText` on the element reads that view as a `String` (a SIGSEGV;
/// loft#1004 was the operand arithmetic underflowing before it got that far).  The
/// bytes a tuple element views belong to whatever built them: a literal, or a `__ncc_N`
/// temp its consumer frees.
fn tuple_owned_elem_frees(
    elems: &[Type],
    v: u16,
    data: &Data,
    function: &crate::variables::Function,
) -> Vec<Value> {
    let mut out = Vec::new();
    for &(_offset, idx) in crate::data::owned_elements(elems).iter().rev() {
        // @FR-O-Proxy asks free, and @FR-O-Override vetoes it at every such site — this is one:
        // the element test concludes "this element owns its store" from an empty dep list,
        // which is @FR-O-Proxy and unsound alone.  `OpFreeRef(TupleGet(v, i))` releases
        // storage reached through `v`, so a binding the parser marked never-free is
        // never-free here too — and a tuple has no dep list of its own to say it through.
        //
        // The test reads NEGATED, guarding a `continue`, so the free is on the FALL-THROUGH
        // and the site concludes ownership exactly as a positive test would.  Reading the
        // `!` as "this asks whether it is a borrow" is what kept this site out of
        // `scripts/o_proxy_check.py`'s obligation set entirely.
        if function.is_skip_free(v)
            || !elems[idx].depend().is_empty()
            || matches!(elems[idx].base(), Type::Text(_))
        {
            continue;
        }
        out.push(Value::Call(
            data.def_nr("OpFreeRef"),
            vec![Value::TupleGet(v, idx as u16)],
        ));
    }
    out
}
fn run_scan_phase(
    data: &mut Data,
    database: &mut crate::database::Stores,
    d_nr: u32,
    orig_code: &Value,
    orig_vars: &Function,
    confined: &HashMap<u16, u16>,
) {
    // @PLN85 `local_source` over-free fix (gated): the heap slots whose OWNED store
    // is displaced by a later borrow/join reassignment. Computed on the pre-scope
    // code so the dep classification is read before any dep-strip below mutates it.
    let displaced_owned = if crate::keys::join_own_enabled() {
        crate::use_analysis::displaced_owned_slots(orig_code, orig_vars, data)
    } else {
        HashSet::new()
    };
    // Computed BEFORE the struct takes its `&mut` on the store: the walk reads the store to
    // convert a field NUMBER into the byte OFFSET a view carries, and the two borrows cannot
    // overlap inside one initialiser.
    let views_to_materialise = collect_views_to_materialise(orig_code, orig_vars, data, database);
    let mut scopes = Scopes {
        database,
        d_nr,
        max_scope: 1,
        scope: 0,
        stack: Vec::new(),
        var_scope: BTreeMap::new(),
        var_order: Vec::new(),
        var_mapping: HashMap::new(),
        confined: confined.clone(),
        loops: vec![],
        scan_depth: 0,
        lift_counter: 0,
        lift_vars: Vec::new(),
        lift_texts: Vec::new(),
        ret_temp_counter: 0,
        paired_witness: HashMap::new(),
        literal_buffer: HashMap::new(),
        lift_join_witness: HashMap::new(),
        pending_join_witness: std::cell::Cell::new(u16::MAX),
        multi_assigned: multi_assigned_in(orig_code),
        capture_build_backing: capture_build_backings(data, orig_vars, orig_code),
        lift_decl_depth: HashMap::new(),
        callref_join_bases: callref_join_bases_in(orig_code, data, d_nr),
        snapshot_witness: HashMap::new(),
        witness_buffer: HashMap::new(),
        owned_refs: HashMap::new(),
        rbuf_witness: None,
        local_owns: HashMap::new(),
        owner_witness: HashMap::new(),
        displaced_owned,
        views_to_materialise,
        fnref_target: collect_fnref_targets(orig_code, orig_vars),
        drop_transferred: collect_drop_transferred(orig_code, orig_vars, data),
        free_transferred: HashSet::new(),
        fn_defs: None,
    };
    let mut function = Function::copy(orig_vars);
    for a in function.arguments() {
        scopes.var_scope.insert(a, 0);
    }
    // loft#1128 — mint the return-buffer's runtime ownership witness BEFORE the scan, so the
    // first assignment that has to maintain it already has a flag to write.  Only for a body
    // that actually reaches a displacing site: everywhere else the static `owned_refs` answer
    // is complete and the slot would be dead weight.
    if let Some(buf) = hidden_return_buffer_var(d_nr, &function, data)
        && displaces_return_buffer(orig_code, buf, data)
    {
        let name = format!("__rbo_{}", function.name(buf));
        let flag = function.add_temp_var(&name, &Type::Boolean);
        scopes.var_scope.insert(flag, 0);
        scopes.var_order.push(flag);
        scopes.rbuf_witness = Some((buf, flag));
    }
    // loft#1200 — the same construction one scope in: a boolean per nullable heap-record LOCAL
    // that a minting call reassigns, recording whether the store it holds is this frame's sole
    // property.  Minted BEFORE the scan for the reason the buffer's is: the first assignment
    // that has to maintain it already needs a flag to write.
    for v in mixed_ownership_locals(orig_code, &function, data, d_nr) {
        function.mark_borrow_arm(v);
    }
    // loft#1336 / @FR-O-Witness — the OWNER WITNESS of a local whose assignments MIX
    // ownership.  Minted before the scan for the same reason the two flags above are: the
    // first assignment that has to maintain it already needs somewhere to write.  The local
    // is marked never-free (@FR-O-Override) here, so every static free site — the pre-`Set`
    // free, the transition frees, the scope-exit sweep — declines it and the witness is the
    // ONE thing that releases its stores.  Minted BEFORE the loft#1200 displacement flags
    // below for the same reason: `nullable_locals_that_displace` excludes a never-free local,
    // so a witnessed local is never also given a `__lbo_` flag whose guarded free the codegen
    // veto would drop anyway — one release mechanism per local, and no dead free in the IR.  The witness carries a self-dep: not a borrow, and
    // not the empty list @FR-O-Proxy reads as "owner", so no site frees it on its own.
    let witness_locals = owner_witness_locals(
        orig_code,
        &function,
        data,
        d_nr,
        &scopes.views_to_materialise,
    );
    for &v in &witness_locals {
        if !crate::keys::owner_witness_enabled() {
            break;
        }
        let Some(record) = function.tp(v).base().heap_def_nr() else {
            continue;
        };
        let name = format!("__own_{}", function.name(v));
        let w = function.add_temp_var(&name, &Type::Reference(record, Deps::none()));
        function.depend(w, w);
        function.set_skip_free(v);
        function.set_owner_witness(v, w);
        scopes.var_scope.insert(w, 0);
        scopes.var_order.push(w);
        scopes.owner_witness.insert(v, w);
    }
    let displace_locals = nullable_locals_that_displace(orig_code, &function, data);
    for &v in &displace_locals {
        let name = format!("__lbo_{}", function.name(v));
        let flag = function.add_temp_var(&name, &Type::Boolean);
        scopes.var_scope.insert(flag, 0);
        scopes.var_order.push(flag);
        scopes.local_owns.insert(v, flag);
    }
    // A nullable heap local that holds a PROJECTION VIEW owns no store it must free (a view
    // is never owned, @FR-O-Owner).  Mark it never-free (@FR-O-Override) so the D-own-16
    // `borrows_one_argument` residual — which reads its single-ARGUMENT dep as ownership —
    // does not free the viewed store it displaces at a reassignment (the caller's nested
    // record, or a local's field).  The two mixed-ownership shapes that DO own a store are
    // excluded: a solely-owned minting call keeps its loft#1200 runtime flag, and a view+mint
    // mix its owner witness (loft#1336).  What remains frees nothing of its own, so this
    // leaks nothing.
    for v in nullable_view_locals(orig_code, &function, data) {
        if !displace_locals.contains(&v)
            && !witness_locals.contains(&v)
            && !scopes.views_to_materialise.contains_key(&v)
        {
            function.set_skip_free(v);
        }
    }
    let mut code = scopes.scan(orig_code, &mut function, data);
    // The witness starts FALSE: on entry the buffer holds the CALLER's store, which this
    // function must never release.  A transition site is reachable with no prior assignment at
    // all (`fn g() -> Res { mk(2) }`), and `needs_pre_init` does not cover `boolean`, so an
    // uninitialised slot would read as garbage and free the caller's buffer.
    if let Some((_, flag)) = scopes.rbuf_witness
        && let Value::Block(bl) = &mut code
    {
        bl.operators.insert(0, v_set(flag, Value::Boolean(false)));
    }
    // Every per-local witness starts FALSE for the same reason: before the local's first
    // assignment there is no store of its own to release, and an uninitialised boolean slot
    // would read as garbage and free one.
    if !scopes.local_owns.is_empty()
        && let Value::Block(bl) = &mut code
    {
        let mut flags: Vec<u16> = scopes.local_owns.values().copied().collect();
        flags.sort_unstable();
        for flag in flags.into_iter().rev() {
            bl.operators.insert(0, v_set(flag, Value::Boolean(false)));
        }
    }
    // An owner witness starts at the null SENTINEL (`store_nr == u16::MAX`): before the
    // local's first owning assignment there is no store of its own to release, and
    // `OpFreeRef` of the sentinel is a no-op (@FR-H-FreeNull).  Spelled as the sentinel
    // call and not as `null`, because a heap local's `= null` lowers to `OpInitRef` — a
    // stack-record placeholder the allocator is expected to replace — and a free of THAT is
    // the `#306` refusal.
    if !scopes.owner_witness.is_empty()
        && let Value::Block(bl) = &mut code
    {
        let mut witnesses: Vec<u16> = scopes.owner_witness.values().copied().collect();
        witnesses.sort_unstable();
        witnesses.dedup();
        for w in witnesses.into_iter().rev() {
            bl.operators.insert(
                0,
                v_set(w, Value::Call(data.def_nr("OpNullRefSentinel"), vec![])),
            );
        }
    }
    // lift vars from `scan_args` are assigned inside conditional branches but
    // their `OpFreeRef` lives at function exit; prepend the null-inits so codegen
    // reserves their slot along every path (see the original comment in check).
    if !scopes.lift_vars.is_empty()
        && let Value::Block(bl) = &mut code
    {
        for &v in scopes.lift_vars.iter().rev() {
            bl.operators.insert(0, v_set(v, Value::Null));
        }
    }
    if !scopes.lift_texts.is_empty()
        && let Value::Block(bl) = &mut code
    {
        for &v in scopes.lift_texts.iter().rev() {
            bl.operators.insert(0, v_set(v, Value::Text(String::new())));
        }
    }
    data.definitions[d_nr as usize].code = code;
    data.definitions[d_nr as usize].variables = function;
    #[cfg(debug_assertions)]
    check_ref_leaks(
        &data.definitions[d_nr as usize].code,
        &data.definitions[d_nr as usize].variables,
        data,
        &data.definitions[d_nr as usize].name.clone(),
        &data.definitions[d_nr as usize].returned.clone(),
        &scopes.var_scope,
    );
    #[cfg(debug_assertions)]
    check_arg_ref_allocs(
        &data.definitions[d_nr as usize].code,
        &data.definitions[d_nr as usize].variables,
        &data.definitions[d_nr as usize].name.clone(),
    );
    #[cfg(debug_assertions)]
    check_text_return(
        &data.definitions[d_nr as usize].code,
        &data.definitions[d_nr as usize].variables,
        &data.definitions[d_nr as usize].name.clone(),
        &data.definitions[d_nr as usize].returned.clone(),
        data,
    );
    for (v_nr, scope) in scopes.var_scope {
        data.definitions[d_nr as usize]
            .variables
            .set_scope(v_nr, scope);
    }
}

/// True if `op` is the null-init `Set(vdb, Null)` — a work-ref's `first_def`.
fn is_var_null_init(op: &Value, vdb: u16) -> bool {
    matches!(op.unspan(), Value::Set(v, val) if *v == vdb && matches!(val.unspan(), Value::Null))
}

/// Prepend `ni` to the operators of the Block whose `scope == target`, descending
/// only the control-flow spine (`Block`/`Insert`/`If`/`Span`/`Return`).  Returns
/// `None` once inserted, or `Some(ni)` (un-consumed) if no such block was reached.
/// A confined block nested inside a `Set`/`Call`/`Iter` value (a `map`/`filter`
/// body, a short-lambda capture) is deliberately NOT entered: the Plan-57 null-init
/// relocation is a best-effort watermark optimization, and such a block keeps its
/// body-0 null-init (the caller's fallback), which is leak/poison-clean.  Widening
/// the descent to every child would relocate into far more functions for marginal
/// benefit — out of scope here; the concern is only to stop the false-positive
/// debug assert on the (correct) un-reached case.
fn prepend_to_scope(node: &mut Value, target: u16, ni: Value) -> Option<Value> {
    match node {
        Value::Block(bl) if bl.scope == target => {
            bl.operators.insert(0, ni);
            None
        }
        Value::Block(bl) | Value::Loop(bl) => {
            let mut carry = Some(ni);
            for op in &mut bl.operators {
                carry = prepend_to_scope(op, target, carry.take().unwrap());
                carry.as_ref()?;
            }
            carry
        }
        Value::Insert(ls) => {
            let mut carry = Some(ni);
            for op in ls {
                carry = prepend_to_scope(op, target, carry.take().unwrap());
                carry.as_ref()?;
            }
            carry
        }
        Value::If(c, t, e) => {
            let ni = prepend_to_scope(c, target, ni)?;
            let ni = prepend_to_scope(t, target, ni)?;
            prepend_to_scope(e, target, ni)
        }
        Value::Span(b) => prepend_to_scope(&mut b.1, target, ni),
        // This relocation is BEST-EFFORT: where no arm reaches, the fallback — leaving the
        // null-init at body position 0 — is correct, so an unreached shape costs placement
        // quality and never correctness.  A confined block off the control-flow spine (a
        // `map`/`filter`/lambda body) is the standing example.
        Value::Return(b) | Value::Drop(b) | Value::Yield(b) => prepend_to_scope(b, target, ni),
        _ => Some(ni),
    }
}

/// Plan-57 cluster-I experiment: move a confined `__vdb`'s null-init
/// `Set(vdb, Null)` from body position 0 into its confined block, so the slot's
/// `first_def` (and therefore the SLOT, and codegen's free) live in the block —
/// not just the IR `OpFreeRef`.  The body-0 hoist (`parse_code`) was a
/// correctness over-reach made *without* lifetime info; the confinement analysis
/// now supplies that info, so the slot can live in its real scope.
fn relocate_null_init(code: &mut Value, vdb: u16, block_scope: u16) -> bool {
    let ni = {
        let Value::Block(body) = code else {
            return false;
        };
        let Some(pos) = body
            .operators
            .iter()
            .position(|op| is_var_null_init(op, vdb))
        else {
            return false;
        };
        body.operators.remove(pos)
    };
    if let Some(ni) = prepend_to_scope(code, block_scope, ni) {
        // Not reached by the control-flow descent — restore the null-init so the
        // `first_def` is never lost, and skip the (best-effort) relocation: the
        // store keeps its body-0 `first_def` and is still freed by the confined
        // block's scope-exit sweep (verified leak/poison-clean, both backends).
        // The confined block can legitimately live inside a `map`/`filter` body or
        // a short-lambda capture (a `Call`/`Iter`/`Set` value `prepend_to_scope`
        // does not enter — 501, 85-short-lambda-capture).  Only a `block_scope`
        // that is ABSENT FROM THE IR ENTIRELY is a `store_confinement` bug worth
        // asserting; a present-but-unreached scope is the expected miss.
        if let Value::Block(body) = code {
            body.operators.insert(0, ni);
        }
        debug_assert!(
            block_scope_present(code, block_scope),
            "relocate_null_init: block scope {block_scope} is not in the IR at all \
             (a store_confinement bug, not merely an unreached inline block)"
        );
        false
    } else {
        true
    }
}

/// True if any `Block`/`Loop` anywhere in `node`'s subtree has `scope == target`.
/// Unlike [`prepend_to_scope`], this descends into EVERY child (`Value::walk`), so
/// it distinguishes a scope that is genuinely absent from one that merely lives off
/// the control-flow spine (inside a `map`/`filter` body, a lambda capture).  Only
/// consulted from the `relocate_null_init` `debug_assert!`, but must compile in
/// release too (the assert's argument is still type-checked there).
fn block_scope_present(node: &Value, target: u16) -> bool {
    let mut found = false;
    node.walk(&mut |n| {
        if let Value::Block(bl) | Value::Loop(bl) = n
            && bl.scope == target
        {
            found = true;
        }
    });
    found
}

/// EXPERIMENTAL (LOFT_BORROW_ELIDE) — inline the Tier-0 Borrow-verdict vector
/// copies: for each elidable `v = copy(s.f)`, replace every read of `v` with the
/// source field-access `s.f` and drop the copy idiom (the `vdb` buffer's alloc /
/// length-set / append / free, and `v`'s defining `Set`). The result is the IR a
/// direct `s.f[i]` program compiles — which already aliases with no copy — so it
/// needs no borrow-dep/skip_free surgery. Runs before the scope/free passes.
fn elide_borrows(data: &mut Data) {
    let op_database = data.def_nr("OpDatabase");
    let op_append = data.def_nr("OpAppendVector");
    let op_set_int4 = data.def_nr("OpSetInt4");
    let op_set_int = data.def_nr("OpSetInt");
    let op_free = data.def_nr("OpFreeRef");

    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let plans = crate::use_analysis::elision_plans(
            &data.def(d_nr).code,
            &data.def(d_nr).variables,
            data,
        );
        if plans.is_empty() {
            continue;
        }
        // A var that BORROWS `v` (`e = v[i]`, `deps` ∋ `v`) is left with a stale dep
        // once `v` is deleted, so the borrowed-view codegen (`stack(dep[0])`) would
        // dereference a dead slot. `use_analysis` only emits a plan when every such
        // borrower is read-only/non-escaping, and lists them; RE-POINT each borrower
        // `v → source_base` so it borrows the live source element instead (codegen
        // then reads the source param's valid slot). This is what lets the
        // borrowed-element accessors elide rather than fall back to copy.
        let mut elide_v: HashMap<u16, Value> = HashMap::new();
        let mut elide_vdb: HashSet<u16> = HashSet::new();
        for p in plans {
            for &e in &p.borrowers {
                data.definitions[d_nr as usize]
                    .variables
                    .make_independent(e, p.var);
                data.definitions[d_nr as usize]
                    .variables
                    .depend(e, p.source_base);
            }
            elide_v.insert(p.var, p.source);
            elide_vdb.insert(p.vdb);
        }
        let ops = ElideOps {
            op_database,
            op_append,
            op_set_int4,
            op_set_int,
            op_free,
        };
        let mut code = data.def(d_nr).code.clone();
        elide_rewrite(&mut code, &elide_v, &elide_vdb, &ops);
        data.definitions[d_nr as usize].code = code;
    }
}

// The `op_` prefix is meaningful (these are operator def-numbers), so the
// same-prefix style lint does not apply.
#[allow(clippy::struct_field_names)]
struct ElideOps {
    op_database: u32,
    op_append: u32,
    op_set_int4: u32,
    op_set_int: u32,
    op_free: u32,
}

/// Is `stmt` a copy-idiom statement for an elided binding (to be dropped)?
fn idiom_drop(
    stmt: &Value,
    elide_v: &HashMap<u16, Value>,
    elide_vdb: &HashSet<u16>,
    o: &ElideOps,
) -> bool {
    match stmt.unspan() {
        // `v = OpGetField(vdb,…)` (the def) and `vdb = null` (the init).
        Value::Set(v, _) => elide_v.contains_key(v) || elide_vdb.contains(v),
        Value::Call(d, args) => {
            let target = match args.first().map(Value::unspan) {
                Some(Value::Var(t)) => Some(*t),
                _ => None,
            };
            if *d == o.op_database || *d == o.op_set_int4 || *d == o.op_set_int {
                target.is_some_and(|t| elide_vdb.contains(&t)) // buffer alloc / length-set
            } else if *d == o.op_append {
                target.is_some_and(|t| elide_v.contains_key(&t)) // copy-fill
            } else if *d == o.op_free {
                target.is_some_and(|t| elide_vdb.contains(&t) || elide_v.contains_key(&t))
            } else {
                false
            }
        }
        _ => false,
    }
}

fn elide_rewrite(
    node: &mut Value,
    elide_v: &HashMap<u16, Value>,
    elide_vdb: &HashSet<u16>,
    o: &ElideOps,
) {
    // Inline a read of an elided var with its source field-access.
    let replacement = match node.unspan() {
        Value::Var(v) => elide_v.get(v).cloned(),
        _ => None,
    };
    if let Some(src) = replacement {
        *node = src;
        return;
    }
    match node {
        Value::Block(b) => {
            b.operators
                .retain(|s| !idiom_drop(s, elide_v, elide_vdb, o));
            for op in &mut b.operators {
                elide_rewrite(op, elide_v, elide_vdb, o);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !idiom_drop(s, elide_v, elide_vdb, o));
            for op in &mut *ops {
                elide_rewrite(op, elide_v, elide_vdb, o);
            }
        }
        _ => node.for_each_child_mut(&mut |c| elide_rewrite(c, elide_v, elide_vdb, o)),
    }
}

// The `op_` prefix is meaningful (operator def-numbers), so the same-prefix lint does not apply.
#[allow(clippy::struct_field_names)]
struct MoveOps {
    op_database: u32,
    op_copy_record: u32,
    op_free: u32,
    op_new_record: u32,
    op_get_field: u32,
    op_get_vector: u32,
}

/// @PLN90 phase B (B1.3) — the last-use MOVE-elision rewrite for the RECORD copy shape
/// (`v[i] = e` / `o.f = src`, lowered as `OpCopyRecord`). When a source `s`'s store is dead
/// after the copy (a [`crate::use_analysis::MovePlan`] with `kind == Record`), build `s`'s
/// fields DIRECTLY into the copy's destination slot instead of constructing a throwaway record
/// and deep-copying it:
///
/// ```text
///   OpDatabase(s)                           (dropped)
///   OpSetInt (s, off, v)  ── retarget ──▶  OpSetInt (dest, off, v)
///   OpSetText(s, off, v)  ── retarget ──▶  OpSetText(dest, off, v)
///   OpCopyRecord(s, dest)                   (dropped — the deep copy is gone)
///   OpFreeRef(s)                            (dropped — no throwaway store to free)
/// ```
///
/// where `dest` is the `OpCopyRecord` destination expression (`OpGetVector(v,…)` / `OpGetField`).
/// Emits NO new op — both backends already lower a retargeted `OpSet*`, so the rewrite is
/// backend-agnostic (one IR pass, like `elide_borrows`). DEFAULT ON (B1.5); `LOFT_NO_MOVE_ELIDE`
/// restores the copy. The CONSTRUCT shape is handled by [`construct_move_rewrite`] (B1.3b) for
/// the reorder-free field-append case; fresh construction (`a = Bag { items: base }`, container
/// built after the source) still needs a build-order reorder and stays a copy. Design:
/// `doc/claude/plans/90-copy-diagnostics/phase-b-design.md`.
fn move_elide(data: &mut Data) {
    if !crate::keys::move_elide_enabled() {
        return;
    }
    let mo = MoveOps {
        op_database: data.def_nr("OpDatabase"),
        op_copy_record: data.def_nr("OpCopyRecord"),
        op_free: data.def_nr("OpFreeRef"),
        op_new_record: data.def_nr("OpNewRecord"),
        op_get_field: data.def_nr("OpGetField"),
        op_get_vector: data.def_nr("OpGetVector"),
    };
    let co = ConstructOps {
        op_database: data.def_nr("OpDatabase"),
        op_free: data.def_nr("OpFreeRef"),
        op_append: data.def_nr("OpAppendVector"),
        op_prealloc: data.def_nr("OpPreAllocVector"),
        op_set_int4: data.def_nr("OpSetInt4"),
        op_get_field: data.def_nr("OpGetField"),
        op_clear: data.def_nr("OpClearVector"),
        op_new_record: data.def_nr("OpNewRecord"),
        op_finish_record: data.def_nr("OpFinishRecord"),
    };
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        // Plan-based rewrites (Record / Construct) key off these; the structural `a.field = base`
        // rewrite (B1.3d) does not — so we do NOT early-continue on an empty plan set.
        let plans = crate::use_analysis::move_plans(data, d_nr);
        let mut code = data.def(d_nr).code.clone();
        // Vars to suppress the (later `variables()`-emitted) scope-exit free for — the moved-out
        // owned store now lives in the destination, so its null slot must not be freed.
        let mut skip: HashSet<u16> = HashSet::new();
        // Containers a rewrite must NOT retarget a source's build into — NOT a stable, pre-existing,
        // single-def owned slot. Two producers, unioned; every rewrite consults the result:
        //  - transient element slots (`_elm_N = OpNewRecord(…)`, reused across a vector literal /
        //    nested construction) — a fresh record defined LATER (use-before-def);
        //  - vars allocated MORE THAN ONCE — REASSIGNED (`b = Bag{…}; … b = Bag{…}`) — the container
        //    has a prior store the reorder's hoist does not retire.
        let mut bad_containers = collect_element_vars(&code, &mo);
        bad_containers.extend(collect_multi_database(&code, &mo));
        // First-def order per var — a Record destination's container must be defined BEFORE its
        // source (else the retargeted build writes into an un-allocated container).
        let def_order = collect_def_order(&code, &mo);

        // ── RECORD shape (`v[i]=e` / `o.f=src`, OpCopyRecord) ──
        let rec_sources: HashSet<u16> = plans
            .iter()
            .filter(|p| p.kind == crate::use_analysis::MoveKind::Record)
            .map(|p| p.source)
            .collect();
        if !rec_sources.is_empty() {
            // Pass 1 — capture each source's UNIQUE copy destination. A source seen copying into
            // two different places is not the clean dead-after shape the plan assumes: skip it.
            let mut dest: HashMap<u16, Value> = HashMap::new();
            let mut ambiguous: HashSet<u16> = HashSet::new();
            collect_move_dest(
                &code,
                &mo,
                &data.def(d_nr).variables,
                &rec_sources,
                &bad_containers,
                &def_order,
                &mut dest,
                &mut ambiguous,
            );
            // A destination TOUCHED between the source's build and the copy cannot take the
            // retarget: the write would move ahead of that access.  See
            // `collect_move_disturbed`.
            let mut disturbed: HashSet<u16> = HashSet::new();
            collect_move_disturbed(&code, &mo, mo.op_copy_record, 0, &dest, &mut disturbed);
            let ready: HashSet<u16> = dest
                .keys()
                .copied()
                .filter(|s| !ambiguous.contains(s) && !disturbed.contains(s))
                .collect();
            if !ready.is_empty() {
                move_rewrite(&mut code, &ready, &dest, &mo);
                skip.extend(&ready);
            }
        }

        // ── CONSTRUCT shape (`x.field += src` field-append, OpAppendVector) — REORDER-FREE only ──
        let con_sources: HashSet<u16> = plans
            .iter()
            .filter(|p| p.kind == crate::use_analysis::MoveKind::Construct)
            .map(|p| p.source)
            .collect();
        if !con_sources.is_empty() {
            // Sources that are USED outside their own construction + the single copy — read between
            // being built and being moved (`out=[]; for{ out+=[…] }; assert("{out:j}"); w={items:out}`
            // — `out` is read by the assert BEFORE the move). Building such a source directly into the
            // destination would leave that intermediate read seeing the un-built source, so leave it
            // a copy. (Also subsumes append-grown sources: `v += w` is `v` at arg0 of a non-write op.)
            let escaping: HashSet<u16> = con_sources
                .iter()
                .copied()
                .filter(|&s| source_escapes(&code, s, &co))
                .collect();
            // B1.3b — reorder-free field-appends (`x.field += src`, container already exists).
            let mut moved_into: HashMap<u16, u16> = HashMap::new();
            construct_move_rewrite(
                &mut code,
                &con_sources,
                &co,
                &mo,
                &bad_containers,
                &escaping,
                &mut skip,
                &mut moved_into,
            );
            // A retargeted source is ERASED: its wrapper alloc, its view-def and the append are
            // all gone, so nothing writes it any more.  What still names it is the `deps` of the
            // element work-refs whose builds were just re-pointed — and a dep is the statement
            // "my store belongs to that variable", which after the retarget belongs to the
            // CONTAINER instead.  Left stale it is wrong twice over: the ownership derivation
            // reads a var that owns nothing (@FR-O-Deps — every store-lifetime
            // decision reads this one fact), and the scope pass declares the dep var so a borrower
            // can name it, which hands the erased local a stack slot no instruction ever writes.
            // That slot is what @PLN120 A's store-span check reports (loft#1241): the local is
            // not merely unrecorded, it is not there.  Re-pointing states the fact the rewrite
            // created rather than exempting the symptom by name.
            for (&src, &cvar) in &moved_into {
                let vars = &mut data.definitions[d_nr as usize].variables;
                for v in 0..vars.next_var() {
                    if v != src && vars.tp(v).depend().contains(&src) {
                        vars.make_independent(v, src);
                        vars.depend(v, cvar);
                    }
                }
            }
            // B1.3c — fresh construction (`a = Bag { items: base }`, container built after the
            // source): hoist `a`'s alloc, then retarget. Runs on the copies B1.3b left standing.
            // B1.4 — the interprocedural mutation set (`find_written_vars` knows which callees
            // mutate a `&`-param in ANY arg position), so a param used as a hoisted field value is
            // allowed only if genuinely never mutated.
            let mut written: HashSet<u16> = HashSet::new();
            crate::parser::find_written_vars(
                &data.def(d_nr).code,
                data,
                &mut written,
                &mut HashMap::new(),
            );
            construct_fresh_rewrite(
                &mut code,
                &con_sources,
                &co,
                &data.def(d_nr).variables,
                &written,
                &bad_containers,
                &escaping,
                &mut skip,
            );
        }

        // B1.3d — the `a.field = base` whole-vector replacement (the `__p154_rhs` idiom): a DOUBLE
        // copy (`base → __p154_rhs → a.field`, with an `OpClearVector`). Its source is a temp, so it
        // is NOT a MovePlan — this rewrite is STRUCTURAL and must run regardless of `con_sources`.
        construct_replace_rewrite(&mut code, &co, &mut skip);

        data.definitions[d_nr as usize].code = code;
        for &s in &skip {
            data.definitions[d_nr as usize].variables.set_skip_free(s);
        }
    }
}

/// Pass 1 of [`move_elide`]: map each move source to the `OpCopyRecord` destination it copies
/// into; a source seen with a second destination is marked ambiguous (and skipped).
#[allow(clippy::too_many_arguments)]
fn collect_move_dest(
    node: &Value,
    mo: &MoveOps,
    function: &Function,
    sources: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    def_order: &HashMap<u16, usize>,
    dest: &mut HashMap<u16, Value>,
    ambiguous: &mut HashSet<u16>,
) {
    if let Value::Call(d, args) = node.unspan()
        && *d == mo.op_copy_record
        && let Some(Value::Var(s)) = args.first().map(Value::unspan)
        && sources.contains(s)
        && args.len() >= 2
    {
        // A stable in-place target is a slot EXPRESSION over a PRE-EXISTING container
        // (`OpGetVector(v,…)` / `OpGetField(o,…)`). NOT stable, and skipped:
        //  - a bare `Var` dest (`x = e`) — no proof it pre-exists the source;
        //  - a dest based on a FRESH `OpNewRecord` element (`OpGetField(_elm_N,…)` nested
        //    construction) — defined AFTER the source (native `var__elm_N` not in scope);
        //  - a dest based on ANY compiler TEMP (`_`-prefixed: `__ref_N` field-iteration refs,
        //    `_slice_*`, …) — these are populated by machinery whose validity point the structural
        //    rewrite can't prove. Only a USER-named container is a proven-stable target.
        let unstable = match args[1].unspan() {
            Value::Var(_) => true,
            _ => base_var_of(&args[1], mo).is_none_or(|base| {
                bad_containers.contains(&base)
                    || function.name(base).starts_with('_')
                    // the container must be DEFINED before the source is built (else the retargeted
                    // build writes into an un-allocated slot).
                    || def_order
                        .get(&base)
                        .zip(def_order.get(s))
                        .is_none_or(|(&bd, &sd)| bd >= sd)
            }),
        };
        if unstable || dest.contains_key(s) {
            ambiguous.insert(*s);
        } else {
            dest.insert(*s, args[1].clone());
        }
    }
    node.for_each_child(&mut |c| {
        collect_move_dest(
            c,
            mo,
            function,
            sources,
            bad_containers,
            def_order,
            dest,
            ambiguous,
        );
    });
}

/// Does `node` contain a `Value::Loop` anywhere inside it?
///
/// The `for` lowering wraps its loop in a block (`{#For block … loop {#For loop …} }`), so the
/// statement an enclosing block holds is a `Block`, not the `Loop` itself.
/// Does `node` contain a fn-ref call anywhere?  The cheap structural gate on
/// [`mixed_ownership_locals`]'s oracle walk — the fact it computes is read only where a
/// `CallRef` delivers a collection.
fn contains_callref(node: &Value) -> bool {
    if matches!(node.unspan(), Value::CallRef(_, _)) {
        return true;
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && contains_callref(c) {
            found = true;
        }
    });
    found
}

fn contains_loop(node: &Value) -> bool {
    if matches!(node.unspan(), Value::Loop(_)) {
        return true;
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && contains_loop(c) {
            found = true;
        }
    });
    found
}

/// Every variable assigned inside a LOOP BODY reachable from `node` — the `Loop`-recursing
/// twin of `find_first_ref_vars`, which deliberately does not descend into a loop (loft#1156).
///
/// ⚠ It descends to the `Value::Loop` FIRST and only then collects.  Walking the whole
/// statement instead collects the statement's own top-level `Set` — and a statement whose
/// value happens to CONTAIN a loop (`test_value = { … for … { } … }`, the shape `expr!`
/// wraps every snippet in) then hoisted the destination of the assignment being scanned.
fn collect_loop_body_sets(node: &Value, mapping: &HashMap<u16, u16>, out: &mut Vec<u16>) {
    if let Value::Loop(b) = node.unspan() {
        for op in &b.operators {
            collect_sets_in(op, mapping, out);
        }
        return;
    }
    node.for_each_child(&mut |c| collect_loop_body_sets(c, mapping, out));
}

/// For every `Set(v, CallRef)` in `node`, the `Join` base the oracle names for it, grouped by
/// `v`.  Sets whose value is not a nameable `Join` contribute nothing.
fn callref_join_bases_in(node: &Value, data: &Data, d_nr: u32) -> HashMap<u16, HashSet<u16>> {
    fn walk(node: &Value, data: &Data, d_nr: u32, out: &mut HashMap<u16, HashSet<u16>>) {
        if let Value::Set(v, rhs) = node.unspan()
            && matches!(rhs.unspan(), Value::CallRef(_, _))
            && let crate::use_analysis::Own::Join { base } =
                crate::use_analysis::ownership_of(data, d_nr, rhs)
            && base != u16::MAX
        {
            out.entry(*v).or_default().insert(base);
        }
        node.for_each_child(&mut |c| walk(c, data, d_nr, out));
    }
    let mut out = HashMap::new();
    walk(node, data, d_nr, &mut out);
    out
}

/// Variables that are the target of two or more `Set` nodes anywhere in `node`.
pub(crate) fn multi_assigned_in(node: &Value) -> HashSet<u16> {
    fn count(node: &Value, out: &mut HashMap<u16, usize>) {
        if let Value::Set(v, _) = node.unspan() {
            *out.entry(*v).or_insert(0) += 1;
        }
        node.for_each_child(&mut |c| count(c, out));
    }
    let mut counts = HashMap::new();
    count(node, &mut counts);
    counts
        .into_iter()
        .filter(|&(_, n)| n >= 2)
        .map(|(v, _)| v)
        .collect()
}

/// Every variable assigned at any depth inside `node`.
fn collect_sets_in(node: &Value, mapping: &HashMap<u16, u16>, out: &mut Vec<u16>) {
    if let Value::Set(v, _) = node.unspan() {
        let resolved = *mapping.get(v).unwrap_or(v);
        if !out.contains(&resolved) {
            out.push(resolved);
        }
    }
    node.for_each_child(&mut |c| collect_sets_in(c, mapping, out));
}

/// What a value-branch arm tail is rewritten into ([`Scopes::arm_bind`]).
enum ArmBind {
    /// `{ __lift_N = <tail>; __lift_N }` — a temp of this type, bound by the single-bind
    /// lowering of the tail (adopt, copy, or `OpBindOrCopy`, whichever that bind does).
    Bind(Type),
    /// `{ OpReplaceVector(__lift_N, <var>, elem); __lift_N }` — a function-scoped vector
    /// buffer refilled from the variable, the copy `@FR-B-Copy` asks of a plain bind.
    CopyVector { tp: Type, elem: i32 },
}

/// Does `node` mention variable `v` outside the binds of the temps in `lifted` — i.e. is `v`
/// still read by an arm that was NOT copied into one of them?
fn mentions_var_outside(node: &Value, v: u16, lifted: &[u16]) -> bool {
    match node.unspan() {
        Value::Set(t, _) if lifted.contains(t) => return false,
        Value::Call(_, args) if matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if lifted.contains(t)) =>
        {
            return false;
        }
        Value::Var(x) if *x == v => return true,
        _ => {}
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && mentions_var_outside(c, v, lifted) {
            found = true;
        }
    });
    found
}

/// Does `node` mention variable `v` anywhere inside it?
fn mentions_var(node: &Value, v: u16) -> bool {
    if matches!(node.unspan(), Value::Var(x) if *x == v) {
        return true;
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && mentions_var(c, v) {
            found = true;
        }
    });
    found
}

/// Whether the first use of a variable in execution order READS it or WRITES it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FirstUse {
    Read,
    Write,
}

/// The first use of `v` in `node`, in execution order.
///
/// This is the liveness question [`Scopes::loop_locals_read_after`] asks: a later region that
/// ASSIGNS `v` before reading it is a fresh binding that happens to share a name, not a reader
/// of the value the loop produced.  [`mentions_var`] cannot tell those apart — it answers
/// "does `v` appear here", so an independent binding reads as a use of the loop's value.
///
/// Two orderings carry the meaning.  A `Set`'s VALUE is evaluated before its target is
/// written, so `r = f(r)` reads first.  A `Loop` body may run ZERO times, so a write inside it
/// never kills the binding — only a read there establishes liveness.
fn first_use_of(node: &Value, v: u16) -> Option<FirstUse> {
    match node.unspan() {
        Value::Set(t, val) => first_use_of(val, v).or_else(|| (*t == v).then_some(FirstUse::Write)),
        Value::Var(x) if *x == v => Some(FirstUse::Read),
        Value::Block(bl) => first_use_in_seq(&bl.operators, v),
        Value::Insert(ops) => first_use_in_seq(ops, v),
        Value::Loop(b) => match first_use_in_seq(&b.operators, v) {
            Some(FirstUse::Read) => Some(FirstUse::Read),
            _ => None,
        },
        Value::If(test, t, f) => first_use_of(test, v).or_else(|| {
            match (first_use_of(t, v), first_use_of(f, v)) {
                // A read on EITHER path makes the binding live; only a write on BOTH kills it.
                (Some(FirstUse::Read), _) | (_, Some(FirstUse::Read)) => Some(FirstUse::Read),
                (Some(FirstUse::Write), Some(FirstUse::Write)) => Some(FirstUse::Write),
                _ => None,
            }
        }),
        // The IR nodes that name a variable as a bare `u16` rather than through `Value::Var`.
        // Each one READS the variable it names (a `TuplePut` writes an ELEMENT, so the tuple
        // itself must already hold a value), and reading them as such is also the safe
        // direction: it can only keep a hoist that loft#1156 wants, never drop one.
        Value::TupleGet(x, _) if *x == v => Some(FirstUse::Read),
        Value::TuplePut(x, _, inner) => {
            first_use_of(inner, v).or_else(|| (*x == v).then_some(FirstUse::Read))
        }
        Value::FnRefDnr(x) if *x == v => Some(FirstUse::Read),
        Value::FnRef(_, clos_var, _) if *clos_var == v => Some(FirstUse::Read),
        _ => {
            let mut found = None;
            node.for_each_child(&mut |c| {
                if found.is_none() {
                    found = first_use_of(c, v);
                }
            });
            found
        }
    }
}

/// The first use of `v` across `ops`, evaluated in order.
fn first_use_in_seq(ops: &[Value], v: u16) -> Option<FirstUse> {
    ops.iter().find_map(|op| first_use_of(op, v))
}

/// The literal-backing accumulator a statement REPOINTS at a destination, if it has one.
///
/// A keyed or vector literal builds through a function-scoped accumulator (`__kvb_N` /
/// `__vdb_N`, [`crate::variables::owns_literal_backing_store`]) that normally OWNS the store it
/// builds into — which is why the scope-exit sweep frees it.  Where the destination is a
/// CAPTURE, @PLN93's build-into-target lowering instead REPOINTS the accumulator at that
/// destination, and the sweep then frees a store belonging to the frame that built the closure
/// (loft#1331).
///
/// The repoint is what this recognises, and it is the exact discriminator: a value-position
/// literal's block only ever assigns the accumulator `null` — it allocates and builds into a
/// store of its own — while a repointing block assigns it the destination first.  A plain
/// non-captured local rebind mints no accumulator at all.  Returning the accumulator here is
/// therefore the same question as *"does this statement leave it naming a store this frame does
/// not own?"*.
///
/// One home for every lowering: the keyed replace reaches the sweep as a bare `Block`, the
/// empty-literal clear as one wrapped in `OpReplaceKeyed`, and both are the same defect.
fn repointed_literal_accumulator(node: &Value, function: &Function) -> Option<u16> {
    if let Value::Block(bl) = node.unspan()
        && let Some(Value::Var(acc)) = bl.operators.last().map(Value::unspan)
        && crate::variables::owns_literal_backing_store(function.name(*acc))
        && bl.operators.iter().any(|op| {
            matches!(op.unspan(), Value::Set(t, val)
                if t == acc && !matches!(val.unspan(), Value::Null))
        })
    {
        return Some(*acc);
    }
    let mut found = None;
    node.for_each_child(&mut |c| {
        if found.is_none() {
            found = repointed_literal_accumulator(c, function);
        }
    });
    found
}

/// Sources whose retarget would move the destination's WRITE across a statement that touches
/// that destination.
///
/// [`move_rewrite`] retargets the source's construction ops onto the destination and drops the
/// copy, so the destination is written at the CONSTRUCTION's position instead of at the copy's.
/// That is sound only while nothing in between touches the destination.  The guards in
/// [`collect_move_dest`] all ask whether the destination is a STABLE container; none of them
/// asks whether it is READ, and that is the hole:
///
/// ```text
///   for p in v { held = Tg { name: p.0.name }; p.0 = p.1; p.1 = held; }
/// ```
///
/// `held`'s build moves into `p.1`, so `p.0 = p.1` then copies the NEW value back and the swap
/// answers `x|x` where it wants `y|x` — silently, on both backends.
///
/// Read by BOTH move shapes — the Record copy (`OpCopyRecord(src, dst)`, source at arg 0) and
/// the Construct append (`OpAppendVector(dst, src)`, source at arg 1) — because they had the
/// same hole and two copies of one predicate is the shape loft#1006 was.  B1.3d already carried
/// this guard for its own rewrite (`try_replace_one`: *"`base`'s BUILD must not read the
/// destination container"*), which is what the other two were missing.
///
/// Conservative by BASE variable: any mention of the destination's base container between the
/// two points disqualifies the move.  The source's own construction ops are excluded (they are
/// what gets retargeted), so `o.f = T { x: o.g }` — building FROM the container into it — is
/// still admitted.  A slot-exact test would admit a few more and cannot be spelled reliably:
/// two spellings of one slot is the shape loft#1006 was.
fn collect_move_disturbed(
    node: &Value,
    mo: &MoveOps,
    copy_op: u32,
    src_arg: usize,
    dest: &HashMap<u16, Value>,
    out: &mut HashSet<u16>,
) {
    let scan = |ops: &[Value], out: &mut HashSet<u16>| {
        for (copy_idx, op) in ops.iter().enumerate() {
            let Value::Call(d, args) = op.unspan() else {
                continue;
            };
            if *d != copy_op {
                continue;
            }
            let Some(Value::Var(s)) = args.get(src_arg).map(Value::unspan) else {
                continue;
            };
            let Some(slot) = dest.get(s) else { continue };
            let Some(base) = base_var_of(slot, mo) else {
                continue;
            };
            // Where `s` is first defined in THIS list; absent (built in another block) is
            // treated as "from the top", which is the conservative reading.
            let def_idx = ops
                .iter()
                .position(|o| match o.unspan() {
                    Value::Set(v, _) => *v == *s,
                    Value::Call(d2, a2) => {
                        *d2 == mo.op_database
                            && matches!(a2.first().map(Value::unspan), Some(Value::Var(v)) if *v == *s)
                    }
                    _ => false,
                })
                .unwrap_or(0);
            if def_idx >= copy_idx {
                continue;
            }
            for between in &ops[def_idx + 1..copy_idx] {
                // The source's OWN construction ops are what the rewrite retargets, so they
                // are not a disturbance — that is what keeps `o.f = T { x: o.g }` elidable.
                let writes_source = matches!(between.unspan(), Value::Call(_, a)
                    if matches!(a.first().map(Value::unspan), Some(Value::Var(v)) if *v == *s));
                if !writes_source && mentions_var(between, base) {
                    out.insert(*s);
                    break;
                }
            }
        }
    };
    match node.unspan() {
        Value::Block(b) => scan(&b.operators, out),
        Value::Insert(ops) => scan(ops, out),
        _ => {}
    }
    node.for_each_child(&mut |c| collect_move_disturbed(c, mo, copy_op, src_arg, dest, out));
}

/// Vars defined by `OpNewRecord` (`_elm_N = OpNewRecord(…)`) — the transient element slots the
/// vector-literal / construction lowering reuses. A copy destination based on one of these is a
/// FRESH element defined after the source, so it is not a stable in-place retarget target.
fn collect_element_vars(node: &Value, mo: &MoveOps) -> HashSet<u16> {
    fn walk(node: &Value, mo: &MoveOps, out: &mut HashSet<u16>) {
        if let Value::Set(v, rhs) = node.unspan()
            && matches!(rhs.unspan(), Value::Call(d, _) if *d == mo.op_new_record)
        {
            out.insert(*v);
        }
        node.for_each_child(&mut |c| walk(c, mo, out));
    }
    let mut out = HashSet::new();
    walk(node, mo, &mut out);
    out
}

/// Vars allocated (`OpDatabase(v, …)`) MORE THAN ONCE — a var REASSIGNED to a fresh record/vector
/// (`b = Bag{…}; … b = Bag{…}`). Such a container has a prior store the reorder rewrites' hoist does
/// not retire, so they must leave it a copy.
fn collect_multi_database(node: &Value, mo: &MoveOps) -> HashSet<u16> {
    fn walk(node: &Value, mo: &MoveOps, seen: &mut HashSet<u16>, multi: &mut HashSet<u16>) {
        if let Value::Call(d, args) = node.unspan()
            && *d == mo.op_database
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
            && !seen.insert(*v)
        {
            multi.insert(*v);
        }
        node.for_each_child(&mut |c| walk(c, mo, seen, multi));
    }
    let mut seen = HashSet::new();
    let mut multi = HashSet::new();
    walk(node, mo, &mut seen, &mut multi);
    multi
}

/// Does `src` ESCAPE its own construction — is it referenced anywhere OTHER than (a) as arg0 of a
/// WRITE op that builds it (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord`/`OpSetInt4`), or (b) as
/// arg1 of the append copy that moves it? A source that is built, then READ (`"{out:j}"`, `out[i]`,
/// passed to a fn), then moved is NOT dead-between-build-and-copy: building it directly into the
/// destination would leave the intermediate read seeing the un-built source (`var_out` not in
/// scope / wrong value). The `Set(src, …)` view-def / null-init targets `src` (not an arg) and is
/// fine; the `vdb` backing / `_elm` element temps aren't `src`. Any other appearance → escapes.
fn source_escapes(node: &Value, src: u16, co: &ConstructOps) -> bool {
    fn walk(node: &Value, src: u16, co: &ConstructOps, bad: &mut bool) {
        if let Value::Call(d, args) = node.unspan() {
            for (i, a) in args.iter().enumerate() {
                if matches!(a.unspan(), Value::Var(v) if *v == src) {
                    let write_arg0 = i == 0
                        && (*d == co.op_prealloc
                            || *d == co.op_new_record
                            || *d == co.op_finish_record
                            || *d == co.op_set_int4);
                    let copy_arg1 = i == 1 && *d == co.op_append;
                    if !(write_arg0 || copy_arg1) {
                        *bad = true;
                    }
                }
            }
        }
        node.for_each_child(&mut |c| walk(c, src, co, bad));
    }
    let mut bad = false;
    walk(node, src, co, &mut bad);
    bad
}

/// First-definition encounter order per var (`Set(v,…)` value-bind or `OpDatabase(v)` alloc). Used
/// to prove a Record destination's container is defined BEFORE the source is built — otherwise
/// retargeting the source's construction into `container.field` writes into an un-allocated slot
/// (`b = SABag { extra: extra }`: `b` allocated AFTER `extra` → `var_b` not in scope).
fn collect_def_order(node: &Value, mo: &MoveOps) -> HashMap<u16, usize> {
    // ⚠ Peels the SCRUTINEE while the recursion below walks the ORIGINAL `node`, so a spanned
    // node is visited TWICE: once here through the peel, once when `for_each_child` descends
    // to the same payload (it sees through a `Span` itself).  Safe here only because both
    // accumulations are IDEMPOTENT — `or_insert` ignores the second write, and `idx` feeds a
    // relative ORDER, so an extra tick shifts every later index equally.  Keep it that way, or
    // bind (`let node = node.unspan();`) so the match and the walk see the same node.  The
    // same shape counted a sandbox callee twice and inflated a heap bound by half — see
    // `sandbox::intrinsic_space`.
    fn walk(node: &Value, mo: &MoveOps, idx: &mut usize, out: &mut HashMap<u16, usize>) {
        match node.unspan() {
            Value::Set(v, _) => {
                out.entry(*v).or_insert(*idx);
            }
            Value::Call(d, args) if *d == mo.op_database => {
                if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                    out.entry(*v).or_insert(*idx);
                }
            }
            _ => {}
        }
        *idx += 1;
        node.for_each_child(&mut |c| walk(c, mo, idx, out));
    }
    let mut idx = 0;
    let mut out = HashMap::new();
    walk(node, mo, &mut idx, &mut out);
    out
}

/// The base container var of a slot expression: peel `OpGetField` / `OpGetVector` down to the
/// leading `Var`. `Var(v) → v`; a non-projection / non-Var expression → `None`.
fn base_var_of(expr: &Value, mo: &MoveOps) -> Option<u16> {
    match expr.unspan() {
        Value::Var(v) => Some(*v),
        Value::Call(d, args) if *d == mo.op_get_field || *d == mo.op_get_vector => {
            args.first().and_then(|a| base_var_of(a, mo))
        }
        _ => None,
    }
}

/// Pass 2 of [`move_elide`]: over each op list, DROP a ready source's `OpDatabase` /
/// `OpCopyRecord` / `OpFreeRef`, and RETARGET every remaining construction op that writes into
/// the source (`OpSet*(s, …)`) onto that source's captured destination expression.
fn move_rewrite(node: &mut Value, ready: &HashSet<u16>, dest: &HashMap<u16, Value>, mo: &MoveOps) {
    // `Value::unspan`'s obligation, in its mutable form.  Measured over the 858-program
    // corpus: 567 values arrive here, 41 of them spanned around an arm (33 `Call`, 8 `Block`).
    //
    // ⚠ Those 41 were NOT misses, and an earlier version of this comment claimed they were.
    // The `_` arm ends in `node.for_each_child_mut(…)`, and that walk DESCENDS THROUGH a
    // `Span`, so a spanned node was already reached one level down — measured
    // behaviour-identical across 120 corpus programs with and without this peel.  It is kept
    // because it obeys the rule and reaches the right arm directly; it fixes nothing.
    //
    // The `if let Value::Call(…) = node` below reads the ORIGINAL binding ON PURPOSE: peeling
    // it as well would retarget the same call twice, once here and once when the trailing walk
    // reaches it.  That is the double-count that peeling only the scrutinee produced in
    // `sandbox::intrinsic_space` — a bound inflated from `24 · n²` to `36 · n²`.
    match node.unspan_mut() {
        Value::Block(b) => {
            b.operators.retain(|s| !move_drop(s, ready, mo));
            for op in &mut b.operators {
                move_rewrite(op, ready, dest, mo);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !move_drop(s, ready, mo));
            for op in &mut *ops {
                move_rewrite(op, ready, dest, mo);
            }
        }
        _ => {
            if let Value::Call(d, args) = node {
                // Only a construction op (not one of the three dropped ops) whose target is a
                // ready source gets its target rewritten to the destination slot.
                let retarget = *d != mo.op_copy_record
                    && *d != mo.op_database
                    && *d != mo.op_free
                    && matches!(args.first().map(Value::unspan),
                        Some(Value::Var(s)) if ready.contains(s));
                if retarget
                    && let Some(Value::Var(s)) = args.first().map(Value::unspan)
                    && let Some(dst) = dest.get(s).cloned()
                {
                    args[0] = dst;
                }
            }
            node.for_each_child_mut(&mut |c| move_rewrite(c, ready, dest, mo));
        }
    }
}

/// Retain-predicate for [`move_rewrite`]: is `stmt` a dropped op (the source's alloc, the
/// `OpCopyRecord`, or the source's free) for a ready move source?
fn move_drop(stmt: &Value, ready: &HashSet<u16>, mo: &MoveOps) -> bool {
    if let Value::Call(d, args) = stmt.unspan()
        && (*d == mo.op_database || *d == mo.op_copy_record || *d == mo.op_free)
        && let Some(Value::Var(s)) = args.first().map(Value::unspan)
    {
        return ready.contains(s);
    }
    false
}

// The `op_` prefix is meaningful (operator def-numbers), so the same-prefix lint does not apply.
#[allow(clippy::struct_field_names)]
struct ConstructOps {
    op_database: u32,
    op_free: u32,
    op_append: u32,
    op_prealloc: u32,
    op_set_int4: u32,
    op_get_field: u32,
    op_clear: u32,
    op_new_record: u32,
    op_finish_record: u32,
}

/// @PLN90 phase B (B1.3b) — the CONSTRUCT copy shape (`x.field += src`, lowered as a copying
/// `OpAppendVector(x.field, src)`), restricted to the **reorder-free** case: the destination
/// container already exists when `src` is built. `src` is a vector view over its own backing
/// wrapper `vdb` (`src = OpGetField(vdb, …)`); instead of building `src`'s elements into `vdb`
/// then copying the whole vector into `x.field`, retarget `src`'s element-build ops
/// (`OpNewRecord`/`OpFinishRecord`) DIRECTLY onto `x.field`, and drop `vdb`'s alloc/init/free,
/// `src`'s view-def + capacity `OpPreAllocVector`, and the `OpAppendVector` copy. No new op — the
/// retargeted appends grow `x.field` exactly as the copy did.
///
/// **Reorder-free guard (the safety line):** fire only when `x`'s allocation precedes `vdb`'s
/// (or `x` is a parameter, i.e. never `OpDatabase`'d here). The fresh-construction case
/// (`a = Bag { items: base }` — `a` built AFTER `base`) needs a build-order reorder and is NOT
/// handled here; it is left as a copy. `skip` receives ONLY the backings of sources actually
/// rewritten, so a skipped source keeps its free (no live-store suppression).
// The eight parameters are one act's worth of state: the code being rewritten, the four
// read-only sets that decide whether a source may move, and the two out-parameters the
// caller reads back.  Bundling them into a struct would put a name between each set and
// the predicate that reads it without removing anything.
#[allow(clippy::too_many_arguments)]
fn construct_move_rewrite(
    code: &mut Value,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    mo: &MoveOps,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
    moved_into: &mut HashMap<u16, u16>,
) {
    // Pass 1 — pre-scan: per source capture the append destination, its backing wrapper, the
    // destination's container var, the `OpDatabase` encounter order (for the reorder guard) and
    // the control-flow position of the build and of the append (for the run-count guard).
    let mut sc = ConstructScan::default();
    construct_prescan(code, co, con_sources, &mut sc);
    let ConstructScan {
        db_order,
        db_path,
        dest,
        dest_path,
        ambiguous,
        vdb,
        container,
        ..
    } = sc;

    // A destination TOUCHED between the source's build and the append cannot take the retarget:
    // the append would move ahead of that access.  `escaping` above guards the SOURCE being read
    // in between and this is its missing twin — measured, `seen = len(b.v); b.v += tmp` reported
    // the POST-append length, and `for x in c.v { t2 += [...] } c.v += t2` retargeted the loop's
    // appends onto the vector the loop was ITERATING and grew it without bound.
    let mut disturbed: HashSet<u16> = HashSet::new();
    collect_move_disturbed(code, mo, co.op_append, 1, &dest, &mut disturbed);

    // Ready = found a unique append destination + a backing wrapper, AND provably reorder-free.
    let ready: HashSet<u16> = con_sources
        .iter()
        .copied()
        .filter(|s| {
            !ambiguous.contains(s)
                && dest.contains_key(s)
                && vdb.contains_key(s)
                // A source READ between its build and the move (or grown by appends) can't be built
                // directly into the destination — leave it a copy.
                && !escaping.contains(s)
                && !disturbed.contains(s)
                // B1.3b handles a PURE append (`x.field += src`). If the field is CLEARED anywhere
                // it is a whole-vector REPLACE (`x.field = src`, an `OpClearVector` + append) — a
                // simple retarget would leave the clear stranded after the retargeted build (empties
                // the result), and the source may even READ the field (`s.v = s.v[1..]`). Leave the
                // replace to B1.3d (which moves the clear) or to a copy.
                && !field_is_cleared(code, &dest[s], co)
                // The rewrite leaves the element builds where the source is BUILT and DROPS the
                // append, so the program it produces runs the build as often as the append it
                // replaced only when the two sit in the same control-flow region.  Where they do
                // not, the count is simply wrong in whichever direction the region runs: an
                // append inside a loop ran once per turn and now runs once for the whole loop
                // (`for i in 0..3 { d.c += s }` grew `d.c` by one copy, not three), and an append
                // inside a branch NOT TAKEN did not run at all and now always does (`if false {
                // d.c += s }` appended anyway).  Both answer wrong with nothing said, on both
                // backends, because this is one shared IR pass (@FR-O-NoDiverge).
                // Equality of the whole path, not of its depth: two arms of one `if` are equally
                // deep and never run together.  @FR-O-Latest — a binding's ownership
                // lives at the LOOP DEPTH its assignment was taken at, which no type-level fact
                // can carry, so the depth has to be measured here.  loft#1243.
                && db_path.get(&vdb[s]) == dest_path.get(s)
                && container.get(s).and_then(|c| *c).is_some_and(|cvar| {
                    // The container must PRE-EXIST when the source is built. A FRESH `OpNewRecord`
                    // element (`[Chunk { … }]` → `_elm_N.field += src`) has no `OpDatabase` but is
                    // NOT pre-existing — it is defined later, so retargeting there is use-before-def.
                    if bad_containers.contains(&cvar) {
                        return false;
                    }
                    // container built before the source's backing (both locals), or a real param.
                    match (db_order.get(&cvar), db_order.get(&vdb[s])) {
                        (Some(&c), Some(&v)) => c < v,
                        (None, Some(_)) => true, // container is a parameter (never allocated here)
                        _ => false,
                    }
                })
        })
        .collect();
    if ready.is_empty() {
        return;
    }
    let vdbs: HashSet<u16> = ready.iter().map(|s| vdb[s]).collect();

    // Pass 2 — retarget the element builds + drop the wrapper / view / prealloc / copy.
    construct_rewrite_ops(code, &ready, &vdbs, &dest, co);
    // Report where each source's records now live, so its borrowers can be re-pointed there
    // (loft#1241).  `ready` already proved the container is a `OpGetField(Var(c), …)` base.
    for s in &ready {
        if let Some(Some(cvar)) = container.get(s) {
            moved_into.insert(*s, *cvar);
        }
    }
    // The moved-out owned store is the backing wrapper (`src` is a borrow of it) — suppress ITS
    // free. Only ready sources' backings are added, so a skipped source keeps its free.
    skip.extend(vdbs);
}

/// The base var of a `OpGetField(Var(x), …)` destination expression (`x`), else `None`.
fn get_field_base(expr: &Value, co: &ConstructOps) -> Option<u16> {
    if let Value::Call(d, args) = expr.unspan()
        && *d == co.op_get_field
        && let Some(Value::Var(x)) = args.first().map(Value::unspan)
    {
        return Some(*x);
    }
    None
}

/// Where a statement sits in the control flow: the chain of enclosing regions that run
/// zero, one, or MANY times — a loop body, an `if` arm, a parallel arm, an iterator's
/// step.  Every region gets its own id, so two statements carry the same path exactly
/// when each execution of one is an execution of the other.  That is the question
/// [`construct_move_rewrite`] has to answer before it may move a build and drop a copy.
type CfPath = Vec<u32>;

/// What [`construct_prescan`] gathers in one walk of a function body, for
/// [`construct_move_rewrite`] to filter.
#[derive(Default)]
struct ConstructScan {
    /// `OpDatabase` encounter index per var (first allocation wins) — the build-order guard.
    db_order: HashMap<u16, usize>,
    /// Where each var's `OpDatabase` sits: the site the retargeted element builds stay at.
    db_path: HashMap<u16, CfPath>,
    /// The one append destination expression per source.
    dest: HashMap<u16, Value>,
    /// Where that append sits: the site whose copy the rewrite DROPS.
    dest_path: HashMap<u16, CfPath>,
    /// Sources appended into two places — not the clean shape, so not rewritten.
    ambiguous: HashSet<u16>,
    /// Source → the backing wrapper it is a view of.
    vdb: HashMap<u16, u16>,
    /// Source → the destination's container var, when the destination is a field read.
    container: HashMap<u16, Option<u16>>,
    /// Walk state: the `OpDatabase` counter, the region-id counter, and the current path.
    idx: usize,
    next_region: u32,
    path: CfPath,
}

/// Pass 1 of [`construct_move_rewrite`]: gather the append destination / backing / container /
/// `OpDatabase` order — and the control-flow position of the build and the append — for every
/// construct source.
fn construct_prescan(node: &Value, co: &ConstructOps, con: &HashSet<u16>, sc: &mut ConstructScan) {
    match node.unspan() {
        Value::Call(d, args) => {
            if *d == co.op_database {
                if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                    sc.db_order.entry(*v).or_insert(sc.idx);
                    sc.db_path.entry(*v).or_insert_with(|| sc.path.clone());
                }
                sc.idx += 1;
            } else if *d == co.op_append
                && let Some(dst) = args.first()
                && let Some(Value::Var(s)) = args.get(1).map(Value::unspan)
                && con.contains(s)
            {
                if sc.dest.contains_key(s) {
                    // ⚠ NOT idempotent, and this function double-visits a spanned node: the
                    // scrutinee above is peeled while `for_each_child` below walks the ORIGINAL,
                    // and that walk sees through a `Span` itself.  A second visit of the SAME
                    // append lands here and reads as two appends, which silently disqualifies the
                    // var from the construct rewrite.  Measured over the 858-program corpus: 77
                    // first-appends in 45 files and exactly ONE mark, which is genuine — it
                    // survives binding the peel.  So the hazard does not fire today; it is one
                    // edit away from firing.  See `sandbox::intrinsic_space` for the shape biting.
                    sc.ambiguous.insert(*s); // appended into two places — not the clean shape.
                } else {
                    sc.container.insert(*s, get_field_base(dst, co));
                    sc.dest.insert(*s, dst.clone());
                    sc.dest_path.insert(*s, sc.path.clone());
                }
            }
        }
        // `src = OpGetField(vdb, …)` — the source's view over its backing wrapper.
        Value::Set(s, rhs) if con.contains(s) => {
            if let Value::Call(gd, gargs) = rhs.unspan()
                && *gd == co.op_get_field
                && let Some(Value::Var(vd)) = gargs.first().map(Value::unspan)
            {
                sc.vdb.insert(*s, *vd);
            }
        }
        _ => {}
    }
    // A node whose children do NOT run exactly once with it opens a region, so everything
    // below carries a path the statements outside cannot match.  Scoped to the whole node
    // rather than to the arms alone: an `if` CONDITION does run once, but charging it the
    // region too only ever withholds a rewrite, and this walk visits a span-wrapped node
    // twice (the ⚠ above), which arm-precise pushes could not survive.
    let region = matches!(
        node.unspan(),
        Value::Loop(_) | Value::If(_, _, _) | Value::Parallel(_) | Value::Iter(_, _, _, _)
    );
    if region {
        sc.path.push(sc.next_region);
        sc.next_region += 1;
    }
    node.for_each_child(&mut |c| construct_prescan(c, co, con, sc));
    if region {
        sc.path.pop();
    }
}

/// Pass 2 of [`construct_move_rewrite`]: DROP the wrapper/view/prealloc/copy statements and
/// RETARGET each ready source's element-build ops (`OpNewRecord`/`OpFinishRecord`) onto its
/// append destination.
fn construct_rewrite_ops(
    node: &mut Value,
    ready: &HashSet<u16>,
    vdbs: &HashSet<u16>,
    dest: &HashMap<u16, Value>,
    co: &ConstructOps,
) {
    match node {
        Value::Block(b) => {
            b.operators.retain(|s| !construct_drop(s, ready, vdbs, co));
            for op in &mut b.operators {
                construct_rewrite_ops(op, ready, vdbs, dest, co);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !construct_drop(s, ready, vdbs, co));
            for op in &mut *ops {
                construct_rewrite_ops(op, ready, vdbs, dest, co);
            }
        }
        _ => {
            if let Value::Call(d, args) = node {
                // An element-build op (NOT one of the dropped/excluded ops) whose target is a
                // ready source → retarget it onto the append destination.
                let retarget = *d != co.op_database
                    && *d != co.op_prealloc
                    && *d != co.op_append
                    && *d != co.op_free
                    && *d != co.op_set_int4
                    && matches!(args.first().map(Value::unspan),
                        Some(Value::Var(s)) if ready.contains(s));
                if retarget
                    && let Some(Value::Var(s)) = args.first().map(Value::unspan)
                    && let Some(dst) = dest.get(s).cloned()
                {
                    args[0] = dst;
                }
            }
            node.for_each_child_mut(&mut |c| construct_rewrite_ops(c, ready, vdbs, dest, co));
        }
    }
}

/// Retain-predicate for [`construct_rewrite_ops`]: the wrapper's alloc/init/free, the source's
/// view-def + capacity pre-alloc, and the `OpAppendVector` copy are all dropped.
fn construct_drop(
    stmt: &Value,
    ready: &HashSet<u16>,
    vdbs: &HashSet<u16>,
    co: &ConstructOps,
) -> bool {
    match stmt.unspan() {
        // `src = OpGetField(vdb, …)` — the view-def is dead once the builds retarget.
        Value::Set(s, _) => ready.contains(s),
        Value::Call(d, args) => {
            let a0 = args.first().map(Value::unspan);
            if *d == co.op_database || *d == co.op_set_int4 || *d == co.op_free {
                matches!(a0, Some(Value::Var(v)) if vdbs.contains(v)) // wrapper alloc/len-init/free
            } else if *d == co.op_prealloc {
                matches!(a0, Some(Value::Var(s)) if ready.contains(s)) // src capacity hint — omit
            } else if *d == co.op_append {
                // the copy: OpAppendVector(dest, Var(src), …) — src in arg1.
                matches!(args.get(1).map(Value::unspan), Some(Value::Var(s)) if ready.contains(s))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// @PLN90 phase B (B1.3c) — the FRESH-construction move-elision (`a = Bag { items: base }`, the
/// container built AFTER the source). Unlike B1.3b's field-append it needs a build-order REORDER:
/// hoist `a`'s allocation ahead of `base`'s build, retarget `base`'s build ops
/// (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord`) onto `a.field`, drop the backing wrapper +
/// the `OpAppendVector` copy. Runs AFTER [`construct_move_rewrite`], on the copies it left standing.
///
/// Conservative — operates on a FLAT top-level block only, and rewrites each construct that passes
/// EVERY guard: `a`'s construction is a contiguous run of statements immediately before the copy
/// that references only `a` or a **never-written parameter** (B1.4 — a param's value is constant,
/// so hoisting reads the same value; any other var could be a local built between `base` and `a`
/// → SKIP); the run contains `a`'s `OpDatabase`; and `a` is genuinely allocated AFTER the source's
/// backing (a real reorder). B1.4 also lifts the one-construct-per-fn cap — each safe construct is
/// rewritten independently (a cross-construct dependency is a non-`a`, non-param run var → SKIPs
/// that one). B1.4 (nested) walks EVERY block, so a construct inside an `if`/loop body is handled
/// too — its `a`-alloc + `base`-build + copy are flat within that block. Anything else stays a
/// copy. `skip` receives only the moved-out backings.
#[allow(clippy::too_many_arguments)]
fn construct_fresh_rewrite(
    code: &mut Value,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) {
    // Apply the per-block reorder to the top block AND every nested block (if/loop bodies etc.).
    if let Value::Block(b) = code {
        fresh_rewrite_block(
            b,
            con_sources,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        );
    }
    code.for_each_child_mut(&mut |c| {
        construct_fresh_rewrite(
            c,
            con_sources,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        );
    });
}

/// Run the fresh-construction reorder over a SINGLE block's operators: rewrite each safe construct,
/// re-scanning after every rewrite (the reorder shifts indices), by earliest remaining copy (a
/// deterministic order); a guard-failing source is recorded so it is not retried (termination).
#[allow(clippy::too_many_arguments)]
fn fresh_rewrite_block(
    b: &mut Block,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) {
    let mut failed: HashSet<u16> = HashSet::new();
    loop {
        let next = con_sources
            .iter()
            .copied()
            .filter(|s| !failed.contains(s))
            .filter_map(|s| {
                b.operators
                    .iter()
                    .position(|op| append_copy_of(op, s, co).is_some())
                    .map(|ci| (ci, s))
            })
            .min_by_key(|&(ci, _)| ci);
        let Some((_, src)) = next else {
            break;
        };
        if !try_fresh_one(
            b,
            src,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        ) {
            failed.insert(src);
        }
    }
}

/// Attempt the fresh-construction reorder for ONE source `src` on the flat block `b`. Returns
/// `true` (and rewrites `b.operators` + records the moved-out backing in `skip`) iff every guard
/// holds; `false` otherwise, leaving `b` unchanged.
#[allow(clippy::too_many_arguments)]
fn try_fresh_one(
    b: &mut Block,
    src: u16,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) -> bool {
    if escaping.contains(&src) {
        return false; // read between build and copy (or append-grown) → not safe to retarget.
    }
    // Copy index + destination expression (`a.field`) — the first copy of `src`.
    let mut ci = None;
    let mut dest = None;
    for (i, op) in b.operators.iter().enumerate() {
        if let Some(d) = append_copy_of(op, src, co) {
            ci = Some(i);
            dest = Some(d);
            break;
        }
    }
    let (Some(ci), Some(dest)) = (ci, dest) else {
        return false;
    };
    let Some(a) = get_field_base(&dest, co) else {
        return false;
    };
    if bad_containers.contains(&a) {
        return false; // a REASSIGNED container has a prior store the hoist does not retire.
    }

    // The source's backing wrapper (`src = OpGetField(vdb, …)`).
    let mut vdb = None;
    for op in &b.operators {
        if let Value::Set(s, rhs) = op.unspan()
            && *s == src
            && let Value::Call(gd, gargs) = rhs.unspan()
            && *gd == co.op_get_field
            && let Some(Value::Var(v)) = gargs.first().map(Value::unspan)
        {
            vdb = Some(*v);
        }
    }
    let Some(vdb) = vdb else {
        return false;
    };

    // `a`'s construction run: the contiguous block [ps..ci) whose statements all target `a`.
    let mut ps = ci;
    while ps > 0 && stmt_targets_var(&b.operators[ps - 1], a) {
        ps -= 1;
    }
    let run = &b.operators[ps..ci];
    // Guards: the run allocates `a`, references only `a` or a never-written param (dependency-safe
    // to hoist past `base`), and `a` is allocated AFTER the backing (else this isn't a reorder).
    if !run.iter().any(|s| call_is(s, co.op_database, a)) {
        return false;
    }
    if !run.iter().all(|s| run_var_ok(s, a, function, written)) {
        return false;
    }
    match b
        .operators
        .iter()
        .position(|s| call_is(s, co.op_database, vdb))
    {
        Some(vi) if vi < ps => {}
        _ => return false,
    }

    // Rebuild: [a-construction run] ++ [base's build, wrapper dropped + retargeted] ++ [rest].
    let ops = std::mem::take(&mut b.operators);
    let mut new_ops = Vec::with_capacity(ops.len());
    for op in &ops[ps..ci] {
        new_ops.push(op.clone());
    }
    for (i, op) in ops.into_iter().enumerate() {
        if (ps..=ci).contains(&i) {
            continue; // run hoisted above; the copy at `ci` is dropped.
        }
        if fresh_drop(&op, src, vdb, co) {
            continue;
        }
        let mut op = op;
        fresh_retarget(&mut op, src, &dest, co);
        new_ops.push(op);
    }
    b.operators = new_ops;
    skip.insert(vdb); // the backing wrapper is the moved-out owned store.
    true
}

/// If `op` is `OpAppendVector(dest, Var(src), …)` (the copy of `src` into a field), return `dest`.
fn append_copy_of(op: &Value, src: u16, co: &ConstructOps) -> Option<Value> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && let Some(Value::Var(s)) = args.get(1).map(Value::unspan)
        && *s == src
    {
        return args.first().cloned();
    }
    None
}

/// Does `stmt` write into variable `v` (its `Set` target, or a `Call`'s first arg)?
fn stmt_targets_var(stmt: &Value, v: u16) -> bool {
    match stmt.unspan() {
        Value::Set(s, _) => *s == v,
        Value::Call(_, args) => {
            matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == v)
        }
        _ => false,
    }
}

/// Is `stmt` the call `op` with first arg `Var(v)` (e.g. `OpDatabase(v, …)`)?
fn call_is(stmt: &Value, op: u32, v: u16) -> bool {
    matches!(stmt.unspan(), Value::Call(d, args) if *d == op
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == v))
}

/// Does every `Var` referenced anywhere in `stmt`'s value positions equal `only`? (A `Set`/`Call`
/// TARGET is `only` by construction; this checks the RHS/args carry no dependency on another var.)
/// Every `Var` in `stmt`'s subtree is either `a` or a never-written PARAMETER (whose value is
/// therefore constant, so hoisting `a`'s construction past `base`'s build reads the same value).
/// Any other var — a local that might be built between `base` and `a` — fails, so the construct
/// stays a copy. (`written` is the fn-wide over-approximation from [`collect_written`].)
fn run_var_ok(stmt: &Value, a: u16, function: &Function, written: &HashSet<u16>) -> bool {
    fn walk(node: &Value, a: u16, function: &Function, written: &HashSet<u16>, ok: &mut bool) {
        if let Value::Var(v) = node.unspan() {
            let v = *v;
            let allowed = v == a || (function.is_argument(v) && !written.contains(&v));
            if !allowed {
                *ok = false;
            }
        }
        node.for_each_child(&mut |c| walk(c, a, function, written, ok));
    }
    let mut ok = true;
    walk(stmt, a, function, written, &mut ok);
    ok
}

/// Retain-drop for [`construct_fresh_rewrite`]: the backing wrapper's alloc/len-init/free and the
/// source's view-def (`src = OpGetField(vdb, …)`).
fn fresh_drop(stmt: &Value, src: u16, vdb: u16, co: &ConstructOps) -> bool {
    match stmt.unspan() {
        Value::Set(s, _) => *s == src,
        Value::Call(d, args) => {
            (*d == co.op_database || *d == co.op_set_int4 || *d == co.op_free)
                && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == vdb)
        }
        _ => false,
    }
}

/// Retarget every source-targeting build op (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord` —
/// NOT the dropped alloc/append/free/set-int4/get-field ops) onto the `dest` field. Unlike the
/// field-append path, the capacity `OpPreAllocVector` IS retargeted here: the fresh field is empty,
/// so pre-claiming its capacity is correct (it mirrors the source's own initial claim).
fn fresh_retarget(node: &mut Value, src: u16, dest: &Value, co: &ConstructOps) {
    if let Value::Call(d, args) = node {
        let hit = *d != co.op_database
            && *d != co.op_append
            && *d != co.op_free
            && *d != co.op_set_int4
            && *d != co.op_get_field
            && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == src);
        if hit {
            args[0] = dest.clone();
        }
    }
    node.for_each_child_mut(&mut |c| fresh_retarget(c, src, dest, co));
}

/// @PLN90 phase B (B1.3d) — the `a.field = base` whole-vector REPLACEMENT, a DOUBLE copy the
/// compiler lowers as `base → __p154_rhs → a.field` with an `OpClearVector` between:
///
/// ```text
///   <build base into __vdb>
///   __p154_rhs = null; OpAppendVector(__p154_rhs, base);   (copy 1 — base → temp)
///   OpClearVector(a.field);                                (clear a.field's old contents)
///   OpAppendVector(a.field, __p154_rhs);                   (copy 2 — temp → a.field)
/// ```
///
/// `base`'s copy target is a temp (`__p154_rhs`), so it is not a `MovePlan`; this rewrite detects
/// the idiom STRUCTURALLY. When `base` is a dead-after local (its own `__vdb` backing), build it
/// DIRECTLY into the cleared `a.field`: move the `OpClearVector` ahead of `base`'s build, retarget
/// `base`'s build ops onto `a.field`, and drop the temp + both copies + the wrapper. Both temp and
/// wrapper join `skip` (their frees are suppressed). Walks every block (nested `if`/loop bodies too);
/// conservative.
fn construct_replace_rewrite(code: &mut Value, co: &ConstructOps, skip: &mut HashSet<u16>) {
    if let Value::Block(b) = code {
        replace_rewrite_block(b, co, skip);
    }
    code.for_each_child_mut(&mut |c| construct_replace_rewrite(c, co, skip));
}

/// Run the whole-vector-replacement rewrite over a SINGLE block's operators.
fn replace_rewrite_block(b: &mut Block, co: &ConstructOps, skip: &mut HashSet<u16>) {
    let mut failed: HashSet<usize> = HashSet::new();
    loop {
        // Find the earliest `copy 2` (`OpAppendVector(a.field, Var(rhs))`) preceded by the clear +
        // `copy 1`, not yet tried.
        let mut hit = None;
        for c2 in 2..b.operators.len() {
            if failed.contains(&c2) {
                continue;
            }
            let Some((field, rhs)) = append_field_temp(&b.operators[c2], co) else {
                continue;
            };
            // Preceding statement must be `OpClearVector(field)` for the SAME field.
            if !is_clear_of(&b.operators[c2 - 1], &field, co) {
                continue;
            }
            // And the one before that `OpAppendVector(Var(rhs), Var(base))`.
            let Some(base) = append_temp_src(&b.operators[c2 - 2], rhs, co) else {
                continue;
            };
            hit = Some((c2, field, rhs, base));
            break;
        }
        let Some((c2, field, rhs, base)) = hit else {
            break;
        };
        if !try_replace_one(b, c2, &field, rhs, base, co, skip) {
            failed.insert(c2);
        }
    }
}

/// One `a.field = base` replacement at `copy 2` index `c2`. Rewrites `b.operators` + records the
/// moved-out temp/wrapper in `skip` iff every guard holds; else returns `false` unchanged.
fn try_replace_one(
    b: &mut Block,
    c2: usize,
    field: &Value,
    rhs: u16,
    base: u16,
    co: &ConstructOps,
    skip: &mut HashSet<u16>,
) -> bool {
    let Some(a) = get_field_base(field, co) else {
        return false;
    };
    // `base` must be a dead-after LOCAL: it owns a backing wrapper `base = OpGetField(vdb, …)`, and
    // is not referenced after `copy 2` (the store transfers, so a later read would dangle).
    let mut vdb = None;
    let mut rhs_set = None;
    for (i, op) in b.operators.iter().enumerate() {
        match op.unspan() {
            Value::Set(s, r) if *s == base => {
                if let Value::Call(gd, ga) = r.unspan()
                    && *gd == co.op_get_field
                    && let Some(Value::Var(v)) = ga.first().map(Value::unspan)
                {
                    vdb = Some(*v);
                }
            }
            Value::Set(s, _) if *s == rhs => rhs_set = Some(i),
            _ => {}
        }
    }
    let (Some(vdb), Some(_)) = (vdb, rhs_set) else {
        return false;
    };
    if a == base || a == vdb {
        return false; // paranoia: the destination container must be independent of the source
    }
    if references_var_after(b, base, c2) {
        return false;
    }
    // `base`'s build start: the first statement that sets up its wrapper or references it. Everything
    // before it (`a`'s already-built value + null-inits) is kept; the clear is inserted there.
    let Some(bs) = b
        .operators
        .iter()
        .position(|op| fresh_drop(op, base, vdb, co) || refs_var(op, base))
    else {
        return false;
    };
    if bs >= c2 - 2 {
        return false; // base built after the field already exists but not as a real preceding build
    }
    // The container `a` must ALREADY EXIST at `base`'s build (we move the clear + build to `bs`, so
    // `a.field` must be valid there). If `a` is allocated in THIS block AFTER `base` (`fresh = …;
    // s = S{…}; s.field = fresh`), building into `a.field` at `bs` would hit an un-allocated `a` —
    // SKIP. A param / outer-scope `a` has no `OpDatabase` here → it pre-exists → fine.
    if let Some(a_db) = b
        .operators
        .iter()
        .position(|op| call_is(op, co.op_database, a))
        && a_db >= bs
    {
        return false;
    }
    let chain: HashSet<usize> = [rhs_set.unwrap(), c2 - 2, c2 - 1, c2].into_iter().collect();
    // `base`'s BUILD must not read the destination container `a` (a SELF-ASSIGN like
    // `s.v = s.v[1..]` builds the source by slicing `s.v` — moving the `OpClearVector` ahead of that
    // read would empty `s.v` before the slice copies it). Guard the build region [bs..c2) (excluding
    // the chain: copy1 / clear / copy2 legitimately reference `a`).
    if (bs..c2).any(|i| !chain.contains(&i) && refs_var(&b.operators[i], a)) {
        return false;
    }

    let clear = b.operators[c2 - 1].clone();
    let ops = std::mem::take(&mut b.operators);
    let mut new_ops = Vec::with_capacity(ops.len());
    for (i, op) in ops.into_iter().enumerate() {
        if i == bs {
            new_ops.push(clear.clone()); // clear a.field BEFORE building base into it
        }
        if i < bs {
            new_ops.push(op);
            continue;
        }
        if chain.contains(&i) || fresh_drop(&op, base, vdb, co) {
            continue;
        }
        let mut op = op;
        fresh_retarget(&mut op, base, field, co); // base's build → a.field
        new_ops.push(op);
    }
    b.operators = new_ops;
    skip.insert(vdb);
    skip.insert(rhs);
    true
}

/// If `op` is `OpAppendVector(OpGetField(…), Var(rhs))` (copy INTO a field from a temp), return
/// `(field_expr, rhs)`.
fn append_field_temp(op: &Value, co: &ConstructOps) -> Option<(Value, u16)> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && let Some(field) = args.first()
        && get_field_base(field, co).is_some()
        && let Some(Value::Var(rhs)) = args.get(1).map(Value::unspan)
    {
        return Some((field.clone(), *rhs));
    }
    None
}

/// Is `op` `OpClearVector(field)` for the given `field` expression?
fn is_clear_of(op: &Value, field: &Value, co: &ConstructOps) -> bool {
    matches!(op.unspan(), Value::Call(d, args) if *d == co.op_clear
        && args.first().map(Value::unspan) == Some(field.unspan()))
}

/// Is `field` cleared (`OpClearVector(field)`) ANYWHERE in `node`'s subtree? A cleared destination
/// means the append is really a whole-vector REPLACE, not a pure `+=` append.
fn field_is_cleared(node: &Value, field: &Value, co: &ConstructOps) -> bool {
    fn walk(node: &Value, field: &Value, co: &ConstructOps, found: &mut bool) {
        if is_clear_of(node, field, co) {
            *found = true;
        }
        node.for_each_child(&mut |c| walk(c, field, co, found));
    }
    let mut found = false;
    walk(node, field, co, &mut found);
    found
}

/// If `op` is `OpAppendVector(Var(rhs), Var(base))` (copy INTO the temp from `base`), return `base`.
fn append_temp_src(op: &Value, rhs: u16, co: &ConstructOps) -> Option<u16> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == rhs)
        && let Some(Value::Var(base)) = args.get(1).map(Value::unspan)
    {
        return Some(*base);
    }
    None
}

/// Does `op` reference `Var(v)` anywhere in its subtree?
fn refs_var(op: &Value, v: u16) -> bool {
    fn walk(node: &Value, v: u16, found: &mut bool) {
        if matches!(node.unspan(), Value::Var(x) if *x == v) {
            *found = true;
        }
        node.for_each_child(&mut |c| walk(c, v, found));
    }
    let mut found = false;
    walk(op, v, &mut found);
    found
}

/// Is `Var(v)` referenced in any operator strictly after index `idx`?
fn references_var_after(b: &Block, v: u16, idx: usize) -> bool {
    b.operators[idx + 1..].iter().any(|op| refs_var(op, v))
}

/// @PLN94 TEST-ONLY: the var name whose scope-exit free `get_free_vars` drops (injecting a genuine
/// leak, the `check-leak` true-positive gate), or `None`. Cached — ONE env read per process, so the
/// production (unset) path pays nothing per-free. Never set outside tests.
fn inject_drop_free() -> Option<&'static str> {
    use std::sync::OnceLock;
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| std::env::var("LOFT_OWN_INJECT_DROP_FREE").ok())
        .as_deref()
}

/// @PLN94 TEST-ONLY: the BORROWED var name whose scope-exit free `get_free_vars` is forced to emit
/// (injecting a genuine OVER-free — an unconditional `OpFreeRef` of a dep-carrying view), the
/// over-free-check (`run_over_free_check`) true-positive gate, or `None`. Cached like
/// [`inject_drop_free`]; the production (unset) path pays nothing. Never set outside tests.
fn inject_free_borrowed() -> Option<&'static str> {
    use std::sync::OnceLock;
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| std::env::var("LOFT_OWN_INJECT_FREE_BORROWED").ok())
        .as_deref()
}

/// @FR-O-Override TEST-ONLY: the NEVER-FREE var name for which `get_free_vars` emits a
/// witness-guarded free against ITSELF — `OpFreeRefIfDistinct(v, v)`, a run-time no-op (one store on
/// both sides) that the IR nevertheless NAMES as a free of a never-free binding, which
/// `ownership_cfg`'s Check D must report.  The check's true-positive gate; cached like
/// [`inject_drop_free`]; never set outside tests.
fn inject_free_skipfree() -> Option<&'static str> {
    use std::sync::OnceLock;
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| std::env::var("LOFT_OWN_INJECT_FREE_SKIPFREE").ok())
        .as_deref()
}

/// @PLN101 — ISOLATED value-struct copy pass. A `value struct` is an ordinary
/// `Type::Reference` record (marked only by `Data.value_structs`); the ONLY behavioural
/// difference is value (copy) semantics. When such a struct is BOUND to a local from a VIEW
/// (a field/element read — `e = record.f` / `e = vec[i]`), rewrite the bind to mint a fresh
/// store and deep-copy into it, so the local owns its own record and mutating it cannot write
/// back through the view. Not wired into the type system: no `Type` variant, no assignment-path
/// surgery, no ownership-oracle change. Runs after `move_elide` and BEFORE the ownership scan,
/// so the emitted `OpDatabase` makes the local classify `Owned` on its own (sound: it earns
/// ownership, it is not asserted). Only fires on a `Set(local, borrowed-view)`; a fresh
/// construction (`Owned` rhs) or a method-call arg is not rewritten (copy-elision / zero-copy
/// hot path fall out).
fn value_struct_copy(data: &mut Data) {
    // No value structs in the program → the pass is a no-op (and the read-only-elision oracles
    // below need not run for any function).
    if data.value_structs.is_empty() {
        return;
    }
    let op_database = data.def_nr("OpDatabase");
    let op_copy_record = data.def_nr("OpCopyRecord");
    if op_database == u32::MAX || op_copy_record == u32::MAX {
        return;
    }
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let mut code = data.definitions[d_nr as usize].code.clone();
        // Read-only-elision oracle (function scope; rescoped per loop body inside the walk): the
        // TAINTED set — variables whose backing may be mutated, or which escape, during a view's
        // lifetime. Seeded from field/element writes (`find_field_written_vars`, catches nested
        // `v.a.b = …`) plus escapes (return / passed to a user fn), then closed over pure-`Var`
        // alias edges so mutating one co-alias taints the shared backing. A value-struct view-bind
        // is left as a zero-cost view iff neither the local nor any base variable is tainted.
        let tainted = vs_scope_taint(std::slice::from_ref(&code), data);
        let mut cleared: Vec<u16> = Vec::new();
        vs_copy_walk(
            &mut code,
            data,
            d_nr,
            op_database,
            op_copy_record,
            &tainted,
            &mut cleared,
        );
        if cleared.is_empty() {
            continue;
        }
        data.definitions[d_nr as usize].code = code;
        // Clear each copied local's VIEW dep so it frees as an owned store (no leak, no
        // double-free): the local now owns a fresh copy, not a borrow into the source.
        for v in cleared {
            if let Type::Reference(p, _) = *data.def(d_nr).variables.tp(v) {
                data.definitions[d_nr as usize]
                    .variables
                    .set_type(v, Type::Reference(p, Deps::none()));
            }
        }
    }
}

/// @PLN101 zero-cost — root variable a VIEW projects from: follow the arg-0 spine of read ops
/// (`OpGet*`) down to a `Var`. Index/offset args are ignored (a mutated loop counter is not a base
/// mutation). `None` if the spine doesn't bottom out in a plain variable.
fn vs_view_root(node: &Value, data: &Data) -> Option<u16> {
    match node.unspan() {
        Value::Var(v) => Some(*v),
        Value::Call(fn_nr, args)
            if !args.is_empty() && data.def(*fn_nr).name().starts_with("OpGet") =>
        {
            vs_view_root(&args[0], data)
        }
        // A view can be wrapped in a block that yields its tail value — notably a for-loop's
        // `{#iter next}` (advance the index, then project the element). Follow the tail.
        Value::Block(b) => b.operators.last().and_then(|t| vs_view_root(t, data)),
        Value::Insert(ops) => ops.last().and_then(|t| vs_view_root(t, data)),
        _ => None,
    }
}

/// First rhs of `Set(var, rhs)` for `var` (its binding), for one-hop base-alias tracing.
fn vs_find_binding(code: &Value, var: u16) -> Option<&Value> {
    fn walk<'a>(node: &'a Value, var: u16, found: &mut Option<&'a Value>) {
        if found.is_some() {
            return;
        }
        match node {
            Value::Set(v, body) => {
                if *v == var {
                    *found = Some(body);
                } else {
                    walk(body, var, found);
                }
            }
            Value::Block(b) | Value::Loop(b) => {
                for op in &b.operators {
                    walk(op, var, found);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    walk(op, var, found);
                }
            }
            Value::If(c, t, e) => {
                walk(c, var, found);
                walk(t, var, found);
                walk(e, var, found);
            }
            Value::Return(x) | Value::Drop(x) => walk(x, var, found),
            Value::Call(_, args) => {
                for a in args {
                    walk(a, var, found);
                }
            }
            Value::Iter(_, a, b, c) => {
                walk(a, var, found);
                walk(b, var, found);
                walk(c, var, found);
            }
            Value::Span(b) => walk(&b.1, var, found),
            _ => {}
        }
    }
    let mut found = None;
    walk(code, var, &mut found);
    found
}

/// The variables whose backing a value-struct view reads from: the spine root plus, one hop at a
/// time, the root of whatever each is bound from (`_vector_1 = b.items` → also `b`), so an alias of
/// a mutated source is caught. Bounded to a few hops (straight-line binds, no cycles).
fn vs_base_vars(rhs: &Value, data: &Data, code: &Value, out: &mut HashSet<u16>) {
    let mut work: Vec<u16> = Vec::new();
    if let Some(r) = vs_view_root(rhs, data) {
        work.push(r);
    }
    let mut hops = 0;
    while let Some(v) = work.pop() {
        if !out.insert(v) {
            continue;
        }
        hops += 1;
        if hops > 8 {
            break;
        }
        if let Some(bind) = vs_find_binding(code, v)
            && let Some(r) = vs_view_root(bind, data)
        {
            work.push(r);
        }
    }
}

/// Walk the function collecting the TAINT seed + pure-`Var` alias edges for read-only elision.
/// A variable is a taint SOURCE if it ESCAPES — returned, or handed as a bare `Var` argument to a
/// user function (non-`Op`), which could store or mutate it. (Field/element mutations are folded in
/// separately from `find_field_written_vars`; a store like `coll.push(p)` copies `p` and so does not
/// taint it.) An `w = x` bind adds a symmetric alias EDGE so that mutating one co-alias taints the
/// backing both share. The caller closes `seed` over `edges` to get the full tainted set.
fn vs_collect_taint(
    node: &Value,
    data: &Data,
    seed: &mut HashSet<u16>,
    edges: &mut Vec<(u16, u16)>,
) {
    match node {
        Value::Set(w, body) => {
            if let Value::Var(x) = body.unspan() {
                edges.push((*w, *x)); // pure alias `w = x` — taint flows both ways
            }
            vs_collect_taint(body, data, seed, edges);
        }
        Value::Return(x) | Value::Drop(x) => {
            if let Value::Var(v) = x.unspan() {
                seed.insert(*v); // escape via return
            }
            vs_collect_taint(x, data, seed, edges);
        }
        Value::Call(fn_nr, args) => {
            let is_op = data.def(*fn_nr).name().starts_with("Op");
            for a in args {
                // A bare `Var` passed to a USER function may be stored or mutated by the callee.
                if !is_op && let Value::Var(v) = a.unspan() {
                    seed.insert(*v);
                }
                vs_collect_taint(a, data, seed, edges);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                vs_collect_taint(op, data, seed, edges);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                vs_collect_taint(op, data, seed, edges);
            }
        }
        Value::If(c, t, e) => {
            vs_collect_taint(c, data, seed, edges);
            vs_collect_taint(t, data, seed, edges);
            vs_collect_taint(e, data, seed, edges);
        }
        Value::Iter(_, a, b, c) => {
            vs_collect_taint(a, data, seed, edges);
            vs_collect_taint(b, data, seed, edges);
            vs_collect_taint(c, data, seed, edges);
        }
        Value::Span(b) => vs_collect_taint(&b.1, data, seed, edges),
        _ => {}
    }
}

/// Close `seed` under the symmetric alias `edges` — the set of variables whose backing may change
/// or escape during a view's lifetime. A value-struct view-bind is elidable iff neither the local
/// nor any base variable is in this set.
fn vs_tainted(seed: HashSet<u16>, edges: &[(u16, u16)]) -> HashSet<u16> {
    let mut adj: HashMap<u16, Vec<u16>> = HashMap::new();
    for &(a, b) in edges {
        adj.entry(a).or_default().push(b);
        adj.entry(b).or_default().push(a);
    }
    let mut tainted = seed;
    let mut work: Vec<u16> = tainted.iter().copied().collect();
    while let Some(v) = work.pop() {
        if let Some(ns) = adj.get(&v) {
            for &n in ns {
                if tainted.insert(n) {
                    work.push(n);
                }
            }
        }
    }
    tainted
}

/// The tainted set over a straight-line SCOPE (a function body, or a loop body). Scoping to the
/// enclosing loop is what makes a `for p in ps { …read p… }` bind elidable: BUILDING `ps` field-
/// writes it, but that construction runs BEFORE the loop, so it cannot diverge a view read INSIDE
/// the loop from a copy. Only a mutation/escape WITHIN the loop (which repeats) can — S1's
/// `b.items[0].x = 99` in the body still taints `b`.
fn vs_scope_taint(ops: &[Value], data: &Data) -> HashSet<u16> {
    let mut seed: HashSet<u16> = HashSet::new();
    let mut edges: Vec<(u16, u16)> = Vec::new();
    for op in ops {
        crate::parser::find_field_written_vars(op, data, &mut seed);
        vs_collect_taint(op, data, &mut seed, &mut edges);
    }
    vs_tainted(seed, &edges)
}

/// Recursive rewriter for [`value_struct_copy`] — mirrors the IR walk in
/// `variables/validate.rs::build_scope_parents`. `tainted` is the function-wide read-only-elision
/// oracle (computed once by the caller): a value-struct view-bind is left as a zero-cost view (like
/// a reference struct) when neither the local nor any base variable is tainted; otherwise the copy
/// is emitted for value semantics.
fn vs_copy_walk(
    node: &mut Value,
    data: &Data,
    d_nr: u32,
    op_database: u32,
    op_copy_record: u32,
    tainted: &HashSet<u16>,
    cleared: &mut Vec<u16>,
) {
    match node {
        Value::Set(v, rhs) => {
            let vv = *v;
            vs_copy_walk(
                rhs,
                data,
                d_nr,
                op_database,
                op_copy_record,
                tainted,
                cleared,
            );
            if let Type::Reference(p, _) = *data.def(d_nr).variables.tp(vv)
                && data.is_value_struct(p)
                && matches!(
                    crate::use_analysis::ownership_of(data, d_nr, rhs),
                    crate::use_analysis::Own::Borrowed { .. }
                )
            {
                // Zero-cost read-only elision: if the local and its whole projection base are only
                // ever read (never field-written, never escaping), a plain view is observably
                // identical to a copy — so skip the copy and keep the reference-struct-cheap view.
                let mut affected: HashSet<u16> = HashSet::new();
                affected.insert(vv);
                vs_base_vars(rhs, data, &data.def(d_nr).code, &mut affected);
                let needs_copy = affected.iter().any(|x| tainted.contains(x));
                if !needs_copy {
                    return;
                }
                let kt = i32::from(data.def(p).known_type());
                let source = (**rhs).clone();
                *node = Value::Insert(vec![
                    Value::Set(vv, Box::new(Value::Null)),
                    Value::Call(op_database, vec![Value::Var(vv), Value::Int(kt)]),
                    Value::Call(op_copy_record, vec![source, Value::Var(vv), Value::Int(kt)]),
                ]);
                cleared.push(vv);
            }
        }
        Value::Block(b) => {
            for op in &mut b.operators {
                vs_copy_walk(
                    op,
                    data,
                    d_nr,
                    op_database,
                    op_copy_record,
                    tainted,
                    cleared,
                );
            }
        }
        Value::Loop(b) => {
            // Rescope taint to this loop body — pre-loop construction of a base does not diverge a
            // view read inside the loop, only an in-body mutation/escape does.
            let loop_taint = vs_scope_taint(&b.operators, data);
            for op in &mut b.operators {
                vs_copy_walk(
                    op,
                    data,
                    d_nr,
                    op_database,
                    op_copy_record,
                    &loop_taint,
                    cleared,
                );
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                vs_copy_walk(
                    op,
                    data,
                    d_nr,
                    op_database,
                    op_copy_record,
                    tainted,
                    cleared,
                );
            }
        }
        Value::If(c, t, e) => {
            vs_copy_walk(c, data, d_nr, op_database, op_copy_record, tainted, cleared);
            vs_copy_walk(t, data, d_nr, op_database, op_copy_record, tainted, cleared);
            vs_copy_walk(e, data, d_nr, op_database, op_copy_record, tainted, cleared);
        }
        Value::Return(x) | Value::Drop(x) => {
            vs_copy_walk(x, data, d_nr, op_database, op_copy_record, tainted, cleared);
        }
        Value::Call(_, args) => {
            for a in args {
                vs_copy_walk(a, data, d_nr, op_database, op_copy_record, tainted, cleared);
            }
        }
        Value::Iter(_, a, b, c) => {
            vs_copy_walk(a, data, d_nr, op_database, op_copy_record, tainted, cleared);
            vs_copy_walk(b, data, d_nr, op_database, op_copy_record, tainted, cleared);
            vs_copy_walk(c, data, d_nr, op_database, op_copy_record, tainted, cleared);
        }
        Value::Span(b) => {
            vs_copy_walk(
                &mut b.1,
                data,
                d_nr,
                op_database,
                op_copy_record,
                tainted,
                cleared,
            );
        }
        _ => {}
    }
}

/// Scope / lifetime analysis pass over every function definition.
///
/// # Panics
/// Under the `LASTUSE_RECLAIM` gate only (a Plan-57 testing build), panics if the
/// reclaim pass left a store the model says is dead un-freed past a later
/// allocation (the Phase-4 Goal-E watermark guard).  Never panics in normal builds.
pub fn check(data: &mut Data, database: &mut crate::database::Stores) {
    // @PLN94 — the CFG/dataflow completeness oracle, an OBSERVER reached only via
    // LOFT_OWN_ORACLE (SI-1: shipped codegen byte-identical; a no-op when unset).
    crate::ownership_cfg::oracle(data);
    // @PLN153 phase 0 — the `τ??` census, an OBSERVER gated on LOFT_NULL_CENSUS.
    crate::null_census::report(data);
    // Behaviour-neutral USE-analysis dump (LOFT_MATERIALIZE_DUMP) — the
    // copy-vs-borrow verdict per binding, before any codegen consumes it.
    crate::use_analysis::dump_all(data);
    // @PLN90 Step 5 — the user-facing copy report (`--report-copies`) is emitted ONCE from
    // main after the whole program is loaded (not here — `check` runs per file-load).
    // @PLN90 phase B B1.2 — dump the last-use MOVE-elision plans (LOFT_MOVE_ELIDE). Detection
    // only; no lowering consumes them yet, so this is behaviour-neutral.
    crate::use_analysis::dump_move_plans(data);
    // Tier-0 borrow elision (DEFAULT ON; opt-out LOFT_NO_BORROW_ELIDE). Inlines
    // Borrow-verdict vector copies before the scope/free passes. `elide_borrows`
    // refuses to elide a `v` that another var borrows (its `deps` point at `v`),
    // which is the dogfood-found dangling-dep hazard. The copy mechanism stays the
    // substrate; the opt-out forces the always-correct copy (the A-B lever).
    if std::env::var_os("LOFT_NO_BORROW_ELIDE").is_none() {
        elide_borrows(data);
    }
    // @PLN90 phase B (B1.3) — last-use MOVE-elision: build a dead-after owned source directly
    // into its destination slot instead of copy-then-free. Gated on `LOFT_MOVE_ELIDE`; a no-op
    // off (byte-identical). Runs AFTER borrow elision — the two cover disjoint verdicts (borrow
    // vs owned copy), so ordering is safe, but move-elision assumes the copy still stands.
    move_elide(data);
    // @PLN101 — insert value-struct copies (BEFORE the ownership scan, so the emitted
    // OpDatabase makes each copied local classify Owned on its own).
    value_struct_copy(data);
    // Plan-57 store-identity gate (Phase 2.5): emit the verifying store ops only
    // when LOFT_STORE_TAG is set.  Counter is global so ids are unique across
    // functions (a cross-function wrong-store free mismatches).
    let tag_mode = std::env::var("LOFT_STORE_TAG").is_ok();
    let mut tag_counter = 1u16;
    // Plan-57 Phase 5: last-use freeing (reclaim) is ON by default.  `LASTUSE_RECLAIM_OFF`
    // disables it for A/B watermark measurement.  The Goal-E enforcement assert runs in
    // debug builds always, and in release on demand via `LOFT_STORE_GUARD`.
    let reclaim_off = std::env::var("LASTUSE_RECLAIM_OFF").is_ok();
    let reclaim_guard = cfg!(debug_assertions) || std::env::var("LOFT_STORE_GUARD").is_ok();
    // Positive-control fault injection (test-only, never set in production): skip the
    // early-free insertion below while STILL running the Phase-4 guard, so a program
    // with reclaim-eligible stores trips the assertion.  This makes the Goal-E guard
    // *falsifiable* — proving it fires on a real reclaim regression, so its silence on
    // the corpus is evidence.  Differs from `LASTUSE_RECLAIM_OFF` (which also disables
    // the guard); correctness is preserved either way by the scope-exit `OpFreeRef`.
    let inject_unfreed = reclaim_guard && std::env::var("LOFT_STORE_GUARD_INJECT").is_ok();
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) || data.def(d_nr).variables.done {
            continue;
        }
        let free_ref_nr = data.def_nr("OpFreeRef");
        let orig_code = data.definitions[d_nr as usize].code.clone();
        let orig_vars = Function::copy(&data.def(d_nr).variables);
        // Phase 1: the normal scan → apply → set-scope pass.
        run_scan_phase(
            data,
            database,
            d_nr,
            &orig_code,
            &orig_vars,
            &HashMap::new(),
        );
        // Plan-57 cluster I-a — two-phase scan.  If a vector store is block-confined,
        // re-scan registering its `__vdb` (+ backed local) at the confined block scope
        // so the block-exit `free_vars` sweep frees the store there, then relocate the
        // null-init into that block (so its `first_def` / codegen free live there too).
        let confined = store_confinement(
            &data.definitions[d_nr as usize].code,
            &data.definitions[d_nr as usize].variables,
            free_ref_nr,
            data.def_nr("OpGetField"),
        );
        // @PLN35 — the `..rest` store-lifetime OBSERVER (reporting only; no IR change).
        if std::env::var("LOFT_REST_ORACLE").is_ok() {
            rest_store_oracle(
                &data.definitions[d_nr as usize].code,
                &data.definitions[d_nr as usize].variables,
                free_ref_nr,
                data.def_nr("OpDatabase"),
                data.def(d_nr).name(),
            );
        }
        if !confined.is_empty() {
            let mut cmap: HashMap<u16, u16> = HashMap::new();
            for (&vdb, &(local, b)) in &confined {
                cmap.insert(vdb, b);
                // Register the backed local at the block only when it is
                // single-store (its lifetime == the store's).  A multi-store
                // local (shared `z`) spans several sibling blocks, so it stays
                // function-scoped; only its per-block stores move.
                if data.def(d_nr).variables.tp(local).depend().len() == 1 {
                    cmap.insert(local, b);
                }
            }
            run_scan_phase(data, database, d_nr, &orig_code, &orig_vars, &cmap);
            for (&vdb, &(_local, b)) in &confined {
                relocate_null_init(&mut data.definitions[d_nr as usize].code, vdb, b);
            }
        }
        // Plan-57 last-use freeing, Phase 3 (DEFAULT since Phase 5): null-init
        // relocation + early free — the combination that lowers the body-0-locked
        // watermark.  Runs before compute_intervals so the moved first_def is
        // reflected.  `LASTUSE_RECLAIM_OFF` disables it for A/B measurement.
        if !reclaim_off {
            let db_nr = data.def_nr("OpDatabase");
            let gf_nr = data.def_nr("OpGetField");
            if !inject_unfreed {
                let d = &mut data.definitions[d_nr as usize];
                lastuse_reclaim(&mut d.code, &d.variables, db_nr, gf_nr, free_ref_nr);
            }
            // Plan-57 Phase 4 — Goal-E enforcement (THE watermark guard, supersedes
            // the scope-exit `store_lifetime_guard`).  Every store the model says is
            // dead and reclaim claimed (its `intent`) must now be freed before its
            // sibling allocates; a non-zero count is a reclaim regression — the rule
            // silently re-acquired an exception.  On in debug; release on demand via
            // LOFT_STORE_GUARD (`reclaim_guard`); zero-cost otherwise.
            if reclaim_guard {
                let d = &data.definitions[d_nr as usize];
                let unfreed =
                    reclaim_unfreed_eligible(&d.code, &d.variables, db_nr, gf_nr, free_ref_nr);
                assert_eq!(
                    unfreed, 0,
                    "plan-57 Phase 4: {} left {unfreed} reclaim-eligible store(s) live-but-dead past a later alloc",
                    d.name,
                );
            }
        }
        // Plan-57 store-identity gate (Phase 2.5): rewrite store ops to verifying
        // variants (gated; no-op in normal builds).
        if tag_mode {
            let db_nr = data.def_nr("OpDatabase");
            let gf_nr = data.def_nr("OpGetField");
            let store_tag_nr = data.def_nr("OpStoreTag");
            let free_ref_tag_nr = data.def_nr("OpFreeRefTag");
            // Scope the gate to the reclaim-eligible stores (the same `owning` set
            // `lastuse_reclaim` acts on).  Adopted / shared / file-reused stores are
            // NOT eligible, so they stay untagged with plain OpFreeRef — the gate
            // verifies exactly the frees reclaim is responsible for and cannot
            // false-positive on legitimate store-sharing.
            let d = &data.definitions[d_nr as usize];
            let (owning, _intent) =
                reclaim_free_intent(&d.code, &d.variables, db_nr, gf_nr, free_ref_nr);
            let tagset: HashSet<u16> = owning.into_iter().collect();
            let mut ids: HashMap<u16, u16> = HashMap::new();
            tag_stores(
                &mut data.definitions[d_nr as usize].code,
                db_nr,
                free_ref_nr,
                store_tag_nr,
                free_ref_tag_nr,
                &tagset,
                &mut ids,
                &mut tag_counter,
            );
        }
        // Compute live intervals so validate_slots can check for slot conflicts after codegen.
        let free_text_nr = data.def_nr("OpFreeText");
        // Plan-57 cluster I: store-lifetime guard (diagnostic, gated).
        if std::env::var("LOFT_STORE_GUARD").is_ok() {
            let gf_nr = data.def_nr("OpGetField");
            let d = &data.definitions[d_nr as usize];
            store_lifetime_guard(&d.code, &d.variables, free_ref_nr, gf_nr, &d.name);
        }
        let code_ref = data.definitions[d_nr as usize].code.clone();
        let create_stack_nr = data.def_nr("OpCreateStack");
        let mut seq = 0u32;
        compute_intervals(
            &code_ref,
            &mut data.definitions[d_nr as usize].variables,
            free_text_nr,
            free_ref_nr,
            create_stack_nr,
            &mut seq,
            0,
        );
        // Plan-57 last-use freeing, Phase 1: definition-point liveness diagnostic
        // (read-only).  Reports each function-scoped owning store held past its
        // last use while later allocations run — the I-b / III-straight-line
        // watermark divergence.  See fix-design-last-use-freeing.md.
        if std::env::var("LOFT_LASTUSE_GUARD").is_ok() {
            let db_nr = data.def_nr("OpDatabase");
            let gf_nr = data.def_nr("OpGetField");
            let d = &data.definitions[d_nr as usize];
            last_use_guard(&d.code, &d.variables, db_nr, gf_nr, free_ref_nr, &d.name);
        }
        // Plan-04 close-out (2026-04-22): V1 remains the slot
        // allocator.  The Phase 2h "codegen is the allocator" pivot
        // and the V2-drive alternative both failed on variables
        // declared at an outer scope but first-Set in an inner scope
        // (e.g. match-arm pattern bindings lifted to body scope by
        // `scan_if`'s `small_both` pre-registration).  V1's zone-1
        // pre-pass is load-bearing — see
        // `doc/claude/plans/finished/04-slot-assignment-redesign/README.md`
        // § Status.  Invariants I1–I7 in `validate.rs` check V1's
        // output at every codegen completion (debug / test builds).
        // @PLAN53 cluster 2 / S4: in aligned mode each arg + the return-address
        // slot occupies a STEPPED span, so the locals start at Σ step(arg) +
        // step(4) — matching codegen's stepped args loop + return slot, which
        // keeps the frame base (args_base) 8-aligned.  Identity when off.
        let local_start: u16 = {
            let vars = &data.definitions[d_nr as usize].variables;
            let step = |s: u16| crate::variables::aligned_stack_step(u32::from(s)) as u16;
            let arg_size: u16 = vars
                .arguments()
                .iter()
                .map(|&a| step(size(vars.var_type(a), &Context::Argument)))
                .sum();
            arg_size + step(4) // return-address slot
        };
        // @PLAN53 — the aligned V2 allocator is the ONLY allocator.  Compute the
        // V2 layout from the (immutable) function intervals, reset stale local
        // slots, then apply it.  `apply_v2_result` also zeroes every block's
        // var_size: V2 is scope-blind, a single function-entry reserve (frame
        // hwm) covers all slots, so there are no per-block reserves.
        let result = {
            let d = &data.definitions[d_nr as usize];
            crate::variables::assign_slots_v2(&d.variables, local_start)
        };
        {
            let d = &mut data.definitions[d_nr as usize];
            d.variables.reset_local_slots();
            crate::variables::apply_v2_result(&mut d.variables, &mut d.code, &result);
        }
        #[cfg(debug_assertions)]
        {
            crate::variables::validate_slots(
                &data.definitions[d_nr as usize].variables,
                data,
                d_nr,
                true, // V2 is scope-blind — skip I7 (zone-frame invariant).
            );
            crate::variables::validate_alignment(&data.definitions[d_nr as usize].variables);
        }
    }
    // @PLN94 C.0 (DEV tier) — the POST-codegen free-based checks (over-free / under-free), now that
    // `get_free_vars` has inserted the frees into `def.code` above. Self-gates on
    // `LOFT_OWN_ORACLE=check-dev`; observer only (SI-1), a no-op on the default `check` path.
    crate::ownership_cfg::oracle_free_checks(data);
    // #682 — record which closure captures the record ADOPTS, now that every dep
    // rewrite above has settled.  Must run after the loop, not inside it: the
    // verdict is the same `owns` fact `get_free_vars` uses, and that fact is only
    // final once the call-result rewrites (`make_independent`) have run.
    mark_borrowed_captures(data);
    // `LOFT_VAR_TABLE=<fn substring>` — the variable table beside the IR dump, with
    // each type dep resolved to `name(index)`.  Observer only; a no-op when unset.
    crate::variables::dump_var_tables(data, 0);
}

/// #682 — decide, per closure-record capture, whether the record ADOPTS the
/// captured store (`free_named`'s cascade reclaims it) or merely BORROWS it, and
/// mark the borrowed ones on the record's attribute.
///
/// Adoption exists for exactly one reason: a store the DEFINING frame owned and
/// would otherwise free at scope exit.  `get_free_vars` suppresses that free for
/// a captured reference and hands the store to the record, which is what keeps an
/// escaping factory closure's capture alive past the frame that minted it (#323).
/// A capture with no such free to hand over is a BORROW — a PARAMETER (whose
/// caller owns the store and outlives this frame) or a projection local viewing
/// into someone else's store — and cascading there is a second free of a live
/// store: the caller's value went dangling and the crash surfaced thousands of
/// ops later in whatever function next touched it.
///
/// Why here and not at record synthesis: a capture's ownership is not knowable
/// while parsing.  `ch = pick(w, 1)` parses as "borrows `w`" from the callee's
/// declared return, and only the scan above rewrites it to OWNED once it knows
/// the return ABI deep-copies into a fresh store (`make_independent`, the
/// `!adopts_fresh_store` arm).  Reading the parse-time dep leaked that copy.
///
/// The record is reached through the enclosing frame's `___clos_N` variable
/// rather than by walking the IR: `emit_lambda_code` always mints one, and it
/// lives in exactly the frame whose variables decide the verdict.
fn mark_borrowed_captures(data: &mut Data) {
    let mut borrowed: Vec<(u32, usize)> = Vec::new();
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let function = &data.def(d_nr).variables;
        for v in 0..function.next_var() {
            // The DEFINING frame holds the record in the `___clos_N` local
            // `emit_lambda_code` mints for it.  A frame that receives the record as
            // an ARGUMENT is the closure BODY (its hidden `__closure` parameter),
            // whose own variable table knows nothing about who owns the captures —
            // reading it flipped the verdict depending on definition order.
            if function.is_argument(v) {
                continue;
            }
            let Type::Reference(record, _) = function.tp(v) else {
                continue;
            };
            let record = *record;
            if !data.def(record).name.starts_with("__closure_") {
                continue;
            }
            for a in 0..data.attributes(record) {
                if !capture_attr_is_cascade_relevant(data, record, a) {
                    continue;
                }
                if !record_adopts_capture(data, function, record, a) {
                    borrowed.push((record, a));
                }
            }
        }
    }
    for (record, a) in borrowed {
        data.mark_capture_borrowed(record, a);
    }
}

/// The `__ref_N` work-ref an inline record LITERAL built, when the literal's value is that
/// buffer itself — the shape `c: S? = S { x: 5 }` lowers to.
///
/// `None` for every other right-hand side, which is what keeps this narrow: a block whose
/// value is a work-ref it did not `OpDatabase` into is someone else's store, and a dense
/// local has no buffer at all.
fn inline_literal_work_ref(rhs: &Value, function: &Function, data: &Data) -> Option<u16> {
    let Value::Block(bl) = rhs.unspan() else {
        return None;
    };
    let last = bl
        .operators
        .iter()
        .rev()
        .find(|o| !matches!(o.unspan(), Value::Line(_)))?;
    let Value::Var(av) = last.unspan() else {
        return None;
    };
    let av = *av;
    if av >= function.count() {
        return None;
    }
    let name = function.name(av);
    if !name.starts_with("__ref_") && !name.starts_with("__rref_") {
        return None;
    }
    // The store has to be MINTED here, not merely named: `OpDatabase(av, …)` is the mint.
    let db = data.def_nr("OpDatabase");
    let built_here = bl.operators.iter().any(|o| {
        matches!(o.unspan(), Value::Call(d, args) if *d == db
            && matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == av))
    });
    built_here.then_some(av)
}

/// Every construction work-ref a right-hand side DELIVERS to the binding it is assigned to:
/// [`inline_literal_work_ref`] at the value's tail, reached through each `if` arm (a `match`
/// lowers to `if`s) and each block's last operator.  `if c { S { … } } else { S { … } }`
/// delivers one of two, and the binding adopts whichever arm ran.
fn adopted_work_refs(rhs: &Value, function: &Function, data: &Data, out: &mut Vec<u16>) {
    let tail = |ops: &[Value]| {
        ops.iter()
            .rev()
            .find(|o| !matches!(o.unspan(), Value::Line(_)))
            .cloned()
    };
    match rhs.unspan() {
        Value::If(_, t, f) => {
            adopted_work_refs(t, function, data, out);
            adopted_work_refs(f, function, data, out);
        }
        Value::Block(bl) => {
            if let Some(w) = inline_literal_work_ref(rhs, function, data) {
                out.push(w);
            } else if let Some(last) = tail(&bl.operators) {
                adopted_work_refs(&last, function, data, out);
            }
        }
        Value::Insert(ops) => {
            if let Some(last) = tail(ops) {
                adopted_work_refs(&last, function, data, out);
            }
        }
        _ => {}
    }
}

/// Does a closure record's adoption take over the frame-exit free of local `v`?
///
/// One home for the question `get_free_vars` asks before emitting a free and
/// [`check_ref_leaks`] asks before calling an unfreed local a leak. They are the emitter and
/// its static mirror, so they have to answer identically: a suppression the mirror does not
/// know about reads as a leak, and a leak the mirror excuses reads as nothing at all.
///
/// Both spellings of "reaches the store" are needed. A struct capture names the local that
/// holds the store outright; a collection capture names a VIEW, and the store lives in the
/// backing local no closure captured by name — which is what
/// [`backs_an_adopted_capture`] asks about. `is_dbref(.base())` is the shape test both
/// halves share: a capture's store may be a `Vector` or a keyed collection, not only a
/// `Reference`.
///
/// ⚠ Restating this rule is how it drifts. loft#1308 was the free emitter and this mirror
/// disagreeing on the shape test; the mirror then kept the `is_captured` half alone and a
/// capturing closure over a local vector false-positived the leak assert. Three consumers
/// now share it — those two and `ownership_cfg`'s leak oracle.
pub(crate) fn capture_adoption_owns_free(
    data: &Data,
    function: &Function,
    built_with: &CaptureBuilds,
    v: u16,
) -> bool {
    // @FR-O-Latest — the record adopts the store the capture named AT THE BUILD, so the
    // handover is only sound while the local still names that store.  A local assigned again
    // after the build names a different one, which nothing else frees: the record's cascade
    // reaches the adopted store and the frame's free was suppressed on the strength of the
    // BINDING being captured.  loft#1324 closed this for the collection half, where the
    // backing local is found through `built_with`; the direct half asked `is_captured` and
    // kept suppressing whatever the local named LAST, so `s = S{…}; h = |i| { s.a + i };
    // s = build(h)` leaked the store `s` ends up holding, on both backends, once per
    // reassignment and once per pass of a loop (loft#1388).
    !built_with.reassigned_after_build.contains(&v)
        && (function.is_captured(v) || backs_an_adopted_capture(data, function, built_with, v))
        && crate::data::is_dbref(function.tp(v).base())
}

/// The backing local a capture named AT THE CLOSURE BUILD — the store the record actually
/// holds — or `None` when the code does not settle it.
///
/// A capture's store is decided by @FR-O-Latest: ownership belongs to the LATEST assignment
/// *at that point*, and a type-level `deps` list cannot express a point.  For a local assigned
/// once the two coincide and the type is enough; for one REASSIGNED after the build they name
/// different stores, and reading the type then aims the frame-exit suppression at the store the
/// closure does NOT hold.  Both directions are wrong at once: the store the record adopted is
/// freed by the frame as well (an escaping closure reads a released store), and the store the
/// local now names is suppressed although nobody adopted it (it leaks) — loft#1324.
///
/// The build is `OpSetDbRef(___clos_N, <offset>, <capture>)`, so walking the body in order and
/// remembering each local's most recent backing root answers it directly.  [`Value::walk`] is
/// pre-order over the children in source order, which is the ordering this needs; a hand-rolled
/// descent here would be the fourth copy of one that has drifted before.
///
/// A capture that owns its store outright — a struct — has no backing root and is not in the
/// map, and neither is one whose build this body does not contain.  Both fall back to the type
/// dep, which is the only fact available for them and is right whenever the local is assigned
/// once.
pub(crate) fn capture_build_backings(
    data: &Data,
    function: &Function,
    code: &Value,
) -> CaptureBuilds {
    let set_dbref = data.def_nr("OpSetDbRef");
    let mut latest: HashMap<u16, u16> = HashMap::new();
    let mut out = CaptureBuilds::default();
    captures_built_in_a_loop(code, set_dbref, false, &mut out.rebuilt_in_loop);
    let mut built: HashSet<u16> = HashSet::new();
    // Capture vars already resolved at their enclosing statement, waiting for the walk to
    // reach the build node itself.  See the `Value::Set` arm.
    let mut resolved_in_rhs: HashSet<u16> = HashSet::new();
    code.walk(&mut |node: &Value| match node.unspan() {
        Value::Set(v, rhs) => {
            // A build inside this statement's OWN right-hand side captures the value the
            // local held BEFORE the assignment — `s = build(|i| { s.a + i })` hands the
            // closure the store `s` is about to stop naming.  `Value::walk` is pre-order, so
            // the build node is reached AFTER this one, by which time `latest` describes the
            // assignment rather than the capture.  Resolve those builds here, against
            // `latest` as it still stands, and let the walk skip them when it arrives.
            for (c, backing) in captures_built_in(data, rhs, set_dbref, &latest) {
                built.insert(c);
                if let Some(b) = backing {
                    out.backing.insert(c, b);
                    built.insert(b);
                }
                resolved_in_rhs.insert(c);
                // @FR-O-Latest — this very assignment is the one that moves the local off
                // the store the record just adopted.
                if c == *v || backing == Some(*v) {
                    out.reassigned_after_build.insert(*v);
                }
            }
            // @FR-O-Latest, the OTHER half of loft#1324.  The record holds the store the
            // capture named at the BUILD; a local assigned again after that names a different
            // one, and the frame is the only thing left to free it.
            if built.contains(v) {
                out.reassigned_after_build.insert(*v);
            }
            match crate::use_analysis::view_root_slots(data, rhs).as_deref() {
                Some([root]) if *root != *v && !function.is_argument(*root) => {
                    latest.insert(*v, *root);
                }
                // A right-hand side that names no single root leaves no backing to remember,
                // and the stale one would be worse than none: drop it.
                _ => {
                    latest.remove(v);
                }
            }
        }
        Value::Call(d, args) if *d == set_dbref => {
            if let Some(Value::Var(c)) = args.get(2).map(Value::unspan) {
                if resolved_in_rhs.remove(c) {
                    return;
                }
                built.insert(*c);
                if let Some(&backing) = latest.get(c) {
                    out.backing.insert(*c, backing);
                    built.insert(backing);
                }
            }
        }
        _ => {}
    });
    out
}

/// The captures a right-hand side BUILDS, each with the backing local it names.
///
/// Two spellings, the pair [`capture_build_backings`] itself carries: a struct capture names
/// the local outright (no backing), a collection capture names a VIEW whose root is the local.
/// A view minted inside this same right-hand side is resolved from the statements walked here;
/// anything older comes from `outer`, which describes the program up to — and not including —
/// the assignment this right-hand side belongs to.
fn captures_built_in(
    data: &Data,
    rhs: &Value,
    set_dbref: u32,
    outer: &HashMap<u16, u16>,
) -> Vec<(u16, Option<u16>)> {
    let mut latest = outer.clone();
    let mut found: Vec<(u16, Option<u16>)> = Vec::new();
    rhs.walk(&mut |node: &Value| match node.unspan() {
        Value::Set(c, src) => match crate::use_analysis::view_root_slots(data, src).as_deref() {
            Some([root]) if root != c => {
                latest.insert(*c, *root);
            }
            _ => {
                latest.remove(c);
            }
        },
        Value::Call(d, args) if *d == set_dbref => {
            if let Some(Value::Var(c)) = args.get(2).map(Value::unspan) {
                found.push((*c, latest.get(c).copied()));
            }
        }
        _ => {}
    });
    found
}

/// What a body's closure BUILDS say about the locals they capture.
///
/// Two facts, one walk, because both are read off the same `OpSetDbRef(___clos_N, …, capture)`
/// point and a second walk would be a copy of an ordering that has drifted before.
#[derive(Default)]
pub(crate) struct CaptureBuilds {
    /// The backing local a capture named AT THE BUILD — the store the record actually holds.
    /// A capture that owns its store outright (a struct) has no backing root and is absent.
    pub(crate) backing: HashMap<u16, u16>,
    /// Locals whose store the record adopted and which were ASSIGNED AGAIN afterwards, so the
    /// local no longer names the adopted store and the frame still owes its own free.
    pub(crate) reassigned_after_build: HashSet<u16>,
    /// Captures whose closure BUILD sits inside a loop, so the record's slot is rewritten on
    /// every pass and only the LAST adoption is the one it still holds.
    pub(crate) rebuilt_in_loop: HashSet<u16>,
}

/// Is `v` the store behind a capture whose closure record ADOPTS it?
///
/// `get_free_vars` suppresses a captured local's scope-exit free by asking `is_captured` of
/// the local it is about to free — but a collection capture names a VIEW, and the local
/// holding the store is the backing one, which no closure captured by name. This asks the
/// question the other way round: does some captured local in this frame reach `v`?
///
/// ⚠ It gates on `frame_owns_capture_store`, the SAME predicate `record_adopts_capture`
/// uses, and that is the whole point. Suppressing the free and adopting the store have to be
/// one decision: suppress without adopting and the store is never freed at all, adopt without
/// suppressing and it is freed twice. An earlier cut answered them in two places — a parse-time
/// mark for the free and this pass for the verdict — and a capture the verdict called BORROWED
/// had already had its backing free suppressed, so it leaked
/// (`1248-a-capture-that-cannot-be-borrowed-from`). Asking one function keeps them from
/// disagreeing by construction.
pub(crate) fn backs_an_adopted_capture(
    data: &Data,
    function: &Function,
    built_with: &CaptureBuilds,
    v: u16,
) -> bool {
    (0..function.next_var()).any(|c| {
        c != v
            && function.is_captured(c)
            // @FR-O-Latest, the collection half of the same sentence — and only where the
            // record REBUILDS.  A build inside a loop rewrites its capture slot on every pass,
            // so the backing the FIRST pass adopted is not what the record holds at the end
            // and suppressing its free hands the store to an adoption two passes stale
            // (measured: `__vdb_1` never freed in a vector-capture loop).  A build that runs
            // ONCE is the opposite case and must keep its suppression however often the
            // capture is reassigned afterwards — that is #323's escaping factory, whose
            // record outlives the frame; declining there frees the store the escaped closure
            // still reads, which `1324-a-reassigned-capture-suppresses-the-store-the-record-\
            // holds` catches as `null(oob)`.
            && !built_with.rebuilt_in_loop.contains(&c)
            && capture_is_adopted(data, function, c)
            && match built_with.backing.get(&c) {
                // @FR-O-Latest — the record holds the store this capture named AT THE BUILD, so
                // that is the one local whose free it takes over.  Reading the type dep instead
                // aims the suppression at whatever the local names LAST, which for a capture
                // reassigned after the build is a different store: the adopted one is then freed
                // by the frame as well and an escaping closure reads a released store, while the
                // one the local now names is suppressed although nobody adopted it and leaks
                // (loft#1324).
                Some(&backing) => backing == v,
                // No build point in this body, or a right-hand side naming no single root: the
                // type dep is the only fact there is, and it is right whenever the local is
                // assigned once.
                None => backing_chain(function, c).contains(&v),
            }
    })
}

/// Would [`mark_borrowed_captures`] ADOPT the capture named by local `c`?
///
/// Asked through the record rather than off `c` alone, because that pass declines captures
/// this frame nonetheless owns: its attribute filter admits only a `Reference` attribute with
/// non-empty deps — the cascade-relevant share marker — and a capture outside that class keeps
/// its frame-exit free. `test_a_store_backed_capture_still_declines_and_still_answers` is one
/// on purpose (loft#1248's minting capture, which still declines the lift), and reading only
/// `frame_owns_capture_store` here suppressed its free while the record declined to adopt it,
/// so the store leaked at program exit.
fn capture_is_adopted(data: &Data, function: &Function, c: u16) -> bool {
    let name = function.name(c);
    for w in 0..function.next_var() {
        if function.is_argument(w) {
            continue;
        }
        let Type::Reference(record, _) = function.tp(w) else {
            continue;
        };
        let record = *record;
        if !data.def(record).name.starts_with("__closure_") {
            continue;
        }
        for a in 0..data.attributes(record) {
            if data.attr_name(record, a) != name {
                continue;
            }
            if !capture_attr_is_cascade_relevant(data, record, a) {
                return false;
            }
            return record_adopts_capture(data, function, record, a);
        }
    }
    false
}

/// Is capture attribute `a` of `record` one the record's death CASCADES through?
///
/// Only the two share markers are: the attribute holds a 12-byte DbRef for `free_named` to
/// follow. An inline-bytes capture — a `text` copy, empty deps — holds no reference, so there
/// is nothing to reclaim and nothing to suppress.
///
/// One home because two callers must agree exactly. `mark_borrowed_captures` uses it to decide
/// which captures get a verdict at all, and `capture_is_adopted` to decide whether a frame-exit
/// free may be suppressed; a capture the first skips must not be one the second adopts, or the
/// store is freed twice. Restating it in the second place is how loft#1308 was written the
/// first time.
fn capture_attr_is_cascade_relevant(data: &Data, record: u32, a: usize) -> bool {
    matches!(data.attr_type(record, a).base(), Type::Reference(_, deps) if !deps.is_empty())
}

/// The locals that BACK `start`, nearest first — the chain `frame_owns_capture_store` walks
/// to reach the one that owns the store.
///
/// Empty when `start` owns its store directly, which is the struct case: there is nothing
/// behind it to mark.
fn backing_chain(function: &Function, start: u16) -> Vec<u16> {
    let mut chain = Vec::new();
    let mut v = start;
    for _ in 0..8 {
        let dep = function.tp(v).depend();
        if dep.len() != 1 || dep[0] == v || function.is_argument(dep[0]) {
            break;
        }
        v = dep[0];
        chain.push(v);
    }
    chain
}

/// Does the closure record own the store behind capture `a` of `record`, as seen
/// from the defining frame `function`?  See [`mark_borrowed_captures`].
fn record_adopts_capture(data: &Data, function: &Function, record: u32, a: usize) -> bool {
    // A `__cell_<T>` is minted FOR this closure (plan-22 boxes a mutated scalar /
    // text capture into one), so the record is its only possible owner however the
    // original binding was reached — including from a parameter.
    //
    // ...as long as it was minted for a binding THIS frame has.  A lambda nested in a lambda
    // RELAYS the capture outward: the enclosing lambda holds no binding of the name at all and
    // reads the handle out of its own closure record, so the record it builds is a second
    // pointer at a cell that belongs further out.  Adopting it freed the cell when the
    // enclosing lambda returned — on its FIRST call, so the second one read a released store
    // (loft#1236).
    if let Type::Reference(cell, _) = data.attr_type(record, a)
        && data.def(cell).name.starts_with("__cell_")
        && function.var(&data.attr_name(record, a)) != u16::MAX
    {
        return true;
    }
    let v = function.var(&data.attr_name(record, a));
    // An unresolvable name defaults to BORROW: an unfreed store is a leak the
    // store checker reports, while an extra free silently corrupts a caller.
    // A parameter never enters the scope-exit sweep at all (`variables()`:
    // "never return function arguments"), so it has no free to hand over.
    if v == u16::MAX || function.is_argument(v) {
        return false;
    }
    // The same test as `get_free_vars`' `owns`, so the two cannot drift: empty
    // deps means owned, and a keyed collection's self-dep is an ownership marker
    // rather than a borrow (@P302) — asked of the local the capture NAMES, and
    // then of whatever backs it.
    frame_owns_capture_store(function, v)
}

/// Does the defining frame own the store behind local `v` — directly, or through the
/// backing local a collection VIEW depends on?  See [`mark_borrowed_captures`].
///
/// A struct local owns its store outright (`s: ref(726) OWNS`, empty deps) and the plain
/// `dep.is_empty()` test saw that.  A vector local does not: `v = [7,2,3]` compiles to a
/// VIEW whose deps name a separate `__vdb_N` local — `v: vec<int> deps=[__vdb_1(2)]`
/// beside `__vdb_1: ref(467) OWNS` — and the frame frees the BACKING local at scope exit.
/// Reading only `v`'s own deps therefore answered "borrow" for a store this frame really
/// does own and really does free, so the record declined the handover and the escaped
/// closure read a released store (loft#1308).
///
/// Ownership is what is being followed, not merely a dep edge: the walk stops at an
/// ARGUMENT, whose store belongs to the caller and outlives this frame, so a capture that
/// projects into a parameter stays the BORROW that #682 made it. The bound is a guard
/// against a cyclic dep chain, not a depth the language imposes.
fn frame_owns_capture_store(function: &Function, start: u16) -> bool {
    let mut v = start;
    for _ in 0..8 {
        let tp = function.tp(v);
        let dep = tp.depend();
        if dep.is_empty() {
            return true;
        }
        // A keyed collection's self-dep is an ownership marker, not a borrow (@P302).
        if dep.len() == 1 && dep[0] == v && crate::parser::vectors::is_keyed(tp) {
            return true;
        }
        // More than one dep names no single backing store to follow, and a self-dep that
        // is not the keyed marker is not one either.
        if dep.len() != 1 || dep[0] == v {
            return false;
        }
        // The caller owns a parameter's store and outlives this frame: no free to hand over.
        if function.is_argument(dep[0]) {
            return false;
        }
        v = dep[0];
    }
    false
}

/// Walk `ir` and panic if any `Call` or `CallRef` argument directly contains a
/// `Set(ref_var, Null)` for an owned Reference (dep empty).
///
/// Such a nested allocation would place `ConvRefFromNull` on the eval stack
/// *between* call arguments, corrupting the arg layout and producing a garbage
/// `y` value inside the callee — the root cause of the A5.6 "Incorrect store"
/// bug.  `scan_args` in scopes.rs is responsible for bubbling these out; if it
/// misses a case this check catches it at compile time.
#[cfg(debug_assertions)]
fn check_arg_ref_allocs(ir: &Value, function: &Function, fn_name: &str) {
    fn check_args(args: &[Value], function: &Function, fn_name: &str) {
        for a in args {
            if let Value::Insert(ops) = a
                && ops.len() >= 2
                && let Value::Set(v, val) = &ops[0]
                && matches!(val.as_ref(), Value::Null)
                && function.tp(*v).is_heap_owned()
            {
                panic!(
                    "[check_arg_ref_allocs] Set('{name}', Null) for owned heap type \
                     is nested inside a Call/CallRef argument in '{fn_name}'. \
                     This corrupts the CallRef arg layout (A5.6). \
                     scan_args in scopes.rs must bubble it out.",
                    name = function.name(*v),
                );
            }
            walk_check(a, function, fn_name);
        }
    }
    fn walk_check(ir: &Value, function: &Function, fn_name: &str) {
        match ir {
            Value::Call(_, args) | Value::CallRef(_, args) => {
                check_args(args, function, fn_name);
            }
            Value::Set(_, inner) => walk_check(inner, function, fn_name),
            Value::If(cond, t, f) => {
                walk_check(cond, function, fn_name);
                walk_check(t, function, fn_name);
                walk_check(f, function, fn_name);
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    walk_check(op, function, fn_name);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    walk_check(op, function, fn_name);
                }
            }
            // Every value-WRAPPING node, not a chosen subset: the shape this walker
            // reports — a `Set(v, Null)` nested inside a call argument — can sit under any
            // of them, so one omitted wrapper is a silently missed diagnostic.
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                walk_check(inner, function, fn_name);
            }
            _ => {}
        }
    }
    walk_check(ir, function, fn_name);
}

impl Scopes<'_> {
    /// The type's scope-end hook for `v`, unless a MOVE-copy already released `v`'s
    /// store — see [`collect_drop_transferred`].  `@FR-H-Drop`: the owner's scope-end
    /// clause.  One home for the rule, because
    /// both emission sites (the buffer-adoption leg and the ordinary one) must agree:
    /// a drop that runs on a released store is a use-after-free either way.
    fn scope_end_drop(&self, function: &Function, v: u16, data: &Data) -> Option<Value> {
        if self.drop_transferred.contains(&v) {
            return None;
        }
        drop_hook(function, v, data)
    }

    /// Record what the `__lift_N` temp `tmp` — holding argument `arg_idx` of the call
    /// `scan_args` is lowering — no longer owns, because the call takes it over.
    ///
    /// Two different hand-offs, and they cost different things to get wrong:
    ///
    /// - the **drop** (@PLN139 stage C): a copy into a container field or a collection
    ///   element makes the container the releaser, so running the source's own scope-end
    ///   hook releases one resource twice.
    /// - the **store** (loft#890): a `0x8000` move FREES the source store inside the op.
    ///   The lift still names it, so its scope-exit `OpFreeRef` is a second free — silent
    ///   while the slot stays free, and a stolen store the moment the allocator hands
    ///   that slot to somebody else.  `br = mk_hash(n); br[7, 0]` in a record-returning
    ///   function is exactly that: the return buffer is allocated between the two frees
    ///   and lands on the recycled slot, so the function returns freed bytes.
    fn mark_lift_handoff(
        &mut self,
        tmp: u16,
        arg_idx: usize,
        transfer_copy: bool,
        moved_arg: Option<usize>,
    ) {
        if transfer_copy && arg_idx == 0 || moved_arg == Some(arg_idx) {
            self.drop_transferred.insert(tmp);
        }
        if moved_arg == Some(arg_idx) {
            self.free_transferred.insert(tmp);
        }
    }

    fn enter_scope(&mut self) -> u16 {
        self.stack.push(self.scope);
        self.scope = self.max_scope;
        self.max_scope += 1;
        self.scope
    }

    fn exit_scope(&mut self) {
        if let Some(scope) = self.stack.pop() {
            self.scope = scope;
        }
    }

    fn scan(&mut self, val: &Value, function: &mut Function, data: &Data) -> Value {
        // loft#1320 — a KEYED branch value is not a `Set` RHS: `r = if … { g(hs) } else …`
        // lowers to `Set(r, Null)` and then `OpReplaceKeyed(if …, r, tp)`, which COPIES the
        // chosen arm into `r`'s own store and leaves the arm that MINTED with no owner.  The
        // branch is that op's first argument, so the per-arm rewrite `scan_set` applies to a
        // vector branch is applied here, with the temps homed where `r` lives.
        if let Some(rewritten) = self.rewrite_keyed_replace_branch(val, function, data) {
            return self.scan(&rewritten, function, data);
        }
        self.scan_depth += 1;
        assert!(
            self.scan_depth <= 1000,
            "expression nesting limit exceeded at depth {}",
            self.scan_depth
        );
        let result = self.scan_inner(val, function, data);
        self.scan_depth -= 1;
        result
    }

    #[allow(clippy::too_many_lines)]
    fn scan_inner(&mut self, val: &Value, function: &mut Function, data: &Data) -> Value {
        match val {
            Value::Var(ov) => Value::Var(*self.var_mapping.get(ov).unwrap_or(ov)),
            Value::Set(ov, value) => self.scan_set(*ov, value, function, data),
            Value::Loop(lp) => {
                let scope = self.enter_scope();
                self.loops.push(scope);
                function.mark_loop_scope(scope);
                // #316 — a loop body executes repeatedly: any ownership entry
                // the body touches is unreliable afterwards.  Keep only the
                // entries the body left unchanged.
                let owned_before = self.owned_refs.clone();
                let ls = self.convert(lp, function, data, false);
                self.owned_refs
                    .retain(|k, depth| owned_before.get(k) == Some(depth));
                self.loops.pop();
                self.exit_scope();
                Value::Loop(Box::new(Block {
                    operators: ls,
                    result: Type::Void,
                    name: lp.name,
                    scope,
                    var_size: 0,
                }))
            }
            Value::If(test, t_val, f_val) => self.scan_if(test, t_val, f_val, function, data),
            Value::Break(lv) => {
                let mut ls = self.get_free_vars(
                    function,
                    data,
                    self.loops[self.loops.len() - *lv as usize - 1],
                    &Type::Void,
                    u16::MAX,
                    &HashSet::new(),
                );
                if ls.is_empty() {
                    Value::Break(*lv)
                } else {
                    ls.push(Value::Break(*lv));
                    Value::Insert(ls)
                }
            }
            Value::Continue(lv) => {
                let mut ls = self.get_free_vars(
                    function,
                    data,
                    self.loops[self.loops.len() - *lv as usize - 1],
                    &Type::Void,
                    u16::MAX,
                    &HashSet::new(),
                );
                if ls.is_empty() {
                    Value::Continue(*lv)
                } else {
                    ls.push(Value::Continue(*lv));
                    Value::Insert(ls)
                }
            }
            Value::Return(v) => {
                let expr = self.scan(v, function, data);
                Value::Insert(self.free_vars(
                    true,
                    &expr,
                    function,
                    data,
                    &data.def(self.d_nr).returned,
                    1,
                ))
            }
            Value::Block(bl) => {
                // pre-register a block-result Reference variable at the OUTER scope
                // before entering the block's inner scope.
                //
                // Without this, `scan_set(w, Null)` registers w at the inner scope.
                // At block exit, `free_vars` skips w (it is `ret_var`). At function exit,
                // `variables(outer_scope)` omits w (inner scope is not in the chain) →
                // OpFreeRef is never emitted → Database N not freed.
                //
                // Pre-registering at the outer scope causes `scan_set(w, Null)` inside the
                // block to see `var_scope.contains_key(&w) && *value == Null` → return
                // Insert([]) (the Set is suppressed from inside the block). We then hoist
                // Set(w, Null) to the outer level by returning Insert([Set(w,Null), Block]).
                //
                // This is necessary (not optional) because DbRef is 12 bytes (> 8) → Zone 2
                // of slot assignment handles it. Zone 2 of the outer scope's `process_scope`
                // walks its direct operators and finds Set(w, Null) in the Insert; Zone 2 of
                // the inner scope skips w (scope mismatch). If Set(w, Null) were left inside
                // the block, the outer Zone 2 would never see it and the slot would remain
                // u16::MAX → "variable never assigned a slot" panic at codegen.
                let mut hoisted_ref: Option<u16> = None;
                if let Some(Value::Var(ret_v)) = bl.operators.last() {
                    let ret_v = *self.var_mapping.get(ret_v).unwrap_or(ret_v);
                    if !self.var_scope.contains_key(&ret_v)
                        && let Type::Reference(_, dep)
                        | Type::Vector(_, dep)
                        | Type::Enum(_, true, dep) = function.tp(ret_v)
                        && dep.is_empty()
                    {
                        self.var_scope.insert(ret_v, self.scope);
                        self.var_order.push(ret_v);
                        hoisted_ref = Some(ret_v);
                    }
                }
                // The function body block (scope 0 → 1) with a non-void
                // result needs is_return=true so frees land between the
                // tail expression and the Return, not after it.
                let is_body_return = self.scope == 0
                    && bl.result != Type::Void
                    && data.def(self.d_nr).returned != Type::Void;
                let outer_scope = self.scope;
                let lift_watermark = self.lift_vars.len();
                let scope = self.enter_scope();
                // Move hoisted var from outer scope (0) to body scope so
                // get_free_vars at body exit can find and free it.
                if let Some(w) = hoisted_ref {
                    self.var_scope.insert(w, scope);
                }
                let mut ls = self.convert(bl, function, data, is_body_return);
                self.exit_scope();
                // loft#722 — a lift temp minted INSIDE a value-producing block must
                // outlive the block, because the block's result can be a borrow INTO
                // it.  `x = f().items[0] ?? Fallback {}` lowers the `??` to a block;
                // the temp holding `f()`'s result was registered at that block's
                // scope and freed on the way out, while `x` — a binding in the
                // ENCLOSING scope — still pointed into it.  It read correctly once
                // and returned zeroes after the store was reused.
                //
                // Without the `??` the same expression is lifted at statement level
                // and is correct, which is exactly the behaviour restored here:
                // re-register at the ENCLOSING scope and drop the block-exit free,
                // so the outer scope frees it instead.
                //
                // The enclosing scope, not the function: a lift inside a LOOP body
                // then still frees once per iteration, which is what keeps a loop
                // over such an expression flat instead of accumulating a store per
                // round.
                // Not for a BODY return.  The hoist hands the temp to the enclosing
                // scope and drops the block-exit free on the promise that the outer
                // scope frees it instead — and when the value block IS the function
                // body, the enclosing scope is that same function, so the free it was
                // handed to is the one just dropped and nothing frees it at all.
                // `fn txt(n: integer) -> text { mk(n).label }` leaked one record per
                // call for exactly that reason.  A body return also does not need the
                // hoist: the return delivers its value into the CALLER's buffer, so
                // freeing the temp at body exit — the behaviour before loft#722 — is
                // both correct and what the caller's copy relies on.
                if !matches!(bl.result, Type::Void)
                    && !is_body_return
                    && self.lift_vars.len() > lift_watermark
                {
                    let fresh: Vec<u16> = self.lift_vars[lift_watermark..]
                        .iter()
                        .copied()
                        .filter(|v| self.var_scope.get(v) == Some(&scope))
                        .collect();
                    // Only the temps the RESULT actually borrows from.  Hoisting
                    // every lift in a value block moves frees that nothing was
                    // waiting on, and those then went missing entirely (27 leaked
                    // `File` stores in the file suite) — the block, not the outer
                    // scope, is the right owner when the result does not point into
                    // the temp.
                    let borrowed = Self::result_borrow_roots(&ls, data);
                    let hoisted: Vec<u16> =
                        fresh.into_iter().filter(|v| borrowed.contains(v)).collect();
                    for v in &hoisted {
                        self.var_scope.insert(*v, outer_scope);
                    }
                    if !hoisted.is_empty() {
                        ls.retain(|op| {
                            scope_free_op_var(op, data).is_none_or(|v| !hoisted.contains(&v))
                        });
                    }
                }
                let block = Value::Block(Box::new(Block {
                    operators: ls,
                    result: bl.result.clone(),
                    name: bl.name,
                    scope,
                    var_size: 0,
                }));
                if let Some(w) = hoisted_ref {
                    // Return Insert([Set(w, Null), Block]) so that:
                    // 1. Zone-2 slot assignment sees Set(w, Null) at the outer scope level.
                    // 2. get_free_vars at the outer scope emits OpFreeRef(w) on block exit.
                    Value::Insert(vec![v_set(w, Value::Null), block])
                } else {
                    block
                }
            }
            Value::Call(d_nr, args) => {
                let (preamble, ls, postamble) = self.scan_args(args, function, data, *d_nr);
                let call = Value::Call(*d_nr, ls);
                if preamble.is_empty() && postamble.is_empty() {
                    call
                } else if postamble.is_empty() {
                    let mut ops = preamble;
                    ops.push(call);
                    Value::Insert(ops)
                } else {
                    // @PLN90 / loft#506 — run the store-back postamble AFTER the call.  For a
                    // non-void call, capture the result into a temp so the Insert still yields
                    // the CALL's value; a scalar must NOT be inline-ref (freeing a scalar-as-ref
                    // corrupts the store), so use a plain slotted temp.
                    let mut ops = preamble;
                    let ret = data.def(*d_nr).returned.clone();
                    if ret == Type::Void {
                        ops.push(call);
                        ops.extend(postamble);
                    } else {
                        let is_scalar = matches!(
                            ret,
                            Type::Integer(..)
                                | Type::Float
                                | Type::Single
                                | Type::Boolean
                                | Type::Character
                        );
                        let rtmp = if is_scalar {
                            self.lift_counter += 1;
                            let name = format!("__wbret_{}", self.lift_counter);
                            let t = function.add_temp_var(&name, &ret);
                            self.var_scope.insert(t, self.scope);
                            t
                        } else {
                            self.new_lift_var(function, &ret)
                        };
                        ops.push(v_set(rtmp, call));
                        ops.extend(postamble);
                        ops.push(Value::Var(rtmp));
                    }
                    Value::Insert(ops)
                }
            }
            Value::CallRef(v_nr, args) => {
                let (preamble, ls, _postamble) = self.scan_args(args, function, data, u32::MAX);
                let call = Value::CallRef(*v_nr, ls);
                if preamble.is_empty() {
                    call
                } else {
                    let mut ops = preamble;
                    ops.push(call);
                    Value::Insert(ops)
                }
            }
            Value::Insert(ops) => {
                Value::Insert(ops.iter().map(|v| self.scan(v, function, data)).collect())
            }
            Value::Drop(inner) => {
                let scanned = self.scan(inner, function, data);
                // #490 — a discarded statement result that owns a fresh store
                // (`json_parse(x);`, `mk();`) lowers to a plain stack-pop
                // (`FreeStack` on the interpreter, a dropped Rust return value
                // on native), which never frees the store — once per iteration
                // inside a loop.  Bind it to a `__lift_N` temp instead, so
                // `get_free_vars` emits the store's `OpFreeRef` at scope exit —
                // the same machinery `scan_args` uses for owned call-argument
                // temps.  A `Set` consumes the value, so the `Drop` wrapper is
                // dropped with it.
                if let Value::Insert(mut ops) = scanned {
                    // A call whose arguments were themselves lifted arrives as
                    // `Insert([Set(__lift_i, …)…, call])` — the owned result is
                    // the final op.
                    if let Some(last) = ops.last()
                        && let Some(tp) = self.inline_struct_return(last, data, u32::MAX, function)
                    {
                        let tmp = self.new_lift_var(function, &tp);
                        let last = ops.pop().unwrap();
                        ops.push(v_set(tmp, last));
                        Value::Insert(ops)
                    } else {
                        Value::Drop(Box::new(Value::Insert(ops)))
                    }
                } else if let Some(tp) =
                    self.inline_struct_return(&scanned, data, u32::MAX, function)
                {
                    let tmp = self.new_lift_var(function, &tp);
                    v_set(tmp, scanned)
                } else {
                    Value::Drop(Box::new(scanned))
                }
            }
            Value::Iter(idx, create, next, extra) => {
                let scanned_create = self.scan(create, function, data);
                // #316 — `next`/`extra` execute once per iteration: drop any
                // ownership entry they touch (same rationale as Value::Loop).
                let owned_before = self.owned_refs.clone();
                let scanned_next = self.scan(next, function, data);
                let scanned_extra = self.scan(extra, function, data);
                self.owned_refs
                    .retain(|k, depth| owned_before.get(k) == Some(depth));
                Value::Iter(
                    *idx,
                    Box::new(scanned_create),
                    Box::new(scanned_next),
                    Box::new(scanned_extra),
                )
            }
            Value::Tuple(elems) => {
                Value::Tuple(elems.iter().map(|v| self.scan(v, function, data)).collect())
            }
            Value::TupleGet(var, idx) => {
                Value::TupleGet(*self.var_mapping.get(var).unwrap_or(var), *idx)
            }
            Value::TuplePut(var, idx, inner) => Value::TuplePut(
                *self.var_mapping.get(var).unwrap_or(var),
                *idx,
                Box::new(self.scan(inner, function, data)),
            ),
            // @PLAN53 cluster 2: remap the var-numbers these IR nodes carry through
            // `var_mapping` — exactly like `Var`/`TupleGet`/`TuplePut` above.  They
            // were missing, so when a sibling scope reuses a name (`copy_variable`),
            // a fn-ref loop break-test (`FnRefDnr`) or a copied closure capture
            // (`FnRef.clos_var`) kept pointing at the ORIGINAL var.  V1 masked it (the
            // original + copy share a slot); V2 gives them distinct slots, so the
            // stale read hit the wrong slot (repro_p352: 2nd reused-name fn-ref loop
            // read loop-1's exhausted sentinel → 0).  `clos_var == u16::MAX`
            // (non-capturing) is left untouched — `var_mapping` never holds u16::MAX.
            Value::FnRefDnr(var) => Value::FnRefDnr(*self.var_mapping.get(var).unwrap_or(var)),
            Value::FnRef(d_nr, clos_var, fn_type) => Value::FnRef(
                *d_nr,
                *self.var_mapping.get(clos_var).unwrap_or(clos_var),
                fn_type.clone(),
            ),
            Value::Yield(inner) => Value::Yield(Box::new(self.scan(inner, function, data))),
            Value::Span(b) => {
                let scanned = self.scan(&b.1, function, data);
                // When scanning lifted an inline struct-returning-call argument
                // (@P297), the result is `Insert([Set(__lift_N, …), final])` — a
                // statement sequence, not a positioned expression.  Re-wrapping
                // it in a Span hides the lift preamble from the consumers that
                // hoist it to statement level (`scan_set`'s flatten and
                // `scan_args`'s `is_p135_hoisted` bubbling, both `if let
                // Value::Insert`).  The interpreter tolerates the hidden Insert;
                // the native backend would emit `Set(__lift_N, …)` inside an
                // enclosing expression and fail to compile.  Inner ops keep
                // their own positions, so dropping the outer span is safe.
                //
                // SURGICAL: only unwrap when the Insert's leading op is a lift
                // `Set(__lift_N, …)`.  Other span-wrapped Inserts (closure-record
                // construction, etc.) MUST keep their span — unwrapping them
                // broadly breaks closure-in-struct-field construction (`invalid
                // fn-ref` in native codegen, @P258/@P259 territory).
                //
                // The A5.6 null-init preamble ([`Self::is_null_init_preamble`]) is the
                // second shape that must survive to `scan_args`, and it is just as
                // narrow: exactly two ops, led by an owned-heap `Set(v, Null)`.
                let is_lift_preamble = matches!(&scanned, Value::Insert(ops)
                    if ops.first().is_some_and(|op| matches!(op,
                        Value::Set(v, _) if function.name(*v).starts_with("__lift_")))
                        || Self::is_null_init_preamble(ops, function));
                if is_lift_preamble {
                    scanned
                } else {
                    Value::with_span(b.0.clone(), scanned)
                }
            }
            _ => val.clone(),
        }
    }

    /// Register `v`'s scope.  Normally the current scope, but on the gated
    /// phase-2 re-scan a confined `__vdb`/local registers at its block scope
    /// (plan-57 cluster I) so the block-exit `free_vars` sweep frees its store
    /// there instead of at function exit.  `confined` is empty on phase 1, so
    /// this is identical to `var_scope.insert(v, self.scope)` in the common case.
    fn put_scope(&mut self, v: u16) {
        let scope = self.confined.get(&v).copied().unwrap_or(self.scope);
        self.var_scope.insert(v, scope);
    }

    fn scan_set(&mut self, ov: u16, value: &Value, function: &mut Function, data: &Data) -> Value {
        assert_ne!(
            ov,
            u16::MAX,
            "Incorrect variable in {} fn {}",
            function.file,
            function.name
        );
        if let Some(s) = self.var_scope.get(&ov)
            && self.scope != *s
            && !self.stack.contains(s)
        {
            if std::env::var("LOFT_LOG").as_deref() == Ok("scope_debug") {
                eprintln!(
                    "[scope_debug] copy trigger: var={ov} name='{}' \
                     registered_scope={s} current_scope={} stack={:?} value={value:?}",
                    function.name(ov),
                    self.scope,
                    self.stack,
                );
            }
            if let Some(&existing_copy) = self.var_mapping.get(&ov) {
                // Replace the mapping only if the existing copy's scope has exited.
                if let Some(&copy_scope) = self.var_scope.get(&existing_copy)
                    && copy_scope != self.scope
                    && !self.stack.contains(&copy_scope)
                {
                    self.var_mapping.insert(ov, function.copy_variable(ov));
                }
            } else {
                self.var_mapping.insert(ov, function.copy_variable(ov));
            }
        }
        let v = *self.var_mapping.get(&ov).unwrap_or(&ov);
        // A scope copy of a witnessed local (`copy_variable` above) is the same binding under
        // a new id: it keeps the witness and the never-free mark, or its own Sets would go
        // back to the static frees the witness replaced.
        if v != ov
            && let Some(&w) = self.owner_witness.get(&ov)
            && !self.owner_witness.contains_key(&v)
        {
            function.set_skip_free(v);
            function.set_owner_witness(v, w);
            self.owner_witness.insert(v, w);
        }
        // `@FR-O-Complete` / `@FR-B-Copy` — a REASSIGNMENT of a heap local from a value
        // branch is lowered to the statement form, `if c { x = a } else { x = b }`, so each
        // arm's `Set` gets the lowering a single bind of that tail has: a whole variable is
        // COPIED, a projection views, a call is copied or adopted by its own arm.  Bound as
        // one value, the branch handed the local the chosen arm's STORE: `x = if c { a }
        // else { b }` on an owned `x` aliased `a` (a write through `x` reached it) and then
        // freed it as its own at scope exit.  The FIRST bind keeps its per-arm lift
        // (`lift_join_arm_tails`), whose temps the binding borrows — a binding assigned
        // elsewhere cannot borrow them (`@FR-O-Latest`), which is exactly why the
        // reassignment is written out per arm instead.  Arms that hand back a compiler temp
        // (a `??` hoist, a literal's work-ref) keep the value form: the join they express is a
        // runtime fact (`Own::Join`), not a copy.  RECORDS only: a sunk vector `Set` keeps
        // the join deps the parser typed the binding with and still aliases (the vector twin
        // is filed, not fixed here).
        if self.var_scope.contains_key(&v)
            && Self::is_value_branch(value)
            && !matches!(function.tp(v), Type::RefVar(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && let Some(sunk) = Self::sink_set_into_arms(v, ov, value, function)
        {
            return self.scan(&sunk, function, data);
        }
        // #316 — capture BEFORE put_scope below: an ownership-transition free
        // only applies to a REassignment.
        let was_in_scope = self.var_scope.contains_key(&v);
        // The record this reassignment DISPLACES is released through its hook before the
        // new value lands — read here, while `owned_refs` still describes the previous
        // assignment.
        let displaced = if was_in_scope && *value != Value::Null {
            let rhs_owned = matches!(self.ref_rhs_ownership(value, data), RefRhs::Owned);
            self.displaced_drop(v, rhs_owned, function, data)
        } else {
            None
        };
        // An UNCONDITIONAL reassignment retires the hand-off of the record it displaces:
        // what `v` holds from here on is its own to release again.  A reassignment inside a
        // deeper scope (one arm of a branch, a loop body) is not certain to run, so the
        // hand-off stays — the leak direction, never a second release.
        if was_in_scope && *value != Value::Null && self.var_scope.get(&v) == Some(&self.scope) {
            self.drop_transferred.remove(&v);
        }
        // A redundant re-init `Set(v, Null)` for an already-in-scope var is
        // elided (Reference/Vector/Enum/Text locals don't need re-null-ing).
        // EXCEPTION (@P302): keyed collections — `s = []` lowers to
        // `Set(s, Null)`, which on a reassignment is a genuine CLEAR (codegen
        // emits an in-place `OpDatabase`).  Eliding it left the old contents
        // intact (silent no-op) and leaked `s`'s store.  Let keyed Set-Null
        // through so codegen's keyed reassign arm clears in place.
        if self.var_scope.contains_key(&v)
            && *value == Value::Null
            && !crate::parser::vectors::is_keyed(function.tp(v))
        {
            return Value::Insert(Vec::new());
        }
        // #316 — ownership-transition free.  When this var's latest scanned
        // assignment gave it an OWNED store and this reassignment installs a
        // BORROW, the merged static type already carries deps, so codegen's
        // dep-empty pre-Set free never fires and the owned store is orphaned
        // (`chosen = m_none(); chosen = pool[i] ?? m_none()` leaked one store
        // per call).  Emit the free here, in the IR, before the new value
        // lands.  Depth guard: only at the loop depth that owned the store —
        // inside a deeper loop the free would re-run on iterations 2+ and
        // release the previous iteration's VIEWED store.
        let mut transition_free: Option<Value> = None;
        // The same transition for a TUPLE: reassigning a tuple local installs new
        // elements over the old ones, and without this the previous turn's owned
        // element stores are named by nobody.  Invisible until a call-site return
        // buffer stopped being pre-allocated by the caller (loft#1085): before that
        // the callee ADOPTED the caller's `__ref_N` store, so the caller's own
        // scope-exit free covered every iteration's element and one store served
        // the whole loop.  A first-iteration free is a no-op — the entry null-init
        // leaves the elements at the sentinel, which `free` ignores.
        if was_in_scope
            && let Type::Tuple(elems) = function.tp(v)
            && !value.reads_var(v)
            && !value.reads_var(ov)
        {
            let elems = elems.clone();
            let frees = tuple_owned_elem_frees(&elems, v, data, function);
            if !frees.is_empty() {
                transition_free = Some(Value::Insert(frees));
            }
        }
        // loft#1126 / @FR-O-Latest — the ownership-transition free for the OTHER
        // reassignment shape: `v = f(…, v, …)` with `v` at `f`'s hidden
        // return-buffer position, where `f` mints a store of its own and never
        // delivers into the buffer.  The hand-off is not taken, so `v`'s current
        // store is displaced with nothing left naming it.
        //
        // Codegen cannot decide this one.  `v` is the function's hidden
        // return-buffer PARAMETER, so `state/codegen.rs`'s `is_hidden_buf_arg`
        // reads "argument → the CALLER owns that store → never free" — the
        // BINDING-level reading.  @FR-O-Latest says ownership is a property of the
        // LATEST ASSIGNMENT: the caller's store is gone the moment this function
        // assigns `v`, and what `v` holds from then on is this function's to
        // release.  `owned_refs` is that fact (@FR-O-Oracle memoised per path and
        // per loop depth), and it lives here, so the free is emitted here.
        //
        // `--native` reaches the same verdict at runtime through its entry-buffer
        // witness (`_rb_w_<name>`, `generation/mod.rs`), which is why only the
        // interpreter reported the leak — an @FR-O-NoDiverge asymmetry, with the
        // interpreter on the deviating side.  Its own guarded free is a no-op after
        // this one: the `OpFreeRef` emitter resets a freed Var to the null
        // sentinel, so the store it captures is already NULL.
        if transition_free.is_none()
            && was_in_scope
            && matches!(function.tp(v), Type::Reference(_, _) | Type::Enum(_, true, _))
            // @FR-O-Proxy asks free — the ownership-TRANSITION free, releasing the store `v`
            // is about to stop naming.
            && function.tp(v).depend().is_empty()
            // @FR-O-Proxy is unsound alone — a free taken on the empty dep list
            // must consult @FR-O-Override.
            && !function.is_skip_free(v)
            && self.owned_refs.get(&v) == Some(&self.loops.len())
            && displaces_owned_through_fresh_callee(value, v, ov, data)
        {
            transition_free = Some(call("OpFreeRef", v, data));
        }
        // A `??` hoist that OWNS its subject (a record from a call, `parser/operators.rs`)
        // is a function-scoped work-ref re-bound on every pass of a loop, and the store it
        // displaces is released HERE, in the IR, rather than left to codegen's pre-Set free:
        // the interpreter emits that free for a dep-empty owned Reference and `--native` does
        // not for a fn-ref re-bind, so without this op the displaced stores were held to
        // frame exit on one backend only (`@FR-O-NoDiverge`).  The op is a no-op on the first
        // pass (the slot holds the sentinel) and the interpreter's own pre-Set free then finds
        // the slot already reset.
        if transition_free.is_none()
            && was_in_scope
            && function.name(v).starts_with("__ncc_")
            // @FR-O-Override, consulted first: a hoist the parser marked never-free (a
            // projection subject) releases nothing here.
            && !function.is_skip_free(v)
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            // @FR-O-Proxy asks free — the ownership-TRANSITION free of the store the hoist
            // is about to stop naming.
            && function.tp(v).depend().is_empty()
        {
            transition_free = Some(call("OpFreeRef", v, data));
        }
        let mut witness_snapshot: Option<Value> = None;
        // loft#1128 — the PATH-SENSITIVE half of the same fact.  `owned_refs` above is
        // intersect-merged at every join (@FR-O-Complete), so an assignment inside ONE `if`
        // arm answers "not owned on every path" and the branch above emits nothing: sound,
        // and incomplete — the store that assignment minted is displaced here with nothing
        // naming it, one per call.  The runtime witness carries the same @FR-O-Latest fact
        // per RUN, so the free is emitted GUARDED instead of not at all.  `--native` has
        // reached this answer all along through its entry-buffer witness `_rb_w_<name>`,
        // which is why only the interpreter reported the leak.
        if transition_free.is_none()
            && was_in_scope
            && let Some((buf, flag)) = self.rbuf_witness
            && buf == v
            && matches!(
                function.tp(v),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            // @FR-O-Proxy asks free — the same transition free, emitted GUARDED on the
            // runtime witness where the static fact is sound but incomplete.
            && function.tp(v).depend().is_empty()
            && !function.is_skip_free(v)
            && displaces_owned_through_fresh_callee(value, v, ov, data)
        {
            transition_free = Some(v_if(
                Value::Var(flag),
                call("OpFreeRef", v, data),
                Value::Null,
            ));
        }
        // loft#1200 — the displaced-store free for a nullable heap-record local, GUARDED by
        // the per-local witness.  See `nullable_locals_that_displace` for why the guard is a
        // runtime flag and not a predicate: the local's first store is shared with a work-ref
        // that frees it too, and no static site separates that iteration from the rest.
        if transition_free.is_none()
            && was_in_scope
            && let Some(&flag) = self.local_owns.get(&v)
            && mints_a_store_the_target_does_not_hold(value, v, ov, data)
        {
            transition_free = Some(v_if(
                Value::Var(flag),
                call("OpFreeRef", v, data),
                Value::Null,
            ));
        }
        // A record ENUM is the second spelling of a struct-like heap store, and this
        // transition — and the `owned_refs` tracking below that licenses it — reads the
        // same @FR-O-Latest fact for both.  The two blocks above already pair the
        // spellings; these two did not, so a record-enum local that OWNED a store and is
        // then assigned a VIEW never freed what it displaced (loft#1202).
        if was_in_scope
            && matches!(
                function.tp(v),
                Type::Reference(_, d) | Type::Enum(_, true, d) if !d.is_empty()
            )
            // @FR-O-Proxy asks free — @FR-O-Override applies here as at every other free
            // site: a witnessed local (loft#1336) releases through its witness only.
            && !function.is_skip_free(v)
            && self.owned_refs.get(&v) == Some(&self.loops.len())
            && matches!(
                self.ref_rhs_ownership(value, data),
                RefRhs::View
            )
            // `value` is pre-scan IR: reads may name the original id (`ov`)
            // or the remapped one (`v`) — guard against both.  A self-
            // reading borrow (`x = x.next`, #328) keeps its owned store
            // until scope exit — a bounded, documented residual of this
            // conservatism (LIFETIME.md § Ownership-transition free).
            && !value.reads_var(v)
            && !value.reads_var(ov)
        {
            transition_free = Some(call("OpFreeRef", v, data));
        }
        // Track the LATEST assignment's ownership for this var.  Through `base()`: a nullable
        // record local holds the same record behind a nullability marker (`@FR-L-Null`), and
        // the memo is what [`Self::displaced_drop`] reads for it; the transition free above
        // keeps its own bare test and is unchanged by the wider memo.
        if matches!(
            function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) {
            match self.ref_rhs_ownership(value, data) {
                RefRhs::Owned => {
                    self.owned_refs.insert(v, self.loops.len());
                }
                RefRhs::View => {
                    self.owned_refs.remove(&v);
                }
            }
        }
        // remember the scope of the variable
        let mut depend = Vec::new();
        for d in function.tp(v).depend() {
            // Skip deps that reference variables from another function's scope
            // (e.g., closure work vars embedded in a fn-ref return type).
            if d >= function.count() {
                continue;
            }
            // loft#1135 — a LOOP VARIABLE reserves its own slot, in its own scope.
            //
            // This prefix exists for a dep whose only other mention is its FREE: a lift var
            // assigned inside a conditional arm needs its slot reserved along every path, or
            // the path that skips the arm reads an uninitialised one.  A `for` loop's
            // variable has no such gap — its header assigns it unconditionally, every
            // iteration, inside the loop.
            //
            // `e`'s type carries ONE dep list while `e` may be assigned in two disjoint
            // scopes (`Function::depend` replaces rather than accumulates), so the surviving
            // dep is whichever assignment parsed LAST.  Two `for` loops over the same keyed
            // type give the FIRST loop's `e` a dep on the SECOND loop's collection, and
            // pre-initialising it here put a `Set(c#1, Null)` — and `var_scope[c#1]` — inside
            // the first loop's body.  The second loop then read its own variable as
            // out-of-scope and copied it, leaving the original a keyed local nothing writes
            // and nothing reads: a store-backed slot, allocated by that init and freed by
            // nobody.  One orphan per program, on `--interpret`.
            //
            // The dep list being wrong is the defect above this one; not reserving a slot for
            // a loop variable is right regardless of which dep names it.
            //
            // ⚠ The predicate is `was_loop_var` and NOT "assigned somewhere in the body",
            // which is the first thing to reach for and is wrong: a lift var IS assigned, in
            // a conditional arm, which is exactly the case the prefix is for.  Measured —
            // `890-consumed-lift-double-free.loft` returned a garbage record under
            // `LOFT_POISON`.
            if !self.var_scope.contains_key(&d) && !function.was_loop_var(d) {
                depend.push(d);
                self.put_scope(d);
                self.var_order.push(d);
            }
        }
        if !self.var_scope.contains_key(&v) {
            self.put_scope(v);
            self.var_order.push(v);
        }
        // When a Reference variable is assigned from a user-function call,
        // codegen has two sub-paths (state/codegen.rs gen_set_first_at_tos /
        // gen_set_first_ref_call_copy), keyed on the SAME carried adopt-vs-copy
        // fact `Definition::return_adopts_fresh_store()` (Cluster A.3,
        // OWNERSHIP_MODEL row 102):
        // - adopts_fresh_store == false → the return is tied to a passed
        //   buffer/param (a visible arg it aliases, or a hidden ref_return
        //   work-ref the caller reuses); gen_set_first_ref_call_copy deep-copies
        //   into a FRESH store `v` owns.
        // - adopts_fresh_store == true → the return is genuinely fresh (empty
        //   dep or the `["??"]` one-buffer marker); `v` adopts the callee's
        //   store (the callee's `__ref_N` store IS the returned struct's store),
        //   OR the callee minted a different fresh store and the caller's
        //   `__ref_N` pre-alloc is orphaned.
        // This reads the precise carried adopt-vs-copy fact rather than the
        // coarse "callee has any visible ref param" proxy `has_ref_params` the
        // 11 sites used to re-derive (A.3): a callee with a ref param that
        // returns a *fresh* store (`fn mk_from(seed) -> Box { Box { v: [...] } }`)
        // now adopts instead of wastefully deep-copying, while a hidden
        // work-ref return (`fn render(p) -> Canvas { cv = …; cv }`, dep
        // `["cv"]`) still copies — the coarse proxy lumped both as "copy".
        // P198 — most operators are wrapped in Value::Span by the parser
        // for diagnostics.  Unwrap before pattern-matching so the
        // deep-copy / make_independent logic fires for Span(Call(...))
        // assignments — without this, OpFreeRef is never emitted for the
        // freshly-allocated store and Database N leaks at scope exit
        // (e.g. tests/scripts/95-alias-copy.loft Database 3 leak).
        let unspanned_value = value.unspan();
        // loft#759 — a `&` parameter is `RefVar(Reference|Enum)`, so the bare
        // `matches!` above read it as "not a record" and skipped this whole
        // block.  Peel the `&` for the record-KIND question (the loft#753 /
        // loft#740 shape: one question, one peel) and carry the `&`-ness
        // separately, because the two halves below want opposite answers:
        //
        // - the deep-copy dep-strips are LOCAL-only.  They exist because
        //   `gen_set_first_ref_call_copy` copies the callee's buffer into a
        //   fresh store `v` owns — but a set THROUGH a `&` never reaches that
        //   path (codegen writes the returned DbRef straight into the caller's
        //   slot with `SetStackRef`), and `v` is a parameter the caller owns,
        //   so stripping its deps would only make the callee free it.
        // - the witness pairing is exactly what the `&` case needs, and needs
        //   it whatever `adopts_fresh_store` says.  With no copy in between,
        //   the buffer the callee filled IS what the caller now holds, so the
        //   scope-exit `OpFreeRef(__ref_N)` freed the record the caller went
        //   on reading and writing through.
        let publishes_through_ref = matches!(function.tp(v), Type::RefVar(_));
        // Asked BARE on purpose, not through `base()`: the two strips below make `v` an
        // OWNER, which is right only where a copy is emitted, and a `-> S?` callee's
        // delivery does not yet materialise what a `-> S` one does — a capture, a parameter's
        // element, a witnessed local's view are handed up raw (loft#1337's selector copies
        // only what this frame frees).  Peeled, a nullable local bound from such a call
        // became an owner of a store it only viewed and freed it (the loft#1181 capture).  The
        // nullable spelling copies through the join guard instead (`nullable_join_first_bind`,
        // whose own strip sits below), which copies exactly the borrow it can witness.
        let mut record_target = function.tp(v);
        while let Type::RefVar(inner) = record_target {
            record_target = inner.base();
        }
        // The WITNESS PAIRING below asks *did the callee adopt the buffer I handed it, or
        // mint its own?*, and that question is the same for a vector: a `vector<T>` is
        // delivered through a `__ref_N` exactly as a record is, and its per-iteration free
        // has the same two cases to tell apart.  Without it a vector local bound from such
        // a call got a PLAIN `OpFreeRef`, which in the adoption case releases the caller's
        // own buffer — reused every iteration, so the next one wrote into a freed store
        // (loft#1201; the record spelling beside it was already correct).
        //
        // The two dep-STRIPS in this block do NOT generalise and keep the record-shaped
        // test they had: both exist for `gen_set_first_ref_call_copy`, the Reference-only
        // deep-copy path, and a vector never reaches it.
        let record_shaped = matches!(
            record_target,
            Type::Reference(_, _) | Type::Enum(_, true, _)
        );
        let vector_shaped = matches!(record_target, Type::Vector(_, _));
        // loft#1245 — BOTH spellings, because this decision and codegen's copy-or-adopt
        // one have to name the SAME set of callees (see below), and codegen now reaches a
        // `CallRef`.  While this read `Value::Call` alone the two disagreed for a fn-ref
        // bind: codegen deep-copied into a store `v` owns and the deps stayed, so
        // `get_free_vars` emitted no `OpFreeRef` and every copy leaked.
        if (record_shaped || vector_shaped)
            && matches!(unspanned_value, Value::Call(_, _) | Value::CallRef(_, _))
            && let Some(fn_nr) = crate::use_analysis::callee_of(data, self.d_nr, unspanned_value)
            // A loft-defined callee — an `n_` global OR a `t_` method / generic
            // monomorph (@PLN85 generic-tuple-return-fix.md — a generic tuple return
            // is a `t_<Type>_<fn>` monomorph; without `t_` the adopts-fresh /
            // OpFreeRefIfDistinct pairing was skipped and the caller freed the
            // aliased return with a plain OpFreeRef, orphaning its text fields).
            // This decision and codegen's copy-or-adopt one have to name the SAME set
            // of callees, which is why the predicate lives in one place (loft#810).
            && data.def(fn_nr).is_loft_defined()
        {
            let adopts_fresh_store = data.def(fn_nr).return_adopts_fresh_store();
            // @PLN85 `local_source` over-free fix (LOFT_JOIN_OWN): `v` holds an OWNED
            // store (this adopts-fresh call) that a later borrow/join reassignment
            // displaces. Strip `v`'s declared deps so it is OWNED everywhere — the
            // owned path then deep-copies the borrow into `v`'s store and frees it at
            // scope exit; without this the displaced owned store is orphaned (it was
            // bound to `v`, not to the source retbuf the cleanup guards) and leaks.
            if record_shaped
                && !publishes_through_ref
                && self.displaced_owned.contains(&ov)
                && !function.tp(v).depend().is_empty()
                // @FR-O-Proxy asks free.  @FR-O-Override vetoes it at every site that frees
                // on it, and stripping the deps IS such a site: `get_free_vars` reads the dep list, so
                // emptying it here is what makes the scope-exit sweep emit `OpFreeRef(v)`.
                // The proxy is read negated — "this still looks like a borrow" — which does
                // not change the conclusion the strip acts on, only its spelling.  A binding
                // the parser marked never-free keeps its deps and its store.
                && !function.is_skip_free(v)
            {
                let deps: Vec<u16> = function.tp(v).depend().clone();
                for d in deps {
                    function.make_independent(v, d);
                }
            }
            if record_shaped && !adopts_fresh_store && !publishes_through_ref {
                // codegen will take gen_set_first_ref_call_copy —
                // OpConvRefFromNull +
                // OpDatabase + lock-args + OpCopyRecord deep-copy into a
                // FRESH store owned by `v`.  Strip v's declared deps so
                // get_free_vars emits OpFreeRef at scope exit; otherwise
                // the parser's "borrows from arg N" inference suppresses
                // emission and the deep-copied store leaks (the
                // `dep_empty=false` path in scopes.rs:906).
                let deps: Vec<u16> = function.tp(v).depend().clone();
                for d in deps {
                    function.make_independent(v, d);
                }
            }
            // `adopts_fresh_store == true` call whose result is assigned
            // to a Reference variable `v`.  At runtime the callee either:
            //   - **adopts** the placeholder (writes into the passed
            //     `__ref_N` and returns the same DbRef) — then `v`
            //     and `__ref_N` share a store;
            //   - **allocates fresh** (e.g. `return map_empty()` or
            //     `T.parse(text)` with an internal fresh alloc) —
            //     then `v`'s store and `__ref_N`'s placeholder store
            //     are distinct, and the placeholder is orphaned.
            //
            // The compiler cannot resolve the choice statically: a
            // single callee (`map_from_json`) branches both ways on
            // `json == ""`.  Both patterns must work.
            //
            // Plain `OpFreeRef(__ref_N)` at scope exit is wrong in
            // the adoption case when `v` flows into the enclosing
            // function's return — the placeholder free happens
            // BEFORE the caller reads `v`, corrupting `v`'s shared
            // store.  Unconditionally skipping the free is wrong in
            // the fresh-store case — placeholder orphaned.
            //
            // Record `__ref_N → v` in `paired_witness`.  At scope
            // exit, `get_free_vars` emits `OpFreeRefIfDistinct(__ref_N,
            // v)` instead of `OpFreeRef(__ref_N)`: the runtime
            // store-nr comparison settles the two cases per execution
            // path (match → skip; differ → free).
            //
            // loft#759 — a set THROUGH a `&` parameter needs the same pairing
            // whatever `adopts_fresh_store` says.  For a LOCAL target the flag
            // decides whether a deep copy stands between the buffer and `v`
            // (`!adopts_fresh_store` copies, so the two are always distinct and
            // the pairing would be a no-op).  No copy stands in the `&` case,
            // so the buffer and the caller's slot alias whenever the callee
            // returned the buffer it was handed — the majority shape, and the
            // one `file()` has (`result = File{..}; result`).
            // ⚠ A VECTOR pairs whatever `adopts_fresh_store` says, and the asymmetry is
            // the whole point.  The flag means *the callee mints its own store rather than
            // filling the one I passed*, so for a RECORD its false case is safe on its own:
            // `gen_set_first_ref_call_copy` interposes a deep copy, and `v` and the buffer
            // cannot alias.  A vector has no such copy path — it is PutRef-ALIASED to the
            // work-ref argument — so there the false case is the one where they DEFINITELY
            // alias, and a plain `OpFreeRef(v)` releases the caller's own buffer.  Hoisted
            // out of a loop and reused every iteration, that buffer is then written after
            // the free (loft#1201).  `OpFreeRefIfDistinct` answers both cases at run time
            // and is conservative in the direction that matters: it frees exactly as the
            // plain free did when the stores DIFFER, and only skips when they alias.
            if (adopts_fresh_store || publishes_through_ref || vector_shaped)
                && let Value::Call(_, args) = unspanned_value
            {
                for arg in args {
                    let arg_var = match arg {
                        Value::Var(av) => Some(*av),
                        Value::Set(av, _) => Some(*av),
                        _ => None,
                    };
                    if let Some(av) = arg_var {
                        let n = function.name(av);
                        if n.starts_with("__ref_") || n.starts_with("__rref_") {
                            // `av`'s scope is inherited from the enclosing
                            // assignment: `self.scope`.  `v`'s scope was
                            // just written above.  Only pair when the
                            // witness `v` lives AT LEAST as long as
                            // `av` — i.e. `var_scope[v] <= var_scope[av]`.
                            // Otherwise, when codegen lowers the function
                            // to Rust, the witness's `let` falls out of
                            // its block scope before `av`'s OpFreeRef
                            // fires, and the emitted `var_f.store_nr`
                            // references a dead name (e.g. `f = file(…,
                            // __ref_1)` inside a nested `{}` block).
                            //
                            // loft#759 — a PARAMETER has no such block to fall
                            // out of: its `let` is the function signature, so
                            // it outlives every local including `av`, on both
                            // backends.  Its `var_scope` entry is written when
                            // the body first assigns it, which for a set inside
                            // an `if` reads as INNER-scoped and would route a
                            // valid witness into the @P378(a) branch below.
                            let av_scope = self.var_scope.get(&av).copied().unwrap_or(u16::MAX);
                            let v_scope = self.var_scope.get(&v).copied().unwrap_or(u16::MAX);
                            // A buffer is never its own witness.  The pairing exists to
                            // skip the buffer's free when ANOTHER variable adopted its
                            // store; `__ref_N = f(__ref_N)` has no other variable, and
                            // the guard then compares the store with itself and never
                            // frees at all — the buffer's own store leaks (loft#1013,
                            // where capturing the call's answer into the buffer it was
                            // handed is what gives the value an owner).
                            if av == v {
                                continue;
                            }
                            // A vector admitted here ONLY by the alias case below
                            // (`!adopts_fresh_store`, no `&`) takes the inner-slot branch
                            // and nothing else.  Making the BUFFER's own free conditional
                            // on the slot is the opposite trade and is wrong for it: the
                            // slot may have no free of its own, and then neither store is
                            // released.  Measured — widening both branches leaked across
                            // sixteen suites (loft#1201).
                            let vector_alias_only =
                                vector_shaped && !adopts_fresh_store && !publishes_through_ref;
                            if !vector_alias_only
                                && ((publishes_through_ref && function.is_argument(v))
                                    || (v_scope <= av_scope && v_scope != u16::MAX))
                            {
                                self.paired_witness.entry(av).or_insert(v);
                            } else if v_scope != u16::MAX
                                && av_scope != u16::MAX
                                && v_scope > av_scope
                            {
                                // @P378(a) — witness `v` is INNER-scoped (e.g.
                                // a loop body) while the `__ref_N` buffer `av`
                                // is OUTER (function).  The buffer is reserved
                                // once but `v` (which adopts the buffer's
                                // store) is freed every iteration; that frees
                                // the buffer's store, which `find_free_slot`
                                // then recycles to a callee temp next
                                // iteration — two OpDatabase targets collide on
                                // one record (self-referential keyed insert →
                                // SIGSEGV).  Make `v`'s per-iteration free
                                // conditional on NOT aliasing the buffer:
                                // adoption → skip (store stays reserved, freed
                                // once by the buffer's function-exit OpFreeRef);
                                // fresh-store → real free.  Scope-safe for
                                // native because `av` (outer) outlives `v`.
                                let buffers = self.witness_buffer.entry(v).or_default();
                                if !buffers.contains(&av) {
                                    buffers.push(av);
                                }
                            }
                        }
                    }
                }
            }
        }
        // @PLN85 over-free class — a VECTOR return-buffer (the hidden SRet arg,
        // NRVO'd into a source local like `best`) bound from a call that
        // DETERMINISTICALLY returns its OWN buffer (`!return_adopts_fresh_store()`,
        // e.g. `best = rows(b, __ref_1)` where `rows` returns its `__retbuf`) is
        // PutRef-ALIASED to the work-ref arg `__ref_1` — no deep copy (unlike the
        // Reference path above, which `gen_set_first_ref_call_copy` copies). A plain
        // `OpFreeRef(__ref_1)` at scope exit then whole-store-frees the RETURNED
        // buffer. Pair `__ref_1 → ov` so the free becomes
        // `OpFreeRefIfDistinct(__ref_1, ov)`: a no-op since they alias (the caller
        // owns + frees the returned buffer). Restricted to the return-buffer ARG —
        // a dead vector LOCAL (e.g. `other`) is not an argument, so it keeps its
        // plain free (its borrowed source must still be released, else it leaks).
        // Extended to Reference (struct) + struct-Enum return-buffers too: a struct
        // retbuf `ns = sim_new_gen_s(…, __ref_1)` that ADOPTS the buffer-returning
        // callee's `__ref_1` has the SAME plain-`OpFreeRef(__ref_1)` over-free (#462's
        // sim_descend driver, tp=194). The witness-pair is conservative —
        // `OpFreeRefIfDistinct(__ref_1, ov)` frees `__ref_1` exactly as the plain free
        // did when they are DISTINCT (the deep-copy case), and only skips when they
        // alias (the adopt case, the bug) — so this can only fix, never regress.
        if matches!(
            function.tp(ov),
            Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && function.is_argument(ov)
            && data
                .def(self.d_nr)
                .attributes
                .iter()
                .any(|a| a.hidden && a.name == function.name(ov))
            && let Value::Call(fn_nr, args) = unspanned_value
            && data.def(*fn_nr).name.starts_with("n_")
            && data.def(*fn_nr).code != Value::Null
            && !data.def(*fn_nr).return_adopts_fresh_store()
        {
            for arg in args {
                let av = match arg {
                    Value::Var(a) => Some(*a),
                    Value::Set(a, _) => Some(*a),
                    _ => None,
                };
                if let Some(av) = av {
                    let n = function.name(av);
                    if n.starts_with("__ref_") || n.starts_with("__rref_") {
                        self.paired_witness.entry(av).or_insert(ov);
                    }
                }
            }
        }
        // loft#1317 — an inline record literal bound to a NULLABLE local mints into a
        // `__ref_N` work-ref and then ALIASES it into the local: `c: S? = S { x: 5 }` lowers
        // to `c = { OpDatabase(__ref_1); OpSetInt(__ref_1, …); __ref_1 }`, so the two names
        // hold ONE store.  The dense twin never gets here — `OpDatabase` builds straight into
        // `c` and there is no buffer — which is why only the nullable spelling had the fault.
        //
        // A work-ref's scope-exit free is FORCED (`is_work_ref` in `get_free_vars`), so it
        // runs even where `in_ret` suppressed the local's own.  A returned nullable record was
        // therefore handed back through a store this frame had already released:
        // `fn f() -> S? { c: S? = S { x: 5 }; c }` answered `0xDEADBEEF` under `LOFT_POISON=1`
        // on both backends, and the right value on an ordinary build, which is why it stood.
        //
        // Pairing the buffer with the local turns that free into
        // `OpFreeRefIfDistinct(__ref_1, c)`, and the run-time comparison answers all four
        // combinations — the same trade the call-shaped pairings above take:
        //
        //   returned, local still names the store  -> alias  -> decline; the caller owns it
        //   returned, local reassigned since       -> differ -> free; the literal store is dead
        //   not returned, still named              -> alias  -> decline; `OpFreeRef(c)` above
        //                                                       it already released the store
        //   not returned, reassigned since         -> differ -> free
        //
        // So it frees exactly where the plain free did whenever the stores differ, and only
        // declines where the plain free was releasing a store someone else still owns.
        //
        // The SAME two names, the other way round, where the local is INNER-scoped — a loop
        // body — and the buffer is the function's: that is @P378(a)'s shape, and it takes
        // @P378(a)'s answer (`witness_buffer`).  The local dies once per iteration, and a plain
        // free there releases the buffer's store while the buffer keeps naming it; the next
        // pass re-mints through `OpDatabase`, which REUSES the slot's store in place — a number
        // the free handed back and another record has since taken.  `for … { o = O { opt: S {
        // n: 6 } }; y: S? = S { n: 3 }; }` wrote the second iteration's literal over `o`'s
        // record on both backends, and nothing reported it: the store was live, just not the
        // buffer's.  With the pairing, the local's free is `OpFreeRefIfDistinct(y, buffer)`:
        // declined while they alias (the buffer keeps its store, reuses it in place, frees it
        // once at exit), a real free where the local moved on.  Every arm of a value branch
        // minted a buffer of its own and the local adopted whichever ran, so the pairing
        // carries them all and the free declines against each.
        if matches!(
            function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) {
            let mut adopted: Vec<u16> = Vec::new();
            adopted_work_refs(unspanned_value, function, data, &mut adopted);
            let v_scope = self.var_scope.get(&v).copied().unwrap_or(u16::MAX);
            for av in adopted {
                if av == v || v_scope == u16::MAX {
                    continue;
                }
                // The witness must outlive the buffer, or native's `let` for it has fallen
                // out of scope by the time the buffer's free runs — the same condition the
                // call-shaped pairing above states at length.
                let av_scope = self.var_scope.get(&av).copied().unwrap_or(u16::MAX);
                if v_scope <= av_scope {
                    self.literal_buffer.entry(av).or_insert(v);
                } else if av_scope != u16::MAX {
                    let buffers = self.witness_buffer.entry(v).or_default();
                    if !buffers.contains(&av) {
                        buffers.push(av);
                    }
                }
            }
        }
        // @PLN130 F2 — an element view that is live across a RESHAPE of its container cannot
        // stay an alias.  `remove` renumbers the positions in the container's store and the
        // view is a `DbRef` pinned to one, so it silently starts naming a different element:
        // measured, a pure READ answered `44/444` where its element held `33/333`, and a
        // write tore a live record (`99/444` — n from the stray write, tag from the real
        // element).  No detector sees it: nothing is freed and the pointer is live.
        //
        // Strip the container dep so the binding materialises into a store it owns (the same
        // F1 arm in `state/codegen.rs` picks it up, and native's generator already
        // materialises off empty deps).  The alias is LOST for this binding, so say so —
        // constraint 2, where rustc errors on a use-after-move loft copies and warns.
        //
        // Only fires for a view that is still USED after the reshape. A view whose last use
        // precedes it is not at risk, and materialising it would lose a write that lands
        // today — see `collect_views_to_materialise`.
        let mut collection_copy: Option<(u16, i32)> = None;
        if matches!(
            function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
        ) && let Some(cause) = self.views_to_materialise.get(&v).copied()
            && let Some(container) = base_container_var(unspanned_value, data)
            // Every dep this binding carries names the container being disturbed — a VIEW
            // test, not an ownership one, and deliberately not the empty-deps proxy: what
            // makes the binding a view here is the walk's answer plus `base_container_var`,
            // and a dep would only be restating it.  A payload binding written
            // `if sh is Holder { inner }` carries NO dep where its `match` twin does, and one
            // with nothing to strip still has a mark to lift and emitters to steer, so both
            // must pass.  A binding whose deps name something ELSE is declined: that one is
            // not a view of what was disturbed.
            && function.tp(v).depend().iter().all(|d| *d == container)
        {
            let vname = function.name(v).to_string();
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
            // The binding has stopped being a view, so the NEVER-FREE mark it was given as
            // one goes with the deps (@FR-O-Override marks a borrow; this is no longer one).
            // A `match`/`is` payload binding (`_mv_<field>_N`) is marked by the parser, and
            // stripping only the deps left it owning a store nothing released — one leaked
            // record per call on `--native`, measured.  Two facts, one statement: an owner
            // frees what it owns.
            function.clear_skip_free(v);
            // ...and the VECTOR arm below gives it back, for a different fact: there the
            // local names a buffer that owns the store, so the clear must precede the set or
            // the local frees the buffer's store at scope exit.
            // A COLLECTION view needs the copy EMITTED, where a record view only needs its
            // dep stripped.  The record bind reads the deps at emit time and copies; the
            // collection bind decides copy-vs-view at PARSE time (`classify_vec_bind`'s
            // `depend().is_empty()`, the `(B-View-Base)` citation), which runs before this
            // pass, so a strip here arrives too late to be heard.  The buffer + refill is the
            // shape a whole-vector copy already takes (`ArmBind::CopyVector`): a `__lift_N`
            // that owns its store for the function's life and is refilled in place, so a
            // materialise inside a loop costs one store rather than one per iteration.
            if let Type::Vector(inner, _) = function.tp(v).base().clone() {
                let wrapper = format!("main_vector<{}>", inner.name(data));
                if data.name_type(&wrapper, data.def(self.d_nr).source) != u16::MAX
                    && let Some(elem) = data.vector_element_type(&inner, self.database)
                {
                    let tp = Type::Vector(inner.clone(), Deps::none());
                    let tmp = self.new_buffer_var(function, &tp);
                    // The local NAMES the buffer; the buffer owns the store and frees it once
                    // at function exit.  Left owning, the local's own scope-exit `OpFreeRef`
                    // releases the buffer's store — harmless at function scope and fatal in a
                    // LOOP, where the next iteration refills a freed store and the read comes
                    // back poisoned (`rec=3735928559`).  This is the same borrow the snapshot
                    // witness takes, for the same reason (@FR-O-Borrow: naming a store is not
                    // owning it).
                    function.set_skip_free(v);
                    collection_copy = Some((tmp, i32::from(elem)));
                }
            }
            let fname = data.def(self.d_nr).original_name();
            // The container the ADVICE names is the one that was DISTURBED, carried on the
            // walk's own answer.  `cname` above is the one the binding's deps name, which the
            // strip needs and the sentence does not: for a binding that views two containers
            // they are different, and only one of them was reassigned.
            let cname = function.name(cause.container).to_string();
            report_materialised_view(cause.cause, &vname, &cname, &fname);
        }
        // Companion to the !adopts_fresh_store (deep-copy) branch above for the
        // var-to-var deep-copy path.  When `Set(v, Var(src))` and
        // both are References to the same struct, codegen takes
        // `gen_set_first_ref_var_copy` (state/codegen.rs:1025-1033)
        // which OpConvRefFromNull + OpDatabase + OpCopyRecord
        // deep-copies src into a FRESH store owned by `v`.  This
        // path is hit by the I13 iterator protocol's hidden
        // `__iter_obj_N = c` setup (parser/collections.rs:209).
        // Strip v's declared deps so get_free_vars emits OpFreeRef.
        //
        // Through `base()` on both sides: `S?` is the same storage behind a nullability
        // marker (@FR-L-Null), and both emitters copy the nullable spelling of this bind
        // exactly as the dense one (`gen_set_first_ref_var_copy` reads `base()`).  Asked
        // bare, a nullable local reassigned from a value branch kept the join deps the parser
        // typed it with once the branch was written out per arm, and the per-arm copies
        // then read as borrows: an alias on both backends, where the dense twin copied.
        if let Value::Var(src) = unspanned_value
            && let Type::Reference(d_nr, _) | Type::Enum(d_nr, true, _) =
                function.tp(v).base().clone()
            && let Type::Reference(src_d, _) | Type::Enum(src_d, true, _) =
                function.tp(*src).base()
            && d_nr == *src_d
            // Not a CAPTURED or never-free local (@FR-L-CapHeap): the peel above widened this
            // strip to the nullable spelling, and a captured heap value is SHARED — the
            // closure holds the store at capture time — so making it an owner frees a store
            // the closure still reads (`x: S? = a; f = fn(){ x }; x = x.next` then `f()` read
            // null).  The dense spelling never reached this because a captured local is bound
            // by literal, not var-copy; the nullable one did.
            && !function.is_captured(v)
            && !function.is_skip_free(v)
        {
            // @PLN130 F1 — this strip is LOAD-BEARING FOR NATIVE, which is why the obvious
            // narrowing does not work.  Skipping it when both sides are borrows fixes the
            // interpreter (probe 30 goes green) and BREAKS native (`len 0 want 3`, the same
            // destruction the other way round): the emptied deps are exactly what makes the
            // native generator materialise `_own_store_k` at the bind, so leaving them makes
            // native alias the container instead.  Measured, then reverted.
            //
            // The real fix therefore cannot be "stop promoting the borrow" — it has to
            // materialise at the BIND for both backends, which is what native already does
            // as a side effect of this strip. See @PLN130 § F1 + F2 design.
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
        }
        // loft#1320 — a value joined from BRANCH ARMS whose tails are fn-ref `??` calls.  The
        // joined binding carries every arm's dep and so reads as a borrow, which is right for
        // the arm that hands back a caller's store and leaves the arm that MINTED with no
        // owner.  Give each such arm its own owner: rewrite the tail call into the BOUND
        // spelling on a temp declared in THIS statement's scope, so the branch borrows from
        // the temp and the temp frees by store identity against its one base (or, for a
        // record, owns unconditionally through `OpBindOrCopy`).  `(O-Complete)` asks for the
        // fact per binding, per path; this gives each path a binding.
        let rewritten_arms;
        let value: &Value = if Self::is_value_branch(value)
            && self.arm_tails_need_binding(value, v, data, function)
        {
            let mut rw = value.clone();
            // The temps live where the BINDING lives: a binding declared outside a loop and
            // re-Set inside it still names the arm's store after the loop, so a temp scoped
            // to the statement would be freed under it.
            let home = self.var_scope.get(&v).copied().unwrap_or(self.scope);
            // `(H-Materialise)` promises the author is TOLD when a view is copied out of its
            // container, and for a branch- or discharge-valued right-hand side the copy is
            // made here rather than by the deps strip above — so the report is owed here too.
            // Keyed on the lift actually TAKEN under the walk's gate, never on the walk's
            // answer alone: a sentence that asserts "writes through `c` no longer reach `v`"
            // over a binding that still aliases is worse than the silence it replaces, which
            // is the measured reason loft#1401's second cure was backed out.
            if self.lift_join_arm_tails(&mut rw, home, v, function, data)
                && let Some(cause) = self.views_to_materialise.get(&v).copied()
            {
                report_materialised_view(
                    cause.cause,
                    function.name(v),
                    function.name(cause.container),
                    &data.def(self.d_nr).original_name(),
                );
            }
            rewritten_arms = rw;
            &rewritten_arms
        } else {
            value
        };
        let scanned = self.scan(value, function, data);
        // Flatten: if the scanned value is Insert([preamble..., final_call]),
        // hoist the preamble out so the IR becomes
        // Insert([preamble..., Set(v, final_call)]) instead of
        // Set(v, Insert([preamble..., final_call])).
        // This keeps Set(v, Call(...)) as a bare Call, which codegen's
        // gen_set_first_at_tos can handle correctly.
        let (mut ls, mut set_value) = if let Value::Insert(mut ops) = scanned {
            if ops.len() >= 2 {
                let final_val = ops.pop().unwrap();
                (ops, final_val)
            } else {
                (Vec::new(), Value::Insert(ops))
            }
        } else {
            (Vec::new(), scanned)
        };
        // loft#1106 — a NULLABLE heap local first-bound from a call whose return may
        // borrow an argument.  `S?` is `Optional(Reference(S))`, and the shape questions the
        // heap first-bind dispatch asks are asked against the BARE type, so the nullable
        // spelling of the same storage never reached the deps strip: the local kept the
        // argument's dep, read as a permanent borrow, and nothing freed the store the callee
        // minted on its other arm.  Both backends bind it through the runtime join guard,
        // which leaves the local owning a store either way — so the deps have to go, or the
        // free that guard exists to make correct is never emitted.
        //
        // The collection materialise decided above, emitted here because it wraps the SCANNED
        // right-hand side: `OpReplaceVector(buffer, <the view>, elem)` clears the buffer and
        // refills it from what the view names, and the local then binds the buffer.  Writes
        // through the local land in the copy, which is what `(B-View)`'s materialise means and
        // what the advice already told the author.
        if let Some((tmp, elem)) = collection_copy {
            let view = std::mem::replace(&mut set_value, Value::Null);
            set_value = Value::Insert(vec![
                Value::Call(
                    data.def_nr("OpReplaceVector"),
                    vec![Value::Var(tmp), view, Value::Int(elem)],
                ),
                Value::Var(tmp),
            ]);
        }
        // Asked against `set_value`, the SCANNED right-hand side, not the raw one: `scan`
        // has just LIFTED any argument the @P290 bracket could not name into a temp, and the
        // witness the join resolves is that temp.  Read before the lift the same call answers
        // "no nameable witness" and the strip declines, while codegen — which only ever sees
        // the scanned form — emits the guard anyway.  Then the local owns a store with no
        // free: one leaked record per call on the minting arm, from the two readers of ONE
        // predicate disagreeing about which value they were reading.
        //
        // loft#1248 — and the same sentence for a bind from a CLOSURE call, which reaches
        // neither this strip's sibling above nor the `Value::Call` strip earlier in this
        // function: both are keyed on the call spelling that names its definition, and a
        // `CallRef` names a runtime value.  So a closure whose return may be its argument or
        // may be a store it minted kept the argument's dep, read as a permanent borrow, and
        // the minted arm's store was owned by nobody — one per call, to FRAME exit, which a
        // loop turns into the 65535-store ceiling.
        if crate::use_analysis::nullable_join_first_bind(
            data,
            self.d_nr,
            function.tp(v),
            &set_value,
        )
        .is_some()
            || crate::use_analysis::callref_join_first_bind(
                data,
                self.d_nr,
                function.tp(v),
                &set_value,
            )
            .is_some()
        {
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
        } else if let Some(base) = crate::use_analysis::callref_collection_join_base(
            data,
            self.d_nr,
            function.tp(v),
            &set_value,
        ) && !function.is_skip_free(v)
            && !function.is_argument(v)
        {
            // loft#1257 / loft#1320 — the COLLECTION twin of the strip above, and it goes
            // the other way: the dep STAYS, because it names the witness.  A collection has
            // no `OpBindOrCopy`, so the local may hold the caller's store or one the closure
            // minted, and only the store number can say which.  `get_free_vars` frees it by
            // identity at scope exit; a RE-Set of a named local releases the store it is
            // about to stop naming the same way, before the new value is computed.  A lift
            // temp gets no transition free: its only Set runs once per scope and the scope's
            // own exit already freed the slot.
            //
            // The witness is the store the base named AT THE BIND.  Where the base is
            // assigned once and this local has one base, the base variable itself still
            // names that store at every later free, so it is the witness.  Where it does not
            // — the base is reassigned in this function, or the local is bound at two sites
            // from two different bases — the witness is a SNAPSHOT of the base taken beside
            // the bind (`@FR-O-Latest`: the fact belongs to the assignment, and here it is
            // carried by a slot the way @PLN87's entry stash carries a rebindable
            // parameter's).  Comparing against the LIVE base instead freed a caller's store
            // on the other site's arm (sum 4034 for 12500), and against a base already
            // re-pointed it could free whatever store reused the slot; comparing against the
            // snapshot, two stale numbers still agree and decline.
            let stable = !self.multi_assigned.contains(&base)
                && self.callref_join_bases.get(&v).is_none_or(|b| b.len() <= 1);
            let w = if stable {
                function.rebind_orig(base).unwrap_or(base)
            } else {
                let wit = self.snapshot_witness_for(v, base, function);
                witness_snapshot = Some(v_set(wit, Value::Var(base)));
                wit
            };
            self.lift_join_witness.insert(v, w);
            // A re-Set releases the store it displaces, guarded the same way — where the slot
            // is LIVE.  A named local's earlier Set in the same scope chain left it live; a
            // lift temp's slot is live only if the temp was created outside the innermost
            // loop that re-runs this Set, since a temp inside it is freed at that loop body's
            // exit and would be freed twice.
            let slot_live = match self.lift_decl_depth.get(&v) {
                Some(&depth) => depth < self.loops.len(),
                None => true,
            };
            if was_in_scope && transition_free.is_none() && slot_live {
                transition_free = Some(Value::Call(
                    data.def_nr("OpFreeRefIfDistinct"),
                    vec![Value::Var(v), Value::Var(w)],
                ));
            }
        } else if self.callref_delivers_collection(function.tp(v), &set_value, data)
            // @FR-O-Proxy asks free — read negated, *"this still looks like a borrow"*:
            // stripping the deps is what makes `get_free_vars` emit the free, so the site is
            // a free site and consults @FR-O-Override like every other.
            && !function.tp(v).depend().is_empty()
            && !function.is_skip_free(v)
            // loft#1333 — and NOT for a MIXED binding, one another path assigns a borrow.
            // The deps this would strip are that other path's, not this delivery's: the two
            // arms share one binding, so declaring it the owner of this buffer also declares
            // it the owner of the store the borrow arm merely views, and the displacement
            // free then released it.  @FR-O-Complete says the fact is per BINDING and per
            // PATH; where one static site cannot be both, the rule names the direction —
            // a retained buffer is recoverable, a premature free is not.
            && !function.has_borrow_arm(v)
        {
            // A collection a closure hands back is DELIVERED — copied by the callee into the
            // buffer minted for that call — unless it is a raw view (`returns_borrowed_view`)
            // or a `Join`, which the two arms above own.  The type inherited from the callee
            // still names the ARGUMENT the tail borrowed before the delivery copied it, so the
            // local read as a borrow and the buffer was freed by nobody: `t = h(bag)` with
            // `h = fn(q: Bag) -> vector<integer> { q.items }` held one store per call.  The
            // binding owns the buffer; say so, and `get_free_vars` frees it (`@FR-O-Move`).
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
        }
        // Prepend dependency initializations.
        let mut prefix = Vec::new();
        // #316 — the ownership-transition free runs FIRST: before the dep
        // inits, the hoisted RHS preamble, and the Set itself, so the owned
        // store is released before any part of the new value is computed.
        if let Some(free) = transition_free {
            prefix.push(free);
        }
        // …and the witness snapshot is written AFTER it, so the free compares against the
        // store the PREVIOUS bind named and the snapshot then names this bind's base.
        if let Some(snap) = witness_snapshot {
            prefix.push(snap);
        }
        for d in depend {
            if d == v {
                continue;
            }
            if matches!(function.tp(d), Type::Text(_)) {
                prefix.push(v_set(d, Value::Text(String::new())));
            } else {
                prefix.push(v_set(d, Value::Null));
            }
            self.put_scope(d);
        }
        // loft#1128 — keep the runtime witness in step with what `v` now holds.  A call that
        // DELIVERS into the buffer (`v` at the callee's buffer position and the callee does
        // NOT adopt a fresh store) leaves `v` holding whatever it held, so the flag is left
        // alone; every other assignment sets it to the same @FR-O-Latest verdict `owned_refs`
        // records statically.
        let mut witness_update = match self.rbuf_witness {
            Some((buf, flag)) if buf == v && !delivers_into_buffer(value, v, ov, data) => {
                let owned = matches!(self.ref_rhs_ownership(value, data), RefRhs::Owned);
                Some(v_set(flag, Value::Boolean(owned)))
            }
            _ => None,
        };
        // loft#1200 — the per-local witness records SOLE ownership, which is a narrower
        // question than `owned_refs`'s.  An inline mint into a work-ref is `Owned` and still
        // not solely owned: the work-ref frees it too.  Only a MINTING CALL hands the local a
        // store nothing else names, so that is the one shape that sets the flag true.
        if let Some(&flag) = self.local_owns.get(&v) {
            let sole = mints_a_store_the_target_does_not_hold(value, v, ov, data);
            witness_update = Some(match witness_update {
                Some(prev) => Value::Insert(vec![prev, v_set(flag, Value::Boolean(sole))]),
                None => v_set(flag, Value::Boolean(sole)),
            });
        }
        // loft#1336 / @FR-O-Witness — keep the OWNER WITNESS naming exactly the store the
        // local minted and still holds.  Three shapes, read off the value being assigned:
        //
        // - the local MINTS and the value does not read it: release the witnessed store
        //   first (the store it is about to stop naming), then point the witness at what the
        //   local now holds;
        // - the local MINTS from a value that READS it (`c = mk(c.x)`, a materialised
        //   `x = x.inner`): @FR-O-Detach — the old store stays live until the value is
        //   computed, so the release comes AFTER the `Set`, and by store identity, because
        //   a callee that filled the store the local already owned displaced nothing;
        // - anything else — a view, a join, a null, a store somebody else owns: the local
        //   stops owning, so release the witnessed store where the local no longer names it.
        //   A view INTO the witnessed store (`x = x.inner` over an owned `x`) keeps it, and
        //   the witness keeps naming it, so scope exit still frees it exactly once.
        //
        // Identity, not a flag, so both backends read one fact from the IR and the two
        // sentinels — a witness that never took a store beside a local holding none — compare
        // EQUAL and release nothing.
        let mut witness_ops: Vec<Value> = Vec::new();
        // @FR-O-Latest — the HAND-OFF, and it comes FIRST.  A closure build in this value
        // makes the record the owner of the store its capture names AT THE BUILD, so from
        // here the frame owes nothing for it and the witness gives it up — a clear, never a
        // free: `free_named`'s cascade owns it now.  Ahead of the guarded release below,
        // because for an INLINE closure argument the capture and the target are the SAME
        // local (`s = build(|i| { s.a + i })`): the record adopts the old store and the local
        // moves to a new one in one statement, so a release asked before the clear would free
        // the store the record has just taken.  That is `(O-Detach)` in ownership clothing,
        // and it is what made dropping the veto answer `a=1` where `a=2` is right.
        let built_here = captures_built_in_value(value, data);
        // ...and the record this build is about to OVERWRITE gives up what it already holds.
        // A build inside a loop reaches the same work-ref every pass and `OpDatabase` records
        // into the store it already names, so the capture slot is rewritten and the store it
        // held is orphaned — a 5-pass loop kept four (loft#1388).  `free_named`'s cascade is
        // the adopted store's releaser, so freeing the record here is what hands it back; on
        // the first pass the work-ref is `Null` and this is a no-op.
        //
        // Gated on EVERY capture in the build having a witness, and that gate is the whole
        // soundness argument: a capture without one is still owned by the FRAME, so the
        // cascade and the frame would free one store between them.  Measured exactly there —
        // a VECTOR capture can hold no witness (`heap_def_nr` is `None` for a vector, so the
        // record-typed witness variable cannot be made), and with the release ungated its
        // loop answered `1,2` where `4,5` is right, on `--native` alone.  `(O-Derived)`: one
        // decision, one home.
        if !built_here.is_empty()
            && built_here
                .iter()
                .all(|(_, c)| self.owner_witness.contains_key(c))
        {
            for (rec, _) in &built_here {
                prefix.push(call("OpFreeRef", *rec, data));
            }
        }
        for (_, c) in &built_here {
            if let Some(&cw) = self.owner_witness.get(c) {
                prefix.push(v_set(
                    cw,
                    Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                ));
            }
        }
        if let Some(&w) = self.owner_witness.get(&v) {
            let kind = {
                let d_nr = self.d_nr;
                let defs = self
                    .fn_defs
                    .get_or_insert_with(|| crate::use_analysis::function_defs(data, d_nr));
                witness_set_kind(value, v, ov, function, data, d_nr, &mut |val| {
                    crate::use_analysis::ownership_of_with(data, d_nr, val, defs)
                })
            };
            let guarded_release = v_if(
                Value::Call(
                    data.def_nr("OpDistinctStore"),
                    vec![Value::Var(w), Value::Var(v)],
                ),
                release_witness(w, data),
                Value::Null,
            );
            match kind {
                WitnessSet::Mint => {
                    prefix.insert(0, release_witness(w, data));
                    witness_ops.push(witness_points_at(w, v, data));
                }
                WitnessSet::MintReading => {
                    witness_ops.push(guarded_release);
                    witness_ops.push(witness_points_at(w, v, data));
                }
                WitnessSet::Other => witness_ops.push(guarded_release),
            }
        }
        if prefix.is_empty()
            && ls.is_empty()
            && witness_update.is_none()
            && witness_ops.is_empty()
            && displaced.is_none()
        {
            Value::Set(v, Box::new(set_value))
        } else {
            // The snapshot of the displaced record is taken FIRST — before the transition
            // free releases its store and before any part of the new value is computed —
            // and its hook runs LAST, after the new value has landed, so a right-hand side
            // that reads the old value (`s = grow(s)`) still finds the resource live.
            let (mut all, post) = match displaced {
                Some((pre, post)) => (pre, post),
                None => (Vec::new(), Vec::new()),
            };
            all.append(&mut prefix);
            all.append(&mut ls);
            all.push(Value::Set(v, Box::new(set_value)));
            all.extend(witness_update);
            all.append(&mut witness_ops);
            all.extend(post);
            Value::Insert(all)
        }
    }

    /// [`Self::displaced_drop`] for a statement-level `OpDatabase(v, tp)` that REBUILDS a
    /// local's record in place.  A construction of a local not yet in scope displaces
    /// nothing; one of a local already in scope is asked the owner question — a first
    /// build after the declaration's null placeholder owns nothing yet, and the snapshot
    /// is null-safe, so a first loop iteration releases nothing either.
    fn in_place_rebuild(
        &mut self,
        stmt: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Vec<Value>)> {
        let Value::Call(d, args) = stmt.unspan() else {
            return None;
        };
        if *d != data.def_nr("OpDatabase") {
            return None;
        }
        let Some(Value::Var(ov)) = args.first().map(Value::unspan) else {
            return None;
        };
        let v = *self.var_mapping.get(ov).unwrap_or(ov);
        if !self.var_scope.contains_key(&v) {
            return None;
        }
        // A construction OWNS what it builds, on every iteration.
        let ops = self.displaced_drop(v, true, function, data);
        if self.var_scope.get(&v) == Some(&self.scope) {
            self.drop_transferred.remove(&v);
        }
        ops
    }

    /// The IR that releases, through its type's hook, the record `v` is about to stop
    /// holding — `(before, after)` the displacing statement — or `None` where `v` owns no
    /// droppable record.  `@FR-H-Drop`: the reassignment clause.
    ///
    /// `OpDrop` runs *"when the value's OWNER dies"* (INTERFACES.md), and a reassignment is
    /// that death for the record it displaces: its store is freed (a displaced free) or
    /// rebuilt in place, and either way the resource it held is gone with no hook run.  A
    /// file opened into a local that is later reassigned was never closed (loft#1362).
    ///
    /// The release is taken on a SNAPSHOT: a fresh temp is deep-copied from `v` before the
    /// statement (null-safe — nothing is copied from an absent local), and the hook runs on
    /// the temp after it.  The copy is what makes the order free of hazards: an in-place
    /// rebuild (`s = S {…}` lowers to `OpDatabase(s, tp)` on the existing store) overwrites
    /// the old bytes before anything after the statement could read them, and a right-hand
    /// side that reads `v` must still see the resource live.  It is a copy of a record with
    /// a droppable member — a handle, not data — so its cost is where drops are.
    ///
    /// The owner predicate is the transition free's: `v` is in scope, its record is OWNED
    /// (the dep-empty proxy, `@FR-O-Override`'s never-free, the oracle's latest-assignment
    /// fact), its drop was not handed off (`drop_transferred`), it is no witnessed
    /// mixed-ownership local, no argument and no capture.  A view's record is somebody
    /// else's resource and is never released here — which is why, inside a LOOP, the
    /// latest-assignment fact from outside the loop is trusted only when THIS assignment
    /// owns too (`rhs_owned`): on the second iteration the displaced record is the one this
    /// statement built, and a view assigned here would otherwise be copied and released as
    /// if it were owned.
    fn displaced_drop(
        &mut self,
        v: u16,
        rhs_owned: bool,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Vec<Value>)> {
        let owned_here = match self.owned_refs.get(&v) {
            Some(depth) => *depth == self.loops.len() || rhs_owned,
            None => false,
        };
        // @FR-O-Proxy asks free — the hook is a release, and it follows only where the
        // empty dep list says `v` OWNS the record; @FR-O-Override (`is_skip_free`) is
        // consulted right after it, as every free on the proxy must.
        if !owned_here
            || function.is_argument(v)
            || function.is_captured(v)
            || !function.tp(v).depend().is_empty()
            || function.is_skip_free(v)
            || self.drop_transferred.contains(&v)
            || self.owner_witness.contains_key(&v)
        {
            return None;
        }
        let d = function.tp(v).base().heap_def_nr()?;
        if data.drop_cascade_nr(d) == u32::MAX {
            return None;
        }
        let kt = data.def(d).known_type();
        let tp = function.tp(v).base().without_deps();
        self.lift_counter += 1;
        let name = format!("__disp_{}", self.lift_counter);
        let disp = function.add_temp_var(&name, &tp);
        function.mark_inline_ref(disp);
        self.var_scope.insert(disp, self.scope);
        self.var_order.push(disp);
        let live = Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(v)]);
        let snapshot = Value::Insert(vec![
            Value::Call(
                data.def_nr("OpDatabase"),
                vec![Value::Var(disp), Value::Int(i32::from(kt))],
            ),
            Value::Call(
                data.def_nr("OpCopyRecord"),
                vec![Value::Var(v), Value::Var(disp), Value::Int(i32::from(kt))],
            ),
        ]);
        let pre = vec![v_set(disp, Value::Null), v_if(live, snapshot, Value::Null)];
        let mut post = Vec::new();
        if let Some(hook) = drop_hook(function, disp, data) {
            post.push(hook);
        }
        post.push(call("OpFreeRef", disp, data));
        // Back to the TRUE sentinel: the sweep visits the temp again at scope end, and a
        // freed reference that still reads `rec != 0` would run the hook a second time on
        // whatever the allocator has since put in that slot.  On the sentinel both the
        // hook's liveness test and the sweep's free are no-ops.
        post.push(v_set(
            disp,
            Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new()),
        ));
        Some((pre, post))
    }

    /// #316 — classify the (pre-scan) RHS of a `Set` into Reference var `v`.
    /// Only two shapes are provably OWNED: a user-fn call whose declared
    /// return carries no visible-attribute dep (the callee materialises /
    /// owns its result), and a same-struct `Var` copy (codegen deep-copies
    /// both first assignment and reassignment).  A `Block` whose result type
    /// carries deps is a view — unless a dep names `v` itself (the new value
    /// might point into the store about to be freed).
    /// The free-side owned-vs-view verdict for a Reference reassignment RHS,
    /// read from the CANONICAL `ownership_of` oracle (@PLN90 D-own-1 — the last
    /// per-site ownership re-derivation, folded onto the one fact the delivery
    /// side already reads).  Owned → track the var as owned.  Borrowed AND Join →
    /// View: a reassignment whose new value is a borrow OR a runtime join
    /// DISPLACES the var's prior owned store (freed by the #316 transition-free),
    /// and the var must NOT be tracked as owned afterward (a join might be a
    /// borrow — tracking it owned would over-free the NEXT transition).  So Join
    /// folds to View, not "don't-track" — the p462 conditional
    /// `chosen = t[i] ?? m_none()` reassign needs the prior-store free.
    /// loft#854 — the memoised form. `ownership_of` recomputes the whole-function
    /// summary per question; `scan_set` asks once per assignment, which made a
    /// function with n assignments cost n whole-function walks.
    fn ref_rhs_ownership(&mut self, value: &Value, data: &Data) -> RefRhs {
        let d_nr = self.d_nr;
        let defs = self
            .fn_defs
            .get_or_insert_with(|| crate::use_analysis::function_defs(data, d_nr));
        match crate::use_analysis::ownership_of_with(data, d_nr, value, defs) {
            crate::use_analysis::Own::Owned => RefRhs::Owned,
            crate::use_analysis::Own::Borrowed { .. } | crate::use_analysis::Own::Join { .. } => {
                RefRhs::View
            }
        }
    }

    fn scan_if(
        &mut self,
        test: &Value,
        t_val: &Value,
        f_val: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Value {
        // Find Reference/Vector/Text variables first assigned inside either branch
        // (including nested ifs, but not inside loops).
        let mut pre_inits: Vec<u16> = Vec::new();
        self.find_first_ref_vars(t_val, function, &mut pre_inits);
        self.find_first_ref_vars(f_val, function, &mut pre_inits);

        // Also find small variables assigned in BOTH branches (or an else-if chain).
        let mut small_both: Vec<u16> = Vec::new();
        let mut t_vars: Vec<u16> = Vec::new();
        let mut f_vars: Vec<u16> = Vec::new();
        Self::find_assigned_vars(t_val, &self.var_mapping, &mut t_vars);
        Self::find_assigned_vars(f_val, &self.var_mapping, &mut f_vars);
        for &v in &t_vars {
            if f_vars.contains(&v)
                && !self.var_scope.contains_key(&v)
                && !pre_inits.contains(&v)
                && !needs_pre_init(function.tp(v))
            {
                small_both.push(v);
            }
        }

        // Register pre-inited vars in var_scope BEFORE scanning branches so that
        // the branch scans see them as already assigned and use the set_var/OpPutRef
        // re-assignment path instead of claim().
        for &v in &pre_inits {
            self.put_scope(v);
            self.var_order.push(v);
        }
        // Register small variables assigned in both branches at the parent scope too.
        for &v in &small_both {
            self.put_scope(v);
            self.var_order.push(v);
        }

        let scanned_test = self.scan(test, function, data);
        // #316 — ownership state is path-sensitive: scan each branch from the
        // same pre-If state, then keep only entries BOTH branches agree on.
        // Enforces @FR-O-Complete — the fact is per BINDING and per PATH, a
        // set-and-reconcile rather than one structural walk.  Both arms are scanned from
        // the SAME pre-`If` state and only what they AGREE on survives, so a store owned
        // on one path only is not treated as owned after the join.
        //
        // ⚠ @FR-O-Complete is the load-bearing invariant of the whole model: loft has no
        // user-facing borrow checker, so an incomplete fact is not a compile error someone
        // fixes — it is a miscompile or a leak.  Erring toward "not owned" here is why the
        // reconcile intersects rather than unions.
        let owned_before = self.owned_refs.clone();
        let scanned_true = self.scan(t_val, function, data);
        let owned_after_true = std::mem::replace(&mut self.owned_refs, owned_before);
        let scanned_false = self.scan(f_val, function, data);
        self.owned_refs
            .retain(|k, depth| owned_after_true.get(k) == Some(depth));
        let scanned_if = Value::If(
            Box::new(scanned_test),
            Box::new(scanned_true),
            Box::new(scanned_false),
        );

        if pre_inits.is_empty() {
            return scanned_if;
        }

        // Emit Set(v, Null/empty) for each variable at the current scope, before the
        // If node.  These are NOT passed through scan() again — the var_scope check
        // in the Set arm would strip them (contains_key + Null → Insert([])).
        let mut stmts: Vec<Value> = Vec::new();
        for &v in &pre_inits {
            if matches!(function.tp(v), Type::Text(_)) {
                stmts.push(v_set(v, Value::Text(String::new())));
            } else {
                stmts.push(v_set(v, Value::Null));
            }
        }
        stmts.push(scanned_if);
        Value::Insert(stmts)
    }

    /// Collect the variables an `if` branch ASSIGNS, so the caller can emit their
    /// null-init before the `If` — a variable written in one arm and read after it must
    /// hold null rather than an uninitialised slot.
    ///
    /// ⚠ `unspan()` first — the requirement `Value::unspan`'s own doc states for every site
    /// that pattern-matches a specific variant.  Without it a `Span` wrapping a `Set` falls
    /// to the catch-all, the assignment is not collected, and the variable loses its init.
    ///
    /// That path is REACHED: instrumented over 200 corpus programs, the catch-all dropped
    /// 2 Span-wrapped `Set`s and 8 whole Span-wrapped `Block`s.  No program's IR changes
    /// when the peel is added, so nothing observable was riding on it — the vars in
    /// question were already covered another way.  The peel stays because the reachability
    /// is what makes it a trap: Span placement has moved before, and the failure mode is a
    /// missing initialisation with nothing to report it.
    fn find_assigned_vars(val: &Value, mapping: &HashMap<u16, u16>, result: &mut Vec<u16>) {
        match val.unspan() {
            Value::Set(v, inner) => {
                let resolved = *mapping.get(v).unwrap_or(v);
                if !result.contains(&resolved) {
                    result.push(resolved);
                }
                Self::find_assigned_vars(inner, mapping, result);
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    Self::find_assigned_vars(op, mapping, result);
                }
            }
            Value::If(c, t, f) => {
                Self::find_assigned_vars(c, mapping, result);
                Self::find_assigned_vars(t, mapping, result);
                Self::find_assigned_vars(f, mapping, result);
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::find_assigned_vars(op, mapping, result);
                }
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::find_assigned_vars(a, mapping, result);
                }
            }
            Value::Drop(inner) | Value::Return(inner) => {
                Self::find_assigned_vars(inner, mapping, result);
            }
            _ => {}
        }
    }

    /// Convert the content of loops and blocks.
    /// `is_return` should be true for the function body block of a non-void
    /// function — frees must happen before the tail expression returns.
    fn convert(
        &mut self,
        bl: &Block,
        function: &mut Function,
        data: &Data,
        is_return: bool,
    ) -> Vec<Value> {
        let mut ls = Vec::new();
        for (i, v) in bl.operators.iter().enumerate() {
            // loft#1156 — a local a LOOP BODY first assigns and something AFTER the loop
            // reads.  Scoped to the body block, its store is freed at the end of every
            // iteration and the later read lands on a freed record: measured, an `A` read
            // `B`'s bytes once the slot was recycled, and `LOFT_STRICT_STORES=1` does NOT
            // catch it — the slot is legitimately free, so nothing stands between a user
            // and the wrong number.  Hoisting the SCOPE is the cure rather than moving the
            // free: pre-initialised here the local gets ONE store that each iteration
            // copies into, which is byte-for-byte the IR a hand-written `e: A = A { … }`
            // before the loop already produces.
            //
            // Registered BEFORE the loop is scanned, for the reason `scan_if` gives at its
            // own pre-init: the body's `Set` must see the variable as already assigned and
            // take the reassignment path, not `claim()`.
            let mut hoist: Vec<u16> = Vec::new();
            self.loop_locals_read_after(v, &bl.operators[i + 1..], function, &mut hoist);
            for &h in &hoist {
                self.put_scope(h);
                self.var_order.push(h);
                ls.push(if matches!(function.tp(h), Type::Text(_)) {
                    v_set(h, Value::Text(String::new()))
                } else {
                    v_set(h, Value::Null)
                });
            }
            // `s = S {…}` on a live record local lowers to `OpDatabase(s, tp)` on its
            // existing store, not to a `Set`: a REBUILD, and the record it overwrites is
            // released through its hook exactly as a reassigned one is.  The first
            // construction of a local (outside a loop) displaces nothing.
            // Re-arm the hand-offs this statement makes, in scan order, so a variable
            // whose earlier hand-off a reassignment retired is handed off again here.
            {
                let transferred = &mut self.drop_transferred;
                v.walk(&mut |n| drop_handoff_node(n, function, data, transferred));
            }
            let rebuilt = self.in_place_rebuild(v, function, data);
            let sv = self.scan(v, function, data);
            if let Some((pre, _)) = &rebuilt {
                ls.extend(pre.iter().cloned());
            }
            if let Value::Insert(to_insert) = sv {
                for i in to_insert {
                    ls.push(i.clone());
                }
            } else {
                ls.push(sv);
            }
            if let Some((_, post)) = rebuilt {
                ls.extend(post);
            }
            // loft#1331 — DETACH an accumulator this statement repointed at a destination the
            // frame does not own, so the scope-exit sweep frees nothing instead of freeing the
            // caller's collection.  @FR-O-Latest is the fact: ownership belongs to the LATEST
            // assignment, and the repoint made the accumulator name a capture.  The sentinel
            // makes that true at RUN time — the sweep still emits its free and finds nothing —
            // while the DISPLACEMENT free that releases the accumulator's own store at the
            // repoint is untouched, which is what @FR-O-Override's blanket veto could not do.
            //
            // @FR-O-Detach places it: after the statement, so it follows every read of the
            // accumulator by the value being built through it.  Skipped where the statement is
            // the block's RESULT, which `expr` below pops — a detach there would become the
            // value the block yields.
            if (bl.result == Type::Void || i + 1 < bl.operators.len())
                && let Some(acc) = repointed_literal_accumulator(v, function)
            {
                ls.push(v_set(
                    acc,
                    Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new()),
                ));
            }
        }
        let expr = if ls.is_empty() || bl.result == Type::Void {
            Value::Null
        } else {
            ls.pop().unwrap()
        };
        // @PLN85 skip_free-orphan (case a) — free each `__ncc_N` text temp that a
        // NON-TAIL statement consumes IN PLACE, right after that statement.  A
        // `skip_free` text ncc temp (`v[i] ?? ""`) is suppressed from its own
        // scope-exit free because the ncc block's result ALIASES it (freeing at
        // the ncc block would dangle the value the consumer still reads).  But a
        // text consumer (SetText / assignment / append) COPIES the String, so once
        // the consuming statement completes the temp's backing String is dead —
        // never freed on the interpreter → orphan.  The tail expression is left
        // untouched (case b: it IS the returned value, copied by the caller after
        // return, so any in-function free UAFs).  Native drops the String via RAII
        // and treats `OpFreeText` as a no-op, so the added op is interp-only.
        {
            let mut with_frees = Vec::with_capacity(ls.len());
            for stmt in ls.drain(..) {
                // An `if` whose CONDITION consumes the temp (`if (v[i] ?? "") == k { return
                // … }`) cannot take its free after the statement: an arm that returns never
                // reaches it, one orphan per early exit (loft#1357).  Evaluate the condition
                // into a boolean first, free what it consumed, then branch on the boolean.
                let (pos, inner) = match stmt {
                    Value::Span(b) => (Some(b.0.clone()), b.1.clone()),
                    other => (None, other),
                };
                // A `parallel { … }` arm runs on a WORKER over a copy of this frame: the
                // `__work_N` text a formatted argument builds there is the worker's copy,
                // which nothing frees — the frame's own scope-exit `OpFreeText` releases
                // main's (empty) copy.  Each arm frees the work texts it wrote, on the
                // worker, once its call has consumed them (loft#1357).
                if let Value::Parallel(arms) = &inner {
                    let arms: Vec<Value> = arms
                        .iter()
                        .map(|arm| {
                            let mut work: Vec<u16> = Vec::new();
                            arm.walk(&mut |v| {
                                if let Value::Var(w) = v
                                    && function.name(*w).starts_with("__work_")
                                    && matches!(function.tp(*w).base(), Type::Text(_))
                                    && !work.contains(w)
                                {
                                    work.push(*w);
                                }
                            });
                            if work.is_empty() {
                                return arm.clone();
                            }
                            let mut ops = Vec::with_capacity(work.len() + 1);
                            ops.push(arm.clone());
                            for w in work {
                                ops.push(call("OpFreeText", w, data));
                            }
                            Value::Insert(ops)
                        })
                        .collect();
                    let stmt = match pos {
                        Some(p) => Value::Span(Box::new((p, Value::Parallel(arms)))),
                        None => Value::Parallel(arms),
                    };
                    with_frees.push(stmt);
                    continue;
                }
                if let Value::If(cond, then, els) = &inner {
                    let mut cond_ncc = Vec::new();
                    collect_consumed_ncc_text(cond, function, &mut cond_ncc);
                    if !cond_ncc.is_empty() {
                        self.ret_temp_counter += 1;
                        let name = format!("__cond_{}", self.ret_temp_counter);
                        let tmp = function.add_temp_var(&name, &Type::Boolean);
                        self.var_scope.insert(tmp, self.scope);
                        self.var_order.push(tmp);
                        with_frees.push(v_set(tmp, (**cond).clone()));
                        for v in cond_ncc {
                            with_frees.push(call("OpFreeText", v, data));
                        }
                        let branch =
                            Value::If(Box::new(Value::Var(tmp)), then.clone(), els.clone());
                        with_frees.push(match pos {
                            Some(p) => Value::Span(Box::new((p, branch))),
                            None => branch,
                        });
                        continue;
                    }
                }
                let stmt = match pos {
                    Some(p) => Value::Span(Box::new((p, inner))),
                    None => inner,
                };
                let mut ncc = Vec::new();
                collect_consumed_ncc_text(&stmt, function, &mut ncc);
                with_frees.push(stmt);
                for v in ncc {
                    with_frees.push(call("OpFreeText", v, data));
                }
            }
            ls = with_frees;
        }
        // Case b's premise — the tail IS the returned value, so its `__ncc_N` temp must
        // outlive the block — holds only when the block YIELDS the text.  A SCALAR tail that
        // consumes the temp (`len(s.name ?? "")`, `t.0 + len(t.1 ?? "")`) copies out the
        // number and leaves the String to nobody: one orphan per call (loft#1357).  Hoist the
        // value first, then free what it consumed, and let the tail be the hoisted scalar.
        let mut expr = expr;
        if !matches!(expr, Value::Null)
            && matches!(
                bl.result,
                Type::Integer(_)
                    | Type::Float
                    | Type::Single
                    | Type::Boolean
                    | Type::Character
                    | Type::Enum(_, false, _)
            )
        {
            let mut ncc = Vec::new();
            collect_consumed_ncc_text(&expr, function, &mut ncc);
            if !ncc.is_empty() {
                // An explicit `return <e>` hoists `<e>` and keeps the `return`.
                let (inner, was_return) = match expr.unspan() {
                    Value::Return(i) => ((**i).clone(), true),
                    _ => (expr.clone(), false),
                };
                self.ret_temp_counter += 1;
                let name = format!("__ret_{}", self.ret_temp_counter);
                let tmp = function.add_temp_var(&name, &bl.result);
                self.var_scope.insert(tmp, self.scope);
                self.var_order.push(tmp);
                ls.push(v_set(tmp, inner));
                for v in ncc {
                    ls.push(call("OpFreeText", v, data));
                }
                expr = if was_return {
                    Value::Return(Box::new(Value::Var(tmp)))
                } else {
                    Value::Var(tmp)
                };
            }
        }
        let scope_vars = self.variables(self.scope);
        for &v in &scope_vars {
            self.var_mapping.remove(&v);
        }
        let frees = self.free_vars(is_return, &expr, function, data, &bl.result, self.scope);
        for v in frees {
            ls.push(v);
        }
        ls
    }

    #[must_use]
    fn variables(&self, to_scope: u16) -> Vec<u16> {
        let mut scopes = HashSet::new();
        let mut sc = self.scope;
        let mut scope_pos = self.stack.len();
        loop {
            if sc == 0 {
                // never return function arguments
                break;
            }
            scopes.insert(sc);
            if sc == to_scope {
                break;
            }
            if scope_pos == 0 {
                break;
            }
            scope_pos -= 1;
            sc = self.stack[scope_pos];
        }
        // Iterate var_order in reverse (most-recently-inserted first) so that
        // OpFreeRef/OpFreeText are emitted in reverse-allocation order, satisfying
        // the LIFO invariant enforced by database::free().
        let mut res = Vec::new();
        for &v_nr in self.var_order.iter().rev() {
            if let Some(sc) = self.var_scope.get(&v_nr)
                && scopes.contains(sc)
            {
                res.push(v_nr);
            }
        }
        res
    }

    /// Is `v` the hidden NRVO return buffer promoted onto an argument slot, rather
    /// than a user parameter?
    ///
    /// A true parameter belongs to the caller and the callee may free none of it; the
    /// promoted buffer is a LOCAL that `classify_ret_promotion` renamed onto the hidden
    /// `__retbuf` attribute and `become_argument`ed, and this function mints its store.
    /// The un-renamed `__retbuf` placeholder is left out — no local was promoted onto
    /// it, so it holds no store of ours (loft#688).
    fn is_promoted_ret_buffer(&self, function: &Function, data: &Data, v: u16) -> bool {
        let n = function.name(v);
        n != "__retbuf"
            && data
                .def(self.d_nr)
                .attr_names
                .get(n)
                .is_some_and(|&a| data.def(self.d_nr).attributes()[a].hidden)
    }

    /// Enforces @FR-O-Derived: free placement is DERIVED, not decided — a local is freed
    /// iff it owns its store and does not transfer it out, once, at scope exit.  A
    /// per-site heuristic anywhere else in codegen is the bug that rule names.
    /// Does `d_nr`'s published return BORROW the closure it carries?
    ///
    /// The one question that decides whether a store this function mints and hands back will
    /// have an owner at the call site.  `published_ret_type` keeps the `__closure` index only
    /// where the caller is meant to read the result as a borrow, so its presence here IS the
    /// caller's reading — asked off the same attribute list, so the two cannot drift.
    fn return_borrows_closure(data: &Data, d_nr: u32) -> bool {
        if d_nr == u32::MAX || d_nr >= data.definitions() {
            return false;
        }
        let def = data.def(d_nr);
        let Some(idx) = def.attributes().iter().position(|a| a.name == "__closure") else {
            return false;
        };
        u16::try_from(idx).is_ok_and(|i| def.returned.depend().contains(&i))
    }

    /// May this function free the store the source `v` names — or does it belong to
    /// someone else?
    ///
    /// Enforces @FR-O-Proxy. The empty dep list is the cheap PROXY for "this binding owns
    /// its store" and the rule says it is unsound alone, so the two obligations it names
    /// are discharged together and in one place: the `O-Override` veto (`is_skip_free`,
    /// whose contract is "no ownership-derived free, in any spelling, for this binding"), and
    /// the carve-out that a user PARAMETER belongs to the caller while the promoted NRVO
    /// buffer is the one argument that is really a local this function minted.
    ///
    /// One home because it was written three times inside [`free_vars`](Self::free_vars) —
    /// once for a record source, once for a keyed one, once for the arm that disagrees
    /// about ownership — and each copy had to be extended on its own when a new shape
    /// arrived (loft#688, then loft#1022, then loft#1078 restating the same carve-out a
    /// third time). The keyed copy never gained the promoted-buffer half at all.
    ///
    /// The override consult is new here and is a GUARD rather than a fix: measured across
    /// `tests/scripts` and `tests/docs`, no `skip_free` binding currently reaches any of
    /// these sites, so today the obligation holds by accident. It now holds by
    /// construction.
    /// @FR-O-Proxy asks free — this IS the free question, asked once for every caller.
    fn owns_freeable_store(
        &self,
        function: &crate::variables::Function,
        data: &Data,
        v: u16,
    ) -> bool {
        function.tp(v).depend().is_empty()
            && !function.is_skip_free(v)
            && (!function.is_argument(v) || self.is_promoted_ret_buffer(function, data, v))
    }

    fn free_vars(
        &mut self,
        is_return: bool,
        expr: &Value,
        function: &mut Function,
        data: &Data,
        tp: &Type,
        to_scope: u16,
    ) -> Vec<Value> {
        let ret_var = returned_var_null_unified(expr, data.def_nr("OpNullRefSentinel"));
        // @PLN85 cluster II / A.1 part i (OWNERSHIP_MODEL row 100, invariant #5
        // "per binding, per path, complete") — the return-source SET, not the
        // single `returned_var`, drives free-suppression.  `returned_var`
        // collapses an arms-differ `match`/`if` to `u16::MAX`, which would free
        // every arm's transferred buffer at scope exit (the freed-return bug,
        // #405 / probe 05).  `collect_return_sources` is the union of every arm's
        // terminal var — the values this return transfers to the caller.
        //
        // The suppression is PATH-LOCAL: it is computed per `free_vars`
        // invocation and consumed only by THIS scope-exit's `get_free_vars`, so a
        // variable transferred on this return path is suppressed here while the
        // SAME variable, dead on a sibling path (an early `return e` vs a tail
        // `return [..]`, or the `null` arm of a nullable return), is still freed
        // by its own path's sweep.  A single global `skip_free` bit cannot encode
        // that path-dependence — marking the source do-not-free everywhere
        // over-suppresses and LEAKS the dead-path allocation (repro_p365's
        // `via_local`/`nested`, 25-nullable's `maybe_row`).  So the set is passed
        // down, not stamped onto the variable.
        let mut null_arm_record_sources: Vec<u16> = Vec::new();
        let return_sources: HashSet<u16> = if is_return {
            let mut sources = Vec::new();
            collect_return_sources(expr, data, &mut sources);
            // A nullable return (`if b { Struct{} } else { null }`) leaves the
            // present arm's work-ref placeholder orphaned on the null path.  When
            // a null arm is reachable, do NOT SET-suppress a Reference/Enum work-
            // ref source — hand it to the standard work-ref free path so the
            // orphan is freed.  See `return_has_null_arm`.
            let null_sentinel_nr = data.def_nr("OpNullRefSentinel");
            if return_has_null_arm(expr, null_sentinel_nr) {
                // @PLN85 P4-records: a record source with a reachable NULL arm
                // is a runtime JOIN — transferred to the caller on the present
                // path, an orphan (its preamble null-init ALLOCATES on interp)
                // on the null path.  The old design freed it unconditionally
                // (no orphan, but the present path returned a FREED store off
                // the eval stack — the poison-visible UAF).  Keep it
                // SUPPRESSED here and record it; the return leg below hoists
                // the value to `__ret_N` and emits
                // `OpFreeRefIfDistinct(src, __ret_N)` — the runtime decides.
                //
                // Not a PARAMETER: its store is the caller's (calls.md F-ParamHeap), so
                // the null arm of `if c { s } else { null }` has nothing of this frame's
                // to release for it — paired with the return it freed the caller's
                // record on every `null` answer, both backends.  A parameter REBOUND in
                // this body may hold a store of its own; that one is released by
                // identity against its entry stash at scope exit (`rebind_orig`).
                // ...and not a VIEW of one either, for the same reason and by the same rule.
                // `match v { [a, ..] => a, _ => null }` over a `vector<t>` PARAMETER binds
                // `a` to an element of the CALLER's store: the present arm hands that
                // element straight back and the null arm never assigns `a` at all, so the
                // conditional free is inert on both paths.  Inert but not harmless — `a`
                // lives in the match block while this free is emitted at the RETURN, so
                // native scoped the Rust `let` to the block and refused the program it had
                // just generated (E0425, loft#1415).  The interpreter never saw it: frame
                // slots have no block scope.
                //
                // `borrows_one_argument` is the exception the rule needs.  A nullable local
                // BOUND from a parameter and later reassigned from a minting call owns a
                // store on some paths and borrows on others (@FR-O-Latest, the D-own-16
                // shape), and this free is its ONLY one — `get_free_vars` skips every return
                // source.  So that one stays in, and the runtime comparison decides.
                let store_is_the_callers = |function: &Function, v: u16| {
                    (function.is_argument(v)
                        || function
                            .tp(v)
                            .depend()
                            .iter()
                            .any(|&d| function.is_argument(d)))
                        && !function.borrows_one_argument(v)
                };
                // Through `base()`: `@FR-L-Null` gives `t?` the same storage as `t`, so it is
                // the same binding and owns its store the same way.  Asked BARE, the one
                // shape this set exists for fell past it — a local that may be ABSENT is
                // exactly what a null-arm return is about, and `d: S?` is
                // `Optional(Reference)`, not `Reference`.  It got no conditional free, and
                // nobody else frees a return source (`get_free_vars` skips them all), so
                // every store the minting path handed up was owned by nobody: one record per
                // call, unbounded (loft#1422).  The dense spelling beside it was always
                // freed, which is what hid it.
                // The free lands where the RETURN is, so it may only name a variable that is
                // live there.  A match-arm binding lives in the arm's block while the return
                // sits outside it, and the interpreter tolerated the mismatch — frame slots
                // have no block scope — while native scopes the Rust `let` to the block and
                // refused the program (E0425, loft#1415).  `variables(to_scope)` is the
                // existing one home for *which variables are live at this scope*, so ask it
                // rather than restate the scope walk.
                let live_here = self.variables(to_scope);
                for &v in &sources {
                    if matches!(
                        function.tp(v).base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    ) && live_here.contains(&v)
                        && !store_is_the_callers(function, v)
                    {
                        null_arm_record_sources.push(v);
                    }
                }
                // loft#936 — a COLLECTION source is the same runtime join, one
                // level down.  `_vec_N` is a VIEW into a backing record
                // (`vector<T>["__vdb_N"]`), so it is the BACKING var that owns
                // the store and that `get_free_vars` suppresses (via
                // `backs_return_source`).  That suppression is only correct on
                // the arm that actually delivers the store: on the null arm the
                // caller gets the sentinel and the entry-allocated backing
                // record is owned by nobody — one orphan per CALL, so a loop
                // calling such a function leaked until the 65,535-slot store
                // table was exhausted.  `OpFreeRefIfDistinct` compares
                // `store_nr`, and the view shares the backing record's store, so
                // the delivering arm still transfers it untouched.
                for &v in &sources {
                    if !crate::parser::vectors::is_collection(function.tp(v)) {
                        continue;
                    }
                    for d in function.tp(v).depend() {
                        if d != v
                            && !function.is_argument(d)
                            && !null_arm_record_sources.contains(&d)
                        {
                            null_arm_record_sources.push(d);
                        }
                    }
                }
            }
            // @PLN85 F2 (the fuzzer's catch: `t[i] ?? N{..}`) — a
            // MULTI-source return mixing an OWNED record work-ref arm with
            // view arms is a runtime JOIN even WITHOUT a null arm: the owned
            // arm's store transfers only when ITS branch ran, else its
            // (interp-allocating) preamble store orphans.  The CALL-default
            // twin never hits this — a Call arm contributes no source var, so
            // the work-ref keeps its plain free.  Route the owned sources
            // through the same hoist + `OpFreeRefIfDistinct` leg the null-arm
            // join uses; view sources (non-empty deps) stay suppressed as
            // borrows.
            //
            // loft#1078 — the promoted NRVO buffer is one of those owned sources,
            // and `is_argument` alone excluded it.  `fn pick(c) -> S { w = S{a:7};
            // if c { S{a:9} } else { w } }` renames `w` onto the hidden buffer, so
            // both arms name an owned candidate and exactly one wins; on the arm
            // that does NOT deliver `w`, the store `w` minted is returned by nobody
            // and freed by nobody — one orphan per call, which is the same u16
            // store-table exhaustion loft#688 and loft#1022 each closed for their
            // own shape.  The carve-out is the one those two already established:
            // a user PARAMETER belongs to the caller and the callee frees none of
            // it, while the promoted buffer is the one argument that is really a
            // local this function minted (`is_promoted_ret_buffer` — hidden attr,
            // renamed off `__retbuf`).  It reaches here rather than loft#688's leg
            // because that leg excludes anything in `sources`, and the owning arm
            // puts it there — the identical reason loft#1022 restated the carve-out
            // in its own gate.
            if sources.len() > 1 {
                for &v in &sources {
                    if matches!(
                        function.tp(v),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    ) && self.owns_freeable_store(function, data, v)
                        && !null_arm_record_sources.contains(&v)
                    {
                        null_arm_record_sources.push(v);
                    }
                }
            }
            // loft#688 — the NRVO return buffer is an ARGUMENT, and `variables()`
            // stops at scope 0 ("never return function arguments"): a true
            // parameter belongs to the caller, so the callee must not free it.
            // An NRVO buffer breaks that premise.  It is a promoted LOCAL — the
            // rename in `classify_ret_promotion` gives the hidden `__retbuf` attr
            // the local's name and `become_argument`s it — and THIS function mints
            // its store with `OpDatabase`.  When a sibling return path delivers a
            // different store, the minted one is returned by nobody and freed by
            // nobody: one orphan per call, which exhausted the 65,535-entry store
            // table after 65,535 calls and was invisible below that.
            //
            // So treat it like any other candidate source that may or may not be
            // the returned store: route it through the same hoist +
            // `OpFreeRefIfDistinct` leg the null-arm join uses.  On the path that
            // DOES return it, `return_sources` holds it and it is skipped here; on
            // a path that returns something else the runtime comparison frees it,
            // and a buffer not yet minted on this path is the null sentinel, which
            // `free` ignores.
            //
            // A promoted buffer is exactly an argument whose attribute is HIDDEN
            // and no longer called `__retbuf`: `ref_return` renames the attr to the
            // local's name but leaves `hidden` set, and a user-declared parameter
            // is never hidden.  The un-renamed `__retbuf` placeholder is left alone
            // — no local was promoted onto it, so it holds no store of ours.
            //
            // loft#1096 — and a COLLECTION return's promoted buffer is left alone too,
            // because the premise above ("a buffer not yet minted on this path is the
            // null sentinel, which `free` ignores") is false for it.  The leg reads the
            // buffer as a local THIS function mints; that is true where the caller's
            // work-ref reaches the call as a bare `OpInitRef` sentinel, which is what a
            // record work-ref does.  A collection work-ref does not:
            // `codegen::gen_set_first_vector_null` gives an owned vector local
            // `OpInitRef` + `OpDatabase`, so the buffer arrives ALIVE, and the callee's
            // own `OpDatabase` then only clears it in place (`alloc_record_at` reuses a
            // live slot rather than minting beside it).  So there is never a distinct
            // callee-minted store to reclaim here, and the store this free reached was
            // always the CALLER's — which the caller still names and still frees at its
            // own scope exit.  A `-> vector<T>` function with a `null` arm therefore
            // handed the next call a freed record to `OpClearVector`, and a loop faulted
            // on its second iteration on both backends (`@FR-O-Owner`: a free is for a
            // store the value OWNS).
            let collection_return = crate::parser::vectors::is_collection(
                data.def(self.d_nr).returned.ret_promo_base(),
            );
            for v in 0..function.count() {
                if function.is_argument(v)
                    && !sources.contains(&v)
                    && !null_arm_record_sources.contains(&v)
                    && matches!(
                        function.tp(v),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                {
                    let n = function.name(v);
                    if n != "__retbuf"
                        && !collection_return
                        && let Some(&a) = data.def(self.d_nr).attr_names.get(n)
                        && data.def(self.d_nr).attributes()[a].hidden
                    {
                        null_arm_record_sources.push(v);
                    }
                }
            }
            // loft#1022 — the THIRD shape of the same runtime join, after the null arm
            // above and loft#688's promoted buffer: a return whose arms disagree about
            // OWNERSHIP.  `fn pick(bx, take) -> P { if take { bx.p } else { P { x: 9 } } }`
            // types as `P["bx"]` — a view — while the else arm mints its own store in a
            // work-ref.  `collect_return_sources` is the UNION of the arms, so the
            // work-ref lands in `return_sources` and its scope-exit free is suppressed on
            // EVERY path; on the borrowing path nothing returns it and nothing frees it.
            // One orphan per call, unbounded in a loop, and the store the entry preamble
            // allocated leaks even when the owning arm never runs.
            //
            // The suppression's own comment calls itself PATH-LOCAL, and it is — across
            // separate `return` statements.  A single `return` whose value is a JOIN puts
            // both arms in one set, which is the case the path-locality argument does not
            // cover.  So route it to the same hoist + `OpFreeRefIfDistinct` leg the other
            // two use and let the runtime decide: on the arm that delivers the store the
            // comparison matches and the free is a no-op, on the borrowing arm the stores
            // differ and the orphan is released.
            //
            // The gate needs BOTH conditions, and the loop a third.
            //
            // A RECORD return only.  A collection or text return carries its own mature
            // machinery — loft#936's backing-store comparison and the B5-L3 text hoist —
            // and its `__vdb_N` backing is a `Type::Reference` too, so a test on the
            // SOURCE's type alone claims it and re-routes a return that was already
            // correct (repro_p365's `nested` leaked its backing under exactly that).
            //
            // And a genuine BORROWING ARM, not merely a return type that carries deps.
            // A record LITERAL whose fields alias locals (`TableDef { columns: cols,
            // indexes: ixs }`) has deps too, and every one of its arms delivers the
            // source — nothing can be orphaned, so hoisting it to `__ret_N` and freeing
            // the original released the vectors the returned copy still names, and the
            // sqldb round trip read back zero indexes.
            //
            // And in the loop: only a source THIS function owns.  A source that is
            // itself a borrow carries deps naming what it views, and freeing that would
            // release the CALLER's store — an over-free where the defect is a leak.
            // loft#1142 — the FOURTH shape, and the one that needed the gate widened rather
            // than a new leg.  A KEYED return orphans differently from a record one: the
            // record case above needs an arm that is NOT a source, because that is the arm
            // whose path leaves the source unreturned.  A keyed join can have EVERY arm a
            // source and still orphan, because each arm's `__kvb_N` buffer is ALLOCATED
            // before the branch is tested — `scan_if`'s pre-init prefix emits `Set(v, Null)`
            // for each, and for a keyed local that is not a cheap null but an `OpDatabase`
            // store.  Exactly one arm runs; the rest are minted and freed by nobody, one
            // store per call and unbounded in a loop.  So the condition is *more than one
            // owned source*, not *an arm that is not a source*, and the leg is the same:
            // hoist the join to `__ret_N` and let `OpFreeRefIfDistinct` decide at runtime,
            // which is the only thing that can — which arm ran is not a static fact
            // (@FR-O-Complete: the ownership fact is per binding and PER PATH, and
            // `get_free_vars` was answering it per FUNCTION by suppressing every
            // `return_source` at once).
            //
            // Fixing the ALLOCATION instead would close only half of it: the leak
            // reproduces identically with named locals minted before the `if`
            // (`m = [..]; p = [..]; if c { p } else { m }`), where no pre-init runs at all.
            //
            // The gate needs an owned keyed source AND a way for it not to be returned.
            // "More than one source" covers the two shapes that actually leak — every arm a
            // minted buffer, and every arm a named local — while a PARAMETER arm counts
            // toward that number without being freeable itself: `if c { x } else { [lit] }`
            // has one owned source and still orphans it on the `x` path, which is the cell
            // that showed the first version of this gate was short.  `return_has_non_source_arm`
            // is the record leg's spelling of the same idea and is kept beside it, for an arm
            // that names no source at all.
            // A source that is itself a BORROW names what it views, and freeing that
            // releases the CALLER's store — an over-free where the defect is a leak.
            // `owns_freeable_store` is that question's one home; this leg asked it with a
            // parameter carve-out of its own that never gained the promoted-NRVO-buffer
            // half the record legs above carry.
            let owned_keyed_source = |v: u16| {
                crate::parser::vectors::is_keyed(function.tp(v))
                    && self.owns_freeable_store(function, data, v)
            };
            let keyed_join = crate::parser::vectors::is_keyed(tp.base())
                && (sources.len() > 1 || return_has_non_source_arm(expr, &sources))
                && sources.iter().any(|&v| owned_keyed_source(v));
            if keyed_join {
                for &v in &sources {
                    if !null_arm_record_sources.contains(&v) && owned_keyed_source(v) {
                        null_arm_record_sources.push(v);
                    }
                }
            }
            if matches!(tp.base(), Type::Reference(_, _) | Type::Enum(_, true, _))
                && return_has_non_source_arm(expr, &sources)
            {
                for &v in &sources {
                    if null_arm_record_sources.contains(&v)
                        || !matches!(
                            function.tp(v),
                            Type::Reference(_, _) | Type::Enum(_, true, _)
                        )
                        || !self.owns_freeable_store(function, data, v)
                    {
                        continue;
                    }
                    null_arm_record_sources.push(v);
                }
            }
            sources.into_iter().collect()
        } else {
            HashSet::new()
        };
        let mut ls = self.get_free_vars(function, data, to_scope, tp, ret_var, &return_sources);
        // @PLN85 P4-records — at a RETURN site, a record work-ref's store may
        // BE the returned store: a named local adopts the arm's fresh Object
        // (`v: E = Pass{..}; if c { v = Fail{..} }; v` — two candidate stores,
        // one winner at runtime), and an unconditional OpFreeRef frees the
        // winner too — the caller then reads a freed store (silently stale
        // without LOFT_POISON; the par t4 catch).  Make every record work-ref
        // free at a return CONDITIONAL on not being the returned store — for
        // an unrelated work-ref the stores are distinct and the free runs
        // exactly as before.
        if is_return
            && ret_var != u16::MAX
            && (ret_var as usize) < function.count() as usize
            && matches!(
                function.tp(ret_var),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
        {
            let free_nr = data.def_nr("OpFreeRef");
            let free_if = data.def_nr("OpFreeRefIfDistinct");
            for op in &mut ls {
                // ANY record-typed local's store may BE the returned store —
                // not only a parser-minted work ref: a NAMED local aliased
                // into the hidden return-buffer param (`best = cand` — the
                // NRVO buffer keeps raw-alias Sets by design) had its
                // unconditional free kill the returned store on the
                // reassigned path (the 150-i306 `choose` shape; poison read
                // the caller's field as 0xDEADBEEF).  Distinct stores free
                // exactly as before.
                if let Value::Call(d, args) = op
                    && *d == free_nr
                    && let Some(a0) = args.first()
                    && let Value::Var(w) = a0.unspan()
                    && matches!(
                        function.tp(*w),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                {
                    *op = Value::Call(free_if, vec![Value::Var(*w), Value::Var(ret_var)]);
                }
            }
        }
        // The B5-L3 wrap (Set(__ret_N, expr); free ops; Return(Var(__ret_N)))
        // must not fire when `expr` is already a `Return` or contains one
        // at its tail — otherwise we'd emit `let _ret = return …` (E0308 in
        // native).  Recurse through `Insert` (which scopes wraps Return in
        // for free-vars cleanup) and `Block`.
        let expr_is_terminal = expr_ends_in_return(expr);
        // @PLN85 P4-records — a null-arm record return: hoist the value to a
        // `__ret_N` temp, then free each suppressed JOIN source CONDITIONALLY
        // (`OpFreeRefIfDistinct(src, __ret_N)`): the present arm returns the
        // source's store (not distinct → kept, transferred to the caller); the
        // null arm returns the sentinel (distinct → the preamble-allocated
        // placeholder is freed — no orphan).  Runs FIRST: the fast path would
        // otherwise emit `Return(expr)` with the sources leaking on the null
        // path (frees suppressed above), and the legacy path would re-open the
        // eval-stack UAF.
        if is_return && !expr_is_terminal && !null_arm_record_sources.is_empty() {
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            // loft#1186 / @PLN150 — when this function's PUBLISHED return names its
            // `__closure`, its callers read the result as a borrow, and the not-distinct leg
            // then leaves the store the callee minted owned by nobody: the callee does not
            // free it (it is the value it returns) and the caller will not (it borrows).
            // `OpFreeRefOrHandUp` is `OpFreeRefIfDistinct` with an owner on that leg.  The
            // distinct leg is identical, so a function whose return does not borrow keeps
            // exactly the op it had.
            let free_if = data.def_nr(if Self::return_borrows_closure(data, self.d_nr) {
                "OpFreeRefOrHandUp"
            } else {
                "OpFreeRefIfDistinct"
            });
            let mut result = Vec::with_capacity(ls.len() + null_arm_record_sources.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            for &src in &null_arm_record_sources {
                result.push(Value::Call(free_if, vec![Value::Var(src), Value::Var(tmp)]));
            }
            result.append(&mut ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        }
        // @PLN85 poison-green — a bare-Var tail is free-safe (its slot holds
        // the value) EXCEPT a `&τ` place ref (`RefVar`): returning it DEREFS
        // the place DbRef at the Return, after `ls`'s frees released the
        // source store (the @PLN87 L3/L4 live-read shapes under LOFT_POISON).
        // Exclude it from the fast path so it takes the B5-L3 hoist below —
        // `Set(__ret_N, Var(r))` performs the deref BEFORE the frees.
        // (Text-inner RefVars are EXCLUDED from the exclusion: a `&text` tail
        // is the promoted out-BUFFER returned per the text-return contract —
        // the buffer lives in the caller, so returning it raw is free-safe,
        // and hoisting it emitted a native `Str::new(&local_String)` dangle.)
        let var_is_place_ref = !ls.is_empty()
            && matches!(expr, Value::Var(v)
                if matches!(function.tp(*v), Type::RefVar(inner)
                    if !matches!(inner.base(), Type::Text(_))));
        // @PLN85 Class B2 — a plain text LITERAL tail reads none of the
        // to-be-freed locals and has no side effects, so `ls (frees); return
        // "lit"` is correct and copy-free.  The B5-L3 text hoist below would
        // otherwise mint an OWNED `__ret_N = "lit"` copy (skip_free) that the
        // caller consumes-and-leaks — the leak that appears whenever a
        // text-returning fn with ANY freeable local returns a literal (the
        // p54 match-of-literals / json-classify family).  Returning the literal
        // directly is exactly what a fn with NO frees already emits.
        //
        // @PLN85 corpus — the null-text sentinel `OpConvTextFromNull()` (an
        // early `return null` in a `-> text?` fn) is the SAME free-safe shape:
        // it lowers to `Str::new(STRING_NULL)`, a borrowed static that owns no
        // allocation and aliases none of the to-be-freed locals.  Treated as a
        // literal it returns directly (matching native's `return
        // Str::new(STRING_NULL)`); routed through the B5-L3 text hoist it minted
        // an OWNED skip_free `__ret_N` copy of the sentinel that a non-copying
        // caller (`f(-1) == null`) never freed → append_text orphan.
        let expr_is_null_text_sentinel = matches!(expr.unspan(),
            Value::Call(d, args) if args.is_empty() && *d == data.def_nr("OpConvTextFromNull"));
        // `return ta` where `ta` is an owned text LOCAL and the function holds a hidden
        // `&text` buffer that is not `ta`: deliver through the buffer and free the local
        // (@FR-F-Ret / @FR-F-Call).  The bare-Var fast path below returns the local's slot
        // as-is, which is right for a scalar and for the buffer itself, and hands up a
        // view of an orphan for a `String` nothing frees — a lambda whose one buffer went
        // to `tb` returned `ta` that way, one orphan per call (loft#1357).
        if is_return
            && let Value::Var(v) = expr.unspan()
            && !function.is_argument(*v)
            && !function.is_skip_free(*v)
            && matches!(function.tp(*v).base(), Type::Text(_))
            && !matches!(function.tp(*v), Type::RefVar(_))
            && let Some(buf) = any_text_return_buffer(function, data, self.d_nr)
            && buf != *v
        {
            let mut result = Vec::with_capacity(ls.len() + 3);
            result.push(v_set(buf, Value::Var(*v)));
            result.push(call("OpFreeText", *v, data));
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(buf))));
            return result;
        }
        if ls.is_empty()
            || ((matches!(expr, Value::Null | Value::Var(_) | Value::Text(_))
                || expr_is_null_text_sentinel)
                && !var_is_place_ref)
        {
            if is_return && !expr_is_terminal {
                ls.push(Value::Return(Box::new(expr.clone())));
            } else if matches!(expr, Value::Null) {
                // skip
            } else {
                ls.push(expr.clone());
            }
        } else if expr_is_terminal {
            // expr is already a `Return(...)` (or a `Block`/`Insert(...)` ending
            // in one) — the cleanup was emitted alongside it by the inner Return
            // arm's free_vars call.  Re-emitting `ls` here would duplicate every
            // OpFreeText/OpFreeRef (and tack on a dead `Return(Null)`).  Just
            // propagate the terminal as-is.  #549 bug 2: a terminal *Block* must
            // hit this dedup BEFORE the `Value::Block` insert_free arm below —
            // an explicit `return (owned_text, …)` at a body tail is processed by
            // both the `Value::Return` scan arm AND `convert`'s is_body_return
            // tail sweep; the first makes the synthetic tuple block terminal, and
            // without ordering this check first the second re-ran `insert_free`,
            // emitting a second `OpFreeText` on the owned element (double free
            // under `-C debug-assertions=on`; text.rs:334).
            return vec![expr.clone()];
        } else if let Value::Block(bl) = expr {
            return self.insert_free(bl, &ls, is_return, data, function);
        } else if is_return && is_value_return_type(tp) && !expr_is_terminal {
            // B5-L3: when a value-returning function's tail expression is a
            // non-Block, non-Var, non-Null value (If/Match/Call etc.) and
            // there are free ops to run before return, save the expression's
            // value to a temp, run the free ops, then return the temp.  The
            // old path inserted the expression as a discarded statement and
            // emitted Return(Null) — interpreter bytecode got away with it by
            // reading the expression's result from top-of-stack via Return's
            // `value` bytes, but native codegen produced `let _ = expr; ...;
            // return 0` and dropped the function's actual return value.
            // Skip when expr is already a `Value::Return(...)` — wrapping
            // would generate `let _ret = return …` (E0308 in native).
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            free_copied_text_sources(&mut result, expr, &ls, function, data);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && !expr_is_terminal
            && ret_var == u16::MAX
            && is_heap_return_type(tp)
            && !matches!(expr, Value::Var(_))
            && expr.is_place_read(data)
        {
            // loft#754 — the B5-L3 rule for a HEAP return (`vector` / record /
            // struct-enum) whose tail is a PLACE read.  `is_value_return_type`
            // names only the scalars and the text branch below only text, so
            // such a tail with pending frees reached the fall-through and was
            // emitted as a DISCARDED statement plus a fabricated
            // `Return(Null)`.  The interpreter read the value off eval-stack
            // top and answered correctly; native emitted
            // `let _ = expr; …; return DbRef::NULL`, so
            // `fn f(w) -> vector<u8> { if … { return []; } w.items[0].bytes }`
            // handed back an EMPTY vector — silently, and on one backend only.
            // (rustc even flagged it as `unused_must_use` on the dropped
            // element read.)
            //
            // The hoist states the interpreter's own order in the IR: evaluate
            // the tail, run the frees, return the captured value.
            //
            // A PLACE read is the whole class, and the bound is load-bearing in
            // both directions.  Only a place leaves its value on the eval stack
            // alone — it allocates nothing and writes no return buffer — so
            // only a place can be dropped by a `Return(Null)`; and `Set(tmp,
            // place)` is a bare `DbRef` copy, which is why the hoist adds no
            // ownership.  A CALL tail already delivers through its hidden
            // buffer, and hoisting one instead engaged the store-transfer
            // machinery (`protect_store_frees` + `CopyRefOrNull`) around a
            // borrowed argument, which over-froze the caller's store
            // ("Delete on locked store", `return-borrow-of-mutated-arg`).  A
            // bare `Var` is excluded because the fast path above already
            // returns it directly.
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && !expr_is_terminal
            && ret_var == u16::MAX
            && is_heap_return_type(tp)
            && !has_return_buffer(self.d_nr, data)
            && tail_is_value_call(expr, data)
        {
            // loft#793 — the CALL sibling of loft#754, in the one regime that
            // fix's "a CALL tail already delivers through its hidden buffer"
            // does not cover: a NULLABLE heap return (`-> S?`,
            // `-> vector<S>?`, `-> StructEnum?`).  Only a DENSE heap return is
            // given a hidden `__retbuf` argument — both reservation sites
            // (`parser/mod.rs`, `parser/definitions.rs`) gate on
            // `Reference | Vector | Enum(_, true, _)`, which `Optional` is not
            // — so for a nullable one the callee's value comes back ONLY as
            // the call's own return value.
            //
            // Dropped, the fall-through emitted the call as a DISCARDED
            // statement plus a fabricated `Return(Null)`.  The interpreter
            // read the value off eval-stack top and answered correctly, so
            // the whole class was invisible there; native — and any
            // `StaticCall` into a library's compiled half, which is why it
            // surfaced across a library boundary first — returned the null
            // sentinel.  `fn f() -> S? { return mk(); }` answered null,
            // silently, with the record left leaked.
            //
            // Hoist the call into `__ret_N` and return that, and make each
            // pending record/vector free CONDITIONAL on not being the hoisted
            // store: a callee with a return buffer may deliver a fresh store
            // OR chain the work ref this frame passed in, and only the
            // runtime knows which.
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let free_nr = data.def_nr("OpFreeRef");
            let free_if = data.def_nr("OpFreeRefIfDistinct");
            for op in &mut ls {
                if let Value::Call(d, args) = op
                    && *d == free_nr
                    && let Some(Value::Var(w)) = args.first().map(Value::unspan)
                    && matches!(
                        function.tp(*w),
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
                {
                    *op = Value::Call(free_if, vec![Value::Var(*w), Value::Var(tmp)]);
                }
            }
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            result.append(&mut ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && matches!(tp.base(), Type::Text(_))
            && !expr_is_terminal
            && !matches!(expr.unspan(), Value::Null)
            && let Some(buf) = text_return_buffer_for(expr, function, data, self.d_nr)
        {
            // @FR-F-Ret / @FR-F-Call — an owned text return is delivered through the
            // CALLER's hidden `&text` buffer, never as a view of a local of this frame,
            // and the frame frees every local it owns when it drops.  The block tail
            // already writes that buffer (`text_return` promotes it); an EARLY
            // `return <call>` / `return <view>` / `return a ?? b` used to reach the
            // `__ret_N` hoist below instead, which copied the value into a frame-local
            // `String` that nothing freed — one orphan per call, unbounded in a loop
            // (loft#1338).  Write each arm of the value into the buffer this function
            // already holds from its caller (per arm, so native's arm types stay
            // uniform), free the frame-local temps the copy drained, run the scope-exit
            // frees, and return the buffer.  The buffer is one the value does not
            // READ: `"{x}-{n}"` written into `x` would clear `x` before rendering it.
            let mut delivered = expr.clone();
            crate::parser::Parser::push_text_arms_into(
                &mut delivered,
                buf,
                data.def_nr("OpCreateStack"),
            );
            let mut result = Vec::with_capacity(ls.len() + 3);
            result.push(delivered);
            free_copied_text_sources(&mut result, expr, &ls, function, data);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(buf))));
            return result;
        } else if is_return
            && matches!(tp.base(), Type::Text(_))
            && !expr_is_terminal
            && !matches!(expr.unspan(), Value::Null)
            && let Some(buf) = any_text_return_buffer(function, data, self.d_nr)
        {
            // The value reads every buffer this function holds (`rest[0..3]` where `rest` IS
            // the promoted buffer; a `match` arm that yields the work text), so it cannot be
            // written into one directly — clearing the buffer first would destroy what is
            // being rendered.  STAGE it: copy into a frame-local temp, move the temp's bytes
            // into the buffer, free the temp, run the frees, return the buffer.  Before this
            // the temp itself was returned and orphaned, one `String` per call (loft#1357).
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            function.set_skip_free(tmp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 4);
            result.push(v_set(tmp, expr.clone()));
            free_copied_text_sources(&mut result, expr, &ls, function, data);
            result.push(v_set(buf, Value::Var(tmp)));
            result.push(call("OpFreeText", tmp, data));
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(buf))));
            return result;
        } else if is_return && matches!(tp.base(), Type::Text(_)) && !expr_is_terminal {
            // The residual of the arm above: a text-returning function that holds NO
            // hidden `&text` buffer (a literal tail, or a tail whose promotion the
            // targeted pass declined) and whose value the value reads every buffer of.
            // Save the value's text to a `__ret_N` temp, run the free ops, then return
            // the temp.  The temp's String holds an OWN copy (`OpAppendText` copies
            // bytes), so the frees do not dangle the returned Str.  Its own scope-exit
            // `OpFreeText` is suppressed (`skip_free`): the caller copies the bytes on
            // return, and the String is ORPHANED — this is the one delivery that
            // violates @FR-F-Call's "owned locals freed", kept only where no buffer
            // exists to deliver through (`use_analysis::text_return_orphan_risk` is
            // the predicate that hands such a function a buffer, so a leak here means
            // that predicate did not see this return).
            //
            // Native codegen also needs the wrap (otherwise the call result is
            // dropped + `return null` returns the typed null sentinel), and then
            // collapses `Set(__ret, call); …; Return(__ret)` back to `return
            // Str::new(call(...))` in `output_block`, dropping the temp — which is
            // why native never orphans here.
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            function.set_skip_free(tmp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            free_copied_text_sources(&mut result, expr, &ls, function, data);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && let Type::Tuple(elems) = tp
            && elems.iter().any(|e| matches!(e, Type::Text(_)))
            && !expr_is_terminal
            && let Value::Tuple(orig_elems) = expr.unspan()
            && orig_elems.len() == elems.len()
        {
            // @P329: tuple-of-text return — when an element is a non-literal
            // expression (typically a Call returning text that borrows from
            // a local), hoist it to a `__ret_text_N` temp before running
            // scope frees.  `Set(__ret_text_N, elem)` lowers to OpAppendText
            // (line 526 / src/state/codegen.rs), which deep-copies bytes
            // into the temp's owned String.  The frees then run safely; the
            // returned tuple's text elements point to temp Strings that
            // outlive the function's scope (skip_free marks them so the
            // function epilogue leaves the allocation for the caller's
            // AppendText to consume on return — same pattern as the
            // single-text B5-L3 branch above, generalised across tuple
            // elements).
            //
            // Without this, a function shape like
            //   fn f<T: Printable>(x: T) -> (text, text) { (x.to_text(), "x") }
            // returns a tuple whose element 0 Str points into the caller's
            // (function-local) __work_1 buffer; the scope's OpFreeText runs
            // BEFORE the Return, invalidating the Str — the caller reads
            // empty / garbage bytes.  See PROBLEMS.md @P329.
            let mut new_elems = Vec::with_capacity(orig_elems.len());
            let mut pre_ops = Vec::new();
            for (elem_expr, elem_type) in orig_elems.iter().zip(elems.iter()) {
                let unspanned = elem_expr.unspan();
                if matches!(elem_type, Type::Text(_))
                    && !matches!(unspanned, Value::Text(_) | Value::Var(_) | Value::Null)
                {
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_text_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, &Type::Text(Deps::none()));
                    function.set_skip_free(tmp);
                    self.var_scope.insert(tmp, self.scope);
                    self.var_order.push(tmp);
                    pre_ops.push(v_set(tmp, elem_expr.clone()));
                    new_elems.push(Value::Var(tmp));
                } else {
                    new_elems.push(elem_expr.clone());
                }
            }
            let mut result = Vec::with_capacity(pre_ops.len() + ls.len() + 1);
            result.extend(pre_ops);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Tuple(new_elems))));
            return result;
        } else if is_return
            && !expr_is_terminal
            && matches!(
                tp,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
            && matches!(expr.unspan(), Value::If(c, _, _)
                if matches!(c.unspan(), Value::Insert(ops)
                    if ops.first().is_some_and(|o| matches!(o,
                        Value::Set(v, _) if function.name(*v).starts_with("__lift_")))))
        {
            // @P378(b) — a heap-returning tail `If` whose CONDITION lifted a
            // ref-temp (`__lift_N = call()`; the branches were not unified to a
            // shared work-ref, so `ret_var == u16::MAX`) and which has frees to
            // run before returning.  The fall-through (below) inserts the If as
            // a discarded statement + `Return(Null)`: interpret reads the value
            // off eval-stack-top, but native returns the null DbRef sentinel
            // (keys.rs:251 OOB in the caller).  A `Set(var, If)` save-to-temp
            // does NOT work (native voids the if/else branches → E0308); only
            // `Return(If(...))` value-emits them.  So PRESERVE the Return(If):
            // pull the condition's lift-preamble out, evaluate the boolean to a
            // value-typed temp, run the frees (the lift's OpFreeRef), then
            // `Return(If(Var(cond_tmp), t, f))` — the if/else stays the Return's
            // value-expression and the lift is freed before (not after) it.
            let Value::If(cond, t, f) = expr.unspan().clone() else {
                unreachable!()
            };
            let mut result = Vec::new();
            // split `Insert([__lift = …, …, bool_expr])` into preamble + bool.
            let bool_expr = if let Value::Insert(ops) = cond.unspan() {
                let mut ops = ops.clone();
                let last = ops.pop().expect("non-empty condition Insert");
                result.extend(ops);
                last
            } else {
                (*cond).clone()
            };
            self.ret_temp_counter += 1;
            let cname = format!("__cond_{}", self.ret_temp_counter);
            let cond_tmp = function.add_temp_var(&cname, &Type::Boolean);
            self.var_scope.insert(cond_tmp, self.scope);
            self.var_order.push(cond_tmp);
            result.push(v_set(cond_tmp, bool_expr));
            result.extend(ls);
            result.push(Value::Return(Box::new(v_if(Value::Var(cond_tmp), *t, *f))));
            return result;
        } else {
            // @PLN85 poison-green — the block-VALUE variant of the B5-L3 rule:
            // a non-Void block's exit frees run AFTER the tail expression but
            // BEFORE the enclosing consumer copies the value out
            // (`test_value = { mk()[0] }` — the text tail borrows the
            // block-local vector's element bytes; the block-exit OpFreeRef
            // poisons them before the Set's byte copy).  Hoist the value to a
            // temp: `Set(__blk_N, expr)` deep-copies the bytes (OpAppendText)
            // while the source store is still live, the frees run, and the
            // temp is the block's value.  Text-typed only — the one shape
            // where the Set IS a deep copy; record/vector block values keep
            // their existing paths.
            // (A branch tail — if/match — is EXCLUDED: its arms are unified
            // to write the assignment target directly, so the hoist's Set
            // would append the branch's value a SECOND time onto what the arm
            // already wrote — `if c { null } else { "error" }` read back
            // "errorerror".  Branch tails keep their existing arm-delivery.)
            if !is_return
                && !ls.is_empty()
                && matches!(tp.base(), Type::Text(_))
                && !matches!(expr, Value::Null | Value::Var(_))
                && !Self::tail_is_branch(expr)
                && !expr_is_terminal
            {
                self.ret_temp_counter += 1;
                let name = format!("__blk_{}", self.ret_temp_counter);
                let tmp = function.add_temp_var(&name, tp);
                // @PLN85 n3 — register the hoist temp at the FUNCTION BODY scope
                // (1), not `self.scope` (this nested block's scope, where `__blk_N`
                // is the tail value and so is EXCLUDED from `get_free_vars` as the
                // block's `ret_var` → its owned String leaked, e.g.
                // `test_value = { a = Item{name:"x"}; b = a; a.name }`).  The block
                // value is delivered to the outer consumer by COPY (OpAppendText),
                // never moved, so the temp stays owned and must be freed.  Its
                // `InitText` is hoisted to the function root (the `lift_texts`
                // mechanism below), so its `OpFreeText` must fire exactly ONCE at
                // function exit — registering at scope 1 makes the function-exit
                // sweep (`get_free_vars(to_scope = 1)`) emit it, matching the
                // root-level init and avoiding a per-iteration double-free in loops.
                // The hoist only fires for `!is_return` blocks (always nested,
                // scope >= 2), so `__blk_N` is never a function return value — the
                // function-exit free can never free a value the caller adopts.
                self.var_scope.insert(tmp, 1);
                self.var_order.push(tmp);
                self.lift_texts.push(tmp);
                let mut result = Vec::with_capacity(ls.len() + 2);
                result.push(v_set(tmp, expr.clone()));
                result.append(&mut ls);
                result.push(Value::Var(tmp));
                return result;
            }
            // Whether anything runs BETWEEN the value and the return.  `ls` holds
            // this scope's frees; the `insert` below puts the value in front of
            // them, so a non-empty `ls` here is exactly "frees follow the value".
            let frees_follow = !ls.is_empty();
            ls.insert(0, expr.clone());
            if is_return {
                // P236: when `expr` is an `If/Match` whose unified
                // tail var is known (returned_var(expr) recurses
                // through If — see line 1454), emit
                // `Return(Var(ret_var))` instead of the legacy
                // `Return(Null)` pattern.  The legacy pattern relied on
                // OpReturn(value=N) reading from eval-stack top —
                // bytecode-only, native discards the if/else's value
                // and returns the typed null sentinel.  After Step 2
                // unification (parser/control.rs::unify_if_branches_work_refs)
                // every branch of the if/else writes to the SAME work-
                // ref via OpDatabase + per-field SetInt; the if/else
                // statement leaves all writes in place; native
                // `return var___ref_N` then returns the active
                // branch's value correctly.
                // loft#1097 — `ret_var` cannot carry the sentinel when the tail has a
                // NULL arm and the var it unified onto is a COLLECTION.
                // `returned_var_null_unified` folds a null arm into its sibling's var
                // on the premise it states itself: *"the work-ref null-inits at
                // function entry and a null arm never allocates into it, so
                // `Return(Var(v))` yields the same null the sentinel did"*.  That holds
                // for a RECORD work-ref, which `gen_set_first_ref_null` sentinel-inits.
                // It is false for a collection: `gen_set_first_vector_null` gives an
                // owned vector local `OpInitRef` + `OpDatabase`, and a PROMOTED buffer
                // arrives alive from the caller — so on the null path `Var(v)` is a
                // live, populated vector.  `f(-1) == null` answered FALSE while
                // `len(f(-1))` answered 2, with no diagnostic: @FR-E-Null says the
                // sentinel is a real observable value, and calls.md's `(F-Return)` says
                // a body ending in an expression returns THAT expression — here the
                // expression was demoted to a statement and its value dropped.
                //
                // Hoist the tail's value to a temp instead — the shape the null-arm
                // RECORD join above already uses — so the frees still run between the
                // value and the return while the arm's own answer, sentinel included,
                // is what comes back.  Like loft#957's temp below it is deliberately
                // NOT registered in `var_scope`: it holds the value being transferred
                // to the caller, and a scope-exit free of it would free what the caller
                // adopts.
                let null_arm_needs_the_value = ret_var != u16::MAX
                    && frees_follow
                    && !expr_is_terminal
                    && (ret_var as usize) < function.count() as usize
                    && crate::parser::vectors::is_collection(function.tp(ret_var))
                    && return_has_null_arm(expr, data.def_nr("OpNullRefSentinel"));
                if null_arm_needs_the_value {
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_tail_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, tp);
                    ls[0] = v_set(tmp, expr.clone());
                    ls.push(Value::Return(Box::new(Value::Var(tmp))));
                } else if ret_var != u16::MAX {
                    ls.push(Value::Return(Box::new(Value::Var(ret_var))));
                } else if frees_follow
                    && *tp != Type::Void
                    && !expr_is_terminal
                    // A CALL, and only a call.  That is the producer the diagnosis
                    // names — the one whose result no binding holds — and it is the
                    // same shape `chain_site_set_shape` promotes into `__retbuf`
                    // when promotion does run.  A wider test is not a safer one: an
                    // earlier cut allowed any non-null tail and so fired on a
                    // COROUTINE's body, whose tail is the `while` loop itself.  That
                    // wrapped the loop into `__ret_tail_1 = loop { … }`, and the
                    // state-machine lowering then could not see the captured
                    // parameters — four `coroutine_matrix` cells failed to compile
                    // with `cannot find value var_n`.  An `iterator` return is
                    // excluded outright for the same reason: a coroutine does not
                    // return its body's value.
                    && matches!(expr.unspan(), Value::Call(_, _) | Value::CallRef(_, _))
                    && !matches!(tp.base(), Type::Iterator(_, _))
                {
                    // loft#957 — the same eval-stack reliance P236 names above, for
                    // the case where the value lives in NO variable: a tail
                    // `return <call>` whose callee is a bodiless `#rust` native
                    // returning a collection (`read_bytes`, `list_dir`).  Return
                    // promotion never runs for it — there is no local candidate to
                    // promote — so nothing binds the result, and the legacy shape
                    // lowered to `read_bytes(p); free …; return null`.  The
                    // interpreter answered correctly by accident, its `OpReturn`
                    // taking the eval-stack top the call happened to leave there;
                    // native emitted the typed null sentinel and the bytes were
                    // gone, silently, on a backend `loft test --interpret` cannot
                    // see.  P236 could reuse a variable that already existed; here
                    // there is none, so give the value one and return THAT.
                    //
                    // Gated on frees actually following: with nothing between the
                    // value and the return, the existing shape already emits
                    // `return <expr>` directly and needs no temp.  The temp is
                    // deliberately NOT registered in `var_scope` — it holds the
                    // value being transferred to the caller, so a scope-exit free
                    // of it would free what the caller adopts.
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_tail_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, tp);
                    ls[0] = v_set(tmp, expr.clone());
                    ls.push(Value::Return(Box::new(Value::Var(tmp))));
                } else {
                    ls.push(Value::Return(Box::new(Value::Null)));
                }
            }
        }
        ls
    }

    /// Which variables this scope must free on the way out, as the `OpFreeRef` /
    /// `OpFreeText` / per-element ops to emit before leaving it.
    ///
    /// Enforces @FR-O-Derived — free placement is DERIVED, not decided: a local is freed
    /// iff it OWNS its store and does not transfer it out, once, at scope exit — and
    /// @FR-O-Owner, the single-owner invariant that makes "once" correct.  There is no
    /// per-site heuristic here; every arm answers from a carried fact.
    ///
    /// `ret_var` and `return_sources` are what "does not transfer it out" means in
    /// practice: a store handed to the caller is the caller's to free (@FR-O-Move), so its
    /// scope-exit free is suppressed.  `return_sources` is PATH-LOCAL — a source dead on a
    /// sibling path is absent from it and is still freed by that path's own sweep, which is
    /// what keeps @FR-O-Complete true without a global "skip" stamp that would over-suppress
    /// and leak.
    ///
    /// ⚠ Ownership is read here from a carried fact, but "empty deps" is only a PROXY for
    /// it (loft#723) — see [`crate::variables::Function::is_skip_free`], the second fact
    /// that vetoes the proxy for a borrow whose dep list was never populated.
    #[allow(clippy::too_many_lines)]
    fn get_free_vars(
        &mut self,
        function: &mut Function,
        data: &Data,
        to_scope: u16,
        tp: &Type,
        ret_var: u16,
        return_sources: &HashSet<u16>,
    ) -> Vec<Value> {
        let scope_debug = std::env::var("LOFT_LOG").as_deref() == Ok("scope_debug");
        let mut ls = Vec::new();
        let vars = self.variables(to_scope);
        if scope_debug {
            eprintln!(
                "[get_free_vars] fn={} to_scope={to_scope} scope={} vars={vars:?} ret_var={ret_var} \
                 return_sources={return_sources:?}",
                data.def(self.d_nr).name,
                self.scope
            );
        }
        // @PLN85 A.1 part i — the directly-owned heap BUFFER of EACH transferred
        // return arm (the union-of-arms SET, not just `returned_var`'s single
        // var) is freed by the caller, so suppress its scope-exit free here.
        // PATH-LOCAL: `return_sources` is this return's set, so a source dead on
        // a sibling path is absent and still freed by that path's sweep (no
        // global `skip_free` stamp — that would over-suppress and leak the
        // dead-path allocation).  The dep buffer of a BORROWING terminal
        // (`_vec_N["__vdb_N"]`) is handled in the `in_ret` computation below.
        //
        // The SET drives suppression ONLY for the heap-buffer block (Vector /
        // Reference / Enum / keyed).  TEXT and TUPLE returns keep their own,
        // mature free paths (`OpFreeText` + the B5-L3 `__ret_N` text hoist;
        // per-element tuple frees) — a `match`-returning-text whose owned
        // `__work_N` buffer is alive but unused on a sibling arm MUST still be
        // freed, and short-circuiting the whole iteration on `return_sources`
        // would leak it (the enum-vector param-return `show()` regression).
        //
        // loft#1150 — `.base()`, because the block this set drives is *"Vector / Reference /
        // Enum / keyed"* and a `τ?` IS one: `@FR-L-Null` gives it the same layout and the same
        // store.  Asked bare, an `Optional(Hash)` arm buffer failed the suppression and took
        // an UNCONDITIONAL scope-exit free on top of the conditional one loft#1142 emits — so
        // the store being returned was freed and the caller read a dead record.  That is the
        // fifth site where this same list has drifted short by the wrapper (`is_dbref` here
        // and at D-own-13, `deps_mut`, `is_keyed`, `depend`); `is_dbref`'s own doc records
        // that it drifts when restated, and it drifts when asked BARE too.
        let suppress_source = |function: &Function, v: u16| {
            return_sources.contains(&v) && crate::data::is_dbref(function.tp(v).base())
        };
        for v in vars {
            if v == ret_var || suppress_source(function, v) {
                continue;
            }
            // loft#1336 / @FR-O-Witness — a witnessed local's store is released through its
            // witness, which names it only while the local still holds it; the local itself
            // is never-free.  A returned local is skipped above like any other: the store is
            // handed up, and the witness is not released for it.
            if let Some(&w) = self.owner_witness.get(&v) {
                ls.push(release_witness(w, data));
                continue;
            }
            // on=4 iteration scratch (`hash_scratch`): a `return` out of an exposed loop
            // bypasses the loop epilogue's OpFreeScratch, so free the dedicated scratch
            // store here at scope exit too.  OpFreeScratch is conditional (frees only a
            // read-only source's dedicated store) and rec==0-guarded; the epilogue nulls
            // the var on the complete/break paths, so the two never double-free.
            // (expose-iteration-scratch.md Open question A.)
            if function.name(v).contains("hash_scratch") {
                ls.push(call("OpFreeScratch", v, data));
                continue;
            }
            // T1.3: tuple scope exit — free owned elements in reverse index order.
            if let Type::Tuple(elems) = function.tp(v) {
                let elems = elems.clone();
                ls.extend(tuple_owned_elem_frees(&elems, v, data, function));
                continue;
            }
            if matches!(function.tp(v).base(), Type::Text(_)) {
                // @PLN25 slice (c): peel `Optional` — a `text?` local owns the same heap
                // text as `text` and must be freed identically (else its interval is not
                // extended and the slot allocator aliases it — the `text? = text?` copy
                // read back empty).
                // @PLAN52 cluster I iteration 2 (2026-05-30): honor skip_free
                // for text vars too.  The file-level "Text exception"
                // doc-comment ("OpFreeText is always emitted ... regardless
                // of deps") remains true for borrowed-from-parameter text
                // (`dep` non-empty, not skip_free).  The new rule only
                // suppresses OpFreeText for an EXPLICITLY-set skip_free text
                // temp — used by the `__ncc_N` null-coalesce temp at
                // `src/parser/operators.rs::build_null_coalesce_default` so
                // the present-path Str outlives the block scope.  Native
                // emit's `needs_ncc_materialise` (in `output_block`)
                // materialises an owned String inside the block tail so the
                // outer consumer takes ownership cleanly on both backends.
                if !function.is_skip_free(v) {
                    ls.push(call("OpFreeText", v, data));
                }
            }
            // P193: include keyed collections (Sorted/Hash/Index/Radix)
            // — `gen_set_first_keyed_null` allocates a fresh store via
            // `OpDatabase` for each local-var keyed collection, so each
            // needs scope-exit `OpFreeRef`.  Without this they leak as
            // "Stores not freed at program exit".
            if let Type::Reference(_, dep)
            | Type::Vector(_, dep)
            | Type::Enum(_, true, dep)
            | Type::Sorted(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Index(_, _, dep)
            // @PLN25 slice (c): peel `Optional` so an owned `vector?`/`reference?` local is
            // still freed at scope exit (same reasoning as the `text?` case above).
            | Type::Radix(_, _, dep) | Type::Trie(_, _, dep) = function.tp(v).base()
            {
                // H2 step 5 (DEPS_INVENTORY): the declared return type's
                // dep list is DEF-space — attr indices from `ref_return`,
                // plus tagged callee-frame notes (a returned fn-ref's
                // closure work var, `Deps::callee_frame1`).  Decode per
                // entry; the historical positional guess (in-range = attr
                // index, out-of-range = frame var) is retired.
                let def = data.def(self.d_nr);
                let ret_borrows_v = def.returned.deps_ref().is_some_and(|deps| {
                    deps.entries().any(|e| match e {
                        crate::data::DepEntry::Attr(a) => {
                            (a as usize) < def.attributes.len()
                                && function.var(&def.attributes[a as usize].name) == v
                        }
                        crate::data::DepEntry::CalleeFrame(w) => w == v,
                    })
                });
                // @PLN85 A.1 part i — `v` is the dep BUFFER of a borrowing
                // return-source terminal (`_vec_N["__vdb_N"]`, where `_vec_N` is
                // in `return_sources` and its dep names `v`): the buffer's store
                // is what the caller adopts, so suppress it here.  Path-local for
                // the same reason as the directly-owned case above.
                let backs_return_source = return_sources
                    .iter()
                    .any(|&src| src != v && function.tp(src).depend().contains(&v));
                let in_ret = ret_borrows_v
                    || backs_return_source
                    || ret_var != u16::MAX && function.tp(ret_var).depend().contains(&v);
                // H2 step 5 (DEPS_INVENTORY): the BLOCK-RESULT type's deps were
                // read here for years under the positional guess.  That read is
                // RETIRED: the declared-return (`ret_borrows_v`, a TYPED decode),
                // returned-var, and return-source-backing checks above decide the
                // suppression.  A debug sentinel used to scream when the old read
                // would have "decided alone" (`tp.depend()` names `v` while
                // `in_ret` is false), on the theory that such a case would need
                // the read re-added.  It does NOT: every firing is a FALSE positive
                // of the retired POSITIONAL decode — a field / enum-field / match-
                // arm return that COPIES its source into the caller's retbuf
                // (`return fv_c.pts`, `match e { Filled{items} => items }`), so the
                // local source `v` is correctly freed at scope exit AFTER the copy.
                // Re-adding the read would instead SUPPRESS that free and LEAK the
                // source.  Verified on the seven firing cases (450, 508, repro_p365,
                // four 85-store-lifetime-*) — value + leak + LOFT_POISON + the DA
                // store-free asserts all clean, both backends — so the read stays
                // retired and the sentinel is removed (the reliable checks subsume
                // every TRUE return source; the positional read only added noise).
                // Work-refs (`__ref_N` / `__rref_N`) carry their own var
                // in the dep list (`src/parser/mod.rs:1924-1928`) so the
                // standard `dep.is_empty()` gate skips them.  But work-
                // refs allocated to back ref-returning calls accumulate
                // unfreed stores when:
                //   - `gen_set_first_ref_call_copy`'s `0x8000` doesn't
                //     fire (e.g. when the callee MIGHT return a DbRef
                //     aliasing one of its args), or
                //   - the call-site reuses the same work-ref slot across
                //     loop iterations and `OpDatabase`'s `clear+claim`
                //     leaves the store marked `free` from the previous
                //     iteration even while live data lives in it.
                // Free them explicitly at function exit so the leak-check
                // at `src/state/debug.rs:1045` doesn't trip.  Skip when
                // the work-ref participates in the return chain.
                let is_work_ref = {
                    let n = function.name(v);
                    n.starts_with("__ref_") || n.starts_with("__rref_")
                };
                // @P302 — a keyed-collection local backed by its OWN store
                // carries a self-dep `[v]` (added by the `s = []` clear path
                // so a later `s += …` re-inits in place).  That self-dep is an
                // ownership marker, not a borrow — treat it like `dep.is_empty()`
                // so the store is freed at scope exit.  Mirrors the fn-ref
                // ownership rule below.  Keyed-only + exact self-dep; `in_ret`
                // still suppresses returned keyed locals.
                // D-own-16 residual — a nullable heap local BOUND FROM A PARAMETER and later
                // reassigned from a minting call owns its store on some paths and borrows on
                // others (`d: S? = p; if c { d = mint(d) }`), and the dep list is
                // flow-INsensitive so it reports the borrow forever: the scope-exit free is
                // suppressed and every mint leaks.  @FR-O-Latest is a per-RUN fact, and here it
                // is decidable at runtime WITHOUT a witness slot, because the dep NAMES the
                // variable this local might still be aliasing — distinct stores mean the local
                // minted its own.  A static strip cannot do this: on the not-taken branch the
                // local still holds the caller's store and freeing it is a use-after-free two
                // frames up, which is why `displaced_owned_slots` excludes arguments.
                //
                // Restricted to an ARGUMENT dep on purpose.  A parameter's slot is stable for
                // the frame (or has an entry stash, below); an arbitrary local dep can itself be
                // freed or reassigned before this scope ends, and then the comparison names a
                // store that is already gone.
                // The one-argument borrow is `Function::borrows_one_argument`, the spelling both
                // backends' displacement frees read (@FR-O-NoDiverge); this sweep asks it of a
                // RECORD local only, and never of a never-free one (@FR-O-Override).
                let borrow_witness = if function.borrows_one_argument(v)
                    && matches!(
                        function.tp(v).base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                    && !function.is_skip_free(v)
                {
                    // A REBINDABLE parameter's slot stops naming the caller's store once it is
                    // rebound, so compare against the @PLN87 entry stash that still does.
                    Some(function.rebind_orig(dep[0]).unwrap_or(dep[0]))
                } else {
                    None
                };
                let owns = dep.is_empty()
                    || self.lift_join_witness.contains_key(&v)
                    || borrow_witness.is_some()
                    || (dep.len() == 1
                        && dep[0] == v
                        && crate::parser::vectors::is_keyed(function.tp(v)));
                // Plan-57 Phase B (Mechanism B), widened by #323: a
                // Reference-typed capture — a boxed `__cell_<T>` AND, per
                // P260's storage rule, any plain struct capture — is OWNED
                // by the closure record, not the defining frame.  The
                // record stores the capture's 12-byte DbRef and
                // `free_named`'s cascade (allocation.rs) walks every DbRef
                // field when the record dies, so the defining-frame
                // `OpFreeRef` here is redundant — and actively harmful: for
                // an ESCAPED closure (factory return) it frees a slot the
                // live closure still references, the next allocation reuses
                // it, and the closure silently corrupts the new occupant
                // (#323; interp only *appeared* sound through slot-reuse
                // luck).  Suppress it; the cascade is the sole owner for
                // escaping AND in-frame captures (in-frame: the fn-ref's
                // own scope-exit free triggers the cascade).
                //
                // The handover is only sound where this free EXISTS to be
                // suppressed — `owns` — so the record's cascade must reach
                // no further.  #682 is what happens when it does: a captured
                // PARAMETER never enters this sweep (see `variables()`: "never
                // return function arguments") and a projection local is
                // `owns == false`, yet the cascade freed both, destroying the
                // caller's store.  `mark_borrowed_captures` recomputes this same
                // verdict once every function is scanned and marks those captures
                // borrowed on the record, which is what stops the cascade.
                //
                // `capture_adoption_owns_free` is where the rule itself lives — this is
                // its first consumer, `check_ref_leaks` its static mirror, and
                // `ownership_cfg`'s leak oracle the third.  It is the SIXTH site in the
                // drifted-list family the loft#1150 note above enumerates (`is_dbref` here
                // and at D-own-13, `deps_mut`, `is_keyed`, `depend`), which is why it is a
                // call and not a `matches!` written out again.
                let captured_ref =
                    capture_adoption_owns_free(data, function, &self.capture_build_backing, v);
                // @PLN94 TEST-ONLY over-free injection (never set in production): force the scope-exit
                // free of a NAMED borrowed var (owns=false) so the over-free check has a firing
                // true-positive. Subject to the same !in_ret/!skip_free/!captured guards as a real free.
                let inject_free = inject_free_borrowed() == Some(function.name(v));
                let emit = (owns || is_work_ref || inject_free)
                    && !in_ret
                    && !function.is_skip_free(v)
                    && !self.free_transferred.contains(&v)
                    && !captured_ref;
                if function.is_skip_free(v) && inject_free_skipfree() == Some(function.name(v)) {
                    ls.push(Value::Call(
                        data.def_nr("OpFreeRefIfDistinct"),
                        vec![Value::Var(v), Value::Var(v)],
                    ));
                }
                if scope_debug && !emit {
                    eprintln!(
                        "[scope_debug] NOT freeing '{}' (var={v}, scope={}, to_scope={to_scope}): \
                         dep_empty={} in_ret={in_ret} skip_free={}",
                        function.name(v),
                        self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        dep.is_empty(),
                        function.is_skip_free(v),
                    );
                }
                if emit {
                    if scope_debug {
                        eprintln!(
                            "[scope_debug] freeing '{}' (var={v}, scope={})",
                            function.name(v),
                            self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        );
                    }
                    // when `v` is a `__ref_*` / `__rref_*` work-ref
                    // that was passed to a user-fn call whose Reference
                    // result lives on as `witness`, emit the runtime-
                    // conditional `OpFreeRefIfDistinct(v, witness)` — it
                    // is a no-op in the adoption case (v and witness
                    // share a store) and a real free in the fresh-store
                    // case (distinct stores, placeholder orphaned).
                    // Falls through to plain `OpFreeRef` when no pairing
                    // was recorded.
                    // @PLN125 arc B — the scope-end hook fires where the BINDING's life
                    // ends, which is not always where the STORE is released.  A value
                    // delivered through a caller-side return buffer has two variables
                    // naming one record: the buffer (`__ref_N`, function-scoped and
                    // reused) and the witness the author actually bound.  The author's
                    // binding is the one that ends — per loop iteration, at its own
                    // scope — so the witness drops and the buffer never does.  Getting
                    // that backwards is a rollback that runs once for a loop that opened
                    // a transaction on every pass.
                    let is_buffer = is_work_ref && self.paired_witness.contains_key(&v)
                        || self.witness_buffer.values().any(|bs| bs.contains(&v));
                    if let Some(&jw) = self.lift_join_witness.get(&v) {
                        // loft#1257 — free the lifted collection return only where it is NOT
                        // the caller's own store.  A `Join` is owned on one arm and a borrow
                        // on the other and they are the SAME call, so nothing static separates
                        // them; the store number does.
                        if let Some(hook) = self.scope_end_drop(function, v, data) {
                            ls.push(hook);
                        }
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(jw)],
                        ));
                    } else if is_work_ref && let Some(&witness) = self.paired_witness.get(&v) {
                        // `OpFreeRefIfDistinct` declines where the two alias, so it is
                        // sound only where somebody ELSE releases the store then: the
                        // witness itself (a local that adopted the buffer and frees at its
                        // own exit), or the CALLER, when the witness — or a binding that
                        // borrows it — is what this return hands out.  A witness that never
                        // frees (a `??` hoist that binds the call for its block) and is not
                        // handed out leaves this free as the store's sole release, so it is
                        // plain: `r = mk(i) ?? d` in a loop held the buffer's last record for
                        // the frame's life once `r` stopped owning what it only borrowed.
                        let handed_out = witness == ret_var
                            || return_sources.iter().any(|&s| {
                                s == witness || function.tp(s).depend().contains(&witness)
                            });
                        if !function.is_skip_free(witness) || handed_out {
                            ls.push(Value::Call(
                                data.def_nr("OpFreeRefIfDistinct"),
                                vec![Value::Var(v), Value::Var(witness)],
                            ));
                        } else {
                            ls.push(call("OpFreeRef", v, data));
                        }
                    } else if is_work_ref
                        && let Some(&witness) = self.literal_buffer.get(&v)
                        && (witness == ret_var || return_sources.contains(&witness))
                    {
                        // loft#1317 — the buffer an inline record literal minted, whose store
                        // the local it was aliased into is now HANDING TO THE CALLER.  This
                        // free is forced (`is_work_ref`) and so ran even though `in_ret`
                        // suppressed the local's own: `fn f() -> S? { c: S? = S { x: 5 }; c }`
                        // returned a released store on both backends, right by luck on an
                        // ordinary build and `0xDEADBEEF` under `LOFT_POISON=1`.
                        //
                        // Conditional on the local being a RETURN source, and that condition is
                        // the whole of the rule.  Where the local is NOT returned, this plain
                        // free is the store's only release — the local may be captured (the
                        // record adopted it and the frame emits nothing), or carry no free of
                        // its own at all — and making it conditional strands the store.
                        // Measured both ways: `1181-a-captured-struct-…` leaks a `Circle` and
                        // `810-method-return-buffer` an `M` when this arm fires unconditionally.
                        //
                        // `OpFreeRefIfDistinct` then answers the two return shapes at run time:
                        // the local still names the buffer's store (alias -> decline, the caller
                        // owns it), or it was reassigned since (differ -> free, the literal
                        // store is dead).
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(witness)],
                        ));
                    } else if let Some(buffers) = self.witness_buffer.get(&v).cloned() {
                        // @P378(a) — `v` is an inner-scoped witness whose store
                        // is the outer `__ref_N` buffer (adoption).  Skip the
                        // per-iteration free when they still alias so the
                        // buffer stays reserved across iterations; the buffer's
                        // own function-exit OpFreeRef releases it once.
                        //
                        // The FREE is skipped in the adoption case; the DROP is not.
                        // The store surviving into the next iteration is a reuse
                        // optimisation, and the value it held is over either way.
                        if let Some(hook) = self.scope_end_drop(function, v, data) {
                            ls.push(hook);
                        }
                        // Several buffers — one per arm of the value branch `v` was bound
                        // from — free only where `v` aliases NONE of them; the single-buffer
                        // case is the one op.
                        if let [buffer] = buffers[..] {
                            ls.push(Value::Call(
                                data.def_nr("OpFreeRefIfDistinct"),
                                vec![Value::Var(v), Value::Var(buffer)],
                            ));
                        } else {
                            let mut free = call("OpFreeRef", v, data);
                            for &buffer in buffers.iter().rev() {
                                free = v_if(
                                    Value::Call(
                                        data.def_nr("OpDistinctStore"),
                                        vec![Value::Var(v), Value::Var(buffer)],
                                    ),
                                    free,
                                    Value::Null,
                                );
                            }
                            ls.push(free);
                        }
                    } else if let Some(w) = borrow_witness {
                        // Free ONLY when the local no longer names what its dep names.
                        if let Some(hook) = self.scope_end_drop(function, v, data) {
                            ls.push(hook);
                        }
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(w)],
                        ));
                    } else if inject_drop_free() == Some(function.name(v)) {
                        // @PLN94 TEST-ONLY positive control (never set in production; one cached env
                        // read/process): drop the scope-exit free for the NAMED owned var, injecting
                        // a genuine leak. The `check-leak` scan must go RED on it — the true-positive
                        // gate. Mirrors LOFT_NO_A1B / LOFT_STORE_GUARD_INJECT.
                    } else {
                        // The type's scope-end hook, immediately BEFORE the free that ends
                        // the value's life — unless `v` is a buffer whose witness already
                        // ran it.
                        if !is_buffer
                            && let Some(hook) = self.scope_end_drop(function, v, data)
                        {
                            ls.push(hook);
                        }
                        ls.push(call("OpFreeRef", v, data));
                    }
                }
            }
            // A generator HANDLE owns its coroutine frame, and through it every heap local
            // the generator body allocated.  `Type::Iterator` was absent from the heap block
            // above, so no scope carried a free for one: a generator whose consumer stopped
            // early never reached the tail where its own `OpFreeRef`s live, and the vector it
            // was walking stayed allocated for the rest of the program (loft#835).  Stopping
            // early is ordinary code — iterating until a match is found and breaking is the
            // main reason to reach for a generator at all.
            //
            // Ownership is unconditional here because a handle is never a view of somebody
            // else's store, so `Type::Iterator` carries no dep list to consult.  A handle
            // being RETURNED is already skipped by the `v == ret_var` test at the top of the
            // loop, and a parameter never enters this sweep, so the caller keeps its own.
            // Freeing an already-exhausted handle is safe: the frame carries a generation
            // stamp the free checks, so a stale handle cannot reach a recycled slot.
            if matches!(function.tp(v).base(), Type::Iterator(_, _)) && !function.is_skip_free(v) {
                ls.push(call("OpFreeRef", v, data));
            }
            // free the closure DbRef embedded at offset+4 in a fn-ref slot.
            // The 16-byte fn-ref stack slot is reclaimed by FreeStack, but the closure
            // store record at offset+4 must be explicitly freed via OpFreeRef.
            if let Type::Function(_, _, _) = function.tp(v) {
                // fn-ref variables OWN their closure store. The dep list
                // tracks captured variables, not store borrowing. Always
                // emit OpFreeRef unless the fn-ref is the return value.
                // H2 step 5: the declared return's closure-work-var note is
                // a TAGGED callee-frame entry — decode it (a raw `contains`
                // never matches the tagged value, and an explicit
                // `return adder;` reaches here with an empty block-result
                // dep list, so the closure record would be freed under the
                // escaping fn-ref).
                let ret_carries = data.def(self.d_nr).returned.deps_ref().is_some_and(|d| {
                    d.entries()
                        .any(|e| matches!(e, crate::data::DepEntry::CalleeFrame(w) if w == v))
                });
                let in_ret = tp.depend().contains(&v) || ret_carries;
                // The free above is what TRIGGERS the capture cascade (see the
                // `captured_ref` note earlier in this function: the frame's own free of a
                // captured cell is suppressed, so the record's cascade is its sole
                // owner).  That makes this fn-ref's scope the lifetime of everything it
                // captured — which is only right while the fn-ref is not scoped TIGHTER
                // than the record it points at.
                //
                // A closure written in a nested block is exactly that: the binding lives
                // in the block, the `___clos_N` record and the `__bx_<n>` cell are frame
                // vars one scope out.  Freeing through the binding at the block's end
                // then cascaded into a cell the frame still reads —
                // `fn f(n) { { b = fn(k) { n = n + k }; b(1); } n + 4 }` read
                // 0xDEADBEEF under LOFT_POISON, and plausible-looking stale bytes
                // without it.  Both backends, every scalar type, and a captured LOCAL as
                // readily as a parameter.
                //
                // The record carries its own scope-exit free (it is a plain OWNS frame
                // var, not a capture, so nothing suppresses it), and that one runs at the
                // right time.  So when the record sits in a different scope, leave the
                // cascade to it.  A fn-ref with no local record — a parameter, a call
                // result — keeps its free: nothing else would ever release it.
                let record_outlives = function.tp(v).depend().iter().any(|&r| {
                    r != v
                        && (r as usize) < function.count() as usize
                        && self.var_scope.get(&r) != self.var_scope.get(&v)
                });
                let emit = !in_ret && !function.is_skip_free(v) && !record_outlives;
                if emit {
                    if scope_debug {
                        eprintln!(
                            "[scope_debug] freeing closure of fn-ref '{}' (var={v}, scope={})",
                            function.name(v),
                            self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        );
                    }
                    ls.push(call("OpFreeRef", v, data));
                }
            }
        }
        // @P376 follow-up — no const-param unlock emitted anymore (matching
        // the dropped function-entry lock in `parser/expressions.rs`).  See
        // there for the rationale: compile-time const checks already cover
        // every mutation path, and the function-entry lock was a
        // false-positive trigger on iteration over a const-param's hash
        // field.  Par-worker `read_only` clones still enforce immutability
        // independently.
        // scope_debug: also report Reference vars in var_order whose scope is NOT in
        // the current chain — these are "orphaned" vars that should never happen after
        // the A5.6 block-pre-registration fix.
        if scope_debug {
            let chain: HashSet<u16> = {
                let mut s = HashSet::new();
                let mut sc = self.scope;
                let mut pos = self.stack.len();
                loop {
                    if sc == 0 {
                        break;
                    }
                    s.insert(sc);
                    if sc == to_scope {
                        break;
                    }
                    if pos == 0 {
                        break;
                    }
                    pos -= 1;
                    sc = self.stack[pos];
                }
                s
            };
            for &v in &self.var_order {
                if v == ret_var {
                    continue;
                }
                let v_scope = *self.var_scope.get(&v).unwrap_or(&0);
                if v_scope == 0 {
                    continue;
                }
                if !chain.contains(&v_scope)
                    && function.tp(v).is_heap_owned()
                    && !function.is_skip_free(v)
                {
                    eprintln!(
                        "[scope_debug] ORPHANED heap var '{}' (var={v}): \
                         its scope={v_scope} is not in the chain to to_scope={to_scope}",
                        function.name(v),
                    );
                }
            }
        }
        // @PLN87 P2.1 — function exit (`to_scope == 1` is the body scope; every
        // `return` and the tail both free up to it).  For each rebindable heap
        // param, free its CURRENT store iff it differs from the caller-supplied
        // original (witness) captured at entry: a wholesale-reassigned param
        // points at a callee-owned FRESH store (freed here), while an
        // un-reassigned or field-only-mutated param still equals its witness
        // (`OpFreeRefIfDistinct` no-ops, the caller owns + frees it).  The
        // runtime distinctness check makes this sound across conditional and
        // repeated rebinds.  Arguments are deliberately excluded from the normal
        // `variables()` sweep ("never return function arguments"), so this is the
        // sole site that frees a rebound param.  Loop scopes / nested blocks use
        // `to_scope >= 2`, so a `break`/`continue`/block-exit never fires this.
        if to_scope == 1 {
            let free_distinct = data.def_nr("OpFreeRefIfDistinct");
            for (param, orig) in function.rebind_params() {
                ls.push(Value::Call(
                    free_distinct,
                    vec![Value::Var(param), Value::Var(orig)],
                ));
            }
        }
        ls
    }

    /// loft#1156 — the locals a LOOP first assigns that something AFTER it READS.
    ///
    /// A body local is scoped to the body block, so `get_free_vars` releases its store at the
    /// end of each iteration.  A read after the loop is then a use-after-free — silent, and
    /// invisible to `LOFT_STRICT_STORES=1` because the slot really is free by then; what the
    /// read returns is whatever the allocator handed that slot next.  `--native` refuses the
    /// program instead (`E0425`), which is the same decision made visible: the free analysis
    /// already put the local's death at the block's end and native additionally scopes the
    /// Rust `let` there.  One decision, expressed twice, half of it visible.
    ///
    /// ⚠ **Only a local READ AFTER the loop is taken**, and the exclusions are what keep this
    /// from re-opening loft#1135.  A loop's own VARIABLE is read after the loop routinely
    /// (`LOFT.md` documents it) and must NOT be hoisted: its header assigns it
    /// unconditionally every iteration, and reserving a slot for it at the enclosing scope
    /// registers it in a scope it does not live in — one orphaned store per program.
    /// `was_loop_var` is the declared home for that question.  A local used INSIDE the loop
    /// alone is correctly per-iteration and is left exactly as it is.
    fn loop_locals_read_after(
        &self,
        op: &Value,
        rest: &[Value],
        function: &Function,
        out: &mut Vec<u16>,
    ) {
        if rest.is_empty() || !contains_loop(op) {
            return;
        }
        let mut assigned: Vec<u16> = Vec::new();
        collect_loop_body_sets(op, &self.var_mapping, &mut assigned);
        for v in assigned {
            if self.var_scope.contains_key(&v)
                || out.contains(&v)
                || function.was_loop_var(v)
                || !needs_pre_init(function.tp(v))
            {
                continue;
            }
            // Same guard as `find_first_ref_vars`: a BORROWED type may only be pre-inited
            // once every dep is in scope, or the `OpCreateStack` emitted here reads an
            // uninitialised slot.
            if !function
                .tp(v)
                .depend()
                .iter()
                .all(|d| self.var_scope.contains_key(d))
            {
                continue;
            }
            // LIVE after the loop, not merely mentioned: a later region that assigns `v`
            // before reading it is an independent binding sharing the name, and hoisting
            // those together gives one function-scope variable whose ownership fact is the
            // JOIN of both — which frees a borrow as if it were owned (loft#1332).
            if first_use_in_seq(rest, v) == Some(FirstUse::Read) {
                out.push(v);
            }
        }
    }

    /// Recursively collect variables that need a pre-init `Set(v, Null)` before an if/else.
    ///
    /// A variable is collected when it:
    /// - appears as the target of `Value::Set(v, ...)`,
    /// - has not yet been assigned (`var_scope` does not contain it), and
    /// - owns its allocation (`needs_pre_init` returns true).
    ///
    /// Recurses into nested `If` and `Block` but NOT into `Loop` — loop variables have
    /// per-iteration scope management and must not be pre-inited at the enclosing scope.
    fn find_first_ref_vars(&self, val: &Value, function: &Function, result: &mut Vec<u16>) {
        // Peeled for the same reason as its sibling [`Self::find_assigned_vars`], which
        // `scan_if` calls two lines below this one for the same job.  The arms discriminate
        // on `Set` / `Block` / `If` / `Insert`, so a spanned one took `_ => {}` — contributing
        // nothing for that whole subtree, where the shape being looked for is a branch's
        // FIRST assignment of a Reference/Vector/Text, i.e. the thing that decides pre-init.
        //
        // Reachable, and measured latent.  Over the 858-program corpus the peel changes the
        // decision at 46 sites in 16 programs, and at every one of them it newly
        // pre-initialises **0** variables: the same variables were already reaching `result`
        // by another path.  So no emitted code moves today.  The peel stays because it is one
        // word and it obeys `Value::unspan`'s documented rule, and because the reachability is
        // what makes it a trap — span placement has moved before, and the failure mode here is
        // a missing initialisation with nothing to report it.  Claiming a defect was fixed
        // would be the dressed-up version of this result.
        match val.unspan() {
            Value::Set(v, _) => {
                let resolved = *self.var_mapping.get(v).unwrap_or(v);
                // For borrowed types (non-empty dep), only pre-init if every dep is already
                // in var_scope — otherwise the OpCreateStack emitted at pre-init time would
                // reference an uninitialised slot.
                let deps_ready = function
                    .tp(resolved)
                    .depend()
                    .iter()
                    .all(|d| self.var_scope.contains_key(d));
                if !self.var_scope.contains_key(&resolved)
                    && needs_pre_init(function.tp(resolved))
                    && deps_ready
                    && !result.contains(&resolved)
                {
                    result.push(resolved);
                }
            }
            Value::Block(bl) => {
                for op in &bl.operators {
                    self.find_first_ref_vars(op, function, result);
                }
            }
            Value::If(_, t, f) => {
                self.find_first_ref_vars(t, function, result);
                self.find_first_ref_vars(f, function, result);
            }
            Value::Insert(ops) => {
                for op in ops {
                    self.find_first_ref_vars(op, function, result);
                }
            }
            // Do NOT recurse into Value::Loop — loop-interior Reference
            // variables are handled by the Loop handler in scan() which
            // pre-inits them at the pre-loop scope.
            _ => {}
        }
    }

    /// Scan a list of call arguments.
    ///
    /// If any scanned arg comes back as `Insert([Set(w, Null), body])` where `w` is an
    /// owned Reference (dep empty) — a hoisted closure-record allocation — the `Set(w,
    /// Null)` is lifted out into `preamble` and the arg is replaced with `body` alone.
    ///
    /// This prevents `ConvRefFromNull` (12 B) from landing on the eval stack *between*
    /// other call arguments, which would corrupt the `CallRef` argument layout and cause
    /// the lambda to receive garbage for `y` (A5.6 "Incorrect store" bug).
    ///
    /// Returns `(preamble, scanned_args)`.  The caller wraps the result as
    /// `Insert([preamble..., Call/CallRef(...)])` when the preamble is non-empty;
    /// `convert` flattens this so the preamble executes before any args are pushed.
    fn scan_args(
        &mut self,
        args: &[Value],
        function: &mut Function,
        data: &Data,
        outer_call: u32,
    ) -> (Vec<Value>, Vec<Value>, Vec<Value>) {
        let mut preamble: Vec<Value> = Vec::new();
        let mut ls: Vec<Value> = Vec::new();
        // @PLN90 / loft#506 — POST-call store-backs for computed-lvalue `&`-write-back args.
        let mut postamble: Vec<Value> = Vec::new();
        // loft#1287 — the rebind witnesses to mark protected-from-free for THIS call.
        let mut amp_foreign: Vec<u16> = Vec::new();
        // #248 (interpreter arg-layout) — when the call's first argument is a
        // borrowed receiver pushed via `OpCreateStack(Var(_))` (a `&self` / `&T`
        // method or free-function call), a LATER argument that is an inline
        // heap-returning call which grows the eval frame (e.g. `tick(s, mk(), …)`
        // where `mk()` returns a vector via a hidden `__ref_N` work buffer)
        // shifts the receiver's stack slot relative to where the callee reads
        // `self`.  The interpreter then derefs the receiver at the wrong offset
        // and lands on a CONST_STORE record → "Write to read-only store".  The
        // native backend passes args by the Rust ABI and is immune.  Force such a
        // trailing call argument into a `__lift_N` temp (preamble), so it is
        // evaluated and its store materialised BEFORE the receiver `OpCreateStack`
        // is pushed — exactly the shape `x = mk(); tick(s, x, …)` that already
        // works on both backends.  Gated on a CreateStack-receiver first arg so
        // ordinary calls keep their existing argument lowering untouched.
        let create_stack_nr = data.def_nr("OpCreateStack");
        // @PLN139 stage C — the lift-site half of [`collect_drop_transferred`].  A copy that
        // hands its source off — the `0x8000` move into a collection element, or a copy
        // whose destination is a container FIELD — must not leave the source dropping what
        // it no longer owns.  When that source is an inline call it is lifted into a
        // `__lift_N` temp right here, so this is the only point that knows which temp the
        // copy took: before the lift the IR still names the CALL, and after it nothing
        // records the pairing.  The copy's source is arg 0.
        let moved_arg = moved_source_arg(outer_call, args, data);
        let transfer_copy = outer_call == data.def_nr("OpCopyRecord") && {
            let moved =
                matches!(args.get(2).map(Value::unspan), Some(Value::Int(tp)) if tp & 0x8000 != 0);
            moved
                || args
                    .get(1)
                    .is_some_and(|d| copy_hands_off(d, function, data))
        };
        let has_create_stack_receiver = args.first().is_some_and(|a| {
            matches!(a.unspan(), Value::Call(d, cargs)
                if *d == create_stack_nr
                    && matches!(cargs.first().map(Value::unspan), Some(Value::Var(_))))
        });
        for (arg_idx, a) in args.iter().enumerate() {
            // loft#1320 — a branch VALUE consumed by this call (`total(if c { g(a) } else { g(n) })`)
            // never reaches `scan_set`, so its arms get their owners here, homed in this statement's
            // scope: the call reads the temp and the scope's exit frees it by identity.
            let rewritten_branch_arg;
            let a: &Value = if Self::is_value_branch(a)
                && self.arm_tails_need_binding(a, u16::MAX, data, function)
            {
                let mut rw = a.clone();
                let home = self.scope;
                let _ = self.lift_join_arm_tails(&mut rw, home, u16::MAX, function, data);
                rewritten_branch_arg = rw;
                &rewritten_branch_arg
            } else {
                a
            };
            let scanned = self.scan(a, function, data);
            // #248 — force-lift a trailing inline heap-returning call argument
            // (one NOT already lifted by the `inline_struct_return` arms below
            // because it returns via a hidden work-ref / non-empty dep) when the
            // receiver is a borrowed CreateStack ref.  Must run before the
            // Insert/`inline_struct_return` handling so it is not skipped for the
            // exact shape that triggers the bug.
            if has_create_stack_receiver
                && arg_idx > 0
                && self
                    .inline_struct_return(&scanned, data, outer_call, function)
                    .is_none()
                && let Some(tp) = Self::heap_call_return(&scanned, data)
            {
                let tmp = self.new_lift_var(function, &tp);
                // loft#735 — this lift is an ORDERING device, never an ownership
                // transfer.  It fires exactly where `inline_struct_return` said "do
                // NOT lift into an owned temp": the callee delivers through the
                // caller's hidden work-ref (`__ref_N`), which the caller already
                // frees at function exit.  `heap_call_return` hands back the type
                // with `Deps::none()` (a caller temp cannot carry the callee's
                // DEF-space dep), so without this mark `get_free_vars` reads
                // `owns == true` and frees a store the caller still owns — the slot
                // is recycled under the live value and the next write lands in it.
                // `new_lift_var` already sets the allocate-time half of the fact
                // (`inline_ref` — borrow, don't allocate); this is the free-time
                // half.  The hand-correct source shape proves the target: binding
                // the same call to a named local yields `flat:vector<integer>
                // ["__ref_1"]` and NO `OpFreeRef`.
                function.set_skip_free(tmp);
                self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
                continue;
            }
            // A `Span` is source position, not structure. `parse_call` wraps a call
            // argument in one, so an argument this pass rewrote into a preamble-plus-value
            // sequence arrives as `Span(Insert(…))` and the bare-variant match below sees
            // nothing to split — which is how loft#1029's hoisted argument reached the
            // emitters still wrapped, with the lift that owns its result never firing.
            // Peel it for the Insert case only, so every other argument keeps its position.
            let scanned = match scanned {
                Value::Span(b) if matches!(b.1, Value::Insert(_)) => b.1,
                other => other,
            };
            // loft#1287 — a `&` argument whose binding this frame does NOT own.  The
            // callee's write-back releases the store the binding stopped naming, and it
            // cannot see whose that store is: for a plain heap PARAMETER it is the store the
            // CALLER handed down (`formal/calls.md` F-ParamHeap), owned a frame further up.
            // Freeing it there is a use-after-free plus a double free against the real
            // owner's own release.  The rebind witness names that store — the parameter's
            // ENTRY store, which is the only one this frame never owns, so a REPEATED call
            // still lets the callee release the fresh store the previous one installed.
            // `free_displaced` honours the mark; `(F-ParamRebind)`'s function-exit
            // `OpFreeRefIfDistinct(param, witness)` releases what the binding ends up naming.
            if outer_call != u32::MAX
                && let Value::Call(cs, cargs) = scanned.unspan()
                && *cs == create_stack_nr
                && let Some(Value::Var(v)) = cargs.first().map(Value::unspan)
                && let Some(orig) = function.rebind_orig(*v)
                && data
                    .def(outer_call)
                    .attributes()
                    .get(arg_idx)
                    .is_some_and(|at| at.typedef.is_amp_rebindable_heap())
            {
                amp_foreign.push(orig);
            }
            if let Value::Insert(ops) = scanned {
                // Existing A5.6 hoisting: lift Set(w, Null) for owned Reference.
                let is_a56_hoisted = Self::is_null_init_preamble(&ops, function);
                // hoist Set(__lift_N, ...) preamble from nested scan_args.
                // These are produced when an inner call's arguments contained
                // inline struct-returning calls that were already lifted.
                let n = ops.len();
                let is_p135_hoisted = n >= 2
                    && ops[..n - 1].iter().all(|v| {
                        matches!(v, Value::Set(v_nr, _) if function.name(*v_nr).starts_with("__lift_"))
                    });
                // hoist Set(__ref_N, expr) preamble produced by the
                // parser's `&T`-conversion path for non-Var sources.  The
                // final op is always OpCreateStack(Var(__ref_N)); after
                // hoisting it stays as the arg value, while the Set moves
                // into the enclosing statement list so the work-ref lives
                // at function scope (its slot must survive the call).
                //
                // loft#745 — a work-ref that ALREADY holds a value carries its
                // overwrite-free, so the parser's `Set` arrives wrapped as
                // `Insert([OpFreeRef(__ref_N), Set(__ref_N, …)])`.  Matching only the
                // bare `Set` left that shape unhoisted, so the materialisation stayed
                // INSIDE the argument: native then had no `OpCreateStack(Var(_))` to
                // recognise, hoisted the whole argument into a `let _pre_N = …`
                // binding whose value is the assignment's `()`, and rustc rejected the
                // call with E0308 (expected `&mut DbRef`, found `()`).
                let is_p179_hoisted = n >= 2
                    && ops[..n - 1]
                        .iter()
                        .all(|v| Self::is_ref_materialisation(v, function, data))
                    && matches!(&ops[n - 1], Value::Call(d_nr, _)
                        if data.def(*d_nr).name == "OpCreateStack");
                if is_a56_hoisted || is_p135_hoisted || is_p179_hoisted {
                    // @PLN90 / loft#506 — a computed-lvalue `&`-WRITE-BACK arg.  Capture
                    // `items[i]` into a FRESH OWNED temp (so the callee's write-back frees the
                    // COPY, never the caller's element), pass the temp, then copy the result
                    // back into the element after the call.  The element's record is never
                    // freed — it is the stable backing.  A field-mutation callee is untouched.
                    if is_p179_hoisted
                        && let Some((pre, arg, post)) =
                            self.amp_writeback_owned_copy(&ops, arg_idx, outer_call, function, data)
                    {
                        preamble.extend(pre);
                        ls.push(arg);
                        postamble.push(post);
                        continue;
                    }
                    // loft#899 — hoisting the null-init out of the value block moves
                    // the temp's OWNER to the enclosing scope: its declaration now
                    // stands in that statement list, and an argument is only READ, so
                    // no binding adopts the store the way `v = <block>` does.  Nothing
                    // freed it, so every unbound `f#read(n) as vector<T>` leaked one
                    // store.  Re-register it at the current scope for `get_free_vars`,
                    // and run the same hand-off marking a lifted call-result gets so an
                    // argument the callee MOVES from does not drop twice.
                    let a56_owned = if is_a56_hoisted {
                        match &ops[0] {
                            Value::Set(v, _) => Some(*v),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let mut it = ops.into_iter();
                    for _ in 0..n - 1 {
                        preamble.push(it.next().unwrap());
                    }
                    if let Some(v) = a56_owned {
                        self.var_scope.insert(v, self.scope);
                        self.mark_lift_handoff(v, arg_idx, transfer_copy, moved_arg);
                    }
                    let final_val = it.next().unwrap();
                    // the remaining Call may also be struct-returning
                    // (e.g. normalize3(__lift_1) inside add_dir).  Lift it too.
                    if let Some(tp) =
                        self.inline_struct_return(&final_val, data, outer_call, function)
                    {
                        let tmp = self.new_lift_var(function, &tp);
                        self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                        preamble.push(v_set(tmp, final_val));
                        ls.push(Value::Var(tmp));
                    } else {
                        ls.push(final_val);
                    }
                } else if let Some(tp) = ops
                    .last()
                    .and_then(|last| self.inline_struct_return(last, data, outer_call, function))
                {
                    // loft#1029 — an `Insert` whose TAIL is a heap-returning call, which is
                    // what the argument hoist below produces one level down: the ops that
                    // build the inner call's argument, then the call.  The lift that gives
                    // such a result an OWNER matches a `Call`, so the wrapper hid it and the
                    // callee's store was orphaned — `print("{pick(S { a: 7 }, false).a}")`
                    // leaked one record per evaluation on both backends while the same call
                    // BOUND to a local was clean.
                    //
                    // That is @P297's pitfall exactly one wrapper later ("the argument
                    // reaching here is `Span(Call(…))`; unspan before matching or the lift
                    // never fires"), so the cure is the same shape: read through to the
                    // value.  The preamble ops move into the enclosing statement list, where
                    // they already ran, and only the call is lifted — the three recognisers
                    // above split the same way for their own shapes.
                    let mut it = ops.into_iter();
                    let call = it.next_back().expect("checked non-empty by `last`");
                    preamble.extend(it);
                    let tmp = self.new_lift_var(function, &tp);
                    self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                    preamble.push(v_set(tmp, call));
                    ls.push(Value::Var(tmp));
                } else {
                    ls.push(Value::Insert(ops));
                }
            } else if let Some(tp) = self.inline_struct_return(&scanned, data, outer_call, function)
            {
                // inline struct-returning or vector-returning call as argument
                // — lift to a temporary variable so get_free_vars emits
                // OpFreeRef at scope exit.  Without this, the callee's store
                // leaks every call.
                //
                // The argument becomes Set(tmp, call(...)) which the codegen
                // handles via gen_set_first_at_tos on first encounter and
                // generate_set (reassignment) on subsequent loop iterations.
                // get_free_vars emits OpFreeRef(tmp) at scope exit because
                // the dep is empty (owned).
                let tmp = self.new_lift_var(function, &tp);
                self.mark_lift_handoff(tmp, arg_idx, transfer_copy, moved_arg);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
            } else if let Some((w, absorb)) =
                self.inline_built_borrow_source(&scanned, outer_call, data)
            {
                // loft#1029 — an argument BUILT INLINE (`pick(S { a: 7 }, …)`) that the
                // callee's return may BORROW.  The @P290 bracket that decides borrow-vs-
                // owned at runtime can only name a bare `Var` slot, so an argument still
                // wrapped in its construction block leaves the witness set incomplete
                // (`use_analysis::protectable_ref_args`) and the caller keeps the
                // conservative answer: it COPIES the returned store and orphans the one
                // the callee minted — one leaked record per call, both backends.
                //
                // The slot already exists and this frame already frees it: the parser
                // builds the literal into a function-scope work-ref and the block's tail
                // IS that work-ref.  So nothing needs a new owner — only the CALL SITE
                // needs to be able to say its name.  Hoisting the construction into the
                // preamble and passing `Var(w)` makes the argument nameable, which is
                // exactly the hand-written spelling that was always clean
                // (`q = S { a: 7 }; pick(q, …)`), and the emitted code becomes identical
                // to it.
                //
                // Deliberately NOT done by widening `protectable_ref_args` to see through
                // the block: `protect_store_frees` reads the DbRef VALUE at call time and
                // the bracket is emitted BEFORE the arguments are evaluated, so a work-ref
                // still holding its null would be "protected" while empty — the witness
                // set would read complete while protecting nothing, and the source-free it
                // then licenses would release a store the caller still reaches.  That
                // trades this leak for a use-after-free.
                let Value::Block(bl) = scanned else {
                    unreachable!("inline_built_borrow_source matched a non-Block")
                };
                // The block's own scope disappears with the block, so every var it
                // declared is now declared in the statement list we hoisted into. Move
                // their `var_scope` entries with them: an entry left pointing at a scope
                // no emitted code opens would have slot assignment place it against a
                // sibling scope's zone. Only the VECTOR-literal shape reaches this — a
                // struct literal's tail is the function-scope work-ref itself.
                if let Some(block_scope) = absorb {
                    for sc in self.var_scope.values_mut() {
                        if *sc == block_scope {
                            *sc = self.scope;
                        }
                    }
                }
                let mut ops = bl.operators;
                ops.pop();
                preamble.extend(ops);
                ls.push(Value::Var(w));
            } else if let Some(tp) =
                Self::unnameable_borrow_source(&scanned, outer_call, arg_idx, data)
            {
                // loft#1105 — an argument the @P290 bracket cannot NAME, at a call whose return
                // may borrow it.  The bracket protects a store through a variable holding a
                // `DbRef`, and `view_root_slots` walks a bare `Var`, a projection chain and a
                // JOIN to find one.  A `??` in argument position lowers to an `ncc` BLOCK whose
                // tail is a join with a CALL arm, and neither the multi-statement block nor the
                // call is nameable — so the witness set read incomplete and the caller copied
                // the returned store, orphaning the one the callee minted.
                //
                // Binding it to a temp is the same cure the cases above take, generalised to the
                // question itself: if the bracket cannot name the value, give it a name.  The
                // preamble runs BEFORE the bracket is emitted, so the temp holds the real
                // `DbRef` by then — which is why the hand-written `e = v[0] ?? mk(); pick(e, …)`
                // was always clean and this now emits the same thing.  And because the name is
                // taken at RUNTIME, the bracket protects whichever store the value turned out to
                // be, which is what makes one temp serve a join whose arms disagree about it.
                //
                // ⚠ LAST in the chain, and that is load-bearing rather than tidy.  The temp
                // takes the CALLEE'S PARAMETER type — the one type available for a value with no
                // variable behind it — and a parameter declaration carries NO DEPS, so the temp
                // reads as an OWNER of whatever it holds.  For every shape the arms above claim
                // that is wrong: a tuple element and a projection chain are VIEWS of a store the
                // caller owns, and an owner's scope-exit free would release a record the caller
                // still reaches.  Ordered after them, this arm only ever sees values no earlier
                // arm could type.
                //
                // …and the temp BORROWS what the value borrows, which is the type
                // `unnameable_borrow_source` answers: the callee's parameter SHAPE carrying
                // `lift_view_deps`'s answer for the argument.  A `skip_free` temp would also
                // stop the over-free, and it says less — "do not free me" rather than "whose
                // store is this", which is the question `Type::depend`'s other readers ask.
                // Where the walk can name no source the argument is NOT bound at all, so a
                // value with no provenance costs the leak it already had rather than a name
                // that cannot say why it is safe.
                let tmp = self.new_lift_var(function, &tp);
                preamble.push(v_set(tmp, scanned));
                ls.push(Value::Var(tmp));
            } else {
                ls.push(scanned);
            }
        }
        for (i, orig) in amp_foreign.iter().enumerate() {
            preamble.insert(
                i,
                Value::Call(
                    data.def_nr("n_protect_store_frees"),
                    vec![Value::Var(*orig)],
                ),
            );
            postamble.push(Value::Call(
                data.def_nr("n_unprotect_store_frees"),
                vec![Value::Var(*orig)],
            ));
        }
        (preamble, ls, postamble)
    }

    /// loft#1105 — the TYPE to bind an argument to when the @P290 bracket cannot NAME the store
    /// its value will lie in, at a call whose return may borrow it.
    ///
    /// `Some(tp)` is the callee's PARAMETER shape — the type the argument is converted to
    /// regardless — carrying the DEPS the value itself borrows ([`lift_view_deps`]).
    ///
    /// The deps are the load-bearing half.  The parameter's declared type has none, and a
    /// temp typed that way reads as the OWNER of a store it only VIEWS: `get_free_vars`
    /// emits a scope-exit free that releases the caller's container.  It is silent while the
    /// container is a local of the same frame — the store was dying at that scope exit
    /// anyway — and a use-after-free the moment the container OUTLIVES the call, which is
    /// why `pick(h[k], …)` corrupted a `hash` passed in as a parameter.
    ///
    /// So a value whose source cannot be named is NOT bound (`lift_view_deps` answers
    /// `None`), and the argument stays exactly as it was — the leak that is already there,
    /// which is the better of the two.
    ///
    /// Gated as its two siblings are, plus one exclusion of its own: a bare `Var` is already
    /// nameable and must not be re-bound, and an argument the bracket CAN name needs nothing.
    /// The inline-construction and tuple-element cases are tried first and handle their shapes
    /// more precisely — a construction is HOISTED rather than bound, because binding a work-ref
    /// that still holds null at bracket-emit time would read as covered while protecting
    /// nothing (loft#981).
    fn unnameable_borrow_source(
        arg: &Value,
        outer_call: u32,
        arg_idx: usize,
        data: &Data,
    ) -> Option<Type> {
        if outer_call == u32::MAX {
            return None;
        }
        let callee = data.def(outer_call);
        if !callee.is_loft_defined() {
            return None;
        }
        if matches!(callee.returned().base(), Type::Function(_, _, _)) {
            return None;
        }
        if !callee.returns_borrowed_view() {
            return None;
        }
        if matches!(arg.unspan(), Value::Var(_)) {
            return None;
        }
        if crate::use_analysis::bracket_can_name(data, arg) {
            return None;
        }
        let tp = callee.attributes().get(arg_idx)?.typedef.clone();
        // Only an argument that CARRIES a store needs a witness at all — asked through
        // `base`, because a NULLABLE parameter (`s: S?`) is `Optional(Reference(S))` and
        // carries exactly the store its non-null twin does.  Asked on the raw type this
        // declined every nullable parameter, so a `??` argument at one was never lifted and
        // kept leaking the callee's minted store while the dense twin was cured.
        if !crate::data::is_dbref(tp.base()) {
            return None;
        }
        Some(tp.with_deps(&Deps::frame(lift_view_deps(arg, data)?)))
    }

    /// loft#1029 — the work-ref an INLINE-built argument yields, when the callee's return
    /// may borrow it.
    ///
    /// `Some(w)` for a value block that fills a work-ref and ends in it — the shape the
    /// parser gives `S { … }` / a collection literal in argument position — at a call whose
    /// return names a visible parameter (`returns_borrowed_view`). `w` is a function-scope
    /// slot this frame already allocates and frees, so hoisting the block moves nothing's
    /// ownership; it only lets the call site NAME the borrow source.
    ///
    /// Gated on `returns_borrowed_view` on purpose. Every other call is already correct as
    /// it stands, and hoisting an argument reorders it relative to its left-hand siblings —
    /// a cost worth paying only where the alternative is a leak.
    fn inline_built_borrow_source(
        &self,
        arg: &Value,
        outer_call: u32,
        data: &Data,
    ) -> Option<(u16, Option<u16>)> {
        // `scan_args` runs for argument lists with no enclosing DEF as well (the
        // `u32::MAX` no-call sentinel), and `Data::def` asserts on it. Nothing to decide
        // there: with no callee there is no return that could borrow the argument.
        if outer_call == u32::MAX {
            return None;
        }
        let callee = data.def(outer_call);
        // Only a call into a LOFT-DEFINED body, which is what the @P290 copy-or-adopt
        // bracket serves. A native accessor answers a borrowed view too (`OpGetText` is
        // `text[v1]`), but it never goes through that machinery, so hoisting its argument
        // buys nothing — and it is reached in EXPRESSION position, where the preamble is
        // not a statement list: the hoisted ops were rendered into the argument parens and
        // `--native` rejected the call with E0277 (`((), (), (), (), &str): AsRef<str>` in
        // 875-json-absent-text-field).
        if !callee.is_loft_defined() {
            return None;
        }
        // A callee returning a CLOSURE is not a heap return, and `returns_borrowed_view`
        // is documented as a heap-return ownership read: a `Type::Function` return carries
        // `CALLEE_FRAME`-tagged deps (a closure-internal frame var, never an attr index)
        // and its own debug assert says such a dep must not reach it.  This bracket serves
        // heap returns alone — the shape test below names them — so the ownership question
        // is asked only of a callee that has one.  Without the gate, `fn make_adder(b) ->
        // fn(integer) -> integer` tripped that assert before any of its own work ran.
        if matches!(callee.returned().base(), Type::Function(_, _, _)) {
            return None;
        }
        if !callee.returns_borrowed_view() {
            return None;
        }
        let Value::Block(bl) = arg else {
            return None;
        };
        // At least one construction op plus the trailing `Var` — a bare `{ v }` has
        // nothing to hoist and is already a nameable value.
        if bl.operators.len() < 2
            || !matches!(
                bl.result.base(),
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
        {
            return None;
        }
        let Value::Var(w) = bl.operators.last()?.unspan() else {
            return None;
        };
        // The block must MINT ITS STORE INTO A SLOT THAT OUTLIVES THE HOIST. Its result dep
        // names that slot — the owner — and the dep the block already carries is the fact
        // itself, so nothing here keeps a second list of it. (Not `is_work_ref`: that set
        // is the return-delivery materialiser's own register and does not contain the
        // parser's object work-ref — measured, it answers false for the `__ref_1` this very
        // shape builds into.)
        let [owner] = bl.result.depend()[..] else {
            return None;
        };
        // The owner's declaration must ENCLOSE the statement list we are hoisting into, so
        // the moved ops land inside its lifetime and nothing changes about who frees the
        // store. `self.stack` is the chain of scopes currently open, so this asks the real
        // question — a numeric `<=` would not, because scope numbers are allocated in
        // encounter order and an earlier SIBLING also compares less while enclosing nothing.
        if !self.scope_encloses(owner) {
            return None;
        }
        // The tail names either that same owner — `S { … }`, whose block fills `__ref_N`
        // and yields it — or a VIEW the block opened at its own scope: a vector literal
        // fills `__vdb_N` one level up and yields `_vec_N`, a view of it. The second is
        // still ownership-neutral, because the store's owner is `__vdb_N` and that is not
        // moving; what moves is the view's DECLARATION, out of a block that ceases to
        // exist. So the block's scope has to be absorbed into the one we hoist into, or
        // `var_scope` would keep pointing those vars at a scope no emitted code opens and
        // slot assignment would place them against a sibling's zone.
        if self.scope_encloses(*w) {
            Some((*w, None))
        } else if self.var_scope.get(w) == Some(&bl.scope) {
            Some((*w, Some(bl.scope)))
        } else {
            None
        }
    }

    /// Is `v` declared in a scope that is still OPEN — the one being scanned or one
    /// enclosing it?  That is the condition for moving code into the current statement
    /// list and still being inside `v`'s lifetime.
    fn scope_encloses(&self, v: u16) -> bool {
        self.var_scope
            .get(&v)
            .is_some_and(|sc| *sc == self.scope || self.stack.contains(sc))
    }

    /// The A5.6 hoistable preamble: `Insert([Set(v, Null), value])` whose `v` OWNS a
    /// heap store.  It is the shape the `Value::Block` arm returns for a value block
    /// that yields an owned temp — a `#reading file` read, a `??` join — and
    /// [`Self::scan_args`] lifts that `Set` into the enclosing statement list so the
    /// slot's `first_def` lives OUTSIDE the argument expression.
    ///
    /// One home for the question, because two places ask it: `scan_args`, which does
    /// the hoisting, and the `Value::Span` arm of [`Self::scan`], which must drop a
    /// span that would otherwise hide this Insert from `scan_args`' `if let
    /// Value::Insert`.  While only `scan_args` knew the shape, a span-wrapped one
    /// stayed inside the argument: `println("{len(f#read(8) as vector<single>)}")`
    /// left the temp's declaration in an expression slot, which native emitted
    /// literally as a `let` statement inside an argument list — rustc "expected
    /// expression, found `let` statement" — and which nothing then freed (loft#899).
    fn is_null_init_preamble(ops: &[Value], function: &Function) -> bool {
        ops.len() == 2
            && matches!(&ops[0], Value::Set(v, val)
                if matches!(val.as_ref(), Value::Null) && function.tp(*v).is_heap_owned())
    }

    /// True when `v` writes the work-ref that a `&`-argument's `OpCreateStack` then
    /// borrows — the `Set(__ref_N, …)` the parser emits for a non-`Var` `&`-source,
    /// either bare or wrapped with the overwrite-`OpFreeRef` a re-assigned work-ref
    /// carries (loft#745).  `scan_args` hoists these out of the argument so the
    /// work-ref lives at function scope and its slot survives the call.
    fn is_ref_materialisation(v: &Value, function: &Function, data: &Data) -> bool {
        let writes_work_ref = |op: &Value| matches!(op, Value::Set(v_nr, _) if function.name(*v_nr).starts_with("__ref_"));
        match v {
            Value::Set(_, _) => writes_work_ref(v),
            // The free targets the work-ref's PREVIOUS value and belongs with the write,
            // so the pair hoists as one unit.  A wrapper holding anything else is not a
            // materialisation and stays inside the argument.
            Value::Insert(inner) => {
                inner.iter().any(writes_work_ref)
                    && inner.iter().all(|op| {
                        writes_work_ref(op)
                            || matches!(op, Value::Call(d_nr, _)
                                if data.def(*d_nr).name == "OpFreeRef")
                    })
            }
            _ => false,
        }
    }

    /// @PLN90 / loft#506 — a computed-lvalue `&`-WRITE-BACK argument.  The arg is
    /// `Insert([Set(wv, orig), OpCreateStack(wv)])` where `orig` reads a heap lvalue
    /// (`OpGetVector`/`OpGetField`).  When the callee WHOLE-REASSIGNS this `&`-param (a
    /// `Set(param, non-Null)` through the ref — a write-back), the reassignment FREES the
    /// displaced record; if the arg aliased the caller's element that would free the
    /// element (corruption).  So capture `orig` into a FRESH OWNED temp: the callee frees
    /// the COPY, the element's record survives, and after the call the temp's new record is
    /// copied back into the element.  Returns `(preamble, arg, postamble)` — the owned-copy
    /// setup, the `OpCreateStack(tmp)` argument, and the copy-back — or `None` (field
    /// mutation / not a computed lvalue → keep the default aliasing lowering).
    fn amp_writeback_owned_copy(
        &mut self,
        ops: &[Value],
        arg_idx: usize,
        outer_call: u32,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Value, Value)> {
        if outer_call == u32::MAX || ops.len() != 2 {
            return None;
        }
        let Value::Set(wv, orig) = &ops[0] else {
            return None;
        };
        let wv = *wv;
        let orig = orig.unspan().clone();
        if !matches!(&orig, Value::Call(g, _)
            if matches!(data.def(*g).name(), "OpGetVector" | "OpVectorRef" | "OpGetField"))
        {
            return None;
        }
        // Does the callee whole-reassign the `&`-param at this position? (`Set(param, non-Null)`
        // through the ref — NOT `rebind_orig`, which is non-`&` reassignment-locality.)
        let attrs = data.def(outer_call).attributes();
        if arg_idx >= attrs.len() {
            return None;
        }
        let callee_vars = &data.definitions[outer_call as usize].variables;
        let param_var = callee_vars.var(&attrs[arg_idx].name);
        if param_var == u16::MAX || !matches!(callee_vars.tp(param_var), Type::RefVar(_)) {
            return None;
        }
        let mut writes_back = false;
        data.definitions[outer_call as usize]
            .code
            .walk(&mut |node| {
                if let Value::Set(v, rhs) = node
                    && *v == param_var
                    && !matches!(rhs.unspan(), Value::Null)
                {
                    writes_back = true;
                }
            });
        if !writes_back {
            return None;
        }
        let struct_d = match function.tp(wv) {
            Type::RefVar(inner) => match &**inner {
                Type::Reference(d, _) => *d,
                _ => return None,
            },
            Type::Reference(d, _) => *d,
            _ => return None,
        };
        let type_val = Value::Int(i32::from(data.def(struct_d).known_type()));
        let inner_tp = Type::Reference(struct_d, Deps::none());
        // A fresh OWNED temp via `new_lift_var` — registers the slot + scope-exit `OpFreeRef`
        // (var_scope / var_order / lift_vars) and an inline-ref entry init (no alloc); the
        // `OpDatabase` below allocates its store, freed at scope exit.
        let tmp = self.new_lift_var(function, &inner_tp);
        let db = data.def_nr("OpDatabase");
        let cp = data.def_nr("OpCopyRecord");
        let cs = data.def_nr("OpCreateStack");
        let preamble = vec![
            Value::Call(db, vec![Value::Var(tmp), type_val.clone()]),
            Value::Call(cp, vec![orig.clone(), Value::Var(tmp), type_val.clone()]),
        ];
        let arg = Value::Call(cs, vec![Value::Var(tmp)]);
        let postamble = Value::Call(cp, vec![Value::Var(tmp), orig, type_val]);
        Some((preamble, arg, postamble))
    }

    /// EXPERIMENT (D-clo-14) — the dep list a lifted collection `Join` return carries: it
    /// NAMES the caller variable the return may still be aliasing, which is what makes the
    /// owner decidable at run time by store identity.  `Deps::none()` (owned) otherwise.
    fn lift_deps(base_witness: u16) -> Deps {
        if base_witness == u16::MAX {
            Deps::none()
        } else {
            Deps::frame1(base_witness)
        }
    }

    /// `OpReplaceKeyed(<branch>, r, tp)` with per-arm temps lifted out of `<branch>`, or
    /// `None` where the value is not that op, its branch has no qualifying arm, or it was
    /// rewritten already (the arms then end in `Insert`s whose tails are temps, not calls).
    fn rewrite_keyed_replace_branch(
        &mut self,
        val: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Option<Value> {
        let Value::Call(d, args) = val.unspan() else {
            return None;
        };
        if data.def(*d).name() != "OpReplaceKeyed" || args.len() < 2 {
            return None;
        }
        let Value::Var(target) = args[1].unspan() else {
            return None;
        };
        if !Self::is_value_branch(&args[0])
            || !self.arm_tails_need_binding(&args[0], *target, data, function)
        {
            return None;
        }
        let home = self.var_scope.get(target).copied().unwrap_or(self.scope);
        let mut branch = args[0].clone();
        let _ = self.lift_join_arm_tails(&mut branch, home, *target, function, data);
        let mut new_args = args.clone();
        new_args[0] = branch;
        let call = Value::Call(*d, new_args);
        Some(match val {
            Value::Span(b) => Value::Span(Box::new((b.0.clone(), call))),
            _ => call,
        })
    }

    /// How many loops enclose scope `home` — the loop depth a binding declared THERE sees,
    /// as opposed to the depth at the current position.  `self.loops` holds the scope ids of
    /// the loops entered so far and `self.stack` the enclosing scopes outer-to-inner, so the
    /// loops that enclose `home` are those at or before it on that stack.
    fn loop_depth_at(&self, home: u16) -> usize {
        if home == self.scope {
            return self.loops.len();
        }
        match self.stack.iter().position(|&sc| sc == home) {
            Some(idx) => self
                .loops
                .iter()
                .filter(|&&l| l == home || self.stack[..idx].contains(&l))
                .count(),
            None => self.loops.len(),
        }
    }

    /// Does binding this fn-ref call hand the binding a collection store of its own — one the
    /// callee COPIED into the buffer minted for this call, or minted outright?  False for a
    /// call that is not a resolved, capture-free fn-ref, for a callee answering a raw VIEW of
    /// its argument (a keyed field, an index read — `returns_borrowed_view`), and for a `Join`,
    /// whose owner is decided per execution by the arms above.  The fallback is *"keep the
    /// borrow"*: a shape this cannot read costs the leak it already had, never a free of the
    /// caller's store.
    fn callref_delivers_collection(&self, tp: &Type, value: &Value, data: &Data) -> bool {
        let base_tp = tp.base();
        if !(matches!(base_tp, Type::Vector(_, _)) || crate::parser::vectors::is_keyed(base_tp)) {
            return false;
        }
        let Value::CallRef(v_nr, _) = value.unspan() else {
            return false;
        };
        let d_nr = match self.fnref_target.get(v_nr).copied() {
            Some(d) if d != u32::MAX => d,
            _ => return false,
        };
        let def = data.def(d_nr);
        // The `["??"]` marker is the callee's own word that its return is a `Join` — the
        // buffer on one arm, the argument on the other — so it is excluded by name as well
        // as by the oracle's verdict.
        def.code != Value::Null
            && !def.returns_borrowed_view()
            && !def.returned().depend().contains(&u16::MAX)
            && !crate::use_analysis::callref_capture_blocks(data, self.d_nr, value)
            && !matches!(
                crate::use_analysis::ownership_of(data, self.d_nr, value),
                crate::use_analysis::Own::Join { .. }
            )
    }

    /// Is this RHS a BRANCH — an `if` expression, or a `match` lowered to a value block whose
    /// tail is the `if` chain?  Only a branch is rewritten: a bare call is the bound spelling
    /// already, and rewriting it into a bound temp would scan the same shape forever.
    /// The statement form of `Set(v, <value branch>)`: every arm tail `t` becomes `Set(ov, t)`
    /// and the arms stop yielding a value — or `None` where an arm tail is not one a plain
    /// bind can take as it is: the binding itself, a compiler temp (a `??` hoist, a lift, a
    /// literal's work-ref), or a shape this cannot read.  `ov` is the ORIGINAL id, so each
    /// sunk `Set` takes the same scope mapping the value form would have.
    fn sink_set_into_arms(v: u16, ov: u16, value: &Value, function: &Function) -> Option<Value> {
        fn sinkable(tail: &Value, v: u16, ov: u16, function: &Function) -> bool {
            match tail.unspan() {
                Value::Var(x) => {
                    *x != v
                        && *x != ov
                        && (*x as usize) < function.count() as usize
                        && !function.is_compiler_generated(*x)
                }
                Value::Null | Value::Call(_, _) | Value::CallRef(_, _) => true,
                Value::If(_, t, f) => sinkable(t, v, ov, function) && sinkable(f, v, ov, function),
                Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => bl
                    .operators
                    .last()
                    .is_some_and(|l| sinkable(l, v, ov, function)),
                Value::Insert(ops) => ops.last().is_some_and(|l| sinkable(l, v, ov, function)),
                _ => false,
            }
        }
        fn sink(node: &mut Value, ov: u16) {
            match node {
                Value::Span(b) => sink(&mut b.1, ov),
                Value::If(_, t, f) => {
                    sink(t, ov);
                    sink(f, ov);
                }
                Value::Block(bl) => {
                    if let Some(last) = bl.operators.last_mut() {
                        sink(last, ov);
                    }
                    bl.result = Type::Void;
                }
                Value::Insert(ops) => {
                    if let Some(last) = ops.last_mut() {
                        sink(last, ov);
                    }
                }
                tail => {
                    let t = std::mem::replace(tail, Value::Null);
                    *tail = Value::Set(ov, Box::new(t));
                }
            }
        }
        if !sinkable(value, v, ov, function) {
            return None;
        }
        let mut out = value.clone();
        sink(&mut out, ov);
        Some(out)
    }

    /// Is this value produced by a BRANCH — one whose paths may each need a binding of their
    /// own ([`Self::lift_join_arm_tails`])?
    ///
    /// A value block counts through its TAIL, and the tail is not always an `if`.  A
    /// `?? return` discharge leaves the absent path by an early return, so what remains is a
    /// statement `if` that diverges and then a bare `Var` naming the temp the block hoisted its
    /// subject into — one surviving arm, spelled without a join.  Read as "not a branch" that
    /// arm never reached [`Self::arm_bind`], so a discharged projection kept aliasing a
    /// container across a `remove` that renumbered it while the `??`-with-default spelling of
    /// the very same read materialised (loft#1401).  One arm is still a path.
    ///
    /// Only a tail naming a local the block ITSELF bound qualifies, never an arbitrary
    /// variable: the value has to be one computed here for a per-path binding to mean anything.
    fn is_value_branch(node: &Value) -> bool {
        match node.unspan() {
            Value::If(_, _, _) => true,
            Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => {
                let tail = bl.operators.last();
                tail.is_some_and(Self::is_value_branch)
                    || matches!(tail.map(Value::unspan), Some(Value::Var(x))
                        if bl.operators.iter().any(|o|
                            matches!(o.unspan(), Value::Set(s, _) if s == x)))
            }
            _ => false,
        }
    }

    /// Does any arm of this value-yielding branch end in a tail that gets a binding of its
    /// own ([`Self::arm_bind`])?  The read-only twin of [`Self::lift_join_arm_tails`], so the
    /// RHS is cloned only when something in it will be rewritten.  A value branch is seen
    /// through its wrappers: a `Span`, a value `Block` (a plain arm, or a `scalar_match`
    /// behind its subject binding) and an `Insert` all yield their LAST operator.
    fn arm_tails_need_binding(
        &mut self,
        node: &Value,
        bound: u16,
        data: &Data,
        function: &Function,
    ) -> bool {
        match node.unspan() {
            Value::If(_, t, f) => {
                self.arm_tails_need_binding(t, bound, data, function)
                    || self.arm_tails_need_binding(f, bound, data, function)
            }
            Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => bl
                .operators
                .last()
                .is_some_and(|l| self.arm_tails_need_binding(l, bound, data, function)),
            Value::Insert(ops) => ops
                .last()
                .is_some_and(|l| self.arm_tails_need_binding(l, bound, data, function)),
            Value::CallRef(_, _) | Value::Call(_, _) | Value::Var(_) => self
                .arm_bind(node, bound, data, function, &mut false)
                .is_some(),
            _ => false,
        }
    }

    /// `@FR-O-Complete` — give each path of a value branch its own binding.  Every arm tail
    /// [`Self::arm_bind`] answers for is rewritten into the BOUND spelling on a `__lift_N`
    /// temp — `{ __lift_N = <tail>; __lift_N }`, or a refill of a vector buffer — declared at
    /// `home`, the scope the joined binding lives in, so it outlives the arm and dies with the
    /// binding that borrows it.  The joined binding `bound` then borrows the temps: its dep
    /// list is rewritten to name them in place of the variables the arms copied, so the fact
    /// it carries is true on every path (a variable another arm still views stays).
    fn lift_join_arm_tails(
        &mut self,
        node: &mut Value,
        home: u16,
        bound: u16,
        function: &mut Function,
        data: &Data,
    ) -> bool {
        let mut copied: Vec<(u16, u16)> = Vec::new();
        let mut viewed: Vec<u16> = Vec::new();
        let mut materialised = false;
        self.lift_arm_tails_into(
            node,
            home,
            bound,
            function,
            data,
            &mut copied,
            &mut viewed,
            &mut materialised,
        );
        // Each `__lift_N = a` is a whole-value copy the collector never saw (the lift is built
        // after it ran), so the drop moves here by the same rule: the arm's variable stops
        // dropping, the temp — and through the join, the binding — owns the resource.
        for &(src, tmp) in &copied {
            if let Some(moved) = copy_moves_drop_from(function, data, tmp, src, true) {
                self.drop_transferred.insert(moved);
            }
        }
        if bound == u16::MAX || (copied.is_empty() && viewed.is_empty()) {
            return materialised;
        }
        // A binding assigned elsewhere keeps the parser's fact: the runtime join bind copies
        // its arms there, and naming a hoist here would make it a borrow at every Set it has.
        if self.multi_assigned.contains(&bound) {
            return materialised;
        }
        let mut deps: Vec<u16> = function.tp(bound).depend().clone();
        // A `??` hoist the join hands back as an arm is a binding the join BORROWS on that
        // path — say so, or the joined binding reads as owning what the hoist holds, and a
        // return of it cannot be seen to hand the hoist's store out.
        for x in viewed {
            if !deps.contains(&x) {
                deps.push(x);
            }
        }
        let lifted: Vec<u16> = copied.iter().map(|&(_, tmp)| tmp).collect();
        for &(src, _) in &copied {
            if !mentions_var_outside(node, src, &lifted) {
                deps.retain(|&d| d != src);
            }
        }
        for &(_, tmp) in &copied {
            if !deps.contains(&tmp) {
                deps.push(tmp);
            }
        }
        function.depend_on_all(bound, &deps);
        materialised
    }

    /// The walk behind [`Self::lift_join_arm_tails`]; `copied` collects `(source, temp)` for
    /// every VARIABLE arm that was copied into a temp, `viewed` every `??` hoist temp an arm
    /// hands back as it is.
    #[allow(clippy::too_many_arguments)]
    fn lift_arm_tails_into(
        &mut self,
        node: &mut Value,
        home: u16,
        bound: u16,
        function: &mut Function,
        data: &Data,
        copied: &mut Vec<(u16, u16)>,
        viewed: &mut Vec<u16>,
        materialised: &mut bool,
    ) {
        match node {
            Value::Span(b) => {
                self.lift_arm_tails_into(
                    &mut b.1,
                    home,
                    bound,
                    function,
                    data,
                    copied,
                    viewed,
                    materialised,
                );
            }
            Value::If(_, t, f) => {
                self.lift_arm_tails_into(
                    t,
                    home,
                    bound,
                    function,
                    data,
                    copied,
                    viewed,
                    materialised,
                );
                self.lift_arm_tails_into(
                    f,
                    home,
                    bound,
                    function,
                    data,
                    copied,
                    viewed,
                    materialised,
                );
            }
            Value::Block(bl) if !matches!(bl.result, Type::Void | Type::Null) => {
                if let Some(last) = bl.operators.last_mut() {
                    self.lift_arm_tails_into(
                        last,
                        home,
                        bound,
                        function,
                        data,
                        copied,
                        viewed,
                        materialised,
                    );
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    self.lift_arm_tails_into(
                        last,
                        home,
                        bound,
                        function,
                        data,
                        copied,
                        viewed,
                        materialised,
                    );
                }
            }
            Value::CallRef(_, _) | Value::Call(_, _) | Value::Var(_) => {
                let src = if let Value::Var(x) = node {
                    Some(*x)
                } else {
                    None
                };
                if let Some(x) = src
                    && is_discharge_hoist(function, x)
                    && crate::data::is_dbref(function.tp(x).base())
                {
                    viewed.push(x);
                }
                match self.arm_bind(node, bound, data, function, materialised) {
                    Some(ArmBind::Bind(tp)) => {
                        let tmp = self.new_lift_var(function, &tp);
                        self.var_scope.insert(tmp, home);
                        self.lift_decl_depth.insert(tmp, self.loop_depth_at(home));
                        let tail = std::mem::replace(node, Value::Null);
                        *node = Value::Insert(vec![v_set(tmp, tail), Value::Var(tmp)]);
                        if let Some(src) = src {
                            copied.push((src, tmp));
                        }
                    }
                    Some(ArmBind::CopyVector { tp, elem }) => {
                        let tmp = self.new_buffer_var(function, &tp);
                        let tail = std::mem::replace(node, Value::Null);
                        *node = Value::Insert(vec![
                            Value::Call(
                                data.def_nr("OpReplaceVector"),
                                vec![Value::Var(tmp), tail, Value::Int(elem)],
                            ),
                            Value::Var(tmp),
                        ]);
                        if let Some(src) = src {
                            copied.push((src, tmp));
                        }
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    /// What gives this arm tail a binding of its own, or `None` where the arm keeps the value
    /// it has.  The answer is what the SINGLE bind `t = <tail>` would leave `t` holding, and
    /// the temp is bound by that single bind's own lowering — nothing here re-derives a copy:
    ///
    ///   * a fn-ref call — [`Self::arm_callref_lift_type`];
    ///   * a named call answering a RECORD the caller must COPY (`@FR-O-Move`: a borrowed or
    ///     `Join` return; codegen's `callee_of` arm is the copy) — an owned temp.  A named
    ///     call's OWNED record and every named COLLECTION return already land in the
    ///     caller-side `__ref_N` buffer, a per-site owner, so those arms stay;
    ///   * a plain VARIABLE, which a plain bind COPIES (`@FR-B-Copy`) — a record temp bound
    ///     from it (codegen copies a same-struct `Var` bind on its first and every later
    ///     Set), or a vector buffer refilled from it (`OpReplaceVector`).  Not a compiler temp
    ///     — a `__lift_N` is this rewrite's own product, an `_elm_N` a slot inside a container
    ///     — except a null-discharge HOIST on a binding the walk has NAMED: there the temp
    ///     holds the subject projection (`@FR-B-View-Depth`) and copies exactly as the inline
    ///     spelling of it does.  Not the binding itself (`r = if c { r } else { … }`), whose
    ///     transition free already reads that it is read.  Not a keyed collection, which
    ///     `OpReplaceKeyed` copies whatever the arm.  Not a `&` binding, which has no
    ///     `Var`-copy lowering to hand the temp to.  A struct-`Enum` variable IS one: it is
    ///     the same heap record shape as a struct (`Type::heap_def_nr`), and both emitters
    ///     copy its `Var` bind.
    ///
    /// The fallback `None` is *"the join borrows this arm as it did"*: a literal or a
    /// comprehension owns through its per-site buffer, a projection is a view, and a shape
    /// this cannot read costs the alias or the leak it already had — never a free of a store
    /// the caller still names.
    fn arm_bind(
        &mut self,
        tail: &Value,
        bound: u16,
        data: &Data,
        function: &Function,
        materialised: &mut bool,
    ) -> Option<ArmBind> {
        match tail.unspan() {
            Value::CallRef(_, _) => self
                .arm_callref_lift_type(tail, data, function)
                .map(ArmBind::Bind),
            // A PROJECTION tail, when the joined binding is one `(B-View)`'s materialise clause
            // names: this arm VIEWS a container that is disturbed while the binding is live, so
            // this path needs a store of its own — `(O-Complete)`, per binding and per PATH.
            // An owned temp is how it gets one: `__lift_N = h.inner` carrying no deps is the
            // plain projection bind the F1 materialise already copies on both backends, so
            // nothing here re-derives a copy, which is this function's whole contract.
            //
            // Gated on the walk's answer and not on the shape, because a projection arm whose
            // container is NEVER disturbed must keep aliasing — that is what `(B-View)` is for,
            // and copying it would lose a write that lands today.  Only the ARM is rewritten;
            // an arm that mints keeps its own store and the join reconciles them as before.
            Value::Call(d, _)
                if crate::use_analysis::is_projection_op(data, *d)
                    && bound != u16::MAX
                    && self.views_to_materialise.contains_key(&bound) =>
            {
                *materialised = true;
                let (base, opt) = function.tp(bound).peel_optional();
                match base {
                    Type::Reference(r, _) => Some(ArmBind::Bind(Self::reopt(
                        opt,
                        Type::Reference(*r, Deps::none()),
                    ))),
                    Type::Enum(r, true, _) => Some(ArmBind::Bind(Self::reopt(
                        opt,
                        Type::Enum(*r, true, Deps::none()),
                    ))),
                    // A COLLECTION arm needs the copy EMITTED, not just a temp bound: the
                    // vector bind's copy-vs-view is decided at PARSE time, so a temp typed
                    // without deps arrives too late to be heard.  Same buffer-and-refill the
                    // whole-vector copy takes (loft#1377's arm), reached here because the walk
                    // NAMES a collection view on this tree — which is what made the same idea
                    // inert where it does not (loft#1399).
                    Type::Vector(inner, _) => {
                        let wrapper = format!("main_vector<{}>", inner.name(data));
                        if data.name_type(&wrapper, data.def(self.d_nr).source) == u16::MAX {
                            return None;
                        }
                        let elem = data.vector_element_type(inner, self.database)?;
                        Some(ArmBind::CopyVector {
                            tp: Type::Vector(inner.clone(), Deps::none()),
                            elem: i32::from(elem),
                        })
                    }
                    _ => None,
                }
            }
            Value::Call(d, _) => {
                let def = data.def(*d);
                if !def.is_loft_defined() {
                    return None;
                }
                let (returned, opt) = def.returned().peel_optional();
                let tp = match returned {
                    Type::Reference(r, _) => Type::Reference(*r, Deps::none()),
                    Type::Enum(r, true, _) => Type::Enum(*r, true, Deps::none()),
                    _ => return None,
                };
                match crate::use_analysis::ownership_of(data, self.d_nr, tail) {
                    crate::use_analysis::Own::Owned => None,
                    crate::use_analysis::Own::Borrowed { .. }
                    | crate::use_analysis::Own::Join { .. } => {
                        Some(ArmBind::Bind(Self::reopt(opt, tp)))
                    }
                }
            }
            Value::Var(x) => {
                // A branch consumed as a call ARGUMENT binds nothing: the argument ALIASES
                // the caller's variable (calls.md F-ParamHeap), and a callee that hands its
                // argument back must hand back that variable's store, not a temp's.  Copying
                // the arm there gave D-own-16's `c = maybe_b(c ?? M {}, i)` a temp that died
                // at the statement while `c` still named it.
                if bound == u16::MAX || *x == bound {
                    return None;
                }
                // A compiler temp is not a plain variable a bind may copy: a `__lift_N` is
                // this rewrite's own product and an `_elm_N` is a slot inside a container.
                // The ONE exception is a null-discharge HOIST on a binding the walk has named
                // as a view to materialise.  There the arm is the subject PROJECTION wearing
                // the temp the lowering hoisted it into — the arm right above copies that same
                // projection when it is spelled inline — and `(H-Materialise)` gives the path
                // a store of its own, which is what makes `c = v[1] ?? d` agree with `c = v[1]`
                // across a disturbance of `v` (loft#1401).
                //
                // Gated on the walk's answer and not on the shape, for the reason the
                // projection arm states: a discharge whose container is NEVER disturbed must
                // keep aliasing, and copying it would lose a write that lands today.
                if function.is_compiler_generated(*x) {
                    if !(is_discharge_hoist(function, *x)
                        && self.views_to_materialise.contains_key(&bound))
                    {
                        return None;
                    }
                    *materialised = true;
                }
                // Only for a binding the join is the ONE assignment of.  A binding assigned
                // elsewhere as an owner — first bound by a plain copy and re-bound from the
                // join, `r = x; for … { r = v[i] ?? x }` — takes the runtime join bind
                // (`OpBindOrCopy` in codegen), which copies whatever the join hands it; lifting
                // there would turn one binding's fact into a borrow at every one of its Sets
                // and orphan the copies the others made (`@FR-O-Latest`: the fact belongs to
                // the assignment, and a type-level list cannot carry two).
                //
                // Unless the walk has NAMED this binding, which is a fact about THIS
                // assignment and not about the type: the arm needs a store of its own or the
                // binding aliases a container that has been disturbed under it.  Nothing
                // type-level is claimed either way — `lift_join_arm_tails` skips the dep
                // rewrite for a multi-assigned binding on its own, so the copy lands and the
                // other Sets keep the fact they had.  The PROJECTION arm above never asked
                // this question, so a `c = Box{…}; c = v[1] ?? Box{…}` differed from its
                // inline twin only in which of the two arms the projection was spelled in
                // (loft#1401).
                if self.multi_assigned.contains(&bound)
                    && !self.views_to_materialise.contains_key(&bound)
                {
                    return None;
                }
                let (base, opt) = function.tp(*x).peel_optional();
                match base {
                    Type::Reference(r, _) => Some(ArmBind::Bind(Self::reopt(
                        opt,
                        Type::Reference(*r, Deps::none()),
                    ))),
                    // A struct-enum is the same heap record shape (`Type::heap_def_nr`), and
                    // codegen copies its `Var` bind exactly as a struct's.
                    Type::Enum(r, true, _) => Some(ArmBind::Bind(Self::reopt(
                        opt,
                        Type::Enum(*r, true, Deps::none()),
                    ))),
                    Type::Vector(inner, _) => {
                        // The buffer's function-entry allocation names the wrapper type by
                        // name; a vector kind this program never built has none, and the
                        // refill op names the element type only the registry knows.
                        let wrapper = format!("main_vector<{}>", inner.name(data));
                        if data.name_type(&wrapper, data.def(self.d_nr).source) == u16::MAX {
                            return None;
                        }
                        let elem = data.vector_element_type(inner, self.database)?;
                        Some(ArmBind::CopyVector {
                            tp: Type::Vector(inner.clone(), Deps::none()),
                            elem: i32::from(elem),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The type a per-arm temp takes for this fn-ref call, or `None` where the arm keeps the
    /// value it has: the target must resolve and the fn-ref must capture no store the oracle
    /// cannot name.  What the temp is typed as is what the SINGLE bind of that call would leave
    /// the binding holding — `@FR-O-Complete` asks for the fact per binding, per path, and the
    /// temp is that binding:
    ///
    ///   * an OWNED return — the closure minted the store — is adopted, so the temp owns;
    ///   * a BORROWED return is what `@FR-O-Move` says the caller COPIES: a record's bind does
    ///     that in codegen (`callee_of`, the same arm a direct call takes), and a collection
    ///     is copied by the CALLEE into its per-call buffer — so both own.  A collection the
    ///     callee hands back as a raw view (`returns_borrowed_view`, a keyed field or an index
    ///     read, which the delivery does not copy) is the one shape left alone: the join then
    ///     borrows the caller's store on that arm, which is the view it always was;
    ///   * a `Join` return is the runtime question: a record temp owns either way through the
    ///     bound spelling's `OpBindOrCopy`, and a collection temp carries the base as its dep,
    ///     which is what `scan_set` reads to free it by identity.
    ///
    /// The fallback `None` is *"this arm hands the join a value the join may keep borrowing"*:
    /// an unresolved target or a blocked capture answers as before (the arm's store leaks
    /// rather than being freed under a caller), and a view stays a view.
    fn arm_callref_lift_type(&self, val: &Value, data: &Data, function: &Function) -> Option<Type> {
        let Value::CallRef(v_nr, _) = val.unspan() else {
            return None;
        };
        let d_nr = match self.fnref_target.get(v_nr).copied() {
            Some(d) if d != u32::MAX => d,
            _ => return None,
        };
        let def = data.def(d_nr);
        if def.code == Value::Null
            || crate::use_analysis::callref_capture_blocks(data, self.d_nr, val)
        {
            return None;
        }
        let _ = function;
        let (returned, opt) = def.returned().peel_optional();
        let record = matches!(returned, Type::Reference(_, _) | Type::Enum(_, true, _));
        let witness = match crate::use_analysis::ownership_of(data, self.d_nr, val) {
            crate::use_analysis::Own::Join { base } => {
                if base == u16::MAX {
                    return None;
                }
                if record {
                    Deps::none()
                } else {
                    Deps::frame1(base)
                }
            }
            crate::use_analysis::Own::Owned => {
                // ⚠ `Own::Owned` is ALSO the `CallRef` arm's fallback for a base it cannot
                // name — its own doc says so — so at a site that frees it is not a verdict.
                // A second hop reaches it: `fwd = fn(q) { inner(q) }` resolves (loft#1329
                // made a fn-ref capture resolvable) and answers `Owned` because the base
                // arrives through the capture, while the callee's own type says the return
                // borrows `q`.  Taking that at face value gave the temp an unwitnessed free
                // of the CALLER's collection, one per evaluation.
                //
                // So ask the CALLEE, which is where the fact is: a return that borrows a
                // visible parameter is not owned, whatever the caller-side walk resolved.
                // Its DECLARED dep still names WHICH parameter, so the arm keeps a binding
                // and the store is decided per execution by identity against the argument —
                // the borrow arm hands back that store and declines, the mint arm is
                // distinct and frees.  With no nameable argument there is no witness, and
                // the arm keeps the leak it had rather than freeing blind.
                if def.returns_borrowed_view() {
                    let base =
                        crate::use_analysis::callref_declared_borrow_base(data, self.d_nr, val)?;
                    if self.multi_assigned.contains(&base) {
                        return None;
                    }
                    if record {
                        Deps::none()
                    } else {
                        Deps::frame1(base)
                    }
                } else {
                    Deps::none()
                }
            }
            crate::use_analysis::Own::Borrowed { .. } => {
                if !record && def.returns_borrowed_view() {
                    return None;
                }
                Deps::none()
            }
        };
        let tp = match returned {
            Type::Reference(d, _) => Type::Reference(*d, Deps::none()),
            Type::Enum(d, true, _) => Type::Enum(*d, true, Deps::none()),
            Type::Vector(inner, _) => Type::Vector(inner.clone(), witness),
            Type::Hash(d, k, _) => Type::Hash(*d, k.clone(), witness),
            Type::Sorted(d, k, _) => Type::Sorted(*d, k.clone(), witness),
            Type::Index(d, k, _) => Type::Index(*d, k.clone(), witness),
            Type::Radix(d, k, _) => Type::Radix(*d, k.clone(), witness),
            Type::Trie(d, k, _) => Type::Trie(*d, k.clone(), witness),
            _ => return None,
        };
        Some(Self::reopt(opt, tp))
    }

    /// The witness SLOT for a collection local bound from a closure's `Join` return where the
    /// base variable cannot stand witness itself — one per local, written beside every bind
    /// of that local from that bind's base.  A borrow of the base's type (never freed, never
    /// allocated for), homed where the local lives so it is readable at every free of it.
    fn snapshot_witness_for(&mut self, v: u16, base: u16, function: &mut Function) -> u16 {
        if let Some(&w) = self.snapshot_witness.get(&v) {
            return w;
        }
        self.lift_counter += 1;
        let name = format!("__wit_{}", self.lift_counter);
        let tp = function.tp(base).base().with_deps(&Deps::frame1(base));
        let wit = function.add_temp_var(&name, &tp);
        function.set_skip_free(wit);
        let home = self.var_scope.get(&v).copied().unwrap_or(self.scope);
        self.var_scope.insert(wit, home);
        self.var_order.push(wit);
        self.lift_vars.push(wit);
        self.snapshot_witness.insert(v, wit);
        wit
    }

    /// A `__lift_N` that OWNS a vector store for the function's whole life — a per-site
    /// buffer like the parser's `__vdb_N`: allocated by its function-entry `Set(tmp, Null)`
    /// (an owned vector's null-init allocates), refilled IN PLACE by the arm that reaches it,
    /// and freed once at function exit.  Homed at the function's root scope for exactly that
    /// reason: freed per iteration, it would be refilled dead.
    fn new_buffer_var(&mut self, function: &mut Function, tp: &Type) -> u16 {
        self.lift_counter += 1;
        let name = format!("__lift_{}", self.lift_counter);
        let tmp = function.add_temp_var(&name, tp);
        let root = self.stack.get(1).copied().unwrap_or(self.scope);
        self.lift_decl_depth.insert(tmp, 0);
        self.var_scope.insert(tmp, root);
        self.var_order.push(tmp);
        self.lift_vars.push(tmp);
        tmp
    }

    /// Create a `__lift_N` temporary that OWNS an inline call result, so
    /// `get_free_vars` emits its `OpFreeRef` at scope exit.  Registers the
    /// var in the current scope and in `lift_vars` (which drives the
    /// function-entry `Set(v, Null)` slot reservation).  The caller emits
    /// the `Set(tmp, call)` itself — as an arg preamble (`scan_args`) or as
    /// the statement replacing a `Drop` (#490).
    fn new_lift_var(&mut self, function: &mut Function, tp: &Type) -> u16 {
        self.lift_counter += 1;
        let name = format!("__lift_{}", self.lift_counter);
        let tmp = function.add_temp_var(&name, tp);
        function.mark_inline_ref(tmp);
        let witness = self.pending_join_witness.replace(u16::MAX);
        if witness != u16::MAX {
            self.lift_join_witness.insert(tmp, witness);
        }
        self.lift_decl_depth.insert(tmp, self.loops.len());
        self.var_scope.insert(tmp, self.scope);
        self.var_order.push(tmp);
        self.lift_vars.push(tmp);
        tmp
    }

    /// #248 — does this scanned argument lower to an inline call (or an
    /// `Insert`/`Span` whose final op is one) that PRODUCES a heap value
    /// (vector / reference / struct-enum)?  Used by `scan_args` to force-lift a
    /// trailing heap-returning call argument when the call's receiver is a
    /// borrowed `OpCreateStack` ref — the interpreter arg-layout hazard #248.
    ///
    /// Unlike [`inline_struct_return`], this DOES match calls that return via a
    /// hidden caller work-ref (non-empty dep like `["??"]`): those are exactly
    /// the frame-growing inline calls that `inline_struct_return` skips but that
    /// still shift the receiver's stack slot.  Returns the owned element/struct
    /// type (empty dep) for the `__lift_N` temp, or `None`.
    fn heap_call_return(val: &Value, data: &Data) -> Option<Type> {
        // Peel `Span` / trailing-op-of-`Insert` to reach the producing call.
        let inner = match val.unspan() {
            Value::Insert(ops) => ops.last()?.unspan(),
            other => other,
        };
        let Value::Call(fn_nr, _) = inner else {
            return None;
        };
        let def = data.def(*fn_nr);
        // Only user/method bodies (n_* / t_*) — native helpers and the
        // OpCreateStack/OpVar* lowering ops never own a fresh return store here.
        if !def.is_loft_defined() {
            return None;
        }
        match &def.returned {
            Type::Vector(elem, _) => Some(Type::Vector(elem.clone(), Deps::none())),
            Type::Reference(d_nr, _) => Some(Type::Reference(*d_nr, Deps::none())),
            Type::Enum(d_nr, true, _) => Some(Type::Enum(*d_nr, true, Deps::none())),
            _ => None,
        }
    }

    /// loft#1176 — does a monomorph whose tail is a call THROUGH A FN-REF hand back a
    /// store this caller must own?
    ///
    /// [`Definition::monomorph_return_is_fresh`] reads the callee's body and answers
    /// `false` for a tail like `f(x)`: what comes back is the fn-ref target's fact, and
    /// inside the monomorph `f` is a runtime value with no definition to ask.  The fact
    /// is not unreachable, only unreachable from THERE — at this call site the caller
    /// wrote which closure it passed, so `fnref_target` resolves it and the same
    /// body-shaped question is put to the definition that actually runs.
    ///
    /// **The resolved target must pass BOTH ownership reads, and the pair is not
    /// redundant.**  `returns_borrowed_view` is the deps proxy and catches a lambda
    /// handing back its own PARAMETER; it does not catch one handing back a CAPTURE,
    /// because the dep then names the hidden `__closure` attribute and a hidden attr is
    /// read as "not a borrow" (loft#1114).  `monomorph_return_is_fresh` is what closes
    /// that: the capture arrives as a place read rooted at `__closure`, which is an
    /// argument, so the body-shaped proof refuses it.  Measured — the concrete twin
    /// `fn once_p(x: P, f: fn(P) -> P) -> P { f(x) }` lifted on the deps proxy alone and
    /// CORRUPTED the captured record on both backends (`formal/closures.md` D-clo-9);
    /// this route declines that same shape.
    ///
    /// Every unresolved position answers `false`, which leaves the leak that was already
    /// there.  That is the direction this whole gate takes when it cannot name what it
    /// would be freeing: a wrong `true` frees a store the caller still holds.
    /// A monomorph whose return sites are direct CALLS delivers a fresh store when every
    /// one of those callees does — loft#1273.
    ///
    /// `fn add<T: Addable>(a: T, b: T) -> T { a + b }` lowers its tail to
    /// `Call(n_OpAdd, …)`, and the user's `OpAdd` mints a record. `monomorph_return_is_fresh`
    /// cannot see that — a callee's ownership is not a fact this body carries — so the
    /// result was never lifted and one record was retained per call, unbounded in a loop,
    /// while the bound spelling (`r = add(…); r.v`) was clean all along.
    ///
    /// The three questions are the fn-ref twin's, for the same reasons: the target must have
    /// a BODY, must not return a borrowed view — a callee handing back its own argument
    /// would make the lift free the caller's record — and must itself be proven fresh, so
    /// the proof stays positive and one unreadable link refuses the chain.
    ///
    /// One level, deliberately. A delegate that itself delegates answers `false` and keeps
    /// its leak, which is the direction every gate here takes when it cannot name what it
    /// would be freeing; recursing would also need a cycle guard for mutual recursion.
    /// No `self`: unlike the fn-ref twin, which resolves a closure through the caller's
    /// `fnref_target`, the target here is written in the IR and only `Data` is needed.
    fn monomorph_delegated_return_is_fresh(data: &Data, def: &crate::data::Definition) -> bool {
        let Some(targets) = def.monomorph_direct_call_return_targets() else {
            return false;
        };
        targets.iter().all(|&d_nr| {
            if d_nr as usize >= data.definitions() as usize {
                return false;
            }
            let target = data.def(d_nr);
            target.code != Value::Null
                && !target.returns_borrowed_view()
                && target.monomorph_return_is_fresh()
        })
    }

    fn monomorph_fnref_return_is_fresh(
        &self,
        val: &Value,
        data: &Data,
        def: &crate::data::Definition,
    ) -> bool {
        let Some(slots) = def.monomorph_fnref_return_slots() else {
            return false;
        };
        let Value::Call(_, args) = val.unspan() else {
            return false;
        };
        slots.iter().all(|&slot| {
            // A parameter's variable slot IS its argument position; a hidden argument the
            // lowering appends sits after every visible one, so a slot in range names the
            // value the caller wrote.
            let Some(arg) = args.get(slot as usize) else {
                return false;
            };
            let Value::Var(fn_var) = arg.unspan() else {
                return false;
            };
            let target = match self.fnref_target.get(fn_var).copied() {
                Some(d) if d != u32::MAX => data.def(d),
                _ => return false,
            };
            target.code != Value::Null
                && !target.returns_borrowed_view()
                && target.monomorph_return_is_fresh()
        })
    }

    /// Check whether a scanned argument at position `arg_idx` is an inline
    /// struct-returning call that needs lifting to a temporary variable.
    /// Returns the struct definition number if lifting is needed, None
    /// otherwise.
    ///
    /// Skips lifting when the outer call's return type depends on this argument
    /// (i.e. the result borrows from the argument's store).  Freeing the lifted
    /// temp at scope exit would be use-after-free in that case.
    /// loft#721 — may the aggregate a `CallRef` returns be LIFTED into an owned
    /// temp (and therefore freed)?
    ///
    /// The direct-call branch answers this from the callee's definition; a
    /// `CallRef`'s callee is a runtime value, so the fn-ref TYPE is all that is
    /// available at the call — and it is not enough. Measured: a closure that
    /// calls a struct-returning function and one that hands back a borrowed
    /// element present the SAME signature (`ret_deps=[1]`, one param), because
    /// the dep index lives in the callee's attribute space and says nothing on
    /// its own. Lifting on the type alone frees a borrowed element: a
    /// use-after-free that only `LOFT_POISON=1` makes visible.
    ///
    /// So resolve the target definition instead — `fnref_target` records which
    /// definition each fn-ref variable was assigned — and then ask the SAME
    /// canonical question the direct call asks, `returns_borrowed_view()`.
    /// An unknown or ambiguous target answers `None`: the result is not lifted,
    /// which is the pre-existing behaviour (it leaks, and a leak is recoverable
    /// where a premature free is not).
    fn callref_owned_return(
        &self,
        val: &Value,
        data: &Data,
        function: &Function,
        outer_call: u32,
    ) -> Option<Type> {
        let Value::CallRef(v_nr, _) = val.unspan() else {
            return None;
        };
        let d_nr = match self.fnref_target.get(v_nr).copied() {
            Some(d) if d != u32::MAX => d,
            _ => return None,
        };
        let def = data.def(d_nr);
        // loft#1245 — the INVARIANT: a call's returned store is ADOPTED by the caller only
        // where the callee minted it, and COPIED otherwise.  Which of the two happened is a
        // runtime fact for a `??` return, so the @P290 bracket decides it per execution —
        // and for that it needs a witness for every place the return could borrow FROM.
        //
        // A fn-ref can borrow from two places, and only one of them is witnessable: its
        // ARGUMENTS (which `protectable_ref_args` names) and its CAPTURES (which nothing at
        // the call site can name).  So the lift is admissible exactly when the witness set
        // is COMPLETE and the fn-ref captures nothing.
        if def.code == Value::Null {
            return None;
        }
        // `returns_borrowed_view()` is the deps PROXY (@FR-O-Proxy), and for a closure whose
        // return is a `??` it is right about one arm and wrong about the other: the dep names
        // the parameter or the capture the SUBJECT arm hands back, so the whole return reads
        // as a borrow and the DEFAULT arm's minted store is left owned by nobody — one store
        // per call, to frame exit, which a loop turns into the 65535-store ceiling
        // (loft#1248).
        //
        // @FR-O-Oracle is the authority, and this is the chokepoint that should read it —
        // the same sentence the direct-call branch below already acts on.  A `Join` lifts
        // only where the bind that follows is the runtime guard `OpBindOrCopy`: the witness
        // has to be NAMEABLE, and the statement has to be one that BINDS.  `outer_call ==
        // u32::MAX` is the bare-statement lowering, which gets no such bind, so there the
        // conservative no-lift stands and costs the mint arm's leak instead.
        //
        // ⚠ The relaxation reaches exactly as far as the GUARD does, and no further.  The
        // arms below also answer for the collection returns, and a lifted collection gets a
        // scope-exit free with nothing deciding per execution whether the store is the
        // caller's — `callref_join_first_bind`, which is what emits `OpBindOrCopy` on both
        // backends, answers only for a `Reference` / record `Enum`.  Widening past that
        // would trade this leak for the over-free in the other direction, which is the
        // trade the whole gate exists to refuse.
        let record_return = matches!(
            def.returned().peel_optional().0,
            Type::Reference(_, _) | Type::Enum(_, true, _)
        );
        let own = crate::use_analysis::ownership_of(data, self.d_nr, val);
        let join_lifts = record_return
            && matches!(own, crate::use_analysis::Own::Join { base }
                if base != u16::MAX && outer_call != u32::MAX);
        // A fn-ref borrows from two places and only one of them is witnessable at the call:
        // its ARGUMENTS, which `protectable_ref_args` names, and its CAPTURES, which nothing
        // at the call site can.  So a lift is admissible two ways, and needing EITHER is what
        // keeps both halves closed at once:
        //
        //   * `join_lifts` — the oracle named a WITNESS and the statement BINDS, so
        //     `OpBindOrCopy` decides per execution whose store it is (loft#1248);
        //   * `witnessed_lifts` — the witness set is COMPLETE and the fn-ref captures
        //     nothing, so there is nothing borrowed for the lift to free (loft#1245).
        //
        // The capture test is separate and unconditional because `returns_borrowed_view()`
        // is FALSE for one: a capture reaches the return through `__closure`, a hidden
        // attribute, and a hidden-only dep otherwise reads as *"the callee minted this"*.
        // A complete witness set says nothing there either — it reads complete VACUOUSLY for
        // a call whose arguments are all scalars, having witnessed nothing.
        // loft#1248's decline, narrowed to the capture that cannot be RESOLVED: the callee's
        // body names the offset its `??` subject reads, and one offset over a variable assigned
        // once is a witness as good as an argument's (D-clo-7).  A collection's chosen arm is
        // COPIED into `__retbuf`, so a capture-reading collection closure returns an owned
        // store and takes the witnessed route like any other.
        let blocks = crate::use_analysis::callref_capture_blocks(data, self.d_nr, val);
        // …and the witnessed route reaches exactly as far as a GUARDED release does.  The
        // bracket refuses the SOURCE-free of a record the callee handed back
        // (`do_copy_record`), so a record temp holds its own store by the time the scope-exit
        // free runs; a collection `Join` gets the identity free below, which names the base.
        // A collection the callee answers as a raw VIEW of its argument (a keyed field, an
        // index read — `Own::Borrowed`) has neither: the lift's `OpFreeRef` releases whatever
        // store the temp names, and that store is the caller's — `t = h(bag)` with
        // `h = fn(q) { q.m }` emptied `bag.m` after one call, both backends, where the release
        // still answered correctly.
        let collection_join = !record_return
            && matches!(own, crate::use_analysis::Own::Join { base } if base != u16::MAX);
        let witnessed_lifts = !blocks
            && (record_return || collection_join)
            && crate::use_analysis::protectable_ref_args(data, self.d_nr, val).1;
        if blocks && !join_lifts {
            return None;
        }
        if def.returns_borrowed_view() && !join_lifts && !witnessed_lifts {
            return None;
        }
        // loft#1257 — and the collection arms need the oracle for the OPPOSITE reason: to
        // stop lifting, not to start.  A collection return is delivered through a HIDDEN
        // buffer, so its dep names only hidden attributes and `returns_borrowed_view()`
        // answers false — *"the callee minted into its own buffer, the caller adopts"*.
        // Right when the closure mints, wrong when its `??` hands back the caller's
        // argument, and the proxy cannot tell those apart because they are the same call:
        // `fn(q: vector<integer>?) -> vector<integer> { q ?? [7, 8] }` reached the lift, and
        // the lifted temp then EMPTIED the caller's own vector — `len(some)` reached 0 after
        // five iterations, with nothing saying so.
        //
        // A `Join` whose base the bracket can NAME is exactly *"this may be that caller
        // variable"*.  The `Reference` / `Enum` arms may still lift it, because
        // `OpBindOrCopy` settles it per execution; a collection has no such guard, so here
        // the answer is to decline.
        //
        // ⚠ THIS IS A TRADE AND THE COST IS MEASURED: the MINT arm of the same closure goes
        // back to leaking one store per call (peak 4 -> 403 at N=400), which at scale is a
        // store-table abort.  Taken deliberately — a leak announces itself and a container
        // silently emptied does not, which is why `silent-wrong` outranks `sev:`
        // (`.github/LABELS.md`).  It costs only the JOIN shape: a pure mint classifies
        // `Owned`, or `Borrowed` of a hidden buffer with no nameable base, and loft#1177's
        // cells are all pure mints and keep their lift.
        //
        // The closure is a WITNESSED lift, and `OpFreeRefIfDistinct` is the right shape for
        // it — built and measured here.  It fixes `--native` and leaves the interpreter
        // wrong, because on that side the damage is not the free but the RE-SET: one
        // iteration is correct, two are not, so the transition-free on `__lift_N`'s
        // reassignment releases the borrowed store before any scope-exit free runs.  Both
        // halves are needed, and only the decline is correct on both backends today.
        //
        // The fn-ref variable's own type is the declared shape; the definition is
        // the authority on what it returns.
        let _ = function;
        let mut base_witness = u16::MAX;
        let (returned, opt) = def.returned().peel_optional();
        if !matches!(returned, Type::Reference(_, _) | Type::Enum(_, true, _))
            && let crate::use_analysis::Own::Join { base } =
                crate::use_analysis::ownership_of(data, self.d_nr, val)
            && base != u16::MAX
        {
            // loft#1257 — the IDENTITY route, and the reason this is no longer a decline.
            // Declining cost the mint arm one store per call (389 live at N=400, a
            // store-table abort at scale).  @FR-O-Oracle already says what a `Join` means at
            // run time — *"adopt iff the value's store ≠ base's store"* — and the dep NAMES
            // that base, so the owner is decidable by store IDENTITY with no witness slot.
            // `ownership.md` D-own-16 closed the same sentence one shape over.
            //
            // The base rides on the temp's TYPE (`lift_deps` below), which does both halves at
            // once: a non-empty dep keeps `state/codegen.rs`'s unconditional pre-Set free from
            // being emitted at all — the RE-SET that left the interpreter wrong when an earlier
            // attempt guarded only the scope-exit free — and `get_free_vars` then emits
            // `OpFreeRefIfDistinct(__lift_N, base)` there.  One guarded free per evaluation.
            if !crate::keys::lift_join_witness_enabled() {
                return None;
            }
            base_witness = base;
        }
        // The witness is handed to the next `new_lift_var` — and ONLY where an arm below
        // actually answers with a type, so a return that lifts nothing cannot leave it
        // standing for an unrelated temp.
        let lifted = match returned {
            Type::Reference(d, _) => Some(Self::reopt(opt, Type::Reference(*d, Deps::none()))),
            Type::Enum(d, true, _) => Some(Self::reopt(opt, Type::Enum(*d, true, Deps::none()))),
            // loft#1177 — a COLLECTION return is the same question with the same answer, and
            // it was missing: the arms named the two aggregate shapes a closure was known to
            // return and `_ => None` read as *"nothing else needs owning"*, which a
            // store-backed collection contradicts.  A lambda handing back a `vector` / keyed
            // collection used INLINE (`len(g(7))`) therefore had its store owned by nothing —
            // one leaked record per call, where the bound form `r = g(7)` was always clean.
            // The dep list is rebuilt empty for the same reason the two arms above rebuild
            // theirs: `returns_borrowed_view` has already refused a callee that hands back a
            // view, so what reaches here is a store the caller must own.
            Type::Vector(inner, _) => Some(Self::reopt(
                opt,
                Type::Vector(inner.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Hash(d, k, _) => Some(Self::reopt(
                opt,
                Type::Hash(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Sorted(d, k, _) => Some(Self::reopt(
                opt,
                Type::Sorted(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Index(d, k, _) => Some(Self::reopt(
                opt,
                Type::Index(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Radix(d, k, _) => Some(Self::reopt(
                opt,
                Type::Radix(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Trie(d, k, _) => Some(Self::reopt(
                opt,
                Type::Trie(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            // Everything else is a value the caller does not own a store for — a scalar
            // lives in the slot, and a `text` is freed by its own delivery path.
            _ => None,
        };
        self.pending_join_witness.set(if lifted.is_some() {
            base_witness
        } else {
            u16::MAX
        });
        lifted
    }

    /// loft#879 — the shape question `inline_struct_return` asks ("does this call
    /// hand back a store the caller must own, and of what shape?") is about the
    /// BASE type: `Optional(τ)` is a compile-time wrapper over τ's own runtime
    /// layout (@PLN25), so `-> C?` allocates and delivers exactly what `-> C`
    /// does.  Matching the arms below on the unpeeled type therefore answered
    /// "not liftable" for every optional aggregate return, and the result got a
    /// bare stack-pop (`FreeStack`) that never freed the store — one leaked
    /// record per call, unbounded in a loop, on the interpreter.
    ///
    /// The arms peel for the match; this puts the wrapper back on the temp's
    /// type, so a lifted temp is typed exactly like the hand-correct bound form
    /// (`x = pick(1)` → `x: optional(reference(C))` + a scope-exit `OpFreeRef`)
    /// that has always been the clean spelling.  Keeping the `Optional` matters:
    /// the temp may legitimately hold the null sentinel, and it is the bound
    /// form — not the non-optional one — that proves this type flows correctly
    /// through slot assignment, `get_free_vars`, and both backends' codegen.
    fn reopt(was_optional: bool, tp: Type) -> Type {
        if was_optional { Type::optional(tp) } else { tp }
    }

    /// loft#1118 — may an `ncc` block that DOES carry a dep be lifted anyway?
    ///
    /// The empty-dep test is the conservative reading of "nobody else owns this", and it
    /// refuses the shape a NULLABLE PARAMETER produces: a callee whose return may be the
    /// argument or may be a store it minted answers `Own::Join`, whose dep names that
    /// argument. The block then stayed inline with the minted store owned by nothing — one
    /// leaked record per evaluation, unbounded in a loop.
    ///
    /// Lifting is safe exactly when the bind that follows is the RUNTIME guard rather than
    /// a static bet. It is: a lifted `__lift_N` is a dense `Reference`, so the heap
    /// first-bind dispatch reaches it and emits `OpBindOrCopy`, which adopts the arm where
    /// the callee minted (making the scope-exit free right) and materialises the arm where
    /// the value is the witness's store (leaving the caller's argument intact). This asks
    /// the same `Own::Join` question of the same value, so the lift cannot fire where that
    /// guard would not.
    ///
    /// **The subject must be a call to a LOFT-DEFINED function**, and that is the whole of
    /// the narrowing rather than a detail. loft's IR spells every operator as a
    /// `Value::Call`, so "the subject is a call" also matches an element read (`t[p] ?? d`
    /// is `OpGetVector`) — a view INTO a container the caller still owns, where the lift
    /// hands the temp a free that reaches inside it. Measured, not hypothetical: admitting
    /// the read made the ownership fuzz gate's `local_source` cell answer WRONG on
    /// `--native` with the two backends diverging. Only the FIRST statement is read, too:
    /// the block's default arm is often a call of its own (`t[p] ?? dflt()`), and asking
    /// `any` operator re-admits the very cell this excludes.
    ///
    /// A join whose witness the bracket cannot name keeps the conservative no-lift, which
    /// costs the leak that was already there rather than a free nothing protects.
    fn ncc_join_is_witnessed(&self, val: &Value, data: &Data) -> bool {
        if !crate::keys::join_own_enabled() {
            return false;
        }
        let Value::Block(bl) = val.unspan() else {
            return false;
        };
        // The subject is the block's first REAL statement, not its first.  A REUSED
        // `__ncc_N` opens its block with an overwrite `OpFreeRef`, which is not a subject:
        // it shifts the `Set` to second, and a predicate reading `first()` then answers
        // "not a user call" for a block that is one.  Whether a given spelling reuses the
        // temp is a numbering property, so the set of spellings this hides is not stable
        // enough to name here —
        // `tests/scripts/1118b-an-inline-join-lifts-in-every-statement-context.loft` is
        // the measurement, one cell per statement context.
        //
        // Skipping a LEADING FREE cannot re-admit the `t[p] ?? dflt()` cell the narrowing
        // above excludes: the first non-free statement still has to be a `Set` of a
        // loft-defined call.
        let subject = bl
            .operators
            .iter()
            .map(Value::unspan)
            .find(|op| !matches!(op, Value::Call(d, _) if data.def(*d).name() == "OpFreeRef"));
        let subject_is_user_call = match subject {
            Some(Value::Set(_, rhs)) => match rhs.unspan() {
                Value::Call(fn_nr, _) => data.def(*fn_nr).is_loft_defined(),
                Value::CallRef(_, _) => true,
                _ => false,
            },
            _ => false,
        };
        subject_is_user_call && self.join_is_witnessed(val, data)
    }

    /// Is this value a `Own::Join` — borrow-or-mint, settled only per execution — whose borrow
    /// arm the @P290 bracket can NAME?
    ///
    /// That is the question deciding whether the bind following a lift is a runtime GUARD
    /// (`OpBindOrCopy`: adopt the minted arm, materialise the borrowed one) or a static bet.
    /// A lift may fire wherever the answer is yes, and must not where it is no — there the
    /// conservative no-lift costs the leak that was already there, rather than a free that
    /// protects nothing.
    ///
    /// One home, because the lift, the deps strip and both backends' `OpBindOrCopy` all read
    /// it: a second spelling of the same question could only agree by accident.
    fn join_is_witnessed(&self, val: &Value, data: &Data) -> bool {
        crate::keys::join_own_enabled()
            && matches!(
                crate::use_analysis::ownership_of(data, self.d_nr, val),
                crate::use_analysis::Own::Join { base } if base != u16::MAX
            )
    }

    /// Does EVERY arm of this `ncc` block yield a store the frame would own?
    ///
    /// ⚠ The keyed lift needs this and the reference arm does not, because for a keyed result
    /// `bl.result.depend()` comes back EMPTY even when an arm is a bare local: `f() ?? d`
    /// reads as owned and lifting it freed the caller's `d` — a use-after-free where the
    /// defect was a leak, caught by the over-free cell rather than by reading (loft#1157).
    ///
    /// An arm the @P290 bracket can NAME is a view of something a variable still holds, which
    /// is exactly the wrong thing to free; an arm it cannot name minted its own store.  The
    /// SUBJECT reaches the tail as `Var(__ncc_N)`, so its own assignment is substituted in —
    /// the temp is a name, and the question is about what the call behind it produced.
    fn ncc_arms_are_all_owned(bl: &Block, data: &Data) -> bool {
        let Some(last) = bl.operators.last() else {
            return false;
        };
        let subject = bl.operators.iter().find_map(|op| match op.unspan() {
            Value::Set(v, val) if !matches!(val.unspan(), Value::Null) => Some((*v, val.as_ref())),
            _ => None,
        });
        let mut arms: Vec<&Value> = Vec::new();
        Self::ncc_tail_arms(last, &mut arms);
        if arms.len() < 2 {
            return false;
        }
        arms.iter().all(|a| {
            let effective = match (a.unspan(), subject) {
                (Value::Var(v), Some((sv, val))) if *v == sv => val,
                _ => *a,
            };
            crate::use_analysis::view_root_slots(data, effective).is_none()
        })
    }

    /// The terminal value of each arm of an `ncc` block's tail `if`.
    fn ncc_tail_arms<'a>(val: &'a Value, out: &mut Vec<&'a Value>) {
        match val.unspan() {
            Value::If(_, t, f) => {
                Self::ncc_tail_arms(t, out);
                Self::ncc_tail_arms(f, out);
            }
            Value::Block(b) if b.operators.len() == 1 => Self::ncc_tail_arms(&b.operators[0], out),
            other => out.push(other),
        }
    }

    fn inline_struct_return(
        &self,
        val: &Value,
        data: &Data,
        outer_call: u32,
        function: &Function,
    ) -> Option<Type> {
        // loft#721 — a closure call is lifted only when its target definition is
        // known AND that definition does not return a borrowed view.
        if let Some(tp) = self.callref_owned_return(val, data, function, outer_call) {
            return Some(tp);
        }
        // loft#879 — a null-coalesce (`??`) lowers to an `ncc` value-block that
        // assigns the subject to a `__ncc_N` temp and yields either that temp or
        // the default arm's `__ref_N`.  The temp is `skip_free` (the block's
        // result ALIASES it, so freeing at the block would dangle the value the
        // consumer reads), which leaves the subject's store owned by nothing when
        // the block is used INLINE — one leaked record per evaluation,
        // unbounded in a loop.  Binding it first (`x = pick(1) ?? C{}`) has always
        // been clean because the `Set` gives the store an owner; lifting here
        // rewrites the inline spelling into exactly that bound form.
        //
        // REFERENCE results only.  A text ncc temp is already freed in place by
        // the @PLN85 skip_free-orphan pass ([`collect_consumed_ncc_text`]) and a
        // vector one by its own delivery path; both measured clean, and lifting
        // them too would free what those mechanisms free.
        // An EMPTY dep list licenses the lift because it says the value is owned.  It says
        // that for a `Call`; for a `CallRef` it says nothing.  `fnref_result_type` maps the
        // callee's return deps through the caller's ARGUMENTS and drops every index naming a
        // HIDDEN attribute, on the stated grounds that the value then arrives owned — and
        // `__closure` is a hidden attribute.  So a lambda returning a value it CAPTURED
        // hands the caller an empty dep list for a store the outer scope still owns, and
        // lifting it emits a free that reaches into that scope: the capture is released
        // while the variable it came from is still live, and the next read of it answers
        // garbage (loft#1114).
        //
        // The witnessed-`Join` route stays open to a `CallRef`, because there the bind that
        // follows is the runtime guard rather than a static bet.  Declining the other route
        // costs the leak that was already there, which is the direction this gate has always
        // taken when it cannot name what it would be freeing.
        let subject_is_call_ref = match val.unspan() {
            Value::Block(bl) if bl.name == "ncc" => {
                match bl.operators.first().map(Value::unspan) {
                    Some(Value::Set(_, rhs)) => match rhs.unspan() {
                        // Only a CAPTURING fn-ref can hand back a store the caller's scope
                        // owns; a fn-ref carrying no captures has nothing to borrow FROM,
                        // so its empty dep list means what it says and the lift stands.
                        // The fn-ref type's own deps are exactly that question, and they
                        // name the closure the call reads.
                        Value::CallRef(fn_var, _) => !matches!(
                            function.tp(*fn_var),
                            Type::Function(_, _, d) if d.is_empty()
                        ),
                        _ => false,
                    },
                    _ => false,
                }
            }
            _ => false,
        };
        if let Value::Block(bl) = val.unspan()
            && bl.name == "ncc"
            && let (Type::Reference(d_nr, dep), opt) = bl.result.peel_optional()
            && ((dep.is_empty() && !subject_is_call_ref) || self.ncc_join_is_witnessed(val, data))
        {
            return Some(Self::reopt(opt, Type::Reference(*d_nr, Deps::none())));
        }
        // loft#1157 — and the KEYED kinds, which that carve-out never named.  Its reasons are
        // PER ITEM and both are about a mechanism that exists elsewhere: text is freed in place
        // by the skip_free-orphan pass, a vector by its own delivery path.  A keyed `??` has
        // NEITHER, so used inline its subject's store is owned by nothing — one retained record
        // per evaluation, unbounded in a loop, while the bound spelling
        // (`a = f() ?? []`) was clean all along because the `Set` gives the store an owner.
        // Lifting rewrites the inline spelling into exactly that bound form.
        //
        // Same ownership gate as the reference arm: an EMPTY dep list is what says the value is
        // owned, and a capturing fn-ref subject is excluded for the reason `subject_is_call_ref`
        // gives — its empty dep list means nothing.
        if let Value::Block(bl) = val.unspan()
            && bl.name == "ncc"
            && crate::parser::vectors::is_keyed(&bl.result)
            && bl.result.depend().is_empty()
            && !subject_is_call_ref
            && Self::ncc_arms_are_all_owned(bl, data)
        {
            return Some(bl.result.without_deps());
        }
        // @P297 — a USER struct-returning call (`n_*` with a body) passed
        // directly as a call argument is wrapped in `Value::Span` by
        // `parse_call` (and re-wrapped by `scan`), so the argument reaching
        // here is `Span(Call(...))`.  Unspan before matching this branch or the
        // lift never fires and the call-result temporary leaks — the same
        // pitfall `scan_set` was patched for under @P198 (`value.unspan()`).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            // loft#879 — peel `Optional` before asking the shape question; see
            // [`Self::reopt`], which puts it back on the temp.
            let (returned, opt) = def.returned.peel_optional();
            // #549 — a generic monomorph (`t_…`) whose return SHAPE is a concrete
            // aggregate (`f<T>(x) -> (integer,integer)` / `-> Struct` / `-> Enum`)
            // leaks its result store when used inline or discarded: the caller
            // lifts+frees an `n_` aggregate return (below) but historically not a
            // `t_` one, so the fresh store the monomorph allocated via `__retbuf`
            // was orphaned (both backends).  Extend the lift to `t_` — BUT a
            // monomorph LOSES its return dep during specialization, so the
            // dep-based ownership guards here cannot tell a fresh-owned return
            // from a borrowed-arg one (`id<T>(x) -> T { x }` reads as empty-dep =
            // owned and would DOUBLE-FREE if lifted).  The reliable "delivers a
            // fresh owned aggregate" signal a monomorph keeps is the `__retbuf`
            // NRVO parameter: a concrete-aggregate return gets it at signature
            // finalization; a borrowed-reference return never does.  So gate the
            // `t_` extension on `__retbuf`, leaving every `n_` case untouched.
            // loft#1066 — a monomorph with NO `__retbuf` still delivers a fresh store when
            // its body allocated one, and nobody freed it: one leaked record per inline
            // call, N calls leaking N stores, on both backends.  `monomorph_return_is_fresh`
            // answers from the BODY the question the deps cannot — substitution gives every
            // monomorph of `-> T` the same empty dep, so `{ x }` and `{ y: T = x; y }` are
            // indistinguishable to `returns_borrowed_view` and lifting on that would double
            // free the first.  The proof is positive and under-approximating: a shape it
            // cannot read stays unlifted, which costs the leak that was already there.
            // loft#1176 — a tail that is a call THROUGH A FN-REF is such a shape read from
            // the wrong frame, and it is decided by [`Self::monomorph_fnref_return_is_fresh`]
            // ALONE, ahead of every gate below.  That ordering is the fix rather than a
            // tidy-up.  The gates below all read facts carried by THIS
            // callee's signature, and none of them can carry the caller's closure: `-> P`
            // says the same thing whether the closure mints, hands back the caller's own
            // argument, or hands back a record it CAPTURED.  Reached through the `n_` arm
            // the last of those was lifted and freed — the captured record answered
            // another value on the next read and garbage once the scope ended, on both
            // backends — while `__retbuf`'s exemption made it worse: `{ f(x) }` never
            // delivers INTO that buffer, so the premise that the lifted temp is the
            // caller's own allocation is simply false here.
            let lift_owned_return = if def.has_fnref_return_site() {
                self.monomorph_fnref_return_is_fresh(val, data, def)
            } else {
                def.name.starts_with("n_")
                    || (def.name.starts_with("t_")
                        && (def.attr_names.contains_key("__retbuf")
                            || def.monomorph_return_is_fresh()
                            // loft#1273 — a tail that DELEGATES (`a + b` is `Call(n_OpAdd)`)
                            // is a shape the callee's own body settles.
                            || Self::monomorph_delegated_return_is_fresh(data, def)))
            };
            if lift_owned_return && def.code != Value::Null {
                // The same `returns_borrowed_view()` question its struct-enum sibling below
                // asks, and for the same reason: an EMPTY return dep (or one naming only a
                // hidden work-ref) is a store the callee minted and the caller adopts, while
                // a dep naming a VISIBLE parameter is a BORROW — lifting that and freeing
                // the temp releases the caller's own record while the variable holding it is
                // still live.  A function delegating to one that borrows its argument is how
                // that is reached without any borrow appearing at the call site.
                //
                // A `__retbuf` callee is exempt, and the exemption is what the borrow means
                // there: it delivers INTO the buffer the caller allocated, so the lifted temp
                // is that buffer and freeing it releases the caller's own allocation rather
                // than the argument.  Declining for those instead orphans one buffer per
                // evaluation — measured on the dense delegating twin, which was correct
                // before this gate and has to stay correct after it.
                //
                // `returns_borrowed_view()` is the deps PROXY (@FR-O-Proxy), and it is
                // deliberately not the last word: a callee that mints into its buffer on one
                // path and returns a parameter on another carries a dep naming that
                // parameter, so the proxy calls it a borrow while the value the caller
                // actually receives is owned.  `ownership_of` is the oracle (@FR-O-Oracle)
                // and this is the chokepoint that should read it.
                //
                // `Owned` lifts.  A `Join` lifts only where the bind that follows is the
                // runtime guard — the witness has to be nameable, and the statement has to
                // be one that BINDS.  `outer_call == u32::MAX` is the bare-statement
                // lowering, where the lifted temp gets no `OpBindOrCopy` on the interpreter,
                // so the free would run on the borrow arm too and release the caller's own
                // record; there the conservative no-lift stands and costs the mint arm's
                // leak instead.  `Borrowed` never lifts.
                let own = crate::use_analysis::ownership_of(data, self.d_nr, val);
                let lift_by_oracle = match own {
                    crate::use_analysis::Own::Owned => true,
                    crate::use_analysis::Own::Join { base } => {
                        outer_call != u32::MAX && base != u16::MAX
                    }
                    crate::use_analysis::Own::Borrowed { .. } => false,
                };
                // An inline-unbound call whose result is a struct-like heap store —
                // a `Reference` or a record ENUM, the two spellings `heap_def_nr`
                // answers for — binds its result to nothing, so nothing frees it.
                // Lifting it into a `__lift_N` gives `get_free_vars` a name to emit
                // the `OpFreeRef` against.  The lifted temp keeps the spelling it
                // arrived with.
                //
                // Three ways to be the caller's to free, and the second is the one a
                // dep list reads backwards:
                //   - EMPTY dep (`fn mk() -> H { Bytes{…} }`) — fresh, owned.
                //   - a dep naming the HIDDEN `__ref_N`/`__retbuf` the callee
                //     delivered through.  That reads as a borrow and is not one: the
                //     buffer is the CALLER's own allocation, so the lift's copy-path
                //     free (`0x8000` source-free) claims it exactly as the bound
                //     `h = f()` case does.  Declining here orphans one store per
                //     evaluation (#490 kt=65 on native, loft#1202 on both backends).
                //   - the ORACLE says owned where the deps proxy cannot (@FR-O-Oracle).
                // A dep naming a VISIBLE parameter (`fn field_of_arg(d) -> H { d.value }`)
                // IS a borrow: lifting it would dangle the caller's own argument.
                //
                // ⚠ Asked ONCE for both spellings on purpose.  These were two arms, and
                // the record-enum one carried only `!returns_borrowed_view()` — so a
                // struct-enum callee delivering through a `__retbuf` fell through the
                // second bullet above and leaked, while its `Reference` twin did not.
                if returned.heap_def_nr().is_some()
                    && (!def.returns_borrowed_view()
                        || def.attr_names.contains_key("__retbuf")
                        || lift_by_oracle)
                {
                    return Some(Self::reopt(opt, returned.with_deps(&Deps::none())));
                }
                // Plan-57: a user fn returning a CAPTURING closure (`fn(...) -> T`
                // whose fn-ref carries a fresh closure record) leaks its result temp
                // when used directly as a call argument — `apply(make())` left the
                // `__closure_*` record at rc 1 and the cell uncollected.  Lift it like
                // the Reference / struct-enum cases so `get_free_vars`' fn-ref arm
                // emits the `OpFreeRef`; codegen frees the closure DbRef at offset+8
                // and `free_named` cascades to the captured `__cell_*`.  NOT guarded on
                // `dep.is_empty()` — a capturing closure's dep IS the cell (e.g.
                // `function([], integer, [1])`), so guarding would skip the very case.
                // A non-capturing return carries the null closure sentinel → the free
                // is a safe no-op; a borrowed fn-ref copy is marked `skip_free`
                // elsewhere, so only a freshly produced closure is lifted here.
                if let Type::Function(params, ret, _) = returned {
                    return Some(Self::reopt(
                        opt,
                        Type::Function(params.clone(), ret.clone(), Deps::none()),
                    ));
                }
            }
            // loft#792 — the same lift for a body-less NATIVE global that MINTS a
            // record.  `type_of(x)` / `type_named(n)` lower to `n_reflect_type` /
            // `n_type_named`, which allocate a `TypeInfo` store; passed straight as a
            // call argument the value was bound to nothing, so nothing freed it.
            // Binding it to a local first leaked nothing, which is what made this read
            // as a reflection quirk rather than the missing lift it is.  Native frees
            // the record through its own drop path, so the leak was interpreter-only —
            // and it CASCADES: with a callee that returns a struct holding a freshly
            // built vector, that vector leaked once per call after the first, so a loop
            // calling `f(type_of(x))` grew the heap without bound.
            //
            // The bound is what keeps this sound.  A native has no body to read, so
            // lift only where the answer cannot be anything but a fresh record: the
            // return names a concrete STRUCT, carries no dep, and no parameter has a
            // type that could have supplied one.  That excludes every view-returning
            // native in the stdlib — `hash_sorted(h: reference, …) -> reference` and
            // `parallel_buf_get_ref(i) -> reference` both hand back a borrow, and both
            // return the untyped `reference` rather than a named struct.  The
            // `JsonValue` constructors are struct-ENUMs and keep their own arm above.
            if lift_owned_return
                && def.code == Value::Null
                && let Type::Reference(d_nr, dep) = returned
                && dep.is_empty()
                && data.def_type(*d_nr) == DefType::Struct
                && !def
                    .attributes()
                    .iter()
                    .any(|a| matches!(a.typedef.base(), Type::Reference(p, _) if p == d_nr))
            {
                return Some(Self::reopt(opt, Type::Reference(*d_nr, Deps::none())));
            }
        }
        // @P393 (t9) — a loft-source fn OR `t_` method returning an OWNED vector
        // BY VALUE (empty dep) is de-NRVO'd when its body builds the result with
        // >=2 distinct element-temps (`01a3f24f` in `ref_return`): the signature
        // is `n_f() -> vector<T>` / `t_..split() -> vector<text>`, with no hidden
        // `__vdb`/`__ref` buffer param.  Used inline-unbound (`len(split(x))`,
        // `split(x).join(y)`) the by-value temp gets no scope-exit free on the
        // interpreter → its store leaks (native codegen already frees it).  Lift
        // it like the Reference / struct-Enum / Function cases above so
        // `get_free_vars` emits the `OpFreeRef`.  A SEPARATE block from the
        // `n_`-gated one above because `split` is a `t_` method — broadening that
        // block's guard would also change the Reference/Enum/Function arms' scope.
        // Gated on `dep.is_empty()`: the NRVO'd hidden-param return carries dep
        // `["??"]` (caller already frees `__ref`) and a borrowed view carries
        // `[self]`, so both are excluded — no double-free, no UAF.  `val.unspan()`
        // because `parse_call`/`parse_method` Span-wrap the call (arg AND
        // receiver = arg0).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            if def.is_loft_defined() {
                // loft#879 — peel `Optional` for the match, restore it on the temp
                // ([`Self::reopt`]).  The dep guards keep reading the BASE's dep,
                // which is where a borrowed view records itself.
                let (returned, opt) = def.returned.peel_optional();
                match returned {
                    Type::Vector(elem, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Vector(elem.clone(), Deps::none())));
                    }
                    // @PLN85 p188 — a discarded (or inline-unbound) owned KEYED
                    // collection return (`build() -> sorted<T[k]>`, and the
                    // index/hash/spatial siblings) leaks its by-value store exactly
                    // like the vector case above — the `Drop` lift binds it to a
                    // `__lift_N` temp so `get_free_vars` emits the store's
                    // `OpFreeRef` (both backends).  Empty dep = OWNED (fresh); a
                    // borrowed view / NRVO'd hidden-buffer return carries a
                    // non-empty dep and is excluded (no double-free / UAF).
                    Type::Sorted(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Sorted(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Index(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Index(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Hash(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Hash(*d, keys.clone(), Deps::none())));
                    }
                    Type::Radix(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Radix(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Trie(d, key, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Trie(*d, key.clone(), Deps::none())));
                    }
                    _ => {}
                }
            }
        }
        // Native-constructor calls arrive BARE when chained onto a builtin
        // (`v.keys().len()`) and Span-wrapped when passed as a call argument or
        // method receiver (`jt(json_parse(x), n)`, `json_parse(x).field(n)`), so
        // match through the span — an unlifted native-constructor temp owns a
        // fresh store nothing ever frees (#490).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            // loft#879 — peel `Optional` for the match, restore it on the temp
            // ([`Self::reopt`]).
            let (returned, opt) = def.returned.peel_optional();
            // Native struct-enum constructors: no body (code == Null), return type
            // is a struct-enum with empty dep (allocates a new store, doesn't borrow).
            // Accessors carry dep=[0] after parser dep-inference and are skipped here.
            if def.code == Value::Null
                && let Type::Enum(d_nr, true, dep) = returned
                && dep.is_empty()
            {
                return Some(Self::reopt(opt, Type::Enum(*d_nr, true, Deps::none())));
            }
            // Native vector-returning fns (e.g. `keys()`, `fields()` on
            // JsonValue) allocate a fresh vector store that the caller owns.
            // Without lifting, the chained call `v.keys().len()` leaks the
            // intermediate vector — same mechanism as the struct-return case.
            if def.code == Value::Null
                && let Type::Vector(elem, dep) = returned
                && dep.is_empty()
            {
                return Some(Self::reopt(opt, Type::Vector(elem.clone(), Deps::none())));
            }
        }
        None
    }
}

/// Does a variable of this type need its slot established at the PARENT scope
/// when an `if`/`else` assigns it in a branch?
///
/// The question is really "is this variable backed by a heap store".  One that
/// is cannot be treated like a scalar written in both arms: `scan_if` registers
/// such a scalar at the parent scope directly (`small_both`), and for a
/// store-backed variable that hoists the OWNERSHIP without ever creating the
/// store, so the scope-exit free meets a stack-record ref where it expects an
/// owned heap store.
///
/// The KEYED collections were missing, and every one of them crashed the
/// interpreter for it: two arms of one `if`/`else` chain declaring the same
/// `hash` / `sorted` / `index` name gave `BUG (#306): a stack-record ref was
/// treated as an owned heap store`, then a SIGSEGV once the branch appended.
/// `vector` was in the list and so was fine, which is why the fault looked
/// type-specific rather than like the omission it was.  `--native` computes the
/// answer separately and was always right, so nothing outside the interpreter
/// changed.
fn needs_pre_init(tp: &Type) -> bool {
    // Through `base()`: a nullable `S?` / `vector<T>?` / `text?` local is the same slot
    // behind a nullability marker (`@FR-L-Null`), and it needs the same initialisation —
    // the null it holds on the path that never assigned it.  Matching the bare spelling
    // left the nullable twin with none: first assigned inside a branch, the second arm's
    // `Set` was a REASSIGNMENT whose guarded displacement free read an uninitialised
    // slot (a refused free of `0xDEADBEEF`, or the free of whatever live store the
    // previous frame left there); first assigned inside a loop body, it stayed scoped
    // to the body and the read after the loop was a use-after-free on the interpreter
    // and an unresolved `var_x` under rustc.  Both backends, every nullable kind.
    matches!(
        tp.base(),
        Type::Text(_) | Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
    ) || crate::parser::vectors::is_keyed(tp.base())
}

/// After a return value has been COPIED — into the caller's hidden `&text` buffer, or into
/// a `__ret_N` temp — the frame-local text temps it read are dead: the caller consumes
/// the copy, not them.  Two kinds are otherwise never freed on this path, because each
/// was suppressed on the premise that the return would TRANSFER it rather than copy it:
///
/// - a `__work_N` text that `wrap_value_text_dest` minted for a callee to fill — as the
///   return's terminal it is skipped by `get_free_vars` (`ret_var`);
/// - a `__ncc_N` null-coalesce temp, `skip_free` so the present-path Str outlives its
///   block; a NON-tail consumer frees it in place (`collect_consumed_ncc_text`), and a
///   return that copies is exactly such a consumer.
///
/// Emit the frees HERE, at the copy, so they fire only when a copy actually happened; the
/// direct-transfer path (fast-path `Return(Var(__work_N))`, no copy) reaches neither this
/// nor a free and correctly leaves the buffer for the caller.  An argument is the
/// caller's.  A user local is freed by the scope sweep already — EXCEPT the one the
/// return names: the sweep suppresses the returned variable on the premise that it is
/// handed up, and once its bytes are copied into the buffer that premise is false, so a
/// `return ta` delivered through the buffer frees `ta` here too (loft#1357; a lambda
/// holding its one buffer for `tb` returned `ta` as a view of an orphan).
fn free_copied_text_sources(
    result: &mut Vec<Value>,
    expr: &Value,
    pending: &[Value],
    function: &Function,
    data: &Data,
) {
    let mut srcs = Vec::new();
    collect_return_sources(expr, data, &mut srcs);
    for w in srcs {
        if !matches!(function.tp(w).base(), Type::Text(_)) || function.is_argument(w) {
            continue;
        }
        // A source the scope-exit sweep already releases is not drained twice: a
        // multi-arm value has no single returned var for `get_free_vars` to suppress,
        // so a `__work_N` behind a formatted-string ARM sits in `pending` as well.
        if pending
            .iter()
            .any(|op| scope_free_op_var(op, data) == Some(w))
        {
            continue;
        }
        let n = function.name(w);
        if n.starts_with("__work_")
            || (n.starts_with("__ncc_") && function.is_skip_free(w))
            || (!function.is_skip_free(w) && !matches!(function.tp(w), Type::RefVar(_)))
        {
            result.push(call("OpFreeText", w, data));
        }
    }
}

/// ANY hidden `&text` return buffer of `d_nr`, read by the value or not — the destination a
/// STAGED return moves into (`text_return_buffer_for` is the direct-write question, which
/// must exclude a buffer the value reads).  `None` = the function holds no buffer at all.
fn any_text_return_buffer(function: &Function, data: &Data, d_nr: u32) -> Option<u16> {
    data.def(d_nr)
        .attributes()
        .iter()
        .filter(|a| {
            a.hidden && matches!(&a.typedef, Type::RefVar(t) if matches!(**t, Type::Text(_)))
        })
        .map(|a| function.var(&a.name))
        .find(|&v| v != u16::MAX)
}

/// The hidden `&text` return buffer of `d_nr` that `expr` does not read — the caller-owned
/// destination an owned text return is delivered through (@FR-F-Ret), as a variable of
/// the function's own frame.
///
/// A function holds one such buffer per promotion its body asked for (`text_return`: the
/// block tail's accumulator, a formatted early return's work text, a built local the tail
/// names), and any of them is a valid destination for a return — the call is leaving, so
/// nothing this frame does with the buffer afterwards matters — EXCEPT one the value
/// itself reads: writing `"{x}-{n}"` into `x` clears `x` before it is rendered.  Only a
/// HIDDEN buffer qualifies; a user-written `&text` parameter is the caller's variable, and
/// a return must not overwrite it.  `None` = the function has no buffer to deliver
/// through, which is the `__ret_N` residual.
fn text_return_buffer_for(
    expr: &Value,
    function: &Function,
    data: &Data,
    d_nr: u32,
) -> Option<u16> {
    let mut read = HashSet::new();
    expr.walk(&mut |v| {
        if let Value::Var(x) = v {
            read.insert(*x);
        }
    });
    data.def(d_nr)
        .attributes()
        .iter()
        .filter(|a| {
            a.hidden && matches!(&a.typedef, Type::RefVar(t) if matches!(**t, Type::Text(_)))
        })
        .map(|a| function.var(&a.name))
        .find(|&v| v != u16::MAX && !read.contains(&v))
}

/// Does this reassignment displace a store the function OWNS through a callee that
/// mints its own?
///
/// The shape is `v = f(…, v, …)` with `v` sitting at `f`'s hidden return-buffer
/// attribute — the NRVO hand-off that lets a callee build its result straight into
/// the destination instead of into a work-ref of its own.  The hand-off is only
/// taken when `f` actually delivers through that buffer.  A callee whose return
/// ADOPTS a fresh store (`Definition::return_adopts_fresh_store`, the carried
/// adopt-vs-copy fact) allocates for itself and never reads the buffer, so the store
/// `v` held before the call is displaced and unreachable.
///
/// Answering "yes" only licenses the free; whether the displaced store is this
/// function's to release is a separate question that @FR-O-Latest answers (the caller
/// checks `owned_refs` first).  `v` must not be read anywhere ELSE in the call — a
/// pre-Set free would then destroy data the call still reads.
/// The hidden return-buffer PARAMETER of `d_nr`, as a variable number — or `None` when the
/// function has none.
///
/// Which attribute is the buffer is `Definition::hidden_return_buffer_attr`'s question; this
/// resolves it to the slot the body actually assigns.
fn hidden_return_buffer_var(d_nr: u32, function: &Function, data: &Data) -> Option<u16> {
    let def = data.def(d_nr);
    let idx = def.hidden_return_buffer_attr()?;
    let name = def.attributes().get(idx)?.name.clone();
    let v = function.var(&name);
    (v != u16::MAX).then_some(v)
}

/// Which heap LOCALS are assigned a BORROW on one path and an owned value on another?
///
/// The single ownership fact such a binding carries cannot be right for both.  Its `deps` come
/// out EMPTY — the owned arm contributes none and the join drops the borrow's — so @FR-O-Proxy
/// answers "owned" and the displacement free releases a store the borrow arm only borrowed.
/// Measured: `if c { r = field_of(q) } else { r = fnref(7) }` in a loop freed `q`'s store on
/// the iteration that took the other arm, on both backends (loft#1333).
///
/// **A FOREIGN base is the borrow that counts, not any non-owned verdict.** `x: vector<τ> = []`
/// followed by `x = fnref(i)` also reads as non-owned on its first assignment, and there the
/// base is the `__vdb_N` that very literal minted — storage `x` is the sole user of, released
/// by that temp's own scope-exit free.  Marking such a binding withdraws the strip loft#1329
/// needs, and every cell of its guard then exhausted the store table.  What separates them is
/// whether the base is the binding's own literal backing
/// ([`crate::variables::owns_literal_backing_store`]) or storage reached through somebody
/// else — a call's return buffer carrying a view of the callee's argument, which is the c3i
/// shape this closes.
///
/// **Why MIXED and not merely viewed.** A local every path views already carries a dep and
/// declines the free on its own; it is the disagreement that produces the empty list.  Keeping
/// the condition to mixed bindings is what stops this from suppressing frees that are correct.
fn mixed_ownership_locals(code: &Value, function: &Function, data: &Data, d_nr: u32) -> Vec<u16> {
    let mut viewed: HashSet<u16> = HashSet::new();
    let mut owned: HashSet<u16> = HashSet::new();
    fn walk(
        node: &Value,
        data: &Data,
        d_nr: u32,
        function: &Function,
        viewed: &mut HashSet<u16>,
        owned: &mut HashSet<u16>,
    ) {
        if let Value::Set(t, val) = node.unspan() {
            match crate::use_analysis::ownership_of(data, d_nr, val) {
                crate::use_analysis::Own::Owned => {
                    owned.insert(*t);
                }
                crate::use_analysis::Own::Borrowed { base }
                | crate::use_analysis::Own::Join { base } => {
                    // A base that is the binding's OWN literal backing is not a foreign
                    // borrow: `x: vector<τ> = []` lowers to a read of the `__vdb_N` this very
                    // literal minted, and that temp's own scope-exit free releases it.  Any
                    // other base is storage reached through somebody else — a call's return
                    // buffer carrying a view of the callee's argument, or a named local.
                    if base != u16::MAX
                        && (base as usize) < function.count() as usize
                        && !crate::variables::owns_literal_backing_store(function.name(base))
                    {
                        viewed.insert(*t);
                    }
                }
            }
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, data, d_nr, function, viewed, owned));
    }
    // The fact is read at ONE site — `callref_delivers_collection`'s strip — so a body with no
    // fn-ref call cannot need it, and the oracle walk below is not free: it asks
    // `ownership_of` per assignment, and a big vector literal is thousands of them
    // (`issue854_a_vector_literal_compiles_in_linear_time` went from seconds to over a minute
    // before this gate).  The structural pre-check is what keeps the cost on the bodies that
    // can actually be wrong.
    if !contains_callref(code) {
        return Vec::new();
    }
    walk(code, data, d_nr, function, &mut viewed, &mut owned);
    let mut out: Vec<u16> = viewed.intersection(&owned).copied().collect();
    out.sort_unstable();
    // ⚠ Deliberately NOT filtered on an empty dep list.  This runs BEFORE the scan, where the
    // view arm's dep is still on the binding — the empty list is what the scan PRODUCES and
    // what this exists to prevent, so testing for it here would drop every var that matters.
    out.retain(|&v| {
        v < function.count()
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
            )
            && !function.is_argument(v)
    });
    out
}

/// Which nullable heap-record LOCALS are reassigned from a call that mints?
///
/// The pre-scan answer to *"is a runtime ownership witness worth a slot here?"*, asked once
/// per function in the shape [`displaces_return_buffer`] already uses.  A body that reaches no
/// such site pays nothing.
///
/// **Why a witness and not a predicate.** A nullable RECORD return gets no delivery buffer —
/// `-> S?` is a synthetic `__nullable<S>` carrying its own delivery, and giving it a buffer as
/// well leaks one record per call — so every call MINTS and the caller owes the release of
/// what it displaces.  A static free at the reassignment cannot be placed, because the local's
/// FIRST store is normally an inline mint into a work-ref (`c: S? = S { x: 5 }` lowers to
/// `c = { Object -> __ref_p2_1 }`): the local and that work-ref name ONE store, and freeing
/// through the local double-frees it against the work-ref's own scope-exit free.  One static
/// site cannot separate the first iteration from the rest, which is what `formal/ownership.md`
/// D-own-16 records; the flag answers it per RUN.
/// Which nullable heap-record LOCALS hold a PROJECTION VIEW on some assignment — a field
/// read (`d = q.inner`), a vector-element read (`d = vs[i]`), a tuple element — a store the
/// local only borrows and never owns (@FR-O-Owner)?
///
/// Such a local is marked never-free (@FR-O-Override).  Its single-ARGUMENT dep otherwise
/// reads as ownership at the D-own-16 `borrows_one_argument` residual (`state/codegen.rs`,
/// this file's scope-exit `borrow_witness`, `generation/dispatch.rs`), which then frees the
/// store the local DISPLACES at a reassignment — the caller's nested store, or a local's
/// field — a store the local only VIEWED.  A view owns nothing, so the proxy that licenses
/// that free is wrong and @FR-O-Override vetoes it.
///
/// A DIRECT projection — a field read (`OpGetField`), a vector-element read (`OpGetVector`
/// &c) or a tuple element — ALIASES its base (@FR-B-View / @FR-B-View-Depth): the local
/// holds a store it does not own.  A whole-value bind of a heap variable COPIES (@FR-B-Copy,
/// pE) and a CALL that returns a borrowed view is COPIED into the local by the set-lowering
/// (@FR-F-Ret, loft#1346) — both mint the local a store of its own, so neither is matched
/// here; the set is exactly the ops in [`crate::use_analysis`]'s projection set plus a tuple
/// read.  The mixed-ownership shapes that own a store are excluded by the caller: a
/// solely-owned minting call by the loft#1200 runtime flag ([`nullable_locals_that_displace`]),
/// a view+mint mix by the owner witness ([`owner_witness_locals`], loft#1336), and a
/// MATERIALISED view (its container is disturbed while it is live, so it takes its own copy,
/// @FR-B-View) by `views_to_materialise`.
fn nullable_view_locals(code: &Value, function: &Function, data: &Data) -> Vec<u16> {
    // Cost gate: restrict the walk to bodies that actually declare a nullable heap-record
    // local (the only thing this marks).
    let has_candidate = (0..function.count()).any(|v| {
        matches!(function.tp(v), Type::Optional(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && !function.is_argument(v)
            && !function.is_captured(v)
    });
    if !has_candidate {
        return Vec::new();
    }
    let projections = &data.op_sets().projections;
    let mut out: Vec<u16> = Vec::new();
    let mut walk = |node: &Value| {
        if let Value::Set(t, val) = node.unspan()
            && !out.contains(t)
            // A DIRECT projection that ALIASES: a tuple element, or one of the projection
            // reads.  NOT a bare `Var` (copies, @FR-B-Copy) and NOT a user/native call
            // returning a borrow (copied into the local, @FR-F-Ret / loft#1346).
            // A tagged slot read through its tag (`if <present> { <projection> } else {
            // nullref }`, the bind of `x = o.opt`) is the projection its present arm is.
            && match crate::use_analysis::through_null_arm(data, val).unspan() {
                Value::TupleGet(_, _) => true,
                Value::Call(fn_nr, _) => projections.contains(fn_nr),
                _ => false,
            }
        {
            out.push(*t);
        }
    };
    code.walk(&mut |n| walk(n));
    out.retain(|&v| {
        v < function.count()
            && matches!(function.tp(v), Type::Optional(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && !function.is_argument(v)
            && !function.is_captured(v)
            && !function.is_compiler_generated(v)
    });
    out
}

fn nullable_locals_that_displace(code: &Value, function: &Function, data: &Data) -> Vec<u16> {
    fn walk(node: &Value, seen: &mut HashSet<u16>, out: &mut Vec<u16>, data: &Data) {
        if let Value::Set(t, val) = node.unspan() {
            // A SECOND assignment is what displaces; the first allocates.
            if !seen.insert(*t)
                && mints_a_store_the_target_does_not_hold(val, *t, *t, data)
                && !out.contains(t)
            {
                out.push(*t);
            }
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, seen, out, data));
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    walk(code, &mut seen, &mut out, data);
    // @FR-O-Proxy asks free — the locals this returns are the ones whose DISPLACED store is
    // released, so the proxy's answer is what licenses that free.
    out.retain(|&v| {
        v < function.count()
            && matches!(function.tp(v), Type::Optional(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && function.tp(v).depend().is_empty()
            && !function.is_skip_free(v)
            && !function.is_argument(v)
    });
    out
}

/// Which heap-record LOCALS have assignments that MIX ownership — at least one that hands
/// the local a store of its own and at least one that hands it a view?
///
/// The pre-scan answer to *"is an OWNER WITNESS worth a slot here?"* (loft#1336,
/// `@FR-O-Witness`), asked once per function like [`nullable_locals_that_displace`].
///
/// **Why a witness and not the dep list.** A binding carries ONE dep list, flow-insensitively,
/// and it records whichever assignment parsed LAST: `cur: Node? = a; cur = cur.next` leaves
/// `cur` reading as a borrow for the whole frame, so the store the copy minted is released by
/// nobody — and the inverse order leaves it reading as an owner while it holds a view, so the
/// view's record is freed as if it were the local's.  Neither static answer is right for both
/// assignments; the witness answers per RUN, by store identity.
///
/// A local with only owning assignments keeps the static free placement (the proxy is right
/// for it), and one with only views has nothing to release.  Excluded on purpose: a
/// parameter (its entry stash is the witness, `Function::rebind_orig`), a captured local (a
/// closure reads the capture-time `DbRef`, @FR-L-CapHeap — the record takes over the free), a
/// loop variable (bound by `Iter`, not by a `Set`), and the compiler's own temporaries.
fn owner_witness_locals(
    code: &Value,
    function: &Function,
    data: &Data,
    d_nr: u32,
    materialised_views: &HashMap<u16, Disturbance>,
) -> Vec<u16> {
    let mut defs: Option<crate::use_analysis::Defs> = None;
    let mut minted: HashSet<u16> = HashSet::new();
    let mut viewed: HashSet<u16> = HashSet::new();
    fn walk(
        node: &Value,
        function: &Function,
        data: &Data,
        d_nr: u32,
        defs: &mut Option<crate::use_analysis::Defs>,
        minted: &mut HashSet<u16>,
        viewed: &mut HashSet<u16>,
    ) {
        if let Value::Set(t, val) = node.unspan()
            && (*t as usize) < function.count() as usize
            && !function.name(*t).starts_with("__")
            && !function.is_argument(*t)
            && !function.is_captured(*t)
            && !function.was_loop_var(*t)
            && !matches!(function.tp(*t), Type::RefVar(_))
            && function.tp(*t).base().heap_def_nr().is_some()
        {
            match witness_set_kind(val, *t, *t, function, data, d_nr, &mut |v| {
                let defs =
                    defs.get_or_insert_with(|| crate::use_analysis::function_defs(data, d_nr));
                crate::use_analysis::ownership_of_with(data, d_nr, v, defs)
            }) {
                WitnessSet::Mint | WitnessSet::MintReading => {
                    minted.insert(*t);
                }
                WitnessSet::Other => {
                    // A view of another variable's storage.  A null or a store nobody names
                    // is not a VIEW and does not make the ownership mixed on its own.
                    if is_view_of_storage(val, data) {
                        viewed.insert(*t);
                    }
                }
            }
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, function, data, d_nr, defs, minted, viewed));
    }
    walk(
        code,
        function,
        data,
        d_nr,
        &mut defs,
        &mut minted,
        &mut viewed,
    );
    // @FR-O-Latest / @FR-O-Witness — a CAPTURED local reassigned after its build has mixed
    // ownership too, and between the same two parties the witness exists for: the closure
    // record owns the store the capture named AT THE BUILD, the frame owns every store the
    // local is given afterwards.  No static reading is right for both — `(O-Latest)` says a
    // type-level `deps` list can express neither which assignment nor the loop depth — so the
    // release has to be by STORE IDENTITY, which is exactly what this witness does.  Without
    // one, `owns_displaced_store`'s per-BINDING `!is_captured` veto is asked at two sites that
    // are statically identical and is right at only one of them (loft#1388).
    let builds = capture_build_backings(data, function, code);
    for &v in &builds.reassigned_after_build {
        // Both spellings of "the record took this local's store": a STRUCT capture names the
        // local outright, a COLLECTION capture names a view whose backing local is this one.
        // The backing needs the witness for the same reason and, once the record's own
        // release is emitted (`fn_ref_with_closure`'s pre-build free), for a sharper one: a
        // local left without one is still owned by the FRAME, so the two would free the same
        // store between them — measured as the vector loop answering `1,2` where `4,5` is
        // right, on `--native` alone.
        if (function.is_captured(v) || builds.backing.values().any(|b| *b == v))
            && !function.is_argument(v)
            && !function.name(v).starts_with("__")
            && !function.was_loop_var(v)
            // RECORD kinds only, and that boundary is measured rather than assumed.  A vector
            // witness was built — a `Vector`-typed `__own_` handle, admitted here and released
            // by the same identity guard — and it answers WRONG: the vector-capture loop read
            // `1,2` where `4,5` is right, on `--native`, because the witness releases a store
            // the record still holds.  A leak is the better trade, so the vector-capture loop
            // keeps one store (values right on both backends) until the release the record
            // owes is decided per SLOT rather than per local.
            && function.tp(v).base().heap_def_nr().is_some()
            && !materialised_views.contains_key(&v)
        {
            minted.insert(v);
            viewed.insert(v);
        }
    }
    let mut out: Vec<u16> = minted
        .intersection(&viewed)
        .copied()
        .filter(|v| !materialised_views.contains_key(v))
        .collect();
    out.sort_unstable();
    out
}

/// Does this value bind a VIEW of storage some other binding owns — a projection, a
/// call answering a borrow, a join?  The `viewed` half of [`owner_witness_locals`].
fn is_view_of_storage(value: &Value, data: &Data) -> bool {
    match value.unspan() {
        Value::Null => false,
        Value::Call(nr, args) if args.is_empty() && data.def(*nr).name() == "OpNullRefSentinel" => {
            false
        }
        // A whole-value copy of a heap record never views (@FR-B-Copy).
        Value::Var(_) => false,
        Value::Call(_, _)
        | Value::CallRef(_, _)
        | Value::Block(_)
        | Value::Insert(_)
        | Value::TupleGet(_, _) => true,
        // A tagged slot read through its tag (`Parser::emit_nullable_slot_read`, the bind
        // of `x = o.opt` under `@FR-L-Null-Which`) answers as its present arm does.
        Value::If(_, _, _) => {
            let seen = crate::use_analysis::through_null_arm(data, value);
            !matches!(seen.unspan(), Value::If(_, _, _)) && is_view_of_storage(seen, data)
        }
        _ => false,
    }
}

/// What a `Set` hands a witnessed local (see [`owner_witness_locals`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WitnessSet {
    /// The local MINTS a store of its own and the value does not read the local.
    Mint,
    /// The local MINTS a store of its own from a value that READS it, so the store it
    /// displaces must stay live until the value is computed (@FR-O-Detach).
    MintReading,
    /// A view, a join, a null, or a store somebody else owns: the local stops owning.
    Other,
}

/// Classify the value assigned to witnessed local `v` (`ov` is its pre-scan id).
///
/// MINTS means *both emitters give the local a store of its own here*: a whole-value copy of
/// another heap variable (@FR-B-Copy, `gen_set_first_ref_var_copy` / native's record-copy
/// arm); a loft-defined callee whose return does NOT adopt a fresh store, which both emitters
/// deep-copy into the local (`state/codegen.rs`'s call-copy arm, `generation/dispatch.rs`'s
/// `OpCopyRecord` arm); a value the oracle calls `Owned` — a minting call, an inline mint —
/// unless that mint lands in a work-ref that frees it itself (the `c = { Object → __ref_N }`
/// literal, whose store is the work-ref's and not solely the local's).
///
/// A PROJECTION is never a mint here, whatever its deps: @FR-B-View's materialise clause
/// (a view live across a reshape of its container) is decided from the binding's FINAL dep
/// list at codegen, which one `Set` cannot see mid-scan, so a witnessed local is kept out of
/// that clause altogether — [`owner_witness_locals`] excludes every binding
/// `collect_views_to_materialise` names, and both emitters decline the materialise arm for a
/// witnessed local.  The two mechanisms never meet on one binding.
///
/// The fallback is `Other`, and it is the SAFE direction: a value this does not recognise as
/// a mint is treated as a view, so the witness is released where the local stops naming it
/// and never pointed at a store the local does not own — a retained store is recoverable, a
/// premature free is not (@FR-O-Oracle's rule for an unnameable base).
fn witness_set_kind(
    value: &Value,
    v: u16,
    ov: u16,
    function: &Function,
    data: &Data,
    d_nr: u32,
    own: &mut impl FnMut(&Value) -> crate::use_analysis::Own,
) -> WitnessSet {
    let reads = value.reads_var(v) || value.reads_var(ov);
    let mints = match value.unspan() {
        Value::Var(src) => {
            *src != v
                && *src != ov
                && (*src as usize) < function.count() as usize
                && function.tp(*src).base().heap_def_nr().is_some()
        }
        Value::Call(nr, args) if args.is_empty() && data.def(*nr).name() == "OpNullRefSentinel" => {
            false
        }
        Value::Call(_, _) | Value::CallRef(_, _) => {
            let copied_by_both =
                crate::use_analysis::callee_of(data, d_nr, value).is_some_and(|fn_nr| {
                    data.def(fn_nr).is_loft_defined()
                        && !data.def(fn_nr).return_adopts_fresh_store()
                });
            copied_by_both || matches!(own(value), crate::use_analysis::Own::Owned)
        }
        Value::Block(b) => {
            let into_work_ref = b.operators.last().is_some_and(|last| {
                matches!(last.unspan(), Value::Var(r)
                    if (*r as usize) < function.count() as usize
                        && (function.name(*r).starts_with("__ref_")
                            || function.name(*r).starts_with("__rref_")))
            });
            !into_work_ref && matches!(own(value), crate::use_analysis::Own::Owned)
        }
        Value::Insert(_) => matches!(own(value), crate::use_analysis::Own::Owned),
        _ => false,
    };
    if !mints {
        WitnessSet::Other
    } else if reads {
        WitnessSet::MintReading
    } else {
        WitnessSet::Mint
    }
}

/// The captures whose closure build sits inside a LOOP.
///
/// A build that runs once adopts one store and keeps it; a build in a loop rewrites its
/// record's capture slot on every pass, so only the LAST adoption is the one the record still
/// holds and every earlier backing is the frame's to free again.
fn captures_built_in_a_loop(node: &Value, set_dbref: u32, in_loop: bool, out: &mut HashSet<u16>) {
    let inner = in_loop || matches!(node.unspan(), Value::Loop(_));
    if in_loop
        && let Value::Call(d, args) = node.unspan()
        && *d == set_dbref
        && let Some(Value::Var(c)) = args.get(2).map(Value::unspan)
    {
        out.insert(*c);
    }
    node.unspan()
        .for_each_child(&mut |ch| captures_built_in_a_loop(ch, set_dbref, inner, out));
}

/// The capture variables a value's closure BUILDS write into their record.
///
/// The record owns the store each names from the build on (`free_named`'s cascade is its
/// releaser), so a capture with an owner witness gives that store up there.  Read off the
/// VALUE rather than the statement, because an INLINE closure argument builds inside the very
/// assignment that moves the local on.
fn captures_built_in_value(value: &Value, data: &Data) -> Vec<(u16, u16)> {
    let set_dbref = data.def_nr("OpSetDbRef");
    fn walk(node: &Value, set_dbref: u32, out: &mut Vec<(u16, u16)>) {
        if let Value::Call(d, args) = node.unspan()
            && *d == set_dbref
            && let (Some(Value::Var(rec)), Some(Value::Var(c))) = (
                args.first().map(Value::unspan),
                args.get(2).map(Value::unspan),
            )
        {
            out.push((*rec, *c));
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, set_dbref, out));
    }
    let mut out = Vec::new();
    walk(value, set_dbref, &mut out);
    out
}

/// Release the store an owner witness names and reset the witness to the sentinel — as ONE
/// unit, because `OpFreeRef` of a variable does not reset its slot on the interpreter, and a
/// witness left naming a freed store would release whatever the allocator hands that slot
/// next.
fn release_witness(w: u16, data: &Data) -> Value {
    Value::Insert(vec![
        call("OpFreeRef", w, data),
        v_set(w, Value::Call(data.def_nr("OpNullRefSentinel"), vec![])),
    ])
}

/// Point owner witness `w` at the store local `v` now holds — an ALIAS, where a plain
/// `Set(w, Var(v))` would copy the record (@FR-B-Copy).
fn witness_points_at(w: u16, v: u16, data: &Data) -> Value {
    v_set(
        w,
        Value::Call(data.def_nr("OpRefAlias"), vec![Value::Var(v)]),
    )
}

/// Does this call hand back a store the target does NOT already hold?
///
/// The complement of [`displaces_owned_through_fresh_callee`]'s NRVO shape.  There the target
/// sits at the callee's return-buffer attribute, so the callee fills the store the target
/// already owns and nothing is displaced.  Here the callee MINTS: it has no return buffer at
/// all — which is every nullable RECORD return — or it was handed someone else's.
///
/// The target must not be READ anywhere in the call: the free is emitted before the
/// assignment, so a call that still reads the old value would be handed a freed store.
fn mints_a_store_the_target_does_not_hold(value: &Value, v: u16, ov: u16, data: &Data) -> bool {
    let Value::Call(fn_nr, args) = value.unspan() else {
        return false;
    };
    let def = data.def(*fn_nr);
    if !def.is_loft_defined() || !def.return_adopts_fresh_store() {
        return false;
    }
    let names_v = |x: &Value| matches!(x.unspan(), Value::Var(w) if *w == v || *w == ov);
    if def
        .hidden_return_buffer_attr()
        .is_some_and(|i| args.get(i).is_some_and(names_v))
    {
        return false;
    }
    args.iter().all(|a| !a.reads_var(v) && !a.reads_var(ov))
}

/// Does this body ever displace a store held by the return-buffer parameter `v`?
///
/// The pre-scan answer to *"is a runtime ownership witness worth a slot here?"* — asked once
/// per function so the witness exists before the first assignment that has to maintain it, in
/// the same shape `displaced_owned` and `collect_views_to_materialise` already use.  A body
/// that never reaches such a site pays nothing.
fn displaces_return_buffer(code: &Value, v: u16, data: &Data) -> bool {
    fn walk(node: &Value, v: u16, data: &Data) -> bool {
        if let Value::Set(t, val) = node.unspan()
            && *t == v
            && displaces_owned_through_fresh_callee(val, v, v, data)
        {
            return true;
        }
        let mut found = false;
        node.unspan().for_each_child(&mut |c| {
            if !found {
                found = walk(c, v, data);
            }
        });
        found
    }
    walk(code, v, data)
}

/// Does this call DELIVER its result into `v`'s existing store?
///
/// The complement of [`displaces_owned_through_fresh_callee`] on the same two facts: `v` sits
/// at the callee's hidden buffer position AND the callee does not adopt a fresh store, so it
/// writes through the buffer it was handed.  `v` then holds exactly what it held before, which
/// is why the runtime ownership witness must be left UNCHANGED rather than recomputed from the
/// call's return (loft#1128).
fn delivers_into_buffer(value: &Value, v: u16, ov: u16, data: &Data) -> bool {
    let Value::Call(fn_nr, args) = value.unspan() else {
        return false;
    };
    let def = data.def(*fn_nr);
    if def.return_adopts_fresh_store() {
        return false;
    }
    let Some(buf_idx) = def.hidden_return_buffer_attr() else {
        return false;
    };
    args.get(buf_idx)
        .is_some_and(|a| matches!(a.unspan(), Value::Var(w) if *w == v || *w == ov))
}

fn displaces_owned_through_fresh_callee(value: &Value, v: u16, ov: u16, data: &Data) -> bool {
    let Value::Call(fn_nr, args) = value.unspan() else {
        return false;
    };
    let def = data.def(*fn_nr);
    if !def.return_adopts_fresh_store() {
        return false;
    }
    // WHICH attribute is the return buffer is `Definition::hidden_return_buffer_attr`'s
    // question — the same answer the substitution that put `v` there read.
    let Some(buf_idx) = def.hidden_return_buffer_attr() else {
        return false;
    };
    let names_v = |x: &Value| matches!(x.unspan(), Value::Var(w) if *w == v || *w == ov);
    if !args.get(buf_idx).is_some_and(names_v) {
        return false;
    }
    // Pre-scan IR can still name the ORIGINAL slot, so both spellings count as a read.
    args.iter()
        .enumerate()
        .all(|(i, a)| i == buf_idx || (!a.reads_var(v) && !a.reads_var(ov)))
}

fn call(to: &'static str, v: u16, data: &Data) -> Value {
    Value::Call(data.def_nr(to), vec![Value::Var(v)])
}

/// @PLN125 arc B — the scope-end hook `v`'s type declares, as a call on `v`.
///
/// > **A drop runs exactly where the value's own `OpFree*` runs — the same binding, the
/// > same scope exit, the same early-exit paths — and never anywhere else.**
///
/// That is the whole design, and phrasing it that way is what makes it small: loft already
/// COMPUTES the fact.  The ownership model decides per binding whether this scope owns the
/// value and whether it dies here, which is what puts an `OpFreeRef` in this list; a
/// returned or borrowed value is already excluded, and the early-`return`, `break` and
/// return-out-of-a-loop paths are already handled here (loft#731 exists because a hand-
/// rolled version of exactly those went wrong).  So the drop DERIVES from the borrow model
/// rather than sitting beside it: there is one answer to "when does this run", not two that
/// can drift.
///
/// Scope is honest and narrow: a **binding this scope owns**.  A droppable that is a FIELD
/// of another record is released by that record's cascade, which is not this list, so it
/// does not fire — a hook that ran from two different mechanisms would be the drift this
/// design exists to avoid.
///
/// **The free is null-tolerant and a drop is not**, which is the one place "where the free
/// runs" needed sharpening.  `OpFreeRef` on a slot that was never written is a no-op — it
/// checks `rec == 0` and returns — so the emitter has never had to know whether a binding
/// actually holds anything.  A drop is a USER call, and running it on an unwritten slot
/// runs the author's rollback against a record that does not exist:
///
/// ```loft
/// if n > 0 { t = Tx { … } }     // the else path never writes `t`
/// ```
///
/// printed `[drop null]` before the guard.  So the call is wrapped in the same liveness
/// test the free performs internally (`OpConvBoolFromRef` IS `rec != 0`), which makes the
/// rule *where the free runs, on a value that exists*.  The same guard settles the aliasing
/// case for free: a caller-side `__ref_N` return buffer that the callee did not adopt is
/// null here and correctly does not fire, while one that WAS adopted never reaches this
/// branch at all (it takes the `OpFreeRefIfDistinct` pairing above).
fn drop_hook(function: &Function, v: u16, data: &Data) -> Option<Value> {
    // A struct-enum binding is a heap record exactly as a `Reference` one is — it just
    // carries a discriminator at its head — so it drops the same way. Reading only
    // `Reference` here is why an enum's cascade was synthesized and then never called
    // (@PLN139 stage D).
    let (Type::Reference(d, _) | Type::Enum(d, true, _)) = function.tp(v).base() else {
        return None;
    };
    // @PLN139 — the CASCADE, not the bare hook: for a type that owns droppable members it
    // is the synthesized function that runs the type's own hook and then releases what it
    // owns, and for every other type it IS the bare hook (`Data::drop_cascade_nr`), so a
    // program with no containers is unchanged.
    let nr = data.drop_cascade_nr(*d);
    if nr == u32::MAX {
        return None;
    }
    let live = Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(v)]);
    Some(Value::If(
        Box::new(live),
        Box::new(Value::Call(nr, vec![Value::Var(v)])),
        Box::new(Value::Null),
    ))
}

/// @PLN85 skip_free-orphan (case a): collect the `skip_free` text `__ncc_N` temps
/// whose null-coalesce value-block is nested (as a sub-expression) inside `node` —
/// i.e. the temps this statement CONSUMES IN PLACE.  Descends through expression
/// constructs (`Call`/`If`/`Insert`/…) and INTO `ncc`-named value-blocks (to reach
/// a nested `??`), but STOPS at any other `Block`/`Loop`: those run their own
/// `convert` and free their own temps, so descending would double-free.  A bare
/// `Set(__ncc, …)` outside an `ncc` block (the temp's own declaration inside the
/// ncc block) is deliberately NOT matched — only the value-block's presence counts,
/// which is why the ncc block's own `convert` attributes no free (its statement is
/// the declaration, not a nested ncc consumer).
fn collect_consumed_ncc_text(node: &Value, function: &Function, out: &mut Vec<u16>) {
    match node {
        Value::Span(b) => collect_consumed_ncc_text(&b.1, function, out),
        Value::Block(bl) if bl.name == "ncc" => {
            for op in &bl.operators {
                if let Value::Set(v, val) = op.unspan()
                    && function.is_staged_text_temp(*v)
                    && function.name(*v).starts_with("__ncc_")
                    // Only the REAL coalesce-subject assignment (a Call / field
                    // access / nested block — a producer of an owned String)
                    // gets an in-place free.  A right-nested `??` (`a ?? (b ?? c)`)
                    // hoists a merge-var pre-declaration `__ncc_N = ""` (a literal
                    // Text init) into the OUTER ncc block while the real
                    // assignment lives in the inner block; collecting the literal
                    // init too freed the temp twice (156 sibling: right-nested
                    // `??` double-free).  A subject is never a bare literal.
                    && !matches!(val.unspan(), Value::Text(_) | Value::Null)
                {
                    out.push(*v);
                }
            }
            // Do NOT recurse INTO this ncc block: a nested `??` (`a ?? b ?? c`)
            // lowers to an ncc block whose Set value is ANOTHER ncc block, and
            // that inner block gets its OWN `convert` (and thus its own in-place
            // free pass) when it is scanned.  Recursing here would ALSO collect
            // the inner block's `__ncc_*` temp from the outer level, freeing it
            // twice (a `text.rs:334` double-free on the interpreter for a chained
            // `??` whose first operand is an owned/call-produced text; 156).
            // Non-ncc structures (call args, if-branches) are still descended
            // through by the `_` arm below, so sibling / nested-in-expression ncc
            // blocks are reached exactly once.
        }
        Value::Block(_) | Value::Loop(_) => {}
        _ => node.for_each_child(&mut |c| collect_consumed_ncc_text(c, function, out)),
    }
}

/// #316 — what kind of store does the RHS of a `Set` into a Reference var
/// yield?  Derived from the `ownership_of` oracle (@PLN90 fold): `Own::Owned`
/// maps to `Owned`, `Borrowed`/`Join` to `View` — the oracle's `_ => Owned`
/// fallback means there is no third "unprovable" case at this site.
enum RefRhs {
    /// A store the variable will own (safe to free on a later transition).
    Owned,
    /// A borrowed view into someone else's store (must never be freed).
    View,
}

/// If `op` is a scope-exit free (`OpFreeRef` / `OpFreeText` /
/// `OpFreeRefIfDistinct`), return the var it frees.  The scopes-side twin of
/// `pre_eval::free_op_var` (generation is not depended on from here).
/// @PLN35 sub-class B — insert `frees` into every arm of an `If`/nested tail, just BEFORE
/// the arm's RESULT value (keeping the result as the tail). Used when a non-hoistable
/// `&text` return's arm allocates a sibling store: the frees run after the allocation
/// inside the arm instead of before the whole `return`. A store null on an arm's path
/// makes its `OpFreeRef` a no-op, so pushing into every arm is safe.
fn push_frees_into_arms(tail: &mut Value, frees: &[Value]) {
    match tail {
        Value::Span(b) => push_frees_into_arms(&mut b.1, frees),
        Value::If(_, then, els) => {
            push_frees_into_arms(then, frees);
            push_frees_into_arms(els, frees);
        }
        Value::Block(bl) => {
            let at = bl.operators.len().saturating_sub(1);
            for (i, fv) in frees.iter().enumerate() {
                bl.operators.insert(at + i, fv.clone());
            }
        }
        Value::Insert(ops) => {
            let at = ops.len().saturating_sub(1);
            for (i, fv) in frees.iter().enumerate() {
                ops.insert(at + i, fv.clone());
            }
        }
        leaf => {
            // A bare result value — wrap `[frees…, value]` in an `Insert` (a flat statement
            // sequence whose value is its last element).
            let v = std::mem::replace(leaf, Value::Null);
            let mut ops: Vec<Value> = frees.to_vec();
            ops.push(v);
            *leaf = Value::Insert(ops);
        }
    }
}

impl Scopes<'_> {
    /// loft#722 — the variables a block's RESULT may point INTO.
    ///
    /// `OpGetField` / `OpGetVector` / `OpGetEnum` read into their first argument's
    /// record, so a chain of them still points at the variable the chain starts
    /// from — the same "walk the getters to the root" fact loft#666 needed for a
    /// `match` subject. A chain rooted in a CALL produces a value of its own and
    /// roots nothing.
    ///
    /// The result of a `??` block is a variable assigned EARLIER in the block
    /// (`__ncc_N = OpGetVectorNullable(OpGetField(tmp, …))`), so a var is resolved
    /// through its in-block assignment before being reported.
    fn result_borrow_roots(ops: &[Value], data: &Data) -> HashSet<u16> {
        // var -> what its assignment points into, for Sets seen in this block.
        let mut from: HashMap<u16, u16> = HashMap::new();
        for op in ops {
            if let Value::Set(v, rhs) = op.unspan()
                && let Some(root) = Self::borrow_root(rhs, data)
            {
                from.insert(*v, root);
            }
        }
        // The result is the last op that is not a scope-exit free; take every var
        // it could evaluate to (both arms of an `if`, etc.).
        let Some(result) = ops
            .iter()
            .rev()
            .find(|o| scope_free_op_var(o, data).is_none())
        else {
            return HashSet::new();
        };
        let mut roots = HashSet::new();
        result.walk(&mut |n| {
            if let Some(r) = Self::borrow_root(n, data) {
                let mut cur = r;
                // Follow the in-block assignment chain, bounded by its own size so
                // a cycle cannot spin.
                for _ in 0..=from.len() {
                    roots.insert(cur);
                    match from.get(&cur) {
                        Some(next) if *next != cur => cur = *next,
                        _ => break,
                    }
                }
            }
        });
        roots
    }

    /// The variable a value points INTO, following getter chains; `None` when it
    /// produces a value of its own.
    fn borrow_root(val: &Value, data: &Data) -> Option<u16> {
        match val.unspan() {
            Value::Var(v) => Some(*v),
            // loft#722 — a getter roots the chain only when it RETURNS A BORROW, and
            // the stdlib declaration already says which do: `OpGetField(v1, fld) ->
            // reference[v1]` and `OpGetVector(r, …) -> reference[r]` name the argument
            // they read into, while `OpGetInt(v1, fld) -> integer` names nothing
            // because it copies a scalar out.  Read that declared dep instead of the
            // `OpGet` name prefix: the prefix is a proxy, and it was too wide.
            //
            // `run() -> integer { make2().n }` lowers to `OpGetInt(__lift_1, 0)`, whose
            // result is an integer that cannot point into the temp. Treating it as a
            // borrow hoisted `__lift_1` to the enclosing scope and dropped its
            // block-exit free, so the call's store was never freed — one leaked record
            // per inline struct-returning call, on both backends.
            //
            // Every borrowing getter names its FIRST argument, so the walk itself is
            // unchanged; only which calls enter it.
            Value::Call(d, args)
                if data.def(*d).name().starts_with("OpGet")
                    && matches!(
                        data.def(*d).returned.base(),
                        Type::Reference(_, _)
                            | Type::Vector(_, _)
                            | Type::Text(_)
                            | Type::Enum(_, true, _)
                            | Type::Sorted(_, _, _)
                            | Type::Hash(_, _, _)
                            | Type::Index(_, _, _)
                    ) =>
            {
                args.first().and_then(|a| Self::borrow_root(a, data))
            }
            _ => None,
        }
    }
}

fn scope_free_op_var(op: &Value, data: &Data) -> Option<u16> {
    if let Value::Call(d, args) = op.unspan()
        && data.op_sets().frees.contains(d)
        && let Some(arg0) = args.first()
        && let Value::Var(v) = arg0.unspan()
    {
        return Some(*v);
    }
    None
}

impl Scopes<'_> {
    fn insert_free(
        &mut self,
        block: &Block,
        free: &[Value],
        is_return: bool,
        data: &Data,
        function: &mut Function,
    ) -> Vec<Value> {
        let mut res = Vec::new();
        let mut ls = Vec::new();
        let n = block.operators.len();
        // @PLN35 — the block's RESULT op is the last op that is NOT a scope-exit free.
        // A value-returning block can end in a free of a block-scoped local that was
        // materialised inside a branch (e.g. a `..rest` text read-temp, freed at the
        // common-parent block after the value-producing `if`): that trailing free is not
        // the block's value.  Hoisting it as the result minted `<int> __ret_N =
        // OpFreeText(local)` — an empty result on interp and invalid native (`= ;`).  So
        // when a value-result return-block's LAST op is a scope-free and its real result
        // is a plain value op (not a nested Block), treat that value op as the result and
        // run the trailing free(s) AFTER the hoist.  All other blocks keep `n-1`.
        let result_idx = if is_return
            && block.result != Type::Void
            && n > 0
            && scope_free_op_var(&block.operators[n - 1], data).is_some()
        {
            (0..n)
                .rev()
                .find(|&i| scope_free_op_var(&block.operators[i], data).is_none())
                .filter(|&i| !matches!(&block.operators[i], Value::Block(_)))
                .unwrap_or(n.wrapping_sub(1))
        } else {
            n.wrapping_sub(1)
        };
        let trailing_frees: Vec<Value> = if n > 0 && result_idx + 1 < n {
            block.operators[result_idx + 1..].to_vec()
        } else {
            Vec::new()
        };
        for (o_nr, o) in block.operators.iter().enumerate() {
            if o_nr > result_idx {
                // A trailing scope-free op (collected into `trailing_frees`); it runs
                // after the result hoist below, never as the block's value.
                continue;
            }
            if o_nr == result_idx {
                if let Value::Block(bl) = &block.operators[o_nr] {
                    for v in self.insert_free(bl, free, is_return, data, function) {
                        ls.push(v);
                    }
                } else if block.result == Type::Void || matches!(block.result, Type::Never) {
                    // `Never` joins `Void` here because it is the same SHAPE for free
                    // placement: a block that never completes yields no value, so there
                    // is nothing to hoist into a `__ret_N` and nothing to return — and
                    // the value leg below, having nothing to hoist, emitted the frees
                    // BEFORE the tail.  When that tail is a branch (a `match` whose arm
                    // `return`s is what types the block `never`), the arm that does NOT
                    // return then reads a variable already released: `null(oob)` on
                    // native, and on a droppable a drop before the arm plus a second one
                    // at the `return` — a use-after-free (loft#992).  The two legs below
                    // put the tail where it belongs either way: a tail that
                    // unconditionally returns keeps the frees in front of it, a tail that
                    // may still complete runs first and the frees follow.
                    //
                    // @P322 — when the function body ends with a nested
                    // Void-result block whose last op is `Return(...)` (the
                    // iterator-generator shape: `for n in […] { yield n; }
                    // return null;`), the OUTER-scope frees passed in via
                    // `free` must run BEFORE the return so function-scope
                    // owned locals (`__vdb_*` vector backings, etc.) get
                    // cleaned up.  Prior to this fix the void branch
                    // dropped `free` entirely and only emitted the inner
                    // `Return` + a redundant trailing `Return(Null)`, so
                    // function-scope vectors leaked at program exit.
                    let o_is_terminal = expr_ends_in_return(o);
                    if o_is_terminal {
                        for v in free {
                            ls.push(v.clone());
                        }
                        ls.push(o.clone());
                    } else {
                        ls.push(o.clone());
                        for v in free {
                            ls.push(v.clone());
                        }
                        ls.push(Value::Return(Box::new(Value::Null)));
                    }
                } else {
                    // @PLN85 poison-green — the B5-L3 invariant extended INTO
                    // block tails: return-site frees must not run before the tail
                    // expression EVALUATES.  The non-block leg (free_vars) has
                    // always hoisted a value tail into a `__ret_N` temp before the
                    // frees; this leg emitted `frees; Return(tail)` — the tail
                    // then evaluated AFTER the frees, and any store it still read
                    // (directly or through an alias no fact carries, e.g. a
                    // fn-ref read out of a struct field calling a closure record
                    // the host struct's free just cascaded) was a use-after-free:
                    // silent stale data without LOFT_POISON, deterministic
                    // garbage with it (the closure/field-capture family).
                    // A bare-Var tail is free-safe (its slot holds the value) —
                    // EXCEPT a `&τ` place ref (`RefVar`): returning it DEREFS the
                    // place DbRef at the Return, after the frees released the
                    // source store (the @PLN87 L3/L4 live-read shapes under
                    // poison).  Hoisting `Set(__ret_N, Var(r))` performs the
                    // deref NOW, before the frees.
                    let tail_needs_eval = match o.unspan() {
                        Value::Null => false,
                        // A `&τ` place ref derefs at the Return (hoist) —
                        // EXCEPT a `&text`: the promoted out-buffer returned
                        // per the text-return contract (alive in the caller;
                        // hoisting it broke native's buffer materialization).
                        Value::Var(v) => {
                            matches!(function.tp(*v), Type::RefVar(inner)
                            if !matches!(inner.base(), Type::Text(_)))
                            // A bare text LOCAL returned while the function holds a hidden
                            // buffer is delivered through that buffer and freed, like any
                            // other owned text return (@FR-F-Ret) — its slot does hold the
                            // value, but nothing frees the `String` behind it once the frame
                            // drops.  A lambda whose one buffer went to `tb` returned `ta`
                            // this way, one orphan per call (loft#1357).
                            || (is_return
                                && !function.is_argument(*v)
                                && !function.is_skip_free(*v)
                                && matches!(function.tp(*v).base(), Type::Text(_))
                                && !matches!(function.tp(*v), Type::RefVar(_))
                                && any_text_return_buffer(function, data, self.d_nr)
                                    .is_some_and(|b| b != *v))
                        }
                        _ => true,
                    };
                    // Text results take the same hoist with the text-leg
                    // mechanics: the temp's String owns a byte copy, and
                    // `skip_free` keeps its OpFreeText out of the scope exit
                    // (the caller copies bytes immediately on return — the
                    // established `__ret_N` text contract pre_eval/native read).
                    let is_text_result = matches!(block.result.base(), Type::Text(_));
                    // @PLN35 sub-class A — a heap-record / vector return (a `Reference`
                    // struct, a struct-`Enum(_, true)`, or a `Vector`) ALSO takes the hoist:
                    // when the block's tail is an `if`/`match` whose taken arm ALLOCATES a
                    // sibling store (a `..rest` materialisation's `__vdb`), the un-hoisted
                    // `frees; return <if>` emits `OpFreeRef(__vdb)` BEFORE the allocation
                    // inside the return → the store leaks (`ANALYSIS.md`, oracle
                    // FREE-before-ALLOC). Hoisting to a `__ret` temp runs the allocation
                    // first. A DbRef `__ret` is native-safe (the `&text` out-buffer is NOT —
                    // it is excluded above via `tail_needs_eval`).
                    let is_heap_ref_result = matches!(
                        block.result.base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
                    );
                    let mut hoist_tmp: Option<u16> = None;
                    if is_return
                        && (!free.is_empty() || !trailing_frees.is_empty())
                        && (is_value_return_type(&block.result)
                            || is_text_result
                            || is_heap_ref_result)
                        && tail_needs_eval
                        && !expr_ends_in_return(o)
                    {
                        if is_text_result
                            && !matches!(o.unspan(), Value::Null)
                            && let Some(buf) = text_return_buffer_for(o, function, data, self.d_nr)
                        {
                            // @FR-F-Ret / @FR-F-Call — the block-tail twin of `free_vars`'s
                            // text delivery: an owned text return goes into the CALLER's
                            // hidden `&text` buffer, never into a frame-local temp nothing
                            // frees.  This is the leg an early `return a ?? b` reaches (its
                            // `??` lowers to a block whose tail is the `if`), and it orphaned
                            // one String per call on the interpreter (loft#1338).  Per arm,
                            // so native's arm types stay uniform; then the temps the copy
                            // drained are freed, the scope frees follow, and the buffer is
                            // what the `Return` below names.
                            let mut delivered = o.clone();
                            crate::parser::Parser::push_text_arms_into(
                                &mut delivered,
                                buf,
                                data.def_nr("OpCreateStack"),
                            );
                            ls.push(delivered);
                            let pending: Vec<Value> =
                                trailing_frees.iter().chain(free.iter()).cloned().collect();
                            free_copied_text_sources(&mut ls, o, &pending, function, data);
                            hoist_tmp = Some(buf);
                        } else {
                            self.ret_temp_counter += 1;
                            let name = format!("__ret_{}", self.ret_temp_counter);
                            let tmp = function.add_temp_var(&name, &block.result);
                            // The hoisted value is the RETURN value (transferred to the
                            // caller): its scope-exit free must NOT fire, else the caller
                            // reads a freed record.  Text already does this — and for text it
                            // is the ORPHAN `free_vars`'s residual arm documents: kept only
                            // where the function has no buffer to deliver through.  A heap
                            // ref/vector needs it too.
                            if is_text_result || is_heap_ref_result {
                                function.set_skip_free(tmp);
                            }
                            self.var_scope.insert(tmp, self.scope);
                            self.var_order.push(tmp);
                            ls.push(v_set(tmp, o.clone()));
                            if is_text_result {
                                // The copy drained the `??` temp inside the tail; free it
                                // even where the temp itself is the residual orphan.
                                let pending: Vec<Value> =
                                    trailing_frees.iter().chain(free.iter()).cloned().collect();
                                free_copied_text_sources(&mut ls, o, &pending, function, data);
                            }
                            hoist_tmp = Some(tmp);
                            // A buffer the tail READS is still the delivery once the value is
                            // staged (the `free_vars` twin says why): move the temp's bytes
                            // into it, free the temp, and return the buffer (loft#1357).
                            if is_text_result
                                && !matches!(o.unspan(), Value::Null)
                                && let Some(buf) = any_text_return_buffer(function, data, self.d_nr)
                            {
                                ls.push(v_set(buf, Value::Var(tmp)));
                                ls.push(call("OpFreeText", tmp, data));
                                hoist_tmp = Some(buf);
                            }
                        }
                    }
                    // The block's OWN trailing scope-frees (after the result op) run first,
                    // then the enclosing scope's `free`; both after the result is hoisted.
                    let mut ret_frees: Vec<Value> = Vec::new();
                    for v in trailing_frees.iter().chain(free.iter()) {
                        // A free of a var the RETURNED tail expression still READS
                        // cannot run before it — that is a use-after-free (the
                        // `?? [literal]` return-tail class: interp read the freed
                        // store silently — LOFT_POISON turns it into a SIGSEGV —
                        // and native crashed on the 65535 sentinel).  Its freeing
                        // is owned INSIDE the expression instead: the return-
                        // delivery materializer consumes an owned-fresh arm local
                        // after its append, on EVERY path (cross-arm frees).  So
                        // the pre-return free is DROPPED here, not moved (even
                        // under the hoist — the materializer already freed the
                        // consumed store inside the expression; re-emitting the
                        // free after the temp would double-free it).
                        if is_return
                            && let Some(fv) = scope_free_op_var(v, data)
                            && o.reads_var(fv)
                            && function.is_arm_consumed(fv)
                        {
                            continue;
                        }
                        ret_frees.push(v.clone());
                    }
                    // @PLN35 sub-class B — a NON-hoistable `&text` (RefVar-text) tail whose `If`
                    // arm ALLOCATES a sibling store: the frees can't run before the return (they
                    // would precede the arm's `OpDatabase` → the store leaks) and the `&text`
                    // out-buffer can't hoist (native `Str::new(&local)` dangle, excluded via
                    // `tail_needs_eval`). Push the frees INTO each arm, just before the arm's
                    // result, so they run AFTER the allocation and the buffer is yielded raw.
                    let is_refvar_text = matches!(block.result.base(),
                        Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)));
                    if hoist_tmp.is_none()
                        && is_return
                        && is_refvar_text
                        && !ret_frees.is_empty()
                        && matches!(o.unspan(), Value::If(_, _, _))
                    {
                        let mut tail = o.clone();
                        push_frees_into_arms(&mut tail, &ret_frees);
                        ls.push(Value::Return(Box::new(tail)));
                    } else {
                        ls.extend(ret_frees);
                        if let Some(tmp) = hoist_tmp {
                            ls.push(Value::Return(Box::new(Value::Var(tmp))));
                        } else if is_return {
                            ls.push(Value::Return(Box::new(o.clone())));
                        } else {
                            ls.push(o.clone());
                        }
                    }
                }
            } else {
                ls.push(o.clone());
            }
        }
        res.push(Value::Block(Box::new(Block {
            name: block.name,
            operators: ls,
            result: block.result.clone(),
            scope: block.scope,
            var_size: 0,
        })));
        res
    }
}

/// True when `expr` is a `Return` (or recursively ends with one through
/// `Insert`/`Block` wrappers).  Used by `free_vars` to decide whether the
/// B5-L3 `__ret_N` wrap is safe — wrapping a terminal expression would
/// produce `let _ret = return …` in native and double-emit the inner
/// Return inside the Set's expression generator.
fn expr_ends_in_return(expr: &Value) -> bool {
    matches!(expr.tail(), Value::Return(_))
}

/// Whether a function's return type holds a plain value (no heap ownership).
/// Used by the B5-L3 fix in `free_vars` to decide whether saving the tail
/// expression into a `__ret_N` temp is safe.  Heap-owned types are excluded
/// for now — their ownership transfer interacts with `OpFreeRef` emission
/// and needs a separate design pass.
fn is_value_return_type(tp: &Type) -> bool {
    // @PLN25: peel `Optional` — a `float?`/`integer?`/… return is the same
    // value-return shape (native reps it as the sentinel-carrying scalar).
    // Without the peel a `-> τ?` tail call with pending free-ops fell through
    // the B5-L3 wrap: the call was emitted as a DISCARDED statement + a
    // fabricated `return null`, which native materialised as `return 0.0`
    // — a silent NON-NULL corruption (the routing elevation-kernel bug).
    // loft#816 — a FLAT pure-value tuple is a value return too.  @PLN135 gave
    // `-> (float, integer)` Rust's own tuple ABI (no synthetic `__tuple`
    // record), and that shape reaches here as a bare `Type::Tuple`; unnamed, an
    // anonymous tail tuple with pending frees fell through to the discard +
    // fabricated `Return(Null)` path and native answered the ZERO initialiser
    // (`return (0.0_f64, 0)`) for the whole tuple.  Silent, and on one backend
    // only — the interpreter read the elements off eval-stack top.
    //
    // Recursing per element is what keeps the bound exact.  A NESTED pure-value
    // tuple qualifies too (loft#817): the wrap emits `Set(__ret_N, <tuple>)` and
    // then reads the temp back, and both halves are nested-aware —
    // `emit_tuple_var_pop_put` writes the leaves and `generate_var` delegates to
    // `emit_tuple_var_push_recursive` to read them.  An element with a lifetime
    // (text / vector / record) is either rewritten to the boxed `Reference`
    // shape upstream or belongs to the @P329 tuple-of-text branch, and neither
    // may be hoisted by a plain `Set` — so those stop the recursion here.
    if let Type::Tuple(elems) = tp.base() {
        return !elems.is_empty() && elems.iter().all(is_value_return_type);
    }
    is_scalar_value_type(tp)
}

/// The scalar half of [`is_value_return_type`]: a type returned in a register,
/// owning nothing on the heap.  Split out so the tuple case reads as "every
/// element is itself a value return", with this as the recursion's base.
fn is_scalar_value_type(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Enum(_, false, _)
    )
}

/// loft#754 — a return type delivered as a store POINTER (`DbRef`): a vector, a
/// record, or a struct-enum, including one reached through a `&`-parameter
/// place ref.  Names the half of the return space that neither
/// `is_value_return_type` nor the text branch covers, so a tail expression of
/// this shape is hoisted before the scope frees instead of being dropped.
fn is_heap_return_type(tp: &Type) -> bool {
    match tp.base() {
        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => true,
        Type::RefVar(inner) => is_heap_return_type(inner),
        _ => false,
    }
}

/// loft#793 — does this function receive a hidden return BUFFER argument?
///
/// Only a DENSE heap return is given one: both reservation sites
/// (`parser/mod.rs`, `parser/definitions.rs`) gate on
/// `Reference | Vector | Enum(_, true, _)`.  A NULLABLE heap return (`-> S?`)
/// is not one of those, so it has no buffer and its value can travel back only
/// as the call's own return value — which is what makes a dropped call tail a
/// silently-null answer there rather than a delivered one.
fn has_return_buffer(d_nr: u32, data: &Data) -> bool {
    data.def(d_nr).attributes().iter().any(|a| {
        a.hidden
            && matches!(
                a.typedef,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
    })
}

/// loft#793 — the tail of `expr` is a CALL that carries the block's VALUE, the
/// shape the legacy `Return(Null)` fall-through drops.
///
/// The typed-NULL producers are excluded — `OpNullRefSentinel` and the
/// `OpConv<T>FromNull` family.  For those `Return(Null)` is exactly right, and
/// hoisting one is actively wrong: a `-> StructEnum?` fall-through null is
/// `OpConvEnumFromNull`, whose native form is the `255u8` discriminator
/// sentinel, so binding it to a `DbRef`-typed temp emitted `255u8 as DbRef`
/// (E0605).
fn tail_is_value_call(expr: &Value, data: &Data) -> bool {
    match expr.tail().unspan() {
        Value::Call(d, _) => {
            let name = data.def(*d).name();
            name != "OpNullRefSentinel"
                && !(name.starts_with("OpConv") && name.ends_with("FromNull"))
        }
        Value::CallRef(_, _) => true,
        _ => false,
    }
}

/// A branch whose TAIL is literally null — `Value::Null` or the
/// `OpNullRefSentinel()` fall-through `parse_match` injects.  Deliberately does
/// NOT recurse into `If`: a branch that merely CONTAINS a null sub-arm is not a
/// null terminal (unifying through it would lose the other sub-arm's value).
impl Scopes<'_> {
    /// The tail of `expr` is an `If` (a lowered if/match) — its arms deliver
    /// into the assignment target via the arm-unification machinery.
    fn tail_is_branch(expr: &Value) -> bool {
        match expr {
            Value::If(_, _, _) => true,
            Value::Block(bl) => bl.operators.last().is_some_and(Self::tail_is_branch),
            Value::Insert(ops) => ops.last().is_some_and(Self::tail_is_branch),
            Value::Span(b) => Self::tail_is_branch(&b.1),
            _ => false,
        }
    }
}

/// Does this return expression's terminal value reduce to `null` — a bare `Value::Null`
/// or the reference sentinel `null_nr` names?
///
/// Descends `Block`/`Insert`/`Span` to the tail.  A nested `if` is NOT descended: an arm
/// that IS null and a branch that merely CONTAINS one are different answers, and only the
/// first belongs to a terminal — `return_has_null_arm` is the walker that asks the second.
///
/// A `Return`/`Drop` wrapper IS descended, because this is asked about a return
/// EXPRESSION and the wrapper is the thing being examined.  The `parser::control`
/// siblings (`branch_yields_null`, `arm_yields_direct_null`, `arm_is_null`) ask the same
/// null question about what an ARM hands to a JOIN — where a `return` hands it nothing —
/// and stop at the wrapper.
fn is_null_terminal(expr: &Value, null_nr: u32) -> bool {
    match expr.unspan() {
        Value::Null => true,
        Value::Call(d, _) => *d == null_nr,
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|o| is_null_terminal(o, null_nr)),
        Value::Insert(ops) => ops.last().is_some_and(|o| is_null_terminal(o, null_nr)),
        Value::Return(inner) | Value::Drop(inner) => is_null_terminal(inner, null_nr),
        _ => false,
    }
}

/// P236 extension (@PLN85 P4-records): `returned_var` with a NULL-arm terminal
/// unifying as a WILDCARD against the other arm's var.  The work-ref null-inits
/// at function entry and a null arm never allocates into it, so
/// `Return(Var(v))` yields the same null the sentinel did — while the PRESENT
/// arm's record now rides the var instead of the freed-TOS channel (the
/// record match/if-arm UAF the poison sweep exposed: the legacy pattern freed
/// the arm's store, then `Return(Null)` handed the caller the freed store's
/// bytes off the eval stack — silently stale without LOFT_POISON, null with).
fn returned_var_null_unified(expr: &Value, null_nr: u32) -> u16 {
    match expr {
        Value::Var(v) => *v,
        Value::Block(bl) => {
            let mut v = u16::MAX;
            for o in &bl.operators {
                v = returned_var_null_unified(o, null_nr);
            }
            v
        }
        Value::Return(inner) | Value::Drop(inner) => returned_var_null_unified(inner, null_nr),
        Value::Insert(ops) => ops
            .last()
            .map_or(u16::MAX, |o| returned_var_null_unified(o, null_nr)),
        Value::If(_, t, f) => {
            let t_var = returned_var_null_unified(t, null_nr);
            let f_var = returned_var_null_unified(f, null_nr);
            if t_var == f_var || (f_var == u16::MAX && is_null_terminal(f, null_nr)) {
                t_var
            } else if t_var == u16::MAX && is_null_terminal(t, null_nr) {
                f_var
            } else {
                u16::MAX
            }
        }
        Value::Span(b) => returned_var_null_unified(&b.1, null_nr),
        _ => u16::MAX,
    }
}

/// @PLN85 cluster II — the SET version of `returned_var`: every terminal var a
/// return expression can yield, INCLUDING all arms of an `If`/`match` (which
/// `returned_var` collapses to `u16::MAX` when the arms differ). These are the
/// function's "return-source" locals — their heap store is transferred to the
/// caller, so the callee must not free them at scope exit.
/// Every variable ANY `return` in this body can hand back — including an EARLY return
/// nested mid-block, which the two helpers below cannot see.
///
/// Both of them answer for the body's TAIL value: `returned_var_null_unified` keeps the
/// last operator's answer and `collect_return_sources` takes a block's last non-free
/// result.  So `if a { return a?; } x` reports only `x`, and the work var the guard arm
/// delivers looks like a local nobody frees — which is exactly what it is NOT, because
/// returning it transfers it to the caller.  `check_ref_leaks` asserted on that shape as a
/// leak (a generic instantiated at a STRUCT, where the return is a heap ref).
#[cfg(debug_assertions)]
fn collect_all_return_vars(expr: &Value, data: &Data, out: &mut Vec<u16>) {
    if let Value::Return(inner) = expr.unspan() {
        collect_return_sources(inner, data, out);
    }
    expr.walk(&mut |v| {
        if let Value::Return(inner) = v {
            collect_return_sources(inner, data, out);
        }
    });
}

fn collect_return_sources(expr: &Value, data: &Data, out: &mut Vec<u16>) {
    match expr {
        Value::Var(v) => {
            if !out.contains(v) {
                out.push(*v);
            }
        }
        // @PLN85 P4-records — a block's VALUE is its last op that is NOT a scope-exit
        // free. A captured struct-enum field binds an owned `_mv_<f>` text whose
        // `OpFreeText` is appended AFTER the arm chain (`[Kw { word }, ..] => LetS{…}`),
        // so `.last()` alone hits that free and hides the record sources — the returned
        // enum store is then freed unconditionally before the return (35c sub-class A,
        // plans/captured-group-elem-uaf.md). Skip trailing frees to reach the real value.
        Value::Block(bl) => {
            if let Some(last) = last_non_free_result(&bl.operators, data) {
                collect_return_sources(last, data, out);
            }
        }
        Value::Return(inner) | Value::Drop(inner) => collect_return_sources(inner, data, out),
        Value::Insert(ops) => {
            if let Some(last) = last_non_free_result(ops, data) {
                collect_return_sources(last, data, out);
            }
        }
        Value::If(_, t, f) => {
            collect_return_sources(t, data, out);
            collect_return_sources(f, data, out);
        }
        Value::Span(b) => collect_return_sources(&b.1, data, out),
        _ => {}
    }
}

/// The last operator of a sequence that carries the block's VALUE — i.e. skipping
/// trailing scope-exit frees (`OpFreeRef` / `OpFreeText` / `OpFreeRefIfDistinct`) and
/// `Line` position markers. A value-returning block can end in a free of a block-local
/// (a captured `_mv_<f>` text, a `..rest` `__vdb`) appended after the result, so the
/// naive `.last()` would mistake that free for the value.
fn last_non_free_result<'a>(ops: &'a [Value], data: &Data) -> Option<&'a Value> {
    ops.iter()
        .rev()
        .find(|op| scope_free_op_var(op, data).is_none() && !matches!(op.unspan(), Value::Line(_)))
}

/// Does this return have an arm that yields something OTHER than one of its
/// `return_sources` — a genuine BORROWING arm (loft#1022)?
///
/// `collect_return_sources` is the UNION of the arms' terminal VARS, so a work-ref that
/// only one arm delivers still lands in it and its scope-exit free is suppressed on
/// every path.  That is right when every arm delivers a source and wrong when one arm
/// hands back a borrow instead: `if take { bx.p } else { P { x: 9 } }` yields a field
/// access on the first arm, which is no variable at all, and the second arm's work-ref
/// is then owned by nobody.
///
/// Answers false for a return with no join in it — one path cannot orphan the value it
/// is itself delivering — and false when every arm's terminal is a source, which is the
/// shape a record literal aliasing locals has.
fn return_has_non_source_arm(expr: &Value, sources: &[u16]) -> bool {
    fn walk(e: &Value, sources: &[u16], in_join: bool) -> bool {
        match e.unspan() {
            Value::If(_, t, f) => walk(t, sources, true) || walk(f, sources, true),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|o| walk(o, sources, in_join)),
            Value::Insert(ops) => ops.last().is_some_and(|o| walk(o, sources, in_join)),
            Value::Return(inner) | Value::Drop(inner) => walk(inner, sources, in_join),
            Value::Var(v) => in_join && !sources.contains(v),
            _ => in_join,
        }
    }
    walk(expr, sources, false)
}

/// @PLN85 A.1 part i — does this return expression have a reachable arm whose
/// terminal is the typed-null sentinel (`OpNullRefSentinel` / `Value::Null`)?
///
/// A nullable `if b { Struct{} } else { null }` allocates the present arm's
/// work-ref placeholder (`__ref_N`) at function entry, but the `null` arm
/// returns the sentinel and leaves that placeholder ORPHANED.  The SET-driven
/// free-suppression must therefore NOT suppress that work-ref: whether it is
/// transferred (present path) or orphaned-and-must-free (null path) is a RUNTIME
/// decision, not a static one — so the SET hands it back to the standard
/// work-ref free path (which frees the orphan correctly).  Resolving the runtime
/// split (NRVO-delivering each arm into `__retbuf`, or a conditional free) is the
/// callee-side control.rs / native work, out of scope-analysis's reach.
///
/// `null_sentinel_nr` is `OpNullRefSentinel`'s def number (resolved by the
/// caller, which holds `data`).
///
/// A `Return`/`Drop` wrapper IS descended, because this is asked about a return
/// EXPRESSION and the wrapper is the thing being examined.  The `parser::control`
/// siblings (`branch_yields_null`, `arm_yields_direct_null`, `arm_is_null`) ask the same
/// null question about what an ARM hands to a JOIN — where a `return` hands it nothing —
/// and stop at the wrapper.
///
/// One home, because the same fact decides two things at opposite ends of one return:
/// whether scope analysis may suppress the work-ref's free (here), and whether the
/// `Bind` leg may copy the tail into the return buffer WHOLE or has to deliver the arms
/// one at a time (`ref_return`) — the whole-tail copy answers the buffer on every path,
/// so with a null arm present it swallows the sentinel.
pub(crate) fn return_has_null_arm(expr: &Value, null_sentinel_nr: u32) -> bool {
    match expr.unspan() {
        Value::Null => true,
        Value::Call(d, _) => *d == null_sentinel_nr,
        Value::If(_, t, f) => {
            return_has_null_arm(t, null_sentinel_nr) || return_has_null_arm(f, null_sentinel_nr)
        }
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|o| return_has_null_arm(o, null_sentinel_nr)),
        Value::Insert(ops) => ops
            .last()
            .is_some_and(|o| return_has_null_arm(o, null_sentinel_nr)),
        Value::Return(inner) | Value::Drop(inner) => return_has_null_arm(inner, null_sentinel_nr),
        _ => false,
    }
}

/// Recursively collect every variable freed by `OpFreeRef` in `ir`.
/// Used by `check_ref_leaks` to verify no Reference variable is leaked.
#[cfg(debug_assertions)]
fn collect_freed_vars(ir: &Value, free_ops: &[u32], result: &mut HashSet<u16>) {
    ir.walk(&mut |n| {
        if let Value::Call(d_nr, args) = n
            && free_ops.contains(d_nr)
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
        {
            result.insert(*v);
        }
    });
}

/// Block-tail temps whose store is ADOPTED (moved) into a freed assignment LHS,
/// so freeing the LHS frees them — no `OpFreeRef` of the temp is emitted, and
/// none should be (it would double-free the shared record).
///
/// The shape is `Set(lhs, Block[…, Var(v)])`: the block allocates a fresh record
/// into `v` and yields it, and `lhs = <block>` PutRef-aliases that record into
/// `lhs` (the empty-dep adopt — e.g. the `#reading file` surface temp behind
/// `q = f#read as S`).  `v`'s free responsibility transfers to `lhs`, so when
/// `lhs` is freed, `v` is covered.
///
/// Narrow by construction: it matches only a BLOCK-valued RHS (a plain
/// `lhs = v` COPY has an RHS of `Var(v)`, not `Block`, and its `v` is freed
/// separately, so `v` is already in `freed` and never reaches the leak assert).
/// It credits `v` only when `lhs` is in `freed`, so it cannot mask a genuine
/// leak where the adopting LHS itself is never freed.
#[cfg(debug_assertions)]
fn collect_adopted_block_results(ir: &Value, freed: &HashSet<u16>, result: &mut HashSet<u16>) {
    ir.walk(&mut |n| {
        if let Value::Set(lhs, rhs) = n
            && freed.contains(lhs)
            && let Value::Block(bl) = rhs.unspan()
            && let Some(Value::Var(v)) = bl.operators.last().map(Value::unspan)
        {
            result.insert(*v);
        }
    });
}

/// Debug-only check: refuse to compile a text-returning function that frees a
/// local text on a path that REACHES a `Return` handing that same local back.
/// The returned Str would dangle into freed `String` memory — the interpreter
/// occasionally gets away with it (if the underlying allocator hasn't reused
/// the slot), but native codegen materialises this as
/// `let _v = String::new(); … free(_v); return &_v;` and trips Rust's UB check.
///
/// The judgement is per-`Return` and path-sensitive, and it has to be both.
/// A function with more than one `return` legitimately frees the locals it is
/// NOT handing back, and that free sits inside the branch returning something
/// else: `if n > 0 { free(ta); return tb; } return ta;` is correct code, and
/// the free of `ta` never runs on the path that returns `ta`.  Asking instead
/// "is this var freed anywhere in the body?" answers that shape as a dangling
/// return.  A branch that always returns cannot fall through, so its frees are
/// carried on its own path only and are dropped at the join.
///
/// Every `Return` is judged, not only the body's tail — an early return is
/// exactly where a second text buffer puts its free.
///
/// Companion to `check_ref_leaks` above — that check catches owned-ref leaks
/// at compile time; this one catches use-after-free on return.
#[cfg(debug_assertions)]
fn check_text_return(ir: &Value, function: &Function, fn_name: &str, ret_type: &Type, data: &Data) {
    if !matches!(ret_type, Type::Text(_)) {
        return;
    }
    let free_text_nr = data.def_nr("OpFreeText");
    if free_text_nr == u32::MAX {
        return;
    }
    let mut freed: HashSet<u16> = HashSet::new();
    // A `break` at function level has no loop to leave, so this set stays empty; it exists
    // so the walker can hand a break arm's frees to the loop that encloses it.
    let mut breaks: HashSet<u16> = HashSet::new();
    check_text_return_path(
        ir,
        &mut freed,
        &mut breaks,
        free_text_nr,
        data.def_nr("OpNullRefSentinel"),
        function,
        fn_name,
    );
}

/// True when control cannot fall through `node` to the statement AFTER it.
///
/// Two ways out qualify, and they differ in where the frees go rather than in
/// whether they fall through:
///
/// - a `Return` leaves the function, so a free on that path reaches nothing;
/// - a `Break` leaves the LOOP, so a free on that path reaches what follows the
///   loop and NOT the rest of the loop body.  The `Loop` arm of the walker is
///   what carries those frees past the loop, so counting `Break` here loses no
///   strictness — it only stops a break arm's frees being charged to the body
///   statements the break jumps over.
///
/// Missing the `Break` case is a FALSE POSITIVE, not a blind spot, and it fired:
/// `for v in it { if done { free(v); break; } return v; }` — the shape every
/// early-returning loop over a text generator has — read as freeing `v` on the
/// path reaching its own `Return`, and hard-failed the debug-assertions gate on
/// a program with nothing wrong with it.
///
/// Answers `false` for anything it cannot prove, including `Loop` itself.  That
/// stays the safe direction: an unproven terminator keeps its frees in the
/// continuation set, which makes the check stricter rather than blinder.
#[cfg(debug_assertions)]
fn always_returns(node: &Value) -> bool {
    match node.unspan() {
        Value::Return(_) => true,
        Value::Block(bl) => bl.operators.iter().any(always_returns),
        Value::Insert(ops) => ops.iter().any(always_returns),
        Value::If(_, t, f) => always_returns(t) && always_returns(f),
        _ => false,
    }
}

/// True when control cannot fall through `node` to the statement after it — by
/// RETURNING or by BREAKING.  Paired with [`always_returns`], which separates the
/// two: a return's frees reach nothing, a break's reach what follows the loop.
#[cfg(debug_assertions)]
fn never_falls_through(node: &Value) -> bool {
    match node.unspan() {
        Value::Return(_) | Value::Break(_) => true,
        // Anything after a `Return` or `Break` in the same block is dead, so one
        // top-level terminator terminates the whole block.
        Value::Block(bl) => bl.operators.iter().any(never_falls_through),
        Value::Insert(ops) => ops.iter().any(never_falls_through),
        Value::If(_, t, f) => never_falls_through(t) && never_falls_through(f),
        _ => false,
    }
}

/// Walk `node` in execution order carrying `freed` — the text vars already
/// released on the path reaching it — and assert each `Return` hands back a
/// var this path has not freed.  See `check_text_return` for the rule.
#[cfg(debug_assertions)]
fn check_text_return_path(
    node: &Value,
    freed: &mut HashSet<u16>,
    breaks: &mut HashSet<u16>,
    free_text_nr: u32,
    null_nr: u32,
    function: &Function,
    fn_name: &str,
) {
    let walk = |n: &Value, set: &mut HashSet<u16>, brk: &mut HashSet<u16>| {
        check_text_return_path(n, set, brk, free_text_nr, null_nr, function, fn_name);
    };
    match node.unspan() {
        Value::Call(d_nr, args) if *d_nr == free_text_nr => {
            if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                freed.insert(*v);
            }
        }
        Value::Return(inner) => {
            // The return expression runs BEFORE the return itself, so a free
            // inside it counts against this very return.
            walk(inner, freed, breaks);
            let ret_var = returned_var_null_unified(inner, null_nr);
            if ret_var != u16::MAX && matches!(function.tp(ret_var), Type::Text(_)) {
                assert!(
                    !freed.contains(&ret_var),
                    "[check_text_return] fn '{}' frees local text '{}' (var_nr={ret_var}) \
                     on the path reaching its Return — the returned Str would dangle into \
                     freed String memory.  scopes.rs must leave '{}' for the caller to free.",
                    fn_name,
                    function.name(ret_var),
                    function.name(ret_var),
                );
            }
        }
        Value::If(cond, t, f) => {
            // The condition runs on both paths; each arm gets its own.
            walk(cond, freed, breaks);
            let mut then_freed = freed.clone();
            walk(t, &mut then_freed, breaks);
            let mut else_freed = freed.clone();
            walk(f, &mut else_freed, breaks);
            // Where an arm's frees go is decided by HOW it leaves.  Falling through hands
            // them to the next statement; RETURNING hands them nowhere, since the function
            // is over; BREAKING hands them to what follows the LOOP, which is what `breaks`
            // carries out to the enclosing `Loop` arm.  Collapsing the last two — treating a
            // break like a return — would lose a real free, and treating it like a
            // fall-through charges it to body statements the break jumps over, which is the
            // false positive this split exists to remove.
            let mut arm = |a: &Value, arm_freed: HashSet<u16>, freed: &mut HashSet<u16>| {
                if always_returns(a) {
                } else if never_falls_through(a) {
                    breaks.extend(arm_freed);
                } else {
                    freed.extend(arm_freed);
                }
            };
            arm(t, then_freed, freed);
            arm(f, else_freed, freed);
        }
        Value::Block(bl) => {
            for op in &bl.operators {
                walk(op, freed, breaks);
            }
        }
        Value::Loop(bl) => {
            // A loop is left either by falling out of its body or by a BREAK, and both
            // continuations resume AFTER the loop — so both sets are unioned here.  The
            // break set is fresh per loop, which is what keeps an inner loop's breaks from
            // reaching the outer loop's continuation.
            let mut body_freed = freed.clone();
            let mut body_breaks = HashSet::new();
            for op in &bl.operators {
                walk(op, &mut body_freed, &mut body_breaks);
            }
            freed.extend(body_breaks);
            freed.extend(body_freed);
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for op in ops {
                walk(op, freed, breaks);
            }
        }
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                walk(a, freed, breaks);
            }
        }
        Value::Iter(_, create, next, extra_init) => {
            walk(create, freed, breaks);
            walk(next, freed, breaks);
            walk(extra_init, freed, breaks);
        }
        Value::Set(_, inner)
        | Value::TuplePut(_, _, inner)
        | Value::Drop(inner)
        | Value::Yield(inner) => walk(inner, freed, breaks),
        _ => {}
    }
}

/// The `check_text_return` walker's own gate.  Every cell is one step from
/// another, and the pair that matters is `free_then_return_same_var_in_arm`
/// (must fire) against `free_in_arm_that_returns_another_var` (must not) —
/// they differ only in WHICH var the arm hands back, which is exactly the
/// distinction the path rule exists to draw.
///
/// Compiled only where the check itself is: `[profile.dev.package.loft]`
/// strips debug assertions from ordinary builds, so these run in the
/// `-C debug-assertions=on` CI gate that runs the check.
#[cfg(all(test, debug_assertions))]
mod text_return_path_tests {
    use super::check_text_return_path;
    use crate::data::{Deps, Type, Value, v_block, v_if, v_loop};
    use crate::variables::Function;
    use std::collections::HashSet;

    const FREE_TEXT: u32 = 7;
    const NULL_SENTINEL: u32 = 8;
    const TA: u16 = 0;
    const TB: u16 = 1;

    fn vars() -> Function {
        let mut f = Function::new("f", "t.loft");
        f.add_temp_var("ta", &Type::Text(Deps::none()));
        f.add_temp_var("tb", &Type::Text(Deps::none()));
        f
    }

    fn free(v: u16) -> Value {
        Value::Call(FREE_TEXT, vec![Value::Var(v)])
    }
    fn ret(v: u16) -> Value {
        Value::Return(Box::new(Value::Var(v)))
    }

    /// Run the walker over a function body; panics exactly as the check does.
    fn check(body: Vec<Value>) {
        let ir = v_block(body, Type::Text(Deps::none()), "body");
        let mut freed = HashSet::new();
        let mut breaks = HashSet::new();
        check_text_return_path(
            &ir,
            &mut freed,
            &mut breaks,
            FREE_TEXT,
            NULL_SENTINEL,
            &vars(),
            "probe",
        );
    }

    /// Straight-line use-after-free: the plainest shape the check exists for.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn free_then_return_same_var() {
        check(vec![free(TA), ret(TA)]);
    }

    /// The free and the `return` of the same var sit in ONE arm.  Skipping a
    /// returning arm wholesale would blind the check here, so this is the cell
    /// that keeps the fall-through rule honest.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn free_then_return_same_var_in_arm() {
        check(vec![
            v_if(
                Value::Var(TB),
                v_block(vec![free(TA), ret(TA)], Type::Never, "then"),
                Value::Null,
            ),
            ret(TB),
        ]);
    }

    /// A free on an arm that FALLS THROUGH still reaches the tail return —
    /// the arm not returning is the whole difference from the cell below.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn free_in_arm_that_falls_through() {
        check(vec![
            v_if(
                Value::Var(TB),
                v_block(vec![free(TA)], Type::Void, "then"),
                Value::Null,
            ),
            ret(TA),
        ]);
    }

    /// The shape the path rule was written for: the arm frees `ta` and returns
    /// `tb`, so the free never runs on the path that returns `ta`.  Correct
    /// code — the check must stay silent (loft#1113's two-text-local lambda).
    #[test]
    fn free_in_arm_that_returns_another_var() {
        check(vec![
            v_if(
                Value::Var(TB),
                v_block(vec![free(TA), ret(TB)], Type::Never, "then"),
                Value::Null,
            ),
            ret(TA),
        ]);
    }

    /// Both arms return, each freeing the local it does not hand back.
    #[test]
    fn each_arm_frees_what_it_does_not_return() {
        check(vec![v_if(
            Value::Var(TB),
            v_block(vec![free(TA), ret(TB)], Type::Never, "then"),
            v_block(vec![free(TB), ret(TA)], Type::Never, "else"),
        )]);
    }

    /// A BREAK arm's free does not reach the loop body that follows it — the shape every
    /// early-returning loop over a text source has, and the one that read as a
    /// use-after-free before `never_falls_through` learned about `Break`:
    ///
    /// ```text
    /// loop { ta = next(); if done { free(ta); break; } return ta; }
    /// ```
    ///
    /// The arm that frees is the arm that LEAVES, so it can never reach the `return`.
    /// Correct code — the check must stay silent.
    #[test]
    fn free_in_a_break_arm_does_not_reach_the_bodys_return() {
        check(vec![v_loop(
            vec![
                v_if(
                    Value::Var(TB),
                    v_block(vec![free(TA), Value::Break(0)], Type::Never, "then"),
                    Value::Null,
                ),
                ret(TA),
            ],
            "loop",
        )]);
    }

    /// The control for the cell above, and the reason it cannot be written as "skip a loop".
    /// Here the free FALLS THROUGH inside the same loop body and the `return` follows it, so
    /// the free really is on the path that returns `ta`.  The two differ only by the `break`.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn free_without_a_break_still_reaches_the_bodys_return() {
        check(vec![v_loop(
            vec![
                v_if(
                    Value::Var(TB),
                    v_block(vec![free(TA)], Type::Void, "then"),
                    Value::Null,
                ),
                ret(TA),
            ],
            "loop",
        )]);
    }

    /// A break arm's free is still charged to what follows the LOOP, which is where that
    /// path actually resumes.  Counting `Break` as a terminator must not lose this.
    #[test]
    #[should_panic(expected = "frees local text 'ta'")]
    fn a_break_arms_free_still_reaches_after_the_loop() {
        check(vec![
            v_loop(
                vec![v_if(
                    Value::Var(TB),
                    v_block(vec![free(TA), Value::Break(0)], Type::Never, "then"),
                    Value::Null,
                )],
                "loop",
            ),
            ret(TA),
        ]);
    }
}

/// After scope analysis, assert that every Reference variable that should be
/// freed has a corresponding `OpFreeRef` somewhere in `ir`.
///
/// A variable "should be freed" when:
/// - Its type is `Reference(_, dep)` with `dep.is_empty()`
/// - It is not a function parameter (scope > 0)
/// - It is not marked `skip_free`
/// - It is not in the function's return-type dependencies
///
/// Only compiled in debug builds; the check panics rather than emitting a
/// diagnostic so that the failure is visible immediately during development.
#[cfg(debug_assertions)]
fn check_ref_leaks(
    ir: &Value,
    function: &Function,
    data: &Data,
    fn_name: &str,
    ret_type: &Type,
    var_scope: &BTreeMap<u16, u16>,
) {
    // Every op that FREES its first argument counts as that var's free
    // site: the plain scope-exit free, the @P317 tag-checked free, and
    // the witness-pair conditional free (`OpFreeRefIfDistinct` — how a
    // ref-returning call's work ref is released when it doesn't alias
    // the assigned var; the armed-corpus sweep's ~130 "no OpFreeRef"
    // false positives were all this shape).
    // ⚠ `OpFreeRefOrHandUp` belongs here for the same reason the other three do, and its
    // absence is what made this assert fire on `n___lambda_3`'s `__ref_p2_1` — a store that
    // IS released, by an op this list had never heard of.  loft#1186 added it (D-clo-13) as
    // `OpFreeRefIfDistinct` with an owner on the not-distinct leg, on the EMIT side only:
    // three files emit it and nineteen name its sibling, so every matcher keyed on the op
    // NAME went blind to the new spelling at once.  A free-op list is a claim about a
    // NOTION — "this op releases its first argument" — and each new spelling of that notion
    // has to arrive here too, or the assert reports a leak the compiler does not have.
    let sets = data.op_sets();
    let free_ops: Vec<u32> = sets
        .unconditional_ref_frees
        .iter()
        .chain(sets.conditional_ref_frees.iter())
        .copied()
        .collect();
    let mut freed: HashSet<u16> = HashSet::new();
    collect_freed_vars(ir, &free_ops, &mut freed);

    // A block-tail temp adopted into a freed LHS (`q = f#read as S`, whose
    // `#reading file` surface temp `_read_N` moves its record into `q`) has no
    // OpFreeRef of its own and must not — `q`'s free covers it.  Credit it so
    // the leak assert below does not false-positive on the moved-from source.
    let mut adopted: HashSet<u16> = HashSet::new();
    collect_adopted_block_results(ir, &freed, &mut adopted);

    // H2: `ret_type` deps are ATTRIBUTE indices — translate each to its
    // frame var through the attribute name before pooling with the
    // frame-space deps below (the old code inserted them raw, so an attr
    // index colliding with an unrelated var number silently suppressed a
    // leak report).
    let fn_def_nr = data.def_nr(fn_name);
    let mut ret_deps: HashSet<u16> = HashSet::new();
    for raw in ret_type.depend() {
        match crate::data::DepEntry::decode(raw) {
            crate::data::DepEntry::Attr(a) => {
                let a_idx = a as usize;
                if fn_def_nr != u32::MAX && a_idx < data.def(fn_def_nr).attributes().len() {
                    let av = function.var(&data.def(fn_def_nr).attributes()[a_idx].name);
                    if av != u16::MAX {
                        ret_deps.insert(av);
                    }
                }
            }
            // H2 step 5: a tagged callee-frame note IS a frame var — pool
            // it directly (the untagged value was silently dropped by the
            // attr-range guard before, so a returned closure's work var
            // could surface as a false leak report).
            crate::data::DepEntry::CalleeFrame(w) => {
                ret_deps.insert(w);
            }
        }
    }
    // The directly-returned variable (e.g. the owned struct constructed by a function
    // whose return type is Reference) passes ownership to the caller — no FreeRef is
    // emitted for it and that is correct.  Exclude it so check_ref_leaks does not
    // false-positive on `fn foo() -> S { S { ... } }`.
    let direct_ret_var = returned_var_null_unified(ir, data.def_nr("OpNullRefSentinel"));
    // Transitive: if the returned variable depends on another variable, that
    // variable's store must also survive — include it in ret_deps.
    if direct_ret_var != u16::MAX {
        for d in function.tp(direct_ret_var).depend() {
            ret_deps.insert(d);
        }
    }
    // …and every variable an EARLY return hands back, which the tail-value helper above
    // cannot see: `if a { return a?; } x` reports only `x`, so the guard arm's work var
    // read as a local nobody freed.  Returning it IS the transfer, wherever the return sits.
    let mut early_ret: Vec<u16> = Vec::new();
    collect_all_return_vars(ir, data, &mut early_ret);
    for v in early_ret {
        ret_deps.insert(v);
        for d in function.tp(v).depend() {
            ret_deps.insert(d);
        }
    }

    let built_with = capture_build_backings(data, function, ir);
    for (&v, &scope) in var_scope {
        if scope == 0 {
            continue; // function parameter — caller frees
        }
        if (v as usize) >= function.count() as usize {
            continue; // variable belongs to outer scope — not our problem
        }
        if function.is_skip_free(v) {
            continue;
        }
        // #323: a heap local a closure record ADOPTS is owned by that record (which
        // stores its 12-byte DbRef; `free_named`'s cascade frees it when the record
        // dies), so `get_free_vars` emits no frame-exit free for it and "unfreed" is not
        // "leaked" here.
        //
        // The same call the emitter makes, not the rule written out again.  This mirror
        // going out of step with it is exactly how loft#1308 stayed hidden, and a mirror
        // that knows only the `is_captured` half calls the BACKING local of a collection
        // capture a leak — which no closure captured by name.
        if capture_adoption_owns_free(data, function, &built_with, v) {
            continue;
        }
        if v == direct_ret_var {
            continue; // ownership transferred to caller
        }
        if adopted.contains(&v) {
            continue; // moved into a freed LHS — that free covers this store
        }
        if let Type::Reference(_, dep) = function.tp(v) {
            // LOFT_REF_LEAK_WARN=1 downgrades the assert to a warning so a
            // debug build can still RUN a program with a known leak shape
            // (e.g. to chase a separate runtime corruption past compile).
            let warn_only = std::env::var("LOFT_REF_LEAK_WARN").is_ok();
            if warn_only && !(!dep.is_empty() || ret_deps.contains(&v) || freed.contains(&v)) {
                eprintln!(
                    "[check_ref_leaks] WARNING: Reference variable '{}' (var_nr={v}) in \
                     function '{fn_name}' has no OpFreeRef (scope {scope}) — store leak.",
                    function.name(v),
                );
            } else {
                assert!(
                    !dep.is_empty() || ret_deps.contains(&v) || freed.contains(&v),
                    "[check_ref_leaks] Reference variable '{}' (var_nr={v}) in function \
                     '{}' has no OpFreeRef — it is in scope {scope} but was never freed. \
                     This is likely a scope-registration bug: the variable was registered \
                     in an inner block scope that is not reachable from function-exit cleanup.",
                    function.name(v),
                    fn_name
                );
            }
            // warn about variables with deps that are only text-return work refs.
            // These deps are spurious (struct copies the text), but OpFreeRef is still
            // skipped, causing a store leak at runtime.
            if !dep.is_empty()
                && !ret_deps.contains(&v)
                && !freed.contains(&v)
                && dep.iter().all(|d| {
                    function.name(*d).starts_with("__ref_")
                        || function.name(*d).starts_with("__rref_")
                })
            {
                eprintln!(
                    "[check_ref_leaks] Warning: Reference variable '{}' (var_nr={v}) in \
                     function '{}' has only text-work deps {:?} — likely spurious. \
                     Store will leak at runtime.",
                    function.name(v),
                    fn_name,
                    dep.iter()
                        .map(|d| function.name(*d).to_string())
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
}

// ── Plan-06 phase 5b — par-safety analyser (DESIGN.md D8) ────────────────────

use crate::data::{ImpureCategory, Purity};

/// Plan-06 phase 5b minimal — purity-driven `is_par_safe` classifier.
///
/// Returns `true` iff `d_nr`'s body contains only par-safe calls per
/// the Purity classification (DESIGN.md D8.1).  Recursive into user
/// fn callees; cycles short-circuit to `true` (5b's placeholder
/// trick — phase 5e replaces with proper monotonic fixed-point).
///
/// **Minimum implementation — covers Purity-driven rejection only.**
/// Full D8 rules require additional analysis not in this commit:
/// - R1 (writes to non-local) — partially captured via stdlib's
///   `Impure(ParentWrite)` annotations on `vector_add`/`hash_set`/etc.
///   The per-call "is first arg local?" check is missing; today
///   any call to a `ParentWrite` fn is rejected outright.
/// - R2 (nested par) — `Impure(ParCall)` returns true here; full
///   5b proper recurses into the inner worker fn.
/// - R4 (mutation through captured Reference) — not yet detected.
///
/// Non-`Function` def_nrs return `false` (only fns can be par
/// workers).  `CallRef` (runtime fn-ref) callsites pessimise to
/// `false` — the actual callee is not statically known.
///
/// ⚠ NOT WIRED TO ANYTHING.  Nothing calls this outside its own tests, so no program is
/// rejected for an impure par worker today.  That is not a missing gate against the rules:
/// @FR-C-Impure says an impure worker's result is UNDEFINED, not that the compiler must
/// refuse it — the contract is "make the worker pure", and conformance is differential
/// (concurrency.md § Deviations).  This classifier is the DIAGNOSTIC that DESIGN.md D8
/// wants on top of that, and its consumer has never been written.  Keep or delete it as a
/// deliberate choice; do not read the `#[allow(dead_code)]` as a temporary state.
#[allow(dead_code)]
#[must_use]
pub fn is_par_safe(data: &Data, d_nr: u32) -> bool {
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return false;
    }
    let mut visited = HashSet::new();
    walk_par_safe(data, d_nr, &mut visited)
}

#[allow(dead_code)]
fn walk_par_safe(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> bool {
    if !visited.insert(d_nr) {
        // Cycle detected — break recursion optimistically (placeholder
        // trick).  Phase 5e replaces this with monotonic fixed-point
        // iteration so mutually-recursive pure pairs classify correctly.
        return true;
    }
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[d_nr as usize];
    if !matches!(def.def_type, DefType::Function) {
        return false;
    }
    walk_par_safe_value(&def.code, data, visited)
}

#[allow(dead_code)]
fn walk_par_safe_value(value: &Value, data: &Data, visited: &mut HashSet<u32>) -> bool {
    body_calls_par_safe(value, data, &mut |callee, data| {
        walk_par_safe(data, callee, visited)
    })
}

/// Shared body walk behind the two par-safety analyses (pass-3 dedupe):
/// every `Call` must pass the purity gate; a `CallRef` (runtime fn-ref,
/// callee unknown at compile time) rejects conservatively.  The two
/// analyses differ ONLY in how an UNKNOWN user fn is decided — `user_fn`
/// supplies that policy (the 5b walk recurses with a visited set; the
/// fixed-point pass looks up its classification table).
fn body_calls_par_safe(
    value: &Value,
    data: &Data,
    user_fn: &mut dyn FnMut(u32, &Data) -> bool,
) -> bool {
    !value.any_node(&mut |n| match n {
        Value::Call(callee, _) => !call_purity_safe(*callee, data, user_fn),
        Value::CallRef(_, _) => true,
        _ => false,
    })
}

/// The shared purity gate: Purity decides directly except for UNKNOWN
/// user fns, which `user_fn` decides.
fn call_purity_safe(callee: u32, data: &Data, user_fn: &mut dyn FnMut(u32, &Data) -> bool) -> bool {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure
        | Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Prng
            | ImpureCategory::Io
            // Nested par: D8 R2 says the inner worker fn must itself be
            // par-safe.  Minimum impl accepts; full 5b looks up the worker
            // fn arg and recurses into it.
            | ImpureCategory::ParCall,
        ) => true,
        Purity::Impure(ImpureCategory::ParentWrite) => false,
        Purity::Unknown => {
            if matches!(def.code, Value::Null) {
                // Native stdlib fn with no annotation — conservative.
                false
            } else {
                user_fn(callee, data)
            }
        }
    }
}

/// @PLN35 PC4 — is a sub-rule PURE, hence safe to invoke speculatively and backtrack over?  This is
/// COMPLEMENTARY to par-safety (which bans only parent-store writes): a sub-rule's effect must be
/// UNOBSERVABLE, because a cursor `match` may invoke it even when the arm is not taken (the call is
/// hoisted unconditionally) and, once mixing lands, may backtrack over it.  So the OBSERVABLE /
/// non-deterministic / concurrent categories (host I/O, I/O, prng, par-call) disqualify it; a
/// `parent_write` does NOT (record construction + the cursor advance ARE parent writes — internal
/// and unavoidable), and an un-annotated native builtin is pure (every observable builtin is
/// annotated `#impure`).  Descends the call graph; a recursion cycle is optimistically pure (it must
/// consume through a base case — and PC3 already rejects left-recursive sub-rules).  An indirect
/// `CallRef` is conservatively impure (its target is not statically known).
pub fn sub_rule_is_pure(data: &Data, d_nr: u32) -> bool {
    let mut visited = HashSet::new();
    walk_sub_rule_pure(data, d_nr, &mut visited)
}

fn walk_sub_rule_pure(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> bool {
    if !visited.insert(d_nr) {
        return true; // cycle — break optimistically (PC3 rejects left recursion separately)
    }
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[d_nr as usize];
    if !matches!(def.def_type, DefType::Function) {
        return false;
    }
    match def.purity {
        Purity::Pure => true,
        // OBSERVABLE / non-deterministic / concurrent effects disqualify a sub-rule.
        Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Io
            | ImpureCategory::Prng
            | ImpureCategory::ParCall,
        ) => false,
        // A parent-store WRITE is how a sub-rule builds its result record and advances the cursor
        // (OpNewRecord / OpSetInt / OpFinishRecord are `#impure(parent_write)`) — unavoidable and
        // internal, so allowed.  The narrow case of writing EXTERNAL shared state and then
        // backtracking over it is not distinguishable at this category granularity — deferred.
        Purity::Impure(ImpureCategory::ParentWrite) => true,
        Purity::Unknown => {
            if matches!(def.code, Value::Null) {
                // A native builtin with NO annotation is pure: every OBSERVABLE builtin (I/O, host)
                // IS annotated `#impure`, so the un-annotated natives are pure reads / arithmetic /
                // record ops (OpGetField, OpGetVector, OpNeRef, …).
                true
            } else {
                !def.code.any_node(&mut |n| match n {
                    Value::Call(callee, _) => !walk_sub_rule_pure(data, *callee, visited),
                    Value::CallRef(_, _) => true, // indirect call — cannot analyze
                    _ => false,
                })
            }
        }
    }
}

#[cfg(test)]
mod par_safety_tests {
    use super::is_par_safe;
    use crate::data::{Block, Data, DefType, ImpureCategory, Purity, Type, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn pure_fn_with_no_calls_is_par_safe() {
        let mut d = Data::new();
        let id = d.add_def("pure_leaf", &pos(), DefType::Function);
        d.definitions[id as usize].code = Value::Int(42);
        assert!(is_par_safe(&d, id));
    }

    #[test]
    fn fn_calling_pure_stdlib_is_par_safe() {
        let mut d = Data::new();
        let stdlib = d.add_def("min", &pos(), DefType::Function);
        d.definitions[stdlib as usize].purity = Purity::Pure;
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        assert!(is_par_safe(&d, user));
    }

    /// The two classifiers must give the same verdict — one returns a bool, the other the
    /// REASON for it, and a verdict with no reason to give is a contradiction.
    ///
    /// They did not. `par_unsafe_reason` was hand-rolled with a `_ => None` fallback and no
    /// `Return` arm, so a body of `return <call>` was reported par-SAFE without ever being
    /// looked at, while `is_par_safe` (walking with `any_node`, which visits every node)
    /// called the same function unsafe. Measured over the corpus par workers: 16 of 42
    /// disagreed, every one of them `is_par_safe=false` against `reason=none`.
    ///
    /// `Return` is the arm that exposed it; the fallback now descends via `for_each_child`,
    /// so a wrapper nobody thought of cannot reintroduce the gap.
    #[test]
    fn the_two_par_classifiers_agree_through_a_return() {
        let mut d = Data::new();
        let native = d.add_def("OpMulInt", &pos(), DefType::Function);
        d.definitions[native as usize].purity = Purity::Unknown;
        d.definitions[native as usize].code = Value::Null; // a native: no loft body
        let user = d.add_def("dbl", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Return(Box::new(Value::Call(native, vec![])));

        let flag = is_par_safe(&d, user);
        let reason = super::par_unsafe_reason(&d, user);
        assert_eq!(
            flag,
            reason.is_none(),
            "bool says {flag} while the reason is {reason:?} — a verdict with no reason \
             behind it, or a reason behind no verdict"
        );
        assert!(
            reason.is_some_and(|r| r.contains("OpMulInt")),
            "and the reason must NAME the call it found through the `Return`"
        );
    }

    #[test]
    fn fn_calling_parent_write_stdlib_is_not_par_safe() {
        let mut d = Data::new();
        let stdlib = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[stdlib as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        assert!(!is_par_safe(&d, user));
    }

    #[test]
    fn fn_calling_host_io_stdlib_is_par_safe() {
        let mut d = Data::new();
        let stdlib = d.add_def("log_warn", &pos(), DefType::Function);
        d.definitions[stdlib as usize].purity = Purity::Impure(ImpureCategory::HostIo);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        assert!(is_par_safe(&d, user));
    }

    #[test]
    fn fn_calling_unannotated_native_is_not_par_safe() {
        let mut d = Data::new();
        let stdlib = d.add_def("mystery_native", &pos(), DefType::Function);
        // purity defaults to Unknown; code defaults to Value::Null
        // (native fn with no body).
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        assert!(!is_par_safe(&d, user));
    }

    #[test]
    fn fn_calling_callref_is_not_par_safe() {
        let mut d = Data::new();
        let user = d.add_def("user", &pos(), DefType::Function);
        // Var slot 5, no args — runtime fn-ref of unknown target.
        d.definitions[user as usize].code = Value::CallRef(5, vec![]);
        assert!(!is_par_safe(&d, user));
    }

    #[test]
    fn user_fn_recursion_into_par_safe_callee() {
        let mut d = Data::new();
        let pure_stdlib = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_stdlib as usize].purity = Purity::Pure;
        let inner = d.add_def("inner", &pos(), DefType::Function);
        d.definitions[inner as usize].code = Value::Call(pure_stdlib, vec![]);
        let outer = d.add_def("outer", &pos(), DefType::Function);
        d.definitions[outer as usize].code = Value::Call(inner, vec![]);
        assert!(is_par_safe(&d, outer));
    }

    #[test]
    fn cycle_breaks_optimistically() {
        // Mutually recursive a→b→a — placeholder trick returns true.
        // Phase 5e's fixed-point iteration handles this properly.
        let mut d = Data::new();
        let a = d.add_def("a", &pos(), DefType::Function);
        let b = d.add_def("b", &pos(), DefType::Function);
        d.definitions[a as usize].code = Value::Call(b, vec![]);
        d.definitions[b as usize].code = Value::Call(a, vec![]);
        assert!(is_par_safe(&d, a));
    }

    #[test]
    fn block_walks_every_operator() {
        let mut d = Data::new();
        let pure_fn = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_fn as usize].purity = Purity::Pure;
        let bad_fn = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad_fn as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Block(Box::new(Block {
            name: "test",
            operators: vec![Value::Call(pure_fn, vec![]), Value::Call(bad_fn, vec![])],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
        assert!(
            !is_par_safe(&d, user),
            "block walk must reject when any operator calls a parent-write fn"
        );
    }
}

// ── Plan-06 phase 5d — par-safety diagnostic helpers (DESIGN.md D8) ──────────

/// Plan-06 phase 5d (DESIGN.md D8 diagnostic shapes) — explains
/// **why** a fn is par-unsafe by walking its body once and
/// returning the first violating call's information.
///
/// Returns `None` if the fn is par-safe (no violations to report).
/// Returns `Some(reason)` describing the first encountered violation
/// — currently one of:
///   - `"call to parent-write stdlib fn '<name>'"`
///   - `"call to unannotated native fn '<name>'"`
///   - `"runtime fn-ref call (callee unknown at compile time)"`
///   - `"recursive descent into par-unsafe user fn '<name>'"`
///
/// Used by phase 5b proper's codegen integration: when
/// `is_par_safe(d_nr) == false`, the parser calls
/// `par_unsafe_reason(d_nr)` to embed the specific cause in the
/// compile-error diagnostic body, matching D8's example error
/// shape with `--> file:line` + offending construct + fix-it.
///
/// ⚠ NOT WIRED TO ANYTHING — the reason-string twin of [`is_par_safe`], and unreached for
/// the same reason.  See there.
#[allow(dead_code)]
#[must_use]
pub fn par_unsafe_reason(data: &Data, d_nr: u32) -> Option<String> {
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return Some(format!("invalid def_nr {d_nr}"));
    }
    let mut visited = HashSet::new();
    walk_par_unsafe_reason(data, d_nr, &mut visited)
}

#[allow(dead_code)]
fn walk_par_unsafe_reason(data: &Data, d_nr: u32, visited: &mut HashSet<u32>) -> Option<String> {
    if !visited.insert(d_nr) {
        // Cycle — same optimistic short-circuit as is_par_safe.
        return None;
    }
    if d_nr == u32::MAX || (d_nr as usize) >= data.definitions.len() {
        return Some(format!("invalid def_nr {d_nr}"));
    }
    let def = &data.definitions[d_nr as usize];
    if !matches!(def.def_type, DefType::Function) {
        return Some(format!("def {} is not a function", def.name));
    }
    walk_par_unsafe_reason_value(&def.code, data, visited)
}

#[allow(dead_code)]
fn walk_par_unsafe_reason_value(
    value: &Value,
    data: &Data,
    visited: &mut HashSet<u32>,
) -> Option<String> {
    match value {
        Value::Call(callee, args) => {
            if let Some(r) = call_reason(*callee, data, visited) {
                return Some(r);
            }
            for a in args {
                if let Some(r) = walk_par_unsafe_reason_value(a, data, visited) {
                    return Some(r);
                }
            }
            None
        }
        Value::CallRef(_, _args) => {
            Some("runtime fn-ref call (callee unknown at compile time)".to_string())
        }
        Value::Block(b) => {
            for v in &b.operators {
                if let Some(r) = walk_par_unsafe_reason_value(v, data, visited) {
                    return Some(r);
                }
            }
            None
        }
        Value::Insert(vs) => {
            for v in vs {
                if let Some(r) = walk_par_unsafe_reason_value(v, data, visited) {
                    return Some(r);
                }
            }
            None
        }
        Value::If(c, t, e) => walk_par_unsafe_reason_value(c, data, visited)
            .or_else(|| walk_par_unsafe_reason_value(t, data, visited))
            .or_else(|| walk_par_unsafe_reason_value(e, data, visited)),
        Value::Loop(body) => {
            for v in &body.operators {
                if let Some(r) = walk_par_unsafe_reason_value(v, data, visited) {
                    return Some(r);
                }
            }
            None
        }
        Value::Set(_, rhs) => walk_par_unsafe_reason_value(rhs, data, visited),
        Value::Span(b) => walk_par_unsafe_reason_value(&b.1, data, visited),
        // Every OTHER node descends via the keystone rather than stopping.
        //
        // This arm used to be `_ => None`, and the named arms above do not include `Return` —
        // so `fn dbl(x) { return x * 2; }` was reported par-SAFE without its body ever being
        // looked at, while `is_par_safe` (which walks with `any_node`, so it sees every node)
        // called the same function unsafe.  16 of 42 par workers in the corpus disagreed that
        // way, every one of them `is_par_safe=false` against `reason=none`: a verdict of
        // "unsafe" with no reason to give for it.
        other => {
            let mut found = None;
            other.for_each_child(&mut |c| {
                if found.is_none() {
                    found = walk_par_unsafe_reason_value(c, data, visited);
                }
            });
            found
        }
    }
}

#[allow(dead_code)]
fn call_reason(callee: u32, data: &Data, visited: &mut HashSet<u32>) -> Option<String> {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return Some(format!("invalid callee def_nr {callee}"));
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure
        | Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Prng
            | ImpureCategory::Io
            | ImpureCategory::ParCall,
        ) => None,
        Purity::Impure(ImpureCategory::ParentWrite) => {
            Some(format!("call to parent-write stdlib fn '{}'", def.name))
        }
        Purity::Unknown => {
            if matches!(def.code, Value::Null) {
                Some(format!("call to unannotated native fn '{}'", def.name))
            } else {
                walk_par_unsafe_reason(data, callee, visited).map(|inner| {
                    format!(
                        "recursive descent into par-unsafe user fn '{}': {}",
                        def.name, inner
                    )
                })
            }
        }
    }
}

#[cfg(test)]
mod par_diag_tests {
    use super::par_unsafe_reason;
    use crate::data::{Block, Data, DefType, ImpureCategory, Purity, Type, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn par_safe_fn_has_no_reason() {
        let mut d = Data::new();
        let id = d.add_def("safe", &pos(), DefType::Function);
        d.definitions[id as usize].code = Value::Int(0);
        assert!(par_unsafe_reason(&d, id).is_none());
    }

    #[test]
    fn parent_write_call_reports_offending_fn_name() {
        let mut d = Data::new();
        let stdlib = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[stdlib as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        let r = par_unsafe_reason(&d, user).unwrap();
        assert!(
            r.contains("vector_add") && r.contains("parent-write"),
            "expected parent-write reason mentioning vector_add; got: {r}"
        );
    }

    #[test]
    fn unannotated_native_reports_specifically() {
        let mut d = Data::new();
        let stdlib = d.add_def("mystery", &pos(), DefType::Function);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Call(stdlib, vec![]);
        let r = par_unsafe_reason(&d, user).unwrap();
        assert!(
            r.contains("unannotated") && r.contains("mystery"),
            "got: {r}"
        );
    }

    #[test]
    fn callref_reports_runtime_unknown() {
        let mut d = Data::new();
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::CallRef(3, vec![]);
        let r = par_unsafe_reason(&d, user).unwrap();
        assert!(r.contains("runtime fn-ref"), "got: {r}");
    }

    #[test]
    fn nested_user_fn_reports_the_chain() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let inner = d.add_def("inner", &pos(), DefType::Function);
        d.definitions[inner as usize].code = Value::Call(bad, vec![]);
        let outer = d.add_def("outer", &pos(), DefType::Function);
        d.definitions[outer as usize].code = Value::Call(inner, vec![]);
        let r = par_unsafe_reason(&d, outer).unwrap();
        assert!(
            r.contains("recursive descent") && r.contains("inner") && r.contains("vector_add"),
            "expected chain explanation through inner→vector_add; got: {r}"
        );
    }

    #[test]
    fn first_violating_call_in_block_wins() {
        let mut d = Data::new();
        let pure_fn = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_fn as usize].purity = Purity::Pure;
        let bad_first = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad_first as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let bad_second = d.add_def("hash_set", &pos(), DefType::Function);
        d.definitions[bad_second as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let user = d.add_def("user", &pos(), DefType::Function);
        d.definitions[user as usize].code = Value::Block(Box::new(Block {
            name: "test",
            operators: vec![
                Value::Call(pure_fn, vec![]),
                Value::Call(bad_first, vec![]),
                Value::Call(bad_second, vec![]),
            ],
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }));
        let r = par_unsafe_reason(&d, user).unwrap();
        // First violator wins: should mention vector_add, not hash_set.
        assert!(r.contains("vector_add"), "got: {r}");
        assert!(!r.contains("hash_set"), "second violator leaked: {r}");
    }
}

// ── Plan-06 phase 5e — fixed-point par-safety (DESIGN.md D8 phase 5e) ────────

/// Plan-06 phase 5e — monotonic fixed-point over the call graph.
///
/// Replaces 5b's placeholder "cycle returns true optimistically"
/// trick with a proper fixed-point iteration: every user fn starts
/// classified true; the worklist demotes fns whose bodies invoke
/// par-unsafe callees; demotions propagate to callers via the
/// caller graph (Data::callers_of / D12).
///
/// Result: mutually-recursive pure fns (`is_even` / `is_odd` shape)
/// classify true, where 5b's placeholder would have returned false
/// pessimistically.  Mutually-recursive fns where ANY participant
/// is impure correctly demote the whole cycle.
///
/// Termination: classifications are monotonic (true → false, never
/// reverse); worklist re-enqueues only when a demotion actually
/// happens.  Worst case: every user fn walked twice = O(N + E)
/// where E = call-graph edge count.
///
/// Currently no production caller — phase 5b' wires this in place
/// of the per-fn `is_par_safe` for the parser's diagnostic.
#[allow(dead_code)]
#[must_use]
pub fn analyse_par_safety_fixpoint(data: &Data) -> HashMap<u32, bool> {
    use std::collections::VecDeque;

    // Step 1: initial classification.  Every user fn starts true;
    // stdlib annotations are taken at face value.
    let user_fns: Vec<u32> = data.user_fn_d_nrs();
    let mut classification: HashMap<u32, bool> = HashMap::new();
    for &d_nr in &user_fns {
        classification.insert(d_nr, true);
    }

    // Step 2: worklist iteration.
    let mut worklist: VecDeque<u32> = user_fns.iter().copied().collect();
    while let Some(d_nr) = worklist.pop_front() {
        // Skip if already demoted — monotonic.
        if !classification.get(&d_nr).copied().unwrap_or(false) {
            continue;
        }
        let def = &data.definitions[d_nr as usize];
        let still_safe = walk_classified(&def.code, data, &classification);
        if !still_safe {
            classification.insert(d_nr, false);
            // Propagate demotion: every caller may need to re-check
            // because their body now calls a newly-demoted callee.
            for caller in data.callers_of(d_nr) {
                if classification.get(&caller).copied().unwrap_or(false) {
                    worklist.push_back(caller);
                }
            }
        }
    }
    classification
}

/// Walk a Value tree using the current classification map (not
/// recursive descent like 5b's walk_par_safe_value).  For user-fn
/// callees, looks up classification[callee]; for stdlib callees,
/// uses the Purity annotation.  No cache placeholder needed —
/// the fixed-point loop owns convergence.
#[allow(dead_code)]
fn walk_classified(value: &Value, data: &Data, classification: &HashMap<u32, bool>) -> bool {
    body_calls_par_safe(value, data, &mut |callee, _| {
        // User fn — look up the classification.  If absent
        // (user_fn_d_nrs missed it), conservative false.
        classification.get(&callee).copied().unwrap_or(false)
    })
}

#[cfg(test)]
mod par_fixpoint_tests {
    use super::analyse_par_safety_fixpoint;
    use crate::data::{Data, DefType, ImpureCategory, Purity, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn mutually_recursive_pure_fns_both_classify_safe() {
        // is_even / is_odd shape — the canonical case 5b's
        // placeholder trick gets WRONG (returns false for both)
        // and that 5e gets RIGHT (returns true for both).
        let mut d = Data::new();
        let pure_fn = d.add_def("min", &pos(), DefType::Function);
        d.definitions[pure_fn as usize].purity = Purity::Pure;
        let is_even = d.add_def("is_even", &pos(), DefType::Function);
        let is_odd = d.add_def("is_odd", &pos(), DefType::Function);
        // is_even calls is_odd + min (pure)
        d.definitions[is_even as usize].code =
            Value::Call(is_odd, vec![Value::Call(pure_fn, vec![])]);
        // is_odd calls is_even
        d.definitions[is_odd as usize].code = Value::Call(is_even, vec![]);
        let result = analyse_par_safety_fixpoint(&d);
        assert_eq!(result.get(&is_even), Some(&true), "is_even should be safe");
        assert_eq!(result.get(&is_odd), Some(&true), "is_odd should be safe");
    }

    #[test]
    fn impure_in_cycle_demotes_all_participants() {
        // a→b→c→a, but b also calls vector_add (parent_write).
        // All three should classify false.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let a = d.add_def("a", &pos(), DefType::Function);
        let b = d.add_def("b", &pos(), DefType::Function);
        let c = d.add_def("c", &pos(), DefType::Function);
        d.definitions[a as usize].code = Value::Call(b, vec![]);
        // b calls bad + c
        d.definitions[b as usize].code = Value::Call(c, vec![Value::Call(bad, vec![])]);
        d.definitions[c as usize].code = Value::Call(a, vec![]);
        let result = analyse_par_safety_fixpoint(&d);
        assert_eq!(result.get(&a), Some(&false), "a → b → bad");
        assert_eq!(result.get(&b), Some(&false), "b → bad");
        assert_eq!(result.get(&c), Some(&false), "c → a → b → bad");
    }

    #[test]
    fn pure_fn_unaffected_by_unrelated_impure_fn() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let pure_user = d.add_def("pure_user", &pos(), DefType::Function);
        let bad_user = d.add_def("bad_user", &pos(), DefType::Function);
        d.definitions[pure_user as usize].code = Value::Int(0);
        d.definitions[bad_user as usize].code = Value::Call(bad, vec![]);
        let result = analyse_par_safety_fixpoint(&d);
        assert_eq!(result.get(&pure_user), Some(&true));
        assert_eq!(result.get(&bad_user), Some(&false));
    }

    #[test]
    fn empty_data_returns_empty_map() {
        let d = Data::new();
        let result = analyse_par_safety_fixpoint(&d);
        assert!(result.is_empty());
    }
}

// ── Plan-06 phase 5b' shallow check (precise, no false positives) ────────────

/// Plan-06 phase 5b' — precise shallow par-safety check.
///
/// Walks `worker_d_nr`'s body looking for **direct** calls to fns
/// classified `Impure(ParentWrite)`.  Does NOT recurse into callee
/// bodies — so it only fires when the worker code itself contains
/// the offending call, not when a transitive callee does.  This
/// produces ZERO false positives because every `parent_write`
/// classification is explicit (came from a `#impure(parent_write)`
/// annotation in the stdlib or user code).
///
/// Trade-off vs the full `is_par_safe`: misses transitive
/// violations.  A worker that calls a user fn that calls
/// vector_add slips through.  But unlike the full check, it
/// never warns on a worker that's actually safe — making it
/// usable as a parser warning today, before the 5a annotation
/// sweep is comprehensive.
///
/// Returns `Some(callee_name)` if a direct ParentWrite call was
/// found; `None` otherwise.
#[allow(dead_code)]
#[must_use]
pub fn worker_calls_parent_write(data: &Data, worker_d_nr: u32) -> Option<String> {
    if worker_d_nr == u32::MAX || (worker_d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[worker_d_nr as usize];
    walk_shallow_parent_write(&def.code, data)
}

/// Plan-06 phase 5b' — DEEP parent-write check for par() workers.
/// Recurses through user-fn callees (those with a body) until it
/// finds a direct call to a `Purity::Impure(ParentWrite)` stdlib fn.
/// Returns the chain "worker → helper → bad_callee" or None.
///
/// Crucially, unannotated declared-only natives (Op*, n_*, t_* with
/// `code == Value::Null` and `purity == Unknown`) are treated as
/// safe — they're C primitives that don't write to parent state
/// unless explicitly tagged.  This is what avoids the 16 false
/// positives the strict `par_unsafe_reason` walk would produce.
///
/// `Purity::Impure(ParCall)` stdlib fns (parallel_for / _light)
/// are also safe — D8 R2 and D2.1.1 cover recursive Arc promotion.
pub fn worker_calls_parent_write_deep(data: &Data, worker_d_nr: u32) -> Option<String> {
    if worker_d_nr == u32::MAX || (worker_d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let mut visited = std::collections::HashSet::new();
    let def = &data.definitions[worker_d_nr as usize];
    let worker_name = def.name.strip_prefix("n_").unwrap_or(&def.name).to_string();
    walk_deep_parent_write(&def.code, data, worker_d_nr, &mut visited).map(|chain| {
        if chain == worker_name {
            chain
        } else {
            format!("{worker_name}{chain}")
        }
    })
}

/// Find a parent-state write reachable from `value`, following calls into user fns.
///
/// The recursive half of `worker_calls_parent_write_deep`: it answers with the chain
/// `" → helper → bad_callee"` for the first write it reaches, and `None` when every path
/// bottoms out in a pure, host-io or unannotated-native primitive.  `visited` stops a
/// recursive call graph from looping.
///
/// This is PRODUCTION-WIRED — `parse_parallel` turns a `Some` into the C93 `Level::Error`
/// that refuses the program — so a subtree it does not enter is a refusal that does not
/// happen, and the write then runs in a worker thread against a read-only store.  That is
/// why the fallback descends via the keystone instead of naming arms: the arm list below
/// is shared with `walk_par_unsafe_reason_value`, and a wrapper missing from both is a
/// verdict issued without looking.
fn walk_deep_parent_write(
    value: &Value,
    data: &Data,
    current_fn: u32,
    visited: &mut std::collections::HashSet<u32>,
) -> Option<String> {
    match value {
        Value::Call(callee, args) => {
            // @PLN102 C93 — a RAW field/element write (`OpSet*`) whose accessor root is a
            // NON-LOCAL PARAMETER is a write to captured/parent state: in a `par` worker the
            // captured state is read-only, so this is the race the deep check must reject
            // (the tagged-stdlib ParentWrite path below only catches `+=`/`hash_set`/… — a
            // bare `s.n = v` / `cap[i] = v` lowers to `OpSet*`, a "safe" primitive, and used
            // to slip through into a codegen slot-panic).  A write to a worker-LOCAL (or the
            // hidden return-buffer destination) is fine — same locality test as
            // `first_arg_is_local_var`.
            if let Some(var) = raw_write_to_captured(*callee, args, current_fn, data) {
                return Some(format!(" writes captured `{var}`"));
            }
            if let Some(chain) = call_deep_parent_write(*callee, args, data, current_fn, visited) {
                return Some(chain);
            }
            for a in args {
                if let Some(chain) = walk_deep_parent_write(a, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        // Don't recurse into CallRef target (callee unknown until runtime).
        Value::CallRef(_, args) => {
            for a in args {
                if let Some(chain) = walk_deep_parent_write(a, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Block(b) => {
            for v in &b.operators {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Insert(vs) => {
            for v in vs {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::If(c, t, e) => walk_deep_parent_write(c, data, current_fn, visited)
            .or_else(|| walk_deep_parent_write(t, data, current_fn, visited))
            .or_else(|| walk_deep_parent_write(e, data, current_fn, visited)),
        Value::Loop(body) => {
            for v in &body.operators {
                if let Some(chain) = walk_deep_parent_write(v, data, current_fn, visited) {
                    return Some(chain);
                }
            }
            None
        }
        Value::Set(_, rhs) => walk_deep_parent_write(rhs, data, current_fn, visited),
        Value::Span(b) => walk_deep_parent_write(&b.1, data, current_fn, visited),
        // Every other shape descends through the keystone, so a wrapper the arms above do
        // not name still has its subtree searched.  `walk_par_unsafe_reason_value` is a
        // near-copy of this arm list and ends the same way for the same reason.
        other => {
            let mut found = None;
            other.for_each_child(&mut |c| {
                if found.is_none() {
                    found = walk_deep_parent_write(c, data, current_fn, visited);
                }
            });
            found
        }
    }
}

/// Plan-06 phase 5b' G5 — for a `ParentWrite` callee, treat the
/// call as safe when its first argument is a LOCAL variable
/// (defined within the calling function, not a parameter).  Avoids
/// the false positive on `out: vector<integer> = []; out += [i]`
/// where `OpAppendVector(out, ...)` mutates a worker-local
/// vector — `out` was just allocated locally and isn't shared
/// with the parent.
///
/// This is a heuristic: a local variable could still hold a
/// reference to parent data (e.g. via `let v = some_param_field`),
/// so the rule is conservative for the common pattern but may
/// miss adversarial aliases.  Future precision work could track
/// per-var initialiser provenance.
fn first_arg_is_local_var(args: &[Value], current_fn: u32, data: &Data) -> bool {
    let Some(Value::Var(v)) = args.first() else {
        return false;
    };
    if current_fn == u32::MAX || (current_fn as usize) >= data.definitions.len() {
        return false;
    }
    let def = &data.definitions[current_fn as usize];
    if !def.variables.is_argument(*v) {
        return true;
    }
    // Plan-06 phase 5b' G5 — heap-typed return values are passed
    // via a hidden destination argument promoted by ref_return().
    // The promotion sets `argument: true` on the variable AND
    // marks the corresponding `def.attributes[…].hidden = true`.
    // Workers writing to these hidden destinations are populating
    // their own per-worker output buffer, NOT parent state.
    let name = def.variables.name(*v);
    def.attributes.iter().any(|a| a.hidden && a.name == name)
}

/// @PLN102 C93 — walk an accessor chain (`OpGet*(base, …)` / `Var`, span-transparent)
/// down to its root variable.  The base of an `OpSet*` write target is arg 0.
fn accessor_root_var(v: &Value, data: &Data) -> Option<u16> {
    match v {
        Value::Var(n) => Some(*n),
        Value::Span(b) => accessor_root_var(&b.1, data),
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.definitions[*d as usize].name.starts_with("OpGet") =>
        {
            args.first().and_then(|a| accessor_root_var(a, data))
        }
        _ => None,
    }
}

/// @PLN102 C93 — is this call a RAW field/element write (`OpSet*`) whose accessor root is
/// a NON-LOCAL PARAMETER of `current_fn` (captured/parent state)?  Returns the captured
/// var's name if so.  A write to a worker-LOCAL variable, or to the hidden return-buffer
/// destination (a promoted `ref_return` output), is safe.  Mirrors `first_arg_is_local_var`.
fn raw_write_to_captured(
    callee: u32,
    args: &[Value],
    current_fn: u32,
    data: &Data,
) -> Option<String> {
    if (callee as usize) >= data.definitions.len()
        || !data.definitions[callee as usize].name.starts_with("OpSet")
    {
        return None;
    }
    let root = args.first().and_then(|a| accessor_root_var(a, data))?;
    if current_fn == u32::MAX || (current_fn as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[current_fn as usize];
    if !def.variables.is_argument(root) {
        return None; // a worker-local write — fine
    }
    let name = def.variables.name(root).to_string();
    // hidden return-buffer destination = the worker's own output, not parent state.
    if def.attributes.iter().any(|a| a.hidden && a.name == name) {
        return None;
    }
    Some(name)
}

#[allow(dead_code)]
fn call_deep_parent_write(
    callee: u32,
    args: &[Value],
    data: &Data,
    current_fn: u32,
    visited: &mut std::collections::HashSet<u32>,
) -> Option<String> {
    if callee == u32::MAX || (callee as usize) >= data.definitions.len() {
        return None;
    }
    let def = &data.definitions[callee as usize];
    match def.purity {
        Purity::Pure
        | Purity::Impure(
            ImpureCategory::HostIo
            | ImpureCategory::Prng
            | ImpureCategory::Io
            | ImpureCategory::ParCall,
        ) => None,
        Purity::Impure(ImpureCategory::ParentWrite) => {
            if first_arg_is_local_var(args, current_fn, data) {
                None
            } else {
                Some(format!(
                    " → {}",
                    def.name.strip_prefix("n_").unwrap_or(&def.name)
                ))
            }
        }
        Purity::Unknown => {
            // Declared-only native (no body) → trust as safe.
            if matches!(def.code, Value::Null) {
                None
            } else if !visited.insert(callee) {
                // Cycle — optimistic short-circuit (consistent with
                // is_par_safe / fixpoint convergence).
                None
            } else {
                walk_deep_parent_write(&def.code, data, callee, visited).map(|chain| {
                    format!(
                        " → {}{}",
                        def.name.strip_prefix("n_").unwrap_or(&def.name),
                        chain
                    )
                })
            }
        }
    }
}

#[allow(dead_code)]
fn walk_shallow_parent_write(value: &Value, data: &Data) -> Option<String> {
    // "Shallow" = the runtime callee behind a `CallRef` is never followed
    // (it is not a child node); arg expressions ARE scanned.
    let mut found = None;
    value.any_node(&mut |n| {
        if let Value::Call(callee, _) = n
            && (*callee as usize) < data.definitions.len()
            && matches!(
                data.definitions[*callee as usize].purity,
                Purity::Impure(ImpureCategory::ParentWrite)
            )
        {
            found = Some(data.definitions[*callee as usize].name.clone());
            true
        } else {
            false
        }
    });
    found
}

#[cfg(test)]
mod par_shallow_tests {
    use super::worker_calls_parent_write;
    use crate::data::{Data, DefType, ImpureCategory, Purity, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    #[test]
    fn direct_parent_write_call_detected() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(bad, vec![]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }

    /// Pass-2 wave 4 regression: the hand-rolled walker had no `Return`
    /// arm (`_ => None`), so a parent-write call appearing only in
    /// `return f(...)` escaped the scan — the worker classified safe.
    /// The keystone descent sees every position.
    #[test]
    fn parent_write_in_return_value_detected() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Return(Box::new(Value::Call(bad, vec![])));
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }

    /// Same hole for a tuple element (`(f(...), 1)` result shapes).
    #[test]
    fn parent_write_in_tuple_element_detected() {
        let mut d = Data::new();
        let bad = d.add_def("hash_set", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code =
            Value::Tuple(vec![Value::Call(bad, vec![]), Value::Int(1)]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("hash_set".to_string())
        );
    }

    #[test]
    fn pure_call_not_detected() {
        let mut d = Data::new();
        let safe = d.add_def("min", &pos(), DefType::Function);
        d.definitions[safe as usize].purity = Purity::Pure;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(safe, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn unannotated_call_not_detected() {
        // Shallow check is precise: only fires for explicit
        // ParentWrite annotations.  Unknown stays None (the full
        // is_par_safe rejects this; shallow doesn't).
        let mut d = Data::new();
        let unknown = d.add_def("mystery", &pos(), DefType::Function);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(unknown, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn transitive_parent_write_not_detected() {
        // Worker calls inner; inner calls vector_add.  Shallow
        // does NOT recurse into inner — only the worker fn's
        // direct calls are checked.  Plan-06 phase 5b' (eventual)
        // adds transitive detection once 5a annotation coverage
        // is comprehensive enough not to false-positive.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let inner = d.add_def("inner", &pos(), DefType::Function);
        d.definitions[inner as usize].code = Value::Call(bad, vec![]);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(inner, vec![]);
        assert!(worker_calls_parent_write(&d, worker).is_none());
    }

    #[test]
    fn parent_write_inside_arg_detected() {
        // bad_call(vector_add(...)) — the arg evaluation is also
        // a parent-write site.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let safe = d.add_def("min", &pos(), DefType::Function);
        d.definitions[safe as usize].purity = Purity::Pure;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(safe, vec![Value::Call(bad, vec![])]);
        assert_eq!(
            worker_calls_parent_write(&d, worker),
            Some("vector_add".to_string())
        );
    }
}

#[cfg(test)]
mod par_deep_tests {
    use super::worker_calls_parent_write_deep;
    use crate::data::{Data, DefType, ImpureCategory, Purity, Value};
    use crate::lexer::Position;

    fn pos() -> Position {
        Position {
            file: String::new(),
            line: 0,
            pos: 0,
        }
    }

    /// A parent write reached only through a `Return` must still be found.
    ///
    /// `walk_deep_parent_write`'s arms are the same list as its twin
    /// `walk_par_unsafe_reason_value`'s, and neither included `Return` — so a worker whose
    /// body is `return helper(...)` was reported free of parent writes without that call ever
    /// being examined. Both now descend via the keystone; this pins the half that would
    /// otherwise drift back the moment one is edited alone.
    #[test]
    fn deep_parent_write_is_found_through_a_return() {
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Return(Box::new(Value::Call(bad, vec![])));
        assert!(
            worker_calls_parent_write_deep(&d, worker).is_some(),
            "a parent write behind a `return` must still be reported"
        );
    }

    #[test]
    fn deep_walks_through_user_helper() {
        // Worker calls helper; helper calls vector_add.  Deep walk
        // returns the chain.
        let mut d = Data::new();
        let bad = d.add_def("vector_add", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let helper = d.add_def("helper", &pos(), DefType::Function);
        d.definitions[helper as usize].code = Value::Call(bad, vec![]);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(helper, vec![]);
        let result = worker_calls_parent_write_deep(&d, worker);
        assert!(result.is_some());
        let chain = result.unwrap();
        assert!(chain.contains("worker"));
        assert!(chain.contains("helper"));
        assert!(chain.contains("vector_add"));
    }

    #[test]
    fn deep_skips_unannotated_native() {
        // Worker calls OpAddInt (Unknown + Value::Null) → safe.
        let mut d = Data::new();
        let op = d.add_def("OpAddInt", &pos(), DefType::Function);
        // purity defaults to Unknown, code defaults to Null
        let _ = op;
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(op, vec![]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_handles_recursive_user_fn() {
        // Worker calls itself — visited set prevents infinite loop.
        let mut d = Data::new();
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(worker, vec![]);
        // No parent-write reachable, even with the cycle.
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_par_call_callee_is_safe() {
        // ARC.md A4 (closed 2026-05-07) — `parallel_for_light` was
        // retired but any Impure(ParCall) callee still demonstrates
        // the same D8 R2 invariant: nested par() under recursive Arc
        // promotion is safe.  Use `parallel_queue` (a current
        // ParCall-purity callee) as the stand-in.
        let mut d = Data::new();
        let pf = d.add_def("parallel_queue", &pos(), DefType::Function);
        d.definitions[pf as usize].purity = Purity::Impure(ImpureCategory::ParCall);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(pf, vec![]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    #[test]
    fn deep_local_arg_to_parent_write_is_safe() {
        // Plan-06 phase 5b' G5 — calling a ParentWrite stdlib fn
        // on a worker-LOCAL variable is safe.  worker calls
        // OpAppendVector(local_v, ...); local_v isn't an arg.
        // Var(0) defaults to argument=false (local).
        let mut d = Data::new();
        let bad = d.add_def("OpAppendVector", &pos(), DefType::Function);
        d.definitions[bad as usize].purity = Purity::Impure(ImpureCategory::ParentWrite);
        let worker = d.add_def("worker", &pos(), DefType::Function);
        d.definitions[worker as usize].code = Value::Call(bad, vec![Value::Var(0)]);
        assert!(worker_calls_parent_write_deep(&d, worker).is_none());
    }

    // Note: the hidden-return-arg exception (`def.attributes[…].hidden`
    // case) is exercised end-to-end by par_struct_to_vector_t4 in
    // tests/threading_chars.rs.  A unit test would need to construct a
    // full Function with a promoted hidden attribute, which the
    // parser does multi-step; the integration test is the cleaner
    // verification.
}

// ── Plan-57 cluster I: store-lifetime guard (diagnostic) ─────────────────────
//
// Fires (under LOFT_STORE_GUARD) when a vector local's references are confined
// to one non-loop nested block, yet its backing `__vdb_N` store is scoped to an
// ancestor (function) and so frees late — the lifetime model under-freeing a
// block-confined store.  A detector for the watermark; once the model scopes
// such stores to their block it goes silent.  Read-only, gated, no behaviour
// change.
//
// Confinement = the least-common-ancestor of the block/loop scope-paths of every
// reference.  Tracking the full path (not just the innermost block) is required:
// a vector created in block B and read in a nested sub-block (nested `if`, a
// for-loop's `#For` block, or inside a loop body) is still confined to B — the
// LCA of `[B]` and `[B, sub]` is `B`.  Exact-scope-match misses these (probes
// 20/25/26).  The LCA's last element being a LOOP scope means the local lives
// only inside that loop (per-iteration reuse) → not relocatable.

/// Record a reference at the current scope-path: fold it into the running LCA
/// (longest common prefix of all reference paths).
fn guard_note(stack: &[(u16, bool)], lca: &mut Option<Vec<(u16, bool)>>) {
    *lca = Some(match lca.take() {
        None => stack.to_vec(),
        Some(prev) => prev
            .iter()
            .zip(stack)
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| *a)
            .collect(),
    });
}

fn guard_refs(
    node: &Value,
    target: u16,
    free_ref_nr: u32,
    stack: &mut Vec<(u16, bool)>,
    lca: &mut Option<Vec<(u16, bool)>>,
) {
    match node {
        Value::Var(v) if *v == target => guard_note(stack, lca),
        Value::Set(v, val) => {
            if *v == target && !matches!(val.unspan(), Value::Null) {
                guard_note(stack, lca);
            }
            guard_refs(val, target, free_ref_nr, stack, lca);
        }
        Value::Call(op, args) => {
            if *op == free_ref_nr
                && args.len() == 1
                && matches!(args[0].unspan(), Value::Var(v) if *v == target)
            {
                return;
            }
            for a in args {
                guard_refs(a, target, free_ref_nr, stack, lca);
            }
        }
        Value::Block(bl) => {
            stack.push((bl.scope, false));
            for op in &bl.operators {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
            stack.pop();
        }
        Value::Loop(lp) => {
            stack.push((lp.scope, true));
            for op in &lp.operators {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
            stack.pop();
        }
        Value::If(t, a, b) => {
            guard_refs(t, target, free_ref_nr, stack, lca);
            guard_refs(a, target, free_ref_nr, stack, lca);
            guard_refs(b, target, free_ref_nr, stack, lca);
        }
        Value::Iter(idx, c, n, e) => {
            if *idx == target {
                guard_note(stack, lca);
            }
            guard_refs(c, target, free_ref_nr, stack, lca);
            guard_refs(n, target, free_ref_nr, stack, lca);
            guard_refs(e, target, free_ref_nr, stack, lca);
        }
        Value::CallRef(v, args) => {
            if *v == target {
                guard_note(stack, lca);
            }
            for a in args {
                guard_refs(a, target, free_ref_nr, stack, lca);
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            guard_refs(v, target, free_ref_nr, stack, lca)
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for op in ops {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
        }
        Value::TupleGet(v, _) if *v == target => guard_note(stack, lca),
        Value::TuplePut(v, _, inner) => {
            if *v == target {
                guard_note(stack, lca);
            }
            guard_refs(inner, target, free_ref_nr, stack, lca);
        }
        Value::Span(b) => guard_refs(&b.1, target, free_ref_nr, stack, lca),
        _ => {}
    }
}

/// True if `target` is handed out of the function as-is — directly returned,
/// yielded, or broken out of a loop (`Return(Var(target))` etc.).  Such a local
/// escapes; its store must NOT be freed at block exit (probe 30).  (Catches the
/// direct form; an escape buried in a sub-expression like `return if c { a }`
/// is left for the fix's full escape analysis.)
/// True if `v` hands `target` out as-is: directly (`a`), or as a direct element
/// of a tuple / vector-literal value (`(a, n)`, `[a, b]`).  A *derived* value
/// (`a[0]`, `a.len()`) does NOT count — it produces a fresh value, not a's store.
fn escapes_value(v: &Value, target: u16) -> bool {
    match v.unspan() {
        Value::Var(t) => *t == target,
        Value::Tuple(elems) | Value::Insert(elems) => {
            elems.iter().any(|e| escapes_value(e, target))
        }
        _ => false,
    }
}

fn guard_escapes(node: &Value, target: u16) -> bool {
    node.any_node(&mut |n| match n {
        Value::Return(v) | Value::Yield(v) => escapes_value(v, target),
        // The block's VALUE is its last operator; if that hands out the local
        // (directly or in a tuple/literal), it escapes (block-result `x = {
        // …; a }` U3; `return (a, n)` t2).
        Value::Block(bl) | Value::Loop(bl) => bl
            .operators
            .last()
            .is_some_and(|o| escapes_value(o, target)),
        _ => false,
    })
}

/// Soundness gate for confining a *multi-store* local — one reassigned a fresh
/// store per block (the shared-`z`-across-`else`-blocks / shared-`x`-across-
/// match-arms shape).  Returns true iff every READ of `local` is dominated by a
/// non-null assignment of `local` earlier in the same straight-line block, so
/// the local never carries a store across a block boundary unreassigned.  When
/// that holds, freeing each store at *its* block exit cannot be a use-after-free
/// (the local's next read sees a freshly-assigned store, never the freed one).
/// Conditional assignments (inside `if`/`loop`) do NOT establish dominance for
/// code after the construct — the walk under-claims, so it stays sound.
fn confine_reassign_safe(code: &Value, local: u16) -> bool {
    let mut ok = true;
    dominance_walk(code, local, false, &mut ok, false);
    ok
}

/// Shared dominance walk behind the two Plan-57 soundness gates
/// (pass-3 dedupe: the two walkers were 80 identical lines apart, and the
/// stronger one had silently lost a child arm).  `dom` = "every read
/// of `local` here is preceded by a non-null assignment that definitely
/// executed".  The two gates differ ONLY in:
/// - the START value (`confine_reassign_safe` starts false — a definite
///   reassignment must precede any read; `store_dead_after_block` starts
///   true — the fn-level init counts);
/// - `invalidate_conditional` (`store_dead_after_block` only): a
///   reassignment inside an `If`/`Loop`/`Iter` body INVALIDATES
///   dominance — afterwards `local` may hold that block's confined store.
///
/// A read of `local` while `!dom` clears `ok`.
fn dominance_walk(node: &Value, local: u16, dom: bool, ok: &mut bool, inv: bool) -> bool {
    match node {
        Value::Set(v, val) => {
            // RHS evaluated before the write lands; a read of `local` in it
            // is gated by the *current* dominance.
            dominance_walk(val, local, dom, ok, inv);
            if *v == local {
                return !matches!(val.unspan(), Value::Null);
            }
            dom
        }
        Value::Var(v) | Value::TupleGet(v, _) => {
            if *v == local && !dom {
                *ok = false;
            }
            dom
        }
        Value::CallRef(v, args) => {
            if *v == local && !dom {
                *ok = false;
            }
            let mut d = dom;
            for a in args {
                d = dominance_walk(a, local, d, ok, inv);
            }
            dom
        }
        Value::Block(bl) => {
            let mut d = dom;
            for op in &bl.operators {
                d = dominance_walk(op, local, d, ok, inv);
            }
            d
        }
        Value::Loop(lp) => {
            // Body runs 0+ times → conditional; dominance does not leak out.
            let mut d = dom;
            for op in &lp.operators {
                d = dominance_walk(op, local, d, ok, inv);
            }
            if inv && assigns_local(node, local) {
                false
            } else {
                dom
            }
        }
        Value::If(t, a, b) => {
            let dc = dominance_walk(t, local, dom, ok, inv);
            dominance_walk(a, local, dc, ok, inv);
            dominance_walk(b, local, dc, ok, inv);
            // Branch assignments are conditional — they establish no
            // post-`if` dominance; under `inv` they additionally invalidate.
            if inv && (assigns_local(a, local) || assigns_local(b, local)) {
                false
            } else {
                dc
            }
        }
        Value::Iter(idx, c, n, e) => {
            if *idx == local && !dom {
                *ok = false;
            }
            let mut d = dominance_walk(c, local, dom, ok, inv); // the iteration SOURCE reads `local`
            d = dominance_walk(n, local, d, ok, inv);
            dominance_walk(e, local, d, ok, inv); // body conditional
            if inv && assigns_local(node, local) {
                false
            } else {
                d
            }
        }
        Value::Call(_, args) | Value::Insert(args) | Value::Tuple(args) | Value::Parallel(args) => {
            let mut d = dom;
            for a in args {
                d = dominance_walk(a, local, d, ok, inv);
            }
            d
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            dominance_walk(v, local, dom, ok, inv);
            dom
        }
        Value::TuplePut(v, _, inner) => {
            if *v == local && !dom {
                *ok = false;
            }
            dominance_walk(inner, local, dom, ok, inv);
            dom
        }
        Value::Span(b) => dominance_walk(&b.1, local, dom, ok, inv),
        _ => dom,
    }
}

/// Plan-57 cluster-III Route 2: recover the backer of an *orphaned* store — one
/// the single-valued `dep` no longer records because its holding local was
/// reassigned.  Returns the local `L` the store flows into via its repoint
/// `Set(L, OpGetField(Var(vdb), …))` (the canonical `z = [..]` lowering).
fn recover_backer(code: &Value, vdb: u16, gf_nr: u32) -> Option<u16> {
    let mut backer = None;
    code.any_node(&mut |n| {
        if let Value::Set(l, val) = n
            && let Value::Call(op, args) = val.unspan()
            && *op == gf_nr
            && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == vdb)
        {
            backer = Some(*l);
            true
        } else {
            false
        }
    });
    backer
}

/// Does `node` contain a non-null reassignment of `local` anywhere?
fn assigns_local(node: &Value, local: u16) -> bool {
    node.any_node(
        &mut |n| matches!(n, Value::Set(v, val) if *v == local && !matches!(val.unspan(), Value::Null)),
    )
}

/// Plan-57 cluster-III Route 2 soundness gate (STRONGER than `confine_reassign_safe`,
/// which only proves the backer is *defined* at every read — the fn-level init
/// satisfies that even when a confined block store is still live, an empirically
/// confirmed UAF via `for x in v` after the block).
///
/// Dominance walk over the body: `dom` = "`local` is known NOT to hold a confined
/// block store here" (it holds the fn-level init or an unconditional reassignment).
/// `dom` starts true and is **invalidated** by any CONDITIONAL reassignment
/// (inside an `If`/`Loop`/`Iter`) — afterwards `local` might hold that block's store,
/// which the fix would free at block exit.  A read of `local` while `!dom` is an
/// over-free hazard → unsound to confine.  Mirrors `confine_reassign_safe` but with
/// the conditional-reassignment invalidation (the missing soundness property).
fn store_dead_after_block(code: &Value, local: u16) -> bool {
    let mut ok = true;
    dominance_walk(code, local, true, &mut ok, true);
    ok
}

/// Per `__vdb` store, the LCA non-loop block scope it is provably confined to —
/// i.e. the scope at which it could be freed instead of at function exit.
/// Returns `vdb -> (backed local, block scope)` for every store-backed local
/// confined to a non-loop block deeper than where its store is currently
/// registered.  Two consumers share this one analysis:
/// - the cluster-I fix — re-register the confined `__vdb` (+ its local) at the
///   block scope so the standard block-exit `free_vars` sweep frees it there;
/// - the `LOFT_STORE_GUARD` detector ([`store_lifetime_guard`]) — a thin
///   wrapper that reports each entry.
///
/// Soundness (adversarially hardened across the probe rounds): excludes escapes
/// (return/yield/break, block-result, tuple/vector element via `guard_escapes`),
/// loop-internal confinement (per-iteration reuse, not a watermark), and any
/// store aliased by a variable that outlives block `b`.
/// @PLN35 `..rest` store-lifetime OBSERVER (reached only via `LOFT_REST_ORACLE`; never
/// rewrites IR). For every `__vdb` store it re-runs the `store_confinement` gates in
/// REPORTING mode and prints the verdict — CONFINED to a block, or REJECTED with the
/// exact gate — so a leak can be attributed to a precise decision (e.g. the ambiguous
/// dep-backer gate that blocks the escaping-field `..rest` shape). Diagnostic only.
fn rest_store_oracle(code: &Value, vars: &Function, free_ref_nr: u32, db_nr: u32, fn_name: &str) {
    let mut any = false;
    for vdb in 0..vars.count() {
        if !vars.name(vdb).starts_with("__vdb") {
            continue;
        }
        // Backers: every var whose dep carries this store.
        let mut backers: Vec<u16> = Vec::new();
        for v in 0..vars.count() {
            if vars.tp(v).depend().contains(&vdb) {
                backers.push(v);
            }
        }
        let user_backers: Vec<u16> = backers
            .iter()
            .copied()
            .filter(|&v| !vars.name(v).starts_with('_'))
            .collect();
        let temp_backers: Vec<u16> = backers
            .iter()
            .copied()
            .filter(|&v| vars.name(v).starts_with('_'))
            .collect();
        let arg_cap = backers
            .iter()
            .any(|&v| vars.is_argument(v) || vars.is_captured(v));
        if !any {
            eprintln!("[rest-oracle] fn={fn_name}");
            any = true;
        }
        let bnames: Vec<String> = backers.iter().map(|&v| vars.name(v).to_string()).collect();
        // Walk the same gate ladder store_confinement uses, but report the stop.
        let verdict = if arg_cap {
            "REJECT(arg/captured backer)".to_string()
        } else if backers.len() > 1 {
            // The gate that blocks the escaping-field `..rest` shape: the store is
            // dep-backed by the user vector AND a `_`-temp (`_elm`/`_comp`).
            format!(
                "REJECT(ambiguous: {} backers — user={:?} temp={:?})",
                backers.len(),
                user_backers
                    .iter()
                    .map(|&v| vars.name(v))
                    .collect::<Vec<_>>(),
                temp_backers
                    .iter()
                    .map(|&v| vars.name(v))
                    .collect::<Vec<_>>(),
            )
        } else if let Some(&local) = backers.first() {
            if vars.is_skip_free(local) || vars.is_skip_free(vdb) {
                "REJECT(skip_free — treated as escaping/borrowed)".to_string()
            } else if guard_escapes(code, local) {
                "REJECT(escapes: return/yield/break/element)".to_string()
            } else {
                let multi_store = vars.tp(local).depend().len() != 1;
                if multi_store && !confine_reassign_safe(code, local) {
                    "REJECT(multi-store, reassign-unsafe)".to_string()
                } else {
                    let span_target = if multi_store { vdb } else { local };
                    let mut stack: Vec<(u16, bool)> = Vec::new();
                    let mut lca: Option<Vec<(u16, bool)>> = None;
                    guard_refs(code, span_target, free_ref_nr, &mut stack, &mut lca);
                    match lca {
                        None => "REJECT(no ref LCA)".to_string(),
                        Some(path) if path.iter().any(|&(_, is_loop)| is_loop) => {
                            "REJECT(loop in LCA path — per-iteration reuse)".to_string()
                        }
                        Some(path) => match path.last() {
                            Some(&(b, _)) if vars.scope(vdb) == b => {
                                format!("already fn/block scope {b} (frees there)")
                            }
                            Some(&(b, _)) => format!("CONFINE to block {b}"),
                            None => "REJECT(empty LCA path)".to_string(),
                        },
                    }
                }
            }
        } else {
            "REJECT(no dep-backer — orphaned store)".to_string()
        };
        // THE DIRECT LEAK PREDICTOR (independent of confinement): does OpFreeRef(vdb)
        // execute BEFORE OpDatabase(vdb)? A value-type return hoists the allocating arm
        // into a `__ret` temp so the free lands after; a Reference / promoted-&text return
        // is NOT hoisted, so the free precedes the allocation → the store leaks. Pre-order
        // index of each op (Return(expr)→[expr], If(c,t,e)→[c,t,e]) approximates exec order.
        let mut ctr = 0usize;
        let mut alloc_at: Option<usize> = None;
        let mut free_at: Option<usize> = None;
        preorder_op_index(
            code,
            vdb,
            db_nr,
            free_ref_nr,
            &mut ctr,
            &mut alloc_at,
            &mut free_at,
        );
        let order = match (free_at, alloc_at) {
            (Some(f), Some(a)) if f < a => "FREE-before-ALLOC → LEAKS".to_string(),
            (Some(_), Some(_)) => "alloc-before-free → clean".to_string(),
            (Some(_), None) => "free, no alloc (null-only) → n/a".to_string(),
            (None, Some(_)) => "alloc, no free → LEAKS".to_string(),
            (None, None) => "neither → n/a".to_string(),
        };
        let ret_ty = block_result_type(code);
        eprintln!(
            "  store {} scope={} backers={bnames:?} ret={ret_ty} conf={verdict}",
            vars.name(vdb),
            vars.scope(vdb),
        );
        eprintln!("      free/alloc order: {order}");
    }
}

/// Pre-order (execution-approximating) index of the FIRST `OpDatabase(vdb)` and the FIRST
/// `OpFreeRef(vdb)` in `node`. `Return(expr)` unfolds to its inner (expr evaluates first);
/// `If(c,t,e)` visits cond then arms. Reporting helper for [`rest_store_oracle`].
fn preorder_op_index(
    node: &Value,
    vdb: u16,
    db_nr: u32,
    free_nr: u32,
    ctr: &mut usize,
    alloc_at: &mut Option<usize>,
    free_at: &mut Option<usize>,
) {
    *ctr += 1;
    if let Value::Call(op, args) = node.unspan()
        && let Some(Value::Var(v)) = args.first().map(Value::unspan)
        && *v == vdb
    {
        if *op == db_nr && alloc_at.is_none() {
            *alloc_at = Some(*ctr);
        }
        if *op == free_nr && free_at.is_none() {
            *free_at = Some(*ctr);
        }
    }
    node.for_each_child(&mut |c| preorder_op_index(c, vdb, db_nr, free_nr, ctr, alloc_at, free_at));
}

/// The result `Type` of a function body: the top-level `Block`'s declared result.
/// Reporting helper (the return type is the free-analysis hoist discriminator).
fn block_result_type(code: &Value) -> String {
    match code.unspan() {
        Value::Block(b) => {
            // The exact hoist gate the free-analysis (`insert_free`) uses.
            let hoists =
                is_value_return_type(&b.result) || matches!(b.result.base(), Type::Text(_));
            format!(
                "{:?}{}",
                b.result,
                if hoists { " (hoists)" } else { " (NO-hoist)" }
            )
        }
        _ => "<non-block>".to_string(),
    }
}

/// loft#750 — the result is a `BTreeMap`, and the ORDER is load-bearing.  Its
/// caller relocates each confined `__vdb`'s null-init, and a relocation that
/// cannot reach its block puts the init back at body position 0; run over
/// several confined stores, the visit order therefore PERMUTES the null-inits
/// at the head of the body — which moves the stack slots under them.  A
/// `HashMap` gave that order Rust's per-process hash seed, so compiling one
/// file twice with one binary produced different bytecode and different slots
/// (same answers, but no reproducible `--native` build, and a byte-identical-IR
/// inertness gate that could not tell "my change did nothing" from "the seed
/// moved").  Keyed by variable number, the visit order is now the declaration
/// order.
fn store_confinement(
    code: &Value,
    vars: &Function,
    free_ref_nr: u32,
    gf_nr: u32,
) -> BTreeMap<u16, (u16, u16)> {
    // Plan-57 cluster-III Route 2: recover the backer of an orphaned (overwritten) store so
    // its per-block store can confine.  DEFAULT ON since 2026-08-21; `LOFT_NO_CONF_RECOVER=1`
    // emits the pre-Route-2 form and is the first bisect step for a wrong answer in a
    // function that reassigns a local across sibling blocks.
    //
    // A local reassigned across sibling `if`/`else if`/`match` arms otherwise keeps EVERY
    // arm's store to scope exit, so the store watermark grows with the number of
    // reassignment SITES and not with how many of them run: a 16-site function measured
    // peak 20 whichever single arm was taken, against a flat 5 with this on.
    //
    // What makes it safe is `store_dead_after_block`, not this flag: a local READ after the
    // blocks does not confine, because freeing a confined block store while the local still
    // holds it returns the wrong element on the branch that did NOT run.  That shape refuted
    // the first gate design and is pinned by
    // `tests/scripts/reassign-across-sibling-blocks.loft`, which asserts the same answers
    // with the recovery on and off.
    let recover = std::env::var("LOFT_NO_CONF_RECOVER").is_err();
    let mut out: BTreeMap<u16, (u16, u16)> = BTreeMap::new();
    for vdb in 0..vars.count() {
        if !vars.name(vdb).starts_with("__vdb") {
            continue;
        }
        // A caller-provided return buffer arrives as a PARAMETER, so it is not this
        // function's store to confine — and "free it at the inner block's exit" would
        // have the callee free the CALLER's buffer.  The backer check below rejects an
        // argument BACKER but never asked whether the store itself is one.
        //
        // `return <vector local>` lowers to exactly that shape: the incoming `__vdb`
        // becomes the buffer and the named local a field-view of it
        // (`buf = OpGetField(__vdb_1, …); return __vdb_1`), so the local is a non-arg
        // backer of an arg store and every such function was reported.  That is the
        // most ordinary vector idiom in the language — `cbor`'s `head` is the shape
        // that surfaced it — so the report was noise wherever anyone looked.
        if vars.is_argument(vdb) {
            continue;
        }
        // The single local that holds `vdb` (vdb in its dep), non-arg,
        // non-captured.  A *single-store* local (dep == [vdb]) shares its store's
        // span; a *multi-store* local (dep ⊇ vdb, reassigned a fresh store per
        // block — shared `z` across `else`-blocks, shared `x` across match arms)
        // spans the whole function even though each store lives in one block.
        let mut backed: Option<u16> = None;
        let mut ambiguous = false;
        for v in 0..vars.count() {
            if vars.tp(v).depend().contains(&vdb) {
                if vars.is_argument(v) || vars.is_captured(v) || backed.is_some() {
                    ambiguous = true;
                    break;
                }
                backed = Some(v);
            }
        }
        if ambiguous {
            continue;
        }
        // Route 2: an orphaned store has no dep-backer (single-valued dep dropped
        // the link when the local was reassigned).  Recover the local it flows into
        // via its `OpGetField` repoint; the recovered backer is necessarily
        // multi-store.  Gated off by default until soundness is locked in.
        let recovered = backed.is_none();
        let local = if let Some(l) = backed {
            l
        } else if recover {
            match recover_backer(code, vdb, gf_nr) {
                Some(l) if !vars.is_argument(l) && !vars.is_captured(l) => l,
                _ => continue,
            }
        } else {
            continue;
        };
        // An escaping local hands its store to the caller — freeing it at block
        // exit is a use-after-free.  Exclude direct returns/yields/breaks
        // (probe 30) and anything scope-analysis already marked skip-free.
        if vars.is_skip_free(local) || vars.is_skip_free(vdb) || guard_escapes(code, local) {
            continue;
        }
        let multi_store = recovered || vars.tp(local).depend().len() != 1;
        // A multi-store local must never carry `vdb`'s store across a block
        // boundary unreassigned, else block-exit freeing is a UAF.  Gate on the
        // write-dominates-read walk before trusting the per-store span.
        if multi_store && !confine_reassign_safe(code, local) {
            continue;
        }
        // Confinement block = the LCA non-loop block of the *store's own* refs
        // (multi-store) or the local's (single-store — equal to the store's, kept
        // for the U3 alias path below).
        let span_target = if multi_store { vdb } else { local };
        let mut stack: Vec<(u16, bool)> = Vec::new();
        let mut lca: Option<Vec<(u16, bool)>> = None;
        guard_refs(code, span_target, free_ref_nr, &mut stack, &mut lca);
        // Confined iff the LCA path is a non-empty chain of NON-LOOP blocks
        // (NO loop anywhere in it — a confinement *inside* a loop is per-
        // iteration reuse, not a watermark; probes 33/34), and the innermost
        // block is deeper than where the store is currently scoped.
        if let Some(path) = lca
            && let Some(&(b, _)) = path.last()
            && path.iter().all(|&(_, is_loop)| !is_loop)
            && vars.scope(vdb) != b
            // Route 2 soundness: the recovered backer's block store must be dead
            // after its block on EVERY path — `confine_reassign_safe` only proves
            // the backer is *defined* at reads (the fn-level init satisfies that),
            // so an extra "no body-scope read of the backer" gate is required.
            && (!recovered || store_dead_after_block(code, local))
            // dep-escape: the store must not be aliased by a USER variable that
            // OUTLIVES block `b`.  A block-result `x = { …; a }` gives x the dep
            // `["a"]` and x is read at function level (U3) — freeing a here would
            // corrupt x.  Compiler temps (`_elm`, `__vdb`) are confined to their
            // own block and, for a multi-store local, may legitimately alias the
            // local in a *sibling* block (holding a different store there) — so
            // skip them rather than false-positive.
            && !(0..vars.count()).any(|w| {
                w != local
                    && !vars.name(w).starts_with('_')
                    && vars.tp(w).depend().contains(&local)
                    && {
                        let mut wst: Vec<(u16, bool)> = Vec::new();
                        let mut wlca: Option<Vec<(u16, bool)>> = None;
                        guard_refs(code, w, free_ref_nr, &mut wst, &mut wlca);
                        // w aliases the store AND is referenced outside `b`
                        // (b absent from w's confinement path) ⇒ outlives it.
                        wlca.is_some_and(|p| !p.iter().any(|&(s, _)| s == b))
                    }
            })
        {
            out.insert(vdb, (local, b));
        }
    }
    out
}

/// `LOFT_STORE_GUARD` detector — reports each store-backed local that frees at
/// function exit despite being confined to an inner block.  Thin wrapper over
/// [`store_confinement`] (the same analysis that drives the cluster-I fix).
/// Returns the number of late-freed stores.
fn store_lifetime_guard(
    code: &Value,
    vars: &Function,
    free_ref_nr: u32,
    gf_nr: u32,
    fn_name: &str,
) -> usize {
    let confined = store_confinement(code, vars, free_ref_nr, gf_nr);
    for (&vdb, &(local, b)) in &confined {
        eprintln!(
            "[store-guard] {fn_name}: store {} (local '{}') confined to block scope {b} but stored at scope {} — frees late",
            vars.name(vdb),
            vars.name(local),
            vars.scope(vdb),
        );
    }
    confined.len()
}

/// Which store a `Set`'s RHS binds the assigned local to (so the local now "holds"
/// that store's data): `OpGetField(Var(s), …)` → `s` (the repoint idiom); a copy
/// `Var(other)` → whatever `other` currently holds; anything else → unbound.
fn binding_source(val: &Value, gf_nr: u32, holds: &HashMap<u16, u16>) -> Option<u16> {
    match val.unspan() {
        Value::Call(op, args) if *op == gf_nr => match args.first().map(Value::unspan) {
            Some(Value::Var(s)) => Some(*s),
            _ => None,
        },
        Value::Var(other) => holds.get(other).copied(),
        _ => None,
    }
}

/// Flow walk: trace which store each local **holds** so each store's *data*
/// liveness is recovered — `alloc` (its `OpDatabase`), `last_read` (last read of
/// its data via a holding local or a direct build op, EXCLUDING the scope-exit
/// `OpFreeRef`), and `dead` (the point a holding local is rebound away, the
/// reassignment case).  This is the real liveness `compute_intervals` cannot give
/// (its `last_use` is pinned to the teardown `OpFreeRef`).  Sequential approximation
/// across branches — fine for the straight-line / sequential shapes this targets.
#[allow(clippy::too_many_arguments)]
fn store_liveness_walk(
    node: &Value,
    seq: &mut u32,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    holds: &mut HashMap<u16, u16>,
    alloc: &mut HashMap<u16, u32>,
    dead: &mut HashMap<u16, u32>,
    last_read: &mut HashMap<u16, u32>,
) {
    *seq += 1;
    let s = *seq;
    match node {
        Value::Var(v) => {
            if let Some(&store) = holds.get(v) {
                last_read.insert(store, s);
            }
        }
        Value::Set(local, val) => {
            if matches!(val.unspan(), Value::Null) {
                return; // null-init — not a real def/rebind
            }
            store_liveness_walk(val, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            let new_store = binding_source(val, gf_nr, holds);
            if let Some(&old) = holds.get(local)
                && Some(old) != new_store
            {
                dead.entry(old).or_insert(s); // local rebound away → old store dead here
            }
            match new_store {
                Some(st) => {
                    holds.insert(*local, st);
                }
                None => {
                    holds.remove(local);
                }
            }
        }
        Value::Call(op, args) => {
            if *op == fr_nr && args.len() == 1 && matches!(args[0].unspan(), Value::Var(_)) {
                return; // OpFreeRef — not a data read
            }
            if *op == db_nr {
                if let Some(Value::Var(st)) = args.first().map(Value::unspan) {
                    alloc.insert(*st, s);
                }
                return; // OpDatabase(store, size) — alloc point, args are not data reads
            }
            for a in args {
                store_liveness_walk(a, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                store_liveness_walk(op, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::If(c, t, e) => {
            store_liveness_walk(c, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            store_liveness_walk(t, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            store_liveness_walk(e, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for op in ops {
                store_liveness_walk(op, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            store_liveness_walk(v, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
        }
        Value::Span(b) => {
            store_liveness_walk(
                &b.1, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read,
            );
        }
        _ => {}
    }
}

/// True if `node` contains the `OpDatabase(Var(store), …)` allocation of `store`.
fn contains_alloc(node: &Value, store: u16, db_nr: u32) -> bool {
    node.any_node(&mut |n| {
        matches!(n, Value::Call(op, args) if *op == db_nr
        && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == store))
    })
}

/// Plan-57 Phase-3 soundness gate (dominance): true only if `store`'s `OpDatabase`
/// allocation is reached **unconditionally** from the body — never gated by an
/// `If`/`Loop`/`Parallel` branch.  A reclaim that early-frees / drops the scope-exit
/// free of a store allocated in an untaken branch leaks or double-frees
/// (`20-binary`).  Plain nested blocks always run, so they stay unconditional.
fn contains_alloc_unconditional(node: &Value, store: u16, db_nr: u32) -> bool {
    match node.unspan() {
        Value::Call(op, args) => {
            (*op == db_nr
                && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == store))
                || args
                    .iter()
                    .any(|a| contains_alloc_unconditional(a, store, db_nr))
        }
        Value::Set(_, val) => contains_alloc_unconditional(val, store, db_nr),
        Value::Block(bl) => bl
            .operators
            .iter()
            .any(|o| contains_alloc_unconditional(o, store, db_nr)),
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            contains_alloc_unconditional(v, store, db_nr)
        }
        Value::Insert(ops) | Value::Tuple(ops) => ops
            .iter()
            .any(|o| contains_alloc_unconditional(o, store, db_nr)),
        Value::Span(b) => contains_alloc_unconditional(&b.1, store, db_nr),
        // If / Loop / Parallel / Iter — conditional, so an alloc inside is
        // NOT unconditional.
        _ => false,
    }
}

/// Plan-57 Phase-3 soundness gate (retention): true if a `holder` var (the store or
/// any local that holds it) appears in a value position that could keep the store's
/// data reachable **past the early-free point** — embedded in a tuple/vector literal,
/// stored as a struct-field / keyed value, returned/yielded, copied to an alias, or
/// passed as a non-receiver argument.  A holder as the FIRST argument of a call (the
/// receiver of an index/len/field read, or an in-place append target) does not
/// retain — that is exactly the straight-line shape this pass targets.  Everything
/// else is treated as retention (conservative — errs toward leaving the store alone).
fn holder_retained(node: &Value, holders: &HashSet<u16>) -> bool {
    match node {
        Value::Var(h) => holders.contains(h),
        Value::Call(_, args) | Value::CallRef(_, args) => args.iter().enumerate().any(|(i, a)| {
            // The receiver (first arg) being a bare holder Var is a read, not a
            // retention; any nested expression there is still scanned.
            if i == 0 && matches!(a.unspan(), Value::Var(h) if holders.contains(h)) {
                false
            } else {
                holder_retained(a, holders)
            }
        }),
        Value::Set(_, val) => holder_retained(val, holders),
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter().any(|o| holder_retained(o, holders))
        }
        Value::If(c, t, e) => {
            holder_retained(c, holders)
                || holder_retained(t, holders)
                || holder_retained(e, holders)
        }
        Value::Insert(xs) | Value::Tuple(xs) | Value::Parallel(xs) => {
            xs.iter().any(|x| holder_retained(x, holders))
        }
        Value::Return(v) | Value::Yield(v) | Value::Drop(v) => holder_retained(v, holders),
        Value::TuplePut(_, _, v) => holder_retained(v, holders),
        Value::Iter(_, c, n, e) => {
            holder_retained(c, holders)
                || holder_retained(n, holders)
                || holder_retained(e, holders)
        }
        Value::Span(b) => holder_retained(&b.1, holders),
        _ => false,
    }
}

/// #426B — every var that transitively reaches store `st` through the dep graph.
///
/// `reclaim_safe`'s holder model needs the full closure, not the one-hop depers:
/// a binding can borrow a store INDIRECTLY through an intermediate local — a
/// fn-return-of-index `b = idx0(ww){ w[0] }` binds `b` with dep `["ww"]`, and
/// `ww` deps `["__vdb_1"]`, so `b` holds `__vdb_1` via `ww`.  Missing `b` here
/// lets reclaim free `__vdb_1` before `b`'s last read (store-reuse-after-free).
///
/// Walks to a fixpoint: a var is a deper of `st` if its dep list contains `st`
/// or contains any var already known to be a deper.  Marker deps (`u16::MAX`
/// one-buffer sentinel, the `0x8000` callee-frame tag) name no frame var and are
/// skipped.  Bounded by `vars.count()` iterations (each pass adds at least one
/// var or stops), so it always terminates.
fn transitive_depers(vars: &Function, st: u16) -> Vec<u16> {
    let n = vars.count();
    let mut reaches: HashSet<u16> = HashSet::new();
    loop {
        let mut added = false;
        for v in 0..n {
            if reaches.contains(&v) {
                continue;
            }
            let deps_st = vars
                .tp(v)
                .depend()
                .into_iter()
                .any(|d| d != u16::MAX && d & 0x8000 == 0 && (d == st || reaches.contains(&d)));
            if deps_st {
                reaches.insert(v);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let mut out: Vec<u16> = reaches.into_iter().collect();
    out.sort_unstable();
    out
}

/// Plan-57 Phase-3 soundness gate: is store `st` (a `__vdb` work-ref) safe to
/// early-free + relocate + strip-its-scope-exit-free?  Mirrors `store_confinement`'s
/// per-store predicates (the I-a soundness model) for the function-scoped case:
///
/// - not `skip_free` / `captured` / a `RefVar` alias, and not directly escaping
///   (`guard_escapes`);
/// - every var that *holds* it (`st ∈ depend`) is a non-arg, non-captured,
///   non-`skip_free`, non-`RefVar`, non-escaping local; at most one is a *user*
///   local; a multi-store user local must pass `confine_reassign_safe`;
/// - none of the holders is *retained* anywhere (`holder_retained`).
///
/// Orphaned stores (no holder — the reassignment case, `__vdb_1..10` in probe 14)
/// are safe-because-dead: nothing reaches them after the rebind.  Conservative by
/// design — an escape/alias/capture/struct-field case falls back to the (sound)
/// scope-exit free.
fn reclaim_safe(code: &Value, vars: &Function, st: u16) -> bool {
    if !vars.name(st).starts_with("__vdb") {
        return false;
    }
    if vars.is_skip_free(st) || vars.is_captured(st) || matches!(vars.tp(st), Type::RefVar(_)) {
        return false;
    }
    if guard_escapes(code, st) {
        return false;
    }
    // #426B — the holder set must be the TRANSITIVE dep closure, not just the
    // one-hop depers.  A view-of-a-view keeps `st` live through an intermediate
    // local: `b = idx0(ww)` where `idx0` returns `w[0]` binds `b` with dep
    // `["ww"]`, and `ww` deps `["__vdb_1"]` — so `b` transitively holds
    // `__vdb_1` and extends its lifetime to `b`'s last use.  A single-hop scan
    // sees only `ww` (the receiver arg of the call, treated as a read, not a
    // retention), misses `b`, and reclaim frees `__vdb_1` right after the call —
    // before `b` reads it.  The freed slot is then recycled into the next
    // allocation, corrupting `b` (store-reuse-after-free).  Walking the dep
    // graph to fixpoint makes `b` a holder, so the retention scan / multi-user-
    // local gate below leave `st`'s sound scope-exit free in place.
    let depers: Vec<u16> = transitive_depers(vars, st);
    let mut user_locals = 0;
    for &v in &depers {
        if vars.is_argument(v)
            || vars.is_captured(v)
            || vars.is_skip_free(v)
            || matches!(vars.tp(v), Type::RefVar(_))
            || guard_escapes(code, v)
        {
            return false;
        }
        if !vars.name(v).starts_with('_') {
            user_locals += 1;
            if vars.tp(v).depend().len() != 1 && !confine_reassign_safe(code, v) {
                return false;
            }
        }
    }
    if user_locals > 1 {
        return false; // multiple live aliases — leave the store alone
    }
    // Holders for the retention scan: the raw store plus its NON-`_` user-var
    // holders.  `_`-prefixed compiler temps (`_elm`, nested `__vdb`) are build-
    // internal — confined to the store they construct, not external aliases — so
    // they are skipped, mirroring `store_confinement`'s dep-escape treatment.
    // Without this, a comprehension's per-iteration element record (`_elm`, which
    // deps the result `__vdb`) false-positives as retention.
    let mut holders: HashSet<u16> = depers
        .into_iter()
        .filter(|&v| !vars.name(v).starts_with('_'))
        .collect();
    holders.insert(st);
    !holder_retained(code, &holders)
}

/// Plan-57 — the reclaim PLAN, the single source of truth shared by
/// [`lastuse_reclaim`] (which acts on it) and [`reclaim_unfreed_eligible`] (the
/// Phase-4 guard, which verifies the frees landed), so the two cannot drift.
/// Returns `(owning, intent)`:
/// - `owning` — function-scoped owning stores passing the dominance + soundness
///   gates (the ones whose null-init may relocate);
/// - `intent` — `(store, trigger)` pairs: `store`'s data dies before the eligible
///   sibling `trigger` allocates, so `store` must be freed before `trigger`'s build.
fn reclaim_free_intent(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
) -> (Vec<u16>, Vec<(u16, u16)>) {
    let body_scope = match code.unspan() {
        Value::Block(bl) => bl.scope,
        _ => return (Vec::new(), Vec::new()),
    };
    let mut holds: HashMap<u16, u16> = HashMap::new();
    let mut alloc: HashMap<u16, u32> = HashMap::new();
    let mut dead: HashMap<u16, u32> = HashMap::new();
    let mut last_read: HashMap<u16, u32> = HashMap::new();
    let mut seq = 0u32;
    store_liveness_walk(
        code,
        &mut seq,
        db_nr,
        gf_nr,
        fr_nr,
        &mut holds,
        &mut alloc,
        &mut dead,
        &mut last_read,
    );
    let dead_at = |st: u16| {
        dead.get(&st)
            .copied()
            .or_else(|| last_read.get(&st).copied())
    };
    let mut owning: Vec<u16> = alloc
        .keys()
        .copied()
        .filter(|&st| {
            vars.scope(st) == body_scope
                && contains_alloc_unconditional(code, st, db_nr)
                && reclaim_safe(code, vars, st)
        })
        .collect();
    owning.sort_unstable();
    let mut intent: Vec<(u16, u16)> = Vec::new();
    for &st in &owning {
        let Some(d) = dead_at(st) else { continue };
        if let Some(&later) = owning
            .iter()
            .filter(|&&w| w != st && alloc.get(&w).is_some_and(|&a| a > d))
            .min_by_key(|&&w| alloc[&w])
        {
            intent.push((st, later));
        }
    }
    (owning, intent)
}

/// True if `op` is a top-level `OpFreeRef(Var(st))`.
fn is_top_free(op: &Value, st: u16, fr_nr: u32) -> bool {
    matches!(op.unspan(), Value::Call(o, args) if *o == fr_nr
        && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == st))
}

/// Plan-57 Phase-4 guard (Goal-E enforcement): after [`lastuse_reclaim`] has run,
/// every store in the reclaim plan's `intent` must have its `OpFreeRef` placed at
/// body top-level BEFORE the op that allocates its `trigger`.  Returns the count of
/// reclaim-eligible stores left live-but-dead past a later alloc — must be 0.  A
/// non-zero result means reclaim silently failed to stop a store the model says is
/// dead (a regression the watermark rule must not re-acquire).  Escape/alias cases
/// are not in `intent` (the soundness gate excluded them) — they legitimately keep
/// their scope-exit free and are not asserted on.
fn reclaim_unfreed_eligible(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
) -> usize {
    let (_owning, intent) = reclaim_free_intent(code, vars, db_nr, gf_nr, fr_nr);
    let Value::Block(bl) = code.unspan() else {
        return 0;
    };
    let mut count = 0;
    for &(st, later) in &intent {
        let free_idx = bl.operators.iter().position(|o| is_top_free(o, st, fr_nr));
        let alloc_idx = bl
            .operators
            .iter()
            .position(|o| contains_alloc(o, later, db_nr));
        match (free_idx, alloc_idx) {
            (Some(f), Some(a)) if f < a => {} // freed before the trigger allocates — good
            _ => count += 1,
        }
    }
    count
}

/// Plan-57 last-use freeing, Phase 3 — reclaim via null-init RELOCATION + early
/// free (gated `LASTUSE_RECLAIM`).
///
/// Phase 2 proved free-alone is inert: every `__vdb`'s null-init (`Set(vdb, Null)`,
/// hoisted to body-0 by `parse_code`) ALLOCATES its store up front, so the runtime
/// watermark is locked before any inserted free can run (probe 14: peak 11 = the 11
/// null-inits stacking at body-0).  This pass closes that with two coordinated edits
/// per dead store:
///
/// 1. **Relocate the null-init** out of body-0 to immediately before its own
///    `OpDatabase` build — so the stores stop batching at body-0 and allocate
///    interleaved.  (The I-a `relocate_null_init` lever, applied to a body *index*
///    instead of a sub-block.)
/// 2. **Early free** before the next store allocates — so a freed slot is reused by
///    the following null-init (`+alloc, -free, +alloc, -free`) instead of stacking.
///
/// The scope-exit `OpFreeRef` is left in place as an idempotent double-free
/// (`free_named` no-ops an already-free store — measured safe in Phase 2).  Both
/// edits run **before** `compute_intervals`, so the moved `first_def` is reflected
/// in the slot intervals.  Flat-body straight-line / sequential shapes only — the
/// I-b / III-straight-line cases block-confinement (I-a) cannot reach.  Returns the
/// count of relocations + frees applied.
fn lastuse_reclaim(code: &mut Value, vars: &Function, db_nr: u32, gf_nr: u32, fr_nr: u32) -> usize {
    // Eligibility + free-intent come from the shared plan, so the Phase-4 guard
    // (`reclaim_unfreed_eligible`) verifies exactly what this pass acts on.
    let (owning, intent) = reclaim_free_intent(code, vars, db_nr, gf_nr, fr_nr);
    if owning.is_empty() {
        return 0;
    }
    let Value::Block(bl) = code else { return 0 };
    // #260 Fix B replaced Fix A here: native codegen now declares every
    // `__vdb` local up front (sentinel-bound prologue, `generation/mod.rs::
    // output_function`), so relocating a null-init below an early-return
    // scope-exit free can no longer strand the free's `var_…` reference out
    // of scope (rustc E0425) — the `has_free_before_alloc` exclusion guard
    // is gone and those stores get their watermark reclaim back (46/46
    // owning stores were forfeited in brick-buster's generator pre-B).
    // Early-free groups: before[trigger] = dead stores to free right before
    // `trigger` allocates.  Their scope-exit `OpFreeRef` is REMOVED, not kept as an
    // "idempotent double-free": under reclaim the freed slot is reused by a later
    // store, so a stale scope-exit free of `st` would target a *different live
    // owner's* store (the tag gate catches exactly this).  The early free becomes
    // the store's sole free.
    let mut before: HashMap<u16, Vec<u16>> = HashMap::new();
    for &(st, later) in &intent {
        before.entry(later).or_default().push(st);
    }
    let freed_set: HashSet<u16> = intent.iter().map(|&(st, _)| st).collect();
    // Reloc set: owning stores whose null-init sits at body top-level AND whose
    // OpDatabase build is a top-level body op (so the null-init can be placed right
    // before it).  Excludes any store I-a already relocated into a sub-block.
    let reloc: Vec<u16> = owning
        .iter()
        .copied()
        .filter(|&st| {
            bl.operators.iter().any(|o| is_var_null_init(o, st))
                && bl.operators.iter().any(|o| contains_alloc(o, st, db_nr))
        })
        .collect();
    // Pull the relocatable null-inits out of the body (keyed by store), and DROP the
    // existing scope-exit `OpFreeRef(Var(st))` for every store we early-free.
    let mut saved: HashMap<u16, Value> = HashMap::new();
    let kept: Vec<Value> = std::mem::take(&mut bl.operators)
        .into_iter()
        .filter_map(|op| {
            if let Some(&st) = reloc.iter().find(|&&st| is_var_null_init(&op, st)) {
                saved.insert(st, op);
                return None;
            }
            if let Value::Call(o, args) = op.unspan()
                && *o == fr_nr
                && let Some(Value::Var(v)) = args.first().map(Value::unspan)
                && freed_set.contains(v)
            {
                return None; // scope-exit free of an early-freed store — drop it
            }
            Some(op)
        })
        .collect();
    // Rebuild: before each op that allocates store `st`, emit the early frees of the
    // stores that died before it, then `st`'s relocated null-init, then the op.
    let mut count = 0;
    for op in kept {
        let allocs_here: Vec<u16> = owning
            .iter()
            .copied()
            .filter(|&st| contains_alloc(&op, st, db_nr))
            .collect();
        for &st in &allocs_here {
            if let Some(frees) = before.get(&st) {
                for &f in frees {
                    bl.operators.push(Value::Call(fr_nr, vec![Value::Var(f)]));
                    count += 1;
                }
            }
        }
        for &st in &allocs_here {
            if let Some(ni) = saved.remove(&st) {
                bl.operators.push(ni);
                count += 1;
            }
        }
        bl.operators.push(op);
    }
    // Safety: restore any null-init whose alloc was not matched, so first_def is
    // never lost (should not happen given the reloc filter, but keep it sound).
    for (_st, ni) in saved {
        bl.operators.insert(0, ni);
    }
    count
}

/// Per-function allocation-site id for a store-owning var (1-based; 0 is the
/// "untagged" sentinel). Stable within a function so a var's `OpDatabase` and its
/// `OpFreeRef` share the same id; the global `counter` keeps ids unique across
/// functions so a cross-function wrong-store free mismatches.
fn store_site_id(v: u16, ids: &mut HashMap<u16, u16>, counter: &mut u16) -> u16 {
    *ids.entry(v).or_insert_with(|| {
        let id = *counter;
        *counter = counter.wrapping_add(1);
        if *counter == 0 {
            *counter = 1;
        }
        id
    })
}

/// Plan-57 store-identity gate (Phase 2.5) — gated IR post-pass (`LOFT_STORE_TAG`).
///
/// Rewrites store ops to their verifying variants so a free can be checked against
/// the allocation that owns the store: insert `OpStoreTag(vdb, id)` right after each
/// `OpDatabase(vdb, …)`, and replace `OpFreeRef(vdb)` with `OpFreeRefTag(vdb, id)`.
/// Normal builds (no env) never run this, so the bytecode stays byte-identical.
#[allow(clippy::too_many_arguments)]
fn tag_stores(
    code: &mut Value,
    db_nr: u32,
    fr_nr: u32,
    store_tag_nr: u32,
    free_ref_tag_nr: u32,
    tagset: &HashSet<u16>,
    ids: &mut HashMap<u16, u16>,
    counter: &mut u16,
) {
    match code {
        Value::Block(bl) | Value::Loop(bl) => {
            let mut i = 0;
            while i < bl.operators.len() {
                tag_stores(
                    &mut bl.operators[i],
                    db_nr,
                    fr_nr,
                    store_tag_nr,
                    free_ref_tag_nr,
                    tagset,
                    ids,
                    counter,
                );
                // Identify a top-level OpDatabase / OpFreeRef on a tracked Var.  Only
                // reclaim-eligible stores (`tagset`) are tagged/verified — adopted /
                // shared / file stores carry no tag (no OpStoreTag) and keep their
                // plain OpFreeRef, so the gate cannot false-positive on them.
                let hit = match bl.operators[i].unspan() {
                    Value::Call(op, args) if *op == db_nr || (*op == fr_nr && args.len() == 1) => {
                        match args.first().map(Value::unspan) {
                            Some(Value::Var(v)) if tagset.contains(v) => Some((*op == db_nr, *v)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some((is_alloc, vdb)) = hit {
                    let id = i32::from(store_site_id(vdb, ids, counter));
                    if is_alloc {
                        bl.operators.insert(
                            i + 1,
                            Value::Call(store_tag_nr, vec![Value::Var(vdb), Value::Int(id)]),
                        );
                        i += 1; // skip the inserted tag op
                    } else {
                        bl.operators[i] =
                            Value::Call(free_ref_tag_nr, vec![Value::Var(vdb), Value::Int(id)]);
                    }
                }
                i += 1;
            }
        }
        Value::If(c, t, e) => {
            tag_stores(
                c,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
            tag_stores(
                t,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
            tag_stores(
                e,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for o in ops {
                tag_stores(
                    o,
                    db_nr,
                    fr_nr,
                    store_tag_nr,
                    free_ref_tag_nr,
                    tagset,
                    ids,
                    counter,
                );
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            tag_stores(
                v,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        Value::Span(b) => {
            tag_stores(
                &mut b.1,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        _ => {}
    }
}

/// Plan-57 last-use freeing, Phase 1 — definition-point liveness diagnostic.
///
/// Reports each **function-scoped store** whose *data* dies (last read or a
/// rebind-away) **before another store allocates** — so it is held dead to scope
/// exit while the watermark grows.  This is the I-b (sequential distinct) and
/// III-straight-line (sequential reassign) divergence block-confinement cannot
/// reach.  Returns the count.  Read-only; gated by `LOFT_LASTUSE_GUARD`.
///
/// Block-confined stores (already freed at block exit by I-a) are excluded by the
/// body-scope filter.  Genuinely live-to-the-end stores self-exclude — nothing
/// allocates after their last read.
fn last_use_guard(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    fn_name: &str,
) -> usize {
    let body_scope = match code.unspan() {
        Value::Block(bl) => bl.scope,
        _ => return 0,
    };
    let mut holds: HashMap<u16, u16> = HashMap::new();
    let mut alloc: HashMap<u16, u32> = HashMap::new();
    let mut dead: HashMap<u16, u32> = HashMap::new();
    let mut last_read: HashMap<u16, u32> = HashMap::new();
    let mut seq = 0u32;
    store_liveness_walk(
        code,
        &mut seq,
        db_nr,
        gf_nr,
        fr_nr,
        &mut holds,
        &mut alloc,
        &mut dead,
        &mut last_read,
    );
    // data-death point: rebind-away if any, else last read of the data.
    let dead_at = |st: u16| {
        dead.get(&st)
            .copied()
            .or_else(|| last_read.get(&st).copied())
    };
    let mut count = 0;
    let mut stores: Vec<u16> = alloc.keys().copied().collect();
    stores.sort_unstable();
    for &st in &stores {
        if vars.scope(st) != body_scope {
            continue; // block-confined — freed at block exit by I-a
        }
        let Some(d) = dead_at(st) else { continue };
        if let Some(&later) = stores
            .iter()
            .filter(|&&w| w != st && alloc.get(&w).is_some_and(|&a| a > d))
            .min_by_key(|&&w| alloc[&w])
        {
            eprintln!(
                "[lastuse-guard] {fn_name}: store '{}' data dead @{d} but held to scope exit \
                 while '{}' allocates @{} — should have been stopped",
                vars.name(st),
                vars.name(later),
                alloc[&later],
            );
            count += 1;
        }
    }
    count
}
