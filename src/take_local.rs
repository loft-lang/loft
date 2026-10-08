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
    *F.get_or_init(|| !std::env::var("LOFT_NO_TAKE_LOCAL").is_ok_and(|val| val != "0"))
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
        let nr = |count: &str| {
            let opd = data.def_nr(count);
            (opd != u32::MAX).then_some(opd)
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
            .filter_map(|count| nr(count))
            .collect(),
        })
    }
}

fn is_var(val: &Value, dst: u16) -> bool {
    matches!(val.unspan(), Value::Var(src) if *src == dst)
}

fn int(val: Option<&Value>) -> Option<i32> {
    match val.map(Value::unspan) {
        Some(Value::Int(at)) => Some(*at),
        _ => None,
    }
}

/// A mint group at `s[at..at + 3]`: `(witness, local, wrapper type, field type)`.
fn mint_group(stmts: &[Value], at: usize, ops: &Ops) -> Option<(u16, u16, i32, i32)> {
    let Value::Call(opd, args) = stmts.get(at)?.unspan() else {
        return None;
    };
    let (true, [Value::Var(wit), Value::Int(tp)]) = (*opd == ops.database, args.as_slice()) else {
        return None;
    };
    let Value::Set(val, init) = stmts.get(at + 1)?.unspan() else {
        return None;
    };
    let Value::Call(getter, ga) = init.unspan() else {
        return None;
    };
    if *getter != ops.get_field
        || !ga.first().is_some_and(|dst| is_var(dst, *wit))
        || int(ga.get(1)) != Some(0)
    {
        return None;
    }
    let ft = int(ga.get(2))?;
    let Value::Call(arg, za) = stmts.get(at + 2)?.unspan() else {
        return None;
    };
    if *arg != ops.set_int4 || !za.first().is_some_and(|dst| is_var(dst, *wit)) {
        return None;
    }
    Some((*wit, *val, *tp, ft))
}

/// How many nodes name `x`: a `Var`, or a `Set` of it.
fn names(val: &Value, dst: u16) -> usize {
    let mut count = 0;
    val.walk(&mut |node| match node {
        Value::Var(src) | Value::Set(src, _) if *src == dst => count += 1,
        _ => {}
    });
    count
}

/// Is every mention of witness `w` in `code` one of: its mint group's three statements
/// (counted by the caller), a null init, or the first argument of a free?  Answers the
/// number of mentions outside those.
fn foreign_witness_mentions(code: &Value, wit: u16, ops: &Ops) -> usize {
    let total = names(code, wit);
    let mut allowed = 0usize;
    code.walk(&mut |node| match node {
        Value::Set(src, val) if *src == wit && matches!(val.unspan(), Value::Null) => allowed += 1,
        Value::Call(opd, args)
            if (*opd == ops.free_ref || *opd == ops.free_if_distinct)
                && args.first().is_some_and(|dst| is_var(dst, wit)) =>
        {
            allowed += 1;
        }
        _ => {}
    });
    total - allowed
}

struct Plan {
    tp: i32,
    wit: u16,
    wy: u16,
    dst: u16,
    src: u16,
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
            for stmts in &mut bl.operators {
                visit(stmts, whole, data, d_nr, ops, fired);
            }
        }
        Value::Insert(list) => {
            rewrite_list(list, whole, data, d_nr, ops, fired);
            for stmts in list {
                visit(stmts, whole, data, d_nr, ops, fired);
            }
        }
        Value::Span(b) => visit(&mut b.1, whole, data, d_nr, ops, fired),
        Value::If(callee, t, e) => {
            visit(callee, whole, data, d_nr, ops, fired);
            visit(t, whole, data, d_nr, ops, fired);
            visit(e, whole, data, d_nr, ops, fired);
        }
        Value::Set(_, dst) | Value::Return(dst) | Value::Drop(dst) => {
            visit(dst, whole, data, d_nr, ops, fired);
        }
        _ => {}
    }
}

fn rewrite_list(
    stmts: &mut Vec<Value>,
    whole: &Value,
    data: &Data,
    d_nr: u32,
    ops: &Ops,
    fired: &mut usize,
) {
    let mut at = 0;
    while at + 3 < stmts.len() {
        if let Some(plan) = admit(stmts, at, whole, data, d_nr, ops) {
            let tp_int = Value::Int(plan.ft);
            // The source is GIVEN UP into `x`'s freshly reset root: `OpCopyRecord`'s
            // free-source form exchanges the two stores where it can (`@FR-H-SwapIn`, O(1))
            // and deep-copies otherwise, freeing the source's store either way — so its
            // witness and the source itself are null after, and its next mint claims anew.
            let given_up = i32::from(crate::keys::COPY_FREE_SOURCE) | plan.tp;
            let group = vec![
                Value::Call(
                    ops.database,
                    vec![Value::Var(plan.wit), Value::Int(plan.tp)],
                ),
                Value::Call(
                    ops.copy,
                    vec![
                        Value::Var(plan.wy),
                        Value::Var(plan.wit),
                        Value::Int(given_up),
                    ],
                ),
                Value::Set(plan.wy, Box::new(Value::Null)),
                Value::Set(
                    plan.dst,
                    Box::new(Value::Call(
                        ops.get_field,
                        vec![Value::Var(plan.wit), Value::Int(0), tp_int],
                    )),
                ),
                Value::Set(plan.src, Box::new(Value::Null)),
            ];
            stmts.splice(at..at + 4, [Value::Insert(group)]);
            *fired += 1;
        }
        at += 1;
    }
}

fn admit(
    stmts: &[Value],
    at: usize,
    whole: &Value,
    data: &Data,
    d_nr: u32,
    ops: &Ops,
) -> Option<Plan> {
    let (wit, dst, tp, ft) = mint_group(stmts, at, ops)?;
    let Value::Call(opd, args) = stmts.get(at + 3)?.unspan() else {
        return None;
    };
    if *opd != ops.append || args.len() != 3 || !is_var(&args[0], dst) {
        return None;
    }
    let Value::Var(src) = args[1].unspan() else {
        return None;
    };
    let src = *src;
    let vars = data.def(d_nr).variables();
    let trace = |why: &str| {
        if crate::keys::trace_place() {
            eprintln!(
                "[take-local] fn={} {} = {}: {why}",
                data.def(d_nr).name(),
                vars.name(dst),
                vars.name(src)
            );
        }
    };
    if !vars.name(wit).starts_with("__vdb_") || src == dst {
        return None;
    }
    // `y` owns its store through its own witness, minted earlier in this list.
    let [wy] = vars.tp(src).depend()[..] else {
        trace("declined — the source is no local owning its store");
        return None;
    };
    if wy == wit || !vars.name(wy).starts_with("__vdb_") {
        trace("declined — the source's witness is not its own");
        return None;
    }
    let mint_at = (0..at).rev().find(|&mint_at| {
        mint_group(stmts, mint_at, ops).is_some_and(|(mw, my, _, _)| mw == wy && my == src)
    });
    let Some(mint_at) = mint_at else {
        trace("declined — the source is not minted earlier in the same block");
        return None;
    };
    let (_, _, tp_y, _) = mint_group(stmts, mint_at, ops)?;
    if tp_y != tp {
        trace("declined — the two wrappers differ");
        return None;
    }
    // Each witness: only its one mint group (3 mentions), null inits and frees.
    if foreign_witness_mentions(whole, wit, ops) != 3
        || foreign_witness_mentions(whole, wy, ops) != 3
    {
        trace("declined — a witness is named outside its mint and frees");
        return None;
    }
    if let Err(why) = source_run(&stmts[mint_at..=at + 3], whole, src, ops) {
        trace(why);
        return None;
    }
    trace("ADMITTED");
    Some(Plan {
        tp,
        wit,
        wy,
        dst,
        src,
        ft,
    })
}

/// The source's mentions: every one lies in `run` (its mint through the rebind, null inits
/// aside), as the destination of a write or the rebind's one read.
fn source_run(run: &[Value], whole: &Value, src: u16, ops: &Ops) -> Result<(), &'static str> {
    // Every mention of `y` lies in s[j ..= i + 3], and there only as an append's
    // destination, the mint's binding, and this rebind's source.
    let inside: usize = run.iter().map(|val| names(val, src)).sum();
    let mut nulls = 0usize;
    whole.walk(&mut |node| {
        if matches!(node, Value::Set(val, count) if *val == src && matches!(count.unspan(), Value::Null)) {
            nulls += 1;
        }
    });
    if names(whole, src) != inside + nulls {
        return Err("declined — the source is named outside its block run");
    }
    // A write INTO the source names it first: a vector append, or a record push's
    // reservation, mint and finish.  The rebind's append is the one that names it second.
    let mut ok_uses = 0usize;
    for val in run {
        val.walk(&mut |node| {
            if let Value::Call(callee, ca) = node {
                if ops.writes.contains(callee) && ca.first().is_some_and(|arg| is_var(arg, src)) {
                    ok_uses += 1;
                }
                if *callee == ops.append && ca.get(1).is_some_and(|arg| is_var(arg, src)) {
                    ok_uses += 1;
                }
            }
        });
    }
    // The mint's own `Set(y, …)` is the one more.
    if ok_uses + 1 != inside {
        return Err("declined — the source is read other than by the rebind");
    }
    // Only the rebind reads `y`: every other append names it as the destination.
    let mut reads = 0usize;
    for val in run {
        val.walk(&mut |node| {
            if let Value::Call(callee, ca) = node
                && *callee == ops.append
                && ca.get(1).is_some_and(|arg| is_var(arg, src))
            {
                reads += 1;
            }
        });
    }
    if reads != 1 {
        return Err("declined — the source is read more than once");
    }
    Ok(())
}
