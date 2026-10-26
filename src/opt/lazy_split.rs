// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-LazySplit` — the `for piece in text.split(c)` loops that take their pieces straight
//! from the text.  Here, in `opt`, because both compiled writers ask it (the Rust writer and
//! the C backend); moved from `generation::hoist`, which re-exports it.

use crate::data::{Block, Data, Value};
use crate::ir_shape::{as_var, call_named, split_call};
use std::collections::BTreeMap;

/// `@FR-R-LazySplit` — one `for piece in text.split(c)` loop that takes its pieces straight
/// from the text instead of reading them back out of a `vector<text>`.
#[derive(Clone, Debug)]
pub struct LazySplit {
    /// The hidden `__ref` buffer the call would have filled, when it serves this call
    /// alone: it is then never minted.  A buffer another call shares keeps its mint.
    pub dead_buf: Option<u16>,
    /// The separator: a character constant that is not the null character.
    pub separator: char,
}

/// The lazy split loops of `def_nr`'s body, keyed by the hidden `_vector_N` variable the
/// `For` block binds the call's result to (`@FR-R-LazySplit`).
///
/// `for piece in text.split(c)` lowers to a call that builds a `vector<text>` and a loop
/// that reads it back one element at a time.  Nothing but that loop can name the vector,
/// so the loop takes each piece from the text itself: the same pieces in the same order,
/// with no vector and no record per piece.  Every gate is an under-approximation — a
/// declined loop keeps the vector, which is always correct:
///
/// - the call is the STANDARD LIBRARY's `split(self: text, separator: character)`, whose
///   pieces `codegen_runtime::lazy_split` reproduces;
/// - the separator is a character CONSTANT other than the null character — a null
///   separator compares equal to a NUL inside the text, which the iterator does not model;
/// - the `For` block and its loop have exactly the shape the parser gives a forward walk of
///   a text vector, and the vector is mentioned by its element read and its length test and
///   NOWHERE else — those two expressions are what the emitter replaces;
/// - the function is not a generator: its loops are re-entered across a `next` call, and
///   the iterator is a local of the activation, not of the state machine.
///
/// The source text needs no gate: the emitter borrows it where nothing can write it and
/// iterates a copy of it everywhere else (`Output::lazy_split_source`).
#[must_use]
pub fn lazy_splits(data: &Data, def_nr: u32) -> BTreeMap<u16, LazySplit> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let trace = std::env::var("LOFT_TRACE_LAZY_SPLIT").is_ok();
    let mut out: BTreeMap<u16, LazySplit> = BTreeMap::new();
    let mut bufs: BTreeMap<u16, u16> = BTreeMap::new();
    if body.any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return out;
    }
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && bl.name == "For block"
        {
            match lazy_split_block(bl, data, vars) {
                Ok((vec, buf, separator)) => {
                    bufs.insert(vec, buf);
                    out.insert(
                        vec,
                        LazySplit {
                            dead_buf: None,
                            separator,
                        },
                    );
                }
                Err(why) if trace && !why.is_empty() => {
                    eprintln!("[lazy-split] {}: declined — {why}", def.name());
                }
                Err(_) => {}
            }
        }
        false
    });
    // The vector is the emitter's to replace only when its two readers are its ONLY
    // mentions and the call its only value: a third mention would read a vector the lazy
    // form never fills.  Its `= null` declaration — hoisted to the function's top when
    // another text depends on it — binds nothing.
    out.retain(|vec, _| {
        let mut mentions = 0u32;
        let mut binds = 0u32;
        body.any_node(&mut |n| {
            match n {
                Value::Var(v) if v == vec => mentions += 1,
                Value::Set(v, to) if v == vec && !matches!(to.unspan(), Value::Null) => {
                    binds += 1;
                }
                _ => {}
            }
            false
        });
        let ok = mentions == 2 && binds == 1;
        if !ok && trace {
            eprintln!(
                "[lazy-split] {}: declined — {} is mentioned {mentions} times and bound {binds}",
                def.name(),
                vars.name(*vec)
            );
        }
        ok
    });
    // A buffer that serves this call alone is never minted.  Its every mention must be
    // accounted for — the one call argument, its frees, its null declaration, and the null
    // test that guards a mint at first use (`@FR-O-LazyBuffer`) — as `move_appends`
    // reconciles a placed buffer; a buffer with any other use keeps its mint, and the loop
    // is lazy all the same.
    for (vec, ls) in &mut out {
        let buf = bufs[vec];
        if buffer_serves_one_call(body, data, buf) {
            ls.dead_buf = Some(buf);
        }
        if trace {
            eprintln!(
                "[lazy-split] {}: {} is lazy, separator {:?}, buffer {} {}",
                def.name(),
                vars.name(*vec),
                ls.separator,
                vars.name(buf),
                if ls.dead_buf.is_some() {
                    "never minted"
                } else {
                    "shared, keeps its mint"
                }
            );
        }
    }
    out
}

/// The vector, the buffer and the separator of ONE `For` block that walks a `split`, or
/// the reason it is not one — empty for a `For` over anything else, which is no decline.
fn lazy_split_block(
    bl: &Block,
    data: &Data,
    vars: &crate::variables::Function,
) -> Result<(u16, u16, char), &'static str> {
    // ops = [.., Set(_vector, t_4text_split(src, sep, buf)), Set(idx, -1), Loop(..)]
    // The bind, the index seed and the loop are consecutive and close the block.  What may
    // stand before them — a `#count` seed, the buffer's mint at first use — is left alone:
    // none of it can name the vector, which the mention count in `lazy_splits` holds it to.
    let Some(at) = bl.operators.iter().position(|op| {
        matches!(op.unspan(), Value::Set(_, call)
            if call_named(call, data, "t_4text_split").is_some())
    }) else {
        return Err("");
    };
    let [bind, seed, walk] = &bl.operators[at..] else {
        return Err("the bind, the index seed and the loop do not close the block");
    };
    let Value::Set(vec, call) = bind.unspan() else {
        return Err("");
    };
    let (buf, separator) = split_call(call, data, vars)?;
    let Value::Set(idx, start) = seed.unspan() else {
        return Err("the block does not seed an index");
    };
    if !matches!(start.unspan(), Value::Int(-1)) {
        return Err("the index does not start before the first element");
    }
    let Value::Loop(lp) = walk.unspan() else {
        return Err("the block's third statement is not its loop");
    };
    // loop[0] = Set(piece, iter next { idx += 1; OpGetText(OpGetVectorNullable(vec, 4, idx), 0) })
    let Some(Value::Set(_, next)) = lp.operators.first().map(Value::unspan) else {
        return Err("the loop does not open by binding its variable");
    };
    let Value::Block(next) = next.unspan() else {
        return Err("the loop variable is not bound from an iterator step");
    };
    let reads_vec = next.name.contains("iter next")
        && next.operators.len() == 2
        && call_named(&next.operators[1], data, "OpGetText")
            .and_then(|a| a.first())
            .and_then(|elem| call_named(elem, data, "OpGetVectorNullable"))
            .is_some_and(|a| as_var(a.first()) == Some(*vec) && as_var(a.get(2)) == Some(*idx));
    if !reads_vec {
        return Err("the iterator step is not a plain element read");
    }
    // loop[1] = If(OpLeInt(OpLengthVector(vec), idx), break, _)
    let ends_on_len = matches!(lp.operators.get(1).map(Value::unspan), Some(Value::If(test, _, _))
    if call_named(test, data, "OpLeInt").is_some_and(|a| {
        as_var(a.get(1)) == Some(*idx)
            && a.first()
                .and_then(|len| call_named(len, data, "OpLengthVector"))
                .is_some_and(|l| as_var(l.first()) == Some(*vec))
    }));
    if !ends_on_len {
        return Err("the loop does not end on the vector's length");
    }
    Ok((*vec, buf, separator))
}

/// Does the hidden buffer `buf` serve ONE call and nothing else, so that a rewrite which
/// drops that call may leave the buffer unminted?  Its every mention must be accounted
/// for — the one call argument, its frees, its null declaration, and the null test that
/// guards a mint at first use (`@FR-O-LazyBuffer`) — as `move_appends` reconciles a placed
/// buffer.  A buffer with any other use keeps its mint, and the rewrite stands all the same.
pub(crate) fn buffer_serves_one_call(body: &Value, data: &Data, buf: u16) -> bool {
    let mut total = 0u32;
    let mut call_args = 0u32;
    let mut benign = 0u32;
    let mut rebound = false;
    body.any_node(&mut |n| {
        match n {
            Value::Var(v) if *v == buf => total += 1,
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let hits = args
                    .iter()
                    .filter(|a| matches!(a.unspan(), Value::Var(v) if *v == buf))
                    .count() as u32;
                let name = data.def(*d).name();
                if name == "OpFreeRef" || name == "OpFreeRefTag" || name == "OpRefIsNull" {
                    benign += hits;
                } else {
                    call_args += hits;
                }
            }
            Value::Set(v, to) if *v == buf && !matches!(to.unspan(), Value::Null) => {
                rebound = true;
            }
            _ => {}
        }
        false
    });
    call_args == 1 && total == call_args + benign && !rebound
}
