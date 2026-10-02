// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-SingleUse`'s statement clause — a temporary assigned a pure value and read once, by
//! the next statement, is that value, decided in the IR phase for both backends.
//!
//! A comprehension binds each element to `_comp_N` and appends it in the next statement
//! (`_comp_1 = i * i + salt; OpPushInt(v, _comp_1)`): a store and a load per element.  When the
//! value is pure — operators over locals and literals, nothing that writes, calls user code or
//! faults into anything observable — and everything the next statement evaluates before the
//! read is pure too, moving the value into the read changes no order anyone can see.  The
//! temporary must be assigned once and read once in the whole function, and never captured.
//!
//! `LOFT_NO_SINGLE_USE=1` keeps every temporary (it also switches the inlined-body clause off,
//! `leaf_inline`); `LOFT_TRACE_SINGLE_USE=1` names each function and count.
use crate::data::{Data, DefType, Value};
use crate::same_read::pure;

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_SINGLE_USE").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_SINGLE_USE").is_ok_and(|v| v != "0"))
}

/// How often `v` reads and assigns `t`.
fn uses(v: &Value, t: u16, reads: &mut usize, sets: &mut usize) {
    match v {
        Value::Var(n) if *n == t => *reads += 1,
        Value::Set(n, _) if *n == t => *sets += 1,
        _ => {}
    }
    v.for_each_child(&mut |c| uses(c, t, reads, sets));
}

/// The value a single-expression block wraps, or the node itself.
fn bare(v: &Value) -> &Value {
    if let Value::Block(b) = v.unspan() {
        let ops: Vec<&Value> = b
            .operators
            .iter()
            .filter(|o| !matches!(o, Value::Line(_)))
            .collect();
        if let [only] = ops.as_slice() {
            return bare(only);
        }
    }
    v
}

/// Does `next` read `t` once, with only pure operands evaluated before the read?  `Some(true)`
/// once the read is found, `Some(false)` while still looking, `None` when an impure operand
/// precedes it or the shape is not followed.
fn read_after_pure(data: &Data, next: &Value, t: u16) -> Option<bool> {
    match next.unspan() {
        Value::Var(n) if *n == t => Some(true),
        Value::Call(_, args) => {
            for a in args {
                match read_after_pure(data, a, t) {
                    Some(true) => return Some(true),
                    Some(false) if pure(data, a) => {}
                    _ => return None,
                }
            }
            Some(false)
        }
        other => pure(data, other).then_some(false),
    }
}

fn substitute(v: &mut Value, t: u16, e: &mut Option<Value>) {
    if matches!(v, Value::Var(n) if *n == t) {
        if let Some(e) = e.take() {
            *v = e;
        }
        return;
    }
    v.for_each_child_mut(&mut |c| substitute(c, t, e));
}

fn rewrite_in(data: &Data, vars: &crate::variables::Function, code: &Value, v: &mut Value) -> usize {
    let mut n = 0;
    v.for_each_child_mut(&mut |c| n += rewrite_in(data, vars, code, c));
    let Value::Block(b) = v else {
        return n;
    };
    let mut i = 0;
    while i + 1 < b.operators.len() {
        let (t, value) = match &b.operators[i] {
            Value::Set(t, e) => (*t, bare(e).clone()),
            _ => {
                i += 1;
                continue;
            }
        };
        let (mut reads, mut sets) = (0, 0);
        uses(code, t, &mut reads, &mut sets);
        let admitted = reads == 1
            && sets == 1
            && !vars.is_captured(t)
            && vars.name(t).starts_with("_comp_")
            && pure(data, &value)
            && matches!(value.unspan(), Value::Call(..))
            && read_after_pure(data, &b.operators[i + 1], t) == Some(true);
        if admitted {
            b.operators.remove(i);
            substitute(&mut b.operators[i], t, &mut Some(value));
            n += 1;
        } else {
            i += 1;
        }
    }
    n
}

/// Move every admitted temporary into its read; answers how many.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() {
        return 0;
    }
    let mut total = 0;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        let vars = def.variables.clone();
        if !(0..vars.count()).any(|v| vars.name(v).starts_with("_comp_")) {
            continue;
        }
        let name = def.name().to_string();
        let snapshot = def.code().clone();
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let n = rewrite_in(data, &vars, &snapshot, &mut code);
        data.definitions[d as usize].code = code;
        if n > 0 {
            if trace() {
                eprintln!("single-use: {name} — {n} temporary(ies) moved into their read");
            }
            data.definitions[d as usize].variables.reset_intervals();
            crate::scopes::compute_function_intervals(data, d);
            crate::scopes::assign_function_slots(data, d);
            total += n;
        }
    }
    crate::rewrite_census::fired("R-SingleUse", total);
    total
}
