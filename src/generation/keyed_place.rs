// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-InPlaceLiteral`'s KEYED clause — `h[k] = R { … }` written into the record the
//! collection claims for it.
//!
//! The plain form builds the literal in a store of its own, then `OpSetKeyed` removes the
//! record already under the key, claims a fresh one in the collection's store, deep-copies the
//! literal into it, writes the key and links it, and the temporary store is freed at the
//! function's exit — a store minted and freed per insert.  Here the literal's field values are
//! evaluated first (STAGED: a value may read the old record under that key, which the removal
//! is about to release), then `Stores::keyed_place_begin` removes and claims, the staged values
//! are written straight into the claimed record, and `Stores::keyed_place_finish` writes the
//! key and links it.  The values, the removal and the link happen in the plain form's order;
//! only the temporary store and the copy are gone.  Native only; the interpreter copies.

use crate::data::{Data, Value};

/// `LOFT_NO_KEYED_IN_PLACE=1` — every keyed literal assignment builds its temporary again.
#[must_use]
pub fn enabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_KEYED_IN_PLACE").is_ok_and(|v| v != "0"))
}

/// An admitted `OpSetKeyed(coll, { B = mint; set B.f…; B }, tp, keys…)`.
pub struct Site<'a> {
    pub coll: &'a Value,
    /// The collection type, the free-source flag masked off.
    pub tp: u16,
    pub keys: &'a [Value],
    pub buf: u16,
    /// The literal's field writes, in program order.
    pub sets: Vec<&'a Value>,
}

fn name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn is_var(v: &Value, x: u16) -> bool {
    matches!(v.unspan(), Value::Var(y) if *y == x)
}

fn mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |m| {
        if is_var(m, x) {
            n += 1;
        }
    });
    n
}

/// A key the caller's subscript can be emitted twice for (begin and finish) with no effect.
fn pure_key(v: &Value) -> bool {
    matches!(
        v.unspan(),
        Value::Var(_) | Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_)
    )
}

/// The statement `stmt` of function `d_nr`, when it is a keyed literal assignment the clause
/// admits.
pub fn site<'a>(stmt: &'a Value, data: &Data, d_nr: u32) -> Option<Site<'a>> {
    let Value::Call(d, args) = stmt.unspan() else {
        return None;
    };
    if name(data, *d) != "OpSetKeyed" {
        return None;
    }
    let [coll, value, Value::Int(tp), Value::Int(_), keys @ ..] = &args[..] else {
        return None;
    };
    // The collection is re-derived for the finish: a pure path, the same place both times.
    if super::hoist::vector_path(data, coll).is_none() || !keys.iter().all(pure_key) {
        return None;
    }
    let Value::Block(bl) = value.unspan() else {
        return None;
    };
    let ops: Vec<&Value> = bl
        .operators
        .iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let (Some(Value::Var(buf)), [body @ .., _]) = (ops.last().map(|v| v.unspan()), &ops[..]) else {
        return None;
    };
    let buf = *buf;
    let mut sets: Vec<&Value> = Vec::new();
    let mut minted = false;
    for op in body {
        match op.unspan() {
            Value::Set(v, init) if *v == buf && matches!(init.unspan(), Value::Null) => {}
            Value::Call(c, a)
                if matches!(name(data, *c), "OpDatabase" | "OpDatabaseNP")
                    && a.first().is_some_and(|x| is_var(x, buf)) =>
            {
                minted = true;
            }
            // A field write of the literal: the buffer as its base, nowhere in its values.  A
            // text field declines — its staged value can borrow the store the claim mutates.
            Value::Call(c, a)
                if name(data, *c).starts_with("OpSet")
                    && name(data, *c) != "OpSetText"
                    && a.first().is_some_and(|x| is_var(x, buf))
                    && a.iter().skip(1).all(|x| mentions(x, buf) == 0) =>
            {
                sets.push(op);
            }
            _ => return None,
        }
    }
    if !minted || sets.is_empty() {
        return None;
    }
    // The buffer is the literal's alone: outside this statement only its null declaration
    // and its frees name it, and a buffer never minted is the null those frees skip.
    let inside = mentions(stmt, buf);
    let mut allowed = 0usize;
    data.def(d_nr).code().walk(&mut |n| {
        if let Value::Call(c, a) = n
            && matches!(
                name(data, *c),
                "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRefTag"
            )
        {
            allowed += a.iter().filter(|x| is_var(x, buf)).count();
        }
    });
    if mentions(data.def(d_nr).code(), buf) != inside + allowed {
        return None;
    }
    Some(Site {
        coll,
        tp: u16::try_from(*tp & 0x7FFF).ok()?,
        keys,
        buf,
        sets,
    })
}

/// Is `v` a constant operand of a setter (an offset, a bias), which needs no staging?
#[must_use]
pub fn constant(v: &Value) -> bool {
    matches!(
        v.unspan(),
        Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_) | Value::Boolean(_)
    )
}
