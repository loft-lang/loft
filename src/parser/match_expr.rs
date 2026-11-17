// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Parsing a `match` expression: its subject, the arms and their bodies, the struct and variant
//! arms with their field captures, and the checks that skip or settle an arm.
//! A part of the match subject split out of `control.rs`; the others are listed there.

use super::control::{
    EnumArm, arm_body_is_null, ends_in_halt, give_halt_a_value, guarded_arm, is_place,
    mark_forward_call, set_arm_null_typed,
};
use super::{
    DefType, Level, LexItem, Parser, Type, Value, diagnostic_format, v_block, v_if, v_set,
};
use crate::data::Deps;
use std::collections::HashSet;

impl Parser {
    /// @PLN85 match_return (LOFT_JOIN_OWN): an arm that yields a borrowed-view vector
    /// FIELD BINDING directly (`Filled { items } => { items }`) returns a view into the
    /// match subject — which the caller cannot free without an over-free. Wrap the yield
    /// in an OWNED copy `{ o = []; o += items; o }` (a fresh local `o`), so the value
    /// escapes OWNED. This is exactly the `deliver3` shape: the existing `ref_return`
    /// promotion then promotes `o` to the buffer arg + emits the `__retbuf` marker, the
    /// separate-buffer ABI the caller adopts (and so frees the argument). Done at PARSE
    /// time (re-parsed each pass) so `create_unique`/`vector_db` stay pass-consistent.
    /// Returns the new OWNED arm type when it rewrote (for cross-arm unification), else
    /// `None`. No-op unless the tail is a `skip_free`, non-empty-dep vector binding.
    #[allow(clippy::question_mark)]
    fn jo_copy_borrowed_arm_yield(
        &mut self,
        arm_body: &mut Value,
        arm_type: &Type,
    ) -> Option<Type> {
        // An arm that yields nothing yields no view, whatever its code reads last:
        // `P { f } => { other = f; }` and `P { f } => other = f` assign the capture, and on
        // pass 1 the bind's lowering leaves `f` where the yield would be.  Taken for a yield,
        // the arm became a `vector` and a statement `match` a value, demanding a `;` after it.
        if !crate::keys::join_own_enabled() || matches!(arm_type.base(), Type::Void | Type::Never) {
            return None;
        }
        let v = match arm_body.unspan() {
            Value::Var(v) => *v,
            Value::Block(bl) => match bl.operators.last().map(Value::unspan) {
                Some(Value::Var(v)) => *v,
                _ => return None,
            },
            _ => return None,
        };
        // @FR-O-Proxy asks copy — a non-empty dep list is what marks the arm's yield as a
        // BORROW, and the answer chooses whether to build the owned `mvcopy`.  It authorises
        // no free: the copy is a fresh binding, and the borrow keeps its own owner.
        // The same notion `ref_return` asks, asked once
        // ([`Function::is_marked_vector_borrow`]) — read negated here, guarding the early exit.
        if v >= self.vars.count() || !self.vars.is_marked_vector_borrow(v) {
            return None;
        }
        let v_type = self.vars.tp(v).clone();
        let elm = match &v_type {
            Type::Vector(b, _) => (**b).clone(),
            _ => return None,
        };
        // Create `o` with the OWNED element type (no deps) — NOT `v_type`, which is the
        // binding's `vector<ref(E)>["e"]`; inheriting that `["e"]` would mark the copy as
        // borrowing `e` and re-propagate it to the return (the leak). `deliver3`'s `o` is
        // dep-free.
        let owned_create = Type::Vector(Box::new(elm.clone()), Deps::none());
        let o = self.create_unique("mvcopy", &owned_create);
        if o == u16::MAX {
            return None;
        }
        self.vars.defined(o);
        // `o = []` (pass-gated: empty on pass 1, the OpDatabase alloc on pass 2 — exactly
        // as a user-written `o: vector = []` lowers), then `o += <binding>`, then yield o.
        let mut ops = self.vector_db(&elm, o);
        // `o = []` is a REPLACE, and `o` becomes the caller's return buffer once
        // `ref_return` promotes it — at which point `vector_db` above no-ops (an argument
        // keeps the caller's store) and the append below piles this call's elements onto
        // the LAST call's.  A two-borrowed-arm match returned 3, then 6, then 9 elements
        // from the same subject.  It went unnoticed while some OTHER arm cleared the
        // buffer as a side effect of its own `= []` and the clear got hoisted to entry —
        // so the bug appeared and vanished with the shape of an unrelated arm.  Clearing
        // here says it once, where the replace is: a no-op on a fresh store, correct on a
        // reused one.  Same reasoning as the `= <literal>` buffer clear in
        // `create_vector` (@PLN85 #492).
        ops.push(self.cl("OpClearVector", &[Value::Var(o)]));
        let elem_tp = self.append_elem_tp(&elm);
        ops.push(self.cl(
            "OpAppendVector",
            &[Value::Var(o), Value::Var(v), Value::Int(elem_tp)],
        ));
        ops.push(Value::Var(o));
        let owned_tp = Type::Vector(Box::new(elm), Deps::frame1(o));
        let copy_block = crate::data::v_block(ops, owned_tp.clone(), "jo_arm_copy");
        // Replace the WHOLE arm body (a bare `{ items }` yield) with the owned copy
        // block, NOT just its last op — wrapping inside leaves the outer block typed by
        // the borrowed binding (`["_mv_items_1"]`), which re-propagates the `["e"]` dep
        // to the return. Only the single-yield shape (the match-field binding IS the arm
        // value) is rewritten; a multi-statement arm is left alone.
        let n = arm_body.unspan_mut();
        match n {
            Value::Var(_) => *n = copy_block,
            Value::Block(bl) if bl.operators.len() == 1 => *n = copy_block,
            _ => return None,
        }
        Some(owned_tp)
    }

    // <match> ::= 'match' <expression> '{' { <pattern> '=>' <expression> } '}'
    // <pattern> ::= '_' | <variant> [ '{' <field> { ',' <field> } '}' ]
    // @F29 — pattern matching (enum/scalar/tuple, guards, or-patterns, exhaustiveness)
    pub(crate) fn parse_match(&mut self, code: &mut Value) -> Type {
        // loft#1382 / loft#1386 — statement position comes from the caller (`parse_block`'s
        // loop is what sees the `;`), and the void-arm fact is scoped to THIS match so a
        // nested one cannot leak into its parent's verdict.
        let is_stmt = std::mem::replace(&mut self.stmt_if_pending, false);
        let outer_arms = std::mem::replace(&mut self.arms_of_statement_construct, is_stmt);
        let outer_void = std::mem::replace(&mut self.match_void_arm, false);
        // @PLN187 — this match's subject and arm facts; a nested match keeps its own.
        let outer_subject = std::mem::take(&mut self.match_subject);
        let outer_arm_facts = std::mem::take(&mut self.match_arm_facts);
        let outer_root = std::mem::replace(&mut self.match_subject_root, u16::MAX);
        let mut r = self.parse_match_inner(code);
        // `@FR-C-Never` — a match never finishes only when EVERY arm does not: beside an arm
        // that finishes without a value it is a statement that finishes (loft#2499).  Typed
        // `never`, an enclosing `if` read it as its arm's diverging tail and put the sibling's
        // null in its place, so the arm that matched did not run.
        if matches!(r.base(), Type::Never) && self.match_void_arm {
            r = Type::Void;
            if let Value::Block(bl) = code
                && matches!(bl.result.base(), Type::Never)
            {
                bl.result = Type::Void;
            }
        }
        let arm_facts = std::mem::replace(&mut self.match_arm_facts, outer_arm_facts);
        self.match_subject = outer_subject;
        self.match_subject_root = outer_root;
        if !matches!(r.base(), Type::Void | Type::Null | Type::Never) {
            self.settle_join(&arm_facts);
        } else if self.abstract_on() {
            self.produced = Some(crate::data::AliasFact::Plain);
        }
        self.operand_fact = self.produced.clone().unwrap_or_default();
        // @FR-F-Block discards a STATEMENT's arms, so a void one there is no defect.  In
        // VALUE position the path that ran yields nothing, and the exemption let the match
        // take the other arms' type: `v = match k { 1 => { 5 }, _ => { println(…) } }`
        // answered `v = null` with nothing said, where the `if` twin is refused (loft#1386).
        if !self.first_pass
            && !self.arms_of_statement_construct
            && self.match_void_arm
            && !matches!(r, Type::Void | Type::Null | Type::Never)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "expected {}, got void on a match arm — this `match` is used as a VALUE, so \
                 every arm has to produce one; give the arm a value, or make the `match` a \
                 statement by ending it with `;`",
                r.source_name(&self.data),
            );
        }
        self.match_void_arm = outer_void;
        self.arms_of_statement_construct = outer_arms;
        r
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the match construct in one pass: the subject's shape picks the arm grammar, and every arm's pattern, guard, body and type join read the same arm list and lexer position"
    )]
    fn parse_match_inner(&mut self, code: &mut Value) -> Type {
        // One charge for the whole construct — arm count is not complexity (a 12-arm flat
        // dispatch reads straight down); its arms deepen via `parse_block("match_arm")`.
        if !self.first_pass {
            *self.complexity.entry(self.context).or_insert(0) += 1 + self.cc_nest;
        }
        // Save position of the match keyword for exhaustiveness diagnostics.
        let match_pos = *self.lexer.pos();
        // 1. Parse the subject expression.
        let mut subject = Value::Null;
        // A struct literal built in place types as `Rewritten(τ)` on the first pass — a signal to
        // the expression that parsed it (`Type::unrewritten`), which the subject dispatch below
        // does not recognise: `match Vn { rs: 7 } { … }` dispatched nowhere on pass 1, typed the
        // match `void`, and a local bound to it refused pass 2's real type.
        let mut subject_type = self.expression(&mut subject).unrewritten();
        self.match_subject_root = self.subject_place_root(&subject);
        // @PLN187 — a tuple subject is inspected position by position, by its patterns; any
        // other abstract subject is read by the match itself.
        let subject_fact = std::mem::take(&mut self.operand_fact);
        if matches!(subject_type.base(), Type::Tuple(_)) {
            self.match_subject = subject_fact;
        } else {
            self.check_subject(&subject_fact, "matching on it");
        }
        // `(T-Ref)`: a `&(…)` binding denotes the bound tuple itself, so a tuple pattern over it
        // reads every element through the reference, as `t.0` does.  The subject becomes the
        // tuple of those element reads, which the tuple match stores and projects like any
        // other.  Left as the reference, a record-backed tuple reached the plain-struct
        // handler and was refused at the first `(` (loft#1530), and a stack-backed one was
        // copied into the match's stack tuple as a reference, which no store lowers.
        if let Some((reads, elems)) = self.ref_tuple_subject(&subject, &subject_type) {
            subject = reads;
            subject_type = Type::Tuple(elems);
        }
        // @PLN25: a `τ?` subject matches as its base (shared sentinel storage) — peel the marker
        // so `match` on an `integer?` routes to the scalar handler instead of falling to the `_`
        // arm ("match requires an enum, struct, or scalar type"). Gate-OFF inert (never Optional).
        // A `&` subject is read through its reference, as every other operation on a `&`
        // binding is (LOFT_DECLARATIONS.md § References): the SHAPE question is asked of the pointee
        // (`Type::peel_link`), and the subject value reads through the link like a field
        // access does (loft#1526).
        let subject_type = subject_type.peel_link().clone();
        // A subject whose type is not linked yet on the FIRST pass — `match p { … }` where
        // `p` came from an enum declared LOWER in the file.  The dispatch below cannot
        // recognise it, so the arms contribute nothing and `result_type` would stay `Void`;
        // a local bound to the match then locks to void and pass 2's real type is REFUSED
        // ("cannot change type from void to integer").  LOFT_DECLARATIONS.md § File structure promises a
        // file may hold its declarations "in any order", so this must stay re-typeable —
        // the same first-pass escape `call_op` takes for an unresolved operand.
        let subject_unresolved = self.first_pass && subject_type.is_unknown();

        // @PLN35 PC1 — a CURSOR subject (a struct with a `vector<T>` source field + an integer
        // `pos` field) PREFIX-consumes: route to the vector-match over its source with cursor mode
        // on, so the arm pattern reads/advances relative to `pos`.  A plain struct without that
        // shape falls through to the normal struct handler.  Routes in BOTH passes so the arm
        // grammar (slice patterns, not struct patterns) is parsed consistently.
        if let Some((cursor_def, source_field, pos_field, celm_tp)) =
            self.cursor_shape(&subject_type)
        {
            return self.parse_cursor_match(
                subject,
                cursor_def,
                source_field,
                pos_field,
                &celm_tp,
                code,
            );
        }

        // Resolve type info from the subject.
        // Accepts: plain enums, struct-enums, struct-enum variants, and plain structs (T1-18).
        let (e_nr, is_struct, valid_enum, is_plain_struct) = match &subject_type {
            Type::Enum(nr, s, _) => (*nr, *s, true, false),
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::EnumValue => {
                let parent = self.data.def(*d_nr).parent();
                (parent, true, true, false)
            }
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Enum => {
                // iterating a `vector<StructEnum>` yields loop variables
                // typed `Type::Reference(enum_def, _)` (via `for_type` in this
                // file, line 1952 — struct-enums degrade to a reference type
                // when carried through generic collections).  Without this
                // arm, matching a for-loop variable over a struct-enum vector
                // dropped into the error branch and every arm produced
                // 'Expect token }' cascades.
                (*d_nr, true, true, false)
            }
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Struct => {
                (*d_nr, true, true, true)
            }
            // scalar types — dispatch to scalar match handler.
            Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Text(_) => {
                // @PLN25 DN1: a scalar match's `_` arm is USER-written (no synthesised default),
                // so the value-level widen is safe — a `_ => null` arm makes the match nullable.
                let tp = self.parse_scalar_match(subject, &subject_type, code, &match_pos);
                return self.dn1_widen_branch_null(tp, code);
            }
            // vector types — dispatch to vector match handler.  A KEYED collection takes the
            // same handler for the arms it has — `null`, a capture, `_` (`@FR-N-Match` at every
            // τ, loft#2427); it has no positions, so a slice arm is refused there.
            Type::Vector(_, _) => {
                return self.parse_vector_match(subject, &subject_type, code);
            }
            t if crate::parser::vectors::is_collection(t) => {
                return self.parse_vector_match(subject, &subject_type, code);
            }
            // @PLN35 Phase 7 (Path B) — a coroutine `iterator<T>` subject: materialise it into a
            // fresh buffer `vector<T>` (eager pull), then run the SAME vector-match over the buffer.
            // Streaming stays entirely behind the Cursor seam; the match logic is untouched.
            Type::Iterator(elem_box, _) => {
                let elm_tp = (**elem_box).clone();
                let iter_tp = subject_type.clone();
                let match_pos = *self.lexer.pos();
                let (buf, vec_tp, setup) =
                    self.collect_iterator_subject(subject, &iter_tp, &elm_tp, &match_pos);
                let mut match_code = Value::Null;
                let result_tp = self.parse_vector_match(Value::Var(buf), &vec_tp, &mut match_code);
                let mut ops = setup;
                ops.push(match_code);
                *code = v_block(ops, result_tp.clone(), "stream match");
                return result_tp;
            }
            // T1.9: tuple types — dispatch to tuple match handler.
            Type::Tuple(_) => {
                return self.parse_tuple_match(subject, &subject_type, code);
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "match requires an enum, struct, or scalar type"
                    );
                }
                (u32::MAX, false, false, false)
            }
        };

        // `@FR-M-Match` — the subject is evaluated ONCE: every arm's test and every binding read
        // the one value.  A plain enum (stack bytes) is always bound to a temp.  A struct enum
        // or struct read from a PLACE (a variable, or a field or element of one) is read in
        // place, since reading it again reads the same value; anything else — a call above
        // all — is bound to a temp, which the scope pass owns and frees like any local.  Spliced
        // into each test and binding, `match next(lx)` ran the call once for the variant and
        // again for each field.
        let (subject_val, preamble): (Value, Option<(u16, Value)>) =
            if !valid_enum || (is_struct && is_place(&subject, &self.data)) {
                (subject, None)
            } else {
                let v = self.create_match_subject(&subject_type);
                self.vars.defined(v);
                (Value::Var(v), Some((v, subject)))
            };

        // Build discriminant expression: integer representation of the active variant.
        let disc_expr = if is_struct {
            let get_enum = self.cl("OpGetEnum", &[subject_val.clone(), Value::Int(0)]);
            self.cl("OpConvIntFromEnum", &[get_enum])
        } else {
            self.cl("OpConvIntFromEnum", std::slice::from_ref(&subject_val))
        };

        self.lexer.token("{");

        let mut arms: Vec<EnumArm> = Vec::new();
        let mut covered: HashSet<u32> = HashSet::new();
        let mut has_wildcard = false;
        // An arm read as a catch-all only to recover from a bare lowercase name: it covers the
        // rest for the exhaustiveness check, but arms after it are not reported unreachable.
        let mut recovered_catch_all = false;
        let mut result_type = Type::Void;
        // L2: field bindings in conditional arms are hoisted before the if-chain
        // to avoid codegen stack-layout issues with text operations inside branches.
        let mut hoisted_bindings: Vec<Value> = Vec::new();

        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            // @PLN25 — a `null` pattern arm on a nullable inline enum element
            // (`match vr[i] { null => …, Some{…} => …/_ => … }`) matches the ABSENT
            // state: discriminant 0.  The synthetic `__nullable<S>` enum represents
            // null as disc 0 (not a produced variant), and `disc_expr` already reads
            // the discriminant, so this arm is just `discs == [0]`.  Scoped to the
            // synth enum (a regular enum's null is the variable store_nr sentinel,
            // not an inline disc — E1).  `null` is a keyword, not an identifier, so
            // it must be matched before the `has_identifier()` variant path below.
            // A nullable HEAP subject spells absence as the store-pointer sentinel rather
            // than an inline discriminant, so its `null` arm asks `OpRefIsNull` and names no
            // variant.  `(N-Match)` is stated for every t — it held for a scalar and for the
            // synthetic inline element only, and the arm-head parser reads a VARIANT name, so
            // `null` broke out of the loop and the run died on a brace (loft#1417).
            let synth_null_elem =
                e_nr != u32::MAX && self.data.def(e_nr).name.starts_with("__nullable<");
            // Gated on the subject being a HEAP value, not on its declared type carrying a
            // `?`: by this point the match setup has already peeled `Optional` off the
            // subject (it reads `Enum(e, true, _)` for a `Tok?` local as much as for a
            // `Tok` one), so the wrapper is not available to test.  That costs nothing —
            // `OpRefIsNull` is well defined for any heap subject, and on one that cannot be
            // absent the arm is simply never taken, exactly like an unreachable `_`.
            let heap_null_subject = is_struct && !synth_null_elem;
            // A plain VALUE enum spells absence in its disc byte (0 or 255), which no variant
            // takes, so its `null` arm asks `null_test` too and a variant arm never matches a
            // null.  Without it the arm-head parser read `null` as a variant name (loft#2230).
            let value_null_subject = !is_struct && !synth_null_elem;
            if valid_enum
                && (synth_null_elem || heap_null_subject || value_null_subject)
                && self.lexer.has_token("null")
            {
                self.expect_match_arm_arrow();
                let mut arm_body = Value::Null;
                let arm_expected = self.match_arm_expected(&result_type);
                let mut arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_body);
                // loft#978 — every arm can deliver this match's value, so the result carries
                // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
                // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
                result_type = self.join_arm_into(&result_type, &arm_body, &arm_type);
                self.match_void_arm |= matches!(arm_type.base(), Type::Void);
                if Self::arm_settles_match(&result_type, &arm_type) {
                    result_type = arm_type.clone();
                } else if let Some(joined) =
                    self.join_null_tuple_arm(&result_type, &mut arm_body, &mut arm_type)
                {
                    self.reconvert_null_tuple_arms(&mut arms, &joined);
                    result_type = joined;
                } else if !self.first_pass
                    && arm_type != Type::Void
                    && arm_type != Type::Null
                    && arm_type != Type::Never
                    && !self.arm_convert_reported
                    && !self.match_arms_unify(&result_type, &arm_type)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot unify: {} and {}",
                        result_type.source_name(&self.data),
                        arm_type.source_name(&self.data)
                    );
                }
                // A `null` arm (disc 0) covers the synth enum's `Null` variant for
                // exhaustiveness — disc 1 (the vestigial `Null` variant) is never
                // produced (null is disc 0), so `null` + `Some` IS exhaustive.
                let null_variant = self.data.variant_of(e_nr, "Null");
                if null_variant != u32::MAX {
                    covered.insert(null_variant);
                }
                // The inline element reads discriminant 0; the heap subject asks its own
                // sentinel, and covers no variant — an absent subject is not a variant of
                // anything, which is why exhaustiveness is untouched for it.
                let (discs, cond) = if heap_null_subject || value_null_subject {
                    // `null_test` is the one home for "is this absent?": a struct-enum
                    // subject bound from an `E` slot is a sub-reference whose absence is its
                    // discriminant, which the bare sentinel test called present, so the
                    // match fell through every arm.
                    let is_null = self
                        .null_test(subject_val.clone(), &subject_type, false)
                        .unwrap_or_else(|| {
                            self.cl("OpRefIsNull", std::slice::from_ref(&subject_val))
                        });
                    (Vec::new(), Some(is_null))
                } else {
                    (vec![0], None)
                };
                arms.push(EnumArm {
                    discs,
                    code: arm_body,
                    tp: arm_type,
                    guard: None,
                    bindings: Vec::new(),
                    cond,
                });
                self.lexer.has_token(","); // optional trailing comma
                continue;
            }
            // A SLICE pattern over a plain struct: the subject would have routed to the cursor
            // path above if it were one, so say which field stopped it and skip the arm.  The
            // skip runs on BOTH passes — leaving the `[` for the struct-pattern parser cost the
            // run its second pass, which is where the reason would have been printed.
            if is_plain_struct && self.lexer.peek_token("[") {
                if !self.first_pass {
                    let why = self.cursor_defect(e_nr);
                    let subject_name = self.data.def(e_nr).name().to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a slice pattern `[ … ]` matches a vector or a cursor; `{subject_name}` \
                         is neither — {why}"
                    );
                }
                self.lexer.token("[");
                self.skip_rest_of_slice();
                self.expect_match_arm_arrow();
                self.skip_match_arm_body();
                self.end_pattern_arm();
                continue;
            }
            // loft#2522 — a subject from a function declared lower in the file has no type on
            // pass 1, so this loop, which reads a VARIANT pattern, cannot read a literal, slice
            // or tuple one (`30 =>`, `[x, ..] =>`).  Pass 1 skips such a pattern to its `=>`
            // and still parses the body, so what pass 1 registers from it stays in step with
            // pass 2, which reads the arm by the subject's real type.
            if subject_unresolved && !matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
                self.skip_unresolved_pattern();
                self.expect_match_arm_arrow();
                let mut arm_body = Value::Null;
                self.parse_match_arm_body(&Type::Unknown(0), &mut arm_body);
                self.end_pattern_arm();
                self.lexer.has_token(",");
                continue;
            }
            // `@FR-P-Cap` at the arm root — `whole: Rect { w, h } => …` binds the matched subject
            // beside the pattern's own names; `other: _ => …` is the catch-all that binds it.
            let root_capture =
                self.root_capture(&subject_val, &subject_type, &mut hoisted_bindings);
            let Some(first_ident) = self.lexer.has_identifier() else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expect variant name or '_' in match arm"
                    );
                }
                break;
            };

            // accept `Library::Variant` or `EnumName::Variant` qualified patterns.
            // The `::` resolves the right-hand identifier in the named scope.
            let pattern_name = if self.lexer.has_token("::") {
                let Some(vname) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after '::'");
                    }
                    break;
                };
                vname
            } else {
                first_ident.clone()
            };

            // A bare lowercase name at the arm root binds nothing: the catch-all that binds the
            // subject is spelled `other: _` (`@FR-P-Cap`).  Say so, and parse the arm as that
            // catch-all, so its body and the match's exhaustiveness do not report the same
            // mistake twice more ("Unknown variable", "not exhaustive").
            let recovering = root_capture.is_none()
                && pattern_name == first_ident
                && Self::is_binding_name(&pattern_name)
                && e_nr != u32::MAX
                && self.data.variant_of(e_nr, &pattern_name) == u32::MAX
                && self.data.def_nr(&pattern_name) == u32::MAX
                && (self.lexer.peek_token("=>") || self.lexer.peek_token("if"));
            let pattern_name = if recovering {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{pattern_name}' is not a variant of {} — to bind the matched value, \
                         write `{pattern_name}: _`",
                        self.data.def(e_nr).name()
                    );
                }
                // A name that is already a local keeps meaning that local in the body; a new
                // one is bound, and counted read, so the recovery adds no lint of its own.
                if !self.vars.name_exists(&pattern_name)
                    && let Some(v) = self.bind_root_capture(
                        &pattern_name,
                        &subject_val,
                        &subject_type,
                        &mut hoisted_bindings,
                    )
                {
                    self.vars.in_use(v, true);
                }
                "_".to_string()
            } else {
                pattern_name
            };

            if pattern_name == "_" {
                // loft#1682 / loft#1793 — a result the wildcard can still widen: an open tuple
                // member, or a numeric type a wider arm joins past.
                let before = (self.tuple_join_open(&result_type)
                    || matches!(result_type.base(), Type::Integer(_) | Type::Single))
                .then(|| result_type.clone());
                let (arm, is_exhaustive) = self.parse_match_wildcard_arm(&mut result_type);
                recovered_catch_all |= recovering;
                let is_exhaustive = is_exhaustive && !recovering;
                if before.is_some_and(|b| !b.is_equal(&result_type)) {
                    // The wildcard arm named the member or widened the join; the earlier arms
                    // follow.
                    let joined = result_type.clone();
                    self.reconvert_null_tuple_arms(&mut arms, &joined);
                }
                has_wildcard = is_exhaustive;
                arms.push(arm);
                self.lexer.has_token(","); // optional trailing comma
                if !has_wildcard {
                    continue;
                }
                // @FR-M-Wild — a total `_` matches everything, so an arm written after it can
                // never be selected.  Say that here: leaving it to the closing-brace
                // expectation at the end of the loop reported "Expect token }" — the right
                // caret with the wrong reason, on the rule the Match chapter states as "put it
                // last".
                //
                // `continue` rather than `break`, so the unreachable arms are parsed as the
                // arms they are and the `}` is consumed normally; breaking here produced a
                // second, spurious error about the brace.  A GUARDED `_ if cond` never reaches
                // this point — it is not total, so it took the `continue` above, and arms are
                // expected to follow it.
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

            // @PLN22 Phase 1 — resolve the variant against the subject enum via
            // the variant_of chokepoint (the (enum, variant) scope key), not the
            // bare global def_nr.  This also subsumes the C53 fix (a library
            // variant not wildcard-imported is still a child of its enum).  A
            // plain-struct match's "pattern" is the struct TYPE itself (still
            // globally keyed, never an EnumValue), so fall back to def_nr when
            // variant_of finds nothing.
            let mut variant_def_nr = self.data.variant_of(e_nr, &pattern_name);
            if variant_def_nr == u32::MAX {
                variant_def_nr = self.data.def_nr(&pattern_name);
            }
            if self.data.def_type(variant_def_nr) == DefType::EnumValue {
                self.check_visibility("variant", variant_def_nr, usize::MAX);
                // loft#2025 — the capture holds this variant for the arm's body: the pattern
                // is the check a field access through it would otherwise ask for.
                if let Some(v) = root_capture {
                    self.capture_variant.insert(v, variant_def_nr);
                }
            }

            // for plain struct match, the pattern name must match the struct type.
            // There is no discriminant — the arm always matches.
            if is_plain_struct {
                if variant_def_nr != e_nr && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' does not match struct type {}",
                        pattern_name,
                        self.data.def(e_nr).name()
                    );
                }
                let (struct_arms, exhaustive) = self.parse_match_struct_arm(
                    e_nr,
                    &subject_val,
                    &mut result_type,
                    &mut hoisted_bindings,
                );
                has_wildcard = exhaustive;
                arms.extend(struct_arms);
                self.lexer.has_token(",");
                // @FR-M-Wild — a pattern with no literal field matches every struct, so an arm
                // written after it can never be selected: say so, as the `_` arm does, and keep
                // parsing the arms (a `break` left them to the closing brace, which reported
                // "Expect token }").
                if has_wildcard && !self.lexer.peek_token("}") {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "this pattern matches every {}, so the arm after it can never be \
                             selected — move it to the end",
                            self.data.def(e_nr).name()
                        );
                    }
                    continue;
                }
                if has_wildcard {
                    break;
                }
                continue;
            }

            let bad_variant = e_nr == u32::MAX
                || variant_def_nr == u32::MAX
                || self.data.def_type(variant_def_nr) != DefType::EnumValue
                || self.data.def(variant_def_nr).parent() != e_nr;
            if bad_variant {
                // @FR-M-Unit — an arm's pattern names a VARIANT of the subject's enum, and an
                // arm that names anything else can never be selected.  The test is on the
                // SUBJECT enum (`e_nr` resolved, second pass), not on whether the pattern name
                // resolved to something: a name that resolves NOWHERE is exactly the common
                // spelling of this mistake — a typo, or a variant that has since been renamed —
                // and it is the one an author cannot see, because a dead arm looks like a live
                // one and the value it should have produced comes from `_` instead.
                //
                // `e_nr == u32::MAX` stays silent: the subject enum is a cross-package forward
                // reference whose dependency parses later (#375), so pass 1 skips the arm and
                // pass 2 reads it with the enum resolved.  The same pairing `valid_enum &&
                // e_nr != u32::MAX` guards the or-pattern branch below.
                if !self.first_pass && valid_enum && e_nr != u32::MAX {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        pattern_name,
                        self.data.def(e_nr).name()
                    );
                }
                // Skip this arm gracefully: its `{ … }` field pattern whole, whatever tokens
                // it holds — a loop consuming only names and commas never advanced past
                // anything else (`=>`, `:`, a literal), and hung the parser.  A subject whose
                // enum pass 1 cannot see yet still binds its captures for the body.
                let forward = if self.first_pass && e_nr == u32::MAX {
                    self.bind_forward_capture_list()
                } else {
                    None
                };
                match forward {
                    Some(mut aliases) => self.pattern_binds_pending.append(&mut aliases),
                    None => self.skip_braced(),
                }
                self.expect_match_arm_arrow();
                let mut arm_code = Value::Null;
                self.expression(&mut arm_code);
                self.end_pattern_arm();
                // Consume the optional trailing comma, mirroring the wildcard /
                // struct arm paths.  Without it, the next loop iteration sees the
                // leading `,` instead of a variant name and breaks early, leaving
                // the lexer mid-arm-list — which desyncs into "Expect token }".
                // This skip path fires on pass 1 whenever `e_nr == u32::MAX`
                // (the subject enum is an unresolved cross-package forward
                // reference whose dependency parses later — #375); a clean skip
                // keeps pass 1 from aborting before the dependency registers, so
                // pass 2 (with the enum resolved) parses the arms normally.
                self.lexer.has_token(",");
                continue;
            }

            // Get the discriminant integer for this variant.
            let disc: i32 = if is_struct {
                // Struct enum: field-carrying variants store the discriminant
                // in attributes[0] (the synthetic "enum" attr added by
                // parse_enum_variants).  Unit variants (`pub enum E { Null,
                // Some { … } }`) carry no attributes of their own — fall
                // back to the parent enum's attribute for this variant name.
                let variant_attrs = self.data.def(variant_def_nr).attributes();
                if let Some(first) = variant_attrs.first()
                    && let Value::Enum(nr, _) = first.value
                {
                    i32::from(nr)
                } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&pattern_name) {
                    if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                        i32::from(nr)
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                // Plain enum: discriminant is stored in the parent enum's attributes.
                if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&pattern_name) {
                    if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                        i32::from(nr)
                    } else {
                        0
                    }
                } else {
                    0
                }
            };

            // or-patterns — collect additional BARE variants separated by `|`.  A variant
            // that opens `{` binds fields, so it is not bare: the lexer is rewound to its
            // `|` and the multi-pattern loop below takes it with its bindings (`@FR-P-Multi`,
            // where `|` and `,` are the same separator).
            let mut all_discs = vec![disc];
            loop {
                let before_or = self.lexer.link();
                if !self.lexer.has_token("|") {
                    break;
                }
                let Some(first_or) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after '|'");
                    }
                    break;
                };
                // accept Lib::Variant in or-patterns as well.
                let next_name = if self.lexer.has_token("::") {
                    let Some(vname) = self.lexer.has_identifier() else {
                        if !self.first_pass {
                            diagnostic!(self.lexer, Level::Error, "expect variant name after '::'");
                        }
                        break;
                    };
                    vname
                } else {
                    first_or.clone()
                };
                if self.lexer.peek_token("{") {
                    self.lexer.revert(before_or);
                    break;
                }
                // @PLN22 Phase 1 — or-pattern variant resolves against the
                // subject enum via the variant_of chokepoint (or-patterns are
                // plain-enum only, so no struct fallback is needed).
                let next_def_nr = self.data.variant_of(e_nr, &next_name);
                if !self.first_pass
                    && (next_def_nr == u32::MAX
                        || self.data.def_type(next_def_nr) != DefType::EnumValue
                        || self.data.def(next_def_nr).parent() != e_nr)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        next_name,
                        self.data.def(e_nr).name()
                    );
                } else {
                    let next_disc = if is_struct {
                        // B1-style guard (same shape as line 603): unit
                        // variants carry no attributes of their own; fall
                        // back to the parent enum's attr list.
                        let next_variant_attrs = self.data.def(next_def_nr).attributes();
                        if let Some(first) = next_variant_attrs.first()
                            && let Value::Enum(nr, _) = first.value
                        {
                            i32::from(nr)
                        } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&next_name) {
                            if let Value::Enum(nr, _) =
                                self.data.def(e_nr).attributes()[*a_nr].value
                            {
                                i32::from(nr)
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&next_name) {
                        if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                            i32::from(nr)
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    all_discs.push(next_disc);
                    // Each or-pattern variant counts for exhaustiveness.
                    if !self.first_pass {
                        covered.insert(next_def_nr);
                    }
                }
            }

            // Parse optional field bindings for struct-enum arms.
            let mut arm_stmts: Vec<Value> = Vec::new();
            let mut field_conditions: Vec<Value> = Vec::new();
            let mut name_aliases: Vec<(String, Option<u16>)> = Vec::new();
            if is_struct && self.lexer.peek_token("{") {
                self.parse_match_enum_field_bindings(
                    variant_def_nr,
                    &pattern_name,
                    &subject_val,
                    &mut arm_stmts,
                    &mut field_conditions,
                    &mut name_aliases,
                );
            }

            // @PLN35 Phase 3 (P-Multi): a `,` here — before the guard/`=>` — begins
            // ANOTHER whole pattern for the SAME arm (a multi-pattern arm).  In a
            // single-pattern arm the next token is `=>` or `if`; a `,` before the
            // arrow is otherwise a parse error, so this collection is purely
            // additive and leaves every existing path untouched.  Each listed
            // pattern binds its captures into shared slots — the first pattern's
            // above, plus one per name a later pattern adds — whichever variant
            // matches assigns those slots from ITS OWN offsets, and the one arm body
            // reads them.  A name some pattern lacks is `τ?` (`@FR-P-Alt-Diff`).
            // Emitted as one `if disc==Vi { binds_i; body }` branch per pattern —
            // identical to hand-expanding into separate single-pattern arms.
            let mut multi_branches: Vec<(i32, Vec<Value>, Vec<Value>)> = Vec::new();
            // The variants this arm's EXTRA patterns marked covered, so a guard parsed after
            // them can take the marks back (`@FR-M-Total`: a guarded arm covers nothing).
            let mut multi_covered: Vec<u32> = Vec::new();
            if (self.lexer.peek_token(",") || self.lexer.peek_token("|"))
                && valid_enum
                && e_nr != u32::MAX
            {
                let mut shared: std::collections::HashMap<String, (u16, Type)> = name_aliases
                    .iter()
                    .filter_map(|(name, _)| {
                        let vn = self.vars.var(name);
                        (vn != u16::MAX).then(|| (name.clone(), (vn, self.vars.tp(vn).clone())))
                    })
                    .collect();
                let first_names: HashSet<String> = shared.keys().cloned().collect();
                let mut branch_names: Vec<HashSet<String>> = Vec::new();
                while self.lexer.has_token(",") || self.lexer.has_token("|") {
                    if self.lexer.peek_token("=>") || self.lexer.peek_token("}") {
                        break; // dangling comma / trailing arm separator
                    }
                    let Some(vid) = self.lexer.has_identifier() else {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "expect a variant name after ',' in a multi-pattern arm"
                            );
                        }
                        break;
                    };
                    let vname = if self.lexer.has_token("::") {
                        self.lexer.has_identifier().unwrap_or_else(|| vid.clone())
                    } else {
                        vid.clone()
                    };
                    let mut ev = self.data.variant_of(e_nr, &vname);
                    if ev == u32::MAX {
                        ev = self.data.def_nr(&vname);
                    }
                    if ev == u32::MAX
                        || self.data.def_type(ev) != DefType::EnumValue
                        || self.data.def(ev).parent() != e_nr
                    {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "'{}' is not a variant of {}",
                                vname,
                                self.data.def(e_nr).name()
                            );
                        }
                        continue;
                    }
                    let disc = self.variant_disc(e_nr, is_struct, ev, &vname);
                    let mut stmts_i: Vec<Value> = Vec::new();
                    let mut conds_i: Vec<Value> = Vec::new();
                    let names_i = if self.lexer.peek_token("{") {
                        self.parse_multi_pattern_extra_bindings(
                            ev,
                            &vname,
                            &subject_val,
                            &mut shared,
                            &mut stmts_i,
                            &mut conds_i,
                            &mut name_aliases,
                        )
                    } else {
                        HashSet::new()
                    };
                    branch_names.push(names_i);
                    // Union coverage (M-Total): each listed total pattern counts
                    // toward exhaustiveness, exactly like the `|` or-pattern arm.
                    // A pattern with a field sub-pattern can fail on its field, so, like a
                    // guarded arm, it covers nothing (`@FR-M-Total`).
                    if !self.first_pass && conds_i.is_empty() && covered.insert(ev) {
                        multi_covered.push(ev);
                    }
                    multi_branches.push((disc, stmts_i, conds_i));
                }
                // `@FR-P-Multi` is `@FR-P-Alt` at arm granularity, so `@FR-P-Alt-Diff` holds: a
                // name some listed pattern does not bind is `τ?`, and reads null when that
                // pattern matched.  Each pattern's bindings are its own statements, so the
                // null goes into the statements of every pattern that lacks the name.
                let mut partial: Vec<(String, u16, Type)> = shared
                    .iter()
                    .filter(|(name, _)| {
                        !first_names.contains(*name)
                            || branch_names.iter().any(|b| !b.contains(*name))
                    })
                    .map(|(name, (v_nr, tp))| (name.clone(), *v_nr, tp.clone()))
                    .collect();
                partial.sort_by_key(|(_, v_nr, _)| *v_nr);
                for (name, v_nr, tp) in partial {
                    let opt = Self::capture_optional(&tp);
                    self.vars.set_type(v_nr, opt.clone());
                    if !first_names.contains(&name) {
                        arm_stmts.push(v_set(v_nr, self.absent_capture(&opt)));
                    }
                    for (names_i, (_, stmts_i, _)) in
                        branch_names.iter().zip(multi_branches.iter_mut())
                    {
                        if !names_i.contains(&name) {
                            stmts_i.push(v_set(v_nr, self.absent_capture(&opt)));
                        }
                    }
                }
            }

            // parse optional guard clause after pattern + field bindings.
            // Field-bound variables are in scope for the guard expression.
            let guard_opt = self.parse_optional_guard();
            // The guard alone, before the field conditions join it: it holds whichever
            // pattern matched, so it is what the body may assume.
            let user_guard = guard_opt.clone();
            // @FR-P-Guard × @FR-P-Multi, @C162 — a guard on a multi-pattern arm is asked of each
            // listed pattern that matches, in order: each extra pattern's branch below carries a
            // clone of it, run after that branch's own bindings, so a guard that rejects the
            // first pattern's bindings lets the next pattern try.  It was refused ("not yet supported") while the
            // two rules it composes were each implemented.  A guard can reject, so the arm
            // covers none of its variants — the extra patterns' coverage marks come back out.
            let multi_guard = guard_opt.clone().filter(|_| !multi_branches.is_empty());
            if multi_guard.is_some() {
                for ev in multi_covered.drain(..) {
                    covered.remove(&ev);
                }
            }
            // L2: combine field sub-pattern conditions with the explicit guard (if any).
            let guard_opt = if field_conditions.is_empty() {
                guard_opt
            } else {
                let mut combined = field_conditions.remove(0);
                for c in field_conditions {
                    combined = v_if(combined, c, Value::Boolean(false));
                }
                // @FR-P-Guard — the guard runs with the arm's bindings in scope and, when it
                // is false, the arm fails exactly as if the pattern had not matched: ANDing it
                // into the arm's own condition is what makes selection move on to the next arm
                // rather than commit this one.
                //
                // If there's also an explicit `if` guard, AND them.
                if let Some(g) = guard_opt {
                    combined = v_if(combined, g, Value::Boolean(false));
                }
                Some(combined)
            };

            // Duplicate arm detection.
            // Guarded arms don't count as covering the variant for exhaustiveness.
            //
            // @FR-M-Match — selection takes the FIRST arm whose pattern matches, so a variant
            // named twice makes the later arm dead; that is what `covered` reports.
            // @FR-M-Total — and it is the same set that decides coverage, which is why the
            // guarded arm is excluded from BOTH: a guard can reject, so a guarded arm neither
            // covers its variant nor makes a later one unreachable.  One test, one insertion,
            // so the two answers cannot drift apart.
            if guard_opt.is_none() {
                if covered.contains(&variant_def_nr) {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Warning,
                            code = "unreachable-match-arm",
                            "unreachable arm: {} already matched",
                            pattern_name
                        );
                        self.lexer.fix_last(crate::diagnostics::Fix {
                            kind: crate::diagnostics::FixKind::Mechanical,
                            title: "delete the arm — an earlier one already matches".to_string(),
                            condition: None,
                            edit: None,
                            concept: "pattern matching",
                            concept_ref: "@F29",
                        });
                    }
                } else {
                    covered.insert(variant_def_nr);
                }
            }

            self.expect_match_arm_arrow();

            // Parse the arm body expression.
            // If the body starts with `{`, parse it as a scoped block so
            // the closing `}` is not confused with the match's `}`.
            // Save/restore write tracking so writes in one arm don't cause
            // false dead-assignment warnings in sibling arms.
            let mut arm_body = Value::Null;
            let arm_expected = self.match_arm_expected(&result_type);
            let facts = self.assume_guard(user_guard.as_ref());
            let mut arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_body);
            self.forget_facts(facts);
            // @PLN85 match_return (LOFT_JOIN_OWN): if this arm yields a borrowed-view
            // vector field binding DIRECTLY (`Filled { items } => { items }`), wrap it in
            // an owned copy `{ o = []; o += items; o }` so the value ESCAPES OWNED — the
            // `deliver3` structure. The existing promotion then builds the separate-buffer
            // ABI the caller adopts + frees the argument. No-op for non-matching arms.
            // Updating `arm_type` keeps cross-arm unification reading the OWNED type.
            if let Some(owned) = self.jo_copy_borrowed_arm_yield(&mut arm_body, &arm_type) {
                arm_type = owned;
            }

            // S15: restore name mappings after arm body so the next arm can
            // create its own alias for the same field name.
            for (name, old) in name_aliases.drain(..) {
                if let Some(old_nr) = old {
                    self.vars.set_name(&name, old_nr);
                } else {
                    self.vars.remove_name(&name);
                }
            }

            // Type unification across arms.  A `null` arm (Type::Null) lowers to
            // the result type's null sentinel — it unifies with any sibling type
            // and never pins the result, so the first CONCRETE arm wins even when
            // a `null` arm comes first (`Jade => null, Crimson => S{…}`).  Treat a
            // current `Null` result like `Void` for promotion, and skip the unify
            // check when this arm is itself `null`.  Without this, struct-or-null
            // enum matches were rejected ("cannot unify: S and null", #365).
            // loft#978 — every arm can deliver this match's value, so the result carries
            // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
            // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
            result_type = self.join_arm_into(&result_type, &arm_body, &arm_type);
            self.match_void_arm |= matches!(arm_type.base(), Type::Void);
            if Self::arm_settles_match(&result_type, &arm_type) {
                result_type = arm_type.clone();
            } else if let Some(joined) =
                self.join_null_tuple_arm(&result_type, &mut arm_body, &mut arm_type)
            {
                self.reconvert_null_tuple_arms(&mut arms, &joined);
                result_type = joined;
            } else if !self.first_pass
                && arm_type != Type::Void
                && arm_type != Type::Null
                && arm_type != Type::Never
                && !self.arm_convert_reported
                && !self.match_arms_unify(&result_type, &arm_type)
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot unify: {} and {}",
                    result_type.source_name(&self.data),
                    arm_type.source_name(&self.data)
                );
            }

            // @PLN35 Phase 3: capture the raw arm body + type for the extra
            // multi-pattern branches before the single-arm assembly below consumes
            // them.  A multi-pattern arm carries no field sub-pattern conditions
            // (rejected above), so each branch is `block(binds_i; body)`, or its binds
            // kept apart and the guard beside them when the arm has one.
            let multi_extra: Option<(Value, Type)> =
                (!multi_branches.is_empty()).then(|| (arm_body.clone(), arm_type.clone()));

            // When there is a guard, keep field bindings separate — they must
            // be emitted before the guard check so bound variables are available.
            // When there is no guard, wrap them into a block as before.
            let (arm_code, binding_stmts) = if guard_opt.is_some() && !arm_stmts.is_empty() {
                (arm_body, arm_stmts)
            } else if arm_stmts.is_empty() {
                (arm_body, Vec::new())
            } else {
                arm_stmts.push(arm_body);
                (
                    v_block(arm_stmts, arm_type.clone(), "match_arm"),
                    Vec::new(),
                )
            };

            arms.push(EnumArm {
                discs: all_discs,
                code: arm_code,
                tp: arm_type,
                guard: guard_opt,
                bindings: binding_stmts,
                cond: None,
            });
            // @PLN35 Phase 3: emit one arm per EXTRA listed pattern, each binding
            // the shared slots from its own variant offsets then running a CLONE of
            // the arm body — the hand-expanded form the single arm above equals.
            if let Some((body, tp)) = multi_extra {
                for (disc, stmts_i, conds_i) in multi_branches {
                    // #673 — the clone carries the FIRST pattern's `text` payload
                    // write-backs, but this branch binds the shared slots from its own
                    // variant's offsets.  Retarget the write-backs to the place this
                    // branch actually read, or `B { items }` reads its own field and
                    // writes the result into `A`'s.
                    let mut body_i = body.clone();
                    self.retarget_text_payload_writes(&mut body_i, &stmts_i);
                    // This branch's own field conditions, then the arm's guard, all under
                    // this branch's bindings.
                    let branch_guard = conds_i
                        .into_iter()
                        .chain(multi_guard.clone())
                        .reduce(|a, b| v_if(a, b, Value::Boolean(false)));
                    if let Some(guard) = &branch_guard {
                        let mut guard_i = guard.clone();
                        self.retarget_text_payload_writes(&mut guard_i, &stmts_i);
                        arms.push(EnumArm {
                            discs: vec![disc],
                            code: body_i,
                            tp: tp.clone(),
                            guard: Some(guard_i),
                            bindings: stmts_i,
                            cond: None,
                        });
                        continue;
                    }
                    let code_i = if stmts_i.is_empty() {
                        body_i
                    } else {
                        let mut ops = stmts_i;
                        ops.push(body_i);
                        v_block(ops, tp.clone(), "match_arm")
                    };
                    arms.push(EnumArm {
                        discs: vec![disc],
                        code: code_i,
                        tp: tp.clone(),
                        guard: None,
                        bindings: Vec::new(),
                        cond: None,
                    });
                }
            }
            self.match_arm_separator();
        }

        self.lexer.token("}");

        // @FR-M-Exhaust — a match on an enum covers EVERY variant, or carries a `_`; a match
        // that forgets one is a STATIC error naming what is missing, never a runtime fault.
        // The `covered` set it reads is filled by the arm loop above under @FR-M-Total's rule
        // (an unguarded arm only), so the two halves of the totality judgment share one fact.
        //
        // Exhaustiveness check (second pass only, when no wildcard, when subject is a known enum).
        if !self.first_pass && !has_wildcard && !recovered_catch_all && valid_enum {
            let missing: Vec<String> = self
                .data
                .definitions
                .iter()
                .enumerate()
                .filter(|(_, d)| d.def_type == DefType::EnumValue && d.parent == e_nr)
                .filter(|(v_nr, _)| !covered.contains(&(*v_nr as u32)))
                .map(|(_, d)| d.name.clone())
                .collect();
            if !missing.is_empty() {
                let msg = format!(
                    "match on {} is not exhaustive — missing: {}; add the missing variants or a '_ =>' wildcard",
                    self.data.def(e_nr).name(),
                    missing.join(", ")
                );
                self.lexer.pos_diagnostic(Level::Error, &match_pos, &msg);
            }
        }

        // A `null` arm lowers to the result type's null sentinel — `parse_if`
        // and `build_scalar_chain` do the same.  Now that
        // result_type is final, convert bare-null (and block-trailing-null) arm
        // bodies, and keep `arm.tp` in step so the guarded-binding block wrapper
        // (below) declares the right result type.  Without this a `null` arm
        // pushes nothing and the if-chain join reads an unwritten, value-sized
        // slot (interp stack underflow / native lost value) — the #365 family.
        //
        // loft#936 — `null_value`, not `null`.  `null` doubles as a VARIABLE's
        // default-init, where a collection must be an allocated empty store, so
        // its catch-all answers a bare `Value::Null` for the whole collection
        // family — which is the very "pushes nothing" this paragraph forbids.
        // `match n { 0 => null, _ => [n] }` therefore left the ARMS at different
        // eval-stack depths and `gives(3)` read back an empty vector on
        // `--interpret` while `--native` answered `[3]`.
        let base = if matches!(result_type, Type::Void | Type::Null) {
            Value::Null
        } else {
            let first_pass = self.first_pass;
            for arm in arms
                .iter_mut()
                .filter(|a| Self::is_null_arm(first_pass, &a.code, &a.tp))
            {
                arm.tp = result_type.clone();
            }
            self.type_null_arms(
                arms.iter_mut()
                    .filter(|a| !Self::is_placeholder_arm(first_pass, &a.tp))
                    .map(|a| &mut a.code),
                &result_type,
            );
            for arm in arms
                .iter_mut()
                .filter(|a| Self::is_placeholder_arm(first_pass, &a.tp))
            {
                mark_forward_call(&mut arm.code);
            }
            let typed_null = self.null_value(&result_type);
            // Seed the chain base with the typed null too: an exhaustive enum
            // match's innermost else is unreachable, but codegen still emits it
            // and it must balance the value-sized stack slot the arms push.
            typed_null
        };

        // Build the if-chain from the collected arms (last to first).
        // `base` is reached only when no arm matches (only possible if
        // exhaustiveness fails, which is a compile error) — but it still has to
        // typecheck and balance the stack, so it carries the typed null.
        let mut chain = base;
        for arm in arms.iter().rev() {
            if arm.discs.is_empty() && arm.cond.is_none() {
                // Wildcard — always taken; becomes the else branch of the chain.
                // guarded wildcard wraps body in If(guard, body, chain_rest).
                chain = match &arm.guard {
                    Some(guard) => v_if(guard.clone(), arm.code.clone(), chain),
                    None => arm.code.clone(),
                };
            } else {
                // An arm that carries its own test uses it; otherwise build the OR'd
                // comparison over this arm's discriminants.  The `null` arm of a nullable
                // HEAP subject is the first kind: absence is the store-pointer sentinel
                // there, which no discriminant names.
                let cmp = if let Some(c) = &arm.cond {
                    c.clone()
                } else {
                    let mut cmp =
                        self.cl("OpEqInt", &[disc_expr.clone(), Value::Int(arm.discs[0])]);
                    for &d in &arm.discs[1..] {
                        let next = self.cl("OpEqInt", &[disc_expr.clone(), Value::Int(d)]);
                        cmp = v_if(cmp, Value::Boolean(true), next);
                    }
                    cmp
                };
                chain = match &arm.guard {
                    Some(guard) => guarded_arm(
                        cmp,
                        arm.bindings.clone(),
                        guard.clone(),
                        arm.code.clone(),
                        chain,
                        &arm.tp,
                    ),
                    None => v_if(cmp, arm.code.clone(), chain),
                };
            }
        }

        // When not a valid enum, emit nothing (errors were already reported) — as an empty
        // BLOCK, because a `match` is a brace statement: the statement loop asks the lowered
        // node whether a `;` is due, and a `Null` there demanded one after the `}` on pass 1
        // of a valid file whose enum is declared lower down.
        if !valid_enum {
            *code = v_block(Vec::new(), Type::Void, "unresolved match");
            if subject_unresolved {
                // ...except on the FIRST pass nothing was reported: the diagnostic above is
                // `!first_pass`-gated, because the subject may simply be declared lower in the
                // file.  `Void` here is what locks a local bound to the match, so pass 2's real
                // type is then REFUSED ("cannot change type from void to integer").
                return Type::Unknown(0);
            }
            return Type::Void;
        }

        // Emit the match:
        // - Plain enum: { match_subj = subject; chain }  (temp var to eval subject once)
        // - Struct enum: chain only  (subject_val is already the original expression/var)
        // L2: hoisted bindings are prepended so field reads happen before the if-chain.
        // @PLN25 DN1: a user `=> null` arm makes the match NULLABLE — widen the result to
        // `Optional(τ)` (τ the non-null scalar of the other arms), so the existing DN3
        // `(N-Store)` forces the caller to declare `τ?` or discharge. Check each USER arm's
        // TYPE (`a.tp == Null` for a bare-null arm); an arm whose value yields null through its
        // own sub-expression already carries that in `a.tp`, which the arm-join folded into
        // `result_type` (so it is `Optional`, and `is_non_null_scalar` is already false).
        // Do NOT `branch_yields_null(&a.code)` — descending the arm's LOWERED code reaches a
        // NESTED exhaustive match's synthesised unreachable `OpConv*FromNull` default and
        // falsely widens (p54: `Wrap { inner } => match inner { Leaf { v } => v }` typed `τ?`).
        if crate::keys::pln25_dn1_enabled()
            && Self::null_arm_widens(&result_type)
            && arms
                .iter()
                .any(|a| matches!(a.tp, Type::Null) || self.arm_yields_direct_null(&a.code))
        {
            result_type = Type::optional(result_type);
        }
        *code = if !hoisted_bindings.is_empty() || preamble.is_some() {
            let mut stmts = Vec::new();
            if let Some((v, init)) = preamble {
                stmts.push(v_set(v, init));
            }
            stmts.append(&mut hoisted_bindings);
            stmts.push(chain);
            v_block(stmts, result_type.clone(), "match")
        } else {
            chain
        };
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_type);
        result_type
    }

    /// The type a match arm is expected to answer in: what the arms have agreed on so
    /// far, or `Unknown` while nothing is settled yet.
    /// `(Slice-Value)` — an `if` or `match` ARM whose value is a vector slice answers the fresh
    /// vector a bind would make of it ([`Parser::iterator_as_vector`]), so its siblings join a
    /// vector rather than the iterator the slice is lowered from (loft#1941).  Any other arm is
    /// returned as it is.
    pub(super) fn slice_arm_as_vector(&mut self, code: &mut Value, tp: Type) -> Type {
        if let Type::Iterator(elm, _) = tp.base()
            && Self::slice_shaped(code)
        {
            let vec_tp = Type::Vector(elm.clone(), crate::data::Deps::none());
            if let Some(t) = self.iterator_as_vector(code, &tp, &vec_tp) {
                // The arm answers the materialised LOCAL, whose store is a function-scoped
                // backing (`["__vdb_N"]`): its type carries that dep, and the arm's must too.
                // Typed dep-free, a `match` whose FIRST arm was the slice gave its result the
                // dep-free type, so the bound local read as the store's OWNER and freed it at
                // the end of every round while the backing still named it — a use after free
                // on `--native` from the second round on.
                if !self.first_pass
                    && let Some(tail) = Self::block_tail_var(code)
                {
                    let held = self.vars.tp(tail).clone();
                    if matches!(held.base(), Type::Vector(..)) {
                        Self::retype_tail_blocks(code, &held);
                        return held;
                    }
                }
                return t;
            }
        }
        tp
    }

    /// The variable a value's tail is, through spans and block tails.
    fn block_tail_var(code: &Value) -> Option<u16> {
        match code.unspan() {
            Value::Var(v) => Some(*v),
            Value::Block(bl) => bl.operators.last().and_then(Self::block_tail_var),
            _ => None,
        }
    }

    /// Give every block on the path to a value's tail the type `tp`.
    fn retype_tail_blocks(code: &mut Value, tp: &Type) {
        if let Value::Block(bl) = code.unspan_mut() {
            bl.result = tp.clone();
            if let Some(last) = bl.operators.last_mut() {
                Self::retype_tail_blocks(last, tp);
            }
        }
    }

    pub(super) fn match_arm_expected(&self, result_type: &Type) -> Type {
        if result_type.is_unknown()
            || Self::match_result_unsettled(result_type)
            || self.tuple_join_open(result_type)
        {
            Type::Unknown(0)
        } else {
            Self::arm_expectation(result_type)
        }
    }

    /// Parse one match arm's body in the type its SIBLING arms answer in.
    ///
    /// A match arm is an else arm: `@FR-N-Decl` checks the destination against the whole
    /// construct, so every arm converts to the construct's type at its own tail exactly as
    /// `parse_block("else", …)` converts an `if`'s else arm.  Without the expected type an
    /// arm was neither converted nor refused — a float arm's bits read as an integer, an
    /// integer arm's as a float, and `250 + 10` reached a `u8` local holding 260, silently
    /// on both backends and for every subject kind (loft#1380's `match` twin).
    ///
    /// A `{ … }` arm is a block, and `block_result` performs the conversion with the
    /// carve-outs it already applies to `else`.  A BARE arm (`1 => x`) has no block tail,
    /// so the same conversion is asked here through the same `convert_admitting` /
    /// `validate_convert` pair, and the carve-outs are restated in the same order.
    /// `@FR-M-Sep` — the separator after a match arm: a comma, which may be left out before the
    /// closing `}` and after an arm whose body is a block (`=> { … }`), as in Rust.
    pub(super) fn match_arm_separator(&mut self) {
        if self.lexer.peek_token("}") || self.arm_body_was_block {
            self.lexer.has_token(",");
        } else {
            self.lexer.token(",");
        }
    }

    pub(super) fn parse_match_arm_body(&mut self, expected: &Type, arm_code: &mut Value) -> Type {
        // A bare `null` arm, told from the SOURCE token; a block arm reports through
        // `block_tail_null_literal`, read in `parse_match_arm_body_inner`.
        let bare_null = self.lexer.peek_token("null");
        // Each arm runs INSTEAD of its siblings, so the dead-store tracking starts every arm from
        // the state before the `match` and leaves it there — as `parse_if` does for its arms.
        // Asked here, the one body every arm kind parses through: held at the arm sites, three
        // of seven had it, and in the others one arm's write read as overwritten by the next.
        let arm_write_state = self.vars.save_and_clear_write_state();
        self.vars.clear_write_state();
        let block_arm = self.lexer.peek_token("{");
        let tp = self.parse_match_arm_body_inner(expected, arm_code);
        // `@FR-C-Never` — a halt arm (`_ => panic(…)`) never returns: it is no void arm, and it
        // settles nothing about the match's type.  `type_null_arms` gives it the result type's
        // null after the halt once that type is known (loft#2260).
        let tp = if matches!(tp.base(), Type::Void) && ends_in_halt(&self.data, arm_code) {
            Type::Never
        } else {
            tp
        };
        // `(Slice-Value)` — an arm's value is a value position, so a vector slice there is the
        // fresh vector (loft#1941).
        let tp = self.slice_arm_as_vector(arm_code, tp);
        if self.abstract_on() {
            let fact = std::mem::take(&mut self.operand_fact);
            self.match_arm_facts.push(fact); // @PLN187
        }
        if (bare_null && matches!(arm_code.unspan(), Value::Null))
            || (block_arm && self.block_tail_null_literal)
        {
            self.null_literal_arms += 1;
        }
        self.vars.restore_write_state(&arm_write_state);
        self.end_pattern_arm();
        // `@FR-M-Sep` — a body written as a block needs no comma after it.  Set last, so a
        // `match` nested in this body cannot leave its own arm's answer behind.
        self.arm_body_was_block = block_arm;
        tp
    }

    fn parse_match_arm_body_inner(&mut self, expected: &Type, arm_code: &mut Value) -> Type {
        // Cleared AFTER the body is parsed on both paths, never at entry: a nested `match`
        // inside this arm runs its own arms through here, so a flag set at entry would still
        // carry that inner arm's answer when THIS one is asked about.  From each clear to the
        // gate that reads it nothing else parses, so the field describes exactly this arm.
        if self.lexer.peek_token("{") {
            let tp = self.parse_block("match_arm", arm_code, expected);
            // A block arm reports through `block_result`, which hands back the EXPECTED type
            // on a failure — so the cross-arm gate sees the arms agreeing and stays silent
            // without being told.  Cleared all the same: the field is this arm's answer, not
            // the previous arm's, and a caller must not have to know which path reports.
            self.arm_convert_reported = false;
            return tp;
        }
        // loft#1540 — the sibling's `const` parameters are set aside for the conversion, and
        // the arm answers with its own: the join is what BOTH declare (`join_arm_into`), as a
        // block arm's is (`parse_block`).
        let sibling_fn;
        let expected = if expected
            .function_consts()
            .is_some_and(|c| c != crate::data::ConstParams::NONE)
        {
            sibling_fn = expected.with_function_consts(crate::data::ConstParams::NONE);
            &sibling_fn
        } else {
            expected
        };
        let at = *self.lexer.pos();
        // A bare arm is a block arm without the braces, so its value is hinted the way a
        // block tail's is: without it an empty `[]` arm had no element type to build with and
        // lowered to the placeholder `Insert([Null])`, which the chain emitted as a jump over
        // nothing — the arm after it fell through into the placeholder's null and answered
        // null (interpreter), or rustc refused the `()` (native).  `0 => { [] }` was right.
        let saved_expected = self.expected.clone();
        self.seed_leaving_value_hint(expected);
        let t = self.expression(arm_code);
        self.expected = saved_expected;
        self.arm_convert_reported = false;
        // loft#2188 — a bare arm is a block arm without the braces, so a call that may hand
        // back an argument is lifted here as `block_result` lifts a block arm's: on both passes
        // (pass 1 reports its type), and before the unknown-expected exit, because the FIRST arm
        // has no sibling type yet and is lifted all the same.
        if let Some(lifted) = self.lift_borrowing_call_value(&t, arm_code) {
            return lifted;
        }
        if self.first_pass || expected.is_unknown() {
            return t;
        }
        // A void or null arm carries no value for the siblings to disagree about; a bare
        // `null` lowers to the result type's sentinel once the result is settled.
        if matches!(t, Type::Void | Type::Null) {
            return t;
        }
        // @FR-C-Var — two variants of one enum join to the ENUM and nothing is licensed
        // between them, so the arm keeps its own shape and the join above still sees that
        // the two differ.  `block_result` carves the same case out for `else`.
        if self.arm_joins_to_enum(&t, expected) {
            return t;
        }
        // @FR-I-Widen — an arm wider than its siblings widens the join, and the match site
        // converts the siblings (`join_open_arm`); `block_result` carves out the same case.
        if Self::numeric_join(expected, &t).is_some() {
            return t;
        }
        // A struct-enum pattern binding yields a BORROW — `Ship { carrier } => carrier` is
        // `&text` — while a sibling arm commonly yields the owned twin (`_ => "none"`).  That
        // is one type modulo ownership: the caller reads the value either way, which is why
        // `match_arm_types_unify` strips the wrapper rather than requiring the two to agree.
        // So it is not a conversion question, and asking `convert` for it re-points the
        // sibling arm's value — the extractor pattern's wildcard stopped answering its own
        // literal (`loft-suite/attic/scripts/35-nested-match.loft`).  A `&τ` on EITHER side is passed
        // through: an arm whose borrow also needs a width change keeps the gap the wrapper
        // already had, and narrowing it further needs the borrow to convert, not this site.
        if matches!(t, Type::RefVar(_)) || matches!(expected, Type::RefVar(_)) {
            return t;
        }
        // @FR-F-Block — the arms of a construct in STATEMENT position yield nothing anybody
        // reads, so their types need not agree.
        if self.arms_of_statement_construct && matches!(expected, Type::Void) {
            return t;
        }
        if !self.convert_admitting(arm_code, &t, expected) {
            self.validate_convert("match_arm", &t, expected, &at);
            self.arm_convert_reported = true;
            return t;
        }
        // loft#1103 — the SHAPE is the expected type's, the NULLABILITY the arm's own:
        // `(N-Join)` makes the construct optional iff some arm is.
        let honest = expected.with_deps_of(&t);
        let honest = match t.function_consts() {
            Some(own) if honest.function_consts().is_some() => honest.with_function_consts(own),
            _ => honest,
        };
        if crate::keys::pln25_dn1_enabled()
            && matches!(t, Type::Optional(_))
            && !matches!(honest, Type::Optional(_))
        {
            Type::optional(honest)
        } else {
            honest
        }
    }

    /// Whether a match's running result type is still UNSETTLED, so the next arm's type takes
    /// it over instead of joining with it.  The one home for the question every arm site asks
    /// — seven sites re-asked it with their own subset, and the ones missing `Null` let a
    /// `null` first arm pin the whole chain to `Null`: every later arm then answered null,
    /// silently, on both backends (loft#1711).
    ///
    /// `Void`, `Null` and `Never` settle nothing: a `null` arm lowers to the result type's
    /// null once the result is known (`match c { false => null, true => S{…} }` answers `S`),
    /// a diverging arm answers no value, and a `Void` result is either the initial value or a
    /// statement `match` whose arms yield nothing — the same "expect nothing" an `else if`
    /// chain passes down for a `Void` then arm.
    /// Does this arm's type become the match's result?  Only where no arm has settled it
    /// (`match_result_unsettled`), and never by a diverging arm over a type an earlier arm
    /// gave: a `Never` arm converts to the result (`@FR-C-Never`), it does not decide it
    /// (loft#2444).  An arm the first pass could not type yet (`Unknown`) or a `null` arm
    /// keeps the result, so `bl = match c { V { b } => b[0], _ => { return; } }` types `bl`
    /// by the first arm on both passes.  The `Void` a match starts from is no arm's type: a `Never` arm still
    /// settles it, so a match whose every arm diverges is `Never`.
    pub(super) fn arm_settles_match(result_type: &Type, arm_type: &Type) -> bool {
        Self::match_result_unsettled(result_type)
            && !(matches!(arm_type.base(), Type::Never)
                && matches!(result_type.base(), Type::Unknown(_) | Type::Null))
    }

    fn match_result_unsettled(result_type: &Type) -> bool {
        // `Unknown`: an arm the first pass could not type yet — a bare variant two enums own
        // (@C157) — so the arms after it settle the match, as an `if`'s else arm settles it.
        matches!(
            result_type.base(),
            Type::Void | Type::Null | Type::Never | Type::Unknown(_)
        )
    }

    /// Give every `null` arm of a value-producing match chain the result type's typed null.
    /// The one home for the repair, asked by every chain kind — scalar, enum, tuple and
    /// vector subjects.
    ///
    /// A bare `null` arm (`0 => null`, or `{ null }`) parses to `Value::Null`, which pushes
    /// NOTHING, while its siblings push a value; the if-chain join then reads a value-sized
    /// slot one arm never wrote.  Held at two of the four chain kinds, the tuple and vector
    /// chains kept the bare null, and on the interpreter the arm AFTER it fell through into
    /// the join's padding and answered null (loft#1711).  `null_value` and not `null`
    /// (loft#936): `null` is also a variable's default-init, which answers a bare
    /// `Value::Null` for the whole collection family.  A no-op for a `Void`/`Null` result,
    /// where the match is a statement and nothing is read.
    pub(super) fn type_null_arms<'a>(
        &mut self,
        codes: impl Iterator<Item = &'a mut Value>,
        result_type: &Type,
    ) {
        if matches!(result_type.base(), Type::Void | Type::Null) {
            return;
        }
        let mut typed_null = None;
        for code in codes {
            if arm_body_is_null(code) {
                let typed_null = typed_null.get_or_insert_with(|| self.null_value(result_type));
                set_arm_null_typed(code, typed_null, result_type);
            } else if ends_in_halt(&self.data, code) {
                // `@FR-C-Never` — the halt, then a value of the result type no run reaches.
                let typed_null = typed_null.get_or_insert_with(|| self.null_value(result_type));
                give_halt_a_value(code, typed_null.clone(), result_type);
            }
        }
    }

    /// Parse a wildcard (`_`) arm in a match expression.
    /// Returns the arm and whether it is exhaustive (no guard).
    fn parse_match_wildcard_arm(&mut self, result_type: &mut Type) -> (EnumArm, bool) {
        let guard_opt = self.parse_optional_guard();
        // @FR-M-Total — `total(pat if cond) = false`: a guard can reject after the pattern has
        // matched, so a guarded `_` secures nothing and the arms after it are reachable.  This
        // one line is why `_ if cond` is not treated as the last arm.
        let is_exhaustive = guard_opt.is_none();
        self.expect_match_arm_arrow();
        let mut arm_code = Value::Null;
        let arm_expected = self.match_arm_expected(result_type);
        let facts = self.assume_guard(guard_opt.as_ref());
        let mut arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_code);
        self.forget_facts(facts);
        // loft#978 — see the arm sites above: the wildcard is an arm like any other.
        let joined = self.join_arm_into(result_type, &arm_code, &arm_type);
        *result_type = joined;
        self.match_void_arm |= matches!(arm_type.base(), Type::Void);
        if Self::arm_settles_match(result_type, &arm_type) {
            *result_type = arm_type.clone();
        } else if let Some(joined) =
            self.join_null_tuple_arm(&result_type.clone(), &mut arm_code, &mut arm_type)
        {
            // loft#1682 — the caller reconverts the arms it holds (`parse_match_inner`).
            *result_type = joined;
        } else if !self.first_pass
            && arm_type != Type::Void
            // loft#2170 — a `_ => null` arm is the null arm every named arm may be: it takes
            // the result's typed null once the result type is final (`type_null_arms`), so it
            // is not unified here.  Asked, it was refused as "cannot unify: integer and null"
            // while `Var { .. } => null` beside the same arms was accepted.
            && arm_type != Type::Null
            && arm_type != Type::Never
            && !self.arm_convert_reported
            && !self.match_arms_unify(result_type, &arm_type)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot unify: {} and {}",
                result_type.source_name(&self.data),
                arm_type.source_name(&self.data)
            );
        }
        let arm = EnumArm {
            discs: vec![],
            code: arm_code,
            tp: arm_type,
            guard: guard_opt,
            bindings: Vec::new(),
            cond: None,
        };
        (arm, is_exhaustive)
    }

    /// The fields of a plain-struct SUB-pattern `S { f: p, g }` over `subject` (loft#1870): each
    /// `f: p` adds `p`'s condition, each bare `g` binds the field — the field loop of
    /// [`Self::parse_match_struct_arm`], with the bindings and conditions going to the
    /// enclosing arm.
    pub(super) fn parse_struct_sub_pattern(
        &mut self,
        s_nr: u32,
        subject: &Value,
        arm_stmts: &mut Vec<Value>,
        field_conditions: &mut Vec<Value>,
        name_aliases: &mut Vec<(String, Option<u16>)>,
    ) {
        if !self.lexer.has_token("{") {
            return;
        }
        while !self.lexer.peek_token("}") {
            if let Some(field_name) = self.lexer.has_identifier() {
                let attr_idx = self.data.attr(s_nr, &field_name);
                if attr_idx != usize::MAX {
                    self.check_visibility("pattern-field", s_nr, attr_idx);
                }
                if attr_idx == usize::MAX {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "unknown field '{}' on struct {}",
                            field_name,
                            self.data.def(s_nr).name()
                        );
                    }
                } else {
                    let field_val = self.get_field(s_nr, attr_idx, subject.clone());
                    let field_type = self.data.attr_type(s_nr, attr_idx);
                    if self.lexer.has_token(":") {
                        if let Some(bind_name) = self.field_pattern_rename() {
                            let v = self.pattern_binding(&bind_name, &field_type);
                            self.vars.defined(v);
                            let bound = self.pattern_field_value(s_nr, attr_idx, subject.clone());
                            arm_stmts.push(v_set(v, bound));
                            self.mark_field_capture_view(v, &field_type, subject);
                        } else if let Some(cond) = self.parse_field_sub_pattern(
                            field_val,
                            &field_type,
                            arm_stmts,
                            field_conditions,
                            name_aliases,
                        ) {
                            field_conditions.push(cond);
                        }
                    } else {
                        let v = self.pattern_binding(&field_name, &field_type);
                        self.vars.defined(v);
                        let bound = self.pattern_field_value(s_nr, attr_idx, subject.clone());
                        arm_stmts.push(v_set(v, bound));
                        self.mark_field_capture_view(v, &field_type, subject);
                    }
                }
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
    }

    /// `@FR-P-Cap` at a match arm's ROOT: `whole: Rect { w, h }`, and `other: _` for the
    /// catch-all.  The name binds the matched subject — the same value whichever arm runs, so
    /// the binding is hoisted ahead of the arm chain with the others.  Like every pattern
    /// capture it is a VIEW of what it names (`@FR-P-Cap-View`, here of the subject's own
    /// place): a heap subject's capture skips its free and records the borrow of the subject's
    /// source.  Consumes `name:`; anything else is left for the variant parse.  A BARE lowercase
    /// name at the root stays what `@FR-M-Unit` makes it — a misspelled variant, refused — so a
    /// typo never turns into a catch-all (a-match-arm-names-a-variant-that-exists.loft).
    fn root_capture(
        &mut self,
        subject_val: &Value,
        subject_type: &Type,
        hoisted: &mut Vec<Value>,
    ) -> Option<u16> {
        let name = self.lexer.peek_named_arg()?;
        if !Self::is_binding_name(&name) {
            return None;
        }
        self.lexer.has_identifier();
        self.lexer.token(":");
        self.bind_root_capture(&name, subject_val, subject_type, hoisted)
    }

    /// Binds `name` to the matched subject itself — the binding half of [`Self::root_capture`].
    fn bind_root_capture(
        &mut self,
        name: &str,
        subject_val: &Value,
        subject_type: &Type,
        hoisted: &mut Vec<Value>,
    ) -> Option<u16> {
        let v = self.pattern_binding(name, subject_type);
        if v != u16::MAX {
            self.vars.defined(v);
            // A record subject is bound through the projection node a view is spelled as
            // (`s = o.inner`, `@FR-R-CopyView`'s `OpGetField(a, 0)`): a plain `Set` from a
            // record variable is a COPY at codegen (`@FR-B-Copy`), and the capture is a view.
            let bound = match subject_type.base() {
                Type::Reference(d, _) | Type::Enum(d, true, _) => {
                    let kt = i32::from(self.data.def(*d).known_type());
                    let get_field = self.data.def_nr("OpGetField");
                    Value::Call(
                        get_field,
                        vec![subject_val.clone(), Value::Int(0), Value::Int(kt)],
                    )
                }
                _ => subject_val.clone(),
            };
            hoisted.push(v_set(v, bound));
            if !matches!(subject_type.base(), Type::Text(_))
                && !crate::data::is_scalar(subject_type.base())
            {
                self.vars.set_skip_free(v);
                if let Some(src) = self.match_borrow_source(subject_val) {
                    let bound_tp = Self::element_view_of(&self.vars.tp(v).clone(), src);
                    self.vars.set_type(v, bound_tp);
                }
            }
        }
        (v != u16::MAX).then_some(v)
    }

    /// The `{ field, field: pattern, … }` of a plain-struct pattern: a bare field binds a new
    /// name (into `pattern_binds_pending`, restored at the arm's end), a `field: pattern` adds
    /// its sub-pattern's condition to `conds` and its names to the pending set.
    fn parse_struct_pattern_fields(
        &mut self,
        e_nr: u32,
        subject_val: &Value,
        binds: &mut Vec<Value>,
        conds: &mut Vec<Value>,
    ) {
        // @PLN35 L2 — a nested struct-enum field sub-pattern binds via
        // parse_match_enum_field_bindings, which uses the name-alias save list; a plain-struct
        // arm has no arm-level alias restore, so give it a local sink (the existing scalar /
        // plain-enum / wildcard sub-pattern branches never touch it → byte-identical).
        let mut name_aliases: Vec<(String, Option<u16>)> = Vec::new();
        if !self.lexer.has_token("{") {
            return;
        }
        while !self.lexer.peek_token("}") {
            if let Some(field_name) = self.lexer.has_identifier() {
                let attr_idx = self.data.attr(e_nr, &field_name);
                if attr_idx != usize::MAX {
                    self.check_visibility("pattern-field", e_nr, attr_idx);
                }
                if attr_idx != usize::MAX {
                    self.sandbox_read_member(e_nr, &field_name, subject_val);
                    let field_val = self.get_field(e_nr, attr_idx, subject_val.clone());
                    let field_type = self.data.attr_type(e_nr, attr_idx);
                    if self.lexer.has_token(":") {
                        // `@FR-P-Point` — a bare lowercase NAME is a binding under that name
                        // (`P { y: v }`), as the variant arm reads it; read as a value it
                        // compared against an outer `v` (loft#1885).
                        if let Some(bind_name) = self.field_pattern_rename() {
                            let v = self.pattern_binding(&bind_name, &field_type);
                            self.vars.defined(v);
                            let bound =
                                self.pattern_field_value(e_nr, attr_idx, subject_val.clone());
                            binds.push(v_set(v, bound));
                            self.mark_field_capture_view(v, &field_type, subject_val);
                        } else if let Some(cond) = self.parse_field_sub_pattern(
                            field_val,
                            &field_type,
                            binds,
                            conds,
                            &mut name_aliases,
                        ) {
                            conds.push(cond);
                        }
                        // The names a sub-pattern bound belong to this arm; its end
                        // restores them.
                        self.pattern_binds_pending.append(&mut name_aliases);
                    } else {
                        let v = self.pattern_binding(&field_name, &field_type);
                        self.vars.defined(v);
                        let bound = self.pattern_field_value(e_nr, attr_idx, subject_val.clone());
                        binds.push(v_set(v, bound));
                        self.mark_field_capture_view(v, &field_type, subject_val);
                    }
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "unknown field '{}' on struct {}",
                        field_name,
                        self.data.def(e_nr).name()
                    );
                }
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
    }

    /// Parse a plain-struct match arm (field bindings + body).
    /// Returns the arm — one per listed pattern (`@FR-P-Multi`: `P { x: 0, y: v } | P { x: v,
    /// y: 0 } => v`) — and whether it is exhaustive.  The first pattern's bindings are hoisted
    /// ahead of the chain as before; each further pattern is its own arm that runs its bindings,
    /// linked into the first one's slots, ahead of a clone of the body.
    fn parse_match_struct_arm(
        &mut self,
        e_nr: u32,
        subject_val: &Value,
        result_type: &mut Type,
        hoisted_bindings: &mut Vec<Value>,
    ) -> (Vec<EnumArm>, bool) {
        let mut field_conditions: Vec<Value> = Vec::new();
        let first_names = self.pattern_binds_pending.len();
        let mut first_binds: Vec<Value> = Vec::new();
        self.parse_struct_pattern_fields(
            e_nr,
            subject_val,
            &mut first_binds,
            &mut field_conditions,
        );
        let struct_name = self.data.def(e_nr).name().to_string();
        let alternatives = self.parse_pattern_alternatives(
            first_names,
            &mut first_binds,
            |p| {
                let at = p.lexer.link();
                let named = p.lexer.has_identifier().is_some_and(|id| id == struct_name);
                if !named {
                    p.lexer.revert(at);
                }
                named
            },
            |p, binds, conds| p.parse_struct_pattern_fields(e_nr, subject_val, binds, conds),
        );
        // @FR-P-Guard — `S { x } if x > 0 =>`: the guard reads the pattern's bindings and is
        // ANDed onto the pattern's own tests below, so a false guard fails the arm and
        // selection moves on, exactly as on an enum arm.
        let user_guard = self.parse_optional_guard();
        self.expect_match_arm_arrow();
        let mut arm_code = Value::Null;
        let arm_expected = self.match_arm_expected(result_type);
        let facts = self.assume_guard(user_guard.as_ref());
        let arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_code);
        self.forget_facts(facts);
        let block = v_block(vec![arm_code.clone()], arm_type.clone(), "struct_match");
        self.match_void_arm |= matches!(arm_type.base(), Type::Void);
        if Self::arm_settles_match(result_type, &arm_type) {
            *result_type = arm_type.clone();
        }
        let and_all = |conds: Vec<Value>| {
            conds
                .into_iter()
                .chain(user_guard.clone())
                .reduce(|a, b| v_if(a, b, Value::Boolean(false)))
        };
        let guard = and_all(field_conditions);
        // A pattern with no literal field and no guard always matches, so the arm covers the
        // struct (@FR-M-Total: a guarded arm covers nothing).
        let mut exhaustive = guard.is_none();
        if alternatives.is_empty() {
            // One pattern: its bindings are hoisted ahead of the chain, as they always were.
            hoisted_bindings.append(&mut first_binds);
            let arm = EnumArm {
                discs: vec![],
                code: block,
                tp: result_type.clone(),
                guard,
                bindings: Vec::new(),
                cond: None,
            };
            return (vec![arm], exhaustive);
        }
        // Several patterns write the same slots, so none is hoisted: each pattern's arm runs
        // its OWN bindings, then its literal-field test, then a clone of the body.
        let pattern_arm = |binds: Vec<Value>, guard: Option<Value>| {
            if let Some(g) = guard {
                // `cond: true` takes the chain's tested path, which runs `bindings` before the
                // guard.
                EnumArm {
                    discs: vec![],
                    code: v_block(vec![arm_code.clone()], arm_type.clone(), "struct_match"),
                    tp: result_type.clone(),
                    guard: Some(g),
                    bindings: binds,
                    cond: Some(Value::Boolean(true)),
                }
            } else {
                let mut stmts = binds;
                stmts.push(arm_code.clone());
                EnumArm {
                    discs: vec![],
                    code: v_block(stmts, arm_type.clone(), "struct_match"),
                    tp: result_type.clone(),
                    guard: None,
                    bindings: Vec::new(),
                    cond: None,
                }
            }
        };
        let mut arms = vec![pattern_arm(first_binds, guard)];
        for (binds_i, conds_i) in alternatives {
            if exhaustive {
                break; // an earlier pattern always matches: the rest are unreachable
            }
            let guard_i = and_all(conds_i);
            exhaustive = guard_i.is_none();
            arms.push(pattern_arm(binds_i, guard_i));
        }
        (arms, exhaustive)
    }

    /// The element reads a tuple VALUE read takes over a `&(…)` binding, with the element
    /// types, when `value` is such a binding — `None` for every other value.
    ///
    /// `@FR-T-Ref` / `@FR-B-Ref-Uniform` — a `&(…)` binding denotes the bound tuple itself, so
    /// every site that wants the tuple AS A VALUE asks here rather than testing the type's
    /// shape for `Type::Tuple`: a bare test answers no for the `&` spelling, and the two
    /// callers this has are the two the question has — a tuple PATTERN over the binding
    /// (loft#1530) and a DESTRUCTURE of it.  `@FR-T-Ref-Rep` gives
    /// the binding two representations and the reads follow each: a STACK-backed tuple (all
    /// scalar members) is projected through the reference with `TupleGet`, which the
    /// generators already read that way; a `__tuple<…>` RECORD is unboxed the way a record
    /// tuple return is, its element types being the record's own fields.
    pub(crate) fn ref_tuple_subject(
        &mut self,
        subject: &Value,
        tp: &Type,
    ) -> Option<(Value, Vec<Type>)> {
        let Value::Var(v) = subject.unspan() else {
            return None;
        };
        let v = *v;
        let pointee = match tp.base() {
            Type::RefVar(inner) => inner.base().clone(),
            _ => return None,
        };
        match pointee {
            Type::Tuple(elems) => {
                let reads = (0..elems.len())
                    .map(|i| Value::TupleGet(v, i as u16))
                    .collect();
                Some((Value::Tuple(reads), elems))
            }
            Type::Reference(d, _) if self.data.def(d).name().starts_with("__tuple<") => {
                let elems: Vec<Type> = self
                    .data
                    .def(d)
                    .attributes
                    .iter()
                    .map(|a| a.typedef.clone())
                    .collect();
                let reads = self.unbox_tuple_from_dbref(Value::Var(v), &elems);
                // A heap member read out of the record is a VIEW into the caller's store, so
                // its type says it borrows the binding.  The record's own field types carry
                // no deps, and read as owned the match's tuple released a struct-enum member
                // at scope exit — the caller's record, freed under it.
                let member_types = elems
                    .into_iter()
                    .map(|e| {
                        if crate::data::is_scalar(e.base()) {
                            e
                        } else {
                            e.depending(v)
                        }
                    })
                    .collect();
                Some((reads, member_types))
            }
            _ => None,
        }
    }

    /// #429: the frame var a match-arm field binding BORROWS from — the
    /// subject's backing variable.  A heap match binding (`CMap { entries }`)
    /// is a DbRef into the subject's record, so its type must carry a borrow
    /// dep on this var (see the call site).
    ///
    /// A bare `Var` subject is the common `match m { … }` / `match self.f { … }`-
    /// into-a-temp case.  A subject reached through GETTERS (`Wrap { inner: Holder
    /// { items } }` binds from `OpGetField(w, …)`, and so does `match w.inner`)
    /// still borrows from one variable — the root the chain starts at.  Reporting
    /// `None` there left the binding dep-free, which reads as "owns its store", so
    /// an append allocated a FRESH backing and repointed the local: the write
    /// vanished, silently, on both backends (loft#664's shape).  A chain rooted in
    /// a CALL has no backing variable and still yields `None`.
    pub(super) fn match_borrow_source(&self, subject_val: &Value) -> Option<u16> {
        match subject_val.unspan() {
            Value::Var(v) => Some(*v),
            // Only the GETTER family forwards a borrow: `OpGetField` / `OpGetVector` /
            // `OpGetEnum` read INTO their first argument's record, so the root of the
            // chain still owns the store.  A user call produces a value of its own, and
            // its first argument is just an argument.
            Value::Call(d_nr, args) if self.data.def(*d_nr).name().starts_with("OpGet") => {
                args.first().and_then(|a| self.match_borrow_source(a))
            }
            // A tuple MEMBER is the second spelling of a projection: `TupleGet(tmp, i)` carries
            // its base as a var number, not a `Var` node, so matching the getter family alone
            // left a binding taken through a tuple subject (`match (a, b) { (_, W { body }) =>
            // … }`) dep-free — its write reached `b`'s record while nothing said it borrowed
            // `b`, and a `&` parameter written only that way was refused as never modified
            // (loft#1526).  The member's own borrow is the source; a member the tuple OWNS
            // (a copied heap value) borrows the tuple temporary.
            Value::TupleGet(tmp, i) => {
                if let Type::Tuple(elems) = self.vars.tp(*tmp).base()
                    && let Some(member) = elems.get(*i as usize)
                    && let Some(&src) = member.depend().first()
                {
                    return Some(src);
                }
                Some(*tmp)
            }
            _ => None,
        }
    }

    /// #673 — record that `bind_nr` mirrors a struct-enum `text` payload, so a write
    /// through the binding can be written back into the subject.
    ///
    /// A heap payload binding holds a DbRef INTO the subject's record, so
    /// `items += …` reaches the subject with no help. A `text` payload binding is an
    /// owned copy of the characters (`_mv_items = OpGetText(subj, off)` — see the
    /// `skip_free` note at the call site), so the identical source line updated the
    /// copy and left the enum untouched, on both backends and with no diagnostic.
    /// Remembering the field read lets `parse_assign` mirror each write back with
    /// `OpSetText(subj, off, binding)`; a binding that is only read never reaches
    /// that path, so its bytecode is unchanged.
    ///
    /// Only a re-evaluable subject qualifies. The read is emitted once per write, so
    /// a subject carrying a user call (`match make_e() { … }`) would run that call
    /// again — and such a subject is a temporary the write could not outlive anyway.
    pub(super) fn record_text_payload_view(&mut self, bind_nr: u16, field_read: &Value) {
        let Value::Call(d_nr, args) = field_read.unspan() else {
            return;
        };
        if self.data.def(*d_nr).name() != "OpGetText"
            || args.len() != 2
            || self.ir_has_user_call(&args[0])
        {
            return;
        }
        let read = field_read.unspan().clone();
        self.text_payload_views
            .insert((self.context, bind_nr), read);
        // The scope pass needs the same fact after the parser's registry is gone (loft#1665).
        self.vars.text_payload_views.insert(bind_nr);
    }

    /// #673 / @PLN35 Phase 3 — point a multi-pattern branch's cloned body at the
    /// field offsets THIS branch bound its captures from.
    ///
    /// Every extra listed pattern runs a clone of the one arm body, so a `text`
    /// payload write-back (`record_text_payload_view`) reaches the clone carrying the
    /// FIRST pattern's `(subject, offset)`. `stmts_i` — this branch's
    /// `capture = OpGetText(subject, off_i)` binds — names the place the branch really
    /// read, so swapping the write to match keeps read and write on one field.
    /// Untouched when the two patterns put the field at the same offset.
    fn retarget_text_payload_writes(&self, body: &mut Value, stmts_i: &[Value]) {
        let get_nr = self.data.def_nr("OpGetText");
        let set_nr = self.data.def_nr("OpSetText");
        if get_nr == u32::MAX || set_nr == u32::MAX {
            return;
        }
        for st in stmts_i {
            let Value::Set(v_nr, rhs) = st.unspan() else {
                continue;
            };
            let Value::Call(d_nr, args) = rhs.unspan() else {
                continue;
            };
            let Some(Value::Call(_, first)) = self.text_payload_views.get(&(self.context, *v_nr))
            else {
                continue;
            };
            if *d_nr != get_nr || args.len() != 2 || first[..] == args[..] {
                continue;
            }
            let from = Value::Call(
                set_nr,
                vec![first[0].clone(), first[1].clone(), Value::Var(*v_nr)],
            );
            let to = Value::Call(
                set_nr,
                vec![args[0].clone(), args[1].clone(), Value::Var(*v_nr)],
            );
            crate::parser::expressions::substitute_value(body, &from, &to);
        }
    }

    /// `@FR-B-Scope` — the variable a PATTERN binds `name` to: a NEW binding per occurrence,
    /// as a `for` loop's variable is (loft#915).  An arm's names belong to the arm, so a later
    /// `match` that binds the same name — at another type, too — gets its own variable instead
    /// of the earlier arm's.  Bound by name (`add_variable`), `match a { [e, ..] => … }` then
    /// `match b { [e, ..] => … }` over a `vector<Pt>` and a `vector<text>` refused the second
    /// as a type change.  The occurrence's name comes from `loop_binding` (`e`, `e#1`, …),
    /// which both passes count alike and diagnostics print as `e`; `loop_variable` keys the
    /// variable by it, and the spelled name is pointed at it for the arm's body.
    pub(super) fn pattern_binding(&mut self, name: &str, tp: &Type) -> u16 {
        if self.context == u32::MAX {
            return u16::MAX;
        }
        let before = self.vars.var(name);
        // The variable's OWN name must differ from the spelling (`e#1`, never `e`): the arm's
        // end re-points the spelling, and a lookup by the variable's own name — the return
        // buffer a local becomes is a hidden attribute found by it — must still reach it.
        let mut id = self.vars.loop_binding(name);
        if id == name {
            id = self.vars.loop_binding(name);
        }
        let v = self.vars.loop_variable(&id, tp, &mut self.lexer);
        if v != u16::MAX {
            self.name_capture(name, v);
            self.pattern_binds_pending
                .push((name.to_string(), (before != u16::MAX).then_some(before)));
        }
        v
    }

    /// `@FR-B-Scope` — `spelling` names the capture `v` a PATTERN binds.  The binding is the
    /// pattern's, into the arm (or the `is` body) it guards, as a `for` header binds its
    /// variable into the loop's body, so it is recorded where the pattern stands: a write to
    /// the capture inside a block nested in the arm is a write, not that block's binding.
    /// Unrecorded, the first such write was recorded as the binding there and the read after
    /// the block was refused as out of scope (loft#2401).  Answers what the spelling named.
    pub(super) fn name_capture(&mut self, spelling: &str, v: u16) -> Option<u16> {
        let before = self.vars.set_name(spelling, v);
        self.record_binding(v);
        before
    }

    /// The end of a match arm: the names its pattern bound stop naming its bindings, and name
    /// what they named before the arm — a parameter, an outer local, or nothing.  Pairs with
    /// the seal in `expect_match_arm_arrow`; a nested `match` in the body seals and ends its own
    /// arms in between, so the frames nest as the arms do.
    fn end_pattern_arm(&mut self) {
        let Some(frame) = self.pattern_bind_frames.pop() else {
            return;
        };
        for (name, before) in frame.into_iter().rev() {
            match before {
                Some(v) => {
                    self.vars.set_name(&name, v);
                }
                None => self.vars.remove_name(&name),
            }
        }
    }

    /// The value a pattern BINDS for field `attr` of `d_nr` read out of `subject` — one home
    /// for every site that binds a field to a local: a struct-enum arm (`V { f } =>`), an
    /// `is` capture, a plain-struct arm, the alternation and multi-pattern slots.
    ///
    /// `@FR-L-Null-Which` — a field declared `S?` is STORED as the tagged `__nullable<S>`,
    /// and a local holds the pointer spelling, so the read goes through the tag here, as
    /// every other bind of such a slot does (`read_through_tag`).  Bound to the slot's
    /// address as if it were a dense `S`, each field read its predecessor (`p.x` answered
    /// the tag) and an absent payload tested present, on both backends.  Any other field
    /// reads as `get_field` has it.
    pub(super) fn pattern_field_value(&mut self, d_nr: u32, attr: usize, subject: Value) -> Value {
        let mut read = self.get_field(d_nr, attr, subject);
        let mut slot_tp = self.data.attr_type(d_nr, attr);
        self.read_through_tag(&mut read, &mut slot_tp);
        read
    }

    /// `elm_tp` re-stated as a VIEW into the frame variable `src` — the borrow dep a heap
    /// element read carries — or the type unchanged when the element holds no `DbRef` (a
    /// scalar, or a `text` element, which is an owned copy and must keep its own free).
    ///
    /// `@FR-O-Deps`: what a value borrows is read off its type, so a read that borrows and says
    /// nothing is read as OWNED.  This fact was spelled by hand at FIVE sites in this file, each
    /// a three-arm `match` over `Reference | Vector | Enum`, and three of them let an `Optional`
    /// fall past — so a NULLABLE element read was left dep-free and the free analysis emitted
    /// `OpFreeRef` for a record the subject owns: a use-after-free in the repetition
    /// materialisation on both backends, and an element handed back from `[a, ..] => a` that the
    /// caller could not see was a view (loft#1414).  The other two peeled correctly and were
    /// duplicates.  `Type::with_deps` writes through the wrapper, which is why the one home is a
    /// call to it rather than a sixth spelling.
    pub(super) fn element_view_of(elm_tp: &Type, src: u16) -> Type {
        if !matches!(
            elm_tp.base(),
            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, _, _)
        ) {
            return elm_tp.clone();
        }
        elm_tp.with_deps(&crate::data::Deps::frame1(src))
    }

    /// @PLN35 Phase 2 (`@FR-P-Cap-View`) — mark a slice-element capture that reads a HEAP
    /// element as a borrowed VIEW of the subject, the same way a struct-enum field
    /// binding is (`parse_match_enum_field_bindings`, #429). A slice binding
    /// `[first, ..] => first` (or `[tok:V, ..]`) reads `OpGetVector(subject, ..)` — a
    /// DbRef pointing INTO the subject's store, not an owned record. Untreated, it fails
    /// two ways: scope cleanup emits `OpFreeRef` for the binding at function exit, freeing
    /// a record the subject owns; and the binding carries empty deps, so when a value
    /// derived from it is returned, `ref_return` can't walk the borrow chain back to the
    /// subject parameter — the fn is mis-classified OWNED and the caller whole-store-frees
    /// the subject (a corrupted subject on BOTH backends when the capture escapes via
    /// return and the subject is reused afterwards).
    ///
    /// Scalars carry no DbRef and a `text` element is an owned copy (`OpGetText`), so
    /// neither needs this — only `Reference` / `Vector` / struct-enum element types do.
    /// `src` is the subject's source var (`Value::Var(v)` in `parse_vector_match`).
    pub(super) fn mark_slice_element_view(&mut self, bind_nr: u16, elm_tp: &Type, src: u16) {
        if bind_nr == u16::MAX
            || !matches!(
                elm_tp.base(),
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
        {
            return;
        }
        self.vars.set_skip_free(bind_nr);
        // A NULLABLE element is the same view: `(L-Null)` gives `E?` the layout of `E`, so a
        // binding off `vector<E?>` holds the subject's address exactly as its dense twin does
        // and borrows the same store.  Asked of the wrapper, the test above said "not a record"
        // and the binding carried EMPTY deps, so `ref_return` could not walk the borrow back to
        // the subject: a `match v { [a, ..] => a }` returning the element was classified OWNED,
        // the subject's store was freed at the callee's exit, and the caller read whatever was
        // allocated next — `101` where its dense twin answers `7`, on both backends (loft#1414).
        let bound_tp = Self::element_view_of(&self.vars.tp(bind_nr).clone(), src);
        self.vars.set_type(bind_nr, bound_tp);
    }

    /// `@FR-P-Cap-View` (loft#2353) — a plain-struct arm's field capture (`N { xs } =>`, and
    /// `N { xs: q }`, at the arm root or in a sub-pattern) names an interior place of the
    /// subject: a HEAP field is a borrowed view, so it skips its free and carries the borrow
    /// dep on the subject's source var — the marks `bind_match_field_capture` gives a
    /// struct-enum arm's capture.  Without them the capture's dep list was empty, which
    /// `(O-Proxy)` reads as an owner, and its `OpFreeRef` released the store the subject
    /// lives in: `match s.nodes[0] { N { xs } => len(xs) }` freed the caller's `s`.  A text
    /// field is an owned copy (`OpGetText`) and a scalar holds no store, so both keep their
    /// own treatment; a subject with no source variable still skips the free, as the enum
    /// arm's does — what it names belongs to the subject.
    fn mark_field_capture_view(&mut self, v: u16, field_type: &Type, subject_val: &Value) {
        if v == u16::MAX
            || !matches!(
                field_type.base(),
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
        {
            return;
        }
        self.vars.set_skip_free(v);
        if let Some(src) = self.match_borrow_source(subject_val) {
            let bound_tp = Self::element_view_of(&self.vars.tp(v).clone(), src);
            self.vars.set_type(v, bound_tp);
        }
    }
}
