// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-InRange` — a record element its loop proves in range needs no null discharge,
//! decided in the IR phase for both backends.
//!
//! `v[i]?` over a vector of records lowers to a discharge block: the nullable element read, and
//! a fallback record minted when it answers null.  An element read answers null for one reason
//! only when the element is a record — an index outside the vector; an in-range element of a
//! vector of records IS a record.  Inside `for i in 0..len(v)` whose body can neither resize `v`
//! nor assign `i` (the check `@FR-R-ForwardWalk` makes, `forward_walk::check_body`), every `i` is
//! in range, so the discharge block is the read alone.
//!
//! Not for a scalar element: `v[i]?` also discharges an element whose VALUE is null (a non-null
//! scalar type can hold the sentinel, C80), which no loop bound rules out.
//!
//! `LOFT_NO_IN_RANGE=1` keeps every discharge; `LOFT_TRACE_IN_RANGE=1` names each loop rewritten.
use crate::data::{Block, Data, DefType, Value};
use crate::forward_walk::{Walk, check_body};

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_IN_RANGE").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_IN_RANGE").is_ok_and(|v| v != "0"))
}

struct Ops {
    add_int: u32,
    le_int: u32,
    length_vector: u32,
    vector_len: u32,
    get_vector_nullable: u32,
    conv_bool_from_ref: u32,
}

fn var(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(n) => Some(*n),
        _ => None,
    }
}

fn plain(ops: &[Value]) -> Vec<&Value> {
    ops.iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect()
}

/// `for i in 0..len(v)` as the parser lowers it: the vector `v`, the counter `i#index`, the
/// loop variable `i`, and where the body starts in the loop.
fn match_range(ops: &Ops, block: &Block) -> Option<(u16, u16, u16, usize)> {
    if block.name != "For block" {
        return None;
    }
    let st = plain(&block.operators);
    let [
        Value::Set(end, len),
        Value::Set(idx, start),
        Value::Loop(lp),
    ] = st.as_slice()
    else {
        return None;
    };
    let Value::Call(ld, largs) = len.unspan() else {
        return None;
    };
    if (*ld != ops.length_vector && *ld != ops.vector_len) || largs.len() != 1 {
        return None;
    }
    let v = var(&largs[0])?;
    if !matches!(start.unspan(), Value::Int(-1)) || lp.name != "For loop" {
        return None;
    }
    let body = plain(&lp.operators);
    let Value::Set(lv, iter) = body.first()? else {
        return None;
    };
    let Value::Block(it) = iter.unspan() else {
        return None;
    };
    let iops = plain(&it.operators);
    let [Value::Set(step_var, step), Value::If(test, _, _), last] = iops.as_slice() else {
        return None;
    };
    let Value::Call(sd, sargs) = step.unspan() else {
        return None;
    };
    let Value::Call(td, targs) = test.unspan() else {
        return None;
    };
    if it.name != "Iter range"
        || step_var != idx
        || *sd != ops.add_int
        || var(&sargs[0])? != *idx
        || !matches!(sargs[1].unspan(), Value::Int(1))
        || *td != ops.le_int
        || var(&targs[0])? != *end
        || var(&targs[1])? != *idx
        || var(last)? != *idx
    {
        return None;
    }
    let first_body = lp.operators.iter().position(|o| std::ptr::eq(o, body[0]))? + 1;
    Some((v, *idx, *lv, first_body))
}

/// The record element read a discharge block wraps, when it reads `v` at `i` or `i#index`.
fn record_discharge(ops: &Ops, node: &Value, v: u16, idx: u16, lv: u16) -> Option<Value> {
    let Value::Block(b) = node.unspan() else {
        return None;
    };
    if b.name != "ncc" {
        return None;
    }
    let st = plain(&b.operators);
    let [Value::Set(t, read), Value::If(test, then_arm, _)] = st.as_slice() else {
        return None;
    };
    let Value::Call(cd, cargs) = test.unspan() else {
        return None;
    };
    let Value::Call(rd, rargs) = read.unspan() else {
        return None;
    };
    let at = var(rargs.get(2)?)?;
    (*cd == ops.conv_bool_from_ref
        && cargs.len() == 1
        && var(&cargs[0]) == Some(*t)
        && var(then_arm) == Some(*t)
        && *rd == ops.get_vector_nullable
        && var(&rargs[0]) == Some(v)
        && (at == idx || at == lv))
        .then(|| (**read).clone())
}

fn strip(ops: &Ops, node: &mut Value, v: u16, idx: u16, lv: u16, n: &mut usize) {
    if let Some(read) = record_discharge(ops, node, v, idx, lv) {
        *node = read;
        *n += 1;
        return;
    }
    node.for_each_child_mut(&mut |c| strip(ops, c, v, idx, lv, n));
}

fn rewrite_in(
    data: &Data,
    ops: &Ops,
    vars: &crate::variables::Function,
    node: &mut Value,
    fname: &str,
) -> usize {
    let mut n = 0;
    node.for_each_child_mut(&mut |c| n += rewrite_in(data, ops, vars, c, fname));
    let block = match node {
        Value::Block(b) => b,
        // A wrapped block was reached as this node's child, just above.
        Value::Span(_) => return n,
        _ => return n,
    };
    let Some((v, idx, lv, first_body)) = match_range(ops, block) else {
        return n;
    };
    let Some(Value::Loop(lp)) = block
        .operators
        .iter_mut()
        .find(|o| matches!(o, Value::Loop(_)))
    else {
        return n;
    };
    // The body may neither resize `v` nor assign `i` or `i#index`.
    for counter in [idx, lv] {
        let w = Walk {
            vec_t: v,
            src: v,
            idx: counter,
            lv,
            read: Value::Null,
            iter_scope: 0,
        };
        if vars.is_captured(v)
            || lp.operators[first_body..]
                .iter()
                .any(|o| check_body(data, vars, &w, o).is_err())
        {
            return n;
        }
    }
    let mut k = 0;
    for o in &mut lp.operators[first_body..] {
        strip(ops, o, v, idx, lv, &mut k);
    }
    if k > 0 && trace() {
        eprintln!(
            "in-range: {fname} {} — {k} discharge(s) dropped",
            vars.name(lv)
        );
    }
    n + k
}

/// Drop the discharge of every in-range record element read; answers how many.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() {
        return 0;
    }
    let ops = Ops {
        add_int: data.def_nr("OpAddInt"),
        le_int: data.def_nr("OpLeInt"),
        length_vector: data.def_nr("OpLengthVector"),
        vector_len: data.def_nr("t_6vector_len"),
        get_vector_nullable: data.def_nr("OpGetVectorNullable"),
        conv_bool_from_ref: data.def_nr("OpConvBoolFromRef"),
    };
    let mut total = 0;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        if !def
            .code()
            .any_node(&mut |n| matches!(n, Value::Block(b) if b.name == "ncc"))
        {
            continue;
        }
        let fname = def.name().to_string();
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let vars = data.definitions[d as usize].variables.clone();
        let n = rewrite_in(data, &ops, &vars, &mut code, &fname);
        data.definitions[d as usize].code = code;
        if n > 0 {
            data.definitions[d as usize].variables.reset_intervals();
            crate::scopes::compute_function_intervals(data, d);
            crate::scopes::assign_function_slots(data, d);
            total += n;
        }
    }
    crate::rewrite_census::fired("R-InRange", total);
    total
}
