// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-CharWalk` — the `for c in T` walks over a text variable, found once and asked by both
//! backends: the interpreter fuses a walk's step into one `OpTextWalkStep`, `--native` emits it
//! as a byte-scanning fast arm.

use crate::data::{Data, Type, Value};
use std::collections::BTreeMap;

/// Does any statement under `node` WRITE text variable `v`?  A write is a `Set`, a
/// by-reference hand-off (`OpCreateStack`), or `v` as the DESTINATION (first operand) of a
/// text-building op.  Anything this does not recognise as a write is a read of an
/// immutable text, which a borrow permits — the question `(R-LazySplit)`'s borrow and
/// `(R-CharWalk)`'s hoisted null test both ask.
#[must_use]
pub fn text_written(node: &Value, v: u16, data: &Data) -> bool {
    node.any_node(&mut |n| match n {
        Value::Set(w, _) => *w == v,
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            let first = matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == v);
            first
                && (name == "OpCreateStack"
                    || name.starts_with("OpAppend")
                    || name.starts_with("OpClear")
                    || name.starts_with("OpFormat"))
        }
        _ => false,
    })
}

/// `@FR-R-CharWalk` — one `for c in T` walk over a text VARIABLE: the loop variable, the
/// `#index` and `#next` the step keeps, the block's result temp (the read the loop variable
/// is bound from, which a fused step writes straight into the loop variable instead), the
/// text, the position of the walk's null test among the loop's statements, and whether that
/// test may be asked once before the loop.
#[derive(Clone, Debug)]
pub struct CharWalk {
    pub loop_var: u16,
    pub index: u16,
    pub next: u16,
    pub result: u16,
    pub src: u16,
    pub null_test: usize,
    pub hoist_null: bool,
}

/// `@FR-R-CharWalk` — this function's character walks, keyed by the loop's scope.
///
/// The shape is the parser's lowering of `for c in T`, matched whole: the loop's first
/// statement binds `c` to a `for text next` block of five statements (`index = next`; the
/// character at `next`; `next` advanced by its width; the zero-width guard; the character),
/// and its second is the null test `if !T { break }`.  T must be a text VARIABLE (a call
/// source is evaluated per iteration by the lowering itself, and stays as it is).  The
/// fast arm needs no condition; the null test moves out of the loop only where no
/// statement of the loop writes T ([`text_written`]).
#[must_use]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn char_walks(data: &Data, def_nr: u32) -> BTreeMap<u16, CharWalk> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let trace = std::env::var("LOFT_TRACE_CHAR_WALK").is_ok();
    let mut out = BTreeMap::new();
    // A generator declines whole, as `(R-LazySplit)` does: its loop variables live on the
    // state machine (`self.var_c`), which the bind below does not spell.
    if def.code().any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return out;
    }
    // The operands of `v` when it is a call of the op `name`; a slice of `v`, so the
    // answer lives as long as `v` does (a closure cannot say so, a fn can).
    fn is_op<'v>(v: &'v Value, data: &Data, name: &str) -> Option<&'v [Value]> {
        match v.unspan() {
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len() && data.def(*d).name() == name =>
            {
                Some(args.as_slice())
            }
            _ => None,
        }
    }
    let var_of = |v: &Value| -> Option<u16> {
        match v.unspan() {
            Value::Var(x) => Some(*x),
            _ => None,
        }
    };
    def.code().any_node(&mut |n| {
        let Value::Loop(lp) = n else {
            return false;
        };
        if lp.name != "For loop" || lp.operators.len() < 2 {
            return false;
        }
        let Value::Set(loop_var, it) = lp.operators[0].unspan() else {
            return false;
        };
        let Value::Block(it) = it.unspan() else {
            return false;
        };
        if it.name != "for text next" {
            return false;
        }
        let ops: Vec<&Value> = it
            .operators
            .iter()
            .filter(|o| !matches!(o, Value::Line(_)))
            .collect();
        let [s_index, s_read, s_step, s_guard, s_result] = ops[..] else {
            return false;
        };
        // index = next
        let Value::Set(index, next_v) = s_index.unspan() else {
            return false;
        };
        let Some(next) = var_of(next_v) else {
            return false;
        };
        // res = OpTextCharacterNullable(src, next)
        let Value::Set(res, read) = s_read.unspan() else {
            return false;
        };
        let Some([src_v, at_v]) = is_op(read, data, "OpTextCharacterNullable") else {
            return false;
        };
        let (Some(src), Some(at)) = (var_of(src_v), var_of(at_v)) else {
            return false;
        };
        if at != next || !matches!(vars.tp(src).base(), Type::Text(_)) {
            return false;
        }
        // next = next + OpLengthCharacter(res)
        let Value::Set(n2, stepped) = s_step.unspan() else {
            return false;
        };
        let Some([step_from, width]) = is_op(stepped, data, "OpAddInt") else {
            return false;
        };
        let Some([width_of]) = is_op(width, data, "OpLengthCharacter") else {
            return false;
        };
        if *n2 != next || var_of(step_from) != Some(next) || var_of(width_of) != Some(*res) {
            return false;
        }
        // if next <= index { next = index + 1 }
        let Value::If(guard, g_then, g_else) = s_guard.unspan() else {
            return false;
        };
        let Some([ga, gb]) = is_op(guard, data, "OpLeInt") else {
            return false;
        };
        if var_of(ga) != Some(next) || var_of(gb) != Some(*index) {
            return false;
        }
        let Value::Set(n3, bump) = g_then.unspan() else {
            return false;
        };
        let Some([ba, bb]) = is_op(bump, data, "OpAddInt") else {
            return false;
        };
        if *n3 != next
            || var_of(ba) != Some(*index)
            || !matches!(bb.unspan(), Value::Int(1))
            || !matches!(g_else.unspan(), Value::Null)
            || var_of(s_result) != Some(*res)
        {
            return false;
        }
        // the null test: if !T { break }
        let Value::If(test, _, t_else) = lp.operators[1].unspan() else {
            return false;
        };
        let Some([inner]) = is_op(test, data, "OpNot") else {
            return false;
        };
        let Some([tsrc]) = is_op(inner, data, "OpConvBoolFromText") else {
            return false;
        };
        if var_of(tsrc) != Some(src) || !matches!(t_else.unspan(), Value::Null) {
            return false;
        }
        let hoist_null = !lp.operators[1..].iter().any(|s| text_written(s, src, data));
        if trace {
            eprintln!(
                "[char-walk] {}: `{}` over `{}` — fast step; null test {}",
                def.name(),
                vars.name(*loop_var),
                vars.name(src),
                if hoist_null {
                    "asked once"
                } else {
                    "per iteration (the text is written in the loop)"
                }
            );
        }
        out.insert(
            lp.scope,
            CharWalk {
                loop_var: *loop_var,
                index: *index,
                next,
                result: *res,
                src,
                null_test: 1,
                hoist_null,
            },
        );
        false
    });
    out
}
