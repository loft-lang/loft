// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! One scan PHASE over one function: gather the facts the scan reads, scan the body, install the
//! result and run the debug checks.  `check` runs it once, or twice when a store is confined to a
//! block.

use super::Scopes;
use super::arm_lift::is_discharge_hoist;
use super::backings::{vector_call_buffers, vector_literal_backings};
use super::buffers::{body_block_mut, lazy_buffer_mints, reuse_record_buffers};
use super::capture_builds::capture_build_backings;
use super::closure_keep::closure_keep_set;
use super::disturbance::DisturbedParams;
use super::handles::{mixed_handle_views, owned_handle_members};
use super::handoff::{
    arm_container_handoffs, assigns_a_self_stopping_copy, caller_record_locals,
    collection_handoffs, local_copy_pairs, loop_self_stopping_copies, per_path_handoffs,
    per_path_stops,
};
#[cfg(debug_assertions)]
use super::leak_check::{check_arg_ref_allocs, check_ref_leaks};
#[cfg(debug_assertions)]
use super::text_return_check::check_text_return;
use super::tuple_members::written_tuple_members_in;
use super::view_walk::collect_views_to_materialise;
use super::witness::{
    displaces_return_buffer, mixed_ownership_locals, nullable_locals_that_displace,
    nullable_view_locals, owner_witness_locals,
};
use crate::data::{Data, Deps, Type, Value, v_set};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;
use std::collections::BTreeMap;

/// Which DEFINITION each fn-ref variable in `code` was assigned, `u32::MAX` where the
/// assignments disagree or the target cannot be named.
///
/// `pub(crate)` because two readers need the same answer and a second spelling of it could
/// only agree by accident: the scope pass decides here whether a `CallRef` result may be
/// lifted, and the ownership oracle needs the same target to resolve that call through the
/// callee's return summary (`@FR-O-Oracle`).
pub(crate) fn collect_fnref_targets(code: &Value, function: &Function) -> HashMap<u16, u32> {
    let mut out: HashMap<u16, u32> = HashMap::default();
    code.walk(&mut |v| {
        let Value::Set(var, rhs) = v else { return };
        if !matches!(function.tp(*var).base(), Type::Function(..)) {
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
    let mut out: HashMap<u16, Vec<(i32, u16)>> = HashMap::default();
    code.walk(&mut |v| {
        let Value::Set(var, rhs) = v else { return };
        if !matches!(function.tp(*var).base(), Type::Function(..)) {
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

#[allow(clippy::too_many_arguments)]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn run_scan_phase(
    data: &mut Data,
    database: &mut crate::database::Stores,
    d_nr: u32,
    orig_code: &Value,
    orig_vars: &Function,
    confined: &HashMap<u16, u16>,
    disturbed: Option<&DisturbedParams>,
    sunk: &HashSet<u16>,
) -> Vec<(usize, u16)> {
    // @PLN85 `local_source` over-free fix (gated): the heap slots whose OWNED store
    // is displaced by a later borrow/join reassignment. Computed on the pre-scope
    // code so the dep classification is read before any dep-strip below mutates it.
    let displaced_owned = if crate::keys::join_own_enabled() {
        crate::use_analysis::displaced_owned_slots(orig_code, orig_vars, data)
    } else {
        HashSet::default()
    };
    // Computed BEFORE the struct takes its `&mut` on the store: the walk reads the store to
    // convert a field NUMBER into the byte OFFSET a view carries, and the two borrows cannot
    // overlap inside one initialiser.
    let views_to_materialise =
        collect_views_to_materialise(orig_code, orig_vars, data, database, disturbed);
    let mut scopes = Scopes {
        database,
        d_nr,
        max_scope: 1,
        scope: 0,
        stack: Vec::new(),
        var_scope: BTreeMap::new(),
        var_order: Vec::new(),
        binding_now: Vec::new(),
        var_mapping: HashMap::default(),
        confined: confined.clone(),
        loops: vec![],
        loop_refills: vec![],
        vector_backings: vector_literal_backings(orig_code, orig_vars, data),
        vector_call_buffers: vector_call_buffers(orig_code, orig_vars),
        promoted_filled: HashSet::default(),
        scan_depth: 0,
        lift_counter: 0,
        lift_vars: Vec::new(),
        lift_texts: Vec::new(),
        ret_temp_counter: 0,
        paired_witness: HashMap::default(),
        minted_pairs: HashSet::default(),
        literal_buffer: HashMap::default(),
        lift_join_witness: HashMap::default(),
        pending_join_witness: std::cell::Cell::new(u16::MAX),
        multi_assigned: multi_assigned_in(orig_code),
        whole_value_hoists: whole_value_hoists_in(orig_code, orig_vars, data),
        null_led: null_led_in(orig_code, data),
        null_led_first: null_led_first_binds_in(orig_code, data),
        mentions: var_mentions_in(orig_code),
        sunk: sunk.clone(),
        assigned: assigned_in(orig_code),
        read_only_locals: crate::use_analysis::read_only_record_locals(
            orig_code,
            orig_vars.var_count(),
            data,
        ),
        capture_build_backing: capture_build_backings(data, orig_vars, orig_code),
        closure_keep: closure_keep_set(data, orig_vars, orig_code),
        keep_build_target: None,
        lift_decl_depth: HashMap::default(),
        callref_join_bases: callref_join_bases_in(orig_code, data, d_nr),
        snapshot_witness: HashMap::default(),
        witness_buffer: HashMap::default(),
        owned_refs: HashMap::default(),
        rbuf_witness: None,
        entry_witness: None,
        local_owns: HashMap::default(),
        owner_witness: HashMap::default(),
        prefix_released: Vec::new(),
        displaced_owned,
        views_to_materialise,
        text_views_reported: HashSet::default(),
        fnref_target: collect_fnref_targets(orig_code, orig_vars),
        // Empty, and filled in SCAN ORDER (`Scopes::convert` arms each statement's hand-offs
        // after that statement is scanned).  A hand-off belongs to the assignment it follows
        // (`@FR-O-Latest`): seeded for the whole body, `b = mk(1); b = mk(2); y = b` read
        // `b`'s later hand-off at the earlier reassignment and never released `mk(1)`.  A loop
        // body is the exception — see the `Value::Loop` arm of `scan_inner`.
        drop_transferred: HashSet::default(),
        tuple_call_mint: HashMap::default(),
        tuple_moved: HashMap::default(),
        tuple_depth: HashMap::default(),
        tuple_member_now: HashMap::default(),
        fnref_bound: Vec::new(),
        written_tuple_members: written_tuple_members_in(orig_code),
        view_backing: HashMap::default(),
        construction_backing: HashMap::default(),
        bind_backing: HashMap::default(),
        join_holders: HashMap::default(),
        witness_aliases: HashMap::default(),
        arm_lift_temps: HashSet::default(),
        handed_off: HashMap::default(),
        per_path_pairs: HashSet::default(),
        handle_views: HashSet::default(),
        owned_handle_members: HashSet::default(),
        free_transferred: HashSet::default(),
        fn_defs: None,
        written_out: Vec::new(),
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
    // @PLN164 B1b / `@FR-O-Buffer` — the promoted buffer's ENTRY WITNESS.  Once the caller's
    // record-buffer pool hands such a callee a live store, "a promoted buffer is a local this
    // function mints" stops being true on every run, and the snapshot is what tells the two
    // apart.  Minted for every promoted record buffer, because every such body can free it
    // (the exit legs of `free_vars` and the interpreter's rebind).
    if crate::keys::adopt_buffer_reuse_enabled()
        && let Some(buf) = hidden_return_buffer_var(d_nr, &function, data)
        && function.name(buf) != "__retbuf"
        && let Some(record) = function.tp(buf).heap_def_nr()
    {
        let name = Function::entry_witness_name(function.name(buf));
        let w = function.add_temp_var(&name, &Type::Reference(record, Deps::none()));
        // A self-dep: not a borrow, and not the empty list @FR-O-Proxy reads as "owner", so no
        // site frees the snapshot on its own (the owner witness's construction).
        function.depend(w, w);
        scopes.var_scope.insert(w, 0);
        scopes.var_order.push(w);
        scopes.entry_witness = Some((buf, w));
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
    // ⚠ @FR-N-Road (@PLN160) — this mechanism is NULLABLE BY CONSTRUCTION, and the dense
    // spelling of the same mixed-ownership local answers the identical question by STORE
    // IDENTITY (`OpFreeRefIfDistinct`, plus the `_old_` guard at the rebind).  C90 makes the two
    // spellings the same slot, so nullability does not require a runtime flag here and the
    // divergence is an accident of this site.  Reach: 5 of 1311 corpus files emit a `__lbo_`.
    // Not folded — whether it CAN be is unmeasured (the falsification was attempted and its own
    // control caught it going vacuous), and it belongs to @PLN155's question, "a limited amount
    // of code that verifies if a free is needed", rather than to @PLN160's.  Note also that
    // `__own_` has `LOFT_NO_OWNER_WITNESS=1` and this one has no `LOFT_NO_*` switch at all, so
    // the one of the three whose necessity cannot be shown is the one with no bisect step.
    // loft#1515 — the per-path hand-offs an author wrote, and one boolean per STOPPED variable
    // to record whether the copy that stops it actually ran.  Minted here rather than during
    // the scan for the reason `local_owns` is: the flag has to exist before the body is walked,
    // so its `false` initialiser can be placed at the top.  The stopped side is the SOURCE for
    // a copy that takes its release and the DESTINATION for a copy off a PARAMETER, because the
    // caller owns what it holds (`per_path_stops`).  Stopped on every path instead, the second
    // lost a release: after `x = mk(); if c { x = p; }` the stopped `x` left `mk()` unreleased
    // on the path where the copy did not run (heap-history.md `D-heap-7`, the branch not taken).
    // `(H-Drop)` — the locals that hold the caller's record on every path, marked before any reader
    // of `copy_moves_drop_from` runs: the per-path registration just below, the statement scan, and
    // the double-move lint after the scope pass all read the mark through that one decider.
    let caller_records = caller_record_locals(orig_code, &function, data);
    for v in caller_records {
        function.mark_caller_record(v);
    }
    // A copy that stops its DESTINATION written in a LOOP body is per path too — per iteration: the
    // next pass displaces what this one copied, so the answer is the flag's (heap-history.md `D-heap-7`, the
    // loop).  A copy that moves its source's release keeps the loop's early seed.
    let loop_copies = loop_self_stopping_copies(orig_code, &function, data);
    scopes.per_path_pairs = per_path_handoffs(orig_code)
        .into_iter()
        .chain(loop_copies)
        .filter(|&(dst, src)| per_path_stops(&function, data, dst, src).is_some())
        .collect();
    let mut stopped: Vec<u16> = scopes
        .per_path_pairs
        .iter()
        .filter_map(|&(dst, src)| per_path_stops(&function, data, dst, src))
        .collect();
    stopped.sort_unstable();
    stopped.dedup();
    for var in stopped {
        scopes.mint_handoff_flag(&mut function, var);
    }
    // D-heap-14 — the same per-path fact for a copy that hands a droppable to a CONTAINER inside
    // a branch arm: a field, an element, a construction or a return buffer.  Recorded as the pair
    // `(u16::MAX, source)` beside the `Set` pairs above, so the collector keeps the source out of
    // the static set, and the scan sets the source's flag where the copy runs.
    for src in arm_container_handoffs(orig_code, &function, data) {
        scopes.per_path_pairs.insert((u16::MAX, src));
        scopes.mint_handoff_flag(&mut function, src);
    }
    scopes.owned_handle_members = owned_handle_members(orig_code, &function, data);
    for v in mixed_handle_views(orig_code, &function, data) {
        function.clear_skip_free(v);
        scopes.mint_handoff_flag(&mut function, v);
        scopes.handle_views.insert(v);
    }
    // D-heap-23 — the store behind a collection the function owns, whose elements a
    // whole-collection copy moved elsewhere, stops releasing them on the path the copy ran.
    for backing in collection_handoffs(orig_code, &function, data, &scopes.vector_backings) {
        scopes.per_path_pairs.insert((u16::MAX, backing));
        scopes.mint_handoff_flag(&mut function, backing);
    }
    // loft#1336 / `(H-Drop)` — a witnessed local assigned a copy off a PARAMETER, anywhere in the
    // body: the witness releases the store the copy is and must skip its hook, so the local gets
    // the same flag.  Sorted, so the flags are minted in one order on every compile.
    let mut witnessed_by_owner: Vec<u16> = scopes.owner_witness.keys().copied().collect();
    witnessed_by_owner.sort_unstable();
    for v in witnessed_by_owner {
        if assigns_a_self_stopping_copy(orig_code, &function, data, v) {
            scopes.mint_handoff_flag(&mut function, v);
        }
    }
    // `(H-Drop)` — a local assigned a copy off a FLAGGED local inherits that flag at the copy
    // (`scan_set`), so it needs a flag of its own: the record it takes is not its to release on
    // exactly the paths it was not the source's.  A fixpoint, because a copy of such a copy
    // inherits in turn.
    let copy_pairs = local_copy_pairs(orig_code, &function, data);
    loop {
        let mut minted = false;
        for &(dst, src) in &copy_pairs {
            if scopes.handed_off.contains_key(&src) && !scopes.handed_off.contains_key(&dst) {
                scopes.mint_handoff_flag(&mut function, dst);
                minted = true;
            }
        }
        if !minted {
            break;
        }
    }
    let displace_locals =
        nullable_locals_that_displace(orig_code, &function, data, &scopes.null_led_first);
    for &v in &displace_locals {
        // loft#1522 — keyed by the VAR, not by its name.  `add_temp_var` identifies a temp by
        // name and hands back the existing one, so two locals of the same name in SIBLING
        // scopes (`for … { y: S? = … }` twice, which is ordinary) shared one ownership bit —
        // and the second got no guarded free at all, leaving its displaced store to whatever
        // free site happened to be left.  A per-run ownership fact belongs to one binding; the
        // `#` matches the compiler's own `i#index` spelling and cannot collide with a user
        // name.
        let name = format!("__lbo_{}#{v}", function.name(v));
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
    // loft#1697 — the scan reads `orig_code` and writes the body back only below, so the
    // ownership oracle's whole-function walk holds for the whole scan: done once, not once per
    // call site `inline_struct_return` asks about.
    let defs_memo = crate::use_analysis::defs_memo_scope(data, d_nr);
    let mut code = scopes.scan(orig_code, &mut function, data);
    drop(defs_memo);
    // The witness starts FALSE: on entry the buffer holds the CALLER's store, which this
    // function must never release.  A transition site is reachable with no prior assignment at
    // all (`fn g() -> Res { mk(2) }`), and `needs_pre_init` does not cover `boolean`, so an
    // uninitialised slot would read as garbage and free the caller's buffer.
    if let Some((_, flag)) = scopes.rbuf_witness
        && let Some(bl) = body_block_mut(&mut code)
    {
        bl.operators.insert(0, v_set(flag, Value::Boolean(false)));
    }
    // The entry witness names what the buffer holds BEFORE any statement can rebind it.
    if let Some((buf, w)) = scopes.entry_witness
        && let Some(bl) = body_block_mut(&mut code)
    {
        bl.operators.insert(
            0,
            v_set(
                w,
                Value::Call(data.def_nr("OpRefAlias"), vec![Value::Var(buf)]),
            ),
        );
    }
    // Every per-local witness starts FALSE for the same reason: before the local's first
    // assignment there is no store of its own to release, and an uninitialised boolean slot
    // would read as garbage and free one.
    if !scopes.local_owns.is_empty()
        && let Some(bl) = body_block_mut(&mut code)
    {
        let mut flags: Vec<u16> = scopes.local_owns.values().copied().collect();
        flags.sort_unstable();
        for flag in flags.into_iter().rev() {
            bl.operators.insert(0, v_set(flag, Value::Boolean(false)));
        }
    }
    // loft#1515 — and every hand-off flag, for the same reason: before the branch runs no
    // copy has taken the source's release, so an uninitialised slot would read as garbage and
    // suppress a release that is owed.
    if !scopes.tuple_moved.is_empty()
        && let Some(bl) = body_block_mut(&mut code)
    {
        let mut flags: Vec<u16> = scopes.tuple_moved.values().copied().collect();
        flags.sort_unstable();
        for flag in flags.into_iter().rev() {
            bl.operators.insert(0, v_set(flag, Value::Boolean(false)));
        }
    }
    if !scopes.handed_off.is_empty()
        && let Some(bl) = body_block_mut(&mut code)
    {
        let mut flags: Vec<u16> = scopes.handed_off.values().copied().collect();
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
        && let Some(bl) = body_block_mut(&mut code)
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
    // @FR-O-Override — a temp whose value a consuming op takes over (`mark_lift_handoff`'s
    // store hand-off: `OpReplaceKeyed` with its source-free bit) owns nothing at ANY later
    // free, not only at scope exit.  The op either freed the store or declined because it
    // was a protected borrow, and neither is the temp's to release.  Marked never-free, the
    // rebind on the next pass through a loop emits no displaced-store free on either
    // backend — that free released the slot the op had already freed, which by then was
    // whatever the allocator had handed it to next.
    let mut transferred: Vec<u16> = scopes.free_transferred.iter().copied().collect();
    transferred.sort_unstable();
    for v in transferred {
        function.set_skip_free(v);
    }
    // lift vars from `scan_args` are assigned inside conditional branches but
    // their `OpFreeRef` lives at function exit; prepend the null-inits so codegen
    // reserves their slot along every path (see the original comment in check).
    if !scopes.lift_vars.is_empty()
        && let Some(bl) = body_block_mut(&mut code)
    {
        for &v in scopes.lift_vars.iter().rev() {
            bl.operators.insert(0, v_set(v, Value::Null));
        }
    }
    if !scopes.lift_texts.is_empty()
        && let Some(bl) = body_block_mut(&mut code)
    {
        for &v in scopes.lift_texts.iter().rev() {
            bl.operators.insert(0, v_set(v, Value::Text(String::new())));
        }
    }
    reuse_record_buffers(
        &mut code,
        &mut function,
        data,
        d_nr,
        &scopes.witness_buffer,
        &scopes.minted_pairs,
        // A local whose other binds are the nulls in front of it on every pass displaces
        // nothing (`null_led_first_binds_in`, loft#1643).
        &scopes
            .multi_assigned
            .difference(&scopes.null_led_first)
            .copied()
            .collect(),
    );
    lazy_buffer_mints(&mut code, &mut function, data);
    function.set_capture_builds(std::mem::take(&mut scopes.capture_build_backing));
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
    let written_out = std::mem::take(&mut scopes.written_out);
    for (v_nr, scope) in scopes.var_scope {
        data.definitions[d_nr as usize]
            .variables
            .set_scope(v_nr, scope);
    }
    written_out
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
    let mut out = HashMap::default();
    walk(node, data, d_nr, &mut out);
    out
}

/// Variables that are the target of ANY `Set` node in `node`.  For a PARAMETER this is the
/// rebind test: its own bind is the call, so a single `Set` already means it stops naming
/// the caller's store (`v = other` on a value-const vector copies into a fresh buffer and
/// re-points the slot), and a witness compared against it afterwards names the wrong
/// store — @PLN157 § V-g's c11 freed the caller's vector that way.
pub(crate) fn assigned_in(node: &Value) -> HashSet<u16> {
    fn collect(node: &Value, out: &mut HashSet<u16>) {
        if let Value::Set(v, _) = node.unspan() {
            out.insert(*v);
        }
        node.for_each_child(&mut |c| collect(c, out));
    }
    let mut out = HashSet::default();
    collect(node, &mut out);
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
    let mut counts = HashMap::default();
    count(node, &mut counts);
    counts
        .into_iter()
        .filter(|&(_, n)| n >= 2)
        .map(|(v, _)| v)
        .collect()
}

/// The `??` hoists (`__ncc_N`) whose every bind is a whole LOCAL or a null — never a projection,
/// a call or a construction.  `@FR-B-Copy` / `@FR-B-View`.
///
/// A subject that is not a variable is hoisted into a temp that VIEWS what the subject
/// evaluated to, and whether the binding of the `??` copies it depends on what that was.  A
/// projection (`v[i] ?? d`, `o.inner ?? d`) is a place, and the binding views it as its plain
/// spelling does.  A whole local reached through a cast or a branch (`(e as B?) ?? d`,
/// `(if c { s } else { null }) ?? d`) is a VALUE, and the plain bind of a value copies — so
/// the hoist has to be read as the local it holds, or the binding aliases it (loft#1752).
///
/// Read off the hoist's own `Set`s, so both the parser's single bind (`__ncc = if c { s }
/// else { null }`) and the per-arm binds the sink writes out of it are seen.  A local counts
/// only when the author wrote it: a compiler temp is a slot inside something else.
fn whole_value_hoists_in(code: &Value, function: &Function, data: &Data) -> HashSet<u16> {
    // `Some(true)`: a whole local on some path; `Some(false)`: only nulls; `None`: anything else.
    fn tails(v: &Value, function: &Function, data: &Data) -> Option<bool> {
        match v.unspan() {
            Value::Var(x) => (!function.is_compiler_generated(*x)).then_some(true),
            Value::Null => Some(false),
            Value::Call(d, a)
                if a.is_empty()
                    && matches!(
                        data.def(*d).name(),
                        "OpNullRefSentinel" | "OpConvRefFromNull"
                    ) =>
            {
                Some(false)
            }
            Value::If(_, t, f) => {
                let (t, f) = (tails(t, function, data)?, tails(f, function, data)?);
                Some(t || f)
            }
            Value::Block(bl) if !matches!(bl.result.base(), Type::Void | Type::Null) => {
                tails(bl.operators.last()?, function, data)
            }
            _ => None,
        }
    }
    // The hoists with a whole local on some path, then minus any with a bind that is neither a
    // local nor null.  In this order the set stays unallocated for the (usual) function whose
    // hoists hold no whole local, which is every hoist in the stdlib (`frontend_counts`).
    let mut whole: HashSet<u16> = HashSet::default();
    code.walk(&mut |n| {
        if let Value::Set(v, rhs) = n.unspan()
            && is_discharge_hoist(function, *v)
            && tails(rhs, function, data) == Some(true)
        {
            whole.insert(*v);
        }
    });
    if whole.is_empty() {
        return whole;
    }
    // A `None` bind disqualifies; a bind that is only null neither makes nor breaks it.
    code.walk(&mut |n| {
        if let Value::Set(v, rhs) = n.unspan()
            && whole.contains(v)
            && tails(rhs, function, data).is_none()
        {
            whole.remove(v);
        }
    });
    whole
}

/// Variables with exactly ONE bind that is not a `null` (a `Value::Null` or the
/// `OpNullRefSentinel` an explicit `x: T? = null` lowers to), and at least one that is.
/// On every path that reaches the one real bind the local holds the sentinel, so that bind
/// is a FIRST bind in `@FR-O-Move`'s sense, whoever wrote the null before it.
pub(crate) fn null_led_in(node: &Value, data: &Data) -> HashSet<u16> {
    fn is_null(v: &Value, data: &Data) -> bool {
        match v.unspan() {
            Value::Null => true,
            Value::Call(d, a) => a.is_empty() && data.def(*d).name() == "OpNullRefSentinel",
            _ => false,
        }
    }
    fn count(node: &Value, data: &Data, out: &mut HashMap<u16, (usize, usize)>) {
        if let Value::Set(v, value) = node.unspan() {
            let e = out.entry(*v).or_insert((0, 0));
            if is_null(value, data) {
                e.0 += 1;
            } else {
                e.1 += 1;
            }
        }
        node.for_each_child(&mut |c| count(c, data, out));
    }
    let mut counts = HashMap::default();
    count(node, data, &mut counts);
    counts
        .into_iter()
        .filter(|&(_, (nulls, real))| nulls >= 1 && real == 1)
        .map(|(v, _)| v)
        .collect()
}

/// Variables whose one real bind can hold nothing but the sentinel it overwrites: every bind
/// but one writes `null`, and a null is written EARLIER in a block that ENCLOSES that real bind
/// with no loop between the two.  So every path to the real bind wrote the null first, on
/// every pass, and the bind displaces no store — the author's `x: T? = null; if c { x = mk() }`
/// inside a loop body, which `(B-Scope)` makes the spelling of a local an arm assigns and a
/// later statement reads.  Two nulls that do not qualify: one written only OUTSIDE the loop
/// the real bind sits in (the second pass's bind displaces the first pass's store), and one in
/// a sibling arm (the path through the other arm reaches the bind without it).
///
/// Stricter than [`null_led_in`], which counts binds and reads no position.
pub(crate) fn null_led_first_binds_in(node: &Value, data: &Data) -> HashSet<u16> {
    fn is_null(v: &Value, data: &Data) -> bool {
        match v.unspan() {
            Value::Null => true,
            Value::Call(d, a) => a.is_empty() && data.def(*d).name() == "OpNullRefSentinel",
            _ => false,
        }
    }
    /// A region is a block, a loop body or an `if` arm: `(id, is_loop)`.
    type Regions = Vec<(usize, bool)>;
    #[derive(Default)]
    struct Binds {
        nulls: Vec<Regions>,
        reals: usize,
        led: bool,
        null_after_real: bool,
    }
    struct Walk<'a> {
        data: &'a Data,
        regions: Regions,
        next: usize,
        out: HashMap<u16, Binds>,
    }
    impl Walk<'_> {
        fn region(&mut self, node: &Value, is_loop: bool) {
            self.next += 1;
            self.regions.push((self.next, is_loop));
            self.node(node);
            self.regions.pop();
        }
        fn node(&mut self, node: &Value) {
            match node.unspan() {
                Value::If(test, t, f) => {
                    self.node(test);
                    self.region(t, false);
                    self.region(f, false);
                    return;
                }
                Value::Loop(_) | Value::Block(_) => {
                    let is_loop = matches!(node.unspan(), Value::Loop(_));
                    self.next += 1;
                    self.regions.push((self.next, is_loop));
                    node.unspan().for_each_child(&mut |c| self.node(c));
                    self.regions.pop();
                    return;
                }
                _ => {}
            }
            node.unspan().for_each_child(&mut |c| self.node(c));
            if let Value::Set(v, value) = node.unspan() {
                let here = self.regions.clone();
                let e = self.out.entry(*v).or_default();
                if is_null(value, self.data) {
                    if e.reals == 0 {
                        e.nulls.push(here);
                    } else {
                        e.null_after_real = true;
                    }
                } else {
                    e.reals += 1;
                    e.led = e
                        .nulls
                        .iter()
                        .any(|n| here.starts_with(n) && here[n.len()..].iter().all(|&(_, lp)| !lp));
                }
            }
        }
    }
    let mut w = Walk {
        data,
        regions: Vec::new(),
        next: 0,
        out: HashMap::default(),
    };
    w.node(node);
    w.out
        .into_iter()
        .filter(|(_, b)| b.reals == 1 && b.led && !b.null_after_real)
        .map(|(v, _)| v)
        .collect()
}

/// How many nodes of `node` name each variable: a read, a write, a tuple member read or
/// write, a fn-ref's closure slot or projection, a call through a fn-ref local and an
/// iterator's own variable.  A count, so a region's share of it says whether a variable is
/// mentioned anywhere else.
pub(crate) fn var_mentions_in(node: &Value) -> HashMap<u16, usize> {
    fn count(node: &Value, out: &mut HashMap<u16, usize>) {
        let named = match node.unspan() {
            Value::Var(v)
            | Value::Set(v, _)
            | Value::TupleGet(v, _)
            | Value::TuplePut(v, _, _)
            | Value::FnRefDnr(v)
            | Value::CallRef(v, _)
            | Value::Iter(v, _, _, _) => Some(*v),
            Value::FnRef(_, v, _) if *v != u16::MAX => Some(*v),
            _ => None,
        };
        if let Some(v) = named {
            *out.entry(v).or_insert(0) += 1;
        }
        node.for_each_child(&mut |c| count(c, out));
    }
    let mut counts = HashMap::default();
    count(node, &mut counts);
    counts
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
pub(super) fn hidden_return_buffer_var(d_nr: u32, function: &Function, data: &Data) -> Option<u16> {
    let def = data.def(d_nr);
    let idx = def.hidden_return_buffer_attr()?;
    let name = def.attributes().get(idx)?.name.clone();
    let v = function.var(&name);
    (v != u16::MAX).then_some(v)
}
