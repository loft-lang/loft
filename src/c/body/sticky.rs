// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-StickySum` (@PLN219) in the C: a loop's integer SUM — a local every mention of which
//! inside the loop is its own update `acc = acc + e` or `acc = acc - e` (`e` not reading `acc`)
//! — is updated with the processor's operator, one flag per loop gathering every event the
//! checked operator answers null for: `acc` null on entry, a null `e`, an overflow, a result
//! equal to the sentinel.  At the loop's exit a set flag makes `acc` null.
//!
//! The checked sum answers null from its first event on: a null `acc` stays null through every
//! later `+` and `-`, and those report nothing.  Inside the loop nothing reads `acc` but its own
//! updates, so the plain value it carries after an event is never seen; at the exit — the one
//! place the loop's value is read — the flag turns it into the checked answer.  A loop that a
//! `break` or `continue` leaves past its own exit, or a sum read anywhere else in it, declines.
//! The outermost loop that admits a sum takes it, so the exit test runs once per pass of what
//! encloses that loop.  `LOFT_NO_STICKY_SUM=1`.

use super::{CType, FnWriter, as_int};
use crate::data::{Block, Value};

impl FnWriter<'_> {
    /// The sums loop `b` takes, each with a fresh flag: their flags are set before the loop
    /// (`acc` null on entry already counts) and registered for the updates inside it.
    pub(super) fn open_sticky_sums(&mut self, b: &Block, ind: usize) -> Vec<(u16, String)> {
        if crate::env_once!(std::env::var("LOFT_NO_STICKY_SUM").is_ok_and(|v| v != "0"))
            || self.gen_elem.is_some()
            || leaves_past(b)
        {
            return Vec::new();
        }
        let mut cands: Vec<u16> = Vec::new();
        for op in &b.operators {
            op.walk(&mut |n| {
                if let Value::Set(v, _) = n
                    && !cands.contains(v)
                {
                    cands.push(*v);
                }
            });
        }
        let data = self.cx.data;
        let mut out = Vec::new();
        for v in cands {
            if self.shapes.get(v as usize) != Some(&CType::Int)
                || self.sticky.iter().any(|(w, _)| *w == v)
                || !sum_only(data, b, v)
            {
                continue;
            }
            let flag = format!("ss{}", self.decls.len() + 1);
            self.decls.push(format!("int {flag} = 0;"));
            let acc = self.var(v);
            self.line(ind, &format!("{flag} = ({acc} == LOFT_NULL);"));
            self.sticky.push((v, flag.clone()));
            out.push((v, flag));
        }
        if !out.is_empty() {
            crate::rewrite_census::fired("R-StickySum", out.len());
        }
        out
    }

    /// After the loop's exit label: each sum the loop took answers null where its flag is set.
    pub(super) fn close_sticky_sums(&mut self, sums: &[(u16, String)], ind: usize) {
        for (v, flag) in sums {
            let acc = self.var(*v);
            self.line(ind, &format!("if ({flag}) {acc} = LOFT_NULL;"));
            self.sticky.retain(|(w, _)| w != v);
        }
    }

    /// `acc = acc ± e` for a sum an enclosing loop took: the plain operator, its events into
    /// the loop's flag.  Answers whether it wrote it.
    pub(super) fn sticky_update(
        &mut self,
        var: u16,
        x: &Value,
        ind: usize,
    ) -> Result<bool, String> {
        let Some(flag) = self
            .sticky
            .iter()
            .find(|(w, _)| *w == var)
            .map(|(_, f)| f.clone())
        else {
            return Ok(false);
        };
        let Value::Call(d, args) = x.unspan() else {
            return Ok(false);
        };
        let ovf = match self.cx.data.def(*d).name() {
            "OpAddInt" => "LOFT_ADD_OVF",
            "OpMinInt" => "LOFT_SUB_OVF",
            _ => return Ok(false),
        };
        let e = self.expr(&args[1], ind)?;
        let t = self.temp(CType::Int);
        self.line(ind, &format!("{t} = {};", as_int(e)?));
        if !self.never_null(&args[1]) {
            self.line(ind, &format!("{flag} |= ({t} == LOFT_NULL);"));
        }
        let acc = self.var(var);
        self.line(ind, &format!("{flag} |= {ovf}({acc}, {t}, &{acc});"));
        self.line(ind, &format!("{flag} |= ({acc} == LOFT_NULL);"));
        Ok(true)
    }
}

/// Does a `break` or `continue` in `b` leave past `b`'s own exit — to an enclosing loop?
fn leaves_past(b: &Block) -> bool {
    fn walk(v: &Value, depth: u16) -> bool {
        match v.unspan() {
            Value::Break(d) | Value::Continue(d) => *d > depth,
            Value::Loop(inner) => inner.operators.iter().any(|o| walk(o, depth + 1)),
            other => {
                let mut hit = false;
                other.for_each_child(&mut |c| hit = hit || walk(c, depth));
                hit
            }
        }
    }
    b.operators.iter().any(|o| walk(o, 0))
}

/// Is every mention of `v` in `b` an update `v = v ± e` with `e` not reading `v` — and is
/// there one at least?
fn sum_only(data: &crate::data::Data, b: &Block, v: u16) -> bool {
    fn ok(data: &crate::data::Data, n: &Value, v: u16, updates: &mut usize) -> bool {
        match n.unspan() {
            Value::Set(t, rhs) if *t == v => {
                let Value::Call(d, args) = rhs.unspan() else {
                    return false;
                };
                if !matches!(data.def(*d).name(), "OpAddInt" | "OpMinInt")
                    || args.len() != 2
                    || !matches!(args[0].unspan(), Value::Var(w) if *w == v)
                    || args[1].reads_var(v)
                {
                    return false;
                }
                *updates += 1;
                true
            }
            other if other.names_var_here(v) => false,
            other => {
                let mut good = true;
                other.for_each_child(&mut |c| good = good && ok(data, c, v, updates));
                good
            }
        }
    }
    let mut updates = 0;
    b.operators.iter().all(|o| ok(data, o, v, &mut updates)) && updates > 0
}
