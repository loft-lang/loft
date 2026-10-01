// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! LAST-USE freeing: the stores whose data is dead before their scope ends, the plan that
//! reclaims them early, its soundness gates, and the store-identity tags and diagnostics around it.

use super::confinement::{confine_reassign_safe, guard_escapes, is_var_null_init};
use crate::data::{Data, Type, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// Which store a `Set`'s RHS binds the assigned local to (so the local now "holds"
/// that store's data): `OpGetField(Var(s), …)` → `s` (the repoint idiom); a copy
/// `Var(other)` → whatever `other` currently holds; anything else → unbound.
fn binding_source(val: &Value, gf_nr: u32, holds: &HashMap<u16, u16>) -> Option<u16> {
    match val.unspan() {
        Value::Call(op, args) if *op == gf_nr => match args.first().map(Value::unspan) {
            Some(Value::Var(s)) => Some(*s),
            _ => None,
        },
        Value::Var(other) => holds.get(other).copied(),
        _ => None,
    }
}

/// Flow walk: trace which store each local **holds** so each store's *data*
/// liveness is recovered — `alloc` (its `OpDatabase`), `last_read` (last read of
/// its data via a holding local or a direct build op, EXCLUDING the scope-exit
/// `OpFreeRef`), and `dead` (the point a holding local is rebound away, the
/// reassignment case).  This is the real liveness `compute_intervals` cannot give
/// (its `last_use` is pinned to the teardown `OpFreeRef`).  Sequential approximation
/// across branches — fine for the straight-line / sequential shapes this targets.
#[allow(clippy::too_many_arguments)]
fn store_liveness_walk(
    node: &Value,
    seq: &mut u32,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    holds: &mut HashMap<u16, u16>,
    alloc: &mut HashMap<u16, u32>,
    dead: &mut HashMap<u16, u32>,
    last_read: &mut HashMap<u16, u32>,
) {
    *seq += 1;
    let s = *seq;
    match node {
        Value::Var(v) => {
            if let Some(&store) = holds.get(v) {
                last_read.insert(store, s);
            }
        }
        Value::Set(local, val) => {
            if matches!(val.unspan(), Value::Null) {
                return; // null-init — not a real def/rebind
            }
            store_liveness_walk(val, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            let new_store = binding_source(val, gf_nr, holds);
            if let Some(&old) = holds.get(local)
                && Some(old) != new_store
            {
                dead.entry(old).or_insert(s); // local rebound away → old store dead here
            }
            match new_store {
                Some(st) => {
                    holds.insert(*local, st);
                }
                None => {
                    holds.remove(local);
                }
            }
        }
        Value::Call(op, args) => {
            if *op == fr_nr && args.len() == 1 && matches!(args[0].unspan(), Value::Var(_)) {
                return; // OpFreeRef — not a data read
            }
            if *op == db_nr {
                if let Some(Value::Var(st)) = args.first().map(Value::unspan) {
                    alloc.insert(*st, s);
                }
                return; // OpDatabase(store, size) — alloc point, args are not data reads
            }
            for a in args {
                store_liveness_walk(a, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for op in &bl.operators {
                store_liveness_walk(op, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::If(c, t, e) => {
            store_liveness_walk(c, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            store_liveness_walk(t, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            store_liveness_walk(e, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for op in ops {
                store_liveness_walk(op, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            store_liveness_walk(v, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read);
        }
        Value::Span(b) => {
            store_liveness_walk(
                &b.1, seq, db_nr, gf_nr, fr_nr, holds, alloc, dead, last_read,
            );
        }
        _ => {}
    }
}

/// True if `node` contains the `OpDatabase(Var(store), …)` allocation of `store`.
fn contains_alloc(node: &Value, store: u16, db_nr: u32) -> bool {
    node.any_node(&mut |n| {
        matches!(n, Value::Call(op, args) if *op == db_nr
        && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == store))
    })
}

/// Plan-57 Phase-3 soundness gate (dominance): true only if `store`'s `OpDatabase`
/// allocation is reached **unconditionally** from the body — never gated by an
/// `If`/`Loop`/`Parallel` branch.  A reclaim that early-frees / drops the scope-exit
/// free of a store allocated in an untaken branch leaks or double-frees
/// (`20-binary`).  Plain nested blocks always run, so they stay unconditional.
fn contains_alloc_unconditional(node: &Value, store: u16, db_nr: u32) -> bool {
    match node.unspan() {
        Value::Call(op, args) => {
            (*op == db_nr
                && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == store))
                || args
                    .iter()
                    .any(|a| contains_alloc_unconditional(a, store, db_nr))
        }
        Value::Set(_, val) => contains_alloc_unconditional(val, store, db_nr),
        Value::Block(bl) => bl
            .operators
            .iter()
            .any(|o| contains_alloc_unconditional(o, store, db_nr)),
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            contains_alloc_unconditional(v, store, db_nr)
        }
        Value::Insert(ops) | Value::Tuple(ops) => ops
            .iter()
            .any(|o| contains_alloc_unconditional(o, store, db_nr)),
        Value::Span(b) => contains_alloc_unconditional(&b.1, store, db_nr),
        // If / Loop / Parallel / Iter — conditional, so an alloc inside is
        // NOT unconditional.
        _ => false,
    }
}

/// Plan-57 Phase-3 soundness gate (retention): true if a `holder` var (the store or
/// any local that holds it) appears in a value position that could keep the store's
/// data reachable **past the early-free point** — embedded in a tuple/vector literal,
/// stored as a struct-field / keyed value, returned/yielded, copied to an alias, or
/// passed as a non-receiver argument.  A holder as the FIRST argument of a call (the
/// receiver of an index/len/field read, or an in-place append target) does not
/// retain — that is exactly the straight-line shape this pass targets.  Everything
/// else is treated as retention (conservative — errs toward leaving the store alone).
fn holder_retained(node: &Value, holders: &HashSet<u16>) -> bool {
    match node {
        Value::Var(h) => holders.contains(h),
        Value::Call(_, args) | Value::CallRef(_, args) => args.iter().enumerate().any(|(i, a)| {
            // The receiver (first arg) being a bare holder Var is a read, not a
            // retention; any nested expression there is still scanned.
            if i == 0 && matches!(a.unspan(), Value::Var(h) if holders.contains(h)) {
                false
            } else {
                holder_retained(a, holders)
            }
        }),
        Value::Set(_, val) => holder_retained(val, holders),
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter().any(|o| holder_retained(o, holders))
        }
        Value::If(c, t, e) => {
            holder_retained(c, holders)
                || holder_retained(t, holders)
                || holder_retained(e, holders)
        }
        Value::Insert(xs) | Value::Tuple(xs) | Value::Parallel(xs) => {
            xs.iter().any(|x| holder_retained(x, holders))
        }
        Value::Return(v) | Value::Yield(v) | Value::Drop(v) => holder_retained(v, holders),
        Value::TuplePut(_, _, v) => holder_retained(v, holders),
        Value::Iter(_, c, n, e) => {
            holder_retained(c, holders)
                || holder_retained(n, holders)
                || holder_retained(e, holders)
        }
        Value::Span(b) => holder_retained(&b.1, holders),
        _ => false,
    }
}

/// #426B — every var that transitively reaches store `st` through the dep graph.
///
/// `reclaim_safe`'s holder model needs the full closure, not the one-hop depers:
/// a binding can borrow a store INDIRECTLY through an intermediate local — a
/// fn-return-of-index `b = idx0(ww){ w[0] }` binds `b` with dep `["ww"]`, and
/// `ww` deps `["__vdb_1"]`, so `b` holds `__vdb_1` via `ww`.  Missing `b` here
/// lets reclaim free `__vdb_1` before `b`'s last read (store-reuse-after-free).
///
/// Walks to a fixpoint: a var is a deper of `st` if its dep list contains `st`
/// or contains any var already known to be a deper.  Marker deps (`u16::MAX`
/// one-buffer sentinel, the `0x8000` callee-frame tag) name no frame var and are
/// skipped.  Bounded by `vars.count()` iterations (each pass adds at least one
/// var or stops), so it always terminates.
fn transitive_depers(vars: &Function, st: u16) -> Vec<u16> {
    let n = vars.count();
    let mut reaches: HashSet<u16> = HashSet::default();
    loop {
        let mut added = false;
        for v in 0..n {
            if reaches.contains(&v) {
                continue;
            }
            let deps_st = vars
                .tp(v)
                .depend()
                .into_iter()
                .any(|d| d != u16::MAX && d & 0x8000 == 0 && (d == st || reaches.contains(&d)));
            if deps_st {
                reaches.insert(v);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let mut out: Vec<u16> = reaches.into_iter().collect();
    out.sort_unstable();
    out
}

/// Plan-57 Phase-3 soundness gate: is store `st` (a `__vdb` work-ref) safe to
/// early-free + relocate + strip-its-scope-exit-free?  Mirrors `store_confinement`'s
/// per-store predicates (the I-a soundness model) for the function-scoped case:
///
/// - not `skip_free` / `captured` / a `RefVar` alias, and not directly escaping
///   (`guard_escapes`);
/// - every var that *holds* it (`st ∈ depend`) is a non-arg, non-captured,
///   non-`skip_free`, non-`RefVar`, non-escaping local; at most one is a *user*
///   local; a multi-store user local must pass `confine_reassign_safe`;
/// - none of the holders is *retained* anywhere (`holder_retained`).
///
/// Orphaned stores (no holder — the reassignment case, `__vdb_1..10` in probe 14)
/// are safe-because-dead: nothing reaches them after the rebind.  Conservative by
/// design — an escape/alias/capture/struct-field case falls back to the (sound)
/// scope-exit free.
fn reclaim_safe(code: &Value, vars: &Function, st: u16) -> bool {
    if !vars.name(st).starts_with("__vdb") {
        return false;
    }
    if vars.is_skip_free(st) || vars.is_captured(st) || matches!(vars.tp(st), Type::RefVar(_)) {
        return false;
    }
    if guard_escapes(code, st) {
        return false;
    }
    // #426B — the holder set must be the TRANSITIVE dep closure, not just the
    // one-hop depers.  A view-of-a-view keeps `st` live through an intermediate
    // local: `b = idx0(ww)` where `idx0` returns `w[0]` binds `b` with dep
    // `["ww"]`, and `ww` deps `["__vdb_1"]` — so `b` transitively holds
    // `__vdb_1` and extends its lifetime to `b`'s last use.  A single-hop scan
    // sees only `ww` (the receiver arg of the call, treated as a read, not a
    // retention), misses `b`, and reclaim frees `__vdb_1` right after the call —
    // before `b` reads it.  The freed slot is then recycled into the next
    // allocation, corrupting `b` (store-reuse-after-free).  Walking the dep
    // graph to fixpoint makes `b` a holder, so the retention scan / multi-user-
    // local gate below leave `st`'s sound scope-exit free in place.
    let depers: Vec<u16> = transitive_depers(vars, st);
    let mut user_locals = 0;
    for &v in &depers {
        if vars.is_argument(v)
            || vars.is_captured(v)
            || vars.is_skip_free(v)
            || matches!(vars.tp(v), Type::RefVar(_))
            || guard_escapes(code, v)
        {
            return false;
        }
        if !vars.name(v).starts_with('_') {
            user_locals += 1;
            if vars.tp(v).depend().len() != 1 && !confine_reassign_safe(code, v) {
                return false;
            }
        }
    }
    if user_locals > 1 {
        return false; // multiple live aliases — leave the store alone
    }
    // Holders for the retention scan: the raw store plus its NON-`_` user-var
    // holders.  `_`-prefixed compiler temps (`_elm`, nested `__vdb`) are build-
    // internal — confined to the store they construct, not external aliases — so
    // they are skipped, mirroring `store_confinement`'s dep-escape treatment.
    // Without this, a comprehension's per-iteration element record (`_elm`, which
    // deps the result `__vdb`) false-positives as retention.
    let mut holders: HashSet<u16> = depers
        .into_iter()
        .filter(|&v| !vars.name(v).starts_with('_'))
        .collect();
    holders.insert(st);
    !holder_retained(code, &holders)
}

/// Plan-57 — the reclaim PLAN, the single source of truth shared by
/// [`lastuse_reclaim`] (which acts on it) and [`reclaim_unfreed_eligible`] (the
/// Phase-4 guard, which verifies the frees landed), so the two cannot drift.
/// Returns `(owning, intent)`:
/// - `owning` — function-scoped owning stores passing the dominance + soundness
///   gates (the ones whose null-init may relocate);
/// - `intent` — `(store, trigger)` pairs: `store`'s data dies before the eligible
///   sibling `trigger` allocates, so `store` must be freed before `trigger`'s build.
///
/// A store in `drop_bearing` is never eligible ([`drop_bearing_stores`]).
pub(super) fn reclaim_free_intent(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    drop_bearing: &HashSet<u16>,
) -> (Vec<u16>, Vec<(u16, u16)>) {
    let body_scope = match code.unspan() {
        Value::Block(bl) => bl.scope,
        _ => return (Vec::new(), Vec::new()),
    };
    let mut holds: HashMap<u16, u16> = HashMap::default();
    let mut alloc: HashMap<u16, u32> = HashMap::default();
    let mut dead: HashMap<u16, u32> = HashMap::default();
    let mut last_read: HashMap<u16, u32> = HashMap::default();
    let mut seq = 0u32;
    store_liveness_walk(
        code,
        &mut seq,
        db_nr,
        gf_nr,
        fr_nr,
        &mut holds,
        &mut alloc,
        &mut dead,
        &mut last_read,
    );
    let dead_at = |st: u16| {
        dead.get(&st)
            .copied()
            .or_else(|| last_read.get(&st).copied())
    };
    let mut owning: Vec<u16> = alloc
        .keys()
        .copied()
        .filter(|&st| {
            vars.scope(st) == body_scope
                && !drop_bearing.contains(&st)
                && contains_alloc_unconditional(code, st, db_nr)
                && reclaim_safe(code, vars, st)
        })
        .collect();
    owning.sort_unstable();
    let mut intent: Vec<(u16, u16)> = Vec::new();
    for &st in &owning {
        let Some(d) = dead_at(st) else { continue };
        if let Some(&later) = owning
            .iter()
            .filter(|&&w| w != st && alloc.get(&w).is_some_and(|&a| a > d))
            .min_by_key(|&&w| alloc[&w])
        {
            intent.push((st, later));
        }
    }
    (owning, intent)
}

/// The locals of function `d_nr` whose record type has a drop cascade: the stores last-use
/// reclaim must leave alone (`@FR-H-Drop`).
///
/// Reclaim moves a store's DEATH — it relocates the null-init down to the build and frees the
/// store early — but a drop belongs to that death and reclaim moves only the free.  The hook
/// stays at scope exit, where the slot names a store a later build has reused, so it releases
/// that store's resources instead; and a rebuild's release snapshot, which reads the slot just
/// above the build, now runs before the relocated declaration.  Kept out of the plan, such a
/// store keeps its body-0 null-init and releases through its hook and its free together.
pub(super) fn drop_bearing_stores(data: &Data, d_nr: u32) -> HashSet<u16> {
    let vars = &data.def(d_nr).variables;
    (0..vars.count())
        .filter(|&v| {
            vars.tp(v)
                .base()
                .heap_def_nr()
                .is_some_and(|d| data.drop_cascade_nr(d) != u32::MAX)
        })
        .collect()
}

/// True if `op` is a top-level `OpFreeRef(Var(st))`.
fn is_top_free(op: &Value, st: u16, fr_nr: u32) -> bool {
    matches!(op.unspan(), Value::Call(o, args) if *o == fr_nr
        && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == st))
}

/// Plan-57 Phase-4 guard (Goal-E enforcement): after [`lastuse_reclaim`] has run,
/// every store in the reclaim plan's `intent` must have its `OpFreeRef` placed at
/// body top-level BEFORE the op that allocates its `trigger`.  Returns the count of
/// reclaim-eligible stores left live-but-dead past a later alloc — must be 0.  A
/// non-zero result means reclaim silently failed to stop a store the model says is
/// dead (a regression the watermark rule must not re-acquire).  Escape/alias cases
/// are not in `intent` (the soundness gate excluded them) — they legitimately keep
/// their scope-exit free and are not asserted on.
pub(super) fn reclaim_unfreed_eligible(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    drop_bearing: &HashSet<u16>,
) -> usize {
    let (_owning, intent) = reclaim_free_intent(code, vars, db_nr, gf_nr, fr_nr, drop_bearing);
    let Value::Block(bl) = code.unspan() else {
        return 0;
    };
    let mut count = 0;
    for &(st, later) in &intent {
        let free_idx = bl.operators.iter().position(|o| is_top_free(o, st, fr_nr));
        let alloc_idx = bl
            .operators
            .iter()
            .position(|o| contains_alloc(o, later, db_nr));
        match (free_idx, alloc_idx) {
            (Some(f), Some(a)) if f < a => {} // freed before the trigger allocates — good
            _ => count += 1,
        }
    }
    count
}

/// Plan-57 last-use freeing, Phase 3 — reclaim via null-init RELOCATION + early
/// free (gated `LASTUSE_RECLAIM`).
///
/// Phase 2 proved free-alone is inert: every `__vdb`'s null-init (`Set(vdb, Null)`,
/// hoisted to body-0 by `parse_code`) ALLOCATES its store up front, so the runtime
/// watermark is locked before any inserted free can run (probe 14: peak 11 = the 11
/// null-inits stacking at body-0).  This pass closes that with two coordinated edits
/// per dead store:
///
/// 1. **Relocate the null-init** out of body-0 to immediately before its own
///    `OpDatabase` build — so the stores stop batching at body-0 and allocate
///    interleaved.  (The I-a `relocate_null_init` lever, applied to a body *index*
///    instead of a sub-block.)
/// 2. **Early free** before the next store allocates — so a freed slot is reused by
///    the following null-init (`+alloc, -free, +alloc, -free`) instead of stacking.
///
/// The scope-exit `OpFreeRef` is left in place as an idempotent double-free
/// (`free_named` no-ops an already-free store — measured safe in Phase 2).  Both
/// edits run **before** `compute_intervals`, so the moved `first_def` is reflected
/// in the slot intervals.  Flat-body straight-line / sequential shapes only — the
/// I-b / III-straight-line cases block-confinement (I-a) cannot reach.  Returns the
/// count of relocations + frees applied.
pub(super) fn lastuse_reclaim(
    code: &mut Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    drop_bearing: &HashSet<u16>,
) -> usize {
    // Eligibility + free-intent come from the shared plan, so the Phase-4 guard
    // (`reclaim_unfreed_eligible`) verifies exactly what this pass acts on.
    let (owning, intent) = reclaim_free_intent(code, vars, db_nr, gf_nr, fr_nr, drop_bearing);
    if owning.is_empty() {
        return 0;
    }
    let Value::Block(bl) = code else { return 0 };
    // #260 Fix B replaced Fix A here: native codegen now declares every
    // `__vdb` local up front (sentinel-bound prologue, `generation/mod.rs::
    // output_function`), so relocating a null-init below an early-return
    // scope-exit free can no longer strand the free's `var_…` reference out
    // of scope (rustc E0425) — the `has_free_before_alloc` exclusion guard
    // is gone and those stores get their watermark reclaim back (46/46
    // owning stores were forfeited in brick-buster's generator pre-B).
    // Early-free groups: before[trigger] = dead stores to free right before
    // `trigger` allocates.  Their scope-exit `OpFreeRef` is REMOVED, not kept as an
    // "idempotent double-free": under reclaim the freed slot is reused by a later
    // store, so a stale scope-exit free of `st` would target a *different live
    // owner's* store (the tag gate catches exactly this).  The early free becomes
    // the store's sole free.
    let mut before: HashMap<u16, Vec<u16>> = HashMap::default();
    for &(st, later) in &intent {
        before.entry(later).or_default().push(st);
    }
    let freed_set: HashSet<u16> = intent.iter().map(|&(st, _)| st).collect();
    // Reloc set: owning stores whose null-init sits at body top-level AND whose
    // OpDatabase build is a top-level body op (so the null-init can be placed right
    // before it).  Excludes any store I-a already relocated into a sub-block.
    let reloc: Vec<u16> = owning
        .iter()
        .copied()
        .filter(|&st| {
            bl.operators.iter().any(|o| is_var_null_init(o, st))
                && bl.operators.iter().any(|o| contains_alloc(o, st, db_nr))
        })
        .collect();
    // Pull the relocatable null-inits out of the body (keyed by store), and DROP the
    // existing scope-exit `OpFreeRef(Var(st))` for every store we early-free.
    let mut saved: HashMap<u16, Value> = HashMap::default();
    let kept: Vec<Value> = std::mem::take(&mut bl.operators)
        .into_iter()
        .filter_map(|op| {
            if let Some(&st) = reloc.iter().find(|&&st| is_var_null_init(&op, st)) {
                saved.insert(st, op);
                return None;
            }
            if let Value::Call(o, args) = op.unspan()
                && *o == fr_nr
                && let Some(Value::Var(v)) = args.first().map(Value::unspan)
                && freed_set.contains(v)
            {
                return None; // scope-exit free of an early-freed store — drop it
            }
            Some(op)
        })
        .collect();
    // Rebuild: before each op that allocates store `st`, emit the early frees of the
    // stores that died before it, then `st`'s relocated null-init, then the op.
    let mut count = 0;
    for op in kept {
        let allocs_here: Vec<u16> = owning
            .iter()
            .copied()
            .filter(|&st| contains_alloc(&op, st, db_nr))
            .collect();
        for &st in &allocs_here {
            if let Some(frees) = before.get(&st) {
                for &f in frees {
                    bl.operators.push(Value::Call(fr_nr, vec![Value::Var(f)]));
                    count += 1;
                }
            }
        }
        for &st in &allocs_here {
            if let Some(ni) = saved.remove(&st) {
                bl.operators.push(ni);
                count += 1;
            }
        }
        bl.operators.push(op);
    }
    // Safety: restore any null-init whose alloc was not matched, so first_def is
    // never lost (should not happen given the reloc filter, but keep it sound).
    for (_st, ni) in saved {
        bl.operators.insert(0, ni);
    }
    count
}

/// Per-function allocation-site id for a store-owning var (1-based; 0 is the
/// "untagged" sentinel). Stable within a function so a var's `OpDatabase` and its
/// `OpFreeRef` share the same id; the global `counter` keeps ids unique across
/// functions so a cross-function wrong-store free mismatches.
fn store_site_id(v: u16, ids: &mut HashMap<u16, u16>, counter: &mut u16) -> u16 {
    *ids.entry(v).or_insert_with(|| {
        let id = *counter;
        *counter = counter.wrapping_add(1);
        if *counter == 0 {
            *counter = 1;
        }
        id
    })
}

/// Plan-57 store-identity gate (Phase 2.5) — gated IR post-pass (`LOFT_STORE_TAG`).
///
/// Rewrites store ops to their verifying variants so a free can be checked against
/// the allocation that owns the store: insert `OpStoreTag(vdb, id)` right after each
/// `OpDatabase(vdb, …)`, and replace `OpFreeRef(vdb)` with `OpFreeRefTag(vdb, id)`.
/// Normal builds (no env) never run this, so the bytecode stays byte-identical.
#[allow(clippy::too_many_arguments)]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn tag_stores(
    code: &mut Value,
    db_nr: u32,
    fr_nr: u32,
    store_tag_nr: u32,
    free_ref_tag_nr: u32,
    tagset: &HashSet<u16>,
    ids: &mut HashMap<u16, u16>,
    counter: &mut u16,
) {
    match code {
        Value::Block(bl) | Value::Loop(bl) => {
            let mut i = 0;
            while i < bl.operators.len() {
                tag_stores(
                    &mut bl.operators[i],
                    db_nr,
                    fr_nr,
                    store_tag_nr,
                    free_ref_tag_nr,
                    tagset,
                    ids,
                    counter,
                );
                // Identify a top-level OpDatabase / OpFreeRef on a tracked Var.  Only
                // reclaim-eligible stores (`tagset`) are tagged/verified — adopted /
                // shared / file stores carry no tag (no OpStoreTag) and keep their
                // plain OpFreeRef, so the gate cannot false-positive on them.
                let hit = match bl.operators[i].unspan() {
                    Value::Call(op, args) if *op == db_nr || (*op == fr_nr && args.len() == 1) => {
                        match args.first().map(Value::unspan) {
                            Some(Value::Var(v)) if tagset.contains(v) => Some((*op == db_nr, *v)),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some((is_alloc, vdb)) = hit {
                    let id = i32::from(store_site_id(vdb, ids, counter));
                    if is_alloc {
                        bl.operators.insert(
                            i + 1,
                            Value::Call(store_tag_nr, vec![Value::Var(vdb), Value::Int(id)]),
                        );
                        i += 1; // skip the inserted tag op
                    } else {
                        bl.operators[i] =
                            Value::Call(free_ref_tag_nr, vec![Value::Var(vdb), Value::Int(id)]);
                    }
                }
                i += 1;
            }
        }
        Value::If(c, t, e) => {
            tag_stores(
                c,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
            tag_stores(
                t,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
            tag_stores(
                e,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        Value::Insert(ops) | Value::Tuple(ops) | Value::Parallel(ops) => {
            for o in ops {
                tag_stores(
                    o,
                    db_nr,
                    fr_nr,
                    store_tag_nr,
                    free_ref_tag_nr,
                    tagset,
                    ids,
                    counter,
                );
            }
        }
        Value::Return(v) | Value::Drop(v) | Value::Yield(v) => {
            tag_stores(
                v,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        Value::Span(b) => {
            tag_stores(
                &mut b.1,
                db_nr,
                fr_nr,
                store_tag_nr,
                free_ref_tag_nr,
                tagset,
                ids,
                counter,
            );
        }
        _ => {}
    }
}

/// Plan-57 last-use freeing, Phase 1 — definition-point liveness diagnostic.
///
/// Reports each **function-scoped store** whose *data* dies (last read or a
/// rebind-away) **before another store allocates** — so it is held dead to scope
/// exit while the watermark grows.  This is the I-b (sequential distinct) and
/// III-straight-line (sequential reassign) divergence block-confinement cannot
/// reach.  Returns the count.  Read-only; gated by `LOFT_LASTUSE_GUARD`.
///
/// Block-confined stores (already freed at block exit by I-a) are excluded by the
/// body-scope filter.  Genuinely live-to-the-end stores self-exclude — nothing
/// allocates after their last read.
pub(super) fn last_use_guard(
    code: &Value,
    vars: &Function,
    db_nr: u32,
    gf_nr: u32,
    fr_nr: u32,
    fn_name: &str,
) -> usize {
    let body_scope = match code.unspan() {
        Value::Block(bl) => bl.scope,
        _ => return 0,
    };
    let mut holds: HashMap<u16, u16> = HashMap::default();
    let mut alloc: HashMap<u16, u32> = HashMap::default();
    let mut dead: HashMap<u16, u32> = HashMap::default();
    let mut last_read: HashMap<u16, u32> = HashMap::default();
    let mut seq = 0u32;
    store_liveness_walk(
        code,
        &mut seq,
        db_nr,
        gf_nr,
        fr_nr,
        &mut holds,
        &mut alloc,
        &mut dead,
        &mut last_read,
    );
    // data-death point: rebind-away if any, else last read of the data.
    let dead_at = |st: u16| {
        dead.get(&st)
            .copied()
            .or_else(|| last_read.get(&st).copied())
    };
    let mut count = 0;
    let mut stores: Vec<u16> = alloc.keys().copied().collect();
    stores.sort_unstable();
    for &st in &stores {
        if vars.scope(st) != body_scope {
            continue; // block-confined — freed at block exit by I-a
        }
        let Some(d) = dead_at(st) else { continue };
        if let Some(&later) = stores
            .iter()
            .filter(|&&w| w != st && alloc.get(&w).is_some_and(|&a| a > d))
            .min_by_key(|&&w| alloc[&w])
        {
            eprintln!(
                "[lastuse-guard] {fn_name}: store '{}' data dead @{d} but held to scope exit \
                 while '{}' allocates @{} — should have been stopped",
                vars.name(st),
                vars.name(later),
                alloc[&later],
            );
            count += 1;
        }
    }
    count
}
