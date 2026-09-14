// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Selection over a name's OVERLOAD SET (@PLN162): which of several definitions a call
//! reaches, decided from the argument types alone.
//!
//! `Disp-Applicable` — a definition takes the call when every argument may satisfy its
//! parameter, which is the parser's own [`Parser::can_convert`] and not a second spelling of
//! it.  `Disp-Specific` — per position, an exact spelling beats a widening (a variant to its
//! enum, `τ` into `τ?`), which beats a lossy discharge (`τ?` into `τ`, `(N-Store)`), which
//! beats any other conversion; one definition is more specific than another when it is no
//! worse at every position and better at one.  `Disp-Select` — the unique most-specific
//! applicable definition.  `Disp-Ambiguous` — two minimal ones that nothing ranks are refused
//! naming both; `Disp-Exhaustive` — none applicable is refused naming what was passed and what
//! is declared.  The nullability routing of `(F-Recv)`'s argument clause runs FIRST
//! ([`crate::data::Data::routed_types`]), so a nullable argument anywhere reaches the `τ?`
//! overload where one is declared.
//!
//! A name with no overload set — one definition, or a bound holder's stubs — is not decided
//! here at all: the caller falls through to [`crate::data::Data::select`], today's ladder,
//! which is why every program that compiled before overload sets existed is untouched.

use crate::data::{DefType, Type};
use crate::diagnostics::diagnostic_format;
use crate::parser::Parser;

/// How far a call's argument is from a parameter's declared type, per position.
/// Smaller is more specific.
type Rank = u8;
const EXACT: Rank = 0;
const WIDENED: Rank = 1;
const LOSSY: Rank = 2;
const CONVERTED: Rank = 3;

/// What selection over an overload set answered.
pub(crate) enum Selection {
    /// The unique most-specific applicable definition.
    One(u32),
    /// Two or more minimal definitions that nothing ranks.
    Ambiguous(Vec<u32>),
    /// An overload set exists and no definition takes the call.
    NoneApplicable,
    /// The name has no overload set (or an argument no spelling): not decided here.
    NotDecidable,
}

impl Parser {
    /// The rank of passing an argument of type `arg` to a parameter of type `param`, or
    /// `None` when the argument cannot satisfy the parameter.
    fn dispatch_rank(&mut self, arg: &Type, param: &Type) -> Option<Rank> {
        let (arg_opt, param_opt) = (
            matches!(arg, Type::Optional(_)),
            matches!(param, Type::Optional(_)),
        );
        let (a, p) = (arg.base(), param.base());
        let same = self.data.type_spelling(a).is_some()
            && self.data.type_spelling(a) == self.data.type_spelling(p);
        // The base relation first, then the nullability step on top of it.
        let base: Rank = if same {
            EXACT
        } else if let (Type::Reference(v_nr, _), Type::Enum(e_nr, _, _)) =
            (arg.base(), param.base())
            && self.data.def_type(*v_nr) == DefType::EnumValue
            && self.data.def(*v_nr).parent() == *e_nr
        {
            // @FR-C-Var — a variant satisfies its enum: the one widening the enum lattice has.
            WIDENED
        } else if self.can_convert(a, p) {
            CONVERTED
        } else {
            return None;
        };
        let step: Rank = match (arg_opt, param_opt) {
            (false, true) => WIDENED, // (N-Intro): a present value into a nullable slot, free
            (true, false) => LOSSY,   // (N-Store): a nullable value into a dense slot, warned
            _ => EXACT,
        };
        Some(base.saturating_add(step))
    }

    /// Every overload of `name` that takes the routed argument types, with its rank vector,
    /// or `None` when the name has no overload set or an argument has no spelling.
    fn ranked_overloads(
        &mut self,
        source: u16,
        name: &str,
        routed: &[Type],
    ) -> Option<Vec<(u32, Vec<Rank>)>> {
        let main = self.data.source_nr(source, name);
        if main == u32::MAX || self.data.def_type(main) != DefType::Dynamic {
            return None;
        }
        if routed.iter().any(|t| self.data.type_spelling(t).is_none()) {
            return None;
        }
        let routines: Vec<u32> = self
            .data
            .def(main)
            .attributes
            .iter()
            .filter_map(|a| match a.typedef.base() {
                Type::Routine(r) => Some(*r),
                _ => None,
            })
            .collect();
        let mut ranked = Vec::new();
        'routine: for r in routines {
            let declared: Vec<Type> = self
                .data
                .def(r)
                .attributes
                .iter()
                .filter(|p| !p.hidden)
                .map(|p| p.typedef.clone())
                .collect();
            if routed.len() > declared.len() {
                continue;
            }
            // A trailing parameter is admitted when it has a default (the one optionality a
            // definition carries — owner, 2026-09-14).
            let trailing_defaulted = self
                .data
                .def(r)
                .attributes
                .iter()
                .filter(|p| !p.hidden)
                .skip(routed.len())
                .all(|p| p.value != crate::data::Value::Null);
            if !trailing_defaulted {
                continue;
            }
            let mut ranks = Vec::with_capacity(routed.len());
            for (arg, param) in routed.iter().zip(&declared) {
                match self.dispatch_rank(arg, param) {
                    Some(k) => ranks.push(k),
                    None => continue 'routine,
                }
            }
            ranked.push((r, ranks));
        }
        Some(ranked)
    }

    /// `Disp-Select` over an overload set for the routed argument types.
    pub(crate) fn select_overload(
        &mut self,
        source: u16,
        name: &str,
        routed: &[Type],
    ) -> Selection {
        let Some(ranked) = self.ranked_overloads(source, name, routed) else {
            return Selection::NotDecidable;
        };
        if ranked.is_empty() {
            return Selection::NoneApplicable;
        }
        // The minimal elements of the pointwise order: a definition no other applicable one is
        // strictly better than.
        let dominated = |a: &[Rank], b: &[Rank]| -> bool {
            // b is strictly better than a
            a.iter().zip(b).all(|(x, y)| y <= x) && a.iter().zip(b).any(|(x, y)| y < x)
        };
        let minimal: Vec<u32> = ranked
            .iter()
            .filter(|(_, ra)| !ranked.iter().any(|(_, rb)| dominated(ra, rb)))
            .map(|(r, _)| *r)
            .collect();
        match minimal.as_slice() {
            [one] => Selection::One(*one),
            _ => Selection::Ambiguous(minimal),
        }
    }

    /// The two refusals selection can end in, worded once for both call spellings.
    pub(crate) fn report_selection(
        &mut self,
        name: &str,
        given: &[Type],
        sel: &Selection,
        at: Option<&crate::lexer::Position>,
    ) {
        let shown: Vec<String> = given.iter().map(|t| t.source_name(&self.data)).collect();
        let text = match sel {
            Selection::Ambiguous(taken) => {
                let by: Vec<String> = taken
                    .iter()
                    .map(|&r| self.data.overload_signature(name, r))
                    .collect();
                format!(
                    "`{name}({})` is ambiguous — it is taken by {} and nothing ranks them; give the call the arguments that pick one, or drop one definition",
                    shown.join(", "),
                    by.join(" and ")
                )
            }
            Selection::NoneApplicable => {
                let bare = self.data.def_nr(name);
                let declared: Vec<String> = self
                    .data
                    .def(bare)
                    .attributes
                    .iter()
                    .filter_map(|a| match a.typedef.base() {
                        Type::Routine(r) => Some(*r),
                        _ => None,
                    })
                    .map(|r| self.data.overload_signature(name, r))
                    .collect();
                format!(
                    "no definition of `{name}` takes ({}) — declared: {}",
                    shown.join(", "),
                    declared.join(", ")
                )
            }
            _ => return,
        };
        match at {
            Some(pos) => {
                crate::diagnostic_at!(self.lexer, pos, crate::diagnostics::Level::Error, "{text}")
            }
            None => crate::diagnostic!(self.lexer, crate::diagnostics::Level::Error, "{text}"),
        }
    }
}
