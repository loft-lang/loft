// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! A statement `match` over an integer subject, written as a C `switch`.
//!
//! The parser lowers `match x { 0 => a, 1 => b, … }` to a `scalar_match` block: the subject
//! bound once, then an `if` chain with each arm in the previous arm's `else`.  Written that way
//! the C nests one level per arm, and MSVC's `cl` refuses a block nested past 128 levels
//! (`C1061`, which a chained `else if` reaches too): the runner's operator dispatch has some 250
//! arms.  A `switch` holds every arm at one level.

use super::{FnWriter, bare};
use crate::data::{Block, Value};

/// A chain shorter than this keeps its nested `if`: so short a chain stays far under `cl`'s
/// limit, and its C stays what every compiler already reads.
const SWITCH_MIN_ARMS: usize = 16;

impl FnWriter<'_> {
    /// Write the `scalar_match` block `b` as the subject's bind and a `switch`, answering
    /// whether it did.  The cases are the chain's leading arms that test the bound subject
    /// against an integer literal with `==`; the chain from the first arm that does not (a
    /// guard, a range, an or-pattern, a repeated literal) is the `default`.
    ///
    /// Each arm runs exactly when the `if` chain runs it: the case literals are distinct and
    /// none is the null sentinel, so at most one equals the subject; a null subject equals none
    /// and reaches `default`, as it fails every `==`; and `default` is the rest of the chain,
    /// which the `if` chain also reaches only when every earlier test failed.  The tests have
    /// no effect and no temporary, so nothing is skipped by not evaluating them.  A block of
    /// any other shape answers `false` and is written as before.
    pub(super) fn switch_match(&mut self, b: &Block, ind: usize) -> Result<bool, String> {
        let [bind, chain] = &b.operators[..] else {
            return Ok(false);
        };
        let Value::Set(subj, _) = bind.unspan() else {
            return Ok(false);
        };
        let Some((arms, rest)) = self.switch_arms(*subj, chain) else {
            return Ok(false);
        };
        self.ops_stmt(std::slice::from_ref(bind), ind)?;
        let s = self.expr(&Value::Var(*subj), ind)?.code;
        self.line(ind, &format!("switch ({}) {{", bare(&s)));
        for (k, cond, arm) in arms {
            self.line(ind, &format!("case INT64_C({k}): {{"));
            let facts = self.grid_facts.len();
            self.grid_test_facts(cond, arm);
            let written = self.stmt(arm, ind + 1);
            self.grid_facts.truncate(facts);
            written?;
            self.line(ind, "} break;");
        }
        if !matches!(rest.unspan(), Value::Null) {
            self.line(ind, "default: {");
            self.stmt(rest, ind + 1)?;
            self.line(ind, "} break;");
        }
        self.line(ind, "}");
        self.flush(ind);
        Ok(true)
    }

    /// The chain's leading `(literal, test, arm)` cases and the rest of the chain, when there
    /// are at least [`SWITCH_MIN_ARMS`] of them.  An arm the cold-path writer claims ends the
    /// whole attempt: its `if` keeps the placement that writer gives it.
    fn switch_arms<'v>(
        &self,
        subj: u16,
        chain: &'v Value,
    ) -> Option<(Vec<(i64, &'v Value, &'v Value)>, &'v Value)> {
        let mut arms = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut cur = chain;
        while let Value::If(c, t, e) = cur.unspan() {
            if self.cold_arms.contains_key(&(&raw const **t as usize)) {
                return None;
            }
            let Some(k) = self.case_literal(subj, c) else {
                break;
            };
            if !seen.insert(k) {
                break;
            }
            arms.push((k, &**c, &**t));
            cur = e;
        }
        (arms.len() >= SWITCH_MIN_ARMS).then_some((arms, cur))
    }

    /// The literal `test` compares the subject with — `OpEqInt(subj, k)` or `OpEqLong(subj, k)`,
    /// the literal a 32-bit `Int` or, past that range, a `Long` under either operator — or
    /// `None` for any other test, a null literal included.
    fn case_literal(&self, subj: u16, test: &Value) -> Option<i64> {
        let Value::Call(op, args) = test.unspan() else {
            return None;
        };
        let [subject, lit] = &args[..] else {
            return None;
        };
        if !matches!(subject.unspan(), Value::Var(v) if *v == subj) {
            return None;
        }
        if !matches!(self.cx.data.def(*op).name(), "OpEqInt" | "OpEqLong") {
            return None;
        }
        match lit.unspan() {
            Value::Int(k) if *k != i32::MIN => Some(i64::from(*k)),
            Value::Long(k) if *k != i64::MIN => Some(*k),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::Parser;
    use std::fmt::Write as _;

    /// The C of `n_<f>` in a unit written for `src`.
    fn c_of(src: &str, f: &str) -> String {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).expect("stdlib");
        p.parse_str(src, "<switch>", false);
        assert!(
            p.diagnostics.level() < crate::diagnostics::Level::Error,
            "the program must parse clean: {:?}",
            p.diagnostics.lines()
        );
        crate::scopes::check(&mut p.data, &mut p.database);
        let unit = crate::c::emit_unit(&p.data, &p.database, &[p.data.def_nr(&format!("n_{f}"))])
            .unwrap_or_else(|r| panic!("{f} is carried: {r:?}"));
        let head = unit
            .find(&format!("n_{f}__fr(int64_t v0_x, int64_t v1_t) {{"))
            .unwrap_or_else(|| panic!("no body of {f}"));
        let body = &unit[head..];
        body[..body.find("\n}\n").map_or(body.len(), |e| e + 3)].to_string()
    }

    fn matcher(arms: usize) -> String {
        let mut s = String::from("fn step(x: integer, t: integer) -> integer {\n  match x {\n");
        for k in 0..arms {
            let _ = writeln!(s, "    {k} => {{ t += {}; }},", k * 7 + 3);
        }
        s.push_str("    _ => { t -= 1; }\n  }\n  t\n}\n");
        s
    }

    /// The deepest indent of `c`, at four columns per level.
    fn depth(c: &str) -> usize {
        c.lines()
            .map(|l| (l.len() - l.trim_start().len()) / 4)
            .max()
            .unwrap_or(0)
    }

    /// A 300-arm integer `match` is one `switch` with a case per arm, nested a few levels —
    /// MSVC's `cl` refuses a block nested past 128 (`C1061`), which the `if` chain was at
    /// 300.  A chain under `SWITCH_MIN_ARMS` keeps its `if`.
    #[test]
    fn a_long_integer_match_is_one_switch_a_short_one_its_if_chain() {
        let long = c_of(&matcher(300), "step");
        assert_eq!(long.matches("switch (").count(), 1, "one switch:\n{long}");
        assert_eq!(long.matches("case INT64_C(").count(), 300, "a case per arm");
        assert_eq!(
            long.matches("default: {").count(),
            1,
            "the wildcard is the default"
        );
        assert!(depth(&long) < 8, "nested {} levels", depth(&long));
        let short = c_of(&matcher(super::SWITCH_MIN_ARMS - 1), "step");
        assert!(
            !short.contains("switch ("),
            "a short chain keeps its if:\n{short}"
        );
    }
}
