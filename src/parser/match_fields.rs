// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Field sub-patterns and multi-pattern bindings, a match pattern for one arm, scalar and cursor
//! matches, and sub-rule (grammar) patterns with their well-formedness checks.
//! A part of the match subject split out of `control.rs`; the others are listed there.

use super::control::{SepSpec, SliceSubject, SubruleGroup, match_arm_types_unify};
use super::{
    DefType, I32, Level, LexItem, Parser, Type, Value, diagnostic_format, v_block, v_if, v_loop,
    v_set,
};
use crate::data::Deps;
use crate::data::IntegerSpec;
use std::collections::HashSet;

impl Parser {
    /// One field capture of a struct-enum pattern: a fresh `mv_<field>` slot bound to the
    /// field's value, named `field_name` for the arm body (the old binding goes to
    /// `name_aliases` for the arm's end to restore), with the free and borrow marks a
    /// field view needs.  Returns the slot, `u16::MAX` when none could be made.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn bind_match_field_capture(
        &mut self,
        variant_def_nr: u32,
        attr_idx: usize,
        field_name: &str,
        field_type: &Type,
        subject_val: &Value,
        arm_stmts: &mut Vec<Value>,
        name_aliases: &mut Vec<(String, Option<u16>)>,
    ) -> u16 {
        let field_read = self.get_field(variant_def_nr, attr_idx, subject_val.clone());
        let v_nr = self.create_unique(&format!("mv_{field_name}"), field_type);
        // The binding's VALUE reads through a nullable slot's tag; the
        // ORIGIN recorded below stays the slot's own read, since a write
        // resolved back to the field (`resolved_group_write`) is spelled
        // against the field.
        let bound = self.pattern_field_value(variant_def_nr, attr_idx, subject_val.clone());
        if v_nr != u16::MAX {
            self.vars.defined(v_nr);
            // loft#1160 — remember which field this binding projects, so a
            // write spelled through it can take the field path and reach the
            // linked group the field belongs to.
            self.vars.mv_field_origin.insert(
                v_nr,
                (
                    field_read.clone(),
                    Type::Reference(variant_def_nr, Deps::none()),
                ),
            );
            arm_stmts.push(v_set(v_nr, bound));
            let old = self.name_capture(field_name, v_nr);
            name_aliases.push((field_name.to_string(), old));
            // B5 remaining half (2026-04-14): a HEAP match-arm
            // binding is a field extraction from the subject —
            // the subject owns the store and the binding is a
            // borrowed view (a DbRef pointing into the subject's
            // record).  Emitting OpFreeRef for it at function
            // exit would decrement a store the binding doesn't
            // own; worse, if the arm wasn't taken the slot is
            // never assigned and the free reads garbage bytes as
            // a DbRef (observed as out-of-bounds store_nr ≈ 4621
            // in `p54_b5_recursive_struct_enum`).  Mark it
            // `skip_free` so scope cleanup leaves it alone in
            // both the taken and not-taken arms.
            //
            // @PLN85 Class B — but a TEXT payload binding is
            // NOT a borrow: `_mv_<f> = OpGetText(subj, off)` is
            // typed plain `text` (an OWNED copy), and it is
            // default-initialised to `""` at block entry — so
            // freeing it is correct (it owns an allocation) AND
            // safe in the not-taken arm (`OpFreeText("")` is a
            // no-op, no garbage read).  Leaving it `skip_free`
            // leaked the copy 1/call whenever a text-payload arm
            // was taken (the whole p54 struct-enum / json-match
            // family).  So skip_free HEAP bindings only; let a
            // text binding free through normal scope cleanup.
            if matches!(field_type.base(), Type::Text(_)) {
                self.record_text_payload_view(v_nr, &field_read);
            } else {
                self.vars.set_skip_free(v_nr);
            }
            // #429: the binding is a BORROWED VIEW of the
            // subject, so its TYPE must record that borrow —
            // otherwise a value derived from it and returned
            // (`CMap { entries } => { r = entries[..]; return r }`)
            // breaks the borrow chain at the binding: `ref_return`
            // walks `r` → `entries` → <this binding> and stops (the
            // binding has empty deps), never reaching the subject
            // parameter, so the fn is mis-classified OWNED and the
            // caller whole-store-frees the subject's record (#429
            // interp-vs-native divergence).  Give a HEAP
            // (DbRef-carrying) binding a frame dep on the subject's
            // source var so the chain reaches the parameter — exactly
            // the `["src"]` dep a `b = subj.field` bind already
            // carries.  Scalars hold no DbRef, so they need no borrow
            // dep (the `_mv_value` integer binding stays dep-free).
            // A nullable payload (`S?`, `vector<T>?`) borrows the same store
            // as its dense twin (`@FR-N-Shape`), so the shape is asked of the
            // base type, as `mark_slice_element_view` asks it.
            if matches!(
                field_type.base(),
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            ) && let Some(src) = self.match_borrow_source(subject_val)
            {
                if crate::env_once!(std::env::var_os("LOFT_MV_DEP_TRACE").is_some()) {
                    eprintln!(
                        "[mv-dep] fn={} pass{} binding={}({}) src={}({})",
                        self.data.def(self.context).name(),
                        u8::from(!self.first_pass) + 1,
                        self.vars.name(v_nr),
                        v_nr,
                        self.vars.name(src),
                        src,
                    );
                }
                let bound_tp = Self::element_view_of(&self.vars.tp(v_nr).clone(), src);
                self.vars.set_type(v_nr, bound_tp);
            }
        }
        v_nr
    }

    /// Does a listed pattern's field of type `field_type` fit the shared slot `shared_ty`
    /// another pattern made for `field_name`?  Reports the mismatch when not (`@FR-P-Alt-Same`).
    pub(super) fn shared_slot_accepts(
        &mut self,
        field_name: &str,
        field_type: &Type,
        shared_ty: &Type,
    ) -> bool {
        // `@FR-L-Null-Which` — a local holds a `S?` field as the pointer, so
        // compare the bound spellings: a declared `S?` can reach here as
        // its tagged slot type (`__nullable<S>`) and was refused against
        // the first pattern's `S?`, naming the synthetic.
        let bound_ty = |tp: &Type| {
            self.tagged_pointer_type(tp)
                .map_or_else(|| tp.clone(), |(_, p)| p)
        };
        // A slot a partial name made `τ?` (`@FR-P-Alt-Diff`, the caller) is
        // still `τ` to the patterns that bind it — pass 2 sees pass 1's
        // widening — so either spelling of the slot may match the field.
        let widened_from = match shared_ty {
            Type::Optional(inner) => Some(inner.as_ref()),
            _ => None,
        };
        let fits = |slot: &Type| match_arm_types_unify(&bound_ty(slot), &bound_ty(field_type));
        let slot_ty = widened_from
            .filter(|inner| !fits(shared_ty) && fits(inner))
            .unwrap_or(shared_ty);
        let ok = fits(slot_ty);
        if !ok && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "multi-pattern arm: capture '{}' is {} in this pattern but {} in the first — every listed pattern must bind the same captures at the same type",
                field_name,
                field_type.source_name(&self.data),
                slot_ty.source_name(&self.data)
            );
        }
        ok
    }

    /// A later listed pattern's field sub-pattern captures join the arm's names like a field
    /// capture: a name no earlier pattern bound is a new shared slot (`τ?` by the caller); a
    /// name one did is copied into that slot, which keeps the name.
    fn join_later_sub_pattern_captures(
        &mut self,
        aliases: &mut Vec<(String, Option<u16>)>,
        sub_binds: &mut Vec<Value>,
        shared: &mut std::collections::HashMap<String, (u16, Type)>,
        stmts: &mut Vec<Value>,
        bound: &mut HashSet<String>,
    ) {
        stmts.append(sub_binds);
        let mut copied: HashSet<String> = HashSet::new();
        for (name, _) in aliases.iter() {
            let v_nr = self.vars.var(name);
            if v_nr == u16::MAX {
                continue;
            }
            let tp = self.vars.tp(v_nr).clone();
            if let Some((slot, slot_ty)) = shared.get(name).cloned() {
                self.shared_slot_accepts(name, &tp, &slot_ty);
                stmts.push(v_set(slot, Value::Var(v_nr)));
                self.name_capture(name, slot);
                copied.insert(name.clone());
            } else {
                shared.insert(name.clone(), (v_nr, tp));
            }
            bound.insert(name.clone());
        }
        // A copied name names its slot again, so its re-point needs no undoing.
        aliases.retain(|(name, _)| !copied.contains(name));
    }

    /// @PLN35 Phase 3 (P-Multi) — parse the `{ field, … }` bindings of a NON-FIRST
    /// pattern in a comma-separated multi-pattern arm, REUSING the first pattern's
    /// capture slots (`shared`: name → (var, type)).  Whichever listed pattern
    /// matches assigns those shared slots from ITS OWN variant offsets, then the
    /// single arm body reads them — so a heap capture inherits the first pattern's
    /// `skip_free` + borrow-dep markings on the shared var for free.
    ///
    /// `@FR-P-Alt-Same` — a name several patterns bind must bind at a compatible type.  A
    /// name no earlier pattern bound gets a slot of its own here, added to `shared` (its old
    /// binding to `name_aliases`); the caller makes every name some pattern lacks `τ?`
    /// (`@FR-P-Alt-Diff`).  Returns the names this pattern bound.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn parse_multi_pattern_extra_bindings(
        &mut self,
        variant_def_nr: u32,
        pattern_name: &str,
        subject_val: &Value,
        shared: &mut std::collections::HashMap<String, (u16, Type)>,
        stmts: &mut Vec<Value>,
        conds: &mut Vec<Value>,
        name_aliases: &mut Vec<(String, Option<u16>)>,
    ) -> HashSet<String> {
        let mut bound: HashSet<String> = HashSet::new();
        self.lexer.token("{");
        while let Some(field_name) = self.lexer.has_identifier() {
            let attr_idx_and_type = {
                let variant_def = self.data.def(variant_def_nr);
                variant_def.attributes[1..]
                    .iter()
                    .enumerate()
                    .find(|(_, a)| a.name == field_name)
                    .map(|(i, a)| (i + 1, a.typedef.clone()))
            };
            match attr_idx_and_type {
                Some((attr_idx, field_type)) => {
                    self.check_visibility("pattern-field", variant_def_nr, attr_idx);
                    // `@FR-P-Point` — a field may itself be a pattern.  In a listed pattern
                    // it TESTS the field and captures nothing, so it becomes this branch's
                    // condition (the caller ANDs it into the branch's own guard).  A
                    // sub-pattern that BINDS a name binds a fresh variable the shared arm body
                    // never reads, so that one is refused by name rather than read wrongly.
                    // A plain lowercase NAME as the sub-pattern is a RENAME (`w: v`): it binds
                    // the capture under that name, and the shared arm body reads it like a
                    // shorthand capture of the first pattern's name.
                    let mut capture = field_name.clone();
                    if self.lexer.has_token(":") {
                        let renamed = self.field_pattern_rename();
                        if let Some(name) = &renamed {
                            capture.clone_from(name);
                        }
                        if renamed.is_none() {
                            let field_read =
                                self.get_field(variant_def_nr, attr_idx, subject_val.clone());
                            let mut sub_binds: Vec<Value> = Vec::new();
                            let mut aliases: Vec<(String, Option<u16>)> = Vec::new();
                            if let Some(c) = self.parse_field_sub_pattern(
                                field_read,
                                &field_type,
                                &mut sub_binds,
                                conds,
                                &mut aliases,
                            ) {
                                conds.push(c);
                            }
                            self.join_later_sub_pattern_captures(
                                &mut aliases,
                                &mut sub_binds,
                                shared,
                                stmts,
                                &mut bound,
                            );
                            self.pattern_binds_pending.append(&mut aliases);
                            if !self.lexer.has_token(",") {
                                break;
                            }
                            continue;
                        }
                    }
                    if let Some((var_nr, shared_ty)) = shared.get(&capture).cloned() {
                        let ok = self.shared_slot_accepts(&capture, &field_type, &shared_ty);
                        // Skip the assignment into the shared slot on a confirmed
                        // type mismatch — a `text`→`integer` store is incoherent and
                        // the arm never runs (compile fails).  First pass still binds
                        // so the two-pass shapes agree.
                        if ok || self.first_pass {
                            let field_read = self.pattern_field_value(
                                variant_def_nr,
                                attr_idx,
                                subject_val.clone(),
                            );
                            stmts.push(v_set(var_nr, field_read));
                        }
                        bound.insert(capture.clone());
                    } else {
                        let v_nr = self.bind_match_field_capture(
                            variant_def_nr,
                            attr_idx,
                            &capture,
                            &field_type,
                            subject_val,
                            stmts,
                            name_aliases,
                        );
                        if v_nr != u16::MAX {
                            let tp = self.vars.tp(v_nr).clone();
                            shared.insert(capture.clone(), (v_nr, tp));
                            bound.insert(capture.clone());
                        }
                    }
                }
                None => {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "variant {} has no field '{}'",
                            pattern_name,
                            field_name
                        );
                    }
                }
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
        bound
    }

    /// Parse an optional `if <expr>` guard clause.
    pub(super) fn parse_optional_guard(&mut self) -> Option<Value> {
        if self.lexer.has_token("if") {
            let at = *self.lexer.pos();
            let mut guard_code = Value::Null;
            let guard_type = self.expression(&mut guard_code);
            self.record_guard(&guard_code, at);
            if !self.first_pass && guard_type != Type::Boolean {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "guard must be boolean, got {}",
                    guard_type.source_name(&self.data)
                );
            }
            Some(guard_code)
        } else {
            None
        }
    }

    /// Parse a sub-pattern in a match field position (L2).
    /// Given a field value expression and its type, returns a boolean condition; a nested
    /// struct-enum sub-pattern also pushes its payload bindings through the passed-in
    /// `arm_stmts` / `field_conditions` / `name_aliases` (the recursive pattern path).
    /// Handles: enum variant names (plain AND struct-enum, nested), scalar literals, ranges,
    /// `_` (wildcard).
    #[expect(
        clippy::too_many_lines,
        reason = "one recursive sub-pattern grammar: each kind (variant, struct-enum, literal, range, wildcard) pushes into the same arm_stmts, field_conditions and name_aliases"
    )]
    pub(super) fn parse_field_sub_pattern(
        &mut self,
        field_val: Value,
        field_type: &Type,
        arm_stmts: &mut Vec<Value>,
        field_conditions: &mut Vec<Value>,
        name_aliases: &mut Vec<(String, Option<u16>)>,
    ) -> Option<Value> {
        // `@FR-P-Point` — a field typed by a struct or enum declared BELOW its holder is the
        // forward stub `Unknown(T)` until the passes resolve it, while `T` itself is already
        // declared when the pattern is read.  Read as `T` here, so pass 1 takes the path
        // pass 2 takes: else pass 1 parsed the sub-pattern as a scalar test (a struct
        // construction, which refused `Pt { x: 0, y }`) and left a pattern temp of the
        // record's type that pass 2 then reused for a scalar (loft#2463).
        let forward = self.forward_field_type(field_type);
        let field_type = forward.as_ref().unwrap_or(field_type);
        // A struct not declared yet when pass 1 reads the pattern has no fields to ask: pass 1
        // steps over `S { … }` and pass 2, where `S` is declared, parses it.
        if self.undeclared_struct_pattern(field_type) {
            self.lexer.has_identifier();
            self.skip_braced_block();
            return None;
        }
        // `@FR-P-Alt` at a field or tuple-element position — `Box { s: Circle { r } | Square
        // { r }, n }`, `(Circle { r } | Square { r }, k)`: the value matches when any
        // alternative's tag does, each name is read from whichever variant matched, `τ?` where
        // only some bind it (the slice element's alternation, without its parentheses).  The
        // names borrow what the value borrows (`match_borrow_source`, a tuple member's own
        // source included).  A PLAIN enum's `Red | Green` is the value-test loop below.  The
        // alternation commits to its first match: an arm guard that rejects it rejects the arm,
        // with no retry of the next alternative here (@C162).
        if let Some((e_nr, true)) = self.pattern_variant_enum(field_type)
            && self.sub_pattern_lists_alternatives()
        {
            let src = self.match_borrow_source(&field_val);
            self.parse_slice_alternation_element(
                e_nr,
                &field_val,
                src,
                arm_stmts,
                field_conditions,
                false,
            );
            return None;
        }
        // @PLN35 L2 — Struct-enum field: the sub-pattern is `Variant { subfields }` (or a bare
        // `Variant`).  Tag-test the field's discriminant and recurse into the variant's payload
        // bindings — the same tag-test + payload-bind a top-level struct-enum arm emits, applied to
        // the field-read value (`OpEqInt(OpConvIntFromEnum(OpGetEnum(field_val, 0)), disc)`).
        if let Some((e_nr, true)) = self.pattern_variant_enum(field_type)
            && let Some(name) = self.lexer.has_identifier()
        {
            if name == "_" {
                return None;
            }
            let disc = if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&name) {
                if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                    i32::from(nr)
                } else {
                    0
                }
            } else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        name,
                        self.data.def(e_nr).name()
                    );
                }
                return None;
            };
            let get_enum = self.cl("OpGetEnum", &[field_val.clone(), Value::Int(0)]);
            let disc_expr = self.cl("OpConvIntFromEnum", &[get_enum]);
            let tag_test = self.cl("OpEqInt", &[disc_expr, Value::Int(disc)]);
            // Nested payload: `Variant { subfields }` binds the variant's fields from field_val.
            if self.lexer.peek_token("{") {
                let variant_def_nr = self.data.variant_of(e_nr, &name);
                self.parse_match_enum_field_bindings(
                    variant_def_nr,
                    &name,
                    &field_val,
                    arm_stmts,
                    field_conditions,
                    name_aliases,
                );
            }
            return Some(tag_test);
        }
        // Enum field: the sub-pattern is a variant name (or `_`).
        if let Some((e_nr, false)) = self.pattern_variant_enum(field_type)
            && let Some(name) = self.lexer.has_identifier()
        {
            // Wildcard — no condition.
            if name == "_" {
                return None;
            }
            // Look up variant discriminant.
            let disc = if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&name) {
                if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                    i32::from(nr)
                } else {
                    0
                }
            } else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        name,
                        self.data.def(e_nr).name()
                    );
                }
                return None;
            };
            // The comparison is asked of the enum ITSELF, never of a `τ?` wrapper around it:
            // `(L-Null)` gives the nullable spelling the same byte, and absence — 0 or the
            // 255 sentinel — is no variant, so a nullable field simply fails every arm it is
            // not.  Asking the wrapper looked for an operator on `Color?` and `Color?` and
            // refused the program (loft#1410); a dense field's `base()` is itself, so the
            // emission there is unchanged.
            let cmp_tp = field_type.base().clone();
            // Build equality: field_val == Enum(disc)
            let variant_val = Value::Enum(disc as u8, e_nr as u16);
            let mut cond = Value::Null;
            self.call_op(
                &mut cond,
                "==",
                &[field_val.clone(), variant_val],
                &[cmp_tp.clone(), cmp_tp.clone()],
            );
            // or-pattern: Paid | Refunded
            while self.lexer.has_token("|") {
                if let Some(next_name) = self.lexer.has_identifier() {
                    let next_disc = if let Some(a_nr) =
                        self.data.def(e_nr).attr_names.get(&next_name)
                    {
                        if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                            i32::from(nr)
                        } else {
                            0
                        }
                    } else {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "'{}' is not a variant of {}",
                                next_name,
                                self.data.def(e_nr).name()
                            );
                        }
                        0
                    };
                    let next_variant = Value::Enum(next_disc as u8, e_nr as u16);
                    let mut next_cond = Value::Null;
                    self.call_op(
                        &mut next_cond,
                        "==",
                        &[field_val.clone(), next_variant],
                        &[cmp_tp.clone(), cmp_tp.clone()],
                    );
                    // OR: if first matches → true, else check next.
                    cond = v_if(cond, Value::Boolean(true), next_cond);
                }
            }
            return Some(cond);
        }
        // A plain STRUCT field — inline or through `reference<S>` — matched by the struct's own
        // pattern `S { f: p, g }`, the spelling a top-level arm over `S` takes
        // ([`Self::parse_match_struct_arm`]).  It has no tag, so it adds only its fields'
        // conditions and bindings.  Unrecognised, the pattern fell to the scalar path below,
        // which BUILT an `S` and tested that record: the arm never matched and `--native`
        // refused its own output (loft#1870).
        // A `S?` field holds no record when it is null, so its struct pattern FAILS there:
        // the pattern then also asks for presence — read through the tag of an inline
        // `__nullable<S>` slot, or of the pointer.  Unasked, a null field matched as a record
        // of nulls, and the arm counted as irrefutable.
        let (rec_val, rec_type) = match self.tagged_pointer_type(field_type) {
            Some((syn, pointer)) => (
                self.emit_nullable_slot_read(syn, field_val.clone(), field_type),
                pointer,
            ),
            None => (field_val.clone(), field_type.clone()),
        };
        if let Type::Reference(s_nr, _) = rec_type.peel_link()
            && self.data.def_type(*s_nr) == DefType::Struct
            && matches!(&self.lexer.peek().has, LexItem::Identifier(id) if *id == self.data.def(*s_nr).name())
        {
            let s_nr = *s_nr;
            self.lexer.has_identifier();
            self.parse_struct_sub_pattern(
                s_nr,
                &rec_val,
                arm_stmts,
                field_conditions,
                name_aliases,
            );
            if !rec_type.peel_optional().1 {
                return None;
            }
            let absent = self.cl("OpRefIsNull", &[rec_val]);
            return Some(self.cl("OpNot", &[absent]));
        }
        // Wildcard for non-enum fields.
        if self.lexer.has_wildcard() {
            return None;
        }
        // Scalar field: store in a temp and test it against each alternative in turn
        // (`@FR-P-Alt` — `S { n: 1 | 2 }`: the first that matches wins, on the one read).
        let tmp = self.create_unique("fp_subj", field_type);
        self.vars.defined(tmp);
        let mut cond = self.scalar_field_test(field_type, tmp);
        while self.lexer.has_token("|") {
            let next = self.scalar_field_test(field_type, tmp);
            cond = v_if(cond, Value::Boolean(true), next);
        }
        Some(v_block(
            vec![v_set(tmp, field_val), cond],
            Type::Boolean,
            "field_sub",
        ))
    }

    /// On pass 1, is the parser at `S {` for a type `S` that is still a forward stub — named
    /// above its declaration, so its fields are not known yet?
    pub(super) fn undeclared_struct_pattern(&self, tp: &Type) -> bool {
        // A `S?` element or field is the tagged `__nullable<S>` from pass 1 on, its payload a
        // reference to the stub.
        let pointer = self.tagged_pointer_type(tp).map(|(_, pointer)| pointer);
        let (Type::Unknown(nr) | Type::Reference(nr, _)) = pointer.as_ref().unwrap_or(tp).base()
        else {
            return false;
        };
        self.first_pass
            && *nr != 0
            && self.data.def_type(*nr) == DefType::Unknown
            && matches!(&self.lexer.peek().has, LexItem::Identifier(id) if *id == self.data.def(*nr).name())
    }

    /// The declared type a forward stub field type `Unknown(T)` (or `Unknown(T)?`) names, once
    /// `T` is a declared struct or enum; `None` for every other type.
    pub(super) fn forward_field_type(&self, tp: &Type) -> Option<Type> {
        let optional = matches!(tp, Type::Optional(_));
        let Type::Unknown(nr) = tp.base() else {
            return None;
        };
        if *nr == 0 || !matches!(self.data.def_type(*nr), DefType::Struct | DefType::Enum) {
            return None;
        }
        let resolved = self.data.def(*nr).returned.clone();
        if matches!(resolved.base(), Type::Unknown(_)) {
            return None;
        }
        Some(if optional {
            Type::Optional(Box::new(resolved))
        } else {
            resolved
        })
    }

    /// One alternative of a scalar field pattern, tested against the field's value in `tmp`: a
    /// range or a `null` pattern is its own test, a literal is compared with `==`.
    fn scalar_field_test(&mut self, field_type: &Type, tmp: u16) -> Value {
        let (pat, pat_type) = self.parse_match_pattern(field_type, tmp);
        // A pattern that is already a test answers a boolean, nullable or not.
        if matches!(pat_type.base(), Type::Boolean) || matches!(pat, Value::Block(_)) {
            return pat;
        }
        let mut eq = Value::Null;
        self.call_op(
            &mut eq,
            "==",
            &[Value::Var(tmp), pat],
            &[field_type.clone(), field_type.clone()],
        );
        eq
    }

    /// One operand of a scalar pattern — a literal, a constant, a range bound — parsed above the
    /// `|` level: in a pattern `|` separates alternatives (`@FR-P-Alt`), so `1..3 | 7` is a range
    /// and a second alternative, never `1..(3 | 7)` (loft#2324).
    fn pattern_operand(&mut self, val: &mut Value) -> Type {
        let mut parent_tp = Type::Null;
        self.parse_operators(
            &Type::Unknown(0),
            val,
            &mut parent_tp,
            super::PATTERN_OPERAND_LEVEL,
        )
    }

    /// Parse a match pattern literal (integer, float, text, boolean) and optionally
    /// a range suffix `..` or `..=`. Returns the pattern Value and its type.
    #[expect(
        clippy::too_many_lines,
        reason = "one literal grammar: each literal kind and its optional range suffix share the type check and the diagnostics"
    )]
    fn parse_match_pattern(&mut self, subject_type: &Type, subject_var: u16) -> (Value, Type) {
        // INC#31: reject open-start ranges (`..hi =>`) in match arms with a
        // useful diagnostic.  The range-pattern codegen further down assumes
        // both `lo` and `hi` are real values — an absent `lo` would be
        // silently encoded as Value::Null and either never match
        // (interpreter) or crash native codegen (E0308: `()` vs i32).
        if self.lexer.peek_token("..") {
            // About the token the parser is HOLDING — the `..` that opens the pattern,
            // consumed just below.  So this site names its position rather than taking
            // `report_pos`'s consumed-source default, which is the `{` of the `match`
            // on the line above.
            let at = self.lexer.peek().clone();
            self.lexer.specific(
                &at,
                Level::Error,
                "open-ended range pattern `..hi` is not supported in match arms — \
                 write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
                 or use a guard like `n if n < hi`",
            );
            // Consume the `..` so the rest of the arm parses cleanly.
            self.lexer.token("..");
            self.lexer.has_token("=");
            let mut hi = Value::Null;
            self.expression(&mut hi);
            return (Value::Boolean(false), Type::Boolean);
        }
        // @FR-G-Pat-Group — a `( … )` in a pattern is a GROUP: `(2 | 3)` is the alternation and
        // `(2..=5)` the range, one condition either way.  Handed to `expression` it was a value:
        // `2 | 3` the bitwise or (3), and a parenthesised range never finished parsing.
        if self.lexer.has_token("(") {
            let mut cond = Value::Null;
            loop {
                let (pat, _) = self.parse_match_pattern(subject_type, subject_var);
                let mut one = Value::Null;
                self.build_scalar_cond(&mut one, subject_var, subject_type, pat);
                cond = if matches!(cond, Value::Null) {
                    one
                } else {
                    v_if(cond, Value::Boolean(true), one)
                };
                if !self.lexer.has_token("|") {
                    break;
                }
            }
            self.lexer.token(")");
            return (
                v_block(vec![cond], Type::Boolean, "or_pattern"),
                Type::Boolean,
            );
        }
        let pat_pos = *self.lexer.pos();
        let mut lit = Value::Null;
        let negate = self.lexer.has_token("-");
        let lit_type = if let Some(n) = self.lexer.has_integer() {
            let v = n as i32;
            lit = Value::Int(if negate { -v } else { v });
            Type::Integer(IntegerSpec::signed32())
        } else if let Some(n) = self.lexer.has_long() {
            let v = n as i64;
            lit = Value::Long(if negate { -v } else { v });
            crate::data::I64.clone()
        } else if let Some(n) = self.lexer.has_float() {
            lit = Value::Float(if negate { -n } else { n });
            Type::Float
        } else if let Some(s) = self.lexer.has_cstring() {
            lit = Value::Text(s);
            Type::Text(Deps::none())
        } else if let Some(c) = self.lexer.has_char() {
            lit = self.cl("OpConvCharacterFromInt", &[Value::Int(c as i32)]);
            Type::Character
        } else if self.lexer.has_token("true") {
            lit = Value::Boolean(true);
            Type::Boolean
        } else if self.lexer.has_token("false") {
            lit = Value::Boolean(false);
            Type::Boolean
        } else {
            // A constant or another operand (`@FR-P-Point`): read above the `|` level, which
            // separates this pattern from the next alternative (loft#2324), and negated when a
            // `-` was written before it (loft#2323).
            let tp = self.pattern_operand(&mut lit);
            if negate && !self.first_pass {
                lit = self.single_op("-", lit, tp.clone());
            }
            tp
        };
        // #493 — a pattern whose type cannot convert to the subject type (a text
        // literal against an integer subject, say) can never match.  Track it and
        // emit a dead `false` condition below so control falls through to the
        // wildcard, instead of a type-mismatched comparison (`OpEqInt(int,
        // "text")`) that pushes a 16 B Str into an 8 B slot — stack corruption
        // that trips the generate_call width assert under debug-assertions.
        // Preserves the existing lenient "silently doesn't match" behaviour.
        let incompatible = !self.first_pass
            && lit_type != Type::Null
            && !lit_type.is_same(subject_type)
            && !self.can_convert(&lit_type, subject_type);
        // Plan-07 phase 6 — a pattern whose type can never match the subject is a
        // static type error, not a silently-dead arm (a text literal against an
        // integer subject is almost always a typo).  Report it, then fall through
        // to the dead-`false` recovery below so codegen stays width-safe (#493).
        if incompatible {
            diagnostic_at!(
                self.lexer,
                &pat_pos,
                Level::Error,
                "cannot match {} against pattern of type {}",
                subject_type.source_name(&self.data),
                lit_type.source_name(&self.data)
            );
        }
        // check for range pattern `lo..hi` or `lo..=hi`.
        if self.lexer.has_token("..") {
            let inclusive = self.lexer.has_token("=");
            // INC#31: reject open-end range `lo..` in match arms — same
            // silent-never-matches / native-codegen-crash trap as open-start.
            if self.lexer.peek_token("=>")
                || self.lexer.peek_token("|")
                || self.lexer.peek_token("if")
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "open-ended range pattern `lo..` is not supported in match arms — \
                     write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
                     or use a guard like `n if n >= lo`"
                );
                return (Value::Boolean(false), Type::Boolean);
            }
            let mut hi = Value::Null;
            let hi_neg = self.lexer.has_token("-");
            let hi_tp = self.pattern_operand(&mut hi);
            if hi_neg && !self.first_pass {
                hi = self.single_op("-", hi, hi_tp);
            }
            let mut lo_cond = Value::Null;
            self.call_op(
                &mut lo_cond,
                "<=",
                &[lit, Value::Var(subject_var)],
                &[subject_type.clone(), subject_type.clone()],
            );
            let mut hi_cond = Value::Null;
            self.call_op(
                &mut hi_cond,
                if inclusive { "<=" } else { "<" },
                &[Value::Var(subject_var), hi],
                &[subject_type.clone(), subject_type.clone()],
            );
            let range_cond = v_if(lo_cond, hi_cond, Value::Boolean(false));
            // An incompatible pattern type can never match (see above): the range
            // bounds were consumed for a clean parse, but the condition is dead.
            let cond = if incompatible {
                Value::Boolean(false)
            } else {
                range_cond
            };
            (
                v_block(vec![cond], Type::Boolean, "range_pattern"),
                Type::Boolean,
            )
        } else if incompatible {
            (
                v_block(vec![Value::Boolean(false)], Type::Boolean, "range_pattern"),
                Type::Boolean,
            )
        } else {
            (lit, lit_type)
        }
    }

    /// Parse a match expression over a scalar (integer, text, boolean, etc.).
    /// Builds an if/else chain: `if subject == lit1 { arm1 } else if subject == lit2 { arm2 } else { wildcard }`
    #[expect(
        clippy::too_many_lines,
        reason = "one if/else chain over the arms: each arm's literal, guard and body extend the chain the next arm continues"
    )]
    pub(super) fn parse_scalar_match(
        &mut self,
        subject: Value,
        subject_type: &Type,
        code: &mut Value,
        match_pos: &crate::lexer::Position,
    ) -> Type {
        // Store subject in a temp var to avoid re-evaluation.
        let v = self.create_match_subject(subject_type);
        self.vars.defined(v);

        self.lexer.token("{");

        // Collect arms: (literal_value, arm_code, arm_type, optional guard)
        let mut arms: Vec<(Option<Value>, Value, Type, Option<Value>)> = Vec::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;
        let saved_null_arms = std::mem::replace(&mut self.null_literal_arms, 0);

        loop {
            if self.lexer.peek_token("}") {
                break;
            }

            // Parse pattern: literal, `true`, `false`, `_`, `name @ pattern`, or string.
            let mut pattern_val: Option<Value> = None;
            let mut is_wildcard = false;
            let mut arm_bindings: Vec<Value> = Vec::new();

            // null pattern — matches when subject is null.
            if self.lexer.has_token("null") {
                let mut null_cond = Value::Null;
                self.call_op(
                    &mut null_cond,
                    "!",
                    &[Value::Var(v)],
                    std::slice::from_ref(subject_type),
                );
                // Wrap as a Block so build_scalar_chain recognizes it as a pre-built condition.
                pattern_val = Some(v_block(vec![null_cond], Type::Boolean, "null_pattern"));
            // Check for wildcard `_` or binding `name @ pattern`.
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    is_wildcard = true;
                } else if self.lexer.has_token("@") || self.lexer.has_token(":") {
                    // @FR-P-Cap at the arm root — `name: pattern` (and the older `name @ pattern`)
                    // binds the subject and the sub-pattern still tests it; `name: _` is the
                    // catch-all that binds it.
                    let bind_nr = self.pattern_binding(&id, subject_type);
                    self.vars.defined(bind_nr);
                    arm_bindings.push(v_set(bind_nr, Value::Var(v)));
                    if matches!(&self.lexer.peek().has, crate::lexer::LexItem::Identifier(n) if n == "_")
                    {
                        self.lexer.has_identifier();
                        is_wildcard = true;
                    } else {
                        let (pat, _) = self.parse_match_pattern(subject_type, v);
                        pattern_val = Some(pat);
                    }
                } else {
                    // Bare identifier without `@` — wildcard binding (binds subject to name).
                    let bind_nr = self.pattern_binding(&id, subject_type);
                    self.vars.defined(bind_nr);
                    arm_bindings.push(v_set(bind_nr, Value::Var(v)));
                    is_wildcard = true;
                }
            } else {
                let (pat, _) = self.parse_match_pattern(subject_type, v);
                pattern_val = Some(pat);
            }

            // or-patterns in scalar match — `1 | 2 | 3 => ...`
            while self.lexer.has_token("|") && !is_wildcard {
                let (next_pat, _) = self.parse_match_pattern(subject_type, v);
                if let Some(prev) = pattern_val.take() {
                    // Combine: build equality condition for prev, equality for next,
                    // then OR them: If(prev_eq, true, next_eq).
                    let mut prev_cond = Value::Null;
                    self.build_scalar_cond(&mut prev_cond, v, subject_type, prev);
                    let mut next_cond = Value::Null;
                    self.build_scalar_cond(&mut next_cond, v, subject_type, next_pat);
                    let or_cond = v_if(prev_cond, Value::Boolean(true), next_cond);
                    pattern_val = Some(v_block(vec![or_cond], Type::Boolean, "or_pattern"));
                }
            }

            let mut guard_opt = self.parse_optional_guard();

            // Only mark exhaustive if wildcard has no guard.
            if is_wildcard && guard_opt.is_none() {
                has_wildcard = true;
            }

            self.expect_match_arm_arrow();
            let mut arm_code = Value::Null;
            let arm_expected = self.match_arm_expected(&result_type);
            let facts = self.assume_guard(guard_opt.as_ref());
            let mut arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_code);
            self.forget_facts(facts);
            // A `null`-first arm must NOT pin the result to `Null` — promote to
            // the first CONCRETE arm's type (else `match c { false => null, true
            // => S{…} }` resolves to `Null`, `build_scalar_chain` can't type the
            // null sentinel, and the value arm returns null — silently wrong).
            // loft#978 — every arm can deliver this match's value, so the result carries
            // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
            // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
            result_type = self.join_arm_into(&result_type, &arm_code, &arm_type);
            self.match_void_arm |= matches!(arm_type.base(), Type::Void);
            if Self::arm_settles_match(&result_type, &arm_type) {
                result_type = arm_type.clone();
            } else if let Some(joined) =
                self.join_null_tuple_arm(&result_type, &mut arm_code, &mut arm_type)
            {
                // loft#1682 — the scalar arms hold `(pattern, code, type, guard)`.
                for prev in &mut arms {
                    if self.arm_takes_join(&prev.2, &joined) {
                        let prev_orig = prev.2.clone();
                        self.convert_arm_tail(&mut prev.1, &prev_orig, &joined);
                        prev.2 = joined.clone();
                    }
                }
                result_type = joined;
            }
            // P209 — when the arm has both a guard and pattern bindings
            // (e.g. `x if x < 0 => …`), the guard must see the bound
            // variable.  Prepend the binding assignments to the guard
            // expression so the bound name is initialised before the
            // guard reads it.  Without this the guard saw the
            // uninitialised slot (typically 0), causing `x if x < 0`
            // to mis-fire and either skip the arm (interp) or fall
            // through to a sibling guard (`x == 0`) silently.  The
            // enum-variant struct-field path at the call site of
            // `build_scalar_chain` already wraps guards this way.
            if !arm_bindings.is_empty()
                && let Some(guard) = guard_opt.take()
            {
                let mut stmts = arm_bindings.clone();
                stmts.push(guard);
                guard_opt = Some(v_block(stmts, Type::Boolean, "binding_guard"));
            }
            // prepend any binding assignments (from `name @ pattern` or bare `name`)
            // to the arm body so the variable is assigned before the body executes.
            if !arm_bindings.is_empty() {
                arm_bindings.push(arm_code);
                arm_code = v_block(arm_bindings, arm_type.clone(), "binding_arm");
            }
            arms.push((pattern_val, arm_code, arm_type, guard_opt));
            if has_wildcard {
                self.lexer.has_token(","); // optional trailing comma
                // The enum path's twin — a total `_` matches everything, so an arm after it can
                // never be selected.  Breaking straight to the closing-brace expectation
                // reported "Expect token }" and then cascaded into four more errors about the
                // rest of the line, none of which named the wildcard.
                if !self.lexer.peek_token("}") {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "a `_` arm matches everything, so this arm can never be selected \
                             — move `_` to the end"
                        );
                    }
                    continue;
                }
                break;
            }
            self.match_arm_separator();
        }
        self.lexer.token("}");

        // `(M-Exhaust)` is stated for an enum, and a boolean is the one scalar whose domain
        // a match can spell out in full: `true` and `false`, each an unguarded arm, leave
        // nothing for a fall-through to answer.  Without this the chain ended in the typed
        // null every wildcard-less scalar match carries for the value no arm matched, and
        // the join read that leaf as a nullable arm — `-> text { match w { true => …,
        // false => … } }` warned nullable-into-non-null on both backends while the same
        // choice spelled as an `if` was quiet (loft#1343).  The last arm becomes the
        // fallback, exactly as a trailing `_` would.
        let bool_exhaustive = *subject_type == Type::Boolean
            && !has_wildcard
            && arms.iter().all(|(_, _, _, guard)| guard.is_none())
            && [true, false].iter().all(|b| {
                arms.iter()
                    .any(|(pat, _, _, _)| matches!(pat, Some(Value::Boolean(x)) if x == b))
            });
        // `(M-Total)` — a SCALAR subject's coverage is not decidable, so a match with no total
        // arm MAY select nothing: as an expression it answers null and its type says so (`τ?`),
        // as a statement it does nothing.  That is the rule's answer, not a fault — but a hole
        // the author did not mean is invisible at the match itself (a lexer's `match c { 'a' =>
        // …, 'b' => … }` skips every other character in silence), so it is said there, as
        // ADVICE: the result is what the language promises, and the type carries the null to
        // whoever reads the value.  `LOFT_NO_SCALAR_HOLE` keeps a deliberate hole quiet
        // (loft-lang/plans#202 P3).
        if !self.first_pass
            && !has_wildcard
            && !bool_exhaustive
            && matches!(
                subject_type.base(),
                Type::Integer(_) | Type::Character | Type::Text(_)
            )
            && crate::keys::scalar_hole_lint_enabled()
        {
            let names = arms.len();
            let plural = if names == 1 { "" } else { "s" };
            let subject_name = subject_type.source_name(&self.data);
            let what = if matches!(result_type.base(), Type::Void | Type::Null) {
                "the statement does nothing".to_string()
            } else {
                format!(
                    "the match answers null (its type is `{}?`)",
                    result_type.base().source_name(&self.data)
                )
            };
            diagnostic_at!(
                self.lexer,
                match_pos,
                Level::Advice,
                code = "match-scalar-hole",
                "this `match` over {subject_name} names {names} value{plural} and no `_` arm — \
                 a value none of them names selects no arm, and {what}"
            );
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "add a `_ =>` arm for the values the arms do not name".to_string(),
                condition: Some(
                    "every value the arms do not name should take that arm's answer".to_string(),
                ),
                edit: None,
                concept: "pattern matching",
                concept_ref: "@F29",
            });
        }
        let has_null_arm = std::mem::replace(&mut self.null_literal_arms, saved_null_arms) > 0;
        let chain = self.build_scalar_chain(
            v,
            subject_type,
            has_wildcard || bool_exhaustive,
            &result_type,
            arms,
        );
        // A struct, collection or enum result with a literal `null` arm; the scalar result is
        // widened by the caller (`dn1_widen_branch_null`), as it always was.
        let result_type = if has_null_arm
            && crate::keys::pln25_dn1_enabled()
            && !Self::is_non_null_scalar(&result_type)
            && Self::null_arm_widens(&result_type)
        {
            Type::optional(result_type)
        } else {
            result_type
        };
        *code = v_block(
            vec![v_set(v, subject), chain],
            result_type.clone(),
            "scalar_match",
        );
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_type);
        result_type
    }

    /// Parse a match expression over a vector subject.
    /// Slice patterns: `[a, b] =>`, `[first, ..] =>`, `[.., last] =>`, `_ =>`.
    /// Each arm generates a length check and element bindings.
    /// @PLN35 PC1 — is `subject_type` a CURSOR-shaped struct: a `vector<T>` source field + an
    /// integer field named `pos`?  Returns `(struct_def, source_field_idx, pos_field_idx, T)`.
    /// Matching such a subject prefix-consumes; any other struct falls through to the struct handler.
    pub(super) fn cursor_shape(&self, subject_type: &Type) -> Option<(u32, usize, usize, Type)> {
        let d_nr = match subject_type {
            Type::Reference(d, _) if self.data.def_type(*d) == DefType::Struct => *d,
            _ => return None,
        };
        let attrs = self.data.def(d_nr).attributes();
        let mut source: Option<(usize, Type)> = None;
        let mut pos: Option<usize> = None;
        for (i, a) in attrs.iter().enumerate() {
            if a.constant {
                continue;
            }
            if let Type::Vector(elem, _) = &a.typedef {
                if source.is_none() {
                    source = Some((i, (**elem).clone()));
                }
            } else if matches!(a.typedef, Type::Integer(_)) && a.name == "pos" {
                pos = Some(i);
            }
        }
        match (source, pos) {
            (Some((si, elm)), Some(pi)) => Some((d_nr, si, pi, elm)),
            _ => None,
        }
    }

    /// Why the struct `d_nr` is NOT a cursor, spelled for the author — the diagnostic half of
    /// [`Self::cursor_shape`], asked when a `match` arm over a plain struct opens with a slice
    /// pattern, which only a vector or a cursor can wear.
    ///
    /// `cursor_shape` answers `None` for every struct that is not one, and the arm was then
    /// parsed as a STRUCT pattern: the `[` was "expect variant name", the first pass broke out
    /// silently and the run died on a brace it had never reached, so the author was told about
    /// a `}` and never about the field that disqualified the struct (loft#1410).  A `?` on the
    /// source or the position field is the case worth naming, because those two fields still
    /// read like a cursor's.
    pub(super) fn cursor_defect(&self, d_nr: u32) -> String {
        let attrs = self.data.def(d_nr).attributes();
        let field_of = |pick: &dyn Fn(&crate::data::Attribute) -> bool| {
            attrs
                .iter()
                .find(|a| !a.constant && pick(a))
                .map(|a| (a.name.clone(), a.typedef.source_name(&self.data)))
        };
        if field_of(&|a| matches!(a.typedef, Type::Vector(_, _))).is_none() {
            return match field_of(&|a| matches!(a.typedef.base(), Type::Vector(_, _))) {
                Some((name, tp)) => format!(
                    "its `{name}` field is `{tp}`, and a cursor reads from a plain `vector<…>`"
                ),
                None => "it has no `vector<…>` field for the pattern to read".to_string(),
            };
        }
        match field_of(&|a| a.name == "pos") {
            Some((_, tp)) => {
                format!("its `pos` field is `{tp}`, and a cursor's position must be an integer")
            }
            None => "it has no integer `pos` field, so nothing says where the pattern starts"
                .to_string(),
        }
    }

    /// @PLN35 PC1 — match over a CURSOR: prefix-consume its source from `pos`, advancing `cursor.pos`
    /// by the consumed count on a match.  Reads source + pos into temps, sets `match_cursor` so the
    /// slice machinery goes prefix-relative (`read_slice_elem` offsets by `pos`, the length gate is
    /// `pos + fixed <= len`), runs the normal vector-match over the source, then clears the mode.
    pub(super) fn parse_cursor_match(
        &mut self,
        subject: Value,
        cursor_def: u32,
        source_field: usize,
        pos_field: usize,
        elm_tp: &Type,
        code: &mut Value,
    ) -> Type {
        // Use the subject's OWN var as the cursor — a struct is a DbRef, so advancing its `pos`
        // must reach the caller's cursor (same rule the struct-match path follows).  Copy only a
        // non-var subject (a temporary cursor's advance is then moot, which is fine).
        let (cursor_var, copy) = if let Value::Var(cv) = &subject {
            (*cv, false)
        } else {
            let t = self.create_unique("cursor", &Type::Reference(cursor_def, Deps::none()));
            self.vars.defined(t);
            (t, true)
        };
        // `cursor.src` is a vector living INSIDE the caller-owned cursor record, so the read
        // is a BORROWED view (Deps::frame1(cursor_var)), not an owned temporary. It must NOT
        // be freed: freeing it frees the cursor's whole record — a use-after-free that lets a
        // later allocation reuse the slot and silently corrupt the caller's cursor (surfaced
        // by an arm that returns a freshly-built struct then rebinds it). Mirrors the heap-
        // capture borrowed-VIEW path (skip_free + a borrow dep on the subject source).
        let borrowed_vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::frame1(cursor_var));
        let source_var = self.create_unique("cursor_src", &borrowed_vec_tp);
        self.vars.defined(source_var);
        self.vars.set_skip_free(source_var);
        let pos_var = self.create_unique("cursor_pos", &I32);
        self.vars.defined(pos_var);
        let read_source = self.get_field(cursor_def, source_field, Value::Var(cursor_var));
        let read_pos = self.get_field(cursor_def, pos_field, Value::Var(cursor_var));
        let mut setup = Vec::new();
        if copy {
            setup.push(v_set(cursor_var, subject));
        }
        setup.push(v_set(source_var, read_source));
        setup.push(v_set(pos_var, read_pos));
        self.match_cursor = Some((cursor_var, cursor_def, pos_field, pos_var));
        // @PLN35 PC5 — an OPTIONAL `farthest: integer` field is a monotonic high-water mark the
        // match maintains at every advance (opt-in; a plain `{ src, pos }` cursor has none).
        self.match_cursor_farthest = self.data.def(cursor_def).attributes().iter().position(|a| {
            !a.constant && matches!(a.typedef, Type::Integer(_)) && a.name == "farthest"
        });
        let mut match_code = Value::Null;
        let result_tp =
            self.parse_vector_match(Value::Var(source_var), &borrowed_vec_tp, &mut match_code);
        self.match_cursor = None;
        self.match_cursor_farthest = None;
        setup.push(match_code);
        *code = v_block(setup, result_tp.clone(), "cursor match");
        result_tp
    }

    /// `f` parsed with no cursor mode (`@FR-P-Cursor`): a cursor match's mode belongs to its
    /// PATTERNS, so a guard or an arm body — where a `match` of its own may be written — is
    /// parsed outside it, and a nested cursor match sets and clears only its own (loft#2322).
    pub(super) fn outside_cursor_mode<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let cursor = self.match_cursor.take();
        let farthest = self.match_cursor_farthest.take();
        let r = f(self);
        self.match_cursor = cursor;
        self.match_cursor_farthest = farthest;
        r
    }

    /// @PLN35 PC2 — is the upcoming slice element `name : rule`, where `rule` names a sub-rule
    /// FUNCTION (`n_<rule>` returning a struct)?  ONE save/revert look-ahead, mirroring
    /// `peek_scalar_type_capture`.  A variant name (`name : Variant`) has no `n_<...>` fn, so this
    /// stays disjoint from the sub-pattern path.  Returns `(name, fn_def_nr, return_type)`.
    pub(super) fn peek_subrule_capture(&mut self) -> Option<(String, u32, Type)> {
        let name = match &self.lexer.peek().has.clone() {
            LexItem::Identifier(id) if id != "_" => id.clone(),
            _ => return None,
        };
        let save = self.lexer.link();
        let mut result = None;
        self.lexer.cont(); // name
        if self.lexer.peek_token(":") {
            self.lexer.cont(); // `:`
            if let LexItem::Identifier(rule) = self.lexer.peek().has.clone() {
                let fn_nr = self.data.def_nr(&format!("n_{rule}"));
                // On the first pass a rule written further down the file is not defined yet.
                // A lower-case name that names nothing else cannot be a variant or a type here,
                // so it is read as that rule: the binding it makes must be the same variable
                // on both passes.
                if fn_nr == u32::MAX
                    && self.first_pass
                    && rule.starts_with(|c: char| c.is_ascii_lowercase())
                    && self.data.def_nr(&rule) == u32::MAX
                {
                    result = Some((name.clone(), u32::MAX, Type::Unknown(0)));
                }
                if fn_nr != u32::MAX {
                    // `@FR-N-Shape` — through the `?`: a sub-rule answers null for "no match",
                    // so `-> N?` is its honest declaration, and `(N-Join)` warns a `-> N` whose
                    // `_ => null` arm says otherwise (loft#1714).  The capture is bound only on
                    // the arm where the rule matched, so it is the non-null `N`.
                    // `@FR-P-Rule` — the rule answers its node, of whatever type: a record, or
                    // an index into an arena the cursor carries.
                    let ret = self.data.def(fn_nr).returned().base().clone();
                    if !matches!(ret, Type::Void | Type::Unknown(_) | Type::Never) {
                        result = Some((name, fn_nr, ret));
                    }
                }
            }
        }
        self.lexer.revert(save);
        result
    }

    /// `@FR-P-Rule` — is the upcoming slice element a sub-rule GROUP `( name : rule )` followed by
    /// `*`, `+` or `?`?  One look-ahead, put back.  `rule` is recognised as
    /// [`Self::peek_subrule_capture`] recognises it.  Returns the capture name, the rule, its
    /// answer type and the quantifier.
    pub(super) fn peek_subrule_group(&mut self) -> Option<(String, u32, Type, char)> {
        if !self.lexer.peek_token("(") {
            return None;
        }
        let save = self.lexer.link();
        self.lexer.cont(); // `(`
        let mut result = None;
        if let Some((name, fn_nr, ret_tp)) = self.peek_subrule_capture() {
            self.lexer.cont(); // name
            self.lexer.cont(); // `:`
            self.lexer.cont(); // rule
            if self.lexer.has_token(")") {
                let quant = ['*', '+', '?']
                    .into_iter()
                    .find(|q| self.lexer.peek_token(&q.to_string()));
                if let Some(q) = quant {
                    result = Some((name, fn_nr, ret_tp, q));
                }
            }
        }
        self.lexer.revert(save);
        result
    }

    /// `@FR-P-Rule` × `@FR-P-Rep` / `@FR-P-Opt` — a sub-rule group of a cursor pattern, `k`
    /// fixed elements after the current segment's base.  ANDed onto the arm's `cond`:
    ///
    /// - `(x: rule)?` binds `x: τ?` — the rule's answer, or null with the cursor unmoved;
    /// - `(xs: rule)*` / `+` binds `xs: vector<τ>` — the rule run while it answers, each answer
    ///   MOVED into `xs`, with an optional separator `*(Sep)` consumed between two answers and
    ///   given back when no answer follows it.  An answer that consumed nothing ends the run
    ///   without being taken, so a rule that can match empty cannot loop.  `+` asks for one.
    ///
    /// The next segment is read from where the group stopped (`seg`).  The capture and the
    /// temps are declared at the match's level (`prechain`), as a single sub-rule's are.
    pub(super) fn parse_subrule_group(
        &mut self,
        subject: &SliceSubject,
        group: &SubruleGroup,
        k: usize,
        consumed: bool,
        cond: &mut Option<Value>,
        prechain: &mut Vec<Value>,
    ) {
        let Some((cursor_var, cursor_def, pos_field, base)) = self.match_cursor else {
            return;
        };
        let SubruleGroup {
            name,
            fn_nr,
            ret_tp,
            quant,
        } = group;
        let (fn_nr, quant) = (*fn_nr, *quant);
        let elm_tp = subject.elm_tp;
        self.lexer.token("(");
        self.lexer.has_identifier(); // name
        self.lexer.token(":");
        let site = self.lexer.peek().position;
        self.lexer.has_identifier(); // rule
        self.lexer.token(")");
        self.lexer.cont(); // the quantifier
        let sep = if quant == '?' {
            None
        } else if let Some((e_nr, _)) = self.pattern_variant_enum(elm_tp) {
            self.parse_repetition_separator(e_nr)
        } else {
            None
        };
        if !self.first_pass && self.context != u32::MAX {
            let consuming = consumed || self.cursor_consumed.contains(&cursor_var);
            self.subrule_edges
                .push((self.context, fn_nr, site, consuming));
        }
        self.slice_subrule = true;
        let seg = self.create_unique("cursor_seg", &I32);
        self.vars.defined(seg);
        prechain.push(v_set(seg, Value::Int(0)));
        let start = self.conv_op(
            "+",
            Value::Var(base),
            Value::Int(k as i32),
            I32.clone(),
            I32.clone(),
        );
        let cursor_tp = self.vars.tp(cursor_var).clone();
        let mut call = Value::Null;
        if fn_nr != u32::MAX {
            self.call_nr(
                &mut call,
                fn_nr,
                &[Value::Var(cursor_var)],
                &[cursor_tp],
                false,
                &[],
                None,
            );
        }
        let mut ops = vec![v_set(seg, start)];
        if quant == '?' {
            // A rule not defined yet (the first pass) has no type to wrap: the binding stays
            // unknown, as a single sub-rule's does, so the second pass can settle it.
            let opt_tp = if fn_nr == u32::MAX {
                Type::Unknown(0)
            } else {
                Type::optional(ret_tp.clone())
            };
            let x = self.pattern_binding(name, &opt_tp);
            self.vars.defined(x);
            let none = self.null_value(ret_tp);
            prechain.push(v_set(x, none));
            let set_pos = self.set_field(
                cursor_def,
                pos_field,
                0,
                Value::Var(cursor_var),
                Value::Var(seg),
            );
            ops.push(set_pos);
            ops.push(v_set(x, call));
            let present = self.subrule_present(fn_nr, x, ret_tp);
            let now = self.get_field(cursor_def, pos_field, Value::Var(cursor_var));
            ops.push(v_if(present, v_set(seg, now), Value::Null));
            ops.push(Value::Boolean(true));
        } else {
            let run = self.subrule_run(subject, group, call, sep.as_ref(), seg, prechain);
            ops.extend(run);
        }
        let test = v_block(ops, Type::Boolean, "sub-rule group");
        *cond = Some(match cond.take() {
            Some(c) => v_if(c, test, Value::Boolean(false)),
            None => test,
        });
        self.match_cursor = Some((cursor_var, cursor_def, pos_field, seg));
    }

    /// `OpFreeRefIfDistinct(r, null)`: the free of a never-free local the code generator keeps
    /// (a plain `OpFreeRef` of one is dropped), a no-op where `r` is null.
    fn free_unless_null(&mut self, r: u16) -> Value {
        let null = self.cl("OpNullRefSentinel", &[]);
        self.cl("OpFreeRefIfDistinct", &[Value::Var(r), null])
    }

    /// The run of `(xs: rule)*` / `+` ([`Self::parse_subrule_group`]): `xs` declared and
    /// minted at the match's level, then the loop that calls the rule from `seg`, takes the
    /// separator between two answers, moves each answer that consumed something into `xs` and
    /// moves `seg` past it.  The ops end with the group's test: true, or `len(xs) > 0` for `+`.
    fn subrule_run(
        &mut self,
        subject: &SliceSubject,
        group: &SubruleGroup,
        call: Value,
        sep: Option<&SepSpec>,
        seg: u16,
        prechain: &mut Vec<Value>,
    ) -> Vec<Value> {
        let Some((cursor_var, cursor_def, pos_field, _)) = self.match_cursor else {
            return Vec::new();
        };
        let (name, fn_nr, ret_tp, quant) = (&group.name, group.fn_nr, &group.ret_tp, group.quant);
        let mut ops = Vec::new();
        let vec_tp = if fn_nr == u32::MAX {
            Type::Unknown(0)
        } else {
            Type::Vector(Box::new(ret_tp.clone()), Deps::none())
        };
        let xs = self.pattern_binding(name, &vec_tp);
        self.vars.defined(xs);
        if fn_nr == u32::MAX {
            // The first pass, before the rule is defined: the binding exists, the run does not.
            ops.push(Value::Boolean(true));
            return ops;
        }
        prechain.extend(self.vector_db(ret_tp, xs));
        let r = self.create_unique("rule_x", ret_tp);
        self.vars.defined(r);
        let held = matches!(
            ret_tp.base(),
            Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
        );
        if held {
            // `r` hands its record over to `xs` (the copy below frees r's store), so its
            // own scope end releases nothing.
            self.vars.set_skip_free(r);
        }
        prechain.push(v_set(r, self.null_value(ret_tp)));
        let ed_nr = self.data.type_def_nr(ret_tp);
        let elm = self.create_unique("rule_elm", &Type::Reference(ed_nr, Deps::none()));
        self.vars.defined(elm);
        self.vars.set_skip_free(elm);
        let mut append = self.append_element_ops(xs, ret_tp, elm, Value::Var(r));
        // `@FR-O-Move` — the copy into `xs` frees the rule's answer only where it is the rule's
        // own.  A rule whose answer is always a store it made (no borrowed return) has the copy
        // free it; one that may answer a record of an argument (`p.nodes[0]`) has its answer
        // released after the copy only where it names none of the arguments' stores — the
        // witnesses the call's free-protection bracket (`n_protect_store_frees`) protects — since
        // the backends differ on whether a borrowed answer bound to `r` is that record or a copy
        // of it.  Freeing a borrowed answer released the caller's whole store.
        // `LOFT_NO_SUBRULE_OWNED_ANSWER=1` frees every answer through the copy, as before.
        let old_way = crate::env_once!(std::env::var_os("LOFT_NO_SUBRULE_OWNED_ANSWER").is_some());
        let owned = held && (!self.data.def(fn_nr).returns_borrowed_view() || old_way);
        if owned {
            let copy_d = self.data.def_nr("OpCopyRecord");
            for op in &mut append {
                if let Value::Call(d, args) = op.unspan_mut()
                    && *d == copy_d
                    && let Some(Value::Int(tp)) = args.get_mut(2).map(Value::unspan_mut)
                {
                    *tp |= i32::from(crate::keys::COPY_FREE_SOURCE);
                }
            }
        }
        // The release of an answer the copy did not free: unguarded for an owned answer, and
        // guarded by each witness for a borrowing rule whose arguments the bracket can name.
        let release = if owned {
            Some(self.free_unless_null(r))
        } else if held {
            let (witnesses, covers_all) =
                crate::use_analysis::protectable_ref_args(&self.data, self.context, &call);
            covers_all.then(|| {
                let mut free = self.free_unless_null(r);
                for w in witnesses.iter().rev() {
                    let distinct = self.cl("OpDistinctStore", &[Value::Var(r), Value::Var(*w)]);
                    free = v_if(distinct, free, Value::Null);
                }
                free
            })
        } else {
            None
        };
        let mut body = Vec::new();
        body.push(self.subrule_separator_step(subject, sep, xs, seg));
        body.push(v_set(r, call));
        let present = self.subrule_present(fn_nr, r, ret_tp);
        let now = self.get_field(cursor_def, pos_field, Value::Var(cursor_var));
        let moved = self.conv_op("<", Value::Var(seg), now, I32.clone(), I32.clone());
        let taken = v_if(present, moved, Value::Boolean(false));
        // An answer that consumed nothing stops the run without reaching the copy: released
        // here as after the copy (a no-op on the null answer).  `LOFT_NO_SUBRULE_STOP_FREE=1`
        // leaves it, as before.
        let stop_free = !crate::env_once!(std::env::var_os("LOFT_NO_SUBRULE_STOP_FREE").is_some());
        let stop = match &release {
            Some(free) if stop_free => Value::Insert(vec![free.clone(), Value::Break(0)]),
            _ => Value::Break(0),
        };
        body.push(v_if(taken, Value::Null, stop));
        body.extend(append);
        if !owned && let Some(free) = release {
            body.push(free);
        }
        let now = self.get_field(cursor_def, pos_field, Value::Var(cursor_var));
        body.push(v_set(seg, now));
        ops.push(v_loop(body, "sub-rule run"));
        if quant == '+' {
            let taken = self.cl("OpLengthVector", &[Value::Var(xs)]);
            ops.push(self.conv_op("<", Value::Int(0), taken, I32.clone(), I32.clone()));
        } else {
            ops.push(Value::Boolean(true));
        }
        ops
    }

    /// The step before each call of a run ([`Self::subrule_run`]): with a separator `*(Sep)`,
    /// the separator between two answers is consumed when present and the run ends when it is
    /// not; before the first answer, and with no separator, the cursor is set to `seg`.
    fn subrule_separator_step(
        &mut self,
        subject: &SliceSubject,
        sep: Option<&SepSpec>,
        xs: u16,
        seg: u16,
    ) -> Value {
        let Some((cursor_var, cursor_def, pos_field, _)) = self.match_cursor else {
            return Value::Null;
        };
        let set_here = self.set_field(
            cursor_def,
            pos_field,
            0,
            Value::Var(cursor_var),
            Value::Var(seg),
        );
        let Some(sep) = sep else {
            return set_here;
        };
        let SliceSubject {
            v,
            elm_tp,
            elm_size,
            ..
        } = *subject;
        let taken = self.cl("OpLengthVector", &[Value::Var(xs)]);
        let some = self.conv_op("<", Value::Int(0), taken, I32.clone(), I32.clone());
        let len_c = self.cursor_len(v);
        let room = self.conv_op("<", Value::Var(seg), len_c, I32.clone(), I32.clone());
        let at_sep = self.sep_match_cond(sep, v, elm_size, elm_tp, Value::Var(seg));
        let is_sep = v_if(room, at_sep, Value::Boolean(false));
        let past = self.conv_op(
            "+",
            Value::Var(seg),
            Value::Int(1),
            I32.clone(),
            I32.clone(),
        );
        let set_past = self.set_field(cursor_def, pos_field, 0, Value::Var(cursor_var), past);
        v_if(some, v_if(is_sep, set_past, Value::Break(0)), set_here)
    }

    /// `@FR-P-Rule` — "the rule answered": a record's `!= null` against the null-ref sentinel
    /// (`OpRefIsNull` tests the store_nr sentinel and misreads a fn-returned null struct on the
    /// interpreter), any other node type's `!= null`.  A rule not defined yet (the first
    /// pass) has no type to test.
    fn subrule_present(&mut self, fn_nr: u32, var: u16, ret_tp: &Type) -> Value {
        if fn_nr == u32::MAX {
            Value::Boolean(true)
        } else if matches!(ret_tp.base(), Type::Reference(..)) {
            // A rule answering `S?` stores its record as `S` does (`@FR-N-Shape`).
            let sentinel = self.cl("OpNullRefSentinel", &[]);
            self.cl("OpNeRef", &[Value::Var(var), sentinel])
        } else if let Some(test) = self.null_test(Value::Var(var), ret_tp, true) {
            // `!= null`, never the value's truth: a rule answering 0 (an arena index, a
            // position) answered (`@FR-P-Rule`).
            test
        } else {
            self.coalesce_not_null(&Value::Var(var), ret_tp)
        }
    }

    /// `@FR-P-Rule` — the fixed elements of the current cursor segment, tested and bound before
    /// a sub-rule ends it: the length gate `base + k <= len`, then each element's condition in
    /// order, ANDed onto `cond`; the bare names bind from `base + i`.  Their reads use the
    /// segment's base, which the next segment replaces, so they are emitted now.
    #[expect(
        clippy::too_many_arguments,
        reason = "the slice pattern's own state, threaded"
    )]
    pub(super) fn flush_cursor_segment(
        &mut self,
        subject: &SliceSubject,
        head: &mut [String],
        seg_start: usize,
        cond: &mut Option<Value>,
        elem_conds: &mut Vec<Value>,
        conds_at_entry: usize,
        bindings: &mut Vec<Value>,
    ) {
        let Some((_, _, _, base)) = self.match_cursor else {
            return;
        };
        let k = (head.len() - seg_start) as i32;
        let SliceSubject {
            v,
            elm_tp,
            elm_size,
            borrow_src,
        } = *subject;
        if k > 0 {
            let need = self.conv_op(
                "+",
                Value::Var(base),
                Value::Int(k),
                I32.clone(),
                I32.clone(),
            );
            let len_call = self.cursor_len(v);
            let ispec = Type::Integer(IntegerSpec {
                min: 0,
                max: 0,
                not_null: false,
                forced_size: None,
            });
            let mut gate = Value::Null;
            self.call_op(&mut gate, "<=", &[need, len_call], &[ispec.clone(), ispec]);
            *cond = Some(match cond.take() {
                Some(c) => v_if(c, gate, Value::Boolean(false)),
                None => gate,
            });
        }
        for ec in elem_conds.drain(conds_at_entry..) {
            *cond = Some(match cond.take() {
                Some(c) => v_if(c, ec, Value::Boolean(false)),
                None => ec,
            });
        }
        for (i, name) in head.iter_mut().enumerate().skip(seg_start) {
            if name == "_" {
                continue;
            }
            let bind_nr = self.pattern_binding(name, elm_tp);
            self.vars.defined(bind_nr);
            let at = Value::Int((i - seg_start) as i32);
            let val = self.read_slice_elem(v, elm_size, elm_tp, at);
            bindings.push(v_set(bind_nr, val));
            self.mark_slice_element_view(bind_nr, elm_tp, borrow_src);
            *name = "_".to_string();
        }
    }

    /// `@FR-P-Rule` — a sub-rule element `name: rule` of a cursor pattern, `k` fixed elements
    /// after the current segment's base.  ANDed onto the arm's `cond` it is
    /// `{ cursor.pos = base + k; name = rule(cursor); if name != null { seg = cursor.pos; true }
    /// else { false } }`: the rule runs only when everything before it matched, in arm order
    /// (`(P-Alt)`), and the next segment is read from `seg`.  `name` and `seg` are declared at
    /// the match's level (`prechain`), because the arm body reads them and a binding first made
    /// inside a condition is scoped to it on native.  The arm restores the cursor when it fails
    /// (`parse_slice_pattern`).  `consumed` says a fixed element of this pattern precedes the
    /// call, which makes its edge a consuming one for the termination check.
    #[expect(
        clippy::too_many_arguments,
        reason = "the slice pattern's own state, threaded"
    )]
    pub(super) fn parse_subrule_slice_element(
        &mut self,
        name: &str,
        fn_nr: u32,
        ret_tp: &Type,
        k: usize,
        consumed: bool,
        cond: &mut Option<Value>,
        prechain: &mut Vec<Value>,
    ) {
        let Some((cursor_var, cursor_def, pos_field, base)) = self.match_cursor else {
            return;
        };
        // Re-read `name : rule` (the peek reverted).
        self.lexer.has_identifier(); // name
        self.lexer.token(":");
        // @PLN35 PC3 — record the edge (enclosing rule -> invoked rule) at the invocation site for
        // the post-parse termination pass; it is a consuming edge when a token is taken before
        // the call, in this pattern or by the arm whose body holds this match.
        let site = self.lexer.peek().position;
        self.lexer.has_identifier(); // rule
        if !self.first_pass && self.context != u32::MAX {
            let consuming = consumed || self.cursor_consumed.contains(&cursor_var);
            self.subrule_edges
                .push((self.context, fn_nr, site, consuming));
        }
        self.slice_subrule = true;
        let name_var = self.pattern_binding(name, ret_tp);
        self.vars.defined(name_var);
        let seg = self.create_unique("cursor_seg", &I32);
        self.vars.defined(seg);
        let none = self.null_value(ret_tp);
        prechain.push(v_set(name_var, none));
        prechain.push(v_set(seg, Value::Int(0)));
        let at = self.conv_op(
            "+",
            Value::Var(base),
            Value::Int(k as i32),
            I32.clone(),
            I32.clone(),
        );
        let set_pos = self.set_field(cursor_def, pos_field, 0, Value::Var(cursor_var), at);
        let cursor_tp = self.vars.tp(cursor_var).clone();
        let mut call = Value::Null;
        if fn_nr != u32::MAX {
            self.call_nr(
                &mut call,
                fn_nr,
                &[Value::Var(cursor_var)],
                &[cursor_tp],
                false,
                &[],
                None,
            );
        }
        let present = self.subrule_present(fn_nr, name_var, ret_tp);
        let now = self.get_field(cursor_def, pos_field, Value::Var(cursor_var));
        let matched = v_block(
            vec![v_set(seg, now), Value::Boolean(true)],
            Type::Boolean,
            "sub-rule matched",
        );
        let step = v_block(
            vec![
                set_pos,
                v_set(name_var, call),
                v_if(present, matched, Value::Boolean(false)),
            ],
            Type::Boolean,
            "sub-rule",
        );
        *cond = Some(match cond.take() {
            Some(c) => v_if(c, step, Value::Boolean(false)),
            None => step,
        });
        self.match_cursor = Some((cursor_var, cursor_def, pos_field, seg));
    }

    /// @PLN35 PC3+PC4 — well-formedness pass over the sub-rule invocation graph, run post-parse over
    /// the edges recorded on pass 2.  **PC3 (termination):** every PC2 invocation `[ name: rule ]` is
    /// a WHOLE pattern, so the sub-rule runs at cursor position 0 (nothing consumed) and its call is
    /// hoisted unconditionally — a CYCLE therefore recurses forever (no base-case arm can intervene),
    /// so reject any cycle as left recursion.  **PC4 (purity):** an invoked sub-rule must be pure —
    /// the hoisted, possibly-backtracked call makes any observable side effect leak.
    pub(crate) fn check_subrule_wellformedness(&mut self) {
        // Drain the edges so a re-parse (REPL / multiple files) starts fresh.
        let edges = std::mem::take(&mut self.subrule_edges);
        if edges.is_empty() {
            return;
        }
        let mut adj: std::collections::HashMap<u32, Vec<(u32, crate::lexer::Position)>> =
            std::collections::HashMap::new();
        // `@FR-P-Rule` — only an edge that can run before any token is consumed can recurse
        // without progress; a consuming edge ends a cycle's danger.
        for (from, to, pos, consuming) in &edges {
            if !consuming {
                adj.entry(*from).or_default().push((*to, *pos));
            }
        }
        let cycles = Self::find_subrule_cycles(&adj);
        for (site, cycle) in &cycles {
            let path = cycle
                .iter()
                .map(|nr| Self::rule_display_name(&self.data, *nr))
                .collect::<Vec<_>>()
                .join(" -> ");
            let head = Self::rule_display_name(&self.data, cycle[0]);
            diagnostic_at!(
                self.lexer,
                site,
                Level::Error,
                "sub-rule `{head}` is left-recursive ({path}): a cursor `match` invokes a sub-rule \
                 before consuming any input, so this cycle would recurse forever"
            );
        }
        // @PLN35 PC4 — an invoked sub-rule must be PURE: a cursor `match` hoists its call
        // unconditionally (so it runs even when the arm is not taken) and may backtrack over it, so
        // an observable side effect (I/O, host mutation, prng) would leak.  Skip callees already in
        // a rejected cycle (that error stands).  One report per impure callee.
        let in_cycle: HashSet<u32> = cycles.iter().flat_map(|(_, c)| c.iter().copied()).collect();
        let mut checked: HashSet<u32> = HashSet::new();
        for (_from, callee, site, _) in &edges {
            if in_cycle.contains(callee) || !checked.insert(*callee) {
                continue;
            }
            if !crate::scopes::sub_rule_is_pure(&self.data, *callee) {
                let name = Self::rule_display_name(&self.data, *callee);
                diagnostic_at!(
                    self.lexer,
                    site,
                    Level::Error,
                    "sub-rule `{name}` is not pure — a cursor `match` may invoke it speculatively \
                     (even when its arm is not taken) and backtrack over it, so its side effects \
                     would be observable; a sub-rule must only advance the cursor and return (no \
                     I/O, host mutation, or randomness)"
                );
            }
        }
    }

    /// @PLN130 F9 / [loft#779](https://github.com/loft-lang/loft/issues/779) — refuse the one
    /// program shape where a `&` reference could not write through: a container reshaped while
    /// a reference into it is still live.
    ///
    /// Post-parse over pass 2, like [`Self::check_subrule_wellformedness`], and for the same
    /// reason: the question is asked of a CALLEE's body (*"does it remove from `&` parameter
    /// k?"*), which is only complete once the whole file is parsed. Runs on the program parse
    /// only — the stdlib's definitions are still in `Data` then, so a separate pass over the
    /// `default` load would only report them twice.
    ///
    /// The analysis is [`crate::scopes::reshape_refusals`]; this only turns its findings into
    /// diagnostics, because the collector lives on the lexer and the analysis does not.
    ///
    /// It is handed the STORE as well as the definitions: a growth names its container by field
    /// NUMBER and a view carries a byte OFFSET, so without `Stores::field_position` the walk
    /// cannot see a growth of a container held in a FIELD at all.  The layouts it reads are
    /// registered when each struct is declared, well before this post-pass-2 point, and
    /// `field_position` answers `u16::MAX` — *cannot say* — for anything it does not know.
    pub(crate) fn check_reshape_under_reference(&mut self) {
        for r in crate::scopes::reshape_refusals(&self.data, &self.database) {
            let pos = crate::lexer::Position {
                file: crate::lexer::intern_file(&r.file),
                line: r.line,
                pos: 1,
            };
            diagnostic_at!(self.lexer, &pos, Level::Error, "{}", r.message);
        }
        // `@FR-H-RefHome` — a stored reference whose holder outlives the record it names.
        // Asked here for the same reason: a callee's half (which arguments it links) is only
        // complete once every body is parsed.  A home that can move out to the holder's block
        // does, before the scope pass scopes it; a body that pass already scoped was judged
        // when it was fresh.
        let found = crate::scopes::stored_reference_homes(&self.data, &self.database);
        for r in found.refusals {
            let pos = crate::lexer::Position {
                file: crate::lexer::intern_file(&r.file),
                line: r.line,
                pos: 1,
            };
            diagnostic_at!(self.lexer, &pos, Level::Error, "{}", r.message);
        }
        let mut placing: Vec<u32> = found
            .places
            .iter()
            .map(|p| p.d_nr)
            .filter(|&d| !self.data.def(d).variables.done)
            .collect();
        placing.sort_unstable();
        placing.dedup();
        // Looked up only when something is placed: a program without stored references
        // pays nothing here.
        let ops = (!placing.is_empty()).then(|| crate::scopes::PlaceOps {
            set_db_ref: self.data.def_nr("OpSetDbRef"),
            database: self.data.def_nr("OpDatabase"),
            place_record: self.data.def_nr("OpPlaceRecord"),
        });
        for d in placing {
            // `@FR-H-RefHome` — a literal or call result written into a holder that outlives
            // it is built in the holder's store, by a fresh variable that borrows the holder.
            let def = &mut self.data.definitions[d as usize];
            let mut here = Vec::new();
            for p in found.places.iter().filter(|p| p.d_nr == d) {
                if here
                    .iter()
                    .any(|(o, _, _): &(&crate::scopes::Place, u16, Type)| {
                        o.site == p.site || (p.own && o.own && o.home == p.home)
                    })
                {
                    continue;
                }
                let tp = crate::scopes::borrowing(def.variables.tp(p.home), p.host);
                if p.own {
                    // The local itself borrows the holder's store: no free is emitted for it.
                    def.variables.set_type(p.home, tp.clone());
                    here.push((p, p.home, tp));
                    continue;
                }
                let fresh = def.variables.copy_variable(p.home);
                // Its own name: the native backend names a local by it, and a second
                // `__ref_p2_1` would shadow the temporary the exit frees.
                def.variables.rename(fresh, &format!("__place_{fresh}"));
                def.variables.set_type(fresh, tp.clone());
                here.push((p, fresh, tp));
            }
            if let Some(ops) = &ops {
                crate::scopes::apply_places(&mut def.variables, &mut def.code, &here, ops);
            }
        }
        for &(d, v) in &found.named {
            if !self.data.def(d).variables.done {
                self.data.definitions[d as usize]
                    .variables
                    .mark_ref_named(v);
            }
        }
        for e in &found.evacuations {
            if self.data.def(e.d_nr).variables.done {
                continue;
            }
            // `@FR-H-RefHome` — a record a parameter's reference names is copied out of its
            // home before the home's store is freed; the store stays the home's own.
            let def = &mut self.data.definitions[e.d_nr as usize];
            for &owner in &e.owners {
                def.variables.mark_ref_home(owner);
                def.variables
                    .add_ref_evacuation(e.holder, e.fld, owner, e.tp);
            }
        }
        let mut fresh: Vec<u32> = found
            .hoists
            .iter()
            .map(|h| h.d_nr)
            .filter(|&d| !self.data.def(d).variables.done)
            .collect();
        fresh.sort_unstable();
        fresh.dedup();
        for d in fresh {
            // Outermost holder first: an enclosing block's number is below its inner blocks'.
            let mut here: Vec<&crate::scopes::Hoist> =
                found.hoists.iter().filter(|h| h.d_nr == d).collect();
            here.sort_by_key(|h| h.block);
            let def = &mut self.data.definitions[d as usize];
            for h in &here {
                def.variables.mark_ref_home(h.owner);
            }
            let mut inserts: Vec<&crate::scopes::Hoist> = Vec::new();
            for h in here.into_iter().filter(|h| h.insert) {
                if !inserts.iter().any(|o| o.owner == h.owner) {
                    inserts.push(h);
                }
            }
            crate::scopes::insert_hoists(&mut def.code, &inserts);
        }
    }

    /// The user-facing name of a rule fn (`n_expr` -> `expr`).
    fn rule_display_name(data: &crate::data::Data, nr: u32) -> String {
        let n = data.def(nr).name();
        n.strip_prefix("n_").unwrap_or(n).to_string()
    }

    /// DFS the sub-rule graph; return one `(site, cycle-path)` per distinct back-edge.  `cycle` is
    /// the closed node sequence `callee … node callee`; `site` the invocation position closing it.
    /// Nodes visited in sorted order for a deterministic report.
    fn find_subrule_cycles(
        adj: &std::collections::HashMap<u32, Vec<(u32, crate::lexer::Position)>>,
    ) -> Vec<(crate::lexer::Position, Vec<u32>)> {
        let mut color: std::collections::HashMap<u32, u8> = std::collections::HashMap::new();
        let mut reported: HashSet<(u32, u32)> = HashSet::new();
        let mut out: Vec<(crate::lexer::Position, Vec<u32>)> = Vec::new();
        let mut nodes: Vec<u32> = adj.keys().copied().collect();
        nodes.sort_unstable();
        for start in nodes {
            if color.get(&start).copied().unwrap_or(0) == 0 {
                Self::dfs_subrule(
                    start,
                    adj,
                    &mut color,
                    &mut Vec::new(),
                    &mut reported,
                    &mut out,
                );
            }
        }
        out
    }

    fn dfs_subrule(
        node: u32,
        adj: &std::collections::HashMap<u32, Vec<(u32, crate::lexer::Position)>>,
        color: &mut std::collections::HashMap<u32, u8>,
        path: &mut Vec<u32>,
        reported: &mut HashSet<(u32, u32)>,
        out: &mut Vec<(crate::lexer::Position, Vec<u32>)>,
    ) {
        color.insert(node, 1); // gray = on the current DFS path
        path.push(node);
        if let Some(edges) = adj.get(&node) {
            for (callee, pos) in edges {
                match color.get(callee).copied().unwrap_or(0) {
                    1 => {
                        // back-edge into a node on the path — a cycle
                        if reported.insert((node, *callee)) {
                            let idx = path.iter().position(|x| x == callee).unwrap_or(0);
                            let mut cycle = path[idx..].to_vec();
                            cycle.push(*callee);
                            out.push((*pos, cycle));
                        }
                    }
                    0 => Self::dfs_subrule(*callee, adj, color, path, reported, out),
                    _ => {}
                }
            }
        }
        path.pop();
        color.insert(node, 2); // black = fully explored
    }
}
