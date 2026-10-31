// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Filling a monomorph: re-resolving the template's calls against the instance's types,
//! substituting `[T ↦ C]` through its IR, re-pointing the type rows and formats it inherited,
//! and lowering the open field, object and set spellings the template parsed as
//! type-variable placeholders.

use super::{
    Data, DefType, ElemRead, Function, HashMap, Level, Parser, Type, Value, diagnostic_format,
    v_block, v_set,
};

impl Parser {
    /// Re-resolve a Call target: if the called function's first parameter references
    /// the type variable, look up the correct overload for the concrete type.
    pub(super) fn re_resolve_call(d_nr: u32, tv_nr: u32, concrete: &Type, data: &Data) -> u32 {
        if d_nr == u32::MAX || d_nr as usize >= data.definitions.len() {
            return d_nr;
        }
        let def = &data.definitions[d_nr as usize];
        if def.attributes().is_empty() {
            return d_nr;
        }
        // Check if any attribute's type references the type variable.
        let has_tv = def
            .attributes
            .iter()
            .any(|a| data.type_mentions(&a.typedef, tv_nr));
        if !has_tv {
            // Also check for Integer(0, tv_nr) patterns — operators sometimes encode
            // type info in the Integer bounds.
            return d_nr;
        }
        // Resolve the concrete first-arg type by substituting tv_nr in the attribute type.
        // @FR-N-Shape — a `τ?` has `τ`'s shape, and its operators and methods are `τ`'s: the
        // concrete `x < y` on two `integer?` IS `OpLtInt`.  Looked up wrapped, `integer?` found
        // nothing and the bound stub stayed in the instance — `smaller<T: Ordered>(a, b)` at
        // `integer?` answered its first argument on the interpreter and did not compile on
        // native, and `min_of` over a `vector<integer?>` read a corrupt reference.
        let concrete_arg =
            Self::substitute_type(def.attributes()[0].typedef.clone(), tv_nr, concrete)
                .base()
                .clone();
        // Extract the user-facing function name from the mangled definition name.
        // Mangled names: "t_<LEN><Type>_<name>" or "n_<name>" or operator names.
        let name = def.name();
        let fn_name = if name.starts_with("t_") {
            Data::split_key(name).map_or(name, |k| k.rest)
        } else if let Some(rest) = name.strip_prefix("n_") {
            rest
        } else {
            // Operator name — use as-is for find_fn.
            name
        };
        let mut resolved = data.find_fn(u16::MAX, fn_name, &concrete_arg);
        // `@FR-Op-Std`, C132 — a bound's operator is never a program's `fn Op…`: `==` falls to
        // the content comparison (`G-Sat-Eq`), and satisfaction refused the rest already.
        if resolved != u32::MAX && fn_name.starts_with("Op") && !data.backs_operator(resolved) {
            return d_nr;
        }
        // `formal/interfaces.md` `(G-Sat)` judges a bound against the SIGNATURE
        // `[Self ↦ C](p̄ -> R)` — the parameter list included — and `find_fn` takes a name and
        // a receiver and no arity.  `-` desugars to `OpMin` at BOTH arities, so an interface
        // declaring `op - (self: Self, other: Self)` resolved to the UNARY `OpMinSingleInt`
        // and the monomorph dropped the second operand: `diff(10, 3)` answered `-10`, on both
        // backends with no diagnostic (loft#1299).
        //
        // The stub's VISIBLE parameter count is the comparand — a stub carries hidden ones (a
        // return buffer), so its raw count is 3 where the operator takes 2.  Measured: the `+`
        // stub is 3 raw / 2 visible against `OpAddInt` (2) and the unary `-` stub is 2 raw / 1
        // visible against `OpMinSingleInt` (1); only binary `-` disagreed.
        //
        // Re-asked of the `possible` map rather than refused, because the right definition IS
        // there — `integer` has `OpMinInt` beside `OpMinSingleInt` — and it is the same list
        // `call_op` resolves a concrete `a - b` from, so the two paths cannot drift about
        // which definitions an operator has.  (Unresolving instead was tried and is worse: the
        // stub reaches runtime with no diagnostic to attach.)
        if resolved != u32::MAX {
            let want = def.attributes().iter().filter(|a| !a.hidden).count();
            // VISIBLE on both sides: a user operator returning a struct carries a hidden
            // return buffer, so its raw count matched the other arity's visible one and the
            // unary `OpMin(self: V) -> V` (raw 2) was kept for a binary `a - b` (loft#1794).
            if Self::visible_arity(data, resolved) != want
                && let Some(by_signature) =
                    data.possible_with_signature(fn_name, want, &concrete_arg)
            {
                resolved = by_signature;
            }
            // `@FR-F-Recv` — at ONE arity the incumbent is still only the member declared
            // first: over `OpMin(self: V, o: integer)` declared before `OpMin(self: V, o: V)`,
            // `diff<T: Subtractable>(a, b)` at `V` bound the integer member and passed it a
            // record (loft#1817).  The stub's own parameters, at this instance, name the member.
            let eq_stub = Data::is_bound_stub_for(def.name(), "OpEq", 2);
            if eq_stub || data.has_overload_set(fn_name) {
                let params: Vec<Type> = def
                    .attributes()
                    .iter()
                    .filter(|a| !a.hidden)
                    .map(|a| {
                        Self::substitute_type(a.typedef.clone(), tv_nr, concrete)
                            .base()
                            .clone()
                    })
                    .collect();
                // A template member fits any list at its variables, and nothing here can
                // instantiate it: the concrete member of the set is the one to bind (loft#1826).
                if data.def(resolved).def_type == DefType::Generic
                    || !data.params_fit(resolved, &params)
                {
                    if let Some(by_params) = data.overload_with_params(fn_name, &params) {
                        resolved = by_params;
                    } else if eq_stub {
                        // `(G-Sat-Eq)`: an `OpEq` that does not take `(Self, Self)` is not the
                        // type's own `==`, and `satisfaction_failures` admitted the type for
                        // its CONTENT comparison — which the caller lowers for a stub left as
                        // it is, as the concrete `a == b` does (loft#1818).
                        return d_nr;
                    }
                }
            }
        }
        // @PLN25 E2 — a bounded-generic method call whose receiver monomorphises to a synth
        // `__nullable<S>` (a nullable vector element, e.g. `for x in v: vector<T>` where
        // `T = IfItem` → `__nullable<IfItem>`, then `x.is_valid()`) must resolve to S's CONCRETE
        // method via the `Some` payload — the nullable enum itself has no methods, so `find_fn`
        // returns MAX and the parametric bound stub `t_1T_<m>` (emitted `todo!()` in native)
        // would leak to runtime (86).  Mirror the interface-satisfaction unwrap above: retry
        // against S.  Gate-off-inert (no `__nullable<` type exists).
        if resolved == u32::MAX
            && let Type::Enum(nd, true, _) = &concrete_arg
            && data.def(*nd).name().starts_with("__nullable<")
        {
            let some = data.variant_of(*nd, "Some");
            let pa = if some == u32::MAX {
                usize::MAX
            } else {
                data.attr(some, "payload")
            };
            if pa != usize::MAX
                && let Type::Reference(s, _) = data.attr_type(some, pa)
            {
                let s_type = data.def(s).returned().clone();
                resolved = data.find_fn(u16::MAX, fn_name, &s_type);
            }
        }
        // loft#813 — the receiver monomorphised to a struct-enum VARIANT (`x = AsA { … }`
        // types as `Reference(AsA)`, not as the enum), and the method is declared on the
        // parent ENUM.  `find_fn` looks for `t_3AsA_one`, which does not exist, so the
        // call stayed on the type PARAMETER: `--interpret` answered the return type's
        // empty value and `--native` emitted a `todo!("t_1S_one")` stub — an empty answer
        // indistinguishable from a real one, with no diagnostic.
        //
        // The bound itself was already satisfied against the enum, because
        // `check_satisfaction` reads `def(variant).returned()` and for a variant that IS
        // the parent enum type.  So the two halves disagreed: satisfaction said yes on
        // `Any`'s method while the body bound `S` to the variant.  This makes them agree.
        //
        // Retried only AFTER the variant's own lookup fails, which is what keeps a
        // per-variant impl winning over the enum's — the same precedence
        // `parser/fields.rs`'s method-on-parent-enum dispatch already gives the DIRECT
        // call path, so `c1(v)` now answers what `v.one()` answers.  Inert for a plain
        // struct or a bare enum receiver (neither is an `EnumValue`).
        if resolved == u32::MAX
            && let Type::Reference(vd, _) = &concrete_arg
            && data.def_type(*vd) == DefType::EnumValue
        {
            let enum_type = data.def(*vd).returned().clone();
            resolved = data.find_fn(u16::MAX, fn_name, &enum_type);
        }
        if resolved != u32::MAX && resolved != d_nr {
            resolved
        } else {
            d_nr
        }
    }

    /// loft#845 — make a monomorph's collection FORMAT agree with the value it formats.
    ///
    /// `"{v}"` on a collection dumps through the schema, so the emitted call carries a
    /// storage-row id rather than an element type. In a template that id is the row the
    /// parser could see — a vector over the TYPE VARIABLE's own storage — and substitution
    /// replaces types, not constants, so the monomorph kept it and walked a
    /// `vector<integer>` through the wrong schema: `{}` instead of `[1, 2, 3]`, silently,
    /// on both backends. A bound does not help; `Printable` gives the ELEMENT a `to_text`,
    /// while the dump needs the element's ROW.
    ///
    /// The row is re-derived from the VALUE's own (already substituted) type rather than
    /// from the type variable's binding. That is the difference between a rule and a guess:
    /// the first attempt recomputed the parametric row from `concrete` and matched on it,
    /// and it did not reproduce what the emitter had chosen — an integer literal's inferred
    /// range picks the NARROW element row, so "the row for T = integer" and "the row this
    /// variable actually has" were two different numbers. Asking the variable removes the
    /// second derivation entirely, and makes the pass a no-op wherever the row was already
    /// right, since the registry is name-keyed.
    ///
    /// Both op spellings appear because they are one op with two delivery targets
    /// (`__work_N` on the stack vs a heap text); `append_data` chooses between them by the
    /// accumulator's storage, long before any of this.
    /// loft#1032 — re-decide every `OpCoroutineNext` in a monomorph from the
    /// generator variable's now-concrete `iterator<τ>`.
    ///
    /// The operands pair a SIZE with a CHANNEL, and both are a function of the
    /// yielded type, so a template that lowered `for y in inner(v)` while `T` was
    /// still the type variable baked the DbRef channel and 12 bytes.  Substitution
    /// then retyped the loop variable to the concrete `T` and left that pairing
    /// behind — the same shape as loft#1016 (`x?`'s default), loft#1020
    /// (`x == null`) and loft#1028 (a `null` literal): an operation whose choice
    /// depends on `τ` decided before `τ` was known.  At `T = integer` the loop read
    /// a 12-byte DbRef out of an 8-byte slot and indexed the store off its end
    /// ("Store access out of bounds … the reference is corrupt"); at `T = text` or a
    /// struct the DbRef channel is the right answer anyway, which is why the
    /// existing generic corpus never saw it — the scalar axis `formal/interfaces.md`
    /// records as this doc's missing one.
    ///
    /// Reads the generator VARIABLE's type rather than pattern-matching the baked
    /// constant, so a genuinely non-parametric nested iterator in a generic body
    /// re-derives to what it already had, and only a stale pairing moves.
    pub(super) fn retarget_parametric_coroutine_next(&mut self, d_nr: u32) {
        let op = self.data.def_nr("OpCoroutineNext");
        if op == u32::MAX {
            return;
        }
        // Collect first — the walk borrows the IR while re-deriving needs the table.
        let mut fixes: Vec<(u16, i32, Vec<i32>)> = Vec::new();
        self.data.def(d_nr).code.walk(&mut |v| {
            if let Value::Call(d, args) = v
                && *d == op
                && args.len() >= 2
                && let Value::Var(g) = args[0].unspan()
                && let Type::Iterator(inner, _) = self.data.def(d_nr).variables.tp(*g).base()
            {
                let (value_size, kinds) = crate::coroutine_layout::next_operands(inner);
                let stale = matches!(&args[1], Value::Int(n) if *n != value_size)
                    || args.len() - 2 != kinds.len();
                if stale && !fixes.iter().any(|(v_nr, _, _)| *v_nr == *g) {
                    fixes.push((*g, value_size, kinds));
                }
            }
        });
        if fixes.is_empty() {
            return;
        }
        let mut code = self.data.definitions[d_nr as usize].code.clone();
        code.map_nodes(&mut |v| {
            if let Value::Call(d, args) = v
                && *d == op
                && args.len() >= 2
                && let Value::Var(g) = args[0].unspan()
                && let Some((_, value_size, kinds)) = fixes.iter().find(|(v_nr, _, _)| *v_nr == *g)
            {
                let gen_arg = args[0].clone();
                *args = vec![gen_arg, Value::Int(*value_size)];
                args.extend(kinds.iter().copied().map(Value::Int));
            }
        });
        self.data.definitions[d_nr as usize].code = code;
    }

    /// loft#1070 — re-point every TYPE ROW this monomorph inherited from the template.
    ///
    /// A schema id is a `const u16` ARGUMENT (`OpDatabase(v, db_tp)`,
    /// `OpCopyRecord(src, dst, tp)`, …), not a type — so type substitution, which rewrites
    /// the variable table and the type annotations, walks straight past it.  Any site the
    /// template lowered while the type was still `T` therefore keeps `__typevar_T`'s row,
    /// and the record it allocates has the type variable's layout: reading a field lands on
    /// another word.  Measured, `f<T>(x: T, c: boolean) -> T { if c { y: T = x; y } else …
    /// }` answered `4294967198` — `-98` read as an unsigned 32-bit word — for a `-7` it was
    /// handed, on both backends, with no diagnostic, and leaked a `__typevar_T` store per
    /// call.
    ///
    /// The invariant makes this total rather than a list of known sites: after substitution
    /// the type variable does not exist, so EVERY surviving reference to its row is stale by
    /// construction, and there is no case where keeping one is right.  The rows are found by
    /// the op's own declaration — an argument named `tp` or `…_tp` — so an op added later is
    /// covered without being enumerated here, and `size: const u16` (which is a width, not a
    /// row) is not swept up with them.
    ///
    /// Why the arm-local in that repro was RIGHT while the return wrapper was wrong: the
    /// local is declared in source and re-derives its row from the substituted variable
    /// table, whereas the compiler-built `materialized_view_return` block was constructed
    /// once, at template parse, from the type the arm had THEN.
    ///
    /// The type variable's own row is not the only one.  A SCALAR or `text` binding has a row
    /// too — the element row an append or a removal carries is `integer`'s or `text`'s — and
    /// skipping those bindings left `OpAppendVector` and `OpRemoveVector` naming
    /// `__typevar_T`, whose width is zero: a divide by zero at `integer` and a slide of the
    /// wrong width at `text` (loft#1536).  And a type BUILT over the variable (`vector<T>`)
    /// was registered with a row of its own at template parse, which is just as stale.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn retarget_parametric_type_rows(
        &mut self,
        d_nr: u32,
        bindings: &[(u32, Type)],
        tmpl_vars: &Function,
    ) {
        fn note(rows: &mut Vec<(i32, i32)>, stale: i32, fresh: i32) {
            let none = i32::from(u16::MAX);
            if stale != fresh
                && stale != none
                && fresh != none
                && !rows.iter().any(|(s, _)| *s == stale)
            {
                rows.push((stale, fresh));
            }
        }
        // stale row -> fresh row: one entry per bound type variable, then one per type in the
        // template's variable table that is built over one.
        let mut rows: Vec<(i32, i32)> = Vec::new();
        for (holder, bound_to) in bindings {
            let stale = i32::from(self.data.def(*holder).known_type());
            // A record or enum keeps its RECORD row, which is also its element row; any other
            // binding answers the element row every append and removal of it is lowered with.
            let fresh = match bound_to.base() {
                Type::Reference(d, _) | Type::Enum(d, _, _) => {
                    i32::from(self.data.def(*d).known_type())
                }
                other => self.append_elem_tp(other),
            };
            note(&mut rows, stale, fresh);
        }
        for v in 0..tmpl_vars.count() {
            let tmpl_tp = tmpl_vars.tp(v).clone();
            // loft#2156 — a vector backing's wrapper row: the rows the template baked for it
            // (`OpDatabase(__vdb, main_vector<K>)`) name the placeholder's layout.  The
            // instance's own table already holds the wrapper `fill_monomorph_body` re-derived
            // (`Self::instance_vector_wrapper`), at the same variable number.
            if let Type::Reference(stale_w, _) = tmpl_tp.base()
                && let Type::Reference(fresh_w, _) =
                    self.data.definitions[d_nr as usize].variables.tp(v).base()
                && stale_w != fresh_w
                && self.data.def(*stale_w).name().starts_with("main_vector<")
            {
                let stale = i32::from(self.data.def(*stale_w).known_type());
                let fresh = i32::from(self.data.def(*fresh_w).known_type());
                note(&mut rows, stale, fresh);
                continue;
            }
            let over_a_variable = bindings
                .iter()
                .any(|(h, _)| self.data.type_mentions(&tmpl_tp, *h));
            let is_a_variable = matches!(tmpl_tp.base(),
                Type::Reference(d, _) if bindings.iter().any(|(h, _)| h == d));
            if !over_a_variable || is_a_variable {
                continue;
            }
            let stale = i32::from(self.get_type(&tmpl_tp));
            let fresh = i32::from(self.get_type(&Self::substitute_all(tmpl_tp, bindings)));
            note(&mut rows, stale, fresh);
        }
        // @PLN165 D5 — the rows of the open instances this monomorph closes: none may survive
        // the rewrite below, or the instance would build a fieldless record where its twin
        // builds `Box<integer>` — loft#1536's class, made loud here.
        let open_rows: Vec<i32> = bindings
            .iter()
            .filter(|(h, _)| self.data.is_open_instance(*h))
            .map(|(h, _)| i32::from(self.data.def(*h).known_type()))
            .filter(|r| *r != i32::from(u16::MAX))
            .collect();
        if rows.is_empty() && open_rows.is_empty() {
            return;
        }
        // Which ARGUMENT of each op is a type row, read off the op's declaration rather
        // than hard-coded — the same source `dispatch_call` fills them from.
        let mut row_arg: std::collections::HashMap<u32, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, def) in self.data.definitions.iter().enumerate() {
            if !def.name.starts_with("Op") {
                continue;
            }
            let at: Vec<usize> = def
                .attributes()
                .iter()
                .enumerate()
                .filter(|(_, a)| a.name == "tp" || a.name.ends_with("_tp"))
                .map(|(k, _)| k)
                .collect();
            if !at.is_empty() {
                row_arg.insert(i as u32, at);
            }
        }
        // `OpCopyRecord` carries two flag bits beside the row in its `tp`
        // ([`crate::keys::COPY_TP_MASK`]): the row is looked up with them masked off and
        // written back with them kept, or a flagged copy of a type built over the variable
        // kept the template's row.
        let copy_record = self.data.def_nr("OpCopyRecord");
        let mask = i32::from(crate::keys::COPY_TP_MASK);
        let mut code = self.data.definitions[d_nr as usize].code.clone();
        code.map_nodes(&mut |v| {
            if let Value::Call(d, args) = v
                && let Some(at) = row_arg.get(d)
            {
                for &k in at {
                    if let Some(Value::Int(row)) = args.get(k) {
                        let (bare, flags) = if *d == copy_record {
                            (row & mask, row & !mask)
                        } else {
                            (*row, 0)
                        };
                        if let Some((_, fresh)) = rows.iter().find(|(stale, _)| *stale == bare) {
                            args[k] = Value::Int(*fresh | flags);
                        }
                    }
                }
            }
        });
        let mut surviving: Option<i32> = None;
        code.map_nodes(&mut |v| {
            if let Value::Call(d, args) = v
                && let Some(at) = row_arg.get(d)
            {
                for &k in at {
                    if let Some(Value::Int(row)) = args.get(k) {
                        let bare = if *d == copy_record { row & mask } else { *row };
                        if open_rows.contains(&bare) {
                            surviving = Some(bare);
                        }
                    }
                }
            }
        });
        assert!(
            surviving.is_none(),
            "the instance `{}` still names an open instance's row ({}) after its rows were \
             retargeted — a template op that bakes a row was not remapped",
            self.data.def(d_nr).name(),
            surviving.unwrap_or(-1)
        );
        self.data.definitions[d_nr as usize].code = code;
    }

    /// loft#1365 — drop the tuple-member copy this instantiation must not carry.
    ///
    /// `tuples.md (T-Cons)` copies a HEAP member into a tuple literal and leaves a scalar one
    /// alone, and a record, a vector and a keyed collection are each copied by a DIFFERENT
    /// step.  Which one a `T`-typed member needs does not exist while the template is parsed
    /// — only per instantiation — and the template has to guess, because a type variable is
    /// spelled `Type::Reference` to its placeholder and so looks exactly like a record.  So
    /// the template emits the RECORD copy, the guess that is right whenever `T` binds a
    /// record, and this pass removes it wherever that guess does not fit what this monomorph
    /// bound.
    ///
    /// The test is the block's own contents against that type
    /// ([`Self::tuple_member_copy_shape_fits`]), never "was this member a type variable" —
    /// which matters because a generic body also builds tuples from CONCRETE members, whose
    /// copies are already right and must not be touched.  A vector member's copy fits a
    /// `Type::Vector` and is left alone; a record copy sitting on an integer or on a vector
    /// is the template's guess, and is unwrapped to the source it was built from
    /// ([`Self::tuple_member_copy_source`], the matcher the return path shares).
    ///
    /// Both wrong answers this replaces were measured, and they are opposite errors.
    /// Declining the copy in the template left a struct-bound `T` ALIASING the local where
    /// `(T-Cons)` and `binding.md (B-Copy)` both say copy (D-tup-9).  Emitting it
    /// unconditionally allocated a record with the type variable's own row — the layout
    /// escape the record-row guard refuses (loft#1070) — for a scalar binding and for a
    /// collection one alike, an ICE on both backends.
    ///
    /// Unwrapping the VALUE is only half of undoing the guess: the template also gave the
    /// tuple's element type the backing's dep, and a type that still names a backing whose
    /// copy is gone makes the element look owned by a variable nothing fills — the store the
    /// member actually holds is then freed by nobody.  So the dep goes with the copy, which
    /// is what leaves a collapsed member exactly as the pre-guess compiler had it.
    pub(super) fn collapse_parametric_tuple_member_copies(&mut self, d_nr: u32) {
        let mut code = self.data.definitions[d_nr as usize].code.clone();
        let this = &*self;
        // (backing the element depended on, the local the member now IS a view of — if any)
        let mut dropped: Vec<(u16, Option<u16>)> = Vec::new();
        code.map_nodes(&mut |v| {
            let Some(src) = this.tuple_member_copy_source(v) else {
                return;
            };
            let Value::Block(b) = v.unspan() else { return };
            if this.tuple_member_copy_shape_fits(&b.operators, &b.result) {
                return;
            }
            // The block's tail is the backing it filled, and that is the dep the element
            // type was given.  Unwrapped to a plain local, the member is that local's VIEW
            // (`(B-View)`): the dep moves to the local rather than vanishing, because an
            // element with no dep is read as OWNED and freed at scope exit — and here it is
            // the caller's store.
            let backing = b.operators.last().and_then(|o| match o.unspan() {
                Value::Var(x) => Some(*x),
                _ => None,
            });
            let view_of = match src.unspan() {
                Value::Var(x) => Some(*x),
                _ => None,
            };
            if let Some(backing) = backing {
                dropped.push((backing, view_of));
            }
            *v = src;
        });
        if dropped.is_empty() {
            return;
        }
        self.data.definitions[d_nr as usize].code = code;
        let vars = &mut self.data.definitions[d_nr as usize].variables;
        for v in 0..vars.count() {
            for (backing, view_of) in &dropped {
                vars.retarget_tuple_member_deps(v, &[*backing], *view_of);
            }
        }
    }

    /// Does the copy the TEMPLATE emitted already suit `tp`?
    ///
    /// Each member kind is copied by its own step, so the step present in the block names the
    /// kind the copy was built for: `OpCopyRecord` a record, `OpAppendVector` a vector,
    /// `OpReplaceKeyed` a keyed collection.  Comparing that against the type this monomorph
    /// bound is what tells a guess that happened to be right from one that has to be redone —
    /// and it asks the question of the block's own contents rather than of a flag set
    /// elsewhere, so a template that stops guessing cannot leave this stale.
    pub(super) fn tuple_member_copy_shape_fits(&self, ops: &[Value], tp: &Type) -> bool {
        let mut fits = false;
        for op in ops {
            match op.unspan() {
                Value::Call(d, _) => match self.data.def(*d).name() {
                    "OpCopyRecord" => {
                        fits |= matches!(tp.base(), Type::Reference(_, _) | Type::Enum(_, true, _));
                    }
                    "OpAppendVector" => fits |= matches!(tp.base(), Type::Vector(_, _)),
                    "OpReplaceKeyed" => {
                        fits |= matches!(
                            tp.base(),
                            Type::Sorted(_, _, _)
                                | Type::Index(_, _, _)
                                | Type::Hash(_, _, _)
                                | Type::Radix(_, _, _)
                                | Type::Trie(_, _, _)
                        );
                    }
                    _ => {}
                },
                Value::Block(inner) => {
                    fits |= self.tuple_member_copy_shape_fits(&inner.operators, tp)
                }
                Value::If(_, then, els) => {
                    fits |= self.tuple_member_copy_shape_fits(std::slice::from_ref(then), tp)
                        || self.tuple_member_copy_shape_fits(std::slice::from_ref(els), tp);
                }
                _ => {}
            }
        }
        fits
    }

    pub(super) fn retarget_parametric_vector_format(&mut self, d_nr: u32) {
        let ops: Vec<u32> = ["OpFormatDatabase", "OpFormatStackDatabase"]
            .iter()
            .map(|n| self.data.def_nr(n))
            .filter(|d| *d != u32::MAX)
            .collect();
        if ops.is_empty() {
            return;
        }
        // Collect first — re-deriving a row needs `&mut self` while the walk holds the IR.
        // Keyed on the ROW rather than on a traversal position: the two walkers treat
        // `Span` differently, so a positional pairing between them would be a coincidence.
        // A row is name-keyed on its content, so one stale row can only have come from one
        // element type, and the map is well defined.
        let mut want: Vec<(u16, Type)> = Vec::new();
        self.data.def(d_nr).code.walk(&mut |v| {
            if let Value::Call(d, args) = v
                && ops.contains(d)
                && args.len() == 4
                && let Value::Var(n) = args[1].unspan()
                && let Value::Int(row) = &args[2]
                && let Ok(stale) = u16::try_from(*row)
                && !want.iter().any(|(r, _)| *r == stale)
                && let Type::Vector(cont, _) = self.data.def(d_nr).variables.tp(*n).base()
            {
                want.push((stale, (**cont).clone()));
            }
        });
        let fixes: Vec<(u16, u16)> = want
            .into_iter()
            .map(|(stale, cont)| (stale, self.format_vector_row(&cont)))
            .filter(|(stale, fresh)| stale != fresh)
            .collect();
        if fixes.is_empty() {
            return;
        }
        let mut code = self.data.definitions[d_nr as usize].code.clone();
        code.map_nodes(&mut |v| {
            if let Value::Call(d, args) = v
                && ops.contains(d)
                && args.len() == 4
                && let Value::Int(row) = &args[2]
                && let Some((_, fresh)) = u16::try_from(*row)
                    .ok()
                    .and_then(|r| fixes.iter().find(|(stale, _)| *stale == r))
            {
                args[2] = Value::Int(i32::from(*fresh));
            }
        });
        self.data.definitions[d_nr as usize].code = code;
    }

    /// The template of `d` when `d` is an instance bound to a type VARIABLE — minted while
    /// another template's body was parsed (`i_1S_n_inner`) — else `u32::MAX`.  An instance
    /// at a real type is its own answer; one at a variable stands for its template.
    pub(super) fn placeholder_instance_template(&self, d: u32) -> u32 {
        if !self.data.is_placeholder_instance(d) {
            return u32::MAX;
        }
        let Some(key) = Data::split_key(self.data.def(d).name()) else {
            return u32::MAX;
        };
        let template = self.data.def_nr(key.rest);
        if template == u32::MAX || self.data.def_type(template) != DefType::Generic {
            return u32::MAX;
        }
        template
    }

    /// The half of [`re_resolve_call`] that has to CREATE rather than look up.
    ///
    /// `re_resolve_call` re-points a call whose target depends on the type variable at the
    /// concrete function, with `find_fn`. That is enough when the concrete function already
    /// exists — a user's `area(self: Box)` does. It cannot be enough when the target is
    /// another GENERIC, because that one's monomorph does not exist until somebody makes it,
    /// and a lookup only finds. So the lookup fell back to the template itself and the
    /// monomorph shipped a call to a definition nothing ever emits: native cannot find
    /// `n_inner`, and the interpreter enters a template whose frame does not match the call,
    /// losing it whole — no output, exit 0.
    ///
    /// A generic reached ONLY through another generic gets no other chance. Instantiation
    /// happens at a call site with a concrete argument type, and inside a template that
    /// argument is the type variable — so `fn outer<S>(s: S) { inner(s) }` is the one shape
    /// where nobody ever names `inner`'s concrete form. Adding any direct `inner(box)` call
    /// elsewhere in the program hid the bug, which is why it survived: the fixtures that
    /// exercise nested generics happen to call the inner one directly as well.
    ///
    /// The substitution has already replaced the type variable throughout, so the argument
    /// that was `S` is now `concrete` — which is exactly the instantiation to ask for.
    ///
    /// Called after `add_def` has registered THIS monomorph, so a self-recursive generic
    /// (`fn f<T>(t: T) { f(t) }`) finds itself in the `existing` check and stops, rather
    /// than instantiating forever.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn instantiate_nested_generics(&mut self, d_nr: u32, bindings: &[(u32, Type)]) {
        let concrete = &bindings
            .first()
            .map_or(Type::Unknown(0), |(_, b)| b.clone());
        // The nested call's own FIRST ARGUMENT type, not the outer binding.
        //
        // This used to instantiate every nested generic at `concrete` — the type the OUTER
        // monomorph bound its variable to.  That is right only when the nested generic's
        // first parameter is a bare `T`; with `vector<T>` — the ordinary spelling — the
        // callee was handed `integer` where its parameter reads `vector<T>`, resolution
        // answered `Unknown`, and the program was refused with *"Cannot resolve generic
        // type parameter from argument type"*.  A generic calling a generic is the whole
        // point of the feature, so the shape is not exotic.
        //
        // The argument's type comes from the monomorph's own variable table, which
        // `fill_monomorph_body` has already substituted, so it is the CONCRETE type the
        // call really passes.  Anything that is not a plain variable falls back to
        // `concrete`, which is what every caller got before.
        let mut targets: Vec<(u32, Type)> = Vec::new();
        // `(instance called, its template)` for a call the template aimed at an instance of
        // ANOTHER generic bound to one of its own type variables (`outer<S>` calling `inner(s)`
        // binds `inner`'s variable to `S`).  That instance is keyed by `S`, so no lookup by
        // the outer binding finds it; its call is aimed through the template it names.
        let mut via_template: Vec<(u32, u32)> = Vec::new();
        let mono_vars = self.data.def(d_nr).variables.clone();
        self.data.def(d_nr).code.walk(&mut |v| {
            if let Value::Call(d, args) = v
                && *d != u32::MAX
                && (*d as usize) < self.data.definitions.len()
                && !targets.iter().any(|(t, _)| t == d)
            {
                let template = if self.data.def(*d).def_type() == DefType::Generic {
                    *d
                } else {
                    self.placeholder_instance_template(*d)
                };
                if template == u32::MAX {
                    return;
                }
                if template != *d {
                    via_template.push((*d, template));
                }
                // A first argument that is not a plain variable takes the callee's first
                // parameter under this instance's bindings — the callee is the template or an
                // instance bound to this template's variables, so that is the argument's type
                // at a variable position.  The FIRST binding is only the last resort: with two
                // variables it is often the other one's (`inner(bs[i], a)` inside
                // `outer<A, B>` bound `inner` at `A`, and read a text as an integer).
                let from_param = self
                    .data
                    .def(*d)
                    .attributes
                    .iter()
                    .find(|a| !a.hidden)
                    .map(|a| Self::substitute_all(a.typedef.clone(), bindings))
                    .filter(|t| !t.is_unknown() && !self.data.mentions_type_var(t));
                let arg_tp = match args.first().map(Value::unspan) {
                    Some(Value::Var(a)) if *a < mono_vars.count() => mono_vars.tp(*a).clone(),
                    _ => from_param.unwrap_or_else(|| concrete.clone()),
                };
                targets.push((*d, arg_tp));
            }
        });
        // loft#1033 — a par WORKER is a generic too, and it does not ride in the IR as a
        // `Value::Call`: the parallel ops carry their worker as a d_nr INTEGER argument
        // (`parallel_for(input, elem_size, return_size, threads, fn_d_nr, …)`), so the walk
        // above cannot see it and a template's worker stayed the template. The interpreter
        // dispatches by d_nr and survived that; `--native` emits a direct call and died on
        // `cannot find function n_idf`, because a template emits no code.
        //
        // Recognised by TWO facts at once rather than by argument position: the enclosing
        // call is one of the `n_parallel_*` family, and the integer is a d_nr this parser
        // already recorded as a par worker (`par_worker_defs`) whose def is a template.
        // Either test alone could match an ordinary integer that happens to equal a d_nr;
        // together they cannot, and neither hardcodes the operand index, which is the
        // layout's own business at the build site.
        let par_workers: Vec<(u32, Type)> = {
            let mut found: Vec<(u32, Type)> = Vec::new();
            self.data.def(d_nr).code.walk(&mut |v| {
                if let Value::Call(d, args) = v
                    && *d != u32::MAX
                    && (*d as usize) < self.data.definitions.len()
                    && self.data.def(*d).name().starts_with("n_parallel_")
                {
                    for a in args {
                        if let Value::Int(w) = a.unspan()
                            && let Ok(w) = u32::try_from(*w)
                            && (w as usize) < self.data.definitions.len()
                            && self.par_worker_defs.contains(&w)
                            && self.data.def(w).def_type() == DefType::Generic
                            && !found.iter().any(|(t, _)| *t == w)
                        {
                            // The worker's first parameter takes the loop ELEMENT, so the
                            // type that resolves `T` is the element type of the collection
                            // this monomorph iterates — `concrete` for the ordinary
                            // `vector<T>` spelling, which is what every par worker here is.
                            found.push((w, concrete.clone()));
                        }
                    }
                }
            });
            found
        };
        targets.extend(par_workers);
        // Create the monomorphs FIRST, before the context swap below: instantiation
        // parses against `self.vars` / `self.context`, and it must see the ordinary
        // ones, not this monomorph's.
        let mut remap: HashMap<u32, u32> = HashMap::new();
        for (callee, arg_tp) in targets {
            let t = via_template
                .iter()
                .find(|(c, _)| *c == callee)
                .map_or(callee, |(_, tmpl)| *tmpl);
            // A free template is stored `n_<name>`; a METHOD template under its receiver's
            // key, `t_6vector_head` (loft#1539).  Both instantiate from the template itself.
            let key = self.data.def(t).name().to_string();
            let name = match key.strip_prefix("n_") {
                Some(free) => free.to_string(),
                None if key.starts_with("t_") => Self::method_spelling(&key),
                None => continue,
            };
            // The call's own first argument, and — for a variable named in a LATER parameter
            // (`D-Every-Var`) — the callee's parameters with this instance's bindings: the
            // callee is the template or an instance bound to this template's variables, so
            // its parameter types are the arguments' static types at every variable position.
            let mut arg_types = vec![arg_tp];
            arg_types.extend(
                self.data
                    .def(callee)
                    .attributes
                    .iter()
                    .filter(|a| !a.hidden)
                    .skip(1)
                    .map(|a| Self::substitute_all(a.typedef.clone(), bindings)),
            );
            let inst = self.instantiate_template(t, &name, &arg_types);
            if inst != u32::MAX && inst != t && inst != callee {
                remap.insert(callee, inst);
            }
        }
        // Repair the ARG LISTS too, and not only for the calls remapped above:
        // `re_resolve_call` retargets a type-variable-dependent call by LOOKUP during
        // the substitution, so a call in this body can already point at a monomorph
        // whose ABI grew — `promote_monomorph_text_return` gives a `-> text` callee a
        // hidden `&text` buffer — while still carrying the template's argument count.
        // That is the "Too few parameters on t_6Sqlite_dump (got 4, need 5)" ICE, and a
        // lookup-retargeted call reaches it without ever having been in `remap`. So the
        // set is every call this body makes that is short of its target's arity.
        let mut short_calls: crate::fxhash::FxHashSet<u32> = crate::fxhash::FxHashSet::default();
        self.data.def(d_nr).code.walk(&mut |v| {
            if let Value::Call(d, args) = v {
                let target = remap.get(d).copied().unwrap_or(*d);
                if target != u32::MAX
                    && (target as usize) < self.data.definitions.len()
                    && args.len() < self.data.attributes(target)
                {
                    short_calls.insert(target);
                }
            }
        });
        if crate::env_once!(std::env::var_os("LOFT_DBG_ACC").is_some()) {
            let names: Vec<String> = remap
                .iter()
                .map(|(a, b)| format!("{}->{}", self.data.def(*a).name(), self.data.def(*b).name()))
                .collect();
            eprintln!(
                "[acc-mono] nested in {}: remap={names:?} short={}",
                self.data.def(d_nr).name(),
                short_calls.len()
            );
        }
        if remap.is_empty() && short_calls.is_empty() {
            return;
        }
        let promoted = short_calls;
        // Swap this monomorph's parse context in — `add_defaults`, reached through
        // `patch_tret_call`, mints a work-text into `self.vars`, which must be THIS
        // function's table. Same swap `promote_monomorph_text_return` performs.
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = d_nr;
        let before: std::collections::HashSet<u16> = self.vars.work_texts().into_iter().collect();
        self.vars.sync_work_counters();
        let mut code =
            std::mem::replace(&mut self.data.definitions[d_nr as usize].code, Value::Null);
        Self::retarget_calls(&mut code, &remap);
        // …and the par-worker d_nr INTEGERS, which `retarget_calls` does not reach
        // (loft#1033). Same two-fact recognition as the collection walk above.
        if !remap.is_empty() {
            let par_names: std::collections::HashSet<u32> = remap.keys().copied().collect();
            code.map_nodes(&mut |v| {
                if let Value::Call(d, args) = v
                    && *d != u32::MAX
                    && (*d as usize) < self.data.definitions.len()
                    && self.data.def(*d).name().starts_with("n_parallel_")
                {
                    for a in args.iter_mut() {
                        if let Value::Int(w) = a
                            && let Ok(w32) = u32::try_from(*w)
                            && par_names.contains(&w32)
                            && let Some(fresh) = remap.get(&w32)
                        {
                            *a = Value::Int(*fresh as i32);
                        }
                    }
                }
            });
        }
        // A `-> text` callee was promoted to deliver through a hidden `&text` buffer
        // (`promote_monomorph_text_return`, inside the instantiation above), so the call
        // this body already carries is one argument short of the ABI it now targets —
        // the "Too few parameters on t_6Sqlite_inner (got 3, need 4)" ICE. This is the
        // same mismatch `patch_tret_callers` fixes for a directly-parsed caller, so it is
        // the same repair.
        self.patch_tret_call(&mut code, &promoted);
        // A minted retbuf is not declared at the top level, so `scopes::check` would
        // scope it to the arg block and free it there — before the callee fills it.
        // Replay the hoist `patch_tret_callers` does for exactly this reason.
        if let Value::Block(bl) = &mut code {
            for wt in self.vars.work_texts() {
                if !before.contains(&wt) {
                    bl.operators
                        .insert(0, v_set(wt, Value::Text(String::new())));
                }
            }
        }
        self.data.definitions[d_nr as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = saved_ctx;
    }

    /// Re-point every `Call` in the tree whose target appears in `remap`.
    pub(super) fn retarget_calls(v: &mut Value, remap: &HashMap<u32, u32>) {
        if let Value::Call(d, _) = v
            && let Some(&to) = remap.get(d)
        {
            *d = to;
        }
        v.for_each_child_mut(&mut |c| Self::retarget_calls(c, remap));
    }

    /// Substitute all occurrences of `Type::Reference(tv_nr, _)` with `concrete` in a type.
    /// Apply every `(holder → bound-to)` pair of a monomorph to one type, in order.
    ///
    /// The pairs are disjoint by construction — a type variable and each associated type
    /// are separate definitions — so the order only fixes which is tried first, never the
    /// result.
    pub(super) fn substitute_all(tp: Type, bindings: &[(u32, Type)]) -> Type {
        tp.substitute_all(bindings)
    }

    /// loft#2156 — a vector backing's wrapper type at this instance's bindings.
    ///
    /// The template mints a vector local's backing (`__vdb_N`) as `ref(main_vector<E>)`, a
    /// NAMED wrapper struct whose only field is `vector<E>`.  A reference to that struct does
    /// not mention the type variable — only its field does — so substitution walks past it
    /// and the instance kept `main_vector<K>`: its backing was built with the placeholder's
    /// zero-size element, and a deep copy of it (`OpCopyRecord` through that row) copied
    /// `length * 0` bytes.  `sort_by` answered a scrambled vector wherever the store swap
    /// that hides the copy is declined.  The wrapper is a function of its element, so the
    /// instance's wrapper is the one over the substituted element (@FR-G-Mono).
    ///
    /// `None` for every other type, and for a wrapper over a concrete element.
    pub(super) fn instance_vector_wrapper(
        &mut self,
        tp: &Type,
        bindings: &[(u32, Type)],
    ) -> Option<Type> {
        let Type::Reference(w, deps) = tp.base() else {
            return None;
        };
        if self.first_pass
            || !matches!(self.data.def_type(*w), DefType::Struct)
            || !self.data.def(*w).name().starts_with("main_vector<")
        {
            return None;
        }
        let Some(Type::Vector(elem, _)) = self
            .data
            .def(*w)
            .attributes()
            .first()
            .map(|a| a.typedef.base())
        else {
            return None;
        };
        if !bindings
            .iter()
            .any(|(h, _)| self.data.type_mentions(elem, *h))
        {
            return None;
        }
        let elem = Self::substitute_all((**elem).clone(), bindings);
        let fresh = self.data.vector_def(&mut self.lexer, &elem);
        self.vector_wrapper_known_type(fresh);
        let fresh = Type::Reference(fresh, deps.clone());
        // A nullable backing stays nullable.
        Some(match tp {
            Type::Optional(_) => Type::Optional(Box::new(fresh)),
            _ => fresh,
        })
    }

    /// `[T ↦ C]` over one type — @FR-G-Mono's *"applied throughout"*, for the signature
    /// half.  Its twin for the variable table is `Function::subst_type`.
    pub(super) fn substitute_type(tp: Type, tv_nr: u32, concrete: &Type) -> Type {
        // Every former descends through the keystone (`Type::substitute`), so `[T ↦ C]`
        // reaches a type variable wherever it sits.  The four arms that one home replaced
        // were added one defect at a time (Plan-17 for the tuple, #493 for the optional,
        // loft#1032 for the iterator), each the same omission a former further out.
        tp.substitute(tv_nr, concrete)
    }

    /// Recursively substitute types in a Value IR tree and re-resolve Call targets
    /// whose first parameter references the type variable.
    /// Walks a generic-template's IR and substitutes the type variable
    /// `tv_nr` with the concrete `concrete` type, both in variable types
    /// and in IR-shape decisions that depend on T's resolved shape.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn substitute_type_in_value(
        val: Value,
        tv_nr: u32,
        concrete: &Type,
        iter_stride: ElemRead,
        data: &Data,
    ) -> Value {
        match val {
            Value::Call(d, args) => {
                let new_args: Vec<_> = args
                    .into_iter()
                    .map(|a| Self::substitute_type_in_value(a, tv_nr, concrete, iter_stride, data))
                    .collect();
                // `@FR-Op-Bound` — `Ordered`'s `<` at a type whose order is its `operator
                // compare`: `a < b` is `a.compare(b) == Less`, one call (`@FR-Op-Order`).
                // `@FR-Op-Bound` — `Addable`'s `+`, `Subtractable`'s `-` and `Numeric`'s `*` at
                // such a type are its `operator plus` / `minus` / `times`, one call each.  The
                // stub is matched at its VISIBLE arity: a member answering a struct carries a
                // hidden return buffer, which the stub and the member both take, in place.
                if let Some(form) = [
                    ("OpAdd", 2),
                    ("OpMin", 2),
                    ("OpMul", 2),
                    ("OpDiv", 2),
                    ("OpRem", 2),
                    ("OpMin", 1),
                ]
                .into_iter()
                .find(|(op, n)| Data::is_bound_stub_for(data.def(d).name(), op, *n))
                .and_then(|(op, n)| Data::operator_form_of_member(op, n))
                    && let Some(m) = data.operator_member_with(
                        form,
                        &data
                            .visible_params(d)
                            .into_iter()
                            .map(|t| Self::substitute_type(t.clone(), tv_nr, concrete))
                            .collect::<Vec<_>>(),
                        Some(&Self::substitute_type(
                            data.def(d).returned().clone(),
                            tv_nr,
                            concrete,
                        )),
                    )
                    && data.def(m).attributes().len() == new_args.len()
                {
                    return Value::Call(m, new_args);
                }
                // The stub must be THIS type variable's: substituting `T` leaves a `<` over
                // another variable `K` alone, or `sort_by<T, K>` compared its keys with `T`'s
                // `operator compare` (loft#2269).
                let names_tv = |d: u32| {
                    data.visible_params(d).first().is_some_and(|t| {
                        Self::substitute_type((*t).clone(), tv_nr, concrete) != **t
                    })
                };
                if new_args.len() == 2
                    && Data::is_bound_stub_for(data.def(d).name(), "OpLt", 2)
                    && names_tv(d)
                    && let Some(cmp) = data.operator_compare_for(concrete)
                {
                    let read = Value::Call(
                        data.def_nr("OpConvIntFromEnum"),
                        vec![Value::Call(cmp, new_args)],
                    );
                    return Value::Call(
                        data.def_nr("OpEqInt"),
                        vec![read, Value::Int(data.ordering_discriminant("Less"))],
                    );
                }
                // Re-resolve call target if it references the type variable.
                let new_d = Self::re_resolve_call(d, tv_nr, concrete, data);
                // `(G-Sat-Eq)`, @C91 — a bound `==` over a type with no `OpEq` of its own is its
                // content `==`.  Its schema row is not reachable from here, so the call is
                // marked and `resolve_content_eq` (parser side) fills the row in.
                if new_args.len() == 2
                    && Data::is_bound_stub_for(data.def(d).name(), "OpEq", 2)
                    && names_tv(d)
                    && (new_d == u32::MAX || new_d == d || data.def(new_d).name() == "OpEqRef")
                {
                    let mut marked = new_args;
                    marked.push(Value::Int(Self::CONTENT_EQ_PENDING));
                    marked.push(Value::Int(tv_nr as i32));
                    return Value::Call(data.def_nr("OpEqContent"), marked);
                }
                // I9-vec: fix vector element access with baked-in elm_size=0.
                // The template bakes elm_size=0 for type-variable elements and omits the
                // value-extraction wrapper (OpGetInt/OpGetFloat/etc.).  Fix both here.
                //
                // P252 fix (2026-05-11): also recognise `OpGetVectorNullable`
                // — plan-07 phase 4 step 4.6 swapped the for-loop iter step
                // from `OpGetVector` to its Nullable peer (so OOB at end-of-
                // iteration returns null instead of raising).  Without this
                // arm, bounded-generic for-loops over a struct vector left
                // the iter step at `OpGetVectorNullable(v, 0, idx)` with
                // size=0 — every iteration read element 0, producing the
                // FIRST item's value for every iteration (P252).  The
                // Nullable peer's arg shape is identical to OpGetVector
                // (r, size, idx) so the elm_size fixup logic is unchanged.
                // **Only a TYPE-VARIABLE element is rewritten**, and the baked
                // stride of 0 is how one is recognised — that is what the
                // paragraph above means by "the template bakes elm_size=0 for
                // type-variable elements".  A read with a NON-ZERO stride is a
                // vector of some concrete type that merely happens to live in a
                // generic body, and its stride is already right.
                //
                // Without this guard every `OpGetVector` in the body was
                // retargeted to T's stride, so a plain `vector<float>` local was
                // written with stride 8 and read with the stride of whatever T
                // happened to be.  Silent: an 8-byte T (a one-field struct — the
                // shape most tests use) gives the correct answer by coincidence,
                // and anything wider reads garbage from element 1 onward, while
                // `len()` and the same vector passed to a NON-generic helper stay
                // correct (loft#791).
                // @PLN165 C2 — with several variables the stride names WHICH variable's element
                // it reads (`type_var_stride`), so this pass rewrites only its own; a legacy `0`
                // (a cached one-variable template) is still that one variable's.
                if new_d != u32::MAX
                    && (new_d as usize) < data.definitions.len()
                    && (data.def(new_d).name() == "OpGetVector"
                        || data.def(new_d).name() == "OpGetVectorNullable")
                    && new_args.len() == 3
                    && matches!(&new_args[1], Value::Int(n)
                        if *n == 0 || *n == Self::type_var_stride(tv_nr))
                {
                    let cur_size = if let Value::Int(n) = &new_args[1] {
                        *n
                    } else {
                        0
                    };
                    // The stride comes from `vector_elem_iter_stride` (the one
                    // home, computed by the caller) — NOT a re-derived byte-sum;
                    // see that helper for why the two drifted.
                    // A linked element is a record id in the slot: the read dereferences it,
                    // as the concrete path's `OpVectorRef` does.  Reading the slot's address
                    // instead handed the body the id's bytes as the record.
                    if iter_stride.linked {
                        let deref = if data.def(new_d).name() == "OpGetVector" {
                            "OpVectorRef"
                        } else {
                            "OpVectorRefNullable"
                        };
                        let mut it = new_args.into_iter();
                        let (vec, _, idx) = (it.next(), it.next(), it.next());
                        return Value::Call(
                            data.def_nr(deref),
                            vec![vec.unwrap_or(Value::Null), idx.unwrap_or(Value::Null)],
                        );
                    }
                    let elm_size = iter_stride.stride;
                    if elm_size != cur_size {
                        let mut fixed = new_args;
                        fixed[1] = Value::Int(elm_size);
                        let call = Value::Call(new_d, fixed);
                        return Self::wrap_vector_get_val(call, concrete, data);
                    }
                    return Self::wrap_vector_get_val(Value::Call(new_d, new_args), concrete, data);
                }
                // P239 fix (2026-05-11): the for-loop iter-termination
                // check generated by `parser/collections.rs::iter_for`
                // emits `OpConvBoolFromRef(Var(loop_var))` for any
                // loop variable typed `Reference` — including
                // `Reference(T_d_nr, …)` for generic-T element
                // iteration.  When T monomorphises to a primitive
                // (integer / text / float / single / character /
                // enum), the substituted Var is now that primitive
                // type but the IR still has `OpConvBoolFromRef`,
                // which crashes interp (treats `i64` as a `DbRef` —
                // SIGSEGV) and breaks native (rustc E0610 `i64.rec`).
                // Swap the conversion op to the matching primitive
                // peer when the substituted concrete type tells us
                // what shape the value actually is.
                if new_d != u32::MAX
                    && (new_d as usize) < data.definitions.len()
                    && data.def(new_d).name() == "OpConvBoolFromRef"
                    && new_args.len() == 1
                {
                    // EXHAUSTIVE, and the arms answer what `coalesce_not_null` answers
                    // for the same type — this map is a MIRROR of that decision, and the
                    // two spellings it was missing (`boolean`, `character`) are the two
                    // the `??` null check was missing for the same reason.  Measured
                    // 2026-08-22: this fixup fires ZERO times across all 826
                    // `tests/scripts` programs, the 33 `tests/docs` ones and every generic
                    // probe, because a `for` over a vector now terminates on the LENGTH
                    // rather than on a null element, so the `OpConvBoolFromRef` the P239
                    // comment below describes is no longer emitted.  It is completed
                    // rather than deleted because completing can only repair more paths
                    // than today and never fewer: if some lowering revives that shape, a
                    // scalar loop variable gets the right test instead of a `rec` read.
                    let conv: Option<Value> = match concrete.base() {
                        Type::Integer(_) => {
                            Self::conv_bool_call("OpConvBoolFromInt", &new_args, data)
                        }
                        Type::Text(_) => {
                            Self::conv_bool_call("OpConvBoolFromText", &new_args, data)
                        }
                        Type::Float => Self::conv_bool_call("OpConvBoolFromFloat", &new_args, data),
                        Type::Single => {
                            Self::conv_bool_call("OpConvBoolFromSingle", &new_args, data)
                        }
                        Type::Character => {
                            Self::conv_bool_call("OpConvBoolFromCharacter", &new_args, data)
                        }
                        Type::Enum(_, false, _) => {
                            Self::conv_bool_call("OpConvBoolFromEnum", &new_args, data)
                        }
                        // A boolean's "is not null" is not a single op — it is `!= 255`,
                        // the same two-op form `coalesce_not_null` builds — so it is the
                        // one arm that cannot be a name.
                        Type::Boolean => {
                            let ne = data.def_nr("OpNeBool");
                            // @C73 — a boolean's null is 255, a third state beside 0 and 1.
                            let null_b = data.def_nr("OpConvBoolFromNull");
                            if ne == u32::MAX || null_b == u32::MAX {
                                None
                            } else {
                                Some(Value::Call(
                                    ne,
                                    vec![new_args[0].clone(), Value::Call(null_b, vec![])],
                                ))
                            }
                        }
                        // DbRef-shaped: `OpConvBoolFromRef` IS the right test, so the
                        // site is left as the template built it.
                        Type::Reference(_, _)
                        | Type::Enum(_, true, _)
                        | Type::Vector(_, _)
                        | Type::Sorted(_, _, _)
                        | Type::Hash(_, _, _)
                        | Type::Index(_, _, _)
                        | Type::Radix(_, _, _)
                        | Type::Trie(_, _, _)
                        | Type::Iterator(_, _)
                        | Type::Function(..)
                        | Type::Routine(_)
                        | Type::RefVar(_)
                        | Type::Tuple(_) => None,
                        // Not loop-variable types: these carry no value to test.
                        Type::Unknown(_)
                        | Type::Null
                        | Type::Void
                        | Type::Never
                        | Type::Keys
                        | Type::Rewritten(_)
                        | Type::Optional(_) => None,
                    };
                    if let Some(call) = conv {
                        return call;
                    }
                }
                // I9-text fixup: when a T-stub had an extra __work_1 parameter
                // (for text-returning interface methods) but the concrete method
                // doesn't, drop the trailing argument to match the concrete signature.
                if new_d != d && new_d != u32::MAX && (new_d as usize) < data.definitions.len() {
                    let concrete_params = data.def(new_d).attributes().len();
                    if new_args.len() > concrete_params {
                        let mut trimmed = new_args;
                        trimmed.truncate(concrete_params);
                        return Value::Call(new_d, trimmed);
                    }
                }
                Value::Call(new_d, new_args)
            }
            Value::Block(bl) => {
                let recursed: Vec<Value> = bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, iter_stride, data))
                    .collect();
                Value::Block(Box::new(crate::data::Block {
                    operators: recursed,
                    result: Self::substitute_type(bl.result, tv_nr, concrete),
                    name: bl.name,
                    scope: bl.scope,
                    var_size: bl.var_size,
                }))
            }
            Value::Set(v, expr) => Value::Set(
                v,
                Box::new(Self::substitute_type_in_value(
                    *expr,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::Return(expr) => Value::Return(Box::new(Self::substitute_type_in_value(
                *expr,
                tv_nr,
                concrete,
                iter_stride,
                data,
            ))),
            Value::If(cond, t, f) => Value::If(
                Box::new(Self::substitute_type_in_value(
                    *cond,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
                Box::new(Self::substitute_type_in_value(
                    *t,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
                Box::new(Self::substitute_type_in_value(
                    *f,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::Loop(bl) => Value::Loop(Box::new(crate::data::Block {
                operators: bl
                    .operators
                    .into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, iter_stride, data))
                    .collect(),
                result: Self::substitute_type(bl.result, tv_nr, concrete),
                name: bl.name,
                scope: bl.scope,
                var_size: bl.var_size,
            })),
            Value::Drop(expr) => Value::Drop(Box::new(Self::substitute_type_in_value(
                *expr,
                tv_nr,
                concrete,
                iter_stride,
                data,
            ))),
            Value::Insert(ops) => Value::Insert(
                ops.into_iter()
                    .map(|v| Self::substitute_type_in_value(v, tv_nr, concrete, iter_stride, data))
                    .collect(),
            ),
            Value::Iter(name, create, next, extra) => Value::Iter(
                name,
                Box::new(Self::substitute_type_in_value(
                    *create,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
                Box::new(Self::substitute_type_in_value(
                    *next,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
                Box::new(Self::substitute_type_in_value(
                    *extra,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::Span(b) => {
                let (pos, inner) = *b;
                let new_inner =
                    Self::substitute_type_in_value(inner, tv_nr, concrete, iter_stride, data);
                Value::with_span(pos, new_inner)
            }
            // P237: tuple-constructor elements may contain calls to
            // bound-supplied operator stubs (`t_1T_OpAdd(x, x)` etc.).
            // Without this recursion the call stayed pointing at the
            // generic stub instead of the concrete `t_<len>integer_OpAdd`,
            // producing rustc E0308 (`expected DbRef, found i64`) at
            // codegen time and silent garbage / SIGSEGV under interp.
            Value::Tuple(elems) => Value::Tuple(
                elems
                    .into_iter()
                    .map(|e| Self::substitute_type_in_value(e, tv_nr, concrete, iter_stride, data))
                    .collect(),
            ),
            Value::TuplePut(v, idx, val) => Value::TuplePut(
                v,
                idx,
                Box::new(Self::substitute_type_in_value(
                    *val,
                    tv_nr,
                    concrete,
                    iter_stride,
                    data,
                )),
            ),
            Value::Yield(val) => Value::Yield(Box::new(Self::substitute_type_in_value(
                *val,
                tv_nr,
                concrete,
                iter_stride,
                data,
            ))),
            Value::CallRef(v_nr, args) => Value::CallRef(
                v_nr,
                args.into_iter()
                    .map(|a| Self::substitute_type_in_value(a, tv_nr, concrete, iter_stride, data))
                    .collect(),
            ),
            other => other,
        }
    }

    /// loft#1016 — replace every deferred `x?` default with the one the CONCRETE
    /// type has.
    ///
    /// `x?` is `x ?? construct_default(T)` (types.md `(N-Default)`), and
    /// `construct_default` is a function of `T`.  In a template `T` is a type
    /// VARIABLE, so the answer does not exist yet; [`Parser::build_default`] stamps
    /// those sites [`TV_DEFAULT_BLOCK`](Parser::TV_DEFAULT_BLOCK) and this pass answers
    /// them here, where the monomorph knows what `T` became.
    ///
    /// Runs AFTER type substitution, so a nested generic (`concrete` still a type
    /// variable — `fn outer<S>(s: S?) { inner(s?) }`) re-marks the site and stays
    /// deferred until the outer instantiation names a real type.
    /// Push the hidden `&text` work buffers a monomorph's fn-ref call needs and the
    /// template could not know about — loft#1175.
    ///
    /// The count is a function of the fn-type's RETURN, which only becomes concrete at
    /// substitution, so this asks [`Data::fnref_text_buffers`] again with the monomorph's
    /// own table and pushes what it now answers.  Same count, same order and same builder
    /// as the four parse-time sites, so the callee's slot layout has one description.
    ///
    /// **`args.len() == params.len()` is what says the buffers are still missing.** The
    /// visible arguments are all a parse-time site pushes when the count was zero; a site
    /// whose return was ALREADY concrete text pushed its buffers then and arrives longer,
    /// so it is left alone rather than served twice.
    ///
    /// The variables come from `caller_text_buf`, not the shared `__work_N` counter: this
    /// mint happens after both passes, and taking it from the shared counter would shift
    /// every later `__work_N` (loft#662's class, the reason
    /// `collections::callback_call_ref` already mints this way).
    pub(super) fn push_deferred_fnref_buffers(&mut self, v_nr: u16, args: &mut Vec<Value>) {
        let Type::Function(params, ret, ..) = self.vars.tp(v_nr).clone() else {
            return;
        };
        if args.len() != params.len() {
            return;
        }
        let n = self.data.fnref_text_buffers(params.len(), &ret);
        if n == 0 {
            return;
        }
        let work_vars: Vec<u16> = (0..n)
            .map(|_| self.vars.caller_text_buf(&mut self.lexer))
            .collect();
        self.push_fnref_text_buffers(args, &work_vars);
    }

    /// A NULLABLE result at a tuple `T` (`fn first<T>(v: vector<T>) -> T? { v[0] }`) is the
    /// stored element's REFERENCE — `(τ, σ)?` has no stack spelling, and
    /// `an-absent-tuple-meets-the-type-its-author-declares` pins what it answers — so a tuple
    /// element read that IS the result keeps the reference instead of being unboxed
    /// (`TV_TUPLE_READ`).  Unboxed, the return boxed the tuple again into a record of its own
    /// that no caller freed: one `__tuple` store leaked per call.  `tail` says whether `v`
    /// is in a result position: the body's tail, a tail block's tail, a tail `if`'s arms, and
    /// every `return`'s value wherever it stands.  Answers whether `v` now yields that
    /// reference; a block that does is retyped to the result `ret`, as a hand-written
    /// function's is — left at the stack tuple, an `if` arm's block exit was sized for the
    /// tuple while it held a reference (the interpreter read past the frame).  An `if` yields
    /// it when every arm does or is absent (`null`), and its absent arms are retyped with it.
    ///
    /// Only where EVERY result is such a read or absent ([`Self::every_result_is_a_tuple_read`]):
    /// a result that also mints on some path (a tuple parameter, a local, a literal) would
    /// be a borrow on one path and an owned record on another, which a call read without a
    /// binding does not free (`@FR-O-Complete`'s join, the residual QUALITY-history.md B7t records).
    /// There every read stays unboxed, every path mints, and the caller owns the result.
    pub(super) fn keep_returned_tuple_reads(v: &mut Value, tail: bool, ret: &Type) -> bool {
        match v {
            Value::Block(bl)
                if tail && bl.name == Self::TV_TUPLE_READ && bl.operators.len() == 1 =>
            {
                *v = bl.operators.remove(0);
                true
            }
            Value::Span(b) => Self::keep_returned_tuple_reads(&mut b.1, tail, ret),
            Value::Return(inner) => {
                Self::keep_returned_tuple_reads(inner, true, ret);
                false
            }
            Value::Block(bl) => {
                let last = bl.operators.len().saturating_sub(1);
                let mut yields = false;
                for (i, op) in bl.operators.iter_mut().enumerate() {
                    let got = Self::keep_returned_tuple_reads(op, tail && i == last, ret);
                    yields = tail && i == last && got;
                }
                if yields && matches!(bl.result.base(), Type::Tuple(_)) {
                    bl.result = ret.clone();
                }
                yields
            }
            Value::If(c, t, e) => {
                Self::keep_returned_tuple_reads(c, false, ret);
                let then = Self::keep_returned_tuple_reads(t, tail, ret);
                let other = Self::keep_returned_tuple_reads(e, tail, ret);
                let yields = tail
                    && (then || other)
                    && (then || Self::is_absent_arm(t))
                    && (other || Self::is_absent_arm(e));
                if yields {
                    Self::retype_absent_arm(t, ret);
                    Self::retype_absent_arm(e, ret);
                }
                yields
            }
            _ => {
                v.for_each_child_mut(&mut |child| {
                    Self::keep_returned_tuple_reads(child, false, ret);
                });
                false
            }
        }
    }

    /// Is every result of `v` a tuple element read (`TV_TUPLE_READ`) or absent?  `tail`
    /// says whether `v` is in a result position, as in [`Self::keep_returned_tuple_reads`];
    /// a statement is searched for the `return`s it holds.
    pub(super) fn every_result_is_a_tuple_read(v: &Value, tail: bool) -> bool {
        match v {
            Value::Block(bl) if tail && bl.name == Self::TV_TUPLE_READ => true,
            Value::Span(b) => Self::every_result_is_a_tuple_read(&b.1, tail),
            Value::Return(inner) => Self::every_result_is_a_tuple_read(inner, true),
            Value::Block(bl) if bl.name != Self::TV_NULL_BLOCK => {
                let last = bl.operators.len().saturating_sub(1);
                bl.operators
                    .iter()
                    .enumerate()
                    .all(|(i, op)| Self::every_result_is_a_tuple_read(op, tail && i == last))
            }
            Value::If(c, t, e) => {
                Self::every_result_is_a_tuple_read(c, false)
                    && Self::every_result_is_a_tuple_read(t, tail)
                    && Self::every_result_is_a_tuple_read(e, tail)
            }
            _ if tail => Self::is_absent_arm(v),
            _ => {
                let mut all = true;
                v.for_each_child(&mut |child| {
                    all &= Self::every_result_is_a_tuple_read(child, false);
                });
                all
            }
        }
    }

    /// An `if` arm that answers `null` — a bare `null`, the template's deferred null of `T`
    /// (`TV_NULL_BLOCK`), or a block ending in either: beside an arm answering the stored
    /// element's reference it is that reference absent.
    pub(super) fn is_absent_arm(arm: &Value) -> bool {
        match arm.unspan() {
            Value::Null => true,
            Value::Block(bl) if bl.name == Self::TV_NULL_BLOCK => true,
            Value::Block(bl) => bl.operators.last().is_some_and(Self::is_absent_arm),
            _ => false,
        }
    }

    /// Retypes an absent arm's blocks to the result type, the deferred null's included, so
    /// its per-instance null is the reference's (a no-op on any other arm).
    pub(super) fn retype_absent_arm(arm: &mut Value, ret: &Type) {
        if !Self::is_absent_arm(arm) {
            return;
        }
        if let Value::Block(bl) = arm.unspan_mut() {
            if matches!(bl.result.base(), Type::Tuple(_)) {
                bl.result = ret.clone();
            }
            if bl.name != Self::TV_NULL_BLOCK
                && let Some(tail) = bl.operators.last_mut()
            {
                Self::retype_absent_arm(tail, ret);
            }
        }
    }

    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn rewrite_generic_type_defaults(&mut self, val: Value) -> Value {
        match val {
            Value::Block(bl) if bl.name == Self::TV_TUPLE_READ && bl.operators.len() == 1 => {
                let mut bl = *bl;
                let read = self.rewrite_generic_type_defaults(bl.operators.remove(0));
                let Type::Tuple(elems) = bl.result.base().clone() else {
                    return read;
                };
                self.unbox_tuple_from_dbref(read, &elems)
            }
            // @FR-G-Mono — a template returns a local that VIEWS a record through an owned
            // copy (`materialize_return_into`: `w = null; OpDatabase(w); OpCopyRecord(src, w);
            // w`), because `T` compiled as a record there.  At a tuple the local is its
            // unboxed stack tuple and the copy's work-ref substituted to that tuple: the
            // record ops ran on a tuple (the interpreter indexed a store table with its
            // bits; `--native` did not compile, E0308).  The instance hands the tuple up
            // as its twin does, and the tuple return boxes it (`synthetic_tuple_return`).
            Value::Block(bl)
                if bl.name == "materialized_view_return"
                    && let [_, _, Value::Call(_, args), Value::Var(w)] =
                        bl.operators.as_slice()
                    && matches!(args.as_slice(), [_, Value::Var(to), _] if to == w)
                    && matches!(self.vars.tp(*w).base(), Type::Tuple(_)) =>
            {
                let mut bl = *bl;
                let Value::Call(_, mut args) = bl.operators.swap_remove(2) else {
                    unreachable!("matched above");
                };
                self.rewrite_generic_type_defaults(args.swap_remove(0))
            }
            Value::Block(bl)
                if bl.name == Self::TV_TUPLE_ELEM && matches!(bl.operators.len(), 2 | 3) =>
            {
                let mut bl = *bl;
                // The member offset a template's tuple FIELD write sits at (loft#1868).
                let base_pos = match bl.operators.get(2) {
                    Some(Value::Int(p)) => u16::try_from(*p).unwrap_or(0),
                    _ => 0,
                };
                let src = self.rewrite_generic_type_defaults(bl.operators.remove(1));
                let elm = self.rewrite_generic_type_defaults(bl.operators.remove(0));
                let Type::Tuple(elems) = bl.result.base().clone() else {
                    return Value::Null;
                };
                let ops = self.emit_tuple_set_ops(&elm, base_pos, &elems, src);
                v_block(ops, Type::Void, "tuple_elem_set")
            }
            // loft#1020 — the deferred `== null` / `!= null`.  `bl.result` came through
            // type substitution, so it is the CONCRETE operand type by now, and
            // `null_test` is the same dispatch the parse site uses.  `None` is the
            // ordinary path (an integer, a text): compare against the typed null, which
            // is what the concrete spelling of this declaration already emits.
            Value::Block(bl)
                if bl.name == Self::TV_NULLTEST_EQ || bl.name == Self::TV_NULLTEST_NE =>
            {
                let negate = bl.name == Self::TV_NULLTEST_NE;
                let mut bl = *bl;
                let operand = self.rewrite_generic_type_defaults(bl.operators.remove(0));
                let tp = bl.result.clone();
                if let Some(test) = self.null_test(operand.clone(), &tp, negate) {
                    return test;
                }
                let mut out = operand.clone();
                self.call_op(
                    &mut out,
                    if negate { "!=" } else { "==" },
                    &[operand, Value::Null],
                    &[tp, Type::Null],
                );
                out
            }
            // The deferred `??` null CHECK.  `bl.result` came through type
            // substitution, so it is the CONCRETE subject type by now, and
            // `coalesce_not_null` is the same dispatch the parse site uses — the ONE
            // place that answers *"is a `τ` not null?"* for a coalesce.  A nested
            // generic (`concrete` still a type variable) re-stamps through that same
            // call and stays deferred until an outer instantiation names a real type.
            Value::Block(bl) if bl.name == Self::TV_NULLCHECK => {
                let mut bl = *bl;
                let subject = self.rewrite_generic_type_defaults(bl.operators.remove(0));
                let tp = bl.result.clone();
                self.coalesce_not_null(&subject, &tp)
            }
            // loft#1028 — the deferred `null`.  `bl.result` came through type
            // substitution, so it names the CONCRETE type by now, and `null` is the same
            // dispatch the parse site uses — the ONE place that answers "what is `τ`'s
            // null?".  A nested generic (`concrete` still a type variable) re-stamps
            // through that same call and stays deferred until an outer instantiation
            // names a real type.
            // A type with no null VALUE of its own (a tuple, boxed as a record where it is
            // returned) keeps the reference sentinel the template wrote.
            Value::Block(bl) if bl.name == Self::TV_NULL_VALUE => match self.null_value(&bl.result)
            {
                Value::Null => bl.operators.first().cloned().unwrap_or(Value::Null),
                null => null,
            },
            Value::Block(bl) if bl.name == Self::TV_NULL_BLOCK => {
                let tp = bl.result.clone();
                let mut null = Value::Null;
                // "What is `τ`'s null?" builds a VALUE; it stores nothing, so @FR-N-Store's
                // bare-null ask is not for it.
                if self.convert_admitting(&mut null, &Type::Null, &tp) {
                    null
                } else {
                    // The conversion the template deferred does not exist for this `T`.
                    // Leave the site as parsed rather than dropping the value: the call
                    // is already an error, and the same reasoning loft#1016 uses.
                    Value::Block(bl)
                }
            }
            // @FR-G-Mono — a builtin whose lowering is a function of the ELEMENT type, deferred
            // by the template (`parse_insert`, `parse_reverse`).  `bl.result` came through type
            // substitution, so it is the CONCRETE vector type by now, and the same parse
            // function the concrete spelling uses lowers it — width, row and setter from one
            // home.  A nested generic re-stamps through that call and stays deferred.
            Value::Block(bl) if bl.name == Self::TV_REDUCE => {
                let bl = *bl;
                let list: Vec<Value> = bl
                    .operators
                    .into_iter()
                    .map(|a| self.rewrite_generic_type_defaults(a))
                    .collect();
                let Type::Tuple(types) = bl.result.clone() else {
                    return Value::Null;
                };
                if self.reduce_is_special(u16::MAX, &list, &types) {
                    let mut out = Value::Null;
                    self.parse_reduce(&mut out, &list, &types);
                    out
                } else if let Some(d) = self.builtin_selected(u16::MAX, "reduce", &types) {
                    // The declaration's template: the nested-generic pass aims the call at its
                    // instance.
                    Value::Call(d, list)
                } else {
                    Value::Null
                }
            }
            Value::Block(bl) if bl.name == Self::TV_SORT => {
                let bl = *bl;
                let list: Vec<Value> = bl
                    .operators
                    .into_iter()
                    .map(|a| self.rewrite_generic_type_defaults(a))
                    .collect();
                let types = [bl.result.clone()];
                if self.sort_is_special(u16::MAX, &types) {
                    let mut out = Value::Null;
                    self.parse_sort(&mut out, &list, &types);
                    out
                } else if let Some(d) = self.builtin_selected(u16::MAX, "sort", &types) {
                    // The declaration's template: the monomorph's nested-generic pass aims
                    // the call at its instance, as it does every call to a template.
                    Value::Call(d, list)
                } else {
                    // The template's bound held where the site was stamped; a concrete type
                    // that fails it was refused at the instantiation.
                    Value::Null
                }
            }
            Value::Block(bl)
                if bl.name == Self::TV_INSERT
                    || bl.name == Self::TV_REVERSE
                    || bl.name == Self::TV_RESERVE =>
            {
                let bl = *bl;
                let list: Vec<Value> = bl
                    .operators
                    .into_iter()
                    .map(|a| self.rewrite_generic_type_defaults(a))
                    .collect();
                let mut out = Value::Null;
                if bl.name == Self::TV_INSERT {
                    self.parse_insert(&mut out, &list, std::slice::from_ref(&bl.result));
                } else if bl.name == Self::TV_REVERSE {
                    self.parse_reverse(&mut out, &list, std::slice::from_ref(&bl.result));
                } else {
                    // The count was checked an integer where the template stamped the site.
                    let count = Type::Integer(crate::data::IntegerSpec::wide());
                    self.parse_reserve(&mut out, &list, &[bl.result.clone(), count]);
                }
                out
            }
            // @PLN165 B3b — a set call at a type variable, decided again with this instance's
            // argument types: the call the instance's concrete twin makes.  Resolved in the
            // source the generic was written in, as its other calls were.  A member returning
            // another type than the body was typed with is refused naming it; a nested
            // generic (a type still a variable) re-stamps through the same call.
            Value::Block(bl) if bl.name == Self::TV_SELECT => self.lower_set_call(&bl),
            Value::Block(bl) if bl.name == Self::TV_FIELD => self.lower_open_field(*bl),
            Value::Block(bl) if bl.name == Self::TV_FIELD_SET => self.lower_open_field_set(*bl),
            Value::Block(bl) if bl.name == Self::TV_OBJECT => self.lower_open_object(&bl),
            // A template lambda, non-capturing: the instance calls its own instance of it.
            Value::FnRef(d, w, _)
                if w == u16::MAX && d >= 0 && self.is_template_lambda(d as u32) =>
            {
                Value::Int(self.instantiate_template_lambda(d as u32) as i32)
            }
            // A template lambda that captures: its record is re-built for the instance, whose
            // captured values may have a width the template's record could not give them.
            Value::Block(bl)
                if bl.name == "fn_ref_with_closure"
                    && matches!(bl.operators.last().map(Value::unspan),
                        Some(Value::FnRef(d, _, _)) if *d >= 0 && self.is_template_lambda(*d as u32)) =>
            {
                let Some(Value::FnRef(d, w, _)) = bl.operators.last().map(Value::unspan).cloned()
                else {
                    return Value::Block(bl);
                };
                let inst = self.instantiate_template_lambda(d as u32);
                let mut out = Value::Null;
                self.emit_lambda_code_in(&mut out, inst, Some(w));
                out
            }
            // A capture read inside a template lambda, re-lowered against this instance's
            // record by the capture's name.
            Value::Block(bl) if bl.name == Self::TV_CAPTURE => {
                let rec = self.data.def(self.context).closure_record();
                let name = match bl.operators.get(1).map(Value::unspan) {
                    Some(Value::Text(n)) => n.clone(),
                    _ => String::new(),
                };
                let fnr = if rec == u32::MAX {
                    usize::MAX
                } else {
                    self.data.attr(rec, &name)
                };
                if fnr == usize::MAX || self.closure_param == u16::MAX {
                    return Value::Block(bl);
                }
                self.closure_capture_read(rec, fnr)
            }
            // loft#1016 — the deferred `x?` default.  `bl.result` came through type
            // substitution, so it is the CONCRETE operand type by now, the way each sibling
            // above reads its own.
            Value::Block(bl) if bl.name == Self::TV_DEFAULT_BLOCK => {
                let tp = bl.result.clone();
                match self.monomorph_default(&tp) {
                    Some((v, _)) => v,
                    // No default for this `T` — `has_default` refuses a bare reference,
                    // and the template could not have known.  Leave the site as parsed
                    // rather than dropping the value: the call is already an error.
                    None => Value::Block(bl),
                }
            }
            // loft#1175 — the hidden `&text` work buffers a fn-ref call needs, which the
            // TEMPLATE could not count.  `Data::fnref_text_buffers` reads the count off the
            // fn-type's RETURN, and inside a template that return is the type variable, so
            // every such site was lowered with zero.  Substitution rewrites the type and
            // leaves the count behind — `(G-Mono)`'s recurring class — and the `text`
            // monomorph then enters its callee one buffer short: a corrupt frame the
            // interpreter faults on.  Answered here, where `T` is real and the fn-ref
            // variable's type in the monomorph's own table is already concrete.
            Value::CallRef(v_nr, args) => {
                let mut args: Vec<Value> = args
                    .into_iter()
                    .map(|a| self.rewrite_generic_type_defaults(a))
                    .collect();
                self.push_deferred_fnref_buffers(v_nr, &mut args);
                Value::CallRef(v_nr, args)
            }
            // Everything else just carries children.  Recursion goes through the
            // `Value::for_each_child_mut` keystone rather than being re-enumerated here:
            // this walk must be TOTAL, because a marker it does not reach ships to the
            // monomorph as the placeholder, and the arms it used to list covered ten of
            // the seventeen child-bearing variants.  A `Tuple` was among the missing
            // seven, so `t = (a?, 1)` answered the placeholder's bytes as data — silently
            // at `T = integer`, as a SIGSEGV in `OpFreeText` at `T = text`, and as an
            // E0308 that would not compile on `--native`.
            //
            // Mutating in place also keeps every non-child field (a block's `scope`,
            // `var_size`, `result`) by construction, where rebuilding the node restated
            // them and a forgotten one would be silent.
            mut other => {
                other.for_each_child_mut(&mut |child| {
                    let taken = std::mem::replace(child, Value::Null);
                    *child = self.rewrite_generic_type_defaults(taken);
                });
                other
            }
        }
    }

    /// Does the work-ref `r` take a null-init in its function's preamble?  The one answer for
    /// the function parse and for a monomorph whose deferred sites minted a buffer after it.
    ///
    /// @FR-O-Proxy asks alloc — this decides which work-refs get a null-init in the preamble,
    /// which is the opposite direction from a free: it puts a slot in a known-absent state, and
    /// releases nothing.
    pub(crate) fn work_ref_takes_preamble(&self, r: u16) -> bool {
        !self.vars.is_argument(r)
            && !self.vars.is_inline_ref(r)
            // @PLAN51 Cluster IV: also null-init caller-side hidden-buffer work-refs even when
            // their typedef carries a non-empty dep list (e.g. Reference(td, [arg_idx]) for
            // if-tail / recursion / explicit-return-in-if shapes).  Without it, the slot
            // allocator skips them ("no first_def") and codegen panics at codegen.rs:2529.
            // Empty-dep refs still take this path (the original arm); caller_hidden_buf is
            // the additional gate.  #319: `__ncc_N` heap-DbRef temps likewise — their only
            // Set is inside the ncc block, so they need the preamble init regardless of their
            // dep list.
            && (self.vars.tp(r).depend().is_empty()
                || self.vars.is_caller_hidden_buf(r)
                || self.vars.name(r).starts_with("__ncc_"))
    }

    /// The [`Parser::TV_SELECT`] arm of [`Parser::rewrite_generic_type_defaults`].
    /// Lower a [`Parser::TV_FIELD`] read for the monomorph being filled: `get_field` on the
    /// instance the open one names here, exactly as the twin's `b.v` reads.  Still open (a
    /// nested template) re-stamps the site against that instance.
    pub(super) fn lower_open_field(&mut self, bl: crate::data::Block) -> Value {
        let mut ops = bl.operators.into_iter();
        let (Some(Value::Int(open)), Some(Value::Int(f_nr)), Some(receiver)) =
            (ops.next(), ops.next(), ops.next())
        else {
            return Value::Null;
        };
        let receiver = self.rewrite_generic_type_defaults(receiver);
        let d = self.bound_instance(open as u32);
        self.get_field(d, f_nr as usize, receiver)
    }

    /// Lower a [`Parser::TV_FIELD_SET`] write: the instance's field read, turned into its
    /// store by `towards_set` as the twin's `b.v = x` is.
    pub(super) fn lower_open_field_set(&mut self, bl: crate::data::Block) -> Value {
        let mut ops = bl.operators.into_iter();
        let (
            Some(Value::Int(open)),
            Some(Value::Int(f_nr)),
            Some(Value::Text(op)),
            Some(receiver),
            Some(Value::Block(val)),
        ) = (ops.next(), ops.next(), ops.next(), ops.next(), ops.next())
        else {
            return Value::Null;
        };
        let receiver = self.rewrite_generic_type_defaults(receiver);
        let val = *val;
        let src_tp = val.result.clone();
        let value = self
            .rewrite_generic_type_defaults(val.operators.into_iter().next().unwrap_or(Value::Null));
        let d = self.bound_instance(open as u32);
        let f_nr = f_nr as usize;
        let to = self.get_field(d, f_nr, receiver);
        let f_type = self.data.attr_type(d, f_nr);
        let parent_tp = Type::Reference(d, crate::data::Deps::none());
        let lhs = crate::parser::collections::AssignPlace {
            parent_tp: &parent_tp,
            fn_attr: None,
        };
        self.towards_set(&to, &value, &f_type, &src_tp, &op, &lhs)
    }

    /// Lower a [`Parser::TV_OBJECT`] literal: a fresh record of the instance the open one
    /// names here, its declared defaults, then each given field written through
    /// `handle_field` — the field write a literal of the twin makes.
    pub(super) fn lower_open_object(&mut self, bl: &crate::data::Block) -> Value {
        let Some(Value::Int(open)) = bl.operators.first().map(Value::unspan) else {
            return Value::Null;
        };
        let d = self.bound_instance(*open as u32);
        let (file, line) = match (
            bl.operators.get(1).map(Value::unspan),
            bl.operators.get(2).map(Value::unspan),
        ) {
            (Some(Value::Text(f)), Some(Value::Int(l))) => (f.clone(), *l),
            _ => (
                self.lexer.pos().file.to_string(),
                self.lexer.pos().line as i32,
            ),
        };
        let mut given: Vec<(String, Value, Type)> = Vec::new();
        for op in &bl.operators[3..] {
            if let Value::Block(f) = op.unspan()
                && f.name == Self::TV_OBJECT_FIELD
                && let [name, value] = &f.operators[..]
                && let Value::Text(name) = name.unspan()
            {
                let value = self.rewrite_generic_type_defaults(value.clone());
                given.push((name.clone(), value, f.result.clone()));
            }
        }
        let tp = self.literal_type(d);
        if self.data.is_open_instance(d) {
            // A template instantiated at another's variable: still open, still deferred.
            let mut out = vec![Value::Int(d as i32), Value::Text(file), Value::Int(line)];
            for (name, value, t) in given {
                out.push(v_block(
                    vec![Value::Text(name), value],
                    t,
                    Self::TV_OBJECT_FIELD,
                ));
            }
            return v_block(out, tp, Self::TV_OBJECT);
        }
        self.data.set_referenced(d, self.context, Value::Null);
        // Minted on pass 2 only (a monomorph is filled there), so from the pass-2 sequence,
        // as `parse_object`'s pass-2 arm draws (loft#848, loft#1078).
        let w = if crate::keys::p2_object_workref_enabled() {
            self.vars.work_refs_p2(&tp, &mut self.lexer)
        } else {
            self.vars.work_refs(&tp, &mut self.lexer)
        };
        self.set_call_refs.push(w);
        let known = i32::from(self.data.def(d).known_type());
        let mut list = vec![
            v_set(w, Value::Null),
            self.cl("OpDatabase", &[Value::Var(w), Value::Int(known)]),
        ];
        let mut code = Value::Var(w);
        let found: std::collections::HashSet<String> =
            given.iter().map(|(n, _, _)| n.clone()).collect();
        let mut fills = Vec::new();
        for (name, mut value, value_tp) in given {
            if self.data.attr(d, &name) == usize::MAX {
                continue;
            }
            if let Some(bulk) =
                self.handle_field(d, &mut code, &mut list, &name, &mut value, &value_tp)
            {
                fills.push(bulk);
            }
        }
        self.object_init(
            &mut list,
            d,
            0,
            &code,
            &found,
            &std::collections::HashSet::new(),
        );
        list.append(&mut fills);
        // The field checks a literal of the twin runs once it is built (`parse_object`).
        let assert_dnr = self.data.def_nr("n_assert");
        for a_nr in 0..self.data.def(d).attributes().len() {
            let check = self.field_check(d, a_nr);
            if check == Value::Null {
                continue;
            }
            let bound = Self::replace_record_ref(check, &code);
            let msg = match self.field_check_message(d, a_nr) {
                Value::Text(s) => Value::Text(s),
                _ => Value::Text(format!(
                    "field constraint failed on {}.{}",
                    self.data.def(d).name(),
                    self.data.attr_name(d, a_nr)
                )),
            };
            list.push(Value::Call(
                assert_dnr,
                vec![bound, msg, Value::Text(file.clone()), Value::Int(line)],
            ));
        }
        list.push(Value::Var(w));
        // The type `parse_object` gives a fresh record: it names the work-ref it is built in,
        // which the return delivery re-points at the caller's buffer.
        v_block(
            list,
            Type::Reference(d, crate::data::Deps::frame1(w)),
            "Object",
        )
    }

    pub(super) fn lower_set_call(&mut self, bl: &crate::data::Block) -> Value {
        let expected = bl.result.clone();
        let mut source = u16::MAX;
        let mut home = self.data.source;
        let mut name = String::new();
        let mut list: Vec<Value> = Vec::new();
        let mut types: Vec<Type> = Vec::new();
        let mut named: Vec<(String, Value, Type)> = Vec::new();
        for (i, op) in bl.operators.iter().enumerate() {
            match (i, op.unspan().clone()) {
                (0, Value::Int(s)) => source = u16::try_from(s).unwrap_or(u16::MAX),
                (1, Value::Int(s)) => home = u16::try_from(s).unwrap_or(home),
                (2, Value::Text(n)) => name = n,
                (_, Value::Block(a)) if a.name == Self::TV_SELECT_ARG => {
                    let a = *a;
                    let v = a.operators.into_iter().next().unwrap_or(Value::Null);
                    list.push(self.rewrite_generic_type_defaults(v));
                    types.push(a.result);
                }
                (_, Value::Block(a)) if a.name == Self::TV_SELECT_NAMED => {
                    let a = *a;
                    let mut ops = a.operators.into_iter();
                    let param = match ops.next() {
                        Some(Value::Text(p)) => p,
                        _ => String::new(),
                    };
                    let v = ops.next().unwrap_or(Value::Null);
                    let v = self.rewrite_generic_type_defaults(v);
                    named.push((param, v, a.result));
                }
                _ => {}
            }
        }
        let pos = *self.lexer.pos();
        let arg_pos = vec![pos; list.len()];
        let saved = self.data.source;
        self.data.source = home;
        let before_refs: std::collections::HashSet<u16> =
            self.vars.work_references().into_iter().collect();
        let mut out = Value::Null;
        let got = self.call(
            &mut out, source, &name, &list, &types, &named, &arg_pos, &pos,
        );
        for r in self.vars.work_references() {
            if !before_refs.contains(&r) {
                self.set_call_refs.push(r);
            }
        }
        self.data.source = saved;
        let differs = !got.is_unknown()
            && !expected.is_unknown()
            && !self.data.mentions_type_var(&got)
            && self.data.key_identity(&got) != self.data.key_identity(&expected);
        if differs {
            let shown: Vec<String> = types.iter().map(|t| t.source_name(&self.data)).collect();
            let routed = self.data.routed_types(&types);
            self.data.source = home;
            let reached = match self.select_overload(source, &name, &routed) {
                crate::parser::dispatch::Selection::One(d) => {
                    self.data.overload_signature(&name, d)
                }
                _ => format!("{name}({})", shown.join(", ")),
            };
            self.data.source = saved;
            diagnostic!(
                self.lexer,
                Level::Error,
                "inside `{}`, the call `{name}` reaches {reached} at ({}), which returns {} — the generic body was typed with {}; give {reached} that return type, or make the call outside the generic",
                self.data.def(self.context).display_name(),
                shown.join(", "),
                got.source_name(&self.data),
                expected.source_name(&self.data)
            );
        }
        out
    }

    /// `construct_default(concrete)` as a VALUE, for a monomorph's deferred `x?`.
    ///
    /// [`build_default`](Parser::build_default) answers every type except the
    /// collections, which it leaves to the caller because a collection default is a real
    /// empty collection whose construction has to be parsed in the position it lands in
    /// (a standalone `[]` leaks).  Here that position IS this parse, so the same
    /// sub-parse route serves both.
    ///
    /// `build_default` takes the already-peeled non-null base, and `concrete` is `T` as the
    /// monomorph bound it — nullable when the call site's `T` was (`sum(v: vector<integer?>)`
    /// binds `T = integer?`).  `x?` at `x: T?` discharges to the default of that base
    /// (`(N-Default)` over `T? = integer?` by `@FR-N-Idem`): `0`, not a record built for
    /// `integer?` (loft#1771).
    pub(super) fn monomorph_default(&mut self, concrete: &Type) -> Option<(Value, Type)> {
        let (base, _) = concrete.peel_optional();
        let base = base.clone();
        if crate::parser::vectors::is_collection(&base) {
            return Some(self.subparse_default("[]", &base));
        }
        self.build_default(&base)
    }
}
