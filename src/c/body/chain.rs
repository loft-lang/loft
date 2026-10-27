// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-GuardedChain` in the C: a counted loop whose body holds integer CHAINS of `+`, `-`,
//! `*` and negation over literals, the loop's own counters, the counters of a counted loop
//! nested in it, integer locals the loop never writes and record scalars an enclosing frame
//! holds runs those chains with C's plain operators when a guard, evaluated once at the
//! loop's entry, proves none can fault — native's `Output::chain_fast_path`, with the same
//! admission (`loop_shape::range_counters`, `hoist::written_vars`,
//! `hoist::nested_counted_loops`, `non_sentinel::collect_escapes`) and the same profitability
//! decline.  The guard's magnitude bounds are spelled with `loft_gc_*`, which clear the guard
//! where a step does not fit.  `LOFT_NO_GUARDED_CHAIN=1` keeps every operator checked;
//! `LOFT_TRACE_CHAIN=1` names each loop admitted.

use super::{E, FnWriter, K, as_int};
use crate::data::{Block, Type, Value};
use std::collections::HashSet;

/// The chain operators and their arity.
fn chain_op(name: &str, arity: usize) -> bool {
    matches!(
        (name, arity),
        (
            "OpAddInt"
                | "OpMinInt"
                | "OpMulInt"
                | "OpAddIntNullable"
                | "OpMinIntNullable"
                | "OpMulIntNullable",
            2
        ) | ("OpMinSingleInt", 1)
    )
}

/// What a guard's bound may name, collected as the chains are bounded.
struct Leaves<'a> {
    own: Vec<u16>,
    nested: Vec<Vec<u16>>,
    written: HashSet<u16>,
    escaped: HashSet<u16>,
    vars: &'a crate::variables::Function,
    invariants: Vec<u16>,
    scalars: Vec<String>,
}

impl FnWriter<'_> {
    /// The C bound of chain node `v` — a literal by its magnitude, a counter by `loft_gc_cb`
    /// (`loft_gc_nc<k>` for a nested loop's), an invariant by `loft_gc_v<k>`, a held record
    /// scalar by `loft_gc_s<k>`, `+`/`-` summing and `*` multiplying — or `None` where a leaf
    /// is none of these.  `counters` false while bounding a nested loop's own seed and end.
    fn chain_bound(&self, v: &Value, l: &mut Leaves, counters: bool) -> Option<String> {
        match v.unspan() {
            Value::Int(k) => Some(format!("INT64_C({})", i64::from(*k).unsigned_abs())),
            Value::Long(n) if *n != i64::MIN && n.unsigned_abs() <= i64::MAX as u64 => {
                Some(format!("INT64_C({})", n.unsigned_abs()))
            }
            Value::Var(x) => {
                if l.own.contains(x) {
                    return counters.then(|| "loft_gc_cb".to_string());
                }
                if let Some(k) = l.nested.iter().position(|c| c.contains(x)) {
                    return counters.then(|| format!("loft_gc_nc{k}"));
                }
                if l.written.contains(x)
                    || l.escaped.contains(x)
                    || !matches!(l.vars.tp(*x).peel_link(), Type::Integer(_))
                {
                    return None;
                }
                let k = l.invariants.iter().position(|i| i == x).unwrap_or_else(|| {
                    l.invariants.push(*x);
                    l.invariants.len() - 1
                });
                Some(format!("loft_gc_v{k}"))
            }
            Value::Call(d, args) => {
                let name = self.cx.data.def(*d).name();
                match (name, args.len()) {
                    ("OpAddInt" | "OpMinInt" | "OpAddIntNullable" | "OpMinIntNullable", 2) => {
                        let a = self.chain_bound(&args[0], l, counters)?;
                        let b = self.chain_bound(&args[1], l, counters)?;
                        Some(format!("loft_gc_add({a}, {b}, &loft_gc_ok)"))
                    }
                    ("OpMulInt" | "OpMulIntNullable", 2) => {
                        let a = self.chain_bound(&args[0], l, counters)?;
                        let b = self.chain_bound(&args[1], l, counters)?;
                        Some(format!("loft_gc_mul({a}, {b}, &loft_gc_ok)"))
                    }
                    ("OpMinSingleInt", 1) => self.chain_bound(&args[0], l, counters),
                    ("OpGetInt", _) => {
                        let key = crate::opt::hoist::scalar_read(name, args)?;
                        let (held, _) = self.held_scalar(&key)?;
                        let k = l
                            .scalars
                            .iter()
                            .position(|s| *s == held)
                            .unwrap_or_else(|| {
                                l.scalars.push(held);
                                l.scalars.len() - 1
                            });
                        Some(format!("loft_gc_s{k}"))
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The guard of counted loop `b`, written before it, with the argument-slice addresses
    /// of its admitted chain operators and whether the loop is INNERMOST (written twice, the
    /// plain copy under the guard) — or `None`: no chain, one admitted operator (the
    /// profitability decline), a nested loop whose seed or end is not a leaf, a body that
    /// must not be copied, or the release-pass probe (its operators are plain already).
    pub(super) fn chain_guard(
        &mut self,
        b: &Block,
        ind: usize,
    ) -> Result<Option<(String, HashSet<usize>, bool)>, String> {
        let probe =
            crate::env_once!(std::env::var("LOFT_RELEASE_PASS_PROBE").is_ok_and(|v| v != "0"));
        if probe
            || self.gen_elem.is_some()
            || crate::env_once!(std::env::var("LOFT_NO_GUARDED_CHAIN").is_ok_and(|v| v != "0"))
            || self.char_walks.contains_key(&b.scope)
        {
            return Ok(None);
        }
        let data = self.cx.data;
        let Ok(rc) = crate::loop_shape::range_counters(b, data) else {
            return Ok(None);
        };
        let def = data.def(self.d);
        let vars = def.variables();
        let mut escaped = HashSet::new();
        crate::opt::non_sentinel::collect_escapes(data, def.code(), &mut escaped);
        let nested = crate::opt::hoist::nested_counted_loops(b, data);
        let mut l = Leaves {
            own: std::iter::once(rc.loop_var)
                .chain(std::iter::once(rc.index))
                .chain(rc.next)
                .collect(),
            nested: nested
                .iter()
                .map(|(n, _)| {
                    std::iter::once(n.loop_var)
                        .chain(std::iter::once(n.index))
                        .chain(n.next)
                        .collect()
                })
                .collect(),
            written: crate::opt::hoist::written_vars(b, data, vars),
            escaped,
            vars,
            invariants: Vec::new(),
            scalars: Vec::new(),
        };
        // The maximal chains: a chain operator whose whole subtree bounds is admitted whole;
        // one that does not is descended for smaller ones.  The iterator (statement 0) steps
        // its counters already; a nested loop guards its own chains; an enclosing guard's
        // operators are plain there already.
        let mut chains: Vec<String> = Vec::new();
        let mut admitted: HashSet<usize> = HashSet::new();
        let enclosing: HashSet<usize> = self
            .plain_chains
            .iter()
            .flat_map(|(s, _)| s.iter().copied())
            .collect();
        let mut stack: Vec<&Value> = b.operators.iter().skip(1).collect();
        while let Some(v) = stack.pop() {
            if matches!(v.unspan(), Value::Loop(_)) {
                continue;
            }
            if let Value::Call(d, args) = v.unspan() {
                if enclosing.contains(&(args.as_ptr() as usize)) {
                    continue;
                }
                if chain_op(data.def(*d).name(), args.len()) {
                    let (inv, sc) = (l.invariants.clone(), l.scalars.clone());
                    if let Some(bound) = self.chain_bound(v, &mut l, true) {
                        chains.push(bound);
                        collect_ops(v, data, &mut admitted);
                        continue;
                    }
                    l.invariants = inv;
                    l.scalars = sc;
                }
            }
            v.for_each_child(&mut |c| stack.push(c));
        }
        // Profitability, as native: an index offset (`kb[oa + t]`, one `+`/`-` of two leaves
        // as an element read's index) does not count, and fewer than two others decline.
        let mut offsets: HashSet<usize> = HashSet::new();
        for op in &b.operators {
            op.walk(&mut |n| {
                if let Value::Call(d, args) = n.unspan()
                    && args.len() == 3
                    && crate::opt::body_writes::is_element_address(data, *d)
                    && let Value::Call(a, aargs) = args[2].unspan()
                    && matches!(
                        (data.def(*a).name(), aargs.len()),
                        (
                            "OpAddInt" | "OpMinInt" | "OpAddIntNullable" | "OpMinIntNullable",
                            2
                        )
                    )
                    && aargs.iter().all(|x| {
                        matches!(x.unspan(), Value::Var(_) | Value::Int(_) | Value::Long(_))
                    })
                {
                    offsets.insert(aargs.as_ptr() as usize);
                }
            });
        }
        if chains.is_empty() || admitted.iter().filter(|a| !offsets.contains(a)).count() < 2 {
            return Ok(None);
        }
        // The nested loops' seeds and ends, bounded over the same leaves (no counters).
        let mut nested_bounds = Vec::new();
        for (n, seed) in &nested {
            let (Some(s), Some(h)) = (
                self.chain_bound(seed, &mut l, false),
                self.chain_bound(n.hi, &mut l, false),
            ) else {
                return Ok(None);
            };
            nested_bounds.push((s, h));
        }
        let lo = match rc.next {
            Some(nx) => self.var(nx),
            None => format!(
                "loft_gc_add({}, INT64_C(1), &loft_gc_ok)",
                self.var(rc.index)
            ),
        };
        let hi = self.expr(rc.hi, ind)?;
        let hi = as_int(hi)?;
        self.flush(ind);
        self.chain_guards += 1;
        let guard = format!("loft_gc{}", self.chain_guards);
        self.decls.push(format!("int {guard} = 0;"));
        self.line(ind, "{");
        self.line(ind + 1, "int loft_gc_ok = 1;");
        self.line(
            ind + 1,
            &format!("int64_t loft_gc_lo = {lo}, loft_gc_hi = {hi};"),
        );
        self.line(
            ind + 1,
            "if (loft_gc_lo == LOFT_NULL || loft_gc_hi == LOFT_NULL) loft_gc_ok = 0;",
        );
        self.line(
            ind + 1,
            "int64_t loft_gc_cb = loft_gc_add(loft_gc_max(loft_gc_abs(loft_gc_lo, &loft_gc_ok), loft_gc_abs(loft_gc_hi, &loft_gc_ok)), INT64_C(1), &loft_gc_ok);",
        );
        for (k, v) in l.invariants.iter().enumerate() {
            let name = self.var(*v);
            self.line(
                ind + 1,
                &format!("int64_t loft_gc_v{k} = loft_gc_abs({name}, &loft_gc_ok);"),
            );
        }
        for (k, s) in l.scalars.iter().enumerate() {
            self.line(
                ind + 1,
                &format!("int64_t loft_gc_s{k} = loft_gc_abs({s}, &loft_gc_ok);"),
            );
        }
        for (k, (s, h)) in nested_bounds.iter().enumerate() {
            self.line(
                ind + 1,
                &format!(
                    "int64_t loft_gc_nc{k} = loft_gc_add(loft_gc_max({s}, {h}), INT64_C(1), &loft_gc_ok);"
                ),
            );
        }
        for c in &chains {
            self.line(ind + 1, &format!("(void) {c};"));
        }
        self.line(ind + 1, "(void) loft_gc_cb;");
        for k in 0..l.invariants.len() {
            self.line(ind + 1, &format!("(void) loft_gc_v{k};"));
        }
        for k in 0..l.scalars.len() {
            self.line(ind + 1, &format!("(void) loft_gc_s{k};"));
        }
        for k in 0..nested_bounds.len() {
            self.line(ind + 1, &format!("(void) loft_gc_nc{k};"));
        }
        self.line(ind + 1, &format!("{guard} = loft_gc_ok;"));
        self.line(ind, "}");
        crate::rewrite_census::fired("R-GuardedChain", 1);
        crate::rewrite_census::fired("R-GuardedChain/operators", admitted.len());
        if crate::env_once!(std::env::var("LOFT_TRACE_CHAIN").is_ok_and(|v| v != "0")) {
            eprintln!(
                "chain: {} loop {} admitted — {} operator(s) in {} chain(s), {} invariant(s), {} held scalar(s), {} nested loop(s)",
                def.name(),
                b.scope,
                admitted.len(),
                chains.len(),
                l.invariants.len(),
                l.scalars.len(),
                nested_bounds.len()
            );
        }
        // A body that must not run twice — a yield, parallel arms, a call through a function
        // value — is written once and branches per operator, as a loop with loops inside is.
        // So is a body that calls a loft function: written twice it doubles what the C
        // compiler inlines there, which cost more than the chains saved (crawler `vis`).
        let once = b.operators.iter().any(|op| {
            op.any_node(&mut |n| match n {
                Value::Yield(_) | Value::Parallel(_) | Value::CallRef(_, _) | Value::Loop(_) => {
                    true
                }
                Value::Call(d, _) => {
                    (*d as usize) < data.definitions.len()
                        && !matches!(data.def(*d).code(), Value::Null)
                }
                _ => false,
            })
        });
        Ok(Some((guard, admitted, !once)))
    }

    /// An admitted chain operator: C's plain operator in a guarded loop's plain copy, or
    /// `guard ? plain : checked` in a loop written once; `None` for any other operator.
    pub(super) fn chain_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if !chain_op(name, args.len()) {
            return Ok(None);
        }
        let at = args.as_ptr() as usize;
        let Some(guard) = self
            .plain_chains
            .iter()
            .rev()
            .find(|(s, _)| s.contains(&at))
            .map(|(_, g)| g.clone())
        else {
            return Ok(None);
        };
        let (plain, checked) = if args.len() == 1 {
            let a = self.one(name, args, ind)?;
            let a = as_int(a)?;
            (format!("(-({a}))"), format!("loft_neg({a})"))
        } else {
            let (a, b) = self.two(name, args, ind)?;
            let (a, b) = (as_int(a)?, as_int(b)?);
            let (op, f) = if name.starts_with("OpAdd") {
                ("+", "loft_add")
            } else if name.starts_with("OpMin") {
                ("-", "loft_sub")
            } else {
                ("*", "loft_mul")
            };
            (format!("(({a}) {op} ({b}))"), format!("{f}({a}, {b})"))
        };
        Ok(Some(E::new(
            match guard {
                None => plain,
                Some(g) => format!("({g} ? {plain} : {checked})"),
            },
            K::Int,
        )))
    }
}

/// Every chain operator under `v`, by its argument slice's address.
fn collect_ops(v: &Value, data: &crate::data::Data, into: &mut HashSet<usize>) {
    if let Value::Call(d, args) = v.unspan()
        && chain_op(data.def(*d).name(), args.len())
    {
        into.insert(args.as_ptr() as usize);
    }
    v.for_each_child(&mut |c| collect_ops(c, data, into));
}
