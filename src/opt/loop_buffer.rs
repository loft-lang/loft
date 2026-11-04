// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks
//! `@FR-R-LoopBuffer` in the IR phase: a vector local declared `[]` inside a loop keeps its
//! store and its vector across the passes.  The buffers are `opt::hoist::loop_buffers`', the
//! analysis rustc's writer read, less the ones another rule owns (`opt::buffer_owner`, the
//! front's one answer: an inline buffer, a hoisted literal, an element-first pair, a move host,
//! the adopted result's witness).
//! Each declaration
//!
//! ```text
//! OpDatabase(vdb, tp) · t = OpGetField(vdb, 0, ftp) · OpSetInt4(vdb, 0, 0)
//! ```
//!
//! becomes
//!
//! ```text
//! if OpRefIsNull(vdb) { OpDatabase(vdb, tp); OpSetInt4(vdb, 0, 0) }
//! else { OpClearVector(OpGetField(vdb, 0, ftp)) }
//! t = OpGetField(vdb, 0, ftp)
//! ```
//!
//! — the first pass mints, every later pass clears the vector (its elements own no heap, so the
//! clear is a length reset) and keeps the store's record and the vector's capacity.  A function
//! that yields or calls a generator is declined: a live generator frame may hold the store, and
//! the reset would empty the value the frame was handed (rustc tested that at run time,
//! `is_frame_held`; the IR has no such test).
//!
//! `LOFT_NO_LOOP_BUFFER_REUSE=1` turns the rule off on every backend;
//! `LOFT_TRACE_IR_LOOP_BUFFER=1` names each buffer kept.
use crate::data::{Data, DefType, Type, Value, v_block, v_if};
use crate::database::Stores;
use crate::opt::hoist;
use std::collections::HashSet;

fn enabled() -> bool {
    crate::keys::loop_buffer_reuse_enabled()
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_IR_LOOP_BUFFER").is_ok_and(|v| v != "0"))
}

struct Ops {
    database: [u32; 2],
    get_field: u32,
    set_int4: u32,
    ref_is_null: u32,
    clear_vector: u32,
}

/// Rewrite every admitted buffer of the program; answers how many.
pub fn rewrite_program(data: &mut Data, stores: &Stores) -> usize {
    if !enabled() {
        return 0;
    }
    let ops = Ops {
        database: [data.def_nr("OpDatabase"), data.def_nr("OpDatabaseNP")],
        get_field: data.def_nr("OpGetField"),
        set_int4: data.def_nr("OpSetInt4"),
        ref_is_null: data.def_nr("OpRefIsNull"),
        clear_vector: data.def_nr("OpClearVector"),
    };
    if [
        ops.database[0],
        ops.get_field,
        ops.set_int4,
        ops.ref_is_null,
        ops.clear_vector,
    ]
    .contains(&u32::MAX)
    {
        return 0;
    }
    let apr = crate::opt::apart_records::plan(data, stores);
    let mut total = 0usize;
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function
            || matches!(def.code(), Value::Null)
            || def.is_operator()
            || !def.rust().is_empty()
        // A native NAME is no reason to skip: a library export or a compiled stdlib
        // function keeps the loft body a native build emits.
        {
            continue;
        }
        if calls_a_generator(data, def.code()) {
            if trace() && !hoist::loop_buffers(data, stores, d).is_empty() {
                eprintln!(
                    "[ir-loop-buffer] fn={} declined: it calls a generator",
                    def.name()
                );
            }
            continue;
        }
        let mut buffers = hoist::loop_buffers(data, stores, d);
        if buffers.is_empty() {
            continue;
        }
        leave_owned(data, stores, d, &apr, &mut buffers);
        if buffers.is_empty() {
            continue;
        }
        let _census = crate::rewrite_census::InBody::enter("ir", def.name());
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        let mut done: HashSet<u16> = HashSet::new();
        rewrite(&mut code, &buffers, &ops, &mut done);
        data.definitions[d as usize].code = code;
        if done.is_empty() {
            continue;
        }
        data.definitions[d as usize].variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, d);
        crate::scopes::assign_function_slots(data, d);
        if trace() {
            let vars = data.def(d).variables();
            for v in &done {
                eprintln!(
                    "[ir-loop-buffer] fn={} {} kept across the passes",
                    data.def(d).name(),
                    vars.name(*v)
                );
            }
        }
        crate::rewrite_census::fired("R-LoopBuffer", done.len());
        total += done.len();
    }
    total
}

/// Leave every buffer another rewrite owns (`opt::buffer_owner`, the one answer).
fn leave_owned(
    data: &Data,
    stores: &Stores,
    d: u32,
    apr: &crate::opt::apart_records::RecPlan,
    buffers: &mut HashSet<u16>,
) {
    let owners = crate::opt::buffer_owner::owners(data, stores, d, apr);
    buffers.retain(|v| {
        let Some(owner) = owners.get(v) else {
            return true;
        };
        if trace() {
            eprintln!(
                "[ir-loop-buffer] fn={} {} left: {}",
                data.def(d).name(),
                data.def(d).variables().name(*v),
                owner.why()
            );
        }
        false
    });
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

/// Rewrite the declarations of `buffers` in every block under `v`.
fn rewrite(v: &mut Value, buffers: &HashSet<u16>, ops: &Ops, done: &mut HashSet<u16>) {
    if let Value::Block(bl) | Value::Loop(bl) = v.unspan_mut() {
        let old = std::mem::take(&mut bl.operators);
        let mut out: Vec<Value> = Vec::with_capacity(old.len());
        let mut i = 0;
        while i < old.len() {
            i += declaration(&old, i, buffers, ops, done, &mut out);
        }
        bl.operators = out;
    }
    v.for_each_child_mut(&mut |c| rewrite(c, buffers, ops, done));
}

/// Emit `stmts[at]` — or, when it opens an admitted buffer's declaration, the kept form of the
/// whole declaration — into `out`; answers how many statements it consumed.
fn declaration(
    stmts: &[Value],
    at: usize,
    buffers: &HashSet<u16>,
    ops: &Ops,
    done: &mut HashSet<u16>,
    out: &mut Vec<Value>,
) -> usize {
    let stmt = &stmts[at];
    let Value::Call(db_op, db_args) = stmt.unspan() else {
        out.push(stmt.clone());
        return 1;
    };
    let Some(vdb) = var_of(db_args.first()) else {
        out.push(stmt.clone());
        return 1;
    };
    if !ops.database.contains(db_op) || !buffers.contains(&vdb) {
        out.push(stmt.clone());
        return 1;
    }
    // `t = OpGetField(vdb, 0, ftp)` next, then the length reset `OpSetInt4(vdb, 0, 0)`.
    let bind = stmts.get(at + 1).map(Value::unspan);
    let Some(Value::Set(_, gf)) = bind else {
        out.push(stmt.clone());
        return 1;
    };
    let Value::Call(gop, gargs) = gf.unspan() else {
        out.push(stmt.clone());
        return 1;
    };
    if *gop != ops.get_field
        || var_of(gargs.first()) != Some(vdb)
        || int_of(gargs.get(1)) != Some(0)
    {
        out.push(stmt.clone());
        return 1;
    }
    let reset = stmts.get(at + 2).is_some_and(|r| {
        matches!(r.unspan(), Value::Call(op, ra) if *op == ops.set_int4
            && var_of(ra.first()) == Some(vdb)
            && int_of(ra.get(1)) == Some(0)
            && int_of(ra.get(2)) == Some(0))
    });
    let mint = vec![
        stmt.clone(),
        Value::Call(
            ops.set_int4,
            vec![Value::Var(vdb), Value::Int(0), Value::Int(0)],
        ),
    ];
    let clear = vec![Value::Call(ops.clear_vector, vec![(**gf).clone()])];
    out.push(v_if(
        Value::Call(ops.ref_is_null, vec![Value::Var(vdb)]),
        v_block(mint, Type::Void, "loop buffer mint"),
        v_block(clear, Type::Void, "loop buffer reset"),
    ));
    out.push(stmts[at + 1].clone());
    done.insert(vdb);
    if reset { 3 } else { 2 }
}
