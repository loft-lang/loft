// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the call shapes it shares with the parser

//! Small readers of the IR's call shapes, asked by the parser and the native generator
//! alike: a call to a definition by name, a bare variable or integer operand, a standard-
//! library function whose whole body is one op (`one_op_wrapper`), and a `split` with a
//! constant separator into a hidden buffer (`split_call`).  Pure functions of `Data`, so
//! they sit in the core layer with it (@PLN191).
use crate::data::{Data, Type, Value};

pub(crate) fn call_named<'v>(stmt: &'v Value, data: &Data, name: &str) -> Option<&'v [Value]> {
    if let Value::Call(d, args) = stmt.unspan()
        && (*d as usize) < data.definitions.len()
        && data.def(*d).name() == name
    {
        Some(args)
    } else {
        None
    }
}

pub(crate) fn as_var(v: Option<&Value>) -> Option<u16> {
    match v.map(Value::unspan) {
        Some(Value::Var(w)) => Some(*w),
        _ => None,
    }
}

pub(crate) fn as_int(v: Option<&Value>) -> Option<i32> {
    match v.map(Value::unspan) {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    }
}

/// @PLN157 § V-o — is `d_nr` a stdlib ONE-OP wrapper: a loft function whose whole body is
/// one native op over its own parameters (`pub fn len(both: vector) -> integer {
/// OpLengthVector(both) }`, `sqrt` = `OpMathFuncFloat(9, both)`)?
///
/// Answers the op and the operand list to emit in the wrapper's place, each operand a
/// PARAMETER INDEX or a constant the body spells.  A body that does anything else — a
/// conversion (`exp` passes `E as single`), a second statement, a call to another loft
/// function, a return buffer — answers `None` and the call is emitted as a call.
/// Emitting the op instead of the call changes nothing the program can observe (the
/// wrapper's Rust body IS the op's template) and lets the registry's header-aware emitters
/// see the op: `len(d)` after a view binding or inside a hoisted loop reads the header's
/// length instead of resolving the store.
#[must_use]
/// Enforces `@FR-R-Wrapper`, the STDLIB qualifier included.
pub fn one_op_wrapper(data: &Data, d_nr: u32) -> Option<(u32, Vec<WrapperOperand>)> {
    if (d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let def = data.def(d_nr);
    if !def.rust().is_empty() || matches!(def.code(), Value::Null) {
        return None;
    }
    if def.hidden_return_buffer_attr().is_some() {
        return None;
    }
    // Only the STDLIB's wrappers.  A user function's CALL is itself observable: the live
    // tier may flip it to the interpreter (`live_flipped`), and its frame is on the shadow
    // call stack — emitting its op in place of the call skips both.  A stdlib wrapper has
    // neither (a `t_` method carries no frame and no live check), so for it and only for
    // it the op IS the call.  Measured: a one-op user function (`fn reader(w: W) -> integer
    // { w.a }`) flipped to the interpreter ran compiled, and the wasm live-dispatch probe
    // counted 0 dispatches where it expected 2.
    if def.source() != crate::data::STD_SOURCE {
        return None;
    }
    // A text parameter or result takes the CALL's conversions (a `String` result handed to
    // a `&str` parameter, a work buffer for the answer) that a template's operands do not
    // get, so `len(t)` and `print(t)` stay calls.
    if def
        .attributes()
        .iter()
        .any(|a| matches!(a.typedef.base(), Type::Text(_)))
        || matches!(def.returned().base(), Type::Text(_))
    {
        return None;
    }
    let params = def.attributes().len();
    // The body: a block of exactly one statement (line markers aside), which is the op
    // call, possibly under a `Return`.
    let mut stmt: Option<&Value> = None;
    match def.code().unspan() {
        Value::Block(b) => {
            for op in &b.operators {
                if matches!(op.unspan(), Value::Line(_)) {
                    continue;
                }
                if stmt.is_some() {
                    return None;
                }
                stmt = Some(op);
            }
        }
        other => stmt = Some(other),
    }
    let mut stmt = stmt?.unspan();
    if let Value::Return(inner) = stmt {
        stmt = inner.unspan();
    }
    let Value::Call(op, args) = stmt else {
        return None;
    };
    let callee = data.def(*op);
    if !matches!(callee.code(), Value::Null) || callee.rust().is_empty() {
        return None;
    }
    let mut operands = Vec::with_capacity(args.len());
    for a in args {
        match a.unspan() {
            Value::Var(v) if (*v as usize) < params => operands.push(WrapperOperand::Param(*v)),
            Value::Int(n) => operands.push(WrapperOperand::Const(Value::Int(*n))),
            _ => return None,
        }
    }
    Some((*op, operands))
}

/// One operand of a [`one_op_wrapper`]'s op: the caller's argument at that parameter
/// position, or a constant the wrapper body spells (a libm selector).
#[derive(Clone, Debug, PartialEq)]
pub enum WrapperOperand {
    Param(u16),
    Const(Value),
}

/// The buffer and the separator of a call that is the standard library's `split` with a
/// constant, non-null separator and a hidden return buffer — the call both
/// `(R-LazySplit)` and `(R-SplitTable)` replace — or why it is not one.
pub(crate) fn split_call(
    call: &Value,
    data: &Data,
    vars: &crate::variables::Function,
) -> Result<(u16, char), &'static str> {
    let Some([_src, sep, buf]) = call_named(call, data, "t_4text_split") else {
        return Err("");
    };
    let Value::Call(d, _) = call.unspan() else {
        return Err("");
    };
    if !crate::file_access::is_stdlib_source(data.def(*d).position.file.as_str()) {
        return Err("`split` is not the standard library's");
    }
    let Some(code) = call_named(sep, data, "OpConvCharacterFromInt")
        .and_then(|a| a.first())
        .and_then(|a| match a.unspan() {
            Value::Int(k) => u32::try_from(*k).ok(),
            _ => None,
        })
    else {
        return Err("the separator is not a character constant");
    };
    let Some(separator) = char::from_u32(code).filter(|c| *c != '\0') else {
        return Err("the separator is the null character");
    };
    let Some(buf) = as_var(Some(buf)) else {
        return Err("the call's buffer is not a variable");
    };
    if buf >= vars.count() || !vars.name(buf).starts_with("__ref") {
        return Err("the call's buffer is not a hidden return buffer");
    }
    Ok((buf, separator))
}

/// Does node `n` ITSELF name variable `v` — as a `Var`, or as one of the variants that
/// carry a variable number outside a `Var` node?  Exhaustive, so a variant added later is a
/// compile error here instead of a mention this pass cannot see.
pub(crate) fn names_var(n: &Value, v: u16) -> bool {
    match n {
        Value::Var(x)
        | Value::Set(x, _)
        | Value::CallRef(x, _)
        | Value::Iter(x, ..)
        | Value::TupleGet(x, _)
        | Value::TuplePut(x, ..)
        | Value::FnRef(_, x, _)
        | Value::FnRefDnr(x) => *x == v,
        Value::Null
        | Value::Line(_)
        | Value::Span(_)
        | Value::Int(_)
        | Value::Enum(..)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Call(..)
        | Value::Block(_)
        | Value::Insert(_)
        | Value::Return(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::If(..)
        | Value::Loop(_)
        | Value::Drop(_)
        | Value::Keys(_)
        | Value::Tuple(_)
        | Value::Yield(_)
        | Value::Parallel(_)
        | Value::RawExpr(_) => false,
    }
}
