// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Miscellaneous pass-through Op emitters, relocated VERBATIM from the
//! `output_call_inner` match (`src/generation/dispatch.rs`) to retire that
//! match.  Each reproduces its old arm byte-for-byte.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

/// `OpConvRefFromNull` — the null-reference literal.  (Native only; the
/// interpreter uses the op's own `#rust` body, which differs — a pre-existing
/// divergence left untouched by the match-elimination.)
pub struct OpConvRefFromNullEmitter;

impl OpEmitter for OpConvRefFromNullEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, _args: &[Value]) -> io::Result<()> {
        write!(ctx.w, "DbRef {{ store_nr: 0, rec: 0, pos: 0 }}")
    }
}

/// `OpGetTextSub` — `text[from..till]` → `&str` slice.  `args`: `[text, from, till]`.
pub struct OpGetTextSubEmitter;

impl OpEmitter for OpGetTextSubEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [text_val, from_val, till_val] = args {
            write!(ctx.w, "OpGetTextSub(")?;
            ctx.emit(text_val)?;
            write!(ctx.w, ", ")?;
            ctx.emit(from_val)?;
            write!(ctx.w, ", ")?;
            ctx.emit(till_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}

/// `OpDatabase` — modifies its `DbRef` argument in-place; emit as a
/// reassignment `<var> = OpDatabase(cell, <var>, <tp>_i32)`.  `args`: `[var, tp]`.
pub struct OpDatabaseEmitter;

impl OpEmitter for OpDatabaseEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [var_val, tp_val] = args {
            // @PLN157 § V-u (`@FR-R-RetAdopt`) — the adopted result local's witness store
            // is never allocated: the local builds in the return buffer instead.
            if let Value::Var(w) = var_val.unspan()
                && ctx.output.ret_adopt.is_some_and(|a| a.vdb == *w)
            {
                return write!(ctx.w, "()");
            }
            // @PLN157 § V-j (`@FR-R-MoveAppend`) — this var can be the `__vdb` a pair
            // PLACED a buffer record into, and OpDatabase's reuse arm clears the whole
            // store: the placement vanishes with the clear (no leak — the clear reclaims
            // it) while the buffer var would still hold the stale ref, so the place-once
            // guard must be re-armed HERE or the next call delivers into unclaimed bytes
            // (the c21 corruption: a host declared inside an enclosing loop).
            let hosted: Vec<String> = if let Value::Var(w) = var_val.unspan() {
                ctx.output
                    .move_pairs
                    .values()
                    .filter(|p| p.host_vdb == *w)
                    .map(|p| {
                        super::super::sanitize(
                            ctx.output
                                .data
                                .def(ctx.output.def_nr)
                                .variables()
                                .name(p.buf),
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            };
            if !hosted.is_empty() {
                write!(ctx.w, "{{ ")?;
            }
            // @PLN157 § V-y (`@FR-R-CompleteWrite`) — a site whose literal group covers
            // every field calls the no-prefill twin.
            let np = matches!(var_val.unspan(), Value::Var(w)
                if ctx.output.complete_writes.db_vars.contains(w));
            ctx.emit(var_val)?;
            if np {
                write!(ctx.w, " = OpDatabaseNP(cell,")?;
            } else {
                write!(ctx.w, " = OpDatabase(cell,")?;
            }
            ctx.emit(var_val)?;
            write!(ctx.w, ", ")?;
            ctx.emit_i32_slot(tp_val)?;
            write!(ctx.w, ")")?;
            if !hosted.is_empty() {
                write!(ctx.w, ";")?;
                for buf in &hosted {
                    write!(ctx.w, " var_{buf} = DbRef::NULL;")?;
                }
                write!(ctx.w, " }}")?;
            }
        }
        Ok(())
    }
}

/// `OpStep` — `OpStep(cell, &mut var_iter, data, on, arg)`.  `args`:
/// `[iter_var, data, on, arg]`.  A non-4-arg shape falls back to the template
/// path (mirrors the old arm's `if vals.len() == 4` guard).
pub struct OpStepEmitter;

impl OpEmitter for OpStepEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if args.len() != 4 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }
        write!(ctx.w, "OpStep(cell,&mut ")?;
        if let Value::Var(v) = &args[0] {
            let nm =
                super::super::sanitize(ctx.output.data.def(ctx.output.def_nr).variables().name(*v));
            write!(ctx.w, "var_{nm}")?;
        } else {
            ctx.emit(&args[0])?;
        }
        write!(ctx.w, ", ")?;
        ctx.emit(&args[1])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[2])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[3])?;
        write!(ctx.w, ")")
    }
}

/// `OpRemove` — `OpRemove(cell, &mut var_state, data, on, arg)`.  Same shape +
/// 4-arg guard as `OpStep`.
pub struct OpRemoveEmitter;

impl OpEmitter for OpRemoveEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if args.len() != 4 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }
        write!(ctx.w, "OpRemove(cell,&mut ")?;
        if let Value::Var(v) = &args[0] {
            let nm =
                super::super::sanitize(ctx.output.data.def(ctx.output.def_nr).variables().name(*v));
            write!(ctx.w, "var_{nm}")?;
        } else {
            ctx.emit(&args[0])?;
        }
        write!(ctx.w, ", ")?;
        ctx.emit(&args[1])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[2])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[3])?;
        write!(ctx.w, ")")
    }
}
