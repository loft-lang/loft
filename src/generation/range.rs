// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-Range` — the RANGE an integer expression provably lies in, and the plain
//! operator it therefore may emit.
//!
//! C120 declined the closure of the non-sentinel proof over `+`, `-` and `*`: an overflow
//! mints the null sentinel on both backends, and a rewrite must leave every value as the
//! interpreter answers it, faults included.  It named the admissible successor — *"a
//! PROOF, not a policy: arithmetic whose operands carry ranges such that the result CANNOT
//! overflow may emit the plain operator, because no fault can occur and no value can
//! differ"* — and `(R-BoundedNest)` built it for one loop shape with a guard evaluated at
//! run time.  This module is the same proof for straight-line arithmetic, decided at
//! GENERATION time: every leaf's range is a fact of the language (a literal, a mask, a
//! text or vector length, a byte), the operators' ranges follow by interval arithmetic in
//! `i128`, and an operator whose RESULT fits the type cannot fault, so the processor's
//! operator answers exactly what the checked template would.
//!
//! ## What is ranged — and what deliberately is not
//!
//! Every fact is structural and conservative; a miss costs the checked template, never
//! correctness.  A range implies non-sentinel: `i64::MIN` is never inside one.
//!
//! - a literal;
//! - `a & lit` with a non-negative literal, when `a` is NON-SENTINEL (the mask of a null
//!   is null — `op_logical_and_int` propagates it — so the operand must be proven first;
//!   the corpus cell that pins this holds a null read from past a vector's end);
//! - `a & b`, `a | b` over two non-negative ranges; `a >> k` over a ranged `a`;
//! - `+`, `-`, `*`, negation over ranged operands whose result fits `(i64::MIN, i64::MAX]`;
//! - `/` and `%` by a ranged divisor that excludes zero (the sentinel is minted by a zero
//!   divisor and by `MIN / -1`, and a ranged dividend is never `MIN`);
//! - a text's size and a vector's length (`u32` words in the store layout), a text byte
//!   (`0..=255` by `text_byte_at_native`'s contract);
//! - `if` merges as the union, the parser's null-discharge shape as the union of its arms;
//! - a counted range's counters, when both ends are ranged;
//! - a LOCAL whose every assignment is ranged, never handed out by reference, and never a
//!   self-step (`x = x + 1` has no least fixpoint; the counter case above is the one shape
//!   read off the loop instead);
//! - a call of a ONE-EXPRESSION user function (`fn color_a(c) -> ((c & m) >> 24) & 255`),
//!   evaluated over the arguments' facts at the call site, three calls deep at most.
//!
//! NOT trusted: parameters and record/element reads (the C80 escape means a non-null
//! type can hold the sentinel), anything a callee with more than one statement answers,
//! and `<<`, `^`, a shift by a variable, a remainder by a signed range.
//!
//! `LOFT_HOIST_VERIFY=1` emits the checking form of every plain operator — the plain
//! answer beside the checked template's, compared at the operator (`ops::range_verify`).
//! `LOFT_NO_RANGE_ARITH=1` keeps every template.

use crate::data::{Data, Value};
use std::collections::HashMap;

/// An inclusive range of `i64` values, `lo > i64::MIN` (the sentinel is never inside).
pub type Range = (i64, i64);

/// A callee is evaluated over its arguments' facts at most this many calls deep.
const CALL_DEPTH: u8 = 3;

/// `u32::MAX` as `i64`: the widest a text size or a vector length can be.
const U32_MAX: i64 = u32::MAX as i64;

/// Make a range from `i128` ends, or `None` when it does not fit the type (the sentinel
/// excluded).
fn fits(lo: i128, hi: i128) -> Option<Range> {
    if lo > i128::from(i64::MIN) && hi <= i128::from(i64::MAX) && lo <= hi {
        Some((lo as i64, hi as i64))
    } else {
        None
    }
}

fn union(a: Range, b: Range) -> Range {
    (a.0.min(b.0), a.1.max(b.1))
}

/// The range of `v` under the per-function facts (`nn` from
/// [`super::non_sentinel::non_sentinel_vars`], `rv` from [`range_vars`]), or `None`.
#[must_use]
pub fn range(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &HashMap<u16, Range>,
    v: &Value,
    depth: u8,
) -> Option<Range> {
    match v.unspan() {
        Value::Int(k) => Some((i64::from(*k), i64::from(*k))),
        Value::Long(l) if *l != i64::MIN => Some((*l, *l)),
        Value::Var(nr) => rv.get(nr).copied(),
        // An `if` merge is the union of its arms — the discharge `if bool(x) x else d`
        // included, whose then-arm is x itself, so an unranged x leaves the merge unranged.
        Value::If(_, then_arm, else_arm) => {
            let then_range = range(data, nn, rv, then_arm, depth)?;
            let else_range = range(data, nn, rv, else_arm, depth)?;
            Some(union(then_range, else_range))
        }
        Value::Block(b) => b
            .operators
            .iter()
            .rev()
            .find(|op| !matches!(op, Value::Line(_)))
            .and_then(|tail| range(data, nn, rv, tail, depth)),
        Value::Return(inner) => range(data, nn, rv, inner, depth),
        Value::Call(d_nr, args) => {
            if (*d_nr as usize) >= data.definitions.len() {
                return None;
            }
            let def = data.def(*d_nr);
            op_range(data, nn, rv, def.name(), args, depth)
                .or_else(|| call_range(data, nn, rv, def, args, depth))
        }
        _ => None,
    }
}

/// The range of the native op `name` over `args` — the operator rules alone, which is
/// what the emitter asks at the op it is about to write.  `None` for a user call: that is
/// [`range`]'s business through the callee's one expression.
#[must_use]
pub fn op_range(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &HashMap<u16, Range>,
    name: &str,
    args: &[Value],
    depth: u8,
) -> Option<Range> {
    let a = |i: usize| range(data, nn, rv, &args[i], depth);
    {
        {
            match (name, args.len()) {
                // The `*Nullable` twins are the same arithmetic with a silent null on a fault
                // (`(E-Report)`): a result that provably fits has no fault to be silent about.
                ("OpAddInt" | "OpAddIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    fits(
                        i128::from(x.0) + i128::from(y.0),
                        i128::from(x.1) + i128::from(y.1),
                    )
                }
                ("OpMinInt" | "OpMinIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    fits(
                        i128::from(x.0) - i128::from(y.1),
                        i128::from(x.1) - i128::from(y.0),
                    )
                }
                ("OpMulInt" | "OpMulIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    let c = [
                        i128::from(x.0) * i128::from(y.0),
                        i128::from(x.0) * i128::from(y.1),
                        i128::from(x.1) * i128::from(y.0),
                        i128::from(x.1) * i128::from(y.1),
                    ];
                    fits(*c.iter().min()?, *c.iter().max()?)
                }
                ("OpMinSingleInt", 1) => {
                    let x = a(0)?;
                    fits(-i128::from(x.1), -i128::from(x.0))
                }
                ("OpDivInt" | "OpDivIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    if y.0 <= 0 && y.1 >= 0 {
                        return None;
                    }
                    // Truncating division is monotone in each operand while the divisor
                    // keeps its sign, so the extremes are at the corners.
                    let c = [x.0 / y.0, x.0 / y.1, x.1 / y.0, x.1 / y.1];
                    fits(i128::from(*c.iter().min()?), i128::from(*c.iter().max()?))
                }
                ("OpRemInt" | "OpRemIntNullable", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    if x.0 < 0 || y.0 <= 0 {
                        return None;
                    }
                    // Non-negative dividend, positive divisor: `0 ..= min(x.hi, y.hi - 1)`.
                    Some((0, x.1.min(y.1 - 1)))
                }
                ("OpLandInt", 2) => {
                    // A non-negative literal masks ANY non-sentinel value into `0..=lit`.
                    let masked = |m: &Value, other: &Value| match m.unspan() {
                        Value::Int(k) if *k >= 0 => {
                            (super::non_sentinel::non_sentinel(data, nn, other)
                                || range(data, nn, rv, other, depth).is_some())
                            .then_some((0, i64::from(*k)))
                        }
                        Value::Long(k) if *k >= 0 => {
                            (super::non_sentinel::non_sentinel(data, nn, other)
                                || range(data, nn, rv, other, depth).is_some())
                            .then_some((0, *k))
                        }
                        _ => None,
                    };
                    if let Some(r) =
                        masked(&args[1], &args[0]).or_else(|| masked(&args[0], &args[1]))
                    {
                        return Some(r);
                    }
                    let (x, y) = (a(0)?, a(1)?);
                    (x.0 >= 0 && y.0 >= 0).then(|| (0, x.1.min(y.1)))
                }
                ("OpLorInt", 2) => {
                    let (x, y) = (a(0)?, a(1)?);
                    if x.0 < 0 || y.0 < 0 {
                        return None;
                    }
                    // Every bit set in either lies below the higher end's top bit.
                    let top = x.1.max(y.1);
                    let hi = if top == 0 {
                        0
                    } else {
                        (1i64 << (64 - top.leading_zeros())).checked_sub(1)?
                    };
                    Some((0, hi))
                }
                ("OpSRightInt", 2) => {
                    let Value::Int(k) = args[1].unspan() else {
                        return None;
                    };
                    if !(0..64).contains(k) {
                        return None;
                    }
                    let x = a(0)?;
                    Some((x.0 >> k, x.1 >> k))
                }
                // The store layout's facts: a length word is `u32`, a byte is a byte.
                ("OpSizeText" | "OpLengthVector", 1) => Some((0, U32_MAX)),
                ("t_4text_size" | "t_4text_len" | "t_6vector_len", 1) => Some((0, U32_MAX)),
                ("t_4text_byte_at", 2) => Some((0, 255)),
                _ => None,
            }
        }
    }
}

/// The range a one-expression user function answers for THESE arguments: its body is one
/// statement (a value or a `return` of one), evaluated over a var map that carries the
/// arguments' facts at the parameters.  Anything else — a body with a second statement, a
/// native, a fn-ref — answers `None`: a range cannot be read off a body this walk does not
/// see whole.
fn call_range(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &HashMap<u16, Range>,
    def: &crate::data::Definition,
    args: &[Value],
    depth: u8,
) -> Option<Range> {
    if depth >= CALL_DEPTH {
        return None;
    }
    let Value::Block(body) = def.code() else {
        return None;
    };
    let mut stmts = body
        .operators
        .iter()
        .filter(|op| !matches!(op, Value::Line(_)));
    let tail = stmts.next()?;
    if stmts.next().is_some() {
        return None;
    }
    let attrs = def.attributes();
    if attrs.len() != args.len() {
        return None;
    }
    let vars = def.variables();
    let mut nn2: HashMap<u16, bool> = HashMap::new();
    let mut rv2: HashMap<u16, Range> = HashMap::new();
    for (at, arg) in attrs.iter().zip(args) {
        let p = vars.var(&at.name);
        if p == u16::MAX {
            return None;
        }
        if let Some(r) = range(data, nn, rv, arg, depth) {
            rv2.insert(p, r);
            nn2.insert(p, true);
        } else if super::non_sentinel::non_sentinel(data, nn, arg) {
            nn2.insert(p, true);
        }
    }
    range(data, &nn2, &rv2, tail, depth + 1)
}

/// Per-function var facts: a local is ranged iff every `Set` to it assigns a ranged
/// expression, none of them names the local itself (no self-step induction: `x = x + 1`
/// has no least fixpoint, and a var that can step itself can overflow itself), and it is
/// never writable behind the map's back ([`super::non_sentinel::collect_escapes`]).  A
/// counted range's counters are seeded from the loop's own ends when both are ranged.
///
/// Pessimistic start, re-walk to fixpoint: a var enters the map only once every input of
/// every assignment is in it, so its range never changes afterwards and the map grows
/// monotonically; it settles in at most `vars` rounds.
#[must_use]
pub fn range_vars(data: &Data, code: &Value, nn: &HashMap<u16, bool>) -> HashMap<u16, Range> {
    let mut escaped: std::collections::HashSet<u16> = std::collections::HashSet::new();
    super::non_sentinel::collect_escapes(data, code, &mut escaped);
    let mut rv: HashMap<u16, Range> = HashMap::new();
    // Counters first: their step names themselves, which the fixpoint below refuses.
    let mut counters: std::collections::HashSet<u16> = std::collections::HashSet::new();
    seed_counters(data, code, code, nn, &mut rv, &mut counters);
    loop {
        let mut round: HashMap<u16, Option<Range>> = HashMap::new();
        scan_sets(code, data, nn, &rv, &counters, &mut round);
        let mut changed = false;
        for (nr, r) in round {
            if let Some(r) = r
                && !escaped.contains(&nr)
                && !counters.contains(&nr)
                && !rv.contains_key(&nr)
            {
                rv.insert(nr, r);
                changed = true;
            }
        }
        if !changed {
            return rv;
        }
    }
}

/// One round: fold every `Set(var, expr)`'s range under the current map into `acc` —
/// the union of the assignments, or `None` once any assignment is unranged or names the
/// var itself.
fn scan_sets(
    v: &Value,
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &HashMap<u16, Range>,
    counters: &std::collections::HashSet<u16>,
    acc: &mut HashMap<u16, Option<Range>>,
) {
    if let Value::Set(nr, expr) = v.unspan()
        && !counters.contains(nr)
    {
        let self_step = expr.any_node(&mut |n| matches!(n, Value::Var(x) if x == nr));
        let r = if self_step {
            None
        } else {
            range(data, nn, rv, expr, 0)
        };
        acc.entry(*nr)
            .and_modify(|e| {
                *e = match (*e, r) {
                    (Some(a), Some(b)) => Some(union(a, b)),
                    _ => None,
                }
            })
            .or_insert(r);
    }
    v.for_each_child(&mut |c| scan_sets(c, data, nn, rv, counters, acc));
}

/// Seed every counted loop's counters from the loop's own ends: with the seed `s` of the
/// stepped counter and the end `hi` both ranged, the index runs over `s ..= hi`, the `next`
/// counter over `s ..= hi`, and the loop variable over `s(+1) ..= hi(-1)` — the single-counter
/// form seeds its index one BELOW the start, the two-counter form seeds `next` AT it.  A
/// counter whose seed or end is unranged is left out, and named in `counters` either way
/// so the fixpoint never reads its self-step.
///
/// `root` is the whole function body and `v` the node being walked.  The seed is looked for
/// in `root`, NOT in the loop: the parser emits `index = <start>` as the statement BEFORE the
/// loop, so searching the loop found nothing and every counted loop went unranged — the
/// clause read as implemented and was inert (loft#1558).  Searching the function is what
/// makes it sound rather than merely wider: `v_seed` counts EVERY non-step `Set` to that
/// counter anywhere in the function, so a counter written from a second place declines on
/// `seeds != 1` instead of taking the first value it meets.
fn seed_counters(
    data: &Data,
    root: &Value,
    v: &Value,
    nn: &HashMap<u16, bool>,
    rv: &mut HashMap<u16, Range>,
    counters: &mut std::collections::HashSet<u16>,
) {
    if let Value::Loop(lp) = v.unspan()
        && let Ok(rc) = super::hoist::range_counters(lp, data)
    {
        counters.insert(rc.index);
        counters.insert(rc.loop_var);
        if let Some(n) = rc.next {
            counters.insert(n);
        }
        let stepped = rc.next.unwrap_or(rc.index);
        // The seed: the one `Set` to the stepped counter that is not its own step.
        let mut seed: Option<Range> = None;
        let mut seeds = 0usize;
        v_seed(data, nn, rv, root, stepped, &mut seed, &mut seeds);
        if seeds == 1
            && let Some(s) = seed
            && let Some(h) = range(data, nn, rv, rc.hi, 0)
        {
            // The stepped counter reaches at most `hi` (exclusive end: the test breaks
            // at `hi <= counter`; inclusive: the stop guard breaks at `hi == index`).
            let top = h.1;
            let lo_var = if rc.next.is_some() {
                s.0
            } else {
                s.0.saturating_add(1)
            };
            let hi_var = if rc.inclusive {
                top
            } else {
                top.saturating_sub(1)
            };
            rv.insert(stepped, (s.0, top.max(s.0)));
            if rc.next.is_some() {
                rv.insert(rc.index, (lo_var, hi_var.max(lo_var)));
            }
            rv.insert(rc.loop_var, (lo_var, hi_var.max(lo_var)));
        }
    }
    v.for_each_child(&mut |c| seed_counters(data, root, c, nn, rv, counters));
}

/// Find the seed `Set` of a counter: every `Set(counter, e)` whose `e` is not the step
/// `counter + 1`.  Counts them, so a counter set from two places declines.
fn v_seed(
    data: &Data,
    nn: &HashMap<u16, bool>,
    rv: &HashMap<u16, Range>,
    v: &Value,
    counter: u16,
    seed: &mut Option<Range>,
    seeds: &mut usize,
) {
    if let Value::Set(nr, e) = v.unspan()
        && *nr == counter
    {
        let is_step = matches!(e.unspan(), Value::Call(d, a)
            if data.def(*d).name() == "OpAddInt"
                && a.len() == 2
                && matches!(a[0].unspan(), Value::Var(c) if *c == counter));
        if !is_step {
            *seeds += 1;
            *seed = range(data, nn, rv, e, 0);
        }
    }
    v.for_each_child(&mut |c| v_seed(data, nn, rv, c, counter, seed, seeds));
}

/// The plain form of an op the range proof may emit: `wrapping_*` for the four arithmetic
/// operators (the exact value, since the range says no step can overflow), `/` and `%`
/// for a division whose range excludes a zero divisor.  `None` for every other op.
#[must_use]
pub fn plain_form(op_name: &str, args: usize) -> Option<&'static str> {
    match (op_name, args) {
        ("OpAddInt" | "OpAddIntNullable", 2) => Some("wrapping_add"),
        ("OpMinInt" | "OpMinIntNullable", 2) => Some("wrapping_sub"),
        ("OpMulInt" | "OpMulIntNullable", 2) => Some("wrapping_mul"),
        ("OpMinSingleInt", 1) => Some("wrapping_neg"),
        ("OpDivInt" | "OpDivIntNullable", 2) => Some("/"),
        ("OpRemInt" | "OpRemIntNullable", 2) => Some("%"),
        _ => None,
    }
}
