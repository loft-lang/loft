// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks
//! `@FR-R-PushFill`'s fill form in the IR phase: a counted loop whose whole body is ONE push of
//! a simple invariant onto a vector variable — `for _ in a..b { v += [c] }` — is one fill of
//! the vector's tail, the form `v += [c; n]` already lowers to: the push of `c`, then
//! `OpAppendCopy(v, n, tp)`, which copies that element up to `n` in all.  Both backends run it,
//! so the interpreter dispatches three operators where it dispatched the loop's per pass.
//!
//! The shape is `opt::push_window::push_loop`'s, the analysis the writers' clauses read (its
//! `fill`: one push, nothing else in the body, the value a simple invariant, the push not
//! biased).  The trip count is the range's end less its start — `#index + 1`, or the `next`
//! counter for a computed start — plus one for an inclusive range, read at the loop's entry;
//! both are pure and the fill writes neither, so the count is read twice with no temporary.
//! It is GUARDED: `if n > 0 { fill } else { the loop as written }` — for an empty or negative
//! range the loop does nothing either, and a null end, whose comparison never breaks, runs
//! the loop exactly as before.  Admitted only onto a plain local vector variable of a
//! non-nullable type: a field path's owner may be absent, which a fill would read where the
//! loop's push does not.
//!
//! `LOFT_NO_PUSH_FILL=1` turns the rule off on every backend (the writers' reservation and
//! window clauses with it); `LOFT_NO_FILL_LOOP=1` this clause alone, the loops then left to
//! the writers' reservation; `LOFT_TRACE_IR_FILL=1` names each loop taken.
use crate::data::{Data, DefType, Type, Value, v_block, v_if};
use crate::database::Stores;
use crate::opt::push_window::{push_loop, push_loop_body};

fn enabled() -> bool {
    crate::keys::push_fill_enabled()
        && !crate::env_once!(std::env::var("LOFT_NO_FILL_LOOP").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_IR_FILL").is_ok_and(|v| v != "0"))
}

struct Ops {
    add_int: u32,
    min_int: u32,
    lt_int: u32,
    append_copy: u32,
}

/// Rewrite every admitted fill loop of the program; answers how many.
pub fn rewrite_program(data: &mut Data, database: &mut Stores) -> usize {
    if !enabled() {
        return 0;
    }
    let ops = Ops {
        add_int: data.def_nr("OpAddInt"),
        min_int: data.def_nr("OpMinInt"),
        lt_int: data.def_nr("OpLtInt"),
        append_copy: data.def_nr("OpAppendCopy"),
    };
    if [ops.add_int, ops.min_int, ops.lt_int, ops.append_copy].contains(&u32::MAX) {
        if trace() {
            eprintln!("[ir-fill] off: an operator the fill is written with is not defined");
        }
        return 0;
    }
    let mut total = 0usize;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function
            || matches!(def.code(), Value::Null)
            || def.is_operator()
            || !def.rust().is_empty()
            || !def.native().is_empty()
        {
            continue;
        }
        let _census = crate::rewrite_census::InBody::enter("ir", def.name());
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let fired = rewrite(&mut code, data, d, database, &ops);
        data.definitions[d as usize].code = code;
        crate::rewrite_census::fired("R-PushFill/fill", fired);
        total += fired;
    }
    total
}

/// Replace each admitted loop under `v`; a replaced loop is not walked again (its `else` arm
/// holds the loop as written).
fn rewrite(v: &mut Value, data: &Data, d: u32, database: &mut Stores, ops: &Ops) -> usize {
    if let Value::Loop(lp) = &*v
        && let Some(replacement) = fill_of(lp, data, d, database, ops)
    {
        let original = std::mem::replace(v, Value::Null);
        let Value::If(test, then, _) = replacement else {
            unreachable!("a fill is an if")
        };
        *v = Value::If(test, then, Box::new(original));
        if trace() {
            eprintln!(
                "[ir-fill] fn={} loop {} is one fill",
                data.def(d).name(),
                lp_scope(v)
            );
        }
        return 1;
    }
    let mut n = 0;
    v.for_each_child_mut(&mut |c| n += rewrite(c, data, d, database, ops));
    n
}

fn lp_scope(v: &Value) -> u16 {
    match v {
        Value::If(_, _, e) => match e.unspan() {
            Value::Loop(lp) => lp.scope,
            _ => u16::MAX,
        },
        _ => u16::MAX,
    }
}

/// `if n > 0 { <the body's push>; OpAppendCopy(v, n, tp) } else { _ }` for loop `lp`, when it is
/// a fill onto a plain local vector variable; the `else` arm is filled in by the caller.
fn fill_of(
    lp: &crate::data::Block,
    data: &Data,
    d: u32,
    database: &mut Stores,
    ops: &Ops,
) -> Option<Value> {
    let p = push_loop(lp, data)?;
    p.fill?;
    if !p.path.1.is_empty() {
        return None;
    }
    let Value::Var(v) = p.vector.unspan() else {
        return None;
    };
    let vars = data.def(d).variables();
    let Type::Vector(elem, _) = vars.tp(*v) else {
        return None;
    };
    // The element type `[c; n]`'s lowering names: the element, or the vector itself when the
    // element is under four bytes (`Parser`'s `known_tp`).
    let elem_tp = data.vector_element_type(elem, database)?;
    let tp = if database.size(database.content(elem_tp)) < 4 {
        database.vector(elem_tp)
    } else {
        elem_tp
    };
    let start = match p.next_var {
        Some(next) => Value::Var(next),
        None => Value::Call(ops.add_int, vec![Value::Var(p.index_var), Value::Int(1)]),
    };
    let mut trip = Value::Call(ops.min_int, vec![p.hi.clone(), start]);
    if p.inclusive {
        trip = Value::Call(ops.add_int, vec![trip, Value::Int(1)]);
    }
    let mut stmts: Vec<Value> = push_loop_body(lp)?.into_iter().cloned().collect();
    stmts.push(Value::Call(
        ops.append_copy,
        vec![Value::Var(*v), trip.clone(), Value::Int(i32::from(tp))],
    ));
    // `0 < n`: loft spells `>` as the swapped `<`.
    let test = Value::Call(ops.lt_int, vec![Value::Int(0), trip]);
    Some(v_if(test, v_block(stmts, Type::Void, "fill"), Value::Null))
}
