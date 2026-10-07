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

/// The callee's buffer mentions: how many there are, how many an admitted form accounts for,
/// and whether one sits at an offset no form serves.
fn buffer_uses(data: &Data, stores: &Stores, code: &Value, shape: &Shape) -> (usize, usize, bool) {
    let rb = shape.rb;
    let mut total = 0usize;
    code.walk(&mut |n| {
        if is_var(n, rb) {
            total += 1;
        }
    });
    let mut accounted = 0usize;
    let mut bad = false;
    code.walk(&mut |n| match n {
        Value::Return(x) if is_var(x, rb) => accounted += 1,
        Value::Call(d, args) => {
            let op = name(data, *d);
            if op == "OpFreeRefIfDistinct" && args.get(1).is_some_and(|a| is_var(a, rb)) {
                accounted += 1;
            }
            if !args.first().is_some_and(|a| is_var(a, rb)) {
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
                _ if op.starts_with("OpSet") => match args.get(1).and_then(int) {
                    Some(off) if off >= shape.vo && off < shape.vend => accounted += 1,
                    Some(off)
                        if shape
                            .scalar_at(off)
                            .is_some_and(|k| shape.scalars[k].setter == op) =>
                    {
                        accounted += 1;
                    }
                    _ => bad = true,
                },
                _ => {}
            }
        }
        _ => {}
    });
    (total, accounted, bad)
}

fn compute_shape(data: &Data, stores: &Stores, d_nr: u32, vo: i64) -> Option<Shape> {
    let def = data.def(d_nr);
    if !def.is_loft_defined() || matches!(def.returned().base(), Type::Iterator(_, _)) {
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
    // The twin binds the buffer TO the destination, so the moved field must start where the
    // buffer does: at any other offset every write to it would land that far past the element.
    if vo != 0 || scalars.is_empty() {
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
    let (total, accounted, bad) = buffer_uses(data, stores, def.code(), &shape);
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

/// One call of a destination chain: the call, its twin's shape, where it builds, and the
/// statements of its `ok` arm around the next level (or, at the last level, the arm's rest).
pub struct Level<'a> {
    pub callee: u32,
    pub call_args: Vec<Value>,
    pub at: usize,
    pub shape: Shape,
    /// The `ok` field's tuple index.
    pub ok: usize,
    /// Where the twin writes: the element, or a field of it.
    pub dest: Value,
    /// The `ok` arm's statements before the next level (all of them at the last level), with
    /// the chain's scalar reads named `__ds#m#.k` for level `m`'s tuple.
    pub pre: Vec<Value>,
    pub inner: Option<Box<Level<'a>>>,
    /// The `ok` arm's statements after the next level.
    pub post: Vec<Value>,
    pub else_v: &'a Value,
}

/// A caller window the twins replace: the statements it consumes, the element's mint (hoisted
/// above the first call) and the chain of calls.
pub struct Site<'a> {
    pub first: usize,
    pub last: usize,
    pub mint: Vec<&'a Value>,
    pub top: Level<'a>,
}

impl Site<'_> {
    /// Every callee of the chain, outermost first.
    #[must_use]
    pub fn callees(&self) -> Vec<u32> {
        let mut out = Vec::new();
        let mut l = Some(&self.top);
        while let Some(level) = l {
            out.push(level.callee);
            l = level.inner.as_deref();
        }
        out
    }
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

/// One window: the optional refill of the pooled buffer, `r = f(…, B)`, `if r.ok { … } else
/// { … }` and the free of `r` against `B`.
struct Window<'a> {
    first: usize,
    last: usize,
    r: u16,
    callee: u32,
    call_args: &'a [Value],
    at: usize,
    ok_off: i64,
    then_ops: &'a [Value],
    else_v: &'a Value,
}

fn window<'a>(ops: &'a [Value], i: usize, data: &Data) -> Option<Window<'a>> {
    let mut k = i;
    let mut refill: Option<u16> = None;
    if let Value::If(test, _, _) = ops.get(k)?.unspan()
        && let Value::Call(d, a) = test.unspan()
        && name(data, *d) == "OpRefIsNull"
        && let Some(Value::Var(b)) = a.first().map(Value::unspan)
    {
        refill = Some(*b);
        k = skip_lines(ops, k + 1);
    }
    let Value::Set(r, call) = ops.get(k)?.unspan() else {
        return None;
    };
    let Value::Call(callee, call_args) = call.unspan() else {
        return None;
    };
    let at = data.def(*callee).hidden_return_buffer_attr()?;
    let Some(Value::Var(b)) = call_args.get(at).map(Value::unspan) else {
        return None;
    };
    if refill.is_some_and(|x| x != *b) {
        return None;
    }
    let ki = skip_lines(ops, k + 1);
    let Value::If(test, then_v, else_v) = ops.get(ki)?.unspan() else {
        return None;
    };
    let fi = skip_lines(ops, ki + 1);
    let Value::Call(fd, fa) = ops.get(fi)?.unspan() else {
        return None;
    };
    if name(data, *fd) != "OpFreeRefIfDistinct"
        || !fa.first().is_some_and(|a| is_var(a, *r))
        || !fa.get(1).is_some_and(|a| is_var(a, *b))
    {
        return None;
    }
    let Value::Call(td, ta) = test.unspan() else {
        return None;
    };
    if name(data, *td) != "OpGetBoolean" || !ta.first().is_some_and(|a| is_var(a, *r)) {
        return None;
    }
    let Value::Block(tb) = then_v.unspan() else {
        return None;
    };
    Some(Window {
        first: i,
        last: fi,
        r: *r,
        callee: *callee,
        call_args,
        at,
        ok_off: ta.get(1).and_then(int)?,
        then_ops: &tb.operators,
        else_v,
    })
}

/// The chain of windows from statement `i`: each level's window found inside the `ok` arm of
/// the one before, with where in that arm it starts and ends.
fn chain<'a>(
    ops: &'a [Value],
    i: usize,
    data: &Data,
) -> Option<(Vec<Window<'a>>, Vec<(usize, usize)>)> {
    let mut wins: Vec<Window<'a>> = vec![window(ops, i, data)?];
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    loop {
        let t = wins.last()?.then_ops;
        let found = (0..t.len()).find_map(|j| window(t, j, data).map(|w| (j, w)));
        match found {
            Some((j, w)) if wins.len() < 8 => {
                cuts.push((j, w.last));
                wins.push(w);
            }
            Some(_) => return None,
            None => return Some((wins, cuts)),
        }
    }
}

/// What the chain's last `ok` arm holds: the element's mint and where it stands, the element
/// and its container, the alias locals of the results, each result's destination, and the
/// statements the twins replace (the aliases and the moves).
struct LastArm<'a> {
    mint: Vec<&'a Value>,
    mint_j: usize,
    e: u16,
    container: u16,
    aliases: HashMap<u16, usize>,
    dests: Vec<Value>,
    drop: Vec<usize>,
}

fn last_arm<'a>(t_ops: &'a [Value], rs: &[u16], data: &Data) -> Option<LastArm<'a>> {
    let mut mint: Vec<&'a Value> = Vec::new();
    let mut mint_at: Option<(usize, u16, u16)> = None;
    for (j, op) in t_ops.iter().enumerate() {
        if let Value::Set(e, nr) = op.unspan()
            && let Value::Call(nd, na) = nr.unspan()
            && name(data, *nd) == "OpNewRecord"
            && let Some(Value::Var(c)) = na.first().map(Value::unspan)
        {
            if mint_at.is_some() {
                return None;
            }
            let prev = (0..j).rev().find(|&q| !matches!(t_ops[q], Value::Line(_)));
            if let Some(q) = prev
                && let Value::Call(d, a) = t_ops[q].unspan()
                && name(data, *d) == "OpPreAllocVector"
                && a.first().is_some_and(|x| is_var(x, *c))
            {
                mint.push(&t_ops[q]);
            }
            mint.push(op);
            mint_at = Some((j, *e, *c));
        }
    }
    let (mint_j, e, container) = mint_at?;
    let level_of = |v: &Value| rs.iter().position(|r| is_var(v, *r));
    let mut aliases: HashMap<u16, usize> = HashMap::new();
    let mut dests: Vec<Option<Value>> = vec![None; rs.len()];
    let mut drop: Vec<usize> = Vec::new();
    for (j, op) in t_ops.iter().enumerate() {
        if let Value::Set(a, g) = op.unspan()
            && let Value::Call(gd, ga) = g.unspan()
            && name(data, *gd) == "OpGetField"
            && ga.get(1).and_then(int) == Some(0)
            && let Some(m) = ga.first().and_then(level_of)
        {
            aliases.insert(*a, m);
            drop.push(j);
            continue;
        }
        if let Value::Call(d, a) = op.unspan()
            && name(data, *d) == "OpMoveField"
            && let [src, dst, _] = &a[..]
        {
            let m = match src.unspan() {
                Value::Var(x) => aliases.get(x).copied(),
                Value::Call(g, ga)
                    if name(data, *g) == "OpGetField" && ga.get(1).and_then(int) == Some(0) =>
                {
                    ga.first().and_then(level_of)
                }
                _ => None,
            };
            let Some(m) = m else { continue };
            let dest_ok = is_var(dst, e)
                || matches!(dst.unspan(), Value::Call(g2, ga2)
                    if name(data, *g2) == "OpGetField" && ga2.first().is_some_and(|x| is_var(x, e)));
            if !dest_ok || dests[m].is_some() || j < mint_j {
                return None;
            }
            dests[m] = Some(dst.clone());
            drop.push(j);
        }
    }
    Some(LastArm {
        mint,
        mint_j,
        e,
        container,
        aliases,
        dests: dests.into_iter().collect::<Option<Vec<_>>>()?,
        drop,
    })
}

/// The zero-inits of the element's bytes the twins fill: their writes replace them, and left in
/// place after the calls they would wipe what the twins built.
fn dest_zeros(
    t_ops: &[Value],
    e: u16,
    dests: &[Value],
    shapes: &[Shape],
    data: &Data,
) -> Vec<usize> {
    let ranges: Vec<(i64, i64)> = dests
        .iter()
        .zip(shapes)
        .map(|(d, sh)| {
            let lo = match d.unspan() {
                Value::Call(_, ga) => ga.get(1).and_then(int).unwrap_or(0),
                _ => 0,
            };
            (lo, lo + (sh.vend - sh.vo))
        })
        .collect();
    let mut out = Vec::new();
    for (j, op) in t_ops.iter().enumerate() {
        if let Value::Call(d, a) = op.unspan()
            && name(data, *d) == "OpSetInt4"
            && a.first().is_some_and(|x| is_var(x, e))
            && let (Some(off), Some(0)) = (a.get(1).and_then(int), a.get(2).and_then(int))
            && ranges.iter().any(|(lo, hi)| off >= *lo && off < *hi)
        {
            out.push(j);
        }
    }
    out
}

/// Does every mention of each result in the function's `code` lie in `window`?
fn owned_by_window(window: &[Value], code: &Value, rs: &[u16]) -> bool {
    rs.iter().all(|r| {
        let inside: usize = window.iter().map(|op| mentions(op, *r)).sum();
        mentions(code, *r) == inside
    })
}

/// The chain's levels as one tree, outermost first.
fn nest<'a>(
    wins: &[Window<'a>],
    shapes: Vec<Shape>,
    args: Vec<Vec<Value>>,
    dests: Vec<Value>,
    levels: Vec<(Vec<Value>, Vec<Value>)>,
) -> Option<Level<'a>> {
    let mut top: Option<Level<'a>> = None;
    let parts = shapes.into_iter().zip(args).zip(dests).zip(levels);
    for (w, (((sh, call_args), dest), (pre, post))) in wins.iter().zip(parts).rev() {
        let ok = sh.scalar_at(w.ok_off)?;
        if sh.scalars[ok].rust != "u8" {
            return None;
        }
        top = Some(Level {
            callee: w.callee,
            call_args,
            at: w.at,
            shape: sh,
            ok,
            dest,
            pre,
            inner: top.map(Box::new),
            post,
            else_v: w.else_v,
        });
    }
    top
}

fn trace_site(data: &Data, d_nr: u32, calls: usize, callee: u32) {
    if trace() {
        eprintln!(
            "destination: {} builds {calls} call(s) of {}(…) in one element",
            data.def(d_nr).name(),
            data.def(callee).name()
        );
    }
}

/// The window chain from statement `i` of `ops`, in function `d_nr`, when every call in it
/// has a twin, the last `ok` arm mints one element and moves each call's result into it once,
/// and the results are read otherwise only as scalars.
pub fn site<'a>(
    ops: &'a [Value],
    i: usize,
    data: &Data,
    stores: &Stores,
    d_nr: u32,
    memo: &mut HashMap<(u32, i64), Option<Shape>>,
) -> Option<Site<'a>> {
    let (wins, cuts) = chain(ops, i, data)?;
    let rs: Vec<u16> = wins.iter().map(|w| w.r).collect();
    let t_ops = wins.last()?.then_ops;
    let LastArm {
        mint,
        mint_j,
        e,
        container,
        aliases,
        dests,
        mut drop,
    } = last_arm(t_ops, &rs, data)?;
    let shapes: Vec<Shape> = wins
        .iter()
        .map(|w| shape(data, stores, w.callee, 0, memo))
        .collect::<Option<_>>()?;
    drop.extend(dest_zeros(t_ops, e, &dests, &shapes, data));
    drop.extend(
        (0..t_ops.len()).filter(|&j| mint.iter().any(|m| std::ptr::eq(&raw const t_ops[j], *m))),
    );
    // Rewrite one statement: the chain's scalar reads named, any other use of a result, an
    // alias or (before the mint) the element declines.
    let rewrite = |op: &Value, before_mint: bool| -> Option<Value> {
        if before_mint && mentions(op, e) > 0 {
            return None;
        }
        let mut v = op.clone();
        let mut bad = false;
        for (m, r) in rs.iter().enumerate() {
            rewrite_reads(&mut v, *r, m, &shapes[m], data, &mut bad);
        }
        if bad
            || rs.iter().any(|r| mentions(&v, *r) > 0)
            || aliases.keys().any(|a| mentions(&v, *a) > 0)
        {
            return None;
        }
        Some(v)
    };
    // Each level's arm, around the next level's window.
    let mut levels: Vec<(Vec<Value>, Vec<Value>)> = Vec::new();
    for (m, w) in wins.iter().enumerate() {
        let (pre_src, post_src): (Vec<(usize, &Value)>, Vec<(usize, &Value)>) =
            if m + 1 < wins.len() {
                let (c0, c1) = cuts[m];
                (
                    w.then_ops.iter().enumerate().take(c0).collect(),
                    w.then_ops.iter().enumerate().skip(c1 + 1).collect(),
                )
            } else {
                (w.then_ops.iter().enumerate().collect(), Vec::new())
            };
        let last = m + 1 == wins.len();
        let mut pre = Vec::new();
        for (j, op) in pre_src {
            if last && drop.contains(&j) {
                continue;
            }
            pre.push(rewrite(op, last && j < mint_j)?);
        }
        let mut post = Vec::new();
        for (_, op) in post_src {
            post.push(rewrite(op, false)?);
        }
        levels.push((pre, post));
        if rs.iter().any(|r| mentions(w.else_v, *r) > 0) || mentions(w.else_v, e) > 0 {
            return None;
        }
    }
    // The calls' arguments: no reach into the container; a scalar of an earlier result named.
    let reach = super::append_twin::views(data.def(d_nr).variables(), container);
    let mut args_rw: Vec<Vec<Value>> = Vec::new();
    for w in &wins {
        let mut out = Vec::new();
        for (idx, a) in w.call_args.iter().enumerate() {
            if idx == w.at {
                out.push(a.clone());
                continue;
            }
            if reach.iter().any(|v| mentions(a, *v) > 0) {
                return None;
            }
            out.push(rewrite(a, false)?);
        }
        args_rw.push(out);
    }
    // The results belong to the window: every mention of them lies inside it.
    let w0 = &wins[0];
    if !owned_by_window(&ops[w0.first..=w0.last], data.def(d_nr).code(), &rs) {
        return None;
    }
    let top = nest(&wins, shapes, args_rw, dests, levels)?;
    trace_site(data, d_nr, wins.len(), wins[0].callee);
    Some(Site {
        first: w0.first,
        last: w0.last,
        mint,
        top,
    })
}

/// Replace every scalar read of result `r` (chain level `m`) in `v` by `__ds#m#.k`, which the
/// emitter resolves to that level's tuple.  A read of `r` that is not a scalar getter at a
/// scalar offset sets `bad`.
fn rewrite_reads(v: &mut Value, r: u16, m: usize, shape: &Shape, data: &Data, bad: &mut bool) {
    if let Value::Call(d, a) = v
        && a.first().is_some_and(|x| is_var(x, r))
    {
        let op = name(data, *d);
        if let Some(off) = a.get(1).and_then(int)
            && let Some(k) = shape.scalar_at(off)
            && shape.scalars[k].getter == op
        {
            *v = Value::RawExpr(format!("__ds#{m}#.{k}"));
            return;
        }
        *bad = true;
        return;
    }
    v.for_each_child_mut(&mut |c| rewrite_reads(c, r, m, shape, data, bad));
}
