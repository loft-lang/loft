// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): which self-calls sit in tail position, and whether a frame can be reused by a jump

//! `@FR-R-TailSelf`'s PREDICATE — the one answer to "does this function call itself in tail
//! position, and can its frame be reused by a jump?" — asked twice: by the parser, which
//! REFUSES a frame it cannot reuse (@C155, `definitions.rs`, on the source shape), and by the
//! rewrite (`opt::tail_self`, on the lowered shape), so that what compiles is exactly what
//! jumps.  It reads both shapes: a `return`'s value, an `if` arm's or a block's value in that
//! position, the last statement of a void body, and the value written to a DELIVERY PLACE — a
//! compiler temporary (`__ret_N`, the join's `__ref_p2_N` and `___ret_join_N`, the hidden
//! `&text` buffer `___acc_N`) before the releases and the `return` of it.  Here, below the
//! parser in the crate's layers, because the parser may not name the optimiser.

use crate::data::{Data, Definition, Type, Value, is_scalar};
use crate::lexer::Position;
use crate::variables::Function;

/// `LOFT_NO_TAIL_SELF=1` switches the whole rule off — the jump AND the parser's refusal — so a
/// tail-recursive program can be bisected as the plain call.
pub(crate) fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_TAIL_SELF").is_ok_and(|v| v != "0"))
}

/// The declared parameters, as variable numbers: the definition's attributes that are not
/// hidden, in order.  A hidden attribute is a buffer a lowering added after the signature (a
/// promoted text local, a return buffer, an apart vector), and the table's own `argument` flag
/// marks those too, so neither is the test for "a parameter the call binds".
pub(crate) fn declared_params(def: &Definition) -> Vec<u16> {
    def.attributes()
        .iter()
        .enumerate()
        .filter(|(_, a)| !a.hidden)
        .map(|(i, _)| i as u16)
        .collect()
}

/// Can this frame be reused by a jump?  The one predicate for the parser's refusal (@C155,
/// `definitions.rs`, on the source shape) and for this pass (on the lowered one): a result
/// that is a scalar, nothing, a text or a store-held value (the kinds the scope pass delivers
/// through a temporary or a buffer); no `&` local; every argument a tail self-call hands to a
/// parameter held in a store is a pointer the frame does not own (a parameter or a projection
/// of one), a text or `&` parameter only itself; no parameter rebound in the body; no
/// generator, `parallel` block or closure in the body.  A local may be held in a store — the
/// jump releases it.  `params` is `declared_params`' list.  `Err` names what blocks it, with
/// the variable and its type, for the refusal's text.
pub(crate) fn admit_frame(
    vars: &Function,
    result: &Type,
    code: &Value,
    data: &Data,
    params: &[u16],
    d: u32,
) -> Result<(), String> {
    admit_frame_at(vars, result, code, data, params, d).map_err(|(_, why)| why)
}

/// [`admit_frame`] with WHERE it was refused: the self-call whose argument blocks the jump,
/// or `None` for a reason the whole frame carries (its result, a `&` local, a rebound
/// parameter, a generator).  The refusal is reported at that call, because its text names
/// that call's argument — reported at another tail call, it told the author to write what
/// that call already wrote.
pub(crate) fn admit_frame_at(
    vars: &Function,
    result: &Type,
    code: &Value,
    data: &Data,
    params: &[u16],
    d: u32,
) -> Result<(), (Option<Position>, String)> {
    let delivered = is_scalar(result)
        || matches!(result.base(), Type::Void | Type::Null | Type::Text(_))
        || crate::data::is_dbref(result.base());
    if !delivered {
        return Err((
            None,
            format!(
                "its result is a `{}`, which the jump cannot yet deliver",
                result.source_name(data)
            ),
        ));
    }
    // A `&` LOCAL has no raw pointer move; the hidden `&text` buffer of a text result is an
    // argument (hidden, so not a declared parameter) and the delivery place, not a local.
    for v in 0..vars.count() {
        if matches!(vars.tp(v).base(), Type::RefVar(_)) && !vars.is_argument(v) {
            return Err((None, format!("local `{}` is a `&` reference", vars.name(v))));
        }
    }
    for call in tail_calls(code, d, vars, data) {
        for (i, &p) in params.iter().enumerate() {
            let tp = vars.tp(p);
            if is_scalar(tp) {
                continue;
            }
            let Some(arg) = call.args.get(i) else {
                continue;
            };
            let identity = matches!(arg.unspan(), Value::Var(q) if *q == p);
            if identity {
                continue;
            }
            // A text and a `&` parameter have no raw pointer move in the bytecode (a text's
            // assignment owns, a `&` assignment writes through), so they pass through only
            // as themselves.
            if !crate::data::is_dbref_slot(tp.base()) {
                return Err((
                    call.at,
                    format!(
                        "parameter `{}` (a `{}`) can only be passed through as itself here",
                        vars.name(p),
                        tp.source_name(data)
                    ),
                ));
            }
            if !borrows_from_params(arg, params, code, data) {
                return Err((
                    call.at,
                    format!(
                        "the call's argument for parameter `{}` (a `{}`) makes a value this \
                         frame would have to own — a fresh value, or a local the jump releases \
                         — where a call only shares; pass the parameter itself or a part of it \
                         (`{0}`, `{0}.field`, `{0}[i]`, a `match` binding of one)",
                        vars.name(p),
                        tp.source_name(data)
                    ),
                ));
            }
        }
    }
    // A heap parameter REBOUND in the body (`p = S { … }`, `(F-ParamRebind)`) holds a store
    // this frame owns, which the epilogue releases: passed through at the jump it would be read
    // into its temporary and then freed under it.  A `&` parameter written through keeps its
    // pointer, so the write is no rebind.
    for &p in params {
        let tp = vars.tp(p);
        if is_scalar(tp) || matches!(tp.base(), Type::RefVar(_)) {
            continue;
        }
        if writes_variable(code, p, data) {
            return Err((
                None,
                format!(
                    "parameter `{}` (a `{}`) is rebound in the body, so this frame owns its store",
                    vars.name(p),
                    tp.source_name(data)
                ),
            ));
        }
    }
    let blocked = code.any_node(&mut |n| {
        matches!(
            n,
            Value::Yield(_) | Value::Parallel(_) | Value::FnRef(_, _, _) | Value::FnRefDnr(_)
        )
    });
    if blocked {
        return Err((
            None,
            "the body holds a generator, a `parallel` block or a closure".to_string(),
        ));
    }
    Ok(())
}

/// Does the body write the VARIABLE `v` itself — not a field or element through it?  A `Set`,
/// or an op that takes the variable as its destination: the parser lowers a record rebind to
/// `OpInitRefSentinel(p); OpDatabase(p, …)` and a text one to a clear and appends, and the
/// scope pass marks a rebindable parameter with `OpFreeRefIfDistinct(p, witness)`.  A field
/// write (`OpSetInt(p, …)`) is a mutation through the pointer, which `(F-ParamGrow)` allows,
/// and is not one of these.
fn writes_variable(code: &Value, v: u16, data: &Data) -> bool {
    code.any_node(&mut |n| matches!(n, Value::Set(x, _) if *x == v))
        || op_writes_variable(code, v, data)
}

/// `writes_variable` without the `Set`s: an op that takes the variable as its destination.
fn op_writes_variable(code: &Value, v: u16, data: &Data) -> bool {
    code.any_node(&mut |n| match n {
        Value::Call(op, items) if !items.is_empty() => {
            let name = data.def(*op).name();
            matches!(items[0].unspan(), Value::Var(x) if *x == v)
                && (name.starts_with("OpInit")
                    || name.starts_with("OpDatabase")
                    || name.starts_with("OpFree")
                    || name.starts_with("OpClear")
                    || name.starts_with("OpAppend")
                    || name.starts_with("OpCreate")
                    || name == "OpPutRef")
        }
        _ => false,
    })
}

/// Is `arg` a pointer the frame does not own: a parameter, or a part of one (`@FR-R-TailSelf`'s
/// "projection rooted at one")?  A part has three spellings: a projection op (a field, an
/// element, a reference read) on a part; the variant check a field read of an ENUM lowers to,
/// `if <tag test> P else OpNullRefSentinel()`, whose other arm is the null pointer and owns
/// nothing; and a local every write of which is a part, which is how a `match` payload binding
/// (`_mv_<name>_N = OpGetField(p, …)`) holds one.  Anything else — a call's result, a literal,
/// an operator's result, any other local — is a value the frame makes or releases, which a
/// call never hands over into the callee's ownership either, so the fallback answers no.
fn borrows_from_params(arg: &Value, params: &[u16], code: &Value, data: &Data) -> bool {
    part_of_params(arg, params, code, data, 0)
}

/// `borrows_from_params` at binding depth `depth`: a local is followed to its writes at most
/// a few levels deep (a binding of a binding), which also ends a local written from itself.
fn part_of_params(arg: &Value, params: &[u16], code: &Value, data: &Data, depth: u8) -> bool {
    match arg.unspan() {
        Value::Var(q) if params.contains(q) => true,
        Value::Var(q) => depth < 4 && binds_a_part(*q, params, code, data, depth + 1),
        Value::Call(op, items) if !items.is_empty() => {
            let name = data.def(*op).name();
            (crate::use_analysis::is_projection_op(data, *op) || name == "OpGetDbRef")
                && part_of_params(&items[0], params, code, data, depth)
        }
        Value::If(_, a, b) => {
            let sentinel_a = is_null_ref_sentinel(a, data);
            let sentinel_b = is_null_ref_sentinel(b, data);
            !(sentinel_a && sentinel_b)
                && (sentinel_a || part_of_params(a, params, code, data, depth))
                && (sentinel_b || part_of_params(b, params, code, data, depth))
        }
        _ => false,
    }
}

fn is_null_ref_sentinel(v: &Value, data: &Data) -> bool {
    matches!(v.unspan(), Value::Call(op, items)
        if items.is_empty() && data.def(*op).name() == "OpNullRefSentinel")
}

/// Is local `q` a part of a parameter wherever it is read: written only by `Set`s whose value
/// is null or a part, and by no op (a store claim, a free, a raw pointer write)?  An op that
/// writes it makes it a store this frame owns or releases, so the answer is no.
fn binds_a_part(q: u16, params: &[u16], code: &Value, data: &Data, depth: u8) -> bool {
    if op_writes_variable(code, q, data) {
        return false;
    }
    let mut parts = true;
    code.any_node(&mut |n| {
        if let Value::Set(x, e) = n
            && *x == q
            && !matches!(e.unspan(), Value::Null)
            && !part_of_params(e, params, code, data, depth)
        {
            parts = false;
        }
        false
    });
    parts
}

/// Is `v` a release the scope pass emits at a scope's end — a free, a clear, a closure drop —
/// or one it guards (`if OpDistinctStore(…) OpFreeRefIfDistinct(…) else null`, the return
/// buffer's witness test)?
pub(crate) fn is_release(data: &Data, v: &Value) -> bool {
    match v.unspan() {
        Value::Call(op, _) => {
            let name = data.def(*op).name();
            name.starts_with("OpFree") || name.starts_with("OpClear") || name == "OpDropFnRef"
        }
        Value::If(_, a, b) => {
            is_release(data, a) && (matches!(b.unspan(), Value::Null) || is_release(data, b))
        }
        _ => false,
    }
}

fn is_line(v: &Value) -> bool {
    matches!(v.unspan(), Value::Line(_))
}

/// The index of a block's value: its last operator that is not a source-line marker.
pub(crate) fn last_value(ops: &[Value]) -> Option<usize> {
    ops.iter().rposition(|op| !is_line(op))
}

/// Is `x` a place the scope pass DELIVERS the result through: a compiler temporary — `__ret_N`,
/// `__ret_tail_N`, the join's `___ret_join_N` and `__ref_p2_N`, the hidden `&text` buffer
/// `___acc_N` / `___tret_N` — which is any variable whose name the compiler minted (`__…`)?  A
/// user local promoted to the buffer keeps its own name, so `t = f(…); t` stays the bound
/// spelling that keeps a call, whatever the result's type.
fn is_delivery_place(x: u16, vars: &Function) -> bool {
    x < vars.count() && vars.name(x).starts_with("__")
}

/// The scope pass's delivery shape in a block: the terminal at `ops[j]` — `return x`, or in
/// value position (`in_value`) the bare `x` the block answers, which is the same shape before
/// the scope pass wraps the body's tail in its `return` (a generic instance is asked at its
/// instantiation, before that) — `x` a delivery place, preceded by releases (and line markers)
/// only, preceded at `ops[m]` by the statement that WRITES `x` — `x = E` itself, or an `if`
/// whose arms write it.  Every `x = E` in that statement puts `E` in value position, with those
/// releases as the epilogue a jump must run.  Answers `(m, j, x)`.
pub(crate) fn delivery(
    ops: &[Value],
    vars: &Function,
    data: &Data,
    in_value: bool,
) -> Option<(usize, usize, u16)> {
    let j = last_value(ops)?;
    let e = match ops[j].unspan() {
        Value::Return(e) => e.unspan(),
        bare @ Value::Var(_) if in_value => bare,
        _ => return None,
    };
    let Value::Var(x) = e else {
        return None;
    };
    if !is_delivery_place(*x, vars) {
        return None;
    }
    let mut m = j;
    while m > 0 && (is_release(data, &ops[m - 1]) || is_line(&ops[m - 1])) {
        m -= 1;
    }
    if m == 0 {
        return None;
    }
    let m = m - 1;
    writes_place(&ops[m], *x).then_some((m, j, *x))
}

/// The join lowering delivers in two steps — `tmp = E; x = tmp`, `tmp` a compiler
/// temporary: at `ops[i]` the first step whose second follows at the next operator (line
/// markers aside), so `E` is in value position.  Answers `(i, tmp)`.
pub(crate) fn two_step(ops: &[Value], i: usize, x: u16, vars: &Function) -> Option<u16> {
    let Value::Set(tmp, _) = ops[i].unspan() else {
        return None;
    };
    if *tmp == x || !is_delivery_place(*tmp, vars) {
        return None;
    }
    let next = ops[i + 1..].iter().find(|op| !is_line(op))?;
    matches!(next.unspan(), Value::Set(y, e) if *y == x && matches!(e.unspan(), Value::Var(z) if z == tmp))
        .then_some(*tmp)
}

/// Does statement `v` write `x` — itself, or through an `if` arm or a block it holds?
pub(crate) fn writes_place(v: &Value, x: u16) -> bool {
    match v.unspan() {
        Value::Set(y, _) => *y == x,
        Value::If(_, a, b) => writes_place(a, x) || writes_place(b, x),
        Value::Block(bl) => bl.operators.iter().any(|op| writes_place(op, x)),
        _ => false,
    }
}

/// The scope pass's return shape inside a block in VALUE position: `ops[k]` is `__ret_N = E`
/// (or the join's `__ref_p2_N = E`), then releases (and line markers) only, then that
/// temporary as the block's last operator.  `E` is then in value position with those releases
/// as the epilogue.  Answers `(k, j)`.
pub(crate) fn ret_pattern(ops: &[Value], vars: &Function, data: &Data) -> Option<(usize, usize)> {
    let last = last_value(ops);
    for (k, op) in ops.iter().enumerate() {
        let Value::Set(r, _) = op.unspan() else {
            continue;
        };
        if !is_delivery_place(*r, vars) {
            continue;
        }
        let mut j = k + 1;
        while j < ops.len() && (is_release(data, &ops[j]) || is_line(&ops[j])) {
            j += 1;
        }
        if j >= ops.len() {
            return None;
        }
        if matches!(ops[j].unspan(), Value::Var(x) if x == r) && Some(j) == last {
            return Some((k, j));
        }
    }
    None
}

/// The last operator of a void body that is not a release: a self-call there is the tail,
/// and the releases after it are its epilogue.
pub(crate) fn void_tail(ops: &[Value], data: &Data) -> Option<usize> {
    ops.iter()
        .rposition(|op| !is_line(op) && !is_release(data, op))
}

/// A self-call in tail position: where it stands (the innermost `Span` on the way to it) and
/// the arguments it passes.
pub(crate) struct TailCall {
    pub(crate) at: Option<Position>,
    pub(crate) args: Vec<Value>,
}

/// Every self-call in tail position of `code` (a function's body).  The gate for the pass and
/// for the parser's refusal, so it reads both the source shape and the lowered one: a
/// `return`'s value, an `if` arm's or a block's value in that position, the last statement of
/// a void body, and the value written to a delivery place (`__ret_N`, the hidden text buffer)
/// before the releases and the `return` of it.
pub(crate) fn tail_calls(code: &Value, d: u32, vars: &Function, data: &Data) -> Vec<TailCall> {
    let f = Finder { d, vars, data };
    let mut out = Vec::new();
    f.body(code.unspan(), &mut out);
    out
}

struct Finder<'a> {
    d: u32,
    vars: &'a Function,
    data: &'a Data,
}

impl Finder<'_> {
    /// A function's body: a block, or the `Insert` the join lowering splices around one
    /// (`___ret_join_N = null` before the block).
    fn body(&self, v: &Value, out: &mut Vec<TailCall>) {
        match v.unspan() {
            Value::Block(b) => self.ops(&b.operators, true, None, out),
            Value::Insert(items) => self.ops(items, true, None, out),
            _ => {}
        }
    }

    /// `in_value`: the block's last value is a tail (a result or a void body's last statement).
    fn block(&self, v: &Value, in_value: bool, at: Option<Position>, out: &mut Vec<TailCall>) {
        if let Value::Block(b) = v.unspan() {
            self.ops(&b.operators, in_value, at, out);
        }
    }

    fn ops(&self, ops: &[Value], in_value: bool, at: Option<Position>, out: &mut Vec<TailCall>) {
        let last = last_value(ops);
        let deliver = delivery(ops, self.vars, self.data, in_value);
        let pat = if in_value && deliver.is_none() {
            ret_pattern(ops, self.vars, self.data)
        } else {
            None
        };
        let vtail = if in_value {
            void_tail(ops, self.data)
        } else {
            None
        };
        for (i, op) in ops.iter().enumerate() {
            if let Some((m, _, x)) = deliver
                && i == m
            {
                self.deliver(op, x, at, out);
            } else if deliver.is_some_and(|(_, j, _)| j == i) {
                // The delivery's own `return x`, no tail of its own.
            } else if let Some((k, _)) = pat
                && i == k
                && let Value::Set(_, e) = op.unspan()
            {
                self.value(e, at, out);
            } else if pat.is_some_and(|(_, j)| j == i) {
                // The pattern's own `__ret_N`, no tail of its own.
            } else if in_value && (Some(i) == last || Some(i) == vtail) {
                self.value(op, at, out);
            } else {
                self.statement(op, at, out);
            }
        }
    }

    /// The statement that writes the delivery place `x`: every `x = E` puts `E` in value
    /// position.
    fn deliver(&self, v: &Value, x: u16, at: Option<Position>, out: &mut Vec<TailCall>) {
        match v {
            Value::Span(bx) => self.deliver(&bx.1, x, Some(bx.0), out),
            Value::Set(y, e) if *y == x => self.value(e, at, out),
            Value::If(_, a, b) => {
                self.deliver(a, x, at, out);
                self.deliver(b, x, at, out);
            }
            Value::Block(bl) => {
                let ops = &bl.operators;
                for (i, op) in ops.iter().enumerate() {
                    if two_step(ops, i, x, self.vars).is_some()
                        && let Value::Set(_, e) = op.unspan()
                    {
                        self.value(e, at, out);
                    } else if writes_place(op, x) {
                        self.deliver(op, x, at, out);
                    } else {
                        self.statement(op, at, out);
                    }
                }
            }
            other => self.statement(other, at, out),
        }
    }

    fn value(&self, v: &Value, at: Option<Position>, out: &mut Vec<TailCall>) {
        match v {
            Value::Span(bx) => self.value(&bx.1, Some(bx.0), out),
            Value::Call(c, args) if *c == self.d => out.push(TailCall {
                at,
                args: args.clone(),
            }),
            Value::If(_, a, b) => {
                self.value(a, at, out);
                self.value(b, at, out);
            }
            Value::Block(_) => self.block(v, true, at, out),
            // A block's second spelling: the scope pass splices a hoisted declaration in front
            // of an arm (`_mv_deeper_1 = null; if …`), and its last operator is the value.
            Value::Insert(items) => self.ops(items, true, at, out),
            Value::Return(e) => self.value(e, at, out),
            other => self.statement(other, at, out),
        }
    }

    fn statement(&self, v: &Value, at: Option<Position>, out: &mut Vec<TailCall>) {
        match v {
            Value::Span(bx) => self.statement(&bx.1, Some(bx.0), out),
            Value::Return(e) => self.value(e, at, out),
            Value::Block(_) => self.block(v, false, at, out),
            other => other.for_each_child(&mut |c| self.statement(c, at, out)),
        }
    }
}

/// What a value position means to the value that is not a jump.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pos {
    /// The value the function answers: it becomes `return value`.
    Return,
    /// The value of a `__ret_N = …` whose releases and `return` follow: it stays the value,
    /// so the releases run on that path.
    Keep,
    /// The last statement of a void body: it stays a statement.
    Void,
}
