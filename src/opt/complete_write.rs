// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-CompleteWrite` in the IR phase: a mint whose literal group writes every field skips
//! the default prefill, on every backend.  The covered mints are `opt::ret_adopt::
//! complete_writes`' (`db_vars` by local, `mint_tps` by the element mint's parent type); each
//! becomes its no-prefill twin:
//!
//! ```text
//! OpDatabase(v, tp)              →  OpDatabaseNP(v, tp)
//! OpNewRecord(data, ptp, fld)    →  OpNewRecordNP(data, ptp, fld)
//! ```
//!
//! The twins take the same operands and do the same work, minus `set_default_value`.  Runs in
//! `close_program` ahead of `opt::loop_record`, which moves a kept record's mint into a guard's
//! arm, away from the literal group the coverage proof reads.
//!
//! `LOFT_IR_COMPLETE_WRITE=1` turns it on while it is compared with the writers' clauses;
//! `LOFT_NO_COMPLETE_WRITE=1` turns the rule off.

use crate::data::{Data, DefType, Value};
use crate::database::Stores;

fn enabled() -> bool {
    crate::env_once!(std::env::var("LOFT_IR_COMPLETE_WRITE").is_ok_and(|v| v != "0"))
        && !crate::env_once!(std::env::var("LOFT_NO_COMPLETE_WRITE").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_IR_COMPLETE_WRITE").is_ok_and(|v| v != "0"))
}

/// The four operators, by definition number.
struct Ops {
    database: u32,
    database_np: u32,
    new_record: u32,
    new_record_np: u32,
}

/// Rewrite every covered mint of the program to its no-prefill twin; answers how many.
pub fn rewrite_program(data: &mut Data, stores: &Stores) -> usize {
    if !enabled() {
        return 0;
    }
    let ops = Ops {
        database: data.def_nr("OpDatabase"),
        database_np: data.def_nr("OpDatabaseNP"),
        new_record: data.def_nr("OpNewRecord"),
        new_record_np: data.def_nr("OpNewRecordNP"),
    };
    if ops.database_np == u32::MAX || ops.new_record_np == u32::MAX {
        return 0;
    }
    let mut total = 0usize;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function
            || matches!(def.code(), Value::Null)
            || def.is_operator()
            || !def.rust().is_empty()
        {
            continue;
        }
        let c = crate::opt::ret_adopt::complete_writes(data, stores, d);
        if c.db_vars.is_empty() && c.mint_tps.is_empty() {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let mut n = 0usize;
        twin(&mut code, &c, &ops, &mut n);
        data.definitions[d as usize].code = code;
        if n > 0 && trace() {
            eprintln!(
                "[ir-complete-write] fn={} {n} mint(s) without the prefill",
                data.def(d).name()
            );
        }
        total += n;
    }
    total
}

/// Rewrite the covered mints under `v` to their twins, counting them in `n`.
fn twin(v: &mut Value, c: &crate::opt::ret_adopt::CompleteWrites, ops: &Ops, n: &mut usize) {
    if let Value::Call(op, args) = v {
        if *op == ops.database
            && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if c.db_vars.contains(x))
        {
            *op = ops.database_np;
            *n += 1;
        } else if *op == ops.new_record
            && args.len() == 3
            && matches!(args.get(1).map(Value::unspan), Some(Value::Int(t))
                if u16::try_from(*t).is_ok_and(|t| c.mint_tps.contains(&t)))
        {
            *op = ops.new_record_np;
            *n += 1;
        }
    }
    v.for_each_child_mut(&mut |ch| twin(ch, c, ops, n));
}
