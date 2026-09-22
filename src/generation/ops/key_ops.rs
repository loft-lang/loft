// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Plan 09 phase 04 — key-keyed Op emitters.
//!
//! Replaces the two arms in
//! `src/generation/dispatch.rs::output_call_inner`:
//!
//! - `"OpGetRecord" => …`  (~25 lines): lookup-by-key for typed
//!   stores.  Reads key types from the schema (`stores.types`) and
//!   emits one `Content::*` wrapper per key argument.
//! - `"OpIterate" => …`  (~45 lines): iterator setup with from/till
//!   key ranges.  Key types come from a `Value::Keys(keys)` payload
//!   carried in `vals[3]`.
//!
//! `OpGetRecord` / `OpIterate` share `emit_content_array` to write the
//! `&[Content::…]` tail.  Adding a third key-keyed Op (e.g. a future
//! `OpIterateRange`) reuses the same helper.
//!
//! The keyed-WRITE family — `OpReplaceKeyed` (whole-collection replace),
//! `OpSetKeyed` (`coll[key] = value` insert-or-replace, @P305) and
//! `OpClearKeyed` (clear a keyed-collection field, @P307) — are plain
//! pass-throughs to their runtime helpers (`Op…(cell, <args…>)`), migrated
//! here out of `dispatch.rs::output_call_inner` to keep that match under the
//! `dispatch_op_arm_budget` ratchet.  Like the original arms, they emit
//! NOTHING when the argument shape doesn't match (no `DefaultEmitter`
//! fallback), preserving byte-identical output.
//!
//! Phase 00 step 0.6 registry-first guard routes these Op names through
//! these emitters before reaching the legacy match in dispatch.rs;
//! phase 04 deletes the legacy arms.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

/// Emit `&[Content::…, …]` for a sequence of key argument values,
/// using `key_types[i]` (or `1` as fallback) for each.
fn emit_content_array(
    ctx: &mut EmitCtx<'_, '_>,
    vals: &[Value],
    key_types: &[i8],
) -> io::Result<()> {
    write!(ctx.w, "&[")?;
    for (i, v) in vals.iter().enumerate() {
        if i > 0 {
            write!(ctx.w, ", ")?;
        }
        let type_nr = key_types.get(i).copied().unwrap_or(1);
        ctx.output.emit_content(&mut *ctx.w, v, type_nr)?;
    }
    write!(ctx.w, "]")
}

/// `OpGetRecord` emitter — lookup by key in a typed-store collection.
///
/// `args`: `[data, db_tp, count, key1, key2, …]`
/// Emits: `OpGetRecord(cell, data, db_tp_i32, &[Content::…, …])`
pub struct OpGetRecordEmitter;

impl OpEmitter for OpGetRecordEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let (Some(Value::Int(db_tp)), Some(Value::Int(_count))) = (args.get(1), args.get(2)) else {
            // Shape didn't match the legacy `if let` guard — fall through
            // to default (matches pre-phase-04 behaviour where the
            // special case skipped malformed inputs silently).
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        if args.len() < 3 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }
        let db_tp = *db_tp;
        let row = ctx
            .output
            .stores
            .types
            .get(usize::try_from(db_tp).unwrap_or(0));
        let key_types: Vec<i8> = row
            .map(|t| t.keys.iter().map(|k| k.type_nr).collect())
            .unwrap_or_default();
        let key_vals = &args[3..];
        // @FR-R-TypedKeyed — the schema says this is a `hash` with ONE key of an integer
        // width `hash::find_long` lists, so the lookup takes the typed entry with the key
        // as a value.  The widths are the ones `emit_content` wraps as `Content::Long`
        // AND `fast_key` resolves; any other key, kind or arity keeps the general call.
        let typed = !ctx.output.typed_keyed_disabled
            && row.is_some_and(|t| matches!(t.parts, crate::database::Parts::Hash(..)))
            && key_vals.len() == 1
            && matches!(key_types.as_slice(), [k] if matches!(k.unsigned_abs(), 1 | 2 | 8 | 12));
        if typed {
            write!(ctx.w, "OpGetHashLong(cell,")?;
            ctx.emit(&args[0])?;
            write!(ctx.w, ", {db_tp}_i32, ")?;
            ctx.output.emit_long_key(&mut *ctx.w, &key_vals[0])?;
            return write!(ctx.w, ")");
        }
        write!(ctx.w, "OpGetRecord(cell,")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", {db_tp}_i32, ")?;
        emit_content_array(ctx, key_vals, &key_types)?;
        write!(ctx.w, ")")
    }
}

/// `OpIterate` emitter — set up an iterator over a typed-store
/// collection with from/till key ranges.
///
/// `args`: `[data, on, arg, Keys(keys), from_count, from_vals…, till_count, till_vals…]`
/// Emits: `OpIterate(cell, data, on, arg, &[Key{…}], &[Content::…], &[Content::…])`
pub struct OpIterateEmitter;

impl OpEmitter for OpIterateEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let Some(Value::Keys(keys)) = args.get(3) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        if args.len() < 4 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }
        let keys = keys.clone();
        let rest = &args[4..];
        let from_count = if let Some(Value::Int(n)) = rest.first() {
            usize::try_from(*n).unwrap_or(0)
        } else {
            0
        };
        let till_start = 1 + from_count;
        let till_count = if let Some(Value::Int(n)) = rest.get(till_start) {
            usize::try_from(*n).unwrap_or(0)
        } else {
            0
        };
        let from_vals = rest.get(1..till_start).unwrap_or(&[]);
        let till_vals = rest
            .get(till_start + 1..till_start + 1 + till_count)
            .unwrap_or(&[]);
        write!(ctx.w, "OpIterate(cell,")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[1])?;
        write!(ctx.w, ", ")?;
        ctx.emit_i32_slot(&args[2])?;
        // Keys array — emitted inline because `Key` is its own
        // struct shape (not a Content wrapper).
        write!(ctx.w, ", &[")?;
        for (i, k) in keys.iter().enumerate() {
            if i > 0 {
                write!(ctx.w, ", ")?;
            }
            write!(
                ctx.w,
                "Key {{ type_nr: {}, position: {}, start: {} }}",
                k.type_nr, k.position, k.start
            )?;
        }
        write!(ctx.w, "], ")?;
        // From + till content arrays — both keyed by `keys[i].type_nr`.
        let key_types: Vec<i8> = keys.iter().map(|k| k.type_nr).collect();
        emit_content_array(ctx, from_vals, &key_types)?;
        write!(ctx.w, ", ")?;
        emit_content_array(ctx, till_vals, &key_types)?;
        write!(ctx.w, ")")
    }
}

/// `OpReplaceKeyed` emitter — whole-collection replace of a keyed field.
///
/// `args`: `[src, dst, tp]` → `OpReplaceKeyed(cell, <src>, <dst>, <tp>_i32)`.
pub struct OpReplaceKeyedEmitter;

impl OpEmitter for OpReplaceKeyedEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [src, dst, tp_val] = args {
            write!(ctx.w, "OpReplaceKeyed(cell,")?;
            ctx.emit(src)?;
            write!(ctx.w, ", ")?;
            ctx.emit(dst)?;
            write!(ctx.w, ", ")?;
            ctx.emit_i32_slot(tp_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}

/// `OpSetKeyed` emitter — @P305 keyed insert-or-replace (`coll[key] = value`).
///
/// `args`: `[coll, value, tp]` → `OpSetKeyed(cell, <coll>, <value>, <tp>_i32)`.
pub struct OpSetKeyedEmitter;

impl OpEmitter for OpSetKeyedEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [coll, value, tp_val] = args {
            write!(ctx.w, "OpSetKeyed(cell,")?;
            ctx.emit(coll)?;
            write!(ctx.w, ", ")?;
            ctx.emit(value)?;
            write!(ctx.w, ", ")?;
            ctx.emit_i32_slot(tp_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}

/// `OpClearKeyed` emitter — @P307 clear a keyed-collection struct field
/// (`remove_claims` frees the contents + zeroes the field pointer).
///
/// `args`: `[dst, tp]` → `OpClearKeyed(cell, <dst>, <tp>_i32)`.
pub struct OpClearKeyedEmitter;

impl OpEmitter for OpClearKeyedEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [dst, tp_val] = args {
            write!(ctx.w, "OpClearKeyed(cell,")?;
            ctx.emit(dst)?;
            write!(ctx.w, ", ")?;
            ctx.emit_i32_slot(tp_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}
