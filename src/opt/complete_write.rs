// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-CompleteWrite` in the IR phase: a mint whose literal group writes every field skips
//! the default prefill, on every backend.  The decision is made ONCE, here, and stored on the
//! function as [`crate::data::NoPrefill`]: the locals whose every `OpDatabase(v, …)` heads a
//! covering group, and the parent types whose every `OpNewRecord` does
//! (`opt::ret_adopt::complete_writes`).  The IR keeps `OpDatabase` and `OpNewRecord`, so the
//! analyses that read a mint by its name are unchanged; each backend reads the fact where it
//! writes the mint — the interpreter's code generator emits the no-prefill twin opcode
//! (`OpDatabaseNP`, `OpNewRecordNP`), rustc and the C their no-prefill mint.  Keyed by local and
//! by type, the fact stays true when a later pass moves a mint (`opt::loop_record` puts a kept
//! record's mint in a guard's arm, away from the group the proof reads).
//!
//! What the IR does spell: each record copy into a FIELD of an element such a group builds
//! (`CompleteWrites::fresh_fields`) is flagged `COPY_FRESH_DEST`.  Its destination was just
//! created, so it releases nothing there and a null source writes the field type's defaults
//! (`Stores::copy_null_into`, `@C137`) — what the prefill would have left, which a no-prefill
//! element does not hold.
//!
//! `LOFT_IR_COMPLETE_WRITE=1` turns it on while it is compared with the writers' clauses;
//! `LOFT_NO_COMPLETE_WRITE=1` turns the rule off.

use crate::data::{Data, DefType, NoPrefill, Value};
use crate::database::Stores;
use std::collections::HashMap;

/// Is the IR pass on?  While it is compared with the writers' clauses, a consumer reads the
/// stored fact only when this says so.
#[must_use]
pub fn enabled() -> bool {
    crate::env_once!(std::env::var("LOFT_IR_COMPLETE_WRITE").is_ok_and(|v| v != "0"))
        && !crate::env_once!(std::env::var("LOFT_NO_COMPLETE_WRITE").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_IR_COMPLETE_WRITE").is_ok_and(|v| v != "0"))
}

/// Decide every function's no-prefill mints and flag its fresh-field copies; answers how many
/// mint keys were stored.
pub fn rewrite_program(data: &mut Data, stores: &Stores) -> usize {
    if !enabled() {
        return 0;
    }
    let copy_record = data.def_nr("OpCopyRecord");
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
        let fact = NoPrefill {
            locals: c.db_vars.iter().copied().collect(),
            mints: c.mint_tps.iter().copied().collect(),
        };
        if fact.is_empty() && c.fresh_fields.is_empty() {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        flag_fresh(&mut code, &c.fresh_fields, copy_record);
        let def = &mut data.definitions[d as usize];
        def.code = code;
        if trace() && !fact.is_empty() {
            eprintln!(
                "[ir-complete-write] fn={} locals {:?} mints {:?} without the prefill",
                def.name, fact.locals, fact.mints
            );
        }
        total += fact.locals.len() + fact.mints.len();
        def.no_prefill = fact;
    }
    total
}

/// Flag each record copy under `v` that `fresh` names (by argument-slice address, on this very
/// IR) as a copy into a fresh destination.
fn flag_fresh(v: &mut Value, fresh: &HashMap<usize, u16>, copy_record: u32) {
    if let Value::Call(op, args) = v
        && *op == copy_record
        && fresh.contains_key(&(args.as_ptr() as usize))
        && let Some(Value::Int(raw)) = args.get_mut(2)
    {
        *raw |= i32::from(crate::keys::COPY_FRESH_DEST);
    }
    v.for_each_child_mut(&mut |ch| flag_fresh(ch, fresh, copy_record));
}
