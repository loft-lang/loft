// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! The PLACE a value reads: the container variable and the field offset a view or an argument
//! names, and whether two places name the same storage.  The view walk, the disturbance analysis
//! and the reshape refusals all ask their questions in these terms.

use crate::data::{Data, Value};
use crate::variables::Function;

/// The PLACE an element/field read ultimately reads out of — the one home is
/// [`crate::use_analysis::projection_container_place`], which carries both the variant-check
/// peel a struct-enum payload projection needs and the field OFFSET that tells `w.a` from
/// `w.b`.  This wrapper exists so the view walk reads it by the name the walk talks about.
pub(super) fn base_container_place(value: &Value, data: &Data) -> Option<(u16, u32)> {
    crate::use_analysis::projection_container_place(data, value)
}

/// The offset of a place that is a whole VARIABLE rather than one of its fields — and the
/// wildcard on both sides of a match, since disturbing a variable ends every place in it.
use crate::use_analysis::ANY_FIELD;

/// The LITERAL key arguments of an `OpGetRecord`, or `None` when any of them is computed.
///
/// `OpGetRecord(coll, type, nkeys, k1, …)` is the one lowering a keyed POINT lookup takes, on
/// every keyed kind, so both sides of the question below read their key through this: the VIEW
/// (`c = &h[30]` binds one) and the REMOVAL (`h[10] = null` carries one inside the
/// `OpHashRemove`'s own argument).  One reader, so the two cannot disagree about what a key is.
///
/// `None` is the CONSERVATIVE answer and is returned for anything that is not a bare literal —
/// a variable, an expression, a call.  Nothing downstream may read `None` as "different keys".
pub(super) fn get_record_literal_keys(value: &Value, data: &Data) -> Option<Vec<Value>> {
    let Value::Call(d, args) = value.unspan() else {
        return None;
    };
    if data.def(*d).name() != "OpGetRecord" {
        return None;
    }
    let n = match args.get(2).map(Value::unspan) {
        Some(Value::Int(n)) if *n >= 0 => *n as usize,
        _ => return None,
    };
    let mut keys = Vec::with_capacity(n);
    for k in args.iter().skip(3).take(n) {
        match k.unspan() {
            v @ (Value::Int(_) | Value::Text(_)) => keys.push(v.clone()),
            _ => return None,
        }
    }
    (keys.len() == n && n > 0).then_some(keys)
}

/// Do a view's place and a disturbance's place name the same storage?  Equal offsets, or
/// either side naming the whole variable.
pub fn same_place(view: (u16, u32), disturbed: (u16, u32)) -> bool {
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
pub(crate) fn value_view_places(
    value: &Value,
    data: &Data,
    function: &Function,
) -> Vec<(u16, u32)> {
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

/// Is `v` a VIEW of a keyed collection that the materialise can copy with `OpReplaceKeyed`?
/// Two kinds: a `match`/`is` payload binding of a keyed field (`(B-View)`, the parser marks
/// it never-free as one), and a plain bind of a keyed projection the parser recorded as a
/// view (`Function::keyed_views`, `(B-View-Depth)`, loft#1759).  A one-level projection off an
/// owned base copies at the bind (`(B-Copy)`) and is neither.  Widening the view walk's type
/// list alone was measured unsound — the naming and the copy have to land together — which
/// is why both kinds are admitted through this one test, read by the walk and the copy alike.
pub(super) fn keyed_payload_view(function: &Function, v: u16) -> bool {
    crate::parser::vectors::is_keyed(function.tp(v))
        && (function.is_overwritten_view(v) || function.keyed_views.contains(&v))
}

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

/// See through the `OpCreateStack` wrapper an argument passed to a `&` parameter carries.
///
/// A bare `Var` is returned unchanged, so a lowering change that stops wrapping cannot
/// silently lose the fact this is asked for.
pub(super) fn peel_stack_ref<'a>(arg: &'a Value, data: &Data) -> &'a Value {
    let inner = arg.unspan();
    if let Value::Call(cs, cargs) = inner
        && data.def(*cs).name() == "OpCreateStack"
        && let Some(first) = cargs.first()
    {
        return first.unspan();
    }
    inner
}

/// A container place a definition disturbs through one of its PARAMETERS: the parameter's
/// SLOT, and the field offset inside it (`ANY_FIELD` for the parameter itself).
///
/// The same `(var, field)` shape every other place in this file carries, read in the callee's
/// own numbering — argument slots lead the variable numbering, so the slot indexes both the
/// attribute list and the argument list at a call site.
pub type ParamPlace = (u16, u32);

/// Compose a place the CALLER handed down with a place the CALLEE disturbed inside it.
///
/// `base` is where the argument came from in the caller (`f(sc)` gives `(sc, ANY_FIELD)`,
/// `f(sc.els)` gives `(sc, off_els)`); `inner` is the field offset the callee disturbed
/// inside its parameter (`p.els += […]` gives `off_els`, `p += […]` on a `&vector` gives
/// `ANY_FIELD`). One of the two must be the whole thing, because the place model carries ONE
/// field offset and a projection of a projection needs two.
///
/// `None` is the lower bound and the safe direction: a missed disturbance costs a materialise,
/// a spurious one costs a program its meaning (the measurement [`grown_containers`] records).
/// Widening `f(o.inner)` + `p.els` to `(o, off_inner)` would shake every view rooted at
/// `o.inner`, siblings of `els` included, which is exactly that mistake.
pub fn compose_param_place(base: (u16, u32), inner: u32) -> Option<ParamPlace> {
    match (base.1, inner) {
        (_, ANY_FIELD) => Some(base),
        (ANY_FIELD, off) => Some((base.0, off)),
        _ => None,
    }
}

/// The place a call ARGUMENT names in the frame that writes the call.
///
/// A bare variable names the whole of itself; a projection (`f(sc.els)`) names the field it
/// reads, through the same [`base_container_place`] the VIEW side uses — so the two cannot
/// disagree about what a place is. Anything else answers `None` and disturbs nothing, which is
/// the lower bound every other producer in this file keeps.
///
/// A projection passed to a `&` parameter arrives WRAPPED: the argument is an `Insert` holding
/// the temp's own `Set` and then `OpCreateStack(temp)`, so the place is one indirection away
/// and the block carries the binding that resolves it. Read here rather than from walk state
/// because the binding travels with the argument — there is no ordering to get wrong.
pub fn call_arg_place(arg: &Value, data: &Data) -> Option<ParamPlace> {
    if let Value::Insert(ops) = arg.unspan() {
        let place = call_arg_place(ops.last()?, data)?;
        if place.1 != ANY_FIELD {
            return Some(place);
        }
        // The temp names a place only where this block is what bound it; otherwise the temp
        // itself is the place, which is what a caller-local container passed down is.
        for op in ops {
            if let Value::Set(t, rhs) = op.unspan()
                && *t == place.0
            {
                return base_container_place(rhs, data);
            }
        }
        return Some(place);
    }
    match peel_stack_ref(arg, data) {
        Value::Var(c) => Some((*c, ANY_FIELD)),
        other => base_container_place(other, data),
    }
}
