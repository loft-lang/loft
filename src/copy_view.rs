// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-CopyView` — a read-only copy of a record nothing can disturb is a view of it.
//!
//! `t = a` of a record, and `t = if c { a } else { b }`, COPY (`@FR-B-Copy`): each binding
//! mints a store, copies the record into it and frees it at scope exit.  Where `t` is only
//! read, its sources view records the caller owns, and nothing in `t`'s live range can change
//! those records' bytes, move them or free them, no program can tell the copy from a view —
//! and a view is a DbRef copy.  Measured on graphics' `polygon_crossings`, which picks an
//! edge's top and bottom vertex that way per crossing per scanline: `fill_polygon` 6.62 →
//! 2.86 ms per op (hand-priced, hash unchanged).
//!
//! Decided after the scope pass on the settled IR, beside `@FR-R-Const`, and spelled IN the
//! IR so both backends read it without a flag: a plain bind becomes the projection
//! `t = OpGetField(a, 0)` — the node `s = o.inner` already is — and a join's arms lose their
//! `__lift_N` copies and answer the source itself; `t`'s deps name its sources, and the frees
//! of what is no longer minted go.
//!
//! The disturbance question is asked of TYPES: an operation reaches only the records its
//! operands' types can hold, so a push into a `vector<integer>` cannot move a `Coord`
//! whatever store it lives in.  An operand whose type is unknown (a projection's result) is
//! taken to reach.  Every fallback here is a DECLINE, which keeps the copy the program
//! already pays for.
//!
//! `LOFT_NO_COPY_VIEW=1` keeps every copy; `LOFT_TRACE_COPY_VIEW=1` names each local made a
//! view and each candidate declined with the reason.

use std::collections::HashSet;

use crate::data::{Data, DefType, Deps, Type, Value};
use crate::variables::Function;

/// Rewrite every admitted copy in `d_nr`; a no-op under the switch.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !crate::keys::copy_view_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    if !matches!(data.def(d_nr).code().unspan(), Value::Block(_)) {
        return;
    }
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    // Only a record local is a candidate, and the test is made in the walk: a compile pays
    // this pass for every function, and most have no record copy at all.
    let mut candidates: Vec<(u16, Shape)> = Vec::new();
    {
        let vars = data.def(d_nr).variables();
        code.walk(&mut |n| {
            if let Value::Set(t, rhs) = n
                && record_of(vars.tp(*t)).is_some()
                && let Some(shape) = shape_of(rhs)
            {
                candidates.push((*t, shape));
            }
        });
    }
    if candidates.is_empty() {
        data.definitions[d_nr as usize].code = code;
        return;
    }
    // A join arm's `L = a` is the join's to judge, never a candidate of its own.
    let lifts: HashSet<u16> = candidates
        .iter()
        .filter_map(|(_, s)| match s {
            Shape::Join(arms) => Some(arms.iter().map(|(l, _)| *l)),
            Shape::Plain(_) => None,
        })
        .flatten()
        .collect();
    candidates.retain(|(t, _)| !lifts.contains(t));
    let frees = crate::const_fn::free_ops(data);
    let trace = crate::keys::trace_copy_view();
    for (t, shape) in candidates {
        let vars = data.def(d_nr).variables();
        match admit(&code, vars, data, &frees, t, &shape) {
            Ok(rd) => {
                apply(&mut code, data, d_nr, &frees, t, &shape, rd);
                crate::rewrite_census::fired("R-CopyView", 1);
                if trace {
                    eprintln!(
                        "[copy-view] fn={} local={} VIEWS its source instead of copying it",
                        data.def(d_nr).name(),
                        data.def(d_nr).variables().name(t)
                    );
                }
            }
            Err(why) => {
                if trace {
                    eprintln!(
                        "[copy-view] fn={} local={} keeps its copy: {why}",
                        data.def(d_nr).name(),
                        vars.name(t)
                    );
                }
            }
        }
    }
    data.definitions[d_nr as usize].code = code;
}

/// What a candidate bind copies from: one source, or a join's arms as (lift, source).
enum Shape {
    Plain(u16),
    Join(Vec<(u16, u16)>),
}

impl Shape {
    fn sources(&self) -> Vec<u16> {
        match self {
            Shape::Plain(a) => vec![*a],
            Shape::Join(arms) => arms.iter().map(|(_, a)| *a).collect(),
        }
    }
}

fn shape_of(rhs: &Value) -> Option<Shape> {
    match rhs.unspan() {
        Value::Var(a) => Some(Shape::Plain(*a)),
        Value::If(_, t, e) => {
            let mut arms = Vec::new();
            (join_arms(t, &mut arms) && join_arms(e, &mut arms)).then_some(Shape::Join(arms))
        }
        _ => None,
    }
}

/// An arm `{ L = a; L }`, or a nested `if` of such arms (an `else if` chain).
fn join_arms(arm: &Value, out: &mut Vec<(u16, u16)>) -> bool {
    match arm.unspan() {
        Value::Block(bl) => match bl.operators.as_slice() {
            [set, tail] => match (set.unspan(), tail.unspan()) {
                (Value::Set(l, src), Value::Var(l2)) if l == l2 => match src.unspan() {
                    Value::Var(a) => {
                        out.push((*l, *a));
                        true
                    }
                    _ => false,
                },
                _ => false,
            },
            _ => false,
        },
        Value::If(_, t, e) => join_arms(t, out) && join_arms(e, out),
        _ => false,
    }
}

/// The plain record `tp` holds and its deps — `None` for a nullable one: the nullability
/// question is asked on its own (`@FR-N-Shape`), and a `τ?` keeps its copy, since a view of
/// an absent record would have to answer absence the copy's store answered for it.
fn record_of(tp: &Type) -> Option<(u32, &[u16])> {
    let (base, nullable) = tp.peel_optional();
    if nullable {
        return None;
    }
    match base {
        Type::Reference(d, deps) => Some((*d, deps.as_slice())),
        _ => None,
    }
}

/// The record a bindable local holds: a plain record with no deps (it owns its copy).
fn owned_record(vars: &Function, v: u16) -> Option<u32> {
    record_of(vars.tp(v)).and_then(|(d, deps)| deps.is_empty().then_some(d))
}

/// The whole admission of `t`; answers the record type it holds.
fn admit(
    code: &Value,
    vars: &Function,
    data: &Data,
    frees: &[u32],
    t: u16,
    shape: &Shape,
) -> Result<u32, &'static str> {
    if vars.is_argument(t) || vars.is_captured(t) {
        return Err("a parameter or a captured local");
    }
    // A plain bind's local owns the copy; a join's local already views its arms, and the
    // arms' lifts own the copies — the same record type throughout, never nullable.
    let rd = match shape {
        Shape::Plain(_) => owned_record(vars, t),
        Shape::Join(arms) => record_of(vars.tp(t)).and_then(|(d, deps)| {
            let own: HashSet<u16> = arms.iter().flat_map(|(l, a)| [*l, *a]).collect();
            deps.iter().all(|x| own.contains(x)).then_some(d)
        }),
    };
    let Some(rd) = rd else {
        return Err("not a plain, non-nullable record");
    };
    let sets = set_counts(code);
    if sets.get(&t).copied().unwrap_or(0) != 1 {
        return Err("assigned more than once");
    }
    if let Shape::Join(arms) = shape {
        for (l, _) in arms {
            if owned_record(vars, *l) != Some(rd) || sets.get(l).copied().unwrap_or(0) != 1 {
                return Err("a join arm that is not one plain copy");
            }
            if non_free_mentions(code, *l, frees) != 2 {
                return Err("a join arm's temporary is used elsewhere");
            }
        }
    }
    for a in shape.sources() {
        if record_of(vars.tp(a)).map(|(d, _)| d) != Some(rd) {
            return Err("a source that is not the same plain record (nullable, a link)");
        }
        // A PARAMETER itself is `@FR-R-ValueRecord`'s: a small record parameter crosses the
        // call as a tuple of its fields, and a copy of it is already free there — a view
        // would need the store the tuple removed (measured: server's `_discard_1 = self` took
        // eight methods' tuple parameters away).  A view INTO a parameter stays admitted.
        if vars.is_argument(a) {
            return Err("a parameter itself (R-ValueRecord may carry it as a tuple)");
        }
        if !roots_are_parameters(vars, a) {
            return Err("a source that is not a view of the caller's records");
        }
    }
    if !only_read(code, t, data, frees, false) {
        return Err("written, returned, stored or handed on");
    }
    let Some(range) = live_range(code, t, frees) else {
        return Err("mentioned outside the block that binds it");
    };
    let reach = Reach::new(data, rd);
    if let Some(why) = range.iter().find_map(|s| disturbs(s, vars, data, &reach)) {
        return Err(why);
    }
    Ok(rd)
}

/// Does every dep chain of `a` end in a parameter (or `a` is one)?  Then the record it views
/// lives in a store the caller owns for the whole call.
fn roots_are_parameters(vars: &Function, a: u16) -> bool {
    let mut seen = HashSet::new();
    let mut stack = vec![a];
    while let Some(v) = stack.pop() {
        if !seen.insert(v) {
            continue;
        }
        let deps = vars.tp(v).depend();
        if deps.is_empty() {
            if !vars.is_argument(v) {
                return false;
            }
        } else {
            stack.extend(deps.into_iter().filter(|d| *d < vars.count()));
        }
    }
    true
}

/// Per variable, its `Set`s other than a preamble null-init (the sentinel, not a value).
fn set_counts(code: &Value) -> std::collections::HashMap<u16, usize> {
    let mut out = std::collections::HashMap::new();
    code.walk(&mut |n| {
        if let Value::Set(v, rhs) = n
            && !matches!(rhs.unspan(), Value::Null)
        {
            *out.entry(*v).or_insert(0) += 1;
        }
    });
    out
}

fn is_free(v: &Value, frees: &[u32]) -> bool {
    matches!(v.unspan(), Value::Call(op, _) if frees.contains(op))
}

/// Mentions of `x` — its reads and its non-null `Set`s — other than as an operand of a free.
fn non_free_mentions(code: &Value, x: u16, frees: &[u32]) -> usize {
    fn go(n: &Value, x: u16, frees: &[u32], k: &mut usize) {
        if is_free(n, frees) {
            return;
        }
        if let Value::Var(y) = n.unspan()
            && *y == x
        {
            *k += 1;
        }
        if let Value::Set(y, rhs) = n.unspan()
            && *y == x
            && !matches!(rhs.unspan(), Value::Null)
        {
            *k += 1;
        }
        n.for_each_child(&mut |c| go(c, x, frees, k));
    }
    let mut k = 0;
    go(code, x, frees, &mut k);
    k
}

/// Is every mention of `t` a READ — the first operand of a scalar getter, possibly through
/// field projections — or an operand of a free (which the rewrite removes)?
fn only_read(node: &Value, t: u16, data: &Data, frees: &[u32], under_getter: bool) -> bool {
    match node.unspan() {
        Value::Var(x) if *x == t => under_getter,
        Value::Call(op, args) => {
            if frees.contains(op) {
                return true;
            }
            let name = data.def(*op).name();
            let getter =
                crate::parser::work_buffer::is_scalar_accessor(name) && name.starts_with("OpGet");
            let projection = name == "OpGetField";
            args.iter().enumerate().all(|(i, a)| {
                let read = i == 0 && (getter || (projection && under_getter));
                only_read(a, t, data, frees, read)
            })
        }
        Value::Set(_, rhs) => only_read(rhs, t, data, frees, false),
        _ => {
            let mut ok = true;
            node.for_each_child(&mut |c| {
                if ok {
                    ok = only_read(c, t, data, frees, false);
                }
            });
            ok
        }
    }
}

/// The statements from `t`'s bind to its last mention, in the block that binds it; `None`
/// when `t` is mentioned outside them (its frees aside).
fn live_range<'a>(code: &'a Value, t: u16, frees: &[u32]) -> Option<Vec<&'a Value>> {
    fn find(n: &Value, t: u16) -> Option<(&[Value], usize)> {
        if let Value::Block(bl) | Value::Loop(bl) = n.unspan()
            && let Some(i) = bl
                .operators
                .iter()
                .position(|op| matches!(op.unspan(), Value::Set(x, _) if *x == t))
        {
            return Some((&bl.operators, i));
        }
        let mut hit = None;
        n.for_each_child(&mut |c| {
            if hit.is_none() {
                hit = find(c, t);
            }
        });
        hit
    }
    let (ops, i) = find(code, t)?;
    let mentions = |s: &Value| non_free_mentions(s, t, frees);
    let j = (i..ops.len()).rev().find(|&k| mentions(&ops[k]) > 0)?;
    let inside: usize = ops[i..=j].iter().map(mentions).sum();
    (inside == non_free_mentions(code, t, frees)).then(|| ops[i + 1..=j].iter().collect())
}

/// Which types can hold a record of type `rd`, through fields and elements.
struct Reach<'a> {
    data: &'a Data,
    rd: u32,
}

impl<'a> Reach<'a> {
    fn new(data: &'a Data, rd: u32) -> Self {
        Reach { data, rd }
    }

    fn reaches(&self, tp: &Type) -> bool {
        self.reaches_in(tp, &mut HashSet::new())
    }

    fn reaches_in(&self, tp: &Type, seen: &mut HashSet<u32>) -> bool {
        match tp {
            Type::Optional(inner) | Type::RefVar(inner) => self.reaches_in(inner, seen),
            Type::Vector(elem, _) => self.reaches_in(elem, seen),
            // A text holds characters, never a record.
            Type::Text(_) => false,
            _ => {
                let Some(d) = tp.heap_def_nr() else {
                    // A keyed collection or any other heap shape this does not name: reaches.
                    return tp.heap_dep().is_some();
                };
                if d == self.rd {
                    return true;
                }
                if !seen.insert(d) {
                    return false;
                }
                self.data
                    .def(d)
                    .attributes()
                    .iter()
                    .any(|a| self.reaches_in(&a.typedef, seen))
            }
        }
    }
}

/// Why statement `s` may disturb a record of the source type, or `None`.
fn disturbs(s: &Value, vars: &Function, data: &Data, reach: &Reach) -> Option<&'static str> {
    let mut why = None;
    s.walk(&mut |n| {
        if why.is_some() {
            return;
        }
        match n {
            Value::Call(op, args) => {
                let def = data.def(*op);
                let name = def.name();
                let reader = name.starts_with("OpGet")
                    || name.starts_with("OpLength")
                    || name.starts_with("OpSize")
                    || matches!(
                        name,
                        "OpVectorRef"
                            | "OpVectorRefNullable"
                            | "OpVectorIsNull"
                            | "OpRefIsNull"
                            | "OpConvBoolFromRef"
                            | "OpEqRef"
                            | "OpNeRef"
                    );
                if reader {
                    return;
                }
                let loft_fn = def.is_loft_defined() && !name.starts_with("Op");
                for (i, a) in args.iter().enumerate() {
                    let reaches = match a.unspan() {
                        Value::Var(x) => reach.reaches(vars.tp(*x)),
                        // A computed heap operand's precise type is not known here.
                        Value::Call(f, _) => data.def(*f).returned().heap_dep().is_some(),
                        _ => false,
                    };
                    if !reaches {
                        continue;
                    }
                    let to_const =
                        loft_fn && def.attributes().get(i).is_some_and(|at| at.value_const);
                    if !to_const {
                        why = Some(if loft_fn {
                            "a call in its live range can write a record of its type"
                        } else {
                            "an operation in its live range can write a record of its type"
                        });
                        return;
                    }
                }
            }
            Value::CallRef(..) => why = Some("a function value is called in its live range"),
            Value::Yield(_) | Value::Parallel(_) => {
                why = Some("its live range suspends or forks");
            }
            _ => {}
        }
    });
    why
}

/// Rewrite the admitted bind of `t` into a view and drop what is no longer minted.
fn apply(
    code: &mut Value,
    data: &mut Data,
    d_nr: u32,
    frees: &[u32],
    t: u16,
    shape: &Shape,
    rd: u32,
) {
    let get_field = data.def_nr("OpGetField");
    let tp_nr = i32::from(data.def(rd).known_type());
    let lifts: HashSet<u16> = match shape {
        Shape::Plain(_) => HashSet::new(),
        Shape::Join(arms) => arms.iter().map(|(l, _)| *l).collect(),
    };
    fn rewrite_arm(arm: &mut Value, lifts: &HashSet<u16>) {
        match arm.unspan_mut() {
            Value::If(_, t, e) => {
                rewrite_arm(t, lifts);
                rewrite_arm(e, lifts);
            }
            Value::Block(bl) => {
                if let [set, _] = bl.operators.as_slice()
                    && let Value::Set(l, src) = set.unspan()
                    && lifts.contains(l)
                {
                    *arm = src.unspan().clone();
                }
            }
            _ => {}
        }
    }
    fn go(
        n: &mut Value,
        t: u16,
        shape: &Shape,
        lifts: &HashSet<u16>,
        get_field: u32,
        tp_nr: i32,
    ) -> bool {
        if let Value::Set(x, rhs) = n.unspan_mut()
            && *x == t
        {
            match shape {
                Shape::Plain(a) => {
                    **rhs = Value::Call(
                        get_field,
                        vec![Value::Var(*a), Value::Int(0), Value::Int(tp_nr)],
                    );
                }
                Shape::Join(_) => {
                    if let Value::If(_, a, b) = rhs.unspan_mut() {
                        rewrite_arm(a, lifts);
                        rewrite_arm(b, lifts);
                    }
                }
            }
            return true;
        }
        let mut done = false;
        n.for_each_child_mut(&mut |c| {
            if !done {
                done = go(c, t, shape, lifts, get_field, tp_nr);
            }
        });
        done
    }
    go(code, t, shape, &lifts, get_field, tp_nr);
    // The frees of `t` (a plain bind's) and of the lifts, and the lifts' null-inits: nothing
    // is minted into them any more.
    let dead: HashSet<u16> = lifts.iter().copied().chain(std::iter::once(t)).collect();
    strip(code, &dead, frees);
    let sources: Vec<u16> = shape.sources();
    let vars = &mut data.definitions[d_nr as usize].variables;
    let tp = vars.tp(t).with_deps(&Deps::frame(sources));
    vars.set_type(t, tp);
    for l in lifts {
        vars.set_skip_free(l);
    }
}

/// Remove every statement that frees one of `dead` or null-inits one of them.
fn strip(n: &mut Value, dead: &HashSet<u16>, frees: &[u32]) {
    let is_dead = |op: &Value| match op.unspan() {
        Value::Call(f, args) if frees.contains(f) => {
            matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if dead.contains(x))
        }
        Value::Set(x, rhs) => dead.contains(x) && matches!(rhs.unspan(), Value::Null),
        _ => false,
    };
    if let Value::Block(bl) | Value::Loop(bl) = n.unspan_mut() {
        bl.operators.retain(|op| !is_dead(op));
    }
    if let Value::Insert(ls) = n.unspan_mut() {
        ls.retain(|op| !is_dead(op));
    }
    n.for_each_child_mut(&mut |c| strip(c, dead, frees));
}
