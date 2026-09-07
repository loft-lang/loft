// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent)

use super::{
    DefType, I32, Level, Parser, Parts, Type, Value, diagnostic_format, v_block, v_if, v_set,
};

// Field access, indexing, and iterator operations.

impl Parser {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn field(&mut self, code: &mut Value, tp: Type) -> Type {
        // @PLN102 — a field access can't be MORE non-null than its receiver: if the
        // receiver is nullable (`s: S? = null`, `w.inner` where `inner: Inner?`),
        // reading `s.field` yields the field type's null when the receiver is absent
        // (C80), so `s.field == null` is NOT "always false" and `s.field ?? d` is NOT
        // redundant.  `get_field` sets `expr_not_null` from the FIELD's own nullness;
        // clear it below when the receiver's TYPE is `Optional`.  (The receiver TYPE is
        // the right signal — `expr_not_null` alone is false even for a non-null
        // constructed struct, which would wrongly suppress genuine warnings, e.g.
        // p285.)
        let receiver_nullable =
            matches!(tp, Type::Optional(_)) || self.reads_a_collection_element(code);
        if let Type::Unknown(_) | Type::Never = tp {
            // @P376 — `Type::Never` is the poison an errored struct construction
            // (`p = Plyer { … }` with an unknown `Plyer`) assigns to its
            // variable.  Recover exactly like an unknown field access but
            // WITHOUT a diagnostic: the real `unknown type '…'` was already
            // reported at the construction, and re-reporting here is the
            // cascade #376 is about.  Returning `tp` (Never) keeps the poison
            // flowing so the enclosing format string / sweep stays silent too.
            if !self.first_pass && matches!(tp, Type::Unknown(_)) {
                diagnostic!(self.lexer, Level::Error, "Field of unknown variable");
            }
            // Skip the member token so parsing continues past an access whose
            // receiver has no type yet.  A member is spelled EITHER way — an
            // identifier for a named field, an integer for a tuple index — and
            // consuming only the identifier left `.0` in the stream, where the
            // statement parser tripped on it as "Expect token ;" (loft#868).
            // That error fires on pass 1, which aborts the run before pass 2 —
            // so the receiver's real problem was never reported, and a forward
            // reference to a tuple-returning function (legal, and Unknown on
            // pass 1 by design) could not be tuple-accessed at all.
            if self.lexer.has_identifier().is_none() {
                self.lexer.has_integer();
            }
            // @P281 — when the dot-access is followed by `(args)`
            // (i.e., `s.method(arg1, arg2)`), the parser must
            // consume the ENTIRE call expression so the surrounding
            // statement parser doesn't trip on the unconsumed `(`
            // and fire a spurious "Expect token ;".  Runs in BOTH
            // passes: pass-1 needs it so the body parses cleanly
            // and pass-2 can re-resolve; pass-2 needs it so the
            // already-emitted "Field of unknown variable" doesn't
            // cascade into "Expect token ;".  Each arg routes
            // through `expression` so nested calls / format strings
            // tokenise correctly.  It goes through the SHARED skipper rather than
            // a copy of its loop: an argument spelling this one had not been
            // taught about is invisible here (the receiver is already unknown, so
            // there is nothing to compare against), and a named argument was
            // exactly that — `r.render(dry: true)` reached this path whenever
            // `render` was declared BELOW its caller, and only then.
            if self.lexer.has_token("(") {
                self.skip_remaining_args();
            }
            // wrap `code` in Value::Drop so an unresolved field access
            // (e.g. `x.v` where x's type is not yet known on pass 1) is no
            // longer treated as a plain `Value::Var(x)` by downstream
            // assignment processing.  Without this wrapping, `x.v = 99` in
            // a function whose `x = callee()` references a struct-returning
            // fn defined LATER in the file collapses to `x = 99` on pass 1
            // (because `.v` is silently dropped) — which sets x's inferred
            // type to integer.  Pass 2 then sees x = integer and rejects
            // the now-resolved `x = callee()` returning the struct.
            // Wrapping in Drop keeps `code != Value::Var(x)` so
            // `assign_var_nr` returns u16::MAX and `change_var` skips the
            // type update.
            *code = Value::Drop(Box::new(code.clone()));
            return tp;
        }
        let mut t = tp;
        // @PLN115 S6 — capture the member name's position for the resolution index,
        // but only when recording: `Position` holds a `String`, so an unconditional
        // clone would tax every field access on a normal compile.
        let field_pos = self
            .record_resolutions
            .then(|| self.lexer.peek_pos().clone());
        let Some(field) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect a field name");
            return t;
        };
        let enr = self.data.type_elm(&t);
        if enr == u32::MAX {
            let shown = t.show(&self.data, &self.vars);
            if let Some(s) = self.suggest_type_name(&shown) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown type {shown} — did you mean '{s}'?"
                );
            } else {
                diagnostic!(self.lexer, Level::Error, "Unknown type {shown}");
            }
            return Type::Unknown(0);
        }
        let e_tp = i32::from(self.data.def(enr).known_type());
        if let Type::RefVar(tp) = t {
            t = *tp;
        }
        // @PLN25 E2 — a METHOD CALL or an S-level field access on a `__nullable<S>`
        // receiver (`v[i].method()` / `v[i].struct_method`).  Two cases reach here:
        //  - a trailing `(` is a method call — `find_poly_enum_field` matches the
        //    method's fn-ref entry copied into `Some` and would read it as a FIELD
        //    (→ "Field access not supported on type fn …");
        //  - no `Some`-variant field of that name — an S-level access.
        // A plain DATA field of `Some` (no `(`) resolves via `find_poly_enum_field`
        // below and is left untouched here.
        //
        // `@FR-L-Null-Which` — the receiver is read THROUGH ITS TAG (`read_through_tag`):
        // the base of a field read or a method call is not a slot, so the tagged value
        // becomes the pointer `S?` here — the payload's address when the discriminant says
        // present, `nullref` when it says absent — and the field/method dispatch below
        // proceeds on that pointer exactly as it does for a `S?` local.  Projecting the
        // payload's sub-ref without consulting the discriminant read an ABSENT element as
        // a record of zeroes: `v[i].n ?? -1` answered `0` where `x = v[i]; x.n ?? -1`
        // beside it answered `-1`, on both backends.
        if let Type::Enum(enum_d, true, _) = &t
            && self.data.def(*enum_d).name.starts_with("__nullable<")
            && (self.lexer.peek_token("(") || self.find_poly_enum_field(*enum_d, &field).is_none())
        {
            self.read_through_tag(code, &mut t);
        }
        let dnr = self.data.type_def_nr(&t);
        if matches!(t, Type::Vector(_, _)) && self.vector_operations(code, &field, e_tp, &t) {
            return Type::Boolean;
        }
        let fnr = self.data.attr(dnr, &field);
        // @PLN86 P6.4 (F4) — record a sandboxed READ of a host field that carries a
        // `#read` capability link, so admission can gate it.  Reads are default-allow,
        // so only a `#read`-linked field is ever recorded.  Second pass only (the base
        // type + attribute have resolved).  (A write LHS also reaches field(); a write
        // to a host field is independently rejected by 2.4, and a field is rarely both
        // read-linked and written — F5 reworks the write path.)
        if fnr != usize::MAX && self.in_sandbox && !self.first_pass {
            let reads: Vec<String> = self
                .member_access
                .get(&(dnr, field.clone()))
                .map(|links| {
                    links
                        .iter()
                        .filter(|t| t.rsplit_once('#').is_some_and(|(_, r)| r == "read"))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let read_count = reads.len();
            if !reads.is_empty() {
                let pos = self.lexer.peek_pos().clone();
                let entry = self.sandbox_field_reads.entry(self.context).or_default();
                for t in reads {
                    entry.push((t, pos.clone()));
                }
            }
            // @PLN86 F5 — remember this field access so a raw write at the assignment
            // site can resolve which field it targets (and un-record the read just
            // logged above, since a write LHS is not a read).
            self.last_field_target = Some((dnr, field.clone(), read_count));
        }
        // Trace point: field/method dispatch entry state.  Captures
        // what type and field name reached `field()`, whether the
        // attribute was found, and which pass we're on.  Recurring
        // vantage during method-dispatch debugging (plan-17 B).
        // Enable with `LOFT_TRACE=field`.
        crate::loft_trace!(
            field,
            "field={} dnr={} fnr={} t={:?} first_pass={}",
            field,
            dnr,
            if fnr == usize::MAX {
                "MAX".to_string()
            } else {
                fnr.to_string()
            },
            t,
            self.first_pass,
        );
        if fnr == usize::MAX {
            // Plan-17 phase 01 (B) — bounded-T method dispatch must run
            // on BOTH passes so the call's return type propagates into
            // first-pass type inference of the enclosing variable.
            //
            // ⚠ A first-pass branch that consumes the `(...)` args and returns
            // `Type::Unknown(0)` without dispatching to the t-stub does not
            // merely defer the answer — it fixes it.  `change_var_type` is a
            // no-op when assigning Unknown, so the variable's Unknown type
            // SURVIVES second pass: `s = x.to_text()` (`x: T`, `T: Printable`)
            // stays Unknown for good, and every downstream operator on `s` is
            // then rejected — `s + "!"` with "No matching operator '+' on
            // 'unknown(0)' and 'text'".
            //
            // The t-stub `t_<n>T_<method>` is registered when the
            // function's bounds are declared (definitions.rs:670+).  By
            // the time the body parser reaches `x.to_text()` on first
            // pass, `find_fn(u16::MAX, "to_text", Reference(tv_nr, []))`
            // finds the stub.  Dispatching to it on first pass returns
            // the bound's declared return type (`Type::Text(_)` for
            // Printable's `to_text`), so the surrounding assignment
            // types `s` correctly.
            //
            // Pinned by `tests/issues.rs::plan17_b_bounded_method_return_type_propagates`.
            // Likely also closes plan-17 (A) caveat (implicit
            // generic-tuple type inference) — same root cause.
            // Plan-17 phase 01 (B) — bounded-T method dispatch must run
            // on BOTH passes so the call's return type propagates into
            // first-pass type inference of the enclosing variable.  On
            // second pass the t-stub `t_<n>T_<method>` exists (created
            // in `definitions.rs:670+`); on first pass it doesn't.
            // Both branches end up at `parse_method(stub_nr, t)`; the
            // first-pass branch creates the stub on demand if missing.
            //
            // @PLN125 A2c — the receiver may equally be an interface's ASSOCIATED type
            // (`r = s.open(); r.width()`, where `open` is declared `-> Self.Rows`). It is
            // the same shape: a name with declared bounds and no definition yet, so the
            // bounds are what authorise the call — the HOLDER's bounds, which for an
            // associated type are its own `type Rows: Cursor` and not the generic's.
            if self.generic_type_name(&t).is_some() {
                // `generic_type_name` answers `Some` only for a `Reference`, so this is
                // the same definition it just named.
                let holder_nr = match &t {
                    Type::Reference(d, _) => *d,
                    _ => u32::MAX,
                };
                let stub_nr = self.data.find_fn(u16::MAX, &field, &t);
                if stub_nr != u32::MAX
                    && self.has_bound_for_method(&field, holder_nr, None)
                    && self.lexer.has_token("(")
                {
                    return self.parse_method(code, stub_nr, t.clone());
                }
            }
            // P54 Q3 second half — `instance.to_json()` /
            // `instance.to_json_pretty()` on any user struct (or
            // struct-enum variant) lowers to a single call to
            // `n_struct_to_json(self, struct_kt)` via the schema
            // walker in `Stores::show_json` (`src/database/format.rs`).
            // No per-type stub registration needed; the parser
            // synthesises the call when the receiver is a
            // `Type::Reference(struct_d, _)` and the method name
            // matches.  Mirror of `parse_type_parse` (P54 step 5)
            // for the static `T.parse(JsonValue)` form.
            //
            // Runs on BOTH passes so first-pass type inference of the
            // enclosing variable sees the `Type::Text` return type
            // (same reason the bounded-T fallback above does the
            // same).  On first pass we consume the `()` to make
            // parser progress; the Call population only happens on
            // second pass when known_type / def_nr are stable.
            if (field == "to_json" || field == "to_json_pretty")
                && matches!(t, Type::Reference(_, _))
                && self.lexer.peek_token("(")
            {
                self.lexer.token("(");
                self.lexer.token(")");
                if !self.first_pass {
                    let Type::Reference(struct_d, _) = &t else {
                        unreachable!("matches! above guards Reference shape");
                    };
                    let known_tp = self.data.def(*struct_d).known_type();
                    let n_walker = if field == "to_json" {
                        self.data.def_nr("n_struct_to_json")
                    } else {
                        self.data.def_nr("n_struct_to_json_pretty")
                    };
                    if known_tp != u16::MAX && n_walker != u32::MAX {
                        *code = Value::Call(
                            n_walker,
                            vec![code.clone(), Value::Int(i32::from(known_tp))],
                        );
                    }
                }
                return Type::Text(crate::data::Deps::none());
            }
            // Plan-19 phase 03 — method-on-parent-enum dispatch.  When
            // the receiver is a variant value (`Type::Reference(child_d, …)`
            // for a struct enum variant) and the method is declared on
            // the parent enum (e.g. `fn classify(self: Shape)`), look
            // up `t_<n>Shape_classify` on the parent and dispatch
            // there.  Without this fallback, `s.classify()` where
            // `s = Circle { … }` (inferred type `Reference(Circle)`)
            // rejected with "Unknown field Circle.classify" — even
            // though `s: Shape = Circle { … }` followed by
            // `s.classify()` worked.
            //
            // Runs on both passes so the call's return type propagates
            // into first-pass inference of the enclosing variable
            // (same reason plan-17 (B) bounded-T dispatch runs on both
            // passes).
            //
            // Pinned by `tests/issues.rs::plan19_method_on_enum_variant_via_dot`.
            if let Type::Reference(child_d, _) = &t {
                let parent_d = self.data.def(*child_d).parent();
                if parent_d != u32::MAX && matches!(self.data.def_type(parent_d), DefType::Enum) {
                    let parent_name = self.data.def(parent_d).name().to_string();
                    let stub_name = format!("t_{}{}_{}", parent_name.len(), parent_name, field);
                    let md_nr = self.data.def_nr(&stub_name);
                    // Only fire when `t_<Parent>_<field>` is the
                    // user's direct declaration on the enum, NOT the
                    // auto-generated polymorphic dispatcher built
                    // from per-variant impls (which carries
                    // `synthetic = Some("enum_dispatcher")` —
                    // see `Definition.synthetic` doc).  Without
                    // this guard, `r.area()` on a variant lacking
                    // its own impl would bypass the long-standing
                    // "Unknown field Rect.area" error and silently
                    // dispatch through the warning-only stub.
                    if md_nr != u32::MAX
                        && self.data.def(md_nr).synthetic().is_none()
                        && matches!(
                            self.data.def_type(md_nr),
                            DefType::Function | DefType::Generic
                        )
                        && self.lexer.has_token("(")
                    {
                        return self.parse_method(code, md_nr, t.clone());
                    }
                }
            }
            // map/filter/reduce as method syntax on vectors: `v.map(fn)` → `map(v, fn)`.
            // Unwrap `&vector<T>` so they work on ref params.
            //
            // loft#945 — on BOTH passes.  Pass 1 used to fall through to
            // `skip_remaining_args` below, so the callback lambda was parsed with no
            // element-type hint on pass 1 and with one on pass 2 — the two passes
            // disagreed about the lambda's own signature, which is how `xs.map(…)` and
            // `map(xs, …)` came to lower differently for the same program.  `parse_map`
            // and friends already have their own pass-1 arms (type only, no variables
            // minted), which is what makes running them here safe.
            let vec_recv = if let Type::RefVar(inner) = &t {
                inner.as_ref().clone()
            } else {
                t.clone()
            };
            if matches!(vec_recv, Type::Vector(_, _))
                && matches!(field.as_str(), "map" | "filter" | "reduce")
                && self.lexer.has_token("(")
            {
                return self.parse_vector_method(code, &vec_recv, &field);
            }
            if self.first_pass && self.lexer.has_token("(") {
                self.skip_remaining_args();
            } else if let Type::Enum(enum_d_nr, true, _) = t.base()
                && let Some((found_d_nr, found_fnr)) = self.find_poly_enum_field(*enum_d_nr, &field)
            {
                // Through `base()`: `Sh?` is `Optional(Enum(Sh, true))`, the same record behind
                // a nullability marker (`@FR-L-Null`), and `type_elm` above already peels it
                // for the struct path.  Asked bare, `e.n` on an `e: Sh?` fell through — pass 1
                // returned the receiver's own type, so a WRITE re-typed `e` ("cannot change
                // type from Sh? to integer") and a READ reported "Unknown field Sh.n" — while
                // `s.v` on an `s: S?` resolved.  The receiver's nullability reaches the read
                // the way it does for a struct: `receiver_nullable` clears `expr_not_null`.
                // For polymorphic enums (incl. @PLN25 `__nullable<S>`), this field
                // lives in a VARIANT struct, not the enum itself.  Resolve in BOTH
                // passes: the first pass needs the field TYPE so the receiver var
                // is not left as `Var(receiver)` and then re-typed to the field
                // type by `change_var` (the bug behind `for o in v { f(o.items) }`
                // → "o cannot change type to vector<…>").  `get_field` (which needs
                // the layout's `known_type`, assigned in `fill_all`) emits only in
                // the second pass.
                let enum_d = *enum_d_nr;
                let dep = t.depend();
                t = self.data.attr_type(found_d_nr, found_fnr);
                for on in dep {
                    t = t.depending(on);
                }
                if let Value::Var(nr) = code {
                    t = t.depending(*nr);
                }
                if self.first_pass {
                    // Type-only: leave a non-Var placeholder so the caller's
                    // `change_var` does not re-type the receiver.
                    *code = Value::Null;
                } else {
                    let recv = self.guard_variant_receiver(enum_d, &field, &t, code.clone());
                    *code = self.get_field(found_d_nr, found_fnr, recv);
                    self.data.attr_used(found_d_nr, found_fnr);
                }
                return t;
            } else if !self.first_pass {
                // generic-specific error for field access on T.
                if let Some(tv_name) = self.generic_type_name(&t) {
                    let tv_name = crate::data::Data::type_var_spelling(tv_name);
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "generic type {tv_name}: field access requires a concrete type",
                    );
                } else {
                    // INC#8 / QUALITY 6c: if a free function `n_<field>` exists
                    // whose first parameter is compatible with the receiver
                    // type, tell the user to call it as a free function
                    // instead of as a method.  The stdlib chooses per
                    // function whether it's `self:` / `both:` / free-only;
                    // readers who don't know that land on "Unknown field
                    // vector.sum_of" without a hint.
                    let free_nr = self.data.def_nr(&format!("n_{field}"));
                    let has_free_hint = free_nr != u32::MAX
                        && !self.data.def(free_nr).attributes().is_empty()
                        && self.data.attr_type(free_nr, 0).is_equal(&t);
                    if has_free_hint {
                        // loft#850 — only the stdlib can be blamed for the stdlib's
                        // choices; a `use`d package that declares `{field}` free-only
                        // is a different file to go and read.
                        let declared_by =
                            if self.data.def(free_nr).source == crate::data::STD_SOURCE {
                                format!("stdlib declared `{field}` as free-only")
                            } else {
                                format!("`{field}` is declared as a free function, not as a method")
                            };
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown field {}.{field} — did you mean the free function `{field}(…)` ? ({declared_by}; see LOFT.md § Methods and function calls)",
                            self.data.def(dnr).name()
                        );
                    } else if let Some(s) = self.suggest_field_name(dnr, &field) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            code = "unknown-field",
                            "Unknown field {}.{field} — did you mean '{s}'?",
                            self.data.def(dnr).name()
                        );
                        self.lexer.suggest_last(&s);
                        // This site reports at the lexer CURSOR, which sits one past the
                        // consumed name — so the name starts `len + 1` back. Measured on
                        // two shapes rather than reasoned: `p.nme}` and `s.starts_wit(`
                        // both land on the same offset, and verification would refuse the
                        // rewrite outright if it did not.
                        let (line, col) = self.lexer.at();
                        let width = u32::try_from(field.len()).unwrap_or(0);
                        self.lexer.fix_last(crate::diagnostics::Fix {
                            kind: crate::diagnostics::FixKind::Mechanical,
                            title: format!("rename to `{s}`"),
                            condition: None,
                            edit: Some(crate::diagnostics::Edit {
                                line,
                                col: col.saturating_sub(width + 1).max(1),
                                len: width,
                                text: s.clone(),
                            }),
                            concept: "struct fields",
                            concept_ref: "@F12",
                        });
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown field {}.{field}",
                            self.data.def(dnr).name()
                        );
                    }
                }
                // Consume a trailing `(…)` to avoid cascading parse errors.
                if self.lexer.has_token("(") {
                    self.skip_remaining_args();
                }
            }
            return Type::Unknown(0);
        }
        // @PLN115 S6 — record the resolved member reference at the member name: a
        // Routine attribute is a METHOD, any other attribute a FIELD.  So `text.len`
        // records Method{text, len} and `p.x` records Field{P, x} — a same-spelled
        // member of another type, or a local of the same name, is thereby excluded.
        if !self.first_pass
            && let Some(fp) = &field_pos
        {
            let len = field.chars().count() as u16;
            match self.data.attr_type(dnr, fnr) {
                Type::Routine(r_nr) => self.record(
                    fp,
                    len,
                    crate::resolution::Resolution::Method {
                        recv_type: dnr,
                        method_def: r_nr,
                    },
                ),
                _ => self.record(
                    fp,
                    len,
                    crate::resolution::Resolution::Field {
                        type_def: dnr,
                        attr: fnr as u16,
                    },
                ),
            }
        }
        if let Type::Routine(r_nr) = self.data.attr_type(dnr, fnr) {
            if self.lexer.has_token("(") {
                t = self.parse_method(code, r_nr, t.clone());
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect call of method {}.{}",
                    self.data.def(dnr).name(),
                    self.data.attr_name(dnr, fnr)
                );
            }
        } else if self.data.def(dnr).attributes()[fnr].constant {
            let expr = self.data.attr_value(dnr, fnr);
            // B2-runtime (2026-04-13): `Sig.Idle` on a mixed struct-enum
            // parent resolves `expr` to a bare `Value::Enum(disc, _)` —
            // same type mismatch as the unqualified `Idle` form.  Wrap in
            // the `OpDatabase` + `object_init` record-allocation sequence
            // used by `parse_constant_value` so `var_s: DbRef = …` gets a
            // proper DbRef, not a u8 byte.
            let parent_is_mixed = matches!(self.data.def(dnr).returned(), Type::Enum(_, true, _));
            if parent_is_mixed && !self.first_pass && matches!(expr, Value::Enum(_, _)) {
                let variant_name = self.data.attr_name(dnr, fnr);
                let variant_d_nr = self.data.def_nr(&variant_name);
                if variant_d_nr != u32::MAX && self.data.def(variant_d_nr).known_type() != u16::MAX
                {
                    let ret = self.data.def(dnr).returned().clone();
                    let w = self.vars.work_refs(&ret, &mut self.lexer);
                    let known_type = i32::from(self.data.def(variant_d_nr).known_type());
                    let mut list = Vec::new();
                    list.push(crate::data::v_set(w, Value::Null));
                    list.push(self.cl("OpDatabase", &[Value::Var(w), Value::Int(known_type)]));
                    self.object_init(
                        &mut list,
                        variant_d_nr,
                        0,
                        &Value::Var(w),
                        &std::collections::HashSet::new(),
                        &std::collections::HashSet::new(),
                    );
                    list.push(Value::Var(w));
                    // `w` is a NORMAL work-ref, freed at scope end — the same discipline
                    // the unqualified form settled in #394, and the reason this path may
                    // not mark it `skip_free`.  Only the ALIAS consumers (assignment,
                    // return) take `w`'s store over, and the ownership-transfer logic
                    // already claims it for them.  Every DEEP-COPY consumer — a vector
                    // element, a struct field, a call argument, a nested vector — copies
                    // the record and orphans `w`, so a `skip_free` here left nothing to
                    // free it (loft#1344: `xs += [V.Null]` leaked while `xs += [Null]`,
                    // the very same value spelled without its enum, did not).
                    *code = crate::data::v_block(
                        list,
                        Type::Enum(dnr, true, crate::data::Deps::none()),
                        "EnumUnitLit",
                    );
                    self.data.attr_used(dnr, fnr);
                    return Type::Enum(dnr, true, crate::data::Deps::none());
                }
            }
            *code = Self::replace_record_ref(expr, &code.clone());
            let dep = t.depend();
            t = self.data.attr_type(dnr, fnr);
            for on in dep {
                t = t.depending(on);
            }
        } else {
            let dep = t.depend();
            t = self.data.attr_type(dnr, fnr);
            for on in dep {
                t = t.depending(on);
            }
            if let Value::Var(nr) = code {
                t = t.depending(*nr);
            }
            *code = self.get_field(dnr, fnr, code.clone());
            // Plan-07 phase 4h — count this read for the
            // `not null` field-reminder hint.  Skip stdlib (the
            // suggestion target is user code), skip first pass
            // (counts must be stable; first pass isn't), skip
            // already-`not null` fields (no recommendation
            // possible), skip method-routine reads (constant
            // branch above handled those), and skip when fnr is
            // out of range (defensive — shouldn't happen here).
            if !self.first_pass
                && !self.default
                && fnr != usize::MAX
                && fnr < self.data.def(dnr).attributes().len()
                && self.data.def(dnr).attributes()[fnr].nullable
            {
                let key = (dnr, fnr as u32);
                *self.field_read_counts.entry(key).or_insert(0) += 1;
                // Record this site so `handle_null_coalesce` can
                // mark it defended when `??` follows immediately
                // (`p.field ?? default`).
                self.last_field_read_site = Some(key);
            }
        }
        // A field of a NULLABLE receiver can itself be null (C80), regardless of the
        // field's declared non-nullness — so it is not "not null" for the redundant-
        // check / redundant-coalesce lints.  (Only the lint signal is cleared; the
        // returned type `t` is unchanged — widening it to `Optional` would force
        // `?? d` on every nullable-receiver field read, a far broader change.)
        if receiver_nullable && self.expr_not_null {
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
        }
        self.data.attr_used(dnr, fnr);
        t
    }

    /// Consume remaining function call arguments after `(` has already been consumed.
    /// Handle `v.map(fn)` / `v.filter(fn)` / `v.reduce(fn)` method syntax.
    fn parse_vector_method(&mut self, code: &mut Value, t: &Type, method: &str) -> Type {
        let mut list = vec![code.clone()];
        let mut types = vec![t.clone()];
        let mut m_arg_idx = 1usize;
        loop {
            if let Type::Vector(elm, _) = t {
                let elem = *elm.clone();
                let hint = match (method, m_arg_idx) {
                    // loft#945 — `map` is `fn(T) -> U`, so only the PARAMETER is the
                    // element type; the return is free.  Pinning it to `elem` type-checked
                    // the lambda's body against `T`, which is why every `U != T` was
                    // refused inside the user's own lambda ("expected integer, got text").
                    // `Unknown` leaves the return to `parse_lambda_short`'s body inference.
                    // `filter`/`reduce` keep their returns: a predicate really is `-> bool`,
                    // and a fold really answers its accumulator's type.
                    ("map", 1) => Some(Type::Function(
                        vec![elem],
                        Box::new(Type::Unknown(0)),
                        crate::data::Deps::none(),
                    )),
                    ("filter", 1) => Some(Type::Function(
                        vec![elem],
                        Box::new(Type::Boolean),
                        crate::data::Deps::none(),
                    )),
                    // @P288 — `v.reduce(init, |acc, x| {…})`: the lambda is ARG 2 (init is
                    // arg 1), so the hint goes on m_arg_idx == 2.
                    //
                    // The declared signature is `fn(U, T) -> U`, so the ACCUMULATOR is the
                    // INIT's type and only the second parameter is the element's.  This
                    // used to hint `elem` for both, on the reasoning that every primitive
                    // case (sum, max, min, count) keeps acc and elm in one numeric domain
                    // — true, and it makes `U != T` unusable in the method form: on a
                    // `vector<(integer, integer)>` with an `integer` init, `acc` was typed
                    // as the TUPLE and `acc + t.0` was refused with "No matching operator
                    // '+' on '(integer, integer)' and 'integer'" (loft#1074).
                    //
                    // Reading the init's own type costs nothing where the two agree — the
                    // homogeneous cases hint exactly what they hinted before — and falls
                    // back to `elem` when the init has not typed (an earlier parse error),
                    // so a broken program still gets the old hint rather than `Unknown`.
                    ("reduce", 2) => {
                        let acc = types
                            .get(1)
                            .filter(|it| !matches!(it, Type::Unknown(_)))
                            .cloned()
                            .unwrap_or_else(|| elem.clone());
                        Some(Type::Function(
                            vec![acc.clone(), elem],
                            Box::new(acc),
                            crate::data::Deps::none(),
                        ))
                    }
                    _ => None,
                };
                if let Some(h) = hint {
                    self.expected = h;
                }
            }
            let mut p = Value::Null;
            let pt = self.expression(&mut p);
            self.expected = Type::Unknown(0);
            list.push(p);
            types.push(pt);
            m_arg_idx += 1;
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        match method {
            "map" => self.parse_map(code, &list, &types),
            "filter" => self.parse_filter(code, &list, &types),
            "reduce" => self.parse_reduce(code, &list, &types),
            _ => unreachable!(),
        }
    }

    /// Consume the argument list of a call this pass cannot resolve, leaving the
    /// cursor after the `)`.
    ///
    /// Both callers reach it with the callee unknown — the first pass, before the
    /// method it names has been read, and the error path for a member that does not
    /// exist — so nothing here can look a parameter up. It still has to accept every
    /// argument spelling the language has, and a NAMED argument (`name: value`) is
    /// one of them: `name` is not an expression, so parsing it as one stopped at the
    /// `:` and reported `Expect token )`.
    ///
    /// That made a legal call depend on where its method was DECLARED — `r.render(dry:
    /// true)` compiled with `render` above the caller and failed with it below —
    /// and it swallowed the one message worth reading, turning `s.nosuch(width: 3)`
    /// into five cascading errors with no `Unknown field` among them.
    pub(crate) fn skip_remaining_args(&mut self) {
        loop {
            if self.lexer.peek_token(")") {
                break;
            }
            if self.lexer.peek_named_arg().is_some() {
                self.lexer.has_identifier();
                self.lexer.has_token(":");
            }
            let mut p = Value::Null;
            self.expression(&mut p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
    }

    /// loft#980 — a struct-enum field access that only SOME variants answer.
    ///
    /// `c.field` resolves at COMPILE time to the first variant declaring the name, and
    /// the layout gives a shared name+type one slot — so the read is right for the
    /// variants that declare it and reads another variant's bytes for the rest. The tag
    /// is never consulted, on either backend, and nothing said so: `a.n` on an `Anon`
    /// answered `Anon.k`'s value as if it were `Named.n`, and `a.label = "x"` wrote into
    /// a record whose tag still says `Anon` — after which `match` still reports `Anon`.
    ///
    /// Direct access STAYS. It is what [C89](../../doc/claude/DESIGN_DECISIONS.md)
    /// permanently decided enum payloads are for — named fields you read straight,
    /// with matching for *dispatch*, never for *extraction* — and the common-prefix case
    /// (every variant declares it) is correct today and stays silent here. The silence
    /// on the PARTIAL case was the defect.
    ///
    /// `warning`, not advice, by the tier rule: ignoring it can produce a wrong result,
    /// and it already has — the value read is another variant's, typed as this one's.
    /// Quiet for a synthetic `__nullable<S>`, whose payload access is @PLN25's null
    /// model rather than a user-visible variant question.
    fn warn_unchecked_variant_field(
        &mut self,
        enum_d: u32,
        field: &str,
        owning: &[u32],
        total: usize,
        recv: &Value,
        guarded: bool,
    ) {
        if self.first_pass || crate::keys::no_variant_field_warning() {
            return;
        }
        let mut names: Vec<String> = owning
            .iter()
            .map(|&v| self.data.def(v).original_name().clone())
            .collect();
        names.sort();
        let have = names.join("`, `");
        let display = self.data.def(enum_d).original_name().clone();
        let subject = match recv.unspan() {
            Value::Var(v) if *v < self.vars.count() => self.vars.name(*v).to_string(),
            _ => "the value".to_string(),
        };
        let first = names[0].clone();
        // Two different facts, and the message must not claim the other one's. A PLACE
        // receiver is tag-guarded, so the miss is answerable — null, and the write ignored.
        // A receiver that is a call is NOT guarded (the guard reads it twice), so it keeps
        // the unchecked access, and the cure it needs is to bind it first.
        let effect = if guarded {
            "on any other one the read answers null and a write to it is IGNORED".to_string()
        } else {
            format!(
                "and it is not a place, so the tag cannot be read without evaluating it \
                 twice — on any other variant this reads THAT variant's bytes at \
                 `{field}`'s offset, and a write lands there leaving the tag alone. Bind \
                 it to a local first"
            )
        };
        diagnostic!(
            self.lexer,
            Level::Warning,
            code = "variant-field-unchecked",
            "only `{have}` of `{display}`'s {total} variants declare `{field}`, and this \
access does not check which variant `{subject}` holds — {effect}. \
Reach it per-variant: `if {subject} is {first} {{ {field} }} {{ … }}`, or `match`"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: format!("bind `{field}` inside `if {subject} is {first} {{ {field} }}`"),
            condition: Some(format!(
                "if `{subject}` can only ever be `{first}` here, the read is already right \
                 — say so with the pattern and the compiler checks it for you"
            )),
            edit: None,
            concept: "pattern matching",
            concept_ref: "@F29",
        });
    }

    /// loft#980 — make a PARTIAL struct-enum field access answer for the variant the
    /// value actually holds.
    ///
    /// `c.field` resolves at COMPILE time to the first variant declaring the name and then
    /// reads that offset whatever the tag says, so `a.n` on an `Anon` answered `Anon.k`'s
    /// bytes as `Named.n`, and `a.label = "x"` wrote into a record whose tag still said
    /// `Anon`. The tag was never consulted.
    ///
    /// The guard goes on the RECEIVER, not the access:
    /// `if tag(c) ∈ declaring { c } else { null }`. A null receiver ALREADY reads as null
    /// and ALREADY swallows a write, on both backends — so the read answers the type's
    /// sentinel (the same answer C80 gives a hash miss or an out-of-range index) and the
    /// write is suppressed, with no new opcode. And because only the receiver changed, the
    /// access is still a PLACE: the assignment path needs no lvalue notion for a guarded
    /// read, which is what made the write half unbuildable when the guard wrapped the
    /// access instead.
    ///
    /// Returns the receiver UNCHANGED — no tag read, no cost — when the question does not
    /// arise:
    /// * every variant declares the field (the common-prefix case C89 promises, correct
    ///   today because the layout gives a shared name+type one slot);
    /// * the enum is a synthetic `__nullable<S>`, whose payload access is @PLN25's null
    ///   model rather than a user-visible variant question (guarding it would make
    ///   `v[i].field` answer null);
    /// * the receiver is not a PLACE READ. The guard reads it twice — once for the tag,
    ///   once as the value — which is what a struct-enum `match` does with its subject and
    ///   is safe only for an expression that allocates nothing and calls nothing. A
    ///   receiver that is a CALL keeps today's unchecked access, and the warning that names
    ///   it, rather than being evaluated twice.
    fn guard_variant_receiver(
        &mut self,
        enum_d: u32,
        field: &str,
        tp: &Type,
        recv: Value,
    ) -> Value {
        if self.data.def(enum_d).name.starts_with("__nullable<") {
            return recv;
        }
        let (owning, total) = self.variants_declaring_field(enum_d, field, tp);
        if owning.is_empty() || owning.len() >= total {
            return recv;
        }
        // ONE derivation decides both the guard and what the diagnostic says: a message
        // describing behaviour the compiler no longer has is worse than none.
        let guarded = recv.is_place_read(&self.data);
        self.warn_unchecked_variant_field(enum_d, field, &owning, total, &recv, guarded);
        if !guarded {
            return recv;
        }
        let mut discs: Vec<i32> = owning
            .iter()
            .map(|&v| self.variant_disc(enum_d, true, v, ""))
            .collect();
        discs.sort_unstable();
        discs.dedup();
        // A disc of 0 is `variant_disc`'s "could not resolve" answer, not a real struct-enum
        // tag (they are +1-biased). Guarding on it would test against a tag no value carries
        // and turn every access into null, so leave the access unchecked instead.
        if discs.contains(&0) {
            return recv;
        }
        let tag = self.elem_tag_int(recv.clone());
        let mut cond = self.cl("OpEqInt", &[tag.clone(), Value::Int(discs[0])]);
        for &d in &discs[1..] {
            let next = self.cl("OpEqInt", &[tag.clone(), Value::Int(d)]);
            cond = v_if(cond, Value::Boolean(true), next);
        }
        let absent = self.cl("OpNullRefSentinel", &[]);
        v_if(cond, recv, absent)
    }

    /// The variants of `enum_d_nr` that declare `field` at the same TYPE as
    /// `(found_d_nr, found_fnr)`, and how many variants the enum has in total.
    ///
    /// A struct-enum field access resolves at compile time to the first variant
    /// declaring the name, and the layout puts a shared name+type at a shared offset —
    /// so the read is right for exactly these variants and reads another variant's
    /// bytes for the rest (loft#980).  Equal counts mean every variant has it, which is
    /// the common-prefix case C89 promises works and needs no tag check at all.
    pub(crate) fn variants_declaring_field(
        &self,
        enum_d_nr: u32,
        field: &str,
        tp: &Type,
    ) -> (Vec<u32>, usize) {
        let mut owning = Vec::new();
        let mut total = 0usize;
        for a_nr in 0..self.data.attributes(enum_d_nr) {
            let a_name = self.data.attr_name(enum_d_nr, a_nr);
            let variant_d_nr = self.data.variant_of(enum_d_nr, &a_name);
            if variant_d_nr == u32::MAX {
                continue;
            }
            total += 1;
            let f = self.data.attr(variant_d_nr, field);
            // Compared WITHOUT deps: two variants declaring the same field differ in the
            // borrow their access records, which says nothing about whether the layout
            // gave them one slot — the question here.
            if f != usize::MAX
                && self
                    .data
                    .attr_type(variant_d_nr, f)
                    .unrewritten()
                    .without_deps()
                    == tp.unrewritten().without_deps()
            {
                owning.push(variant_d_nr);
            }
        }
        (owning, total)
    }

    /// Search for `field` in the variant structs of a polymorphic enum.
    /// Returns `(variant_d_nr, attr_nr)` if found.
    pub(crate) fn find_poly_enum_field(&self, enum_d_nr: u32, field: &str) -> Option<(u32, usize)> {
        for a_nr in 0..self.data.attributes(enum_d_nr) {
            let a_name = self.data.attr_name(enum_d_nr, a_nr);
            // @PLN22 Phase 1 — resolve the variant within its enum via the
            // variant_of chokepoint, not the bare global def_nr.
            let variant_d_nr = self.data.variant_of(enum_d_nr, &a_name);
            if variant_d_nr == u32::MAX {
                continue;
            }
            let f = self.data.attr(variant_d_nr, field);
            if f != usize::MAX {
                return Some((variant_d_nr, f));
            }
        }
        None
    }

    /// `v.remove(i)` — answers whether `i` named an element.  On the vector member of a
    /// linked group the element leaves every keyed sibling first (`group_elem_write`), so the
    /// removal means the same thing whichever member it is spelled through.
    pub(crate) fn vector_operations(
        &mut self,
        code: &mut Value,
        field: &str,
        e_tp: i32,
        vec_tp: &Type,
    ) -> bool {
        if field == "remove" {
            // @FR-Col-RemoveDense — the successors slide down by ONE element, and the slide
            // is measured in the ELEMENT's width (@FR-H-Stride).  This is where that width is decided: the
            // element db type the vector's STORAGE was built with, not the one the DEF names.  Six integer widths share the one `integer` def, so a
            // def carries no width: `remove_vector_at` took the element's stride from it
            // and slid the tail eight bytes per element whatever the declaration said, so
            // a `vector<u8>` read past its live data while `len` stayed right (loft#1412).
            // `vector_element_type` is the one home the storage is registered through
            // (`narrow_vector_content`), so the removal and the layout cannot disagree.
            let e_tp = match vec_tp {
                Type::Vector(content, _) => self
                    .data
                    .vector_element_type(content, &mut self.database)
                    .map_or(e_tp, i32::from),
                _ => e_tp,
            };
            self.lexer.token("(");
            let (tps, ls) = self.parse_parameters();
            let mut cd = ls[0].clone();
            // validate types
            if tps.len() != 1 || !self.convert(&mut cd, &tps[0], &I32) {
                diagnostic!(self.lexer, Level::Error, "Invalid index in remove");
            }
            let elem = self.cl("OpVectorRef", &[code.clone(), cd.clone()]);
            // Typed as the element PLACE resolves — deps included — so the temporary that
            // binds it is a borrow of the vector on both backends; without the deps the
            // native emitter reads the bind as owning and deep-copies the record, and the
            // unlinks then run on the copy (@FR-B-Copy, @FR-O-NoDiverge).
            let mut elem_tp = self.index_type(vec_tp);
            for on in vec_tp.depend() {
                elem_tp = elem_tp.depending(on);
            }
            let coll = code.clone();
            if let Some(ops) = self.group_elem_write(&elem, &elem_tp, false, |p, _, place| {
                let idx = match place.unspan() {
                    Value::Call(_, a) => a.last().cloned().unwrap_or(Value::Null),
                    _ => Value::Null,
                };
                p.cl("OpRemoveVector", &[coll.clone(), Value::Int(e_tp), idx])
            }) {
                *code = v_block(ops, Type::Boolean, "group_elem_remove");
            } else {
                *code = self.cl("OpRemoveVector", &[code.clone(), Value::Int(e_tp), cd]);
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn parse_index(&mut self, code: &mut Value, tp: &Type) -> Type {
        let mut t = tp.clone();
        let mut p = Value::Null;
        self.un_ref(&mut t, &mut p);
        // @PLN25 — index/slice dispatch peels an `Optional` receiver (`s: text?;
        // s[i]`, `s[a..b]`) to its base, mirroring method dispatch which already
        // peels (`s.starts_with(..)` works on a `text?`). Inert gate-OFF: no
        // `Optional` is ever constructed, so `.base()` is a no-op. A null-check
        // discharges the value; indexing the null sentinel behaves as gate-OFF.
        t = t.base().clone();
        let mut elm_type = self.index_type(&t);
        for on in t.depend() {
            elm_type = elm_type.depending(on);
        }
        /*let nr = if self.types.exists("$") {
            self.types.var_nr("$")
        } else {
            self.create_var("$".to_string(), elm_type.clone())
        };
        self.data.definitions[self.context as usize].variables[nr as usize].uses = 0;
         */
        if let Type::Vector(etp, _) = &t {
            // loft#889 — the same naming the keyed arms below do.  A vector read out of
            // an inline call's FIELD (`make_bag().rows[i]`) views into the bag's store,
            // and the bag has no name until one is bound here.
            let dep = self.container_dep(code, &t);
            if let Some(cv) = dep {
                elm_type = elm_type.depending(cv);
            }
            let iter_value = self.parse_vector_index(code, &elm_type, etp);
            // A `v[i]` read is nullable — an out-of-bounds index yields the
            // null sentinel (exactly what the OOB defensive-check warns
            // about).  Clear `expr_not_null` (mirroring the keyed-collection
            // arms below, @P285) so `v[i] ?? default` is NOT flagged
            // "Redundant null coalescing" even when the vector's element
            // type — or the vector field itself — is `not null`.  Without
            // this, indexing a `not null` vector hit two contradictory
            // checks: bare `v[i]` warned OOB while `v[i] ?? d` warned
            // redundant, leaving no clean idiom.  Covers BOTH the scalar
            // path (`parse_vector_index` returns `None`, having mutated
            // `code`) and the iterator path (returns `Some`).
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
            if let Some(value) = iter_value {
                return value;
            }
            // @PLN25 DN3 (index): a scalar `v[i]` read is nullable (OOB → the null sentinel) unless
            // the index is provably in-bounds (`parse_vector_index` set `last_index_fit` from a
            // constant / for-loop iter var / `if idx < len(v)` guard). Wrap the element type
            // `Optional` so `(N-Store)` forces a `?? d` / `τ?` slot / guard at the store site.
            // The F1a landing (deps Optional-transparency + the corpus/lib migrations) cleared the
            // blast radius, so this is now folded into the DN1 default (was DEV-GATED `LOFT_INDEX_DEV`).
            // A tagged `__nullable<S>` element is ALREADY the slot's spelling of `S?`
            // (`@FR-L-Null-Tag`), and its absence is read through the tag when the value
            // leaves the slot; wrapping it here built `Optional(__nullable<S>)` — the `τ??`
            // `@FR-N-Idem` forbids, which `Type::optional` cannot see because the synthetic
            // is an `Enum` to it — and a `vector<S?>` read by a variable index then typed
            // its local `S?` on one pass and `__nullable<S>?` on the other and refused the
            // program as a type change.
            if crate::keys::pln25_dn1_enabled()
                && !self.last_index_fit
                && self.tagged_pointer_type(&elm_type).is_none()
            {
                elm_type = Type::optional(elm_type);
            }
        } else if matches!(t, Type::Text(_)) {
            let index_t = if self.lexer.peek_token("..") {
                p = Value::Int(0);
                I32.clone()
            } else {
                self.expression(&mut p)
            };
            if self.parse_text_index(code, &mut p, &index_t) == Type::Character {
                elm_type = Type::Character;
            }
        } else if let Some((el_nr, keys)) = match &t {
            Type::Hash(el, keys, _) | Type::Radix(el, keys, _) => Some((*el, keys.clone())),
            // A trie carries ONE key name; this path wants a list, so wrap rather
            // than fork it — the subscript logic below is identical for every keyed
            // collection, and only the RANGE forms differ (guarded per kind).
            Type::Trie(el, key, _) => Some((*el, vec![key.clone()])),
            _ => None,
        } {
            // @PLN25 E2 — key fields live in the `Some` variant when the element was
            // rewritten to `__nullable<S>`; resolve names against the key-bearing def.
            let el = crate::typedef::key_bearing_def(&self.data, el_nr);
            let mut key_types = Vec::new();
            for k in &keys {
                key_types.push(self.data.attr_type(el, self.data.attr(el, k)).clone());
            }
            // @PLN48 S3 — a `spatial` RANGE SLICE `xs[(fx,fy)..(tx,ty)]` /
            // `xs[(fx,fy)..:n]` / `xs[(fx,fy)..]`: iterate the records whose Morton
            // code is in the interval (a bounding box is the raw code interval), in
            // natural order.  Lowers to `n_spatial_range(xs, tp, fx, fy, has_till, tx,
            // ty, limit)` — the same scratch path as iteration — and returns the Radix
            // type so `parse_for` iterates the already-built scratch.  A `(` opens the
            // coordinate tuple.
            if matches!(t, Type::Radix(_, _, _)) && self.lexer.peek_token("(") {
                elm_type = self.parse_spatial_slice(code, &t, &key_types);
            } else if matches!(t, Type::Trie(_, _, _)) {
                // A trie subscript is `t[k]` (exact) or `t[pre..]` (prefix) — which one
                // is only known after the key expression is parsed, so both live in one
                // parse.  A prefix slice returns the Trie type, so the enclosing `for`
                // iterates the scratch the call builds.
                let dep = self.container_dep(code, &t);
                if let Some(slice) = self.parse_trie_slice(code, &t, &key_types) {
                    elm_type = slice;
                } else if let Some(cv) = dep {
                    elm_type = elm_type.depending(cv);
                }
            } else {
                let dep = self.container_dep(code, &t);
                self.parse_key(code, &t, &key_types);
                if let Some(cv) = dep {
                    elm_type = elm_type.depending(cv);
                }
            }
            // @P285 — a keyed-collection lookup RESULT is nullable (an absent
            // key returns the null record).  `parse_key` parsed the KEY last,
            // so `expr_not_null` still reflects the key (e.g. a `not null`
            // field) — clear it so a following `lookup == null` membership
            // test doesn't fire a bogus "Redundant null check" attributed to
            // the key.
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
        } else if let Type::Sorted(el, keys, _) | Type::Index(el, keys, _) = &t {
            let el = crate::typedef::key_bearing_def(&self.data, *el);
            let mut key_types = Vec::new();
            for (k, _) in keys {
                key_types.push(self.data.attr_type(el, self.data.attr(el, k)).clone());
            }
            let dep = self.container_dep(code, &t);
            self.parse_key(code, &t, &key_types);
            if let Some(cv) = dep {
                elm_type = elm_type.depending(cv);
            }
            // @P285 — see the Hash/Radix arm above; the lookup result is nullable.
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
        } else if self.user_index_op(&t) != u32::MAX {
            // @PLN125 arc C — `x[i]` on a library type is the call the type declared.
            // `OpIndex` takes the receiver and the indices, so the lowering is the
            // ordinary method call `t_<LEN><Type>_OpIndex(x, i, …)`, and every rule that
            // governs a method call — argument conversion, the heap-return buffer, the
            // ownership deps, the arity and type checks — governs this one because it IS
            // one.
            //
            // loft#996 — COMMA-separated indices, so `m[r, c]` reaches the two-index
            // method the feature's own motivating case (a matrix) wants. That
            // declaration was always accepted and callable as `OpIndex(m, r, c)`; only
            // its own syntax could not reach it, and the parser said `Expect token ]` at
            // the comma. Passing the indices through as ARGUMENTS is what the accepted
            // declaration already means, and it leaves the arity check where it belongs:
            // `call_nr` reports a mismatch against the signature the author wrote.
            //
            // The index expressions are parsed here rather than by `parse_method`, which
            // reads a parenthesised argument list; the brackets are the caller's
            // (`operators.rs` consumes the `]`).
            let md_nr = self.user_index_op(&t);
            let recv = code.clone();
            let mut args = vec![recv];
            let mut types = vec![t.clone()];
            let mut arg_pos = vec![self.lexer.peek_pos().clone()];
            loop {
                arg_pos.push(self.lexer.peek_pos().clone());
                if self.user_index_slice_refused(&t) {
                    return Type::Never;
                }
                let mut idx = Value::Null;
                let idx_t = self.expression(&mut idx);
                args.push(idx);
                types.push(idx_t);
                if self.user_index_slice_refused(&t) {
                    return Type::Never;
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            elm_type = self.call_nr(code, md_nr, &args, &types, true, &arg_pos, None);
        } else if t.is_unknown() {
            // @P278/P281 — pass-1 Unknown receiver: consume the
            // entire bracket content including range syntax
            // (`a..b`, `a..=b`, `a..`, `..b`, `..`) so the outer
            // caller's `]` consumption matches and no spurious
            // "Expect token ]" / "Expect token ;" cascade fires.
            // Pass-2 (with all defs registered) re-parses the body
            // cleanly and dispatches correctly — or emits the real
            // diagnostic if the receiver is genuinely unknown.
            // A COMPOUND key is one of those spellings: `hash<Tile[x, y]>` is read
            // `g.cells[1, 2]`, and consuming only the first expression left the `,`
            // for the caller's `]`.  That made the lookup depend on where its types
            // were DECLARED — fine above the use, `Expect token ]` below it — which
            // is the same ordering dependency a named argument had at
            // `skip_remaining_args`.  Whatever this pass cannot resolve, it still
            // has to be able to read.
            if !self.lexer.peek_token("]") {
                if !self.lexer.peek_token("..") && !self.lexer.peek_token("..=") {
                    let mut p = Value::Null;
                    self.expression(&mut p);
                }
                if (self.lexer.has_token("..") || self.lexer.has_token("..="))
                    && !self.lexer.peek_token("]")
                {
                    let mut p2 = Value::Null;
                    self.expression(&mut p2);
                }
                while self.lexer.has_token(",") && !self.lexer.peek_token("]") {
                    let mut pn = Value::Null;
                    self.expression(&mut pn);
                }
            }
        } else {
            // index_type() already emitted a diagnostic; consume the inner expression
            // so that the caller can still parse the closing `]` without cascading errors.
            let mut p = Value::Null;
            self.expression(&mut p);
        }
        elm_type
    }

    /// @PLN25 DN3 (index) — is a vector index provably in-bounds, so `v[i]` cannot be OOB-null?
    /// True for a non-negative constant literal (the developer typed it), a for-loop iteration
    /// variable (`for i in <range> { v[i] }` — the loop's bound is the contract), or (@PLN102 D1)
    /// an integer-arithmetic index built purely from those trusted leaves (`m[k*4+row]` — the
    /// matrix-indexing contract). Everything else (a general var, an index touching an untrusted
    /// var) can overrun → the read types `τ?`. Mirrors the pass-2 warning walk's skip patterns 2/3;
    /// the `i < len(v)` guard (pattern 5) is added separately.
    /// Does `code` read an ELEMENT out of a collection — `v[i]`, `m[k]` — possibly
    /// through a chain of field reads (`v[i].pos` for `v[i].pos.x`)?
    ///
    /// Such a read yields null at runtime when the element is absent, whatever the
    /// element type says: an out-of-range index and a missing key both produce the
    /// null value (C80).  The index arms already clear `expr_not_null` for exactly
    /// that reason — a bare `v[i]` warning about an overrun while `v[i] ?? d`
    /// warned "redundant" left the author no clean idiom.  A following field read
    /// re-armed the flag from the FIELD's own non-nullness and put the trap back
    /// one level down, so `v[i].pos.x ?? 0.0` — the correct defence — was reported
    /// as dead code.  This carries the receiver's fact through the chain, which is
    /// the rule stated at the top of `field`: an access cannot be MORE non-null
    /// than the thing it reads from.
    ///
    /// Lint signal only: the element type is unchanged, so a constant index stays
    /// the trusted developer contract `index_provably_fit` makes it.
    fn reads_a_collection_element(&self, code: &Value) -> bool {
        match code.unspan() {
            Value::Call(d, args) => {
                let name = self.data.def(*d).name.as_str();
                if matches!(
                    name,
                    "OpGetVector"
                        | "OpGetVectorNullable"
                        | "OpVectorRef"
                        | "OpVectorRefNullable"
                        | "OpGetHash"
                        | "OpGetSorted"
                        | "OpGetIndex"
                        | "OpGetRadix"
                ) {
                    return true;
                }
                // Walk down a field-read chain to the value it started from.
                name.starts_with("OpGet")
                    && args
                        .first()
                        .is_some_and(|a| self.reads_a_collection_element(a))
            }
            _ => false,
        }
    }

    fn index_provably_fit(&self, index: &Value, vec: &Value) -> bool {
        // A compile-time-constant index — positive OR negative (`v[-1]` is the Python-style
        // last-element idiom; `-1` lowers to a negation, so use `const_int`, not a literal match)
        // — is the developer's explicit contract: trust it, exactly as the runtime does (a genuine
        // overrun still raises the recoverable OOB fault). A variable index carries no such
        // contract, so it stays `τ?` unless proven fit below.
        if self.const_int(index).is_some() {
            return true;
        }
        match index.unspan() {
            Value::Var(v) => {
                // A for-loop iteration variable (`for i in <range> { v[i] }`) — the vars system
                // already tracks the active loop stack, so no separate parse-time stack is needed.
                if self.vars.is_active_loop_var(*v) {
                    return true;
                }
                // An `if idx < len(vec) { vec[idx] }` guard proved the (idx, vec) pair in-bounds
                // for this branch — match `v` AND the indexed vector's `VecKey`.
                crate::parser::operators::vec_key(vec, &self.data).is_some_and(|vk| {
                    self.index_bounded
                        .iter()
                        .any(|(iv, ik)| *iv == *v && *ik == vk)
                })
            }
            // @PLN102 D1 — a computed index like `m[k*4+row]` is the matrix-indexing contract: an
            // integer-arithmetic tree over constants and active loop vars. Trust it exactly as a
            // bare loop var (a real OOB still faults → null at runtime, C80). This deliberately does
            // NOT thread the `i < len(v)` guard through arithmetic — that proof is specific to
            // `v[i]` and does not survive `v[i*2]` — so `index_arith_trusted` reads only the two
            // by-contract leaves, never `index_bounded`.
            _ => self.index_arith_trusted(index),
        }
    }

    /// @PLN102 D1 — is `index` an integer-arithmetic expression built purely from trusted leaves
    /// (constants and active for-loop iteration variables)? The recursive half of
    /// `index_provably_fit` for computed matrix/vector indices (`k * 4 + row`, `col * stride + k`).
    /// A leaf is a constant (`const_int`) or an active loop var; a node is one of the integer
    /// arithmetic ops. Any other var (a plain local, a guard-bounded var) or non-arithmetic call
    /// (`len(w)`, `f(i)`) breaks the chain → `false`, keeping the read `τ?`.
    fn index_arith_trusted(&self, index: &Value) -> bool {
        if self.const_int(index).is_some() {
            return true;
        }
        match index.unspan() {
            Value::Var(v) => self.vars.is_active_loop_var(*v),
            Value::Call(op, args) if self.is_index_arith_op(*op) => {
                args.iter().all(|a| self.index_arith_trusted(a))
            }
            _ => false,
        }
    }

    /// @PLN102 D1 — is `op` one of the integer/long arithmetic operators an index expression can be
    /// composed from (`+ - * / %`)? Bitwise / shift / comparison ops are intentionally excluded:
    /// the trusted set is ordinary index arithmetic, nothing wider.
    fn is_index_arith_op(&self, op: u32) -> bool {
        matches!(
            self.data.def(op).name.as_str(),
            "OpAddInt"
                | "OpMinInt"
                | "OpMulInt"
                | "OpDivInt"
                | "OpModInt"
                | "OpAddLong"
                | "OpMinLong"
                | "OpMulLong"
                | "OpDivLong"
                | "OpModLong"
        )
    }

    /// @PLN125 arc C — the `OpIndex` a library type defines for `x[i]`, or `u32::MAX`.
    ///
    /// The last place a library type was visibly not a built-in one: `OpIndex` was the one
    /// operator the parser never dispatched, so a matrix, a bitset, a row or a ring buffer
    /// had to read as `x.at(i)` while every other operator already had its
    /// `OpCamelCase` method (@PLN99). This follows that precedent exactly — the method is
    /// found the same way, by the same `t_<LEN><Type>_Op…` name — so nothing about the
    /// shape is new.
    ///
    /// Looked up as a METHOD, not through `find_fn`, whose fallback to a global
    /// `n_OpIndex` would let an unrelated free function of that name capture every
    /// subscript in the program.
    ///
    /// One home for the answer: [`Self::index_type`] uses it for the TYPE of `x[i]` and
    /// [`Self::parse_index`] for the CODE, and a type that answers one must answer the
    /// other or the two disagree about what indexing means.
    pub(crate) fn user_index_op(&self, t: &Type) -> u32 {
        let d = match t.base() {
            Type::Reference(d, _) | Type::Enum(d, _, _) => *d,
            _ => return u32::MAX,
        };
        if d as usize >= self.data.definitions.len() {
            return u32::MAX;
        }
        // loft#1153 — a HOLDER's stub and a concrete type's method are spelled differently,
        // and this site looks up BOTH: `x[0]` reaches here for a bounded type variable and for a
        // struct defining `OpIndex` alike.  `method_key` is the one home that knows which.
        // `x[i]` is arity 2 — receiver plus index — which is what keys a HOLDER's stub
        // (loft#1275); for a concrete type the arity is not part of the spelling.
        let md = self.data.def_nr(&self.data.method_key(d, "OpIndex", 2));
        if md == u32::MAX || !matches!(self.data.def_type(md), DefType::Function | DefType::Generic)
        {
            return u32::MAX;
        }
        // A stub is named for the HOLDER — a type variable or an associated type — and
        // holder names are shared: `fn a<I: Indexable>` mints `t_1I_OpIndex`, and an
        // unrelated `fn b<I>(x: I) { x[0] }` in the same program would then find it and
        // subscript a type it was never promised anything about. So for a holder the
        // lookup is not enough; the BOUNDS have to declare it. (The same guard the
        // binary-operator path carries, for the same reason.)
        if self.data.is_type_var_placeholder(d) && !self.has_bound_for_method("OpIndex", d, None) {
            return u32::MAX;
        }
        md
    }

    /// Refuse `x[a..b]` on a library type, and say what to write — loft#996.
    ///
    /// A slice is not a subscript with a different argument: every built-in kind lowers
    /// its own (`parse_vector_index`, `parse_text_index`, `parse_spatial_slice`,
    /// `parse_trie_slice`), each to a dedicated runtime call, and there is no range VALUE
    /// in the language for a user method to take. So this cannot be sugar for `OpIndex`
    /// the way the comma form is — it needs a range type or an `OpSlice` of its own, which
    /// is a language addition and not a parse.
    ///
    /// What it must not do is stay `Expect token ]` pointing at the `..`, beside the two
    /// messages this feature already gets right. Answers `true` when the subscript is a
    /// slice, having consumed the rest of the bracket so the caller's `]` still matches —
    /// the same recovery the pass-1 `Unknown` receiver takes above, and for the same
    /// reason: returning with `..2` unread cascades into `Expect token ]` on pass 1, which
    /// aborts before pass 2 can report anything at all.
    fn user_index_slice_refused(&mut self, t: &Type) -> bool {
        if !self.lexer.peek_token("..") && !self.lexer.peek_token("..=") {
            return false;
        }
        if !self.first_pass {
            let name = match t.base() {
                Type::Reference(d, _) | Type::Enum(d, _, _) => self.data.def(*d).name().to_string(),
                _ => String::from("this type"),
            };
            diagnostic!(
                self.lexer,
                Level::Error,
                "`{name}` defines `OpIndex`, which takes INDEX arguments — there is no \
                 range value to hand it, so `x[a..b]` has nothing to dispatch to; write \
                 the bounds as indices (`x[a, b]`, if `OpIndex` declares two) or give the \
                 type a method that slices (`x.slice(a, b)`)"
            );
        }
        // Consume `..` / `..=` and any till-expression, leaving the `]` for the caller.
        let _ = self.lexer.has_token("..") || self.lexer.has_token("..=");
        if !self.lexer.peek_token("]") {
            let mut till = Value::Null;
            self.expression(&mut till);
        }
        true
    }

    pub(crate) fn index_type(&mut self, t: &Type) -> Type {
        // @PLN125 arc C — a library type that defines `OpIndex` indexes like a built-in
        // one, and the element type is what that method returns.  Answered BEFORE the
        // refusal below, which is what used to be the only answer for a struct.
        let user_op = self.user_index_op(t);
        if user_op != u32::MAX {
            return self.data.def(user_op).returned().clone();
        }
        if let Type::Vector(v_t, _) = t {
            *v_t.clone()
        } else if let Type::Sorted(d_nr, _, _)
        | Type::Hash(d_nr, _, _)
        | Type::Index(d_nr, _, _)
        | Type::Radix(d_nr, _, _)
        | Type::Trie(d_nr, _, _) = t
        {
            let ret = self.data.def(*d_nr).returned().clone();
            // S16b: struct-enum variants have .returned = Type::Enum(parent, true, []).
            // For collection element access we need Type::Reference(variant_def_nr, [])
            // so that field access and range-query for-loops resolve fields against the
            // variant struct (not the parent enum), and for_type() can map the element type.
            if matches!(ret, Type::Enum(_, true, _)) {
                // @PLN25 E2 — a synth `__nullable<S>` element keeps its `Enum`
                // type (here `d_nr` IS the enum, not a variant), exactly as a
                // `vector<__nullable<S>>` element does, so the field-access
                // unwrap in `field()` resolves S's fields through `Some` and a
                // `lookup[k] == null` test works. Converting it to `Reference`
                // would point at the enum (no payload fields) → "Unknown field".
                if self.data.def(*d_nr).name.starts_with("__nullable<") {
                    ret
                } else {
                    Type::Reference(*d_nr, crate::data::Deps::none())
                }
            } else {
                ret
            }
        } else if matches!(t, Type::Text(_)) {
            t.clone()
        } else if let Type::RefVar(tp) = t {
            *tp.clone()
        } else if t.is_unknown() {
            // First pass: type not yet resolved; suppress error until second pass.
            Type::Unknown(0)
        } else if matches!(t, Type::Never) {
            // A poisoned receiver stays poisoned.  The receiver's own error (an
            // unknown function, an unknown struct) is already on screen, so
            // indexing it has nothing left to say — and reporting anyway names the
            // index line, which is correct as written, right beside the line that
            // is not (loft#868).  Mirrors the `Unknown | Never` recovery in
            // `field()`.
            Type::Never
        } else {
            // @PLN125 arc C — a library type CAN be subscripted now, so a struct or enum
            // arriving here has a cause of its own: it did not define `OpIndex`.  Saying
            // that names the one line the author has to add, where the keyed-collection
            // message below would send them to a construct that has nothing to do with it.
            //
            // A type VARIABLE gets the third message: the type may well define `OpIndex`,
            // but inside a generic only the BOUNDS may be relied on, so the fix is a bound
            // and not a method.
            match t.base() {
                Type::Reference(d, _) | Type::Enum(d, _, _)
                    if self.data.is_type_var_placeholder(*d) =>
                {
                    let name =
                        crate::data::Data::type_var_spelling(self.data.def(*d).name()).to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "generic type {name}: `[…]` needs a bound that declares it — add \
                         `op [] (self: Self, i: integer) -> τ` to an interface and bound \
                         `{name}` by it"
                    );
                }
                Type::Reference(d, _) | Type::Enum(d, _, _) => {
                    let name = self.data.def(*d).name().to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`{name}` cannot be indexed — define \
                         `fn OpIndex(self: {name}, i: integer) -> τ` to give it `x[i]`"
                    );
                }
                // QUALITY 6d: the "Indexing a non vector" message fires for two
                // very different user intents — real misuse of `[..]` on a
                // scalar, and an attempted generic-constructor
                // (`hash<Row[id]>()`, `sorted<Elm[k]>()`) that the language
                // doesn't support.  The second case leaves readers stuck; point
                // at the type-annotated local idiom that *does* work (a struct
                // field works too, but the local form is usually what they want).
                _ => diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Indexing a non vector — keyed collections (hash/sorted/index/spatial) have no generic-constructor expression; name the key via a type annotation and initialise from a vector literal: `h: hash<Row[id]> = [Row {{ id: 1 }}];` (a struct field `struct Db {{ h: hash<Row[id]> }}` works too)"
                ),
            }
            Type::Unknown(0)
        }
    }

    pub(crate) fn parse_vector_index(
        &mut self,
        code: &mut Value,
        elm_type: &Type,
        etp: &Type,
    ) -> Option<Type> {
        let mut p = Value::Null;
        let index_t = self.parse_in_range(&mut p, code, "$");
        // @PLN25 — a nullable `τ?` INDEX is ACCEPTED (not an (N-Store) violation): `v[i]`
        // is already `τ?` (out-of-bounds → null), so the caller must null-check the result
        // regardless, and a null index just propagates to that null result. (N-Store) governs
        // storing null INTO a non-null slot (decl/field/return/typed-store), not passing a
        // nullable THROUGH an op whose result stays honestly nullable. So no rejection here.
        // Pass-1 deferral: a `vector<S>` whose element `S` is not yet registered
        // (forward-referenced or cross-package, e.g. `vector<WallDef>` indexed
        // before `WallDef` is parsed) yields `type_elm == u32::MAX`, which would
        // panic the `def(elm_td)` below.  `etp` is still `Unknown` here, so the
        // op-dispatch matches (Tuple/Function/base) all fall through to the
        // generic linked-handle read.  Substitute the builtin `reference` def so
        // a placeholder read (stride + `OpVectorRef`) builds and the caller's
        // index chain / assignment stays well-formed; pass-2 sees the resolved
        // element, skips this branch, and rebuilds the real op + stride.  Only
        // reachable in pass-1 (pass-2 has every def); a genuinely-undefined
        // element surfaces its error at the type declaration, not here.
        let elm_td = match self.data.type_elm(etp) {
            u32::MAX => self.data.source_nr(0, "reference"),
            td => td,
        };
        let known = self.data.def(elm_td).known_type();
        // honour narrow vector-element stride when the content Type::Integer carries a
        // forced_size that registers a direct-encoded narrow type (see
        // `IntegerSpec::vector_narrow_width`, which accepts 1, 2 and 4 bytes).  Falls back
        // to the bounds-heuristic via `database.size(known_type)` otherwise.  @FR-H-Stride:
        // the width is the declared TYPE's, and `known` above cannot carry it — one
        // `integer` def serves every width.
        let elm_size_raw = if let Type::Integer(spec) = etp
            && let Some(n) = spec.vector_narrow_width(false)
        {
            i32::from(n)
        } else if let Some(elem) = self.data.vector_element_type(etp, &mut self.database) {
            // Read the stride from the SAME element-type derivation the storage
            // side uses (`Data::vector_element_type`), so the reader can never
            // stride differently from the writer.
            //
            // P214: Type::Function vector elements route through
            // `narrow_vector_content` to a `database.int(0, false)`
            // (size 4 d_nr storage).  The previous fallback via
            // `data.def(elm_td).known_type` returned `u16::MAX` for
            // synthetic `i32` defs without a registered known_type,
            // making `database.size(known) = 0` and producing a
            // stride-0 read that always hit slot 0 regardless of
            // index.
            //
            // A NESTED vector element resolves to a real `vector<…>` id (4-byte
            // handle) instead of the level-collapsed inner scalar; without that,
            // `v[i]` on a `vector<vector<u8>>` strode 8 while the rows were
            // written 4 apart, so every row but the first read as empty.
            i32::from(self.database.size(elem))
        } else {
            i32::from(self.database.size(known))
        };
        // @PLAN58 cluster-I (boolean outer-handle stride): a vector-typed element
        // is a 4-byte rec-id HANDLE.  The bounds-heuristic above yields the inner
        // scalar size — fine when ≥4, but a 1-byte `boolean` inner makes adjacent
        // handles overlap on read.  Clamp to ≥4, matching the construction-side
        // `known` fix in `new_record`.  A no-op for ≥4 strides; no classification
        // change (`known` / is_base / is_linked / deref type are untouched).
        let elm_size = if matches!(elm_type, Type::Vector(_, _)) {
            elm_size_raw.max(4)
        } else {
            elm_size_raw
        };
        if let Value::Iter(var, init, next, extra_init) = p {
            if matches!(*next, Value::Block(_)) {
                // Plan-07 phase 4 step 4.6 — this is the for-loop
                // iteration branch (`Value::Iter` carrying the loop's
                // step block).  Use the *Nullable* peers so OOB at
                // end-of-iteration returns a null DbRef instead of
                // raising — matches the loop-driver expectation
                // documented at parser/collections.rs:1492-1499.
                // The explicit `v[i]` branch below (line 629-640) is
                // unchanged and emits the raising OpGetVector / OpVectorRef.
                //
                // Linked structs: array stores 4-byte record pointers → use OpVectorRefNullable
                // which internally uses elm_size=4 and dereferences to the actual record.
                // Base/primitive types: array stores inline values → use OpGetVectorNullable + get_val.
                let op = if self.database.is_linked(known) {
                    self.cl("OpVectorRefNullable", &[code.clone(), *next.clone()])
                } else {
                    let mut v = self.cl(
                        "OpGetVectorNullable",
                        &[code.clone(), Value::Int(elm_size), *next.clone()],
                    );
                    if self.database.is_base(known) {
                        // Pass the ELEMENT's declared nullability (its slot
                        // reserves a sentinel iff `τ?`), NOT the OOB-nullable
                        // read result — a nullable narrow element decodes via
                        // its sentinel op, a `vector<u8>` element stays raw.
                        v = self.get_val(etp, matches!(etp, Type::Optional(_)), 0, v, u32::MAX);
                    } else if let Type::Tuple(elems) = etp {
                        v = self.unbox_tuple_from_dbref(v, elems);
                    }
                    v
                };
                *code = Value::Iter(
                    var,
                    init,
                    Box::new(v_block(vec![op], etp.clone(), "Vector Index")),
                    extra_init,
                );
                return Some(Type::Iterator(
                    Box::new(elm_type.clone()),
                    Box::new(Type::Null),
                ));
            }
            diagnostic!(self.lexer, Level::Error, "Invalid iterator expression");
            return None;
        }
        // @PLN25 DN3 (index): decide this scalar read's element nullability. A `v[i]` is nullable
        // (OOB → the null sentinel) UNLESS the index is provably in-bounds — a non-negative constant
        // (the developer typed a literal), or a for-loop iteration variable (the loop's bound is the
        // contract). `parse_index` reads this flag to wrap the element type `Optional` when unfit.
        self.last_index_fit = self.index_provably_fit(&p, code);
        // @PLN102 strict-index lint (gated by `LOFT_LINT_STRICT_INDEX`, advisory). The index is
        // trusted in-bounds because it is a for-loop iter var — but if that loop is bounded by
        // `len(<other vector>)`, `for i in 0..len(v) { w[i] }` types `w[i]` non-null yet reads
        // C80-null on overrun. Warn on the mismatch; the type stays non-null (a real proof would
        // break the ubiquitous `for i in 0..n { v[i] }` idiom — see `strict_index_lint_enabled`).
        if self.last_index_fit
            && !self.first_pass
            && crate::keys::strict_index_lint_enabled()
            && let Value::Var(iv) = p.unspan()
            && let Some(bound) = self.vars.loop_len_bound(*iv)
            && let Some(indexed) = crate::parser::operators::vec_key(code, &self.data)
            && bound != indexed
        {
            let iname = self.vars.name(*iv).to_string();
            diagnostic!(
                self.lexer,
                Level::Warning,
                code = "index-bounds-other-vector",
                "index `{iname}` is bounded by `len(...)` of a different vector than the one \
                 indexed here — the index is typed non-null but reads null on overrun"
            );
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "bound the loop by the vector it indexes".to_string(),
                condition: Some("the two vectors are not guaranteed the same length".to_string()),
                edit: None,
                concept: "vector",
                concept_ref: "@F6",
            });
        }
        // @FR-N-Store — an index is a slot: `v[i]` with `i: integer?` reads null on a null
        // index, and nothing said so (the eighth hole of @PLN153 phase 3's census).
        if !self.first_pass
            && !self.convert_store_lenient(&mut p, &index_t, &I32, "the index", None)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Invalid index type {} on vector",
                index_t.show(&self.data, &self.vars)
            );
        }
        // Linked structs: array stores 4-byte record pointers → OpVectorRef dereferences correctly.
        // Base/primitive types: inline data → OpGetVector + get_val reads the primitive value.
        // Plain inline structs: OpGetVector only (field access happens at the next level).
        // Tuple elements: OpGetVector + per-field reads via the DbRef (P189b — see
        // `unbox_tuple_from_dbref`).  Without this the assignment `p = pairs[0]`
        // wrote DbRef bytes into the tuple slot and `p.0` / `p.1` decoded them
        // as garbage integers.
        if self.database.is_linked(known) {
            *code = self.cl("OpVectorRef", &[code.clone(), p]);
        } else {
            *code = self.cl("OpGetVector", &[code.clone(), Value::Int(elm_size), p]);
            if self.database.is_base(known) {
                // Element's DECLARED nullability drives the sentinel decode (see
                // the Nullable-iterator branch above); the OOB-nullable read
                // result is orthogonal (the caller null-checks `v[i]` regardless).
                *code = self.get_val(
                    etp,
                    matches!(etp, Type::Optional(_)),
                    0,
                    code.clone(),
                    u32::MAX,
                );
            } else if let Type::Tuple(elems) = etp {
                *code = self.unbox_tuple_from_dbref(code.clone(), elems);
            } else if matches!(etp, Type::Function(_, _, _)) {
                // P214: vector elements of `fn(...) -> ...` type are
                // stored as 4-byte d_nr only (non-capturing — capturing
                // closures in vectors are deferred).  The variable's
                // stack representation is `(u32, DbRef)` (a fn-ref
                // tuple), so the read assembles the tuple from the
                // 4B slot d_nr + a null closure sentinel.  Mirrors the
                // struct-field non-capturing path in
                // `parser/mod.rs::get_val` Type::Function arm but uses
                // an explicit `OpNullRefSentinel` for the closure half
                // since vectors don't have a separate `__closure_rec`
                // sub-field.
                let slot = code.clone();
                let read_dnr = self.cl("OpGetInt4", &[slot, Value::Int(0)]);
                let read_clos = self.cl("OpNullRefSentinel", &[]);
                *code = crate::data::v_block(
                    vec![read_dnr, read_clos],
                    etp.clone(),
                    // Reuse the field-read block name so native codegen's
                    // tuple-emit shortcut (`((d_nr) as u32, closure_DbRef)`)
                    // fires here too — the block layout is identical.
                    "fn_ref_field_read",
                );
            }
        }
        None
    }

    /// `Some(deps)` when `v` reads an element out of a vector — the DbRef it yields is a
    /// pointer INTO that vector's store rather than a record of its own — carrying the
    /// deps a cursor over it should be typed with.  `None` for anything else.
    ///
    /// The two ops named here are the only borrowing producers that can reach
    /// [`Parser::unbox_tuple_from_dbref`]: the `is_linked` element paths emit
    /// `OpVectorRef`/`OpVectorRefNullable` and dereference to a record instead of
    /// unboxing, and every other caller hands over an expression it does not read out of
    /// a vector at all.  Anything unrecognised answers `None`, so a new producer keeps
    /// the owning treatment rather than silently inheriting a suppression.
    ///
    /// The receiver is a plain variable for `v[i]` and a field / call chain for
    /// `s.pts[i]`.  Only a variable can be named in a frame dep list, so a chain answers
    /// EMPTY deps: still exempt from the free, but read as owning by the assignment
    /// lowering, which keeps the copy it has always made there.
    fn vector_element_cursor_deps(&self, v: &Value) -> Option<crate::data::Deps> {
        let Value::Call(d_nr, args) = v.unspan() else {
            return None;
        };
        if !matches!(
            self.data.def(*d_nr).name(),
            "OpGetVector" | "OpGetVectorNullable"
        ) {
            return None;
        }
        Some(match args.first().map(Value::unspan) {
            Some(Value::Var(x)) => crate::data::Deps::frame1(*x),
            _ => crate::data::Deps::none(),
        })
    }

    /// P189b: assemble a stack-tuple from a DbRef pointing to an
    /// inline tuple in vector storage.
    ///
    /// `dbref` is a Value that, at runtime, evaluates to a `DbRef`
    /// pointing at the start of an inline tuple's bytes (typically
    /// `OpGetVector(pairs, stride, idx)`).  This helper stashes the
    /// DbRef in a fresh work-ref of type `Reference(__tuple<…>)` so
    /// per-element loads (`OpGetInt(dbref, off)`, `OpGetText(dbref,
    /// off)`, etc. via `get_val`) read at the right field offsets,
    /// then wraps the per-element loads in `Value::Tuple` so the
    /// caller's `Set(p, …)` produces the correct stack-tuple
    /// representation (each element pushed onto contiguous slots).
    ///
    /// Heap-vs-stack layout differs for `text` and reference fields
    /// (heap stores a 4-byte interned pointer; stack uses a 16-byte
    /// `Str` / `DbRef`).  `get_val`'s per-type dispatch handles the
    /// inflation correctly because each type uses the same opcodes
    /// the struct-field path uses (`OpGetText` reads a 4-byte heap
    /// text pointer and pushes a 16-byte `Str` onto the stack).
    ///
    /// Whether that work-ref BORROWS what it points at or owns it is
    /// [`Parser::vector_element_cursor_deps`]' question — it decides both the deps the
    /// cursor is typed with and whether the scope-exit free applies (loft#857/#858).
    pub(crate) fn unbox_tuple_from_dbref(&mut self, dbref: Value, elems: &[Type]) -> Value {
        let elems_vec = elems.to_vec();
        let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
        if tuple_d_nr == u32::MAX {
            // No record shape for this tuple, so there is nothing to unbox INTO — a member
            // never resolved.  Typing the temp `Reference(u32::MAX)` instead reached
            // `data.def(MAX)` and reported an internal compiler error for a plain undefined
            // name in a tuple element (loft#944).  The unresolved member is reported by the
            // type resolution that failed to find it; say nothing further and hand back the
            // cursor unchanged so the parse can finish and collect the rest.
            return dbref;
        }
        // Is this cursor a POINTER INTO somebody else's store, or a record of its own?
        // Both arrive at this one helper, and the answer is a property of the DbRef: read
        // an element out of a `vector<(…)>` and it points into that vector; unbox a
        // heap-carrying tuple RETURN and it is the callee's own record, which nobody else
        // will release.  A vector element read is the positive, checkable case and the
        // only borrowing producer that reaches here (the `is_linked` element paths never
        // unbox), so everything unrecognised keeps the owning treatment it had.
        let borrows = self.vector_element_cursor_deps(&dbref);
        // loft#858 — say BORROW in the type, not merely "do not free".  The deps list is
        // what the assignment lowering reads: an owning `Reference` local must hold its
        // own store, so `__ref_1 = <foreign DbRef>` lowered to `OpDatabase` + `OpCopyRecord`
        // — every `v[i]` on a `vector<(…)>` allocated a store, deep-copied the element into
        // it and freed the previous one, then read the elements back out of the copy.  That
        // is the ~13× against `vector<struct>`, whose cursor has carried a dep on the
        // vector all along and therefore just copies a pointer; it is also why the gap
        // barely moved with arity (the allocate/free dominates, not the element loads).
        let ref_tp = Type::Reference(
            tuple_d_nr,
            borrows.clone().unwrap_or_else(crate::data::Deps::none),
        );
        let tmp = self.vars.work_refs(&ref_tp, &mut self.lexer);
        if !self.first_pass {
            self.change_var_type(tmp, &ref_tp);
            // loft#857 — and the deps alone are not enough to stop the free, because the
            // scope-exit sweep frees a `__ref_N` on its NAME (`scopes.rs`: "work-refs …
            // accumulate unfreed stores") ahead of asking whether it owns anything — a rule
            // written for the work-refs that back ref-returning calls, which do own what
            // they hold.  So it freed the borrowing cursor too, and reading `v[i]` out of a
            // `vector<(…)>` PARAMETER destroyed the CALLER's vector store on return: the
            // slot was recycled, and the next call's `+=` appended through a handle that
            // now named another record entirely.  Suppressing the free for the OWNING
            // sources instead leaks their return buffer, which is what a blanket skip did
            // to the four `pair(…)` returns in `822-vector-tuple-spellings.loft`.
            //
            // Only the MARK is pass-2: the variable table persists across passes by name
            // while the `__ref_N` counter restarts, so a pass-1 mark could land on whatever
            // temp pass 2 gives that name (loft#848) and suppress a free that is real.
            if borrows.is_some() {
                self.vars.set_skip_free(tmp);
            }
        }
        // Stored tuples MUST use the synthetic `__tuple<…>` struct's
        // post-finish field positions (the same offsets used by
        // `OpGetInt` for ordinary struct fields).  Falls back to the
        // alignment-aware `element_stack_offsets` only on early-parse paths
        // before `finish_type` has run.
        let offsets: Vec<u16> = crate::data::stored_tuple_offsets_for_def(
            &self.data,
            &self.database,
            tuple_d_nr,
            elems_vec.len(),
        )
        .unwrap_or_else(|| {
            crate::data::element_stack_offsets(&elems_vec)
                .into_iter()
                .map(|x| x as u16)
                .collect()
        });
        let mut tuple_elems = Vec::new();
        for (i, et) in elems_vec.iter().enumerate() {
            let off = u32::from(offsets[i]);
            // @PLN25 — a member declared `S?` is STORED as the tagged `__nullable<S>`, so its
            // bytes at `off` are the discriminant followed by the payload.  Reading them as a
            // dense `S` starts one field early AND cannot spell absence, which is why an
            // indexed read answered the discriminant as the first field and a cleared element
            // came back as a record of zeroes.  Project through the tag instead.
            if let Some(tagged) = self.tuple_elem_tag_read(tuple_d_nr, i, &Value::Var(tmp), off, et)
            {
                tuple_elems.push(tagged);
                continue;
            }
            tuple_elems.push(self.get_val(et, false, off, Value::Var(tmp), u32::MAX));
        }
        v_block(
            vec![v_set(tmp, dbref), Value::Tuple(tuple_elems)],
            Type::Tuple(elems_vec),
            "tuple_unbox",
        )
    }

    /// loft#821 — the DbRef a [`Parser::unbox_tuple_from_dbref`] block reads FROM, if
    /// `code` is such a block.
    ///
    /// `v[i]` on a `vector<(…)>` parses as a READ: the element's DbRef is unboxed into a
    /// stack tuple.  As the left-hand side of `v[i] = t` that read is the wrong shape —
    /// the write needs the DbRef the elements live at.  Peel the unbox to recover it
    /// rather than re-deriving the element address, so the write can never address a
    /// different slot than the read.
    pub(crate) fn stored_tuple_dest(code: &Value) -> Option<Value> {
        let Value::Block(b) = code.unspan() else {
            return None;
        };
        if b.name != "tuple_unbox" {
            return None;
        }
        match b.operators.first()?.unspan() {
            Value::Set(_, dbref) => Some((**dbref).clone()),
            _ => None,
        }
    }

    /// loft#1072 — the PLACE a `fn_ref_field_read` block reads from: the host reference,
    /// the byte offset the 4-byte d_nr sits at, and whether the field carries a
    /// `__closure_rec` half at `pos + 4`.
    ///
    /// A fn-ref read is a Block, not the `Call` (a getter) or `Var` shape the assignment
    /// dispatcher recognises as a place, because @PLN114's split layout takes two reads to
    /// assemble the 20-byte pair. So `h.f = inc` was not recognised as writing ANYWHERE
    /// and fell through to *"Not implemented operation = for type function(…)"* — a
    /// message about the `=` operator, for a field that had accepted the same value in a
    /// literal one line earlier. Peel the read to recover the destination, exactly as
    /// [`Self::stored_tuple_dest`] does for a stored tuple, so the write can never address
    /// a different slot than the read.
    ///
    /// The layout answer comes from the READ's own shape — an `OpRefFromChildRec` second
    /// half means split, an `OpNullRefSentinel` means the legacy four bytes. That is the
    /// same source of truth the reader used, so reader and writer cannot disagree about
    /// where the closure half lives (or whether there is one).
    pub(crate) fn fn_ref_place(&self, code: &Value) -> Option<(Value, Value, bool)> {
        let Value::Block(b) = code.unspan() else {
            return None;
        };
        if b.name != "fn_ref_field_read" || b.operators.len() != 2 {
            return None;
        }
        let Value::Call(get_nr, args) = b.operators[0].unspan() else {
            return None;
        };
        if self.data.def(*get_nr).name() != "OpGetInt4" {
            return None;
        }
        let host = args.first()?.clone();
        let pos = args.get(1)?.clone();
        let split = matches!(
            b.operators[1].unspan(),
            Value::Call(d, _) if self.data.def(*d).name() == "OpRefFromChildRec"
        );
        Some((host, pos, split))
    }

    /// The attribute index of the `fn(…)` field of `d_nr` stored at byte offset `pos`.
    ///
    /// Inverts [`Self::field_position`], which is what put the offset into the read in the
    /// first place — the assignment site has the offset and needs the attribute back, to
    /// hand the write to the same `set_field` the struct LITERAL uses. Only `fn(…)` fields
    /// are considered, so an offset shared with a differently-typed field cannot match.
    pub(crate) fn fn_ref_attr_at(&mut self, d_nr: u32, pos: i32) -> Option<usize> {
        let names: Vec<(usize, String)> = (0..self.data.def(d_nr).attributes().len())
            .filter(|&f| matches!(self.data.attr_type(d_nr, f).base(), Type::Function(_, _, _)))
            .map(|f| (f, self.data.attr_name(d_nr, f)))
            .collect();
        names.into_iter().find_map(|(f, nm)| {
            let p = self
                .database
                .position(self.data.def(d_nr).known_type(), &nm);
            (p != u16::MAX && i32::from(p) == pos).then_some(f)
        })
    }

    /// Bind an element accessor's INDEX to a local, so a destination that is written
    /// through more than once evaluates it exactly once.
    ///
    /// Returns the accessor with its index replaced by that local, plus the statement that
    /// binds it — prepend those to the writes.  An index that is already a variable or a
    /// constant is returned untouched (nothing to save), and so is any shape that is not an
    /// element accessor.
    pub(crate) fn hoist_index_arg(&mut self, dest: Value) -> (Value, Vec<Value>) {
        let Value::Call(d_nr, args) = dest.unspan() else {
            return (dest, Vec::new());
        };
        let Some(index) = args.last() else {
            return (dest, Vec::new());
        };
        if matches!(index.unspan(), Value::Var(_) | Value::Int(_)) {
            return (dest, Vec::new());
        }
        let (d_nr, mut args) = (*d_nr, args.clone());
        let tmp = self.create_unique("__elm_idx", &crate::data::I32);
        self.vars.defined(tmp);
        let bind = crate::data::v_set(tmp, args.pop().unwrap_or(Value::Null));
        args.push(Value::Var(tmp));
        (Value::Call(d_nr, args), vec![bind])
    }

    /// @PLN110 3a / loft#749 — warn when a text slice ENDS at `len()` of the same text.
    ///
    /// `end` is the range end as written (before `convert` wraps it) and `subject` is the
    /// text being sliced.  Only a bound naming the SAME text warns: `s[i..len(other)]` is
    /// a different (and possibly deliberate) expression, and a bound held in a local is
    /// carried by `len_bound_locals` the same way the loop form carries it — that spelling
    /// is not a stylistic variant, it is the one real code reaches for.
    fn warn_text_len_slice_bound(
        &mut self,
        end: &Value,
        subject: &Value,
        bound_span: Option<(u32, u32, u32)>,
    ) {
        if self.first_pass || !crate::keys::text_index_units_lint_enabled() {
            return;
        }
        let bound = match end {
            Value::Call(d, largs)
                if matches!(
                    self.data.def(*d).original_name().as_str(),
                    "len" | "LengthVector"
                ) && largs.len() == 1 =>
            {
                crate::parser::operators::vec_key(&largs[0], &self.data)
            }
            Value::Var(n) => self.len_bound_locals.get(n).copied(),
            _ => None,
        };
        if bound.is_none() || bound != crate::parser::operators::vec_key(subject, &self.data) {
            return;
        }
        diagnostic!(
            self.lexer,
            Level::Warning,
            code = "text-slice-char-bound",
            "a text slice ends at `len(text)` (a character count) but slice bounds are \
             byte offsets — this stops short on multi-byte text"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Mechanical,
            title: "use `size(text)` for the byte length".to_string(),
            condition: None,
            edit: None,
            concept: "len vs size",
            concept_ref: "@F97",
        });
        // loft#1003 — the applicable one of the two.  `len(t)` -> `size(t)` cannot carry an
        // edit from here: BOTH spellings warn (`s[i..len(s)]` and `s[i..s.len()]`) and they
        // put the `len` token in different places, so a 3-character rename at the bound's
        // start would turn `s.len()` into `sizelen()`.  That needs the `len` TOKEN's own
        // position, which this site does not have.
        //
        // Deleting the bound does not care which spelling it is: `s[i..<anything>]` becomes
        // `s[i..]`, which takes the rest — the cure this fix already names, and the one the
        // doc recommends for exactly this shape.  Measured on both spellings.
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Mechanical,
            title: "take the rest with `text[i..]`".to_string(),
            condition: None,
            edit: bound_span.map(|(line, col, len)| crate::diagnostics::Edit {
                line,
                col,
                len,
                text: String::new(),
            }),
            concept: "len vs size",
            concept_ref: "@F97",
        });
    }

    pub(crate) fn parse_text_index(
        &mut self,
        code: &mut Value,
        p: &mut Value,
        index_t: &Type,
    ) -> Type {
        // @PLN110 3a — snapshot the raw index BEFORE `convert` may wrap it, so the
        // strict-index lint below can still recognise a bare loop variable.
        let raw_index = p.unspan().clone();
        // A first-pass UNRESOLVED index is not a wrong one — it is one whose type
        // pass 2 has not supplied yet, and refusing it here makes declaration order
        // decide whether a program compiles.  `heading_text = line[start..ln]` in the
        // published `markdown` broke exactly this way once operand deferral widened:
        // `start = hlevel + 1` correctly defers on pass 1, so the index arrives here
        // as `unknown` and this refusal fired on a program that had been compiling
        // for two releases.  Pass 2 sees the real type and still refuses a genuinely
        // non-integer index, which is the case this message is for.
        let deferred = self.first_pass && index_t.is_unknown();
        // @FR-N-Store — the index is a slot (see the vector index above).
        if !self.convert_store_lenient(p, index_t, &I32, "the index", None) && !deferred {
            // Name the offending type: the bare "invalid index" this used to
            // print reads as "indexing text is unsupported" and sent a consumer
            // hunting for a missing feature instead of at their index expression.
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot index text with '{}' — an index must be an integer (`s[i]`, `s[i..]`, `s[i..j]`)",
                index_t.name(&self.data)
            );
        }
        let mut other = Value::Null;
        if self.lexer.has_token("..") {
            let incl = self.lexer.has_token("=");
            if self.lexer.peek_token("]") {
                *code = self.cl(
                    "OpGetTextSub",
                    &[code.clone(), p.clone(), Value::Int(i32::MAX)],
                );
            } else {
                // loft#1003 — the bound's own extent, for the "take the rest" fix's edit:
                // the start of the end expression, and (after it is parsed) the start of the
                // `]` that closes the slice.  Taken here because this is the only point that
                // brackets the whole bound whatever it is spelled as.
                let bound_start = self.lexer.peek_pos().clone();
                let ot_type = self.expression(&mut other);
                let bound_end = self.lexer.peek_pos().clone();
                let bound_span = (bound_end.line == bound_start.line
                    && bound_end.pos > bound_start.pos)
                    .then(|| {
                        (
                            bound_start.line,
                            bound_start.pos,
                            bound_end.pos - bound_start.pos,
                        )
                    });
                // @PLN110 3a / loft#749 — snapshot the END before `convert` wraps it, for
                // the units lint below.
                let raw_end = other.unspan().clone();
                // Deferred on the first pass exactly as the START bound is, and for the
                // same reason: operand deferral can leave a bound untyped on pass 1, and
                // refusing it there makes declaration order decide whether a program
                // compiles.  `s[start..start + 2]` reaches this site with BOTH bounds
                // unresolved, which is why guarding only the start left the shape still
                // refused — with a different message, from four lines down.
                let end_deferred = self.first_pass && ot_type.is_unknown();
                if !self.convert_store_lenient(&mut other, &ot_type, &I32, "the index", None)
                    && !end_deferred
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot end a text slice at '{}' — a range end must be an integer (`s[i..j]`)",
                        ot_type.name(&self.data)
                    );
                }
                // @PLN110 3a / loft#749 — `s[i..len(s)]` mixes units: a slice bound is a
                // BYTE offset but `len(text)` is a CHARACTER count, so on any text with a
                // multi-byte character the slice stops short.  It is the obvious spelling
                // of "from here to the end" and it is wrong for exactly the inputs an
                // ASCII test never produces — a `key=value` parse over an author-given
                // name reached it on the first accented character.  `size(s)` is the byte
                // count and is what this means; `s[i..]` says it without a bound at all.
                self.warn_text_len_slice_bound(&raw_end, code, bound_span);
                if incl {
                    other = self.cl("OpAddInt", &[other.clone(), Value::Int(1)]);
                }
                *code = self.cl("OpGetTextSub", &[code.clone(), p.clone(), other]);
            }
            Type::Text(crate::data::Deps::none())
        } else {
            // @PLN110 3a — `for i in 0..len(s) { s[i] }` is a units error: `len(s)` is a
            // CHARACTER count but `s[i]` is byte-indexed, so the loop walks char-count byte
            // positions and under-runs / misreads multi-byte text. Warn (default-on) when the
            // index is a loop var bounded by `len` of the SAME text being indexed. Advisory —
            // iterate with `for c in s`, or use `0..size(s)` for a byte walk.
            if !self.first_pass
                && crate::keys::text_index_units_lint_enabled()
                && let Value::Var(iv) = &raw_index
                && let Some(bound) = self.vars.loop_len_bound(*iv)
                && crate::parser::operators::vec_key(code, &self.data) == Some(bound)
            {
                let iname = self.vars.name(*iv).to_string();
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    code = "text-index-char-bound",
                    "index `{iname}` walks `0..len(text)` (a character count) but `text[{iname}]` \
                     is byte-indexed — this under-runs / misreads multi-byte text"
                );
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Mechanical,
                    title: "iterate the characters with `for c in text`".to_string(),
                    condition: None,
                    edit: None,
                    concept: "len vs size",
                    concept_ref: "@F97",
                });
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: "walk bytes with `0..size(text)`".to_string(),
                    condition: Some("you meant BYTES, not characters".to_string()),
                    edit: None,
                    concept: "len vs size",
                    concept_ref: "@F97",
                });
            }
            *code = self.cl("OpTextCharacter", &[code.clone(), p.clone()]);
            Type::Character
        }
    }

    /// Parse a trie subscript: `t[k]` (exact lookup) or `t[pre..]` / `t[pre..:n]`
    /// (the prefix slice).  Returns `Some(typedef)` for the slice — the caller uses it
    /// as the iterated type — and `None` for the exact lookup, which `parse_key` has
    /// already lowered to `OpGetRecord`.
    ///
    /// The prefix form does NOT go through `parse_key`'s range branch: that branch
    /// walks a key INTERVAL between two bounds, and a trie has one bound because the
    /// prefix is the entire query (`doc/claude/plans/text-keyed-trie.md` step 5).
    /// `t[a..b]` is therefore refused rather than silently answering an interval.
    fn parse_trie_slice(
        &mut self,
        code: &mut Value,
        typedef: &Type,
        key_types: &[Type],
    ) -> Option<Type> {
        let mut pre = Value::Null;
        let pt = self.expression(&mut pre);
        if !self.convert(&mut pre, &pt, &key_types[0]) && !self.first_pass {
            diagnostic!(self.lexer, Level::Error, "Invalid index key");
        }
        if !self.lexer.has_token("..") {
            // Exact lookup — same lowering every keyed collection uses.
            let known = if self.first_pass {
                Value::Null
            } else {
                self.type_info(typedef)
            };
            let ls = vec![code.clone(), known, Value::Int(1), pre];
            *code = self.cl("OpGetRecord", &ls);
            return None;
        }
        self.lexer.has_token("="); // `..=` reads the same: a prefix has no upper bound
        let limit = if self.lexer.has_token(":") {
            let mut n = Value::Null;
            let nt = self.expression(&mut n);
            if !self.convert(&mut n, &nt, &crate::data::I64) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "trie prefix limit must be an integer"
                );
            }
            n
        } else {
            Value::Int(-1)
        };
        if !self.first_pass && !self.lexer.peek_token("]") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a trie slice is a PREFIX, not an interval — write `t[\"kerk\"..]` (every key \
                 beginning with `kerk`) or `t[\"kerk\"..:n]` for the first n; an upper bound \
                 asks for a key interval, which `sorted<…>` answers"
            );
        }
        if !self.first_pass && !self.iterable_context {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a trie prefix slice is a `for`-loop iterator, not a value — iterate it \
                 directly (`for x in t[pre..] {{ … }}`) or materialise a vector with a \
                 comprehension (`[for x in t[pre..] {{ x }}]`)"
            );
        }
        if !self.first_pass {
            let tp = self.get_type(typedef);
            let fn_nr = self.data.def_nr("n_trie_prefix");
            if tp != u16::MAX && fn_nr != u32::MAX {
                *code = Value::Call(
                    fn_nr,
                    vec![code.clone(), Value::Int(i32::from(tp)), pre, limit],
                );
            }
        }
        Some(typedef.clone())
    }

    /// @PLN48 S3 — parse a `spatial` range slice `xs[(fx,fy)..(tx,ty)]`,
    /// `xs[(fx,fy)..:n]`, or the open `xs[(fx,fy)..]`, and lower it to an
    /// `n_spatial_range` scratch-builder call.  Returns the Radix `typedef` so the
    /// enclosing `for` iterates the scratch it builds.  `(` has already been peeked.
    fn parse_spatial_slice(
        &mut self,
        code: &mut Value,
        typedef: &Type,
        key_types: &[Type],
    ) -> Type {
        // Parse a `(c0, c1, …)` coordinate tuple with exactly one value per axis of the
        // collection (`key_types.len()`), padded to MAX_AXES with `0` for the fixed-arity
        // `n_spatial_range` call.  The collection's own axis count drives how many the
        // range builder reads, so the padding is inert.
        let axes = key_types.len();
        let max_axes = crate::radix_db::MAX_AXES;
        let parse_tuple = |s: &mut Self| -> Vec<Value> {
            let mut out = Vec::new();
            s.lexer.token("(");
            loop {
                let i = out.len();
                let mut v = Value::Null;
                let vt = s.expression(&mut v);
                if !s.convert(&mut v, &vt, &key_types[i.min(axes - 1)]) {
                    diagnostic!(s.lexer, Level::Error, "Invalid spatial coordinate");
                }
                out.push(v);
                if !s.lexer.has_token(",") {
                    break;
                }
            }
            s.lexer.token(")");
            if out.len() != axes {
                diagnostic!(
                    s.lexer,
                    Level::Error,
                    "a spatial coordinate needs {axes} axes, got {}",
                    out.len()
                );
            }
            out.resize(max_axes, Value::Int(0));
            out
        };
        let from = parse_tuple(self);
        if !self.lexer.has_token("..") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a spatial slice needs a range: `xs[(x,y)..]`, `xs[(x,y)..:n]`, or `xs[(x1,y1)..(x2,y2)]`"
            );
        }
        // The `..` is followed by a till tuple `(tx,ty,…)`, a limit `:n`, or nothing.
        let (has_till, till, limit) = if self.lexer.peek_token("(") {
            (Value::Int(1), parse_tuple(self), Value::Int(-1))
        } else if self.lexer.has_token(":") {
            let mut n = Value::Null;
            let nt = self.expression(&mut n);
            // A first-pass UNRESOLVED limit is not a wrong one — the same escape the
            // start bound and the range end take a few hundred lines up, and for the
            // same reason: operand deferral can leave `xs[(0, 0)..: lim()]` untyped on
            // pass 1 when `lim` is declared lower in the file, and refusing it here makes
            // declaration order decide whether the program compiles.  Pass 2 has the real
            // type and still refuses a genuinely non-integer limit, which is the case this
            // message is for.
            //
            // Fifth site of one rule, and the one a behavioural sweep missed: 29 probes
            // over operation kinds and return types came back clean because nobody thinks
            // to write a spatial slice.  It was found by ENUMERATING the parser's
            // type-requirement refusals instead — see STABILITY_REDFLAGS.md § Result 5.
            let limit_deferred = self.first_pass && nt.is_unknown();
            if !self.convert(&mut n, &nt, &crate::data::I64) && !limit_deferred {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "spatial slice limit must be an integer"
                );
            }
            (Value::Int(0), vec![Value::Int(0); max_axes], n)
        } else {
            (Value::Int(0), vec![Value::Int(0); max_axes], Value::Int(-1))
        };
        if !self.first_pass {
            let tp = self.get_type(typedef);
            let fn_nr = self.data.def_nr("n_spatial_range");
            if tp != u16::MAX && fn_nr != u32::MAX {
                let mut args = vec![code.clone(), Value::Int(i32::from(tp))];
                args.extend(from); // fx, fy, fz
                args.push(has_till);
                args.extend(till); // tx, ty, tz
                args.push(limit);
                *code = Value::Call(fn_nr, args);
            }
        }
        typedef.clone()
    }

    /// Parse the key expression(s) of an index — `h[k]`, `h[k1, k2]`, `h[lo..hi]`.
    ///
    /// loft#683 — every "Invalid index key" below is reported in the SECOND pass only.
    /// Pass 1 collects definitions, so it sees an incomplete table by construction: a
    /// key whose type comes from a function declared further down the file reads as
    /// unknown there, and the conversion fails for a reason that has nothing to do
    /// with the program.  Raising it anyway aborted the parse before pass 2 — which
    /// knows every signature — ever ran, so `h[keys_declared_below()]` was rejected
    /// while the identical code with the callee moved up compiled.  File order is not
    /// otherwise significant in loft, which is what made it surprising.
    ///
    /// Nothing is lost by waiting: pass 2 re-parses the whole file and re-runs each
    /// check with the full type picture, so a genuinely wrong key is still rejected —
    /// just once, and pointing at a real mismatch.
    #[allow(clippy::too_many_lines)]
    /// Expand a TUPLE key value into one value per element, so a lookup supplies exactly the
    /// key contents the collection's descriptors expect.
    ///
    /// A tuple key field is a compound key spelled as one field: `hash<Cell[pos]>` with
    /// `pos: (integer, integer)` registers TWO descriptors (`determine_keys_for`), so
    /// `h[(3, 4)]` has to hand over two contents.  Handing over the tuple whole left the
    /// runtime reading one content against two descriptors and indexing off the end of the
    /// collection reference — a panic inside `hash::find`, not a diagnostic.
    ///
    /// An operand that is not already a tuple local is bound to one first (its elements are
    /// read one by one, so an expression that does work must not be re-evaluated per
    /// element); the binding lands in `prelude` for the caller to run before the lookup.
    /// Nested tuples expand recursively, matching the descriptor side's flat element order.
    fn expand_tuple_key_values(
        &mut self,
        key: Vec<Value>,
        key_types: &[Type],
        prelude: &mut Vec<Value>,
    ) -> Vec<Value> {
        let mut out = Vec::with_capacity(key.len());
        for (i, val) in key.into_iter().enumerate() {
            let Some(tp) = key_types.get(i) else {
                out.push(val);
                continue;
            };
            self.expand_one_tuple_key(val, tp, prelude, &mut out);
        }
        out
    }

    fn expand_one_tuple_key(
        &mut self,
        val: Value,
        tp: &Type,
        prelude: &mut Vec<Value>,
        out: &mut Vec<Value>,
    ) {
        let Some(elems) = self.tuple_elements(tp) else {
            out.push(val);
            return;
        };
        // A tuple written AT the lookup — `h[(3, 4)]` — already has its elements as separate
        // expressions; take them directly rather than building a local to read them back
        // out of.
        if let Value::Tuple(written) = val.unspan()
            && written.len() == elems.len()
        {
            let written = written.clone();
            for (v, elem_tp) in written.into_iter().zip(elems.iter()) {
                self.expand_one_tuple_key(v, elem_tp, prelude, out);
            }
            return;
        }
        let holder = self.bind_tuple_operand(&val, tp, &elems, "__key_t", prelude);
        for (i, elem_tp) in elems.iter().enumerate() {
            let read = Value::TupleGet(holder, i as u16);
            self.expand_one_tuple_key(read, elem_tp, prelude, out);
        }
    }

    /// loft#882 / loft#889 — name the container a read VIEWS INTO, so the value read out
    /// of it can depend on it.
    ///
    /// A dep naming the owner is the whole reason a borrowed read is safe:
    /// `return_views_local` sees a borrow from a local and `materialize_view_return`
    /// copies the value into the return buffer BEFORE the container is freed.  A read
    /// that says nothing hands back a pointer into a store the same function frees on the
    /// way out.  (`--native` hides most of it: an empty dep list reads as OWNED there, so
    /// the assignment lowering inserts a defensive record copy — which is why such a
    /// program is deterministic garbage interpreted and correct natively.)
    ///
    /// Two container shapes need two moves.  A NAMED container — a local, a parameter, a
    /// field — is depended on directly.  An inline call producing a FRESH container has
    /// no name at parse time (`scopes.rs` lifts it into a `__lift_N` long after the
    /// materialisation decision has been made), so it is bound to a work-ref here and
    /// that is what the read depends on.  The work-ref comes from the `__ref_p2_N`
    /// sequence, which is separate from the one `ref_return` promotes out of, so a mint
    /// here cannot land on the name pass 1 left on the return buffer (loft#848).
    ///
    /// It binds on BOTH passes, and that is load-bearing rather than incidental: this
    /// dep is what tells `ref_return` the binding borrows, and a verdict that differs
    /// between the passes is worse than no verdict at all.  Skipping pass 1 read the
    /// binding as owned and renamed it ONTO the return buffer; pass 2 then saw the view
    /// and materialised — into the buffer the binding now was — so the copy read the
    /// record it had just re-minted and `e = make().f[0]; e` answered an empty one.
    ///
    /// Only a call that MINTS the container is bound — a loft-defined body whose return
    /// store is genuinely fresh.  WHICH call that is takes a step: `make_bag().h[k]` must
    /// name the BAG, because the element lives in the bag's store and not in the `h`
    /// projection's.  Naming the projection puts the bag in an inner scope, freed before
    /// the materialised copy reads it — worse than the borrow.
    ///
    /// The SUBSCRIPT is what asks, not the field read, and that placement is what keeps
    /// the binding to the reads that need it.  `return make().rows` returns the field
    /// itself, which the delivery machinery already copies out (loft#877, zt12) — binding
    /// a container there adds a holder nothing releases.  Only an element read out of the
    /// field views into the container, and only the subscript knows that is what this is.
    ///
    /// Returns the variable the read borrows from, if there is one to name.
    fn container_dep(&mut self, code: &mut Value, typedef: &Type) -> Option<u16> {
        if let Value::Var(x) = code.unspan() {
            return Some(*x);
        }
        // Reached THROUGH one or more field projections (`make_bag().h[k]`): bind the
        // ROOT call, because the element lives in the bag's store and not in the `h`
        // projection's.  Its type comes from the call's own return type, the only place
        // the bag is written down by the time the subscript asks.
        if let Some(root) = self.projection_root_mut(code) {
            let Value::Call(d_nr, _) = root.unspan() else {
                return None;
            };
            if !self.data.def(*d_nr).is_loft_defined() {
                return None;
            }
            let root_tp = self.data.def(*d_nr).returned().clone();
            return Some(self.bind_inline_container(root, &root_tp));
        }
        let Value::Call(d_nr, _) = code.unspan() else {
            return None;
        };
        if !self.data.def(*d_nr).is_loft_defined() {
            return None;
        }
        Some(self.bind_inline_container(code, typedef))
    }

    /// The innermost base of a chain of field projections, when `code` IS such a chain
    /// rooted at a CALL.  `None` for anything else, including a chain rooted at a
    /// variable — that root already has a name, and `parse_index` inherits its dep.
    fn projection_root_mut<'a>(&self, code: &'a mut Value) -> Option<&'a mut Value> {
        let Value::Call(d, args) = code.unspan_mut() else {
            return None;
        };
        // ONE list of the projection ops, shared with the @P290 bracket's witness walk
        // (`use_analysis::view_root_slots`), which asks the same question for the
        // opposite reason: this decides which inline container needs a NAME, that
        // decides which store the bracket MARKS.
        if !crate::use_analysis::is_projection_op(&self.data, *d) {
            return None;
        }
        let base = args.first_mut()?;
        match base.unspan() {
            Value::Call(bd, _) if crate::use_analysis::is_projection_op(&self.data, *bd) => {
                self.projection_root_mut(base)
            }
            Value::Call(_, _) => Some(base),
            _ => None,
        }
    }

    /// Replace `code` with `{ w = <code>; w }` and answer `w` — the work-ref that now
    /// NAMES the container, typed `tp`.
    fn bind_inline_container(&mut self, code: &mut Value, tp: &Type) -> u16 {
        let w = self.vars.work_refs_p2(tp, &mut self.lexer);
        self.vars.mark_inline_ref(w);
        let orig = std::mem::replace(code, Value::Null);
        *code = v_block(
            vec![v_set(w, orig), Value::Var(w)],
            tp.clone().depending(w),
            "inline_container",
        );
        w
    }

    pub(crate) fn parse_key(&mut self, code: &mut Value, typedef: &Type, key_types: &[Type]) {
        // detect open-start `col[..hi]` or `col[..]` before parsing expression.
        let open_start = self.lexer.peek_token("..") || self.lexer.peek_token("..=");
        let mut p = Value::Null;
        let _index_t = if open_start {
            Type::Null // from=[] → no lower bound
        } else {
            let t = self.expression(&mut p);
            // @FR-N-Store — a lookup KEY is a slot like an index: a null key reads null.
            if !self.convert_store_lenient(&mut p, &t, &key_types[0], "the key", None)
                && !self.first_pass
            {
                // A tuple key is the one place the arity is worth naming: `h[(1, 2, 3)]` on
                // a `(integer, integer)` key is a plain miscount, and "Invalid index key"
                // leaves the reader comparing the two spellings by eye.
                if let (Some(given), Some(want)) =
                    (self.tuple_elements(&t), self.tuple_elements(&key_types[0]))
                    && given.len() != want.len()
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "this collection keys on {} — a {}-element tuple, but {} were given",
                        key_types[0].name(&self.data),
                        want.len(),
                        given.len()
                    );
                } else {
                    diagnostic!(self.lexer, Level::Error, "Invalid index key");
                }
            }
            t
        };
        let known = if self.first_pass {
            Value::Null
        } else {
            self.type_info(typedef)
        };
        let mut nr = usize::from(!open_start);
        let mut key = Vec::new();
        if !open_start {
            key.push(p);
        }
        if key_types.len() > 1 {
            while self.lexer.has_token(",") {
                if nr >= key_types.len() {
                    diagnostic!(self.lexer, Level::Error, "Too many key values on index");
                    break;
                }
                let mut ex = Value::Null;
                let ex_t = self.expression(&mut ex);
                if !self.convert_store_lenient(&mut ex, &ex_t, &key_types[nr], "the key", None)
                    && !self.first_pass
                {
                    diagnostic!(self.lexer, Level::Error, "Invalid index key");
                }
                key.push(ex);
                nr += 1;
            }
        }
        if self.lexer.has_token("..") || open_start {
            // Consume "..=" if present (open_start already peeked but didn't consume)
            let inclusive = if open_start {
                self.lexer.has_token(".."); // consume the ".."
                self.lexer.has_token("=")
            } else {
                self.lexer.has_token("=")
            };
            // D-key-1: a keyed range slice yields a `for`-only iterator (`Value::Iter`).
            // In a value position it would leave an un-consumed Iter (a parse-time panic in
            // `set_loop`, or a codegen "Iter should have been rewritten" panic) — emit one
            // clean diagnostic instead.  `set_loop` tolerates the missing loop so parsing
            // reaches here rather than panicking.
            if !self.first_pass && !self.iterable_context {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a keyed range slice is a `for`-loop iterator, not a value — iterate it \
                     directly (`for x in coll[lo..hi] {{ … }}`) or materialise a vector with a \
                     comprehension (`[for x in coll[lo..hi] {{ x }}]`)"
                );
            }
            // loft#689 — a range asks the collection to walk its keys IN ORDER, which only
            // an ordered one can answer.  `hash` is unordered, and `spatial` is ordered by
            // Morton code rather than by a scalar key, so neither can step a scalar range:
            // both walked off the end of their iterator and SIGSEGV'd (open `coll[..]`
            // included — it is the same walk without bounds).  Single-key lookup `h[k]` and
            // whole-collection iteration are unaffected; they never step a range.
            if !self.first_pass {
                match typedef {
                    Type::Hash(_, _, _) => diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a range needs an ordered collection, and `hash` is unordered — use \
                         `sorted<…>` or `index<…>` for a range, look one key up with `h[key]`, \
                         or iterate the whole collection with `for x in h`"
                    ),
                    Type::Radix(_, _, _) => diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a `spatial` range is a COORDINATE slice, not a scalar one — write \
                         `s[(x1, y1)..(x2, y2)]` (the bounding box), or iterate the whole \
                         collection with `for x in s`"
                    ),
                    // `Type::Trie` is deliberately absent, not overlooked: a trie's range
                    // is a PREFIX, which is the reason the kind exists, and it never
                    // reaches here — the two `parse_key` callers route a trie subscript
                    // to `parse_trie_slice` before this branch is entered.
                    _ => {}
                }
            }
            let iter = self.create_unique("iter", &crate::data::I64);
            // A bounded range is the OTHER lowering of a keyed iteration, and this is its
            // cursor. Record it on the loop so `#remove` inside `for x in c[a..b]` reaches
            // the same local `OpStep` does (loft#1272).
            self.vars.set_loop_state_var(iter);
            let mut ls = Vec::new();
            if !self.first_pass {
                self.fill_iter(&mut ls, code, typedef, true, inclusive);
                ls.push(Value::Int(nr as i32));
                ls.append(&mut key);
            }
            // open-end — if next token is `]` or `,`, skip upper-bound expression.
            let open_end = self.lexer.peek_token("]") || self.lexer.peek_token(",");
            let mut nr = 0;
            if !open_end {
                let mut n = Value::Null;
                let n_t = self.expression(&mut n);
                if !self.convert_store_lenient(&mut n, &n_t, &key_types[0], "the key", None)
                    && !self.first_pass
                {
                    diagnostic!(self.lexer, Level::Error, "Invalid index key");
                }
                key.push(n);
                nr = 1;
            }
            if key_types.len() > 1 {
                while self.lexer.has_token(",") {
                    if nr >= key_types.len() {
                        diagnostic!(self.lexer, Level::Error, "Too many key values on index");
                        break;
                    }
                    let mut ex = Value::Null;
                    let ex_t = self.expression(&mut ex);
                    if !self.convert_store_lenient(&mut ex, &ex_t, &key_types[nr], "the key", None)
                        && !self.first_pass
                    {
                        diagnostic!(self.lexer, Level::Error, "Invalid index key");
                    }
                    key.push(ex);
                    nr += 1;
                }
            }
            ls.push(Value::Int(nr as i32));
            ls.append(&mut key);
            let start = v_set(iter, self.cl("OpIterate", &ls));
            let mut ls = vec![Value::Var(iter)];
            self.fill_iter(&mut ls, code, typedef, false, inclusive);
            // S16b: annotate the step-block with the element type, not the collection type,
            // so that IR dumps and any type-driven passes see the correct element type.
            let elem_type = match typedef {
                Type::Sorted(el, _, dep) | Type::Index(el, _, dep) => {
                    Type::Reference(*el, dep.clone())
                }
                _ => typedef.clone(),
            };
            *code = Value::Iter(
                u16::MAX,
                Box::new(start),
                Box::new(v_block(
                    vec![self.cl("OpStep", &ls)],
                    elem_type,
                    "Iterate keys",
                )),
                Box::new(Value::Null),
            );
        } else if matches!(typedef, Type::Index(_, _, _) | Type::Sorted(_, _, _))
            && key_types.len() > 1
            && nr < key_types.len()
        {
            // partial-key match — rewrite idx[k1] as idx[k1..=k1].
            // Uses the existing inclusive-range iteration path with from=till=key.
            // D-key-1: like the range branch, a partial-key match yields a `for`-only
            // iterator — reject it in a value position with the same clean diagnostic.
            if !self.first_pass && !self.iterable_context {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a keyed partial-key match is a `for`-loop iterator, not a value — iterate \
                     it directly (`for x in coll[key] {{ … }}`) or give every key field for a \
                     single-record lookup"
                );
            }
            let inclusive = true;
            let iter = self.create_unique("iter", &crate::data::I64);
            let mut ls = Vec::new();
            if !self.first_pass {
                // fill_iter calls set_loop which requires an active loop context.
                let loop_nr = self.vars.start_loop();
                self.fill_iter(&mut ls, code, typedef, true, inclusive);
                self.vars.finish_loop(loop_nr);
                ls.push(Value::Int(nr as i32));
                let from_key = key.clone();
                ls.append(&mut key);
                // till = same key values as from (inclusive prefix match)
                ls.push(Value::Int(nr as i32));
                ls.extend(from_key);
            }
            let start = v_set(iter, self.cl("OpIterate", &ls));
            let mut ls = vec![Value::Var(iter)];
            {
                let loop_nr = self.vars.start_loop();
                self.fill_iter(&mut ls, code, typedef, false, inclusive);
                self.vars.finish_loop(loop_nr);
            }
            let elem_type = match typedef {
                Type::Sorted(el, _, dep) | Type::Index(el, _, dep) => {
                    Type::Reference(*el, dep.clone())
                }
                _ => typedef.clone(),
            };
            *code = Value::Iter(
                u16::MAX,
                Box::new(start),
                Box::new(v_block(
                    vec![self.cl("OpStep", &ls)],
                    elem_type,
                    "Partial key match",
                )),
                Box::new(Value::Null),
            );
        } else {
            // A tuple key field carries several key contents; `nr` counts the FIELDS the
            // caller supplied, which is what the too-few check below is about.
            let mut prelude = Vec::new();
            let mut vals = self.expand_tuple_key_values(key, key_types, &mut prelude);
            let mut ls = vec![code.clone(), known.clone(), Value::Int(vals.len() as i32)];
            ls.append(&mut vals);
            let lookup = self.cl("OpGetRecord", &ls);
            *code = if prelude.is_empty() {
                lookup
            } else {
                // The block DELIVERS the looked-up element, so it carries the element's
                // reference type.  Typed `Void` it type-checks and then hands the caller
                // nothing — the assigned variable read a garbage DbRef.
                let elem_type = match typedef {
                    Type::Sorted(el, _, dep)
                    | Type::Index(el, _, dep)
                    | Type::Hash(el, _, dep)
                    | Type::Radix(el, _, dep)
                    | Type::Trie(el, _, dep) => Type::Reference(*el, dep.clone()),
                    other => other.clone(),
                };
                prelude.push(lookup);
                v_block(prelude, elem_type, "keyed_tuple_lookup")
            };
            if matches!(typedef, Type::Hash(_, _, _)) && nr < key_types.len() {
                diagnostic!(self.lexer, Level::Error, "Too few key fields");
            }
        }
    }

    pub(crate) fn fill_iter(
        &mut self,
        ls: &mut Vec<Value>,
        code: &mut Value,
        typedef: &Type,
        add_keys: bool,
        inclusive: bool,
    ) {
        let known = self.get_type(typedef);
        if known == u16::MAX {
            return;
        }
        let mut on;
        let arg;
        // The element type, for the arms whose `arg` is a WIDTH rather than a type —
        // `e#remove` needs the type to free what an element owns.
        let mut elem_tp = u16::MAX;
        match self.database.types[known as usize].parts {
            Parts::Index(_, _, _) => {
                on = 1;
                arg = self.database.fields(known);
            }
            Parts::Sorted(tp, _) => {
                on = 2;
                arg = self.database.size(tp);
                elem_tp = tp;
            }
            Parts::Ordered(tp, _) => {
                on = 3;
                arg = 4;
                elem_tp = tp;
            }
            Parts::Hash(_, _) | Parts::Radix(_, _) | Parts::Trie(_, _) => {
                // Route hash/radix iteration through the Ordered code as on=4.
                // The parser has substituted the iterated expression with a
                // `hash_scratch` ref to a fresh u32-stride rec-nr vector
                // (`build_rec_scratch`), so `data.pos` is always 4 and the source
                // (records) store_nr sits in the scratch header — on=4's `step`
                // yields there, which lets the scratch live in a writable store
                // when the source is read-only/exposed (expose-iteration-scratch).
                // In-place Ordered fields (a keyed struct field) stay on=3 above.
                on = 4;
                arg = 4;
            }
            _ => {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot iterate; expected vector, sorted, index, text, or range"
                );
                return;
            }
        }
        if inclusive {
            on += 128;
        }
        // Key DIRECTION is the comparator's business and nothing else's: `keys::compare`
        // reverses per descending key, so the tree `tree::put` builds is already in the
        // declared order and a forward walk of it is the declared order.  The reverse bit
        // therefore carries ONE fact — did the user write `rev(...)` — the same way it
        // does for `sorted`, whose `ordered_range_cursors` reads no sign at all
        // (@FR-Col-Order-Sign, loft#1267).
        if self.reverse_iterator {
            on += 64;
        }
        if self.reverse_iterator {
            // Do not reset here — `iterator()` calls fill_iter twice and resets after both.
        }
        ls.push(code.clone());
        ls.push(Value::Int(i32::from(on)));
        ls.push(Value::Int(i32::from(arg)));
        // What `OpRemove` is handed, which is NOT always `arg`:
        //  - Index (on 1): the COLLECTION type, so it can reach `fields()` and
        //    `remove_owned(..., tp)`;
        //  - Sorted / Ordered (on 2 / 3): the ELEMENT type, because removing an
        //    element has to free what it owns and a width cannot say what that is
        //    (loft#903).  `arg` stays the STRIDE the stepper needs.
        //  - the rest: `arg`, which `#remove` never reads (hash iteration is
        //    rejected at the `#remove` site).
        let loop_db_tp = match on & 63 {
            1 => known,
            2 | 3 => elem_tp,
            _ => arg,
        };
        self.vars.set_loop(on, loop_db_tp, code);
        if add_keys {
            // loft#689 — the descriptor list is BAKED into the operand here, but the
            // table it reads is only filled by `determine_keys` at the end of a parse.
            // A collection type first created in pass 2 (`sorted<Rec[k]>` reached here
            // before any `finish()` had seen it) therefore baked an EMPTY list, and the
            // bounded range iterator then compared against `keys[0]` on it: SIGSEGV on
            // the interpreter, an out-of-bounds in `key_compare` on `--native`.  An open
            // range never compares, which is why `coll[..]` looked fine.
            //
            // Determine this one type's keys on demand instead of waiting for the sweep.
            // It is idempotent, so the end-of-parse pass still produces the same table.
            self.database.determine_keys_for(known as usize);
            ls.push(Value::Keys(
                self.database.types[known as usize].keys.clone(),
            ));
        }
    }
}
