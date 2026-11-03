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
        let db_tp_lit = ctx
            .output
            .type_lit(u16::try_from(db_tp).unwrap_or(u16::MAX));
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
        let typed = !ctx.output.switches.typed_keyed_disabled
            && row.is_some_and(|t| matches!(t.parts, crate::database::Parts::Hash(..)))
            && key_vals.len() == 1
            && matches!(key_types.as_slice(), [k] if matches!(k.unsigned_abs(), 1 | 2 | 8 | 12));
        if typed {
            crate::rewrite_census::fired("R-TypedKeyed", 1);
            // An 8-byte integer key's position is a constant the call carries
            // (`OpGetHashLongAt`); the narrow widths keep the general typed entry.
            let wide = row
                .and_then(|t| t.keys.first())
                .filter(|k| matches!(k.type_nr.unsigned_abs(), 1 | 2));
            if let Some(k) = wide {
                write!(ctx.w, "OpGetHashLongAt(cell,")?;
                ctx.emit(&args[0])?;
                write!(ctx.w, ", {db_tp_lit}, {}_u32, ", k.position)?;
            } else {
                write!(ctx.w, "OpGetHashLong(cell,")?;
                ctx.emit(&args[0])?;
                write!(ctx.w, ", {db_tp_lit}, ")?;
            }
            ctx.output.emit_long_key(&mut *ctx.w, &key_vals[0])?;
            return write!(ctx.w, ")");
        }
        // The compound clause: two to four `integer` / `long` parts, handed over as values
        // (`hash::find_ints`).
        let typed_ints = !ctx.output.switches.typed_keyed_disabled
            && row.is_some_and(|t| matches!(t.parts, crate::database::Parts::Hash(..)))
            && (2..=4).contains(&key_vals.len())
            && key_types.len() == key_vals.len()
            && key_types.iter().all(|k| matches!(k.unsigned_abs(), 1 | 2));
        if typed_ints {
            crate::rewrite_census::fired("R-TypedKeyed/compound", 1);
            write!(ctx.w, "OpGetHashInts(cell,")?;
            ctx.emit(&args[0])?;
            write!(ctx.w, ", {db_tp_lit}, &[")?;
            for (i, v) in key_vals.iter().enumerate() {
                if i > 0 {
                    write!(ctx.w, ", ")?;
                }
                ctx.output.emit_long_key(&mut *ctx.w, v)?;
            }
            return write!(ctx.w, "])");
        }
        // The text clause: ONE `text` key, handed over as the text (`hash::find_str`).
        let typed_text = !ctx.output.switches.typed_keyed_disabled
            && row.is_some_and(|t| matches!(t.parts, crate::database::Parts::Hash(..)))
            && key_vals.len() == 1
            && matches!(key_types.as_slice(), [k] if k.unsigned_abs() == 6);
        if typed_text {
            crate::rewrite_census::fired("R-TypedKeyed/text", 1);
            write!(ctx.w, "OpGetHashText(cell,")?;
            ctx.emit(&args[0])?;
            write!(ctx.w, ", {db_tp_lit}, ")?;
            ctx.output.emit_str_key(&mut *ctx.w, &key_vals[0])?;
            return write!(ctx.w, ")");
        }
        write!(ctx.w, "OpGetRecord(cell,")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ", {db_tp_lit}, ")?;
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
        ctx.emit_param_slot(1, &args[1])?;
        write!(ctx.w, ", ")?;
        ctx.emit_iter_arg(&args[1], &args[2])?;
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
            ctx.emit_type_slot(tp_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}

/// `@FR-R-KeyedEntry` — the IR's find-or-insert (`opt::keyed_entry`):
/// `n_keyed_entry_ints(data, parent_tp, fld, k1, k2, k3, k4)` and
/// `n_keyed_entry_text(data, parent_tp, fld, key)`.  One integer part takes the inline walk
/// (`Stores::keyed_entry_long_at`, the field's offset and element type resolved here, where
/// they are constants); two to four take `keyed_entry_ints_at`; a text key `keyed_entry_text`.
pub struct KeyedEntryEmitter {
    pub text: bool,
}

impl OpEmitter for KeyedEntryEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let [data, Value::Int(ptp), Value::Int(fld), keys @ ..] = args else {
            return Err(io::Error::other(
                "a keyed entry without its constant type and field",
            ));
        };
        let (Ok(ptp), Ok(fld)) = (u16::try_from(*ptp), u16::try_from(*fld)) else {
            return Err(io::Error::other(
                "a keyed entry's type or field out of range",
            ));
        };
        let coll = ctx.output.expr_string(data)?;
        let tp_lit = ctx.output.type_lit_u16(ptp);
        let (off, etp) = ctx.output.stores.keyed_field_consts(ptp, fld);
        let key_types: Vec<i8> = ctx.output.stores.types[etp as usize]
            .keys
            .iter()
            .map(|k| k.type_nr)
            .collect();
        let n = key_types.len().clamp(1, keys.len());
        let mut parts = Vec::with_capacity(n);
        for k in &keys[..n] {
            parts.push(ctx.output.expr_string(k)?);
        }
        write!(ctx.w, "{{ let __ked: DbRef = {coll}; ")?;
        if self.text {
            write!(
                ctx.w,
                "let __kek = {}; stores.keyed_entry_text(&__ked, {tp_lit}, {fld}_u16, &*__kek) }}",
                parts[0]
            )?;
        } else if n == 1 {
            // An 8-byte integer key's position, for the inline walk; a narrow key takes none.
            let kp = ctx.output.stores.types[etp as usize]
                .keys
                .first()
                .filter(|k| matches!(k.type_nr.unsigned_abs(), 1 | 2))
                .map_or(u32::MAX, |k| u32::from(k.position));
            write!(
                ctx.w,
                "let __kek = ({}) as i64; stores.keyed_entry_long_at(&__ked, {tp_lit}, {fld}_u16, {off}_u32, {}, {kp}_u32, __kek) }}",
                parts[0],
                ctx.output.type_lit_u16(etp)
            )?;
        } else {
            let list: Vec<String> = parts.iter().map(|p| format!("({p}) as i64")).collect();
            write!(
                ctx.w,
                "let __kek = [{}]; stores.keyed_entry_ints_at(&__ked, {tp_lit}, {fld}_u16, &__kek) }}",
                list.join(", ")
            )?;
        }
        Ok(())
    }
}

/// `OpSetKeyed` emitter — @P305 keyed insert-or-replace (`coll[key] = value`).
///
/// `args`: `[coll, value, tp, count, key1, …]` →
/// `OpSetKeyed(cell, <coll>, <value>, <tp>_i32, &[Content::…])` — the subscript's keys
/// (loft#1716), wrapped as `OpGetRecord` wraps its own.
pub struct OpSetKeyedEmitter;

impl OpEmitter for OpSetKeyedEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let [coll, value, tp_val, Value::Int(_), keys @ ..] = args {
            let db_tp = if let Value::Int(t) = tp_val {
                *t & 0x7FFF
            } else {
                0
            };
            let key_types: Vec<i8> = ctx
                .output
                .stores
                .types
                .get(usize::try_from(db_tp).unwrap_or(0))
                .map(|t| t.keys.iter().map(|k| k.type_nr).collect())
                .unwrap_or_default();
            write!(ctx.w, "OpSetKeyed(cell,")?;
            ctx.emit(coll)?;
            write!(ctx.w, ", ")?;
            ctx.emit(value)?;
            write!(ctx.w, ", ")?;
            ctx.emit_type_slot(tp_val)?;
            write!(ctx.w, ", ")?;
            emit_content_array(ctx, keys, &key_types)?;
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
            ctx.emit_type_slot(tp_val)?;
            write!(ctx.w, ")")?;
        }
        Ok(())
    }
}
