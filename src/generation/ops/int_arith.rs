// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! @PLN157 P3c — integer arithmetic on provably non-sentinel operands.
//!
//! Every integer op's `#rust` template routes through a sentinel-guarded
//! helper (`ops::op_add_int` tests both operands for `i64::MIN` before the
//! checked add).  The helpers are `#[inline]`, so LLVM already folds the
//! tests it can prove — what remains after `-O` is exactly the part no layer
//! below can remove: the MIN branches on operands whose values LLVM cannot
//! bound.  When the emitter can prove both operands non-sentinel
//! (`generation::non_sentinel`), it emits the form with those branches
//! already gone:
//!
//! - `+`/`-`/`*`/unary `-` → the `ops::*_long_nn` family — still
//!   `checked_*`, so overflow → `i64::MIN` (C85's decided edge) is
//!   preserved exactly; only the operand pre-tests drop.
//! - `&`/`|`/`^` → the plain Rust operator.  The guarded body past its MIN
//!   tests is `sentinel_long!`, a debug-only assert — generated code builds
//!   without debug assertions, so plain is release-identical.
//! - `>>` with a literal amount in `0..64` → plain `>>` (the template's
//!   range check is compile-time dead for a literal; the MIN tests are what
//!   the proof discharges).
//! - `OpConvFloatFromInt` → `as f64`; `OpConvBoolFromInt` → `true` (the op
//!   IS the non-sentinel test, and the proof already answered it).
//!
//! Anything unproven falls through to the template unchanged.  Division and
//! remainder stay out: their null is minted by a ZERO divisor, which is not
//! this proof.  `LOFT_NN_VERIFY=1` emits the checking form; `LOFT_NO_NN_FAST=1`
//! disables the pass — see `generation::non_sentinel`.

use super::{EmitCtx, OpEmitter};
use crate::data::Value;
use std::io;

/// How a proven-operand op is emitted.
enum Fast {
    /// `ops::<helper>(a, b)` / `ops::<helper>(a)` — checked arithmetic
    /// without the operand pre-tests.
    Helper(&'static str),
    /// `((a) <op> (b))` — release-identical plain operator.
    Plain(&'static str),
    /// `((a) as f64)`.
    ConvFloat,
    /// `true` — the op is the non-sentinel test itself.
    ConvBool,
}

fn fast_form(op_name: &str, args: &[Value]) -> Option<Fast> {
    match (op_name, args.len()) {
        ("OpAddInt", 2) => Some(Fast::Helper("op_add_long_nn")),
        ("OpMinInt", 2) => Some(Fast::Helper("op_min_long_nn")),
        ("OpMulInt", 2) => Some(Fast::Helper("op_mul_long_nn")),
        ("OpMinSingleInt", 1) => Some(Fast::Helper("op_neg_long_nn")),
        ("OpLandInt", 2) => Some(Fast::Plain("&")),
        ("OpLorInt", 2) => Some(Fast::Plain("|")),
        ("OpEorInt", 2) => Some(Fast::Plain("^")),
        // The literal keeps the template's range check compile-time dead;
        // a computed amount keeps the template (range → C85-null).
        ("OpSRightInt", 2) if matches!(args[1].unspan(), Value::Int(k) if (0..64).contains(k)) => {
            Some(Fast::Plain(">>"))
        }
        ("OpConvFloatFromInt", 1) => Some(Fast::ConvFloat),
        ("OpConvBoolFromInt", 1) => Some(Fast::ConvBool),
        _ => None,
    }
}

pub struct IntArithEmitter;

impl OpEmitter for IntArithEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        let Some(fast) = fast_form(ctx.def_fn.name(), args) else {
            return super::default::DefaultEmitter.emit(ctx, args);
        };
        if ctx.output.nn_fast_disabled || !ctx.output.non_sentinel_args(args) {
            return super::default::DefaultEmitter.emit(ctx, args);
        }
        if ctx.output.nn_verify {
            // Evaluate once, assert the proof per operand, then the fast form.
            write!(ctx.w, "{{ ")?;
            for (i, a) in args.iter().enumerate() {
                write!(ctx.w, "let __nn{i} = (")?;
                ctx.emit(a)?;
                write!(
                    ctx.w,
                    "); assert!(__nn{i} != i64::MIN, \"{}: non-sentinel proof failed\"); ",
                    ctx.def_fn.name()
                )?;
            }
            match fast {
                Fast::Helper(h) => write!(ctx.w, "ops::{h}(__nn0")
                    .and_then(|()| {
                        if args.len() == 2 {
                            write!(ctx.w, ", __nn1")
                        } else {
                            Ok(())
                        }
                    })
                    .and_then(|()| write!(ctx.w, ") }}")),
                Fast::Plain(op) => write!(ctx.w, "__nn0 {op} __nn1 }}"),
                Fast::ConvFloat => write!(ctx.w, "__nn0 as f64 }}"),
                Fast::ConvBool => write!(ctx.w, "true }}"),
            }
        } else {
            match fast {
                Fast::Helper(h) => {
                    write!(ctx.w, "ops::{h}((")?;
                    ctx.emit(&args[0])?;
                    if args.len() == 2 {
                        write!(ctx.w, "), (")?;
                        ctx.emit(&args[1])?;
                    }
                    write!(ctx.w, "))")
                }
                Fast::Plain(op) => {
                    write!(ctx.w, "((")?;
                    ctx.emit(&args[0])?;
                    write!(ctx.w, ") {op} (")?;
                    ctx.emit(&args[1])?;
                    write!(ctx.w, "))")
                }
                Fast::ConvFloat => {
                    write!(ctx.w, "((")?;
                    ctx.emit(&args[0])?;
                    write!(ctx.w, ") as f64)")
                }
                Fast::ConvBool => write!(ctx.w, "(true)"),
            }
        }
    }
}
