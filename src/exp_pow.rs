// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-ExpPow` — `E` raised to a power is the exponential function: `pow(E, x)` is
//! `exp(x)`, computed by the platform's `exp` instead of its general `pow`.
//!
//! `pow(E, x)` and `exp(x)` name the same mathematical function; a general `pow` computes it as
//! `exp(x · ln E)` with the rounding of the double `E` carried in, while `exp` is the dedicated
//! routine every reference language calls (Rust's `f64::exp`, C's `exp`) — faster, and as close
//! or closer to the true value.  The two can differ in the LAST bit of the answer, so this fold
//! is an owner's decision recorded with the rule (the answer moves toward the reference's).
//! Every special value answers the same: a null (NaN) operand answers null, `±0` answers 1,
//! `-inf` answers 0, `+inf` and an overflow answer `+inf`.
//!
//! Folded, for `float` and `single`: the operator `OpPowFloat(E, x)` (the stdlib `exp` and
//! `pow` inlined) and the stdlib call `pow(E, x)`, where `E` is the constant `OpMathEFloat()`
//! (cast to `single` for the single form) or a literal with exactly its bits.  The result is
//! `OpMathFuncFloat(10, x)` / `OpMathFuncSingle(10, x)`.  One IR rewrite on the closed program,
//! so every backend gets it; `LOFT_NO_EXP_POW=1` is the switch.
use crate::data::{Data, DefType, Value};

/// The math function id of `exp` in `OpMathFuncFloat` / `OpMathFuncSingle`.
const EXP_ID: i32 = 10;

struct Ops {
    pow_float: [u32; 2],
    pow_single: [u32; 2],
    e_float: u32,
    single_from_float: u32,
    func_float: u32,
    func_single: u32,
}

impl Ops {
    fn lookup(data: &Data) -> Option<Self> {
        let nr = |n: &str| {
            let d = data.def_nr(n);
            (d != u32::MAX).then_some(d)
        };
        Some(Self {
            pow_float: [nr("OpPowFloat")?, nr("t_5float_pow").unwrap_or(u32::MAX)],
            pow_single: [nr("OpPowSingle")?, nr("t_6single_pow").unwrap_or(u32::MAX)],
            e_float: nr("OpMathEFloat")?,
            single_from_float: nr("OpCastSingleFromFloat")?,
            func_float: nr("OpMathFuncFloat")?,
            func_single: nr("OpMathFuncSingle")?,
        })
    }

    /// Is `v` the double `E`?
    fn is_e(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Call(d, args) => *d == self.e_float && args.is_empty(),
            Value::Float(f) => f.to_bits() == std::f64::consts::E.to_bits(),
            _ => false,
        }
    }

    /// Is `v` the single `E`: the double `E` cast down, or a literal with its bits?
    fn is_single_e(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Call(d, args) => {
                *d == self.single_from_float && args.len() == 1 && self.is_e(&args[0])
            }
            #[allow(clippy::cast_possible_truncation)]
            Value::Single(f) => f.to_bits() == (std::f64::consts::E as f32).to_bits(),
            _ => false,
        }
    }
}

/// Fold every `pow(E, x)` of the program into `exp(x)`; a no-op under the switch.
pub fn rewrite_program(data: &mut Data) {
    if !crate::keys::exp_pow_enabled() {
        return;
    }
    let Some(ops) = Ops::lookup(data) else {
        return;
    };
    let mut fired = 0usize;
    for d_nr in 0..data.definitions.len() {
        let d = d_nr as u32;
        if data.def_type(d) != DefType::Function || !data.def(d).is_loft_defined() {
            continue;
        }
        let mut code = std::mem::replace(&mut data.definitions[d_nr].code, Value::Null);
        code.map_nodes(&mut |n| {
            let Value::Call(op, args) = n else {
                return;
            };
            if args.len() != 2 {
                return;
            }
            let func = if ops.pow_float.contains(op) && ops.is_e(&args[0]) {
                ops.func_float
            } else if ops.pow_single.contains(op) && ops.is_single_e(&args[0]) {
                ops.func_single
            } else {
                return;
            };
            *op = func;
            args[0] = Value::Int(EXP_ID);
            fired += 1;
        });
        data.definitions[d_nr].code = code;
    }
    crate::rewrite_census::fired("R-ExpPow", fired);
}
