// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-FaultFlag` — a `for` statement whose only effect is on scalar locals runs a FAST
//! copy first, in which every node that can fault (a `??` / `?` discharge, a template's
//! recoverable raise, a checked integer operator) sets a flag instead of branching, raising or
//! reporting.  When the flag is set the locals are restored and the statement runs again as
//! written, so a fault is answered and reported exactly once, by the exact statement.  Inside
//! the fast copy a constant range of at most 64 rounds is unrolled: with no branch left in its
//! body, LLVM can overlap one call's work with the next call's.
//!
//! The admission walk is the rule's soundness: a node it does not name is declined, because a
//! node whose effect it has not proven local could be observed before a rerun.

use super::{Output, sanitize};
use crate::data::{Block, Data, Type, Value};
use std::collections::HashSet;
use std::io::Write;

/// The longest constant range the fast copy unrolls.
const UNROLL_MAX: i64 = 64;
/// The largest body, in IR nodes, the fast copy repeats per round.
const UNROLL_NODES: usize = 48;

/// Is `if test { then } else { … }` the lowering of `??` / `?` over a scalar — the null test
/// of a local, answering that local when it is not null?  Its fast form is the flag and the
/// local, which equals the exact answer whenever the flag stays clear.
pub(super) fn is_discharge(data: &Data, test: &Value, then: &Value) -> bool {
    matches!(
        (test.unspan(), then.unspan()),
        (Value::Call(d, a), Value::Var(y))
            if a.len() == 1
                && data.def(*d).name().starts_with("OpConvBoolFrom")
                && matches!(a[0].unspan(), Value::Var(x) if x == y)
    )
}

/// The `ops::ff_*` helper an integer operator takes in a fast copy, or `None` for an operator
/// that keeps its ordinary emission.
pub(super) fn int_form(name: &str, arity: usize) -> Option<&'static str> {
    match (name, arity) {
        ("OpAddInt" | "OpAddIntNullable", 2) => Some("ff_add"),
        ("OpMinInt" | "OpMinIntNullable", 2) => Some("ff_sub"),
        ("OpMulInt" | "OpMulIntNullable", 2) => Some("ff_mul"),
        ("OpMinSingleInt", 1) => Some("ff_neg"),
        ("OpLandInt", 2) => Some("&"),
        ("OpLorInt", 2) => Some("|"),
        ("OpEorInt", 2) => Some("^"),
        _ => None,
    }
}

/// Does an operator template touch anything but its operands once its recoverable raise is
/// taken out?  A `s.` method, the `stores` or `cell` handles, or a `crate::` path all reach
/// state a rerun could observe.
fn touches_state(template: &str) -> bool {
    let t = template
        .replace("s.raise_recoverable(", "")
        .replace("crate::runtime_error::", "");
    if t.contains("stores") || t.contains("cell") || t.contains("crate::") {
        return true;
    }
    let b = t.as_bytes();
    t.match_indices("s.")
        .any(|(at, _)| at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_'))
}

/// The admission walk over one `for` statement.
struct Walk<'a> {
    data: &'a Data,
    writes: HashSet<u16>,
    fault_forms: usize,
}

impl Walk<'_> {
    /// `depth` counts the loops of the statement around `v`, so a `break` / `continue` whose
    /// level reaches past them leaves the statement and declines.
    fn node(&mut self, v: &Value, depth: u16) -> Result<(), String> {
        match v.unspan() {
            Value::Null
            | Value::Line(_)
            | Value::Int(_)
            | Value::Long(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Boolean(_)
            | Value::Enum(..)
            | Value::Var(_) => Ok(()),
            Value::Set(x, e) => {
                self.writes.insert(*x);
                self.node(e, depth)
            }
            Value::Drop(e) => self.node(e, depth),
            Value::Block(b) => self.all(&b.operators, depth),
            Value::Insert(ops) => self.all(ops, depth),
            Value::Loop(b) => self.all(&b.operators, depth + 1),
            Value::Break(n) | Value::Continue(n) => {
                if *n < depth {
                    Ok(())
                } else {
                    Err("a `break` or `continue` leaves the statement".to_string())
                }
            }
            Value::If(t, a, b) => {
                if is_discharge(self.data, t, a) {
                    self.fault_forms += 1;
                }
                self.node(t, depth)?;
                self.node(a, depth)?;
                self.node(b, depth)
            }
            Value::Call(d, args) => {
                let def = self.data.def(*d);
                let name = def.name();
                if def.rust.is_empty() {
                    return Err(format!("`{name}` is a call, not an operator template"));
                }
                if touches_state(&def.rust) {
                    return Err(format!("`{name}` touches state outside its operands"));
                }
                if def.rust.contains("s.raise_recoverable(") || int_form(name, args.len()).is_some()
                {
                    self.fault_forms += 1;
                }
                self.all(args, depth)
            }
            Value::Return(_) => Err("a `return` leaves the statement".to_string()),
            Value::Text(_) => Err("a text is not a scalar".to_string()),
            _ => Err("a node whose effect is not local to the statement".to_string()),
        }
    }

    fn all(&mut self, ops: &[Value], depth: u16) -> Result<(), String> {
        ops.iter().try_for_each(|o| self.node(o, depth))
    }
}

/// A scalar a fast copy may write and a rerun restore by copy.
fn is_scalar(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Integer(_) | Type::Float | Type::Single | Type::Boolean | Type::Character
    )
}

/// The constant range a fast copy unrolls: the statement's seed position, the counter and the
/// loop variable, the first value and the round count, and the body after the iterator.
pub(super) struct Unroll<'a> {
    seed_at: usize,
    index: u16,
    loop_var: u16,
    first: i64,
    rounds: i64,
    body: &'a [Value],
}

/// Is the `for` statement `bl` a constant range of 1 to 64 rounds over a straight-line body —
/// the seed a literal, the end a literal, the single-counter form, exclusive?  The rounds then
/// are a compile-time fact, and the body has nothing (`break`, `continue`, `return`, a loop)
/// that would make one round differ from the next in shape.
pub(super) fn unroll_shape<'a>(
    data: &Data,
    bl: &'a Block,
    fn_vars: &crate::variables::Function,
) -> Option<Unroll<'a>> {
    let ops: Vec<(usize, &Value)> = bl
        .operators
        .iter()
        .enumerate()
        .filter(|(_, o)| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let [.., (seed_at, seed), (_, lp)] = ops.as_slice() else {
        return None;
    };
    let (Value::Set(index, seed_v), Value::Loop(lp)) = (seed.unspan(), lp.unspan()) else {
        return None;
    };
    let seed_k = match seed_v.unspan() {
        Value::Int(k) => i64::from(*k),
        Value::Long(k) => *k,
        _ => return None,
    };
    let rc = crate::loop_shape::range_counters(lp, data).ok()?;
    if rc.next.is_some() || rc.inclusive || rc.index != *index {
        return None;
    }
    let hi = match rc.hi.unspan() {
        Value::Int(k) => i64::from(*k),
        Value::Long(k) => *k,
        _ => return None,
    };
    let rounds = hi.checked_sub(seed_k)?.checked_sub(1)?;
    if !(1..=UNROLL_MAX).contains(&rounds)
        || !matches!(fn_vars.tp(rc.loop_var).base(), Type::Integer(_))
    {
        return None;
    }
    let body = &lp.operators[1..];
    let mut nodes = 0usize;
    let mut straight = true;
    for o in body {
        o.walk(&mut |n| {
            nodes += 1;
            if matches!(
                n,
                Value::Loop(_)
                    | Value::Break(_)
                    | Value::Continue(_)
                    | Value::Return(_)
                    | Value::Yield(_)
            ) {
                straight = false;
            }
        });
    }
    (straight && nodes <= UNROLL_NODES).then_some(Unroll {
        seed_at: *seed_at,
        index: *index,
        loop_var: rc.loop_var,
        first: seed_k + 1,
        rounds,
        body,
    })
}

impl Output<'_> {
    /// `@FR-R-FaultFlag` admission: the written locals the rerun restores (those declared
    /// before the statement), or `None` with the reason traced under `LOFT_TRACE_FAULT_FLAG`.
    pub(super) fn fault_flag_admits(&self, bl: &Block) -> Option<Vec<u16>> {
        if self.switches.fault_flag_disabled
            || self.fault_flag.is_some()
            || self.in_coroutine_body
            || self.release_pass_probe
        {
            return None;
        }
        let data = self.data;
        let fn_name = data.def(self.def_nr).name();
        let decline = |why: &str| {
            if self.fault_flag_trace {
                eprintln!("fault-flag: {fn_name} declines a `for`: {why}");
            }
            None
        };
        let mut walk = Walk {
            data,
            writes: HashSet::new(),
            fault_forms: 0,
        };
        if let Err(why) = walk.all(&bl.operators, 0) {
            return decline(&why);
        }
        if walk.fault_forms == 0 {
            return decline("nothing in it can fault");
        }
        let variables = data.def(self.def_nr).variables();
        let mut escaped: HashSet<u16> = HashSet::new();
        super::non_sentinel::collect_escapes(data, data.def(self.def_nr).code(), &mut escaped);
        let mut saved: Vec<u16> = Vec::new();
        for &v in &walk.writes {
            if !is_scalar(variables.tp(v)) {
                return decline(&format!("it writes `{}`, not a scalar", variables.name(v)));
            }
            if escaped.contains(&v) {
                return decline(&format!(
                    "`{}` is referenced, not only read",
                    variables.name(v)
                ));
            }
            if self.declared.contains(&v) {
                saved.push(v);
            }
        }
        saved.sort_unstable();
        if self.fault_flag_trace {
            eprintln!(
                "fault-flag: {fn_name} admits a `for`: {} fault form(s), {} local(s) restored",
                walk.fault_forms,
                saved.len()
            );
        }
        Some(saved)
    }

    /// `@FR-R-FaultFlag` emission: the saved locals, the flag, the fast copy, and the exact
    /// statement under the flag.  `code` is the statement's own node, emitted once more for the
    /// exact copy through the ordinary path, which `fault_flag_exact` keeps from admitting it
    /// again.
    pub(super) fn fault_flag_statement(
        &mut self,
        w: &mut dyn Write,
        code: &Value,
        bl: &Block,
        saved: &[u16],
    ) -> std::io::Result<()> {
        let n = self.fault_flag_next;
        self.fault_flag_next += 1;
        let variables = self.data.def(self.def_nr).variables();
        let names: Vec<String> = saved
            .iter()
            .map(|v| format!("var_{}", sanitize(variables.name(*v))))
            .collect();
        crate::rewrite_census::fired("R-FaultFlag", 1);
        writeln!(w, "{{ //@FR-R-FaultFlag")?;
        self.indent(w)?;
        writeln!(
            w,
            "static __FFM_{n}: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);"
        )?;
        if !names.is_empty() {
            self.indent(w)?;
            writeln!(w, "let __ffs_{n} = ({},);", names.join(", "))?;
        }
        self.indent(w)?;
        writeln!(
            w,
            "let mut __ff_{n}: bool = loft::ops::fault_flag_skip(&__FFM_{n});"
        )?;
        self.indent(w)?;
        writeln!(w, "if !__ff_{n} {{ //@FR-R-FaultFlag fast copy")?;
        let declared_before = self.declared.clone();
        let range_before = self.in_range_copy;
        self.in_range_copy = true;
        self.fault_flag = Some(format!("__ff_{n}"));
        self.indent(w)?;
        let r = self.output_code_inner(w, code);
        self.fault_flag = None;
        self.in_range_copy = range_before;
        self.declared = declared_before;
        r?;
        writeln!(w)?;
        self.indent(w)?;
        writeln!(w, "}}")?;
        self.indent(w)?;
        write!(
            w,
            "if __ff_{n} {{ loft::ops::fault_flag_rerun(&__FFM_{n}); "
        )?;
        if !names.is_empty() {
            write!(w, "({},) = __ffs_{n}; ", names.join(", "))?;
        }
        writeln!(w)?;
        let ptr = std::ptr::from_ref(bl) as usize;
        self.fault_flag_exact.insert(ptr);
        self.indent(w)?;
        let r = self.output_code_inner(w, code);
        self.fault_flag_exact.remove(&ptr);
        r?;
        writeln!(w)?;
        self.indent(w)?;
        write!(w, "}} }}")
    }

    /// The unrolled form of a constant range inside a fast copy: the statement's leading
    /// operators, the seed, then each round as its own block with the loop variable set, and
    /// the counter left where the loop leaves it.
    pub(super) fn fault_flag_unrolled(
        &mut self,
        w: &mut dyn Write,
        bl: &Block,
        u: &Unroll<'_>,
    ) -> std::io::Result<()> {
        let variables = self.data.def(self.def_nr).variables();
        let index = format!("var_{}", sanitize(variables.name(u.index)));
        let lv = format!("var_{}", sanitize(variables.name(u.loop_var)));
        writeln!(w, "{{ //@FR-R-FaultFlag unrolled, {} rounds", u.rounds)?;
        for op in &bl.operators[..=u.seed_at] {
            self.indent(w)?;
            self.output_code_inner(w, op)?;
            writeln!(w, ";")?;
        }
        for k in 0..u.rounds {
            let declared_before = self.declared.clone();
            self.indent(w)?;
            if self.declared.contains(&u.loop_var) {
                writeln!(w, "{{ {lv} = {}_i64;", u.first + k)?;
            } else {
                writeln!(w, "{{ let mut {lv}: i64 = {}_i64;", u.first + k)?;
            }
            for op in u.body {
                self.indent(w)?;
                self.output_code_inner(w, op)?;
                writeln!(w, ";")?;
            }
            self.indent(w)?;
            writeln!(w, "}}")?;
            self.declared = declared_before;
        }
        self.indent(w)?;
        write!(w, "{index} = {}_i64; }}", u.first + u.rounds)
    }
}

/// Whether a `for` statement is one the ordinary path should hand to `@FR-R-FaultFlag`: not
/// the exact copy of a statement already being emitted under it.
pub(super) fn is_exact_copy(out: &Output<'_>, bl: &Block) -> bool {
    out.fault_flag_exact
        .contains(&(std::ptr::from_ref(bl) as usize))
}
