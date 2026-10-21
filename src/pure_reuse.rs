// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-PureReuse` — a call of an EFFECT-FREE function, made twice in one body on arguments
//! nothing writes in between, is computed once.
//!
//! Runs at the start of the scope pass, on the parser's bodies, so the local it binds is
//! freed like any other local the author wrote.  Three parts:
//!
//! - [`effect_free`]: the functions whose only effect is their result — a fixed point over
//!   the call graph, each body built from read and arithmetic operators, writes whose target
//!   is a store the function itself made (never a parameter's), and calls of effect-free
//!   functions.  Anything else declines, so an operator this list does not know costs the
//!   reuse, never a dropped effect.
//! - [`projection_of`]: a WRAPPER whose body is one call of such a function followed by a
//!   read of the result (`d = decode(b); return d.ok`), or by a field handed back
//!   (`d = decode(b); v = d.value; return v`) — its call is that read of the inner call.
//!   The natural spelling of the same wrapper, the read written straight on the call
//!   (`decode(b).ok`, `decode(b).value`, with or without `return`), is the same projection.
//! - [`reuse_in_block`]: two such calls on the same argument variables, in statements that
//!   always evaluate them, with nothing in between that writes a store or rebinds an
//!   argument: the first statement is preceded by one call into a fresh local, and both
//!   uses read that local.
//!
//! `LOFT_NO_PURE_REUSE=1` keeps every call; `LOFT_TRACE_PURE_REUSE=1` names each reuse.

use crate::data::{Data, DefType, Value};
use crate::fxhash::FxHashSet as HashSet;

/// Operators that only read their operands and answer a value.
pub(crate) fn reads_only(name: &str) -> bool {
    const PREFIXES: [&str; 4] = ["OpGet", "OpLength", "OpConv", "OpCast"];
    const ARITH: [&str; 18] = [
        "OpAdd",
        "OpMin",
        "OpMul",
        "OpDiv",
        "OpRem",
        "OpLt",
        "OpLe",
        "OpGt",
        "OpGe",
        "OpAnd",
        "OpOr",
        "OpXor",
        "OpNeg",
        "OpAbs",
        "OpShl",
        "OpShr",
        "OpLogical",
        "OpMathFunc",
    ];
    PREFIXES.iter().any(|p| name.starts_with(p))
        || crate::shared_call::comparison(name)
        || ARITH.iter().any(|p| name.starts_with(p))
        || matches!(
            name,
            "OpNot" | "OpRefIsNull" | "OpRefAlias" | "OpCreateStack" | "OpGetVectorNullable"
        )
        // Stdlib functions known to answer a value and do nothing else.  The `#impure`
        // annotations are not a complete list (`print`, `file`, `env_variable` carry none),
        // so a native is admitted by name only.
        || matches!(
            name,
            "t_6vector_len" | "t_4text_len" | "n_text_from_bytes" | "n_text_from_byte_range"
        )
}

/// Operators that write into the store one argument names: that argument's index.
pub(crate) fn write_target(name: &str) -> Option<usize> {
    match name {
        "OpCopyRecord" => Some(1),
        n if n.starts_with("OpSet") => Some(0),
        "OpDatabase" | "OpPreAllocVector" | "OpAppendVector" | "OpAppendCopy" | "OpNewRecord"
        | "OpFinishRecord" | "OpSliceVector" => Some(0),
        n if n.starts_with("OpPush") => Some(0),
        _ => None,
    }
}

/// Operators that write a text or a store through one operand, beyond [`write_target`]'s list.
pub(crate) fn text_write_target(name: &str) -> Option<usize> {
    match name {
        "OpAppendText"
        | "OpAppendStackText"
        | "OpAppendCharacter"
        | "OpAppendStackCharacter"
        | "OpAppendTextBytes"
        | "OpClearText"
        | "OpClearStackText"
        | "OpClearVector"
        | "OpSetTextReplace"
        | "OpFormatText"
        | "OpFormatStackText"
        | "OpFormatInt"
        | "OpFormatStackInt"
        | "OpFormatFloat"
        | "OpFormatStackFloat"
        | "OpFormatSingle"
        | "OpFormatStackSingle"
        | "OpFormatDatabase"
        | "OpFormatStackDatabase"
        | "OpPutRef"
        | "OpPutText"
        | "OpPutInt"
        | "OpPutFloat"
        | "OpPutSingle"
        | "OpPutBool"
        | "OpPutCharacter"
        | "OpPutEnum"
        | "OpPutNarrow" => Some(0),
        _ => write_target(name),
    }
}

/// Every operand an operator writes or consumes: [`write_target`]'s, a text's or a buffer's
/// (the stream walk's list), and both sides of an adoption — the source gives up its storage.
fn own_writes(name: &str) -> Option<[Option<usize>; 2]> {
    match name {
        "OpAdoptVector" => Some([Some(0), Some(1)]),
        n => text_write_target(n).map(|t| [Some(t), None]),
    }
}

/// The variable a store place is rooted at: a variable, a field or element of one, or the
/// last value of a block.  `None` for anything else — the caller declines.
fn root(v: &Value, data: &Data) -> Option<u16> {
    match v.unspan() {
        Value::Var(x) => Some(*x),
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.def(*d).name().starts_with("OpGet")
                && !args.is_empty() =>
        {
            root(&args[0], data)
        }
        Value::Block(bl) => bl.operators.last().and_then(|l| root(l, data)),
        _ => None,
    }
}

/// The user functions this pass may reason about: loft bodies, not natives.
fn candidates(data: &Data) -> Vec<u32> {
    (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type, DefType::Function)
                && def.name().starts_with("n_")
                && !matches!(def.code(), Value::Null)
        })
        .collect()
}

/// The variables of `f` that are its parameters, its own hidden return buffer excluded.
fn params(data: &Data, f: u32) -> HashSet<u16> {
    let def = data.def(f);
    let retbuf = def.hidden_return_buffer_attr();
    def.attributes()
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != retbuf)
        .map(|(_, a)| def.variables().var(&a.name))
        .filter(|&v| v != u16::MAX)
        .collect()
}

/// Is every operation of `f`'s body one whose only effect is on stores `f` made itself?
fn body_ok(data: &Data, f: u32, free: &HashSet<u32>, cands: &HashSet<u32>) -> bool {
    let ps = params(data, f);
    let mut ok = true;
    data.def(f).code().any_node(&mut |n| {
        match n {
            Value::Call(d, args) => {
                if (*d as usize) >= data.definitions.len() {
                    ok = false;
                    return true;
                }
                let callee = data.def(*d);
                let name = callee.name();
                if cands.contains(d) {
                    if !free.contains(d) {
                        ok = false;
                        return true;
                    }
                    // A callee writes into the buffer it is handed: never a parameter's store.
                    if let Some(k) = callee.hidden_return_buffer_attr()
                        && let Some(a) = args.get(k)
                        && root(a, data).is_none_or(|r| ps.contains(&r))
                    {
                        ok = false;
                        return true;
                    }
                } else if reads_only(name) || crate::shared_call::pure_native(data, *d) {
                } else if let Some(ts) = own_writes(name) {
                    if ts.iter().flatten().any(|&t| {
                        args.get(t)
                            .and_then(|a| root(a, data))
                            .is_none_or(|r| ps.contains(&r))
                    }) {
                        ok = false;
                        return true;
                    }
                } else {
                    ok = false;
                    return true;
                }
            }
            Value::CallRef(..) | Value::Yield(_) => {
                ok = false;
                return true;
            }
            _ => {}
        }
        false
    });
    ok
}

/// The user functions whose only effect is their result (the greatest fixed point: assume
/// every candidate, drop each whose body breaks the rule, until none does).
pub fn effect_free(data: &Data) -> HashSet<u32> {
    let cands: Vec<u32> = candidates(data);
    let all: HashSet<u32> = cands.iter().copied().collect();
    let mut free = all.clone();
    loop {
        let drop: Vec<u32> = cands
            .iter()
            .copied()
            .filter(|f| free.contains(f) && !body_ok(data, *f, &free, &all))
            .collect();
        if drop.is_empty() {
            return free;
        }
        for f in drop {
            free.remove(&f);
        }
    }
}

/// What a wrapper's call is: a read of one call of `inner`.
#[derive(Clone)]
pub struct Projection {
    /// The effect-free function called.
    pub inner: u32,
    /// For each argument of `inner` before its return buffer: the wrapper parameter it is.
    pub args: Vec<usize>,
    /// The read applied to the inner result: the operator and its constant operands, or a
    /// field handed back (`OpGetField` with the field's offset and type).
    pub read: u32,
    pub consts: Vec<Value>,
}

/// The statements of a body that are not `Line` markers or a buffer's null init.
fn significant(ops: &[Value]) -> Vec<&Value> {
    ops.iter()
        .filter(|o| match o.unspan() {
            Value::Line(_) => false,
            Value::Set(_, v) => !matches!(v.unspan(), Value::Null),
            _ => true,
        })
        .collect()
}

/// `inner(args…)` as a call of an effect-free function on the parameters of wrapper `w`: the
/// function and, per argument before its return buffer, the parameter position it is.
fn inner_call(data: &Data, w: u32, free: &HashSet<u32>, call: &Value) -> Option<(u32, Vec<usize>)> {
    let Value::Call(inner, iargs) = call.unspan() else {
        return None;
    };
    if !free.contains(inner) {
        return None;
    }
    let def = data.def(w);
    let iret = data.def(*inner).hidden_return_buffer_attr();
    let wattrs = def.attributes();
    let mut args = Vec::new();
    for (k, a) in iargs.iter().enumerate() {
        if Some(k) == iret {
            continue;
        }
        let Value::Var(x) = a.unspan() else {
            return None;
        };
        let pos = wattrs
            .iter()
            .position(|at| def.variables().var(&at.name) == *x)?;
        args.push(pos);
    }
    Some((*inner, args))
}

/// `OpGet*(<base>, <constants>…)` whose base `is_base` accepts: the read and its constants.
fn read_of(data: &Data, v: &Value, is_base: &dyn Fn(&Value) -> bool) -> Option<(u32, Vec<Value>)> {
    let Value::Call(r, rargs) = v.unspan() else {
        return None;
    };
    if !data.def(*r).name().starts_with("OpGet") {
        return None;
    }
    let (first, rest) = rargs.split_first()?;
    if !is_base(first)
        || !rest
            .iter()
            .all(|c| matches!(c.unspan(), Value::Int(_) | Value::Long(_)))
    {
        return None;
    }
    Some((*r, rest.to_vec()))
}

/// The natural spelling of a projection wrapper: its body is ONE expression, the read written
/// on the call itself — `inner(x).k` (a scalar field), or the materialised copy of
/// `inner(x).k` into the result (a record field) — returned or as the block's value.
fn natural_projection(
    data: &Data,
    w: u32,
    free: &HashSet<u32>,
    only: &Value,
) -> Option<Projection> {
    let e = match only.unspan() {
        Value::Return(r) => r.unspan(),
        v => v,
    };
    // A record-field projection whose root is the call binds the call to a local first
    // (loft#2006, `return_source`: `_ret_root = inner(x); <the copy of _ret_root.k, tested>`),
    // so the read is rooted at that local and the call is the block's first statement.  The
    // wrapper is the same projection, read through its local.
    if let Value::Block(b) = e
        && b.name == "return_source"
    {
        let body = significant(&b.operators);
        // The bind is the first statement that binds a call of an effect-free function; the
        // preparation of that call's own buffer (`if OpRefIsNull(buf) … else OpClear(…)`)
        // may stand before it.
        let (at, d, inner, args) = body.iter().enumerate().find_map(|(i, op)| {
            let Value::Set(d, call) = op.unspan() else {
                return None;
            };
            let (inner, args) = inner_call(data, w, free, call)?;
            Some((i, d, inner, args))
        })?;
        let on_d = |b: &Value| matches!(b.unspan(), Value::Var(x) if x == d);
        let mut copied = None;
        for op in &body[at + 1..] {
            op.walk(&mut |v| {
                if let Value::Block(m) = v.unspan()
                    && m.name == "materialized_view_return"
                    && copied.is_none()
                {
                    copied = m.operators.iter().find_map(|o| match o.unspan() {
                        Value::Call(c, cargs) if data.def(*c).name() == "OpCopyRecord" => {
                            cargs.first()
                        }
                        _ => None,
                    });
                }
            });
        }
        let (read, consts) = read_of(data, copied?, &on_d)?;
        if data.def(read).name() != "OpGetField" {
            return None;
        }
        return Some(Projection {
            inner,
            args,
            read,
            consts,
        });
    }
    let read = match e {
        Value::Block(b) if b.name == "materialized_view_return" => {
            b.operators.iter().find_map(|o| match o.unspan() {
                Value::Call(c, cargs) if data.def(*c).name() == "OpCopyRecord" => cargs.first(),
                _ => None,
            })?
        }
        v => v,
    };
    let Value::Call(_, rargs) = read.unspan() else {
        return None;
    };
    let (inner, args) = inner_call(data, w, free, rargs.first()?)?;
    let (read, consts) = read_of(data, read, &|_| true)?;
    if matches!(e, Value::Block(_)) && data.def(read).name() != "OpGetField" {
        return None;
    }
    Some(Projection {
        inner,
        args,
        read,
        consts,
    })
}

/// The [`Projection`] a wrapper's body spells, if it is one.
pub fn projection_of(data: &Data, w: u32, free: &HashSet<u32>) -> Option<Projection> {
    let def = data.def(w);
    if def.hidden_return_buffer_attr().is_some() && !returns_field_copy(def.code()) {
        return None;
    }
    let Value::Block(bl) = def.code().unspan() else {
        return None;
    };
    let body = significant(&bl.operators);
    if let [only] = body.as_slice() {
        return natural_projection(data, w, free, only);
    }
    // The bound spelling of the natural projection (loft#2006, `return_source`) may stand
    // beside the witness bind of the wrapper's own buffer; it is still the whole body.
    if let Some(rs) = body
        .iter()
        .find(|o| matches!(o.unspan(), Value::Block(b) if b.name == "return_source"))
    {
        return natural_projection(data, w, free, rs);
    }
    let Value::Set(d, call) = body.first()?.unspan() else {
        return None;
    };
    let (inner, args) = inner_call(data, w, free, call)?;
    let on_d = |b: &Value| matches!(b.unspan(), Value::Var(x) if x == d);
    match body.get(1..)? {
        // `return d.k`
        [ret] => {
            let Value::Return(inner_v) = ret.unspan() else {
                return None;
            };
            let (read, consts) = read_of(data, inner_v, &on_d)?;
            Some(Projection {
                inner,
                args,
                read,
                consts,
            })
        }
        // `v = d.k; return <copy of v>` — the field handed back.
        [set, ret] => {
            let Value::Set(v, rv) = set.unspan() else {
                return None;
            };
            let (read, consts) = read_of(data, rv, &on_d)?;
            if data.def(read).name() != "OpGetField" {
                return None;
            }
            let Value::Return(r) = ret.unspan() else {
                return None;
            };
            if !copies_var(r, *v, data) {
                return None;
            }
            Some(Projection {
                inner,
                args,
                read,
                consts,
            })
        }
        _ => None,
    }
}

/// Does a function body end in the parser's materialised view return (`return {__retbuf =
/// null; OpDatabase(__retbuf, tp); OpCopyRecord(v, __retbuf, tp); __retbuf}`), returned or
/// as the body's value?
fn returns_field_copy(code: &Value) -> bool {
    let Value::Block(bl) = code.unspan() else {
        return false;
    };
    let last = significant(&bl.operators).last().map(|v| v.unspan());
    let tail = match last {
        Some(Value::Return(r)) => r.unspan(),
        Some(v) => v,
        None => return false,
    };
    // The bound spelling (loft#2006, `return_source`) holds the same materialised copy,
    // tested, after the bind of its root.
    let mut holds_copy = false;
    tail.walk(&mut |v| {
        if let Value::Block(b) = v.unspan()
            && b.name == "materialized_view_return"
        {
            holds_copy = true;
        }
    });
    matches!(tail, Value::Block(b) if b.name == "materialized_view_return")
        || (matches!(tail, Value::Block(b) if b.name == "return_source") && holds_copy)
}

/// Is `r` the materialised copy of variable `v` into the return buffer?
fn copies_var(r: &Value, v: u16, data: &Data) -> bool {
    let Value::Block(b) = r.unspan() else {
        return false;
    };
    if b.name != "materialized_view_return" {
        return false;
    }
    b.operators.iter().any(|o| {
        matches!(o.unspan(), Value::Call(d, args)
            if data.def(*d).name() == "OpCopyRecord"
                && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == v))
    })
}

/// One call this pass can reuse: the effect-free function and its argument variables.
#[derive(Clone, PartialEq)]
struct Key {
    inner: u32,
    args: Vec<u16>,
}

/// How a reusable call reads the one value: a projection wrapper's read, a sharing
/// wrapper's variant, or the call of the function itself.
#[derive(Clone)]
enum Site {
    Read(u32, Vec<Value>),
    Variant(u32),
    Bare,
}

/// What the pass knows about the program's functions.
struct Rules<'a> {
    free: &'a HashSet<u32>,
    total: &'a HashSet<u32>,
    wrappers: &'a crate::fxhash::FxHashMap<u32, Projection>,
    sharers: &'a crate::fxhash::FxHashMap<u32, crate::shared_call::Sharing>,
}

/// The reusable call `v` is and how its value is read.  A call of a TOTAL function is one
/// itself only when its result is a vector handed back in a buffer local.
fn site_of(v: &Value, r: &Rules, data: &Data) -> Option<(Key, Site)> {
    let Value::Call(w, wargs) = v.unspan() else {
        return None;
    };
    let vars_at = |positions: &mut dyn Iterator<Item = usize>| -> Option<Vec<u16>> {
        positions
            .map(|p| match wargs.get(p)?.unspan() {
                Value::Var(x) => Some(*x),
                _ => None,
            })
            .collect()
    };
    if let Some(p) = r.wrappers.get(w) {
        let args = vars_at(&mut p.args.iter().copied())?;
        return Some((
            Key {
                inner: p.inner,
                args,
            },
            Site::Read(p.read, p.consts.clone()),
        ));
    }
    if let Some(s) = r.sharers.get(w) {
        let args = vars_at(&mut s.args.iter().copied())?;
        return Some((Key { inner: s.f, args }, Site::Variant(*w)));
    }
    if r.total.contains(w)
        && let Some(kbuf) = crate::shared_call::vector_buffer(data, *w)
        && matches!(wargs.get(kbuf).map(Value::unspan), Some(Value::Var(_)))
    {
        let args = vars_at(&mut (0..wargs.len()).filter(|&i| i != kbuf))?;
        return Some((Key { inner: *w, args }, Site::Bare));
    }
    None
}

/// The positions of a statement that are evaluated whenever the statement runs: the
/// statement, an `if`'s test, an assignment's value, a call's arguments — never an `if`'s
/// arms, a loop's body, or a block's later statements.
fn always_evaluated<'a>(v: &'a Value, out: &mut Vec<&'a Value>) {
    out.push(v);
    match v.unspan() {
        Value::If(test, _, _) => always_evaluated(test, out),
        Value::Set(_, val) => always_evaluated(val, out),
        Value::Call(_, args) => {
            for a in args {
                always_evaluated(a, out);
            }
        }
        Value::Return(r) => always_evaluated(r, out),
        _ => {}
    }
}

/// Does statement `s` hold a call of `key`, and is one of them always evaluated by it?
fn sites_in(s: &Value, key: &Key, r: &Rules, data: &Data) -> (bool, bool) {
    let mut always = Vec::new();
    always_evaluated(s, &mut always);
    let (mut has, mut alw) = (false, false);
    s.walk(&mut |n| {
        if site_of(n, r, data).is_some_and(|(k, _)| &k == key) {
            has = true;
            alw |= always.iter().any(|p| std::ptr::eq(p.unspan(), n.unspan()));
        }
    });
    (has, alw)
}

/// Does an `if` arm leave the function — its last statement a `return`?  Nothing it does
/// reaches a statement after the `if`.
fn leaves(arm: &Value) -> bool {
    match arm.unspan() {
        Value::Return(_) => true,
        Value::Block(bl) => bl
            .operators
            .iter()
            .rev()
            .find(|o| !matches!(o.unspan(), Value::Line(_)))
            .is_some_and(leaves),
        _ => false,
    }
}

/// The context [`disturbs`] answers in: the caller's FRESH locals (rooted at a store the
/// caller made after its arguments were bound) and whether the reused call's arguments are
/// out of their reach (every one a parameter or a scalar).
struct Reach<'a> {
    fresh: &'a HashSet<u16>,
    apart: bool,
}

/// Does `v` write a store the call's arguments may live in, call a function this pass
/// cannot see the effects of, or assign one of the arguments?  With `beside`, a call of
/// `key` itself is not looked into.  A write rooted at a fresh local is none of these when
/// the arguments are apart; an arm that leaves the function reaches no later statement.
///
/// With `after`, what statement `v` does once a call of `key` inside it has been evaluated is
/// not looked at either: a call or an assignment that takes the call's value acts after it.
/// That answers whether the call there still reads what the first call read; whether a LATER
/// call does is the question without `after`.
fn disturbs(
    v: &Value,
    key: &Key,
    data: &Data,
    r: &Rules,
    reach: &Reach,
    beside: bool,
    after: bool,
) -> bool {
    if beside && site_of(v, r, data).is_some_and(|(k, _)| &k == key) {
        return false;
    }
    let deeper = |c: &Value| disturbs(c, key, data, r, reach, beside, after);
    let takes_site = |c: &Value| after && contains_site(c, key, data, r);
    match v.unspan() {
        Value::Set(x, val) => (key.args.contains(x) && !takes_site(val)) || deeper(val),
        Value::Call(d, args) => {
            let name = data.def(*d).name();
            let this = if args.iter().any(takes_site)
                || r.free.contains(d)
                || data.def(*d).synthetic() == Some(crate::shared_call::VARIANT)
                || reads_only(name)
                || crate::shared_call::pure_native(data, *d)
            {
                false
            } else if let Some(ts) = own_writes(name) {
                !(reach.apart
                    && ts.iter().flatten().all(|&t| {
                        args.get(t)
                            .and_then(|a| root(a, data))
                            .is_some_and(|x| reach.fresh.contains(&x))
                    }))
            } else {
                true
            };
            this || args.iter().any(deeper)
        }
        Value::If(test, then, els) => {
            deeper(test) || (!leaves(then) && deeper(then)) || (!leaves(els) && deeper(els))
        }
        Value::CallRef(..) | Value::Yield(_) => true,
        _ => {
            let mut hit = false;
            v.for_each_child(&mut |c| hit = hit || deeper(c));
            hit
        }
    }
}

/// Does `v` hold a call of `key`?
fn contains_site(v: &Value, key: &Key, data: &Data, r: &Rules) -> bool {
    v.any_node(&mut |n| site_of(n, r, data).is_some_and(|(k, _)| &k == key))
}

/// The locals of a body that only ever hold a store the body made: assigned `null`, a
/// literal, or a field, element or new record of another such local (the greatest fixed
/// point: a local leaves the set once one assignment gives it anything else).
fn fresh_locals(ops: &[Value], f: u32, data: &Data) -> HashSet<u16> {
    let vars = &data.def(f).variables;
    let mut sets: Vec<(u16, &Value)> = Vec::new();
    for op in ops {
        op.walk(&mut |n| {
            if let Value::Set(x, v) = n {
                sets.push((*x, v));
            }
        });
    }
    let mut fresh: HashSet<u16> = sets
        .iter()
        .map(|(x, _)| *x)
        .filter(|x| !vars.is_argument(*x))
        .collect();
    loop {
        let out: Vec<u16> = sets
            .iter()
            .filter(|(x, v)| fresh.contains(x) && !fresh_value(v, &fresh, data))
            .map(|(x, _)| *x)
            .collect();
        if out.is_empty() {
            return fresh;
        }
        for x in out {
            fresh.remove(&x);
        }
    }
}

fn fresh_value(v: &Value, fresh: &HashSet<u16>, data: &Data) -> bool {
    match v.unspan() {
        Value::Null
        | Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Enum(..)
        | Value::Text(_) => true,
        Value::Var(y) => fresh.contains(y),
        Value::Call(d, args) => {
            let name = data.def(*d).name();
            (name.starts_with("OpGet") || name == "OpNewRecord")
                && args
                    .first()
                    .and_then(|a| root(a, data))
                    .is_some_and(|x| fresh.contains(&x))
        }
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|l| fresh_value(l, fresh, data)),
        _ => false,
    }
}

/// Can no store write reach the call's arguments except through a parameter's store: is
/// every argument a parameter of `f`, or a scalar?
fn apart(key: &Key, f: u32, data: &Data) -> bool {
    let vars = &data.def(f).variables;
    key.args.iter().all(|&a| {
        vars.is_argument(a)
            || matches!(
                vars.tp(a).base(),
                crate::data::Type::Integer(_)
                    | crate::data::Type::Float
                    | crate::data::Type::Single
                    | crate::data::Type::Boolean
                    | crate::data::Type::Character
            )
    })
}

/// Reuse within one block's statements; answers whether it rewrote anything.
///
/// A key is taken from statement `i` and its later calls are collected up to the first
/// statement that disturbs the arguments.  An effect-free call is reused when its first and
/// later calls are each where their statement always evaluates them; a TOTAL one wherever
/// they stand, as long as one of them is always evaluated, so the call computed up front is
/// one a path would have made anyway.  `done` holds the keys already computed up front.
#[allow(clippy::too_many_arguments)]
fn reuse_in_block(
    ops: &mut Vec<Value>,
    f: u32,
    data: &mut Data,
    db: &mut crate::database::Stores,
    r: &Rules,
    variants: &mut crate::fxhash::FxHashMap<u32, (u32, usize)>,
    done: &mut Vec<Key>,
    trace: bool,
) -> bool {
    // Asked only once a statement holds a reusable call: most bodies hold none.
    let mut fresh: Option<HashSet<u16>> = None;
    for i in 0..ops.len() {
        let mut keys: Vec<Key> = Vec::new();
        ops[i].walk(&mut |n| {
            if let Some((k, _)) = site_of(n, r, data)
                && !done.contains(&k)
                && !keys.contains(&k)
            {
                keys.push(k);
            }
        });
        for key in keys {
            let total = r.total.contains(&key.inner);
            let (_, always_i) = sites_in(&ops[i], &key, r, data);
            if !total && !always_i {
                continue;
            }
            let fresh = fresh.get_or_insert_with(|| fresh_locals(ops, f, data));
            let reach = Reach {
                fresh,
                apart: apart(&key, f, data),
            };
            let mut later = Vec::new();
            let mut any_always = always_i;
            for (j, stmt) in ops.iter().enumerate().skip(i + 1) {
                let (has, alw) = sites_in(stmt, &key, r, data);
                if !has {
                    if disturbs(stmt, &key, data, r, &reach, false, false) {
                        break;
                    }
                    continue;
                }
                if !total && !alw {
                    break;
                }
                if disturbs(stmt, &key, data, r, &reach, true, false) {
                    // Disturbed only once its call has been evaluated: the last one reused.
                    if !disturbs(stmt, &key, data, r, &reach, true, true) {
                        later.push(j);
                        any_always |= alw;
                    }
                    break;
                }
                later.push(j);
                any_always |= alw;
            }
            if later.is_empty()
                || !any_always
                || disturbs(&ops[i], &key, data, r, &reach, true, false)
            {
                continue;
            }
            // A sharing wrapper's call needs the wrapper's variant.
            let mut ok = true;
            let mut need = Vec::new();
            for &at in std::iter::once(&i).chain(&later) {
                ops[at].walk(&mut |n| {
                    if let Some((k, Site::Variant(w))) = site_of(n, r, data)
                        && k == key
                        && !variants.contains_key(&w)
                        && !need.contains(&w)
                    {
                        need.push(w);
                    }
                });
            }
            for w in need {
                match crate::shared_call::variant(data, db, w, &r.sharers[&w]) {
                    Some(v) => {
                        variants.insert(w, v);
                    }
                    None => ok = false,
                }
            }
            if !ok {
                continue;
            }
            let shared = rewrite(ops, i, &later, &key, f, data, db, r, variants);
            if trace {
                crate::loft_eprintln!(
                    "[pure-reuse] {}: {} computed once{}",
                    data.def(f).name(),
                    data.def(key.inner).name(),
                    if shared { ", shared" } else { "" }
                );
            }
            done.push(key);
            return true;
        }
    }
    false
}

/// Bind the call once before statement `i` and make it and every statement of `later` read
/// it.  Answers whether a call other than a projection's read was replaced (the shared-call
/// clause).
#[allow(clippy::too_many_arguments)]
fn rewrite(
    ops: &mut Vec<Value>,
    i: usize,
    later: &[usize],
    key: &Key,
    f: u32,
    data: &mut Data,
    db: &mut crate::database::Stores,
    r: &Rules,
    variants: &crate::fxhash::FxHashMap<u32, (u32, usize)>,
) -> bool {
    let inner = data.def(key.inner);
    let ret_tp = inner.returned.without_deps();
    let kbuf = inner.hidden_return_buffer_attr();
    let buf_tp = kbuf.map(|k| inner.attributes()[k].typedef.clone());
    let vector = crate::shared_call::vector_buffer(data, key.inner).is_some();
    let vars = &mut data.definitions[f as usize].variables;
    let mut args: Vec<Value> = key.args.iter().map(|a| Value::Var(*a)).collect();
    let mut prelude = Vec::new();
    let mut prologue = None;
    // The local is numbered before its buffer, as the parser binds `v = f(…)`.
    let local = vars.add_unique("reuse", &ret_tp, u16::MAX);
    if let (Some(k), Some(btp)) = (kbuf, &buf_tp) {
        let buf = vars.add_unique("reuse_buf", btp, u16::MAX);
        if vector {
            // Declared in the body's prologue beside the parser's own buffers.
            prologue = Some(Value::Set(buf, Box::new(Value::Null)));
        } else {
            prelude.push(Value::Set(buf, Box::new(Value::Null)));
        }
        args.insert(k.min(args.len()), Value::Var(buf));
    }
    prelude.push(Value::Set(local, Box::new(Value::Call(key.inner, args))));
    let mut shared = false;
    for &at in std::iter::once(&i).chain(later) {
        shared |= replace_calls(&mut ops[at], key, local, data, r, variants);
    }
    if vector && let Some(k) = kbuf {
        let given_tp = ret_tp.clone();
        if let crate::data::Type::Vector(content, _) = given_tp.base()
            && let Some(tp) = data.vector_element_type(content, db)
        {
            let counts = crate::shared_call::buffer_uses(ops.iter());
            let hit = std::cell::Cell::new(false);
            for &at in std::iter::once(&i).chain(later) {
                crate::shared_call::hand_in(
                    &mut ops[at],
                    data,
                    &|c: &Value| {
                        let is = site_of(c, r, data)
                            .is_some_and(|(kk, s)| &kk == key && matches!(s, Site::Bare));
                        hit.set(hit.get() || is);
                        is
                    },
                    &Value::Var(local),
                    k,
                    tp,
                    &|b| counts.throwaway(b),
                    &given_tp,
                );
            }
            shared |= hit.get();
        }
    }
    crate::rewrite_census::fired(
        if shared {
            "R-PureReuse/shared"
        } else {
            "R-PureReuse"
        },
        1,
    );
    for (offset, stmt) in prelude.into_iter().enumerate() {
        ops.insert(i + offset, stmt);
    }
    if let Some(init) = prologue {
        ops.insert(0, init);
    }
    shared
}

/// Replace each projection read and sharing-wrapper call of `key` in `v` by its read of
/// `local`: the read, or the variant handed `local`.  Answers whether a variant was called.
fn replace_calls(
    v: &mut Value,
    key: &Key,
    local: u16,
    data: &Data,
    r: &Rules,
    variants: &crate::fxhash::FxHashMap<u32, (u32, usize)>,
) -> bool {
    if let Some((k, site)) = site_of(v, r, data)
        && &k == key
    {
        match site {
            Site::Read(read, consts) => {
                let mut args = vec![Value::Var(local)];
                args.extend(consts);
                *v = Value::Call(read, args);
                return false;
            }
            Site::Variant(w) => {
                let (w2, at) = variants[&w];
                if let Value::Call(_, wargs) = v.unspan() {
                    let mut args = wargs.clone();
                    args.insert(at.min(args.len()), Value::Var(local));
                    *v = Value::Call(w2, args);
                }
                return true;
            }
            Site::Bare => return false,
        }
    }
    let mut shared = false;
    v.for_each_child_mut(&mut |c| shared |= replace_calls(c, key, local, data, r, variants));
    shared
}

/// `LOFT_PURE_REUSE_DUMP=<fn>` — print the named function's body as the scope pass receives it.
fn dump(data: &Data) {
    let Some(name) = std::env::var_os("LOFT_PURE_REUSE_DUMP") else {
        return;
    };
    let name = name.to_string_lossy().into_owned();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.name() != name && def.name() != format!("n_{name}") {
            continue;
        }
        let mut vars = def.variables.clone();
        let mut buf: Vec<u8> = Vec::new();
        let _ = data.show_code(&mut buf, &mut vars, def.code(), 0, true);
        eprintln!(
            "[pure-reuse] {}:\n{}",
            def.name(),
            String::from_utf8_lossy(&buf)
        );
    }
}

/// The pass over the whole program.
pub fn rewrite_program(data: &mut Data, db: &mut crate::database::Stores) {
    if crate::keys::pure_reuse_enabled() {
        let free = effect_free(data);
        let total = crate::shared_call::total(data, &free);
        let mut wrappers = crate::fxhash::FxHashMap::default();
        let mut sharers = crate::fxhash::FxHashMap::default();
        // A variant changes which functions a call enters: not where a definition can change
        // after its calls are compiled, nor where a run reports the functions it entered.
        let variants_allowed = !data.open_world && !data.observes_entries;
        for w in candidates(data) {
            if let Some(p) = projection_of(data, w, &free) {
                wrappers.insert(w, p);
            } else if variants_allowed
                && let Some(s) = crate::shared_call::sharing_of(data, w, &free, &total)
            {
                sharers.insert(w, s);
            }
        }
        // A call of a total function is a site of its own only when it hands back a vector.
        let bare = total
            .iter()
            .any(|&f| crate::shared_call::vector_buffer(data, f).is_some());
        if !wrappers.is_empty() || !sharers.is_empty() || bare {
            let trace = crate::keys::trace_pure_reuse();
            let rules = Rules {
                free: &free,
                total: &total,
                wrappers: &wrappers,
                sharers: &sharers,
            };
            let mut variants = crate::fxhash::FxHashMap::default();
            for f in candidates(data) {
                // Most bodies call nothing this pass reuses: a scan that allocates nothing
                // skips them before anything is set up for them.
                if data.def(f).variables.done
                    || !data.def(f).code().any_node(&mut |n| {
                        matches!(n, Value::Call(d, _)
                            if wrappers.contains_key(d)
                                || sharers.contains_key(d)
                                || (total.contains(d)
                                    && crate::shared_call::vector_buffer(data, *d).is_some()))
                    })
                {
                    continue;
                }
                let _in = crate::rewrite_census::InBody::enter("ir", &data.def(f).name.clone());
                let mut code =
                    std::mem::replace(&mut data.definitions[f as usize].code, Value::Null);
                if let Value::Block(bl) = &mut code {
                    let mut ops = std::mem::take(&mut bl.operators);
                    let mut done = Vec::new();
                    while reuse_in_block(
                        &mut ops,
                        f,
                        data,
                        db,
                        &rules,
                        &mut variants,
                        &mut done,
                        trace,
                    ) {}
                    bl.operators = ops;
                }
                data.definitions[f as usize].code = code;
            }
        }
    }
    dump(data);
}
