// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! What a generator's definition says about its frame, read once here for both readers: the
//! interpreter's coroutine operators (`State::coroutine_create` … `free_coroutine`) and the run
//! image, which carries the same facts so the loft runner can run a generator without the
//! definitions (@PLN91, `ir_store::save_run_image`).
//!
//! Every offset is a byte offset into the frame: the argument region first (each argument at
//! its `Context::Argument` size, unstepped, as the arguments are copied), then the locals at
//! their stack slots.

use crate::data::{Context, Definition, Type};
use crate::variables::size as var_size;

/// Where each argument starts in the argument region, with its type — the walk
/// `serialise_text_args` and `hold_arg_stores` both make.
fn arg_offsets(def: &Definition) -> Vec<(u32, &Type)> {
    let mut out = Vec::new();
    let mut byte_offset: u32 = 0;
    for attr in def.attributes() {
        out.push((byte_offset, &attr.typedef));
        byte_offset += u32::from(var_size(&attr.typedef, &Context::Argument));
    }
    out
}

/// The text arguments (a `Str` view each), which a created frame copies into texts of its own:
/// the caller's text may be gone before the generator first runs.
#[must_use]
pub fn text_arg_offsets(def: &Definition) -> Vec<u32> {
    arg_offsets(def)
        .into_iter()
        .filter(|(_, tp)| matches!(tp, Type::Text(_)))
        .map(|(off, _)| off)
        .collect()
}

/// The reference arguments, whose stores the frame holds for its life (`@FR-G-Arg-Rebind`).
#[must_use]
pub fn ref_arg_offsets(def: &Definition) -> Vec<u32> {
    arg_offsets(def)
        .into_iter()
        .filter(|(_, tp)| crate::data::is_dbref(tp.base()))
        .map(|(off, _)| off)
        .collect()
}

/// The size of the locals region — past the arguments and the return slot, each at its
/// stepped width as `scopes.rs` lays them — which the first resume zeroes, so a local never
/// written reads back empty when an abandoned frame is released.
#[must_use]
pub fn zone2_size(def: &Definition) -> usize {
    let vars = &def.variables();
    let step = |s: u16| crate::variables::aligned_stack_step(u32::from(s)) as u16;
    let local_start: u16 = vars
        .arguments()
        .iter()
        .map(|&a| step(var_size(vars.tp(a), &Context::Argument)))
        .sum::<u16>()
        .saturating_add(step(4));
    let mut top: u16 = local_start;
    for v in 0..vars.count() {
        if vars.is_argument(v) {
            continue;
        }
        let slot = vars.stack(v);
        if slot == u16::MAX {
            continue;
        }
        let sz = vars.size(v, &Context::Variable);
        top = top.max(slot.saturating_add(sz));
    }
    top.saturating_sub(local_start) as usize
}

/// The slots of the text locals, whose texts an abandoned suspended frame releases.
#[must_use]
pub fn text_local_slots(def: &Definition) -> Vec<u16> {
    let vars = &def.variables();
    (0..vars.count())
        .filter(|&v| !vars.is_argument(v) && matches!(vars.tp(v), Type::Text(_)))
        .map(|v| vars.stack(v))
        .filter(|&slot| slot != u16::MAX)
        .collect()
}

/// The slots of the locals whose stores an abandoned suspended frame frees: a heap local or an
/// iterator that owns its store.  An argument is the caller's — except a generator handle,
/// which the frame holds (`(G-Hold)`, loft#1708); a local a closure record adopted is the
/// record's to release (`@FR-L-CapOwn`, loft#1676).
#[must_use]
pub fn owned_store_local_slots(def: &Definition) -> Vec<u16> {
    let vars = &def.variables();
    let mut out = Vec::new();
    for v in 0..vars.count() {
        let tp = vars.tp(v);
        if vars.is_argument(v) && !matches!(tp.base(), Type::Iterator(_, _)) {
            continue;
        }
        if !crate::data::is_dbref(tp) && !matches!(tp, Type::Iterator(_, _)) {
            continue;
        }
        if !vars.owns_store(v) || vars.is_captured(v) {
            continue;
        }
        let slot = vars.stack(v);
        if slot != u16::MAX {
            out.push(slot);
        }
    }
    out
}

/// The tuple a generator yields, when one of its elements is a reference: those elements are
/// nulled in the null value an exhausted generator answers.
#[must_use]
pub fn yield_tuple(def: &Definition) -> Option<Vec<Type>> {
    match def.returned().base() {
        Type::Iterator(inner, _) => match inner.base() {
            Type::Tuple(elems)
                if elems.iter().any(|e| {
                    crate::data::holds_dbref(e) || crate::data::is_dbref_slot(e.base())
                }) =>
            {
                Some(elems.clone())
            }
            _ => None,
        },
        _ => None,
    }
}

/// The offsets, from the value's start, of the reference elements of a yielded tuple, nested
/// tuples included — where `State::coroutine_return` writes the null reference.
#[must_use]
pub fn tuple_ref_offsets(elems: &[Type]) -> Vec<u32> {
    let mut out = Vec::new();
    tuple_refs_at(elems, 0, &mut out);
    out
}

fn tuple_refs_at(elems: &[Type], base: u32, out: &mut Vec<u32>) {
    let offsets = crate::data::element_stack_offsets(elems);
    for (elm, off) in elems.iter().zip(offsets) {
        let at = base + off as u32;
        match elm.base() {
            Type::Tuple(inner) => tuple_refs_at(inner, at, out),
            t if crate::data::is_dbref_slot(t) => out.push(at),
            _ => {}
        }
    }
}
