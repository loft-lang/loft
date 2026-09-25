// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Coroutine Op emitters (N8b — native stackful generators).
//!
//! Migrated out of `dispatch.rs::output_call_inner` to keep that match under
//! the `dispatch_op_arm_budget` ratchet.  Both are byte-identical
//! pass-throughs to the `loft::codegen_runtime::coroutine_*` runtime helpers;
//! like the original arms they emit NOTHING when no generator argument is
//! present (no `DefaultEmitter` fallback).

use super::{EmitCtx, OpEmitter};
use crate::coroutine_layout::{YieldSlot, slot_count};
use crate::data::Value;
use std::io;

/// `OpCoroutineNext` emitter — advance a native coroutine and read its yield.
///
/// `args`: `[gen, value_size?]`.  `value_size` packs two fields:
/// - low byte  = byte size of the yielded value
/// - high byte = channel tag:
///   - 0 = legacy per-type channel — dispatch on byte size:
///     8 → `i64` / 1 → `bool` / 12 → `DbRef` (@P326) /
///     16 → text / else → `i32`
///   - 1 = unified `next_into(stores, &mut [i64; N])` channel (@P327 native)
///     — allocate a stack `[i64; N]` buffer, call `coroutine_next_into`,
///     and rebuild the consumer's tuple from buffer slots
///   - `CHANNEL_NONE` = no native transport for this yield type; emit a diverging
///     expression so the producer's `compile_error!` is the only diagnostic
pub struct OpCoroutineNextEmitter;

impl OpEmitter for OpCoroutineNextEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let Some(gen_val) = args.first() {
            let gen_code = ctx.output.generate_expr_buf(gen_val)?;
            let raw_value_size = if let Some(Value::Int(n)) = args.get(1) {
                *n
            } else {
                4 // fallback: i32
            };
            let channel = (raw_value_size >> 8) & 0xFF;
            let byte_size = raw_value_size & 0xFF;
            if channel == crate::coroutine_layout::CHANNEL_NONE {
                // `--native` has no transport for this yield type, and the generator's own
                // emitter carries the `compile_error!` that names it and the workaround.
                // Emitting the legacy `as i64` read here would type-check against nothing
                // and bury that message under an E0605/E0308 cascade, so answer with a
                // DIVERGING expression: `!` coerces to whatever the consumer's slot is, and
                // exactly one diagnostic survives — the same rule the routeless-`#native`
                // skip in `generation/mod.rs` follows (loft#1132).
                write!(ctx.w, "unreachable!()")?;
                return Ok(());
            }
            if channel == 1 {
                // @PLAN16 phase 02 — unified channel, layout-driven rebuild.
                // The per-slot kind list rides as extra OpCoroutineNext args
                // (`args[2..]`, each `Int(code)`); the interpreter ignores
                // them, and both ends derive the list from `tuple_kinds` over
                // the same `T`, so producer (`yield_slot_write`) and consumer
                // (`yield_slot_read`) agree by construction.  Buffer size is the
                // accumulated slot count (a `Ref` element takes two slots), not
                // `byte_size`.
                let kinds: Vec<YieldSlot> = args[2..]
                    .iter()
                    .filter_map(|a| match a {
                        Value::Int(c) => Some(YieldSlot::from_code(*c)),
                        _ => None,
                    })
                    .collect();
                if kinds.is_empty() {
                    // Defensive: a channel-1 call without a kind list (should
                    // not occur post-@PLAN16) decodes as the legacy all-i64
                    // tuple so it never silently emits an empty `()`.
                    let slots = (byte_size as usize).div_ceil(8);
                    write!(
                        ctx.w,
                        "{{ let mut _loft_yield_buf: [i64; {slots}] = [0; {slots}]; \
                         loft::codegen_runtime::coroutine_next_into({gen_code}, stores, &mut _loft_yield_buf); \
                         ("
                    )?;
                    for i in 0..slots {
                        if i > 0 {
                            write!(ctx.w, ", ")?;
                        }
                        write!(ctx.w, "_loft_yield_buf[{i}]")?;
                    }
                    write!(ctx.w, ") }}")?;
                    return Ok(());
                }
                let slots = slot_count(&kinds);
                let init = crate::generation::coroutine::yield_buf_init(&kinds);
                write!(
                    ctx.w,
                    "{{ let mut _loft_yield_buf: [i64; {slots}] = {init}; \
                     loft::codegen_runtime::coroutine_next_into({gen_code}, stores, &mut _loft_yield_buf); \
                     ("
                )?;
                let mut slot = 0usize;
                for (i, &kind) in kinds.iter().enumerate() {
                    if i > 0 {
                        write!(ctx.w, ", ")?;
                    }
                    write!(
                        ctx.w,
                        "{}",
                        crate::generation::coroutine::yield_slot_read(kind, slot)
                    )?;
                    slot += kind.width();
                }
                // A 1-element tuple needs the trailing comma to stay a tuple.
                if kinds.len() == 1 {
                    write!(ctx.w, ",")?;
                }
                write!(ctx.w, ") }}")?;
                return Ok(());
            }
            if channel == 2 {
                // @P328 native — unified channel, fn-ref rebuild.  Native
                // fn-ref repr is `(u32 d_nr, DbRef closure)` (see
                // `generation/mod.rs::rust_type` for `Type::Function`).
                // The state machine packs (d_nr, closure) into 2 i64
                // slots; the consumer unpacks to rebuild the tuple:
                //   slot[0]: low 32 bits = d_nr; bits 32..48 = closure.store_nr
                //   slot[1]: low 32 = closure.rec; high 32 = closure.pos
                // An exhausted advance writes nothing, so the buffer's INITIAL value is what
                // the consumer reads then: `store_nr` 0xFFFF makes that the null closure
                // handle, as the interpreter's exhausted fn-ref is (loft#1676).  Zeroed, it
                // decoded as store #0, and the drained loop's break arm freed that store — the
                // generator's own local, on a `yield from` of a lambda.
                write!(
                    ctx.w,
                    "{{ let mut _loft_yield_buf: [i64; 2] = [0xFFFF_i64 << 32, 0]; \
                     loft::codegen_runtime::coroutine_next_into({gen_code}, stores, &mut _loft_yield_buf); \
                     ((_loft_yield_buf[0] as u32), DbRef {{ \
                       store_nr: ((_loft_yield_buf[0] >> 32) & 0xFFFF) as u16, \
                       rec: _loft_yield_buf[1] as u32, \
                       pos: (_loft_yield_buf[1] >> 32) as u32 \
                     }}) }}"
                )?;
                return Ok(());
            }
            // #401 — scalar yields whose Rust type differs from the i64
            // transport.  byte_size alone is ambiguous (8 = i64 OR f64, 4 = i32
            // OR f32, 1 = bool OR u8 enum), so the parser tags these channels.
            if channel == 3 {
                // float: the i64 transport holds the f64 bit-pattern.
                write!(
                    ctx.w,
                    "f64::from_bits(loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) as u64)"
                )?;
                return Ok(());
            }
            if channel == 4 {
                // single: the low 32 bits hold the f32 bit-pattern.
                write!(
                    ctx.w,
                    "f32::from_bits(loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) as u32)"
                )?;
                return Ok(());
            }
            if channel == 5 {
                // enum: the variant index, repr'd as u{8 * byte_size}.
                let uty = match byte_size {
                    1 => "u8",
                    2 => "u16",
                    4 => "u32",
                    _ => "u64",
                };
                write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) as {uty}"
                )?;
                return Ok(());
            }
            match byte_size {
                8 => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_i64({gen_code}, stores)"
                )?,
                1 => write!(
                    ctx.w,
                    "(loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) != 0)"
                )?,
                // size_of::<DbRef>() == 12 — Reference-yielding generator (@P326).
                12 => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_dbref({gen_code}, stores)"
                )?,
                // Text-yielding generator.  `byte_size` is a `text` slot, whose
                // width is HOST-dependent (`&str` = pointer + length: 16 bytes
                // where a pointer is 8 wide, 8 on wasm32).  Matching the literal
                // would silently fall through to the i64 arm on a 32-bit host, so
                // the arm is keyed on the type instead.  Only the host compiles
                // today, which is why a literal survived here.
                n if n as usize == size_of::<&str>() => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_text({gen_code}, stores)"
                )?,
                _ => write!(
                    ctx.w,
                    "loft::codegen_runtime::coroutine_next_i64({gen_code}, stores) as i32"
                )?,
            }
        }
        Ok(())
    }
}

/// `OpCoroutineExhausted` emitter — N8b.2 test whether a native coroutine is
/// exhausted.  `args`: `[gen]` (the generator `DbRef` expression).
pub struct OpCoroutineExhaustedEmitter;

impl OpEmitter for OpCoroutineExhaustedEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if let Some(gen_val) = args.first() {
            let gen_code = ctx.output.generate_expr_buf(gen_val)?;
            write!(
                ctx.w,
                "loft::codegen_runtime::coroutine_is_exhausted({gen_code})"
            )?;
        }
        Ok(())
    }
}
