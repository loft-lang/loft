// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-TailSelf` — a SELF-call in tail position becomes a jump to the function's entry,
//! decided in the IR phase for both backends (loft-lang/plans#202 P5a and P5b).
//!
//! The function's body is wrapped in a loop.  Every self-call in TAIL position — the value a
//! `return` answers, the value an `if` arm or a block answers in that position, the last
//! statement of a void body, the value DELIVERED to the result (the `__ret_N` temporary the
//! scope pass hands to a `return`, or the hidden buffer a heap result is written into) — binds
//! its arguments to fresh temporaries, releases what the frame owns (the scope pass's own
//! release ops for the locals, the ones that would have run after the call returned), binds the
//! parameters from the temporaries, and continues the loop; every other value in that position
//! answers as before.  So a `count(n - 1, acc + 1)` that is the last thing a function does
//! takes no frame, and the recursion a compiler's loops are written as runs to any depth.  A
//! self-call anywhere else stays a call, and a tail call inside a user loop continues the
//! function's loop by the loops between.
//!
//! A scalar parameter is rebound by a copy, as a call binds it.  A parameter held in a store
//! (a record, a vector, a collection, an iterator) is a pointer the frame never owned — a call
//! SHARES it with the caller (calls.md `(F-ParamHeap)`) — so its rebind is exact when the
//! argument is such a pointer too: the parameter itself (then nothing moves), another
//! parameter, or a part of one (`p.next`, `v[i]`, a projection rooted at a parameter); a text
//! or `&` parameter passes through only as itself.  An argument that MAKES a value — a call, a
//! literal, an operator result — or names a local the jump releases would have to be handed
//! over into the frame's ownership, which a call does not do either, and a parameter the body
//! REBINDS holds a store this frame owns: both are refused.  The frame's LOCALS may be held in
//! a store (a text, a vector, a record, the compiler's own `__work_N` buffers): each is
//! declared afresh by the body, so the jump releases them where the return would have.
//!
//! A heap RESULT needs nothing of its own: the scope pass delivers it through a temporary
//! (`__ret_N`, `__ret_tail_N`) or the compiler's hidden `&text` buffer (`___acc_N`,
//! `___tret_N`), written on every arm and returned after the releases, and a record's hidden
//! `__retbuf` is the caller's to mint into.  In the loop the final arm writes the same
//! temporary or buffer the first iteration would have, so the caller reads exactly what the
//! call would have delivered; the fresh buffer a tail call hands its callee (`__ref_N`) is a
//! hidden argument the jump drops.
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
    Pos, admit_frame, declared_params, delivery, is_release, last_value, off, ret_pattern,
    tail_calls, two_step, void_tail, writes_place,
};
use crate::variables::Function;

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_TAIL_SELF").is_ok_and(|v| v != "0"))
}

/// Rewrite every admitted function of the program; answers the number of self-calls made
/// jumps, which the rewrite census records as `R-TailSelf`.
/// # Panics
/// An internal compiler error — the finder saw a self-call in tail position that the lowered
/// body shows no shape for, so the parser admitted what the pass cannot make a jump; the
/// message names the function and `LOFT_NO_TAIL_SELF=1`, which keeps the call.
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
        // The temporary holds a POINTER for the length of the jump — never a view into another
        // variable, never a container of its own — so it is typed a plain reference to the
        // parameter's type definition: declared with `null` it is a null pointer, where a
        // container-typed slot declared with `null` is a minted store nothing frees.  A scalar
        // temporary keeps the scalar's type.
        let temp_types: Vec<Type> = params
            .iter()
            .map(|&p| {
                let tp = data.def(d).variables().tp(p);
                if crate::data::is_dbref_slot(tp.base()) {
                    let def = data.type_def_nr(tp.base());
                    if def == u32::MAX {
                        tp.without_deps()
                    } else {
                        Type::Reference(def, crate::data::Deps::default())
                    }
                } else {
                    tp.without_deps()
                }
            })
            .collect();
        let vars = &mut data.definitions[d as usize].variables;
        let temps: Vec<u16> = temp_types
            .iter()
            .map(|tp| vars.add_unique("tail", tp, scope))
            .collect();
        let n = {
            let mut cx = Cx {
                d,
                params: &params,
                temps: &temps,
                data,
                vars: data.def(d).variables(),
                put_ref: data.def_nr("OpPutRef"),
                free_ref_if_distinct: data.def_nr("OpFreeRefIfDistinct"),
                null_ref: data.def_nr("OpNullRefSentinel"),
                jumps: 0,
            };
            rewrite_body(&mut code, voids, &mut cx);
            cx.jumps
        };
        data.definitions[d as usize].code = code;
        // The parser admitted this frame on the source shape, so every tail self-call the
        // finder saw there must be a jump here: a lowered shape the walk does not know would
        // leave a call that stops at `MAX_CALL_DEPTH`, which the rule forbids (@C155).  That is
        // a defect of this pass, reported as one, with the switch that restores the call.
        assert!(
            n > 0,
            "internal compiler error — `{name}` calls itself in tail position, but the lowered \
             body shows no shape (R-TailSelf) can make a jump; LOFT_NO_TAIL_SELF=1 keeps the \
             call — please report this program"
        );
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

struct Cx<'a> {
    d: u32,
    params: &'a [u16],
    temps: &'a [u16],
    data: &'a Data,
    vars: &'a Function,
    /// `OpPutRef`: the raw slot store the scope pass binds a witness with — a pointer moved
    /// without a release of what the slot held, which is what a shared parameter needs.
    put_ref: u32,
    /// `OpFreeRefIfDistinct` and `OpNullRefSentinel`: what a release that read the delivery
    /// place becomes on the jump path (`for_jump`).
    free_ref_if_distinct: u32,
    null_ref: u32,
    jumps: usize,
}

/// Wrap the body in the loop — the block's operators, or the items of the `Insert` the join
/// lowering splices around the block (its `___ret_join_N = null` runs once per pass, as it
/// would per call).
fn rewrite_body(code: &mut Value, voids: bool, cx: &mut Cx) {
    if let Value::Span(bx) = code {
        return rewrite_body(&mut bx.1, voids, cx);
    }
    let ops = match code {
        Value::Block(b) => std::mem::take(&mut b.operators),
        Value::Insert(items) => std::mem::take(items),
        _ => return,
    };
    let pos = if voids { Pos::Void } else { Pos::Return };
    let mut body = block(ops, Some(pos), &[], 0, cx);
    if voids {
        // A void body that reaches its end returns; only a jump stays in the loop.
        body.push(Value::Return(Box::new(Value::Null)));
    }
    // A pointer temporary is declared ONCE, before the loop: a `Set` on a store-held slot is
    // an owning assignment, and on a later pass the slot still holds the previous pass's
    // parameter pointer — the caller's store, which that `Set` would free.  Inside the loop
    // the temporary is only ever written by the raw slot store.
    let mut ops: Vec<Value> = cx
        .params
        .iter()
        .zip(cx.temps)
        .filter(|(p, _)| crate::data::is_dbref_slot(cx.vars.tp(**p).base()))
        .map(|(_, t)| v_set(*t, Value::Null))
        .collect();
    ops.push(v_loop(body, "tail_self"));
    match code {
        Value::Block(b) => b.operators = ops,
        Value::Insert(items) => *items = ops,
        _ => {}
    }
}

/// The releases among `ops[from..to]`, then `epi`: the epilogue a jump from inside must run.
/// `place` is the delivery place those releases may read: on the jump path the result does
/// not exist yet, so "free this unless it is the result" is "free this" (`for_jump`).
fn releases(
    ops: &[Value],
    from: usize,
    to: usize,
    epi: &[Value],
    place: Option<u16>,
    cx: &Cx,
) -> Vec<Value> {
    let mut f: Vec<Value> = ops[from..to]
        .iter()
        .filter(|op| is_release(cx.data, op))
        .map(|op| match place {
            Some(x) => for_jump(op.clone(), x, cx),
            None => op.clone(),
        })
        .collect();
    f.extend_from_slice(epi);
    f
}

/// A release as the jump runs it, where `x` is the delivery place no value has reached:
/// `OpFreeRefIfDistinct(a, x)` and `OpFreeRefOrHandUp(a, x)` ("free `a` unless it is the
/// result") free `a` when it holds a store — `OpFreeRefIfDistinct(a, null)`, since the
/// placeholder may be null and the plain free does not test for that;
/// `OpFreeRefUnlessEntry(a, x, entry)` frees `a` unless it is the entry's store; a guard
/// around one is kept.  A release that does not read `x` is unchanged.
fn for_jump(op: Value, x: u16, cx: &Cx) -> Value {
    let reads_place = |v: &Value| matches!(v.unspan(), Value::Var(y) if *y == x);
    match op {
        Value::Span(bx) => {
            let (p, inner) = *bx;
            Value::Span(Box::new((p, for_jump(inner, x, cx))))
        }
        Value::If(c, a, b) => Value::If(
            c,
            Box::new(for_jump(*a, x, cx)),
            Box::new(for_jump(*b, x, cx)),
        ),
        Value::Call(d, mut items) if items.len() >= 2 && reads_place(&items[1]) => {
            let name = cx.data.def(d).name();
            if name == "OpFreeRefIfDistinct" || name == "OpFreeRefOrHandUp" {
                items.truncate(1);
                items.push(Value::Call(cx.null_ref, Vec::new()));
                Value::Call(cx.free_ref_if_distinct, items)
            } else if name == "OpFreeRefUnlessEntry" && items.len() == 3 {
                let entry = items.pop().expect("three arguments");
                items.truncate(1);
                items.push(entry);
                Value::Call(cx.free_ref_if_distinct, items)
            } else {
                Value::Call(d, items)
            }
        }
        other => other,
    }
}

/// A block's operators.  `pos` is `Some` when the block's last value is in value position;
/// `epi` the releases an enclosing delivery runs after this block yields, which a jump from
/// inside it must run first.
fn block(
    ops: Vec<Value>,
    pos: Option<Pos>,
    epi: &[Value],
    depth: usize,
    cx: &mut Cx,
) -> Vec<Value> {
    let last = last_value(&ops);
    let deliver = delivery(&ops, cx.vars, cx.data, pos.is_some());
    let pat = if pos.is_some() && deliver.is_none() {
        ret_pattern(&ops, cx.vars, cx.data)
    } else {
        None
    };
    let frees: Vec<Value> = match (deliver, pat) {
        (Some((m, j, x)), _) => releases(&ops, m + 1, j, epi, Some(x), cx),
        (None, Some((k, j))) => {
            let Value::Set(r, _) = ops[k].unspan() else {
                unreachable!("ret_pattern found a `Set`")
            };
            releases(&ops, k + 1, j, epi, Some(*r), cx)
        }
        (None, None) => Vec::new(),
    };
    let vtail = if pos == Some(Pos::Void) {
        void_tail(&ops, cx.data)
    } else {
        None
    };
    let trailing: Vec<Value> = match vtail {
        Some(m) => releases(&ops, m + 1, ops.len(), epi, None, cx),
        None => Vec::new(),
    };
    let mut out = Vec::with_capacity(ops.len());
    for (i, op) in ops.into_iter().enumerate() {
        let op = if let Some((m, _, x)) = deliver
            && i == m
        {
            deliver_in(op, x, &frees, depth, cx)
        } else if deliver.is_some_and(|(_, j, _)| j == i) {
            // The delivery's own `return x` — or the bare `x` a value block answers, which in
            // `Return` position is what the function returns on the path that did not jump.
            if pos == Some(Pos::Return) && !matches!(op.unspan(), Value::Return(_)) {
                Value::Return(Box::new(op))
            } else {
                op
            }
        } else if let Some((k, _)) = pat
            && i == k
        {
            let (span, set) = unspan_owned(op);
            let Value::Set(r, e) = set else {
                unreachable!("ret_pattern found a `Set`")
            };
            respan(span, keep_set(r, value(*e, Pos::Keep, &frees, depth, cx)))
        } else if pat.is_some_and(|(_, j)| j == i) {
            // The pattern's own trailing `__ret_N`: the value this block answers, which in
            // `Return` position is what the function returns on the path that did not jump.
            if pos == Some(Pos::Return) {
                Value::Return(Box::new(op))
            } else {
                op
            }
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
        let ends = is_jump(&op);
        out.push(op);
        if ends {
            // A standalone jump: the block's remaining operators (the releases it already
            // carries, the `return` of the place) are unreachable.
            break;
        }
    }
    out
}

/// The statement that writes the delivery place `x`: every `x = E` — and the first half of
/// the join's two-step `tmp = E; x = tmp` — puts `E` in `Keep` position with `epi` as its
/// epilogue; anything else in it is a statement.  A standalone jump ends its block.
fn deliver_in(v: Value, x: u16, epi: &[Value], depth: usize, cx: &mut Cx) -> Value {
    match v {
        Value::Span(bx) => {
            let (p, inner) = *bx;
            Value::Span(Box::new((p, deliver_in(inner, x, epi, depth, cx))))
        }
        Value::Set(y, e) if y == x => keep_set(y, value(*e, Pos::Keep, epi, depth, cx)),
        Value::If(cond, a, b) => Value::If(
            cond,
            Box::new(deliver_in(*a, x, epi, depth, cx)),
            Box::new(deliver_in(*b, x, epi, depth, cx)),
        ),
        Value::Block(mut bl) => {
            let ops = std::mem::take(&mut bl.operators);
            let steps: Vec<bool> = (0..ops.len())
                .map(|i| two_step(&ops, i, x, cx.vars).is_some())
                .collect();
            let mut out = Vec::with_capacity(ops.len());
            for (op, step) in ops.into_iter().zip(steps) {
                let op = if step {
                    let (span, set) = unspan_owned(op);
                    let Value::Set(tmp, e) = set else {
                        unreachable!("two_step found a `Set`")
                    };
                    respan(span, keep_set(tmp, value(*e, Pos::Keep, epi, depth, cx)))
                } else if writes_place(&op, x) {
                    deliver_in(op, x, epi, depth, cx)
                } else {
                    let mut op = op;
                    statement(&mut op, epi, depth, cx);
                    op
                };
                let ends = is_jump(&op);
                out.push(op);
                if ends {
                    break;
                }
            }
            bl.operators = out;
            Value::Block(bl)
        }
        mut other => {
            statement(&mut other, epi, depth, cx);
            other
        }
    }
}

/// `x = v` in `Keep` position — unless `v` IS the jump, which then stands alone: a `Set` on a
/// store-held place is an OWNING assignment that releases what the slot holds first (for the
/// return buffer, the caller's store), and native writes a conversion around a text place's
/// value that no type fits on a block that never yields.  The operators after a standalone
/// jump are unreachable and dropped (`block`, `deliver_in`), so no read of an unwritten
/// `__ret_N` remains.
fn keep_set(x: u16, v: Value) -> Value {
    if is_jump(&v) {
        v
    } else {
        Value::Set(x, Box::new(v))
    }
}

fn is_jump(v: &Value) -> bool {
    matches!(v.unspan(), Value::Block(b) if b.name == "tail_jump")
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
    // A value with no self-call in it has nothing to jump: in `Return` position it is
    // `return v` exactly as the parser writes one, its blocks keeping their types.  Rewritten
    // into a block that returns from inside, an `Object` build lost the result type the
    // value-record readers recognise it by, and a record-returning tail-recursive function
    // fell back from its tuple to a return buffer.
    if pos == Pos::Return && !calls(&v, cx.d) {
        return if matches!(v.unspan(), Value::Return(_)) {
            v
        } else {
            Value::Return(Box::new(v))
        };
    }
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
        // The `Insert` the scope pass splices around an arm: `Finder::value` reads it as a
        // block, so it is rewritten as one (loft#2208).
        Value::Insert(items) => Value::Insert(block(items, Some(pos), epi, depth, cx)),
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
/// own, a block below it may carry a delivery, and a loop below it is one level the jump has
/// to count.
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
/// itself is not touched: it already holds what the call would bind.  A hidden argument (the
/// buffer a tail call hands its callee) has no declared parameter and is dropped.
fn jump(args: Vec<Value>, epi: &[Value], depth: usize, cx: &Cx) -> Value {
    let mut ops = Vec::with_capacity(2 * cx.params.len() + epi.len() + 1);
    let mut bound: Vec<(u16, u16, bool)> = Vec::with_capacity(cx.params.len());
    for ((&p, &t), a) in cx.params.iter().zip(cx.temps).zip(args) {
        if matches!(a.unspan(), Value::Var(q) if *q == p) {
            continue;
        }
        let pointer = crate::data::is_dbref_slot(cx.vars.tp(p).base());
        if pointer {
            // A record `Set` COPIES (`(B-Copy)`: a fresh record per pass, which nothing would
            // free) and a `Set` on a store-held slot is an owning assignment (it releases what
            // the slot holds), so a pointer travels by the raw slot store on both sides; the
            // temporary's slot was declared before the loop (`rewrite_body`).
            ops.push(Value::Call(cx.put_ref, vec![Value::Var(t), a]));
        } else {
            // The temporary's `Set` is its first write — a declaration the bytecode stores
            // without a release, and the slot assignment reads as the variable's birth.
            ops.push(v_set(t, a));
        }
        bound.push((p, t, pointer));
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

/// Does `v` call `d` anywhere?
fn calls(v: &Value, d: u32) -> bool {
    v.any_node(&mut |n| matches!(n, Value::Call(c, _) if *c == d))
}
