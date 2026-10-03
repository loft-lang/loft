// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `(R-LiteralAppend)` — a record literal bound to a local whose one use is the append of it
//! is built in its element.
//!
//! `tri = Triangle { a: …, b: …, c: … }; m.triangles += [tri]` mints a store for `tri`,
//! writes its fields, mints the vector's element, copies `tri` into it and frees `tri`'s store
//! — six times per hex in moros' `emit_hex_surface`.  `m.triangles += [Triangle { … }]`
//! already builds in the element.  The rewrite gives the bound spelling the same IR: the
//! element is minted first and the literal's field writes go to it, then the finish.
//!
//! Admission, per statement list: `L = null; OpDatabase(L, tp); OpSet<scalar>(L, off, v)…;
//! E = OpNewRecord(V, ptp, fld); OpCopyRecord(L, E, _); OpFinishRecord(V, E, ptp, fld);
//! OpFreeRef(L)` as consecutive statements, `L` named nowhere else in the function, every
//! field write a scalar setter, and every value `v` a SIMPLE value — a literal, a scalar
//! variable, or a store-free scalar op over those.  The last condition is what makes the
//! reorder sound: the values now run AFTER the element's mint, which may grow `V`'s store and
//! move its records, so a value that read through a record could read a moved one.  A value
//! that reads no record cannot.  Every decline keeps the copy.  `LOFT_NO_LITERAL_APPEND=1` is
//! the switch; `LOFT_TRACE_LITERAL_APPEND=1` names each rewrite.  Both backends: decided in
//! the scope pass on the settled IR.
use crate::data::{Data, DefType, Type, Value};

/// Rewrite every admitted literal-append in function `d_nr`; a no-op under the switch.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !crate::keys::literal_append_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    if !data.def(d_nr).is_loft_defined() {
        return;
    }
    let trace = std::env::var("LOFT_TRACE_LITERAL_APPEND").is_ok();
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    // The candidates and the function-wide mention counts are taken on the code as it stands.
    let mut sites: Vec<(u16, usize)> = Vec::new(); // (L, statements in its run)
    {
        let vars = data.def(d_nr).variables();
        code.walk(&mut |n| {
            if let Value::Block(bl) | Value::Loop(bl) = n {
                for at in 0..bl.operators.len() {
                    if matches!(bl.operators[at].unspan(), Value::Line(_)) {
                        continue;
                    }
                    if let Some((l, len)) = admit(&bl.operators[at..], data, vars) {
                        sites.push((l, len));
                    }
                }
            }
        });
        sites.retain(|&(l, len)| mentions(&code, l) == len_mentions(len));
    }
    let mut fired = 0usize;
    for (l, _) in &sites {
        code.map_nodes(&mut |n| {
            let (Value::Block(bl) | Value::Loop(bl)) = n else {
                return;
            };
            let ops = &mut bl.operators;
            let mut at = 0;
            while at < ops.len() {
                if starts_run(&ops[at], *l) {
                    apply(ops, at, *l);
                    fired += 1;
                }
                at += 1;
            }
        });
        if trace {
            let vars = data.def(d_nr).variables();
            eprintln!(
                "literal-append: {} builds `{}` in its element",
                data.def(d_nr).name(),
                vars.name(*l)
            );
        }
    }
    data.definitions[d_nr as usize].code = code;
    crate::rewrite_census::fired("R-LiteralAppend", fired);
}

/// How often `x` is named in `v`, as a variable or a `Set` target.
fn mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |c| {
        if matches!(c, Value::Var(y) if *y == x) || matches!(c, Value::Set(y, _) if *y == x) {
            n += 1;
        }
    });
    n
}

/// The mentions of `L` a run of `len` statements makes: the null init, the mint, one per
/// field write, the copy's source and the free.
fn len_mentions(len: usize) -> usize {
    // `len` = null init + mint + k writes + new + copy + finish + free = k + 6;
    // L is named by the init, the mint, each write, the copy and the free: k + 4.
    len - 2
}

fn named<'a>(v: &'a Value, data: &Data, name: &str) -> Option<&'a [Value]> {
    match v.unspan() {
        Value::Call(d, a) if (*d as usize) < data.definitions.len() && data.def(*d).name() == name => {
            Some(a)
        }
        _ => None,
    }
}

fn is_var(v: Option<&Value>, x: u16) -> bool {
    matches!(v.map(Value::unspan), Some(Value::Var(y)) if *y == x)
}

const SCALAR_SETTERS: [&str; 6] = [
    "OpSetInt",
    "OpSetFloat",
    "OpSetSingle",
    "OpSetInt4",
    "OpSetBoolean",
    "OpSetCharacter",
];

fn scalar(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Integer(_) | Type::Boolean | Type::Float | Type::Single | Type::Character
    )
}

/// A literal, a scalar variable, or a native scalar op over those: reads no record.
fn simple(v: &Value, data: &Data, vars: &crate::variables::Function) -> bool {
    match v.unspan() {
        Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_) => true,
        Value::Var(x) => *x < vars.count() && scalar(vars.tp(*x)),
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let def = data.def(*d);
            matches!(def.code(), Value::Null)
                && def.name().starts_with("Op")
                && !def.attributes().is_empty()
                && def.attributes().iter().all(|a| !a.constant && scalar(&a.typedef))
                && scalar(def.returned())
                && args.iter().all(|a| simple(a, data, vars))
        }
        _ => false,
    }
}

/// Is `ops` the start of an admissible run for some `L`?  Answers `L` and the run's length.
fn admit(all: &[Value], data: &Data, vars: &crate::variables::Function) -> Option<(u16, usize)> {
    // `Line` markers stand between statements and are not statements.
    let ops: Vec<&Value> = all
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let Value::Set(l, init) = ops.first()?.unspan() else {
        return None;
    };
    let l = *l;
    if !matches!(init.unspan(), Value::Null)
        || vars.is_argument(l)
        || vars.is_captured(l)
        || !matches!(vars.tp(l), Type::Reference(_, _))
    {
        return None;
    }
    let mint = named(ops.get(1).copied()?, data, "OpDatabase")?;
    if !is_var(mint.first(), l) {
        return None;
    }
    let mut k = 2;
    while let Some(op) = ops.get(k) {
        let Value::Call(d, a) = op.unspan() else { break };
        if !SCALAR_SETTERS.contains(&data.def(*d).name()) || !is_var(a.first(), l) {
            break;
        }
        if !matches!(a.get(1).map(Value::unspan), Some(Value::Int(_)))
            || !a.get(2).is_some_and(|v| simple(v, data, vars))
        {
            return None;
        }
        k += 1;
    }
    let Value::Set(e, rhs) = ops.get(k).copied()?.unspan() else {
        return None;
    };
    let new = named(rhs, data, "OpNewRecord")?;
    let copy = named(ops.get(k + 1).copied()?, data, "OpCopyRecord")?;
    if !is_var(copy.first(), l) || !is_var(copy.get(1), *e) {
        return None;
    }
    let finish = named(ops.get(k + 2).copied()?, data, "OpFinishRecord")?;
    if !is_var(finish.get(1), *e) || finish.first() != new.first() {
        return None;
    }
    let free = named(ops.get(k + 3).copied()?, data, "OpFreeRef")?;
    if !is_var(free.first(), l) {
        return None;
    }
    // The values must not name the element or the vector either (they run after the mint).
    for op in ops[2..k].iter().copied() {
        if let Value::Call(_, a) = op.unspan()
            && (mentions(&a[2], *e) > 0
                || new.first().is_some_and(|v| matches!(v.unspan(), Value::Var(x) if mentions(&a[2], *x) > 0)))
        {
            return None;
        }
    }
    Some((l, k + 4))
}

fn starts_run(op: &Value, l: u16) -> bool {
    matches!(op.unspan(), Value::Set(x, init) if *x == l && matches!(init.unspan(), Value::Null))
}

/// `L = null; mint(L); set(L,…)…; E = new; copy(L, E); finish; free(L)` →
/// `E = new; set(E,…)…; finish`, the `Line` markers of the run kept in front of it.
fn apply(ops: &mut Vec<Value>, at: usize, l: u16) {
    let idx: Vec<usize> = (at..ops.len())
        .filter(|&i| !matches!(ops[i].unspan(), Value::Line(_)))
        .collect();
    // The writes are the calls on L after the mint.
    let mut k = 2;
    while matches!(idx.get(k).map(|&i| ops[i].unspan()), Some(Value::Call(_, a)) if is_var(a.first(), l)) {
        k += 1;
    }
    let Some(Value::Set(e, _)) = idx.get(k).map(|&i| ops[i].unspan()) else {
        return;
    };
    let e = *e;
    let last = idx[k + 3];
    let new = ops[idx[k]].clone();
    let finish = ops[idx[k + 2]].clone();
    let mut writes: Vec<Value> = idx[2..k].iter().map(|&i| ops[i].clone()).collect();
    for w in &mut writes {
        if let Value::Call(_, a) = w.unspan_mut()
            && let Some(first) = a.first_mut()
        {
            *first = Value::Var(e);
        }
    }
    let lines: Vec<Value> = (at..=last)
        .filter(|&i| matches!(ops[i].unspan(), Value::Line(_)))
        .map(|i| ops[i].clone())
        .collect();
    let mut run = lines;
    run.push(new);
    run.extend(writes);
    run.push(finish);
    ops.splice(at..=last, run);
}
