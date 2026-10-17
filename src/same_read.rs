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
//! THE GET-OR-INSERT CLAUSE — `if !c[k] { c += [T{k: k, …}] }` followed by `b = c[k]`: the
//! assignment moves before the `if`, whose condition tests the variable, and the arm reads the
//! key again into it as its last statement: a hit looks the key up once and a miss reads what
//! the plain second lookup would.
//!
//! THE MATCH CLAUSE — a slice `match`'s arm tests each read the subject's length and the
//! elements their patterns test; the length and each integer computation of one element two of
//! them make are bound once, before the first test (`bind_match_reads`).
//!
//! `LOFT_NO_SAME_READ=1` keeps every read; `LOFT_NO_MATCH_READ=1` only the match clause's;
//! `LOFT_TRACE_SAME_READ=1` names each statement it binds.
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

/// Can `v` be evaluated a second time without changing anything?  [`pure`]'s operators,
/// with a discharge block (`x ?? d`) admitted around any such expression rather than only a
/// plain read: the block writes nothing but its own temporary, the same value each time.
/// `@FR-R-RecPtr`'s index clause names an index this way.
pub(crate) fn repeatable(data: &Data, v: &Value) -> bool {
    match v {
        Value::Span(s) => repeatable(data, &s.1),
        Value::Call(op, args) => {
            named(data, *op, &PURE) && args.iter().all(|a| repeatable(data, a))
        }
        Value::Block(_) => discharge(v)
            .is_some_and(|(_, read, _, d)| repeatable(data, read) && repeatable(data, d)),
        Value::If(c, t, e) => repeatable(data, c) && repeatable(data, t) && repeatable(data, e),
        other => pure(data, other),
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

/// THE GET-OR-INSERT CLAUSE's shape: `if !L { … }` with no `else` and a block for its arm,
/// `L` a plain keyed read (`c[k]`, `OpGetRecord` over read operators and locals) — answers
/// `L` and the arm.
fn absent_test<'a>(data: &Data, v: &'a Value) -> Option<(&'a Value, &'a Value)> {
    let Value::If(cond, then_arm, else_arm) = v.unspan() else {
        return None;
    };
    if !empty_arm(else_arm) || !matches!(then_arm.unspan(), Value::Block(_)) {
        return None;
    }
    let Value::Call(not, nargs) = cond.unspan() else {
        return None;
    };
    let [test] = &nargs[..] else { return None };
    let Value::Call(to_bool, cargs) = test.unspan() else {
        return None;
    };
    let [read] = &cargs[..] else { return None };
    (data.def(*not).name() == "OpNot"
        && data.def(*to_bool).name() == "OpConvBoolFromRef"
        && matches!(read.unspan(), Value::Call(op, _) if data.def(*op).name() == "OpGetRecord")
        && plain_read(data, read))
    .then_some((read, then_arm))
}

/// THE GET-OR-INSERT CLAUSE over one block's statements: `if !c[k] { …arm… }` followed by
/// `b = c[k]` looks the key up once on a hit.  The assignment moves before the `if`, the
/// condition tests `b`, and the arm ends by reading `c[k]` into `b` again.  On a hit nothing
/// ran between the two reads, so the first answers what the second would; on a miss the arm's
/// last read IS the second, after everything the arm did.
fn bind_get_or_insert(data: &Data, ops: &[Value]) -> (Vec<Value>, usize) {
    let mut out: Vec<Value> = Vec::with_capacity(ops.len());
    let mut bound = 0;
    let mut at = 0;
    while at < ops.len() {
        let next = (at + 1..ops.len()).find(|&n| !matches!(ops[n], Value::Line(_)));
        let site = match (absent_test(data, &ops[at]), next.map(|n| ops[n].unspan())) {
            (Some((read, arm)), Some(Value::Set(b, val))) if same_read(val, read) => {
                Some((read.clone(), arm.clone(), *b))
            }
            _ => None,
        };
        let Some((read, arm, b)) = site else {
            out.push(ops[at].clone());
            at += 1;
            continue;
        };
        let nx = next.expect("a next statement");
        // The assigned variable itself is bound before the `if`: `b = t` between two record
        // locals is a COPY, so a local of its own would leave `b` no view of the entry.  So
        // `b` must not be read or written by the condition or the arm, which would see it
        // bound early.
        let names_b = |v: &Value| {
            v.any_node(&mut |n| matches!(n, Value::Var(x) | Value::Set(x, _) if *x == b))
        };
        if !matches!(arm.unspan(), Value::Block(_)) || names_b(&arm) || names_b(&ops[at]) {
            out.push(ops[at].clone());
            at += 1;
            continue;
        }
        let mut arm = arm;
        if let Value::Block(bl) = arm.unspan_mut() {
            bl.operators.push(Value::Set(b, Box::new(read.clone())));
        }
        // The `if` and the assignment are edited in place, so their source spans stay.
        let mut guard = ops[at].clone();
        if let Value::If(cond, then_arm, _) = guard.unspan_mut() {
            replace_expr(cond, &read, b);
            **then_arm = arm;
        }
        out.push(ops[nx].clone());
        out.push(guard);
        out.extend(ops[at + 1..nx].iter().cloned());
        bound += 1;
        at = nx + 1;
    }
    (out, bound)
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
    let (out, gi) = bind_get_or_insert(data, &out);
    if gi > 0 && trace() {
        eprintln!("same-read: {fname} binds {gi} get-or-insert lookup(s)");
    }
    let (out, sq) = if sequence_off() {
        (out, 0)
    } else {
        bind_sequences(data, vars, *scope, &out)
    };
    if sq > 0 && trace() {
        eprintln!("same-read: {fname} binds {sq} computation(s) a sequence of statements repeats");
    }
    *operators = out;
    let m = bind_match_reads(data, vars, block);
    if m > 0 && trace() {
        eprintln!("same-read: {fname} binds {m} read(s) a slice match's patterns share");
    }
    n + g + gi + sq + m
}

// ─── THE SEQUENCE CLAUSE ───────────────────────────────────────────────────────────────

fn sequence_off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_SEQUENCE_READ").is_ok_and(|v| v != "0"))
}

/// A text's two counts spelled as stdlib calls (`OpLengthText` and `OpSizeText`, the
/// operator spellings, are pure operators already): each answers from the text alone.
fn text_count(data: &Data, op: u32) -> bool {
    matches!(data.def(op).name(), "t_4text_len" | "t_4text_size")
}

/// [`pure`], with a text's count admitted.
fn seq_pure(data: &Data, v: &Value) -> bool {
    match v.unspan() {
        Value::Call(op, args) if text_count(data, *op) || named(data, *op, &PURE) => {
            args.iter().all(|a| seq_pure(data, a))
        }
        _ => pure(data, v),
    }
}

/// The pure computations of `v` that read a store, outermost first — [`computed`]'s walk,
/// with a text's count admitted.  One that reads nothing but locals is left to the backends,
/// which merge it themselves; a store read they cannot carry across the writes between.
fn seq_computed<'a>(data: &Data, v: &'a Value, out: &mut Vec<&'a Value>) {
    if let Value::Call(_, args) = v.unspan() {
        let step = args.len() == 2
            && args.iter().any(|a| matches!(a.unspan(), Value::Var(_)))
            && args
                .iter()
                .any(|a| matches!(a.unspan(), Value::Int(_) | Value::Long(_)));
        let reads =
            v.any_node(&mut |n| matches!(n, Value::Call(op, _) if named(data, *op, &READS)));
        if seq_pure(data, v) && !step && reads {
            out.push(v);
        }
        for a in args {
            seq_computed(data, a, out);
        }
    }
}

/// The field writes the clause sees through: one value into one field of the record named
/// by the first argument.  A reference write (`OpSetRef`) is not among them.
const FIELD_SETS: [&str; 13] = [
    "OpSetEnum",
    "OpSetBoolean",
    "OpSetInt",
    "OpSetCharacter",
    "OpSetSingle",
    "OpSetFloat",
    "OpSetByte",
    "OpSetByteNullable",
    "OpSetShort",
    "OpSetInt4",
    "OpSetInt4Raw",
    "OpSetShortRaw",
    "OpSetText",
];

/// One statement of a sequence, when the clause can see what it does.
enum Step<'a> {
    /// A source position: nothing.
    Line,
    /// `v = <value>`, the value pure.
    Assign(u16, &'a Value),
    /// A field write into record `e`, its operands pure.
    Fill(u16, &'a [Value]),
    /// `OpFinishRecord(V, e, …)`: record `e` completed in its collection.
    Finish(u16),
}

fn step<'a>(data: &Data, s: &'a Value) -> Option<Step<'a>> {
    match s.unspan() {
        Value::Line(_) => Some(Step::Line),
        Value::Set(v, val) if seq_pure(data, val) => Some(Step::Assign(*v, val)),
        Value::Call(op, args) => {
            let name = data.def(*op).name();
            let var = |i: usize| match args.get(i).map(Value::unspan) {
                Some(Value::Var(e)) => Some(*e),
                _ => None,
            };
            if name == "OpFinishRecord" && args.iter().all(|a| seq_pure(data, a)) {
                return var(1).map(Step::Finish);
            }
            if FIELD_SETS.contains(&name)
                && matches!(args.get(1).map(Value::unspan), Some(Value::Int(_)))
                && args[1..].iter().all(|a| seq_pure(data, a))
            {
                return var(0).map(|e| Step::Fill(e, &args[1..]));
            }
            None
        }
        _ => None,
    }
}

/// The record type a local names, when it names one.
fn record_of(tp: &crate::data::Type) -> Option<u32> {
    match tp.base() {
        crate::data::Type::Reference(d, _) => Some(*d),
        _ => None,
    }
}

/// Is `tp` a value no record write can change — a scalar or a text — answering `true` for
/// these, `false` for anything else?
fn plain_value(tp: &crate::data::Type) -> bool {
    use crate::data::Type;
    matches!(
        tp.base(),
        Type::Integer(..)
            | Type::Boolean
            | Type::Float
            | Type::Single
            | Type::Character
            | Type::Text(_)
    )
}

/// Can no write into a fresh record of type `minted` change what a read through local `l`
/// answers?  `l` holds a plain value, or names a record of ANOTHER type whose fields are all
/// plain values: nothing reached from it is a collection that a record joins, and the record
/// itself is not the fresh one (that has type `minted`).  Any other local answers no.
fn untouched(data: &Data, vars: &crate::variables::Function, l: u16, minted: &[u32]) -> bool {
    let tp = vars.tp(l);
    if plain_value(tp) {
        return true;
    }
    let Some(d) = record_of(tp) else {
        return false;
    };
    !minted.contains(&d)
        && data.def_type(d) == DefType::Struct
        && data
            .def(d)
            .attributes()
            .iter()
            .all(|a| plain_value(&a.typedef))
}

/// THE SEQUENCE CLAUSE over one block's statements: a pure computation that reads a store,
/// evaluated by a statement and again by later ones, is bound once before the first when
/// nothing between can change it.  Between them may stand only assignments of pure values to
/// locals the computation does not read, and writes into a record this block minted
/// (`e = OpNewRecord(…)` earlier in the block, `e` not assigned since), together with its
/// `OpFinishRecord`; every local the computation reads holds a plain value or names a flat
/// record of another type ([`untouched`]).  An assignment to a local the computation reads may
/// be the last statement that uses it: its value is evaluated before the assignment.
/// Moving the evaluation to before the first statement crosses nothing: that statement's
/// other operands are pure, and its own write comes after them.
fn bind_sequences(
    data: &Data,
    vars: &mut crate::variables::Function,
    scope: u16,
    ops: &[Value],
) -> (Vec<Value>, usize) {
    let mut ops = ops.to_vec();
    let mut bindings: Vec<(usize, Value)> = Vec::new();
    // The records minted in this block before each statement, with their types.
    let mut fresh: Vec<(u16, u32)> = Vec::new();
    for i in 0..ops.len() {
        let mut cands: Vec<Value> = Vec::new();
        match step(data, &ops[i]) {
            Some(Step::Assign(v, val)) if !fresh.iter().any(|(e, _)| *e == v) => {
                let mut c = Vec::new();
                seq_computed(data, val, &mut c);
                cands.extend(c.into_iter().cloned());
            }
            Some(Step::Fill(e, rest)) if fresh.iter().any(|(f, _)| *f == e) => {
                let mut c = Vec::new();
                for r in rest {
                    seq_computed(data, r, &mut c);
                }
                cands.extend(c.into_iter().cloned());
            }
            _ => {}
        }
        let minted: Vec<u32> = fresh.iter().map(|(_, d)| *d).collect();
        let mut kept: Vec<Value> = Vec::new();
        for cand in cands {
            if kept.iter().any(|held| evaluates(held, &cand)) {
                continue;
            }
            let Value::Call(op, _) = cand.unspan() else {
                continue;
            };
            let tp = data.def(*op).returned().clone();
            if !matches!(
                tp.base(),
                crate::data::Type::Integer(..)
                    | crate::data::Type::Float
                    | crate::data::Type::Single
                    | crate::data::Type::Boolean
            ) {
                continue;
            }
            let mut reads: Vec<u16> = Vec::new();
            cand.walk(&mut |n| {
                if let Value::Var(l) = n {
                    reads.push(*l);
                }
            });
            if !reads.iter().all(|l| untouched(data, vars, *l, &minted)) {
                continue;
            }
            // The first statement's own write: a local the computation reads ends the run.
            if matches!(step(data, &ops[i]), Some(Step::Assign(v, _)) if reads.contains(&v)) {
                continue;
            }
            let live: Vec<u16> = fresh.iter().map(|(e, _)| *e).collect();
            let users = run_users(data, &ops[i + 1..], &cand, &reads, live);
            if users.is_empty() {
                continue;
            }
            let t = vars.add_unique("same_read", &tp, scope);
            for k in std::iter::once(i).chain(users.into_iter().map(|j| i + 1 + j)) {
                let n = replace_in_step(&mut ops[k], &cand, t);
                debug_assert!(n > 0);
            }
            bindings.push((i, Value::Set(t, Box::new(cand.clone()))));
            kept.push(cand);
        }
        // What this statement leaves minted for the ones after it.
        if let Value::Set(e, val) = ops[i].unspan()
            && matches!(val.unspan(), Value::Call(op, _) if data.def(*op).name() == "OpNewRecord")
            && let Some(d) = record_of(vars.tp(*e))
        {
            fresh.retain(|(f, _)| f != e);
            fresh.push((*e, d));
        } else {
            ops[i].walk(&mut |n| {
                if let Value::Set(v, _) = n {
                    fresh.retain(|(f, _)| f != v);
                }
            });
        }
    }
    let bound = bindings.len();
    let mut out = Vec::with_capacity(ops.len() + bound);
    let mut b = bindings.into_iter().peekable();
    for (i, s) in ops.into_iter().enumerate() {
        while let Some((_, set)) = b.next_if(|(at, _)| *at == i) {
            out.push(set);
        }
        out.push(s);
    }
    (out, bound)
}

/// The statements of `after` that evaluate `cand` again before anything can change it, as
/// indexes into `after`: the run ends at the first statement the clause cannot see through, a
/// field write into a record not in `live` (the block's minted records), or an assignment to a
/// local in `reads` (which may itself be the run's last user).
fn run_users(
    data: &Data,
    after: &[Value],
    cand: &Value,
    reads: &[u16],
    mut live: Vec<u16>,
) -> Vec<usize> {
    let mut users: Vec<usize> = Vec::new();
    for (j, s) in after.iter().enumerate() {
        match step(data, s) {
            Some(Step::Line) => {}
            Some(Step::Assign(v, val)) => {
                if evaluates(val, cand) {
                    users.push(j);
                }
                if reads.contains(&v) {
                    break;
                }
                live.retain(|e| *e != v);
            }
            Some(Step::Fill(e, rest)) if live.contains(&e) => {
                if rest.iter().any(|r| evaluates(r, cand)) {
                    users.push(j);
                }
            }
            Some(Step::Finish(e)) if live.contains(&e) => {}
            _ => break,
        }
    }
    users
}

/// Replace `e` by local `t` in the values of statement `s` (an assignment's value, a field
/// write's operands); answers how many.
fn replace_in_step(s: &mut Value, e: &Value, t: u16) -> usize {
    match s {
        Value::Span(b) => replace_in_step(&mut b.1, e, t),
        Value::Set(_, v) => replace_expr(v, e, t),
        Value::Call(_, args) => args[1..].iter_mut().map(|a| replace_expr(a, e, t)).sum(),
        _ => 0,
    }
}

// ─── THE MATCH CLAUSE ──────────────────────────────────────────────────────────────────

fn match_off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_MATCH_READ").is_ok_and(|v| v != "0"))
}

/// A length a pattern test proves: `base + n <= len(subject)`, `base` the cursor's position
/// local or `None` for a plain subject.
type Proved = (Option<u16>, i32);

/// The element read `subject[base + k]` (`OpGetVector(subject, size, index)` with the index
/// `k` or `base + k`, `k >= 0`), as its `(base, k)`.
fn elem_read(data: &Data, v: &Value, subject: u16) -> Option<Proved> {
    let Value::Call(op, args) = v.unspan() else {
        return None;
    };
    let [s, Value::Int(_), idx] = args.as_slice() else {
        return None;
    };
    if data.def(*op).name() != "OpGetVector"
        || !matches!(s.unspan(), Value::Var(x) if *x == subject)
    {
        return None;
    }
    index_of(data, idx)
}

/// `k` or `base + k` with `k >= 0`, as `(base, k)`.
fn index_of(data: &Data, idx: &Value) -> Option<Proved> {
    match idx.unspan() {
        Value::Int(k) if *k >= 0 => Some((None, *k)),
        Value::Call(op, a) if data.def(*op).name() == "OpAddInt" => match a.as_slice() {
            [b, k] => match (b.unspan(), k.unspan()) {
                (Value::Var(b), Value::Int(k)) if *k >= 0 => Some((Some(*b), *k)),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

/// Is `v` `len(subject)`?
fn is_len(data: &Data, v: &Value, subject: u16) -> bool {
    matches!(v.unspan(), Value::Call(op, a) if data.def(*op).name() == "OpLengthVector"
        && matches!(a.as_slice(), [s] if matches!(s.unspan(), Value::Var(x) if *x == subject)))
}

/// The lengths a TRUE `c` proves: a length test `base + n <= len` (or `n == len`), and both
/// halves of a short-circuit `if a { b } else { false }`.
fn proves(data: &Data, c: &Value, subject: u16, out: &mut Vec<Proved>) {
    match c.unspan() {
        Value::If(a, b, f) if matches!(f.unspan(), Value::Boolean(false)) => {
            proves(data, a, subject, out);
            proves(data, b, subject, out);
        }
        Value::Call(op, a) if a.len() == 2 => {
            let name = data.def(*op).name();
            if name == "OpLeInt" && is_len(data, &a[1], subject) {
                out.extend(index_of(data, &a[0]));
            } else if name == "OpEqInt" {
                for (l, n) in [(&a[0], &a[1]), (&a[1], &a[0])] {
                    if is_len(data, l, subject)
                        && let Value::Int(n) = n.unspan()
                    {
                        out.push((None, *n));
                    }
                }
            }
        }
        _ => {}
    }
}

/// The one element `v` reads, when every element read inside it is the same one.
fn one_elem(data: &Data, v: &Value, subject: u16) -> Option<Proved> {
    let mut seen: Option<Proved> = None;
    let mut other = false;
    v.any_node(&mut |n| {
        if let Value::Call(..) = n
            && let Some(e) = elem_read(data, n, subject)
        {
            other |= seen.is_some_and(|s| s != e);
            seen = Some(e);
        }
        other
    });
    if other { None } else { seen }
}

/// Visit the outermost integer computations of a single element in `v`, each with whether
/// the tests enclosing it prove the element in range.  `f` answers `Some(t)` to replace
/// the computation with the local `t`.
fn visit(
    data: &Data,
    node: &mut Value,
    subject: u16,
    facts: &mut Vec<Proved>,
    seen: &mut dyn FnMut(&Value, Proved, bool) -> Option<u16>,
) {
    match node {
        Value::Span(s) => visit(data, &mut s.1, subject, facts, seen),
        Value::If(test, then_arm, else_arm) => {
            visit(data, test, subject, facts, seen);
            let at = facts.len();
            proves(data, test, subject, facts);
            visit(data, then_arm, subject, facts, seen);
            facts.truncate(at);
            visit(data, else_arm, subject, facts, seen);
        }
        Value::Call(op, _) => {
            let op = *op;
            if matches!(data.def(op).returned.base(), crate::data::Type::Integer(..))
                && let Some((base, k)) = one_elem(data, node, subject)
            {
                let inside = facts.iter().any(|(b, n)| *b == base && *n > k);
                if let Some(local) = seen(node, (base, k), inside) {
                    *node = Value::Var(local);
                }
                return;
            }
            if let Value::Call(_, args) = node {
                for arg in args.iter_mut() {
                    visit(data, arg, subject, facts, seen);
                }
            }
        }
        _ => {}
    }
}

/// The arm tests of a chain: the condition of each `if` down its else-spine.
fn arm_tests(chain: &mut Value) -> Vec<&mut Value> {
    let mut tests = Vec::new();
    let mut at = chain;
    loop {
        match at {
            Value::Span(s) => at = &mut s.1,
            Value::If(c, _, e) => {
                tests.push(&mut **c);
                at = &mut **e;
            }
            _ => break,
        }
    }
    tests
}

/// The statements that bind the length (when two tests read it) and each repeated element
/// computation under its own length test; answers them with the length's local and one
/// local per group, in `groups`' order.
fn match_bindings(
    data: &Data,
    vars: &mut crate::variables::Function,
    scope: u16,
    subject: u16,
    groups: &[(Value, Proved, usize)],
    lens: usize,
) -> (Vec<Value>, Option<u16>, Vec<u16>) {
    let int_tp = data.def(data.def_nr("OpLengthVector")).returned.clone();
    let len_call = Value::Call(data.def_nr("OpLengthVector"), vec![Value::Var(subject)]);
    let mut before = Vec::new();
    let len_val = if lens >= 2 {
        let l = vars.add_unique("match_len", &int_tp, scope);
        before.push(Value::Set(l, Box::new(len_call.clone())));
        Some(l)
    } else {
        None
    };
    let mut temps = Vec::new();
    for (e, (base, k), _) in groups {
        let need = match base {
            Some(b) => Value::Call(
                data.def_nr("OpAddInt"),
                vec![Value::Var(*b), Value::Int(k + 1)],
            ),
            None => Value::Int(k + 1),
        };
        let len = len_val.map_or_else(|| len_call.clone(), Value::Var);
        let test = Value::Call(data.def_nr("OpLeInt"), vec![need, len]);
        let tp = match e {
            Value::Call(op, _) => data.def(*op).returned.clone(),
            _ => int_tp.clone(),
        };
        let t = vars.add_unique("match_read", &tp, scope);
        before.push(Value::Set(
            t,
            Box::new(Value::If(
                Box::new(test),
                Box::new(e.clone()),
                Box::new(Value::Int(0)),
            )),
        ));
        temps.push(t);
    }
    (before, len_val, temps)
}

/// THE MATCH CLAUSE — a slice `match`'s arms each test the subject again: its length, then
/// the element at each position the pattern names (`[NeedVal { idx }, Pos { word }]` reads
/// element 0's variant in every arm that starts with a variant test).  The arm tests run one
/// after another with nothing between them, so a read two of them make is one read: the
/// length and each integer computation of one element (a variant number, a scalar
/// element) are bound once, before the first test.
///
/// Only the chain's own tests change — never an arm's body, its bindings or its guard: a
/// guarded arm runs its bindings and the guard inside its own branch, and the tests after a
/// failed guard run there too, unchanged.  Every test must be pure, so nothing between two
/// of them writes a local or a store.  An element read is bound under `base + k + 1 <= len`,
/// and replaced only where the tests enclosing it prove that much (`base + n <= len` with
/// `n > k`): there the bound value IS the value the read answers, and a read the plain
/// chain never makes — one past the end, which would raise — is never made.
fn bind_match_reads(
    data: &Data,
    vars: &mut crate::variables::Function,
    block: &mut Block,
) -> usize {
    if match_off() || block.name != "vector_match" {
        return 0;
    }
    let live: Vec<usize> = (0..block.operators.len())
        .filter(|i| !matches!(block.operators[*i], Value::Line(_)))
        .collect();
    // The chain is the block's last `if`: the frees of the arms' text bindings follow it.
    let (Some(&first), Some(&last)) = (
        live.first(),
        live.iter()
            .rev()
            .find(|i| matches!(block.operators[**i].unspan(), Value::If(..))),
    ) else {
        return 0;
    };
    let Value::Set(subject, _) = block.operators[first].unspan() else {
        return 0;
    };
    let subject = *subject;
    if first == last {
        return 0;
    }
    let mut tests = arm_tests(&mut block.operators[last]);
    if tests.len() < 2 || !tests.iter().all(|t| pure(data, t)) {
        return 0;
    }
    // Count each computation where its element is proved in range.
    let mut groups: Vec<(Value, Proved, usize)> = Vec::new();
    for t in &mut tests {
        visit(data, t, subject, &mut Vec::new(), &mut |e, at, inside| {
            if inside {
                match groups.iter_mut().find(|(g, _, _)| same_read(g, e)) {
                    Some((_, _, n)) => *n += 1,
                    None => groups.push((e.clone(), at, 1)),
                }
            }
            None
        });
    }
    groups.retain(|(_, _, n)| *n >= 2);
    let lens: usize = tests
        .iter()
        .map(|t| {
            let mut n = 0;
            t.any_node(&mut |x| {
                n += usize::from(is_len(data, x, subject));
                false
            });
            n
        })
        .sum();
    if groups.is_empty() && lens < 2 {
        return 0;
    }
    let (before, len_val, temps) = match_bindings(data, vars, block.scope, subject, &groups, lens);
    for t in &mut tests {
        visit(data, t, subject, &mut Vec::new(), &mut |e, _, inside| {
            if !inside {
                return None;
            }
            groups
                .iter()
                .position(|(g, _, _)| same_read(g, e))
                .map(|i| temps[i])
        });
        if let Some(l) = len_val {
            replace_len(data, t, subject, l);
        }
    }
    let bound = before.len();
    let tail = block.operators.split_off(last);
    block.operators.extend(before);
    block.operators.extend(tail);
    bound
}

fn replace_len(data: &Data, v: &mut Value, subject: u16, l: u16) {
    if is_len(data, v, subject) {
        *v = Value::Var(l);
        return;
    }
    v.for_each_child_mut(&mut |c| replace_len(data, c, subject, l));
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
        // Two discharged reads (the statement clause), an `if` (the guard clause) or two
        // store reads (the sequence clause).
        let mut found = 0usize;
        let mut reads = 0usize;
        let mut guard = false;
        def.code().any_node(&mut |n| {
            if discharge(n).is_some() {
                found += 1;
            }
            if matches!(n, Value::Call(op, _) if named(data, *op, &READS)) {
                reads += 1;
            }
            guard |= matches!(n, Value::If(..));
            found >= 2 || guard || reads >= 2
        });
        if found < 2 && !guard && reads < 2 {
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
