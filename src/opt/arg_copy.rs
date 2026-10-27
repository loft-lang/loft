// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-F-ParamText` — a store-viewing `text` argument is copied only where the call can move it.
//!
//! The parser spells every such argument (`f(w, w.rs[0].name)`) as a copy into a work text
//! (`Parser::copy_store_text_args`), because a callee that grows the store the text was read
//! from would leave the parameter reading moved bytes (loft#2203).  Once the whole program is
//! parsed, a call that cannot write any store — not its callee at any depth, not one of its
//! arguments — cannot move the bytes either, and the copy is dropped: the argument is the
//! view again, as fast as before.  [`writes_store`] is the judge, and it answers yes for a
//! call through a fn-ref, a recursion cycle, a parallel arm and a `yield`, so a callee that
//! reaches the store through a closure keeps its copy.  It also answers yes for a callee that
//! RETURNS a built text, because its body writes the hidden `&text` buffer through a link the
//! summary reads without the callee's own variables; such a call keeps a copy it does not
//! need — correct, and one small copy per call, which the C backend pays at every call.
//!
//! `LOFT_KEEP_ARG_COPY=1` keeps every copy (the falsifier of the drop); `LOFT_TRACE_ARG_COPY=1`
//! names each call's verdict.

use crate::data::{Data, DefType, TEXT_ARG_COPY, Value};
use crate::opt::writes::writes_store;
use crate::variables::Function;
use std::collections::{HashMap, HashSet};

fn is_copy(v: &Value) -> bool {
    matches!(v.unspan(), Value::Block(b) if b.name == TEXT_ARG_COPY)
}

/// The view a copy block was made from — only while the block is still the three statements
/// the parser wrote (`work = ""; OpAppendText(work, view); work`).  The scope pass may have
/// lifted a temporary into it that is freed before the call (`__lift = parse_errors_of(u); …;
/// OpFreeRef(__lift)`): then the view's source dies first, and the copy is what holds it.
fn view_of<'a>(v: &'a Value, data: &Data) -> Option<&'a Value> {
    let Value::Block(b) = v.unspan() else {
        return None;
    };
    let [Value::Set(w, _), append, Value::Var(r)] = b.operators.as_slice() else {
        return None;
    };
    match append.unspan() {
        Value::Call(op, a)
            if w == r
                && data.def(*op).name() == "OpAppendText"
                && matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if x == w) =>
        {
            a.get(1)
        }
        _ => None,
    }
}

fn rewrite(node: &mut Value, data: &Data, vars: &Function, cache: &mut HashMap<u32, bool>) {
    node.for_each_child_mut(&mut |c| rewrite(c, data, vars, cache));
    let Value::Call(d, args) = node.unspan_mut() else {
        return;
    };
    if !args.iter().any(is_copy) {
        return;
    }
    let mut views = Vec::with_capacity(args.len());
    for a in args.iter() {
        if !is_copy(a) {
            views.push(a.clone());
        } else if let Some(v) = view_of(a, data) {
            views.push(v.clone());
        } else {
            return;
        }
    }
    let probe = Value::Call(*d, views);
    // Judged with the caller's variables, so the hidden `&text` buffer a text-returning callee
    // is handed (`OpCreateStack` of a text local) reads as the frame address it is.
    let keep = writes_store(&probe, data, cache, &mut HashSet::new(), Some(vars), None);
    if std::env::var_os("LOFT_TRACE_ARG_COPY").is_some() {
        let verdict = if keep {
            "kept: the call can write a store"
        } else {
            "dropped"
        };
        crate::loft_eprintln!("arg_copy: {} — {verdict}", data.def(*d).name());
    }
    if keep {
        return;
    }
    let Value::Call(_, views) = probe else {
        return;
    };
    for (a, v) in args.iter_mut().zip(views) {
        if is_copy(a) {
            *a = v;
        }
    }
}

/// Drop the argument copies no store write can need (`@FR-F-ParamText`).
pub fn rewrite_program(data: &mut Data) {
    if std::env::var("LOFT_KEEP_ARG_COPY").is_ok_and(|v| v != "0") {
        return;
    }
    let mut cache = HashMap::new();
    for f in 0..data.definitions() {
        let def = data.def(f);
        if !matches!(def.def_type, DefType::Function) || !def.code().any_node(&mut |n| is_copy(n)) {
            continue;
        }
        // A clone, so the callee summaries read every body — this one included — as written.
        let mut code = def.code().clone();
        rewrite(&mut code, data, &def.variables, &mut cache);
        data.definitions[f as usize].code = code;
    }
}
