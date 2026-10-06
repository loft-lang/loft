// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN178 — a store's failure arm: `place = v else { B }` (`@FR-H-Write-Else`).
//!
//! A write that does not take is a no-op that continues (`@FR-H-WriteNull`,
//! `@FR-H-WriteOOB`): an index out of range, an absent key, a null view anywhere on the
//! place's path — every one of them leaves the setter a target whose `rec` is 0, and the
//! setter's `rec == 0` test skips the write (`default/01_code.loft`, the `OpSet…` bodies).  A
//! narrow slot the value does not fit takes its default instead (`@FR-E-Uncomp-NN`).  The arm
//! is the author's way to say what happens then, with the place spelled once.
//!
//! The lowering, shared by both backends because it is IR:
//!
//! ```text
//! place = v else { B }   ⟹   __wt = <the place's target>;      (evaluated ONCE)
//!                             [__fv = <the value> ]             (a narrow store only)
//!                             OpSet…(__wt, …, v');              (every target read → __wt)
//!                             if !__wt [|| __fv does not fit] { B }
//! ```
//!
//! `!__wt` is `OpNot(OpConvBoolFromRef(__wt))` — `rec == 0`, exactly the setters' own test,
//! so the arm runs if and only if the setter skipped.  A compound store's read of the place
//! reads the temp too, so `w[bump()] += 1 else { … }` runs `bump` once.  A store with no
//! `else` never reaches this file and emits what it always emitted.

use crate::data::{Type, Value};
use crate::diagnostics::{Level, diagnostic_format};
use crate::parser::{Parser, v_if, v_set};

impl Parser {
    /// The `else { B }` after an assignment statement `n` (the lexer is ON `else`): parse the
    /// arm and rewrite `n` into the lowering above.  A statement that is no store is refused,
    /// naming the cure; the arm is still parsed so the block reads on.
    pub(crate) fn parse_store_else(&mut self, n: &mut Value) {
        self.lexer.token("else");
        let mut arm = Value::Null;
        self.parse_block("else", &mut arm, &Type::Void);
        let Some(shape) = self.store_shape(n) else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`else` after a statement belongs to a store — `place = value else {{ … }}` \
                     runs when the write did not take; this statement writes no place"
                );
            }
            return;
        };
        let mut steps = Vec::new();
        let mut failed: Option<Value> = None;
        let mut store = std::mem::replace(n, Value::Null);
        if let Some(target) = shape.target {
            // The place, bound once: a view of what it names, never freed by this frame.
            let tp = self.store_target_type(&target);
            let wt = self.create_unique("store_target", &tp);
            self.vars.defined(wt);
            self.vars.set_skip_free(wt);
            if let Some(src) = store_borrow_source(&target) {
                let viewed = self.vars.tp(wt).clone().depending(src);
                self.vars.set_type(wt, viewed);
            }
            replace_value(&mut store, &target, &Value::Var(wt));
            steps.push(v_set(wt, target));
            let absent = self.cl("OpConvBoolFromRef", &[Value::Var(wt)]);
            failed = Some(self.cl("OpNot", &[absent]));
        }
        if let Some((lo, hi)) = shape.narrow {
            // `@FR-E-Uncomp-NN` — the value that did not fit took the slot's default; the
            // checked value, bound once, says so.
            let fv = self.create_unique("store_value", &crate::parser::fit::fit_var_type());
            self.vars.defined(fv);
            if let Some(inner) = take_range_default_operand(&mut store, &Value::Var(fv)) {
                steps.push(v_set(fv, inner));
                let below = self.cl("OpLtInt", &[Value::Var(fv), Value::Long(lo)]);
                let above = self.cl("OpLtInt", &[Value::Long(hi), Value::Var(fv)]);
                let present = self.cl("OpConvBoolFromInt", &[Value::Var(fv)]);
                let absent = self.cl("OpNot", &[present]);
                let out = v_if(below, Value::Boolean(true), above);
                let misfit = v_if(absent, Value::Boolean(true), out);
                failed = Some(match failed {
                    Some(f) => v_if(f, Value::Boolean(true), misfit),
                    None => misfit,
                });
            }
        }
        steps.push(store);
        if let Some(cond) = failed {
            steps.push(v_if(cond, arm, Value::Null));
        }
        *n = Value::Insert(steps);
    }
}

/// What `parse_store_else` needs of a store statement: the place's TARGET (the reference
/// expression its setters write through), and the bounds when the slot is narrow.
struct StoreShape {
    target: Option<Value>,
    narrow: Option<(i64, i64)>,
}

impl Parser {
    /// The shape of the store `n` is, or `None` when it writes no place.  A setter call
    /// (`OpSet…(target, field, …)`), a whole-record build (setters on ONE target), and a local
    /// `Set` (no target: it always lands, a narrow local excepted).
    fn store_shape(&self, n: &Value) -> Option<StoreShape> {
        let narrow = find_range_default(n);
        match n.unspan() {
            Value::Set(_, _) => Some(StoreShape {
                target: None,
                narrow,
            }),
            Value::Call(op, args) if self.is_setter(*op) => Some(StoreShape {
                target: args.first().map(|t| t.unspan().clone()),
                narrow,
            }),
            Value::Insert(list) => {
                let mut target: Option<Value> = None;
                for s in list {
                    match s.unspan() {
                        Value::Call(op, args) if self.is_setter(*op) => {
                            let t = args.first()?.unspan().clone();
                            if target.as_ref().is_some_and(|x| *x != t) {
                                return None;
                            }
                            target = Some(t);
                        }
                        Value::Line(_) => {}
                        _ => return None,
                    }
                }
                target.map(|t| StoreShape {
                    target: Some(t),
                    narrow,
                })
            }
            _ => None,
        }
    }

    fn is_setter(&self, op: u32) -> bool {
        let name = self.data.def(op).name.as_str();
        name.starts_with("OpSet")
            && !matches!(name, "OpSetStackRef" | "OpSetStackFnRef" | "OpSetKeyed")
    }

    /// The temp's type: what the target expression answers.
    fn store_target_type(&self, target: &Value) -> Type {
        match target {
            Value::Var(v) => self.vars.tp(*v).clone(),
            Value::Call(op, _) => self.data.def(*op).returned.clone(),
            _ => Type::Reference(u32::MAX, crate::data::Deps::default()),
        }
    }
}

/// The frame variable the target's path starts from: what the bound view borrows.  A path
/// that starts at no variable (a call's result) has no source, and the temp then borrows
/// nothing — it is never freed either way (`set_skip_free`).
fn store_borrow_source(target: &Value) -> Option<u16> {
    match target.unspan() {
        Value::Var(v) => Some(*v),
        Value::Call(_, args) => args.first().and_then(store_borrow_source),
        _ => None,
    }
}

/// Every occurrence of `from` in `n` replaced by `to` — the place's other reads (a compound
/// store's read of the same field) read the bound target too.
fn replace_value(n: &mut Value, from: &Value, to: &Value) {
    if n.unspan() == from {
        *n = to.clone();
        return;
    }
    n.for_each_child_mut(&mut |c| replace_value(c, from, to));
}

/// The bounds of the narrow store's `OpRangeDefault`, when the statement has one.
fn find_range_default(n: &Value) -> Option<(i64, i64)> {
    let mut found = None;
    walk(n, &mut |v| {
        if let Value::Call(_, args) = v
            && let [_, Value::Long(lo), Value::Long(hi), _] = args.as_slice()
            && found.is_none()
        {
            found = Some((*lo, *hi));
        }
    });
    found
}

fn walk(n: &Value, f: &mut impl FnMut(&Value)) {
    f(n.unspan());
    n.for_each_child(&mut |c| walk(c, f));
}

/// Replace the operand of the statement's `OpRangeDefault` with `to`, answering the operand.
fn take_range_default_operand(n: &mut Value, to: &Value) -> Option<Value> {
    let mut taken = None;
    take_in(n, to, &mut taken);
    taken
}

fn take_in(n: &mut Value, to: &Value, taken: &mut Option<Value>) {
    if taken.is_some() {
        return;
    }
    if let Value::Call(_, args) = n.unspan_mut()
        && args.len() == 4
        && matches!((&args[1], &args[2]), (Value::Long(_), Value::Long(_)))
    {
        *taken = Some(std::mem::replace(&mut args[0], to.clone()));
        return;
    }
    n.for_each_child_mut(&mut |c| take_in(c, to, taken));
}
