// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Variable assignment and function call dispatch code generation.

use crate::data::{Context, Type, Value};
use crate::ir_node::IrNode;
use std::io::Write;

use super::calls::contains_op_database;
use super::{
    Output, block_needs_i64_widen, default_native_value_in, narrow_int_cast, rust_type, sanitize,
};

impl Output<'_> {
    /// @PLN90 #495 — variable-assignment entry.  A "runtime-Join" local (owned
    /// init + ≥1 ncc-borrow reassign; see [`Output::witness_vars`]) is routed
    /// through the owned-store-tracker path so neither free-site whole-store-frees
    /// a borrowed view.  Every other var goes straight to [`Self::output_set_body`].
    pub(super) fn output_set(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        to: &Value,
    ) -> std::io::Result<()> {
        if crate::keys::join_own_enabled() && self.witness_vars.contains(&var) {
            return self.output_set_witnessed(w, var, to);
        }
        self.output_set_body(w, var, to)
    }

    /// The owned-store-tracker emission for a runtime-Join local.  An OWNED assign
    /// (the init) emits normally, then points `_own_store_<name>` at the store r
    /// now owns.  A BORROW reassign (the ncc) emits the plain assignment, frees the
    /// tracked OWNED store it displaces (never r's new view), and NULLs the tracker.
    fn output_set_witnessed(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        to: &Value,
    ) -> std::io::Result<()> {
        let variables = self.data.def(self.def_nr).variables();
        let name = sanitize(variables.name(var));
        // A whole-value Var-copy OWNS its fresh store (C86) even though the oracle
        // reports the SOURCE var as Borrowed — mirror collect_witness_vars.
        let is_var_copy = matches!(
            to.unspan(),
            Value::Var(src) if variables.tp(*src).heap_def_nr().is_some()
        );
        let owned = is_var_copy
            || matches!(
                crate::use_analysis::ownership_of(self.data, self.def_nr, to),
                crate::use_analysis::Own::Owned
            );
        let reassign = self.declared.contains(&var);
        if reassign && !owned {
            write!(w, "{{ ")?;
            self.output_set_inner(w, var, to)?;
            write!(
                w,
                // Clear the tracker only on the branch that actually freed.  A
                // borrow reassign whose source is `r` ITSELF (`r = keep(r)`,
                // a callee that returns its argument) leaves `r` holding the
                // store it already owned — the free is correctly skipped, but
                // nulling the tracker too made scope exit skip it as well and
                // the store leaked.  Keeping it means exactly one free: the
                // scope-exit one, of the store `r` still holds.
                "; if _own_store_{name}.store_nr != u16::MAX \
                 && _own_store_{name}.store_nr != var_{name}.store_nr \
                 {{ OpFreeRef(cell, _own_store_{name}, \"{name}(owned)\"); \
                 _own_store_{name} = DbRef::NULL; }} }}"
            )?;
            return Ok(());
        }
        if reassign && owned && to.reads_var(var) {
            // `r = f(r, …)` — the new value is computed FROM the old one, so `r`
            // must still hold its store while the value is evaluated.  The
            // prelude below frees the tracked store and NULLs `var_r` before
            // emitting the value, which handed the callee a null DbRef: it read
            // through `stores[65535]` and panicked (the zero-trust `ztedit`
            // report — `ed = ed_set_caret(ed, 3)` after an `ed = new_editor(…)`).
            // Only a runtime-Join local took this path, which is why the same
            // line was fine in a sibling function and the fault looked like it
            // scaled with code volume: adding an assignment elsewhere in the body
            // is what makes `r` a Join local in the first place.
            //
            // Defer instead.  Rust evaluates the RHS before the assignment, so
            // the emitted `var_r = f(var_r, …)` already reads the OLD store;
            // snapshot what to free, emit, then free — and skip the free when the
            // callee ADOPTED the same store (it is now `r`'s own value, not a
            // displaced one).  The NULL-reset the branch below needs for in-place
            // `OpDatabase(var_r)` reuse must not happen here at all: reusing the
            // store the value reads is the same fault one step earlier.
            write!(w, "{{ let _disp_{name}: DbRef = _own_store_{name}; ")?;
            self.output_set_body(w, var, to)?;
            write!(
                w,
                "; if _disp_{name}.store_nr != u16::MAX \
                 && _disp_{name}.store_nr != var_{name}.store_nr \
                 {{ OpFreeRef(cell, _disp_{name}, \"{name}(owned)\"); }} \
                 _own_store_{name} = var_{name}; }}"
            )?;
            return Ok(());
        }
        if reassign && owned {
            // An OWNED reassign of a runtime-Join local (a 2nd+ owned assign).
            // r currently holds a store that may be a BORROWED view — so free the
            // tracked OWNED store this displaces, then RESET var_r to the null
            // sentinel so `output_set_body`'s in-place `OpDatabase(var_r)` reuse /
            // `owned_ref_reassign` displaced-free allocate FRESH and no-op the
            // free (never touching the view).  The new owned store becomes the
            // tracked one.
            write!(
                w,
                "{{ if _own_store_{name}.store_nr != u16::MAX \
                 {{ OpFreeRef(cell, _own_store_{name}, \"{name}(owned)\"); }} \
                 var_{name}.store_nr = u16::MAX; "
            )?;
            self.output_set_body(w, var, to)?;
            write!(w, "; _own_store_{name} = var_{name}; }}")?;
            return Ok(());
        }
        // First-decl.  `collect_witness_vars` requires ≥1 owned assign, and the
        // init is owned (a `first = v[i] ?? d` borrow-only local is borrow-TYPED,
        // never a candidate), so this is the owned init — a fresh store, no prior.
        //
        // `owned` is the ORACLE's verdict, and the oracle reports an element read as
        // Borrowed because `c = v[i]` normally IS a view.  When `output_set_body` takes the
        // F1/F2 arm it MINTS an owner the oracle never saw — a copy created after the
        // analysis — so the tracker has to learn about it here or nothing ever frees it:
        // the first tracked reassignment does `var_k.store_nr = u16::MAX` and drops the
        // materialised store on the floor (one leaked record per `k = v[i]` that is later
        // rebound).  Asking `materialises_element` rather than widening the oracle keeps
        // this to the store that was actually allocated.
        let materialised = self.materialises_element(var, to);
        self.output_set_body(w, var, to)?;
        if owned || materialised {
            write!(w, "; _own_store_{name} = var_{name}")?;
        }
        Ok(())
    }

    /// Does a bind of `to` into `var` take the @PLN130 F1/F2 arm — the one that allocates a
    /// record for `var` and deep-copies a container element into it?
    ///
    /// The single home for that question.  `output_set_body` asks it to choose the arm, and
    /// `output_set_witnessed` asks it to decide whether the owned-store tracker must be
    /// pointed at what the arm allocated; two spellings of the condition would drift, and a
    /// tracker that disagreed with the emitter is exactly how the store leaked (loft#823).
    fn materialises_element(&self, var: u16, to: &Value) -> bool {
        let variables = self.data.def(self.def_nr).variables();
        // Through `base()`: `heap_def_nr` names the variant and not the marker, so a nullable
        // heap local answered `None` and this arm declined — the element stayed an ALIAS and a
        // write through it reached the container.  `@FR-L-Null` makes `E?` the same record
        // behind a nullability bit, so the copy question cannot turn on the `?`.  The
        // interpreter twin peels for the same reason; asked bare, the two backends disagreed
        // and only `--native` kept the defect (loft#1456).
        variables.tp(var).base().heap_def_nr().is_some()
            // @FR-O-Proxy asks copy — the arm this selects ALLOCATES a record and deep-copies
            // into it, so a proxy that answered "owner" for a borrow costs a materialisation
            // and never a release.
            && variables.tp(var).depend().is_empty()
            // Not for a witnessed local (loft#1336, @FR-O-Witness): its ownership is a
            // per-`Set` runtime fact the FINAL dep list cannot stand in for, so every
            // projection stays an alias — as the interpreter's twin arm declines it too.
            && variables.owner_witness(var).is_none()
            && crate::generation::container_element_base(self.data, to.unspan()).is_some()
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn output_set_body(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        to: &Value,
    ) -> std::io::Result<()> {
        // REASSIGNING an owned heap ref orphans its previous store unless
        // someone frees it — the interpreter's Set emits a pre-free
        // (state/codegen.rs `owned_ref`), but native callees mint a FRESH
        // store per call, so a plain `var_x = n_f(...)` in a loop leaked one
        // store per iteration (the `sweep`/`hexn` tuple leak that blocked
        // #354, and `s = grow(s)` per-iteration struct leaks).  Mirror the
        // interpreter at the same chokepoint with the post-free shape: stash
        // the old DbRef, run the assignment, free the stash when the new
        // value is a different store (an adopting callee returns the SAME
        // store — the free must then no-op, and a never-assigned sentinel
        // store_nr is a no-op inside OpFreeRef).  Excluded, matching the
        // interpreter's predicate: borrowed views (non-empty dep) and the
        // fn's hidden return buffer (the CALLER owns that store).
        //
        // A coroutine-persistent field used to be excluded too, on the reasoning that
        // "no `var_x` local exists" — true, but the answer is to name the FIELD, not to
        // skip the free.  Skipping it leaked one store per reassignment inside a
        // generator (`s = mk(11); yield …; s = mk(22)` orphans the first).
        {
            let variables = self.data.def(self.def_nr).variables();
            let is_retbuf_attr =
                self.data.def(self.def_nr).attributes().iter().any(|a| {
                    a.hidden && self.data.def(self.def_nr).variables().var(&a.name) == var
                });
            // A retbuf-attr return-local is normally EXCLUDED (the caller owns its
            // store).  But when it has an entry-buffer witness, a CONDITIONAL
            // reassignment must free the orphaned fn-owned intermediate — guarded
            // (below) against the witness so the caller's buffer is never freed.
            // @FR-O-Proxy asks free — the fact-reading half is `Function::owns_displaced_store`,
            // the ONE spelling both backends read (@FR-O-NoDiverge; the interpreter's twin is
            // `state/codegen.rs`'s `owned_ref`).  Until it had one home this list was kept "the
            // interpreter's verbatim" by hand, and four rounds of drift are in its history: the
            // keyed kinds, the VECTOR destination (`x: vector<T> = []; for i in 0..N { x = m(i) }`
            // held one store per iteration while the interpreter stayed flat), the override
            // veto, and the detach.  What this side adds is native's own: the Rust local must
            // already be DECLARED (a reassignment, not the first bind); the right-hand side must
            // PRODUCE a store — a call in either spelling, an inline object `Insert`, a
            // `Block` that builds one (the `nullable_unwrap_copy` / `ncc` materialisers,
            // `chosen = v[i] ?? d`), or a value-`if` whose arms do (`s = if c { mk(7) } else
            // { s }`: the interpreter stashes and post-frees that shape through `rhs_reads_v`,
            // and without it here the store the then arm displaced leaked on this backend
            // alone — the `_old != place` guard is what makes the else arm, which hands back
            // the same store, a no-op), where a bare `Var` rhs is a copy whose own arm frees
            // what it displaces — which is true, but only became true for the NULLABLE bare
            // `Var` with loft#1369: that arm deferred back to here, and the two exclusions
            // closed a circle in which nothing freed the displaced store (loft#1328: `CallRef`
            // is the second call spelling and was missing — one store per iteration to frame
            // exit and a `store table exhausted` abort at 70 000 iterations on this backend
            // alone, an accept/reject split the rule forbids); and a
            // retbuf-attr return-local frees only with an entry-buffer witness, guarded below so
            // the caller's buffer is never the store released.  The displaced free is guarded by
            // `_old != place` and released through `free_displaced`, which declines a
            // free-protected store.
            let owned_ref_reassign = self.declared.contains(&var)
                && variables.owns_displaced_store(var, to, self.data)
                && matches!(
                    to.unspan(),
                    Value::Call(_, _)
                        | Value::CallRef(_, _)
                        | Value::Insert(_)
                        | Value::Block(_)
                        | Value::If(_, _, _)
                )
                && (!is_retbuf_attr || self.retbuf_witness.contains(&var));
            if owned_ref_reassign {
                let name = sanitize(variables.name(var));
                // For a witnessed retbuf-attr, also exclude the caller's entry
                // buffer (`_rb_w_<name>`): freeing it would orphan the buffer the
                // caller passed (an over-free / UAF).  Only a fn-owned
                // intermediate (distinct from both the new value AND the witness)
                // is freed.
                let witness_guard = if is_retbuf_attr {
                    format!(" && _old_{name}.store_nr != _rb_w_{name}.store_nr")
                } else {
                    String::new()
                };
                let place = match self.coroutine_persistent_fields.get(&var) {
                    Some(field) => format!("self.var_{field}"),
                    None => format!("var_{name}"),
                };
                write!(w, "{{ let _old_{name}: DbRef = {place}; ")?;
                self.output_set_inner(w, var, to)?;
                write!(
                    w,
                    "; if _old_{name}.store_nr != {place}.store_nr{witness_guard} \
                     {{ OpFreeRef(cell, _old_{name}, \"var_{name}(prev)\"); }} }}"
                )?;
                return Ok(());
            }
        }
        self.output_set_inner(w, var, to)
    }

    fn output_set_inner(&mut self, w: &mut dyn Write, var: u16, to: &Value) -> std::io::Result<()> {
        let variables = self.data.def(self.def_nr).variables();
        // P224: writes to coroutine-persistent locals target the struct
        // field directly so the value survives across `next_*` calls.
        // The same Var/Set pair would otherwise produce a state-arm-scoped
        // `let mut var_X = …` shadow that arm 1+ cannot see.
        if let Some(field) = self.coroutine_persistent_fields.get(&var) {
            // The struct's own spelling for this field, not the variable's name — two
            // `for i in …` loops in one generator put two `i`s on the struct (loft#928).
            let name = field.clone();
            // A heap local's DECLARATION arrives as `Set(v, Null)` — what the non-persisted
            // path lowers to `let mut var_x: DbRef = DbRef::NULL`.  As a field it needs no
            // declaration (the factory already initialised it), but the IR still carries the
            // statement, and `Null` in EXPRESSION position for a DbRef emits `()` — which is
            // the `expected DbRef, found ()` this produced for every struct-literal local.
            // Keyed off the lowered Rust type so it cannot drift from `rust_type`'s DbRef
            // group, which is the same list `coroutine_persistent_locals` selects on.
            if matches!(to.unspan(), Value::Null)
                && rust_type(variables.tp(var), &Context::Variable) == "DbRef"
            {
                // A keyed-collection local has NO `OpDatabase` in the IR at all — its store is
                // allocated by the DECLARATION, so a field that merely starts at `DbRef::NULL`
                // is inserted into with `store_nr == u16::MAX` and panics in `keys::mut_store`.
                // Route through the same helper the ordinary local uses, which is also what
                // keeps the @P302 in-place `s = []` clear from leaking the old store.
                let lv = format!("self.var_{name}");
                let first = self.coroutine_allocated_vars.insert(var);
                write!(w, "{lv} = ")?;
                return self.emit_null_dbref(w, var, &name, &lv, first);
            }
            // @PLN25: a `text?` var stores as `String` like plain `text` — peel so the literal→String
            // `.to_string()` conversion fires for an Optional(Text) local.
            let needs_to_string = matches!(variables.tp(var).base(), Type::Text(_));
            write!(w, "self.var_{name} = ")?;
            if needs_to_string {
                write!(w, "(")?;
            }
            self.output_code_inner(w, to)?;
            if needs_to_string {
                write!(w, ").to_string()")?;
            }
            return Ok(());
        }
        if variables.is_argument(var)
            && let Type::RefVar(inner) = variables.tp(var)
        {
            if to != &Value::Null {
                let name = sanitize(variables.name(var));
                // @PLN87 P2.2 / @PLN85 t4 — a `&`-param whole-record write-back that
                // installs a fresh OWNED store (`o = Obj{..}` literal OR `o = mk()`
                // owned-returning call) must FREE the DISPLACED caller store
                // (`*var_o`), else it orphans.  P2.2 covered only the literal (via
                // `is_skip_free`); t4 is the call twin, whose NRVO buffer `__ref_N`
                // is not skip_free — the ownership oracle's `Own::Owned` names both.
                // Aliasing-safe: stash the OLD DbRef by value, install the new store,
                // then free the old one only if distinct — a self-reading RHS
                // (`o = mk_from(o)`) keeps the old store live across the eval (no
                // free-before UAF) and a same-store install is a no-op.  The native
                // twin of the interp stash + `OpFreeRefIfDistinct` at the RefVar-set
                // site (codegen.rs).  Heap inner type only; a `RefVar(Text)` buffer
                // has no such displaced store.
                // loft#1372 — through `base()` at every one of these: a `&τ?` PARAMETER has
                // the same representation as its `&τ` twin (`Optional(τ)` shares `τ`'s
                // storage), so the displacement question, the text coercion and the boolean
                // one are all the SLOT's.  Asked bare, a `&text?` write-back emitted
                // `*var_p = "z"` with no `.to_string()` and rustc rejected it.
                let inner_slot = inner.base();
                let amp_owned_writeback = (matches!(
                    inner_slot,
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                ) || crate::parser::vectors::is_keyed(inner_slot))
                    && matches!(
                        crate::use_analysis::ownership_of(self.data, self.def_nr, to),
                        crate::use_analysis::Own::Owned
                    );
                let needs_text_coerce = matches!(inner_slot, Type::Text(_));
                // A `&boolean` slot holds the tri-state STORAGE byte (`u8`: 0/1/255,
                // null-capable) while an expression like `!b` produces a two-state
                // `bool`, so the write needs the same conversion `OpSetBoolean` carries
                // on the interpreter side.  Without it native emitted
                // `*var_b = (…) != 1;` into a `&mut u8` and rustc rejected it — the
                // second half of loft#655, invisible until the interpreter half was
                // fixed and compilation got far enough to reach it.
                let needs_bool_coerce = matches!(inner_slot, Type::Boolean);
                // The fn-ref half of the same rule: a `&fn(…)` slot holds the `(u32, DbRef)`
                // PAIR, while a bare fn name or a NON-capturing lambda carries only the d_nr
                // and emits as an `i64` — rustc E0308 against the pair, where the interpreter
                // wrote the slot correctly (loft#1443).  Signalled through `fn_ref_context`
                // rather than by wrapping the emitted text, for the reason the tuple-element
                // write gives: an if-VALUED source has to build the pair inside EACH branch,
                // and a wrap around the whole `if` would leave both arms a bare `i64`.  A
                // CAPTURING lambda already emits the pair and is not a bare `Int`, so it is
                // untouched — which is why this hid behind the shape people write first.
                let needs_fn_pair = matches!(inner_slot, Type::Function(_, _, _));
                if amp_owned_writeback {
                    write!(w, "{{ let _old_disp = *var_{name}; *var_{name} = ")?;
                } else {
                    write!(w, "*var_{name} = ")?;
                    if needs_bool_coerce {
                        write!(w, "u8::from(")?;
                    }
                    if needs_text_coerce {
                        // P223: wrap the RHS in `(...)` so that `.to_string()`
                        // attaches to the whole expression — without parens
                        // an inner `Var(text_local)` (which emits `&var_x`)
                        // would parse as `&(var_x.to_string())` (E0308:
                        // `&String` vs `String`) per Rust method-call
                        // precedence.  The parens are harmless for other
                        // RHS shapes that already produce `String`/`&str`.
                        write!(w, "(")?;
                    }
                }
                let prev_fn_ref_ctx = self.fn_ref_context;
                if needs_fn_pair {
                    self.fn_ref_context = true;
                }
                self.output_code_inner(w, to)?;
                self.fn_ref_context = prev_fn_ref_ctx;
                if amp_owned_writeback {
                    // Through the same helper the interpreter's `OpFreeRefIfDistinct`
                    // reaches, so the distinctness test and the caller's
                    // protected-from-free refusal (loft#1287) cannot drift between the
                    // backends.  Inlining the comparison here is what let the native
                    // side keep freeing a store the caller had marked as not its own.
                    write!(w, "; OpFreeRefIfDistinct(cell, _old_disp, *var_{name}); }}")?;
                } else if needs_text_coerce {
                    write!(w, ").to_string()")?;
                } else if needs_bool_coerce {
                    write!(w, ")")?;
                }
            }
            return Ok(());
        }
        // @PLN87 L1 — a local SCALAR `&`-link.  `b = &a` lowers to
        // `b: &T = OpCreateStack(a)`; native represents it as a Rust mutable borrow
        // (`&mut i64`), so reads/writes of `b` deref to `a`'s slot (the same shape a
        // `&integer` PARAMETER already uses).  First-Set (the `OpCreateStack` value)
        // (re)binds the reference to the source local; any other value is a
        // write-THROUGH (`*var_b = …`).
        // A local `&text` link joins them (loft#1371): a text local is a `String`, so the
        // link is `*mut String` and every read derefs it, exactly as the scalar link does.
        // The `&mut String` a `&text` PARAMETER carries cannot serve here — it would freeze
        // the source local for the link's whole life, and loft allows reading `c` while
        // `pc` links it.
        // loft#1372 — through `base()`: `Optional(τ)` shares `τ`'s storage exactly, so a
        // `&τ?` local link has the same representation as its `&τ` twin and the null rides
        // the slot's own sentinel.  Asked bare, `Optional` matched no arm, the bind emitted
        // no right-hand side at all (`let mut var_q: … =  as …;`) and rustc reported it —
        // which is why @FR-B-Ref-Intro's `&τ` for every τ had to be declined here too.
        if !variables.is_argument(var)
            && let Type::RefVar(inner) = variables.tp(var)
            && matches!(
                inner.base(),
                Type::Integer(..)
                    | Type::Float
                    | Type::Single
                    | Type::Boolean
                    | Type::Character
                    | Type::Tuple(_)
                    | Type::Text(_)
                    // loft#1454 — a fn-ref local link.  A fn-ref lives in the frame as a
                    // 20-byte value exactly as a tuple does, so it takes the same `*mut T`
                    // shape and the same `addr_of_mut!` bind.  Left out, it reproduced the
                    // `Optional` symptom the paragraph above records verbatim: no arm
                    // matched, the bind emitted no right-hand side (`let mut var_pd: … =
                    // as …;`) and rustc reported that instead of the missing case.
                    | Type::Function(_, _, _)
            )
        {
            let name = sanitize(variables.name(var));
            // A RAW pointer (`*mut T`), not `&mut T`: the source local stays usable
            // and assignable while the link is alive (loft allows the aliasing that
            // Rust's borrow checker forbids), matching the interpreter and loft's
            // internal unchecked-aliasing model.  Scalars don't move, so the pointer
            // stays valid for the source's scope.
            // The SLOT's Rust type, for the same reason: a `&integer?` local is a
            // `*mut i64` exactly as a `&integer` one is (loft#1372).
            let base = rust_type(inner.base(), &Context::Variable);
            // Dispatch on the construction VALUE, which is in the IR (so it survives a
            // snapshot round-trip) — no per-variable flag needed: an `OpGetField` value
            // is an L3 struct-field link, `OpGetVector`/`OpVectorRef` an L4 element link,
            // `OpCreateStack` an L1 local link, anything else a write-through.  All share
            // the `*mut T` representation; only the construction differs.
            if let Value::Call(d_nr, _) = to.unspan()
                && matches!(
                    self.data.def(*d_nr).name(),
                    "OpGetField" | "OpGetVector" | "OpVectorRef"
                )
            {
                // @PLN87 L3/L4 — a heap field (`r = &s.x`) / element (`c = &v[0]`)
                // `&`-ref.  `r`/`c` is the SAME `*mut T` shape as L1, so reads/writes
                // (`*var`) stay uniform; only construction differs — the value yields the
                // place's DbRef (`OpGetField` → record+offset; `OpGetVector` → the inline
                // element location), and we take a `*mut` into that store slot.  Aliases
                // unchecked like L1; a realloc / move of the backing staleness-invalidates
                // it, the same as the interpreter's DbRef.
                if self.declared.contains(&var) {
                    write!(w, "var_{name} = ")?;
                } else {
                    self.declared.insert(var);
                    write!(w, "let mut var_{name}: *mut {base} = ")?;
                }
                write!(w, "unsafe {{ let __ed = ")?;
                self.output_code_inner(w, to)?;
                write!(
                    w,
                    "; stores.store_mut(&__ed).addr_mut::<{base}>(__ed.rec, __ed.pos) \
                     as *mut {base} }}"
                )?;
            } else if let Value::Call(d_nr, cargs) = to.unspan()
                && self.data.def(*d_nr).name() == "OpCreateStack"
                && let [src_arg] = cargs.as_slice()
                && let Value::Var(src) = src_arg.unspan()
            {
                let src_name = sanitize(variables.name(*src));
                if self.declared.contains(&var) {
                    write!(w, "var_{name} = std::ptr::addr_of_mut!(var_{src_name})")?;
                } else {
                    self.declared.insert(var);
                    write!(
                        w,
                        "let mut var_{name}: *mut {base} = std::ptr::addr_of_mut!(var_{src_name})"
                    )?;
                }
            } else if let Value::Var(src) = to.unspan()
                && matches!(variables.tp(*src), Type::RefVar(_))
            {
                // @PLN87 L7 — ref-to-ref (`c = &b`, `b` a scalar reference): `c` copies
                // `b`'s pointer, referencing the same source `b` does (the scalar analogue
                // of the struct ref-to-ref the #257 alias already handles).
                let src_name = sanitize(variables.name(*src));
                if self.declared.contains(&var) {
                    write!(w, "var_{name} = var_{src_name}")?;
                } else {
                    self.declared.insert(var);
                    write!(w, "let mut var_{name}: *mut {base} = var_{src_name}")?;
                }
            } else {
                // write-THROUGH to the linked source
                // Write half of the same conversion the read does (loft#655): the
                // slot is the `u8` storage byte, the RHS is a `bool`.
                let bool_link = matches!(inner.base(), Type::Boolean);
                // A text RHS yields a `&str` or a `String`; the slot is a `String`.  The
                // same coercion the `&text` PARAMETER write-back carries.
                let text_link = matches!(inner.base(), Type::Text(_));
                // The fn-ref half of the same family: the slot is the `(u32, DbRef)` PAIR,
                // while a bare fn name or a non-capturing lambda carries only the d_nr.
                // Asked through `fn_ref_context` like the parameter write-back, so an
                // if-VALUED source builds the pair inside each arm (loft#1454).
                let fn_link = matches!(inner.base(), Type::Function(_, _, _));
                write!(w, "unsafe {{ *var_{name} = ")?;
                if bool_link {
                    write!(w, "u8::from(")?;
                }
                if text_link {
                    write!(w, "(")?;
                }
                let prev_fn_ref_ctx = self.fn_ref_context;
                if fn_link {
                    self.fn_ref_context = true;
                }
                self.output_code_inner(w, to)?;
                self.fn_ref_context = prev_fn_ref_ctx;
                if bool_link {
                    write!(w, ")")?;
                }
                if text_link {
                    write!(w, ").to_string()")?;
                }
                write!(w, " }}")?;
            }
            return Ok(());
        }
        // @PLN87 L5 — a heap whole-value reference (`p = &o`): `OpCreateStack(o)` with a
        // Reference referent.  The interp form is a stack-ref to `o`'s slot; native aliases
        // the record BY VALUE (the #257 alias shape, read via the record DbRef) — store
        // `o`'s DbRef.  `p` is non-owning (skip_free at the bind site); a realloc/rebind of
        // `o` is the same L7 edge the #257 alias has.
        if !variables.is_argument(var)
            && let Type::RefVar(inner) = variables.tp(var)
            && matches!(inner.base(), Type::Reference(..))
            && let Value::Call(d_nr, cargs) = to.unspan()
            && self.data.def(*d_nr).name() == "OpCreateStack"
            && let [src_arg] = cargs.as_slice()
            && let Value::Var(src) = src_arg.unspan()
        {
            let name = sanitize(variables.name(var));
            let src_name = sanitize(variables.name(*src));
            // loft#1371 — a `*mut DbRef` into the source's slot, not the source's DbRef by
            // VALUE.  By value the link could carry a read and an interior write but never
            // a WHOLE-VALUE one: `pd = S { n: 2 }` re-pointed the alias and left `d` alone,
            // so `@FR-B-Ref-Write` held on the interpreter (a real stack ref) and not here.
            // The same raw-pointer shape the scalar and text links use, for the same
            // reason: the source local stays readable while the link is alive.
            self.local_record_link.insert(var);
            if self.declared.contains(&var) {
                write!(w, "var_{name} = std::ptr::addr_of_mut!(var_{src_name})")?;
            } else {
                self.declared.insert(var);
                write!(
                    w,
                    "let mut var_{name}: *mut DbRef = std::ptr::addr_of_mut!(var_{src_name})"
                )?;
            }
            return Ok(());
        }
        // loft#1371 — a WRITE through a local `&struct` link (`pd = S { n: 2 }`).  The bind
        // just above is the only value that re-points the link; every other value writes
        // THROUGH it, which is `@FR-B-Ref-Write`.  The store it DISPLACES has to be released
        // or it orphans — the same stash / install / free-if-distinct the `&` PARAMETER
        // write-back does, and aliasing-safe for the same reason: the old `DbRef` is taken
        // by value, so a self-reading right-hand side keeps it live across the evaluation
        // and a same-store install degrades to a no-op.
        if !variables.is_argument(var)
            && self.local_record_link.contains(&var)
            && to != &Value::Null
        {
            let name = sanitize(variables.name(var));
            write!(w, "unsafe {{ let _old_disp = *var_{name}; *var_{name} = ")?;
            self.output_code_inner(w, to)?;
            write!(w, "; OpFreeRefIfDistinct(cell, _old_disp, *var_{name}); }}")?;
            return Ok(());
        }
        // #257: aliasing a `&ref` param into a fresh local (`snap = s`), both
        // RefVars.  A local RefVar alias is non-owning and never written back
        // through, so storing the record `DbRef` by value (a pointer triple,
        // `Copy`) is enough — and avoids a `&mut DbRef` reborrow that would
        // freeze the source for as long as the alias lives (so `s` could not be
        // read after `snap = s`).  `output_code_inner` yields the record DbRef:
        // `*var_s` for a `&mut DbRef` arg, `var_src` for a local DbRef alias.
        if !variables.is_argument(var)
            && matches!(variables.tp(var), Type::RefVar(_))
            && let Value::Var(src) = to.unspan()
            && matches!(variables.tp(*src), Type::RefVar(_))
        {
            let name = sanitize(variables.name(var));
            if self.declared.contains(&var) {
                write!(w, "var_{name} = ")?;
            } else {
                self.declared.insert(var);
                write!(w, "let mut var_{name}: DbRef = ")?;
            }
            self.output_code_inner(w, to)?;
            return Ok(());
        }
        // @PLN25: a `text?` var stores as `String` like plain `text` — peel so the literal→String
        // `.to_string()` conversion fires for an Optional(Text) local.
        let needs_to_string = matches!(variables.tp(var).base(), Type::Text(_));
        let name = sanitize(variables.name(var));
        // loft#1336 / @FR-O-Witness — a local whose OWNER WITNESS releases its stores, from
        // the IR.  Two consequences for the copy arms below: the copy lands in a FRESH store
        // (never `OpDatabase` over what the local currently names, which may be a VIEW —
        // the copy would be written into the viewed record), and the displaced-store free
        // an arm carries is left out (the IR releases through the witness, by identity).
        let witnessed = variables.owner_witness(var).is_some();
        // At a FIRST bind the local holds the `null_named` placeholder, which the allocation
        // is meant to consume; only a REASSIGNMENT can find a view in the slot.
        let copy_target = |current: &str, first_bind: bool| -> String {
            if witnessed && !first_bind {
                "DbRef::NULL".to_string()
            } else {
                current.to_string()
            }
        };
        let displaced_free = |free: &str| -> String {
            if witnessed {
                String::new()
            } else {
                free.to_string()
            }
        };
        // P198 — most operators are wrapped in Value::Span by the parser.
        // Unwrap before pattern-matching so the deep-copy emission below
        // fires for Span(Call(...)) / Span(Var(...)) RHS values.  Without
        // this, native codegen falls through to a plain assignment that
        // aliases the parameter's store, breaking the loft invariant that
        // each ref-typed variable owns its own store
        // (tests/scripts/95-alias-copy.loft assert orig.x == 1.0 fails
        // because mutating ac_copy mutates ac_orig too).
        let to_unspanned = to.unspan();
        // A call whose return is NOT a fresh-adopt store (it borrows a visible
        // param OR is a reused hidden buffer) must be deep-copied into a store
        // this binding owns — else the returned DbRef aliases the callee's
        // arg/buffer and a later `OpFreeRef(var)` whole-store-frees it.
        // Cluster-A A.3: reads the canonical `return_adopts_fresh_store()` fact,
        // the SAME gate the interpreter uses (`state/codegen.rs`
        // gen_set_first_at_tos), instead of the coarse "callee has a visible
        // Reference/Enum param" proxy — which MISSED a borrowed VECTOR-element
        // view (`fn pick(t: vector<M>) -> M { t[i] }`: no Reference param, yet
        // the return aliases `t`, so freeing the bound local freed the caller's
        // vector).
        //
        // loft#1017 — the callee test is `is_loft_defined()`, the FACT, not a name.
        // It read `name().starts_with("n_")`, so a `t_` METHOD (or a generic monomorph)
        // with a byte-identical body and the same `return_adopts_fresh_store()` verdict
        // fell straight through to a plain ALIAS: no deep copy, and no @P290 bracket
        // either.  The unconditional `OpFreeRef` on the bound temp then whole-store-freed
        // the RECEIVER, and the next allocation recycled that slot — `stage`'s `view_at`
        // read canvas pixels as a record number several calls later.  `scopes.rs`'s own
        // lift gate already says the two "have to name the SAME set of callees, which is
        // why the predicate lives in one place (loft#810)"; this end had drifted off it.
        // Measured: the identical body written as a FREE function was correct on
        // `--native` and as a METHOD answered zeros from the second call on.
        // loft#1106 — a NULLABLE heap local reaches this dispatch too.  `S?` is
        // `Optional(Reference(S))`, the same storage behind a nullability marker, and
        // `heap_def_nr` answers `None` for it — so a `r: S?` bound from a call whose
        // return may borrow an argument got neither the runtime adopt-or-copy guard
        // below nor the @P290 bracket, and stayed a plain alias that nothing freed.
        // The peel is gated on the same one question the interpreter and `scopes` read
        // (`nullable_join_first_bind`), so the three cannot disagree about which binds
        // change shape.
        // Through `base()` for a REASSIGNMENT: a nullable local rebound from a callee that
        // answers a borrow of its argument is copied like its dense twin (@FR-B-Copy,
        // loft#1336) — asked bare, `c = keep(other)` on a `c: S?` fell to the plain
        // assignment below and ALIASED the argument.  A FIRST bind keeps the bare question
        // plus the join fallback loft#1106 gave it; widening that is a separate walk.
        let record_def = if self.declared.contains(&var) {
            variables.tp(var).base().heap_def_nr()
        } else {
            variables.tp(var).heap_def_nr()
        }
        .or_else(|| {
            crate::use_analysis::nullable_join_first_bind(
                self.data,
                self.def_nr,
                variables.tp(var),
                to_unspanned,
            )
            .map(|(rec, _)| rec)
        });
        // loft#1245 — BOTH spellings of a call reach this dispatch.  `CallRef` is `Call`
        // with the callee in a variable, and matching only `Call` sent every fn-ref bind
        // past the copy-or-adopt split to a plain adopt: a borrowed return was ALIASED
        // (a write through the bind reached the caller's own variable, against B-Copy)
        // and a minted one was left with no owner.  An unresolved fn-ref answers `None`
        // and keeps that pre-existing emit.
        if let (Some(d_nr), Some(fn_nr)) = (
            record_def,
            crate::use_analysis::callee_of(self.data, self.def_nr, to),
        ) && matches!(to_unspanned, Value::Call(_, _) | Value::CallRef(_, _))
            && self.data.def(fn_nr).is_loft_defined()
            && !self.data.def(fn_nr).return_adopts_fresh_store()
        {
            let tp_nr = self.data.def(d_nr).known_type();
            let first_bind = !self.declared.contains(&var);
            if first_bind {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            // @P298 / @P297 (native half) — free the callee's return store
            // after the deep copy by OR-ing the `0x8000` source-free bit into
            // the OpCopyRecord type-nr (the runtime `OpCopyRecord` honours it).
            // Without this, `_src` (the freshly-allocated struct the callee
            // returned) is copied into `var_{name}` and then leaked — one
            // store per call.  Mirrors the interpreter's
            // `gen_set_first_ref_call_copy` (`src/state/codegen.rs`).
            //
            // Clear the bit when the callee returns a BORROWED view (its
            // return type carries a `dep` chain naming one of its args): the
            // "source" is then a slice of an arg's store, and freeing it would
            // corrupt the caller.  Return-dep inference tags these correctly.
            // A dep naming only HIDDEN attrs is NOT a borrow — it is the
            // one-buffer return marker (`["??"]`): the callee minted a fresh
            // store into its buffer param and nobody else owns it, so the
            // source-free bit must stay set (skipping it leaked one store
            // per `s = grow(s)` loop iteration).
            // Cluster-A A.4: ONE return-ownership query, shared with the
            // interpreter (`state/codegen.rs`).  Both backends read the same
            // fact, so they cannot diverge on the hidden-only / out-of-range edge.
            let is_borrowed_view = self.data.def(fn_nr).returns_borrowed_view();
            // loft#981/#982 — a borrowed-view return is not always a borrow: the callee
            // may hand back the parameter's store OR one it minted (a `??` whose arms
            // split, a `return o` the return hoist materialises into a fresh `__ret_N`),
            // and clearing the bit for both leaked the minted one, once per call.
            // The @P290 bracket below decides which per EXECUTION — it refuses the free
            // on a protected argument's store and allows it on a callee-minted one — so
            // the bit is set exactly when this site brackets every ref argument.
            // The interpreter reads the same fact and runs the same bracket.
            let tp_with_free: i32 =
                if crate::use_analysis::call_return_frees_source(self.data, self.def_nr, to) {
                    i32::from(tp_nr) | 0x8000
                } else {
                    i32::from(tp_nr)
                };
            // Only a borrowed-view return needs the bracket; every other call keeps its
            // previous emit byte-for-byte.
            let bracket: Vec<String> = if is_borrowed_view {
                crate::use_analysis::protectable_ref_args(self.data, self.def_nr, to)
                    .0
                    .iter()
                    .map(|&av| format!("var_{}", sanitize(variables.name(av))))
                    .collect()
            } else {
                Vec::new()
            };
            let mut protect = String::new();
            let mut unprotect = String::new();
            for v in &bracket {
                use std::fmt::Write as _;
                let _ = write!(protect, "n_protect_store_frees(cell, {v}); ");
                let _ = write!(unprotect, " n_unprotect_store_frees(cell, {v});");
            }
            // @P290 — evaluate the call BEFORE touching the destination.  The
            // call's args can reference `var_{name}` itself (e.g. `g = id(g)`),
            // and `OpDatabase` clears the destination's store IN PLACE — so the
            // old code (`OpDatabase(var)` then the call) handed the callee an
            // already-emptied destination.  Compute `_src` first against the
            // live `_dst`, then re-init + deep-copy.  When `_src` lives in the
            // destination's OWN store (the callee returned its arg — a borrowed
            // view), clearing that store would wipe the very data we copy, so
            // pass the reference through unchanged instead (the interpreter's
            // PutRef path in `gen_set_first_ref_call_copy`).
            // P198 — the inner user-fn call uses the new `cell` ABI; the
            // outer OpCopyRecord wraps `cell` to a fresh `&mut Stores`.
            let callee = self.data.def(fn_nr);
            // @PLN85 unification — collapse the native first-bind onto the ownership
            // oracle. A `??`-JOIN return needs the runtime ARG-ALIASING guard
            // witnessed by the oracle's interprocedurally-resolved base (the borrowed
            // arg): adopt the owned arm (`_src` fresh, not aliasing the witness) and
            // materialise the borrow arm (`_src` aliases the witness). The old
            // `_src == _dst` guard re-derived this and LEAKED the owned arm — `_src`
            // (a fresh `m_none()`) never equals `_dst` (the old slot), so it
            // materialised + dropped the owned store.
            let join_witness = if crate::keys::join_own_enabled()
                && let crate::use_analysis::Own::Join { base } =
                    crate::use_analysis::ownership_of(self.data, self.def_nr, to)
                && base != u16::MAX
            {
                Some(sanitize(variables.name(base)))
            } else {
                None
            };
            // `fn_ident`, not `callee.name()`: two modules may define the same fn name,
            // and emitted Rust is one flat namespace, so the DEFINITION is written with a
            // file-hash suffix (#305).  This site re-derived the identifier and emitted the
            // bare name, so a call reached a `fn` that had been emitted under another —
            // rustc E0425 "cannot find function `n_defaulted` in this scope", from a package
            // whose test file happened to name a helper the way the library did (loft#878).
            write!(w, "{{ let _dst = var_{name}; {protect}let _src = ")?;
            if let Value::Call(_, args) = to_unspanned {
                write!(w, "{}(cell", self.fn_ident(callee))?;
                // Emit each arg through the shared `emit_call_arg` helper so the
                // ABI-B call applies the same per-parameter coercions (boolean→u8,
                // narrow-int, text deref, typed-null, fn-ref) as the normal call
                // path.  Re-deriving arg emission here is what dropped the
                // boolean→u8 wrap and tripped rustc E0308 (issue #366).
                for (idx, arg) in args.iter().enumerate() {
                    write!(w, ", ")?;
                    self.emit_call_arg(w, callee, idx, arg)?;
                }
                write!(w, ")")?;
            } else {
                // A `CallRef` is not one identifier and an argument list: it lowers to a
                // MATCH over the candidate definitions of that signature
                // (`output_call_ref`).  Emitting it through the general value path is what
                // keeps the candidate set, the closure argument and the hidden buffers in
                // one place instead of re-deriving a second fn-ref ABI here.
                self.output_code_inner(w, to)?;
            }
            // The ADOPT condition. Default (`_src == _dst`): adopt a null return or
            // a same-store NRVO alias, else deep-copy. JOIN (witnessed): adopt a
            // null/fresh `_src` that does NOT alias the borrowed arg `witness`, else
            // (it aliases the witness) materialise — the join's owned/borrow split.
            // Enforces @FR-O-Detach: the COPY arm clears `_dst` in place, so it may not run
            // where `_src` still names that store — the destination would be prepared before
            // the value it is copying from is read.
            //
            // The passthrough both arms share: a NULL return has no store to copy, and a
            // return already living in the destination's own store IS the destination's
            // store.  Spelled once because the witnessed arm is a REFINEMENT of the default
            // and not a second opinion — writing it twice is how the witnessed form came to
            // omit it.
            const PASSTHROUGH: &str = "_src.store_nr == u16::MAX || _src.store_nr == _dst.store_nr";
            let adopt = match &join_witness {
                // @FR-O-Move — the caller COPIES only to obtain its OWN store.  When `_src`
                // already lives in the destination's own store the caller HAS that store, so
                // the rule asks for nothing and the copy is not merely redundant but
                // destructive: the COPY arm clears `_dst` in place via `OpDatabase`, which
                // wipes the record `_src` names before `OpCopyRecord` reads it.  That is the
                // same-store passthrough the `None` arm below carries and the @P290 comment
                // above requires ("clearing that store would wipe the very data we copy, so
                // pass the reference through unchanged"); the witnessed form REPLACED the
                // whole condition instead of refining it and so dropped it.  Measured:
                // `c = cond(c, 3)` where `cond` returns its argument on one path answered
                // `x = 0` on `--native` against `2` on the interpreter, silently, on the
                // shipped 2026.8.0 release.  Guard
                // `tests/scripts/1017b-a-conditional-borrow-into-its-own-binding.loft`.
                //
                // It is a strict widening of the ADOPT arm: the extra disjunct fires only
                // where the destination's old store and the returned value are one store, so
                // the adopt arm's own displaced-free (`_dst.store_nr != _src.store_nr`) is
                // false there and nothing is freed — the assignment becomes the no-op it
                // always was.
                Some(witness) => {
                    format!("{PASSTHROUGH} || _src.store_nr != var_{witness}.store_nr")
                }
                // loft#974 — a callee that returns a VIEW hands back a pointer into a
                // store the CALLER already owns, so the destination ALIASES it: that is
                // what the borrow in the signature means, and it is what the interpreter
                // emits here (a bare `PutRef`).  Copying instead mints a store the IR —
                // which types such a destination as a borrow and therefore emits no
                // `OpFreeRef` — never frees: one leaked record per call, measured.  It
                // also made the two backends disagree about what a view IS, so a write
                // through the result would land on one and be lost on the other.
                //
                // BOTH halves are required, and the destination is the half loft#677's
                // guard proved: a lifted call temporary (`__lift_1`) takes a borrowed
                // return too, and its own type carries NO deps — the IR calls it an owner
                // and frees it at scope exit.  Aliasing there hands that free the
                // CALLER's store (`USE AFTER FREE (write) … killed by the free of
                // var___lift_1`, native-only, the interpreter's own copy path unaffected).
                // So the alias follows the destination's ownership, not the callee's
                // return alone.
                // A WITNESSED local (loft#1336) is never-free for a different reason — its
                // witness releases its stores — and it is copied into like its owned twin.
                None if is_borrowed_view && variables.skip_free(var) && !witnessed => {
                    "true".to_string()
                }
                None => PASSTHROUGH.to_string(),
            };
            // @PLN85 (the adopt-arm placeholder leak) — the ADOPT arm replaces
            // `var_{name}`'s slot with `_src`, orphaning `_dst` when it is a
            // REAL store (the first-bind `null_named` pre-allocation, or a
            // displaced prior store on reassignment): one store leaked per
            // adopting bind (`d = choose(..)`).  Free the real, distinct
            // placeholder first — the same exclusive-ownership assumption the
            // COPY arm already makes (it clears `_dst` in place via
            // `OpDatabase`).  A same-store adopt (the NRVO alias) and the
            // null-sentinel `_dst` are excluded by the guard.
            let disp = displaced_free(&format!(
                "if _dst.store_nr != u16::MAX && _dst.store_nr != _src.store_nr \
                 {{ OpFreeRef(cell, _dst, \"{name}(displaced)\"); }} "
            ));
            let target = copy_target("_dst", first_bind);
            write!(
                w,
                "; if {adopt} {{ {disp}var_{name} = _src; }} \
                 else {{ var_{name} = OpDatabase(cell, {target}, {tp_nr}_i32); \
                 OpCopyRecord(cell,_src, var_{name}, {tp_with_free}_i32); }}{unprotect} }}"
            )?;
            // @PLN130 — a MAY-copy site: the emitted code branches on store identity at
            // runtime and copies on the non-adopting arm.  Recorded regardless, because the
            // guard asks whether the diagnostic ACCOUNTS for the site, not whether this
            // particular execution took the copying arm.
            crate::copy_manifest::record(
                self.def_nr,
                var,
                tp_nr,
                crate::copy_manifest::Origin::NativeCallReturn,
            );
            return Ok(());
        }
        // loft#1248 — a first bind from a CLOSURE call whose return may borrow.  The
        // call-return block above is keyed on `Value::Call`, which names its definition; a
        // `CallRef` names a runtime VALUE and reaches none of it.  `scopes` strips such a
        // bind's deps so the interpreter's `OpBindOrCopy` can own the minted arm, and those
        // empty deps reach THIS generator too — so without this arm native adopted the
        // borrow arm as an owner and freed the caller's own record at scope exit
        // (`USE AFTER FREE (read) … killed by the free of var_r`, native-only, measured).
        //
        // The guard is the one its direct-call twin above emits, with the same witness from
        // the same oracle: adopt when the returned store is not the witness's (the closure
        // minted it), materialise when it is (the closure handed back what the caller still
        // holds).  The copy takes NO source-free bit for that reason — the source on that
        // arm is the witness's store.
        //
        // The source is emitted through `output_code_inner`, which is spelling-agnostic, so
        // this arm needs none of the call block's `emit_call_arg` / `fn_ident` / @P290
        // machinery: a fn-ref call site lowers to its own `match` dispatch and the value it
        // yields is all this guard reads.
        if let Some((rec, base)) = crate::use_analysis::callref_join_first_bind(
            self.data,
            self.def_nr,
            variables.tp(var),
            to_unspanned,
        ) {
            let tp_nr = self.data.def(rec).known_type();
            let witness = sanitize(variables.name(base));
            let first_bind = !self.declared.contains(&var);
            if first_bind {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            write!(w, "{{ let _dst = var_{name}; let _src = ")?;
            self.output_code_inner(w, to)?;
            let disp = displaced_free(&format!(
                "if _dst.store_nr != u16::MAX && _dst.store_nr != _src.store_nr \
                 {{ OpFreeRef(cell, _dst, \"{name}(displaced)\"); }} "
            ));
            let target = copy_target("_dst", first_bind);
            write!(
                w,
                "; if _src.store_nr == u16::MAX || _src.store_nr != var_{witness}.store_nr \
                 {{ {disp}var_{name} = _src; }} \
                 else {{ var_{name} = OpDatabase(cell, {target}, {tp_nr}_i32); \
                 OpCopyRecord(cell,_src, var_{name}, {tp_nr}_i32); }} }}"
            )?;
            // A MAY-copy site: the branch is decided at run time, and the manifest asks
            // whether the diagnostic accounts for the site, not which arm ran.
            crate::copy_manifest::record(
                self.def_nr,
                var,
                tp_nr,
                crate::copy_manifest::Origin::NativeCallReturn,
            );
            return Ok(());
        }
        // @PLN130 F1/F2 — MATERIALISE an element/field read into a store `var` owns.
        //
        // Sibling of the interpreter's `gen_set_first_ref_elem_copy`.  `c = v[i]` normally
        // keeps a dep on its container and stays a borrow (the documented alias, loft#774);
        // reaching here with EMPTY deps means some earlier pass decided `var` is an owner —
        // either it is reassigned later (F1) or its container is reshaped while it is live
        // (F2, where `scopes` strips the dep and warns).  Emitting the raw interior pointer
        // then leaves an "owner" whose store belongs to the container.
        //
        // Native needed its own arm: it materialises `_own_store_*` for a CALL return, but
        // not for an element read, so the F2 strip alone left `--native` still reading the
        // wrong element (probe 05: `c.n 44 want 33`) while the interpreter was already
        // correct.  One fact, and until this both backends did not act on it.
        // `base()` for the same reason the predicate above peels: the marker does not change
        // which record this is (@FR-L-Null).  The GATE and the EMIT both ask, so peeling only
        // one leaves the arm selected and unreachable.
        if let Some(d_nr) = variables.tp(var).base().heap_def_nr()
            && self.materialises_element(var, to)
        {
            let tp_nr = self.data.def(d_nr).known_type();
            let first_bind = !self.declared.contains(&var);
            if first_bind {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            write!(w, "{{ let _src = ")?;
            self.output_code_inner(w, to)?;
            writeln!(w, ";")?;
            self.indent(w)?;
            // A copy of an ABSENT element is absent — so ask before allocating (loft#823).
            //
            // Absence has two spellings and this arm knew only one.  `OpCopyRecord` guards
            // the true null sentinel (`store_nr == u16::MAX`); an index past the end is the
            // OTHER one — `vector::get_vector` answers `{store_nr: <the real store>, rec: 0}`,
            // and its own doc says the two "read as the same absent value".  Allocating first
            // and copying second turned that absent element into a live empty record, so
            // `v[oob] ?? d` saw a PRESENT value and answered with the record's uninitialised
            // bytes.  `rec == 0` is the predicate every store accessor already uses for
            // absence (`if db.rec == 0 { f64::NAN }`), not a new one invented here.
            //
            // On the absent arm the placeholder must go back: at a FIRST bind that is the
            // `null_named` store allocated just above (the same orphan the call-return arm
            // frees as `(displaced)`), while a REASSIGNMENT is already wrapped by
            // `output_set`'s `_old_*` stash, which frees the displaced store itself — freeing
            // here too would free it twice.
            let release = if first_bind {
                format!(
                    "if var_{name}.store_nr != u16::MAX \
                     {{ OpFreeRef(cell, var_{name}, \"{name}(absent)\"); }} "
                )
            } else {
                String::new()
            };
            let target = copy_target(&format!("var_{name}"), first_bind);
            write!(
                w,
                "if _src.rec == 0 {{ {release}var_{name} = DbRef::NULL; }} \
                 else {{ var_{name} = OpDatabase(cell,{target}, {tp_nr}_i32); \
                 OpCopyRecord(cell,_src, var_{name}, {tp_nr}_i32); }} }}"
            )?;
            crate::copy_manifest::record(
                self.def_nr,
                var,
                tp_nr,
                crate::copy_manifest::Origin::NativeRecordBind,
            );
            return Ok(());
        }
        // When assigning a reference to a reference variable, a pointer copy is not
        // sufficient — emit an OpCopyRecord call for a deep copy.
        // For a first declaration, we also need to allocate a fresh store via
        // OpDatabase(null_named(…)) so the destination has its own record to copy into.
        // For reassignment, the existing destination record is reused in-place.
        // `base()`, because `S?` is `Optional(Reference(S))` — the same storage behind a
        // nullability marker — and `heap_def_nr` reads the bare spelling only.  Unpeeled,
        // a nullable whole-value bind reached neither this arm nor any other and fell
        // through to `let mut var_d = var_s;`, a pointer copy: an ALIAS, where `@FR-B-Copy`
        // says the bound variable is INDEPENDENT (loft#1319).
        if let (Some(d_nr), Value::Var(src)) =
            (variables.tp(var).base().heap_def_nr(), to_unspanned)
            && variables.tp(*src).base().heap_def_nr().is_some()
        {
            let src_name = sanitize(variables.name(*src));
            let tp_nr = self.data.def(d_nr).known_type();
            let first_bind = !self.declared.contains(&var);
            if self.declared.contains(&var) {
                // Reassignment: the variable was pre-declared via null_named
                // (Set(var, Null)) at function entry.  OpDatabase below
                // ensures it has a valid allocated record.
            } else {
                self.declared.insert(var);
                let var_tp = variables.tp(var);
                let tp_str = rust_type(var_tp, &Context::Variable);
                // Two statements: null_named and OpDatabase cannot share a &mut stores borrow.
                writeln!(
                    w,
                    "let mut var_{name}: {tp_str} = stores.null_named(\"var_{name}\");"
                )?;
                self.indent(w)?;
            }
            // A source that may be ABSENT is asked before it is dereferenced: a copy of an
            // absent value is absent, and `OpCopyRecord` would read `allocations[u16::MAX]`.
            // Allocating first and copying second would be worse than the panic — it leaves
            // the destination holding the record allocated for it, PRESENT where its source
            // was absent.  Same shape and same predicate as the element-read arm above
            // (loft#823): `rec == 0` is the absence test every store accessor uses.  WHICH
            // binds may carry absence is `Variables::bind_admits_absence`'s question, asked of
            // BOTH sides and shared with the interpreter's record bind — a source typed
            // non-null can still hold `nullref`, and a nullable destination has to receive it
            // as absence.
            if variables.bind_admits_absence(var, *src) {
                // On the absent arm the placeholder must go back: writing `DbRef::NULL`
                // DISPLACES whatever the destination held, and nothing downstream releases
                // it.  At a FIRST bind that is the `null_named` store allocated just above.
                // At a REASSIGNMENT it is whatever the destination held, and this is the ONE
                // site that can free it — `output_set`'s `_old_*` stash wraps only a
                // right-hand side that PRODUCES a store (`Call` / `CallRef` / `Insert` /
                // `Block`), and this arm's is a bare `Var`, which is none of those.
                //
                // The two sites used to name EACH OTHER as the one that frees — the stash
                // says a bare `Var` "is a copy whose own arm frees what it displaces", and
                // this comment said a reassignment "is already wrapped by `output_set`'s
                // `_old_*` stash".  For `x = <nullable Var>` at a reassignment neither did,
                // so the record the destination held orphaned once per call whose source was
                // absent AT RUNTIME (loft#1369 — the `x: S? = z; x = y` shape, native only:
                // the interpreter's twin frees it).  The ELSE arm needs nothing, which is why
                // only the null side lost a store: `OpDatabase(cell, var_x, …)` recycles the
                // slot's existing store rather than displacing it.
                //
                // Gated on the same predicate the stash asks (@FR-O-Proxy), so @FR-O-Borrow
                // still holds through it: a destination that only VIEWS a parameter owns no
                // store and frees nothing here.
                let release =
                    if first_bind || variables.owns_displaced_store(var, to_unspanned, self.data) {
                        format!(
                            "if var_{name}.store_nr != u16::MAX \
                         {{ OpFreeRef(cell, var_{name}, \"{name}(absent)\"); }} "
                        )
                    } else {
                        String::new()
                    };
                let target = copy_target(&format!("var_{name}"), first_bind);
                write!(
                    w,
                    "if var_{src_name}.rec == 0 {{ {release}var_{name} = DbRef::NULL; }} \
                     else {{ var_{name} = OpDatabase(cell,{target}, {tp_nr}_i32); \
                     OpCopyRecord(cell,var_{src_name}, var_{name}, {tp_nr}_i32); }}"
                )?;
                crate::copy_manifest::record(
                    self.def_nr,
                    var,
                    tp_nr,
                    crate::copy_manifest::Origin::NativeRecordBind,
                );
                return Ok(());
            }
            let target = copy_target(&format!("var_{name}"), first_bind);
            writeln!(w, "var_{name} = OpDatabase(cell,{target}, {tp_nr}_i32);")?;
            self.indent(w)?;
            write!(
                w,
                "OpCopyRecord(cell,var_{src_name}, var_{name}, {tp_nr}_i32)"
            )?;
            // @PLN130 — native's whole-record bind deep-copies unconditionally (it has no
            // last-use move; that asymmetry with the interpreter is @PLN130 cluster V).
            crate::copy_manifest::record(
                self.def_nr,
                var,
                tp_nr,
                crate::copy_manifest::Origin::NativeRecordBind,
            );
            return Ok(());
        }
        // For text/reference block assignments, pre-declare the variable so that
        // any drop(@var) inside the block (e.g., on break) can reference it.
        if !self.declared.contains(&var) && matches!(to, Value::Block(_)) {
            let var_tp = variables.tp(var);
            // @PLN25: peel `Optional(Text)` — a `text?` block-assigned local is `String`-typed.
            if matches!(var_tp.base(), Type::Text(_)) {
                self.declared.insert(var);
                write!(w, "let mut var_{name} = ")?;
                self.output_code_inner(w, to)?;
                if needs_to_string {
                    write!(w, ".to_string()")?;
                }
                return Ok(());
            }
        }
        // S35: Set(var, Insert([stmt1, ..., last_expr])) — hoist all-but-last ops as
        // statements before the declaration, then assign only from the final expression.
        // Without this, the inner Set ops are emitted inline inside an expression context,
        // producing malformed Rust like `let mut var_rv: DbRef = let mut var__read: DbRef = …`.
        //
        // @P321g: unspan `to` first.  The parser wraps an assignment RHS in
        // `Value::Span` for source-position tracking, so `x = route_click(p,
        // st.es_tools, …)` — where the `&`-ref arg `st.es_tools` materialises a
        // `Set(__ref_N, …)` statement ahead of the call — arrives here as
        // `Span(Insert([Set(__ref_N, …), Call(…)]))`.  Matching the bare
        // `Insert` missed it, falling through to the brace-less `Insert` arm in
        // `output_code_inner` and re-emitting the exact malformed shape above.
        if let Value::Insert(ops) = to_unspanned
            && !ops.is_empty()
        {
            for op in &ops[..ops.len() - 1] {
                self.indent(w)?;
                self.output_code_inner(w, op)?;
                writeln!(w, ";")?;
            }
            self.indent(w)?;
            if self.declared.contains(&var) {
                write!(w, "var_{name} = ")?;
            } else {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                write!(w, "let mut var_{name}: {tp_str} = ")?;
            }
            // The tail is the assigned VALUE, so it needs the same storage-form coercion
            // the plain-RHS path applies below: a `boolean` variable stores as `u8`, and a
            // narrow-int value-block widens to the `integer` slot.  Emitting it raw dropped
            // both — `return <record-returning-call>.<field> == <lit>` lifts the call into a
            // `__lift_N` temp, which makes the RHS an `Insert`, and the boolean landed here
            // uncast: `let mut var___ret_1: u8 = <bool>` (E0308, loft#672).  Binding the
            // record to a local first needs no lift, which is why only the call form broke.
            let tail = &ops[ops.len() - 1];
            let wrap_bool =
                !matches!(tail, Value::Null) && matches!(variables.tp(var).base(), Type::Boolean);
            let widen_block = block_needs_i64_widen(tail, variables.tp(var));
            // The TEXT twin of the boolean cast above, and the same shape it
            // describes: `return <text-fn>(<record-returning-call>(…))` inside a
            // loop lifts the record call into a `__lift_N` temp, which makes this
            // RHS an `Insert`, and the tail then landed uncast — a text-returning
            // callee answers `Str` while the local is declared `String`
            // (E0308).  Binding the record to a local first needs no lift, which
            // is why only the call form broke, exactly as in loft#672.
            //
            // Keyed on the VARIABLE's type, like the other three text-assignment
            // paths in this function (`needs_to_string`), rather than on the
            // tail's node shape — a per-shape test here is what left this hole
            // when the boolean one was closed.
            let text_tail = !matches!(tail, Value::Null) && needs_to_string;
            if wrap_bool || widen_block || text_tail {
                write!(w, "(")?;
            }
            self.output_code_inner(w, tail)?;
            if wrap_bool {
                write!(w, ") as u8")?;
            } else if widen_block {
                write!(w, ") as i64")?;
            } else if text_tail {
                write!(w, ").to_string()")?;
            }
            return Ok(());
        }
        // Hoist call arguments that mutate stores into temporaries to prevent
        // double-mutable-borrow of `stores` in the call expression.
        //
        // ONLY for ops emitted as an actual CALL — a user fn or a `codegen_runtime`
        // Op stub (`rust().is_empty()`).  An inline `#rust` op (non-empty template,
        // e.g. a byte/short field read like `OpGetByteNullable`) has NO callable
        // fn: emitting `OpGetByteNullable(cell, _harg, …)` is unresolved.  Those
        // fall through to the normal emit, which inlines the `#rust` body — whose
        // own `let db = @v1; …` sequences the mutating arg before the read borrow,
        // so no double-borrow (and the receiver was already pre-eval-hoisted).
        if let Value::Call(call_dnr, args) = to.unspan()
            && self.data.def(*call_dnr).rust().is_empty()
            && args.iter().any(|a| self.arg_needs_text_hoist(a))
        {
            let def_fn = self.data.def(*call_dnr);
            let mut hoisted: Vec<Option<String>> = vec![None; args.len()];
            for (idx, arg) in args.iter().enumerate() {
                if self.arg_needs_text_hoist(arg) {
                    let param_tp = if idx < def_fn.attributes().len() {
                        rust_type(&def_fn.attributes()[idx].typedef, &Context::Argument)
                    } else {
                        "DbRef".to_string()
                    };
                    let tmp = format!("_harg_{name}_{idx}");
                    write!(w, "let {tmp}: {param_tp} = ")?;
                    self.output_code_inner(w, arg)?;
                    writeln!(w, ";")?;
                    self.indent(w)?;
                    hoisted[idx] = Some(tmp);
                }
            }
            if self.declared.contains(&var) {
                write!(w, "var_{name} = ")?;
            } else {
                self.declared.insert(var);
                let tp_str = rust_type(variables.tp(var), &Context::Variable);
                write!(w, "let mut var_{name}: {tp_str} = ")?;
            }
            // P199 — user-fn / Op-stub callees take `&UnsafeCell<Stores>`
            // (cell), not `&mut Stores` (stores).  Plan 09 phase 01 added
            // per-fn ABI tagging via `crate::codegen_runtime::abi_of`:
            //   - Cell  → `name(cell, args...)`     (default)
            //   - None  → `name(args...)`           (no implicit Stores)
            let abi = if *def_fn.code() == Value::Null {
                crate::codegen_runtime::abi_of(def_fn.name())
            } else {
                crate::codegen_runtime::Abi::Cell
            };
            // loft#1032 — a generator callee answers `Box<dyn LoftCoroutine>`, and every
            // caller holds the handle as a `DbRef` from the coroutine table, so the call
            // must be wrapped exactly as `user_fn_call_body` wraps it.  This path had the
            // per-parameter coercions in lockstep (issue #366) but not the RETURN side, so
            // a generator reached through it emitted a bare call into a `DbRef` local:
            // `expected DbRef, found Box<dyn LoftCoroutine>`.  The hoist fires when an
            // argument mutates a store, which a vector literal does — so `h([4,5,6])` on a
            // plain `fn h(v: vector<integer>) -> iterator<integer>` did not compile either.
            let is_generator = matches!(def_fn.returned(), Type::Iterator(_, _));
            if is_generator {
                write!(w, "loft::codegen_runtime::alloc_coroutine(")?;
            }
            write!(w, "{}(", self.fn_ident(def_fn))?;
            let mut first_arg = true;
            if matches!(abi, crate::codegen_runtime::Abi::Cell) {
                write!(w, "cell")?;
                first_arg = false;
            }
            for (idx, arg) in args.iter().enumerate() {
                if !first_arg {
                    write!(w, ", ")?;
                }
                first_arg = false;
                // A store-mutating arg was hoisted to a typed temporary above;
                // emit its name.  Every other arg goes through the shared
                // `emit_call_arg` so this path applies the same per-parameter
                // coercions (boolean→u8, …) as the normal + ABI-B call paths
                // (issue #366 — keep all three call paths in lockstep).
                if let Some(ref tmp) = hoisted[idx] {
                    write!(w, "{tmp}")?;
                } else {
                    self.emit_call_arg(w, def_fn, idx, arg)?;
                }
            }
            write!(w, ")")?;
            if is_generator {
                write!(w, ")")?; // close alloc_coroutine(…)
            }
            if needs_to_string {
                write!(w, ".to_string()")?;
            }
            return Ok(());
        }
        // @P302 — capture first-vs-reassign BEFORE the `declared` insert below
        // (the first-decl branch inserts `var`, so the flag must be read now).
        // A #260-predeclared `__vdb` counts as FIRST here (consume the entry):
        // its prologue `let` bound only the sentinel, so this Set still emits
        // the named-store `null_named` + `OpDatabase` pair.
        // #354: the `_` discard loop var shares ONE table entry across all
        // the fn's loops, so a later loop whose iter-value type differs from
        // the table type (an integer range after a float range gives an i64
        // value into the f64 `var__`) fails E0308.  `_` is a discard, so
        // emit a fresh shadowing `let` typed from THIS loop's own iter
        // value.  Restricted to a SCALAR table type AND scalar iter value:
        // a collection `for _ in <vec>` binds a DbRef view whose OpFreeRef
        // (emitted by scope analysis against the table type) must keep
        // seeing a DbRef — re-typing it scalar there orphaned the store (a
        // leak, crawler's hex/sim libs).
        // One home: `data::is_scalar` (formal/IMPLEMENTATIONS.md #1).
        use crate::data::is_scalar;
        let discard_loop_var = variables.name(var) == "_"
            && is_scalar(variables.tp(var))
            && matches!(to.unspan(), Value::Block(bl) if is_scalar(&bl.result));
        let first_assign = !self.declared.contains(&var) || self.predeclared.remove(&var);
        if self.declared.contains(&var) && !discard_loop_var {
            write!(w, "var_{name} = ")?;
        } else {
            self.declared.insert(var);
            let var_tp = if discard_loop_var && let Value::Block(bl) = to.unspan() {
                bl.result.clone()
            } else {
                variables.tp(var).clone()
            };
            let tp_str = rust_type(&var_tp, &Context::Variable);
            write!(w, "let mut var_{name}: {tp_str} = ")?;
        }
        if matches!(to, Value::Null) && rust_type(variables.tp(var), &Context::Variable) == "DbRef"
        {
            let lv = format!("var_{name}");
            self.emit_null_dbref(w, var, &name, &lv, first_assign)?;
        } else if to == &Value::Null {
            // Emit the null sentinel for the variable's type, not bare `()` — in the
            // VARIABLE context, which is the one the declaration above was written in.
            let null_val = default_native_value_in(variables.tp(var), &Context::Variable);
            write!(w, "{null_val}")?;
        } else {
            // @PLN17: a boolean variable's storage form is u8.  The RHS may be a
            // compound `bool` expression (`!b` → `(..) != 1`, `a == b`), so the
            // ` as u8` coercion must WRAP the whole RHS — a bare suffix binds to
            // the last token (`!= 1 as u8`) and leaves a `bool`.  Open the paren
            // here; the narrow-cast suffix below closes it with `) as u8`.
            let wrap_bool =
                !matches!(to, Value::Null) && matches!(variables.tp(var).base(), Type::Boolean);
            // #433 — a narrow-int value-block (`vec<u8>[i] ?? <int>` ncc) assigned to
            // a plain `integer` (i64) variable needs an `as i64` widen, same as the
            // return seam (see block_needs_i64_widen).  Open the wrapping paren here;
            // the suffix closes it below.
            let widen_block = block_needs_i64_widen(to, variables.tp(var));
            if wrap_bool || widen_block {
                write!(w, "(")?;
            }
            // O7: when this text assignment opens a multi-segment format string,
            // pre-allocate capacity to avoid repeated reallocations.
            if needs_to_string
                && self.next_format_count > 1
                && let Value::Text(initial) = to
            {
                let n = self.next_format_count;
                self.next_format_count = 0;
                let cap = initial.len() + n * 8;
                if initial.is_empty() {
                    write!(w, "String::with_capacity({cap}_usize)")?;
                } else {
                    write!(
                        w,
                        "{{ let mut _s = String::with_capacity({cap}_usize); \
                         _s.push_str({initial:?}); _s }}"
                    )?;
                }
            } else {
                // wrap plain Int or If-with-Int values assigned to Function vars.
                let is_fn_ref_var = matches!(variables.tp(var), Type::Function(_, _, _));
                let wrap_fn_ref = is_fn_ref_var && matches!(to, Value::Int(_));
                if wrap_fn_ref {
                    write!(w, "(")?;
                }
                // set fn_ref_context so if-else branches with bare Int
                // values produce (u32, null_DbRef) tuples.  Cleared inside
                // Call argument processing to avoid wrapping OpDatabase args.
                let prev_ctx = self.fn_ref_context;
                if is_fn_ref_var && !wrap_fn_ref {
                    self.fn_ref_context = true;
                }
                // When assigning to a `(String, …)` tuple variable, the
                // element values that emit as `&str` literals need a
                // `.to_string()` wrap so the tuple's runtime type
                // matches its declared `(String, …)` shape.  Without
                // this the Rust compiler rejects `("a", "b")` against
                // `(String, String)`.  See `Value::Text` in emit.rs.
                let prev_tuple_text = self.tuple_text_to_string;
                // loft#1069 — hand the emitter the DECLARED element types, so a fn-ref
                // member written as a bare name is built as the `(u32, DbRef)` pair its
                // slot is rather than emitted as the lone d_nr it infers to.
                let prev_tuple_slots = std::mem::take(&mut self.tuple_slot_types);
                if let Type::Tuple(elems) = variables.tp(var)
                    && elems.iter().any(crate::data::tuple_carries_fn_ref)
                {
                    self.tuple_slot_types = elems.clone();
                }
                if let Type::Tuple(elems) = variables.tp(var)
                    && tuple_has_text_leaf(elems)
                {
                    // Recurse through nested tuples so `((i64, String),
                    // (i64, String))` triggers the flag too — without
                    // this, plan-14 phase-02 cells like
                    // `((1, "a"), (2, "b"))` emitted `"a"` (`&str`)
                    // against a declared `String` slot and rustc raised
                    // E0308.
                    self.tuple_text_to_string = true;
                }
                // When assigning to a String variable from a text-local source,
                // output_code_inner emits `&var_name` (borrow to &str), and
                // appending `.to_string()` yields `&String` not `String`.
                // Detect this case and emit `.clone()` on the owned String directly.
                // Unspan the RHS so the detection fires for Span-wrapped Var
                // (the common case for assignment RHS — same shape as P228).
                let text_local_clone = needs_to_string
                    && matches!(to.unspan(), Value::Var(v) if {
                        let vars = self.data.def(self.def_nr).variables();
                        // @PLN25 slice (c): `.base()` — a `text?` local source is a `String`
                        // just like plain `text`, so it must emit `var.clone()` here. Without
                        // the peel it fell through to `&var.to_string()` (E0308: `&String`).
                        !vars.is_argument(*v) && matches!(vars.tp(*v).base(), Type::Text(_))
                    });
                // @P283 — source is a `RefVar(Text)` argument (`&mut String`).
                // `output_code_inner` for `Value::Var` in this case emits
                // `&*var_X` (emit.rs:141); appending `.to_string()` then parses
                // as `&*(var_X.to_string())` per Rust method-call precedence,
                // which evaluates to `&str` and breaks the `String`-typed
                // assignment with E0308.  Emit `var_X.to_string()` directly —
                // auto-deref through `&mut String` produces a fresh owned
                // `String` of the correct type.
                let refvar_text_clone = needs_to_string
                    && matches!(to.unspan(), Value::Var(v) if {
                        let vars = self.data.def(self.def_nr).variables();
                        vars.is_argument(*v)
                            && matches!(vars.tp(*v), Type::RefVar(inner) if matches!(**inner, Type::Text(_)))
                    });
                // T1.8a: same pattern when the source is a `TupleGet` of a
                // text element from a Variable-context tuple — the tuple's
                // text fields are `String`, and `output_code_inner` for
                // `Value::TupleGet` of a text element emits `&var_t.0`
                // (borrow to &str).  Appending `.to_string()` yields
                // `&String` not `String`, breaking destructuring of a
                // tuple-of-text return.  Emit `var_t.0.clone()` instead.
                //
                // P228: unspan `to` before pattern-matching so the
                // detection fires when the parser wraps the TupleGet in
                // a `Value::Span` (the common case for assignment RHS).
                // Without the unspan, plain `label = t.0` (where `t` is
                // a tuple with a text element) emitted
                // `let mut var_label: String = &var_t.0.to_string();` —
                // E0308 because `&var_t.0.to_string()` parses as
                // `&(var_t.0.to_string())` per Rust method-call
                // precedence (`&String` vs declared `String`).
                let to_inner = to.unspan();
                let tuple_text_elem_clone = needs_to_string
                    && matches!(to_inner, Value::TupleGet(v, idx) if {
                        let vars = self.data.def(self.def_nr).variables();
                        // loft#1038 — the SAME predicate the read arm emits from
                        // (`tuple_elem_is_text`).  Re-deriving it here with an unpeeled
                        // `Type::Text` match is how the two came apart: a `text?`
                        // element was "text" at neither site, so the read moved it; had
                        // only one been fixed, the read would borrow and this site
                        // would still append `.to_string()` to a borrow — E0308 rather
                        // than E0382, the same program refused for a new reason.
                        !vars.is_argument(*v)
                            && crate::generation::tuple_elem_is_text(vars, *v, u32::from(*idx))
                    });
                // P247 — destination is a tuple-typed work var (e.g.
                // `__ref_N: (i64, String)` materialised by the
                // operators.rs nested-TupleGet read path) AND the
                // source is a `TupleGet(parent, idx)` whose result
                // type contains a non-Copy leaf (Text / Reference).
                // Default emission `let __ref_N = var_t.0;` MOVES
                // `var_t.0`, invalidating subsequent reads of
                // `var_t.0.X` in the same expression.  Emit
                // `var_t.0.clone()` instead so each chained access
                // gets its own owned copy.
                let nested_tuple_clone = matches!(variables.tp(var), Type::Tuple(elems)
                    if tuple_has_non_copy_leaf(elems))
                    && matches!(to_inner, Value::TupleGet(v, _) if {
                        let vars = self.data.def(self.def_nr).variables();
                        !vars.is_argument(*v) && matches!(vars.tp(*v), Type::Tuple(_))
                    });
                // loft#1325 — the WHOLE tuple, one level out from the arm above.  `u = a` over
                // a local `(text, text)` emitted `let mut var_u: (String, String) = var_a;`,
                // which MOVES it, so every later read of `var_a` is rustc E0382 and the
                // program does not build — while the interpreter runs it and answers what
                // `@FR-B-Copy` promises, an INDEPENDENT copy.  A backend that refuses what the
                // other computes is the divergence `formal/operational.md` D-op-1 forbids, and
                // the refusing side is the one that is wrong here: `B-Copy` says the bind is a
                // copy, so `.clone()` is the emission that keeps the promise.
                //
                // Only a non-Copy leaf needs it — an all-scalar tuple is `Copy` and the move is
                // a copy already — and only a LOCAL source: a tuple PARAMETER arrives borrowed
                // and is re-spelled by `tuple_arg_owned_elems` below, which owns that pair.
                let whole_tuple_clone = matches!(variables.tp(var), Type::Tuple(elems)
                    if tuple_has_non_copy_leaf(elems))
                    && matches!(to_inner, Value::Var(v) if {
                        let vars = self.data.def(self.def_nr).variables();
                        !vars.is_argument(*v) && matches!(vars.tp(*v), Type::Tuple(_))
                    });
                // loft#840 — the destination is an owned tuple slot holding text
                // (`(i64, String, u8)`) and the source is a tuple PARAMETER, which
                // the native backend passes borrowed (`(i64, &str, u8)`).  Nothing
                // else reconciles the two spellings, so the default emission
                // `let mut var_x: (i64, String, u8) = var_t;` is rustc E0308 and the
                // program does not build at all — while the interpreter, which has no
                // owned/borrowed split, runs it.  Every failing shape funnels through
                // here: a match temp, `local = t`, a struct field, and the synthetic
                // return work var all reach `set_var` with this exact pair.
                //
                // loft#1005 — and the same pair one level in.  A NESTED tuple parameter
                // (`p: ((integer, text), text)`) reaches its inner tuple as `TupleGet(p, 0)`,
                // not as a bare `Var`, so the whole-parameter rule above did not see it and
                // `let mut var___ref_1: (i64, String) = var_p.0;` was the identical E0308.
                // Both are one fact — a tuple crossing from a BORROWED parameter into an
                // OWNED slot — so they share the re-spelling and differ only in the place
                // they name.
                let tuple_arg_owned_elems = match variables.tp(var) {
                    Type::Tuple(elems) if tuple_has_text_leaf(elems) => {
                        let from_param = match to_inner {
                            Value::Var(v) => {
                                let vars = self.data.def(self.def_nr).variables();
                                vars.is_argument(*v) && matches!(vars.tp(*v).base(), Type::Tuple(_))
                            }
                            Value::TupleGet(v, _) => {
                                let vars = self.data.def(self.def_nr).variables();
                                vars.is_argument(*v) && matches!(vars.tp(*v).base(), Type::Tuple(_))
                            }
                            _ => false,
                        };
                        from_param.then(|| elems.clone())
                    }
                    _ => None,
                };
                if let Some(elems) = tuple_arg_owned_elems {
                    let place = match to_inner {
                        Value::Var(v) => Some(format!(
                            "var_{}",
                            sanitize(self.data.def(self.def_nr).variables().name(*v))
                        )),
                        Value::TupleGet(v, idx) => Some(format!(
                            "var_{}.{idx}",
                            sanitize(self.data.def(self.def_nr).variables().name(*v))
                        )),
                        _ => None,
                    };
                    if let Some(place) = place {
                        write!(w, "{}", owned_tuple_from_arg(&place, &elems))?;
                    }
                } else if text_local_clone {
                    if let Value::Var(v) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.clone()")?;
                    }
                } else if refvar_text_clone {
                    if let Value::Var(v) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.to_string()")?;
                    }
                } else if tuple_text_elem_clone {
                    // P228: read through the same unspan as the detection above.
                    if let Value::TupleGet(v, idx) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.{idx}.clone()")?;
                    }
                } else if nested_tuple_clone {
                    if let Value::TupleGet(v, idx) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.{idx}.clone()")?;
                    }
                } else if whole_tuple_clone {
                    if let Value::Var(v) = to.unspan() {
                        let src_name = sanitize(self.data.def(self.def_nr).variables().name(*v));
                        write!(w, "var_{src_name}.clone()")?;
                    }
                } else {
                    self.output_code_inner(w, to)?;
                }
                self.fn_ref_context = prev_ctx;
                self.tuple_text_to_string = prev_tuple_text;
                self.tuple_slot_types = prev_tuple_slots;
                if needs_to_string
                    && !text_local_clone
                    && !refvar_text_clone
                    && !tuple_text_elem_clone
                    && !nested_tuple_clone
                    && !whole_tuple_clone
                {
                    write!(w, ".to_string()")?;
                } else if wrap_fn_ref {
                    write!(w, " as u32, loft::keys::DbRef::NULL)")?;
                } else if matches!(variables.tp(var), Type::Routine(_))
                    && !matches!(to, Value::Null)
                {
                    write!(w, " as u32")?;
                } else if to != &Value::Null && narrow_int_cast(variables.tp(var)).is_some() {
                    // Variable is a narrow integer type, but the RHS expression
                    // (a function returning u16 or an iterator block returning as u16) produces
                    // the narrow type.  Post-2c: widen to i64 to match the default Integer.
                    // @PLN17: a boolean variable's storage form is u8 (not i64) — cast the
                    // RHS (`bool` literal/comparison or `u8`) to u8, not i64.
                    if matches!(variables.tp(var).base(), Type::Boolean) {
                        write!(w, ") as u8")?; // closes the `(` opened before the RHS
                    } else {
                        write!(w, " as i64")?;
                    }
                } else if widen_block {
                    // #433 — close the paren opened above and widen the narrow-int
                    // value-block to the `integer` (i64) variable's slot width.
                    write!(w, ") as i64")?;
                } else if let Value::Call(d_nr, _) = to.unspan() {
                    // When the variable type and the called function's return type differ
                    // (e.g., multiple parallel-for loops reusing `b` with different worker types),
                    // add a cast so Rust accepts the assignment.
                    let var_tp_str = rust_type(variables.tp(var), &Context::Variable);
                    let ret = self.data.def(*d_nr).returned();
                    let ret_str = rust_type(ret, &Context::Variable);
                    if ret_str != var_tp_str && !matches!(ret, Type::Void) {
                        write!(w, " as {var_tp_str}")?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Emit a null-initialised `DbRef` variable, matching the interpreter's pre-init order.
    ///
    /// `first == false` is an @P302 keyed-collection reassignment (`s = []`
    /// clear): the slot already holds a live `DbRef`, so emit `OpDatabase`
    /// ALONE (in-place clear, reuses the store, no leak) — skip `null_named`,
    /// which would reset to a sentinel and leak the old store.
    /// `lvalue` is the Rust place being initialised — `var_x` for an ordinary local, or
    /// `self.var_x` for a coroutine-persistent field.  `name` stays the loft variable's name,
    /// because it is only the debug label `null_named` records.
    fn emit_null_dbref(
        &mut self,
        w: &mut dyn Write,
        var: u16,
        name: &str,
        lvalue: &str,
        first: bool,
    ) -> std::io::Result<()> {
        let variables = self.data.def(self.def_nr).variables();
        // Only a slot that OWNS its store gets a backing allocation here.  Reading
        // the one `owns_store` predicate rather than re-deriving ownership from the
        // dep list is what keeps this correct for the borrows the deps cannot see: a
        // match-arm binding (`match e { V { v } => … }`) aliases the subject's
        // payload record and carries an EMPTY dep, so a dep-only test would allocate
        // a store the borrowing read immediately overwrites — orphaned, and never
        // freed because the binding emits no `OpFreeRef`.  Same for a vector-literal
        // element whose container is a field DbRef (loft#664).
        if variables.owns_store(var) {
            let ref_buf_type_id = {
                // @FR-L-Null — `base()`, because a nullable collection local owns the SAME
                // store its dense twin owns (layout(τ) = layout(τ?)).  Asked bare, a
                // `hash<S[k]>?` fell to the catch-all, got no `OpDatabase`, and the slot kept a
                // NULL DbRef — which `keys.rs` then refused as *"a NULL DbRef reached a store
                // accessor … the producer published an absent value where a real store was
                // required"* the moment an element was written.
                let var_tp = variables.tp(var).base().clone();
                match &var_tp {
                    Type::Vector(elm_tp, _) => {
                        let elm_name = elm_tp.name(self.data);
                        self.data.name_type(&format!("main_vector<{elm_name}>"), 0)
                    }
                    // P188: keyed-collection locals need an OpDatabase
                    // call against the specific keyed-collection type so
                    // the backing store is allocated and the root pointer
                    // is zero-initialised.  Resolves to the same database
                    // type id that struct-field registration uses.
                    Type::Sorted(td, key, _) | Type::Index(td, key, _) => {
                        let c = self.data.def(*td).known_type();
                        if c == u16::MAX {
                            u16::MAX
                        } else {
                            let prefix = match &var_tp {
                                Type::Sorted(_, _, _) => "sorted",
                                Type::Index(_, _, _) => "index",
                                _ => unreachable!(),
                            };
                            let mut name =
                                format!("{prefix}<{}[", self.stores.types[c as usize].name);
                            for (k_nr, (k, asc)) in key.iter().enumerate() {
                                if k_nr > 0 {
                                    name += ",";
                                }
                                if !*asc {
                                    name += "-";
                                }
                                name += k;
                            }
                            name += "]>";
                            self.stores.name(&name)
                        }
                    }
                    Type::Trie(td, key, _) => {
                        let c = self.data.def(*td).known_type();
                        if c == u16::MAX {
                            u16::MAX
                        } else {
                            // Same spelling `Stores::trie` registers, built here
                            // because this context holds `stores` immutably — a
                            // LOOKUP, not a registration.
                            let name =
                                format!("trie<{}[{key}]>", self.stores.types[c as usize].name);
                            self.stores.name(&name)
                        }
                    }
                    Type::Hash(td, key, _) | Type::Radix(td, key, _) => {
                        let c = self.data.def(*td).known_type();
                        if c == u16::MAX {
                            u16::MAX
                        } else {
                            let prefix = match &var_tp {
                                Type::Hash(_, _, _) => "hash",
                                Type::Radix(_, _, _) => "spatial",
                                _ => unreachable!(),
                            };
                            let mut name =
                                format!("{prefix}<{}[", self.stores.types[c as usize].name);
                            for (k_nr, k) in key.iter().enumerate() {
                                if k_nr > 0 {
                                    name += ",";
                                }
                                name += k;
                            }
                            name += "]>";
                            self.stores.name(&name)
                        }
                    }
                    _ => u16::MAX,
                }
            };
            if ref_buf_type_id == u16::MAX {
                // @P317 — a struct `Reference` local with no resolvable
                // backing-store type id: emit the NULL SENTINEL, not
                // `null_named` (which allocates a real store slot).  Every
                // use of such a local is preceded by either an `OpDatabase`
                // (which allocates fresh from the sentinel — see
                // codegen_runtime::OpDatabase's `store_nr == u16::MAX` arm)
                // or a reassignment from a call return (which supplies its
                // own store).  Allocating a `null_named` placeholder here
                // LEAKS one store per reassignment-from-call: the placeholder
                // slot is overwritten by the call's DbRef and never freed
                // (e.g. `nk = chunk_of(...)` first-assigned inside an `if` in
                // a loop — the C3-incremental store exhaustion that tripped
                // `assert!(store.free)`).  Matches the interpreter, which
                // null-inits ref locals to a sentinel, not an allocation.
                write!(w, "DbRef::NULL")?;
            } else if first {
                writeln!(w, "stores.null_named(\"var_{name}\");")?;
                self.indent(w)?;
                write!(
                    w,
                    "{lvalue} = OpDatabase(cell,{lvalue}, {ref_buf_type_id}_i32)"
                )?;
            } else {
                // @P302 reassignment / `s = []` clear: the slot already holds a
                // live DbRef → OpDatabase clears that store in place (no
                // null_named, which would reset to a sentinel and leak the
                // old store).  The caller already wrote the `var_{name} = `.
                write!(w, "OpDatabase(cell,{lvalue}, {ref_buf_type_id}_i32)")?;
            }
        } else {
            write!(w, "DbRef::NULL")?;
        }
        Ok(())
    }

    /// Use this to dispatch a `Value::Call` to either the user-function or template emitter.
    /// Certain built-in text operations are intercepted here because their generated Rust
    /// differs structurally from both a regular call and a template substitution.
    #[allow(clippy::too_many_lines)] // large opcode dispatch — splitting would lose context
    pub(super) fn output_call(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
        vals: &[Value],
    ) -> std::io::Result<()> {
        // clear fn_ref_context inside calls — arguments like OpDatabase's
        // type number are plain integers, not fn-ref d_nr values.
        let saved_ctx = self.fn_ref_context;
        self.fn_ref_context = false;
        // P238: clear tuple_text_to_string inside calls — call arguments
        // bind to the callee's parameter signature (typically `&str` for
        // text params), not to the outer assignment target's tuple slot
        // type.  Without this clear, `let var_s: (String, String) =
        // t_4text_pair_t(cell, "hi")` would emit `"hi".to_string()` for
        // the arg because the outer `(String, String)` flag propagated
        // into `Value::Text` rendering.
        let saved_tuple = self.tuple_text_to_string;
        self.tuple_text_to_string = false;
        let result = self.output_call_inner(w, def_nr, vals);
        self.fn_ref_context = saved_ctx;
        self.tuple_text_to_string = saved_tuple;
        result
    }

    /// Does this call ARGUMENT still mutate a store at the point it is emitted, so that
    /// leaving it inline would double-borrow `stores`?
    ///
    /// The raw IR is the wrong thing to ask.  `pre_eval` runs first and hoists exactly these
    /// arguments into `let _pre_N = …;` bindings, after which
    /// [`Output::output_code`] substitutes the NAME — so an argument already in
    /// `active_pre_eval` contributes no mutation here, whatever its IR says.
    ///
    /// Asking the IR made the text-level hoist below fire on top of the IR-level one, and that
    /// is not merely redundant: the branch it guards writes the call ITSELF and so never
    /// reaches the op registry.  `OpGetRecord`'s emitter — the one that reads the key types off
    /// the store and builds `&[Content::…]` — was skipped, and its four-parameter runtime fn
    /// was handed the IR's `[data, db_tp, count, key…]` verbatim:
    /// `OpGetRecord(cell, _harg_s_0, 80_i32, 1_i32, 3_i64)`, rejected by rustc as E0061
    /// (loft#1217).  Every registered emitter shares the exposure, because the branch is gated
    /// on `rust().is_empty()` and that does not exclude a registry-owned Op; `OpGetRecord` is
    /// only the one whose emitter changes the ARITY, so it is the one that failed loudly.
    fn arg_needs_text_hoist(&self, arg: &Value) -> bool {
        !self
            .active_pre_eval
            .contains_key(&(std::ptr::from_ref(arg) as usize))
            && contains_op_database(IrNode::Native(arg), self.data)
    }

    #[allow(clippy::too_many_lines)]
    fn output_call_inner(
        &mut self,
        w: &mut dyn Write,
        def_nr: u32,
        vals: &[Value],
    ) -> std::io::Result<()> {
        let def_fn = self.data.def(def_nr);
        let name: &str = def_fn.name();
        // Phase 09 phase 00 step 0.6: registry-first dispatch.  When a
        // custom emitter is registered for this Op, run it instead of
        // the special-case match arms below.  Today the registry is
        // empty so every Op falls through to the existing dispatch.
        // Future phases register per-Op emitters that take over these
        // emissions one Op at a time (without touching the bulk match).
        if crate::generation::ops::has_custom_emitter(name) {
            let name_owned = name.to_string();
            let mut ctx = crate::generation::ops::EmitCtx {
                w,
                def_fn,
                output: self,
            };
            return crate::generation::ops::emit_op(&mut ctx, &name_owned, vals);
        }
        if def_fn.rust().is_empty() {
            self.output_call_user_fn(w, def_fn, vals)
        } else {
            self.output_call_template(w, def_fn, vals)
        }
    }
}

/// True iff `elems` contains a `Type::Text` element at any depth —
/// includes text reached through nested `Type::Tuple`.  Used by
/// `tuple_text_to_string` activation in `output_set` so a tuple
/// destination like `((i64, String), (i64, String))` triggers the
/// `"a".to_string()` wrap on the inner literals (plan-14 phase 02).
pub(crate) fn tuple_has_text_leaf(elems: &[Type]) -> bool {
    for e in elems {
        // `.base()` peels `Optional`: a `text?` element occupies the same owned
        // `String` slot a `text` does, so a tuple carrying one has a text leaf.
        // Without the peel `-> (text?, integer)` was invisible here, the owned-text
        // flag never fired for it, and `--native` refused the function with E0308 —
        // for a plain declaration, not only a generic one.
        match e.base() {
            Type::Text(_) => return true,
            Type::Tuple(inner) if tuple_has_text_leaf(inner) => return true,
            _ => {}
        }
    }
    false
}

/// Re-spell a tuple ARGUMENT element-wise so it fits an owned tuple slot.
///
/// A tuple's Rust element types depend on the context it sits in: `text` is
/// `&str` in [`Context::Argument`] and `String` in [`Context::Variable`]
/// (`rust_type`).  Every other element type spells the same either way, so the
/// split is invisible until a tuple carrying text crosses from a parameter into
/// an owned slot — a local, a match temp, a struct field, or a return buffer —
/// and rustc rejects `(i64, &str, u8)` against `(i64, String, u8)` (loft#840).
///
/// `expr` is the Rust place holding the borrowed tuple (`var_t`, or a nested
/// `var_t.0`); the result borrows each text leaf into a fresh `String` and
/// passes every other leaf through untouched, so nothing is copied that the
/// owned slot did not already require.
fn owned_tuple_from_arg(expr: &str, elems: &[Type]) -> String {
    let parts: Vec<String> = elems
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let field = format!("{expr}.{i}");
            match e.base() {
                Type::Text(_) => format!("{field}.to_string()"),
                Type::Tuple(inner) => owned_tuple_from_arg(&field, inner),
                _ => field,
            }
        })
        .collect();
    format!("({})", parts.join(", "))
}

/// Re-spell a tuple ARGUMENT element-wise so it fits a BORROWED tuple parameter.
///
/// The mirror of [`owned_tuple_from_arg`], for the direction a call takes. `text` is
/// `String` in [`Context::Variable`] and `&str` in [`Context::Argument`] (`rust_type`), so a
/// tuple LOCAL is `(i64, String)` while the parameter it is passed to is `(i64, &str)`, and
/// rustc rejects the call. A tuple LITERAL argument is emitted in place and already spells
/// `&str`, which is why the literal form compiled and only the variable form did not
/// (loft#1005).
///
/// `expr` is the Rust place holding the owned tuple; each text leaf becomes `&*place`, which
/// derefs `String`, `Str` and `&str` alike — so this is also correct when the argument is
/// itself a parameter and already borrowed. Every other leaf passes through untouched.
///
/// Borrowing rather than cloning: the callee reads through the parameter, and a by-value
/// tuple parameter is a COPY in loft, so nothing the callee does to its own binding is
/// visible here. That keeps the call free of an allocation the language never asked for.
pub(crate) fn borrowed_tuple_from_owned(expr: &str, elems: &[Type]) -> String {
    let parts: Vec<String> = elems
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let field = format!("{expr}.{i}");
            match e.base() {
                Type::Text(_) => format!("&*{field}"),
                Type::Tuple(inner) => borrowed_tuple_from_owned(&field, inner),
                _ => field,
            }
        })
        .collect();
    format!("({})", parts.join(", "))
}

/// True iff `elems` contains a leaf type that doesn't impl `Copy` in
/// generated Rust — text, references, vectors, hash/index/sorted/
/// spatial, iterators, struct-enums.  Used by `nested_tuple_clone`
/// (P247) to decide whether `let __ref_N = var_t.0;` needs a
/// `.clone()` to avoid moving non-Copy data out of the parent tuple.
/// Plain integers / floats / booleans / characters / plain enums
/// are Copy and don't need cloning.
fn tuple_has_non_copy_leaf(elems: &[Type]) -> bool {
    for e in elems {
        match e {
            Type::Text(_)
            | Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Hash(_, _, _)
            | Type::Index(_, _, _)
            | Type::Sorted(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _)
            | Type::Iterator(_, _)
            | Type::Enum(_, true, _) => return true,
            Type::Tuple(inner) if tuple_has_non_copy_leaf(inner) => return true,
            _ => {}
        }
    }
    false
}
