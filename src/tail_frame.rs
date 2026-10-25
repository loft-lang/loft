// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): which self-calls sit in tail position, and whether a frame can be reused by a jump

//! `@FR-R-TailSelf`'s analysis: the tail self-calls of a body and whether its frame can be
//! reused by a jump.  Here, in the front layer, because the parser asks it for its refusal
//! (@C155, on the source shape) and `opt::tail_self` for the rewrite (on the lowered one).

use crate::data::{Data, Definition, Type, Value, is_scalar};
use crate::lexer::Position;
use crate::variables::Function;

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
/// that is a scalar or nothing; no `&` local; every argument a tail self-call hands to a
/// parameter held in a store, or to a `&` parameter, is a pointer the frame does not own (a
/// parameter or a projection of one); no generator, `parallel` block or closure in the body.
/// A local may be held in a store — the jump releases it.  `params` is `declared_params`'
/// list.  `Err` names what blocks it, with the variable and its type, for the refusal's text.
pub(crate) fn admit_frame(
    vars: &Function,
    result: &Type,
    code: &Value,
    data: &Data,
    params: &[u16],
    d: u32,
) -> Result<(), String> {
    if !(is_scalar(result) || matches!(result.base(), Type::Void | Type::Null)) {
        return Err(format!(
            "its result is a `{}`, which is delivered into the caller's buffer, not a scalar",
            result.source_name(data)
        ));
    }
    for v in 0..vars.count() {
        if matches!(vars.tp(v).base(), Type::RefVar(_)) && !params.contains(&v) {
            return Err(format!("local `{}` is a `&` reference", vars.name(v)));
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
                return Err(format!(
                    "parameter `{}` (a `{}`) can only be passed through as itself here",
                    vars.name(p),
                    tp.source_name(data)
                ));
            }
            if !borrows_from_params(arg, params, data) {
                return Err(format!(
                    "the call's argument for parameter `{}` (a `{}`) makes a value this frame \
                     would have to own — a fresh value, or a local the jump releases — where a \
                     call only shares; pass the parameter itself or a part of it (`{0}`, \
                     `{0}.field`, `{0}[i]`)",
                    vars.name(p),
                    tp.source_name(data)
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
            return Err(format!(
                "parameter `{}` (a `{}`) is rebound in the body, so this frame owns its store",
                vars.name(p),
                tp.source_name(data)
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
        return Err("the body holds a generator, a `parallel` block or a closure".to_string());
    }
    Ok(())
}

/// Does the body write the VARIABLE `v` itself — not a field or element through it?  A `Set`,
/// or an op that takes the variable as its destination: the parser lowers a record rebind to
/// `OpInitRefSentinel(p); OpDatabase(p, …)` and a text one to a clear and appends, and the
/// scope pass marks a rebindable parameter with `OpFreeRefIfDistinct(p, witness)`.  A field
/// write (`OpSetInt(p, …)`) is a mutation through the pointer, which `(F-ParamGrow)` allows,
/// and is not one of these.
pub(crate) fn writes_variable(code: &Value, v: u16, data: &Data) -> bool {
    code.any_node(&mut |n| match n {
        Value::Set(x, _) => *x == v,
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

/// Is `arg` a pointer the frame does not own: a parameter, or a projection (a field, an
/// element, a reference read) rooted at one?  Anything else — a call's result, a literal, an
/// operator's result, a local — is a value the frame makes or releases, which a call never
/// hands over into the callee's ownership either.
pub(crate) fn borrows_from_params(arg: &Value, params: &[u16], data: &Data) -> bool {
    match arg.unspan() {
        Value::Var(q) => params.contains(q),
        Value::Call(op, items) if !items.is_empty() => {
            let name = data.def(*op).name();
            (crate::use_analysis::is_projection_op(data, *op) || name == "OpGetDbRef")
                && borrows_from_params(&items[0], params, data)
        }
        _ => false,
    }
}

/// Is `v` a release the scope pass emits at a scope's end — a free, a clear, a closure drop?
pub(crate) fn is_release(data: &Data, v: &Value) -> bool {
    matches!(v.unspan(), Value::Call(op, _) if {
        let name = data.def(*op).name();
        name.starts_with("OpFree") || name.starts_with("OpClear") || name == "OpDropFnRef"
    })
}

pub(crate) fn is_line(v: &Value) -> bool {
    matches!(v.unspan(), Value::Line(_))
}

/// The index of a block's value: its last operator that is not a source-line marker.
pub(crate) fn last_value(ops: &[Value]) -> Option<usize> {
    ops.iter().rposition(|op| !is_line(op))
}

/// The scope pass's return shape: `ops[k]` is `__ret_N = E`, then releases (and line markers)
/// only, then at `ops[j]` `return __ret_N` — or, for a block in value position, `__ret_N` as
/// the block's last operator.  Answers `(k, j)`; `E` is then in value position with those
/// releases as the epilogue a jump must run.
pub(crate) fn ret_pattern(
    ops: &[Value],
    vars: &Function,
    data: &Data,
    in_value: bool,
) -> Option<(usize, usize)> {
    let last = last_value(ops);
    for (k, op) in ops.iter().enumerate() {
        let Value::Set(r, _) = op.unspan() else {
            continue;
        };
        if !vars.name(*r).starts_with("__ret_") {
            continue;
        }
        let mut j = k + 1;
        while j < ops.len() && (is_release(data, &ops[j]) || is_line(&ops[j])) {
            j += 1;
        }
        if j >= ops.len() {
            return None;
        }
        match ops[j].unspan() {
            Value::Return(e) if matches!(e.unspan(), Value::Var(x) if x == r) => {
                return Some((k, j));
            }
            Value::Var(x) if x == r && in_value && Some(j) == last => return Some((k, j)),
            _ => {}
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
/// a void body, the `__ret_N` pattern, and the value assigned to a hidden `&` result buffer (a
/// heap result's delivery, which the parser lowers before this is asked).
pub(crate) fn tail_calls(code: &Value, d: u32, vars: &Function, data: &Data) -> Vec<TailCall> {
    let f = Finder { d, vars, data };
    let mut out = Vec::new();
    f.block(code.unspan(), true, None, &mut out);
    out
}

pub(crate) struct Finder<'a> {
    d: u32,
    vars: &'a Function,
    data: &'a Data,
}

impl Finder<'_> {
    /// `in_value`: the block's last value is a tail (a result or a void body's last statement).
    fn block(&self, v: &Value, in_value: bool, at: Option<Position>, out: &mut Vec<TailCall>) {
        let Value::Block(b) = v.unspan() else {
            return;
        };
        let ops = &b.operators;
        let last = last_value(ops);
        let pat = ret_pattern(ops, self.vars, self.data, in_value);
        let vtail = if in_value {
            void_tail(ops, self.data)
        } else {
            None
        };
        for (i, op) in ops.iter().enumerate() {
            if let Some((k, _)) = pat
                && i == k
                && let Value::Set(_, e) = op.unspan()
            {
                self.value(e, at, out);
            } else if pat.is_some_and(|(_, j)| j == i) {
                // The pattern's own `return __ret_N`, no tail of its own.
            } else if in_value && (Some(i) == last || Some(i) == vtail) {
                self.value(op, at, out);
            } else {
                self.statement(op, at, out);
            }
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
            Value::Return(e) => self.value(e, at, out),
            other => self.statement(other, at, out),
        }
    }

    fn statement(&self, v: &Value, at: Option<Position>, out: &mut Vec<TailCall>) {
        match v {
            Value::Span(bx) => self.statement(&bx.1, Some(bx.0), out),
            Value::Return(e) => self.value(e, at, out),
            Value::Block(_) => self.block(v, false, at, out),
            // A heap result is delivered into the compiler's hidden `&` buffer: `___acc = f(…)`
            // is the tail call of a text-returning function, which the parser lowers this way.
            // A user local promoted to the buffer keeps its own name, and `t = f(…); t` is the
            // bound spelling that keeps a call, whatever the result's type.
            Value::Set(x, e)
                if *x < self.vars.count()
                    && self.vars.is_argument(*x)
                    && self.vars.name(*x).starts_with("__")
                    && matches!(self.vars.tp(*x).base(), Type::RefVar(_)) =>
            {
                self.value(e, at, out);
            }
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
