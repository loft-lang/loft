// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Interface bounds (`(G-Sat)`): whether a concrete type satisfies every bound of a
//! template's variables — the members it must carry, the operators that meet them, the
//! associated types the bounds bind — and every reason it does not, as the message says it.

use super::{Data, DefType, Level, Parser, Type};

impl Parser {
    /// @PLN125 A2c — what each associated type of `g_nr`'s bounds binds to, for the
    /// implementor `concrete_nr`.  One `(placeholder, companion)` pair per associated type
    /// that can be inferred; the empty vector when the bounds declare none.
    ///
    /// The inference is the implementor's own signature read back through the interface's:
    /// where the interface writes `Self.Rows`, the implementor writes a concrete type, and
    /// THAT is the companion.  Nothing else can say it — satisfaction is structural, so
    /// there is no `impl` block to name the companion in.
    ///
    /// Both the return and the parameters are read, and every position must AGREE.  An
    /// implementor whose `open` yields one type while its `feed` takes another does not
    /// have one companion, and binding to whichever the walk reached first would make the
    /// answer depend on declaration order.  Disagreement is a compile error naming both
    /// types, because the alternative is a monomorph typed against a companion the author
    /// never chose.
    pub(super) fn associated_bindings(
        &mut self,
        g_nr: u32,
        var_bindings: &[(u32, Type)],
    ) -> Vec<(u32, Type)> {
        let (out, clashes) = self.infer_associated(g_nr, var_bindings);
        for text in clashes {
            let msg = crate::diagnostics::diagnostic_format(Level::Error, format_args!("{text}"));
            let peek_pos = self.lexer.peek().position;
            self.lexer.pos_diagnostic(Level::Error, &peek_pos, &msg);
        }
        out
    }

    /// [`Self::associated_bindings`] without its diagnostic: the pairs it infers, and one
    /// message per associated type the implementor names two different companions for.
    /// Asked on its own by [`Self::satisfies`], which must answer without reporting.
    pub(super) fn infer_associated(
        &self,
        g_nr: u32,
        var_bindings: &[(u32, Type)],
    ) -> (Vec<(u32, Type)>, Vec<String>) {
        // Per VARIABLE (@PLN165 C3, F13): each variable's bounds name the associated types
        // its binding supplies — `<S: Source, K: Sink>` binds `S.Rows` from `S`'s type and
        // `K.Item` from `K`'s.  Reading only the first left `K.Item` unbound, and a call
        // through its bound read garbage.
        let mut out: Vec<(u32, Type)> = Vec::new();
        let mut clashes: Vec<String> = Vec::new();
        for (i, (holder, bound)) in var_bindings.iter().enumerate() {
            let concrete_nr = self.data.type_def_nr(bound);
            let (o, c) =
                self.infer_associated_for(self.var_bounds(g_nr, *holder, i == 0), concrete_nr);
            out.extend(o);
            clashes.extend(c);
        }
        (out, clashes)
    }

    /// [`Self::infer_associated`] for one variable: the companions its `bounds` declare, read
    /// off the implementor `concrete_nr`'s own signatures.
    pub(super) fn infer_associated_for(
        &self,
        bounds: Vec<u32>,
        concrete_nr: u32,
    ) -> (Vec<(u32, Type)>, Vec<String>) {
        if bounds.is_empty() || concrete_nr == u32::MAX {
            return (Vec::new(), Vec::new());
        }
        let concrete_type = self.data.def(concrete_nr).returned().clone();
        let mut out: Vec<(u32, Type)> = Vec::new();
        let mut clashes: Vec<String> = Vec::new();
        for iface_nr in bounds {
            let children: Vec<u32> = self.data.children_of(iface_nr).collect();
            // The placeholders this interface declares — its non-method children.
            for &ph in &children {
                if !matches!(self.data.def_type(ph), DefType::Struct) {
                    continue;
                }
                let mut found: Option<(Type, String)> = None;
                let mut clash: Option<(String, String)> = None;
                for &child_nr in &children {
                    let Some(method) = Self::interface_method_name(&self.data, child_nr) else {
                        continue;
                    };
                    let imp = self.data.find_fn(u16::MAX, &method, &concrete_type);
                    if imp == u32::MAX {
                        continue;
                    }
                    // Every position where the interface named the placeholder, paired with
                    // what the implementor put there.  Parameters align by index: an
                    // interface method carries no hidden parameters, and the hidden ones a
                    // concrete method carries are appended after the declared ones.
                    //
                    // **`without_deps`, because a companion is a TYPE and not a place.**
                    // The implementor's `returned` carries a dep list indexed in the
                    // IMPLEMENTOR's frame — `Reference(Rows, deps=[1])` where 1 is that
                    // function's own return buffer — and it is non-empty exactly when the
                    // producer's return comes from a nested call rather than an inline
                    // construction.  Recorded verbatim, that list is then substituted into
                    // every use of `Self.Rows` inside a monomorph, where the same indices
                    // name unrelated CALLER locals: the caller's binding turns into a view
                    // of whatever variable 1 happens to be, and the free lands on a stack
                    // record (the `#306` guard, then a fault).  loft#666 is the same defect
                    // through a different door, which is why the answer already had a name.
                    let mut candidates: Vec<Type> = Vec::new();
                    if self.data.def(child_nr).returned().contains_def(ph) {
                        candidates.push(self.data.def(imp).returned().without_deps());
                    }
                    for a in 0..self.data.def(child_nr).attributes().len() {
                        if self.data.attr_type(child_nr, a).contains_def(ph)
                            && a < self.data.def(imp).attributes().len()
                        {
                            candidates.push(self.data.attr_type(imp, a).without_deps());
                        }
                    }
                    for cand in candidates {
                        let Some(cand_nr) = Self::named_def(&cand) else {
                            continue;
                        };
                        match &found {
                            None => found = Some((cand, method.clone())),
                            Some((seen, seen_method)) => {
                                if Self::named_def(seen) != Some(cand_nr) {
                                    clash = Some((seen_method.clone(), method.clone()));
                                }
                            }
                        }
                    }
                }
                if let Some((first, second)) = clash {
                    let ph_name = self.data.def(ph).name().to_string();
                    let concrete_name = self.data.def(concrete_nr).name().to_string();
                    clashes.push(format!(
                        "'{concrete_name}' does not agree with itself about '{ph_name}': \
                         '{first}' and '{second}' name different types for it",
                    ));
                    continue;
                }
                if let Some((companion, _)) = found {
                    out.push((ph, companion));
                }
            }
        }
        (out, clashes)
    }

    /// The definition a type NAMES, for a struct, a vector element or a struct-enum —
    /// `None` for a scalar, which has no definition to compare.
    pub(super) fn named_def(tp: &Type) -> Option<u32> {
        match tp.base() {
            Type::Reference(d, _) | Type::Enum(d, _, _) => Some(*d),
            Type::Vector(inner, _) => Self::named_def(inner),
            _ => None,
        }
    }

    /// Every way `concrete_nr` fails to satisfy `iface_nr` — empty when it satisfies it.
    ///
    /// One home for the structural judgement, because it is now asked TWICE: of the
    /// implementor against the generic's bounds, and of the COMPANION against the bound
    /// declared on an associated type (@PLN125 A2c). Answering with the reasons rather
    /// than emitting them lets each caller say whose promise was broken; the words after
    /// the colon are the same either way.
    /// The parameter count a SIGNATURE declares — hidden parameters excluded.
    ///
    /// The raw count is not the signature's: a struct-returning function carries a hidden
    /// return buffer and a bound stub carries one too, so comparing raw counts makes a
    /// two-operand operator look like a three-parameter function (loft#1299).
    pub(super) fn visible_arity(data: &Data, d_nr: u32) -> usize {
        if d_nr == u32::MAX || d_nr as usize >= data.definitions.len() {
            return usize::MAX;
        }
        data.definitions[d_nr as usize]
            .attributes()
            .iter()
            .filter(|a| !a.hidden)
            .count()
    }

    /// Does content `==` cover the type `concrete_nr` names — a struct (a `value struct` and a
    /// tuple's `__tuple<…>` included), a struct-enum or one of its variants, a vector, a keyed
    /// collection?
    /// `(G-Sat-Eq)` admits such a type for an `OpEq` bound it declares no operator for.
    pub(super) fn content_comparable(&self, concrete_nr: u32) -> bool {
        let name = self.data.def(concrete_nr).name();
        match self.data.def_type(concrete_nr) {
            DefType::Struct => true,
            DefType::EnumValue => true,
            // A struct-enum compares by content and a plain enum by its variant — both are the
            // concrete `a == b` the monomorph lowers to, so `(G-Sat-Eq)` holds for every enum;
            // asked of struct-enums alone it refused `<T: Equatable>` at a plain one (loft#2057).
            DefType::Enum => true,
            _ => matches!(
                name,
                "vector" | "hash" | "sorted" | "index" | "spatial" | "trie"
            ),
        }
    }

    /// Is `child_nr` (named `member`) the symbolic alias of an `operator` member of the same
    /// interface — `OpLt` beside `operator compare`, `OpMin` at one operand beside `operator
    /// negate`?  Read from the interface's own members, so it holds for an interface loaded
    /// from the startup cache as for one parsed now.
    pub(super) fn is_operator_alias(&self, iface_nr: u32, child_nr: u32, member: &str) -> bool {
        let arity = Self::visible_arity(&self.data, child_nr);
        let Some(form) = Data::operator_form_of_member(member, arity) else {
            return false;
        };
        self.data.children_of(iface_nr).any(|c| {
            c != child_nr
                && self.data.def(c).operator_form()
                && Self::visible_arity(&self.data, c) == arity
                && Self::interface_method_name(&self.data, c).as_deref() == Some(form)
        })
    }

    /// The source symbol of an interface's operator member, for a message: `OpMin` at arity 1
    /// is the unary `-`.
    pub(super) fn member_symbol(member: &str, arity: usize) -> String {
        let sym = match (member, arity) {
            ("OpMin", 1) => "unary -",
            ("OpMin", _) => "-",
            ("OpMul", _) => "*",
            ("OpDiv", _) => "/",
            ("OpRem", _) => "%",
            ("OpAdd", _) => "+",
            ("OpLt", _) => "<",
            _ => return format!("`{member}`"),
        };
        format!("`{sym}`")
    }

    /// Why `concrete` does not meet the interface member `child_nr` (`method_suffix`) when it
    /// has no method of the name.  An operator member is met by an `operator` method (C132):
    /// the reason names the one to write, the type's own `operator` at other parameter types
    /// or as a template, or that no program type can meet the member yet.
    pub(super) fn missing_member(
        &self,
        method_suffix: &str,
        child_nr: u32,
        concrete: &Type,
    ) -> String {
        let arity = Self::visible_arity(&self.data, child_nr);
        let t = concrete.source_name(&self.data);
        // A member written `operator` IS its form; a symbolic one names it.
        let form = if self.data.def(child_nr).operator_form() {
            [
                "compare",
                "plus",
                "minus",
                "times",
                "divided_by",
                "remainder",
                "negate",
            ]
            .into_iter()
            .find(|f| *f == method_suffix)
        } else {
            Data::operator_form_of_member(method_suffix, arity)
        };
        let present = form
            .map(|f| self.data.find_op_method(u16::MAX, f, concrete))
            .filter(|d| *d != u32::MAX && self.data.def(*d).operator_form());
        if let (Some(f), Some(d)) = (form, present) {
            if self.data.def_type(d) == DefType::Generic {
                return format!(
                    "'{f}' is a template, and a bound takes a concrete '{f}' of its \
                     signature; declare one for '{t}'"
                );
            }
            if let Some(other) = self.data.visible_params(d).get(1) {
                return format!(
                    "'{f}' takes '{}' where the interface declares '{t}'",
                    other.source_name(&self.data)
                );
            }
        }
        match form {
            // The member's own signature, `Self` replaced: what the type has to define.
            Some(form) => {
                let params = self.bound_params_at(child_nr, concrete);
                let names: Vec<String> = (0..self.data.def(child_nr).attributes().len())
                    .filter(|a| !self.data.def(child_nr).attributes()[*a].hidden)
                    .map(|a| self.data.attr_name(child_nr, a))
                    .collect();
                let list: Vec<String> = names
                    .iter()
                    .zip(&params)
                    .map(|(n, p)| format!("{n}: {}", p.source_name(&self.data)))
                    .collect();
                let self_nr = self.data.def_nr("Self");
                let ret = Self::substitute_type(
                    self.data.def(child_nr).returned().clone(),
                    self_nr,
                    concrete,
                );
                let ret = if form == "compare" {
                    "Ordering".to_string()
                } else {
                    ret.source_name(&self.data)
                };
                format!("missing `operator {form}({}) -> {ret}`", list.join(", "))
            }
            None if method_suffix.starts_with("Op") => format!(
                "it needs {}, which only the built-in types define",
                Self::member_symbol(method_suffix, arity)
            ),
            None => format!("missing {method_suffix}"),
        }
    }

    /// `@FR-Op-Bound` — does `concrete` meet the operator member `child_nr` with an `operator`
    /// method?  Asked at the member's own signature, `Self` replaced: `op <` is met by
    /// `operator compare`, `op +` by `plus`, a two-operand `op -` by `minus`, and `op * (self:
    /// Self, k: float)` by `operator times(self: C, k: float)`; the monomorph's operator then
    /// calls it (`substitute_type_in_value`).
    pub(super) fn operator_meets_member(
        &self,
        method_suffix: &str,
        child_nr: u32,
        concrete: &Type,
    ) -> bool {
        let arity = Self::visible_arity(&self.data, child_nr);
        let form = if self.data.def(child_nr).operator_form() {
            Data::operator_form_of_member(method_suffix, arity).or(match method_suffix {
                "compare" => Some("compare"),
                "plus" => Some("plus"),
                "minus" => Some("minus"),
                "times" => Some("times"),
                "divided_by" => Some("divided_by"),
                "remainder" => Some("remainder"),
                "negate" => Some("negate"),
                _ => None,
            })
        } else {
            Data::operator_form_of_member(method_suffix, arity)
        };
        let Some(form) = form else {
            return false;
        };
        let params = self.bound_params_at(child_nr, concrete);
        let self_nr = self.data.def_nr("Self");
        let result = Self::substitute_type(
            self.data.def(child_nr).returned().clone(),
            self_nr,
            concrete,
        );
        self.data
            .operator_member_with(form, &params, Some(&result))
            .is_some()
    }

    pub(crate) fn satisfaction_failures(&self, iface_nr: u32, concrete_nr: u32) -> Vec<String> {
        let concrete_name = self.data.def(concrete_nr).name().to_string();
        let concrete_type = self.data.def(concrete_nr).returned().clone();
        // A struct-enum VARIANT's `returned` is its parent ENUM, so a method declared on the
        // variant itself (`fn tag(self: Cat)`) was never seen: `greet<T: Named>(Cat {…})` was
        // "'Cat' does not satisfy interface 'Named': missing tag" while `c.tag()` compiled —
        // `(G-Sat)` says the methods VISIBLE for the type decide.  The variant's own type is
        // asked first; the enum's methods, visible for every variant, stay the fallback.
        let variant_type = (self.data.def_type(concrete_nr) == DefType::EnumValue)
            .then(|| Type::Reference(concrete_nr, crate::data::Deps::none()));
        let mut out = Vec::new();
        for child_nr in self.data.children_of(iface_nr).collect::<Vec<u32>>() {
            // Only the METHODS are structural satisfaction; a companion type is matched
            // against the implementor's own by `associated_bindings`, not by looking for a
            // method of that name.  Without this skip, `type Rows` would demand a `rows()`
            // method of every implementor and no type could satisfy the interface at all.
            let Some(method_suffix) = Self::interface_method_name(&self.data, child_nr) else {
                continue;
            };
            // The symbolic alias an `operator` member brings (`OpLt` beside `compare`) is the
            // generic body's handle, not a second requirement: the member decides.
            if self.is_operator_alias(iface_nr, child_nr, &method_suffix) {
                continue;
            }
            // `@FR-Op-Bound` — an operator member met by the type's `operator` method.
            if self.operator_meets_member(&method_suffix, child_nr, &concrete_type) {
                continue;
            }
            // I9-prim: use find_fn which checks both the method-style convention
            // (t_7integer_OpLt) and the add_op convention (OpLtInt via possible map).
            let mut found = variant_type
                .as_ref()
                .map(|v| self.data.find_fn(u16::MAX, &method_suffix, v))
                .filter(|d| *d != u32::MAX)
                .unwrap_or_else(|| self.data.find_fn(u16::MAX, &method_suffix, &concrete_type));
            // @PLN25 E2 — a synth `__nullable<S>` delegates method/interface
            // resolution to its underlying `S` (a method call on a nullable
            // element unwraps through `Some` to call `S`'s method), so satisfy
            // the bound against `S`'s methods when the wrapper itself lacks them.
            // Gate-off-inert (no `__nullable<` type exists).
            if found == u32::MAX
                && let Some(inner) = concrete_name
                    .strip_prefix("__nullable<")
                    .and_then(|r| r.strip_suffix('>'))
            {
                let s_nr = self.data.def_nr(inner);
                if s_nr != u32::MAX {
                    let s_type = self.data.def(s_nr).returned().clone();
                    found = self.data.find_fn(u16::MAX, &method_suffix, &s_type);
                }
            }
            // `(G-Sat)` satisfies a bound against the SIGNATURE `[Self ↦ C](p̄ -> R)`, and
            // `find_fn` takes a name and a receiver and no arity.  While no interface could
            // declare one name twice that gap was invisible; now that `Subtractable` asks for a
            // two-operand `OpMin`, a type providing only the UNARY one answered the name and
            // satisfied the bound, and the monomorph then called it with one operand too many
            // and dropped the second — `diff(a, b)` computed `-a`, on both backends, with no
            // diagnostic.  That is loft#1274's defect at the satisfaction site rather than the
            // use site (loft#1275).
            //
            // Compared on the VISIBLE count on both sides: a struct-returning function carries
            // a hidden return buffer that an interface declaration does not, so the raw counts
            // disagree by design.  The re-ask goes through `possible_with_signature` — the same
            // resolver `re_resolve_call` uses for the same question — so satisfaction and
            // monomorphisation cannot disagree about which definition a signature names.
            // `@FR-Op-Std`, C132 — a program's `fn Op…` meets no operator member: only the
            // stdlib's definitions, or the type's `operator` method (`@FR-Op-Bound`, above).
            if found != u32::MAX
                && method_suffix.starts_with("Op")
                && !self.data.backs_operator(found)
            {
                found = u32::MAX;
            }
            let want = Self::visible_arity(&self.data, child_nr);
            if found != u32::MAX && Self::visible_arity(&self.data, found) != want {
                found = self
                    .data
                    .possible_with_signature(&method_suffix, want, &concrete_type)
                    .unwrap_or(u32::MAX);
            }
            // `@FR-G-Sat` — the signature is `[Self ↦ C](p̄ -> R)`, and `p̄` is a parameter LIST,
            // not a count.  A member of the right arity whose parameters are other types does
            // not take the call the bound promises: `OpMin(self: W, o: integer)` satisfied
            // `Subtractable`, and the monomorph handed the `integer` parameter a record —
            // `-3` on the interpreter, a SIGSEGV or a panic for `<` over `text` and `scale`
            // over `text`, rustc refusing the native source (loft#1818).  A member of the
            // name's overload set that does take it is the one the monomorph binds
            // (`re_resolve_call` asks `Data::overload_with_params`, the same question).
            let mut misfit = None;
            if found != u32::MAX {
                let params = self.bound_params_at(child_nr, &concrete_type);
                // `@FR-G-Sat` asks for a CONCRETE function of the signature.  A template member
                // (`fn OpAdd<U>(self: W, o: U)`) is not one: the monomorph resolves the bound's
                // call to a definition and has no instantiation step, so the template itself
                // reached the bytecode — an internal compiler error on both backends
                // (loft#1826).  A concrete member of the set that takes the list still
                // satisfies; otherwise the refusal names the template.
                if self.data.def_type(found) == DefType::Generic {
                    if let Some(member) = self.data.overload_with_params(&method_suffix, &params) {
                        found = member;
                    } else {
                        misfit = Some(format!(
                            "'{method_suffix}' is a template, and a bound takes a concrete \
                             '{method_suffix}' of its signature; declare one for '{}'",
                            concrete_type.source_name(&self.data)
                        ));
                        // Refused outright, `==` included: the type HAS an `OpEq`, which the
                        // concrete `a == b` instantiates, so a content comparison in the
                        // generic would answer differently from the concrete site.
                        out.extend(misfit.take());
                        continue;
                    }
                } else if !self.data.params_fit(found, &params) {
                    if let Some(member) = self.data.overload_with_params(&method_suffix, &params) {
                        found = member;
                    } else {
                        misfit = self.param_misfit(found, &params, &method_suffix);
                        found = u32::MAX;
                    }
                }
            }
            // `(G-Sat-Eq)`, @C91 — every type satisfies `==`: one with no `OpEq` of its own is
            // compared by content, which the monomorph lowers (`content_eq_pending`).  An
            // `OpEq` that does not take `(Self, Self)` is not its own for this purpose: the
            // concrete `a == b` compares such a type by content too.
            if found == u32::MAX
                && method_suffix == "OpEq"
                && want == 2
                && self.content_comparable(concrete_nr)
            {
                continue;
            }
            if let Some(msg) = misfit {
                out.push(msg);
            } else if found == u32::MAX {
                out.push(self.missing_member(&method_suffix, child_nr, &concrete_type));
            } else if self.data.def(child_nr).operator_form()
                && !self.data.def(found).operator_form()
            {
                // `@FR-Op-Iface` — a member written `operator` is met only by an `operator`
                // definition; a plain `fn` of the name is an ordinary method.
                out.push(format!(
                    "`{method_suffix}` is a plain `fn`, and the interface asks for `operator \
                     {method_suffix}`"
                ));
            } else if let Some(msg) =
                self.return_type_mismatch(child_nr, found, concrete_nr, &method_suffix)
            {
                out.push(msg);
            }
        }
        out
    }

    /// Check that the concrete type (identified by `concrete_nr`) implements every
    /// interface in `g_nr`'s bounds, and that each companion it binds to an associated type
    /// satisfies that type's declared bound.  Returns `true` if satisfied (or no bounds),
    /// `false` and a diagnostic per failure otherwise.
    ///
    /// Enforces @FR-G-Sat: satisfaction is STRUCTURAL and judged at the point of use — a
    /// type satisfies an interface exactly when the required methods are VISIBLE there.
    /// No `impl` declaration exists to consult, so having the methods IS satisfying, and
    /// [`Self::satisfaction_messages`] is the only thing that decides it.
    ///
    /// `assoc` is what [`Self::try_generic_instantiation`] already inferred for this
    /// monomorph — passed in rather than recomputed, so a companion the implementor cannot
    /// agree on is reported once, not once per place that asks.
    pub(super) fn check_satisfaction(
        &mut self,
        g_nr: u32,
        var_bindings: &[(u32, Type)],
        assoc: &[(u32, Type)],
    ) -> bool {
        let messages = self.satisfaction_messages(g_nr, var_bindings, assoc);
        for message in &messages {
            let msg =
                crate::diagnostics::diagnostic_format(Level::Error, format_args!("{message}"));
            let peek_pos = self.lexer.peek().position;
            self.lexer.pos_diagnostic(Level::Error, &peek_pos, &msg);
        }
        messages.is_empty()
    }

    /// Does the type `concrete_nr` satisfy every bound of the template `g_nr` — the question
    /// [`Self::check_satisfaction`] reports on, answered without reporting.  Selection asks it
    /// of each template member of an overload set (`D-Rank`: a template takes a call where its
    /// variables bind AND its bounds hold), and a template whose bounds fail is simply not a
    /// candidate there; the diagnostic belongs to the instantiation that goes ahead anyway.
    pub(crate) fn satisfies(&self, g_nr: u32, var_bindings: &[(u32, Type)]) -> bool {
        let (assoc, clashes) = self.infer_associated(g_nr, var_bindings);
        clashes.is_empty()
            && self
                .satisfaction_messages(g_nr, var_bindings, &assoc)
                .is_empty()
    }

    /// A type's name as a message shows it: a type-variable placeholder by the spelling its
    /// header wrote (`T`, not the `T#4` key a second variable spelled alike is minted under).
    pub(super) fn shown_type_name(&self, d_nr: u32) -> String {
        let name = self.data.def(d_nr).name();
        if self.data.is_type_var_placeholder(d_nr) {
            Data::type_var_spelling(name).to_string()
        } else {
            name.to_string()
        }
    }

    /// The bounds of the template `g_nr`'s variable `holder`: the variable's own, recorded on
    /// its placeholder (@PLN165 C2), or — for the first variable of a template whose
    /// placeholder carries none (`LOFT_NO_SEVERAL_VARS=1`, an image cached before) — the
    /// template's list, which is the first variable's.
    pub(super) fn var_bounds(&self, g_nr: u32, holder: u32, first: bool) -> Vec<u32> {
        let own = &self.data.definitions[holder as usize].bounds;
        if !own.is_empty() {
            own.clone()
        } else if first {
            self.data.definitions[g_nr as usize].bounds.clone()
        } else {
            Vec::new()
        }
    }

    /// Every reason `concrete_nr` fails `g_nr`'s bounds and its associated types' bounds, as
    /// the words a diagnostic prints — empty when it satisfies them.  The one home of the
    /// judgement [`Self::check_satisfaction`] and [`Self::satisfies`] both read.
    pub(super) fn satisfaction_messages(
        &self,
        g_nr: u32,
        var_bindings: &[(u32, Type)],
        assoc: &[(u32, Type)],
    ) -> Vec<String> {
        let mut messages: Vec<String> = Vec::new();
        // Each variable's binding against ITS OWN bounds (@PLN165 C2): `<K: Named, V: Named>`
        // judges `V`'s type too, where the template's list held `K`'s alone and a `text`
        // bound to `V` passed unchecked — its stub then resolved to nothing and answered
        // empty, silently.
        let mut first_concrete = u32::MAX;
        for (i, (holder, bound)) in var_bindings.iter().enumerate() {
            let concrete_nr = self.data.type_def_nr(bound);
            if i == 0 {
                first_concrete = concrete_nr;
            }
            if concrete_nr == u32::MAX {
                continue; // can't check without a concrete type def_nr
            }
            // A tuple is spelled as the author wrote it, not as its synthetic `__tuple<…>`.
            let concrete_name = if matches!(bound.peel_link(), Type::Tuple(_)) {
                bound.source_name(&self.data)
            } else {
                self.shown_type_name(concrete_nr)
            };
            for iface_nr in self.var_bounds(g_nr, *holder, i == 0) {
                let iface_name = self.data.def(iface_nr).name().to_string();
                for why in self.satisfaction_failures(iface_nr, concrete_nr) {
                    messages.push(format!(
                        "'{concrete_name}' does not satisfy interface '{iface_name}': {why}"
                    ));
                }
            }
        }
        if first_concrete == u32::MAX {
            return messages;
        }
        // The implementor an associated type was read off: the variable whose bounds declare
        // the interface that owns it (@PLN165 C3), the first variable's where none does.
        let implementor_of = |ph: u32| -> String {
            let iface = self.data.def(ph).parent;
            let owner = var_bindings
                .iter()
                .enumerate()
                .find(|(i, (holder, _))| self.var_bounds(g_nr, *holder, *i == 0).contains(&iface));
            let concrete = owner.map_or(first_concrete, |(_, (_, b))| self.data.type_def_nr(b));
            let concrete = if concrete == u32::MAX {
                first_concrete
            } else {
                concrete
            };
            self.shown_type_name(concrete)
        };
        // @PLN125 A2c — the bound declared on an associated type is a promise about the
        // COMPANION, and this is the only place it can be kept: `type Rows: Cursor` says a
        // generic may call `Cursor`'s methods on whatever `open` returns, and the template
        // was typed on exactly that promise.  Checked per monomorph, because the companion
        // is per implementor.
        //
        // The message names all four parties, because three of them are invisible at the
        // call site: the reader wrote `use_it(db)` and gets told about a type they never
        // mentioned, so it has to say how that type was reached.
        for (ph, companion) in assoc {
            let Some(companion_nr) = Self::named_def(companion) else {
                continue;
            };
            let ph_name = self.data.def(*ph).name().to_string();
            let companion_name = self.data.def(companion_nr).name().to_string();
            let concrete_name = implementor_of(*ph);
            for bound_nr in self.data.definitions[*ph as usize].bounds.clone() {
                let bound_name = self.data.def(bound_nr).name().to_string();
                for why in self.satisfaction_failures(bound_nr, companion_nr) {
                    messages.push(format!(
                        "'{concrete_name}' binds '{ph_name}' to '{companion_name}', which does \
                         not satisfy the declared bound '{bound_name}': {why}"
                    ));
                }
            }
        }
        messages
    }

    /// @PLN125 A2a — does the implementor's method return a DIFFERENT named type
    /// than the interface declares?  The message, or `None` when they agree.
    ///
    /// Satisfaction checked only that a method of the right name existed, so an
    /// implementor returning something else was accepted in silence. What saved
    /// most programs is not the check but a coincidence: the generic body is
    /// typed from the INTERFACE's return, so a field the interface's type lacks
    /// fails later with `Unknown field Rows.m` — a message about the wrong type,
    /// at the use site, blaming the caller. Give both structs the same field name
    /// and even that goes quiet.
    ///
    /// Deliberately narrow: it reports only when both sides name a struct or enum
    /// and the definitions DIFFER. That is the unambiguous case and it cannot
    /// false-positive on the two things that legitimately vary — dependency sets
    /// (`text["b"]` vs `text`) and integer ranges. A2c widens this to the
    /// associated-type bound, which is where the remaining shapes belong.
    ///
    /// `Self` in the interface's return substitutes to the concrete type first,
    /// so `fn mk(self: Self) -> Self` against `fn mk(self: A) -> A` agrees.
    /// The visible parameter types an interface member `iface_method` declares, at the
    /// implementor `concrete` (`[Self ↦ C]`).  A parameter typed by one of the interface's own
    /// ASSOCIATED types (`Self.X`) stands for whatever companion the implementor supplies, so
    /// it answers `Unknown` and asks nothing — as [`Self::return_type_mismatch`] leaves such a
    /// return unchecked.
    pub(super) fn bound_params_at(&self, iface_method: u32, concrete: &Type) -> Vec<Type> {
        let self_nr = self.data.def_nr("Self");
        let iface = self.data.def(iface_method).parent;
        self.data
            .def(iface_method)
            .attributes
            .iter()
            .filter(|a| !a.hidden)
            .map(|a| {
                let t = if self_nr == u32::MAX {
                    a.typedef.clone()
                } else {
                    Self::substitute_type(a.typedef.clone(), self_nr, concrete)
                };
                match t.base() {
                    Type::Reference(d, _)
                        if iface != u32::MAX
                            && self.data.def(*d).parent == iface
                            && matches!(self.data.def_type(*d), DefType::Struct) =>
                    {
                        Type::Unknown(0)
                    }
                    _ => t,
                }
            })
            .collect()
    }

    /// The `(G-Sat)` refusal for a member whose parameter list does not take `params`: the
    /// first position that does not, named as the author wrote both types.
    pub(super) fn param_misfit(&self, found: u32, params: &[Type], method: &str) -> Option<String> {
        let have = self.data.visible_params(found);
        have.iter()
            .zip(params)
            .find(|(h, w)| !self.data.param_fits(h, w))
            .map(|(h, w)| {
                format!(
                    "'{method}' takes '{}' where the interface declares '{}'",
                    h.source_name(&self.data),
                    w.source_name(&self.data)
                )
            })
    }

    /// Does `tp` name one of the interface `iface_method` belongs to's ASSOCIATED types anywhere
    /// in it (`Self.Rows`, `vector<Self.Item>`)?  Such a type stands for whatever companion the
    /// implementor supplies, which `associated_bindings` matches; satisfaction asks nothing of
    /// it.
    pub(super) fn names_associated_type(&self, iface_method: u32, tp: &Type) -> bool {
        let iface = self.data.def(iface_method).parent;
        iface != u32::MAX
            && tp.any_node(&mut |t| {
                matches!(t.base(), Type::Reference(d, _)
                    if self.data.def(*d).parent == iface
                        && matches!(self.data.def_type(*d), DefType::Struct))
            })
    }

    pub(super) fn return_type_mismatch(
        &self,
        iface_method: u32,
        concrete_method: u32,
        concrete_nr: u32,
        method: &str,
    ) -> Option<String> {
        let self_nr = self.data.def_nr("Self");
        if self_nr == u32::MAX {
            return None;
        }
        let concrete_type = self.data.def(concrete_nr).returned().clone();
        let want = Self::substitute_type(
            self.data.def(iface_method).returned().clone(),
            self_nr,
            &concrete_type,
        );
        let got = self.data.def(concrete_method).returned().clone();
        let named = |t: &Type| match t.base() {
            Type::Reference(d, _) | Type::Enum(d, _, _) => Some(*d),
            _ => None,
        };
        let (Some(w), Some(g)) = (named(&want), named(&got)) else {
            // `@FR-G-Sat` — the return is part of the signature for every type, not only for a
            // named one: a `-> float` member met `-> integer` and the generic read the float's
            // bits as an integer (loft#1927).  A return typed by the interface's own associated
            // type asks nothing, as such a parameter does (`bound_params_at`).  A member whose
            // interface member returns nothing may return a value: the generic discards it.
            if matches!(want.base(), Type::Void) || self.names_associated_type(iface_method, &want)
            {
                return None;
            }
            return (!self.data.return_fits(&got, &want)).then(|| {
                let shown = |t: &Type| {
                    if matches!(t.base(), Type::Void) {
                        "nothing".to_string()
                    } else {
                        format!("'{}'", t.source_name(&self.data))
                    }
                };
                format!(
                    "'{method}' returns {} but the interface declares {}",
                    shown(&got),
                    shown(&want)
                )
            });
        };
        if w == g {
            return None;
        }
        // @PLN125 arc A step A2b — the interface declares `Self.X`, an ASSOCIATED type.
        // Its placeholder stands for whatever companion the implementor supplies, so at
        // this step any concrete return satisfies it: there is nothing yet that says which
        // type the companion is.  A2c is what binds the placeholder per monomorph and
        // checks the declared bound; until then the alternative is worse than unchecked —
        // every implementor would be rejected for returning something other than the
        // placeholder itself, which no type can be, so `Self.X` would be unwritable in the
        // very step that introduces it.
        //
        // This widens NOTHING that A2a narrowed: it applies only where the interface's
        // declared return is one of its own associated-type placeholders, which could not
        // be spelled before A2b.
        // The placeholder and the method stub are both children of the interface, so
        // "declared by the interface this method belongs to" is a parent comparison.
        let iface = self.data.def(iface_method).parent;
        if iface != u32::MAX
            && self.data.def(w).parent == iface
            && matches!(self.data.def_type(w), DefType::Struct)
        {
            return None;
        }
        Some(format!(
            "'{method}' returns '{}' but the interface declares '{}'",
            self.data.def(g).name(),
            self.data.def(w).name(),
        ))
    }

    /// The type a call's argument at `arg` is checked against — the callee's declared
    /// parameter — with, for a GENERIC callee, every type variable the arguments parsed so far
    /// (`types`) have bound replaced by what it bound to.  A short lambda takes its parameter
    /// types from this, so `my_map(a, |x| { x * 10 })` hands `x` the element type of `a`, as
    /// the built-in `map` and a concrete twin do; a variable no earlier argument binds stays
    /// as declared, and the lambda says it cannot infer it.
    pub(crate) fn callee_param_hint(&mut self, d_nr: u32, arg: usize, types: &[Type]) -> Type {
        let declared = self.data.attr_type(d_nr, arg);
        if self.data.def_type(d_nr) != DefType::Generic {
            return declared;
        }
        let Some(bindings) = Self::bind_template(&self.data, d_nr, types) else {
            return declared;
        };
        let known: Vec<(u32, Type)> = bindings
            .into_iter()
            .filter(|(_, b)| !b.is_unknown())
            .collect();
        // Closed: `f: fn(Grid<T>) -> integer` at `T = integer` hands the lambda the call's
        // `Grid<integer>`, where a plain substitution left the open `Grid<T>` (@PLN165 D11).
        let hint = self.close_open(&declared, &known);
        // A callback's RETURN naming a variable no argument has bound yet — `U` in
        // `map_grid<T, U>(g: Grid<T>, f: fn(T) -> U)` — is the lambda's to declare: the hint
        // leaves it open, a short lambda's body gives it (loft#945, as for `map`'s callback),
        // and the call binds `U` from the lambda's type.
        if let Type::Function(params, ret, deps, consts) = hint.base() {
            let unbound: Vec<u32> = Self::template_vars(&self.data, d_nr)
                .into_iter()
                .filter(|v| !known.iter().any(|(h, _)| h == v))
                .collect();
            if unbound.iter().any(|v| self.data.type_mentions(ret, *v)) {
                return Type::Function(
                    params.clone(),
                    Box::new(Type::Unknown(0)),
                    deps.clone(),
                    *consts,
                );
            }
        }
        hint
    }

    /// The type variables of the template `g_nr`: the distinct placeholders its declared
    /// parameters mention, in the order they FIRST appear.  An interface's associated type
    /// (a placeholder whose parent is the interface) is not one — it is bound from the
    /// implementor, not from an argument ([`Self::associated_bindings`]).
    ///
    /// Read off the parameters rather than off the header, so a template loaded from a
    /// cached image answers the same as one parsed here.  Enforces @FR-G-Gen's
    /// `fn f<T>(x: …T…)` ellipsis: a variable may sit anywhere inside a parameter type, so
    /// the walk descends every former the keystone knows ([`Type::for_each_child`]) — a
    /// hand-rolled descent that knew only `vector<T>` once accepted `fn f<T>(x: T?, …)` at
    /// its declaration and reported it as *"Unknown function"* at every call.  The leaf is a
    /// type-var PLACEHOLDER, not any `Reference`, so a parameter that also names a concrete
    /// struct — `(P, T)` — answers with `T`.
    pub(crate) fn template_vars(data: &Data, g_nr: u32) -> Vec<u32> {
        // Through an open instance's arguments too (@PLN165 D5): `b: Box<T>` names `T`.
        let mut found: Vec<u32> = Vec::new();
        for a in data.def(g_nr).attributes().iter().filter(|a| !a.hidden) {
            data.placeholders_in(&a.typedef, &mut found);
        }
        found
            .into_iter()
            .filter(|d| {
                let parent = data.def(*d).parent;
                !((parent as usize) < data.definitions.len()
                    && data.def_type(parent) == DefType::Interface)
            })
            .collect()
    }

    /// What each of `g_nr`'s type variables binds to at a call whose argument types are
    /// `types` — [`Self::resolve_type_var`] over every (declared parameter, argument) pair,
    /// the first pair that relates deciding.  A variable no argument binds answers
    /// `Type::Unknown(0)`; `None` when the template has no variable at all.
    pub(crate) fn bind_template(
        data: &Data,
        g_nr: u32,
        types: &[Type],
    ) -> Option<Vec<(u32, Type)>> {
        let vars = Self::template_vars(data, g_nr);
        if vars.is_empty() {
            return None;
        }
        let params = data.def(g_nr).attributes();
        Some(
            vars.into_iter()
                .map(|v| {
                    let bound = params
                        .iter()
                        .zip(types)
                        .map(|(p, t)| Self::resolve_type_var(data, &p.typedef, v, t))
                        .find(|r| !r.is_unknown())
                        .unwrap_or(Type::Unknown(0));
                    (v, bound)
                })
                .collect(),
        )
    }

    /// Unify a template parameter type with a concrete argument type to extract
    /// what the type variable `tv_nr` resolves to.
    ///
    /// The binding half of @FR-G-Mono — *"a call `f(ā)` with concrete argument types C̄
    /// specialises f"* — and the read twin of the declaration check that @FR-G-Gen's
    /// `…T…` is satisfied.  Both descend `Type::for_each_child`, so a declaration the
    /// parser accepts is one an instantiation can reach.
    /// E.g. template `vector<T>` + concrete `vector<integer>` → `integer`.
    pub(super) fn resolve_type_var(
        data: &Data,
        template_tp: &Type,
        tv_nr: u32,
        concrete_tp: &Type,
    ) -> Type {
        // `Rewritten(T)` is a value-construction marker (e.g. `P { v: 99 }`
        // becoming an Insert sequence) that should not propagate into the
        // bound T — the type variable describes the data shape, not how
        // a particular argument got assembled.  Strip it before unifying.
        if let Type::Rewritten(inner) = concrete_tp {
            return Self::resolve_type_var(data, template_tp, tv_nr, inner);
        }
        // A `&` link is the same category as the `Rewritten` marker above: it records how
        // the argument is REACHED, not what it IS, and a type variable is bound to the data
        // shape.  @FR-C-Ref says a `&τ` is accepted wherever a `τ` is, so `vector<T>` must
        // unify with a `&vector<integer>` argument exactly as it does with a plain one —
        // without this, `zip_children` pairs `Vector` against `RefVar` and answers "these
        // two types do not relate", which surfaces as "Cannot resolve generic type
        // parameter from argument type" for `sum` / `min_of` / `max_of` and for EVERY
        // user-written generic over a collection.
        if let Type::RefVar(inner) = concrete_tp {
            return Self::resolve_type_var(data, template_tp, tv_nr, inner);
        }
        match template_tp {
            // Same principle as the `Rewritten` strip above: the call-site
            // argument's dep list records what THAT expression borrows in the
            // CALLER's frame — not part of the data shape.  Bound verbatim, it
            // is baked into the instantiated def's attr/return types where the
            // indices are misread as callee attr deps: the instantiation's
            // return then claims to borrow its argument, so the caller never
            // frees the fresh store the method's `__retbuf` delivered (one
            // record leaked per call).
            Type::Reference(d, _) if *d == tv_nr => match concrete_tp {
                // `null`, a call answering nothing and a value poisoned by its own report
                // (@P376) name no type: they bind nothing, as in a literal's fields (D4).  A
                // short lambda answering only `null` bound `U` to `null` and minted
                // `Grid<null>`, whose element had no layout (@PLN165 D11).
                Type::Null | Type::Void | Type::Never => Type::Unknown(0),
                // A tuple a callback returns is delivered boxed (loft#1349); the variable is
                // the TUPLE the author wrote, whichever form the argument arrives in —
                // bound to the box on one pass and the tuple on the other, it minted two
                // `Grid<(integer, text)>` (@PLN165 D11).
                Type::Reference(cd, _) if data.boxed_tuple(*cd).is_some() => {
                    data.boxed_tuple(*cd).unwrap_or(Type::Unknown(0))
                }
                Type::Reference(cd, _) => Type::Reference(*cd, crate::data::Deps::none()),
                Type::Vector(inner, _) => Type::Vector(inner.clone(), crate::data::Deps::none()),
                Type::Enum(cd, mixed, _) => Type::Enum(*cd, *mixed, crate::data::Deps::none()),
                Type::Text(_) => Type::Text(crate::data::Deps::none()),
                other => other.clone(),
            },
            // A `T?` template against an argument that is not spelled `Optional`.
            // @PLN25 gives `τ?` and `τ` one runtime layout and `Type::optional`
            // normalises the wrapper away for a scalar, so a nullable `integer`
            // argument arrives as `Integer(nullable)` rather than as
            // `Optional(Integer)` — the same shape, one spelling apart.  Peel the
            // template and unify against the base; the nullability of the binding is
            // the template's to state, not the argument's to carry.
            Type::Optional(inner) if !matches!(concrete_tp, Type::Optional(_)) => {
                Self::resolve_type_var(data, inner, tv_nr, concrete_tp.base())
            }
            // An OPEN instance (@PLN165 D5) against an instance of the same template pairs
            // their recorded arguments: `Box<T>` against `Box<integer>` binds `T` to
            // `integer`.  The arguments live in `Data`, where the keystone pairing below
            // does not look.  A generic enum's instance is an `Enum` (@PLN165 D8).
            // An open TUPLE (loft#1868) against a tuple — stack or stored — pairs its members.
            Type::Reference(o, _)
                if data.is_open_instance(*o) && data.def(*o).instance_of == u32::MAX =>
            {
                let members: Vec<Type> = match concrete_tp.base() {
                    Type::Tuple(ts) => ts.clone(),
                    Type::Reference(c, _) if data.is_tuple_def(*c) => data
                        .def(*c)
                        .attributes()
                        .iter()
                        .map(|a| a.typedef.clone())
                        .collect(),
                    _ => return Type::Unknown(0),
                };
                data.def(*o)
                    .instance_args
                    .iter()
                    .zip(&members)
                    .map(|(t, a)| Self::resolve_type_var(data, t, tv_nr, a))
                    .find(|r| !r.is_unknown())
                    .unwrap_or(Type::Unknown(0))
            }
            Type::Reference(o, _) | Type::Enum(o, _, _)
                if data.is_open_instance(*o)
                    && data.def(*o).instance_of != u32::MAX
                    && matches!(concrete_tp.base(), Type::Reference(c, _) | Type::Enum(c, _, _)
                        if data.def(*c).instance_of == data.def(*o).instance_of) =>
            {
                let (Type::Reference(c, _) | Type::Enum(c, _, _)) = concrete_tp.base() else {
                    return Type::Unknown(0);
                };
                data.def(*o)
                    .instance_args
                    .iter()
                    .zip(&data.def(*c).instance_args)
                    .map(|(t, a)| Self::resolve_type_var(data, t, tv_nr, a))
                    .find(|r| !r.is_unknown())
                    .unwrap_or(Type::Unknown(0))
            }
            // Every other former descends through the keystone pairing, so a type
            // variable under a `(T, U)`, a `fn(T) -> T`, an `iterator<T>`, a `T?` or
            // a `vector<T>` is found the same way.  `None` is "these two types do not
            // relate", which is what an unbindable argument is.
            _ => match template_tp.zip_children(concrete_tp) {
                Some(pairs) => pairs
                    .into_iter()
                    .map(|(t, c)| Self::resolve_type_var(data, t, tv_nr, c))
                    .find(|r| !r.is_unknown())
                    .unwrap_or(Type::Unknown(0)),
                None => Type::Unknown(0),
            },
        }
    }
}
