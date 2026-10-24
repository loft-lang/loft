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
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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

/// Does an operator template touch anything but its operands once its recoverable raise is
/// taken out?  A `s.` method, the `stores` or `cell` handles, or a `crate::` path all reach
/// state a rerun could observe.
fn touches_state(template: &str) -> bool {
    let t = template
        .replace("s.raise_recoverable(", "")
        .replace("crate::runtime_error::", "")
        .replace("crate::state::STRING_NULL", "");
    if t.contains("stores") || t.contains("cell") || t.contains("crate::") {
        return true;
    }
    let b = t.as_bytes();
    t.match_indices("s.")
        .any(|(at, _)| at == 0 || !(b[at - 1].is_ascii_alphanumeric() || b[at - 1] == b'_'))
}

/// The operators that READ a store and change nothing, by name: their templates reach the
/// store the same way a write does (`stores.store(&db)`), so the text cannot tell them apart.
/// A statement that writes no store reads the same values in its fast copy and its rerun.  The
/// file readers (`OpGetFile`, `OpGetDir`, `OpSizeFile`) are not here: a file can change between
/// the two runs.
const READS: &[&str] = &[
    "OpGetBoolean",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetCharacter",
    "OpGetDbRef",
    "OpGetEnum",
    "OpGetField",
    "OpGetFloat",
    "OpGetInt",
    "OpGetRecord",
    "OpGetRef",
    "OpGetShort",
    "OpGetShortFull",
    "OpGetShortRaw",
    "OpGetShortSpare",
    "OpGetSingle",
    "OpGetText",
    "OpGetVector",
    "OpGetVectorNullable",
    "OpLengthCharacter",
    "OpLengthHash",
    "OpLengthIndex",
    "OpLengthSorted",
    "OpLengthSpatial",
    "OpLengthText",
    "OpLengthTrie",
    "OpLengthVector",
    "OpSizeHash",
    "OpSizeIndex",
    "OpSizeofRef",
    "OpSizeScalar",
    "OpSizeSpatial",
    "OpSizeStruct",
    "OpSizeText",
    "OpSizeTrie",
    "OpSizeVector",
];

/// How deep the pure-call clause follows calls before it declines.
const CALL_DEPTH: u8 = 4;

/// The admission walk over one `for` statement, or over the body of a function it calls.
struct Walk<'a> {
    data: &'a Data,
    writes: HashSet<u16>,
    fault_forms: usize,
    /// Walking a CALLEE: its raise would report from inside the fast copy, so a raising
    /// template declines; its `return` ends only its own frame.
    callee: bool,
    /// The calls followed to reach this body.
    calls: u8,
    /// Each function already asked: `None` pure, `Some(why)` not.
    pure: &'a RefCell<HashMap<u32, Option<String>>>,
}

impl Walk<'_> {
    /// `depth` counts the loops of the statement around `v`, so a `break` / `continue` whose
    /// level reaches past them leaves the statement and declines.  The fallback declines: a
    /// node this does not name is one whose effect it has not proven local, and a fast copy's
    /// effect must be undone by restoring locals alone.
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
            | Value::Text(_)
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
                if !self.callee && is_discharge(self.data, t, a) {
                    self.fault_forms += 1;
                }
                self.node(t, depth)?;
                self.node(a, depth)?;
                self.node(b, depth)
            }
            Value::Call(d, args) => {
                self.call(*d)?;
                self.all(args, depth)
            }
            Value::Return(e) if self.callee => self.node(e, depth),
            Value::Return(_) => Err("a `return` leaves the statement".to_string()),
            _ => Err("a node whose effect is not local to the statement".to_string()),
        }
    }

    /// A call: a store READ by name, an operator whose template touches nothing but its
    /// operands, or a loft function whose own body passes this walk.
    fn call(&mut self, d: u32) -> Result<(), String> {
        let def = self.data.def(d);
        let name = def.name();
        if READS.contains(&name) {
            return Ok(());
        }
        if !def.rust.is_empty() {
            if touches_state(&def.rust) {
                return Err(format!("`{name}` touches state outside its operands"));
            }
            let raises = def.rust.contains("s.raise_recoverable(");
            if raises && self.callee {
                return Err(format!("`{name}` raises inside a called function"));
            }
            if !self.callee && raises {
                self.fault_forms += 1;
            }
            return Ok(());
        }
        if !def.is_loft_defined() || !def.native.is_empty() {
            return Err(format!("`{name}` is not a loft function"));
        }
        if let Some(known) = self.pure.borrow().get(&d) {
            return match known {
                None => Ok(()),
                Some(why) => Err(why.clone()),
            };
        }
        if self.calls >= CALL_DEPTH {
            return Err(format!("`{name}` is more than {CALL_DEPTH} calls deep"));
        }
        // Provisionally impure, so a recursive call declines instead of looping.
        self.pure
            .borrow_mut()
            .insert(d, Some(format!("`{name}` is recursive")));
        let verdict = self.callee_body(d);
        let entry = verdict.clone().err();
        self.pure.borrow_mut().insert(d, entry);
        verdict
    }

    /// A called function is pure when it takes no hidden or by-reference parameter (each
    /// writes the caller's state) and its body passes the walk as a callee.
    fn callee_body(&self, d: u32) -> Result<(), String> {
        let def = self.data.def(d);
        let name = def.name();
        for a in def.attributes() {
            // A by-reference parameter, also as `τ?` (`.base()` would peel the `&` away too).
            let (shape, _) = a.typedef.peel_optional();
            if a.hidden || matches!(shape, Type::RefVar(_)) {
                return Err(format!(
                    "`{name}` writes through its parameter `{}`",
                    a.name
                ));
            }
        }
        let mut inner = Walk {
            data: self.data,
            writes: HashSet::new(),
            fault_forms: 0,
            callee: true,
            calls: self.calls + 1,
            pure: self.pure,
        };
        inner
            .node(def.code(), 0)
            .map_err(|why| format!("`{name}` is not pure: {why}"))
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
            callee: false,
            calls: 0,
            pure: &self.fault_flag_pure,
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
            // A local declared inside the statement lives and dies in each copy, so it needs no
            // restore whatever it holds: a value made by an operator the walk admitted (a view
            // into a store, a text) owns nothing.  One declared before is restored by copy, so
            // it must be a scalar.
            if !is_scalar(variables.tp(v)) && self.declared.contains(&v) {
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
            // The counter as well as the loop variable: the body may read `#index`, which is
            // the counter's value in this round (single-counter form: the two are equal).
            if self.declared.contains(&u.loop_var) {
                writeln!(
                    w,
                    "{{ {index} = {k1}_i64; {lv} = {k1}_i64;",
                    k1 = u.first + k
                )?;
            } else {
                writeln!(
                    w,
                    "{{ {index} = {k1}_i64; let mut {lv}: i64 = {k1}_i64;",
                    k1 = u.first + k
                )?;
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
