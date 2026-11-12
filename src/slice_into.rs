// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-SliceInto` in the IR phase: a scalar slice used as a value and appended to a place —
//! `OpAppendVector(P, <slice block>, tp)`, a literal's field `R { f: v[a..b] }` or
//! `x.f += v[a..b]` — is the slice's bound statements and one `OpSliceVector(P, src, lo, hi,
//! row)`, which appends `[lo, hi)` clamped onto P as one block.  The form the parser already
//! writes for a LOCAL place (`p += v[a..b]`), here for every place, so no backend fills a
//! temporary store to append it.
//!
//! The slice block is the one [`crate::slice_text::admit`] reads (its mint, bind, zero, bounds
//! and one slice; the temporary its value), and outside the block the temporary is never named
//! and its store only declared null and freed ([`crate::slice_text::outside_mentions`]): then
//! that store is never minted, and its frees pass a null by.  The append was the temporary's
//! only reader, and `OpSliceVector` copies exactly the span the temporary's fill copied and the
//! append copied again, read before anything is written — so it is safe when P is the source
//! (`c.f += c.f[a..b]`: the span lies below the length).  A foreign source is copied, as the
//! append copied the view's bytes: P's store holds more than this vector.
//!
//! The bound statements move in front of P's evaluation, so P must be a PLACE that no statement
//! can change: variables and constant field offsets (`OpGetField`), none of them assigned by a
//! bound statement.  The append's element type must be the slice's.  Anything else keeps the
//! append.  Both backends run the rewritten IR (`@FR-R-Phase`).
//!
//! `LOFT_NO_SLICE_INTO=1` keeps every append; `LOFT_TRACE_SLICE_INTO=1` names each site taken
//! and each declined with why.
use crate::data::{Data, DefType, Value, v_block};
use crate::slice_text::{BlockOps, admit, outside_mentions, slice_block, split};

const RULE: &str = "R-SliceInto";

fn disabled() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_SLICE_INTO").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_SLICE_INTO").is_ok_and(|v| v != "0"))
}

/// Rewrite every admitted append of a slice value in function `d_nr`.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if disabled() || data.def_type(d_nr) != DefType::Function || !data.def(d_nr).is_loft_defined() {
        return;
    }
    let append = data.def_nr("OpAppendVector");
    let into = data.def_nr("OpSliceVector");
    let get_field = data.def_nr("OpGetField");
    if append == u32::MAX || into == u32::MAX {
        return;
    }
    let ops = BlockOps::new(data);
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    let mut tmps: Vec<u16> = Vec::new();
    {
        let vars = data.def(d_nr).variables();
        code.walk(&mut |n| {
            let Some((place, block, tp)) = host(n, append) else {
                return;
            };
            match admit(block, vars, &ops).and_then(|site| {
                let (prelude, [_, _, _, row]) = split(block, &site, &ops);
                fits(place, &prelude, tp, &row, get_field).map(|()| site)
            }) {
                Ok(site) => match outside_mentions(&code, &site, &ops) {
                    Ok(()) => tmps.push(site.tmp),
                    Err(why) => decline(data, d_nr, vars.name(site.tmp), why),
                },
                Err(why) => decline(data, d_nr, "slice", why),
            }
        });
    }
    let mut fired = 0usize;
    if !tmps.is_empty() {
        let vars = data.def(d_nr).variables();
        code.map_nodes(&mut |n| {
            // The bare statement, not a span around it: `map_nodes` reaches the node inside a
            // span too, and rewriting that one keeps the statement's source line.
            let Some((place, block, _)) = host(n, append) else {
                return;
            };
            let Ok(site) = admit(block, vars, &ops) else {
                return;
            };
            if !tmps.contains(&site.tmp) {
                return;
            }
            let (mut stmts, [src, lo, hi, row]) = split(block, &site, &ops);
            stmts.push(Value::Call(into, vec![place.clone(), src, lo, hi, row]));
            // The bounds' variables belong to the slice block's scope: the statement block that
            // replaces the append takes it over.
            let scope = block.scope;
            *n = v_block(stmts, crate::data::Type::Void, "slice into");
            if let Value::Block(bl) = n {
                bl.scope = scope;
            }
            fired += 1;
            if trace() {
                eprintln!(
                    "[slice-into] fn={} {} is sliced into its place",
                    data.def(d_nr).name(),
                    vars.name(site.tmp)
                );
            }
        });
    }
    data.definitions[d_nr as usize].code = code;
    crate::rewrite_census::fired(RULE, fired);
}

/// `OpAppendVector(place, <block>, tp)`: the place, the slice block and the element type.
fn host(n: &Value, append: u32) -> Option<(&Value, &crate::data::Block, &Value)> {
    match n {
        Value::Call(f, args) if *f == append => match args.as_slice() {
            [place, other, tp] => slice_block(other).map(|bl| (place, bl, tp)),
            _ => None,
        },
        _ => None,
    }
}

/// The append's place reads only variables and constant field offsets, and no bound statement
/// assigns one of those variables — so evaluating it after the bounds finds what it found
/// before them; and the append's element type is the slice's.
fn fits(
    place: &Value,
    prelude: &[Value],
    tp: &Value,
    row: &Value,
    get_field: u32,
) -> Result<(), &'static str> {
    if !matches!((tp.unspan(), row.unspan()), (Value::Int(a), Value::Int(b)) if a == b) {
        return Err("the append's element type is not the slice's");
    }
    let mut read: Vec<u16> = Vec::new();
    if !plain_place(place, get_field, &mut read) {
        return Err("the place is not variables and field offsets");
    }
    let mut assigned = false;
    for op in prelude {
        op.walk(&mut |n| {
            if matches!(n, Value::Set(x, _) if read.contains(x)) {
                assigned = true;
            }
        });
    }
    if assigned {
        return Err("a bound statement assigns the place");
    }
    Ok(())
}

/// `v` is a variable, or `OpGetField` of a plain place at constant operands; `read` collects the
/// variables.  Anything else — a call, an index, a block — may evaluate differently after the
/// bound statements, so it is not a plain place.
fn plain_place(v: &Value, get_field: u32, read: &mut Vec<u16>) -> bool {
    match v.unspan() {
        Value::Var(x) => {
            read.push(*x);
            true
        }
        Value::Call(f, args) if *f == get_field => match args.split_first() {
            Some((base, rest)) => {
                rest.iter().all(|a| matches!(a.unspan(), Value::Int(_)))
                    && plain_place(base, get_field, read)
            }
            None => false,
        },
        _ => false,
    }
}

fn decline(data: &Data, d_nr: u32, site: &str, why: &str) {
    crate::rewrite_census::declined(RULE, site, why);
    if trace() {
        eprintln!(
            "[slice-into] fn={} keeps its append: {why}",
            data.def(d_nr).name()
        );
    }
}
