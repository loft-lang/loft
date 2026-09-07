// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! @PLN157 P3 (loft#1426 M3) — the "provably non-sentinel" fact, and the float
//! compares it lets the emitter simplify.
//!
//! loft's float null is NaN, and loft's `<`/`<=`/`==` define a total order in
//! which null sorts below every number — so the `#rust` templates expand each
//! compare into a NaN-aware form (`(a.is_nan() && !b.is_nan()) || …`).  That
//! expansion is Rust the LLVM layer cannot reduce: the `is_nan` branches
//! survive `-O` in every inner loop (~6 per pixel in the drawing pass) and
//! their bulk makes LLVM decline the wider rewrites the plain form would get.
//! When BOTH operands provably cannot be NaN, the plain Rust operator computes
//! the same boolean, so the emitter may use it.
//!
//! ## What "provably" means here — and what it deliberately excludes
//!
//! A fact is structural and conservative; every miss costs the template form,
//! never correctness:
//!
//! - a non-NaN float/single LITERAL;
//! - the parser's own null-discharge shape (`x ?? d`, and `x?`, both lower to
//!   `if OpConvBoolFromFloat(t) t else d`): the then-arm is guarded by the
//!   very condition, so the whole expression is non-sentinel when the
//!   ELSE-arm is;
//! - unary negation (`OpMinSingleFloat` / `OpMinSingle`): `-x` is NaN only
//!   when `x` is;
//! - an `if` merge whose arm values are both non-sentinel;
//! - a LOCAL whose every assignment in the function body is non-sentinel and
//!   which is never handed to a call as a bare argument (fixpoint below).
//!
//! NOT trusted, on purpose: parameters and record/element reads (the C80
//! escape means a non-null TYPE can still hold the sentinel), and — the float
//! twist — ARITHMETIC.  `+`/`-`/`*` over non-NaN operands can produce NaN
//! through the infinities (`inf - inf`, `inf * 0`), so unlike the integer
//! family the closure over arithmetic is unsound and is not taken.
//!
//! ## The verification instrument
//!
//! `LOFT_NN_VERIFY=1` at generation time emits the CHECKING form of every
//! simplified compare — evaluate both operands once, `assert!` neither is NaN
//! (naming the op), then compare plain.  A wrong proof then panics at the
//! exact site instead of answering a differently-ordered boolean; run the
//! suite and the drawing pass under it after every widening of the fact.
//! `LOFT_NO_NN_FAST=1` emits the template form everywhere — the bisect
//! switch, same contract as `LOFT_NO_VECTOR_HOIST`.

use crate::data::{Data, Value};
use std::collections::HashMap;

/// The compare ops this pass may simplify, with the plain Rust operator that
/// is equivalent when both operands are non-NaN.
#[must_use]
pub fn plain_float_compare(op_name: &str) -> Option<&'static str> {
    match op_name {
        "OpEqFloat" | "OpEqSingle" => Some("=="),
        "OpNeFloat" | "OpNeSingle" => Some("!="),
        "OpLtFloat" | "OpLtSingle" => Some("<"),
        "OpLeFloat" | "OpLeSingle" => Some("<="),
        _ => None,
    }
}

/// Is `v` provably non-sentinel — non-NaN as a float, non-`i64::MIN` as an
/// integer — given the per-function var facts from [`non_sentinel_vars`]?
/// The two families' shapes are disjoint by op name, so one predicate serves
/// both; the closures differ where the sentinels' arithmetic does.
#[must_use]
pub fn non_sentinel(data: &Data, vars: &HashMap<u16, bool>, v: &Value) -> bool {
    match v.unspan() {
        Value::Float(f) => !f.is_nan(),
        Value::Single(s) => !s.is_nan(),
        // A `Value::Int` is an i32 payload, which can never widen to
        // `i64::MIN`; a `Value::Long` carries the sentinel only literally.
        Value::Int(_) => true,
        Value::Long(l) => *l != i64::MIN,
        Value::Var(nr) => vars.get(nr).copied().unwrap_or(false),
        Value::If(c, t, e) => {
            if discharge_guard(data, c, t) {
                non_sentinel(data, vars, e)
            } else {
                non_sentinel(data, vars, t) && non_sentinel(data, vars, e)
            }
        }
        Value::Block(b) => b
            .operators
            .last()
            .is_some_and(|tail| non_sentinel(data, vars, tail)),
        Value::Call(d_nr, args) => match data.def(*d_nr).name() {
            // Unary negation, both families: NaN in NaN out; and for i64,
            // `-x` can only be `MIN` when `x` is (checked_neg minted MIN is
            // the excluded `x == MIN` case itself).
            "OpMinSingleFloat" | "OpMinSingle" | "OpMinSingleInt" if args.len() == 1 => {
                non_sentinel(data, vars, &args[0])
            }
            // `a & lit` with a non-negative literal: the result lies in
            // [0, lit] whatever the proven operand holds, so it cannot be
            // the sentinel.  (Proven & proven does NOT prove: two negative
            // non-sentinel values can AND to exactly `i64::MIN`.)
            "OpLandInt" if args.len() == 2 => {
                (non_negative_literal(&args[1]) && non_sentinel(data, vars, &args[0]))
                    || (non_negative_literal(&args[0]) && non_sentinel(data, vars, &args[1]))
            }
            // `a >> k` for a literal k in 1..=63 halves the magnitude at
            // least once, so the result can never reach `i64::MIN`.
            "OpSRightInt" if args.len() == 2 => {
                matches!(args[1].unspan(), Value::Int(k) if (1..64).contains(k))
                    && non_sentinel(data, vars, &args[0])
            }
            // NOT closed, deliberately: `+`/`-`/`*` mint the sentinel on
            // overflow (C85's decided edge — the REWRITE to `_nn` keeps that
            // via checked_*, but the RESULT cannot be trusted onward), `^`
            // and `|` can compose it bitwise, `/`/`%` mint it on zero.
            _ => false,
        },
        _ => false,
    }
}

/// The parser's null-discharge shape: `if OpConvBoolFrom*(x) x else d` —
/// true when the condition tests exactly the then-arm, so inside the then
/// branch the value is proven non-null by the test itself.  (`x ?? d` and
/// `x?` both lower to this; see the module doc.)
fn discharge_guard(data: &Data, cond: &Value, then: &Value) -> bool {
    let Value::Call(c_nr, c_args) = cond.unspan() else {
        return false;
    };
    if !matches!(
        data.def(*c_nr).name(),
        "OpConvBoolFromFloat" | "OpConvBoolFromInt"
    ) || c_args.len() != 1
    {
        return false;
    }
    match (c_args[0].unspan(), then.unspan()) {
        (Value::Var(a), Value::Var(b)) => a == b,
        _ => false,
    }
}

/// A literal the emitter can bound from above and below: `Int(n)`/`Long(n)`
/// with `n >= 0`.
fn non_negative_literal(v: &Value) -> bool {
    match v.unspan() {
        Value::Int(n) => *n >= 0,
        Value::Long(l) => *l >= 0,
        _ => false,
    }
}

/// Per-function var facts: a local is non-sentinel iff it has at least one
/// assignment, EVERY `Set` to it assigns a non-sentinel expression, and it is never
/// writable behind the map's back — handed to a call through a `RefVar`
/// parameter, a `TuplePut` destination, or an `Iter` variable.  (The
/// parser's write analysis is a deny-list this pass deliberately does not
/// lean on.)
///
/// NO self-step induction, deliberately.  An earlier cut admitted
/// `v = v ± <proven>` (the counted-for counter's shape) on the argument
/// that overflow-minting-MIN is C85's decided edge — and the corpus
/// falsified it: `1246-a-nullable-narrow-slot-answers-null.loft` pins that
/// a plain integer driven past `i64::MAX` reads as null AND that `??`
/// FIRES on it, so the overflow sentinel is an observable value contract,
/// not an ignorable edge.  A var that can step itself can overflow itself;
/// it proves nothing here, whatever C85 says about its static TYPE.
///
/// Pessimistic start, re-walk to fixpoint: each round re-evaluates every
/// `Set` against the current map, so facts only turn true as their inputs
/// do, the map grows monotonically, and it settles in at most `vars` rounds.
#[must_use]
pub fn non_sentinel_vars(data: &Data, code: &Value) -> HashMap<u16, bool> {
    let mut escaped: std::collections::HashSet<u16> = std::collections::HashSet::new();
    collect_escapes(data, code, &mut escaped);
    let mut vars: HashMap<u16, bool> = HashMap::new();
    loop {
        let mut round: HashMap<u16, bool> = HashMap::new();
        scan_sets(code, data, &vars, &mut round);
        let mut changed = false;
        for (nr, all_ok) in round {
            if all_ok && !escaped.contains(&nr) && !vars.get(&nr).copied().unwrap_or(false) {
                vars.insert(nr, true);
                changed = true;
            }
        }
        if !changed {
            return vars;
        }
    }
}

/// One round: fold every `Set(var, expr)`'s verdict under the current map
/// into `acc` — a var's entry is true only while every assignment to it is.
fn scan_sets(v: &Value, data: &Data, vars: &HashMap<u16, bool>, acc: &mut HashMap<u16, bool>) {
    if let Value::Set(nr, expr) = v.unspan() {
        let ok = non_sentinel(data, vars, expr);
        acc.entry(*nr).and_modify(|e| *e &= ok).or_insert(ok);
    }
    v.for_each_child(&mut |c| scan_sets(c, data, vars, acc));
}

/// Every var something other than a `Set` could write: a bare `Var` handed
/// to a `RefVar`-typed parameter (the callee writes through it — a by-VALUE
/// scalar argument can never be written back), any bare `Var` given to a
/// fn-ref call (the callee is unknown), a `TuplePut` destination, an `Iter`
/// variable.  Composite shapes are walked through the exhaustive child
/// iterator, so no site can be missed by a variant this match forgot.
fn collect_escapes(data: &Data, v: &Value, escaped: &mut std::collections::HashSet<u16>) {
    match v.unspan() {
        Value::Call(d_nr, args) => {
            let def = data.def(*d_nr);
            for (i, a) in args.iter().enumerate() {
                if let Value::Var(nr) = a.unspan() {
                    let by_ref = def.attributes().get(i).is_some_and(|at| {
                        matches!(at.typedef.base(), crate::data::Type::RefVar(_))
                    });
                    if by_ref {
                        escaped.insert(*nr);
                    }
                }
            }
        }
        Value::CallRef(_, args) => {
            for a in args {
                if let Value::Var(nr) = a.unspan() {
                    escaped.insert(*nr);
                }
            }
        }
        Value::TuplePut(nr, _, _) | Value::Iter(nr, _, _, _) => {
            escaped.insert(*nr);
        }
        _ => {}
    }
    v.for_each_child(&mut |c| collect_escapes(data, c, escaped));
}
