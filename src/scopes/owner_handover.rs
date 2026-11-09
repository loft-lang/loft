// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! loft#2363 — a closure record that OWNS a capture through its owner slot hands that
//! ownership to another record holding the same store when it is released first.
//!
//! A local re-assigned after its build takes the owner-witness path (`@FR-O-Witness`): the
//! build hands the local's store to the record's OWNER slot (`<capture>__own`) and resets the
//! witness, so of two records built over the same store the FIRST owns it and the second only
//! captures it.  `@FR-L-CapOne` says the one that LEAVES owns it, and `@FR-L-CapKeep` that the
//! store is released by the last of its names to let go.  So at each record's release (its
//! cascade, `<record>_OpDropAll`), a record whose owner slot holds a store another live record's
//! capture slot holds — and that record owns nothing there yet — passes the ownership over
//! instead of releasing it.  Decided by store identity at the release; only records that carry
//! an owner slot for a capture another record also carries are touched.

use crate::data::{Data, DefType, Value, v_if};
use crate::fxhash::FxHashMap as HashMap;
use crate::variables::Function;

/// One record's hold on a capture: its capture slot and its owner slot.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Hold {
    record: u16,
    slot: i32,
    own: i32,
}

/// Per captured local, every record that holds it through an owner slot.
fn owner_holds(code: &Value, vars: &Function, data: &Data) -> HashMap<u16, Vec<Hold>> {
    let set_dbref = data.def_nr("OpSetDbRef");
    // (record, capture) -> capture slot / owner slot
    let mut slot: HashMap<(u16, u16), i32> = HashMap::default();
    let mut own: HashMap<(u16, String), i32> = HashMap::default();
    code.any_node(&mut |n| {
        if let Value::Call(d, args) = n.unspan()
            && *d == set_dbref
            && let (Some(Value::Var(w)), Some(Value::Int(off)), Some(Value::Var(x))) = (
                args.first().map(Value::unspan),
                args.get(1).map(Value::unspan),
                args.get(2).map(Value::unspan),
            )
            && vars.name(*w).starts_with("___clos_")
        {
            match vars.name(*x).strip_prefix("__own_") {
                Some(cap) => {
                    own.insert((*w, cap.to_string()), *off);
                }
                None => {
                    slot.insert((*w, *x), *off);
                }
            }
        }
        false
    });
    let mut out: HashMap<u16, Vec<Hold>> = HashMap::default();
    for ((w, c), s) in slot {
        if let Some(&o) = own.get(&(w, vars.name(c).to_string())) {
            out.entry(c).or_default().push(Hold {
                record: w,
                slot: s,
                own: o,
            });
        }
    }
    out.retain(|_, hs| {
        hs.sort_unstable_by_key(|h| h.record);
        hs.len() > 1
    });
    out
}

/// The record a release statement cascades: `if OpStoreLive(W) <record>_OpDropAll(W) else …`.
fn released_record(op: &Value, data: &Data) -> Option<u16> {
    let Value::If(cond, then, _) = op.unspan() else {
        return None;
    };
    let Value::Call(live, largs) = cond.unspan() else {
        return None;
    };
    // The guard reads `OpConvBoolFromRef` here and becomes `OpStoreLive` later on.
    if !matches!(data.def(*live).name(), "OpStoreLive" | "OpConvBoolFromRef") {
        return None;
    }
    let Some(Value::Var(w)) = largs.first().map(Value::unspan) else {
        return None;
    };
    match then.unspan() {
        Value::Call(d, args)
            if data.def(*d).name().ends_with("_OpDropAll")
                && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if x == w) =>
        {
            Some(*w)
        }
        _ => None,
    }
}

fn live(record: u16, data: &Data) -> Value {
    v_if(
        Value::Call(data.def_nr("OpRefIsNull"), vec![Value::Var(record)]),
        Value::Boolean(false),
        Value::Call(data.def_nr("OpStoreLive"), vec![Value::Var(record)]),
    )
}

fn field(record: u16, off: i32, data: &Data) -> Value {
    Value::Call(
        data.def_nr("OpGetDbRef"),
        vec![Value::Var(record), Value::Int(off)],
    )
}

/// The hand-over from `from` to `to` ahead of `from`'s release: `to` is there, `from`'s owner
/// slot holds the store `to`'s capture slot names, and `to` owns nothing there yet.
fn handover(from: Hold, to: Hold, data: &Data) -> Value {
    let set = data.def_nr("OpSetDbRef");
    let held = data.def_nr("OpConvBoolFromRef");
    let distinct = data.def_nr("OpDistinctStore");
    let sentinel = data.def_nr("OpNullRefSentinel");
    let give = Value::Insert(vec![
        Value::Call(
            set,
            vec![
                Value::Var(to.record),
                Value::Int(to.own),
                field(from.record, from.own, data),
            ],
        ),
        Value::Call(
            set,
            vec![
                Value::Var(from.record),
                Value::Int(from.own),
                Value::Call(sentinel, Vec::new()),
            ],
        ),
    ]);
    let same = Value::Call(
        distinct,
        vec![
            field(from.record, from.own, data),
            field(to.record, to.slot, data),
        ],
    );
    let unowned = Value::Call(held, vec![field(to.record, to.own, data)]);
    let owns = Value::Call(held, vec![field(from.record, from.own, data)]);
    v_if(
        live(to.record, data),
        v_if(
            owns,
            v_if(same, Value::Null, v_if(unowned, Value::Null, give)),
            Value::Null,
        ),
        Value::Null,
    )
}

fn insert_handovers(code: &mut Value, holds: &HashMap<u16, Vec<Hold>>, data: &Data) {
    let ops = match code.unspan_mut() {
        Value::Block(bl) | Value::Loop(bl) => Some(&mut bl.operators),
        // The scope pass places a frame's releases as one `Insert` run.
        Value::Insert(ops) => Some(ops),
        _ => None,
    };
    if let Some(ops) = ops {
        let mut at = 0;
        while at < ops.len() {
            let mut before = Vec::new();
            if let Some(w) = released_record(&ops[at], data) {
                for hs in holds.values() {
                    let Some(&from) = hs.iter().find(|h| h.record == w) else {
                        continue;
                    };
                    for &to in hs.iter().filter(|h| h.record != w) {
                        before.push(handover(from, to, data));
                    }
                }
            }
            let n = before.len();
            for (k, op) in before.into_iter().enumerate() {
                ops.insert(at + k, op);
            }
            at += n + 1;
        }
    }
    code.for_each_child_mut(&mut |ch| insert_handovers(ch, holds, data));
}

/// loft#2363 over every function holding one capture in two records' owner slots.  Runs after
/// the scope pass, which placed every record's release.
pub(super) fn hand_over_owned_captures(data: &mut Data, fresh: &[u32]) {
    if crate::env_once!(std::env::var_os("LOFT_NO_OWNER_HANDOVER").is_some()) {
        return;
    }
    for &d_nr in fresh {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let holds = owner_holds(data.def(d_nr).code(), data.def(d_nr).variables(), data);
        if holds.is_empty() {
            continue;
        }
        let mut code = data.def(d_nr).code.clone();
        insert_handovers(&mut code, &holds, data);
        data.definitions[d_nr as usize].code = code;
    }
}
