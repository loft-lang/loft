// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `(R-TableHandle)` — the keyed operations on one hash collection place share a table handle.
//!
//! The runtime's keyed operations re-derive their table from the record each time: the
//! collection slot, the claim, the header, the seed and the entry directory.  A `loft_hh`
//! holds that decoding with the place and the store's table generation it was resolved under,
//! and every handle operation checks it first and resolves it again when it went stale
//! (`loft_hh_ready`, `src/c/loft_hash.h`).  So a handle is never wrong, only stale, and the one
//! decision here — which operations share a handle — is about speed, never correctness: a place
//! named by two or more covered operations in the function, or by one inside a loop, takes a
//! handle; every other place keeps today's call.  `LOFT_NO_TABLE_HANDLE=1` emits today's calls
//! everywhere.  Design: `bench/portal/analysis/table-handle.md`.
use super::FnWriter;
use crate::c::{Cx, Keyed};
use crate::data::Value;
use crate::opt::writes::{PathKey, vector_path};
use std::collections::{HashMap, HashSet};

/// Is `tp` a hash whose one key is an 8-byte integer — the tables the handle's lookup,
/// removal and one-probe insert cover?
fn int_hash(cx: &Cx, tp: u16) -> bool {
    matches!(cx.keyed(tp), Some((Keyed::Hash, _)))
        && cx
            .stores
            .types
            .get(tp as usize)
            .is_some_and(|t| t.keys.len() == 1 && matches!(t.keys[0].type_nr, 1 | 2))
}

fn literal(v: &Value) -> Option<i64> {
    match v.unspan() {
        Value::Int(i) => Some(i64::from(*i)),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

/// The collection place a covered keyed operation names, `None` for any other call.  Every
/// shape a handle form is emitted for is listed here; a call this misses only keeps today's
/// form (`R-TableHandle` admits a place, it never requires one).
fn keyed_place(cx: &Cx, op: u32, args: &[Value]) -> Option<PathKey> {
    let data = cx.data;
    match (data.def(op).name(), args) {
        ("OpGetRecord", [coll, tp, ..]) => {
            let tp = u16::try_from(literal(tp)?).ok()?;
            int_hash(cx, tp).then(|| vector_path(data, coll)).flatten()
        }
        ("OpLengthHash" | "t_4hash_len", [coll]) => vector_path(data, coll),
        ("OpHashRemove", [coll, _, tp]) => {
            let tp = u16::try_from(literal(tp)?).ok()? & !crate::database::CLEAR_KEYED_VIEW;
            int_hash(cx, tp).then(|| vector_path(data, coll)).flatten()
        }
        ("OpFinishRecord", [owner, _, ptp, fld]) => {
            let ptp = u16::try_from(literal(ptp)?).ok()?;
            let fld = u16::try_from(literal(fld)?).ok()?;
            finish_place(cx, owner, ptp, fld)
        }
        _ => None,
    }
}

/// The hash place `OpFinishRecord(owner, _, ptp, fld)` files into: `owner` itself when `fld`
/// is the bare clause, else `owner`'s field `fld` at its byte position (the offset an
/// `OpGetField` of the same field carries, so the two spellings share one key).
pub(super) fn finish_place(cx: &Cx, owner: &Value, ptp: u16, fld: u16) -> Option<PathKey> {
    if fld == u16::MAX {
        return int_hash(cx, ptp)
            .then(|| vector_path(cx.data, owner))
            .flatten();
    }
    if !matches!(
        cx.stores.types.get(ptp as usize).map(|t| &t.parts),
        Some(crate::database::Parts::Struct(_) | crate::database::Parts::EnumValue(_, _))
    ) {
        return None;
    }
    if !int_hash(cx, cx.stores.field_type(ptp, fld)) {
        return None;
    }
    let (root, mut offs) = vector_path(cx.data, owner)?;
    offs.push(i64::from(cx.stores.field_position(ptp, fld)));
    Some((root, offs))
}

/// Counts the covered keyed operations per place under `v`; `in_loop` is whether `v` sits in
/// a loop body.
fn count(cx: &Cx, v: &Value, in_loop: bool, seen: &mut HashMap<PathKey, (u32, bool)>) {
    let v = v.unspan();
    let in_loop = in_loop || matches!(v, Value::Loop(_));
    if let Value::Call(op, args) = v
        && let Some(path) = keyed_place(cx, *op, args)
    {
        let e = seen.entry(path).or_insert((0, false));
        e.0 += 1;
        e.1 |= in_loop;
    }
    v.for_each_child(&mut |c| count(cx, c, in_loop, seen));
}

/// The places of function body `code` that take a table handle: named by two or more covered
/// keyed operations, or by one inside a loop.
pub(super) fn places(cx: &Cx, code: &Value) -> HashSet<PathKey> {
    let mut seen = HashMap::new();
    count(cx, code, false, &mut seen);
    seen.into_iter()
        .filter(|(_, (n, looped))| *n >= 2 || *looped)
        .map(|(p, _)| p)
        .collect()
}

impl FnWriter<'_> {
    /// The handle the keyed operations on collection `coll` share, declared at its first use;
    /// `None` keeps today's call (a place `places` did not admit, or the switch).
    pub(super) fn table_handle(&mut self, coll: &Value) -> Option<String> {
        let path = vector_path(self.cx.data, coll)?;
        self.table_handle_at(path)
    }

    /// `table_handle` for a place already resolved to its path.
    pub(super) fn table_handle_at(&mut self, path: PathKey) -> Option<String> {
        if self.cx.switches.table_handle_disabled || !self.table_places.contains(&path) {
            return None;
        }
        if let Some(h) = self.table_names.get(&path) {
            return Some(h.clone());
        }
        let h = format!("hh{}", self.decls.len() + 1);
        self.decls.push(format!("loft_hh {h} = LOFT_HH_NONE;"));
        crate::rewrite_census::fired("R-TableHandle", 1);
        self.table_names.insert(path, h.clone());
        Some(h)
    }
}
