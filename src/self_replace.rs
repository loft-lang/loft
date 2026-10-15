// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-SelfReplace` — a vector delivered into the buffer it already is costs nothing.
//!
//! A function that builds its result in its buffer names the vector as a view of it:
//! `runs = OpGetField(B, 0)`, the same reference as `B`.  Every return then delivers that
//! local into the buffer, `OpReplaceVector(B, runs)`, because the parser emits the delivery
//! wherever a local leaves.  When the two name the same slot the replace does nothing —
//! `Stores::vector_replace` answers the identity case without a change — but only after it
//! has asked about parse errors and read both slots.  This pass drops the replace when the
//! IR proves the two are the same reference: the view is bound once, in the body's own
//! statement list (so exactly once per call, and before every replace it can reach), and
//! after that binding neither the view nor `B` is assigned, re-pointed by an operator that writes a variable's slot, or freed.
//! One IR rewrite on the closed program, so both backends get it; `LOFT_NO_SELF_REPLACE=1`
//! is the switch.
use crate::data::{Data, DefType, Value};

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_SELF_REPLACE").is_ok_and(|v| v != "0"))
}

/// May the call `op(args)` change which record `Var(v)` names?  A variable handed to an
/// operator's `const` parameter is its SLOT (`OpDatabase(pos, tp)` writes it); a free
/// releases what its first argument names (the others are witnesses it only compares).  A
/// parameter list shorter than the call answers yes.
fn repoints(data: &Data, op: u32, args: &[Value], v: u16) -> bool {
    let def = data.def(op);
    let attrs = def.attributes();
    args.iter().enumerate().any(|(i, a)| {
        matches!(a.unspan(), Value::Var(n) if *n == v)
            && ((i == 0 && def.name().starts_with("OpFreeRef"))
                || attrs.get(i).is_none_or(|at| at.constant))
    })
}

/// Is `v` assigned, re-pointed or freed anywhere in `code`?
fn disturbed(data: &Data, code: &[Value], v: u16) -> bool {
    code.iter().any(|s| {
        s.any_node(&mut |n| match n {
            Value::Set(x, _) | Value::TuplePut(x, _, _) | Value::Iter(x, _, _, _) => *x == v,
            Value::Call(op, args) => repoints(data, *op, args, v),
            _ => false,
        })
    })
}

/// Drop the self-replaces of one body's statement list; answers how many.
fn drop_in(data: &Data, ops: &mut [Value], replace: u32, field: u32) -> usize {
    let mut views: Vec<(usize, u16, u16)> = Vec::new();
    for (k, s) in ops.iter().enumerate() {
        if let Value::Set(x, val) = s.unspan()
            && let Value::Call(f, args) = val.unspan()
            && *f == field
            && let [base, at, ..] = args.as_slice()
            && let Value::Var(b) = base.unspan()
            && matches!(at.unspan(), Value::Int(0))
        {
            views.push((k, *x, *b));
        }
    }
    let mut dropped = 0;
    for (k, x, b) in views {
        let after = &ops[k + 1..];
        if x == b {
            continue;
        }
        for s in &mut ops[k + 1..] {
            s.map_nodes(&mut |n| {
                if let Value::Call(op, args) = n.unspan()
                    && *op == replace
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(_)))
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Var(s)) if *s == x)
                {
                    *n = Value::Insert(Vec::new());
                    dropped += 1;
                }
            });
        }
    }
    dropped
}

/// Drop every proven self-replace of the program; a no-op under the switch.
pub fn rewrite_program(data: &mut Data) {
    if off() {
        return;
    }
    let (replace, field) = (data.def_nr("OpReplaceVector"), data.def_nr("OpGetField"));
    if replace == u32::MAX || field == u32::MAX {
        return;
    }
    let mut fired = 0usize;
    for d_nr in 0..data.definitions.len() {
        let d = d_nr as u32;
        if data.def_type(d) != DefType::Function || !data.def(d).is_loft_defined() {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[d_nr].code, Value::Null);
        if let Value::Block(b) = code.unspan_mut() {
            fired += drop_in(data, &mut b.operators, replace, field);
        }
        data.definitions[d_nr].code = code;
    }
    crate::rewrite_census::fired("R-SelfReplace", fired);
}
