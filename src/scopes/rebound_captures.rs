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
    /// A record capture's OWNER slot (`<capture>__own`, filled from the local's owner witness
    /// `__own_<capture>`): per `(record local, captured local)`, its offset.
    owners: HashMap<(u16, u16), i32>,
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
    let mut owners: HashMap<(u16, u16), i32> = HashMap::default();
    code.any_node(&mut |n| {
        match n.unspan() {
            Value::Call(d, args) if *d == set_dbref => {
                if let (Some(Value::Var(w)), Some(Value::Int(off)), Some(Value::Var(x))) = (
                    args.first().map(Value::unspan),
                    args.get(1).map(Value::unspan),
                    args.get(2).map(Value::unspan),
                ) && vars.name(*w).starts_with("___clos_")
                    && let Some(cap) = vars.name(*x).strip_prefix("__own_")
                    && let Some(c) = rebound.iter().find(|c| vars.name(**c) == cap)
                {
                    owners.insert((*w, *c), *off);
                }
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
            // `c = OpGetField(__vdb_N, 0, …)`: a collection local backed by a literal buffer;
            // `c = { OpDatabase(__ref_N, …); …; __ref_N }`: a record built in a work-ref.
            Value::Set(c, rhs) if rebound.contains(c) => {
                if let Some(b) = built_backing(rhs, database, get_field) {
                    let e = backings.entry(*c).or_default();
                    if !e.contains(&b) {
                        e.push(b);
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
    // A record confined to its loop pass dies with the pass and BORROWS what it captured: the
    // frame's own release stands (`closure_keep::pass_confined_records`), so it is no name here.
    let confined = super::closure_keep::pass_confined_records(data, vars, code);
    for ss in slots.values_mut() {
        ss.retain(|s| {
            !matches!(vars.tp(s.record).base(), crate::data::Type::Reference(rec, _)
                if confined.contains(rec))
        });
    }
    slots.retain(|c, ss| !ss.is_empty() && backings.contains_key(c));
    if slots.is_empty() {
        return None;
    }
    Some(Rebound {
        slots,
        backings,
        owners,
    })
}

/// The backing local an assignment's right-hand side mints into: a collection's literal buffer
/// (`OpGetField(__vdb_N, 0, …)`) or a record built in a work-ref
/// (`{ OpDatabase(__ref_N, …); …; __ref_N }`).
fn built_backing(rhs: &Value, database: u32, get_field: u32) -> Option<u16> {
    match rhs.unspan() {
        Value::Call(d, args) if *d == get_field => match args.first().map(Value::unspan) {
            Some(Value::Var(b)) => Some(*b),
            _ => None,
        },
        Value::Block(bl) => match bl.operators.last().map(Value::unspan) {
            Some(Value::Var(b))
                if bl.operators.iter().any(|op| {
                    matches!(op.unspan(), Value::Call(d, a) if *d == database
                        && matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if x == b))
                }) =>
            {
                Some(*b)
            }
            _ => None,
        },
        _ => None,
    }
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
    // backing -> every slot that can hold its store, with the local it captures.
    let mut held_by: HashMap<u16, Vec<(Slot, u16)>> = HashMap::default();
    for (c, bs) in &facts.backings {
        if let Some(ss) = facts.slots.get(c) {
            for b in bs {
                let e = held_by.entry(*b).or_default();
                for s in ss {
                    if !e.contains(&(*s, *c)) {
                        e.push((*s, *c));
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
            .map(|&(s, c)| {
                let same = Value::Call(distinct, vec![Value::Var(*b), slot_read(s, data)]);
                let mut detach = v_set(*b, Value::Call(sentinel, Vec::new()));
                // A record capture with an OWNER slot takes the store over there, so its
                // cascade releases it and the frame's own name is cleared (`@FR-L-CapOwn`).
                if let Some(&own) = facts.owners.get(&(s.record, c)) {
                    let take = Value::Call(
                        data.def_nr("OpSetDbRef"),
                        vec![Value::Var(s.record), Value::Int(own), slot_read(s, data)],
                    );
                    detach = Value::Insert(vec![take, detach]);
                }
                v_if(
                    record_live(s.record, data),
                    v_if(same, Value::Null, detach),
                    Value::Null,
                )
            })
            .collect()
    });
}

/// The capture slots a BUILD statement writes: a fn-ref local's build
/// (`g = { OpDatabase(W); OpSetDbRef(W, off, c); …; FnRef }`) or a fn field's
/// (`fn_ref_field_set`), each `(slot, captured local)`.
fn slots_written_by(op: &Value, facts: &Rebound, data: &Data) -> Vec<(Slot, u16)> {
    let build = match op.unspan() {
        Value::Set(_, rhs) => match rhs.unspan() {
            Value::Block(b) if b.name == "fn_ref_with_closure" => rhs.unspan(),
            _ => return Vec::new(),
        },
        Value::Block(b) if b.name == "fn_ref_field_set" => op.unspan(),
        _ => return Vec::new(),
    };
    let set_dbref = data.def_nr("OpSetDbRef");
    let get_dbref = data.def_nr("OpGetDbRef");
    let free_distinct = data.def_nr("OpFreeRefIfDistinct");
    // The slots the build already releases itself (a record capture's rebuild,
    // `OpFreeRefIfDistinct(OpGetDbRef(old, off), c)`): those are not released twice.
    let mut released: Vec<i32> = Vec::new();
    build.any_node(&mut |n| {
        if let Value::Call(d, args) = n.unspan()
            && *d == free_distinct
            && let Some(Value::Call(g, gargs)) = args.first().map(Value::unspan)
            && *g == get_dbref
            && let Some(Value::Int(off)) = gargs.get(1).map(Value::unspan)
        {
            released.push(*off);
        }
        false
    });
    let mut out = Vec::new();
    build.any_node(&mut |n| {
        if let Value::Call(d, args) = n.unspan()
            && *d == set_dbref
            && let (Some(Value::Var(w)), Some(Value::Int(off)), Some(Value::Var(c))) = (
                args.first().map(Value::unspan),
                args.get(1).map(Value::unspan),
                args.get(2).map(Value::unspan),
            )
        {
            let s = Slot {
                record: *w,
                off: *off,
            };
            if facts.slots.get(c).is_some_and(|ss| ss.contains(&s))
                && !released.contains(off)
                && !out.contains(&(s, *c))
            {
                out.push((s, *c));
            }
        }
        false
    });
    out
}

/// Let-go point 2 — the REBUILD of a record (each run of the build is its own record,
/// `@FR-L-CapKeep`): the store the record captured on its previous run is released here unless
/// another name still holds it — the captured local itself, one of its backings, or another
/// record's slot for it.  Placed before the build statement, where the record local still
/// names the previous run's record.
fn release_on_rebuild(code: &mut Value, facts: &Rebound, data: &Data) {
    let distinct = data.def_nr("OpDistinctStore");
    let held = data.def_nr("OpConvBoolFromRef");
    let live = data.def_nr("OpStoreLive");
    let free = data.def_nr("OpFreeRef");
    insert_in_loops(code, false, &mut |op| {
        let mut out = Vec::new();
        for (s, c) in slots_written_by(op, facts, data) {
            let old = slot_read(s, data);
            let mut others: Vec<Value> = vec![Value::Var(c)];
            others.extend(
                facts
                    .backings
                    .get(&c)
                    .into_iter()
                    .flatten()
                    .filter(|b| **b != c)
                    .map(|b| Value::Var(*b)),
            );
            // Distinct from every frame name, folded as `&&` lowers.
            let mut test = Value::Call(free, vec![old.clone()]);
            for other in others.into_iter().rev() {
                test = v_if(
                    Value::Call(distinct, vec![old.clone(), other]),
                    test,
                    Value::Null,
                );
            }
            // …and from every other record's slot for the same local.
            for o in facts.slots.get(&c).into_iter().flatten().rev() {
                if *o == s {
                    continue;
                }
                let same = v_if(
                    Value::Call(distinct, vec![old.clone(), slot_read(*o, data)]),
                    Value::Boolean(false),
                    Value::Boolean(true),
                );
                test = v_if(
                    v_if(record_live(o.record, data), same, Value::Boolean(false)),
                    Value::Null,
                    test,
                );
            }
            let present = v_if(
                Value::Call(held, vec![old.clone()]),
                Value::Call(live, vec![old.clone()]),
                Value::Boolean(false),
            );
            out.push(v_if(
                record_live(s.record, data),
                v_if(present, test, Value::Null),
                Value::Null,
            ));
        }
        out
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

/// The record locals whose record LEAVES the frame: a fn field's record whose host the
/// function returns (`W = OpChildRec(OpGetField(h, …), …)`, `return h`), or a fn-ref local's
/// record the function returns (`g = { …; FnRef(_, W) }`, `return g`).
fn leaving_records(code: &Value, vars: &Function, data: &Data) -> HashSet<u16> {
    let child_rec = data.def_nr("OpChildRec");
    let get_field = data.def_nr("OpGetField");
    let mut holder: HashMap<u16, u16> = HashMap::default();
    let mut returned: HashSet<u16> = HashSet::default();
    code.any_node(&mut |n| {
        match n.unspan() {
            Value::Set(w, rhs) if vars.name(*w).starts_with("___clos_") => {
                if let Value::Call(d, args) = rhs.unspan()
                    && *d == child_rec
                    && let Some(Value::Call(g, gargs)) = args.first().map(Value::unspan)
                    && *g == get_field
                    && let Some(Value::Var(h)) = gargs.first().map(Value::unspan)
                {
                    holder.insert(*w, *h);
                }
            }
            Value::Set(g, rhs) => {
                rhs.any_node(&mut |m| {
                    if let Value::FnRef(_, w, _) = m.unspan()
                        && *w != u16::MAX
                        && vars.name(*w).starts_with("___clos_")
                    {
                        holder.insert(*w, *g);
                    }
                    false
                });
            }
            Value::Return(v) => {
                if let Value::Var(x) = v.unspan() {
                    returned.insert(*x);
                }
            }
            _ => {}
        }
        false
    });
    holder
        .into_iter()
        .filter(|(_, h)| returned.contains(h))
        .map(|(w, _)| w)
        .collect()
}

/// The local a frame release frees: `OpFreeRef(x)`, `OpFreeRefIfDistinct(x, …)`, or the
/// conditional `if <a record is there> null else OpFreeRef(x)` a loop build's capture gets.
fn released_local(op: &Value, data: &Data) -> Option<u16> {
    let free = data.def_nr("OpFreeRef");
    let free_distinct = data.def_nr("OpFreeRefIfDistinct");
    match op.unspan() {
        Value::Call(d, args) if *d == free || *d == free_distinct => {
            match args.first().map(Value::unspan) {
                Some(Value::Var(x)) => Some(*x),
                _ => None,
            }
        }
        Value::If(_, t, e) if matches!(t.unspan(), Value::Null) => released_local(e, data),
        _ => None,
    }
}

/// Let-go point 3 — the FRAME's end: a release of the captured local or one of its backings
/// stands down where a record that LEAVES the frame holds that store (its cascade is the
/// release, `@FR-L-CapOwn`), decided by identity rather than by the record's presence — the
/// loop's last pass may have re-assigned the local after its build, or built it after the
/// re-assignment.  Only releases outside the loop are frame-end ones.
fn keep_leaving_captures(code: &mut Value, facts: &Rebound, leaving: &HashSet<u16>, data: &Data) {
    let distinct = data.def_nr("OpDistinctStore");
    let free = data.def_nr("OpFreeRef");
    // released local -> the leaving slots that can hold its store.
    let mut kept_by: HashMap<u16, Vec<Slot>> = HashMap::default();
    for (c, ss) in &facts.slots {
        let leave: Vec<Slot> = ss
            .iter()
            .filter(|s| leaving.contains(&s.record))
            .copied()
            .collect();
        if leave.is_empty() {
            continue;
        }
        let mut names = vec![*c];
        names.extend(facts.backings.get(c).into_iter().flatten().copied());
        for x in names {
            let e = kept_by.entry(x).or_default();
            for s in &leave {
                if !e.contains(s) {
                    e.push(*s);
                }
            }
        }
    }
    if kept_by.is_empty() {
        return;
    }
    wrap_outside_loops(code, &mut |op| {
        let x = released_local(op, data)?;
        let slots = kept_by.get(&x)?;
        // The presence-decided form: its release is the plain one, decided here by identity.
        let release = match op.unspan() {
            Value::If(..) => Value::Call(free, vec![Value::Var(x)]),
            _ => op.clone(),
        };
        Some(slots.iter().rev().fold(release, |rest, &s| {
            let same = v_if(
                Value::Call(distinct, vec![Value::Var(x), slot_read(s, data)]),
                Value::Boolean(false),
                Value::Boolean(true),
            );
            v_if(
                v_if(record_live(s.record, data), same, Value::Boolean(false)),
                Value::Null,
                rest,
            )
        }))
    });
}

/// Replace every statement outside a `Loop` that `wrap` answers for with its answer.
fn wrap_outside_loops(code: &mut Value, wrap: &mut dyn FnMut(&Value) -> Option<Value>) {
    if matches!(code.unspan(), Value::Loop(_)) {
        return;
    }
    if let Value::Block(bl) = code.unspan_mut() {
        for op in &mut bl.operators {
            if let Some(w) = wrap(op) {
                *op = w;
            }
        }
    }
    code.for_each_child_mut(&mut |ch| wrap_outside_loops(ch, wrap));
}

/// loft#2357's three let-go points, over every function whose closures capture a local the
/// loop re-assigns.  Runs after the scope pass, which placed every other release.
pub(super) fn release_rebound_captures(data: &mut Data, fresh: &[u32]) {
    // The A/B switch for a bisect: the scope pass's releases as they stood without this pass.
    if crate::env_once!(std::env::var_os("LOFT_NO_REBOUND_CAPTURES").is_some()) {
        return;
    }
    for &d_nr in fresh {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let Some(facts) = rebound_facts(data.def(d_nr).code(), data.def(d_nr).variables(), data)
        else {
            continue;
        };
        let mut code = data.def(d_nr).code.clone();
        detach_held_backing(&mut code, &facts, data);
        release_on_rebuild(&mut code, &facts, data);
        let leaving = leaving_records(&code, data.def(d_nr).variables(), data);
        if !leaving.is_empty() {
            keep_leaving_captures(&mut code, &facts, &leaving, data);
        }
        data.definitions[d_nr as usize].code = code;
    }
}
