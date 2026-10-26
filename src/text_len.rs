// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-TextLen` — a loop counts a text's length once, until the text is written.
//!
//! ```text
//! while i < len(t) { … }     →   m = -1;
//!                                while { if m < 0 { m = len(t) }; i < m } { …; t += x; m = -1; … }
//! ```
//!
//! `len` of a text counts its characters, so a loop that asks it per iteration walks the whole
//! text each time.  For a text VARIABLE `t` that borrows from nothing and does not escape the
//! function, every `len(t)` in a loop reads a memo instead: the count is made before the
//! innermost statement that reads it, when the memo is clear, and every statement that may
//! write `t` clears the memo after it.  The memo is cleared before each entry to the outermost
//! loop that reads `len(t)`.  A function that yields or runs arms in parallel is left alone.
//! It is the last of the compile phase's IR passes: after `tail_self` (a self-tail call is a
//! loop only from there), after `zero_count` (a count compared with zero reads the byte size)
//! and after every pass that reasons about `len` itself.
//!
//! `LOFT_NO_TEXT_LEN=1` keeps every count; `LOFT_TRACE_TEXT_LEN=1` names each memoised text.

use crate::data::{Data, DefType, Type, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};

struct Ops {
    /// `OpLengthText` and the stdlib method `len(self: text)`: the two spellings of a count.
    op_len: u32,
    method_len: u32,
    lt: u32,
    int_tp: Type,
    /// The function body's scope, which a memo is declared in (the scope pass has run).
    scope: u16,
    /// Per candidate text, the text variables it borrows from: a write to one of them is a
    /// write to the candidate.
    deps: BTreeMap<u16, Vec<u16>>,
}

/// The text variable a node counts, when it is a count of one of `cands`.
fn count_of(v: &Value, ops: &Ops, cands: &HashSet<u16>) -> Option<u16> {
    if let Value::Call(d, args) = v.unspan()
        && (*d == ops.op_len || *d == ops.method_len)
        && let [a] = args.as_slice()
        && let Value::Var(t) = a.unspan()
        && cands.contains(t)
    {
        return Some(*t);
    }
    None
}

/// The texts of `cands` that `v` counts anywhere.
fn counted(v: &Value, ops: &Ops, cands: &HashSet<u16>, out: &mut BTreeSet<u16>) {
    if let Some(t) = count_of(v, ops, cands) {
        out.insert(t);
    }
    v.for_each_child(&mut |c| counted(c, ops, cands, out));
}

/// The texts of `among` a statement may write ITSELF — a rebind, an operator whose
/// written-text operand (`pure_reuse::text_write_target`) names it, a release — outside the
/// statement lists nested in it, which clear the memo after their own writes.
fn written(v: &Value, data: &Data, ops: &Ops, among: &BTreeMap<u16, u16>, out: &mut BTreeSet<u16>) {
    // A write to `x` writes every memoised text that is `x` or borrows from it.
    let mut hit = |x: u16| {
        for t in among.keys() {
            if *t == x || ops.deps.get(t).is_some_and(|d| d.contains(&x)) {
                out.insert(*t);
            }
        }
    };
    match v.unspan() {
        Value::Set(x, _) => hit(*x),
        Value::Call(d, args) => {
            let name = data.def(*d).name();
            let target = crate::pure_reuse::text_write_target(name)
                .or_else(|| name.starts_with("OpFree").then_some(0));
            if let Some(k) = target
                && let Some(Value::Var(x)) = args.get(k).map(Value::unspan)
            {
                hit(*x);
            }
        }
        Value::Block(_) | Value::Loop(_) | Value::Insert(_) => return,
        _ => {}
    }
    v.for_each_child(&mut |c| written(c, data, ops, among, out));
}

/// The texts of `cands` a statement counts ITSELF: outside the statement lists nested in it,
/// which their own pass rewrites (and whose fills count the text again).
fn counted_direct(v: &Value, ops: &Ops, cands: &HashSet<u16>, out: &mut BTreeSet<u16>) {
    if let Some(t) = count_of(v, ops, cands) {
        out.insert(t);
        return;
    }
    if matches!(
        v.unspan(),
        Value::Block(_) | Value::Loop(_) | Value::Insert(_)
    ) {
        return;
    }
    v.for_each_child(&mut |c| counted_direct(c, ops, cands, out));
}

/// Every count a statement makes itself (as [`counted_direct`]) becomes a read of its memo.
fn replace_direct(v: &mut Value, ops: &Ops, memo: &BTreeMap<u16, u16>, cands: &HashSet<u16>) {
    if let Some(t) = count_of(v, ops, cands) {
        *v = Value::Var(memo[&t]);
        return;
    }
    if matches!(
        v.unspan(),
        Value::Block(_) | Value::Loop(_) | Value::Insert(_)
    ) {
        return;
    }
    v.for_each_child_mut(&mut |c| replace_direct(c, ops, memo, cands));
}

/// Rewrite the statement lists nested in `v` (not `v` itself).
fn nested_lists(v: &mut Value, data: &Data, ops: &Ops, memo: &BTreeMap<u16, u16>) {
    match v.unspan_mut() {
        Value::Block(bl) | Value::Loop(bl) => statements(&mut bl.operators, data, ops, memo),
        Value::Insert(list) => statements(list, data, ops, memo),
        other => other.for_each_child_mut(&mut |c| nested_lists(c, data, ops, memo)),
    }
}

/// One statement list inside a memoised loop: its nested lists first, then each statement's
/// own counts filled before it, and the memo of each text it may write cleared after it.
fn statements(list: &mut Vec<Value>, data: &Data, ops: &Ops, memo: &BTreeMap<u16, u16>) {
    let cands: HashSet<u16> = memo.keys().copied().collect();
    let mut i = 0;
    while i < list.len() {
        nested_lists(&mut list[i], data, ops, memo);
        let mut reads = BTreeSet::new();
        counted_direct(&list[i], ops, &cands, &mut reads);
        let mut writes = BTreeSet::new();
        written(&list[i], data, ops, memo, &mut writes);
        replace_direct(&mut list[i], ops, memo, &cands);
        let fills: Vec<Value> = reads
            .iter()
            .map(|t| {
                let m = memo[t];
                Value::If(
                    Box::new(Value::Call(ops.lt, vec![Value::Var(m), Value::Int(0)])),
                    Box::new(Value::Set(
                        m,
                        Box::new(Value::Call(ops.op_len, vec![Value::Var(*t)])),
                    )),
                    Box::new(Value::Null),
                )
            })
            .collect();
        let n = fills.len();
        for (k, fill) in fills.into_iter().enumerate() {
            list.insert(i + k, fill);
        }
        i += n + 1;
        for t in writes {
            list.insert(i, Value::Set(memo[&t], Box::new(Value::Int(-1))));
            i += 1;
        }
    }
}

/// Find the outermost loops in the statement list `list` (and in the lists below the ones
/// that are not loops) and memoise the counts each makes.
fn loops_in(
    list: &mut Vec<Value>,
    f: u32,
    data: &mut Data,
    ops: &Ops,
    cands: &HashSet<u16>,
    trace: bool,
) {
    let scope = ops.scope;
    let mut i = 0;
    while i < list.len() {
        if matches!(list[i].unspan(), Value::Loop(_)) {
            let mut texts = BTreeSet::new();
            counted(&list[i], ops, cands, &mut texts);
            if !texts.is_empty() {
                let mut memo = BTreeMap::new();
                for t in &texts {
                    let vars = &mut data.definitions[f as usize].variables;
                    memo.insert(*t, vars.add_unique("len", &ops.int_tp, scope));
                }
                if trace {
                    let def = data.def(f);
                    let names: Vec<&str> = texts.iter().map(|t| def.variables.name(*t)).collect();
                    eprintln!(
                        "text_len: {} counts {} once per write",
                        def.name(),
                        names.join(", ")
                    );
                }
                crate::rewrite_census::fired("R-TextLen", texts.len());
                if let Value::Loop(bl) = list[i].unspan_mut() {
                    statements(&mut bl.operators, data, ops, &memo);
                }
                for m in memo.values().rev() {
                    list.insert(i, Value::Set(*m, Box::new(Value::Int(-1))));
                    i += 1;
                }
            }
            i += 1;
            continue;
        }
        lists_below(&mut list[i], f, data, ops, cands, trace);
        i += 1;
    }
}

fn lists_below(
    v: &mut Value,
    f: u32,
    data: &mut Data,
    ops: &Ops,
    cands: &HashSet<u16>,
    trace: bool,
) {
    match v.unspan_mut() {
        Value::Block(bl) => loops_in(&mut bl.operators, f, data, ops, cands, trace),
        Value::Insert(list) => loops_in(list, f, data, ops, cands, trace),
        other => other.for_each_child_mut(&mut |c| lists_below(c, f, data, ops, cands, trace)),
    }
}

/// Memoise the text counts of every loop of every function (`@FR-R-TextLen`).
pub fn rewrite_program(data: &mut Data) {
    if !crate::keys::text_len_enabled() {
        return;
    }
    let trace = crate::keys::trace_text_len();
    let method_len = data.def_nr("t_4text_len");
    let op_len = data.def_nr("OpLengthText");
    let lt = data.def_nr("OpLtInt");
    if op_len == u32::MAX || lt == u32::MAX {
        return;
    }
    let mut ops = Ops {
        op_len,
        method_len,
        lt,
        int_tp: data.def(op_len).returned.clone(),
        scope: 0,
        deps: BTreeMap::new(),
    };
    for f in 0..data.definitions() {
        let def = data.def(f);
        if !matches!(def.def_type, DefType::Function) || matches!(def.code(), Value::Null) {
            continue;
        }
        let code = def.code();
        if !code.any_node(&mut |n| matches!(n, Value::Loop(_)))
            || code.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_)))
        {
            continue;
        }
        // The candidates: text variables that borrow from nothing and do not escape.
        let mut escaped = HashSet::new();
        crate::opt::non_sentinel::collect_escapes(data, code, &mut escaped);
        // A text that borrows only from texts that do not escape either — itself, a format
        // work buffer — is a candidate: a write to one of those is a write to it.  A text that
        // borrows from a store-held variable (a record, a vector) is not: a write through that
        // variable would change it without naming it.
        let vars = &def.variables;
        let is_text = |v: u16| matches!(vars.tp(v), Type::Text(_));
        let mut deps = BTreeMap::new();
        let cands: HashSet<u16> = (0..vars.count())
            .filter(|v| {
                let d: Vec<u16> = vars
                    .tp(*v)
                    .depend()
                    .into_iter()
                    .filter(|x| x != v)
                    .collect();
                let ok = is_text(*v)
                    && !escaped.contains(v)
                    && d.iter().all(|x| is_text(*x) && !escaped.contains(x));
                if ok {
                    deps.insert(*v, d);
                }
                ok
            })
            .collect();
        ops.deps = deps;
        if cands.is_empty() {
            continue;
        }
        let mut texts = BTreeSet::new();
        counted(code, &ops, &cands, &mut texts);
        if texts.is_empty() {
            continue;
        }
        ops.scope = match code {
            Value::Block(b) => b.scope,
            _ => 0,
        };
        let mut code = std::mem::replace(&mut data.definitions[f as usize].code, Value::Null);
        lists_below(&mut code, f, data, &ops, &cands, trace);
        data.definitions[f as usize].code = code;
        // The memos are locals the scope pass never saw: the function's live intervals and
        // slots are computed again, as `tail_self` does for its temporaries.
        data.definitions[f as usize].variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, f);
        crate::scopes::assign_function_slots(data, f);
    }
}
