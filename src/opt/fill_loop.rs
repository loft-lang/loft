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
        // A native NAME is no reason to skip: a library export or a compiled stdlib
        // function keeps the loft body a native build emits.
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
    let Some(p) = push_loop(lp, data) else {
        return several_fill(lp, data, d, database, ops);
    };
    p.fill?;
    if !p.path.1.is_empty() {
        return None;
    }
    let Value::Var(v) = p.vector.unspan() else {
        return None;
    };
    let vars = data.def(d).variables();
    let elem = match vars.tp(*v) {
        Type::Vector(elem, _) => elem,
        // `@FR-N-Shape`: a `vector<T>?` local is declined on purpose, not missed — the fill
        // would read a vector the loop's push may first have to create (the header above).
        Type::Optional(_) => return None,
        _ => return None,
    };
    let tp = copy_type(elem, data, database)?;
    let trip = trip_count(ops, p.hi, p.next_var, p.index_var, p.inclusive);
    let mut stmts: Vec<Value> = push_loop_body(lp)?.into_iter().cloned().collect();
    stmts.push(Value::Call(
        ops.append_copy,
        vec![Value::Var(*v), trip.clone(), Value::Int(i32::from(tp))],
    ));
    // `0 < n`: loft spells `>` as the swapped `<`.
    let test = Value::Call(ops.lt_int, vec![Value::Int(0), trip]);
    Some(v_if(test, v_block(stmts, Type::Void, "fill"), Value::Null))
}

/// The element type `[c; n]`'s lowering names: the element, or the vector itself when the
/// element is under four bytes (`Parser`'s `known_tp`).
fn copy_type(elem: &Type, data: &Data, database: &mut Stores) -> Option<u16> {
    let elem_tp = data.vector_element_type(elem, database)?;
    Some(if database.size(database.content(elem_tp)) < 4 {
        database.vector(elem_tp)
    } else {
        elem_tp
    })
}

/// The range's trip count read at the loop's entry: its end less its start (the `next`
/// counter, or `#index + 1`), plus one for an inclusive range.
fn trip_count(ops: &Ops, hi: &Value, next: Option<u16>, index: u16, inclusive: bool) -> Value {
    let start = match next {
        Some(next) => Value::Var(next),
        None => Value::Call(ops.add_int, vec![Value::Var(index), Value::Int(1)]),
    };
    let trip = Value::Call(ops.min_int, vec![hi.clone(), start]);
    if inclusive {
        Value::Call(ops.add_int, vec![trip, Value::Int(1)])
    } else {
        trip
    }
}

/// THE SEVERAL CLAUSE: a counted loop whose whole body is ONE push of a simple invariant onto
/// each of two or more vector variables (with each push's own reservation) — `for _ in 0..n
/// { m += [0]; sf += [0] }` — is one fill per vector: every push once, then each vector's
/// `OpAppendCopy` up to the trip count.  The vectors end as the loop leaves them, each its own
/// value `n` times: they are distinct plain locals none of which is a view of another, no
/// pushed value reads one, and nothing else in the body runs.  A byte push writes its encoded
/// element, which `OpAppendCopy` copies as it is.
fn several_fill(
    lp: &crate::data::Block,
    data: &Data,
    d: u32,
    database: &mut Stores,
    ops: &Ops,
) -> Option<Value> {
    let body = push_loop_body(lp)?;
    let rc = crate::loop_shape::range_counters(lp, data).ok()?;
    let vars = data.def(d).variables();
    let mut pushed: Vec<(u16, &Value)> = Vec::new();
    let mut reserved: Vec<u16> = Vec::new();
    for s in body.iter().filter(|o| !matches!(o.unspan(), Value::Line(_))) {
        let Value::Call(op, args) = s.unspan() else {
            return None;
        };
        let name = data.def(*op).name();
        if name == "OpPreAllocVector" {
            let Value::Var(v) = args.first()?.unspan() else {
                return None;
            };
            reserved.push(*v);
            continue;
        }
        crate::opt::writes::FUSABLE_PUSHES
            .iter()
            .find(|(n, _, _)| *n == name)?;
        let (vec_arg, _, val) = crate::opt::writes::push_operands(name, args)?;
        let Value::Var(v) = vec_arg.unspan() else {
            return None;
        };
        if pushed.iter().any(|(w, _)| w == v) {
            return None;
        }
        pushed.push((*v, val));
    }
    if pushed.len() < 2 || reserved.iter().any(|r| !pushed.iter().any(|(v, _)| v == r)) {
        return None;
    }
    let mut banned: Vec<u16> = vec![rc.loop_var, rc.index];
    banned.extend(rc.next);
    banned.extend(pushed.iter().map(|(v, _)| *v));
    if !crate::opt::push_window::simple_invariant(rc.hi, data, &banned) {
        return None;
    }
    let trip = trip_count(ops, rc.hi, rc.next, rc.index, rc.inclusive);
    let mut copies: Vec<Value> = Vec::new();
    for (v, val) in &pushed {
        let Type::Vector(elem, _) = vars.tp(*v) else {
            return None;
        };
        let views = crate::opt::body_writes::views(vars, *v);
        if vars.is_argument(*v)
            || !crate::opt::push_window::simple_invariant(val, data, &banned)
            || pushed.iter().any(|(w, _)| w != v && views.contains(w))
        {
            return None;
        }
        let tp = copy_type(elem, data, database)?;
        copies.push(Value::Call(
            ops.append_copy,
            vec![Value::Var(*v), trip.clone(), Value::Int(i32::from(tp))],
        ));
    }
    let mut stmts: Vec<Value> = body.into_iter().cloned().collect();
    stmts.extend(copies);
    crate::rewrite_census::fired("R-PushFill/several", 1);
    let test = Value::Call(ops.lt_int, vec![Value::Int(0), trip]);
    Some(v_if(test, v_block(stmts, Type::Void, "fill"), Value::Null))
}
