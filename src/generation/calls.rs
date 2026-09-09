// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Function call code generation: user-defined functions and `#rust` template calls.

use crate::data::{Context, Data, Definition, Type, Value};
use crate::data_store::ValueType;
use crate::ir_node::IrNode;
use std::io::Write;

use super::{Output, narrow_int_cast, rust_type, sanitize};

/// True when the `@name` placeholder starting at `at` in `hay` is a WHOLE
/// token — the character right after it is not an identifier character.  This
/// is what stops `@lo` from matching inside `@local`: without the boundary
/// check the substring `@lo` is seen twice (once inside `@local`, once
/// standalone), which over-fires the repeated-use pre-eval and rewrites
/// `@local` to an unbound `_v_local` in the generated call.
fn placeholder_boundary_at(hay: &str, ph: &str, at: usize) -> bool {
    let after = at + ph.len();
    hay[after..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '_')
}

/// The `&mut` store helpers a template can call from inside a VALUE expression.
///
/// Read off the templates rather than listed as op names, so a new raising op
/// inherits the fix by spelling its guard the way every other one does. Shared
/// reads (`stores.store(&db)`) are deliberately absent: two-phase borrows already
/// allow one during argument evaluation, and treating them as conflicts hoists
/// constants and float literals for nothing.
///
/// A `&mut` use NOT on this list fails to compile at its own call site, loudly and
/// with rustc naming both borrows — the same way loft#818 was found. Extending the
/// list is then a one-line change.
const STORE_MUT_MARKERS: [&str; 3] = ["s.raise", "_or_raise", "store_mut"];

/// Can this argument expression take a MUTABLE borrow of the store while it is
/// evaluated?  loft#818.
///
/// Rust evaluates a method call's RECEIVER PLACE before its arguments, so a
/// template shaped `stores.method(@count)` holds `&mut *stores` while `@count`
/// runs.  An argument that itself calls a `&mut Stores` method — the
/// divide-by-zero guard behind `/` and `%`, the bounds guard behind `v[i]` — is
/// then a second mutable borrow and rustc rejects the whole function (E0499).
///
/// Answered from the IR rather than from generated text: the text does not exist
/// until the argument is emitted, and emitting it twice would duplicate the
/// pre-eval bindings it appends. A USER function is not a conflict — it is called
/// with `cell`, not with a live `&mut Stores` — so only `#rust` templates count.
fn may_borrow_store(v: &Value, data: &Data) -> bool {
    match v.unspan() {
        Value::Call(d, args) => {
            let rust = data.def(*d).rust();
            STORE_MUT_MARKERS.iter().any(|m| rust.contains(m))
                || args.iter().any(|a| may_borrow_store(a, data))
        }
        Value::CallRef(_, args) | Value::Insert(args) => {
            args.iter().any(|a| may_borrow_store(a, data))
        }
        Value::Set(_, inner) | Value::Return(inner) | Value::Drop(inner) => {
            may_borrow_store(inner, data)
        }
        Value::If(c, t, e) => {
            may_borrow_store(c, data) || may_borrow_store(t, data) || may_borrow_store(e, data)
        }
        Value::Block(b) | Value::Loop(b) => b.operators.iter().any(|s| may_borrow_store(s, data)),
        _ => false,
    }
}

/// Count whole-token occurrences of the placeholder `@name` in `hay`, skipping
/// any that sit inside a longer identifier (see [`placeholder_boundary_at`]).
fn count_placeholder(hay: &str, ph: &str) -> usize {
    let mut n = 0;
    let mut from = 0;
    while let Some(i) = hay[from..].find(ph) {
        let at = from + i;
        if placeholder_boundary_at(hay, ph, at) {
            n += 1;
        }
        from = at + ph.len();
    }
    n
}

/// Replace every whole-token occurrence of the placeholder `@name` with `with`,
/// leaving `@name` substrings inside a longer placeholder untouched (see
/// [`placeholder_boundary_at`]).  The token-aware counterpart of
/// `str::replace`, used for every `@param` substitution so that one attribute
/// name being a prefix of another (`lo` / `local`) can never corrupt the longer
/// one — regardless of the order the attributes are substituted in.
fn replace_placeholder(hay: &str, ph: &str, with: &str) -> String {
    let mut out = String::with_capacity(hay.len());
    let mut from = 0;
    while let Some(i) = hay[from..].find(ph) {
        let at = from + i;
        out.push_str(&hay[from..at]);
        if placeholder_boundary_at(hay, ph, at) {
            out.push_str(with);
        } else {
            out.push_str(ph);
        }
        from = at + ph.len();
    }
    out.push_str(&hay[from..]);
    out
}

/// Check if a Value tree contains a Call to OpDatabase (which mutates stores).
pub(super) fn contains_op_database(node: IrNode, data: &Data) -> bool {
    match node.kind() {
        ValueType::Call => {
            if data.def(node.call_to()).name() == "OpDatabase" {
                return true;
            }
            node.call_args()
                .iter()
                .any(|a| contains_op_database(a, data))
        }
        ValueType::Block => node
            .as_block()
            .operators()
            .iter()
            .any(|v| contains_op_database(v, data)),
        ValueType::Insert => node
            .insert_items()
            .iter()
            .any(|v| contains_op_database(v, data)),
        ValueType::Set => contains_op_database(node.set_inner(), data),
        ValueType::If => {
            contains_op_database(node.if_cond(), data)
                || contains_op_database(node.if_then(), data)
                || contains_op_database(node.if_else(), data)
        }
        _ => false,
    }
}

impl Output<'_> {
    /// Emit a user-defined function call (or Op-stub call without a `#rust`
    /// template).  Public entry point — dispatches through
    /// `emit_op` so custom emitters can override per Op.
    /// Default emitter delegates back to [`Self::user_fn_call_body`].
    pub(super) fn output_call_user_fn(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        vals: &[Value],
    ) -> std::io::Result<()> {
        let name = def_fn.name().to_string();
        let mut ctx = crate::generation::ops::EmitCtx {
            w,
            def_fn,
            output: self,
        };
        crate::generation::ops::emit_op(&mut ctx, &name, vals)
    }

    /// Internal helper: emits the user-fn / Op-stub call body.  Reachable
    /// from `crate::generation::ops::default::DefaultEmitter` when
    /// `def_fn.rust.is_empty()`.  Behaviour is byte-identical to the
    /// pre-phase-09 `output_call_user_fn`.
    pub(super) fn user_fn_call_body(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        vals: &[Value],
    ) -> std::io::Result<()> {
        // N8b.2: generator function calls produce a DbRef via the coroutine table.
        let is_generator = matches!(def_fn.returned(), Type::Iterator(_, _));
        if is_generator {
            write!(w, "loft::codegen_runtime::alloc_coroutine(")?;
        }
        // P199 — user-defined functions, generated stubs, AND Op stubs in
        // `src/codegen_runtime.rs` all take `&UnsafeCell<Stores>` (Track 1).
        // Plan 09 phase 01 added per-fn ABI tagging via the
        // `CODEGEN_RUNTIME_FNS` registry.  `abi_of(name)` returns:
        //   - `Cell`  → emit `name(cell, args...)`     (default)
        //   - `None`  → emit `name(args...)`           (no implicit Stores)
        let abi = if *def_fn.code() == Value::Null {
            crate::codegen_runtime::abi_of(def_fn.name())
        } else {
            crate::codegen_runtime::Abi::Cell
        };
        // @PLN157 § V-p — the TWIN form (`@FR-R-Inputs`): every invariant input of the callee
        // is a local the enclosing frames hold for the argument variable.  The definition
        // number comes from `output_call_inner`; the identity check keeps a stale one from
        // naming another function's twin.
        let twin_args = if (self.current_call_def as usize) < self.data.definitions.len()
            && std::ptr::eq(self.data.def(self.current_call_def), def_fn)
        {
            self.twin_call_inputs(self.current_call_def, vals)
        } else {
            None
        };
        write!(
            w,
            "{}{}(",
            self.fn_ident(def_fn),
            if twin_args.is_some() { "__inv" } else { "" }
        )?;
        let mut first_arg = true;
        if matches!(abi, crate::codegen_runtime::Abi::Cell) {
            write!(w, "cell")?;
            first_arg = false;
        }
        for (idx, v) in vals.iter().enumerate() {
            if !first_arg {
                write!(w, ", ")?;
            }
            first_arg = false;
            self.emit_call_arg(w, def_fn, idx, v)?;
        }
        if let Some(extra) = &twin_args {
            for e in extra {
                write!(w, ", {e}")?;
            }
        }
        write!(w, ")")?;
        if is_generator {
            write!(w, ")")?; // close alloc_coroutine(...)
        } else if narrow_int_cast(def_fn.returned()).is_some()
            && !matches!(def_fn.returned().base(), Type::Boolean)
        {
            // Narrow integer return types (u8/u16/i8/i16) must be widened so that
            // assignments and comparisons with default-Integer expressions type-check.
            // Post-2c: widen to i64 (the default Integer width).
            // @PLN17: boolean's expression form is u8/bool, never i64 — no widening
            // (incl. `boolean?`: its slot is u8, so `.base()` excludes it here too).
            write!(w, " as i64")?;
        }
        Ok(())
    }

    /// Emit ONE call argument (no leading separator) — the argument at
    /// position `idx` in `def_fn`'s parameter list, given source value `v`.
    ///
    /// This is the single source of truth for argument emission, shared by
    /// the normal call path ([`Self::user_fn_call_body`]) and the ABI-B
    /// deep-copy call path (`output_set_inner` in `dispatch.rs`).  Keeping
    /// it in one place is load-bearing: when the two paths drifted, a
    /// boolean literal arg on the ABI-B path emitted Rust `true`/`false`
    /// against a `u8` parameter and tripped rustc E0308 (issue #366).  All
    /// per-parameter coercions (boolean→`u8`, narrow-int, text deref,
    /// typed-null, fn-ref / routine wrapping, `&`-param forwarding) live
    /// here so both paths apply them identically.
    pub(super) fn emit_call_arg(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        idx: usize,
        v: &Value,
    ) -> std::io::Result<()> {
        if let Some(vr) = self.create_stack_var(v) {
            let name = sanitize(self.data.def(self.def_nr).variables().name(vr));
            write!(w, "&mut var_{name}")?;
        // OpCreateStack wrapping an addressable expression
        // (e.g. v[i] as & param).  Emit a temporary + &mut so the
        // callee can write through the DbRef into the store.
        } else if let Value::Call(d_nr, args) = v.unspan()
            && self.data.def(*d_nr).name() == "OpCreateStack"
            && args.len() == 1
            && !matches!(&args[0], Value::Var(_))
        {
            let expr = self.generate_expr_buf(&args[0])?;
            write!(w, "&mut ({expr})")?;
        } else if idx < def_fn.attributes().len()
            && matches!(def_fn.attributes()[idx].typedef, Type::RefVar(_))
            && let Value::Var(nr) = v
            && matches!(
                self.data.def(self.def_nr).variables().tp(*nr),
                Type::RefVar(_)
            )
        {
            // forwarding a & parameter to another & parameter.
            let caller_vars = self.data.def(self.def_nr).variables();
            let name = sanitize(caller_vars.name(*nr));
            if caller_vars.is_argument(*nr) {
                // An argument RefVar is already &mut DbRef — pass it
                // directly instead of dereferencing with *var_name.
                write!(w, "var_{name}")?;
            } else if crate::generation::is_raw_tuple_link(caller_vars, *nr) {
                // A `&`-bound tuple LOCAL holds a raw `*mut (…)`, so `&mut var_b` would
                // hand the callee a reference to the POINTER.  Re-borrow through it to
                // give the `&(…)` parameter the `&mut (…)` it declares — the caller's
                // own tuple, which is what the callee is there to write.
                write!(w, "unsafe {{ &mut *var_{name} }}")?;
            } else {
                // A local RefVar alias (#257) is a plain `DbRef` — borrow
                // it so the callee gets the &mut DbRef it expects.
                write!(w, "&mut var_{name}")?;
            }
        } else if matches!(v.unspan(), Value::Null)
            && idx < def_fn.attributes().len()
            && !matches!(def_fn.attributes()[idx].typedef, Type::Function(_, _, _))
        {
            // #307 — a `Value::Null` argument (a parser-filled default for
            // an omitted parameter, or an explicit `null`) renders as `()`
            // through `output_code_inner`, which rustc rejects at any typed
            // parameter (E0308: expected `&str`, found `()` on
            // `render("a", "b")`).  Emit the parameter-typed null sentinel
            // instead — the same repair the interpreter call paths apply
            // via `emit_typed_null`.  Fn-ref params keep the generic path
            // (their two-part (d_nr, closure) form is handled below).
            Self::write_typed_null_in(w, &def_fn.attributes()[idx].typedef, true)?;
        } else {
            // wrap i32 literal into (u32, null_DbRef) for fn-ref params.
            let param_is_fnref = idx < def_fn.attributes().len()
                && matches!(def_fn.attributes()[idx].typedef, Type::Function(_, _, _));
            let param_is_routine = idx < def_fn.attributes().len()
                && matches!(def_fn.attributes()[idx].typedef, Type::Routine(_));
            if param_is_fnref && matches!(v, Value::Int(_)) {
                let mut buf = Vec::new();
                self.output_code_inner(&mut buf, v)?;
                let s = String::from_utf8(buf).unwrap();
                write!(w, "({s} as u32, loft::keys::DbRef::NULL)")?;
            } else if param_is_routine && matches!(v, Value::Int(_)) {
                let mut buf = Vec::new();
                self.output_code_inner(&mut buf, v)?;
                let s = String::from_utf8(buf).unwrap();
                write!(w, "{s} as u32")?;
            } else {
                // B7-native: text-returning user fn calls produce `Str`,
                // but callees expect `&str`.  Wrap with `&*` to deref.
                // P262: must unspan() before matching — the parser
                // wraps inline call expressions in Value::Span (for
                // source-position tracking), and the bare matches!
                // pattern does not see through Span.  Without this,
                // the consuming-call argument site emitted the raw
                // `Str`-returning call expression with no deref,
                // tripping rustc E0308 ("expected `&str`, found `Str`").
                let v_unspanned = v.unspan();
                // @P299 — a text-returning fn-ref call (`CallRef`) lowers to
                // a `match` block whose arms yield owned `String` (the
                // candidate results are unified via `.to_string()`).  A
                // callee param of `&str` needs the same `&*` deref as a
                // direct text-returning `Call`.  Without this, a text
                // closure called through a struct field DIRECTLY as an
                // argument (`print(g.fmt(7))`) trips rustc E0308
                // (`expected &str, found String`) — binding the result to a
                // local first sidesteps it, which is why it surfaced late.
                // The CallRef is usually wrapped in a work-buffer `Block`
                // (P227's `cref_work_buf` materialisation), so walk Block
                // tails to find the text-returning fn-ref call.
                let arg_is_text_callref = {
                    let mut probe = v_unspanned;
                    loop {
                        match probe {
                            Value::CallRef(vr, _) => {
                                // @PLN25 — `.base()` so a `text?` return derefs too.
                                break matches!(
                                    self.data.def(self.def_nr).variables().tp(*vr),
                                    Type::Function(_, ret, _) if matches!(ret.base(), Type::Text(_))
                                );
                            }
                            Value::Block(b) => match b.operators.last() {
                                Some(op) => probe = op.unspan(),
                                None => break false,
                            },
                            _ => break false,
                        }
                    }
                };
                // @P304 — fire for ANY non-`Op` text-returning call, USER
                // or NATIVE.  A native (`#rust`) text fn returns owned
                // `String` (e.g. `store_memory()` → `stores.memory_report()`),
                // so it needs the `&*` deref into `&str` just like a user
                // fn's `Str` return — the old `rust.is_empty()` clause wrongly
                // excluded native fns (E0308 `expected &str, found String`).
                // `&*` is safe for all text shapes (`String`/`Str`/`&str` are
                // `Deref<Target=str>`).  `Op*` runtime helpers stay excluded.
                // @P386: a text-result `Value::Block` argument whose body
                // contains the `__ncc_*` skip_free pattern emits its tail
                // as `_ret.to_string()` (owned `String`, see
                // `output_block`'s @P321e/@P323 materialisation at
                // emit.rs:~1466).  Direct-call `&str` params then trip
                // rustc E0308 ("expected `&str`, found `String`").  Wrap
                // with `&*` (which derefs both `String` and `Str` to
                // `&str`).  Same idea as the existing CallRef detection
                // above, just applied at the Block level.
                // @PLN25 — `.base()` throughout: an `Optional(Text)` return /
                // param shares the same `String`/`Str` shapes, so a `text?`
                // call result passed to a `&str` param needs the identical
                // `&*` deref (else rustc E0308 "expected &str, found String").
                let arg_is_text_block_string = matches!(v_unspanned, Value::Block(b)
                        if matches!(b.result.base(), Type::Text(_))
                            && self.block_contains_ncc_skip_free(b));
                let needs_deref = idx < def_fn.attributes().len()
                    && matches!(def_fn.attributes()[idx].typedef.base(), Type::Text(_))
                    && (arg_is_text_callref
                        || arg_is_text_block_string
                        || matches!(v_unspanned, Value::Call(d, _) if
                                matches!(self.data.def(*d).returned().base(), Type::Text(_))
                                && !self.data.def(*d).name().starts_with("Op")));
                // Post-2c: Op* runtime helpers (defined in
                // `src/codegen_runtime.rs`) keep hand-written i32 params
                // for tp-numbers / field offsets / flag enums.  When the
                // loft param is a narrow Integer (u8/u16/i8/i16), those
                // Op* signatures take `i32`.  Emit any descendant
                // Value::Int as `_i32` instead of the post-2c `_i64`
                // default so the literal lands at the expected width.
                // User-defined fns have their Rust signatures generated
                // from `rust_type`, which widens narrow Integer to i64
                // in Context::Argument — so this narrow-match only
                // applies to Op-prefixed runtime calls.
                let param_is_narrow = def_fn.name().starts_with("Op")
                    && idx < def_fn.attributes().len()
                    && narrow_int_cast(&def_fn.attributes()[idx].typedef).is_some();
                // @PLN17: a boolean param's storage form is u8 (null-capable);
                // a `bool` literal / comparison arg must coerce.  `((arg) as u8)`
                // is idempotent for a u8 arg, 0/1 for a bool.
                let param_is_boolean = idx < def_fn.attributes().len()
                    && matches!(def_fn.attributes()[idx].typedef.base(), Type::Boolean);
                // loft#1005 — a tuple ARGUMENT carrying text has to be re-spelled to reach a
                // tuple PARAMETER.  `text` is `String` in a variable slot and `&str` in a
                // parameter (`rust_type`), so a tuple local is `(i64, String)` against a
                // parameter of `(i64, &str)` and rustc refused the whole program.  A tuple
                // LITERAL argument is emitted in place and already spells `&str`, which is
                // why only the variable form failed.
                //
                // Applied to a PLACE (a variable), not to an arbitrary expression: the
                // re-spelling names `expr.0` / `expr.1`, so it must be something that can be
                // projected more than once without re-evaluating it.  A literal needs no
                // conversion anyway, and a call result is materialised into a local before
                // it gets here.
                let tuple_arg_place = match (v_unspanned, idx < def_fn.attributes().len()) {
                    (Value::Var(var), true) => {
                        match (
                            &def_fn.attributes()[idx].typedef,
                            self.data.def(self.def_nr).variables().tp(*var),
                        ) {
                            (Type::Tuple(param_elems), Type::Tuple(_))
                                if crate::generation::dispatch::tuple_has_text_leaf(
                                    param_elems,
                                ) =>
                            {
                                let name = self
                                    .data
                                    .def(self.def_nr)
                                    .variables()
                                    .name(*var)
                                    .to_string();
                                Some(crate::generation::dispatch::borrowed_tuple_from_owned(
                                    &format!("var_{name}"),
                                    param_elems,
                                ))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if needs_deref {
                    write!(w, "&*(")?;
                }
                if param_is_boolean {
                    write!(w, "((")?;
                }
                if let Some(respelled) = tuple_arg_place {
                    write!(w, "{respelled}")?;
                } else if param_is_narrow {
                    self.emit_i32_slot(w, v)?;
                } else {
                    self.output_code_inner(w, v)?;
                }
                if param_is_boolean {
                    write!(w, ") as u8)")?;
                }
                if needs_deref {
                    write!(w, ")")?;
                }
            }
        }
        Ok(())
    }

    /// Use this to inline a `#rust` template operator by substituting `@param` placeholders
    /// with generated argument expressions.
    ///
    /// Phase 09 phase 00 step 0.4: dispatches through
    /// `crate::generation::ops::emit_op`.  When no custom emitter is
    /// registered for `def_fn.name`, `emit_op` falls through to
    /// `DefaultTemplateEmitter` which calls back into
    /// [`Self::substitute_template_body`] — the byte-identical
    /// extraction of the original substitution body.  Result: every
    /// existing call site sees the same emission as before.
    pub(super) fn output_call_template(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        vals: &[Value],
    ) -> std::io::Result<()> {
        // Clone the name so we can pass it to `emit_op` without
        // co-borrowing `def_fn` from inside the EmitCtx.
        let name = def_fn.name().to_string();
        let mut ctx = crate::generation::ops::EmitCtx {
            w,
            def_fn,
            output: self,
        };
        crate::generation::ops::emit_op(&mut ctx, &name, vals)
    }

    /// Internal helper holding the actual `#rust` template substitution
    /// logic.  Reachable by name from [`Self::output_call_template`] (the
    /// pre-step-0.4 caller) and from
    /// `crate::generation::ops::default::DefaultTemplateEmitter` (the
    /// post-step-0.4 fall-through).
    ///
    /// Behaviour exactly matches the pre-phase-09 `output_call_template`.
    /// The byte-identical golden corpus at `/tmp/p09-baseline/*.rs` is
    /// the regression oracle.
    #[allow(clippy::too_many_lines)]
    pub(super) fn substitute_template_body(
        &mut self,
        w: &mut dyn Write,
        def_fn: &Definition,
        vals: &[Value],
    ) -> std::io::Result<()> {
        let mut res = def_fn.rust().to_string();
        // @P361 — a `#rust` body that names the loft-internal pub modules
        // `crate::rpc::` / `crate::store::` must qualify them `loft::…` in
        // generated code (in a generated binary `crate::` is the binary's own
        // root, with no `rpc`/`store` module; `crate::` resolves to loft only
        // in the interpreter's native registry where crate == loft).  That
        // rewrite is NO LONGER done here per-call-site: it missed an emission
        // path (the nightly index-hygiene leg's intermittent `error[E0433]:
        // cannot find rpc in crate`).  It now runs ONCE over the complete
        // assembled source at every native-emission entry — see
        // `scrub_generated_crate_refs` in `mod.rs`.  Host-import intrinsics
        // (`crate::loft_host_print`, `crate::wasm`) stay `crate::` there too.
        // @P321b — `OpSetText` with a NULL value must store the null pointer
        // (offset 0), matching the interpreter (which reads it back as null).
        // The generic template does `@val.to_string()` + `set_str`, which
        // renders `null` as `(()).to_string()` (E0599: `()` is not `Display`)
        // and would store a real string regardless.  Triggered by e.g.
        // `vec_of_text += null` (lib/arguments `flag()`: `self.results += null`).
        if def_fn.name() == "OpSetText"
            && let Some(vi) = def_fn.attributes().iter().position(|a| a.name == "val")
            && matches!(vals.get(vi), Some(Value::Null))
        {
            res = "{{let db = @v1; if db.rec != 0 {{ stores.store_mut(&db).set_u32_raw(db.rec, db.pos + u32::from(@fld), 0u32); }}}}".to_string();
        }
        // Bytecode templates wrap text values in Str::new(...) for put_stack compatibility.
        // Native code uses &str directly — strip the wrapper by extracting its argument.
        // Must be done before @param substitution so argument expressions are not affected.
        while let Some(start) = res.find("Str::new(") {
            let arg_start = start + "Str::new(".len();
            let mut depth = 1usize;
            let mut end = arg_start;
            for (i, c) in res[arg_start..].char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = arg_start + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            res = format!(
                "{}{}{}",
                &res[..start],
                &res[arg_start..end],
                &res[end + 1..]
            );
        }
        // P203 fix — bind repeated @<name> placeholders to a local so
        // side-effecting argument expressions (e.g. n_delete(...) compared to
        // an enum) evaluate exactly once.  For each attribute whose
        // @<name> appears two or more times in the template, replace every
        // occurrence with `_v_<name>` and prepend a `let _v_<name> = @<name>;`
        // wrapping `{ ... }`.  The substitution loop below then rewrites the
        // single remaining `@<name>` (the let-RHS) into the actual value
        // expression; subsequent uses read `_v_<name>` instead of re-evaluating.
        for a in def_fn.attributes() {
            let placeholder = format!("@{}", a.name);
            // Whole-token count: `@lo` must NOT be seen inside `@local`, or the
            // pre-eval fires spuriously and rewrites `@local` to an unbound
            // `_v_local`.  (See `count_placeholder` / `replace_placeholder`.)
            if count_placeholder(&res, &placeholder) >= 2 {
                let local = format!("_v_{}", a.name);
                res = replace_placeholder(&res, &placeholder, &local);
                res = format!("{{ let {local} = {placeholder}; {res} }}");
            }
        }
        // loft#818 — evaluate store-borrowing ARGUMENTS before the template takes
        // its own store borrow.
        //
        // The same defect has now been hand-patched into three templates one at a
        // time (`OpGetVector` @P321d, again @P338, then `reserve` on a hash), each
        // time by writing `{{let __x = @arg; …}}` into `default/01_code.loft`. The
        // producer is the GENERATOR, not any of those templates, so this is where
        // it is settled: whichever template comes next inherits the fix.
        //
        // A hoisted argument keeps its position in evaluation order, so every
        // argument up to the last one needing a hoist is hoisted too — otherwise
        // an earlier inline argument would run AFTER a later hoisted one, and two
        // arguments that both raise would report the wrong one first.
        //
        // TEXT blocks it. Binding a `text` argument to a local either MOVES a
        // `String` variable out of the caller's frame or borrows a temporary that
        // dies at the end of the `let` — so if a text argument sits before the one
        // that needs hoisting, nothing is hoisted and the call fails to compile
        // exactly as it does today. Loud, not silent, and `store_load_key(h, p(),
        // n / 2)` is what would show it.
        let hoist_upto = def_fn
            .attributes()
            .iter()
            .enumerate()
            .filter(|(i, a)| {
                *i < vals.len()
                    && count_placeholder(&res, &format!("@{}", a.name)) > 0
                    && may_borrow_store(&vals[*i], self.data)
            })
            .map(|(i, _)| i)
            .next_back();
        let hoist_upto = hoist_upto.filter(|last| {
            def_fn
                .attributes()
                .iter()
                .take(*last + 1)
                .all(|a| !matches!(a.typedef.base(), Type::Text(_)))
        });
        let mut prelude = String::new();
        // Bind `with` to a fresh local when this argument is hoisted, and answer
        // the text to substitute in its place. The local holds the RAW value; any
        // cast or wrapper the template needs is still applied at the use site, so
        // nothing about the emitted call's types changes.
        let hoisted = |a_nr: usize, with: String, prelude: &mut String| -> String {
            if hoist_upto.is_some_and(|last| a_nr <= last) {
                let local = format!("_ha{a_nr}");
                use std::fmt::Write as _;
                let _ = write!(prelude, "let {local} = {with}; ");
                local
            } else {
                with
            }
        };
        for (a_nr, a) in def_fn.attributes().iter().enumerate() {
            let name = "@".to_string() + &a.name;
            if a_nr < vals.len() {
                // loft#1234 — what a `null` looks like AT A PARAMETER'S TYPE has one home,
                // [`Self::write_typed_null_in`], and this is the second of the two places
                // that ask it.  The direct-call path below already routes here (#307); this
                // template path used to re-spell a SUBSET of the same list by hand — enums,
                // heap refs and booleans — and every type the hand-written list omitted fell
                // through to `generate_expr_buf`, which renders a bare `Value::Null` as `()`.
                //
                // So `c += null` emitted `let v = (()); … set_int(…, v)` and `--native`
                // refused to compile the program at `integer`, `float`, `single`,
                // `character` and narrow-int element types, while the interpreter appended
                // an absent element at every one of them.  A published package depends on
                // that append (`arguments 0.2.1`), so the two backends disagreed about a
                // program the registry ships.
                //
                // The lists had also already drifted: the enum arm matched `Enum(_, _, _)`,
                // so it claimed the struct-enum spelling `Enum(_, true, _)` and answered
                // `255u8` for a DbRef-backed parameter — which made the reference arm's own
                // `Enum(_, true, _)` case dead, and disagreed with what the direct-call path
                // emits for the very same type.  Asking the one home removes the subset and
                // the disagreement together.
                //
                // Fn-ref parameters keep the generic path: their two-part
                // `(d_nr, closure)` form is not a single sentinel, and the direct-call path
                // excludes them for the same reason.
                if matches!(vals[a_nr], Value::Null)
                    && !matches!(a.typedef, Type::Function(_, _, _))
                {
                    let mut buf: Vec<u8> = Vec::new();
                    Self::write_typed_null_in(&mut buf, &a.typedef, true)?;
                    let null_lit = String::from_utf8(buf).unwrap_or_else(|_| "()".to_string());
                    res = replace_placeholder(&res, &name, &format!("({null_lit})"));
                    continue;
                }
                // @PLN17 — boolean operands: the op `#rust` templates do u8
                // arithmetic (`@v != 1`, `@v1 == @v2`, `@v == 255`).  Rendered to the u8
                // storage form — a no-op cast for a `u8` variable, a `0/1` narrowing for a
                // transient `bool` sub-expression.  (The null case is the shared arm above.)
                if matches!(a.typedef.base(), Type::Boolean) {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    let inner = hoisted(a_nr, inner, &mut prelude);
                    res = replace_placeholder(&res, &name, &format!("(({inner}) as u8)"));
                    continue;
                }
                // For character-typed parameters, Value::Int means a character code point.
                if matches!(a.typedef, Type::Character)
                    && let Value::Int(n) = vals[a_nr]
                {
                    let with = format!("char::from_u32({n}_u32).unwrap_or('\\0')");
                    res = replace_placeholder(&res, &name, &format!("({with})"));
                    continue;
                }
                // A character-typed parameter wants a `char`; every operand
                // except the integer literal above arrives as the `i32`
                // STORAGE form — `rust_type` gives `character` an `i32` slot,
                // and a character-returning template is coerced `as u32 as i32`
                // at its own production site.  So the conversion is decided by
                // the operand's TYPE, not by which IR node produced it.
                //
                // It used to be decided by the node kind: four arms (`Var`,
                // `TupleGet`, `Call`, `Block`) each added when a new shape
                // turned up, and every shape outside the list emitted a bare
                // `i32` against a template that says `char::from(0)` — rustc
                // E0308, so the whole `--native` build failed.  The shape that
                // exposed it is the `?` discharge, which lowers to an `If`:
                // `fn f(a: character? = null) { (a?) == '\0' }` did not compile
                // at all (loft#1014).  An allow-list here costs correctness, not
                // just an optimisation, which is why it is a type test now.
                if matches!(a.typedef.base(), Type::Character)
                    && self
                        .infer_type(IrNode::Native(&vals[a_nr]))
                        .is_some_and(|t| matches!(t.base(), Type::Character))
                {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    let inner = hoisted(a_nr, inner, &mut prelude);
                    res = replace_placeholder(&res, &name, &format!("(ops::to_char({inner}))"));
                    continue;
                }
                // Text-typed parameters: all text-returning calls produce `Str` or `String`,
                // but templates expect `&str`. Deref with `&*` to get `&str` in all cases.
                // @PLN25 — `.base()` so `text?` params/returns take the same deref.
                if matches!(a.typedef.base(), Type::Text(_))
                    && let Value::Call(d, _) = vals[a_nr].unspan()
                    && matches!(self.data.def(*d).returned().base(), Type::Text(_))
                {
                    let inner = self.generate_expr_buf(&vals[a_nr])?;
                    res = replace_placeholder(&res, &name, &format!("(&*({inner}))"));
                    continue;
                }
                let mut with = self.generate_expr_buf(&vals[a_nr])?;
                with = hoisted(a_nr, with, &mut prelude);
                // Integer parameter receiving a char value needs explicit cast.
                if matches!(a.typedef, Type::Integer(_)) {
                    let val_is_char = match vals[a_nr].unspan() {
                        Value::Var(n) => {
                            matches!(
                                self.data.def(self.def_nr).variables().tp(*n),
                                Type::Character
                            )
                        }
                        Value::Call(d, _) => {
                            matches!(self.data.def(*d).returned(), Type::Character)
                        }
                        _ => false,
                    };
                    if val_is_char {
                        with += " as u32 as i32";
                    }
                    // P196: tuple element of fn-ref type lands as
                    // `(u32, DbRef)` in native — but the template
                    // expects an i64-shaped value (it does `@val ==
                    // i64::MIN` and `@val as i32`).  Project `.0` to
                    // get the `u32` d_nr and widen to i64 so the null
                    // check + narrow cast both compile.  The
                    // d_nr u32 can never equal i64::MIN, so the null
                    // branch is a tautological no-op — unavoidable
                    // until a fn-ref-aware OpSet variant exists.
                    let val_is_fn_ref_tuple_elem = match vals[a_nr].unspan() {
                        Value::TupleGet(var, idx) => {
                            match self.data.def(self.def_nr).variables().tp(*var) {
                                Type::Tuple(elems) => matches!(
                                    elems.get(*idx as usize),
                                    Some(Type::Function(_, _, _))
                                ),
                                _ => false,
                            }
                        }
                        _ => false,
                    };
                    if val_is_fn_ref_tuple_elem {
                        with = format!("(i64::from(({with}).0))");
                    }
                }
                // Templates use u32::from(@name) for field offsets; that was written for u16
                // parameters (fill.rs).  Native codegen emits i32 literals, so substitute the
                // entire u32::from(@name) pattern with (@value) as u32 to stay type-correct.
                let u32_from_pat = format!("u32::from({name})");
                if res.contains(&u32_from_pat) {
                    res = res.replace(&u32_from_pat, &format!("({with}) as u32"));
                } else {
                    // When the template parameter expects a narrow integer (u8/u16/i8/i16),
                    // native codegen's default emits the wrong width literal.  Patch the
                    // suffix (`_i32` / `_i64`) to match the narrow type, or add an explicit
                    // `as <narrow>` cast for non-literal expressions.
                    // Use Context::Result to get the precise narrow type (e.g. u16) since
                    // Context::Variable / Context::Argument widen to i32.
                    let tp_str = rust_type(&a.typedef, &Context::Result);
                    if matches!(tp_str.as_str(), "u8" | "u16" | "i8" | "i16") {
                        let typed_with = if with.ends_with("_i32") || with.ends_with("_i64") {
                            format!("{}_{tp_str}", &with[..with.len() - 4])
                        } else {
                            format!("({with}) as {tp_str}")
                        };
                        res = replace_placeholder(&res, &name, &format!("({typed_with})"));
                    } else {
                        res = replace_placeholder(&res, &name, &format!("({with})"));
                    }
                }
            } else {
                println!(
                    "Problem def_fn {def_fn} attributes {:?} vals {vals:?}",
                    def_fn.attributes()
                );
                break;
            }
        }
        // loft#818 — the hoisted arguments, in their original order, ahead of the
        // call that would otherwise have borrowed the store before evaluating them.
        if !prelude.is_empty() {
            res = format!("{{ {prelude}{res} }}");
        }
        // Templates use `s.database.` and `s.` for bytecode interpreter (State).
        // In generated native code, `stores` is the direct Stores reference.
        res = res.replace("s.database.", "stores.");
        res = res.replace("s.db_from_text(", "db_from_text(stores, ");
        res = res.replace("crate::state::", "loft::state::");
        // Plan-07 phase 4 — RuntimeErrorKind enum lives in
        // `loft::runtime_error` (the loft crate); annotations spell
        // it `crate::runtime_error::...` so the interpreter context
        // (where `crate` IS loft) compiles.  Native context's `crate`
        // is the user's binary; rewrite to the loft-qualified path
        // so rustc resolves it.  Mirrors the `crate::state::` rewrite
        // above.
        res = res.replace("crate::runtime_error::", "loft::runtime_error::");
        // Initiative 03 Phase 3b: const_refs lives on both `State`
        // (interpreter path) and `Stores` (mirrored for native).
        // Translate template references so OpConstRef / OpConstStoreText
        // resolve under `&mut Stores` in native code.  Both use the
        // `_runtime` suffix on the destination name so the substitution
        // can't re-match its own output when this op is nested inside
        // another op's template — without the suffix, the substring
        // `s.const_ref_at(` reappears at offset 5 in
        // `stores.const_ref_at(...)` and accumulates `stor` prefixes
        // (`storestores.const_ref_at`, `storestorestores...`).  Same
        // trick used for `s.raise(` → `stores.raise_runtime(` below.
        // @P275 fix.
        res = res.replace("s.const_ref_at(", "stores.const_ref_at_runtime(");
        res = res.replace(
            "s.string_from_const_store(",
            "stores.string_from_const_store_runtime(",
        );
        // Plan-07 phase 4c — Stores-side counterparts of the
        // State::raise / State::vec_get_or_raise / State::vec_ref_or_raise /
        // State::text_char_or_raise helpers (added 2026-05-11 by 4a).
        // The interpreter's `fn(s: &mut State)` dispatcher binds `s`;
        // the native context has only `stores: &mut Stores` (via
        // `unsafe { &mut *cell.get() }`).  Translate `s.X(...)` →
        // `stores.X_runtime(...)` so the same `default/01_code.loft`
        // annotations work in both contexts.  Closes the P204 / p200 /
        // native_tuple_* / native_binary_script regressions opened by
        // 4a's div/mod/vec/text annotation changes.
        //
        // The `_runtime` suffix is mandatory — a plain
        // `stores.raise(` substitution would let a second pass
        // re-match `s.raise(` inside the just-produced output and
        // accumulate `stor` prefixes (`storestores.raise(`,
        // `storestorestores.raise(`, …).  The suffix breaks the
        // substring relationship.  See `Stores::raise_runtime` for
        // the full explanation.
        //
        // Position info is dropped on the native path today (the
        // Stores-side helpers use `position: None`).  Phase 4g (backtrace
        // + polish) will thread codegen-time positions through.
        res = res.replace("s.raise(", "stores.raise_runtime(");
        // C80 / E-Uncomp — div-by-zero templates now `raise_recoverable`
        // (null-and-continue) instead of `raise`.  `s.raise(` above does NOT
        // match inside `s.raise_recoverable(` (an `_` follows `s.raise`, not
        // `(`), so this is an independent substitution; the `_runtime` suffix
        // again breaks the self-re-match (see `Stores::raise_recoverable_runtime`).
        res = res.replace("s.raise_recoverable(", "stores.raise_recoverable_runtime(");
        res = res.replace("s.vec_get_or_raise(", "stores.vec_get_or_raise_runtime(");
        res = res.replace("s.vec_ref_or_raise(", "stores.vec_ref_or_raise_runtime(");
        res = res.replace(
            "s.text_char_or_raise(",
            "stores.text_char_or_raise_runtime(",
        );
        // loft represents `character` as `i32`; template functions that return `char`
        // (like `ops::text_character`) need an explicit cast at the call site.
        // Narrow integer returns (u8/u16/i8/i16) must be widened so that
        // pre-eval bindings (`let _pre_N = narrow_func()`) do not cause type-mismatch
        // errors when compared against the default Integer width (post-2c: i64).
        // Multi-statement template bodies (containing `;`) are wrapped in `{...}` so
        // they are valid in expression position when inlined as function arguments.
        if matches!(def_fn.returned(), Type::Character) {
            write!(w, "({res}) as u32 as i32")
        } else if narrow_int_cast(def_fn.returned()).is_some()
            && !matches!(def_fn.returned().base(), Type::Boolean)
        {
            // @PLN17: boolean op results stay u8/bool — never widened to i64
            // (incl. `boolean?`: u8 slot, excluded via `.base()`).
            if res.contains(';') {
                write!(w, "({{{res}}}) as i64")
            } else {
                write!(w, "({res}) as i64")
            }
        } else if res.contains(';') {
            write!(w, "{{{res}}}")
        } else {
            write!(w, "{res}")
        }
    }
}

#[cfg(test)]
mod placeholder_tests {
    use super::{count_placeholder, replace_placeholder};

    // A `#rust` template like `stores.load_range(&(@local), @path, @lo, @hi)`:
    // the short `@lo` must not be seen inside the longer `@local`, or the
    // repeated-use pre-eval fires and rewrites `@local` to an unbound
    // `_v_local` (the native-codegen bug this file's helpers exist to prevent).
    #[test]
    fn short_name_is_not_counted_inside_a_longer_placeholder() {
        let body = "stores.load_range(&(@local), @path, @lo, @hi)";
        assert_eq!(count_placeholder(body, "@lo"), 1);
        assert_eq!(count_placeholder(body, "@local"), 1);
    }

    #[test]
    fn replace_leaves_the_longer_placeholder_intact() {
        let body = "f(@local, @lo)";
        assert_eq!(replace_placeholder(body, "@lo", "X"), "f(@local, X)");
        // The longer name still substitutes cleanly on its own turn.
        assert_eq!(replace_placeholder(body, "@local", "L"), "f(L, @lo)");
    }

    #[test]
    fn genuine_repeated_use_still_counts_and_replaces() {
        assert_eq!(count_placeholder("@v1 == @v1", "@v1"), 2);
        assert_eq!(replace_placeholder("@v1 == @v1", "@v1", "x"), "x == x");
        // `@v1` must not bleed into `@v10`.
        assert_eq!(replace_placeholder("@v1 @v10", "@v1", "x"), "x @v10");
    }
}
