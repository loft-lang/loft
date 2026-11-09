// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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
