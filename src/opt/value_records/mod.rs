// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-ValueRecord` — the functions whose no-heap record result, and the parameters whose
//! record argument, travel BY VALUE (a tuple of the fields) instead of through a store
//! record.  The IR pass `value_record` and the native generator both read the answer.

use crate::data::{Data, DefType, Type, Value};
use crate::database::Stores;
use crate::opt::attrs::ret_buffer_attr;
use crate::opt::body_writes::{copy_type, field_on, sub_record_of};
use crate::opt::value_records::layout::{
    FnRefSites, TupleParams, ValueRecords, ValueTuple, ViewOffsets, ViewPlan, fnref_value,
    is_nullable_layout, is_view_part, layout_record, nullable_layout, nullable_layout_of,
    nullable_presence_layout, nullable_value_record_disabled, tuple_param_candidates,
    tuple_param_vars, type_layout, value_record_disabled,
};
use crate::opt::value_records::leaves::{
    VALUE_RECORD_GETTERS, VALUE_RECORD_SETTERS, collect_leaves, converted_build_ref, dropped_reads,
    fnref_handoffs, fnref_site_listed, handoffs_and_presence, is_null_record, is_presence_test,
    object_own_return, release_uses, served_reads, temp_presence_unserved, view_leaf_plan,
};
use crate::opt::value_records::sites::{
    Pos, ShapeCtx, close_fnref_groups, local_surely_present, mv_block_buffer, mv_return_source,
    own_retbuf, site_walk, surely_present, view_field_on, view_field_reads, view_sites_declined,
};
use crate::opt::writes::plain_record_type;
use std::collections::{HashMap, HashSet};

pub mod layout;
pub mod leaves;
pub mod sites;

/// @PLN157 § V-aa (`@FR-R-ValueRecord`) — which record-returning functions may return
/// their fields in registers.
///
/// Two gates, and the second is what makes the first safe to apply per FUNCTION: the
/// result type is a plain no-heap struct of at most [`VALUE_RECORD_MAX_FIELDS`](layout::VALUE_RECORD_MAX_FIELDS) scalar
/// fields; and EVERY call site in the program consumes the result by reading fields off a
/// local it binds — never storing it, passing it on, returning it or binding it into a
/// collection.  A single non-reading site declines the whole function, so no site has to
/// materialise a record out of a tuple and no site can be made slower.
///
/// `library` is a library artifact's generation: its dispatches can end in the call back to
/// the interpreter, whose callee takes and answers records (loft#2129).
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn value_records(data: &Data, stores: &Stores, library: bool) -> ValueRecords {
    let mut out = ValueRecords::default();
    if value_record_disabled() {
        return out;
    }
    let trace = std::env::var("LOFT_TRACE_VALUEREC").is_ok();
    let mut clock = Clock::new(std::env::var_os("LOFT_TIMING").is_some());
    // Candidates by RESULT TYPE.
    let mut cand: HashMap<u32, u16> = HashMap::new();
    for d_nr in 0..data.definitions.len() as u32 {
        let def = data.def(d_nr);
        if !matches!(def.def_type, crate::data::DefType::Function)
            || matches!(def.code(), Value::Null)
        {
            continue;
        }
        let Type::Reference(rd, _) = def.returned().peel_link() else {
            continue;
        };
        if trace {
            eprintln!("[valuerec] candidate {} returns ref({rd})", def.name());
        }
        // `@FR-R-ValueRecord`'s nullable clause (`@FR-N-Road`) — a `-> S?` is a candidate
        // exactly as `-> S` is, carried by the nullable layout: the `?` must not be what
        // selects the road.
        let (ret, nullable) = def.returned().peel_optional();
        if nullable && nullable_value_record_disabled() {
            continue;
        }
        let Some(tp) = plain_record_type(data, ret) else {
            if trace {
                eprintln!("[valuerec] {}: not a plain record", def.name());
            }
            continue;
        };
        // @PLN164 C5 (`@FR-O-ViewField`) — a record that OWNS heap is a candidate only
        // through the view leaf: each heap field must be one a reference can deliver
        // ([`view_leaf_type`]) and the body gate must find the place it views.  With the
        // unit off, owning heap declines the record as it did before.
        let views_ok = view_field_on();
        if stores.owns_heap(tp) && !views_ok {
            continue;
        }
        // A nullable record that owns heap keeps its buffer: its heap fields would be view
        // leaves, and a view leaf's absence is a REPORTED null where a `null` exit's is not
        // ([`ValueTuple::absent`] spells only the unreported one), so the two could not
        // share one presence word.
        if nullable && stores.owns_heap(tp) {
            if trace {
                eprintln!(
                    "[valuerec] {}: a nullable record that owns heap",
                    def.name()
                );
            }
            continue;
        }
        let Some(layout) = type_layout(data, stores, *rd, tp, views_ok, &mut out.index) else {
            continue;
        };
        // The BODY gate — every result position a value leaf — runs in the fixpoint
        // below, because what counts as a leaf depends on what else is admitted.
        let key = if nullable {
            let Some(nkey) = nullable_layout(tp) else {
                continue;
            };
            let nlayout = nullable_layout_of(&layout, tp, nkey, &mut out.index);
            out.types.entry(nkey).or_insert(nlayout);
            nkey
        } else {
            tp
        };
        out.types.entry(tp).or_insert(layout);
        cand.insert(d_nr, key);
    }
    // `(R-ValueLocal)` — the by-value parameters that may be received as tuples, by the
    // facts that never change: the parameter's type and shape, and what the body writes.
    // Which of them ARE received as tuples is decided in the fixpoint below beside the
    // value locals, because a parameter handed on is served only by another tuple parameter.
    let mut params = tuple_param_candidates(data, stores, &mut out.types, &mut out.index);
    clock.add("candidates");
    if cand.is_empty() && params.is_empty() {
        return out;
    }
    // A function reachable through a FN-REF changes its ABI only together with every other
    // arm of the dispatch that reaches it.  The dispatch a `CallRef` emits is a `match` whose
    // arms are every definition `fnref::dispatch_arms` admits for the fn variable's type —
    // all of them sharing one argument list and one return type — so converting one arm to a
    // tuple alone breaks the join.  The question is asked ONCE, of the same function the
    // emitter builds the match from (asked a second way it drifted: a by-record test missed a
    // lambda reaching a dispatch through a typed variable with no `FnRef` node beside it).
    //
    // `(R-FnRefValue)` (`@FR-R-FnRefValue`) — a site whose signature returns a plain record
    // lists its arms as a GROUP ([`FnRefSites`]): the group answers the tuple when every arm
    // is admitted, and the fixpoint below takes the whole group out when one arm leaves.
    // Declined outright, as before: the arms of every other dispatch, a `par` worker, and a
    // `FnRef` target no listed site dispatches — a function value with no `CallRef` of its
    // type in sight keeps its record return, which costs the rewrite and never a value.
    let mut arms: HashSet<u32> = HashSet::new();
    let mut grouped: HashSet<u32> = HashSet::new();
    let mut targets: HashSet<u32> = HashSet::new();
    let mut sites: FnRefSites = HashMap::new();
    let every: HashSet<u32> = HashSet::new();
    for d_nr in 0..data.definitions.len() as u32 {
        let def = data.def(d_nr);
        // A generic TEMPLATE is never emitted — only its instances are (`@FR-G-Mono`) — so a
        // call through a fn-ref inside one dispatches nowhere.  Asked of it anyway, its
        // `fn(T) -> U` reads `fn(DbRef) -> DbRef` (the placeholder is a record) and declined
        // every record-to-record function in the program: the stdlib's `map`/`filter`/
        // `reduce` declarations (@PLN165 E5–E7) turned the value-record return off for a
        // method as plain as `rdouble(self: RColor) -> RColor`.
        if matches!(def.code(), Value::Null) || def.def_type() == DefType::Generic {
            continue;
        }
        let vars = def.variables();
        def.code().any_node(&mut |n| {
            match n {
                Value::FnRef(target, _, _) => {
                    if let Ok(t) = u32::try_from(*target)
                        && (t as usize) < data.definitions.len()
                    {
                        targets.insert(t);
                    }
                }
                Value::CallRef(v, args) => {
                    if *v < vars.count()
                        && let Some(found) =
                            crate::fnref::dispatch_arms(data, &every, vars.tp(*v), args.len())
                    {
                        let ds: Vec<u32> = found.iter().map(|a| a.d_nr).collect();
                        // A group changes its arms' signatures together, which holds only
                        // while they are every callee (`@FR-R-FnRefValue`): in a library
                        // artifact a dispatch the call back carries ends in an interpreted one
                        // (`output_call_ref`'s own condition), so its arms keep their records.
                        let open = library && crate::fnref::upcall_carries(vars.tp(*v));
                        if !open && fnref_site_listed(data, vars.tp(*v), args, &found) {
                            // The record the dispatch may answer as a tuple, when it answers one.
                            let rec = match vars.tp(*v).base() {
                                Type::Function(_, ret, ..) => plain_record_type(data, ret),
                                _ => None,
                            };
                            grouped.extend(ds.iter().copied());
                            sites.insert((d_nr, *v), (ds, rec));
                        } else {
                            arms.extend(ds);
                        }
                    }
                }
                // The third spelling: a `par` worker, named by its number as an integer
                // argument of the queue op, which the parallel emitter calls through its
                // own buffered spelling.
                Value::Call(d, args) => {
                    if (*d as usize) < data.definitions.len()
                        && let Some((i, min)) =
                            crate::fnref::parallel_worker_arg(data.def(*d).name())
                        && args.len() >= min
                        && let Value::Int(n) = args[i].unspan()
                        && *n >= 0
                    {
                        arms.insert(*n as u32);
                    }
                }
                _ => {}
            }
            false
        });
    }
    clock.add("fnref_scan");
    arms.extend(targets.iter().filter(|t| !grouped.contains(t)));
    cand.retain(|d_nr, _| {
        let keep = !arms.contains(d_nr);
        if !keep && trace {
            crate::loft_eprintln!(
                "[valuerec] {}: an arm of a fn-ref dispatch",
                data.def(*d_nr).name()
            );
        }
        keep
    });
    // A tuple PARAMETER changes the signature exactly as a tuple result does, so an arm of a
    // fn-ref dispatch keeps every record parameter too (`@FR-R-ValueLocal`) — unless the
    // dispatch is a listed group, whose arms change a parameter together
    // ([`close_fnref_groups`]).
    params.retain(|d_nr, _| {
        let keep = !arms.contains(d_nr);
        if !keep && trace {
            crate::loft_eprintln!(
                "[valuerec] {}: an arm of a fn-ref dispatch keeps its record parameters",
                data.def(*d_nr).name()
            );
        }
        keep
    });
    close_fnref_groups(data, &sites, &mut cand, &mut params, trace);
    // A dispatch passes ONE argument list to all its arms, built from the signature's dense
    // record types, so an arm keeps a `p: S?` a reference and is never promoted
    // ([`promote_presence`]).
    let mut frozen: HashSet<u32> = arms.clone();
    frozen.extend(grouped.iter().copied());
    for d in &frozen {
        if let Some(ps) = params.get_mut(d) {
            ps.retain(|_, key| !is_nullable_layout(*key));
        }
    }
    params.retain(|_, ps| !ps.is_empty());
    // The view-leaf plans, rebuilt each round beside the body gate that decides them (the
    // admitted set they read changes with it) and kept for the round that ends the fixpoint.
    let mut views: HashMap<u32, ViewPlan> = HashMap::new();
    // @PLN164 C5 — the callee half of `(B-Disturb)`, built ONCE over the whole program as
    // the scope pass builds it: a site asks whether a call between its bind and its last
    // read can grow the container its leaf views, and that is a fact about the callee.
    clock.add("fnref_groups");
    let disturbed =
        view_field_on().then(|| crate::scopes::disturbed_params_map(data, Some(stores)));
    clock.add("disturbed_map");

    // The view-leaf OFFSETS are fixed by the record's type, so they are read once: every
    // reader of a value local needs them to account a field read the tuple serves.
    let view_offs: ViewOffsets = out
        .types
        .iter()
        .map(|(tp, t)| {
            (
                *tp,
                t.fields
                    .iter()
                    .filter(|(_, rt)| is_view_part(rt))
                    .map(|(off, _)| *off)
                    .collect(),
            )
        })
        .collect();
    // The functions each body calls directly, read once.  A round's site gate declines only
    // an ADMITTED callee a caller calls ([`site_walk`], [`view_sites_declined`]), an arm of
    // one of the caller's own listed fn-ref sites ([`site_walk_callref`]), or a tuple
    // parameter of the caller itself — so a caller that calls no admitted function, is
    // neither admitted nor a parameter holder, and owns no listed site has nothing to
    // decline, and the round skips it.  Most of a program is such callers, and walking
    // them every round was nearly the whole cost of this analysis.
    let callees: Vec<HashSet<u32>> = (0..data.definitions.len() as u32)
        .map(|d| {
            let mut called = HashSet::new();
            data.def(d).code().any_node(&mut |n| {
                if let Value::Call(c, _) = n {
                    called.insert(*c);
                }
                false
            });
            called
        })
        .collect();
    let site_owners: HashSet<u32> = sites.keys().map(|(caller, _)| *caller).collect();
    // `LOFT_NO_VR_SKIP=1` walks every caller every round, as before: the control half of
    // the A/B that shows the skip changes no decision.
    let skip = std::env::var_os("LOFT_NO_VR_SKIP").is_none();
    // Admission is a FIXPOINT.  A body's tail may FORWARD another candidate's result and a
    // site may bind a BRANCH of candidate calls, so declining one function can decline
    // another; every round only removes, so it ends.  The body gate and the site gate read
    // the same three helpers the emitter reads (`value_shape`, `value_locals_in`,
    // `value_view_leaves`), so what is admitted here is exactly what is emitted there.
    let presence_keys = |cand: &HashMap<u32, u16>, params: &TupleParams| {
        cand.values()
            .chain(params.values().flat_map(HashMap::values))
            .filter(|k| is_nullable_layout(**k))
            .count()
    };
    loop {
        clock.rounds += 1;
        let before = (
            cand.len(),
            params.values().map(HashMap::len).sum::<usize>(),
            presence_keys(&cand, &params),
        );
        // `@FR-N-Store` — first, carry the presence wherever a dense tuple would otherwise
        // be widened without it.  Every round, because a decline can leave a new leaf at a
        // nullable position; it only ever turns a dense key into a nullable one, so it ends.
        if !nullable_value_record_disabled() {
            loop {
                let sp = surely_present_fns(data, &cand, &params, &sites);
                if !promote_presence(
                    data,
                    &mut cand,
                    &mut params,
                    &mut out.types,
                    &mut out.index,
                    &frozen,
                    &sp,
                    trace,
                ) {
                    break;
                }
            }
        }
        clock.add("presence");
        let admitted: HashMap<u32, u16> = cand.clone();
        let sp = surely_present_fns(data, &admitted, &params, &sites);
        cand.retain(|d_nr, record| {
            let mut why = value_body(data, *d_nr, &admitted, &params, &view_offs, &sites, &sp);
            // @PLN164 C5 — and, for a record with a heap field, the body must deliver a
            // view of a place that outlives the frame for every one of its exits.
            if why.is_none()
                && let Some(fields) = out.types.get(record).map(|t| t.fields.as_slice())
                && fields.iter().any(|(_, rt)| is_view_part(rt))
            {
                // @C122 — a rewrite is free where its conditions are validated, and stops at the
                // library API.
                // @PLN164 C5 / `(R-Escape)` — a `pub` function's result can leave the
                // unit the compiler sees whole: a library's cdylib bridge materialises a
                // tuple field by field into the record its ABI promises, and a reference
                // is not a field it can write.  So the record form stands for every `pub`
                // function, which costs the rewrite there and never a value; the internal
                // functions a `pub` one calls are where the row is.
                if data.def(*d_nr).pub_visible {
                    why = Some("a view leaf would cross a library API (R-Escape)");
                } else {
                    let locals =
                        value_locals_in(data, *d_nr, &admitted, &params, &view_offs, &sites, &sp);
                    let leaves = collect_leaves(data.def(*d_nr).code(), &locals, true, data);
                    match view_leaf_plan(
                        data,
                        stores,
                        *d_nr,
                        fields,
                        &leaves,
                        *record,
                        disturbed.as_ref(),
                    ) {
                        Some(plan) => {
                            views.insert(*d_nr, plan);
                        }
                        None => why = Some("a heap field is not a view leaf"),
                    }
                }
            }
            if let Some(why) = why
                && trace
            {
                crate::loft_eprintln!("[valuerec] {}: {why}", data.def(*d_nr).name());
            }
            why.is_none()
        });
        clock.add("body_gate");
        let admitted: HashMap<u32, u16> = cand.clone();
        let sp = surely_present_fns(data, &admitted, &params, &sites);
        clock.add("surely_present");
        let mut declined: HashSet<u32> = HashSet::new();
        // `(R-ValueLocal)` — a parameter whose uses the tuple cannot serve leaves this
        // round; a site that handed a value local to it is then unserved next round.
        let mut declined_params: Vec<(u32, usize)> = Vec::new();
        for caller in 0..data.definitions.len() as u32 {
            let cdef = data.def(caller);
            if matches!(cdef.code(), Value::Null) {
                continue;
            }
            if skip
                && !admitted.contains_key(&caller)
                && !params.contains_key(&caller)
                && !site_owners.contains(&caller)
                && !callees[caller as usize]
                    .iter()
                    .any(|c| admitted.contains_key(c))
            {
                continue;
            }
            let locals = value_locals_in(data, caller, &admitted, &params, &view_offs, &sites, &sp);
            let own = admitted.contains_key(&caller).then_some(caller);
            let c = ShapeCtx {
                data,
                def_nr: caller,
                admitted: &admitted,
                params: &params,
                locals: &locals,
                own,
                fnref: &sites,
                sp: &sp,
            };
            let top = if own.is_some() {
                Pos::Tail
            } else {
                Pos::Operand
            };
            site_walk(cdef.code(), top, &c, &mut declined);
            if let Some(ps) = params.get(&caller) {
                let served = served_reads(cdef.code(), &locals, own.is_some(), data);
                let cvars = cdef.variables();
                for (idx, tp) in ps {
                    let v = cdef
                        .attributes()
                        .get(*idx)
                        .map_or(u16::MAX, |a| cvars.var(&a.name));
                    let empty = HashSet::new();
                    let offs = view_offs.get(tp).unwrap_or(&empty);
                    if v == u16::MAX
                        || !local_uses_ok(cdef.code(), v, &c, &served, offs, None, &HashSet::new())
                    {
                        declined_params.push((caller, *idx));
                    }
                }
            }
            // @PLN164 C5 — and the view leaf's own site condition: the bind dominates the
            // reads and nothing between them disturbs the place the leaf views.
            if !views.is_empty() {
                declined.extend(view_sites_declined(
                    data,
                    stores,
                    caller,
                    &views,
                    &admitted,
                    &locals,
                    &view_offs,
                    disturbed.as_ref(),
                ));
            }
        }
        clock.add("site_gate");
        if trace {
            for d in &declined {
                crate::loft_eprintln!(
                    "[valuerec] {}: a site consumes its record",
                    data.def(*d).name()
                );
            }
        }
        cand.retain(|d, _| !declined.contains(d));
        for (d, idx) in declined_params {
            if trace {
                crate::loft_eprintln!(
                    "[valuerec] {}: parameter `{}` is used as a record",
                    data.def(d).name(),
                    data.def(d).attributes()[idx].name
                );
            }
            if let Some(ps) = params.get_mut(&d) {
                ps.remove(&idx);
            }
        }
        close_fnref_groups(data, &sites, &mut cand, &mut params, trace);
        params.retain(|_, ps| !ps.is_empty());
        let after = (
            cand.len(),
            params.values().map(HashMap::len).sum::<usize>(),
            presence_keys(&cand, &params),
        );
        if after == before {
            break;
        }
    }
    if trace {
        for (d, ps) in &params {
            for idx in ps.keys() {
                crate::loft_eprintln!(
                    "[valuerec] {}: parameter `{}` is carried as a tuple",
                    data.def(*d).name(),
                    data.def(*d).attributes()[*idx].name
                );
            }
        }
    }
    clock.add("fixpoint_tail");
    out.sp = surely_present_fns(data, &cand, &params, &sites);
    clock.add("final_presence");
    clock.report(data.definitions.len());
    out.fns = cand;
    out.params = params;
    out.fnref_sites = sites;
    views.retain(|d, _| out.fns.contains_key(d));
    out.views = views;
    out.view_offs = view_offs;
    out.view_offs.retain(|_, offs| !offs.is_empty());
    out
}

/// `@FR-N-Store` — CARRY THE PRESENCE where a dense tuple would otherwise be widened without
/// it.  A dense tuple has no absent spelling (@C144), so the null a program stored into a
/// dense slot is gone once that slot is read into one; widening such a tuple with the
/// constant word would answer "present" for it.  So what stands where a nullable tuple is
/// expected is made to carry its own presence:
///
/// * a dense TUPLE PARAMETER takes the nullable layout — its call sites read whether the
///   argument names a record, exactly as they do for a `p: S?`;
/// * a dense FUNCTION whose tuple is not surely present ([`surely_present_fns`]) returns the
///   nullable layout, as does one whose own result is such a function's or such a
///   parameter;
/// * a dense parameter handed a tuple that carries the nullable layout takes it too, so the
///   hand-off stays a tuple.
///
/// The positions that expect a nullable tuple are read from the SOURCE types, which the
/// rewrite does not change: a result of a function admitted with the nullable layout, an
/// argument of a nullable tuple parameter, and — for parameters — a bind of a `τ?` local.
/// A local is followed to its binds.  Only ever turns a dense key into a nullable one; answers whether it did.  An arm
/// of a fn-ref dispatch (`frozen`) keeps its ABI, and the gate then declines what it cannot
/// serve.
#[allow(clippy::too_many_arguments)]
pub(crate) fn promote_presence(
    data: &Data,
    cand: &mut HashMap<u32, u16>,
    params: &mut TupleParams,
    types: &mut HashMap<u16, ValueTuple>,
    index: &mut HashMap<(u16, i64), usize>,
    frozen: &HashSet<u32>,
    sp: &HashSet<u32>,
    trace: bool,
) -> bool {
    let mut want_params: HashSet<(u32, usize)> = HashSet::new();
    let mut want_fns: HashSet<u32> = HashSet::new();
    for d in 0..data.definitions.len() as u32 {
        if matches!(data.def(d).code(), Value::Null) {
            continue;
        }
        let w = PresenceWalk {
            data,
            d,
            cand,
            params,
            sp,
        };
        w.scan(&mut want_params, &mut want_fns);
    }
    let mut changed = false;
    for (d, idx) in want_params {
        if frozen.contains(&d) {
            continue;
        }
        let Some(key) = params.get(&d).and_then(|ps| ps.get(&idx)).copied() else {
            continue;
        };
        if is_nullable_layout(key) {
            continue;
        }
        let Some(nkey) = nullable_presence_layout(types, index, key) else {
            continue;
        };
        if let Some(ps) = params.get_mut(&d) {
            ps.insert(idx, nkey);
            changed = true;
            if trace {
                crate::loft_eprintln!(
                    "[valuerec] {}: parameter `{}` carries its presence",
                    data.def(d).name(),
                    data.def(d).attributes()[idx].name
                );
            }
        }
    }
    for d in want_fns {
        if frozen.contains(&d) {
            continue;
        }
        let Some(key) = cand.get(&d).copied() else {
            continue;
        };
        if is_nullable_layout(key) {
            continue;
        }
        let Some(nkey) = nullable_presence_layout(types, index, key) else {
            continue;
        };
        cand.insert(d, nkey);
        changed = true;
        if trace {
            crate::loft_eprintln!(
                "[valuerec] {}: its result carries its presence",
                data.def(d).name()
            );
        }
    }
    changed
}

/// One function's view for [`promote_presence`]: the tables as they stand, read only.
pub(crate) struct PresenceWalk<'a> {
    pub(crate) data: &'a Data,
    pub(crate) d: u32,
    pub(crate) cand: &'a HashMap<u32, u16>,
    pub(crate) params: &'a TupleParams,
    pub(crate) sp: &'a HashSet<u32>,
}

impl PresenceWalk<'_> {
    /// Every position of this function that expects a nullable tuple, walked for what must
    /// carry its presence there ([`PresenceWalk::demand`]), and every position that
    /// expects a dense one and is handed a tuple that carries it
    /// ([`PresenceWalk::carries_presence`]).
    pub(crate) fn scan(
        &self,
        want_params: &mut HashSet<(u32, usize)>,
        want_fns: &mut HashSet<u32>,
    ) {
        let (data, d) = (self.data, self.d);
        let def = data.def(d);
        let body = def.code();
        let vars = def.variables();
        let own = self.cand.get(&d).copied();
        let own_nullable = own.is_some_and(is_nullable_layout);
        if own_nullable {
            self.demand(body, &mut HashSet::new(), want_params, want_fns);
        } else if own.is_some() && self.carries_presence(body, &mut HashSet::new()) {
            want_fns.insert(d);
        }
        body.any_node(&mut |n| {
            match n {
                Value::Return(x) if own_nullable => {
                    self.demand(x, &mut HashSet::new(), want_params, want_fns);
                }
                Value::Return(x)
                    if own.is_some() && self.carries_presence(x, &mut HashSet::new()) =>
                {
                    want_fns.insert(d);
                }
                // A bind of a `τ?` local asks its PARAMETERS only: a function is left as it
                // is, because a local bound from dense tuples alone holds the dense tuple and
                // widens nothing, while a promoted function loses its registers at every
                // other site a nullable tuple does not serve.
                Value::Set(v, rhs) if *v < vars.count() => {
                    let (inner, optional) = vars.tp(*v).peel_optional();
                    if optional && plain_record_type(data, inner).is_some() {
                        self.demand(rhs, &mut HashSet::new(), want_params, &mut HashSet::new());
                    }
                }
                // A whole-record copy FROM a tuple writes its fields into the destination,
                // so the tuple must say whether there is anything to write.
                Value::Call(callee, args)
                    if (*callee as usize) < data.definitions.len()
                        && data.def(*callee).name() == "OpCopyRecord" =>
                {
                    if let Some(src) = args.first() {
                        self.demand(src, &mut HashSet::new(), want_params, want_fns);
                    }
                }
                Value::Call(callee, args) => {
                    for (i, key) in self.params.get(callee).into_iter().flatten() {
                        let Some(arg) = args.get(*i) else { continue };
                        if is_nullable_layout(*key) {
                            self.demand(arg, &mut HashSet::new(), want_params, want_fns);
                        } else if self.carries_presence(arg, &mut HashSet::new()) {
                            want_params.insert((*callee, *i));
                        }
                    }
                }
                _ => {}
            }
            false
        });
    }

    /// The dense tuple-parameter key of variable `v` of this function, when it is one.
    pub(crate) fn param_of(&self, v: u16) -> Option<(usize, u16)> {
        let def = self.data.def(self.d);
        let vars = def.variables();
        if !vars.is_argument(v) {
            return None;
        }
        let ps = self.params.get(&self.d)?;
        ps.iter()
            .find(|(idx, _)| {
                def.attributes()
                    .get(**idx)
                    .is_some_and(|a| vars.var(&a.name) == v)
            })
            .map(|(idx, key)| (*idx, *key))
    }

    /// Every bind of local `v` (the declaration's `null` aside), handed to `each`.
    pub(crate) fn binds(&self, v: u16, each: &mut dyn FnMut(&Value)) {
        self.data.def(self.d).code().any_node(&mut |n| {
            if let Value::Set(w, rhs) = n
                && *w == v
                && !matches!(rhs.unspan(), Value::Null)
            {
                each(rhs);
            }
            false
        });
    }

    /// `node` stands where a nullable tuple is expected: name what must carry its presence
    /// for that — the dense tuple parameters and the dense functions, not surely present,
    /// among its leaves.  The walk is [`value_shape`]'s own, a local followed to its binds.
    pub(crate) fn demand(
        &self,
        node: &Value,
        seen: &mut HashSet<u16>,
        want_params: &mut HashSet<(u32, usize)>,
        want_fns: &mut HashSet<u32>,
    ) {
        match node.unspan() {
            Value::Block(bl) if bl.name == "Object" => {}
            Value::Block(bl) if mv_return_source(bl, self.data).is_some() => {
                if let Some(src) = mv_return_source(bl, self.data) {
                    self.demand(src, seen, want_params, want_fns);
                }
            }
            Value::Block(bl) if !matches!(bl.result.base(), Type::Void) => {
                if let Some(last) = bl.operators.last() {
                    self.demand(last, seen, want_params, want_fns);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last() {
                    self.demand(last, seen, want_params, want_fns);
                }
            }
            Value::Return(x) => self.demand(x, seen, want_params, want_fns),
            Value::If(_, a, b) => {
                self.demand(a, seen, want_params, want_fns);
                self.demand(b, seen, want_params, want_fns);
            }
            Value::Var(v) => {
                if !seen.insert(*v) {
                    return;
                }
                if let Some((idx, key)) = self.param_of(*v) {
                    if !is_nullable_layout(key) {
                        want_params.insert((self.d, idx));
                    }
                    return;
                }
                self.binds(*v, &mut |rhs| self.demand(rhs, seen, want_params, want_fns));
            }
            Value::Call(g, _)
                if self.cand.get(g).is_some_and(|k| !is_nullable_layout(*k))
                    && !self.sp.contains(g) =>
            {
                want_fns.insert(*g);
            }
            _ => {}
        }
    }

    /// Does a leaf of `node` carry the NULLABLE layout — a call to a function admitted with
    /// it, a tuple parameter received with it, a local bound from either?  Then what `node`
    /// is handed to, or returned from, must carry it too.
    pub(crate) fn carries_presence(&self, node: &Value, seen: &mut HashSet<u16>) -> bool {
        match node.unspan() {
            Value::Block(bl) if bl.name == "Object" => false,
            Value::Block(bl) if mv_return_source(bl, self.data).is_some() => {
                mv_return_source(bl, self.data).is_some_and(|s| self.carries_presence(s, seen))
            }
            Value::Block(bl) if !matches!(bl.result.base(), Type::Void) => bl
                .operators
                .last()
                .is_some_and(|l| self.carries_presence(l, seen)),
            Value::Insert(ops) => ops.last().is_some_and(|l| self.carries_presence(l, seen)),
            Value::Return(x) => self.carries_presence(x, seen),
            Value::If(_, a, b) => self.carries_presence(a, seen) || self.carries_presence(b, seen),
            Value::Var(v) => {
                if !seen.insert(*v) {
                    return false;
                }
                if let Some((_, key)) = self.param_of(*v) {
                    return is_nullable_layout(key);
                }
                let mut any = false;
                self.binds(*v, &mut |rhs| any = any || self.carries_presence(rhs, seen));
                any
            }
            // A `null` exit is the absent tuple, which only the nullable layout spells.
            Value::Call(g, args) if args.is_empty() && is_null_record(self.data, *g) => true,
            Value::Call(g, _) => self.cand.get(g).is_some_and(|k| is_nullable_layout(*k)),
            _ => false,
        }
    }
}

/// Is `v` the bare `null` record sentinel, `OpNullRefSentinel()` ([`is_null_record`])?
#[must_use]
pub fn is_null_sentinel(data: &Data, v: &Value) -> bool {
    matches!(v.unspan(), Value::Call(d, args) if args.is_empty() && is_null_record(data, *d))
}

/// The LAYOUT KEY an admitted body's own leaves must carry: its record type, or that type's
/// nullable key for a `-> S?` ([`nullable_layout`]) — which is also what `admitted` holds
/// for it.
pub(crate) fn own_record(c: &ShapeCtx) -> Option<u16> {
    let own = c.own?;
    if let Some(key) = c.admitted.get(&own) {
        return Some(*key);
    }
    let (ret, nullable) = c.data.def(own).returned().peel_optional();
    let tp = plain_record_type(c.data, ret)?;
    if nullable {
        nullable_layout(tp)
    } else {
        Some(tp)
    }
}

/// Is `v` a VALUE SHAPE — every result position a value LEAF — and if so, which record
/// TYPE's tuple does it carry?  The leaves: a call to an admitted function (forwards
/// its tuple), a value local (already a tuple), and — inside an admitted body only — an
/// `Object` build of the body's own record (the tuple of its writes) or a borrowed VIEW of
/// that record (a tuple of its field reads; a view is never freed, so reading it is all the
/// value form owes).  `Block`, `If`, `Insert` and `Return` carry the question to their
/// result positions.
///
/// The fallback is `None` because every shape not named here delivers a RECORD — a bare
/// call to a buffer-returning function, an OWNED local (whose store the record form hands
/// up and the value form would have to mint per call, which is slower than the buffer it
/// replaces: `own`'s promoted parameter), a null, a copy — and a caller that reads a tuple
/// off a record cannot compile.
pub(crate) fn value_shape(node: &Value, ctx: &ShapeCtx) -> Option<u16> {
    match node.unspan() {
        // `@FR-R-ValueRecord`'s nullable clause — a `null` exit of an admitted `-> S?` body
        // is the ABSENT tuple ([`ValueTuple::absent`]).  Only there: a null anywhere else has
        // no layout to be absent in.
        Value::Call(callee, args) if args.is_empty() && is_null_record(ctx.data, *callee) => {
            ctx.own?;
            own_record(ctx).filter(|k| is_nullable_layout(*k))
        }
        Value::Call(callee, _) => ctx.admitted.get(callee).copied(),
        // The second spelling of a call: through a fn-ref, whose dispatch answers the tuple
        // when every arm is admitted (`(R-FnRefValue)`).
        Value::CallRef(v, _) => fnref_value(ctx.fnref, ctx.admitted, ctx.def_nr, *v),
        // The build of the body's own record — present, under a nullable key.
        Value::Block(bl) if bl.name == "Object" => {
            ctx.own?;
            let rec = own_record(ctx)?;
            (plain_record_type(ctx.data, &bl.result) == Some(layout_record(rec))).then_some(rec)
        }
        // A discharged view returned through the buffer: the leaf is the copy's source,
        // which carries the body's layout or widens to it ([`fits`]).
        // As a VALUE the block is that leaf only when the buffer it fills is the body's own
        // return buffer, the one mint the value form has no use for.
        Value::Block(bl) if mv_return_source(bl, ctx.data).is_some() => {
            ctx.own?;
            let src = mv_return_source(bl, ctx.data)?;
            if let Some((rb, false)) = mv_block_buffer(bl)
                && own_retbuf(ctx.data, ctx.own) != Some(rb)
            {
                return None;
            }
            let rec = own_record(ctx)?;
            fits(src, ctx, rec).then_some(rec)
        }
        Value::Block(bl) => {
            if matches!(bl.result.base(), Type::Void) {
                return None;
            }
            value_shape(bl.operators.last()?, ctx)
        }
        // The arms JOIN ([`join_layout`]): one layout, or a dense arm beside a nullable one
        // of the same record, which is the nullable layout once the dense arm is widened —
        // where it can be without inventing its presence ([`widens`]).
        Value::If(_, then_v, else_v) => {
            let (a, b) = (value_shape(then_v, ctx)?, value_shape(else_v, ctx)?);
            let key = join_layout(a, b)?;
            let dense = [(a, then_v), (b, else_v)]
                .into_iter()
                .filter(|(k, _)| *k != key)
                .all(|(_, arm)| widens(arm, ctx));
            dense.then_some(key)
        }
        Value::Insert(ops) => value_shape(ops.last()?, ctx),
        Value::Return(x) => value_shape(x, ctx),
        Value::Var(var) => {
            if let Some(tp) = ctx.locals.get(var) {
                return Some(*tp);
            }
            ctx.own?;
            let rec = own_record(ctx)?;
            let vars = ctx.data.def(ctx.def_nr).variables();
            // A compiler `__lift_` temp OWNS what it is bound to (`scopes::new_lift_var`):
            // its whole-record bind from a view MINTS a store and copies into it, and the
            // record form hands that store UP as the result (the join local it feeds is a
            // view of it, so nothing in the frame frees it).  The oracle reads the
            // un-minted bind as Borrowed of its source — the one answer a leaf must not
            // rest on: read as a view, the store the lowering mints is nobody's, one
            // leaked record per call (t15).  A lift is a leaf only as a VALUE LOCAL, above.
            if vars.name(*var).starts_with("__lift_") {
                return None;
            }
            let tp = vars.tp(*var);
            // The same bind in its other spellings: a whole-record bind into a LOCAL copies
            // (`@FR-B-Copy`), and the bind emitter aliases a borrowed call result only into a
            // `skip_free` destination (`dispatch.rs`, loft#974) — any other local takes the
            // copy arm and OWNS what it holds, whatever its deps say.  `__ret_tail_N =
            // op(a, b, __ref_N)` over an operator that answers its `self` is that bind, and
            // the oracle reads it through the call as Borrowed of `a`: read as a view the copy
            // was freed by nobody, one record per call on `--native` alone, in a concrete
            // function and an instance alike (Found-via loft#1820).  A parameter is exempt —
            // the frame never owns it — unless the body REBINDS it: then it may hold a store
            // of its own (`@FR-F-ParamRebind`), which the record form hands to the caller to
            // release and the tuple form hands to nobody (`fn f(p: S, q: S) -> S { p = q; p }`,
            // one record per call, loft#1871).
            if !vars.is_argument(*var) && !vars.is_skip_free(*var)
                || vars.rebind_orig(*var).is_some()
            {
                return None;
            }
            // `@FR-N-Road` — the view carries the layout its own TYPE names: a view of `S` the
            // dense one in a `-> S` and a `-> S?` body alike, a view of `S?` the nullable one.
            // A nullable view is a leaf of a `-> S?` body only, where that layout exists.
            let (inner, optional) = tp.peel_optional();
            let view = plain_record_type(ctx.data, inner).filter(|t| *t == layout_record(rec));
            let key = if optional {
                view.filter(|_| is_nullable_layout(rec)).map(|_| rec)
            } else {
                view
            };
            // A VIEW by the ownership oracle (`@FR-O-Oracle`), not by the dep list: then_v
            // `__ret_N` typed `ref(P)["then_v"]` holds the parameter's store on one arm and then_v
            // minted default on the other (`Own::Join`), and reading it as then_v tuple would
            // leave that mint nobody's.  Only then_v value the frame never owns is read as one.
            key.filter(|_| {
                matches!(
                    crate::use_analysis::ownership_of(ctx.data, ctx.def_nr, node),
                    crate::use_analysis::Own::Borrowed { .. }
                )
            })
        }
        _ => None,
    }
}

/// The layout two value shapes JOIN to — the arms of a value branch, the binds of one local.
/// The same key is itself.  A DENSE key beside the NULLABLE key of the same record is the
/// nullable one (`@FR-N-Road`): the dense side is the present tuple once its leaves are
/// widened ([`widen_marks`]), so a `τ?` result that joins a `τ` with a `τ?` takes the road
/// its all-`τ` twin takes.  Two records have no join.
pub(crate) fn join_layout(a: u16, b: u16) -> Option<u16> {
    if a == b {
        Some(a)
    } else if layout_record(a) == layout_record(b) {
        nullable_layout(layout_record(a))
    } else {
        None
    }
}

/// Does `node` stand for layout `key` — carry it, or carry the DENSE layout of a nullable
/// `key`'s record and widen to it with its presence known ([`widens`])?
pub(crate) fn fits(node: &Value, c: &ShapeCtx, key: u16) -> bool {
    value_shape(node, c)
        .is_some_and(|k| k == key || (join_layout(k, key) == Some(key) && widens(node, c)))
}

/// Can every leaf under `node` that stands where a nullable tuple is expected deliver one
/// without inventing its presence?  A leaf that already carries the nullable layout does, a
/// VIEW does (its presence word is read off its reference), an `Object` build and a `null`
/// do; a DENSE tuple — a `-> S` call, a dense value local, a tuple parameter — only where it
/// is [`surely_present`].  The walk is [`widen_marks`]' own.
pub(crate) fn widens(node: &Value, c: &ShapeCtx) -> bool {
    match node.unspan() {
        Value::Block(bl) if bl.name == "Object" => true,
        Value::Block(bl) if mv_return_source(bl, c.data).is_some() => {
            mv_return_source(bl, c.data).is_none_or(|src| widens(src, c))
        }
        Value::Block(bl) if !matches!(bl.result.base(), Type::Void) => {
            bl.operators.last().is_none_or(|l| widens(l, c))
        }
        Value::Insert(ops) => ops.last().is_none_or(|l| widens(l, c)),
        Value::Return(x) => widens(x, c),
        Value::If(_, a, b) => widens(a, c) && widens(b, c),
        Value::Var(v) => c.locals.get(v).is_none_or(|k| {
            is_nullable_layout(*k) || local_surely_present(*v, c, &mut HashSet::new())
        }),
        Value::Call(d, _) => {
            c.admitted.get(d).is_none_or(|k| is_nullable_layout(*k)) || c.sp.contains(d)
        }
        Value::CallRef(..) => value_shape(node, c).is_none_or(is_nullable_layout),
        _ => true,
    }
}

/// `@FR-N-Store` — the admitted DENSE functions whose tuple is surely present: every result
/// position is a build, a call to another such function, or a local bound only from those
/// ([`surely_present`]).  The GREATEST such set — a recursion through its own result is
/// present when its other exits are — found by removing until nothing leaves.
pub(crate) fn surely_present_fns(
    data: &Data,
    admitted: &HashMap<u32, u16>,
    params: &TupleParams,
    fnref: &FnRefSites,
) -> HashSet<u32> {
    let mut sp: HashSet<u32> = admitted
        .iter()
        .filter(|(_, k)| !is_nullable_layout(**k))
        .map(|(d, _)| *d)
        .collect();
    let no_locals: HashMap<u16, u16> = HashMap::new();
    loop {
        let keep: HashSet<u32> = sp
            .iter()
            .copied()
            .filter(|d| {
                let c = ShapeCtx {
                    data,
                    def_nr: *d,
                    admitted,
                    params,
                    locals: &no_locals,
                    own: Some(*d),
                    fnref,
                    sp: &sp,
                };
                let body = data.def(*d).code();
                let mut present = surely_present(body, &c, &mut HashSet::new());
                // A block that returns the buffer it filled is judged as the block; its own
                // `return` reads that buffer, which is bound by no assignment.
                let mut own_returns: HashSet<usize> = HashSet::new();
                body.any_node(&mut |n| {
                    match n {
                        Value::Block(bl)
                            if object_own_return(bl).is_some()
                                || mv_return_source(bl, data).is_some() =>
                        {
                            present = present && surely_present(n, &c, &mut HashSet::new());
                            if let Some(last) = bl.operators.last() {
                                own_returns.insert(std::ptr::from_ref(last.unspan()) as usize);
                            }
                        }
                        Value::Return(x)
                            if !own_returns.contains(&(std::ptr::from_ref(n) as usize)) =>
                        {
                            present = present && surely_present(x, &c, &mut HashSet::new());
                        }
                        _ => {}
                    }
                    !present
                });
                present
            })
            .collect();
        if keep.len() == sp.len() {
            return sp;
        }
        sp = keep;
    }
}

/// Does every RESULT position of `d_nr`'s body carry a value leaf ([`value_shape`]), and
/// is its return buffer mentioned only where the value form can drop the mention?  The
/// buffer becomes a PHANTOM — the parameter is gone from the signature — so a body may
/// name it only inside a converted `Object` block, as the buffer argument of a call that
/// drops it, or as the subject of a free.  `None` admits; `Some` names the refusing test
/// for the `LOFT_TRACE_VALUEREC` line.
pub(crate) fn value_body(
    data: &Data,
    d_nr: u32,
    admitted: &HashMap<u32, u16>,
    params: &TupleParams,
    view_offs: &ViewOffsets,
    fnref: &FnRefSites,
    sp: &HashSet<u32>,
) -> Option<&'static str> {
    let def = data.def(d_nr);
    let locals = value_locals_in(data, d_nr, admitted, params, view_offs, fnref, sp);
    let c = ShapeCtx {
        data,
        def_nr: d_nr,
        admitted,
        params,
        locals: &locals,
        own: Some(d_nr),
        fnref,
        sp,
    };
    let body = def.code();
    // Every result position carries THIS function's layout, or WIDENS to it ([`fits`]): a
    // dense leaf of a `-> S?` body is the shorter tuple until the emitter appends the presence
    // word ([`widen_marks`]).
    let key = admitted.get(&d_nr).copied();
    if !key.is_some_and(|k| fits(body, &c, k)) {
        return Some("the tail is not a value leaf");
    }
    // An `Object` that returns the record it builds is the leaf; its own `return`, the
    // block's last statement, is not asked again below.
    let mut ok = true;
    let mut own_returns: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && (object_own_return(bl).is_some() || mv_return_source(bl, data).is_some())
        {
            if value_shape(n, &c).is_none_or(|k| Some(k) != key) {
                ok = false;
                return true;
            }
            if let Some(last) = bl.operators.last() {
                own_returns.insert(std::ptr::from_ref(last.unspan()) as usize);
            }
        }
        false
    });
    if !ok {
        return Some("an object's return is not a value leaf");
    }
    body.any_node(&mut |n| {
        if let Value::Return(x) = n
            && !own_returns.contains(&(std::ptr::from_ref(n) as usize))
            && !key.is_some_and(|k| fits(x, &c, k))
        {
            ok = false;
            return true;
        }
        false
    });
    if !ok {
        return Some("a return is not a value leaf");
    }
    // No return buffer: nothing left to account for, admitted.
    let rb = own_retbuf(data, Some(d_nr))?;
    // A phantom that is itself a value local ([`own_retbuf`]) has every mention
    // accounted by [`local_uses_ok`] already.
    if locals.contains_key(&rb) {
        return None;
    }
    let leaves = collect_leaves(body, &locals, true, data);
    (!retbuf_uses_ok(body, rb, &c, &leaves.objects)).then_some("the return buffer is used")
}

/// Every mention of the return buffer `rb` in `v` is one the value form drops: inside a
/// converted `Object` block, as the buffer argument of an admitted callee, or as the
/// subject of a free.  Anything else — a write through it, a read of it, a rebind — needs
/// the parameter the value form no longer has.
pub(crate) fn retbuf_uses_ok(v: &Value, rb: u16, c: &ShapeCtx, objects: &HashSet<usize>) -> bool {
    match v.unspan() {
        Value::Var(w) => *w != rb,
        Value::Set(w, _) if *w == rb => false,
        Value::Block(bl) if objects.contains(&(std::ptr::from_ref(&**bl) as usize)) => true,
        // A discharged-view return: its mint, copy and `return` of the buffer go with the
        // block; only the copy's SOURCE and the statements kept beside it are asked.
        Value::Block(bl) if mv_return_source(bl, c.data).is_some() => {
            let copy_nr = c.data.def_nr("OpCopyRecord");
            let db_nr = c.data.def_nr("OpDatabase");
            bl.operators.iter().all(|op| match op.unspan() {
                Value::Call(d, args) if *d == copy_nr => retbuf_uses_ok(&args[0], rb, c, objects),
                Value::Call(d, args)
                    if *d == db_nr && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == rb) =>
                {
                    true
                }
                Value::Return(x) if matches!(x.unspan(), Value::Var(w) if *w == rb) => true,
                // The block as a VALUE ends in the buffer it filled ([`mv_block_buffer`]).
                Value::Var(w) if *w == rb => true,
                other => retbuf_uses_ok(other, rb, c, objects),
            })
        }
        Value::Call(d, args) => {
            let callee = c.data.def(*d);
            let is_free = matches!(callee.name(), "OpFreeRef" | "OpFreeRefIfDistinct");
            // As the WITNESS of a store-identity test the phantom is in no store, so the
            // emitter answers the test `true` and makes the guarded free unconditional
            // (`OpFreeRefIfDistinctEmitter`, `OpDistinctStoreEmitter`).
            let witnessed = matches!(callee.name(), "OpFreeRefIfDistinct" | "OpDistinctStore");
            // `@FR-O-Buffer` — the promoted buffer's ENTRY WITNESS snapshots it
            // (`OpRefAlias`) and every free of it is guarded by `OpDistinctStore(buffer,
            // witness)`.  A phantom has no store to snapshot: the alias emits the null
            // reference and the test answers `true` (`OpRefAliasEmitter`,
            // `OpDistinctStoreEmitter`), so both are accounted for.
            let snapshot = matches!(callee.name(), "OpRefAlias" | "OpDistinctStore");
            let dropped = if c.admitted.contains_key(d) {
                ret_buffer_attr(callee)
            } else {
                None
            };
            args.iter().enumerate().all(|(i, a)| {
                if matches!(a.unspan(), Value::Var(w) if *w == rb) {
                    (is_free && i == 0)
                        || (witnessed && i == 1)
                        || (snapshot && i == 0)
                        || dropped == Some(i)
                } else {
                    retbuf_uses_ok(a, rb, c, objects)
                }
            })
        }
        other => {
            let mut ok = true;
            other.for_each_child(&mut |ch| {
                if ok && !retbuf_uses_ok(ch, rb, c, objects) {
                    ok = false;
                }
            });
            ok
        }
    }
}

/// Per local of `body`, the layout key its binds carry under `c`'s value locals — or `None`
/// once ANY bind is not a value shape or carries another record (the declaration's `null`
/// aside).  The binds JOIN as the arms of a branch do ([`join_layout`]): a local bound from a
/// `-> S` call on one path and a `-> S?` call on another holds the nullable tuple, and the
/// dense bind is widened where it stands.
///
/// `@FR-R-ValueRecord`'s nullable clause (@C144) — a bind of the bare `null` sentinel
/// (`w: S? = null`) is the ABSENT tuple of whatever layout the local's other binds carry, so
/// it casts no vote; it is asked afterwards that the layout be nullable.  A dense tuple has no
/// absent spelling, so such a local keeps the record.  A local bound ONLY from `null` takes
/// the layout of the `-> S?` body it stands in (the parser's `__ret_N = null; return __ret_N`
/// exit) and has none anywhere else.
pub(crate) fn bind_shapes(body: &Value, c: &ShapeCtx) -> HashMap<u16, Option<u16>> {
    let mut shapes: HashMap<u16, Option<u16>> = HashMap::new();
    let mut null_bound: HashSet<u16> = HashSet::new();
    let mut unwidened: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, rhs) = n
            && is_null_sentinel(c.data, rhs)
        {
            null_bound.insert(*v);
        } else if let Value::Set(v, rhs) = n
            && !matches!(rhs.unspan(), Value::Null)
        {
            let s = value_shape(rhs, c);
            if s.is_some_and(|k| !is_nullable_layout(k)) && !widens(rhs, c) {
                unwidened.insert(*v);
            }
            shapes
                .entry(*v)
                .and_modify(|e| *e = e.zip(s).and_then(|(a, b)| join_layout(a, b)))
                .or_insert(s);
        }
        false
    });
    // A dense bind of a local that holds the nullable tuple is widened where it stands, so
    // it must be one that can be ([`widens`]).
    for v in unwidened {
        if let Some(e) = shapes.get_mut(&v)
            && e.is_some_and(is_nullable_layout)
        {
            *e = None;
        }
    }
    let own_null = own_record(c).filter(|k| is_nullable_layout(*k));
    for v in null_bound {
        let e = shapes.entry(v).or_insert(own_null);
        if e.is_some_and(|k| !is_nullable_layout(k)) {
            *e = None;
        }
    }
    shapes
}

/// Which locals of `def_nr` hold a value-returned record — every non-null assignment a
/// value shape, every use one the tuple serves ([`local_uses_ok`]), never a parameter
/// (except the PHANTOM return buffer of an admitted body, [`own_retbuf`], and the TUPLE
/// PARAMETERS of `(R-ValueLocal)`, seeded first) — mapped to the record type whose tuple
/// they carry.  ONE home: the gate decides admission over it and the emitter types the
/// locals from it.
///
/// A lift bound from a bare `Var` — the copy of a parameter's view a selecting arm
/// returns — is a value local through the whole-record copy arm, which turns into the
/// tuple of the view's reads; a lift read as a VIEW leaf instead leaks the store that
/// copy mints (t15).  A fixpoint, because a leaf may name another value local.
#[must_use]
pub fn value_locals_in(
    data: &Data,
    def_nr: u32,
    admitted: &HashMap<u32, u16>,
    params: &TupleParams,
    view_offs: &ViewOffsets,
    fnref: &FnRefSites,
    sp: &HashSet<u32>,
) -> HashMap<u16, u16> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let own = admitted.contains_key(&def_nr).then_some(def_nr);
    // `(R-ValueLocal)` — a tuple PARAMETER is a value local the caller bound: the body
    // reads it exactly as one, and a leaf may name it.
    let mut locals: HashMap<u16, u16> = tuple_param_vars(data, def_nr, params);
    // A GENERATOR's locals persist as fields of its coroutine struct, typed `DbRef` by the
    // factory; a tuple has no such field, so a generator binds no value local.
    if body.any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return locals;
    }
    let rb = own_retbuf(data, own);
    // Fixed for the body: the reads whose value is dropped.
    let dropped = dropped_reads(body);
    // Every local is eligible, a `__lift_` temp bound from a CALL included (since
    // `(R-ValueLocal)`, 2026-09-25): its bind from an ADMITTED callee takes the plain
    // assignment (the adopt-or-copy arm asks `value_records.fns` first), so nothing reads
    // `.store_nr` off it.  A nested call argument (`vertex(vec3(…), …)`) is exactly such a
    // lift, and a tuple parameter is served only by a lift that is a value local.
    let eligible = |v: u16| !vars.is_argument(v) || Some(v) == rb;
    // Per local, GIVEN a locals set: the tuple its assignments carry, or `None` once ANY
    // assignment is not a value shape (the declaration's `null` aside).
    let empty_locals: HashMap<u16, u16> = HashMap::new();
    let base = ShapeCtx {
        data,
        def_nr,
        admitted,
        params,
        locals: &empty_locals,
        own,
        fnref,
        sp,
    };
    let shapes_given = |locals: &HashMap<u16, u16>| bind_shapes(body, &base.with_locals(locals));
    let joined = |locals: &HashMap<u16, u16>, cands: &HashMap<u16, u16>| -> HashMap<u16, u16> {
        let mut with = locals.clone();
        with.extend(cands.iter().map(|(v, d)| (*v, *d)));
        with
    };
    loop {
        // GROW: this round's candidates, admitted optimistically.  A candidate's shape may
        // rest on ANOTHER candidate — the join local of a selecting branch reads the lift
        // each arm binds, and the lift's read stands at that join's right — so a step over
        // `locals` alone reaches neither.  Every candidate still traces to a source outside
        // the set: the first pass admits from `locals`, views and admitted calls only, and
        // each later pass only from what the pass before found.
        let mut cands: HashMap<u16, u16> = HashMap::new();
        loop {
            let mut grown = cands.clone();
            for (v, s) in shapes_given(&joined(&locals, &cands)) {
                if let Some(d) = s
                    && !locals.contains_key(&v)
                    && eligible(v)
                {
                    grown.insert(v, d);
                }
            }
            if grown.len() == cands.len() {
                break;
            }
            cands = grown;
        }
        // PRUNE to a consistent set: a candidate whose uses the tuple cannot serve, or
        // whose shape rested on a candidate just pruned, leaves — until nothing else does.
        loop {
            let with = joined(&locals, &cands);
            let shapes = shapes_given(&with);
            let leaves = collect_leaves(body, &with, own.is_some(), data);
            let c = base.with_locals(&with);
            let mut served = leaves.reads;
            served.extend(dropped.iter().copied());
            let kept: HashMap<u16, u16> = cands
                .keys()
                .filter_map(|v| {
                    let d = shapes.get(v).copied().flatten()?;
                    let empty = HashSet::new();
                    let offs = view_offs.get(&d).unwrap_or(&empty);
                    let phantom = (Some(*v) == rb).then_some(&leaves.objects);
                    if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
                        crate::loft_eprintln!(
                            "[valuerec]   {} candidate local `{}`:",
                            def.name(),
                            vars.name(*v)
                        );
                    }
                    local_uses_ok(body, *v, &c, &served, offs, phantom, &leaves.objects)
                        .then_some((*v, d))
                })
                .collect();
            let stable = kept.len() == cands.len();
            cands = kept;
            if stable {
                break;
            }
        }
        if cands.is_empty() {
            return locals;
        }
        locals.extend(cands);
    }
}

/// Is every use of local `v` in `body` one a TUPLE can serve?  A scalar field read at a
/// constant offset (the tuple index), a free of it (nothing to release), a store-identity
/// test or a free guarded by one against it (the tuple is in no store, so always
/// distinct), a copy FROM it (the tuple is materialised into the destination), the buffer
/// ARGUMENT of an admitted callee (the argument the call site drops — how the phantom
/// reaches the callee of a `return f(…)` chain), and a whole-value read the tuple SERVES
/// (`served`: one at a VALUE POSITION — a return tail of an admitted body, the right of a
/// value local, the tail of a branch arm either stands at — consumed as the tuple; or one
/// whose value is dropped, [`dropped_reads`]).  For the PHANTOM return buffer (`phantom`
/// carries the body's converted `Object` leaves), every mention inside such a leaf is
/// dropped with the block: a literal exit built into a buffer a chain also binds
/// (@PLN164 B2) writes it there, exactly as [`retbuf_uses_ok`] accounts for a phantom that
/// is no local.  Any other use — an argument, an append, a copy INTO it, a whole-value
/// read anywhere else — needs the record, so the local keeps its buffer.
pub(crate) fn local_uses_ok(
    body: &Value,
    v: u16,
    c: &ShapeCtx,
    served: &HashSet<usize>,
    view_offs: &HashSet<i64>,
    phantom: Option<&HashSet<usize>>,
    value_objects: &HashSet<usize>,
) -> bool {
    let (data, admitted, params) = (c.data, c.admitted, c.params);
    // The layout this local carries, when the walk already holds it as a candidate: a
    // nullable one serves its presence tests and declines every hand-off a DENSE tuple
    // would be expected at (a tuple parameter, a copy into a record).
    let key = c.locals.get(&v).copied();
    let nullable = key.is_some_and(is_nullable_layout);
    let temp_test_unserved = !nullable && temp_presence_unserved(c, v);
    let present = || local_surely_present(v, c, &mut HashSet::new());
    let no_objects = HashSet::new();
    let objects = phantom.unwrap_or(&no_objects);
    // @PLN164 C5 — the view-field reads this local's uses may be accounted against, read
    // once: which of them stand where the field can only be READ.
    let view_reads = view_field_reads(body, v, data, view_offs);
    // Every node inside a converted `Object`, by address — the walk below is transparent
    // to `Span`, and so is this one.
    let mut in_object: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && objects.contains(&(std::ptr::from_ref(&**bl) as usize))
        {
            n.any_node(&mut |m| {
                in_object.insert(std::ptr::from_ref(m) as usize);
                false
            });
        }
        false
    });
    let mut mentions = 0u32;
    let mut accounted = 0u32;
    body.any_node(&mut |n| {
        let inside = in_object.contains(&(std::ptr::from_ref(n) as usize));
        match n {
            Value::Var(w) if *w == v => {
                mentions += 1;
                if inside || served.contains(&(std::ptr::from_ref(n) as usize)) {
                    accounted += 1;
                }
            }
            // Its `Var` operands are accounted above; no rule below may count them twice.
            Value::Call(..) if inside => {}
            // `(R-FnRefValue)` — handed whole to a parameter every arm of the dispatch
            // receives as a tuple: the same hand-off as a direct call's.
            Value::CallRef(f, args) if !inside && !nullable => {
                accounted += fnref_handoffs(c, v, *f, args);
            }
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                let arg_is_v = |i: usize| {
                    matches!(args.get(i).map(Value::unspan), Some(Value::Var(w)) if *w == v)
                };
                if admitted.contains_key(d)
                    && let Some(a) = ret_buffer_attr(data.def(*d))
                    && arg_is_v(a)
                {
                    accounted += 1;
                }
                // `(R-ValueLocal)` — a hand-off to a TUPLE PARAMETER, and a nullable tuple's
                // presence test.
                if !(temp_test_unserved && is_presence_test(name, args, v)) {
                    accounted += handoffs_and_presence(data, params, *d, name, args, v, key, &present);
                }
                // A SCALAR FIELD READ at a constant offset — what the value path turns
                // into a tuple index.  (`OpGetField` is the COLLECTION-field spelling; a
                // record's scalar field reads through its typed getter.)
                if VALUE_RECORD_GETTERS.contains(&name) && field_on(data, args, v).is_some() {
                    accounted += 1;
                }
                // A SCALAR FIELD WRITE at a constant offset is the tuple element's
                // assignment (`cur.h = x` on a local the tuple carries) — never at a
                // view-leaf offset, whose element is a place and not a value.  Only the
                // setters `FusedElementWriteEmitter` owns are admitted, so every accounted
                // write has the emitter arm that spells it.
                if VALUE_RECORD_SETTERS.contains(&name)
                    && field_on(data, args, v).is_some_and(|off| !view_offs.contains(&off))
                {
                    accounted += 1;
                }
                // @PLN164 C5 — a VIEW-LEAF field read that stands in a read-only context
                // ([`view_field_reads`]): the tuple hands over the same reference the
                // record's field slot would, so the read is served.
                if name == "OpGetField" && arg_is_v(0) && view_reads.contains(&(std::ptr::from_ref(n) as usize)) {
                    accounted += 1;
                }
                // A copy FROM the local MATERIALISES the tuple into the destination, one
                // typed write per field — which a VIEW LEAF has no write for: its element
                // is a reference, and materialising it would be a vector copy the value
                // path does not emit.  So a view-leaf record declines a copy site
                // (@PLN164 C5); the record form stands there.
                // A NULLABLE tuple materialises under its presence word — an absent one
                // writes nothing, which is what a copy from the null record does — and a
                // DENSE one only where it is surely present (`@FR-N-Store`): a tuple that
                // may have been read off a null record would write that record's field
                // nulls where the record form copies nothing.
                let copies = name == "OpCopyRecord"
                    && (!view_offs.is_empty()
                        || (!nullable
                            && arg_is_v(0)
                            && !present()));
                // The copy inside a discharged-view return is dropped with the block and
                // its source stands as a LEAF read, accounted above as served — so that
                // one mention is not counted here a second time (a bare mint beside it
                // would then have passed unaccounted).
                let src_served = name == "OpCopyRecord"
                    && args.first().is_some_and(|a| {
                        served.contains(&(std::ptr::from_ref(a) as usize))
                            || served.contains(&(std::ptr::from_ref(a.unspan()) as usize))
                    });
                // loft#2014 — a store-identity test whose OTHER operand is a value local, or
                // the work-ref of a build the value form converts, releases nothing on either
                // path: neither value is in a store, so the guard is moot rather than
                // path-dependent, and the test is served.
                // Never for a record that owns a DROP hook: its release is observable
                // (`(H-Drop)`), and the guard is what times it — the drop gate's
                // `q_default_param_var_ret` ran the hook before a read when it was moot.
                let droppable = |w: u16| {
                    data.type_owns_droppable_anywhere(data.def(c.def_nr).variables().tp(w))
                };
                let moot = |w: u16| {
                    !droppable(w)
                        && (c.locals.contains_key(&w) || converted_build_ref(body, value_objects, w))
                };
                accounted += release_uses(body, name, args, v, copies || src_served, &moot);
                // A copy FROM a sub-record of the local (`Vtx { pos: w.pos, … }`, a push of
                // one): the tuple's range is materialised into the destination.  A bind
                // that VIEWS the sub-record (`q = w.pos`) is no copy and is not here.
                if name == "OpCopyRecord"
                    && !copies
                    && sub_record_of(data, args.first(), v).is_some_and(|sub| copy_type(args.get(2)) == Some(sub))
                {
                    accounted += 1;
                }
                // `@FR-O-Buffer` — the phantom buffer's entry witness snapshots it; a phantom
                // is in no store, so the snapshot is the null reference (`OpRefAliasEmitter`).
                if phantom.is_some() && arg_is_v(0) && name == "OpRefAlias" {
                    accounted += 1;
                }
            }
            _ => {}
        }
        false
    });
    if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
        crate::loft_eprintln!("[valuerec]     {mentions} mentions, {accounted} accounted");
    }
    mentions == accounted
}

/// `LOFT_TIMING` — what each step of [`value_records`] cost, summed over the fixpoint's
/// rounds.  Inert unless `on`.
struct Clock {
    last: Option<std::time::Instant>,
    steps: Vec<(&'static str, f64)>,
    rounds: u32,
}

impl Clock {
    fn new(on: bool) -> Self {
        Clock {
            last: on.then(std::time::Instant::now),
            steps: Vec::new(),
            rounds: 0,
        }
    }

    fn add(&mut self, step: &'static str) {
        if let Some(last) = self.last {
            let now = std::time::Instant::now();
            let ms = now.duration_since(last).as_secs_f64() * 1000.0;
            match self.steps.iter_mut().find(|(s, _)| *s == step) {
                Some(entry) => entry.1 += ms,
                None => self.steps.push((step, ms)),
            }
            self.last = Some(now);
        }
    }

    fn report(&self, definitions: usize) {
        if self.last.is_some() {
            let mut line = format!(
                "LOFT_TIMING value_records defs={definitions} rounds={}",
                self.rounds
            );
            for (step, ms) in &self.steps {
                let _ = std::fmt::Write::write_fmt(&mut line, format_args!(" {step}={ms:.2}ms"));
            }
            eprintln!("{line}");
        }
    }
}
