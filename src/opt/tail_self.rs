// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-TailSelf` — a SELF-call in tail position becomes a jump to the function's entry,
//! decided in the IR phase for both backends (loft-lang/plans#202 P5a).
//!
//! The body of a function whose parameters and locals are all scalars, and whose result is a
//! scalar or nothing, is wrapped in a loop.  Every self-call in TAIL position — the value a
//! `return` answers, the value an `if` arm or a block answers in that position, the last
//! statement of a void body — binds its arguments to fresh temporaries, the parameters from
//! them, and continues the loop; every other value in that position returns.  So `count(n -
//! 1, acc + 1)` as the last thing a function does takes no frame, and the recursion a
//! compiler's loops are written as runs to any depth.  A self-call anywhere else stays a call,
//! and a tail call inside a user loop continues the outer loop by its depth.
//!
//! The first clause admits SCALAR parameters and locals only: the frame owns no store, so
//! nothing is released before the jump.  A heap parameter's rebind is a copy where a call
//! hands a reference over (calls.md), which is P5b's clause.  Not a body with a generator, a
//! `parallel` block or a closure.
//!
//! Unlike the leaf inliner, this runs in an `open_world` program and in a coverage run too:
//! the depth a program reaches is a property of the language, not of the lane, so an
//! `--interpret` run, a `--native` semantics run (which ships the live tier and so is
//! `open_world`), a shipped binary and `loft test` must agree on it.  A live flip therefore
//! reaches a tail-recursive function at its next entry from outside, as it reaches a `while`
//! loop; coverage still sees the function entered by the call that starts the recursion.
//! `LOFT_NO_TAIL_SELF=1` keeps the calls; `LOFT_TRACE_TAIL_SELF=1` names each rewrite and
//! each decline.

use crate::data::{Data, DefType, Type, Value, is_scalar, v_block, v_loop, v_set};

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_TAIL_SELF").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_TAIL_SELF").is_ok_and(|v| v != "0"))
}

/// Rewrite every admitted function of the program; answers the number of self-calls made
/// jumps, which the rewrite census records as `R-TailSelf`.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() {
        return 0;
    }
    let mut jumps = 0usize;
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
        let params: Vec<u16> = (0..def.variables().count())
            .filter(|&v| def.variables().is_argument(v))
            .collect();
        if tail_self_calls(def.code(), d, params.len(), 0) == 0 {
            continue;
        }
        let name = def.name().to_string();
        if let Err(why) = admit(data, d) {
            if trace() {
                eprintln!("tail-self: {name} declined: {why}");
            }
            crate::rewrite_census::declined("R-TailSelf", &name, why);
            continue;
        }
        let _census_body = crate::rewrite_census::InBody::enter("ir", &name);
        let voids = matches!(data.def(d).returned().base(), Type::Void | Type::Null);
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let scope = match &code {
            Value::Block(b) => b.scope,
            _ => 0,
        };
        let vars = &mut data.definitions[d as usize].variables;
        let temps: Vec<u16> = params
            .iter()
            .map(|&p| {
                let tp = vars.tp(p).clone();
                vars.add_unique("tail", &tp, scope)
            })
            .collect();
        let n = rewrite_body(&mut code, d, &params, &temps, voids);
        data.definitions[d as usize].code = code;
        if trace() {
            eprintln!("tail-self: fn {name} — {n} self-call(s) made jumps");
        }
        jumps += n;
        data.definitions[d as usize].variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, d);
        crate::scopes::assign_function_slots(data, d);
    }
    crate::rewrite_census::fired("R-TailSelf", jumps);
    jumps
}

/// The first clause's conditions, each named for the census.
fn admit(data: &Data, d: u32) -> Result<(), &'static str> {
    let def = data.def(d);
    let result = def.returned();
    if !(is_scalar(result) || matches!(result.base(), Type::Void | Type::Null)) {
        return Err("the result is not a scalar");
    }
    let vars = def.variables();
    for v in 0..vars.count() {
        let tp = vars.tp(v);
        if matches!(tp.base(), Type::RefVar(_)) {
            return Err("a `&` parameter");
        }
        if !is_scalar(tp) {
            return Err(if vars.is_argument(v) {
                "a parameter that is not a scalar"
            } else {
                "a local that is not a scalar"
            });
        }
    }
    let blocked = def.code().any_node(&mut |n| {
        matches!(
            n,
            Value::Yield(_) | Value::Parallel(_) | Value::FnRef(_, _, _) | Value::FnRefDnr(_)
        )
    });
    if blocked {
        return Err("a generator, a parallel block or a closure in the body");
    }
    Ok(())
}

/// Count the self-calls in tail position — the same walk the rewrite makes, without it.
fn tail_self_calls(code: &Value, d: u32, arity: usize, depth: usize) -> usize {
    let Value::Block(b) = code else {
        return 0;
    };
    let mut n = 0;
    let last = last_value(&b.operators);
    for (i, op) in b.operators.iter().enumerate() {
        if Some(i) == last {
            n += count_tail(op, d, arity, depth);
        } else {
            n += count_statement(op, d, arity, depth);
        }
    }
    n
}

fn count_tail(v: &Value, d: u32, arity: usize, depth: usize) -> usize {
    match v {
        Value::Span(bx) => count_tail(&bx.1, d, arity, depth),
        Value::Call(c, args) if *c == d && args.len() == arity => 1,
        Value::If(_, a, b) => count_tail(a, d, arity, depth) + count_tail(b, d, arity, depth),
        Value::Block(b) => {
            let last = last_value(&b.operators);
            b.operators
                .iter()
                .enumerate()
                .map(|(i, op)| {
                    if Some(i) == last {
                        count_tail(op, d, arity, depth)
                    } else {
                        count_statement(op, d, arity, depth)
                    }
                })
                .sum()
        }
        Value::Return(e) => count_tail(e, d, arity, depth),
        other => count_statement(other, d, arity, depth),
    }
}

/// Outside tail position only an explicit `return` opens a tail position, so the count is the
/// sum over the `return`s below — each visited once, through the immutable child walk.
fn count_statement(v: &Value, d: u32, arity: usize, depth: usize) -> usize {
    match v {
        Value::Span(bx) => count_statement(&bx.1, d, arity, depth),
        Value::Return(e) => count_tail(e, d, arity, depth),
        Value::Loop(b) => b
            .operators
            .iter()
            .map(|op| count_statement(op, d, arity, depth + 1))
            .sum(),
        other => {
            let below = if matches!(other, Value::Iter(..)) {
                depth + 1
            } else {
                depth
            };
            let mut n = 0;
            other.for_each_child(&mut |c| n += count_statement(c, d, arity, below));
            n
        }
    }
}

/// The index of a block's value: its last operator that is not a source-line marker.
fn last_value(ops: &[Value]) -> Option<usize> {
    ops.iter()
        .rposition(|op| !matches!(op.unspan(), Value::Line(_)))
}

/// Wrap the body in the loop; answers the number of self-calls made jumps.
fn rewrite_body(code: &mut Value, d: u32, params: &[u16], temps: &[u16], voids: bool) -> usize {
    let Value::Block(b) = code else {
        return 0;
    };
    let mut n = 0;
    let ops = std::mem::take(&mut b.operators);
    let last = last_value(&ops);
    let mut body: Vec<Value> = Vec::with_capacity(ops.len() + 1);
    for (i, mut op) in ops.into_iter().enumerate() {
        if Some(i) == last {
            if voids {
                op = tail_void(op, d, params, temps, 0, &mut n);
            } else {
                op = tail(op, d, params, temps, 0, &mut n);
            }
        } else {
            statement(&mut op, d, params, temps, 0, &mut n);
        }
        body.push(op);
    }
    if voids {
        // A void body that reaches its end returns; only a jump stays in the loop.
        body.push(Value::Return(Box::new(Value::Null)));
    }
    b.operators = vec![v_loop(body, "tail_self")];
    n
}

/// A value in tail position: a self-call becomes the jump, an `if` or block hands the position
/// to its arms or its value, a `return` is already there, anything else returns.  The
/// fallback is right because the value position of a non-void function holds the VALUE: a
/// statement node there (`Set`, `Insert`, a void call) would make the function's value void,
/// which the parser refuses before this pass runs — so every other node is the expression
/// the function answers, and `return` of it is what the body already meant.  A block visited
/// here ends in a terminator on every path, so its result is `never`, as the parser types a
/// block that ends in `return`.
fn tail(v: Value, d: u32, params: &[u16], temps: &[u16], depth: usize, n: &mut usize) -> Value {
    match v {
        Value::Span(bx) => {
            let (pos, inner) = *bx;
            Value::Span(Box::new((pos, tail(inner, d, params, temps, depth, n))))
        }
        Value::Call(c, args) if c == d && args.len() == params.len() => {
            *n += 1;
            jump(args, params, temps, depth)
        }
        Value::If(cond, then, otherwise) => Value::If(
            cond,
            Box::new(tail(*then, d, params, temps, depth, n)),
            Box::new(tail(*otherwise, d, params, temps, depth, n)),
        ),
        Value::Block(mut b) => {
            let last = last_value(&b.operators);
            let ops = std::mem::take(&mut b.operators);
            b.operators = ops
                .into_iter()
                .enumerate()
                .map(|(i, mut op)| {
                    if Some(i) == last {
                        tail(op, d, params, temps, depth, n)
                    } else {
                        statement(&mut op, d, params, temps, depth, n);
                        op
                    }
                })
                .collect();
            b.result = Type::Never;
            Value::Block(b)
        }
        Value::Return(e) => tail(*e, d, params, temps, depth, n),
        mut other => {
            statement(&mut other, d, params, temps, depth, n);
            Value::Return(Box::new(other))
        }
    }
}

/// The last statement of a void body: a self-call becomes the jump; anything else stays a
/// statement (the loop's fall-through return follows it).
fn tail_void(
    v: Value,
    d: u32,
    params: &[u16],
    temps: &[u16],
    depth: usize,
    n: &mut usize,
) -> Value {
    match v {
        Value::Span(bx) => {
            let (pos, inner) = *bx;
            Value::Span(Box::new((
                pos,
                tail_void(inner, d, params, temps, depth, n),
            )))
        }
        Value::Call(c, args) if c == d && args.len() == params.len() => {
            *n += 1;
            jump(args, params, temps, depth)
        }
        Value::If(cond, then, otherwise) => Value::If(
            cond,
            Box::new(tail_void(*then, d, params, temps, depth, n)),
            Box::new(tail_void(*otherwise, d, params, temps, depth, n)),
        ),
        Value::Block(mut b) => {
            let last = last_value(&b.operators);
            let ops = std::mem::take(&mut b.operators);
            b.operators = ops
                .into_iter()
                .enumerate()
                .map(|(i, mut op)| {
                    if Some(i) == last {
                        tail_void(op, d, params, temps, depth, n)
                    } else {
                        statement(&mut op, d, params, temps, depth, n);
                        op
                    }
                })
                .collect();
            Value::Block(b)
        }
        mut other => {
            statement(&mut other, d, params, temps, depth, n);
            other
        }
    }
}

/// A statement outside tail position: its explicit `return`s are tail positions of their own,
/// and a loop below it is one level the jump has to count.
fn statement(v: &mut Value, d: u32, params: &[u16], temps: &[u16], depth: usize, n: &mut usize) {
    match v {
        Value::Span(bx) => statement(&mut bx.1, d, params, temps, depth, n),
        Value::Return(e) => {
            let inner = std::mem::replace(&mut **e, Value::Null);
            *v = tail(inner, d, params, temps, depth, n);
        }
        Value::Loop(b) => {
            for op in &mut b.operators {
                statement(op, d, params, temps, depth + 1, n);
            }
        }
        Value::Iter(..) => {
            v.for_each_child_mut(&mut |c| statement(c, d, params, temps, depth + 1, n));
        }
        _ => v.for_each_child_mut(&mut |c| statement(c, d, params, temps, depth, n)),
    }
}

/// The jump: every argument into its temporary, every parameter from its temporary — so an
/// argument that reads a parameter reads the value before any rebind — then the loop
/// continues, by the depth of the user loops between the call and the function's own loop.
fn jump(args: Vec<Value>, params: &[u16], temps: &[u16], depth: usize) -> Value {
    let mut ops = Vec::with_capacity(2 * params.len() + 1);
    for (&t, a) in temps.iter().zip(args) {
        ops.push(v_set(t, a));
    }
    for (&p, &t) in params.iter().zip(temps) {
        ops.push(v_set(p, Value::Var(t)));
    }
    ops.push(Value::Continue(depth as u16));
    v_block(ops, Type::Never, "tail_jump")
}
