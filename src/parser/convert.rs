// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Conversion between types at a use site (`@FR-T-Sub`): [`Parser::convert`] accepts a
//! value synthesised as one type where another is expected, with its store, condition and
//! admitting faces; [`Parser::cast`] is the explicit `as`; [`Parser::can_convert`] answers the
//! question without emitting anything.

use super::{
    ArmMismatch, Deps, Level, Parser, Position, StoreCtx, Type, Value, diagnostic_format, v_block,
    v_if, v_set,
};

impl Parser {
    /// Bring a CONDITION to `boolean` — the `if` / `while` position, where LOFT.md
    /// § Conversions promises the coercion for every type: *"`false` and null are falsy;
    /// an `integer` holding its null, `i64::MIN`, is falsy; every other value is truthy"*.
    ///
    /// Separate from [`convert`](Parser::convert) because the promise is about this
    /// POSITION, not about the pair of types: passing a `vector` where a `boolean`
    /// PARAMETER is declared stays the error it has always been, and that error catches
    /// real mistakes.  Only a condition asks to be read as present-or-absent.
    ///
    /// The heap types register no `OpConv*FromX → Boolean`, so without this the raw
    /// `DbRef` reached the condition.  `--native` emitted `rec != 0` from its own
    /// lowering and obeyed the rule; the interpreter's `GotoFalseWord` read
    /// `*get_stack::<u8>()` — the FIRST BYTE of a twelve-byte pointer.  So `if [1, 2]`
    /// took the ELSE arm on one backend and the THEN arm on the other, a live collection
    /// FIELD read as absent, and the eleven extra bytes on the evaluation stack ran the
    /// next argument off its slot and ended a run in a SIGSEGV.  The type list is the one
    /// [`coalesce_not_null`](Parser::coalesce_not_null) uses, for the same reason.
    pub(crate) fn convert_condition(&mut self, code: &mut Value, tp: &Type) -> bool {
        self.convert_condition_at(code, tp, None)
    }

    /// [`convert_condition`](Parser::convert_condition) anchored at the condition's own
    /// start, so the refusal below names the expression and not the `{` after it.
    ///
    /// **A store stays a statement (@C130).**  `if v[9] = 2 { … }` is the C `=`-for-`==`
    /// typo, and it is refused here rather than read as a flag: the expression answers
    /// NOTHING, so there is no value to test.  Nothing is wrong with the pair of types —
    /// the refusal is about the POSITION, as the heap-handle admission above it is.  Without
    /// it the interpreter read a stack byte as the flag, `if x = 5 { … }` then left `x`
    /// holding that byte's neighbours, and `--native` failed in rustc (`expected bool, found
    /// ()`): one program, two drivers disagreeing whether it is a program (measured on
    /// both `if` and `while`).  A `Never` condition (a `return` in it) is not
    /// this case and keeps its own path.
    pub(crate) fn convert_condition_at(
        &mut self,
        code: &mut Value,
        tp: &Type,
        at: Option<&Position>,
    ) -> bool {
        if matches!(tp.base(), Type::Void) {
            if !self.first_pass {
                let msg = "A condition needs a value, and this expression answers nothing \
                           — write `==` to compare, or move the store to its own line";
                if let Some(at) = at {
                    diagnostic_at!(self.lexer, at, Level::Error, "{msg}");
                } else {
                    diagnostic!(self.lexer, Level::Error, "{msg}");
                }
            }
            return false;
        }
        // `@FR-E-Truthy-1` — an UNTYPED `[]` is a vector VALUE like any other, so a truthiness
        // position reads it as present.  With no element type to size a store it parses to the
        // value-less placeholder (`parse_vector`), which reached the condition as nothing at
        // all: the interpreter's jump read a stack byte (and the next local took the damage)
        // and `--native` emitted `if ()` (loft#1919).  The literal has no effect to keep.
        if !self.first_pass && Self::is_untyped_empty_literal(code, tp) {
            *code = Value::Boolean(true);
            return true;
        }
        // `(Slice-Value)` — a vector slice is a FRESH vector, owed at every position a value
        // is read, and a condition reads one: it is materialised exactly as a bind would
        // (`iterator_as_vector`) and then tested like any vector.  Left an iterator, its
        // `Value::Iter` reached the scopes pass outside the loop its `break` belongs to — an
        // internal compiler error on both backends (loft#1922).
        if let Type::Iterator(elm, _) = tp.base()
            && Self::slice_shaped(code)
        {
            let vec_tp = Type::Vector(elm.clone(), Deps::none());
            if self.iterator_as_vector(code, tp, &vec_tp).is_some() {
                if !self.first_pass {
                    let not_null = self.coalesce_not_null(&code.clone(), &vec_tp);
                    *code = not_null;
                }
                return true;
            }
        }
        if Self::is_heap_handle(tp) {
            if !self.first_pass {
                let not_null = self.coalesce_not_null(&code.clone(), tp.base());
                *code = not_null;
            }
            return true;
        }
        self.convert_admitting(code, tp, &Type::Boolean)
    }

    /// The value-less placeholder an UNTYPED standalone `[]` parses to (`parse_vector`): an
    /// `Insert` of the incoming value, typed by an expected type that was itself unknown.
    pub(crate) fn is_untyped_empty_literal(code: &Value, tp: &Type) -> bool {
        tp.is_unknown()
            && matches!(code.unspan(), Value::Insert(v) if v.len() == 1 && matches!(v[0], Value::Null))
    }

    /// The STORE face of [`convert`](Parser::convert) — @FR-N-Store's home for every store
    /// that converts: `what` names the slot for the diagnostic ("the field", "parameter 2 of
    /// `f`", "the return value"), `at` anchors it (a block tail reports at the tail, not the
    /// `}`), and the rule's two halves are asked inside `convert` where the value meets the
    /// slot: a bare `null` at the entry, a `τ?` at the one arm that peels it.  Returns what
    /// `convert` returns; after a REFUSAL (a narrow width) it returns `true`, because the
    /// refusal has been reported and the caller's generic "cannot assign" would be a second
    /// diagnostic for one store.
    pub(crate) fn convert_store(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
        what: &str,
        at: Option<&Position>,
    ) -> bool {
        let ctx = StoreCtx {
            what: what.to_string(),
            at: at.copied(),
            never_error: false,
            dense: false,
        };
        self.convert_store_as(code, is_type, should, ctx)
    }

    /// [`convert_store`](Parser::convert_store) into a slot that can never be declared
    /// nullable — a `yield` into its `iterator<τ>` element — so the refusals name only the
    /// cures that compile there.
    pub(crate) fn convert_store_dense(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
        what: &str,
    ) -> bool {
        let ctx = StoreCtx {
            what: what.to_string(),
            at: None,
            never_error: false,
            dense: true,
        };
        self.convert_store_as(code, is_type, should, ctx)
    }

    /// [`convert_store`](Parser::convert_store) for a seam that warns at EVERY width — the
    /// collection-literal element (loft#1232) and every seam first covered by @PLN153 phase 3
    /// (a tuple literal's member, an index): where there was silence yesterday an ERROR
    /// today refuses a program that compiled, which the freeze forbids; a warning is the
    /// strict gain, and raising the tier is COMPATIBILITY.md's process.
    pub(crate) fn convert_store_lenient(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
        what: &str,
        at: Option<&Position>,
    ) -> bool {
        let ctx = StoreCtx {
            what: what.to_string(),
            at: at.copied(),
            never_error: true,
            dense: false,
        };
        self.convert_store_as(code, is_type, should, ctx)
    }

    pub(super) fn convert_store_as(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
        ctx: StoreCtx,
    ) -> bool {
        self.store_ctx.push(ctx);
        let accepted = self.convert(code, is_type, should);
        self.store_ctx.pop();
        accepted
    }

    /// The TEST face of [`convert`](Parser::convert): the caller reads a `τ?` as a `τ` (or a
    /// `null` as a value) without STORING it anywhere — a null test (`x != null`, `valid`), a
    /// condition, an overload trial, `&&`/`||` operands — so @FR-N-Store has nothing to say
    /// and is not asked.  Every admitting caller is one of those and is named at its site; a
    /// caller that should admit and does not gets a spurious warning, which the corpus shows.
    pub(crate) fn convert_admitting(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
    ) -> bool {
        self.admit_unwrap += 1;
        let accepted = self.convert(code, is_type, should);
        self.admit_unwrap -= 1;
        accepted
    }

    /// Is this binding's type a commitment the AUTHOR wrote — an explicit `: τ` annotation,
    /// or a parameter of the signature?  The declared / inferred split the storage rules
    /// turn on (`@FR-N-Decl` keeps a declared slot's type and asks `@FR-N-Store`;
    /// `@FR-N-Join` widens an inferred one), refined past the variable table's own
    /// [`Function::is_declared`](crate::variables::Function::is_declared): a local the
    /// compiler PROMOTED to a hidden out-parameter — the text return buffer `text_return`
    /// hoists, `__work_ret` — carries `argument` too, but its type is the compiler's, not
    /// the author's, so for every rule that reads this split it is still the inferred local
    /// the author wrote.  Read off the definition (the attribute is `hidden`) rather than
    /// declared per hoist site, so a new promotion is covered by existing.
    pub(crate) fn author_declared(&self, var_nr: u16) -> bool {
        if !self.vars.is_declared(var_nr) {
            return false;
        }
        if !self.vars.is_argument(var_nr) {
            return true;
        }
        let name = self.vars.name(var_nr);
        !self
            .data
            .def(self.context)
            .attributes()
            .iter()
            .any(|a| a.hidden && a.name == name)
    }

    /// The slot the current store names, for @FR-N-Store's wording: the top of the context
    /// stack, or the generic spelling when a bare `convert` stores (lenient, so a seam nobody
    /// has classified yet can only warn).
    pub(super) fn store_slot(&self) -> (String, Option<Position>, bool) {
        match self.store_ctx.last() {
            Some(c) => (c.what.clone(), c.at, c.never_error),
            None => ("a slot".to_string(), None, true),
        }
    }

    /// `(Slice-Value)` — an ITERATOR at a vector-typed position is the fresh vector a bind
    /// would make of it: an argument, a return, a block tail, a struct field's value and a
    /// literal's element take `v[a..b]` exactly as `s = v[a..b]` does, through the same
    /// materialisation, bound to a hidden local and read as a block whose tail is that
    /// local.  Answers the vector type when the pair is that shape — in the FIRST pass the
    /// type alone (a bind's materialisation waits for the second pass the same way), in the
    /// second the rewrite too — and `None` for anything else.  Only a vector slice has the
    /// shape (`Value::Iter` whose step is a block); a keyed range slice is refused at its own
    /// site (`(Slice-KeyedIter)`) and never reaches here.  ONE home: every site that meets an
    /// iterator where it wants a vector asks this, so the coercion cannot differ by site.
    pub(crate) fn iterator_as_vector(
        &mut self,
        code: &mut Value,
        is_type: &Type,
        should: &Type,
    ) -> Option<Type> {
        let (Type::Iterator(elm, _), Type::Vector(want, _)) = (is_type.base(), should.base())
        else {
            return None;
        };
        if !Self::slice_shaped(code) || !(elm.is_equal(want) || self.can_convert(elm, want)) {
            return None;
        }
        let vec_tp = Type::Vector(elm.clone(), Deps::none());
        if self.first_pass {
            return Some(vec_tp);
        }
        self.materialise_slice_leaves(code, elm, &vec_tp);
        Some(vec_tp)
    }

    /// Is every VALUE position of `code` a vector slice — the `Value::Iter` a range
    /// subscript builds (its step a block), reached through the arms of an `if` and the
    /// tail of a block?  The shape [`Parser::iterator_as_vector`] materialises.
    pub(crate) fn slice_shaped(code: &Value) -> bool {
        match code.unspan() {
            Value::Iter(_, _, n, _) => matches!(n.as_ref(), Value::Block(_)),
            Value::If(_, a, b) => Self::slice_shaped(a) && Self::slice_shaped(b),
            Value::Block(bl) => bl.operators.last().is_some_and(Self::slice_shaped),
            _ => false,
        }
    }

    /// Materialise each slice leaf of `code` (see [`Parser::slice_shaped`]) into a hidden
    /// local read in its place, and retype the blocks and `if`s on the way as the vector.
    pub(super) fn materialise_slice_leaves(&mut self, code: &mut Value, elm: &Type, vec_tp: &Type) {
        match code {
            Value::Iter(..) => {
                let tmp = self.create_unique("__iter_vec", vec_tp);
                self.vars.defined(tmp);
                let iter_tp = Type::Iterator(Box::new(elm.clone()), Box::new(Type::Null));
                let mut mat = code.clone();
                self.materialize_iterator(&mut mat, &iter_tp, &Value::Var(tmp), vec_tp, tmp, "=");
                *code = v_block(vec![mat, Value::Var(tmp)], vec_tp.clone(), "iter_vec");
            }
            Value::If(_, a, b) => {
                self.materialise_slice_leaves(a, elm, vec_tp);
                self.materialise_slice_leaves(b, elm, vec_tp);
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last_mut() {
                    self.materialise_slice_leaves(last, elm, vec_tp);
                }
                bl.result = vec_tp.clone();
            }
            other => {
                // Only a span wrapper carries a leaf; anything else is not slice-shaped.
                if matches!(other, Value::Span(..)) {
                    self.materialise_slice_leaves(other.unspan_mut(), elm, vec_tp);
                }
            }
        }
    }

    /// `@FR-T-Sub` — a value synthesised as `is_type` is accepted where `should` is expected
    /// when `is_type ⤳ should`, and `code` is rewritten to deliver it (a widening, a member-wise
    /// tuple rebuild, a variant into its enum).  `false` when no conversion is licensed; the
    /// caller owns the diagnostic.
    #[track_caller]
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(crate) fn convert(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        // @FR-N-Reserve (loft#1796) — a CONSTANT stored into a NULLABLE narrow slot must lie in
        // its usable range: one code at the edge is the slot's null, so `255` into a `u8?`, or
        // `300`, could only ever be stored as null.  Asked at the top because a literal that
        // fits the FULL width (`255`) needs no narrowing and takes no later branch; the
        // assignment seam asks the same predicate (`narrow_store_checks`).  An author's own
        // `e as u8?` never reaches a slot as a bare constant, so asking for null still works.
        if !self.first_pass
            && let Some(msg) = self.nullable_narrow_constant_refusal(code, should)
        {
            self.refuse_nullable_narrow_constant(&msg);
            // The null it would have become, so a later check of the same value (the
            // assignment seam's) meets a null and does not report it again.
            *code = self.cl("OpConvIntFromNull", &[]);
            return true;
        }
        // @PLN167 C3 (loft#1656) — a text field or element reaching a `&text` parameter HERE
        // comes through a FUNCTION VALUE: a direct call lowers it to the place before any
        // conversion (`process_call_args`).  It is lowered the same way: the argument is the
        // slot's `DbRef`, and the call's store mask (`Data::store_text_mask`) makes both
        // backends dispatch to the STORE instance of whichever function the value holds —
        // minted for every candidate of the value's type after pass 2 (`store_text.rs`).
        if matches!(should.base(), Type::RefVar(inner) if matches!(inner.base(), Type::Text(_)))
            && self.is_text_place(code)
            && let Some(place) = self.scalar_place_ref(code)
        {
            self.store_text_args = true;
            *code = place;
            return true;
        }
        // loft#1540 — a function value meets a function-typed slot only in the direction `const`
        // allows: a function whose parameter is plain may not stand where the slot promises that
        // parameter is `const` (`ConstParams::stands_for`), or a caller trusting the promise hands a
        // read-only value to a function that may write it.  Asked before every accept below,
        // because `is_equal` compares function types by shape and would take it.
        // Pass 2 only, as every refusal here: on pass 1 a function declared further down has not
        // yet said which of its parameters are `const`.
        if !self.first_pass
            && let (Type::Function(.., have), Type::Function(.., want)) =
                (is_type.base(), should.base())
            && !have.stands_for(*want)
        {
            return false;
        }
        // @PLAN48 P2: implicitly narrowing a loft `integer` to a smaller explicit
        // width (e.g. `integer` → `i32`) loses data and must be an explicit `as`.
        // A constant that provably fits is exempt.  Emit here, then fall through to
        // the `is_equal` accept — the Error fails compilation, and returning via
        // `is_equal` avoids a second (generic) diagnostic from the caller's
        // `validate_convert` (which still sees integer/i32 as can_convert-compatible).
        // `null` is not a narrowing store: there is no value to lose, and whether the
        // target may HOLD null is the separate (N-Store) nullability rule.  It reaches
        // here typed as the full `integer` — a nullable narrow field left out of a struct
        // literal takes its default this way — so the widened width test (loft#931) would
        // otherwise refuse `n.c = null` and `N { t: 1 }` on an `i32?` field.
        // @PLN152 — an author-written `??` names what happens when the value does not fit,
        // so a discharged store is not an unanswered narrowing.  The guard goes inside the
        // discharge and this seam then leaves the store alone: no refusal, and no second
        // guard around the outside.
        let discharged = !self.first_pass
            && !self.in_explicit_cast
            && self.range_guard_inside_discharge(code, should);
        let narrows = if self.in_explicit_cast {
            Self::is_narrowing_int(is_type, should)
        } else {
            Self::is_narrowing_int_store(is_type, should)
        } && !self.is_null_source(code);
        // A value that may not fit a narrow slot is refused where the author can choose (@C127).
        if !discharged && !self.first_pass && narrows && !self.int_value_fits(code, should) {
            let src = self.int_type_name(is_type);
            let dst = self.int_type_name(should);
            let dense = self.store_ctx.last().is_some_and(|c| c.dense);
            let cures = Self::narrowing_cures(code, should, &dst, dense);
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot implicitly narrow {src} to {dst} (may lose data) — {cures}"
            );
        }
        // loft#984 — a value meeting a slot that DECLARES a range is guarded here, which
        // is what reaches the shapes the assignment seam does not: a struct LITERAL's
        // field, a call ARGUMENT, a return.  An explicit `as` is excluded — a cast has its
        // own answer for a value that does not fit (`400 as u8` is null), and folding the
        // default in would silently change it.
        if !discharged && !self.first_pass && !self.in_explicit_cast && !self.is_null_source(code) {
            // A struct LITERAL's field, an ARGUMENT and a RETURN all name a DECLARED type,
            // so the wrapper is the answer here — the element-write ambiguity
            // `target_holds_null` exists for cannot arise at this seam.
            let holds_null = matches!(should, Type::Optional(_));
            self.guard_declared_range(code, should, is_type, holds_null);
        }
        // @FR-N-Store, the bare-`null` half, asked where every converting store passes.  An
        // explicit cast (`null as integer?`) is the author naming the type, not a store.
        if !self.first_pass
            && self.admit_unwrap == 0
            && !self.in_explicit_cast
            && matches!(is_type, Type::Null)
        {
            if crate::env_once!(std::env::var_os("LOFT_TRACE_UNWRAP").is_some()) {
                let (what, _, _) = self.store_slot();
                eprintln!(
                    "[null] -> {} what={} at {}",
                    should.name(&self.data), // schema-key — a developer trace behind `LOFT_TRACE_UNWRAP`, not a user diagnostic
                    what.replace(' ', "_"),
                    std::panic::Location::caller()
                );
            }
            let (what, at, lenient) = self.store_slot();
            if self.nstore_null_report(should, &what, at.as_ref(), lenient) {
                return true;
            }
        }
        // loft#1529 — an `E?` slot value meeting an `E?` store is the pointer on the far side
        // (`(L-Null-Which)`).  Asked before the equality accept, which is the path it takes:
        // the two types are the same and nothing else would convert it.  Only a STORE: a
        // comparison or a null test asks the slot itself.
        if !self.store_ctx.is_empty() && matches!(should, Type::Optional(_)) {
            self.read_through_enum_slot(code, is_type);
        }
        // A tuple's members are narrow slots of their own (`narrow_tuple_members`).
        if !self.first_pass
            && !self.in_explicit_cast
            && let (Type::Tuple(slots), Type::Tuple(values)) = (should.base(), is_type.base())
        {
            let (slots, values) = (slots.clone(), values.clone());
            self.narrow_tuple_members(code, &slots, &values);
        }
        if is_type.is_equal(should) {
            return true;
        }
        // Never (return/break/continue) is compatible with any type.
        if matches!(is_type, Type::Never) {
            return true;
        }
        // `(Slice-Value)` — an ITERATOR at a vector-typed position is the fresh vector a bind
        // would make of it ([`Parser::iterator_as_vector`]).
        if self.iterator_as_vector(code, is_type, should).is_some() {
            return true;
        }
        // `LOFT_TRACE_ITERVEC=1` — name an iterator that meets a vector slot and is NOT
        // materialised: the shape the arm above declined.
        if !self.first_pass
            && matches!(is_type.base(), Type::Iterator(_, _))
            && matches!(should.base(), Type::Vector(_, _))
            && crate::env_once!(std::env::var_os("LOFT_TRACE_ITERVEC").is_some())
        {
            let shown = format!("{:?}", code.unspan());
            eprintln!(
                "[itervec] declined: {} -> {} shape={}",
                is_type.source_name(&self.data),
                should.source_name(&self.data),
                &shown[..shown.len().min(160)]
            );
        }
        // @FR-L-Ref — a `reference<E>` over a STRUCT-enum is a record pointer, and a value of
        // that enum is a record (`Enum(E, true)`), so the value meets the slot as the pointer
        // to its own record — the relation a struct value already has with `reference<S>`,
        // where both sides are `Reference(S)`.  A PLAIN enum's value is a discriminant with no
        // record to point at, so it is not admitted.
        // A nullable side is left to the `?` arms below, which peel both and ask again.
        if !matches!(is_type, Type::Optional(_))
            && !matches!(should, Type::Optional(_))
            && let (Type::Enum(e, true, _), Type::Reference(r, _)) = (is_type.base(), should.base())
            && e == r
        {
            return true;
        }
        let _ = code;
        // Struct-literal inline constructors are typed as Rewritten(Reference(...)); strip
        // the wrapper so method calls chained on the constructor are accepted correctly.
        if let Type::Rewritten(inner) = is_type {
            return self.convert(code, inner, should);
        }
        if let Type::Rewritten(inner) = should {
            return self.convert(code, is_type, inner);
        }
        // @PLN25 slice (b): `Optional(τ)` is the nullable former. Behaviour-preserving until
        // DN1/DN3 give it teeth — peel and recurse on the base. A nullable TARGET also accepts
        // a bare `null`; a nullable SOURCE still implicitly unwraps to its base (DN2 removes
        // that unwrap later). Both arms converge: `Optional==Optional` is caught by `is_equal`
        // above, and a differing pair peels each side once.
        // @FR-N-Intro — `τ ⤳ τ?` is the ONE null-direction conversion the rules admit, and
        // this arm is it: a nullable TARGET accepts the base.  The arm below it (a nullable
        // SOURCE recursing on its base) is the implicit unwrap the rules say does not exist;
        // it stands because `convert` also services comparisons, and @FR-N-Store's teeth
        // refuse it at the store sites instead (the scattered shape @PLN153 phase 3 folds).
        if let Type::Optional(inner) = should {
            if matches!(is_type, Type::Null) {
                // null into a nullable target: run the base's null→typed-null coercion so
                // `code` becomes the base sentinel op (e.g. `OpConvIntFromNull`, not a bare
                // `null` that natively renders `()`), but a nullable target ALWAYS accepts
                // null regardless of the base's own nullability.  The recursion is the
                // base's null→sentinel coercion, not a second store: the OUTER target holds
                // null, so @FR-N-Store's bare-null ask at the entry must not see the peeled
                // base (`File { ref: null }` on an `i32?` field read as a narrow refusal).
                self.convert_admitting(code, is_type, inner);
                return true;
            }
            // Implicit CHECKED narrowing into a nullable narrow target — see
            // `implicit_checked_narrow`, which is the one home for it.
            if self.implicit_checked_narrow(code, is_type, should) {
                return true;
            }
            // `@FR-L-Null-Tag` — a tagged `__nullable<S>` flowing into a NULLABLE target has to
            // keep its absence.  The dense unwrap this would otherwise recurse into projects
            // the `Some` payload without consulting the discriminant, and a sub-ref into an
            // absent slot is a perfectly valid `DbRef` — so `null` arrived as a record of
            // zeroes at every function boundary, in the argument position and the return
            // position alike (loft#1138).  Read through the tag, which answers the reference
            // null sentinel: what `S?` means once it is out of the slot.
            //
            // Only the NULLABLE target takes this route.  A DENSE `S` target keeps the bare
            // payload sub-ref, because `(N-Store)` has already told it that it cannot hold
            // absence and because several sites downstream recognise that unwrap by its SHAPE —
            // `control.rs::tail_is_nullable_unwrap` (the #306 view-return materialise) and
            // `vectors.rs::new_record_field_op` both match `Value::Call(OpGetField, …)`, and an
            // `If` is not that.  Splitting on the target keeps one spelling per question.
            if !self.first_pass
                && let Type::Enum(syn, true, _) = is_type
                && let Type::Reference(struct_d, _) = inner.base()
                && self.data.def(*syn).name
                    == format!("__nullable<{}>", self.data.def(*struct_d).name())
            {
                let syn = *syn;
                let src = is_type.clone();
                *code = self.emit_nullable_slot_read(syn, code.clone(), &src);
                return true;
            }
            // Both sides nullable: `τ? ⤳ σ?` is `τ ⤳ σ` inside the wrapper.  A null the source
            // holds lands in a slot that holds null, so @FR-N-Intro's widening is the whole
            // conversion and there is no store for @FR-N-Store to ask about.  Peeling only the
            // target sent the source's unwrap to a DENSE `σ`, which reported a store the
            // program never makes (a `Fireball?` "stored into the non-null type `Entity`" on an
            // `Entity?` parameter, loft#1528).
            if let Type::Optional(src_inner) = is_type {
                return self.convert(code, src_inner, inner);
            }
            // A LINK to a nullable is the same source read through `(C-Ref)`: `&integer?` is
            // `RefVar(Optional(Integer))`, and peeling the target alone sent it to a dense
            // `integer`, reporting a store no program makes — formatting `"{e}"` with
            // `e = &z; z: integer?` warned that a nullable "becomes null there" (loft#1614).
            if let Type::RefVar(pointee) = is_type.base()
                && pointee.peel_optional().1
            {
                return self.convert(code, pointee, should);
            }
            return self.convert(code, is_type, inner);
        }
        if let Type::Optional(inner) = is_type {
            // @FR-N-Store — THE junction: every `τ? ⤳ τ` peel in the compiler passes this arm
            // (measured over the whole corpus, @PLN153 phase 3 — no peel happens anywhere
            // else), so the rule's τ? half is asked HERE and at no store site.  A caller that
            // TESTS nullability rather than storing (`x == null`, a condition, an overload
            // trial) admits the peel through `convert_admitting`; a store names its slot
            // through `convert_store`; a bare `convert` is asked with generic wording.  The
            // one-time census instrument stays: `LOFT_TRACE_UNWRAP=1` names every peel's
            // caller, face and slot.
            if !self.first_pass && !matches!(should, Type::Optional(_)) {
                if crate::env_once!(std::env::var_os("LOFT_TRACE_UNWRAP").is_some()) {
                    let (what, _, _) = self.store_slot();
                    eprintln!(
                        "[unwrap] {} -> {} admit={} what={} at {}",
                        is_type.name(&self.data), // schema-key — a developer trace behind `LOFT_TRACE_UNWRAP`
                        should.name(&self.data), // schema-key — a developer trace behind `LOFT_TRACE_UNWRAP`
                        self.admit_unwrap,
                        what.replace(' ', "_"),
                        std::panic::Location::caller()
                    );
                }
                // `@FR-T-Absent` — an in-flight `(τ₁, …, τₙ)?` landing in a slot whose members are
                // EACH nullable is the sanctioned store (owner ruling 2026-09-16, `tuples.md`
                // D-tup-10): the nulls land, and the slot then holds a tuple that exists.  Asked
                // here, the whole-type report named `(integer?, text?)` as "the non-null type" —
                // a type whose members are exactly what the value carries.
                //
                // The conversion itself was never wrong: this arm peels and recurses into the
                // element-wise tuple arm below, which converts each member.  Only the REPORT is,
                // so only the report is skipped — and only for this shape.  A destination with a
                // NON-null member keeps it, which is the half `(N-Store)` exists for: peeled, a
                // `(integer, text)` destination is identical to the source and reports nothing
                // at all, so dropping the report for every tuple pair would lose that warning.
                let sanctioned_tuple = self
                    .tuple_elements(inner)
                    .zip(self.tuple_elements(should))
                    .is_some_and(|(v, t)| {
                        v.len() == t.len() && t.iter().all(|m| matches!(m, Type::Optional(_)))
                    });
                if self.admit_unwrap == 0 && !self.in_explicit_cast && !sanctioned_tuple {
                    let (what, at, lenient) = self.store_slot();
                    if self.nstore_unwrap_report(inner, should, &what, at.as_ref(), lenient) {
                        return true;
                    }
                }
            }
            return self.convert(code, inner, should);
        }
        // Plan-06 phase 4d: tuple-to-tuple convert is element-wise.
        // Without this, a value with a `Rewritten(Reference)` element
        // (e.g. `(Inner { … }, 11)` from inline struct construction)
        // fails to match a field declared as `(Inner, integer)` even
        // though the underlying types are compatible.
        //
        // When `code` IS the tuple literal, each element is converted IN PLACE —
        // a shape-only check accepts the tuple and leaves every element
        // un-coerced, which for a bare `null` element means no op is emitted at
        // all and the slot keeps whatever the stack held (loft#808:
        // `fn f() -> (integer?, integer) { (null, 2) }` read back 8051).  The
        // element needs the same `null` → `OpConvIntFromNull` coercion the
        // synthetic-struct path gets from its per-field assignment.  Elements
        // are converted through a temporary so a failing element leaves `code`
        // untouched for the caller's fallback arms.
        // `@FR-T-Absent` — an in-flight `(τ₁, …, τₙ)?` reaching a member-nullable slot is the
        // SANCTIONED store: the nulls land, and the slot then holds a tuple that exists (owner
        // ruling 2026-09-16, `tuples.md` D-tup-10).  Read BARE, the `Optional` wrapper made this
        // arm decline, the value fell through to the `τ? ⤳ τ` report one level down, and a field
        // declared `(integer?, text?)` was told that a nullable `(integer, text)?` "is stored
        // into the field of the non-null type `(integer?, text?)`" — naming a type whose members
        // are exactly what the value carries.  `n_store_violation`'s own walk already peels this
        // way; one question with two decoders is what let them disagree.
        let tuple_convert_elems = |tp: &Type| -> Option<Vec<Type>> {
            let (base, absent) = tp.peel_optional();
            let Type::Tuple(elems) = base else {
                return None;
            };
            Some(if absent {
                elems.iter().cloned().map(Type::optional).collect()
            } else {
                elems.clone()
            })
        };
        if let (Some(src_elems), Some(dst_elems)) =
            (tuple_convert_elems(is_type), tuple_convert_elems(should))
            && src_elems.len() == dst_elems.len()
        {
            let mut items = match code.unspan_mut() {
                Value::Tuple(elements) if elements.len() == src_elems.len() => {
                    std::mem::take(elements)
                }
                _ => Vec::new(),
            };
            let mut all_compatible = true;
            // @FR-N-Store — a tuple stores ELEMENT-WISE, so each member is its own slot and
            // the diagnostic names which (`element 0 of the field`); the six tuple-literal
            // cells loft#1366 lists were silent because nothing named them.
            // With no enclosing store named, a member is still a SLOT (`(N-Reserve)`, and
            // `(L-Tuple)` lays it out as a field), so a narrow one keeps `(N-Store)`'s hard
            // error: `never_error` defaulted to true here, and `t: (integer, u8) = (1, x as
            // u8?)` warned and stored null into the non-null `u8` (loft#1815).  Only a narrow
            // integer member is affected — a heap member never escalates either way.
            let (what, at, lenient, dense) = match self.store_ctx.last() {
                Some(c) => (c.what.clone(), c.at, c.never_error, c.dense),
                None => ("this tuple".to_string(), None, false, false),
            };
            for (i, (s, d)) in src_elems.iter().zip(dst_elems.iter()).enumerate() {
                let mut placeholder = Value::Null;
                let elem = items.get_mut(i).unwrap_or(&mut placeholder);
                self.store_ctx.push(StoreCtx {
                    what: format!("element {i} of {what}"),
                    at,
                    never_error: lenient,
                    dense,
                });
                let ok = self.convert(elem, s, d);
                self.store_ctx.pop();
                if !ok {
                    all_compatible = false;
                    break;
                }
            }
            let literal = !items.is_empty();
            if literal && let Value::Tuple(elements) = code.unspan_mut() {
                *elements = items;
            }
            // `@FR-C-Tuple` over `@FR-C-Num` — a member whose conversion changes the
            // REPRESENTATION (`integer` → `float`) needs that conversion applied, and a tuple
            // that is not a literal has no member expression to apply it to: the loop above
            // converted a placeholder, and the tuple was copied bit for bit — `(1, 2)` held in a
            // variable read back `(1, 1e-323)` as `(integer, float)` on the interpreter and did
            // not compile natively.  Such a tuple is rebuilt from its members, each converted.
            if all_compatible
                && !literal
                && !self.first_pass
                && !matches!(is_type, Type::Optional(_))
                && !matches!(should, Type::Optional(_))
                && src_elems
                    .iter()
                    .zip(dst_elems.iter())
                    .any(|(s, d)| self.changes_representation(s, d))
            {
                let tmp = self.create_unique("_tconv", is_type);
                let mut members = Vec::with_capacity(src_elems.len());
                for (i, (s, d)) in src_elems.iter().zip(dst_elems.iter()).enumerate() {
                    let mut member = Value::TupleGet(tmp, i as u16);
                    self.convert(&mut member, s, d);
                    // The member is READ out of the temporary, so its type names it — which is
                    // what makes a heap member take the copy a hand-written `(t.0, …)` gets.
                    self.tuple_member_owned_copy(&mut member, &d.with_deps(&Deps::frame1(tmp)));
                    members.push(member);
                }
                *code = v_block(
                    vec![v_set(tmp, code.clone()), Value::Tuple(members)],
                    should.clone(),
                    "tuple_convert",
                );
            }
            if all_compatible {
                return true;
            }
        }
        // loft#822 — the STORED spelling of a tuple reaching a STACK-tuple slot.
        //
        // A tuple is one loft type written two ways: `Type::Tuple(…)` (its elements on
        // contiguous stack slots) and `Reference(__tuple<…>)` (a DbRef at the element's
        // inline bytes).  The stored spelling is what a `vector<(…)>` loop variable
        // carries (`for_type`, P189b) and what a HEAP-carrying tuple return carries
        // (`tuple_return_rewrite`), so it arrives wherever such a value is used as the
        // plain type: a call argument, a `return`, a declared local, a vector append.
        // Every one of those answered "expected (float, float, float), got
        // __tuple<float,float,float>" — a type error between a type and itself.
        //
        // Accepting the pair as EQUAL would be worse than the error: the slot would take
        // 12 bytes of DbRef where the reader expects the elements.  So the conversion is
        // a real one — `unbox_tuple_from_dbref` reads each element at its stored offset
        // and rebuilds the stack tuple, exactly as `v[i]` already does.
        // …and a stored tuple whose member changes REPRESENTATION on the way (`@FR-C-Num`) is
        // unboxed at its OWN members — the destination's would name another `__tuple` def
        // with other offsets — and then converted member by member like a stack tuple.
        if let (Type::Reference(d, _), Type::Tuple(_)) = (is_type, should)
            && self.data.def(*d).name().starts_with("__tuple<")
            && !self.unboxes_stored_tuple(is_type, should)
            && self.changes_representation(is_type, should)
        {
            let src = Type::Tuple(self.stored_tuple_elements(is_type));
            if !self.first_pass && !matches!(code.unspan(), Value::Null) {
                let elems = self.stored_tuple_elements(is_type);
                *code = self.unbox_tuple_from_dbref(code.clone(), &elems);
            }
            return self.convert(code, &src, should);
        }
        if self.unboxes_stored_tuple(is_type, should) {
            // A `Value::Null` here is the shape-only probe the Tuple→Tuple arm above
            // runs for a non-literal source; there is no expression to unbox.
            if !self.first_pass && !matches!(code.unspan(), Value::Null) {
                // The DESTINATION spellings, not the stored ones.  `unbox_tuple_from_dbref`
                // re-derives the `__tuple<…>` def from what it is handed, and the def is
                // NAMED by the source-level spelling — so handing it a tagged member yields
                // `__tuple<__nullable<S>,integer>`, a different def with different offsets,
                // and every member after it reads at the wrong one.  Passing the destination
                // types re-derives the SAME def the value is stored in, and each member's
                // declared type is what tells the unboxer to read a tagged slot through its
                // tag.  Offsets and types then come from one def, which is the same rule the
                // write side answers (loft#1134).
                let dst_elems = match should {
                    Type::Tuple(elems) => elems.clone(),
                    _ => self.stored_tuple_elements(is_type),
                };
                *code = self.unbox_tuple_from_dbref(code.clone(), &dst_elems);
            }
            return true;
        }
        // loft#1065 — `should.base()`, so a variant literal lands in a NULLABLE struct-enum
        // as readily as in a bare one.  `s: Shape? = Shape::Circle { r: 7 }` was refused
        // ("cannot change type from Shape? to Circle") while `s: Shape = …` beside it was
        // accepted, and a no-payload `Shape::Dot` was accepted either way — so the `?` was
        // the axis and only for the variants that carry a record.  Whether the slot may be
        // absent says nothing about which variants it can hold.
        if let (Type::Reference(ref_tp, _), Type::Enum(enum_tp, true, _)) = (is_type, should.base())
        {
            for a in self.data.def(*enum_tp).attributes() {
                if a.name == self.data.def(*ref_tp).name() {
                    return true;
                }
            }
            // @PLN25 — a nullable struct SOURCE (`Reference(S)`, possibly the null
            // sentinel) flows into a synthetic `__nullable<S>` field.  Accept it here;
            // `handle_field` emits the wrap (null → discriminant 0, present → `Some`).
            if self.data.def(*enum_tp).name
                == format!("__nullable<{}>", self.data.def(*ref_tp).name())
            {
                return true;
            }
        }
        // @PLN25 single-payload — the REVERSE coercion: a `__nullable<S>` value flows
        // into a dense `S` slot (`f(v[i])` where `fn f(r: S)`, a `??` result, a
        // dense-local assign, a return).  The dense `S` IS the `Some` variant's inline
        // `payload` field, so unwrap by SUB-REFERENCING the payload — the sub-ref is a
        // valid dense `S` reference (it shares S's offset table), with NO copy.
        // (NOTE: the by-REF `&S` arg form is NOT handled here — a sub-ref reaches the
        // arg path as a by-VALUE `Reference` and gets copied, so a `&mut` write would not
        // propagate.  That seam is handled at the call-arg site, not in `convert`.)
        if let (Type::Enum(enum_d, true, _), Type::Reference(struct_d, _)) = (is_type, should)
            && self.data.def(*enum_d).name
                == format!("__nullable<{}>", self.data.def(*struct_d).name())
        {
            if !self.first_pass {
                let enum_d = *enum_d;
                let struct_d = *struct_d;
                let some_d = self.data.variant_of(enum_d, "Some");
                if some_d != u32::MAX {
                    // `get_val` for an INLINE struct field produces a sub-ref (pos+offset),
                    // not a pointer deref — the same form the field-access unwrap uses
                    // (fields.rs).  A raw `OpGetField` with the struct type would deref the
                    // payload bytes as a DbRef → garbage.  The payload IS dense `S`.
                    let payload_pos = u32::from(
                        self.database
                            .position(self.data.def(some_d).known_type(), "payload"),
                    );
                    *code = self.get_val(
                        &Type::Reference(struct_d, crate::data::Deps::none()),
                        false,
                        payload_pos,
                        code.clone(),
                        u32::MAX,
                    );
                }
            }
            return true;
        }
        // @PLN25 E2 — a synth `__nullable<S>` value used as a BOOLEAN (a condition
        // or bool arg, e.g. `if hash[k]` / `assert(hash[k])`) coerces to its
        // truthiness = "is present" = discriminant != 0.  Read the disc directly
        // (`OpGetEnum` @ offset 0, which has the rec==0 null-record guard, so an
        // ABSENT lookup result — rec 0 — reads disc 0 = not present), mirroring the
        // inline branch of `null_check_builder`.  Without this the raw nullable
        // DbRef reaches the bool context and native emits `(DbRef) as u8` (E0605);
        // a dense `Reference` lookup gets `OpConvBoolFromRef` on the same path.
        if let (Type::Enum(enum_d, true, _), Type::Boolean) = (is_type, should)
            && self.data.def(*enum_d).name.starts_with("__nullable<")
        {
            if !self.first_pass {
                let get_enum = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
                let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
                let is_null = self.cl("OpEqInt", &[disc, Value::Int(0)]);
                *code = self.cl("OpNot", &[is_null]);
            }
            return true;
        }
        if let Type::RefVar(ref_tp) = is_type
            && self.convert(code, ref_tp, should)
        {
            return true;
        }
        // @PLN25 single-payload — a `__nullable<S>` value flows into a `&S` (`RefVar(Reference
        // (S))`) parameter (`fn f(r: &S)` that may MUTATE through `r`).  Unwrap to the payload
        // sub-ref, then pass it BY-REFERENCE via a work-ref + `OpCreateStack` (with `skip_free`,
        // since the work-ref holds only a borrowed DbRef into the element, not its own store) —
        // exactly the complex-expression by-ref path below.  A `&mut` write through the sub-ref
        // then propagates straight back to the source element's payload.  This must run BEFORE
        // the `ref_tp.is_equal(is_type)` arm (which fails: `Reference(S)` != `Enum(__nullable<S>)`).
        if let Type::RefVar(ref_tp) = should
            && let Type::Reference(struct_d, _) = &**ref_tp
            && let Type::Enum(enum_d, true, _) = is_type
            && self.data.def(*enum_d).name
                == format!("__nullable<{}>", self.data.def(*struct_d).name())
        {
            if !self.first_pass {
                let struct_d = *struct_d;
                let enum_d = *enum_d;
                let some_d = self.data.variant_of(enum_d, "Some");
                if some_d != u32::MAX {
                    let payload_pos = u32::from(
                        self.database
                            .position(self.data.def(some_d).known_type(), "payload"),
                    );
                    let dense = Type::Reference(struct_d, Deps::none());
                    let sub = self.get_val(&dense, false, payload_pos, code.clone(), u32::MAX);
                    let wv = self.vars.work_refs(&dense, &mut self.lexer);
                    self.vars.set_skip_free(wv);
                    *code = Value::Insert(vec![
                        v_set(wv, sub),
                        self.cl("OpCreateStack", &[Value::Var(wv)]),
                    ]);
                }
            }
            return true;
        }
        // tuples.md T-Ref — a tuple LOCAL passed to a `&(…)` parameter whose elements are
        // not all scalars.  The parameter is a reference to the `__tuple<…>` record, and the
        // local is a stack tuple on pass 1: record that it must be BUILT as that record, and
        // let pass 1 through.  On pass 2 the bind has already made it one, so a stack tuple
        // still arriving here is not a local this function declares (a by-value parameter,
        // a projection), and a link cannot name it — B-Ref-Reshape's rule: refuse rather
        // than downgrade to a copy.
        if let Type::RefVar(ref_tp) = should
            && let Type::Reference(d, _) = &**ref_tp
            && self.data.def(*d).name().starts_with("__tuple<")
            && let Type::Tuple(elems) = is_type
            && crate::data::ref_tuple_is_record(elems)
        {
            // `@FR-T-Ref-Src` — a vector element or a struct field of tuple type, read with
            // its address: the parameter names that record, through a borrowed work-ref as
            // any `&S` argument that is not a variable is passed (loft#1883).
            if !self.first_pass
                && let Some(dbref) = Self::stored_tuple_dest(code)
                && dbref.is_place_read(&self.data)
            {
                // Typed as the place's VIEW (it borrows the container), so neither backend
                // copies the element into a store of its own.  Pass 2 only, so it draws from
                // the pass-2 sequence (loft#848).
                let mut rec = (**ref_tp).clone();
                if let Some(root) = dbref.base_var() {
                    rec = rec.depending(root);
                }
                let wv = self.vars.work_refs_p2(&rec, &mut self.lexer);
                self.vars.set_skip_free(wv);
                *code = Value::Insert(vec![
                    v_set(wv, dbref),
                    self.cl("OpCreateStack", &[Value::Var(wv)]),
                ]);
                return true;
            }
            if let Value::Var(v) = code.unspan() {
                self.ref_linked_tuple_locals
                    .insert((self.context, self.vars.name(*v).to_string()));
            }
            if !self.first_pass {
                let what = match code.unspan() {
                    Value::Var(v) if self.vars.is_argument(*v) => "a by-value parameter",
                    Value::Var(_) => "a variable this function did not declare",
                    _ => "not a tuple variable",
                };
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a `&(…)` reference tuple with a heap element links to a tuple LOCAL \
                     of the calling function, and this argument is {what} — bind it to a \
                     local first and pass that"
                );
            }
            return true;
        }
        // loft#1372 — matched through the SLOT.  A `&τ?` parameter's referent is
        // `Optional(τ)` while the argument expression reads as plain `τ` (`Optional(τ)`
        // shares `τ`'s storage, and the read yields the base), so an exact comparison
        // answered no and the argument was passed BY VALUE with no `OpCreateStack` — the
        // callee then deref'd a plain integer as a stack ref and the store accessor went out
        // of bounds.  Nullability is @FR-N-Store's question, asked where a value lands, not
        // what decides whether this argument is a LINK.
        if let Type::RefVar(ref_tp) = should
            && (ref_tp.is_equal(is_type) || ref_tp.base().is_equal(is_type.base()))
        {
            // #266: a receiver/argument that is ITSELF an already-borrowed
            // reference (its declared var type is `RefVar(_)`, e.g. a `&self`
            // parameter) must be forwarded to a `&`-parameter by VALUE, not
            // re-wrapped in `OpCreateStack`.  A borrowed-reference var's slot
            // already holds a DbRef that points one level up (toward the
            // owning struct); `OpCreateStack` would build a DbRef pointing at
            // *that slot*, so the callee's single `OpGetStackRef` deref lands
            // on the borrowing frame instead of the struct — the inner
            // method's writes hit the stack store and never persist (and
            // clobber the caller's reference slot).  Passing the var through
            // unchanged gives the callee exactly the same live borrow the
            // current frame holds.  Native passes the reference by the Rust
            // ABI and is immune, so this is interpreter-correctness parity.
            // Only non-text references need this — `&text` borrows have their
            // own work-text copy semantics handled in the Text arm below.
            if !matches!(**ref_tp, Type::Text(_))
                && let Value::Var(v) = code
                && matches!(self.vars.tp(*v), Type::RefVar(vinner) if vinner.is_equal(ref_tp))
            {
                return true;
            }
            if matches!(**ref_tp, Type::Text(_)) {
                // Text → &text: use OpCreateStack for plain variables (write-back),
                // allocate a work-text copy for complex expressions (read-only).
                let orig = std::mem::replace(code, Value::Null);
                if let Value::Var(_) = &orig {
                    *code = self.cl("OpCreateStack", &[orig]);
                } else if let Value::TupleGet(t, i) = orig.unspan() {
                    // `@FR-T-Record` / `@FR-B-Ref-Lvalue` — a tuple's text member is a place: the
                    // parameter links to it, as `&s` links to a text local, so the callee's write
                    // reaches the tuple.  A member no link can honour is refused, never copied —
                    // a work copy would take the write and drop it.
                    let (t, i) = (*t, *i);
                    if let Some(place) = self.scalar_place_ref(&orig) {
                        *code = place;
                    } else {
                        if let Err(elem) = self.linkable_tuple_member(t, i) {
                            self.refuse_tuple_member_link(&elem, i);
                        }
                        *code = orig;
                    }
                } else {
                    let wv = self.vars.work_text(&mut self.lexer);
                    let mut ls = Vec::new();
                    if orig != Value::Text(String::new()) {
                        ls.push(self.cl("OpAppendText", &[Value::Var(wv), orig]));
                    }
                    ls.push(self.cl("OpCreateStack", &[Value::Var(wv)]));
                    *code = v_block(
                        ls,
                        Type::Reference(self.data.def_nr("reference"), Deps::frame1(wv)),
                        "text_ref",
                    );
                }
            } else {
                let orig = std::mem::replace(code, Value::Null);
                if let Value::Var(v) = orig {
                    // @PLN167 decision 1 — handed to a `&` parameter, a narrow local holds its
                    // field encoding from here on (`Variable::linked_narrow`).
                    self.vars.set_linked_narrow(v);
                    // @C118 — a null collection local is given its slot before the callee
                    // links to it, or an append in the callee has no place to land.
                    let mut ls = self.null_local_slot(v);
                    let create = self.cl("OpCreateStack", &[orig]);
                    *code = if ls.is_empty() {
                        create
                    } else {
                        ls.push(create);
                        Value::Insert(ls)
                    };
                } else if crate::data::is_scalar(ref_tp)
                    && Self::is_amp_place(&orig, &self.data)
                    && let Some(place) = self.scalar_place_ref(&orig)
                {
                    // A scalar field or element: the parameter links to the place itself, the
                    // reference the `&` bind would hold (@FR-B-Ref-Lvalue).  A copy in a work
                    // variable would take the callee's write and drop it.
                    *code = place;
                } else if let Value::TupleGet(t, i) = orig.unspan()
                    && let Err(elem) = self.linkable_tuple_member(*t, *i)
                {
                    // A tuple member no link can honour: refused, never copied (the copy would
                    // take the callee's write and drop it).  The argument stays in the call, so
                    // the refusal is the only diagnostic.
                    let i = *i;
                    self.refuse_tuple_member_link(&elem, i);
                    *code = orig;
                } else {
                    // produce a `Value::Insert` so that scope
                    // analysis (`scopes::scan_args`) hoists the
                    // pre-call Set into the enclosing statement list.
                    // Insert does not form a scope, so the work-ref
                    // lives at function scope and its slot survives
                    // the call.  Using `v_block` instead would create
                    // a block scope whose exit FreeStack clobbers the
                    // ref-target bytes and corrupts preceding args.
                    //
                    // The work-ref holds only a COPY of an existing
                    // DbRef — it does not own a store — so tell
                    // scopes to suppress the scope-exit `OpFreeRef`.
                    // Without `skip_free`, the shared store would
                    // be decremented once per call and eventually
                    // reach ref_count 0, dangling the caller's
                    // owning reference across loop iterations.
                    //
                    // Pass-2 ONLY: `skip_free` is a GLOBAL per-var bit on a
                    // NAME-pooled work-ref (`__ref_N`, counter-numbered per
                    // pass), and it persists in the stored var table across
                    // the pass boundary.  When the two passes' `work_refs`
                    // call sequences differ, pass 1's carrier NAME can be
                    // pass 2's OWNED literal temp — the pass-1 stamp then
                    // disarms that temp's scope-exit free and its store
                    // leaks (the p179 `&`-field-arg cell; the counter-
                    // coupling hazard, COMPILER.md).  Pass-1 IR is discarded,
                    // so the stamp's only lasting effect IS the poison.
                    let wv = self.vars.work_refs(is_type, &mut self.lexer);
                    if !self.first_pass {
                        self.vars.set_skip_free(wv);
                    }
                    *code = Value::Insert(vec![
                        v_set(wv, orig),
                        self.cl("OpCreateStack", &[Value::Var(wv)]),
                    ]);
                }
            }
            return true;
        }
        let mut check_type = is_type;
        let r = Type::Reference(self.data.def_nr("reference"), Deps::none());
        let e = Type::Enum(0, false, Deps::none());
        if let Type::Vector(_nr, _) = is_type {
            if let Type::Vector(v, _) = should
                && v.is_unknown()
            {
                return true;
            }
        } else if let Type::Reference(_, _) = is_type {
            if matches!(*should, Type::Reference(0, _)) {
                return true;
            }
            check_type = &r;
        } else if crate::parser::vectors::is_keyed(is_type) {
            // A keyed-collection handle IS a `DbRef`, so it satisfies a bare
            // `reference` parameter unchanged — no conversion op.  Used by
            // `store_persist_bind`, whose `bind_path` snapshots the whole
            // dedicated Store regardless of the collection kind.
            if let Type::Reference(rd, _) = should
                && *rd == self.data.def_nr("reference")
            {
                return true;
            }
        } else if let Type::Enum(_, false, _) = is_type {
            if *should == e {
                return true;
            }
            check_type = &e;
        }
        // @PLN25: a null literal returned/assigned where a COLLECTION is expected becomes
        // the null sentinel (store_nr=u16::MAX), reusing the reference sentinel producer
        // — distinct from an empty `[]` (a valid store with length 0).
        //
        // @FR-L-Null, through the one home `vectors::is_collection` rather than a hand-
        // spelled variant list: every store-backed collection has the same twelve-byte slot,
        // so the same thing means absent in all of them.  Spelled `Type::Vector` alone, the
        // five KEYED kinds kept a bare `Value::Null` — which writes nothing — and the
        // scope-exit `OpFreeRef` then read the untouched bytes as store #0 and tried to free
        // the STACK, exactly the fault the struct-enum arm below records for its own shape.
        // `base()` for the same reason that arm gives: the shape needing this most is the
        // nullable one (`h: hash<S[k]>? = null`).
        if *is_type == Type::Null && Self::is_collection_type(should.base()) {
            let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
            *code = Value::Call(sentinel_nr, vec![]);
            return true;
        }
        // loft#1065 — and a STRUCT-enum target, for the same reason: its slot is a
        // twelve-byte `DbRef`, so the only thing that can mean absent in it is the
        // reference sentinel.  Read through `base()`, because the shape that needs this
        // most is the nullable one (`s: Shape? = null`) — without it the slot kept a bare
        // `Value::Null`, which writes nothing, and the scope-exit `OpFreeRef` then read
        // the untouched bytes as store #0 and tried to free the STACK ("a stack-record
        // ref was treated as an owned heap store", BUG #306).
        if *is_type == Type::Null && matches!(should.base(), Type::Enum(_, true, _)) {
            let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
            *code = Value::Call(sentinel_nr, vec![]);
            return true;
        }
        // @PLN102 — `null` assigned to a value-enum target (`n: Color? = null`, a
        // return, an arg, a field) must become the enum's typed null
        // (`OpConvEnumFromNull` → the 255 sentinel), NOT stay a bare `Value::Null`
        // (byte 0).  The `FromNull` loop below matches by RETURN type, but
        // `OpConvEnumFromNull` returns the generic `enumerate`, never `is_equal` to a
        // specific `Enum(Color)` — so the conversion was skipped and the slot held 0
        // while every consumer (`== null`, `??`, `!`, `{n}`) tests the 255 sentinel,
        // leaving the value un-null-checkable.  Mirrors `Parser::null`'s Enum arm.
        // Inline `__nullable<S>` fields keep their own disc-0 representation (excluded);
        // a value-enum VECTOR element has no wired per-element null and is rejected in
        // `parse_vector` (which no longer relies on this convert failing).
        //
        // loft#1065 — the `false` is load-bearing: THREE shapes share the `Type::Enum`
        // spelling and take three different nulls, and this arm is only right for one of
        // them.  A VALUE enum is a discriminant byte, so its null is the `255` sentinel.
        // A STRUCT-enum is carried as a twelve-byte `DbRef`, so its null is the reference
        // sentinel — `formal/types.md` derives the representation from the base type's
        // storage, and a handle's is `nullref`.  Undiscriminated, this wrote one byte
        // into a handle slot: `--interpret` then read the slot as a live ref and tripped
        // its own store-lifetime guard, and `--native` would not compile at all
        // (`non-primitive cast: u8 as loft::keys::DbRef`).
        if *is_type == Type::Null
            && let Type::Enum(tp, false, _) = should.base()
            && !self.data.def(*tp).name.starts_with("__nullable<")
        {
            *code = self.cl(
                "OpConvEnumFromNull",
                &[Value::Int(i32::from(self.data.def(*tp).known_type()))],
            );
            return true;
        }
        // loft#1028 — the target is still a TYPE VARIABLE, so which null this is has not
        // been decided yet.  A template's `T` is an attribute-less placeholder STRUCT, so
        // the `Reference` arm of the loop below wins and every monomorph keeps
        // `OpNullRefSentinel`: a 12-byte DbRef written into whatever slot `T` turned out
        // to be.  Substitution rewrites the TYPE and leaves the already-chosen OP, and
        // nothing downstream re-asks.  The interpreter then read the sentinel's own bytes
        // back as the answer — `65535` for an `integer`, U+FFFF for a `character`, a
        // denormal for a `float`, the empty text for a `text` — while `--native` refused
        // the program outright, `DbRef` having no `Display`.  A `boolean` came out right,
        // but by coincidence: the sentinel's low byte is `0xFF`, which is also boolean
        // null, so it is a control that does not control.
        //
        // Stamp the site and let `rewrite_generic_type_defaults` re-run THIS conversion
        // once `T` is concrete, exactly as loft#1016 does for that declaration's DEFAULT
        // and loft#1020 for its `== null`.  Re-running the same dispatch rather than
        // answering here is what keeps one spelling of "what is `τ`'s null?" — minting a
        // second is the defect loft#1014 was.  The wrapped sentinel keeps the template's
        // own IR as well-formed as it was — a template is type-checked and slot-allocated
        // even though it never runs — and the rewrite discards it.
        // loft#1372 — a bare `null` written THROUGH a `&` link is a null value of the SLOT
        // (`@FR-C-Ref` on the write side: a `&τ` reads and writes as its referent).  Asked
        // against the LINK type, no `OpConv…FromNull` returns a `&integer?`, the conversion
        // failed, and the whole store was dropped in silence — `q = null` through a link
        // left the source holding its old value on both backends, with nothing said.
        if *is_type == Type::Null
            && let Type::RefVar(inner) = should
        {
            let inner = inner.as_ref().clone();
            return self.convert(code, is_type, &inner);
        }
        if *is_type == Type::Null
            && let Type::Reference(target, _) = should.base()
            && self.data.is_type_var_placeholder(*target)
        {
            let placeholder = Value::Call(self.data.def_nr("OpNullRefSentinel"), vec![]);
            *code = v_block(vec![placeholder], should.clone(), Self::TV_NULL_BLOCK);
            return true;
        }
        // @PLN99 Arc C — a struct/reference-returning user conversion carries a hidden
        // destination parameter (attributes() > 1), so it must go through `call_nr` (whose
        // `add_defaults` appends the dest).  But `call_nr` needs `&mut self`, and this scan
        // borrows `self.data` immutably — so record the winner and dispatch it AFTER the loop.
        let mut struct_conv: Option<u32> = None;
        for &dnr in self.data.get_possible("OpConv", &self.lexer) {
            if self.data.def(dnr).name().ends_with("FromNull") {
                if *is_type == Type::Null {
                    if matches!(self.data.def(dnr).returned(), Type::Reference(_, _))
                        && let Type::Reference(_, _) = *should
                    {
                        // Use the non-allocating sentinel instead of OpConvRefFromNull so that
                        // null comparisons (`s == null`, `s != null`) do not leak a store.
                        let sentinel_nr = self.data.def_nr("OpNullRefSentinel");
                        *code = Value::Call(sentinel_nr, vec![]);
                        return true;
                    } else if self.data.def(dnr).returned().is_equal(should) {
                        *code = Value::Call(dnr, vec![]);
                        return true;
                    }
                }
            } else if self.data.converts_with(dnr, check_type, is_type, should) {
                // @PLN99 Arc C — for a `Reference` source, `check_type` was flattened to the
                // generic `reference` (line ~2408) so stdlib `OpConv…FromRef` match any handle.
                // That hides a specific struct→struct user conversion (`fn OpConvBFromA(a: A)`),
                // so we ALSO try the concrete `is_type`.  The `returned().is_equal(should)` guard
                // keeps this exact: a candidate only wins when BOTH its source and target align.
                if self.data.attributes(dnr) > 1 {
                    struct_conv = Some(dnr); // struct-returning: dispatch after the loop
                    break;
                }
                // A character LITERAL met as an integer is its code point: the literal is
                // spelled `OpConvCharacterFromInt(cp)`, so the conversion would only undo it
                // (`c == 'x'` compared through three conversions on every evaluation).  Not for
                // NUL, the character null, which converts to the integer null rather than 0.
                if self.data.def(dnr).name() == "OpConvIntFromCharacter"
                    && let Value::Call(inner, args) = code.unspan()
                    && self.data.def(*inner).name() == "OpConvCharacterFromInt"
                    && let [arg] = args.as_slice()
                    && let Value::Int(cp) = arg.unspan()
                    && *cp != 0
                    && u32::try_from(*cp).ok().and_then(char::from_u32).is_some()
                {
                    *code = Value::Int(*cp);
                    return true;
                }
                // Stdlib primitive conversions (attributes() == 1) keep the direct Call.
                *code = Value::Call(dnr, vec![code.clone()]);
                return true;
            }
        }
        if let Some(dnr) = struct_conv {
            // Pass the CONCRETE source type (`is_type`), not the flattened `check_type` —
            // `process_call_args` needs the real type to wire the argument.  A Null result
            // means the arg couldn't be wired; leave `code` untouched and report no match.
            let src = code.clone();
            let rtp = self.call_nr(
                code,
                dnr,
                &[src],
                std::slice::from_ref(is_type),
                false,
                &[],
                None,
            );
            if rtp == Type::Null {
                return false;
            }
            // The conversion allocated a fresh owned store; hand its real return type
            // (Owned deps) to the `as` handler so it does NOT graft the source's deps.
            self.conv_owned_result = Some(rtp);
            return true;
        }
        false
    }

    /// Does casting `is_type` to `should` ALLOCATE — hand back a store of its own
    /// rather than a view of what it read?
    ///
    /// True for a text source and a heap target: the only way to answer a `vector`
    /// or a struct `ref` from text is to intern it into a new store, which is what
    /// `OpCastVectorFromText` (`db_from_text`) does. Every other cast either
    /// produces a scalar, where ownership means nothing, or reinterprets a handle it
    /// was given.
    ///
    /// Asked of the two TYPES rather than of the op that gets emitted, because the
    /// answer has to be the same on both parse passes — pass 1 has not resolved the
    /// op table yet, and its answer is the one that freezes the signature (see
    /// `cast_allocated`).
    pub(super) fn cast_allocates(is_type: &Type, should: &Type) -> bool {
        matches!(is_type, Type::Text(_))
            && matches!(
                should,
                Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _)
            )
    }

    /// Record that this cast ALLOCATED its result, so the `as` handler hands back
    /// `should` as written instead of grafting the source's deps onto it.
    ///
    /// Call it for any cast that allocates, the same way `convert`'s allocating user
    /// conversions do. `text as Struct` / `text as vector<T>` intern the text into a
    /// NEW store, so grafting makes the result read as a view of the text it parsed
    /// rather than as the fresh value it is. A borrow crossing a function
    /// return is then delivered as one, and the return-buffer machinery renames its
    /// source onto `__retbuf`: a text local retyped to the record type and sharing
    /// its slot (loft#867 — the #306 guard then SIGSEGV on the interpreter, four
    /// rustc E0308s on native), or a lifted temp bound as the buffer with the cast's
    /// value dropped on the floor (loft#866 — an empty vector, silently).
    ///
    /// The return-buffer decisions run on PASS 1 and freeze the signature, so this
    /// must be set on both passes. A pass-2-only correction arrives after the text
    /// local has already become a parameter, and nothing later can take that back.
    pub(super) fn cast_allocated(&mut self, should: &Type) {
        self.conv_owned_result = Some(should.without_deps());
    }

    /// Cast a type to another type when possible
    /// Returns false when impossible.
    pub(super) fn cast(&mut self, code: &mut Value, is_type: &Type, should: &Type) -> bool {
        if Self::cast_allocates(is_type, should) {
            self.cast_allocated(should);
        }
        if self.first_pass {
            return true;
        }
        let mut should_nr = self.data.type_def_nr(should);
        if let Type::Vector(c_tp, _) = should {
            let c_nr = self.data.type_def_nr(c_tp);
            // route through `vector_of` so narrow aliases
            // (vector<i32>, vector<u8>) get the correct narrow element
            // type — matching what fill_database registers for struct
            // fields.
            let tp = self.vector_of(c_tp);
            should_nr = self.data.check_vector(c_nr, tp, self.lexer.pos());
        }
        let should_kt = if should_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(should_nr).known_type()
        };
        let is_nr = self.data.type_def_nr(is_type);
        let is_kt = if is_nr == u32::MAX {
            u16::MAX
        } else {
            self.data.def(is_nr).known_type()
        };
        if let Type::Reference(tp, _) = should
            && self.data.def(*tp).returned().is_equal(is_type)
            && matches!(is_type, Type::Enum(_, true, _))
        {
            let get_e = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
            let get = self.cl("OpConvIntFromEnum", &[get_e]);
            if let Value::Enum(nr, _) = self.data.def(*tp).attributes()[0].value {
                *code = v_if(
                    self.cl("OpEqInt", &[get, Value::Int(i32::from(nr))]),
                    code.clone(),
                    self.cl("OpConvRefFromNull", &[]),
                );
            }
            return true;
        }
        if matches!(is_type, Type::Text(_))
            && matches!(should, Type::Enum(_, true, _) | Type::Reference(_, _))
        {
            *code = self.cl(
                "OpCastVectorFromText",
                &[code.clone(), Value::Int(i32::from(should_kt))],
            );
            return true;
        }
        for &dnr in self.data.get_possible("OpCast", &self.lexer) {
            if self.data.attributes(dnr) == 1
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned().is_same(should)
            {
                if let Type::Enum(tp, false, _) = should {
                    *code = Value::Call(
                        dnr,
                        vec![
                            code.clone(),
                            Value::Int(i32::from(self.data.def(*tp).known_type())),
                        ],
                    );
                } else {
                    *code = Value::Call(dnr, vec![code.clone()]);
                }
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned().is_same(should)
                && should_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(should_kt))]);
                return true;
            } else if self.data.attributes(dnr) == 2
                && self.data.attr_type(dnr, 0).is_same(is_type)
                && self.data.def(dnr).returned().is_same(should)
                && is_kt != u16::MAX
            {
                *code = Value::Call(dnr, vec![code.clone(), Value::Int(i32::from(is_kt))]);
                return true;
            }
        }
        false
    }

    /// Validate that two types are equal
    pub(crate) fn can_convert(&mut self, test_type: &Type, should: &Type) -> bool {
        if *test_type != *should && !test_type.is_unknown() {
            // `(Slice-Value)` — a vector slice is a value wherever a vector is expected
            // (`Parser::convert` materialises it); a keyed slice is refused at its own site.
            if let (Type::Iterator(elm, _), Type::Vector(want, _)) =
                (test_type.base(), should.base())
                && (elm.is_equal(want) || self.can_convert(elm, want))
            {
                return true;
            }
            if let Type::RefVar(tp) = should
                && tp.is_equal(test_type)
            {
                return true;
            }
            // loft#824 — the mirror of that arm, on the argument side.  `convert` peels a
            // `RefVar` SOURCE before deciding (a `&τ` value satisfies whatever `τ` does),
            // and this predicate has to answer the same question or the two disagree.  They
            // did: `len(h)` on `h: &hash<Row[id]>` needs no conversion op — a bare
            // `hash` parameter takes any parameterised hash — so `convert` emitted nothing
            // and `validate_convert` then read the UNPEELED `&hash<Row[id]>` against the
            // bare-collection arm below, which matches only `Type::Hash`, and refused the
            // call.  The plain `hash<Row[id]>` spelling took that arm and compiled.
            if let Type::RefVar(inner) = test_type
                && self.can_convert(inner, should)
            {
                return true;
            }
            if let (Type::Enum(_e, _, _), Type::Enum(o, _, _)) = (test_type, should)
                && self.data.def(*o).name() == "enumerate"
            {
                return true;
            }
            if let (Type::Reference(r_nr, _), Type::Enum(e_nr, true, _)) = (test_type, should)
                && e_nr == r_nr
            {
                return true;
            }
            // @FR-C-Var — a VARIANT satisfies the enum it belongs to: `Reference(S) ⤳
            // Enum(E)` when `S ∈ variants(E)`.  The argument, return and struct-field
            // positions all accept `Named { … }` where a `Tagged` is declared, so a
            // predicate that answers *"may this value satisfy that slot"* has to say the
            // same — asked from a fourth position it said no, and the caller then read a
            // legal value as unrelated to the slot it was going into (loft#1215).
            if let (Type::Reference(v_nr, _), Type::Enum(e_nr, _, _)) = (test_type, should)
                && self.data.def_type(*v_nr) == crate::data::DefType::EnumValue
                && self.data.def(*v_nr).parent == *e_nr
            {
                return true;
            }
            // @PLN25 E2 — a `__nullable<S>` value is accepted into a dense `S`
            // slot; `convert` emits the payload sub-ref (gap 2).
            if let (Type::Enum(enum_d, true, _), Type::Reference(struct_d, _)) = (test_type, should)
                && self.data.def(*enum_d).name
                    == format!("__nullable<{}>", self.data.def(*struct_d).name())
            {
                return true;
            }
            if let (Type::Enum(t, false, _), Type::Enum(s, false, _)) = (test_type, should)
                && *t == *s
            {
                return true;
            }
            if let (Type::Enum(_, false, _), Type::Integer(_)) = (test_type, should) {
                return true;
            }
            if let Type::Reference(r, _) = should
                && *r == self.data.def_nr("reference")
                && let Type::Reference(_, _) = test_type
                && self.generic_type_name(test_type).is_none()
            {
                return true;
            }
            // A keyed-collection argument (hash / sorted / index / spatial)
            // also satisfies a bare `reference` parameter — its handle is a
            // `DbRef`.  Mirrors the `convert` branch; used by
            // `store_persist_bind`.
            if let Type::Reference(r, _) = should
                && *r == self.data.def_nr("reference")
                && crate::parser::vectors::is_keyed(test_type)
            {
                return true;
            }
            // Bare collection parameter (sorted / hash / index / spatial)
            // accepts the corresponding parameterised collection
            // argument.  Mirrors how `Type::Vector(_, _)` matches via
            // is_same: the parameter type carries no element-type
            // constraint, so any concrete instantiation is structurally
            // compatible.  Used for stdlib helpers like `len(both: sorted)`.
            if let Type::Reference(r, _) = should {
                let r = *r;
                // Through the value's `?`: whether a keyed collection may be absent is
                // `@FR-N-Store`'s question, which `convert`'s junction has already asked and
                // warned about before this is reached, and it says nothing about which generic
                // parameter the collection is.  Matching the bare shape only refused
                // `len(h)` on a `hash<…>?` with a second verdict — after the warning — where a
                // `vector<…>?` warns and compiles.
                let (keyed, _) = test_type.peel_optional();
                let bare = (r == self.data.def_nr("sorted")
                    && matches!(keyed, Type::Sorted(_, _, _)))
                    || (r == self.data.def_nr("hash") && matches!(keyed, Type::Hash(_, _, _)))
                    || (r == self.data.def_nr("index") && matches!(keyed, Type::Index(_, _, _)))
                    || (r == self.data.def_nr("spatial") && matches!(keyed, Type::Radix(_, _, _)))
                    || (r == self.data.def_nr("trie") && matches!(keyed, Type::Trie(_, _, _)));
                if bare {
                    return true;
                }
            }
            // Text types with different dep lists are structurally compatible.
            if matches!((test_type, should), (Type::Text(_), Type::Text(_))) {
                return true;
            }
            // Function types with compatible params and return type.
            // loft#1540 — and the direction of `const`: a function whose parameter is plain may
            // not stand where the expected signature promises that parameter is `const`
            // (`ConstParams::stands_for`), or a caller trusting the promise hands a read-only
            // value to a function that may write it.
            if let (Type::Function(tp, tr, _, tc), Type::Function(sp, sr, _, sc)) =
                (test_type, should)
                && tc.stands_for(*sc)
                && tp.len() == sp.len()
                && tp.iter().zip(sp.iter()).all(|(a, b)| a.is_equal(b))
                && tr.is_equal(sr)
            {
                return true;
            }
            false
        } else {
            true
        }
    }

    /// loft#1382 — an arm-agreement mismatch deferred until statement position is known.
    pub(crate) fn arm_mismatch_report(&mut self, m: &ArmMismatch) {
        let (test, should, context, at) =
            (m.test.clone(), m.should.clone(), m.context.clone(), m.at);
        self.validate_convert(&context, &test, &should, &at);
    }

    pub(super) fn validate_convert(
        &mut self,
        context: &str,
        test_type: &Type,
        should: &Type,
        pos: &Position,
    ) {
        // The block CONTEXT doubles as the internal name `parse_block` and `block_result`
        // key on, so the one that reads as a token gets a reader's spelling here — the
        // message says *"on a match arm"* beside the twin's *"on else"*, and the keying
        // stays untouched.
        let context = if context == "match_arm" {
            "a match arm"
        } else {
            context
        };
        if !self.first_pass && !self.can_convert(test_type, should) {
            // Plan-07 phase 6 (partial) — "expected E, got G on context"
            // reads the same direction as English ("we expected this,
            // we got that"); the old shape "G should be E on context"
            // forced a mental flip and confused users new to the
            // language.  `pos` is the offending value's start, captured at
            // parse time — the lexer cursor has drifted to the `;` by now.
            let want = should.source_name(&self.data);
            let have = test_type.source_name(&self.data);
            // Two DIFFERENT definitions can render the same name — loft#1094: a package
            // declaring `Frame` while a module beside it wildcard-imports a dependency
            // that also has one.  Then "expected Frame, got Frame" names the two types
            // identically and the reader has nothing to go on; the collision is exactly
            // what they need told, and it is the one case where the position of a TYPE
            // (rather than of the value) is the useful fact.
            let (want_nr, have_nr) = (
                self.data.type_def_nr(should),
                self.data.type_def_nr(test_type),
            );
            if want == have && want_nr != have_nr && want_nr != u32::MAX && have_nr != u32::MAX {
                // @C101 — the program's type shadows a standard-library one wherever the bare
                // name is written; `std::` names the standard library's (loft#1882).
                let std_cure = if self.data.def(have_nr).source == crate::data::STD_SOURCE
                    || self.data.def(want_nr).source == crate::data::STD_SOURCE
                {
                    format!(
                        "; the program's `{want}` shadows the standard library's — write \
                         `std::{want}` where the standard library's is meant"
                    )
                } else {
                    String::new()
                };
                diagnostic_at!(
                    self.lexer,
                    pos,
                    Level::Error,
                    "expected the `{want}` declared at {}, got the one declared at {} — two \
                     different types share this name on {context}{std_cure}",
                    self.data.def(want_nr).position(),
                    self.data.def(have_nr).position()
                );
            } else {
                diagnostic_at!(
                    self.lexer,
                    pos,
                    Level::Error,
                    "expected {want}, got {have} on {context}"
                );
            }
        }
    }
}
