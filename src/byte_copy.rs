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
pub fn rewrite(data: &mut Data, database: &mut crate::database::Stores, d_nr: u32) {
    if !crate::keys::byte_copy_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    let push_byte = data.def_nr("OpPushByte");
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    if code.any_node(&mut |n| matches!(n, Value::Call(d, _) if *d == push_byte)) {
        let ops = Ops::new(data);
        let whole = code.clone();
        let cx = Cx {
            data,
            d_nr,
            ops: &ops,
            whole: &whole,
            push_byte,
            byte_at: data.def_nr("t_4text_byte_at"),
            land_int: data.def_nr("OpLandInt"),
            size_text: data.def_nr("t_4text_size"),
            append: data.def_nr("OpAppendTextBytes"),
            get_byte: data.def_nr("OpGetByte"),
            get_nullable: data.def_nr("OpGetVectorNullable"),
            length_vector: data.def_nr("OpLengthVector"),
            min_int: data.def_nr("OpMinInt"),
            slice: data.def_nr("OpSliceVector"),
        };
        // The element type each admitted vector clause names, read before the walk mutates:
        // it needs the store registry, the walk only the definitions.
        let mut tps = Vec::new();
        collect_vector_types(&whole, &cx, database, &mut tps);
        visit(&mut code, &cx, &tps);
        // `@FR-R-TextRun` — a run copied only to be read as text is read in place: decided
        // on the copies just guarded.
        if !tps.is_empty() {
            crate::text_run::rewrite(data, d_nr, &mut code);
        }
    }
    data.definitions[d_nr as usize].code = code;
}

struct Cx<'a> {
    data: &'a Data,
    d_nr: u32,
    ops: &'a Ops,
    /// The body as the scan left it, for the destination's exclusivity.
    whole: &'a Value,
    push_byte: u32,
    byte_at: u32,
    land_int: u32,
    size_text: u32,
    append: u32,
    get_byte: u32,
    get_nullable: u32,
    length_vector: u32,
    min_int: u32,
    slice: u32,
}

/// Where the copied bytes come from.
enum Source {
    /// `t.byte_at(i)` of a text variable.
    Text(u16),
    /// `v[off + i] ?? d` of a byte vector variable, `off` (a variable or a literal) absent
    /// for `v[i]`.
    Vector { v: u16, off: Option<Value> },
}

/// One admitted copy: the byte vector, the source, and the range's bounds as the loop
/// spelled them.
struct Copy {
    buf: u16,
    src: Source,
    lo: Value,
    hi: Value,
}

/// `(buf, v)` → the element type the slice append names, for every vector clause the body
/// admits; a pair the registry cannot type is absent and its loop stays.
fn collect_vector_types(
    v: &Value,
    cx: &Cx,
    database: &mut crate::database::Stores,
    out: &mut Vec<((u16, u16), i32)>,
) {
    if let Some(Copy {
        buf,
        src: Source::Vector { v: src, .. },
        ..
    }) = match_copy(v, cx)
    {
        if !out.iter().any(|(k, _)| *k == (buf, src))
            && let Some(tp) =
                crate::vec_copy::shared_element_type(cx.data, database, cx.d_nr, buf, src)
        {
            out.push(((buf, src), tp));
        }
        return;
    }
    v.for_each_child(&mut |c| collect_vector_types(c, cx, database, out));
}

fn visit(v: &mut Value, cx: &Cx, tps: &[((u16, u16), i32)]) {
    if let Some(copy) = match_copy(v, cx) {
        let tp = match &copy.src {
            Source::Text(_) => Some(0),
            Source::Vector { v: src, .. } => tps
                .iter()
                .find(|(k, _)| *k == (copy.buf, *src))
                .map(|(_, tp)| *tp),
        };
        if let Some(tp) = tp {
            let original = std::mem::replace(v, Value::Null);
            *v = guarded(copy, original, tp, cx);
        } else {
            decline(
                cx,
                copy.buf,
                "the source's element is not the destination's",
            );
        }
        return;
    }
    match v.unspan_mut() {
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &mut bl.operators {
                visit(op, cx, tps);
            }
        }
        Value::Insert(ls) => {
            for op in ls {
                visit(op, cx, tps);
            }
        }
        other => other.for_each_child_mut(&mut |c| visit(c, cx, tps)),
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
    let src = if let Some(src) = vector_read(&args[2], i, cx) {
        let Source::Vector { v, .. } = src else {
            unreachable!("vector_read answers a vector source")
        };
        if v == buf || !crate::vec_copy::exclusive_vector(cx.data, cx.d_nr, cx.whole, buf) {
            decline(
                cx,
                buf,
                "the destination may share a vector with the source",
            );
            return None;
        }
        src
    } else {
        let read = match compact::call(&args[2], cx.land_int) {
            Some(m) if m.len() == 2 && compact::int(&m[1]) == Some(255) => &m[0],
            Some(_) => return None,
            None => &args[2],
        };
        let ba = compact::call(read, cx.byte_at)?;
        if ba.len() != 2 || compact::var(&ba[1]) != Some(i) {
            return None;
        }
        Source::Text(compact::var(&ba[0])?)
    };
    if !pure_bound(&lo, cx) || !pure_bound(&hi, cx) {
        decline(cx, buf, "a bound the guard cannot evaluate twice");
        return None;
    }
    Some(Copy { buf, src, lo, hi })
}

/// The read of one RAW byte element of a vector variable at the loop index, discharged with
/// a literal: `v[i] ?? d` or `v[off + i] ?? d` (`off + i` either way round), as the parser
/// lowers it — `__ncc = { __dn4 = OpGetByte(OpGetVectorNullable(v, 1, idx), 0, 0); if 0 <=
/// __dn4 { if __dn4 <= 255 { __dn4 } else null } else null }; if __ncc { __ncc } else d`.
/// A raw byte in range is 0..=255, so the range check never answers null for it and the
/// discharge never fires: inside the guard the push is the element itself, whatever `d`.
/// Anything else — another bias, another check, a computed default, a field or an element as
/// the source — answers `None` and keeps the loop.
fn vector_read(v: &Value, i: u16, cx: &Cx) -> Option<Source> {
    let ncc = block_ops(v, "ncc")?;
    let [Value::Set(nv, checked), Value::If(test, then, els)] = ncc.as_slice() else {
        return None;
    };
    let t = compact::call(test, cx.ops.conv_bool_from_int)?;
    if compact::var(&t[0]) != Some(*nv)
        || compact::var(then) != Some(*nv)
        || compact::int(els).is_none()
    {
        return None;
    }
    let dn4 = block_ops(checked, "dn4cast")?;
    let [Value::Set(dv, get), Value::If(lo_ok, inner, null_a)] = dn4.as_slice() else {
        return None;
    };
    let lo_ok = compact::call(lo_ok, cx.ops.le_int)?;
    let Value::If(hi_ok, keep, null_b) = inner.unspan() else {
        return None;
    };
    let hi_ok = compact::call(hi_ok, cx.ops.le_int)?;
    if compact::int(&lo_ok[0]) != Some(0)
        || compact::var(&lo_ok[1]) != Some(*dv)
        || compact::var(&hi_ok[0]) != Some(*dv)
        || compact::int(&hi_ok[1]) != Some(255)
        || compact::var(keep) != Some(*dv)
        || compact::call(null_a, cx.ops.conv_int_from_null).is_none()
        || compact::call(null_b, cx.ops.conv_int_from_null).is_none()
    {
        return None;
    }
    let gb = compact::call(get, cx.get_byte)?;
    if gb.len() != 3 || compact::int(&gb[1]) != Some(0) || compact::int(&gb[2]) != Some(0) {
        return None;
    }
    let elm = compact::call(&gb[0], cx.get_nullable)?;
    if elm.len() != 3 || compact::int(&elm[1]) != Some(1) {
        return None;
    }
    let src = compact::var(&elm[0])?;
    // An offset the guard may read a second time: a literal, or a variable other than the
    // loop's and the source's.
    let pure_off = |o: &Value| match o.unspan() {
        Value::Int(_) => true,
        Value::Var(x) => *x != i && *x != src,
        _ => false,
    };
    let off = match elm[2].unspan() {
        Value::Var(x) if *x == i => None,
        idx => {
            let add = compact::call(idx, cx.ops.add_int)?;
            if compact::var(&add[1]) == Some(i) && pure_off(&add[0]) {
                Some(add[0].unspan().clone())
            } else if compact::var(&add[0]) == Some(i) && pure_off(&add[1]) {
                Some(add[1].unspan().clone())
            } else {
                return None;
            }
        }
    };
    if src == i {
        return None;
    }
    Some(Source::Vector { v: src, off })
}

/// The significant statements of a block named `name`.
fn block_ops<'a>(v: &'a Value, name: &str) -> Option<Vec<&'a Value>> {
    let Value::Block(bl) = v.unspan() else {
        return None;
    };
    (bl.name == name).then(|| {
        compact::significant(&bl.operators)
            .into_iter()
            .map(|(_, op)| op.unspan())
            .collect()
    })
}

/// `if 0 <= lo && lo <= hi && hi <= size(t) { append } else { the loop as written }` for a
/// text; for a vector, `if 0 <= off && 0 <= lo && lo <= hi && hi <= len(v) - off { the slice
/// [off + lo, off + hi) appended } else { the loop as written }` — every index the loop reads
/// in range, and no sum that can overflow (`off` and `len(v) - off` are both non-negative).
fn guarded(c: Copy, original: Value, tp: i32, cx: &Cx) -> Value {
    crate::rewrite_census::fired("R-ByteCopy", 1);
    let (src_var, src_kind) = match c.src {
        Source::Text(t) => (t, "text"),
        Source::Vector { v, .. } => (v, "vector"),
    };
    if crate::keys::trace_byte_copy() {
        eprintln!(
            "[byte-copy] fn={} ADMITTED: `{}` takes the bytes of {src_kind} `{}` as one append",
            cx.data.def(cx.d_nr).name(),
            cx.data.def(cx.d_nr).variables().name(c.buf),
            cx.data.def(cx.d_nr).variables().name(src_var)
        );
    }
    let ops = cx.ops;
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
    let append = match c.src {
        Source::Text(t) => {
            let size = Value::Call(cx.size_text, vec![Value::Var(t)]);
            tests.push(le(Value::Int(0), c.lo.clone()));
            tests.push(le(c.lo.clone(), c.hi.clone()));
            tests.push(le(c.hi.clone(), size));
            Value::Call(
                cx.append,
                vec![Value::Var(c.buf), Value::Var(t), c.lo, c.hi],
            )
        }
        Source::Vector { v, off } => {
            let len = Value::Call(cx.length_vector, vec![Value::Var(v)]);
            let (room, lo, hi) = match off {
                None => (len, c.lo.clone(), c.hi.clone()),
                Some(o) => {
                    tests.extend(not_null(&o));
                    tests.push(le(Value::Int(0), o.clone()));
                    let room = Value::Call(cx.min_int, vec![len, o.clone()]);
                    let add = |b: Value| Value::Call(ops.add_int, vec![o.clone(), b]);
                    (room, add(c.lo.clone()), add(c.hi.clone()))
                }
            };
            tests.push(le(Value::Int(0), c.lo.clone()));
            tests.push(le(c.lo.clone(), c.hi.clone()));
            tests.push(le(c.hi.clone(), room));
            Value::Call(
                cx.slice,
                vec![Value::Var(c.buf), Value::Var(v), lo, hi, Value::Int(tp)],
            )
        }
    };
    let mut cond = tests.pop().expect("at least three tests");
    while let Some(t) = tests.pop() {
        cond = v_if(t, cond, Value::Boolean(false));
    }
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
