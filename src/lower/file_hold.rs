// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-FileHold` — which `File` a loop may read through a held cursor.
//!
//! A scalar read (`f#read(2) as i16`) moves the cursor in the `File` record: it reads the
//! record's format, handle and `#next` and writes `#index` and `#next` back, per value.  A
//! loop that reads `f` and in which NOTHING ELSE can see that record needs neither: the
//! cursor rides in locals and the record gets it back when the loop is left
//! (`codegen_runtime::FileCursor`).
//!
//! "Nothing else can see the record" is decided on types.  A `File` record is reached
//! through a value whose type holds a `File` — the variable itself, a second name for it, a
//! structure or collection with a `File` in it, a function value (which may have captured
//! one) — and a call reaches only what its arguments reach.  So the loop is admitted when
//! `f` is the ONE variable in it whose type can hold a `File`, and every mention of `f` is
//! the file operand of a read `generation::ops::file_ops` types as a fixed-width integer.

use crate::data::{Block, Data, Type, Value};
use std::collections::HashMap;

/// Can a value of type `tp` hold, or lead to, a `File` record?
///
/// A type former this walk does not name answers YES: an unknown shape might carry a
/// reference, and a yes only keeps the per-value read.  A record type under inspection
/// answers no for itself, which is the least fixpoint — a `File` reachable through a cycle
/// is also reachable along a path that does not repeat a type.
fn holds_file(data: &Data, file: u32, tp: &Type, seen: &mut HashMap<u32, bool>) -> bool {
    match tp {
        Type::Null
        | Type::Void
        | Type::Never
        | Type::Integer(_)
        | Type::Boolean
        | Type::Float
        | Type::Single
        | Type::Character
        | Type::Text(_)
        | Type::Keys => false,
        Type::Enum(_, false, _) => false,
        Type::Reference(d, _)
        | Type::Sorted(d, _, _)
        | Type::Index(d, _, _)
        | Type::Radix(d, _, _)
        | Type::Trie(d, _, _)
        | Type::Hash(d, _, _) => record_holds_file(data, file, *d, seen),
        Type::RefVar(t) | Type::Vector(t, _) | Type::Optional(t) | Type::Rewritten(t) => {
            holds_file(data, file, t, seen)
        }
        Type::Tuple(items) => items.iter().any(|t| holds_file(data, file, t, seen)),
        // A struct-enum's payloads are its variants' own records, not read here.
        Type::Enum(_, true, _)
        | Type::Unknown(_)
        | Type::Routine(_)
        | Type::Iterator(..)
        | Type::Function(..) => true,
    }
}

/// [`holds_file`] for the record type `d`: itself a `File`, or a field that holds one.
fn record_holds_file(data: &Data, file: u32, d: u32, seen: &mut HashMap<u32, bool>) -> bool {
    if d == file {
        return true;
    }
    if d as usize >= data.definitions.len() {
        return true;
    }
    if let Some(&known) = seen.get(&d) {
        return known;
    }
    seen.insert(d, false);
    let holds = data
        .def(d)
        .attributes
        .iter()
        .any(|a| holds_file(data, file, &a.typedef, seen));
    seen.insert(d, holds);
    holds
}

/// The `File` variable the loop `body` may read through a held cursor, or `None`.
///
/// `typed` answers whether one `OpReadFile` call is a fixed-width integer read the emitter
/// will route through the guard; the emitter asks the same function, so a read this gate
/// counted is a read the guard serves.
pub(crate) fn held_file(
    body: &Block,
    data: &Data,
    def_nr: u32,
    typed: &dyn Fn(&[Value]) -> bool,
) -> Option<u16> {
    let file = data.def_nr("File");
    let read = data.def_nr("OpReadFile");
    if file == u32::MAX || read == u32::MAX {
        return None;
    }
    let vars = data.def(def_nr).variables();
    let mut seen: HashMap<u32, bool> = HashMap::new();
    // Every variable of a type that can hold a `File`, with how often the body names it.
    let mut named: HashMap<u16, usize> = HashMap::new();
    let mut reads: HashMap<u16, usize> = HashMap::new();
    let mut declined = false;
    for o in &body.operators {
        o.any_node(&mut |n| {
            let var = match n {
                Value::Var(x) | Value::TupleGet(x, _) | Value::TuplePut(x, _, _) => Some(*x),
                // A write of the variable names another record from here on.
                Value::Set(x, _) => {
                    if holds_file(data, file, vars.tp(*x), &mut seen) {
                        declined = true;
                    }
                    None
                }
                // A function value may have captured a `File`; a generator's frame and a
                // parallel body outlive or leave the loop the guard is a local of; raw text
                // names what this walk cannot read.
                Value::CallRef(..)
                | Value::FnRef(..)
                | Value::FnRefDnr(_)
                | Value::Iter(..)
                | Value::Yield(_)
                | Value::Parallel(_)
                | Value::RawExpr(_) => {
                    declined = true;
                    None
                }
                Value::Call(d, args) if *d == read => {
                    if let Some(Value::Var(f)) = args.first().map(Value::unspan)
                        && typed(args)
                    {
                        *reads.entry(*f).or_default() += 1;
                    }
                    None
                }
                _ => None,
            };
            if let Some(x) = var
                && holds_file(data, file, vars.tp(x), &mut seen)
            {
                *named.entry(x).or_default() += 1;
            }
            declined
        });
        if declined {
            return None;
        }
    }
    let mut it = named.iter();
    let (&f, &mentions) = it.next()?;
    if it.next().is_some() {
        return None;
    }
    // The record itself, never a `File?`: a null file has no record to give the cursor to.
    let tp = vars.tp(f);
    let plain = !tp.peel_optional().1
        && matches!(tp.base(), Type::Reference(d, _) if *d == file)
        && !vars.is_captured(f);
    (plain && reads.get(&f) == Some(&mentions)).then_some(f)
}
