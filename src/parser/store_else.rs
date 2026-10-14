// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @C130 / @F126 — a store's failure arm: `place = v else { B }` (`@FR-H-Write-Else`).
//!
//! A write that does not take is a no-op that continues (`@FR-H-WriteNull`,
//! `@FR-H-WriteOOB`): an index out of range, an absent key, a null view anywhere on the
//! place's path — every one of them leaves the setter a target whose `rec` is 0, and the
//! setter's `rec == 0` test skips the write (`default/01_code.loft`, the `OpSet…` bodies; a
//! record copy drops a null place the same way).  A narrow slot the value does not fit takes
//! its default instead (`@FR-E-Uncomp-NN`), and a locked store discards the write
//! (`@FR-H-WriteLocked`).  The arm is the author's way to say what happens then, with the
//! place spelled once.
//!
//! The lowering, shared by both backends because it is IR:
//!
//! ```text
//! place = v else { B }   ⟹   …the statement's own leading temps…
//!                             __wt = <the place's target>;
//!                             if !__wt || get_store_lock(__wt) { __wt = <reported null> }
//!                             [__fv = <the value>]               (a narrow store only)
//!                             OpSet…(__wt, …, v');               (each write's TARGET → __wt)
//!                             if !__wt [|| __fv does not fit] { B }
//! ```
//!
//! `!__wt` is `OpNot(OpConvBoolFromRef(__wt))` — `rec == 0`, exactly the setters' own test,
//! so the arm runs if and only if the write was skipped.  Only the target ARGUMENT of each
//! write reads the temp: a compound store's read of the same place keeps its own expression,
//! which the parser has already bound to a `__place_N` temp whenever it could do more than
//! fetch (`w[bump()] += 1` runs `bump` once with or without the arm).  A local, a tuple
//! element and every other write that names no store always land; their arm runs only for a
//! value a narrow slot cannot hold.  A statement with no trailing `else` never reaches this
//! file and emits what it always emitted.

use crate::data::{Type, Value};
use crate::diagnostics::{Level, diagnostic_format};
use crate::parser::{Parser, v_if, v_set};

impl Parser {
    /// The `else { B }` after an assignment statement `n` (the lexer is ON `else`): parse the
    /// arm and rewrite `n` into the lowering above.  A statement that is no store is refused,
    /// naming the cure; the arm is still parsed so the block reads on.
    pub(crate) fn parse_store_else(&mut self, n: &mut Value) {
        let at = *self.lexer.pos();
        self.lexer.token("else");
        let mut arm = Value::Null;
        // The arm is a branch that may not run, bracketed like an `if` with no `else`: a
        // write inside it cannot make a write before the store dead.
        let write_state = self.vars.save_and_clear_write_state();
        self.vars.clear_write_state();
        self.parse_block("else", &mut arm, &Type::Void);
        self.vars.restore_write_state(&write_state);
        let mut steps = match std::mem::replace(n, Value::Null) {
            Value::Insert(list) => list,
            other => vec![other],
        };
        let Some(shape) = self.store_shape(&steps) else {
            if !self.first_pass {
                crate::diagnostic_at!(
                    self.lexer,
                    &at,
                    Level::Error,
                    "`else` after a statement belongs to a store — `place = value else {{ … }}` \
                     runs when the write did not take; this statement writes no place"
                );
            }
            *n = Value::Insert(steps);
            return;
        };
        // Every binding goes in front of the step that needs it, by ORIGINAL index; applied
        // back to front, so no insertion moves another's index.  At one index the target's
        // group precedes the value's, which keeps the store's target-before-value order.
        let mut inserts: Vec<(usize, u8, Vec<Value>)> = Vec::new();
        let mut absent: Option<Value> = None;
        if let Some((target, writes)) = shape.target {
            // The place, bound once: a view of what it names, never freed by this frame.
            let tp = self.store_target_type(&target);
            let wt = self.create_unique("store_target", &tp);
            self.vars.defined(wt);
            self.vars.set_skip_free(wt);
            if let Some(src) = store_borrow_source(&target) {
                let viewed = self.vars.tp(wt).clone().depending(src);
                self.vars.set_type(wt, viewed);
            }
            for &(at, arg) in &writes {
                if let Value::Call(_, args) = steps[at].unspan_mut() {
                    args[arg] = Value::Var(wt);
                }
            }
            // `@FR-H-WriteLocked` under an arm: a locked store is a write that does not take,
            // in both run modes.  An absent or locked target becomes the REPORTED null
            // (`OpNullReported`, @FR-E-Report): the setter skips through its own `rec == 0` test
            // — the value is still evaluated once — and says nothing, because the arm is the
            // report; the one `!temp` below is the whole answer.
            let locked = self.cl("n_get_store_lock", &[Value::Var(wt)]);
            let present = self.cl("OpConvBoolFromRef", &[Value::Var(wt)]);
            let missing = self.cl("OpNot", &[present]);
            let take_over = v_if(missing, Value::Boolean(true), locked);
            let null = self.cl("OpNullReported", &[]);
            // A record VARIABLE is bound through the projection a view is spelled as
            // (`@FR-R-CopyView`'s `OpGetField(a, 0)`): a plain `Set` from it is a COPY
            // (`@FR-B-Copy`), and the write would land in the copy.
            let bound = match (target.unspan(), tp.base()) {
                (Value::Var(_), Type::Reference(d, _) | Type::Enum(d, true, _)) => {
                    let kt = i32::from(self.data.def(*d).known_type());
                    let get_field = self.data.def_nr("OpGetField");
                    Value::Call(get_field, vec![target, Value::Int(0), Value::Int(kt)])
                }
                _ => target,
            };
            inserts.push((
                writes[0].0,
                0,
                vec![
                    v_set(wt, bound),
                    v_if(take_over, v_set(wt, null), Value::Null),
                ],
            ));
            let present = self.cl("OpConvBoolFromRef", &[Value::Var(wt)]);
            absent = Some(self.cl("OpNot", &[present]));
        }
        let mut misfit: Option<Value> = None;
        if let Some((at, lo, hi)) = shape.narrow {
            // `@FR-E-Uncomp-NN` — the value that did not fit took the slot's default; the
            // checked value, bound once, says so.  A full-width `integer?` like fit.rs's temp,
            // so the slot's own top code (`@FR-N-Reserve`) is never mistaken for a misfit.
            let fv = self.create_unique("store_value", &crate::parser::fit::fit_var_type());
            self.vars.defined(fv);
            if let Some(inner) = take_range_default_operand(&mut steps[at], &Value::Var(fv)) {
                inserts.push((at, 1, vec![v_set(fv, inner)]));
                let below = self.cl("OpLtInt", &[Value::Var(fv), Value::Long(lo)]);
                let above = self.cl("OpLtInt", &[Value::Long(hi), Value::Var(fv)]);
                let present = self.cl("OpConvBoolFromInt", &[Value::Var(fv)]);
                let null = self.cl("OpNot", &[present]);
                let out = v_if(below, Value::Boolean(true), above);
                misfit = Some(v_if(null, Value::Boolean(true), out));
            }
        }
        inserts.sort_by_key(|&(at, order, _)| std::cmp::Reverse((at, order)));
        for (at, _, group) in inserts {
            for v in group.into_iter().rev() {
                steps.insert(at, v);
            }
        }
        let failed = match (absent, misfit) {
            (Some(a), Some(m)) => Some(v_if(a, Value::Boolean(true), m)),
            (a, m) => a.or(m),
        };
        if let Some(cond) = failed {
            steps.push(v_if(cond, arm, Value::Null));
        }
        *n = Value::Insert(steps);
    }
}

/// What `parse_store_else` needs of a store statement's steps.
struct StoreShape {
    /// The place's target and every write through it, as `(step, argument)` — `None` for a
    /// write that names no store (a local, a tuple element), which always lands.
    target: Option<(Value, Vec<(usize, usize)>)>,
    /// The step holding a narrow slot's range check, with the slot's bounds.
    narrow: Option<(usize, i64, i64)>,
}

impl Parser {
    /// The shape of the store `steps` spell, or `None` when they write no place.  A store
    /// lowers to its own temps (`__lift_N`, `__place_N`), then its WRITES — a setter
    /// (`OpSet…(target, …)`) per field, or a record copy (`OpCopyRecord(source, target, tp)`)
    /// — all through one target, then its frees.  A statement ending in a local `Set` or a
    /// `TuplePut` is a store with no target.  Anything else — a call, an expression, writes
    /// through two different targets — is no single place, and the arm is refused rather
    /// than guessed at.
    fn store_shape(&self, steps: &[Value]) -> Option<StoreShape> {
        let mut target: Option<Value> = None;
        let mut writes = Vec::new();
        for (at, s) in steps.iter().enumerate() {
            let Value::Call(op, args) = s.unspan() else {
                continue;
            };
            let Some(arg) = self.write_target_arg(*op) else {
                continue;
            };
            let t = args.get(arg)?.unspan().clone();
            if target.as_ref().is_some_and(|x| *x != t) {
                return None;
            }
            target = Some(t);
            writes.push((at, arg));
        }
        let narrow = steps
            .iter()
            .enumerate()
            .find_map(|(at, s)| find_range_default(s).map(|(lo, hi)| (at, lo, hi)));
        if let Some(t) = target {
            return Some(StoreShape {
                target: Some((t, writes)),
                narrow,
            });
        }
        let last = steps
            .iter()
            .rposition(|s| !matches!(s.unspan(), Value::Line(_)))?;
        match steps[last].unspan() {
            Value::Set(..) | Value::TuplePut(..) => Some(StoreShape {
                target: None,
                narrow,
            }),
            _ => None,
        }
    }

    /// Which argument of operator `op` is the place it writes, when it is a store's write.
    fn write_target_arg(&self, op: u32) -> Option<usize> {
        let name = self.data.def(op).name.as_str();
        if name == "OpCopyRecord" {
            Some(1)
        } else if name.starts_with("OpSet")
            && !matches!(name, "OpSetStackRef" | "OpSetStackFnRef" | "OpSetKeyed")
        {
            Some(0)
        } else {
            None
        }
    }

    /// The temp's type: what the target expression answers.
    fn store_target_type(&self, target: &Value) -> Type {
        match target.unspan() {
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

/// The bounds of a narrow store's `OpRangeDefault(value, lo, hi, default)` in `n`, if any.
/// Only that operator takes two literal bounds after its operand, so the shape names it.
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
        && let [_, Value::Long(_), Value::Long(_), _] = args.as_slice()
    {
        *taken = Some(std::mem::replace(&mut args[0], to.clone()));
        return;
    }
    n.for_each_child_mut(&mut |c| take_in(c, to, taken));
}
