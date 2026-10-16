// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Variables and PLACES: creating a binding or a loop variable, what counts as an
//! addressable place for `&`, the text links a `&text` binding records, the single-pass
//! `par`-result analysis, the check that every `&` parameter is written, and the null a type
//! delivers.

use super::{
    Argument, Data, HashMap, HashSet, Level, Parser, Type, Value, diagnostic_format,
    find_field_written_vars, find_written_vars, lambda_mutated_captures, v_block,
};

impl Parser {
    // Determine if there need to be special enum functions that call enum_value variants.
    pub fn create_var(&mut self, name: &str, var_type: &Type) -> u16 {
        if self.context == u32::MAX {
            return u16::MAX;
        }
        self.vars.add_variable(name, var_type, &mut self.lexer)
    }

    pub(crate) fn create_unique(&mut self, name: &str, var_type: &Type) -> u16 {
        self.vars.unique(name, var_type, &mut self.lexer)
    }

    /// Create the variable a `for` loop binds — keyed by the LOOP, not by the name, so a
    /// second loop over one name is a second variable (loft#915).  `name` comes from
    /// [`crate::variables::Function::loop_binding`].
    pub(super) fn create_loop_var(&mut self, name: &str, var_type: &Type) -> u16 {
        if self.context == u32::MAX {
            return u16::MAX;
        }
        self.vars.loop_variable(name, var_type, &mut self.lexer)
    }

    pub(super) fn var_usages(&mut self, vnr: u16, plus: bool) {
        // @P387 — a captured-var number can arrive from a DIFFERENT var table
        // (a capturing lambda passed as a fn-value records its captures, which
        // the outer call then marks): such a number is out of range here.  Skip
        // it rather than index OOB — consistent with the `u16::MAX` guard.
        if vnr == u16::MAX || vnr >= self.vars.count() {
            return;
        }
        if plus {
            self.vars.in_use(vnr, true);
            if vnr == self.rebuild_watch {
                self.rebuild_watch_hit = true;
            }
        } else if self.vars.uses(vnr) > 0 {
            self.vars.in_use(vnr, false);
        }
    }

    /// check whether a value is "addressable" — rooted in a Var and
    /// reached through field access (OpGetField) or vector element access
    /// (OpGetVector / OpVectorRef) chains.  Addressable values produce a
    /// DbRef into existing mutable storage, so they are safe to pass as
    /// `&` parameters.
    pub(super) fn is_addressable(val: &Value, data: &Data) -> bool {
        // Plan-07 phase 1: unspan() so wraps on `[` / `.` (steps 1.11
        // / 1.12) don't hide an addressable shape from the `&` arg
        // check.
        val.is_place_read(data)
    }

    /// @PLN87 #1 — is `val` a PLACE (an addressable lvalue: a variable, struct field,
    /// or vector/array element) versus a TEMPORARY (a literal, a computed value, or a
    /// call result)?  Broader than [`is_addressable`] (which is the narrower
    /// "produces a heap DbRef" test the `&`-ARGUMENT path needs): a place includes a
    /// SCALAR field/element (`s.x` → `OpGetInt(s, …)`), whose reference codegen is a
    /// later rung (L3/L4) and copies for now — but it is still a place, not a
    /// temporary.  Used to reject `&<temporary>` at a `&` binding.  The accessor
    /// allowlist (place-GETTERS only) keeps temporary-builders like `OpGetTextSub`
    /// and every arithmetic / `n_*` op out.
    pub(super) fn is_amp_place(val: &Value, data: &Data) -> bool {
        // A stored tuple read with its address (`tuple_unbox`, built while a link parses): a
        // vector element or a struct field of tuple type, the record a link names
        // (`@FR-T-Ref-Src`, loft#1883).
        if let Some(dbref) = Self::stored_tuple_dest(val) {
            return dbref.is_place_read(data);
        }
        match val.unspan() {
            // A tuple local's member (`@FR-B-Ref-Lvalue`): the bind lowering decides which
            // member types it can link, and refuses the rest by name.
            Value::Var(_) | Value::TupleGet(_, _) => true,
            Value::Call(d_nr, args) => {
                let name = data.def(*d_nr).name();
                let listed = matches!(
                    name,
                    "OpGetField"
                        | "OpGetVector"
                        | "OpVectorRef"
                        | "OpGetInt"
                        | "OpGetFloat"
                        | "OpGetSingle"
                        | "OpGetByte"
                        | "OpGetCharacter"
                        | "OpGetBoolean"
                        | "OpGetEnum"
                        | "OpGetShort"
                        | "OpGetRecord"
                        | "OpGetRef"
                        | "OpGetDbRef"
                        | "OpGetVectorNullable"
                        // A text field or element (@PLN167 C1), and the base of a mention of
                        // a store-kind text link (`OpGetText(OpVarRef(t), 0)`).
                        | "OpGetText"
                        | "OpVarRef"
                );
                // loft#1567 — every NARROW read op, asked from the one home that names them
                // (`NarrowIntKind::get_op`) rather than re-listed here.  The hand-kept list
                // above carried `OpGetByte` and `OpGetShort` while the family had grown past
                // it: a `u16` field reads `OpGetShortFull` and an `i32` field `OpGetInt4`, so
                // both answered "not an addressable operand" — a place the language plainly
                // has, refused by a stale list.
                (listed || crate::data::NarrowIntKind::is_get_op(name))
                    && !args.is_empty()
                    && Self::is_amp_place(&args[0], data)
            }
            _ => false,
        }
    }

    /// Is this a read of a text FIELD or ELEMENT — `OpGetText(<place>, fld)` over a variable, a
    /// field or an element, the spelling `o.s` and `v[i]` of a `text` take?
    pub(super) fn is_text_place(&self, code: &Value) -> bool {
        matches!(code.unspan(), Value::Call(d, args)
            if self.data.def(*d).name() == "OpGetText"
                && args.first().is_some_and(|a| Self::is_amp_place(a, &self.data)))
    }

    /// A mention of store-kind text link `v`, spelled as the field it names:
    /// `OpGetText(OpVarRef(v), 0)`.  The link holds the slot's `DbRef`, so this is the field
    /// read `o.s` already is, and an assignment to it is the field's setter (@PLN167 decision 2:
    /// the store kind reads and writes through the field ops, the stack kind is untouched).
    pub(crate) fn store_text_link_place(&self, v: u16) -> Value {
        Value::Call(
            self.data.def_nr("OpGetText"),
            vec![
                Value::Call(self.data.def_nr("OpVarRef"), vec![Value::Var(v)]),
                Value::Int(0),
            ],
        )
    }

    /// The store-kind text link a value is a mention of — the shape
    /// [`Self::store_text_link_place`] builds — or `None`.  Anything else, a field read
    /// included, is not a link and answers `None`.
    pub(crate) fn store_text_link_of(&self, code: &Value) -> Option<u16> {
        if let Value::Call(g, args) = code.unspan()
            && self.data.def(*g).name() == "OpGetText"
            && let [r, Value::Int(0)] = args.as_slice()
            && let Value::Call(vr, vargs) = r.unspan()
            && self.data.def(*vr).name() == "OpVarRef"
            && let [Value::Var(v)] = vargs.as_slice()
            && self.vars.is_store_text_link(*v)
        {
            Some(*v)
        } else {
            None
        }
    }

    /// Record the kind a `&text` bind gives link `var_nr`: `store` for a text field or element,
    /// otherwise a text variable.  One link names texts of ONE kind (`@FR-B-Ref-Repoint`: a
    /// re-point keeps the link's `τ`, and the kind is part of what the link is), so a bind of
    /// the other kind is refused, on any path — the kind is the variable's, never a path's.
    pub(crate) fn bind_text_link_kind(&mut self, var_nr: u16, store: bool) {
        if var_nr == u16::MAX {
            return;
        }
        if store {
            self.vars.set_store_text_link(var_nr);
        }
        if self.first_pass {
            return;
        }
        match self.text_link_kinds.insert((self.context, var_nr), store) {
            Some(prev) if prev != store => {
                let name = self.vars.name(var_nr).to_string();
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    code = "text-link-kind",
                    "`{name}` links a {}, so it cannot also link a {} — a text field or element \
                     and a text variable are different places to a link. Use a second link \
                     for the other one",
                    if prev {
                        "text field or element"
                    } else {
                        "text variable"
                    },
                    if store {
                        "text field or element"
                    } else {
                        "text variable"
                    }
                );
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: "bind a second `&` link for the other place".to_string(),
                    condition: Some(
                        "both places are wanted: a link keeps the kind of place it was first \
                         bound to (a text variable, or a text field or element), so one link \
                         cannot serve both"
                            .to_string(),
                    ),
                    edit: None,
                    concept: "references",
                    concept_ref: "@F21",
                });
            }
            _ => {}
        }
    }

    /// Is this `&` operand an integer STORE place narrower than 8 bytes — an element or a field
    /// of `u8`, `i8`, `u16`, `i32` or any range that stores in fewer bytes?
    ///
    /// A link reads and writes a whole integer, so a link to such a place cannot honour its width:
    /// `(B-Ref-Reshape)` refuses it rather than let it copy or read the neighbouring bytes
    /// (`formal/binding.md` D-bind-39).  A plain variable is not a store place — a narrow local, a
    /// parameter and a tuple local all live in an 8-byte frame slot and link correctly — so it
    /// answers no for a `Var`.
    /// loft#1639 — does an explicit `&τ` ANNOTATION name a different integer type than the
    /// target does?  `Some((annotation, target))` when it does, spelled as the author would.
    ///
    /// `(B-Ref-Intro)` gives a `&`-annotated binding the type `&(typeof a)`, so the annotation
    /// cannot choose: one that disagrees is a mismatch `(C-Ref)` has no conversion for, since
    /// it converts `τ ↔ &τ` at ONE τ.  An UNANNOTATED `p = &x` already arrives here with
    /// `var_tp` inferred from the target, so it compares equal and is never refused.
    ///
    /// ⚠ Compared on (min, max, forced_size) and NOT with `==`, because `IntegerSpec` also
    /// carries `not_null` — a claim about the SLOT that a declaration and an annotation can
    /// disagree on without naming different types.  The same flag caught `non_null_reads_null`
    /// and `Data::integer_alias` out on the same day (C127, loft#1641).
    ///
    /// Scoped to INTEGERS, which is what loft#1639 measured and what the width argument is
    /// about.  A mismatch at another type is left to the machinery that already handles it
    /// rather than widened into blind: a refusal earns its reach by the cell that proves it.
    pub(super) fn amp_annotation_mismatch(
        &self,
        var_tp: &Type,
        target: &Type,
    ) -> Option<(String, String)> {
        let Type::RefVar(want) = var_tp else {
            return None;
        };
        let (Type::Integer(w), Type::Integer(g)) = (want.base(), target.base()) else {
            return None;
        };
        if w.min == g.min && w.max == g.max && w.forced_size == g.forced_size {
            return None;
        }
        // Named through `int_type_name`, which is loft#1641's fix: a type is spelled `u8` only
        // when its values ARE `u8`'s, and the author's own alias otherwise.  A message that
        // said `integer(0, 65535)` would be true and unwritable — the cure it names has to be
        // something the reader can type back in.
        Some((
            self.int_type_name(want.base()),
            self.int_type_name(target.base()),
        ))
    }

    /// Plan-06 PRIORITY.md spine step 5 — par-result use-site analyser.
    ///
    /// After the function body is fully parsed, find each
    /// `let r = parallel_for(...)` (or fused `for x in input par(r=...,
    /// N) { body }` whose result-vector flows into the body), and walk
    /// the body for uses of `r`.  Each use is classified:
    ///
    /// - **Streaming-eligible**: the result is iterated once via `for x
    ///   in r { … }` — lowers to `Stitch::Queue` in spine step 5b.
    /// - **Materialising**: random access (`r[i]`), length-of-filter,
    ///   multi-pass, alias (`r2 = r`), passed as `vector<S>` arg, stored
    ///   in `vector<S>` field, returned from `-> vector<S>` fn.  These
    ///   require the materialised vector (today's path) and will fail
    ///   compilation in spine step 7 unless rewritten via the explicit
    ///   `par_to_vec(...)` helper (planned phase 11).
    ///
    /// Spine step 7 (DONE 2026-04-29): emits `Level::Error` on
    /// materialising uses.  The deprecation window from step 5
    /// (warning) closed once the audit in step 6 confirmed zero
    /// corpus sites trigger it.  Materialising par results now fail
    /// to compile.  Streaming-eligible patterns (single
    /// `for x in r {}` use) stay silent and keep working through the
    /// existing materialised path until step 8 lands the streaming
    /// rewrite.
    pub(super) fn check_par_result_singlepass(&mut self) {
        let par_for_d_nr = self.data.def_nr("n_parallel_for");
        if par_for_d_nr == u32::MAX || self.context == u32::MAX {
            return;
        }
        let body = self.data.def(self.context).code().clone();
        // Find each (var, par_call_pos) where `Set(var, Call(n_parallel_for, ...))`.
        let mut par_results: Vec<u16> = Vec::new();
        Self::collect_par_assignments(&body, par_for_d_nr, &mut par_results);
        for v in par_results {
            // Walk the body counting uses of v.  Each use lands in a
            // category — `streaming` (Iter init position) or `other`
            // (everything else, treated as materialising for the warn
            // policy).
            let mut streaming = 0usize;
            let mut other = 0usize;
            Self::classify_var_uses(&body, v, &mut streaming, &mut other);
            // The `Set(v, par_call)` itself counts as a use.  Subtract
            // exactly one read (the par-call's appearance in arg-position
            // of the assignment is an artifact of how Set is encoded —
            // some encodings do, some don't; conservative: don't double
            // count by subtracting the assignment read).
            //
            // Heuristic: if `other > 0` AND the only `streaming` use is
            // 0, the result is purely materialising — warn.  If
            // `streaming` >= 1 and `other` >= 1, mixed-use — warn (one
            // read can't be both streamed and materialised).
            let var_name = self.vars.name(v).to_string();
            // Skip compiler-generated bindings: the fused for-par
            // desugar in `parse_parallel_for_loop` emits a hidden
            // `_par_results_N` var that's bound to a parallel_for
            // call and then indexed per iteration.  That's the
            // existing materialised path we WILL retire (step 8),
            // but warning on the compiler's own desugaring spams
            // every fused for-par site with output the user can't
            // act on.  Names starting with `_` are unique-counter
            // generated; user-typed bindings never start with `_`.
            if var_name.starts_with('_') {
                continue;
            }
            if streaming == 0 && other == 0 {
                // Result is bound but never read — par dispatched for
                // worker side effects only.  Suggest the explicit
                // form so the materialised result vector isn't
                // allocated for nothing.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par result '{var_name}' is never read — the materialised \
                     result vector is allocated but unused.  Use a fused `for x \
                     in input par(_=fn(x), N) {{}}` loop with discard policy \
                     (plan-06 spine step 3c) instead, or remove the assignment \
                     if the worker has no observable side effects."
                );
            } else if other > 0 {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "par result '{var_name}' is used in a materialising context \
                     (random access, multi-pass, or stored as vector<S>) — \
                     phase 10 of plan-06 will require single-pass consumption.  \
                     Either rewrite to a fused `for x in input par(r=fn(x), N) {{ body }}` \
                     loop, or call `par_to_vec(input, fn, N)` (planned phase 11) \
                     for an explicit materialised vector.  \
                     {streaming} streaming use(s), {other} materialising use(s) detected."
                );
            }
            // streaming == 1 && other == 0 — single-pass eligible.
            // Step 5b's lowering would rewrite to a fused for-par
            // dispatching through `run_parallel_queue` (no result
            // vector).  Deferred — bundles with step 8 (Concat
            // retirement) where the runtime question becomes
            // concrete and the rewrite has a clear destination IR.
            // For now this case is silent: existing code keeps
            // working through the materialised path.
        }
    }

    /// Plan-06 spine step 5 — find each `Set(v, Call(par_for_d_nr, ...))`
    /// occurrence in the IR tree and push `v` into `result`.  Recurses
    /// through every compound variant (Block / Loop / If / Insert /
    /// Iter / Span) so a par assignment buried inside an `if`
    /// branch or a sub-block is still found.
    pub(super) fn collect_par_assignments(val: &Value, par_for_d_nr: u32, result: &mut Vec<u16>) {
        match val {
            Value::Set(v, inner) => {
                if let Value::Call(d, _) = inner.as_ref().unspan()
                    && *d == par_for_d_nr
                {
                    result.push(*v);
                }
                Self::collect_par_assignments(inner, par_for_d_nr, result);
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    Self::collect_par_assignments(op, par_for_d_nr, result);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    Self::collect_par_assignments(op, par_for_d_nr, result);
                }
            }
            Value::If(c, t, e) => {
                Self::collect_par_assignments(c, par_for_d_nr, result);
                Self::collect_par_assignments(t, par_for_d_nr, result);
                Self::collect_par_assignments(e, par_for_d_nr, result);
            }
            Value::Iter(_, a, b, c) => {
                Self::collect_par_assignments(a, par_for_d_nr, result);
                Self::collect_par_assignments(b, par_for_d_nr, result);
                Self::collect_par_assignments(c, par_for_d_nr, result);
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::collect_par_assignments(a, par_for_d_nr, result);
                }
            }
            Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
                Self::collect_par_assignments(v, par_for_d_nr, result);
            }
            Value::TuplePut(_, _, v) => {
                Self::collect_par_assignments(v, par_for_d_nr, result);
            }
            Value::Span(b) => Self::collect_par_assignments(&b.1, par_for_d_nr, result),
            _ => {}
        }
    }

    /// Plan-06 spine step 5 — classify each `Value::Var(v)` read in the
    /// IR tree.  A read inside the `init` arm of a `Value::Iter` (the
    /// collection being iterated) counts as **streaming**; every other
    /// read counts as **other** (materialising).
    ///
    /// Counts are accumulated into the caller's mutable refs.  The
    /// `Set(v, …)` site that introduced `v` is intentionally NOT a
    /// read (Set's first field is a write target, not a read).
    pub(super) fn classify_var_uses(val: &Value, v: u16, streaming: &mut usize, other: &mut usize) {
        match val {
            Value::Var(u) if *u == v => {
                *other += 1;
            }
            Value::Var(_) => {}
            Value::Iter(_, init, next, extra) => {
                // Iter's init is the iterable expression.  A bare
                // `Var(v)` in init means `for x in v { … }` — count
                // as streaming and don't recurse into the var read.
                if let Value::Var(u) = init.as_ref()
                    && *u == v
                {
                    *streaming += 1;
                } else {
                    Self::classify_var_uses(init, v, streaming, other);
                }
                Self::classify_var_uses(next, v, streaming, other);
                Self::classify_var_uses(extra, v, streaming, other);
            }
            Value::Set(_, inner) => Self::classify_var_uses(inner, v, streaming, other),
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args {
                    Self::classify_var_uses(a, v, streaming, other);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    Self::classify_var_uses(op, v, streaming, other);
                }
            }
            Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
                for op in ops {
                    Self::classify_var_uses(op, v, streaming, other);
                }
            }
            Value::If(c, t, e) => {
                Self::classify_var_uses(c, v, streaming, other);
                Self::classify_var_uses(t, v, streaming, other);
                Self::classify_var_uses(e, v, streaming, other);
            }
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                Self::classify_var_uses(inner, v, streaming, other);
            }
            Value::TuplePut(_, _, inner) => {
                Self::classify_var_uses(inner, v, streaming, other);
            }
            Value::Span(b) => Self::classify_var_uses(&b.1, v, streaming, other),
            _ => {}
        }
    }

    /// After parsing a function body, check that each `&` (`RefVar`) argument is actually
    /// mutated somewhere in the body. If not, emit a compile error suggesting to drop the `&`.
    /// Also check for redundant `const` annotations on primitive parameters that are never
    /// written to — the `const` has no effect when the parameter is not modified.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn check_ref_mutations(&mut self, arguments: &[Argument]) {
        let code = self.data.def(self.context).code().clone();
        let mut written = crate::fxhash::FxHashSet::default();
        // interprocedural param-write cache, local to this check.
        // Re-created per function-body check; small cost, avoids
        // persisting state across passes or across unrelated checks.
        let mut callee_cache = crate::fxhash::FxHashMap::default();
        find_written_vars(
            &code,
            &self.data,
            &self.vars,
            &mut written,
            &mut callee_cache,
        );
        // Enhancement: when a for-loop variable is FIELD-WRITTEN (OpSet*
        // through the loop var, not just loop-advance Set), also mark the
        // collection it iterates over as written.  The dep chain is:
        //   it: ref(T)[_vector_1]  →  _vector_1: vector<T>[items]
        // Only propagate for vars that have a field-level write (OpSet*,
        // OpCopyRecord, OpNewRecord etc.) — not plain Set (which is just
        // the loop-iterator advance).
        let mut field_written = crate::fxhash::FxHashSet::default();
        find_field_written_vars(&code, &self.data, &mut field_written);
        let mut propagated: HashSet<u16> = HashSet::new();
        for &w in &field_written {
            if w < self.vars.next_var() {
                for dep in self.vars.tp(w).depend() {
                    propagated.insert(dep);
                    if dep < self.vars.next_var() {
                        for dep2 in self.vars.tp(dep).depend() {
                            propagated.insert(dep2);
                        }
                    }
                }
            }
        }
        written.extend(propagated);
        // `@FR-B-Ref-Write` — a write through a local LINK is a write to the place it names:
        // `c = &p; c += 10` modifies `p`.  A scalar link carries no deps
        // (`@FR-O-Borrow-Scalar`), so the dep walk above cannot see it; the bind itself says
        // which place the link names (`OpCreateStack(x)` for a local, `OpVarRef(b)` for a
        // copy of another link), and a write to the link carries back along that chain.
        // The bind is a `Set` of the link too, so the writes are measured on the body with the
        // binds blanked: a link only READ leaves its target unwritten.
        // A link to a tuple MEMBER (`c = &t.0`, `OpCreateStack(TupleGet(t, i))`) names a place
        // inside `t`, so a write through it writes `t` (`@FR-B-Ref-Lvalue`).  A member of a
        // `&(…)` tuple is the second spelling of the same link: that tuple is the caller's
        // `__tuple<…>` record (`@FR-T-Ref-Rep`), so its member link is `OpGetField(p, i)`.
        // Only a tuple record is followed: a struct's fields propagate without the `&`, so a
        // field link on a `&S` parameter leaves the advice to drop the `&` standing.
        let vars = &self.vars;
        let link_bind = |data: &Data, n: &Value| -> Option<(u16, u16)> {
            if let Value::Set(v, rhs) = n.unspan()
                && let Value::Call(op, args) = rhs.unspan()
            {
                let name = data.def(*op).name();
                if matches!(name, "OpCreateStack" | "OpVarRef")
                    && let Some(Value::Var(src) | Value::TupleGet(src, _)) =
                        args.first().map(Value::unspan)
                {
                    return Some((*v, *src));
                }
                if name == "OpGetField"
                    && matches!(vars.tp(*v).base(), Type::RefVar(_))
                    && let Some(Value::Var(src)) = args.first().map(Value::unspan)
                    && matches!(vars.tp(*src).base(), Type::RefVar(inner)
                        if matches!(inner.base(), Type::Reference(d, _)
                            if data.def(*d).name().starts_with("__tuple<")))
                {
                    return Some((*v, *src));
                }
            }
            None
        };
        let mut link_src: HashMap<u16, u16> = HashMap::new();
        code.walk(&mut |n| {
            if let Some((v, src)) = link_bind(&self.data, n) {
                link_src.insert(v, src);
            }
        });
        let mut through_written = crate::fxhash::FxHashSet::default();
        if !link_src.is_empty() {
            fn blank(code: &mut Value, data: &Data, is_bind: &impl Fn(&Data, &Value) -> bool) {
                if is_bind(data, code) {
                    *code = Value::Null;
                    return;
                }
                code.for_each_child_mut(&mut |c| blank(c, data, is_bind));
            }
            let mut unbound = code.clone();
            blank(&mut unbound, &self.data, &|d, n| link_bind(d, n).is_some());
            find_written_vars(
                &unbound,
                &self.data,
                &self.vars,
                &mut through_written,
                &mut callee_cache,
            );
            find_field_written_vars(&unbound, &self.data, &mut through_written);
        }
        let through: Vec<u16> = through_written
            .iter()
            .copied()
            .filter(|w| link_src.contains_key(w))
            .collect();
        for w in through {
            let (mut at, mut hops) = (w, 0);
            while let Some(&src) = link_src.get(&at)
                && hops < link_src.len()
            {
                written.insert(src);
                (at, hops) = (src, hops + 1);
            }
        }
        // A write from inside a CLOSURE lives in the lambda's own definition, so walking this
        // function's code cannot see it.  Two routes reach the same fact and both feed the
        // `&`-parameter test below, so both are kept: this one reads the mutated-capture set
        // already recorded on this definition (`accumulate_scalars_to_box`, filled as each
        // lambda is parsed) and answers by variable INDEX.  Without it the `const` half
        // contradicted the refusal one line above it — `fn f(p: const integer) { g = fn() {
        // p += 1; }; }` reported both "Cannot modify const parameter 'p' from a closure" and
        // "'p' is const but is never modified".
        for name in self.data.def(self.context).scalars_to_box() {
            let v = self.vars.var(name);
            if v != u16::MAX {
                written.insert(v);
            }
        }
        // The second route walks the lambda's own code and answers by parameter NAME, which
        // is what a `&` parameter needs — the lambda is its own definition (loft#1276).
        let mut lambda_written: HashSet<String> = HashSet::new();
        lambda_mutated_captures(&code, &self.data, &mut lambda_written);
        for (a_nr, a) in arguments.iter().enumerate() {
            if matches!(a.typedef, Type::RefVar(_))
                && !a.constant
                && !written.contains(&(a_nr as u16))
                && !lambda_written.contains(&a.name)
            {
                // loft#1003 — point at the `&` itself when the declaration captured it.
                // The fallback is the variable's source, which for a parameter is a position
                // inside the BODY: the caret landed on an unrelated construct and there was
                // no span to delete, so the fix could only be prose.
                let src = if a.ref_pos == (0, 0) {
                    self.vars.var_source(a_nr as u16)
                } else {
                    a.ref_pos
                };
                self.lexer.to(src);
                // T1.6: a `&` TUPLE parameter — downgrade to warning (DIAGNOSTICS.md
                // `needless-reference-parameter`); other RefVar types are an error (the & serves
                // no purpose and misleads).  Both spellings `(T-Ref-Rep)` gives a `&(…)`: the
                // stack-backed `Tuple` and the `__tuple<…>` record (loft#1673) — asked of the
                // first alone, a tuple with a heap member got the error its all-scalar twin
                // did not.
                let tuple_param = matches!(a.typedef.base(), Type::RefVar(inner)
                    if matches!(inner.base(), Type::Tuple(_))
                        || matches!(inner.base(), Type::Reference(d, _)
                            if self.data.def(*d).name().starts_with("__tuple<")));
                if tuple_param {
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        code = "needless-reference-parameter",
                        "Parameter '{}' does not need to be a reference",
                        a.name
                    );
                    // The cure is deleting one token, so the edit is that token's span with
                    // empty replacement text — applicable exactly when the position is real.
                    let edit = (a.ref_pos != (0, 0)).then(|| crate::diagnostics::Edit {
                        line: a.ref_pos.0,
                        col: a.ref_pos.1,
                        len: 1,
                        text: String::new(),
                    });
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Mechanical,
                        title: "drop the `&`".to_string(),
                        condition: None,
                        edit,
                        concept: "reference",
                        concept_ref: "@F21",
                    });
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Parameter '{}' has & but is never modified; remove the &",
                        a.name
                    );
                }
            }
            // warn when `const` is used on a primitive parameter that is never
            // written to — the annotation is redundant since the parameter would not
            // have been modified anyway.  Compound types (vector, reference, struct)
            // are exempt: `const` serves as read-only documentation on those.
            let base_tp = if let Type::RefVar(inner) = &a.typedef {
                inner.as_ref()
            } else {
                &a.typedef
            };
            if a.constant
                && !written.contains(&(a_nr as u16))
                && matches!(
                    base_tp,
                    Type::Integer(_) | Type::Float | Type::Single | Type::Boolean | Type::Character
                )
            {
                // loft#1003 — the `const` token's own position, as for the `&` above.
                let src = if a.const_pos == (0, 0) {
                    self.vars.var_source(a_nr as u16)
                } else {
                    a.const_pos
                };
                self.lexer.to(src);
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    code = "needless-const-parameter",
                    "Parameter '{}' is const but is never modified; \
                     'const' has no effect on an unmodified primitive parameter",
                    a.name
                );
                // `const` plus the one space that separates it from the type: deleting the
                // keyword alone would leave `a:  integer`, which is legal but is not what a
                // hand-applied fix looks like.
                let edit = (a.const_pos != (0, 0)).then(|| crate::diagnostics::Edit {
                    line: a.const_pos.0,
                    col: a.const_pos.1,
                    len: 6,
                    text: String::new(),
                });
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Mechanical,
                    title: "drop the `const`".to_string(),
                    condition: None,
                    edit,
                    concept: "const parameters",
                    concept_ref: "@F18",
                });
            }
        }
    }

    // <function> ::= 'fn' <identifier> '(' <attributes> ] [ '->' <type> ] (';' <rust> | <code>)
    // @C90 — each nullable scalar's null is ONE reserved in-band value; this is the table.
    pub fn null(&mut self, tp: &Type) -> Value {
        // @PLN25 slice (b): an `Optional(τ)`'s null is `τ`'s typed null (same sentinel) — peel.
        match tp.base() {
            Type::Integer(_) => self.cl("OpConvIntFromNull", &[]),
            // `character` is a 4-byte (char-domain) type: its null is `'\0'`
            // (`OpConvCharacterFromNull`), NOT the i64 integer sentinel.  Folding
            // it into `OpConvIntFromNull` (i64::MIN) made native emit `i64::MIN`
            // into an `i32` character return slot — rustc E0308 (Cluster D H4).
            Type::Character => self.cl("OpConvCharacterFromNull", &[]),
            Type::Boolean => {
                // @PLN17 spike: boolean is tri-state (255 = null sentinel).
                // Supersedes the #256 rejection — `null` on a boolean now emits the
                // 255 sentinel; truthiness contexts coerce it to false.
                self.cl("OpConvBoolFromNull", &[])
            }
            // loft#1065 — a STRUCT-enum is a `DbRef`, so its null is the reference
            // sentinel, not the `255` discriminant byte a value enum uses.  This is the
            // answer [`Self::null_value`] already documents; the two now agree.
            Type::Enum(_, true, _) => self.cl("OpNullRefSentinel", &[]),
            Type::Enum(tp, false, _) => self.cl(
                "OpConvEnumFromNull",
                &[Value::Int(i32::from(self.data.def(*tp).known_type()))],
            ),
            Type::Float => self.cl("OpConvFloatFromNull", &[]),
            Type::Single => self.cl("OpConvSingleFromNull", &[]),
            Type::Text(_) => self.cl("OpConvTextFromNull", &[]),
            Type::RefVar(tp) if matches!(**tp, Type::Text(_)) => self.cl("OpConvTextFromNull", &[]),
            Type::Reference(_, _) => self.cl("OpNullRefSentinel", &[]),
            // loft#1469 — a fn-ref's null is the full twenty-byte shape with d_nr 0, the
            // same value [`crate::data::to_default`] already builds for a default-initialised
            // fn-ref field, and `set_field_check`'s `Type::Function` arm reduces both to the
            // 4-byte `d_nr = 0` storage write.  Untreated it fell to the catch-all below and
            // pushed NOTHING, which is the failure this function's sibling `null_value`
            // documents one paragraph down: the consumer read an uninitialised 12-byte
            // closure half, judged it non-null and freed it ("refused free of out-of-range
            // store", then SIGSEGV).  It reached a value position through the fallback of a
            // non-total `match` over an enum whose arms yield lambdas, where nothing else
            // names the width.
            Type::Function(..) => Value::FnRef(0, u16::MAX, Box::new(tp.base().clone())),
            _ => Value::Null,
        }
    }

    /// loft#793/loft#936 — the null a VALUE POSITION delivers for `tp`:
    /// `null()`, plus the store-pointer sentinel for every remaining
    /// DbRef-backed heap type (the collections, and a struct-enum — whose
    /// payload is a record, not the `255` discriminator a plain enum uses).
    ///
    /// A value position is anywhere the null travels on the eval stack rather
    /// than into a declared slot: a `return`, and the arm of an `if`/`match`
    /// whose sibling arm decides the merged type.
    ///
    /// Separate from `null()` because that one also supplies a variable's
    /// DEFAULT-INIT, and the two want different answers here: a collection
    /// LOCAL must start as an ALLOCATED empty store, because a later write
    /// (`w: vector<single> = f#read(16) as vector<single>`) fills that store in
    /// place and the sentinel's `store_nr` indexes nothing.  A value in flight
    /// has no slot to fill — it travels as a DbRef — so there the sentinel is
    /// the only thing that can mean null.
    ///
    /// Untreated, `null()`'s catch-all answers a bare `Value::Null`, which
    /// pushes NOTHING where the consumer reads a 12-byte DbRef, so the
    /// interpreter hands on whatever the eval stack held: the consumer saw an
    /// uninitialised ref as NON-null and freed it ("refused free of
    /// out-of-range store", sometimes a SIGSEGV).  Native was already right, so
    /// the two backends disagreed — on a bare `return null` (#793), and on
    /// `if n == 0 { null } else { [n] }`, where the null arm is parsed BEFORE
    /// the sibling that names the type and so is back-patched here (#936).
    pub fn null_value(&mut self, tp: &Type) -> Value {
        // A TYPE VARIABLE's null is a function of the type it is bound to, so it is asked
        // again by each monomorph (`TV_NULL_VALUE`).  Answered here as a reference null, a
        // generic `match` returning `T` fell back to a `DbRef` in an `integer` instance —
        // native refused to compile it.
        if let Type::Reference(d, _) = tp.base()
            && self.data.is_type_var_placeholder(*d)
        {
            let placeholder = self.cl("OpNullRefSentinel", &[]);
            return v_block(vec![placeholder], tp.clone(), Self::TV_NULL_VALUE);
        }
        match tp.base() {
            Type::Vector(_, _)
            | Type::Sorted(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
            | Type::Index(_, _, _)
            | Type::Enum(_, true, _) => self.cl("OpNullRefSentinel", &[]),
            _ => self.null(tp),
        }
    }
}
