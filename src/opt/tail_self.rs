// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-TailSelf` — a SELF-call in tail position becomes a jump to the function's entry,
//! decided in the IR phase for both backends (loft-lang/plans#202 P5a and P5b).
//!
//! The body of a function whose result is a scalar or nothing is wrapped in a loop.  Every
//! self-call in TAIL position — the value a `return` answers, the value an `if` arm or a block
//! answers in that position, the last statement of a void body, the value of the `__ret_N`
//! temporary the scope pass hands to a `return` — binds its arguments to fresh temporaries,
//! releases what the frame owns (the scope pass's own release ops for the locals, the ones
//! that would have run after the call returned), binds the parameters from the temporaries,
//! and continues the loop; every other value in that position returns.  So a `count(n - 1,
//! acc + 1)` that is the last thing a function does takes no frame, and the recursion a
//! compiler's loops are written as runs to any depth.  A self-call anywhere else stays a call,
//! and a tail call inside a user loop continues the function's loop by the loops between.
//!
//! A scalar parameter is rebound by a copy, as a call binds it.  A parameter held in a store
//! (a record, a vector, a collection, a text, an iterator) and a `&` parameter are pointers the
//! frame never owned — a call SHARES them with the caller (calls.md `(F-ParamHeap)`) — so their
//! rebind is exact when the argument is such a pointer too: the parameter itself (then nothing
//! moves), another parameter, or a part of one (`p.next`, `v[i]`, a projection rooted at a
//! parameter).  An argument that MAKES a value — a call, a literal, an operator result — or
//! names a local the jump releases would have to be handed over into the frame's ownership,
//! which a call does not do either; that binding has no spelling here, and the heap RESULT
//! (delivered into the caller's hidden buffer) has none yet.  The frame's LOCALS may be held
//! in a store (a text, a vector, a record, the compiler's own `__work_N` buffers): each is
//! declared afresh by the body, so the jump releases them where the return would have.
//!
//! What this pass cannot make a jump is REFUSED by the parser (@C155, `definitions.rs`), with
//! the same predicate (`admit_frame`), so a self-call in tail position is never silently a
//! call that stops at `MAX_CALL_DEPTH`.
//!
//! Unlike the leaf inliner, this runs in an `open_world` program and in a coverage run too:
//! the depth a program reaches is a property of the language, not of the lane, so an
//! `--interpret` run, a `--native` semantics run (which ships the live tier and so is
//! `open_world`), a shipped binary and `loft test` must agree on it.  A live flip therefore
//! reaches a tail-recursive function at its next entry from outside, as it reaches a `while`
//! loop; coverage still sees the function entered by the call that starts the recursion.
//! `LOFT_NO_TAIL_SELF=1` keeps the calls; `LOFT_TRACE_TAIL_SELF=1` names each rewrite and
//! each decline.

use crate::data::{Data, DefType, Type, Value, v_block, v_loop, v_set};
use crate::lexer::Position;
use crate::tail_frame::{
    Pos, admit_frame, declared_params, is_release, last_value, ret_pattern, tail_calls, void_tail,
};
use crate::variables::Function;

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
        if tail_calls(def.code(), d, def.variables(), data).is_empty() {
            continue;
        }
        let name = def.name().to_string();
        if let Err(why) = admit_frame(
            def.variables(),
            def.returned(),
            def.code(),
            data,
            &declared_params(def),
            d,
        ) {
            if trace() {
                eprintln!("tail-self: {name} declined: {why}");
            }
            crate::rewrite_census::declined("R-TailSelf", &name, &why);
            continue;
        }
        let _census_body = crate::rewrite_census::InBody::enter("ir", &name);
        let voids = matches!(data.def(d).returned().base(), Type::Void | Type::Null);
        let params = declared_params(data.def(d));
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let scope = match &code {
            Value::Block(b) => b.scope,
            _ => 0,
        };
        let vars = &mut data.definitions[d as usize].variables;
        let temps: Vec<u16> = params
            .iter()
            .map(|&p| {
                // The temporary OWNS what it holds for the length of the jump — never a view
                // into another variable — so the parameter's borrow deps do not travel with
                // the type: a reference typed as a view gets no slot of its own.
                let tp = vars.tp(p).without_deps();
                vars.add_unique("tail", &tp, scope)
            })
            .collect();
        let n = {
            let mut cx = Cx {
                d,
                params: &params,
                temps: &temps,
                data,
                vars: data.def(d).variables(),
                put_ref: data.def_nr("OpPutRef"),
                jumps: 0,
            };
            rewrite_body(&mut code, voids, &mut cx);
            cx.jumps
        };
        data.definitions[d as usize].code = code;
        if n == 0 {
            // No jump: the finder's tail call has no counterpart in the lowered body.  The
            // loop is inert, and the census records the function so the shape gets a clause.
            if trace() {
                eprintln!("tail-self: {name}: no tail call left in the lowered body");
            }
            crate::rewrite_census::declined("R-TailSelf", &name, "lowered shape not matched");
        } else if trace() {
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

struct Cx<'a> {
    d: u32,
    params: &'a [u16],
    temps: &'a [u16],
    data: &'a Data,
    vars: &'a Function,
    /// `OpPutRef`: the raw slot store the scope pass binds a witness with — a pointer moved
    /// without a release of what the slot held, which is what a shared parameter needs.
    put_ref: u32,
    jumps: usize,
}

/// Wrap the body in the loop.
fn rewrite_body(code: &mut Value, voids: bool, cx: &mut Cx) {
    let Value::Block(b) = code else {
        return;
    };
    let ops = std::mem::take(&mut b.operators);
    let pos = if voids { Pos::Void } else { Pos::Return };
    let mut body = block(ops, Some(pos), &[], 0, cx);
    if voids {
        // A void body that reaches its end returns; only a jump stays in the loop.
        body.push(Value::Return(Box::new(Value::Null)));
    }
    b.operators = vec![v_loop(body, "tail_self")];
}

/// A block's operators.  `pos` is `Some` when the block's last value is in value position;
/// `epi` the releases an enclosing `__ret_N` pattern runs after this block yields, which a
/// jump from inside it must run first.
fn block(
    ops: Vec<Value>,
    pos: Option<Pos>,
    epi: &[Value],
    depth: usize,
    cx: &mut Cx,
) -> Vec<Value> {
    let last = last_value(&ops);
    let pat = ret_pattern(&ops, cx.vars, cx.data, pos.is_some());
    let frees: Vec<Value> = match pat {
        Some((k, j)) => {
            let mut f: Vec<Value> = ops[k + 1..j]
                .iter()
                .filter(|op| is_release(cx.data, op))
                .cloned()
                .collect();
            f.extend_from_slice(epi);
            f
        }
        None => Vec::new(),
    };
    let vtail = if pos == Some(Pos::Void) {
        void_tail(&ops, cx.data)
    } else {
        None
    };
    let trailing: Vec<Value> = match vtail {
        Some(m) => {
            let mut f: Vec<Value> = ops[m + 1..]
                .iter()
                .filter(|op| is_release(cx.data, op))
                .cloned()
                .collect();
            f.extend_from_slice(epi);
            f
        }
        None => Vec::new(),
    };
    let mut out = Vec::with_capacity(ops.len());
    for (i, op) in ops.into_iter().enumerate() {
        let op = if let Some((k, _)) = pat
            && i == k
        {
            let (span, set) = unspan_owned(op);
            let Value::Set(r, e) = set else {
                unreachable!("ret_pattern found a `Set`")
            };
            respan(
                span,
                Value::Set(r, Box::new(value(*e, Pos::Keep, &frees, depth, cx))),
            )
        } else if pat.is_some_and(|(_, j)| j == i) {
            op
        } else if Some(i) == vtail {
            value(op, Pos::Void, &trailing, depth, cx)
        } else if let Some(p) = pos
            && Some(i) == last
        {
            value(op, p, epi, depth, cx)
        } else {
            let mut op = op;
            statement(&mut op, epi, depth, cx);
            op
        };
        out.push(op);
    }
    out
}

fn unspan_owned(v: Value) -> (Option<Position>, Value) {
    match v {
        Value::Span(bx) => {
            let (p, inner) = *bx;
            (Some(p), inner)
        }
        other => (None, other),
    }
}

fn respan(span: Option<Position>, v: Value) -> Value {
    match span {
        Some(p) => Value::Span(Box::new((p, v))),
        None => v,
    }
}

/// A value in a value position: a self-call becomes the jump, an `if` or block hands the
/// position to its arms or its value, a `return` is a value position of its own (its releases
/// already ran before it, so it carries none), anything else answers as `pos` says.  The
/// fallback is right because the value position of a non-void function holds the VALUE: a
/// statement node there (`Set`, `Insert`, a void call) would make the function's value void,
/// which the parser refuses before this pass runs — so every other node is the expression the
/// function answers.  A block visited in `Return` position ends in a terminator on every
/// path, so its result is `never`, as the parser types a block that ends in `return`.
fn value(v: Value, pos: Pos, epi: &[Value], depth: usize, cx: &mut Cx) -> Value {
    match v {
        Value::Span(bx) => {
            let (p, inner) = *bx;
            Value::Span(Box::new((p, value(inner, pos, epi, depth, cx))))
        }
        Value::Call(c, args) if c == cx.d => {
            cx.jumps += 1;
            jump(args, epi, depth, cx)
        }
        Value::If(cond, then, otherwise) => Value::If(
            cond,
            Box::new(value(*then, pos, epi, depth, cx)),
            Box::new(value(*otherwise, pos, epi, depth, cx)),
        ),
        Value::Block(mut b) => {
            let ops = std::mem::take(&mut b.operators);
            b.operators = block(ops, Some(pos), epi, depth, cx);
            if pos == Pos::Return {
                b.result = Type::Never;
            }
            Value::Block(b)
        }
        Value::Return(e) => value(*e, Pos::Return, &[], depth, cx),
        mut other => {
            statement(&mut other, epi, depth, cx);
            match pos {
                Pos::Return => Value::Return(Box::new(other)),
                Pos::Keep | Pos::Void => other,
            }
        }
    }
}

/// A statement outside a value position: its explicit `return`s are value positions of their
/// own, a block below it may carry the `__ret_N` pattern, and a loop below it is one level the
/// jump has to count.
fn statement(v: &mut Value, epi: &[Value], depth: usize, cx: &mut Cx) {
    match v {
        Value::Span(bx) => statement(&mut bx.1, epi, depth, cx),
        Value::Return(e) => {
            let inner = std::mem::replace(&mut **e, Value::Null);
            *v = value(inner, Pos::Return, &[], depth, cx);
        }
        Value::Block(b) => {
            let ops = std::mem::take(&mut b.operators);
            b.operators = block(ops, None, epi, depth, cx);
        }
        Value::Loop(b) => {
            for op in &mut b.operators {
                statement(op, epi, depth + 1, cx);
            }
        }
        Value::Iter(..) => {
            v.for_each_child_mut(&mut |c| statement(c, epi, depth + 1, cx));
        }
        _ => v.for_each_child_mut(&mut |c| statement(c, epi, depth, cx)),
    }
}

/// The jump: every argument into its temporary, the frame's releases (after the reads, as
/// `(O-Detach)` asks), every parameter from its temporary — so an argument that reads a
/// parameter reads the value before any rebind — then the loop continues, by the depth of the
/// user loops between the call and the function's own loop.  A parameter passed through as
/// itself is not touched: it already holds what the call would bind.
fn jump(args: Vec<Value>, epi: &[Value], depth: usize, cx: &Cx) -> Value {
    let mut ops = Vec::with_capacity(2 * cx.params.len() + epi.len() + 1);
    let mut bound: Vec<(u16, u16, bool)> = Vec::with_capacity(cx.params.len());
    for ((&p, &t), a) in cx.params.iter().zip(cx.temps).zip(args) {
        if matches!(a.unspan(), Value::Var(q) if *q == p) {
            continue;
        }
        // The temporary's `Set` is its first write — a declaration the bytecode stores
        // without a release, and the slot assignment reads as the variable's birth.
        ops.push(v_set(t, a));
        // A `Set` on a store-held PARAMETER is an owning assignment in the bytecode (it
        // releases what the slot held), and that slot holds the caller's store: the pointer is
        // moved with the raw slot store instead, on both backends.
        bound.push((p, t, crate::data::is_dbref_slot(cx.vars.tp(p).base())));
    }
    ops.extend_from_slice(epi);
    for (p, t, pointer) in bound {
        if pointer {
            ops.push(Value::Call(cx.put_ref, vec![Value::Var(p), Value::Var(t)]));
        } else {
            ops.push(v_set(p, Value::Var(t)));
        }
    }
    ops.push(Value::Continue(depth as u16));
    v_block(ops, Type::Never, "tail_jump")
}
