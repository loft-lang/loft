// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-Version` in the C: a held-inputs twin's call in an innermost counted loop takes the
//! leaf's FAST twin (`f__inv__vf`, [`super::super::VF_BIT`]) where the leaf's precondition
//! holds over the whole range of the loop's counter, and the twin as written elsewhere.
//!
//! - THE LOOP'S PRELUDE decides a flag `vg<k>` per call site of a leaf: the precondition
//!   `f__vp` asked at the arguments the counter's first value gives them and at those its last
//!   value gives them.  An argument the precondition reads is invariant, `X + i`, `i + X` or
//!   `X - i` over the counter `i` with `X` invariant: `X` is evaluated here, and the argument
//!   at a counter value is `X ± i`, refused where it does not fit (`loft_vg_lin`).  Decided
//!   before the loop, the flag is invariant in it, and both compilers unswitch on it.
//! - THE CALL is `vg<k> ? f__inv__vf(…) : f__inv(…)`, when its held inputs are the ones the
//!   prelude read.
//! - THE PRECONDITION `f__vp`: the leaf's tests over its parameters, inputs and header
//!   lengths, every operation through `loft_vg_*` (a null operand or an overflow refuses).
//! - THE FAST TWIN: the leaf written with each decided `if` its arm, each proven element
//!   access through `loft_vh_at_int`/`loft_vh_put_int`, each tested operation plain.

use super::{FnWriter, as_int};
use crate::c::{CType, Cx, c_name};
use crate::data::{Block, Value};
use crate::opt::version::{Leaf, Lin};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::rc::Rc;

/// One innermost counted loop's versioned call sites: the flag of each, by its argument
/// slice, and the held inputs its precondition read.
pub(crate) struct VLoop {
    flags: HashMap<usize, (String, Vec<String>)>,
}

/// The counter values a loop must have left for its guard to be asked: the precondition is
/// asked twice per entry, which a shorter loop does not repay — drawing's `fill_gradient`
/// rows measured +3-4 % (gcc) versioned without a floor, unchanged at eight.
const MIN_TRIPS: i64 = 8;

/// What a counter's facts say of one argument: its invariant part (`None` for none), the
/// counter's coefficient, -1, 0 or 1, and which loop of the box that counter belongs to.
type ArgShape<'v> = (Option<&'v Value>, i64, usize);

/// One counted loop of the box a site's precondition is asked over: its counters, where its
/// first value comes from, its end and whether that end is inclusive.
struct Level<'v> {
    counters: Vec<u16>,
    first: First<'v>,
    /// The end as C, or (`end_value`) the expression a nested loop's end is set from.
    end: String,
    end_value: Option<&'v Value>,
    inclusive: bool,
}

/// A loop's first counter value, as its prelude can read it: the index-form counter's seed
/// (its value at the prelude) plus one, the two-counter form's `next`, or — for a loop nested
/// in the one whose prelude asks — the seed expression the enclosing body sets it from (`+ 1`
/// for the index form).
enum First<'v> {
    Index(u16),
    Next(u16),
    Seed(&'v Value, bool),
}

impl Cx<'_> {
    /// `d`'s leaf plan, as its held-inputs twin's inputs name them; memoised.
    pub(crate) fn version_leaf(&self, d: u32) -> Option<Rc<Leaf>> {
        if let Some(l) = self.version_leaves.borrow().get(&d) {
            return l.clone();
        }
        let l = self.compute_leaf(d).map(Rc::new);
        self.version_leaves.borrow_mut().insert(d, l.clone());
        l
    }

    fn compute_leaf(&self, d: u32) -> Option<Leaf> {
        if !crate::opt::version::enabled() || (d as usize) >= self.data.definitions.len() {
            return None;
        }
        let fi = self.callee_inputs(d)?;
        let args = self.data.def(d).variables().arguments();
        let var = |p: u16| args.get(p as usize).copied();
        let scalars: Vec<(u16, i64)> = fi
            .scalars
            .iter()
            .filter(|(_, _, g)| super::super::twin_scalar_shape(self, g) == Some(CType::Int))
            .filter_map(|(p, f, _)| Some((var(*p)?, *f)))
            .collect();
        let headers: Vec<(u16, Vec<i64>)> = fi
            .headers
            .iter()
            .filter_map(|(p, o, _)| Some((var(*p)?, o.clone())))
            .collect();
        crate::opt::version::leaf(
            self.data,
            d,
            &crate::opt::version::Inputs {
                scalars: &scalars,
                headers: &headers,
            },
        )
    }

    /// The precondition function of leaf `d`: written once into the unit, named here.
    pub(crate) fn version_pre(&self, d: u32, leaf: &Leaf) -> String {
        let name = format!("{}__vp", c_name(self.data, d));
        if !self.version_pres.borrow_mut().insert(d) {
            return name;
        }
        let leaves = leaf.leaves();
        let params: Vec<String> = (0..leaves.len()).map(|i| format!("int64_t a{i}")).collect();
        let mut out = String::new();
        let _ = writeln!(
            out,
            "\n/* @FR-R-Version: {}'s precondition. */\nstatic inline int {name}({}) {{\n    int ok = 1;",
            self.data.def(d).original_name(),
            if params.is_empty() {
                "void".to_string()
            } else {
                params.join(", ")
            }
        );
        for t in &leaf.tests {
            let op = if t.strict { "<" } else { "<=" };
            let _ = writeln!(
                out,
                "    {{ int64_t l = {}, h = {}; if (!ok || !(l {op} h)) return 0; }}",
                pre_expr(&t.lo, &leaves),
                pre_expr(&t.hi, &leaves)
            );
        }
        out.push_str("    return ok;\n}\n");
        self.gen_decls.borrow_mut().push_str(&out);
        name
    }
}

/// A linear expression in the precondition: every operation through `loft_vg_*`.
fn pre_expr(l: &Lin, leaves: &[Lin]) -> String {
    match l {
        Lin::Const(k) => format!("INT64_C({k})"),
        Lin::Param(_) | Lin::Input(..) | Lin::Len(..) => {
            let i = leaves.iter().position(|x| x == l).unwrap_or(0);
            format!("a{i}")
        }
        Lin::Add(a, b, _) => format!(
            "loft_vg_add({}, {}, &ok)",
            pre_expr(a, leaves),
            pre_expr(b, leaves)
        ),
        Lin::Sub(a, b, _) => format!(
            "loft_vg_sub({}, {}, &ok)",
            pre_expr(a, leaves),
            pre_expr(b, leaves)
        ),
        Lin::Mul(a, b, _) => format!(
            "loft_vg_mul({}, {}, &ok)",
            pre_expr(a, leaves),
            pre_expr(b, leaves)
        ),
        Lin::Neg(a, _) => format!("loft_vg_neg({}, &ok)", pre_expr(a, leaves)),
    }
}

impl FnWriter<'_> {
    /// A fast twin's decided `if`: the arm it takes, by its condition.
    pub(super) fn version_decided(&self, cond: &Value) -> Option<bool> {
        self.vf
            .as_ref()?
            .decided
            .get(&(std::ptr::from_ref(cond) as usize))
            .copied()
    }

    /// A fast twin's proven element access, by its argument slice.
    pub(super) fn version_raw(&self, args: &[Value]) -> bool {
        self.vf
            .as_ref()
            .is_some_and(|l| l.raw.contains(&(args.as_ptr() as usize)))
    }

    /// The loop prelude: for an innermost counted loop, the flag of each call of a leaf in its
    /// body, decided here.  Every loop pushes a frame; only such a loop pushes sites.
    pub(super) fn hold_versions(&mut self, b: &Block, ind: usize) -> Result<(), String> {
        let facts = self.version_loop(b, ind)?;
        self.versions.push(facts);
        Ok(())
    }

    fn version_loop(&mut self, b: &Block, ind: usize) -> Result<Option<VLoop>, String> {
        if !crate::opt::version::enabled()
            || self.gen_elem.is_some()
            || self.lean()
            || self.vf.is_some()
            || b.operators
                .iter()
                .any(|op| op.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_))))
        {
            return Ok(None);
        }
        let inner = inner_loop(b);
        if inner.is_err() {
            return Ok(None);
        }
        let Ok(rc) = crate::loop_shape::range_counters(b, self.cx.data) else {
            return Ok(None);
        };
        let mut rebound = crate::opt::body_writes::rebound_vars(b, self.cx.data, self.vars);
        for op in &b.operators {
            crate::opt::non_sentinel::collect_escapes(self.cx.data, op, &mut rebound);
        }
        // The counters step only in their iterator.
        let counters: Vec<u16> = [Some(rc.loop_var), Some(rc.index), rc.next]
            .into_iter()
            .flatten()
            .collect();
        let mut steps = 0usize;
        for op in &b.operators {
            op.walk(&mut |n| {
                if let Value::Set(x, _) = n
                    && counters.contains(x)
                {
                    steps += 1;
                }
            });
        }
        if steps > counters.len() {
            return Ok(None);
        }
        let end = match rc.hi.unspan() {
            Value::Var(x) if !rebound.contains(x) => self.var(*x),
            Value::Int(k) => format!("INT64_C({k})"),
            Value::Long(k) if *k != i64::MIN => format!("INT64_C({k})"),
            _ => return Ok(None),
        };
        let mut levels = vec![Level {
            counters,
            first: match rc.next {
                Some(n) => First::Next(n),
                None => First::Index(rc.index),
            },
            end,
            end_value: None,
            inclusive: rc.inclusive,
        }];
        // The NEST clause: a loop holding one innermost counted loop asks the precondition of
        // that loop's calls over both counters at once, its seed and end invariant here.
        let body: &[Value] = match inner {
            Ok(None) => &b.operators,
            Ok(Some(lp)) => match self.version_level(b, lp, &rebound, &levels[0].counters) {
                Some(level) => {
                    levels.push(level);
                    &lp.operators
                }
                None => return Ok(None),
            },
            Err(()) => return Ok(None),
        };
        let mut sites: Vec<(usize, u32, &[Value])> = Vec::new();
        for op in body {
            op.walk(&mut |n| {
                if let Value::Call(d, args) = n
                    && !super::super::is_operator(self.cx.data, *d)
                {
                    sites.push((args.as_ptr() as usize, *d, args.as_slice()));
                }
            });
        }
        let mut flags = HashMap::new();
        for (at, d, args) in sites {
            // A site an enclosing nest decided keeps that flag: asked again per entry of this
            // loop as a fallback where the box failed, it cost draw_quads 25 % where it never
            // ran (the row loop grows by the inlined preconditions).
            if self.cx.version_leaf(d).is_none()
                || flags.contains_key(&at)
                || self.version_site_flag(at).is_some()
            {
                continue;
            }
            if let Some(site) = self.version_site(&levels, &rebound, d, args, ind)? {
                flags.insert(at, site);
            }
        }
        Ok((!flags.is_empty()).then_some(VLoop { flags }))
    }

    /// The inner level of a nest: loop `lp` inside `b`, counted, its counters stepped only by
    /// its iterator, its seed and end set once in `b` from values invariant over `b`.
    fn version_level<'v>(
        &self,
        b: &'v Block,
        lp: &'v Block,
        rebound: &HashSet<u16>,
        outer: &[u16],
    ) -> Option<Level<'v>> {
        let data = self.cx.data;
        let (rc, seed) = crate::opt::hoist::nested_counted_loops(b, data)
            .into_iter()
            .find(|(rc, _)| {
                crate::loop_shape::range_counters(lp, data)
                    .is_ok_and(|own| own.index == rc.index && own.loop_var == rc.loop_var)
            })?;
        let counters: Vec<u16> = [Some(rc.loop_var), Some(rc.index), rc.next]
            .into_iter()
            .flatten()
            .collect();
        let mut steps = 0usize;
        for op in &lp.operators {
            op.walk(&mut |n| {
                if let Value::Set(x, _) = n
                    && counters.contains(x)
                {
                    steps += 1;
                }
            });
        }
        let all: Vec<u16> = outer.iter().chain(&counters).copied().collect();
        if steps > counters.len()
            || !self.version_invariant(rebound, &all, seed)
            || !self.version_invariant(rebound, &all, rc.hi)
        {
            return None;
        }
        Some(Level {
            counters,
            first: First::Seed(seed, rc.next.is_none()),
            end: String::new(),
            end_value: Some(rc.hi),
            inclusive: rc.inclusive,
        })
    }

    /// The flag an enclosing loop's prelude decided for the call whose argument slice is at
    /// `at`, with the held inputs it read.
    fn version_site_flag(&self, at: usize) -> Option<&(String, Vec<String>)> {
        self.versions
            .iter()
            .rev()
            .flatten()
            .find_map(|l| l.flags.get(&at))
    }

    /// Is `x` invariant over the loop: literals, variables it neither assigns nor lets escape,
    /// the integer operators over those, and record scalars an enclosing frame holds?
    fn version_invariant(&self, rebound: &HashSet<u16>, counters: &[u16], x: &Value) -> bool {
        match x.unspan() {
            Value::Int(_) | Value::Long(_) => true,
            Value::Var(v) => !rebound.contains(v) && !counters.contains(v),
            Value::Call(d, a) => {
                let name = self.cx.data.def(*d).name();
                match (name, a.len()) {
                    (
                        "OpAddInt" | "OpAddIntNullable" | "OpMinInt" | "OpMinIntNullable"
                        | "OpMulInt" | "OpMulIntNullable",
                        2,
                    )
                    | ("OpMinSingleInt", 1) => a
                        .iter()
                        .all(|y| self.version_invariant(rebound, counters, y)),
                    _ => crate::opt::hoist::scalar_read(name, a)
                        .is_some_and(|key| self.held_scalar(&key).is_some()),
                }
            }
            _ => false,
        }
    }

    /// Argument `x` over the box's loops: invariant over all of them, or `X ± i` over the
    /// counters of one, `X` invariant over all — answered with that loop's position.
    fn version_arg_levels<'v>(
        &self,
        rebound: &HashSet<u16>,
        levels: &[Level],
        all: &[u16],
        x: &'v Value,
    ) -> Option<ArgShape<'v>> {
        if self.version_invariant(rebound, all, x) {
            return Some((Some(x), 0, 0));
        }
        levels.iter().enumerate().find_map(|(m, lv)| {
            let (inv, c) = self.version_arg(rebound, &lv.counters, x)?;
            (c != 0 && inv.is_none_or(|y| self.version_invariant(rebound, all, y)))
                .then_some((inv, c, m))
        })
    }

    /// Argument `x` over the counters: invariant, the counter, `X + i`, `i + X` or `X - i`.
    fn version_arg<'v>(
        &self,
        rebound: &HashSet<u16>,
        counters: &[u16],
        x: &'v Value,
    ) -> Option<(Option<&'v Value>, i64)> {
        let counter = |y: &Value| matches!(y.unspan(), Value::Var(v) if counters.contains(v));
        if counter(x) {
            return Some((None, 1));
        }
        if self.version_invariant(rebound, counters, x) {
            return Some((Some(x), 0));
        }
        let Value::Call(d, a) = x.unspan() else {
            return None;
        };
        if a.len() != 2 {
            return None;
        }
        let inv = |y: &Value| self.version_invariant(rebound, counters, y);
        match self.cx.data.def(*d).name() {
            "OpAddInt" | "OpAddIntNullable" if counter(&a[1]) && inv(&a[0]) => {
                Some((Some(&a[0]), 1))
            }
            "OpAddInt" | "OpAddIntNullable" if counter(&a[0]) && inv(&a[1]) => {
                Some((Some(&a[1]), 1))
            }
            "OpMinInt" | "OpMinIntNullable" if counter(&a[1]) && inv(&a[0]) => {
                Some((Some(&a[0]), -1))
            }
            _ => None,
        }
    }

    /// One call site, at the loop's prelude: its flag decided, and the held inputs its
    /// precondition read; `None` where it is not versioned.
    #[expect(
        clippy::too_many_lines,
        reason = "one guard: the inputs, the arguments, the two ends"
    )]
    fn version_site(
        &mut self,
        levels: &[Level],
        rebound: &HashSet<u16>,
        d: u32,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<(String, Vec<String>)>, String> {
        let Some(leaf) = self.cx.version_leaf(d) else {
            return Ok(None);
        };
        let Some(fi) = self.cx.callee_inputs(d) else {
            return Ok(None);
        };
        if args.len() >= 64 {
            return Ok(None);
        }
        let def_args = self.cx.data.def(d).variables().arguments();
        if def_args.len() != args.len() {
            return Ok(None);
        }
        let pos = |v: u16| def_args.iter().position(|a| *a == v);
        // Every scalar input must be a held scalar here — invariant over the loop — before
        // `twin_inputs` is asked for the names.
        let scalar_keys: Vec<(u16, i64)> = fi
            .scalars
            .iter()
            .filter(|(_, _, g)| super::super::twin_scalar_shape(self.cx, g).is_some())
            .map(|(p, f, _)| (*p, *f))
            .collect();
        for (p, f) in &scalar_keys {
            let Some(arg) = args.get(*p as usize) else {
                return Ok(None);
            };
            let key = match arg.unspan() {
                Value::Var(av) => (*av, *f),
                a => match crate::opt::hoist::path_scalar(self.cx.data, self.vars, a, *f) {
                    Some((key, _)) => key,
                    None => return Ok(None),
                },
            };
            if self.held_scalar(&key).is_none() {
                return Ok(self.version_decline(d, "an input read through its record"));
            }
        }
        let Some(inputs) = self.twin_inputs(d, args, ind)? else {
            return Ok(None);
        };
        if inputs.len() != scalar_keys.len() + fi.headers.len() {
            return Ok(None);
        }
        let counters: Vec<u16> = levels
            .iter()
            .flat_map(|l| l.counters.iter().copied())
            .collect();
        let leaves = leaf.leaves();
        let mut shapes: Vec<ArgShape> = Vec::with_capacity(leaves.len());
        let mut varying = HashSet::new();
        for lf in &leaves {
            let shape = match lf {
                Lin::Param(v) => {
                    let Some(i) = pos(*v) else { return Ok(None) };
                    let Some(s) = self.version_arg_levels(rebound, levels, &counters, &args[i])
                    else {
                        return Ok(self.version_decline(d, "an argument not linear in the counter"));
                    };
                    if s.1 != 0 {
                        varying.insert(*v);
                    }
                    s
                }
                _ => (None, 0, 0),
            };
            shapes.push(shape);
        }
        if varying.is_empty() {
            return Ok(self.version_decline(d, "no argument moves with the counter"));
        }
        if !leaf
            .tests
            .iter()
            .all(|t| t.lo.linear(&varying) && t.hi.linear(&varying))
        {
            return Ok(self.version_decline(d, "a product of two terms the counter moves"));
        }
        let flag = format!("vg{}", self.decls.len() + 1);
        self.decls.push(format!(
            "int {flag} = 0; int64_t {flag}_f = 0, {flag}_l = 0;"
        ));
        // The leaves' values at the prelude: an argument's invariant part, an input, a length.
        let mut parts: Vec<String> = Vec::with_capacity(leaves.len());
        for (k, lf) in leaves.iter().enumerate() {
            parts.push(match lf {
                Lin::Param(_) => match shapes[k].0 {
                    Some(x) => {
                        let e = as_int(self.expr(x, ind)?)?;
                        if is_simple(&e) {
                            e
                        } else {
                            let t = self.temp(CType::Int);
                            self.line(ind, &format!("{t} = {e};"));
                            t
                        }
                    }
                    None => "INT64_C(0)".to_string(),
                },
                Lin::Input(var, fld) => {
                    let at = pos(*var).ok_or("a leaf input of no parameter")?;
                    let which = scalar_keys
                        .iter()
                        .position(|(q, g)| *q as usize == at && g == fld)
                        .ok_or("a leaf input the twin does not take")?;
                    inputs[which].clone()
                }
                Lin::Len(var, offs) => {
                    let at = pos(*var).ok_or("a leaf length of no parameter")?;
                    let which = fi
                        .headers
                        .iter()
                        .position(|(q, o, _)| *q as usize == at && o == offs)
                        .ok_or("a leaf length the twin does not take")?;
                    format!("{}.len", inputs[scalar_keys.len() + which])
                }
                _ => return Err("a precondition leaf that is no parameter, input or length".into()),
            });
        }
        // Each loop's first and last counter values: the outer loop's as its prelude reads
        // them (`{flag}_f`, `{flag}_l`), a nested loop's from the seed and end its enclosing
        // body sets it from (`{flag}_f<m>`, `{flag}_l<m>`).
        let mut guard: Vec<String> = Vec::new();
        let mut ends: Vec<(String, String)> = Vec::with_capacity(levels.len());
        for (lvl, lv) in levels.iter().enumerate() {
            let (first, last) = if lvl == 0 {
                (format!("{flag}_f"), format!("{flag}_l"))
            } else {
                let (first, last) = (format!("{flag}_f{lvl}"), format!("{flag}_l{lvl}"));
                self.decls.push(format!("int64_t {first} = 0, {last} = 0;"));
                (first, last)
            };
            guard.push(match &lv.first {
                First::Index(ix) => format!("loft_vg_lin({}, 1, 1, &{first})", self.var(*ix)),
                First::Next(nx) => format!("(({first} = {}), 1)", self.var(*nx)),
                First::Seed(x, index_form) => {
                    let seed = as_int(self.expr(x, ind)?)?;
                    if *index_form {
                        format!("loft_vg_lin({seed}, 1, 1, &{first})")
                    } else {
                        format!("(({first} = {seed}) != LOFT_NULL)")
                    }
                }
            });
            let end = match lv.end_value {
                Some(x) => as_int(self.expr(x, ind)?)?,
                None => lv.end.clone(),
            };
            let min = if levels.len() == 1 { MIN_TRIPS } else { 1 };
            guard.push(format!(
                "loft_vg_span({first}, {end}, {}, {min}, &{last})",
                i32::from(lv.inclusive)
            ));
            ends.push((first, last));
        }
        if let [(f0, l0), (f1, l1)] = ends.as_slice() {
            guard.push(format!("loft_vg_area({f0}, {l0}, {f1}, {l1}, {MIN_TRIPS})"));
            crate::rewrite_census::fired("R-Version/nest", 1);
        }
        // The corners of the box over the loops some argument moves with: a linear value takes
        // its extremes at them.  One loop: its two ends (`a`, `b`); two: four corners.
        let used: Vec<usize> = (0..levels.len())
            .filter(|lvl| shapes.iter().any(|sh| sh.1 != 0 && sh.2 == *lvl))
            .collect();
        let corners: Vec<(String, Vec<bool>)> = match used.len() {
            1 => vec![("a".into(), vec![false]), ("b".into(), vec![true])],
            _ => vec![
                ("a".into(), vec![false, false]),
                ("b".into(), vec![true, true]),
                ("c".into(), vec![false, true]),
                ("d".into(), vec![true, false]),
            ],
        };
        let pre = self.cx.version_pre(d, &leaf);
        for (side, at) in &corners {
            let mut vals = Vec::with_capacity(leaves.len());
            for (k, part) in parts.iter().enumerate() {
                let (_, coef, lvl) = shapes[k];
                if coef == 0 {
                    vals.push(part.clone());
                } else {
                    let slot = used
                        .iter()
                        .position(|u| *u == lvl)
                        .ok_or("an unused level")?;
                    let (first, last) = &ends[lvl];
                    let t = if at[slot] { last } else { first };
                    let v = format!("{flag}_{side}{k}");
                    self.decls.push(format!("int64_t {v} = 0;"));
                    guard.push(format!("loft_vg_lin({part}, INT64_C({coef}), {t}, &{v})"));
                    vals.push(v);
                }
            }
            guard.push(format!("{pre}({})", vals.join(", ")));
        }
        self.line(ind, &format!("{flag} = {};", guard.join("\n        && ")));
        crate::rewrite_census::fired("R-Version", 1);
        if crate::opt::version::trace() {
            eprintln!(
                "[version] {} calls {} versioned",
                self.cx.data.def(self.d).original_name(),
                self.cx.data.def(d).original_name()
            );
        }
        Ok(Some((flag, inputs)))
    }

    /// `@FR-R-GuardedChain`'s version clause: the flags of loop `b` when every call of a loft
    /// function in it is a site its prelude decided — the only calls a plain copy would write a
    /// second time are then fast twins the written loop calls already.  `None` otherwise.
    pub(super) fn version_unswitch(&self, b: &Block) -> Option<Vec<String>> {
        let data = self.cx.data;
        let mut flags: Vec<String> = Vec::new();
        let mut all = true;
        for op in &b.operators {
            op.walk(&mut |n| match n {
                Value::Call(d, args)
                    if (*d as usize) < data.definitions.len()
                        && !matches!(data.def(*d).code(), Value::Null) =>
                {
                    match self.version_site_flag(args.as_ptr() as usize) {
                        Some((flag, _)) if !flags.contains(flag) => flags.push(flag.clone()),
                        Some(_) => {}
                        None => all = false,
                    }
                }
                Value::CallRef(_, _) | Value::Yield(_) | Value::Parallel(_) | Value::Loop(_) => {
                    all = false;
                }
                _ => {}
            });
        }
        (all && !flags.is_empty()).then_some(flags)
    }

    /// The call site: the flag its loop's prelude decided, when its held inputs are the ones
    /// the prelude read.
    pub(super) fn version_flag(&self, args: &[Value], inputs: &[String]) -> Option<String> {
        let (flag, read) = self.version_site_flag(args.as_ptr() as usize)?;
        (read.as_slice() == inputs).then(|| flag.clone())
    }

    fn version_decline(&self, d: u32, why: &str) -> Option<(String, Vec<String>)> {
        if crate::opt::version::trace() {
            eprintln!(
                "[version] {} call of {} declined: {why}",
                self.cx.data.def(self.d).original_name(),
                self.cx.data.def(d).original_name()
            );
        }
        None
    }
}

/// A proven element read or write in a fast twin, through the held header `held`.
pub(super) fn version_access(
    name: &str,
    held: &str,
    size: i64,
    fld: i64,
    index: &str,
    value: Option<&str>,
) -> String {
    match (name, value) {
        ("OpSetInt", Some(v)) => {
            format!("loft_vh_put_int({held}, {size}, {index}, {fld}, {v})")
        }
        ("OpSetFloat", Some(v)) => {
            format!("loft_vh_put_float({held}, {size}, {index}, {fld}, {v})")
        }
        ("OpSetBoolean", Some(v)) => {
            format!("loft_vh_put_bool({held}, {size}, {index}, {fld}, {v})")
        }
        ("OpGetFloat", None) => format!("loft_vh_at_float({held}, {size}, {index}, {fld})"),
        ("OpGetBoolean", None) => {
            format!("loft_pa_bool(loft_vh_at_addr({held}, {size}, {index}), {fld})")
        }
        _ => format!("loft_vh_at_int({held}, {size}, {index}, {fld})"),
    }
}

/// A C operand that names a value without computing it.
/// The one loop directly inside `b` (no loop between), when it holds no loop itself:
/// `Ok(None)` for a body with no loop, `Err` for two loops side by side or a deeper nest.
fn inner_loop(b: &Block) -> Result<Option<&Block>, ()> {
    fn direct<'a>(v: &'a Value, out: &mut Vec<&'a Block>) {
        if let Value::Loop(lp) = v {
            out.push(lp);
        } else {
            v.for_each_child(&mut |c| direct(c, out));
        }
    }
    let mut found: Vec<&Block> = Vec::new();
    for op in &b.operators {
        direct(op, &mut found);
    }
    match found.as_slice() {
        [] => Ok(None),
        [lp] if !lp
            .operators
            .iter()
            .any(|op| op.any_node(&mut |n| matches!(n, Value::Loop(_)))) =>
        {
            Ok(Some(lp))
        }
        _ => Err(()),
    }
}

fn is_simple(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
