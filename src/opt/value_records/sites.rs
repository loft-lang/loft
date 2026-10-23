// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! The value-record SITES: each call, forward and fn-ref hand-off where a value tuple
//! would cross, the places a view leaf reads there, and whether every naming of a result
//! buffer is accounted for.  A site no rule here resolves declines its function.

use crate::data::{Block, Data, Type, Value};
use crate::database::Stores;
use crate::opt::attrs::ret_buffer_attr;
use crate::opt::loop_records::{IN_PLACE_SET_OPS, SCALAR_GETTERS};
use crate::opt::value_records::layout::{
    FnRefSites, TupleParams, ViewOffsets, ViewPlan, fnref_value, is_nullable_layout, layout_record,
};
use crate::opt::writes::plain_record_type;
use std::collections::{HashMap, HashSet};

/// `(R-FnRefValue)` — take every arm of a group out of `cand` once one arm of it is out, or
/// admitted with another record, and a parameter index out of every arm's tuple parameters
/// once one arm does not take it as a tuple of the same record: the dispatch passes and
/// answers one ABI for all its arms.  Repeated, because an arm can sit in two groups; it
/// only removes, so it ends.
pub(crate) fn close_fnref_groups(
    data: &Data,
    sites: &FnRefSites,
    cand: &mut HashMap<u32, u16>,
    params: &mut TupleParams,
    trace: bool,
) {
    loop {
        let mut out: Vec<u32> = Vec::new();
        let mut out_params: Vec<(u32, usize)> = Vec::new();
        for (arms, rec) in sites.values() {
            // A dispatch that answers no plain record keeps every arm's record return.
            if rec.is_none_or(|r| arms.iter().any(|a| cand.get(a) != Some(&r))) {
                out.extend(arms.iter().filter(|a| cand.contains_key(a)));
            }
            let idxs: HashSet<usize> = arms
                .iter()
                .filter_map(|a| params.get(a))
                .flat_map(|ps| ps.keys().copied())
                .collect();
            for idx in idxs {
                let first = params.get(&arms[0]).and_then(|ps| ps.get(&idx)).copied();
                if first.is_none()
                    || arms
                        .iter()
                        .any(|a| params.get(a).and_then(|ps| ps.get(&idx)).copied() != first)
                {
                    out_params.extend(
                        arms.iter()
                            .filter(|a| params.get(a).is_some_and(|ps| ps.contains_key(&idx)))
                            .map(|a| (*a, idx)),
                    );
                }
            }
        }
        if out.is_empty() && out_params.is_empty() {
            return;
        }
        for d in out {
            if cand.remove(&d).is_some() && trace {
                crate::loft_eprintln!(
                    "[valuerec] {}: another arm of its fn-ref dispatch keeps the record",
                    data.def(d).name()
                );
            }
        }
        for (d, idx) in out_params {
            if let Some(ps) = params.get_mut(&d)
                && ps.remove(&idx).is_some()
                && trace
            {
                crate::loft_eprintln!(
                    "[valuerec] {}: another arm of its fn-ref dispatch keeps parameter {idx} a record",
                    data.def(d).name()
                );
            }
        }
    }
}

/// `(R-FnRefValue)` — the record type argument `idx` of a call through fn-ref variable `v`
/// of function `def_nr` is passed as a tuple of, when every arm of the dispatch receives
/// that parameter as one.  `None` passes the record, as before.
#[must_use]
pub fn fnref_param(
    sites: &FnRefSites,
    params: &TupleParams,
    def_nr: u32,
    v: u16,
    idx: usize,
) -> Option<u16> {
    let (arms, _) = sites.get(&(def_nr, v))?;
    let first = params.get(arms.first()?)?.get(&idx).copied()?;
    arms.iter()
        .all(|a| params.get(a).and_then(|ps| ps.get(&idx)) == Some(&first))
        .then_some(first)
}

/// Default-ON since 2026-09-17, with the view leaf; `LOFT_NO_FORWARD_TUPLE=1` restores the
/// decline — the first bisect step for a wrong field, a leak or a null-store panic at a
/// `return g(…)` of a record-returning function (read once, generation time, `--native` only).
pub(crate) fn forward_tuple_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_FORWARD_TUPLE").is_ok_and(|v| v != "0"))
}

/// @PLN164 E-1 — is `target = rhs` a FORWARD: a call to an admitted function `g` that is
/// handed `target` as its return buffer and whose answer is bound back into `target`?  That is
/// `g`'s record form writing the caller's buffer, and it is the one record-consuming position
/// the value form can serve without declining `g`: the site writes the tuple into `target`
/// exactly as `g`'s own exit would have — minting the buffer where it is null, setting every
/// field, copying a view part's vector — and the call's value is that buffer again.
///
/// It answers `g`.  `None` for everything else, which keeps the ordinary site rules: a
/// `target` that is a value local (that bind takes the tuple itself), a buffer argument that
/// is not `target`, a record type that differs, a nullable buffer.  The parser writes this
/// shape for every `return g(…)` of a function that keeps its record (`one_buffer_chain`).
#[must_use]
pub fn forward_site(
    data: &Data,
    def_nr: u32,
    target: u16,
    rhs: &Value,
    admitted: &HashMap<u32, u16>,
    locals: &HashMap<u16, u16>,
) -> Option<u32> {
    if !forward_tuple_on() || locals.contains_key(&target) {
        return None;
    }
    let Value::Call(g, args) = rhs.unspan() else {
        return None;
    };
    if !admitted.contains_key(g) {
        return None;
    }
    let callee = data.def(*g);
    let idx = ret_buffer_attr(callee)?;
    if !matches!(args.get(idx).map(Value::unspan), Some(Value::Var(b)) if *b == target) {
        return None;
    }
    let want = plain_record_type(data, callee.returned())?;
    let vars = data.def(def_nr).variables();
    (plain_record_type(data, vars.tp(target)) == Some(want)).then_some(*g)
}

/// Default-ON since 2026-09-17, once the native corpus held with it armed (1372 scripts, 69
/// docs, 36 feature examples, no failure); `LOFT_NO_VIEW_FIELD=1` restores the record form —
/// the first bisect step for a wrong or stale vector read out of a record-returning call (read
/// once, generation time, `--native` only — the interpreter keeps the record form and is the
/// values oracle, as § V-aa's value record already does).
pub(crate) fn view_field_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_VIEW_FIELD").is_ok_and(|v| v != "0"))
}

/// The ops a view leaf may stand under as an OPERAND: each answers a VALUE — a length, a
/// null test — rather than a place inside the vector, so nothing the site does with the
/// result can reach the container.  An op that answers a PLACE (an element read) is not one,
/// however read-only it is itself.
pub(crate) const VIEW_LEAF_READ_OPS: [&str; 3] =
    ["OpLengthVector", "OpRefIsNull", "OpConvBoolFromRef"];

/// Is every mention of `buffer` in this exit one the VALUE form accounts for?
///
/// The value form emits the tuple and drops the block, so the question is not what the block
/// does but what the tuple would lose: the allocate-or-reuse guard and the scalar `OpSet*`
/// writes become tuple elements, the one recognised append (`leaf`) becomes the leaf's place,
/// the trailing yield becomes the tuple itself, and the frees the exit owes keep running — a
/// free OF the buffer is nothing once there is no store, and a store-identity test against it
/// is always distinct, which is what makes the free it guards unconditional
/// (`@FR-R-ValueRecord`).  A mention outside those — an element pushed into a field, a record
/// appended through the buffer, a call handed it — is a fill or an effect the tuple cannot
/// carry, so the function keeps its record.
///
/// This is the INSIDE of an `Object` exit; [`retbuf_uses_ok`] asks the same question over the
/// rest of the body and exempts these blocks wholesale.  The two carry the same list of
/// accounted positions on purpose: they are one question over two scopes, and a position
/// added to one belongs in the other.
///
/// The fallback is `false`, and it is a claim worth stating: an unrecognised mention of the
/// buffer is assumed to WRITE something the tuple would not, which costs the rewrite where the
/// mention was harmless and never a value.
pub(crate) fn buffer_uses_accounted(
    data: &Data,
    bl: &Block,
    buffer: u16,
    leaf: Option<&Value>,
) -> bool {
    /// Is this node one the value form accounts for, whose OPERANDS still have to be read?
    fn accounted_head(data: &Data, n: &Value, buffer: u16) -> bool {
        let Value::Call(d, args) = n else {
            return false;
        };
        if (*d as usize) >= data.definitions.len() {
            return false;
        }
        if !matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == buffer) {
            return false;
        }
        let name = data.def(*d).name();
        name.starts_with("OpSet")
            || matches!(
                name,
                "OpDatabase"
                    | "OpDatabaseNP"
                    | "OpRefIsNull"
                    | "OpConvBoolFromRef"
                    | "OpFreeRef"
                    | "OpFreeRefIfDistinct"
                    | "OpFreeRefTag"
            )
    }

    /// The buffer as the WITNESS of a store-identity test (argument 1): the tuple is in no
    /// store, so the test answers `true` and the free it guards runs unconditionally.
    fn accounted_witness(data: &Data, n: &Value, buffer: u16) -> bool {
        let Value::Call(d, args) = n else {
            return false;
        };
        (*d as usize) < data.definitions.len()
            && matches!(
                data.def(*d).name(),
                "OpFreeRefIfDistinct" | "OpDistinctStore"
            )
            && matches!(args.get(1).map(Value::unspan), Some(Value::Var(w)) if *w == buffer)
    }

    fn walk(data: &Data, n: &Value, buffer: u16, leaf_addr: Option<usize>, ok: &mut bool) {
        let node = n.unspan();
        if Some(std::ptr::from_ref(node) as usize) == leaf_addr {
            return;
        }
        if matches!(node, Value::Var(w) if *w == buffer) {
            *ok = false;
            return;
        }
        if accounted_head(data, node, buffer) {
            if let Value::Call(_, args) = node {
                for a in args.iter().skip(1) {
                    walk(data, a, buffer, leaf_addr, ok);
                }
            }
            return;
        }
        if accounted_witness(data, node, buffer) {
            if let Value::Call(_, args) = node {
                for (i, a) in args.iter().enumerate() {
                    if i != 1 {
                        walk(data, a, buffer, leaf_addr, ok);
                    }
                }
            }
            return;
        }
        node.for_each_child(&mut |c| walk(data, c, buffer, leaf_addr, ok));
    }

    let leaf_addr = leaf.map(|v| std::ptr::from_ref(v.unspan()) as usize);
    let mut ok = true;
    for (i, op) in bl.operators.iter().enumerate() {
        // The block's last statement yields the buffer, and an explicit `return buf` is the
        // same yield: the tuple replaces both.
        let yields = matches!(op.unspan(), Value::Var(w) if *w == buffer)
            || matches!(op.unspan(), Value::Return(x) if matches!(x.unspan(), Value::Var(w) if *w == buffer));
        if yields && i + 1 == bl.operators.len() {
            continue;
        }
        walk(data, op, buffer, leaf_addr, &mut ok);
        if !ok {
            return false;
        }
    }
    ok
}

/// Every `src` an `OpAppendVector` in `op` copies into `buffer`'s field at `off` — the
/// record form's deep copy of a heap field, which is what a view leaf replaces.
pub(crate) fn appends_into<'a>(
    data: &Data,
    op: &'a Value,
    buffer: u16,
    off: i64,
) -> Vec<(&'a Value, &'a Value)> {
    let mut out = Vec::new();
    op.any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpAppendVector"
            && let [dst, src, ..] = &args[..]
            && let Value::Call(g, gargs) = dst.unspan()
            && (*g as usize) < data.definitions.len()
            && data.def(*g).name() == "OpGetField"
            && matches!(gargs.first().map(Value::unspan), Some(Value::Var(w)) if *w == buffer)
            && matches!(gargs.get(1).map(Value::unspan), Some(Value::Int(o)) if i64::from(*o) == off)
        {
            out.push((n, src));
        }
        false
    });
    out
}

/// Does every naming of `root` in this statement leave the watched place alone?  The question
/// a mention rule cannot answer on its own, asked per naming SITE.
///
/// Four shapes name a container, and each says what it reaches:
///
/// * `OpPlaceRecord(root, tp)` — a record claimed in the store, which moves nothing: a
///   `DbRef` is logical, so growing the store's arena leaves every record where it is.  This
///   is the naming @PLN164 B2 puts in every function that places a call's result.
/// * `OpNewRecord(root, tp, fld)` and its `OpFinishRecord(root, elem, tp, fld)` half — an
///   element append, naming its field by NUMBER, which the schema turns into the byte offset a
///   place carries (the same conversion `scopes::grown_containers` makes, and for the same
///   reason).  A sibling field is harmless: claiming a new array for `sc.unparsed` moves
///   nothing in `sc.ops`.
/// * `OpGetField(root, off, _)` — a projection, whose offset IS the place.
/// * an ARGUMENT of a user call, licensed when the call's own disturbance summary
///   (`disturbed`, closed over the call graph) does not reach the watched place.
///
/// Anything else that names the root — a bare `Var` somewhere else, a `Set`, a hand-off this
/// walk was not told about — answers `false`.  That is the rule's direction: a naming whose
/// reach cannot be resolved is treated as reaching the watched place, because a missed
/// disturbance here is a read through an element that moved.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(crate) fn namings_avoid_place(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    stmt: &Value,
    watch: crate::scopes::ParamPlace,
    disturbed: Option<&crate::scopes::DisturbedParams>,
) -> bool {
    let (root, watch_off) = watch;
    let vars = data.def(def_nr).variables();
    let parent = data.type_def_nr(vars.tp(root).base());
    let field_off = |fld: i32| -> Option<u32> {
        if fld == i32::from(u16::MAX) {
            return None;
        }
        let fld = u16::try_from(fld).ok()?;
        if parent == u32::MAX {
            return None;
        }
        let off = stores.field_position(data.def(parent).known_type(), fld);
        (off != u16::MAX).then(|| u32::from(off))
    };
    struct Cx<'a> {
        data: &'a Data,
        root: u16,
        watch: crate::scopes::ParamPlace,
        disturbed: Option<&'a crate::scopes::DisturbedParams>,
        ok: bool,
    }
    fn walk(cx: &mut Cx, n: &Value, field_off: &dyn Fn(i32) -> Option<u32>) {
        let node = n.unspan();
        if let Value::Call(d, args) = node
            && (*d as usize) < cx.data.definitions.len()
        {
            let def = cx.data.def(*d);
            let name = def.name();
            let first_is_root =
                matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == cx.root);
            // A user call: every argument that names the root is licensed when the callee's
            // own summary does not reach the watched place.
            if name.starts_with("n_") && !matches!(def.code(), Value::Null) {
                let places = cx.disturbed.and_then(|m| m.get(d));
                for (i, a) in args.iter().enumerate() {
                    if matches!(a.unspan(), Value::Var(w) if *w == cx.root) {
                        let reaches = places.is_some_and(|p| {
                            p.keys().any(|&(slot, inner)| {
                                usize::from(slot) == i
                                    && crate::scopes::same_place(cx.watch, (cx.root, inner))
                            })
                        });
                        // The summary records what MOVES; a callee that writes a value
                        // into the viewed vector's elements moves nothing and is still seen
                        // through a view where the copy it stands for would not see it —
                        // `s.ops[0].pts[1].py = 70` read back through the result (loft#2077,
                        // `@FR-O-ViewField`, `@FR-B-Copy`).  So a callee that writes through
                        // the argument at all declines the site.
                        if reaches
                            || cx.disturbed.is_none()
                            || crate::parser::callee_writes_param(*d, cx.data, i)
                        {
                            cx.ok = false;
                            return;
                        }
                    } else {
                        walk(cx, a, field_off);
                    }
                }
                return;
            }
            // An argument that PROJECTS the watched field is harmless where the call can only
            // read it: `len(sc.ops)` between a bind and its read moves nothing, and declining
            // it declined every cell that checks the container beside the result.
            let mut read_args: HashSet<usize> = HashSet::new();
            for (i, a) in args.iter().enumerate() {
                if read_context(cx.data, *d, i)
                    && let Value::Call(g, gargs) = a.unspan()
                    && (*g as usize) < cx.data.definitions.len()
                    && matches!(cx.data.def(*g).name(), "OpGetField" | "OpGetDbRef")
                    && matches!(gargs.first().map(Value::unspan), Some(Value::Var(w)) if *w == cx.root)
                {
                    read_args.insert(i);
                }
            }
            if !read_args.is_empty() {
                for (i, a) in args.iter().enumerate() {
                    if !read_args.contains(&i) {
                        walk(cx, a, field_off);
                    }
                }
                return;
            }
            if first_is_root {
                let reach = match name {
                    "OpPlaceRecord" => Some(None),
                    // The append's two halves name the container the same way and differ
                    // only in WHERE the field number sits: the mint takes it third, the
                    // finish fourth (after the element it publishes).
                    "OpNewRecord" | "OpNewRecordNP" => match args.get(2).map(Value::unspan) {
                        Some(Value::Int(fld)) => Some(Some(field_off(*fld).unwrap_or(u32::MAX))),
                        _ => None,
                    },
                    "OpFinishRecord" => match args.get(3).map(Value::unspan) {
                        Some(Value::Int(fld)) => Some(Some(field_off(*fld).unwrap_or(u32::MAX))),
                        _ => None,
                    },
                    "OpGetField" | "OpGetDbRef" => match args.get(1).map(Value::unspan) {
                        Some(Value::Int(off)) => {
                            Some(Some(u32::try_from(*off).unwrap_or(u32::MAX)))
                        }
                        _ => None,
                    },
                    // A fixed-width scalar read or write through an address it is GIVEN:
                    // `IN_PLACE_SET_OPS` and `SCALAR_GETTERS` are the two lists that already
                    // carry that property (`@FR-R-InPlace`), each documented as moving
                    // nothing, so a naming through one reaches its own field and no other.
                    // `s.seen = s.seen + 1` beside a view of `s.ops` is the shape.
                    n if IN_PLACE_SET_OPS.contains(&n) || SCALAR_GETTERS.contains(&n) => {
                        match args.get(1).map(Value::unspan) {
                            Some(Value::Int(off)) => {
                                Some(Some(u32::try_from(*off).unwrap_or(u32::MAX)))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match reach {
                    Some(None) => {}
                    Some(Some(off))
                        if off != cx.watch.1
                            && off != u32::MAX
                            && cx.watch.1 != crate::use_analysis::ANY_FIELD => {}
                    _ => {
                        cx.ok = false;
                        return;
                    }
                }
                for a in args.iter().skip(1) {
                    walk(cx, a, field_off);
                }
                return;
            }
        }
        if node.names_var_here(cx.root) {
            cx.ok = false;
            return;
        }
        node.for_each_child(&mut |c| walk(cx, c, field_off));
    }
    let mut cx = Cx {
        data,
        root,
        watch: (root, watch_off),
        disturbed,
        ok: true,
    };
    walk(&mut cx, stmt, &field_off);
    cx.ok
}

/// What a walk over one function knows while it classifies value shapes.
pub(crate) struct ShapeCtx<'a> {
    pub(crate) data: &'a Data,
    pub(crate) def_nr: u32,
    /// The functions admitted so far, each with the record type its tuple carries.
    pub(crate) admitted: &'a HashMap<u32, u16>,
    /// The tuple parameters admitted so far (`(R-ValueLocal)`).
    pub(crate) params: &'a TupleParams,
    /// This function's value locals so far ([`value_locals_in`]), each with the record
    /// type its tuple carries.
    pub(crate) locals: &'a HashMap<u16, u16>,
    /// `Some(def_nr)` when THIS function is admitted: then an `Object` build of its own
    /// record and a borrowed VIEW of one are value leaves as well.
    pub(crate) own: Option<u32>,
    /// `(R-FnRefValue)` — the fn-ref call sites that may answer a tuple.
    pub(crate) fnref: &'a FnRefSites,
    /// The admitted dense functions whose tuple is surely present ([`surely_present_fns`]).
    pub(crate) sp: &'a HashSet<u32>,
}

impl<'a> ShapeCtx<'a> {
    /// The same context over another set of value locals.
    pub(crate) fn with_locals<'b>(&self, locals: &'b HashMap<u16, u16>) -> ShapeCtx<'b>
    where
        'a: 'b,
    {
        ShapeCtx { locals, ..*self }
    }
}

/// The return-buffer variable of `own`, when it is admitted and has one — the PHANTOM
/// parameter the value form drops from the signature.  It may still be ASSIGNED in the
/// body: the parser's `return f(…)` lowering hands the callee this buffer and returns it
/// (`rb = f(…, rb); …; return rb`, the `one_buffer_chain` block), and a local the parser
/// PROMOTED into the buffer (`n = f(…); if !n.ok { … }; n`) is this variable under the
/// local's own name.  Both are the phantom bound from a value shape, so both are it as a
/// VALUE LOCAL ([`value_locals_in`]): the tuple the callee answered, read where the record
/// was.
pub(crate) fn own_retbuf(data: &Data, own: Option<u32>) -> Option<u16> {
    let def = data.def(own?);
    let a = ret_buffer_attr(def)?;
    let v = def.variables().var(&def.attributes()[a].name);
    (v != u16::MAX).then_some(v)
}

/// A `materialized_view_return` block — the parser's `return v[i]?`-shaped exit, which mints
/// the return buffer, copies a discharged view into it and returns the buffer — read as the
/// SOURCE of that copy: the discharge (`ncc`) block whose value is the view or its default
/// literal.  The value form returns that source's tuple and drops the mint, the copy and
/// the buffer's return; the block's other statements (the discharge buffer's frees) stay.
/// `None` for any other block.
///
/// The block has a second spelling ([`mv_block_buffer`]): where a result can be ABSENT the
/// parser copies arm by arm (`Parser::materialize_return_into`), and each copying arm is this
/// block used as a VALUE — it ends in the buffer, not in a `return` of it.  The source is the
/// leaf of both.
pub fn mv_return_source<'a>(bl: &'a Block, data: &Data) -> Option<&'a Value> {
    let (rb, _) = mv_block_buffer(bl)?;
    let copy_nr = data.def_nr("OpCopyRecord");
    bl.operators.iter().find_map(|op| match op.unspan() {
        Value::Call(d, args)
            if *d == copy_nr
                && args.len() == 3
                && matches!(args[1].unspan(), Value::Var(dst) if *dst == rb) =>
        {
            Some(&args[0])
        }
        _ => None,
    })
}

/// The buffer a `materialized_view_return` block delivers, and whether the block RETURNS it
/// (`…; return rb`) or is the buffer as a value (`…; rb`, an arm of a result that may be
/// absent).
pub(crate) fn mv_block_buffer(bl: &Block) -> Option<(u16, bool)> {
    if bl.name != "materialized_view_return" && bl.name != "materialized_view_return_armed" {
        return None;
    }
    match bl.operators.last()?.unspan() {
        Value::Return(ret) => match ret.unspan() {
            Value::Var(rb) => Some((*rb, true)),
            _ => None,
        },
        Value::Var(rb) => Some((*rb, false)),
        _ => None,
    }
}

/// `@FR-N-Store` — is the DENSE tuple `node` evaluates to SURELY PRESENT: built here, or
/// answered by a function whose every result is ([`ShapeCtx::sp`])?  A dense tuple has no
/// absent spelling (@C144), so one read off a record that may be null — a view in a dense
/// body, a tuple parameter, the result of a function that returns either — does not say
/// whether that record was there.  Only a surely present one may be widened with the
/// constant presence word; a null stored into a dense slot must read back null.
///
/// The fallback is `false` because a shape not named here is not KNOWN present, and the
/// caller then keeps the record, which costs the rewrite and never a value.
pub(crate) fn surely_present(node: &Value, c: &ShapeCtx, seen: &mut HashSet<u16>) -> bool {
    match node.unspan() {
        Value::Block(bl) if bl.name == "Object" => true,
        Value::Block(bl) if mv_return_source(bl, c.data).is_some() => {
            mv_return_source(bl, c.data).is_some_and(|src| surely_present(src, c, seen))
        }
        Value::Block(bl) if !matches!(bl.result.base(), Type::Void) => bl
            .operators
            .last()
            .is_some_and(|l| surely_present(l, c, seen)),
        Value::Insert(ops) => ops.last().is_some_and(|l| surely_present(l, c, seen)),
        Value::Return(x) => surely_present(x, c, seen),
        Value::If(_, a, b) => surely_present(a, c, seen) && surely_present(b, c, seen),
        Value::Call(d, _) => c.sp.contains(d),
        Value::Var(v) => local_surely_present(*v, c, seen),
        _ => false,
    }
}

/// [`surely_present`] of a local: every bind of it is (the declaration's `null` aside), and
/// it has one.  A parameter is not — its tuple was read off whatever the caller held — the
/// phantom return buffer aside, which is bound as a local is.  A local met again on its own
/// path is taken as present: what it finally holds came from a bind outside the cycle.
pub(crate) fn local_surely_present(v: u16, c: &ShapeCtx, seen: &mut HashSet<u16>) -> bool {
    if !seen.insert(v) {
        return true;
    }
    let def = c.data.def(c.def_nr);
    if def.variables().is_argument(v) && own_retbuf(c.data, c.own) != Some(v) {
        return false;
    }
    let (mut binds, mut present) = (0u32, true);
    def.code().any_node(&mut |n| {
        if let Value::Set(w, rhs) = n
            && *w == v
            && !matches!(rhs.unspan(), Value::Null)
        {
            binds += 1;
            present = present && surely_present(rhs, c, seen);
        }
        false
    });
    binds > 0 && present
}

/// The leaf NODES of a function's value positions, by address — what the emitter
/// converts: a `Var` that is a view (the tuple of its getters) and an `Object` block (the
/// tuple of its writes).  Any other `Object` in the body — one bound to a local that is
/// not a value local, one whose result is dropped — stays a record build.
#[derive(Default)]
pub struct ValueLeaves {
    /// `Var` leaves that are views (never value locals), by the node's address and its
    /// unspanned address, which is the identity `output_code_inner` keys on.
    pub views: HashSet<usize>,
    /// `Object` leaves, by the `Block`'s address (the `Box`'s content, which is what the
    /// block emitter is handed).
    pub objects: HashSet<usize>,
    /// EVERY `Var` node at a value position, view or value local, by both addresses — the
    /// whole-value reads a tuple serves, which is what [`local_uses_ok`] accounts a value
    /// local's read at the tail of a branch arm against.
    pub reads: HashSet<usize>,
    /// `materialized_view_return` blocks at a value position, by the `Block`'s address: the
    /// emitter returns the copy source's tuple and drops the mint, the copy and the buffer.
    pub mv_returns: HashSet<usize>,
    /// `@FR-R-ValueRecord`'s nullable clause — the `null` exits at a value position of a
    /// `-> S?` body, by both addresses: each emits the ABSENT tuple
    /// ([`ValueTuple::absent`]).
    pub nulls: HashSet<usize>,
    /// The leaves whose own tuple is DENSE and that stand where a nullable one is expected
    /// ([`widened_leaves`]), by both addresses, with the nullable key: each emits its tuple
    /// with the presence word appended.
    pub widen: HashMap<usize, u16>,
}

/// @PLN164 C5 — the SITE condition of a view leaf, per `(O-ViewField)`: the functions
/// `caller` declines because the place their leaf views cannot be shown to survive to the
/// last read of it.
///
/// Three things must hold at a site, and each has a cell:
///
/// * every read of the leaf field is a READ ([`view_field_reads`] has already accounted
///   that, or the local would not be a value local at all);
/// * every read stands in the statement list that carries the BIND, at or after it — so
///   the bind dominates the reads and a stale value from a previous loop iteration is
///   never read (a read before the bind in a loop body would be exactly that);
/// * no statement from the bind to the last read DISTURBS the place, in this frame or
///   through anything it calls ([`crate::scopes::places_disturbed_by`]).
///
/// A place the call's argument does not resolve to declines the function: the site cannot
/// then say what it must keep undisturbed.  A null-view leaf (an exit that writes the field
/// not at all) contributes no place and asks nothing of the site.
#[allow(clippy::too_many_arguments)]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(crate) fn view_sites_declined(
    data: &Data,
    stores: &Stores,
    caller: u32,
    views: &HashMap<u32, ViewPlan>,
    admitted: &HashMap<u32, u16>,
    locals: &HashMap<u16, u16>,
    view_offs: &ViewOffsets,
    disturbed: Option<&crate::scopes::DisturbedParams>,
) -> HashSet<u32> {
    let mut out = HashSet::new();
    let def = data.def(caller);
    let body = def.code();
    let mut lists: Vec<&Vec<Value>> = Vec::new();
    collect_lists(body, &mut lists);
    for list in lists {
        for (i, stmt) in list.iter().enumerate() {
            let Value::Set(v, rhs) = stmt.unspan() else {
                continue;
            };
            let Value::Call(d, args) = rhs.unspan() else {
                continue;
            };
            let Some(plan) = views.get(d) else { continue };
            let Some(tp) = admitted.get(d) else { continue };
            if locals.get(v) != Some(tp) {
                continue;
            }
            let Some(offs) = view_offs.get(tp) else {
                continue;
            };
            let reads = view_field_reads(body, *v, data, offs);
            let trace = std::env::var("LOFT_TRACE_VALUEREC").is_ok();
            // Every read inside a statement of THIS list, at or after the bind.  A read the
            // walk cannot place declines the function rather than being assumed later.
            let mut last = i;
            let mut placed: HashSet<usize> = HashSet::new();
            for (j, other) in list.iter().enumerate() {
                let mut hits = 0usize;
                other.any_node(&mut |n| {
                    let a = std::ptr::from_ref(n) as usize;
                    if reads.contains(&a) {
                        placed.insert(a);
                        hits += 1;
                    }
                    false
                });
                if hits == 0 {
                    continue;
                }
                if j < i {
                    if trace {
                        eprintln!(
                            "[viewleaf] {}: a read of `{}` stands before its bind",
                            data.def(caller).name(),
                            def.variables().name(*v)
                        );
                    }
                    out.insert(*d);
                }
                last = last.max(j);
            }
            if placed.len() < reads.len() {
                if trace {
                    eprintln!(
                        "[viewleaf] {}: {} of {} read(s) of `{}` are outside the bind's list",
                        data.def(caller).name(),
                        reads.len() - placed.len(),
                        reads.len(),
                        def.variables().name(*v)
                    );
                }
                out.insert(*d);
            }
            // The places the leaf views, mapped onto the arguments this site passed.
            let mut watch: Vec<crate::scopes::ParamPlace> = Vec::new();
            for place in plan.roots.values().flatten() {
                let Some(arg) = args.get(usize::from(place.0)) else {
                    out.insert(*d);
                    continue;
                };
                if let Some(mapped) = crate::scopes::call_arg_place(arg, data)
                    .and_then(|base| crate::scopes::compose_param_place(base, place.1))
                {
                    watch.push(mapped);
                } else {
                    if trace {
                        eprintln!(
                            "[viewleaf] {}: the argument at slot {} names no place",
                            data.def(caller).name(),
                            place.0
                        );
                    }
                    out.insert(*d);
                }
            }
            if trace {
                eprintln!(
                    "[viewleaf] {}: site of `{}` at {i}, last read {last}, {} read(s), watch {watch:?}",
                    data.def(caller).name(),
                    def.variables().name(*v),
                    reads.len()
                );
            }
            if watch.is_empty() {
                continue;
            }
            // The BIND statement is excluded, and that is the whole reason the span is a
            // span: the call itself grows the container — the append that makes the element
            // the leaf names — and the callee's own half of that question is
            // [`fresh_leaf`].  Anything else in the bind statement runs BEFORE the
            // call, since a call's arguments are evaluated first, so no disturbance in it
            // can reach the view.
            for stmt in list.iter().take(last + 1).skip(i + 1) {
                if !watch
                    .iter()
                    .all(|w| namings_avoid_place(data, stores, caller, stmt, *w, disturbed))
                {
                    if trace {
                        eprintln!(
                            "[viewleaf] {}: the place `{}` views can be disturbed before its last read",
                            data.def(caller).name(),
                            def.variables().name(*v)
                        );
                    }
                    out.insert(*d);
                }
            }
        }
    }
    out
}

/// Every STATEMENT LIST of a body — what an ordering question is asked over, because a
/// statement's position only means something inside the list that carries it.
pub(crate) fn collect_lists<'a>(v: &'a Value, out: &mut Vec<&'a Vec<Value>>) {
    match v.unspan() {
        Value::Block(bl) | Value::Loop(bl) => {
            out.push(&bl.operators);
            for op in &bl.operators {
                collect_lists(op, out);
            }
        }
        Value::Insert(ops) => {
            out.push(ops);
            for op in ops {
                collect_lists(op, out);
            }
        }
        other => other.for_each_child(&mut |c| collect_lists(c, out)),
    }
}

/// `(R-ValueLocal)` — is `arg`, at a tuple-parameter position, ALREADY a tuple: a value
/// local (or tuple parameter) of the caller, or a call to an admitted function?  Every
/// other expression there is a record the call site reads the tuple's fields off
/// (`Output::emit_call_arg`).  ONE home: the site gate and the emitter ask this together,
/// so what the gate serves as a tuple is exactly what the emitter passes as one.
#[must_use]
///
/// `tp` is the layout the parameter takes: a tuple of ANOTHER layout — a nullable one
/// handed to a dense parameter — is not ready, and is read field by field as a record
/// would be, which a tuple cannot be; the site gate therefore declines the callee there.
pub fn tuple_arg_ready(
    arg: &Value,
    tp: u16,
    admitted: &HashMap<u32, u16>,
    locals: &HashMap<u16, u16>,
) -> bool {
    match arg.unspan() {
        Value::Var(v) => locals.get(v) == Some(&tp),
        Value::Call(g, _) => admitted.get(g) == Some(&tp),
        _ => false,
    }
}

/// How `arg` crosses to a tuple parameter of layout `tp`: `Some(false)` as the tuple it
/// already is ([`tuple_arg_ready`]); `Some(true)` a DENSE tuple WIDENED into a nullable
/// parameter of its record, which only a surely present one may be (`@FR-N-Store`); `None`
/// a record the site reads field by field — with, for a nullable parameter, whether the
/// record is there.  ONE home for the site gate and the emitter.
pub(crate) fn tuple_arg_form(arg: &Value, tp: u16, c: &ShapeCtx) -> Option<bool> {
    if tuple_arg_ready(arg, tp, c.admitted, c.locals) {
        return Some(false);
    }
    if !is_nullable_layout(tp) {
        return None;
    }
    let dense = layout_record(tp);
    match arg.unspan() {
        Value::Var(v)
            if c.locals.get(v) == Some(&dense)
                && local_surely_present(*v, c, &mut HashSet::new()) =>
        {
            Some(true)
        }
        Value::Call(g, _) if c.admitted.get(g) == Some(&dense) && c.sp.contains(g) => Some(true),
        _ => None,
    }
}

/// Where a node stands, for the site gate: an admitted call is consumed as a TUPLE at a
/// result position of an admitted body (`Tail`), on the right of a value local (`Bound`),
/// or as a statement whose result is dropped (`Discard`); anywhere else (`Operand`) the
/// consumer wants a record.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pos {
    Tail,
    Bound,
    Discard,
    Operand,
}

/// The SITE gate: every admitted call in `v` that stands at an `Operand` position — an
/// argument, a field value, a return of a non-admitted function, the right of a local
/// that is not a value local — is declined into `declined`.  Result positions carry the
/// caller's position down; everything else is an operand.
#[expect(
    clippy::too_many_lines,
    reason = "the census call stands beside each trace"
)]
pub(crate) fn site_walk(node: &Value, pos: Pos, ctx: &ShapeCtx, declined: &mut HashSet<u32>) {
    match node.unspan() {
        Value::Call(callee, args) => {
            // The insert runs whenever the site declines; only the trace is conditional.
            if pos == Pos::Operand && ctx.admitted.contains_key(callee) && declined.insert(*callee)
            {
                crate::rewrite_census::declined(
                    "R-ValueRecord",
                    ctx.data.def(*callee).name(),
                    "its record is consumed as an operand at a call site",
                );
                if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
                    eprintln!(
                        "[valuerec] {}: its record is consumed in {}",
                        ctx.data.def(*callee).name(),
                        ctx.data.def(ctx.def_nr).name()
                    );
                }
            }
            for (i, arg) in args.iter().enumerate() {
                // `(R-ValueLocal)` — an admitted call handed straight to a TUPLE PARAMETER
                // is consumed as a tuple ([`tuple_arg_ready`]); any other argument
                // expression at that position is a record the site reads the fields off.
                let pos_here = if let Some(ptp) =
                    ctx.params.get(callee).and_then(|ps| ps.get(&i).copied())
                    && tuple_arg_form(arg, ptp, ctx).is_some()
                {
                    Pos::Bound
                } else {
                    Pos::Operand
                };
                site_walk(arg, pos_here, ctx, declined);
            }
        }
        Value::CallRef(v, args) => site_walk_callref(*v, args, pos, ctx, declined),
        Value::Tuple(args) => {
            for arg in args {
                site_walk(arg, Pos::Operand, ctx, declined);
            }
        }
        Value::Set(var, rhs) => {
            // @PLN164 E-1 — a forward writes the tuple into the buffer it was handed, which
            // is a position the value form serves: only the call's operands are sites.
            if forward_site(ctx.data, ctx.def_nr, *var, rhs, ctx.admitted, ctx.locals).is_some()
                && let Value::Call(_, args) = rhs.unspan()
            {
                for arg in args {
                    site_walk(arg, Pos::Operand, ctx, declined);
                }
                return;
            }
            let pos_here = if ctx.locals.contains_key(var) {
                Pos::Bound
            } else {
                Pos::Operand
            };
            site_walk(rhs, pos_here, ctx, declined);
        }
        Value::Block(bl) => {
            let count = bl.operators.len();
            for (idx, op) in bl.operators.iter().enumerate() {
                let pos_here = if idx + 1 == count && !matches!(bl.result.base(), Type::Void) {
                    pos
                } else {
                    Pos::Discard
                };
                site_walk(op, pos_here, ctx, declined);
            }
        }
        Value::Insert(ops) => {
            let count = ops.len();
            for (idx, op) in ops.iter().enumerate() {
                let pos_here = if idx + 1 == count { pos } else { Pos::Discard };
                site_walk(op, pos_here, ctx, declined);
            }
        }
        Value::If(test, then_v, else_v) => {
            site_walk(test, Pos::Operand, ctx, declined);
            site_walk(then_v, pos, ctx, declined);
            site_walk(else_v, pos, ctx, declined);
        }
        Value::Return(inner) => {
            let pos_here = if ctx.own.is_some() {
                Pos::Tail
            } else {
                Pos::Operand
            };
            site_walk(inner, pos_here, ctx, declined);
        }
        Value::Loop(bl) => {
            for op in &bl.operators {
                site_walk(op, Pos::Discard, ctx, declined);
            }
        }
        // A `par` arm'step result crosses the parallel machinery, which spells its own
        // buffered call (`n_make_pair(cell, elm, _pd1)`): then_v record, never then_v tuple.
        Value::Parallel(arms) => {
            for arm in arms {
                site_walk(arm, Pos::Operand, ctx, declined);
            }
        }
        Value::Drop(inner) => site_walk(inner, Pos::Discard, ctx, declined),
        Value::Iter(_, then_v, else_v, step) => {
            site_walk(then_v, Pos::Operand, ctx, declined);
            site_walk(else_v, Pos::Operand, ctx, declined);
            site_walk(step, Pos::Operand, ctx, declined);
        }
        Value::TuplePut(_, _, inner) | Value::Yield(inner) => {
            site_walk(inner, Pos::Operand, ctx, declined)
        }
        _ => {}
    }
}

/// [`site_walk`] over a call through fn-ref `v` (`(R-FnRefValue)`): the same gate as a direct
/// call — a tuple-answering dispatch consumed where a record is wanted takes every arm back
/// to the buffer road — and each argument a hand-off when every arm receives it as a tuple.
pub(crate) fn site_walk_callref(
    v: u16,
    args: &[Value],
    pos: Pos,
    ctx: &ShapeCtx,
    declined: &mut HashSet<u32>,
) {
    if pos == Pos::Operand
        && fnref_value(ctx.fnref, ctx.admitted, ctx.def_nr, v).is_some()
        && let Some((arms, _)) = ctx.fnref.get(&(ctx.def_nr, v))
    {
        for a in arms {
            if declined.insert(*a) {
                crate::rewrite_census::declined(
                    "R-ValueRecord",
                    ctx.data.def(*a).name(),
                    "its record is consumed through a fn-ref at a call site",
                );
                if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
                    eprintln!(
                        "[valuerec] {}: its record is consumed through a fn-ref in {}",
                        ctx.data.def(*a).name(),
                        ctx.data.def(ctx.def_nr).name()
                    );
                }
            }
        }
    }
    for (i, arg) in args.iter().enumerate() {
        let pos_here = if let Some(ptp) = fnref_param(ctx.fnref, ctx.params, ctx.def_nr, v, i)
            && tuple_arg_ready(arg, ptp, ctx.admitted, ctx.locals)
        {
            Pos::Bound
        } else {
            Pos::Operand
        };
        site_walk(arg, pos_here, ctx, declined);
    }
}

/// Is argument `i` of a call to `def` a pure READ of the collection it is given — a value the
/// callee cannot write through?
///
/// Two contexts qualify, and they are the ones a reader actually writes: an argument at a
/// `const` parameter (`len(v)`, `totp(v)` — `value_const` is the signature fact C124 carries,
/// and the `const` of a stdlib `len(self: const vector)` is the same fact), and the operand of
/// an op in [`VIEW_LEAF_READ_OPS`], which answers a VALUE rather than a place.  ONE home: the
/// site gate asks it of a leaf read and the disturbance walk asks it of a naming that reaches
/// the watched field, and the two must agree about what a read is.
pub(crate) fn read_context(data: &Data, d: u32, i: usize) -> bool {
    if (d as usize) >= data.definitions.len() {
        return false;
    }
    let def = data.def(d);
    if VIEW_LEAF_READ_OPS.contains(&def.name()) {
        return i == 0;
    }
    def.attributes().get(i).is_some_and(|a| a.value_const)
}

/// The view-field READS of local `v` in `body`, by node address — every
/// `OpGetField(v, off, _)` at one of `offs` that stands where the value can only be READ.
///
/// It is an ALLOW-list, and that direction is the point (`@FR-O-ViewField`'s site
/// condition): a read hands over the container's own vector, so anything that could WRITE
/// through it — an append, a clear, a keyed write, a by-value parameter the callee writes
/// into — must decline the function, and a use this walk does not recognise is not
/// admitted.  A missed read costs the rewrite; a missed WRITE would cost the program its
/// meaning, because the write would land in a container the caller never named.
///
/// The two recognised contexts are the ones a reader actually writes: an argument at a
/// `const` parameter (`len(m.pts)`, `totp(m.pts)` — `value_const` is the signature fact C124
/// carries, and the `const` of a stdlib `len(self: const vector)` is the same fact), and the
/// operand of an op in [`VIEW_LEAF_READ_OPS`], which answers a VALUE and not a place.
///
/// An ELEMENT read is deliberately not one of them, and that is the measurement this list is
/// cut from: `m.pts[0]?.px = 100` reads the leaf with `OpGetVectorNullable` — an op that
/// only reads its container — and then WRITES through the element it answered.  An op being
/// read-only says nothing about what its RESULT is used for, so admitting one that answers a
/// place admits the write with it (measured: the site wrote 100 into the caller's own copy in
/// the record form and into `s.ops[0].opts` through the leaf).
pub(crate) fn view_field_reads(
    body: &Value,
    v: u16,
    data: &Data,
    offs: &HashSet<i64>,
) -> HashSet<usize> {
    let mut out = HashSet::new();
    if offs.is_empty() {
        return out;
    }
    let field_of = |arg: &Value| -> Option<i64> {
        let Value::Call(g, gargs) = arg.unspan() else {
            return None;
        };
        if (*g as usize) >= data.definitions.len() || data.def(*g).name() != "OpGetField" {
            return None;
        }
        if !matches!(gargs.first().map(Value::unspan), Some(Value::Var(w)) if *w == v) {
            return None;
        }
        match gargs.get(1).map(Value::unspan) {
            Some(Value::Int(off)) if offs.contains(&i64::from(*off)) => Some(i64::from(*off)),
            _ => None,
        }
    };
    body.any_node(&mut |n| {
        let Value::Call(d, args) = n else {
            return false;
        };
        if (*d as usize) >= data.definitions.len() {
            return false;
        }
        for (i, arg) in args.iter().enumerate() {
            if field_of(arg).is_none() {
                continue;
            }
            if read_context(data, *d, i) {
                // The UNSPANNED address only: every IR walk here visits a spanned node's
                // content, so that is the identity a use is recognised by, and one address
                // per node is what makes the site gate able to COUNT them.
                out.insert(std::ptr::from_ref(arg.unspan()) as usize);
            }
        }
        false
    });
    out
}
