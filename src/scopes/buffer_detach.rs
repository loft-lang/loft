// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! A literal buffer whose store a local OWNER takes over is detached from it.

use crate::data::{Data, DefType, Value};
use crate::fxhash::FxHashSet as HashSet;
use crate::variables::Function;

/// `@FR-O-Owner` — one store, one owner.  A collection literal or copy is built in a
/// `__vdb_N` buffer and handed to its local through the buffer's field
/// (`r = OpGetField(__vdb_N, 0)`).  Normally that local is a VIEW of the buffer (its deps name
/// it) and the buffer releases the store.  A local that is an OWNER — bound at two sites, as
/// a value branch whose other arm mints (`r = if c { m(0) } else { cp }`) — releases the store
/// itself.  The buffer then still named it: the next pass's mint cleared a store already freed,
/// or by then handed to someone else, and the buffer's own exit free released it a second time.
/// So once the owner has taken the store, the buffer is set to the null sentinel after its last
/// use in that block, and its next mint is a fresh store (`@FR-O-Detach`: after the reads).
pub(super) fn detach_owned_buffers(data: &mut Data) {
    let sentinel = data.def_nr("OpNullRefSentinel");
    if sentinel == u32::MAX {
        return;
    }
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let vars = &data.def(d_nr).variables;
        // The locals the scope pass releases on their own: only such a binder takes the store.
        let mut freed: HashSet<u16> = HashSet::default();
        data.def(d_nr).code.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && matches!(data.def(*d).name(), "OpFreeRef" | "OpFreeRefIfDistinct")
                && let Some(Value::Var(w)) = args.first().map(Value::unspan)
            {
                freed.insert(*w);
            }
            false
        });
        let mut owned_buffers: HashSet<u16> = HashSet::default();
        data.def(d_nr).code.any_node(&mut |n| {
            if let Some((w, b)) = owner_takes_buffer(n, vars, data)
                && freed.contains(&w)
            {
                owned_buffers.insert(b);
            }
            false
        });
        if owned_buffers.is_empty() {
            continue;
        }
        let mut code = data.def(d_nr).code.clone();
        let vars = &data.def(d_nr).variables;
        code.map_nodes(&mut |n| {
            let Value::Block(bl) = n else {
                return;
            };
            let ops = &mut bl.operators;
            let mut at = 0;
            while at < ops.len() {
                let Some((_, b)) = owner_takes_buffer(&ops[at], vars, data)
                    .filter(|(w, b)| freed.contains(w) && owned_buffers.contains(b))
                else {
                    at += 1;
                    continue;
                };
                // After the last statement of this block that still uses the buffer: the
                // literal's own length write and fill follow the bind.
                let last = (at..ops.len())
                    .rev()
                    .find(|&j| ops[j].reads_var(b))
                    .unwrap_or(at);
                ops.insert(
                    last + 1,
                    crate::data::v_set(b, Value::Call(sentinel, Vec::new())),
                );
                at = last + 2;
            }
        });
        data.definitions[d_nr as usize].code = code;
    }
}

/// `Set(w, OpGetField(__vdb_N, 0, …))` where `w` is a local OWNER (no dep on the buffer):
/// answers `(w, buffer)`.
fn owner_takes_buffer(n: &Value, vars: &Function, data: &Data) -> Option<(u16, u16)> {
    let Value::Set(w, rhs) = n.unspan() else {
        return None;
    };
    let Value::Call(d, args) = rhs.unspan() else {
        return None;
    };
    if data.def(*d).name() != "OpGetField" {
        return None;
    }
    let (Some(Value::Var(b)), Some(Value::Int(0))) = (
        args.first().map(Value::unspan),
        args.get(1).map(Value::unspan),
    ) else {
        return None;
    };
    (*w != *b
        && vars.name(*b).starts_with("__vdb")
        && !vars.name(*w).starts_with("__")
        && !vars.is_argument(*w)
        && !vars.tp(*w).depend().contains(b))
    .then_some((*w, *b))
}
