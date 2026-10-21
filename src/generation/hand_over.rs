// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-HandOver`'s rotate clause — an append to a full vector beside the drop of its oldest
//! element is one step on the elements already there.
//!
//! `history_push(h, e)` appends a copy of `e` to `h.entries`, then, past a depth K, lets a
//! callee drop the oldest: `<append of a copy to h.F>; S; if K < len(h.F) { g(h, len(h.F) -
//! K) }`, `g` a SUFFIX keep — its body keeps `[n, len)` of its first parameter's field F
//! (`(R-Compact)`'s `OpKeepVectorRange`) and reads F nowhere else.  When `len(h.F) == K`
//! before the append, the sequence claims a slot, claims a block per vector of the copy,
//! finishes the record, moves K elements one place forward and frees the oldest element's
//! blocks.  The rotate does the same in place (`Stores::rotate_into`): the copy is written
//! into the oldest element — its vectors into the blocks that element already owns — and the
//! slots rotate left by one.  S runs, then `g`'s body without its keep (`g__nk`), which runs
//! whatever else `g` does (`h.cursor -= n`).  A vector of any other length, a source the
//! rotate cannot take, takes the statements as written.
//!
//! `g`'s early returns are PROVED false for `n = 1` and `len = K >= 1`, atom by atom; a
//! condition this module cannot prove declines the site.  S is run in both branches, so it
//! declares nothing and touches neither the vector nor a call.  `LOFT_NO_HAND_OVER=1` keeps
//! the statements as written; `LOFT_TRACE_HAND_OVER=1` names each site.  Native only: the
//! interpreter runs the statements as written, and is the oracle the rotate is checked by.
//!
//! The rule's other clause, a dying element's blocks TAKEN for an append beside a truncation
//! (`truncate_to(h, c)` before the append), was built and measured a loss: the exact-size
//! free lists already hand a freed block to the next claim, so the take saved a claim and a
//! free that cost what the take costs (`bench/portal/analysis/worklist.md`, truncate_to).
use crate::data::{Data, Value};
use crate::database::Stores;

/// An admitted rotate site.
pub struct Site<'a> {
    /// The depth: the length at which the append drops the oldest element.
    pub k: i64,
    /// The index of the `if` statement, the last of the range.
    pub last: usize,
    /// The suffix-keep callee.
    pub g: u32,
    /// The vector, as the caller names it: `OpGetField(h, pos, tp)`.
    pub vector: Value,
    /// The copy statement that writes the appended element: `OpCopyRecord(src, elm, tp)`.
    pub copy: &'a Value,
    /// The element type.
    pub elem_tp: u16,
    /// The record copied, and the vector's owner `h`.
    pub src: &'a Value,
    pub owner: &'a Value,
    /// The statements between the append and the `if` (S), and every statement of the range
    /// as written.
    pub between: Vec<&'a Value>,
    pub range: Vec<&'a Value>,
}

fn name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn int_of(v: &Value) -> Option<i64> {
    match v.unspan() {
        Value::Int(i) => Some(i64::from(*i)),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

fn call<'v>(data: &Data, v: &'v Value, op: &str) -> Option<&'v [Value]> {
    match v.unspan() {
        Value::Call(d, a) if name(data, *d) == op => Some(a),
        _ => None,
    }
}

/// The statements of a list with `Insert`s flattened and line markers dropped.
fn flat(ops: &[Value]) -> Vec<&Value> {
    let mut out = Vec::new();
    for o in ops {
        match o.unspan() {
            Value::Insert(inner) => out.extend(flat(inner)),
            Value::Line(_) => {}
            _ => out.push(o),
        }
    }
    out
}

/// `OpGetField(Var(p), pos, tp)` → `(pos, tp)`.
fn field_of(data: &Data, v: &Value, p: u16) -> Option<(i64, i64)> {
    let [base, pos, tp] = call(data, v, "OpGetField")? else {
        return None;
    };
    matches!(base.unspan(), Value::Var(x) if *x == p).then_some(())?;
    Some((int_of(pos)?, int_of(tp)?))
}

/// A length of the field `(pos, tp)` of `Var(p)`: `len(…)` or `OpLengthVector(…)`.
fn is_len(data: &Data, v: &Value, p: u16, field: (i64, i64)) -> bool {
    let arg = call(data, v, "t_6vector_len").or_else(|| call(data, v, "OpLengthVector"));
    matches!(arg, Some([f]) if field_of(data, f, p) == Some(field))
}

/// A term of an early-return condition.
#[derive(Clone, Copy)]
enum Term {
    N,
    Len,
    K(i64),
}

/// Is `a > b` (strict) or `a >= b` (not strict) certain when the callee runs with `n = 1` and
/// `len = K >= 1` — what a rotate site calls it with?  `Len` is known only from below.
fn proves(a: Term, b: Term, strict: bool) -> bool {
    let cmp = |x: i64, y: i64| if strict { x > y } else { x >= y };
    match (a, b) {
        (Term::K(x), Term::K(y)) => cmp(x, y),
        (Term::N, Term::K(y)) => cmp(1, y),
        (Term::Len, Term::K(y)) => cmp(1, y),
        (Term::Len, Term::N) => !strict,
        (Term::K(x), Term::N) => cmp(x, 1),
        _ => false,
    }
}

fn term(data: &Data, v: &Value, field: (i64, i64)) -> Option<Term> {
    if let Some(k) = int_of(v) {
        return Some(Term::K(k));
    }
    if matches!(v.unspan(), Value::Var(1)) {
        return Some(Term::N);
    }
    is_len(data, v, 0, field).then_some(Term::Len)
}

/// Is the condition `c` certainly false at a rotate's call (`proves`)?  An `or` (`if a true
/// else b`) is false when both sides are; an atom `a <= b` when `a > b` is certain, `a < b`
/// when `a >= b` is.  Any other shape answers no, which declines the site: an unproved
/// early return could skip the rest of the callee's body, which the rotate runs.
fn certainly_false(data: &Data, c: &Value, field: (i64, i64)) -> bool {
    if let Value::If(a, t, b) = c.unspan()
        && matches!(t.unspan(), Value::Boolean(true))
    {
        return certainly_false(data, a, field) && certainly_false(data, b, field);
    }
    for (op, strict) in [("OpLeInt", true), ("OpLtInt", false)] {
        if let Some([a, b]) = call(data, c, op) {
            let (Some(a), Some(b)) = (term(data, a, field), term(data, b, field)) else {
                return false;
            };
            return proves(a, b, strict);
        }
    }
    false
}

/// Does `stmt` mention `Var(owner)`'s field at `pos`, call a user function, or leave the
/// statement list?  Every other node answers no: an operator or a stdlib method reaches no
/// vector it is not handed, and the field reaches the vector only through `OpGetField`.
fn touches_field_or_calls(data: &Data, stmt: &Value, owner: u16, pos: i64) -> bool {
    stmt.any_node(&mut |node| match node {
        Value::Call(def, args) => {
            let nm = name(data, *def);
            (!nm.starts_with("Op") && !nm.starts_with("t_"))
                || (nm == "OpGetField"
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == owner)
                    && args.get(1).and_then(int_of) == Some(pos))
        }
        Value::CallRef(..) | Value::Return(_) | Value::Break(_) | Value::Continue(_) => true,
        _ => false,
    })
}

/// The suffix keep a callee makes of its first parameter's vector field — `[n, len)` —
/// as `(pos, field tp, element tp)`.  `None` unless the body is: null inits of locals, early
/// returns proved false at a rotate's call, ONE `(R-Compact)` keep, and statements that read
/// neither the field nor anything a call could reach.  Every other shape is `None`: a
/// statement this cannot read might observe the element the rotate kept in place.
fn keep_of(data: &Data, f: u32) -> Option<(i64, i64, u16)> {
    let def = data.def(f);
    let vars = def.variables();
    if !(vars.is_argument(0) && vars.is_argument(1))
        || (2..vars.count()).any(|v| vars.is_argument(v))
    {
        return None;
    }
    let Value::Block(b) = def.code() else {
        return None;
    };
    let ops = flat(&b.operators);
    // Find the keep first: it names the field every other test is about.
    let mut keep: Option<(usize, (i64, i64), u16)> = None;
    for (i, o) in ops.iter().enumerate() {
        let Value::If(_, then, _) = o.unspan() else {
            continue;
        };
        let mut found = None;
        then.walk(&mut |n| {
            if let Value::Call(d, a) = n
                && name(data, *d) == "OpKeepVectorRange"
                && let [v, etp, lo, hi] = &a[..]
                && let Some(field) = field_of(data, v, 0)
                && let Some(etp) = int_of(etp).and_then(|t| u16::try_from(t).ok())
                && matches!(lo.unspan(), Value::Var(1))
                && is_len(data, hi, 0, field)
            {
                found = Some((field, etp));
            }
        });
        if let Some((field, etp)) = found {
            if keep.is_some() {
                return None;
            }
            keep = Some((i, field, etp));
        }
    }
    let (ki, field, etp) = keep?;
    for (i, o) in ops.iter().enumerate() {
        if i == ki {
            continue;
        }
        match o.unspan() {
            Value::Set(x, init)
                if !vars.is_argument(*x) && matches!(init.unspan(), Value::Null) => {}
            Value::If(c, then, els)
                if i < ki
                    && matches!(els.unspan(), Value::Null)
                    && then.any_node(&mut |n| matches!(n, Value::Return(_))) =>
            {
                if !certainly_false(data, c, field) {
                    return None;
                }
            }
            Value::Call(d, _) if name(data, *d) == "OpFreeRef" => {}
            _ if i > ki && !touches_field_or_calls(data, o, 0, field.0) => {}
            _ => return None,
        }
    }
    Some((field.0, field.1, etp))
}

/// The append group starting at `ops[at]`: `elm = OpNewRecord(h, ptp, fld)`, a FRESH copy
/// into `elm` that keeps its source, and the finish.  Answers the index after the group, the
/// copy statement, `h`, the field's byte position and the copied type.
fn append_group<'a>(
    ops: &[&'a Value],
    at: usize,
    data: &Data,
    stores: &Stores,
) -> Option<(usize, &'a Value, &'a Value, i64, u16)> {
    let Value::Set(elm, mint) = ops.get(at)?.unspan() else {
        return None;
    };
    let [h, ptp, fld] = call(data, mint, "OpNewRecord")? else {
        return None;
    };
    let ptp = u16::try_from(int_of(ptp)?).ok()?;
    let fld = u16::try_from(int_of(fld)?).ok()?;
    let pos = i64::from(stores.field_position(ptp, fld));
    let copy = *ops.get(at + 1)?;
    let [_src, dst, tp] = call(data, copy, "OpCopyRecord")? else {
        return None;
    };
    if !matches!(dst.unspan(), Value::Var(x) if x == elm) {
        return None;
    }
    let raw = u16::try_from(int_of(tp)?).ok()?;
    if raw & crate::keys::COPY_FRESH_DEST == 0 || raw & crate::keys::COPY_FREE_SOURCE != 0 {
        return None;
    }
    let [h2, ..] = call(data, ops.get(at + 2)?, "OpFinishRecord")? else {
        return None;
    };
    (h2.unspan() == h.unspan()).then_some(())?;
    Some((at + 3, copy, h, pos, raw & crate::keys::COPY_TP_MASK))
}

/// The vector `OpGetField(h, pos, tp)` for the caller's `h`.
fn vector_of(data: &Data, h: &Value, pos: i64, tp: i64) -> Option<Value> {
    let d = data.def_nr("OpGetField");
    (d != u32::MAX).then(|| {
        Value::Call(
            d,
            vec![h.clone(), Value::Int(pos as i32), Value::Int(tp as i32)],
        )
    })
}

/// The rotate site starting at `operators[at]`, if there is one.
pub fn site<'a>(
    operators: &'a [Value],
    at: usize,
    data: &Data,
    stores: &Stores,
) -> Option<Site<'a>> {
    if !crate::keys::hand_over_enabled() {
        return None;
    }
    let ops: Vec<&Value> = operators.iter().collect();
    // The append, S, then `if K < len(h.F) { g(h, len(h.F) - K) }`.
    let (mut next, copy, owner, pos, tp) = append_group(&ops, at, data, stores)?;
    let Value::Var(owner_var) = owner.unspan() else {
        return None;
    };
    let Value::Set(elm, _) = ops[at].unspan() else {
        return None;
    };
    let Value::Call(_, copy_args) = copy.unspan() else {
        return None;
    };
    let src = copy_args.first()?;
    let mut between = Vec::new();
    loop {
        let stmt = ops.get(next)?;
        match stmt.unspan() {
            Value::Line(_) => next += 1,
            Value::If(..) => {
                let (k, callee, vtp) = drop_if(data, stores, stmt, owner, *owner_var, (pos, tp))?;
                // The temporary the append names is the range's own: nothing after it reads it.
                if ops[next + 1..].iter().any(|later| {
                    later.any_node(&mut |node| matches!(node, Value::Var(x) if x == elm))
                }) {
                    return None;
                }
                return Some(Site {
                    k,
                    last: next,
                    g: callee,
                    vector: vector_of(data, owner, pos, vtp)?,
                    copy,
                    elem_tp: tp,
                    src,
                    owner,
                    between,
                    range: ops[at..=next].to_vec(),
                });
            }
            // S: run in both branches, so it declares nothing (`Value::Set`) a later
            // statement could read, and touches neither the vector nor a call.
            _ if !touches_field_or_calls(data, stmt, *owner_var, pos)
                && !stmt.any_node(&mut |node| matches!(node, Value::Set(..))) =>
            {
                between.push(*stmt);
                next += 1;
            }
            _ => return None,
        }
    }
}

/// `if K < len(h.F) { g(h, len(h.F) - K) }` with `g` a suffix keep of the same field and
/// element type: `(K, g, the vector type)`.  `field` is the append's `(byte position,
/// element type)`.
fn drop_if(
    data: &Data,
    stores: &Stores,
    stmt: &Value,
    owner: &Value,
    owner_var: u16,
    field: (i64, u16),
) -> Option<(i64, u32, i64)> {
    let Value::If(cond, then, els) = stmt.unspan() else {
        return None;
    };
    if !matches!(els.unspan(), Value::Null) {
        return None;
    }
    let [depth, length] = call(data, cond, "OpLtInt")? else {
        return None;
    };
    let k = int_of(depth)?;
    let (fpos, vtp) = field_of(
        data,
        call(data, length, "t_6vector_len")?.first()?,
        owner_var,
    )?;
    if fpos != field.0 || k < 1 || !plain_vector(stores, vtp) {
        return None;
    }
    let Value::Block(blk) = then.unspan() else {
        return None;
    };
    let inner = flat(&blk.operators);
    let [Value::Call(callee, cargs)] = inner.iter().map(|v| v.unspan()).collect::<Vec<_>>()[..]
    else {
        return None;
    };
    let [c_owner, count] = &cargs[..] else {
        return None;
    };
    (c_owner.unspan() == owner.unspan()).then_some(())?;
    let [len_again, depth_again] = call(data, count, "OpMinInt")? else {
        return None;
    };
    if int_of(depth_again) != Some(k) || !is_len(data, len_again, owner_var, (fpos, vtp)) {
        return None;
    }
    let (gpos, _, etp) = keep_of(data, *callee)?;
    (gpos == field.0 && etp == field.1).then_some((k, *callee, vtp))
}

/// Is the vector type `vtp` a plain `vector` (no order, no key) — the one kind whose slots a
/// rotate may move?
fn plain_vector(stores: &Stores, vtp: i64) -> bool {
    usize::try_from(vtp)
        .ok()
        .and_then(|t| stores.types.get(t))
        .is_some_and(|t| matches!(t.parts, crate::database::Parts::Vector(_)))
}

/// The keep statement of a suffix-keep callee (by node address), which its `__nk` twin skips.
pub fn keep_statement(data: &Data, g: u32) -> Option<usize> {
    let Value::Block(b) = data.def(g).code() else {
        return None;
    };
    let mut found = None;
    for o in flat(&b.operators) {
        if let Value::If(_, then, _) = o.unspan()
            && then.any_node(
                &mut |n| matches!(n, Value::Call(d, _) if name(data, *d) == "OpKeepVectorRange"),
            )
        {
            found = Some(std::ptr::from_ref(o.unspan()) as usize);
        }
    }
    found
}
