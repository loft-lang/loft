// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! VALUE STRUCT copies: a `value struct` view read into a fresh copy where its backing may
//! change, and left a zero-cost view where it cannot.

use crate::data::{Data, DefType, Deps, Type, Value, v_if};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// @PLN101 — ISOLATED value-struct copy pass. A `value struct` is an ordinary
/// `Type::Reference` record (marked only by `Data.value_structs`); the ONLY behavioural
/// difference is value (copy) semantics. When such a struct is BOUND to a local from a VIEW
/// (a field/element read — `e = record.f` / `e = vec[i]`), rewrite the bind to mint a fresh
/// store and deep-copy into it, so the local owns its own record and mutating it cannot write
/// back through the view. Not wired into the type system: no `Type` variant, no assignment-path
/// surgery, no ownership-oracle change. Runs after `move_elide` and BEFORE the ownership scan,
/// so the emitted `OpDatabase` makes the local classify `Owned` on its own (sound: it earns
/// ownership, it is not asserted). Only fires on a `Set(local, borrowed-view)`; a fresh
/// construction (`Owned` rhs) or a method-call arg is not rewritten (copy-elision / zero-copy
/// hot path fall out).
pub(super) fn value_struct_copy(data: &mut Data) {
    // No value structs in the program → the pass is a no-op (and the read-only-elision oracles
    // below need not run for any function).
    if data.value_structs.is_empty() {
        return;
    }
    let op_database = data.def_nr("OpDatabase");
    let op_copy_record = data.def_nr("OpCopyRecord");
    let op_conv_bool_from_ref = data.def_nr("OpConvBoolFromRef");
    if op_database == u32::MAX || op_copy_record == u32::MAX || op_conv_bool_from_ref == u32::MAX {
        return;
    }
    let ops = VsCopyOps {
        database: op_database,
        copy_record: op_copy_record,
        conv_bool_from_ref: op_conv_bool_from_ref,
    };
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let mut code = data.definitions[d_nr as usize].code.clone();
        // The walk needs `&mut Data` to mint its source temp, so the taint oracle's view of the
        // body is taken HERE, once, rather than re-borrowed inside it.
        let body_snapshot = code.clone();
        // Read-only-elision oracle (function scope; rescoped per loop body inside the walk): the
        // TAINTED set — variables whose backing may be mutated, or which escape, during a view's
        // lifetime. Seeded from field/element writes (`find_field_written_vars`, catches nested
        // `v.a.b = …`) plus escapes (return / passed to a user fn), then closed over pure-`Var`
        // alias edges so mutating one co-alias taints the shared backing. A value-struct view-bind
        // is left as a zero-cost view iff neither the local nor any base variable is tainted.
        let tainted = vs_scope_taint(std::slice::from_ref(&code), data);
        let mut cleared: Vec<u16> = Vec::new();
        vs_copy_walk(
            &mut code,
            data,
            d_nr,
            ops,
            &tainted,
            &mut cleared,
            &body_snapshot,
        );
        if cleared.is_empty() {
            continue;
        }
        data.definitions[d_nr as usize].code = code;
        // Clear each copied local's VIEW dep so it frees as an owned store (no leak, no
        // double-free): the local now owns a fresh copy, not a borrow into the source.
        for v in cleared {
            if let Type::Reference(p, _) = *data.def(d_nr).variables.tp(v) {
                data.definitions[d_nr as usize]
                    .variables
                    .set_type(v, Type::Reference(p, Deps::none()));
            }
            // …and the `skip_free` mark with it.  The PARSER sets it on a `??` temp whose
            // subject is a borrowed place read — right at the time, because a borrow must not
            // free what it points at.  This pass then makes that same temp an OWNER, and the
            // stale mark suppressed the free of the store it now owns: `(v[0] ?? d)` on a
            // value struct leaked one record per evaluation, silently, on both backends.
            //
            // The dep and the mark are two spellings of ONE fact — "does this variable own its
            // store?" — so the site that changes the first owes the second.  Clearing them
            // together here is what keeps them from drifting again.
            data.definitions[d_nr as usize].variables.clear_skip_free(v);
        }
    }
}

/// @PLN101 zero-cost — root variable a VIEW projects from: follow the arg-0 spine of read ops
/// (`OpGet*`) down to a `Var`. Index/offset args are ignored (a mutated loop counter is not a base
/// mutation). `None` if the spine doesn't bottom out in a plain variable.
fn vs_view_root(node: &Value, data: &Data) -> Option<u16> {
    match node.unspan() {
        Value::Var(v) => Some(*v),
        Value::Call(fn_nr, args)
            if !args.is_empty() && data.def(*fn_nr).name().starts_with("OpGet") =>
        {
            vs_view_root(&args[0], data)
        }
        // A view can be wrapped in a block that yields its tail value — notably a for-loop's
        // `{#iter next}` (advance the index, then project the element). Follow the tail.
        Value::Block(b) => b.operators.last().and_then(|t| vs_view_root(t, data)),
        Value::Insert(ops) => ops.last().and_then(|t| vs_view_root(t, data)),
        _ => None,
    }
}

/// First rhs of `Set(var, rhs)` for `var` (its binding), for one-hop base-alias tracing.
fn vs_find_binding(code: &Value, var: u16) -> Option<&Value> {
    fn walk<'a>(node: &'a Value, var: u16, found: &mut Option<&'a Value>) {
        if found.is_some() {
            return;
        }
        match node {
            Value::Set(v, body) => {
                if *v == var {
                    *found = Some(body);
                } else {
                    walk(body, var, found);
                }
            }
            Value::Block(b) | Value::Loop(b) => {
                for op in &b.operators {
                    walk(op, var, found);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    walk(op, var, found);
                }
            }
            Value::If(c, t, e) => {
                walk(c, var, found);
                walk(t, var, found);
                walk(e, var, found);
            }
            Value::Return(x) | Value::Drop(x) => walk(x, var, found),
            Value::Call(_, args) => {
                for a in args {
                    walk(a, var, found);
                }
            }
            Value::Iter(_, a, b, c) => {
                walk(a, var, found);
                walk(b, var, found);
                walk(c, var, found);
            }
            Value::Span(b) => walk(&b.1, var, found),
            _ => {}
        }
    }
    let mut found = None;
    walk(code, var, &mut found);
    found
}

/// The variables whose backing a value-struct view reads from: the spine root plus, one hop at a
/// time, the root of whatever each is bound from (`_vector_1 = b.items` → also `b`), so an alias of
/// a mutated source is caught. Bounded to a few hops (straight-line binds, no cycles).
fn vs_base_vars(rhs: &Value, data: &Data, code: &Value, out: &mut HashSet<u16>) {
    let mut work: Vec<u16> = Vec::new();
    if let Some(r) = vs_view_root(rhs, data) {
        work.push(r);
    }
    let mut hops = 0;
    while let Some(v) = work.pop() {
        if !out.insert(v) {
            continue;
        }
        hops += 1;
        if hops > 8 {
            break;
        }
        if let Some(bind) = vs_find_binding(code, v)
            && let Some(r) = vs_view_root(bind, data)
        {
            work.push(r);
        }
    }
}

/// Walk the function collecting the TAINT seed + pure-`Var` alias edges for read-only elision.
/// A variable is a taint SOURCE if it ESCAPES — returned, or handed as a bare `Var` argument to a
/// user function (non-`Op`), which could store or mutate it. (Field/element mutations are folded in
/// separately from `find_field_written_vars`; a store like `coll.push(p)` copies `p` and so does not
/// taint it.) An `w = x` bind adds a symmetric alias EDGE so that mutating one co-alias taints the
/// backing both share. The caller closes `seed` over `edges` to get the full tainted set.
fn vs_collect_taint(
    node: &Value,
    data: &Data,
    seed: &mut HashSet<u16>,
    edges: &mut Vec<(u16, u16)>,
) {
    match node {
        Value::Set(w, body) => {
            if let Value::Var(x) = body.unspan() {
                edges.push((*w, *x)); // pure alias `w = x` — taint flows both ways
            }
            vs_collect_taint(body, data, seed, edges);
        }
        Value::Return(x) | Value::Drop(x) => {
            if let Value::Var(v) = x.unspan() {
                seed.insert(*v); // escape via return
            }
            vs_collect_taint(x, data, seed, edges);
        }
        Value::Call(fn_nr, args) => {
            let is_op = data.def(*fn_nr).name().starts_with("Op");
            for a in args {
                // A bare `Var` passed to a USER function may be stored or mutated by the callee.
                if !is_op && let Value::Var(v) = a.unspan() {
                    seed.insert(*v);
                }
                vs_collect_taint(a, data, seed, edges);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                vs_collect_taint(op, data, seed, edges);
            }
        }
        Value::Insert(ops) => {
            for op in ops {
                vs_collect_taint(op, data, seed, edges);
            }
        }
        Value::If(c, t, e) => {
            vs_collect_taint(c, data, seed, edges);
            vs_collect_taint(t, data, seed, edges);
            vs_collect_taint(e, data, seed, edges);
        }
        Value::Iter(_, a, b, c) => {
            vs_collect_taint(a, data, seed, edges);
            vs_collect_taint(b, data, seed, edges);
            vs_collect_taint(c, data, seed, edges);
        }
        Value::Span(b) => vs_collect_taint(&b.1, data, seed, edges),
        _ => {}
    }
}

/// Close `seed` under the symmetric alias `edges` — the set of variables whose backing may change
/// or escape during a view's lifetime. A value-struct view-bind is elidable iff neither the local
/// nor any base variable is in this set.
fn vs_tainted(seed: HashSet<u16>, edges: &[(u16, u16)]) -> HashSet<u16> {
    let mut adj: HashMap<u16, Vec<u16>> = HashMap::default();
    for &(a, b) in edges {
        adj.entry(a).or_default().push(b);
        adj.entry(b).or_default().push(a);
    }
    let mut tainted = seed;
    let mut work: Vec<u16> = tainted.iter().copied().collect();
    while let Some(v) = work.pop() {
        if let Some(ns) = adj.get(&v) {
            for &n in ns {
                if tainted.insert(n) {
                    work.push(n);
                }
            }
        }
    }
    tainted
}

/// The tainted set over a straight-line SCOPE (a function body, or a loop body). Scoping to the
/// enclosing loop is what makes a `for p in ps { …read p… }` bind elidable: BUILDING `ps` field-
/// writes it, but that construction runs BEFORE the loop, so it cannot diverge a view read INSIDE
/// the loop from a copy. Only a mutation/escape WITHIN the loop (which repeats) can — S1's
/// `b.items[0].x = 99` in the body still taints `b`.
fn vs_scope_taint(ops: &[Value], data: &Data) -> HashSet<u16> {
    let mut seed: HashSet<u16> = HashSet::default();
    let mut edges: Vec<(u16, u16)> = Vec::new();
    for op in ops {
        crate::parser::find_field_written_vars(op, data, &mut seed);
        vs_collect_taint(op, data, &mut seed, &mut edges);
    }
    vs_tainted(seed, &edges)
}

/// Recursive rewriter for [`value_struct_copy`] — mirrors the IR walk in
/// `variables/validate.rs::build_scope_parents`. `tainted` is the function-wide read-only-elision
/// oracle (computed once by the caller): a value-struct view-bind is left as a zero-cost view (like
/// a reference struct) when neither the local nor any base variable is tainted; otherwise the copy
/// is emitted for value semantics.
/// The three opcode def-numbers `vs_copy_walk` emits, resolved once by its caller.
///
/// Bundled because they travel together through every arm of the walk and are meaningless
/// apart: `OpDatabase` allocates the copy's store, `OpCopyRecord` fills it, and
/// `OpConvBoolFromRef` is the presence test that decides whether either runs at all
/// (`@FR-B-Copy` over `@FR-L-Null` — a copy of an absent value is absent).
#[derive(Clone, Copy)]
struct VsCopyOps {
    database: u32,
    copy_record: u32,
    conv_bool_from_ref: u32,
}

fn vs_copy_walk(
    node: &mut Value,
    data: &mut Data,
    d_nr: u32,
    ops: VsCopyOps,
    tainted: &HashSet<u16>,
    cleared: &mut Vec<u16>,
    body: &Value,
) {
    match node {
        Value::Set(v, rhs) => {
            let vv = *v;
            vs_copy_walk(rhs, data, d_nr, ops, tainted, cleared, body);
            if let Type::Reference(p, _) = *data.def(d_nr).variables.tp(vv)
                && data.is_value_struct(p)
                && matches!(
                    crate::use_analysis::ownership_of(data, d_nr, rhs),
                    crate::use_analysis::Own::Borrowed { .. }
                )
            {
                // Zero-cost read-only elision: if the local and its whole projection base are only
                // ever read (never field-written, never escaping), a plain view is observably
                // identical to a copy — so skip the copy and keep the reference-struct-cheap view.
                let mut affected: HashSet<u16> = HashSet::default();
                affected.insert(vv);
                vs_base_vars(rhs, data, body, &mut affected);
                let needs_copy = affected.iter().any(|x| tainted.contains(x));
                if !needs_copy {
                    return;
                }
                let kt = i32::from(data.def(p).known_type());
                let source = (**rhs).clone();
                // ⚠ The materialisation must preserve ABSENCE.  Emitted unconditionally — null
                // the destination, ALLOCATE, copy — it did not: `OpCopyRecord` from a null
                // source "does nothing at all" (its own documented behaviour), so the
                // destination was left ALLOCATED and zeroed and every downstream null test
                // answered present.  A `??` on the copied view then never ran its default and an
                // absent value read back as a zero record, on both backends (loft#1472).
                //
                // A copy of an absent value is absent (`@FR-B-Copy` over `@FR-L-Null`), so the
                // allocate-and-copy moves under a presence test.  The source is bound to its own
                // temp FIRST because it must be evaluated exactly once: it is a place read, but
                // the index inside it can be an arbitrary expression, and splicing it into both
                // the test and the copy would run `v[f()]`'s `f` twice.
                //
                // The temp is a VIEW — it carries the source's deps, so the ownership scan
                // classifies it `Borrowed` and emits no free for it; only `vv` owns, exactly as
                // before.  Named after the destination so the mint is stable and idempotent.
                let view_type = data.def(d_nr).variables.tp(vv).clone();
                let src_name = format!("__vs_src_{vv}");
                let src_tmp = data.definitions[d_nr as usize]
                    .variables
                    .add_temp_var(&src_name, &view_type);
                *node = Value::Insert(vec![
                    Value::Set(src_tmp, Box::new(source)),
                    Value::Set(vv, Box::new(Value::Null)),
                    v_if(
                        Value::Call(ops.conv_bool_from_ref, vec![Value::Var(src_tmp)]),
                        Value::Insert(vec![
                            Value::Call(ops.database, vec![Value::Var(vv), Value::Int(kt)]),
                            Value::Call(
                                ops.copy_record,
                                vec![Value::Var(src_tmp), Value::Var(vv), Value::Int(kt)],
                            ),
                        ]),
                        Value::Null,
                    ),
                ]);
                cleared.push(vv);
            }
        }
        Value::Block(b) => {
            for op in &mut b.operators {
                vs_copy_walk(op, data, d_nr, ops, tainted, cleared, body);
            }
        }
        Value::Loop(b) => {
            // Rescope taint to this loop body — pre-loop construction of a base does not diverge a
            // view read inside the loop, only an in-body mutation/escape does.
            let loop_taint = vs_scope_taint(&b.operators, data);
            for op in &mut b.operators {
                vs_copy_walk(op, data, d_nr, ops, &loop_taint, cleared, body);
            }
        }
        Value::Insert(items) => {
            for item in items {
                vs_copy_walk(item, data, d_nr, ops, tainted, cleared, body);
            }
        }
        Value::If(c, t, e) => {
            vs_copy_walk(c, data, d_nr, ops, tainted, cleared, body);
            vs_copy_walk(t, data, d_nr, ops, tainted, cleared, body);
            vs_copy_walk(e, data, d_nr, ops, tainted, cleared, body);
        }
        Value::Return(x) | Value::Drop(x) => {
            vs_copy_walk(x, data, d_nr, ops, tainted, cleared, body);
        }
        Value::Call(_, args) => {
            for a in args {
                vs_copy_walk(a, data, d_nr, ops, tainted, cleared, body);
            }
        }
        Value::Iter(_, a, b, c) => {
            vs_copy_walk(a, data, d_nr, ops, tainted, cleared, body);
            vs_copy_walk(b, data, d_nr, ops, tainted, cleared, body);
            vs_copy_walk(c, data, d_nr, ops, tainted, cleared, body);
        }
        Value::Span(b) => {
            vs_copy_walk(&mut b.1, data, d_nr, ops, tainted, cleared, body);
        }
        _ => {}
    }
}
