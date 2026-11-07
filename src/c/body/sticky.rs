// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-StickySum` (@PLN219) in the C: a loop's integer SUM — a local every WRITE of which
//! inside the loop is its own update `acc = acc + e` or `acc = acc - e` (`e` not reading `acc`)
//! — is updated with the processor's operator, one flag per loop gathering every event the
//! checked operator answers null for: `acc` null on entry, a null `e`, an overflow, a result
//! equal to the sentinel.  At the loop's exit a set flag makes `acc` null.
//!
//! The checked sum answers null from its first event on: a null `acc` stays null through every
//! later `+` and `-`, and those report nothing.  A read inside the loop is `flag ? null : acc`,
//! and at the exit the flag turns the value into the checked answer.  A TEMP the sum is staged
//! through (`t = acc + e; …; acc = t`, as `(R-SameRead)` writes it) has a flag of its own — the
//! sum's plus the step's events — so a read of `t` and a read of `acc` in between each answer
//! their own checked value.  A loop that a `break` or `continue` leaves past its own exit
//! declines.
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
                || self.sum_ranged(b, v)
            {
                continue;
            }
            let code = data.def(self.d).code();
            let Some(temps) = sum_only(data, code, b, v) else {
                continue;
            };
            if temps
                .iter()
                .any(|t| self.shapes.get(*t as usize) != Some(&CType::Int))
            {
                continue;
            }
            let flag = format!("ss{}", self.decls.len() + 1);
            self.decls.push(format!("int {flag} = 0;"));
            let acc = self.var(v);
            self.line(ind, &format!("{flag} = ({acc} == LOFT_NULL);"));
            self.sticky.push((v, flag.clone()));
            out.push((v, flag));
            for t in temps {
                let tflag = format!("ss{}", self.decls.len() + 1);
                self.decls.push(format!("int {tflag} = 0;"));
                self.sticky.push((t, tflag.clone()));
                self.sticky_temps.push((t, v));
                out.push((t, tflag));
            }
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
            self.sticky_temps.retain(|(t, _)| t != v);
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
        // The sum takes its staged temp: the temp's flag becomes the sum's.
        if let Value::Var(t) = x.unspan()
            && self.sticky_temps.iter().any(|(tt, s)| tt == t && *s == var)
            && let Some(tflag) = self
                .sticky
                .iter()
                .find(|(w, _)| w == t)
                .map(|(_, f)| f.clone())
        {
            let (acc, tn) = (self.var(var), self.var(*t));
            self.line(ind, &format!("{flag} = {tflag};"));
            self.line(ind, &format!("{acc} = {tn};"));
            return Ok(true);
        }
        let Value::Call(d, args) = x.unspan() else {
            return Ok(false);
        };
        let name = self.cx.data.def(*d).name();
        let ovf = match name {
            "OpAddInt" => "LOFT_ADD_OVF",
            "OpMinInt" => "LOFT_SUB_OVF",
            _ => return Ok(false),
        };
        // An update `(R-Range)` proves cannot fault is that rule's: plain, no flag.
        if self.range_proves(name, args) {
            return Ok(false);
        }
        // A staged temp's step reads the sum's raw value and starts from the sum's flag.
        let (source, start) = match self.sticky_temps.iter().find(|(t, _)| *t == var) {
            Some((_, s)) => {
                let sflag = self
                    .sticky
                    .iter()
                    .find(|(w, _)| w == s)
                    .map(|(_, f)| f.clone())
                    .ok_or("a staged temp without its sum")?;
                (*s, Some(sflag))
            }
            None => (var, None),
        };
        if !matches!(args[0].unspan(), Value::Var(w) if *w == source) {
            return Ok(false);
        }
        let e = self.expr(&args[1], ind)?;
        let t = self.temp(CType::Int);
        self.line(ind, &format!("{t} = {};", as_int(e)?));
        if let Some(sflag) = start {
            self.line(ind, &format!("{flag} = {sflag};"));
        }
        if !self.never_null(&args[1]) {
            self.line(ind, &format!("{flag} |= ({t} == LOFT_NULL);"));
        }
        let (acc, src) = (self.var(var), self.var(source));
        self.line(ind, &format!("{flag} |= {ovf}({src}, {t}, &{acc});"));
        self.line(ind, &format!("{flag} |= ({acc} == LOFT_NULL);"));
        Ok(true)
    }
}

impl FnWriter<'_> {
    /// Does `(R-Range)` prove every update of `v` in `b` plain?  Then the sum is that rule's.
    fn sum_ranged(&mut self, b: &Block, v: u16) -> bool {
        let mut updates: Vec<(&'static str, Vec<Value>)> = Vec::new();
        for op in &b.operators {
            op.walk(&mut |n| {
                if let Value::Set(t, rhs) = n
                    && *t == v
                    && let Value::Call(d, args) = rhs.unspan()
                {
                    let name = if self.cx.data.def(*d).name() == "OpAddInt" {
                        "OpAddInt"
                    } else {
                        "OpMinInt"
                    };
                    updates.push((name, args.clone()));
                }
            });
        }
        !updates.is_empty()
            && updates
                .iter()
                .all(|(name, args)| self.range_proves(name, args))
    }
}

impl FnWriter<'_> {
    /// A read of a sum an enclosing loop took: its checked value at this point, `flag ? null :
    /// acc` — null from the first event on.
    pub(super) fn sticky_read(&self, v: u16) -> Option<String> {
        let (_, flag) = self.sticky.iter().find(|(w, _)| *w == v)?;
        Some(format!("({flag} ? LOFT_NULL : {})", self.var(v)))
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

/// The writes of `v` in `b`: each an update `v = v ± e` (`e` not reading `v`) or `v = t` for a
/// staged temp `t` (`t_update`); at least one.  Answers the temps, or `None` where a write is
/// anything else, or `v` is handed out by reference.
fn sum_only(data: &crate::data::Data, code: &Value, b: &Block, v: u16) -> Option<Vec<u16>> {
    fn ok(
        data: &crate::data::Data,
        code: &Value,
        b: &Block,
        n: &Value,
        v: u16,
        updates: &mut usize,
        temps: &mut Vec<u16>,
    ) -> bool {
        match n.unspan() {
            Value::Set(t, rhs) if *t == v => {
                if let Value::Var(tmp) = rhs.unspan() {
                    if *tmp != v && t_update(data, code, b, *tmp, v) {
                        if !temps.contains(tmp) {
                            temps.push(*tmp);
                        }
                        *updates += 1;
                        return true;
                    }
                    return false;
                }
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
            // A plain read is answered `flag ? null : acc` (`sticky_read`) — the checked
            // sum's value at that point.  A by-reference use would hand out the raw value.
            Value::Var(w) if *w == v => true,
            Value::Call(d, args)
                if data.def(*d).name().contains("CreateStack")
                    && args
                        .iter()
                        .any(|a| matches!(a.unspan(), Value::Var(w) if *w == v)) =>
            {
                false
            }
            other if other.names_var_here(v) => false,
            other => {
                let mut good = true;
                other.for_each_child(&mut |c| {
                    good = good && ok(data, code, b, c, v, updates, temps);
                });
                good
            }
        }
    }
    let mut updates = 0;
    let mut temps = Vec::new();
    let all = b
        .operators
        .iter()
        .all(|o| ok(data, code, b, o, v, &mut updates, &mut temps));
    (all && updates > 0).then_some(temps)
}

/// Is `t` a temp the sum `v` is staged through: written once in `b`, as `t = v ± e` with `e`
/// reading neither, and nowhere else in the function but by a null init — never by reference?
fn t_update(data: &crate::data::Data, code: &Value, b: &Block, t: u16, v: u16) -> bool {
    let mut in_loop = 0;
    let mut shaped = true;
    for op in &b.operators {
        op.walk(&mut |n| {
            if let Value::Set(x, rhs) = n
                && *x == t
            {
                in_loop += 1;
                shaped &= matches!(rhs.unspan(), Value::Call(d, a)
                    if matches!(data.def(*d).name(), "OpAddInt" | "OpMinInt")
                        && a.len() == 2
                        && matches!(a[0].unspan(), Value::Var(w) if *w == v)
                        && !a[1].reads_var(v)
                        && !a[1].reads_var(t));
            }
        });
    }
    let mut elsewhere = 0;
    let mut by_ref = false;
    code.walk(&mut |n| match n {
        Value::Set(x, rhs) if *x == t && !matches!(rhs.unspan(), Value::Null) => elsewhere += 1,
        Value::Call(d, args)
            if data.def(*d).name().contains("CreateStack")
                && args
                    .iter()
                    .any(|a| matches!(a.unspan(), Value::Var(w) if *w == t)) =>
        {
            by_ref = true;
        }
        _ => {}
    });
    in_loop == 1 && shaped && elsewhere == 1 && !by_ref
}
