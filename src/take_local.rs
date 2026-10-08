// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `(R-MoveLast)`'s LOCAL clause — a vector local rebound from another local that is DEAD
//! after the rebind takes that local's store instead of copying it.
//!
//! The double-buffer shape: `tmp: vector<T> = []; tmp += …; result = tmp;` — a fresh
//! vector built every turn and handed to the one that survives.  `x = y` deep-copies `y`
//! into `x`'s own store (B-Copy: a vector local owns its elements), and `y` is never read
//! again before it is minted afresh, so the copy is a whole vector's claims made and the
//! source's released for a value only `x` will hold.  Here the source is GIVEN UP into `x`'s
//! reset root (`OpCopyRecord`'s free-source form): the runtime exchanges the two stores where
//! it can (`@FR-H-SwapIn`, O(1)) and deep-copies otherwise, and frees the source's store
//! either way; its witness is null after, so its next mint claims a fresh store.
//!
//! Admitted on the IR the parser lowered: the rebind group `OpDatabase(W, tp); x =
//! OpGetField(W, 0, ft); OpSetInt4(W, 0, 0); OpAppendVector(x, y, et)` — `W` a `__vdb_`
//! witness named only by that group, its null inits and its frees — and `y` a local whose
//! one dep is its own witness `Wy`, minted by the same three-statement group EARLIER IN THE
//! SAME STATEMENT LIST with the same wrapper type, with every mention of `y` lying between
//! that mint and the rebind, and `y` named there only as the DESTINATION of an append.  So
//! each run of the rebind reads the value the same pass of the block built, and nothing
//! reads `y` until the block runs its mint again.  `Wy` is named only by its mint, its null
//! inits and its frees.  Anything else keeps the copy.  `LOFT_NO_TAKE_LOCAL=1` is the
//! switch; `LOFT_TRACE_PLACE=1` names each admission.  Both backends.
use crate::data::{Data, Value};

fn enabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_TAKE_LOCAL").is_ok_and(|v| v != "0"))
}

struct Ops {
    database: u32,
    get_field: u32,
    set_int4: u32,
    append: u32,
    free_ref: u32,
    free_if_distinct: u32,
    copy: u32,
    /// The ops that write INTO a vector named by their first argument.
    writes: Vec<u32>,
}

impl Ops {
    fn lookup(data: &Data) -> Option<Self> {
        let nr = |n: &str| {
            let d = data.def_nr(n);
            (d != u32::MAX).then_some(d)
        };
        Some(Self {
            database: nr("OpDatabase")?,
            get_field: nr("OpGetField")?,
            set_int4: nr("OpSetInt4")?,
            append: nr("OpAppendVector")?,
            free_ref: nr("OpFreeRef")?,
            free_if_distinct: nr("OpFreeRefIfDistinct")?,
            copy: nr("OpCopyRecord")?,
            writes: [
                "OpAppendVector",
                "OpPreAllocVector",
                "OpNewRecord",
                "OpFinishRecord",
            ]
            .iter()
            .filter_map(|n| nr(n))
            .collect(),
        })
    }
}

fn is_var(v: &Value, x: u16) -> bool {
    matches!(v.unspan(), Value::Var(y) if *y == x)
}

fn int(v: Option<&Value>) -> Option<i32> {
    match v.map(Value::unspan) {
        Some(Value::Int(i)) => Some(*i),
        _ => None,
    }
}

/// A mint group at `s[at..at + 3]`: `(witness, local, wrapper type, field type)`.
fn mint_group(s: &[Value], at: usize, ops: &Ops) -> Option<(u16, u16, i32, i32)> {
    let Value::Call(d, a) = s.get(at)?.unspan() else {
        return None;
    };
    let (true, [Value::Var(w), Value::Int(tp)]) = (*d == ops.database, a.as_slice()) else {
        return None;
    };
    let Value::Set(v, init) = s.get(at + 1)?.unspan() else {
        return None;
    };
    let Value::Call(g, ga) = init.unspan() else {
        return None;
    };
    if *g != ops.get_field
        || !ga.first().is_some_and(|x| is_var(x, *w))
        || int(ga.get(1)) != Some(0)
    {
        return None;
    }
    let ft = int(ga.get(2))?;
    let Value::Call(z, za) = s.get(at + 2)?.unspan() else {
        return None;
    };
    if *z != ops.set_int4 || !za.first().is_some_and(|x| is_var(x, *w)) {
        return None;
    }
    Some((*w, *v, *tp, ft))
}

/// How many nodes name `x`: a `Var`, or a `Set` of it.
fn names(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |m| match m {
        Value::Var(y) | Value::Set(y, _) if *y == x => n += 1,
        _ => {}
    });
    n
}

/// Is every mention of witness `w` in `code` one of: its mint group's three statements
/// (counted by the caller), a null init, or the first argument of a free?  Answers the
/// number of mentions outside those.
fn foreign_witness_mentions(code: &Value, w: u16, ops: &Ops) -> usize {
    let total = names(code, w);
    let mut allowed = 0usize;
    code.walk(&mut |m| match m {
        Value::Set(y, v) if *y == w && matches!(v.unspan(), Value::Null) => allowed += 1,
        Value::Call(d, a)
            if (*d == ops.free_ref || *d == ops.free_if_distinct)
                && a.first().is_some_and(|x| is_var(x, w)) =>
        {
            allowed += 1;
        }
        _ => {}
    });
    total - allowed
}

struct Plan {
    tp: i32,
    w: u16,
    wy: u16,
    x: u16,
    y: u16,
    ft: i32,
}

/// Rewrite every admitted rebind in function `d_nr`.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !enabled() || !data.def(d_nr).is_loft_defined() {
        return;
    }
    let Some(ops) = Ops::lookup(data) else {
        return;
    };
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let whole = code.clone();
    let mut fired = 0usize;
    visit(&mut code, &whole, data, d_nr, &ops, &mut fired);
    data.definitions[d_nr as usize].code = code;
    crate::rewrite_census::fired("R-MoveLast/local", fired);
}

fn visit(node: &mut Value, whole: &Value, data: &Data, d_nr: u32, ops: &Ops, fired: &mut usize) {
    match node {
        Value::Block(bl) | Value::Loop(bl) => {
            rewrite_list(&mut bl.operators, whole, data, d_nr, ops, fired);
            for s in &mut bl.operators {
                visit(s, whole, data, d_nr, ops, fired);
            }
        }
        Value::Insert(list) => {
            rewrite_list(list, whole, data, d_nr, ops, fired);
            for s in list {
                visit(s, whole, data, d_nr, ops, fired);
            }
        }
        Value::Span(b) => visit(&mut b.1, whole, data, d_nr, ops, fired),
        Value::If(c, t, e) => {
            visit(c, whole, data, d_nr, ops, fired);
            visit(t, whole, data, d_nr, ops, fired);
            visit(e, whole, data, d_nr, ops, fired);
        }
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) => {
            visit(x, whole, data, d_nr, ops, fired);
        }
        _ => {}
    }
}

fn rewrite_list(
    s: &mut Vec<Value>,
    whole: &Value,
    data: &Data,
    d_nr: u32,
    ops: &Ops,
    fired: &mut usize,
) {
    let mut i = 0;
    while i + 3 < s.len() {
        if let Some(plan) = admit(s, i, whole, data, d_nr, ops) {
            let tp_int = Value::Int(plan.ft);
            // The source is GIVEN UP into `x`'s freshly reset root: `OpCopyRecord`'s
            // free-source form exchanges the two stores where it can (`@FR-H-SwapIn`, O(1))
            // and deep-copies otherwise, freeing the source's store either way — so its
            // witness and the source itself are null after, and its next mint claims anew.
            let given_up = i32::from(crate::keys::COPY_FREE_SOURCE) | plan.tp;
            let group = vec![
                Value::Call(ops.database, vec![Value::Var(plan.w), Value::Int(plan.tp)]),
                Value::Call(
                    ops.copy,
                    vec![
                        Value::Var(plan.wy),
                        Value::Var(plan.w),
                        Value::Int(given_up),
                    ],
                ),
                Value::Set(plan.wy, Box::new(Value::Null)),
                Value::Set(
                    plan.x,
                    Box::new(Value::Call(
                        ops.get_field,
                        vec![Value::Var(plan.w), Value::Int(0), tp_int],
                    )),
                ),
                Value::Set(plan.y, Box::new(Value::Null)),
            ];
            s.splice(i..i + 4, [Value::Insert(group)]);
            *fired += 1;
        }
        i += 1;
    }
}

fn admit(s: &[Value], i: usize, whole: &Value, data: &Data, d_nr: u32, ops: &Ops) -> Option<Plan> {
    let (w, x, tp, ft) = mint_group(s, i, ops)?;
    let Value::Call(d, a) = s.get(i + 3)?.unspan() else {
        return None;
    };
    if *d != ops.append || a.len() != 3 || !is_var(&a[0], x) {
        return None;
    }
    let Value::Var(y) = a[1].unspan() else {
        return None;
    };
    let y = *y;
    let vars = data.def(d_nr).variables();
    let trace = |why: &str| {
        if crate::keys::trace_place() {
            eprintln!(
                "[take-local] fn={} {} = {}: {why}",
                data.def(d_nr).name(),
                vars.name(x),
                vars.name(y)
            );
        }
    };
    if !vars.name(w).starts_with("__vdb_") || y == x {
        return None;
    }
    // `y` owns its store through its own witness, minted earlier in this list.
    let [wy] = vars.tp(y).depend()[..] else {
        trace("declined — the source is no local owning its store");
        return None;
    };
    if wy == w || !vars.name(wy).starts_with("__vdb_") {
        trace("declined — the source's witness is not its own");
        return None;
    }
    let j = (0..i)
        .rev()
        .find(|&j| mint_group(s, j, ops).is_some_and(|(mw, my, _, _)| mw == wy && my == y));
    let Some(j) = j else {
        trace("declined — the source is not minted earlier in the same block");
        return None;
    };
    let (_, _, tp_y, _) = mint_group(s, j, ops)?;
    if tp_y != tp {
        trace("declined — the two wrappers differ");
        return None;
    }
    // Each witness: only its one mint group (3 mentions), null inits and frees.
    if foreign_witness_mentions(whole, w, ops) != 3 || foreign_witness_mentions(whole, wy, ops) != 3
    {
        trace("declined — a witness is named outside its mint and frees");
        return None;
    }
    // Every mention of `y` lies in s[j ..= i + 3], and there only as an append's
    // destination, the mint's binding, and this rebind's source.
    let inside: usize = s[j..=i + 3].iter().map(|v| names(v, y)).sum();
    let mut nulls = 0usize;
    whole.walk(&mut |m| {
        if matches!(m, Value::Set(v, n) if *v == y && matches!(n.unspan(), Value::Null)) {
            nulls += 1;
        }
    });
    if names(whole, y) != inside + nulls {
        trace("declined — the source is named outside its block run");
        return None;
    }
    // A write INTO the source names it first: a vector append, or a record push's
    // reservation, mint and finish.  The rebind's append is the one that names it second.
    let mut ok_uses = 0usize;
    for v in &s[j..=i + 3] {
        v.walk(&mut |m| {
            if let Value::Call(c, ca) = m {
                if ops.writes.contains(c) && ca.first().is_some_and(|z| is_var(z, y)) {
                    ok_uses += 1;
                }
                if *c == ops.append && ca.get(1).is_some_and(|z| is_var(z, y)) {
                    ok_uses += 1;
                }
            }
        });
    }
    // The mint's own `Set(y, …)` is the one more.
    if ok_uses + 1 != inside {
        trace("declined — the source is read other than by the rebind");
        return None;
    }
    // Only the rebind reads `y`: every other append names it as the destination.
    let mut reads = 0usize;
    for v in &s[j..=i + 3] {
        v.walk(&mut |m| {
            if let Value::Call(c, ca) = m
                && *c == ops.append
                && ca.get(1).is_some_and(|z| is_var(z, y))
            {
                reads += 1;
            }
        });
    }
    if reads != 1 {
        trace("declined — the source is read more than once");
        return None;
    }
    trace("ADMITTED");
    Some(Plan {
        tp,
        w,
        wy,
        x,
        y,
        ft,
    })
}
