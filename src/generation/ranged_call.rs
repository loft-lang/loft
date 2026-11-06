// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-RangedCall` — integer arithmetic over a helper's parameters, made plain where a
//! guard proved the values bounded.
//!
//! `(R-Range)` never ranges a parameter or a record read by shape (@C80): a non-null integer
//! can hold the sentinel.  So the per-slot helpers of a grid walk — `nb_q(q, r, d)`'s `q + 1`,
//! `eg_index`'s `(dr * gw + dq) * 3 + slot` — keep every checked operator, and the checked
//! operators' null and overflow branches are what stop LLVM folding the chain.  This module
//! supplies the bound from the CALLER, where it can be tested once:
//!
//! - **The guarded copy.**  A counted `For` block whose loop hoists integer record scalars
//!   (`e.eg_q0`, `e.eg_w`, …, invariant over the loop by the hoist's own write set) is emitted
//!   twice behind `|x| <= G` for each of them.  In the guarded copy those field reads carry
//!   `[-G, G]`, and every fact `(R-Range)` derives from them follows: the range ends, the
//!   counters, `cq = e.eg_q0 + iq`.  A fact about a local the copy derives is kept only when
//!   every assignment of that local in the whole function lies inside the block and it never
//!   escapes by reference, so a value computed before the loop never inherits a seed.
//! - **The ranged variant.**  A call whose every integer argument (and every twin scalar
//!   input) is proven within `[-L, L]` calls `f__rg` — the same body, emitted with its integer
//!   parameters and scalar inputs seeded to `[-L, L]` (a parameter the body assigns is not
//!   seeded).  Inside it `(R-Range)` decides every operator by interval arithmetic as usual,
//!   and its own calls are routed by the same test, so the chain is ranged end to end.
//!
//! `L = 2^30` leaves room for the products a grid index takes (`(2L+1)·L·3 < 2^63`); `G =
//! 2^20` leaves `L` room for the sums a guarded copy forms before it calls.  Every value the
//! copy and the variants compute is the checked template's — the bounds only make the
//! operators provably unable to fault.  `LOFT_NO_RANGED_CALLS=1` keeps every template;
//! `LOFT_HOIST_VERIFY=1` checks each plain operator as `(R-Range)` does.

use super::range::{self, Range, Ranges};
use crate::data::{Block, Value};

use crate::opt::ranged_call::G;

impl super::Output<'_> {
    /// `@FR-R-RangedCall`'s guarded copy — for a `For` block, the run-time test and the
    /// facts its guarded copy runs under, or `None` when the copy would call no ranged
    /// variant (it is then not worth its duplicated code).
    pub(super) fn range_guard_for(&mut self, bl: &Block) -> Option<(String, Ranges)> {
        if !range::ranged_calls_enabled()
            || self.switches.range_arith_disabled
            || self.hoist_disabled
            || self.in_coroutine_body
            || bl.operators.iter().any(|op| {
                op.any_node(&mut |n| {
                    matches!(
                        n,
                        Value::Yield(_) | Value::Parallel(_) | Value::CallRef(_, _)
                    )
                })
            })
        {
            return None;
        }
        // The block's loop: the hoisted integer record scalars are its guard's leaves.
        let lp = bl.operators.iter().find_map(|op| match op.unspan() {
            Value::Loop(lp) => Some(lp.clone()),
            _ => None,
        })?;
        let hoisted = self.compute_loop_hoist(&lp);
        let mut fields: Vec<crate::opt::ranged_call::GuardField> = Vec::new();
        for (key, getter) in hoisted.scalars {
            if let Value::Call(g, args) = getter.unspan()
                && self.data.def(*g).name() == "OpGetInt"
                && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == key.0)
            {
                fields.push((key, getter.clone()));
            }
        }
        let known = self.current_ranges();
        let nn = self.nn_facts();
        let facts = crate::opt::ranged_call::GuardFacts {
            f: self.def_nr,
            known: &known,
            nn: &nn,
            walks: &self.char_walks,
        };
        let (fields, leaves, merged) =
            crate::opt::ranged_call::guard_plan(self.data, bl, fields, &facts, &mut self.rg_gains)?;
        Some((self.guard_test(&fields, &leaves)?, merged))
    }

    /// Does `d`'s ranged variant turn any operator plain that its plain body keeps checked —
    /// one of its own, under its parameters seeded `[-L, L]`, or one of a callee whose call it
    /// newly ranges and whose variant gains in turn?  A variant equal to its plain body only
    /// duplicates code: a function whose arithmetic is all in the apart variants it calls
    /// (`__apv`) has nothing for the ranges to change.  Memoised; a recursive function meets
    /// itself as no gain.
    pub(super) fn variant_gains(&mut self, d: u32, depth: u8) -> bool {
        crate::opt::ranged_call::variant_gains(self.data, d, depth, &mut self.rg_gains)
    }

    /// The guard's text: `|x| <= G` for each hoisted field read and each variable leaf.
    fn guard_test(&mut self, fields: &[((u16, i64), Value)], leaves: &[u16]) -> Option<String> {
        let mut test: Vec<String> = Vec::new();
        for (_, getter) in fields {
            let read = self.expr_string(getter).ok()?;
            test.push(format!("({read}).unsigned_abs() <= {G}u64"));
        }
        for v in leaves {
            let read = self.expr_string(&Value::Var(*v)).ok()?;
            test.push(format!("({read}).unsigned_abs() <= {G}u64"));
        }
        Some(test.join(" && "))
    }

    /// Are `d`'s integer arguments `args` all proven within `[-L, L]` under `facts` — at
    /// least one of them?
    fn args_ranged(
        &self,
        d: u32,
        args: &[Value],
        facts: &Ranges,
        nn: &std::collections::HashMap<u16, bool>,
    ) -> bool {
        crate::opt::ranged_call::args_ranged(self.data, d, args, facts, nn)
    }

    /// `@FR-R-RangedCall` — may this call of `d` take the ranged variant: the callee gains
    /// from it, every integer argument is proven within `[-L, L]`, and — for a twin call —
    /// every scalar input the twin takes is too.  Records the request; the variant is
    /// emitted after the program's functions.
    pub(super) fn ranged_call(&mut self, d: u32, args: &[Value], twin: bool, kept: bool) -> bool {
        if !range::ranged_calls_enabled()
            || self.switches.range_arith_disabled
            || self.in_coroutine_body
            || (d as usize) >= self.data.definitions.len()
            || !self.variant_gains(d, 0)
        {
            return false;
        }
        let nn = self.nn_facts();
        let facts = self.current_ranges();
        let trace = std::env::var("LOFT_TRACE_RANGED_CALL").is_ok();
        if !self.args_ranged(d, args, &facts, &nn) {
            if trace {
                let ranges: Vec<Option<Range>> = args
                    .iter()
                    .map(|a| range::range(self.data, &nn, &*facts, a, 0))
                    .collect();
                eprintln!(
                    "ranged-call: {} → {} declines: arguments {ranges:?}",
                    self.data.def(self.def_nr).name(),
                    self.data.def(d).name()
                );
            }
            return false;
        }
        if twin {
            let Some(inputs) = self.callee_inputs_of(d) else {
                return false;
            };
            let vars = self.data.def(self.def_nr).variables();
            if let Err(key) = crate::opt::ranged_call::scalar_inputs_ranged(
                self.data,
                vars,
                args,
                &inputs.scalars,
                &facts,
            ) {
                if trace {
                    eprintln!(
                        "ranged-call: {} → {} declines: scalar input {key:?} unranged",
                        self.data.def(self.def_nr).name(),
                        self.data.def(d).name()
                    );
                }
                return false;
            }
        }
        // `(R-Header)`'s kept clause — the ranged variant of the callee's `__kh` twin.
        let key = (
            self.dv_key(d) | if kept { super::kept::KH_BIT } else { 0 },
            twin,
        );
        if !self.rg_emitted.contains(&key) && !self.rg_requests.contains(&key) {
            self.rg_requests.push(key);
        }
        crate::rewrite_census::fired("R-RangedCall", 1);
        true
    }

    /// The facts a ranged variant of `d` runs under: each integer parameter in `[-L, L]`
    /// (`range_vars` drops the seed of one the body assigns), and for a twin each scalar
    /// input field of its parameter.
    pub(super) fn variant_ranges(&mut self, d: u32, twin: bool) -> Ranges {
        let seeds = match self.callee_inputs_of(d) {
            Some(inputs) if twin => {
                crate::opt::ranged_call::twin_seeds(self.data, d, &inputs.scalars)
            }
            _ => crate::opt::ranged_call::param_seeds(self.data, d),
        };
        crate::opt::ranged_call::ranges_under(self.data, d, &seeds)
    }
}
