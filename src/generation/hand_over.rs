// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-HandOver` — a dying element's vectors are handed to the element appended beside it.
//!
//! `history_push(h, e)` appends a copy of `e` to `h.entries` and lets a callee drop an element
//! of the same vector — `truncate_to(h, c)` before the append, `drop_oldest(h, n)` after it.
//! The copy claims a block for each of the entry's vectors; the drop releases the same kinds
//! of blocks.  Here the emitter TAKES the dying element's blocks before the drop: each owned
//! vector's slot in the element is set to 0, so the drop frees nothing of it, and the copy
//! writes the new element's vectors into the taken blocks (`Stores::hand_over_detach`,
//! `Stores::hand_over_copy`).  Every statement still runs as written, in its order; only the
//! blocks change hands.
//!
//! Two shapes, each with the condition under which the drop CERTAINLY happens — the element
//! taken is otherwise left with empty vectors, so that condition is tested at run time before
//! anything is taken:
//!
//! - **Before the append** — `f(h, c); <append of a copy to h.F>`, `f` a PREFIX keep: its body
//!   keeps `[0, n)` of its first parameter's field F (`(R-Compact)`'s `OpKeepVectorRange`) and
//!   does nothing else that reads F.  When `0 <= c < len(h.F)`, element c dies in `f`.
//! - **After the append** — `<append of a copy to h.F>; S; if K < len(h.F) { g(h, len(h.F) -
//!   K) }`, `g` a SUFFIX keep (keeps `[n, len)`), S reading neither F nor anything a call could
//!   reach.  When `len(h.F) + 1 > K` before the append, element 0 dies in `g`.
//!
//! The callee's early return (`if len <= n { return }`, `if n <= 0 || len < n { return }`)
//! is PROVED false under the condition, atom by atom; a condition this module cannot prove
//! declines the site.  `LOFT_NO_HAND_OVER=1` keeps the plain copy; `LOFT_TRACE_HAND_OVER=1`
//! names each site.  Native only: the interpreter runs the statements with their own claims.
use crate::data::{Data, Value};
use crate::database::Stores;

/// Which element dies, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `f(h, c)` before the append: element `c` (the callee's count argument) dies.
    Before,
    /// `if K < len(h.F) { g(h, len(h.F) - K) }` after the append: element 0 dies when
    /// `len + 1 > K` before it.
    After { k: i64 },
}

/// An admitted site.
pub struct Site<'a> {
    pub kind: Kind,
    /// The statement the take is emitted in front of.
    pub at: usize,
    /// The vector the element dies in, as the caller names it: `OpGetField(h, pos, tp)`.
    pub vector: Value,
    /// `Before`: the index expression (the callee's count argument).
    pub index: Option<&'a Value>,
    /// The copy statement that writes the new element: `OpCopyRecord(src, elm, tp)`.
    pub copy: &'a Value,
    /// The element type.
    pub elem_tp: u16,
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

/// What the site's condition guarantees about the count `n` and the length `L`.
#[derive(Clone, Copy)]
enum Facts {
    /// `Before`: `0 <= n < L`.
    Prefix,
    /// `After`: `1 <= n <= L`.
    Suffix,
}

impl Facts {
    /// Is `a > b` (strict) or `a >= b` (not strict) certain?
    fn proves(self, a: Term, b: Term, strict: bool) -> bool {
        let (lo_n, lo_l) = match self {
            Facts::Prefix => (0, 1),
            Facts::Suffix => (1, 1),
        };
        match (a, b) {
            (Term::K(x), Term::K(y)) => {
                if strict {
                    x > y
                } else {
                    x >= y
                }
            }
            (Term::Len, Term::N) => match self {
                Facts::Prefix => true,
                Facts::Suffix => !strict,
            },
            (Term::N, Term::K(y)) => {
                if strict {
                    lo_n > y
                } else {
                    lo_n >= y
                }
            }
            (Term::Len, Term::K(y)) => {
                if strict {
                    lo_l > y
                } else {
                    lo_l >= y
                }
            }
            _ => false,
        }
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

/// Is the condition `c` certainly false under `facts`?  An `or` (`if a true else b`) is false
/// when both sides are; an atom `a <= b` when `a > b` is certain, `a < b` when `a >= b` is.
fn certainly_false(data: &Data, c: &Value, field: (i64, i64), facts: Facts) -> bool {
    if let Value::If(a, t, b) = c.unspan()
        && matches!(t.unspan(), Value::Boolean(true))
    {
        return certainly_false(data, a, field, facts) && certainly_false(data, b, field, facts);
    }
    for (op, strict) in [("OpLeInt", true), ("OpLtInt", false)] {
        if let Some([a, b]) = call(data, c, op) {
            let (Some(a), Some(b)) = (term(data, a, field), term(data, b, field)) else {
                return false;
            };
            return facts.proves(a, b, strict);
        }
    }
    false
}

/// Does `v` mention `Var(p)`'s field at `pos`, or call a user function?
fn touches_field_or_calls(data: &Data, v: &Value, p: u16, pos: i64) -> bool {
    v.any_node(&mut |n| match n {
        Value::Call(d, a) => {
            let nm = name(data, *d);
            (!nm.starts_with("Op") && !nm.starts_with("t_"))
                || (nm == "OpGetField"
                    && matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if *x == p)
                    && a.get(1).and_then(int_of) == Some(pos))
        }
        Value::CallRef(..) | Value::Return(_) | Value::Break(_) | Value::Continue(_) => true,
        _ => false,
    })
}

/// The keep a callee makes of its first parameter's vector field: `(kind, pos, field tp,
/// element tp)`, `kind` `true` for a prefix keep `[0, n)`, `false` for a suffix keep
/// `[n, len)`.  `None` unless the body is: null inits of locals, early returns whose
/// conditions the site's facts prove false, ONE `(R-Compact)` keep, and statements that
/// read neither the field nor anything a call could reach.
fn keep_of(data: &Data, f: u32) -> Option<(bool, i64, i64, u16)> {
    let def = data.def(f);
    let vars = def.variables();
    if !(vars.is_argument(0) && vars.is_argument(1)) || (2..vars.count()).any(|v| vars.is_argument(v)) {
        return None;
    }
    let Value::Block(b) = def.code() else {
        return None;
    };
    let ops = flat(&b.operators);
    // Find the keep first: it names the field every other test is about.
    let mut keep: Option<(usize, bool, (i64, i64), u16)> = None;
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
            {
                let prefix = int_of(lo) == Some(0) && matches!(hi.unspan(), Value::Var(1));
                let suffix = matches!(lo.unspan(), Value::Var(1)) && is_len(data, hi, 0, field);
                if prefix || suffix {
                    found = Some((prefix, field, etp));
                }
            }
        });
        if let Some((prefix, field, etp)) = found {
            if keep.is_some() {
                return None;
            }
            keep = Some((i, prefix, field, etp));
        }
    }
    let (ki, prefix, field, etp) = keep?;
    let facts = if prefix { Facts::Prefix } else { Facts::Suffix };
    for (i, o) in ops.iter().enumerate() {
        if i == ki {
            continue;
        }
        match o.unspan() {
            Value::Set(x, init) if !vars.is_argument(*x) && matches!(init.unspan(), Value::Null) => {}
            Value::If(c, then, els)
                if i < ki
                    && matches!(els.unspan(), Value::Null)
                    && then.any_node(&mut |n| matches!(n, Value::Return(_))) =>
            {
                if !certainly_false(data, c, field, facts) {
                    return None;
                }
            }
            Value::Call(d, _) if name(data, *d) == "OpFreeRef" => {}
            _ if i > ki && !touches_field_or_calls(data, o, 0, field.0) => {}
            _ => return None,
        }
    }
    Some((prefix, field.0, field.1, etp))
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

/// A pure read for an index: a literal, a variable, or a scalar field read of one.
fn pure(data: &Data, v: &Value) -> bool {
    !v.any_node(&mut |n| match n {
        Value::Call(d, _) => !matches!(name(data, *d), "OpGetInt" | "OpGetLong"),
        Value::CallRef(..) | Value::Set(..) => true,
        _ => false,
    })
}

/// The hand-over site starting at `operators[at]`, if there is one.
pub fn site<'a>(operators: &'a [Value], at: usize, data: &Data, stores: &Stores) -> Option<Site<'a>> {
    if !crate::keys::hand_over_enabled() {
        return None;
    }
    // Indices into the flattened list map back to `operators` only where nothing was
    // flattened: a site is recognised among top-level statements alone.
    let ops: Vec<&Value> = operators.iter().collect();
    let here = ops.get(at)?;
    // BEFORE: `f(h, c)` then the append.
    if let Value::Call(f, args) = here.unspan()
        && let [h, c] = &args[..]
        && matches!(h.unspan(), Value::Var(_))
        && pure(data, c)
        && let Some((true, pos, vtp, etp)) = keep_of(data, *f)
    {
        let mut next = at + 1;
        while matches!(ops.get(next).map(|o| o.unspan()), Some(Value::Line(_))) {
            next += 1;
        }
        if let Some((_, copy, h2, apos, tp)) = append_group(&ops, next, data, stores)
            && h2.unspan() == h.unspan()
            && apos == pos
            && tp == etp
        {
            return Some(Site {
                kind: Kind::Before,
                at,
                vector: vector_of(data, h, pos, vtp)?,
                index: Some(c),
                copy,
                elem_tp: etp,
            });
        }
        return None;
    }
    // AFTER: the append, S, then `if K < len(h.F) { g(h, len(h.F) - K) }`.
    let (mut next, copy, h, pos, tp) = append_group(&ops, at, data, stores)?;
    let hv = match h.unspan() {
        Value::Var(x) => *x,
        _ => return None,
    };
    loop {
        let o = ops.get(next)?;
        match o.unspan() {
            Value::Line(_) => next += 1,
            Value::If(c, then, els) if matches!(els.unspan(), Value::Null) => {
                let [kv, lenv] = call(data, c, "OpLtInt")? else {
                    return None;
                };
                let k = int_of(kv)?;
                let lf = call(data, lenv, "t_6vector_len")?;
                let (fpos, vtp) = field_of(data, lf.first()?, hv)?;
                if fpos != pos {
                    return None;
                }
                let Value::Block(b) = then.unspan() else {
                    return None;
                };
                let inner = flat(&b.operators);
                let [Value::Call(g, gargs)] = inner.iter().map(|v| v.unspan()).collect::<Vec<_>>()[..]
                else {
                    return None;
                };
                let [gh, gn] = &gargs[..] else {
                    return None;
                };
                (gh.unspan() == h.unspan()).then_some(())?;
                let [l2, k2] = call(data, gn, "OpMinInt")? else {
                    return None;
                };
                if int_of(k2) != Some(k) || !is_len(data, l2, hv, (pos, vtp)) {
                    return None;
                }
                let (false, gpos, _, etp) = keep_of(data, *g)? else {
                    return None;
                };
                if gpos != pos || etp != tp {
                    return None;
                }
                return Some(Site {
                    kind: Kind::After { k },
                    at,
                    vector: vector_of(data, h, pos, vtp)?,
                    index: None,
                    copy,
                    elem_tp: etp,
                });
            }
            _ if !touches_field_or_calls(data, o, hv, pos) => next += 1,
            _ => return None,
        }
    }
}
