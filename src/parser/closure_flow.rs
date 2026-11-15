// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! `@FR-L-Escape` — a capturing closure passed to a parameter its callee KEEPS (loft#2492).
//!
//! A place that keeps a function value beyond a call — a collection element (@C116) or a
//! struct's fn-ref field (one capture shape per field, `formal/closures.md`) — holds a plain
//! function: its function number, never a closure record.  Where the source is a closure the
//! writing function can see, the parser refuses it on the spot.  A fn-typed PARAMETER is not
//! such a source: nothing in its own function says whether the caller's value captures, so the
//! write stored the number and dropped the closure, and a call through the stored value then
//! read a released or absent record (a corrupt reference on the interpreter, a capture reading
//! nothing on the C backend).
//!
//! The caller is the one that knows what it passes, so the refusal is at the argument.  Each
//! call whose argument may carry a closure (`fn_ref_source_captures`) is recorded on pass 2,
//! and judged once every body is final: refused when the callee keeps that parameter.

use crate::data::{Data, Type, Value};
use crate::diagnostics::{Level, diagnostic_format};
use crate::fxhash::{FxHashMap, FxHashSet};
use crate::lexer::Position;
use crate::parser::Parser;

/// One call argument that may carry a closure record, recorded on pass 2.
pub(crate) struct ClosureArgument {
    callee: u32,
    arg: usize,
    at: Position,
}

/// The fn-typed parameters of `function` that its body KEEPS beyond the call, by number:
/// written to a fn-ref slot as a function number alone (`FnRefDnr`, which is only built to
/// write one), directly or through a local assigned from the parameter.  A write into a
/// closure RECORD is not one: that is a lambda capturing the parameter, and the record keeps
/// the closure half in the `<name>__clos` slot beside it (`@FR-L-Fn`, loft#1869).  `forwards` collects
/// each fn-typed parameter passed on as another call's argument, for the caller's fixpoint.
fn kept_here(
    data: &Data,
    function: u32,
    forwards: &mut Vec<(u32, u16, u32, usize)>,
) -> FxHashSet<u16> {
    let def = data.def(function);
    let vars = def.variables();
    let is_fn_param =
        |v: u16| vars.is_argument(v) && matches!(vars.tp(v).base(), Type::Function(..));
    let mut written: FxHashSet<u16> = FxHashSet::default();
    let mut from: Vec<(u16, u16)> = Vec::new();
    // The variable a value is: itself, or the one an `if` arm or a block yields.
    fn sources(v: &Value, out: &mut Vec<u16>) {
        match v.unspan() {
            Value::Var(x) => out.push(*x),
            Value::If(_, t, e) => {
                sources(t, out);
                sources(e, out);
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last() {
                    sources(last, out);
                }
            }
            _ => {}
        }
    }
    // A capture's write: `OpSetInt4(record, pos, FnRefDnr(v))` into a `__closure_*` record.
    let into_record = |args: &[Value]| -> bool {
        let [host, _, dnr] = args else { return false };
        let Value::Var(h) = host.unspan() else {
            return false;
        };
        let host_type = vars.tp(*h).base();
        matches!(dnr.unspan(), Value::FnRefDnr(_))
            && matches!(host_type, Type::Reference(r, _) if data.def(*r).name().starts_with("__closure_"))
    };
    fn walk(
        node: &Value,
        data: &Data,
        into_record: &dyn Fn(&[Value]) -> bool,
        written: &mut FxHashSet<u16>,
        from: &mut Vec<(u16, u16)>,
        calls: &mut Vec<(u32, usize, u16)>,
    ) {
        match node.unspan() {
            Value::FnRefDnr(v) => {
                written.insert(*v);
            }
            Value::Call(d, args) if data.def(*d).name() == "OpSetInt4" && into_record(args) => {
                for a in &args[..2] {
                    walk(a, data, into_record, written, from, calls);
                }
                return;
            }
            Value::Set(to, val) => {
                let mut s = Vec::new();
                sources(val, &mut s);
                for x in s {
                    from.push((*to, x));
                }
            }
            Value::Call(d, args) => {
                for (k, a) in args.iter().enumerate() {
                    if let Value::Var(x) = a.unspan() {
                        calls.push((*d, k, *x));
                    }
                }
            }
            _ => {}
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, data, into_record, written, from, calls));
    }
    let mut calls = Vec::new();
    walk(
        &def.code,
        data,
        &into_record,
        &mut written,
        &mut from,
        &mut calls,
    );
    // A local written as a number reaches back to every variable it was assigned from.
    let mut reach: Vec<u16> = written.iter().copied().collect();
    let mut seen: FxHashSet<u16> = reach.iter().copied().collect();
    while let Some(v) = reach.pop() {
        for &(to, x) in &from {
            if to == v && seen.insert(x) {
                reach.push(x);
            }
        }
    }
    for (d, k, x) in calls {
        if is_fn_param(x) {
            forwards.push((function, x, d, k));
        }
    }
    seen.into_iter().filter(|&v| is_fn_param(v)).collect()
}

impl Parser {
    /// Record each argument of the call to `callee` (`actual`, defaults filled) that may carry a
    /// closure record, for [`Self::check_closure_keeps`].
    pub(crate) fn record_closure_arguments(
        &mut self,
        callee: u32,
        actual: &[Value],
        arg_pos: &[Position],
    ) {
        if self.first_pass || (callee as usize) >= self.data.definitions.len() {
            return;
        }
        for (k, a) in actual.iter().enumerate() {
            if self.fn_ref_source_captures(a) {
                let at = arg_pos.get(k).copied().unwrap_or_else(|| *self.lexer.pos());
                self.closure_arguments
                    .push(ClosureArgument { callee, arg: k, at });
            }
        }
    }

    /// Refuse every recorded closure argument whose callee keeps that parameter: in its own
    /// body, or by passing it on to a callee that keeps it (a fixpoint over the forwards).
    pub(crate) fn check_closure_keeps(&mut self) {
        let checks = std::mem::take(&mut self.closure_arguments);
        if checks.is_empty() {
            return;
        }
        let mut kept: FxHashMap<u32, FxHashSet<u16>> = FxHashMap::default();
        let mut forwards = Vec::new();
        let mut asked: FxHashSet<u32> = FxHashSet::default();
        let mut queue: Vec<u32> = checks.iter().map(|c| c.callee).collect();
        while let Some(f) = queue.pop() {
            if !asked.insert(f) || (f as usize) >= self.data.definitions.len() {
                continue;
            }
            let start = forwards.len();
            let here = kept_here(&self.data, f, &mut forwards);
            kept.insert(f, here);
            for &(_, _, d, _) in &forwards[start..] {
                queue.push(d);
            }
        }
        // A parameter handed on to a callee's kept parameter is kept too.
        let mut changed = true;
        while changed {
            changed = false;
            for &(caller, param, callee, arg) in &forwards {
                let callee_keeps = kept
                    .get(&callee)
                    .is_some_and(|k| u16::try_from(arg).is_ok_and(|a| k.contains(&a)));
                if callee_keeps && kept.entry(caller).or_default().insert(param) {
                    changed = true;
                }
            }
        }
        for c in checks {
            let Ok(arg) = u16::try_from(c.arg) else {
                continue;
            };
            if !kept.get(&c.callee).is_some_and(|k| k.contains(&arg)) {
                continue;
            }
            let def = self.data.def(c.callee);
            let callee = def.name().trim_start_matches("n_").to_string();
            let param = def.variables().name(arg).to_string();
            diagnostic_at!(
                self.lexer,
                &c.at,
                Level::Error,
                "a capturing closure cannot be passed to '{callee}' as '{param}': '{callee}' keeps \
                 it in a collection or a struct field, which holds a plain function — pass a \
                 function that captures nothing and reads its state from a struct field, or \
                 build the closure where it is stored"
            );
        }
    }
}
