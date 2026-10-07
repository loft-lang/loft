// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-Destination` — a call whose result's one heap field is moved whole into a fresh
//! element is built IN that element.
//!
//! `sub = f(args); if sub.ok { e = mint(C); move(sub.value → e); finish(C, e); p = sub.next }`
//! builds `value` in a pooled buffer, then mints `e`, prefills it and relocates `value` into
//! it.  The DESTINATION TWIN `f__d(args, e)` is f's body with its return buffer bound to `e`:
//! the record's field `value` sits at offset 0 of the buffer, so every write the body makes to
//! `value` (its sub-record, its placements in the buffer's store) lands in `e` as it stands,
//! and the scalar fields (`next`, `ok`) — which would land past the element — go to locals
//! the twin answers as a tuple.  The caller mints `e` BEFORE the call and finishes it only
//! where the plain form moved: an element minted and not finished is no member (`@FR-R-PushRec`),
//! and on the failing path the caller releases what the twin placed in it, the release
//! walk the plain form's discarded result runs.
//!
//! Admission asks the CALLEE ([`shape`]): every mention of its return buffer is the buffer's
//! mint guard, a write or sub-record INSIDE the moved field, a write to one of the scalar
//! fields, a placement in the buffer's store, a free compared against it, or the `return` of
//! it — counted mention by mention, so a use no form accounts for declines.  And it asks the
//! CALLER ([`site`]): the result is read only by the `ok` test, the one move, and scalar reads,
//! the element is minted first in the `ok` arm, and no argument of the call reaches the
//! container.  Native only; the interpreter moves as before and is the reference.

use crate::data::{Data, Type, Value};
use crate::database::{Parts, Stores};
use std::collections::HashMap;

/// `LOFT_NO_DESTINATION=1` — every call keeps its buffer and its move.
#[must_use]
pub fn enabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_DESTINATION").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var_os("LOFT_TRACE_DESTINATION").is_some())
}

/// One scalar field of the result record: answered by the twin as a tuple element.
#[derive(Clone, Debug)]
pub struct Scalar {
    pub off: i64,
    /// The Rust type the tuple carries it as: `i64`, `u8` (a boolean's storage), `f64`, `f32`.
    pub rust: &'static str,
    pub setter: &'static str,
    pub getter: &'static str,
}

/// What a destination twin of a callee is: its buffer variable, the moved field's byte range
/// and type, and the scalar fields in offset order.
#[derive(Clone, Debug)]
pub struct Shape {
    pub rb: u16,
    pub vo: i64,
    pub vend: i64,
    pub vt: u16,
    pub scalars: Vec<Scalar>,
}

impl Shape {
    /// The tuple type the twin answers.
    #[must_use]
    pub fn tuple(&self) -> String {
        let parts: Vec<&str> = self.scalars.iter().map(|s| s.rust).collect();
        if parts.len() == 1 {
            format!("({},)", parts[0])
        } else {
            format!("({})", parts.join(", "))
        }
    }

    fn scalar_at(&self, off: i64) -> Option<usize> {
        self.scalars.iter().position(|s| s.off == off)
    }
}

fn scalar_kind(tp: &Type) -> Option<(&'static str, &'static str, &'static str)> {
    match tp.base() {
        // A narrow field is written by a narrow setter, which admission's setter check refuses.
        Type::Integer(_) => Some(("i64", "OpSetInt", "OpGetInt")),
        Type::Boolean => Some(("u8", "OpSetBoolean", "OpGetBoolean")),
        Type::Float => Some(("f64", "OpSetFloat", "OpGetFloat")),
        Type::Single => Some(("f32", "OpSetSingle", "OpGetSingle")),
        _ => None,
    }
}

fn name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn int(v: &Value) -> Option<i64> {
    match v.unspan() {
        Value::Int(i) => Some(i64::from(*i)),
        _ => None,
    }
}

fn is_var(v: &Value, x: u16) -> bool {
    matches!(v.unspan(), Value::Var(y) if *y == x)
}

/// Does callee `d_nr` have a destination twin for its result field at byte offset `vo`?
/// Memoised per `(callee, offset)`.
pub fn shape(
    data: &Data,
    stores: &Stores,
    d_nr: u32,
    vo: i64,
    memo: &mut HashMap<(u32, i64), Option<Shape>>,
) -> Option<Shape> {
    if let Some(known) = memo.get(&(d_nr, vo)) {
        return known.clone();
    }
    let out = compute_shape(data, stores, d_nr, vo);
    if trace() {
        match &out {
            Some(s) => eprintln!(
                "destination: {} has a twin for +{vo} answering {}",
                data.def(d_nr).name(),
                s.tuple()
            ),
            None => eprintln!(
                "destination: {} has no twin for +{vo}",
                data.def(d_nr).name()
            ),
        }
    }
    memo.insert((d_nr, vo), out.clone());
    out
}

fn compute_shape(data: &Data, stores: &Stores, d_nr: u32, vo: i64) -> Option<Shape> {
    let def = data.def(d_nr);
    if !def.is_loft_defined() || matches!(def.returned(), Type::Iterator(_, _)) {
        return None;
    }
    let rb = super::hoist::retbuf_var(data, d_nr)?;
    let vars = def.variables();
    let Type::Reference(rd, _) = vars.tp(rb).base() else {
        return None;
    };
    let rd = *rd;
    let kt = data.def(rd).known_type();
    let Some(Parts::Struct(fields)) = stores.types.get(kt as usize).map(|t| &t.parts) else {
        return None;
    };
    let mut moved: Option<(i64, u16)> = None;
    let mut scalars = Vec::new();
    for f in fields {
        let off = i64::from(f.position);
        if off == vo {
            moved = Some((off, f.content));
            continue;
        }
        let attr = data
            .def(rd)
            .attributes()
            .iter()
            .find(|a| a.name == f.name)?;
        let (rust, setter, getter) = scalar_kind(&attr.typedef)?;
        scalars.push(Scalar {
            off,
            rust,
            setter,
            getter,
        });
    }
    let (vo, vt) = moved?;
    if scalars.is_empty() {
        return None;
    }
    scalars.sort_by_key(|s| s.off);
    let vend = vo + i64::from(stores.size(vt));
    let shape = Shape {
        rb,
        vo,
        vend,
        vt,
        scalars,
    };
    // Every mention of the buffer, then the mentions each admitted form accounts for.
    let mut total = 0usize;
    def.code().walk(&mut |n| {
        if is_var(n, rb) {
            total += 1;
        }
    });
    let mut accounted = 0usize;
    let mut bad = false;
    def.code().walk(&mut |n| match n {
        Value::Return(x) if is_var(x, rb) => accounted += 1,
        Value::Call(d, args) => {
            let op = name(data, *d);
            let first = args.first().is_some_and(|a| is_var(a, rb));
            if op == "OpFreeRefIfDistinct" && args.get(1).is_some_and(|a| is_var(a, rb)) {
                accounted += 1;
            }
            if !first {
                return;
            }
            match op {
                "OpRefIsNull" | "OpConvBoolFromRef" | "OpDatabase" | "OpPlaceRecord" => {
                    accounted += 1;
                }
                "OpGetField" => {
                    if let (Some(off), Some(tp)) =
                        (args.get(1).and_then(int), args.get(2).and_then(int))
                        && let Ok(tp) = u16::try_from(tp)
                        && off >= shape.vo
                        && off + i64::from(stores.size(tp)) <= shape.vend
                    {
                        accounted += 1;
                    } else {
                        bad = true;
                    }
                }
                _ if op.starts_with("OpSet") => {
                    let Some(off) = args.get(1).and_then(int) else {
                        bad = true;
                        return;
                    };
                    if off >= shape.vo && off < shape.vend {
                        accounted += 1;
                    } else if let Some(k) = shape.scalar_at(off)
                        && shape.scalars[k].setter == op
                    {
                        accounted += 1;
                    } else {
                        bad = true;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    });
    if bad || accounted != total {
        if trace() {
            eprintln!(
                "destination: {} declines — its buffer has {total} mentions, {accounted} accounted{}",
                def.name(),
                if bad {
                    ", one at an offset no form serves"
                } else {
                    ""
                }
            );
        }
        return None;
    }
    Some(shape)
}

/// A caller window the twin replaces.
pub struct Site<'a> {
    /// The statements the window consumes, as indices into the block: the buffer's refill
    /// (when present) through the free of the result.
    pub first: usize,
    pub last: usize,
    pub callee: u32,
    pub call_args: &'a [Value],
    pub at: usize,
    pub shape: Shape,
    /// The `ok` field's tuple index.
    pub ok: usize,
    /// The element's mint: the optional reservation and the `Set(e, OpNewRecord(…))`.
    pub mint: Vec<&'a Value>,
    /// Where the twin writes: `e`, or a field of it.
    pub dest: Value,
    /// The `ok` arm with the mint, the move and any zero of the destination removed, and the
    /// scalar reads of the result replaced by `__dsN.k`, where `N` is filled in by the emitter.
    pub then_ops: Vec<Value>,
    pub then_block: &'a crate::data::Block,
    pub else_v: &'a Value,
    /// The result variable, whose scalar reads `then_ops` names by tuple index.
    pub r: u16,
}

fn skip_lines(ops: &[Value], mut i: usize) -> usize {
    while matches!(ops.get(i), Some(Value::Line(_))) {
        i += 1;
    }
    i
}

/// Every mention of `x` in `v`.
fn mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |m| {
        if is_var(m, x) {
            n += 1;
        }
    });
    n
}

/// The window starting at statement `i` of `ops`, in function `d_nr`, when it has the shape
/// [`Site`] describes and its callee a twin.
#[allow(clippy::too_many_lines)]
pub fn site<'a>(
    ops: &'a [Value],
    i: usize,
    data: &Data,
    stores: &Stores,
    d_nr: u32,
    memo: &mut HashMap<(u32, i64), Option<Shape>>,
) -> Option<Site<'a>> {
    // The buffer's refill, when the pooled buffer has one: `if null { place } else { clear }`.
    let mut k = i;
    let mut buf_refill: Option<u16> = None;
    if let Value::If(test, _, _) = ops.get(k)?.unspan()
        && let Value::Call(d, a) = test.unspan()
        && name(data, *d) == "OpRefIsNull"
        && let Some(Value::Var(b)) = a.first().map(Value::unspan)
    {
        buf_refill = Some(*b);
        k = skip_lines(ops, k + 1);
    }
    let Value::Set(r, call) = ops.get(k)?.unspan() else {
        return None;
    };
    let r = *r;
    let Value::Call(callee, call_args) = call.unspan() else {
        return None;
    };
    let at = data.def(*callee).hidden_return_buffer_attr()?;
    let Some(Value::Var(b)) = call_args.get(at).map(Value::unspan) else {
        return None;
    };
    if buf_refill.is_some_and(|x| x != *b) {
        return None;
    }
    let b = *b;
    let ki = skip_lines(ops, k + 1);
    let Value::If(test, then_v, else_v) = ops.get(ki)?.unspan() else {
        return None;
    };
    let fi = skip_lines(ops, ki + 1);
    let Value::Call(fd, fa) = ops.get(fi)?.unspan() else {
        return None;
    };
    if name(data, *fd) != "OpFreeRefIfDistinct"
        || !fa.first().is_some_and(|a| is_var(a, r))
        || !fa.get(1).is_some_and(|a| is_var(a, b))
    {
        return None;
    }
    let Value::Call(td, ta) = test.unspan() else {
        return None;
    };
    if name(data, *td) != "OpGetBoolean" || !ta.first().is_some_and(|a| is_var(a, r)) {
        return None;
    }
    let ok_off = ta.get(1).and_then(int)?;
    let Value::Block(tb) = then_v.unspan() else {
        return None;
    };
    // The mint first: an optional reservation, then `Set(e, OpNewRecord(C, …))`.
    let t_ops = &tb.operators;
    let mut j = skip_lines(t_ops, 0);
    let mut mint: Vec<&Value> = Vec::new();
    if let Value::Call(d, _) = t_ops.get(j)?.unspan()
        && name(data, *d) == "OpPreAllocVector"
    {
        mint.push(&t_ops[j]);
        j = skip_lines(t_ops, j + 1);
    }
    let Value::Set(e, nr) = t_ops.get(j)?.unspan() else {
        return None;
    };
    let e = *e;
    let Value::Call(nd, na) = nr.unspan() else {
        return None;
    };
    if name(data, *nd) != "OpNewRecord" {
        return None;
    }
    let Some(Value::Var(container)) = na.first().map(Value::unspan) else {
        return None;
    };
    let container = *container;
    mint.push(&t_ops[j]);
    let mint_end = j;
    // The one move of the result's heap field into the element or a field of it.
    let mut moved: Option<(usize, i64, Value)> = None;
    for (idx, op) in t_ops.iter().enumerate().skip(mint_end + 1) {
        if let Value::Call(d, a) = op.unspan()
            && name(data, *d) == "OpMoveField"
            && let [src, dst, _] = &a[..]
            && let Value::Call(g, ga) = src.unspan()
            && name(data, *g) == "OpGetField"
            && ga.first().is_some_and(|x| is_var(x, r))
        {
            if moved.is_some() {
                return None;
            }
            let vo = ga.get(1).and_then(int)?;
            let dest_ok = is_var(dst, e)
                || matches!(dst.unspan(), Value::Call(g2, ga2)
                    if name(data, *g2) == "OpGetField" && ga2.first().is_some_and(|x| is_var(x, e)));
            if !dest_ok {
                return None;
            }
            moved = Some((idx, vo, dst.clone()));
        }
    }
    let (move_idx, vo, dest) = moved?;
    let shape = shape(data, stores, *callee, vo, memo)?;
    let ok = shape.scalar_at(ok_off)?;
    if shape.scalars[ok].rust != "u8" {
        return None;
    }
    // The destination's own byte range inside `e`, for the zero-inits the twin's writes replace.
    let (d_lo, d_hi) = match dest.unspan() {
        Value::Call(_, ga) => {
            let off = ga.get(1).and_then(int)?;
            (off, off + (shape.vend - shape.vo))
        }
        _ => (0, shape.vend - shape.vo),
    };
    // The `ok` arm, rewritten: the mint and the move gone, a zero of the destination gone, the
    // result's scalar reads named by tuple index.  Any other mention of the result declines,
    // as does a mention of the element before the move that is not such a zero.
    let mut then_ops: Vec<Value> = Vec::new();
    for (idx, op) in t_ops.iter().enumerate() {
        if idx <= mint_end && !matches!(op, Value::Line(_)) {
            continue;
        }
        if idx == move_idx {
            continue;
        }
        if idx < move_idx
            && let Value::Call(d, a) = op.unspan()
            && name(data, *d) == "OpSetInt4"
            && a.first().is_some_and(|x| is_var(x, e))
            && let (Some(off), Some(0)) = (a.get(1).and_then(int), a.get(2).and_then(int))
            && off >= d_lo
            && off < d_hi
        {
            continue;
        }
        if idx < move_idx && mentions(op, e) > 0 {
            return None;
        }
        let mut rewritten = op.clone();
        let mut bad = false;
        rewrite_reads(&mut rewritten, r, &shape, data, &mut bad);
        if bad || mentions(&rewritten, r) > 0 {
            return None;
        }
        then_ops.push(rewritten);
    }
    if mentions(else_v, r) > 0 || mentions(else_v, e) > 0 {
        return None;
    }
    // No argument reaches the container: the element waits unfinished across the call.
    let reach = super::append_twin::views(data.def(d_nr).variables(), container);
    for (idx, a) in call_args.iter().enumerate() {
        if idx == at {
            continue;
        }
        if reach.iter().any(|v| mentions(a, *v) > 0) || mentions(a, r) > 0 {
            return None;
        }
    }
    // The result is the window's alone: every mention of it in the function lies inside.
    let window: usize = ops[i..=fi].iter().map(|op| mentions(op, r)).sum();
    if mentions(data.def(d_nr).code(), r) != window {
        return None;
    }
    if trace() {
        eprintln!(
            "destination: {} builds {}(…) in its element",
            data.def(d_nr).name(),
            data.def(*callee).name()
        );
    }
    Some(Site {
        first: i,
        last: fi,
        callee: *callee,
        call_args,
        at,
        shape,
        ok,
        mint,
        dest,
        then_ops,
        then_block: tb,
        else_v,
        r,
    })
}

/// Replace every scalar read of result `r` in `v` by the placeholder `__ds#.k` (the emitter
/// substitutes the tuple's name for `#`).  A read of `r` that is not a scalar getter at a
/// scalar offset sets `bad`.
fn rewrite_reads(v: &mut Value, r: u16, shape: &Shape, data: &Data, bad: &mut bool) {
    if let Value::Call(d, a) = v
        && a.first().is_some_and(|x| is_var(x, r))
    {
        let op = name(data, *d);
        if let Some(off) = a.get(1).and_then(int)
            && let Some(k) = shape.scalar_at(off)
            && shape.scalars[k].getter == op
        {
            *v = Value::RawExpr(format!("__ds#.{k}"));
            return;
        }
        *bad = true;
        return;
    }
    v.for_each_child_mut(&mut |c| rewrite_reads(c, r, shape, data, bad));
}
