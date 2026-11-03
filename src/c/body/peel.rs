// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-Peel` — a counted loop of a few trips with constant bounds (`for s in 0..3`) written
//! once per trip, its counters set to that trip's constant.  The C compiler folds what the
//! constant decides (a branch on the counter, a direction search the counter selects) before
//! its own late unrolling would reach it; gcc unrolls such a loop only after those passes.
//!
//! Admitted: the exclusive range, both counter forms, at most `MAX_TRIPS` trips, a start the
//! statements just before the loop seed with a constant, and a body with no `break`,
//! `continue`, yield or `par` (each copy is straight-line code, so no label is needed).  The
//! counters are left as the whole range leaves them.  `LOFT_NO_PEEL=1` writes every loop.

use super::FnWriter;
use crate::data::{Block, Value};

/// The trips a peeled loop may have: past this the copies cost more code than the fold saves.
const MAX_TRIPS: i64 = 8;

/// The code the copies may hold together, counted as IR nodes of the body times the trips.
const MAX_NODES: usize = 1200;

pub(super) struct Peel {
    /// The values the loop variable takes, in order.
    values: Vec<i64>,
    loop_var: u16,
    index: u16,
    next: Option<u16>,
}

/// The counter seeds the (at most two) statements before `ops[i]` set to a constant, when
/// `ops[i]` is a loop — the range's start lives there, outside the loop's block.
pub(super) fn seeds(ops: &[Value], i: usize) -> Vec<(u16, i64)> {
    if !matches!(ops[i].unspan(), Value::Loop(_)) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for op in ops[i.saturating_sub(2)..i].iter().rev() {
        if let Value::Set(v, x) = op.unspan() {
            match x.unspan() {
                Value::Int(k) => out.push((*v, i64::from(*k))),
                Value::Long(k) if *k != i64::MIN => out.push((*v, *k)),
                _ => {}
            }
        }
    }
    out
}

/// The range ends the (at most two) statements before `ops[i]` set to a vector's length
/// (`end = len(v)`), when `ops[i]` is a loop: `@FR-R-Base`'s counter clause reads them.
pub(super) fn range_lens(data: &crate::data::Data, ops: &[Value], i: usize) -> Vec<(u16, Value)> {
    if !matches!(ops[i].unspan(), Value::Loop(_)) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for op in ops[i.saturating_sub(2)..i].iter().rev() {
        if let Value::Set(v, x) = op.unspan()
            && let Value::Call(d, a) = x.unspan()
            && a.len() == 1
            && (*d as usize) < data.definitions.len()
            && matches!(data.def(*d).name(), "t_6vector_len" | "OpLengthVector")
        {
            out.push((*v, a[0].clone()));
        }
    }
    out
}

fn disabled() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_PEEL").is_ok_and(|v| v != "0"))
}

impl FnWriter<'_> {
    pub(super) fn peel_plan(&self, b: &Block) -> Option<Peel> {
        if disabled() || self.gen_elem.is_some() {
            return None;
        }
        let rc = crate::loop_shape::range_counters(b, self.cx.data).ok()?;
        if rc.inclusive {
            return None;
        }
        let hi = match rc.hi.unspan() {
            Value::Int(k) => i64::from(*k),
            Value::Long(k) if *k != i64::MIN => *k,
            _ => return None,
        };
        // The counter that steps: `next` in the two-counter form, the index itself otherwise.
        let counter = rc.next.unwrap_or(rc.index);
        let seed = self
            .peel_seeds
            .iter()
            .find(|(v, _)| *v == counter)
            .map(|(_, k)| *k)?;
        // The first value yielded: one past the seed in the single-counter form (it steps
        // before its test), the seed itself in the two-counter form (it tests first).
        let lo = if rc.next.is_some() {
            seed
        } else {
            seed.checked_add(1)?
        };
        let trips = hi.checked_sub(lo)?;
        if !(1..=MAX_TRIPS).contains(&trips) {
            return None;
        }
        let mut nodes = 0usize;
        let mut refused = false;
        for op in &b.operators[1..] {
            op.walk(&mut |n| {
                nodes += 1;
                refused |= matches!(
                    n,
                    Value::Break(_) | Value::Continue(_) | Value::Yield(_) | Value::Parallel(_)
                );
            });
        }
        // The counters step only in their iterator.
        let mut steps = false;
        for op in &b.operators[1..] {
            op.walk(&mut |n| {
                if let Value::Set(x, _) = n {
                    steps |= *x == rc.index || *x == rc.loop_var || Some(*x) == rc.next;
                }
            });
        }
        if refused || steps || nodes.saturating_mul(trips as usize) > MAX_NODES {
            return None;
        }
        Some(Peel {
            values: (lo..hi).collect(),
            loop_var: rc.loop_var,
            index: rc.index,
            next: rc.next,
        })
    }

    /// The copies: per trip a block setting the counters to the trip's value, then the body.
    pub(super) fn peel_loop(&mut self, b: &Block, p: &Peel, ind: usize) -> Result<(), String> {
        let (idx, var) = (self.var(p.index), self.var(p.loop_var));
        let next = p.next.map(|n| self.var(n));
        // Each copy is written as the loop body is: from the binding state the loop starts
        // with.  `bound` says which variables a statement written so far has bound, so a
        // `Set` in copy two onward read as a REBIND and took a displacing free the loop body
        // never has — of a value that borrows a call's buffer (a join of two calls, a view),
        // the store that buffer still names (157-join-witness under poison).  The body's own
        // frees are per trip already.
        let bound_at_entry = self.bound.clone();
        for &k in &p.values {
            self.bound.clone_from(&bound_at_entry);
            self.line(ind, "{");
            let set = match &next {
                Some(n) => format!(
                    "{idx} = INT64_C({k}); {var} = INT64_C({k}); {n} = INT64_C({});",
                    k + 1
                ),
                None => format!("{idx} = INT64_C({k}); {var} = INT64_C({k});"),
            };
            self.line(ind + 1, &set);
            self.stmts(&b.operators[1..], ind + 1)?;
            self.line(ind, "}");
        }
        // As the whole range leaves them: the index at the last value yielded and the counter
        // that steps one past it (`#index` itself in the single-counter form).
        let last = *p.values.last().ok_or("a peeled loop of no trips")?;
        let past = last + 1;
        match &next {
            Some(n) => self.line(
                ind,
                &format!("{idx} = INT64_C({last}); {n} = INT64_C({past});"),
            ),
            None => self.line(ind, &format!("{idx} = INT64_C({past});")),
        }
        crate::rewrite_census::fired("R-Peel", 1);
        Ok(())
    }
}
