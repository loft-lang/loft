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
use crate::loop_shape::RangeCounters;
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

/// What a counter's facts say of one argument: its invariant part (`None` for none) and the
/// counter's coefficient, -1, 0 or 1.
type ArgShape<'v> = (Option<&'v Value>, i64);

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
            || b.operators.iter().any(|op| {
                op.any_node(&mut |n| {
                    matches!(n, Value::Loop(_) | Value::Yield(_) | Value::Parallel(_))
                })
            })
        {
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
        let mut sites: Vec<(usize, u32, &[Value])> = Vec::new();
        for op in &b.operators {
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
            if self.cx.version_leaf(d).is_none() || flags.contains_key(&at) {
                continue;
            }
            if let Some(site) = self.version_site(&rc, &rebound, &end, d, args, ind)? {
                flags.insert(at, site);
            }
        }
        Ok((!flags.is_empty()).then_some(VLoop { flags }))
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

    /// Argument `x` over the counters: invariant, the counter, `X + i`, `i + X` or `X - i`.
    fn version_arg<'v>(
        &self,
        rebound: &HashSet<u16>,
        counters: &[u16],
        x: &'v Value,
    ) -> Option<ArgShape<'v>> {
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
        rc: &RangeCounters,
        rebound: &HashSet<u16>,
        end: &str,
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
        let counters: Vec<u16> = [Some(rc.loop_var), Some(rc.index), rc.next]
            .into_iter()
            .flatten()
            .collect();
        let leaves = leaf.leaves();
        let mut shapes: Vec<ArgShape> = Vec::with_capacity(leaves.len());
        let mut varying = HashSet::new();
        for lf in &leaves {
            let shape = match lf {
                Lin::Param(v) => {
                    let Some(i) = pos(*v) else { return Ok(None) };
                    let Some(s) = self.version_arg(rebound, &counters, &args[i]) else {
                        return Ok(self.version_decline(d, "an argument not linear in the counter"));
                    };
                    if s.1 != 0 {
                        varying.insert(*v);
                    }
                    s
                }
                _ => (None, 0),
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
        let first = match rc.next {
            Some(n) => format!("(({flag}_f = {}), 1)", self.var(n)),
            None => format!("loft_vg_lin({}, 1, 1, &{flag}_f)", self.var(rc.index)),
        };
        let mut guard = vec![
            first,
            format!(
                "loft_vg_span({flag}_f, {end}, {}, {MIN_TRIPS}, &{flag}_l)",
                i32::from(rc.inclusive)
            ),
        ];
        let pre = self.cx.version_pre(d, &leaf);
        for (side, at) in [("a", format!("{flag}_f")), ("b", format!("{flag}_l"))] {
            let mut vals = Vec::with_capacity(leaves.len());
            for (k, part) in parts.iter().enumerate() {
                let c = shapes[k].1;
                if c == 0 {
                    vals.push(part.clone());
                } else {
                    let v = format!("{flag}_{side}{k}");
                    self.decls.push(format!("int64_t {v} = 0;"));
                    guard.push(format!("loft_vg_lin({part}, INT64_C({c}), {at}, &{v})"));
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

    /// The call site: the flag its loop's prelude decided, when its held inputs are the ones
    /// the prelude read.
    pub(super) fn version_flag(&self, args: &[Value], inputs: &[String]) -> Option<String> {
        let Some(Some(l)) = self.versions.last() else {
            return None;
        };
        let (flag, read) = l.flags.get(&(args.as_ptr() as usize))?;
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
        _ => format!("loft_vh_at_int({held}, {size}, {index}, {fld})"),
    }
}

/// A C operand that names a value without computing it.
fn is_simple(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}
