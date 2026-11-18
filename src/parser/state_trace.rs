// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent)

//! @PLN91 — `LOFT_STATE_TRACE=1`: the parser's SCOPED state at fixed points, on stderr.
//!
//! The loft-in-loft elaborator (`selfhost/src/el*.loft`) mirrors this parser, and gate E
//! compares the two only by the IR they leave.  Much of that IR is decided by state a caller
//! hands down into an expression — the fields `scripts/parser_state.py` lists as *scoped* — which
//! the function at hand does not show.  Both front ends print the same lines here, so a diff of
//! the two traces names the first place the state differs, and which channel, where the IR diff
//! only says that the output does.
//!
//! One line per point: `state <pass> <line>:<col> <point> <channel>=<value>…`, the position the
//! lexer's (its peek the point's first token), each channel printed only where it differs from
//! its default.  A type is printed in a small form of its own (`Self::trace_shape`) that both
//! sides spell alike — deps left out.  Points: `stmt` (a statement of a block about to be read)
//! and `arg` (a call's argument about to be read, its hint set).
//!
//! Two points say what a pass LEFT rather than what it was handed (elaborator-instruments.md
//! § 1), in the spelling the IR dump gives a type (`Type::show`: deps by variable name), so a
//! borrow one side derived and the other did not is a differing line:
//! - `stmt-end value=<type>` — the type a statement's expression answered, at the position
//!   its `stmt` line has.  A line of its own, after the statement: the type is not known
//!   where `stmt` is printed, and holding that line back would reorder it against the points
//!   of the statements nested inside.
//! - `fn-end <fn> ret=<type>` and `fn-end <fn> var <nr> <name>=<type>` — a function's result
//!   and every variable in number order, once per pass where its variables are handed to its
//!   definition, at the definition's position.  A lambda is a function of its own.

use super::Parser;
use crate::data::Type;
use std::fmt::Write as _;

impl Parser {
    /// Print the scoped state at `point` when `LOFT_STATE_TRACE` is set.
    pub(crate) fn state_trace(&self, point: &str) {
        // The standard library is the image the elaborator loads, never a parse it repeats.
        if !crate::env_once!(std::env::var_os("LOFT_STATE_TRACE").is_some())
            || self.data.source == crate::data::STD_SOURCE
        {
            return;
        }
        let pos = self.lexer.pos();
        let mut line = format!(
            "state {} {}:{} {point}",
            if self.first_pass { 1 } else { 2 },
            pos.line,
            pos.pos
        );
        if !self.expected.is_unknown() {
            let _ = write!(line, " expected={}", self.trace_shape(&self.expected));
        }
        if self.assign_target != u16::MAX {
            // The number too: a lambda's body sees its enclosing statement's target, a variable
            // of ANOTHER frame, which the name read in this one does not show.
            let _ = write!(
                line,
                " assign_target={}({})",
                self.assign_target,
                self.vars.name(self.assign_target)
            );
        }
        if self.in_bind_rhs {
            line += " in_bind_rhs";
        }
        if !self.cur_type_vars.is_empty() {
            let names: Vec<&str> = self.cur_type_vars.iter().map(|(n, _)| n.as_str()).collect();
            let _ = write!(line, " cur_type_vars={}", names.join(","));
        }
        eprintln!("{line}");
    }

    /// Is the trace asked for, for the source being read?  The standard library is the image
    /// the elaborator loads, never a parse it repeats.
    pub(crate) fn state_trace_on(&self) -> bool {
        crate::env_once!(std::env::var_os("LOFT_STATE_TRACE").is_some())
            && self.data.source != crate::data::STD_SOURCE
    }

    /// The `route` point (elaborator-instruments.md § 2): the front end took the special route
    /// `handler` for the construct at the lexer's position — a handler's name, or
    /// `<function>=<verdict>` where one function decides between routes.
    pub(crate) fn state_route(&self, handler: &str) {
        if !self.state_trace_on() {
            return;
        }
        let pos = self.lexer.pos();
        eprintln!(
            "state {} {}:{} route {handler}",
            if self.first_pass { 1 } else { 2 },
            pos.line,
            pos.pos
        );
    }

    /// The `stmt-end` point: the type `tp` the statement read from `at` answered.
    pub(crate) fn state_trace_value(&self, at: &crate::lexer::Position, tp: &Type) {
        if !self.state_trace_on() {
            return;
        }
        eprintln!(
            "state {} {}:{} stmt-end value={}",
            if self.first_pass { 1 } else { 2 },
            at.line,
            at.pos,
            tp.show(&self.data, &self.vars)
        );
    }

    /// The `var` point (port-debug-output.md § 1): what the statement read from `at` did to
    /// the function's variable table — `+ <type>` for a variable it created, `<old> -> <new>`
    /// for one whose type or deps it changed — found by comparing the table with what this
    /// function last printed on this pass.  A statement nested in another prints first, so
    /// each change goes to the innermost statement that ended after it.
    pub(crate) fn state_trace_vars(&mut self, at: &crate::lexer::Position) {
        if !self.state_trace_on() || self.context == u32::MAX {
            return;
        }
        let now: Vec<String> = (0..self.vars.count())
            .map(|v| self.vars.tp(v).show(&self.data, &self.vars))
            .collect();
        let head = format!(
            "state {} {}:{} var {}",
            if self.first_pass { 1 } else { 2 },
            at.line,
            at.pos,
            self.data.def(self.context).name()
        );
        let seen = self
            .trace_vars
            .entry((self.context, self.first_pass))
            .or_default();
        // A table shorter than the one printed is another reading of the function.
        if seen.len() > now.len() {
            seen.clear();
        }
        for (v, tp) in now.iter().enumerate() {
            let name = self.vars.name(v as u16);
            match seen.get(v) {
                None => eprintln!("{head} {v} {name} + {tp}"),
                Some(old) if old != tp => eprintln!("{head} {v} {name} {old} -> {tp}"),
                Some(_) => {}
            }
        }
        *seen = now;
    }

    /// The `fn-end` point of definition `d`, whose variables `self.vars` still holds: its
    /// result and each variable, as this pass leaves them.
    pub(crate) fn state_trace_fn_end(&self, d: u32) {
        if !self.state_trace_on() {
            return;
        }
        let def = self.data.def(d);
        let head = format!(
            "state {} {}:{} fn-end {}",
            if self.first_pass { 1 } else { 2 },
            def.position().line,
            def.position().pos,
            def.name()
        );
        eprintln!("{head} ret={}", def.returned().show(&self.data, &self.vars));
        for v in 0..self.vars.count() {
            eprintln!(
                "{head} var {v} {}={}",
                self.vars.name(v),
                self.vars.tp(v).show(&self.data, &self.vars)
            );
        }
    }

    /// A type as the trace spells it: the shape and the definitions it names, no deps.
    fn trace_shape(&self, t: &Type) -> String {
        let name = |d: u32| self.data.def(d).name().to_string();
        match t {
            Type::Unknown(_) => "?".to_string(),
            Type::Null => "null".to_string(),
            Type::Void => "void".to_string(),
            Type::Never => "never".to_string(),
            Type::Integer(_) => "int".to_string(),
            Type::Boolean => "bool".to_string(),
            Type::Float => "float".to_string(),
            Type::Single => "single".to_string(),
            Type::Character => "char".to_string(),
            Type::Text(_) => "text".to_string(),
            Type::Optional(inner) => format!("{}?", self.trace_shape(inner)),
            Type::RefVar(inner) => format!("&{}", self.trace_shape(inner)),
            Type::Rewritten(inner) => self.trace_shape(inner),
            Type::Vector(inner, _) => format!("vector<{}>", self.trace_shape(inner)),
            Type::Reference(d, _) => format!("ref({})", name(*d)),
            Type::Enum(d, _, _) => format!("enum({})", name(*d)),
            Type::Hash(d, _, _)
            | Type::Sorted(d, _, _)
            | Type::Index(d, _, _)
            | Type::Radix(d, _, _)
            | Type::Trie(d, _, _) => format!("keyed({})", name(*d)),
            Type::Function(args, result, _, _) => {
                let a: Vec<String> = args.iter().map(|x| self.trace_shape(x)).collect();
                format!("fn({})->{}", a.join(","), self.trace_shape(result))
            }
            Type::Tuple(elems) => {
                let e: Vec<String> = elems.iter().map(|x| self.trace_shape(x)).collect();
                format!("({})", e.join(","))
            }
            Type::Iterator(..) => "iter".to_string(),
            _ => "other".to_string(),
        }
    }
}
