// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Plan 09 phase 10 step 10.3 — integer-comparison emitter family.
//!
//! Closes P200 (read-side comparison emission, E0308 between
//! narrowed block-tail LHS and `_i64` literal RHS).
//!
//! ## Background
//!
//! Native codegen emits block-tail narrow casts via `narrow_int_cast`:
//! a block whose result is `Type::Integer(narrow_subtype)` gets a
//! `(value) as u8` (or `as u16` / `as i8` / `as i16`) cast at the
//! tail.  This is correct for blocks whose CONSUMER expects the
//! narrow Rust type (e.g. `let var: u8 = (...)`).
//!
//! For integer COMPARISONS, the consumer doesn't need a narrow
//! representation — `==` is logically integer equality.  But the
//! comparison's RHS literal emits as `_i64` (the default Int
//! emission), and Rust requires both sides of `==` to share a type.
//! The mismatch breaks compilation:
//!
//! ```text
//! error[E0308]: mismatched types
//!     ((var__read) as u8) == (0_i64)
//!                            ^^^^^^^ expected `u8`, found `i64`
//! ```
//!
//! ## Fix shape
//!
//! Wrap each operand in `(operand as i64)`.  Both sides become
//! i64; comparison works.  `as i64` is widening for u8/u16/i8/i16
//! and a no-op for i64 itself.
//!
//! Affects 4 Ops: `OpEqInt`, `OpNeInt`, `OpLtInt`, `OpLeInt`.
//! (`OpGtInt` / `OpGeInt` don't exist — the parser desugars `>` and
//! `>=` into `<` / `<=` with swapped operands.)

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

/// Emitter for integer comparison Ops (`OpEqInt`, `OpNeInt`,
/// `OpLtInt`, `OpLeInt`).  Reads the operator from
/// `ctx.def_fn.name` and emits `((lhs) as i64) <op> ((rhs) as i64)`.
pub struct IntCompareEmitter;

impl OpEmitter for IntCompareEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let op = match ctx.def_fn.name() {
            "OpEqInt" => "==",
            "OpNeInt" => "!=",
            "OpLtInt" => "<",
            "OpLeInt" => "<=",
            // Defensive: if the registry adds an Op name this emitter
            // was registered for that we don't recognise, fall through
            // to the default emitter (template substitution).
            _ => return super::default::DefaultEmitter.emit(ctx, args),
        };

        if args.len() != 2 {
            return super::default::DefaultEmitter.emit(ctx, args);
        }

        // `@FR-R-LazySplit` — the loop over a lazy split ends where its element read ran
        // out of pieces; the vector whose length this test compared is never built.
        if op == "<="
            && let Some(vec) = ctx.output.lazy_split_reader(&args[0], "OpLengthVector")
        {
            return write!(ctx.w, "__ls_done_{vec}");
        }

        write!(ctx.w, "((")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ") as i64) {op} ((")?;
        ctx.emit(&args[1])?;
        write!(ctx.w, ") as i64)")
    }
}
