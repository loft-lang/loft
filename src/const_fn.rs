// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-Const` — a literal-bodied function is a constant: a call whose result is only
//! READ answers a view of the one pre-built vector instead of building it again.
//!
//! `fn face_rows() -> vector<integer> { [0, 0, 0, 4, …] }` is a table spelled as a
//! function — the idiom the `const` diagnostics themselves prescribe for anything the
//! constant store cannot paste.  Written that way it was rebuilt on every call: a store
//! minted, 392 elements pushed, the store freed at the caller's exit, once per glyph drawn.
//! The parser gives such a function a synthetic `DefType::Constant` twin holding its
//! literal ([`crate::data::Definition::literal_const`]), which `compile::build_const_vectors`
//! and the native `emit_const_vectors` pre-build ONCE in `CONST_STORE` exactly as they build
//! a top-level `NAMES = [ … ]`; this pass then answers each call whose result only lands in
//! read positions with `OpConstRef` — the very node a top-level constant's use site emits, so
//! the bind, the index read, the length and the iteration that follow are the forms both
//! backends already emit for a constant.  The call's hidden buffer argument is left null, its
//! guarded mint removed where the buffer has no other user, and its exit free stays a no-op.
//!
//! Admitted where `use_analysis::read_only_uses` proves the result is only read: the value of
//! the single bind of a local that is only ever indexed, measured, iterated or copied into
//! another such local, or the call itself at arg 0 of a scalar getter or a projection chain
//! ending in one.  Every other position DECLINES and keeps the call — a write through the
//! local, an append, a hand-off to a user call (a by-value vector parameter is written
//! through), a store into a field or element, a return, a fn-ref, a `&` link — because the
//! constant store is write-locked and the program's copy must be its own.  A wrong decline is
//! the build the program already paid; a wrong admission would be a fault at the first write.
//!
//! `LOFT_NO_CONST_VIEW=1` keeps every call (the parser then makes no twin either);
//! `LOFT_TRACE_CONST=1` names each call admitted and each declined.

use std::collections::HashSet;

use crate::data::{Data, DefType, Value};

/// Rewrite every admitted call of a literal-bodied function in `d_nr`; a no-op under the
/// switch, and a cheap one for a body that calls no such function.
pub fn rewrite(data: &mut Data, database: &mut crate::database::Stores, d_nr: u32) {
    if !crate::keys::const_view_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    if data.def(d_nr).literal_const != u32::MAX {
        copy_from_twin(&mut code, data, database, d_nr);
    }
    let calls_one = code.any_node(
        &mut |n| matches!(n, Value::Call(f, _) if data.def(*f).literal_const != u32::MAX),
    );
    if calls_one {
        let n_vars = data.def(d_nr).variables().var_count();
        let marked = |f: u32| data.def(f).literal_const != u32::MAX;
        let uses = crate::use_analysis::read_only_uses(&code, n_vars, data, &marked, true);
        let mut cx = Cx {
            data,
            d_nr,
            admitted: &uses.calls,
            const_ref: data.def_nr("OpConstRef"),
            is_null: data.def_nr("OpRefIsNull"),
            buffers: HashSet::new(),
        };
        substitute(&mut code, &mut cx);
        if !cx.buffers.is_empty() {
            let frees = free_ops(data);
            let dead: Vec<u16> = cx
                .buffers
                .iter()
                .copied()
                .filter(|&b| !mentions_outside_guards(&code, b, cx.is_null, &frees))
                .collect();
            for b in dead {
                drop_guards(&mut code, b, cx.is_null);
            }
        }
    }
    data.definitions[d_nr as usize].code = code;
}

struct Cx<'a> {
    data: &'a Data,
    d_nr: u32,
    /// The admitted call nodes, by address (`read_only_uses`).
    admitted: &'a crate::fxhash::FxHashSet<usize>,
    const_ref: u32,
    is_null: u32,
    /// The hidden buffer variables the substituted calls were handed.
    buffers: HashSet<u16>,
}

/// Replace each admitted call in place; a declined one is named under the trace.
fn substitute(v: &mut Value, cx: &mut Cx) {
    let node = v.unspan_mut();
    let addr = std::ptr::from_ref(&*node) as usize;
    if let Value::Call(f, args) = node {
        let f = *f;
        if cx.data.def(f).literal_const != u32::MAX {
            let admitted = cx.admitted.contains(&addr);
            let buffer = match args.as_slice() {
                [only] => match only.unspan() {
                    Value::Var(b) => Some(*b),
                    _ => None,
                },
                _ => None,
            };
            if admitted && let Some(b) = buffer {
                if crate::keys::trace_const() {
                    eprintln!(
                        "[const] fn={} callee={} ADMITTED: the result is only read — a view of the constant",
                        cx.data.def(cx.d_nr).name(),
                        cx.data.def(f).name()
                    );
                }
                let k = cx.data.def(f).literal_const;
                *node = Value::Call(cx.const_ref, vec![Value::Int(k as i32)]);
                cx.buffers.insert(b);
                return;
            }
            if crate::keys::trace_const() {
                eprintln!(
                    "[const] fn={} callee={} DECLINED: {}",
                    cx.data.def(cx.d_nr).name(),
                    cx.data.def(f).name(),
                    if buffer.is_none() {
                        "the call does not take its one hidden buffer"
                    } else {
                        "the result reaches a position that is not a read"
                    }
                );
            }
        }
    }
    node.for_each_child_mut(&mut |c| substitute(c, cx));
}

/// Is `If(OpRefIsNull(b), …)` — the lazy mint guard of buffer `b` (`@FR-O-LazyBuffer`)?
fn is_guard_of(v: &Value, b: u16, is_null: u32) -> bool {
    matches!(v.unspan(), Value::If(cond, _, _)
        if matches!(cond.unspan(), Value::Call(op, args)
            if *op == is_null && matches!(args.as_slice(), [a] if matches!(a.unspan(), Value::Var(x) if *x == b))))
}

/// The ops that release a buffer, by def number: a mention inside one is not a use.
pub(crate) fn free_ops(data: &Data) -> Vec<u32> {
    [
        "OpFreeRef",
        "OpFreeRefIfDistinct",
        "OpFreeRefUnlessEntry",
        "OpFreeRecordIn",
    ]
    .iter()
    .map(|n| data.def_nr(n))
    .filter(|&d| d != u32::MAX)
    .collect()
}

/// Does anything still name `b` apart from its guards, its null inits and its frees?
fn mentions_outside_guards(v: &Value, b: u16, is_null: u32, frees: &[u32]) -> bool {
    if is_guard_of(v, b, is_null) {
        return false;
    }
    match v.unspan() {
        Value::Var(x) => *x == b,
        Value::Set(x, rhs) => {
            (*x == b && !matches!(rhs.unspan(), Value::Null))
                || mentions_outside_guards(rhs, b, is_null, frees)
        }
        Value::Call(op, _) if frees.contains(op) => false,
        other => {
            let mut hit = false;
            other.for_each_child(&mut |c| hit |= mentions_outside_guards(c, b, is_null, frees));
            hit
        }
    }
}

/// Remove every guard of `b` from every statement list of `v`.
fn drop_guards(v: &mut Value, b: u16, is_null: u32) {
    match v.unspan_mut() {
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.retain(|op| !is_guard_of(op, b, is_null));
            for op in &mut bl.operators {
                drop_guards(op, b, is_null);
            }
        }
        Value::Insert(ls) => {
            ls.retain(|op| !is_guard_of(op, b, is_null));
            for op in ls {
                drop_guards(op, b, is_null);
            }
        }
        other => other.for_each_child_mut(&mut |c| drop_guards(c, b, is_null)),
    }
}

/// A literal-bodied function's OWN body — the call sites above could not answer with a view
/// (the result is written, stored, returned) — builds its fresh vector as ONE copy of its
/// pre-built constant twin instead of one push per literal: `OpAppendVector(v, OpConstRef(k),
/// elem)`, the op `v += K` lowers to, deep-copying a text element as it always does.  The
/// twin is built from this very literal (`compile::build_const_vectors`, native
/// `emit_const_vectors`), so the elements are the same; what changes is the cost — a block
/// copy against N pushes — and the size of the emitted Rust, where a 115 000-element terrain
/// table was a 115 000-line function (loft#1697).  The literal block must be exactly the
/// parser's: the vector's declaration, an optional reservation, and statements that only build
/// its elements; anything else keeps the pushes.
fn copy_from_twin(
    code: &mut Value,
    data: &Data,
    database: &mut crate::database::Stores,
    d_nr: u32,
) {
    let crate::data::Type::Vector(content, _) = data.def(d_nr).returned().base() else {
        return;
    };
    let Some(elem) = data.vector_element_type(content, database) else {
        return;
    };
    let names =
        |d: &u32, n: &str| (*d as usize) < data.definitions.len() && data.def(*d).name() == n;
    let append = data.def_nr("OpAppendVector");
    let const_ref = data.def_nr("OpConstRef");
    let twin = data.def(d_nr).literal_const;
    let mut done = false;
    let mut visit = |v: &mut Value| {
        let Value::Block(bl) = v.unspan_mut() else {
            return;
        };
        if done || bl.name != "Vector" || bl.operators.len() < 4 {
            return;
        }
        let ops = &bl.operators;
        let Value::Set(vec, _) = ops[1].unspan() else {
            return;
        };
        let vec = *vec;
        if !matches!(ops[0].unspan(), Value::Call(d, _) if names(d, "OpDatabase"))
            || !matches!(ops[2].unspan(), Value::Call(d, _) if names(d, "OpSetInt4"))
            || !matches!(ops.last().map(Value::unspan), Some(Value::Var(x)) if *x == vec)
        {
            return;
        }
        // Every statement between builds an element of `vec`: a reservation, a fused push of a
        // literal, or a record mint group (mint, literal field sets, finish) into it.
        let body = &ops[3..ops.len() - 1];
        // A constant argument: no variable and no call but a built-in operator — what the
        // twin's extractor folds (`-5` is `OpMinSingleInt(5)`, `BASE + 1` an `OpAddInt`).
        let literal = |a: &Value| {
            !a.any_node(&mut |n| match n {
                Value::Var(_) => true,
                Value::Call(d, _) => !data.def(*d).name().starts_with("Op"),
                _ => false,
            })
        };
        let mut elems: HashSet<u16> = HashSet::new();
        let only_elements = body.iter().all(|st| match st.unspan() {
            Value::Call(d, a)
                if names(d, "OpPreAllocVector") || data.def(*d).name().starts_with("OpPush") =>
            {
                matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if *x == vec)
                    && a[1..].iter().all(literal)
            }
            Value::Set(e, rhs) => {
                let ok = matches!(rhs.unspan(), Value::Call(d, a) if names(d, "OpNewRecord")
                    && matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if *x == vec));
                if ok {
                    elems.insert(*e);
                }
                ok
            }
            Value::Call(d, a) if names(d, "OpFinishRecord") => {
                matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if *x == vec)
            }
            Value::Call(d, a) if data.def(*d).name().starts_with("OpSet") => {
                matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if elems.contains(x))
                    && a[1..].iter().all(literal)
            }
            Value::Line(_) | Value::Null => true,
            _ => false,
        });
        if !only_elements || body.is_empty() {
            return;
        }
        if crate::keys::trace_const() {
            eprintln!(
                "[const] fn={} BODY: its literal is one copy of the constant",
                data.def(d_nr).name()
            );
        }
        let copy = Value::Call(
            append,
            vec![
                Value::Var(vec),
                Value::Call(const_ref, vec![Value::Int(twin as i32)]),
                Value::Int(i32::from(elem)),
            ],
        );
        let last = bl.operators.pop().expect("the tail checked above");
        bl.operators.truncate(3);
        bl.operators.push(copy);
        bl.operators.push(last);
        done = true;
    };
    walk_mut(code, &mut visit);
}

fn walk_mut(v: &mut Value, f: &mut impl FnMut(&mut Value)) {
    f(v);
    v.for_each_child_mut(&mut |c| walk_mut(c, f));
}
