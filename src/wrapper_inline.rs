// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-Wrapper` in the IR phase: a call to a standard-library function whose whole body is
//! one native op over its own parameters and constants (`len(v)` is `OpLengthVector(v)`,
//! `sqrt(x)` is `OpMathFuncFloat(9, x)`) is that op, with the caller's arguments in the
//! operand positions — so neither backend makes the call.
//!
//! The wrapper is recognised by `ir_shape::one_op_wrapper` (only the stdlib's: a user
//! function's call is observable — the live tier may flip it, its frame is on the call
//! stack — and no text parameter or result).  The call is admitted where every argument is a
//! LEAF — a variable, a literal — or a PURE VECTOR PATH (constant field projections over a
//! variable): the op may name a parameter twice or in another order, and a leaf evaluates to
//! the same value however often and in whatever order it is read.  Two more conditions, which
//! the IR needs and an emitter did not: the op answers the wrapper's kind of value (a call's
//! type is what the code generator reads), and no argument is a reference variable (the call
//! would have read through it; an operand position need not).  Anything else keeps the call.
//!
//! The pass runs last in the scope pass, so every rewrite before it sees the IR it saw before.
//! `LOFT_IR_WRAPPER=1` turns it on (opt-in while it is compared with the rustc writer's
//! clause); `LOFT_TRACE_WRAPPER=1` names each call taken, and each kept with why.
use crate::data::{Data, DefType, Type, Value};
use crate::ir_shape::{WrapperOperand, one_op_wrapper};
use crate::variables::Function;
use std::collections::HashMap;

const RULE: &str = "R-Wrapper";

fn enabled() -> bool {
    crate::env_once!(std::env::var("LOFT_IR_WRAPPER").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_WRAPPER").is_ok_and(|v| v != "0"))
}

/// Replace every admitted one-op wrapper call of function `d_nr` by its op.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !enabled() || data.def_type(d_nr) != DefType::Function || !data.def(d_nr).is_loft_defined() {
        return;
    }
    let get_field = data.def_nr("OpGetField");
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let mut cache: HashMap<u32, Option<(u32, Vec<WrapperOperand>)>> = HashMap::new();
    let mut fired = 0usize;
    {
        let data: &Data = data;
        let vars = data.def(d_nr).variables();
        code.map_nodes(&mut |n| {
            let Value::Call(d, args) = &*n else { return };
            let d = *d;
            if (d as usize) >= data.definitions.len() {
                return;
            }
            let Some((op, operands)) = cache
                .entry(d)
                .or_insert_with(|| one_op_wrapper(data, d))
                .clone()
            else {
                return;
            };
            let why = if !args.iter().all(|a| leaf(a, vars, get_field)) {
                Some("an argument is neither a leaf nor a pure vector path")
            } else if !same_kind(data.def(op).returned(), data.def(d).returned()) {
                Some("the op answers another kind of value than the wrapper")
            } else {
                None
            };
            if let Some(why) = why {
                crate::rewrite_census::declined(RULE, data.def(d).name(), why);
                if trace() {
                    eprintln!(
                        "[wrapper] fn={} keeps the call of {}: {why}",
                        data.def(d_nr).name(),
                        data.def(d).name()
                    );
                }
                return;
            }
            let mut operand_values = Vec::with_capacity(operands.len());
            for o in &operands {
                match o {
                    WrapperOperand::Param(i) => match args.get(*i as usize) {
                        Some(a) => operand_values.push(a.clone()),
                        None => return,
                    },
                    WrapperOperand::Const(c) => operand_values.push(c.clone()),
                }
            }
            if trace() {
                eprintln!(
                    "[wrapper] fn={} {} is {}",
                    data.def(d_nr).name(),
                    data.def(d).name(),
                    data.def(op).name()
                );
            }
            *n = Value::Call(op, operand_values);
            fired += 1;
        });
    }
    data.definitions[d_nr as usize].code = code;
    crate::rewrite_census::fired(RULE, fired);
}

/// A variable that is not a reference, a literal, or constant field projections over a
/// variable: a value that reads the same however often and in whatever order it is read.
fn leaf(v: &Value, vars: &Function, get_field: u32) -> bool {
    match v.unspan() {
        Value::Var(x) => !matches!(vars.tp(*x), Type::RefVar(_)),
        Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_) | Value::Boolean(_) => true,
        Value::Call(f, args) if *f == get_field => {
            matches!(args.as_slice(), [inner, Value::Int(_), Value::Int(_)]
                if matches!(inner.unspan(), Value::Var(_) | Value::Call(..)) && leaf(inner, vars, get_field))
        }
        _ => false,
    }
}

/// Do two return types answer the same kind of value (the same base type, bounds aside)?
fn same_kind(a: &Type, b: &Type) -> bool {
    std::mem::discriminant(a.base()) == std::mem::discriminant(b.base())
}
