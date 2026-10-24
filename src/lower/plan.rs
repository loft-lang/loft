// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a compiled target's decisions, as data

//! The WHOLE-PROGRAM plan (@PLN200 B3.1): the decisions a compiled target makes once per
//! program, because each asks every call site — which records return by value
//! (`@FR-R-ValueRecord`), which locals keep their fields apart (`@FR-R-Apart`), which views
//! decode in place (`@FR-R-DecodeView`) and which vectors stream (`@FR-R-StreamWalk`).  An
//! emitter computes it with [`plan_program`] before the first function and reads it while it
//! writes; the Rust writer holds it as `Output::plan`.

use crate::data::{Data, Value};
use crate::database::Stores;
use crate::lower::{apart_records, apart_value, decode_structure, decode_view, stream_walk};
use crate::opt::value_records::layout::ValueRecords;
use crate::opt::value_records::{dead_buffers, value_records};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// What a compiled target decides about the whole program.
#[derive(Default)]
pub struct ProgramPlan {
    /// The functions whose record result or record parameters travel as a tuple.
    pub value_records: ValueRecords,
    /// Every function's DEAD result buffers under [`Self::value_records`]: a loop judging a
    /// call reads the callee's body with these exempt (`HoistOwned::for_callee`).
    pub dead_by_fn: Rc<HashMap<u32, HashSet<u16>>>,
    /// `@FR-R-Apart`: the locals whose records live as separate fields.
    pub apart: apart_value::ApartPlan,
    /// `@FR-R-Apart`'s record clause.
    pub apr: apart_records::RecPlan,
    /// `@FR-R-DecodeView`: the functions that decode a byte vector in place.
    pub dv: decode_view::DvPlan,
    /// `@FR-R-DecodeView`'s structure clause, over the region [`Self::dv`] admitted.
    pub ps: decode_structure::PsPlan,
    /// `@FR-R-StreamWalk`: the vectors a caller walks once, streamed.
    pub sw: stream_walk::SwPlan,
}

/// Plan `data`'s program.  `reachable` is the set the emitter writes; `library` is a
/// library artifact's generation, whose dispatches can end in a call back to the interpreter;
/// `emit_live` is a build that ships the live tier, which keeps every call on the function a
/// flip reaches and so takes none of the twin plans.
#[must_use]
pub fn plan_program(
    data: &Data,
    stores: &Stores,
    reachable: &HashSet<u32>,
    library: bool,
    emit_live: bool,
) -> ProgramPlan {
    let mut plan = ProgramPlan {
        value_records: value_records(data, stores, library),
        ..ProgramPlan::default()
    };
    crate::rewrite_census::fired("R-ValueRecord", plan.value_records.fns.len());
    // Every function's DEAD buffers, once: a loop judging a call reads the callee's
    // body with its own dead buffers exempt (`HoistOwned::for_callee`).
    plan.dead_by_fn = Rc::new(
        (0..data.definitions.len() as u32)
            .filter(|d| !matches!(data.def(*d).code(), Value::Null))
            .map(|d| (d, dead_buffers(data, d, &plan.value_records)))
            .filter(|(_, s)| !s.is_empty())
            .collect(),
    );
    if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
        for (d, set) in plan.dead_by_fn.iter() {
            let vars = data.def(*d).variables();
            eprintln!(
                "[valuerec] {} dead buffers: {:?}",
                data.def(*d).name(),
                set.iter().map(|v| vars.name(*v)).collect::<Vec<_>>()
            );
        }
    }
    // `@FR-R-Apart` — after the value records, whose functions it leaves alone: a
    // function that answers a tuple, or takes one, has no buffer to convert into.
    let value_fns: crate::fxhash::FxHashSet<u32> = plan
        .value_records
        .fns
        .keys()
        .copied()
        .chain(plan.value_records.params.keys().copied())
        .collect();
    // A build that ships the live tier keeps every call on the function a flip
    // reaches: a twin's call would run the native body of a function flipped to the
    // interpreter, so that build has no plan.
    if !emit_live {
        plan.apart = apart_value::plan(data, stores, &value_fns);
        plan.apr = apart_records::plan(data, stores);
        plan.dv = decode_view::plan(data, stores);
        // A function both plans re-emit would need a twin of a twin.
        if plan
            .dv
            .region
            .iter()
            .any(|d| plan.apart.twins.contains_key(d))
        {
            plan.dv = decode_view::DvPlan::default();
        }
        // `@FR-R-DecodeView`'s structure clause rides on the region the view clause
        // admitted: its roots, its frame parameters and its view types.
        plan.ps = decode_structure::plan(data, stores, &plan.dv);
        // `@FR-R-StreamWalk` — a vector its caller walks once, streamed.
        plan.sw = stream_walk::plan(data, reachable);
    }
    plan
}
