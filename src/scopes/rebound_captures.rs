// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! loft#2357 — a closure built in a loop over a local the loop re-assigns: each name of a
//! captured store lets go of it by store identity (`@FR-L-CapKeep`).
//!
//! A loop that builds a closure over `c` and assigns `c` after the build, or builds it in a
//! branch of a pass that assigns `c` (`captures_rebound_in_a_loop`), leaves a store with two
//! kinds of name: the frame's BACKING locals of `c` (`__vdb_N`, or `c` itself) and the capture
//! SLOT of the record each pass built (`___clos_K` at the capture's offset).  Each of them lets
//! go somewhere — a re-mint of the backing, a rebuild of the record, the frame's end — and the
//! store is released by the last, which is a run-time fact decided by identity
//! (`OpDistinctStore`).  Only those shapes are touched; every other program is unchanged.

use crate::data::{Data, DefType, Value, v_if, v_set};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// One capture of a rebound local: the record local that holds it and the slot's offset.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Slot {
    record: u16,
    off: i32,
}

/// The facts one function's rebound captures need.
struct Rebound {
    /// Per captured local: the capture slots that can hold its store.
    slots: HashMap<u16, Vec<Slot>>,
    /// Per captured local: the frame's backing locals — the variables whose `OpDatabase` mints
    /// the store the local names.
    backings: HashMap<u16, Vec<u16>>,
}

fn rebound_facts(code: &Value, vars: &Function, data: &Data) -> Option<Rebound> {
    let set_dbref = data.def_nr("OpSetDbRef");
    let mut rebound: HashSet<u16> = HashSet::default();
    super::capture_builds::captures_rebound_in_a_loop(code, set_dbref, &mut rebound);
    if rebound.is_empty() {
        return None;
    }
    let database = data.def_nr("OpDatabase");
    let get_field = data.def_nr("OpGetField");
    let mut slots: HashMap<u16, Vec<Slot>> = HashMap::default();
    let mut backings: HashMap<u16, Vec<u16>> = HashMap::default();
    code.any_node(&mut |n| {
        match n.unspan() {
            Value::Call(d, args) if *d == set_dbref => {
                if let (Some(Value::Var(w)), Some(Value::Int(off)), Some(Value::Var(c))) = (
                    args.first().map(Value::unspan),
                    args.get(1).map(Value::unspan),
                    args.get(2).map(Value::unspan),
                ) && rebound.contains(c)
                    && vars.name(*w).starts_with("___clos_")
                {
                    let s = Slot {
                        record: *w,
                        off: *off,
                    };
                    let e = slots.entry(*c).or_default();
                    if !e.contains(&s) {
                        e.push(s);
                    }
                }
            }
            // `c = OpGetField(__vdb_N, 0, …)`: a collection local backed by a literal buffer.
            Value::Set(c, rhs) if rebound.contains(c) => {
                if let Value::Call(d, args) = rhs.unspan()
                    && *d == get_field
                    && let Some(Value::Var(b)) = args.first().map(Value::unspan)
                {
                    let e = backings.entry(*c).or_default();
                    if !e.contains(b) {
                        e.push(*b);
                    }
                }
            }
            // `OpDatabase(c, …)`: a record local mints its own store.
            Value::Call(d, args) if *d == database => {
                if let Some(Value::Var(c)) = args.first().map(Value::unspan)
                    && rebound.contains(c)
                {
                    let e = backings.entry(*c).or_default();
                    if !e.contains(c) {
                        e.push(*c);
                    }
                }
            }
            _ => {}
        }
        false
    });
    slots.retain(|c, _| backings.contains_key(c));
    if slots.is_empty() {
        return None;
    }
    Some(Rebound { slots, backings })
}

/// `record` is there and its store is live: a slot read through it is safe.
fn record_live(record: u16, data: &Data) -> Value {
    v_if(
        Value::Call(data.def_nr("OpRefIsNull"), vec![Value::Var(record)]),
        Value::Boolean(false),
        Value::Call(data.def_nr("OpStoreLive"), vec![Value::Var(record)]),
    )
}

fn slot_read(s: Slot, data: &Data) -> Value {
    Value::Call(
        data.def_nr("OpGetDbRef"),
        vec![Value::Var(s.record), Value::Int(s.off)],
    )
}

/// Let-go point 1 — the RE-MINT of a backing (`OpDatabase(b, …)` inside a loop): where a
/// record's capture slot holds `b`'s store, the record keeps it and `b` is detached first, so
/// the mint makes a FRESH store (`@FR-O-Latest`: the record keeps the store the capture named
/// at its build).  The model is `@FR-G-Arg-Rebind`'s `detach_if_frame_held`.
fn detach_held_backing(code: &mut Value, facts: &Rebound, data: &Data) {
    let database = data.def_nr("OpDatabase");
    let sentinel = data.def_nr("OpNullRefSentinel");
    let distinct = data.def_nr("OpDistinctStore");
    // backing -> every slot that can hold its store.
    let mut held_by: HashMap<u16, Vec<Slot>> = HashMap::default();
    for (c, bs) in &facts.backings {
        if let Some(ss) = facts.slots.get(c) {
            for b in bs {
                let e = held_by.entry(*b).or_default();
                for s in ss {
                    if !e.contains(s) {
                        e.push(*s);
                    }
                }
            }
        }
    }
    insert_in_loops(code, false, &mut |op| {
        let Value::Call(d, args) = op.unspan() else {
            return Vec::new();
        };
        if *d != database {
            return Vec::new();
        }
        let Some(Value::Var(b)) = args.first().map(Value::unspan) else {
            return Vec::new();
        };
        let Some(slots) = held_by.get(b) else {
            return Vec::new();
        };
        slots
            .iter()
            .map(|&s| {
                let same = Value::Call(distinct, vec![Value::Var(*b), slot_read(s, data)]);
                let detach = v_set(*b, Value::Call(sentinel, Vec::new()));
                v_if(
                    record_live(s.record, data),
                    v_if(same, Value::Null, detach),
                    Value::Null,
                )
            })
            .collect()
    });
}

/// Insert, before every statement `pick` answers for, the statements it answers — only for
/// statements inside a `Loop`.
fn insert_in_loops(code: &mut Value, in_loop: bool, pick: &mut dyn FnMut(&Value) -> Vec<Value>) {
    let inner = in_loop || matches!(code.unspan(), Value::Loop(_));
    if inner && let Value::Block(bl) | Value::Loop(bl) = code.unspan_mut() {
        let mut at = 0;
        while at < bl.operators.len() {
            let before = pick(&bl.operators[at]);
            let n = before.len();
            for (k, op) in before.into_iter().enumerate() {
                bl.operators.insert(at + k, op);
            }
            at += n + 1;
        }
    }
    code.for_each_child_mut(&mut |ch| insert_in_loops(ch, inner, pick));
}

/// loft#2357's three let-go points, over every function whose closures capture a local the
/// loop re-assigns.  Runs after the scope pass, which placed every other release.
pub(super) fn release_rebound_captures(data: &mut Data) {
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let Some(facts) = rebound_facts(data.def(d_nr).code(), data.def(d_nr).variables(), data)
        else {
            continue;
        };
        let mut code = data.def(d_nr).code.clone();
        detach_held_backing(&mut code, &facts, data);
        data.definitions[d_nr as usize].code = code;
    }
}
