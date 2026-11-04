// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the analyses every backend asks
//! `@FR-R-ElemFirst` in the IR phase: a local vector built by a literal and consumed once as a
//! field of an appended record (`t = []; …t += …; out += [R { f: t, … }]`) is built INSIDE the
//! appended element.  The pairing is `opt::hoist::element_first`'s, the analysis the writers
//! read; this writes its answer into the IR, so every backend runs it:
//!
//! * at the first temp's declaration, the append's reservation and mint
//!   (`OpPreAllocVector(out, 1, size) · elm = OpNewRecord(out, tp, fld)`) and a zero for every
//!   paired field's vector handle;
//! * each temp's declaration (`OpDatabase(vdb) · t = OpGetField(vdb, 0, ftp) · OpSetInt4(vdb,
//!   0, 0)`) becomes `t = OpGetField(elm, off, ftp)` — the temp IS the element's field;
//! * at the append, the reservation, the mint, the paired zeros and the paired
//!   `OpAppendVector` copies go; the scalar sets and the finish stay, so the element is
//!   invisible until the finish exactly as before.
//!
//! Also a temp a CALL fills (its lazy-buffer guard becomes the mint, and the call is handed the
//! element's field for its buffer), an append in EACH arm of an `if` (each arm's element is
//! renamed to the one minted at the declaration), and a comprehension's row appended as a whole
//! element (its declaration inside the comprehension, the whole-row `OpCopyRecord` dropped).  A
//! function whose exits copy a temp (`@FR-O-ViewField`, which needs the writers' view plan) is
//! still the writers' (they ask the same analysis of the IR they receive, which no longer shows
//! a pair this pass took).
//!
//! `LOFT_NO_ELEMENT_FIRST=1` turns the rule off on every backend; `LOFT_NO_IR_ELEM_FIRST=1`
//! this clause alone, its pairs then left to the writers; `LOFT_TRACE_IR_ELEM_FIRST=1` names
//! each pair and its destination.
use crate::data::{Data, DefType, Value};
use crate::database::Stores;
use crate::opt::hoist::{ElemFirst, element_first};
use std::collections::HashMap;

fn enabled() -> bool {
    !crate::env_once!(std::env::var("LOFT_NO_ELEMENT_FIRST").is_ok_and(|v| v != "0"))
        && !crate::env_once!(std::env::var("LOFT_NO_IR_ELEM_FIRST").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_IR_ELEM_FIRST").is_ok_and(|v| v != "0"))
}

struct Ops {
    database: u32,
    database_np: u32,
    copy_record: u32,
    get_field: u32,
    set_int4: u32,
    pre_alloc: u32,
    new_record: u32,
    append_vector: u32,
    ref_is_null: u32,
}

/// What one function's rewrite does, keyed by variable.
#[derive(Default)]
struct Plan {
    /// A literal-built temp's `__vdb` witness → (pair, bind).
    decl: HashMap<u16, (usize, usize)>,
    /// A call-filled temp's hidden buffer → (pair, bind), and the field type its copy names.
    call_buf: HashMap<u16, (usize, usize, Value)>,
    /// An element variable — the pair's own, or another arm's alias of it — → its pair.
    elm: HashMap<u16, usize>,
    /// The comprehension clause's `(block value, element)` pairs: the row was built in the
    /// element, so its whole-row `OpCopyRecord(c, elm, tp)` goes.
    whole_copies: std::collections::HashSet<(u16, u16)>,
}

/// Rewrite every admitted pair of the program; answers how many.
///
/// # Panics
/// When the analysis paired an append whose temp's declaration the rewrite does not find — a
/// defect of this pass, reported with the switch that keeps the copy.
pub fn rewrite_program(data: &mut Data, stores: &Stores) -> usize {
    if !enabled() {
        return 0;
    }
    let ops = Ops {
        database: data.def_nr("OpDatabase"),
        database_np: data.def_nr("OpDatabaseNP"),
        copy_record: data.def_nr("OpCopyRecord"),
        get_field: data.def_nr("OpGetField"),
        set_int4: data.def_nr("OpSetInt4"),
        pre_alloc: data.def_nr("OpPreAllocVector"),
        new_record: data.def_nr("OpNewRecord"),
        append_vector: data.def_nr("OpAppendVector"),
        ref_is_null: data.def_nr("OpRefIsNull"),
    };
    if [
        ops.ref_is_null,
        ops.database,
        ops.get_field,
        ops.set_int4,
        ops.pre_alloc,
        ops.new_record,
        ops.append_vector,
    ]
    .contains(&u32::MAX)
    {
        return 0;
    }
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
        let mut appends = false;
        def.code().walk(&mut |n| {
            // Every pair's element is a mint the append files; a comprehension row reaches it
            // through `OpCopyRecord`, not `OpAppendVector`.
            if matches!(n, Value::Call(c, _) if *c == ops.new_record) {
                appends = true;
            }
        });
        if !appends {
            continue;
        }
        let map = element_first(data, stores, d, None);
        let pairs: Vec<&ElemFirst> = map.pairs.iter().collect();
        let whole_copies = map.whole_copies.clone();
        if pairs.is_empty() {
            continue;
        }
        let Some(mut plan) = plan_of(def.code(), &pairs, &ops) else {
            continue;
        };
        plan.whole_copies = whole_copies;
        let _census = crate::rewrite_census::InBody::enter("ir", def.name());
        let mut code = std::mem::replace(&mut data.definitions[d as usize].code, Value::Null);
        merge_arms(&mut code, &pairs);
        let mut minted = vec![false; pairs.len()];
        rewrite(&mut code, &pairs, &plan, &ops, &mut minted);
        data.definitions[d as usize].code = code;
        let n = minted.iter().filter(|m| **m).count();
        assert_eq!(
            n,
            pairs.len(),
            "internal compiler error — (R-ElemFirst) paired an append in `{}` whose temp's \
             declaration the IR form did not find; LOFT_NO_ELEMENT_FIRST=1 keeps the copy",
            data.def(d).name()
        );
        data.definitions[d as usize].variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, d);
        crate::scopes::assign_function_slots(data, d);
        if trace() {
            // One line per pair, naming its destination: the vector itself, or a record's
            // collection field (the emission pins count them so).
            for p in &pairs {
                let dest = if p.out_fld == 65535 { "local" } else { "field" };
                eprintln!("[ir-elem-first] fn={} pair {dest}", data.def(d).name());
            }
        }
        crate::rewrite_census::fired("R-ElemFirst", n);
        total += n;
    }
    total
}

/// What the rewrite does with `pairs` in `code`, keyed by variable; `None` when a call-filled
/// temp's copy names no field type, so its pair cannot be bound and every pair keeps the
/// writers' form.
fn plan_of(code: &Value, pairs: &[&ElemFirst], ops: &Ops) -> Option<Plan> {
    let mut plan = Plan::default();
    for (pi, p) in pairs.iter().enumerate() {
        plan.elm.insert(p.elm, pi);
        for a in &p.aliases {
            plan.elm.insert(*a, pi);
        }
        for (bi, b) in p.binds.iter().enumerate() {
            if b.from_call {
                let Some(ftp) = copy_field_type(code, ops, &plan, pi, b) else {
                    continue;
                };
                plan.call_buf.insert(b.vdb, (pi, bi, ftp));
            } else {
                plan.decl.insert(b.vdb, (pi, bi));
            }
        }
    }
    // A call-filled temp whose copy names no field type cannot be bound: its pair keeps
    // the writers' form.
    let complete: Vec<bool> = pairs
        .iter()
        .map(|p| {
            p.binds
                .iter()
                .all(|b| !b.from_call || plan.call_buf.contains_key(&b.vdb))
        })
        .collect();
    if complete.iter().any(|c| !c) {
        return None;
    }
    Some(plan)
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

/// Rewrite the statements of every block under `v`.
fn rewrite(v: &mut Value, pairs: &[&ElemFirst], plan: &Plan, ops: &Ops, minted: &mut [bool]) {
    if let Value::Block(bl) | Value::Loop(bl) = v.unspan_mut() {
        let old = std::mem::take(&mut bl.operators);
        let mut out: Vec<Value> = Vec::with_capacity(old.len());
        let mut i = 0;
        while i < old.len() {
            i += statement(&old, i, pairs, plan, ops, minted, &mut out);
        }
        bl.operators = out;
    }
    v.for_each_child_mut(&mut |c| rewrite(c, pairs, plan, ops, minted));
}

/// Emit `ops[i]` (or what replaces it) into `out`; answers how many statements it consumed.
#[allow(clippy::too_many_arguments)]
fn statement(
    stmts: &[Value],
    at: usize,
    pairs: &[&ElemFirst],
    plan: &Plan,
    ops: &Ops,
    minted: &mut [bool],
    out: &mut Vec<Value>,
) -> usize {
    let stmt = &stmts[at];
    match stmt.unspan() {
        // A temp's declaration.
        Value::Call(op, args) if *op == ops.database || *op == ops.database_np => {
            declaration(stmts, at, args, pairs, plan, ops, minted, out)
        }
        // The append's reservation, when the next statement is a paired mint.
        Value::Call(op, args) if *op == ops.pre_alloc && int_of(args.get(1)) == Some(1) => {
            let next = stmts[at + 1..]
                .iter()
                .find(|o| !matches!(o.unspan(), Value::Line(_)));
            let paired = next.is_some_and(|n| {
                matches!(n.unspan(), Value::Set(elm_var, mint) if plan.elm.contains_key(elm_var)
                    && matches!(mint.unspan(), Value::Call(md, ma) if *md == ops.new_record
                        && var_of(ma.first()) == var_of(args.first())))
            });
            if !paired {
                out.push(stmt.clone());
            }
            1
        }
        // A call-filled temp's lazy buffer guard: the buffer is never minted, and the element
        // is, when this temp carries the mint.
        Value::If(cond, _, _)
            if matches!(cond.unspan(), Value::Call(op, ca) if *op == ops.ref_is_null
                && var_of(ca.first()).is_some_and(|b| plan.call_buf.contains_key(&b))) =>
        {
            if let Value::Call(_, ca) = cond.unspan()
                && let Some(&(pi, bi, _)) = var_of(ca.first()).and_then(|b| plan.call_buf.get(&b))
                && pairs[pi].binds[bi].first
            {
                mint(pairs[pi], ops, out);
                minted[pi] = true;
            }
            1
        }
        // The call that fills the temp: handed the element's field in place of its buffer.
        Value::Set(tmp, _)
            if plan.call_buf.iter().any(|(buf, (pi, bi, _))| {
                pairs[*pi].binds[*bi].tmp == *tmp && stmt.reads_var(*buf)
            }) =>
        {
            let mut s2 = stmt.clone();
            for (buf, (pi, bi, ftp)) in &plan.call_buf {
                let b = &pairs[*pi].binds[*bi];
                if b.tmp == *tmp {
                    let place = Value::Call(
                        ops.get_field,
                        vec![
                            Value::Var(pairs[*pi].elm),
                            Value::Int(b.field_off),
                            ftp.clone(),
                        ],
                    );
                    if let Value::Set(_, val) = s2.unspan_mut() {
                        replace_var(val, *buf, &place);
                    }
                }
            }
            out.push(s2);
            1
        }
        // The append's mint and the element's null pre-init (it would reset the element the
        // declaration minted) — and, for another arm of a joined append, that arm's own
        // element's: its uses name the minted element already ([`merge_arms`]).
        Value::Set(elm_var, mint)
            if plan.elm.contains_key(elm_var)
                && (matches!(mint.unspan(), Value::Null)
                    || matches!(mint.unspan(), Value::Call(md, _) if *md == ops.new_record)) =>
        {
            1
        }
        // The paired zeros on the element.
        Value::Call(op, args)
            if *op == ops.set_int4
                && var_of(args.first())
                    .and_then(|elm_var| plan.elm.get(&elm_var))
                    .is_some_and(|&pi| {
                        int_of(args.get(1))
                            .is_some_and(|off| pairs[pi].binds.iter().any(|b| b.field_off == off))
                    }) =>
        {
            1
        }
        // The comprehension clause's whole-row copy: the row was built in the element.
        Value::Call(op, args)
            if *op == ops.copy_record
                && var_of(args.first())
                    .zip(var_of(args.get(1)))
                    .is_some_and(|ce| plan.whole_copies.contains(&ce)) =>
        {
            1
        }
        // The paired copies into the element's field.
        Value::Call(op, args)
            if *op == ops.append_vector
                && matches!(args.first().map(Value::unspan), Some(Value::Call(gd, ga))
                if *gd == ops.get_field
                    && var_of(ga.first())
                        .and_then(|elm_var| plan.elm.get(&elm_var))
                        .is_some_and(|&pi| int_of(ga.get(1)).is_some_and(|off| {
                            pairs[pi].binds.iter().any(|b| b.field_off == off)
                        }))) =>
        {
            1
        }
        _ => {
            out.push(stmt.clone());
            1
        }
    }
}

/// The append's reservation and mint at the first temp's declaration, and a zero for every
/// paired field's vector handle (a fresh C slot is not zeroed, so the zero is the IR's).
fn mint(p: &ElemFirst, ops: &Ops, out: &mut Vec<Value>) {
    if p.out_fld == 65535 {
        out.push(Value::Call(
            ops.pre_alloc,
            vec![
                Value::Var(p.out),
                Value::Int(1),
                Value::Int(p.prealloc_size),
            ],
        ));
    }
    out.push(Value::Set(
        p.elm,
        Box::new(Value::Call(
            ops.new_record,
            vec![
                Value::Var(p.out),
                Value::Int(p.out_tp),
                Value::Int(p.out_fld),
            ],
        )),
    ));
    for pb in &p.binds {
        out.push(Value::Call(
            ops.set_int4,
            vec![Value::Var(p.elm), Value::Int(pb.field_off), Value::Int(0)],
        ));
    }
}

/// The field type a call-filled temp's paired copy names — `OpAppendVector(OpGetField(e, off,
/// ftp), tmp, …)` with `e` the pair's element or an alias — which the call's field place uses.
fn copy_field_type(
    code: &Value,
    ops: &Ops,
    plan: &Plan,
    pi: usize,
    b: &crate::opt::hoist::ElemBind,
) -> Option<Value> {
    let mut ftp = None;
    code.walk(&mut |n| {
        if let Value::Call(op, args) = n
            && *op == ops.append_vector
            && var_of(args.get(1)) == Some(b.tmp)
            && let Some(Value::Call(gf, ga)) = args.first().map(Value::unspan)
            && *gf == ops.get_field
            && var_of(ga.first()).and_then(|e| plan.elm.get(&e)) == Some(&pi)
            && int_of(ga.get(1)) == Some(b.field_off)
        {
            ftp = ga.get(2).cloned();
        }
    });
    ftp
}

/// A joined append (@PLN164 E-2b): every arm's element is the one minted at the declaration.
/// The other arms' elements are not aliased — a `Set` of one record variable from another
/// binds a COPY (`(B-Copy)`), so the arm would write and file the copy — but renamed: each use
/// of an alias names the pair's element instead.  Its own mint and pre-init stay as `Set`s of
/// the alias, which [`statement`] drops.
fn merge_arms(code: &mut Value, pairs: &[&ElemFirst]) {
    for p in pairs {
        for a in &p.aliases {
            replace_var(code, *a, &Value::Var(p.elm));
        }
    }
}

/// `v` with every `Var(buf)` replaced by `place`.
fn replace_var(v: &mut Value, buf: u16, place: &Value) {
    if matches!(v.unspan(), Value::Var(x) if *x == buf) {
        *v = place.clone();
        return;
    }
    v.for_each_child_mut(&mut |c| replace_var(c, buf, place));
}

/// A temp's declaration, `OpDatabase(vdb, …) · t = OpGetField(vdb, 0, ftp) · OpSetInt4(vdb, 0,
/// 0)` with `stmt` its `OpDatabase`: the element's mint (the first temp), then the field-slot
/// bind.  Answers how many statements it consumed.
#[allow(clippy::too_many_arguments)]
fn declaration(
    stmts: &[Value],
    at: usize,
    args: &[Value],
    pairs: &[&ElemFirst],
    plan: &Plan,
    ops: &Ops,
    minted: &mut [bool],
    out: &mut Vec<Value>,
) -> usize {
    let stmt = &stmts[at];
    let Some(&(pi, bi)) = var_of(args.first()).and_then(|vdb| plan.decl.get(&vdb)) else {
        out.push(stmt.clone());
        return 1;
    };
    let p = pairs[pi];
    let b = &p.binds[bi];
    let vdb = b.vdb;
    let Some(Value::Set(tmp, bind)) = stmts.get(at + 1).map(Value::unspan) else {
        out.push(stmt.clone());
        return 1;
    };
    let Value::Call(gf, gargs) = bind.unspan() else {
        out.push(stmt.clone());
        return 1;
    };
    if *gf != ops.get_field || var_of(gargs.first()) != Some(vdb) || *tmp != b.tmp {
        out.push(stmt.clone());
        return 1;
    }
    let ftp = gargs.get(2).cloned().unwrap_or(Value::Int(0));
    if b.first {
        mint(p, ops, out);
        minted[pi] = true;
    }
    out.push(Value::Set(
        b.tmp,
        Box::new(Value::Call(
            ops.get_field,
            vec![Value::Var(p.elm), Value::Int(b.field_off), ftp],
        )),
    ));
    // The declaration's length reset on the witness goes with it.
    let reset = stmts.get(at + 2).is_some_and(|r| {
        matches!(r.unspan(), Value::Call(op, ra) if *op == ops.set_int4
            && var_of(ra.first()) == Some(vdb))
    });
    if reset { 3 } else { 2 }
}
