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

/// Division or remainder by a LITERAL that is neither `0` nor `-1`, as the plain Rust
/// operator behind one sentinel test.  The null a `/` or `%` mints comes from a zero
/// divisor, from the `MIN / -1` overflow, or from a null operand; a literal rules the first
/// two out at generation time, so the template's guarded call is exactly
/// `if x == MIN { MIN } else { x / k }` — and needs no proof of the dividend, which is
/// what lets it fire on an arithmetic result the non-sentinel pass never trusts.  LLVM
/// turns the plain division by a constant into a multiply-and-shift; the guarded call
/// never became one.  The template's fault note is dropped with it: it fires only when the
/// result is the sentinel while neither operand is, which a literal divisor makes
/// impossible.
fn literal_divisor_form(op_name: &str, args: &[Value]) -> Option<&'static str> {
    let [_, k] = args else {
        return None;
    };
    let ok = match k.unspan() {
        Value::Int(k) => *k != 0 && *k != -1,
        Value::Long(k) => *k != 0 && *k != -1,
        _ => false,
    };
    if !ok {
        return None;
    }
    match op_name {
        "OpDivIntNullable" | "OpDivInt" => Some("/"),
        "OpRemIntNullable" | "OpRemInt" => Some("%"),
        _ => None,
    }
}

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

/// The release-pass PROBE's form of an op (`LOFT_RELEASE_PASS_PROBE=1`): the processor's
/// wrapping arithmetic, a zero divisor still answering the sentinel (a Rust division by
/// zero would abort, and the probe measures speed, not that).  The `*Nullable` twins and
/// the null test itself (`OpConvBoolFromInt`) keep their templates — they ARE the
/// language's null semantics, not its fault protection.  `None` keeps the ordinary path.
fn probe_form(op_name: &str, args: &[Value]) -> Option<&'static str> {
    match (op_name, args.len()) {
        ("OpAddInt", 2) => Some("wrapping_add"),
        ("OpMinInt", 2) => Some("wrapping_sub"),
        ("OpMulInt", 2) => Some("wrapping_mul"),
        ("OpDivInt", 2) => Some("wrapping_div"),
        ("OpRemInt", 2) => Some("wrapping_rem"),
        ("OpMinSingleInt", 1) => Some("wrapping_neg"),
        ("OpLandInt", 2) => Some("&"),
        ("OpLorInt", 2) => Some("|"),
        ("OpEorInt", 2) => Some("^"),
        ("OpSRightInt", 2) if matches!(args[1].unspan(), Value::Int(k) if (0..64).contains(k)) => {
            Some(">>")
        }
        _ => None,
    }
}

/// A guarded plain nest's form of an op (`@FR-R-BoundedNest`, `Output::plain_nest > 0`):
/// the processor's operator for exactly the four operators [`hoist::bounded_nest`] admits —
/// the nest's guard has proved none of them can fault in the loop's extent.  `None` keeps
/// the ordinary path, so an op the matcher never admits (it cannot appear in an admitted
/// nest, but the counter test's compare and the discharge's null test do) is untouched.
fn nest_form(op_name: &str, args: &[Value]) -> Option<&'static str> {
    match (op_name, args.len()) {
        // The `*Nullable` twins reach here only through a guarded CHAIN (`@FR-R-GuardedChain`
        // admits them; the nest matcher never does), and a chain the guard admits has no
        // fault for the twin to be silent about.
        ("OpAddInt" | "OpAddIntNullable", 2) => Some("wrapping_add"),
        ("OpMinInt" | "OpMinIntNullable", 2) => Some("wrapping_sub"),
        ("OpMulInt" | "OpMulIntNullable", 2) => Some("wrapping_mul"),
        ("OpMinSingleInt", 1) => Some("wrapping_neg"),
        _ => None,
    }
}

/// `@FR-R-Range`'s plain form: the wrapping operator for `+ - *` and negation (the exact
/// value — the range proved no step overflows), the bare `/` or `%` for a division whose
/// divisor's range excludes zero.
fn write_range_plain(ctx: &mut EmitCtx<'_, '_>, form: &str, args: &[Value]) -> io::Result<()> {
    if form == "/" || form == "%" {
        write!(ctx.w, "((")?;
        ctx.emit(&args[0])?;
        write!(ctx.w, ") {form} (")?;
        ctx.emit(&args[1])?;
        return write!(ctx.w, "))");
    }
    write_plain(ctx, form, args)
}

/// `((a).wrapping_add(b))` / `((a).wrapping_neg())` — the nest form of one op.
fn write_plain(ctx: &mut EmitCtx<'_, '_>, form: &str, args: &[Value]) -> io::Result<()> {
    write!(ctx.w, "((")?;
    ctx.emit(&args[0])?;
    if form == "wrapping_neg" {
        return write!(ctx.w, ").wrapping_neg())");
    }
    write!(ctx.w, ").{form}(")?;
    ctx.emit(&args[1])?;
    write!(ctx.w, "))")
}

impl OpEmitter for IntArithEmitter {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()> {
        if ctx.output.plain_nest > 0
            && !ctx.output.release_pass_probe
            && let Some(form) = nest_form(ctx.def_fn.name(), args)
        {
            if ctx.output.hoist_verify {
                // `LOFT_HOIST_VERIFY=1` — the plain answer beside the checked template's,
                // compared at the operator (`ops::nest_verify`); the checked copy is the
                // ordinary emission, taken with the nest mode suspended.
                write!(ctx.w, "ops::nest_verify(")?;
                write_plain(ctx, form, args)?;
                write!(ctx.w, ", ")?;
                let depth = ctx.output.plain_nest;
                ctx.output.plain_nest = 0;
                let checked = self.emit(ctx, args);
                ctx.output.plain_nest = depth;
                checked?;
                return write!(ctx.w, ", \"{}\")", ctx.def_fn.name());
            }
            return write_plain(ctx, form, args);
        }
        // `@FR-R-GuardedChain` — an op of an admitted chain: the plain operator under the
        // loop's guard, the checked template otherwise.  The guard is loop-invariant, so LLVM
        // unswitches the loop on it once and the body is emitted once.
        if !ctx.output.release_pass_probe
            && let Some(guard) = ctx.output.plain_chain_guard(args)
            && let Some(form) = nest_form(ctx.def_fn.name(), args)
        {
            let name = ctx.def_fn.name();
            if let Some(g) = &guard {
                write!(ctx.w, "(if {g} {{ ")?;
            }
            if ctx.output.hoist_verify {
                write!(ctx.w, "ops::range_verify(")?;
                write_plain(ctx, form, args)?;
                write!(ctx.w, ", ")?;
                ctx.output.chains_suspended += 1;
                let checked = self.emit(ctx, args);
                ctx.output.chains_suspended -= 1;
                checked?;
                write!(ctx.w, ", \"{name}\")")?;
            } else {
                write_plain(ctx, form, args)?;
            }
            if guard.is_some() {
                write!(ctx.w, " }} else {{ ")?;
                ctx.output.chains_suspended += 1;
                let checked = self.emit(ctx, args);
                ctx.output.chains_suspended -= 1;
                checked?;
                write!(ctx.w, " }})")?;
            }
            return Ok(());
        }
        if ctx.output.release_pass_probe
            && let Some(form) = probe_form(ctx.def_fn.name(), args)
        {
            match form {
                "&" | "|" | "^" | ">>" => {
                    write!(ctx.w, "((")?;
                    ctx.emit(&args[0])?;
                    write!(ctx.w, ") {form} (")?;
                    ctx.emit(&args[1])?;
                    return write!(ctx.w, "))");
                }
                "wrapping_neg" => {
                    write!(ctx.w, "((")?;
                    ctx.emit(&args[0])?;
                    return write!(ctx.w, ").wrapping_neg())");
                }
                "wrapping_div" | "wrapping_rem" => {
                    write!(ctx.w, "{{ let _a = (")?;
                    ctx.emit(&args[0])?;
                    write!(ctx.w, "); let _b = (")?;
                    ctx.emit(&args[1])?;
                    return write!(
                        ctx.w,
                        "); if _b == 0 {{ i64::MIN }} else {{ _a.{form}(_b) }} }}"
                    );
                }
                _ => {
                    write!(ctx.w, "((")?;
                    ctx.emit(&args[0])?;
                    write!(ctx.w, ").{form}(")?;
                    ctx.emit(&args[1])?;
                    return write!(ctx.w, "))");
                }
            }
        }
        // `@FR-R-Range` — the result provably fits the type, so no operation can fault and
        // the processor's operator answers exactly what the checked template would.
        if !ctx.output.nn_fast_disabled
            && let Some(form) = crate::generation::range::plain_form(ctx.def_fn.name(), args.len())
            && ctx.output.op_range(ctx.def_fn.name(), args).is_some()
        {
            if ctx.output.hoist_verify {
                write!(ctx.w, "ops::range_verify(")?;
                write_range_plain(ctx, form, args)?;
                write!(ctx.w, ", ")?;
                // The checked copy: the ordinary emission with the range arm held off.
                let name = ctx.def_fn.name();
                ctx.output.range_suspended += 1;
                let checked = self.emit(ctx, args);
                ctx.output.range_suspended -= 1;
                checked?;
                return write!(ctx.w, ", \"{name}\")");
            }
            return write_range_plain(ctx, form, args);
        }
        if !ctx.output.nn_fast_disabled
            && let Some(sym) = literal_divisor_form(ctx.def_fn.name(), args)
        {
            write!(ctx.w, "{{ let _d = (")?;
            ctx.emit(&args[0])?;
            write!(
                ctx.w,
                "); if _d == i64::MIN {{ i64::MIN }} else {{ _d {sym} ("
            )?;
            ctx.emit(&args[1])?;
            return write!(ctx.w, ") }} }}");
        }
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
