// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-TextBorrow` — the text walks whose loop variable may BORROW the element's text
//! instead of copying it: no statement in the walk writes the store it lives in, and the
//! variable does not escape the iteration.

use crate::data::{Data, Type, Value};
use crate::ir_shape::{as_var, call_named};
use crate::opt::iteration::iteration_step;
use crate::opt::writes::may_write_store;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

/// `@FR-R-TextBorrow` — the head of a walk of TEXTS, as the parser lowers
/// `for p in words` over a `vector<text>`: the loop variable, its `#index`, the vector
/// variable and the element read, `OpGetText(OpGetVectorNullable(vec, size, index), 0)`.  Shape only,
/// like [`iteration_head`](crate::opt::iteration::iteration_head); a record walk, a reversed walk or a text iterator answers
/// `None`.  The vector operand must be a VARIABLE: the hidden `_vector_N` the parser binds
/// before the loop, which is what a lazy split is keyed by too.
#[must_use]
pub fn text_walk_head<'a>(stmt: &'a Value, data: &Data) -> Option<(u16, u16, u16, &'a Value)> {
    let (loop_var, index, read) = iteration_step(stmt, data)?;
    let Value::Call(td, targs) = read.unspan() else {
        return None;
    };
    if (*td as usize) >= data.definitions.len()
        || data.def(*td).name() != "OpGetText"
        || targs.len() != 2
        || !matches!(targs[1].unspan(), Value::Int(0))
    {
        return None;
    }
    let Value::Call(rd, rargs) = targs[0].unspan() else {
        return None;
    };
    if (*rd as usize) >= data.definitions.len()
        || data.def(*rd).name() != "OpGetVectorNullable"
        || rargs.len() != 3
        || !matches!(rargs[2].unspan(), Value::Var(c) if *c == index)
    {
        return None;
    }
    let Value::Var(vec) = rargs[0].unspan() else {
        return None;
    };
    Some((loop_var, index, *vec, read))
}

/// `@FR-R-TextBorrow` — one admitted walk's loop variable: the element read the walk
/// binds it to, kept for the checking form (`LOFT_HOIST_VERIFY=1` re-reads it at the
/// walk's release), or `None` for a lazy split's piece, which borrows no store and has
/// nothing to re-derive.
#[derive(Clone, Debug)]
pub struct TextBorrow {
    pub read: Option<Value>,
}

/// `stmt` as the emitter writes it under the split tables `tables` (`@FR-R-SplitTable`):
/// each table's bind, and the mint and frees of the buffer the table leaves dead, as
/// `Null`.  The fallback keeps every other node as it stands, so a walk over the result
/// sees exactly the statements that still run; a statement naming no table is returned
/// as a plain clone.
pub(crate) fn as_emitted_by_tables(
    stmt: &Value,
    tables: &BTreeMap<u16, SplitTable>,
    data: &Data,
) -> Value {
    let mut out = stmt.clone();
    if tables.is_empty() {
        return out;
    }
    let dead: HashSet<u16> = tables.values().filter_map(|t| t.dead_buf).collect();
    out.map_nodes(&mut |n| {
        let lowered = match n {
            Value::Set(v, _) => tables.contains_key(v),
            Value::Call(d, args) => {
                (*d as usize) < data.definitions.len()
                    && matches!(
                        data.def(*d).name(),
                        "OpDatabase" | "OpDatabaseNP" | "OpFreeRef" | "OpFreeRefIfDistinct"
                    )
                    && matches!(args.first().map(Value::unspan),
                        Some(Value::Var(b)) if dead.contains(b))
            }
            _ => false,
        };
        if lowered {
            *n = Value::Null;
        }
    });
    out
}

/// `@FR-R-TextBorrow` — the loop variables of this function's walks of texts that BORROW
/// their element: `for p in words` where the loop's rest — the parser's two bound tests,
/// the body and the walk's release — reads `p` only as a text value, and (for a vector the
/// loop really walks) writes no store.  A walk over a SLICED vector (`sliced`: a lazy
/// split's hidden vector, or a split table's walk alias) needs no store condition: its
/// pieces borrow the split's source, a borrowed parameter or a block-long copy, never a
/// store.  A generator declines whole, as `(R-LazySplit)` does: its loops are re-entered
/// across `next`, and a borrow cannot cross the state machine.
///
/// A mention of `p` is a text VALUE where it is an operand of a call at a `text` position
/// (an op or a user function; a text-writer's `pos: const u16` destination is not one, so
/// `p += "x"` declines here), the whole source of a bind or a tuple write into ANOTHER
/// slot (the one site converts it, `Output::text_borrowed`), or the operand of the walk's
/// own `OpFreeText`.  Every other mention — a rebind, a `&p` link, a `return p`, a tuple
/// literal, a capture — declines: the failure a missed shape would produce is rustc
/// refusing the program, but a decline costs a copy and refuses nothing.
///
/// The store condition reads the body AS EMITTED: a split bind `(R-SplitTable)` lowered
/// (`tables`) is a table of slices over a copy of its source, and its dead buffer is never
/// minted or freed — none of them writes a store ([`as_emitted_by_tables`]).
#[must_use]
pub fn borrowed_text_walks(
    data: &Data,
    def_nr: u32,
    sliced: &BTreeSet<u16>,
    tables: &BTreeMap<u16, SplitTable>,
) -> HashMap<u16, TextBorrow> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let trace = std::env::var("LOFT_TRACE_TEXT_BORROW").is_ok();
    let mut out = HashMap::new();
    if body.any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return out;
    }
    let mut cache: HashMap<u32, bool> = HashMap::new();
    body.any_node(&mut |n| {
        let Value::Loop(lp) = n else {
            return false;
        };
        if lp.name != "For loop" || lp.operators.is_empty() {
            return false;
        }
        let Some((p, _index, vec, read)) = text_walk_head(&lp.operators[0], data) else {
            return false;
        };
        let rest = &lp.operators[1..];
        let verdict = if !matches!(vars.tp(p).base(), Type::Text(_)) {
            Err("the loop variable is not a text")
        } else if !sliced.contains(&vec)
            && rest
                .iter()
                .any(|s| may_write_store(&as_emitted_by_tables(s, tables, data), data, &mut cache))
        {
            Err("the body may write a store")
        } else if let Some(why) = rest.iter().find_map(|s| text_escapes(s, p, data)) {
            Err(why)
        } else {
            Ok(())
        };
        match verdict {
            Ok(()) => {
                // A sliced vector's piece has no store to re-read: nothing to verify.
                let read = (!sliced.contains(&vec)).then(|| read.clone());
                out.insert(p, TextBorrow { read });
                if trace {
                    eprintln!(
                        "[text-borrow] {}: `{}` borrows its element",
                        def.name(),
                        vars.name(p)
                    );
                }
            }
            Err(why) if trace => {
                eprintln!(
                    "[text-borrow] {}: `{}` declined — {why}",
                    def.name(),
                    vars.name(p)
                );
            }
            Err(_) => {}
        }
        false
    });
    out
}

/// The first mention of text variable `p` under `node` that is NOT a text-value read, as
/// [`borrowed_text_walks`] defines one — or `None` when every mention is.  Walks the tree
/// by hand rather than through `any_node` because the question is about a mention's
/// PARENT: the same `Var(p)` is a value read as a call's `text` operand and an escape as a
/// link's.
pub(crate) fn text_escapes(node: &Value, p: u16, data: &Data) -> Option<&'static str> {
    text_escapes_with(node, p, data, &|_| false)
}

/// [`text_escapes`] with the ONE bind of `p` the caller admits: a `Set(p, to)` for which
/// `bind_ok(to)` holds is the borrow's own binding, not a rebind.  A walk's loop variable
/// is bound in the loop head the walk never hands here; a discharge temp
/// ([`borrowed_discharge_temps`]) is bound by a statement inside the body it reads.
pub(crate) fn text_escapes_with(
    node: &Value,
    p: u16,
    data: &Data,
    bind_ok: &dyn Fn(&Value) -> bool,
) -> Option<&'static str> {
    let text_escapes = |n: &Value, p: u16, data: &Data| text_escapes_with(n, p, data, bind_ok);
    match node {
        Value::Var(v) if *v == p => Some("read outside a text-value position"),
        Value::Set(v, to) => {
            if *v == p {
                if bind_ok(to) {
                    return text_escapes(to, p, data);
                }
                return Some("rebound in the body");
            }
            // The whole source of a bind into another slot is converted at the bind.
            if matches!(to.unspan(), Value::Var(s) if *s == p) {
                return None;
            }
            text_escapes(to, p, data)
        }
        Value::TuplePut(t, _, to) => {
            if matches!(to.unspan(), Value::Var(s) if *s == p) {
                return None;
            }
            if *t == p {
                return Some("written as a tuple");
            }
            text_escapes(to, p, data)
        }
        Value::Call(d, args) => {
            let known = (*d as usize) < data.definitions.len();
            let name = if known { data.def(*d).name() } else { "" };
            let attrs = if known {
                data.def(*d).attributes()
            } else {
                &[]
            };
            for (i, a) in args.iter().enumerate() {
                if matches!(a.unspan(), Value::Var(s) if *s == p) {
                    // `OpGetRecord`'s keys trail its declared parameters, and each is read
                    // into a `Content` at the call: a text VALUE.
                    let value_position = name == "OpFreeText"
                        || (matches!(name, "OpGetRecord" | "OpSetKeyed") && i >= attrs.len())
                        || attrs.get(i).is_some_and(|at| {
                            !at.constant && matches!(at.typedef.base(), Type::Text(_))
                        });
                    if !value_position {
                        return Some("handed to a call outside a text-value position");
                    }
                } else if let Some(why) = text_escapes(a, p, data) {
                    return Some(why);
                }
            }
            None
        }
        Value::Span(inner) => text_escapes(&inner.1, p, data),
        Value::Block(b) => b.operators.iter().find_map(|o| text_escapes(o, p, data)),
        Value::Loop(b) => b.operators.iter().find_map(|o| text_escapes(o, p, data)),
        Value::Insert(ops) => ops.iter().find_map(|o| text_escapes(o, p, data)),
        // The `?` discharge's own value arm — `if OpConvBoolFromText(p) { p } else { d }`
        // — reads `p` as the value its parent consumes within the statement.
        Value::If(cond, then, otherwise) => {
            let discharge = matches!(then.unspan(), Value::Var(s) if *s == p)
                && call_named(cond, data, "OpConvBoolFromText")
                    .and_then(|a| as_var(a.first()))
                    .is_some_and(|s| s == p);
            text_escapes(cond, p, data)
                .or_else(|| {
                    if discharge {
                        None
                    } else {
                        text_escapes(then, p, data)
                    }
                })
                .or_else(|| text_escapes(otherwise, p, data))
        }
        Value::Drop(inner) => text_escapes(inner, p, data),
        Value::Return(inner) => {
            if matches!(inner.unspan(), Value::Var(s) if *s == p) {
                return Some("returned");
            }
            text_escapes(inner, p, data)
        }
        // A fn-ref call, a parallel body, a yield, a keyed cursor, a tuple literal, a
        // link: any mention of `p` under these is an escape, and a subtree without one
        // is not — so the walk continues through them by the generic mention test.
        _ => {
            if node.any_node(&mut |n| matches!(n, Value::Var(v) if *v == p)) {
                Some("mentioned under a shape the borrow does not read")
            } else {
                None
            }
        }
    }
}

/// `@FR-R-SplitTable` — one `V = T.split(c)` bound to a local whose every other mention
/// the TABLE of its pieces answers: the vector is never built.
#[derive(Clone, Debug)]
pub struct SplitTable {
    /// The hidden `__ref` buffer the call would have filled, when it serves this call
    /// alone: it is then never minted.  A buffer another call shares keeps its mint.
    pub dead_buf: Option<u16>,
    /// The separator: a character constant that is not the null character.
    pub separator: char,
    /// The hidden `_vector_N` locals the parser binds from V for a `for p in V` walk —
    /// aliases of the table, read by the walk's element read and length test alone.
    pub aliases: Vec<u16>,
}
