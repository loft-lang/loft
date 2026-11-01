// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `(N-Index)` — the locals that hold a vector's LENGTH for the rest of their function.
//!
//! `n = len(v); … v[n - 1]` reads the same element as `v[len(v) - 1]`, which `(N-Index)` trusts,
//! exactly when `n` still equals `len(v)` at the read.  That holds, on every path, when:
//!
//! - `n` is assigned once in the function, by that `len(v)`, and never handed to a `&` parameter;
//! - no statement that can run after it writes `v` — the variable, or for a field `s.v` the
//!   record `s` — nor any variable whose type views it, and no closure captures either.  What
//!   can run after it is the rest of each enclosing statement list and, for a loop around it, the
//!   whole loop: its next turn runs what stands before the assignment too.
//!
//! The answer is read off the function's PASS-1 body, which is whole when pass 2 parses the read
//! (a read can come before a write in the source and after it at run time, inside a loop).

use crate::data::{Data, Value};
use crate::parser::operators::{VecKey, vec_key};
use crate::variables::Function;

/// The `(local, variable)` pairs of `code` whose local holds the length of a vector reached
/// through that variable after its one assignment.  Pass 1 has not laid out a FIELD yet, so the
/// vector itself is named when pass 2 parses the assignment ([`settle`]).
pub(crate) fn length_locals(code: &Value, data: &Data, vars: &Function) -> Vec<(u16, u16)> {
    let mut sets: Vec<(u16, VecKey)> = Vec::new();
    code.walk(&mut |n| {
        if let Value::Set(v, rhs) = n {
            if let Value::Call(op, args) = rhs.unspan()
                && args.len() == 1
                && is_len_call(data, *op)
                && let Some(key) = vec_key(&args[0], data)
            {
                sets.push((*v, key));
            }
        }
    });
    sets.retain(|(n, key)| {
        assigned_once(code, *n)
            && !vars.is_argument(*n)
            && !vars.is_captured(*n)
            && !passed_by_reference(code, data, *n)
            && stable_after(code, data, vars, *n, base_var(key))
    });
    sets.into_iter()
        .map(|(n, key)| (n, base_var(&key)))
        .collect()
}

/// The vector a pass-2 assignment `n = len(<vector>)` names, when `n` is one of `found`'s locals
/// and the vector is reached through the variable recorded for it.
pub(crate) fn settle(code: &Value, data: &Data, found: &[(u16, u16)]) -> Option<(u16, VecKey)> {
    let Value::Set(n, rhs) = code.unspan() else {
        return None;
    };
    let Value::Call(op, args) = rhs.unspan() else {
        return None;
    };
    if args.len() != 1 || !is_len_call(data, *op) {
        return None;
    }
    let key = vec_key(&args[0], data)?;
    found
        .iter()
        .any(|(m, base)| m == n && *base == base_var(&key))
        .then_some((*n, key))
}

/// Is `op` a `len` on a collection — the free `len` or a `t_<N><Type>_len` method?
pub(crate) fn is_len_call(data: &Data, op: u32) -> bool {
    let n = data.def(op).name.as_str();
    n == "n_len" || (n.starts_with("t_") && n.ends_with("_len"))
}

/// The variable a write to the keyed vector goes through.
fn base_var(key: &VecKey) -> u16 {
    match key {
        VecKey::Var(v) | VecKey::Field(v, _, _) | VecKey::OpenField(v, _, _) => *v,
    }
}

/// Is `n` an argument of a call whose parameter is a `&` — a write the call makes?
fn passed_by_reference(code: &Value, data: &Data, n: u16) -> bool {
    code.any_node(&mut |node| {
        let Value::Call(op, args) = node else {
            return false;
        };
        let attrs = data.def(*op).attributes();
        args.iter().enumerate().any(|(i, a)| {
            matches!(a.unspan(), Value::Var(x) if *x == n)
                && attrs
                    .get(i)
                    .is_some_and(|at| matches!(at.typedef.base(), crate::data::Type::RefVar(_)))
        })
    })
}

/// Does no statement that can run after the assignment of `n` write `base` — by a
/// reassignment, or a write through `base`, a variable viewing it, or one it views?  Binding a
/// new view of it (`e = v[i]`) writes nothing.
fn stable_after(code: &Value, data: &Data, vars: &Function, n: u16, base: u16) -> bool {
    if vars.is_captured(base) {
        return false;
    }
    let mut after: Vec<&Value> = Vec::new();
    if !statements_after(code, n, &mut after) {
        return false;
    }
    let reach = |w: u16| {
        w == base || vars.tp(w).depend().contains(&base) || vars.tp(base).depend().contains(&w)
    };
    let touches = |arg: &Value| {
        let mut named = crate::fxhash::FxHashSet::default();
        crate::parser::closures::collect_vars_in(arg, &mut named);
        named.into_iter().any(reach)
    };
    let mut cache = crate::fxhash::FxHashMap::default();
    !after.iter().any(|stmt| {
        stmt.any_node(&mut |node| match node {
            Value::Set(w, _) => *w == base,
            Value::TuplePut(w, _, _) => reach(*w),
            Value::Call(op, args) => {
                let def = data.def(*op);
                let attrs = def.attributes();
                let callee = if *def.code() == Value::Null {
                    Vec::new()
                } else {
                    crate::parser::closures::callee_param_writes(*op, data, &mut cache)
                };
                args.iter().enumerate().any(|(i, a)| {
                    let writes = (i == 0 && crate::parser::op_writes_first_arg(def.name()))
                        || (i == 1 && def.name() == "OpCopyRecord")
                        || attrs.get(i).is_some_and(|at| {
                            matches!(at.typedef.base(), crate::data::Type::RefVar(_))
                        })
                        || callee.get(i).copied().unwrap_or(false);
                    writes && touches(a)
                })
            }
            Value::CallRef(..) => true,
            _ => false,
        })
    })
}

/// Collect into `after` every statement that can run after the assignment of `n` in `node`,
/// level by level up the statement lists that hold it — a loop's whole body.  `false` when the
/// assignment is not found as a statement.
fn statements_after<'a>(node: &'a Value, n: u16, after: &mut Vec<&'a Value>) -> bool {
    match node.unspan() {
        Value::Set(v, _) if *v == n => true,
        Value::Block(bl) => statements_after_in(&bl.operators, n, after),
        Value::Loop(bl) => {
            let found = statements_after_in(&bl.operators, n, after);
            if found {
                after.extend(&bl.operators);
            }
            found
        }
        Value::Insert(ops) => statements_after_in(ops, n, after),
        Value::If(_, then, els) => {
            statements_after(then, n, after) || statements_after(els, n, after)
        }
        _ => false,
    }
}

/// [`statements_after`] over one statement list.
fn statements_after_in<'a>(ops: &'a [Value], n: u16, after: &mut Vec<&'a Value>) -> bool {
    for (i, op) in ops.iter().enumerate() {
        if statements_after(op, n, after) {
            after.extend(&ops[i + 1..]);
            return true;
        }
    }
    false
}

/// Is `v` assigned exactly once in `code`?
pub(crate) fn assigned_once(code: &Value, v: u16) -> bool {
    let mut count = 0;
    code.walk(&mut |n| {
        if matches!(n, Value::Set(w, _) if *w == v) {
            count += 1;
        }
    });
    count == 1
}
