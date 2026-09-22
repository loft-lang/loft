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
/// @PLN164 C5 (`@FR-O-ViewField`) — `OpGetField` on a local that holds a VALUE-RETURNED
/// record answers the tuple's own element: a VIEW-LEAF field is delivered as the reference
/// to the place it views, so `m.pts` is `var_m.2` where the record form read the field slot
/// out of a record in a store.
///
/// Every other `OpGetField` falls through to the `#rust` template unchanged — including a
/// SCALAR field of a value local, which never reaches here (the scalar getters are
/// `FusedElementReadEmitter`'s) and a collection field of a record that is still a record.
pub struct ViewFieldReadEmitter;

impl OpEmitter for ViewFieldReadEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [base, fld, ..] = args
            && let Value::Var(v) = base.unspan()
            && let Some(d) = ctx.output.value_record_locals.get(v).copied()
            && let Some(tp) = ctx.output.value_records.fns.get(&d).copied()
            && let Value::Int(off) = fld.unspan()
            && ctx
                .output
                .value_records
                .view_offs
                .get(&d)
                .is_some_and(|offs| offs.contains(&i64::from(*off)))
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
        super::default::DefaultEmitter.emit(ctx, args)
    }
}

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
            if emit_join_read(ctx, args)? {
                return Ok(());
            }
            return emit_hoisted_scalar_or_default(ctx, args);
        };
        let Some(header) = ctx.output.active_vec_header(&fused.path) else {
            return emit_hoisted_scalar_or_default(ctx, args);
        };
        let (header, ty, absent) = (header.to_string(), fused.rust_type, fused.absent);
        let verify = verify(ctx);
        // @PLN157 § V-ak (`@FR-R-Base`) — a growth-free loop holds the element base too;
        // the read through it is `unsafe` at the call, which is where the proof lives.
        let base = ctx.output.active_vec_base(&fused.path).map(str::to_owned);
        // `@FR-R-BoundedNest` step 2 — inside the raw arm of an admitted nest the guard has
        // proved this index in range and the element not null, so the read is one load
        // through the held base.  Under `LOFT_HOIST_VERIFY=1` the checked read is emitted
        // beside it and the two compared at the read.
        if ctx.output.nest_raw_arm
            && let Some(base) = &base
        {
            let raw_verify = ctx.output.hoist_verify;
            if raw_verify {
                write!(ctx.w, "{{ let _raw: {ty} = ")?;
            }
            write!(ctx.w, "unsafe {{ {base}.add(((")?;
            ctx.emit(fused.index)?;
            write!(ctx.w, ") as usize) * ((")?;
            ctx.emit(fused.size)?;
            write!(ctx.w, ") as usize) + ((")?;
            ctx.emit(fused.fld)?;
            write!(ctx.w, ") as usize)).cast::<{ty}>().read_unaligned() }}")?;
            if raw_verify {
                write!(ctx.w, "; let _chk: {ty} = ")?;
                ctx.output.nest_raw_arm = false;
                let r = self.emit(ctx, args);
                ctx.output.nest_raw_arm = true;
                r?;
                write!(
                    ctx.w,
                    "; assert!(_raw == _chk, \"bounded nest: a raw read disagrees with the checked read — the guard admitted an index out of range or a null element\"); _raw }}"
                )?;
            }
            return Ok(());
        }
        if let Some(base) = &base {
            write!(
                ctx.w,
                "unsafe {{ vector::get_elem_at::<{ty}, {verify}>(&{header}, {base}, &("
            )?;
        } else {
            write!(
                ctx.w,
                "vector::get_elem_hoisted::<{ty}, {verify}>(&{header}, &("
            )?;
        }
        ctx.emit(fused.vector)?;
        write!(ctx.w, "), (")?;
        ctx.emit(fused.size)?;
        write!(ctx.w, ") as u32, ")?;
        ctx.emit(fused.index)?;
        write!(ctx.w, ", (")?;
        ctx.emit(fused.fld)?;
        write!(ctx.w, ") as u32, {absent}, &stores.allocations)")?;
        if base.is_some() {
            write!(ctx.w, " }}")?;
        }
        Ok(())
    }
}

/// `@FR-R-Base`'s join clause — `v[i]?.f` where the loop holds `v`'s header and element
/// base: one range test and one load through the base, with the join the `?` lowers to run
/// only for an index that test refuses.  Answers whether it emitted.
///
/// In range the join's temp is still assigned — the very `DbRef` the join would have given
/// it — so the rewrite drops no effect of the join and owes no proof that the temp is
/// unread.  Off the fast path the JOIN BLOCK is emitted whole, as it stood, and its result
/// read by the runtime's one general typed read (`vector::field_of`): a negative index
/// addresses from the end there, and an absent element answers its default RECORD's field,
/// which a declared field default can make non-zero — so that arm is never a constant.
/// The join is bound to a local before the read for the templates' own reason: it takes
/// `stores` mutably, and the read borrows it.
fn emit_join_read(ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<bool> {
    let Some(join) = ctx.output.fused_join_read(ctx.def_fn.name(), args) else {
        return Ok(false);
    };
    let (Some(header), Some(base)) = (
        ctx.output.active_vec_header(&join.path).map(str::to_owned),
        ctx.output.active_vec_base(&join.path).map(str::to_owned),
    ) else {
        return Ok(false);
    };
    let (ty, absent) = (join.rust_type, join.absent);
    let verify = verify(ctx);
    let vars = ctx.output.data.def(ctx.output.def_nr).variables();
    let temp = super::super::sanitize(vars.name(join.temp));
    let index = super::super::sanitize(vars.name(join.index));
    write!(
        ctx.w,
        "{{ match unsafe {{ vector::elem_field_at::<{ty}, {verify}>(&{header}, {base}, &("
    )?;
    ctx.emit(join.vector)?;
    write!(ctx.w, "), (")?;
    ctx.emit(join.size)?;
    write!(ctx.w, ") as u32, var_{index}, (")?;
    ctx.emit(join.fld)?;
    write!(
        ctx.w,
        ") as u32, &stores.allocations) }} {{ Some((__je, __jv)) => {{ var_{temp} = __je; __jv }} None => {{ let __jr: DbRef = "
    )?;
    ctx.emit(join.join)?;
    write!(ctx.w, "; vector::field_of::<{ty}>(&__jr, (")?;
    ctx.emit(join.fld)?;
    write!(
        ctx.w,
        ") as u32, {absent}, &stores.allocations) }} }} }} /*@FR-R-Base join read*/"
    )?;
    Ok(true)
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
        // `@FR-R-RecPtr` — a field read off a record VIEW whose address the block holds is
        // one load through it; `unsafe` at the call, where the block's proof lives.
        if let Some((v, fld)) = crate::generation::hoist::scalar_read(ctx.def_fn.name(), args)
            && let Some(expr) = ctx.output.rec_ptr_read(v, fld, ctx.def_fn.name())
        {
            return write!(ctx.w, "{expr}");
        }
        // …and so is a field reached through INLINE sub-records (`v.pos.x`): the same
        // address, at the summed offset (`hoist::view_field`, the path clause).
        if crate::generation::hoist::nested_field_enabled()
            && let [base, fld, ..] = args
            && !matches!(base.unspan(), Value::Var(_))
            && let Some((v, off)) = crate::generation::hoist::view_field(ctx.output.data, base, fld)
            && let Some(expr) = ctx.output.rec_ptr_read(v, off, ctx.def_fn.name())
        {
            // The checking form walks the path the unrewritten way and compares: `rec_get`
            // re-reads at the offset it is given, so only this can see one summed wrongly.
            if ctx.output.hoist_verify {
                write!(ctx.w, "vector::path_read_verify({expr}, ")?;
                super::default::DefaultEmitter.emit(ctx, args)?;
                return write!(ctx.w, ")");
            }
            return write!(ctx.w, "{expr}");
        }
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
            // `@FR-R-SplitTable` — the length of a split bound to a name is its table's.
            if let Some(Value::Var(x)) = args.first().map(Value::unspan)
                && let Some(t) = ctx.output.split_table_var(*x)
            {
                return write!(ctx.w, "(__st_{t}.len() as i64)");
            }
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        write!(ctx.w, "(i64::from({header}.len))")
    }
}

/// `OpGetText` — the text of an element a loop over a lazy split reads is the iterator's
/// next piece (`@FR-R-LazySplit`): `OpGetText(OpGetVectorNullable(vec, …), 0)` with `vec`
/// a vector the function's lazy loops replaced.  Running out of pieces raises the loop's
/// `__ls_done_N`, which its length test reads, and answers the null text the vector form
/// reads past its last element — the loop leaves before anything looks at it.
///
/// `OpGetText(OpGetVectorNullable(v, …, i), 0)` with `v` a split TABLE or a walk's alias
/// of one (`@FR-R-SplitTable`) is the slice at `i` — `codegen_runtime::split_table_get`,
/// which answers what the element read answers: a negative index from the end, a null or
/// out-of-range one the null text.  The raising twin `OpGetVector` takes
/// `split_table_get_or_raise`, which raises what the op raises outside the table.
///
/// Every other `OpGetText` emits the `#rust` template unchanged.
pub struct LazySplitNextEmitter;

impl OpEmitter for LazySplitNextEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let Some(elem) = args.first()
            && let Some(vec) = ctx.output.lazy_split_reader(elem, "OpGetVectorNullable")
        {
            return write!(
                ctx.w,
                "(match __ls_{vec}.next() {{ Some(__piece) => __piece, None => {{ __ls_done_{vec} = true; loft::state::STRING_NULL }} }})"
            );
        }
        // `@FR-R-Base`'s text clause — the element's record number through the held base,
        // the text sliced off the vector's own store; out of range takes the unfused path.
        if let Some((header, base, span, vector, index)) = ctx.output.fused_text_read(args) {
            let verify = verify(ctx);
            write!(
                ctx.w,
                "unsafe {{ vector::text_elem_at::<{verify}>(&{header}, {base}, {span}, &("
            )?;
            ctx.emit(vector)?;
            write!(ctx.w, "), (")?;
            ctx.emit(index)?;
            return write!(ctx.w, ") as i64, &stores.allocations) }}");
        }
        if let Some(elem) = args.first()
            && let Some((t, raising)) = ctx.output.split_table_read(elem)
            && let Value::Call(_, inner) = elem.unspan()
            && let Some(index) = inner.get(2)
        {
            // The raising read binds its index first, as the op's template does: the
            // index may still borrow `stores`, which the getter takes mutably.
            if raising {
                write!(ctx.w, "{{ let __vi = (")?;
                ctx.emit(index)?;
                return write!(
                    ctx.w,
                    ") as i64; loft::codegen_runtime::split_table_get_or_raise(stores, &__st_{t}, __vi) }}"
                );
            }
            write!(ctx.w, "loft::codegen_runtime::split_table_get(&__st_{t}, (")?;
            ctx.emit(index)?;
            return write!(ctx.w, ") as i64)");
        }
        super::default::DefaultEmitter.emit(ctx, args)
    }
}

/// Emits `@FR-R-Header` (the fused element write) under `@FR-R-InPlace`.
pub struct FusedElementWriteEmitter;

impl OpEmitter for FusedElementWriteEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        // `@FR-R-RecPtr` — an in-place field write of a record VIEW whose address the block
        // holds is one store through it (the setter's `rec != 0` test is the null address).
        // The field is the view's own, or one reached through INLINE sub-records
        // (`v.pos.x = …`) at the summed offset (`hoist::view_field`, the path clause); the
        // `DbRef` handed to the checking form is the VIEW's, which the offset counts from.
        if let [base, fld, val] = args
            && (matches!(base.unspan(), Value::Var(_))
                || crate::generation::hoist::nested_field_enabled())
            && let Some((v, off)) = crate::generation::hoist::view_field(ctx.output.data, base, fld)
            && let Some(ty) = crate::generation::hoist::setter_kind(ctx.def_fn.name())
            && let Some(ptr) = ctx.output.active_rec_ptr(v).map(str::to_owned)
        {
            let verify = ctx.output.hoist_verify;
            write!(ctx.w, "{{ let __wv = (")?;
            ctx.emit(val)?;
            write!(ctx.w, "); unsafe {{ vector::rec_set::<{ty}>({ptr}, &(")?;
            ctx.emit(&Value::Var(v))?;
            return write!(
                ctx.w,
                "), ({off}_i64) as u32, __wv, &stores.allocations, {verify}) }} }}"
            );
        }
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
        // @PLN157 § V-ak (`@FR-R-Base`) — the write through the held base, when there is
        // one; `unsafe` at the call, which is where the growth-free proof lives.
        let base = ctx.output.active_vec_base(&fused.path).map(str::to_owned);
        if let Some(base) = &base {
            write!(
                ctx.w,
                "); unsafe {{ stores.vec_set_at::<{ty}, {verify}>(&{header}, {base}, &("
            )?;
        } else {
            write!(
                ctx.w,
                "); stores.vec_set_hoisted_or_raise_runtime::<{ty}, {verify}>(&{header}, &("
            )?;
        }
        ctx.emit(fused.vector)?;
        write!(ctx.w, "), (")?;
        ctx.emit(fused.size)?;
        write!(ctx.w, ") as u32, __wi, (")?;
        ctx.emit(fused.fld)?;
        write!(ctx.w, ") as u32, __wv)")?;
        if base.is_some() {
            write!(ctx.w, " }}")?;
        }
        write!(ctx.w, " }}")
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
        // `@FR-R-PushFill`'s window clause — the loop being emitted opened a push window
        // for this path: the push is a comparison, one store through the held base and a
        // bump of the window's length.
        let window = ctx
            .output
            .active_push_window(&fused.path)
            .map(str::to_owned);
        write!(ctx.w, "{{ let __pv = (")?;
        ctx.emit(fused.val)?;
        if let Some(win) = window {
            write!(
                ctx.w,
                "); unsafe {{ stores.push_windowed::<{ty}, {verify}>(&mut {header}, &mut {win}, &("
            )?;
            ctx.emit(fused.vector)?;
            return write!(ctx.w, "), {size}, __pv) }} }}");
        }
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
        let Some((target, header)) = (!out.record_push_disabled)
            .then(|| {
                crate::generation::hoist::mint_target(
                    out.data,
                    out.stores,
                    "OpNewRecord",
                    args,
                    vars,
                )
            })
            .flatten()
            .and_then(|t| {
                let header = out.active_mint_push(&t.path)?.to_owned();
                Some((t, header))
            })
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
        let elem = out.stores.content(target.vector_tp);
        let size = out.stores.size(elem);
        let verify = verify(ctx);
        // `@FR-R-PushRec` heap clause — a heap-owning element's slot is zeroed at the mint;
        // so is a struct-enum's, whose narrower variants leave a wider one's tail unwritten.
        let zero = if out.stores.owns_heap(elem) || !out.stores.is_struct(elem) {
            "_zero"
        } else {
            ""
        };
        // `@FR-R-PushFill`'s record clause — the loop being emitted opened a window for
        // this path: the slot is the window's next, addressed off the held base.
        if let Some(win) = out.active_push_window(&target.path).map(str::to_owned) {
            let zero = if zero.is_empty() { "false" } else { "true" };
            write!(
                ctx.w,
                "unsafe {{ stores.push_record_windowed::<{zero}, {verify}>(&mut {header}, &mut {win}, &("
            )?;
            ctx.emit(&target.vector)?;
            return write!(ctx.w, "), {size}) }}");
        }
        write!(
            ctx.w,
            "stores.push_record_hoisted{zero}::<{verify}>(&mut {header}, &("
        )?;
        ctx.emit(&target.vector)?;
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
        let Some((target, header)) = (!out.record_push_disabled)
            .then(|| {
                crate::generation::hoist::mint_target(
                    out.data,
                    out.stores,
                    "OpFinishRecord",
                    args,
                    vars,
                )
            })
            .flatten()
            .and_then(|t| {
                let header = out.active_mint_push(&t.path)?.to_owned();
                Some((t, header))
            })
        else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        let verify = verify(ctx);
        // `@FR-R-PushFill`'s record clause — through a window the finish is the window's
        // length bump; the record's own length is written when the window closes.
        if let Some(win) = out.active_push_window(&target.path).map(str::to_owned) {
            return write!(ctx.w, "{win}.len += 1 /*@FR-R-PushFill windowed finish*/");
        }
        write!(
            ctx.w,
            "stores.push_record_finish::<{verify}>(&mut {header}, &("
        )?;
        ctx.emit(&target.vector)?;
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
