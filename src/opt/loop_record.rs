// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks
//! `@FR-R-LoopRecord` in the IR phase: a plain no-heap record local declared and minted by a
//! literal inside a loop (`p = Pair { … }` per pass) keeps its store and its record across the
//! passes.  The records are `opt::loop_records`' (the writers' analysis); this writes its answer
//! into the IR, so every backend runs it:
//!
//! - the local's declaration `p = null` moves before the OUTERMOST loop around it (the nest
//!   clause: every pass of every loop reaches the mint) and one `OpFreeRef(p)` follows that
//!   loop; the local's scope becomes the block holding the loop, so its slot lives across the
//!   passes;
//! - in the block that declared it, the per-pass `p = null` and the end-of-body free go;
//! - the mint of a COMPLETE literal (`complete_writes`: every field written by the statements
//!   that follow) is taken only where the local holds no store — `if OpRefIsNull(p) { mint }` —
//!   and a kept pass finds the record the literal then overwrites whole.  A partial literal keeps
//!   its mint, whose reuse arm re-establishes the omitted fields' defaults on the kept store;
//! - the REFILL clause (a record whose vector fields hold no-heap elements, under a complete
//!   literal): the literal's zeros of those fields move into the mint's arm, and a kept pass
//!   clears each vector instead (`OpClearVector`), keeping its block;
//! - a free on an early exit (`return`, `continue`) stays, and is followed by `p = null`, so a
//!   later pass mints afresh (the interpreter's free leaves the variable as it was).
//!
//! A function that yields or calls a generator is declined (a live frame may hold the store).
//! `LOFT_NO_LOOP_RECORD=1` turns the rule off on every backend (and `LOFT_NO_LOOP_BUFFER_REUSE=1`
//! with the loop buffers); `LOFT_TRACE_IR_LOOP_RECORD=1` names each record.
use crate::data::{Data, DefType, Type, Value, v_block, v_if};
use crate::database::{Parts, Stores};
use crate::opt::loop_records::{LoopRecord, loop_records};
use std::collections::HashMap;

fn enabled() -> bool {
    !crate::env_once!(std::env::var("LOFT_NO_LOOP_RECORD").is_ok_and(|v| v != "0"))
        && crate::keys::loop_buffer_reuse_enabled()
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_IR_LOOP_RECORD").is_ok_and(|v| v != "0"))
}

struct Ops {
    database: [u32; 2],
    frees: [u32; 3],
    set_int4: u32,
    get_field: u32,
    ref_is_null: u32,
    clear_vector: u32,
    free_ref: u32,
}

/// One admitted record, as the rewrite needs it.
struct Rec {
    lr: LoopRecord,
    complete: bool,
    /// The record's vector fields: `(byte offset, the field's type number)`.
    vectors: Vec<(i32, i32)>,
}

/// Rewrite every admitted loop record of the program; answers how many.
pub fn rewrite_program(data: &mut Data, stores: &Stores) -> usize {
    if !enabled() {
        return 0;
    }
    let ops = Ops {
        database: [data.def_nr("OpDatabase"), data.def_nr("OpDatabaseNP")],
        frees: [
            data.def_nr("OpFreeRef"),
            data.def_nr("OpFreeRefIfDistinct"),
            data.def_nr("OpFreeRefTag"),
        ],
        set_int4: data.def_nr("OpSetInt4"),
        get_field: data.def_nr("OpGetField"),
        ref_is_null: data.def_nr("OpRefIsNull"),
        clear_vector: data.def_nr("OpClearVector"),
        free_ref: data.def_nr("OpFreeRef"),
    };
    let apr = crate::opt::apart_records::plan(data, stores);
    let mut total = 0usize;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function
            || matches!(def.code(), Value::Null)
            || def.is_operator()
            || !def.rust().is_empty()
            || calls_a_generator(data, def.code())
        {
            continue;
        }
        let found = loop_records(data, stores, d);
        if found.recs.is_empty() {
            continue;
        }
        let recs = admit(data, stores, d, found, &apr, &ops);
        if recs.is_empty() {
            continue;
        }
        let _census = crate::rewrite_census::InBody::enter("ir", def.name());
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let mut hoisted: HashMap<u16, u16> = HashMap::new();
        rewrite(&mut code, &recs, &ops, &mut hoisted, false);
        data.definitions[d as usize].code = code;
        let vars = &mut data.definitions[d as usize].variables;
        for (v, scope) in &hoisted {
            vars.move_scope(*v, *scope);
        }
        let n = hoisted.len();
        if n == 0 {
            continue;
        }
        data.definitions[d as usize].variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, d);
        crate::scopes::assign_function_slots(data, d);
        let refill = hoisted.keys().filter(|v| recs[v].lr.refill).count();
        if trace() {
            let vars = data.def(d).variables();
            for v in hoisted.keys() {
                eprintln!(
                    "[ir-loop-record] fn={} {} kept across the passes{}",
                    data.def(d).name(),
                    vars.name(*v),
                    if recs[v].lr.refill { " (refill)" } else { "" }
                );
            }
        }
        crate::rewrite_census::fired("R-LoopRecord", n - refill);
        crate::rewrite_census::fired("R-LoopRecord/refill", refill);
        total += n;
    }
    total
}

/// The admitted records of function `d` the rewrite takes: a reference local no other rule
/// owns, and — for the refill clause — a complete literal with vector fields to keep.
fn admit(
    data: &Data,
    stores: &Stores,
    d: u32,
    found: crate::opt::loop_records::LoopRecords,
    apr: &crate::opt::apart_records::RecPlan,
    ops: &Ops,
) -> HashMap<u16, Rec> {
    let complete = crate::opt::ret_adopt::complete_writes(data, stores, d).db_vars;
    // A record another rule owns (`opt::buffer_owner`: an apart record holds no store at
    // all, which a kept store cannot beat) is left to it.
    let owned = crate::opt::buffer_owner::owners(data, stores, d, apr);
    let def = data.def(d);
    let vars = def.variables();
    let mut recs: HashMap<u16, Rec> = HashMap::new();
    for (v, rec) in found.recs {
        let Type::Reference(_, _) = vars.tp(v) else {
            continue;
        };
        if let Some(owner) = owned.get(&v) {
            if trace() {
                eprintln!(
                    "[ir-loop-record] fn={} {} left: {}",
                    def.name(),
                    vars.name(v),
                    owner.why()
                );
            }
            continue;
        }
        let vectors = if rec.refill {
            record_type(def.code(), v, ops)
                .map(|tp| vector_fields(stores, tp))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        // The refill clause keeps its vectors only under a complete literal: a partial
        // one's prefill would zero a kept vector's slot and strand its block.
        let complete = complete.contains(&v);
        if rec.refill && (!complete || vectors.is_empty()) {
            continue;
        }
        recs.insert(
            v,
            Rec {
                lr: rec,
                complete,
                vectors,
            },
        );
    }
    recs
}

/// Does `code` call a function that answers a generator, or yield itself?
fn calls_a_generator(data: &Data, code: &Value) -> bool {
    code.any_node(&mut |n| match n {
        Value::Yield(_) => true,
        Value::Call(d, _) => {
            (*d as usize) < data.definitions.len()
                && matches!(data.def(*d).returned().base(), Type::Iterator(..))
        }
        _ => false,
    })
}

fn var_of(v: Option<&Value>) -> Option<u16> {
    match v.map(Value::unspan) {
        Some(Value::Var(x)) => Some(*x),
        _ => None,
    }
}

fn int_of(v: Option<&Value>) -> Option<i32> {
    match v.map(Value::unspan) {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    }
}

/// The type number `v`'s mint names (`OpDatabase*(v, tp)`).
fn record_type(code: &Value, v: u16, ops: &Ops) -> Option<u16> {
    let mut tp = None;
    code.walk(&mut |n| {
        if let Value::Call(op, a) = n
            && ops.database.contains(op)
            && var_of(a.first()) == Some(v)
        {
            tp = int_of(a.get(1)).and_then(|t| u16::try_from(t).ok());
        }
    });
    tp
}

/// The vector fields of record type `tp`: `(byte offset, the field's type number)`.
fn vector_fields(stores: &Stores, tp: u16) -> Vec<(i32, i32)> {
    let Some(Parts::Struct(fields)) = stores.types.get(tp as usize).map(|t| &t.parts) else {
        return Vec::new();
    };
    fields
        .iter()
        .filter(|f| {
            matches!(
                stores.types.get(f.content as usize).map(|c| &c.parts),
                Some(Parts::Vector(_))
            )
        })
        .map(|f| (i32::from(f.position), i32::from(f.content)))
        .collect()
}

/// Rewrite the statement lists under `v`.  `hoisted` collects each record whose declaration
/// moved, with the scope it moved to.
fn rewrite(
    v: &mut Value,
    recs: &HashMap<u16, Rec>,
    ops: &Ops,
    hoisted: &mut HashMap<u16, u16>,
    in_loop: bool,
) {
    if let Value::Block(bl) | Value::Loop(bl) = v.unspan_mut() {
        let scope = bl.scope;
        let old = std::mem::take(&mut bl.operators);
        let mut out: Vec<Value> = Vec::with_capacity(old.len());
        for stmt in old {
            // The outermost loop of a record: its declaration before, its free after.
            let loop_scope = match stmt.unspan() {
                Value::Loop(lp) => Some(lp.scope),
                _ => None,
            };
            let mine: Vec<u16> = loop_scope
                .map(|ls| {
                    let mut m: Vec<u16> = recs
                        .iter()
                        .filter(|(_, r)| r.lr.loop_scope == ls)
                        .map(|(v, _)| *v)
                        .collect();
                    m.sort_unstable();
                    m
                })
                .unwrap_or_default();
            for r in &mine {
                out.push(Value::Set(*r, Box::new(Value::Null)));
                hoisted.insert(*r, scope);
            }
            statement(stmt, scope, recs, ops, &mut out);
            for r in &mine {
                out.push(Value::Call(ops.free_ref, vec![Value::Var(*r)]));
            }
        }
        bl.operators = out;
    }
    let inner = in_loop || matches!(v.unspan(), Value::Loop(_));
    v.for_each_child_mut(&mut |c| rewrite(c, recs, ops, hoisted, inner));
}

/// Emit statement `stmt` of a block of `scope` (or what replaces it) into `out`.
fn statement(stmt: Value, scope: u16, recs: &HashMap<u16, Rec>, ops: &Ops, out: &mut Vec<Value>) {
    match stmt.unspan() {
        // The per-pass declaration, in the block that declared the record.
        Value::Set(v, rhs)
            if matches!(rhs.unspan(), Value::Null)
                && recs.get(v).is_some_and(|r| r.lr.decl_block == scope) => {}
        // A free of the record: its end-of-body free goes; any other (an early exit's) stays
        // and nulls the local, so a later pass mints afresh.
        Value::Call(op, a)
            if ops.frees.contains(op)
                && var_of(a.first()).is_some_and(|v| recs.contains_key(&v)) =>
        {
            let v = var_of(a.first()).unwrap_or(u16::MAX);
            if recs[&v].lr.decl_block != scope {
                out.push(stmt.clone());
                out.push(Value::Set(v, Box::new(Value::Null)));
            }
        }
        // The mint of a complete literal: taken only where the local holds no store; the
        // refill clause's vector zeros go into that arm, a kept pass clearing each vector.
        Value::Call(op, a)
            if ops.database.contains(op)
                && var_of(a.first()).is_some_and(|v| {
                    recs.get(&v)
                        .is_some_and(|r| r.complete && r.lr.decl_block == scope)
                }) =>
        {
            let v = var_of(a.first()).unwrap_or(u16::MAX);
            let r = &recs[&v];
            let mut mint = vec![stmt.clone()];
            let mut keep: Vec<Value> = Vec::new();
            for (off, ftp) in &r.vectors {
                mint.push(Value::Call(
                    ops.set_int4,
                    vec![Value::Var(v), Value::Int(*off), Value::Int(0)],
                ));
                keep.push(Value::Call(
                    ops.clear_vector,
                    vec![Value::Call(
                        ops.get_field,
                        vec![Value::Var(v), Value::Int(*off), Value::Int(*ftp)],
                    )],
                ));
            }
            let other = if keep.is_empty() {
                Value::Null
            } else {
                v_block(keep, Type::Void, "loop record keep")
            };
            out.push(v_if(
                Value::Call(ops.ref_is_null, vec![Value::Var(v)]),
                v_block(mint, Type::Void, "loop record mint"),
                other,
            ));
        }
        // The literal's zero of a refill record's vector field: now in the mint's arm.
        Value::Call(op, a)
            if *op == ops.set_int4
                && int_of(a.get(2)) == Some(0)
                && var_of(a.first()).is_some_and(|v| {
                    recs.get(&v).is_some_and(|r| {
                        r.complete
                            && r.lr.decl_block == scope
                            && int_of(a.get(1))
                                .is_some_and(|off| r.vectors.iter().any(|(o, _)| *o == off))
                    })
                }) => {}
        _ => out.push(stmt),
    }
}
