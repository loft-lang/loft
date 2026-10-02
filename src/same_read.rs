// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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
fn pure(data: &Data, v: &Value) -> bool {
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
    *operators = out;
    n
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
        let mut found = 0usize;
        def.code().any_node(&mut |n| {
            if discharge(n).is_some() {
                found += 1;
            }
            found >= 2
        });
        if found < 2 {
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
