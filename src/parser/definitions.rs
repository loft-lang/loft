// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    Argument, DefType, Function, HashMap, HashSet, IntegerSpec, Level, Link, Parser, Position,
    ToString, Type, Value, complete_definition, diagnostic_format, is_camel, is_lower, is_op,
    is_upper, rename, v_block, v_if,
};

impl Parser {
    /// loft#799 — a keyed collection whose key field is the wrong TYPE for its kind
    /// is refused at DECLARATION, naming the kind that does key on it.
    ///
    /// `spatial` interleaves its axes into a Morton code, which is a numeric
    /// operation; a `text` key produced a collection that compiled, counted right,
    /// and then answered NULL for a key just inserted — indistinguishable from
    /// "not found" at the call site, which is the shape that reaches production. A
    /// `trie` walks one key's BYTES, so a numeric key is the mirror of the same
    /// mistake.
    ///
    /// Pass 2 only: pass 1 has an incomplete definition table by construction, so a
    /// key whose element type is declared further down the file would read as
    /// unknown there (loft#683's rule).
    fn check_key_is_text(&mut self, content: u32, field: &str, want_text: bool) {
        if self.first_pass {
            return;
        }
        let el = crate::typedef::key_bearing_def(&self.data, content);
        let a_nr = self.data.attr(el, field);
        if a_nr == usize::MAX {
            return; // an unknown field name is reported by the layout pass
        }
        let tp = self.data.attr_type(el, a_nr);
        // @FR-Col-Trie / @FR-Col-Spatial — a trie's key is text-NOT-NULL and a spatial's axes are
        // integer-NOT-NULL, because a trie walks its key's BYTES and a spatial interleaves its
        // axes into a Morton code, and an absence has neither.  The three VALUE-keyed kinds key
        // on the value, where an absence is a value like any other, and never ask this.
        //
        // @FR-L-Null — the KIND question peels: `text?` is a text in the same four bytes, and
        // asked bare it read as "not a text", which let it take a `spatial` axis' place.  That
        // compiled, iterated its records, and answered null for a point just inserted — the
        // loft#799 failure the dense spelling is refused for, reached through the `?`.
        let is_text = matches!(tp.base(), Type::Text(_));
        // …and nullability is its own answer, because neither kind has a key for absence:
        // `(Col-Trie)` walks the BYTES of a text and an absent text has none, `(Col-Spatial)`
        // interleaves integer-NOT-NULL coordinates and an absent one has no position on the
        // curve.  `hash` / `sorted` / `index` key on the VALUE, where absence is a value like
        // any other, and they do not ask this.
        if matches!(tp, Type::Optional(_)) {
            let shown = tp.name(&self.data);
            if want_text {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a trie keys on the BYTES of a text field, and `{field}` is `{shown}` — an \
                     absent text has no bytes to walk; declare the key `text`, or key on the \
                     VALUE with `hash<{}[{field}]>`, which holds an absent key like any other",
                    self.data.def(content).name()
                );
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a spatial index interleaves its axes into a Morton code, and the axis \
                     `{field}` is `{shown}` — an absent coordinate has no position on the \
                     curve; declare it `integer`, or key on the VALUE with `sorted` / `index`, \
                     which order an absent key before every present one"
                );
            }
            return;
        }
        if is_text == want_text {
            return;
        }
        if want_text {
            // A TUPLE key gets its own advice: `sorted` / `index` / `hash` take one, so the
            // fix is the collection kind, not the field.  The generic line would send the
            // author to `spatial<…>` "for coordinates" or to "order on a number", neither of
            // which describes a `(text, text)`.
            if matches!(tp, Type::Tuple(_)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a trie keys on the BYTES of ONE text field, and `{field}` is {} — a \
                     tuple key needs `sorted<…>` / `index<…>` (ordered lexicographically, \
                     element by element) or `hash<…>` (exact lookup)",
                    tp.name(&self.data)
                );
                return;
            }
            diagnostic!(
                self.lexer,
                Level::Error,
                "a trie keys on the BYTES of a text field, and `{field}` is {} — use \
                 `spatial<…>` for coordinates, or `sorted<…>` / `index<…>` to order on a \
                 number",
                tp.name(&self.data)
            );
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a spatial index interleaves its axes into a Morton code, which needs \
                 numbers, and `{field}` is text — use `trie<{}[{field}]>`, which keys on \
                 text and answers a prefix",
                self.data.def(content).name()
            );
        }
    }

    /// Consume an optional `not null` annotation, warning that it is deprecated.
    /// Returns `true` when it was present.
    ///
    /// @PLN25 F2: a type is non-null by DEFAULT now (`τ?` is the nullable form),
    /// so `not null` carries nothing — it stays parseable as an accepted no-op for
    /// back-compat, but every use gets a deprecation warning. That drives the
    /// warning-gate attrition (packages fail CI on the warning unless they carry
    /// `.allow_warnings`) toward the eventual hard "retired" error, which stays
    /// blocked on the registry republish (RESUME.md § F2 task #4). Warns once
    /// (pass 2 only) so a definition parsed in both passes reports a single note.
    fn has_deprecated_not_null(&mut self) -> bool {
        // loft#1003 — the annotation's own span, which the emit site did not have.  `not` and
        // `null` are two tokens with arbitrary whitespace between them, so the extent is read
        // from the cursor AFTER `null` rather than assumed to be eight characters.  A span
        // that crosses a line is left alone: the edit model is one line, and a `not\nnull` is
        // rare enough that prose is the honest answer there.
        let start = self.lexer.peek_pos().clone();
        if self.lexer.has_keyword("not") {
            // The start of `null` — taken before it is consumed, because `pos()` afterwards is
            // the scan cursor at the end of the token AFTER it (already past the closing `}`),
            // and a span measured to there deletes the rest of the declaration.
            let at_null = self.lexer.peek_pos().clone();
            self.lexer.token("null");
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Advice,
                    code = "not-null-deprecated",
                    "`not null` is deprecated and has no effect — a type is non-null by default now"
                );
                // `null` is four characters, so the annotation ends exactly there — no
                // dependence on what follows it, and no trailing whitespace swallowed.
                let edit = (at_null.line == start.line && at_null.pos >= start.pos).then(|| {
                    crate::diagnostics::Edit {
                        line: start.line,
                        col: start.pos,
                        len: at_null.pos + 4 - start.pos,
                        text: String::new(),
                    }
                });
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Mechanical,
                    title: "delete `not null`".to_string(),
                    condition: None,
                    edit,
                    concept: "struct records",
                    concept_ref: "@F12",
                });
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: "write `T?` instead".to_string(),
                    condition: Some("the type SHOULD allow null — `not null` was hiding that it already did not".to_string()),
                    edit: None,
                    concept: "nullable values",
                    concept_ref: "@F1",
                });
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn warn_missing_enum_variants(&mut self, e_nr: u32, nrs: &[usize], name: &str) {
        // @FR-F-Recv — which variant an implementation is FOR is `receiver_def_nr`'s answer,
        // so a `self: V?` receiver counts as an implementation of `V`.  Asked bare, this
        // reported "no implementation of 'area' for variant 'Square'" with one written five
        // lines above it.
        let implemented: HashSet<u32> = nrs
            .iter()
            .map(|nr| self.data.receiver_def_nr(*nr as u32))
            .filter(|a_nr| *a_nr != u32::MAX)
            .collect();
        let missing: Vec<(String, Position)> = self
            .data
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, v)| v.def_type == DefType::EnumValue && v.parent == e_nr)
            .filter(|(v_nr, _)| !implemented.contains(&(*v_nr as u32)))
            .map(|(_, v)| (v.name.clone(), v.position.clone()))
            .collect();
        for (variant_name, pos) in &missing {
            self.lexer.pos_diagnostic(
                Level::Warning,
                pos,
                &format!("no implementation of '{name}' for variant '{variant_name}'"),
            );
        }
    }

    // @F20 — variant-based dynamic dispatch (synthesised enum dispatcher)
    pub(crate) fn create_enum_dispatch_fn(&mut self, e_nr: u32, nrs: &[usize]) {
        let from_nr = nrs[0] as u32;
        let name = self.data.def(from_nr).original_name().clone();
        let attrs = self.data.def(from_nr).attributes()[1..].to_vec();
        let mut common = attrs.len();
        for nr in &nrs[1..] {
            let mut c = 0;
            for a in &self.data.def(*nr as u32).attributes()[1..] {
                for o in &attrs {
                    if a.name == o.name && a.typedef == o.typedef {
                        c += 1;
                    }
                }
            }
            if c < common {
                common = c;
            }
        }
        for nr in nrs {
            if self.data.def(*nr as u32).attributes().len() > common + 1 {
                for a in &self.data.def(*nr as u32).attributes()[common + 1..] {
                    if a.value == Value::Null {
                        return;
                    }
                }
            }
        }
        let mut args = Vec::new();
        args.push(Argument {
            name: "self".to_string(),
            typedef: Type::Enum(e_nr, true, crate::data::Deps::none()),
            default: Value::Null,
            constant: false,
            // Synthesized receiver — no `&` / `const` token in any source file to point at.
            ref_pos: (0, 0),
            const_pos: (0, 0),
        });
        for a in &attrs[..common] {
            args.push(Argument {
                name: a.name.clone(),
                typedef: a.typedef.clone(),
                default: a.value.clone(),
                constant: false,
                ref_pos: (0, 0),
                const_pos: (0, 0),
            });
        }
        let fn_nr = self.data.add_fn(&mut self.lexer, &name, &args);
        // `add_fn` answers `u32::MAX` when it refused the definition, and it has already
        // said why. Indexing the definition table with that sentinel panicked the compiler
        // — an internal-compiler-error on a program whose only sin was a name two packages
        // share (loft#850's family). The refusal itself is now unreachable from here, but a
        // sentinel is never an index: a synthesised function that was not created has
        // nothing left to fill in.
        if fn_nr == u32::MAX {
            return;
        }
        self.data.mark_synthetic(fn_nr, "enum_dispatcher");
        self.context = fn_nr;
        self.vars = Function::new(&name, &self.data.def(from_nr).position().file);
        self.data
            .set_returned(fn_nr, self.data.def(from_nr).returned().clone());
        for a in &args {
            let v_nr = self.create_var(&a.name, &a.typedef);
            if v_nr != u16::MAX {
                self.vars.become_argument(v_nr);
            }
        }
        // Build forwarding args for extra (non-self) attributes (e.g. RefVar(Text) buffers).
        // Variant calls must write into the dispatcher's own text-buffer argument, not a
        // freshly-allocated work_text that has no stack slot yet.
        let mut extra_call_args: Vec<Value> = Vec::new();
        let mut extra_call_types: Vec<Type> = Vec::new();
        for a in &args[1..] {
            let v = self.vars.var(&a.name);
            if v != u16::MAX {
                extra_call_args.push(Value::Var(v));
                extra_call_types.push(a.typedef.clone());
            }
        }
        let mut ls = Vec::new();
        let get_enum = self.cl("OpGetEnum", &[Value::Var(0), Value::Int(0)]);
        let get_int = self.cl("OpConvIntFromEnum", &[get_enum]);
        self.enum_numbers(
            nrs.to_vec(),
            &name,
            &mut ls,
            &get_int,
            &extra_call_args,
            &extra_call_types,
        );
        // No-variant-matched fallback: an explicit `return null`, not a bare
        // `Null` tail. As the tail of a value-typed (e.g. text) block the bare
        // Null was wrapped in `Str::new(<dispatch if>)` and emitted `Str::new(())`
        // (E0308) under --native; `Return(Null)` routes through the typed-null
        // return path (STRING_NULL for text, i64::MIN for int, …) on both backends.
        ls.push(Value::Return(Box::new(Value::Null)));
        self.data.definitions[fn_nr as usize].code =
            v_block(ls, self.data.def(from_nr).returned().clone(), "dynamic_fn");
        self.data.definitions[self.context as usize].variables = self.vars.clone();
        self.warn_missing_enum_variants(e_nr, nrs, &name);
    }

    pub(crate) fn enum_fn(&mut self) {
        if !self.first_pass {
            return;
        }
        let mut todo = HashMap::new();
        for (d_nr, d) in self.data.definitions.iter().enumerate() {
            if d.def_type != DefType::Function || d.attributes.is_empty() {
                continue;
            }
            // @FR-F-Recv — the receiver's base type names the variant, so both `self: V` and
            // `self: V?` are implementations of `V` and both belong in the dispatcher.
            let recv = self.data.receiver_def_nr(d_nr as u32);
            if recv != u32::MAX
                && let e_tp = &recv
                && matches!(self.data.def(*e_tp).returned(), Type::Enum(_, true, _))
                && self.data.find_fn(
                    u16::MAX,
                    &d.original_name(),
                    self.data.def(*e_tp).returned(),
                ) == u32::MAX
                && let Type::Enum(e_nr, true, _) = self.data.def(*e_tp).returned()
                // loft#850 — whether an enum already HAS this dispatcher is a fact about
                // the enum, so ask the enum. The `find_fn` test above searches the source
                // being parsed, while the scan it guards runs over EVERY definition in the
                // program: reaching a second package, it re-answered "no dispatcher" for
                // the first package's enum — whose dispatcher was filed under the first
                // package's source — and set out to synthesise a duplicate. `add_fn` then
                // refused it, correctly, and returned the sentinel that crashed the
                // compiler. A method lives in its type's own attribute table, which is
                // shared and source-independent, so it answers the same from anywhere.
                && self.data.attr(*e_nr, &d.original_name()) == usize::MAX
            {
                // Keyed by the enum AND the method NAME: a dispatcher dispatches ONE method,
                // and `create_enum_dispatch_fn` names it after `nrs[0]`.  Keyed by the enum
                // alone, an enum with two methods per variant put both lists in one bucket, so
                // one method got a dispatcher named after whichever definition came first and
                // the other got none — its call site then failed as a FIELD read (*"field 'per'
                // of 'Ca' has no storage in that type's layout"*), and where the two lists'
                // attributes disagreed the shared bucket bailed and NEITHER was built
                // (`tests/scripts/05-enums.loft` had three `area` and three `describe`
                // implementations and no dispatcher for either).
                todo.entry((*e_nr, d.original_name().clone()))
                    .or_insert(vec![])
                    .push(d_nr);
            }
        }
        // Sorted, because a `HashMap`'s order is not stable across runs and the dispatchers are
        // emitted in the order they are created: an unsorted walk makes the generated Rust
        // differ between two builds of one program.
        let mut keys: Vec<(u32, String)> = todo.keys().cloned().collect();
        keys.sort();
        for key in keys {
            let nrs = self.one_implementation_per_variant(&todo[&key]);
            self.create_enum_dispatch_fn(key.0, &nrs);
        }
    }

    /// One implementation per VARIANT, the way a direct call picks one.
    ///
    /// A variant may carry two overloads of a method — `self: V` and `self: V?` — and a call
    /// on a dense receiver takes the dense one (`find_fn`; the reverse declaration order is
    /// refused outright as a redefinition).  The dispatcher's arm must answer what that call
    /// answers, so the dense spelling wins here too, and the arm list keeps ONE def per
    /// discriminant rather than an unreachable second `if` on the same tag (@FR-F-Recv).
    fn one_implementation_per_variant(&self, nrs: &[usize]) -> Vec<usize> {
        let mut kept: Vec<usize> = Vec::with_capacity(nrs.len());
        for nr in nrs {
            let variant = self.data.receiver_def_nr(*nr as u32);
            let dense = !matches!(
                self.data.def(*nr as u32).attributes()[0].typedef,
                Type::Optional(_)
            );
            match kept
                .iter()
                .position(|k| self.data.receiver_def_nr(*k as u32) == variant)
            {
                Some(at) if dense => kept[at] = *nr,
                Some(_) => {}
                None => kept.push(*nr),
            }
        }
        kept
    }

    pub(crate) fn enum_numbers(
        &mut self,
        nrs: Vec<usize>,
        name: &str,
        ls: &mut Vec<Value>,
        get_int: &Value,
        extra_args: &[Value],
        extra_types: &[Type],
    ) {
        for nr in nrs {
            let d_nr = nr as u32;
            // @FR-F-Recv — the arm's discriminant comes from the variant the receiver names,
            // which a `self: V?` names exactly as `self: V` does.
            let a_nr = match self.data.receiver_def_nr(d_nr) {
                u32::MAX => 0,
                nr => nr,
            };
            let e_nr = if let Value::Enum(nr, _) = self.data.def(a_nr).attributes()[0].value {
                nr
            } else {
                0
            };
            let self_type = self.data.def(d_nr).attributes()[0].typedef.clone();
            let mut call_args = vec![Value::Var(0)];
            call_args.extend_from_slice(extra_args);
            let mut call_types = vec![self_type];
            call_types.extend_from_slice(extra_types);
            let mut code = Value::Null;
            let name_pos = self.lexer.pos().clone();
            self.call(
                &mut code,
                u16::MAX,
                name,
                &call_args,
                &call_types,
                &[],
                &[],
                &name_pos,
            );
            let ret_call = v_block(
                vec![Value::Return(Box::new(code.clone()))],
                Type::Void,
                "ret",
            );
            ls.push(v_if(
                self.cl("OpEqInt", &[get_int.clone(), Value::Int(i32::from(e_nr))]),
                ret_call,
                Value::Null,
            ));
        }
    }

    /// Parse the `{ Value { fields }, Value, ... }` body of an enum definition.
    /// Returns false if a fatal parse error occurred and parsing should stop.
    pub(crate) fn parse_enum_values(&mut self, d_nr: u32) -> bool {
        let mut nr: u8 = 0;
        loop {
            let Some(value_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
                return false;
            };
            if !is_camel(&value_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect enum values to be in camel case style"
                );
            }
            let v_nr = if self.first_pass {
                // A forward reference may already have left a stub under this name:
                // `Circle{ r: 2 }` written ABOVE `enum Shape { Circle { r: integer } }`
                // registers `Circle` speculatively (`objects.rs`, the `Name { … }` branch).
                // ADOPT that stub in place, exactly as `parse_struct` and the typedef path
                // do for a type name — adding a SECOND def under the same name leaves the
                // construction site still pointing at the stub, which pass 2 then reports
                // as `unknown type 'Circle'` even though the declaration is right there
                // (loft#1046).  A plain `struct` forward reference already worked for this
                // reason; the enum VARIANT was the one declaration kind that did not adopt.
                let existing = self.data.def_nr(&value_name);
                let v = if existing != u32::MAX
                    && self.data.def_type(existing) == DefType::Unknown
                    && matches!(
                        self.data.definitions[existing as usize].returned,
                        Type::Unknown(_)
                    ) {
                    self.data.definitions[existing as usize].position = self.lexer.pos().clone();
                    self.data.definitions[existing as usize].def_type = DefType::EnumValue;
                    self.data.note_stub_adopted(existing);
                    existing
                } else {
                    self.data
                        .add_def(&value_name, self.lexer.pos(), DefType::EnumValue)
                };
                self.data.definitions[v as usize].parent = d_nr;
                v
            } else {
                // @PLN22 Phase 1 — the second-pass re-resolve goes through the
                // enum (d_nr) we are parsing via the variant_of chokepoint, not
                // the bare global def_nr.  Non-breaking while variants are still
                // globally keyed (the variant is a child of d_nr); required once
                // de-globalize lands (step 4), when def_nr no longer keys variants.
                self.data.variant_of(d_nr, &value_name)
            };
            if self.lexer.has_token("{") {
                if self.first_pass {
                    self.data.definitions[d_nr as usize].returned =
                        Type::Enum(d_nr, true, crate::data::Deps::none());
                    self.data
                        .set_returned(v_nr, Type::Enum(d_nr, true, crate::data::Deps::none()));
                    self.data.add_attribute(
                        &mut self.lexer,
                        d_nr,
                        &value_name,
                        Type::Enum(d_nr, true, crate::data::Deps::none()),
                    );
                    self.data.definitions[d_nr as usize].attributes[nr as usize].constant = true;
                    // Enum values start with 1 as 0 is de null/undefined value.
                    self.data
                        .set_attr_value(d_nr, nr as usize, Value::Enum(nr + 1, u16::MAX));
                    // Create an "enum" field inside the new structure
                    let e_attr = self.data.add_attribute(
                        &mut self.lexer,
                        v_nr,
                        "enum",
                        Type::Enum(
                            self.data.def_nr("enumerate"),
                            false,
                            crate::data::Deps::none(),
                        ),
                    );
                    // Enum values start with 1 as 0 is de null/undefined value.
                    self.data
                        .set_attr_value(v_nr, e_attr, Value::Enum(nr + 1, u16::MAX));
                }
                // Each variant field's NAME with its position, for a diagnostic about the
                // SHAPE of the declaration — see `advise_group_apart`.
                let mut field_at: Vec<(String, crate::lexer::Position)> = Vec::new();
                loop {
                    // @PLN35 — `#lexeme` marks the field carrying this token variant's surface
                    // text, so a bare literal in a slice pattern (`[ "fn", … ]`) matches against
                    // it.  It precedes the field name: `Keyword { #lexeme name: text }`.
                    let is_lexeme = if self.lexer.has_token("#") {
                        match self.lexer.has_identifier().as_deref() {
                            Some("lexeme") => true,
                            other => {
                                if !self.first_pass {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "unknown field annotation `#{}` (expected `#lexeme`)",
                                        other.unwrap_or("")
                                    );
                                }
                                false
                            }
                        }
                    } else {
                        false
                    };
                    // @PLN40 — a variant field may also be `const` (write-once).
                    let is_const = self.lexer.has_keyword("const");
                    let field_pos = self.lexer.peek_pos().clone();
                    let Some(a_name) = self.lexer.has_identifier() else {
                        diagnostic!(self.lexer, Level::Error, "Expect attribute");
                        return true;
                    };
                    if self.first_pass && self.data.attr(v_nr, &a_name) != usize::MAX {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "field `{}` is already declared",
                            a_name
                        );
                    }
                    self.lexer.token(":");
                    field_at.push((a_name.clone(), field_pos));
                    self.parse_field(v_nr, &a_name);
                    if is_lexeme {
                        let idx = self.data.attr(v_nr, &a_name);
                        if idx != usize::MAX {
                            self.data.definitions[v_nr as usize].attributes[idx].lexeme = true;
                        }
                    }
                    if is_const {
                        self.mark_const_field(v_nr, &a_name);
                    }
                    // accept trailing comma after the last field,
                    // matching struct parsing (line 1380).
                    if !self.lexer.has_token(",") || self.lexer.peek_token("}") {
                        break;
                    }
                }
                self.lexer.token("}");
                // A struct-enum VARIANT holds fields like a struct, and `Stores::field` forms a
                // linked group inside one on the same terms (@FR-Col-Group).  Both halves of
                // that therefore belong here as well as in `parse_struct`: without the rewrite,
                // a keyed view beside a `vector<S?>` in a variant stayed dense and silently
                // built a second collection — all five kinds, the shape `parse_struct` was
                // fixed for one container kind over.
                self.link_shared_nullable_views(v_nr);
                self.advise_group_apart(v_nr, &field_at);
            } else if self.first_pass {
                self.data
                    .set_returned(v_nr, Type::Enum(d_nr, false, crate::data::Deps::none()));
                self.data.add_attribute(
                    &mut self.lexer,
                    d_nr,
                    &value_name,
                    Type::Enum(d_nr, false, crate::data::Deps::none()),
                );
                self.data.definitions[d_nr as usize].attributes[nr as usize].constant = true;
                // Enum values start with 1 as 0 is de null/undefined value.
                self.data
                    .set_attr_value(d_nr, nr as usize, Value::Enum(nr + 1, u16::MAX));
            } else if *self.data.def(d_nr).returned() != *self.data.def(v_nr).returned() {
                self.data.definitions[v_nr as usize].returned =
                    self.data.def(d_nr).returned().clone();
            }
            // accept trailing comma after the last variant,
            // matching the trailing-comma guard on the field-list loop above.
            if !self.lexer.has_token(",") || self.lexer.peek_token("}") {
                break;
            }
            // A plain enum is ONE BYTE and both ends of it are reserved: `0` is the
            // undefined value the variants are numbered away from (`nr + 1` below), and
            // `255` is the null sentinel every scalar type has
            // (formal/types.md § Per-type null, `OpConvBoolFromEnum` = `@v1 != 255 && @v1
            // != 0`).  So the highest variant a program can READ BACK is `254`, and the
            // 254th variant is the last one.  The refusal has to come BEFORE the next
            // variant is numbered, because numbering it is what breaks: variant 255 takes
            // the null sentinel and answers `null` from a name that matched, and variant
            // 256 overflows the `u8` — an internal compiler error under debug assertions,
            // a wrap to the reserved `0` without them.
            if nr == 253 {
                self.lexer.diagnostic(
                    Level::Error,
                    "Too many enum variants — an enum holds at most 254, because a variant is one byte and 0 and 255 are reserved",
                );
                // Skip the rest of the variant list rather than breaking out mid-name: the
                // caller's `token("}")` is standing at the next variant otherwise, and one
                // count mistake reported four diagnostics — the real one and three
                // `Expect token` cascades behind it.  The `}` is left for that caller.
                self.lexer.recover_to(&["}"]);
                break;
            }
            nr += 1;
        }
        // B2 fix: in a mixed-kind enum (some unit variants, some struct-
        // field variants), the unit variants processed *before* the
        // first struct variant got typed as Enum(d_nr, false, _) because
        // the parent enum had not yet been upgraded to struct-enum.
        // Sync both each variant's `returned` type and the parent's
        // per-variant attribute types to the final parent.returned so
        // pattern match / construction / return paths all see the same
        // struct-enum discriminator width.
        if self.first_pass {
            let parent_returned = self.data.def(d_nr).returned().clone();
            if matches!(parent_returned, Type::Enum(_, true, _)) {
                let num_variants = self.data.def(d_nr).attributes().len();
                for a_nr in 0..num_variants {
                    let v_name = self.data.def(d_nr).attributes()[a_nr].name.clone();
                    let v_nr = self.data.def_nr(&v_name);
                    if v_nr != u32::MAX {
                        self.data.definitions[v_nr as usize].returned = parent_returned.clone();
                    }
                    self.data.definitions[d_nr as usize].attributes[a_nr].typedef =
                        parent_returned.clone();
                }
            }
        }
        true
    }

    /// @PLN22 Phase 2 — true if `name` resolves ONLY through the source-0 prelude
    /// fallback (a stdlib / wildcard-imported name) and is NOT present in the
    /// source being parsed — so a definition of it here SHADOWS the prelude one
    /// rather than being rejected as a redefinition; the shadowed one stays
    /// reachable via qualification (`std::E`).
    ///
    /// The membership test is by NAME against the current source's namespace
    /// (`source_nr(cur, name)`), NOT by the found def's physical `.source`: a
    /// cross-file forward-ref type (p173) lives physically in another file's
    /// source yet is imported INTO this source's namespace, so it must be filled,
    /// not shadowed.  Built-in type-keywords (`integer`, `vector`, … — the
    /// stdlib's `DefType::Type` at source 0) are NEVER shadowable (shadowing them
    /// would re-point the language's own types).  Every other prelude/import kind
    /// (const, struct, enum, library typedef) is shadowable.
    fn prelude_shadowed(&self, name: &str) -> bool {
        let cur = self.data.source;
        // the stdlib itself (source 0) never shadows; and a name already in THIS
        // source's namespace (a real def or a cross-file forward-ref imported
        // into it) is filled/conflicts, not shadowed.
        if cur == 0 || self.data.source_nr(cur, name) != u32::MAX {
            return false;
        }
        // resolvable only via the source-0 prelude fallback?
        let prelude = self.data.def_nr(name);
        prelude != u32::MAX
            && self.data.def_type(prelude) != DefType::Unknown
            // built-in type-keyword (stdlib typedef) — sacred.
            && !(self.data.def_type(prelude) == DefType::Type
                && self.data.def(prelude).source == crate::data::STD_SOURCE)
    }

    // <enum> ::= 'enum' <identifier> '{' <value> {, <value>} '}' [';']
    // @F13 — simple enums (ordered value types)
    // @F14 — polymorphic struct-enums (per-variant fields)
    // @F15 — enum-scoped variant names + context inference
    pub(crate) fn parse_enum(&mut self) -> bool {
        if !self.lexer.has_token("enum") {
            return false;
        }
        let Some(type_name) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
            return false;
        };
        if !is_camel(&type_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect enum definitions to be in camel case style"
            );
        }
        let mut d_nr = self.data.def_nr(&type_name);
        // @PLN22 Phase 2 — shadow a prelude/import name of the same key.
        if self.prelude_shadowed(&type_name) {
            d_nr = u32::MAX;
        }
        let mut conflict = false;
        if d_nr == u32::MAX {
            let pos = self.lexer.pos();
            d_nr = self.data.add_def(&type_name, pos, DefType::Enum);
        } else if self.first_pass && self.data.def_type(d_nr) == DefType::Unknown {
            self.data.definitions[d_nr as usize].def_type = DefType::Enum;
            self.data.definitions[d_nr as usize].position = self.lexer.pos().clone();
            self.data.note_stub_adopted(d_nr);
        } else if self.first_pass {
            // a name that already exists must not be reused — that
            // would overwrite the existing definition's type and crash in
            // `set_returned` below.  Emit a clear diagnostic naming the
            // existing definition's location.
            let prev_pos = self.data.def(d_nr).position().clone();
            let prev_kind = format!("{:?}", self.data.def(d_nr).def_type()).to_lowercase();
            diagnostic!(
                self.lexer,
                Level::Error,
                "enum '{type_name}' conflicts with a {prev_kind} of the same name \
                 already defined at {prev_pos} — pick a different name"
            );
            conflict = true;
        }
        if self.first_pass && !conflict {
            self.data
                .set_returned(d_nr, Type::Enum(d_nr, false, crate::data::Deps::none()));
        }
        if !self.lexer.token("{") {
            return false;
        }
        if !self.parse_enum_values(d_nr) {
            return false;
        }
        // Skip type-completion when this enum conflicts with a builtin of the
        // same name (e.g. `enum hash`): `d_nr` is then the existing builtin, and
        // `complete_definition` would re-`set_returned` an already-typed def and
        // panic.  The conflict diagnostic above is the user-facing result.
        if self.first_pass && !conflict {
            complete_definition(&mut self.lexer, &mut self.data, d_nr);
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        true
    }

    // <typedef> ::= 'type' <identifier> '=' <type_def> [ 'size' '(' <integer> ')' ] ';'
    // @F46 — type aliases (type X = …)
    pub(crate) fn parse_typedef(&mut self) -> bool {
        if !self.lexer.has_token("type") {
            return false;
        }
        let Some(type_name) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect name in type definition");
            return false;
        };
        if !self.default && !is_camel(&type_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect type definitions to be in camel case style"
            );
        }
        // detect a name collision before calling `add_def`, which
        // would otherwise panic with `Dual definition of <name>`.  Emit a
        // clear diagnostic citing the prior definition's location.
        let mut conflict = false;
        // A forward-reference placeholder left by a file that named this type before it
        // was declared.  `parse_struct` and `parse_enum` both ADOPT such a stub in place —
        // that adoption is what makes a cross-module forward reference resolve at all, so
        // a typedef that reported it as a name clash was the one declaration kind a module
        // could not forward-reference (loft#801).  A stub is `DefType::Unknown` with an
        // `Unknown` returned type; a reserved builtin type-keyword (`type iterator;`) is
        // `DefType::Type` with an Unknown returned type and must NOT be adopted, or a
        // user typedef would silently shadow the builtin.
        let mut adopted = u32::MAX;
        if self.first_pass {
            let mut existing = self.data.def_nr(&type_name);
            // @PLN22 Phase 2 — shadow a prelude/import name (but not a built-in
            // type-keyword, which prelude_shadowed excludes).
            if self.prelude_shadowed(&type_name) {
                existing = u32::MAX;
            }
            if existing != u32::MAX
                && self.data.def_type(existing) == DefType::Unknown
                && matches!(
                    self.data.definitions[existing as usize].returned,
                    Type::Unknown(_)
                )
            {
                self.data.definitions[existing as usize].position = self.lexer.pos().clone();
                self.data.definitions[existing as usize].def_type = DefType::Type;
                self.data.note_stub_adopted(existing);
                adopted = existing;
            } else if existing != u32::MAX {
                let prev_pos = self.data.def(existing).position().clone();
                let prev_kind = format!("{:?}", self.data.def(existing).def_type()).to_lowercase();
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "type '{type_name}' conflicts with a {prev_kind} of the same name \
                     already defined at {prev_pos} — pick a different name"
                );
                conflict = true;
            }
        }
        let d_nr = if adopted != u32::MAX {
            adopted
        } else if self.first_pass && !conflict {
            self.data
                .add_def(&type_name, self.lexer.pos(), DefType::Type)
        } else {
            self.data.def_nr(&type_name)
        };
        if self.lexer.has_token("=") {
            if let Some(tp) = self.parse_type_full(d_nr, false) {
                if self.first_pass && !conflict && d_nr != u32::MAX {
                    self.data.set_returned(d_nr, tp);
                }
            } else if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Expected a type after =");
            }
        }
        if self.lexer.has_keyword("size") {
            self.lexer.token("(");
            if let Some(n) = self.lexer.has_integer() {
                // Only 1/2/4/8 are meaningful for integer subtypes.  Larger
                // values (e.g. size(12) on the built-in `reference` alias)
                // are accepted silently — forced_size is only consulted for
                // integer types, so non-integer annotations are harmless.
                if matches!(n, 1 | 2 | 4 | 8) && self.first_pass && d_nr != u32::MAX {
                    self.data.definitions[d_nr as usize].forced_size = Some(n as u8);
                }
            }
            self.lexer.token(")");
        }
        // Same guard as `parse_enum`: a conflicting `type hash = …` leaves `d_nr`
        // pointing at the existing builtin, so re-completing it would panic.
        if self.first_pass && !conflict {
            complete_definition(&mut self.lexer, &mut self.data, d_nr);
        }
        self.lexer.token(";");
        true
    }

    /// Fold a TEXT constant's initialiser to a single literal, or `None` when it is not
    /// all-literal.
    ///
    /// A `"x" + "y"` initialiser parses to `{ OpClearText(w); OpAppendText(w, "x");
    /// OpAppendText(w, "y"); w }` — a block that builds its value in a WORK BUFFER.  A
    /// constant is pasted verbatim at each use, work-var numbers included, so inside a
    /// formatted string the pasted block clears and appends the very buffer the format
    /// is being built into: `"[{B}]"` printed `xyxy]`, having wiped the `[` and then
    /// appended the buffer to itself.  Folding removes the buffer, so there is nothing
    /// left to alias.
    ///
    /// Vector constants avoid the same trap by living in the const store and being
    /// referenced (`OpConstRef`) rather than pasted; text has no such store, and a
    /// literal fold makes one unnecessary.
    ///
    /// The fold itself is [`crate::const_eval::fold_text_block`] — a constant-store
    /// ELEMENT needs the same answer (loft#1090), and one home keeps the two from
    /// disagreeing about which initialisers are all-literal.
    fn fold_text_constant(&self, val: &Value) -> Option<String> {
        crate::const_eval::fold_text_block(val, &self.data)
    }

    /// Whether a text constant's block can be re-pointed at a buffer owned by the
    /// function that pastes it (see [`crate::parser::Parser::rebind_constant_buffer`]).
    ///
    /// True when the block builds in exactly ONE variable and hands that variable back
    /// as its result — the shape every text initialiser has.  Checked here, at the
    /// declaration, so a constant that can never be pasted safely is reported where the
    /// reader can act on it rather than at each use.
    fn constant_block_is_rebindable(val: &Value) -> bool {
        let Value::Block(bl) = val.unspan() else {
            return false;
        };
        let mut seen = std::collections::BTreeSet::new();
        let mut probe = val.clone();
        if !visit_constant_vars(&mut probe, &mut |v| {
            seen.insert(*v);
        }) {
            return false;
        }
        seen.len() == 1
            && matches!(bl.operators.last().map(Value::unspan),
                Some(Value::Var(v)) if seen.contains(v))
    }

    /// loft#702 — what about this vector-constant ELEMENT type the constant store cannot
    /// pre-build, or `None` when the element is flat enough to hold.
    ///
    /// Flat means scalars and text: exactly what the initialiser's literal field writes
    /// describe, which is all the pre-builder has to work from.  Anything holding a
    /// record of its own — an inner collection, a struct or enum field — has data those
    /// writes never mention, so it would be built empty.  Returns the phrase naming the
    /// offending part, for the diagnostic to place in a sentence.
    fn const_elem_unsupported(&self, elem: &Type) -> Option<String> {
        if crate::parser::vectors::is_collection(elem.base()) {
            return Some("a collection as its element".to_string());
        }
        let (Type::Reference(s_nr, _) | Type::Enum(s_nr, _, _)) = elem.base() else {
            return None;
        };
        let def = self.data.def(*s_nr);
        for a in def.attributes() {
            let nested = crate::parser::vectors::is_collection(a.typedef.base())
                || matches!(
                    a.typedef.base(),
                    Type::Reference(_, _) | Type::Enum(_, _, _)
                );
            if nested {
                let sn = def.name().to_string();
                let fname = a.name.clone();
                return Some(format!("a nested record in `{sn}.{fname}`"));
            }
        }
        None
    }

    /// The name of a call in a constant's initialiser that makes re-evaluation COST
    /// something, or `None` when the initialiser is free to inline.
    ///
    /// Costly = a user-defined function (any source but the stdlib — its body is
    /// arbitrary and unannotated), or a stdlib function marked `#impure(category)`.
    /// Everything else — arithmetic, a pure stdlib call, a literal — re-evaluates for
    /// free, which is the whole point of inlining a constant, so it stays silent.
    fn const_initialiser_cost(&self, val: &Value) -> Option<String> {
        match val.unspan() {
            Value::Call(d_nr, args) => {
                let def = self.data.def(*d_nr);
                // Only NAMED functions (`n_`-prefixed).  The internal operator
                // vocabulary — `OpNewRecord` for a vector literal, `OpAddInt` for
                // arithmetic — is what a constant is MADE of; warning on it would fire
                // on `NUMS = [1, 2, 3];`, which re-evaluates for free and is exactly
                // the case inlining exists for.
                let raw = def.name();
                let named_fn = def.def_type == DefType::Function && raw.starts_with("n_");
                let costly = def.source != crate::data::STD_SOURCE
                    || matches!(def.purity, crate::data::Purity::Impure(_));
                if named_fn && costly {
                    return Some(format!("{}()", raw.trim_start_matches("n_")));
                }
                args.iter().find_map(|a| self.const_initialiser_cost(a))
            }
            Value::Block(bl) => bl
                .operators
                .iter()
                .find_map(|o| self.const_initialiser_cost(o)),
            Value::Insert(ops) => ops.iter().find_map(|o| self.const_initialiser_cost(o)),
            Value::Set(_, inner) | Value::Return(inner) | Value::Drop(inner) => {
                self.const_initialiser_cost(inner)
            }
            _ => None,
        }
    }

    // <constant>
    // Accepts either `NAME = expr;` or `NAME: type = expr;`. The optional
    // type annotation is parsed (so the parser doesn't reject the form)
    // but the inferred type from the initialiser is the source of truth.
    pub(crate) fn parse_constant(&mut self) -> bool {
        // P246 — accept the optional `const` keyword at file scope as
        // a synonym for the bare-name form (`const PI = 3.14;` ===
        // `PI = 3.14;`).  Pre-fix the leading `const` swallowed
        // identifiable name, the parser fell through to
        // `expression()`, and `change_var_type` panicked on an empty
        // file-scope variable table.  The two forms are identical at
        // every level — same definition kind, same UPPER_CASE check,
        // same code path — so the keyword is purely an explicitness
        // signal at the declaration site (matches the in-fn `const`
        // syntax and lib/hex_world/src/wall.loft's existing usage).
        let _explicit_const = self.lexer.has_keyword("const");
        if let Some(id) = self.lexer.has_identifier() {
            // Optional `: type` annotation between the identifier and `=`.
            // Parsed and discarded — the literal's element type is used.
            // A future enhancement could validate the inferred type matches
            // the annotation (after dep-list normalisation).
            if self.lexer.has_token(":") {
                let _ = self.parse_type_full(u32::MAX, false);
            }
            self.lexer.token("=");
            if !is_upper(&id) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect constants to be in upper case style"
                );
            }
            let mut val = Value::Null;
            let mut tp = self.expression(&mut val);
            // A struct-valued file-scope constant (`P = Point { … }`) is NOT supported: its
            // value is the constructor's field-writes with no allocated record, so every use
            // inlines writes into a null record — silently reading `null` on `--interpret`,
            // panicking codegen on a plain bind, and failing to compile (`E0308`) on
            // `--native`.  Scalars inline fine and scalar-element vector constants ride the
            // `OpConstRef` const-store path; a heap record has neither.  Reject with the
            // working idiom (a zero-arg fn re-materialises the record per call).  Full
            // support = a const-store record builder (see the routing-feedback triage doc).
            if !self.first_pass
                && let Type::Reference(a_nr, _) = tp.base()
            {
                let type_name = self.data.def(*a_nr).name().to_string();
                let fn_name = id.to_lowercase();
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a struct-valued constant ('{id}') is not supported — a record cannot be \
                     materialised at each use site (it reads `null` on --interpret and fails \
                     to compile on --native).  Wrap it in a zero-argument function instead: \
                     `fn {fn_name}() -> {type_name} {{ … }}`, then call `{fn_name}()`"
                );
            }
            // loft#702 — the const store pre-builds a vector ELEMENT from the literal
            // field writes its initialiser emits, so it can hold only what those writes
            // describe: scalars and text, laid out flat.  A nested record — an inner
            // vector, a keyed collection, a struct field — lives in a store of its own
            // that no field write names, so it was pre-built EMPTY and read back empty
            // (`NEST = [[7], [8]]` gave two rows of nothing; a `vector<integer>` field
            // read length 0).  Say so here, where the reader can act on it, instead of at
            // whichever use first trusts the value.
            let mut refused_as_unbuildable = false;
            if !self.first_pass
                && let Type::Vector(elem, _) = tp.base()
            {
                // The element TYPE first, because when it is the problem that message
                // names the limitation better; then the initialiser, which is the only
                // thing that can answer for a type the store could otherwise hold.
                //
                // loft#1090 — a constant that cannot be pre-built has no fallback: the
                // use site emits `OpConstRef`, so an unbuilt constant reads `null` at
                // every reference with no error and no warning.  `Row { r_id: BASE + 1 }`
                // was enough, and the vector it was in reported length 0 — a `for` over
                // the table ran zero times and every lookup fell through to a default,
                // far away from the declaration.  Ask the pre-builder itself, so the
                // refusal and the build can never disagree about what is buildable.
                let unsupported = self
                    .const_elem_unsupported(elem)
                    .map(|what| format!("has {what}, which a constant cannot hold"))
                    .or_else(|| {
                        crate::compile::const_vector_blocker(&val, &self.data)
                            .map(|what| format!("is built from {what}"))
                    });
                if let Some(what) = unsupported {
                    refused_as_unbuildable = true;
                    let fn_name = id.to_lowercase();
                    let tn = tp.base().name(&self.data);
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "constant '{id}' {what} — its elements are pre-built in the \
                         constant store from their literal fields, and anything they do \
                         not describe reads back empty.  Use a zero-argument function \
                         instead: `fn {fn_name}() -> {tn} {{ … }}`, then call \
                         `{fn_name}()`"
                    );
                }
            }
            // A constant is INLINED at each reference, so an initialiser that calls
            // something pays that cost per use.  Name it, because the word "constant"
            // promises the opposite and the failure is invisible until the one target
            // with bounded memory: a consumer's `FNT = load_bundled();` re-parsed a
            // 760 KB font once per word per frame and trapped the browser wasm.
            // A text initialiser builds its value in a work buffer, and a constant is
            // PASTED at each use — buffer number included.  Two ways to make that safe,
            // in order of preference: fold an all-literal initialiser to a single
            // literal (no buffer survives, and the value stops being rebuilt per use),
            // or leave the block for `rebind_constant_buffer` to re-point onto a buffer
            // the pasting function owns.  Anything neither can handle is refused here,
            // at the declaration, rather than pasted as numbering that means something
            // else wherever it lands.
            if matches!(tp.base(), Type::Text(_)) && matches!(val.unspan(), Value::Block(_)) {
                if let Some(folded) = self.fold_text_constant(&val) {
                    val = Value::Text(folded);
                    // The block's TYPE named that buffer too (`text["b"]`).  A literal
                    // depends on nothing, and a dep left pointing at the declaration's
                    // numbering makes the text-return path promote a variable the using
                    // function does not have.
                    tp = Type::Text(crate::data::Deps::none());
                } else if !Self::constant_block_is_rebindable(&val) && !self.first_pass {
                    let fn_name = id.to_lowercase();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "text constant '{id}' is assembled in a way that cannot be pasted at a use site — a constant is inlined at every reference, and this initialiser builds its value across more than one buffer.  Use a zero-argument function instead: `fn {fn_name}() -> text {{ … }}`, then call `{fn_name}()`"
                    );
                }
            }
            // loft#744 — the same policy, for the carrier the text case does not
            // cover. A constant is pasted at each use, so ANY frame slot its
            // initialiser carries is numbering that means something else wherever
            // it lands. Text has two ways out above (fold, or rebind); a value that
            // needs a TEMPORARY has neither — `const R = item_at(0).s_r;` holds a
            // work-ref for the struct the call returns, and pasting that number
            // into `main` named a slot that was not there. It SIGSEGV'd at the
            // REFERENCE, after the program had run, so it read as a crash in
            // whatever was executing rather than as a bad constant.
            //
            // Refused HERE, at the declaration, for the reason the text case gives
            // — and because that is the line worth naming. The report that found
            // this had no file, no line and no mention of the constant.
            //
            // `visit_constant_vars` is exhaustive, so a new IR variant that carries
            // a variable arrives here rather than silently pasting. A scalar call
            // (`const N = count();`) carries no slot and is unaffected, which is
            // the shape the reporter actually wanted.
            // Text and Vector have their own answers above; a Reference (a
            // struct-VALUED constant, `POINT_NONE = Point { x: 1 }`) has its own
            // refusal too, and that message names the limitation better than this
            // one would. This covers what is left: a constant whose VALUE is a
            // scalar but whose initialiser needs a temporary to compute it.
            if !self.first_pass
                && !matches!(
                    tp.base(),
                    Type::Text(_) | Type::Vector(_, _) | Type::Reference(_, _)
                )
            {
                let mut carries_a_slot = false;
                let mut probe = val.clone();
                let accounted = visit_constant_vars(&mut probe, &mut |_| carries_a_slot = true);
                if carries_a_slot || !accounted {
                    let fn_name = id.to_lowercase();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "constant '{id}' has an initialiser that needs a temporary, and a \
                         constant is inlined at every reference — the slot it names belongs \
                         to this declaration, not to the function it is pasted into.  A call \
                         returning a struct is the usual cause.  Use a zero-argument function \
                         instead: `fn {fn_name}() -> … {{ … }}`, then call `{fn_name}()`"
                    );
                }
            }
            // Not when the constant has already been refused: the error prescribes the
            // same function, and a second line saying the initialiser is re-evaluated
            // describes a constant that will not compile at all.
            if !self.first_pass
                && !refused_as_unbuildable
                && crate::keys::const_effect_lint_enabled()
                && let Some(callee) = self.const_initialiser_cost(&val)
            {
                {
                    let fn_name = id.to_lowercase();
                    diagnostic!(
                        self.lexer,
                        Level::Advice,
                        code = "const-reevaluated",
                        "constant '{id}' is re-evaluated at EVERY reference — a file-scope \
                         constant is an inlined expression, not a once-computed value, so \
                         `{callee}` runs again for each use"
                    );
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Conditional,
                        title: format!("compute it once in a function that caches: `fn {fn_name}() -> … {{ … }}`"),
                        condition: Some("the initialiser is expensive or has an effect you want to happen once".to_string()),
                        edit: None,
                        concept: "const",
                        concept_ref: "@F18",
                    });
                }
            }
            if self.first_pass {
                // detect a name collision before calling `add_def`,
                // which would otherwise panic with `Dual definition of <name>`.
                let mut existing = self.data.def_nr(&id);
                // @PLN22 Phase 2 — shadow a prelude/import name of the same key.
                if self.prelude_shadowed(&id) {
                    existing = u32::MAX;
                }
                if existing == u32::MAX {
                    let c_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Constant);
                    self.data.set_returned(c_nr, tp);
                    self.data.definitions[c_nr as usize].code = val;
                } else {
                    let prev_pos = self.data.def(existing).position().clone();
                    let prev_kind =
                        format!("{:?}", self.data.def(existing).def_type()).to_lowercase();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "constant '{id}' conflicts with a {prev_kind} of the same name \
                         already defined at {prev_pos} — pick a different name"
                    );
                }
            } else {
                // The SECOND pass's initialiser is the one that gets pasted — for every
                // kind of constant, not just the vector one.
                //
                // A constant is stored as IR and inlined at each reference, so whatever
                // the declaration resolved to is what every use gets.  Pass 1 resolves it
                // against an INCOMPLETE definition table by construction: a name declared
                // later in the file, or in a sibling module the aggregator has not
                // finished exporting, is not there yet.  `create_var` answers `u16::MAX`
                // at file scope (no frame to allocate in), so the unresolved name froze
                // into the stored IR as `Var(u16::MAX)` and the type froze as `Unknown`.
                // Pass 2 derived the right value and threw it away.
                //
                // What that cost, all from one mechanism (loft#962): an integer or single
                // constant panicked the variable allocator at `index 65535`, a text or
                // boolean one printed EMPTY, and one initialised from a function declared
                // below it printed `null` — no diagnostic on any of the silent three.
                //
                // loft#702 fixed the vector kind alone, for the same reason in a different
                // costume (pass-1 field offsets are placeholders).  One rule for one fact:
                // pass 2 sees every declaration, so pass 2's answer wins.  The type is
                // re-stored with it — `const N = later() * 2;` picks its operator from the
                // callee's return type, which pass 1 does not have.
                //
                // Not the struct-valued kind.  That one is REFUSED at the declaration
                // above — a record cannot be materialised at each use — so its stored IR
                // is never pasted by a program that compiles, and re-storing it only
                // moves the diagnostics of one that does not.
                let c_nr = self.data.def_nr(&id);
                if c_nr != u32::MAX
                    && self.data.def(c_nr).def_type() == DefType::Constant
                    && !matches!(tp.base(), Type::Reference(_, _))
                {
                    // Written straight into the slot rather than through `set_returned`,
                    // which is set-once: `data.reset()` between passes keeps definitions,
                    // so pass 1's answer is still sitting there and this is a REPLACEMENT.
                    self.data.definitions[c_nr as usize].returned = tp;
                    self.data.definitions[c_nr as usize].code = val;
                }
            }
            self.lexer.token(";");
            true
        } else {
            false
        }
    }

    /// Read the function name after `fn`.  In user code only identifiers are accepted.
    /// In the default library, `assert` and `panic` are also allowed even though they are
    /// keywords — they remain real functions with call-site file/line injection.
    fn parse_fn_name(&mut self) -> Option<String> {
        if let Some(name) = self.lexer.has_identifier() {
            return Some(name);
        }
        if self.default {
            if self.lexer.has_token("assert") {
                return Some("assert".to_string());
            }
            if self.lexer.has_token("panic") {
                return Some("panic".to_string());
            }
        }
        diagnostic!(
            self.lexer,
            Level::Error,
            "Expect name in function definition"
        );
        None
    }

    #[allow(clippy::too_many_lines)]
    // @F16 — functions & declarations (pub, parameters, return)
    pub(crate) fn parse_function(&mut self) -> bool {
        if !self.lexer.has_token("fn") {
            return false;
        }
        let Some(fn_name) = self.parse_fn_name() else {
            return false;
        };
        self.vars = Function::new(&fn_name, &self.lexer.pos().file);
        // @PLN110 3a — var numbers are per-function, so a stale `len(X)` binding from
        // the previous body would attach to an unrelated local here.
        self.len_bound_locals.clear();
        // @PLN25 E2 — clear any type-var from a previous function before parsing
        // this one; set below if this function is generic.
        self.cur_type_var = u32::MAX;
        self.cur_type_var_name.clear();
        if !self.default && !is_lower(&fn_name) && !is_op(&fn_name) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect function names to be in lower case style"
            );
        }
        // detect `<T>` type parameter after function name.
        // @F25 — generics: single type variable <T>, inferred
        let mut is_generic = false;
        let mut type_var_name = String::new();
        // I4: bound names collected from `<T: A + B>` — resolved to def_nrs in the second pass.
        let mut pending_bounds: Vec<String> = Vec::new();
        if self.lexer.has_token("<") {
            if let Some(tv) = self.lexer.has_identifier() {
                if !is_camel(&tv) && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Type variable '{}' must be CamelCase",
                        tv
                    );
                }
                type_var_name = tv;
                is_generic = true;
                // I4: parse `<T: A + B>` bound list; collect raw names here, resolve in second pass.
                if self.lexer.has_token(":") {
                    loop {
                        if let Some(bound_name) = self.lexer.has_identifier() {
                            pending_bounds.push(bound_name);
                        } else if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Expected interface name in type bound"
                            );
                        }
                        if !self.lexer.has_token("+") {
                            break;
                        }
                    }
                }
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expected type variable name after '<'"
                );
            }
            self.lexer.closing_angle();
        }
        let mut arguments = Vec::new();
        if self.lexer.token("(") {
            // register the type variable as a struct so parse_type
            // resolves it to Reference(d, []).  The definition is never
            // compiled — it only exists for the template's type resolution.
            if is_generic {
                let bounds_key = Self::type_var_bounds_key(&pending_bounds);
                let claimed = self
                    .type_var_holders
                    .get(&(type_var_name.clone(), bounds_key.clone()))
                    .copied();
                let existing = self.data.def_nr(&type_var_name);
                // A prior generic's type-var placeholder is an attribute-less `Struct`, safe to
                // reuse (that is how `<T>` is shared across functions). Any OTHER existing def
                // — a constant (e.g. `E`), a function, an enum, or a real struct/type — is a
                // COLLISION: loft has one flat namespace, so a generic parameter cannot share a
                // name. Report it (mirroring the `type X conflicts with …` diagnostic) instead
                // of silently binding the parameter to that def and panicking later in
                // `predict_generic_return_type`.
                let collision = claimed.is_none()
                    && existing != u32::MAX
                    && !(self.data.def(existing).def_type() == DefType::Struct
                        && self.data.def(existing).attributes().is_empty());
                if collision {
                    if self.first_pass {
                        let ed = self.data.def(existing);
                        let prev_pos = ed.position().clone();
                        let prev_kind = format!("{:?}", ed.def_type()).to_lowercase();
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "generic type parameter '{type_var_name}' conflicts with a \
                             {prev_kind} of the same name already defined at {prev_pos} — \
                             pick a different name"
                        );
                    }
                    // Stop treating the function as generic so the unresolved parameter never
                    // reaches the generic type-resolution path (which would panic).
                    is_generic = false;
                } else if let Some(holder) = claimed {
                    // This exact `(spelling, bounds)` header has been seen — on the other
                    // pass, or in another function declaring the same variable the same way.
                    self.cur_type_var = holder;
                } else {
                    // `(G-Gen)`: this header INTRODUCES the variable.  It may reuse the
                    // placeholder the spelling already names, but only while that placeholder
                    // stands for the same bound set — a second bound set is a second variable
                    // and needs a placeholder of its own, because the placeholder is what keys
                    // the bound-method stubs (loft#1300, loft#1301).
                    let reusable = existing != u32::MAX
                        && self
                            .type_var_bounds
                            .get(&existing)
                            .is_none_or(|b| *b == bounds_key);
                    let holder = if reusable {
                        existing
                    } else if !self.first_pass {
                        // Placeholders are minted on the first pass; reaching here on the
                        // second means the first refused this header, and there is nothing
                        // to bind.
                        u32::MAX
                    } else {
                        // register the type variable as a struct so parse_type
                        // resolves it to Reference(d, []).  The definition is never
                        // compiled — it only exists for the template's type resolution.
                        //
                        // Under its own spelling while that is free; otherwise under a name
                        // the source cannot write, since `#` is not an identifier character,
                        // so `T#2` is reachable only through this header.
                        //
                        // Uniqueness is asked PROGRAM-WIDE, not of this source: the
                        // placeholder is registered as a store structure under
                        // `__typevar_<name>`, and that registry is not keyed by source.
                        let mut name = type_var_name.clone();
                        let mut n = 1;
                        while self.data.name_taken_anywhere(&name) {
                            n += 1;
                            name = format!("{type_var_name}#{n}");
                        }
                        let tv_nr = self.data.add_def(&name, self.lexer.pos(), DefType::Struct);
                        self.data
                            .set_returned(tv_nr, Type::Reference(tv_nr, crate::data::Deps::none()));
                        tv_nr
                    };
                    if holder != u32::MAX {
                        self.type_var_holders
                            .insert((type_var_name.clone(), bounds_key.clone()), holder);
                        self.type_var_bounds.insert(holder, bounds_key);
                    }
                    self.cur_type_var = holder;
                }
            }
            // @PLN25 E2 — the type-var def_nr is recorded above (valid in both passes: the
            // placeholder is added on the first and found again on the second) so
            // `e2_nullable_elem` leaves a generic `vector<T>` dense.  It is also what
            // `parse_type` resolves the spelling to from here on, which is what keeps two
            // headers writing `T` apart.
            if is_generic {
                self.cur_type_var_name.clone_from(&type_var_name);
            }
            if !self.parse_arguments(&fn_name, &mut arguments) {
                return true;
            }
            self.lexer.token(")");
        }
        // The entry point takes nothing, or the invocation arguments as one `vector<text>`.
        //
        // `State::execute_argv` fills exactly that one shape: it pushes a TEXT vector before
        // the return address when `main` declares a single vector parameter, and pushes
        // nothing otherwise.  Every other signature was still accepted and simply never
        // filled — `main(who: text)` read `""`, two integers read whatever the frame held,
        // and a `text` among two crashed on a corrupt store reference.  A `vector` of any
        // OTHER element type is the same fault one step on: the text vector is pushed into a
        // slot typed for something else (loft#1172).
        //
        // Refused rather than filled, because none of these shapes does anything today: there
        // is no argument to lose.  Reading them is `args: vector<text>`.
        if !self.default && !self.first_pass && fn_name == "main" {
            let visible: Vec<&crate::data::Argument> = arguments
                .iter()
                .filter(|a| !a.name.starts_with("__work_") && !a.name.starts_with("__ref_"))
                .collect();
            let supported = visible.is_empty()
                || (visible.len() == 1
                    && matches!(&visible[0].typedef,
                                Type::Vector(inner, _) if matches!(inner.base(), Type::Text(_))));
            if !supported {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`main` takes no parameters, or one `vector<text>` for the invocation \
                     arguments — `fn main(args: vector<text>)`.  Any other signature is never \
                     filled, so it would read empty or worse"
                );
            }
        }
        // validate that the type variable appears in the first parameter.
        if is_generic && !arguments.is_empty() {
            // The HEADER's variable, not whatever the spelling names globally: two headers
            // writing `T` bind two placeholders, and the parameter was resolved against
            // this one.
            let has_tv = arguments[0].typedef.contains_def(self.cur_type_var);
            if !has_tv && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Type variable {} must appear in the first parameter — \
                     move {} to the first parameter position",
                    type_var_name,
                    type_var_name
                );
            }
        } else if is_generic && arguments.is_empty() && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Generic function must have at least one parameter of type {}",
                type_var_name
            );
        }
        self.context = if self.default && self.first_pass && is_op(&fn_name) {
            self.data.add_op(&mut self.lexer, &fn_name, &arguments)
        } else if self.first_pass {
            let d = self.data.add_fn(&mut self.lexer, &fn_name, &arguments);
            if is_generic && d != u32::MAX {
                self.data.definitions[d as usize].def_type = DefType::Generic;
            }
            // @PLN99 Arc C — a user-defined conversion `fn OpConvXFromY` / `fn OpCastXFromY`
            // is a global stored `n_OpConv…`, so it skipped `add_op` and never entered the
            // `possible` map that `convert`/`cast` search.  Register it (by type-matched
            // prefix) so `value as T` and implicit conversions dispatch a user `S → T`,
            // exactly like a built-in — the loop in `convert` still matches on arg/return type.
            if d != u32::MAX {
                if fn_name.starts_with("OpConv") {
                    self.data.register_possible("OpConv", d);
                } else if fn_name.starts_with("OpCast") {
                    self.data.register_possible("OpCast", d);
                }
            }
            d
        } else if self.default && is_op(&fn_name) {
            self.data.def_nr(&fn_name)
        } else {
            self.data.get_fn(&fn_name, &arguments)
        };
        if self.context == u32::MAX {
            return false;
        }
        // @PLN86 §7.2 (F7) — now the function's def_nr exists, key each parsed parameter
        // `…#default` lock by `(this fn, param index)`.  First pass only (definitions
        // resolve there); the call-site gate reads `param_locks` on the second pass.
        if self.first_pass {
            for (idx, token) in std::mem::take(&mut self.pending_param_locks) {
                self.param_locks.insert((self.context, idx as u32), token);
            }
        }
        // @PLN115 tail — now `self.context` (the fn's def_nr) is known, record each
        // parameter's DECLARATION occurrence: `Local{fn_def, var_nr}` at the signature
        // name.  The param arg-index IS its var_nr (pass 2 re-types params by index —
        // `change_var_type(a_nr, …)`), so no var-table lookup is needed.  Pass 2 only
        // (the recording pass); populated only when recording (else the vec is empty).
        if !self.first_pass {
            for (idx, pos, len) in std::mem::take(&mut self.pending_param_positions) {
                self.record_decl(
                    &pos,
                    len,
                    crate::resolution::Resolution::Local {
                        fn_def: self.context,
                        var_nr: idx,
                    },
                );
            }
        }
        // @PLN86 step 1.2 — record the sandbox profile for a host-designated
        // function so the admission walk (and the nesting guard, 0.1) know this
        // def is restricted.  Designation is host-controlled — `fn:<name>`, or a
        // path selector matching this def's source file (#631) — never from the
        // source, so a script cannot mark itself.  Re-derived on every pass
        // (def_sandbox is cleared at parse start).
        let src_file = self.data.def(self.context).position().file.clone();
        if let Some(profile) = self
            .sandbox
            .designation_for(&fn_name, &src_file)
            .map(str::to_string)
        {
            self.def_sandbox.insert(self.context, profile);
            // @PLN86 step 0.1 — enter restricted parsing for this def's body: the
            // nesting guard activates and its depth state starts fresh.
            self.in_sandbox = true;
            self.parse_depth = 0;
            self.depth_overflowed = false;
        } else {
            self.in_sandbox = false;
        }
        // Plan-17 phase 01 (B) — bound resolution + t-stub creation now
        // happens on BOTH passes.  Before, this block was gated on
        // `!self.first_pass`, leaving `definitions[ctx].bounds` empty
        // on first pass.  The body parser's bounded-T method-dispatch
        // path (`fields.rs::field` I7) then couldn't find the t-stub,
        // returned `Type::Unknown(0)`, and the receiving variable
        // stayed Unknown — `s + "!"` after `s = x.to_text()` then
        // failed with "No matching operator '+' on 'unknown(0)' and
        // 'text'".  By resolving on first pass too, the t-stub exists
        // when the body parses on first pass and the dispatch returns
        // the bound's declared return type.
        //
        // First-pass forward-decl tolerance: if a bound's interface
        // hasn't been declared yet (forward reference), `def_nr` returns
        // u32::MAX.  We skip the diagnostic on first pass (it'll fire
        // again on second pass with all defs visible) but still install
        // any bounds we CAN resolve so the body can dispatch.
        // I4: resolve pending bound names to interface def_nrs.
        if !pending_bounds.is_empty() {
            let mut bounds = Vec::new();
            for bname in &pending_bounds {
                let b_nr = self.data.def_nr(bname);
                if b_nr == u32::MAX {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "'{}' is not a known interface",
                            bname
                        );
                    }
                    // First pass: silent skip — interface may be a
                    // forward declaration; second pass will catch
                    // genuinely-unknown ones.
                } else if !matches!(self.data.def_type(b_nr), DefType::Interface) {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "'{}' is not an interface — bounds must be interface names",
                            bname
                        );
                    }
                } else {
                    bounds.push(b_nr);
                }
            }
            self.data.definitions[self.context as usize].bounds = bounds;
            // I7/I8.1: Create T-parameterized stubs for each bound interface's methods so
            // the body parser can emit `Value::Call(t_stub_nr, ...)` for method/op calls on T.
            // `re_resolve_call` then substitutes these with the concrete type's implementation.
            let iface_nrs: Vec<u32> = self.data.definitions[self.context as usize].bounds.clone();
            self.create_bound_method_stubs(self.cur_type_var, &iface_nrs);
        }
        let mut returned_not_null = false;
        let mut result = if self.lexer.has_token("->") {
            // Will be the correct def_nr on the second pass
            if let Some(tp) = self.parse_type_full(self.data.def_nr(&fn_name), true) {
                if self.has_deprecated_not_null() {
                    returned_not_null = true;
                }
                tp
            } else {
                // message
                Type::Void
            }
        } else {
            Type::Void
        };
        // @PLN102 Phase 3 (N-Domain) — the domain-partial math fns are declared `-> τ?` in the
        // stdlib (they yield the reserved null out of their real domain). When LOFT_NULLFLOW is
        // OFF, strip the `?` so their return stays non-null and the default surface is byte-
        // identical until the flag flips default-on. Self-contained fns only (no desugar cascade):
        // sqrt / asin / acos. (pow / log — and their desugar consumers exp / ln / log2 / log10 —
        // land with the constant-in-domain elision, step 3.5.)
        if !crate::keys::ndomain_enabled()
            && matches!(result, Type::Optional(_))
            && matches!(
                fn_name.as_str(),
                "sqrt" | "asin" | "acos" | "ln" | "log" | "log2" | "log10" | "pow"
            )
        {
            result = result.base().clone();
        }
        // @PLN86 P6.2 — the call-gate capability link in the SIGNATURE, after the output
        // (`-> int fs#read`, or a void fn's `) fs#update`): a first-class part of the
        // contract beside the params + return, NOT in the `#native`/`#impure`/`#wasm`
        // implementation plumbing.  A restricted caller needs this granted to CALL the
        // function (`admit_capabilities`); passing arguments is part of the call.  After
        // an output, only `;` / `{` / a link is legal, so a leading identifier is
        // unambiguously a link.  Re-read each pass, so set unconditionally.
        if let Some(token) = self.try_cap_link() {
            self.data.definitions[self.context as usize].cap = token;
        }
        // Plan-14 phase 07 (P234 runtime): when the declared return type
        // is `Type::Tuple(elems)` and any element carries a lifetime
        // concern (Text, Reference, Vector, Enum-struct, keyed
        // collection, RefVar, or a nested tuple containing one of
        // those), rewrite the return type to `Reference(__tuple<…>)`
        // — the synthetic struct that loft already creates via
        // `data.tuple_def(...)` for stored tuples (P189b path).
        // The function then returns a DbRef and all existing
        // `ref_return` / `text_return` ownership-transfer machinery
        // applies unchanged.  Pure-value tuples skip the rewrite and
        // keep using Rust's tuple ABI (the T1.8a path for shapes
        // like `(integer, integer)`).
        //
        // Skip for generic templates — `T` resolves later to a concrete
        // type which may or may not have lifetime concerns; rewriting
        // pre-specialisation would freeze the wrong shape.  Mirrors the
        // generic-template guard in `block_result` (control.rs ~line 426)
        // that excludes generic templates from `ref_return` /
        // `text_return`.
        let is_generic_template =
            self.context != u32::MAX && self.data.def_type(self.context) == DefType::Generic;
        // A7.1: a pure-value tuple wider than the 8-byte primitive return slot is
        // boxed the same way, but ONLY for a par worker (loft#808).  Par dispatch
        // carries a worker result home through per-route buffers that cover ≤8-byte
        // primitives, text, fn-refs and references and nothing else, so a bare
        // `(integer, integer)` return has nowhere to ride; routing it through the
        // synthetic struct puts it on the reference route with the lifetime-bearing
        // case from Phase 07.  Safe after P236's fix (work-ref unification across If
        // branches in `parser/control.rs::unify_if_branches_work_refs`); without it,
        // `min_max(...) -> (integer, integer) { if cond { (a, b) } else { (c, d) } }`
        // loses the if/else's value on `--native`, because each branch's separate
        // synthetic-struct work-ref drops it.
        //
        // Everywhere ELSE the boxing is pure cost: it turns `(float, float, float)`
        // into a store record claimed and freed on every call, so the SAME arithmetic
        // ran ~5.6x slower crossing a function boundary than inline (loft#808 measured
        // 728ms vs 129ms on `--native-release`; the local compiles to a real Rust
        // `(f64, f64, f64)`).  A tuple return keeps Rust's tuple ABI unless it is a
        // worker's.  See `Parser::par_worker_defs` for why the set is complete here on
        // pass 2 and topped up between the passes.
        //
        // P196 follow-up (2026-05-12): exclude tuples that contain a
        // `Type::Function` element from the size>8 trigger.  Function
        // values are 16 bytes (u32 d_nr + DbRef closure ref), so any
        // tuple containing one trips size>8 even when the OTHER
        // elements are pure primitives — but the synthetic struct
        // wrapping breaks at the assignment site `Pair { v: pp }` where
        // `Pair.v: (fn, integer)` stays as a bare tuple type but
        // `pp: Reference(__tuple<fn, integer>)` after the rewrite.
        // The `has_lifetime_concern` arm still fires for Text / Reference /
        // Vector / etc. elements that genuinely need by-reference passing;
        // only the size-driven trigger is narrowed.
        let needs_tuple_rewrite = matches!(&result, crate::data::Type::Tuple(elems)
            if elems.iter().any(crate::data::has_lifetime_concern)
                || (self.par_worker_defs.contains(&self.context)
                    && u32::from(crate::variables::size(&result, &crate::data::Context::Argument)) > 8
                    && !elems.iter().any(|e| matches!(e, crate::data::Type::Function(_, _, _)))));
        // @PLN85 generic-tuple-return-fix.md — a generic template whose return SHAPE
        // is already concrete (`-> (text, text)`, no `T` in any element) is not the
        // "T resolves later" case the skip guards; let it ride the same promotion the
        // non-generic gets so the monomorph inherits it (sites 1 + 3 + block_result).
        // Only a shape that DEPENDS on `T` (`-> (T, T)`) defers to instantiation.
        let generic_return_promotable =
            !is_generic_template || !self.return_shape_depends_on_type_var(&result);
        if generic_return_promotable && needs_tuple_rewrite {
            result = self.boxed_tuple_return(result);
        }
        self.vars
            .append(&mut self.data.definitions[self.context as usize].variables);
        if self.first_pass {
            self.data.set_returned(self.context, result);
            self.data.definitions[self.context as usize].returned_not_null = returned_not_null;
        }
        // Dep inference for native methods: if a native fn (no body, `;`-terminated)
        // has a `self` parameter and returns the same struct-enum type, the return
        // borrows from self's store.  Mark dep=[0] (self attribute) so
        // inline_struct_return can distinguish accessors (dep non-empty, borrow)
        // from constructors (dep empty, own).
        if self.first_pass && self.lexer.peek_token(";") {
            let def = &self.data.definitions[self.context as usize];
            if let Some(self_attr) = def.attributes().first()
                && self_attr.name == "self"
                && let Type::Enum(ret_nr, true, dep) = def.returned()
                && dep.is_empty()
                && let Type::Enum(self_nr, true, _) = &self_attr.typedef
                && ret_nr == self_nr
            {
                self.data.definitions[self.context as usize].returned =
                    Type::Enum(*ret_nr, true, crate::data::Deps::attrs(vec![0]));
            }
        }
        if !self.lexer.has_token(";") {
            for (a_nr, a) in arguments.iter().enumerate() {
                if self.first_pass {
                    let v_nr = self.create_var(&a.name, &a.typedef);
                    if v_nr != u16::MAX {
                        self.vars.become_argument(v_nr);
                        self.var_usages(v_nr, false);
                        // @PLN40 const-model — `p: const T` (const before the type) is
                        // VALUE-const: a read-only borrow.  Mutation through `p` is
                        // rejected (step 3 base-resolution), but a rebind `p = other`
                        // re-points the local slot and is allowed.  Set on BOTH passes:
                        // a text arg's read-only-borrow auto-promotion (parse_assign_op)
                        // is decided on the FIRST pass, so the flag must exist by then to
                        // suppress promotion and let the write reach the const guard.
                        if a.constant {
                            self.vars.set_value_const(v_nr);
                        }
                    }
                } else {
                    self.change_var_type(a_nr as u16, &a.typedef);
                    if a.constant {
                        self.vars.set_value_const(a_nr as u16);
                    }
                }
            }
            // @PLAN59 / H1 phase 1 — the unconditional heap-return buffer:
            // every BODY-carrying plain fn returning Reference / Vector /
            // struct-Enum gets its hidden `__retbuf` attribute + backing
            // argument var at signature parse, so arity is fixed before ANY
            // caller parses.  `ref_return` binds a promoted local to it by
            // renaming the ATTR and retiring this placeholder var (probes
            // C3/C6 in plans/59-return-abi).  Excluded: native decls (`;`,
            // handled by the enclosing branch), ops / `#rust`-templated fns
            // (implemented in Rust, never promoted — their ABI must not
            // change), generic templates (specialisations never promote,
            // I9-var), lambdas (separate parse path, no earlier callers).
            if self.first_pass
                && generic_return_promotable
                && matches!(
                    self.data.def_type(self.context),
                    DefType::Function | DefType::Generic
                )
                && self.data.def(self.context).rust().is_empty()
                // loft#938 gate 1 of 5 — `ret_promo_base` peels `Optional(Vector)` so a
                // NULLABLE collection return gets the buffer too.  Identity while
                // `LOFT_NULLABLE_RETBUF` is off, which is the default.
                && matches!(
                    self.data.def(self.context).returned().ret_promo_base(),
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                )
            {
                // The buffer's own type is the BASE: it is storage, and storage is never
                // absent.  The RETURN keeps its `?` — a null answer is a value the caller
                // reads, not a buffer it fails to receive.
                let ret = self
                    .data
                    .def(self.context)
                    .returned()
                    .ret_promo_base()
                    .clone();
                let a =
                    self.data
                        .add_attribute(&mut self.lexer, self.context, "__retbuf", ret.clone());
                self.data.definitions[self.context as usize].attributes[a].hidden = true;
                let v = self.create_var("__retbuf", &ret);
                if v != u16::MAX {
                    self.vars.become_argument(v);
                    self.var_usages(v, false);
                    self.vars.mark_used(v);
                }
            }
            // re-apply name remaps for promoted text arguments in second pass.
            if !self.first_pass {
                for (shadow, original) in self.vars.promoted_text_args() {
                    let orig_name = self.vars.name(original).to_string();
                    self.vars.remap_name(&orig_name, shadow);
                    // Mark original as used so test_used doesn't warn.
                    self.vars.mark_used(original);
                }
                // Plan-22 phase 02d-iii.e — ACTIVATE the type
                // flip.  After this point, every mutated-scalar-
                // capture local in this function carries
                // `Reference(__cell_<T>, [])` instead of its
                // original scalar type.  Lambdas inside the body
                // will snapshot the flipped type into
                // `capture_context`, so phase 02c's auto-Reference
                // closure-record encoding fires correctly.
                //
                // Reads (parent body + closure body): wrapped by
                // 02d-iii.b's `auto_deref_boxed_scalar` hook in
                // `parse_var`.  Writes (parent body): wrapped by
                // 02d-iii.d's `maybe_prepend_cell_alloc` hook in
                // `parse_assign_op` (alloc on first-set; existing
                // `towards_set` → `call_to_set_op` machinery
                // emits the OpSet<T> via the auto-deref'd LHS
                // pattern).  Writes (closure body): handled
                // for free by the same machinery.
                //
                // The void-return write-back path in
                // `parse_call_ref` is gated to skip boxed-scalar
                // attributes — propagation goes through the
                // shared cell DbRef, not via per-call slot copy.
                self.flip_scalars_to_box_types();
                // #318 sink R1: a fn cannot RETURN a closure-carrying
                // struct — the value's closure record holds raw DbRefs
                // into this frame's stores, which die at return (the
                // caller then silently corrupts whatever reuses the
                // slots).  Checked in pass 2 only: by then pass 1 has
                // recorded every capturing assignment on the struct's
                // attributes, so the predicate is complete.  Returning
                // a BARE capturing closure stays supported (the case-C
                // factory transfer owns the record + captures).
                let returned = self.data.def(self.context).returned().clone();
                if self.type_carries_closure(&returned) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "function returns a struct type that holds a capturing closure; \
                         the closure references state owned by this function's frame, so \
                         the value cannot outlive it — construct the struct in the frame \
                         that owns the captured state and pass it down, or return the \
                         closure itself (#318)"
                    );
                }
            }
            self.parse_code();
            // #314 — pass-1 sibling of the pass-2 flip above: now that
            // the whole body is parsed, `scalars_to_box` is final;
            // reject any mutated scalar that more than one closure
            // captures (GOALS.md § "Stability trumps features").
            if self.first_pass {
                // #687 — same moment, same reason: a capture's STORAGE also depends on
                // facts that are only final now (whether the binding ended up with a
                // hidden out-parameter of its own).  Runs first: the rejection below
                // consumes this parent's lambda list.
                self.finalize_capture_storage(self.context);
                self.box_nested_capture_attrs(self.context);
                self.reject_shared_mutable_scalar_captures(self.context);
            }
            // reset transient closure state after each function body.
            // Without this, a lambda inside make_adder leaks last_closure_work_var
            // into the next function parsed (main), causing closure_var_of to
            // return a stale value for add5 = make_adder(5).
            self.last_closure_work_var = u16::MAX;
            if !self.first_pass {
                self.check_ref_mutations(&arguments);
                // Plan-06 PRIORITY.md spine step 5 — analyse each
                // `let r = parallel_for(...)` site for materialising
                // uses of r and emit a deprecation warning pointing
                // users at the streaming form (fused for-par) or
                // explicit `par_to_vec(...)` (planned phase 11).
                self.check_par_result_singlepass();
            }
        }
        if !self.first_pass {
            // Stub functions with an empty body `{ }` and a `self` parameter are intentional
            // skips (e.g. to silence the "no implementation for variant" warning).
            // Don't warn about unused parameters in that case.
            let is_stub = {
                let def = &self.data.definitions[self.context as usize];
                let body_empty = matches!(def.code(), Value::Block(bl) if bl.operators.is_empty());
                // @F19 — method dispatch via self / both
                let first_is_self = def.attributes().first().is_some_and(|a| a.name == "self");
                body_empty && first_is_self
            };
            // Each warning pass below seeks the lexer to a diagnostic site
            // (`lexer.to(..)`) without rewinding the actual read cursor.
            // `to()` only moves the *reporting* line/pos, but the tokenizer
            // increments that line on every subsequent physical line it
            // pulls — so a backward seek here silently offsets the line
            // number of every diagnostic emitted for the code that follows
            // (e.g. a later dead-assignment).  Save the true position and
            // restore it once the passes finish so they are position-neutral.
            let warn_pos = self.lexer.at();
            // @PLN107 S1 — observable dead-store classification dump (gated on
            // LOFT_DUMP_READS; no warning). Runs before test_used so the read/write-target
            // split can be inspected against the corpus.
            {
                let ctx = self.context as usize;
                let body = self.data.definitions[ctx].code();
                let fn_name = self.data.definitions[ctx].name();
                self.vars.debug_dead_store_dump(fn_name, body, &self.data);
            }
            let body = self.data.definitions[self.context as usize].code().clone();
            if !is_stub {
                self.vars
                    .test_used(&mut self.lexer, &self.data, &body, self.context);
            }
            // P246 follow-up — UPPER_CASE locals without `const`
            // violate the "UPPER_CASE means immutable constant"
            // convention.  Run once per function in the second pass
            // (after const_param flags are settled).  Takes the body for the
            // same reason `test_used` does: a name the code never mentions is
            // a pass-1 placeholder, not a local of this function.
            self.vars.warn_upper_case_locals(&mut self.lexer, &body);
            // Plan-07 phase 4e.2 — undefended fault-site warning.
            // Walks this function's body looking for fault-prone op
            // calls (OpDivInt / OpRemInt / OpGetVector / OpVectorRef /
            // OpTextCharacter) that survived the 4d.1 / 4d.2 / 4e.1
            // swap passes; emits `Level::Warning` unless an easy-proof
            // skip pattern applies.  Silenceable via
            // `LOFT_NO_WARN_RUNTIME=1` env var.  Second-pass only —
            // first pass doesn't have the swap-pass results yet.
            self.warn_undefended_fault_sites(&body);
            // @PLN87 P3 (W4) — a `&` on a heap struct param that is never reassigned
            // has no effect (field mutation propagates regardless).
            self.warn_redundant_amp(&body);
            // @PLN46 W3 — auto-infer `#null_safe` from entry guards (after the warn
            // pass, so this fn's flag is set for LATER callers' walks).
            self.infer_function_null_safe(&body);
            self.warn_function_complexity();
            self.warn_parameter_count();
            self.warn_boolean_flag_cluster();
            self.lexer.to(warn_pos);
        }
        self.lexer.has_token(";");
        self.parse_rust();
        self.check_drop_signature();
        self.data.op_code(self.context);
        self.data.definitions[self.context as usize]
            .variables
            .append(&mut self.vars);
        self.context = u32::MAX;
        // @PLN86 step 0.1 — leave restricted parsing; trusted top-level code that
        // follows is never depth-guarded.
        self.in_sandbox = false;
        true
    }

    /// @PLN24 arc A — `#c "<symbol>" "<c-signature>"`.
    ///
    /// Stores both strings and checks the signature against the loft
    /// declaration it annotates. Every problem is reported, not just the first:
    /// an author fixing a signature wants the whole list, and each message
    /// names the C position so a long parameter list stays navigable.
    ///
    /// The check is the entire safety of this feature. A `#c` call has no
    /// runtime signal — the probe pointed the caller at a wrong arity and at a
    /// variadic function and both returned the *right answer* — so a mistake
    /// that gets past here gets past everything.
    fn parse_c_binding(&mut self) {
        // The cursor drifts to the NEXT definition while the two strings are
        // read, so the annotation's own position is captured up front and every
        // message below points at it — a signature error that pointed at the
        // following `fn` would send the author to the wrong line.
        let at = self.lexer.pos().clone();
        let Some(symbol) = self.lexer.has_cstring() else {
            diagnostic_at!(
                self.lexer,
                &at,
                Level::Error,
                "Expect the C symbol after #c, e.g. #c \"PQstatus\" \"int(void*)\""
            );
            return;
        };
        let Some(sig_src) = self.lexer.has_cstring() else {
            diagnostic_at!(
                self.lexer,
                &at,
                Level::Error,
                "Expect the C signature after the symbol, e.g. #c \"{symbol}\" \"int(void*)\" — \
                 the signature is required because nothing at runtime can check the binding"
            );
            return;
        };
        let ctx = self.context as usize;
        self.data.definitions[ctx].c_symbol.clone_from(&symbol);
        self.data.definitions[ctx].c_sig.clone_from(&sig_src);
        if self.first_pass {
            // Parameter types are not resolved yet on pass 1, so checking here
            // would report against half-built types.  The strings are stored;
            // pass 2 does the checking.
            return;
        }
        let target = crate::c_signature::CTarget::host();
        let sig = match crate::c_signature::CSignature::parse(&symbol, &sig_src, target) {
            Ok(s) => s,
            Err(e) => {
                diagnostic_at!(self.lexer, &at, Level::Error, "#c signature: {e}");
                return;
            }
        };
        let def = self.data.def(self.context);
        // The SAME filter the two marshallers use (`c_call::register` and
        // `generation::output_c_direct_call`). It has to be: they derive the
        // slot assignment from this list, so a list that differs here would
        // check one binding and call another.
        let params: Vec<crate::data::Type> = def
            .attributes()
            .iter()
            .filter(|a| !a.name.starts_with("__") && !a.name.starts_with('#'))
            .map(|a| a.typedef.clone())
            .collect();
        let ret = def.returned().clone();
        let void_return = matches!(ret, crate::data::Type::Void);
        for problem in crate::c_signature::check(&self.data, &sig, &params, &ret, void_return) {
            diagnostic_at!(self.lexer, &at, Level::Error, "#c \"{symbol}\": {problem}");
        }
    }

    // <rust> ::= { '#rust' <string> | '#iterator' <string> <string> }
    // <native> ::= '#native' <string>   (any file)
    pub(crate) fn parse_rust(&mut self) {
        loop {
            if !self.lexer.peek_token("#") {
                break;
            }
            // Speculatively consume `#`; revert if the annotation is not recognised.
            let link = self.lexer.link();
            self.lexer.has_token("#");
            let id = self.lexer.has_identifier();
            if id == Some("native".to_string()) {
                // Capture the cstring's position BEFORE consuming it — once
                // `has_cstring()` returns, the lexer has advanced past the
                // closing quote onto the next token, so a diagnostic at the
                // current cursor would point at the NEXT declaration instead
                // of the offending annotation.
                let sym_pos = self.lexer.peek().position;
                if let Some(sym) = self.lexer.has_cstring() {
                    // Explicit override — for the rare case where the native
                    // symbol differs from the loft fn name (e.g. a
                    // `foo_native` loft wrapper backing the `n_foo` symbol).
                    // @PLAN12 — if the explicit string EQUALS the canonical
                    // default (the fn's own stored name, `n_<fn>` for a free
                    // fn), it's redundant: a bare `#native` derives the same
                    // symbol.  Reject it so the redundant form can't drift
                    // back in — bare `#native` is the only spelling for the
                    // common case; an explicit string is reserved for a
                    // symbol that genuinely DIFFERS from the fn name.
                    let canonical = self.data.def(self.context).name().to_string();
                    if sym == canonical {
                        diagnostic_at!(
                            self.lexer,
                            &sym_pos,
                            Level::Error,
                            "redundant `#native \"{sym}\"` — the symbol equals the \
                             function name; write a bare `#native` instead (an \
                             explicit string is only for a symbol that differs)"
                        );
                    }
                    self.data.definitions[self.context as usize].native = sym;
                } else {
                    // @PLAN12 — bare `#native` defaults the symbol to the
                    // function's own name.  A free `fn foo(...)` is stored as
                    // `n_foo` (see `Data::add_fn`), which IS the conventional
                    // native symbol, so the binding needs no separate string.
                    let default_sym = self.data.def(self.context).name().to_string();
                    self.data.definitions[self.context as usize].native = default_sym;
                }
            } else if self.default && id == Some("rust".to_string()) {
                if let Some(c) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust = c;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust string");
                }
            } else if self.default && id == Some("iterator".to_string()) {
                if let Some(init) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust = init;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust init string");
                }
                if let Some(next) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].rust += "#";
                    self.data.definitions[self.context as usize].rust += &next;
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expect rust next string");
                }
            } else if id == Some("c".to_string()) {
                // @PLN24 arc A — `#c "<symbol>" "<c-signature>"` binds this
                // declaration straight to a C symbol.  Both strings are
                // required, and the SIGNATURE is the load-bearing one: the
                // architecture probe (tests/fixtures/c_abi/) showed a runtime
                // caller cannot detect a wrong arity or a variadic mismatch —
                // both returned the right answer by luck — so a `#c` binding is
                // checked here or nowhere.
                //
                // Arc A lands INERT: this parses, checks and stores.  Nothing
                // calls a `#c` function yet, and `native` is deliberately left
                // empty so the Rust dispatch path does not pick it up.
                self.parse_c_binding();
            } else if id == Some("null_safe".to_string()) {
                // @PLN46 W2 — `#null_safe` asserts every nullable parameter
                // tolerates null and yields a defined result, so a fault-prone
                // expression (`s[i]`) passed DIRECTLY as an argument is not flagged
                // at the call site (the possible-null is the callee's contract).
                self.data.definitions[self.context as usize].null_safe = true;
            } else if id == Some("superseded".to_string()) {
                // @PLN102 arc C — `#superseded "Y"` marks this callable as
                // superseded by the successor symbol Y (a bare name, e.g.
                // `write_through`).  Step 1 only PARSES + STORES the name;
                // nothing reads it yet, so it is inert (byte-identical bytecode).
                // A later step steers an OWNED-source caller toward Y, and a
                // `make ci` lint checks Y resolves and this body is a shim over it.
                if let Some(succ) = self.lexer.has_cstring() {
                    self.data.definitions[self.context as usize].superseded = succ;
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Expect #superseded successor name string, e.g. #superseded \"write_through\""
                    );
                }
            } else if id == Some("pure".to_string()) {
                // Plan-06 phase 5a (DESIGN.md D8.1): `#pure`
                // declares "no observable side effects, no
                // parent-store writes".  Always par-safe.
                self.data.definitions[self.context as usize].purity = crate::data::Purity::Pure;
            } else if id == Some("impure".to_string()) {
                // Plan-06 phase 5a (DESIGN.md D8.1):
                // `#impure(category)` classifies the side effect.
                // Five categories: host_io, prng, io, parent_write,
                // par_call.
                if !self.lexer.has_token("(") {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Expect '(' after #impure — use #impure(host_io|prng|io|parent_write|par_call)"
                    );
                    self.lexer.revert(link);
                    break;
                }
                let category = self.lexer.has_identifier();
                let cat = match category.as_deref() {
                    Some("host_io") => crate::data::ImpureCategory::HostIo,
                    Some("prng") => crate::data::ImpureCategory::Prng,
                    Some("io") => crate::data::ImpureCategory::Io,
                    Some("parent_write") => crate::data::ImpureCategory::ParentWrite,
                    Some("par_call") => crate::data::ImpureCategory::ParCall,
                    other => {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown #impure category {:?} — expected host_io|prng|io|parent_write|par_call",
                            other.unwrap_or("(missing)")
                        );
                        self.lexer.revert(link);
                        break;
                    }
                };
                if !self.lexer.has_token(")") {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Expect ')' after #impure category"
                    );
                    self.lexer.revert(link);
                    break;
                }
                self.data.definitions[self.context as usize].purity =
                    crate::data::Purity::Impure(cat);
            } else {
                // Not a recognised annotation — put the `#` back and stop.
                self.lexer.revert(link);
                break;
            }
        }
    }

    /// @PLN125 arc B — check a just-parsed `OpDrop`, the hook a type runs at scope end.
    ///
    /// `OpDrop` is a reserved name rather than an attribute, for the same reason `to_text`,
    /// `OpAdd`, `next` and `OpIndex` are: every other first-grade surface keys behaviour to
    /// a TYPE by the method's name, and the scope-end hook is one more of those.  (The
    /// original sketch proposed a `#drop` attribute; attributes describe a function's
    /// implementation — `#pure`, `#native` — and this describes a type's contract.)
    ///
    /// Two things are checked here because neither can be recovered from later:
    ///
    /// **It must not answer.** A drop runs at a closing brace with no caller left to tell,
    /// and loft has no runtime errors (C80), so a result would go nowhere.  That is a real
    /// semantic weakening and it is the design: anything whose failure MATTERS stays an
    /// explicit call (`tx.commit()` answers; the scope end does not).  Better to say so at
    /// the declaration than to let an author write a `-> boolean` nobody reads.
    ///
    /// **It must take exactly the receiver.** A drop is called by the compiler, so there is
    /// nowhere for a second argument to come from.
    fn check_drop_signature(&mut self) {
        if self.context == u32::MAX || self.first_pass {
            return;
        }
        let def = self.data.def(self.context);
        if !def.name().ends_with("_OpDrop") {
            return;
        }
        let declared = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && !a.name.starts_with("__"))
            .count();
        let returns = !matches!(def.returned(), Type::Void);
        // The whole body is parsed before either check runs, so the cursor has already
        // reached the NEXT declaration — reporting at it sends the reader to an unrelated
        // function that the message never mentions.  Point at the `OpDrop` itself.
        let at = def.position().clone();
        if returns {
            diagnostic_at!(
                self.lexer,
                &at,
                Level::Error,
                "`OpDrop` cannot return — it runs at scope end with no caller to answer; \
                 anything whose failure matters stays an explicit call"
            );
        }
        if declared != 1 {
            diagnostic_at!(
                self.lexer,
                &at,
                Level::Error,
                "`OpDrop` takes only `self` — the compiler calls it, so a second argument \
                 has nowhere to come from"
            );
        }
    }

    pub(crate) fn parse_arguments(&mut self, fn_name: &str, arguments: &mut Vec<Argument>) -> bool {
        // @PLN86 §7.2 (F7) — collect this list's `…#default` parameter locks fresh; the
        // caller (`parse_function`) records them once the function's def_nr exists.
        self.pending_param_locks.clear();
        // @PLN115 tail — likewise collect each parameter's name position; the def_nr /
        // var_nr are not established until after this list, so the DECLARATION
        // occurrence is recorded in `parse_function` (param arg-index == var_nr).
        self.pending_param_positions.clear();
        loop {
            if self.lexer.peek_token(")") {
                break;
            }
            // Capture the parameter name's position before it is consumed (only when
            // recording — `Position` holds a `String`, so no clone on a normal compile).
            let attr_pos = self
                .record_resolutions
                .then(|| self.lexer.peek_pos().clone());
            let Some(attr_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect attribute");
                return false;
            };
            if let Some(pos) = attr_pos {
                self.pending_param_positions.push((
                    arguments.len() as u16,
                    pos,
                    attr_name.chars().count() as u16,
                ));
            }
            if !is_lower(&attr_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect function attributes to be in lower case style"
                );
            }
            for a in arguments.iter() {
                if attr_name == a.name {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Double attribute '{fn_name}.{attr_name}'"
                    );
                }
            }
            let mut constant = false;
            let mut reference = false;
            // loft#1003 — the modifier's OWN position, taken before the token is consumed
            // (`peek_pos` is the start of what is about to be read, where `pos` is the scan
            // cursor already past it).  `needless-reference-parameter` and
            // `needless-const-parameter` both cure by deleting exactly this token, and this
            // is the only point in the parse where it has a position: the checks run after
            // the body, from the variable's source, which is inside the body.  Without it the
            // caret pointed at the wrong construct and neither fix could carry an edit.
            let mut ref_pos = (0, 0);
            let mut const_pos = (0, 0);
            let typedef = if self.lexer.has_token(":") {
                let at_ref = self.lexer.peek_pos().clone();
                if self.lexer.has_token("&") {
                    reference = true;
                    ref_pos = (at_ref.line, at_ref.pos);
                }
                // Will be the correct def_nr on the second pass
                let at_const = self.lexer.peek_pos().clone();
                if self.lexer.has_keyword("const") {
                    constant = true;
                    const_pos = (at_const.line, at_const.pos);
                }
                if let Some(tp) = self.parse_type_full(self.data.def_nr(fn_name), false) {
                    // @PLN25 E2/E3 — a `vector<Struct>` PARAM is already rewritten by
                    // the vector-type-resolution chokepoint (`sub_type` `vector` arm),
                    // so no per-site hook here.
                    if reference {
                        // loft#1006 — a `&(…)` reference tuple reaches its elements through the
                        // tuple's stored DbRef with the `(ref, offset)` ops a struct field uses,
                        // and only the SCALAR kinds are laid out for that. `ref_var_type` is the
                        // one place that decides, shared with the annotated-local and `b = &a`
                        // positions so a `&` refused here cannot be accepted there.
                        self.ref_var_type(tp)
                    } else {
                        tp
                    }
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expecting a type");
                    return true;
                }
            } else {
                Type::Unknown(0)
            };
            // if this parameter has `= expr`, the expression may
            // reference earlier parameters of the same function.  Inject
            // those earlier params into `self.vars` before parsing the
            // default, track which var_nr each maps to, then rewrite the
            // parsed Value tree so references use the *argument index*
            // (0, 1, …) rather than the parser's internal var_nr.
            // `fill_defaults` in src/parser/mod.rs::substitute_param_refs
            // replaces `Var(argument_index)` with the caller's actual arg.
            let injected: Vec<(String, u16, u16)> = if self.lexer.peek_token("=") {
                let mut mapping = Vec::new();
                for (i, a) in arguments.iter().enumerate() {
                    if a.typedef.is_unknown() {
                        continue;
                    }
                    if self.vars.var(&a.name) != u16::MAX {
                        continue;
                    }
                    let v = self.vars.add_variable(&a.name, &a.typedef, &mut self.lexer);
                    if v != u16::MAX {
                        self.vars.become_argument(v);
                        self.vars.defined(v);
                        mapping.push((a.name.clone(), v, i as u16));
                    }
                }
                mapping
            } else {
                Vec::new()
            };
            let val = if self.lexer.has_token("=") {
                let dpos = self.lexer.pos().clone();
                // loft#699 — where the default's value is BUILT decides whether it can be
                // replayed at a call site.  Mark the source position first: one that turns
                // out to need a temporary is re-parsed from here into a function of its own.
                let value_start = self.lexer.link();
                let unresolved_before = self.unresolved_names;
                let unresolved_types_before = self.unresolved_types;
                let mut t = Value::Var(arguments.len() as u16);
                // loft#1067 — the parameter's declared type is the expected type for its
                // DEFAULT, exactly as it is for an argument a caller passes: a default is
                // checked against it a few lines below, and `fn takes(f: fn(integer) ->
                // integer = |x| { x * 2 })` has no other way to say what `x` is.
                let saved_expected = std::mem::replace(&mut self.expected, Type::Unknown(0));
                if Self::seeds_lambda_hint(&typedef) {
                    self.expected = typedef.base().clone();
                }
                let dtype = self.expression(&mut t);
                self.expected = saved_expected;
                // @PLN102 arc-E (E2 Tier-0): type-check + coerce the default
                // expression against the parameter type, exactly as a call-site
                // argument is (`convert` then `validate_convert`, mod.rs:5907).
                // An unchecked default (`text = 42`) otherwise reaches runtime and
                // the interpreter uses the wrong-typed value as a pointer → SIGSEGV;
                // `float = 5` here coerces to `5.0` just as `f(5)` would.  Only a
                // definite KNOWN-vs-KNOWN mismatch is rejected; the check is skipped
                // when either side is not a concrete type to check against:
                //   * a literal `null` default — the internal "no default" sentinel
                //     and a valid absent-value default for any type (C80 in-band
                //     null), e.g. a `&Data = null` mutable-reference param;
                //   * an `unknown` default type — an untyped literal that takes its
                //     type from the parameter (e.g. `&vector<integer> = []`, an
                //     empty vector whose element type is only fixed by context);
                //   * an `unknown` parameter type (nothing to check against).
                // A `Rewritten(T)` marker wraps an untyped literal whose type is
                // fixed by context later (e.g. `[]` parses as `Rewritten(Unknown)`);
                // look through it so such a literal counts as unknown here.
                let dtype_concrete = match &dtype {
                    Type::Rewritten(inner) => inner.as_ref(),
                    other => other,
                };
                // A by-reference param (`&T`, `Type::RefVar`) is exempt: `convert`
                // would ref-coerce the default into a `text_ref`-style block that
                // takes `&Var(slot)` of the DEFINING frame — valid transiently at a
                // call site but a dangling reference once the default is stored and
                // re-injected at each caller (segfaults at runtime).  Such a default
                // is kept raw here; the reference is taken at injection time (the
                // pre-check behaviour).  The check still guards by-VALUE params, which
                // is where the wrong-type-as-pointer SIGSEGV (`text = 42`) lives.
                if !typedef.is_unknown()
                    && !dtype_concrete.is_unknown()
                    && !matches!(t, Value::Null)
                    && !matches!(typedef, Type::RefVar(_))
                    && !self.convert(&mut t, &dtype, &typedef)
                {
                    self.validate_convert("default value", &dtype, &typedef, &dpos);
                }
                // Rewrite Var(injected_slot) → Var(arg_index) so the stored
                // default is portable across call sites.
                for (_name, slot, arg_idx) in &injected {
                    t = Self::remap_var_nr(t, *slot, *arg_idx);
                }
                // loft#699 — a parameter default is stored on the SIGNATURE and replayed
                // in the CALLER's frame, so the only names that survive are the earlier
                // parameters `substitute_param_refs` transplants.  A default needing a
                // temporary numbered one in THIS function's table, and that index then
                // resolved against the caller's locals: `= [1, 2]` tripped the
                // database-reference assert, `= "a" + "b"` returned the wrong text on the
                // interpreter and would not compile natively, `= []` on a `hash` read
                // garbage.  Give it a function of its own, exactly as loft#698 does for a
                // field default, and store the call — the earlier parameters it references
                // become that function's own arguments, so nothing crosses tables.
                //
                // A by-reference param keeps its default RAW (see the `convert` exemption
                // above): `add_defaults`'s `RefVar` arm appends it into the buffer it
                // mints, so there is no crossing to remove.
                let site = DefaultSite::Parameter {
                    count: arguments.len() as u16,
                };
                //
                // A default that named something pass 1 could not resolve is lowered too,
                // however simple its tree looks.  What pass 1 parsed is what a call site
                // replays — the pass-2 re-parse is discarded — so a forward-referenced
                // constant froze the collapsed pass-1 reading: `b: integer = a + LATER`
                // stored just `a` and answered 5 where 15 was written, silently
                // (loft#1086).  Lowering it puts the value in a function BODY, and a body
                // is re-parsed in pass 2, where the name resolves.
                let dflt_fn = self.default_fn_name(fn_name, arguments, &attr_name);
                let named_a_forward_reference =
                    self.first_pass && self.unresolved_names != unresolved_before;
                // The same collapse reached WITHOUT losing a name.  A call to a function
                // declared below resolves its name — definitions are recorded before bodies
                // are parsed — while its RETURN TYPE is not linked yet, so the counter above
                // stays still and pass 1 froze a reading it would have got right on the
                // second: `= 1 + late(0)` stored the bare `1`, and `= true && late(0)`
                // answered `false` and left the interpreter a short stack (loft#1170).
                let typed_against_a_forward_declaration =
                    self.first_pass && self.unresolved_types != unresolved_types_before;
                let hoisted_in_pass_1 =
                    !self.first_pass && self.minted_default_nr(&dflt_fn, arguments) != u32::MAX;
                if !matches!(typedef, Type::RefVar(_))
                    && (named_a_forward_reference
                        || typed_against_a_forward_declaration
                        || hoisted_in_pass_1
                        || !default_replayable_in_place(&t, site))
                {
                    // Pass 1's signature is the one the function HAS, so pass 2 re-parses
                    // the body against those parameters rather than against a list
                    // recomputed from a tree that now resolves more names than pass 1's
                    // did.  And a pass-1 tree that lost a name to a forward reference
                    // cannot be asked which earlier parameters the default reads, so it
                    // takes them all — an over-wide signature is wasteful where a short
                    // one fails to compile.
                    let (params, call_args) = self
                        .hoisted_default_signature(&dflt_fn, arguments)
                        .unwrap_or_else(|| {
                            // A tree that LOST a node cannot be asked which earlier
                            // parameters the default reads, and an unresolved TYPE drops one
                            // the same way `call_op_as` does — so both take them all.  An
                            // over-wide signature is wasteful where a short one fails to
                            // compile.
                            if named_a_forward_reference || typed_against_a_forward_declaration {
                                Self::every_earlier_parameter(arguments, &injected)
                            } else {
                                Self::default_fn_params(&t, arguments, &injected)
                            }
                        });
                    self.lexer.revert(value_start);
                    t = self.default_value_fn(&dflt_fn, &params, &typedef, call_args);
                }
                t
            } else {
                Value::Null
            };
            for (name, _, _) in &injected {
                self.vars.remove_name(name);
            }
            // @PLN86 §7.2 (F7) — an optional `group#default` lock after the parameter
            // (its default): `count: int = 1 spawn.count#default`.  Consumed on BOTH
            // passes (else the second pass chokes on the token); the index is the slot
            // this parameter is about to occupy (`arguments.len()`).  `try_cap_link` is
            // non-destructive, so a `,`/`)` after the default is never mis-consumed.
            if let Some(token) = self.try_cap_link() {
                self.pending_param_locks.push((arguments.len(), token));
            }
            if !self.first_pass
                && typedef.is_unknown()
                && val == Value::Null
                && (!self.default || !matches!(typedef, Type::Vector(_, _)))
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expecting a clear type, found {}",
                    typedef.name(&self.data)
                );
            }
            (*arguments).push(Argument {
                name: attr_name,
                typedef,
                default: val,
                constant,
                ref_pos,
                const_pos,
            });
            if !self.lexer.has_token(",") {
                break;
            }
        }
        true
    }

    pub(crate) fn parse_fn_type(&mut self, d_nr: u32) -> Type {
        let mut r_type = Type::Void;
        let mut args = Vec::new();
        self.lexer.token("(");
        loop {
            if self.lexer.peek_token(")") {
                break;
            }
            if let Some(tp) = self.parse_type_full(d_nr, false) {
                args.push(tp);
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        if self.lexer.has_token("->")
            && let Some(tp2) = self.parse_type_full(d_nr, false)
        {
            r_type = tp2;
        }
        Type::Function(args, Box::new(r_type), crate::data::Deps::none())
    }

    // <type> ::= <identifier> [::<identifier>] [ '<' ( <sub_type> | <type> ) '>' ] [ <depend> ] [ '?' ]
    //
    // @PLN25 Phase 0 (EXPAND): accept a postfix `?` on ANY scalar / struct type
    // (`integer?`, `text?`, `S?`) as the nullable opt-in. Today plain types are
    // ALREADY nullable by default, so `?` is a behaviour-preserving no-op — its
    // job in EXPAND is purely to let nullable sites be pre-annotated (MIGRATE)
    // before the Phase-2 default flip gives the marker teeth. The vector ELEMENT
    // `?` (`vector<S?>`) is consumed earlier in `sub_type_inner` (before this
    // returns), so this wrapper never steals it — it only catches the outer `?`.
    /// @PLN125 arc A step A2b — resolve a `Self.X` associated-type reference.
    ///
    /// `base` is the type just parsed.  Answers `Some(placeholder)` only when all of it
    /// holds: the parser is inside an interface body, `base` is that interface's `Self`,
    /// the next token is `.`, and the name after it was declared by a `type` line in this
    /// interface.  Anything else leaves the lexer untouched and answers `None`, so no
    /// existing spelling changes meaning.
    ///
    /// A `.` after `Self` naming something NOT declared is an error rather than a silent
    /// fallthrough: the alternative is the A1 symptom, where the leftover `.X` desynced the
    /// interface-body loop and reported "Expected 'fn' in interface body" twice, pointing
    /// at neither the cause nor the line the author has to change.
    fn self_assoc_type(&mut self, base: &Type) -> Option<Type> {
        let iface = self.context;
        if iface == u32::MAX || !matches!(self.data.def_type(iface), DefType::Interface) {
            return None;
        }
        let self_nr = self.data.def_nr("Self");
        if self_nr == u32::MAX || !matches!(base, Type::Reference(nr, _) if *nr == self_nr) {
            return None;
        }
        if !self.lexer.has_token(".") {
            return None;
        }
        let Some(name) = self.lexer.has_identifier() else {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect an associated type name after 'Self.'"
                );
            }
            return None;
        };
        let a_nr = self
            .data
            .def_nr(&format!("{}.{name}", self.data.def(iface).name()));
        if a_nr == u32::MAX {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}' has no associated type '{name}' — declare it with `type {name}` \
                     in the interface body",
                    self.data.def(iface).name()
                );
            }
            return Some(Type::Unknown(0));
        }
        Some(self.data.def(a_nr).returned().clone())
    }

    pub(crate) fn parse_type(
        &mut self,
        on_d: u32,
        type_name: &str,
        returned: bool,
    ) -> Option<Type> {
        let t = self.parse_type_inner(on_d, type_name, returned)?;
        // @PLN125 arc A step A2b — `Self.X`, an interface's ASSOCIATED TYPE used in one
        // of its own method signatures:
        //
        //   interface SqlDb {
        //     type Rows: SqlRows
        //     fn select(self: Self, sql: text) -> Self.Rows
        //   }
        //
        // Resolves to the placeholder the `type` line registered, which is what lets the
        // signature parse and the generic body type against a name rather than a concrete
        // def.  Inert: the placeholder behaves as any interface-scoped type does today, and
        // A2c is what binds it to the implementor's companion per monomorph.  Only fires
        // inside an interface body on the `Self` type — anywhere else `.` after a type is
        // not this construct, and is left to fail as it always has.
        if let Some(assoc) = self.self_assoc_type(&t) {
            return Some(assoc);
        }
        if self.lexer.has_token("?") {
            // @PLN101 — a `value struct` is stored INLINE (bytes, no `DbRef`), so it has no
            // `store_nr` null sentinel: `<value struct>?` cannot be represented. Reject it with
            // a clear diagnostic; fall through as the plain (non-null) type to avoid a cascade.
            if let Type::Reference(p, _) = &t
                && self.data.is_value_struct(*p)
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{type_name}?` is not allowed — a `value struct` is stored inline and has \
                     no null; use a plain `{type_name}`, or a reference `struct` for nullability"
                );
                return Some(t);
            }
            // @PLN25 slice (a): the postfix `?` constructs the real `Optional` former
            // (idempotent + normalising via `Type::optional`). GATED on `LOFT_PLN25_OPT`
            // while the slice-(b) peel audit is incomplete — OFF keeps the Phase-0 no-op
            // (suite byte-identical), ON surfaces the remaining consuming-site mis-routes.
            if crate::keys::pln25_optional_enabled() {
                return Some(Type::optional(t));
            }
            // Gate OFF — Phase-0 behaviour: Integer records nullable explicitly; other
            // scalars have no flag yet, so accept-and-ignore (the base `t` falls through).
            if let Type::Integer(spec) = &t {
                return Some(Type::Integer(IntegerSpec {
                    not_null: false,
                    ..*spec
                }));
            }
        }
        Some(t)
    }

    // `pub(crate)` so the `as`-cast (operators.rs) can parse the target type
    // WITHOUT the postfix-`?` consumer — the cast detects the `?` itself to tell
    // `as τ` (fit-checked) from `as τ?` (checked cast); see DN4.
    pub(crate) fn parse_type_inner(
        &mut self,
        on_d: u32,
        type_name: &str,
        returned: bool,
    ) -> Option<Type> {
        // Phase 2c round 10c: `long` has been removed as a user-facing
        // type.  Callers now use `integer` everywhere; if anyone still
        // writes `long` it parses as an unknown identifier and fails
        // normally via the standard `data.def_nr` lookup path below.
        let tp_nr = if self.lexer.has_token("::") {
            if let Some(name) = self.lexer.has_identifier() {
                let source = self.data.get_source(type_name);
                self.data.source_nr(source, &name)
            } else {
                diagnostic!(self.lexer, Level::Error, "Expect type from {type_name}");
                return None;
            }
        } else {
            // `(G-Gen)` — the enclosing generic header gets first refusal on the spelling
            // (loft#1300, loft#1301).
            self.def_nr_in_scope(type_name)
        };
        if self.first_pass && tp_nr == u32::MAX && type_name != "spatial" {
            // @P296-sibling — for a qualified `lib::Type` reference, `tp_nr`
            // was computed as `source_nr(source, name)` (the type, e.g.
            // `CellSnap`), but the pass-1 placeholder is keyed on
            // `type_name` (the lib prefix, e.g. `audience_crystal`).  When
            // the lib isn't resolvable yet in pass-1 (e.g. Windows lib-path
            // resolution lands the load after this reference), a SECOND
            // unresolved `lib::Other` ref would re-`add_def(lib_prefix)` →
            // "Dual definition" panic.  Reuse an existing same-named Unknown
            // placeholder instead of re-adding it; pass-2 resolves the real
            // type once the lib is fully parsed.  (The non-qualified path
            // already dedups via `tp_nr` below, so this only affects the
            // qualified shape.)
            let existing = self.data.def_nr(type_name);
            let u_nr = if existing != u32::MAX && self.data.def_type(existing) == DefType::Unknown {
                existing
            } else {
                self.data
                    .add_def(type_name, self.lexer.pos(), DefType::Unknown)
            };
            return Some(Type::Unknown(u_nr));
        }
        if tp_nr != u32::MAX && self.data.def_type(tp_nr) == DefType::Unknown {
            return Some(Type::Unknown(tp_nr));
        }
        let link = self.lexer.link();
        if self.lexer.has_token("<")
            && let Some(value) = self.sub_type(on_d, type_name, link)
        {
            return Some(value);
        }
        let mut dep = Vec::new();
        self.parse_depended(returned, &mut dep);
        let mut min = i32::MIN + 1;
        let mut max = i32::MAX as u32;
        if type_name == "integer" {
            let has_limit = self.parse_type_limit(&mut min, &mut max);
            // T1.7: check for `not null` annotation after the integer type
            let not_null = self.has_deprecated_not_null();
            if has_limit || not_null {
                // Phase 2c round 10c — all integer ranges stay as Type::Integer
                // (i64 storage + i64 arithmetic at rest).  Narrow-bounded
                // ranges (u8/u16/i8/i16/i32-range) get packed storage via
                // `forced_size`; wide ranges (up to u32::MAX) use full
                // 8-byte storage.  Type::Long is no longer produced.
                return Some(Type::Integer(IntegerSpec {
                    min,
                    max,
                    not_null,
                    forced_size: None,
                }));
            }
        }
        let dt = self.data.def_type(tp_nr);
        if tp_nr != u32::MAX
            && matches!(
                dt,
                DefType::Type | DefType::Enum | DefType::EnumValue | DefType::Struct
            )
        {
            if matches!(dt, DefType::EnumValue)
                || (self.first_pass && matches!(dt, DefType::Struct))
            {
                Some(Type::Reference(tp_nr, crate::data::Deps::unknown(dep)))
            } else if matches!(self.data.def(tp_nr).returned(), Type::Text(_)) {
                Some(Type::Text(crate::data::Deps::unknown(dep)))
            } else {
                // when a user-typed integer alias carries an
                // explicit `size(N)` annotation (e.g. `i32`, `u8`, `u16`),
                // stamp the forced width onto the returned Type::Integer so
                // the signal flows through `Box<Type>` in `Type::Vector` /
                // `Hash` / `Sorted` / `Index` to the element resolver
                // (Phase 2) and the indexing codegen (Phase 3).
                //
                // Skip the base `integer` primitive: its `forced_size = 8`
                // matches the default heuristic; stamping would clutter
                // every `Type::Integer` with `Some(8)` for no benefit.
                let mut tp = self.data.def(tp_nr).returned().clone();
                if type_name != "integer"
                    && let Type::Integer(mut spec) = tp
                    && let Some(forced) = self.data.forced_size(tp_nr)
                    && let Some(nz) = std::num::NonZeroU8::new(forced)
                    && forced != 8
                {
                    spec.forced_size = Some(nz);
                    tp = Type::Integer(spec);
                }
                Some(tp)
            }
        } else {
            None
        }
    }

    /// Parse a type expression that may be a tuple `(T1, T2, ...)` or an identifier-based type.
    /// This is the entry point for type positions (return types, parameter types, annotations).
    pub(crate) fn parse_type_full(&mut self, on_d: u32, returned: bool) -> Option<Type> {
        if self.lexer.has_token("(") {
            // Tuple type: (T1, T2, ...)
            let mut types = Vec::new();
            loop {
                if self.lexer.peek_token(")") {
                    break;
                }
                if let Some(tp) = self.parse_type_full(on_d, false) {
                    types.push(tp);
                } else {
                    break;
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token(")");
            if types.len() < 2 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Tuple types require at least 2 elements"
                );
                return types.into_iter().next();
            }
            // Plan-06 phase 4d: register the synthetic `__tuple<…>`
            // struct as soon as the tuple type appears in a type
            // position (struct-field declaration, parameter, return
            // type, …).  Without this, `fill_database` later sees
            // `Type::Tuple` with `type_elm == u32::MAX` and silently
            // skips registering the host struct's tuple field —
            // leaving `database.position("v")` as `u16::MAX` when
            // codegen needs the host field offset.  Mirrors the
            // `sub_type` tuple arm below.  Idempotent.
            self.data.tuple_def(&mut self.lexer, &types);
            // A `?` after the closing paren.  `(N-Opt)` licenses `τ?` for every τ, but a tuple
            // is the one type former with NO representation for absence: `(L-Null)`'s sentinel
            // needs a value the type reserves and a tuple reserves none, and `(L-Null-Tag)`'s
            // discriminant is for a STRUCT stored inline.  The `?` was left unconsumed here, so
            // every position spelled it as a syntax cascade naming nothing — *"Expect token ;"*
            // for a local, *"Expect token )"* for a parameter, *"Expect token >"* inside a
            // `vector<…>`.  Consume it and name the refusal wherever the type is parsed, the
            // way the `value struct` case five lines into `parse_type` does — the other type
            // with no null to spend.  NOT gated on the second pass: a local's and a field's
            // declared type reach this branch on pass 1 only, so a pass-2 gate reports for a
            // parameter and stays silent for the two positions an author writes most often.
            // A `?` after `)` is a syntactic fact, so no resolution can change the answer.
            // `formal/types-history.md` D-Opt-NoNull; loft#1423 carries the design question of
            // giving a tuple the tagged representation.
            if self.lexer.has_token("?") {
                let spelled = Type::Tuple(types.clone()).name(&self.data);
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{spelled}?` is not allowed — a tuple is its members' bytes, with no \
                     sentinel and no discriminant to spend on absence; make the MEMBERS nullable \
                     (`(integer?, text?)`), or wrap the tuple in a `struct`, which can be `?`"
                );
            }
            Some(Type::Tuple(types))
        } else if self.lexer.has_token("fn") {
            Some(self.parse_fn_type(on_d))
        } else if let Some(id) = self.lexer.has_identifier() {
            self.parse_type(on_d, &id, returned)
        } else {
            None
        }
    }

    pub(crate) fn sub_type(&mut self, on_d: u32, type_name: &str, link: Link) -> Option<Type> {
        let tp = self.sub_type_inner(on_d, type_name, link)?;
        // #318 sink R3: no collection of a closure-carrying struct.
        // An element copy embeds a closure record whose raw DbRefs
        // point into the constructing frame (silent corruption once
        // the frame dies and slots are reused) — the same reason the
        // plan-15 matrix CLOSED `vector<capturing fn>`; a struct
        // wrapper was a loophole around that decision.  Checked in
        // pass 2 (layout complete).
        if !self.first_pass
            && crate::parser::vectors::is_collection(&tp)
            && self.type_carries_closure(&tp)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "collection of a struct type that holds a capturing closure is not \
                 supported — element copies would dangle into the constructing \
                 function's frame; keep closure holders in local variables and pass \
                 them down as arguments (#318)"
            );
        }
        Some(tp)
    }

    fn sub_type_inner(&mut self, on_d: u32, type_name: &str, link: Link) -> Option<Type> {
        // Plan-06 phase 4d.A — accept tuple as the inner type of
        // `vector<(T1, T2, ...)>` (and reserve the same shape for
        // `iterator<(T1, T2)>` once that lands).  Without this, the
        // identifier-only check below would reject `(` and the parser
        // would mis-parse the rest of `<(...)>` as a less-than
        // expression on a bare `vector` type.
        if self.lexer.peek_token("(") {
            let tp = self.parse_type_full(on_d, false)?;
            // P189: register a synthetic tuple struct so the rest of
            // the type system (fill_database, type_def_nr, vector_of)
            // can treat the tuple identically to a named struct.
            // Idempotent — the same tuple shape resolves to the same
            // def_nr across the program.
            if let Type::Tuple(types) = &tp {
                self.data.tuple_def(&mut self.lexer, types);
            }
            return Some(match type_name {
                "vector" => {
                    self.lexer.closing_angle();
                    Type::Vector(Box::new(tp), crate::data::Deps::none())
                }
                "iterator" => {
                    self.lexer.closing_angle();
                    Type::Iterator(Box::new(tp), Box::new(Type::Null))
                }
                _ => {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "{type_name}<(...)> not supported — tuple element types only \
                         allowed on vector / iterator"
                    );
                    self.lexer.has_closing_angle();
                    Type::Unknown(0)
                }
            });
        }
        // Plan-06 phase 4d.A.2 — recognise `fn` as a type-keyword inside
        // `vector<...>` and `iterator<...>`.  Before this, the parser
        // saw `vector<fn(integer) -> integer>`, sub_type's
        // identifier-only check rejected `fn`, the lexer reverted
        // past `<`, and the caller's annotation parser
        // (`parse_assign:1009`) entered a tight retry loop on the
        // unconsumed `<` — `loft --dump file.loft` hung at 100% CPU.
        //
        // Storage uses 4-byte i32 d_nr — `data::type_def_nr` and
        // `type_elm` route Type::Function to `i32`'s def_nr, and
        // `parser/vectors::get_type` returns `database.int(0, false)`
        // so vector elements are written as raw 4-byte d_nrs (the same
        // path `vector<i32>` uses).  At read-back time, the par
        // dispatcher's `read_tuple_at_wide` (with a single-element
        // `[Type::Function]` shape) inflates each row's 4 bytes into
        // the worker's 20-byte fn-ref slot ([8B i64 d_nr][12B null
        // closure DbRef]).
        if self.lexer.peek_token("fn") {
            self.lexer.token("fn");
            let tp = self.parse_fn_type(on_d);
            return Some(match type_name {
                "vector" => {
                    self.lexer.closing_angle();
                    Type::Vector(Box::new(tp), crate::data::Deps::none())
                }
                "iterator" => {
                    self.lexer.closing_angle();
                    Type::Iterator(Box::new(tp), Box::new(Type::Null))
                }
                _ => {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "{type_name}<fn(...)> not supported — fn-ref element types \
                         only allowed on vector / iterator"
                    );
                    self.lexer.has_closing_angle();
                    Type::Unknown(0)
                }
            });
        }
        if let Some(sub_name) = self.lexer.has_identifier() {
            // @PLN25 storage-vs-access-nullability — a POSTFIX `?` on the element type
            // (`vector<S?>`) is the nullable-element opt-IN. Dense is the default: a
            // struct / enum element stores inline; `S?` synthesises the `__nullable<S>`
            // enum. The `?` is a flag on the type, not the headline — hence postfix,
            // pairing with `x ?? d`. Keyed collections stay dense (a key denotes
            // presence) — `?` there is an error.
            let nullable_elem = self.lexer.has_token("?");
            if nullable_elem && type_name != "vector" {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`?` (nullable element) is only valid on `vector` — `{type_name}` \
                     elements are always dense (a key / slot denotes presence)"
                );
            }
            // before trying to resolve the element type, fail fast if the
            // identifier shadows a non-type definition (constant, function).
            // parse_type silently returns None in that case; sub_type's later
            // assert!(self.first_pass) masks the issue in pass 1 and
            // typedef.rs::fill_database panics later when a struct-def happens
            // to carry the same name without being a real type.
            let dn = self.data.def_nr(&sub_name);
            if dn != u32::MAX {
                let dt = self.data.def_type(dn);
                if !matches!(
                    dt,
                    DefType::Struct
                        | DefType::Enum
                        | DefType::EnumValue
                        | DefType::Type
                        | DefType::Unknown
                ) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is a {:?}, not a type — the element of {}<T> must \
                         be a struct or enum (defined at {})",
                        sub_name,
                        dt,
                        type_name,
                        self.data.def(dn).position()
                    );
                    // Consume the rest of the <...> so the parser stays
                    // synchronised on the next token.
                    self.lexer.recover_to(&[">", ";", "}"]);
                    self.lexer.has_closing_angle();
                    return Some(Type::Unknown(0));
                }
            }
            if let Some(tp) = self.parse_type(on_d, &sub_name, false) {
                let sub_nr = if let Type::Unknown(d) = tp {
                    d
                } else {
                    self.data.type_def_nr(&tp)
                };
                let mut fields = Vec::new();
                return Some(match type_name {
                    "index" => {
                        // @PLN25 — keyed collections (`index`/`hash`/`sorted`) are DENSE:
                        // nullability is a SEQUENCE concept (`vector`/`array`) only, so a keyed
                        // element is implicitly `not null` (a key denotes presence).  Accept an
                        // explicit `not null` in the definition as a no-op.  (Design:
                        // single-payload-refactor.md § "DESIGN DECISION (2026-06-20)".)
                        self.has_deprecated_not_null();
                        self.parse_fields(true, &mut fields);
                        Type::Index(
                            self.data.type_def_nr(&tp),
                            fields,
                            crate::data::Deps::none(),
                        )
                    }
                    "hash" => {
                        // Dense (implicitly `not null`) — see the `index` arm.
                        self.has_deprecated_not_null();
                        self.parse_fields(false, &mut fields);
                        self.data.set_referenced(sub_nr, on_d, Value::Null);
                        let mut f = Vec::new();
                        for (field, _) in fields {
                            f.push(field);
                        }
                        Type::Hash(sub_nr, f, crate::data::Deps::none())
                    }
                    "vector" => {
                        // @PLN25 E2/E3 — the ONE chokepoint where every inline
                        // `vector<S>` element type resolves (local / param / return /
                        // field / nested all reach here).  Default = nullable: rewrite
                        // a struct element `Reference(S)` to the synthetic
                        // `__nullable<S>` enum.  A `not null` after a NAMED element
                        // (`vector<Row not null>`) is the dense opt-out — consume it
                        // and skip the rewrite, leaving the bare inline struct.
                        // (Scalar elements consume their own `not null` in the type
                        // parse above, so this fires only for a named element.)
                        // `not null` still accepted (now redundant — dense IS the
                        // default) as a no-op, for back-compat with existing source.
                        self.has_deprecated_not_null();
                        self.lexer.closing_angle();
                        // loft#923 — a KEYED collection is not a vector element.
                        //
                        // ⚠ This is the chokepoint every WRITTEN `vector<…>` element passes
                        // through, and that is not every vector: an INFERRED literal
                        // (`hs = [mk(1), mk(2)]`) never writes the type, so it reached no
                        // check here and panicked the interpreter instead — the copy landed
                        // on `u16::MAX`, which the source-free bit masks to `0x7FFF`, an
                        // index into an 85-row table (loft#1298).  `parse_vector` asks
                        // `refuse_keyed_vector_element` for the same refusal.
                        //
                        // Nothing could ever fill one. A literal element types as the
                        // CONTENT struct ("cannot store vector<E> elements in a
                        // vector<hash<E,[\"k\"]>>"), and appending a keyed LOCAL walked
                        // off the type table. `--native` did not even get that far: the
                        // element type was never created, so the generated `init()`
                        // named a binding no line made and rustc refused the program.
                        // So it could be declared, and only declared.
                        //
                        // Refused where it is written rather than given storage, the
                        // same call DESIGN_DECISIONS C113 makes for two `index`
                        // members over one key — and the cure named below is a real
                        // one that costs nothing the element would not have cost.
                        //
                        // Named by its KIND, not by `Type::name`: a keyed type's
                        // registered name carries its key list in the schema's own
                        // spelling (`sorted<E,[("k", true)]>`), which is not what the
                        // author wrote and not something to hand back to them.
                        if self.refuse_keyed_vector_element(&tp) {
                            return Some(Type::Unknown(0));
                        }
                        // @PLN25 storage-vs-access-nullability: DENSE by default; the
                        // leading `?` (`vector<?S>`) opts in to a nullable element,
                        // synthesising the `__nullable<S>` enum.
                        let elem = if nullable_elem {
                            self.e2_nullable_elem(tp)
                        } else {
                            tp
                        };
                        Type::Vector(Box::new(elem), crate::data::Deps::none())
                    }
                    "sorted" => {
                        // Dense (implicitly `not null`) — see the `index` arm.
                        self.has_deprecated_not_null();
                        self.parse_fields(true, &mut fields);
                        Type::Sorted(sub_nr, fields, crate::data::Deps::none())
                    }
                    "trie" => {
                        // `trie<T[w]>` — a radix tree over ONE text key, answering
                        // exact lookup, key order and PREFIX.  Its own keyword rather
                        // than a `spatial` spelling because `spatial` means Morton
                        // interleaving of coordinate axes, and none of that applies to
                        // a word.  See doc/claude/plans/text-keyed-trie.md.
                        self.has_deprecated_not_null();
                        if self.lexer.peek_token("[") {
                            self.parse_fields(false, &mut fields);
                            self.data.set_referenced(sub_nr, on_d, Value::Null);
                            let f: Vec<String> = fields.into_iter().map(|(k, _)| k).collect();
                            if f.len() == 1 {
                                self.check_key_is_text(sub_nr, &f[0], true);
                                Type::Trie(sub_nr, f[0].clone(), crate::data::Deps::none())
                            } else {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "trie<T[k]> keys on exactly ONE text field, got {} — a trie \
                                     orders one key's bytes, so several keys have no order to \
                                     share; use `sorted<T[a, b]>` for a multi-field order",
                                    f.len()
                                );
                                Type::Unknown(0)
                            }
                        } else {
                            self.lexer.closing_angle();
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "trie<T[k]> needs its text key field, e.g. trie<Word[w]>"
                            );
                            Type::Unknown(0)
                        }
                    }
                    "spatial" => {
                        // @PLN48 S2 — `spatial<T[x, y]>` lowers to the shared `Radix`
                        // runtime kind (RADIX_TREE.md §8.1): the coordinate key fields
                        // become the Morton-interleaved axes.  Mirrors the `hash` arm;
                        // a spatial index needs its coordinate keys, so a bare
                        // `spatial<T>` (no key-spec) is a helpful error rather than a
                        // silent empty key.
                        self.has_deprecated_not_null();
                        if self.lexer.peek_token("[") {
                            self.parse_fields(false, &mut fields);
                            self.data.set_referenced(sub_nr, on_d, Value::Null);
                            let mut f = Vec::new();
                            for (field, _) in fields {
                                f.push(field);
                            }
                            // The Morton code interleaves at most `MAX_AXES` axes; a wider
                            // key would index past the `[u64; MAX_AXES]` array at runtime
                            // (a production panic).  Reject it here with a clean message.
                            if f.len() > crate::radix_db::MAX_AXES {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "spatial<T[…]> supports at most {} coordinate axes, got {}",
                                    crate::radix_db::MAX_AXES,
                                    f.len()
                                );
                            }
                            for k in &f {
                                self.check_key_is_text(sub_nr, k, false);
                            }
                            Type::Radix(sub_nr, f, crate::data::Deps::none())
                        } else {
                            self.lexer.closing_angle();
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "spatial<T[x, y]> needs coordinate key fields, e.g. spatial<Mob[x, y]>"
                            );
                            Type::Unknown(0)
                        }
                    }
                    "reference" => {
                        self.lexer.closing_angle();
                        self.data.set_referenced(sub_nr, on_d, Value::Null);
                        // #328: in struct-field position `reference<T>` is the
                        // documented POINTER — the `u16::MAX` dep is the
                        // auto-Reference share marker that selects the 12-byte
                        // `Parts::DbRef` layout in `fill_database` and the
                        // OpGetDbRef / OpSetDbRef read/write arms.  Without it
                        // the parse erased the pointer-ness: the field laid
                        // out as INLINE `T` bytes (a silent deep copy on
                        // write), `next: null` construction panicked on the
                        // unpositioned-field marker, and `reference<Self>`
                        // could not exist at all.  Non-field positions
                        // (locals, parameters, return types) keep the plain
                        // shape — their semantics are unchanged by #328.
                        if on_d != u32::MAX
                            && matches!(
                                self.data.def_type(on_d),
                                DefType::Struct | DefType::EnumValue
                            )
                        {
                            Type::Reference(sub_nr, crate::data::Deps::pointer_marker())
                        } else {
                            Type::Reference(sub_nr, crate::data::Deps::none())
                        }
                    }
                    "iterator" => {
                        // CO1.3c: comma and second type are optional for generators.
                        // iterator<T> = generator yield type; iterator<T, I> = collection iterator.
                        let mut it_tp = Type::Null;
                        if self.lexer.has_token(",") {
                            if let Some(iter) = self.lexer.has_identifier() {
                                if let Some(it) = self.parse_type(on_d, &iter, false) {
                                    self.data.set_referenced(sub_nr, on_d, Value::Null);
                                    it_tp = it;
                                } else {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "Expect an iterator type"
                                    );
                                }
                            } else {
                                diagnostic!(self.lexer, Level::Error, "Expect an iterator type");
                            }
                        }
                        self.lexer.closing_angle();
                        Type::Iterator(Box::new(tp), Box::new(it_tp))
                    }
                    _ => {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Subtype only allowed on structures"
                        );
                        Type::Unknown(0)
                    }
                });
            }
            assert!(self.first_pass, "Incorrect handling of unknown types");
        } else {
            self.lexer.revert(link);
        }
        None
    }

    // <depend> ::= '[' { <field> [ ',' ] } ']'
    pub(crate) fn parse_depended(&mut self, returned: bool, dep: &mut Vec<u16>) {
        if self.default && returned && self.lexer.has_token("[") && self.context != u32::MAX {
            loop {
                if let Some(id) = self.lexer.has_identifier() {
                    if let Some(nr) = self.data.def(self.context).attr_names.get(&id) {
                        dep.push(*nr as u16);
                    } else {
                        diagnostic!(self.lexer, Level::Error, "Unknown field name '{id}'");
                    }
                } else {
                    diagnostic!(self.lexer, Level::Error, "Expected a field name");
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("]");
        }
    }

    pub(crate) fn parse_fields(&mut self, directions: bool, result: &mut Vec<(String, bool)>) {
        self.lexer.token("[");
        loop {
            let desc = self.lexer.has_token("-");
            if !directions && desc {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Structure doesn't support descending fields"
                );
            }
            if let Some(field) = self.lexer.has_identifier() {
                result.push((field, !desc));
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("]");
        self.lexer.closing_angle();
    }

    // <field_limit> ::= 'limit' '(' [ '-' ] <min-integer> ',' [ '-' ] <max-integer> ')'
    ///
    /// loft#1037 — a bound the spec cannot CARRY is refused here, not truncated.
    /// `IntegerSpec` holds `min: i32` / `max: u32`, so `limit(0, 5000000000)` used to
    /// store `5000000000 as u32` = `705032704`: the declaration silently became a
    /// range narrower than the one written, and the range guard then mapped every
    /// in-range value to the slot's default — `4999999995` read back as `0`, an
    /// ordinary value of the type, with nothing to distinguish it from a genuine
    /// out-of-range write.  A bound is the one thing in a declaration a program cannot
    /// check for itself, so an unrepresentable one is a compile error naming the
    /// representable edge and the type that does hold the value.
    pub(crate) fn parse_type_limit(&mut self, min: &mut i32, max: &mut u32) -> bool {
        if self.lexer.has_keyword("limit") {
            self.lexer.token("(");
            let min_neg = self.lexer.has_token("-");
            if let Some(nr) = self.lexer.has_integer() {
                *min = if min_neg { -(nr as i32) } else { nr as i32 };
            } else if let Some(nr) = self.lexer.has_long() {
                // Beyond `i32`, so it tokenised as a Long and never reached the
                // branch above — which left `min` at its default and desynced the
                // parser into *"Expect token ,"*, an error about punctuation for a
                // bound that is simply too wide.  `i32::MIN` itself is reserved as the
                // null sentinel, so the lowest bound a declaration may name is
                // `i32::MIN + 1`.
                // Pass 2 only: a pass-1 diagnostic aborts before the second pass runs, so
                // reporting there would hide every other error in the file behind this
                // one.  The bound is read (and truncated) on both passes either way, and
                // pass 2 is where the program stops.
                let sign = if min_neg { "-" } else { "" };
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "lower bound {sign}{nr} is outside the range `limit(...)` can carry \
                     ({} to {}); declare it plain `integer`, which holds the full 64-bit \
                     range, and check the bound in code",
                        i32::MIN + 1,
                        i32::MAX
                    );
                }
            }
            self.lexer.token(",");
            // An upper bound below zero is not representable: `IntegerSpec::max` is a
            // `u32`, so a range lying entirely below zero has no encoding.  Say that,
            // because the `-` otherwise reaches no branch below and the parser desyncs
            // into *"Expect token )"* — an error about punctuation for a bound the type
            // system simply cannot carry, which is the same shape the lower bound's
            // too-wide case was fixed for above.
            if self.lexer.has_token("-") {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`limit(...)`'s upper bound cannot be negative, so a range lying \
                         entirely below zero cannot be declared; widen it to zero \
                         (`limit({min}, 0)`) and check the upper edge in code, or declare \
                         it plain `integer`"
                    );
                }
                // Consume the digits so the `)` below still lines up and the file's other
                // errors are reported rather than buried under a cascade.
                let _ = self
                    .lexer
                    .has_integer()
                    .or_else(|| self.lexer.has_long().and_then(|n| u32::try_from(n).ok()));
                self.lexer.token(")");
                return true;
            }
            // C54.A incremental 2a — accept both Integer and Long literals.
            // Values > i32::MAX tokenise as Long, so u32-range bounds like
            // `limit(0, 4_294_967_294)` work.
            if let Some(nr) = self.lexer.has_integer() {
                *max = nr;
            } else if let Some(nr) = self.lexer.has_long() {
                if let Ok(fits) = u32::try_from(nr) {
                    *max = fits;
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "upper bound {nr} is outside the range `limit(...)` can carry \
                         (up to {}); declare it plain `integer`, which holds the full \
                         64-bit range, and check the bound in code",
                        u32::MAX
                    );
                }
            }
            self.lexer.token(")");
            true
        } else {
            false
        }
    }

    // <struct> = 'struct' <identifier> [ ':' <type> ] '{' <param-id> ':' <field> { ',' <param-id> ':' <field> } '}'
    /// @PLN86 P6.1 — a `capability <dotted.name>` top-level declaration.  Registers
    /// a namespaced capability group that functions + data members link to via the
    /// `group#right` notation (P6.2 / P6.4); the dotted name IS the namespace (matched
    /// hierarchically on the grant side, like the existing `fs.read` groups).  A
    /// capability has no code or type — it is a pure annotation target — so this only
    /// records the name; a `group#right` link to an UNDECLARED group is a load error,
    /// validated at admission once every declaration is registered.  `capability` is a
    /// contextual keyword (not reserved), so it never shadows an identifier.
    pub(crate) fn parse_capability(&mut self) -> bool {
        if !self.lexer.has_keyword("capability") {
            return false;
        }
        let Some(mut name) = self.lexer.has_identifier() else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect a capability name after `capability`"
            );
            return true;
        };
        // A dotted name (`cmd.move`) is the namespace, like the `fs.read` groups.
        while self.lexer.has_token(".") {
            let Some(seg) = self.lexer.has_identifier() else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect an identifier after `.` in a capability name"
                );
                return true;
            };
            name.push('.');
            name.push_str(&seg);
        }
        self.declared_capabilities.insert(name);
        true
    }

    /// @PLN86 P6.2 — try to parse a `group#right` capability-link token in a position
    /// where one is OPTIONAL: a dotted group, `#`, then one of `read`/`update`/`append`.
    /// Returns the canonical `"group#right"` string; returns `None` **silently** when
    /// no link is present (the next token is not an identifier — e.g. the `;`/`{`
    /// terminator after a signature, or a field separator); errors only on a *malformed*
    /// link (a group with no `#right`).  The group's existence as a declared `capability`
    /// is validated at admission, so forward + cross-file declarations resolve.  Shared
    /// by the function call gate (P6.2, in the signature) and a struct-field link (P6.4).
    pub(crate) fn try_cap_link(&mut self) -> Option<String> {
        // NON-DESTRUCTIVE: in this optional position the next token might instead be
        // a separator, a default `=`, or a field-modifier keyword (`not`, `assert`)
        // — all of which lex as ordinary tokens/identifiers.  So consume nothing
        // unless this is really a link: save the cursor and revert on a miss.  Only
        // a real `#` followed by a bad right is a hard error.
        let saved = self.lexer.link();
        let Some(mut group) = self.lexer.has_identifier() else {
            return None; // no identifier consumed
        };
        while self.lexer.has_token(".") {
            let Some(seg) = self.lexer.has_identifier() else {
                self.lexer.revert(saved);
                return None;
            };
            group.push('.');
            group.push_str(&seg);
        }
        if !self.lexer.has_token("#") {
            // not a link (e.g. `not null`, `assert(...)`, the next field) — un-consume.
            self.lexer.revert(saved);
            return None;
        }
        let Some(right) = self.lexer.has_identifier() else {
            self.lexer.revert(saved);
            return None;
        };
        if crate::sandbox::Right::parse(&right).is_none() {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown capability right `{right}` — expected read, update, or append"
            );
            return None;
        }
        Some(format!("{group}#{right}"))
    }

    /// @PLN86 P6.4 — consume zero-or-more `group#right` capability links after a
    /// struct field's type (`health: int stats#read stats#update`), recording each
    /// on `(struct, field)` for the admission walk.  Consumed every pass, recorded
    /// once (first pass) so the per-field link list does not double on re-parse.
    pub(crate) fn parse_field_links(&mut self, d_nr: u32, a_name: &str) {
        while let Some(token) = self.try_cap_link() {
            if self.first_pass {
                self.record_member_link(d_nr, a_name, token);
            }
        }
    }

    /// Refuse a SECOND `index` field over the same element type and the same key
    /// in one structure (loft#902).
    ///
    /// Every other collection kind keeps its own storage, so two of them over one
    /// element type are two routes to a shared record set — the linked-collection
    /// group loft#843 built. An `index` cannot be: a red-black tree keeps its
    /// links in FIELDS OF THE ELEMENT RECORD, and the field triple is allocated
    /// per index TYPE (`#left_N / #right_N / #color_N`). Two fields whose declared
    /// type is identical resolve to that one type, so they name one set of links —
    /// not two trees, but ONE tree reached through two roots.
    ///
    /// That is why the FILL looked right and only removal fell over: both roots
    /// walked the same structure, so lengths and iteration agreed, and the first
    /// removal rebalanced through one root and left the other stale — a panic in
    /// `tree.rs` on the next walk, which "no runtime errors, ever" does not allow.
    ///
    /// It is refused rather than made to work because there is nothing to make
    /// work: a second index with the SAME key answers exactly what the first
    /// answers, in the same order. A different key is a different type with its
    /// own link triple, so `index<E[k]> + index<E[n]>` is untouched and correct.
    fn reject_duplicate_index(&mut self, d_nr: u32, a_name: &str, a_type: &Type) {
        // @FR-Col-Group — membership of a record set is not about whether a MEMBER is nullable,
        // and two `index` members over one element type are refused because an index keeps its
        // tree links in a field OF the record (DESIGN_DECISIONS.md, the two-index ruling).
        //
        // @FR-L-Null — `base()` on both sides: a `?` on a field is a compile-time bit over the
        // same storage, so `index<E[k]>?` is the same tree over the same records as
        // `index<E[k]>`.  Asked bare, a nullable spelling escaped this refusal on either side
        // and the two indexes then overwrote each other's links in the one field the record
        // has for them — a lookup answering null for a record its own `for` loop yields.
        let Type::Index(elem, keys, _) = a_type.base() else {
            return;
        };
        for a_nr in 0..self.data.attributes(d_nr) {
            let Type::Index(other_elem, other_keys, _) = self.data.attr_type(d_nr, a_nr).base().clone()
            else {
                continue;
            };
            if other_elem != *elem || other_keys != *keys {
                continue;
            }
            let earlier = self.data.attr_name(d_nr, a_nr);
            // The same NAME is the same field declared twice — a re-derivation
            // (generic instantiation, a stub refilled), not two indexes.
            if earlier == a_name {
                continue;
            }
            // Spelled the way the user wrote it — `Type::show` renders an index as
            // its debug pair (`index<E,[("k", true)]>`), which nothing reading the
            // error would recognise as their own declaration.
            let spelled = keys
                .iter()
                .map(|(k, asc)| if *asc { k.clone() } else { format!("-{k}") })
                .collect::<Vec<_>>()
                .join(",");
            let shown = format!("index<{}[{spelled}]>", self.data.def(*elem).name);
            diagnostic!(
                self.lexer,
                Level::Error,
                "'{a_name}' and '{earlier}' are both '{shown}' in the same structure — two \
                 indexes cannot share records, because an index keeps its tree links in a \
                 field of the record and one field of links cannot hold two trees. Give the \
                 second route a different kind ('hash' or 'sorted' over the same records), \
                 or index a different key"
            );
            return;
        }
    }

    // @F12 — struct records (fields, `= default`, `computed`, `limit`/`not null`/`assert`)
    pub(crate) fn parse_struct(&mut self) -> bool {
        // @PLN101 — optional `value` modifier: `value struct T {…}` marks T a value (copy,
        // inline, non-null) type. `value` is a plain IDENTIFIER (not a keyword), so peek it
        // (`has_token` only matches Token lexemes) and consume only the `value struct` prefix.
        let is_value = matches!(
            self.lexer.peek().has,
            crate::lexer::LexItem::Identifier(ref t) if t == "value"
        );
        if is_value {
            self.lexer.cont();
        }
        if !self.lexer.has_token("struct") {
            if is_value {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`value` must be followed by `struct`"
                );
                return true;
            }
            return false;
        }
        let Some(id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect attribute");
            return true;
        };
        let mut d_nr = self.data.def_nr(&id);
        // @PLN22 Phase 2 — shadow a prelude/import struct of the same key.  This
        // includes the stdlib's generic type-var marker (`<T>`): a user `struct T`
        // shadows it just like the enum path does, so `T` is a usable type name.
        // (A SAME-SOURCE clash — the user's own `struct T` plus their own `fn foo<T>`
        // — keeps `prelude_shadowed` false, so it still hits the "reserved" arm below.)
        if self.prelude_shadowed(&id) {
            d_nr = u32::MAX;
        }
        if d_nr == u32::MAX {
            d_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Struct);
            self.data.definitions[d_nr as usize].returned =
                Type::Reference(d_nr, crate::data::Deps::none());
        } else if self.first_pass {
            // fix-tvscope: a SAME-SOURCE type-var placeholder (the user's own
            // `fn foo<T>` before their `struct T`) blocks the struct — a genuine
            // clash worth the dedicated diagnostic rather than the confusing
            // "Redefined struct".  (A cross-source stdlib `<T>` was shadowed above.)
            if self.data.is_type_var_placeholder(d_nr) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}' is reserved as a generic type variable — choose a different struct name",
                    id
                );
            } else if self.data.def_type(d_nr) == DefType::Unknown
                && matches!(
                    self.data.definitions[d_nr as usize].returned,
                    Type::Unknown(_)
                )
            {
                // Adopt a genuine same-file forward-reference placeholder (an
                // `add_def(.., DefType::Unknown)` stub left by a use-before-def).
                // A reserved builtin type-keyword forward-declared in the stdlib
                // (e.g. `type iterator;`) is `DefType::Type` with an Unknown
                // returned type — it must NOT be adopted, or `struct iterator`
                // would silently shadow the builtin.  Without the def_type guard
                // it fell through here; now it lands in the conflict arm below.
                self.data.definitions[d_nr as usize].position = self.lexer.pos().clone();
                self.data.definitions[d_nr as usize].def_type = DefType::Struct;
                self.data.definitions[d_nr as usize].returned =
                    Type::Reference(d_nr, crate::data::Deps::none());
                self.data.note_stub_adopted(d_nr);
            } else {
                let prev_pos = self.data.def(d_nr).position().clone();
                let prev_kind = format!("{:?}", self.data.def(d_nr).def_type()).to_lowercase();
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "struct '{id}' conflicts with a {prev_kind} of the same name \
                     already defined at {prev_pos} — pick a different name"
                );
            }
        }
        let context = self.context;
        self.context = d_nr;
        // @PLN101 — mark the value-struct kind now that d_nr is the confirmed struct def.
        if is_value {
            self.data.value_structs.insert(d_nr);
        }
        self.lexer.token("{");
        // #91: collect init field dependency info for circular detection, each with the
        // position of the field NAME.  The check can only run once every field is known,
        // by which time the construct — and `report_pos` with it — is at the closing `}`;
        // a struct has many fields and "somewhere in this struct" is not an answer.
        let mut init_deps: Vec<(String, Vec<String>, crate::lexer::Position)> = Vec::new();
        // Each field's NAME with the position it starts at — what a diagnostic about the
        // SHAPE of the declaration has to point at.
        let mut field_at: Vec<(String, crate::lexer::Position)> = Vec::new();
        loop {
            self.lexer.has_token("pub");
            // @PLN40 — a `const` field is write-once at construction.  Consume the
            // keyword (if present) and mark the field once it has parsed; see
            // doc/claude/plans/40-const-fields/.
            let is_const = self.lexer.has_keyword("const");
            // The field name is the current token, so this is its START.
            let field_pos = self.lexer.peek_pos().clone();
            let Some(a_name) = self.lexer.has_identifier() else {
                diagnostic!(self.lexer, Level::Error, "Expect attribute");
                self.context = context;
                return true;
            };
            if self.first_pass && self.data.attr(d_nr, &a_name) != usize::MAX {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "field `{}` is already declared",
                    a_name
                );
            }
            self.lexer.token(":");
            self.init_field_deps.clear();
            field_at.push((a_name.clone(), field_pos.clone()));
            self.parse_field(d_nr, &a_name);
            if is_const {
                self.mark_const_field(d_nr, &a_name);
            }
            if !self.init_field_deps.is_empty() {
                init_deps.push((
                    a_name.clone(),
                    self.init_field_deps.clone(),
                    field_pos.clone(),
                ));
            }
            if !self.lexer.has_token(",") || self.lexer.peek_token("}") {
                break;
            }
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        self.link_shared_nullable_views(d_nr);
        self.advise_group_apart(d_nr, &field_at);
        // #91: check for circular init dependencies (second pass, all fields known).
        if !self.first_pass {
            self.check_circular_init(&init_deps);
        }
        self.context = context;
        true
    }

    /// @PLN25 Scope B — a keyed field that shares its record set with a sibling NULLABLE
    /// vector (the `other_indexes` "two views, one record set" pattern, e.g.
    /// `struct Db { entries: vector<S?>, lookup: hash<S[k]> }`) indexes the `Some`-wrapped
    /// records, not dense `S`.  Rewrite such a view's element from `S` to the sibling's
    /// `__nullable<S>` enum so the parser type, db storage, lookup type-id, and field-access all
    /// agree on ONE type: the db link then matches by content, `determine_keys` bakes the key at
    /// the payload offset, and `c.lookup[k].field` unwraps via the `Some` payload sub-ref — all
    /// reusing the kept nullable machinery.
    ///
    /// **Every keyed kind is a view on the same terms** (@FR-Col-Group) — `hash`, `sorted`,
    /// `index`, `spatial` and `trie` are the set `Stores::field` groups, so the rewrite covers
    /// exactly that set.
    /// Naming a subset does not refuse the pairing: the view's element type simply stays `S`
    /// while the vector's is `__nullable<S>`, the two no longer match by content, and the
    /// declaration silently builds a SECOND, independent collection that every insert through
    /// the vector misses (loft#927, one axis over).
    ///
    /// **The `?` a member carries on ITSELF is not part of the question.**  `Optional(τ)` is
    /// τ's slot plus a compile-time bit (@FR-L-Null), so `hash<S[k]>?` and `vector<S?>?` name
    /// the same element type their dense spellings do and belong to the same group.  Both
    /// halves below peel before they ask, and the rewrite restores the wrapper, so a member
    /// keeps the nullability its author wrote and still joins.
    ///
    /// A DENSE vector sibling's element is `Reference(S)`, not the `Enum`, so nothing matches
    /// and the rewrite is a no-op — which is also what makes it inert with the `LOFT_E2_SYNTH`
    /// gate off.  The trigger is the `?` the author wrote (`vector<S?>`), gate or no gate.
    /// Refuse a linked group whose members disagree about NULLABILITY — a dense `vector<S>`
    /// beside a `vector<S?>`, with a keyed member to group them.
    ///
    /// Two rules meet here and cannot both hold.  `(Col-Group)` says membership is "not about
    /// whether the element is dense (`vector<E>`) or nullable (`vector<E?>`)", so all three
    /// are ROUTES to one record set.  `(N-Dense)` says a `vector<E>` stores `E` and its
    /// elements are non-null unless the author wrote `vector<E?>`.  One record set that may
    /// hold absence cannot be read through a non-null element type: the records are not even
    /// the same shape, since a nullable element is the tagged `__nullable<E>` (a discriminant
    /// plus the payload) and a dense one is `E` itself.
    ///
    /// Measured both ways before choosing the refusal.  Left as it was, the dense member
    /// silently falls out of its own group — a write through it reaches nothing else, and
    /// `len` of the member that never received the record is a legal `0` (loft#1385).  Made to
    /// join by comparing the element through the nullable peel, it receives the record and
    /// MISREADS it: `a[0].n` answered `7` and `a[0].k` answered `2`, the `Some` discriminant,
    /// which is loft#1134's misread — a zero turned into garbage, which is worse.
    ///
    /// So the declaration has no coherent meaning and is declined at the point the group would
    /// form, with a message naming the cure.  That is the direction `D-bind-17` took for `&τ?`
    /// and the direction the freeze axis wants: a refusal is loud, and both alternatives here
    /// are silent.  Rewriting the DENSE member to nullable is not open — it would give the
    /// author an element type they did not write and contradict `(N-Dense)`.
    ///
    /// Only fires where a group would actually form: two vectors with NO keyed member over
    /// that element type are independent (`(Col-Group)`'s last sentence), so a dense and a
    /// nullable vector alone keep compiling.
    fn refuse_mixed_nullability_group(
        &mut self,
        d_nr: u32,
        n: usize,
        nullable_of: &std::collections::HashMap<u32, u32>,
    ) {
        for a in 0..n {
            let declared = match self.data.attr_type(d_nr, a) {
                Type::Optional(inner) => *inner,
                other => other,
            };
            // A DENSE vector over a struct that also has a NULLABLE sibling in this struct.
            let Type::Vector(inner, _) = declared else {
                continue;
            };
            let Type::Reference(s, _) = *inner else {
                continue;
            };
            if !nullable_of.contains_key(&s) {
                continue;
            }
            // …and a KEYED member over the same struct, which is what makes them one group.
            let keyed = (0..n).any(|k| {
                let kt = match self.data.attr_type(d_nr, k) {
                    Type::Optional(i) => *i,
                    other => other,
                };
                matches!(kt,
                    Type::Hash(el, _, _)
                        | Type::Sorted(el, _, _)
                        | Type::Index(el, _, _)
                        | Type::Radix(el, _, _)
                        | Type::Trie(el, _, _) if el == s)
            });
            if !keyed {
                continue;
            }
            let dense = self.data.def(d_nr).attributes[a].name.clone();
            let elem = self.data.def(s).original_name().clone();
            diagnostic!(
                self.lexer,
                Level::Error,
                "`{dense}` holds `{elem}` while another member of its record set holds `{elem}?` — one set cannot be read both ways, because a nullable element is stored behind a tag and a dense one is not. Write `{dense}: vector<{elem}?>` to join the group, or drop the keyed member that groups them"
            );
            return;
        }
    }

    fn link_shared_nullable_views(&mut self, d_nr: u32) {
        let n = self.data.definitions[d_nr as usize].attributes.len();
        // payload struct `S` -> its `__nullable<S>` enum, gathered from nullable vector siblings.
        let mut nullable_of: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        for a in 0..n {
            // A nullable FIELD still has the element type the pairing is about: `?` is a
            // compile-time bit over the same slot (@FR-L-Null), so `vector<S?>?` carries the
            // `__nullable<S>` this gather looks for exactly as `vector<S?>` does.
            let declared = match self.data.attr_type(d_nr, a) {
                Type::Optional(inner) => *inner,
                other => other,
            };
            if let Type::Vector(inner, _) = declared
                && let Type::Enum(nd, true, _) = *inner
                // ONLY the synth `__nullable<S>` enum — not an arbitrary user struct-enum
                // element (`vector<Shape>`), whose `variant_of(.., "Some")` would be MAX.
                && self.data.def(nd).name().starts_with("__nullable<")
            {
                let some = self.data.variant_of(nd, "Some");
                let pa = self.data.attr(some, "payload");
                if pa != usize::MAX
                    && let Type::Reference(s, _) = self.data.attr_type(some, pa)
                {
                    nullable_of.insert(s, nd);
                }
            }
        }
        if nullable_of.is_empty() {
            return;
        }
        self.refuse_mixed_nullability_group(d_nr, n, &nullable_of);
        for a in 0..n {
            // Which record set a view indexes is a question about its ELEMENT, so it is asked
            // of the peeled type and the wrapper is restored below: a member declared
            // `hash<S[k]>?` is a collection over `S` in this struct and is therefore a member
            // of the group (@FR-Col-Group), and it keeps the nullability its author wrote.
            let (declared, nullable_field) = match self.data.attr_type(d_nr, a) {
                Type::Optional(inner) => (*inner, true),
                other => (other, false),
            };
            let rewritten = match declared {
                Type::Hash(el, keys, deps) => {
                    nullable_of.get(&el).map(|&nd| Type::Hash(nd, keys, deps))
                }
                Type::Sorted(el, keys, deps) => {
                    nullable_of.get(&el).map(|&nd| Type::Sorted(nd, keys, deps))
                }
                Type::Index(el, keys, deps) => {
                    nullable_of.get(&el).map(|&nd| Type::Index(nd, keys, deps))
                }
                Type::Radix(el, keys, deps) => {
                    nullable_of.get(&el).map(|&nd| Type::Radix(nd, keys, deps))
                }
                Type::Trie(el, key, deps) => {
                    nullable_of.get(&el).map(|&nd| Type::Trie(nd, key, deps))
                }
                _ => None,
            };
            if let Some(t) = rewritten {
                self.data.definitions[d_nr as usize].attributes[a].typedef =
                    if nullable_field { Type::optional(t) } else { t };
            }
        }
    }

    /// Advise when a linked collection group's members are declared APART — @FR-Col-Group
    /// made legible at the one place it is decidable.
    ///
    /// Two collections over one element type in one struct are one record set, and the
    /// declaration is the only place that is decidable — by the time a `len` reads 0 the
    /// question looks like an empty collection instead. Nothing else in the source says
    /// which of the two you have.
    ///
    /// Fires only when an unrelated field sits BETWEEN two members, because that is the
    /// shape whose author was probably not thinking of them as a pair; see
    /// [`crate::keys::group_apart_lint_enabled`] for why adjacency is the signal and why
    /// this can only ever be advice.
    ///
    /// `field_at` pairs each DECLARED field's name with its position. Resolved by NAME, not
    /// by index: a struct-enum variant carries an implicit `enum` discriminator field that the
    /// source never wrote, so its attribute indices and its written fields do not line up.
    /// The line points at the member that JOINED — the later one, which is the field the
    /// author most likely added without knowing what it would join.
    fn advise_group_apart(&mut self, d_nr: u32, field_at: &[(String, crate::lexer::Position)]) {
        // No ownership test here: `Diagnostics::reaches_author` is the one home for who a
        // lint is addressed to (loft#1260).  This site used to ask `source_is_owned`, which
        // is `source == MAIN_SOURCE` — the ENTRY file, not the project — so the lint was
        // silent in a package's own `loft test`, where the entry is `tests/*.loft` and the
        // struct under review is in `src/*.loft`.  That is the one run it exists for.
        if self.default || self.first_pass || !crate::keys::group_apart_lint_enabled() {
            return;
        }
        for (_, members) in self.collection_groups(d_nr) {
            let (Some(first), Some(last)) = (members.first(), members.last()) else {
                continue;
            };
            // Contiguous in declaration order — the idiom, and quiet.
            if last.a_nr - first.a_nr + 1 == members.len() {
                continue;
            }
            let Some((_, at)) = field_at.iter().find(|(nm, _)| *nm == last.name) else {
                continue;
            };
            let earlier = members[..members.len() - 1]
                .iter()
                .map(|m| format!("`{}`", m.name))
                .collect::<Vec<_>>()
                .join(", ");
            let is_are = if members.len() > 2 { "are" } else { "is" };
            diagnostic_at!(
                self.lexer,
                at,
                Level::Advice,
                code = "linked-group-apart",
                "`{}` shares one record set with {earlier}, which {is_are} declared further \
                 up — two collections over one element type in one struct are two routes to \
                 the same records, so filling either fills both",
                last.name
            );
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!(
                    "give `{}` its own element type so the two stay independent",
                    last.name
                ),
                condition: Some("they were meant to be separate collections".to_string()),
                edit: None,
                concept: "keyed collections",
                concept_ref: "@F7",
            });
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "declare them next to each other so the pairing reads as deliberate"
                    .to_string(),
                condition: Some("they were meant as two routes to one record set".to_string()),
                edit: None,
                concept: "keyed collections",
                concept_ref: "@F7",
            });
        }
    }

    /// The key that identifies a generic header's BOUND SET — the bound names as written,
    /// ordered and de-duplicated so `<T: A + B>` and `<T: B + A>` are one set.
    ///
    /// Deliberately the spellings and not the resolved `def_nr`s: an interface may still be
    /// a forward reference on the first pass, and the key has to answer the same on both.
    fn type_var_bounds_key(bounds: &[String]) -> String {
        let mut names: Vec<&str> = bounds.iter().map(String::as_str).collect();
        names.sort_unstable();
        names.dedup();
        names.join("+")
    }

    /// I7/I8.1: build the `t_<LEN><Holder>_<method>` stubs that let a body call a bound
    /// interface's methods on a value whose type is not concrete yet.
    ///
    /// `holder_name` names the type standing in for the concrete one — a generic's type
    /// variable (`<T: Printable>`), or an interface's associated type (`type Rows: Cursor`,
    /// named `Source.Rows`).  Both are the same thing seen from two places: a name with
    /// declared bounds and no definition yet, so a method call on it has to resolve against
    /// the bounds.  The body parser emits `Value::Call(stub, …)` at the stub, and
    /// `re_resolve_call` re-points it at the concrete implementation once the monomorph
    /// knows what the holder is.
    ///
    /// A stub stands in for the method it will be replaced by, so **its ABI must equal that
    /// method's ABI** — including the hidden parameters a `text` / heap return carries.
    /// The pair `(stub, interface method)` is recorded as it is built, because on the FIRST
    /// pass the interface method's return type can still be an unresolved forward
    /// reference: [`Parser::refresh_bound_method_stubs`] re-derives the signature between
    /// the passes, where every type IS resolved.
    fn create_bound_method_stubs(&mut self, holder_nr: u32, bounds: &[u32]) {
        if holder_nr == u32::MAX || self.data.def_nr("Self") == u32::MAX {
            return;
        }
        let holder_name = self.data.def(holder_nr).name().to_string();
        let holder_name = holder_name.as_str();
        if holder_name.is_empty() {
            return;
        }
        // loft#1153 — this is the ONE place that knows a definition is a bound HOLDER, so it is
        // where the durable flag is set.  Every method key for it then takes the stub spelling,
        // on both passes, whatever its structure looks like at the moment it is asked.
        self.data.mark_bound_holder(holder_nr);
        for &iface_nr in bounds {
            let children: Vec<u32> = self.data.children_of(iface_nr).collect();
            for child_nr in children {
                // Only a METHOD becomes a callable stub. An interface's children are its
                // method stubs AND its associated-type placeholders; without this the
                // placeholder `type Rows` minted a bogus `t_1D_Source.Rows` that native
                // emitted as a `todo!()`.
                let Some(method_suffix) = Self::interface_method_name(&self.data, child_nr) else {
                    continue;
                };
                // The stub is keyed per SIGNATURE, so the arity is part of the name.  Two
                // requirements of one name at different arities are two stubs, which is what
                // lets `Numeric` declare unary negation beside binary subtraction — both
                // desugar to `OpMin` (loft#1275, `formal/interfaces.md` `D-gen-4`).
                let t_stub_name = crate::data::Data::bound_stub_name(
                    holder_name,
                    &method_suffix,
                    self.data.attributes(child_nr),
                );
                let existing_stub = self.data.def_nr(&t_stub_name);
                // Two arities of one NAMED method cannot both be reached, and this is the only
                // place that can say so.  An OPERATOR's arity is fixed by its syntax — `-a` is
                // one operand and `a - b` is two — so `call_op` asks for the exact stub and
                // both are usable, which is what loft#1275 opened up.  A method call resolves
                // its RECEIVER before its arguments are parsed, so `x.sizer()` has no arity to
                // ask with; `find_fn` answers "ambiguous" and the caller can only report a
                // field it could not resolve.  Refusing at the DECLARATION names both arities
                // and the cure instead.
                if self.first_pass
                    && existing_stub == u32::MAX
                    && !crate::parser::is_op(&method_suffix)
                    && let Some(other) = (1..=crate::data::Data::MAX_BOUND_ARITY)
                        .filter(|a| *a != self.data.attributes(child_nr))
                        .find(|a| {
                            self.data.def_nr(&crate::data::Data::bound_stub_name(
                                holder_name,
                                &method_suffix,
                                *a,
                            )) != u32::MAX
                        })
                {
                    let spelling = crate::data::Data::type_var_spelling(holder_name);
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "the bounds on '{spelling}' require two different '{method_suffix}' — \
                         one taking {} parameter(s) and one taking {} — and a method call \
                         resolves its receiver before its arguments, so only one of them can \
                         be reached. Bound '{spelling}' by the interface that declares the one \
                         this body calls, and give the other its own generic",
                        other,
                        self.data.attributes(child_nr),
                    );
                }
                if existing_stub != u32::MAX {
                    // Sharing one stub is the NORM and now the only case that reaches here:
                    // two bounds declaring the same method the SAME way, or the same header
                    // seen again on the second pass, want the same stub.  A pair that
                    // disagreed on ARITY used to land here too and had to be refused, because
                    // one name could hold only one signature; the key carries the arity now,
                    // so they are two stubs and never meet (loft#1275).
                    //
                    // Two bounds agreeing on the arity and disagreeing on the parameter TYPES
                    // still share, and still silently take the first.  That was true before
                    // this change as well — the check it replaces compared arities only — so
                    // it is an unclosed gap and not a regression.
                    continue;
                }
                let t_stub_nr =
                    self.data
                        .add_def(&t_stub_name, self.lexer.pos(), DefType::Function);
                self.bound_method_stubs
                    .push((t_stub_nr, child_nr, holder_nr));
                // Durable, unlike the vec above, which `refresh_bound_method_stubs` takes
                // between the passes — leaving nothing for the conflict check to compare.
                self.stub_origin.insert(t_stub_nr, child_nr);
                self.set_bound_stub_signature(t_stub_nr, child_nr, holder_nr);
            }
        }
    }

    /// Give a bound-method stub the signature of the interface method it stands in for,
    /// with `Self` replaced by the holder.  Split out of [`Self::create_bound_method_stubs`]
    /// so the between-passes refresh can re-derive it against resolved types.
    pub(crate) fn set_bound_stub_signature(
        &mut self,
        t_stub_nr: u32,
        child_nr: u32,
        holder_nr: u32,
    ) {
        let self_nr = self.data.def_nr("Self");
        if self_nr == u32::MAX {
            return;
        }
        let holder = crate::data::Type::Reference(holder_nr, crate::data::Deps::none());
        // `add_attribute` appends, so a re-derivation starts from an empty list.
        self.data.clear_attributes(t_stub_nr);
        let attrs_count = self.data.def(child_nr).attributes().len();
        for a_nr in 0..attrs_count {
            let a_name = self.data.attr_name(child_nr, a_nr);
            let a_type = self.data.attr_type(child_nr, a_nr);
            let new_type = Self::substitute_type(a_type, self_nr, &holder);
            self.data
                .add_attribute(&mut self.lexer, t_stub_nr, &a_name, new_type);
        }
        let ret_type = self.data.def(child_nr).returned().clone();
        let t_ret_type = Self::substitute_type(ret_type, self_nr, &holder);
        // `set_returned` asserts the slot is still unknown (it guards the signature-time
        // write), so a re-derivation writes the field directly.
        self.data.definitions[t_stub_nr as usize].returned = t_ret_type.clone();
        // I9-text: if the interface method returns text, add the hidden
        // __work_1 parameter that text_return would add for concrete
        // implementations.  Without this, the call-site argument count
        // won't match after re_resolve_call substitutes the concrete
        // text-returning method (which has the hidden param).
        //
        // loft#733 — `.base()` peels `Optional`: a `-> text?` method uses
        // the SAME buffered-text ABI as `-> text` (@PLN25 slice (c) — the
        // sentinel layout is shared, so `text_return` converts both), but
        // the bare `Type::Text(_)` match here saw only the unwrapped form.
        // The stub then carried one attribute fewer than the impl
        // `re_resolve_call` substitutes, and the result was read from a
        // slot nobody wrote: the call returned EMPTY on `--interpret` —
        // exit 0, no diagnostic — and did not compile on `--native`.
        if matches!(t_ret_type.base(), crate::data::Type::Text(_)) {
            self.data.add_attribute(
                &mut self.lexer,
                t_stub_nr,
                "__work_1",
                crate::data::Type::RefVar(Box::new(crate::data::Type::Text(
                    crate::data::Deps::none(),
                ))),
            );
        }
        // The @PLAN59 twin of the I9-text arm: a concrete method
        // returning Reference / Vector / struct-Enum carries the
        // hidden `__retbuf` attribute (the H1 heap-return ABI,
        // added at signature parse), so the stub must carry it
        // too — otherwise the template call site parses one
        // argument short of the concrete implementation that
        // `re_resolve_call` substitutes, and byte_code's arity
        // assert fires ("Too few parameters … got 2, need 3").
        // `add_defaults` fills the slot with a caller-side
        // work-ref exactly like a direct call; when T
        // instantiates to a primitive whose operator has no
        // `__retbuf` (a `#rust` op), the trailing-argument trim
        // in `substitute_type_in_value` drops it again.
        if matches!(
            t_ret_type,
            crate::data::Type::Reference(_, _)
                | crate::data::Type::Vector(_, _)
                | crate::data::Type::Enum(_, true, _)
        ) {
            let a =
                self.data
                    .add_attribute(&mut self.lexer, t_stub_nr, "__retbuf", t_ret_type.clone());
            self.data.definitions[t_stub_nr as usize].attributes[a].hidden = true;
            // Mirror ref_return's finalisation on concrete
            // implementations ("returned = {__retbuf}"): the
            // stub RETURNS its retbuf, so the returned type's
            // deps must name the retbuf attribute.  The call
            // site then binds the result as a BORROW of the
            // caller-side buffer (one free at scope end).
            // Without it the bind was owned — the buffer's
            // store was freed twice-in-name and the second
            // local vector's store never freed (#482, one
            // main_vector leaked per call).
            let dep = crate::data::Deps::attrs(vec![a as u16]);
            let dep_ret = match t_ret_type.clone() {
                crate::data::Type::Reference(d, _) => crate::data::Type::Reference(d, dep),
                crate::data::Type::Vector(e, _) => crate::data::Type::Vector(e, dep),
                crate::data::Type::Enum(d, m, _) => crate::data::Type::Enum(d, m, dep),
                other => other,
            };
            self.data.definitions[t_stub_nr as usize].returned = dep_ret;
        }
    }

    /// Parse an `interface` declaration and register it as `DefType::Interface`.
    ///
    /// Enforces @FR-G-Iface: an interface is a set of method SIGNATURES and nothing else —
    /// no bodies — each taking `self: Self`, where `Self` stands for whatever concrete type
    /// ends up satisfying it.  An operator requirement written `op <tok> (self: Self, …)`
    /// desugars here to the canonical method name (`<` ⟶ `OpLt`).
    ///
    /// Syntax: `interface Name { fn method(params) -> type [;] ... }`
    ///
    /// Signatures are parsed for syntactic correctness only, with param/return types
    /// resolved against the current scope.  Whether a type SATISFIES the interface is a
    /// separate question asked at the use site, never here — see
    /// `Parser::check_satisfaction` (@FR-G-Sat).
    #[allow(clippy::too_many_lines)]
    // @F26 — interfaces & bounded generics (<T: A + B>, operator interfaces)
    pub(crate) fn parse_interface(&mut self) -> bool {
        if !self.lexer.has_token("interface") {
            return false;
        }
        let Some(id) = self.lexer.has_identifier() else {
            diagnostic!(self.lexer, Level::Error, "Expect interface name");
            return true;
        };
        if !is_camel(&id) && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Interface name '{}' must be CamelCase",
                id
            );
        }
        // Register or locate the interface definition.
        let mut d_nr = self.data.def_nr(&id);
        if d_nr == u32::MAX {
            if self.first_pass {
                d_nr = self.data.add_def(&id, self.lexer.pos(), DefType::Interface);
            }
        } else if self.first_pass {
            diagnostic!(self.lexer, Level::Error, "Cannot redefine interface '{id}'");
        }
        // I3: register 'Self' as a type placeholder for method signature parsing.
        // 'Self' resolves to its own definition (like a generic type variable) so
        // that parse_type_full succeeds.  I6 substitutes the concrete satisfying type.
        if self.first_pass && self.data.def_nr("Self") == u32::MAX {
            let self_nr = self.data.add_def("Self", self.lexer.pos(), DefType::Struct);
            self.data
                .set_returned(self_nr, Type::Reference(self_nr, crate::data::Deps::none()));
        }
        let context = self.context;
        if d_nr != u32::MAX {
            self.context = d_nr;
        }
        if !self.lexer.token("{") {
            self.context = context;
            return true;
        }
        // Parse zero or more method/operator signatures.
        while !self.lexer.peek_token("}") {
            if self.lexer.peek().has == crate::lexer::LexItem::None {
                break;
            }
            // @PLN125 arc A — an ASSOCIATED TYPE declaration: the interface names a
            // COMPANION type rather than only a set of methods.
            //
            //   interface SqlDb {
            //     type Rows: SqlRows          // <- here
            //     fn select(self: Self, sql: text) -> Self.Rows
            //   }
            //
            // An associated type is a type variable owned by the INTERFACE. Inside a
            // generic it dispatches through its declared bounds exactly as `<T: I>`
            // does (the stubs built below), and at instantiation it binds to the one
            // concrete type the implementor's methods agree on
            // (`Parser::associated_bindings`), which must satisfy those bounds.
            //
            // `type` is a reserved token (the typedef keyword), so it arrives as
            // `Token`, not `Identifier` — `has_token`, like `parse_typedef`.
            if self.lexer.has_token("type") {
                let assoc_name = self.lexer.has_identifier();
                if assoc_name.is_none() && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Expect an associated type name after 'type' in interface body"
                    );
                }
                // The name is a TYPE name and is enforced like every other one. It is
                // also load-bearing: it becomes the `t_<LEN><Interface>.<Name>_<method>`
                // stub whose LEN prefix `re_resolve_call` parses back to find the method,
                // and an underscore in it would split that name in the wrong place.
                if let Some(name) = &assoc_name
                    && !is_camel(name)
                    && !self.first_pass
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Associated type '{name}' must be CamelCase"
                    );
                }
                // `: Bound [+ Bound]*` — the same shape a bounded generic uses.
                let mut bounds: Vec<String> = Vec::new();
                if self.lexer.has_token(":") {
                    loop {
                        match self.lexer.has_identifier() {
                            Some(b) => bounds.push(b),
                            None => {
                                if !self.first_pass {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "Expect an interface name after ':' in an associated \
                                         type bound"
                                    );
                                }
                            }
                        }
                        if !self.lexer.has_token("+") {
                            break;
                        }
                    }
                }
                // RECORD the declaration, so `Self.X` in a signature below has something to
                // resolve against, and so the monomorph has something to bind.
                //
                // Held as a CHILD definition rather than a new `Definition` field, because
                // that is what an interface's methods already are: `children_of` enumerates
                // them, and it needs no IR-store schema change to survive a cached parse.
                // `check_satisfaction` walks the same children looking for METHODS, and
                // tells them apart by `DefType` — a method stub is a `Function`, this is a
                // `Struct`.
                //
                // Named `<Interface>.<Name>`, which is both interface-scoped (two
                // interfaces may declare the same associated type) and what the author
                // wrote, so a diagnostic naming this type reads back as source.  A `.`
                // cannot occur in a user identifier, so the name cannot collide with one.
                //
                // Its `returned` points at ITSELF, exactly as `Self` does — that is what
                // makes it usable as a type before any concrete type is known.
                if let Some(name) = assoc_name
                    && d_nr != u32::MAX
                {
                    let assoc_def = format!("{}.{name}", self.data.def(d_nr).name());
                    let mut a_nr = self.data.def_nr(&assoc_def);
                    if a_nr == u32::MAX && self.first_pass {
                        a_nr = self
                            .data
                            .add_def(&assoc_def, self.lexer.pos(), DefType::Struct);
                        self.data
                            .set_returned(a_nr, Type::Reference(a_nr, crate::data::Deps::none()));
                        self.data.definitions[a_nr as usize].parent = d_nr;
                    }
                    if a_nr != u32::MAX {
                        // The declared bounds ride on the placeholder's own `bounds`, the
                        // same list a bounded generic carries, so `check_satisfaction` takes
                        // them without a second representation.
                        //
                        // Re-resolved on BOTH passes, and REPLACED rather than appended: an
                        // interface named here may itself be declared further down the file,
                        // and the first pass then resolves nothing.  A bound that silently
                        // stayed empty would be worse than no bound at all — it reads as a
                        // promise in the source while letting every companion through.
                        let mut resolved: Vec<u32> = Vec::new();
                        for b in &bounds {
                            let b_nr = self.data.def_nr(b);
                            if b_nr == u32::MAX {
                                if !self.first_pass {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "'{b}' is not a known interface"
                                    );
                                }
                            } else if !matches!(self.data.def_type(b_nr), DefType::Interface) {
                                if !self.first_pass {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "'{b}' is not an interface — an associated type's \
                                         bounds must be interface names"
                                    );
                                }
                            } else {
                                resolved.push(b_nr);
                            }
                        }
                        self.data.definitions[a_nr as usize]
                            .bounds
                            .clone_from(&resolved);
                        // The bound is what lets a generic CALL those methods on a value of
                        // the associated type, so it needs the same dispatch stubs a bounded
                        // type variable gets.  This is the whole of "an associated type is a
                        // type variable owned by the interface".
                        let assoc_nr = self.data.def_nr(&assoc_def);
                        self.create_bound_method_stubs(assoc_nr, &resolved);
                    }
                }
                self.lexer.has_token(";");
                continue;
            }
            // I3.1: `op <token> (params) -> type` desugars to an `OpCamelCase` method stub.
            let method_name = if self.lexer.has_keyword("op") {
                if let crate::lexer::LexItem::Token(tok) = self.lexer.peek().has.clone() {
                    self.lexer.cont();
                    // @PLN125 arc C — subscripting is spelled `op [] (self: Self, i: τ)`.
                    // The lexer has no `[]` token (the two brackets are separate, as they
                    // must be for `v[i]`), so the pair is recognised here, where an `op`
                    // has just been read and a `[` can be nothing else.
                    if tok == "[" {
                        self.lexer.token("]");
                        "OpIndex".to_string()
                    } else {
                        format!("Op{}", rename(&tok))
                    }
                } else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Expected operator symbol after 'op' in interface body"
                        );
                    }
                    self.lexer.cont();
                    continue;
                }
            } else {
                if !self.lexer.has_token("fn") {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "Expected 'fn' in interface body");
                    }
                    self.lexer.cont();
                    continue;
                }
                let Some(name) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Expected method name in interface"
                        );
                    }
                    break;
                };
                name
            };
            let mut args = Vec::new();
            if self.lexer.token("(") {
                self.parse_arguments(&method_name, &mut args);
                self.lexer.token(")");
            }
            let return_tp = if self.lexer.has_token("->") {
                self.parse_type_full(d_nr, true)
            } else {
                None
            };
            // I6/I9-stub: register method stubs as children of the interface.
            // Use interface-scoped names (`__iface_{d_nr}_{method}`) to avoid
            // collision when multiple interfaces declare the same operator.
            // `children_of(d_nr)` enumerates them for satisfaction checking;
            // T-stub creation strips the prefix to extract the method name.
            if self.first_pass && d_nr != u32::MAX {
                // The arity is part of the name for the same reason it keys a BOUND stub:
                // `(G-Iface)` calls an interface a set of SIGNATURES, and a name is not one.
                // Without it a second `op -` — binary subtraction beside unary negation, both
                // spelled `OpMin` — collided with the first and the `def_nr` guard below
                // silently dropped it, so the interface never carried two requirements and no
                // bound could offer `a - b` (loft#1275, `D-gen-4`).
                let stub_name = format!("__iface_{d_nr}#{}_{method_name}", args.len());
                if self.data.def_nr(&stub_name) == u32::MAX {
                    let stub_nr =
                        self.data
                            .add_def(&stub_name, self.lexer.pos(), DefType::Function);
                    for a in &args {
                        self.data.add_attribute(
                            &mut self.lexer,
                            stub_nr,
                            &a.name,
                            a.typedef.clone(),
                        );
                    }
                    self.data.definitions[stub_nr as usize].parent = d_nr;
                    // loft#734 — a method with NO `->` returns Void, and the stub
                    // has to say so. Leaving it unset kept the definition's
                    // default `Unknown`, which the native generator renders as
                    // `??` — `fn __iface_N_shut(…) -> ?? {`, invalid Rust, so any
                    // interface with a `close`/`shut`/`flush` method could not be
                    // compiled at all. `--interpret` never asked for the type and
                    // ran correctly, which is what kept it hidden.
                    self.data
                        .set_returned(stub_nr, return_tp.clone().unwrap_or(Type::Void));
                }
            }
            // I5 (phase 1): factory methods (Self in return without self: Self first param)
            // are not yet supported.  Emit a clear diagnostic rather than silently producing
            // wrong code when I6 lands.
            if !self.first_pass {
                let self_nr = self.data.def_nr("Self");
                if self_nr != u32::MAX
                    && let Some(Type::Reference(ret_nr, _)) = &return_tp
                    && *ret_nr == self_nr
                {
                    let has_self_param = args.first().is_some_and(|a| {
                        a.name == "self"
                            && matches!(&a.typedef, Type::Reference(nr, _) if *nr == self_nr)
                    });
                    if !has_self_param {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "factory methods not yet supported: '{}' returns Self without a 'self: Self' parameter",
                            method_name
                        );
                    }
                }
            }
            self.lexer.has_token(";");
        }
        self.lexer.token("}");
        self.lexer.has_token(";");
        self.context = context;
        true
    }

    /// #91: DFS cycle detection on init field dependencies.
    fn check_circular_init(&mut self, init_deps: &[(String, Vec<String>, Position)]) {
        let names: HashSet<String> = init_deps.iter().map(|(n, _, _)| n.clone()).collect();
        for (start, deps, start_pos) in init_deps {
            let mut visited: Vec<String> = vec![start.clone()];
            let mut stack = deps.clone();
            while let Some(dep) = stack.pop() {
                if dep == *start {
                    visited.push(start.clone());
                    let path = visited.join(" -> ");
                    // Each cycle is reported at the field it STARTS from, which is the
                    // field the message names first — so two cycles through the same
                    // fields land on two different lines, as they read.
                    self.lexer.pos_diagnostic(
                        Level::Error,
                        start_pos,
                        &format!("circular init dependency: {path}"),
                    );
                    break;
                }
                if names.contains(&dep) && !visited.contains(&dep) {
                    visited.push(dep.clone());
                    if let Some((_, subdeps, _)) = init_deps.iter().find(|(n, _, _)| *n == dep) {
                        stack.extend(subdeps.clone());
                    }
                }
            }
        }
    }

    /// Mark a just-parsed field as `const` (write-once at construction).  Call
    /// after [`Self::parse_field`], once the field's attribute exists.  Rejects
    /// `const virtual(…)`: a virtual field is already computed and read-only, so
    /// `const` on it is redundant.  See doc/claude/plans/40-const-fields/.
    fn mark_const_field(&mut self, on_d: u32, a_name: &str) {
        let idx = self.data.attr(on_d, a_name);
        if idx == usize::MAX {
            return;
        }
        if self.data.def(on_d).attributes()[idx].constant {
            diagnostic!(
                self.lexer,
                Level::Error,
                "`const virtual(…)` is redundant — a virtual field is already computed and read-only; drop `const`"
            );
        } else {
            self.data.definitions[on_d as usize].attributes[idx].const_field = true;
        }
    }

    // <field> ::= { <field_limit> | 'not' 'null' | <field_default> | 'check' '(' <expr> ')' | <type-id> [ '[' ['-'] <field> { ',' ['-'] <field> } ']' ] } }
    #[allow(clippy::too_many_lines)] // pre-existing length; T1.11a added one branch
    pub(crate) fn parse_field(&mut self, d_nr: u32, a_name: &String) {
        let mut a_type: Type = Type::Unknown(0);
        let mut defined = false;
        let mut value = Value::Null;
        let mut check = Value::Null;
        let mut check_message = Value::Null;
        let mut nullable = true;
        let mut is_computed = false;
        let mut is_init = false;
        // @PLN40 Phase 2 — VALUE-const on the field (`v: const T`): a `const`
        // keyword before the field TYPE marks the field's contents read-only
        // (deep-frozen), distinct from the binding-const PREFIX (`const v: T`)
        // the caller consumes.  Combined as `const v: const T` = fully frozen.
        let mut value_const = false;
        // Post-2c: remember the integer alias name the user typed (e.g. `i32`)
        // so `fill_database` / codegen can consult `forced_size(alias)` even
        // though the resolved Type::Integer collapses the alias info.
        let mut alias_d_nr: u32 = u32::MAX;
        loop {
            // @PLN40 Phase 2 — consume a `const` before the field type (`v: const T`).
            // Runs on BOTH passes so the lexer position stays aligned; the flag is
            // recorded onto the attribute below.  Was previously a parse error
            // ("Undefined type const"), so this is purely additive.
            if self.lexer.has_keyword("const") {
                value_const = true;
            }
            // @PLN25 F2 — `not null` is RETIRED but still ACCEPTED as a no-op (a scalar field
            // is non-null by DEFAULT now; `is_optional` below sets the attribute non-null and
            // the `not_null` flag is stamped for the range). `has_deprecated_not_null` consumes
            // it and emits the deprecation warning; the hard "retired" error stays blocked on
            // the registry republish (RESUME.md § F2 task #4).
            if self.has_deprecated_not_null() {
                nullable = false;
            }
            {
                let (comp, init) =
                    self.parse_field_default(&mut value, &mut a_type, d_nr, a_name, &mut defined);
                is_computed |= comp;
                is_init |= init;
            }
            if self.parse_field_assert(&mut check, &mut check_message) {
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "CHECK" {
                    // Legacy CHECK syntax — parse and discard for backward compat
                    self.lexer.token("(");
                    let mut p = Value::Null;
                    self.expression(&mut p);
                    if self.lexer.has_token(",") {
                        let mut q = Value::Null;
                        self.expression(&mut q);
                    }
                    self.lexer.token(")");
                } else if let Some(tp) = self.parse_type(d_nr, &id, false) {
                    defined = true;
                    // If the type carries a not-null flag (e.g. integer not null),
                    // propagate it to the field's nullable flag so is_null and
                    // redundant-null-check warnings work correctly.
                    if let Type::Integer(IntegerSpec { not_null: true, .. }) = &tp {
                        nullable = false;
                    }
                    // Capture the alias def_nr for size(N) routing.  Only
                    // real aliases (i32, u8, etc.) — "integer" is the base type
                    // and its forced_size is 8, which would override the narrow
                    // limit()-based heuristic for `integer limit(0, 255)`.
                    // @PLN25: peel `Optional(τ)` so a nullable narrow field (`u8?`)
                    // captures its alias and stores at the narrow width like `u8`.
                    if matches!(tp.base(), Type::Integer(_)) && id != "integer" {
                        alias_d_nr = self.data.def_nr(&id);
                    }
                    a_type = tp;
                    // '= expr' shorthand for a field default value
                    self.parse_stored_default(d_nr, a_name, &mut a_type, &mut value);
                    // @PLN86 P6.4 — links after a scalar/named field type.
                    self.parse_field_links(d_nr, a_name);
                }
            } else if let Some(tp) = self.parse_type_full(d_nr, false) {
                // Plan-06 phase 4d: tuple-typed struct fields are now
                // accepted.  Storage layout uses the synthetic
                // `__tuple<…>` struct's positions (registered via
                // `tuple_def`); set/get codegen routes element access
                // through the same OpInt variants used for ordinary
                // struct fields.  See `parser/mod.rs::set_field_check`
                // and `get_val` for the per-element write/read paths.
                defined = true;
                a_type = tp;
                // ⚠ This branch ENDS the field: it breaks out of the loop instead of
                // falling back into it, so every capability a field carries AFTER its
                // type has to be asked for here by name.  The identifier branch above
                // collects them by looping, which is why a capability added there does
                // not appear here on its own.  Each one has a single home, called from
                // both, so the two spellings of a field type stay in step.
                self.parse_stored_default(d_nr, a_name, &mut a_type, &mut value);
                // @PLN86 P6.4 — links after a vector/generic/tuple field type.
                self.parse_field_links(d_nr, a_name);
                self.parse_field_assert(&mut check, &mut check_message);
                break;
            } else {
                break;
            }
        }
        if !defined {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Attribute {a_name} needs type or definition"
            );
        }
        if self.first_pass {
            // @PLN25 F2: a scalar field's nullability is carried by its `Optional` wrapper (DN1's
            // single source of truth), NOT the pre-DN1 parser default (`nullable = true`) or the
            // `not null` keyword.  So a plain scalar field is NON-null (an Integer field also gets
            // the FULL range — no reserved sentinel; only a `u8?` field reserves the top value),
            // and `not null` is redundant on EVERY scalar field — the prerequisite for retiring
            // it.  Now covers all scalars (Integer/Text/Boolean/Float/Single/Character): the
            // attribute flag was the last place a plain non-Integer scalar field still read as
            // nullable (inconsistently — `(N-Store)` already rejected a `null` store into it).
            // loft#995 — asked of EVERY field kind, not only the scalars the DN1 rollout
            // reached first.  A record, an enum, a vector and a keyed collection kept the
            // pre-DN1 parser default (`true`), so `FieldInfo.nullable` — documented as
            // "was the field DECLARED nullable" and named as the fact a generated
            // `CREATE TABLE` needs for `NOT NULL` — answered a constant `true` for all
            // four, and a serialiser dropped a `NOT NULL` the declaration had asked for.
            // The two spellings genuinely differ (`r: At107` cannot hold null, `rq:
            // At107?` can), so this was a fact being LOST, not two things that are one.
            // The synthetic tuple attributes have derived it this way from every element
            // type since @PLN114; a declared field now agrees with them.
            //
            // The ONE kind where the `?` is not the question is `reference<T>` in field
            // position: #328 made that the documented POINTER, and a pointer holds null
            // whatever it is spelled — `n.next = null` is legal on it and an omitted one
            // DEFAULTS to null, both pinned by `issue_328_reference_field_pointer_
            // semantics`.  Deriving from the wrapper there reported a field non-null that
            // the same test then compares against null, and the redundant-null-check
            // warning said so.  The pointer marker (`u16::MAX` dep) is what the parse
            // stamps to select that layout, so it is the exact discriminator: a by-VALUE
            // `r: At107` is `Reference` too and genuinely cannot hold null.
            let is_pointer_field = matches!(&a_type, Type::Reference(_, deps)
                if deps.is_pointer_marker());
            if crate::keys::pln25_f2_enabled() && !is_pointer_field {
                nullable = matches!(a_type, Type::Optional(_));
            }
            // H6: stamp the field's nullability onto its integer spec so the
            // STORED attribute type is self-describing.  The alias path
            // (`u8 not null`) otherwise leaves `not_null` at the alias default
            // (`false`), and the literal range-check (`int_value_fits`) reads the
            // usable bounds off `not_null` — so a NOT-NULL `u8` field must report
            // `not_null:true` or its `255` would be wrongly rejected.  `nullable`
            // here is exactly what `set_attr_nullable` records, so the spec flag
            // and the attribute flag cannot disagree.
            if let Type::Integer(ref mut spec) = a_type {
                spec.not_null = !nullable;
            }
            self.reject_duplicate_index(d_nr, a_name, &a_type);
            let a = self
                .data
                .add_attribute(&mut self.lexer, d_nr, a_name, a_type);
            if value_const {
                self.data.definitions[d_nr as usize].attributes[a].value_const = true;
            }
            self.data.set_attr_nullable(d_nr, a, nullable);
            self.data.set_attr_value(d_nr, a, value);
            if alias_d_nr != u32::MAX {
                self.data.definitions[d_nr as usize].attributes[a].alias_d_nr = alias_d_nr;
            }
            if is_computed {
                self.data.definitions[d_nr as usize].attributes[a].constant = true;
            }
            if is_init {
                self.data.definitions[d_nr as usize].attributes[a].init = true;
            }
            if check != Value::Null {
                self.data.definitions[d_nr as usize].attributes[a].check = check;
                self.data.definitions[d_nr as usize].attributes[a].check_message = check_message;
            }
        } else {
            let a = self.data.attr(d_nr, a_name);
            if value_const {
                self.data.definitions[d_nr as usize].attributes[a].value_const = true;
            }
            if is_computed {
                self.data.definitions[d_nr as usize].attributes[a].constant = true;
            }
            if is_init {
                self.data.definitions[d_nr as usize].attributes[a].init = true;
            }
            if value != Value::Null {
                self.data.set_attr_value(d_nr, a, value);
            }
            if check != Value::Null {
                self.data.definitions[d_nr as usize].attributes[a].check = check;
                self.data.definitions[d_nr as usize].attributes[a].check_message = check_message;
            }
        }
    }

    // <field_default> ::= 'virtual' <value-expr> | 'init' '(' <value-expr> ')'
    //                   | 'default' '(' <value-expr> ')'
    // Returns (is_computed, is_init).
    /// Parse a field's `assert(condition)` / `assert(condition, message)` check.
    ///
    /// One home for both field-type branches of `parse_field`.  Answers whether a check
    /// was consumed, so the identifier branch can keep using it as the head of its
    /// if-chain while the tuple branch, which ends the field itself, calls it directly.
    pub(crate) fn parse_field_assert(&mut self, check: &mut Value, message: &mut Value) -> bool {
        if !self.lexer.has_token("assert") {
            return false;
        }
        self.lexer.token("(");
        self.expression(check);
        if self.lexer.has_token(",") {
            self.expression(message);
        }
        self.lexer.token(")");
        true
    }

    /// Parse the `= expr` shorthand that gives a struct field a stored default.
    ///
    /// One home for every field type former.  A field whose type is written as an
    /// IDENTIFIER (`integer`, `text`, `vector<T>`, a struct name) and one written as a
    /// TUPLE (`(text, text)`) reach `parse_field` down different branches, and a default
    /// means the same thing in both: an expression lowered in the STRUCT's context, which
    /// has no frame, and replayed at every construction site.
    ///
    /// Answers whether a default was consumed.
    pub(crate) fn parse_stored_default(
        &mut self,
        d_nr: u32,
        a_name: &String,
        a_type: &mut Type,
        value: &mut Value,
    ) -> bool {
        if !self.lexer.has_token("=") {
            return false;
        }
        // #91: enable dep tracking so $.field accesses are recorded
        // for circular-init detection (same as init(expr) path).
        self.init_field_tracking = true;
        self.init_field_deps.clear();
        self.init_reads_record = false;
        // @PLN22 Phase 1 — hint the field's enum so a bare variant
        // default (`level: Level = Warning`) resolves against the
        // declared field type.
        // loft#1067 — a DEFAULT is checked against the declared type, so
        // `fn takes(f: fn(integer) -> integer = |x| { x * 2 })` infers `x`
        // exactly as a caller passing the same lambda would.
        if self.enum_context(a_type) || Self::seeds_lambda_hint(a_type) {
            self.expected = a_type.clone();
        }
        // loft#698 — where the default's value is BUILT decides whether it
        // can be replayed.  Mark the source position first: a default that
        // turns out to need a temporary is re-parsed from here into a
        // function of its own (`default_value_fn`).
        let value_start = self.lexer.link();
        let tp = self.expression(value);
        self.expected = Type::Unknown(0);
        self.init_field_tracking = false;
        if a_type.is_unknown() {
            *a_type = tp;
        }
        // A default is lowered HERE, in the STRUCT's context, which has no
        // frame, and replayed at every construction site inside some
        // FUNCTION.  So it may reference only the record being built
        // (`Var(0)`, the `$` placeholder); any other variable is an index
        // into this struct's variable table, which is discarded before
        // replay.  The indices then resolved against whatever locals the
        // construction site happened to have, and the default's own
        // `OpDatabase` re-allocated one of them mid-construction — `= [1, 2]`
        // hung, `text = "a" + "b"` SIGSEGV'd.
        //
        // Every default that needs no temporary needs no help: a scalar, a
        // text literal, arithmetic, a struct literal, `= []`.  One that DOES
        // is re-parsed into a function of its own, and the stored default
        // becomes the var-free call to it — so nothing crosses tables.
        //
        // A default reading `$` is the one shape that cannot move: it needs
        // the record, which the function it would move into does not have.
        // Needing BOTH `$` and a temporary stays refused, and says so.
        // `$` is `Var(0)`, and so is the first temporary the struct's empty
        // table hands out — the dep tracking above is what tells the two
        // apart, so the replay question cannot be asked without it.
        let reads_record = self.init_reads_record;
        let site = DefaultSite::Field {
            reads_record,
            struct_typed: matches!(a_type.base(), Type::Reference(_, _)),
        };
        let dflt_fn = format!("__dflt_{}_{a_name}", self.data.def(d_nr).name());
        if self.default_hoisted_in_pass_1(&dflt_fn) || !default_replayable_in_place(value, site) {
            // A non-empty KEYED default was refused here because loft had no
            // keyed literal as a VALUE at all — `[K { … }]` was a `vector<K>`
            // wherever it stood alone, so there was nothing for the function
            // it moves into to return.  loft#703 gave it one, so it is now
            // lowered like any other default needing a temporary.
            if !reads_record {
                self.lexer.revert(value_start);
                *value = self.default_value_fn(&dflt_fn, &[], a_type, Vec::new());
            } else if !self.first_pass {
                let tn = a_type.name(&self.data);
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "the default for `{a_name}: {tn}` both reads `$` and \
                     needs a temporary value — one that reads `$` is built \
                     against the record at every construction site, so it \
                     cannot be built once and shared; drop the `$` reference \
                     or set the field at each construction site"
                );
            }
        }
        true
    }
    pub(crate) fn parse_field_default(
        &mut self,
        value: &mut Value,
        a_type: &mut Type,
        _d_nr: u32,
        _a_name: &String,
        defined: &mut bool,
    ) -> (bool, bool) {
        let mut is_computed = false;
        let mut is_init = false;
        if self.lexer.has_keyword("computed") || self.lexer.has_keyword("virtual") {
            is_computed = true;
            // Computed field: calculate on every access, no store space.
            self.lexer.token("(");
            let tp = self.expression(value);
            if a_type.is_unknown() {
                *a_type = tp;
                *defined = true;
            } else {
                self.convert(value, &tp, a_type);
            }
            self.lexer.token(")");
        }
        if self.lexer.has_keyword("init") {
            is_init = true;
            // L7: init(expr) — stored at creation, writable after. $ allowed.
            // #91: enable dep tracking for circular-init detection.
            self.init_field_tracking = true;
            self.init_field_deps.clear();
            self.init_reads_record = false;
            self.lexer.token("(");
            let tp = self.expression(value);
            if a_type.is_unknown() {
                *a_type = tp;
                *defined = true;
            } else {
                self.convert(value, &tp, a_type);
            }
            self.lexer.token(")");
            self.init_field_tracking = false;
        }
        if self.lexer.has_keyword("default") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "default(expr) is removed; use 'computed(expr)' for calculated fields or '= expr' for stored defaults"
            );
            self.lexer.token("(");
            self.expression(value);
            self.lexer.token(")");
        }
        (is_computed, is_init)
    }

    /// loft#699 — the name of the function a parameter default is lowered into.
    ///
    /// Keyed by the RECEIVER type as well as the method name, exactly as `add_fn` mangles
    /// the method itself: a type's methods are global but two types may each carry a
    /// `scale`, and their generated defaults would otherwise collide under one name.
    fn default_fn_name(&self, fn_name: &str, arguments: &[Argument], a_name: &str) -> String {
        let owner = match arguments.first() {
            Some(a) if a.name == "self" || a.name == "both" => {
                let tn = self.data.type_def_nr(&a.typedef);
                if tn == u32::MAX {
                    String::new()
                } else {
                    format!("{}_", self.data.def(tn).name())
                }
            }
            _ => String::new(),
        };
        format!("__dflt_{owner}{fn_name}_{a_name}")
    }

    /// Whether pass 1 already lowered this default into a function of its own.
    ///
    /// The hoist decision belongs to pass 1, and pass 2 has to reach the same one.  A
    /// name declared LATER in the file does not resolve during pass 1, so the default
    /// parses as a fresh variable and reads as "needs a temporary"; the identical source
    /// is a literal in pass 2 and reads as "replayable".  Pass 1 mints the function
    /// either way, so a pass 2 that declines to re-mint leaves that function holding a
    /// variable no frame ever allocated — codegen then aborts with `Incorrect var
    /// NAME[65535]` on a program whose only fault is the order of two declarations
    /// (loft#1086).
    ///
    /// Asking whether the definition is already there is what makes pass 1's decision
    /// the one that stands: pass 2 re-parses the same default into the same function,
    /// where the name now resolves.  The opposite divergence — pass 2 wanting a
    /// definition pass 1 never minted — is rejected separately, because a definition may
    /// not first appear in pass 2.
    fn default_hoisted_in_pass_1(&self, fn_name: &str) -> bool {
        !self.first_pass && self.data.def_nr(&format!("n_{fn_name}")) != u32::MAX
    }

    /// The definition pass 1 minted for this default, under either key it could carry.
    ///
    /// A minted default takes the earlier parameters it reads, so one inside a METHOD
    /// that reads `self` is registered under the receiver key rather than `n_<name>`.
    /// Which of the two pass 1 used depends on the tree pass 1 parsed, and pass 2 does
    /// not have that tree — so try both.
    fn minted_default_nr(&self, dflt_fn: &str, arguments: &[Argument]) -> u32 {
        let plain = self.data.def_nr(&format!("n_{dflt_fn}"));
        if plain != u32::MAX {
            return plain;
        }
        let Some(key) = self.data.fn_key(dflt_fn, arguments) else {
            return u32::MAX;
        };
        self.data.def_nr(&key)
    }

    /// The signature pass 1 gave this default's function, as `(parameters, call
    /// arguments)`, or `None` when pass 1 minted nothing.
    ///
    /// A minted function is created once, in pass 1, and pass 2 re-parses its body into
    /// the same definition — so pass 2 must hand it the parameters it already has.
    /// Recomputing them from the pass-2 tree gives a different list whenever pass 1
    /// resolved fewer names, and the body is then parsed against parameters the
    /// definition does not carry: the name reads back as an unknown variable
    /// (loft#1086).
    fn hoisted_default_signature(
        &self,
        fn_name: &str,
        arguments: &[Argument],
    ) -> Option<(Vec<Argument>, Vec<Value>)> {
        if self.first_pass {
            return None;
        }
        let d_nr = self.minted_default_nr(fn_name, arguments);
        if d_nr == u32::MAX {
            return None;
        }
        let params: Vec<Argument> = self
            .data
            .def(d_nr)
            .attributes()
            .iter()
            .map(|a| Argument {
                name: a.name.clone(),
                typedef: a.typedef.clone(),
                default: Value::Null,
                constant: false,
                ref_pos: (0, 0),
                const_pos: (0, 0),
            })
            .collect();
        // Each parameter by NAME, because the call passes the caller's argument at that
        // index and the two lists are ordered independently.
        let call_args = params
            .iter()
            .map(|p| {
                let at = arguments.iter().position(|a| a.name == p.name);
                #[allow(clippy::cast_possible_truncation)]
                Value::Var(at.map_or(u16::MAX, |i| i as u16))
            })
            .collect();
        Some((params, call_args))
    }

    /// Every earlier parameter that was in scope for the default's parse, as a signature.
    ///
    /// The safe over-approximation [`Self::default_fn_params`] falls back to, reached
    /// here for a different reason: a pass-1 tree that lost a name to a forward
    /// reference no longer mentions the parameters the default reads, so asking it which
    /// ones they are answers "none".
    fn every_earlier_parameter(
        arguments: &[Argument],
        injected: &[(String, u16, u16)],
    ) -> (Vec<Argument>, Vec<Value>) {
        let used: Vec<u16> = injected.iter().map(|(_, _, arg_idx)| *arg_idx).collect();
        let params = used
            .iter()
            .map(|i| Argument {
                name: arguments[*i as usize].name.clone(),
                typedef: arguments[*i as usize].typedef.clone(),
                default: Value::Null,
                constant: false,
                ref_pos: (0, 0),
                const_pos: (0, 0),
            })
            .collect();
        (params, used.iter().map(|i| Value::Var(*i)).collect())
    }

    /// loft#699 — the parameters the function a default is lowered into must take, and
    /// the arguments the stored call passes them.
    ///
    /// Only the EARLIER parameters the default actually references: passing the rest
    /// would borrow heap arguments the body never reads.  When the tree holds a shape
    /// `visit_constant_vars` cannot enumerate, fall back to every earlier parameter that
    /// was in scope for the parse — the body must be able to resolve any name it used,
    /// and an over-wide signature is merely wasteful where a short one fails to compile.
    fn default_fn_params(
        val: &Value,
        arguments: &[Argument],
        injected: &[(String, u16, u16)],
    ) -> (Vec<Argument>, Vec<Value>) {
        let count = arguments.len() as u16;
        let mut used: Vec<u16> = Vec::new();
        let mut probe = val.clone();
        let complete = visit_constant_vars(&mut probe, &mut |v| {
            if *v < count {
                used.push(*v);
            }
        });
        if complete {
            used.sort_unstable();
            used.dedup();
        } else {
            used = injected.iter().map(|(_, _, arg_idx)| *arg_idx).collect();
        }
        let params = used
            .iter()
            .map(|i| Argument {
                name: arguments[*i as usize].name.clone(),
                typedef: arguments[*i as usize].typedef.clone(),
                default: Value::Null,
                constant: false,
                ref_pos: (0, 0),
                const_pos: (0, 0),
            })
            .collect();
        let call_args = used.iter().map(|i| Value::Var(*i)).collect();
        (params, call_args)
    }

    /// loft#698 / loft#699 — lower a default that needs a temporary into a function of
    /// its own, and return the call that stands in for it.
    ///
    /// The lexer must sit at the start of the default's expression: it is parsed HERE,
    /// in the new function's context, so every temporary it needs is numbered in that
    /// function's variable table and never leaves it.  What gets stored is then a `Call`
    /// naming only `params` — the one shape every replay site can already handle, and
    /// the shape `default_replayable_in_place` already calls safe.
    ///
    /// The value comes back BY RETURN rather than being written in place.  That is what
    /// keeps an INLINE nested struct working: the caller writes the field, so it applies
    /// its own `pos + fld` offset — a function writing through the record could only bake
    /// offsets that are right at the top level.  It also means the whole class is covered
    /// by one mechanism, because "a value returned from a call" is what a replay site
    /// already handles for a supplied `S { v: build() }` / `f(1, build())`, on both
    /// backends.
    ///
    /// A FIELD default takes no parameters — the record is the only thing it could name,
    /// and one that names it cannot move at all.  A PARAMETER default takes the earlier
    /// parameters it references, and `call_args` names them by ARGUMENT INDEX so
    /// `substitute_param_refs` transplants each into the caller's actual argument.
    ///
    /// The definition is minted on BOTH passes.  `default_replayable_in_place` reads the same
    /// on each (a `[1, 2]` default is a `Vector` block in pass 1 as well), so the decision
    /// cannot diverge — which it must not, because H5 rejects any definition that first
    /// appears in pass 2.
    fn default_value_fn(
        &mut self,
        fn_name: &str,
        params: &[Argument],
        a_type: &Type,
        call_args: Vec<Value>,
    ) -> Value {
        // The key `add_fn` registered it under in pass 1, derived from the same
        // parameters — a default inside a method that reads `self` is itself a method,
        // and `n_<name>` does not find it.
        let stored_name = self
            .data
            .fn_key(fn_name, params)
            .unwrap_or_else(|| format!("n_{fn_name}"));
        let outer_context = self.context;
        let outer_vars = std::mem::replace(
            &mut self.vars,
            Function::new(fn_name, &self.lexer.pos().file),
        );
        let outer_loop = self.in_loop;
        self.in_loop = false;
        self.context = if self.first_pass {
            self.data.add_fn(&mut self.lexer, fn_name, params)
        } else {
            self.data.def_nr(&stored_name)
        };
        if self.context == u32::MAX {
            self.context = outer_context;
            self.vars = outer_vars;
            self.in_loop = outer_loop;
            return Value::Null;
        }
        let d_nr = self.context;
        if self.first_pass {
            self.data.set_returned(d_nr, a_type.clone());
        }
        self.vars
            .append(&mut self.data.definitions[d_nr as usize].variables);
        // The referenced earlier parameters, as this function's own arguments — in the
        // order `call_args` passes them, so index `k` here is `call_args[k]` there.
        for (a_nr, a) in params.iter().enumerate() {
            if self.first_pass {
                let v_nr = self.create_var(&a.name, &a.typedef);
                if v_nr != u16::MAX {
                    self.vars.become_argument(v_nr);
                    self.var_usages(v_nr, false);
                    self.vars.mark_used(v_nr);
                }
            } else {
                self.change_var_type(a_nr as u16, &a.typedef);
            }
        }
        // The declared field type is the hint the default was already parsed under, and
        // the collection kinds NEED it: `= [K { … }]` is a `vector<K>` literal until the
        // expected type says the field is a `hash<K[k]>`.
        let result = self.data.def(d_nr).returned().clone();
        let mut expr = Value::Null;
        self.expected = result.clone();
        let tp = self.expression(&mut expr);
        self.expected = Type::Unknown(0);
        if !self.first_pass && !result.is_unknown() {
            self.convert(&mut expr, &tp, &result);
        }
        // The tail still has to be DELIVERED, which for a body read from source is what
        // `parse_block`'s "return from block" context does — it binds the value to the
        // function's return buffer (threading that buffer into a tail call, so the callee
        // writes straight into it) and wraps the tail in a `Return`.  Skipped, a heap
        // value belonged to nobody: it was freed at scope exit and the function returned
        // null, so the caller read a freed store — invisible in a normal run, a SIGSEGV
        // under the POISON gate.
        let mut ops = vec![expr];
        let pos = self.lexer.pos().clone();
        let tail = self.block_result("return from block", &result, &tp, &mut ops, &pos);
        self.finish_body(v_block(ops, tail, "block"), Type::Void);
        self.data.definitions[d_nr as usize]
            .variables
            .append(&mut self.vars);
        self.context = outer_context;
        self.vars = outer_vars;
        self.in_loop = outer_loop;
        self.data.def_used(d_nr);
        Value::Call(d_nr, call_args)
    }

    /// The mangled name of a type's synthesized drop CASCADE — `t_<LEN><Type>_OpDropAll`.
    ///
    /// Keyed exactly like the hook itself (`Data::drop_hook_nr`), so a cascade for a
    /// library's type resolves through that library's source the same way its hook does.
    /// The `_OpDropAll` suffix is deliberately not `_OpDrop`: `check_drop_signature` keys on
    /// the latter, and a synthesized function must not be validated as a user declaration.
    pub(crate) fn drop_cascade_name(data: &crate::data::Data, type_def: u32) -> String {
        let n = data.def(type_def).name();
        format!("t_{}{}_OpDropAll", n.len(), n)
    }

    /// @PLN139 stage B — give every type that OWNS a droppable through a field a function
    /// that releases what it owns, so a container's death releases its members.
    ///
    /// The cascade is a synthesized loft function rather than IR emitted at each free site,
    /// because the free sites live in `scopes.rs`, which holds `Data` and NOT the schema —
    /// and a field cascade is nothing but field offsets. Here both are in hand, one function
    /// per type serves every site that drops one, and nesting falls out of each cascade
    /// calling the next.
    ///
    /// Body order is **the container's own hook first, then its fields in reverse
    /// declaration order**. The container runs before the things it owns for the reason
    /// RAII always gives: a wrapper's own release may still need the resource it wraps (a
    /// connection says goodbye over the socket it is about to close). Reverse declaration
    /// order among the fields matches the scope-exit free order a reader already knows.
    ///
    /// Runs at the end of pass 2, where every type and every `OpDrop` are known, and is
    /// IDEMPOTENT — a type whose cascade already exists is skipped, so the several parse
    /// tails that call it cost nothing after the first.
    ///
    /// **Stage B is fields only.** A droppable reachable only through an enum payload or a
    /// collection element is deliberately NOT cascaded yet (stages D and E): emitting half a
    /// cascade would be worse than none, because a partial release reads as a working one.
    /// So the members walked here are exactly the members emitted here.
    pub(crate) fn synth_drop_cascades(&mut self) {
        // Cheap exit for the overwhelmingly common program: no `OpDrop` anywhere means no
        // type can own a droppable, so nothing below can fire.
        if !self.data.any_drop_hook() {
            return;
        }
        let mut targets: Vec<u32> = Vec::new();
        for d_nr in 0..self.data.definitions() {
            if self.data.def_nr(&Self::drop_cascade_name(&self.data, d_nr)) != u32::MAX {
                continue; // already synthesized — the several parse tails share this pass
            }
            let wanted = match self.data.def_type(d_nr) {
                // A STRUCT, and an enum VARIANT, both release their own droppable fields —
                // a variant is a record whose attributes are its payload.
                DefType::Struct | DefType::EnumValue => {
                    self.data.def(d_nr).known_type() != u16::MAX
                        && (!self.cascade_fields(d_nr).is_empty()
                            || !self.cascade_vectors(d_nr).is_empty())
                }
                // An ENUM releases through whichever variant it currently holds.
                DefType::Enum => !self.cascade_variants(d_nr).is_empty(),
                _ => false,
            };
            if wanted {
                targets.push(d_nr);
            }
        }
        // Two phases, because a cascade body CALLS the cascade of each droppable field and
        // each droppable variant, and a nested chain would otherwise depend on definition
        // order: declare them all, then fill the bodies once every name resolves.
        let mut made: Vec<(u32, u32)> = Vec::new();
        for &t in &targets {
            let name = Self::drop_cascade_name(&self.data, t);
            let pos = self.data.def(t).position().clone();
            let c_nr = self.data.add_def(&name, &pos, DefType::Function);
            self.data.set_returned(c_nr, Type::Void);
            let self_tp = self.cascade_self_type(t);
            let _ = self
                .data
                .add_attribute(&mut self.lexer, c_nr, "self", self_tp);
            made.push((t, c_nr));
        }
        for (t, c_nr) in made {
            if self.data.def_type(t) == DefType::Enum {
                self.fill_enum_drop_cascade(t, c_nr);
            } else {
                self.fill_drop_cascade(t, c_nr);
            }
        }
    }

    /// The `self` parameter type of a type's cascade — a struct-enum carries its
    /// discriminator, so its cascade must be typed as the ENUM and not as a bare record.
    fn cascade_self_type(&self, t: u32) -> Type {
        if self.data.def_type(t) == DefType::Enum {
            Type::Enum(t, true, crate::data::Deps::none())
        } else {
            Type::Reference(t, crate::data::Deps::none())
        }
    }

    /// @PLN139 stage D — the variants of `e_nr` that own a droppable, each with the
    /// DISCRIMINATOR value that selects it: the 1-based position of the variant's name in
    /// the enum's attribute list, the same numbering `fill_database` writes at construction.
    ///
    /// A variant with nothing to release gets no arm at all, so a unit-only enum synthesizes
    /// no cascade and an enum with one droppable variant tests once rather than per variant.
    fn cascade_variants(&self, e_nr: u32) -> Vec<(u32, i32)> {
        let mut out = Vec::new();
        for v in self.data.children_of(e_nr) {
            if self.data.def_type(v) != DefType::EnumValue
                || self.data.def(v).known_type() == u16::MAX
            {
                continue;
            }
            if self.cascade_fields(v).is_empty() && self.cascade_vectors(v).is_empty() {
                continue;
            }
            let vname = self.data.def(v).name().to_string();
            let Some(idx) = self
                .data
                .def(e_nr)
                .attributes()
                .iter()
                .position(|a| a.name == vname)
            else {
                continue;
            };
            out.push((v, i32::try_from(idx).unwrap_or(0) + 1));
        }
        out
    }

    /// Build the body of an ENUM's cascade: read the discriminator once, then release
    /// through the variant that is actually present.
    ///
    /// The variant's own cascade does the releasing, so this function is pure dispatch and a
    /// payload's nesting needs no special case here. `self` is a reference to the variant
    /// RECORD (the discriminator sits at its head), which is why the arm can hand `self`
    /// straight to a cascade whose parameter is typed as that variant.
    fn fill_enum_drop_cascade(&mut self, t: u32, c_nr: u32) {
        let variants = self.cascade_variants(t);
        let Some(&(first, _)) = variants.first() else {
            return;
        };
        let disc_pos = self
            .database
            .position(self.data.def(first).known_type(), "enum");
        let name = Self::drop_cascade_name(&self.data, t);
        let file = self.data.def(t).position().file.clone();
        let mut vars = Function::new(&name, &file);
        let self_tp = self.cascade_self_type(t);
        let self_var = vars.add_variable("self", &self_tp, &mut self.lexer);
        vars.become_argument(self_var);
        vars.defined(self_var);
        let outer_vars = std::mem::replace(&mut self.vars, vars);
        let outer_context = self.context;
        self.context = c_nr;

        let mut ops: Vec<Value> = Vec::new();
        let own = self.data.drop_hook_nr(t);
        if own != u32::MAX {
            ops.push(Value::Call(own, vec![Value::Var(self_var)]));
        }
        let get_enum = self.cl(
            "OpGetEnum",
            &[Value::Var(self_var), Value::Int(i32::from(disc_pos))],
        );
        let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
        for (v, number) in variants {
            let target = self.data.drop_cascade_nr(v);
            if target == u32::MAX {
                continue;
            }
            let test = self.cl("OpEqInt", &[disc.clone(), Value::Int(number)]);
            ops.push(v_if(
                test,
                Value::Call(target, vec![Value::Var(self_var)]),
                Value::Null,
            ));
        }

        let body = v_block(ops, Type::Void, "drop_cascade_enum");
        let built = std::mem::replace(&mut self.vars, outer_vars);
        self.context = outer_context;
        self.data.definitions[c_nr as usize].code = body;
        self.data.definitions[c_nr as usize].variables = built;
        self.data.def_used(c_nr);
    }

    /// The DIRECT struct fields a stage-B cascade releases: `(byte offset, field type, field
    /// definition)` for each field whose own type owns a droppable, in declaration order.
    ///
    /// Only `Reference` fields — a dense inline sub-record, whose offset is the whole of what
    /// releasing it needs. An enum-payload or collection field is left for stages D/E and is
    /// therefore NOT reported here, so `synth_drop_cascades` never declares a cascade it
    /// cannot fully fill.
    fn cascade_fields(&self, d_nr: u32) -> Vec<(u16, Type, u32)> {
        let kt = self.data.def(d_nr).known_type();
        let mut out = Vec::new();
        for a_nr in 0..self.data.def(d_nr).attributes().len() {
            let a = &self.data.def(d_nr).attributes()[a_nr];
            if a.hidden {
                continue;
            }
            let Type::Reference(fd, _) = a.typedef.base() else {
                continue;
            };
            let fd = *fd;
            if fd == d_nr || !self.data.owns_droppable(fd) {
                continue; // a self-field cannot exist inline; skip defensively
            }
            let name = self.data.attr_name(d_nr, a_nr);
            let off = self.database.position(kt, &name);
            if off == u16::MAX {
                continue; // not laid out in this record — nothing to reach
            }
            out.push((off, a.typedef.base().clone(), fd));
        }
        out
    }

    /// @PLN139 stage E — the COLLECTION fields a cascade releases element by element:
    /// `(byte offset, vector type, element type, element definition)`.
    ///
    /// Owning a collection of droppables is owning the droppables, so the container's death
    /// releases every element. Only `Vector` — the keyed collections (`hash`/`sorted`/…) share
    /// their records with the collections they are indexed from, so releasing through one
    /// would release somebody else's element; they need the ownership question answered first
    /// and are deliberately out of stage E.
    fn cascade_vectors(&self, d_nr: u32) -> Vec<(u16, Type, Type, u32)> {
        let kt = self.data.def(d_nr).known_type();
        let mut out = Vec::new();
        for a_nr in 0..self.data.def(d_nr).attributes().len() {
            let a = &self.data.def(d_nr).attributes()[a_nr];
            if a.hidden {
                continue;
            }
            let Type::Vector(elm, _) = a.typedef.base() else {
                continue;
            };
            let elm = (**elm).clone();
            let (Type::Reference(ed, _) | Type::Enum(ed, true, _)) = elm.base() else {
                continue;
            };
            let ed = *ed;
            if !self.data.owns_droppable(ed) {
                continue;
            }
            let name = self.data.attr_name(d_nr, a_nr);
            let off = self.database.position(kt, &name);
            if off == u16::MAX {
                continue;
            }
            out.push((off, a.typedef.base().clone(), elm, ed));
        }
        out
    }

    /// The per-element release loop for one collection field — see [`Self::cascade_vectors`].
    ///
    /// Shaped check-then-read rather than the read-then-check a `for` loop lowers to, because
    /// there is no user body that could shrink the vector mid-walk: a drop receives only
    /// `self`. The length is still re-read each iteration rather than hoisted, so the loop
    /// cannot outrun a vector that changed under it.
    ///
    /// The element variable is `skip_free`: it is a VIEW into the container's own storage, and
    /// the container's cascade runs immediately before the free that releases that storage.
    /// Freeing it here would release the container's block one element at a time.
    fn drop_elements_loop(
        &mut self,
        self_var: u16,
        idx: usize,
        off: u16,
        vec_tp: &Type,
        elem_tp: &Type,
        target: u32,
    ) -> Value {
        let int_tp = self
            .data
            .def(self.data.def_nr("integer"))
            .returned()
            .clone();
        let i_var = self
            .vars
            .add_variable(&format!("__dc_i{idx}"), &int_tp, &mut self.lexer);
        let e_var = self
            .vars
            .add_variable(&format!("__dc_e{idx}"), elem_tp, &mut self.lexer);
        self.vars.set_skip_free(e_var);
        let field = self.get_val(
            vec_tp,
            false,
            u32::from(off),
            Value::Var(self_var),
            u32::MAX,
        );

        let len = self.cl("OpLengthVector", std::slice::from_ref(&field));
        let past_end = self.cl("OpLeInt", &[len, Value::Var(i_var)]);
        let stride = self.vector_elem_iter_stride(elem_tp);
        let vec_def = self.data.type_def_nr(elem_tp);
        let db_tp = self.data.def(vec_def).known_type();
        let read = if self.database.is_linked(db_tp) {
            self.cl("OpVectorRefNullable", &[field.clone(), Value::Var(i_var)])
        } else {
            self.cl(
                "OpGetVectorNullable",
                &[
                    field.clone(),
                    Value::Int(i32::from(stride)),
                    Value::Var(i_var),
                ],
            )
        };
        let live = self.cl("OpConvBoolFromRef", &[Value::Var(e_var)]);
        let step = self.cl("OpAddInt", &[Value::Var(i_var), Value::Int(1)]);

        let body = vec![
            v_if(
                past_end,
                v_block(vec![Value::Break(0)], Type::Void, "break"),
                Value::Null,
            ),
            crate::data::v_set(e_var, read),
            v_if(
                live,
                Value::Call(target, vec![Value::Var(e_var)]),
                Value::Null,
            ),
            crate::data::v_set(i_var, step),
        ];
        v_block(
            vec![
                crate::data::v_set(i_var, Value::Int(0)),
                crate::data::v_loop(body, "drop_elements"),
            ],
            Type::Void,
            "drop_elements_block",
        )
    }

    /// Build the body of the cascade declared for `t` — see [`Self::synth_drop_cascades`].
    fn fill_drop_cascade(&mut self, t: u32, c_nr: u32) {
        let name = Self::drop_cascade_name(&self.data, t);
        let file = self.data.def(t).position().file.clone();
        let mut vars = Function::new(&name, &file);
        let self_tp = Type::Reference(t, crate::data::Deps::none());
        let self_var = vars.add_variable("self", &self_tp, &mut self.lexer);
        vars.become_argument(self_var);
        vars.defined(self_var);
        // Build the body with the cascade's OWN table current, so anything `get_val` mints
        // for a field read lands in the function that will hold the code.
        let outer_vars = std::mem::replace(&mut self.vars, vars);
        let outer_context = self.context;
        self.context = c_nr;

        let mut ops: Vec<Value> = Vec::new();
        let own = self.data.drop_hook_nr(t);
        if own != u32::MAX {
            ops.push(Value::Call(own, vec![Value::Var(self_var)]));
        }
        for (n, (off, vec_tp, elem_tp, ed)) in self.cascade_vectors(t).into_iter().enumerate().rev()
        {
            let target = self.data.drop_cascade_nr(ed);
            if target == u32::MAX {
                continue;
            }
            let loop_code = self.drop_elements_loop(self_var, n, off, &vec_tp, &elem_tp, target);
            ops.push(loop_code);
        }
        for (off, ftype, fd) in self.cascade_fields(t).into_iter().rev() {
            let target = self.data.drop_cascade_nr(fd);
            if target == u32::MAX {
                continue;
            }
            let field = self.get_val(
                &ftype,
                false,
                u32::from(off),
                Value::Var(self_var),
                u32::MAX,
            );
            // Guarded for the same reason the scope-exit call is: the free is null-tolerant
            // and a drop is not, so a field on a record that was never written must not run
            // the author's release against a record that does not exist.
            let live = self.cl("OpConvBoolFromRef", std::slice::from_ref(&field));
            ops.push(Value::If(
                Box::new(live),
                Box::new(Value::Call(target, vec![field])),
                Box::new(Value::Null),
            ));
        }

        let body = v_block(ops, Type::Void, "drop_cascade");
        let built = std::mem::replace(&mut self.vars, outer_vars);
        self.context = outer_context;
        self.data.definitions[c_nr as usize].code = body;
        self.data.definitions[c_nr as usize].variables = built;
        self.data.def_used(c_nr);
    }
}

/// Visit every variable index a constant's initialiser carries, in place.
///
/// Returns `false` on the first node whose variable numbering this walker cannot
/// account for.  A caller re-pointing a pasted constant onto a fresh buffer has to
/// rewrite ALL of the numbering or none of it — a half-rewritten block would read one
/// buffer and write another.  The `match` is exhaustive on purpose: a new IR variant
/// that carries a variable makes this fail to compile rather than silently paste
/// numbering that is only valid where it was parsed.
pub(crate) fn visit_constant_vars(val: &mut Value, f: &mut dyn FnMut(&mut u16)) -> bool {
    match val {
        // The two forms that name a variable a constant's initialiser owns.
        Value::Var(v) => {
            f(v);
            true
        }
        Value::Set(v, inner) => {
            f(v);
            visit_constant_vars(inner, f)
        }

        // No variable of their own — recurse.  The `u16` on `Break`/`Continue` is a
        // loop level, not a variable, so it is left alone.
        Value::Span(b) => visit_constant_vars(&mut b.1, f),
        Value::Call(_, args) | Value::Insert(args) | Value::Tuple(args) | Value::Parallel(args) => {
            args.iter_mut().all(|a| visit_constant_vars(a, f))
        }
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter_mut().all(|o| visit_constant_vars(o, f))
        }
        Value::Return(b) | Value::Drop(b) | Value::Yield(b) => visit_constant_vars(b, f),
        Value::If(c, t, e) => {
            visit_constant_vars(c, f) && visit_constant_vars(t, f) && visit_constant_vars(e, f)
        }
        Value::Null
        | Value::Line(_)
        | Value::Int(_)
        | Value::Enum(..)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::RawExpr(_) => true,

        // These carry a variable too, in a form no constant initialiser takes.  Refuse
        // rather than guess at the rewrite.
        Value::CallRef(..)
        | Value::Iter(..)
        | Value::Keys(_)
        | Value::TupleGet(..)
        | Value::TuplePut(..)
        | Value::FnRef(..)
        | Value::FnRefDnr(_) => false,
    }
}

/// Where a stored default is replayed, which is what decides the variables it may name.
///
/// A default is always lowered in one frame and replayed in another, so the two sites
/// differ only in which names survive the crossing — and that is exactly what
/// [`default_replayable_in_place`] has to be told.
#[derive(Clone, Copy)]
pub(crate) enum DefaultSite {
    /// A struct field, replayed by `object_init` inside whatever function builds the
    /// record.  It rewrites `Var(0)` to that record and re-homes the block it builds by
    /// hand, so `Var(0)` survives — but only when the default actually read `$`, since
    /// `Var(0)` is also the first work-ref the struct's empty table hands out.
    Field {
        reads_record: bool,
        /// The field's declared type is a struct, so a default that BUILDS rather than
        /// calls or names is a nested struct literal — and its field writes carry the
        /// NESTED struct's offsets, which `object_init`'s `Var(0)` rewrite then applies
        /// to the record (loft#701).  Read from the declared TYPE, not from the IR:
        /// the shape differs between passes — pass 1 has an offset-less `Insert`, pass 2
        /// an `Object` block — and the verdict may not, because a function that first
        /// appears in pass 2 is exactly what H5 rejects.
        struct_typed: bool,
    },
    /// A function parameter, replayed at every CALL site by `add_defaults`, which
    /// transplants `Var(i)` for each EARLIER parameter `i` into the caller's actual
    /// argument (`substitute_param_refs`).  So those indices survive and nothing else
    /// does — including `Var(count)`, the destination the default was parsed against.
    Parameter { count: u16 },
}

/// loft#698 / loft#699 — can this default be replayed at its replay site AS IT STANDS?
///
/// A default is lowered in a context that has no frame of its own (a struct, or a
/// signature) and replayed in one that does (a construction site, or a call site).  So
/// the question is what variables the default names, and whether the replay site can
/// give each of them a meaning — see [`DefaultSite`] for the two answers.  For a field:
///
/// * a literal (`= 7`, `= "hi"`, `= []`) names no variable at all — always replayable;
/// * `reads_record` says the default read `$`, which IS `Var(0)` — so `= $.x + 1` is
///   replayable, and that is the only thing `Var(0)` may legitimately be;
/// * an `Object` block builds the nested struct IN PLACE through `Var(0)`, so pointing it
///   at the record is exactly right — that is how `= P { px: 1, py: 2 }` works;
/// * an `EnumUnitLit` block is re-homed to a fresh work-ref by `object_init`;
/// * anything else naming a variable wanted its OWN temporary, and the struct's table is
///   discarded before replay, so the index resolves against the construction site's
///   locals.  `= "a" + "b"` used the record as a text buffer (`OpClearText` on the
///   struct — a SIGSEGV), `= [1, 2]` re-allocated it mid-construction (a hang), and
///   `= mk()` handed the callee the RECORD as its return buffer (a hang) — that last one
///   survived an earlier index-based check, because a call's return buffer is the FIRST
///   work-ref the struct's empty table hands out and so lands on `Var(0)`, the very index
///   that check read as "the record, therefore safe".
///
/// A PARAMETER's replay site is a call, which re-homes nothing: the only names it can
/// give a meaning to are the earlier parameters, so a `Block` of any kind and the
/// statement sequence a collection literal builds (`Insert`) both have to move.  That is
/// what `= []` needs — an empty `Insert` names no variable, but it is a build-into-a-
/// destination, not a value, and a call site has no destination to build into.
///
/// Which is why this is a WHITELIST and returns `false` for anything it does not
/// recognise: an unrecognised shape gets lowered into a function of its own, which is
/// always sound, where guessing it replayable is silent corruption.  The match is
/// exhaustive on purpose — a new IR variant has to be classified here rather than
/// inheriting whichever answer happened to be the fallback.
pub(crate) fn default_replayable_in_place(value: &crate::data::Value, site: DefaultSite) -> bool {
    use crate::data::Value;
    let every = |vs: &[Value]| vs.iter().all(|v| default_replayable_in_place(v, site));
    // Is this index a name the replay site can still give a meaning to?
    let legit = |v: u16| match site {
        DefaultSite::Field { reads_record, .. } => v == 0 && reads_record,
        DefaultSite::Parameter { count } => v < count,
    };
    let is_field = matches!(site, DefaultSite::Field { .. });
    // loft#701 — a nested struct literal builds THROUGH the record, at the nested
    // struct's own offsets: `A { x, p: P = P { … } }` wrote `px` over `x`, and with `p`
    // first the writes landed right but the supplied sibling was lost.  Hoisting is what
    // gets the offsets right, because then the CALLER writes the field and applies its
    // own `pos + fld`.  Both build shapes are refused for a struct-typed field, so the
    // two passes agree.
    let builds_struct = matches!(
        site,
        DefaultSite::Field {
            struct_typed: true,
            ..
        }
    );
    match value.unspan() {
        // The block `object_init` re-homes by hand — `EnumUnitLit` goes to a fresh
        // work-ref, so it never writes through the record at all.
        Value::Block(b) => {
            is_field && (b.name == "EnumUnitLit" || (b.name == "Object" && !builds_struct))
        }

        Value::Var(v) => legit(*v),
        Value::Set(v, inner) => legit(*v) && default_replayable_in_place(inner, site),

        // Operands carry the variables; the callee/branch structure itself carries none.
        Value::Call(_, args) => every(args),
        // A statement sequence writes into a destination this frame owns — which a
        // construction site supplies (the record) and a call site does not.
        Value::Insert(items) => is_field && !builds_struct && every(items),
        Value::Tuple(items) | Value::Parallel(items) => every(items),
        Value::If(c, t, e) => {
            default_replayable_in_place(c, site)
                && default_replayable_in_place(t, site)
                && default_replayable_in_place(e, site)
        }
        Value::Return(b) | Value::Drop(b) | Value::Yield(b) => default_replayable_in_place(b, site),

        // Literals name nothing.
        Value::Null
        | Value::Line(_)
        | Value::Int(_)
        | Value::Enum(..)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::RawExpr(_) => true,

        // Each names a variable in a form whose meaning cannot be recovered here, or is
        // not something a default is built from at all.  Lower it into its own function.
        Value::Loop(_)
        | Value::CallRef(..)
        | Value::Iter(..)
        | Value::Keys(_)
        | Value::TupleGet(..)
        | Value::TuplePut(..)
        | Value::FnRef(..)
        | Value::FnRefDnr(_) => false,

        // `unspan` above already peeled the outer wrapper; peel any nested one too.
        Value::Span(b) => default_replayable_in_place(&b.1, site),
    }
}

impl Parser {
    /// A lifetime-bearing tuple return, boxed as the synthetic `__tuple<…>` record.
    ///
    /// The one rule for a declared `-> (a, b)` whose elements carry a lifetime concern — a
    /// text, a record, a collection: the function returns the synthetic struct loft already
    /// registers for stored tuples, so the whole `ref_return` / `text_return` delivery
    /// machinery applies and every element is COPIED out.  A pure-value tuple keeps Rust's
    /// tuple ABI (`has_lifetime_concern` says no for it) and is returned unchanged.
    ///
    /// Named functions took this at their declaration; a LAMBDA declared the same way did
    /// not, so its tail `(q.items, q.nm)` was handed up as the bare tuple its arms yield —
    /// the vector element a view of the argument's field, and the caller's bind aliased it
    /// while the named twin copied (loft#1349, @FR-F-Ret).  Both lambda forms now box
    /// through this, at the same point on both passes.
    pub(crate) fn boxed_tuple_return(&mut self, result: Type) -> Type {
        let Type::Tuple(elems) = &result else {
            return result;
        };
        if !elems.iter().any(crate::data::has_lifetime_concern) {
            return result;
        }
        let elems = elems.clone();
        let synthetic_d_nr = self.data.tuple_def(&mut self.lexer, &elems);
        Type::Reference(synthetic_d_nr, crate::data::Deps::none())
    }
}
