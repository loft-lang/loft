// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
// loft#885 — indexed reads through a loop-invariant vector header

//! Indexed reads emitted against a header the enclosing loop already derived, once
//! [`crate::generation::hoist`] proved the loop writes no store.
//!
//! Two element-address ops (`v[i]` as an address, raising and nullable) plus the scalar
//! getters, which fuse the address and the load into one call rather than building a `DbRef`
//! between them. Every emitter falls back to the `#rust` template the moment there is no
//! header for the vector being read, which is every read outside such a loop — so the shape
//! that changes is exactly the one the analysis vouched for.
//!
//! The runtime helpers keep the fast path to an in-range index and route everything else
//! (negative, out of range, `i64::MIN`, a null or empty vector) back into `get_vector` /
//! `vec_get_or_raise_runtime`. That is deliberate: the answers those cases give — and the
//! `IndexOutOfBounds` / `NegativeIndex` raise the non-nullable form owes — keep one
//! definition rather than two that can drift.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

/// The `&(vector)` operand plus the header local, or `None` when this read is not covered.
fn header_for<'a>(ctx: &'a EmitCtx<'_, '_>, arg: &Value) -> Option<&'a str> {
    let path = crate::generation::hoist::vector_path(ctx.output.data, arg)?;
    ctx.output.active_vec_header(&path)
}

/// `LOFT_HOIST_VERIFY=1` picks the checking monomorphisation.
fn verify(ctx: &EmitCtx<'_, '_>) -> &'static str {
    if ctx.output.hoist_verify {
        "true"
    } else {
        "false"
    }
}

/// `OpGetInt` / `OpGetSingle` / `OpGetFloat` — a scalar read of `v[i]` inside a loop that
/// hoisted `v`'s header, emitted as ONE load: the bounds test, then the value.
///
/// Everything the pair used to do between those two — build the element `DbRef`, test its
/// `rec` against the null element, resolve the store from it again, and re-check
/// `rec != 0 && valid(..)` inside the getter — is decided by the bounds test already.
/// Worth ~3.2× on top of the header hoist alone — more than the hoist itself, because the
/// second store resolution it removes costs more than the arithmetic it saves
/// (loft#885 stage 2; PERFORMANCE.md § what the fusion is worth). `LOFT_NO_ELEM_FUSE=1`
/// emits the unfused form, which is the middle rung of that measurement.
///
/// Anything else (no header, an expression instead of a variable for the vector, a getter
/// with a different shape) emits the `#rust` template unchanged.
/// Emits `@FR-R-Header` (the fused element read) and falls back to `@FR-R-Scalar`.
pub struct FusedElementReadEmitter;

impl OpEmitter for FusedElementReadEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // @PLN157 § V-aa (`@FR-R-ValueRecord`) — a field read off a local that holds a
        // VALUE-RETURNED record is a tuple index: there is no record in a store to read.
        // Checked here rather than from a second registration, because this emitter owns
        // every scalar getter and a later insert would silently replace it.
        if let [base, fld, ..] = args
            && let Value::Var(v) = base.unspan()
            && let Some(d) = ctx.output.value_record_locals.get(v).copied()
            && let Some(tp) = ctx.output.value_records.fns.get(&d).copied()
            && let Value::Int(off) = fld.unspan()
            && let Some(idx) = ctx
                .output
                .value_records
                .index
                .get(&(tp, i64::from(*off)))
                .copied()
        {
            let name =
                super::super::sanitize(ctx.output.data.def(ctx.output.def_nr).variables().name(*v));
            return write!(ctx.w, "var_{name}.{idx}");
        }

        let Some(fused) = ctx.output.fused_element_read(ctx.def_fn.name(), args) else {
            return emit_hoisted_scalar_or_default(ctx, args);
        };
        let Some(header) = ctx.output.active_vec_header(&fused.path) else {
            return emit_hoisted_scalar_or_default(ctx, args);
        };
        let (header, ty, absent) = (header.to_string(), fused.rust_type, fused.absent);
        let verify = verify(ctx);
        write!(
            ctx.w,
            "vector::get_elem_hoisted::<{ty}, {verify}>(&{header}, &("
        )?;
        ctx.emit(fused.vector)?;
        write!(ctx.w, "), (")?;
        ctx.emit(fused.size)?;
        write!(ctx.w, ") as u32, ")?;
        ctx.emit(fused.index)?;
        write!(ctx.w, ", (")?;
        ctx.emit(fused.fld)?;
        write!(ctx.w, ") as u32, {absent}, &stores.allocations)")
    }
}

/// A record scalar read (`lay.x0`, any getter in [`crate::generation::hoist::SCALAR_GETTERS`])
/// inside a loop that hoisted it (@PLN157 P4c): the local the prelude bound stands for the
/// whole getter — its store resolution, its `rec == 0` test and its load.  Under
/// `LOFT_HOIST_VERIFY=1` the getter is ALSO emitted and the two are compared, so a scalar
/// the loop can still change under its hoist panics at the read instead of answering a
/// stale value.  Everything else — no hoist, a read the collector did not admit — emits the
/// `#rust` template unchanged.
/// Emits `@FR-R-Scalar`: the hoisted local, or its checking form under `@FR-R-Switch`.
fn emit_hoisted_scalar_or_default(ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
    let Some(local) = ctx
        .output
        .hoisted_scalar_read(ctx.def_fn.name(), args)
        .map(str::to_owned)
    else {
        return super::default::DefaultEmitter.emit(ctx, args);
    };
    if ctx.output.hoist_verify {
        write!(ctx.w, "vector::hoisted_scalar_verify({local}, ")?;
        super::default::DefaultEmitter.emit(ctx, args)?;
        return write!(ctx.w, ")");
    }
    write!(ctx.w, "{local}")
}

/// `OpSetInt` / `OpSetSingle` / `OpSetFloat` — a scalar write of `v[i]` inside a loop
/// that hoisted `v`'s header (@PLN157 P4b), emitted as ONE store: the bounds test,
/// then the value lands.
///
/// The unfused pair resolved the vector from scratch (`vec_get_or_raise_runtime`:
/// store lookup, container-slot load, length load, element `DbRef`), then resolved the
/// store AGAIN through that `DbRef` and re-tested `rec != 0` inside the typed setter —
/// per element, per write.  The bounds test against the hoisted length decides all of
/// it.  Off the fast path the runtime falls back to `vec_get_or_raise_runtime` plus the
/// template's own `rec != 0` write, so the raise and the null-element behaviour keep
/// one definition.
///
/// The index and value are bound to locals before the call for the template's own
/// reason (@P321d / @P338): the helper takes `&mut stores`, and either expression may
/// still be evaluating its own `stores` borrow when that one is taken (E0499).
///
/// Anything else — no header, a setter this table does not fuse, a field write on a
/// record — emits the `#rust` template unchanged.
/// `len(v)` inside a loop that hoisted `v`'s header reads the header's length instead of
/// resolving the vector through the store table again (@PLN157 queue item 1).  The same
/// proof that made the header loop-invariant makes its length so: the loop writes no
/// store, or only in place, so nothing inside it can change how many elements `v` has.
/// Outside such a loop — and for a vector the analysis did not cover — the `#rust`
/// template's runtime read stands, which is `DefaultEmitter`.
/// Emits `@FR-R-Header` for `len(P)`.
pub struct HoistedLengthEmitter;

impl OpEmitter for HoistedLengthEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let header = args
            .first()
            .and_then(|v| header_for(ctx, v))
            .map(str::to_string);
        let Some(header) = header else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        write!(ctx.w, "(i64::from({header}.len))")
    }
}

/// Emits `@FR-R-Header` (the fused element write) under `@FR-R-InPlace`.
pub struct FusedElementWriteEmitter;

impl OpEmitter for FusedElementWriteEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let Some(fused) = ctx.output.fused_element_write(ctx.def_fn.name(), args) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let Some(header) = ctx.output.active_vec_header(&fused.path) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let (header, ty) = (header.to_string(), fused.rust_type);
        let verify = verify(ctx);
        write!(ctx.w, "{{ let __wi = (")?;
        ctx.emit(fused.index)?;
        write!(ctx.w, "); let __wv = (")?;
        ctx.emit(fused.val)?;
        write!(
            ctx.w,
            "); stores.vec_set_hoisted_or_raise_runtime::<{ty}, {verify}>(&{header}, &("
        )?;
        ctx.emit(fused.vector)?;
        write!(ctx.w, "), (")?;
        ctx.emit(fused.size)?;
        write!(ctx.w, ") as u32, __wi, (")?;
        ctx.emit(fused.fld)?;
        write!(ctx.w, ") as u32, __wv) }}")
    }
}

/// `OpGetVectorNullable` — `v[i]` where an out-of-range index answers the null element
/// (for-loop iteration depends on that null as its end signal).  `args`:
/// `[vector, elem_size, index]`.
pub struct OpGetVectorNullableEmitter;

impl OpEmitter for OpGetVectorNullableEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let (Some(header), [vec_val, size_val, index_val]) =
            (args.first().and_then(|a| header_for(ctx, a)), args)
        else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let header = header.to_string();
        let verify = verify(ctx);
        write!(
            ctx.w,
            "vector::get_vector_hoisted::<{verify}>(&{header}, &("
        )?;
        ctx.emit(vec_val)?;
        write!(ctx.w, "), (")?;
        ctx.emit(size_val)?;
        write!(ctx.w, ") as u32, ")?;
        ctx.emit(index_val)?;
        write!(ctx.w, ", &stores.allocations)")
    }
}

/// `OpGetVector` — user-facing `v[i]`, which RAISES on an out-of-range or negative index.
/// `args`: `[vector, elem_size, index]`.
///
/// The receiver and the index are bound to locals before the call for the same reason the
/// template does it (@P321d / @P338): the fallback takes `&mut stores`, and a nested index
/// or a checked index expression would otherwise still be evaluating its own borrow when
/// that one is taken (E0499).
pub struct OpGetVectorEmitter;

impl OpEmitter for OpGetVectorEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let (Some(header), [vec_val, size_val, index_val]) =
            (args.first().and_then(|a| header_for(ctx, a)), args)
        else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let header = header.to_string();
        let verify = verify(ctx);
        write!(ctx.w, "{{let __vr = ")?;
        ctx.emit(vec_val)?;
        write!(ctx.w, "; let __vi = ")?;
        ctx.emit(index_val)?;
        write!(
            ctx.w,
            "; stores.vec_get_hoisted_or_raise_runtime::<{verify}>(&{header}, &__vr, ("
        )?;
        ctx.emit(size_val)?;
        write!(ctx.w, ") as u32, __vi)}}")
    }
}

/// `OpPushInt` / `OpPushSingle` / `OpPushFloat` — `v += [x]` inside a loop that hoisted a
/// PUSH header for `v` (@PLN157 § V-q, `@FR-R-Push`), emitted as ONE call that tests the
/// capacity, stores the element and bumps the length, re-entering the runtime's append only
/// at a growth step.  The value is bound to a local before the call for the template's own
/// reason (@P321d / @P338): the helper takes `&mut stores`, and the value may still be
/// evaluating its own `stores` borrow when that one is taken.
///
/// Anything else — no push header, a kind this table does not fuse — emits the `#rust`
/// template unchanged.
pub struct HoistedPushEmitter;

impl OpEmitter for HoistedPushEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let Some((fused, header)) = ctx.output.fused_push(ctx.def_fn.name(), args) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let (ty, size) = (fused.rust_type, fused.size);
        let verify = verify(ctx);
        write!(ctx.w, "{{ let __pv = (")?;
        ctx.emit(fused.val)?;
        write!(
            ctx.w,
            "); stores.push_hoisted::<{ty}, {verify}>(&mut {header}, &("
        )?;
        ctx.emit(fused.vector)?;
        write!(ctx.w, "), {size}, __pv) }}")
    }
}

/// `OpNewRecord` — inside a loop that binds a RECORD-push header for the target path
/// (@PLN157 § V-t, `@FR-R-PushRec`), the fresh element is the header's next slot: no
/// `record_new` dispatch and no default prefill, because the group that follows writes
/// every field explicitly (the IR's literal lowering emits omitted fields' defaults and
/// sentinels itself, and a declined delivery lands as a whole-record `OpCopyRecord`).
/// Everywhere else — no header, a keyed container, a heap-owning element (the hoist
/// declined those paths a header) — the `#rust` template stands.
pub struct NewRecordEmitter;

impl OpEmitter for NewRecordEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let out = &ctx.output;
        let vars = out.data.def(out.def_nr).variables();
        let Some(header) = (!out.record_push_disabled)
            .then(|| crate::generation::hoist::mint_path(out.data, "OpNewRecord", args, vars))
            .flatten()
            .and_then(|path| out.active_mint_push(&path))
            .map(str::to_owned)
        else {
            // @PLN157 § V-y (`@FR-R-CompleteWrite`) — an UNFUSED mint whose every group
            // in this function covers the element type calls the no-prefill twin; the
            // call shape mirrors the `#rust` template exactly.
            if let Some(Value::Int(ptp)) = args.get(1).map(Value::unspan)
                && u16::try_from(*ptp)
                    .is_ok_and(|t| ctx.output.complete_writes.mint_tps.contains(&t))
                && args.len() == 3
            {
                write!(ctx.w, "OpNewRecordNP(cell,")?;
                ctx.emit(&args[0])?;
                write!(ctx.w, ", ")?;
                ctx.emit_i32_slot(&args[1])?;
                write!(ctx.w, ", ")?;
                ctx.emit_i32_slot(&args[2])?;
                return write!(ctx.w, ")");
            }
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let Some(Value::Int(tp)) = args.get(1).map(Value::unspan) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let Ok(tp) = u16::try_from(*tp) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let size = out.stores.size(out.stores.content(tp));
        let verify = verify(ctx);
        write!(
            ctx.w,
            "stores.push_record_hoisted::<{verify}>(&mut {header}, &("
        )?;
        ctx.emit(&args[0])?;
        write!(ctx.w, "), {size})")
    }
}

/// `OpFinishRecord` — the finish half of [`NewRecordEmitter`]'s fused form: the length
/// bump, written to the header and the record, exactly where `record_finish`'s
/// `vector_finish` bumped.  Same gate; everywhere else the `#rust` template stands.
pub struct FinishRecordEmitter;

impl OpEmitter for FinishRecordEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let out = &ctx.output;
        let vars = out.data.def(out.def_nr).variables();
        let Some(header) = (!out.record_push_disabled)
            .then(|| crate::generation::hoist::mint_path(out.data, "OpFinishRecord", args, vars))
            .flatten()
            .and_then(|path| out.active_mint_push(&path))
            .map(str::to_owned)
        else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let verify = verify(ctx);
        write!(
            ctx.w,
            "stores.push_record_finish::<{verify}>(&mut {header}, &("
        )?;
        ctx.emit(&args[0])?;
        write!(ctx.w, "))")
    }
}

/// `OpPreAllocVector` — the reservation the parser emits before a push to a local vector.
/// Inside a loop that holds a push header for the path it is emitted as NOTHING (@PLN157
/// § V-q): the push grows the vector on demand, and the reservation would cost a store
/// resolution per iteration for a record that is either already there or about to be
/// claimed by the push's own growth step.  Everywhere else the `#rust` template stands.
pub struct PreAllocEmitter;

impl OpEmitter for PreAllocEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let held = crate::generation::hoist::pre_alloc_path(
            ctx.output.data,
            ctx.def_fn.name(),
            args,
        )
        .is_some_and(|path| {
            (!ctx.output.push_hoist_disabled && ctx.output.active_push_header(&path).is_some())
                    // @PLN157 § V-t — a record-push header's growth step reserves for
                    // itself, exactly as the scalar push's does.
                    || ctx.output.active_mint_push(&path).is_some()
        });
        if held {
            return write!(ctx.w, "()");
        }
        super::default::DefaultEmitter.emit(ctx, args)
    }
}
