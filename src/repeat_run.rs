// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-RepeatRun` — a run of the same scalar constant pushed one element at a time is the
//! repeat literal.
//!
//! `Mat4 { m: [0.0, 0.0, …, 0.0] }` — sixteen zeros spelled out, the way a matrix library
//! writes its zero result — lowers to sixteen `OpPushFloat(target, 0.0)` statements: sixteen
//! appends, each a capacity test and a length bump, and on a fresh vector a growth step
//! partway.  `[0.0; 16]` lowers to ONE push of the template and `OpAppendCopy(target, 16, tp)`,
//! which grows once and fills by doubling block copies.  The two build the same vector:
//! `OpAppendCopy` takes the TOTAL the run asks for and copies the vector's LAST element — the
//! template the first push just appended — `count - 1` more times, leaving every element
//! appended before the run where it was.
//!
//! Recognised, on the IR the parser lowered: four or more consecutive statements that are the
//! SAME push of the SAME literal into the SAME target — `OpPushFloat`, `OpPushInt`,
//! `OpPushSingle` or `OpPushInt4` (an element of at least four bytes, whose vector type the
//! target names), the literal compared by its bits (`-0.0` is not `0.0`), the target a local
//! or a field path over one with literal operands, so evaluating it once reads what each push
//! read.  A shorter run, a narrower element, a computed value or target keep the pushes: a
//! wrong decline is the appends the program already pays, a wrong admission would be a
//! different vector.  The template's push is preceded by `OpPreAllocVector(target, count,
//! size)`, which claims the run's whole width for an ABSENT vector and does nothing to one
//! that exists, so a literal's fresh vector is claimed once and never grown.
//!
//! `LOFT_NO_REPEAT_RUN=1` keeps every run.
use crate::compact::int;
use crate::data::{Data, DefType, Type, Value};

/// The shortest run worth one fill: below it the fill's own growth and copy cost more than
/// the pushes it replaces.
const MIN_RUN: usize = 4;

/// The push ops whose element is at least four bytes, with that element's size.
const PUSHES: [(&str, i32); 4] = [
    ("OpPushFloat", 8),
    ("OpPushInt", 8),
    ("OpPushSingle", 4),
    ("OpPushInt4", 4),
];

/// Rewrite every admitted run in `d_nr`; a no-op under the switch, and a cheap one for a body
/// that pushes no constant.
pub fn rewrite(data: &mut Data, database: &mut crate::database::Stores, d_nr: u32) {
    if !crate::keys::repeat_run_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    let pushes: [u32; 4] = PUSHES.map(|(n, _)| data.def_nr(n));
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    if code.any_node(&mut |n| matches!(n, Value::Call(d, _) if pushes.contains(d))) {
        let cx = Cx {
            data,
            d_nr,
            pushes,
            get_field: data.def_nr("OpGetField"),
            append_copy: data.def_nr("OpAppendCopy"),
            pre_alloc: data.def_nr("OpPreAllocVector"),
        };
        // The vector type each run names, read before the walk mutates: a local's needs the
        // store registry, the walk only the definitions.
        let mut tps = Vec::new();
        collect_types(&code, &cx, database, &mut tps);
        visit(&mut code, &cx, &tps);
    }
    data.definitions[d_nr as usize].code = code;
}

struct Cx<'a> {
    data: &'a Data,
    d_nr: u32,
    pushes: [u32; 4],
    get_field: u32,
    append_copy: u32,
    pre_alloc: u32,
}

/// `(push op, target, literal)` of one statement, when it is a push of a literal into a
/// target the rule can evaluate once.
fn push_of<'v>(v: &'v Value, cx: &Cx) -> Option<(u32, &'v Value, &'v Value)> {
    let Value::Call(d, args) = v.unspan() else {
        return None;
    };
    if !cx.pushes.contains(d) || args.len() != 2 {
        return None;
    }
    let lit = args[1].unspan();
    if !matches!(
        lit,
        Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_)
    ) {
        return None;
    }
    pure_target(&args[0], cx).then_some((*d, args[0].unspan(), lit))
}

/// A local, or a field path over one with literal operands.
fn pure_target(v: &Value, cx: &Cx) -> bool {
    match v.unspan() {
        Value::Var(_) => true,
        Value::Call(d, a) if *d == cx.get_field && a.len() == 3 => {
            int(&a[1]).is_some() && int(&a[2]).is_some() && pure_target(&a[0], cx)
        }
        _ => false,
    }
}

/// The same literal, compared by its bits so a float's sign and NaN payload count.
fn same_literal(a: &Value, b: &Value) -> bool {
    match (a.unspan(), b.unspan()) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Long(x), Value::Long(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x.to_bits() == y.to_bits(),
        (Value::Single(x), Value::Single(y)) => x.to_bits() == y.to_bits(),
        _ => false,
    }
}

/// The runs of a statement list: `(first index, last index, count)` over its significant
/// statements, each run four or more pushes of one literal into one target.
fn runs(ops: &[Value], cx: &Cx) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut cur: Option<(usize, usize, usize)> = None;
    let mut prev: Option<(u32, &Value, &Value)> = None;
    for (i, op) in ops.iter().enumerate() {
        if matches!(op.unspan(), Value::Line(_) | Value::Null) {
            continue;
        }
        let this = push_of(op, cx);
        let extends = match (this, prev) {
            (Some((d, t, l)), Some((pd, pt, pl))) => d == pd && t == pt && same_literal(l, pl),
            _ => false,
        };
        if extends {
            if let Some(run) = &mut cur {
                run.1 = i;
                run.2 += 1;
            }
        } else {
            if let Some(run) = cur.take()
                && run.2 >= MIN_RUN
            {
                out.push(run);
            }
            cur = this.map(|_| (i, i, 1));
        }
        prev = this;
    }
    if let Some(run) = cur
        && run.2 >= MIN_RUN
    {
        out.push(run);
    }
    out
}

/// The vector type `OpAppendCopy` names for a target: a field's own type, or a local's
/// vector type from the registry.
fn vector_type(target: &Value, cx: &Cx, database: &mut crate::database::Stores) -> Option<i32> {
    match target.unspan() {
        Value::Call(d, a) if *d == cx.get_field => int(&a[2]),
        Value::Var(v) => {
            let Type::Vector(elem, _) = cx.data.def(cx.d_nr).variables().tp(*v).base() else {
                return None;
            };
            let elem_tp = cx.data.vector_element_type(elem, database)?;
            Some(i32::from(database.vector(elem_tp)))
        }
        _ => None,
    }
}

fn collect_types(
    v: &Value,
    cx: &Cx,
    database: &mut crate::database::Stores,
    out: &mut Vec<(Value, i32)>,
) {
    let lists: Option<&[Value]> = match v.unspan() {
        Value::Block(bl) | Value::Loop(bl) => Some(&bl.operators),
        Value::Insert(ls) => Some(ls),
        _ => None,
    };
    if let Some(ops) = lists {
        for (first, _, _) in runs(ops, cx) {
            let (_, target, _) = push_of(&ops[first], cx).expect("a run starts with a push");
            if !out.iter().any(|(t, _)| t == target)
                && let Some(tp) = vector_type(target, cx, database)
            {
                out.push((target.clone(), tp));
            }
        }
    }
    v.for_each_child(&mut |c| collect_types(c, cx, database, out));
}

fn visit(v: &mut Value, cx: &Cx, tps: &[(Value, i32)]) {
    let lists: Option<&mut Vec<Value>> = match v.unspan_mut() {
        Value::Block(bl) | Value::Loop(bl) => Some(&mut bl.operators),
        Value::Insert(ls) => Some(ls),
        _ => None,
    };
    if let Some(ops) = lists {
        for (first, last, count) in runs(ops, cx) {
            let (op, target, _) = push_of(&ops[first], cx).expect("a run starts with a push");
            let size = PUSHES[cx.pushes.iter().position(|p| *p == op).expect("a push op")].1;
            let target = target.clone();
            let Some(&(_, tp)) = tps.iter().find(|(t, _)| *t == target) else {
                continue;
            };
            crate::rewrite_census::fired("R-RepeatRun", 1);
            if crate::keys::trace_repeat_run() {
                eprintln!(
                    "[repeat-run] fn={} ADMITTED: {count} equal pushes are one fill",
                    cx.data.def(cx.d_nr).name()
                );
            }
            // The template is preceded by the reservation, which claims the run's whole width
            // for an ABSENT vector and leaves one that exists alone, so the fill never grows
            // what the template's push just claimed.  Every later push of the run becomes
            // nothing, the second the fill that writes them.
            let total = i32::try_from(count).unwrap_or(i32::MAX);
            // The reservation's width is a `u16`; a longer run fills without it.
            if u16::try_from(count).is_ok() {
                let template = std::mem::replace(&mut ops[first], Value::Null);
                ops[first] = Value::Insert(vec![
                    Value::Call(
                        cx.pre_alloc,
                        vec![target.clone(), Value::Int(total), Value::Int(size)],
                    ),
                    template,
                ]);
            }
            let mut fill = Some(Value::Call(
                cx.append_copy,
                vec![target, Value::Int(total), Value::Int(tp)],
            ));
            for op in &mut ops[first + 1..=last] {
                if matches!(op.unspan(), Value::Line(_) | Value::Null) {
                    continue;
                }
                *op = fill.take().unwrap_or(Value::Null);
            }
        }
    }
    v.for_each_child_mut(&mut |c| visit(c, cx, tps));
}
