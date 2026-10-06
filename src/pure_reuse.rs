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
fn reads_only(name: &str) -> bool {
    const PREFIXES: [&str; 6] = ["OpGet", "OpLength", "OpConv", "OpCast", "OpEq", "OpNe"];
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
fn write_target(name: &str) -> Option<usize> {
    match name {
        "OpCopyRecord" => Some(1),
        n if n.starts_with("OpSet") => Some(0),
        "OpDatabase" | "OpPreAllocVector" | "OpAppendVector" | "OpAppendCopy" | "OpNewRecord"
        | "OpFinishRecord" | "OpSliceVector" => Some(0),
        n if n.starts_with("OpPush") => Some(0),
        _ => None,
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
                } else if reads_only(name) {
                } else if let Some(t) = write_target(name) {
                    if args
                        .get(t)
                        .and_then(|a| root(a, data))
                        .is_none_or(|r| ps.contains(&r))
                    {
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
    matches!(tail, Value::Block(b) if b.name == "materialized_view_return")
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

/// One call this pass can reuse: the effect-free function, its argument variables, and the
/// read a wrapper applies (none for a bare call).
#[derive(Clone, PartialEq)]
struct Key {
    inner: u32,
    args: Vec<u16>,
}

/// The reusable call `v` is — a call of a projection wrapper — and how its value is read.
fn call_key(
    v: &Value,
    wrappers: &crate::fxhash::FxHashMap<u32, Projection>,
) -> Option<(Key, u32, Vec<Value>)> {
    let Value::Call(w, wargs) = v.unspan() else {
        return None;
    };
    let p = wrappers.get(w)?;
    let mut args = Vec::new();
    for &pos in &p.args {
        let Value::Var(x) = wargs.get(pos)?.unspan() else {
            return None;
        };
        args.push(*x);
    }
    Some((
        Key {
            inner: p.inner,
            args,
        },
        p.read,
        p.consts.clone(),
    ))
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

/// Does `v` write a store, call a function this pass cannot see the effects of, or assign
/// one of `vars`?
fn disturbs(v: &Value, vars: &[u16], data: &Data, free: &HashSet<u32>) -> bool {
    v.any_node(&mut |n| match n {
        Value::Set(x, _) => vars.contains(x),
        Value::Call(d, _) => {
            let name = data.def(*d).name();
            !(free.contains(d) || reads_only(name))
        }
        Value::CallRef(..) | Value::Yield(_) => true,
        _ => false,
    })
}

/// Reuse within one block's statements; answers whether it rewrote anything.
fn reuse_in_block(
    ops: &mut Vec<Value>,
    f: u32,
    data: &mut Data,
    free: &HashSet<u32>,
    wrappers: &crate::fxhash::FxHashMap<u32, Projection>,
    trace: bool,
) -> bool {
    // The first reusable call of each statement, with where it stands.
    for i in 0..ops.len() {
        let mut first_positions = Vec::new();
        always_evaluated(&ops[i], &mut first_positions);
        let Some((key, ..)) = first_positions.iter().find_map(|p| call_key(p, wrappers)) else {
            continue;
        };
        for j in i + 1..ops.len() {
            let mut positions = Vec::new();
            always_evaluated(&ops[j], &mut positions);
            if !positions
                .iter()
                .any(|p| call_key(p, wrappers).is_some_and(|(k, ..)| k == key))
            {
                // A statement between the two must disturb nothing the call reads.
                if disturbs(&ops[j], &key.args, data, free) {
                    break;
                }
                continue;
            }
            // Statement `i` beyond its call — its arms — must disturb nothing either.
            if disturbs_beside_call(&ops[i], &key, &key.args, data, free, wrappers) {
                break;
            }
            rewrite(ops, i, j, &key, f, data, wrappers);
            if trace {
                crate::loft_eprintln!(
                    "[pure-reuse] {}: {} computed once",
                    data.def(f).name(),
                    data.def(key.inner).name()
                );
            }
            return true;
        }
    }
    false
}

/// Does statement `s`, apart from its reusable call itself, disturb the call's arguments?
fn disturbs_beside_call(
    s: &Value,
    key: &Key,
    vars: &[u16],
    data: &Data,
    free: &HashSet<u32>,
    wrappers: &crate::fxhash::FxHashMap<u32, Projection>,
) -> bool {
    s.any_node(&mut |n| {
        if call_key(n, wrappers).is_some_and(|(k, ..)| &k == key) {
            return false;
        }
        match n {
            Value::Set(x, _) => vars.contains(x),
            Value::Call(d, _) => {
                let name = data.def(*d).name();
                !(free.contains(d) || reads_only(name) || wrappers.contains_key(d))
            }
            Value::CallRef(..) | Value::Yield(_) => true,
            _ => false,
        }
    })
}

/// Bind the call once before statement `i` and make both statements read it.
fn rewrite(
    ops: &mut Vec<Value>,
    i: usize,
    j: usize,
    key: &Key,
    f: u32,
    data: &mut Data,
    wrappers: &crate::fxhash::FxHashMap<u32, Projection>,
) {
    crate::rewrite_census::fired("R-PureReuse", 1);
    let inner = data.def(key.inner);
    let ret_tp = inner.returned.without_deps();
    let buf_tp = inner
        .hidden_return_buffer_attr()
        .map(|k| inner.attributes()[k].typedef.clone());
    let vars = &mut data.definitions[f as usize].variables;
    let local = vars.add_unique("reuse", &ret_tp, u16::MAX);
    let mut args: Vec<Value> = key.args.iter().map(|a| Value::Var(*a)).collect();
    let mut prelude = Vec::new();
    if let Some(btp) = buf_tp {
        let buf = vars.add_unique("reuse_buf", &btp, u16::MAX);
        prelude.push(Value::Set(buf, Box::new(Value::Null)));
        args.push(Value::Var(buf));
    }
    prelude.push(Value::Set(local, Box::new(Value::Call(key.inner, args))));
    for at in [i, j] {
        replace_calls(&mut ops[at], key, local, wrappers);
    }
    for (offset, stmt) in prelude.into_iter().enumerate() {
        ops.insert(i + offset, stmt);
    }
}

/// Replace each reusable call `key` in `v` by its read of `t`.
fn replace_calls(
    v: &mut Value,
    key: &Key,
    local: u16,
    wrappers: &crate::fxhash::FxHashMap<u32, Projection>,
) {
    if let Some((k, read, consts)) = call_key(v, wrappers)
        && &k == key
    {
        let mut args = vec![Value::Var(local)];
        args.extend(consts);
        *v = Value::Call(read, args);
        return;
    }
    v.for_each_child_mut(&mut |c| replace_calls(c, key, local, wrappers));
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
pub fn rewrite_program(data: &mut Data) {
    if crate::keys::pure_reuse_enabled() {
        let free = effect_free(data);
        let mut wrappers = crate::fxhash::FxHashMap::default();
        for w in candidates(data) {
            if let Some(p) = projection_of(data, w, &free) {
                wrappers.insert(w, p);
            }
        }
        if !wrappers.is_empty() {
            let trace = crate::keys::trace_pure_reuse();
            for f in candidates(data) {
                if data.def(f).variables.done {
                    continue;
                }
                let _in = crate::rewrite_census::InBody::enter("ir", &data.def(f).name.clone());
                let mut code =
                    std::mem::replace(&mut data.definitions[f as usize].code, Value::Null);
                if let Value::Block(bl) = &mut code {
                    let mut ops = std::mem::take(&mut bl.operators);
                    while reuse_in_block(&mut ops, f, data, &free, &wrappers, trace) {}
                    bl.operators = ops;
                }
                data.definitions[f as usize].code = code;
            }
        }
    }
    dump(data);
}
