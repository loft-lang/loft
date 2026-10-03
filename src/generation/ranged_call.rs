// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-RangedCall` — integer arithmetic over a helper's parameters, made plain where a
//! guard proved the values bounded.
//!
//! `(R-Range)` never ranges a parameter or a record read by shape (C80): a non-null integer
//! can hold the sentinel.  So the per-slot helpers of a grid walk — `nb_q(q, r, d)`'s `q + 1`,
//! `eg_index`'s `(dr * gw + dq) * 3 + slot` — keep every checked operator, and the checked
//! operators' null and overflow branches are what stop LLVM folding the chain.  This module
//! supplies the bound from the CALLER, where it can be tested once:
//!
//! - **The guarded copy.**  A counted `For` block whose loop hoists integer record scalars
//!   (`e.eg_q0`, `e.eg_w`, …, invariant over the loop by the hoist's own write set) is emitted
//!   twice behind `|x| <= G` for each of them.  In the guarded copy those field reads carry
//!   `[-G, G]`, and every fact `(R-Range)` derives from them follows: the range ends, the
//!   counters, `cq = e.eg_q0 + iq`.  A fact about a local the copy derives is kept only when
//!   every assignment of that local in the whole function lies inside the block and it never
//!   escapes by reference, so a value computed before the loop never inherits a seed.
//! - **The ranged variant.**  A call whose every integer argument (and every twin scalar
//!   input) is proven within `[-L, L]` calls `f__rg` — the same body, emitted with its integer
//!   parameters and scalar inputs seeded to `[-L, L]` (a parameter the body assigns is not
//!   seeded).  Inside it `(R-Range)` decides every operator by interval arithmetic as usual,
//!   and its own calls are routed by the same test, so the chain is ranged end to end.
//!
//! `L = 2^30` leaves room for the products a grid index takes (`(2L+1)·L·3 < 2^63`); `G =
//! 2^20` leaves `L` room for the sums a guarded copy forms before it calls.  Every value the
//! copy and the variants compute is the checked template's — the bounds only make the
//! operators provably unable to fault.  `LOFT_NO_RANGED_CALLS=1` keeps every template;
//! `LOFT_HOIST_VERIFY=1` checks each plain operator as `(R-Range)` does.

use super::range::{self, Range, Ranges};
use crate::data::{Block, Data, Type, Value};
use std::collections::HashSet;

/// The bound a ranged variant's integer parameters and scalar inputs are assumed within.
pub const L: i64 = 1 << 30;
/// The bound a guarded copy tests each hoisted field against.
pub const G: i64 = 1 << 20;

fn within(r: Range, b: i64) -> bool {
    r.0 >= -b && r.1 <= b
}

fn is_integer(tp: &Type) -> bool {
    matches!(tp.base(), Type::Integer(_))
}

/// Does a call of `d` gain from ranged parameters: its body (or a callee's, three deep) does
/// checked integer arithmetic?  A variant of a function that does none is code for nothing.
pub fn benefits(data: &Data, d: u32, depth: u8) -> bool {
    if depth > 3 || (d as usize) >= data.definitions.len() {
        return false;
    }
    let def = data.def(d);
    if !def.is_loft_defined() || matches!(def.returned(), Type::Iterator(_, _)) {
        return false;
    }
    def.code().any_node(&mut |n| {
        let Value::Call(c, _) = n else { return false };
        if (*c as usize) >= data.definitions.len() {
            return false;
        }
        let cd = data.def(*c);
        matches!(
            cd.name(),
            "OpAddInt"
                | "OpMinInt"
                | "OpMulInt"
                | "OpMinSingleInt"
                | "OpAddIntNullable"
                | "OpMinIntNullable"
                | "OpMulIntNullable"
        ) || (cd.is_loft_defined() && *c != d && benefits(data, *c, depth + 1))
    })
}

/// The variables written anywhere in `code` outside the block `skip` (by address).
fn sets_outside(code: &Value, skip: *const Block, out: &mut HashSet<u16>) {
    if let Value::Block(b) | Value::Loop(b) = code
        && std::ptr::eq(&**b, skip)
    {
        return;
    }
    if let Value::Set(v, _) | Value::TuplePut(v, _, _) = code {
        out.insert(*v);
    }
    code.for_each_child(&mut |c| sets_outside(c, skip, out));
}

impl super::Output<'_> {
    /// `@FR-R-RangedCall`'s guarded copy — for a `For` block, the run-time test and the
    /// facts its guarded copy runs under, or `None` when the copy would call no ranged
    /// variant (it is then not worth its duplicated code).
    pub(super) fn range_guard_for(&mut self, bl: &Block) -> Option<(String, Ranges)> {
        if !range::ranged_calls_enabled()
            || self.range_arith_disabled
            || self.hoist_disabled
            || self.in_coroutine_body
            || bl.operators.iter().any(|op| {
                op.any_node(&mut |n| {
                    matches!(
                        n,
                        Value::Yield(_) | Value::Parallel(_) | Value::CallRef(_, _)
                    )
                })
            })
        {
            return None;
        }
        // The block's loop: the hoisted integer record scalars are its guard's leaves.
        let lp = bl.operators.iter().find_map(|op| match op.unspan() {
            Value::Loop(lp) => Some(lp.clone()),
            _ => None,
        })?;
        let hoisted = self.compute_loop_hoist(&lp);
        let mut fields: Vec<((u16, i64), Value)> = Vec::new();
        for (key, getter) in hoisted.scalars {
            if let Value::Call(g, args) = getter.unspan()
                && self.data.def(*g).name() == "OpGetInt"
                && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == key.0)
            {
                fields.push((key, getter.clone()));
            }
        }
        // The block's PRELUDE reads too — a non-literal range end is taken into a local
        // before the loop (`@FR-I-Range`).  Seeding the field is sound only where the seed
        // stays true: the read runs right after the guard (the prelude calls no user
        // function that could write it first), and inside the loop the field is either
        // hoisted (invariant) or not read at all.
        let at_loop = bl
            .operators
            .iter()
            .position(|op| matches!(op.unspan(), Value::Loop(_)))?;
        let prelude = &bl.operators[..at_loop];
        let calls_user = prelude.iter().any(|op| {
            op.any_node(&mut |n| {
                matches!(n, Value::Call(c, _) if (*c as usize) < self.data.definitions.len()
                    && self.data.def(*c).is_loft_defined())
            })
        });
        if !calls_user {
            let mut extra: Vec<((u16, i64), Value)> = Vec::new();
            for op in prelude {
                op.walk(&mut |n| {
                    if let Value::Call(g, args) = n
                        && self.data.def(*g).name() == "OpGetInt"
                        && let (Some(Value::Var(v)), Some(Value::Int(off))) = (
                            args.first().map(Value::unspan),
                            args.get(1).map(Value::unspan),
                        )
                    {
                        let key = (*v, i64::from(*off));
                        if !fields.iter().any(|(k, _)| *k == key)
                            && !extra.iter().any(|(k, _)| *k == key)
                        {
                            extra.push((key, n.clone()));
                        }
                    }
                });
            }
            for (key, getter) in extra {
                let read_in_loop = lp.operators.iter().any(|op| {
                    op.any_node(&mut |n| {
                        matches!(n, Value::Call(g, a) if self.data.def(*g).name() == "OpGetInt"
                            && matches!(a.first().map(Value::unspan), Some(Value::Var(v)) if *v == key.0)
                            && matches!(a.get(1).map(Value::unspan), Some(Value::Int(o)) if i64::from(*o) == key.1))
                    })
                });
                if !read_in_loop {
                    fields.push((key, getter));
                }
            }
        }
        let trace = std::env::var("LOFT_TRACE_RANGED_CALL").is_ok();
        if fields.is_empty() {
            if trace {
                eprintln!(
                    "ranged-call: {} For block {} declines: no hoisted integer field",
                    self.data.def(self.def_nr).name(),
                    bl.scope
                );
            }
            return None;
        }
        let mut seeds = Ranges::default();
        for (key, _) in &fields {
            seeds.fields.insert(*key, (-G, G));
        }
        // The copy's own facts, then only what is sound beside the function's.
        let vars = self.data.def(self.def_nr).variables();
        let nn = self.nn_facts();
        let block_code = Value::Block(Box::new(bl.clone()));
        let copy = range::range_vars(self.data, vars, &block_code, &nn, &self.char_walks, &seeds);
        let code = self.data.def(self.def_nr).code();
        let mut outside: HashSet<u16> = HashSet::new();
        sets_outside(code, std::ptr::from_ref(bl), &mut outside);
        let mut escaped: HashSet<u16> = HashSet::new();
        super::non_sentinel::collect_escapes(self.data, code, &mut escaped);
        let mut merged = (*self.current_ranges()).clone();
        for (v, r) in copy.vars {
            if !outside.contains(&v) && !escaped.contains(&v) {
                merged.vars.entry(v).or_insert(r);
            }
        }
        merged.fields.extend(seeds.fields);
        // Worth it only when some call in the block would take a ranged variant.
        let mut gains = false;
        block_code.walk(&mut |n| {
            if !gains
                && let Value::Call(d, args) = n
                && benefits(self.data, *d, 0)
                && self.args_ranged(*d, args, &merged, &nn)
            {
                gains = true;
            }
        });
        if !gains {
            if trace {
                let ranged: Vec<String> = merged
                    .vars
                    .iter()
                    .map(|(v, r)| format!("{}={r:?}", vars.name(*v)))
                    .collect();
                eprintln!(
                    "ranged-call: {} For block {} declines: no call gains (fields {:?}; vars {})",
                    self.data.def(self.def_nr).name(),
                    bl.scope,
                    fields.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
                    ranged.join(" ")
                );
            }
            return None;
        }
        let mut test: Vec<String> = Vec::new();
        for (_, getter) in &fields {
            let read = self.expr_string(getter).ok()?;
            test.push(format!("({read}).unsigned_abs() <= {G}u64"));
        }
        Some((test.join(" && "), merged))
    }

    /// Are `d`'s integer arguments `args` all proven within `[-L, L]` under `facts` — at
    /// least one of them?
    fn args_ranged(
        &self,
        d: u32,
        args: &[Value],
        facts: &Ranges,
        nn: &std::collections::HashMap<u16, bool>,
    ) -> bool {
        let def = self.data.def(d);
        let attrs = def.attributes();
        if attrs.len() < args.len() {
            return false;
        }
        let mut any = false;
        for (at, arg) in attrs.iter().zip(args) {
            if !is_integer(&at.typedef) {
                continue;
            }
            match range::range(self.data, nn, facts, arg, 0) {
                Some(r) if within(r, L) => any = true,
                _ => return false,
            }
        }
        any
    }

    /// `@FR-R-RangedCall` — may this call of `d` take the ranged variant: the callee gains
    /// from it, every integer argument is proven within `[-L, L]`, and — for a twin call —
    /// every scalar input the twin takes is too.  Records the request; the variant is
    /// emitted after the program's functions.
    pub(super) fn ranged_call(&mut self, d: u32, args: &[Value], twin: bool) -> bool {
        if !range::ranged_calls_enabled()
            || self.range_arith_disabled
            || self.in_coroutine_body
            || (d as usize) >= self.data.definitions.len()
            || !benefits(self.data, d, 0)
        {
            return false;
        }
        let nn = self.nn_facts();
        let facts = self.current_ranges();
        let trace = std::env::var("LOFT_TRACE_RANGED_CALL").is_ok();
        if !self.args_ranged(d, args, &facts, &nn) {
            if trace {
                let ranges: Vec<Option<Range>> = args
                    .iter()
                    .map(|a| range::range(self.data, &nn, &*facts, a, 0))
                    .collect();
                eprintln!(
                    "ranged-call: {} → {} declines: arguments {ranges:?}",
                    self.data.def(self.def_nr).name(),
                    self.data.def(d).name()
                );
            }
            return false;
        }
        if twin {
            let Some(inputs) = self.callee_inputs_of(d) else {
                return false;
            };
            let vars = self.data.def(self.def_nr).variables();
            for (p, fld, _) in &inputs.scalars {
                let Some(arg) = args.get(*p as usize) else {
                    return false;
                };
                let key = match arg.unspan() {
                    Value::Var(c) => (*c, *fld),
                    _ => match super::hoist::path_scalar(self.data, vars, arg, *fld) {
                        Some((k, _)) => k,
                        None => return false,
                    },
                };
                match facts.fields.get(&key) {
                    Some(r) if within(*r, L) => {}
                    _ => {
                        if trace {
                            eprintln!(
                                "ranged-call: {} → {} declines: scalar input {key:?} unranged",
                                self.data.def(self.def_nr).name(),
                                self.data.def(d).name()
                            );
                        }
                        return false;
                    }
                }
            }
        }
        if !self.rg_emitted.contains(&(d, twin)) && !self.rg_requests.contains(&(d, twin)) {
            self.rg_requests.push((d, twin));
        }
        crate::rewrite_census::fired("R-RangedCall", 1);
        true
    }

    /// The facts a ranged variant of `d` runs under: each integer parameter in `[-L, L]`
    /// (`range_vars` drops the seed of one the body assigns), and for a twin each scalar
    /// input field of its parameter.
    pub(super) fn variant_ranges(&mut self, d: u32, twin: bool) -> Ranges {
        let def = self.data.def(d);
        let vars = def.variables();
        let mut seeds = Ranges::default();
        for at in def.attributes() {
            let v = vars.var(&at.name);
            if v != u16::MAX && is_integer(vars.tp(v)) && !at.hidden {
                seeds.vars.insert(v, (-L, L));
            }
        }
        if twin && let Some(inputs) = self.callee_inputs_of(d) {
            for (p, fld, _) in &inputs.scalars {
                seeds.fields.insert((*p, *fld), (-L, L));
            }
        }
        let nn = super::non_sentinel::non_sentinel_vars(self.data, def.code());
        range::range_vars(
            self.data,
            vars,
            def.code(),
            &nn,
            &std::collections::BTreeMap::new(),
            &seeds,
        )
    }
}
