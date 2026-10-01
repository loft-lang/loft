// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! STORE CONFINEMENT: the block a vector store is provably confined to, so it registers — and
//! is freed — at that block's exit, and the guard that reports a store freed later than it could be.

use super::returns::is_value_return_type;
use crate::data::{Type, Value};
use crate::variables::Function;
use std::collections::BTreeMap;

/// True if `op` is the null-init `Set(vdb, Null)` — a work-ref's `first_def`.
pub(super) fn is_var_null_init(op: &Value, vdb: u16) -> bool {
    matches!(op.unspan(), Value::Set(v, val) if *v == vdb && matches!(val.unspan(), Value::Null))
}

/// Prepend `ni` to the operators of the Block whose `scope == target`, descending
/// only the control-flow spine (`Block`/`Insert`/`If`/`Span`/`Return`).  Returns
/// `None` once inserted, or `Some(ni)` (un-consumed) if no such block was reached.
/// A confined block nested inside a `Set`/`Call`/`Iter` value (a `map`/`filter`
/// body, a short-lambda capture) is deliberately NOT entered: the Plan-57 null-init
/// relocation is a best-effort watermark optimization, and such a block keeps its
/// body-0 null-init (the caller's fallback), which is leak/poison-clean.  Widening
/// the descent to every child would relocate into far more functions for marginal
/// benefit — out of scope here; the concern is only to stop the false-positive
/// debug assert on the (correct) un-reached case.
fn prepend_to_scope(node: &mut Value, target: u16, ni: Value) -> Option<Value> {
    match node {
        Value::Block(bl) if bl.scope == target => {
            bl.operators.insert(0, ni);
            None
        }
        Value::Block(bl) | Value::Loop(bl) => {
            let mut carry = Some(ni);
            for op in &mut bl.operators {
                carry = prepend_to_scope(op, target, carry.take().unwrap());
                carry.as_ref()?;
            }
            carry
        }
        Value::Insert(ls) => {
            let mut carry = Some(ni);
            for op in ls {
                carry = prepend_to_scope(op, target, carry.take().unwrap());
                carry.as_ref()?;
            }
            carry
        }
        Value::If(c, t, e) => {
            let ni = prepend_to_scope(c, target, ni)?;
            let ni = prepend_to_scope(t, target, ni)?;
            prepend_to_scope(e, target, ni)
        }
        Value::Span(b) => prepend_to_scope(&mut b.1, target, ni),
        // This relocation is BEST-EFFORT: where no arm reaches, the fallback — leaving the
        // null-init at body position 0 — is correct, so an unreached shape costs placement
        // quality and never correctness.  A confined block off the control-flow spine (a
        // `map`/`filter`/lambda body) is the standing example.
        Value::Return(b) | Value::Drop(b) | Value::Yield(b) => prepend_to_scope(b, target, ni),
        _ => Some(ni),
    }
}

/// Plan-57 cluster-I experiment: move a confined `__vdb`'s null-init
/// `Set(vdb, Null)` from body position 0 into its confined block, so the slot's
/// `first_def` (and therefore the SLOT, and codegen's free) live in the block —
/// not just the IR `OpFreeRef`.  The body-0 hoist (`parse_code`) was a
/// correctness over-reach made *without* lifetime info; the confinement analysis
/// now supplies that info, so the slot can live in its real scope.
pub(super) fn relocate_null_init(code: &mut Value, vdb: u16, block_scope: u16) -> bool {
    let ni = {
        let Value::Block(body) = code else {
            return false;
        };
        let Some(pos) = body
            .operators
            .iter()
            .position(|op| is_var_null_init(op, vdb))
        else {
            return false;
        };
        body.operators.remove(pos)
    };
    if let Some(ni) = prepend_to_scope(code, block_scope, ni) {
        // Not reached by the control-flow descent — restore the null-init so the
        // `first_def` is never lost, and skip the (best-effort) relocation: the
        // store keeps its body-0 `first_def` and is still freed by the confined
        // block's scope-exit sweep (verified leak/poison-clean, both backends).
        // The confined block can legitimately live inside a `map`/`filter` body or
        // a short-lambda capture (a `Call`/`Iter`/`Set` value `prepend_to_scope`
        // does not enter — 501, 85-short-lambda-capture).  Only a `block_scope`
        // that is ABSENT FROM THE IR ENTIRELY is a `store_confinement` bug worth
        // asserting; a present-but-unreached scope is the expected miss.
        if let Value::Block(body) = code {
            body.operators.insert(0, ni);
        }
        debug_assert!(
            block_scope_present(code, block_scope),
            "relocate_null_init: block scope {block_scope} is not in the IR at all \
             (a store_confinement bug, not merely an unreached inline block)"
        );
        false
    } else {
        true
    }
}

/// True if any `Block`/`Loop` anywhere in `node`'s subtree has `scope == target`.
/// Unlike [`prepend_to_scope`], this descends into EVERY child (`Value::walk`), so
/// it distinguishes a scope that is genuinely absent from one that merely lives off
/// the control-flow spine (inside a `map`/`filter` body, a lambda capture).  Only
/// consulted from the `relocate_null_init` `debug_assert!`, but must compile in
/// release too (the assert's argument is still type-checked there).
fn block_scope_present(node: &Value, target: u16) -> bool {
    let mut found = false;
    node.walk(&mut |n| {
        if let Value::Block(bl) | Value::Loop(bl) = n
            && bl.scope == target
        {
            found = true;
        }
    });
    found
}

// ── Plan-57 cluster I: store-lifetime guard (diagnostic) ─────────────────────
//
// Fires (under LOFT_STORE_GUARD) when a vector local's references are confined
// to one non-loop nested block, yet its backing `__vdb_N` store is scoped to an
// ancestor (function) and so frees late — the lifetime model under-freeing a
// block-confined store.  A detector for the watermark; once the model scopes
// such stores to their block it goes silent.  Read-only, gated, no behaviour
// change.
//
// Confinement = the least-common-ancestor of the block/loop scope-paths of every
// reference.  Tracking the full path (not just the innermost block) is required:
// a vector created in block B and read in a nested sub-block (nested `if`, a
// for-loop's `#For` block, or inside a loop body) is still confined to B — the
// LCA of `[B]` and `[B, sub]` is `B`.  Exact-scope-match misses these (probes
// 20/25/26).  The LCA's last element being a LOOP scope means the local lives
// only inside that loop (per-iteration reuse) → not relocatable.

/// Record a reference at the current scope-path: fold it into the running LCA
/// (longest common prefix of all reference paths).
fn guard_note(stack: &[(u16, bool)], lca: &mut Option<Vec<(u16, bool)>>) {
    *lca = Some(match lca.take() {
        None => stack.to_vec(),
        Some(prev) => prev
            .iter()
            .zip(stack)
            .take_while(|(a, b)| a == b)
            .map(|(a, _)| *a)
            .collect(),
    });
}

fn guard_refs(
    node: &Value,
    target: u16,
    free_ref_nr: u32,
    stack: &mut Vec<(u16, bool)>,
    lca: &mut Option<Vec<(u16, bool)>>,
) {
    match node {
        Value::Var(v) if *v == target => guard_note(stack, lca),
        Value::Set(v, val) => {
            if *v == target && !matches!(val.unspan(), Value::Null) {
                guard_note(stack, lca);
            }
            guard_refs(val, target, free_ref_nr, stack, lca);
        }
        Value::Call(op, args) => {
            if *op == free_ref_nr
                && args.len() == 1
                && matches!(args[0].unspan(), Value::Var(v) if *v == target)
            {
                return;
            }
            for a in args {
                guard_refs(a, target, free_ref_nr, stack, lca);
            }
        }
        Value::Block(bl) => {
            stack.push((bl.scope, false));
            for op in &bl.operators {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
            stack.pop();
        }
        Value::Loop(lp) => {
            stack.push((lp.scope, true));
            for op in &lp.operators {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
            stack.pop();
        }
        Value::If(t, a, b) => {
            guard_refs(t, target, free_ref_nr, stack, lca);
            guard_refs(a, target, free_ref_nr, stack, lca);
            guard_refs(b, target, free_ref_nr, stack, lca);
        }
        Value::Iter(idx, c, n, e) => {
            if *idx == target {
                guard_note(stack, lca);
            }
            guard_refs(c, target, free_ref_nr, stack, lca);
            guard_refs(n, target, free_ref_nr, stack, lca);
            guard_refs(e, target, free_ref_nr, stack, lca);
        }
        Value::CallRef(v, args) => {
            if *v == target {
                guard_note(stack, lca);
            }
            for a in args {
                guard_refs(a, target, free_ref_nr, stack, lca);
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            guard_refs(v, target, free_ref_nr, stack, lca)
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for op in ops {
                guard_refs(op, target, free_ref_nr, stack, lca);
            }
        }
        Value::TupleGet(v, _) if *v == target => guard_note(stack, lca),
        Value::TuplePut(v, _, inner) => {
            if *v == target {
                guard_note(stack, lca);
            }
            guard_refs(inner, target, free_ref_nr, stack, lca);
        }
        Value::Span(b) => guard_refs(&b.1, target, free_ref_nr, stack, lca),
        _ => {}
    }
}

/// True if `target` is handed out of the function as-is — directly returned,
/// yielded, or broken out of a loop (`Return(Var(target))` etc.).  Such a local
/// escapes; its store must NOT be freed at block exit (probe 30).  (Catches the
/// direct form; an escape buried in a sub-expression like `return if c { a }`
/// is left for the fix's full escape analysis.)
/// True if `v` hands `target` out as-is: directly (`a`), or as a direct element
/// of a tuple / vector-literal value (`(a, n)`, `[a, b]`).  A *derived* value
/// (`a[0]`, `a.len()`) does NOT count — it produces a fresh value, not a's store.
fn escapes_value(v: &Value, target: u16) -> bool {
    match v.unspan() {
        Value::Var(t) => *t == target,
        Value::Tuple(elems) | Value::Insert(elems) => {
            elems.iter().any(|e| escapes_value(e, target))
        }
        _ => false,
    }
}

pub(super) fn guard_escapes(node: &Value, target: u16) -> bool {
    node.any_node(&mut |n| match n {
        Value::Return(v) | Value::Yield(v) => escapes_value(v, target),
        // The block's VALUE is its last operator; if that hands out the local
        // (directly or in a tuple/literal), it escapes (block-result `x = {
        // …; a }` U3; `return (a, n)` t2).
        Value::Block(bl) | Value::Loop(bl) => bl
            .operators
            .last()
            .is_some_and(|o| escapes_value(o, target)),
        _ => false,
    })
}

/// Soundness gate for confining a *multi-store* local — one reassigned a fresh
/// store per block (the shared-`z`-across-`else`-blocks / shared-`x`-across-
/// match-arms shape).  Returns true iff every READ of `local` is dominated by a
/// non-null assignment of `local` earlier in the same straight-line block, so
/// the local never carries a store across a block boundary unreassigned.  When
/// that holds, freeing each store at *its* block exit cannot be a use-after-free
/// (the local's next read sees a freshly-assigned store, never the freed one).
/// Conditional assignments (inside `if`/`loop`) do NOT establish dominance for
/// code after the construct — the walk under-claims, so it stays sound.
pub(super) fn confine_reassign_safe(code: &Value, local: u16) -> bool {
    let mut ok = true;
    dominance_walk(code, local, false, &mut ok, false);
    ok
}

/// Shared dominance walk behind the two Plan-57 soundness gates
/// (pass-3 dedupe: the two walkers were 80 identical lines apart, and the
/// stronger one had silently lost a child arm).  `dom` = "every read
/// of `local` here is preceded by a non-null assignment that definitely
/// executed".  The two gates differ ONLY in:
/// - the START value (`confine_reassign_safe` starts false — a definite
///   reassignment must precede any read; `store_dead_after_block` starts
///   true — the fn-level init counts);
/// - `invalidate_conditional` (`store_dead_after_block` only): a
///   reassignment inside an `If`/`Loop`/`Iter` body INVALIDATES
///   dominance — afterwards `local` may hold that block's confined store.
///
/// A read of `local` while `!dom` clears `ok`.
fn dominance_walk(node: &Value, local: u16, dom: bool, ok: &mut bool, inv: bool) -> bool {
    match node {
        Value::Set(v, val) => {
            // RHS evaluated before the write lands; a read of `local` in it
            // is gated by the *current* dominance.
            dominance_walk(val, local, dom, ok, inv);
            if *v == local {
                return !matches!(val.unspan(), Value::Null);
            }
            dom
        }
        Value::Var(v) | Value::TupleGet(v, _) => {
            if *v == local && !dom {
                *ok = false;
            }
            dom
        }
        Value::CallRef(v, args) => {
            if *v == local && !dom {
                *ok = false;
            }
            let mut d = dom;
            for a in args {
                d = dominance_walk(a, local, d, ok, inv);
            }
            dom
        }
        Value::Block(bl) => {
            let mut d = dom;
            for op in &bl.operators {
                d = dominance_walk(op, local, d, ok, inv);
            }
            d
        }
        Value::Loop(lp) => {
            // Body runs 0+ times → conditional; dominance does not leak out.
            let mut d = dom;
            for op in &lp.operators {
                d = dominance_walk(op, local, d, ok, inv);
            }
            if inv && assigns_local(node, local) {
                false
            } else {
                dom
            }
        }
        Value::If(t, a, b) => {
            let dc = dominance_walk(t, local, dom, ok, inv);
            dominance_walk(a, local, dc, ok, inv);
            dominance_walk(b, local, dc, ok, inv);
            // Branch assignments are conditional — they establish no
            // post-`if` dominance; under `inv` they additionally invalidate.
            if inv && (assigns_local(a, local) || assigns_local(b, local)) {
                false
            } else {
                dc
            }
        }
        Value::Iter(idx, c, n, e) => {
            if *idx == local && !dom {
                *ok = false;
            }
            let mut d = dominance_walk(c, local, dom, ok, inv); // the iteration SOURCE reads `local`
            d = dominance_walk(n, local, d, ok, inv);
            dominance_walk(e, local, d, ok, inv); // body conditional
            if inv && assigns_local(node, local) {
                false
            } else {
                d
            }
        }
        Value::Call(_, args) | Value::Insert(args) | Value::Tuple(args) | Value::Parallel(args) => {
            let mut d = dom;
            for a in args {
                d = dominance_walk(a, local, d, ok, inv);
            }
            d
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            dominance_walk(v, local, dom, ok, inv);
            dom
        }
        Value::TuplePut(v, _, inner) => {
            if *v == local && !dom {
                *ok = false;
            }
            dominance_walk(inner, local, dom, ok, inv);
            dom
        }
        Value::Span(b) => dominance_walk(&b.1, local, dom, ok, inv),
        _ => dom,
    }
}

/// Plan-57 cluster-III Route 2: recover the backer of an *orphaned* store — one
/// the single-valued `dep` no longer records because its holding local was
/// reassigned.  Returns the local `L` the store flows into via its repoint
/// `Set(L, OpGetField(Var(vdb), …))` (the canonical `z = [..]` lowering).
fn recover_backer(code: &Value, vdb: u16, gf_nr: u32) -> Option<u16> {
    let mut backer = None;
    code.any_node(&mut |n| {
        if let Value::Set(l, val) = n
            && let Value::Call(op, args) = val.unspan()
            && *op == gf_nr
            && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == vdb)
        {
            backer = Some(*l);
            true
        } else {
            false
        }
    });
    backer
}

/// Does `node` contain a non-null reassignment of `local` anywhere?
fn assigns_local(node: &Value, local: u16) -> bool {
    node.any_node(
        &mut |n| matches!(n, Value::Set(v, val) if *v == local && !matches!(val.unspan(), Value::Null)),
    )
}

/// Plan-57 cluster-III Route 2 soundness gate (STRONGER than `confine_reassign_safe`,
/// which only proves the backer is *defined* at every read — the fn-level init
/// satisfies that even when a confined block store is still live, an empirically
/// confirmed UAF via `for x in v` after the block).
///
/// Dominance walk over the body: `dom` = "`local` is known NOT to hold a confined
/// block store here" (it holds the fn-level init or an unconditional reassignment).
/// `dom` starts true and is **invalidated** by any CONDITIONAL reassignment
/// (inside an `If`/`Loop`/`Iter`) — afterwards `local` might hold that block's store,
/// which the fix would free at block exit.  A read of `local` while `!dom` is an
/// over-free hazard → unsound to confine.  Mirrors `confine_reassign_safe` but with
/// the conditional-reassignment invalidation (the missing soundness property).
fn store_dead_after_block(code: &Value, local: u16) -> bool {
    let mut ok = true;
    dominance_walk(code, local, true, &mut ok, true);
    ok
}

/// Per `__vdb` store, the LCA non-loop block scope it is provably confined to —
/// i.e. the scope at which it could be freed instead of at function exit.
/// Returns `vdb -> (backed local, block scope)` for every store-backed local
/// confined to a non-loop block deeper than where its store is currently
/// registered.  Two consumers share this one analysis:
/// - the cluster-I fix — re-register the confined `__vdb` (+ its local) at the
///   block scope so the standard block-exit `free_vars` sweep frees it there;
/// - the `LOFT_STORE_GUARD` detector ([`store_lifetime_guard`]) — a thin
///   wrapper that reports each entry.
///
/// Soundness (adversarially hardened across the probe rounds): excludes escapes
/// (return/yield/break, block-result, tuple/vector element via `guard_escapes`),
/// loop-internal confinement (per-iteration reuse, not a watermark), and any
/// store aliased by a variable that outlives block `b`.
/// @PLN35 `..rest` store-lifetime OBSERVER (reached only via `LOFT_REST_ORACLE`; never
/// rewrites IR). For every `__vdb` store it re-runs the `store_confinement` gates in
/// REPORTING mode and prints the verdict — CONFINED to a block, or REJECTED with the
/// exact gate — so a leak can be attributed to a precise decision (e.g. the ambiguous
/// dep-backer gate that blocks the escaping-field `..rest` shape). Diagnostic only.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn rest_store_oracle(
    code: &Value,
    vars: &Function,
    free_ref_nr: u32,
    db_nr: u32,
    fn_name: &str,
) {
    let mut any = false;
    for vdb in 0..vars.count() {
        if !vars.name(vdb).starts_with("__vdb") {
            continue;
        }
        // Backers: every var whose dep carries this store.
        let mut backers: Vec<u16> = Vec::new();
        for v in 0..vars.count() {
            if vars.tp(v).depend().contains(&vdb) {
                backers.push(v);
            }
        }
        let user_backers: Vec<u16> = backers
            .iter()
            .copied()
            .filter(|&v| !vars.name(v).starts_with('_'))
            .collect();
        let temp_backers: Vec<u16> = backers
            .iter()
            .copied()
            .filter(|&v| vars.name(v).starts_with('_'))
            .collect();
        let arg_cap = backers
            .iter()
            .any(|&v| vars.is_argument(v) || vars.is_captured(v));
        if !any {
            eprintln!("[rest-oracle] fn={fn_name}");
            any = true;
        }
        let bnames: Vec<String> = backers.iter().map(|&v| vars.name(v).to_string()).collect();
        // Walk the same gate ladder store_confinement uses, but report the stop.
        let verdict = if arg_cap {
            "REJECT(arg/captured backer)".to_string()
        } else if backers.len() > 1 {
            // The gate that blocks the escaping-field `..rest` shape: the store is
            // dep-backed by the user vector AND a `_`-temp (`_elm`/`_comp`).
            format!(
                "REJECT(ambiguous: {} backers — user={:?} temp={:?})",
                backers.len(),
                user_backers
                    .iter()
                    .map(|&v| vars.name(v))
                    .collect::<Vec<_>>(),
                temp_backers
                    .iter()
                    .map(|&v| vars.name(v))
                    .collect::<Vec<_>>(),
            )
        } else if let Some(&local) = backers.first() {
            if vars.is_skip_free(local) || vars.is_skip_free(vdb) {
                "REJECT(skip_free — treated as escaping/borrowed)".to_string()
            } else if guard_escapes(code, local) {
                "REJECT(escapes: return/yield/break/element)".to_string()
            } else {
                let multi_store = vars.tp(local).depend().len() != 1;
                if multi_store && !confine_reassign_safe(code, local) {
                    "REJECT(multi-store, reassign-unsafe)".to_string()
                } else {
                    let span_target = if multi_store { vdb } else { local };
                    let mut stack: Vec<(u16, bool)> = Vec::new();
                    let mut lca: Option<Vec<(u16, bool)>> = None;
                    guard_refs(code, span_target, free_ref_nr, &mut stack, &mut lca);
                    match lca {
                        None => "REJECT(no ref LCA)".to_string(),
                        Some(path) if path.iter().any(|&(_, is_loop)| is_loop) => {
                            "REJECT(loop in LCA path — per-iteration reuse)".to_string()
                        }
                        Some(path) => match path.last() {
                            Some(&(b, _)) if vars.scope(vdb) == b => {
                                format!("already fn/block scope {b} (frees there)")
                            }
                            Some(&(b, _)) => format!("CONFINE to block {b}"),
                            None => "REJECT(empty LCA path)".to_string(),
                        },
                    }
                }
            }
        } else {
            "REJECT(no dep-backer — orphaned store)".to_string()
        };
        // THE DIRECT LEAK PREDICTOR (independent of confinement): does OpFreeRef(vdb)
        // execute BEFORE OpDatabase(vdb)? A value-type return hoists the allocating arm
        // into a `__ret` temp so the free lands after; a Reference / promoted-&text return
        // is NOT hoisted, so the free precedes the allocation → the store leaks. Pre-order
        // index of each op (Return(expr)→[expr], If(c,t,e)→[c,t,e]) approximates exec order.
        let mut ctr = 0usize;
        let mut alloc_at: Option<usize> = None;
        let mut free_at: Option<usize> = None;
        preorder_op_index(
            code,
            vdb,
            db_nr,
            free_ref_nr,
            &mut ctr,
            &mut alloc_at,
            &mut free_at,
        );
        let order = match (free_at, alloc_at) {
            (Some(f), Some(a)) if f < a => "FREE-before-ALLOC → LEAKS".to_string(),
            (Some(_), Some(_)) => "alloc-before-free → clean".to_string(),
            (Some(_), None) => "free, no alloc (null-only) → n/a".to_string(),
            (None, Some(_)) => "alloc, no free → LEAKS".to_string(),
            (None, None) => "neither → n/a".to_string(),
        };
        let ret_ty = block_result_type(code);
        eprintln!(
            "  store {} scope={} backers={bnames:?} ret={ret_ty} conf={verdict}",
            vars.name(vdb),
            vars.scope(vdb),
        );
        eprintln!("      free/alloc order: {order}");
    }
}

/// Pre-order (execution-approximating) index of the FIRST `OpDatabase(vdb)` and the FIRST
/// `OpFreeRef(vdb)` in `node`. `Return(expr)` unfolds to its inner (expr evaluates first);
/// `If(c,t,e)` visits cond then arms. Reporting helper for [`rest_store_oracle`].
fn preorder_op_index(
    node: &Value,
    vdb: u16,
    db_nr: u32,
    free_nr: u32,
    ctr: &mut usize,
    alloc_at: &mut Option<usize>,
    free_at: &mut Option<usize>,
) {
    *ctr += 1;
    if let Value::Call(op, args) = node.unspan()
        && let Some(Value::Var(v)) = args.first().map(Value::unspan)
        && *v == vdb
    {
        if *op == db_nr && alloc_at.is_none() {
            *alloc_at = Some(*ctr);
        }
        if *op == free_nr && free_at.is_none() {
            *free_at = Some(*ctr);
        }
    }
    node.for_each_child(&mut |c| preorder_op_index(c, vdb, db_nr, free_nr, ctr, alloc_at, free_at));
}

/// The result `Type` of a function body: the top-level `Block`'s declared result.
/// Reporting helper (the return type is the free-analysis hoist discriminator).
fn block_result_type(code: &Value) -> String {
    match code.unspan() {
        Value::Block(b) => {
            // The exact hoist gate the free-analysis (`insert_free`) uses.
            let hoists =
                is_value_return_type(&b.result) || matches!(b.result.base(), Type::Text(_));
            format!(
                "{:?}{}",
                b.result,
                if hoists { " (hoists)" } else { " (NO-hoist)" }
            )
        }
        _ => "<non-block>".to_string(),
    }
}

/// loft#750 — the result is a `BTreeMap`, and the ORDER is load-bearing.  Its
/// caller relocates each confined `__vdb`'s null-init, and a relocation that
/// cannot reach its block puts the init back at body position 0; run over
/// several confined stores, the visit order therefore PERMUTES the null-inits
/// at the head of the body — which moves the stack slots under them.  A
/// `HashMap` gave that order Rust's per-process hash seed, so compiling one
/// file twice with one binary produced different bytecode and different slots
/// (same answers, but no reproducible `--native` build, and a byte-identical-IR
/// inertness gate that could not tell "my change did nothing" from "the seed
/// moved").  Keyed by variable number, the visit order is now the declaration
/// order.
pub(super) fn store_confinement(
    code: &Value,
    vars: &Function,
    free_ref_nr: u32,
    gf_nr: u32,
) -> BTreeMap<u16, (u16, u16)> {
    // Plan-57 cluster-III Route 2: recover the backer of an orphaned (overwritten) store so
    // its per-block store can confine.  DEFAULT ON since 2026-08-21; `LOFT_NO_CONF_RECOVER=1`
    // emits the pre-Route-2 form and is the first bisect step for a wrong answer in a
    // function that reassigns a local across sibling blocks.
    //
    // A local reassigned across sibling `if`/`else if`/`match` arms otherwise keeps EVERY
    // arm's store to scope exit, so the store watermark grows with the number of
    // reassignment SITES and not with how many of them run: a 16-site function measured
    // peak 20 whichever single arm was taken, against a flat 5 with this on.
    //
    // What makes it safe is `store_dead_after_block`, not this flag: a local READ after the
    // blocks does not confine, because freeing a confined block store while the local still
    // holds it returns the wrong element on the branch that did NOT run.  That shape refuted
    // the first gate design and is pinned by
    // `tests/scripts/reassign-across-sibling-blocks.loft`, which asserts the same answers
    // with the recovery on and off.
    let recover = crate::env_once!(std::env::var("LOFT_NO_CONF_RECOVER").is_err());
    let mut out: BTreeMap<u16, (u16, u16)> = BTreeMap::new();
    for vdb in 0..vars.count() {
        if !vars.name(vdb).starts_with("__vdb") {
            continue;
        }
        // A caller-provided return buffer arrives as a PARAMETER, so it is not this
        // function's store to confine — and "free it at the inner block's exit" would
        // have the callee free the CALLER's buffer.  The backer check below rejects an
        // argument BACKER but never asked whether the store itself is one.
        //
        // `return <vector local>` lowers to exactly that shape: the incoming `__vdb`
        // becomes the buffer and the named local a field-view of it
        // (`buf = OpGetField(__vdb_1, …); return __vdb_1`), so the local is a non-arg
        // backer of an arg store and every such function was reported.  That is the
        // most ordinary vector idiom in the language — `cbor`'s `head` is the shape
        // that surfaced it — so the report was noise wherever anyone looked.
        if vars.is_argument(vdb) {
            continue;
        }
        // The single local that holds `vdb` (vdb in its dep), non-arg,
        // non-captured.  A *single-store* local (dep == [vdb]) shares its store's
        // span; a *multi-store* local (dep ⊇ vdb, reassigned a fresh store per
        // block — shared `z` across `else`-blocks, shared `x` across match arms)
        // spans the whole function even though each store lives in one block.
        let mut backed: Option<u16> = None;
        let mut ambiguous = false;
        for v in 0..vars.count() {
            if vars.tp(v).depend().contains(&vdb) {
                if vars.is_argument(v) || vars.is_captured(v) || backed.is_some() {
                    ambiguous = true;
                    break;
                }
                backed = Some(v);
            }
        }
        if ambiguous {
            continue;
        }
        // Route 2: an orphaned store has no dep-backer (single-valued dep dropped
        // the link when the local was reassigned).  Recover the local it flows into
        // via its `OpGetField` repoint; the recovered backer is necessarily
        // multi-store.  Gated off by default until soundness is locked in.
        let recovered = backed.is_none();
        let local = if let Some(l) = backed {
            l
        } else if recover {
            match recover_backer(code, vdb, gf_nr) {
                Some(l) if !vars.is_argument(l) && !vars.is_captured(l) => l,
                _ => continue,
            }
        } else {
            continue;
        };
        // An escaping local hands its store to the caller — freeing it at block
        // exit is a use-after-free.  Exclude direct returns/yields/breaks
        // (probe 30) and anything scope-analysis already marked skip-free.
        if vars.is_skip_free(local) || vars.is_skip_free(vdb) || guard_escapes(code, local) {
            continue;
        }
        let multi_store = recovered || vars.tp(local).depend().len() != 1;
        // A multi-store local must never carry `vdb`'s store across a block
        // boundary unreassigned, else block-exit freeing is a UAF.  Gate on the
        // write-dominates-read walk before trusting the per-store span.
        if multi_store && !confine_reassign_safe(code, local) {
            continue;
        }
        // Confinement block = the LCA non-loop block of the *store's own* refs
        // (multi-store) or the local's (single-store — equal to the store's, kept
        // for the U3 alias path below).
        let span_target = if multi_store { vdb } else { local };
        let mut stack: Vec<(u16, bool)> = Vec::new();
        let mut lca: Option<Vec<(u16, bool)>> = None;
        guard_refs(code, span_target, free_ref_nr, &mut stack, &mut lca);
        // Confined iff the LCA path is a non-empty chain of NON-LOOP blocks
        // (NO loop anywhere in it — a confinement *inside* a loop is per-
        // iteration reuse, not a watermark; probes 33/34), and the innermost
        // block is deeper than where the store is currently scoped.
        if let Some(path) = lca
            && let Some(&(b, _)) = path.last()
            && path.iter().all(|&(_, is_loop)| !is_loop)
            && vars.scope(vdb) != b
            // Route 2 soundness: the recovered backer's block store must be dead
            // after its block on EVERY path — `confine_reassign_safe` only proves
            // the backer is *defined* at reads (the fn-level init satisfies that),
            // so an extra "no body-scope read of the backer" gate is required.
            && (!recovered || store_dead_after_block(code, local))
            // dep-escape: the store must not be aliased by a USER variable that
            // OUTLIVES block `b`.  A block-result `x = { …; a }` gives x the dep
            // `["a"]` and x is read at function level (U3) — freeing a here would
            // corrupt x.  Compiler temps (`_elm`, `__vdb`) are confined to their
            // own block and, for a multi-store local, may legitimately alias the
            // local in a *sibling* block (holding a different store there) — so
            // skip them rather than false-positive.
            && !(0..vars.count()).any(|w| {
                w != local
                    && !vars.name(w).starts_with('_')
                    && vars.tp(w).depend().contains(&local)
                    && {
                        let mut wst: Vec<(u16, bool)> = Vec::new();
                        let mut wlca: Option<Vec<(u16, bool)>> = None;
                        guard_refs(code, w, free_ref_nr, &mut wst, &mut wlca);
                        // w aliases the store AND is referenced outside `b`
                        // (b absent from w's confinement path) ⇒ outlives it.
                        wlca.is_some_and(|p| !p.iter().any(|&(s, _)| s == b))
                    }
            })
        {
            out.insert(vdb, (local, b));
        }
    }
    out
}

/// `LOFT_STORE_GUARD` detector — reports each store-backed local that frees at
/// function exit despite being confined to an inner block.  Thin wrapper over
/// [`store_confinement`] (the same analysis that drives the cluster-I fix).
/// Returns the number of late-freed stores.
pub(super) fn store_lifetime_guard(
    code: &Value,
    vars: &Function,
    free_ref_nr: u32,
    gf_nr: u32,
    fn_name: &str,
) -> usize {
    let confined = store_confinement(code, vars, free_ref_nr, gf_nr);
    for (&vdb, &(local, b)) in &confined {
        eprintln!(
            "[store-guard] {fn_name}: store {} (local '{}') confined to block scope {b} but stored at scope {} — frees late",
            vars.name(vdb),
            vars.name(local),
            vars.scope(vdb),
        );
    }
    confined.len()
}
