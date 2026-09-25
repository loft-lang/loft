// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-Compact` — a vector rebuilt from a contiguous run of its own elements is compacted
//! IN PLACE.
//!
//! `t: vector<Stroke> = []; for i in 0..n { t += [h.entries[i]?]; } h.entries = t;` — a
//! history truncated to its cursor — paid three deep copies of every kept entry: the build
//! of `t`, the snapshot the field rebind takes, and the copy back, each claiming the entry's
//! inner vectors, with the old entries released only at the end.  The value that results
//! already exists: it is the run `[lo, hi)` of the vector itself.  So the three statements
//! become ONE guarded op — `if 0 <= lo && lo <= hi && hi <= len(V) { OpKeepVectorRange(V,
//! tp, lo, hi) } else { the statements as written }` — where the op releases every element
//! outside the run where it stands, moves the run to the front as one block and sets the
//! length (`Stores::keep_vector_range`).  The fallback arm is the program as the parser
//! lowered it, statement for statement, so a range the guard refuses — a negative start, an
//! end past the length, whose `?`-discharged reads pad with default records — answers
//! exactly what it always answered.
//!
//! Recognised, on the IR the parser lowered and the scan settled: the declaration `t = []`
//! (a mint, the field read, the length word), a counted `for` over `a..b` (either range
//! prelude) whose body appends exactly the `?`-discharged element `V[i]` of the loop
//! variable — a RECORD element, since an in-range record element is never absent, where a
//! scalar element can hold the null its `??` would replace — and the rebind `V = t` in its
//! field form (snapshot, clear, copy back) or its local form (a fresh store filled from `t`),
//! with `t` mentioned nowhere else in the function and the bounds pure reads (a literal, a
//! variable the loop does not write, the length of `V` itself).  Anything else keeps the
//! rebuild: a wrong decline is the copies the program already pays, a wrong admission would
//! be a silently different vector.
//!
//! `LOFT_NO_COMPACT=1` keeps every rebuild; `LOFT_TRACE_COMPACT=1` names each site.

use crate::data::{Block, Data, DefType, Value, v_if};

/// Rewrite every admitted rebuild in `d_nr`; a no-op under the switch.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !crate::keys::compact_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    let ops = Ops::new(data);
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    // Cheap exit: a rebuild needs the parser's field-rebind snapshot or a local's fresh
    // store, both of which append from a local — no `OpAppendVector`, no rebuild.
    if code.any_node(&mut |n| matches!(n, Value::Call(d, _) if *d == ops.append_vector)) {
        let mut cx = Cx {
            data,
            d_nr,
            ops: &ops,
            whole: &code,
        };
        let mut fresh = code.clone();
        visit(&mut fresh, &mut cx);
        code = fresh;
    }
    data.definitions[d_nr as usize].code = code;
}

/// The def numbers of every op the matcher names, looked up once.
struct Ops {
    database: u32,
    get_field: u32,
    set_int4: u32,
    pre_alloc: u32,
    new_record: u32,
    copy_record: u32,
    finish_record: u32,
    get_vector_nullable: u32,
    conv_bool_from_ref: u32,
    conv_bool_from_int: u32,
    conv_int_from_null: u32,
    append_vector: u32,
    clear_vector: u32,
    free_ref: u32,
    add_int: u32,
    le_int: u32,
    vector_len: u32,
    length_vector: u32,
    keep_range: u32,
}

impl Ops {
    fn new(data: &Data) -> Self {
        Self {
            database: data.def_nr("OpDatabase"),
            get_field: data.def_nr("OpGetField"),
            set_int4: data.def_nr("OpSetInt4"),
            pre_alloc: data.def_nr("OpPreAllocVector"),
            new_record: data.def_nr("OpNewRecord"),
            copy_record: data.def_nr("OpCopyRecord"),
            finish_record: data.def_nr("OpFinishRecord"),
            get_vector_nullable: data.def_nr("OpGetVectorNullable"),
            conv_bool_from_ref: data.def_nr("OpConvBoolFromRef"),
            conv_bool_from_int: data.def_nr("OpConvBoolFromInt"),
            conv_int_from_null: data.def_nr("OpConvIntFromNull"),
            append_vector: data.def_nr("OpAppendVector"),
            clear_vector: data.def_nr("OpClearVector"),
            free_ref: data.def_nr("OpFreeRef"),
            add_int: data.def_nr("OpAddInt"),
            le_int: data.def_nr("OpLeInt"),
            vector_len: data.def_nr("t_6vector_len"),
            length_vector: data.def_nr("OpLengthVector"),
            keep_range: data.def_nr("OpKeepVectorRange"),
        }
    }
}

struct Cx<'a> {
    data: &'a Data,
    d_nr: u32,
    ops: &'a Ops,
    /// The whole body as it was, for the "mentioned nowhere else" test.
    whole: &'a Value,
}

/// What one rebuild consists of, once matched.
struct Rebuild {
    /// How many SIGNIFICANT statements of the list it spans (`significant`).
    span: usize,
    /// The field form's snapshot local whose free the scan placed LATER in the list (at the
    /// block's end): that free is moved into the fallback arm beside the statements that fill
    /// the snapshot, so the fast arm never names a slot it never initialised.
    deferred_free: Option<u16>,
    /// The vector place: a local, or a field chain over one.
    place: Value,
    /// The element type the `OpFinishRecord` names.
    elem_tp: i32,
    lo: Value,
    hi: Value,
    t: u16,
}

fn visit(v: &mut Value, cx: &mut Cx) {
    match v.unspan_mut() {
        Value::Block(bl) | Value::Loop(bl) => visit_list(&mut bl.operators, cx),
        Value::Insert(ls) => visit_list(ls, cx),
        other => other.for_each_child_mut(&mut |c| visit(c, cx)),
    }
}

/// The statements of a list that carry code: a `Line` marker or a `Null` between two
/// statements is position bookkeeping the matcher sees through (and the fallback arm keeps).
fn significant(ls: &[Value]) -> Vec<(usize, &Value)> {
    ls.iter()
        .enumerate()
        .filter(|(_, v)| !matches!(v.unspan(), Value::Line(_) | Value::Null))
        .collect()
}

fn visit_list(ls: &mut Vec<Value>, cx: &mut Cx) {
    let mut i = 0;
    while i < ls.len() {
        let sig = significant(&ls[i..]);
        if !sig.is_empty()
            && sig[0].0 == 0
            && let Some(r) = match_rebuild(&sig, cx)
        {
            // `span` counts significant statements; take the list up to the last of them.
            let end = i + sig[r.span - 1].0 + 1;
            // A snapshot freed later in this list: that free joins the fallback arm, and it
            // must be the snapshot's only other mention — else the rebuild stays as written.
            let later_free = match r.deferred_free {
                Some(rhs) => {
                    let at = ls[end..]
                        .iter()
                        .position(|v| matches!(call(v, cx.ops.free_ref), Some(a) if var(&a[0]) == Some(rhs)))
                        .map(|j| end + j);
                    match at {
                        Some(j) if mentions(cx.whole, rhs) == 4 => Some(j),
                        _ => {
                            let why = format!(
                                "the snapshot's free is not the one statement after the rebuild that names it (free found: {}, mentions: {})",
                                at.is_some(),
                                mentions(cx.whole, rhs)
                            );
                            trace_decline(cx, r.t, &why);
                            visit(&mut ls[i], cx);
                            i += 1;
                            continue;
                        }
                    }
                }
                None => None,
            };
            let mut taken: Vec<Value> = ls.drain(i..end).collect();
            if let Some(j) = later_free {
                taken.push(ls.remove(j - (end - i)));
            }
            ls.insert(i, guarded(r, taken, cx));
            i += 1;
            continue;
        }
        visit(&mut ls[i], cx);
        i += 1;
    }
}

/// `if 0 <= lo && lo <= hi && hi <= len(V) { keep } else { the statements as written }`.
fn guarded(r: Rebuild, original: Vec<Value>, cx: &Cx) -> Value {
    let ops = cx.ops;
    if crate::keys::trace_compact() {
        eprintln!(
            "[compact] fn={} t={} ADMITTED: the rebuild is one in-place keep of [lo, hi)",
            cx.data.def(cx.d_nr).name(),
            cx.data.def(cx.d_nr).variables().name(r.t)
        );
    }
    let len = Value::Call(ops.length_vector, vec![r.place.clone()]);
    let le = |a: Value, b: Value| Value::Call(ops.le_int, vec![a, b]);
    // A bound that is a variable may be null; the checked compare answers null on it and
    // the guard must read that as "not proven", so each such bound is tested first.
    let not_null = |b: &Value| match b.unspan() {
        Value::Var(_) => Some(Value::Call(ops.conv_bool_from_int, vec![b.clone()])),
        _ => None,
    };
    let mut tests: Vec<Value> = Vec::new();
    tests.extend(not_null(&r.lo));
    tests.extend(not_null(&r.hi));
    tests.push(le(Value::Int(0), r.lo.clone()));
    tests.push(le(r.lo.clone(), r.hi.clone()));
    tests.push(le(r.hi.clone(), len));
    // `a && b && c` as the parser spells it: `if a { if b { c } else { false } } else { false }`.
    let mut cond = tests.pop().expect("at least three tests");
    while let Some(t) = tests.pop() {
        cond = v_if(t, cond, Value::Boolean(false));
    }
    let keep = Value::Call(
        ops.keep_range,
        vec![r.place, Value::Int(r.elem_tp), r.lo, r.hi],
    );
    v_if(cond, Value::Insert(vec![keep]), Value::Insert(original))
}

// ── the matcher ─────────────────────────────────────────────────────────────────────────

fn call(v: &Value, op: u32) -> Option<&[Value]> {
    match v.unspan() {
        Value::Call(d, args) if *d == op => Some(args),
        _ => None,
    }
}

fn var(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(x) => Some(*x),
        _ => None,
    }
}

fn int(v: &Value) -> Option<i32> {
    match v.unspan() {
        Value::Int(n) => Some(*n),
        _ => None,
    }
}

fn block<'a>(v: &'a Value, name: &str) -> Option<&'a Block> {
    match v.unspan() {
        Value::Block(bl) if bl.name == name => Some(bl),
        _ => None,
    }
}

/// A vector PLACE the guard and the op may re-read: a local, or a field chain over one.
fn is_place(v: &Value, ops: &Ops) -> bool {
    match v.unspan() {
        Value::Var(_) => true,
        Value::Call(d, args) if *d == ops.get_field && args.len() == 3 => {
            int(&args[1]).is_some() && int(&args[2]).is_some() && is_place(&args[0], ops)
        }
        _ => false,
    }
}

fn same(a: &Value, b: &Value) -> bool {
    a.unspan() == b.unspan()
}

/// `[a] OpDatabase(vdb, _); [b] t = OpGetField(vdb, 0, _); [c] OpSetInt4(vdb, 0, 0)`.
fn match_decl(stmts: &[(usize, &Value)], ops: &Ops) -> Option<(u16, u16)> {
    let mint = call(stmts.first()?.1, ops.database)?;
    let vdb = var(mint.first()?)?;
    let Value::Set(target, rhs) = stmts.get(1)?.1.unspan() else {
        return None;
    };
    let field = call(rhs, ops.get_field)?;
    if var(field.first()?)? != vdb || int(field.get(1)?)? != 0 {
        return None;
    }
    let zero = call(stmts.get(2)?.1, ops.set_int4)?;
    if var(zero.first()?)? != vdb || int(zero.get(1)?)? != 0 || int(zero.get(2)?)? != 0 {
        return None;
    }
    Some((*target, vdb))
}

/// A bound the guard may re-evaluate: a literal, a variable (not one the loop writes), or
/// the length of the place itself.
fn is_bound(v: &Value, place: &Value, forbidden: &[u16], ops: &Ops) -> bool {
    match v.unspan() {
        Value::Int(_) => true,
        Value::Var(x) => !forbidden.contains(x),
        Value::Call(d, args)
            if (*d == ops.vector_len || *d == ops.length_vector) && args.len() == 1 =>
        {
            same(&args[0], place)
        }
        _ => false,
    }
}

/// The parser's exclusive-range iteration, either prelude; answers `(i, lo, hi, body)`.
fn match_loop<'a>(v: &'a Value, ops: &Ops) -> Option<(u16, Value, Value, &'a Block)> {
    let bl = block(v, "For block")?;
    let s = &bl.operators;
    // Literal start: `[_range_end = hi;] i#index = start - 1; loop { … }` — the end a
    // variable of the prelude, or a literal spelled in the compare itself.
    let (end, hi, rest) = match s.len() {
        3 => match s[0].unspan() {
            Value::Set(end, hi) => (Some(*end), hi.unspan().clone(), &s[1..]),
            _ => return None,
        },
        2 => (None, Value::Null, &s[..]),
        _ => (None, Value::Null, &s[..]),
    };
    if (s.len() == 2 || s.len() == 3)
        && let Value::Set(index, init) = rest[0].unspan()
        && let Some(start_minus_one) = int(init)
        && let Value::Loop(lp) = rest[1].unspan()
        && lp.operators.len() == 2
        && let Value::Set(i, iter) = lp.operators[0].unspan()
        && let Some(it) = block(iter, "Iter range")
        && it.operators.len() == 3
        && let Some(step) = call(&it.operators[0], u32::MAX).or_else(|| {
            let Value::Set(x, rhs) = it.operators[0].unspan() else {
                return None;
            };
            (*x == *index).then_some(call(rhs, ops.add_int)?)
        })
        && var(&step[0]) == Some(*index)
        && int(&step[1]) == Some(1)
        && let Value::If(c, brk, no) = it.operators[1].unspan()
        && matches!(brk.unspan(), Value::Break(_))
        && matches!(no.unspan(), Value::Null)
        && let Some(cmp) = call(c, ops.le_int)
        && var(&cmp[1]) == Some(*index)
        && var(&it.operators[2]) == Some(*index)
        && let Value::Block(body) = lp.operators[1].unspan()
    {
        let hi = match end {
            Some(end) if var(&cmp[0]) == Some(end) => hi,
            None if int(&cmp[0]).is_some() => cmp[0].unspan().clone(),
            _ => return None,
        };
        let lo = Value::Int(start_minus_one.checked_add(1)?);
        return Some((*i, lo, hi, body));
    }
    // Computed start: `_range_start = lo; _range_end = hi; _next = _range_start;
    // i#index = null; loop { … }`.
    if s.len() == 5
        && let Value::Set(start, lo) = s[0].unspan()
        && let Value::Set(end, hi) = s[1].unspan()
        && let Value::Set(next, from) = s[2].unspan()
        && var(from) == Some(*start)
        && let Value::Set(index, init) = s[3].unspan()
        && call(init, ops.conv_int_from_null).is_some()
        && let Value::Loop(lp) = s[4].unspan()
        && lp.operators.len() == 2
        && let Value::Set(i, iter) = lp.operators[0].unspan()
        && let Some(it) = block(iter, "Iter range")
        && it.operators.len() == 4
        && let Value::If(c, brk, no) = it.operators[0].unspan()
        && matches!(brk.unspan(), Value::Break(_))
        && matches!(no.unspan(), Value::Null)
        && let Some(cmp) = call(c, ops.le_int)
        && var(&cmp[0]) == Some(*end)
        && var(&cmp[1]) == Some(*next)
        && let Value::Set(index2, from2) = it.operators[1].unspan()
        && *index2 == *index
        && var(from2) == Some(*next)
        && let Value::Set(next2, step) = it.operators[2].unspan()
        && *next2 == *next
        && let Some(add) = call(step, ops.add_int)
        && var(&add[0]) == Some(*next)
        && int(&add[1]) == Some(1)
        && var(&it.operators[3]) == Some(*index)
        && let Value::Block(body) = lp.operators[1].unspan()
    {
        return Some((*i, lo.unspan().clone(), hi.unspan().clone(), body));
    }
    None
}

/// The body `t += [V[i]?]` for a record element; answers `V`.
fn match_record_append(body: &Block, target: u16, index: u16, ops: &Ops) -> Option<Value> {
    let stmts = &body.operators;
    if stmts.len() != 4 {
        return None;
    }
    let pre = call(&stmts[0], ops.pre_alloc)?;
    if var(&pre[0]) != Some(target) {
        return None;
    }
    let Value::Set(elm, mint) = stmts[1].unspan() else {
        return None;
    };
    let new_rec = call(mint, ops.new_record)?;
    if var(&new_rec[0]) != Some(target) {
        return None;
    }
    let parent_tp = int(&new_rec[1])?;
    let copy = call(&stmts[2], ops.copy_record)?;
    if var(&copy[1]) != Some(*elm) {
        return None;
    }
    let fin = call(&stmts[3], ops.finish_record)?;
    if var(&fin[0]) != Some(target) || var(&fin[1]) != Some(*elm) || int(&fin[2]) != Some(parent_tp)
    {
        return None;
    }
    // The copied value: `{ ncc = OpGetVectorNullable(V, size, i); if ncc then ncc else {Object} }`.
    let ncc = block(&copy[0], "ncc")?;
    if ncc.operators.len() != 2 {
        return None;
    }
    let Value::Set(read_var, read) = ncc.operators[0].unspan() else {
        return None;
    };
    let get = call(read, ops.get_vector_nullable)?;
    if get.len() != 3 || var(&get[2]) != Some(index) || !is_place(&get[0], ops) {
        return None;
    }
    let Value::If(cond, then, absent) = ncc.operators[1].unspan() else {
        return None;
    };
    let test = call(cond, ops.conv_bool_from_ref)?;
    if var(&test[0]) != Some(*read_var)
        || var(then) != Some(*read_var)
        || block(absent, "Object").is_none()
    {
        return None;
    }
    Some(get[0].unspan().clone())
}

/// The rebind `V = t`: the field form (5 statements) or the local form (4); answers the span
/// and the element RECORD type the append names (`known_type` of the element's struct — the
/// type `OpRemoveVector` takes, and the one the layout is decided on).
fn match_rebind(
    s: &[(usize, &Value)],
    t: u16,
    place: &Value,
    ops: &Ops,
) -> Option<(usize, i32, Option<u16>)> {
    // Field form: `rhs = null; OpAppendVector(rhs, t, tp); OpClearVector(V);
    // OpAppendVector(V, rhs, tp)`, the snapshot's `OpFreeRef(rhs)` right after (then it is
    // part of the span) or wherever the scan put it (then it stays, and the init the fast
    // arm needs for it is hoisted ahead of the guard).
    if let Value::Set(rhs, nul) = s.first()?.1.unspan()
        && matches!(nul.unspan(), Value::Null)
        && let Some(a1) = call(s.get(1)?.1, ops.append_vector)
        && var(&a1[0]) == Some(*rhs)
        && var(&a1[1]) == Some(t)
        && let Some(cl) = call(s.get(2)?.1, ops.clear_vector)
        && same(&cl[0], place)
        && let Some(a2) = call(s.get(3)?.1, ops.append_vector)
        && same(&a2[0], place)
        && var(&a2[1]) == Some(*rhs)
        && let Some(elem_tp) = int(a1.get(2)?)
        && elem_tp > 0
        && int(a2.get(2)?) == Some(elem_tp)
    {
        let freed_here = s
            .get(4)
            .and_then(|(_, v)| call(v, ops.free_ref))
            .is_some_and(|fr| var(&fr[0]) == Some(*rhs));
        return Some(if freed_here {
            (5, elem_tp, None)
        } else {
            (4, elem_tp, Some(*rhs))
        });
    }
    // Local form: `OpDatabase(vdb2, _); V = OpGetField(vdb2, 0, _); OpSetInt4(vdb2, 0, 0);
    // OpAppendVector(V, t, tp)`.
    let v = var(place)?;
    let (v2, _) = match_decl(s, ops)?;
    if v2 != v {
        return None;
    }
    let a = call(s.get(3)?.1, ops.append_vector)?;
    if var(&a[0]) != Some(v) || var(&a[1]) != Some(t) {
        return None;
    }
    let elem_tp = int(a.get(2)?)?;
    (elem_tp > 0).then_some((4, elem_tp, None))
}

/// How many nodes name variable `x` — in EVERY spelling: a `Var`, and the variants that
/// carry a var number on the node itself (a `Set`'s target, an `Iter`'s, a tuple access, a
/// closure call).  A count keyed on `Var` alone read a snapshot's init as no mention.
fn mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |node| {
        let named = match node {
            Value::Var(y)
            | Value::Set(y, _)
            | Value::Iter(y, _, _, _)
            | Value::TupleGet(y, _)
            | Value::TuplePut(y, _, _)
            | Value::CallRef(y, _)
            | Value::FnRefDnr(y) => *y == x,
            _ => false,
        };
        if named {
            n += 1;
        }
    });
    n
}

fn match_rebuild(s: &[(usize, &Value)], cx: &Cx) -> Option<Rebuild> {
    let ops = cx.ops;
    let (t, vdb) = match_decl(s, ops)?;
    let (i, lo, hi, body) = match_loop(s.get(3)?.1, ops)?;
    let place = match_record_append(body, t, i, ops)?;
    let (rebind, elem_tp, deferred_free) = match_rebind(&s[4..], t, &place, ops)?;
    let span = 4 + rebind;
    // The bounds are pure and read nothing the loop or the rebuild writes.
    let forbidden = [t, vdb, i];
    if !is_bound(&lo, &place, &forbidden, ops) || !is_bound(&hi, &place, &forbidden, ops) {
        trace_decline(
            cx,
            t,
            "a bound that is not a literal, a variable or the place's length",
        );
        return None;
    }
    // `t` lives in these statements alone: mentioned nowhere else in the function.
    let inside: usize = s[..span].iter().map(|(_, v)| mentions(v, t)).sum();
    if mentions(cx.whole, t) != inside {
        trace_decline(cx, t, "the local is read or written outside the rebuild");
        return None;
    }
    // The loop variable is the body's alone: a use of the place's index elsewhere in the
    // body would already have failed the exact body match.
    Some(Rebuild {
        span,
        deferred_free,
        place,
        elem_tp,
        lo,
        hi,
        t,
    })
}

fn trace_decline(cx: &Cx, t: u16, why: &str) {
    if crate::keys::trace_compact() {
        eprintln!(
            "[compact] fn={} t={} DECLINED: {why}",
            cx.data.def(cx.d_nr).name(),
            cx.data.def(cx.d_nr).variables().name(t)
        );
    }
}
