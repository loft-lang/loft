// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F11 — Tuples — anonymous fixed-arity `(T1, T2, …)`

//! `@FR-T-Record` — which NARROW members of a tuple local a `&` link names.
//!
//! A tuple is a record whose fields are named `0` … `n` (tuples.md `(T-Record)`, @C139), so a
//! `&` to a narrow member is a `&u8` (`&i16`, …) to a FIELD: a byte pointer reading and writing
//! the member's field encoding (`data::NarrowSlot`).  The stack form keeps a member in an 8-byte
//! slot, which is a representation the record layout lets loft keep only where no program can
//! tell the difference — and a link tells.  So a linked narrow member holds its field encoding
//! instead, exactly as a linked narrow LOCAL does (`variables::linked_narrow_slot`), and every
//! reader and writer of that member decodes and encodes it.
//!
//! The fact is DERIVED from the function's IR, never stored: a member is linked when the body
//! holds `OpCreateStack(TupleGet(t, i))`, the one lowering the `&` bind and the `&` argument
//! share (`Parser::scalar_place_ref`).  Derived, it survives a serialised IR and needs no
//! schema field; both emitters ask this one function, so they cannot disagree.
//! `LOFT_LINK_ALL_NARROW=1` treats every narrow member of a tuple the author can name as
//! linked, so the whole corpus runs the linked shape.

use crate::data::{Data, NarrowSlot, Type, Value};
use std::collections::HashMap;

/// For each tuple local of function `def_nr`, the top-level members that hold their field
/// encoding, one bit per member.  A local that appears in no entry keeps every member in its
/// 8-byte slot.
#[must_use]
pub fn linked_narrow_members(data: &Data, def_nr: u32) -> HashMap<u16, u64> {
    let mut out: HashMap<u16, u64> = HashMap::new();
    if def_nr == u32::MAX || def_nr as usize >= data.definitions.len() {
        return out;
    }
    let def = data.def(def_nr);
    let vars = def.variables();
    let narrow_bits = |t: u16| -> u64 {
        let Type::Tuple(elems) = vars.tp(t).base() else {
            return 0;
        };
        let mut bits = 0u64;
        for (i, e) in elems.iter().enumerate().take(64) {
            if NarrowSlot::of_type(e).is_some() {
                bits |= 1 << i;
            }
        }
        bits
    };
    if crate::variables::link_all_narrow() {
        for t in 0..vars.count() {
            if !vars.is_compiler_generated(t) {
                let bits = narrow_bits(t);
                if bits != 0 {
                    out.insert(t, bits);
                }
            }
        }
        return out;
    }
    let create_stack = data.def_nr("OpCreateStack");
    if create_stack == u32::MAX {
        return out;
    }
    collect(def.code(), create_stack, &mut |t, i| {
        if i < 64 && narrow_bits(t) & (1 << i) != 0 {
            *out.entry(t).or_insert(0) |= 1 << i;
        }
    });
    out
}

/// Every `OpCreateStack(TupleGet(t, i))` in `v`, as `(t, i)`.  Any other node is searched
/// through its children: a link can sit anywhere an expression can.
fn collect(v: &Value, create_stack: u32, found: &mut impl FnMut(u16, usize)) {
    if let Value::Call(d, args) = v.unspan()
        && *d == create_stack
        && let Some(Value::TupleGet(t, i)) = args.first().map(Value::unspan)
    {
        found(*t, *i as usize);
    }
    v.for_each_child(&mut |c| collect(c, create_stack, found));
}

/// The field encoding member `idx` of tuple local `var` holds, given `linked`
/// ([`linked_narrow_members`]'s answer for the function) — `None` for a member kept in its
/// 8-byte slot.
#[must_use]
pub fn member_slot(
    linked: &HashMap<u16, u64>,
    elems: &[Type],
    var: u16,
    idx: usize,
) -> Option<NarrowSlot> {
    if idx >= 64 || linked.get(&var).is_none_or(|bits| bits & (1 << idx) == 0) {
        return None;
    }
    NarrowSlot::of_type(elems.get(idx)?)
}
