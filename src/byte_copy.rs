// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-ByteCopy` — a text copied into a byte vector one byte at a time is ONE append.
//!
//! `for i in lo..hi { buf += [(t.byte_at(i) & 255) as u8] }` — the byte-wise copy a binary
//! encoder spells (cbor's text payload) — paid a bounds-checked byte read, a null-aware mask
//! and a fused push PER BYTE: 4.3 ns a byte on native against a block copy.  Where the loop
//! is exactly that — a counted range over `i` with pure bounds, a body that is the one push
//! of `t`'s byte at `i` (masked with 255 or not) into a byte vector `buf` whose element is
//! stored raw (`min` 0), `t` a variable — it becomes
//! `if 0 <= lo && lo <= hi && hi <= size(t) { OpAppendTextBytes(buf, t, lo, hi) } else { the
//! loop as written }`: one reservation and one block copy (`Stores::append_text_bytes`), the
//! fallback arm answering exactly what the loop answers for a range the guard refuses (an
//! index past the text reads null, and the loop's push of it is whatever it always was).
//! Anything else in the body, a bound that is a call other than `size(t)`, a destination
//! whose element is not a raw byte, keep the loop: a wrong decline is the loop the program
//! already pays, a wrong admission would be a byte the loop never wrote.
//!
//! `LOFT_NO_BYTE_COPY=1` keeps every loop; `LOFT_TRACE_BYTE_COPY=1` names each site.
use crate::compact::{self, Ops};
use crate::data::{Data, DefType, Value, v_if};

/// Rewrite every admitted byte-wise copy in `d_nr`; a no-op under the switch, and a cheap
/// one for a body that pushes no byte.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !crate::keys::byte_copy_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    let push_byte = data.def_nr("OpPushByte");
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    if code.any_node(&mut |n| matches!(n, Value::Call(d, _) if *d == push_byte)) {
        let ops = Ops::new(data);
        let cx = Cx {
            data,
            d_nr,
            ops: &ops,
            push_byte,
            byte_at: data.def_nr("t_4text_byte_at"),
            land_int: data.def_nr("OpLandInt"),
            size_text: data.def_nr("t_4text_size"),
            append: data.def_nr("OpAppendTextBytes"),
        };
        visit(&mut code, &cx);
    }
    data.definitions[d_nr as usize].code = code;
}

struct Cx<'a> {
    data: &'a Data,
    d_nr: u32,
    ops: &'a Ops,
    push_byte: u32,
    byte_at: u32,
    land_int: u32,
    size_text: u32,
    append: u32,
}

/// One admitted copy: the byte vector, the text, and the range's bounds as the loop spelled
/// them.
struct Copy {
    buf: u16,
    t: u16,
    lo: Value,
    hi: Value,
}

fn visit(v: &mut Value, cx: &Cx) {
    if let Some(copy) = match_copy(v, cx) {
        let original = std::mem::replace(v, Value::Null);
        *v = guarded(copy, original, cx);
        return;
    }
    match v.unspan_mut() {
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &mut bl.operators {
                visit(op, cx);
            }
        }
        Value::Insert(ls) => {
            for op in ls {
                visit(op, cx);
            }
        }
        other => other.for_each_child_mut(&mut |c| visit(c, cx)),
    }
}

/// A bound the guard may evaluate a second time: a literal, a variable, or `size(t)`.
fn pure_bound(v: &Value, cx: &Cx) -> bool {
    matches!(v.unspan(), Value::Int(_) | Value::Var(_))
        || compact::call(v, cx.size_text)
            .is_some_and(|a| a.len() == 1 && compact::var(&a[0]).is_some())
}

fn match_copy(v: &Value, cx: &Cx) -> Option<Copy> {
    let (i, lo, hi, body) = compact::match_loop(v, cx.ops)?;
    let stmts = compact::significant(&body.operators);
    // The reservation `(R-PushFill)` writes ahead of a counted push, then the one push.
    let push = match stmts.as_slice() {
        [(_, one)] => *one,
        [(_, first), (_, second)] if compact::call(first, cx.ops.pre_alloc).is_some() => *second,
        _ => return None,
    };
    let args = compact::call(push, cx.push_byte)?;
    if args.len() != 3 {
        return None;
    }
    let buf = compact::var(&args[0])?;
    if compact::int(&args[1]) != Some(0) {
        decline(cx, buf, "the element is not stored as a raw byte");
        return None;
    }
    let read = match compact::call(&args[2], cx.land_int) {
        Some(m) if m.len() == 2 && compact::int(&m[1]) == Some(255) => &m[0],
        Some(_) => return None,
        None => &args[2],
    };
    let ba = compact::call(read, cx.byte_at)?;
    if ba.len() != 2 || compact::var(&ba[1]) != Some(i) {
        return None;
    }
    let t = compact::var(&ba[0])?;
    if !pure_bound(&lo, cx) || !pure_bound(&hi, cx) {
        decline(cx, buf, "a bound the guard cannot evaluate twice");
        return None;
    }
    Some(Copy { buf, t, lo, hi })
}

/// `if 0 <= lo && lo <= hi && hi <= size(t) { append } else { the loop as written }`.
fn guarded(c: Copy, original: Value, cx: &Cx) -> Value {
    if crate::keys::trace_byte_copy() {
        eprintln!(
            "[byte-copy] fn={} ADMITTED: `{}` takes the bytes of `{}` as one append",
            cx.data.def(cx.d_nr).name(),
            cx.data.def(cx.d_nr).variables().name(c.buf),
            cx.data.def(cx.d_nr).variables().name(c.t)
        );
    }
    let ops = cx.ops;
    let size = Value::Call(cx.size_text, vec![Value::Var(c.t)]);
    let le = |a: Value, b: Value| Value::Call(ops.le_int, vec![a, b]);
    // A bound that is a variable may be null; the checked compare answers null on it and
    // the guard must read that as "not proven", so each such bound is tested first.
    let not_null = |b: &Value| match b.unspan() {
        Value::Var(_) => Some(Value::Call(ops.conv_bool_from_int, vec![b.clone()])),
        _ => None,
    };
    let mut tests: Vec<Value> = Vec::new();
    tests.extend(not_null(&c.lo));
    tests.extend(not_null(&c.hi));
    tests.push(le(Value::Int(0), c.lo.clone()));
    tests.push(le(c.lo.clone(), c.hi.clone()));
    tests.push(le(c.hi.clone(), size));
    let mut cond = tests.pop().expect("at least three tests");
    while let Some(t) = tests.pop() {
        cond = v_if(t, cond, Value::Boolean(false));
    }
    let append = Value::Call(
        cx.append,
        vec![Value::Var(c.buf), Value::Var(c.t), c.lo, c.hi],
    );
    v_if(
        cond,
        Value::Insert(vec![append]),
        Value::Insert(vec![original]),
    )
}

fn decline(cx: &Cx, buf: u16, why: &str) {
    if crate::keys::trace_byte_copy() {
        eprintln!(
            "[byte-copy] fn={} buf={} keeps its loop: {why}",
            cx.data.def(cx.d_nr).name(),
            cx.data.def(cx.d_nr).variables().name(buf)
        );
    }
}
