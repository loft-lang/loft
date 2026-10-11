// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-SameRead` — two equal discharged reads in one statement are one read, decided in the
//! IR phase for both backends.
//!
//! `v[i]?` lowers to a discharge block `{t = <read>; if t is not null then t else <default>}`.
//! A statement that spells the same read twice evaluates the read and its five-op discharge
//! twice.  When nothing in the statement can change what the read answers, the first block is
//! bound to a fresh local before the statement and every equal block reads that local.
//!
//! "Equal" is structural: the same read — operators, the same locals, the same literals — the
//! same conversion test and the same default.  "Nothing can change it": the read is built only
//! from read operators, locals and literals, and every call in the statement is a pure operator
//! (arithmetic, comparison, conversion, a read), so nothing assigns a local or writes a store
//! between the two.  A discharged read raises no fault, so the faults a run reports are the
//! ones it reported.  The pass does not look inside `if` arms or nested statement blocks: a read
//! one arm makes is not a read the other arm can reuse.
//!
//! THE GUARD CLAUSE — `if a < cum + p.count { return … }` followed by `cum += p.count`: the
//! condition of an `if` whose arm LEAVES (its last statement a `return`, `break` or
//! `continue`, and no `else`) evaluates a pure expression the very next statement evaluates
//! again.  On the path that reaches that statement nothing ran in between, so both answer the
//! same; the expression is bound once before the `if`.  A variable and a literal is not bound:
//! that is a counter's step, a spelling the counter rewrites read.  Checked arithmetic is admitted: an
//! overflow is still reported, once where the program spelled it twice (the owner's ruling,
//! formal/rewrites.md `(R-SameRead)`).
//!
//! `LOFT_NO_SAME_READ=1` keeps every read; `LOFT_TRACE_SAME_READ=1` names each statement it binds.
use crate::data::{Block, Data, DefType, Value};

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_SAME_READ").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_SAME_READ").is_ok_and(|v| v != "0"))
}

/// The operator families that read or compute without writing anything.
const PURE: [&str; 27] = [
    "OpAdd", "OpMin", "OpMul", "OpDiv", "OpRem", "OpLand", "OpLor", "OpEor", "OpSLeft", "OpSRight",
    "OpEq", "OpNe", "OpLt", "OpLe", "OpGt", "OpGe", "OpConv", "OpCast", "OpGet", "OpLength",
    "OpNot", "OpNeg", "OpAbs", "OpMath", "OpVecGet", "OpSize", "OpIntV",
];

/// The read operators a discharged read may be built from.
const READS: [&str; 4] = ["OpGet", "OpVecGet", "OpLength", "OpSize"];

fn named(data: &Data, op: u32, families: &[&str]) -> bool {
    let name = data.def(op).name();
    families.iter().any(|f| name.starts_with(f))
}

/// The parts of a discharge block: the temporary, the read, the conversion test's operator
/// and the default.
fn discharge(v: &Value) -> Option<(u16, &Value, u32, &Value)> {
    let Value::Block(b) = v.unspan() else {
        return None;
    };
    if b.name != "ncc" {
        return None;
    }
    let ops: Vec<&Value> = b
        .operators
        .iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let [Value::Set(t, read), Value::If(test, then_arm, default)] = ops.as_slice() else {
        return None;
    };
    let Value::Call(conv, cargs) = test.unspan() else {
        return None;
    };
    let reads_t = |x: &Value| matches!(x.unspan(), Value::Var(n) if n == t);
    (cargs.len() == 1 && reads_t(&cargs[0]) && reads_t(then_arm))
        .then_some((*t, &**read, *conv, &**default))
}

/// Is `v` built only from read operators, locals and literals?
fn plain_read(data: &Data, v: &Value) -> bool {
    match v.unspan() {
        Value::Var(_) | Value::Int(_) | Value::Long(_) => true,
        Value::Call(op, args) => {
            named(data, *op, &READS) && args.iter().all(|a| plain_read(data, a))
        }
        _ => false,
    }
}

/// Can evaluating `v` change what a plain read answers?  Every call must be a pure
/// operator; any other node that is not a value is refused.
pub(crate) fn pure(data: &Data, v: &Value) -> bool {
    match v {
        Value::Span(s) => pure(data, &s.1),
        Value::Var(_)
        | Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Null => true,
        Value::Call(op, args) => named(data, *op, &PURE) && args.iter().all(|a| pure(data, a)),
        Value::Block(_) => {
            discharge(v).is_some_and(|(_, read, _, d)| plain_read(data, read) && pure(data, d))
        }
        Value::If(c, t, e) => pure(data, c) && pure(data, t) && pure(data, e),
        _ => false,
    }
}

/// Collect the discharge blocks of `v` that a binding before the statement may replace: not
/// inside an `if` arm or another statement block.
fn collect<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    match v.unspan() {
        Value::Block(_) if discharge(v).is_some() => out.push(v),
        Value::Call(_, args) => args.iter().for_each(|a| collect(a, out)),
        _ => {}
    }
}

/// Two plain reads, their source positions ignored — the two spellings of `v[i]?` in one
/// statement sit at different columns.
fn same_read(a: &Value, b: &Value) -> bool {
    match (a.unspan(), b.unspan()) {
        (Value::Call(x, xa), Value::Call(y, ya)) => {
            x == y && xa.len() == ya.len() && xa.iter().zip(ya).all(|(p, q)| same_read(p, q))
        }
        (p, q) => p == q,
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (discharge(a), discharge(b)) {
        (Some((_, ra, ca, da)), Some((_, rb, cb, db))) => {
            same_read(ra, rb) && ca == cb && same_read(da, db)
        }
        _ => false,
    }
}

/// Bind the repeated reads of one statement's expression; answers how many were bound.
fn bind_statement(
    data: &Data,
    vars: &mut crate::variables::Function,
    scope: u16,
    stmt: &mut Value,
    before: &mut Vec<Value>,
) -> usize {
    // An assignment's value, a `return`'s, or the statement itself as a block's value.
    let expr: &mut Value = match stmt {
        // A source position is transparent: the wrapped statement is the statement.
        Value::Span(s) => return bind_statement(data, vars, scope, &mut s.1, before),
        Value::Set(_, e) | Value::Return(e) => e,
        Value::Call(..) => stmt,
        _ => return 0,
    };
    if !pure(data, expr) {
        return 0;
    }
    let mut found: Vec<&Value> = Vec::new();
    collect(expr, &mut found);
    let mut groups: Vec<(Value, usize)> = Vec::new();
    for f in &found {
        match groups.iter_mut().find(|(g, _)| same(g, f)) {
            Some((_, n)) => *n += 1,
            None => groups.push(((*f).clone(), 1)),
        }
    }
    let mut bound = 0;
    for (block, n) in groups {
        if n < 2 {
            continue;
        }
        let tp = vars
            .tp(discharge(&block).expect("a discharge block").0)
            .clone();
        let t = vars.add_unique("same_read", &tp, scope);
        replace(expr, &block, t);
        before.push(Value::Set(t, Box::new(block)));
        bound += 1;
    }
    bound
}

fn replace(v: &mut Value, block: &Value, t: u16) {
    if same(v, block) {
        *v = Value::Var(t);
        return;
    }
    if let Value::Call(_, args) = v {
        for a in args.iter_mut() {
            replace(a, block, t);
        }
    } else if let Value::Span(s) = v {
        replace(&mut s.1, block, t);
    }
}

/// Does `v` LEAVE — a `return`, `break` or `continue`, or a block whose last statement does?
fn leaves(v: &Value) -> bool {
    match v.unspan() {
        Value::Return(_) | Value::Break(_) | Value::Continue(_) => true,
        Value::Block(b) => b
            .operators
            .iter()
            .rev()
            .find(|o| !matches!(o, Value::Line(_)))
            .is_some_and(leaves),
        _ => false,
    }
}

/// An arm that runs nothing: absent, or an empty block.
fn empty_arm(v: &Value) -> bool {
    match v.unspan() {
        Value::Null => true,
        Value::Block(b) => b.operators.iter().all(|o| matches!(o, Value::Line(_))),
        _ => false,
    }
}

/// The pure computations of `v` evaluated on every evaluation of `v` — not inside an `if`
/// arm or a block, so not under a short-circuit — outermost first: each pure call, before
/// the calls inside it.
fn computed<'a>(data: &Data, v: &'a Value, out: &mut Vec<&'a Value>) {
    if let Value::Call(_, args) = v.unspan() {
        // A variable and a literal is a counter's step (`i = i - 1`), a spelling the
        // counter rewrites (R-Counter, R-GuardedChain) and the range proofs recognise; it
        // costs one operation, and stays.
        let step = args.len() == 2
            && args.iter().any(|a| matches!(a.unspan(), Value::Var(_)))
            && args
                .iter()
                .any(|a| matches!(a.unspan(), Value::Int(_) | Value::Long(_)));
        if pure(data, v) && !step && args.iter().any(|a| !matches!(a.unspan(), Value::Int(_))) {
            out.push(v);
        }
        for a in args {
            computed(data, a, out);
        }
    }
}

/// Does `v` evaluate `e` on every evaluation (outside `if` arms and blocks)?
fn evaluates(v: &Value, e: &Value) -> bool {
    if same_read(v, e) {
        return true;
    }
    match v.unspan() {
        Value::Call(_, args) => args.iter().any(|a| evaluates(a, e)),
        _ => false,
    }
}

fn replace_expr(v: &mut Value, e: &Value, t: u16) -> usize {
    if same_read(v, e) {
        *v = Value::Var(t);
        return 1;
    }
    match v {
        Value::Call(_, args) => args.iter_mut().map(|a| replace_expr(a, e, t)).sum(),
        Value::Span(s) => replace_expr(&mut s.1, e, t),
        _ => 0,
    }
}

/// THE GUARD CLAUSE over one block's statements: per `if` whose arm leaves, the pure
/// computations of its condition the next statement's assigned value repeats, bound once
/// before the `if`.  A computation with a scalar result only — the local holds a value.
fn bind_guards(
    data: &Data,
    vars: &mut crate::variables::Function,
    scope: u16,
    ops: &[Value],
) -> (Vec<Value>, usize) {
    let mut out: Vec<Value> = Vec::with_capacity(ops.len());
    let mut bound = 0;
    let mut at = 0;
    while at < ops.len() {
        let next = (at + 1..ops.len()).find(|&n| !matches!(ops[n], Value::Line(_)));
        let shared: Vec<Value> = match (ops[at].unspan(), next.map(|n| ops[n].unspan())) {
            (Value::If(cond, then_arm, else_arm), Some(Value::Set(_, val)))
                if leaves(then_arm) && empty_arm(else_arm) =>
            {
                let mut cands = Vec::new();
                computed(data, cond, &mut cands);
                // The largest the next statement repeats: a computation inside one already
                // kept is bound with it.
                let mut kept: Vec<&Value> = Vec::new();
                for cand in cands {
                    if evaluates(val, cand) && !kept.iter().any(|held| evaluates(held, cand)) {
                        kept.push(cand);
                    }
                }
                kept.into_iter().cloned().collect()
            }
            _ => Vec::new(),
        };
        if shared.is_empty() {
            out.push(ops[at].clone());
            at += 1;
            continue;
        }
        let nx = next.expect("a next statement");
        let mut guard = ops[at].clone();
        let mut after: Vec<Value> = ops[at + 1..=nx].to_vec();
        for e in shared {
            let Value::Call(op, _) = e.unspan() else {
                continue;
            };
            let tp = data.def(*op).returned().clone();
            // The local takes the op's declared type whole, a `τ?` wrapper included; the
            // scalar test reads through it.
            if !matches!(
                tp.base(),
                crate::data::Type::Integer(..)
                    | crate::data::Type::Float
                    | crate::data::Type::Single
                    | crate::data::Type::Boolean
            ) {
                continue;
            }
            let t = vars.add_unique("same_read", &tp, scope);
            let in_guard = match &mut guard {
                Value::If(c, _, _) => replace_expr(c, &e, t),
                Value::Span(s) => match &mut s.1 {
                    Value::If(c, _, _) => replace_expr(c, &e, t),
                    _ => 0,
                },
                _ => 0,
            };
            let last = after.last_mut().expect("the next statement");
            let in_next = match last {
                Value::Set(_, v) => replace_expr(v, &e, t),
                Value::Span(s) => match &mut s.1 {
                    Value::Set(_, v) => replace_expr(v, &e, t),
                    _ => 0,
                },
                _ => 0,
            };
            debug_assert!(in_guard > 0 && in_next > 0);
            out.push(Value::Set(t, Box::new(e)));
            bound += 1;
        }
        out.push(guard);
        out.extend(after);
        at = nx + 1;
    }
    (out, bound)
}

/// Bind the repeated reads of every statement of every block in `v`; answers how many.
fn rewrite_in(
    data: &Data,
    vars: &mut crate::variables::Function,
    v: &mut Value,
    fname: &str,
) -> usize {
    let mut n = 0;
    v.for_each_child_mut(&mut |c| n += rewrite_in(data, vars, c, fname));
    let Value::Block(block) = v else {
        return n;
    };
    let Block {
        operators, scope, ..
    } = &mut **block;
    let mut out = Vec::with_capacity(operators.len());
    for mut stmt in std::mem::take(operators) {
        let mut before = Vec::new();
        let k = bind_statement(data, vars, *scope, &mut stmt, &mut before);
        if k > 0 && trace() {
            eprintln!("same-read: {fname} binds {k} read(s) of one statement");
        }
        n += k;
        out.extend(before);
        out.push(stmt);
    }
    let (out, g) = bind_guards(data, vars, *scope, &out);
    if g > 0 && trace() {
        eprintln!(
            "same-read: {fname} binds {g} computation(s) a guard shares with its next statement"
        );
    }
    *operators = out;
    n + g
}

/// Bind the repeated reads of every statement in the program; answers how many.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() {
        return 0;
    }
    let mut total = 0;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        // Two discharged reads (the statement clause) or an `if` (the guard clause).
        let mut found = 0usize;
        let mut guard = false;
        def.code().any_node(&mut |n| {
            if discharge(n).is_some() {
                found += 1;
            }
            guard |= matches!(n, Value::If(..));
            found >= 2 || guard
        });
        if found < 2 && !guard {
            continue;
        }
        let fname = def.name().to_string();
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let mut vars = data.definitions[d as usize].variables.clone();
        let n = rewrite_in(data, &mut vars, &mut code, &fname);
        data.definitions[d as usize].code = code;
        if n > 0 {
            data.definitions[d as usize].variables = vars;
            data.definitions[d as usize].variables.reset_intervals();
            crate::scopes::compute_function_intervals(data, d);
            crate::scopes::assign_function_slots(data, d);
            total += n;
        }
    }
    crate::rewrite_census::fired("R-SameRead", total);
    total
}
