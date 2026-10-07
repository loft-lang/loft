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

use super::{DefType, Level, Parser, Type, Value, diagnostic_format};
use crate::data::{Data, STD_SOURCE};

/// The high bit of an alias fact: a VECTOR whose elements are that alias (`vector<Handle>`).
/// Its element read, its iteration and its container operations (`len`, `insert`, …) are
/// open; reading an element as the underlying type is not.  A def number never reaches it.
pub(crate) const ELEM: u32 = 1 << 31;
/// A vector literal with no elements: it fits a `vector<Handle>` place as well as a plain one.
pub(crate) const EMPTY: u32 = ELEM;

/// Is `a` the `vector<alias>` fact (`u32::MAX`, the plain answer, has the bit set too)?
pub(crate) const fn is_elem(a: u32) -> bool {
    a != u32::MAX && a & ELEM != 0
}

/// Does a value whose fact is `got` fit a place declared `want`?
pub(crate) const fn fits(want: u32, got: u32) -> bool {
    want == got || (got == EMPTY && (want == u32::MAX || is_elem(want)))
}

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
        let base = alias & !ELEM;
        if alias == u32::MAX || base == 0 || base >= self.data.definitions() {
            return u32::MAX;
        }
        let def = self.data.def(base);
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

    /// A postfix step on an abstract receiver `recv`.  An element read of a `vector<Handle>`
    /// answers a `Handle` (`result_is_vector`: a slice answers the vector).  A method takes the
    /// receiver as its first parameter ([`Self::param_takes`]).  Anything else — a member,
    /// `.0`, an index into a `Handle` itself — reads the representation.
    pub(crate) fn check_postfix(
        &mut self,
        recv: u32,
        is_index: bool,
        result_is_vector: bool,
        code: &Value,
    ) {
        if recv == u32::MAX || recv == EMPTY || !self.abstract_on() {
            return;
        }
        if is_index && is_elem(recv) {
            self.operand_alias = if result_is_vector { recv } else { recv & !ELEM };
            return;
        }
        // A method on a `vector<Handle>` is the vector method of that name; what an argument
        // of it called last is not (`v.insert(0, open(…))` recorded `open`).
        let member = std::mem::take(&mut self.postfix_member);
        if !is_index && is_elem(recv) && !member.is_empty() {
            let d = self.data.def_nr(&format!("t_6vector_{member}"));
            if d != u32::MAX {
                self.last_called = d;
            }
        }
        if !is_index && let Some(d) = self.chosen_callee(code) {
            let mut bound = u32::MAX;
            if self.param_takes(d, 0, recv, &mut bound) {
                self.operand_alias = self.call_result(d, bound);
                return;
            }
        }
        self.refuse_reveal(
            recv,
            "reading a member, an element or a method of the underlying type reads its representation",
        );
    }

    /// The definition a call or method step selected: the one `call_with_named` recorded
    /// (a builtin lowered to an operator, `sort` → `OpSortVector`, is still `sort`), else the
    /// user function the lowered code calls.
    pub(crate) fn chosen_callee(&self, code: &Value) -> Option<u32> {
        if self.last_called < self.data.definitions() {
            Some(self.last_called)
        } else {
            self.callee_of(code)
        }
    }

    /// A builtin over a vector (`insert`, `sort`, `len`) is a special form: it lowers to an
    /// operator without passing `call_with_named`, so nothing recorded its definition.  Its
    /// declaration is still the vector method of that name, which says whether it looks at the
    /// elements (`sort<T: Ordered>`) or not (`insert<T>`).
    pub(crate) fn recall_vector_builtin(&mut self, name: &str, receiver: Option<u32>) {
        if self.last_called == u32::MAX && receiver.is_some_and(is_elem) && self.abstract_on() {
            self.last_called = self.data.def_nr(&format!("t_6vector_{name}"));
        }
    }

    /// The template a generic instance (`i_7integer_n_sum`) was minted from, else `d`.
    fn template_of(&self, d: u32) -> u32 {
        let name = &self.data.def(d).name;
        if let Some(k) = Data::split_key(name)
            && k.kind == crate::data::KeyKind::Instance
        {
            let t = self.data.def_nr(k.rest);
            if t != u32::MAX {
                return t;
            }
        }
        d
    }

    /// Does parameter `param` of `callee` take a value whose fact is `got`?  A parameter declared with
    /// that alias does; so does one that never looks at what it holds: an untyped `vector`, or
    /// a `vector<T>` / `T` of a generic whose type variables carry no bound (`insert`,
    /// `reverse`).  A bound (`sort<T: Ordered>`, `sum<T: Addable>`) or a concrete type reads
    /// the representation.  `bound` receives what the type variable stands for.
    pub(crate) fn param_takes(&self, callee: u32, param: usize, got: u32, bound: &mut u32) -> bool {
        if fits(self.abstract_attr_of(callee, param), got) {
            return true;
        }
        if got == u32::MAX || got == EMPTY {
            return false;
        }
        let template = self.template_of(callee);
        let def = self.data.def(template);
        let Some(attr) = def.attributes.get(param) else {
            return false;
        };
        let unbounded = def.bounds.is_empty();
        let type_var = |tp: &Type| matches!(tp.base(), Type::Reference(n, _) if self.data.is_type_var_placeholder(*n));
        match attr.typedef.base() {
            Type::Vector(elem, _) if is_elem(got) => {
                if matches!(**elem, Type::Unknown(_)) {
                    return true;
                }
                if unbounded && type_var(elem) {
                    *bound = got & !ELEM;
                    return true;
                }
                false
            }
            tp if !is_elem(got) && unbounded && type_var(tp) => {
                *bound = got;
                true
            }
            _ => false,
        }
    }

    /// The fact a call to `d` answers: its declared result alias, or — for a generic whose
    /// type variable an argument bound to an abstract alias — that alias in the result's place.
    fn call_result(&self, d: u32, bound: u32) -> u32 {
        let declared = self.abstract_result_of(d);
        if declared != u32::MAX || bound == u32::MAX {
            return declared;
        }
        let t = self.template_of(d);
        let type_var = |tp: &Type| matches!(tp.base(), Type::Reference(n, _) if self.data.is_type_var_placeholder(*n));
        match self.data.def(t).returned.base() {
            Type::Vector(elem, _) if type_var(elem) => bound | ELEM,
            tp if type_var(tp) => bound,
            _ => u32::MAX,
        }
    }

    /// A binary operator over operands whose aliases are `left` and `right`: `==` / `!=` and
    /// `??` take two values of ONE abstract alias; every other operator reads the
    /// representation.  Returns the result's alias (`??` keeps it).
    pub(crate) fn check_binary(&mut self, operator: &str, left: u32, right: u32) -> u32 {
        let plain = |a: u32| a == u32::MAX || a == EMPTY;
        if (plain(left) && plain(right)) || !self.abstract_on() {
            return if left == EMPTY && right == EMPTY {
                EMPTY
            } else {
                u32::MAX
            };
        }
        if matches!(operator, "==" | "!=" | "??") && left == right {
            return if operator == "??" { left } else { u32::MAX };
        }
        // Joining two vectors of one alias is a container operation: `v += [h]`, `v + w`.
        if matches!(operator, "+" | "+=")
            && (is_elem(left) || is_elem(right))
            && (fits(left, right) || fits(right, left))
        {
            return if left == EMPTY { right } else { left };
        }
        let alias = if plain(left) { right } else { left };
        self.refuse_reveal(alias, &format!("`{operator}` reads its representation"));
        u32::MAX
    }

    /// A value whose alias is `got` handed to a place declared with `want` (a parameter, a
    /// field, a variable, a result).  Equal is the only acceptable answer.
    pub(crate) fn check_handover(&mut self, want: u32, got: u32, place: &str) {
        if fits(want, got) || !self.abstract_on() {
            return;
        }
        if got == u32::MAX || got == EMPTY {
            let name = self.alias_spelling(want);
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
    /// it lands on ([`Self::param_takes`]).  A call that selected no definition (a fn-ref, an
    /// operator) declares nothing, so an abstract argument to it reads its representation.
    pub(crate) fn check_call_arguments(&mut self, code: &Value, aliases: &[u32]) {
        if !self.abstract_on() {
            return;
        }
        let callee = self.chosen_callee(code);
        let mut bound = u32::MAX;
        for (i, &got) in aliases.iter().enumerate() {
            let takes = callee.is_some_and(|d| self.param_takes(d, i, got, &mut bound));
            if takes || (callee.is_none() && got == u32::MAX) {
                continue;
            }
            let want = callee.map_or(u32::MAX, |d| self.abstract_attr_of(d, i));
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
        self.operand_alias = callee.map_or(u32::MAX, |d| self.call_result(d, bound));
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
                    && value != EMPTY
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
        // a generic's instance (`i_7integer_n_sum`) is called by its template's name
        if let Some(k) = Data::split_key(name)
            && k.kind == crate::data::KeyKind::Instance
        {
            return Self::callable_name(k.rest);
        }
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
            self.print_census_site("abstract", alias & !ELEM, usize::MAX);
        }
        if enforce {
            let def = self.data.def(alias & !ELEM);
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

    /// How a diagnostic spells a fact: `Handle`, or `vector<Handle>`.
    fn alias_spelling(&self, a: u32) -> String {
        let name = &self.data.def(a & !ELEM).name;
        if is_elem(a) {
            format!("vector<{name}>")
        } else {
            name.clone()
        }
    }

    /// A vector literal whose elements' facts are `elements`: all one alias makes a
    /// `vector<Handle>`; none makes [`EMPTY`]; a plain element beside a `Handle` builds one
    /// from a plain value.  The answer waits in [`Self::field_alias`] for `settle_operand`.
    pub(crate) fn settle_vector_literal(&mut self, elements: &[u32]) {
        if !self.abstract_on() {
            return;
        }
        let Some(&first) = elements.iter().find(|&&a| a != u32::MAX) else {
            self.field_alias = if elements.is_empty() { EMPTY } else { u32::MAX };
            return;
        };
        if is_elem(first) {
            // a vector of vectors: the nested level is not followed (D-call-29)
            self.field_alias = u32::MAX;
            return;
        }
        for &a in elements {
            if a != first {
                self.check_handover(first, a, "an element of this vector");
            }
        }
        self.field_alias = first | ELEM;
    }

    /// `for x in <iterable>`: the iterable's fact (`iterable`) and the loop variable.  The
    /// elements of a `vector<Handle>` are `Handle`s; iterating a `Handle` itself reads it.
    pub(crate) fn bind_loop_variable(&mut self, iterable: u32, var: u16) {
        if iterable == u32::MAX || iterable == EMPTY || !self.abstract_on() {
            return;
        }
        if is_elem(iterable) && var != u16::MAX {
            self.abstract_vars
                .insert((self.context, var), iterable & !ELEM);
        } else if !is_elem(iterable) {
            self.refuse_reveal(iterable, "iterating it reads its representation");
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
