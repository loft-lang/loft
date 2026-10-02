// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-DischargeInto` — a text discharge that assigns a local is read straight into that
//! local, decided in the IR phase for both backends.
//!
//! `w = v[i] ?? d` in statement position lowers to `{t = <read>; if t is not null then w = t
//! else w = d}` and a `free t` after it.  A text local owns its bytes, so the interpreter
//! copies the element into `t`, copies `t` into `w`, and frees `t`: two allocations a word
//! (`bench/portal/analysis/word-count.md`).  The temporary only exists to hold the read
//! while it is tested, and `w` can hold it as well: `w = <read>; if w is null then w = d`.
//!
//! Exact when `w` appears in neither the read nor the default.  Assigning a text clears the
//! target before its value is evaluated, so a read of `w` would see it emptied, and the
//! default now runs after `w` holds the read rather than its old value.  The temporary must
//! be used nowhere else: its one assignment, the test, the arm that moves it, and its free.
//!
//! `LOFT_NO_DISCHARGE_INTO=1` keeps every temporary; `LOFT_TRACE_DISCHARGE_INTO=1` names each
//! function and how many it read straight in.
use crate::data::{Data, DefType, Value};

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_DISCHARGE_INTO").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_DISCHARGE_INTO").is_ok_and(|v| v != "0"))
}

struct Ops {
    conv_bool_from_text: u32,
    free_text: u32,
    not: u32,
}

fn mentions(v: &Value, var: u16) -> bool {
    v.any_node(&mut |n| matches!(n, Value::Var(x) | Value::Set(x, _) if *x == var))
}

/// How often `v` names `var` at all — reads, assignments and the frees that take it.
fn uses(v: &Value, var: u16) -> usize {
    let mut n = 0;
    count(v, var, &mut n);
    n
}

fn count(v: &Value, var: u16, n: &mut usize) {
    if matches!(v, Value::Var(x) | Value::Set(x, _) if *x == var) {
        *n += 1;
    }
    v.for_each_child(&mut |c| count(c, var, n));
}

/// The parts of `{t = read; if conv(t) then w = t else w = d}`: `t`, `w`, the read, `d`.
fn discharge_into<'a>(ops: &Ops, node_v: &'a Value) -> Option<(u16, u16, &'a Value, &'a Value)> {
    let Value::Block(blk) = node_v.unspan() else {
        return None;
    };
    if blk.name != "ncc" {
        return None;
    }
    let st: Vec<&Value> = blk
        .operators
        .iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let [Value::Set(tmp, read), Value::If(test, then_arm, else_arm)] = st.as_slice() else {
        return None;
    };
    let Value::Call(conv, cargs) = test.unspan() else {
        return None;
    };
    let Value::Set(target, moved) = then_arm.unspan() else {
        return None;
    };
    let Value::Set(target2, dflt) = else_arm.unspan() else {
        return None;
    };
    (*conv == ops.conv_bool_from_text
        && cargs.len() == 1
        && matches!(cargs[0].unspan(), Value::Var(x) if x == tmp)
        && matches!(moved.unspan(), Value::Var(x) if x == tmp)
        && target == target2
        && target != tmp
        && !mentions(read, *target)
        && !mentions(dflt, *target))
    .then_some((*tmp, *target, &**read, &**dflt))
}

/// Is `node_v` the statement `free t`?
fn frees(ops: &Ops, node_v: &Value, tmp: u16) -> bool {
    matches!(node_v.unspan(), Value::Call(f, a) if *f == ops.free_text && a.len() == 1
        && matches!(a[0].unspan(), Value::Var(x) if *x == tmp))
}

fn rewrite_in(ops: &Ops, code: &Value, node: &mut Value) -> usize {
    let mut n = 0;
    node.for_each_child_mut(&mut |c| n += rewrite_in(ops, code, c));
    let block = match node {
        Value::Block(blk) | Value::Loop(blk) => blk,
        _ => return n,
    };
    let mut at = 0;
    while at < block.operators.len() {
        let found = discharge_into(ops, &block.operators[at])
            .map(|(tmp, target, read, dflt)| (tmp, target, read.clone(), dflt.clone()));
        let Some((tmp, target, read, dflt)) = found else {
            at += 1;
            continue;
        };
        // The free of the temporary is the next statement that names it.
        let free_at = (at + 1..block.operators.len())
            .find(|&k| mentions(&block.operators[k], tmp))
            .filter(|&k| frees(ops, &block.operators[k], tmp));
        // Its assignment, the test, the moving arm and the free: nothing else may name it.
        let Some(free_at) = free_at.filter(|_| uses(code, tmp) == 4) else {
            at += 1;
            continue;
        };
        block.operators.remove(free_at);
        let Value::Block(blk) = block.operators[at].unspan_mut() else {
            unreachable!("matched as a block above");
        };
        let is_null = Value::Call(
            ops.not,
            vec![Value::Call(
                ops.conv_bool_from_text,
                vec![Value::Var(target)],
            )],
        );
        blk.operators = vec![
            Value::Set(target, Box::new(read)),
            Value::If(
                Box::new(is_null),
                Box::new(Value::Set(target, Box::new(dflt))),
                Box::new(Value::Null),
            ),
        ];
        n += 1;
        at += 1;
    }
    n
}

/// Read every admitted text discharge straight into its local; answers how many.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() {
        return 0;
    }
    let ops = Ops {
        conv_bool_from_text: data.def_nr("OpConvBoolFromText"),
        free_text: data.def_nr("OpFreeText"),
        not: data.def_nr("OpNot"),
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
        let name = def.name().to_string();
        let snapshot = def.code().clone();
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let n = rewrite_in(&ops, &snapshot, &mut code);
        data.definitions[d as usize].code = code;
        if n > 0 {
            if trace() {
                eprintln!("discharge-into: {name} — {n} read(s) straight into their local");
            }
            data.definitions[d as usize].variables.reset_intervals();
            crate::scopes::compute_function_intervals(data, d);
            crate::scopes::assign_function_slots(data, d);
            total += n;
        }
    }
    crate::rewrite_census::fired("R-DischargeInto", total);
    total
}
