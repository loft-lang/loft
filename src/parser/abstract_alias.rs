// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN187 (@C140, @FR-F-Visible) — a non-`pub` `type` alias is ABSTRACT outside its file.
//!
//! ```loft
//! type Handle = integer;                 // units.loft: not `pub`
//! pub fn open(path: text) -> Handle { … }
//! pub fn close(h: Handle) { … }
//! ```
//!
//! Outside `units` a `Handle` is bound, passed back to a parameter declared `Handle`, stored
//! in a variable or a field declared `Handle`, compared with `==` and printed — and nothing
//! that reads its representation is allowed: an operator, a member or element read, a method
//! of the underlying type, a parameter of the underlying type, destructuring, iteration, and
//! building one from a plain value.  Inside `units`, and for a `pub type`, the alias is the
//! transparent substitution it always was.
//!
//! **Abstraction is a fact BESIDE the type, never in it.**  The parser lowers by type shape —
//! a tuple's slots, a text work buffer, a record's store — so the expression keeps the alias's
//! underlying type and everything emitted is unchanged.  What the checker tracks is
//! [`Parser::operand_alias`]: the abstract alias of the operand just parsed, or `u32::MAX`.
//! Its producers are a variable ([`Parser::abstract_vars`]), a call (the callee's declared
//! `returned_alias`) and a field read (the field's `alias_d_nr`); each consumer below checks
//! it and either accepts (the operations listed above) or refuses.  A consumer this module
//! does not name ACCEPTS — the open edges are recorded in `formal/calls.md` D-call-29.
//!
//! Enforced under `LOFT_PUB_ENFORCE=1` and counted under `LOFT_TRACE_VISIBILITY=1` (kind
//! `abstract`), exactly like the other C140 refusals until @PLN187 step 6 turns them on.

use super::{DefType, Level, Parser, Value, diagnostic_format};
use crate::data::STD_SOURCE;

impl Parser {
    /// Is the abstract-alias check live?  Second pass only: a callee or a field may still be
    /// unresolved on the first, and the census and the refusals are the second pass's.
    pub(crate) fn abstract_on(&self) -> bool {
        let trace = crate::env_once!(std::env::var_os("LOFT_TRACE_VISIBILITY").is_some());
        let enforce = crate::env_once!(std::env::var_os("LOFT_PUB_ENFORCE").is_some());
        !self.first_pass && (trace || enforce)
    }

    /// `alias` when it is abstract HERE — a non-`pub` user alias declared in another file —
    /// else `u32::MAX`.  The one judgement every producer filters through.
    pub(crate) fn abstract_here(&self, alias: u32) -> u32 {
        if alias == u32::MAX || alias == 0 || alias >= self.data.definitions() {
            return u32::MAX;
        }
        let def = self.data.def(alias);
        if def.def_type != DefType::Type
            || def.pub_visible
            || def.source == STD_SOURCE
            || def.source == self.data.source
            || def.position.file == self.lexer.pos().file
        {
            return u32::MAX;
        }
        alias
    }

    /// The abstract alias a call to `d_nr` answers: its declared result alias, when that is
    /// abstract here.
    pub(crate) fn abstract_result_of(&self, d_nr: u32) -> u32 {
        if d_nr >= self.data.definitions() {
            return u32::MAX;
        }
        self.abstract_here(self.data.def(d_nr).returned_alias)
    }

    /// The abstract alias a parameter or field `f_nr` of `d_nr` was declared with.
    pub(crate) fn abstract_attr_of(&self, d_nr: u32, f_nr: usize) -> u32 {
        if d_nr >= self.data.definitions() {
            return u32::MAX;
        }
        self.data
            .def(d_nr)
            .attributes
            .get(f_nr)
            .map_or(u32::MAX, |a| self.abstract_here(a.alias_d_nr))
    }

    /// The user function a lowered call expression invokes, through the spans and the
    /// return-buffer blocks a call can be wrapped in.  `None` for anything else — the
    /// callers then keep the operand's alias as its producer left it.
    pub(crate) fn callee_of(&self, code: &Value) -> Option<u32> {
        match code.unspan() {
            Value::Call(d, _) if *d < self.data.definitions() => {
                // An operator (`OpGetInt`, the field read itself) is the language's, not a
                // function a library declared with an alias.
                let def = self.data.def(*d);
                (matches!(
                    def.def_type,
                    DefType::Function | DefType::Dynamic | DefType::Generic
                ) && !super::is_op(&def.name))
                .then_some(*d)
            }
            // A block's VALUE is its last operator: a call, or a variable a `Set` in the
            // block filled from one (a return buffer).  Anything earlier in the block — the
            // pieces a format string is built from — is not what the block answers.
            Value::Block(bl) => match bl.operators.last().map(Value::unspan) {
                Some(Value::Var(v)) => bl.operators.iter().rev().find_map(|op| match op.unspan() {
                    Value::Set(w, inner) if w == v => self.callee_of(inner),
                    _ => None,
                }),
                Some(last) => self.callee_of(last),
                None => None,
            },
            Value::Drop(inner) => self.callee_of(inner),
            _ => None,
        }
    }

    /// After an operand's primary or one of its postfix steps: a variable carries its own
    /// alias, a call its callee's result alias, a field read the field's ([`Self::field_alias`],
    /// set by the read).  Any other shape — a literal, an operator, a format string, a block —
    /// is plain: its value is not one an abstract alias's file produced as such.
    pub(crate) fn settle_operand(&mut self, code: &Value) {
        let field = std::mem::replace(&mut self.field_alias, u32::MAX);
        if !self.abstract_on() {
            return;
        }
        self.operand_alias = match code.unspan() {
            Value::Var(v) => self
                .abstract_vars
                .get(&(self.context, *v))
                .copied()
                .unwrap_or(u32::MAX),
            other => self
                .callee_of(other)
                .map_or(field, |d| self.abstract_result_of(d)),
        };
    }

    /// A postfix step (`.field`, `.0`, `[i]`, `.method(…)`) on an abstract receiver `recv`:
    /// only a method whose `self` is declared with that same alias takes it; anything else
    /// reads the representation.
    pub(crate) fn check_postfix(&mut self, recv: u32, code: &Value) {
        if recv == u32::MAX || !self.abstract_on() {
            return;
        }
        if let Some(d) = self.callee_of(code)
            && self.abstract_attr_of(d, 0) == recv
        {
            return;
        }
        self.refuse_reveal(recv, "reading a member, an element or a method of the underlying type reads its representation");
    }

    /// A binary operator over operands whose aliases are `left` and `right`: `==` / `!=` and
    /// `??` take two values of ONE abstract alias; every other operator reads the
    /// representation.  Returns the result's alias (`??` keeps it).
    pub(crate) fn check_binary(&mut self, operator: &str, left: u32, right: u32) -> u32 {
        if (left == u32::MAX && right == u32::MAX) || !self.abstract_on() {
            return u32::MAX;
        }
        if matches!(operator, "==" | "!=" | "??") && left == right {
            return if operator == "??" { left } else { u32::MAX };
        }
        let alias = if left == u32::MAX { right } else { left };
        self.refuse_reveal(alias, &format!("`{operator}` reads its representation"));
        u32::MAX
    }

    /// A value whose alias is `got` handed to a place declared with `want` (a parameter, a
    /// field, a variable, a result).  Equal is the only acceptable answer.
    pub(crate) fn check_handover(&mut self, want: u32, got: u32, place: &str) {
        if want == got || !self.abstract_on() {
            return;
        }
        if got == u32::MAX {
            let name = self.data.def(want).name.clone();
            self.refuse_reveal(
                want,
                &format!("{place} takes a `{name}`, and a plain value is not one — only its own file builds it"),
            );
        } else {
            self.refuse_reveal(
                got,
                &format!("{place} takes its underlying type, which reads its representation"),
            );
        }
    }

    /// Each positional argument handed to the call `code` resolved to, against the parameter
    /// it lands on: a parameter declared with an alias abstract here takes only that alias,
    /// any other parameter only a plain value.  A call that resolved to no user function (a
    /// builtin, an operator, a fn-ref) declares no alias, so an abstract argument to it reads
    /// its representation.
    pub(crate) fn check_call_arguments(&mut self, code: &Value, aliases: &[u32]) {
        if !self.abstract_on() {
            return;
        }
        let callee = self.callee_of(code);
        for (i, &got) in aliases.iter().enumerate() {
            let want = callee.map_or(u32::MAX, |d| self.abstract_attr_of(d, i));
            if want != got {
                let place = callee.map_or_else(
                    || "this call".to_string(),
                    |d| {
                        let def = self.data.def(d);
                        let param = def.attributes.get(i).map_or("", |a| a.name.as_str());
                        let fname = Self::callable_name(&def.name);
                        format!("parameter `{param}` of `{fname}`")
                    },
                );
                self.check_handover(want, got, &place);
            }
        }
        self.operand_alias = callee.map_or(u32::MAX, |d| self.abstract_result_of(d));
    }

    /// `to op value`: an abstract place takes only its own alias; a plain variable that is
    /// not declared with a plain type BECOMES abstract by binding one; any other plain place
    /// (a field, an element, a variable annotated `integer`) reads the representation.  A
    /// compound operator (`+=`) is arithmetic and reads it.
    pub(crate) fn check_assignment(&mut self, op: &str, to: &Value, place: u32, value: u32) {
        if !self.abstract_on() {
            return;
        }
        let var = match to.unspan() {
            Value::Var(v) => Some(*v),
            _ => None,
        };
        let ctx = self.context;
        let place = var.map_or(place, |v| {
            self.abstract_vars
                .get(&(ctx, v))
                .copied()
                .unwrap_or(u32::MAX)
        });
        if op != "=" {
            self.check_binary(op, place, value);
            return;
        }
        match var {
            Some(v)
                if place == u32::MAX
                    && value != u32::MAX
                    && !self.plain_declared.contains(&(ctx, v)) =>
            {
                self.abstract_vars.insert((ctx, v), value);
            }
            _ => self.check_handover(place, value, "this place"),
        }
    }

    /// `v: T = …` — a local declared with an alias abstract here holds only that alias; one
    /// declared with any other type holds only plain values.
    pub(crate) fn declare_local_alias(&mut self, v: u16) {
        if !self.abstract_on() {
            return;
        }
        let alias = self.abstract_here(self.declared_alias);
        let key = (self.context, v);
        if alias == u32::MAX {
            self.abstract_vars.remove(&key);
            self.plain_declared.insert(key);
        } else {
            self.plain_declared.remove(&key);
            self.abstract_vars.insert(key, alias);
        }
    }

    /// `(a, b) = value` — destructuring reads every member.
    pub(crate) fn check_unpacked(&mut self) {
        let alias = std::mem::replace(&mut self.operand_alias, u32::MAX);
        if alias != u32::MAX && self.abstract_on() {
            self.refuse_reveal(alias, "destructuring it reads its representation");
        }
    }

    /// The value a `return` (`how`) or the body's tail hands back, against the alias the
    /// function's result was declared with.
    pub(crate) fn check_returned(&mut self, how: &str) {
        let got = std::mem::replace(&mut self.operand_alias, u32::MAX);
        if !self.abstract_on() || self.context == u32::MAX {
            return;
        }
        let want = self.abstract_result_of(self.context);
        let name = self.data.def(self.context).name.clone();
        let fname = Self::callable_name(&name).to_string();
        self.check_handover(want, got, &format!("the {how} of `{fname}`"));
    }

    /// A function body's value: its block's last operator, when that is a value and not a
    /// statement (a `return` was checked where it was written).
    pub(crate) fn check_result(&mut self, body: &Value) {
        if !self.abstract_on()
            || self.context == u32::MAX
            || matches!(
                self.data.def(self.context).returned.base(),
                crate::data::Type::Void
            )
        {
            self.operand_alias = u32::MAX;
            return;
        }
        let tail = match body.unspan() {
            Value::Block(bl) => bl.operators.last().map(Value::unspan),
            other => Some(other),
        };
        if matches!(tail, None | Some(Value::Return(_) | Value::Set(..))) {
            self.operand_alias = u32::MAX;
            return;
        }
        self.check_returned("result");
    }

    /// The name a program calls a definition by: a function's without its `n_`, a method's
    /// without its `t_<N><Type>_` receiver prefix.
    pub(crate) fn callable_name(name: &str) -> &str {
        if let Some(f) = name.strip_prefix("n_") {
            return f;
        }
        if let Some(rest) = name.strip_prefix("t_") {
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            if let Ok(len) = rest[..digits].parse::<usize>()
                && let Some(method) = rest.get(digits + len + 1..)
            {
                return method;
            }
        }
        name
    }

    /// The census line or the refusal for an abstract `alias` whose representation the
    /// program reads; `what` says how.
    pub(crate) fn refuse_reveal(&mut self, alias: u32, what: &str) {
        let trace = crate::env_once!(std::env::var_os("LOFT_TRACE_VISIBILITY").is_some());
        let enforce = crate::env_once!(std::env::var_os("LOFT_PUB_ENFORCE").is_some());
        if trace {
            self.print_census_site("abstract", alias, usize::MAX);
        }
        if enforce {
            let def = self.data.def(alias);
            let name = def.name.clone();
            let lib = Self::library_of(&def.position.file);
            diagnostic!(
                self.lexer,
                Level::Error,
                "`{name}` is abstract outside `{lib}`: {what}.\n  fix: go through `{lib}`'s \
                 functions, or `{lib}` declares `pub type {name}`"
            );
        }
    }

    /// The parameters of the function whose body is about to be parsed, seeded with the
    /// aliases they were declared with.
    pub(crate) fn seed_abstract_params(&mut self) {
        self.operand_alias = u32::MAX;
        if !self.abstract_on() || self.context == u32::MAX {
            return;
        }
        for a_nr in 0..self.data.attributes(self.context) {
            let alias = self.abstract_attr_of(self.context, a_nr);
            if alias != u32::MAX {
                self.abstract_vars
                    .insert((self.context, a_nr as u16), alias);
            }
        }
    }
}
