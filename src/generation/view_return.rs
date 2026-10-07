// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-ViewReturn` — a lookup's found entry answered as a view the caller reads in place.
//!
//! `fn get(m: R, k: text) -> V { for e in m.entries { if … { return e.value; } } V0 }` returns
//! a VIEW of its parameter's record, which the parser materialises: the caller mints a buffer
//! store per call, the callee deep-copies the entry into it, and the caller frees it after
//! reading one field.  The callee's TWIN (`__vr`) answers `(address, owned)`: a materialised
//! view exit answers the view's address and `false`, every other exit its own store and
//! `true`.  The caller mints no buffer, and frees the result only when it is owned — so every
//! exit keeps the ownership it had, and only the mint and the copy of a view go.
//!
//! The view is read where the caller's argument still holds it: the caller's admission is a
//! window — the statements after the call in its block — that writes no store, calls no user
//! function and reads the result only through getters that copy (a tag, a scalar, a text).
//! The callee's admission is a body that writes only stores it minted itself.  Native only.

use crate::data::{Data, Value};

/// `LOFT_NO_VIEW_RETURN=1` — every view return is materialised again.
#[must_use]
pub fn enabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_VIEW_RETURN").is_ok_and(|v| v != "0"))
}

/// `LOFT_TRACE_VIEW_RETURN=1` — names each call site's admission or declining condition.
fn trace() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_TRACE_VIEW_RETURN").is_ok_and(|v| v != "0"))
}

fn name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn is_var(v: &Value, x: u16) -> bool {
    matches!(v.unspan(), Value::Var(y) if *y == x)
}

fn mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.walk(&mut |m| {
        if is_var(m, x) {
            n += 1;
        }
    });
    n
}

/// An operator that only READS a store: it may run between the view's answer and its last
/// read, and in the callee outside its exits.
fn read_op(n: &str) -> bool {
    const PREFIX: &[&str] = &[
        "OpGet",
        "OpEq",
        "OpNe",
        "OpLt",
        "OpLe",
        "OpGt",
        "OpGe",
        "OpAdd",
        "OpMin",
        "OpMul",
        "OpDiv",
        "OpRem",
        "OpConv",
        "OpLength",
        "OpNot",
        "OpAnd",
        "OpOr",
        "OpRefIsNull",
        "OpFreeText",
        "OpSize",
        "OpCast",
        "OpAbs",
        "OpNeg",
        "OpLogical",
        "OpExclusive",
    ];
    // Names a prefix catches that write, mint or reach outside the program: `OpRem` is also
    // `OpRemove`, `OpNe` is also `OpNewRecord`, `OpCast` also builds a vector from a text.
    const WRITES: &[&str] = &[
        "OpRemove",
        "OpRemoveVector",
        "OpNewRecord",
        "OpCastVectorFromText",
        "OpGetFile",
        "OpGetDir",
        "OpGetFileText",
    ];
    PREFIX.iter().any(|p| n.starts_with(p)) && !WRITES.contains(&n)
}

/// A getter that copies its answer out of the record: a tag, a scalar, a text.
fn copying_getter(n: &str) -> bool {
    matches!(
        n,
        "OpGetEnum"
            | "OpGetText"
            | "OpGetInt"
            | "OpGetLong"
            | "OpGetByte"
            | "OpGetByteNullable"
            | "OpGetShort"
            | "OpGetShortRaw"
            | "OpGetShortSpare"
            | "OpGetShortFull"
            | "OpGetCharacter"
            | "OpGetFloat"
            | "OpGetSingle"
            | "OpGetBoolean"
    )
}

/// Is `d` a function the clause may emit a twin of?  Its return buffer appears only in its
/// materialised view exits, it has at least one, and outside them it writes only stores it
/// minted and calls no user function.
pub fn callee(data: &Data, d: u32) -> bool {
    let def = data.def(d);
    let Some(ai) = super::hoist::ret_buffer_attr(def) else {
        return false;
    };
    if matches!(def.returned().base(), crate::data::Type::Iterator(_, _)) {
        return false;
    }
    let vars = def.variables();
    let rb = vars.var(&def.attributes()[ai].name);
    if rb == u16::MAX {
        return false;
    }
    let args: Vec<u16> = vars.arguments();
    // The locals the function mints: what it may write and hand up as owned.
    let mut own: Vec<u16> = Vec::new();
    def.code().walk(&mut |n| {
        if let Value::Call(c, a) = n
            && matches!(name(data, *c), "OpDatabase" | "OpDatabaseNP")
            && let Some(Value::Var(x)) = a.first().map(Value::unspan)
            && *x != rb
            && !args.contains(x)
        {
            own.push(*x);
        }
    });
    let mut exits = 0usize;
    let ok = body_ok(data, def.code(), rb, &own, &mut exits);
    ok && exits > 0 && ends_in_exit(def.code())
}

/// Does the body end in a `return` (or diverge), so no value leaves it but through one?
fn ends_in_exit(code: &Value) -> bool {
    let Value::Block(bl) = code.unspan() else {
        return false;
    };
    bl.operators
        .iter()
        .rev()
        .find(|o| !matches!(o, Value::Line(_)))
        .is_some_and(|o| matches!(o.unspan(), Value::Return(_)))
}

/// A materialised view exit: `{ OpDatabase(rb); OpCopyRecord(src, rb); frees…; return rb }`.
/// Answers the view's source.
pub fn view_exit<'a>(data: &Data, v: &'a Value, rb: u16) -> Option<&'a Value> {
    let Value::Block(bl) = v.unspan() else {
        return None;
    };
    if bl.name != "materialized_view_return" {
        return None;
    }
    let ops: Vec<&Value> = bl
        .operators
        .iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let [mint, copy, frees @ .., ret] = &ops[..] else {
        return None;
    };
    let Value::Call(m, ma) = mint.unspan() else {
        return None;
    };
    if name(data, *m) != "OpDatabase" || !ma.first().is_some_and(|x| is_var(x, rb)) {
        return None;
    }
    let Value::Call(c, ca) = copy.unspan() else {
        return None;
    };
    if name(data, *c) != "OpCopyRecord" || ca.len() != 3 || !is_var(&ca[1], rb) {
        return None;
    }
    let src = &ca[0];
    if mentions(src, rb) != 0 || !pure_read(data, src) {
        return None;
    }
    for f in frees {
        let Value::Call(fd, fa) = f.unspan() else {
            return None;
        };
        let ok = match name(data, *fd) {
            "OpFreeText" => true,
            "OpFreeRefIfDistinct" => fa.len() == 2 && !is_var(&fa[0], rb) && is_var(&fa[1], rb),
            "OpFreeRef" => fa.first().is_some_and(|x| !is_var(x, rb)),
            _ => false,
        };
        if !ok {
            return None;
        }
    }
    matches!(ret.unspan(), Value::Return(r) if is_var(r, rb)).then_some(src)
}

/// Every call in `v` is a read.
fn pure_read(data: &Data, v: &Value) -> bool {
    let mut ok = true;
    v.walk(&mut |n| {
        if let Value::Call(c, _) = n
            && !read_op(name(data, *c))
        {
            ok = false;
        }
    });
    ok
}

/// The callee's body outside its view exits: `rb` unnamed, writes only into `own`, no user
/// calls, every other `return` handing up an own store.
fn body_ok(data: &Data, val: &Value, rb: u16, own: &[u16], exits: &mut usize) -> bool {
    if view_exit(data, val, rb).is_some() {
        *exits += 1;
        return true;
    }
    match val.unspan() {
        Value::Var(x) => *x != rb,
        Value::Call(callee, vals) => {
            let op = name(data, *callee);
            let into_own = vals
                .first()
                .is_some_and(|x| matches!(x.unspan(), Value::Var(y) if own.contains(y)));
            let allowed = read_op(op)
                || (op.starts_with("Op") && into_own && !data.def(*callee).name().is_empty())
                || (op == "OpFreeRefIfDistinct" && into_own);
            allowed && vals.iter().all(|x| body_ok(data, x, rb, own, exits))
        }
        Value::Return(r) => owned_value(r, own) && body_ok(data, r, rb, own, exits),
        Value::Set(x, init) => *x != rb && body_ok(data, init, rb, own, exits),
        Value::Block(bl) | Value::Loop(bl) => bl
            .operators
            .iter()
            .all(|o| body_ok(data, o, rb, own, exits)),
        Value::If(t, vals, b) => {
            body_ok(data, t, rb, own, exits)
                && body_ok(data, vals, rb, own, exits)
                && body_ok(data, b, rb, own, exits)
        }
        Value::Null
        | Value::Line(_)
        | Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Enum(_, _)
        | Value::Text(_)
        | Value::Break(_)
        | Value::Continue(_) => true,
        _ => false,
    }
}

/// A `return` value the twin answers as OWNED: a store the function minted, or a block
/// ending in one.
fn owned_value(v: &Value, own: &[u16]) -> bool {
    match v.unspan() {
        Value::Var(x) => own.contains(x),
        Value::Block(bl) => bl
            .operators
            .iter()
            .rev()
            .find(|o| !matches!(o, Value::Line(_)))
            .is_some_and(|o| matches!(o.unspan(), Value::Var(x) if own.contains(x))),
        _ => false,
    }
}

/// An admitted call site: the buffer prep at `ops[at]`, the call binding `subject` at
/// `ops[call]`.
pub struct Site {
    pub callee: u32,
    pub buf: u16,
    pub subject: u16,
    /// The call's argument list, which identifies it when it is emitted.
    pub args_at: usize,
}

/// The buffer prep `if OpRefIsNull(B) { OpDatabase(B, tp) } else OpClear(B, tp)`.
fn prep_buffer(data: &Data, stmt: &Value) -> Option<u16> {
    let Value::If(cond, then, els) = stmt.unspan() else {
        return None;
    };
    let Value::Call(probe, probe_args) = cond.unspan() else {
        return None;
    };
    let Some(Value::Var(buf)) = (name(data, *probe) == "OpRefIsNull")
        .then(|| probe_args.first().map(Value::unspan))
        .flatten()
    else {
        return None;
    };
    let ops: &[Value] = match then.unspan() {
        Value::Block(bl) => &bl.operators,
        Value::Insert(ops) => ops,
        other => std::slice::from_ref(other),
    };
    let ops: Vec<&Value> = ops
        .iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let [mint] = &ops[..] else {
        return None;
    };
    let is = |stmt: &Value, op: &str| matches!(stmt.unspan(), Value::Call(c, a) if name(data, *c) == op && a.first().is_some_and(|x| is_var(x, *buf)));
    (is(mint, "OpDatabase") && is(els, "OpClear")).then_some(*buf)
}

/// The statement `ops[at]` of function `d_nr`, when it is the buffer prep of an admitted
/// view-return call.  `admits` answers (and memoises) the callee question.
pub fn site(
    ops: &[Value],
    at: usize,
    data: &Data,
    d_nr: u32,
    admits: &mut dyn FnMut(u32) -> bool,
) -> Option<Site> {
    let buf = prep_buffer(data, &ops[at])?;
    let (call_at, call) = ops
        .iter()
        .enumerate()
        .skip(at + 1)
        .find(|(_, o)| !matches!(o, Value::Line(_)))?;
    let Value::Set(subject, init) = call.unspan() else {
        return None;
    };
    let subject = *subject;
    let Value::Call(f, args) = init.unspan() else {
        return None;
    };
    let decline = |why: &str| {
        if trace() {
            eprintln!(
                "[view-return] {} -> {}: declined, {why}",
                data.def(d_nr).name(),
                name(data, *f)
            );
        }
        None
    };
    if !args.last().is_some_and(|x| is_var(x, buf)) {
        return None;
    }
    if (*f as usize) >= data.definitions.len() || *data.def(*f).code() == Value::Null {
        return None;
    }
    if !admits(*f) {
        return decline("the callee writes or returns otherwise");
    }
    // The arguments name stores the caller holds for the whole window: variables and reads.
    if !args[..args.len() - 1]
        .iter()
        .all(|a| mentions(a, buf) == 0 && mentions(a, subject) == 0 && pure_read(data, a))
    {
        return decline("an argument is computed");
    }
    let code = data.def(d_nr).code();
    // The buffer: its prep, the call, and the frees — `OpFreeRef(B)`, `OpFreeRefIfDistinct(S, B)`.
    let mut allowed = 0usize;
    let mut foreign = false;
    code.walk(&mut |n| {
        if let Value::Call(c, a) = n {
            match name(data, *c) {
                "OpFreeRef" if a.first().is_some_and(|x| is_var(x, buf)) => allowed += 1,
                "OpFreeRefIfDistinct" if a.get(1).is_some_and(|x| is_var(x, buf)) => {
                    if a.first().is_some_and(|x| is_var(x, subject)) {
                        allowed += 1;
                    } else {
                        foreign = true;
                    }
                }
                _ => {}
            }
        }
    });
    if foreign || mentions(code, buf) != mentions(&ops[at], buf) + 1 + allowed {
        return decline("the buffer is named elsewhere");
    }
    // The subject: bound once, read only in the window, only through copying getters.
    let window = &ops[call_at + 1..];
    let in_window: usize = window.iter().map(|o| mentions(o, subject)).sum();
    if mentions(code, subject) != in_window + mentions(init, subject) {
        return decline("the result is read outside its block");
    }
    let mut sets = 0usize;
    code.walk(&mut |n| {
        if let Value::Set(x, v) = n
            && *x == subject
            && !matches!(v.unspan(), Value::Null)
        {
            sets += 1;
        }
    });
    if sets != 1 {
        return decline("the result's variable is bound twice");
    }
    for o in window {
        if !window_ok(data, o, subject, buf) {
            return decline("the window writes, calls or holds the result");
        }
    }
    if trace() {
        eprintln!(
            "[view-return] {} -> {}: admitted",
            data.def(d_nr).name(),
            name(data, *f)
        );
    }
    Some(Site {
        callee: *f,
        buf,
        subject,
        args_at: args.as_ptr() as usize,
    })
}

/// A statement of the window: no store write, no user call, the subject read only by a
/// copying getter (or freed against the buffer), the buffer only freed.
fn window_ok(data: &Data, val: &Value, subj: u16, buf: u16) -> bool {
    match val.unspan() {
        Value::Var(lhs) => *lhs != subj && *lhs != buf,
        Value::Call(callee, vals) => {
            let op = name(data, *callee);
            if op == "OpFreeRefIfDistinct"
                && vals.len() == 2
                && is_var(&vals[0], subj)
                && is_var(&vals[1], buf)
            {
                return true;
            }
            if op == "OpFreeRef" && vals.first().is_some_and(|lhs| is_var(lhs, buf)) {
                return true;
            }
            if copying_getter(op) && vals.first().is_some_and(|lhs| is_var(lhs, subj)) {
                return vals[1..].iter().all(|lhs| window_ok(data, lhs, subj, buf));
            }
            let local_text = matches!(
                op,
                "OpFormatStackText"
                    | "OpFormatStackInt"
                    | "OpFormatStackFloat"
                    | "OpFormatStackSingle"
                    | "OpAppendStackText"
                    | "OpAppendStackCharacter"
                    | "OpClearStackText"
            );
            (read_op(op) || local_text) && vals.iter().all(|lhs| window_ok(data, lhs, subj, buf))
        }
        Value::Return(r) => window_ok(data, r, subj, buf),
        Value::Set(lhs, init) => *lhs != subj && *lhs != buf && window_ok(data, init, subj, buf),
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter().all(|o| window_ok(data, o, subj, buf))
        }
        Value::If(tst, lhs, rhs) => {
            window_ok(data, tst, subj, buf)
                && window_ok(data, lhs, subj, buf)
                && window_ok(data, rhs, subj, buf)
        }
        Value::Null
        | Value::Line(_)
        | Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Enum(_, _)
        | Value::Text(_)
        | Value::Break(_)
        | Value::Continue(_) => true,
        _ => false,
    }
}
