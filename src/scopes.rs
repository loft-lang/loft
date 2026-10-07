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
//!
//! ## Parts
//!
//! - [`places`] — the place a view or an argument names, and when two places are the same storage
//! - [`disturbance`] — what code grows, removes from or reshapes, directly or through a callee
//! - [`view_walk`] — which view bindings must be copied out because their container is disturbed
//! - [`reshape_refusals`] — the shapes refused because a callee reshapes what an argument points into
//! - [`handoff`] — which side of a copy keeps the drop, and the per-path hand-off flags
//! - [`handles`] — generator handles: owned, viewed, and retained while shared
//! - [`backings`] — the hidden store a collection's elements live in
//! - [`join_rewrite`] — joins written out per arm before the scan
//! - [`tuple_members`] — tuple members that own their element
//! - [`buffers`] — hidden return buffers minted once or lazily
//! - [`phase`] — one scan phase over one function
//! - [`witness`] — owner witnesses: release by store identity where ownership is mixed
//! - [`move_elide`] — borrow and last-use move elision after the scan
//! - [`construct_elide`] — move elision into a record under construction
//! - [`value_struct`] — value-struct views copied only where their backing may change
//! - [`capture_adoption`] — closure records that adopt a captured store
//! - [`capture_builds`] — what a closure build captures, and which frees an adoption takes over
//! - [`closure_keep`] — closure stores kept alive past a pass or a rebind
//! - [`drops`] — the drop hook at scope end and on reassignment
//! - [`scan`] — the walk: scopes, bindings, and dispatch per node
//! - [`scan_set`] — one assignment
//! - [`branches`] — an `if`: both arms and the slots they share
//! - [`convert`] — the releases and re-mints around each statement of a block
//! - [`free_vars`] — a scope exit and the value leaving with it
//! - [`get_free_vars`] — which variables a scope exit frees, and how
//! - [`insert_free`] — placing releases around a block's value
//! - [`returns`] — what a return hands out
//! - [`args`] — call arguments
//! - [`arm_lift`] — a binding per path for a value branch's arm tails
//! - [`inline_returns`] — whether an inline call result is fresh and owned
//! - [`text_return_check`] — debug check: a returned text is never freed
//! - [`leak_check`] — who releases each local, and the debug leak checks
//! - [`par_safety`] — par safety from purity
//! - [`par_writes`] — par workers writing parent state
//! - [`confinement`] — the block a store is confined to
//! - [`last_use`] — last-use freeing

use crate::data::{Context, Data, DefType, Value};
use crate::variables::{Function, compute_intervals, size};
// The Fx tables (`crate::fxhash`): the scope pass keys its sets by variable and definition
// number on every node, and SipHash was a measurable share of a compile (@PLN166 B4).
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::collections::BTreeMap;

mod args;
mod arm_lift;
mod backings;
mod branches;
mod buffer_detach;
mod buffers;
mod capture_adoption;
mod capture_builds;
mod closure_keep;
mod confinement;
mod construct_elide;
mod convert;
mod disturbance;
mod drops;
mod free_vars;
mod get_free_vars;
mod handles;
mod handoff;
mod inline_returns;
mod insert_free;
mod join_rewrite;
mod last_use;
mod leak_check;
mod move_elide;
mod par_safety;
mod par_writes;
mod phase;
mod places;
mod reshape_refusals;
mod returns;
mod scan;
mod scan_set;
mod text_replace;
mod text_return_check;
mod tuple_members;
mod value_struct;
mod view_walk;
mod witness;

pub(crate) use backings::tuple_projection_of;
use capture_adoption::mark_borrowed_captures;
pub(crate) use capture_builds::CaptureBuilds;
use closure_keep::ClosureKeep;
use confinement::{relocate_null_init, rest_store_oracle, store_confinement, store_lifetime_guard};
use disturbance::Disturbance;
pub use disturbance::{DisturbedParams, ViewCause, disturbed_params_map, places_disturbed_by};
pub(crate) use handles::handle_rhs_kinds;
use handles::retain_shared_handles;
pub(crate) use handoff::{
    appends_to_element, copy_hands_off, copy_moves_drop_from, drop_bearing_source,
};
use join_rewrite::{reassociate_coalesce_chains, rewrite_written_out, write_out_joined_copies};
use last_use::{
    drop_bearing_stores, last_use_guard, lastuse_reclaim, reclaim_free_intent,
    reclaim_unfreed_eligible, tag_stores,
};
pub(crate) use leak_check::FrameReleases;
use move_elide::{elide_borrows, move_elide};
pub use par_safety::{
    analyse_par_safety_fixpoint, is_par_safe, par_unsafe_reason, sub_rule_is_pure,
};
pub use par_writes::{worker_calls_parent_write, worker_calls_parent_write_deep};
pub(crate) use phase::{collect_fnref_captures, collect_fnref_targets, multi_assigned_in};
use phase::{hidden_return_buffer_var, run_scan_phase};
pub(crate) use places::value_view_places;
pub use places::{ParamPlace, call_arg_place, compose_param_place, same_place};
pub use reshape_refusals::{ReshapeRefusal, reshape_refusals};
pub(crate) use returns::return_has_null_arm;
use value_struct::value_struct_copy;
pub(crate) use view_walk::link_set_repoints;

/// What a bind of a call result is, as `scan_set` decides it — the four facts
/// [`Scopes::pair_call_buffers`] reads to choose which of the call's buffers to pair with the
/// destination and how.
#[derive(Clone, Copy)]
struct BindShape {
    /// Whose store the destination takes.
    adopts: Adopts,
    /// The destination is a `&` parameter the result is published through.
    publishes_through_ref: bool,
    /// The destination is a vector (aliased to the work-ref, never copied).
    vector_shaped: bool,
}

/// Whose store a call result's destination takes — the two adopting cases exclude each other
/// (`adopts_minted_at_bind` answers only for a callee that does NOT adopt fresh).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Adopts {
    /// The callee's return is dep-empty: it mints its own store rather than filling the one
    /// it was handed (`Definition::return_adopts_fresh_store`).
    Fresh,
    /// The callee returns the local it promoted onto its buffer and the destination adopts
    /// that store (`use_analysis::adopts_minted_at_bind`).
    Minted,
    /// Neither: the destination copies, or aliases a work-ref.
    Neither,
}

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
    /// Variables whose FIRST binding's right-hand side is being scanned.  They are registered
    /// before that scan, but hold nothing until it completes, so an exit inside it — the
    /// `return` of `x = e ?? return` — must not release them.
    binding_now: Vec<u16>,
    /// Variables that are redefined after running out-of-scope get copied with this mapping.
    var_mapping: HashMap<u16, u16>,
    /// Plan-57 cluster-I two-phase scan: confined `__vdb`/local var → the block
    /// scope it should register at instead of function scope, so the block-exit
    /// `free_vars` sweep frees the store there.  Empty on phase 1; populated from
    /// `store_confinement` for the gated phase-2 re-scan (`put_scope` consults it).
    confined: HashMap<u16, u16>,
    /// The scopes of the currently traversed loops.
    loops: Vec<u16>,
    /// Beside each entry of `loops`: the variables its body REFILLS on every pass
    /// ([`loop_body_refills`]).
    loop_refills: Vec<HashSet<u16>>,
    /// Every literal backing (`__vdb_N`) each vector local is bound to anywhere in the function
    /// ([`vector_literal_backings`]), keyed by the pre-scan id.
    vector_backings: HashMap<u16, Vec<u16>>,
    /// Every call buffer (`__ref_N`) each vector local is bound through anywhere in the function
    /// ([`vector_call_buffers`]), keyed by the pre-scan id.
    vector_call_buffers: HashMap<u16, Vec<u16>>,
    /// The vector locals promoted onto the return buffer that a statement has already filled
    /// ([`Scopes::promoted_vector_refill`]).
    promoted_filled: HashSet<u16>,
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
    /// @PLN164 B1 — the `__ref_N` buffers paired through `use_analysis::adopts_minted_at_bind`:
    /// the callee returns the local it promoted onto its buffer, the caller's local adopts the
    /// store the callee MINTED, and the pairing exists for the identity-guarded free alone.
    /// `reuse_record_buffers` must not pre-mint these — a buffer handed to such a callee
    /// non-null is written by its literal and then FREED by the interpreter's rebind of the
    /// promoted local from a call (`143-plan51-cluster3-mixed-lit-call`'s shape: the free
    /// native guards with its `_rb_w_` witness fires unguarded on the interpreter), so the
    /// caller's next turn reads a recycled store.  Reuse for this callee shape is @PLN164 B1b.
    minted_pairs: HashSet<u16>,
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
    /// @PLN157 § V-g — the record locals this function only ever READS, off the raw body
    /// (`use_analysis::read_only_record_locals`); one input of `view_elision_bind`.
    read_only_locals: Vec<bool>,
    /// Variables assigned ANYWHERE in this function ([`assigned_in`]).  A parameter in this
    /// set is rebound after entry, so it cannot stand witness for a store it named at a
    /// bind: `view_elision_bind` declines it, and the collection join takes its snapshot.
    assigned: HashSet<u16>,
    /// Variables assigned at MORE THAN ONE site in this function.  The identity route
    /// (`lift_join_witness`) compares a local's store against the variable its dep names at
    /// scope exit; a base reassigned while the local is live could by then name a store that
    /// is already gone, so such a base is not offered as a witness.  Conservative in the safe
    /// direction: declining keeps today's leak, never frees a store twice.
    multi_assigned: HashSet<u16>,
    /// The `??` hoists that hold a whole LOCAL rather than a projection ([`whole_value_hoists_in`]).
    whole_value_hoists: HashSet<u16>,
    /// Variables every bind of which but ONE writes `null` ([`null_led_in`]): the author's
    /// `x: T? = null; if c { x = mk() }`, which `(B-Scope)` makes the spelling of a local an
    /// arm assigns and a later statement reads.
    null_led: HashSet<u16>,
    /// The locals whose one real bind follows an enclosing null on every path and every pass
    /// ([`null_led_first_binds_in`]): that bind displaces no store.
    null_led_first: HashSet<u16>,
    /// How many nodes of the body name each variable ([`var_mentions_in`]) — what tells
    /// `scan_if` a local whose every mention lies inside one arm.
    mentions: HashMap<u16, usize>,
    /// The variables a value-branch bind was written out into the arms for
    /// (`rewrite_written_out`): declared by the statement around the `if`, so never an arm's
    /// local however its mentions fall.
    sunk: HashSet<u16>,
    /// The backing local each CAPTURE named at the closure build — see
    /// [`capture_build_backings`].  Computed once off the raw body, because the answer is
    /// positional (@FR-O-Latest) and the variable table carries only the LAST assignment.
    capture_build_backing: CaptureBuilds,
    /// The closure records and fn-ref locals whose releases are decided by store identity —
    /// see [`closure_keep_set`] (`@FR-L-CapKeep`).
    closure_keep: ClosureKeep,
    /// The fn-ref local the statement being scanned binds to a closure build, if any.
    keep_build_target: Option<u16>,
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
    /// @PLN164 B1b / `@FR-O-Buffer` — `(promoted buffer var, its ENTRY WITNESS)`: the store
    /// the caller handed, snapshotted before the body runs (`Function::entry_witness`).  A
    /// promoted buffer holds the caller's store or one this frame minted, and only the run
    /// can say which; every free of the buffer this pass emits is guarded by it.
    entry_witness: Option<(u16, u16)>,
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
    /// `@FR-L-CapRebind` — the closure records the statement being scanned releases in its
    /// prefix (the record a rebuild is about to overwrite, loft#1388).  The rebuild's own
    /// snapshot (`displaced_drop`) skips them: one release per displaced record.
    prefix_released: Vec<u16>,
    /// @PLN85 `local_source` over-free fix (gated by `LOFT_JOIN_OWN`): heap slots
    /// that hold an OWNED store displaced by a later `Borrowed`/`Join` reassignment
    /// (`use_analysis::displaced_owned_slots`). For these, `scan_set` strips the
    /// declared deps so the OWNED path deep-copies + frees the slot — otherwise the
    /// displaced owned store is orphaned and leaks. Empty when the flag is off.
    displaced_owned: HashSet<u16>,
    /// @PLN130 F2/F8 — view bindings live across a disturbance of their container, and which
    /// disturbance it was.  See [`collect_views_to_materialise`].
    views_to_materialise: HashMap<u16, Disturbance>,
    /// The `text` payload views whose mirrors were dropped and already reported, so a binding
    /// written several times says so once (loft#1665).
    text_views_reported: HashSet<u16>,
    /// loft#721 — fn-ref variable -> the definition it was assigned, or
    /// `u32::MAX` when more than one definition reaches it.  A `CallRef`'s callee
    /// is a runtime value, so this local fact is what lets the lift ask the
    /// callee's own `returns_borrowed_view()` instead of guessing from the type.
    fnref_target: HashMap<u16, u32>,
    /// loft#849 / @PLN139 — vars that no longer OWN what they hold, so their scope end must
    /// not drop it.  See [`collect_drop_transferred`].
    drop_transferred: HashSet<u16>,
    /// loft#1511 / `formal/heap.md (H-Drop)` — a TUPLE local's elements minted by the
    /// element's own initializing CALL: tuple var → (element index → the call's hidden
    /// buffer var, or `None` for a bufferless mint such as a nullable return).  A tuple has
    /// no dep list of its own, so the element's free site cannot see that the record it
    /// releases is a frame-minted droppable — this map carries that fact from the `Set` that
    /// established it to [`tuple_owned_elem_frees`], which then runs the type's cascade
    /// before the free and disarms the buffer's own scope-end claim.  Path-sensitive like
    /// [`Self::owned_refs`]: intersect-merged at every join, so a pairing that holds on one
    /// path only is dropped (losing the hook, never doubling it).
    tuple_call_mint: HashMap<u16, HashMap<u16, Option<u16>>>,
    /// loft#1645 — the per-path flag of a tuple MEMBER a whole-tuple move written in an `if` arm
    /// moves (`if c { u = t }`): `false` at function entry, set where the move runs, read by
    /// the source's release (which then skips the hook the move took) and reset by a rebind or
    /// a member write, after which the tuple holds a value of its own again.
    tuple_moved: HashMap<(u16, u16), u16>,
    /// The loop depth a tuple local was first bound at: a move of it under a deeper loop moves
    /// it once per pass, which `(H-Spent)` refuses, and takes no per-path flag.
    tuple_depth: HashMap<u16, usize>,
    /// loft#1588 — the backing each VECTOR member of a tuple variable's CURRENT value lives in,
    /// read off the literal that assigned it: the pairing a whole-tuple move hands the release
    /// of.  The variable's type cannot answer it, because it carries only the LATEST
    /// assignment's deps.  Recorded for an assignment in the variable's own scope and dropped by
    /// one in a nested scope, and intersect-merged at every join like
    /// [`Self::tuple_call_mint`], so a move that cannot know which backing it copies declines.
    tuple_member_now: HashMap<u16, HashMap<u16, u16>>,
    /// Per open block, the fn-ref locals a statement of that block has already bound: a
    /// `Set` of one of them is a REBIND, which displaces a value (`fnref_call_rebind`).  A
    /// local declared ahead of a branch is first bound in an arm, where its slot holds no
    /// value yet — on the interpreter not even a null one.
    fnref_bound: Vec<HashSet<u16>>,
    /// loft#1532 — the `(tuple, element)` pairs a member ASSIGNMENT (`t.1 = …`) writes in this
    /// function, by the variable numbers of the unscanned body.  Such an element is handed to
    /// the tuple ALONE at the literal that builds it: the construction or call that delivered it
    /// is disarmed right after that `Set`, and its pairing becomes `None` (a sole owner, as for a
    /// bufferless mint).  Every member write can then release the record it displaces with no
    /// other claimant left, on every path, and both arms of a join agree on the pairing.
    written_tuple_members: HashSet<(u16, u16)>,
    /// D-heap-3 (loft#1506) — a local BOUND to a projection of another local, and the
    /// `(offset, depth)` path from that local's record to the member it views.
    ///
    /// `(B-View)` makes `e = d.h` a view, and a `return e` publishes an owned COPY of it —
    /// so `(H-Drop)`'s responsibility clause moves the member's release to that copy, and
    /// `d`'s cascade must leave it alone.  The copy names the VIEW, not the projection, so
    /// the path has to be carried from the bind that established it to the return that
    /// hands it out.  Path-sensitive like [`Self::owned_refs`]: intersect-merged at every
    /// join, and retired when either end is reassigned (@FR-O-Latest — the fact belongs to
    /// the latest assignment, and a view whose BASE was rebuilt names a different record).
    view_backing: HashMap<u16, (u16, (u16, u16))>,
    /// loft#1510 / `formal/heap-history.md` D-heap-4 — a local whose LATEST assignment delivered a
    /// CONSTRUCTION's work-ref record (`construction_work_ref`), mapped to that work-ref.
    /// Read by the owned→view transition free: releasing the store by identity there must
    /// run the type's cascade first and then disarm the work-ref, whose own scope-end drop
    /// covers the record on the paths where no transition fires.  Same intersect-merge
    /// discipline as [`Self::owned_refs`]; an entry that does not survive a join falls back
    /// to the bare free (losing the hook, never doubling it).
    construction_backing: HashMap<u16, u16>,
    /// loft#1607 / `formal/heap-history.md` D-heap-38 — a VECTOR local's latest bind on THIS path, mapped
    /// to the backing that bind filled (`w = OpGetField(__vdb_N, …)`).  The variable's type names
    /// one backing, the LAST bind's (`@FR-O-Latest`), which is the wrong one for an arm that bound
    /// an earlier one; the scope-end release reads this first.  Same per-arm save and intersect
    /// merge as [`Self::construction_backing`].
    bind_backing: HashMap<u16, u16>,
    /// loft#1623 — the frame-owned TEMPS a joined binding borrows, keyed by that binding.
    ///
    /// A binding whose value branch declined the per-arm write-out borrows a temp per arm and
    /// releases nothing itself, so at a RETURN of it `(H-Move)`'s *"the function's own
    /// variables end with it"* is about those temps and not about the binding.  The set cannot
    /// be read back off the binding's deps: an arm the PARSER owns hands back a
    /// `join-arm-owner` `__ref_N` that the dep list never names, and it hooks all the same.
    /// Recorded where the temps are made, and read at the return.
    ///
    /// A container the frame keeps — `s` in `x = s.h ?? b` on the path that chose `s.h` — is
    /// NOT in here, and must not be: the caller got a copy of a member and `s` still owes its
    /// own release.  That is the cell this map exists to keep apart from the rest.
    join_holders: HashMap<u16, Vec<u16>>,
    /// loft#1628 — the plain locals a WITNESSED local was bound to as they are (a value
    /// branch's arm `b` in `x = s.h ?? b`), keyed by that local.  Such a local may hold one of
    /// their records at a return, and then that record is what the return hands out.
    witness_aliases: HashMap<u16, Vec<u16>>,
    /// The `__lift_N` temps an arm lift built — the destinations whose hand-off is PER PATH.
    ///
    /// The fact belongs to the CONSTRUCTION and cannot be read back off the IR: by the time
    /// the statement scan meets `__lift_1 = a` again it is scanning INSIDE the arm, so the
    /// branch that makes the copy conditional is no longer in view. Recorded where the temps
    /// are made and read by [`handoff_target`], so both deciders answer from the same fact.
    arm_lift_temps: HashSet<u16>,
    /// loft#1515 — a SOURCE whose release is handed off inside a branch ARM to a destination
    /// that is not per-arm, mapped to the boolean recording whether that hand-off actually
    /// RAN.
    ///
    /// `(H-Drop)` moves a release with a copy and `(O-Complete)` makes that fact per path.
    /// Where the arms lift into a temp of their own the fact needs no value — the temp is null
    /// on the paths that did not take it, so stopping the temp is enough ([`Self::
    /// arm_lift_temps`]). Where the arms assign one SHARED local it does: on the path that ran
    /// the destination owns what it took and must keep its release, on the path that did not
    /// the source still owes one, and no static answer covers both. So the path fact is
    /// MATERIALISED, exactly as loft#1200's [`Self::local_owns`] materialises sole ownership
    /// for the free it guards.
    ///
    /// Read at the source's scope-end release, set at the hand-off, and cleared when the
    /// SOURCE is reassigned — `@FR-O-Latest`: what it holds from there on is its own again.
    handed_off: HashMap<u16, u16>,
    /// The `(destination, source)` pairs [`Self::handed_off`] was minted for — the copies
    /// whose hand-off is per path. Keyed by the PAIR because one source may be handed to
    /// different destinations on different arms, and a bare source would then set the flag at
    /// a copy that is not the one it stands for.
    per_path_pairs: HashSet<(u16, u16)>,
    /// loft#1585 — the generator locals that VIEW a member's handle at one assignment
    /// (`h = t.g`) and hold a frame of their own at another (`h = steps()`).  The parser marks
    /// a view never-free, and here that is lifted: the local's [`Self::handed_off`] flag says,
    /// per path, whether what it holds now is a view, and its releases read the flag.
    handle_views: HashSet<u16>,
    /// loft#1585 — the generator-handle members `(tuple local, index)` the tuple OWNS: every
    /// assignment of the member gives it a fresh handle — a call, a generator's advance, a
    /// tuple a call returned.  One read out of a member (`OpGetDbRef`) is a VIEW of the frame
    /// the member's container holds, and so is a local placed there, which keeps its own.
    owned_handle_members: HashSet<(u16, u16)>,
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
    /// The reassignments this scan writes out per arm, as (the address of the `Set`'s value node
    /// inside the code the scan was handed, the variable).  `run_scan_phase` returns them, and its
    /// caller rewrites exactly those before scanning again (`rewrite_written_out`).
    written_out: Vec<(usize, u16)>,
}

/// Scope / lifetime analysis pass over every function definition.
///
/// # Panics
/// Under the `LASTUSE_RECLAIM` gate only (a Plan-57 testing build), panics if the
/// reclaim pass left a store the model says is dead un-freed past a later
/// allocation (the Phase-4 Goal-E watermark guard).  Never panics in normal builds.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn check(data: &mut Data, database: &mut crate::database::Stores) {
    // `@FR-R-PureReuse` — on the parser's bodies, before any of this pass's own rewrites.
    crate::pure_reuse::rewrite_program(data);
    // `(H-Spent)` — the reads of a moved name, found while every body is still the parser's:
    // the scan below adds reads of its own (releases, hooks, snapshots) that are not the author's.
    crate::spent::record_all(data);
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
    if crate::env_once!(std::env::var_os("LOFT_NO_BORROW_ELIDE").is_none()) {
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
    let tag_mode = crate::env_once!(std::env::var("LOFT_STORE_TAG").is_ok());
    let mut tag_counter = 1u16;
    // Plan-57 Phase 5: last-use freeing (reclaim) is ON by default.  `LASTUSE_RECLAIM_OFF`
    // disables it for A/B watermark measurement.  The Goal-E enforcement assert runs in
    // debug builds always, and in release on demand via `LOFT_STORE_GUARD`.
    let reclaim_off = std::env::var("LASTUSE_RECLAIM_OFF").is_ok();
    let reclaim_guard =
        cfg!(debug_assertions) || crate::env_once!(std::env::var("LOFT_STORE_GUARD").is_ok());
    // Positive-control fault injection (test-only, never set in production): skip the
    // early-free insertion below while STILL running the Phase-4 guard, so a program
    // with reclaim-eligible stores trips the assertion.  This makes the Goal-E guard
    // *falsifiable* — proving it fires on a real reclaim regression, so its silence on
    // the corpus is evidence.  Differs from `LASTUSE_RECLAIM_OFF` (which also disables
    // the guard); correctness is preserved either way by the scope-exit `OpFreeRef`.
    let inject_unfreed =
        reclaim_guard && crate::env_once!(std::env::var("LOFT_STORE_GUARD_INJECT").is_ok());
    // @PLN164 C3 — the callee half of `(B-Disturb)`, built ONCE over the whole world rather
    // than re-derived at each call site: the question is asked once per CALL, and a callee body
    // would otherwise be re-walked once per call to it.  Built BEFORE the loop, so every
    // definition is read in the same pre-scope form the inline producers read `orig_code` in.
    let disturbed =
        crate::keys::callee_disturb_enabled().then(|| disturbed_params_map(data, Some(database)));
    let disturbed = disturbed.as_ref();
    // The functions this call checks: `text_replace::admit` decides only theirs.
    let fresh: Vec<u32> = (0..data.definitions())
        .filter(|&d| {
            matches!(data.def(d).def_type, DefType::Function) && !data.def(d).variables.done
        })
        .collect();
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) || data.def(d_nr).variables.done {
            continue;
        }
        let _census_body = crate::rewrite_census::InBody::enter("ir", data.def(d_nr).name());
        let free_ref_nr = data.def_nr("OpFreeRef");
        let mut orig_code = data.definitions[d_nr as usize].code.clone();
        let mut orig_vars = Function::copy(&data.def(d_nr).variables);
        // A join copied into a container is written out per arm before any analysis reads it,
        // so each arm's copy hands its own source over (`formal/heap-history.md` D-heap-15).
        write_out_joined_copies(&mut orig_code, &orig_vars, data);
        reassociate_coalesce_chains(&mut orig_code, &mut orig_vars, data);
        retain_shared_handles(d_nr, &mut orig_code, &orig_vars, data, database);
        // Phase 1: the normal scan → apply → set-scope pass.
        let written_out = run_scan_phase(
            data,
            database,
            d_nr,
            &orig_code,
            &orig_vars,
            &HashMap::default(),
            disturbed,
            &HashSet::default(),
        );
        let sunk: HashSet<u16> = written_out.iter().map(|&(_, v)| v).collect();
        // A reassignment the scan wrote out per arm was seen in its VALUE form by every analysis
        // that ran before the scan.  Rewrite exactly those and scan again, so those analyses read
        // the per-arm form; the confinement rescan below starts from the rewritten pair too.
        if !written_out.is_empty()
            && rewrite_written_out(&mut orig_code, &mut orig_vars, data, &written_out)
        {
            run_scan_phase(
                data,
                database,
                d_nr,
                &orig_code,
                &orig_vars,
                &HashMap::default(),
                disturbed,
                &sunk,
            );
        }
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
        if crate::env_once!(std::env::var("LOFT_REST_ORACLE").is_ok()) {
            rest_store_oracle(
                &data.definitions[d_nr as usize].code,
                &data.definitions[d_nr as usize].variables,
                free_ref_nr,
                data.def_nr("OpDatabase"),
                data.def(d_nr).name(),
            );
        }
        if !confined.is_empty() {
            let mut cmap: HashMap<u16, u16> = HashMap::default();
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
            run_scan_phase(
                data, database, d_nr, &orig_code, &orig_vars, &cmap, disturbed, &sunk,
            );
            for (&vdb, &(_local, b)) in &confined {
                relocate_null_init(&mut data.definitions[d_nr as usize].code, vdb, b);
            }
        }
        // Plan-57 last-use freeing, Phase 3 (DEFAULT since Phase 5): null-init
        // relocation + early free — the combination that lowers the body-0-locked
        // watermark.  Runs before compute_intervals so the moved first_def is
        // reflected.  `LASTUSE_RECLAIM_OFF` disables it for A/B measurement.
        // The stores reclaim must not move, for every pass below that reads the reclaim plan.
        let drop_bearing = drop_bearing_stores(data, d_nr);
        if !reclaim_off {
            let db_nr = data.def_nr("OpDatabase");
            let gf_nr = data.def_nr("OpGetField");
            if !inject_unfreed {
                let d = &mut data.definitions[d_nr as usize];
                lastuse_reclaim(
                    &mut d.code,
                    &d.variables,
                    db_nr,
                    gf_nr,
                    free_ref_nr,
                    &drop_bearing,
                );
            }
            // Plan-57 Phase 4 — Goal-E enforcement (THE watermark guard, supersedes
            // the scope-exit `store_lifetime_guard`).  Every store the model says is
            // dead and reclaim claimed (its `intent`) must now be freed before its
            // sibling allocates; a non-zero count is a reclaim regression — the rule
            // silently re-acquired an exception.  On in debug; release on demand via
            // LOFT_STORE_GUARD (`reclaim_guard`); zero-cost otherwise.
            if reclaim_guard {
                let d = &data.definitions[d_nr as usize];
                let unfreed = reclaim_unfreed_eligible(
                    &d.code,
                    &d.variables,
                    db_nr,
                    gf_nr,
                    free_ref_nr,
                    &drop_bearing,
                );
                assert_eq!(
                    unfreed, 0,
                    "plan-57 Phase 4: {} left {unfreed} reclaim-eligible store(s) live-but-dead past a later alloc",
                    d.name,
                );
            }
        }
        // @PLN164 B2 (`@FR-R-Place`, `@FR-R-MoveLast`) — a call result built where it will
        // live: decided here, after the scan has settled the function's frees and before the
        // slot intervals are computed off the final IR.  One home for both backends.
        crate::place_result::rewrite(data, d_nr);
        // `(R-ExitVector)` — a local vector returned inside the exit literal is built in the
        // return buffer's store: decided on the same IR, after the placements it may host.
        crate::exit_vector::rewrite(data, d_nr);
        // `(R-ExitVector)`'s record clause — a local record returned inside the exit literal
        // is built in its field of the return buffer.
        crate::exit_record::rewrite(data, database, d_nr);
        // `(R-ForwardResult)` — a returned local bound from a call is built in the return
        // buffer: the call is handed the buffer the delivery copied into.
        crate::forward_result::rewrite(data, d_nr);
        // `(R-LiteralAppend)` — a record literal bound to a local whose one use is the append
        // of it is built in its element.
        crate::literal_append::rewrite(data, d_nr);
        // `(R-ReturnField)` — the returned field of an owned local is the local's store handed
        // over at the field's position: decided on the same settled IR, after the exit vector
        // a decoder's tree may have been built in.
        crate::return_field::rewrite(data, database, d_nr);
        // `@FR-R-Rebind` — `x = f(x, …)` hands the callee x's own record as its buffer:
        // decided here for the same reason, on the same settled IR.
        crate::rebind_place::rewrite(data, database, d_nr);
        // `@FR-R-Const` — a call of a literal-bodied function whose result is only read
        // answers a view of the pre-built constant: decided on the same settled IR.
        crate::const_fn::rewrite(data, database, d_nr);
        // `@FR-R-CopyView` — a read-only copy of a record nothing can disturb is a view of
        // it: decided on the same settled IR, after R-Const has turned its calls into views.
        crate::copy_view::rewrite(data, d_nr);
        // `@FR-R-Compact` — a vector rebuilt from a contiguous run of its own elements is
        // compacted in place behind an in-range guard: decided on the same settled IR.
        crate::compact::rewrite(data, d_nr);
        // A reduction loop a loop kernel covers is one call of it (@PLN180 § Kernels): decided
        // on the same settled IR, so both backends run the kernel and neither the loop.
        crate::loop_kernels::rewrite(data, d_nr);
        // `@FR-R-ByteCopy` — a text copied into a byte vector one byte at a time is one
        // append behind an in-range guard: decided on the same settled IR.
        crate::byte_copy::rewrite(data, database, d_nr);
        // `@FR-R-RepeatRun` — equal constant pushes into one target are one repeat fill:
        // decided on the same settled IR.
        crate::repeat_run::rewrite(data, database, d_nr);
        // `@FR-R-VecCopy` — a vector copied into an exclusive one element at a time is one
        // append: decided on the same settled IR.
        crate::vec_copy::rewrite(data, database, d_nr);
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
            let (owning, _intent) = reclaim_free_intent(
                &d.code,
                &d.variables,
                db_nr,
                gf_nr,
                free_ref_nr,
                &drop_bearing,
            );
            let tagset: HashSet<u16> = owning.into_iter().collect();
            let mut ids: HashMap<u16, u16> = HashMap::default();
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
        // Plan-57 cluster I: store-lifetime guard (diagnostic, gated).
        if crate::env_once!(std::env::var("LOFT_STORE_GUARD").is_ok()) {
            let gf_nr = data.def_nr("OpGetField");
            let d = &data.definitions[d_nr as usize];
            store_lifetime_guard(&d.code, &d.variables, free_ref_nr, gf_nr, &d.name);
        }
        // Compute live intervals so validate_slots can check for slot conflicts after codegen.
        compute_function_intervals(data, d_nr);
        // Plan-57 last-use freeing, Phase 1: definition-point liveness diagnostic
        // (read-only).  Reports each function-scoped owning store held past its
        // last use while later allocations run — the I-b / III-straight-line
        // watermark divergence.  See fix-design-last-use-freeing.md.
        if crate::env_once!(std::env::var("LOFT_LASTUSE_GUARD").is_ok()) {
            let db_nr = data.def_nr("OpDatabase");
            let gf_nr = data.def_nr("OpGetField");
            let d = &data.definitions[d_nr as usize];
            last_use_guard(&d.code, &d.variables, db_nr, gf_nr, free_ref_nr, &d.name);
        }
        assign_function_slots(data, d_nr);
    }
    // @PLN94 C.0 (DEV tier) — the POST-codegen free-based checks (over-free / under-free), now that
    // `get_free_vars` has inserted the frees into `def.code` above. Self-gates on
    // `LOFT_OWN_ORACLE=check-dev`; observer only (SI-1), a no-op on the default `check` path.
    // `@FR-O-Owner` — a literal buffer whose store a local owner takes over is detached, now
    // that the deps say who owns and the frees say who releases.
    if crate::keys::buffer_detach_enabled() {
        buffer_detach::detach_owned_buffers(data);
    }
    crate::ownership_cfg::oracle_free_checks(data);
    // #682 — record which closure captures the record ADOPTS, now that every dep
    // rewrite above has settled.  Must run after the loop, not inside it: the
    // verdict is the same `owns` fact `get_free_vars` uses, and that fact is only
    // final once the call-result rewrites (`make_independent`) have run.
    mark_borrowed_captures(data, database);
    // `(H-Copy-Lease)` — every copy of a type that declares `OpCopy` runs it on the new structure.
    crate::use_analysis::lease_calls(data);
    // `@FR-H-TextReplace` — an assignment to a text field releases the text it replaces only
    // where no borrow of that text can be live; every other site writes without releasing.
    text_replace::admit(data, &fresh);
    // `LOFT_VAR_TABLE=<fn substring>` — the variable table beside the IR dump, with
    // each type dep resolved to `name(index)`.  Observer only; a no-op when unset.
    crate::variables::dump_var_tables(data, 0);
}

/// The live intervals of `d_nr`'s variables, computed off its FINAL body — the input
/// [`assign_function_slots`] reads.  Split out of [`check`] so a whole-program rewrite that
/// changes a settled body afterwards (`value_record::rewrite_program`) can lay the frame out
/// again by the same two steps.
pub(crate) fn compute_function_intervals(data: &mut Data, d_nr: u32) {
    let free_ref_nr = data.def_nr("OpFreeRef");
    let free_text_nr = data.def_nr("OpFreeText");
    let create_stack_nr = data.def_nr("OpCreateStack");
    let mut seq = 0u32;
    // The body is read and the variables written: two fields of one definition, so
    // neither needs a copy of the other.
    let def = &mut data.definitions[d_nr as usize];
    compute_intervals(
        &def.code,
        &mut def.variables,
        free_text_nr,
        free_ref_nr,
        create_stack_nr,
        &mut seq,
        0,
    );
    // `@FR-B-Ref-Lvalue` — a `&` link names a PLACE, so its target's slot may not be
    // handed to another local while the link is live.  Runs right after the intervals
    // are computed and before `assign_slots` reads them.
    data.definitions[d_nr as usize]
        .variables
        .extend_links_to_their_targets();
    // `@FR-O-Buffer` — the interpreter reads a promoted buffer's entry witness at every
    // rebind of the buffer, outside the IR, so the witness lives as long as the buffer: a
    // slot handed on after the snapshot's own initialisation would answer another
    // variable's store.
    let witnessed = {
        let vars = &data.definitions[d_nr as usize].variables;
        hidden_return_buffer_var(d_nr, vars, data)
            .and_then(|buf| vars.entry_witness(buf).map(|w| (buf, w)))
    };
    if let Some((buf, w)) = witnessed {
        data.definitions[d_nr as usize]
            .variables
            .extend_last_use_to(w, buf);
    }
}

/// Stack slots for `d_nr`'s variables, from the intervals [`compute_function_intervals`]
/// left.
pub(crate) fn assign_function_slots(data: &mut Data, d_nr: u32) {
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
        let aliases = crate::slot_alias::range_slot_aliases(data, d_nr);
        let d = &data.definitions[d_nr as usize];
        crate::variables::assign_slots_v2(&d.variables, local_start, &aliases)
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

fn call(to: &'static str, v: u16, data: &Data) -> Value {
    Value::Call(data.def_nr(to), vec![Value::Var(v)])
}
