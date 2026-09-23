// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @F4 — Ranged/width integer types (u8/i8/u16/i16/i32/u32), their FIT half; @F38 is the
//! arithmetic-safety side of the same story.
//!
//! @PLN152 step 5 (`@FR-E-Uncomp-Seen`) — let `!` read a fit-failure the value cannot hold.
//!
//! `u8`, `i8`, `u16`, `i16`, `u32` and every `integer limit(lo, hi)` fill their own range,
//! so a store whose value does not fit takes the type's default and leaves nothing behind:
//! `x: u8 = 250; x += 10` answers `0`, and `!x`, `x == null` and `x == 0` say exactly what
//! they say for a computed zero. The failure is a property of the STORE, never of the
//! expression — `x + 10` widens to `integer`, where `260` is a perfectly good value — so the
//! only place the question can be asked is beside an assignment that names a narrow slot.
//!
//! The answer is carried in a `__fit_N` temp that lives across exactly the pair the author
//! wrote:
//!
//! ```text
//! place op= expr;          __fit_N: integer? = (place op expr) as <narrow>?;
//! if !place { … }     ⟹    place = <the same range guard, reading __fit_N>;
//!                          if !__fit_N { … }
//! ```
//!
//! Nothing is stored. Not in the slot, not in a field, not in an element — which is the
//! constraint that keeps the types worth declaring: a `vector<u8>` of 200 000 elements is
//! 0.362 MB against `vector<integer>`'s 5.909 MB, and a status bit beside every element
//! would hand half of that back. The temp is an ordinary `integer?` local of the enclosing
//! function, minted only where an author wrote the test.
//!
//! **The temp is a full-width `integer?`, never the narrow `τ?`.** A `u8?` slot sacrifices
//! its top code to hold null (`@FR-N-Reserve`), so binding the fit to one turns the
//! perfectly good answer `255` into a reported failure — measured on the plan's own
//! hand-written target shape, which was proven on `300` and on `200` and is wrong on `255`.
//!
//! ## What is and is not fused
//!
//! Only a `!place` in the CONDITION of the `if` that is the very next statement, naming the
//! very place the store named. One statement further apart and there is nothing left to
//! carry the status but the slot, and the slot is storage. That boundary is invisible in the
//! source, so it is policed rather than documented: `redundant-null-negation` reports every
//! `!` on a value whose type keeps no code for null, so moving the `if` one line away makes
//! the check announce that it is always false instead of silently testing nothing.
//!
//! Everything below stays exactly as it was, and none of it is silent — each still draws
//! that warning wherever the `!` is always false:
//!
//! | shape | why it is not fused |
//! |---|---|
//! | a statement between the store and the `if` | only the PLACE could carry the status that far, and the place is storage |
//! | `!place` in the `if`'s BODY, or in a `while` | the body is past the pair; a `while` would spin on a status that cannot change |
//! | `!place` in an `else if` arm | it is not the condition of the statement's own `if` |
//! | a place whose read contains a call — `w[bump()]` | the fused form DROPS the second read, so a read that can do more than fetch must not match |
//! | a TUPLE element — `t.0` | its read is a [`Value::TupleGet`], not an `OpGet…` call, so [`same_place`] cannot name it |
//! | a KEYED element — `ks[k].v` | the read is already nullable (the key may be absent), so `!` has a meaning there and keeps it |
//! | `place = (place + 10) ?? d` | the author already chose the value; `range_guard_inside_discharge` owns that store |
//! | `integer`, `i32`, and every `τ?` | they keep a code for their own failure, and `!` reads it anywhere |
//!
//! Where no `!` names the place, nothing here runs and emission is byte-identical.

use crate::data::{Data, IntegerSpec, Type, Value};
use crate::diagnostics::{Level, diagnostic_format};

/// A narrow store whose fit-failure the next statement may ask about.
///
/// Built at the compound-assignment seam — the one place a compound store passes through
/// with its target still a readable place — and consumed by the `!` in the following
/// condition.
#[derive(Debug, Clone)]
pub(crate) struct FitFusion {
    /// The place the store names, spelled as the expression that READS it: a `Var` for a
    /// local, an `OpGet…` chain for a field or an element. This is what a `!` operand is
    /// compared against, and comparing the read is what makes the three spellings one case.
    pub(crate) place: Value,
    /// The narrow target's spec — the bounds the checked cast tests against.
    pub(crate) spec: IntegerSpec,
    /// The `__fit_N` temp, minted the first time a `!place` asks for it and `None` while
    /// nobody has. `None` at the end of the statement means the author wrote no test, which
    /// is the ordinary case and the one that must emit exactly what it emitted before — and,
    /// since C127, the one [`crate::parser::Parser::retire_fit_candidate`] advises about.
    pub(crate) fit_var: Option<u16>,
    /// Where the compound assignment stood, captured at the seam.  The advice is decided a
    /// statement later, by which time the lexer's cursor is inside the NEXT statement.
    pub(crate) at: crate::lexer::Position,
}

/// Do these two expressions name the same place, using only nodes it is safe to READ ONCE?
///
/// Both questions at once, deliberately. The fused form deletes the second read of the
/// place — the `!` reads `__fit_N` instead — so a place whose read can do anything other
/// than fetch a value must not match, and the cheapest way to guarantee that is to admit no
/// node that could. `Var`, integer literals and `OpGet…` calls are the whole admitted set:
/// a user call, a `TupleGet`, a discharge block or an arithmetic op all fail to match and so
/// simply do not fuse.
///
/// The fallback is `false`, and that is the safe direction: a shape this cannot read leaves
/// the pair exactly as it is today, which is a missed opportunity and never a wrong answer.
pub(crate) fn same_place(data: &Data, a: &Value, b: &Value) -> bool {
    match (a.unspan(), b.unspan()) {
        (Value::Var(x), Value::Var(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Long(x), Value::Long(y)) => x == y,
        (Value::Call(dx, ax), Value::Call(dy, ay)) => {
            dx == dy
                && data.def(*dx).name().starts_with("OpGet")
                && ax.len() == ay.len()
                && ax
                    .iter()
                    .zip(ay.iter())
                    .all(|(p, q)| same_place(data, p, q))
        }
        _ => false,
    }
}

/// The `OpRangeDefault(value, lo, hi, dflt)` a narrow store wraps its value in, inside the
/// statement node that performs the store.
///
/// Two shapes reach here and they are the two a store has: `Set(var, value)` for a local and
/// `Call(OpSet…, [place…, value])` for a field or an element, whose value is always the last
/// argument. Anything else answers `None` and the pair is left alone.
///
/// Asked BEFORE the fused form is armed, so a store shape this cannot recognise never
/// reaches the point where the `!` has already been rewritten to read a temp that was never
/// bound.
pub(crate) fn guard_slot<'a>(data: &Data, stmt: &'a mut Value) -> Option<&'a mut Value> {
    let slot = match stmt.unspan_mut() {
        Value::Set(_, value) => Some(&mut **value),
        Value::Call(_, args) => args.last_mut(),
        _ => None,
    }?;
    match slot.unspan() {
        Value::Call(d, args) if data.def(*d).name() == "OpRangeDefault" && args.len() == 4 => {
            Some(slot)
        }
        _ => None,
    }
}

/// Does this statement carry a recognisable narrow-store guard? The read-only half of
/// [`guard_slot`], for the promotion test.
pub(crate) fn has_guard_slot(data: &Data, stmt: &Value) -> bool {
    let slot = match stmt.unspan() {
        Value::Set(_, value) => Some(&**value),
        Value::Call(_, args) => args.last(),
        _ => None,
    };
    matches!(slot.map(Value::unspan),
        Some(Value::Call(d, args)) if data.def(*d).name() == "OpRangeDefault" && args.len() == 4)
}

/// The type a `__fit_N` temp is declared with: a full-width nullable `integer`.
///
/// Never `Optional(τ)` for the narrow τ being stored — see the module header. The checked
/// cast still tests τ's own bounds; only the temp that holds the outcome is wide.
pub(crate) fn fit_var_type() -> Type {
    Type::optional(crate::data::I64.clone())
}

impl crate::parser::Parser {
    /// The `__fit_N` temp a `!place` in a fused position must read, or `None` when this `!`
    /// is not one — which is every `!` in a program that did not write the pair.
    ///
    /// Minted on demand and at most once per store, so `if !a and !a { … }` reads one temp
    /// and a condition naming a different place mints nothing.
    pub(crate) fn fuse_fit_test(&mut self, operand: &Value) -> Option<u16> {
        if self.first_pass || !self.fit_in_condition {
            return None;
        }
        let cand = self.fit_armed.as_ref()?;
        if let Some(v) = cand.fit_var {
            return same_place(&self.data, &cand.place, operand).then_some(v);
        }
        if !same_place(&self.data, &cand.place, operand) {
            return None;
        }
        let v = self.create_unique("_fit", &fit_var_type());
        if let Some(c) = self.fit_armed.as_mut() {
            c.fit_var = Some(v);
        }
        Some(v)
    }

    /// C127 `narrow-fallback` — a compound step into a DECLARED narrow range whose result
    /// does not fit takes the type's DEFAULT, and the site says nothing.
    ///
    /// The ONE place a fit candidate dies, so the advice is decided once however the
    /// candidate was lost: the next statement did not open with `if`, the `if` named another
    /// place, or the pushed node carried no guard slot to rewrite.  A candidate that DID fuse
    /// carries its `__fit_N` temp and is silent — the author wrote the check, and an advice
    /// telling them to write it again is the kind that spends the credibility the other sites
    /// live on.
    ///
    /// Armed at the same `dflt != i64::MIN` the fusion is (`Parser::towards_set`): a target
    /// that keeps a code for its own failure raises no candidate at all, because `!` reads
    /// that failure off the value anywhere.
    ///
    /// The `advice` tier and the reason for it are on
    /// [`crate::keys::narrow_fallback_lint_enabled`].
    pub(crate) fn retire_fit_candidate(&mut self, cand: Option<FitFusion>) {
        let Some(cand) = cand else { return };
        if cand.fit_var.is_some() || self.first_pass {
            return;
        }
        self.narrow_fallback_advice(cand.spec, &cand.at);
    }

    fn narrow_fallback_advice(&mut self, spec: IntegerSpec, at: &crate::lexer::Position) {
        if !crate::keys::narrow_fallback_lint_enabled() {
            return;
        }
        let dflt = spec.default_value();
        let (lo, hi) = (spec.min, spec.max);
        crate::diagnostic_at!(
            self.lexer,
            at,
            Level::Advice,
            code = "narrow-fallback",
            "a step past `{lo}..{hi}` takes this type's default, `{dflt}` — nothing here \
             says which values the step may produce"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: "read the failure: `if !<place> { … }` as the very next statement".to_string(),
            condition: Some(
                "the step can leave the range and the program must know — the pair is fused, \
                 so nothing is stored beside the value"
                    .to_string(),
            ),
            edit: None,
            concept: "ranged integer types",
            concept_ref: "@F4",
        });
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: "choose the value: `<place> = (<place> + n) ?? <d>`".to_string(),
            condition: Some("a different fallback than the type's default is wanted".to_string()),
            edit: None,
            concept: "ranged integer types",
            concept_ref: "@F4",
        });
    }
}
