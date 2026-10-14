// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I59 — Type resolver

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
//! of the underlying type, a parameter of the underlying type, destructuring it, iterating
//! it, branching or matching on it, and building one from a plain value.  Inside `units`, and
//! for a `pub type`, the alias is the transparent substitution it always was.
//!
//! **Abstraction is a fact BESIDE the type, never in it.**  The parser lowers by type shape —
//! a tuple's slots, a text work buffer, a record's store — so the expression keeps the alias's
//! underlying type and everything emitted is unchanged.  What the checker tracks is an
//! [`AliasFact`] per operand ([`Parser::operand_fact`]): where in the value an abstract alias
//! sits — the value itself, a vector's elements, a tuple's members.  A declaration records
//! the fact of its spelling (`Attribute::fact`, `Definition::returned_fact`); the producers
//! are a variable ([`Parser::abstract_vars`]), a call, a field read, a literal, a branch's
//! value and an element or member read; each consumer below accepts what fits the place it
//! hands the value to and refuses what reads the representation.
//!
//! Refused on every build, like the other C140 refusals, and counted under
//! `LOFT_TRACE_VISIBILITY=1` (kind `abstract`).

use super::{DefType, Level, Parser, Type, Value, diagnostic_format};
use crate::data::{AliasFact, Data, STD_SOURCE};

/// Does a value whose fact is `got` fit a place declared `want`?  An empty vector literal
/// fits any vector; a vector or a tuple fits position by position.
pub(crate) fn fits(want: &AliasFact, got: &AliasFact) -> bool {
    match (want, got) {
        (w, g) if w == g => true,
        (AliasFact::Plain | AliasFact::Vector(_), AliasFact::Empty) => true,
        (AliasFact::Vector(w), AliasFact::Vector(g)) => fits(w, g),
        (AliasFact::Tuple(ws), AliasFact::Tuple(gs)) => {
            ws.len() == gs.len() && ws.iter().zip(gs).all(|(w, g)| fits(w, g))
        }
        // a lambda handed to a place that is not a generic's `fn(…) -> U`: what it yields
        // must be plain, since the place reads it as its declared type
        (AliasFact::Plain, AliasFact::Lambda(r)) => is_plainish(r),
        _ => false,
    }
}

/// The first alias a fact names, position by position.
fn first_alias(f: &AliasFact) -> Option<u32> {
    match f {
        AliasFact::Alias(a) => Some(*a),
        AliasFact::Vector(inner) => first_alias(inner),
        AliasFact::Tuple(ms) => ms.iter().find_map(first_alias),
        AliasFact::Lambda(r) => first_alias(r),
        AliasFact::Plain | AliasFact::Empty => None,
    }
}

/// A fact that names no alias: a plain value or an empty vector literal.
fn is_plainish(f: &AliasFact) -> bool {
    match f {
        AliasFact::Plain | AliasFact::Empty => true,
        AliasFact::Lambda(r) => is_plainish(r),
        _ => false,
    }
}

/// What each of a generic's type variables stands for, as the arguments bound them.
pub(crate) type Bounds = Vec<(u32, AliasFact)>;

fn bound_of(bounds: &Bounds, var: u32) -> AliasFact {
    bounds
        .iter()
        .find(|(v, _)| *v == var)
        .map_or(AliasFact::Plain, |(_, f)| f.clone())
}

/// One step of an operand's postfix chain.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// `[i]` — an element of a vector, or a keyed lookup.
    Index,
    /// `.name` / `.0` / `.method(…)`.
    Member,
    /// `x?` — the default fallback.
    Fallback,
}

impl Parser {
    /// Is the abstract-alias check live?  Second pass only: a callee or a field may still be
    /// unresolved on the first, and the census and the refusals are the second pass's.
    pub(crate) fn abstract_on(&self) -> bool {
        !self.first_pass
    }

    /// Is `alias` abstract HERE — a non-`pub` user alias declared in another file?  The one
    /// judgement every fact goes through.
    pub(crate) fn is_abstract(&self, alias: u32) -> bool {
        if alias == 0 || alias >= self.data.definitions() {
            return false;
        }
        let def = self.data.def(alias);
        def.def_type == DefType::Type
            && !def.pub_visible
            && def.source != STD_SOURCE
            && def.source != self.data.source
            && def.position.file != self.lexer.pos().file
    }

    /// A recorded fact as THIS file sees it: an alias abstract here stays; one that is not
    /// (its own file, a `pub type`) is its right-hand side's fact — `pub type Pair =
    /// (Handle, u8)` is transparent, and its first member is still a `Handle`.
    pub(crate) fn resolve(&self, f: &AliasFact) -> AliasFact {
        self.resolve_at(f, 0)
    }

    fn resolve_at(&self, f: &AliasFact, depth: u8) -> AliasFact {
        match f {
            AliasFact::Alias(a) if self.is_abstract(*a) => AliasFact::Alias(*a),
            AliasFact::Alias(a) if depth < 16 && *a < self.data.definitions() => {
                let rhs = self.data.def(*a).returned_fact.clone();
                self.resolve_at(&rhs, depth + 1)
            }
            AliasFact::Vector(inner) => AliasFact::vector(self.resolve_at(inner, depth + 1)),
            AliasFact::Tuple(ms) => {
                AliasFact::tuple(ms.iter().map(|m| self.resolve_at(m, depth + 1)).collect())
            }
            AliasFact::Empty => AliasFact::Empty,
            AliasFact::Lambda(r) => AliasFact::Lambda(Box::new(self.resolve_at(r, depth + 1))),
            _ => AliasFact::Plain,
        }
    }

    /// What a call to `d_nr` answers, as this file sees its declared result.
    pub(crate) fn result_fact_of(&self, d_nr: u32) -> AliasFact {
        if d_nr >= self.data.definitions() {
            return AliasFact::Plain;
        }
        self.resolve(&self.data.def(d_nr).returned_fact)
    }

    /// What parameter or field `f_nr` of `d_nr` holds, as this file sees its declaration.
    pub(crate) fn attr_fact_of(&self, d_nr: u32, f_nr: usize) -> AliasFact {
        if d_nr >= self.data.definitions() {
            return AliasFact::Plain;
        }
        self.data
            .def(d_nr)
            .attributes
            .get(f_nr)
            .map_or(AliasFact::Plain, |a| self.resolve(&a.fact))
    }

    /// How a diagnostic spells a fact: `Handle`, `vector<Handle>`, `(Handle, _)`.
    fn spelling(&self, f: &AliasFact) -> String {
        match f {
            AliasFact::Alias(a) => self.data.def(*a).name.clone(),
            AliasFact::Vector(inner) => format!("vector<{}>", self.spelling(inner)),
            AliasFact::Tuple(ms) => format!(
                "({})",
                ms.iter()
                    .map(|m| self.spelling(m))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            AliasFact::Lambda(r) => format!("fn(…) -> {}", self.spelling(r)),
            AliasFact::Plain | AliasFact::Empty => "_".to_string(),
        }
    }

    /// The user function a lowered call expression invokes, through the spans and the
    /// return-buffer blocks a call can be wrapped in.  `None` for anything else.
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

    /// After an operand's primary or one of its postfix steps: a producer's fact (a field
    /// read, a literal, a branch's value — [`Self::produced`]) if one ran, else a variable's,
    /// else a call's result.  Any other shape — an operator, a format string — is plain.
    pub(crate) fn settle_operand(&mut self, code: &Value) {
        let produced = self.produced.take();
        if !self.abstract_on() {
            return;
        }
        self.operand_fact = if let Some(f) = produced {
            f
        } else {
            match code.unspan() {
                Value::Var(v) => self
                    .abstract_vars
                    .get(&(self.context, *v))
                    .cloned()
                    .unwrap_or_default(),
                other => self
                    .callee_of(other)
                    .map_or(AliasFact::Plain, |d| self.result_fact_of(d)),
            }
        };
    }

    /// A postfix step on a receiver whose fact is `recv`.  An element of a `vector<Handle>`
    /// is a `Handle` (`result_is_vector`: a slice is the vector); member `i` of a tuple is its
    /// member's fact; a method takes the receiver as its first parameter
    /// ([`Self::param_takes`]).  Anything else — a field or index of a `Handle` itself — reads
    /// the representation.
    pub(crate) fn check_postfix(
        &mut self,
        recv: &AliasFact,
        step: Step,
        result_is_vector: bool,
        code: &Value,
    ) {
        let member = std::mem::take(&mut self.postfix_member);
        // a method call was checked as a call, receiver included (`parse_method_selecting`)
        if std::mem::take(&mut self.method_checked) || is_plainish(recv) || !self.abstract_on() {
            return;
        }
        match (step, recv) {
            (Step::Fallback, _) => {
                self.operand_fact = recv.clone();
                return;
            }
            (Step::Index, AliasFact::Vector(inner)) => {
                self.operand_fact = if result_is_vector {
                    recv.clone()
                } else {
                    (**inner).clone()
                };
                return;
            }
            (Step::Member, AliasFact::Tuple(ms)) => {
                let at = match code.unspan() {
                    Value::TupleGet(_, i) => Some(*i as usize),
                    _ => member.parse::<usize>().ok(),
                };
                if let Some(m) = at.and_then(|i| ms.get(i)) {
                    self.operand_fact = m.clone();
                    return;
                }
            }
            _ => {}
        }
        if step == Step::Member {
            // A method on a `vector<Handle>` is the vector method of that name; what an
            // argument of it called last is not (`v.insert(0, open(…))` recorded `open`).
            if matches!(recv, AliasFact::Vector(_)) && !member.is_empty() {
                let d = self.data.def_nr(&format!("t_6vector_{member}"));
                if d != u32::MAX {
                    self.last_called = d;
                }
            }
            if let Some(d) = self.chosen_callee(code) {
                let mut bounds = Bounds::new();
                if self.param_takes(d, 0, recv, &mut bounds) {
                    self.operand_fact = self.call_result(d, &bounds);
                    return;
                }
            }
        }
        let alias = first_alias(recv).unwrap_or(0);
        self.refuse_reveal(
            alias,
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
    pub(crate) fn recall_vector_builtin(&mut self, name: &str, receiver: Option<&AliasFact>) {
        if self.last_called == u32::MAX
            && matches!(receiver, Some(AliasFact::Vector(_)))
            && self.abstract_on()
        {
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

    /// The type variable `tp` is, if it is one of a generic's.
    fn type_var(&self, tp: &Type) -> Option<u32> {
        match tp.base() {
            Type::Reference(n, _) if self.data.is_type_var_placeholder(*n) => Some(*n),
            _ => None,
        }
    }

    /// Does parameter `param` of `callee` take a value whose fact is `got`?  A parameter whose
    /// declaration fits it does; so does one that never looks at what it holds: an untyped
    /// `vector`, or a `vector<T>` / `T` of a generic whose type variables carry no bound
    /// (`insert`, `reverse`).  A bound (`sort<T: Ordered>`, `sum<T: Addable>`) or a concrete
    /// type reads the representation.  `bound` receives what the type variable stands for.
    pub(crate) fn param_takes(
        &self,
        callee: u32,
        param: usize,
        got: &AliasFact,
        bounds: &mut Bounds,
    ) -> bool {
        let want = self.param_want(callee, param, bounds);
        if fits(&want, got) {
            return true;
        }
        let template = self.template_of(callee);
        let def = self.data.def(template);
        let Some(attr) = def.attributes.get(param) else {
            return false;
        };
        // what an earlier argument bound decides: `insert(v, 0, 5)` with `v: vector<Handle>`
        if !want.is_plain() {
            return false;
        }
        if is_plainish(got) {
            return false;
        }
        let unbounded = def.bounds.is_empty();
        match (attr.typedef.base(), got) {
            (Type::Vector(elem, _), AliasFact::Vector(inner)) => {
                if matches!(elem.base(), Type::Unknown(_)) {
                    return true;
                }
                if unbounded && let Some(v) = self.type_var(elem) {
                    bounds.push((v, (**inner).clone()));
                    return true;
                }
                false
            }
            // `fn(T) -> U` taking a lambda: what the lambda yields is what `U` stands for
            (Type::Function(_, ret, ..), AliasFact::Lambda(r)) => {
                if unbounded && let Some(v) = self.type_var(ret) {
                    bounds.push((v, (**r).clone()));
                    return true;
                }
                false
            }
            (tp, _) if unbounded => self.type_var(tp).is_some_and(|v| {
                bounds.push((v, got.clone()));
                true
            }),
            _ => false,
        }
    }

    /// What parameter `param` of `callee` takes: what its declaration names, or — for a
    /// generic's `T` / `vector<T>` an earlier argument bound to an abstract fact — that fact.
    fn param_want(&self, callee: u32, param: usize, bounds: &Bounds) -> AliasFact {
        let declared = self.attr_fact_of(callee, param);
        if !declared.is_plain() {
            return declared;
        }
        let template = self.template_of(callee);
        let Some(attr) = self.data.def(template).attributes.get(param) else {
            return declared;
        };
        match attr.typedef.base() {
            Type::Vector(elem, _) => self
                .type_var(elem)
                .map_or(declared, |v| AliasFact::vector(bound_of(bounds, v))),
            tp => self.type_var(tp).map_or(declared, |v| bound_of(bounds, v)),
        }
    }

    /// The fact a call to `d` answers: its declared result, or — for a generic whose type
    /// variables the arguments bound to abstract facts — those facts in the result's place.
    fn call_result(&self, d: u32, bounds: &Bounds) -> AliasFact {
        let declared = self.result_fact_of(d);
        if !declared.is_plain() || bounds.is_empty() {
            return declared;
        }
        let template = self.template_of(d);
        match self.data.def(template).returned.base() {
            Type::Vector(elem, _) => self
                .type_var(elem)
                .map_or(AliasFact::Plain, |v| AliasFact::vector(bound_of(bounds, v))),
            tp => self
                .type_var(tp)
                .map_or(AliasFact::Plain, |v| bound_of(bounds, v)),
        }
    }

    /// The facts of the parameters a lambda passed at `param` of `callee` receives: a
    /// generic's `fn(T, …)` parameter whose `T` an earlier argument bound (`map(v, |x| …)`
    /// with `v: vector<Handle>` makes `x` a `Handle`).
    pub(crate) fn lambda_param_facts(
        &self,
        callee: u32,
        param: usize,
        bounds: &Bounds,
    ) -> Vec<AliasFact> {
        if bounds.is_empty() || callee >= self.data.definitions() {
            return Vec::new();
        }
        let template = self.template_of(callee);
        let Some(attr) = self.data.def(template).attributes.get(param) else {
            return Vec::new();
        };
        match attr.typedef.base() {
            Type::Function(ps, ..) => ps
                .iter()
                .map(|p| {
                    self.type_var(p)
                        .map_or(AliasFact::Plain, |v| bound_of(bounds, v))
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Before argument `arg` of a call to `name` is parsed: when it is a lambda handed to a
    /// generic whose type variable the arguments before it bound (`map(v, |x| …)`), the
    /// lambda's parameters start out with those facts ([`Self::pending_lambda_facts`]).
    pub(crate) fn prepare_lambda_argument(&mut self, name: &str, arg: usize, before: &[AliasFact]) {
        if !self.abstract_on()
            || !(self.lexer.peek_token("|")
                || self.lexer.peek_token("||")
                || self.lexer.peek_token("fn"))
            || before.iter().all(is_plainish)
        {
            return;
        }
        let mut callee = self.data.def_nr(&format!("n_{name}"));
        if callee == u32::MAX && matches!(before.first(), Some(AliasFact::Vector(_))) {
            callee = self.data.def_nr(&format!("t_6vector_{name}"));
        }
        if callee == u32::MAX {
            return;
        }
        let mut bounds = Bounds::new();
        for (i, f) in before.iter().enumerate() {
            self.param_takes(callee, i, f, &mut bounds);
        }
        self.pending_lambda_facts = self.lambda_param_facts(callee, arg, &bounds);
    }

    /// A binary operator over operands whose facts are `left` and `right`: `==` / `!=` and
    /// `??` take two values of ONE fact; `+` / `+=` join two vectors of one fact; every other
    /// operator reads the representation.  Returns the result's fact.
    pub(crate) fn check_binary(
        &mut self,
        operator: &str,
        left: &AliasFact,
        right: &AliasFact,
    ) -> AliasFact {
        if (is_plainish(left) && is_plainish(right)) || !self.abstract_on() {
            return AliasFact::Plain;
        }
        if matches!(operator, "==" | "!=" | "??") && (fits(left, right) || fits(right, left)) {
            return if operator == "??" {
                left.clone()
            } else {
                AliasFact::Plain
            };
        }
        if matches!(operator, "+" | "+=")
            && (matches!(left, AliasFact::Vector(_)) || matches!(right, AliasFact::Vector(_)))
            && (fits(left, right) || fits(right, left))
        {
            return if *left == AliasFact::Empty {
                right.clone()
            } else {
                left.clone()
            };
        }
        let side = if is_plainish(left) { right } else { left };
        let alias = first_alias(side).unwrap_or(0);
        self.refuse_reveal(alias, &format!("`{operator}` reads its representation"));
        AliasFact::Plain
    }

    /// A value whose fact is `got` handed to a place declared `want` (a parameter, a field,
    /// a variable, a result, an element).
    pub(crate) fn check_handover(&mut self, want: &AliasFact, got: &AliasFact, place: &str) {
        if fits(want, got) || !self.abstract_on() {
            return;
        }
        match (first_alias(want), first_alias(got)) {
            (Some(w), None) => {
                let spelled = self.spelling(want);
                self.refuse_reveal(
                    w,
                    &format!(
                        "{place} takes a `{spelled}`, and a plain value is not one — only its own file builds it"
                    ),
                );
            }
            (None, Some(g)) => self.refuse_reveal(
                g,
                &format!("{place} takes its underlying type, which reads its representation"),
            ),
            (_, g) => {
                let (w, gs) = (self.spelling(want), self.spelling(got));
                self.refuse_reveal(
                    g.unwrap_or(0),
                    &format!("{place} takes a `{w}`, not a `{gs}`"),
                );
            }
        }
    }

    /// Each argument handed to the call `code` resolved to, against the parameter it lands
    /// on ([`Self::param_takes`]) — positional by position, named by the parameter's name.  A
    /// call that selected no definition (a fn-ref, an operator) declares nothing, so an
    /// abstract argument to it reads its representation.
    pub(crate) fn check_call_arguments(
        &mut self,
        code: &Value,
        positional: &[AliasFact],
        named: &[(String, AliasFact)],
    ) {
        if !self.abstract_on() {
            return;
        }
        let callee = self.chosen_callee(code);
        let mut bounds = Bounds::new();
        let mut args: Vec<(usize, &AliasFact)> = positional.iter().enumerate().collect();
        if let Some(d) = callee {
            for (name, fact) in named {
                if let Some(i) = self
                    .data
                    .def(d)
                    .attributes
                    .iter()
                    .position(|a| a.name == *name)
                {
                    args.push((i, fact));
                }
            }
        }
        for (i, got) in args {
            let takes = callee.is_some_and(|d| self.param_takes(d, i, got, &mut bounds));
            if takes || (callee.is_none() && is_plainish(got)) {
                continue;
            }
            let want = callee.map_or(AliasFact::Plain, |d| self.param_want(d, i, &bounds));
            let place = callee.map_or_else(
                || "this call".to_string(),
                |d| {
                    let def = self.data.def(d);
                    let param = def.attributes.get(i).map_or("", |a| a.name.as_str());
                    let fname = Self::callable_name(&def.name);
                    format!("parameter `{param}` of `{fname}`")
                },
            );
            self.check_handover(&want, got, &place);
        }
        // handed on through `produced`: the lowered call names the generic's INSTANCE, whose
        // declared result knows nothing of what its type variables were bound to here
        let result = callee.map_or(AliasFact::Plain, |d| self.call_result(d, &bounds));
        self.operand_fact = result.clone();
        self.produced = Some(result);
    }

    /// `to op value`: a place takes only what fits its fact; a plain variable that is not
    /// declared with a plain type BECOMES what it binds; any other plain place (a field, an
    /// element, a variable annotated `integer`) reads the representation.  A compound
    /// operator is an operator ([`Self::check_binary`]).
    pub(crate) fn check_assignment(
        &mut self,
        op: &str,
        to: &Value,
        place: &AliasFact,
        value: &AliasFact,
    ) {
        if !self.abstract_on() {
            return;
        }
        let var = match to.unspan() {
            Value::Var(v) => Some(*v),
            _ => None,
        };
        let ctx = self.context;
        let place = var.map_or_else(
            || place.clone(),
            |v| {
                self.abstract_vars
                    .get(&(ctx, v))
                    .cloned()
                    .unwrap_or_default()
            },
        );
        if op != "=" {
            self.check_binary(op, &place, value);
            return;
        }
        match var {
            Some(v)
                if place.is_plain()
                    && !is_plainish(value)
                    && !self.plain_declared.contains(&(ctx, v)) =>
            {
                self.abstract_vars.insert((ctx, v), value.clone());
            }
            _ => self.check_handover(&place, value, "this place"),
        }
    }

    /// `v: T = …` — a local declared with a type that names an abstract alias holds only what
    /// fits it; one declared plain holds only plain values.
    pub(crate) fn declare_local_alias(&mut self, v: u16) {
        if !self.abstract_on() {
            return;
        }
        let fact = self.resolve(&self.type_fact.clone());
        let key = (self.context, v);
        if fact.is_plain() {
            self.abstract_vars.remove(&key);
            self.plain_declared.insert(key);
        } else {
            self.plain_declared.remove(&key);
            self.abstract_vars.insert(key, fact);
        }
    }

    /// `(a, b) = value` — the members of a tuple fact go to the variables they bind; an
    /// abstract value itself cannot be taken apart.
    pub(crate) fn bind_unpacked(&mut self, value: &AliasFact, vars: &[u16]) {
        if is_plainish(value) || !self.abstract_on() {
            return;
        }
        if let AliasFact::Tuple(ms) = value {
            for (v, m) in vars.iter().zip(ms) {
                if !m.is_plain() {
                    self.abstract_vars.insert((self.context, *v), m.clone());
                }
            }
            return;
        }
        let alias = first_alias(value).unwrap_or(0);
        self.refuse_reveal(alias, "destructuring it reads its representation");
    }

    /// A value a branch, a block or a match arm yields, beside its siblings: one fact for all
    /// of them is the construct's; a plain value beside an abstract one builds it.  The answer
    /// waits in [`Self::produced`] for the operand that reads the construct.
    pub(crate) fn settle_join(&mut self, arms: &[AliasFact]) {
        if !self.abstract_on() {
            return;
        }
        let Some(first) = arms.iter().find(|f| **f != AliasFact::Empty).cloned() else {
            self.produced = Some(if arms.is_empty() {
                AliasFact::Plain
            } else {
                AliasFact::Empty
            });
            return;
        };
        for a in arms {
            if !fits(&first, a) {
                let other = a.clone();
                self.check_handover(&first, &other, "another branch of this value");
                self.produced = Some(AliasFact::Plain);
                return;
            }
        }
        self.produced = Some(first);
    }

    /// Element `i` of a tuple pattern over the subject of the enclosing `match`
    /// ([`Self::match_subject`]): a binder takes the member's fact; a literal or a variant
    /// compared against an abstract member reads it.
    pub(crate) fn tuple_pattern_element(&mut self, i: usize, binder: Option<u16>) {
        if !self.abstract_on() {
            return;
        }
        let AliasFact::Tuple(ms) = &self.match_subject else {
            return;
        };
        let Some(m) = ms.get(i).cloned() else {
            return;
        };
        match binder {
            Some(v) if !m.is_plain() => {
                self.abstract_vars.insert((self.context, v), m);
            }
            Some(_) => {}
            None => self.check_subject(&m, "matching a pattern against it"),
        }
    }

    /// The value a lambda's body yields, recorded instead of checked: a lambda declares no
    /// result a caller reads as such, and the generic it is handed to binds its type
    /// variable to it ([`Self::lambda_result`]).
    pub(crate) fn is_lambda_context(&self) -> bool {
        self.context != u32::MAX && self.data.def(self.context).name.starts_with("n___lambda")
    }

    /// Key `nr` of a keyed lookup (`h[k]`), against the key field it is compared with: a
    /// key field declared `Handle` takes a `Handle`, a plain one a plain value.
    pub(crate) fn check_key_fact(&mut self, nr: usize, got: &AliasFact) {
        if !self.abstract_on() {
            return;
        }
        let want = self.pending_key_facts.get(nr).cloned().unwrap_or_default();
        self.check_handover(&want, got, "this key");
    }

    /// The value an `if` tests or a `match` inspects: an abstract one is read by the test.
    pub(crate) fn check_subject(&mut self, fact: &AliasFact, what: &str) {
        if is_plainish(fact) || !self.abstract_on() {
            return;
        }
        let alias = first_alias(fact).unwrap_or(0);
        self.refuse_reveal(alias, &format!("{what} reads its representation"));
    }

    /// A vector literal whose elements' facts are `elements`: one fact throughout makes a
    /// vector of it; none makes [`AliasFact::Empty`]; a plain element beside an abstract one
    /// builds it.
    pub(crate) fn settle_vector_literal(&mut self, elements: &[AliasFact]) {
        if !self.abstract_on() {
            return;
        }
        let Some(first) = elements.first().cloned() else {
            self.produced = Some(AliasFact::Empty);
            return;
        };
        for e in &elements[1..] {
            if !fits(&first, e) {
                let other = e.clone();
                self.check_handover(&first, &other, "an element of this vector");
            }
        }
        self.produced = Some(AliasFact::vector(first));
    }

    /// A tuple literal `(a, b)`: its members' facts, by position.
    pub(crate) fn settle_tuple_literal(&mut self, members: Vec<AliasFact>) {
        if self.abstract_on() {
            self.produced = Some(AliasFact::tuple(members));
        }
    }

    /// `for x in <iterable>`: the elements of a vector of a fact carry it; iterating an
    /// abstract value itself reads it.
    pub(crate) fn bind_loop_variable(&mut self, iterable: &AliasFact, var: u16) {
        if is_plainish(iterable) || !self.abstract_on() {
            return;
        }
        if let AliasFact::Vector(inner) = iterable {
            if var != u16::MAX {
                self.abstract_vars
                    .insert((self.context, var), (**inner).clone());
            }
        } else {
            let alias = first_alias(iterable).unwrap_or(0);
            self.refuse_reveal(alias, "iterating it reads its representation");
        }
    }

    /// After a lambda expression: it yields a function whose body's value has the recorded
    /// fact ([`AliasFact::Lambda`]).
    pub(crate) fn settle_lambda(&mut self) {
        if self.abstract_on() {
            let r = std::mem::take(&mut self.lambda_result);
            self.produced = Some(AliasFact::Lambda(Box::new(r)));
        }
    }

    /// A lambda's parameters as its parse recorded them (`pending_param_facts`), stored on
    /// the lambda's definition the way `parse_function` stores a function's.
    pub(crate) fn store_lambda_param_facts(&mut self) {
        // drained, not taken: the buffer is reused rather than allocated per lambda
        if self.context != u32::MAX {
            let attrs = &mut self.data.definitions[self.context as usize].attributes;
            for (a, fact) in attrs.iter_mut().zip(self.pending_param_facts.drain(..)) {
                a.fact = fact;
            }
        }
        self.pending_param_facts.clear();
    }

    /// The value a `return` (`how`) or the body's tail hands back, against the function's
    /// declared result.
    pub(crate) fn check_returned(&mut self, how: &str) {
        let got = std::mem::take(&mut self.operand_fact);
        if !self.abstract_on() || self.context == u32::MAX {
            return;
        }
        let want = self.result_fact_of(self.context);
        let name = self.data.def(self.context).name.clone();
        let fname = Self::callable_name(&name).to_string();
        self.check_handover(&want, &got, &format!("the {how} of `{fname}`"));
    }

    /// A function body's value: its block's last operator, when that is a value and not a
    /// statement (a `return` was checked where it was written).
    pub(crate) fn check_result(&mut self, body: &Value) {
        if self.abstract_on() && self.is_lambda_context() {
            self.lambda_result = std::mem::take(&mut self.operand_fact);
            if self.result_fact_of(self.context).is_plain() {
                return;
            }
            self.operand_fact = self.lambda_result.clone();
        }
        if !self.abstract_on()
            || self.context == u32::MAX
            || matches!(self.data.def(self.context).returned.base(), Type::Void)
        {
            self.operand_fact = AliasFact::Plain;
            return;
        }
        let tail = match body.unspan() {
            Value::Block(bl) => bl.operators.last().map(Value::unspan),
            other => Some(other),
        };
        if matches!(tail, None | Some(Value::Return(_) | Value::Set(..))) {
            self.operand_fact = AliasFact::Plain;
            return;
        }
        self.check_returned("result");
    }

    /// The name a program calls a definition by: a function's without its `n_`, a method's
    /// without its `t_<N><Type>_` receiver prefix, a generic's instance by its template's.
    pub(crate) fn callable_name(name: &str) -> &str {
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
        if alias == 0 || alias >= self.data.definitions() {
            return;
        }
        if trace {
            self.print_census_site("abstract", alias, usize::MAX);
        }
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

    /// The parameters of the function or lambda whose body is about to be parsed, seeded with
    /// what their declarations name — and, for a lambda handed to a generic, what the
    /// generic's type variable was bound to ([`Self::pending_lambda_facts`]).
    pub(crate) fn seed_abstract_params(&mut self) {
        self.operand_fact = AliasFact::Plain;
        let inferred = std::mem::take(&mut self.pending_lambda_facts);
        if !self.abstract_on() || self.context == u32::MAX {
            return;
        }
        for a_nr in 0..self.data.attributes(self.context) {
            let mut fact = self.attr_fact_of(self.context, a_nr);
            if fact.is_plain()
                && let Some(f) = inferred.get(a_nr)
            {
                fact = f.clone();
            }
            if !fact.is_plain() {
                self.abstract_vars.insert((self.context, a_nr as u16), fact);
            }
        }
    }
}
