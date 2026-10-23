// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Reading and writing the members of a record and the elements of a vector: field
//! offsets, the tuple and fn-ref field layouts, keyed-member writes, and the per-element set
//! sequence with its constraint check.

use super::{
    DefType, Level, Parser, Parts, Type, Value, diagnostic_format, emit_fn_ref_field_write,
    find_capturing_fn_ref, v_block, v_if, v_set,
};

impl Parser {
    /// The stored offset of `field` within `d_nr`'s layout.
    ///
    /// loft#796 — [`Stores::position`](crate::database::Stores::position) answers
    /// `u16::MAX` for a field the layout does not have, and that answer used to flow
    /// straight out as the field OFFSET: the interpreter then read and WROTE at
    /// `record + 65535`. A struct whose declaration carries a field its layout does
    /// not is not something any code can access correctly, so every lookup goes
    /// through here and says so instead of emitting an access outside the record.
    ///
    /// The gap opens when the field's TYPE is not declared yet at the moment the
    /// struct is laid out — a type from a module the package loads later. A second
    /// pass resolves the ATTRIBUTE, so the declaration looks complete while the layout
    /// keeps the hole (loft#797 tracks closing that; this is the guard, not the cure).
    ///
    /// It was silent: in the reported case a ten-field struct was laid out with nine,
    /// and every write to the missing one corrupted whatever followed the record.
    /// Usually a SIGSEGV inside a later claim walk, sometimes an unbounded allocation,
    /// and non-deterministic either way, because WHICH it is depends on what the
    /// neighbouring bytes happened to hold — which is why the same program read as
    /// "random", "non-monotonic in scene size", and "fine as a program, crashes under
    /// the test runner".
    ///
    /// Pass 2 only: pass 1 has no layouts yet, so the sentinel there is ordinary and
    /// says nothing.  Quiet once the parser has reported an error, for the reason
    /// `parse_file` gives `validate_all_layouts`: a program that already failed can hold
    /// unlaid types as a CONSEQUENCE, and the hole is then noise on top of the real
    /// diagnostic.  `v = [Nope { n: 1 }]` reported the undefined type, then a missing
    /// field on the synthetic `main_vector<never>` the compiler built for it — a type the
    /// author never wrote.  Nothing is lost by staying quiet: the compile is already
    /// aborting, and once the first error is fixed a genuine hole reports on the next run.
    pub(crate) fn field_position(&mut self, d_nr: u32, field: &str) -> u16 {
        let p = self
            .database
            .position(self.data.def(d_nr).known_type(), field);
        let already_failed = matches!(
            self.lexer.diagnostics().level(),
            Level::Error | Level::Fatal
        );
        if p == u16::MAX && !self.first_pass && !already_failed {
            let owner = self.data.def(d_nr).name().to_string();
            let unresolved = {
                let a_nr = self.data.attr(d_nr, field);
                a_nr != usize::MAX && self.data.attr_type(d_nr, a_nr).is_unknown()
            };
            if unresolved {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "the type of field '{field}' of '{owner}' never resolved, so the field \
                     has no storage — declare that type, or `use` the module that declares it"
                );
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "field '{field}' of '{owner}' has no storage in that type's layout — \
                     the declaration and the layout disagree, so the field cannot be read \
                     or written"
                );
            }
        }
        p
    }

    pub(super) fn get_field(&mut self, d_nr: u32, f_nr: usize, code: Value) -> Value {
        // @PLN196 step 9 — while a field default is parsed, the path of every stored field it
        // reads through `$` (`inner.label`), so `loft layout accept` can record where a
        // computed field's value comes from (`@FR-L-Upgrade-Source`).
        let base = if self.init_field_tracking && f_nr != usize::MAX {
            if code == Value::Var(0) {
                Some(String::new())
            } else {
                // The receiver may wrap the read it continues (a conversion, a null guard).
                self.default_path_reads
                    .iter()
                    .rev()
                    .find(|(v, _, of)| *of == d_nr && Self::holds_node(&code, v))
                    .map(|(_, p, _)| p.clone())
            }
        } else {
            None
        };
        let read = self.get_field_read(d_nr, f_nr, code);
        if let Some(base) = base {
            let name = self.data.attr_name(d_nr, f_nr);
            let path = if base.is_empty() {
                name
            } else {
                format!("{base}.{name}")
            };
            if !self.default_paths.contains(&path) {
                self.default_paths.push(path.clone());
            }
            // The struct the read's value is, so only a read OF it extends the path.
            let of = match self.data.attr_type(d_nr, f_nr).base() {
                Type::Reference(d, _) => *d,
                _ => u32::MAX,
            };
            self.default_path_reads.push((read.clone(), path, of));
        }
        read
    }

    /// Is `needle` `v` or a node anywhere inside it?
    fn holds_node(v: &Value, needle: &Value) -> bool {
        if v == needle {
            return true;
        }
        let mut found = false;
        v.for_each_child(&mut |c| found |= !found && Self::holds_node(c, needle));
        found
    }

    fn get_field_read(&mut self, d_nr: u32, f_nr: usize, code: Value) -> Value {
        // @PLN165 D5 — a field of an OPEN instance has no position until a monomorph names
        // the instance; the read is deferred to it.  So is one a generic struct's OWN
        // declaration reads (a field `assert`, a default over `$`), bound to each instance
        // where that code is replayed (`bind_instance_code`).
        if f_nr != usize::MAX
            && (self.data.is_open_instance(d_nr)
                || self.data.def_type(d_nr) == DefType::TypeTemplate)
        {
            let tp = self.data.attr_type(d_nr, f_nr);
            self.expr_not_null =
                !self.data.attr_nullable(d_nr, f_nr) && !tp.non_null_slot_reads_null();
            self.expr_not_null_name.clear();
            return v_block(
                vec![
                    Value::Int(d_nr as i32),
                    Value::Int(i32::try_from(f_nr).unwrap_or(i32::MAX)),
                    code,
                ],
                tp,
                Self::TV_FIELD,
            );
        }
        // #91: track $.<field> accesses during init(expr) parsing.
        if self.init_field_tracking && code == Value::Var(0) && f_nr != usize::MAX {
            let name = self.data.attr_name(d_nr, f_nr);
            if !self.init_field_deps.contains(&name) {
                self.init_field_deps.push(name);
            }
        }
        let tp = self.data.attr_type(d_nr, f_nr);
        let nullable = self.data.attr_nullable(d_nr, f_nr);
        // Not declared `?` is not the same as never null: a field of an in-band kind holds the
        // null `(N-Store)` lets into it, so the "never null" lints ask the slot's TYPE too.
        self.expr_not_null = !nullable && !tp.non_null_slot_reads_null();
        if self.expr_not_null && f_nr != usize::MAX {
            self.expr_not_null_name = self.data.attr_name(d_nr, f_nr);
        } else {
            self.expr_not_null_name.clear();
        }
        let pos = if f_nr == usize::MAX {
            0
        } else {
            let nm = self.data.attr_name(d_nr, f_nr);
            self.field_position(d_nr, &nm)
        };
        // Post-2c: pass the field's alias def_nr so `get_val` can honor
        // size(N) for integer subtypes (e.g. i32 → OpGetInt4).
        let alias = if f_nr == usize::MAX {
            u32::MAX
        } else {
            self.data.def(d_nr).attributes()[f_nr].alias_d_nr
        };
        // P215: for fn-ref fields with the legacy 4B int layout, the
        // database has no `<attr>__closure_rec` half — reading at
        // pos+4 would corrupt the next attribute.  Synthesise the
        // null DbRef sentinel for the closure half instead.  The 8B
        // split layout (assigned by a capturing lambda) keeps the
        // existing dual-read path.  The split/legacy answer comes
        // from `fn_ref_field_is_split` (the database layout), NOT
        // from `assigned_lambda_d_nr` directly — the flag is only
        // set when the assigning body parses, so a body parsed
        // earlier would wrongly see the legacy layout (#313).
        if let Type::Function(..) = &tp
            && f_nr != usize::MAX
        {
            // Remember WHICH attribute this read came from, for an assignment through it
            // (loft#1072). The read carries a byte offset, and on pass 1 that offset is
            // `u16::MAX` for every field of a struct whose layout does not exist yet.
            self.fn_ref_read_attr = Some((d_nr, f_nr));
            // The pair is TWO reads of one record, so a record a call returns is bound once
            // and both halves read the binding: spliced into each read, `mk().f(21)` ran `mk`
            // twice on `--native`, and the call lifted out of the pair broke the pair's own
            // shape there (E0308, a `DbRef` where `(u32, DbRef)` belongs).
            let (code, bind) = if matches!(code.unspan(), Value::Call(c, _)
                if !self.data.def(*c).name().starts_with("Op"))
            {
                let host = Type::Reference(d_nr, crate::data::Deps::none());
                let w = self.vars.work_refs(&host, &mut self.lexer);
                if !self.first_pass {
                    self.change_var_type(w, &host);
                }
                (Value::Var(w), Some(v_set(w, code)))
            } else {
                (code, None)
            };
            // @PLN114 — the layout decides the reader, and BOTH answers are now
            // explicit: a split field reads its closure_rec child, a legacy one
            // synthesises a NULL closure.  `get_val`'s Function arm is the legacy
            // read (tuple / vector elements), so a split field must not fall
            // through to it.
            let read = if self.fn_ref_field_is_split(d_nr, f_nr) {
                self.read_fn_ref_split(&tp, u32::from(pos), code)
            } else {
                // loft#1869 — a closure record's fn-ref capture reads its closure half from the
                // `<name>__clos` capture beside it.
                let clos_pos = self.database.position(
                    self.data.def(d_nr).known_type(),
                    &format!("{}__clos", self.data.attr_name(d_nr, f_nr)),
                );
                let read_clos = if clos_pos == u16::MAX || self.first_pass {
                    self.cl("OpNullRefSentinel", &[])
                } else {
                    self.cl(
                        "OpGetDbRef",
                        &[code.clone(), Value::Int(i32::from(clos_pos))],
                    )
                };
                let read_dnr = self.cl("OpGetInt4", &[code, Value::Int(i32::from(pos))]);
                crate::data::v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
            };
            return match bind {
                Some(bind) => v_block(vec![bind, read], tp, "fn_ref_field_base"),
                None => read,
            };
        }
        self.get_val(&tp, nullable, u32::from(pos), code, alias)
    }

    /// Is the fn-ref struct field `d_nr.f_nr` stored in the split 8B
    /// layout (`<attr>` d_nr + `<attr>__closure_rec`) rather than the
    /// legacy 4B int layout?
    ///
    /// In the second pass the database layout is the answer's one
    /// stable home: it was built from the COMPLETE first pass, while
    /// `assigned_lambda_d_nr` is derived during body parsing — a body
    /// parsed before the assigning body cannot trust the flag and
    /// would bake in the wrong field shape (#313).  Deriving from the
    /// layout keeps the read/write shape in lockstep with the bytes
    /// actually laid out.  In the first pass the layout does not
    /// exist yet, so fall back to the flag; the first pass's IR is
    /// discarded and only the flag's end state (consumed by
    /// `typedef::fill_database`) matters.
    pub(super) fn fn_ref_field_is_split(&self, d_nr: u32, f_nr: usize) -> bool {
        if self.first_pass {
            self.data.def(d_nr).attributes()[f_nr].assigned_lambda_d_nr != u32::MAX
        } else {
            let nm = self.data.attr_name(d_nr, f_nr);
            let kt = self.data.def(d_nr).known_type();
            self.database.position(kt, &format!("{nm}__closure_rec")) != u16::MAX
        }
    }

    /// @PLN114 — read a fn-ref struct field stored in the SPLIT layout: 4B `d_nr`
    /// at `pos` plus the `<attr>__closure_rec` child-record at `pos + 4`, which
    /// `typedef.rs` registers only when a capturing lambda was assigned to the
    /// attribute.  Tuple and vector elements never have that second field — they
    /// read through [`Self::get_val`]'s legacy arm instead.
    pub(super) fn read_fn_ref_split(&mut self, tp: &Type, pos: u32, code: Value) -> Value {
        let p = Value::Int(pos as i32);
        let read_dnr = self.cl("OpGetInt4", &[code.clone(), p]);
        let crec_field = self.cl(
            "OpGetField",
            &[code, Value::Int(pos as i32 + 4), Value::Int(0)],
        );
        let read_clos = self.cl("OpRefFromChildRec", &[crec_field]);
        crate::data::v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
    }

    /// A struct field of tuple type, read member by member at the synthetic `__tuple<…>`
    /// record's own offsets — [`Self::get_val`]'s tuple arm.
    pub(super) fn get_tuple_field(
        &mut self,
        tp: &Type,
        elems: &[Type],
        pos: u32,
        code: &Value,
    ) -> Value {
        let p = Value::Int(pos as i32);
        // Plan-06 phase 4d: tuple struct field read.  Each
        // element is read from `pos + element_stack_offsets[i]`
        // using the same OpGet* opcodes that ordinary struct
        // fields use; the assembled stack tuple matches the
        // shape `Type::Tuple(...)` consumers expect.
        let elems_vec = elems.to_vec();
        let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
        // `@FR-T-Ref-Src` — while a `&` link parses, the field is read WITH its address:
        // a `tuple_field_place` block holding the field's `OpGetField` beside the
        // member-wise read, which `stored_tuple_dest` peels for the link (loft#1883).
        // The same value, and no work-ref, so both passes allocate alike.
        let place = (self.amp_pending || self.tuple_place_wanted)
            && tuple_d_nr != u32::MAX
            && crate::data::ref_tuple_is_record(&elems_vec);
        let field = place.then(|| {
            let kt = i32::from(self.data.def(tuple_d_nr).known_type());
            self.cl("OpGetField", &[code.clone(), p.clone(), Value::Int(kt)])
        });
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
        let mut tuple_elems = Vec::with_capacity(elems_vec.len());
        for (i, et) in elems_vec.iter().enumerate() {
            let elem_pos = pos + u32::from(offsets[i]);
            // @PLN25 — a member declared `S?` is stored behind its tag, so its bytes
            // here are the discriminant followed by the payload.
            if let Some(tagged) = self.tuple_elem_tag_read(tuple_d_nr, i, code, elem_pos, et) {
                tuple_elems.push(tagged);
                continue;
            }
            // loft#1503 — read the member in the spelling it was STORED under, not
            // in the caller's, whose inferred deps would pick the `OpGetDbRef` arm
            // against bytes the def laid out inline.
            let et = &crate::data::Data::tuple_member_stored(et);
            let nullable = matches!(et, Type::Optional(_));
            let elem_val = self.get_val(et, nullable, elem_pos, code.clone(), u32::MAX);
            tuple_elems.push(elem_val);
        }
        if let Some(field) = field {
            return crate::data::v_block(
                vec![Value::Drop(Box::new(field)), Value::Tuple(tuple_elems)],
                tp.clone(),
                Self::TUPLE_FIELD_PLACE,
            );
        }
        Value::Tuple(tuple_elems)
    }

    pub(super) fn get_val(
        &mut self,
        tp: &Type,
        nullable: bool,
        pos: u32,
        code: Value,
        alias: u32,
    ) -> Value {
        let p = Value::Int(pos as i32);
        match tp {
            // @PLN25 slice (b): an `Optional(τ)` field shares its base's sentinel storage —
            // read it exactly as `τ` (the marker is compile-time only).
            Type::Optional(inner) => self.get_val(inner, nullable, pos, code, alias),
            Type::Integer(spec) => {
                // Narrow-integer width selection — ONE derivation.  A struct-field
                // read carries the `alias` whose `size(N)` annotation (if any) wins;
                // everything else — a vector element above all — takes the width from
                // `byte_width`, which honours the spec's own `forced_size` first and
                // falls back to the value RANGE.  loft#1036: the vector-element case
                // used to ask `vector_narrow_width`, keyed on `forced_size` alone, so
                // it answered 8 for the bare `limit(lo, hi)` spelling of a range the
                // alias spelling stored in one byte.
                //
                // `narrow_vec` does NOT pick the width: it picks the raw-vs-full
                // ENCODING for a 2-byte element (`ShortRaw` for a `u16`-style alias,
                // `ShortFull` for a range that merely fits), which is why it keeps the
                // `forced_size` gate the width no longer has.
                let narrow_vec = alias == u32::MAX
                    && spec.forced_size.is_some()
                    && spec.vector_narrow_width(nullable).is_some();
                let s = self
                    .data
                    .forced_size(alias)
                    .unwrap_or_else(|| spec.byte_width(nullable));
                debug_assert!(
                    matches!(s, 1 | 2 | 4 | 8),
                    "get_val: unexpected integer field width s={s} \
                     (alias_d_nr={alias}) — only 1/2/4/8 are supported \
                     by the OpGet* family"
                );
                // H4-medium: the op KIND comes from the ONE width→op home
                // (`NarrowIntKind::of`), so this READ op and the matching WRITE
                // op in `set_field_check` cannot drift.  A nullable byte STRUCT
                // FIELD decodes the reserved 256th code to null (`ByteNullable`);
                // a narrow-vector element keeps the raw direct encoding (its
                // stride/value contract is the narrow-vector one, not the
                // field-sentinel one) — `narrow_vec` selects that.
                // loft#1615: `NarrowSlot` is the one home for a store place's ops, so a
                // NON-nullable type that kept a top code reads that code as null (@C85) here
                // exactly as a LOCAL of the type does — `kind.get_op()` alone decodes every
                // code as a value, which read the overflow's `u16::MAX` back as `64535`.
                let slot = crate::data::NarrowSlot::of_slot(s, nullable, narrow_vec, spec);
                let kind = slot.kind;
                if kind.takes_min() {
                    // H6: a sentinel-reserving kind (`ByteNullable`/`Short` — a
                    // nullable narrow FIELD *or* vector element) shrinks its usable
                    // range by one edge, so the read decodes against `usable_min`;
                    // raw kinds keep the full `min`.  Deriving from the KIND (not a
                    // re-computed `nullable && !narrow_vec`) keeps this in lockstep
                    // with the write op's `min`.
                    self.cl(slot.get_op(), &[code, p, Value::Int(slot.min)])
                } else {
                    self.cl(slot.get_op(), &[code, p])
                }
            }
            Type::Enum(_, false, _) => self.cl("OpGetEnum", &[code, p]),
            // @PLN17: byte-stored boolean read, preserving 0/1/255 (like enum).
            Type::Boolean => self.cl("OpGetBoolean", &[code, p]),
            Type::Float => self.cl("OpGetFloat", &[code, p]),
            Type::Single => self.cl("OpGetSingle", &[code, p]),
            // A `vector<character>` element read had no `get_val` arm (only the
            // write side — OpSetCharacter — existed), so `v[0]` / `for c in v`
            // fell through to "Field access not supported on type character".
            // Mirror the OpSetCharacter write with the OpGetCharacter read.
            Type::Character => self.cl("OpGetCharacter", &[code, p]),
            Type::Text(_) => self.cl("OpGetText", &[code, p]),
            Type::Hash(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _)
            | Type::Vector(_, _) => {
                let info = self.type_info(tp);
                self.cl("OpGetField", &[code, p, info])
            }
            Type::Reference(_, deps) => {
                if deps.is_empty() {
                    // Inline struct field: OpGetField adds the field offset to the base ref.
                    // Linked/base type dereference is handled at the call site (fields.rs)
                    // using OpVectorRef, which combines the 4-byte pointer read + deref.
                    let info = self.type_info(tp);
                    self.cl("OpGetField", &[code, p, info])
                } else {
                    // Plan-22 phase 02b (2026-05-12): auto-Reference field
                    // — the field stores a 12-byte DbRef pointing at the
                    // source record (shared storage).  Read the full
                    // DbRef back via OpGetDbRef.  Phase 02c is the
                    // producer that gives a closure-record attribute
                    // non-empty deps; user struct fields always have
                    // empty deps (legacy inline-bytes path above).
                    self.cl("OpGetDbRef", &[code, p])
                }
            }
            Type::Function(..) => {
                // P213: storage is two database fields per loft attribute
                //   `<attr>`              — 4B i32 holding the lambda's d_nr
                //   `<attr>__closure_rec` — 4B vector header at pos+4
                //                            (empty = non-capturing /
                //                            default-init; populated =
                //                            capturing closure record
                //                            co-located in host's Store).
                // Read both halves: `OpGetInt4` pushes the 8B d_nr,
                // `OpVectorFirstOrNull` pushes the 12B closure DbRef
                // (or the null sentinel when the vector is empty).
                // Together they form the 20B stack-side fn-ref slot
                // shape `fn_call_ref` already consumes unchanged.
                // @PLN114 — this is the LEGACY single-field read: 4B d_nr at `pos`
                // and a synthesised NULL closure.  `get_val` is reached for TUPLE and
                // VECTOR elements, which `typedef.rs`'s `Type::Function` arm keeps on
                // the legacy layout by design ("closure_rec field would be wasted
                // space and breaks layouts of containers (tuples) that pre-computed
                // positions assuming 4B per fn-ref slot").
                //
                // Reading a closure_rec at `pos + 4` here read a field that does not
                // exist for those elements — harmless only while alignment padding
                // sat there, and a collision with the NEXT element once tuples pack
                // tight like records.  The SPLIT layout (a capturing lambda assigned
                // to a struct field) has its own reader, `read_fn_ref_split`, called
                // from the site that knows the field is split.
                let read_dnr = self.cl("OpGetInt4", &[code, p.clone()]);
                let read_clos = self.cl("OpNullRefSentinel", &[]);
                crate::data::v_block(vec![read_dnr, read_clos], tp.clone(), "fn_ref_field_read")
            }
            Type::Tuple(elems) => self.get_tuple_field(tp, elems, pos, &code),
            // Pass-1 deferral: reading a field whose declared type is still
            // `Unknown` (a forward-referenced or cross-package field type, e.g.
            // `struct Box { inner: Cell }` parsed above `struct Cell`) must not
            // emit — `actual_types_deferred` resolves the field type after this
            // pass, and pass-2 re-reads it with the concrete type.  Emitting
            // here would (via the pass-2 gate) suppress that resolution.  In
            // pass-2 a still-`Unknown` type is genuinely undefined and its
            // "Undefined type" error already fires from type resolution, so
            // fall through to the diagnostic only there.
            Type::Unknown(_) if self.first_pass => Value::Null,
            // A generator handle (loft#1585) is stored as the 12-byte `DbRef` it is.
            Type::Iterator(_, _) => self.cl("OpGetDbRef", &[code, p]),
            _ => {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Field access not supported on type {}",
                    tp.source_name(&self.data)
                );
                Value::Null
            }
        }
    }

    /// Write into a vector ELEMENT slot whose declared type is `elm_tp`.
    ///
    /// @FR-H-Stride — the element's width is the declared TYPE's, and the def it resolves to
    /// cannot answer it, so the type is handed to the write path rather than re-derived from
    /// `d_nr` (loft#1420).  The read half is `read_slice_elem`.
    pub(super) fn set_element(
        &mut self,
        elm_tp: &Type,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        let d_nr = self.data.type_def_nr(elm_tp);
        self.set_field_check(
            d_nr,
            usize::MAX,
            d_pos,
            ref_code,
            val_code,
            true,
            Some(elm_tp),
        )
    }

    pub(super) fn set_field(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        self.set_field_check(d_nr, f_nr, d_pos, ref_code, val_code, true, None)
    }

    /// @PLN130 F4 — writing a KEY field through an element view re-keys the element, and a
    /// keyed collection does not re-index itself when a record is written behind its back.
    /// Measured on `sorted`, `hash` AND `index` alike: after `c.key = 5` the element updates,
    /// `s[5]` answers null, `s[30]` answers null, and iteration still yields it — reachable by
    /// no key while still in the collection.
    ///
    /// Treat it as a RESHAPE of that collection, which is exactly F2's situation: the view
    /// materialises, the author is told, `c.key = 5` changes only `c`, and the collection
    /// keeps a consistent reachable entry. Changing a key keeps its supported spelling —
    /// `h[k] = value`, which routes through `set_keyed` and re-indexes properly. That is how
    /// a database treats a primary key.
    ///
    /// Hooked at the getter->setter conversion (`call_to_set_op`), which is where a plain
    /// `c.key = 5` actually lands — `set_field` looked like the site and is not on this path.
    /// That leaves only the byte OFFSET (`OpSetInt(c, 0, 5)`), so each key NAME is resolved to
    /// its offset through `self.database.types[…].parts` and compared. The parser holds a
    /// `Stores`; `scopes::check` does not, which is why this cannot live there.
    ///
    /// Deliberately narrow: only a field that IS one of the collection's keys triggers it.
    /// Widening to any field write through an element view would take write-through away from
    /// keyed elements generally, which is a far larger loss than F4 is worth.
    /// Byte offset of the named field within `content`'s record, via the runtime type table.
    ///
    /// `None` when the type is not laid out yet or the name is not a field — a missed answer
    /// just leaves the binding alone, which is today's behaviour rather than a new one.
    pub(super) fn key_field_offset(&self, content: u32, field: &str) -> Option<i64> {
        let idx = self
            .data
            .def(content)
            .attributes()
            .iter()
            .position(|a| a.name == field)?;
        let kt = self.data.def(content).known_type();
        let parts = &self.database.types.get(kt as usize)?.parts;
        let fields = match parts {
            Parts::Struct(f) | Parts::EnumValue(_, f) => f,
            _ => return None,
        };
        let pos = fields.get(idx)?.position;
        if pos == u16::MAX {
            return None;
        }
        Some(i64::from(pos))
    }

    pub(crate) fn note_key_field_write(&mut self, base: u16, offset: i64) {
        // Through `base()`: a keyed element view is `Reference(E)` when the lookup types
        // non-null and `Optional(Reference(E))` once `@FR-Col-Lookup` gives the lookup its `?`
        // (loft#1450) — one notion, two spellings, and reading only the first skipped this
        // whole analysis for every keyed binding.  Measured: `c = s[30]; c.key = 5` then left
        // `s[30]` reachable by NO key and said nothing, which is the exact @PLN130 F4 defect
        // this function exists to prevent, silently reintroduced by a nullability marker.
        // `depend()` below is already `Optional`-transparent, so the deps arrive either way —
        // it was only the SHAPE test that could not see through the `?`.
        if !matches!(self.vars.tp(base).base(), Type::Reference(_, _)) {
            return;
        }
        let deps: Vec<u16> = self.vars.tp(base).depend().clone();
        for dep in deps {
            if dep == u16::MAX || dep == base {
                continue;
            }
            // Through `base()`: the container a view depends on may itself be `τ?` — a
            // `sorted<E[k]>?` field or local is `Optional(Sorted(…))`, and reading only the
            // bare spelling dropped every such container to `_ => continue`.  A key write
            // through a view of a NULLABLE keyed collection was then allowed in silence: the
            // record was re-keyed, the collection never re-indexed, the element left reachable
            // by no key, and the author told nothing — @PLN130 F4's defect, alive again behind
            // one nullability marker.
            let (content, key_names): (u32, Vec<String>) = match self.vars.tp(dep).base() {
                Type::Sorted(c, keys, _) | Type::Index(c, keys, _) => {
                    (*c, keys.iter().map(|(k, _)| k.clone()).collect())
                }
                Type::Trie(c, key, _) => (*c, vec![key.clone()]),
                Type::Hash(c, keys, _) | Type::Radix(c, keys, _) => (*c, keys.clone()),
                _ => continue,
            };
            let Some(field) = key_names
                .into_iter()
                .find(|k| self.key_field_offset(content, k) == Some(offset))
            else {
                continue;
            };
            let vname = self.vars.name(base).to_string();
            let cname = self.vars.name(dep).to_string();
            // @PLN130 F9 — an explicit `&` is an ownership decision, so it may not be quietly
            // downgraded to a copy: where the write cannot reach the source, REFUSE it
            // (`@FR-B-Ref-Reshape`, @C79).  Unlike the removal
            // and reassignment causes there is no liveness question here — the key write IS the
            // use — and no "move it later" remedy either, so the message points at re-inserting.
            if self.vars.is_amp_link(base) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot write the key field `{field}` through `{vname}` — `{field}` is one \
                     of `{cname}`'s keys, and changing it would leave the element reachable by \
                     no key, so the write cannot reach `{cname}`. Re-insert with \
                     `{cname}[key] = value`, or bind without `&` to work on a copy"
                );
                continue;
            }
            self.vars.make_independent(base, dep);
            let fname = self.data.def(self.context).original_name();
            crate::copy_manifest::note_rekeyed_view(&vname, &cname, &field, &fname);
        }
    }

    /// Plan-06 phase 4d: emit the per-element OpSet* sequence for a
    /// tuple struct field's value.  Recurses for nested Tuple element
    /// types so `((1, 2), (3, 4))` written into a `((int, int), (int,
    /// int))` field flattens to four `OpSetInt` calls at offsets
    /// `[0, 8, 16, 24]`.
    ///
    /// `base_pos` is the byte offset within the host record where the
    /// tuple's first element begins.  `elems` lists the tuple's
    /// element types in declaration order.  `val_code` is the IR for
    /// the tuple value.
    ///
    /// Two flavours of `val_code`:
    /// - `Value::Tuple([…])` literal — walked element-wise without a
    ///   temp variable, allowing direct recursion for nested tuples.
    /// - Anything else (variable, function call, etc.) — stashed in a
    ///   work-ref tuple temp first, then read element-by-element via
    ///   `Value::TupleGet`.  Nested tuple elements in this branch
    ///   require codegen support for `TupleGet` on `Type::Tuple`
    ///   (see `state/codegen.rs::Value::TupleGet` stack-tuple arm).
    pub(crate) fn emit_tuple_set_ops(
        &mut self,
        ref_code: &Value,
        base_pos: u16,
        elems: &[Type],
        val_code: Value,
    ) -> Vec<Value> {
        let elems_vec = elems.to_vec();
        let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
        // A tuple of a type variable has no member offsets yet: each monomorph writes the
        // tuple it closes to, member by member (loft#1868).
        if tuple_d_nr != u32::MAX && self.data.is_open_instance(tuple_d_nr) {
            let mut ops = vec![ref_code.clone(), val_code];
            if base_pos != 0 {
                ops.push(Value::Int(i32::from(base_pos)));
            }
            return vec![v_block(ops, Type::Tuple(elems_vec), Self::TV_TUPLE_ELEM)];
        }
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
        if let Value::Tuple(values) = val_code {
            // Literal tuple value — recurse element-wise so nested
            // tuple elements flatten into per-leaf OpSet* calls.
            let mut ops = Vec::new();
            for (i, (elem_tp, value_i)) in elems_vec.iter().zip(values).enumerate() {
                // First-pass `base_pos` can be the `database.position`
                // u16::MAX sentinel for not-yet-resolved fields.  The
                // raw `+` panics under dev-profile overflow checks
                // (caught by `make iter`); release silently wraps.
                // The IR built during pass 1 is regenerated in pass 2,
                // so a saturating placeholder is safe and keeps the
                // arithmetic well-defined across both profiles.
                let elem_pos = base_pos.saturating_add(offsets[i]);
                if let Some(tagged) =
                    self.tuple_elem_tag_write(tuple_d_nr, i, ref_code, elem_pos, elem_tp, &value_i)
                {
                    ops.extend(tagged);
                    continue;
                }
                ops.extend(self.emit_set_one_element(ref_code, elem_pos, elem_tp, value_i));
            }
            return ops;
        }
        // PLAN51 V-b — when the source is a Call returning the
        // heap-promoted form of THIS tuple shape (i.e.
        // `Type::Reference(__tuple<…>, _)` whose def_nr matches
        // tuple_d_nr we just resolved), the native codegen would
        // otherwise stash a `DbRef` return into a `Type::Tuple` work-ref
        // and emit an illegal Rust `as (DbRef, DbRef)` cast at
        // `src/generation/dispatch.rs:459-468` (rustc E0605).  Skip
        // the per-element TupleGet stash and emit a SINGLE
        // OpCopyRecord that deep-copies the entire inner-tuple struct
        // into the host field at `base_pos`.  Mirrors the
        // `Type::Reference(_, deps.is_empty())` arm in
        // `set_field_check` (line 3202-3216).
        //
        // The question is the SOURCE's representation, so it is read off the source's TYPE —
        // a call's return, a variable's, a block's result — not off one node shape.  Asked of
        // a call alone, a VARIABLE holding the stored form took the per-element path and its
        // 12-byte reference was read as the tuple's members: `[for x in v { x }]` over a
        // `vector<(integer, integer)>` (the loop variable IS a stored tuple, a record of the
        // vector) answered garbage on `--interpret`, and a text member crashed it; `--native`
        // did not compile either.
        let promoted_src_def: Option<u32> = if self.first_pass {
            None
        } else {
            let src_tp = match val_code.unspan() {
                Value::Call(d_nr, _) => Some(self.data.def(*d_nr).returned().clone()),
                Value::Var(v) => Some(self.vars.tp(*v).clone()),
                Value::Block(bl) => Some(bl.result.clone()),
                _ => None,
            };
            // `.base()` — a nullable stored tuple is the same record (`@FR-N-Shape`).
            match src_tp.as_ref().map(Type::base) {
                Some(Type::Reference(d, _)) if *d == tuple_d_nr => Some(tuple_d_nr),
                _ => None,
            }
        };
        if let Some(inner_d) = promoted_src_def {
            let inner_kt = i32::from(self.data.def(inner_d).known_type());
            let field_ref = self.cl(
                "OpGetField",
                &[
                    ref_code.clone(),
                    Value::Int(i32::from(base_pos)),
                    Value::Int(inner_kt),
                ],
            );
            return vec![self.cl("OpCopyRecord", &[val_code, field_ref, Value::Int(inner_kt)])];
        }
        // Non-literal source: stash to a work-ref Tuple local, then
        // read each element via `Value::TupleGet`.  A tuple VARIABLE is read in place: it
        // needs no second evaluation guarded against, and each member copy then names the
        // variable itself as its source, so a copy that hands a member's release over
        // (`scopes::copy_hands_off`) stops THAT variable's release.  Through a stash it
        // stopped only the stash, and the variable released the member a second time.
        let (tmp, mut ops) = match val_code.unspan() {
            Value::Var(v)
                if *v < self.vars.count() && matches!(self.vars.tp(*v).base(), Type::Tuple(_)) =>
            {
                (*v, Vec::new())
            }
            _ => {
                let tup_tp = Type::Tuple(elems_vec.clone());
                let tmp = self.vars.work_refs(&tup_tp, &mut self.lexer);
                if !self.first_pass {
                    self.change_var_type(tmp, &tup_tp);
                }
                (tmp, vec![v_set(tmp, val_code)])
            }
        };
        for (i, elem_tp) in elems_vec.iter().enumerate() {
            let elem_pos = base_pos.saturating_add(offsets[i]);
            let elem_val = Value::TupleGet(tmp, i as u16);
            if let Some(tagged) =
                self.tuple_elem_tag_write(tuple_d_nr, i, ref_code, elem_pos, elem_tp, &elem_val)
            {
                ops.extend(tagged);
                continue;
            }
            ops.extend(self.emit_set_one_element(ref_code, elem_pos, elem_tp, elem_val));
        }
        ops
    }

    /// The tagged READ for tuple element `i`, or `None` when that element is not a tagged
    /// slot — the mirror of [`Self::tuple_elem_tag_write`], and every stored-tuple reader
    /// goes through it so a slot cannot be written by one spelling and read by the other.
    ///
    /// `elem_pos` is the slot's byte offset within `ref_code`'s record.
    pub(crate) fn tuple_elem_tag_read(
        &mut self,
        tuple_d_nr: u32,
        i: usize,
        ref_code: &Value,
        elem_pos: u32,
        elem_tp: &Type,
    ) -> Option<Value> {
        if self.first_pass {
            return None;
        }
        let Type::Enum(syn, true, _) = self.data.attr_type(tuple_d_nr, i) else {
            return None;
        };
        if !self.needs_nullable_wrap(syn, elem_tp) {
            return None;
        }
        let enum_kt = i32::from(self.data.def(syn).known_type());
        let slot = self.cl(
            "OpGetField",
            &[
                ref_code.clone(),
                Value::Int(elem_pos as i32),
                Value::Int(enum_kt),
            ],
        );
        let stored = self.data.attr_type(tuple_d_nr, i);
        Some(self.emit_nullable_slot_read(syn, slot, &stored))
    }

    /// The tagged write for tuple element `i`, or `None` when that element is not a tagged
    /// slot (every non-nullable member, and a source already in the slot's own form).
    ///
    /// The member's SOURCE type is the spelling the author writes (`S?`) while the element's
    /// STORED type is the tagged `__nullable<S>` the layout uses; both are needed to decide,
    /// and this is the one place in the tuple writer that holds both.  See
    /// [`Self::emit_nullable_slot_write`] for what the tag buys.
    pub(super) fn tuple_elem_tag_write(
        &mut self,
        tuple_d_nr: u32,
        i: usize,
        ref_code: &Value,
        elem_pos: u16,
        elem_tp: &Type,
        value: &Value,
    ) -> Option<Vec<Value>> {
        if self.first_pass {
            return None;
        }
        let Type::Enum(syn, true, _) = self.data.attr_type(tuple_d_nr, i) else {
            return None;
        };
        if !self.needs_nullable_wrap(syn, elem_tp) {
            return None;
        }
        let enum_kt = i32::from(self.data.def(syn).known_type());
        let slot = self.cl(
            "OpGetField",
            &[
                ref_code.clone(),
                Value::Int(i32::from(elem_pos)),
                Value::Int(enum_kt),
            ],
        );
        Some(self.emit_nullable_slot_write(syn, &slot, value.clone(), false))
    }

    /// Whether `value` is a plain `vector<…>` — the source a keyed member must FILL from rather
    /// than copy from (loft#1675).  Read off the node that carries the type: a variable, a
    /// tuple member, a call's return, a block's result.  Any other node answers `false`, which
    /// keeps the keyed copy: that is the behaviour before this question was asked, so a shape
    /// this cannot type loses the fix and never gains a wrong fill.
    pub(super) fn value_is_plain_vector(&self, value: &Value) -> bool {
        let tp = match value.unspan() {
            Value::Var(v) if *v < self.vars.count() => self.vars.tp(*v).clone(),
            Value::TupleGet(t, i) if *t < self.vars.count() => match self.vars.tp(*t).base() {
                Type::Tuple(elems) => elems.get(*i as usize).cloned().unwrap_or(Type::Unknown(0)),
                _ => return false,
            },
            Value::Call(d, _) => self.data.def(*d).returned.clone(),
            Value::Block(bl) => bl.result.clone(),
            _ => return false,
        };
        matches!(tp.base(), Type::Vector(..))
    }

    /// Write a whole VALUE into a keyed collection member of a record (a struct field, a
    /// tuple member), replacing what it held.  A keyed source is copied with `OpReplaceKeyed`.
    /// A plain VECTOR source is inserted record by record with `OpFillKeyed`, after the
    /// member is cleared — the pair loft#1159 gave the keyed struct-field ASSIGNMENT, and
    /// what `(T-Cons)` and `(F-Ret)` owe a tuple member: `OpReplaceKeyed` walks its source
    /// under the destination's type, so a vector read as a `hash` answered empty, an `index`
    /// kept one record and a `sorted` searched unsorted storage (loft#1675).  The member is
    /// the collection the fill names (`field_nr == u16::MAX`): a tuple member belongs to no
    /// linked group, and a struct field that does is written by the group-aware assignment
    /// in `collections.rs`, not here.
    pub(super) fn keyed_member_write(
        &mut self,
        field_ref: Value,
        value: Value,
        kt: u16,
        tp_val: i32,
    ) -> Value {
        if !self.value_is_plain_vector(&value) {
            return self.cl("OpReplaceKeyed", &[value, field_ref, Value::Int(tp_val)]);
        }
        let clear = self.cl(
            "OpClearKeyed",
            &[field_ref.clone(), Value::Int(i32::from(kt))],
        );
        let fill = self.cl(
            "OpFillKeyed",
            &[
                field_ref,
                value,
                Value::Int(tp_val),
                Value::Int(i32::from(kt)),
                Value::Int(i32::from(u16::MAX)),
            ],
        );
        Value::Insert(vec![clear, fill])
    }

    /// Emit the OpSet* (or recursive flatten) for a single tuple
    /// element at a fixed byte offset within the host record.
    /// Returns a vec because nested-tuple elements expand to multiple
    /// per-leaf set ops.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per member type, the exhaustive match"
    )]
    pub(super) fn emit_set_one_element(
        &mut self,
        ref_code: &Value,
        pos: u16,
        elem_tp: &Type,
        value: Value,
    ) -> Vec<Value> {
        let pos_v = Value::Int(i32::from(pos));
        // @PLN25: peel `Optional(τ)` — a nullable tuple element stores via its base's
        // OpSet* (sentinel storage); without this it fell to `_` and was REJECTED with
        // "Tuple struct field cannot contain element of type integer?".
        let single = match elem_tp.base() {
            // The member's slot is the twin of its READ (`unbox_tuple_from_dbref` → `get_val`
            // with the member's own nullability and no alias): the same `NarrowSlot`, so a
            // `u8` member is written in its one byte and a `u8?` member spells absence.  A
            // wide `OpSetInt` here wrote eight bytes into a one-byte member, over the members
            // and fields after it (`@FR-L-Narrow`: a narrow slot is its declared width).
            Type::Integer(spec) => {
                let nullable = matches!(elem_tp, Type::Optional(_));
                let narrow_vec =
                    spec.forced_size.is_some() && spec.vector_narrow_width(nullable).is_some();
                let slot = crate::data::NarrowSlot::of_slot(
                    spec.byte_width(nullable),
                    nullable,
                    narrow_vec,
                    spec,
                );
                if slot.kind.takes_min() {
                    self.cl(
                        slot.set_op(),
                        &[ref_code.clone(), pos_v, Value::Int(slot.min), value],
                    )
                } else {
                    self.cl(slot.set_op(), &[ref_code.clone(), pos_v, value])
                }
            }
            Type::Function(..) => {
                // P196: storage holds the 4-byte i32 d_nr only.  Reduce
                // `Value::FnRef` to its bare `Value::Int(d_nr)` so the
                // OpSetInt4 template body sees an i64 the interpreter
                // pops in 8 bytes; for fn-ref-shaped sources (Var,
                // TupleGet, function call), interpreter `TupleGet`
                // already pushes only the d_nr's 8 bytes via OpVarInt,
                // and `output_call` projects `.0` natively (see the
                // val_is_fn_ref handling in src/generation/calls.rs).
                //
                // P251 fix (2026-05-11): for `Value::Var(v)` where `v`
                // has type `Function`, wrap in `Value::FnRefDnr(v)` so
                // native emit projects `(var_v.0 as i64)` (the d_nr
                // half of the runtime `(u32, DbRef)` tuple).  Without
                // this projection, native codegen emits
                // `let _v_val = (var_v); ... _v_val as i32` which
                // rustc rejects (E0308: expected `(u32, DbRef)`,
                // found `i64`; E0605: non-primitive cast).  The
                // direct fn-ref-struct-field-write path
                // (`emit_fn_ref_field_write`, parser/mod.rs:4886)
                // already does this projection — extending it to the
                // tuple-element-of-struct-field path closes the
                // remaining gap.
                let d_nr_only = match value {
                    Value::FnRef(d_nr, _, tp) => {
                        return vec![self.fn_number_write(ref_code.clone(), pos_v, d_nr, &tp)];
                    }
                    Value::Var(v)
                        if matches!(self.vars.tp(v), Type::Function(..))
                            && self.vars.closure_var_of(v).is_none() =>
                    {
                        Value::FnRefDnr(v)
                    }
                    other => other,
                };
                self.cl("OpSetInt4", &[ref_code.clone(), pos_v, d_nr_only])
            }
            Type::Float => self.cl("OpSetFloat", &[ref_code.clone(), pos_v, value]),
            Type::Single => self.cl("OpSetSingle", &[ref_code.clone(), pos_v, value]),
            Type::Character => self.cl("OpSetCharacter", &[ref_code.clone(), pos_v, value]),
            Type::Boolean => {
                let v = v_if(value, Value::Int(1), Value::Int(0));
                self.cl("OpSetByte", &[ref_code.clone(), pos_v, Value::Int(0), v])
            }
            Type::Text(_) => self.cl("OpSetText", &[ref_code.clone(), pos_v, value]),
            // A plain enum member is its tag byte, as a struct's enum field is (loft#1868: a
            // generic's tuple element of an enum type was refused here).
            Type::Enum(_, false, _) => self.cl("OpSetEnum", &[ref_code.clone(), pos_v, value]),
            Type::Reference(inner_d_nr, _) | Type::Enum(inner_d_nr, true, _) => {
                let type_nr = if self.first_pass {
                    Value::Int(i32::from(u16::MAX))
                } else {
                    Value::Int(i32::from(self.data.def(*inner_d_nr).known_type()))
                };
                let field_ref = self.cl(
                    "OpGetField",
                    &[ref_code.clone(), pos_v.clone(), type_nr.clone()],
                );
                self.cl("OpCopyRecord", &[value, field_ref, type_nr])
            }
            Type::Vector(content, _) => {
                let vec_tp = self.vector_of(content);
                let elem_db_tp = self.database.content(vec_tp);
                let field_ref = self.cl(
                    "OpGetField",
                    &[
                        ref_code.clone(),
                        pos_v.clone(),
                        Value::Int(i32::from(vec_tp)),
                    ],
                );
                // loft#1689 — the member's write REPLACES what it held, as the keyed leg below
                // does (`keyed_member_write`): an append was right for a freshly built record,
                // whose member is empty, and wrong for a reassignment — `k.p = (3, [9])`
                // answered `[7,8,9]` on both backends.  `OpReplaceVector` takes the append's
                // arguments, clears nothing on a fresh member, and is a no-op when the value is
                // the member's own vector (`(H-CopySelf)`), which a clear-then-append is not.
                // An EMPTY literal (`k.p = (4, [])`) reaches here as a bare `null` — no vector is
                // built for `[]` — which neither the append nor the replace can take: the
                // interpreter read a corrupt reference and `--native` did not compile, before
                // and after the replace.  Its write is the clear.
                let empty = match value.unspan() {
                    Value::Null => true,
                    Value::Insert(ops) => ops.iter().all(|o| matches!(o.unspan(), Value::Null)),
                    _ => false,
                };
                if empty {
                    return vec![self.cl("OpClearVector", &[field_ref])];
                }
                self.cl(
                    "OpReplaceVector",
                    &[field_ref, value, Value::Int(i32::from(elem_db_tp))],
                )
            }
            Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
            | Type::Sorted(_, _, _) => {
                // A KEYED element is a record SET, and the element write COPIES it —
                // `OpReplaceKeyed`, exactly as `set_field_check` writes a keyed struct field
                // (loft#698).  Writing the source's 4-byte header with `OpSetInt4` left two
                // owners of one record set, and on the return path the `__tuple` record then
                // held the header of a frame-local backing the frame freed: the interpreter
                // wrote into a released, reused store (`Write to read-only store`, the const
                // store had taken the slot) and `--native` refused an int for a `DbRef`
                // (E0308) — an accept/reject split on `s = x; t = (s, 7); return t` with a
                // keyed `x` (QUALITY-history.md B7t; the cell D-tup-8's guard did not cross).  A bare
                // `Int` source is a raw header and keeps the raw write.
                let keyed_tp = elem_tp.base().clone();
                if matches!(value.unspan(), Value::Int(_))
                    || self.keyed_field_kt(&keyed_tp).is_none()
                {
                    self.cl("OpSetInt4", &[ref_code.clone(), pos_v, value])
                } else {
                    let kt = self.keyed_field_kt(&keyed_tp).unwrap_or(u16::MAX);
                    let tp_val = if self.call_gives_away_its_store(&value) {
                        i32::from(kt) | 0x8000
                    } else {
                        i32::from(kt)
                    };
                    let field_ref = self.cl(
                        "OpGetField",
                        &[ref_code.clone(), pos_v, Value::Int(i32::from(kt))],
                    );
                    self.keyed_member_write(field_ref, value, kt, tp_val)
                }
            }
            // Plan-06 phase 4d: nested tuple element — recurse into
            // `emit_tuple_set_ops` with the inner tuple's offsets so
            // each leaf primitive lands at `outer_pos +
            // outer_offsets[i] + inner_offsets[j]`.  The flattened
            // ops are returned as a single Vec.
            Type::Tuple(inner_elems) => {
                return self.emit_tuple_set_ops(ref_code, pos, inner_elems, value);
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Tuple struct field cannot contain element of type {}",
                        elem_tp.source_name(&self.data)
                    );
                }
                Value::Null
            }
        };
        vec![single]
    }

    pub(super) fn set_field_no_check(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
    ) -> Value {
        self.set_field_check(d_nr, f_nr, d_pos, ref_code, val_code, false, None)
    }

    /// `@FR-H-RefHome` — the local collection whose record a stored reference is set to
    /// (`h.r = items[1]`, `h.r = by_key[k]`), when there is one.
    pub(super) fn referenced_vector_local(&self, val: &Value) -> Option<u16> {
        let Value::Call(d, args) = val.unspan() else {
            return None;
        };
        if !matches!(
            self.data.def(*d).name(),
            "OpVectorRef" | "OpGetVector" | "OpGetRecord"
        ) {
            return None;
        }
        match args.first()?.unspan() {
            Value::Var(v)
                if !self.vars.is_argument(*v)
                    && matches!(
                        self.vars.tp(*v).base(),
                        Type::Vector(_, _)
                            | Type::Hash(_, _, _)
                            | Type::Sorted(_, _, _)
                            | Type::Index(_, _, _)
                    ) =>
            {
                Some(*v)
            }
            _ => None,
        }
    }

    // The field is addressed by `(d_nr, f_nr, d_pos)` and the write by `(ref_code, val_code)`;
    // `emit_check` and `elm_override` are the two independent switches over that. Bundling them
    // would name a struct after this one call site.
    #[allow(clippy::too_many_arguments)]
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn set_field_check(
        &mut self,
        d_nr: u32,
        f_nr: usize,
        d_pos: u16,
        ref_code: Value,
        val_code: Value,
        emit_check: bool,
        elm_override: Option<&Type>,
    ) -> Value {
        // @FR-H-Stride — for a vector ELEMENT (`f_nr == usize::MAX`) `attr_type` answers
        // `def.returned`, and a def cannot carry a width: the one `integer` def serves all
        // seven, so the write op below was chosen 8 bytes wide however the vector was
        // declared.  `insert(v, 0, 9)` into a `vector<u8>` then wrote eight bytes over a
        // one-byte slot and zeroed the five elements after it (loft#1420).  A caller that
        // holds the DECLARED element type passes it here instead.
        let tp = match elm_override {
            Some(t) => t.clone(),
            None => self.data.attr_type(d_nr, f_nr),
        };
        // @PLN25 slice (b): an `Optional(τ)` field writes exactly like its base — same
        // sentinel storage, same set-op. Peel the marker here so the whole emit path is
        // transparent to it (nullability is read separately via `attr_nullable`).
        let tp = tp.base().clone();
        let nm = self.data.attr_name(d_nr, f_nr);
        // `@FR-L-Escape` (loft#1877) — a struct holding a capturing closure may be placed into
        // another struct's field: a fresh value or an owned one placed once moves its closure
        // records into the field's store, and a copy of a value this function does not own is
        // refused by `(H-Copy-Refuse)`, since such a type owns a release (@C75).
        let pos = self
            .database
            .position(self.data.def(d_nr).known_type(), &nm);
        let pos_val = Value::Int(if f_nr == usize::MAX {
            i32::from(d_pos)
        } else {
            i32::from(pos + d_pos)
        });
        let has_check = emit_check
            && f_nr != usize::MAX
            && !self.first_pass
            && f_nr < self.data.def(d_nr).attributes.len()
            && self.field_check(d_nr, f_nr) != Value::Null;
        let ref_for_check = if has_check {
            Some(ref_code.clone())
        } else {
            None
        };
        let set_op = match tp {
            Type::Integer(ref spec) => {
                // Post-2c: honor size(N) on the alias recorded during field
                // parsing; fall back to the limit()-based heuristic.
                let alias_nr = if f_nr == usize::MAX {
                    u32::MAX
                } else {
                    self.data.def(d_nr).attributes()[f_nr].alias_d_nr
                };
                // narrow-vec path mirrors `get_val`.
                // Reached by `insert(vector<u16>, ...)` where
                // `set_field` is invoked with `f_nr == usize::MAX` and
                // `tp` is the narrow element Type.  Struct-field
                // writes (alias_nr != u32::MAX) stay on the legacy
                // `OpSetShort` / `Parts::Short` `+1` encoding path.
                let narrow_vec = alias_nr == u32::MAX
                    && spec.forced_size.is_some()
                    && spec
                        .vector_narrow_width(self.data.attr_nullable(d_nr, f_nr))
                        .is_some();
                let s = self.data.forced_size(alias_nr).unwrap_or_else(|| {
                    // loft#1036 — `byte_width` is the one width home (`forced_size`
                    // first, then the range), so the alias and `limit(...)` spellings
                    // of one range write at the same width the READ decodes at.
                    spec.byte_width(self.data.attr_nullable(d_nr, f_nr))
                });
                // Size-consistency gate: the size resolved from
                // `forced_size` / limit must be one of the four
                // supported widths.  Any other value indicates a
                // post-2c regression in `size()` or a novel alias that
                // needs a matching Op emission branch here.
                debug_assert!(
                    matches!(s, 1 | 2 | 4 | 8),
                    "set_field_check: unexpected integer field width \
                     s={s} for {}.{} (alias_d_nr={alias_nr}) — only \
                     1/2/4/8 are supported by the OpSet* family",
                    self.data.def(d_nr).name(),
                    if f_nr == usize::MAX {
                        "<unknown>".to_string()
                    } else {
                        self.data.def(d_nr).attributes()[f_nr].name.clone()
                    },
                );
                // H4-medium: same width→op home (`NarrowIntKind::of`) as the
                // READ in `get_val`, so the write op matches the read op for the
                // field.  A NULLABLE byte STRUCT FIELD reserves the 256th code as
                // the null sentinel (the Nullable op pair translates null ↔ the
                // sentinel); `not null` fields and narrow-vector elements keep the
                // raw op.
                let nullable = f_nr != usize::MAX && self.data.attr_nullable(d_nr, f_nr);
                // loft#1615: the same `NarrowSlot` the READ (`get_val`) asks, so the ops
                // and the `min` cannot drift.  A NON-nullable byte that kept a top code
                // writes through the NULLABLE setter: the plain one casts its value
                // `as i32` before the store sees it, and `i64::MIN as i32` is `0` — a
                // VALUE — so @C85's overflow would land in the slot as the number zero.
                let slot = crate::data::NarrowSlot::of_slot(s, nullable, narrow_vec, spec);
                // H6: the WRITE op encodes against the same `usable_min` the READ
                // op (`get_val`) decodes against — derived from the KIND so a
                // sentinel-reserving kind (`ByteNullable`/`Short`) shrinks the
                // range identically on both sides and raw kinds keep the full `min`.
                let m = Value::Int(slot.min);
                if slot.kind.takes_min() {
                    self.cl(slot.set_op(), &[ref_code, pos_val, m, val_code])
                } else {
                    self.cl(slot.set_op(), &[ref_code, pos_val, val_code])
                }
            }
            Type::Vector(ref content, _)
                if f_nr != usize::MAX && !matches!(val_code.unspan(), Value::Int(_)) =>
            {
                // A real struct/tuple vector FIELD write — e.g. the tuple-return
                // heap-promotion (`control.rs::rewrite_tail_tuple_with_work_ref`)
                // hands `val_code` the WHOLE vector.  Deep-copy it into the
                // field's own freshly-allocated store, exactly as
                // `emit_set_one_element` (above) and the struct constructor
                // (`objects.rs`) already do.  A bare `OpSetInt4` here writes the
                // 8-byte vector DbRef as a 4-byte int → stack skew → garbage
                // write into the locked CONST_STORE (`store.rs:1374` tuple-return
                // crash; native rejects the `DbRef as i32` cast).  The
                // `f_nr == usize::MAX` narrow-vec `insert` path (raw 4-byte
                // header) keeps `OpSetInt4` in the arm below.
                let vec_tp = self.vector_of(content);
                let elem_db_tp = self.database.content(vec_tp);
                let field_ref = self.cl(
                    "OpGetField",
                    &[ref_code, pos_val, Value::Int(i32::from(vec_tp))],
                );
                self.cl(
                    "OpAppendVector",
                    &[field_ref, val_code, Value::Int(i32::from(elem_db_tp))],
                )
            }
            Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
            | Type::Sorted(_, _, _)
                if f_nr != usize::MAX
                    && !matches!(val_code.unspan(), Value::Int(_))
                    && self.keyed_field_kt(&tp).is_some() =>
            {
                // A KEYED collection field written from a whole VALUE — a `hash<K[k]>`
                // field taking its default from a call (loft#698).  Deep-copy it into
                // the field the same way `objects.rs::handle_field` does for a supplied
                // `F { h: build() }`: `OpReplaceKeyed` = remove_claims(field) (a no-op on
                // the zero-inited field) + copy_claims(source, field).  The `OpSetInt4`
                // below would instead write the source's 4-byte header straight into the
                // field, leaving two owners of one record set — it panicked in `store.rs`
                // on the first insert through the aliased header.  `0x8000` frees the
                // call's fresh return storage after the copy, or it leaks per construction.
                let kt = self.keyed_field_kt(&tp).unwrap_or(u16::MAX);
                let tp_val = if self.call_gives_away_its_store(&val_code) {
                    i32::from(kt) | 0x8000
                } else {
                    i32::from(kt)
                };
                let field_ref = self.cl(
                    "OpGetField",
                    &[ref_code, pos_val, Value::Int(i32::from(kt))],
                );
                self.keyed_member_write(field_ref, val_code, kt, tp_val)
            }
            Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
            | Type::Sorted(_, _, _) => {
                // Collection header is a 4-byte u32 record pointer.  Post-2c
                // `OpSetInt` writes 8 bytes (i64), which overflows into the
                // next field.  Use `OpSetInt4` to write only 4 bytes.  Reached
                // by the narrow-vec `insert` raw-header path (`f_nr ==
                // usize::MAX`); keyed-collection struct fields deep-copy earlier
                // in `objects.rs`.
                self.cl("OpSetInt4", &[ref_code, pos_val, val_code])
            }
            Type::Function(..) => {
                // P213: storage is now TWO database fields per loft
                // attribute — `<attr>` (4B int holding the lambda's
                // d_nr; database name matches the loft attribute name
                // so `database.position(name)` lookups still resolve)
                // and `<attr>__closure_rec` (4B vector header at
                // `pos + 4`, pointing at the co-located closure record
                // in host's Store; empty for non-capturing).  At
                // read time `OpGetInt4` recovers the d_nr;
                // `OpVectorFirstOrNull` recovers the closure DbRef.
                //
                // Field-write paths:
                //   - Non-capturing lambda (`FnRef(d, MAX, _)`):
                //     write d_nr only; the closure_rec vector stays
                //     at its zero default (no records).
                //   - Capturing lambda (a `fn_ref_with_closure` Block
                //     ending in `FnRef(d, w, _)`): run the lambda's
                //     existing alloc_steps to build the closure
                //     record in parent's Store under `var(w)`, then
                //     write d_nr + `OpAppendVector` deep-copies the
                //     parent-Store closure record into element [0]
                //     of host's `__closure_rec` vector.  Parent's
                //     record gets freed by the existing
                //     parent-scope `OpFreeRef(w)`; host owns its own
                //     deep-copied copy.
                //   - Non-inline source (Var / Call returning a
                //     fn-ref): diagnose ("only inline lambda
                //     literals can be stored in fn-ref struct
                //     fields in this release") — out of scope here;
                //     follow-on plan extends to the non-inline case.
                //
                // Record the lambda's d_nr on the host attribute so
                // `typedef::fill_database`'s `Type::Function` arm
                // can register the `__closure_rec` field with the
                // correct closure-record schema.  Heterogeneous
                // captures across multiple constructors of the same
                // host struct are diagnosed at the second site.
                if let Some((lambda_d, _w)) = find_capturing_fn_ref(&self.data, &val_code)
                    && f_nr != usize::MAX
                {
                    let lambda_d_u = lambda_d as u32;
                    let prev = self.data.def(d_nr).attributes()[f_nr].assigned_lambda_d_nr;
                    if prev == u32::MAX {
                        self.data.definitions[d_nr as usize].attributes[f_nr]
                            .assigned_lambda_d_nr = lambda_d_u;
                    } else if prev != lambda_d_u && self.first_pass {
                        // `@FR-L-Escape` (loft#1874) — a field holds any closure, so every lambda
                        // written into it shares ONE closure record: the union of their captures,
                        // each lambda reading its own by name.  The field's child record then has
                        // one layout whatever closure it holds.
                        self.unite_field_closure_records(prev, lambda_d_u);
                    }
                    // `@FR-L-Escape` (loft#1867): a host rooted in an ARGUMENT is the caller's
                    // struct, and the closure record is built in its store (`OpChildRec`).  The
                    // record leaves this frame with it and adopts its captures, as it does for
                    // a struct this function returns (`claimed_into_delivered`).
                }
                emit_fn_ref_field_write(self, d_nr, f_nr, ref_code, pos_val, &val_code)
            }
            Type::Tuple(ref elems) => {
                // Plan-06 phase 4d: tuple struct field assignment.
                // Storage layout matches the synthetic `__tuple<…>`
                // struct's element positions (registered via
                // `tuple_def`) so each element write goes through
                // the same `OpSetInt`/`OpSetFloat`/etc. opcodes used
                // for ordinary struct fields.  Recurses for nested
                // Tuple element types — see `emit_tuple_set_ops`.
                let elems_vec = elems.clone();
                let host_field_pos = if let Value::Int(p) = pos_val {
                    p as u16
                } else {
                    0
                };
                let ops = self.emit_tuple_set_ops(&ref_code, host_field_pos, &elems_vec, val_code);
                v_block(ops, Type::Void, "tuple_field_set")
            }
            Type::Character => self.cl("OpSetCharacter", &[ref_code, pos_val, val_code]),
            Type::Reference(inner_tp, deps) => {
                // loft#821 — a `vector<(…)>` element slot arrives here typed
                // `Reference(__tuple<…>)`, the STORED spelling of a tuple, while the
                // value being written is a stack tuple.  `OpCopyRecord` below would
                // then read that tuple's own bytes as a `DbRef`: SIGSEGV on the
                // interpreter, `E0308: expected DbRef, found (f64, f64, f64)` on
                // `--native`.  A tuple LITERAL never showed it, because a literal is
                // typed `Type::Tuple` and takes the arm above.
                //
                // The two spellings are one loft type, so the arm is chosen by the
                // SOURCE's representation rather than by which spelling the slot
                // happened to carry — and `emit_tuple_set_ops` already makes exactly
                // that choice: per-element writes for a stack tuple, one
                // `OpCopyRecord` when the source is itself a promoted
                // `Reference(__tuple<…>)` (PLAN51 V-b).
                // …and the slot has to actually HOLD that record.  `deps.contains(&u16::MAX)`
                // is the #328 marker saying the attribute stores a 12-byte `DbRef` rather
                // than inline bytes — the spelling three other sites already read for the
                // same question (`objects.rs`'s two, and the #318 carrying-walk below).  A
                // closure record's capture of a `vector<(…)>` is exactly that: it is typed
                // `Reference(__tuple<…>)` because `closure_attr_type` names the ELEMENT def
                // as a stand-in for "some DbRef", and the value stored is the COLLECTION
                // handle.  Reading the destination alone, this arm wrote the vector's own
                // bytes out as two integers, so the closure captured a garbage pair and a
                // `for` over it read no elements at all — silently on `--interpret`, `E0308`
                // on `--native` (loft#1131).
                if !self.first_pass
                    && !deps.contains(&u16::MAX)
                    && self.data.def(inner_tp).name().starts_with("__tuple<")
                {
                    // Read in the SOURCE spelling: `emit_tuple_set_ops` re-derives the
                    // `__tuple<…>` def from these types to get its offsets, and the def is
                    // named by what the author wrote.  Handing it the stored spelling of a
                    // nullable member minted `__tuple<__nullable<S>,integer>` instead, whose
                    // `_1` sits at 16 where the real one sits at 24 — so the append wrote a
                    // member the read could not find.
                    let elems: Vec<Type> = self
                        .data
                        .def(inner_tp)
                        .attributes()
                        .iter()
                        .map(|a| self.source_spelling(&a.typedef))
                        .collect();
                    let base_pos = if let Value::Int(p) = pos_val {
                        p as u16
                    } else {
                        0
                    };
                    let ops = self.emit_tuple_set_ops(&ref_code, base_pos, &elems, val_code);
                    return v_block(ops, Type::Void, "tuple_elem_set");
                }
                // `@FR-H-RefHome` — on both passes and either arm below: pass 1 may take the
                // copy arm for a write pass 2 lowers as a stored reference.
                if let Some(v) = self.referenced_vector_local(&val_code) {
                    self.referenced_locals.insert((self.context, v));
                }
                if deps.is_empty() {
                    // The value is a 12-byte DbRef; OpSetInt would only read 4 bytes of it.
                    // Copy the struct bytes into the embedded field instead.
                    let type_nr = if self.first_pass {
                        Value::Int(i32::from(u16::MAX))
                    } else {
                        Value::Int(i32::from(self.data.def(inner_tp).known_type()))
                    };
                    let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
                    // Note: the free-source high-bit for Issue #120 is set in
                    // copy_ref() (operators.rs), which is the path for struct
                    // field reassignment. This set_field_check path is for
                    // construction (initial field population).
                    self.cl("OpCopyRecord", &[val_code.clone(), field_ref, type_nr])
                } else {
                    // Plan-22 phase 02b (2026-05-12): auto-Reference field
                    // — store the source's 12-byte DbRef directly,
                    // sharing the underlying record.  Phase 02c sets
                    // non-empty deps for mutated Reference captures on
                    // closure records; today's user code paths keep
                    // empty deps and stay on the legacy OpCopyRecord
                    // path above.
                    self.cl("OpSetDbRef", &[ref_code, pos_val, val_code])
                }
            }
            Type::Enum(_, false, _) => self.cl("OpSetEnum", &[ref_code, pos_val, val_code]),
            Type::Enum(nr, true, _) => {
                // A struct-enum field holds an inline record; copy the source enum
                // into the field AT ITS OFFSET via a field sub-ref — exactly like
                // the Type::Reference arm above.  Copying into `ref_code` (the
                // struct base) instead landed a non-first struct-enum field at
                // offset 0, clobbering field 0 and reading a garbage discriminant
                // (#406: `Entry { key: a, value: b }` from enum variables).
                let type_nr = if self.first_pass {
                    // known_type() is u16::MAX until the enum registers in pass 2;
                    // emit the placeholder now (codegen re-runs in pass 2), matching
                    // the Type::Reference arm.
                    Value::Int(i32::from(u16::MAX))
                } else {
                    Value::Int(i32::from(self.data.def(nr).known_type()))
                };
                let field_ref = self.cl("OpGetField", &[ref_code, pos_val, type_nr.clone()]);
                self.cl("OpCopyRecord", &[val_code, field_ref, type_nr])
            }
            // @PLN17: store the boolean's u8 form (0/1/255) directly, like enum —
            // the old `if val {1} else {0}` forced 0/1 and dropped the null sentinel.
            Type::Boolean => self.cl("OpSetBoolean", &[ref_code, pos_val, val_code]),
            Type::Float => self.cl("OpSetFloat", &[ref_code, pos_val, val_code]),
            Type::Single => self.cl("OpSetSingle", &[ref_code, pos_val, val_code]),
            Type::Text(_) => self.cl("OpSetText", &[ref_code, pos_val, val_code]),
            // A generator handle (loft#1585) is stored as the 12-byte `DbRef` it is.
            Type::Iterator(_, _) => self.cl("OpSetDbRef", &[ref_code, pos_val, val_code]),
            _ => {
                if self.first_pass {
                    Value::Null
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot assign to field '{}' of type {}",
                        self.data.attr_name(d_nr, f_nr),
                        self.data.attr_type(d_nr, f_nr).source_name(&self.data)
                    );
                    Value::Null
                }
            }
        };
        self.emit_field_constraint(set_op, ref_for_check, d_nr, f_nr, &nm)
    }

    /// Wrap a set operation with a constraint assertion if the field has one.
    pub(super) fn emit_field_constraint(
        &mut self,
        set_op: Value,
        ref_for_check: Option<Value>,
        d_nr: u32,
        f_nr: usize,
        field_name: &str,
    ) -> Value {
        let Some(ref_val) = ref_for_check else {
            return set_op;
        };
        let check = self.field_check(d_nr, f_nr);
        let bound = Self::replace_record_ref(check, &ref_val);
        let msg = if let Value::Text(s) = &self.field_check_message(d_nr, f_nr) {
            Value::Text(s.clone())
        } else {
            Value::Text(format!(
                "field constraint failed on {}.{field_name}",
                self.data.def(d_nr).name()
            ))
        };
        let assert_dnr = self.data.def_nr("n_assert");
        let pos = self.lexer.pos();
        let assert_call = Value::Call(
            assert_dnr,
            vec![
                bound,
                msg,
                Value::Text(pos.file.to_string()),
                Value::Int(pos.line as i32),
            ],
        );
        Value::Insert(vec![set_op, assert_call])
    }

    /// Append a `--show-types --trace` entry for the type at the
    /// current parse position.  No-op unless `trace_types` is set
    /// AND we are on the second pass (first-pass types are
    /// placeholders and would emit thousands of meaningless lines).
    pub(crate) fn record_type_trace(&mut self, t: &Type) {
        if !self.trace_types || self.first_pass {
            return;
        }
        let pos = self.lexer.pos();
        let fn_name = self.vars.name.clone();
        if fn_name.is_empty() {
            return;
        }
        let type_str = t.show(&self.data, &self.vars);
        self.trace_types_lines
            .push(format!("{fn_name}\t{}:{}\t{type_str}", pos.line, pos.pos));
    }
}
