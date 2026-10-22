// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-BoundAppend` — a call result bound to a local and later appended whole is appended
//! where it is made.
//!
//! ```text
//! k = f(args);  S;  dst += k        →   b0 = len(dst);  dst += f(args);  kl = len(dst) - b0;  S'
//! ```
//!
//! where `S'` is `S` with `len(dst)` read as `b0` and `len(k)` as `kl`.  On `--native` the
//! appended call is `(R-AppendTwin)`'s shape, so `f`'s twin builds its result after `dst`'s
//! elements and neither `k`'s buffer nor the copy is needed.  cbor's `encode_map` encodes each
//! key into `k`, records its offset and length, and copies it into `keybuf`.
//!
//! Admitted, in one block, when: `k` is a local bound ONCE, to a call of a function that
//! answers a vector through its hidden buffer; the append is a later statement of the same
//! block, `dst += k` with `dst` a local or a parameter; every statement between mentions `k`
//! and `dst` (and every local whose dependencies reach `dst`) only as the operand of a length,
//! rebinds neither, and leaves nothing early (`return`, `break`, `continue`, `yield`), so the
//! append the rewrite moves forward is one the original runs; and `k` is mentioned nowhere
//! else.  Then nothing between can see that `dst` grew sooner: the only reads of it are its
//! length, which the rewrite serves from before the growth.
//!
//! `LOFT_NO_BOUND_APPEND=1` keeps the bind and the copy; `LOFT_TRACE_BOUND_APPEND=1` names
//! each rewritten append.

use crate::data::{Data, DefType, Type, Value};

fn is_len(data: &Data, d: u32) -> bool {
    matches!(data.def(d).name(), "t_6vector_len" | "OpLengthVector")
}

fn op(data: &Data, v: &Value) -> Option<(String, Vec<Value>)> {
    match v.unspan() {
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            Some((data.def(*d).name().to_string(), args.clone()))
        }
        _ => None,
    }
}

/// Every mention of a variable of `watched` in `v` is the operand of a length; no `Set` of
/// one, and nothing that leaves the block.
fn only_lengths(v: &Value, data: &Data, watched: &[u16]) -> bool {
    match v.unspan() {
        Value::Var(x) => !watched.contains(x),
        Value::Set(x, val) => !watched.contains(x) && only_lengths(val, data, watched),
        Value::Return(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Yield(_)
        | Value::CallRef(..)
        | Value::Parallel(_) => false,
        Value::Call(d, args)
            if is_len(data, *d)
                && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if watched.contains(x)) =>
        {
            true
        }
        _ => {
            let mut ok = true;
            v.for_each_child(&mut |c| ok = ok && only_lengths(c, data, watched));
            ok
        }
    }
}

/// Replace `len(k)` by `kl` and `len(dst)` by `b0` in `v`.
fn serve_lengths(v: &mut Value, data: &Data, k: u16, dst: u16, kl: u16, b0: u16) {
    if let Value::Call(d, args) = v.unspan()
        && is_len(data, *d)
        && let Some(Value::Var(x)) = args.first().map(Value::unspan)
    {
        if *x == k {
            *v = Value::Var(kl);
            return;
        }
        if *x == dst {
            *v = Value::Var(b0);
            return;
        }
    }
    v.for_each_child_mut(&mut |c| serve_lengths(c, data, k, dst, kl, b0));
}

fn mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |m| {
        if matches!(m, Value::Var(y) | Value::Set(y, _) if *y == x) {
            n += 1;
        }
    });
    n
}

/// The variables whose dependencies reach `root` — views of it — and `root` itself.
fn views(data: &Data, f: u32, root: u16) -> Vec<u16> {
    let vars = data.def(f).variables();
    let mut set = vec![root];
    loop {
        let before = set.len();
        for v in 0..vars.next_var() {
            if !set.contains(&v) && vars.tp(v).depend().iter().any(|d| set.contains(d)) {
                set.push(v);
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

/// One rewrite in `ops`, if one is admitted; answers whether it rewrote.  `whole(k)` counts
/// `k`'s mentions in the whole function as it stood before the pass.
fn rewrite_block(
    ops: &mut Vec<Value>,
    f: u32,
    data: &mut Data,
    whole: &dyn Fn(u16) -> usize,
    trace: bool,
) -> bool {
    for i in 0..ops.len() {
        let Value::Set(k, call) = ops[i].unspan() else {
            continue;
        };
        let k = *k;
        let Value::Call(callee, cargs) = call.unspan() else {
            continue;
        };
        let callee = *callee;
        let vars = data.def(f).variables();
        if vars.is_argument(k) {
            continue;
        }
        let Some(at) = data.def(callee).hidden_return_buffer_attr() else {
            continue;
        };
        if !matches!(
            data.def(callee).attributes()[at].typedef.base(),
            Type::Vector(..)
        ) || !matches!(cargs.get(at).map(Value::unspan), Some(Value::Var(_)))
        {
            continue;
        }
        // The append: `dst += k`, a later statement of this block.
        let Some(j) = (i + 1..ops.len()).find(|&j| {
            op(data, &ops[j]).is_some_and(|(n, a)| {
                n == "OpAppendVector"
                    && matches!(a.get(1).map(Value::unspan), Some(Value::Var(x)) if *x == k)
            })
        }) else {
            continue;
        };
        let Some((_, aargs)) = op(data, &ops[j]) else {
            continue;
        };
        let Some(Value::Var(dst)) = aargs.first().map(Value::unspan).cloned() else {
            continue;
        };
        if dst == k || !matches!(vars.tp(dst).base(), Type::Vector(..)) {
            continue;
        }
        let mut watched = views(data, f, dst);
        watched.push(k);
        if !ops[i + 1..j]
            .iter()
            .all(|s| only_lengths(s, data, &watched))
        {
            continue;
        }
        // `k` nowhere else: its bind, its lengths between, the append.
        let here: usize = ops[i..=j].iter().map(|s| mentions(s, k)).sum();
        if here != whole(k) {
            continue;
        }
        let tp = aargs[2].clone();
        let len_op = data.def_nr("t_6vector_len");
        let minus = data.def_nr("OpMinInt");
        let append_op = data.def_nr("OpAppendVector");
        let int_tp = data.def(len_op).returned.clone();
        let Value::Set(_, call) = ops[i].unspan().clone() else {
            continue;
        };
        let vars = &mut data.definitions[f as usize].variables;
        let b0 = vars.add_unique("ba_b0", &int_tp, u16::MAX);
        let kl = vars.add_unique("ba_kl", &int_tp, u16::MAX);
        let len_dst = Value::Call(len_op, vec![Value::Var(dst)]);
        for s in &mut ops[i + 1..j] {
            serve_lengths(s, data, k, dst, kl, b0);
        }
        ops.remove(j);
        ops[i] = Value::Set(b0, Box::new(len_dst.clone()));
        ops.insert(
            i + 1,
            Value::Call(append_op, vec![Value::Var(dst), *call, tp]),
        );
        ops.insert(
            i + 2,
            Value::Set(
                kl,
                Box::new(Value::Call(minus, vec![len_dst, Value::Var(b0)])),
            ),
        );
        crate::rewrite_census::fired("R-BoundAppend", 1);
        if trace {
            crate::loft_eprintln!(
                "[bound-append] {}: {} appended where it is made",
                data.def(f).name(),
                data.def(callee).name()
            );
        }
        return true;
    }
    false
}

/// Rewrite every block of `v`, innermost blocks included.
fn rewrite_in(v: &mut Value, f: u32, data: &mut Data, whole: &dyn Fn(u16) -> usize, trace: bool) {
    if let Value::Block(bl) | Value::Loop(bl) = v {
        let mut ops = std::mem::take(&mut bl.operators);
        while rewrite_block(&mut ops, f, data, whole, trace) {}
        for op in &mut ops {
            rewrite_in(op, f, data, whole, trace);
        }
        bl.operators = ops;
        return;
    }
    v.for_each_child_mut(&mut |c| rewrite_in(c, f, data, whole, trace));
}

/// The pass over the whole program: every function compiled in this load.
pub fn rewrite_program(data: &mut Data) {
    if !crate::keys::bound_append_enabled() {
        return;
    }
    let trace = crate::keys::trace_bound_append();
    let append = data.def_nr("OpAppendVector");
    for f in 0..data.definitions() {
        let def = data.def(f);
        if !matches!(def.def_type, DefType::Function)
            || def.variables.done
            || matches!(def.code(), Value::Null)
            || !def.name().starts_with("n_")
        {
            continue;
        }
        // Most functions append no local at all: they keep their body, and the whole-body
        // snapshot below is never taken (it was every user function's, on every compile).
        if !def.code().any_node(&mut |n| {
            matches!(n, Value::Call(d, a) if *d == append
                && matches!(a.get(1).map(Value::unspan), Some(Value::Var(_))))
        }) {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[f as usize].code, Value::Null);
        let snapshot = code.clone();
        let whole = |x: u16| mentions(&snapshot, x);
        rewrite_in(&mut code, f, data, &whole, trace);
        data.definitions[f as usize].code = code;
    }
}
