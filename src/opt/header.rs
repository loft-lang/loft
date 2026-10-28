// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks

//! `@FR-R-Header`: which vectors a loop may read through a header — the record and the
//! length, and so the element base — derived once before it.  Sound when nothing the loop
//! runs writes a store but in place ([`blocks_header_hoist`]'s gate), so no record moves and
//! no length changes.  The Rust writer and the C writer both ask it.

use crate::data::{Block, Data, Type, Value};
use crate::database::Stores;
use crate::opt::body_writes::{is_element_address, rebound_vars};
use crate::opt::writes::{HoistOwned, HoistTiers, PathKey, blocks_header_hoist, vector_path};
use std::collections::{HashMap, HashSet};

/// The vector paths whose header `body` may derive once up front, each with a clone of
/// the operand expression the prelude evaluates.
///
/// Empty when anything in the loop could write a store — except, when `allow_in_place`
/// (the @PLN157 P4a tier, off under `LOFT_NO_WRITE_HOIST=1`), the [`IN_PLACE_SET_OPS`],
/// which cannot invalidate a header — when the body rebinds the path's ROOT variable, or
/// when nothing indexes a vector at all. Order is the order the accesses appear in, so
/// the generated prelude is stable across runs.
pub fn hoistable_vectors(
    body: &Block,
    data: &Data,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
) -> Vec<(PathKey, Value)> {
    let tiers = HoistTiers {
        in_place: allow_in_place,
        ..HoistTiers::default()
    };
    if body_blocks_hoist(body, data, None, def_nr, cache, tiers, None, true) {
        return Vec::new();
    }
    vector_candidates(body, data, def_nr)
}

/// For the decline trace: descend `op`'s statement lists to the innermost statement that
/// `blocks_header_hoist` refuses, and the `Line` marker before it.
fn innermost_blocking<'a>(
    op: &'a Value,
    data: &Data,
    stores: Option<&Stores>,
    cache: &mut HashMap<u32, bool>,
    vars: Option<&crate::variables::Function>,
    tiers: HoistTiers,
    owned: Option<&HoistOwned>,
) -> (u32, &'a Value) {
    let mut cur = op;
    let mut line = 0;
    loop {
        let ops = match cur.unspan() {
            Value::Block(bl) | Value::Loop(bl) => &bl.operators,
            Value::Insert(ls) => ls,
            // A statement with no list of its own (a `Set`, an `If`, a call): descend to the
            // first CHILD the admission declines — the culprit is inside it, and naming the
            // whole statement names nothing.
            other => {
                let mut next: Option<&'a Value> = None;
                other.for_each_child(&mut |c: &'a Value| {
                    if next.is_none()
                        && blocks_header_hoist(
                            c,
                            data,
                            stores,
                            cache,
                            &mut HashSet::new(),
                            vars,
                            tiers,
                            &mut HashSet::new(),
                            owned,
                        )
                    {
                        next = Some(c);
                    }
                });
                match next {
                    Some(o) => {
                        cur = o;
                        continue;
                    }
                    None => return (line, cur),
                }
            }
        };
        let mut next: Option<&'a Value> = None;
        let mut inner_line = line;
        for o in ops {
            if let Value::Line(n) = o.unspan() {
                inner_line = *n;
                continue;
            }
            if blocks_header_hoist(
                o,
                data,
                stores,
                cache,
                &mut HashSet::new(),
                vars,
                tiers,
                &mut HashSet::new(),
                owned,
            ) {
                next = Some(o);
                break;
            }
        }
        match next {
            Some(o) => {
                cur = o;
                line = inner_line;
            }
            None => return (line, cur),
        }
    }
}

/// Does anything in `body` invalidate a hoisted header?  The ONE gate both the vector
/// headers and the scalar hoist (@PLN157 P4c) stand behind: a scalar hoist is admitted only
/// in a loop whose store writes are all in place, so the two cannot disagree about which
/// loops qualify.
/// Enforces `@FR-R-InPlace` as the ONE gate `@FR-R-Header` and `@FR-R-Scalar` stand behind.
// The seven parameters are `blocks_header_hoist`'s context and whether this verdict is the
// final one (a verdict asked again under the text clause traces only its second answer).
#[allow(clippy::too_many_arguments)]
pub(crate) fn body_blocks_hoist(
    body: &Block,
    data: &Data,
    stores: Option<&Stores>,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    tiers: HoistTiers,
    owned: Option<&HoistOwned>,
    traced: bool,
) -> bool {
    let mut fresh: HashSet<u16> = HashSet::new();
    let trace = traced && std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok();
    body.operators.iter().any(|op| {
        let blocks = blocks_header_hoist(
            op,
            data,
            stores,
            cache,
            &mut HashSet::new(),
            Some(data.def(def_nr).variables()),
            tiers,
            &mut fresh,
            owned,
        );
        if blocks && trace {
            // The INNERMOST statement the admission declines, with the source line before
            // it — the one to read when a loop that should hoist does not.  A loop body is
            // one statement to this walk, and naming the body names nothing.
            let (line, culprit) = innermost_blocking(
                op,
                data,
                stores,
                cache,
                Some(data.def(def_nr).variables()),
                tiers,
                owned,
            );
            let shown = format!("{culprit:?}");
            eprintln!(
                "hoist: {} loop {} declined at line {line} by {}",
                data.def(def_nr).name(),
                body.scope,
                &shown[..shown.len().min(400)]
            );
        }
        blocks
    })
}

/// The vector paths `body` indexes, in the order they appear, minus those whose root the
/// body rebinds (a field path's header describes the record the ROOT named; repointing the
/// field itself is an `OpSetRef`, which is not in [`IN_PLACE_SET_OPS`] and blocks outright).
/// Enforces `@FR-R-Header`: which paths a loop derives a header for.
pub(crate) fn vector_candidates(body: &Block, data: &Data, def_nr: u32) -> Vec<(PathKey, Value)> {
    let vars = data.def(def_nr).variables();
    let rebound = rebound_vars(body, data, vars);
    let mut found = indexed_vectors(body, data, def_nr);
    found.retain(|(p, _)| !rebound.contains(&p.0));
    found
}

/// The vector paths `body` indexes through an element address, in the order they appear,
/// rebound roots included (`vector_candidates` filters those; `@FR-R-Header`'s rebind clause
/// asks about them itself).
pub(crate) fn indexed_vectors(body: &Block, data: &Data, def_nr: u32) -> Vec<(PathKey, Value)> {
    let vars = data.def(def_nr).variables();
    let mut found: Vec<(PathKey, Value)> = Vec::new();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && args.len() == 3
                && is_element_address(data, *d)
                && let Some(path) = vector_path(data, &args[0])
            {
                // A bare var must TYPE as a vector (an odd non-vector shape stays out); a
                // field path is shape-trusted — it is the first argument of an
                // element-address op, which takes a vector.
                let vector_typed =
                    !path.1.is_empty() || matches!(vars.tp(path.0).base(), Type::Vector(_, _));
                if vector_typed && !found.iter().any(|(p, _)| *p == path) {
                    found.push((path, args[0].clone()));
                }
            }
            false
        });
    }
    found
}
