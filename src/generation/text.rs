// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Text and format-string code generation helpers.

use crate::data::{Data, Type, Value};
use std::io::Write;

use super::Output;

/// O7: count the number of consecutive format/append ops in `ops[start..]`,
/// skipping `Value::Line` nodes.  Used by `clear_stack_text` to emit a
/// `String::with_capacity` hint when ≥ 2 segments are present.
pub(super) fn count_format_ops(ops: &[Value], start: usize, data: &Data) -> usize {
    ops[start..]
        .iter()
        .filter(|v| !matches!(v, Value::Line(_)))
        .take_while(|v| {
            if let Value::Call(d, _) = v.unspan() {
                let name = data.def(*d).name();
                name.starts_with("OpFormat") || name.starts_with("OpAppend")
            } else {
                false
            }
        })
        .count()
}

impl Output<'_> {
    pub(super) fn clear_vector(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
    ) -> std::io::Result<()> {
        if let [Value::Var(nr)] = vals {
            let v_nr = self.var_place(*nr);
            write!(
                w,
                "if {v_nr}.rec != 0 {{ vector::clear_vector(&{v_nr}, &mut stores.allocations); }}"
            )?;
            return Ok(());
        }
        // vector field whole-replacement (e.g. `b.data = f#read(...)`)
        // emits OpClearVector with a non-Var arg (typically OpGetField).
        // Compute the DbRef inline and call vector::clear_vector.
        if let [val] = vals {
            let expr = self.generate_expr_buf(val)?;
            write!(
                w,
                "{{ let _cv = {expr}; if _cv.rec != 0 {{ vector::clear_vector(&_cv, &mut stores.allocations); }} }}"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// `OpAppendVector(target, source, tp)` → `stores.vector_add(&t, &s, tp)`.
    ///
    /// #261: the default `#rust"s.database.vector_add(&@r, &@other, @tp)"`
    /// template inlines the target/source expressions into the `vector_add`
    /// call.  When the target is a vector ELEMENT (`c[i] += …`), that
    /// expression is an inline `stores.vec_get_or_raise_runtime(…)` read which
    /// borrows `*stores` mutably — colliding with `vector_add`'s own `&mut
    /// stores` borrow (E0499).  Hoist both operands into `let` temps first
    /// (each read borrows-then-releases), then call `vector_add` once.  Mirrors
    /// `clear_vector`'s non-Var path; harmless for the plain-local case.
    pub(super) fn append_vector(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
    ) -> std::io::Result<()> {
        if let [target, source, tp] = vals {
            let target_expr = self.generate_expr_buf(target)?;
            let source_expr = self.generate_expr_buf(source)?;
            let known_expr = self.generate_expr_buf(tp)?;
            write!(
                w,
                "{{ let _av_t = {target_expr}; let _av_s = {source_expr}; \
                 stores.vector_add(&_av_t, &_av_s, ({known_expr}) as u16); }}"
            )?;
            return Ok(());
        }
        panic!("OpAppendVector expects [target, source, tp], got {vals:?}");
    }

    /// #250: a RUNTIME expression resolving a nested-vector type-id from stable
    /// base ids via the idempotent, order-independent `stores.vector(_)`.
    ///
    /// Native codegen normally emits the parser's LITERAL vector type-id, but
    /// the native runtime registers vector types in a different order than the
    /// parser, so for 3+-deep `vector<vector<vector<X>>>` the literal id points
    /// at the wrong type (`copy_claims` then dispatches as e.g. a 7-variant
    /// enum → OOB / wrong value).  `stores.vector(elem)` returns the same id
    /// regardless of registration order, so rebuilding the chain at runtime
    /// (`stores.vector(stores.vector(<stable base id>))`) is divergence-proof.
    ///
    /// Returns `Some((depth, base_id))` where the type is `vector` applied
    /// `depth` times to a stable `base_id` — or `None` when the chain bottoms
    /// at a non-stable type (struct/enum/narrow-int, whose ids are NOT
    /// parser↔runtime stable) so the caller keeps the literal.  The caller
    /// emits a sequential `let _v0 = stores.vector(base); let _v1 =
    /// stores.vector(_v0); …` chain (one borrow at a time — a nested
    /// `stores.vector(stores.vector(..))` would double-borrow `*stores`).
    pub(super) fn vector_runtime_id_chain(&self, known: u16) -> Option<(usize, u16)> {
        let mut id = known;
        let mut depth = 0usize;
        loop {
            match &self.stores.types.get(id as usize)?.parts {
                crate::database::Parts::Vector(inner) => {
                    depth += 1;
                    id = *inner;
                }
                // Base types (integer/text/single/float/bool/char) occupy the
                // fixed low ids the native init binds as `let tN: u16 = N` —
                // parser id == runtime id, so they are the chain's stable floor.
                crate::database::Parts::Base => return Some((depth, id)),
                _ => return None,
            }
        }
    }

    /// Use this to emit a single key value as a typed `Content::…` constructor.
    /// `type_nr` is from a `Key` struct; sign indicates sort direction (ignored here),
    /// absolute value indicates the data type: 1 = integer, 2 = long, 3 = f32, 4 = f64,
    /// 5 = bool, 6 = text, 7 = byte, 8/9/10/11/12 = the narrow integer storage widths
    /// (`Parts::Int` / `Short` / `Byte` / `ShortRaw` / `IntRaw`).
    ///
    /// Every integer width answers a `Content::Long(i64)`, which is what the runtime
    /// side reconstructs from the record (`keys::get_key`) and hashes (`keys::hash_ref`);
    /// the WIDTH lives in the stored record, not in the lookup key.  loft#811: the four
    /// narrow widths were missing here and fell to the catch-all, so a lookup in a
    /// collection with a `u8`/`i16`/`i32` KEY searched for `Content::Long(0)` on
    /// `--native` — a `hash<T[k]>` missed every present record and a `sorted<T[k]>`
    /// answered a reference whose fields all read null, while the interpreter (which
    /// shares `get_key` for both sides) was correct.
    pub(super) fn emit_content(
        &mut self,
        w: &mut dyn Write,
        v: &Value,
        type_nr: i8,
    ) -> std::io::Result<()> {
        let expr = self.generate_expr_buf(v)?;
        match type_nr.unsigned_abs() {
            1 | 5 | 7 | 8 | 9 | 10 | 11 | 12 => write!(w, "Content::Long({expr} as i64)"),
            2 => write!(w, "Content::Long({expr})"),
            3 => write!(w, "Content::Single({expr})"),
            4 => write!(w, "Content::Float({expr})"),
            6 => write!(w, "Content::Str(Str::new(&*({expr})))"),
            _ => write!(w, "Content::Long(0)"),
        }
    }

    /// Use this to emit `OpClearStackText` as a `.clear()` call on the target string variable.
    ///
    /// O7: when the block lookahead (`next_format_count`) indicates ≥ 2 format/append ops
    /// follow, emits a `with_capacity` guard instead of a bare `.clear()` to avoid repeated
    /// `String` reallocations during format-string assembly (significant in WASM linear memory).
    pub(super) fn clear_stack_text(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
    ) -> std::io::Result<()> {
        if let [Value::Var(nr)] = vals {
            let variables = self.data.def(self.def_nr).variables();
            let s_nr = self.var_place(*nr);
            // When the variable is a `&mut String` parameter (RefVar(Text)), the capacity
            // re-allocation assignment needs an explicit dereference; auto-deref does not
            // apply to assignment left-hand sides in Rust.
            // loft#1371 — the LOCAL `&text` link needs it for the same reason the parameter
            // does: the destination is the link, and an assignment left-hand side does not
            // auto-deref.  Its raw-pointer `unsafe` block is opened by the op dispatch.
            let deref = if matches!(variables.tp(*nr), Type::RefVar(_)) {
                "*"
            } else {
                ""
            };
            let n = self.next_format_count;
            self.next_format_count = 0;
            if n > 1 {
                // avg_element_len = 8 is a conservative estimate for mixed text/integer fields.
                write!(
                    w,
                    "{{ let _cap = {n}_usize * 8; \
                     if {s_nr}.capacity() < _cap \
                     {{ {deref}{s_nr} = String::with_capacity(_cap); }} \
                     else {{ {s_nr}.clear(); }} }}"
                )?;
            } else {
                write!(w, "{s_nr}.clear()")?;
            }
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpAppendCharacter` with a null-character guard,
    /// because loft represents characters as integers and zero means no character.
    pub(super) fn append_character(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
    ) -> std::io::Result<()> {
        if let [Value::Var(nr), val] = vals {
            let s_nr = self.var_place(*nr);
            let val_expr = self.generate_expr_buf(val)?;
            write!(
                w,
                "{{let c = {val_expr}; if c != 0 {{ {s_nr}.push(ops::to_char(c)); }} }}"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpAppendText` as a `+=` on the target string variable.
    pub(super) fn append_text(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [Value::Var(nr), val] = vals {
            let s_nr = self.var_place(*nr);
            let val_expr = self.generate_expr_buf(val)?;
            // P222: when the RHS expression references the destination
            // variable (e.g. `s = s + s` lowers to OpAppendText(s, Var(s))
            // after the P217 self-append strip), `var_s += &*(&var_s)`
            // raises E0502 — mutable and immutable borrows of the same
            // place.  Hoist the RHS through a fresh `String` so the
            // self-borrow never overlaps the `+=` target.
            // @PLN25 slice (c): a nullable dest (`text?` local, an owned `String`) skips the
            // append when it holds the null sentinel — `s += x` on a null `s` stays null
            // (propagate). Gated on `Optional` so plain-text / `&mut String` work-buffer
            // appends (never null, and not always owned Strings) keep the bare emission.
            let dest_nullable = matches!(
                self.data.def(self.def_nr).variables().tp(*nr),
                Type::Optional(_)
            );
            if val.reads_var(*nr) {
                if dest_nullable {
                    write!(
                        w,
                        "{{ let __p222_tmp: String = (&*({val_expr})).to_string(); if {s_nr}.as_str() != loft::state::STRING_NULL {{ {s_nr} += &__p222_tmp; }} }}"
                    )?;
                } else {
                    write!(
                        w,
                        "{{ let __p222_tmp: String = (&*({val_expr})).to_string(); {s_nr} += &__p222_tmp; }}"
                    )?;
                }
                return Ok(());
            }
            if dest_nullable {
                write!(
                    w,
                    "{{ let __app_tmp: &str = &*({val_expr}); if {s_nr}.as_str() != loft::state::STRING_NULL {{ {s_nr} += __app_tmp; }} }}"
                )?;
            } else {
                write!(w, "{s_nr} += &*({val_expr})")?;
            }
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatText`/`OpFormatStackText` as a call to `ops::format_text`.
    pub(super) fn format_text(&mut self, w: &mut dyn Write, vals: &[Value]) -> std::io::Result<()> {
        if let [
            Value::Var(nr),
            val,
            width,
            Value::Int(dir),
            Value::Int(token),
        ] = vals
        {
            let s_nr = self.var_place(*nr);
            let val_expr = self.generate_expr_buf(val)?;
            // All text-returning calls produce either `Str` or `String` (never `&str`).
            // Wrap with `&*` so `format_text` (which expects `&str`) always gets the right type.
            // `&*Str` and `&*String` both deref to `&str` via their `Deref<Target=str>` impls.
            //
            // P247 — extends to `Value::Block` whose tail emits a
            // `String` (the nested-tuple text-read path emits
            // `var___ref_N.idx.clone()`).  The wrap forces the
            // temporary's lifetime to extend across the enclosing
            // statement; without it, rustc rejects the
            // String-returning Block with E0308 (`expected &str,
            // found String`).
            // `.base()` peels `Optional`: a `text?`-returning call produces an owned
            // `Str`/`String` (holding the null sentinel) exactly like a `text` one, so
            // it needs the same `&*` wrap. `format_text` renders the sentinel as
            // `null`, so `&*` is null-safe. (@PLN102 H4 surfaced this via `content()
            // -> text?`; the general #534 native text-branch E0308 class.)
            let val_str = if let Value::Call(d, _) = val.unspan()
                && matches!(self.data.def(*d).returned().base(), Type::Text(_))
            {
                format!("&*({val_expr})")
            } else if let Value::CallRef(v_nr, _) = val.unspan()
                && let Type::Function(_, ret, _) = self.data.def(self.def_nr).variables().tp(*v_nr)
                && matches!(ret.base(), Type::Text(_))
            {
                format!("&*({val_expr})")
            } else if let Value::Block(b) = val.unspan()
                && matches!(b.result.base(), Type::Text(_))
            {
                format!("&*({val_expr})")
            } else {
                val_expr
            };
            let width_expr = self.generate_expr_buf(width)?;
            write!(
                w,
                "ops::format_text(&mut {s_nr}, {val_str}, {width_expr}, {dir}, {token})"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatInt` as a call to
    /// `ops::format_long_with_tag` — the tag-aware wrapper that
    /// renders `null(<reason>)` when the preceding `OpTagFault`
    /// (4e.1 format-scope swap sibling) set a fault kind on
    /// `stores.format_fault_tag`.  Bare `null` rendering for
    /// genuine null values stays unchanged.
    pub(super) fn format_long(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [
            Value::Var(nr),
            val,
            Value::Int(radix),
            width,
            Value::Int(token),
            Value::Boolean(plus),
            Value::Boolean(note),
            Value::Int(dir),
        ] = vals
        {
            let s_nr = self.var_place(*nr);
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_long_with_tag({prefix}{s_nr}, {val_expr}, ops::take_format_fault(), {radix} as u8, {width_expr}, {token} as u8, {plus}, {note}, {dir} as i8)"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    pub(super) fn format_float(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [
            Value::Var(nr),
            val,
            width,
            prec,
            Value::Int(token),
            Value::Boolean(plus),
            dir,
        ] = vals
        {
            let s_nr = self.var_place(*nr);
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prec_expr = self.generate_expr_buf(prec)?;
            let dir_expr = self.generate_expr_buf(dir)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_float({prefix}{s_nr}, {val_expr}, {width_expr}, {prec_expr}, {token} as u8, {plus}, {dir_expr} as i8)"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }

    /// Use this to emit `OpFormatSingle`/`OpFormatStackSingle` as a call to `ops::format_single`.
    pub(super) fn format_single(
        &mut self,
        w: &mut dyn Write,
        vals: &[Value],
        stack: bool,
    ) -> std::io::Result<()> {
        if let [
            Value::Var(nr),
            val,
            width,
            prec,
            Value::Int(token),
            Value::Boolean(plus),
            dir,
        ] = vals
        {
            let s_nr = self.var_place(*nr);
            let val_expr = self.generate_expr_buf(val)?;
            let width_expr = self.generate_expr_buf(width)?;
            let prec_expr = self.generate_expr_buf(prec)?;
            let dir_expr = self.generate_expr_buf(dir)?;
            let prefix = if stack { "" } else { "&mut " };
            write!(
                w,
                "ops::format_single({prefix}{s_nr}, {val_expr}, {width_expr}, {prec_expr}, {token} as u8, {plus}, {dir_expr} as i8)"
            )?;
            return Ok(());
        }
        panic!("Could not parse {vals:?}");
    }
}
