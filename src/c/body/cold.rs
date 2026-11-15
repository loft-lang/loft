// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! `@FR-R-Cold`'s loft clause: an early exit that builds a text is written as a cold helper.
//!
//! A guard that reports and leaves — `if pos + n > len(code) { fault = "…{pos}…"; return
//! false }` — builds its message in a work text.  Written in place, that work text is a
//! local of the whole function: taken from its park at entry, held, emptied and parked again
//! on every exit, and its statements count against `@FR-R-Inline`'s budget, so an accessor
//! that is two compares on its hot path is never inlined.  Written as a `LOFT_NOINLINE
//! LOFT_COLD` helper taking the function's own parameters, the arm costs one call where it
//! runs — far less than the text it builds — and the function keeps only its hot path.
use super::FnWriter;
use crate::data::Value;
use std::collections::{HashMap, HashSet};

/// What [`FnWriter::cold_finish`] answers when the body named a variable the plan gave to an
/// arm — an arm the writer reached another way than as a statement `if`.  The caller writes
/// the function again with the clause off; a helper is written only for a body that keeps it.
pub(crate) const COLD_ESCAPED: &str = "a cold exit's variable named outside its arm";

/// A planned helper: its name, the arm, the variables it owns, its parameter list and shapes,
/// and whether the body wrote its call.
pub(crate) struct Pending<'a> {
    name: String,
    arm: &'a Value,
    own: HashSet<u16>,
    params: String,
    shapes: Vec<super::CType>,
    called: bool,
}

/// The variable numbers `v` names anywhere: a variable read, an assignment, an indirect call
/// through a local, a loop's iterator, a tuple element read or write.  A function reference's
/// numbers are not variables; `cold_arm` refuses an arm holding one, and the plan a body
/// holding one.
fn mentions(v: &Value, out: &mut HashMap<u16, usize>) {
    v.walk(&mut |n| {
        let x = match n {
            Value::Var(x)
            | Value::Set(x, _)
            | Value::CallRef(x, _)
            | Value::Iter(x, ..)
            | Value::TupleGet(x, _)
            | Value::TuplePut(x, ..) => *x,
            _ => return,
        };
        *out.entry(x).or_default() += 1;
    });
}

/// The variables `v` releases with `OpFreeText`: a text local's free at an exit, which an
/// outlined arm's own epilogue does for it — and, the text being undeclared outside the arm,
/// the body skips.
fn text_frees(v: &Value, data: &crate::data::Data, out: &mut HashMap<u16, usize>) {
    v.walk(&mut |n| {
        if let Some(w) = text_free(n, data) {
            *out.entry(w).or_default() += 1;
        }
    });
}

/// `OpFreeText(w)`'s variable.
pub(super) fn text_free(n: &Value, data: &crate::data::Data) -> Option<u16> {
    match n.unspan() {
        Value::Call(op, a)
            if (*op as usize) < data.definitions.len() && data.def(*op).name() == "OpFreeText" =>
        {
            match a.as_slice() {
                [x] => match x.unspan() {
                    Value::Var(w) => Some(*w),
                    _ => None,
                },
                _ => None,
            }
        }
        _ => None,
    }
}

/// Is `x` an assignment of the empty or null value — what a body's work text and temporaries
/// are initialised with, which an outlined arm's own declaration gives as well?
fn empty_init(x: &Value) -> Option<u16> {
    match x.unspan() {
        Value::Set(w, v)
            if matches!(v.unspan(), Value::Text(t) if t.is_empty())
                || matches!(v.unspan(), Value::Null) =>
        {
            Some(*w)
        }
        _ => None,
    }
}

impl<'a> FnWriter<'a> {
    /// May this body outline its cold exits?  Only the ordinary writer asks: a twin's body
    /// carries the masks and buffers its own call site set up.
    pub(crate) fn cold_exits(mut self, on: bool) -> Self {
        self.cold_on = on;
        self
    }

    /// Is `arm` — the `then` of an `if` — a cold exit: a block that ends in its one `return`,
    /// of a literal or of a local the block set to one (`r = false; …; return r`, as the scope
    /// pass writes a return that frees), builds a text in a work text, and holds nothing that
    /// leaves it another way (a loop, a `break`, a `continue`, a `yield`) or names a variable
    /// another way than by number (a function reference, a `par` arm)?  Answers the literal the
    /// exit returns; any other shape is written in place.
    fn cold_arm(&self, arm: &Value) -> Option<Value> {
        let literal = |value: &Value| {
            matches!(
                value.unspan(),
                Value::Null
                    | Value::Int(_)
                    | Value::Long(_)
                    | Value::Boolean(_)
                    | Value::Float(_)
                    | Value::Single(_)
                    | Value::Enum(..)
            )
        };
        let Value::Block(block) = arm.unspan() else {
            return None;
        };
        let Some(Value::Return(ret)) = block.operators.last().map(Value::unspan) else {
            return None;
        };
        let answer = match ret.unspan() {
            value if literal(value) => value.clone(),
            Value::Var(x) => {
                let mut set = None;
                let mut named = 0usize;
                arm.walk(&mut |n| match n {
                    Value::Set(y, v) if y == x => {
                        named += 1;
                        set = Some(v.unspan().clone());
                    }
                    Value::Var(y)
                    | Value::CallRef(y, _)
                    | Value::Iter(y, ..)
                    | Value::TupleGet(y, _)
                    | Value::TuplePut(y, ..)
                        if y == x =>
                    {
                        named += 1;
                    }
                    _ => {}
                });
                match set {
                    Some(v) if named == 2 && literal(&v) => v,
                    _ => return None,
                }
            }
            _ => return None,
        };
        let mut returns = 0usize;
        let mut other_exit = false;
        let mut builds_text = false;
        arm.walk(&mut |node| match node {
            Value::Return(_) => returns += 1,
            Value::Loop(_)
            | Value::Break(_)
            | Value::Continue(_)
            | Value::Yield(_)
            | Value::FnRef(..)
            | Value::FnRefDnr(_)
            | Value::Parallel(_) => other_exit = true,
            Value::Set(var, _) if self.vars.name(*var).starts_with("__work") => {
                builds_text = true;
            }
            _ => {}
        });
        (returns == 1 && !other_exit && builds_text).then_some(answer)
    }

    /// `LOFT_TRACE_COLD=1`: one line per statement `if` arm that builds a text — outlined, or
    /// why not — with the arm's line and its size in IR nodes, on stderr.
    fn cold_trace(&self, arm: &Value, verdict: &str) {
        if !crate::env_once!(std::env::var("LOFT_TRACE_COLD").is_ok_and(|v| v != "0")) {
            return;
        }
        let mut line = 0u32;
        let mut nodes = 0usize;
        arm.walk(&mut |n| {
            nodes += 1;
            if line == 0
                && let Value::Line(l) = n
            {
                line = *l;
            }
        });
        eprintln!(
            "[cold] {}\tL{line}\t{nodes}\t{verdict}",
            self.cx.data.def(self.d).name()
        );
    }

    /// Does `arm` build a text in a work text — the arms this clause is about?
    fn builds_text(&self, arm: &Value) -> bool {
        let mut hit = false;
        arm.walk(&mut |n| {
            if let Value::Set(var, _) = n
                && self.vars.name(*var).starts_with("__work")
            {
                hit = true;
            }
        });
        hit
    }

    /// Why [`Self::cold_arm`] refuses `arm`, for the trace.
    fn shape_reason(arm: &Value) -> &'static str {
        let Value::Block(block) = arm.unspan() else {
            return "refused: not a block";
        };
        let last = block.operators.last().map(Value::unspan);
        let Some(Value::Return(ret)) = last else {
            return "refused: ends without a return";
        };
        let mut returns = 0usize;
        let mut other = false;
        arm.walk(&mut |n| match n {
            Value::Return(_) => returns += 1,
            Value::Loop(_)
            | Value::Break(_)
            | Value::Continue(_)
            | Value::Yield(_)
            | Value::FnRef(..)
            | Value::FnRefDnr(_)
            | Value::Parallel(_) => other = true,
            _ => {}
        });
        if other {
            "refused: holds a loop, break, continue, yield, fn ref or par"
        } else if returns > 1 {
            "refused: more than one return"
        } else if matches!(ret.unspan(), Value::Var(_)) {
            "refused: returns a local not set to a literal"
        } else {
            "refused: returns a computed value"
        }
    }

    /// Find the cold exits and write their helpers.  An arm is outlined when every variable
    /// it names, besides the parameters, is named nowhere else in the body but by an empty
    /// initialisation: those variables are the helper's own, and this body does not
    /// declare them.  `LOFT_NO_COLD_EXIT=1` writes every arm in place.
    #[expect(
        clippy::too_many_lines,
        reason = "one plan: the arms, each one's verdict and its trace, the helpers' signature"
    )]
    pub(super) fn cold_plan(&mut self) {
        if !self.cold_on
            || crate::env_once!(std::env::var("LOFT_NO_COLD_EXIT").is_ok_and(|v| v != "0"))
        {
            return;
        }
        let code: &'a Value = self.code;
        let mut whole = HashMap::new();
        mentions(code, &mut whole);
        let mut frees = HashMap::new();
        text_frees(code, self.cx.data, &mut frees);
        let mut unnamed = false;
        let mut arms: Vec<&'a Value> = Vec::new();
        code.walk(&mut |n| match n {
            Value::FnRef(..) | Value::FnRefDnr(_) | Value::Parallel(_) | Value::Yield(_) => {
                unnamed = true;
            }
            // An `if` standing as a statement of a block: one in value position is written by
            // the value writers, which do not outline.
            Value::Block(b) => {
                for op in &b.operators {
                    if let Value::If(_, t, _) = op.unspan() {
                        arms.push(t);
                    }
                }
            }
            _ => {}
        });
        if unnamed {
            for arm in arms.iter().filter(|a| self.builds_text(a)) {
                self.cold_trace(
                    arm,
                    "refused: the function holds a fn ref, par arm or yield",
                );
            }
            return;
        }
        if arms.is_empty() {
            return;
        }
        // The empty initialisations of each variable at the body's top level.
        let mut inits: HashMap<u16, usize> = HashMap::new();
        if let Value::Block(b) = code.unspan() {
            for op in &b.operators {
                if let Some(w) = empty_init(op) {
                    *inits.entry(w).or_default() += 1;
                }
            }
        }
        let mut admitted: Vec<(&'a Value, HashSet<u16>, Value)> = Vec::new();
        for arm in arms {
            let Some(answer) = self.cold_arm(arm) else {
                if self.builds_text(arm) {
                    self.cold_trace(arm, Self::shape_reason(arm));
                }
                continue;
            };
            let mut inside = HashMap::new();
            mentions(arm, &mut inside);
            let mut freed_inside = HashMap::new();
            text_frees(arm, self.cx.data, &mut freed_inside);
            let mut own = HashSet::new();
            let mut ok = true;
            for (&x, &n) in &inside {
                if self.params[x as usize] {
                    continue;
                }
                // Outside the arm only its empty initialisations and its frees may name it.
                let outside = whole.get(&x).copied().unwrap_or(0) - n;
                let released = frees.get(&x).copied().unwrap_or(0)
                    - freed_inside.get(&x).copied().unwrap_or(0);
                if outside != inits.get(&x).copied().unwrap_or(0) + released {
                    self.cold_trace(
                        arm,
                        &format!("refused: reads local {} from outside", self.vars.name(x)),
                    );
                    ok = false;
                    break;
                }
                own.insert(x);
            }
            // Two arms sharing a variable: the first takes it, the second stays in place.
            if ok && admitted.iter().all(|(_, o, _)| o.is_disjoint(&own)) {
                admitted.push((arm, own, answer));
            } else if ok {
                self.cold_trace(arm, "refused: shares a variable with an earlier arm");
            }
        }
        if admitted.is_empty() {
            return;
        }
        let (params, shapes) = match self.cx.plain_params(self.d) {
            Ok(p) => p,
            Err(why) => {
                for (arm, _, _) in &admitted {
                    self.cold_trace(arm, &format!("refused: {why}"));
                }
                return;
            }
        };
        for (arm, _, _) in &admitted {
            self.cold_trace(arm, "outlined");
        }
        let base = super::c_name(self.cx.data, self.d);
        for (k, (arm, own, answer)) in admitted.into_iter().enumerate() {
            let name = format!("{base}__cold{k}");
            self.undeclared.extend(own.iter().copied());
            self.cold_arms
                .insert(&raw const *arm as usize, (name.clone(), answer));
            self.cold_pending.push(Pending {
                name,
                arm,
                own,
                params: params.clone(),
                shapes: shapes.clone(),
                called: false,
            });
        }
    }

    /// After the body: refuse it ([`COLD_ESCAPED`]) where it names a variable the plan gave
    /// to an arm, or did not write an arm's call; else write each helper once per unit.
    pub(super) fn cold_finish(&mut self, out: &str) -> Result<(), String> {
        if self.cold_pending.is_empty() {
            return Ok(());
        }
        let bytes = out.as_bytes();
        let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let named = |v: u16| {
            let n = super::var_name(self.vars.name(v), v);
            out.match_indices(&n).any(|(at, _)| {
                let next = bytes.get(at + n.len()).copied().unwrap_or(b' ');
                let prev = at.checked_sub(1).map_or(b' ', |p| bytes[p]);
                !ident(next) && !ident(prev)
            })
        };
        if self.undeclared.iter().any(|v| named(*v)) || self.cold_pending.iter().any(|p| !p.called)
        {
            return Err(COLD_ESCAPED.into());
        }
        for (k, p) in self.cold_pending.iter().enumerate() {
            if !self.cx.tr.borrow_mut().cold_helpers.insert((self.d, k)) {
                continue;
            }
            let mut helper = FnWriter::new(self.cx, self.d, p.shapes.clone(), super::CType::Void);
            helper.code = p.arm;
            // The helper declares only what the arm names besides the parameters.
            helper.undeclared = (0..self.shapes.len() as u16)
                .filter(|v| !self.params[*v as usize] && !p.own.contains(v))
                .collect();
            let (body, _) = helper.write()?;
            let sig = format!(
                "static LOFT_NOINLINE LOFT_COLD void {}({})",
                p.name, p.params
            );
            let _ = std::fmt::Write::write_fmt(
                &mut *self.cx.gen_decls.borrow_mut(),
                format_args!("{sig};\n"),
            );
            let _ = std::fmt::Write::write_fmt(
                &mut *self.cx.gen_bodies.borrow_mut(),
                format_args!("\n{sig} {{\n{body}}}\n"),
            );
            crate::rewrite_census::fired("R-Cold/loft", 1);
        }
        Ok(())
    }

    /// `if (cond) { <helper>(params); return <literal>; } else …`: the outlined arm.
    pub(super) fn cold_if(
        &mut self,
        cond: &Value,
        then_arm: &Value,
        else_arm: &Value,
        ind: usize,
    ) -> Result<(), String> {
        let (name, answer) = self.cold_arms[&(&raw const *then_arm as usize)].clone();
        if let Some(p) = self.cold_pending.iter_mut().find(|p| p.name == name) {
            p.called = true;
        }
        let ret = Value::Return(Box::new(answer));
        let args: Vec<String> = self
            .vars
            .arguments()
            .iter()
            .map(|&a| super::var_name(self.vars.name(a), a))
            .collect();
        let test = self.expr(cond, ind)?;
        let test = super::as_cond(test)?;
        self.line(
            ind,
            &format!("if (LOFT_UNLIKELY({})) {{", super::bare(&test)),
        );
        self.line(ind + 1, &format!("{name}({});", args.join(", ")));
        self.stmt(&ret, ind + 1)?;
        if !matches!(else_arm.unspan(), Value::Null) {
            self.line(ind, "} else {");
            self.stmt(else_arm, ind + 1)?;
        }
        self.line(ind, "}");
        self.flush(ind);
        Ok(())
    }
}
