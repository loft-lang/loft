// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-RangedCall`'s facts, which every writer reads: whether a call of a function repays a
//! ranged variant (the gain test), whether a call's integer arguments are proven within
//! `[-L, L]`, and the facts a ranged variant's body runs under (its integer parameters seeded
//! to `[-L, L]`).  The writers keep only their spelling — native's `__rg` variants and guarded
//! copy (`generation::ranged_call`), the C's `__rg` twin (`c::RG_BIT`).  `LOFT_NO_RANGED_CALLS=1`
//! keeps every template on both (`range::ranged_calls_enabled`).

use crate::data::{Data, Type, Value};
use crate::opt::range::{self, Range, Ranges};
use std::collections::HashMap;

/// The bound a ranged variant's integer parameters and scalar inputs are assumed within.
/// `2^30` leaves room for the products a grid index takes (`(2L+1)·L·3 < 2^63`).
pub const L: i64 = 1 << 30;

/// The checked operators a callee must run for its ranged variant to be worth emitting.
const MIN_CHECKED_OPS: usize = 3;

/// Is `r` within `[-b, b]`?
#[must_use]
pub fn within(r: Range, b: i64) -> bool {
    r.0 >= -b && r.1 <= b
}

fn is_integer(tp: &Type) -> bool {
    matches!(tp.base(), Type::Integer(_))
}

/// The checked integer operators a call of `d` runs — its own and its callees', three deep —
/// which a ranged variant may turn plain.
fn checked_ops(data: &Data, d: u32, depth: u8) -> usize {
    if depth > 3 || (d as usize) >= data.definitions.len() {
        return 0;
    }
    let def = data.def(d);
    if !def.is_loft_defined() || matches!(def.returned().base(), Type::Iterator(_, _)) {
        return 0;
    }
    let mut n = 0;
    def.code().walk(&mut |c| {
        let Value::Call(c, _) = c else { return };
        if (*c as usize) >= data.definitions.len() {
            return;
        }
        let cd = data.def(*c);
        if matches!(
            cd.name(),
            "OpAddInt"
                | "OpMinInt"
                | "OpMulInt"
                | "OpMinSingleInt"
                | "OpAddIntNullable"
                | "OpMinIntNullable"
                | "OpMulIntNullable"
        ) {
            n += 1;
        } else if cd.is_loft_defined() && *c != d {
            n += checked_ops(data, *c, depth + 1);
        }
    });
    n
}

/// Does a call of `d` repay a ranged variant — and, in a guarded copy, the duplicated loop?
/// The guard and the copy cost per loop and per code byte; a callee with one or two checked
/// operators saves too little to repay them (`(R-GuardedChain)` measured the same trade:
/// one-operator loops LOST 18–50 %).  Three or more reachable operators qualify.
#[must_use]
pub fn benefits(data: &Data, d: u32) -> bool {
    checked_ops(data, d, 0) >= MIN_CHECKED_OPS
}

/// Are `d`'s integer arguments `args` all proven within `[-L, L]` under `facts` — at least one
/// of them?
#[must_use]
pub fn args_ranged(
    data: &Data,
    d: u32,
    args: &[Value],
    facts: &Ranges,
    nn: &HashMap<u16, bool>,
) -> bool {
    let attrs = data.def(d).attributes();
    if attrs.len() < args.len() {
        return false;
    }
    let mut any = false;
    for (at, arg) in attrs.iter().zip(args) {
        if !is_integer(&at.typedef) {
            continue;
        }
        match range::range(data, nn, facts, arg, 0) {
            Some(r) if within(r, L) => any = true,
            _ => return false,
        }
    }
    any
}

/// The seeds of `d`'s ranged variant: each visible integer parameter within `[-L, L]`.
#[must_use]
pub fn param_seeds(data: &Data, d: u32) -> Ranges {
    let def = data.def(d);
    let vars = def.variables();
    let mut seeds = Ranges::default();
    for at in def.attributes() {
        let v = vars.var(&at.name);
        if v != u16::MAX && is_integer(vars.tp(v)) && !at.hidden {
            seeds.vars.insert(v, (-L, L));
        }
    }
    seeds
}

/// The facts `d`'s body runs under from `seeds` (`range_vars` drops the seed of a parameter
/// the body assigns).
#[must_use]
pub fn ranges_under(data: &Data, d: u32, seeds: &Ranges) -> Ranges {
    let def = data.def(d);
    let nn = crate::opt::non_sentinel::non_sentinel_vars(data, def.variables(), def.code());
    range::range_vars(
        data,
        def.variables(),
        def.code(),
        &nn,
        &std::collections::BTreeMap::new(),
        seeds,
    )
}

/// Does `d`'s ranged variant turn any operator plain that its plain body keeps checked — one of
/// its own, under its parameters seeded `[-L, L]`, or one of a callee whose call it newly
/// ranges and whose variant gains in turn?  A variant equal to its plain body only duplicates
/// code: a function whose arithmetic is all in the apart variants it calls (`__apv`) has
/// nothing for the ranges to change.  Memoised in `memo`; a recursive function meets itself as
/// no gain.
pub fn variant_gains(data: &Data, d: u32, depth: u8, memo: &mut HashMap<u32, bool>) -> bool {
    if depth > 3 || !benefits(data, d) {
        return false;
    }
    if let Some(g) = memo.get(&d) {
        return *g;
    }
    memo.insert(d, false);
    let def = data.def(d);
    let code = def.code();
    let seeded = ranges_under(data, d, &param_seeds(data, d));
    let nn = crate::opt::non_sentinel::non_sentinel_vars(data, def.variables(), code);
    let plain = range::range_vars(
        data,
        def.variables(),
        code,
        &nn,
        &std::collections::BTreeMap::new(),
        &Ranges::default(),
    );
    let mut gain = false;
    let mut calls: Vec<u32> = Vec::new();
    code.walk(&mut |n| {
        let Value::Call(c, args) = n else { return };
        if gain || (*c as usize) >= data.definitions.len() {
            return;
        }
        let name = data.def(*c).name();
        if range::plain_form(name, args.len()).is_some() {
            gain = range::op_range(data, &nn, &seeded, name, args, 0).is_some()
                && range::op_range(data, &nn, &plain, name, args, 0).is_none();
        } else if *c != d
            && data.def(*c).is_loft_defined()
            && args_ranged(data, *c, args, &seeded, &nn)
            && !args_ranged(data, *c, args, &plain, &nn)
        {
            calls.push(*c);
        }
    });
    if !gain {
        gain = calls
            .into_iter()
            .any(|c| variant_gains(data, c, depth + 1, memo));
    }
    memo.insert(d, gain);
    gain
}
