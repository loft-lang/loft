// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Generic templates and their instances (`@FR-G-Mono`): the type variables a header
//! declares, predicting a template's return for a call, and instantiating a template — or a
//! lambda written inside one — for concrete argument types, body and variable table included.

use super::{
    Argument, Data, DefType, Deps, Function, Level, OutputState, Parser, Type, Value, collections,
    diagnostic_format, v_set,
};

impl Parser {
    /// The refusal for a type variable named outside the definition whose header declares
    /// it (`D-Scope`, @FR-G-Gen-Scope) — one wording for every site that finds one.
    pub(crate) fn type_var_out_of_scope(spelled: &str) -> String {
        format!(
            "`{spelled}` is not a type here — a type variable is a type only inside the \
             definition whose header declares it; declare it on this one \
             (`fn f<{spelled}>(x: {spelled})`) or name a type"
        )
    }

    /// Resolve a NAME the source wrote, giving the enclosing generic header first refusal.
    ///
    /// `formal/interfaces.md` `(G-Gen)` — a header `fn f<T: I>` *introduces* `T`, so inside
    /// it the spelling names that header's variable and not whatever the flat namespace holds
    /// under the same letter.  Two headers may both write `T` while binding different
    /// placeholders, and the second's is minted under a name no source can spell, so this is
    /// the only route back to it.
    ///
    /// One home, because a spelling that resolves one way in a type position and another in a
    /// value position is two variables wearing one name.
    pub(crate) fn def_nr_in_scope(&self, name: &str) -> u32 {
        if let Some((_, holder)) = self.cur_type_vars.iter().find(|(n, _)| n == name) {
            return *holder;
        }
        self.data.def_nr(name)
    }

    /// Is `d_nr` one of the variables the enclosing generic header declares?
    pub(crate) fn is_header_type_var(&self, d_nr: u32) -> bool {
        self.cur_type_vars.iter().any(|(_, h)| *h == d_nr)
    }

    /// Check if a type is a generic type variable (a dummy struct used as T).
    /// Returns the type variable name if it is, None otherwise.
    pub(crate) fn generic_type_name(&self, tp: &Type) -> Option<&str> {
        if let Type::Reference(d, _) = tp {
            let d = *d as usize;
            if d < self.data.definitions.len()
                && self.data.definitions[d].def_type == DefType::Struct
                && self.data.definitions[d].attributes.is_empty()
                && self.context != u32::MAX
                && self.data.definitions[self.context as usize].def_type == DefType::Generic
            {
                // The placeholder's KEY (`T#2` for a second variable spelled alike) — a caller
                // may look the definition up by it; a message respells it with
                // `Data::type_var_spelling`.
                return Some(&self.data.definitions[d].name);
            }
        }
        None
    }

    /// The user-facing method name behind an interface child's internal one.
    ///
    /// An interface's method stubs are named `__iface_{interface}_{method}` so two
    /// interfaces can declare the same method without colliding (legacy stubs use
    /// `t_4Self_{method}`).  Three places need the name back — satisfaction, stub building,
    /// and companion inference — so it is derived here once.
    ///
    /// Answers `None` for a child that is not a method: an interface's children are its
    /// method stubs AND, since @PLN125 A2b, its associated-type placeholders.  A method
    /// stub is a `Function` and a placeholder a `Struct`, so the two are told apart by
    /// `DefType` rather than by name — a placeholder named `Rows` must never read as a
    /// demand for a `rows()` method.
    pub(crate) fn interface_method_name(data: &Data, child_nr: u32) -> Option<String> {
        if !matches!(data.def_type(child_nr), DefType::Function) {
            return None;
        }
        let name = data.def(child_nr).name();
        let self_prefix = format!("t_{}Self_", "Self".len());
        Some(if let Some(rest) = name.strip_prefix("__iface_") {
            // `__iface_<interface>#<arity>_<method>` — the arity keeps two signatures of one
            // name apart (loft#1275) and sits BEFORE the underscore, on the interface's side,
            // so what comes back here is the method name and nothing else.
            rest.split_once('_')
                .map_or_else(|| rest.to_string(), |(_, m)| m.to_string())
        } else if let Some(rest) = name.strip_prefix(&self_prefix) {
            rest.to_string()
        } else {
            name.to_string()
        })
    }

    /// Check whether the bounds that authorise `holder_nr`'s methods include an interface
    /// declaring `method`.
    ///
    /// The bounds are the HOLDER's, not the enclosing function's. For a generic's type
    /// variable the two are the same list (a bound `<T: A + B>` is recorded on the function
    /// that declares it), which is why this used to read only the function. An interface's
    /// ASSOCIATED type carries its own bounds — `type Rows: Cursor` says which methods a
    /// `Self.Rows` value has, and the enclosing generic's bounds say nothing about it.
    ///
    /// `arity` is the number of arguments the use site passes, INCLUDING the receiver, or
    /// `None` where the caller has no arity to offer (a field, a bare method name). It is not
    /// decoration: `formal/interfaces.md` (G-Sat) satisfies a bound only when a function with
    /// the interface's SIGNATURE is visible, and a name alone is not a signature. `Numeric`
    /// declares `op - (self: Self) -> Self` — UNARY negation — and a binary `a - b` in a
    /// `<T: Numeric>` body desugars to the same `OpMin`, so a name-only test said the bound
    /// covered it. The call then bound one operand too many, dropped `b`, and computed `-a`
    /// on both backends with no diagnostic (loft#1274).
    pub(crate) fn has_bound_for_method(
        &self,
        method: &str,
        holder_nr: u32,
        arity: Option<usize>,
    ) -> bool {
        let from_holder = (holder_nr != u32::MAX)
            .then(|| &self.data.definitions[holder_nr as usize].bounds)
            .filter(|b| !b.is_empty());
        let bounds = match from_holder {
            Some(b) => b,
            None if self.context != u32::MAX => {
                &self.data.definitions[self.context as usize].bounds
            }
            None => return false,
        };
        for &iface_nr in bounds {
            for child_nr in self.data.children_of(iface_nr) {
                if Self::interface_method_name(&self.data, child_nr).as_deref() != Some(method) {
                    continue;
                }
                // A name is not a signature: where the caller knows the arity, it must match.
                if arity.is_none_or(|n| self.data.attributes(child_nr) == n) {
                    return true;
                }
            }
        }
        false
    }

    /// Scan all definitions for methods named `name` (encoded as
    /// `t_<LEN><TypeName>_<name>`) and return how to NAME each receiver, in
    /// definition order and de-duplicated, plus whether every one of them was
    /// declared by the stdlib.  Powers the 6c free→method hint in `call`.
    ///
    /// loft#850 — a receiver is named `pkg::Type` unless it is the stdlib's.
    /// The bare name is what the mangled key already spells, and it stops
    /// identifying anything once two packages declare it: a hint reading
    /// "did you mean `x.go(…)` on Thing?" points at a `Thing` the author is
    /// already holding one of. The qualified name says which package's, and
    /// is also what they would have to type to reach it.
    pub(super) fn find_method_receivers(&self, name: &str) -> (Vec<String>, bool) {
        let mut all_stdlib = true;
        let mut receivers: Vec<String> = Vec::new();
        for d_nr in 0..self.data.definitions() {
            let Some(key) = Data::split_key(self.data.def(d_nr).name()) else {
                continue;
            };
            if key.kind != crate::data::KeyKind::Method || key.rest != name {
                continue;
            }
            let type_name = key.spelling;
            if type_name.is_empty() {
                continue;
            }
            // The receiver the definition itself declares outranks the name parsed out
            // of its key — that name is exactly what cannot tell two packages apart.
            let receiver_nr = self
                .data
                .def(d_nr)
                .attributes()
                .first()
                .map_or(u32::MAX, |a| self.data.type_def_nr(&a.typedef));
            let display = if receiver_nr == u32::MAX {
                type_name.to_string()
            } else if self.data.def(receiver_nr).source == crate::data::STD_SOURCE {
                self.data.def(receiver_nr).name.clone()
            } else {
                all_stdlib = false;
                self.data.qualified_type_name(receiver_nr)
            };
            if !receivers.contains(&display) {
                receivers.push(display);
            }
        }
        (receivers, all_stdlib)
    }

    /// Plan-17 phase 01 (A) — predict the substituted return type of a
    /// generic call WITHOUT instantiating the def.  Used on first pass
    /// so the receiving variable's type is set correctly for downstream
    /// inference (e.g. `t = min_max(7, 3)` followed by `t.0`).  Returns
    /// `Type::Unknown(0)` if no prediction is possible (forward decl,
    /// unresolvable type variable, etc.) — caller falls back to the
    /// existing first-pass-Unknown path.
    ///
    /// #395 — route a monomorph's concrete `Type::Tuple` return through the
    /// synthetic `__tuple<…>` struct, exactly as `parser/definitions.rs`'s
    /// `needs_tuple_rewrite` (~line 897) does for a normally-parsed tuple return.
    /// A generic TEMPLATE deliberately defers that rewrite ("`T` resolves later")
    /// and nothing re-applied it once `T` was concrete — so the copied body returns
    /// a DbRef (the template compiled `T` as a `Reference` dummy) while the declared
    /// return stayed `Tuple`, and the caller read the DbRef inline as garbage
    /// (interp) / mismatched the Rust tuple ABI (native E0308).  Both the first-pass
    /// prediction and the second-pass instantiation route through HERE so the
    /// receiving variable's type agrees across passes.  Wide (>8 B) or
    /// lifetime-bearing tuples are rewritten; an 8-byte pure-value tuple and
    /// fn-element tuples keep their existing ABI (mirrors the deferral predicate).
    /// Does `t` mention the type variable `tv_nr` anywhere?  A type variable appears
    /// as a `Reference`/`Enum` to its def; recurses `Vector`/`Optional`/`Tuple`.
    /// Does the current def's return SHAPE depend on its generic type variable
    /// (`-> T`, `-> (T, T)`, `-> vector<T>`)?  False for a non-generic context and
    /// for a generic template whose return is already CONCRETE (`-> (text, text)`).
    ///
    /// @PLN85 generic-tuple-return-fix.md — the pass-STABLE predicate that lets the
    /// return-promotion chokepoint (the `__tuple` sig rewrite + hidden `__retbuf`
    /// param in `definitions.rs`, the body rewrite + `ref_return` in `block_result`)
    /// run for a concrete-return generic template so the monomorph inherits it.
    /// Keying the guards on this (not on `is_generic_template`) collapses the 4
    /// re-assertion sites onto the one existing non-generic flow.  The type variables
    /// are the template's own ([`Self::template_vars`]) — NOT any `DefType::Generic` def
    /// (that is the FUNCTION, not a type parameter).
    pub(super) fn return_shape_depends_on_type_var(&self, t: &Type) -> bool {
        if self.context == u32::MAX || self.data.def_type(self.context) != DefType::Generic {
            return false;
        }
        let tvs = Self::template_vars(&self.data, self.context);
        // `Type::contains_def` is the keystone-backed answer to "does this type mention
        // `d_nr`?" — `any_node` over `Type::for_each_child`, so it descends every
        // child-bearing variant and a new one extends ONE match.  The hand-rolled
        // `type_mentions_tv` this replaces knew Vector / Optional / Tuple and answered
        // `_ => false` for the rest, so a type variable inside an `Iterator`, a
        // `Function`, a `RefVar` or a keyed collection read as "no type variable here".
        // `contains_def`'s own doc records being unified from two earlier copies with
        // exactly that drift; this was a third, two hundred lines from a call to it.
        // STRUCTURAL on purpose: this asks about the return's SHAPE, and an open instance
        // (`Box<T>`, @PLN165 D5) is a record whatever `T` becomes — the ABI its concrete twin
        // has (a `__retbuf`), so `type_mentions` would be the wrong question here.
        tvs.iter().any(|tv| t.contains_def(*tv))
    }

    /// Is the template's declared return the type VARIABLE itself, however it is wrapped?
    ///
    /// `-> T` spells it `Reference(tv)`; `-> T?` spells it `Optional(Reference(tv))`.  One
    /// notion, two spellings — and reading only the first answered "no, this return is a
    /// literal shape" for every `-> T?`, so the instantiation took the branch written for a
    /// signature that does not mention `T` (loft#1451).  Both call sites compute this on the
    /// PRE-substitution template return and must agree, which is why it is one function
    /// rather than the same `matches!` written twice.
    pub(super) fn return_is_the_type_var(tmpl_returned: &Type, tv_nr: u32) -> bool {
        matches!(tmpl_returned.base(), Type::Reference(d, _) if *d == tv_nr)
    }

    pub(super) fn tuple_return_rewrite(&mut self, returned: Type, from_type_var: bool) -> Type {
        // `τ?` is a SECOND SPELLING of the shape this rewrites, and matching only the first
        // is what loft#1451 was.  A `-> T?` instantiated at a tuple arrives as
        // `Optional(Tuple(…))`, which is not a `Type::Tuple`, so it fell through unrewritten:
        // the monomorph declared `(integer, integer)?` — a type the language refuses at every
        // declaration and which has no layout — while its body still handed up the DbRef the
        // template compiled `T` as.  The interpreter read that pointer's bits as member 0
        // (`34359738371` is `(1 << 35) | 3`) and `--native` would not compile the function at
        // all, one shape emitting two different wrong answers.
        //
        // Peel, decide on the tuple, re-wrap.  The nullability rides along and the boxed form
        // `Optional(Reference(__tuple<…>))` is an ordinary nullable record reference — which
        // is exactly the shape the NON-GENERIC `v[i]` spelling produces for the same element
        // type, and that spelling is right on both backends.  Peeling also puts the bare tuple
        // in front of the `wide` size test below, which is the type that test means.
        if let Type::Optional(inner) = returned {
            return Type::optional(self.tuple_return_rewrite(*inner, from_type_var));
        }
        let Type::Tuple(elems) = &returned else {
            return returned;
        };
        // When the template return type IS the bare type variable (`-> T`), the body
        // delivers T as a DbRef (the template compiled T as a `Reference` dummy), so a
        // tuple substitution must wrap it in the synthetic struct — the arm below.
        //
        // A LITERAL tuple in the signature whose SHAPE depends on `T` (`-> (T, integer)`) is
        // the case the declaration DEFERRED to instantiation
        // (`return_shape_depends_on_type_var`), and this is the instantiation — reached from
        // the pass-1 prediction and the pass-2 signature alike, so the two agree.  A
        // pure-value shape (`-> (integer, integer)`) is constructed BY VALUE in the body and
        // keeps the bare tuple ABI, which is what the old unconditional gate protected
        // (p329/p330/p240/plan17); a lifetime-bearing one is boxed exactly as
        // `boxed_tuple_return` boxes a concrete declaration, and
        // `promote_monomorph_tuple_return` rewrites the body's tuple tails to match.  Left
        // bare, the instance handed up a stack tuple whose heap member was the ARGUMENT's own
        // store, on both backends (QUALITY-history.md B7t; loft#1365's collection half,
        // `formal/tuples.md` D-tup-9).
        if !from_type_var {
            if !elems.iter().any(crate::data::has_lifetime_concern) {
                return returned;
            }
            let elems_clone = elems.clone();
            let synth = self.data.tuple_def(&mut self.lexer, &elems_clone);
            return Type::Reference(synth, crate::data::Deps::none());
        }
        let wide = u32::from(crate::variables::size(
            &returned,
            &crate::data::Context::Argument,
        )) > 8;
        let has_fn = elems.iter().any(|e| matches!(e, Type::Function(..)));
        if elems.iter().any(crate::data::has_lifetime_concern) || (wide && !has_fn) {
            let elems_clone = elems.clone();
            let synth = self.data.tuple_def(&mut self.lexer, &elems_clone);
            Type::Reference(synth, crate::data::Deps::none())
        } else {
            returned
        }
    }

    /// Reads the generic template's already-populated `returned` field, applies
    /// the type substitution, then routes a concrete tuple return through the
    /// synthetic `__tuple` struct (see [`tuple_return_rewrite`]) so the predicted
    /// type matches what `try_generic_instantiation` later produces (else the
    /// receiving variable would "change type" between passes — #395).  Registers
    /// the synthetic struct on first encounter (idempotent via `tuple_def`);
    /// otherwise side-effect-free and safe to call repeatedly.
    /// `source` is the library a qualified call names (`lib::f`), or `u16::MAX` for a
    /// bare one: a qualified call reaches the library's generic whether or not the caller
    /// imported its name (@C98).
    pub(super) fn predict_generic_return_type(
        &mut self,
        source: u16,
        name: &str,
        types: &[Type],
    ) -> Type {
        let generic_name = format!("n_{name}");
        let g_nr = self.data.source_nr(source, &generic_name);
        if g_nr == u32::MAX || self.data.def(g_nr).def_type() != DefType::Generic {
            return Type::Unknown(0);
        }
        self.predict_template_return(g_nr, name, types)
    }

    /// [`Self::predict_generic_return_type`] for the template `g_nr` itself: a free generic
    /// found by its `n_` name, or a METHOD template (`t_6vector_head`) that a method call
    /// selected (loft#1539).  `name` is the spelling the call wrote.
    pub(super) fn predict_template_return(
        &mut self,
        g_nr: u32,
        name: &str,
        types: &[Type],
    ) -> Type {
        if types.is_empty() || (self.first_param_binds(g_nr) && types[0].is_unknown()) {
            return Type::Unknown(0);
        }
        let Some(mut bindings) = Self::bind_template(&self.data, g_nr, types) else {
            return Type::Unknown(0);
        };
        if bindings.iter().any(|(_, b)| b.is_unknown()) {
            return Type::Unknown(0);
        }
        // `-> Box<T>` predicts the instance the binding names, as the monomorph declares it.
        let open = self.open_instance_bindings(&bindings, g_nr);
        bindings.extend(open);
        let tmpl_returned = self.data.definitions[g_nr as usize].returned.clone();
        let from_tv = bindings
            .iter()
            .any(|(tv, _)| Self::return_is_the_type_var(&tmpl_returned, *tv));
        let predicted =
            self.tuple_return_rewrite(Self::substitute_all(tmpl_returned, &bindings), from_tv);
        // Trace point: predicted return type for first-pass type
        // inference of generic call sites.  Used during plan-17 (A)
        // debugging.  Enable with `LOFT_TRACE=generic`.
        crate::loft_trace!(
            generic,
            "predict name={} types={:?} bindings={:?} → {:?}",
            name,
            types,
            bindings,
            predicted,
        );
        predicted
    }

    /// Try to instantiate a generic function template for the given call-site types.
    /// Returns the `def_nr` of the instantiated function, or `u32::MAX` if no generic matches.
    /// May a call target this definition?
    ///
    /// Ordinarily only a `Function` — a `Generic` is a template and a call must name a
    /// monomorph. The exception is loft#761: a template calling ANOTHER generic (itself,
    /// a mutual partner, a helper declared later). Inside a template the argument is the
    /// type VARIABLE, so no monomorph can exist yet and `try_generic_instantiation`
    /// answers the template; `instantiate_nested_generics` retargets the call at the real
    /// monomorph while building one, for whatever concrete type a caller finally names.
    ///
    /// Gated on the CALLER being a template too, so a call to a generic from ordinary
    /// code stays the error it was — there the concrete type is known and an
    /// instantiation must happen.
    pub(super) fn callable_target(&self, d_nr: u32) -> bool {
        match self.data.def_type(d_nr) {
            DefType::Function => true,
            DefType::Generic => {
                self.context != u32::MAX
                    && matches!(self.data.def_type(self.context), DefType::Generic)
            }
            _ => false,
        }
    }

    /// The stride an element read of `vector<V>` bakes while `V` is a type VARIABLE: it has no
    /// width yet, so the read carries a marker naming the variable — negative, which no real
    /// stride is — and each instance rewrites the reads whose marker names the variable it
    /// binds (`substitute_type_in_value`).  A plain `0` could not say which of two variables a
    /// read was over, and the first binding's pass took every one (@PLN165 C2).
    pub(crate) fn type_var_stride(tv_nr: u32) -> i32 {
        -1 - i32::try_from(tv_nr).unwrap_or(i32::MAX - 1)
    }

    /// The instance the open instance `open` names in the monomorph being filled — its
    /// `(open ↦ concrete)` pair in `instance_bindings` — or `open` itself when those bindings
    /// leave it open (a template instantiated at another template's variable).
    pub(crate) fn bound_instance(&self, open: u32) -> u32 {
        // A variant of an open enum instance: the variant of that name in the bound instance.
        let d = self.data.def(open);
        if d.def_type == DefType::EnumValue && self.data.is_open_instance(open) {
            let parent = self.bound_instance(d.parent);
            return if parent == d.parent {
                open
            } else {
                self.data.variant_of(parent, &d.name)
            };
        }
        match Type::Reference(open, crate::data::Deps::none())
            .substitute_all(&self.instance_bindings)
            .base()
        {
            Type::Reference(d, _) => *d,
            _ => open,
        }
    }

    /// Specialise a generic `name` for the concrete argument types `types`, returning the
    /// monomorph's def_nr (or `u32::MAX` when `name` names no generic).
    ///
    /// Enforces @FR-G-Mono: a call with concrete argument types produces ONE specialised
    /// copy per distinct instantiation, in the PARSER — so a generic never survives into
    /// the IR and every backend sees ordinary monomorphic functions.  Re-instantiating the
    /// same types reuses the existing monomorph rather than building a second.
    ///
    /// Also the gate for @FR-G-Gen's bound half: the `bindings` computed here are handed to
    /// [`Self::check_satisfaction`], so a type that does not satisfy the declared bounds is
    /// rejected at the instantiation rather than inside the specialised body.
    /// `source` as in [`Self::predict_generic_return_type`].
    pub(super) fn try_generic_instantiation(
        &mut self,
        source: u16,
        name: &str,
        types: &[Type],
    ) -> u32 {
        let generic_name = format!("n_{name}");
        let g_nr = self.data.source_nr(source, &generic_name);
        if g_nr == u32::MAX || self.data.def(g_nr).def_type() != DefType::Generic {
            return u32::MAX;
        }
        self.instantiate_template(g_nr, name, types)
    }

    /// The instance key before `D-Key` — `t_<LEN><first bound type>_<name>`, the spelling of a
    /// METHOD on that type — kept for `LOFT_NO_INSTANCE_KEY=1` and for the rename map
    /// `LOFT_TRACE_INSTANCE_KEY=1` prints (old key -> new key).
    pub(super) fn method_shaped_instance_key(
        &self,
        g_nr: u32,
        name: &str,
        concrete: &Type,
        type_nr: u32,
    ) -> String {
        if type_nr == u32::MAX {
            format!("n_{name}")
        } else {
            // @PLN25 E2 — this mangled name becomes a Rust function identifier in
            // native codegen.  A concrete element type whose NAME carries angle
            // brackets / commas (synthetic wrappers — `__nullable<Row>`,
            // `__tuple<…>`) would emit `fn t_15__nullable<Row>_count(…)`, which
            // rustc parses as a chained comparison.  Flatten those to
            // identifier-safe chars.  The replacement is 1:1 (each bracket/comma
            // → one `_`), so the LEN prefix that `original_name` /
            // `find_method_receivers` parse back stays correct.  Plain names
            // (user structs, `vector`) contain none of these and are unchanged.
            //
            // loft#1024 — key on the CONCRETE type's own name for a collection.  A
            // collection's type DEF is the bare `vector` / `hash` / … , which erases the
            // element: `T = vector<integer>` and `T = vector<text>` both mangled to
            // `t_6vector_idl`, and that name is the spelling of a METHOD ON `vector`.  So
            // the first collection instantiation captured every later call whose argument
            // was any vector at all — including the call that wanted `T = integer`, which
            // was then refused against the monomorph's `vector<vector<integer>>` parameter
            // and never reached generic resolution.  Order-dependent, and the message
            // named the innocent second call site.
            //
            // loft#1383 — an INTEGER keys on its own name for the same reason a collection
            // does: its type DEF erases the WIDTH.  Every `Integer(_)` resolves to the one
            // `integer` def, so `T = u8` and `T = u16` both mangled to `t_7integer_id` and
            // the second call was checked against the FIRST instantiation — narrow first
            // refused the wider call outright, wide first admitted the narrower one by
            // widening, so the program's meaning depended on the ORDER of two statements.
            // `Type::name` carries the range (`integer(0, 255)` vs `integer(0, 65535)`),
            // which is the identity `@FR-G-Mono` wants: ONE copy per DISTINCT instantiation.
            // Collapsing the width is right for CONVERSION (`(C-Int)` admits a widening) and
            // wrong for identity, which is why one predicate could not serve both.
            //
            // Non-collection, non-integer concretes are unchanged: a struct's, a float's and
            // a text's type-def name IS their own name.
            //
            // loft#1418 — the RANGE is not the whole of an integer's identity either, and
            // `IntegerSpec`'s own doc says so: *"Test `forced_size` to tell an alias from a
            // template; the range alone cannot."*  `i32`'s range IS the signed-32
            // template's, so `Type::name` spells both `integer` and the two collided where
            // `u8` and `u32` did not — a call at `i32` bound the monomorph and the next call
            // at plain `integer` was checked against it and refused as a narrowing, in a
            // function that may be nowhere near the one that bound it.  The declared WIDTH
            // is part of which instantiation this is, so it belongs in the key.
            let base = if Self::is_collection_type(concrete.base()) {
                concrete.name(&self.data) // schema-key — the method-name IDENTITY `t_<LEN><Type>_`; @FR-G-Mono wants the range in it
            } else if let Type::Integer(spec) = concrete.base() {
                // schema-key — same identity; loft#1418's forced WIDTH is part of which
                // instantiation this is, so the suffix stays and the two cannot collide.
                let named = concrete.name(&self.data); // schema-key — the method-name identity, as above
                match spec.forced_size {
                    Some(n) => format!("{named}s{n}"),
                    None => named,
                }
            } else {
                self.data.def(type_nr).name().to_string()
            };
            // The parentheses come from an integer's range spelling; the replacement stays
            // 1:1 so the LEN prefix `original_name` / `find_method_receivers` parse back is
            // still correct.
            let safe = base.replace(['<', '>', ',', ' ', '(', ')'], "_");
            // loft#1539 — a METHOD template and a free template of one name on DIFFERENT
            // receivers (`f<T>(self: vector<T>)` beside `f<T>(x: (T, integer))`; one receiver
            // type is refused by (F-OneBody)) bound at the same `T` would otherwise mint ONE
            // name, and whichever instantiated first would answer both calls.  The marker is a
            // spelling no source identifier can take (`__` is the compiler's).
            let safe = if self.data.def(g_nr).name().starts_with("t_") {
                format!("__self_{safe}")
            } else {
                safe
            };
            crate::data::Data::mangle_method(&safe, name)
        }
    }

    /// [`Self::try_generic_instantiation`] for the template `g_nr` itself: a free generic
    /// found by its `n_` name, or a METHOD template (`t_6vector_head`) that a method call
    /// selected (loft#1539).  `name` is the spelling the call wrote, and the monomorph is
    /// named from it exactly as a free generic's is.
    /// Does the template `g_nr`'s FIRST parameter name one of its type variables?  Until
    /// `D-Every-Var` (@PLN165 C1) it always did, and an unknown first argument meant nothing
    /// could bind yet; a variable elsewhere is bound by its own argument, and a missing one is
    /// then [`Self::bind_template`]'s to report.
    pub(super) fn first_param_binds(&self, g_nr: u32) -> bool {
        self.data
            .def(g_nr)
            .attributes
            .iter()
            .find(|a| !a.hidden)
            .is_some_and(|a| self.data.mentions_type_var(&a.typedef))
    }

    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn instantiate_template(&mut self, g_nr: u32, name: &str, types: &[Type]) -> u32 {
        if types.is_empty() || (self.first_param_binds(g_nr) && types[0].is_unknown()) {
            // First-pass argument types may be incomplete; defer the diagnostic
            // to second pass when types are stable.  Returning MAX here is the
            // same effect; it just doesn't emit a noisy first-pass error.
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot infer type for generic parameter — provide an explicit type annotation"
                );
            }
            return u32::MAX;
        }
        // loft#1538 — a template refused at its declaration has no instance; the refusal
        // already names why, and the call site answers its declared return.
        if self.refused_templates.contains(&g_nr) {
            return u32::MAX;
        }
        // What each of the template's variables binds to at this call.
        let Some(var_bindings) = Self::bind_template(&self.data, g_nr, types) else {
            return u32::MAX;
        };
        if !self.first_pass && self.binding_clash(g_nr, name, types, &var_bindings) {
            self.reported_binding_clash = true;
            return u32::MAX;
        }
        let tvs: Vec<u32> = var_bindings.iter().map(|(tv, _)| *tv).collect();
        // loft#761 — a SELF-recursive generic arrives here while its own template is
        // still being parsed, and the argument type is the type VARIABLE, so `concrete`
        // resolves to that variable rather than to any type. Instantiating against it
        // built a def by cloning the template's variable table — which the parser has
        // not written back yet, so the clone was EMPTY. That def is a plain `Function`,
        // so codegen emitted it and indexed a table of length 0: "index out of bounds:
        // the len is 0 but the index is 1", before any of the program ran.
        //
        // There is nothing to instantiate at this point, and nothing to diagnose
        // either: the call is perfectly good, it just has no concrete type YET. Answer
        // the TEMPLATE. A real caller then instantiates for its own concrete type, and
        // `instantiate_nested_generics` retargets this call at that monomorph while
        // building it — the same path any other call to a generic takes from inside a
        // template. Self-recursion terminates there on the `existing` check.
        if var_bindings
            .iter()
            .any(|(_, b)| tvs.iter().any(|tv| self.data.type_mentions(b, *tv)))
        {
            return g_nr;
        }
        // An INFERRED tuple bound to a variable (`same((1, 2), (1, 2))`) has no stored form
        // until something registers it, and the monomorph's body is built on that form —
        // the same registration an inferred tuple literal takes (`Data::ensure_tuple_defs`,
        // loft#943).  Without it the instance looked up `def(u32::MAX)`: an ICE.
        for (_, b) in &var_bindings {
            self.data.ensure_tuple_defs(&mut self.lexer, b);
        }
        if var_bindings.iter().any(|(_, b)| b.is_unknown()) {
            if !self.first_pass {
                let unbound: Vec<String> = var_bindings
                    .iter()
                    .filter(|(_, b)| b.is_unknown())
                    .map(|(v, _)| Data::type_var_spelling(self.data.def(*v).name()).to_string())
                    .collect();
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{name}` cannot tell what {} is — no argument's type names it (a `null`, \
                     or a lambda that answers only `null`, names none)",
                    unbound.join(" and ")
                );
                // Refused at the binding, as a clash is: nothing further to say.
                self.reported_binding_clash = true;
            }
            return u32::MAX;
        }
        // `concrete` is the first variable's binding: what the associated types and the
        // bound check are read against (C3 carries them per variable).
        let concrete = var_bindings[0].1.clone();
        let type_nr = self.data.type_def_nr(&concrete);
        // `D-Key` (@FR-G-Key) — the instance's key names its template and every bound type.
        let mangled = if crate::keys::instance_key_enabled() {
            let spelling: Vec<String> = var_bindings
                .iter()
                .map(|(_, b)| self.data.identity_spelling(b))
                .collect();
            crate::data::Data::mangle_instance(&spelling.join("#"), self.data.def(g_nr).name())
        } else {
            self.method_shaped_instance_key(g_nr, name, &concrete, type_nr)
        };
        if crate::env_once!(std::env::var_os("LOFT_TRACE_INSTANCE_KEY").is_some()) {
            let spelling: Vec<String> = var_bindings
                .iter()
                .map(|(_, b)| self.data.identity_spelling(b))
                .collect();
            eprintln!(
                "[instance-key] {} -> {}",
                self.method_shaped_instance_key(g_nr, name, &concrete, type_nr),
                crate::data::Data::mangle_instance(&spelling.join("#"), self.data.def(g_nr).name())
            );
        }
        // Return existing instantiation if already created.
        let existing = self.data.def_nr(&mangled);
        if existing != u32::MAX {
            if crate::env_once!(std::env::var_os("LOFT_DBG_ACC").is_some()) {
                eprintln!(
                    "[acc-mono] existing {mangled} pass1={} buffers={}",
                    self.first_pass,
                    self.data.def(existing).text_work_buffers()
                );
            }
            return existing;
        }
        // @PLN125 A2c — a monomorph substitutes a LIST of holders, not one.  The type
        // variable is the first, and each associated type the bounds declare follows: the
        // template holds `Source.Rows` wherever the interface wrote `Self.Rows`, and this
        // is where that name becomes the implementor's own companion.  With no associated
        // types the list is one long and every step below reads exactly as it did.
        let mut bindings: Vec<(u32, Type)> = var_bindings.clone();
        bindings.extend(self.associated_bindings(g_nr, &var_bindings));
        // @PLN165 D5 — each open instance the bindings close (`Box<T>` ↦ `Box<integer>`),
        // substituted wherever the template's types mention it.
        let open = self.open_instance_bindings(&bindings, g_nr);
        bindings.extend(open);
        // Clone the template data before mutating self.data.
        let tmpl_code = self.data.definitions[g_nr as usize].code.clone();
        let tmpl_returned = self.data.definitions[g_nr as usize].returned.clone();
        let tmpl_attrs: Vec<_> = self.data.definitions[g_nr as usize]
            .attributes
            .iter()
            .map(|a| Argument {
                name: a.name.clone(),
                typedef: Self::substitute_all(a.typedef.clone(), &bindings),
                default: a.value.clone(),
                // C124 — a parameter's `const` is part of the SIGNATURE a call is judged by, so
                // the monomorph keeps the template's: `sum<T>(v: const vector<T>)` answered a
                // plain `v` once instantiated, and refused `sum(ps, 0)` over a const `ps`.
                constant: a.value_const,
                // A generic instantiation copies the template's parameters; the modifier
                // position belongs to the template's own source, not this synthetic copy.
                ref_pos: (0, 0),
                const_pos: (0, 0),
            })
            .collect();
        // Which parameters are HIDDEN (a record return's `__retbuf`, a text return's buffer):
        // `Argument` carries no such flag, so it rides beside the list and is set below.
        let tmpl_hidden: Vec<bool> = self.data.definitions[g_nr as usize]
            .attributes
            .iter()
            .map(|a| a.hidden)
            .collect();
        let tmpl_vars = self.data.definitions[g_nr as usize].variables.clone();
        let tmpl_pos = self.data.definitions[g_nr as usize].position;
        // The per-element iteration stride for vector<T=concrete> — from the
        // ONE home (`vector_elem_iter_stride`), threaded into the fixup so the
        // generic path can never drift from the direct-emission stride again.
        // Per binding, because an associated type is as much a element type as the
        // type variable is.
        let mut new_code = tmpl_code;
        for (holder, bound_to) in &bindings {
            let iter_stride = self.vector_elem_read(bound_to);
            new_code = Self::substitute_type_in_value(
                new_code,
                *holder,
                bound_to,
                iter_stride,
                &self.data,
            );
        }
        // `from_tv` computed on the PRE-substitution template return, identically to
        // `predict_generic_return_type`, so the second-pass instantiated return type
        // matches the first-pass prediction (the cross-pass H5 contract).
        let from_tv = tvs
            .iter()
            .any(|tv| Self::return_is_the_type_var(&tmpl_returned, *tv));
        let tmpl_ret_deps: Vec<u16> = tmpl_returned.depend();
        let mut new_returned =
            self.tuple_return_rewrite(Self::substitute_all(tmpl_returned, &bindings), from_tv);
        // @FR-F-Ret — the template's RETURN DEPS come along.  `substitute_all` replaces the
        // type variable and drops the deps it carried, and those deps are what say *this
        // result borrows argument N*: without them a `fn g<T>(x: T) -> T { x }` instance
        // reads as a fresh owner, the caller binds the raw argument store, and mutating
        // the result mutated the argument on both backends (QUALITY-history.md B7t) — where the
        // concrete twin, whose `-> Ctr["x"]` says so, is copied by the caller.  Attribute
        // indices are frame-independent and the instance copies the template's parameters
        // in order, so re-attaching is exact; `expand_deferred_par` does the same for a
        // par worker's return.  A tuple carries no dep list of its own and is boxed below.
        // ATTR space, all of them, and NOTHING the substitution dragged in.  The
        // paragraph above says these entries are attribute indices, and
        // `Definition.returned` is the DEF-space home `call_dependencies` reads with
        // `as_attr_indices` — so `Type::depending`, which builds `Deps::frame1`, tagged
        // them as caller frame variables.  It also REPLACES the list rather than
        // appending, so a template returning more than one dep kept only the last.
        //
        // The unconditional write is the other half: `substitute_all` puts the CONCRETE
        // type where the type variable was, and that type was read at the CALL SITE, so
        // it arrives carrying the CALLER's frame deps.  Those name nothing in the
        // callee's attribute space and must not survive into this home — a keyed
        // monomorph reached `as_attr_indices` holding a caller local's frame number.
        // Setting the list even when the template had none is what drops them; an empty
        // ATTR list is the owned answer, and the `Own::Borrowed` block below re-derives
        // any real borrow from the instance's own body.
        if !matches!(new_returned.base(), Type::Tuple(_)) {
            new_returned = new_returned.with_deps(&crate::data::Deps::attrs(tmpl_ret_deps));
        }
        // Register the new definition.
        let d_nr = self.data.add_def(&mangled, &tmpl_pos, DefType::Function);
        // @PLN204 — the body is the template's: 64-bit when the template's file is.
        if !self.data.width32_source(self.data.def(g_nr).source) {
            self.data.mark_wide_instance(d_nr);
        }
        for (a, hidden) in tmpl_attrs.iter().zip(&tmpl_hidden) {
            let a_nr = self
                .data
                .add_attribute(&mut self.lexer, d_nr, &a.name, a.typedef.clone());
            self.data.set_attr_value(d_nr, a_nr, a.default.clone());
            // C124 — registered here and not through `Data::add_fn`, so the template's
            // `const` has to be carried by hand, or the instance answers a plain parameter.
            self.data.definitions[d_nr as usize].attributes[a_nr].value_const = a.constant;
            // `(G-Mono)` — and so does HIDDEN: an instance whose `__retbuf` read as a declared
            // parameter was not its twin's signature.  Native kept the buffer where the twin's
            // value record drops it, and a variant decision that forwards its leaves' hidden
            // buffers minted an undeclared one for the instance (@PLN165 B7).
            self.data.definitions[d_nr as usize].attributes[a_nr].hidden = *hidden;
        }
        self.data.set_returned(d_nr, new_returned.clone());
        // Trace point: full instantiation result.  Used during plan-17
        // (A) debugging when verifying that the second-pass def
        // creation produced the right monomorphised signature.
        // Enable with `LOFT_TRACE=generic`.
        crate::loft_trace!(
            generic,
            "instantiate name={} mangled={} d_nr={} concrete={:?} returned={:?}",
            name,
            mangled,
            d_nr,
            concrete,
            new_returned,
        );
        self.fill_monomorph_body(d_nr, new_code, &tmpl_vars, &bindings);
        self.derive_monomorph_return_deps(d_nr, g_nr);
        // loft#1023 — a template declared BELOW its caller has not had its pass-2 body
        // parsed yet when the call instantiates, so the monomorph above was built from the
        // PASS-1 body.  Record it and re-derive once the whole file is through.
        if !self.first_pass && !self.pass2_bodies.contains(&g_nr) {
            self.stale_monomorphs.push((d_nr, g_nr, bindings.clone()));
        }
        self.instantiate_nested_generics(d_nr, &bindings);
        // loft#1934 — and again once the nested calls name their INSTANCES: a tail that
        // forwards a nested generic's borrow (`g5<T>(x) -> T { g1(x) }`) read the TEMPLATE's
        // dep-free `-> T` above.  Only where no dep was derived, so nothing is written twice.
        self.derive_monomorph_return_deps(d_nr, g_nr);
        // loft#2179 — what the deps could not name, the body now copies: an instance declared
        // fresh returns no parameter's store.
        self.copy_param_returns_of_fresh_instance(d_nr);
        // The body's text-return promotion ran while its tail call still named the nested
        // TEMPLATE (`inner(s, c)` in `outer<S>(s: S) -> S?`): not a text call, nothing to
        // promote.  Now that the call names `inner`'s monomorph, ask again — a `-> text?`
        // forwarder that stays unpromoted copies the callee's result into a frame-local
        // `__ret_N` and hands up a view of it, one orphan per call (loft#1357).  Only where
        // nothing was promoted: a second hidden buffer would move the ABI.
        if self.data.def(d_nr).text_work_buffers() == 0 {
            self.promote_monomorph_text_return(d_nr);
        }
        // I6: verify the concrete type satisfies every declared bound.
        // Emit a diagnostic and return u32::MAX if any required method is missing.
        if !self.check_satisfaction(g_nr, &var_bindings, &bindings[var_bindings.len()..]) {
            // Return d_nr (not u32::MAX) so `call` doesn't emit a redundant
            // "Unknown function" error — the satisfaction error is sufficient.
            // The function won't execute because parsing will halt on errors.
        }
        self.warn_instance_tail(d_nr, g_nr, &bindings);
        d_nr
    }

    /// F9 (@PLN165 C1): does one type variable take two types from two parameters at this
    /// call — `same(1, "x")` for `same<T>(a: T, b: T)`?  The variable is bound by the first
    /// parameter that relates to its argument; a later one whose argument cannot take the
    /// parameter AS BOUND is the clash, refused naming both parameters.  The predicate is the
    /// argument check's own (`convert_admitting`, as an overload trial asks it), so this
    /// refuses exactly what the check after instantiation would, and says why.
    pub(super) fn binding_clash(
        &mut self,
        g_nr: u32,
        name: &str,
        types: &[Type],
        var_bindings: &[(u32, Type)],
    ) -> bool {
        let params: Vec<(String, Type)> = self
            .data
            .def(g_nr)
            .attributes
            .iter()
            .filter(|a| !a.hidden)
            .map(|a| (a.name.clone(), a.typedef.clone()))
            .collect();
        for (v, bound) in var_bindings {
            if bound.is_unknown() || self.data.mentions_type_var(bound) {
                continue;
            }
            let binders: Vec<usize> = params
                .iter()
                .zip(types)
                .enumerate()
                .filter(|(_, ((_, p), a))| {
                    self.data.type_mentions(p, *v)
                        && !Self::resolve_type_var(&self.data, p, *v, a).is_unknown()
                })
                .map(|(i, _)| i)
                .collect();
            let Some(&first) = binders.first() else {
                continue;
            };
            for &i in &binders[1..] {
                // A parameter that also names a variable no argument binds cannot be judged
                // here: that variable's refusal is the one to give (`|x| { null }` for
                // `f: fn(T) -> U` read as a clash on `T`).
                if var_bindings
                    .iter()
                    .any(|(u, b)| b.is_unknown() && self.data.type_mentions(&params[i].1, *u))
                {
                    continue;
                }
                // Closed: `b: Grid<T>` at `T = text` is `Grid<text>`, where a plain substitution
                // leaves the open instance `Grid<T>`, which no argument converts to.
                let expected = self.close_open(&params[i].1, var_bindings);
                let mut trial = Value::Null;
                if self.convert_admitting(&mut trial, &types[i], &expected) {
                    continue;
                }
                let other = Self::resolve_type_var(&self.data, &params[i].1, *v, &types[i]);
                // The SAME type through both: what failed is not the variable but the rest of
                // the parameter (`fn(D)` for `fn(const T)`), and the argument check names it.
                if other == *bound {
                    continue;
                }
                let spelled = Data::type_var_spelling(self.data.def(*v).name()).to_string();
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{name}` binds {spelled} to {} through `{}` and to {} through `{}` — a type \
                     variable is one type at a call; pass `{}` a value of the same type, or give \
                     the two parameters a variable each",
                    bound.source_name(&self.data),
                    params[first].0,
                    other.source_name(&self.data),
                    params[i].0,
                    params[i].0
                );
                return true;
            }
        }
        false
    }

    /// Is `d` a lambda written inside a template ([`Parser::mark_template_lambda`])?
    pub(crate) fn is_template_lambda(&self, d: u32) -> bool {
        (d as usize) < self.data.definitions.len()
            && self.data.def_type(d) == DefType::Generic
            && self.data.def(d).name().starts_with("n___lambda_")
    }

    /// The instance of the template lambda `d` that belongs to the instance being filled
    /// (`self.context`), bound with that instance's bindings — `(G-Mono)` for a lambda: an
    /// instance's lambda is the lambda its concrete twin would have written.  Named after the
    /// enclosing instance (`n___lambda_3_in_t_7integer_f`), so each instance owns its own
    /// and a re-derivation finds it again.  Its closure record is instantiated beside it,
    /// because a capture typed by a variable has no width in the template's.
    pub(crate) fn instantiate_template_lambda(&mut self, d: u32) -> u32 {
        let bindings = self.instance_bindings.clone();
        let owner = self.data.def(self.context).name().to_string();
        let name = format!("{}_in_{owner}", self.data.def(d).name());
        let mut inst = self.data.def_nr(&name);
        let rec = self.data.def(d).closure_record();
        if inst == u32::MAX {
            let pos = self.data.definitions[d as usize].position;
            let attrs: Vec<(String, Type, Value, bool, bool)> = self.data.definitions[d as usize]
                .attributes
                .iter()
                .map(|a| {
                    (
                        a.name.clone(),
                        Self::substitute_all(a.typedef.clone(), &bindings),
                        a.value.clone(),
                        a.value_const,
                        a.hidden,
                    )
                })
                .collect();
            let tmpl_returned = self.data.definitions[d as usize].returned.clone();
            let ret_deps = tmpl_returned.depend();
            let mut returned = Self::substitute_all(tmpl_returned, &bindings);
            if !matches!(returned.base(), Type::Tuple(_)) {
                returned = returned.with_deps(&crate::data::Deps::attrs(ret_deps));
            }
            inst = self.data.add_def(&name, &pos, DefType::Function);
            self.data.definitions[inst as usize].synthetic = Some("lambda_instance");
            let rec_inst = if rec == u32::MAX {
                u32::MAX
            } else {
                self.instantiate_closure_record(rec, &owner, &bindings)
            };
            self.data.definitions[inst as usize].closure_record = rec_inst;
            for (a_name, a_tp, a_value, a_const, a_hidden) in attrs {
                let a_tp = match a_tp.base() {
                    Type::Reference(r, _) if *r == rec && rec != u32::MAX => {
                        Type::Reference(rec_inst, crate::data::Deps::none())
                    }
                    _ => a_tp,
                };
                let a_nr = self
                    .data
                    .add_attribute(&mut self.lexer, inst, &a_name, a_tp);
                self.data.set_attr_value(inst, a_nr, a_value);
                self.data.definitions[inst as usize].attributes[a_nr].value_const = a_const;
                self.data.definitions[inst as usize].attributes[a_nr].hidden = a_hidden;
            }
            self.data.set_returned(inst, returned);
        }
        let rec_inst = self.data.def(inst).closure_record();
        let tmpl_code = self.data.definitions[d as usize].code.clone();
        let tmpl_vars = self.data.definitions[d as usize].variables.clone();
        let mut code = tmpl_code;
        for (holder, bound_to) in &bindings {
            let iter_stride = self.vector_elem_read(bound_to);
            code = Self::substitute_type_in_value(code, *holder, bound_to, iter_stride, &self.data);
        }
        self.fill_monomorph_body(inst, code, &tmpl_vars, &bindings);
        // The `__closure` parameter names the TEMPLATE's record in the copied table; the
        // instance reads its own.
        if rec != u32::MAX {
            let vars = &mut self.data.definitions[inst as usize].variables;
            for v in 0..vars.count() {
                if matches!(vars.tp(v).base(), Type::Reference(r, _) if *r == rec) {
                    vars.set_type(v, Type::Reference(rec_inst, crate::data::Deps::none()));
                }
            }
            self.closure_parameter_last(inst);
        }
        if !bindings.is_empty() {
            self.instantiate_nested_generics(inst, &bindings);
        }
        inst
    }

    /// Keep a lambda's `__closure` parameter LAST.  A closure call hands the record over after
    /// every other argument, work buffers included, so it must be the last attribute and the
    /// last argument variable.  A template lambda's instance gains a text-return buffer only
    /// once its return is known to be `text` — after the closure parameter it copied from the
    /// template — so the two are swapped back: the attributes, the return's attribute-space
    /// deps, and the argument variables with every reference to them (the 3-way renumbering
    /// `av_renumber_retbuf` uses for a late return buffer).
    pub(super) fn closure_parameter_last(&mut self, d: u32) {
        loop {
            let def = &self.data.definitions[d as usize];
            let n = def.attributes.len();
            let Some(ci) = def.attributes.iter().position(|a| a.name == "__closure") else {
                return;
            };
            if ci + 1 >= n {
                return;
            }
            let args = def.variables.arguments();
            let (Some(&a), Some(&b)) = (args.get(ci), args.get(ci + 1)) else {
                return;
            };
            self.data.definitions[d as usize]
                .attributes
                .swap(ci, ci + 1);
            // The name index answers the same question as the list, so it moves with it.
            for i in [ci, ci + 1] {
                let n = self.data.definitions[d as usize].attributes[i].name.clone();
                self.data.definitions[d as usize].attr_names.insert(n, i);
            }
            let ret = self.data.definitions[d as usize].returned.clone();
            let deps: Vec<u16> = ret
                .depend()
                .into_iter()
                .map(|x| {
                    if x as usize == ci {
                        (ci + 1) as u16
                    } else if x as usize == ci + 1 {
                        ci as u16
                    } else {
                        x
                    }
                })
                .collect();
            if !ret.depend().is_empty() {
                self.data.definitions[d as usize].returned =
                    ret.with_deps(&crate::data::Deps::attrs(deps));
            }
            let mut code =
                std::mem::replace(&mut self.data.definitions[d as usize].code, Value::Null);
            let vars = &mut self.data.definitions[d as usize].variables;
            let tmp = vars.count();
            Self::renumber_frame_var(&mut code, a, tmp);
            Self::renumber_frame_var(&mut code, b, a);
            Self::renumber_frame_var(&mut code, tmp, b);
            vars.renumber_frame_in_types(a, tmp);
            vars.renumber_frame_in_types(b, a);
            vars.renumber_frame_in_types(tmp, b);
            vars.swap_variables(a, b);
            self.data.definitions[d as usize].code = code;
        }
    }

    /// The closure record of a template lambda's instance: the template's record with every
    /// capture's type bound, stored the way a concrete capture of that type is
    /// ([`Parser::closure_attr_type`]), and laid out now — the instance's closure is built
    /// before the pass ends.
    pub(super) fn instantiate_closure_record(
        &mut self,
        rec: u32,
        owner: &str,
        bindings: &[(u32, Type)],
    ) -> u32 {
        let name = format!("{}_in_{owner}", self.data.def(rec).name());
        let existing = self.data.def_nr(&name);
        if existing != u32::MAX {
            return existing;
        }
        let pos = self.data.definitions[rec as usize].position;
        let fields: Vec<(String, Type, bool)> = self.data.definitions[rec as usize]
            .attributes
            .iter()
            .map(|a| {
                (
                    a.name.clone(),
                    Self::substitute_all(a.typedef.clone(), bindings),
                    a.value_const,
                )
            })
            .collect();
        let r = self.data.add_def(&name, &pos, DefType::Struct);
        self.data.definitions[r as usize].synthetic = Some("closure_instance");
        self.data.definitions[r as usize].returned = Type::Reference(r, crate::data::Deps::none());
        for (f_name, f_tp, f_const) in fields {
            let stored = self.closure_attr_type(&f_tp);
            let a = self.data.add_attribute(&mut self.lexer, r, &f_name, stored);
            self.data.definitions[r as usize].attributes[a].value_const = f_const;
        }
        crate::typedef::fill_database(&mut self.data, &mut self.database, r);
        self.database.lay_out_record(self.data.def(r).known_type());
        r
    }

    /// Substitute a template's body + variable table into the monomorph `d_nr`.
    ///
    /// Split out of [`Self::try_generic_instantiation`] so loft#1023's re-derivation can
    /// run exactly the same steps once the template's own pass-2 body exists.  The
    /// SIGNATURE is not re-derived: attributes and the return type come from the
    /// template's declaration, which pass 1 already has — only the body was ever stale.
    /// P223 in a monomorph (@PLN165 D11): the interpreter's text `Set` clears its destination
    /// before it evaluates the value, so an assignment whose value READS its destination is
    /// staged through a work text — `acc = f(acc)` otherwise hands `f` the cleared `acc`.  The
    /// parse decides that where it sees a text variable; a template's `acc: T` is not one, so
    /// each instance where a variable BECAME text decides it here, the parse's own way.
    pub(super) fn stage_text_self_reads(&mut self, code: &mut Value, tmpl_vars: &Function) {
        let became_text: Vec<u16> = (0..self.vars.count())
            .filter(|&v| {
                matches!(self.vars.tp(v).base(), Type::Text(_))
                    && (v as usize) < tmpl_vars.count() as usize
                    && !matches!(tmpl_vars.tp(v).base(), Type::Text(_))
            })
            .collect();
        if became_text.is_empty() {
            return;
        }
        code.map_nodes(&mut |n| {
            let Value::Set(v, value) = n else {
                return;
            };
            if !became_text.contains(v) || !value.reads_var(*v) {
                return;
            }
            // A branch delivers per ARM into the destination, as the parse binds one
            // (`try_branch_text_bind`); each arm's assignment is then staged on its own if it
            // reads the destination — the walk visits them next.  Appended whole, the two
            // arms' values met as `String` and `&String` in the native emission.
            if let Value::If(..) = value.unspan() {
                let v = *v;
                let Value::If(c, t, e) = std::mem::replace(value.as_mut(), Value::Null)
                    .unspan()
                    .clone()
                else {
                    return;
                };
                *n = Value::If(c, Box::new(v_set(v, *t)), Box::new(v_set(v, *e)));
                return;
            }
            let (v, value) = (*v, std::mem::replace(value.as_mut(), Value::Null));
            let work = self.vars.work_text_p2(&mut self.lexer);
            *n = Value::Insert(vec![
                self.cl("OpClearText", &[Value::Var(work)]),
                self.cl("OpAppendText", &[Value::Var(work), value]),
                v_set(v, Value::Var(work)),
            ]);
        });
    }

    /// Lower each marked format hole of a fresh monomorph with the type the instance binds its
    /// type variable to: `append_data`, the lowering a concrete hole gets, spec checks and all.
    /// Run inside the instance's frame, as [`Self::resolve_content_eq`] is.
    pub(super) fn resolve_pending_formats(&mut self, code: &mut Value, bindings: &[(u32, Type)]) {
        if let Value::Call(_, args) = code.unspan_mut()
            && args.len() == 14
            && matches!(args[5].unspan(), Value::Int(Self::FORMAT_PENDING))
            && let (Value::Var(append), Value::Int(holder), Value::Int(append_value)) =
                (args[0].unspan(), args[6].unspan(), args[7].unspan())
            && let (Value::Int(dir), Value::Int(radix)) = (args[3].unspan(), args[8].unspan())
            && let (Value::Boolean(plus), Value::Boolean(note), Value::Boolean(float)) =
                (args[9].unspan(), args[10].unspan(), args[11].unspan())
            && let (Value::Text(spec), Value::Text(token)) = (args[12].unspan(), args[13].unspan())
        {
            let concrete = bindings
                .iter()
                .find(|(h, _)| *h as i32 == *holder)
                .map_or(Type::Unknown(0), |(_, t)| t.clone());
            let (append, append_value) = (*append, *append_value as u16);
            let (spec, token) = (spec.clone(), token.clone());
            let state = OutputState {
                radix: *radix,
                width: args[2].clone(),
                token: &token,
                plus: *plus,
                note: *note,
                dir: *dir,
                float: *float,
                spec: &spec,
            };
            let format = args[1].clone();
            let mut list = Vec::new();
            self.append_data(concrete, &mut list, append, append_value, &format, state);
            *code = Value::Insert(list);
            return;
        }
        code.for_each_child_mut(&mut |c| self.resolve_pending_formats(c, bindings));
    }

    /// `@FR-G-Mono` — size each pending `sizeof` of a fresh monomorph at the type its holder is
    /// bound to, with the lowering a concrete `sizeof` gets (loft#2054).
    pub(super) fn resolve_pending_sizeof(&mut self, code: &mut Value, bindings: &[(u32, Type)]) {
        if let Value::Call(_, args) = code.unspan()
            && args.len() == 2
            && matches!(args[0].unspan(), Value::Int(Self::SIZEOF_PENDING))
            && let Value::Int(holder) = *args[1].unspan()
            && let Some((_, concrete)) = bindings.iter().find(|(h, _)| *h as i32 == holder)
        {
            let concrete = concrete.clone();
            *code = Value::Int(self.packed_size(&concrete, u32::MAX));
            return;
        }
        code.for_each_child_mut(&mut |c| self.resolve_pending_sizeof(c, bindings));
    }

    /// Lower each marked bound `==` of a fresh monomorph (`(G-Sat-Eq)`): the call
    /// `OpEqContent(a, b, PENDING, holder)` becomes the CONCRETE `a == b` for the type the
    /// monomorph binds `holder` to — `call_op`, the one lowering a concrete site gets, so a
    /// generic and a concrete `==` cannot answer differently.  Run inside the instance's frame
    /// (`fill_monomorph_body`), where a lowering that needs a temporary (a tuple's) makes it.
    pub(super) fn resolve_content_eq(&mut self, code: &mut Value, bindings: &[(u32, Type)]) {
        if let Value::Call(_, args) = code.unspan_mut()
            && args.len() == 4
            && matches!(args[2].unspan(), Value::Int(Self::CONTENT_EQ_PENDING))
            && let Value::Int(holder) = *args[3].unspan()
        {
            let concrete = bindings
                .iter()
                .find(|(h, _)| *h as i32 == holder)
                .map_or(Type::Unknown(0), |(_, t)| t.clone());
            let operands = [args[0].clone(), args[1].clone()];
            let mut lowered = Value::Null;
            let tp = self.call_op(
                &mut lowered,
                "==",
                &operands,
                &[concrete.clone(), concrete.clone()],
            );
            if matches!(tp.peel_link(), Type::Boolean) && lowered != Value::Null {
                *code = lowered;
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Internal error: `==` on `{}` inside a generic has no comparison \
                     (report this as a bug)",
                    concrete.source_name(&self.data)
                );
                *code = Value::Boolean(false);
            }
            return;
        }
        code.for_each_child_mut(&mut |c| self.resolve_content_eq(c, bindings));
    }

    /// @FR-F-ParamScalar / @FR-G-Mono — a generic's `p: T` is spelled as a reference to `T`'s
    /// placeholder, so the template lowered `p = …` as a HEAP-parameter rebind: an entry
    /// witness, a guarded release and a detach of `p` (`rebind_local_heap_param`).  That is
    /// the twin's answer for a record instance and the wrong one for any other: an `integer`
    /// or `float` instance was an internal error on the witness's slot, and a `text` one wrote
    /// the borrowed argument.  An instance whose parameter is not a heap type drops the
    /// witness's statements and its mapping, so the scope pass adds no exit release either.
    pub(super) fn strip_scalar_instance_rebinds(&mut self, code: &mut Value) {
        fn strip(code: &mut Value, data: &Data, p: u16, orig: u16) {
            let is_var = |v: &Value, n: u16| matches!(v.unspan(), Value::Var(x) if *x == n);
            let witness_op = |v: &Value| match v.unspan() {
                Value::Call(op, a) => match data.def(*op).name() {
                    "OpFreeRefIfDistinct" => {
                        a.len() == 2 && is_var(&a[0], p) && is_var(&a[1], orig)
                    }
                    "OpInitRefSentinel" => a.len() == 1 && is_var(&a[0], p),
                    "OpPutRef" => a.len() == 2 && is_var(&a[0], orig),
                    _ => false,
                },
                Value::Set(v, _) => *v == orig,
                _ => false,
            };
            match code {
                Value::Block(bl) => bl.operators.retain(|o| !witness_op(o)),
                Value::Insert(list) => list.retain(|o| !witness_op(o)),
                _ => {}
            }
            code.for_each_child_mut(&mut |c| strip(c, data, p, orig));
        }
        for p in 0..self.vars.count() {
            let Some(orig) = self.vars.rebind_orig(p) else {
                continue;
            };
            let heap = matches!(
                self.vars.tp(p).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            ) || crate::parser::vectors::is_keyed(self.vars.tp(p));
            if heap {
                continue;
            }
            strip(code, &self.data, p, orig);
            self.vars.clear_rebind_orig(p);
        }
    }

    /// @FR-F-ParamScalar / @FR-G-Mono — a `text` PARAMETER the instance body assigns gets the
    /// owned shadow local a plain function's parse gives it (`__tp_<name>`, seeded from the
    /// argument at entry, every use renamed).  The template decided its assignments while the
    /// parameter was still `T`, so the instance wrote the BORROWED argument slot:
    /// `fn g<T>(p: T, q: T) -> T { p = q; p }` at `T = text` answered the argument on
    /// `--interpret` and did not build on `--native`.  The instance takes the twin's answer.
    pub(super) fn promote_assigned_text_params(&mut self, code: &mut Value) {
        let params = self.vars.count();
        for p in 0..params {
            if !self.vars.is_argument(p)
                || self.vars.name(p).starts_with("__")
                || !matches!(self.vars.tp(p).base(), Type::Text(_))
            {
                continue;
            }
            let mut assigned = false;
            code.walk(&mut |n| {
                if let Value::Set(v, _) = n.unspan()
                    && *v == p
                {
                    assigned = true;
                }
            });
            if !assigned {
                continue;
            }
            let name = self.vars.name(p).to_string();
            let shadow = self.vars.add_variable(
                &format!("__tp_{name}"),
                &Type::Text(Deps::none()),
                &mut self.lexer,
            );
            self.vars.set_promoted_from(shadow, p);
            collections::rename_var(code, p, shadow);
            if let Value::Block(bl) = code {
                bl.operators.insert(0, v_set(shadow, Value::Var(p)));
            }
        }
    }

    pub(super) fn fill_monomorph_body(
        &mut self,
        d_nr: u32,
        code: Value,
        tmpl_vars: &Function,
        bindings: &[(u32, Type)],
    ) {
        // Copy the variable table with substituted types.
        let mut vars = Function::copy(tmpl_vars);
        // `done` says *this function's code already carries the scope pass's output*, and an
        // instance's code is new: nothing has scoped it yet.  The copy inherits the TEMPLATE's
        // flag, which a template loaded from the stdlib bundle carries as `true` (a snapshot is
        // reconstructed as scoped) — so on a warm start `scopes::check` skipped every stdlib
        // generic's instance, no variable got a slot, and `tree_walk<Crate>` was an internal
        // compiler error on both backends where a cold start ran it.
        vars.done = false;
        for (holder, bound_to) in bindings {
            vars.substitute_type(*holder, bound_to);
        }
        // loft#2156 — and each vector backing's wrapper, which names the variable only
        // through its field (`Self::instance_vector_wrapper`).
        for v in 0..vars.count() {
            let tp = vars.tp(v).clone();
            if let Some(fresh) = self.instance_vector_wrapper(&tp, bindings) {
                vars.set_type(v, fresh);
            }
        }
        vars.drop_scalar_deps();
        // @FR-G-Mono — a match payload binding is decided by its FIELD's type, and the template
        // decided it before that type was known: a field of type `T` reads as a record, so the
        // binding became a VIEW of the subject (a dep on it, never freed).  Bound to `text`, the
        // twin's binding is an owned copy — `OpGetText` hands it a `String` of its own, freed at
        // scope exit — and keeping the view marking leaked that copy once per call on the
        // interpreter.  The instance takes the twin's answer.  A payload binding is named
        // `_mv_<field>` and marked `skip_free` when it is a view — the pair
        // `Function::is_overwritten_view` reads — and the template's `mv_field_origin` is gone
        // by now (it is per pass, cleared when the parse hands the table back).
        for v in 0..vars.count() {
            if vars.name(v).starts_with("_mv_")
                && matches!(vars.tp(v).base(), Type::Text(_))
                && vars.is_skip_free(v)
            {
                vars.clear_skip_free(v);
                let owned = vars.tp(v).without_deps();
                vars.set_type(v, owned);
            }
        }
        // P241 fix (2026-05-11): post-substitution rewrite of the
        // parametric vector-element-write triplet to the primitive
        // shape, plus elm-var type patch.  Runs after both code
        // substitution AND vars substitution because the patch needs
        // to override `vars`' substituted-to-primitive elm var type
        // back to `Reference(...)` (it holds a DbRef, not the
        // primitive value).  See `rewrite_generic_vector_writes`.
        let code = Self::rewrite_generic_vector_writes(
            code,
            bindings,
            &mut vars,
            &self.data,
            &mut self.database,
        );
        // loft#1016 — fill in every `x?` whose default the TEMPLATE could not decide.
        // Runs in the MONOMORPH's frame (the `self.vars` / `self.context` swap the
        // drop-cascade builder uses), because a record default parses `S {}` and its
        // work-ref belongs to the function the code lands in, not to whoever happens to
        // be calling.
        let outer_vars = std::mem::replace(&mut self.vars, vars);
        let outer_context = self.context;
        self.context = d_nr;
        // The frame a template lambda is re-lowered in: this instance's bindings, and — for a
        // lambda instance — its own `__closure` parameter, which its capture reads go through.
        let outer_bindings = std::mem::replace(&mut self.instance_bindings, bindings.to_vec());
        let outer_closure_param = self.closure_param;
        self.closure_param = if self.data.def(d_nr).closure_record() == u32::MAX {
            u16::MAX
        } else {
            self.vars.var("__closure")
        };
        let outer_assign_target = std::mem::replace(&mut self.assign_target, u16::MAX);
        let before_work: std::collections::HashSet<u16> =
            self.vars.work_texts().into_iter().collect();
        let outer_set_call_refs = std::mem::take(&mut self.set_call_refs);
        let mut code = code;
        // Inside the instance's own frame (its variables, its context), so a comparison that
        // needs a temporary — a tuple's element-wise one — makes it in the right function.
        self.resolve_content_eq(&mut code, bindings);
        self.resolve_pending_sizeof(&mut code, bindings);
        self.resolve_pending_formats(&mut code, bindings);
        let returned = self.data.def(d_nr).returned().clone();
        if matches!(returned, Type::Optional(_)) && Self::every_result_is_a_tuple_read(&code, true)
        {
            Self::keep_returned_tuple_reads(&mut code, true, &returned);
        }
        let mut code = self.rewrite_generic_type_defaults(code);
        self.settle_parametric_yield_copies(&mut code);
        self.stage_text_self_reads(&mut code, tmpl_vars);
        let set_call_refs = std::mem::replace(&mut self.set_call_refs, outer_set_call_refs);
        self.instance_bindings = outer_bindings;
        self.closure_param = outer_closure_param;
        self.assign_target = outer_assign_target;
        // loft#1175 — a work buffer minted by the rewrite above is not declared at the top
        // level, so `scopes::check` would scope it to the ARGUMENT block it appears in and
        // free it there, before the callee fills it.  Hoisting a top-level `Set` is the same
        // replay `patch_tret_callers` and `retarget_parametric_*` already do for a buffer
        // minted after the parse; without it `--native` declared the `String` inside the
        // argument block and emitted an empty `OpCreateStack`, which does not compile.
        // A call a deferred set-call site lowered here (`TV_SELECT`, @PLN165 B3b) mints its
        // return buffer the same way, and takes the null-init the function parse gives every
        // work-ref — the one predicate, `work_ref_takes_preamble`.  Only those: a buffer
        // another deferred site mints is declared where that site's lowering sets it.
        if let Value::Block(bl) = &mut code {
            for wt in self.vars.work_texts() {
                if !before_work.contains(&wt) {
                    bl.operators
                        .insert(0, v_set(wt, Value::Text(String::new())));
                }
            }
            for r in set_call_refs {
                if self.work_ref_takes_preamble(r) {
                    bl.operators.insert(0, v_set(r, Value::Null));
                }
            }
        }
        self.strip_scalar_instance_rebinds(&mut code);
        self.promote_assigned_text_params(&mut code);
        let vars = std::mem::replace(&mut self.vars, outer_vars);
        self.context = outer_context;
        self.data.definitions[d_nr as usize].code = code;
        self.data.definitions[d_nr as usize].variables = vars;
        // @PLN85 category A — engage the text-return promotion the parse-time
        // path skips for IR-substituted monomorphs, so a `-> text` monomorph
        // delivers through a hidden `&text` buffer (no orphaned owned String).
        // Runs BEFORE returning `d_nr` so the call site sees the promoted ABI.
        self.promote_monomorph_text_return(d_nr);
        // @FR-F-Ret — and its tuple twin: a `-> (T, …)` whose shape the declaration deferred
        // is boxed by `tuple_return_rewrite` above, and the body's tuple tails are rewritten
        // into that synthetic record here, so signature and body agree on both passes.
        self.promote_monomorph_tuple_return(d_nr);
        // @PLN165 D5 — and the record twin, for a return an open instance became.
        let from_open = matches!(self.data.def(d_nr).returned().base(),
            Type::Reference(td, _) if bindings.iter().any(|(h, b)|
                self.data.is_open_instance(*h)
                    && matches!(b.base(), Type::Reference(c, _) if c == td)));
        self.promote_monomorph_record_return(d_nr, from_open);
        // @FR-F-Ret / @FR-B-Copy — and the vector twin: a `T`-typed local bound from another
        // vector COPIES, and a `-> T` that hands a vector argument up returns a copy of it.
        let holders: Vec<u32> = bindings.iter().map(|(h, _)| *h).collect();
        self.promote_monomorph_vector_return(d_nr, tmpl_vars, &holders);
        // loft#1387 — and the two-source join twin: a return whose tail is a value branch
        // over two parameters is FRESH (`@FR-F-Ret`), which the non-generic path gets in
        // `parse_block` and a monomorph can only get here.
        self.bind_monomorph_join_return(d_nr);
        // loft#845 — a `"{v}"` on a `vector<T>` was emitted against the row the TEMPLATE
        // could see, which is a vector over the type variable's own storage.  Substitution
        // replaces the type and leaves that row behind, so the dump walked a
        // `vector<integer>` through the wrong schema and printed `{}`.
        self.retarget_parametric_coroutine_next(d_nr);
        self.retarget_parametric_vector_format(d_nr);
        // loft#1070 — and any TYPE ROW the template baked as a constant.  Substitution
        // rewrites types, never the `const u16` schema id an op already carries, so a site
        // that was lowered while the element was still `T` kept pointing at
        // `__typevar_T`'s row.
        // loft#1365 — and drop the tuple-member copy this instantiation does not need,
        // BEFORE the rows are retargeted: a scalar binding contributes no row mapping, so
        // the copy left standing would keep the type variable's own row.
        self.collapse_parametric_tuple_member_copies(d_nr);
        self.retarget_parametric_type_rows(d_nr, bindings, tmpl_vars);
        // loft#1040 — and lower any `par` clause the template could not: it needs the
        // types this body now carries, so it runs after every substitution above.
        self.expand_deferred_par(d_nr);
    }

    /// `(G-Own)`, @FR-G-Own — decide every `yield` copy the TEMPLATE made again, now that this
    /// monomorph knows the type it yields.
    ///
    /// A template yielding a `T` it holds copied it while `T` was still the type variable, so
    /// the copy was built for a record and named the variable's own row.  Substitution rewrites
    /// the types and leaves that choice behind: at `T = integer` the copy allocated a record for
    /// a scalar (`OpDatabase` of an `i64`).  So each copy is rebuilt from its source against the
    /// concrete type, through the one decision a non-generic yield takes
    /// ([`Self::yield_owned_value`]): a scalar or text yield keeps the source, a record or a
    /// vector gets the copy its kind needs, and a type that owns a droppable is refused.  Runs in
    /// the monomorph's frame; a work ref the rebuild mints takes the top-level declaration every
    /// other late work ref takes (`set_call_refs`).
    pub(super) fn settle_parametric_yield_copies(&mut self, code: &mut Value) {
        let first = self.vars.count();
        code.map_nodes(&mut |v| {
            let Value::Block(b) = v.unspan() else {
                return;
            };
            if b.name != "yield_copy" {
                return;
            }
            let tp = b.result.clone();
            let Some(mut src) = self.tuple_member_copy_source(v) else {
                return;
            };
            self.yield_owned_value(&mut src, &tp);
            *v = src;
        });
        for r in first..self.vars.count() {
            if self.work_ref_takes_preamble(r) {
                self.set_call_refs.push(r);
            }
        }
    }

    /// loft#1040 — lower every `par` clause the TEMPLATE deferred, now that this monomorph
    /// has the types the route is picked from.
    ///
    /// The template left a marker call carrying everything substitution had to reach: the
    /// parsed body, the input vector expression, the thread count, the pre-loop fill and
    /// the worker's extra arguments — all already rewritten for this monomorph by the
    /// time this runs.  What it did NOT carry is the types: they are read back here from
    /// the monomorph's own variable table (the element) and from the worker instantiated
    /// for that element (the return).  Re-running the ordinary lowering against them
    /// picks the queue variant, the sizes, the accessor and the re-wrap the same way any
    /// non-generic par site does — which is the whole point of waiting.
    ///
    /// Runs in the MONOMORPH's frame: the lowering creates its own index / length /
    /// result variables, and they belong to the function the code lands in.
    pub(super) fn expand_deferred_par(&mut self, d_nr: u32) {
        let marker = self.data.def_nr(crate::parser::collections::PAR_MARKER_FN);
        if marker == u32::MAX || self.par_deferred.is_empty() {
            return;
        }
        // Collect first — re-lowering needs `&mut self` while the walk holds the IR.
        let mut sites: Vec<(usize, Vec<Value>)> = Vec::new();
        self.data.def(d_nr).code.walk(&mut |v| {
            if let Value::Call(d, args) = v
                && *d == marker
                && let Some(Value::Int(id)) = args.first().map(Value::unspan)
                && let Ok(id) = usize::try_from(*id)
                && id < self.par_deferred.len()
                && !sites.iter().any(|(seen, _)| *seen == id)
            {
                sites.push((id, args.clone()));
            }
        });
        if sites.is_empty() {
            return;
        }
        // Instantiate the workers BEFORE the context swap below: instantiation parses
        // against `self.vars` / `self.context` and must see the ordinary ones, not this
        // monomorph's (the same ordering `instantiate_nested_generics` keeps).
        let mut plans: Vec<(usize, Vec<Value>, u32, Type, Type)> = Vec::new();
        for (id, args) in sites {
            let plan = self.par_deferred[id].clone();
            // The element type IS the loop variable's type, and this table has been
            // substituted — so `T` is whatever this monomorph bound it to.
            let elem_tp = self.data.def(d_nr).variables.tp(plan.elem_var).clone();
            // The worker is generic for the same reason this clause was deferred; give it
            // the element type an ordinary call site would have given it.
            let worker = if matches!(self.data.def_type(plan.worker), DefType::Generic) {
                let name = self
                    .data
                    .def(plan.worker)
                    .name()
                    .strip_prefix("n_")
                    .unwrap_or_default()
                    .to_string();
                let inst =
                    self.try_generic_instantiation(u16::MAX, &name, std::slice::from_ref(&elem_tp));
                if inst == u32::MAX { plan.worker } else { inst }
            } else {
                plan.worker
            };
            // The monomorph's signature was built with `substitute_type`, which replaces
            // the type variable and DROPS its deps — and a worker's RETURN deps are what
            // say "this result borrows argument 0".  Losing them makes the par result
            // binding look owned, so the body frees it: with the identity worker a
            // generic often is (`fn w(x: T) -> T { x }`), that frees the very element the
            // caller handed in.  The template still declares them, and they are ATTRIBUTE
            // indices — frame-independent — so re-attaching them is exact.
            let mut ret_type = self.data.def(worker).returned().clone();
            for d in self.data.def(plan.worker).returned().depend() {
                ret_type = ret_type.depending(d);
            }
            plans.push((id, args, worker, ret_type, elem_tp));
        }
        // Now the monomorph's frame: the lowering creates its own index / length / result
        // variables, and they belong to the function the code lands in.
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = d_nr;
        let mut built: Vec<(usize, Value)> = Vec::new();
        for (id, args, worker, ret_type, elem_tp) in plans {
            let plan = self.par_deferred[id].clone();
            let return_size = self.par_return_size(&ret_type, worker);
            let elem_size = self.par_elem_size(&elem_tp);
            let mut it = args.into_iter().skip(1);
            let (Some(body), Some(vec_expr), Some(threads_expr), Some(fill)) =
                (it.next(), it.next(), it.next(), it.next())
            else {
                continue;
            };
            let extra_args: Vec<Value> = it.collect();
            self.par_replay_body = Some((body, plan.count));
            let mut out = Value::Null;
            self.build_parallel_for_ir(
                &mut out,
                &plan.result_name,
                worker,
                &ret_type,
                elem_size,
                return_size,
                &vec_expr,
                threads_expr,
                fill,
                plan.loop_nr,
                extra_args,
                plan.elem_var,
                &elem_tp,
            );
            self.par_replay_body = None;
            built.push((id, out));
        }
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = saved_ctx;
        let mut code =
            std::mem::replace(&mut self.data.definitions[d_nr as usize].code, Value::Null);
        code.map_nodes(&mut |v| {
            if let Value::Call(d, args) = v
                && *d == marker
                && let Some(Value::Int(id)) = args.first().map(Value::unspan)
                && let Ok(id) = usize::try_from(*id)
                && let Some((_, lowered)) = built.iter().find(|(b, _)| *b == id)
            {
                *v = lowered.clone();
            }
        });
        self.data.definitions[d_nr as usize].code = code;
    }

    /// loft#1023 — re-derive every monomorph that was built from a template's PASS-1 body.
    ///
    /// The second pass re-parses the file in source order, so a call instantiates a generic
    /// declared BELOW it before that template's own pass-2 body has been produced.  The
    /// monomorph then keeps whatever pass 1 left, and anything a construct emits only on
    /// the second pass is missing from it.  Declaration order is not supposed to matter —
    /// the two-pass parser exists so a function may be used before it is written — so this
    /// runs where every template body finally exists and derives the bodies again.
    ///
    /// Re-deriving reaches further generics (a template body that calls another one), and
    /// those are stale for the same reason, so it drains to a fixpoint.  Bounded because a
    /// re-derived monomorph is never recorded again.
    /// `@FR-R-TailSelf` (@C155) — the warning the template could not decide: whether a frame
    /// can be reused depends on the type arguments (a text passed through only as itself, a
    /// record result delivered), so it is asked of EACH instance once its body is final — at
    /// instantiation, and again when a stale instance is rebuilt.  The self-call names the
    /// instance, or still the template when the nested calls have not been renamed yet; both
    /// are read.  The same predicate as `definitions.rs` and the pass.
    fn warn_instance_tail(&mut self, d_nr: u32, g_nr: u32, bindings: &[(u32, Type)]) {
        if self.first_pass || crate::tail_frame::off() {
            return;
        }
        let def = &self.data.definitions[d_nr as usize];
        let (d_self, calls) = {
            let own = crate::tail_frame::tail_calls(&def.code, d_nr, &def.variables, &self.data);
            if own.is_empty() {
                (
                    g_nr,
                    crate::tail_frame::tail_calls(&def.code, g_nr, &def.variables, &self.data),
                )
            } else {
                (d_nr, own)
            }
        };
        if std::env::var_os("LOFT_TRACE_TAIL_SELF").is_some() {
            eprintln!(
                "tail-rule: instance {} — {} tail call(s)",
                self.data.def(d_nr).name(),
                calls.len()
            );
        }
        let Some(call) = calls.into_iter().next() else {
            return;
        };
        let Err(why) = crate::tail_frame::admit_frame(
            &def.variables,
            def.returned(),
            &def.code,
            &self.data,
            &crate::tail_frame::declared_params(def),
            d_self,
        ) else {
            return;
        };
        let name = self
            .data
            .def(g_nr)
            .name()
            .trim_start_matches("n_")
            .to_string();
        let shown: Vec<String> = bindings
            .iter()
            .map(|(_, t)| t.source_name(&self.data))
            .collect();
        let msg = format!(
            "`{name}<{}>` calls itself in tail position, but this call stays an ordinary call, \
             which stops at 10 000 frames: {why}.  Write the recursion as a loop (`while`), or \
             keep that value outside the frame",
            shown.join(", ")
        );
        let level = crate::diagnostics::Level::Warning;
        match call.at {
            Some(pos) => {
                diagnostic_at!(
                    self.lexer,
                    &pos,
                    level,
                    code = "tail-call-not-jumped",
                    "{msg}"
                )
            }
            None => diagnostic!(self.lexer, level, code = "tail-call-not-jumped", "{msg}"),
        }
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: format!(
                "keep it an ordinary call on purpose: bind its value first, `r = {name}(…); r`"
            ),
            condition: Some(
                "the recursion stays well under 10 000 frames for every input it is given"
                    .to_string(),
            ),
            edit: None,
            concept: "functions",
            concept_ref: "@F16",
        });
    }

    pub(super) fn rebuild_stale_monomorphs(&mut self) {
        let mut rounds = 0;
        while !self.stale_monomorphs.is_empty() && rounds < 16 {
            rounds += 1;
            for (d_nr, g_nr, bindings) in std::mem::take(&mut self.stale_monomorphs) {
                let tmpl_code = self.data.definitions[g_nr as usize].code.clone();
                if tmpl_code == Value::Null {
                    continue;
                }
                let tmpl_vars = self.data.definitions[g_nr as usize].variables.clone();
                let mut code = tmpl_code;
                for (holder, bound_to) in &bindings {
                    let iter_stride = self.vector_elem_read(bound_to);
                    code = Self::substitute_type_in_value(
                        code,
                        *holder,
                        bound_to,
                        iter_stride,
                        &self.data,
                    );
                }
                self.fill_monomorph_body(d_nr, code, &tmpl_vars, &bindings);
                // The body is fresh, so every call it makes to ANOTHER generic has to be
                // retargeted at that one's monomorph again.
                self.instantiate_nested_generics(d_nr, &bindings);
                // loft#2179 — and the refilled body copies its parameter returns again: the
                // deps kept from the first instantiation still say fresh.
                self.copy_param_returns_of_fresh_instance(d_nr);
                // And the text-return promotion has to be asked again: the first
                // instantiation asked it of the pass-1 body, whose tail call still named
                // the nested TEMPLATE (`-> S?` — not text, so nothing to promote), and the
                // re-derived body is what the program runs.  Only where the first pass
                // promoted nothing — a second hidden buffer would move the ABI (loft#1357).
                if self.data.def(d_nr).text_work_buffers() == 0 {
                    if crate::env_once!(std::env::var_os("LOFT_DBG_ACC").is_some()) {
                        eprintln!(
                            "[acc-mono] re-derived stale monomorph {}",
                            self.data.def(d_nr).name()
                        );
                    }
                    self.promote_monomorph_text_return(d_nr);
                }
                self.warn_instance_tail(d_nr, g_nr, &bindings);
            }
        }
    }
}
