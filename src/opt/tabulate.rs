// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — shared by the native Rust and the C99 generators

//! `@FR-R-Tabulate` — a branchy classifier of a byte answered from a table.
//!
//! A function of one integer answering an integer, whose body the compiler can evaluate, is
//! evaluated here for every argument in `0..256`; when every evaluation answers a plain value
//! the writer emits the answers as a table the function reads for an argument in that range,
//! its body running for every other argument.  The evaluation replays loft's checked
//! integer arithmetic, so an entry is exactly what the body answers; an argument whose
//! evaluation would overflow, answer the null sentinel or reach a node this evaluator does not
//! model declines the whole function — no fault moves from a call to compile time (@C120).

use crate::data::{Data, Type, Value};
use std::collections::HashMap;

/// The arguments a table covers: a byte.
pub const DOMAIN: i64 = 256;

/// The branches a body takes at least before a table pays for its load.
const MIN_BRANCHES: usize = 8;

fn enabled() -> bool {
    !crate::env_once!(std::env::var("LOFT_NO_TABULATE").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_TABULATE").is_ok_and(|v| v != "0"))
}

/// Plain `integer`: the 64-bit value the writers carry, null its sentinel.
fn plain_integer(tp: &Type) -> bool {
    matches!(tp, Type::Integer(s) if s.is_signed32_template() || s.is_wide_template())
}

/// The table of function `d` — its answer for every argument in `0..DOMAIN` — or `None` when
/// `d` is not such a classifier or one evaluation declines.
#[must_use]
pub fn table(data: &Data, d: u32) -> Option<Vec<i64>> {
    if !enabled() || (d as usize) >= data.definitions.len() {
        return None;
    }
    let def = data.def(d);
    let decline = |why: &str| -> Option<Vec<i64>> {
        if trace() {
            eprintln!("tabulate: {} declined — {why}", def.name());
        }
        None
    };
    let Value::Block(_) = def.code() else {
        return None;
    };
    if !def.rust().is_empty() {
        return None;
    }
    let [param] = def.attributes() else {
        return None;
    };
    if param.hidden || !plain_integer(&param.typedef) || !plain_integer(def.returned()) {
        return None;
    }
    let p = def.variables().var(&param.name);
    if p == u16::MAX {
        return None;
    }
    let mut branches = 0usize;
    def.code().walk(&mut |v| {
        if matches!(v, Value::If(..)) {
            branches += 1;
        }
    });
    if branches < MIN_BRANCHES {
        return decline(&format!("{branches} branches, under {MIN_BRANCHES}"));
    }
    let ops = Ops::of(data);
    let mut out = Vec::with_capacity(DOMAIN as usize);
    for x in 0..DOMAIN {
        let mut env: HashMap<u16, i64> = HashMap::from([(p, x)]);
        let answer = match eval(&ops, def.code(), &mut env) {
            Some(Out::Int(v) | Out::Return(v)) => v,
            _ => return decline(&format!("the body does not evaluate at {x}")),
        };
        out.push(answer);
    }
    if trace() {
        eprintln!("tabulate: {} admitted ({branches} branches)", def.name());
    }
    crate::rewrite_census::fired("R-Tabulate", 1);
    Some(out)
}

/// The operators the evaluator models, by definition number.
struct Ops {
    names: HashMap<u32, &'static str>,
}

// The `Nullable` forms are the same arithmetic spelled for an operand that may be null: the
// evaluation never holds a null, so on its values they answer what the plain forms answer.
const MODELLED: [&str; 16] = [
    "OpAddInt",
    "OpMinInt",
    "OpMulInt",
    "OpAddIntNullable",
    "OpMinIntNullable",
    "OpMulIntNullable",
    "OpMinSingleInt",
    "OpEqInt",
    "OpNeInt",
    "OpLtInt",
    "OpLeInt",
    "OpGtInt",
    "OpGeInt",
    "OpLandInt",
    "OpLorInt",
    "OpEorInt",
];

impl Ops {
    fn of(data: &Data) -> Ops {
        let names = MODELLED
            .iter()
            .filter_map(|n| {
                let d = data.def_nr(n);
                (d != u32::MAX).then_some((d, *n))
            })
            .collect();
        Ops { names }
    }
}

/// What a node evaluates to: an integer, a boolean, nothing (a statement), or a `return`
/// that ends the body.
enum Out {
    Int(i64),
    Bool(bool),
    Unit,
    Return(i64),
}

/// A plain integer: never the null sentinel, which every checked operator answers on a fault.
fn plain(v: Option<i64>) -> Option<i64> {
    v.filter(|v| *v != i64::MIN)
}

/// `v` evaluated with the locals in `env`, or `None` for anything the evaluator does not
/// model, an overflow, or a null — each of which declines the table rather than guess.
fn eval(ops: &Ops, v: &Value, env: &mut HashMap<u16, i64>) -> Option<Out> {
    match v {
        Value::Span(b) => eval(ops, &b.1, env),
        Value::Line(_) | Value::Null => Some(Out::Unit),
        Value::Int(k) => Some(Out::Int(i64::from(*k))),
        Value::Long(l) => plain(Some(*l)).map(Out::Int),
        Value::Boolean(b) => Some(Out::Bool(*b)),
        Value::Var(x) => env.get(x).copied().map(Out::Int),
        Value::Set(x, e) => {
            let Out::Int(val) = eval(ops, e, env)? else {
                return None;
            };
            env.insert(*x, val);
            Some(Out::Unit)
        }
        Value::Return(e) => match eval(ops, e, env)? {
            Out::Int(val) => Some(Out::Return(val)),
            _ => None,
        },
        Value::If(c, t, e) => match eval(ops, c, env)? {
            Out::Bool(true) => eval(ops, t, env),
            Out::Bool(false) => eval(ops, e, env),
            _ => None,
        },
        Value::Block(b) => {
            let mut last = Out::Unit;
            for op in &b.operators {
                last = eval(ops, op, env)?;
                if let Out::Return(_) = last {
                    return Some(last);
                }
            }
            Some(last)
        }
        Value::Call(d, args) => {
            let name = ops.names.get(d)?;
            let int = |a: &Value, env: &mut HashMap<u16, i64>| match eval(ops, a, env)? {
                Out::Int(v) => Some(v),
                _ => None,
            };
            if *name == "OpMinSingleInt" {
                let [a] = &args[..] else { return None };
                return plain(int(a, env)?.checked_neg()).map(Out::Int);
            }
            let [a, b] = &args[..] else { return None };
            let (a, b) = (int(a, env)?, int(b, env)?);
            Some(match *name {
                "OpAddInt" | "OpAddIntNullable" => Out::Int(plain(a.checked_add(b))?),
                "OpMinInt" | "OpMinIntNullable" => Out::Int(plain(a.checked_sub(b))?),
                "OpMulInt" | "OpMulIntNullable" => Out::Int(plain(a.checked_mul(b))?),
                "OpLandInt" => Out::Int(plain(Some(a & b))?),
                "OpLorInt" => Out::Int(plain(Some(a | b))?),
                "OpEorInt" => Out::Int(plain(Some(a ^ b))?),
                "OpEqInt" => Out::Bool(a == b),
                "OpNeInt" => Out::Bool(a != b),
                "OpLtInt" => Out::Bool(a < b),
                "OpLeInt" => Out::Bool(a <= b),
                "OpGtInt" => Out::Bool(a > b),
                "OpGeInt" => Out::Bool(a >= b),
                _ => return None,
            })
        }
        _ => None,
    }
}
