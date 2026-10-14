// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the shape of a counted loop

//! The shape of a counted `for` loop in the IR: its counters, its stop guard and its plain
//! body.  A pure reading of the tree the parser lowered — the native generator's fill and
//! non-sentinel rewrites ask it, and so do the slot allocator and its validator
//! (`slot_alias`), which is why it sits in the core layer with the IR types (@PLN191).

use crate::data::{Block, Data, Value};

/// Is `v` a `break` — bare, or a block whose only statement is one?
pub(crate) fn is_break_block(v: &Value) -> bool {
    match v.unspan() {
        Value::Break(_) => true,
        Value::Block(b) => {
            b.operators.len() == 1 && matches!(b.operators[0].unspan(), Value::Break(_))
        }
        _ => false,
    }
}

/// The `if index == hi { break }` an INCLUSIVE range emits before its step (loft#1525) — the
/// stop decided on the value just yielded, so `hi + 1` never has to exist.  Recognised by SHAPE
/// (an equality whose arms are a break and a null), never by position alone, so an ordinary
/// leading statement is not mistaken for it.
fn is_inclusive_stop_guard(v: &Value, data: &Data) -> bool {
    let Value::If(cond, on_true, on_false) = v.unspan() else {
        return false;
    };
    if !is_break_block(on_true) || !matches!(on_false.unspan(), Value::Null) {
        return false;
    }
    // The EQUALITY is what tells this guard from the range's own break test, and the
    // distinction is load-bearing: the two-counter form is `[test, index, step, yield]`, whose
    // leading statement is ALSO an if-break over a two-argument call.  Matching on the shape
    // alone peeled that test and read the remainder as the single-counter form — a different
    // loop, silently.  The range tests with `OpLtInt`/`OpLeInt`; only the stop guard is `==`.
    matches!(
        cond.unspan(),
        Value::Call(d, args) if args.len() == 2 && data.def(*d).name() == "OpEqInt"
    )
}

/// The counters of a counted `for` loop, as the parser lowers `for v in a..b`: the loop
/// variable, the `#index` it yields, the `next` counter of the two-counter form (a
/// computed start, § V-ab; `None` in the single-counter form, P3b), whether the range is
/// inclusive, and its end operand.  ONE home — the fill rewrite reads the shape here and
/// so does the non-sentinel pass, which seeds these counters as never-null off the bound.
pub struct RangeCounters<'a> {
    pub loop_var: u16,
    pub index: u16,
    pub next: Option<u16>,
    pub inclusive: bool,
    pub hi: &'a Value,
}

/// Parse a loop's iterator — its FIRST statement — into its counters, or say which part of
/// the shape it is not.  The statement alone is read, because the counters are a fact about
/// the iterator and not about what follows it: a `for` statement is `[iterator, body]`, with
/// a filter (`for … if c`) or a `#count` step it carries more, and a comprehension
/// `[for i in a..b { e }]` is `[iterator, element value, append]`.  All of them step the
/// same counters under the same test, so all of them are counted ranges; a rewrite that
/// also reads the BODY as one block asks [`plain_for_body`] for that shape itself.
///
/// # Errors
///
/// The reason the block is not a counted range — the trace the fill rewrite prints.
pub fn range_counters<'a>(lp: &'a Block, data: &Data) -> Result<RangeCounters<'a>, String> {
    let decline = |why: &str| -> Result<RangeCounters<'a>, String> { Err(why.to_string()) };
    let Some(first) = lp.operators.first() else {
        return decline("loop has no statements");
    };
    let Value::Set(loop_var, iter) = first.unspan() else {
        return decline("first statement is not the loop variable's Set");
    };
    let Value::Block(it) = iter.unspan() else {
        return decline("iterator is not a block");
    };
    if it.name != "Iter range" {
        return decline(&format!("iterator block is `{}`, not a range", it.name));
    }
    // loft#1525 — an INCLUSIVE range decides its stop on the value just YIELDED, so its
    // iterator carries one extra leading statement: `if index == hi { break }`, emitted before
    // the step so the loop never has to represent `hi + 1`.  Peel it, because the loop under it
    // is the same fill: the guard changes when the loop ENDS, not which elements it writes.
    //
    // Safe to drop rather than carry, and `fill_hoisted` is why — it declines (returns false,
    // falling back to this very loop) whenever the span does not fit the vector, including the
    // `end >= h.len` and `u32::try_from` edges an unbounded end reaches.  So the fill applies
    // only where the guard would never have fired, and where it would fire the fallback is the
    // corrected loop.
    let ops: &[Value] = match &it.operators[..] {
        [first, rest @ ..] if rest.len() >= 3 && is_inclusive_stop_guard(first, data) => rest,
        all => all,
    };
    // The three statements and the yield, in one of the two orders.
    let (test, seed_index, step, yielded, next_var) = match ops {
        [a, b, c, d] => {
            // § V-ab: [if …; index = next; next = next + 1; index]
            let Value::Set(xi, from) = b.unspan() else {
                return decline("two-counter form: second statement is not the index Set");
            };
            let Value::Var(nx) = from.unspan() else {
                return decline("two-counter form: the index is not set from a variable");
            };
            let Value::Set(nx2, step) = c.unspan() else {
                return decline("two-counter form: third statement is not the step Set");
            };
            if nx2 != nx {
                return decline("two-counter form: the step is on another variable");
            }
            (a, *xi, step.unspan(), d, Some(*nx))
        }
        [a, b, c] => {
            // P3b: [index = index + 1; if …; index]
            let Value::Set(xi, step) = a.unspan() else {
                return decline("single-counter form: first statement is not the step Set");
            };
            (b, *xi, step.unspan(), c, None)
        }
        _ => return decline(&format!("iterator has {} statements", ops.len())),
    };
    let counter = next_var.unwrap_or(seed_index);
    let loop_var = *loop_var;
    // The step: counter = counter + 1.
    let Value::Call(sd, sargs) = step else {
        return decline("the step is not a call");
    };
    if data.def(*sd).name() != "OpAddInt"
        || sargs.len() != 2
        || !matches!(sargs[0].unspan(), Value::Var(c) if *c == counter)
        || !matches!(sargs[1].unspan(), Value::Int(1))
    {
        return decline("the step is not `counter + 1`");
    }
    // The test: if hi <cmp> counter { break } else null.
    let Value::If(cond, on_true, on_false) = test.unspan() else {
        return decline("the test is not an if");
    };
    if !is_break_block(on_true) || !matches!(on_false.unspan(), Value::Null) {
        return decline("the test does not break");
    }
    let Value::Call(cd, cargs) = cond.unspan() else {
        return decline("the test's condition is not a call");
    };
    let inclusive = match data.def(*cd).name() {
        "OpLtInt" => true,
        "OpLeInt" => false,
        _ => return decline("the test is not OpLtInt/OpLeInt"),
    };
    if cargs.len() != 2 || !matches!(cargs[1].unspan(), Value::Var(c) if *c == counter) {
        return decline("the test is not against the counter");
    }
    // The yield: the index variable.
    if !matches!(yielded.unspan(), Value::Var(y) if *y == seed_index) {
        return decline("the iterator does not yield the index");
    }
    Ok(RangeCounters {
        loop_var,
        index: seed_index,
        next: next_var,
        inclusive,
        hi: &cargs[0],
    })
}

/// The body of a PLAIN `for` statement — a loop that is exactly `[iterator, body block]`.
/// A filter (`for … if c`) and a `#count` step add statements beside the body, and a
/// comprehension has no body block at all; a rewrite that replaces or re-runs "the body"
/// as one unit (the fill, the bounded nest) is written for this shape alone, and `None`
/// keeps every other loop on its general emission.
#[must_use]
pub fn plain_for_body(lp: &Block) -> Option<&Block> {
    if lp.name != "For loop" || lp.operators.len() != 2 {
        return None;
    }
    match lp.operators[1].unspan() {
        Value::Block(body) => Some(body),
        _ => None,
    }
}
