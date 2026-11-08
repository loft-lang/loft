// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! `@FR-R-InRange` — a record element its loop proves in range needs no null discharge,
//! decided in the IR phase for both backends.
//!
//! `v[i]?` over a vector of records lowers to a discharge block: the nullable element read, and
//! a fallback record minted when it answers null.  An element read answers null for one reason
//! only when the element is a record — an index outside the vector; an in-range element of a
//! vector of records IS a record.  Inside `for i in 0..len(v)` whose body can neither resize `v`
//! nor assign `i` (the check `@FR-R-ForwardWalk` makes, `forward_walk::check_body`), every `i` is
//! in range, so the discharge block is the read alone.
//!
//! Not for a scalar element: `v[i]?` also discharges an element whose VALUE is null (a non-null
//! scalar type can hold the sentinel, @C80), which no loop bound rules out.
//!
//! `LOFT_NO_IN_RANGE=1` keeps every discharge; `LOFT_TRACE_IN_RANGE=1` names each loop rewritten.
use crate::data::{Block, Data, DefType, Value};
use crate::forward_walk::{Walk, check_body};

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_IN_RANGE").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_IN_RANGE").is_ok_and(|v| v != "0"))
}

struct Ops {
    add_int: u32,
    le_int: u32,
    length_vector: u32,
    vector_len: u32,
    get_vector_nullable: u32,
    conv_bool_from_ref: u32,
    get_field: u32,
}

fn var(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(n) => Some(*n),
        _ => None,
    }
}

/// The vector a range walks: a variable, or a field of one (`d.pieces`: the root and the
/// field's offset).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Src {
    Var(u16),
    Field(u16, i32),
}

fn src(ops: &Ops, v: &Value) -> Option<Src> {
    match v.unspan() {
        Value::Var(n) => Some(Src::Var(*n)),
        Value::Call(d, a) if *d == ops.get_field && a.len() == 3 => {
            match (a[0].unspan(), a[1].unspan()) {
                (Value::Var(root), Value::Int(off)) => Some(Src::Field(*root, *off)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn plain(ops: &[Value]) -> Vec<&Value> {
    ops.iter()
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect()
}

/// `for i in 0..len(v)` as the parser lowers it: the vector `v`, the counter `i#index`, the
/// loop variable `i`, and where the body starts in the loop.
/// `n = len(v)` — a length bound to a variable: the variable and the vector.
fn len_bind(ops: &Ops, stmt: &Value) -> Option<(u16, Src)> {
    let Value::Set(n, len) = stmt.unspan() else {
        return None;
    };
    let Value::Call(ld, largs) = len.unspan() else {
        return None;
    };
    if (*ld != ops.length_vector && *ld != ops.vector_len) || largs.len() != 1 {
        return None;
    }
    Some((*n, src(ops, &largs[0])?))
}

/// `for i in 0..len(v)`, or `for i in 0..n` right after `n = len(v)` (`alias`).
fn match_range(
    ops: &Ops,
    block: &Block,
    alias: Option<(u16, Src)>,
) -> Option<(Src, u16, u16, usize)> {
    if block.name != "For block" {
        return None;
    }
    let st = plain(&block.operators);
    let [
        Value::Set(end, len),
        Value::Set(idx, start),
        Value::Loop(lp),
    ] = st.as_slice()
    else {
        return None;
    };
    let v = match (len.unspan(), alias) {
        (Value::Var(n), Some((an, v))) if *n == an => v,
        _ => len_bind(ops, &Value::Set(*end, Box::new((**len).clone())))?.1,
    };
    if !matches!(start.unspan(), Value::Int(-1)) || lp.name != "For loop" {
        return None;
    }
    let body = plain(&lp.operators);
    let Value::Set(lv, iter) = body.first()? else {
        return None;
    };
    let Value::Block(it) = iter.unspan() else {
        return None;
    };
    let iops = plain(&it.operators);
    let [Value::Set(step_var, step), Value::If(test, _, _), last] = iops.as_slice() else {
        return None;
    };
    let Value::Call(sd, sargs) = step.unspan() else {
        return None;
    };
    let Value::Call(td, targs) = test.unspan() else {
        return None;
    };
    if it.name != "Iter range"
        || step_var != idx
        || *sd != ops.add_int
        || var(&sargs[0])? != *idx
        || !matches!(sargs[1].unspan(), Value::Int(1))
        || *td != ops.le_int
        || var(&targs[0])? != *end
        || var(&targs[1])? != *idx
        || var(last)? != *idx
    {
        return None;
    }
    let first_body = lp.operators.iter().position(|o| std::ptr::eq(o, body[0]))? + 1;
    Some((v, *idx, *lv, first_body))
}

/// The record element read a discharge block wraps, when it reads `v` at `i` or `i#index`.
fn record_discharge(ops: &Ops, node: &Value, v: Src, idx: u16, lv: u16) -> Option<Value> {
    let Value::Block(b) = node.unspan() else {
        return None;
    };
    if b.name != "ncc" {
        return None;
    }
    let st = plain(&b.operators);
    let [Value::Set(t, read), Value::If(test, then_arm, _)] = st.as_slice() else {
        return None;
    };
    let Value::Call(cd, cargs) = test.unspan() else {
        return None;
    };
    let Value::Call(rd, rargs) = read.unspan() else {
        return None;
    };
    let at = var(rargs.get(2)?)?;
    (*cd == ops.conv_bool_from_ref
        && cargs.len() == 1
        && var(&cargs[0]) == Some(*t)
        && var(then_arm) == Some(*t)
        && *rd == ops.get_vector_nullable
        && src(ops, &rargs[0]) == Some(v)
        && (at == idx || at == lv))
        .then(|| (**read).clone())
}

fn strip(ops: &Ops, node: &mut Value, v: Src, idx: u16, lv: u16, n: &mut usize) {
    if let Some(read) = record_discharge(ops, node, v, idx, lv) {
        *node = read;
        *n += 1;
        return;
    }
    node.for_each_child_mut(&mut |c| strip(ops, c, v, idx, lv, n));
}

fn rewrite_in(
    data: &Data,
    ops: &Ops,
    vars: &crate::variables::Function,
    node: &mut Value,
    fname: &str,
) -> usize {
    let mut n = 0;
    node.for_each_child_mut(&mut |c| n += rewrite_in(data, ops, vars, c, fname));
    let block = match node {
        Value::Block(b) => b,
        // A wrapped block was reached as this node's child, just above.
        Value::Span(_) => return n,
        _ => return n,
    };
    // `n = len(v)` and, as the very next statement, `for i in 0..n`: nothing between them can
    // shrink `v`, so `n` is its length where the loop starts.
    for k in 1..block.operators.len() {
        // The statement before, past the line marks.
        let Some(prev) = block.operators[..k]
            .iter()
            .rev()
            .find(|o| !matches!(o.unspan(), Value::Line(_)))
        else {
            continue;
        };
        let Some(alias) = len_bind(ops, prev) else {
            continue;
        };
        if let Value::Block(fb) = block.operators[k].unspan_mut() {
            n += rewrite_for(data, ops, vars, fb, Some(alias), fname);
        }
    }
    n + rewrite_for(data, ops, vars, block, None, fname)
}

/// One `for` block: its in-range discharges dropped when the range and the body allow.
fn rewrite_for(
    data: &Data,
    ops: &Ops,
    vars: &crate::variables::Function,
    block: &mut Block,
    alias: Option<(u16, Src)>,
    fname: &str,
) -> usize {
    let n = 0;
    let Some((v, idx, lv, first_body)) = match_range(ops, block, alias) else {
        return n;
    };
    let Some(Value::Loop(lp)) = block
        .operators
        .iter_mut()
        .find(|o| matches!(o, Value::Loop(_)))
    else {
        return n;
    };
    // The body may not assign the bound length either.
    if let Some((len_var, _)) = alias
        && lp.operators[first_body..]
            .iter()
            .any(|o| o.any_node(&mut |x| matches!(x, Value::Set(y, _) if *y == len_var)))
    {
        return n;
    }
    // The body may neither resize `v` nor assign `i` or `i#index`.
    match v {
        Src::Var(v) => {
            for counter in [idx, lv] {
                let w = Walk {
                    vec_t: v,
                    src: v,
                    idx: counter,
                    lv,
                    read: Value::Null,
                    iter_scope: 0,
                    field: None,
                };
                if vars.is_captured(v)
                    || lp.operators[first_body..]
                        .iter()
                        .any(|o| check_body(data, vars, &w, o).is_err())
                {
                    return n;
                }
            }
        }
        Src::Field(root, off) => {
            let why = field_decline(data, vars, root, off, idx, lv, &lp.operators[first_body..]);
            if let Some(why) = why {
                if trace() {
                    eprintln!("in-range: {fname} {} declined: {why}", vars.name(lv));
                }
                return n;
            }
        }
    }
    let mut k = 0;
    for o in &mut lp.operators[first_body..] {
        strip(ops, o, v, idx, lv, &mut k);
    }
    if k > 0 && trace() {
        eprintln!(
            "in-range: {fname} {} — {k} discharge(s) dropped",
            vars.name(lv)
        );
    }
    n + k
}

/// The field clause's body check, for `for i in 0..len(d.f)`: why the body might resize
/// `d.f` or move the counter, or `None`.  `d` is named only through its fields, never assigned,
/// handed bare to a call or captured; the walked field is named only as the vector a read
/// operator reads; a `&` parameter that could be `d` itself declines, and so does any vector
/// local that is a view into `d`, since a write through it would name no `d` at all.
pub(crate) fn field_decline(
    data: &Data,
    vars: &crate::variables::Function,
    root: u16,
    off: i32,
    idx: u16,
    lv: u16,
    body: &[Value],
) -> Option<&'static str> {
    if vars.is_captured(root) {
        return Some("a closure captures the record");
    }
    for x in 0..vars.count() {
        if x != root
            && matches!(vars.tp(x).base(), crate::data::Type::Vector(..))
            && vars.tp(x).depend().contains(&root)
            && body
                .iter()
                .any(|o| o.any_node(&mut |n| matches!(n, Value::Var(y) if *y == x)))
        {
            return Some("a vector local that views the record is used in the body");
        }
    }
    let cx = FieldWalk {
        data,
        vars,
        root,
        off,
        root_is_param: vars.is_argument(root),
        idx,
        lv,
    };
    body.iter().try_for_each(|o| cx.walk(o)).err()
}

/// What the field clause's body check carries through the walk.
struct FieldWalk<'a> {
    data: &'a Data,
    vars: &'a crate::variables::Function,
    root: u16,
    off: i32,
    root_is_param: bool,
    idx: u16,
    lv: u16,
}

impl FieldWalk<'_> {
    /// `d.f` itself, the walked field.
    fn ours(&self, v: &Value) -> bool {
        matches!(v.unspan(), Value::Call(d, a) if a.len() == 3
            && self.data.def(*d).name() == "OpGetField"
            && matches!(a[0].unspan(), Value::Var(r) if *r == self.root)
            && matches!(a[1].unspan(), Value::Int(o) if *o == self.off))
    }

    fn walk(&self, node: &Value) -> Result<(), &'static str> {
        match node {
            Value::Span(sp) => self.walk(&sp.1),
            Value::Var(n) if *n == self.root => {
                Err("the record is used other than through a field")
            }
            Value::Set(n, e) => {
                if *n == self.root || *n == self.idx || *n == self.lv {
                    return Err("the body assigns the record or the counter");
                }
                self.walk(e)
            }
            Value::CallRef(..) | Value::Yield(_) | Value::Parallel(_) => {
                Err("a call through a reference, a yield or a par in the body")
            }
            Value::Call(op, args) => {
                let name = self.data.def(*op).name();
                if self.ours(node) {
                    return Err("the walked vector is used other than by a read");
                }
                // A field of the record, read whole: another field, or the record's own scalar.
                if name == "OpGetField"
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(r)) if *r == self.root)
                {
                    return Ok(());
                }
                let reads = READS.contains(&name);
                for a in args {
                    if self.ours(a) {
                        if reads {
                            continue;
                        }
                        return Err(
                            "the walked vector is handed to an operator that may resize it",
                        );
                    }
                    if let Value::Var(n) = a.unspan()
                        && self.root_is_param
                        && *n != self.root
                        && self.vars.is_argument(*n)
                        && match self.vars.tp(*n).base() {
                            // A record of scalars only owns no vector: it is not `d`, holds
                            // no part of it, and no write through it resizes one (a hidden
                            // `Cell` result buffer, `get_cell`).
                            crate::data::Type::Reference(t, _)
                                if crate::data::all_scalar_record(self.data, *t) =>
                            {
                                false
                            }
                            crate::data::Type::RefVar(t) => match t.base() {
                                crate::data::Type::Reference(r, _) => {
                                    !crate::data::all_scalar_record(self.data, *r)
                                }
                                crate::data::Type::Vector(..) => true,
                                _ => false,
                            },
                            // A record or vector passed plainly names the caller's record too:
                            // `drain(y, y)` empties `y.pieces` through `e` while `d.pieces` is
                            // walked (a-parameters-vector-holds-its-header-where-the-buffer-is-
                            // apart.loft, h7).  Its use is a field read, so the operator alone
                            // cannot tell a write.
                            crate::data::Type::Reference(..) | crate::data::Type::Vector(..) => {
                                true
                            }
                            _ => false,
                        }
                    {
                        // A `&text` or a `&` scalar (a result buffer among them) holds no record.
                        return Err("another record parameter, which may be the same record");
                    }
                    self.walk(a)?;
                }
                Ok(())
            }
            other => {
                let mut res = Ok(());
                let mut node = other.clone();
                node.for_each_child_mut(&mut |c| {
                    if res.is_ok() {
                        res = self.walk(c);
                    }
                });
                res
            }
        }
    }
}

/// The operators that read the walked vector and change nothing about it.
const READS: [&str; 6] = [
    "OpGetVector",
    "OpGetVectorNullable",
    "OpLengthVector",
    "t_6vector_len",
    "OpVecGetInt",
    "OpVecGetIntNullable",
];

/// Drop the discharge of every in-range record element read; answers how many.
pub fn rewrite_program(data: &mut Data) -> usize {
    if off() {
        return 0;
    }
    let ops = Ops {
        add_int: data.def_nr("OpAddInt"),
        le_int: data.def_nr("OpLeInt"),
        length_vector: data.def_nr("OpLengthVector"),
        vector_len: data.def_nr("t_6vector_len"),
        get_vector_nullable: data.def_nr("OpGetVectorNullable"),
        conv_bool_from_ref: data.def_nr("OpConvBoolFromRef"),
        get_field: data.def_nr("OpGetField"),
    };
    let mut total = 0;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        if !def
            .code()
            .any_node(&mut |n| matches!(n, Value::Block(b) if b.name == "ncc"))
        {
            continue;
        }
        let fname = def.name().to_string();
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let vars = data.definitions[d as usize].variables.clone();
        let n = rewrite_in(data, &ops, &vars, &mut code, &fname);
        data.definitions[d as usize].code = code;
        if n > 0 {
            data.definitions[d as usize].variables.reset_intervals();
            crate::scopes::compute_function_intervals(data, d);
            crate::scopes::assign_function_slots(data, d);
            total += n;
        }
    }
    crate::rewrite_census::fired("R-InRange", total);
    total
}
