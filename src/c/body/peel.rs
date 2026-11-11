// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-Peel` — a counted loop of a few trips with constant bounds (`for s in 0..3`) written
//! once per trip, its counters set to that trip's constant.  The C compiler folds what the
//! constant decides (a branch on the counter, a direction search the counter selects) before
//! its own late unrolling would reach it; gcc unrolls such a loop only after those passes.
//!
//! Admitted: the exclusive range, both counter forms, at most `MAX_TRIPS` trips, a start the
//! statements just before the loop seed with a constant, and a body with no inner loop, yield
//! or `par` (an inner loop would be copied whole per trip, the fold reaching none of it).
//! Each copy has its own loop label: a `continue` of the loop jumps to the end of its copy, a
//! `break` past the last copy; a `break` or `continue` of an enclosing loop names that loop as
//! the loop body did.  The counters are left as the whole range leaves them, or, after a
//! `break`, as the trip it left set them.  `LOFT_NO_PEEL=1` writes every loop.

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

/// One range end [`range_lens`] found: the end variable, the vector it is the length of,
/// whether it came through a length local, and the local it was copied from.
pub(super) struct RangeLen {
    pub end: u16,
    pub vec: Value,
    pub local: bool,
    pub via: Option<u16>,
}

/// The range ends the (at most two) statements before `ops[i]` set to a vector's length
/// (`end = len(v)`, or `end = n` for a length local `n`, marked `true`), when `ops[i]` is a
/// loop, each with the local `n` the end was copied from (`None` for `end = len(v)`):
/// `@FR-R-Base`'s counter clause reads them, and the loop body may not assign that `n`.
pub(super) fn range_lens(
    data: &crate::data::Data,
    ops: &[Value],
    i: usize,
    alias: Option<&(u16, Value)>,
    locals: &[(u16, u16)],
) -> Vec<RangeLen> {
    if !matches!(ops[i].unspan(), Value::Loop(_)) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for op in ops[i.saturating_sub(2)..i].iter().rev() {
        if let Some((end, v)) = len_bind(data, op) {
            out.push(RangeLen {
                end,
                vec: v,
                local: false,
                via: None,
            });
        } else if let Value::Set(end, x) = op.unspan()
            && let Value::Var(n) = x.unspan()
        {
            // `end = n` where the statement before the `for` block was `n = len(v)`; the
            // loop body may not assign `n` (`counters_in_range` checks).
            if let Some((an, v)) = alias
                && n == an
            {
                out.push(RangeLen {
                    end: *end,
                    vec: v.clone(),
                    local: false,
                    via: Some(*n),
                });
            }
            // `end = n` where `n` holds `len(v)` for the rest of the function (`length_locals`):
            // assigned once, and nothing that can run after it writes `v` — one entry for each
            // vector it is the length of, its lockstep twins among them, beside the one above.
            for (_, v) in locals.iter().filter(|(m, _)| m == n) {
                let vec = Value::Var(*v);
                if !out.iter().any(|r| r.end == *end && r.vec == vec) {
                    out.push(RangeLen {
                        end: *end,
                        vec,
                        local: true,
                        via: Some(*n),
                    });
                }
            }
        }
    }
    out
}

/// The values the (at most two) statements before `ops[i]` give variables, when `ops[i]` is a
/// loop — a range's end among them (`@FR-R-Base`'s grid clause reads it).
pub(super) fn range_ends(ops: &[Value], i: usize) -> Vec<(u16, Value)> {
    if !matches!(ops[i].unspan(), Value::Loop(_)) {
        return Vec::new();
    }
    ops[i.saturating_sub(2)..i]
        .iter()
        .filter_map(|op| match op.unspan() {
            Value::Set(v, x) => Some((*v, (**x).clone())),
            _ => None,
        })
        .collect()
}

/// `n = len(v)`: the variable and the vector.
pub(super) fn len_bind(data: &crate::data::Data, op: &Value) -> Option<(u16, Value)> {
    if let Value::Set(v, x) = op.unspan()
        && let Value::Call(d, a) = x.unspan()
        && a.len() == 1
        && (*d as usize) < data.definitions.len()
        && matches!(data.def(*d).name(), "t_6vector_len" | "OpLengthVector")
    {
        return Some((*v, a[0].clone()));
    }
    None
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
                refused |= matches!(n, Value::Loop(_) | Value::Yield(_) | Value::Parallel(_));
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
    /// Copy `k` is a loop of its own to the body's `break` and `continue`: its label `m` is on
    /// the loop stack while it is written, `cont_m` ends the copy and `brk_m` follows the
    /// last copy, after the counters the whole range leaves — so a `break` keeps the trip's.
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
        let mut labels = Vec::with_capacity(p.values.len());
        for &k in &p.values {
            self.bound.clone_from(&bound_at_entry);
            self.labels += 1;
            let m = self.labels;
            labels.push(m);
            self.loops.push(m);
            self.line(ind, "{");
            let set = match &next {
                Some(n) => format!(
                    "{idx} = INT64_C({k}); {var} = INT64_C({k}); {n} = INT64_C({});",
                    k + 1
                ),
                None => format!("{idx} = INT64_C({k}); {var} = INT64_C({k});"),
            };
            self.line(ind + 1, &set);
            let r = self.stmts(&b.operators[1..], ind + 1);
            self.loops.pop();
            r?;
            self.line(ind + 1, &format!("cont_{m}:;"));
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
        for m in labels {
            self.line(ind, &format!("brk_{m}:;"));
        }
        crate::rewrite_census::fired("R-Peel", 1);
        Ok(())
    }
}
