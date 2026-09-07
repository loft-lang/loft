// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! @PLN157 P3 — float/single comparison emitter.
//!
//! loft's float compares define a total order with null (NaN) below every
//! number, so the `#rust` templates expand each one NaN-aware.  When BOTH
//! operands are provably non-sentinel (`generation::non_sentinel`), the plain
//! Rust operator computes the same boolean — and unlike the expansion it is a
//! single flag-setting compare LLVM can fold into the surrounding loop.
//! Anything unproven falls through to the template unchanged.
//!
//! `LOFT_NN_VERIFY=1` emits the checking form (assert the proof, then compare
//! plain); `LOFT_NO_NN_FAST=1` disables the pass — see the module doc of
//! `generation::non_sentinel`.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use crate::generation::non_sentinel;
use std::io;

pub struct FloatCompareEmitter;

impl OpEmitter for FloatCompareEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let Some(op) = non_sentinel::plain_float_compare(ctx.def_fn.name()) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        if args.len() != 2
            || ctx.output.nn_fast_disabled
            || !ctx.output.non_sentinel_float_pair(&args[0], &args[1])
        {
            return super::default::DefaultEmitter.emit(ctx, args);
        }
        if ctx.output.nn_verify {
            write!(ctx.w, "{{ let __nn1 = (")?;
            ctx.emit(&args[0])?;
            write!(ctx.w, "); let __nn2 = (")?;
            ctx.emit(&args[1])?;
            write!(
                ctx.w,
                "); assert!(!__nn1.is_nan() && !__nn2.is_nan(), \"{}: non-sentinel proof failed\"); __nn1 {op} __nn2 }}",
                ctx.def_fn.name()
            )
        } else {
            write!(ctx.w, "((")?;
            ctx.emit(&args[0])?;
            write!(ctx.w, ") {op} (")?;
            ctx.emit(&args[1])?;
            write!(ctx.w, "))")
        }
    }
}
