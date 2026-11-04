// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks
//! Whether a vector is EMPTY at a point of a statement list — the front's one answer, read by
//! every writer that would plan on it (the C's `@FR-R-PushFill` run clause reserves room for a
//! run of pushes onto a vector it knows is empty).  A slot is emptied by a statement that empties
//! it on EVERY path: its zero (`OpSetInt4(r, off, 0)`), a clear of it
//! (`OpClearVector(OpGetField(r, off, …))`), an `if` whose two arms both do, a block that does —
//! so a rewrite's output shape needs no reader of its own (`@FR-R-LoopBuffer`'s `if
//! OpRefIsNull(vdb) { mint; zero } else { clear }` is two of these).
use crate::data::{Data, Value};

fn call_args<'a>(data: &Data, v: &'a Value, name: &str) -> Option<&'a [Value]> {
    match v.unspan() {
        Value::Call(d, a)
            if (*d as usize) < data.definitions.len() && data.def(*d).name() == name =>
        {
            Some(a)
        }
        _ => None,
    }
}

fn on_slot(a: &[Value], slot: (u16, i32)) -> bool {
    matches!(
        (a.first().map(Value::unspan), a.get(1).map(Value::unspan)),
        (Some(Value::Var(r)), Some(Value::Int(off))) if (*r, *off) == slot
    )
}

/// Does statement `op` leave the vector slot `slot` (`(record variable, byte offset)`) EMPTY on
/// every path through it?
#[must_use]
pub fn empties(data: &Data, op: &Value, slot: (u16, i32)) -> bool {
    if let Some(a) = call_args(data, op, "OpSetInt4") {
        return on_slot(a, slot) && matches!(a.get(2).map(Value::unspan), Some(Value::Int(0)));
    }
    if let Some(a) = call_args(data, op, "OpClearVector") {
        return a
            .first()
            .and_then(|f| call_args(data, f, "OpGetField"))
            .is_some_and(|g| on_slot(g, slot));
    }
    match op.unspan() {
        Value::If(_, then, other) => {
            !matches!(other.unspan(), Value::Null)
                && empties(data, then, slot)
                && empties(data, other, slot)
        }
        Value::Block(bl) => block_empties(data, &bl.operators, slot),
        _ => false,
    }
}

/// A block empties the slot when its LAST statement naming the slot's record empties it.
fn block_empties(data: &Data, ops: &[Value], slot: (u16, i32)) -> bool {
    for op in ops.iter().rev() {
        if empties(data, op, slot) {
            return true;
        }
        if op.any_node(&mut |n| matches!(n, Value::Var(x) if *x == slot.0)) {
            return false;
        }
    }
    false
}

/// Whether `vector` is EMPTY where statement list `ops` ends: its slot was zeroed —
/// `OpSetInt4(r, off, 0)` for the field `OpGetField(r, off, …)`, or for the wrapper field a
/// local is bound to — and no statement after that names the vector's root.  A run onto a
/// vector that may already hold elements (an append twin's buffer, an accumulating local)
/// usually has the room, and a reservation there is a call for nothing.
pub fn known_empty(data: &Data, ops: &[Value], vector: &Value) -> bool {
    let field_of = |v: &Value| -> Option<(u16, i32)> {
        match v.unspan() {
            Value::Call(d, a)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpGetField" =>
            {
                match (a.first().map(Value::unspan), a.get(1).map(Value::unspan)) {
                    (Some(Value::Var(r)), Some(Value::Int(off))) => Some((*r, *off)),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    // The slot: the field itself, or the wrapper field a local vector is bound to.
    let (root, slot) = match vector.unspan() {
        Value::Var(local) => {
            let Some(bound) = ops.iter().find_map(|o| match o.unspan() {
                Value::Set(var, x) if var == local => field_of(x),
                _ => None,
            }) else {
                return false;
            };
            (Some(*local), bound)
        }
        other => match field_of(other) {
            Some(f) => (None, f),
            None => return false,
        },
    };
    let names = |o: &Value, var: u16| o.any_node(&mut |n| matches!(n, Value::Var(x) if *x == var));
    // A mention of the root that only reaches ANOTHER field — `OpGetField(r, off2, …)` or a
    // write `OpSet…(r, off2, …)`, `off2` not the slot — leaves the slot as it was.
    let elsewhere = |o: &Value| -> bool {
        let (mut all, mut other) = (0usize, 0usize);
        o.any_node(&mut |n| {
            if matches!(n, Value::Var(x) if *x == slot.0) {
                all += 1;
            }
            if let Value::Call(d, a) = n
                && (*d as usize) < data.definitions.len()
                && (data.def(*d).name() == "OpGetField" || data.def(*d).name().starts_with("OpSet"))
                && matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if *x == slot.0)
                && matches!(a.get(1).map(Value::unspan), Some(Value::Int(off)) if *off != slot.1)
            {
                other += 1;
            }
            false
        });
        all == other
    };
    for op in ops.iter().rev() {
        if empties(data, op, slot) {
            return true;
        }
        if let Value::Set(var, _) = op.unspan()
            && Some(*var) == root
        {
            continue;
        }
        if (names(op, slot.0) && !elsewhere(op)) || root.is_some_and(|l| names(op, l)) {
            return false;
        }
    }
    false
}
