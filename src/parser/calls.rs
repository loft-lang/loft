// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A call by NAME: finding the definitions a name reaches and calling the one the arguments
//! select, the named-argument and receiver-slice spellings, the operator calls, and the
//! domain checks a partial math function's argument must pass.

use super::{
    DefType, Deps, Level, Parser, Position, Sign, Type, Value, callee_param_writes,
    diagnostic_format, is_op, registry_fn_hint,
};

impl Parser {
    /// Search for definitions with the given name and call that with the given parameters.
    #[allow(clippy::too_many_arguments)]
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn call(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
        named_args: &[(String, Value, Type)],
        arg_pos: &[Position],
        name_pos: &Position,
    ) -> Type {
        // Create a new list of parameters based on the current ones
        // We still need to know the types.
        let mut d_nr = if self.default && is_op(name) {
            self.data.def_nr(name)
        } else {
            // Selection over the name's overload set first (`parser::dispatch`, @PLN162), on
            // the routed argument types; a name with no set falls to today's ladder.  A
            // refusal here is the whole answer: the arguments are parsed, nothing else is owed.
            // Selection over the name's overload set first (`parser::dispatch`, @PLN162), on
            // the routed argument types.  A tie is the whole answer: the arguments are parsed
            // and nothing else is owed.  None applicable, and a name with no set, fall to
            // today's ladder — the free `n_<name>` that coexists beside a `both` set (the
            // stdlib's `exists(text)` beside `exists(File)`), then the operator map — and
            // the unknown-function site below names the set when that finds nothing either.
            let routed = self.data.routed_types(types);
            // `source == u16::MAX` is a BARE call; any other names the package it was
            // qualified with, whose own definitions alone it reaches (`@FR-Disp-Key`).
            let selection = if source == u16::MAX {
                self.select_overload(source, name, &routed)
            } else {
                self.select_qualified_overload(source, name, &routed)
            };
            let d = match selection {
                crate::parser::dispatch::Selection::One(d) => {
                    // A call inside a generic at the generic's own variable is decided again
                    // in each instance (@PLN165 B3b).
                    if let Some(tp) =
                        self.defer_set_call(code, source, name, list, types, named_args, &routed, d)
                    {
                        return tp;
                    }
                    // `Disp-Dynamic`: a position held at the enum where the set decides by
                    // variant calls the synthesised dispatcher instead.
                    self.dynamic_dispatcher(source, name, &routed, Some(d))
                        .unwrap_or(d)
                }
                sel @ crate::parser::dispatch::Selection::Ambiguous(_) => {
                    if self.first_pass {
                        return Type::Unknown(0);
                    }
                    self.report_selection(name, &routed, &sel, Some(name_pos));
                    // @P376 — a reported expression is poisoned to `never`, so the call it is
                    // an argument of does not report it a second time as "missing".
                    return Type::Never;
                }
                crate::parser::dispatch::Selection::NoneApplicable => {
                    // `Disp-Exhaustive` over the closed enum: no definition takes the static
                    // types, but the set may cover every variant tuple of the enum-held
                    // positions — then the dispatcher is total and the call is covered; a
                    // tuple it does not cover is refused naming that tuple, and the ladder
                    // answers for a name that is no dynamic site at all.
                    match self.dynamic_dispatcher(source, name, &routed, None) {
                        Some(dd) => dd,
                        None if !self.first_pass && self.reported_dynamic_refusal => {
                            self.reported_dynamic_refusal = false;
                            return Type::Never;
                        }
                        None => self.data.select_fn(source, name, types),
                    }
                }
                crate::parser::dispatch::Selection::NotDecidable => {
                    self.data.select_fn(source, name, types)
                }
            };
            // @FR-F-Surface (loft#2115) — `lib::m(x)` reaches a METHOD by its receiver's key,
            // which `Data::qualified_refusal`, asking by name, cannot see.
            if source != u16::MAX && source != self.data.source {
                self.refuse_unpublished_method(d, true);
            }
            // loft#788 — a bare CALL is ambiguous the same way a bare type is,
            // and worse: both import orders compile and RUN, answering
            // differently. The key is the mangled one, since that is what a
            // function is bound under.
            //
            // `source == MAX` is what makes it BARE. A qualified `ra::dup_name()`
            // has already said which package it means, and two packages
            // deliberately exporting one name and calling it qualified is a
            // shape that works today (#305, `n2_cdylib`) — refusing it would
            // break a program for a collision it has already resolved.
            if source == u16::MAX {
                self.refuse_ambiguous_import(&format!("n_{name}"), d);
            }
            d
        };
        // Trace point: post-find_fn dispatch state.  Captures the most
        // common debugging vantage — what name resolved to which
        // d_nr, whether it was a Generic that got skipped, and which
        // pass we're on.  Enable with `LOFT_TRACE=call`.
        crate::loft_trace!(
            call,
            "name={} types={:?} d_nr={} def_type={:?} first_pass={}",
            name,
            types,
            d_nr,
            if d_nr == u32::MAX {
                None
            } else {
                Some(self.data.def(d_nr).def_type().clone())
            },
            self.first_pass,
        );
        // skip generic templates — they are not callable directly.  A METHOD template the
        // bare lookup found (`fn head<T>(self: vector<T>)` for `head(a)`) is kept aside and
        // instantiated: a bare call reaches a method template exactly as it reaches a concrete
        // `self` method ((F-Recv)), and a free template of that name on that receiver type
        // cannot also exist ((F-OneBody)) (loft#1539).  So is a template that SELECTION chose
        // from its name's overload set (@PLN165 B3, `D-Rank`): the set is what named it, so
        // no lookup by the bare name can find it again.
        let mut method_template = u32::MAX;
        if d_nr != u32::MAX && self.data.def(d_nr).def_type() == DefType::Generic {
            if self.data.def(d_nr).name().starts_with("t_")
                || self.data.def(d_nr).is_free_overload()
            {
                method_template = d_nr;
            }
            d_nr = u32::MAX;
        }
        // @PLN115 S5 — record a free-function CALL as a Global reference at its
        // name.  Gated on the recording flag (so it is a single predictable bool
        // check on every normal compile) and pass 2 (the name has resolved).  Only
        // user free functions (`n_<name>`) are Globals here; operators (`Op…`) are
        // not user-navigable names and methods (`t_…`) resolve in fields.rs (S6).
        if self.record_resolutions
            && !self.first_pass
            && d_nr != u32::MAX
            && self.data.def(d_nr).name().starts_with("n_")
        {
            self.record(
                name_pos,
                name.chars().count() as u16,
                crate::resolution::Resolution::Global(d_nr),
            );
        }
        // Plan-17 phase 01 (A) — propagate the substituted return type
        // on first pass so receiving variables (`t = min_max(7, 3)`)
        // get a correctly-typed `Type::Tuple([…])` slot, enabling
        // `t.0` / `t.1` on the SAME pass.  Without this, first-pass
        // returned `Type::Unknown(0)`; the receiving variable stayed
        // Unknown (change_var_type is a no-op for Unknown); downstream
        // `t.0` rejected with "Expect token ;" because tuple element
        // access requires a typed Tuple receiver.  That error aborted
        // second pass entirely (lexer.token() emits errors regardless
        // of pass), so the second-pass full instantiation never ran.
        //
        // The fix splits the work: on first pass, we predict the
        // return type only (no def creation — first-pass body IR is
        // still being built and would produce a stale instantiation);
        // on second pass, the full `try_generic_instantiation`
        // creates the monomorphised def.  First-pass IR for the call
        // is `Value::Null` placeholder; second-pass re-parse builds
        // the real `Value::Call`.
        // @PLN102 arc C — generic instantiation is CALLER-SOURCE-AGNOSTIC: a
        // stdlib fn calling a generic (`sum_of`→`sum`) instantiates it exactly like
        // a user program does.  The old `&& !self.default` guard skipped the stdlib
        // parse, so a stdlib-internal generic call resolved to "Unknown function"
        // (surfaced by the step-6 dogfood).  Inert for the current stdlib — no
        // stdlib call was a generic — and inert for a non-generic unresolved name
        // (`predict_generic_return_type` / `try_generic_instantiation` return
        // Unknown/MAX with no side effect, so it falls through to "Unknown function"
        // exactly as before).
        if d_nr == u32::MAX {
            if self.first_pass {
                let predicted = if method_template == u32::MAX {
                    self.predict_generic_return_type(source, name, types)
                } else {
                    self.predict_template_return(method_template, name, types)
                };
                if !predicted.is_unknown() {
                    *code = Value::Null;
                    return predicted;
                }
            } else {
                d_nr = if method_template == u32::MAX {
                    self.try_generic_instantiation(source, name, types)
                } else {
                    self.instantiate_template(method_template, name, types)
                };
                // loft#1538 — a template refused at its declaration cannot instantiate, and
                // the declaration already says why; "Unknown function" here would point the
                // author at the call instead.  Answer the declared return, so the rest of the
                // expression types as written.
                let g_nr = if method_template == u32::MAX {
                    self.data.source_nr(source, &format!("n_{name}"))
                } else {
                    method_template
                };
                // F9 — refused naming both parameters; nothing further to say about the call.
                if d_nr == u32::MAX && self.reported_binding_clash {
                    self.reported_binding_clash = false;
                    return Type::Never;
                }
                if d_nr == u32::MAX && self.refused_templates.contains(&g_nr) {
                    *code = Value::Null;
                    return self.data.def(g_nr).returned().clone();
                }
            }
        }
        // loft#824 — the receiver the type-directed builtins below dispatch on, read
        // THROUGH a `&`.  `len` / `size` are not stdlib overloads: each arm matches
        // `types[0]` against one concrete type kind and derives its constant (a stride, a
        // width, a bookkeeping offset) from it, so a `RefVar(τ)` argument matched no arm and
        // fell out as "Unknown function" — while the same helper taking `τ` by value
        // compiled.  `&τ` is a passing MODE, not a type the user wrote: `size(v)` inside
        // `fn f(v: &vector<integer>)` asks for the vector's footprint, the same as it does
        // one signature over.  The emitted ops are unchanged — they take the argument var,
        // which carries the reference exactly as `v += [9]` in the same body already does.
        let recv_unknown = Type::Unknown(0);
        let recv: &Type = match types.first() {
            Some(Type::RefVar(inner)) => inner,
            Some(t) => t,
            None => &recv_unknown,
        };
        if d_nr != u32::MAX {
            let ret = self.call_with_named(
                code,
                d_nr,
                list,
                types,
                named_args,
                true,
                arg_pos,
                Some(name_pos),
            );
            // @PLN102 Phase 3.5 — constant-in-domain elision ("PI blocks it"): a domain-partial
            // math fn with a provably in-domain CONSTANT argument cannot be null (`sqrt(4.0)`,
            // `pow(2.0, 3.0)`, `ln(2.0)`), so peel the `τ?` its decl carries. Only under
            // LOFT_NULLFLOW, and only the constant subset (variable-arg range-tracking is deferred).
            if crate::keys::ndomain_enabled()
                && matches!(ret, Type::Optional(_))
                && self.math_arg_in_domain(name, list)
            {
                // Phase 3.5 elision: a provably-in-domain constant arg peels the `τ?`.
                ret.base().clone()
            } else if (matches!(name, "min" | "max" | "clamp") || crate::keys::nprop_enabled())
                && Self::is_null_transparent(name)
                && !matches!(ret, Type::Optional(_))
                && Self::is_non_null_scalar(&ret)
                && types.iter().any(|t| matches!(t, Type::Optional(_)))
            {
                // Phase 5.3 propagation: a null-transparent fn with a nullable arg → `τ?` + a
                // runtime guard (if any arg is null the result is null). `min`/`max`/`clamp` used
                // to carry this in hand-written `τ?` overloads, which were DEFAULT-ON (DN3), so the
                // guard runs for them in ALL modes (it replaces those overloads); the rest
                // (`abs`, `floor`, …) are the new LOFT_NULLFLOW behaviour.
                if self.first_pass {
                    Type::optional(ret)
                } else {
                    self.wrap_null_transparent(code, types, &ret)
                }
            } else {
                ret
            }
        } else if self.first_pass && !self.default {
            Type::Unknown(0)
        } else if let Some(tp) = self.call_with_slice_receiver(
            code, source, name, list, types, named_args, arg_pos, name_pos,
        ) {
            tp
        } else if name == "len"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(recv.peel_optional().0, Type::Index(_, _, _))
        {
            // P192: `len(ix)` for `ix: index<T[key]>`, dispatched here to
            // `OpLengthIndex` (`btree::count`, the header's length).
            //
            // Strict arity / named-arg gates: `len(ix, x)` or
            // `len(ix, key: 1)` should NOT route here — they would
            // fall through standard dispatch to the existing
            // `Unknown function len` error message which lists the
            // method-style alternative.
            //
            // Through the `?` as every other keyed `len` goes: an `index<…>?` answered
            // "Unknown function len".  This route bypasses `convert`, so it asks
            // `@FR-N-Store` itself, and the absent index counts 0 (`btree::count`).
            if let Some(tp) = self.index_len(code, recv, list.to_vec(), arg_pos.first()) {
                return tp;
            }
            // Type or op not registered — drop to the standard
            // error path so the user sees the same diagnostic shape
            // they get for any other unresolved `len()` call.  P07.5
            // adds a "did you mean" suffix when a similarly-named
            // user function exists.
            if let Some(s) = self.suggest_function_name(name) {
                diagnostic_at!(
                    self.lexer,
                    name_pos,
                    Level::Error,
                    code = "unknown-function",
                    "Unknown function {name} — did you mean '{s}'?"
                );
                self.lexer.suggest_last(&s);
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Mechanical,
                    title: format!("rename to `{s}`"),
                    condition: None,
                    edit: Some(crate::diagnostics::Edit {
                        line: name_pos.line,
                        col: name_pos.pos,
                        len: u32::try_from(name.len()).unwrap_or(0),
                        text: s.clone(),
                    }),
                    concept: "functions",
                    concept_ref: "@F16",
                });
            } else {
                diagnostic_at!(
                    self.lexer,
                    name_pos,
                    Level::Error,
                    "Unknown function {name}"
                );
            }
            self.reported_call(code)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(*recv, Type::Vector(_, _))
        {
            // @PLN110 1a: `size(v)` for `v: vector<T>` = element count × the
            // element's in-buffer stride (a heap element — text, nested
            // collection, linked struct — counts as its 4-byte record pointer,
            // never the target's content).  Dispatched here (not a stdlib
            // overload) because the stride is type-derived — the SAME
            // `vector_elem_iter_stride` iteration walks the buffer with — and
            // only known at parse time.  Strict arity / named-arg gates mirror
            // `len(ix)` above; `size(v, x)` falls through to the standard error.
            let op_d_nr = self.data.def_nr("OpSizeVector");
            if let Type::Vector(inner, _) = recv
                && op_d_nr != u32::MAX
            {
                let elem = (**inner).clone();
                let stride = self.vector_elem_iter_stride(&elem);
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(stride)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(*recv, Type::Reference(_, _))
        {
            // @PLN110 1b: `size(s)` for a struct `s` = its packed record size — a
            // compile-time constant (all instances of a struct type share one
            // layout).  Inline fields and inline sub-records count fully; `text` /
            // collection / reference fields count as their 4-byte stored width
            // (allocation-local).  The argument is still evaluated for its side
            // effects (the op consumes it); only its type feeds the const size.
            // A bare enum-variant value (`Circle { r: 2.0 }`) is also a
            // `Type::Reference`, to an `EnumValue` — a struct-like record — so it is
            // handled here too, reporting the variant's own packed record size.  A
            // `Type::Reference` that is NEITHER a struct NOR a variant record is not
            // a valid `size` target: emit the standard error (never a silent
            // `Unknown`, which would leave the call unresolved).
            let known = self.get_type(recv);
            let op_d_nr = self.data.def_nr("OpSizeStruct");
            if known != u16::MAX
                && op_d_nr != u32::MAX
                && (self.database.is_struct(known) || self.database.is_enum_value(known))
            {
                let sz = self.database.size(known);
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(sz)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            diagnostic_at!(
                self.lexer,
                name_pos,
                Level::Error,
                "Unknown function {name}"
            );
            self.reported_call(code)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(
                *recv,
                Type::Integer(_) | Type::Float | Type::Boolean | Type::Character | Type::Single
            )
        {
            // @PLN110 1e: `size(x)` for a scalar `x` = its storage width — a
            // compile-time constant (L-Scalar / L-Narrow).  The arg is evaluated
            // for its side effects (every scalar is one 8-byte eval-stack slot,
            // which the op consumes) but only its type feeds the const width.
            // Integers go through the FINISHED type (`get_type` → `database.size`)
            // so a narrow / `forced_size` width (u8 1, u16 2, i32 4) is honoured —
            // `Type::size` reads the value range only and would over-report i32 as
            // 8.  The other scalars have a fixed width via `element_size`
            // (boolean 1, character / single 4, float 8).
            let known = self.get_type(recv);
            let sz: u16 = if matches!(*recv, Type::Integer(_)) && known != u16::MAX {
                self.database.size(known)
            } else {
                crate::data::element_stack_size(recv) as u16
            };
            let op_d_nr = self.data.def_nr("OpSizeScalar");
            if op_d_nr != u32::MAX {
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(sz)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(*recv, Type::Sorted(_, _, _))
        {
            // @PLN110 1d: a `sorted` shares vector's length-prefixed buffer, so its
            // size is that buffer = element count × the element's in-buffer stride
            // (identical to `size(vector)` — reuse `OpSizeVector`).
            let elem = recv.content();
            let stride = self.vector_elem_iter_stride(&elem);
            let op_d_nr = self.data.def_nr("OpSizeVector");
            if op_d_nr != u32::MAX {
                let mut args = list.to_vec();
                args.push(Value::Int(i32::from(stride)));
                *code = Value::Call(op_d_nr, args);
                return crate::data::I64.clone();
            }
            Type::Unknown(0)
        } else if name == "size"
            && types.len() == 1
            && named_args.is_empty()
            && matches!(*recv, Type::Enum(_, _, _))
        {
            // @PLN110 enums: a SIMPLE enum (no data-carrying variant) is a 1-byte
            // inline discriminant — scalar-like, reuse `OpSizeScalar` (an 8-byte eval
            // slot it consumes); a DATA enum is a `DbRef` to a record (1-byte tag +
            // the max variant's packed fields) — struct-like, reuse `OpSizeStruct`.
            // `database.size` gives the right width either way (1 vs the record size);
            // only the op differs, because the two are delivered differently (inline
            // value vs reference). The arg is evaluated; only its type feeds the const.
            let known = self.get_type(recv);
            if known != u16::MAX {
                let sz = self.database.size(known);
                let op_name = if matches!(*recv, Type::Enum(_, true, _)) {
                    "OpSizeStruct"
                } else {
                    "OpSizeScalar"
                };
                let op_d_nr = self.data.def_nr(op_name);
                if op_d_nr != u32::MAX {
                    let mut args = list.to_vec();
                    args.push(Value::Int(i32::from(sz)));
                    *code = Value::Call(op_d_nr, args);
                    return crate::data::I64.clone();
                }
            }
            Type::Unknown(0)
        } else if types.iter().any(|t| matches!(t.base(), Type::Never))
            && self.lexer.diagnostics().error_count() > 0
        {
            // An argument poisoned by a reported error (@P376): the error named the cause,
            // and no overload can be judged against a value that is not there — "Unknown
            // function len" for `len(<a binding that failed>)` would name a second one.
            // Guarded on the report, so a diverging argument alone never hides a real
            // unknown function.
            Type::Never
        } else if self.vars.name_exists(name)
            && matches!(self.vars.tp(self.vars.var(name)).base(), Type::Never)
        {
            // A local whose binding already failed holds no value (its type is poisoned
            // `never`, the way an errored assignment's is): the binding's diagnostic names
            // the cause, and "Unknown function" for calling it would name a second one.
            self.reported_call(code)
        } else {
            // A name with an overload set, called at a generic's type VARIABLE, where no member
            // takes it (@PLN165 B3b: only a template member can take a variable, through its
            // bounds): the set's refusal names what was passed and what is declared, which a
            // "method call" message about a free call would not.
            let routed = self.data.routed_types(types);
            let set_refuses = types.iter().any(|t| self.data.mentions_type_var(t))
                && self
                    .select_overload(source, name, &routed)
                    .is_none_applicable();
            if set_refuses {
                let sel = crate::parser::dispatch::Selection::NoneApplicable;
                self.report_selection(name, &routed, &sel, Some(name_pos));
            } else if let Some(tv_name) = types.first().and_then(|t| self.generic_type_name(t)) {
                // generic-specific error for method calls on T.
                let tv_name = crate::data::Data::type_var_spelling(tv_name);
                diagnostic_at!(
                    self.lexer,
                    name_pos,
                    Level::Error,
                    "generic type {tv_name}: method call requires a concrete type",
                );
            } else {
                // QUALITY-history.md 6c (follow-on): when a free call fails but a method
                // `t_<LEN><Type>_<name>` exists on some other type, tell the
                // user to call it as a method.  Mirror image of the
                // field-access hint that covers the method→free direction.
                // P07.5: when no method receiver is found EITHER, fall back to
                // a similar-name suggestion across all user functions.
                let (method_types, from_stdlib) = self.find_method_receivers(name);
                // `Disp-Exhaustive` (@PLN162): the name has an overload set, none of its
                // definitions takes these argument types, and the ladder found nothing else
                // either.  Name what was passed and what is declared.
                let routed = self.data.routed_types(types);
                let none_applicable = matches!(
                    self.select_overload(source, name, &routed),
                    crate::parser::dispatch::Selection::NoneApplicable
                );
                if none_applicable && method_types.is_empty() {
                    let sel = crate::parser::dispatch::Selection::NoneApplicable;
                    self.report_selection_with(name, &routed, &sel, Some(name_pos), list);
                } else if method_types.is_empty() {
                    // @PLN13 phase 6 (diagnostics slice): the name may simply be
                    // unimported rather than wrong.  An EXACT hit in a published
                    // package outranks the fuzzy same-name guess below — `rand`
                    // is not a misspelling of a local function, it lives in
                    // `random` — so this is offered first.  Bare calls still do
                    // not resolve; this only replaces a dead end with the two
                    // ways to say what was meant.
                    // loft#826 — a name declared by the file that `use`d this one
                    // is not a misspelling of anything, and it outranks both the
                    // registry hint and the fuzzy guess: those answer "where else
                    // could this live?", and here we already know exactly where it
                    // lives and why it does not reach.  The fuzzy guess is what
                    // turned this into "did you mean 'move'?".
                    // A lambda on the right-hand side of the name's own FIRST binding: the
                    // variable holds nothing when the lambda captures, so there is no function
                    // value to call yet (`first_bind_targets`).
                    if self.first_bind_targets.iter().any(|t| t == name) {
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            "'{name}' is not bound yet — a lambda captures its variables when it \
                             is created, so it cannot call the variable it is being assigned to; \
                             for recursion declare a file-scope 'fn {name}(…)'"
                        );
                    } else if let Some(note) = self.importer_boundary_note(&format!("n_{name}")) {
                        diagnostic_at!(self.lexer, name_pos, Level::Error, "{note}");
                    } else if let Some(msg) =
                        self.data
                            .import_cure("Unknown function", name, self.data.source)
                    {
                        diagnostic_at!(self.lexer, name_pos, Level::Error, "{msg}");
                    } else if let Some(hint) =
                        registry_fn_hint(name, &self.data.resolved_libraries())
                    {
                        diagnostic_at!(self.lexer, name_pos, Level::Error, "{hint}");
                    } else if let Some(s) = self.suggest_function_name(name) {
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            code = "unknown-function",
                            "Unknown function {name} — did you mean '{s}'?"
                        );
                        self.lexer.suggest_last(&s);
                        self.lexer.fix_last(crate::diagnostics::Fix {
                            kind: crate::diagnostics::FixKind::Mechanical,
                            title: format!("rename to `{s}`"),
                            condition: None,
                            edit: Some(crate::diagnostics::Edit {
                                line: name_pos.line,
                                col: name_pos.pos,
                                len: u32::try_from(name.len()).unwrap_or(0),
                                text: s.clone(),
                            }),
                            concept: "functions",
                            concept_ref: "@F16",
                        });
                    } else {
                        diagnostic_at!(
                            self.lexer,
                            name_pos,
                            Level::Error,
                            "Unknown function {name}"
                        );
                    }
                } else {
                    let receivers = method_types.join(" / ");
                    // loft#850 — say who declared it as a method, and only say "stdlib"
                    // when that is true. A hint that blames the stdlib for a name a
                    // `use`d package declared sends the reader to the wrong source file.
                    let declared_by = if from_stdlib {
                        format!("stdlib declared `{name}` as a method")
                    } else {
                        // A method answers the bare spelling too ((F-Recv)) — for a receiver of
                        // its type, once the name is in scope — so "not a free function" was
                        // wrong for every method, and pointed away from the import it needed.
                        format!(
                            "`{name}` is a method: `{name}(x, …)` reaches it for a receiver of \
                             that type once the name is in scope — a package's method is \
                             imported by name, `use <package>::({name})`"
                        )
                    };
                    diagnostic_at!(
                        self.lexer,
                        name_pos,
                        Level::Error,
                        "Unknown function {name} — did you mean the method `x.{name}(…)` on {receivers}? ({declared_by}; see LOFT_DATA.md § Methods and function calls)"
                    );
                }
            }
            self.reported_call(code)
        }
    }

    /// loft#1728, @FR-Slice-Value — a call whose RECEIVER is a vector slice (`len(v[a..b])`)
    /// and which resolves for no definition at the iterator: the receiver is a vector-typed
    /// position like any other, so it is materialised as one (`iterator_as_vector`, the one
    /// home) and the call resolved again.  Asked only where resolution would otherwise report
    /// an unknown function, so no call that resolved before resolves differently.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call_with_slice_receiver(
        &mut self,
        code: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
        named_args: &[(String, Value, Type)],
        arg_pos: &[Position],
        name_pos: &Position,
    ) -> Option<Type> {
        let first = types.first()?;
        let Type::Iterator(elm, _) = first.base() else {
            return None;
        };
        let want = Type::Vector(elm.clone(), Deps::none());
        let mut args = list.to_vec();
        let vec_tp = self.iterator_as_vector(args.first_mut()?, &types[0], &want)?;
        let mut arg_types = types.to_vec();
        arg_types[0] = vec_tp;
        Some(self.call(
            code, source, name, &args, &arg_types, named_args, arg_pos, name_pos,
        ))
    }

    /// Is `code` a top-level vector constant's use site — a view of the write-locked constant
    /// store, bound to its skip-free `_const_view_N` anchor (`Parser::parse_var`'s constant
    /// arm, loft#1936)?  The one test for the shape, so the copy decisions that must treat it
    /// as a whole-value read (`(B-Copy)`, loft#1686 and loft#1729) cannot drift from it.
    pub(crate) fn is_const_view(&self, code: &Value) -> bool {
        let op = self.data.def_nr("OpConstRef");
        match code.unspan() {
            Value::Call(d, _) => *d == op,
            Value::Block(bl) if bl.name == "const_view" => matches!(
                bl.operators.first().map(Value::unspan),
                Some(Value::Set(_, v)) if matches!(v.unspan(), Value::Call(d, _) if *d == op)
            ),
            _ => false,
        }
    }

    /// Does argument `nr` of a call to `callee` hand a top-level constant's view to a
    /// parameter the callee may WRITE, so that it must travel as a copy (loft#1729)?
    ///
    /// Asked on both passes with the same answer, because the copy mints its temporaries on
    /// both (`materialize_collection_value`): the callee's write fact is read only for a
    /// callee parsed BEFORE the caller — pass 1 has its body then, and pass 2 the same one —
    /// and a callee parsed at or after it (a forward call, a recursive one) is assumed to
    /// write.  A `const` or `&` parameter and a native callee never copy; a literal-bodied
    /// function's result already declines at a user call (`const_fn`).
    pub(super) fn constant_arg_needs_copy(
        &self,
        callee: u32,
        nr: usize,
        arg: &Value,
        param: &Type,
    ) -> bool {
        if !self.is_const_view(arg) {
            return false;
        }
        if !matches!(param.base(), Type::Vector(_, _)) || self.context == u32::MAX {
            return false;
        }
        let def = self.data.def(callee);
        if !def.is_loft_defined() || def.attributes().get(nr).is_none_or(|a| a.value_const) {
            return false;
        }
        if callee >= self.context {
            return true;
        }
        let mut cache = crate::fxhash::FxHashMap::default();
        callee_param_writes(callee, &self.data, &mut cache)
            .get(nr)
            .copied()
            .unwrap_or(true)
    }

    /// The type of a call whose failure was reported (or whose callee's binding failed):
    /// poisoned to `never`, as every reporting site leaves it, so the operator or call it is an operand of reads a
    /// value that is not there and does not report it again as a missing argument naming an
    /// internal opcode (loft#1719): the slot IS filled, by an expression that errored, and
    /// @FR-F-Arity refuses only an unfilled one.  Its code is cleared too: a failed call writes none, and
    /// the buffer still holds what the caller put there — an assignment's own target, so
    /// `x = nofn(v) + 1` lowered to `x + 1`, typed as an integer, and escaped the poison.
    /// The first pass reports nothing and keeps `unknown`, so a function declared further
    /// down still types there.
    pub(super) fn reported_call(&self, code: &mut Value) -> Type {
        if self.first_pass {
            Type::Unknown(0)
        } else {
            *code = Value::Null;
            Type::Never
        }
    }

    /// The constant `f64` value of `v` if it is one — a literal, or (case-C residual) any
    /// expression that `const_eval` reduces to a number, e.g. the call-valued consts `PI` /
    /// `E` (`OpMathPiFloat()` → π). So a math arg / divisor written in terms of `PI` folds
    /// like a literal would.
    #[allow(clippy::cast_precision_loss)]
    pub(super) fn const_f64(&self, v: Option<&Value>) -> Option<f64> {
        match v?.unspan() {
            Value::Float(f) => Some(*f),
            Value::Single(f) => Some(f64::from(*f)),
            Value::Int(n) => Some(f64::from(*n)),
            Value::Long(n) => Some(*n as f64),
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Float(f)) => Some(f),
                Some(Value::Single(f)) => Some(f64::from(f)),
                Some(Value::Int(n)) => Some(f64::from(n)),
                Some(Value::Long(n)) => Some(n as f64),
                _ => None,
            },
        }
    }

    /// @PLN102 Phase 3.5 — is a call to a domain-partial math fn provably IN its real domain,
    /// from CONSTANT arguments alone? Then its result is non-null and the `τ?` elides. The
    /// constant subset of the "provably-fits" elision (variable-arg range-tracking is deferred).
    // These are exact domain-boundary checks, not approximate arithmetic: a `Long` too large to
    // represent exactly in `f64` is trivially outside any bounded domain, and `base != 1.0` is the
    // genuine boundary (a `log` base of exactly 1 is undefined), so the two lints do not apply.
    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
    pub(super) fn math_arg_in_domain(&self, name: &str, args: &[Value]) -> bool {
        let a0 = self.const_f64(args.first());
        let a1 = self.const_f64(args.get(1));
        let const_ok = match name {
            "sqrt" => matches!(a0, Some(x) if x >= 0.0),
            "asin" | "acos" => matches!(a0, Some(x) if (-1.0..=1.0).contains(&x)),
            "ln" | "log2" | "log10" => matches!(a0, Some(x) if x > 0.0),
            // `log(x, base)`: x > 0 and a valid base (> 0, ≠ 1).
            "log" => {
                matches!(a0, Some(x) if x > 0.0) && matches!(a1, Some(b) if b > 0.0 && b != 1.0)
            }
            // `pow(base, exp)`: a non-negative base is always defined; a negative base only when
            // the exponent is a whole number (`pow(-2, 3)` = -8, but `pow(-2, 0.5)` = null).
            "pow" => matches!(a0, Some(b) if b >= 0.0) || matches!(a1, Some(e) if e.fract() == 0.0),
            _ => false,
        };
        if const_ok {
            return true;
        }
        // @PLN102 case B (soften-nullflow-discharge.md) — beyond the constant subset, prove the
        // argument is in-domain from an EXPRESSION via the sign lattice (`sqrt(a*a + b*b)`,
        // `sqrt(max(x, 0.01))`), or from an enclosing GUARD via `domain_sign`'s var arm.
        // Default-ON since the B5 flip; `LOFT_NO_MATH_DOMAIN=1` is the escape hatch and reverts
        // BOTH licences to the constant-only elision, which is the A/B for either.
        if !crate::keys::math_domain_enabled() {
            return false;
        }
        let base_sign = args.first().map_or(Sign::Unknown, |v| self.domain_sign(v));
        match name {
            // sqrt / pow-base need arg ≥ 0; ln / log need arg > 0 (strict).
            "sqrt" => matches!(base_sign, Sign::NonNeg | Sign::Pos),
            "ln" | "log2" | "log10" => base_sign == Sign::Pos,
            "pow" => matches!(base_sign, Sign::NonNeg | Sign::Pos),
            // `log(x, base)`: x > 0 (lattice) and a valid CONSTANT base (upper-bound-free lattice
            // can't prove base ≠ 1, so keep base constant).
            "log" => base_sign == Sign::Pos && matches!(a1, Some(b) if b > 0.0 && b != 1.0),
            // asin / acos need a TWO-sided bound [-1, 1] — the interval pass proves it for the
            // real cases (`sin`/`cos` outputs, `clamp(_, -1, 1)`, the `min(max(e,-1),1)` clamp).
            "asin" | "acos" => {
                let (lo, hi) = args
                    .first()
                    .map_or((f64::NEG_INFINITY, f64::INFINITY), |v| self.pm_bounds(v));
                lo >= -1.0 && hi <= 1.0
            }
            _ => false,
        }
    }

    /// @PLN102 case B — a small interval-bounds pass for the TWO-sided `asin`/`acos` domain
    /// `[-1, 1]` (the sign lattice gives only a lower bound). Returns a provable `[lo, hi]`;
    /// `±∞` means unbounded. Only the constructs that actually keep a value in range: constants,
    /// `sin`/`cos` outputs, `clamp(e, lo, hi)` with constant bounds, and `min`/`max` (so the
    /// manual `min(max(e, -1.0), 1.0)` clamp is proved). Everything else → unbounded (matched by
    /// exact stdlib def name; `OpMinFloat` is subtraction, not `min`, and is deliberately unmatched).
    #[allow(clippy::cast_precision_loss, clippy::float_cmp)]
    pub(super) fn pm_bounds(&self, v: &Value) -> (f64, f64) {
        let open = (f64::NEG_INFINITY, f64::INFINITY);
        match v.unspan() {
            Value::Float(f) => (*f, *f),
            Value::Single(f) => (f64::from(*f), f64::from(*f)),
            Value::Int(n) => (f64::from(*n), f64::from(*n)),
            Value::Long(n) => (*n as f64, *n as f64),
            Value::Call(d, args) => {
                let nm = self.data.def(*d).name.as_str();
                // Unary negation (`-1.0` parses to `OpMinSingleFloat(1.0)`): flip + swap bounds,
                // so a negated literal reaches `clamp`/`min`/`max` as a real constant bound.
                if args.len() == 1 && matches!(nm, "OpMinSingleFloat" | "OpMinSingleSingle") {
                    let (lo, hi) = self.pm_bounds(&args[0]);
                    return (-hi, -lo);
                }
                if matches!(
                    nm,
                    "t_5float_sin" | "t_6single_sin" | "t_5float_cos" | "t_6single_cos"
                ) {
                    return (-1.0, 1.0);
                }
                if args.len() == 3 && matches!(nm, "t_5float_clamp" | "t_6single_clamp") {
                    // clamp(e, lo, hi) ∈ [lo, hi] when lo/hi are constants and lo ≤ hi.
                    let (llo, lhi) = self.pm_bounds(&args[1]);
                    let (hlo, hhi) = self.pm_bounds(&args[2]);
                    if llo == lhi && hlo == hhi && llo <= hhi {
                        return (llo, hhi);
                    }
                    return open;
                }
                if args.len() == 2 && matches!(nm, "t_5float_min" | "t_6single_min") {
                    let (al, ah) = self.pm_bounds(&args[0]);
                    let (bl, bh) = self.pm_bounds(&args[1]);
                    return (al.min(bl), ah.min(bh));
                }
                if args.len() == 2 && matches!(nm, "t_5float_max" | "t_6single_max") {
                    let (al, ah) = self.pm_bounds(&args[0]);
                    let (bl, bh) = self.pm_bounds(&args[1]);
                    return (al.max(bl), ah.max(bh));
                }
                open
            }
            _ => open,
        }
    }

    /// @PLN102 case B — the sign / lower-bound lattice over a PURE float/single expression:
    /// is its value provably `> 0` (`Pos`), `≥ 0` (`NonNeg`), or unknown? Conservative — the
    /// default is `Unknown`, and only exact, sound transfer functions promote (a square is ≥ 0,
    /// a sum of non-negatives is ≥ 0, `abs`/`sqrt` are ≥ 0, `max` takes the stronger bound).
    /// Node kinds are matched by their EXACT stdlib def name (`OpMulFloat`, `t_5float_max`, …),
    /// never a suffix, so a user method can't be mistaken for one. Anything unrecognised → Unknown.
    /// ⚠ **One question, three decoders.**  `@FR-N-Domain` promises a SINGLE elision —
    /// "provably in-domain (constant / range / guard)" — over three families of partial
    /// operation, and it is answered here for the domain-partial math functions, in
    /// `Parser::index_provably_fit` for `v[i]`, and in `Parser::divisor_provably_nonzero` /
    /// `divisor_proof_from_condition` for `÷0`.  Each carries its own set of admissible
    /// spellings, and none reads the others.
    ///
    /// The flow-GUARD licence enters here too, and that is deliberate rather than incidental:
    /// a comparison against zero contributes a `Sign` for the slot (`math_sign_proven`, read by
    /// the `Value::Var` arm below), so a guard COMPOSES with this lattice instead of sitting
    /// beside it as a second mechanism.  It is what makes `ln` given only `x >= 0.0` still
    /// refuse — `NonNeg` is not `Pos`, and the lattice says so without a special case.
    /// `formal/types-history.md` § D-Domain-Guard (opened 2026-09-08, closed 2026-09-12: the
    /// owner ruled the lattice widens rather than the rule narrowing).
    pub(super) fn domain_sign(&self, v: &Value) -> Sign {
        fn of_const(x: f64) -> Sign {
            if x > 0.0 {
                Sign::Pos
            } else if x == 0.0 {
                Sign::NonNeg
            } else {
                Sign::Unknown
            }
        }
        match v.unspan() {
            Value::Float(f) => of_const(*f),
            #[allow(clippy::cast_precision_loss)]
            Value::Single(f) => of_const(f64::from(*f)),
            Value::Int(n) => of_const(f64::from(*n)),
            #[allow(clippy::cast_precision_loss)]
            Value::Long(n) => of_const(*n as f64),
            // A bare variable is Unknown UNLESS an enclosing guard proved its sign
            // (`@FR-N-Domain`'s third licence).  Latest proof wins: the stack is pushed
            // in parse order, so an inner guard shadows an outer one for the same slot.
            Value::Var(v) => self
                .math_sign_proven
                .iter()
                .rev()
                .find(|(slot, _)| slot == v)
                .map_or(Sign::Unknown, |(_, sg)| *sg),
            Value::Call(d_nr, args) => {
                let nm = self.data.def(*d_nr).name.as_str();
                if args.len() == 2 && matches!(nm, "OpMulFloat" | "OpMulSingle") {
                    // A square (`a * a`, structurally identical operands) is ≥ 0 regardless of
                    // sign; otherwise combine signs (a non-null float is never the null sentinel,
                    // so no null leaks through a product).
                    if args[0].unspan() == args[1].unspan() {
                        return Sign::NonNeg;
                    }
                    return match (self.domain_sign(&args[0]), self.domain_sign(&args[1])) {
                        (Sign::Pos, Sign::Pos) => Sign::Pos,
                        (Sign::Pos | Sign::NonNeg, Sign::Pos | Sign::NonNeg) => Sign::NonNeg,
                        _ => Sign::Unknown,
                    };
                }
                if args.len() == 2 && matches!(nm, "OpAddFloat" | "OpAddSingle") {
                    return match (self.domain_sign(&args[0]), self.domain_sign(&args[1])) {
                        (Sign::Pos, Sign::Pos | Sign::NonNeg) | (Sign::NonNeg, Sign::Pos) => {
                            Sign::Pos
                        }
                        (Sign::NonNeg, Sign::NonNeg) => Sign::NonNeg,
                        _ => Sign::Unknown,
                    };
                }
                // `abs(x)` and `sqrt(x)` are ≥ 0 by definition (exact stdlib names only).
                if matches!(
                    nm,
                    "t_5float_abs" | "t_6single_abs" | "t_5float_sqrt" | "t_6single_sqrt"
                ) {
                    return Sign::NonNeg;
                }
                // `max(a, b)` ≥ each operand, so its lower bound is the STRONGER of the two.
                if args.len() == 2 && matches!(nm, "t_5float_max" | "t_6single_max") {
                    let (a, b) = (self.domain_sign(&args[0]), self.domain_sign(&args[1]));
                    return if a == Sign::Pos || b == Sign::Pos {
                        Sign::Pos
                    } else if a == Sign::NonNeg || b == Sign::NonNeg {
                        Sign::NonNeg
                    } else {
                        Sign::Unknown
                    };
                }
                // `min(a, b)` ≤ each, so its lower bound is the WEAKER of the two.
                if args.len() == 2 && matches!(nm, "t_5float_min" | "t_6single_min") {
                    let (a, b) = (self.domain_sign(&args[0]), self.domain_sign(&args[1]));
                    return if a == Sign::Unknown || b == Sign::Unknown {
                        Sign::Unknown
                    } else if a == Sign::NonNeg || b == Sign::NonNeg {
                        Sign::NonNeg
                    } else {
                        Sign::Pos
                    };
                }
                Sign::Unknown
            }
            _ => Sign::Unknown,
        }
    }

    /// @PLN102 Phase 5.3 — a NULL-TRANSPARENT stdlib scalar fn: `f(…, null, …) = null`. A nullable
    /// argument propagates to a nullable result. One general list drives the [`Self::wrap_null_transparent`]
    /// guard, replacing the per-function `τ?` overloads. (`sqrt`/`ln`/… already return `τ?`, so they
    /// are not here.)
    pub(super) fn is_null_transparent(name: &str) -> bool {
        matches!(
            name,
            "abs"
                | "min"
                | "max"
                | "clamp"
                | "floor"
                | "ceil"
                | "round"
                | "sin"
                | "cos"
                | "tan"
                | "atan"
                | "exp"
        )
    }

    /// @PLN102 Phase 5.3 — the GENERAL null-propagation algorithm. A null-transparent call `f(a, b)`
    /// with one or more nullable args becomes `{ t = a; …; if (t not null && …) { f(t, …) } else
    /// { null } }` typed `τ?`. Each nullable arg is evaluated ONCE into a temp (used in both the
    /// null-check and the call). This is the general form of what the `min`/`max` `τ?` overloads did
    /// by hand — floats propagate NaN on their own, but integer `max`/`abs` need this guard, so it
    /// runs uniformly. Second-pass only (the caller returns the `τ?` type in the first pass).
    pub(super) fn wrap_null_transparent(
        &mut self,
        code: &mut Value,
        arg_types: &[Type],
        ret_base: &Type,
    ) -> Type {
        let opt = Type::optional(ret_base.clone());
        let mut sets: Vec<Value> = Vec::new();
        let mut checks: Vec<(u16, Type)> = Vec::new();
        if let Value::Call(_d, args) = code.unspan_mut() {
            for (i, at) in arg_types.iter().enumerate() {
                if matches!(at, Type::Optional(_)) && i < args.len() {
                    let tmp = self.create_unique("_ntp", at);
                    let orig = std::mem::replace(&mut args[i], Value::Var(tmp));
                    sets.push(crate::data::v_set(tmp, orig));
                    checks.push((tmp, at.base().clone()));
                }
            }
        } else {
            return opt;
        }
        if checks.is_empty() {
            return opt;
        }
        let mut inner = code.clone();
        for (tmp, base) in checks.iter().rev() {
            let mut is_not_null = Value::Var(*tmp);
            self.convert(&mut is_not_null, base, &Type::Boolean);
            let null_arm = self.null(ret_base);
            inner = crate::data::v_if(is_not_null, inner, null_arm);
        }
        sets.push(inner);
        *code = crate::data::v_block(sets, opt.clone(), "null_transparent");
        opt
    }

    // @F17 — resolve named args into positional slots + fill omitted defaults
    /// Resolve named arguments into positional slots, then delegate to `call_nr`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn call_with_named(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        positional: &[Value],
        pos_types: &[Type],
        named: &[(String, Value, Type)],
        is_method: bool,
        arg_pos: &[Position],
        // The call NAME's position, forwarded to `call_nr` for the arc-C steer caret.
        name_pos: Option<&Position>,
    ) -> Type {
        let tp = self.call_with_named_inner(
            code, d_nr, positional, pos_types, named, is_method, arg_pos, name_pos,
        );
        // @PLN187 — recorded AFTER: a generic's instance parses its body in here, and the
        // calls in that body are not this one.
        self.last_called = d_nr;
        tp
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn call_with_named_inner(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        positional: &[Value],
        pos_types: &[Type],
        named: &[(String, Value, Type)],
        is_method: bool,
        arg_pos: &[Position],
        name_pos: Option<&Position>,
    ) -> Type {
        if named.is_empty() {
            return self.call_nr(
                code, d_nr, positional, pos_types, is_method, arg_pos, name_pos,
            );
        }
        // Build full argument vector with named args placed at the correct indices.
        let n_params = self.data.attributes(d_nr);
        let mut args = vec![Value::Null; n_params];
        let mut arg_types = vec![Type::Unknown(0); n_params];
        // Place positional args first.
        for (i, (val, tp)) in positional.iter().zip(pos_types.iter()).enumerate() {
            if i < n_params {
                args[i] = val.clone();
                arg_types[i] = tp.clone();
            }
        }
        let pos_count = positional.len();
        // Place named args by looking up parameter names.
        for (name, val, tp) in named {
            let idx = self.data.attr(d_nr, name);
            if idx == usize::MAX {
                if !self.first_pass {
                    diagnostic!(self.lexer, Level::Error, "Unknown parameter '{name}'");
                }
                continue;
            }
            if idx < pos_count {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Parameter '{name}' already provided as positional argument {idx}"
                    );
                }
                continue;
            }
            if args[idx] != Value::Null {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Duplicate named argument '{name}'"
                    );
                }
                continue;
            }
            args[idx] = val.clone();
            arg_types[idx] = tp.clone();
        }
        // Trim trailing Null args — add_defaults will fill them.
        let mut last_provided = args.len();
        while last_provided > 0 && args[last_provided - 1] == Value::Null {
            last_provided -= 1;
        }
        args.truncate(last_provided);
        arg_types.truncate(last_provided);
        // Named args are reordered into parameter slots, so the parse-order
        // `arg_pos` no longer aligns; fall back to the cursor for the rare
        // named-mismatch case.
        self.call_nr(code, d_nr, &args, &arg_types, is_method, &[], name_pos)
    }

    pub(super) fn single_op(&mut self, op: &str, f: Value, t: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f], &[t]);
        code
    }

    pub(super) fn conv_op(
        &mut self,
        op: &str,
        f: Value,
        n: Value,
        f_tp: Type,
        n_tp: Type,
    ) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f, n], &[f_tp, n_tp]);
        code
    }

    pub(super) fn op(&mut self, op: &str, f: Value, n: Value, t: Type) -> Value {
        let mut code = Value::Null;
        self.call_op(&mut code, op, &[f, n], &[t.clone(), t]);
        code
    }

    pub(super) fn call_op(
        &mut self,
        code: &mut Value,
        op: &str,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        let tp = self.call_op_as(code, op, op, list, types);
        if matches!(op, "==" | "!=")
            && !self.first_pass
            && crate::env_once!(std::env::var_os("LOFT_TRACE_EQ_IDENTITY").is_some())
        {
            self.trace_eq_identity(code, op, types);
        }
        tp
    }

    /// Call a specific definition
    #[allow(clippy::too_many_arguments)]
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn call_nr(
        &mut self,
        code: &mut Value,
        d_nr: u32,
        list: &[Value],
        types: &[Type],
        report: bool,
        arg_pos: &[Position],
        // The call NAME's position, when the caller knows it (a free-function call
        // via `call()`).  The @PLN102 arc-C steer points its caret here and carries a
        // `codeAction` suggestion, so the quick-fix replaces the right token; `None`
        // (a method / operator path) keeps the cursor caret and offers no quick-fix.
        name_pos: Option<&Position>,
    ) -> Type {
        // `Disp-Exhaustive` (@PLN162): a call reaching @F20's synthesised dispatcher is a
        // `match` on the enum, and a variant with no implementation is refused here — both
        // call spellings, and every receiver spelling, arrive at this one site.
        if !self.first_pass
            && d_nr != u32::MAX
            && self.data.def(d_nr).synthetic == Some("enum_dispatcher")
        {
            self.refuse_uncovered_variants(d_nr);
        }
        // @PLN102 pre-freeze — `OpEqBool`/`OpNeBool` are BOOLEAN (in)equality; they must
        // not be the implicit truthiness fallback for mismatched types.  Without this,
        // `5 == "banana"` resolves as `OpEqBool(OpConvBoolFromInt(5),
        // OpConvBoolFromText("banana"))` = `true == true` = **true** (likewise
        // `float == text`, `char == float`, `bool == text`, and their `!=` twins).  These
        // reach here via THREE call_op sub-paths (find_op_method, the `possible` loop, and
        // find_fn — the last resolves the boolean's own operator), so the guard lives at
        // this single chokepoint they all share: refuse the boolean (in)equality op unless
        // both operands are genuinely boolean, returning "no match" so the caller falls
        // through to a numeric/same-type op or the "No matching operator" reject (as the
        // ordering operators `<`/`<=`/… already do).  A `null` operand is EXEMPT: a bare
        // `boolean?`/`integer?`/… `== null` null-check legitimately lowers through the
        // boolean op (no separate bool-null branch upstream), so only reject when BOTH
        // operands are concrete non-null and not both boolean.
        if matches!(self.data.def(d_nr).name(), "OpEqBool" | "OpNeBool") && types.len() >= 2 {
            let (a, b) = (types[0].base(), types[1].base());
            // Only a VALUE-vs-value mismatch is the truthiness bug: in-band scalars
            // plus enums (`Color.Green == 1` truthiness-coerces both).  A reference /
            // heap operand (`DT? == DT?`, a nullable struct-ref) legitimately reaches
            // OpEqBool as its null-ness comparison — blocking it there would strand it
            // with "No matching operator" (there is no OpEqRef coercion for it).  So
            // require BOTH operands to be value types before rejecting.
            let value = |t: &Type| {
                matches!(
                    t,
                    Type::Integer(_)
                        | Type::Float
                        | Type::Single
                        | Type::Text(_)
                        | Type::Character
                        | Type::Boolean
                        | Type::Enum(..)
                )
            };
            let both_bool = matches!(a, Type::Boolean) && matches!(b, Type::Boolean);
            // A boolean-vs-integer comparison is rejected UPSTREAM (parser/operators.rs)
            // with a bespoke "a boolean is true/false/null, not 0/1" message; leave that
            // pair to it (this guard would otherwise pre-empt it with the generic reject).
            let bool_int = (matches!(a, Type::Boolean) && matches!(b, Type::Integer(_)))
                || (matches!(a, Type::Integer(_)) && matches!(b, Type::Boolean));
            if value(a) && value(b) && !both_bool && !bool_int {
                return Type::Null;
            }
        }
        // @PLN102 pre-freeze — an enum compared to a RAW integer coerces the enum to
        // its INTERNAL discriminant (`OpConvIntFromEnum`, +1-biased so variant 0 is
        // disc 1), so `Color.Green == 1` leaks that encoding and reads a confusing
        // false (Green's disc is 2).  Reject the enum-vs-integer pair like the other
        // cross-type comparisons — `enum == enum` is untouched (BOTH sides convert, so
        // both operands are `Enum` here, not one enum + one integer; a bare `enum ==
        // null` is `Enum` + `Null`).  The internal is-absent lowering builds `OpEqInt`
        // via `self.cl` with an already-integer discriminant, bypassing this path.
        if matches!(self.data.def(d_nr).name(), "OpEqInt" | "OpNeInt") && types.len() >= 2 {
            let (a, b) = (types[0].base(), types[1].base());
            let enum_int = (matches!(a, Type::Enum(..)) && matches!(b, Type::Integer(_)))
                || (matches!(a, Type::Integer(_)) && matches!(b, Type::Enum(..)));
            if enum_int {
                return Type::Null;
            }
        }
        let mut all_types = Vec::from(types);
        if self.data.def_type(d_nr) == DefType::Dynamic {
            for a_nr in 0..self.data.attributes(d_nr) {
                let Type::Routine(r_nr) = self.data.attr_type(d_nr, a_nr) else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Incorrect dynamic function {}",
                        self.data.def(d_nr).name()
                    );
                    return Type::Void;
                };
                if self.data.attr_type(r_nr, 0).is_equal(&types[0]) {
                    return self.call_nr(code, r_nr, list, types, report, arg_pos, name_pos);
                }
            }
            diagnostic!(
                self.lexer,
                Level::Error,
                "No matching function {}",
                self.data.def(d_nr).name()
            );
        } else if !self.callable_target(d_nr) {
            if report {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Unknown definition {}",
                    self.data.def(d_nr).name()
                );
            }
            return Type::Null;
        }
        let mut actual = self.process_call_args(d_nr, list, types, &mut all_types, report, arg_pos);
        if actual.is_empty() && !types.is_empty() {
            return Type::Null;
        }
        // @PLN86 §7.2 (F7) — gate an OVERRIDE of a `…#default`-locked parameter BEFORE
        // defaults fill the gaps: at this point a parameter slot is non-`Null` exactly
        // when the caller supplied it explicitly, so a defaulted parameter is invisible.
        if self.in_sandbox && !self.first_pass {
            self.record_param_lock_overrides(d_nr, &actual);
        }
        self.add_defaults(d_nr, &mut actual, &mut all_types);
        let tp = self.call_dependencies(d_nr, &all_types);
        *code = Value::Call(d_nr, actual);
        self.warn_text_len_byte_index(d_nr, code);
        // @PLN102 arc C step 3 — the recommended-idiom STEER.  A resolved call FROM
        // OWNED source (the entry project) to a `#superseded "Y"` symbol warns the
        // author toward `Y`; the old form keeps working (a never-break signpost, not
        // a removal).  The caller-provenance gate (`caller_source_is_owned`) means a
        // consumer re-parsing a dependency's source is NEVER nagged about the
        // dependency's internal old-idiom use — only whoever can act sees it.
        // Second-pass + `report` only, so it fires exactly once per call site;
        // `LOFT_NO_STEER` opts out; inert until a symbol is actually marked.
        if report
            && !self.first_pass
            && crate::keys::steer_enabled()
            && self.data.caller_source_is_owned()
        {
            let succ = self.data.def(d_nr).superseded().to_string();
            if !succ.is_empty() {
                let shown = self.data.def(d_nr).display_name().to_string();
                // Position on the call NAME when the caller supplied it, and carry the
                // successor as a structured suggestion so a `codeAction` can offer a
                // "Change to `Y`" quick-fix (step B).  Without a name position (method /
                // operator path) keep the cursor caret and no suggestion — a quick-fix
                // there could replace the wrong token.
                // ADVICE on both paths (loft#785-era audit, @PLN131). This used to be
                // Advice with a name position and Warning without one — the same condition
                // at two levels, decided by whether the compiler happened to know where the
                // call name sat, which is an implementation detail and not a property of
                // the program. It was load-bearing: a library calling a superseded METHOD
                // failed `--tests --deny-warnings` while the identical call to a FUNCTION
                // passed, so the never-break signpost broke a build. The tier rule settles
                // it — a diagnostic gates iff ignoring it can produce a wrong result, and
                // ignoring a steer cannot: the old form keeps working, identically, forever.
                let msg =
                    format!("`{shown}` is superseded — use `{succ}` (the old form keeps working)");
                if let Some(pos) = name_pos {
                    let (line, col) = (pos.line, pos.pos);
                    diagnostic_at!(
                        self.lexer,
                        pos,
                        Level::Advice,
                        code = "superseded-call",
                        "{msg}"
                    );
                    self.lexer.suggest_last(&succ);
                    // The one fix in the set whose rewrite the diagnostic already names:
                    // the successor is in the message, and `name_pos` is the identifier to
                    // replace. Mechanical — swapping a call for its shim changes nothing a
                    // program can observe, which is the whole promise of a fold.
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Mechanical,
                        title: format!("call `{succ}` instead"),
                        condition: None,
                        edit: Some(crate::diagnostics::Edit {
                            line,
                            col,
                            len: u32::try_from(shown.len()).unwrap_or(0),
                            text: succ.clone(),
                        }),
                        concept: "superseded",
                        concept_ref: "@F109",
                    });
                } else {
                    diagnostic!(self.lexer, Level::Advice, code = "superseded-call", "{msg}");
                    // No `name_pos`, so no edit: a method or operator call reaches here
                    // with the cursor somewhere after the name, and a rewrite placed there
                    // would replace the wrong token. The title still carries the fix.
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Mechanical,
                        title: format!("call `{succ}` instead"),
                        condition: None,
                        edit: None,
                        concept: "superseded",
                        concept_ref: "@F109",
                    });
                }
            }
        }
        tp
    }
}
