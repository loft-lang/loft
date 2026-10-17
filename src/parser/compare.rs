// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Comparison and equality over compound values (`@FR-E-Eq`): tuples element by element,
//! value structs by content, struct-enums by their enum, and the user operator method an
//! operator site reaches when the author's own type supplies one.

use super::{
    DefType, Deps, Level, Parser, Type, Value, diagnostic_format, rename, v_block, v_if, v_set,
};

impl Parser {
    /// Compare two tuples, element by element — `==`, `!=`, `<`, `<=`, `>`, `>=`.
    ///
    /// Ordering is LEXICOGRAPHIC, the rule the written form already suggests: the first
    /// element decides, and later elements are consulted only while the earlier ones are
    /// equal, so `(1, 9) < (2, 0)`.  Equality is all elements equal.
    ///
    /// Lowered to the element types' OWN operators rather than to a tuple opcode, so every
    /// element type that can already be compared can be compared inside a tuple — text by
    /// value, a scalar enum by discriminant, a nested tuple by recursion into this same
    /// function — and both backends inherit it with nothing to add.  An element type with
    /// no such operator reports itself (`No matching operator '<' on 'Point' and 'Point'`),
    /// which names the element rather than the tuple around it.
    ///
    /// Returns `None` when this is not a tuple comparison, leaving `call_op`'s own
    /// resolution and diagnostics untouched.
    pub(super) fn tuple_compare(
        &mut self,
        code: &mut Value,
        op: &str,
        list: &[Value],
        types: &[Type],
    ) -> Option<Type> {
        if !matches!(op, "==" | "!=" | "<" | "<=" | ">" | ">=") || types.len() != 2 {
            return None;
        }
        let (left_elems, right_elems) = (
            self.tuple_elements(&types[0])?,
            self.tuple_elements(&types[1])?,
        );
        // Different arities are not a tuple comparison at all — fall through so the error
        // names both types, which says more than "arity 2 vs 3" would.
        if left_elems.len() != right_elems.len() || left_elems.is_empty() {
            return None;
        }
        if self.first_pass {
            return Some(Type::Boolean);
        }
        // Each element read names its side again, so an operand that DOES work — a call, an
        // index read — would be evaluated once per element.  Bind each to a local first; a
        // side that is already a tuple local is used as it stands.
        let mut prelude = Vec::new();
        let left =
            self.bind_tuple_operand(&list[0], &types[0], &left_elems, "__cmp_l", &mut prelude);
        let right =
            self.bind_tuple_operand(&list[1], &types[1], &right_elems, "__cmp_r", &mut prelude);
        let mut cmp = self.lex_compare(op, 0, left, right, &left_elems, &right_elems);
        // `@FR-T-Absent` — `==` compares tuples that EXIST.  The one tuple that does not is an
        // out-of-range read, and its representation is all-null members, which the element-wise
        // chain above compares EQUAL to another absent tuple: two separate misses answered
        // `true`.  Owner ruling 2026-09-16 (`tuples.md` D-tup-10): a side that is not there
        // makes `==` false and `!=` true, so the chain is consulted only once BOTH sides exist.
        //
        // Guarded per side, and only for a side whose TYPE can be absent — a tuple a program
        // wrote down always exists, so guarding it would emit a test that is constantly true.
        // Ordering (`<`, `<=`, `>`, `>=`) is deliberately untouched: the ruling is about the
        // null question, and lexicographic order over an absent operand is its own design call.
        if matches!(op, "==" | "!=") {
            // The operand's OWN type, not the one operator resolution handed us: a `τ?` operand
            // resolves against the `τ` operator, so `types` arrives PEELED here and an in-flight
            // tuple is indistinguishable from a written one by the time this runs (measured —
            // both sides read `(integer, text)` for two out-of-range reads).  `(T-Absent)` puts
            // the in-flight type in a LOCAL and nowhere else, so the variable's declared type is
            // where the `?` survives.  Guarding unconditionally instead would be wrong, not just
            // wasteful: `coalesce_not_null` reports an all-null WRITTEN tuple absent, and those
            // must still compare equal to each other.
            let operand_absent = |s: &Self, v: &Value, t: &Type| -> bool {
                let tp = match v.unspan() {
                    Value::Var(n) => s.vars.tp(*n).clone(),
                    _ => t.clone(),
                };
                Self::is_absent_tuple(&s.data, &tp)
            };
            let l_absent = operand_absent(self, &list[0], &types[0]);
            let r_absent = operand_absent(self, &list[1], &types[1]);
            if l_absent || r_absent {
                // The answer when a side is missing: `==` is false, `!=` is its negation.
                let missing = Value::Boolean(op == "!=");
                if r_absent {
                    let exists = self
                        .coalesce_not_null(&Value::Var(right), &Type::Tuple(right_elems.clone()));
                    cmp = v_if(exists, cmp, missing.clone());
                }
                if l_absent {
                    let exists =
                        self.coalesce_not_null(&Value::Var(left), &Type::Tuple(left_elems.clone()));
                    cmp = v_if(exists, cmp, missing);
                }
            }
        }
        *code = if prelude.is_empty() {
            cmp
        } else {
            prelude.push(cmp);
            v_block(prelude, Type::Boolean, "tuple_compare")
        };
        Some(Type::Boolean)
    }

    /// The element types of either spelling of a tuple — `None` for any other type.
    pub(crate) fn tuple_elements(&self, tp: &Type) -> Option<Vec<Type>> {
        match tp.base() {
            Type::Tuple(elems) => Some(elems.clone()),
            Type::Reference(d_nr, _) if self.data.def(*d_nr).name().starts_with("__tuple<") => {
                Some(self.stored_tuple_elements(tp.base()))
            }
            _ => None,
        }
    }

    /// Give a comparison operand a name its elements can be read from repeatedly, unboxing
    /// the STORED spelling on the way.  Appends the binding statement to `prelude`.
    pub(crate) fn bind_tuple_operand(
        &mut self,
        value: &Value,
        tp: &Type,
        elems: &[Type],
        name: &str,
        prelude: &mut Vec<Value>,
    ) -> u16 {
        if let Value::Var(v) = value.unspan()
            && matches!(self.vars.tp(*v), Type::Tuple(_))
        {
            return *v;
        }
        let tuple_tp = Type::Tuple(elems.to_vec());
        let mut val = value.clone();
        if self.unboxes_stored_tuple(tp.base(), &tuple_tp) {
            val = self.unbox_tuple_from_dbref(val, elems);
        }
        let tmp = self.create_unique(name, &tuple_tp);
        self.vars.defined(tmp);
        self.change_var_type(tmp, &tuple_tp);
        prelude.push(v_set(tmp, val));
        tmp
    }

    /// The lexicographic chain from element `i` onwards.
    ///
    /// The LAST element carries the operator as written, which is what separates `<` from
    /// `<=`: every earlier element only asks "strictly before?" and, failing that, "equal,
    /// so keep looking?".  `==` / `!=` have no chain — they are the same question asked of
    /// every element and folded with `&&` / `||`.
    pub(super) fn lex_compare(
        &mut self,
        op: &str,
        i: usize,
        left: u16,
        right: u16,
        left_elems: &[Type],
        right_elems: &[Type],
    ) -> Value {
        let last = left_elems.len() - 1;
        let elem = |s: &mut Self, o: &str, i: usize| -> Value {
            let mut c = Value::Null;
            s.call_op(
                &mut c,
                o,
                &[
                    Value::TupleGet(left, i as u16),
                    Value::TupleGet(right, i as u16),
                ],
                &[left_elems[i].clone(), right_elems[i].clone()],
            );
            c
        };
        if op == "==" || op == "!=" {
            let mut acc = elem(self, op, last);
            for i in (0..last).rev() {
                let head = elem(self, op, i);
                acc = if op == "==" {
                    v_if(head, acc, Value::Boolean(false))
                } else {
                    v_if(head, Value::Boolean(true), acc)
                };
            }
            return acc;
        }
        if i == last {
            return elem(self, op, i);
        }
        let strict = if op.starts_with('<') { "<" } else { ">" };
        let before = elem(self, strict, i);
        let same = elem(self, "==", i);
        let rest = self.lex_compare(op, i + 1, left, right, left_elems, right_elems);
        v_if(
            before,
            Value::Boolean(true),
            v_if(same, rest, Value::Boolean(false)),
        )
    }

    /// Try to find a matching defined operator. There can be multiple possible definitions for each operator.
    /// The `value struct` both operands of a comparison are, when they are the same one.
    pub(super) fn value_struct_operands(&self, types: &[Type]) -> Option<u32> {
        let [a, b] = types else {
            return None;
        };
        match (a.base(), b.base()) {
            (Type::Reference(d, _), Type::Reference(e, _))
                if d == e && self.data.is_value_struct(*d) =>
            {
                Some(*d)
            }
            _ => None,
        }
    }

    /// The schema row both operands of a CONTENT comparison are stored as (`@FR-E-Eq`, @C91) —
    /// the `tp` `OpEqContent` walks them by — when they are two values of one kind that `==`
    /// compares by content in the store: two vectors of one element type, two keyed
    /// collections of one element and key, two records of one struct, or two struct-enum
    /// values of one enum (a variant is compared as its enum, so two variants differ).
    pub(super) fn content_eq_type(&mut self, types: &[Type]) -> Option<u16> {
        let [a, b] = types else {
            return None;
        };
        match (a.peel_link(), b.peel_link()) {
            (Type::Vector(ea, _), Type::Vector(eb, _)) => {
                if !ea.is_equal(eb) || ea.is_unknown() {
                    return None;
                }
                let elem = (**ea).clone();
                Some(self.vector_of(&elem))
            }
            (
                l @ (Type::Hash(..)
                | Type::Sorted(..)
                | Type::Index(..)
                | Type::Radix(..)
                | Type::Trie(..)),
                r,
            ) if l.is_equal(r) => {
                let l = l.clone();
                self.keyed_field_kt(&l)
            }
            // A struct by its fields, through every `reference<T>` it holds.  A `value struct`
            // keeps its inline field-by-field comparison (`value_struct_eq`).
            (Type::Reference(da, _), Type::Reference(db, _))
                if da == db
                    && self.data.def_type(*da) == DefType::Struct
                    && !self.data.is_value_struct(*da) =>
            {
                let known = self.data.def(*da).known_type();
                (known != u16::MAX).then_some(known)
            }
            (l, r) => {
                let (ea, eb) = (self.struct_enum_of(l)?, self.struct_enum_of(r)?);
                let known = self.data.def(ea).known_type();
                (ea == eb && known != u16::MAX).then_some(known)
            }
        }
    }

    /// The enum a struct-enum value belongs to: the enum itself, or a variant's parent.
    /// The synthetic `__nullable<S>` is not one: it is the tagged spelling of a struct's
    /// absence, and a struct's `==` is the struct's.
    pub(super) fn struct_enum_of(&self, tp: &Type) -> Option<u32> {
        match tp.peel_link() {
            Type::Enum(d, true, _) if !self.data.def(*d).name.starts_with("__nullable<") => {
                Some(*d)
            }
            Type::Reference(v, _) if self.data.def_type(*v) == DefType::EnumValue => {
                let parent = self.data.def(*v).parent;
                (parent != u32::MAX && self.data.def_type(parent) == DefType::Enum)
                    .then_some(parent)
            }
            _ => None,
        }
    }

    /// `a == b` (or `a != b`) for two values of the `value struct` `d`, by CONTENT (`@FR-E-Eq`,
    /// @C91): every field compared with its own type's `==`, left to right, stopping at the
    /// first that differs — a float field by float equality, a text by its characters, a
    /// nested value struct by its own content.  A reference field still compares by identity:
    /// that is deviation D-op-16, closed when `==` on a struct becomes content.
    ///
    /// An operand that runs code is evaluated ONCE into a temporary; any other is read in
    /// place per field.  `None` when a field has no `==`, which leaves the comparison to the
    /// caller's ordinary resolution.
    pub(super) fn value_struct_eq(
        &mut self,
        d: u32,
        list: &[Value],
        negate: bool,
    ) -> Option<Value> {
        let mut stmts = Vec::new();
        let mut sides = Vec::new();
        for v in list {
            if self.ir_has_user_call(v) {
                let tmp = self.create_unique("__veq", &Type::Reference(d, Deps::none()));
                self.vars.defined(tmp);
                stmts.push(v_set(tmp, v.clone()));
                sides.push(Value::Var(tmp));
            } else {
                sides.push(v.clone());
            }
        }
        let mut acc: Option<Value> = None;
        for f in 0..self.data.def(d).attributes().len() {
            let ftp = self.data.attr_type(d, f);
            // A method is a member of its type, not a field (`api_surface` reads it the same
            // way): comparing it as one refused `==` on every value struct that has a method.
            if matches!(ftp.base(), Type::Routine(_)) {
                continue;
            }
            let fa = self.get_field(d, f, sides[0].clone());
            let fb = self.get_field(d, f, sides[1].clone());
            let mut cmp = Value::Null;
            let t = self.call_op(&mut cmp, "==", &[fa, fb], &[ftp.clone(), ftp]);
            if t.is_unknown() || matches!(t.base(), Type::Null) {
                return None;
            }
            acc = Some(match acc {
                None => cmp,
                Some(prev) => v_if(prev, cmp, Value::Boolean(false)),
            });
        }
        let mut res = acc.unwrap_or(Value::Boolean(true));
        if negate {
            res = self.cl("OpNot", &[res]);
        }
        Some(if stmts.is_empty() {
            res
        } else {
            stmts.push(res);
            v_block(stmts, Type::Boolean, "value_eq")
        })
    }

    /// `LOFT_TRACE_EQ_IDENTITY=1` — the census of @C91's flip (`@FR-E-Eq`): one line per
    /// `==` / `!=` this parse lowered to IDENTITY (`OpEqRef` / `OpNeRef`), naming the site, both
    /// operand types and the kind whose answer the content `==` changes.  Silent on a test
    /// against the `null` literal, which asks presence and keeps its answer.
    /// `scripts/eq_census.sh` collects it.
    pub(super) fn trace_eq_identity(&self, code: &Value, op: &str, types: &[Type]) {
        let Value::Call(nr, _) = code else {
            return;
        };
        if !matches!(self.data.def(*nr).name.as_str(), "OpEqRef" | "OpNeRef")
            || types.iter().any(|t| matches!(t, Type::Null))
        {
            return;
        }
        let Some(first) = types.first() else {
            return;
        };
        let kind = match first.peel_link() {
            Type::Reference(d, _) => match self.data.def_type(*d) {
                DefType::Enum | DefType::EnumValue => "struct-enum",
                _ if self.data.is_value_struct(*d) => "value-struct",
                _ => "struct",
            },
            Type::Vector(..) => "vector",
            Type::Hash(..)
            | Type::Sorted(..)
            | Type::Index(..)
            | Type::Radix(..)
            | Type::Trie(..) => "collection",
            _ => "other",
        };
        let pos = self.lexer.pos();
        let name = |t: Option<&Type>| t.map_or(String::new(), |t| self.data.display_type_name(t));
        eprintln!(
            "[eq-identity] {}:{}  {kind}  {} {op} {}",
            pos.file,
            pos.line,
            name(types.first()),
            name(types.get(1)),
        );
    }

    /// The user operator METHOD `op_name` a concrete operator site reaches for operands of
    /// `types` — `u32::MAX` when the first operand's type defines none, `None` when the
    /// overload set ties and the tie was reported.
    ///
    /// `@FR-F-Recv` — `a - 1` is a third spelling of the call `OpMin(a, 1)`, and the spellings
    /// of one call resolve identically.  The method KEY names only the slot's incumbent, the
    /// definition declared FIRST, so an operator overloaded on its second operand reached that
    /// one alone and refused the rest, while `OpMin(a, 1)` and `a.OpMin(1)` dispatched both
    /// (loft#1817).  Where the name carries an overload set the call spelling's own selection
    /// (`Disp-Select`) decides; the incumbent, re-asked at this call's arity (loft#1794),
    /// answers only a name with no set, or a set that no member of takes the operands — the
    /// mismatch the caller then reports as "No matching operator".
    pub(super) fn user_op_method(&mut self, op_name: &str, types: &[Type]) -> Option<u32> {
        // `@FR-Op-Std`, C132 — only the stdlib's `Op…` definitions back an operator; a
        // program's is an ordinary function, so the lookup answers "none" for it.
        let found = self.user_op_method_any(op_name, types);
        Some(match found {
            Some(d) if d != u32::MAX && !self.data.backs_operator(d) => u32::MAX,
            Some(d) => d,
            None => return None,
        })
    }

    /// [`Self::user_op_method`] without the stdlib filter.
    pub(super) fn user_op_method_any(&mut self, op_name: &str, types: &[Type]) -> Option<u32> {
        let first = types.first()?;
        let mut m = self.data.find_op_method(u16::MAX, op_name, first);
        if m == u32::MAX {
            return Some(m);
        }
        let set_source = self.data.receiver_overload_source(op_name, first);
        if !self.data.has_overload_set_in(set_source, op_name) {
            if Self::visible_arity(&self.data, m) != types.len() {
                m = self
                    .data
                    .possible_with_signature(op_name, types.len(), first)
                    .unwrap_or(u32::MAX);
            }
            return Some(m);
        }
        let routed = self.data.routed_types(types);
        match self.select_overload(set_source, op_name, &routed) {
            crate::parser::dispatch::Selection::One(d) => return Some(d),
            sel @ crate::parser::dispatch::Selection::Ambiguous(_) => {
                if !self.first_pass {
                    self.report_selection(op_name, &routed, &sel, None);
                }
                return None;
            }
            _ => {}
        }
        if Self::visible_arity(&self.data, m) != types.len() {
            m = self
                .data
                .possible_with_signature(op_name, types.len(), first)
                .unwrap_or(u32::MAX);
        }
        Some(m)
    }

    /// [`Self::call_op`] where the operator the author WROTE differs from the one being
    /// resolved (loft#1151).
    ///
    /// `handle_operator` derives the reversed spellings by swapping operands — `a > b` resolves
    /// `<`, `a >= b` resolves `<=` — so by the time a refusal is reached the original spelling
    /// is gone and the message named an operator the author never typed: `a >= b` on an
    /// unbounded `<T>` reported *"operator '<=' requires a concrete type"*.  `spelled` is
    /// carried for the DIAGNOSTIC only; every resolution decision still reads `op`, so the two
    /// cannot drift into disagreeing about what is being resolved.  The prefix operators pass
    /// the token too — `-x` resolves `Min`, `!x` `Not`, `~x` `BitNot` — so a refusal names
    /// `-` and not the internal `Min` (loft#1807).
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn call_op_as(
        &mut self,
        code: &mut Value,
        op: &str,
        spelled: &str,
        list: &[Value],
        types: &[Type],
    ) -> Type {
        // A first-pass UNARY op on an operand whose type is still UNRESOLVED — an
        // Unknown-rooted value, e.g. `x = f()` where `f`'s return type isn't linked
        // yet (a cross-package fn resolved only after this body's first pass) — must
        // stay re-typeable.  Otherwise the `possible` loop below matches a concrete
        // built-in (`-x` → OpMinInt → integer) and locks the result var to integer;
        // pass 2 then re-resolves `x` to its real (e.g. float) type and the assignment
        // errors "cannot change type from integer to float".  Return Unknown so pass 2
        // refines it cleanly — the same re-typeable escape the generic-type-variable
        // arm below takes on the first pass.  A truly-unresolvable operand re-errors on
        // pass 2 (this guard is first-pass only).  (@PLN102 transitive cross-package
        // inference.)
        //
        // ANY unresolved operand defers, not only an all-unresolved set.  A KNOWN operand
        // beside an unknown one does not steer resolution, it OVERRIDES it: in `f() - 1`
        // the loop matches `OpMinInt` off the literal and locks the local to integer, and
        // pass 2's real `float` return then errors at a line the reader never chose.  Even
        // a written-down `a: float = f() - 1` was refused this way, with the message
        // reversed.  Declaration order must not decide a type — the two-pass parser exists
        // precisely so a callee may be defined below its caller.
        //
        // This guard was once restricted to the all-unknown case to keep a genuine
        // mismatch ("No matching operator '<' on 'float' and 'boolean'") reportable, on
        // the theory that only a known operand could carry it to the error path.  loft#918
        // retired that reason: it added a SECOND deferral at the reject site, after the
        // operator search comes up empty, so pass 2 reports the mismatch there — naming
        // the type the operand really has rather than the `unknown` pass 1 saw.  Both
        // behaviours are guarded (`pln102_*` in tests/issues.rs); widen this predicate and
        // that pair is what tells you whether the diagnostic path still works.
        if self.first_pass && !types.is_empty() && types.iter().any(Type::is_unknown) {
            // Returning here leaves `code` as the bare LEFT operand — the operator is never
            // built — which is invisible to every later reading of the tree.  Pass 2 rebuilds
            // it, so this is only a collapse where the source is parsed once; record it so
            // the one caller that is (a parameter default) can tell.  See
            // `Parser::unresolved_types`.
            self.unresolved_types = self.unresolved_types.saturating_add(1);
            return Type::Unknown(0);
        }
        // Comparing two tuples is decided element by element, BEFORE the `possible` loop —
        // which would otherwise match `OpEqRef` for two STORED tuples and answer whether
        // they are the same record, not whether they hold the same values.
        if let Some(tp) = self.tuple_compare(code, op, list, types) {
            return tp;
        }
        // I8.1: if any operand is a generic type variable, skip the main operator loop
        // and go straight to the T-stub lookup.  The main loop would otherwise false-match
        // concrete operators (e.g. OpEqRef, OpEqBool) via implicit type conversions on T.
        let generic_name = types
            .iter()
            .find_map(|t| self.generic_type_name(t))
            .map(str::to_string);
        if let Some(tv_name) = generic_name {
            if self.first_pass {
                // Return the type variable type so assignments keep a consistent type
                // through the first pass (Type::Void would trigger "cannot change type").
                let tv_nr = self.data.def_nr(&tv_name);
                return if tv_nr == u32::MAX {
                    Type::Unknown(0)
                } else {
                    Type::Reference(tv_nr, Deps::none())
                };
            }
            let op_method = format!("Op{}", rename(op));
            // The stub is keyed per SIGNATURE, and `list.len()` is this call's arity — the
            // same count `has_bound_for_method` compares below.  `-` reaches here at BOTH
            // arities as `OpMin`, and picking the stub by name alone is what kept a bound from
            // offering binary subtraction beside unary negation (loft#1275).
            let stub_name = crate::data::Data::bound_stub_name(&tv_name, &op_method, list.len());
            let stub_nr = self.data.def_nr(&stub_name);
            // Only use the T-stub if the CURRENT function's bounds declare this method.
            // Without this check, T-stubs from unrelated bounded generics (e.g., stdlib's
            // sum<T: Addable>) would leak into unbound generics like `fn bad<T>(x+y)`.
            if stub_nr != u32::MAX
                && self.context != u32::MAX
                && self.has_bound_for_method(
                    &op_method,
                    self.data.def_nr(&tv_name),
                    Some(list.len()),
                )
            {
                let tp = self.call_nr(code, stub_nr, list, types, false, &[], None);
                if tp != Type::Null {
                    return tp;
                }
            }
            // loft#1144 — `!=` DERIVES from the bound's `==`.  `Equatable` declares `==`
            // alone, deliberately: an interface demanding every spelling would break every
            // user type that implements the minimum, and `default/01_code.loft` says the
            // bound is *"satisfied by … user types defining OpEq"*.  So the bound names
            // exactly the operator the derivation needs, and refusing `!=` left the
            // interface unable to serve the commonest use of itself — a guard that fires on
            // inequality — forcing every generic comparison to be written backwards.
            //
            // `!(a == b)` is EXACT, not an approximation, and that is what makes it
            // shippable where the non-strict comparisons are not: it agrees with the
            // concrete `OpNe*` on every value including IEEE `NaN` (`NaN == NaN` is false,
            // so `!(NaN == NaN)` is true, which is what `NaN != NaN` answers), and each
            // operand is emitted exactly ONCE, so an operand with side effects runs once.
            // `<=` / `>=` have neither property when derived from `<` alone — see loft#1151.
            //
            // The swap that already derives `>` from `<` lives one layer up, in
            // `operators.rs::handle_operator`; this is the same act for the equality pair,
            // placed HERE because the fallback is only correct once the direct stub lookup
            // above has come up empty — a bound that DOES declare `!=` keeps its own.
            if op == "!="
                && self.context != u32::MAX
                && self.has_bound_for_method("OpEq", self.data.def_nr(&tv_name), Some(2))
            {
                let eq_stub = crate::data::Data::bound_stub_name(&tv_name, "OpEq", 2);
                let eq_nr = self.data.def_nr(&eq_stub);
                if eq_nr != u32::MAX {
                    let mut eq_code = Value::Null;
                    let tp = self.call_nr(&mut eq_code, eq_nr, list, types, false, &[], None);
                    if tp != Type::Null {
                        *code = self.cl("OpNot", &[eq_code]);
                        return Type::Boolean;
                    }
                }
            }
            // loft#1151 — `<=` DERIVES from the bound's `<`, and `>=` comes with it because
            // `handle_operator` already rewrites `a >= b` to `b <= a`.  `Ordered` declares
            // `op <` alone — deliberately, so a user type satisfies it by defining one
            // method — and `>` has always derived from it by swapping operands; this is the
            // same act for the non-strict spelling.
            //
            // `!(b < a)` is exact for a TOTAL order, and every type `Ordered` names has one.
            // The case to worry about is a NaN, and loft does not have one here: its float
            // null IS the NaN and the type system keeps it in `float?`, so the NON-NULL
            // `float` a `<T: Ordered>` binds holds only real numbers.  Measured against the
            // concrete `<=` over float, single, integer and text INCLUDING their extremes —
            // 84 pairs, zero disagreements.  A user type's `OpLt` is promised to be an order
            // by the bound it satisfies; where it is not, no derivation can help.
            //
            // Each operand is emitted ONCE.  That is what makes this shippable where
            // `a < b || a == b` is not: the sound-looking alternative needs `Equatable` as a
            // second bound AND evaluates both operands twice, so `f(x) <= g(y)` would call
            // `f` and `g` twice while `f(x) < g(y)` beside it called them once.
            if op == "<="
                && self.context != u32::MAX
                && list.len() == 2
                && self.has_bound_for_method("OpLt", self.data.def_nr(&tv_name), Some(2))
            {
                let lt_stub = crate::data::Data::bound_stub_name(&tv_name, "OpLt", 2);
                let lt_nr = self.data.def_nr(&lt_stub);
                if lt_nr != u32::MAX {
                    // SWAPPED: `a <= b` is `!(b < a)`.
                    let swapped = [list[1].clone(), list[0].clone()];
                    let swapped_tp = [types[1].clone(), types[0].clone()];
                    let mut lt_code = Value::Null;
                    let tp =
                        self.call_nr(&mut lt_code, lt_nr, &swapped, &swapped_tp, false, &[], None);
                    if tp != Type::Null {
                        *code = self.cl("OpNot", &[lt_code]);
                        return Type::Boolean;
                    }
                }
            }
        } else {
            // @PLN99 Arc A completion — a first-grade struct's OWN operator method
            // (`t_<len><Type>_Op<Name>`) must take precedence over the built-in `possible`
            // loop below, which otherwise wins via (a) reference-identity for `==`/`!=` on
            // two struct refs (OpEqRef), or (b) coercing an operand through a user
            // `T → builtin` conversion and using the built-in operator (`a - b` → OpMinInt).
            // A built-in `integer` never coerces itself away; a user type must not either.
            // Method-only lookup (NOT full `find_fn`, whose `possible` fallback would
            // pre-empt the coercion the loop legitimately does for mixed built-in operands).
            if !types.is_empty() {
                let op_name = format!("Op{}", rename(op));
                let Some(m) = self.user_op_method(&op_name, types) else {
                    return if self.first_pass {
                        Type::Unknown(0)
                    } else {
                        Type::Never
                    };
                };
                // `@FR-F-Recv` — a TEMPLATE member (`fn OpAdd<U>(self: W, o: U)`) is reached the
                // way the call spelling `OpAdd(a, b)` reaches it: instantiated at the operand
                // types.  Handed to `call_nr` as the template, it matched nothing and `a + b`
                // was refused while `OpAdd(a, b)` beside it answered (loft#1826).
                let m = if m != u32::MAX && self.data.def_type(m) == DefType::Generic {
                    if self.first_pass {
                        let predicted = self.predict_template_return(m, &op_name, types);
                        if !predicted.is_unknown() {
                            *code = Value::Null;
                            return predicted;
                        }
                        u32::MAX
                    } else {
                        self.instantiate_template(m, &op_name, types)
                    }
                } else {
                    m
                };
                if m != u32::MAX {
                    let tp = self.call_nr(code, m, list, types, false, &[], None);
                    if tp != Type::Null {
                        return tp;
                    }
                }
            }
            // `@FR-E-Eq`, @C91 — vectors, keyed collections and struct-enum values compare by
            // CONTENT (`Stores::eq_content`): a vector by length and then element by element,
            // a keyed collection by its records, an enum value by its variant and then fields.
            if matches!(op, "==" | "!=")
                && list.len() == 2
                && let Some(tp) = self.content_eq_type(types)
            {
                *code = self.cl(
                    if op == "==" {
                        "OpEqContent"
                    } else {
                        "OpNeContent"
                    },
                    &[list[0].clone(), list[1].clone(), Value::Int(i32::from(tp))],
                );
                return Type::Boolean;
            }
            // `@FR-E-Eq`, loft#1580 — a `value struct` is a VALUE, and values compare
            // by content over their own storage.  Without a user `OpEq` the loop below matched
            // `OpEqRef` and compared identity, so a copy was unequal to its source.
            if matches!(op, "==" | "!=")
                && list.len() == 2
                && let Some(d) = self.value_struct_operands(types)
                && let Some(eq) = self.value_struct_eq(d, list, op == "!=")
            {
                *code = eq;
                return Type::Boolean;
            }
            // Two references to one struct type compare the records they name, absent ones
            // included (`OpEqRef` answers two absent as equal).  With either side optional no
            // `OpEqRef` candidate matched, and the loop below took `OpEqBool` over two
            // presence tests — so two present references to DIFFERENT records compared equal.
            // The presence test is right only against the `null` literal, whose type is
            // `Null`, not `Reference` (@C91).
            if (op == "==" || op == "!=")
                && list.len() == 2
                && let (Type::Reference(da, _), Type::Reference(db, _)) =
                    (types[0].base(), types[1].base())
                && da == db
                && (matches!(types[0], Type::Optional(_)) || matches!(types[1], Type::Optional(_)))
            {
                *code = self.cl(if op == "==" { "OpEqRef" } else { "OpNeRef" }, list);
                return Type::Boolean;
            }
            let mut possible = Vec::new();
            for pos in self
                .data
                .get_possible(&format!("Op{}", rename(op)), &self.lexer)
            {
                possible.push(*pos);
            }
            for pos in possible {
                // `OpEqBool` truthiness-fallback guard lives in `call_nr` (the single
                // chokepoint all three resolution sub-paths share) — see there.
                let tp = self.call_nr(code, pos, list, types, false, &[], None);
                if tp != Type::Null {
                    // We cannot compare two different types of enums, both will be integers in the same range
                    if let (Some(Type::Enum(f, _, _)), Some(Type::Enum(s, _, _))) =
                        (types.first(), types.get(1))
                        && f != s
                    {
                        break;
                    }
                    return tp;
                }
            }
            // @PLN99 Arc A — a user-defined operator on a concrete struct is stored
            // as `t_<len><Type>_Op<Name>` / `n_Op<Name>`, never in the `possible`
            // map (`add_op` fills it only for prefix-named built-ins like `OpLtInt`).
            // Resolve it via `find_fn` — the same resolver the generic/method path
            // uses — so a DIRECT `a < b` on a user struct dispatches the user def,
            // not only inside a `<T: Ordered>` body. A type with no such def falls
            // through to the unchanged "No matching operator" error below.
            if let Some(first) = types.first() {
                let user_op = self
                    .data
                    .find_fn(u16::MAX, &format!("Op{}", rename(op)), first);
                // Only at this call's arity (loft#1794): a type defining `-` at ONE arity
                // answered the other spelling with it — `a - b` over a unary-only `OpMin`
                // computed `-a` and dropped `b`, silently.  Refused below instead, as the
                // operator that it is not.
                if user_op != u32::MAX
                    && self.data.backs_operator(user_op)
                    && Self::visible_arity(&self.data, user_op) == list.len()
                {
                    let tp = self.call_nr(code, user_op, list, types, false, &[], None);
                    if tp != Type::Null {
                        return tp;
                    }
                }
            }
        }
        // @PLN25 E2 — `!nullable` (a `__nullable<S>` value in boolean position)
        // means "is absent" (discriminant 0): no struct operator overload
        // matches, so lower it directly (mirrors the `== null` is-null lowering).
        // The common shape is `!v[oob]` ("out-of-bounds is null").
        if op == "Not"
            && types.len() == 1
            && let Type::Enum(syn, true, _) = &types[0]
            && self.data.def(*syn).name.starts_with("__nullable<")
        {
            let get_enum = self.cl("OpGetEnum", &[list[0].clone(), Value::Int(0)]);
            let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
            *code = self.cl("OpEqInt", &[disc, Value::Int(0)]);
            return Type::Boolean;
        }
        // loft#918 — no operator matched, and one of the operands has no type YET:
        // `w_t + "!"` where `w_t` was bound to a call whose callee is declared lower in
        // the file.  On pass 1 that is a deferral, not a reject — pass 2 has the callee
        // and resolves the operand — and rejecting it here ended the parse before the
        // pass that would have accepted the program.
        //
        // The deferral at the top of this function is deliberately narrower (it fires
        // only when EVERY operand is unknown) because it also suppresses the `possible`
        // loop, which one known operand can still steer.  Here the loop has already run
        // and found nothing, so there is no resolution left to steer — only a message to
        // hold back.  A genuinely unresolvable operand reaches this same site on pass 2,
        // where the reject fires with the type it really has.
        // `has_unknown`, not `is_unknown`: a stub under a wrapper (`Optional(Unknown)` — a
        // forward alias behind a `?`) is just as undecidable, and reading it as settled emitted
        // *"No matching operator on 'unknown?'"* on pass 1 for a program pass 2 resolves.
        if self.first_pass && types.iter().any(Type::has_unknown) {
            return Type::Unknown(0);
        }
        // generic-specific error message for operators on T.
        let generic_name = types
            .iter()
            .find_map(|t| self.generic_type_name(t))
            .map(crate::data::Data::type_var_spelling);
        if let Some(tv_name) = generic_name {
            specific!(
                self.lexer,
                &self.lexer.peek().clone(),
                Level::Error,
                "generic type {tv_name}: operator '{spelled}' requires a concrete type",
            );
        } else if types.len() > 1 {
            let cure = self.operator_cure(spelled, types);
            specific!(
                self.lexer,
                &self.lexer.peek().clone(),
                Level::Error,
                "No matching operator '{spelled}' on '{}' and '{}'{cure}",
                types[0].source_name(&self.data),
                types[1].source_name(&self.data)
            );
        } else {
            let cure = self.operator_cure(spelled, types);
            specific!(
                self.lexer,
                &self.lexer.peek().clone(),
                Level::Error,
                "No matching operator '{spelled}' on '{}'{cure}",
                types[0].source_name(&self.data)
            );
        }
        Type::Unknown(0)
    }

    /// C132 (@PLN182) — what a program writes to give its own type the operator `spelled`:
    /// the `operator` method of a built form, or a named method for a form a type cannot
    /// define yet.  Empty when the left operand is not a program's type.
    pub(super) fn operator_cure(&self, spelled: &str, types: &[Type]) -> String {
        let Some(left) = types.first() else {
            return String::new();
        };
        let (Type::Reference(d, _) | Type::Enum(d, _, _)) = left.base() else {
            return String::new();
        };
        // A type variable is not a type a program declares an operator on: its cure is a bound.
        if self.data.def(*d).is_stdlib() || self.data.is_type_var_placeholder(*d) {
            return String::new();
        }
        let t = types[0].base().source_name(&self.data);
        let u = types
            .get(1)
            .map_or_else(|| t.clone(), |r| r.base().source_name(&self.data));
        match (spelled, types.len()) {
            ("<" | "<=" | ">" | ">=", 2) => {
                format!("; declare `operator compare(self: {t}, other: {u}) -> Ordering`")
            }
            ("+" | "+=", 2) => format!("; declare `operator plus(self: {t}, other: {u}) -> …`"),
            ("-" | "-=", 2) => format!("; declare `operator minus(self: {t}, other: {u}) -> …`"),
            ("*" | "*=", 2) => format!("; declare `operator times(self: {t}, other: {u}) -> …`"),
            ("/" | "/=", 2) => {
                format!("; declare `operator divided_by(self: {t}, other: {u}) -> …`")
            }
            ("%" | "%=", 2) => {
                format!("; declare `operator remainder(self: {t}, other: {u}) -> …`")
            }
            ("-", 1) => format!("; declare `operator negate(self: {t}) -> …`"),
            // The reserved forms (@PLN182 Q10): a type will define these, and cannot yet.
            ("**" | "&" | "|" | "^" | "<<" | ">>", 2) | ("~", 1) => {
                format!("; `{t}` cannot define '{spelled}' yet — call a named method")
            }
            _ => String::new(),
        }
    }
}
