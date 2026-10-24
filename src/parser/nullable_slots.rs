// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! A nullable struct in a SLOT (`__nullable<S>`, `@FR-L-Null-Which`): the tagged spelling a
//! slot stores, the dense-or-absent value it reads back as, and the writes that mark it
//! present or absent.

use super::{Deps, Parser, Type, Value, v_block, v_if, v_set};

impl Parser {
    /// @PLN25 single-payload — emit the steps that turn a `Some` record (`some_ref`, type
    /// `some_d`) into the PRESENT state from a dense-`S` source `src`: set the discriminant
    /// present (offset 0), then copy the whole dense `S` into the inline `payload` field.
    /// Single-payload makes `Some = { enum, payload: S }`, so this is ONE record copy (the
    /// `Reference` arm of `set_field_check` emits the `OpGetField`+`OpCopyRecord`), replacing
    /// the per-field copy loops the individual-field layout forced.  The caller owns the
    /// null branch (set the discriminant 0) and any source stashing.
    pub(crate) fn build_some_present(
        &mut self,
        some_d: u32,
        some_ref: Value,
        src: Value,
    ) -> Vec<Value> {
        let payload_attr = self.data.attr(some_d, "payload");
        vec![
            self.cl(
                "OpSetEnum",
                &[some_ref.clone(), Value::Int(0), Value::Enum(2, u16::MAX)],
            ),
            self.set_field_no_check(some_d, payload_attr, 0, some_ref, src),
        ]
    }

    /// The dense struct a synthetic `__nullable<S>` wraps, or `None` for any other type.
    ///
    /// Reads it off the `Some` variant's `payload` field rather than parsing the enum's
    /// NAME, so it answers with the def the layout actually uses.
    pub(crate) fn nullable_payload_struct(&self, syn: u32) -> Option<u32> {
        if !self.data.def(syn).name.starts_with("__nullable<") {
            return None;
        }
        let some_d = self.data.variant_of(syn, "Some");
        if some_d == u32::MAX {
            return None;
        }
        let payload_attr = self.data.attr(some_d, "payload");
        if payload_attr == usize::MAX {
            return None;
        }
        match self.data.attr_type(some_d, payload_attr) {
            Type::Reference(struct_d, _) => Some(struct_d),
            _ => None,
        }
    }

    /// Write a value that is spelled `S?` into a slot that is STORED as the tagged
    /// `__nullable<S>` — one home for the three positions that hold such a slot inline: a
    /// struct field, a collection element, and a tuple member.
    ///
    /// Enforces `@FR-L-Null-Tag`: an inline struct slot has no out-of-band absent value the way
    /// a `DbRef` field does — it spends its whole width on real values — so `S?` is the tagged
    /// `__nullable<S>` (discriminant, then the payload) rather than `@FR-L-Null`'s sentinel in
    /// the same bytes.  A PRESENT value therefore has
    /// to say it is present. Copying the dense `S` in raw leaves the discriminant aliased with
    /// the payload's first byte, and then presence is decided by whatever that byte holds: a
    /// legitimate `S { a: 0, … }` reads back ABSENT, and a `float` first member reads absent
    /// for every value whose low byte is zero.
    ///
    /// The source arrives DENSE because it is parsed against the spelling the author writes
    /// (`S?`), which is `Optional(Reference(S))` — the enum is the STORAGE spelling and exists
    /// only in the layout. So the write is: stash the source once, set the discriminant
    /// present and copy the dense payload when it is non-null, and otherwise leave the slot
    /// absent. `build_nullable_set_null` is what the absent arm uses, so a slot being
    /// REASSIGNED releases the payload it held instead of leaking it.
    ///
    /// The PRESENT arm releases it too, for the same reason and with the same op: both arms
    /// clobber the slot in place, so a `Some` payload holding a keyed collection or a nested
    /// store is orphaned unless the write says otherwise. Exactly one clear on either path —
    /// `build_nullable_set_null` carries the absent arm's — because `OpClearKeyed` reads the
    /// discriminant, and a second one over a slot whose discriminant still says `Some` would
    /// release the same claims twice.
    ///
    /// `slot_ref` addresses the slot itself — an `OpGetField` at the enum's own type.
    /// Answers the statements to emit; the caller decides where they go.
    pub(crate) fn emit_nullable_slot_write(
        &mut self,
        syn: u32,
        slot_ref: &Value,
        value: Value,
        fresh: bool,
    ) -> Vec<Value> {
        let Some(struct_d) = self.nullable_payload_struct(syn) else {
            return Vec::new();
        };
        // The stash is typed by the PAYLOAD struct, not by the source expression: a bare
        // `null` member has type `Type::Null`, which names no storage to stash into.
        let src_tp = Type::Reference(struct_d, crate::data::Deps::none());
        let src_var = self.vars.work_refs(&src_tp, &mut self.lexer);
        if src_var == u16::MAX {
            return Vec::new();
        }
        let mut list = vec![v_set(src_var, value)];
        list.push(self.nullable_slot_write_from(syn, slot_ref, src_var, fresh));
        list
    }

    /// [`Self::emit_nullable_slot_write`] from a value already held in `src_var`, a reference
    /// local of the payload struct (or the sentinel): the slot takes its own copy of it, or
    /// is emptied when it is null.  For a source that IS a variable, so no stash is needed —
    /// a stash `Set` from a variable is a record copy on `--native`, which nothing releases
    /// (loft#2146's store-back).
    pub(crate) fn nullable_slot_write_from(
        &mut self,
        syn: u32,
        slot_ref: &Value,
        src_var: u16,
        fresh: bool,
    ) -> Value {
        let some_d = self.data.variant_of(syn, "Some");
        let kt = self.data.def(syn).known_type();
        let mut present = Vec::with_capacity(3);
        // `@FR-H-Claim` — a FRESH slot, an element the construction just minted, holds no value yet, so
        // there is nothing to release and nothing to read: its bytes are not zero by any
        // promise.  Every other slot may hold a previous value whose payload is released.
        if kt != u16::MAX && !fresh {
            present.push(self.cl(
                "OpClearKeyed",
                &[slot_ref.clone(), Value::Int(i32::from(kt))],
            ));
        }
        let mut some = self.build_some_present(some_d, slot_ref.clone(), Value::Var(src_var));
        if fresh {
            self.mark_copies_fresh(&mut some);
        }
        present.extend(some);
        let absent = if fresh {
            self.cl(
                "OpSetEnum",
                &[slot_ref.clone(), Value::Int(0), Value::Enum(0, u16::MAX)],
            )
        } else {
            self.build_nullable_set_null(syn, slot_ref.clone())
        };
        let is_null = self.cl("OpRefIsNull", &[Value::Var(src_var)]);
        let not_null = self.cl("OpNot", &[is_null]);
        v_if(not_null, Value::Insert(present), absent)
    }

    /// Flag every record copy in `steps` as writing a FRESH destination
    /// (`keys::COPY_FRESH_DEST`): the payload of a slot the construction just minted has no
    /// previous value for the copy to release.
    pub(crate) fn mark_copies_fresh(&self, steps: &mut [Value]) {
        fn mark(v: &mut Value, copy: u32) {
            match v {
                Value::Span(b) => mark(&mut b.1, copy),
                Value::Call(d, args) if *d == copy => {
                    if let Some(Value::Int(tp)) = args.get_mut(2) {
                        *tp |= i32::from(crate::keys::COPY_FRESH_DEST);
                    }
                    v.for_each_child_mut(&mut |c| mark(c, copy));
                }
                _ => v.for_each_child_mut(&mut |c| mark(c, copy)),
            }
        }
        let copy = self.data.def_nr("OpCopyRecord");
        for step in steps {
            mark(step, copy);
        }
    }

    /// The POINTER spelling a tagged `__nullable<S>` value takes once it leaves its slot:
    /// `Optional(Reference(S))` carrying the slot's deps — or `None` when `tp` is not a
    /// tagged nullable.  The type half of [`Self::read_through_tag`], for a site that has
    /// to know the local's type before the value's read exists (the tuple destructure).
    pub(crate) fn tagged_pointer_type(&self, tp: &Type) -> Option<(u32, Type)> {
        let Type::Enum(syn, true, _) = tp else {
            return None;
        };
        let struct_d = self.nullable_payload_struct(*syn)?;
        let pointer = Type::Optional(Box::new(
            Type::Reference(struct_d, crate::data::Deps::none()).with_deps_of(tp),
        ));
        Some((*syn, pointer))
    }

    /// `@FR-L-Null-Which` — a tagged `__nullable<S>` is a SLOT's spelling of `S?` (an
    /// embedded field, a vector element, a tuple member); everywhere else `S?` is the
    /// pointer, `nullref` for absence.  A tagged value reaching a non-slot position — a
    /// local's bind, a `??` or `?` subject — is therefore read through its tag HERE, once,
    /// and both the value and its type become the pointer.  `false` when `tp` was not a
    /// tagged nullable and nothing moved.  Left in the slot's spelling, a local took
    /// whichever spelling its LAST assignment parsed and the other assignment's value went
    /// in unconverted (loft#1367), and a `??` joined a tagged arm with a pointer arm and
    /// refused the program naming the synthetic (`__nullable<S>?`).
    ///
    /// BOTH passes, so the local's type is the pointer on each: a `vector<S?>` element is the
    /// synthetic from pass 1 (the element type is rewritten where the vector type resolves),
    /// and converting only on pass 2 typed the local as the slot on pass 1 and refused pass
    /// 2's pointer as a type change.  The pass-1 VALUE may come back unconverted — the
    /// emitter declines while the layout has no known type yet — and is rebuilt anyway.
    pub(crate) fn read_through_tag(&mut self, code: &mut Value, tp: &mut Type) -> bool {
        let Some((syn, pointer)) = self.tagged_pointer_type(tp) else {
            // A struct-enum slot is the same rule with the discriminant in its own bytes.
            return self.read_through_enum_slot(code, tp);
        };
        *code = self.emit_nullable_slot_read(syn, code.clone(), tp);
        *tp = pointer;
        true
    }

    /// Read a tagged `__nullable<S>` slot back as the dense-or-absent value its declared
    /// type `S?` promises — the mirror of [`emit_nullable_slot_write`].
    ///
    /// The read half of `@FR-L-Null-Tag`.  A reader that takes the slot's address as if it were
    /// a dense `S` is wrong twice: it
    /// starts at the DISCRIMINANT rather than at the payload, so every field answers with its
    /// predecessor's value, and it has no way to say "absent" at all, so a cleared slot reads
    /// as a present record of zeroes. Consult the discriminant and project the payload, or
    /// answer the reference null sentinel — which is what `S?` means outside the slot.
    ///
    /// The discriminant test is spelled exactly as the inline null test is
    /// (`operators.rs::enum_null`), so a slot written by one and read by the other cannot
    /// disagree about which values are present.
    pub(crate) fn emit_nullable_slot_read(
        &mut self,
        syn: u32,
        slot_ref: Value,
        src_tp: &Type,
    ) -> Value {
        let Some(struct_d) = self.nullable_payload_struct(syn) else {
            return slot_ref;
        };
        let some_kt = {
            let some_d = self.data.variant_of(syn, "Some");
            self.data.def(some_d).known_type()
        };
        let struct_kt = self.data.def(struct_d).known_type();
        if some_kt == u16::MAX || struct_kt == u16::MAX {
            return slot_ref;
        }
        let payload_pos = i32::from(self.database.position(some_kt, "payload"));
        // The result is a VIEW into whatever store the slot lives in, so it inherits that
        // store's deps.  Building it with `Deps::none()` instead said the argument borrows
        // nothing, the @P290 bracket then saw no store to protect, `scan_args` stopped lifting
        // the argument to a name, and the caller kept the callee's minted return without ever
        // freeing it — one record per call (loft#1105's leak gate is the only channel that
        // scores that; every value was identical either way).
        let dense = Type::Reference(struct_d, crate::data::Deps::none()).with_deps_of(src_tp);
        let result_tp = Type::Optional(Box::new(dense));
        // The slot is read TWICE — once for the discriminant, once for the payload — so an
        // expression that is not free to repeat is named first.  A projection chain
        // (`OpGetField(v, …)`, the tuple-element route) IS free to repeat: it is address
        // arithmetic with no store in it.  Anything else can be a whole `??` block, whose
        // default arm BUILDS a value, and evaluating that twice mints two stores and orphans
        // one.
        if Self::is_repeatable_place(&self.data, &slot_ref) {
            let pick = self.nullable_pick(slot_ref.clone(), slot_ref, payload_pos, struct_kt);
            return pick;
        }
        let named = self.vars.work_refs(src_tp, &mut self.lexer);
        if named == u16::MAX {
            return slot_ref;
        }
        // The name BORROWS — it holds a `DbRef` into a store the container owns — so the
        // scope-exit sweep must not free it, and it allocates nothing of its own.
        self.vars.set_skip_free(named);
        self.vars.mark_inline_ref(named);
        let pick = self.nullable_pick(Value::Var(named), Value::Var(named), payload_pos, struct_kt);
        v_block(
            vec![v_set(named, slot_ref), pick],
            result_tp,
            "nullable_slot_read",
        )
    }

    /// Is `code` a nullable STRUCT-ENUM read that is still its SLOT — a field or a
    /// `vector<E?>` element, or a variable that views one?  Layout.md `(L-Null)` spells the
    /// absence of such a slot as discriminant 0 in its own bytes, so the value is a
    /// sub-reference into the holder and never itself null.  The two shapes are exactly the
    /// two slot branches of [`Parser::null_test`], which is where the question was first
    /// answered: an `OpGetField` read (a field, and an element, which is read as a field at
    /// offset 0) and a variable [`Parser::views_a_nullable_element_slot`] says is one.
    /// The tagged `__nullable<S>` has its own reader ([`Self::read_through_tag`]).
    pub(crate) fn enum_slot_view(&self, code: &Value, tp: &Type) -> bool {
        if !matches!(tp, Type::Optional(_)) {
            return false;
        }
        let Type::Enum(e_nr, true, _) = tp.base() else {
            return false;
        };
        if self.data.def(*e_nr).name.starts_with("__nullable<") {
            return false;
        }
        self.inline_slot_word(code).is_some()
            || (matches!(code.unspan(), Value::Var(_)) && self.views_a_nullable_element_slot(tp))
    }

    /// `(L-Null-Which)` for a nullable struct-enum slot: a value LEAVING the slot for a
    /// position that is not a slot — a local, a parameter, a return, a `??` or `?` subject,
    /// the base of a field read — is the pointer, `nullref` when the slot is absent.  Left as
    /// the slot's sub-reference it was present to every handle test and read as a record of
    /// zeroes (loft#1529).  The type is unchanged (`E?` either way), so this is pass-2 only.
    /// The slot is read twice, for the tag and as the value, which a projection is free to
    /// do.  A slot read off a CALL's record (`fresh().e`) is not: the call is named once and
    /// the slot re-read off the name.  Argument lifting would name it too, but only after
    /// parsing, which is too late for this decision, so the name takes the lift's place and
    /// with it the lift's OWNERSHIP of the delivered record — marked as a borrow instead, the
    /// record was never released (one `Holder` store per call, on both backends).
    pub(crate) fn read_through_enum_slot(&mut self, code: &mut Value, tp: &Type) -> bool {
        if self.first_pass || !self.enum_slot_view(code, tp) {
            return false;
        }
        if Self::is_repeatable_place(&self.data, code) {
            let Some(absent) = self.null_test(code.clone(), tp, false) else {
                return false;
            };
            let null = self.cl("OpNullRefSentinel", &[]);
            *code = v_if(absent, null, code.clone());
            return true;
        }
        let Value::Call(get_d, args) = code.unspan().clone() else {
            return false;
        };
        let Some(base) = args.first() else {
            return false;
        };
        let Value::Call(call_d, _) = base.unspan() else {
            return false;
        };
        let holder_d = match self.data.def(*call_d).returned().base() {
            Type::Reference(d, _) => *d,
            _ => return false,
        };
        let named = self
            .vars
            .work_refs(&Type::Reference(holder_d, Deps::none()), &mut self.lexer);
        if named == u16::MAX {
            return false;
        }
        self.vars.mark_inline_ref(named);
        let mut slot_args = args.clone();
        slot_args[0] = Value::Var(named);
        let slot = Value::Call(get_d, slot_args);
        let Some(absent) = self.null_test(slot.clone(), tp, false) else {
            return false;
        };
        let null = self.cl("OpNullRefSentinel", &[]);
        *code = v_block(
            vec![v_set(named, base.clone()), v_if(absent, null, slot)],
            tp.clone(),
            "enum_slot_read_named",
        );
        true
    }

    /// `if <discriminant is set> { <payload> } else { <the reference null sentinel> }` — the
    /// body of [`Self::emit_nullable_slot_read`], with the slot's address supplied twice
    /// because the tag and the payload are two reads of it.
    pub(super) fn nullable_pick(
        &mut self,
        for_tag: Value,
        for_payload: Value,
        payload_pos: i32,
        struct_kt: u16,
    ) -> Value {
        let disc = self.cl("OpGetEnum", &[for_tag, Value::Int(0)]);
        let as_int = self.cl("OpConvIntFromEnum", &[disc]);
        let is_absent = self.cl("OpEqInt", &[as_int, Value::Int(0)]);
        let present = self.cl("OpNot", &[is_absent]);
        let payload = self.cl(
            "OpGetField",
            &[
                for_payload,
                Value::Int(payload_pos),
                Value::Int(i32::from(struct_kt)),
            ],
        );
        let absent = self.cl("OpNullRefSentinel", &[]);
        v_if(present, payload, absent)
    }

    /// Is this value free to evaluate TWICE — a variable, or a projection chain rooted at one?
    ///
    /// A projection is address arithmetic: it reads a `DbRef` out of a place and allocates
    /// nothing, so repeating it costs a few instructions.  Everything else is assumed to have
    /// an effect worth keeping to one evaluation — a block that BUILDS a value is the case
    /// that matters, since repeating it mints a second store nothing then frees.
    pub(super) fn is_repeatable_place(data: &crate::data::Data, v: &Value) -> bool {
        match v.unspan() {
            Value::Var(_) => true,
            // A projection has two IR spellings and both are address arithmetic: the CALL
            // (`OpGetField`, `OpGetVector`, …) and `TupleGet`, which reads one element off a
            // stack tuple.  Answering only the first would stash a value that was free to
            // repeat — harmless, but it is the same half-answer the `spellings` screen exists
            // to find, so both are here.
            Value::TupleGet(_, _) => true,
            Value::Call(d, args) => {
                crate::use_analysis::is_projection_op(data, *d)
                    && args
                        .first()
                        .is_some_and(|a| Self::is_repeatable_place(data, a))
            }
            _ => false,
        }
    }

    /// The SOURCE spelling of a stored element type — the one the author wrote.
    ///
    /// A member declared `S?` is stored as the tagged `Enum(__nullable<S>)` (`@FR-L-Null-Tag`)
    /// while the author wrote `Optional(Reference(S))`, and the synthetic `__tuple<…>` def is
    /// NAMED by the second. So a list read straight off the def's attributes is in the storage
    /// spelling, and handing that to anything which re-derives the def from its element types
    /// mints a DIFFERENT tuple — different offsets, and every member after the tagged one read
    /// or written at the wrong place. It also hides the member from
    /// [`Self::needs_nullable_wrap`], which asks about the declared type.
    ///
    /// Everything that is not a nullable-struct wrapper answers unchanged, so this is safe to
    /// apply to a whole element list.
    pub(crate) fn source_spelling(&self, stored: &Type) -> Type {
        let Type::Enum(syn, true, deps) = stored else {
            return stored.clone();
        };
        match self.nullable_payload_struct(*syn) {
            Some(struct_d) => Type::Optional(Box::new(Type::Reference(struct_d, deps.clone()))),
            None => stored.clone(),
        }
    }

    /// Is this element type the DENSE spelling of `syn`'s payload — i.e. a value parsed
    /// against it arrives needing [`emit_nullable_slot_write`]?
    ///
    /// `S?` (`Optional(Reference(S))`), a bare `S` and a bare `null` all answer yes; the
    /// tagged enum itself answers NO, because such a value is already in the slot's own form
    /// and wrapping it a second time would bury the real discriminant one payload deep.
    pub(crate) fn needs_nullable_wrap(&self, syn: u32, src_tp: &Type) -> bool {
        let Some(struct_d) = self.nullable_payload_struct(syn) else {
            return false;
        };
        match src_tp.base() {
            Type::Reference(src_d, _) => *src_d == struct_d,
            Type::Null => true,
            _ => false,
        }
    }

    /// @PLN25 E2a.5 / loft#896 — emit "this `__nullable<S>` slot is ABSENT" against `to`, the
    /// slot's own `OpGetField`/element ref.
    ///
    /// A nullable struct field or vector element is the synthetic enum stored INLINE, so
    /// absence is discriminant `0` — not the `store_nr` null sentinel a real DbRef uses, since
    /// an inline slot has no DbRef of its own. Emitting a `copy_ref` here instead produces
    /// `OpCopyRecord(null, dest, …)`, which is a silent no-op on the interpreter and does not
    /// compile at all on native (`OpCopyRecord(cell, (), …)` — `()` where a `DbRef` is
    /// expected). The inline form of `== null` reads this same discriminant
    /// (`operators.rs::enum_null`), which is what makes the write and the test agree.
    ///
    /// A present `Some` carrying a heap payload (text, nested vector) is released FIRST via
    /// `OpClearKeyed` → `remove_claims`; without it the old payload leaks until the host store
    /// dies. That op reads the discriminant, so it is for a slot that HOLDS a value — an
    /// assignment's.  A slot the construction just minted holds none (its bytes are not zero
    /// by any promise) and takes the fresh form of [`Self::emit_nullable_slot_write`].
    pub(crate) fn build_nullable_set_null(&mut self, syn: u32, to: Value) -> Value {
        let set_null = self.cl(
            "OpSetEnum",
            &[to.clone(), Value::Int(0), Value::Enum(0, u16::MAX)],
        );
        let kt = self.data.def(syn).known_type();
        if !self.first_pass && kt != u16::MAX {
            let free = self.cl("OpClearKeyed", &[to, Value::Int(i32::from(kt))]);
            return v_block(vec![free, set_null], Type::Void, "nullable_elem_set_null");
        }
        set_null
    }
}
