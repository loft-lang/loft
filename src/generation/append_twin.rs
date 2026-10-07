// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-AppendTwin` — `X += f(args)` built in X.
//!
//! A function returning a vector builds it in the buffer its caller hands it, after clearing
//! that buffer; a caller that APPENDS the result then copies it into its own vector.  The
//! append twin `f__ap(args, X)` is f's body with X as the buffer and none of its clears: the
//! body's appends land after X's own elements, and the copy is gone.  That is sound exactly
//! when the body never OBSERVES what the buffer held on entry, which [`eligible`] decides from
//! the IR: every mention of the buffer, or of a view of it, is a leading clear or mint, an
//! append into it, a call handing it to an eligible callee AS that callee's buffer, or the
//! value flowing out as the result.  Anything else — a length read, an index, a walk, the
//! buffer passed as an ordinary argument, a free — reads its content, and declines.
//!
//! The caller's half ([`call_site`]) asks that no argument reach X: a caller never hands a
//! buffer one of its other arguments reaches (loft#1895 closed the one rebind that did), so
//! X's root must be the frame's own buffer or a local, and no argument may name that root or
//! a local whose dependencies do.  Native only; the interpreter copies as before and is the
//! reference.

use crate::data::{Data, Value};

fn trace() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_TRACE_APPEND_TWIN").is_some())
}
use crate::variables::Function;
use std::collections::{HashMap, HashSet};

/// The function's hidden vector return buffer, if it has one.
#[must_use]
pub fn buffer(data: &Data, d_nr: u32) -> Option<u16> {
    let def = data.def(d_nr);
    if !matches!(def.returned().base(), crate::data::Type::Vector(_, _)) {
        return None;
    }
    super::hoist::retbuf_var(data, d_nr)
}

/// The buffer and every local whose dependencies reach it: the views a body builds through.
#[must_use]
pub fn views(vars: &Function, b: u16) -> HashSet<u16> {
    let mut set: HashSet<u16> = HashSet::from([b]);
    loop {
        let before = set.len();
        for v in 0..vars.next_var() {
            if !set.contains(&v) && vars.tp(v).depend().iter().any(|d| set.contains(d)) {
                set.insert(v);
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Pos {
    /// The value is a statement's or a result's: a view here only flows on.
    Flow,
    /// The value is an operand: a view here is read.
    Operand,
}

struct Scan<'a> {
    data: &'a Data,
    vars: &'a Function,
    b: u16,
    /// The buffer's views: those whose dependencies reach it, and every alias the body binds
    /// of one (`w = OpRefAlias(B)`, a rebind witness) — its uses are the buffer's too.
    vb: HashSet<u16>,
    written: bool,
    memo: &'a mut HashMap<u32, bool>,
    /// The first node that declined, for `LOFT_TRACE_APPEND_TWIN`.
    why: Option<String>,
}

impl Scan<'_> {
    fn is_view(&self, v: &Value) -> bool {
        matches!(v.unspan(), Value::Var(x) if self.vb.contains(x))
    }

    fn name(&self, d: u32) -> &str {
        self.data.def(d).name()
    }

    /// Scan every argument as an operand: none may be a view of the buffer.
    fn operands(&mut self, args: &[Value]) -> bool {
        args.iter().all(|a| self.scan(a, Pos::Operand))
    }

    fn scan(&mut self, v: &Value, pos: Pos) -> bool {
        let ok = self.scan_node(v, pos);
        if !ok && self.why.is_none() {
            self.why = Some(match v.unspan() {
                Value::Call(d, _) => format!("{} (written={})", self.name(*d), self.written),
                Value::Var(x) => format!("a view read as an operand: {}", self.vars.name(*x)),
                other => format!("{:?}", std::mem::discriminant(other)),
            });
        }
        ok
    }

    fn scan_node(&mut self, v: &Value, pos: Pos) -> bool {
        match v.unspan() {
            Value::Var(x) => !self.vb.contains(x) || pos == Pos::Flow,
            Value::Set(x, rhs) => {
                if let Value::Call(d, a) = rhs.unspan()
                    && self.name(*d) == "OpRefAlias"
                    && a.first().is_some_and(|f| self.is_view(f))
                {
                    self.vb.insert(*x);
                    return self.operands(&a[1..]);
                }
                if self.vb.contains(x) {
                    // `view = OpGetField(B, …)` / `w = OpRefAlias(B)`: deriving a view.
                    if let Value::Call(d, a) = rhs.unspan()
                        && matches!(self.name(*d), "OpGetField" | "OpRefAlias")
                        && a.first().is_some_and(|f| self.is_view(f))
                    {
                        return self.operands(&a[1..]);
                    }
                    self.scan(rhs, Pos::Flow)
                } else {
                    self.scan(rhs, Pos::Operand)
                }
            }
            Value::Call(d, args) => self.call(*d, args),
            Value::Block(bl) | Value::Loop(bl) => {
                let n = bl.operators.len();
                bl.operators
                    .iter()
                    .enumerate()
                    .all(|(i, op)| self.scan(op, if i + 1 == n { pos } else { Pos::Flow }))
            }
            Value::Insert(ops) => ops.iter().all(|op| self.scan(op, Pos::Flow)),
            Value::If(c, t, e) => {
                self.scan(c, Pos::Operand) && self.scan(t, pos) && self.scan(e, pos)
            }
            Value::Return(r) => self.scan(r, Pos::Flow),
            Value::Drop(r) => self.scan(r, Pos::Operand),
            Value::CallRef(..) | Value::Parallel(_) | Value::Yield(_) => false,
            other => {
                // Any other node is read as an operand: the walk below reaches every
                // variable it names, and a view among them is a read.
                let mut ok = true;
                other.walk(&mut |n| {
                    if let Value::Var(x) = n
                        && self.vb.contains(x)
                    {
                        ok = false;
                    }
                });
                ok
            }
        }
    }

    fn call(&mut self, d: u32, args: &[Value]) -> bool {
        let name = self.name(d).to_string();
        let first_is_view = args.first().is_some_and(|a| self.is_view(a));
        let first_is_b =
            matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == self.b);
        match name.as_str() {
            // The leading forms: before the body writes, clearing or minting the buffer
            // changes nothing a twin's caller can see — the twin skips them.
            "OpClearVector" if first_is_view => !self.written && args.len() == 1,
            "OpDatabase" if first_is_b => !self.written && self.operands(&args[1..]),
            "OpSetInt4" if first_is_b => {
                !self.written
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Int(0)))
                    && matches!(args.get(2).map(Value::unspan), Some(Value::Int(0)))
            }
            // Appends into the buffer: the values may not read it.
            // `OpInsertVector` is not among them: it inserts at an index counted from the
            // buffer's start, which in a twin holds the caller's elements.
            "OpAppendVector" | "OpAppendTextBytes" | "OpPreAllocVector" if first_is_view => {
                self.written = true;
                self.operands(&args[1..])
            }
            n if n.starts_with("OpPush") && first_is_view => {
                self.written = true;
                self.operands(&args[1..])
            }
            // `OpReplaceVector(B, view)` — the self-replace a buffer's own view delivers.
            "OpReplaceVector" if first_is_b && args.get(1).is_some_and(|a| self.is_view(a)) => {
                self.operands(&args[2..])
            }
            _ => {
                let def = self.data.def(d);
                if def.is_loft_defined()
                    && let Some(at) = def.hidden_return_buffer_attr()
                    && args.get(at).is_some_and(|a| self.is_view(a))
                {
                    // The buffer handed on as the callee's own buffer.
                    self.written = true;
                    let others: Vec<&Value> = args
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| *i != at)
                        .map(|(_, a)| a)
                        .collect();
                    return others.iter().all(|a| self.scan(a, Pos::Operand))
                        && eligible(self.data, d, self.memo);
                }
                self.operands(args)
            }
        }
    }
}

/// Does `d_nr` have an append twin?  A fixpoint over the call graph: a function in progress
/// counts as eligible, so a recursive builder (`encode`'s array arm) can call its own twin.
pub fn eligible(data: &Data, d_nr: u32, memo: &mut HashMap<u32, bool>) -> bool {
    if let Some(&known) = memo.get(&d_nr) {
        return known;
    }
    let def = data.def(d_nr);
    if !def.is_loft_defined() {
        return false;
    }
    let Some(b) = buffer(data, d_nr) else {
        memo.insert(d_nr, false);
        return false;
    };
    memo.insert(d_nr, true);
    let vars = def.variables();
    let mut scan = Scan {
        data,
        vars,
        b,
        vb: views(vars, b),
        written: false,
        memo,
        why: None,
    };
    let ok = scan.scan(def.code(), Pos::Flow);
    let why = scan.why.take();
    memo.insert(d_nr, ok);
    if trace() {
        if ok {
            eprintln!("append-twin: {} has a twin", def.name());
        } else {
            eprintln!(
                "append-twin: {} observes its buffer at {}",
                def.name(),
                why.as_deref().unwrap_or("?")
            );
        }
    }
    ok
}

/// `OpAppendVector(X, f(args…, buf), tp)` where `f` has an append twin: answers the callee,
/// its arguments, the position of its buffer argument and X.  X's root is the frame's own
/// return buffer or a local, and no argument names that root or a local whose dependencies
/// reach it.
pub struct Site<'a> {
    pub callee: u32,
    pub args: &'a [Value],
    pub at: usize,
    pub dest: &'a Value,
}

#[must_use]
pub fn call_site<'a>(
    stmt: &'a Value,
    data: &Data,
    d_nr: u32,
    memo: &mut HashMap<u32, bool>,
) -> Option<Site<'a>> {
    let Value::Call(op, a) = stmt.unspan() else {
        return None;
    };
    if data.def(*op).name() != "OpAppendVector" {
        return None;
    }
    let [dest, src, _] = &a[..] else { return None };
    let Value::Call(callee, args) = src.unspan() else {
        return None;
    };
    let at = data.def(*callee).hidden_return_buffer_attr()?;
    if !matches!(args.get(at).map(Value::unspan), Some(Value::Var(_))) {
        return None;
    }
    let decline = |why: &str| {
        if trace() {
            eprintln!(
                "append-twin: {} += {}(…) declined — {why}",
                data.def(d_nr).name(),
                data.def(*callee).name()
            );
        }
    };
    let Some((root, _)) = super::hoist::vector_path(data, dest) else {
        decline("the destination is not a vector path");
        return None;
    };
    let vars = data.def(d_nr).variables();
    let own = buffer(data, d_nr);
    if vars.is_argument(root) && own.is_none_or(|b| !views(vars, b).contains(&root)) {
        decline("the destination is a parameter");
        return None;
    }
    let reach: HashSet<u16> = views(vars, root);
    let clean = args.iter().enumerate().all(|(i, x)| {
        if i == at {
            return true;
        }
        let mut ok = true;
        x.walk(&mut |n| {
            if let Value::Var(v) = n
                && reach.contains(v)
            {
                ok = false;
            }
        });
        ok
    });
    if !clean {
        decline("an argument reaches the destination");
        return None;
    }
    if !eligible(data, *callee, memo) {
        decline("the callee observes its buffer");
        return None;
    }
    Some(Site {
        callee: *callee,
        args,
        at,
        dest,
    })
}
