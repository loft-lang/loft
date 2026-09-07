// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    AmpHead, Data, IntegerSpec, Level, OPERATORS, Parser, Position, Type, Value, diagnostic_format,
    rename, to_default, v_block, v_if, v_set,
};

// Operator parsing and type dispatch.

/// An integer literal at the width the value needs: `Value::Int` carries an `i32`, so a
/// bound outside that range (a `u32` maximum, say) must be emitted as `Value::Long` or it
/// wraps into a nonsensical guard.  Both are `Type::Integer` at rest, so the choice is
/// invisible downstream.
fn int_literal(n: i64) -> Value {
    i32::try_from(n).map_or(Value::Long(n), Value::Int)
}

impl Parser {
    /// Whether a DIRECT write (`nr = …` / `nr += …`) with operator `op` to binding
    /// `nr` must be rejected.
    ///
    /// Enforces @FR-Const-Bind and @FR-Const-Value — the two axes of the const model —
    /// for the case where the write TARGET is the bare variable.  @FR-Const-ScalarCollapse
    /// falls out of it: a by-value scalar has no interior distinct from its binding, so
    /// both axes reject and either spelling freezes it fully.  @FR-Const-Compose is the
    /// two axes together, and needs no arm of its own: each rejects its own operation.
    ///
    /// The two const axes reject opposite operations:
    /// - **binding-const** (`const x`, prefix): the slot is write-once — reject a
    ///   rebind (`=`); allow contents mutation (`+=`).
    /// - **value-const** (`p: const T`, type-qualifier): the value is read-only —
    ///   reject contents mutation (`+=`); allow a rebind (`=`) that re-points the slot.
    ///
    /// This guards only writes whose target IS the bare variable.  A mutation through
    /// a value-const binding — `p.x = …`, `p[i] = …` — has no `Value::Var` target and
    /// is caught by `validate_write`'s base-resolution instead.
    /// (Phase 1 of the const model, doc/claude/plans/40-const-fields/const-model.md.)
    pub(crate) fn const_write_blocked(&self, nr: u16, op: &str) -> bool {
        if self.first_pass || nr == u16::MAX {
            return false;
        }
        let binding = self.vars.is_const_binding(nr);
        let value = self.vars.is_value_const(nr);
        if !binding && !value {
            return false;
        }
        // Rule 1 — scalar collapse: a by-value scalar (integer/float/…/character) has no
        // contents distinct from its binding, so `+=` rewrites the whole value exactly
        // as `=` does.  Both axes therefore make it FULLY immutable (reject `=` AND `+=`).
        // A `&`-reference binding writes THROUGH to its referent, so a value-const `&`
        // param (`& const T`) has no local rebind either — every write mutates the
        // referent — and is likewise fully blocked.
        let tp = self.vars.var_type(nr).base();
        let collapses = matches!(
            tp,
            Type::Integer(_) | Type::Float | Type::Single | Type::Boolean | Type::Character
        ) || matches!(self.vars.var_type(nr), Type::RefVar(_));
        // binding-const rejects a rebind (`=`); value-const rejects contents mutation
        // (`+=`) while allowing a `=` rebind that re-points the slot.  Under collapse both
        // reject everything.
        let binding_blocks = binding && (op == "=" || collapses);
        let value_blocks = value && (op != "=" || collapses);
        binding_blocks || value_blocks
    }

    /// Reject a write that `const` forbids on `nr`, reporting against the binding the
    /// SOURCE names.
    ///
    /// The one home for the const question on the assignment path: [`Parser::parse_assign_op_inner`]
    /// calls it before it picks a lowering route, because the answer is a property of the
    /// binding and not of the route.  Held inside a route, the guard is only as complete as
    /// that route's target-shape test, and the shapes it declines fall through unchecked.
    pub(crate) fn guard_const_write(&mut self, nr: u16, op: &str) {
        if !self.const_write_blocked(nr, op) {
            return;
        }
        // `const_report_var` — see loft#1250: a const text argument is promoted to a
        // `__tp_` local, and the promoted local is not marked an argument, so reporting
        // against it demotes "const parameter" to "const variable".
        let report = self.vars.const_report_var(nr);
        diagnostic!(
            self.lexer,
            Level::Error,
            "Cannot modify {} '{}'; remove 'const' or use a local copy",
            self.const_noun(report),
            self.vars.name(report)
        );
    }

    pub(crate) fn assign_text(
        &mut self,
        code: &mut Value,
        tp: &Type,
        to: &Value,
        op: &str,
        var_nr: u16,
    ) {
        // The const guard is `parse_assign_op_inner`'s, run before it routed here.
        if let Value::Call(_, parms) = to.unspan().clone() {
            if op == "=" {
                let mut p = parms.clone();
                p.push(code.clone());
                *code = self.cl("OpSetText", &p);
            } else {
                let mut ls = Vec::new();
                ls.push(v_set(var_nr, to.clone()));
                if let Value::Insert(cd) = code {
                    for c in cd {
                        ls.push(c.clone());
                    }
                } else if *tp == Type::Character {
                    ls.push(self.cl("OpAppendCharacter", &[Value::Var(var_nr), code.clone()]));
                } else {
                    ls.push(self.cl("OpAppendText", &[Value::Var(var_nr), code.clone()]));
                }
                let mut p = parms.clone();
                p.push(Value::Var(var_nr));
                ls.push(self.cl("OpSetText", &p));
                *code = Value::Insert(ls);
            }
        } else if let Value::Insert(ls) = code {
            if op == "=" {
                // P217: detect `h = h + expr`: the first Insert entry is a
                // self-append `OpAppendText(var, Var(var))`.  Convert to
                // `+=` semantics by removing the self-append and skipping
                // the up-front clear.  Without this, the clear destroys
                // h's content before the appends read it.
                //
                // The `args[1]` operand is whatever `parse_append_text`
                // received as `code` — a `Var(var)` wrapped by the parser
                // in a `Value::Span` for source-position tracking.  We
                // unspan to compare structural identity, not literal
                // equality (the original `args[1] == Value::Var(var_nr)`
                // check missed every Span-wrapped self-reference and let
                // the clear-then-append path corrupt `h`).
                let self_append = ls.first().is_some_and(|first| {
                    if let Value::Call(_, args) = first.unspan() {
                        args.len() >= 2
                            && matches!(args[0].unspan(), Value::Var(v) if *v == var_nr)
                            && matches!(args[1].unspan(), Value::Var(v) if *v == var_nr)
                    } else {
                        false
                    }
                });
                if self_append {
                    ls.remove(0);
                } else {
                    ls.insert(0, v_set(var_nr, Value::Text(String::new())));
                }
            }
        } else if op == "=" && var_nr != u16::MAX {
            // A branch RHS (`q = if …`, `q = match …`, `q = a ?? b`) delivers per ARM
            // into `q` instead of being lowered as a value — see `try_branch_text_bind`.
            if self.try_branch_text_bind(code, var_nr) {
                // `code` is now a void branch of `Set(q, …)`; nothing further to wrap.
            }
            // detect self-reference (t = t[N..], t = fn(t), etc.)
            // If the RHS reads from the same variable being assigned, use a
            // work text to avoid the clear-before-read problem.
            else if code.reads_var(var_nr) {
                let work = self.vars.work_text(&mut self.lexer);
                let ls = vec![
                    self.cl("OpClearText", &[Value::Var(work)]),
                    self.cl("OpAppendText", &[Value::Var(work), code.clone()]),
                    v_set(var_nr, Value::Var(work)),
                ];
                *code = Value::Insert(ls);
            } else {
                *code = v_set(var_nr, code.clone());
            }
        } else if *tp == Type::Character {
            *code = self.cl("OpAppendCharacter", &[Value::Var(var_nr), code.clone()]);
        } else {
            *code = self.cl("OpAppendText", &[Value::Var(var_nr), code.clone()]);
        }
    }

    /// `snapshot_len` is how many ops at the head of `code` are the literal's snapshot of the
    /// destination it reads ([`Parser::snapshot_read_destination`]); the `=` repoint this
    /// inserts lands after them.
    pub(crate) fn create_vector(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        op: &str,
        var_nr: u16,
        snapshot_len: usize,
    ) -> bool {
        // @PLN25 / loft#909 — whether the target needs a record-pointer BACKING is a
        // question about its storage, and `Optional(τ)` shares τ's storage exactly.  Peel
        // the marker for that question; every nullability check downstream reads the
        // unpeeled type, because those ARE about nullability.
        //
        // Unpeeled, this selector missed a `vector<T>?` LOCAL, and only for an EMPTY
        // literal: `v: vector<integer>? = [1, 2]` was rescued by its elements, whose own
        // `build_vector_list` allocates the backing, while `[]` has no element to do it —
        // so nothing ever defined `v` and it reached codegen with no stack slot at all
        // (*"Incorrect var v[65535]"*, an internal compiler error on both backends).  That
        // is the loft#909 shape from the other side: there a nullable FIELD took the temp
        // path and the EMPTY literal was what rescued it.
        let f_storage = f_type.base();
        if let (Value::Insert(ls), Type::Vector(tp, _)) = (code, f_storage) {
            // The const guard is `parse_assign_op_inner`'s, run before it routed here.
            if op == "=" {
                // Self-concat reassign `v = v + [...]` (sibling of @P390's
                // self-slice `v = v[a..b]`): `parse_append_vector` emitted a
                // leading identity `Set(v, Var(v))` because the concat's
                // accumulator IS the reassignment target.  The `vector_db` splice
                // below allocates a FRESH store for v and repoints v to it (empty)
                // BEFORE that Set / the append read v's OLD contents → v's original
                // elements are lost (only the appended tail survives; `v=[1,2];
                // v=v+[9]` gave `[9]`).  When the body is self-referential, skip
                // the new-store allocation and drop the now-useless identity-set
                // so the parts append IN PLACE to v's existing store — exactly what
                // `v += [...]` does.  The discriminator is the IR shape
                // `Set(var_nr, Var(var_nr))` (absent for `u = v + [9]` →
                // `Set(u, Var(v))` and `v = a + b` → `Set(v, Var(a))`), identical
                // on both parser passes — pass-stable, no @P384 alloc divergence.
                let self_ref = ls.first().is_some_and(|first| {
                    matches!(first.unspan(), Value::Set(s, rhs)
                        if *s == var_nr
                            && matches!(rhs.unspan(), Value::Var(r) if *r == var_nr))
                });
                if self_ref {
                    ls.remove(0);
                } else {
                    // Trailing self-reference `v = a + v` (the accumulator `v`
                    // appears as a NON-first operand): the parts loop emitted
                    // `OpAppendVector(v, <operand mentioning v>)` which would read
                    // v AFTER the `vector_db` clear below empties it → v's OLD
                    // contents are lost (the trailing read sees the freshly-copied
                    // prefix instead).  Sibling of @P390's self-slice.  Materialise
                    // each such operand's OLD value into a fresh temp BEFORE the
                    // clear (deep copy while v still holds its contents), then
                    // rewrite the operand to read the temp.  The first-operand
                    // deep-copy (`OpAppendVector(v, Var(a))`, a != v) never matches;
                    // the self-concat first-operand case is the `self_ref` branch
                    // above.  Pass-2 work (vector_db / create_unique are pass-2),
                    // matching the @P390 / @P287 temp precedent.
                    let mut prefix: Vec<Value> = Vec::new();
                    if !self.first_pass {
                        let append_nr = self.data.def_nr("OpAppendVector");
                        for stmt in ls.iter_mut() {
                            if let Value::Call(d, args) = stmt
                                && *d == append_nr
                                && args.len() == 3
                                && matches!(args[0].unspan(), Value::Var(t) if *t == var_nr)
                                && args[1].reads_var(var_nr)
                            {
                                let rec = args[2].clone();
                                let tmp = self.create_unique("__trail_tmp", f_type);
                                self.vars.defined(tmp);
                                let mut mat = self.vector_db(tp, tmp);
                                mat.push(self.cl(
                                    "OpAppendVector",
                                    &[Value::Var(tmp), Value::Var(var_nr), rec],
                                ));
                                prefix.extend(mat);
                                args[1] = Value::Var(tmp);
                            }
                        }
                    }
                    // plan-57 cluster II — a literal / comprehension / struct-vector
                    // init already allocated v's store in the RHS body (the literal's
                    // own `vector_db` emitted a head `Set(v, OpGetField(__vdb))`).  The
                    // `=` `vector_db` below would allocate a SECOND, immediately-
                    // orphaned store for v (the 2× store high-watermark).  Skip it when
                    // the body already allocated.  Concat (`OpAppendVector` / `Set(v,
                    // <temp>)`) and reassignment (`a=[1,2,3]; a=[4,5]` — no head alloc)
                    // have no such repoint, so they KEEP the `vector_db`: concat needs
                    // it for v's store, reassignment relies on the fresh store as its
                    // clear (cluster III — unchanged by this fix).  Pure watermark
                    // optimisation; results + lifetimes are identical.
                    let get_field_nr = self.data.def_nr("OpGetField");
                    let body_allocates = ls.iter().any(|stmt| {
                        matches!(stmt.unspan(),
                            Value::Set(s, rhs) if *s == var_nr
                                && matches!(rhs.unspan(), Value::Call(d, _) if *d == get_field_nr))
                    });
                    // Cluster I-d (@PLN85 cluster V) — `a = <call> + …`:
                    // parse_append_vector emits a leading `Set(a, call())` so `a`
                    // ADOPTS the call's fresh `["??"]` store (no copy).  Allocating a
                    // SECOND `__vdb` backing here orphans that empty store — it leaks
                    // when `a` escapes (the N shape: `a = head()+head()`, returned).
                    // The adopt IS the body providing a's store, exactly like the
                    // literal-init `body_allocates` case — so skip `vector_db` too.
                    let body_adopts_call = ls.first().is_some_and(|first| {
                        matches!(first.unspan(), Value::Set(s, rhs) if *s == var_nr
                            && matches!(rhs.unspan(), Value::Call(d, _)
                                if self.data.def(*d).name().starts_with("n_")
                                    && *self.data.def(*d).code() != Value::Null
                                    && !self.data.def(*d).returns_borrowed_view()))
                    });
                    // The materialise prefix reads v while it is still intact, so
                    // it MUST precede the `vector_db` clear: insert prefix ++
                    // vector_db ops together, ahead of the body (prefix first).
                    let mut front = prefix;
                    if !body_allocates && !body_adopts_call {
                        let db_ops = self.vector_db(tp, var_nr);
                        // @PLN85 #492 — `vector_db` no-ops on a non-rebind ARGUMENT (the
                        // NRVO return buffer IS the caller's store, so it cannot get a
                        // fresh backing).  Without a fresh store, a `=` NON-EMPTY literal
                        // reassignment on the buffer (`r = [9,9]` in a JOIN arm, after an
                        // earlier arm's `r = x` filled it) would APPEND to the stale
                        // content instead of REPLACING — the returned vector piles both
                        // arms (`[1,2,3,9,9]`).  Clear the buffer first: `=` is a replace,
                        // so this is correct on a fresh buffer (a no-op) and a filled one.
                        // The empty-literal `v = []` reassign is handled by the
                        // `ls.is_empty()` clear below; a rebind param gets a fresh backing
                        // (`db_ops` non-empty), so neither reaches here.
                        // loft#1371 — a `&`-LINKED vector LOCAL (`pe = &e`) is the same
                        // question one step out: a fresh backing would re-point the link,
                        // so the whole-value write must clear the SHARED store and refill
                        // it in place.  `vector_db` DOES allocate for a local, so this arm
                        // drops `db_ops` rather than finding it empty.  The empty literal
                        // (`pe = []`) takes its clear from the `ls.is_empty()` block below.
                        let amp_linked = !self.first_pass
                            && var_nr != u16::MAX
                            && self
                                .amp_vector_locals
                                .contains(&(self.context, self.vars.name(var_nr).to_string()));
                        if amp_linked {
                            if !ls.is_empty() {
                                front.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
                            }
                        } else if db_ops.is_empty()
                            && !self.first_pass
                            && self.vars.is_argument(var_nr)
                            && !ls.is_empty()
                        {
                            front.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
                        } else {
                            front.extend(db_ops);
                        }
                    }
                    // After the literal's snapshot of the destination, when it took one
                    // (`snapshot_read_destination`): the repoint below is the detach the
                    // snapshot must precede.
                    let at = snapshot_len;
                    for (i, p) in front.into_iter().enumerate() {
                        ls.insert(at + i, p);
                    }
                    if ls.is_empty()
                        && !self.first_pass
                        && var_nr != u16::MAX
                        && matches!(f_type, Type::Vector(_, _))
                    {
                        ls.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
                    }
                }
            }
            true
        } else {
            false
        }
    }

    /// P193: eager init for `local: keyed_collection<T[key]> = []`.
    ///
    /// Without this, the empty `[]` literal parses to `Value::Insert(empty)`
    /// which doesn't match either the `Set(v, Value::Null)` arm of
    /// codegen (which would call `gen_set_first_keyed_null`) nor the
    /// vector-style `create_vector` rewrite.  The variable then has no
    /// init bytecode emitted: subsequent reads hit u16::MAX slot, and
    /// when the FIRST WRITE is inside a loop body the lazy init runs
    /// once per iteration — every iteration zeros the collection's
    /// root pointer.  Symptom: `for i in 0..N { ix += ... }` over a
    /// local-var keyed collection leaves `len(ix) == 1`.
    ///
    /// Fix: rewrite `Set(v, Insert(empty))` to `Set(v, Null)` for
    /// keyed-collection types and op == "=", so the standard codegen
    /// path emits `gen_set_first_keyed_null` at the declaration site
    /// (outside any loop body).
    ///
    /// Returns true when the rewrite happened so the caller short-
    /// circuits the rest of the assign pipeline.
    pub(crate) fn create_keyed(code: &mut Value, f_type: &Type, op: &str, var_nr: u16) -> bool {
        if op != "=" || var_nr == u16::MAX || !crate::parser::vectors::is_keyed(f_type) {
            return false;
        }
        let is_empty_insert = matches!(code, Value::Insert(ls) if ls.is_empty());
        if !is_empty_insert {
            return false;
        }
        // The const guard is `parse_assign_op_inner`'s, asked before it routed here.
        // Codegen's Set(v, Null) arm matches keyed types and dispatches
        // to gen_set_first_keyed_null — emits OpInitRef + OpDatabase
        // for the slot, anchored at the declaration's statement
        // position (NOT inside any enclosing loop body).
        *code = Value::Null;
        true
    }

    /// Check whether `val` is a call to a user-defined function that returns a struct
    /// via a temporary store.  Used by `copy_ref` and the vector-append
    /// emit path (`vectors.rs`) to decide whether to free the source
    /// store after the deep copy.  The free bit's behaviour differs
    /// under WASM but the query is the same on every target — call
    /// sites in expressions.rs / objects.rs / vectors.rs / collections.rs
    /// are not feature-gated, so this helper must not be either.
    pub(crate) fn is_struct_returning_call(&self, val: &Value) -> bool {
        if self.first_pass {
            return false;
        }
        match val.unspan() {
            Value::Call(fn_nr, args) => {
                let def = &self.data.def(*fn_nr);
                // User function with code (not a built-in op)
                def.name().starts_with("n_")
                    && *def.code() != Value::Null
                    && !self.answers_caller_buffer(*fn_nr, args)
            }
            // Struct constructor blocks allocate a store too — when assigned
            // to a field, the source store is a temporary that should be freed.
            Value::Block(bl) => bl.name == "Object",
            _ => false,
        }
    }

    /// loft#1154 — the `0x8000` source-free decision for a JOIN right-hand side, and the
    /// witnesses the @P290 bracket must mark for it.
    ///
    /// [`Self::is_struct_returning_call`] asks *is the RHS a call*, and
    /// `g = if c { mk(1) } else { mk(2) }` is not one — so the bit was never set and the store
    /// the taken arm's callee minted was copied out of and then abandoned, once per evaluation
    /// of a call arm.
    ///
    /// A join may free its source when EVERY arm is one of two things: a fresh-storage CALL,
    /// whose store nothing else names, or a value the bracket can NAME, which the runtime then
    /// refuses to free.  An arm that is neither leaves the decision unmakeable, and the
    /// conservative never-free stands — a leak, where the alternative is taking the caller's
    /// store out from under it.  That split is why this cannot be a static "are the arms
    /// calls" test: `if c { mk(1) } else { m }` mixes the two, and which arm ran is a runtime
    /// fact.
    ///
    /// `Some(witnesses)` licenses the bit; `None` keeps today's answer.
    pub(crate) fn join_source_frees(&self, code: &Value) -> Option<Vec<u16>> {
        let mut arms: Vec<&Value> = Vec::new();
        Self::join_arms(code.unspan(), &mut arms);
        if arms.len() < 2 {
            return None;
        }
        let mut witnesses: Vec<u16> = Vec::new();
        let mut any_freeing_call = false;
        for a in &arms {
            if self.is_struct_returning_call(a)
                && crate::use_analysis::call_return_frees_source(&self.data, self.context, a)
            {
                any_freeing_call = true;
                witnesses.extend(
                    crate::use_analysis::protectable_ref_args(&self.data, self.context, a).0,
                );
                continue;
            }
            witnesses.extend(crate::use_analysis::view_root_slots(&self.data, a)?);
        }
        any_freeing_call.then_some(witnesses)
    }

    /// The terminal value of every arm an `if` / `else if` chain can answer with.
    ///
    /// A one-operator block is how the parser wraps a bare arm and carries nothing of its own;
    /// a block with construction ops in it is a MINT and is the arm itself.
    fn join_arms<'a>(val: &'a Value, out: &mut Vec<&'a Value>) {
        match val.unspan() {
            Value::If(_, t, f) => {
                Self::join_arms(t, out);
                Self::join_arms(f, out);
            }
            // loft#1157 — `??` is a join in the LANGUAGE and its IR is a block named `ncc`
            // holding the SUBJECT in a temp and then the `if` that chooses.  The arms live in
            // that tail `if`, so reaching them is a descent and not a special case; without it
            // `X ?? mk(…)` kept the conservative never-free and retained the default arm's
            // store.
            //
            // ⚠ The subject reaches that `if` as a plain `Var(__ncc_N)`, and taking it at face
            // value is wrong in the expensive direction: `view_root_slots` NAMES a var, so the
            // bracket would PROTECT the temp — and that temp is the one thing here nobody else
            // owns, so protecting it is what keeps the present-path store alive forever.  The
            // subject's own ASSIGNMENT is what the decision is about, so it is substituted in.
            Value::Block(bl) if bl.name == "ncc" => {
                let Some(last) = bl.operators.last() else {
                    return;
                };
                let subject = bl.operators.iter().find_map(|op| match op.unspan() {
                    Value::Set(v, val) if !matches!(val.unspan(), Value::Null) => {
                        Some((*v, val.as_ref()))
                    }
                    _ => None,
                });
                let mut raw = Vec::new();
                Self::join_arms(last, &mut raw);
                for a in raw {
                    match (a.unspan(), subject) {
                        (Value::Var(v), Some((sv, val))) if *v == sv => out.push(val),
                        _ => out.push(a),
                    }
                }
            }
            // A value block's arms are its TAIL's: a plain arm block holds the one value, and
            // a `match` lowers to a block that binds its subject first (`scalar_match`,
            // `tuple_match`, `vector_match`, …) and then holds the `if` chain that chooses
            // — the same shape as `ncc` above, without a subject to substitute.  Taking the
            // whole block as one arm made every `match` a value the bracket cannot name, so
            // the conservative never-free stood: `x = match k { 0 => { mk(1) }, _ => { mk(2)
            // } }` on a keyed local copied the taken arm's fresh store out and abandoned it,
            // one per evaluation, where the `if … else if …` spelling of the same program
            // freed it (loft#1154's fix reached the `if`, not the `match`).
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last() {
                    Self::join_arms(last, out);
                }
            }
            other => out.push(other),
        }
    }

    /// Give an OWNING call a home in THIS frame, so a join can VIEW it.
    ///
    /// [`formal/ownership.md`](../../doc/claude/formal/ownership.md) settles the shape:
    /// a join whose arms disagree about ownership is classified conservatively as a
    /// view, and that is correct *"because the OWNED arm is materialised into the
    /// return buffer whose own lifetime frees it"*.  The materialisation was the half
    /// that did not exist — for `??` (loft#1013) and then for a plain `if`, an
    /// `else if` chain and every `match` form (loft#1019).  Without it the fresh store
    /// the owning arm allocated is owned by no frame at all, while the consumer reads
    /// the join as a borrow and frees nothing: one leaked record per evaluation,
    /// unbounded in a loop.
    ///
    /// The buffer to bind is the one the call ALREADY carries — a heap-returning
    /// callee is handed a hidden `__ref_N` destination the caller mints and frees
    /// (`add_defaults`), and all that is missing is capturing what the call answered
    /// into it.  Reusing that buffer keeps ONE owner and ONE free; a freshly minted
    /// second work-ref would double-free the callee that DOES deliver through its
    /// buffer, which `is_struct_returning_call` already excludes.
    ///
    /// Answers the rewritten value, or `None` when this value is not an owning call
    /// that needs a home.
    pub(crate) fn materialise_owned_call(&mut self, val: &Value, tp: &Type) -> Option<Value> {
        if self.first_pass {
            return None;
        }
        // A struct LITERAL is already clean for the opposite reason: `parse_object`
        // allocates it into a work-ref this frame frees unconditionally.
        let Value::Call(d_nr, args) = val.unspan() else {
            return None;
        };
        let def = self.data.def(*d_nr);
        // `is_loft_defined`, not `is_struct_returning_call`'s `n_` test: a METHOD is
        // stored as `t_<LEN><Type>_<method>` and allocates exactly as a free function
        // does, so a name-shaped gate would leave `bx.make()` leaking while `mk()` was
        // cured — the same NAME-vs-SHAPE mistake loft#1017 was.
        if !def.is_loft_defined()
            // A callee answering a borrowed VIEW of its arguments allocated nothing,
            // and this frame may free none of what it points at.
            || def.returns_borrowed_view()
            // A callee that DELIVERS through the buffer has already claimed it; binding
            // it again here would free the one store twice.
            || self.answers_caller_buffer(*d_nr, args)
        {
            return None;
        }
        let buf = args.iter().find_map(|a| match a.unspan() {
            Value::Var(w) if self.vars.is_caller_hidden_buf(*w) => Some(*w),
            _ => None,
        })?;
        Some(v_block(
            vec![v_set(buf, val.clone()), Value::Var(buf)],
            tp.clone(),
            "join-arm-owner",
        ))
    }

    /// Materialise every owning CALL arm of a join whose merged type is a VIEW
    /// (loft#1019).
    ///
    /// `if`, an `else if` chain and all five `match` forms lower to the same tree of
    /// [`Value::If`], so one walk covers them — which is why this is a post-pass over
    /// the assembled value rather than a rewrite repeated at each of the six join
    /// sites.
    ///
    /// Gated on the MERGED type being a view: when every arm is owned the join is
    /// owned too, the consumer's own free claims the store, and a buffer here would
    /// free it a second time.
    pub(crate) fn own_joined_call_arms(&mut self, code: &mut Value, result_tp: &Type) {
        if self.first_pass
            || !matches!(result_tp.base(), Type::Reference(_, _))
            || result_tp.depend().is_empty()
        {
            return;
        }
        self.own_join_arm(code, result_tp);
    }

    /// One arm of [`Self::own_joined_call_arms`]: descend to the value the arm
    /// actually yields — through nested `if`s and to a block's TAIL — and rewrite it
    /// when it is an owning call.
    fn own_join_arm(&mut self, arm: &mut Value, tp: &Type) {
        match arm.unspan_mut() {
            Value::If(_, t, e) => {
                self.own_join_arm(t, tp);
                self.own_join_arm(e, tp);
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last_mut() {
                    self.own_join_arm(last, tp);
                }
            }
            other => {
                if let Some(owned) = self.materialise_owned_call(other, tp) {
                    *other = owned;
                }
            }
        }
    }

    /// Is `tp` a VALUE enum — an enum whose variants carry no payload, so a value of it
    /// is one discriminant BYTE rather than a reference?  True for the bare spelling and
    /// for `τ?` alike, which is the point: a nullable field or parameter arrives as
    /// `Optional(Enum)` and asks the same null question as the bare form.
    ///
    /// The shape is read from the enum DEFINITION's `returned` type, never from the
    /// access type, which is polluted — a field read of a value enum comes back
    /// `Enum(_, true, _)` even though the enum itself has no payload.  A synthetic
    /// `__nullable<S>` is excluded: it is @PLN25's nullable wrapper, whose absent
    /// discriminant is answered by its own path.
    pub(crate) fn is_value_enum(&self, tp: &Type) -> bool {
        matches!(tp.base(), Type::Enum(d, _, _)
            if *d != u32::MAX
                && matches!(self.data.def(*d).returned, Type::Enum(_, false, _))
                && !self.data.def(*d).name.starts_with("__nullable<"))
    }

    /// Is this operand's type still a TYPE VARIABLE — a template's `T` / `T?`, whose
    /// placeholder is an attribute-less struct rather than a real type (loft#1020)?
    fn is_type_var_operand(&self, tp: &Type) -> bool {
        matches!(tp.base(), Type::Reference(d, _) if self.data.is_type_var_placeholder(*d))
    }

    /// The lowering of `x == null` / `x != null` for an operand of CONCRETE type `tp`
    /// — the ONE place that answers *"what is `τ`'s null?"* for a comparison.
    ///
    /// Four types answer it with a dedicated test rather than an equality, each for its
    /// own reason:
    ///
    /// - a **collection** null is the store_nr sentinel, which `OpVectorIsNull` reads;
    ///   `eq_ref`'s `rec == 0` would also match an empty `[]` (loft#917 covers every
    ///   keyed kind, not just `vector`).
    /// - a **float/single** null is the NaN sentinel, and NaN compares unequal to
    ///   everything including itself, so equality can only ever answer false.  Test
    ///   VALIDITY instead — `convert(float, bool)` is `!is_nan` — and read it the right
    ///   way round.
    /// - an **enum** null is discriminant 0 for an inline `__nullable<S>` or a value
    ///   enum, and the store_nr sentinel for a struct-enum reference.
    /// - a nullable **struct reference** holds the store_nr sentinel, which
    ///   `OpRefIsNull` reads; the generic path compared `rec != 0` against the bool
    ///   sentinel and answered false for every input (@PLN99 A5).
    ///
    /// Anything else — an integer, a text, a boolean — answers `None`, and the caller
    /// falls through to the ordinary `==` dispatch against the typed null.
    ///
    /// It is a function of the type ALONE so a generic body can defer it: inside a
    /// template `T?` is `Optional(Reference(<type var>))`, whose placeholder is a
    /// STRUCT, so the reference branch used to win and the monomorph kept `OpRefIsNull`
    /// over an `integer` slot (loft#1020).  [`Parser::rewrite_generic_type_defaults`]
    /// re-asks it once `T` is real.  Answering it anywhere else would mint a sixth
    /// spelling of `τ`'s null, which is the defect loft#1014 was.
    /// *"is this collection ABSENT?"* — the ONE lowering both spellings of the question
    /// share: [`null_test`](Parser::null_test)'s `c == null` and
    /// [`coalesce_not_null`](Parser::coalesce_not_null)'s `c ?? d`.
    ///
    /// Enforces @FR-E-Coalesce for a collection subject — `e ?? d` yields `d` for exactly the
    /// null @FR-N-Index and @FR-Col-Lookup define — and it is one lowering because the two
    /// spellings ask one question: a `??` that discharged a null `== null` reports, or the
    /// reverse, would be two answers where the rules give one.
    ///
    /// Absence has two encodings, because a collection is reached two ways: a VALUE (a
    /// local, a parameter, a return, and what a missed lookup or an index past the end
    /// answers — `DbRef::or_null`) carries the store_nr sentinel; a SLOT (a field, a
    /// vector element) holds [`DbRef::ABSENT_REC`] in its four-byte word.
    /// `OpVectorIsNull` (`vector::is_absent_collection`) is the one test that reads both,
    /// and it also reads a reference whose HOLDER has no record — a field read through an
    /// unallocated buffer — as absent, since no slot is there to consult.
    ///
    /// `OpConvBoolFromRef`'s `rec != 0` reads only that last case, which is why the
    /// coalesce used to call every null collection FIELD present: a field read is a
    /// sub-reference whose `rec` is the HOLDER's record, so the default was unreachable
    /// and a `hash` / `index` then dereferenced the absent record (loft#1120).
    fn collection_is_null(&mut self, operand: &Value) -> Value {
        self.cl("OpVectorIsNull", std::slice::from_ref(operand))
    }

    pub(crate) fn null_test(&mut self, operand: Value, tp: &Type, negate: bool) -> Option<Value> {
        let is_null = if Self::is_collection_type(tp.base()) {
            // A heap-owning collection TEMP (a call result) consumed by the null test
            // would be orphaned — `OpVectorIsNull` reads the sentinel and discards the
            // store — so capture it in a work-ref for scopes.rs to free.  A `Var` already
            // owns its store, and an operand whose type declares a BORROW (a field read,
            // `optional(vector(…, deps { items: [0] }))`) is not ours to free at all:
            // freeing it took the holder's storage with it and the next read of the same
            // field dereferenced poison (loft#920).
            let borrows = !tp.depend().is_empty();
            let w = if matches!(operand.unspan(), Value::Var(_)) || borrows {
                u16::MAX
            } else {
                self.vars.work_refs(tp, &mut self.lexer)
            };
            if w == u16::MAX {
                self.collection_is_null(&operand)
            } else {
                self.vars.mark_inline_ref(w);
                let test = self.collection_is_null(&Value::Var(w));
                crate::data::v_block(
                    vec![crate::data::v_set(w, operand), test],
                    Type::Boolean,
                    "vec-null-workref",
                )
            }
        } else if matches!(tp.base(), Type::Float | Type::Single) {
            // `valid` is is-NON-null, so the two answers are the other way round here.
            let mut valid = operand;
            // A null TEST, not a store — @FR-N-Store admits the read.
            self.convert_admitting(&mut valid, tp, &Type::Boolean);
            return Some(if negate {
                valid
            } else {
                self.cl("OpNot", &[valid])
            });
        } else if self.is_value_enum(tp) {
            // A VALUE enum (`Color`, no payload variants) is a disc byte, and BOTH of
            // its absent bytes are asked for through the one predicate the coalesce and
            // the renderer already use.  Testing the discriminant against 0 alone read a
            // WRITTEN null (255) as a value; testing it against the typed null alone read
            // a zero-init one (0) as a value — and a field never set is the second kind,
            // so `h.c == null` answered `false` for a field that renders `null`.
            //
            // `τ?` is peeled here on purpose: it is the spelling a nullable field or a
            // nullable parameter arrives as, and leaving it to the generic `==` lowering
            // is what let the two encodings disagree.  Read the shape from the enum
            // DEFINITION's `returned` type — the ACCESS type is polluted, since a field
            // read of a simple enum comes back `Enum(_, true, _)`.
            let has_value = self.cl("OpConvBoolFromEnum", &[operand]);
            self.cl("OpNot", &[has_value])
        } else if let Type::Enum(e_def, _, _) = tp.base() {
            // loft#1065 — read through `base()`, so `Shape?` asks the same question
            // `Shape` does.  Matching the bare spelling alone left a nullable struct-enum
            // with no test at all: `s == null` was refused outright ("No matching
            // operator '==' on 'Shape?' and 'null'") while the same test on the bare type
            // beside it worked.  Whether a slot may be absent does not change how absence
            // is spelled in it.
            let e_def = *e_def;
            // A synthetic `__nullable<S>` enum backs an INLINE struct field / vector
            // element (no DbRef slot), so null is discriminant 0 — read it directly,
            // NEVER `OpRefIsNull`, whose store_nr test would deref the absent record.
            let inline = e_def != u32::MAX && self.data.def(e_def).name.starts_with("__nullable<");
            if inline {
                let get_enum = self.cl("OpGetEnum", &[operand, Value::Int(0)]);
                let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
                self.cl("OpEqInt", &[disc, Value::Int(0)])
            } else if matches!(tp.base(), Type::Enum(d, true, _)
                    if !self.data.def(*d).name.starts_with("__nullable<"))
                && matches!(operand.unspan(), Value::Var(_))
                && self.views_a_nullable_element_slot(tp)
            {
                // loft#1071 — a struct-enum bound FROM an inline element: the `for e in v`
                // loop variable over a `vector<Shape?>`.  It is a sub-reference to the
                // element SLOT, not a handle, so neither the store-pointer sentinel nor
                // its own record says anything — both read "present" for an absent
                // element.  The four-byte word AT that slot is where absence lives, the
                // same word a FIELD read tests below.
                //
                // The discriminator is what the binding VIEWS, followed through the dep
                // chain: the loop variable's own dep names ITSELF, and only its
                // declaration's dep names the vector. A plain local, parameter or return
                // reaches no collection that way and keeps the handle test.
                let word = self.cl("OpGetInt4", &[operand, Value::Int(0)]);
                self.cl("OpEqInt", &[word, Value::Int(0)])
            } else if let Some((base, fld)) = self.inline_slot_word(&operand) {
                // loft#1071 — an INLINE slot (a struct field) is a four-byte RECORD
                // POINTER, which cannot hold the twelve-byte store_nr sentinel at all.
                // There absent is pointer `0` — what the field prime writes and what a
                // `null` value now stores — and a PRESENT value always points at a real
                // record, so testing the pointer is exact rather than the ambiguity it
                // would be for a handle.
                //
                // The STORED WORD, not `OpGetField`'s result: reading the field yields a
                // sub-reference whose own `rec` is the HOLDER's record, so it is non-null
                // whatever the slot contains — which is why the read answered "present"
                // for an absent field even once the write was right.
                let word = self.cl("OpGetInt4", &[base, fld]);
                self.cl("OpEqInt", &[word, Value::Int(0)])
            } else {
                // A struct-enum reference in a HANDLE (a local, a parameter, a return):
                // null IS the store_nr sentinel, NOT `OpEqRef`'s `rec == 0` (a present
                // enum is inline on native).
                self.cl("OpRefIsNull", &[operand])
            }
        } else if matches!(tp, Type::Optional(_)) && matches!(tp.base(), Type::Reference(_, _)) {
            // Gate on `Optional`, NOT bare `Reference`: a bare `Reference` — a lookup
            // result, an element read trusted by contract — keeps the generic `OpEqRef`
            // dispatch, whose `rec` test is total over every value a read answers
            // (`nullref` has `rec == 0` too, `DbRef::or_null`) and over a reference whose
            // holder has no record, which `OpRefIsNull`'s store-number test is not.
            self.cl("OpRefIsNull", &[operand])
        } else {
            return None;
        };
        Some(if negate {
            self.cl("OpNot", &[is_null])
        } else {
            is_null
        })
    }

    /// loft#953 — does this call hand back a buffer the CALLER already owns?
    ///
    /// A heap-returning callee is given a hidden destination argument: the caller mints
    /// a `__ref_N` work-ref (`mark_caller_hidden_buf`, `parse_call`'s three heap arms),
    /// the callee fills it, and the value it returns IS that buffer.  The work-ref is a
    /// caller FUNCTION-scope variable with its own scope-exit `OpFreeRef`, so the store
    /// is already spoken for — nothing at the call site may claim it.
    ///
    /// Without this the free-source bit releases the caller's buffer once per call while
    /// the caller still holds it.  In a loop that is a use-after-free on every iteration
    /// after the first: the callee clears and refills a freed store, and once anything
    /// recycles the store number — the callee's own second vector local is enough — the
    /// two collide and rows written earlier read back empty or zeroed.  Silent on both
    /// backends; `LOFT_STRICT_STORES=1` is the witness.
    ///
    /// Same mechanism as the @P313 carve-out on the `Block "Object"` leg below, reached
    /// through a call instead of a literal: a source that carries its own scope free
    /// must not also be freed by the copy.  A callee with no hidden buffer allocates its
    /// own store and the caller has no free for it — that is the case the bit exists
    /// for, and it still gets it.
    ///
    /// # Why the return SHAPE is part of the question
    ///
    /// A buffer argument alone does not settle it, because the caller does not always
    /// copy from the call.  `scopes.rs`'s `lift_owned_return` interposes a `__lift_N`
    /// temp for a `Reference` / struct-`Enum` return, and that lift adopts the result
    /// only when it is owned — when the value aliases an argument it takes a private
    /// copy instead:
    ///
    /// ```text
    /// let _src = n_pick(cell, var_t, var_i, var___ref_1);
    /// if _src.store_nr == u16::MAX || _src.store_nr != var_t.store_nr { var___lift_1 = _src; }
    /// else { var___lift_1 = OpDatabase(…); OpCopyRecord(_src, var___lift_1, 79); }
    /// ```
    ///
    /// So by the time the copy runs, its source is a temp that OWNS its store, not the
    /// caller's buffer, and claiming it is right.  A COLLECTION return has no such lift
    /// — the value handed back is the buffer itself — which is why the cut is exactly
    /// there.  `tests/use_analysis.rs`'s `collect` fixture is the witness for the lifted
    /// side; loft#953's is the witness for the unlifted one.
    fn answers_caller_buffer(&self, fn_nr: u32, args: &[Value]) -> bool {
        if !crate::keys::retbuf_claim_guard_enabled() {
            return false;
        }
        // Only the shapes `lift_owned_return` does NOT lift — a collection return, whose
        // hidden buffer IS what the call answers.
        if !crate::parser::vectors::is_collection(self.data.def(fn_nr).returned().base()) {
            return false;
        }
        args.iter()
            .any(|a| matches!(a.unspan(), Value::Var(v) if self.vars.is_caller_hidden_buf(*v)))
    }

    /// Does `val` PRODUCE a whole record of its own, rather than READ a place that already
    /// holds one?
    ///
    /// The question a `&`-linked local's whole-value write has to ask (loft#1376).  A `&` at
    /// a struct projection is invisible in the IR — `pi = &o.j` and `pi = o.j` emit identical
    /// ops (@PLN130 F9) — so a place READ cannot be told apart from a re-point of the link,
    /// and it keeps the binding meaning it has.  A value that MINTS a record has no place
    /// behind it to link to, so a write of one through a link is unambiguously
    /// `@FR-B-Ref-Write`: copy it INTO the place the link names.
    ///
    /// An object literal (a `Block`, or the `Insert` spelling) and a call answer yes; a
    /// builtin `Op*` accessor — the field, element and keyed-element reads every link bind
    /// is built from — answers no, as does a bare variable.
    pub(crate) fn produces_whole_record(&self, val: &Value) -> bool {
        match val.unspan() {
            Value::Block(_) | Value::Insert(_) | Value::CallRef(_, _) => true,
            Value::Call(d, _) => !self.data.def(*d).name().starts_with("Op"),
            _ => false,
        }
    }

    pub(crate) fn copy_ref(&mut self, to: &Value, code: &Value, f_type: &Type) -> Value {
        // @FR-N-Store's fifth position (loft#1404).  `(N-Store)` names *"a field, a collection
        // element"* among the slots a bare `null` may not enter, and the four positions
        // loft#1313 wired all ask — the ASSIGNMENT TARGET asked only for a SCALAR target, so
        // the heap half passed in silence.
        //
        // It is asked HERE and not at the parse site, because `x = null` at a heap target
        // spells FIVE different things and only the lowering tells them apart.  Measured, all
        // six on both backends:
        //
        //   * `c[key] = null` on a keyed collection  → `OpHashRemove` — `(Col-Remove)`'s
        //     documented by-key delete;
        //   * `s.coll = null` on a collection field  → `OpClearVector` / `OpClearKeyed` — that
        //     field's clear, the same thing `s.coll = []` does (@P307);
        //   * `n.next = null` on a `reference<T>` POINTER field (#328's share marker) →
        //     `OpSetDbRef(…, OpNullRefSentinel())` — the store LANDS and the slot holds null;
        //   * `s.rec = null` on a dense record field, and `v[i] = null` on a vector element →
        //     `OpCopyRecord(null, …)`, which does nothing at all.
        //
        // Only the last reaches this function, so the ask needs no container test, no keyed
        // test and no pointer-marker test — the four shapes that are not stores never arrive.
        // Gating at the parse site instead needed all three and still got the pointer field
        // wrong, because the marker is not on the resolved target type by then
        // (`issues::issue_328_reference_field_pointer_semantics` is the cell that caught it).
        //
        // The CONSEQUENCE clause is this position's own: the shared default *"the slot holds
        // null"* is measured true for a scalar and for a record travelling as a HANDLE (a
        // `null` argument arrives null, a `return null` reads back null), and false here.  The
        // cure is unchanged and is the real one — a dense field has no discriminant to spend
        // on absence (`synth_nullable_struct_fields`), and the `?` is what creates the room.
        if matches!(code.unspan(), Value::Null) {
            self.nstore_null_report_as(
                f_type,
                "the assignment target",
                None,
                false,
                Some("the store does not happen, so the slot keeps the value it had"),
            );
        }
        let d_nr = self.data.type_def_nr(f_type);
        let tp = self.data.def(d_nr).known_type();
        // When the source is a struct-returning function CALL, set the high
        // bit (0x8000) on the type parameter to signal copy_record to free the
        // callee's temporary store after the deep copy.  Without this, the
        // __ref_N work-ref store the callee allocated to build its return value
        // leaks on every call in a loop.
        //
        // @P313: do NOT set 0x8000 for an inline struct-literal `Block "Object"`
        // source.  Unlike a call's unowned return temporary, the literal's
        // work-ref ALREADY carries its own scope `OpFreeRef`, so the free-source
        // bit double-frees it: the store is released while still owned, then
        // recycled by the next iteration's OpDatabase, corrupting the
        // nested-vector backings of every element written before it (silent
        // data loss on `vec[i] = Struct{…}`; use-after-free SIGSEGV under
        // churn).  Same shape as the @P311 OpSetKeyed fix, here on the
        // whole-value element-set path.
        #[cfg(not(feature = "wasm"))]
        let tp_val =
            if matches!(code.unspan(), Value::Call(_, _)) && self.is_struct_returning_call(code) {
                i32::from(tp) | 0x8000
            } else {
                i32::from(tp)
            };
        #[cfg(feature = "wasm")]
        let tp_val = i32::from(tp);
        self.cl(
            "OpCopyRecord",
            &[code.clone(), to.clone(), Value::Int(tp_val)],
        )
    }

    /** Mutate current code when it reads a value into writing it. This is needed for assignments.
     */
    pub(crate) fn compute_op_code(
        &mut self,
        op: &str,
        to: &Value,
        val: &Value,
        f_type: &Type,
    ) -> Value {
        if op == "=" {
            val.clone()
        } else if op == ">" {
            self.op("Lt", val.clone(), to.clone(), f_type.clone())
        } else if op == ">=" {
            self.op("Le", val.clone(), to.clone(), f_type.clone())
        } else {
            self.op(rename(op), to.clone(), val.clone(), f_type.clone())
        }
    }

    /// Dispatch an `OpGetX` getter name to the corresponding `OpSetX` setter call.
    pub(crate) fn call_to_set_op(
        &mut self,
        name: &str,
        args: &[Value],
        code: Value,
        _op: &str,
    ) -> Value {
        // @PLN130 F4 — every `c.field = …` on a scalar field arrives here as a getter name
        // plus (base, byte-offset). If that field is a KEY of a collection `c` views, the
        // write would re-key the record without re-indexing the collection, leaving the
        // element reachable by no key at all. Materialise the view instead and say so.
        if let (Some(Value::Var(base)), Some(Value::Int(off))) = (
            args.first().map(Value::unspan),
            args.get(1).map(Value::unspan),
        ) {
            self.note_key_field_write(*base, i64::from(*off));
        }
        match name {
            "OpGetInt" => {
                // f#next = pos: seek the file AND update the stored field.
                if args[1] == Value::Int(16)
                    && let Value::Var(v_nr) = &args[0]
                    && self.is_file_var(*v_nr)
                {
                    let seek = self.cl("OpSeekFile", &[args[0].clone(), code.clone()]);
                    let set = self.cl(
                        "OpSetInt",
                        &[args[0].clone(), args[1].clone(), code.clone()],
                    );
                    return Value::Insert(vec![seek, set]);
                }
                self.cl("OpSetInt", &[args[0].clone(), args[1].clone(), code])
            }
            "OpGetByte" => self.cl(
                "OpSetByte",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            // #334: the nullable byte pair (sentinel-translating twin).
            "OpGetByteNullable" => self.cl(
                "OpSetByteNullable",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            "OpGetEnum" => self.cl("OpSetEnum", &[args[0].clone(), args[1].clone(), code]),
            // @PLN17: byte-stored boolean write, storing the u8 form 0/1/255 directly.
            "OpGetBoolean" => self.cl("OpSetBoolean", &[args[0].clone(), args[1].clone(), code]),
            "OpGetShort" => self.cl(
                "OpSetShort",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            // not-null 2-byte field write — reuses the raw `(val - min)` store
            // (the read twin `OpGetShortFull` decodes without a sentinel).
            //
            // All three 2-byte readers share `OpSetShortRaw`'s `(val - min)` store; only
            // their sentinel decode differs.  `OpGetShortRaw` is the narrow-VECTOR
            // element reader, and its absence here is why `v[i] = x` on a
            // `vector<u16>`/`vector<i16>` failed to compile ("Cannot assign to attribute
            // on type 'OpGetShortRaw'") while the same write to a `u16` struct FIELD —
            // which reads through `OpGetShort`/`OpGetShortFull` — compiled fine.
            "OpGetShortFull" | "OpGetShortRaw" => self.cl(
                "OpSetShortRaw",
                &[args[0].clone(), args[1].clone(), args[2].clone(), code],
            ),
            "OpGetInt4" => self.cl("OpSetInt4", &[args[0].clone(), args[1].clone(), code]),
            // The unsigned 4-byte pair, one width up from the `OpGetShortRaw` arm above:
            // both readers share `OpSetInt4Raw`'s store and neither takes a `min`, so the
            // shape is `OpGetInt4`'s (ref, fld, val).  Omitting either here reproduces
            // exactly the defect that arm documents — the write side reports "Cannot
            // assign to attribute on type 'OpGetInt4Raw'" and `v[i] = x` stops compiling.
            "OpGetInt4Raw" | "OpGetInt4Full" => {
                self.cl("OpSetInt4Raw", &[args[0].clone(), args[1].clone(), code])
            }
            "OpGetFloat" => self.cl("OpSetFloat", &[args[0].clone(), args[1].clone(), code]),
            "OpGetSingle" => self.cl("OpSetSingle", &[args[0].clone(), args[1].clone(), code]),
            // Plan-22 phase 02d-iv — character / text cell writes
            // route through OpSetCharacter / OpSetText (3-arg
            // shape: ref, fld, val).  Mirrors the existing
            // OpGetInt → OpSetInt mapping above.  Required by
            // 02d-iii's auto-deref'd LHS pattern for boxed-
            // scalar locals + closure-body writes.
            "OpGetCharacter" => {
                self.cl("OpSetCharacter", &[args[0].clone(), args[1].clone(), code])
            }
            "OpGetText" => self.cl("OpSetText", &[args[0].clone(), args[1].clone(), code]),
            "OpGetField" => code,
            "n_get_store_lock" => {
                // d#lock = val — validation enforced in parse_assign before this call.
                self.cl("n_set_store_lock", &[args[0].clone(), code])
            }
            "OpSizeFile" => {
                // f#size = n: delegate to set_file_size which validates format and sign.
                let fn_nr = self.data.def_nr("t_4File_set_file_size");
                if fn_nr == u32::MAX {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "set_file_size is not defined");
                    }
                    Value::Null
                } else {
                    Value::Call(fn_nr, vec![args[0].clone(), code])
                }
            }
            _ => {
                if !self.first_pass {
                    // @PLN125 arc C — `x[i]` on a library type lowers to that type's
                    // `OpIndex`, so a WRITE lands here with the read's method name and no
                    // way to reach a setter.  Reading it back as an attribute assignment
                    // names an internal symbol the author never wrote; say what actually
                    // happened and what to write instead.  A writing counterpart is a
                    // separate decision (it needs its own method, and a decision about
                    // whether `x[i] += 1` may then read-modify-write), so this is a
                    // refusal, not a gap left silent.
                    if let Some(tp) = name.strip_suffix("_OpIndex").and_then(|n| {
                        n.strip_prefix("t_")
                            .map(|r| r.trim_start_matches(|c: char| c.is_ascii_digit()))
                    }) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "`{tp}` defines `OpIndex`, which READS — `x[i] = …` has nothing \
                             to write through; give the type a method that sets \
                             (`x.set(i, …)`)"
                        );
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Cannot assign to attribute on type '{name}'"
                        );
                    }
                }
                Value::Null
            }
        }
    }

    // @F37 — operator set (arithmetic/comparison/logical/bitwise/unary, precedence, **)
    pub(crate) fn parse_operators(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
    ) -> Type {
        let mut ls = Vec::new();
        if precedence >= OPERATORS.len() {
            // @PLN87 B-Ref-AnnotationOnly — the FIRST primary of a binding RHS (or of a
            // statement) consumes the head marker; every operand after it sees `false`.
            // Taken before the `&` test so a non-`&` head (`1` in `1 + &a`, the `(` in
            // `(&a)`, a call receiver) consumes it too — that is what makes the `&`
            // arriving later a sub-expression use rather than the whole RHS.
            let at_head = std::mem::take(&mut self.amp_head);
            // @PLN87 — a PREFIX `&<lvalue>` is a reference-to, distinct from the binary
            // `&` (bitwise-and "Land"). The binary form only ever sits BETWEEN operands,
            // so a `&` seen here at an operand START is the prefix: it flags the `&`-ness
            // via `amp_pending` so `parse_assign_op` can lower a scalar reference to
            // `OpCreateStack`. (`&&` is its own token, so this never mis-fires on
            // logical-and.)
            if self.lexer.has_token("&") {
                self.amp_pending = true;
                let t = self.parse_part(var_tp, code, parent_tp);
                // @PLN87 — `&` is a binding marker, NOT a general operator.  It is valid
                // ONLY as the WHOLE right-hand side of an assignment (`a = &b`), which —
                // since loft statements are `;`-terminated — is always followed by `;`
                // (a block-final reference `{ … &x }` by `}`).  `parse_part` has consumed
                // the whole lvalue, so the NEXT token classifies the use:
                //   `=`/`+=`/…  → `&` on the assignment TARGET (`&var = …`)   [error]
                //   not `;`/`}` → `&` as a sub-expression / argument          [error]
                //   `;`/`}`     → a valid binding RHS → check the operand is a PLACE (#1)
                // (Local peek; avoids the global `amp_pending` flag which leaks.)
                if !self.first_pass {
                    let next_assign = self.lexer.peek_token("=")
                        || self.lexer.peek_token("+=")
                        || self.lexer.peek_token("-=")
                        || self.lexer.peek_token("*=")
                        || self.lexer.peek_token("%=")
                        || self.lexer.peek_token("/=");
                    // What ENDS the operand depends on the position the head opened in.
                    // A statement ends at `;` (a block-final one at `}`); a struct-literal
                    // field value ends at the `,` before the next field just as well.
                    // `@FR-B-Ref-StoredRef` admits the `&` on the strength of the FIELD'S
                    // TYPE and conditions it on nothing else, so a `reference<τ>` field
                    // that is not the LAST field in the literal is the same legal position
                    // — `TrailNN { l: &pool[0], n: 4 }`.  Reading only `;`/`}` refused it,
                    // which made the rule's one admitted position depend on field order.
                    let next_terminates = self.lexer.peek_token(";")
                        || self.lexer.peek_token("}")
                        || (at_head == AmpHead::StoredRefField && self.lexer.peek_token(","));
                    // An invalid `&` must not stay "pending": clear the flag in each
                    // error branch so it cannot leak into `parse_assign_op`'s
                    // reference-lowering (1160) or the D-bind-7 bare-statement guard
                    // in `parse_assign` (which would then double-report).  Only the
                    // accepted case (terminates AND is a place) keeps `amp_pending`
                    // true, to be consumed by the binding that follows.
                    if next_assign {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "`&` cannot appear on the left of an assignment — it marks a \
                             binding as a link to its source at the binding site (`x = &src`), \
                             not an assignment target; drop the `&` (the binding is already linked)"
                        );
                        self.amp_pending = false;
                    } else if !next_terminates || at_head == AmpHead::No {
                        // `next_terminates` alone accepts the LAST operand of any
                        // expression (`b = 1 + &a;`, `b += &a;`, `S { x: &a }`, a
                        // block-final `{ 1 + &a }`) — it only proves nothing FOLLOWS
                        // the `&`, not that nothing PRECEDED it.  `at_head` supplies
                        // the other half, so the pair is total.
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "`&` is not a general operator — it binds a reference only as the \
                             whole right-hand side of an assignment (`a = &b`). Pass a `&` \
                             parameter WITHOUT `&` (`f(x)`, the reference comes from the \
                             parameter type); do not use `&` in an argument or sub-expression"
                        );
                        self.amp_pending = false;
                    } else if !Self::is_amp_place(code, &self.data) {
                        // #1 — a valid binding RHS still needs a PLACE operand.
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "`&` requires an addressable operand — a variable, struct field, \
                             or vector element — not a temporary (a literal, computed value, \
                             or call result)"
                        );
                        self.amp_pending = false;
                    }
                }
                return t;
            }
            let t = self.parse_part(var_tp, code, parent_tp);
            return t;
        }
        let orig_var = if let Value::Var(nr) = code {
            *nr
        } else {
            u16::MAX
        };
        // Start of the left operand — `known_var_or_type` below must point an
        // "Unknown variable" caret here, not at the cursor that has drifted to
        // the operator / statement terminator while the operand was parsed.
        let operand_pos = self.lexer.peek_pos().clone();
        let mut current_type = self.parse_operators(var_tp, code, parent_tp, precedence + 1);
        // @PLN102 pre-freeze — comparison operators are NON-ASSOCIATIVE.  A chain like
        // `a == b == c` (or `a < b < c`) parses as `(a == b) == c`, silently comparing a
        // BOOLEAN to the third operand — a classic footgun.  Reject the second comparison at
        // this level; the explicit `(a == b) == c` still works (its inner compare is a
        // separate parse inside the paren primary), as does `a < b && b < c`.
        let mut compared_at_this_level = false;
        loop {
            // a void left operand cannot have any binary operator
            // applied to it. Returning early prevents the pratt loop from
            // consuming a token that's actually the start of the *next*
            // statement — e.g. `if cond { return 0; }\n -1` where `-1` is
            // the function's tail expression, not `void - 1`.
            if matches!(current_type, Type::Void) {
                return current_type;
            }
            // Plan-07 phase 1, step 1.B.1 — capture the operator's source
            // position *before* `has_token` consumes it.  `op_pos` is then
            // threaded into `handle_operator` so the resulting Value::Span
            // points at the operator token (e.g. the `/`), not at whatever
            // the lexer drifted to while parsing the RHS.
            let op_pos = self.lexer.pos().clone();
            let mut operator = "";
            for op in OPERATORS[precedence] {
                if self.lexer.has_token(op) {
                    operator = op;
                    break;
                }
            }
            if operator.is_empty() {
                // `expr is VariantName` — variant check at comparison precedence.
                // Returns boolean: true if the enum value matches the named variant.
                if precedence == 3 && self.lexer.has_token("is") {
                    if let Some(variant_name) = self.lexer.has_identifier() {
                        current_type = self.parse_is_variant(code, &current_type, &variant_name);
                    } else if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after 'is'");
                    }
                    continue;
                }
                if !ls.is_empty() {
                    // Unwrap RefVar(Text) for text_return work buffer loop variables
                    let effective_type = if let Type::RefVar(inner) = &current_type {
                        if matches!(**inner, Type::Text(_)) {
                            *inner.clone()
                        } else {
                            current_type.clone()
                        }
                    } else {
                        // @PLN25 slice (c): peel `Optional` so a `text?` accumulator enters
                        // the concat path (the nullable-left propagate wrap fires below).
                        current_type.base().clone()
                    };
                    if matches!(effective_type, Type::Text(_) | Type::Character) {
                        if current_type == Type::Character {
                            // a Character variable cannot serve as an OpAppendText
                            // destination.  Prepend it to the parts list and use an empty
                            // text literal as the first operand so parse_append_text
                            // creates a fresh work text.
                            ls.insert(0, (code.clone(), Type::Character));
                            *code = Value::Text(String::new());
                            return self.parse_append_text(
                                code,
                                &Type::Text(crate::data::Deps::none()),
                                &ls,
                                u16::MAX,
                            );
                        }
                        // P223: `orig_var` was captured BEFORE the recursive
                        // `parse_operators` filled `code`.  When the LHS of an
                        // assignment is `s` (so `code` started as `Var(s)`) and
                        // the RHS is a literal-first concat like `"hello " + s`,
                        // `code` ends up as `Text("hello ")` after recursion —
                        // but `orig_var` still points at `s`.  Passing this
                        // stale `orig_var` makes `parse_append_text` use `s` as
                        // the accumulator and emit `OpAppendText(s, "hello ")`
                        // as the first op, which (combined with the
                        // `assign_text` self-reference clear) destroys `s`'s
                        // original content before the second append reads it.
                        // Fall back to a fresh work-text whenever `code`
                        // (unspanned) no longer is `Var(orig_var)`.
                        let effective_orig = if orig_var != u16::MAX
                            && !matches!(code.unspan(), Value::Var(v) if *v == orig_var)
                        {
                            u16::MAX
                        } else {
                            orig_var
                        };
                        // @PLN25 slice (c): nullable LEFT accumulator (`text? + …`) PROPAGATES —
                        // if the left is null the whole concat is null (people must be aware of
                        // nulls, not have them silently swept into "\0"/"null"). Build the concat
                        // into a FRESH work-text off the left, then wrap:
                        //   if is_not_null(left) { concat } else { null } : text?
                        // (proven equivalent to `if s==null {null} else {(s??"")+parts}`).
                        if matches!(&current_type, Type::Optional(inner) if matches!(**inner, Type::Text(_)))
                        {
                            let base_text = Type::Text(crate::data::Deps::none());
                            // The null arm must WRITE the null-text sentinel (`OpConvTextFromNull`),
                            // not a bare `Value::Null` — for a buffer-backed return the bare null
                            // leaves the return buffer empty and interp reads "" back instead of
                            // null (the normal `else { null }` flow converts it the same way).
                            let null_arm = |this: &mut Self| {
                                let mut n = Value::Null;
                                this.convert(&mut n, &Type::Null, &base_text);
                                n
                            };
                            if matches!(code.unspan(), Value::Var(_)) {
                                // simple var — reading twice is side-effect-free
                                let mut is_not_null = code.clone();
                                self.convert(&mut is_not_null, &base_text, &Type::Boolean);
                                let mut concat = code.clone();
                                // KEEP parse_append_text's return type — it carries the fresh
                                // work-text's `frame1` dep, without which the text-return
                                // conversion sees an empty work-var list and returns null.
                                let ct =
                                    self.parse_append_text(&mut concat, &base_text, &ls, u16::MAX);
                                let n = null_arm(self);
                                *code = v_if(is_not_null, concat, n);
                                return Type::optional(ct);
                            }
                            // non-trivial left — materialise into a temp to avoid double-eval
                            let tmp = self.create_unique("_nlhs", &current_type);
                            let set_tmp = v_set(tmp, code.clone());
                            let mut is_not_null = Value::Var(tmp);
                            self.convert(&mut is_not_null, &base_text, &Type::Boolean);
                            let mut concat = Value::Var(tmp);
                            let ct = self.parse_append_text(&mut concat, &base_text, &ls, u16::MAX);
                            let opt = Type::optional(ct);
                            let n = null_arm(self);
                            let if_expr = v_if(is_not_null, concat, n);
                            *code = v_block(vec![set_tmp, if_expr], opt.clone(), "nullable_concat");
                            return opt;
                        }
                        return self.parse_append_text(code, &current_type, &ls, effective_orig);
                    } else if matches!(current_type, Type::Vector(_, _)) {
                        return self.parse_append_vector(code, &current_type, &ls, orig_var);
                    } else if let Type::RefVar(inner) = &current_type
                        && matches!(**inner, Type::Vector(_, _))
                    {
                        return self.parse_append_vector(code, inner, &ls, orig_var);
                    } else if matches!(current_type.base(), Type::Vector(_, _)) {
                        // A NULLABLE vector reaches none of the arms above, and the
                        // fall-through returns the left operand — so `a + b` over two
                        // `vector<T>?`s answered `a`, silently, on both backends.
                        //
                        // Concatenating them is not the fix: the nullable TEXT form
                        // directly above propagates null (`null + "x"` is null), and a
                        // vector has no such lowering to be consistent with yet.  Refuse
                        // it, and name the discharge that already works, rather than
                        // inventing the answer here or keeping the wrong one.
                        // Unguarded by pass, like the vector arms above: the verdict reads
                        // only the resolved left type, so pass 1 can stay silent where the
                        // type is still `Unknown` but never contradict pass 2.  A pass-2-only
                        // diagnostic would go missing in any file that already aborts on
                        // pass 1, which is exactly where the expected-error suite lives.
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "cannot concatenate a nullable `{}` — discharge the `?` \
                             first (`(a ?? []) + (b ?? [])`), since a null operand has \
                             no defined result for a vector",
                            current_type.name(&self.data)
                        );
                        return current_type;
                    }
                }
                return current_type;
            }
            // @PLN102 — non-associative comparison guard (see `compared_at_this_level`).
            if matches!(operator, "==" | "!=" | "<" | "<=" | ">" | ">=") {
                if compared_at_this_level && self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "comparison operators do not chain — `{operator}` follows another \
                         comparison, which would compare a boolean to the next operand; \
                         parenthesise (e.g. `(a == b) == c`) or combine with `&&` \
                         (e.g. `a < b && b < c`)"
                    );
                }
                compared_at_this_level = true;
            }
            self.known_var_or_type(code, &operand_pos);
            // Detect '++': not a valid operator in loft. Consume the extra '+',
            // emit an error, and continue as if a single '+' was written.
            if operator == "+" && self.lexer.has_token("+") && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'++' is not a valid operator — use '+' for concatenation or addition"
                );
            }
            // Unwrap RefVar(Text) for the + text concatenation check
            let eff_type_for_plus = if let Type::RefVar(inner) = &current_type {
                if matches!(**inner, Type::Text(_)) {
                    *inner.clone()
                } else {
                    current_type.clone()
                }
            } else {
                // @PLN25 slice (c): peel `Optional` so a nullable LEFT operand of `+`
                // (`text?`) accumulates into the concat instead of falling to the generic
                // operator lookup (the "No matching operator '+' on 'text?'" reject).
                current_type.base().clone()
            };
            if operator == "+"
                && matches!(
                    eff_type_for_plus,
                    Type::Text(_) | Type::Character | Type::Vector(_, _)
                )
            {
                let mut second_code = Value::Null;
                let tp = self.parse_operators(var_tp, &mut second_code, parent_tp, precedence + 1);
                ls.push((second_code, tp));
            } else if let Some(value) = self.handle_operator(
                var_tp,
                code,
                parent_tp,
                precedence,
                &mut current_type,
                operator,
                &op_pos,
            ) {
                return value;
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    pub(crate) fn parse_part(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
    ) -> Type {
        let mut t = self.parse_single(var_tp, code, parent_tp);
        // --show-types --trace: log the type after the initial
        // `parse_single` (variable, literal, parenthesised expr).
        self.record_type_trace(&t);
        while self.lexer.peek_token(".")
            // `[` postfix-indexes a VALUE, and a `Void` subject produced none — so the
            // bracket opens the next expression instead.  `if` is the construct that
            // needs this said: it is an expression, so a bare `if c { … }` STATEMENT
            // reached this chain and swallowed the `[` of the line below it.  A whole
            // function's tail literal was read as an index on the `if`, and the
            // resulting "Indexing a non vector — keyed collections have no
            // generic-constructor expression" named a feature the program never used
            // (loft#910).  `for` / `while` never had it: they are statements and never
            // reach here.  An `if` that DOES yield a value keeps its index —
            // `if c { [1,2] } else { [3,4] }[0]` is unaffected, because its type is a
            // vector, not `Void`.
            || (self.lexer.peek_token("[") && !matches!(t, Type::Void))
            || (self.lexer.peek_token("(") && matches!(t, Type::Function(_, _, _)))
            || self.lexer.peek_token("?")
        {
            // @PLN116 — postfix default-fallback `x?`.  Handled first (a default-
            // fallback never faults, so it skips the `.`/`[]` span-wrapping below),
            // then re-enter the loop so a following `.`/`[]` chains onto the
            // discharged, now non-null value (`x?.field`).  The lexer's greedy
            // two-char match means `??` never reaches here as two `?` tokens.
            if self.lexer.has_token("?") {
                self.handle_default_fallback(var_tp, code, parent_tp, &mut t);
                self.record_type_trace(&t);
                continue;
            }
            // Plan-07 phase 1, steps 1.11 + 1.12 — capture the chaining
            // token's source position before `has_token` consumes it.
            // Wrapped when the iteration consumes a fault-prone access
            // (`.` field/method or `[` index — both can deref null or
            // out-of-bounds at runtime).  The `(` chained-call branch
            // is wrapped under step 1.13.
            let chain_pos = self.lexer.pos().clone();
            let mut wrap_chain = false;
            if !self.first_pass
                && t.is_unknown()
                && let Value::Var(nr) = code
            {
                // Name it, like every sibling site does (`objects.rs` reports `Unknown
                // variable 'x' — did you mean 'y'?`).  A bare "Unknown variable" leaves
                // the reader to find which of the line's names is the unresolved one
                // (loft#934).
                let name = self.vars.name(*nr).to_string();
                // loft#1008 — a METHOD (`t_<len><Type>_<name>`) has no definition under its
                // bare name, so naming one where a VALUE is wanted reported that the file's
                // own function does not exist. Say what it is instead.
                let receivers = self.method_receivers_named(&name);
                if receivers.is_empty() {
                    diagnostic!(self.lexer, Level::Error, "Unknown variable '{name}'");
                } else {
                    let on = receivers.join("`, `");
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`{name}` is a method on `{on}`, and a method is not a function VALUE — \
                         there is nothing to bind here. Wrap it: `|x| {{ x.{name}(…) }}`, or \
                         declare the function with a plain first-parameter name (not `self` / \
                         `both`), which makes it a free function and a usable fn-ref"
                    );
                }
            }
            if self.lexer.has_token(".") {
                wrap_chain = true;
                *parent_tp = t.clone();
                // T1.2: tuple element access — t.0, t.1, etc.
                if let Type::Tuple(ref elems) = t {
                    let elems = elems.clone();
                    if let Some(idx) = self.lexer.has_integer() {
                        let idx = idx as usize;
                        if idx >= elems.len() {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Tuple index {idx} out of range — tuple has {} elements",
                                elems.len()
                            );
                            t = Type::Unknown(0);
                        } else {
                            // P197: propagate parent tuple's deps into the
                            // element type so a returned text/reference
                            // carries the host's lifetime through.  Without
                            // this, `fn f() -> text { ...; a.v.0 }` returns
                            // a `Str` whose ptr points into a freed host.
                            let parent_deps = t.depend();
                            t = elems[idx].clone();
                            for on in parent_deps {
                                t = t.depending(on);
                            }
                            // T1.4: emit TupleGet IR for codegen.
                            // Plan-07 phase 1: unspan() so wraps on `.`
                            // (step 1.12) don't hide the underlying Var
                            // or Tuple shape of the parent expression.
                            let unspanned = code.unspan().clone();
                            if let Value::Var(var_nr) = unspanned {
                                *code = Value::TupleGet(var_nr, idx as u16);
                            } else if let Value::Tuple(elems_v) = unspanned {
                                // P197: code is already a literal tuple of
                                // per-element reads (e.g. produced by
                                // `get_val::Type::Tuple` for a tuple struct
                                // field).  Materialising the whole tuple
                                // into a `(String, String)` work var causes
                                // a native-codegen borrow lifetime error
                                // because the owned-`String` tuple temp
                                // dies before the returned `&str` is
                                // consumed.  Short-circuit: take the
                                // already-built element read directly.
                                if idx < elems_v.len() {
                                    *code = elems_v[idx].clone();
                                } else {
                                    *code = Value::Null;
                                }
                            } else {
                                // Temporary tuple — store in work var first.
                                let tmp_tp = Type::Tuple(elems.clone());
                                let w = self.vars.work_refs(&tmp_tp, &mut self.lexer);
                                if !self.first_pass {
                                    self.change_var_type(w, &tmp_tp);
                                }
                                let orig = code.clone();
                                *code = Value::TupleGet(w, idx as u16);
                                // Prepend Set(w, orig) in a block.
                                *code = crate::data::v_block(
                                    vec![
                                        crate::data::v_set(w, orig),
                                        Value::TupleGet(w, idx as u16),
                                    ],
                                    t.clone(),
                                    "tuple_tmp",
                                );
                            }
                        }
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Tuple element access requires a numeric index (e.g. .0, .1)"
                        );
                    }
                } else if let Type::RefVar(ref inner) = t
                    && let Type::Tuple(ref elems) = **inner
                {
                    // T1.5: element access through a reference-tuple parameter — pair.0, pair.1.
                    let elems = elems.clone();
                    self.parse_ref_tuple_elem(&mut t, code, &elems);
                } else if let Some(d_nr) = Self::record_tuple_def(&self.data, &t)
                    && matches!(self.lexer.peek().has, crate::lexer::LexItem::Integer(_, _))
                {
                    // P189b: vector-of-tuple loop var / index result —
                    // the loop variable is typed as `Reference(__tuple<…>)`
                    // pointing at inline tuple bytes inside the vector
                    // record.  `.0` / `.1` route through `get_val` so
                    // both interpreter (`OpGetInt(off)` / `OpGetText(off)`)
                    // and native codegen (per-type `stores.store(&db)`
                    // pattern) read at the right field offset.  Element
                    // types come from the synthetic struct's attributes.
                    let elems: Vec<Type> = self
                        .data
                        .def(d_nr)
                        .attributes
                        .iter()
                        .map(|a| a.typedef.clone())
                        .collect();
                    if let Some(idx) = self.lexer.has_integer() {
                        let idx = idx as usize;
                        if idx >= elems.len() {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Tuple index {idx} out of range — tuple has {} elements",
                                elems.len()
                            );
                            t = Type::Unknown(0);
                        } else {
                            // Stored-tuple field offset goes through the
                            // synthetic struct's post-finish layout — same
                            // offsets `OpGetInt` uses for an ordinary
                            // struct field.
                            let elem_offset = if let Some(v) =
                                crate::data::stored_tuple_offsets_for_def(
                                    &self.data,
                                    &self.database,
                                    d_nr,
                                    elems.len(),
                                ) {
                                u32::from(v[idx])
                            } else {
                                crate::data::element_stack_offsets(&elems)[idx] as u32
                            };
                            let elem_tp = elems[idx].clone();
                            // Carry the BASE's lifetime into the element, exactly as the
                            // plain-tuple site above (P197) and the struct-field read in
                            // `fields.rs` already do.  This was the one of the three that
                            // took the synthetic struct's attribute type VERBATIM, so a
                            // stored-tuple element bound off a BORROWED base came out with
                            // NO deps — typed as an owner while holding a handle into
                            // someone else's record, and handed an `OpFreeRef` to match.
                            // That is why a struct-typed element read as a COPY while its
                            // three sibling projections are views (`formal/binding.md`
                            // B-View: a struct-typed projection aliases without `&`).
                            let parent_deps = t.depend();
                            let base_var = match code.unspan() {
                                Value::Var(nr) => Some(*nr),
                                _ => None,
                            };
                            *code =
                                self.get_val(&elem_tp, false, elem_offset, code.clone(), u32::MAX);
                            t = elem_tp;
                            for on in parent_deps {
                                t = t.depending(on);
                            }
                            if let Some(nr) = base_var {
                                t = t.depending(nr);
                            }
                        }
                    }
                } else {
                    t = self.field(code, t);
                }
                // If the method returned an owned ref and more chaining follows, capture
                // it in a work-ref so scopes.rs emits OpFreeRef at end-of-scope.
                // Without this, the store allocated by the callee leaks and the LIFO
                // invariant in database::free() is violated.
                if !self.first_pass
                    && !matches!(code, Value::Var(_))
                    && (self.lexer.peek_token(".") || self.lexer.peek_token("["))
                    && let Type::Reference(d_nr, dep) = &t
                    && dep.is_empty()
                {
                    let d_nr = *d_nr;
                    let w = self.vars.work_refs(&t.clone(), &mut self.lexer);
                    // Mark as inline-ref temp so parse_code inserts its
                    // null-init after the first user statement, ensuring
                    // it appears after user-scope vars in var_order and is
                    // therefore freed before them (LIFO).
                    self.vars.mark_inline_ref(w);
                    let orig = code.clone();
                    // @PLN85 (the chained field-of-call class) — a PROJECTION
                    // of the value must COPY into `w`, not alias: argument
                    // lifting later splits the inner call into a `__lift_N`
                    // whose scope-exit free runs INSIDE this block, so an
                    // aliasing `Set(w, GetField(call, ..))` left `w` (and
                    // every further projection) dangling into the freed
                    // store — chain depth ≥ 2 read stale data at ANY
                    // consumption site (bind / argument / return / loop) on
                    // BOTH backends; the depth-1 `materialized_view_return`
                    // path was already copy-then-free and is the spec this
                    // mirrors (the C86 escape rule: a view outliving its
                    // dying temporary materialises).  A direct CALL result
                    // (not a projection) still binds raw — `w` genuinely
                    // adopts that fresh store.
                    let is_projection = matches!(orig.unspan(), Value::Call(pd, _)
                        if matches!(self.data.def(*pd).name(),
                            "OpGetField" | "OpGetVector" | "OpVectorRef" | "OpGetDbRef"));
                    let kt = self.data.def(d_nr).known_type();
                    if is_projection && kt != u16::MAX {
                        let copy_d = self.data.def_nr("OpCopyRecord");
                        *code = v_block(
                            vec![
                                v_set(w, Value::Null),
                                self.cl("OpDatabase", &[Value::Var(w), Value::Int(i32::from(kt))]),
                                Value::Call(
                                    copy_d,
                                    vec![orig, Value::Var(w), Value::Int(i32::from(kt))],
                                ),
                                Value::Var(w),
                            ],
                            Type::Reference(d_nr, crate::data::Deps::frame1(w)),
                            "inline ref copy",
                        );
                        // @PLN130 — parser-emitted materialisation of a projection into a
                        // store `w` owns; see `ParserMaterialise`.
                        crate::copy_manifest::record(
                            self.context,
                            w,
                            kt,
                            crate::copy_manifest::Origin::ParserMaterialise,
                        );
                    } else {
                        *code = v_block(
                            vec![v_set(w, orig), Value::Var(w)],
                            Type::Reference(d_nr, crate::data::Deps::frame1(w)),
                            "inline ref",
                        );
                    }
                    t = Type::Reference(d_nr, crate::data::Deps::frame1(w));
                }
            } else if self.lexer.has_token("[") {
                wrap_chain = true;
                // #246: record the indexed container's type as the parent,
                // mirroring the `.` field branch above.  Without this, a
                // trailing index (`vv[0]`, or `h.vv[0]` after a `.field`) leaves
                // `parent_tp` either null or stale-from-the-`.`, so a compound
                // append to a vector element can't tell it is appending to a
                // VECTOR (not a struct field).
                *parent_tp = t.clone();
                t = self.parse_index(code, &t);
                self.lexer.token("]");
            } else if self.lexer.has_token("(") {
                // chained call on a Type::Function expression — expr(args).
                if let Type::Function(param_types, ret_type, fn_deps) = t.clone() {
                    // @PLN85 t1 — does the SOURCE fn-ref OWN a freshly-minted
                    // closure?  A closure-factory return (`make_greeter(..)`) carries
                    // a `CalleeFrame` dep (the lambda's `___clos_N` work var); a
                    // borrowed pass-through (`passthru(g)`) or a non-capturing lambda
                    // returns EMPTY deps.  Owned ⇒ this immediately-invoked temp is
                    // the SOLE owner of the freshly-minted closure store (nobody else
                    // frees it), so it must KEEP the ownership dep and be freed at
                    // scope exit; borrowed ⇒ skip_free (the source var frees it, else
                    // a double-free).
                    // Does the SOURCE fn-ref OWN a freshly-minted closure?  A
                    // closure-factory return (`make_greeter(..)`) carries a
                    // `CalleeFrame` dep (the lambda's `___clos_N` work var); a
                    // borrowed pass-through (`passthru(g)`) or a non-capturing lambda
                    // returns EMPTY deps.  Pass 1's type does NOT yet carry the dep
                    // (the lambda propagation runs later), so this reads FALSE there —
                    // fine, because pass-1 IR is discarded and the stamp below is
                    // pass-2-only.
                    let owns_closure = fn_deps
                        .entries()
                        .any(|e| matches!(e, crate::data::DepEntry::CalleeFrame(_)));
                    let fn_type = Type::Function(
                        param_types.clone(),
                        ret_type.clone(),
                        if owns_closure {
                            fn_deps.clone()
                        } else {
                            crate::data::Deps::none()
                        },
                    );
                    // Allocate temp variable on BOTH passes (consistent unique counter).
                    let fn_work = self.create_unique("__fn_ref_tmp", &fn_type);
                    self.vars.defined(fn_work);
                    // @PLN85 t1 — stamp `skip_free` ONLY in pass 2 and ONLY for a
                    // BORROWED source.  `skip_free` is a GLOBAL per-var bit that
                    // persists across passes; a pass-1 stamp (where `owns_closure`
                    // reads FALSE for a fresh closure factory, its dep not yet
                    // propagated) poisons the pass-2 role — the counter-coupling
                    // hazard, cf. t3/p179.  An OWNED source (`make_greeter(..)(..)`)
                    // makes this temp the SOLE owner of the freshly-minted closure
                    // store, so it must be freed at scope exit (no stamp); a BORROWED
                    // source's closure DbRef aliases the source's store, so stamping
                    // avoids a double-free (and blocks the insert_free Return-wrap
                    // that would return `()` from a value block, P249-mirror).
                    if !self.first_pass && !owns_closure {
                        self.vars.set_skip_free(fn_work);
                    }
                    // P227 — the fn-ref call through a FIELD, tuple member or other
                    // expression.  Like its two siblings in `control.rs` it pushes the
                    // `&text` work buffers the widest candidate of this signature could
                    // want, because a `&text` points into the CALLER's frame and only the
                    // caller can supply one (loft#1116).  Every site that builds a
                    // text-returning `CallRef` must read the SAME count: `fn_call_ref`
                    // trims the frame against it, so a site one buffer short has a real
                    // buffer trimmed away and the callee reads `__closure` from the wrong
                    // offset.
                    let work_vars =
                        self.fnref_text_buffer_vars(param_types.len(), ret_type.as_ref());
                    // Parse arguments (both passes).
                    let mut list: Vec<Value> = Vec::new();
                    let mut types: Vec<Type> = Vec::new();
                    let mut first = true;
                    while !self.lexer.peek_token(")") && !self.lexer.peek_token("") {
                        if !first {
                            self.lexer.token(",");
                        }
                        first = false;
                        let mut arg_val = Value::Null;
                        let arg_tp = self.expression(&mut arg_val);
                        list.push(arg_val);
                        types.push(arg_tp);
                    }
                    self.lexer.token(")");
                    if !self.first_pass {
                        let mut converted = list;
                        for (i, expected) in param_types.iter().enumerate() {
                            if i < converted.len() {
                                self.convert(&mut converted[i], &types[i], expected);
                            }
                        }
                        self.push_fnref_text_buffers(&mut converted, &work_vars);
                        let orig = std::mem::replace(code, Value::Null);
                        *code = v_block(
                            vec![v_set(fn_work, orig), Value::CallRef(fn_work, converted)],
                            *ret_type.clone(),
                            "fn_call_tmp",
                        );
                    }
                    t = *ret_type;
                }
            }
            // Plan-07 phase 1, steps 1.11 + 1.12 — wrap the chained
            // expression in a Span at the access-token position so
            // runtime null-deref / out-of-bounds errors can be
            // reported with the source location of the offending `.`
            // or `[` token.  Skip when the result is an `Iter` (range
            // subscript like `v[0..5]` produces an iterator that
            // parse_for / iterator() must pattern-match without
            // a Span wrapper) — the access token doesn't fault for
            // a range-subscript shape, only the eventual element
            // reads do, and those go through the normal `[idx]`
            // path inside the rewritten loop.
            if wrap_chain && !self.first_pass && !matches!(code, Value::Iter(_, _, _, _)) {
                let inner = std::mem::replace(code, Value::Null);
                *code = Value::with_span(chain_pos, inner);
            }
            // --show-types --trace: log the resulting type after
            // each chaining step (`.field`, `.tuple_idx`, `[idx]`,
            // `(args)`).  Combined with the post-`parse_single`
            // log at the top, this produces a per-step "tape" of
            // how the type evolves through a chained expression
            // like `a.v.0` — the shape that hid the P197 dep loss.
            self.record_type_trace(&t);
        }
        t
    }

    /// T1.5: parse a `.N` element index on a `&(T1, T2, ...)` reference-tuple.
    /// Updates `t` to the element type and rewrites `code` to `TupleGet(var, idx)`
    /// when `code` is a plain variable reference.
    /// The synthesized `__tuple<…>` struct a record-backed tuple receiver names — a plain
    /// `Reference(__tuple<…>)` (a loop variable, a local bound from a heap-tuple return) or the
    /// same behind the `&` a `&(…)` with a heap element denotes (tuples.md T-Ref).  Both read
    /// and write their elements as that struct's fields.
    fn record_tuple_def(data: &crate::data::Data, t: &Type) -> Option<u32> {
        let d = match t {
            Type::Reference(d, _) => *d,
            Type::RefVar(inner) => match **inner {
                Type::Reference(d, _) => d,
                _ => return None,
            },
            _ => return None,
        };
        data.def(d).name().starts_with("__tuple<").then_some(d)
    }

    fn parse_ref_tuple_elem(&mut self, t: &mut Type, code: &mut Value, elems: &[Type]) {
        if let Some(idx) = self.lexer.has_integer() {
            let idx = idx as usize;
            if idx >= elems.len() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Tuple index {idx} out of range — tuple has {} elements",
                    elems.len()
                );
                *t = Type::Unknown(0);
            } else {
                *t = elems[idx].clone();
                if let Value::Var(var_nr) = code {
                    *code = Value::TupleGet(*var_nr, idx as u16);
                }
            }
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Tuple element access requires a numeric index (e.g. .0, .1)"
            );
        }
    }

    /// C54.G-hybrid helper: if `code` is a direct call to an arithmetic
    /// opcode (`OpAddInt` / `OpMulInt` / etc. or their `*Long` siblings),
    /// swap its def number to the Nullable variant (`OpAddIntNullable` /
    /// `OpMulIntNullable` / …).  Only the outermost call is rewritten;
    /// nested sub-expressions keep trap semantics.  No-op if the Nullable
    /// variant isn't registered (defensive — should always be, via
    /// `default/01_code.loft`).
    pub(crate) fn rewrite_outer_arith_to_nullable(code: &mut Value, data: &crate::data::Data) {
        // Helper: try to swap the call's def_nr to its Nullable peer.
        // Returns `true` if it found and applied a swap.
        fn try_swap(def_nr: &mut u32, data: &crate::data::Data) -> bool {
            let name = data.def(*def_nr).original_name();
            let nullable_name = match name.as_str() {
                "AddInt" => "OpAddIntNullable",
                "MinInt" => "OpMinIntNullable",
                "MulInt" => "OpMulIntNullable",
                "DivInt" => "OpDivIntNullable",
                "RemInt" => "OpRemIntNullable",
                // Plan-07 phase 4d — vector / text indexing followed by `??`.
                // Mirrors the C54.G-hybrid pattern: when codegen detects
                // `??` immediately after a fault-prone op, swap to the
                // Nullable peer so the op returns its sentinel silently
                // (no log, no halt) and `??` discharges it.
                "GetVector" => "OpGetVectorNullable",
                "VectorRef" => "OpVectorRefNullable",
                "TextCharacter" => "OpTextCharacterNullable",
                // Plan-07 phase 4f.5 — float / single div / mod by zero
                // peers.  Same defense-dispatch contract as integer.
                "DivFloat" => "OpDivFloatNullable",
                "RemFloat" => "OpRemFloatNullable",
                "DivSingle" => "OpDivSingleNullable",
                "RemSingle" => "OpRemSingleNullable",
                _ => return false,
            };
            let new_nr = data.def_nr(nullable_name);
            if new_nr == u32::MAX {
                false
            } else {
                *def_nr = new_nr;
                true
            }
        }
        let Value::Call(def_nr, args) = code.unspan_mut() else {
            return;
        };
        // First try the outer call.  Direct hits cover OpDivInt /
        // OpRemInt / OpVectorRef / OpTextCharacter / arithmetic.
        if try_swap(def_nr, data) {
            return;
        }
        // Wrapped case: typed-vector indexing emits a type-specific
        // element getter wrapping the inner `OpGetVector` —
        // `OpGetInt(OpGetVector(v, size, idx), 0)` for `vector<integer>`
        // and analogues for every other base element type.  The outer
        // getter is null-tolerant (returns the type's null sentinel on a
        // `rec == 0` DbRef per the Store-accessor guards); the inner
        // `OpGetVector` is the one that raises.  Recurse into the first arg
        // to swap the inner op.  @P356: this list previously covered only
        // the integer wrappers, so `tv[i] ?? fb` over a non-integer vector
        // (text / float / single / enum / char / nested collection / ref)
        // kept the RAISING `OpGetVector` and still halted on OOB.
        let outer_name = data.def(*def_nr).original_name();
        if matches!(
            outer_name.as_str(),
            "GetInt"
                | "GetInt4"
                | "GetByte"
                | "GetShortRaw"
                | "GetShort"
                | "GetText"
                | "GetSingle"
                | "GetFloat"
                | "GetEnum"
                // @PLN17: byte-stored boolean field/element read (single-level
                // wrap of OpGetVector), so the inner OpGetVector swaps to its
                // mutable form for `v[i] = bool` — same as GetEnum.
                | "GetBoolean"
                | "GetCharacter"
                | "GetField"
                | "GetDbRef"
        ) && let Some(first_arg) = args.first_mut()
            && let Value::Call(inner_nr, _) = first_arg.unspan_mut()
        {
            try_swap(inner_nr, data);
        } else if outer_name == "EqInt"
            // `vector<boolean>` is `OpEqInt(OpGetByte(OpGetVector(...)), 1)`
            // — a TWO-level wrap, so descend through the `OpGetByte` to reach
            // the inner `OpGetVector`.
            && let Some(first_arg) = args.first_mut()
            && let Value::Call(byte_nr, byte_args) = first_arg.unspan_mut()
            && data.def(*byte_nr).original_name() == "GetByte"
            && let Some(gv) = byte_args.first_mut()
            && let Value::Call(inner_nr, _) = gv.unspan_mut()
        {
            try_swap(inner_nr, data);
        }
    }

    /// Plan-07 phase 4e.1 — recursive variant of
    /// [`Self::rewrite_outer_arith_to_nullable`].  Walks the whole
    /// `Value` tree and swaps every fault-prone call to its Nullable
    /// peer.  Used in format-string contexts (`"{expr}"` interpolation)
    /// where the interpolated expression may contain arbitrarily nested
    /// fault-prone ops (`"{a + v[i] / b}"`).  Per C66 +
    /// `DESIGN_DECISIONS.md` 2026-05-11: format strings are the user's
    /// observability surface and must NEVER halt, log, or warn — so
    /// every interpolated fault site routes through its silent peer.
    ///
    /// Phase 4e.3 — also returns the OUTERMOST fault kind id (as
    /// recognised by the swap table) so the caller (the format-
    /// string emitter in `parser/objects.rs::parse_format`) can
    /// prepend an `OpTagFault(kind_id)` sibling statement.  When
    /// the next format-conversion op (`OpFormatInt` /
    /// `OpAppendCharacter`) sees the type's null sentinel AND the
    /// tag is set, it renders `null(<reason>)` instead of bare
    /// `null`.  Returns `None` for non-fault outer calls (the
    /// expression may still contain inner faults that get swapped,
    /// but only the outermost one tags — inner faults have no
    /// renderer to feed the tag to).
    pub(crate) fn rewrite_subtree_to_nullable_kind(
        code: &mut Value,
        data: &crate::data::Data,
    ) -> Option<u8> {
        // Determine the kind BEFORE the swap.  For integer-vector
        // indexing the IR shape is `OpGetInt(OpGetVector(v, 4, i), 0)`
        // — the outer is `GetInt`, the fault-prone op is the inner
        // `GetVector`.  Mirror the recurse-one-level case from
        // `rewrite_outer_arith_to_nullable` so the kind id matches
        // the inner op when wrapped.
        fn classify(name: &str) -> Option<u8> {
            match name {
                "DivInt" | "DivFloat" | "DivSingle" => Some(1),
                "RemInt" | "RemFloat" | "RemSingle" => Some(2),
                "GetVector" | "VectorRef" | "TextCharacter" => Some(3),
                _ => None,
            }
        }
        let outer_kind = match code.unspan() {
            Value::Call(def_nr, args) => {
                let outer_name = data.def(*def_nr).original_name();
                let direct = classify(&outer_name);
                if direct.is_some() {
                    direct
                } else if matches!(
                    outer_name.as_str(),
                    "GetInt" | "GetInt4" | "GetByte" | "GetShortRaw"
                ) && let Some(first) = args.first()
                    && let Value::Call(inner_nr, _) = first.unspan()
                {
                    classify(&data.def(*inner_nr).original_name())
                } else {
                    None
                }
            }
            _ => None,
        };
        Self::rewrite_subtree_to_nullable(code, data);
        outer_kind
    }

    pub(crate) fn rewrite_subtree_to_nullable(code: &mut Value, data: &crate::data::Data) {
        // Helper mirrors the swap table in
        // `rewrite_outer_arith_to_nullable`; kept inline (no shared
        // helper) so the dispatch table stays grep-discoverable from
        // both swap sites.
        fn try_swap(def_nr: &mut u32, data: &crate::data::Data) -> bool {
            let name = data.def(*def_nr).original_name();
            let nullable_name = match name.as_str() {
                "AddInt" => "OpAddIntNullable",
                "MinInt" => "OpMinIntNullable",
                "MulInt" => "OpMulIntNullable",
                "DivInt" => "OpDivIntNullable",
                "RemInt" => "OpRemIntNullable",
                "GetVector" => "OpGetVectorNullable",
                "VectorRef" => "OpVectorRefNullable",
                "TextCharacter" => "OpTextCharacterNullable",
                // Plan-07 phase 4f.5 — float / single div / mod peers.
                "DivFloat" => "OpDivFloatNullable",
                "RemFloat" => "OpRemFloatNullable",
                "DivSingle" => "OpDivSingleNullable",
                "RemSingle" => "OpRemSingleNullable",
                _ => return false,
            };
            let new_nr = data.def_nr(nullable_name);
            if new_nr == u32::MAX {
                false
            } else {
                *def_nr = new_nr;
                true
            }
        }
        match code.unspan_mut() {
            Value::Call(def_nr, args) => {
                try_swap(def_nr, data);
                for arg in args {
                    Self::rewrite_subtree_to_nullable(arg, data);
                }
                // Phase 4e.3 (slice 2 — deferred): the design adds a
                // sibling `OpTagFault(kind)` immediately before each
                // swapped Nullable peer in format-string scope so the
                // format-conversion op renders `null(<reason>)`
                // instead of bare `null`.  The runtime infrastructure
                // (`Stores::set_format_fault` /
                // `Stores::take_format_fault` / `format_fault_tag`
                // field / `OpTagFault` opcode) is in place; the
                // Block-wrapping insertion proved fragile when run
                // through the format-string emitter (the Block's
                // result type isn't filled in time for `append_data`
                // to pick the right `OpAppend*` op, producing wrong-
                // type bytecode and SIGSEGV).  Wiring slice 2 needs
                // a different emit shape — likely new dedicated
                // `Op*Fmt` peers (one per fault kind) that fold the
                // tag into the Nullable peer's body instead of
                // sequencing.  Tracked in plan-07 phase 4e.3 row.
            }
            Value::CallRef(_, args) => {
                for arg in args {
                    Self::rewrite_subtree_to_nullable(arg, data);
                }
            }
            Value::Block(b) => {
                for child in &mut b.operators {
                    Self::rewrite_subtree_to_nullable(child, data);
                }
            }
            Value::Loop(b) => {
                for child in &mut b.operators {
                    Self::rewrite_subtree_to_nullable(child, data);
                }
            }
            Value::If(cond, then_b, else_b) => {
                Self::rewrite_subtree_to_nullable(cond, data);
                Self::rewrite_subtree_to_nullable(then_b, data);
                Self::rewrite_subtree_to_nullable(else_b, data);
            }
            Value::Set(_, src)
            | Value::Return(src)
            | Value::Drop(src)
            | Value::Yield(src)
            | Value::TuplePut(_, _, src) => {
                Self::rewrite_subtree_to_nullable(src, data);
            }
            Value::Iter(_, init, step, body) => {
                Self::rewrite_subtree_to_nullable(init, data);
                Self::rewrite_subtree_to_nullable(step, data);
                Self::rewrite_subtree_to_nullable(body, data);
            }
            Value::Tuple(items) | Value::Insert(items) | Value::Parallel(items) => {
                for child in items {
                    Self::rewrite_subtree_to_nullable(child, data);
                }
            }
            // Other Value variants (Int / Text / Var / FnRef / Keys / etc.)
            // carry no nested fault-prone calls to rewrite.  Spans are
            // stripped by `unspan_mut` above.
            _ => {}
        }
    }

    /// @PLN102 case-B flip grandfather — is `code` a call to a fn DECLARED nullable
    /// (`-> τ?`)?  The domain lattice may narrow such a call's result to non-null at a
    /// specific site (`sqrt(a*a + b*b)`), but a `??` / `== null` guarding it is still a
    /// real defense of a declared-nullable op, never "redundant".  Distinct from a bare
    /// non-null field (`s.nn ?? d`), which IS redundant and still warns.
    fn call_declares_nullable(&self, code: &Value) -> bool {
        matches!(code.unspan(), Value::Call(d, _)
            if matches!(self.data.def(*d).returned(), Type::Optional(_)))
    }

    /// Desugar `lhs ?? ...` — both the plain-default form and the
    /// `?? return ret_expr` early-return form.  Lifted out of
    /// [`Self::handle_operator`] so each shape has its own focused helper.
    ///
    /// `?? default`         → `if lhs_nonnull { lhs } else { default }`,
    ///                        with a temp for non-trivial lhs.
    ///
    /// `?? return ret_expr` → `{ tmp = lhs; if is_null(tmp) { return ret_expr }; tmp }`.
    fn handle_null_coalesce(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
        ctp: &mut Type,
        op_pos: &Position,
    ) {
        // A tagged slot's value is read through its tag before anything else asks about it:
        // the subject is a pointer from here on (`@FR-L-Null-Which`), so the null test, the
        // default's hint and the result type all see `S?`, never the slot's synthetic.
        self.read_through_tag(code, ctp);
        // Redundant-coalesce warning — fire ONLY when the LHS type is genuinely
        // non-null.  `expr_not_null` tracks the last-read name's not-null-ness but
        // does NOT account for a fault op (`sqrt`, `/`, `ln`, …) nulling a non-null
        // input — `sqrt(non_null_negative)` IS null — so it can be stale-true over an
        // `Optional`-typed result.  The type is the authority: an `Optional` LHS means
        // the `??` is genuinely needed, so gate on it (else the lint contradicts the
        // type system, which correctly REQUIRES the discharge — @PLN102 case E).
        // Grandfather (case-B flip): a call to a fn DECLARED `-> τ?` whose result the
        // domain lattice just narrowed to non-null (`sqrt(a*a+b*b) ?? d`) is a real
        // defense, not redundant — never warn on a declared-nullable-returning call.
        // loft#1003 — `(diagnostic index, line, column)` of a `redundant-coalesce` notice
        // whose deletion span is still open, or `None` when none fired.
        let mut redundant_at: Option<(usize, u32, u32)> = None;
        if self.expr_not_null
            && !self.first_pass
            && !matches!(ctp, Type::Optional(_))
            && !self.call_declares_nullable(code)
        {
            diagnostic!(
                self.lexer,
                Level::Warning,
                code = "redundant-coalesce",
                "Redundant null coalescing — '{}' is 'not null', default is never used",
                self.expr_not_null_name,
            );
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Mechanical,
                title: "delete the `?? <default>`".to_string(),
                condition: None,
                // Spelled below, once the default has been parsed and the span has an end
                // (loft#1003).  The notice has to fire HERE — it is about the `??` — but
                // the deletion runs through a default that does not exist yet.
                edit: None,
                concept: "null coalescing",
                concept_ref: "@F2",
            });
            // `op_pos` is the `??`'s own position — the lexer's cursor is no help here,
            // it has already scanned past the default (loft#1003).
            // `op_pos` is the cursor just PAST the `??`, so the token itself starts two
            // columns back.  The lexer's own cursor is no help — it has already scanned
            // beyond the default (loft#1003).
            redundant_at = self
                .lexer
                .last_diagnostic_index()
                .filter(|_| op_pos.pos >= 3)
                .map(|i| (i, op_pos.line, op_pos.pos - 2));
        }
        self.expr_not_null = false;
        // Plan-07 phase 4h — if the `??` LHS is the just-emitted
        // field read site (set by `Parser::field()`), mark the
        // (struct, field) as defended so the not-null hint won't
        // fire on it.  Conservative: covers `p.field ?? default`;
        // complex expressions like `(p.field + 1) ?? 0` and
        // `if p.field != null` are slice-2 work.
        if let Some(key) = self.last_field_read_site.take() {
            self.defended_field_reads.insert(key);
        }

        // C54.G-hybrid: if the LHS is an immediate arithmetic call
        // (`a + b` / `a - b` / etc.), swap it to the Nullable variant so
        // overflow / div-zero produce `i32::MIN` instead of trapping — the
        // `??` below then discharges the null to the RHS.  Only the
        // outermost op gets swapped; nested sub-expressions still trap.
        if !self.first_pass {
            Self::rewrite_outer_arith_to_nullable(code, &self.data);
        }

        // @PLN25 slice (b): a `??` discharges null, so its RESULT is the non-null base. Peel
        // any `Optional` from the LHS type here so the null-check builder + the result type
        // see the base (e.g. `Optional<text>` routes exactly as plain `text` did pre-marker).
        // loft#1372 — through a `&` LINK first.  `@FR-C-Ref`: a `&τ` reads through to its
        // referent, so a `??` on a linked slot is a read of the SLOT and its result is the
        // slot's non-null base.  `base()` peels only `Optional`, so for a `&τ?` it was the
        // identity: the coalesce typed its own result `&integer?`, against which EVERY
        // default is a mismatch, and `p ?? 0` inside a `&integer?` parameter was reported as
        // the author's error.  That refusal is half of why @FR-B-Ref-Intro's `&τ` for every
        // τ had to be declined (D-bind-17).
        *ctp = match &*ctp {
            Type::RefVar(inner) => inner.base().clone(),
            other => other.base().clone(),
        };
        let lhs_type = ctp.clone();
        // @PLN17: boolean now has a real null sentinel (255), so `??` works — the
        // null-check for a boolean LHS is `lhs == null` (raw `== 255`), NOT the
        // value's truthiness (see the Boolean arm in `null_check_builder` below).
        // `false ?? x` stays `false` (false is not null); `null ?? x` → x.
        if self.lexer.has_token("return") {
            self.build_null_coalesce_return(code, ctp, &lhs_type);
        } else {
            self.build_null_coalesce_default(var_tp, code, parent_tp, precedence, ctp, &lhs_type);
        }
        // loft#1003 — the default is parsed, so the deletion now has an end.  Spelling it
        // here is what lets `loft fix` see this fix at all: it reports only fixes carrying
        // an `edit`, which is why it could act on no warning-level fix in the whole index.
        //
        // Only when the whole `?? <default>` sits on ONE line — an `Edit` is a single-line
        // span, and a fix that cannot be placed must not be spelled (the same rule the
        // checked-cast edit above follows).
        if let Some((idx, line, start_col)) = redundant_at
            && let Some((end_line, end_col)) = self.ncc_default_end.take()
            && end_line == line
            && end_col > start_col
        {
            {
                self.lexer.set_fix_edit(
                    idx,
                    0,
                    crate::diagnostics::Edit {
                        line,
                        col: start_col,
                        len: end_col - start_col,
                        text: String::new(),
                    },
                );
            }
        }
        self.ncc_default_end = None;
    }

    /// *"`src` is NOT null"* for a `??` whose subject has type `tp` — the ONE place
    /// that answers it for the coalesce, as [`null_test`](Parser::null_test) does for
    /// the `== null` comparison.
    ///
    /// Which test that is depends on the type, and every arm below exists because a
    /// different representation carries null differently: a tuple in its first field,
    /// a synthetic `__nullable<S>` in its discriminant, the whole heap-DbRef family in
    /// `rec`, a boolean in the `255` byte, and every remaining scalar through its
    /// registered `OpConv*FromX → Boolean`.
    ///
    /// **A type VARIABLE has no answer yet, so it does not get one.**  The
    /// site is stamped [`TV_NULLCHECK`](Parser::TV_NULLCHECK) and
    /// [`rewrite_generic_type_defaults`](Parser::rewrite_generic_type_defaults) re-runs
    /// this same dispatch once `T` is concrete.  A nested generic — where `concrete` is
    /// still an outer template's variable — re-stamps through that same call and stays
    /// deferred, the way loft#1016 / #1020 / #1028 each do.
    /// Enforces the test half of @FR-N-Coal — `e ?? d` discharges `τ?` to `τ`, and this is
    /// the per-representation `is null` that decides whether `d` is taken.
    pub(crate) fn coalesce_not_null(&mut self, src: &Value, tp: &Type) -> Value {
        if self.is_type_var_operand(tp) {
            // Deferred, NOT decided.  The placeholder is an attribute-less struct, so
            // every arm below would read it as a reference and bake `rec != 0` — which
            // substitution keeps while it rewrites the type around it.
            return v_block(vec![src.clone()], tp.clone(), Self::TV_NULLCHECK);
        }
        // @PLAN52 cluster IV-Tuple (2026-05-30): Tuple values are not heap-DbRef
        // and have no `.rec != 0` discriminant; testing the WHOLE tuple as a
        // DbRef (the default `convert(Tuple, Boolean)` path) produces wrong
        // codegen (native E0308: `expected bool, found tuple`) and silent
        // corruption on interpret (treating bytes as a DbRef tag).  Convention:
        // a tuple is null when its FIRST FIELD is its type's null sentinel —
        // which matches what `OpGetVectorNullable` produces for OOB tuple reads
        // (each field gets its own null sentinel).
        if let Type::Tuple(elems) = tp
            && !elems.is_empty()
            && let Value::Var(v) = src
        {
            let first_tp = elems[0].clone();
            // RECURSE rather than calling the generic `convert(first_tp, Boolean)`: the
            // heap-DbRef branch below exists precisely because that generic path has no
            // registered `OpConv*FromX -> Boolean` for a collection and hands back the
            // bare Var, which the interpreter then tests as raw bytes.  Asking it here
            // puts a `vector`-first tuple through exactly that hole: `v[0] ?? fb` then
            // answers the FALLBACK for a PRESENT element, losing the scalar half with
            // it, on `--interpret` only.  A `Reference` first element hides it: that one
            // does have a generic path.  Recursion is also what keeps the two answers
            // from drifting, which is the same reason `ref_tuple_element_ok` is one list.
            self.coalesce_not_null(&Value::TupleGet(*v, 0), &first_tp)
        } else if let Type::Enum(syn, true, _) = tp
            && self.data.def(*syn).name.starts_with("__nullable<")
        {
            // @PLN25 E2 — a synthetic `__nullable<S>` enum backs an INLINE
            // field / vector element (no DbRef slot), so null is discriminant 0
            // — NOT the `.rec`/store_nr sentinel that `OpConvBoolFromRef` below
            // tests (it would misread the inline value).  The builder produces
            // "is NOT null" (v_if true → keep lhs), so test discriminant != 0.
            // Mirrors the E2a.4 `== null` lowering (`enum_null`).
            let get_enum = self.cl("OpGetEnum", &[src.clone(), Value::Int(0)]);
            let disc = self.cl("OpConvIntFromEnum", &[get_enum]);
            let is_null = self.cl("OpEqInt", &[disc, Value::Int(0)]);
            self.cl("OpNot", &[is_null])
        } else if Self::is_collection_type(tp.base()) {
            // A collection answers through [`collection_is_null`](Parser::collection_is_null),
            // the same lowering `== null` uses, because they are one question.  Reading the
            // `rec` of a heap DbRef — what the arm below does — is right only for a value
            // that IS the collection; a field, an element and a parameter given either are
            // addresses of the SLOT holding it, and their `rec` belongs to the holder.
            //
            // `is_collection_type` also covers `spatial` (`Radix`) and `trie`, which the
            // hand-written variant list below never named: they fell through to the generic
            // `convert`, which has no `OpConv*FromX -> Boolean` for a collection and hands
            // back the bare handle — so the interpreter tested twelve pointer bytes as a
            // boolean and `--native` would not compile the `if` at all (loft#1120).
            let is_null = self.collection_is_null(src);
            self.cl("OpNot", &[is_null])
        } else if matches!(tp, Type::Reference(_, _) | Type::Enum(_, true, _)) {
            // @PLAN52 cluster IV interpret (2026-05-30): a heap-DbRef type registers no
            // `OpConv*FromX → Boolean`, so the generic `convert(Reference, Boolean)` hands
            // back the bare Var and the interpreter's `GotoFalseWord` tests the FIRST BYTE
            // of a twelve-byte pointer.  `OpConvBoolFromRef` asks `rec != 0` instead.
            //
            // Right for a bare reference and a struct-enum, whose value IS the record it
            // points at — a missing one has no record.  A COLLECTION is the case above:
            // there the DbRef addresses the slot HOLDING the collection, so `rec` names
            // the holder and says nothing about what the slot contains.
            let conv_nr = self.data.def_nr("OpConvBoolFromRef");
            Value::Call(conv_nr, vec![src.clone()])
        } else if matches!(tp, Type::Boolean) {
            // @PLN17: the null-check is "is NOT null" (v_if true → keep lhs).
            // For a boolean that is `src != null` (raw `!= 255`), NOT the
            // value's truthiness — so `false ?? x` keeps `false`.
            let null_b = self.cl("OpConvBoolFromNull", &[]);
            self.cl("OpNeBool", &[src.clone(), null_b])
        } else {
            let mut nc = src.clone();
            self.convert(&mut nc, tp, &Type::Boolean);
            nc
        }
    }

    /// `lhs ?? return ret_expr` — emit the block that returns early when `lhs`
    /// is null and otherwise evaluates to `lhs`.
    fn build_null_coalesce_return(&mut self, code: &mut Value, ctp: &mut Type, lhs_type: &Type) {
        // Parse the optional return expression, coercing to the function's
        // declared return type.  Empty `return` in a non-void function
        // produces the typed null sentinel.
        let mut ret_val = Value::Null;
        let r_type = self.data.def(self.context).returned().clone();
        if !self.lexer.peek_token(";") && !self.lexer.peek_token("}") {
            let ret_pos = self.lexer.peek_pos().clone();
            let t = self.expression(&mut ret_val);
            // @FR-N-Store: `lhs ?? return ret` returns `ret` into the caller's non-null return
            // slot — the store face asks; a bare `null` return takes the sentinel path below
            // and is asked at the site.
            if t == Type::Null {
                self.n_store_violation(&t, &r_type, "the return value", None);
            }
            if t != Type::Null
                && !self.convert_store(&mut ret_val, &t, &r_type, "the return value", None)
                && !self.first_pass
            {
                self.validate_convert("return", &t, &r_type, &ret_pos);
            }
        } else if r_type != Type::Void && !self.first_pass {
            ret_val = self.null_value(&r_type);
        }
        let ret_stmt = Value::Return(Box::new(ret_val));

        // { tmp = lhs; if (tmp == null) { return ret_expr; }; tmp }
        let tmp = self.create_unique("ncr", lhs_type);
        let set_tmp = v_set(tmp, code.clone());
        let is_null = if self.is_type_var_operand(lhs_type) {
            // `T` has no null test yet, so ask the one home and negate its
            // answer.  Inside a template that call stamps the deferral marker, which the
            // monomorph answers; for a concrete type this branch never runs, so the two
            // arms below keep emitting exactly what they always did.
            let not_null = self.coalesce_not_null(&Value::Var(tmp), lhs_type);
            self.cl("OpNot", &[not_null])
        } else if matches!(lhs_type, Type::Boolean) {
            // @PLN17: a boolean is null iff its byte is the 255 sentinel — a raw
            // `tmp == null` compare, NOT truthiness (which would treat `false` as
            // missing).  `false ?? return` keeps `false`; `null ?? return` returns.
            let null_b = self.cl("OpConvBoolFromNull", &[]);
            self.cl("OpEqBool", &[Value::Var(tmp), null_b])
        } else {
            let mut null_check = Value::Var(tmp);
            self.convert(&mut null_check, lhs_type, &Type::Boolean);
            // Negate: true when null (i.e. when the boolean conversion is false).
            self.cl("OpNot", &[null_check])
        };
        let if_ret = v_if(is_null, ret_stmt, Value::Null);
        *code = v_block(
            vec![set_tmp, if_ret, Value::Var(tmp)],
            lhs_type.clone(),
            "ncr",
        );
        *ctp = lhs_type.clone();
    }

    /// `lhs ?? default` — emit the `if` (or temp + `if` for non-trivial lhs)
    /// that selects the default when `lhs` is null.
    /// @PLN25 — wrap a DENSE `S` default in a `__nullable<S>::Some` so both arms of a `??`
    /// have the same shape.
    ///
    /// The coalesce over a nullable STRUCT keeps its `__nullable<S>` result on purpose — the
    /// E2 gap-2 note in [`build_null_coalesce_default`](Self::build_null_coalesce_default)
    /// records why, and every use site coerces from there.  So the `if` it builds projects the
    /// payload out of its result, and an arm that is a bare `S` record is read at the
    /// payload's OFFSET instead: `s = o.f ?? dflt` answered the default's SECOND field.
    ///
    /// A LITERAL default never needs this — it is parsed against the join's shape and
    /// `parse_object` builds the `Some` directly.  A value that already exists (a variable, a
    /// call result, a field read) cannot be re-parsed, which is the whole difference.
    ///
    /// A no-op unless the result really is the synth for the default's own struct, so a
    /// nullable default (`S? ?? S?`) and every non-struct coalesce are untouched.
    fn wrap_dense_default_as_some(&mut self, result_type: &Type, rhs_type: &Type, rhs: &mut Value) {
        if self.first_pass {
            return;
        }
        let (Type::Enum(syn, true, _), Type::Reference(struct_d, _)) =
            (result_type.base(), rhs_type.base())
        else {
            return;
        };
        if !self.data.is_nullable_synth_of(*syn, *struct_d) {
            return;
        }
        let syn = *syn;
        let some_d = self.data.variant_of(syn, "Some");
        let kt = self.data.def(syn).known_type();
        if some_d == u32::MAX || kt == u16::MAX {
            return;
        }
        let w = self.vars.work_refs(
            &Type::Enum(syn, true, crate::data::Deps::none()),
            &mut self.lexer,
        );
        if w == u16::MAX {
            return;
        }
        let src = std::mem::replace(rhs, Value::Null);
        let mut ops = vec![
            v_set(w, Value::Null),
            self.cl("OpDatabase", &[Value::Var(w), Value::Int(i32::from(kt))]),
        ];
        ops.extend(self.build_some_present(some_d, Value::Var(w), src));
        ops.push(Value::Var(w));
        *rhs = v_block(
            ops,
            Type::Enum(syn, true, crate::data::Deps::frame1(w)),
            "NullableSome",
        );
    }

    /// Build the null-check both `a ?? d` and the postfix `a?` lower to, and record
    /// WHICH of the two this was in [`Parser::last_place_discharge`].
    ///
    /// The two spellings are one mechanism and produce one shape, so nothing downstream
    /// can tell them apart by reading the IR — and on an assignment place they do not
    /// mean the same thing (loft#1205).  The postfix form is the one carrying a pending
    /// default: the caller supplies it from the type rather than from the source text.
    ///
    /// The flag is set on the way OUT, so a `?` nested inside the default (`v[i?] ?? 0`)
    /// leaves the OUTER spelling's answer standing rather than its own.
    fn build_null_coalesce_default(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
        ctp: &mut Type,
        lhs_type: &Type,
    ) {
        let is_postfix = self.pending_default_src.is_some() || self.pending_default_rhs.is_some();
        self.build_null_coalesce_default_inner(var_tp, code, parent_tp, precedence, ctp, lhs_type);
        self.last_place_discharge = is_postfix;
    }

    #[allow(clippy::too_many_arguments)] // the wrapper's list, unchanged from before the split
    fn build_null_coalesce_default_inner(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
        ctp: &mut Type,
        lhs_type: &Type,
    ) {
        let mut rhs = Value::Null;
        let rhs_pos = self.lexer.peek_pos().clone();
        // Thread the LHS BASE type as the default's hint when the context gives
        // none (`var_tp` unknown/null — e.g. a RETURN-TAIL `v[i] ?? []`, where
        // parse_return's `[`-led vector hint cannot fire).  Without it an EMPTY
        // vector default types Unknown and collapses to `null` (the P365 family),
        // which downstream classifies the ncc `if` as a null-arm shape — the
        // return delivery then skips materialisation and native emits `()`.
        // loft#1208 — a POSTFIX `x?` supplies its own default, and `(N-Default)` says exactly
        // which one: `construct_default(τ)` for the SUBJECT's τ.  `var_tp` is the type the
        // enclosing context wants from the whole postfix CHAIN — `integer` in `e = v?[0]`,
        // because `?` binds tighter than `[]` and the chain continues after it
        // (@FR-G-Post-Default).  That is a different expression, so it is never the hint for
        // this default.  Preferring it parsed the collection default `[]` against `integer`
        // and then reported the disagreement it had just created: *"`??` default of type
        // `integer` is not assignable to `vector<integer>`"* — naming a `??` the program does
        // not contain.  The parenthesised `(v?)[0]` escaped only because a sub-expression
        // parse enters with no expected type, which falls through to the same `lhs_type.base()`
        // this now takes directly.
        let postfix_default = self.pending_default_src.is_some();
        let rhs_hint = if postfix_default || matches!(var_tp, Type::Unknown(_) | Type::Null) {
            lhs_type.base()
        } else if let (Type::Enum(syn, true, _), Type::Reference(struct_d, _)) =
            (lhs_type.base(), var_tp.base())
            && self.data.is_nullable_synth_of(*syn, *struct_d)
        {
            // @PLN25 — the target is the DENSE `S` while the subject is the synthetic
            // `__nullable<S>`: one notion, two spellings, and the hint has to answer for the
            // JOIN, not for the destination.  The two arms of the `if` this builds must have
            // the SAME shape, and reaching the dense target is a separate step the assignment
            // site already does (`nullable_to_dense_assign` unwraps the payload and copies).
            //
            // Hinted with the dense `S`, `?? S { … }` built a bare `S` record beside a
            // `__nullable<S>` one, and the payload projection over the join then read the
            // literal at the payload's OFFSET: `s = o.f ?? P { x: 6, y: 5 }` answered `s.x ==
            // 5` and `s.y` past the end of the record, silently, on both backends.  The same
            // expression used directly — with no target to hint from — was always right.
            lhs_type.base()
        } else {
            var_tp
        };
        // @PLN116 `x?` — the postfix default-fallback supplies its right operand here
        // instead of the source (there is no `?? <expr>` in the text).  Two forms, both
        // emitting the exact same null-check `if`/block a hand-written `x ?? <default>`
        // would: a SOURCE (an empty collection `[]`, parsed HERE so its ownership
        // view-model matches the in-context `?? []`), or a PRE-BUILT value (scalars,
        // text, enum, record).
        let rhs_type = if let Some(src) = self.pending_default_src.take() {
            let saved = std::mem::take(&mut self.lexer);
            self.lexer.parse_string(&format!("{src}\n"), "<x?-default>");
            let t = self.parse_operators(rhs_hint, &mut rhs, parent_tp, precedence + 1);
            self.lexer = saved;
            t
        } else if let Some((pending, pending_tp)) = self.pending_default_rhs.take() {
            rhs = pending;
            pending_tp
        } else {
            self.parse_operators(rhs_hint, &mut rhs, parent_tp, precedence + 1)
        };
        // loft#1003 — the default's END, for the `redundant-coalesce` deletion span.
        // Taken HERE and not at the caller's tail: by then the cursor has moved past the
        // statement terminator, and a span that swallows the `;` is a rewrite that
        // introduces a syntax error — which the fix verifier duly REJECTED.
        // `peek_pos()`, not `at()`: positions here are the cursor AFTER a token and the
        // lexer peeks one ahead, so `at()` is already past the statement terminator.  The
        // START of the following token is the default's exclusive end.
        self.ncc_default_end = Some({
            let p = self.lexer.peek_pos();
            (p.line, p.pos)
        });
        self.known_var_or_type(&rhs, &rhs_pos);

        // @PLN102 gate-2 `?? null` soundness — `a ?? b` yields `a` when non-null else `b`, so it
        // can STILL be null exactly when the FALLBACK `b` is nullable (a bare `null` literal, or a
        // `τ?`-typed expression). In that case the result type must stay `τ?`, not peel to the
        // non-null base — else `y: integer = x ?? null` accepts a null into a non-null slot. The
        // code below builds with the non-null base (the null sentinel is representable in the
        // base's storage: `i64::MIN`, a null DbRef, …); only the REPORTED `*ctp` is re-wrapped, so
        // the N-Store check at the consuming slot rejects. Gated `LOFT_NO_QQ_NULL` (default on).
        let fallback_nullable = crate::keys::qq_null_typing_enabled()
            && (matches!(rhs.unspan(), Value::Null) || matches!(rhs_type, Type::Optional(_)));

        // The default may be a vector literal (`?? []`, `?? [99]`) that builds into
        // its OWN work-ref `_vec_N` — the last operator of the emitted `"Vector"`
        // block.  That work-ref's only `Set` lives in the else-branch block the
        // slot scan does not walk (the #319 unwalked-block gap — here for the
        // DEFAULT literal, not the subject temp handled below), so without help its
        // slot stays unassigned and codegen panics `Incorrect var _vec_N[65535]`.
        // Register it (function-entry preamble reserves the slot) and clear its
        // borrow-dep so the preamble owned-inits it as the owned vector it is.
        if let Value::Block(bl) = rhs.unspan()
            && bl.name == "Vector"
            && let Some(Value::Var(w)) = bl.operators.last().map(Value::unspan)
        {
            let w = *w;
            self.vars.register_work_ref(w);
            // Whether `_vec_N` is BACKED — a view of a `__vdb_N` wrapper record rather
            // than the owner of its own store.  Read off the emitted block, which SAYS so:
            // `vector_db` writes `_vec_N = OpGetField(__vdb_N, 0)` into it, and the shape
            // that owns its store has no such assignment (its literal builds straight into
            // `_vec_N` through `OpPreAllocVector`).  The dep list cannot answer this — the
            // strip below clears it for both shapes.
            let backed = bl.operators.iter().any(|op| {
                matches!(op.unspan(), Value::Set(sv, val)
                    if *sv == w && self.inline_slot_word(val).is_some())
            });
            // The backing record this view reads out of, read BEFORE the strip below removes
            // the only place it is written down.
            let backing: Option<u16> = match self.vars.tp(w).clone() {
                Type::Vector(_, dep) => dep.as_slice().first().copied(),
                _ => None,
            };
            if let Type::Vector(elm, dep) = self.vars.tp(w).clone()
                && !dep.is_empty()
            {
                // An owned literal default borrows nothing — clear the frame-dep so
                // the preamble gate (dep-empty) fires AND `gen_set_first_vector_null`
                // takes the owned-vector alloc path.  `set_type` overwrites the dep;
                // `change_var_type` early-returns on `is_equal` (deps ignored) and
                // only ADDS deps, so it cannot clear.
                self.vars
                    .set_type(w, Type::Vector(elm, crate::data::Deps::none()));
            }
            // @PLN102 default-taken leak (OWNED subject only): `_vec_N` is ALWAYS
            // field 0 of the default record `__vdb_N` (`_vec_N = OpGetField(__vdb_N,
            // 0)`), never a store it owns.  Without this, the owned-init preamble above
            // `OpDatabase`-allocates a `main_vector` store at `_vec_N`'s slot that the
            // `OpGetField` then OVERWRITES — orphaning it (the kt=21 leak when the
            // default arm runs, e.g. `nun() ?? [d]` in an assign).  `skip_free` lowers
            // the preamble to a NULL SENTINEL (no alloc, no orphan — the slot is still
            // reserved by the frame bump) and suppresses its scope-exit `OpFreeRef`, so
            // the record `__vdb_N` is the SOLE owner and its free cascades to the whole
            // vector.  Mirrors the `if`/`match` view-model (the arm's `_vec` is a
            // never-freed view of its `__vdb`).  Gated on an OWNED subject: a BORROWED
            // subject's `??` in return-tail position hands `_vec_N` to the return-
            // delivery materializer, which OWNS its free (free-after-append + the
            // cross-arm sweep — see 85-ncc-literal-return-delivery.loft); suppressing
            // it there double-drops / panics.
            if matches!(lhs_type, Type::Vector(_, dep) if dep.is_empty()) {
                self.vars.set_skip_free(w);
            } else if backed {
                // A BORROWED subject gets the OTHER half of that bit: `inline_ref` says
                // *"do not ALLOCATE in the preamble"* WITHOUT saying *"never free"*, so the
                // doomed store goes away and every free that exists today still runs — which
                // is what the return-delivery materializer relies on.  `skip_free` says both
                // at once, and it is only the second half this case cannot have.
                //
                // The orphan is the same one described above, reached from the other side:
                // the preamble `OpDatabase`-allocates at `_vec_N`'s slot and the
                // `OpGetField(__vdb_N, 0)` overwrites it.  `--interpret` recovers by freeing
                // the displaced store before it rebinds; `--native` has no such step, so it
                // orphaned one store per evaluation, unbounded in a loop (loft#1121).
                //
                // `backed` is what makes this safe to say: an UNBACKED `_vec_N` IS the owner
                // of its store — nothing overwrites it, `OpPreAllocVector` fills it in place
                // — and taking its preamble allocation away leaves the default arm building
                // into a null sentinel, which answers length 0 for every literal default.
                self.vars.mark_inline_ref(w);
                // loft#1322 — and then exactly one of the pair may free the store.
                //
                // `_vec_N` and `__vdb_N` name ONE store: the view is `OpGetField(__vdb_N, 0)`.
                // This arm leaves the view's free in place (the return-delivery materializer
                // owns it) while the record keeps a scope-exit free of its own, so the same
                // store was released twice — `free #N already_free=false` then `=true`, both
                // backends.  `(O-Borrow)`: one owner, one free.  Here the view's free is the
                // one that runs, so the record is the one that goes quiet.
                //
                // ⚠ ONLY WHERE THE VIEW'S FREE ACTUALLY RUNS, and the flags are CUMULATIVE
                // ACROSS PASSES, which is what makes that a real question rather than a
                // rhetorical one.  A capture subject reads `deps=[]` on pass 1 and takes the
                // OWNED arm above — `skip_free(_vec_N)`, the view model — and reads `deps=[…]`
                // on pass 2 and arrives here.  Such a `_vec_N` is already never-freed, so
                // silencing the record too leaves the store with no owner at all: measured, it
                // exhausts the 65535-store table in
                // `1248b-a-capture-witness-is-the-slot-the-return-reads`.  Asking
                // `is_skip_free(w)` is asking which of the two arms this variable ENDED in,
                // not which one this pass took.
                if !self.vars.is_skip_free(w)
                    && let Some(db) = backing
                {
                    self.vars.set_skip_free(db);
                }
            }
        }

        if matches!(lhs_type, Type::Null) {
            // LHS is an untyped null literal: always use the RHS.
            *code = rhs;
            *ctp = rhs_type;
            return;
        }

        // @P316 — pick the coalesce result type.  When the value and default are
        // integers of DIFFERENT specs (e.g. a narrow `u8` element + an
        // `integer`/`i64` default), native can't unify the `if` branches — it emits
        // `(if … {u8} else {0_i64}) as u8` → E0308 (`convert` leniently accepts any
        // Integer→Integer without actually re-typing, so the branches keep their
        // own native widths).  Widen BOTH branches to `i64` so they share a native
        // type; the result is `i64` (matching the surrounding integer arithmetic).
        // Matching-width integers and non-integer types keep the original
        // behaviour (bring the default to the value's type; result = value's type).
        let widen_ints = matches!(lhs_type, Type::Integer(_))
            && matches!(rhs_type, Type::Integer(_))
            && *lhs_type != rhs_type
            // A CONSTANT integer default that provably fits the value's narrow type
            // coerces to that type instead of widening: a literal emits at the
            // target width, so the `if` branches still share a native type and the
            // E0308 @P316 guards against (a wider-width *variable* default) cannot
            // arise.  Keeping the result narrow lets `vec_of_u8[i] ?? 0` stay `u8`
            // — no `as u8` cast and no spurious narrowing error at a later
            // `out += [x]`.  A non-const or non-fitting default still widens.
            && !self.int_value_fits(&rhs, lhs_type);
        // NB (@PLN25 E2 gap 2): do NOT coalesce a `__nullable<S> ?? dense_S` to dense
        // here.  It is tempting (it would make `chosen = v[i] ?? mk()` infer dense
        // `S` and dodge the return-boundary unwrap), but it BREAKS the common
        // `out += [chosen]` shape — a dense `chosen` then needs a dense→Some WRAP on
        // every append, which fails in loop/if contexts.  Keep the conservative
        // `__nullable<S>` result and let each USE site coerce (dense-assign routing,
        // the ref_return return-boundary unwrap) — that keeps appends nullable→nullable.
        // A checked narrowing cast discharged here (`e as N ?? d` / `e as N? ?? d`):
        // the `??` removes the null, so the surviving value is provably in N's range.
        // Type the RESULT as the narrow `N` — the cast BLOCK stayed `integer` to hold
        // its null sentinel, but past the discharge a `u8` field / return / local can
        // accept it with no second cast — provided the default also fits `N`.
        let checked_narrow = self
            .dn4_checked_narrow
            .take()
            .filter(|nt| self.int_value_fits(&rhs, nt));
        let result_type = if let Some(Type::Integer(nspec)) = &checked_narrow {
            // The discharged value is provably in N's range, but both `??` branches are
            // full 8-byte integers (the cast block keeps its i64 null sentinel). So type
            // the result as a FULL-WIDTH integer (`forced_size: None`) carrying N's RANGE:
            // the range makes a later `u8` field / return / local accept it without a cast
            // (is_narrowing_int = false, exactly like an `integer limit(0,255)` value),
            // while the full width keeps the branches the same native type (no @P316 E0308).
            Type::Integer(IntegerSpec {
                forced_size: None,
                ..*nspec
            })
        } else if widen_ints {
            crate::data::I64.clone()
        } else {
            lhs_type.clone()
        };
        // Bring the default to the result type (widen narrow→i64, or the original
        // default→value-type convert); report a genuine mismatch (e.g. `text ?? 0`).
        // The default meets the coalesce's RESULT type — `(N-Coal)`'s `d ⇐ τ` is the
        // coalesce's own typing (`?? null` keeps the value nullable, a nullable default
        // widens it), not a store into a slot: @FR-N-Store admits it here and is asked
        // wherever the coalesced value lands.
        if !self.convert_admitting(&mut rhs, &rhs_type, &result_type) && !self.first_pass {
            // @PLN102 arc-E — `convert` FAILED: the default `d` is not assignable to the
            // coalesce type, i.e. `τ? ?? d` with `d` not usable where a `τ` is expected.
            // Left unreported (the old dead `can_convert` call), this built a MISMATCHED
            // coalesce that the interpreter reinterprets one representation as the other:
            // SIGSEGV (`ref? ?? int` — int used as a DbRef pointer) or silent corruption
            // (`int? ?? float` — float bits read as int; `int? ?? text` — pointer as int);
            // `--native` rejected it at rustc (E0308), a backend divergence. Rejecting it
            // here in the parser closes ALL of that on BOTH backends. Falsified over the
            // whole corpus/scripts/libs: zero valid `??` fires this (widen-int, `float?`←int
            // widening, `?? null`, `?? []`, checked-narrow all `convert` cleanly above).
            let (given, wanted) = (rhs_type.name(&self.data), result_type.name(&self.data));
            diagnostic!(
                self.lexer,
                Level::Error,
                code = "coalesce-default-type-mismatch",
                "`??` default of type `{given}` is not assignable to `{wanted}` — a default \
                 must be usable where the value's type is expected",
            );
            // @PLN131 — ONE fix, and no `edit`.  The obvious rewrite (`… as {wanted}`) is
            // not sound to spell: `"x" as integer` is a text parse that can fail, so
            // synthesising it here would answer one error with another.  Which SIDE is
            // wrong is the author's to say, and that is what the condition asks.
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: format!("give the default a value of type `{wanted}`"),
                condition: Some(format!(
                    "the `??` is meant to produce `{wanted}` — if it should produce \
                     `{given}`, the VALUE's type is what to change instead"
                )),
                edit: None,
                concept: "null coalescing",
                concept_ref: "@F2",
            });
        }
        // `convert(value → result_type)` widens the value branch when widen_ints,
        // and is a no-op otherwise (result_type == lhs_type).
        let null_check_builder =
            |this: &mut Self, src: &Value| -> Value { this.coalesce_not_null(src, lhs_type) };
        // loft#1013 — a CALL default needs an owner when the RESULT is a BORROW.
        //
        // `x = v[i] ?? mk()` types the result from the SUBJECT, which names the vector
        // (`ref(S)["v"]`), so the consumer reads `x` as a borrow and frees nothing while
        // the MISS path hands back a store `mk()` freshly allocated.  A struct LITERAL
        // default was always clean for the opposite reason — `parse_object` allocates it
        // into a work-ref this frame already frees — and the compiler's refusal of a
        // struct-valued constant PRESCRIBES the call spelling, so the leaking form is the
        // one it tells you to write.  [`Self::materialise_owned_call`] has the model; the
        // same join shape written as an `if` or a `match` is loft#1019.
        if !self.first_pass
            && matches!(result_type.base(), Type::Reference(_, _))
            && !result_type.depend().is_empty()
            && let Some(owned) = self.materialise_owned_call(&rhs, &result_type)
        {
            rhs = owned;
        }
        if let Value::Var(_) = code {
            // Simple variable: reading twice is side-effect-free.
            let mut lhs = code.clone();
            let null_check = null_check_builder(self, code);
            self.convert(&mut lhs, lhs_type, &result_type);
            *code = v_if(null_check, lhs, rhs);
        } else {
            // Non-trivial expression: materialise into a temp to avoid double
            // evaluation (L6 fix).
            //
            // @PLAN52 cluster I iteration 2 (2026-05-30): name `__ncc_N`
            // (double-underscore, matching loft's hoisted-temp convention)
            // and mark `skip_free` for text.  The skip_free flag suppresses
            // `OpFreeText(_ncc_N)` at block-scope exit (interpret side, see
            // `src/scopes.rs::get_free_vars`), so the present-path Str's
            // backing String outlives the block.  Native emit recognises
            // the `__ncc_*` prefix at `src/generation/emit.rs::output_block`
            // and wraps the tail with `.to_string()` INSIDE the block,
            // producing an owned String that the outer consumer can copy
            // safely.
            let tmp = self.create_unique("_ncc", lhs_type);
            // @PLAN52 cluster IV interpret (2026-05-30, iteration 3): extend
            // skip_free from Text to ALL heap-DbRef LHS types.  Same
            // mechanism: the scope-exit free op (`OpFreeText` for text,
            // `OpFreeRef` for heap-DbRef per `src/scopes.rs::get_free_vars`
            // heap-Free branch at line ~1274 which ALREADY honors
            // `is_skip_free`) is suppressed for `__ncc_N` temps so the
            // present-path value's backing storage outlives the block.
            // Closes Set E interpret (probes 21, 22, 23, 36, 41, 50).
            // @PLN102 `??` heap-ownership: distinguish an OWNED Vector subject (a
            // call result / comprehension — the value the `??` block must free) from
            // a BORROWED one (`vv[i]`, `s.field` — deps name the source; freeing it
            // is a use-after-free).  Only the OWNED case migrates to the view-model
            // below; a BORROWED Vector keeps the original skip_free hand-off (its
            // store is owned elsewhere and must NOT be freed here).
            //
            // loft's core convention is `dep.is_empty()` == owned, and that is a
            // PROXY: what "owned" means here is that no one else's store is reached
            // through this value.  A dep naming THIS FRAME's own hidden return buffer
            // satisfies that — the buffer is the caller's, minted for this very call
            // — so emptiness is too strong a test.  loft#938's dep translation made
            // that visible: once `mkv()`'s nullable result correctly names the
            // `__ref_N` it is delivered into, `dep.is_empty()` flipped false, the
            // subject was misread as borrowed, and the `??` handed its store off to
            // nobody — `return mkv() ?? [1]` answered length 0 on `--native`
            // (`562-ncc-owned-subject-consumers.loft`, native-only, both switches on).
            //
            // Ask the question the proxy stood for: every dep is one of our own
            // caller-minted buffers.  Same predicate `is_struct_returning_call` uses
            // for loft#953, and the same hidden-vs-visible rule as
            // `Definition::returns_borrowed_view` — a hidden attr is not a borrow.
            let owned_vector = matches!(lhs_type, Type::Vector(_, dep)
                if dep.iter().all(|&d| self.vars.is_caller_hidden_buf(d)));
            // A RECORD subject that is a CALL is what a plain bind would leave the local
            // OWNING — a fresh mint is adopted, a borrowed or `Join` return is deep-copied by
            // codegen (`Definition::return_adopts_fresh_store` is the split), and a fn-ref
            // call's is the store minted for that call or `OpBindOrCopy`'s answer — so the
            // temp that binds it owns a store on every arm too (`@FR-O-Complete`: the fact
            // per binding, per path).  Marked never-free, the arm that minted was owned by
            // nobody: `r = g(none) ?? d` held one record per call to frame exit, and
            // `r = mk(i) ?? d` was clean only while `r` freed on the mint arm a store it
            // merely borrowed on the other.  A projection subject (`o.inner ?? d`) stays the
            // view it is (`@FR-B-View`), and a bare variable is never hoisted at all.
            let owned_record = matches!(lhs_type, Type::Reference(_, _) | Type::Enum(_, true, _))
                && match code.unspan() {
                    Value::Call(d, _) => self.data.def(*d).is_loft_defined(),
                    Value::CallRef(_, _) => true,
                    _ => false,
                };
            if matches!(
                lhs_type,
                Type::Text(_) | Type::Sorted(_, _, _) | Type::Hash(_, _, _) | Type::Index(_, _, _)
            ) || (matches!(lhs_type, Type::Reference(_, _) | Type::Enum(_, true, _))
                && !owned_record)
                || (matches!(lhs_type, Type::Vector(_, _)) && !owned_vector)
            {
                self.vars.set_skip_free(tmp);
            }
            // An OWNED Vector subject OWNS the value `code` produced (typically
            // `mkv()`'s returned store) and MUST be freed at scope exit —
            // `skip_free` would suppress that and every transient consumer
            // (return/append/method/discard) would leak the present-path store.
            // Mark `inline_ref` instead: the de-conflated "borrow the slot, don't
            // ALLOCATE in the preamble" bit (`gen_set_first_vector_null` emits a
            // null sentinel, NOT an empty-vector `OpDatabase` that `code` would
            // immediately orphan), while `get_free_vars` still emits
            // `OpFreeRef(__ncc_N)` (inline_ref is not skip_free).  Exactly the
            // `__lift_N` owns-an-inline-call pattern (`scopes.rs::new_lift_var`).
            // Consumers borrow via the block's view dep (added below), so the
            // adopting assign does not double-free.
            if owned_vector {
                self.vars.mark_inline_ref(tmp);
            }
            // #319 — heap-DbRef ncc temps need a function-entry `Set(tmp,
            // Null)` (the work-ref preamble) to reserve their stack slot:
            // their only Set lives inside the ncc block, which the Zone-2
            // slot scan does not walk.  Shapes whose assigned-var deps reach
            // the temp got a slot via scan_set's dep-prefix; a subject whose
            // dep chain is broken (e.g. a comprehension-built vector) did
            // not — "Incorrect var __ncc_N[65535]" at codegen.  (An OWNED Vector
            // uses the `inline_ref` relocation path in `expressions.rs` for the
            // same slot reservation, so it is intentionally excluded here; a
            // BORROWED Vector keeps the original work-ref registration.)
            if matches!(lhs_type, Type::Reference(_, _) | Type::Enum(_, true, _))
                || (matches!(lhs_type, Type::Vector(_, _)) && !owned_vector)
            {
                self.vars.register_work_ref(tmp);
            }
            let set_tmp = v_set(tmp, code.clone());
            let null_check = null_check_builder(self, &Value::Var(tmp));
            let mut true_branch = Value::Var(tmp);
            self.convert(&mut true_branch, lhs_type, &result_type);
            self.wrap_dense_default_as_some(&result_type, &rhs_type, &mut rhs);
            let if_expr = v_if(null_check, true_branch, rhs);
            // @PLN102 `??` Vector view-model (OWNED subject only): the subject
            // `__ncc_N` is a function-scope OWNER freed by `get_free_vars` (see
            // `inline_ref` above), exactly like the `if`/`match` view-model's
            // `__vdb_N`.  So the block RESULT is a dep-backed VIEW naming
            // `__ncc_N` (`vector<T>["__ncc_N"]`), NOT the bare owner: a consuming
            // assign `x = e ?? d` then makes `x` a BORROW (no second `OpFreeRef`
            // → no double-free), while every transient consumer borrows and never
            // frees.  The default arm keeps its own `__vdb_N` owner (freed
            // independently); the result names the present arm's owner, mirroring
            // `if`'s `["__vdb_1"]`.  A BORROWED Vector subject and every non-Vector
            // heap subject keep the skip_free hand-off, so their result stays the
            // bare owner.
            if owned_vector && let Type::Vector(elm, _) = &result_type {
                let view = Type::Vector(elm.clone(), crate::data::Deps::frame(vec![tmp]));
                *code = v_block(vec![set_tmp, if_expr], view.clone(), "ncc");
                *ctp = Self::wrap_if_fallback_nullable(view, fallback_nullable);
                return;
            }
            *code = v_block(vec![set_tmp, if_expr], result_type.clone(), "ncc");
        }
        *ctp = Self::wrap_if_fallback_nullable(result_type, fallback_nullable);
    }

    /// @PLN102 gate-2 `?? null` — re-mark a coalesce RESULT nullable when its fallback can be null
    /// (see `build_null_coalesce_default`). Idempotent: never double-wraps an already-`Optional`.
    fn wrap_if_fallback_nullable(t: Type, fallback_nullable: bool) -> Type {
        if fallback_nullable && !matches!(t, Type::Optional(_)) {
            Type::optional(t)
        } else {
            t
        }
    }

    /// @PLN116 — the postfix default-fallback operator `x?`.  `x? : T` discharges a
    /// nullable `x: T?` to `T` by falling back to `T`'s default when `x` is null — pure
    /// sugar for `x ?? construct_default(T)`, reusing the whole `??` emission path.
    ///
    /// On a non-null operand the default is dead, so it is an identity plus a
    /// redundant-`?` warning (mirrors the redundant-`??` lint).  When `T` has no
    /// well-defined default (a bare reference, or a record with an un-defaulted
    /// non-null field) it is a COMPILE error — a static well-definedness check, fully
    /// consistent with "no runtime errors ever" (C80).
    pub(crate) fn handle_default_fallback(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        ctp: &mut Type,
    ) {
        // The subject of a postfix `?` is read through its tag first, as a `??` subject is:
        // the default is then built for the pointer's base `S`, the shape the present arm
        // has.
        self.read_through_tag(code, ctp);
        // Same C54.G-hybrid swap `??` does: if the operand is an immediate trapping
        // arithmetic / index op (`(a / b)?`, `v[i]?`), swap it to the Nullable peer so
        // it yields the sentinel silently instead of raising, then the fallback below
        // discharges it — making `x?` a true synonym for `x ?? <default>`.
        if !self.first_pass {
            Self::rewrite_outer_arith_to_nullable(code, &self.data);
        }
        let base = ctp.base().clone();
        // The single well-definedness check (the home `S{}` shares): a bare reference,
        // or a record with an un-defaulted non-null field or a bare enum field with no
        // explicit choice, has no default.  `x?` needing one there is a COMPILE error —
        // a static check, consistent with "no runtime errors ever" (C80).  `x?` is new
        // syntax (no program relies on it), so this always fires when the base lacks a
        // default; the message names the culprit and the remedy.
        if let Err(reason) = self.data.has_default(&base) {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`?` needs a default here, but {reason} — discharge with `?? <default>` \
                     instead, or keep the value nullable"
                );
            }
            self.expr_not_null = false;
            *ctp = base;
            return;
        }
        // Redundant-`?` warning — the SAME authority as the redundant-`??` lint: a
        // provably not-null operand (`expr_not_null`) whose type is not `Optional` and is
        // not a declared-nullable call.  A C80-nullable index / field read has
        // `expr_not_null == false`, so it is correctly NOT flagged — its default is live
        // (an out-of-bounds / missing read yields null).  Like `??`, the coalesce is
        // still emitted even when redundant (the default is simply never taken).
        if self.expr_not_null
            && !self.first_pass
            && !matches!(ctp, Type::Optional(_))
            && !self.call_declares_nullable(code)
        {
            diagnostic!(
                self.lexer,
                Level::Warning,
                code = "redundant-default-fallback",
                "Redundant `?` — '{}' is 'not null', so the type default is never used",
                self.expr_not_null_name,
            );
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Mechanical,
                title: "delete the `?`".to_string(),
                condition: None,
                edit: None,
                concept: "default fallback",
                concept_ref: "@F96",
            });
        }
        self.expr_not_null = false;
        // `a.b?` defends the `b` read (the default handles a null), so the not-null
        // field hint must not fire on it — mirrors `p.field ?? d`.
        if let Some(key) = self.last_field_read_site.take() {
            self.defended_field_reads.insert(key);
        }
        // A collection defaults to a REAL empty collection.  Its `[]` construction must
        // be parsed IN the coalesce's right-operand context (its ownership view-model
        // depends on that context — a standalone `[]` leaks), so route it as a SOURCE
        // that `build_null_coalesce_default` parses at its own parse site.
        if crate::parser::vectors::is_collection(&base) {
            *ctp = base;
            let lhs_type = ctp.clone();
            self.pending_default_src = Some("[]".to_string());
            self.build_null_coalesce_default(
                var_tp,
                code,
                parent_tp,
                OPERATORS.len(),
                ctp,
                &lhs_type,
            );
            return;
        }
        let Some((default, default_tp)) = self.build_default(&base) else {
            // `has_default` passed but the builder cannot form the value — recover as the
            // non-null base so the cascade is bounded (should not happen in practice).
            *ctp = base;
            return;
        };
        // Peel `Optional` → the non-null base, exactly as `handle_null_coalesce` does,
        // then hand the pre-built default to `build_null_coalesce_default` (it consumes
        // `pending_default_rhs` instead of parsing a right operand).
        *ctp = base;
        let lhs_type = ctp.clone();
        self.pending_default_rhs = Some((default, default_tp));
        // The precedence is irrelevant — the right operand is pre-built, never parsed.
        self.build_null_coalesce_default(var_tp, code, parent_tp, OPERATORS.len(), ctp, &lhs_type);
    }

    /// @PLN116 — build type `tp`'s default VALUE (the single source both `x?` and, in
    /// time, `S{}` consult).  `None` means `tp` has no well-defined default (the caller
    /// raises the compile error).  `tp` is the already-peeled non-null base.
    pub(crate) fn build_default(&mut self, tp: &Type) -> Option<(Value, Type)> {
        match tp {
            Type::Integer(_) | Type::Float | Type::Single | Type::Boolean => {
                Some((to_default(tp, &self.data), tp.clone()))
            }
            // `character`'s default `'\0'` is codepoint 0 wrapped in the char-typed
            // conversion (`OpConvCharacterFromInt`) — a bare `Int(0)` is a plain `i32`
            // that native rejects in the distinct `character` slot (E0308).  This is
            // exactly the value the `'\0'` literal emits.
            Type::Character => Some((
                self.cl("OpConvCharacterFromInt", &[Value::Int(0)]),
                tp.clone(),
            )),
            Type::Text(_) => Some((Value::Text(String::new()), tp.clone())),
            // Collections are handled by the caller via `pending_default_src` (parsed
            // in-context), never here — see `handle_default_fallback`.
            // An enum defaults to its first-defined variant (a marked default variant
            // would override — arc B; no marker exists yet).  `emit_variant_value`
            // handles both a unit enum (the discriminant) and a struct-enum (an
            // allocated variant record with its own fields defaulted).
            Type::Enum(enum_nr, _, _) => {
                let first = self.data.attr_name(*enum_nr, 0);
                if first.is_empty() {
                    return None;
                }
                let mut v = Value::Null;
                let t = self.emit_variant_value(*enum_nr, &first, &mut v);
                Some((v, t))
            }
            // A TYPE VARIABLE has no default of its own.  `construct_default` is a
            // function of the CONCRETE type (types.md `(D-Scalar)` … `(D-Rec)`), and
            // inside a template `T` is an attribute-less placeholder STRUCT — which the
            // record arm below defaults, perfectly happily, to an empty record of the
            // placeholder's own store type.  That is what shipped: `g<T>(v, a: T? = null)
            // { a? }` allocated a `__typevar_T` record, leaked it, and read it back as
            // whatever the instantiation's type is — `34359738369` for `integer`, a
            // denormal for `float`, a SIGSEGV for `text`.
            //
            // So mark the site instead and leave it to the monomorph, where `T` is
            // concrete and `has_default` is decidable
            // ([`Parser::rewrite_generic_type_defaults`]).  The placeholder record is
            // still built underneath, because a template's IR is type-checked and
            // slot-allocated even though it never runs (loft#1016).
            Type::Reference(d_nr, _) if self.data.is_type_var_placeholder(*d_nr) => {
                // The sub-parse is SOURCE text, so it needs the spelling the header wrote —
                // a second header binding the same letter holds a placeholder minted as
                // `T#2`, which no source can name.  Pinning the enclosing header's binding
                // over the sub-parse is what makes the spelling resolve back to THIS
                // placeholder rather than to whichever one the letter names elsewhere.
                let d_nr = *d_nr;
                let name =
                    crate::data::Data::type_var_spelling(self.data.def(d_nr).name()).to_string();
                let saved = (
                    self.cur_type_var,
                    std::mem::take(&mut self.cur_type_var_name),
                );
                self.cur_type_var = d_nr;
                self.cur_type_var_name.clone_from(&name);
                let (v, t) = self.subparse_default(&format!("{name} {{}}"), tp);
                self.cur_type_var = saved.0;
                self.cur_type_var_name = saved.1;
                Some((v_block(vec![v], t.clone(), Self::TV_DEFAULT_BLOCK), t))
            }
            // A record defaults to `S{}` — every field defaulted, exactly the value a
            // bare `S{}` literal builds (`has_default` has already verified each field
            // has a default).  Parsed from the synthetic `S {}` source so it reuses
            // `parse_object`'s construction AND its work-ref ownership / freeing (a
            // hand-rolled `OpDatabase` + `object_init` mismanages the store free — #306).
            Type::Reference(d_nr, _)
                if self.data.def_type(*d_nr) == crate::data::DefType::Struct =>
            {
                let name = self.data.def(*d_nr).name().to_string();
                Some(self.subparse_default(&format!("{name} {{}}"), tp))
            }
            _ => None,
        }
    }

    /// @PLN116 — parse a synthetic default-value SOURCE (e.g. `Point {}`) as if it
    /// were a `??` right operand, then restore the main token stream.  Used for the
    /// record default: a struct construction is token-driven (`parse_object` reads
    /// `{ … }`), so it cannot be hand-built as a bare `Value`; sub-parsing `S {}`
    /// reuses `parse_object`'s construction AND its work-ref ownership / freeing.
    /// The parse runs in the SAME function context (only the lexer is swapped), so
    /// its work-refs, types and pass state are all correct; `hint` types the parse.
    pub(crate) fn subparse_default(&mut self, src: &str, hint: &Type) -> (Value, Type) {
        let saved = std::mem::take(&mut self.lexer);
        self.lexer.parse_string(&format!("{src}\n"), "<x?-default>");
        let mut rhs = Value::Null;
        let mut parent = Type::Unknown(0);
        let rhs_type = self.parse_operators(hint, &mut rhs, &mut parent, 0);
        self.lexer = saved;
        (rhs, rhs_type)
    }

    /// The checked half of @FR-N-Cast (`N-Cast?` in `types.md`): the value if it fits,
    /// else the type's null — never a fault, never a wrapped value.
    /// @PLN25 DN4 `(N-Cast?)` — lower `e as τ?` (narrowing, not provably-fit) to a
    /// range guard: bind `e` once, yield it if it lies in τ's range, else the
    /// integer null. Pure parse-time desugar over existing ops (`OpLeInt`,
    /// `OpConvIntFromNull`, `if`) — no new runtime op, no runtime error. `src_tp`
    /// is `e`'s type (the temp's slot); the result type is the nullable `τ`.
    pub(crate) fn dn4_checked_cast(&mut self, code: &mut Value, tp: &Type, src_tp: &Type) -> Type {
        let Type::Integer(spec) = tp else {
            return tp.clone();
        };
        // `spec.max` is a u32 and the bound is compared at `integer` (i64) width, so it
        // must be widened, never truncated: `u32`'s max is 4_294_967_294, and `as i32`
        // wrapped that to -2.  The emitted guard became `v <= -2`, which no legal value
        // satisfies, so EVERY `x as u32?` on a runtime operand returned null — silently,
        // because the idiomatic `?? 0` then supplied a plausible zero.  A constant operand
        // hid it by const-folding before this runs.  u8/u16/i8/i16/i32 were unaffected:
        // their maxima are the only ones that fit i32.
        let min = spec.min;
        let max = i64::from(spec.max);
        // Result type is `Optional(τ)` — the narrow target τ, marked nullable. As a Rust
        // VALUE this is carried full-width (`rust_type(Optional(Integer))` is `i64`, so the
        // else-branch's integer null sentinel fits and the value is not truncated); the
        // narrow width is a domain fact used at the store/field seam, which packs it. This
        // preserves τ so the checked cast composes downstream — e.g. `e as u8?` returned into
        // a `u8?` slot is an exact type match, not an implicit narrow of a bare integer.
        // (Earlier this dropped to the full source integer to dodge a native `as u8` on the
        // tail that turned `i64::MIN` into 0 and destroyed the null; that hazard is now fixed
        // at the `rust_type` seam, so the narrow τ is preserved.)
        let res_tp = Type::optional(tp.clone());
        let tmp = self.create_unique("_dn4", src_tp);
        let set = v_set(tmp, code.clone());
        let cond_lo = self.cl("OpLeInt", &[Value::Int(min), Value::Var(tmp)]);
        let cond_hi = self.cl("OpLeInt", &[Value::Var(tmp), int_literal(max)]);
        let null_lo = self.cl("OpConvIntFromNull", &[]);
        let null_hi = self.cl("OpConvIntFromNull", &[]);
        let inner = v_if(cond_hi, Value::Var(tmp), null_hi);
        let outer = v_if(cond_lo, inner, null_lo);
        *code = v_block(vec![set, outer], res_tp.clone(), "dn4cast");
        res_tp
    }

    /// The constant integer value of `code`, if it is one (literal or const-foldable).
    pub(crate) fn const_int(&self, code: &Value) -> Option<i64> {
        match code.unspan() {
            Value::Int(n) => Some(i64::from(*n)),
            Value::Long(n) => Some(*n),
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Int(n)) => Some(i64::from(n)),
                Some(Value::Long(n)) => Some(n),
                _ => None,
            },
        }
    }

    /// If the operand is provably in `[0, M]` — a non-negative constant, or an
    /// integer type with `min >= 0` — return `M`. Used by `&` range-tracking.
    fn nonneg_bound(&self, ty: &Type, code: &Value) -> Option<u32> {
        if let Some(c) = self.const_int(code)
            && (0..=i64::from(u32::MAX)).contains(&c)
        {
            return Some(u32::try_from(c).unwrap_or(u32::MAX));
        }
        if let Type::Integer(s) = ty
            && s.min >= 0
        {
            return Some(s.max);
        }
        None
    }

    #[allow(clippy::too_many_arguments)]
    /// @PLN25 DN3 — is a division/mod DIVISOR provably non-zero (so `a / v` cannot fault)?
    /// True for a constant non-zero literal, a var proven non-zero by an enclosing `if v != 0`
    /// guard (`divisor_nonzero`), or (@PLN102) any expression that const-folds to a non-zero number
    /// — a named constant like `SFX_RATE`, or a cast of one like `RATE as single` (both inline to a
    /// literal that `const_eval` reduces). This closes the gap where a direct `x / 600.0` was
    /// non-null but `x / NAMED_CONST` spuriously typed `τ?`. Anything else can be zero → `τ?`.
    fn divisor_provably_nonzero(&self, divisor: &Value) -> bool {
        match divisor.unspan() {
            Value::Int(n) => *n != 0,
            Value::Long(n) => *n != 0,
            // @PLN102 Phase 3 — a constant non-zero float / single divisor is provably safe too
            // (`1.0 / 2.0` stays non-null). Only consulted for float `/` under LOFT_NULLFLOW; for
            // integer division these arms never match, so the default path is unchanged.
            Value::Float(f) => *f != 0.0,
            Value::Single(f) => *f != 0.0,
            Value::Var(v) => self.divisor_nonzero.contains(v),
            other => match crate::const_eval::const_eval(other, &self.data) {
                Some(Value::Int(n)) => n != 0,
                Some(Value::Long(n)) => n != 0,
                Some(Value::Float(f)) => f != 0.0,
                Some(Value::Single(f)) => f != 0.0,
                _ => false,
            },
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn handle_operator(
        &mut self,
        var_tp: &Type,
        code: &mut Value,
        parent_tp: &mut Type,
        precedence: usize,
        ctp: &mut Type,
        operator: &str,
        op_pos: &Position,
    ) -> Option<Type> {
        // The checked-narrow target is only for a `??` that DIRECTLY discharges the
        // cast (`e as N ?? d`).  Any other operator between the cast and a later `??`
        // invalidates it — clear so a stale target can't mis-narrow an unrelated `??`.
        if operator != "??" {
            self.dn4_checked_narrow = None;
        }
        // @F2 — ?? null-coalescing operator (incl. `?? return`)
        if operator == "??" {
            self.handle_null_coalesce(var_tp, code, parent_tp, precedence, ctp, op_pos);
        // @F5 — type conversions: explicit `as` cast (+ implicit / format-only OpConv*)
        } else if operator == "as" {
            self.expr_not_null = false;
            if let Some(tps) = self.lexer.has_identifier() {
                // Parse the target WITHOUT the postfix-`?` consumer, then detect the
                // `?` here — `as τ` and `as τ?` are otherwise indistinguishable (the
                // narrow aliases all carry `not_null:false`).
                let Some(mut tp) = self.parse_type_inner(u32::MAX, &tps, false) else {
                    diagnostic!(self.lexer, Level::Error, "Expect type");
                    return Some(Type::Null);
                };
                // @PLN131 step 4 — where a `?` would go to make this cast checked, captured
                // HERE because it is the only moment the parser knows it: the lookahead has
                // not moved past the type yet, and by the time either cast diagnostic fires
                // the cursor has drifted beyond the statement terminator. Both fixes spell
                // `as τ?`, and an edit that cannot be placed must not be spelled at all.
                let type_end = {
                    let p = self.lexer.peek_pos();
                    (p.line, p.pos)
                };
                let nullable_cast = self.lexer.has_token("?");
                // @PLN25 DN4/DN5 — a scalar cast target has a DOMAIN: its integer value
                // RANGE, and (when it is a plain non-null scalar) that domain EXCLUDES null.
                // A value fits `as τ` (no `?`) only if it lies in the domain on BOTH
                // dimensions.  `null` is simply the reserved out-of-domain element — not a
                // special case — so the range fit (DN4) and the nullness fit (DN5) are ONE
                // containment test.  Peel `Optional` first so the RANGE check sees the
                // underlying integer: a nullable source otherwise slips past
                // `is_narrowing_int` entirely, laundering BOTH dimensions
                // (`integer? as u8` skipped the range check AND the null check).
                //
                // @PLAN48 P2: an explicit `as <narrow-int>` is the sanctioned way to narrow
                // `integer` → `i32`/`u8`/…, so the implicit-narrowing diagnostic in `convert`
                // does NOT fire on an explicit cast.  `as τ?` is the single CHECKED form: it
                // lowers to a range guard yielding the value or the null sentinel, and (below)
                // types as `Optional<τ>` so the laundering stays closed downstream.
                let src_base = ctp.base().clone();
                let src_may_be_null = matches!(ctp, Type::Optional(_) | Type::Null);
                let narrowing = Self::is_narrowing_int(&src_base, &tp);
                if !self.first_pass
                    && crate::keys::pln25_dn1_enabled()
                    && src_may_be_null
                    && !nullable_cast
                    && Self::is_non_null_scalar(&tp)
                {
                    // DN5 — the nullness dimension: a possibly-null value cast to a non-null
                    // scalar.  (Heap targets stay nullable, so `null as SomeStruct` is legal
                    // and never reaches here — `is_non_null_scalar` is scalar-only.)
                    if matches!(ctp, Type::Null) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "cannot cast `null` to the non-null `{tps}` — cast to `{tps}?` \
                             (a typed null), or use a real value",
                        );
                    } else {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "cannot cast a possibly-null `{}` to the non-null `{tps}` — it may \
                             be null; use `as {tps}?` for a checked cast (value or null), or \
                             discharge first with `?` (the type's default) or `?? <default>`",
                            ctp.name(&self.data),
                        );
                    }
                    // Keep `tp` (the non-null target) as the result to bound the cascade.
                } else if narrowing {
                    // @PLN25 DN4 (F5 cutover — UNCONDITIONAL, no opt-out): a narrowing cast
                    // whose value is NOT provably in range is a fit violation, exactly like
                    // DN5's nullness dimension (its sibling, also unconditional). The old
                    // `LOFT_NO_DN4` escape reverted to a SILENT 8-byte width-tag
                    // (`400 as u8 == 400`) — the very truncation the model eliminates — so it
                    // is retired. `as τ` errors (use `as τ?`); `as τ?` is the checked cast.
                    if !self.first_pass && !self.int_value_fits(code, &tp) {
                        // A bare `e as N` immediately left of `??` is a CHECKED cast: the
                        // `??` supplies the out-of-range fallback, so treat it like `e as N?`
                        // rather than rejecting it (the diagnostic already advertised `?? d`).
                        let coalesced = !nullable_cast && self.lexer.peek_token("??");
                        if nullable_cast || coalesced {
                            // Remember the narrow target so the discharging `??` types its
                            // result as `N` (build_null_coalesce_default); the cast BLOCK
                            // itself stays `integer` to keep the null sentinel at full width.
                            self.dn4_checked_narrow = Some(tp.clone());
                            tp = self.dn4_checked_cast(code, &tp, &src_base);
                        } else {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "narrowing cast from {} to {tps} may not fit at runtime; \
                                 use `{tps}?` for a checked cast (value or null), or guard \
                                 the value (`?? d`, mask, or an `if` range check)",
                                self.int_type_name(&src_base),
                            );
                        }
                    }
                    // else: in range, or DN4 off — accept as a width-tag (value
                    // stays in the 8-byte slot), the pre-DN4 behaviour.
                } else {
                    // @PLN102 arc-E E2 (B) — a float→integer cast of a STATICALLY
                    // out-of-range constant is a compile error, mirroring the
                    // narrowing-int DN4 rule above. The bare cast ASSERTS the value
                    // fits; a constant the developer can see does not fit is a bug,
                    // not a null. The checked forms (`as τ?`, `?? d`) stay the
                    // sanctioned escape — they yield the null sentinel / fallback —
                    // so gate on neither being present. Dynamic floats stay C80-null
                    // at runtime, unchanged. The range mirrors `cast_int_from_single`
                    // /`cast_int_from_float` in fill.rs: NaN or outside [-2^63, 2^63).
                    if !self.first_pass
                        && !nullable_cast
                        && matches!(tp.base(), Type::Integer(_))
                        && !self.lexer.peek_token("??")
                    {
                        let cf = match crate::const_eval::const_eval(code.unspan(), &self.data) {
                            Some(Value::Float(f)) => Some(f),
                            Some(Value::Single(f)) => Some(f64::from(f)),
                            _ => None,
                        };
                        if let Some(f) = cf
                            && (f.is_nan()
                                || !(-9_223_372_036_854_775_808.0_f64
                                    ..9_223_372_036_854_775_808.0_f64)
                                    .contains(&f))
                        {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                code = "cast-constant-out-of-range",
                                "the constant {f} is out of range for `{tps}` — a bare cast \
                                 asserts the value fits",
                            );
                            // @PLN131 — the ALWAYS-sound fix leads, and the better idiom does
                            // not. `as τ?` makes the expression nullable, which a target
                            // declared non-null rejects — and the parser cannot see that
                            // target, because applying the fix changes what pass 1 INFERS,
                            // so only a re-parse knows. Ranking is on soundness first and
                            // teaching only as a tiebreak between fixes that both hold.
                            self.lexer.fix_last(crate::diagnostics::Fix {
                                kind: crate::diagnostics::FixKind::Conditional,
                                title: "give the cast a fallback: `?? <default>`".to_string(),
                                condition: Some(format!(
                                    "`<default>` is the right value when {f} does not fit \
                                     `{tps}`"
                                )),
                                edit: None,
                                concept: "null coalescing",
                                concept_ref: "@F2",
                            });
                            self.lexer.fix_last(crate::diagnostics::Fix {
                                kind: crate::diagnostics::FixKind::Conditional,
                                title: "make the cast checked".to_string(),
                                condition: Some(
                                    "the result may be nullable here — if the target is \
                                     declared non-null (`x: \u{3c4}`), widen it or take the \
                                     fix above instead"
                                        .to_string(),
                                ),
                                edit: Some(crate::diagnostics::Edit {
                                    line: type_end.0,
                                    col: type_end.1,
                                    len: 0,
                                    text: "?".to_string(),
                                }),
                                concept: "checked cast",
                                concept_ref: "@F5",
                            });
                        }
                    }
                    // @PLN99 Arc C — clear the owned-conversion signal, then let `convert`
                    // set it iff it dispatches an allocating user conversion (`fn OpConvTFromS`).
                    self.conv_owned_result = None;
                    // @PLN102 — a nullable SCALAR source (`float?`/`single?`/`integer?`) casts by its
                    // BASE: the null rides in-band (NaN is the float null, C90; the reserved sentinel
                    // for integers) and the cast op propagates it (`NaN as integer` = null), so the
                    // per-type `OpCast` is keyed on the base, not the `Optional`. Peel it here so the
                    // checked `float? as integer?` resolves instead of reporting "Unknown cast"; the
                    // result re-wraps `τ?` below (nullable_cast). A BARE `float? as integer` never
                    // reaches this arm — DN5 rejects it above — so this only enables the checked form.
                    let cast_src: &Type = if src_may_be_null && Self::is_non_null_scalar(&src_base)
                    {
                        &src_base
                    } else {
                        ctp
                    };
                    // An explicit cast is not an implicit store: `convert` serves both, and
                    // the widened `integer` -> `i32` test (loft#931) must not fire on the
                    // cast this diagnostic prescribes as its own cure.
                    let outer_cast = self.in_explicit_cast;
                    self.in_explicit_cast = true;
                    let converted =
                        self.convert(code, cast_src, &tp) || self.cast(code, cast_src, &tp);
                    self.in_explicit_cast = outer_cast;
                    if !converted {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Unknown cast from {} to {tps}",
                            &ctp.name(&self.data),
                        );
                    }
                }
                // Post-2c: remember the cast target alias so `f += x as i32`
                // can narrow the file-serialisation width.  Only stored when
                // the alias carries a `size(N)` annotation; otherwise
                // `u32::MAX` = "no alias info".
                let alias_nr = self.data.def_nr(&tps);
                if alias_nr != u32::MAX && self.data.forced_size(alias_nr).is_some() {
                    self.last_cast_alias = alias_nr;
                }
                let mut rt = tp;
                // @PLN25 — `as τ?` yields `Optional<τ>`: the checked form's domain includes
                // null, so downstream sees a nullable and `(N-Store)` keeps the hole closed
                // (a later `z: τ = (e as τ?)` still requires a discharge).  `base()` guards
                // against a double-wrap from `dn4_checked_cast`.
                if (nullable_cast || self.dn4_checked_narrow.is_some())
                    && crate::keys::pln25_dn1_enabled()
                    && Self::is_non_null_scalar(rt.base())
                {
                    rt = Type::optional(rt.base().clone());
                }
                // A text→numeric PARSE is fit-failing: unparseable input yields the null sentinel
                // at runtime (C80, no trap). Two regimes:
                //  · @PLN25 DN3 (default / OFF): the result auto-TYPES `τ?` (`s as integer` is
                //    `integer?`; discharge with `?? d` or a `τ?` slot).
                //  · @FR-N-Cast — a bare `as τ` ASSERTS the value fits; a parse cannot be
                //    asserted, which is @FR-N-Parse folded in: `text as τ` is refused bare and
                //    written `as τ?`, the checked form (N-Cast?, lowered by `implicit_checked_narrow`'s
                //    explicit twin below).
                //  · @PLN102 Phase 4 (N-Cast, LOFT_NULLFLOW): a cast `as τ` is an ASSERTION, and a
                //    parse can't be proven, so a BARE `text as τ` is a compile error directing to
                //    the checked `as τ?` (value or null) or the assert-or-default `as τ ?? d`. A
                //    `?? d` immediately after IS that form — allowed, and typed `τ?` so the `??`
                //    discharges it (mirrors the DN4 narrowing `coalesced` rule above).
                if !nullable_cast
                    && crate::keys::pln25_dn1_enabled()
                    && matches!(ctp, Type::Text(_))
                    && matches!(rt.base(), Type::Integer(_) | Type::Float | Type::Single)
                {
                    if crate::keys::ncast_asserts() && !self.lexer.peek_token("??") {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                code = "text-parse-may-fail",
                                "a text parse `as {tps}` may fail, and a bare cast asserts \
                                 it cannot",
                            );
                            // @PLN131 — three ways out, ranked on SOUNDNESS first. The two
                            // discharging forms hold wherever this diagnostic fires,
                            // measured in both shapes; the checked cast does not, because
                            // `as τ?` makes the expression nullable and a target declared
                            // non-null rejects that. The parser cannot see the target —
                            // applying the fix changes what pass 1 INFERS, so only a
                            // re-parse knows — which is why it ships as a condition the
                            // author can check in a second rather than as a rewrite that
                            // works three times in four.
                            self.lexer.fix_last(crate::diagnostics::Fix {
                                kind: crate::diagnostics::FixKind::Conditional,
                                title: "give the parse a fallback: `?? <default>`".to_string(),
                                condition: Some(
                                    "`<default>` is the right value when the text does not \
                                     parse"
                                        .to_string(),
                                ),
                                edit: None,
                                concept: "null coalescing",
                                concept_ref: "@F2",
                            });
                            self.lexer.fix_last(crate::diagnostics::Fix {
                                kind: crate::diagnostics::FixKind::Conditional,
                                title: format!("fall back to the type's default: `(… as {tps}?)?`"),
                                condition: Some(format!(
                                    "an unparseable text becoming the `{tps}` default is the \
                                     result you want"
                                )),
                                edit: None,
                                concept: "default fallback",
                                concept_ref: "@F96",
                            });
                            self.lexer.fix_last(crate::diagnostics::Fix {
                                kind: crate::diagnostics::FixKind::Conditional,
                                title: "make the cast checked".to_string(),
                                condition: Some(
                                    "the result may be nullable here — if the target is \
                                     declared non-null (`x: \u{3c4}`), widen it or take one of \
                                     the fixes above instead"
                                        .to_string(),
                                ),
                                edit: Some(crate::diagnostics::Edit {
                                    line: type_end.0,
                                    col: type_end.1,
                                    len: 0,
                                    text: "?".to_string(),
                                }),
                                concept: "checked cast",
                                concept_ref: "@F5",
                            });
                        }
                        // Keep `rt` non-null (the asserted target) to bound the cascade.
                    } else {
                        rt = Type::optional(rt.base().clone());
                    }
                }
                if let Some(owned) = self.conv_owned_result.take() {
                    // @PLN99 Arc C — an allocating user conversion produced a FRESH owned
                    // store.  Use the conversion fn's real return type (Owned deps) and do
                    // NOT graft the source's deps: the result is not a view of the source, so
                    // grafting would mark it a borrow and leak the new store at scope end.
                    rt = owned;
                } else {
                    for d in ctp.depend() {
                        rt = rt.depending(d);
                    }
                }
                // #254: set the current type and fall through to `None` rather
                // than returning — `as` sits at the top precedence level, so
                // letting the Pratt loop continue lets a chained cast
                // (`x as integer as float`) parse left-associatively instead of
                // stranding the second `as` against an expected `;`.
                *ctp = rt;
            } else {
                diagnostic!(self.lexer, Level::Error, "Expect type after as");
            }
        } else if operator == "or" || operator == "||" {
            self.expr_not_null = false;
            self.boolean_operator(code, ctp, precedence, true);
            *ctp = Type::Boolean;
        } else if operator == "and" || operator == "&&" {
            self.expr_not_null = false;
            self.boolean_operator(code, ctp, precedence, false);
            *ctp = Type::Boolean;
        } else if operator == "=="
            || operator == "!="
            || operator == "<"
            || operator == "<="
            || operator == ">"
            || operator == ">="
        {
            let lhs_not_null = self.expr_not_null;
            let lhs_not_null_name = self.expr_not_null_name.clone();
            // Same @PLN102 case-E soundness gate as the `??` lint: the operand type is
            // the authority.  An `Optional` LHS can be null (a fault op nulled a
            // not-null input), so the `== null` check is NOT always-constant.
            let lhs_nullable = matches!(ctp, Type::Optional(_));
            self.expr_not_null = false;
            let mut second_code = Value::Null;
            let tp = parent_tp.clone();
            *parent_tp = ctp.clone();
            let second_pos = self.lexer.peek_pos().clone();
            let second_type =
                self.parse_operators(var_tp, &mut second_code, parent_tp, precedence + 1);
            self.known_var_or_type(&second_code, &second_pos);
            if !self.first_pass && (operator == "==" || operator == "!=") {
                if second_type == Type::Null
                    && lhs_not_null
                    && !lhs_nullable
                    && !self.call_declares_nullable(code)
                {
                    let always = if operator == "==" { "false" } else { "true" };
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        code = "redundant-null-check",
                        "Redundant null check — '{lhs_not_null_name}' is 'not null', so this is {always} unless a null reached the slot anyway (an overflow, a NaN, or an out-of-range read)",
                    );
                    // CONDITIONAL, not mechanical.  A non-null slot CAN observably hold null:
                    // `formal/types.md` C85 writes the reserved sentinel there on an integer
                    // overflow (and names a `float` holding `NaN` as the parallel), and C80
                    // answers an out-of-range read with null while the index expression stays
                    // typed non-null.  So `s.a == null` on a non-null `integer` field is TRUE
                    // after `s.a = big * big`, and deleting it removes the one test that
                    // catches that (loft#1297).
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Conditional,
                        title: "delete the check".to_string(),
                        condition: Some(
                            "no overflow, NaN or out-of-range read can have reached this slot"
                                .to_string(),
                        ),
                        edit: None,
                        concept: "nullable values",
                        concept_ref: "@F1",
                    });
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Conditional,
                        title: "compare the VALUE instead (`x == 0`)".to_string(),
                        condition: Some(
                            "you meant to test what the value IS, not whether it is present"
                                .to_string(),
                        ),
                        edit: None,
                        concept: "nullable values",
                        concept_ref: "@F1",
                    });
                } else if *ctp == Type::Null
                    && self.expr_not_null
                    && !matches!(second_type, Type::Optional(_))
                    && !self.call_declares_nullable(&second_code)
                {
                    let always = if operator == "==" { "false" } else { "true" };
                    diagnostic!(
                        self.lexer,
                        Level::Warning,
                        code = "redundant-null-check",
                        "Redundant null check — '{}' is 'not null', so this is {always} unless a null reached the slot anyway (an overflow, a NaN, or an out-of-range read)",
                        self.expr_not_null_name,
                    );
                    // Conditional for the reason the mirrored arm above gives (loft#1297).
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Conditional,
                        title: "delete the check".to_string(),
                        condition: Some(
                            "no overflow, NaN or out-of-range read can have reached this slot"
                                .to_string(),
                        ),
                        edit: None,
                        concept: "nullable values",
                        concept_ref: "@F1",
                    });
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Conditional,
                        title: "compare the VALUE instead (`x == 0`)".to_string(),
                        condition: Some(
                            "you meant to test what the value IS, not whether it is present"
                                .to_string(),
                        ),
                        edit: None,
                        concept: "nullable values",
                        concept_ref: "@F1",
                    });
                }
            }
            self.expr_not_null = false;
            // Match on the BASE type so a nullable vector (`vector<u8>? == null`, e.g.
            // a `read_bytes`/`list_dir` result — @PLN102 H4) is caught, not only a bare
            // `Type::Vector` — the same @PLN99 A5 gap the ref/enum null cases fixed.
            // loft#917 — every COLLECTION kind, not just `vector`.  A keyed field is the
            // same 4-byte slot and now carries the same absent marker, but `hash<K[k]>? ==
            // null` fell past this test to `OpEqRef` — a DbRef comparison against
            // `DbRef::NULL`, which a slot POINTER can never equal, so it answered false
            // however the field was written.  `is_collection_type` is the shared list.
            let vec_null = (operator == "==" || operator == "!=")
                && ((Self::is_collection_type(ctp.base()) && second_type == Type::Null)
                    || (*ctp == Type::Null && Self::is_collection_type(second_type.base())));
            // A float/single null is the NaN sentinel, and NaN compares unequal to
            // everything (including itself), so `f == null` can't go through OpEq —
            // it would always be false.  Test validity instead: convert(float, bool)
            // is `!is_nan` (= non-null), so `== null` is its negation.
            // @PLN25: peel `Optional(τ)` — a `float?`/`single?` shares its base's NaN-sentinel
            // null. Without the peel `float? == null` skipped this validity test and fell to
            // OpEqFloat on the NaN sentinel (NaN != everything) → ALWAYS false (a null read as
            // "present"). `Type::Null` itself is never wrapped, so the Null sides stay bare.
            let float_null = (operator == "==" || operator == "!=")
                && ((matches!(ctp.base(), Type::Float | Type::Single)
                    && second_type == Type::Null)
                    || (*ctp == Type::Null
                        && matches!(second_type.base(), Type::Float | Type::Single)));
            // A nullable enum variable holds the reference null sentinel
            // (`store_nr==u16::MAX`), exactly like a struct reference.  Its `== null`
            // must test that sentinel via OpEqRef — the default path reads the
            // discriminant (`OpGetEnum`), which derefs the absent record and OOB-crashes.
            // `Optional(<value enum>)` is admitted beside the bare spelling: it is how a
            // nullable enum FIELD and a nullable enum PARAMETER arrive, and leaving it to
            // the generic `==` lowering is what let a field never set — discriminant 0,
            // which the renderer shows as `null` — answer `false` to `== null`.  Widened
            // only for the value enum, because `null_test` answers every shape that
            // passes this gate and returns `None` for the others, and a `None` here
            // leaves the operand in place while retyping it boolean.
            let enum_null = (operator == "==" || operator == "!=")
                && (((matches!(*ctp, Type::Enum(_, _, _)) || self.is_value_enum(ctp))
                    && second_type == Type::Null)
                    || (*ctp == Type::Null
                        && (matches!(second_type, Type::Enum(_, _, _))
                            || self.is_value_enum(&second_type))));
            // @PLN99 A5 — a nullable STRUCT-reference VARIABLE (`s: DT? = null`,
            // typed `Optional(Reference)`) holds the reference null sentinel
            // (`store_nr==u16::MAX`, from OpNullRefSentinel), like a nullable enum variable.
            // Its `== null` must test that sentinel via OpRefIsNull.  Without this case it fell
            // to the generic `==`, which lowered to
            // `OpEqBool(OpConvBoolFromRef(s), OpConvBoolFromNull())` — but OpConvBoolFromRef is
            // `rec != 0` (is-NON-null, 0/1) while OpConvBoolFromNull is the 255 bool sentinel,
            // so the two NEVER compare equal → `s == null` was ALWAYS false (a live `main`
            // bug: `??` and `{s}` saw null but `== null` disagreed).
            // Gate on `Optional`, NOT bare `Reference`: a hash/collection LOOKUP result is a
            // bare `Type::Reference` whose miss is `rec==0` (not the store_nr sentinel) and is
            // handled correctly by the existing path — OpRefIsNull would misread it (p285).
            // INLINE nullable struct fields are `Type::Enum(__nullable<…>)` (enum_null path).
            let opt_ref_l =
                matches!(*ctp, Type::Optional(_)) && matches!(ctp.base(), Type::Reference(_, _));
            let opt_ref_r = matches!(second_type, Type::Optional(_))
                && matches!(second_type.base(), Type::Reference(_, _));
            let ref_null = (operator == "==" || operator == "!=")
                && ((opt_ref_l && second_type == Type::Null) || (*ctp == Type::Null && opt_ref_r));
            // loft#1065 — a nullable STRUCT-enum (`Shape?`) is the third shape that
            // reaches here, and it was the one with no arm: `s == null` was refused
            // outright ("No matching operator") while the bare `Shape` beside it worked.
            // It is carried as a `DbRef` and holds the same reference sentinel a nullable
            // struct reference does, so it wants the same test.  The `enum_null` comment
            // above sets the precondition for widening a gate here — that `null_test`
            // actually answers the shape — and it now does: its enum arm reads through
            // `base()`, so `Shape?` asks what `Shape` asks.
            //
            // loft#1071 — an INLINE operand (a field, a vector element) is admitted too.
            // It was excluded while its WRITE still emitted a record copy into a
            // four-byte slot, because answering then would have traded a refusal for a
            // silent wrong answer; with the write storing pointer `0` and `null_test`
            // reading `rec == 0` for such a slot, the answer is right and the exclusion
            // would only keep a working shape refused.
            let opt_senum = |tp: &Type| {
                matches!(tp, Type::Optional(_)) && matches!(tp.base(), Type::Enum(_, true, _))
            };
            let opt_senum_l = opt_senum(ctp);
            let opt_senum_r = opt_senum(&second_type);
            let senum_null = (operator == "==" || operator == "!=")
                && ((opt_senum_l && second_type == Type::Null)
                    || (*ctp == Type::Null && opt_senum_r));
            // @PLN102 pre-freeze — a boolean and an integer are NOT comparable with `==`/`!=`.
            // The old path coerced the integer to boolean by "is non-null", so `true == 0` was
            // TRUE and `true == 2` was TRUE (nonsense) — while `bool < int` already errored.
            // Reject `==`/`!=` too, so the whole comparison family is consistent.  (`null` is
            // Type::Null, not Integer, so `bool == null` / `bool? == null` are unaffected.)
            if !self.first_pass
                && matches!(operator, "==" | "!=")
                && ((matches!(ctp.base(), Type::Boolean)
                    && matches!(second_type.base(), Type::Integer(_)))
                    || (matches!(ctp.base(), Type::Integer(_))
                        && matches!(second_type.base(), Type::Boolean)))
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot compare a boolean and an integer with `{operator}` — a boolean is \
                     true/false/null, not 0/1; they are different types (convert one explicitly \
                     if that is really what you mean)"
                );
            }
            // loft#1020 — the operand is still a TYPE VARIABLE, so which null test it
            // wants is not decidable here.  Inside a template `T?` is
            // `Optional(Reference(<type var>))` and the placeholder is a STRUCT, so
            // `ref_null` below would win and the monomorph would keep `OpRefIsNull` over
            // whatever `T` became — a 12-byte DbRef read out of an 8-byte `integer` slot
            // (interpreter panic, `--native` E0610 on `.store_nr` of an `i64`).
            //
            // Stamp the site instead and let `rewrite_generic_type_defaults` re-ask
            // `null_test` once `T` is real, exactly as loft#1016 does for the DEFAULT of
            // the same declaration.  The block's `result` is the operand's type, which
            // substitution rewrites to the concrete one; a nested generic re-marks and
            // stays deferred.
            let tv_null = (operator == "==" || operator == "!=")
                && ((self.is_type_var_operand(ctp) && second_type == Type::Null)
                    || (*ctp == Type::Null && self.is_type_var_operand(&second_type)));
            if tv_null {
                // Marked on BOTH passes, unlike every branch below — they emit ops, which
                // is what needs a resolved second pass, while this only WRAPS the operand.
                //
                // loft#1023: the second pass re-parses in source order, so a call
                // instantiates a generic declared BELOW it before that template's own
                // pass-2 body exists, and the monomorph keeps whatever pass 1 left.  A
                // marker emitted on pass 2 alone was therefore absent from exactly those
                // monomorphs, and the body handed back the raw operand where a boolean
                // belonged: `--native` refused the program and the interpreter read the
                // slot as whatever it held.  Marking on pass 1 too makes the template's
                // body carry the site whichever pass a monomorph is derived from.
                let (n_code, n_tp) = if *ctp == Type::Null {
                    (second_code, second_type.clone())
                } else {
                    (code.clone(), ctp.clone())
                };
                *code = v_block(
                    vec![n_code],
                    n_tp,
                    if operator == "==" {
                        Self::TV_NULLTEST_EQ
                    } else {
                        Self::TV_NULLTEST_NE
                    },
                );
                *ctp = Type::Boolean;
            } else if vec_null || float_null || enum_null || ref_null || senum_null {
                if !self.first_pass {
                    let (n_code, n_tp) = if *ctp == Type::Null {
                        (second_code, second_type.clone())
                    } else {
                        (code.clone(), ctp.clone())
                    };
                    if let Some(test) = self.null_test(n_code, &n_tp, operator == "!=") {
                        *code = test;
                    }
                }
                *ctp = Type::Boolean;
            } else if operator == ">" {
                // loft#1151 — the SWAP is a resolution detail; the refusal must still name the
                // operator the author wrote.
                *ctp = self.call_op_as(
                    code,
                    "<",
                    ">",
                    &[second_code, code.clone()],
                    &[second_type, ctp.clone()],
                );
            } else if operator == ">=" {
                *ctp = self.call_op_as(
                    code,
                    "<=",
                    ">=",
                    &[second_code, code.clone()],
                    &[second_type, ctp.clone()],
                );
            } else {
                *ctp = self.call_op(
                    code,
                    operator,
                    &[code.clone(), second_code],
                    &[ctp.clone(), second_type],
                );
            }
            *parent_tp = tp;
        } else {
            self.expr_not_null = false;
            let mut second_code = Value::Null;
            let second_pos = self.lexer.peek_pos().clone();
            // Enforces @FR-G-Assoc — this is the ONE place associativity is decided, since
            // the level table (`parser/mod.rs::OPERATORS`) carries precedence only.
            //
            // `**` is RIGHT-associative — `2 ** 3 ** 2` is `2 ** (3 ** 2)` = 512, matching
            // maths and most languages, so the maker never carries a surprise here.  Its RHS
            // therefore parses at the SAME precedence (it recurses into a following `**`),
            // where every other binary operator is left-associative (RHS at `precedence + 1`).
            let rhs_precedence = if operator == "**" {
                precedence
            } else {
                precedence + 1
            };
            let second_type =
                self.parse_operators(var_tp, &mut second_code, parent_tp, rhs_precedence);
            self.known_var_or_type(&second_code, &second_pos);
            if !self.first_pass
                && (operator == "/" || operator == "%")
                && (matches!(second_code, Value::Int(0)) || matches!(second_code, Value::Long(0)))
            {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    code = "divide-by-constant-zero",
                    "{} by constant zero — result is always null",
                    if operator == "/" {
                        "Division"
                    } else {
                        "Modulo"
                    }
                );
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Conditional,
                    title: "divide by a value that can be non-zero".to_string(),
                    condition: Some(
                        "the zero is a mistake — a deliberate null is better written as `null`"
                            .to_string(),
                    ),
                    edit: None,
                    concept: "arithmetic safety",
                    concept_ref: "@F38",
                });
            }
            // @FR-N-Arith — non-null integer operands give a non-null result typed by the
            // operands' RANGES; only a nullable operand (@FR-N-Prop) or a partial op
            // (@FR-N-Div) makes the result nullable.
            // @PLN25 (N-Arith) range-tracking — capture the operand bounds BEFORE
            // call_op consumes them, so the result range of `&`/`%` can be narrowed
            // (a masked/modded value becomes provably-fit for a later narrowing
            // cast, removing DN4's `(x & 255) as u8` friction).
            let (and_bound, mod_const, lhs_nonneg) = if matches!(operator, "&" | "%") {
                let and_bound = [
                    self.nonneg_bound(ctp, code),
                    self.nonneg_bound(&second_type, &second_code),
                ]
                .into_iter()
                .flatten()
                .min();
                let lhs_nonneg = matches!(ctp, Type::Integer(s) if s.min >= 0);
                (and_bound, self.const_int(&second_code), lhs_nonneg)
            } else {
                (None, None, false)
            };
            // @PLN25 DN3: integer `/` and `%` PRODUCE null at runtime on a zero divisor (the C80
            // sentinel), so the RESULT types `τ?` UNLESS the divisor is provably non-zero — a
            // constant non-zero literal (`a / 2`) or a var proven non-zero by an `if v != 0` guard
            // (the `divisor_nonzero` narrowing stack). Captured HERE, before `call_op` consumes
            // `second_code`; the `τ?` wrap is applied after the range-narrowing below. DN1-gated.
            // Makes the type carry the null so `(N-Store)` forces `?? d` / a `τ?` slot at the store.
            // @PLN102 Phase 3 (N-Domain) — float / single `/` and `%` also fault on a zero
            // divisor, so they type `τ?` too (like integer `/`). Gated on LOFT_NULLFLOW; the
            // `divisor_provably_nonzero` elision keeps `x / 2.0` non-null. Integer stays default-on.
            // @FR-N-Div and its float twin @FR-N-Domain: `/` and `%` type `τ?` because a zero
            // divisor yields the reserved null; a divisor provably non-zero keeps `τ`.
            let div_nullable = (operator == "/" || operator == "%")
                && (matches!(ctp.base(), Type::Integer(_))
                    || (crate::keys::ndomain_enabled()
                        && matches!(ctp.base(), Type::Float | Type::Single)))
                && !self.divisor_provably_nonzero(&second_code);
            // @PLN102 (N-Prop) Phase 2 — capture operand nullability BEFORE `call_op` consumes
            // the operand types. A nullable operand PROPAGATES through a value-preserving scalar
            // op: the runtime already carries the null sentinel / NaN through (`n+5`, `5-n`,
            // `abs(n)` on a null n all stay null). Gated on LOFT_NULLFLOW; the result is wrapped
            // `τ?` below (beside the div wrap). C85 stays the complement — non-null operands are
            // untouched here, so overflow of two non-null values still types non-null.
            // @FR-N-Prop: a nullable operand makes the result nullable — null PROPAGATES
            // through a value-preserving scalar op rather than being laundered by it.
            let operand_nullable = crate::keys::nprop_enabled()
                && (matches!(*ctp, Type::Optional(_)) || matches!(second_type, Type::Optional(_)));
            *ctp = self.call_op(
                code,
                operator,
                &[code.clone(), second_code],
                &[ctp.clone(), second_type],
            );
            // Tighten the result range — sound + conservative (never narrower than
            // the operation guarantees), and only ever a tightening of call_op's
            // range. `a & c` (c ≥ 0) ∈ [0, c]; `a % c` ∈ [-(|c|-1), |c|-1], or
            // [0, |c|-1] when `a` is non-negative.
            if let Type::Integer(s) = &*ctp {
                let narrowed = match operator {
                    "&" => and_bound.map(|m| (0i32, m)),
                    "%" => mod_const.filter(|c| *c != 0).map(|c| {
                        let m =
                            u32::try_from(c.unsigned_abs().saturating_sub(1)).unwrap_or(u32::MAX);
                        let lo = if lhs_nonneg {
                            0
                        } else {
                            -i32::try_from(m).unwrap_or(i32::MAX)
                        };
                        (lo, m)
                    }),
                    _ => None,
                };
                if let Some((nmin, nmax)) = narrowed
                    && nmin >= s.min
                    && nmax <= s.max
                {
                    *ctp = Type::Integer(IntegerSpec {
                        min: nmin,
                        max: nmax,
                        ..*s
                    });
                }
            }
            if div_nullable && crate::keys::pln25_dn1_enabled() {
                *ctp = Type::optional(ctp.clone());
            } else if operand_nullable
                && !matches!(*ctp, Type::Optional(_))
                && matches!(ctp.base(), Type::Integer(_) | Type::Float | Type::Single)
                && matches!(
                    operator,
                    "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "<<" | ">>"
                )
            {
                // @PLN102 (N-Prop): a nullable operand rode into a value-preserving scalar op —
                // the result is nullable too (the null value already flows through at runtime).
                *ctp = Type::optional(ctp.clone());
            }
            // Plan-07 phase 1, step 1.B.1 — wrap binary fault-prone
            // arithmetic ops in `Value::Span` so runtime errors
            // (div-by-zero, narrow overflow, signed-overflow panic
            // from `checked_long!`) can be reported with the
            // operator's source position.  Covers `+ - * / %` plus
            // shifts; comparisons and boolean ops never panic, so
            // they stay unwrapped (saves IR size).
            //
            // Walker discipline: every IR walker that pattern-matches
            // `Value::Call(...)` either calls `unspan()` first or
            // carries a `Value::Span(b)` arm (scopes.rs, intervals.rs,
            // slots.rs, slots_v2.rs, validate.rs, codegen.rs,
            // parser/mod.rs::substitute_type_in_value, generation/*).
            // @PLN102 arc-E E2 (B) — a shift by a STATICALLY-KNOWN amount
            // outside [0,63] is a compile error.  A runtime null is the
            // FALLBACK for a *dynamic* out-of-range shift (C80); it is not a
            // cover for a constant the developer can see is wrong.  Dynamic
            // shift amounts still read null at runtime, unchanged.
            // A bare `<<`/`>>` with a statically-out-of-range amount ASSERTS a
            // defined result; there is none, so it is a compile error. The `?? d`
            // fallback (there is no checked-shift form) stays the sanctioned escape
            // — mirroring the cast rule below — so skip the check when a `??`
            // follows. A *dynamic* out-of-range amount still reads null (C80/C85).
            if !self.first_pass && matches!(operator, "<<" | ">>") && !self.lexer.peek_token("??") {
                // Fold the shift amount: `const_int` catches `-1` (a `neg(1)`
                // node), `2+3`, a named constant — not just a bare `Int` literal.
                let amt = if let Value::Call(_, args) = code.unspan() {
                    args.get(1).and_then(|a| self.const_int(a))
                } else {
                    None
                };
                if let Some(amt) = amt
                    && !(0..=63).contains(&amt)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        code = "shift-amount-out-of-range",
                        "shift by {amt} is out of the valid range 0..=63 — a constant \
                         out-of-range shift has no defined result",
                    );
                    // @PLN131 — the in-range amount leads, and the teaching rule does NOT
                    // promote `??` over it here: the two are not equally sound.  A constant
                    // out-of-range shift is nearly always a wrong amount, so offering the
                    // escape first would teach an idiom by papering over a bug.
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Conditional,
                        title: "shift by an amount inside `0..=63`".to_string(),
                        condition: Some(format!("{amt} is not the amount you meant")),
                        edit: None,
                        concept: "shift operators",
                        concept_ref: "@F37",
                    });
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Conditional,
                        title: "give the shift a fallback: `?? <default>`".to_string(),
                        condition: Some(format!(
                            "shifting by {amt} is deliberate and `<default>` is the right \
                             result when it has none"
                        )),
                        edit: None,
                        concept: "null coalescing",
                        concept_ref: "@F2",
                    });
                }
            }
            if !self.first_pass && matches!(operator, "+" | "-" | "*" | "/" | "%" | "<<" | ">>") {
                let inner = std::mem::replace(code, Value::Null);
                *code = Value::with_span(op_pos.clone(), inner);
            }
            // The result of a binary arithmetic op is a *computed value*, not
            // a `not null` field read — `/` and `%` can yield null on
            // divide-by-zero, and parsing the RHS just above re-set
            // `expr_not_null` to reflect the RHS operand's field-nullness
            // (e.g. `len(v) / set.stride` where `stride` is `not null`).
            // Clear it so `a / b ?? default` is NOT flagged "Redundant null
            // coalescing" — companion to the vector-index clear in
            // `fields.rs` (a `??` after a fault-prone op is a real defense).
            self.expr_not_null = false;
            self.expr_not_null_name.clear();
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Plan-07 phase 4e.2 — undefended fault-site compile-time warning
// ---------------------------------------------------------------------------
//
// Walks a parsed function body and emits `Level::Warning` at each
// fault-prone op call (OpDivInt / OpRemInt / OpGetVector / OpVectorRef /
// OpTextCharacter) that survived the 4d.1 / 4d.2 / 4e.1 swap passes —
// i.e., sites with no recognised defense.  The four canonical safe
// patterns from `04-runtime-error-kinds.md § Easy-proof skip list` are
// recognised here and quietly suppressed; without the skip list the
// warning would fire on most well-written loft code and developers
// would silence it within a session, defeating its purpose.
//
// Skip patterns implemented (per the design):
//   1. Constant non-zero literal divisor for OpDivInt / OpRemInt.
//   2. Constant non-negative literal index for OpGetVector / OpVectorRef
//      / OpTextCharacter (the developer typed a literal — trust it; if
//      it overruns at runtime the runtime fault still fires).
//   3. Index is the iteration variable of an enclosing for-loop
//      (`for i in <range> { … v[i] … }` — the loop's bound is the
//      developer's contract).
//   4. (Implicitly) sites inside `if x != null` / `if x` / format-string
//      interpolations — those were already swapped to Nullable peers by
//      4d.2 / 4e.1 BEFORE this walker runs, so the raising form is
//      gone from the IR there.

#[derive(Default)]
struct WarnCtx {
    /// Variable slots currently active as for-loop iteration variables.
    /// When a fault op uses `Var(v)` as its index AND `v` ∈ this set, we
    /// skip the warning (skip pattern 3).
    iter_vars: std::collections::HashSet<u16>,
    /// `(idx_var, vec)` pairs proven safe by an enclosing
    /// `if idx_var < len(vec) { ... }` guard, where `vec` is a `VecKey`
    /// (a bare var OR a struct field read).  Pushed on entry to the
    /// `then` block, popped on exit.  Skip pattern 5.
    guarded_pairs: Vec<(u16, VecKey)>,
    /// Map from local-var slot → vec `VecKey` when the local was bound
    /// via `n = len(vec)`.  Skip pattern 5 with `if i < n { v[i] }` then
    /// becomes equivalent to `if i < len(v) { v[i] }` via the lookup.
    /// Populated when walking `Value::Set(local, Call(len, [vec]))`.
    len_captures: std::collections::HashMap<u16, VecKey>,
    /// Position of the innermost enclosing `Value::Span` — used as the
    /// fault site's source location when we emit a warning.
    last_pos: Option<Position>,
    /// @PLN46 W2 — true while walking an argument that is passed DIRECTLY to a
    /// `#null_safe` function: a fault-prone op that IS this argument tolerates
    /// null by the callee's contract, so it is not flagged.  Reset (to the called
    /// function's own null-safety) on entry to each nested call, so the
    /// suppression never reaches a fault op buried inside a non-null-safe callee.
    arg_to_null_safe_param: bool,
}

#[derive(Copy, Clone)]
enum FaultKind {
    Div,
    Rem,
    VectorIndex,
    TextIndex,
}

/// Identity of a "vector" a bounds guard can prove safe.  Either a bare
/// local/parameter (`Var`) or a struct field read
/// `OpGetField(Var(base), off, tp)`.  Skip-pattern 5 matches the guard's
/// vec against the indexing's vec by this key, so
/// `if i < len(self.v) { self.v[i] }` is proven exactly like the
/// bare-local `if i < len(loc) { loc[i] }` form.  (A struct vector-field
/// read COPIES post-@PLN85 #415, so the old `loc = self.v` alias trick is
/// gone and value-correct code indexes the field directly.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum VecKey {
    Var(u16),
    Field(u16, i32, i32),
}

/// `VecKey` of an expression used as a vector — the indexing's first arg
/// or a `len(...)` arg.  `Some` for a bare `Var` or an
/// `OpGetField(Var(base), Int(off), Int(tp))`; `None` otherwise.
pub(crate) fn vec_key(v: &Value, data: &Data) -> Option<VecKey> {
    match v.unspan() {
        Value::Var(n) => Some(VecKey::Var(*n)),
        Value::Call(def_nr, args) => {
            if data.def(*def_nr).original_name() != "GetField" || args.len() != 3 {
                return None;
            }
            let Value::Var(base) = args[0].unspan() else {
                return None;
            };
            let (Value::Int(off), Value::Int(tp)) = (args[1].unspan(), args[2].unspan()) else {
                return None;
            };
            Some(VecKey::Field(*base, *off, *tp))
        }
        _ => None,
    }
}

impl Parser {
    /// Plan-07 phase 4e.2 — entry point.  Called from `parse_function`
    /// AFTER `parse_code` (the body parse) AND after `vars.test_used` /
    /// `warn_upper_case_locals` (so the diagnostic ordering matches
    /// existing per-function passes).  Second-pass only — the first
    /// pass doesn't have the swap-pass results in place.
    ///
    /// Silenceable via `LOFT_NO_WARN_RUNTIME=1` env var.  Stdlib
    /// (`default/*.loft`) is exempt — its functions are
    /// language-internal trusted code (they implement the very
    /// fault-handling primitives the warning is meant to nudge users
    /// toward) and warning on them is noise.
    pub(crate) fn warn_undefended_fault_sites(&mut self, body: &Value) {
        if self.default {
            return;
        }
        if std::env::var("LOFT_NO_WARN_RUNTIME").is_ok_and(|v| v == "1" || v == "true") {
            return;
        }
        let mut ctx = WarnCtx::default();
        // Initialise `last_pos` to the function's own source position so
        // any fault inside the body that lacks a finer-grained
        // `Value::Span` wrapper attributes to the function (the
        // legitimate fallback) instead of leaking to the lexer's
        // current cursor — which is *past* the just-parsed body and
        // would otherwise point at the next function's start.
        let fn_pos = self.data.definitions[self.context as usize]
            .position
            .clone();
        ctx.last_pos = Some(fn_pos);
        self.walk_for_warnings(body, &mut ctx);
    }

    /// @PLN87 P3 (W4) — flag a `&` on a HEAP struct parameter that the body never
    /// REASSIGNS.  For a heap param, field mutation (`o.x = 9`) already propagates
    /// to the caller (reference-default); `&` only changes a WHOLE-BINDING
    /// reassignment (`o = X`) into a write-back.  So a `&Obj` that is only
    /// field-mutated or read gains nothing from the `&` — and it is not free: a
    /// `&`-reference is a DOUBLE-INDIRECT `RefVar` (every access dereferences twice),
    /// materially slower than a plain heap binding that aliases directly.  So the
    /// redundant `&` is a silent efficiency footgun, not just a style nit — hence ON
    /// by default.  Scalar `&` (always load-bearing) and never-read params (already
    /// covered by `test_used`) are not flagged.  Stdlib exempt; silenceable via
    /// `LOFT_NO_WARN_RUNTIME`.
    pub(crate) fn warn_redundant_amp(&mut self, body: &Value) {
        if self.default || self.context == u32::MAX {
            return;
        }
        // ON by default (P3.2): the redundant `&` costs real runtime (double-indirect
        // RefVar access), so users need to see it.  Silenceable with the runtime-warning
        // family switch when a `&` is kept deliberately (e.g. a codegen regression test
        // that exercises the RefVar path).
        if std::env::var("LOFT_NO_WARN_RUNTIME").is_ok_and(|v| v == "1" || v == "true") {
            return;
        }
        let attrs = self.data.def(self.context).attributes().to_vec();
        for a in &attrs {
            // `self` is the method-receiver convention — `&self` is idiomatic and
            // not the "&Object to mutate" misconception this lint targets, so it is
            // never flagged even when only field-mutated.  Hidden synthetic params
            // (return buffers) are not user `&` either.
            if a.hidden || a.name == "self" {
                continue;
            }
            let var = self.vars.var(&a.name);
            if var == u16::MAX
                || self.vars.uses(var) == 0
                || !matches!(self.vars.tp(var), Type::RefVar(inner) if matches!(**inner, Type::Reference(_, _)))
            {
                continue;
            }
            // A `&(…)` with a heap element is a reference to a `__tuple<…>` record, and the
            // advice's premise does not hold for it: a by-value TUPLE parameter is a copy whose
            // writes stay in the callee, so the `&` is what makes them land (tuples.md T-Ref).
            if let Type::RefVar(inner) = self.vars.tp(var)
                && let Type::Reference(d, _) = **inner
                && self.data.def(d).name().starts_with("__tuple<")
            {
                continue;
            }
            // A reassignment of the whole binding is what `&` is FOR, and it need not be
            // written here: a FORWARDER (`fn forward(b: &B) { replace_ref(b); }`) never
            // reassigns — its callee does — so the one shape where the `&` is carrying
            // someone else's write-back is exactly the shape a body-local walk reads as
            // redundant.  Taking the advice there silently LOSES the write-back, which makes
            // it the worst kind of false positive: it fired only on the correct spelling and
            // said nothing about the broken one (loft#1286).
            //
            // `callee_param_reassigns` is the interprocedural half, and it asks about
            // REASSIGNMENT rather than about writes — its sibling `callee_param_writes` would
            // also answer yes to a FIELD write, which is precisely the case this advice
            // exists to flag.
            let mut reassigned = false;
            let mut cache: std::collections::HashMap<u32, Vec<bool>> =
                std::collections::HashMap::new();
            body.walk(&mut |node| {
                if matches!(node, Value::Set(v, _) if *v == var) {
                    reassigned = true;
                }
            });
            if !reassigned {
                let data = &self.data;
                body.walk(&mut |node| {
                    if let Value::Call(fn_nr, args) = node.unspan()
                        && *data.def(*fn_nr).code() != Value::Null
                    {
                        let callee =
                            crate::parser::callee_param_reassigns(*fn_nr, data, &mut cache);
                        for (i, arg) in args.iter().enumerate() {
                            if i < callee.len()
                                && callee[i]
                                && matches!(arg.unspan(), Value::Var(v) if *v == var)
                            {
                                reassigned = true;
                            }
                        }
                    }
                });
            }
            if reassigned {
                continue;
            }
            self.lexer.to(self.vars.var_source(var));
            diagnostic!(
                self.lexer,
                Level::Advice,
                code = "slow-reference-parameter",
                "`&` on parameter `{}` only slows it down here — a `&`-reference is \
                 double-indirect on every access, and field mutation already propagates \
                 to the caller without it",
                a.name,
            );
            self.lexer.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "drop the `&`".to_string(),
                condition: Some("you never REASSIGN the whole binding (`x = …`), which is the one thing `&` is for".to_string()),
                edit: None,
                concept: "reference",
                concept_ref: "@F21",
            });
        }
    }

    fn walk_for_warnings(&mut self, code: &Value, ctx: &mut WarnCtx) {
        match code {
            Value::Span(boxed) => {
                let saved = ctx.last_pos.clone();
                ctx.last_pos = Some(boxed.0.clone());
                self.walk_for_warnings(&boxed.1, ctx);
                ctx.last_pos = saved;
            }
            Value::Call(def_nr, args) => {
                let name = self.data.def(*def_nr).original_name();
                let kind: Option<FaultKind> = match name.as_str() {
                    "DivInt" | "DivFloat" | "DivSingle" => Some(FaultKind::Div),
                    "RemInt" | "RemFloat" | "RemSingle" => Some(FaultKind::Rem),
                    "GetVector" | "VectorRef" => Some(FaultKind::VectorIndex),
                    "TextCharacter" => Some(FaultKind::TextIndex),
                    _ => None,
                };
                if let Some(kind) = kind
                    && !is_easy_proof(kind, args, ctx, &self.data)
                {
                    self.emit_undefended_warning(kind, ctx);
                }
                // @PLN46 W2 — arguments to a `#null_safe` function tolerate null;
                // a fault op that IS such an argument is suppressed.  Override the
                // flag to THIS callee's null-safety so a nested non-null-safe call
                // resets it (the suppression is direct-argument only).
                let saved = ctx.arg_to_null_safe_param;
                ctx.arg_to_null_safe_param = self.data.def(*def_nr).null_safe();
                for arg in args {
                    self.walk_for_warnings(arg, ctx);
                }
                ctx.arg_to_null_safe_param = saved;
            }
            Value::CallRef(_, args) => {
                for arg in args {
                    self.walk_for_warnings(arg, ctx);
                }
            }
            Value::Iter(_, init, step, body) => {
                // The Iter's `u16` is the iterator's INTERNAL var
                // (e.g., `i#index` / `range`), NOT the user-visible
                // loop var.  The user-visible loop var is set OUTSIDE
                // the Iter via `Loop { Set(loop_var, Iter(…)); body }`
                // — handled by the `Loop` arm's lookahead below.
                self.walk_for_warnings(init, ctx);
                self.walk_for_warnings(step, ctx);
                self.walk_for_warnings(body, ctx);
            }
            Value::Block(b) => {
                for child in &b.operators {
                    self.walk_for_warnings(child, ctx);
                }
            }
            Value::Loop(b) => {
                // Recognise the canonical for-loop shape that the
                // parser emits:
                //   Loop {
                //     Set(loop_var, Block { name: "Iter range" / "Iter …",
                //                           operators: [increment, break-check, yield] });
                //     Block { user body using loop_var };
                //   }
                // Marking `loop_var` as iter-bound for the whole
                // Loop's operators makes skip-pattern 3 fire on
                // `v[loop_var]` reads inside the body.
                //
                // We accept either:
                //  (a) the RHS Block is named with an "Iter " prefix
                //      (today: "Iter range" / "Iter " variants), or
                //  (b) the RHS subtree contains a `Value::Break` (the
                //      iter step signals end-of-iteration via break).
                // Both are robust to parser-name changes — (b) is the
                // semantic check, (a) is the fast-path.
                fn contains_break(v: &Value) -> bool {
                    v.any_node(&mut |n| matches!(n, Value::Break(_)))
                }
                let mut loop_vars_added: Vec<u16> = Vec::new();
                for child in &b.operators {
                    if let Value::Set(loop_var, src) = child.unspan() {
                        let is_iter_step = match src.unspan() {
                            Value::Iter(..) => true,
                            Value::Block(blk) => {
                                blk.name.starts_with("Iter ")
                                    || blk.operators.iter().any(contains_break)
                            }
                            _ => false,
                        };
                        if is_iter_step && ctx.iter_vars.insert(*loop_var) {
                            loop_vars_added.push(*loop_var);
                        }
                    }
                }
                for child in &b.operators {
                    self.walk_for_warnings(child, ctx);
                }
                for v in loop_vars_added {
                    ctx.iter_vars.remove(&v);
                }
            }
            Value::If(cond, then_b, else_b) => {
                self.walk_for_warnings(cond, ctx);
                // Skip pattern 5 — recognise `if idx < len(vec) { ... }` and
                // `if idx < n { ... }` (where `n` is a captured `len(vec)`),
                // and push (idx_var, vec_var) onto the guarded-pairs stack so
                // indexing inside `then_b` is treated as safe.  Also walks
                // AND-conjuncted conditions (`if a<len(u) and b<len(v) { ... }`)
                // and pushes each qualifying conjunct.
                let pushed = collect_guard_pairs(cond.unspan(), &self.data, &ctx.len_captures);
                for pair in &pushed {
                    ctx.guarded_pairs.push(*pair);
                }
                self.walk_for_warnings(then_b, ctx);
                for _ in &pushed {
                    ctx.guarded_pairs.pop();
                }
                self.walk_for_warnings(else_b, ctx);
            }
            Value::Set(local_var, src) => {
                // Skip-pattern 5 capture — `n = len(vec)` registers `n` as
                // "the length of vec" for later `if i < n { v[i] }` proofs.
                if let Some(vec_var) = len_capture_target(src.unspan(), &self.data) {
                    ctx.len_captures.insert(*local_var, vec_var);
                }
                self.walk_for_warnings(src, ctx);
            }
            Value::Return(src)
            | Value::Drop(src)
            | Value::Yield(src)
            | Value::TuplePut(_, _, src) => {
                self.walk_for_warnings(src, ctx);
            }
            Value::Tuple(items) | Value::Insert(items) | Value::Parallel(items) => {
                for child in items {
                    self.walk_for_warnings(child, ctx);
                }
            }
            // Other Value variants (Int / Long / Text / Var / FnRef /
            // Keys / Boolean / etc.) carry no nested fault-prone calls.
            _ => {}
        }
    }

    fn emit_undefended_warning(&mut self, kind: FaultKind, ctx: &WarnCtx) {
        // @PLN25 DN3: the fault ops now TYPE `τ?` — the type carries the null and `(N-Store)`
        // forces discharge, so the runtime-null warning is redundant under DN1 (and fired
        // inconsistently vs the `if b != 0` / `if i < len` narrowing). Division/mod flipped
        // first; the index ops (`v[i]`/`s[i]`) are now flipped too (F1a landed), so retire all four.
        if crate::keys::pln25_dn1_enabled()
            && matches!(
                kind,
                FaultKind::Div | FaultKind::Rem | FaultKind::VectorIndex | FaultKind::TextIndex
            )
        {
            return;
        }
        let msg = match kind {
            // @P368 — wording: not "integer" (the same `/` warning covers float
            // and single division too).
            FaultKind::Div => {
                "division may produce null on divide-by-zero with no defensive check; \
                 consider `a / b ?? 0` or wrap in `if b != 0 { ... }`"
            }
            FaultKind::Rem => {
                "modulus may produce null on divide-by-zero with no defensive check; \
                 consider `a % b ?? 0` or wrap in `if b != 0 { ... }`"
            }
            FaultKind::VectorIndex => {
                "`v[i]` may produce null on out-of-bounds with no defensive check; \
                 consider `v[i] ?? <fallback>`, `if i < len(v) { v[i] }`, \
                 or `x = v[i]; if x != null { ... }`"
            }
            FaultKind::TextIndex => {
                "`s[i]` may produce null on out-of-bounds with no defensive check; \
                 consider `s[i] ?? <fallback>`, `if i < len(s) { s[i] }`, \
                 or `x = s[i]; if x != null { ... }`"
            }
        };
        if let Some(pos) = &ctx.last_pos {
            self.lexer.pos_diagnostic(Level::Warning, pos, msg);
        } else {
            self.lexer.diagnostic(Level::Warning, msg);
        }
    }

    /// @PLN46 W3 — auto-set `#null_safe` (function-wide) when EVERY parameter is
    /// entry-guarded by a leading `if p == null { return … }`.  SOUND: an
    /// unguarded parameter leaves the flag unset, so its null is never silently
    /// suppressed at call sites — the plan's "explicit assertion only" rule, here
    /// the explicit guard.  Only SETS the flag, so a `#null_safe` annotation (W2)
    /// is preserved.  Runs on pass 2 after the body, so a forward-defined callee's
    /// flag is set before a caller's warning walk.
    pub(crate) fn infer_function_null_safe(&mut self, body: &Value) {
        if self.first_pass || self.data.def(self.context).null_safe() {
            return;
        }
        let params = self.vars.arguments();
        if params.is_empty() {
            return;
        }
        let guarded = leading_null_guards(&self.data, body);
        if params.iter().all(|p| guarded.contains(p)) {
            self.data.definitions[self.context as usize].null_safe = true;
        }
    }
}

/// @PLN46 W3 — the parameter slots guarded by LEADING `if p == null { return … }`
/// statements (line markers skipped; the first real non-guard ends the entry
/// region, so a recognised guard always precedes any use of its parameter).
fn leading_null_guards(data: &Data, body: &Value) -> std::collections::HashSet<u16> {
    let mut guarded = std::collections::HashSet::new();
    let Value::Block(b) = body.unspan() else {
        return guarded;
    };
    for stmt in &b.operators {
        if let Some(p) = null_guard_param(data, stmt) {
            guarded.insert(p);
        } else if !matches!(stmt.unspan(), Value::Line(_) | Value::Null) {
            break; // first real non-guard ends the entry region (markers are skipped)
        }
    }
    guarded
}

/// The parameter an entry guard `if p == null { return … }` protects, if `stmt`
/// is exactly that shape: an `OpEq*` of `Var(p)` against `null` (either operand
/// order), whose `then` branch returns.
fn null_guard_param(data: &Data, stmt: &Value) -> Option<u16> {
    let Value::If(cond, then, _) = stmt.unspan() else {
        return None;
    };
    if !contains_return(then) {
        return None;
    }
    let Value::Call(op, args) = cond.unspan() else {
        return None;
    };
    if args.len() != 2 || !data.def(*op).name().starts_with("OpEq") {
        return None;
    }
    // A comparison may convert the operand first — e.g. a `character` is compared
    // as int: `OpConvIntFromCharacter(Var(c))`.  Look through one `OpConv*` layer.
    let var_of = |v: &Value| -> Option<u16> {
        match v.unspan() {
            Value::Var(p) => Some(*p),
            Value::Call(c, a) if a.len() == 1 && data.def(*c).name().starts_with("OpConv") => {
                match a[0].unspan() {
                    Value::Var(p) => Some(*p),
                    _ => None,
                }
            }
            _ => None,
        }
    };
    if let Some(p) = var_of(&args[0])
        && is_null_literal(data, &args[1])
    {
        return Some(p);
    }
    if let Some(p) = var_of(&args[1])
        && is_null_literal(data, &args[0])
    {
        return Some(p);
    }
    None
}

/// `null` as it appears in a comparison: a bare `Null`, or a `…FromNull()` cast
/// (the parser converts `null` to the compared type, e.g. `OpConvTextFromNull()`).
fn is_null_literal(data: &Data, v: &Value) -> bool {
    match v.unspan() {
        Value::Null => true,
        Value::Call(op, args) if args.is_empty() => data.def(*op).name().ends_with("FromNull"),
        _ => false,
    }
}

/// Does `v` contain a `return`?  The guard must EXIT on null, not fall through to
/// a use of the (still-possibly-null) parameter.
fn contains_return(v: &Value) -> bool {
    let mut found = false;
    v.walk(&mut |n| {
        if matches!(n, Value::Return(_)) {
            found = true;
        }
    });
    found
}

/// Evaluate the easy-proof skip list against a fault-prone call's args.
/// Returns `true` when a skip pattern matches and the warning should
/// NOT fire.
fn is_easy_proof(kind: FaultKind, args: &[Value], ctx: &WarnCtx, data: &Data) -> bool {
    // @PLN46 W2 — this fault op is a direct argument to a `#null_safe` function,
    // which contracts to handle the possible null.  Skip pattern 6.
    if ctx.arg_to_null_safe_param {
        return true;
    }
    fn lit_int(v: &Value) -> Option<i64> {
        match v.unspan() {
            Value::Int(n) => Some(i64::from(*n)),
            Value::Long(n) => Some(*n),
            _ => None,
        }
    }
    // @P368 — a literal divisor is statically known; only a *non-zero* literal
    // makes divide-by-zero impossible.  Covers float / single literals too
    // (`x / 2.0`, `x / 0.75`), which `lit_int` missed — so the divide-by-zero
    // warning no longer fires on a statically-safe float division.
    //
    // @P368 follow-up — when the dividend is float / single and the divisor
    // is an integer literal (`x / 3` with `x: float`), the parser wraps the
    // literal in an `OpConvFloatFromInt` / `OpConvSingleFromInt` cast so the
    // types match.  Without seeing through that cast, `lit_nonzero` on the
    // outer Call returns None and the warning fires spuriously.  Add a
    // recursive look-through for the two widening casts that wrap integer
    // literals on the divisor path; OpConvFloatFromSingle (single→float)
    // doesn't apply because no single literal can wrap an integer literal.
    fn lit_nonzero(v: &Value, data: &Data) -> Option<bool> {
        match v.unspan() {
            Value::Int(n) => Some(*n != 0),
            Value::Long(n) => Some(*n != 0),
            Value::Float(f) => Some(*f != 0.0),
            Value::Single(f) => Some(*f != 0.0),
            Value::Call(def_nr, call_args) => {
                // `original_name()` strips the "Op" prefix, so the
                // names here are without it.
                let name = data.def(*def_nr).original_name();
                if (name == "ConvFloatFromInt" || name == "ConvSingleFromInt")
                    && call_args.len() == 1
                {
                    lit_nonzero(&call_args[0], data)
                } else {
                    None
                }
            }
            _ => None,
        }
    }
    match kind {
        FaultKind::Div | FaultKind::Rem => {
            // Skip pattern 1 — divisor is a non-zero literal (int or float).
            args.get(1)
                .and_then(|v| lit_nonzero(v, data))
                .unwrap_or(false)
        }
        FaultKind::VectorIndex | FaultKind::TextIndex => {
            // Index is the LAST arg in both `OpGetVector(coll, size, idx)`
            // and `OpVectorRef(coll, idx)` and `OpTextCharacter(text, idx)`.
            let Some(idx) = args.last() else {
                return false;
            };
            // Skip pattern 2 — non-negative literal index.
            if lit_int(idx).is_some_and(|n| n >= 0) {
                return true;
            }
            // Skip pattern 3 — index is an active for-loop iteration var.
            if let Value::Var(v) = idx.unspan()
                && ctx.iter_vars.contains(v)
            {
                return true;
            }
            // Skip pattern 4 — index is *loop-bounded arithmetic*: an
            // expression built only from active loop-iteration variables
            // and integer literals (e.g. `mul_k * 4 + mul_row`, `i / 4`,
            // `row * width + col` where every variable is a loop counter).
            // This generalises pattern 3 (a bare loop var) to the integer
            // arithmetic the counters participate in — the developer is
            // iterating within the loop's bounds, so the computed index is
            // as trustworthy as the bare counter.  An index that mixes in a
            // struct field, a parameter, or a call result is NOT bounded
            // here and still warns (the genuine "could be OOB from data"
            // case).
            if index_loop_bounded(idx, ctx, data) {
                return true;
            }
            // Skip pattern 5 — `if idx_var < len(vec_var) { ... v[idx_var] ... }`.
            // When an enclosing If's condition proves `idx < len(vec)`, the
            // walker has pushed `(idx_var, vec_var)` onto `guarded_pairs`.
            // Match the indexing's vec arg (first arg) + idx arg (last arg)
            // against any pushed pair; if both are bare `Var(_)` references
            // to a guarded pair, the index is safe.
            // Skip pattern 5 — `if idx_var < len(vec_var)` proves any
            // indexing `vec_var[expr]` where `expr` is loop/literal/guarded-var
            // arithmetic over the guard's idx_var.  Strip casts on the idx.
            if let Some(first) = args.first()
                && let Some(vk) = vec_key(first, data)
            {
                let unwrapped = unwrap_cond(idx, data);
                if let Value::Var(idx_var) = unwrapped
                    && ctx
                        .guarded_pairs
                        .iter()
                        .any(|(i, v)| i == idx_var && *v == vk)
                {
                    return true;
                }
            }
            false
        }
    }
}

/// Recognise `idx_var < len(vec_var)` (canonical bounds check).
/// Returns `Some((idx_var, vec_var))` on a match, `None` otherwise.
/// Skip pattern 5 in `is_easy_proof` consults the resulting pair via
/// `WarnCtx::guarded_pairs`.
/// Recognise `if idx_var < len(vec_var) { ... }` (or
/// `if idx_var < n { ... }` where `n` was captured by `n = len(vec_var)`),
/// returning `(idx_var, vec_var)` on a match.  Skip pattern 5's entry
/// point for the If walker — pushed onto `WarnCtx::guarded_pairs` for
/// the duration of the then-block.
fn guard_pair_with_ctx(
    v: &Value,
    data: &Data,
    captures: Option<&std::collections::HashMap<u16, VecKey>>,
) -> Option<(u16, VecKey)> {
    let inner = unwrap_cond(v, data);
    let Value::Call(def_nr, args) = inner else {
        return None;
    };
    let raw = data.def(*def_nr).original_name();
    let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
    if name != "LtInt" || args.len() != 2 {
        return None;
    }
    let Value::Var(idx_var) = args[0].unspan() else {
        return None;
    };
    // RHS can be either `len(<vec>)` inline (where `<vec>` is a bare var or
    // a struct field read) OR a bare Var captured earlier as
    // `<local> = len(<vec>)`.
    let rhs = args[1].unspan();
    let vec = match rhs {
        Value::Call(len_def, len_args) => {
            let len_raw = data.def(*len_def).original_name();
            let len_name = len_raw.strip_suffix("Nullable").unwrap_or(len_raw.as_str());
            if !matches!(len_name, "len" | "LengthVector") || len_args.len() != 1 {
                return None;
            }
            vec_key(&len_args[0], data)?
        }
        Value::Var(n) => {
            let caps = captures?;
            *caps.get(n)?
        }
        _ => return None,
    };
    Some((*idx_var, vec))
}

/// Collect every `(idx_var, vec_var)` pair that `cond` proves safe.
/// Handles both a single comparison and AND-conjuncted comparisons
/// like `if a < len(u) and b < len(v) { ... }`.  Caller pushes each
/// returned pair onto `ctx.guarded_pairs` for the duration of the
/// then-block, then pops them.
pub(crate) fn collect_guard_pairs(
    cond: &Value,
    data: &Data,
    captures: &std::collections::HashMap<u16, VecKey>,
) -> Vec<(u16, VecKey)> {
    let mut out = Vec::new();
    collect_guard_pairs_into(cond, data, captures, &mut out);
    out
}

fn collect_guard_pairs_into(
    cond: &Value,
    data: &Data,
    captures: &std::collections::HashMap<u16, VecKey>,
    out: &mut Vec<(u16, VecKey)>,
) {
    // Strip Conv* casts (handled by unwrap_cond).
    let inner = unwrap_cond(cond, data);
    // Loft's short-circuit `a and b` lowers to `if a then b else false`
    // — recurse into both arms so each conjunct contributes.  Match on
    // shape `If(left, right, Boolean(false))`.
    if let Value::If(left, right, else_v) = inner
        && matches!(else_v.unspan(), Value::Boolean(false))
    {
        collect_guard_pairs_into(left, data, captures, out);
        collect_guard_pairs_into(right, data, captures, out);
        return;
    }
    // Also recurse into explicit AND-call forms in case the lowering
    // changes / for `int & int`-style bool checks built on LandInt.
    if let Value::Call(def_nr, args) = inner {
        let raw = data.def(*def_nr).original_name();
        let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
        if matches!(name, "AndBool" | "And" | "LandInt") && args.len() == 2 {
            collect_guard_pairs_into(&args[0], data, captures, out);
            collect_guard_pairs_into(&args[1], data, captures, out);
            return;
        }
    }
    if let Some(pair) = guard_pair_with_ctx(inner, data, Some(captures)) {
        out.push(pair);
    }
}

/// True when `src` is a `len(<Var>)` call — used to recognise the
/// `<local> = len(<vec>)` capture pattern.  Returns the vec var-id.
fn len_capture_target(src: &Value, data: &Data) -> Option<VecKey> {
    let Value::Call(def_nr, args) = src.unspan() else {
        return None;
    };
    let raw = data.def(*def_nr).original_name();
    let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
    if !matches!(name, "len" | "LengthVector") || args.len() != 1 {
        return None;
    }
    vec_key(&args[0], data)
}

/// Strip `ConvBoolFromInt` / `ConvIntFromInt` casts from a condition
/// expression so the underlying comparison call is reachable.
fn unwrap_cond<'a>(v: &'a Value, data: &Data) -> &'a Value {
    let mut cur = v.unspan();
    loop {
        let Value::Call(def_nr, args) = cur else {
            return cur;
        };
        if args.len() != 1 {
            return cur;
        }
        let raw = data.def(*def_nr).original_name();
        if !raw.starts_with("Conv") {
            return cur;
        }
        cur = args[0].unspan();
    }
}

/// True when `v` is an index expression composed solely of active
/// loop-iteration variables (`ctx.iter_vars`), integer literals, and
/// integer arithmetic over them — i.e. the index is bounded by the same
/// loops that produce it.  See `is_easy_proof` skip-pattern 4.
fn index_loop_bounded(v: &Value, ctx: &WarnCtx, data: &Data) -> bool {
    match v.unspan() {
        Value::Int(_) | Value::Long(_) => true,
        Value::Var(n) => ctx.iter_vars.contains(n),
        Value::Call(def_nr, call_args) => {
            // `original_name()` strips the "Op" prefix; arithmetic also
            // appears in `*Nullable` form when an operand is nullable.
            let raw = data.def(*def_nr).original_name();
            let name = raw.strip_suffix("Nullable").unwrap_or(raw.as_str());
            // Integer arithmetic / bitwise ops ("MinInt" is subtraction —
            // loft spells minus "Min").  A result built from bounded
            // operands stays bounded.
            const ARITH: &[&str] = &[
                "AddInt",
                "MinInt",
                "MulInt",
                "DivInt",
                "RemInt",
                "AbsInt",
                "EorInt",
                "LandInt",
                "LorInt",
                "SLeftInt",
                "SRightInt",
            ];
            ARITH.contains(&name) && call_args.iter().all(|a| index_loop_bounded(a, ctx, data))
        }
        _ => false,
    }
}
