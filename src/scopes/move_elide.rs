// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! MOVE elision after the scan: a borrow-verdict vector bound in place of its copy, and a record
//! copy from a source's last use turned into a move.

use super::construct_elide::{
    ConstructOps, construct_fresh_rewrite, construct_move_rewrite, construct_replace_rewrite,
};
use crate::data::{Data, DefType, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// EXPERIMENTAL (LOFT_BORROW_ELIDE) — inline the Tier-0 Borrow-verdict vector
/// copies: for each elidable `v = copy(s.f)`, replace every read of `v` with the
/// source field-access `s.f` and drop the copy idiom (the `vdb` buffer's alloc /
/// length-set / append / free, and `v`'s defining `Set`). The result is the IR a
/// direct `s.f[i]` program compiles — which already aliases with no copy — so it
/// needs no borrow-dep/skip_free surgery. Runs before the scope/free passes.
pub(super) fn elide_borrows(data: &mut Data) {
    let op_database = data.def_nr("OpDatabase");
    let op_append = data.def_nr("OpAppendVector");
    let op_set_int4 = data.def_nr("OpSetInt4");
    let op_set_int = data.def_nr("OpSetInt");
    let op_free = data.def_nr("OpFreeRef");

    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let plans = crate::use_analysis::elision_plans(
            &data.def(d_nr).code,
            &data.def(d_nr).variables,
            data,
        );
        if plans.is_empty() {
            continue;
        }
        // A var that BORROWS `v` (`e = v[i]`, `deps` ∋ `v`) is left with a stale dep
        // once `v` is deleted, so the borrowed-view codegen (`stack(dep[0])`) would
        // dereference a dead slot. `use_analysis` only emits a plan when every such
        // borrower is read-only/non-escaping, and lists them; RE-POINT each borrower
        // `v → source_base` so it borrows the live source element instead (codegen
        // then reads the source param's valid slot). This is what lets the
        // borrowed-element accessors elide rather than fall back to copy.
        let mut elide_v: HashMap<u16, Value> = HashMap::default();
        let mut elide_vdb: HashSet<u16> = HashSet::default();
        for p in plans {
            // The elision deletes a copy the line wrote; the copy-lease rules still judge it.
            if let Value::Var(src) = p.source.unspan() {
                crate::copy_manifest::note_elided_copy(d_nr, p.var, *src);
            }
            for &e in &p.borrowers {
                data.definitions[d_nr as usize]
                    .variables
                    .make_independent(e, p.var);
                data.definitions[d_nr as usize]
                    .variables
                    .depend(e, p.source_base);
            }
            elide_v.insert(p.var, p.source);
            elide_vdb.insert(p.vdb);
        }
        let ops = ElideOps {
            op_database,
            op_append,
            op_set_int4,
            op_set_int,
            op_free,
        };
        let mut code = data.def(d_nr).code.clone();
        elide_rewrite(&mut code, &elide_v, &elide_vdb, &ops);
        data.definitions[d_nr as usize].code = code;
    }
}

// The `op_` prefix is meaningful (these are operator def-numbers), so the
// same-prefix style lint does not apply.
#[allow(clippy::struct_field_names)]
struct ElideOps {
    op_database: u32,
    op_append: u32,
    op_set_int4: u32,
    op_set_int: u32,
    op_free: u32,
}

/// Is `stmt` a copy-idiom statement for an elided binding (to be dropped)?
fn idiom_drop(
    stmt: &Value,
    elide_v: &HashMap<u16, Value>,
    elide_vdb: &HashSet<u16>,
    o: &ElideOps,
) -> bool {
    match stmt.unspan() {
        // `v = OpGetField(vdb,…)` (the def) and `vdb = null` (the init).
        Value::Set(v, _) => elide_v.contains_key(v) || elide_vdb.contains(v),
        Value::Call(d, args) => {
            let target = match args.first().map(Value::unspan) {
                Some(Value::Var(t)) => Some(*t),
                _ => None,
            };
            if *d == o.op_database || *d == o.op_set_int4 || *d == o.op_set_int {
                target.is_some_and(|t| elide_vdb.contains(&t)) // buffer alloc / length-set
            } else if *d == o.op_append {
                target.is_some_and(|t| elide_v.contains_key(&t)) // copy-fill
            } else if *d == o.op_free {
                target.is_some_and(|t| elide_vdb.contains(&t) || elide_v.contains_key(&t))
            } else {
                false
            }
        }
        _ => false,
    }
}

fn elide_rewrite(
    node: &mut Value,
    elide_v: &HashMap<u16, Value>,
    elide_vdb: &HashSet<u16>,
    o: &ElideOps,
) {
    // Inline a read of an elided var with its source field-access.
    let replacement = match node.unspan() {
        Value::Var(v) => elide_v.get(v).cloned(),
        _ => None,
    };
    if let Some(src) = replacement {
        *node = src;
        return;
    }
    match node {
        Value::Block(b) => {
            b.operators
                .retain(|s| !idiom_drop(s, elide_v, elide_vdb, o));
            for op in &mut b.operators {
                elide_rewrite(op, elide_v, elide_vdb, o);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !idiom_drop(s, elide_v, elide_vdb, o));
            for op in &mut *ops {
                elide_rewrite(op, elide_v, elide_vdb, o);
            }
        }
        _ => node.for_each_child_mut(&mut |c| elide_rewrite(c, elide_v, elide_vdb, o)),
    }
}

// The `op_` prefix is meaningful (operator def-numbers), so the same-prefix lint does not apply.
#[allow(clippy::struct_field_names)]
pub(super) struct MoveOps {
    op_database: u32,
    op_copy_record: u32,
    op_free: u32,
    op_new_record: u32,
    op_get_field: u32,
    op_get_vector: u32,
}

/// @PLN90 phase B (B1.3) — the last-use MOVE-elision rewrite for the RECORD copy shape
/// (`v[i] = e` / `o.f = src`, lowered as `OpCopyRecord`). When a source `s`'s store is dead
/// after the copy (a [`crate::use_analysis::MovePlan`] with `kind == Record`), build `s`'s
/// fields DIRECTLY into the copy's destination slot instead of constructing a throwaway record
/// and deep-copying it:
///
/// ```text
///   OpDatabase(s)                           (dropped)
///   OpSetInt (s, off, v)  ── retarget ──▶  OpSetInt (dest, off, v)
///   OpSetText(s, off, v)  ── retarget ──▶  OpSetText(dest, off, v)
///   OpCopyRecord(s, dest)                   (dropped — the deep copy is gone)
///   OpFreeRef(s)                            (dropped — no throwaway store to free)
/// ```
///
/// where `dest` is the `OpCopyRecord` destination expression (`OpGetVector(v,…)` / `OpGetField`).
/// Emits NO new op — both backends already lower a retargeted `OpSet*`, so the rewrite is
/// backend-agnostic (one IR pass, like `elide_borrows`). DEFAULT ON (B1.5); `LOFT_NO_MOVE_ELIDE`
/// restores the copy. The CONSTRUCT shape is handled by [`construct_move_rewrite`] (B1.3b) for
/// the reorder-free field-append case; fresh construction (`a = Bag { items: base }`, container
/// built after the source) still needs a build-order reorder and stays a copy. Design:
/// `doc/claude/plans/90-copy-diagnostics/phase-b-design.md`.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn move_elide(data: &mut Data) {
    if !crate::keys::move_elide_enabled() {
        return;
    }
    let mo = MoveOps {
        op_database: data.def_nr("OpDatabase"),
        op_copy_record: data.def_nr("OpCopyRecord"),
        op_free: data.def_nr("OpFreeRef"),
        op_new_record: data.def_nr("OpNewRecord"),
        op_get_field: data.def_nr("OpGetField"),
        op_get_vector: data.def_nr("OpGetVector"),
    };
    let co = ConstructOps {
        op_database: data.def_nr("OpDatabase"),
        op_free: data.def_nr("OpFreeRef"),
        op_append: data.def_nr("OpAppendVector"),
        op_prealloc: data.def_nr("OpPreAllocVector"),
        op_set_int4: data.def_nr("OpSetInt4"),
        op_get_field: data.def_nr("OpGetField"),
        op_clear: data.def_nr("OpClearVector"),
        op_new_record: data.def_nr("OpNewRecord"),
        op_finish_record: data.def_nr("OpFinishRecord"),
        op_push: crate::parser::FUSED_PUSH_OPS
            .iter()
            .map(|n| data.def_nr(n))
            .filter(|d| *d != u32::MAX)
            .collect(),
    };
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        // Plan-based rewrites (Record / Construct) key off these; the structural `a.field = base`
        // rewrite (B1.3d) does not — so we do NOT early-continue on an empty plan set.
        let plans = crate::use_analysis::move_plans(data, d_nr);
        let mut code = data.def(d_nr).code.clone();
        // Vars to suppress the (later `variables()`-emitted) scope-exit free for — the moved-out
        // owned store now lives in the destination, so its null slot must not be freed.
        let mut skip: HashSet<u16> = HashSet::default();
        // Containers a rewrite must NOT retarget a source's build into — NOT a stable, pre-existing,
        // single-def owned slot. Two producers, unioned; every rewrite consults the result:
        //  - transient element slots (`_elm_N = OpNewRecord(…)`, reused across a vector literal /
        //    nested construction) — a fresh record defined LATER (use-before-def);
        //  - vars allocated MORE THAN ONCE — REASSIGNED (`b = Bag{…}; … b = Bag{…}`) — the container
        //    has a prior store the reorder's hoist does not retire.
        let mut bad_containers = collect_element_vars(&code, &mo);
        bad_containers.extend(collect_multi_database(&code, &mo));
        // First-def order per var — a Record destination's container must be defined BEFORE its
        // source (else the retargeted build writes into an un-allocated container).
        let def_order = collect_def_order(&code, &mo);

        // ── RECORD shape (`v[i]=e` / `o.f=src`, OpCopyRecord) ──
        let rec_sources: HashSet<u16> = plans
            .iter()
            .filter(|p| p.kind == crate::use_analysis::MoveKind::Record)
            .map(|p| p.source)
            .collect();
        if !rec_sources.is_empty() {
            // Pass 1 — capture each source's UNIQUE copy destination. A source seen copying into
            // two different places is not the clean dead-after shape the plan assumes: skip it.
            let mut dest: HashMap<u16, Value> = HashMap::default();
            let mut ambiguous: HashSet<u16> = HashSet::default();
            collect_move_dest(
                &code,
                &mo,
                &data.def(d_nr).variables,
                &rec_sources,
                &bad_containers,
                &def_order,
                &mut dest,
                &mut ambiguous,
            );
            // A destination TOUCHED between the source's build and the copy cannot take the
            // retarget: the write would move ahead of that access.  See
            // `collect_move_disturbed`.
            let mut disturbed: HashSet<u16> = HashSet::default();
            collect_move_disturbed(&code, &mo, mo.op_copy_record, 0, &dest, &mut disturbed);
            let ready: HashSet<u16> = dest
                .keys()
                .copied()
                .filter(|s| !ambiguous.contains(s) && !disturbed.contains(s))
                .collect();
            if !ready.is_empty() {
                move_rewrite(&mut code, &ready, &dest, &mo);
                skip.extend(&ready);
            }
        }

        // ── CONSTRUCT shape (`x.field += src` field-append, OpAppendVector) — REORDER-FREE only ──
        let con_sources: HashSet<u16> = plans
            .iter()
            .filter(|p| p.kind == crate::use_analysis::MoveKind::Construct)
            .map(|p| p.source)
            .collect();
        if !con_sources.is_empty() {
            // Sources that are USED outside their own construction + the single copy — read between
            // being built and being moved (`out=[]; for{ out+=[…] }; assert("{out:j}"); w={items:out}`
            // — `out` is read by the assert BEFORE the move). Building such a source directly into the
            // destination would leave that intermediate read seeing the un-built source, so leave it
            // a copy. (Also subsumes append-grown sources: `v += w` is `v` at arg0 of a non-write op.)
            let escaping: HashSet<u16> = con_sources
                .iter()
                .copied()
                .filter(|&s| source_escapes(&code, s, &co))
                .collect();
            // B1.3b — reorder-free field-appends (`x.field += src`, container already exists).
            let mut moved_into: HashMap<u16, u16> = HashMap::default();
            construct_move_rewrite(
                &mut code,
                &con_sources,
                &co,
                &mo,
                &bad_containers,
                &escaping,
                &mut skip,
                &mut moved_into,
            );
            // A retargeted source is ERASED: its wrapper alloc, its view-def and the append are
            // all gone, so nothing writes it any more.  What still names it is the `deps` of the
            // element work-refs whose builds were just re-pointed — and a dep is the statement
            // "my store belongs to that variable", which after the retarget belongs to the
            // CONTAINER instead.  Left stale it is wrong twice over: the ownership derivation
            // reads a var that owns nothing (@FR-O-Deps — every store-lifetime
            // decision reads this one fact), and the scope pass declares the dep var so a borrower
            // can name it, which hands the erased local a stack slot no instruction ever writes.
            // That slot is what @PLN120 A's store-span check reports (loft#1241): the local is
            // not merely unrecorded, it is not there.  Re-pointing states the fact the rewrite
            // created rather than exempting the symptom by name.
            for (&src, &cvar) in &moved_into {
                let vars = &mut data.definitions[d_nr as usize].variables;
                for v in 0..vars.next_var() {
                    if v != src && vars.tp(v).depend().contains(&src) {
                        vars.make_independent(v, src);
                        vars.depend(v, cvar);
                    }
                }
            }
            // B1.3c — fresh construction (`a = Bag { items: base }`, container built after the
            // source): hoist `a`'s alloc, then retarget. Runs on the copies B1.3b left standing.
            // B1.4 — the interprocedural mutation set (`find_written_vars` knows which callees
            // mutate a `&`-param in ANY arg position), so a param used as a hoisted field value is
            // allowed only if genuinely never mutated.
            let mut written: HashSet<u16> = HashSet::default();
            crate::parser::find_written_vars(
                &data.def(d_nr).code,
                data,
                &mut written,
                &mut HashMap::default(),
            );
            construct_fresh_rewrite(
                &mut code,
                &con_sources,
                &co,
                &data.def(d_nr).variables,
                &written,
                &bad_containers,
                &escaping,
                &mut skip,
            );
        }

        // B1.3d — the `a.field = base` whole-vector replacement (the `__p154_rhs` idiom): a DOUBLE
        // copy (`base → __p154_rhs → a.field`, with an `OpClearVector`). Its source is a temp, so it
        // is NOT a MovePlan — this rewrite is STRUCTURAL and must run regardless of `con_sources`.
        construct_replace_rewrite(&mut code, &co, &mut skip);

        data.definitions[d_nr as usize].code = code;
        for &s in &skip {
            data.definitions[d_nr as usize].variables.set_skip_free(s);
        }
    }
}

/// Pass 1 of [`move_elide`]: map each move source to the `OpCopyRecord` destination it copies
/// into; a source seen with a second destination is marked ambiguous (and skipped).
#[allow(clippy::too_many_arguments)]
fn collect_move_dest(
    node: &Value,
    mo: &MoveOps,
    function: &Function,
    sources: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    def_order: &HashMap<u16, usize>,
    dest: &mut HashMap<u16, Value>,
    ambiguous: &mut HashSet<u16>,
) {
    if let Value::Call(d, args) = node.unspan()
        && *d == mo.op_copy_record
        && let Some(Value::Var(s)) = args.first().map(Value::unspan)
        && sources.contains(s)
        && args.len() >= 2
    {
        // A stable in-place target is a slot EXPRESSION over a PRE-EXISTING container
        // (`OpGetVector(v,…)` / `OpGetField(o,…)`). NOT stable, and skipped:
        //  - a bare `Var` dest (`x = e`) — no proof it pre-exists the source;
        //  - a dest based on a FRESH `OpNewRecord` element (`OpGetField(_elm_N,…)` nested
        //    construction) — defined AFTER the source (native `var__elm_N` not in scope);
        //  - a dest based on ANY compiler TEMP (`_`-prefixed: `__ref_N` field-iteration refs,
        //    `_slice_*`, …) — these are populated by machinery whose validity point the structural
        //    rewrite can't prove. Only a USER-named container is a proven-stable target.
        let unstable = match args[1].unspan() {
            Value::Var(_) => true,
            _ => base_var_of(&args[1], mo).is_none_or(|base| {
                bad_containers.contains(&base)
                    || function.name(base).starts_with('_')
                    // the container must be DEFINED before the source is built (else the retargeted
                    // build writes into an un-allocated slot).
                    || def_order
                        .get(&base)
                        .zip(def_order.get(s))
                        .is_none_or(|(&bd, &sd)| bd >= sd)
            }),
        };
        if unstable || dest.contains_key(s) {
            ambiguous.insert(*s);
        } else {
            dest.insert(*s, args[1].clone());
        }
    }
    node.for_each_child(&mut |c| {
        collect_move_dest(
            c,
            mo,
            function,
            sources,
            bad_containers,
            def_order,
            dest,
            ambiguous,
        );
    });
}

/// Does `node` mention variable `v` anywhere inside it?
fn mentions_var(node: &Value, v: u16) -> bool {
    if matches!(node.unspan(), Value::Var(x) if *x == v) {
        return true;
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && mentions_var(c, v) {
            found = true;
        }
    });
    found
}

/// Sources whose retarget would move the destination's WRITE across a statement that touches
/// that destination.
///
/// [`move_rewrite`] retargets the source's construction ops onto the destination and drops the
/// copy, so the destination is written at the CONSTRUCTION's position instead of at the copy's.
/// That is sound only while nothing in between touches the destination.  The guards in
/// [`collect_move_dest`] all ask whether the destination is a STABLE container; none of them
/// asks whether it is READ, and that is the hole:
///
/// ```text
///   for p in v { held = Tg { name: p.0.name }; p.0 = p.1; p.1 = held; }
/// ```
///
/// `held`'s build moves into `p.1`, so `p.0 = p.1` then copies the NEW value back and the swap
/// answers `x|x` where it wants `y|x` — silently, on both backends.
///
/// Read by BOTH move shapes — the Record copy (`OpCopyRecord(src, dst)`, source at arg 0) and
/// the Construct append (`OpAppendVector(dst, src)`, source at arg 1) — because they had the
/// same hole and two copies of one predicate is the shape loft#1006 was.  B1.3d already carried
/// this guard for its own rewrite (`try_replace_one`: *"`base`'s BUILD must not read the
/// destination container"*), which is what the other two were missing.
///
/// Conservative by BASE variable: any mention of the destination's base container between the
/// two points disqualifies the move.  The source's own construction ops are excluded (they are
/// what gets retargeted), so `o.f = T { x: o.g }` — building FROM the container into it — is
/// still admitted.  A slot-exact test would admit a few more and cannot be spelled reliably:
/// two spellings of one slot is the shape loft#1006 was.
pub(super) fn collect_move_disturbed(
    node: &Value,
    mo: &MoveOps,
    copy_op: u32,
    src_arg: usize,
    dest: &HashMap<u16, Value>,
    out: &mut HashSet<u16>,
) {
    let scan = |ops: &[Value], out: &mut HashSet<u16>| {
        for (copy_idx, op) in ops.iter().enumerate() {
            let Value::Call(d, args) = op.unspan() else {
                continue;
            };
            if *d != copy_op {
                continue;
            }
            let Some(Value::Var(s)) = args.get(src_arg).map(Value::unspan) else {
                continue;
            };
            let Some(slot) = dest.get(s) else { continue };
            let Some(base) = base_var_of(slot, mo) else {
                continue;
            };
            // Where `s` is first defined in THIS list; absent (built in another block) is
            // treated as "from the top", which is the conservative reading.
            let def_idx = ops
                .iter()
                .position(|o| match o.unspan() {
                    Value::Set(v, _) => *v == *s,
                    Value::Call(d2, a2) => {
                        *d2 == mo.op_database
                            && matches!(a2.first().map(Value::unspan), Some(Value::Var(v)) if *v == *s)
                    }
                    _ => false,
                })
                .unwrap_or(0);
            if def_idx >= copy_idx {
                continue;
            }
            for between in &ops[def_idx + 1..copy_idx] {
                // The source's OWN construction ops are what the rewrite retargets, so they
                // are not a disturbance — that is what keeps `o.f = T { x: o.g }` elidable.
                let writes_source = matches!(between.unspan(), Value::Call(_, a)
                    if matches!(a.first().map(Value::unspan), Some(Value::Var(v)) if *v == *s));
                if !writes_source && mentions_var(between, base) {
                    out.insert(*s);
                    break;
                }
            }
        }
    };
    match node.unspan() {
        Value::Block(b) => scan(&b.operators, out),
        Value::Insert(ops) => scan(ops, out),
        _ => {}
    }
    node.for_each_child(&mut |c| collect_move_disturbed(c, mo, copy_op, src_arg, dest, out));
}

/// Vars defined by `OpNewRecord` (`_elm_N = OpNewRecord(…)`) — the transient element slots the
/// vector-literal / construction lowering reuses. A copy destination based on one of these is a
/// FRESH element defined after the source, so it is not a stable in-place retarget target.
fn collect_element_vars(node: &Value, mo: &MoveOps) -> HashSet<u16> {
    fn walk(node: &Value, mo: &MoveOps, out: &mut HashSet<u16>) {
        if let Value::Set(v, rhs) = node.unspan()
            && matches!(rhs.unspan(), Value::Call(d, _) if *d == mo.op_new_record)
        {
            out.insert(*v);
        }
        node.for_each_child(&mut |c| walk(c, mo, out));
    }
    let mut out = HashSet::default();
    walk(node, mo, &mut out);
    out
}

/// Vars allocated (`OpDatabase(v, …)`) MORE THAN ONCE — a var REASSIGNED to a fresh record/vector
/// (`b = Bag{…}; … b = Bag{…}`). Such a container has a prior store the reorder rewrites' hoist does
/// not retire, so they must leave it a copy.
fn collect_multi_database(node: &Value, mo: &MoveOps) -> HashSet<u16> {
    fn walk(node: &Value, mo: &MoveOps, seen: &mut HashSet<u16>, multi: &mut HashSet<u16>) {
        if let Value::Call(d, args) = node.unspan()
            && *d == mo.op_database
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
            && !seen.insert(*v)
        {
            multi.insert(*v);
        }
        node.for_each_child(&mut |c| walk(c, mo, seen, multi));
    }
    let mut seen = HashSet::default();
    let mut multi = HashSet::default();
    walk(node, mo, &mut seen, &mut multi);
    multi
}

/// Does `src` ESCAPE its own construction — is it referenced anywhere OTHER than (a) as arg0 of a
/// WRITE op that builds it (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord`/`OpSetInt4`, or a
/// fused `OpPush<Kind>`), or (b) as
/// arg1 of the append copy that moves it? A source that is built, then READ (`"{out:j}"`, `out[i]`,
/// passed to a fn), then moved is NOT dead-between-build-and-copy: building it directly into the
/// destination would leave the intermediate read seeing the un-built source (`var_out` not in
/// scope / wrong value). The `Set(src, …)` view-def / null-init targets `src` (not an arg) and is
/// fine; the `vdb` backing / `_elm` element temps aren't `src`. Any other appearance → escapes.
fn source_escapes(node: &Value, src: u16, co: &ConstructOps) -> bool {
    fn walk(node: &Value, src: u16, co: &ConstructOps, bad: &mut bool) {
        if let Value::Call(d, args) = node.unspan() {
            for (i, a) in args.iter().enumerate() {
                if matches!(a.unspan(), Value::Var(v) if *v == src) {
                    let write_arg0 = i == 0
                        && (*d == co.op_prealloc
                            || *d == co.op_new_record
                            || *d == co.op_finish_record
                            || *d == co.op_set_int4
                            || co.op_push.contains(d));
                    let copy_arg1 = i == 1 && *d == co.op_append;
                    if !(write_arg0 || copy_arg1) {
                        *bad = true;
                    }
                }
            }
        }
        node.for_each_child(&mut |c| walk(c, src, co, bad));
    }
    let mut bad = false;
    walk(node, src, co, &mut bad);
    bad
}

/// First-definition encounter order per var (`Set(v,…)` value-bind or `OpDatabase(v)` alloc). Used
/// to prove a Record destination's container is defined BEFORE the source is built — otherwise
/// retargeting the source's construction into `container.field` writes into an un-allocated slot
/// (`b = SABag { extra: extra }`: `b` allocated AFTER `extra` → `var_b` not in scope).
fn collect_def_order(node: &Value, mo: &MoveOps) -> HashMap<u16, usize> {
    // ⚠ Peels the SCRUTINEE while the recursion below walks the ORIGINAL `node`, so a spanned
    // node is visited TWICE: once here through the peel, once when `for_each_child` descends
    // to the same payload (it sees through a `Span` itself).  Safe here only because both
    // accumulations are IDEMPOTENT — `or_insert` ignores the second write, and `idx` feeds a
    // relative ORDER, so an extra tick shifts every later index equally.  Keep it that way, or
    // bind (`let node = node.unspan();`) so the match and the walk see the same node.  The
    // same shape counted a sandbox callee twice and inflated a heap bound by half — see
    // `sandbox::intrinsic_space`.
    fn walk(node: &Value, mo: &MoveOps, idx: &mut usize, out: &mut HashMap<u16, usize>) {
        match node.unspan() {
            Value::Set(v, _) => {
                out.entry(*v).or_insert(*idx);
            }
            Value::Call(d, args) if *d == mo.op_database => {
                if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                    out.entry(*v).or_insert(*idx);
                }
            }
            _ => {}
        }
        *idx += 1;
        node.for_each_child(&mut |c| walk(c, mo, idx, out));
    }
    let mut idx = 0;
    let mut out = HashMap::default();
    walk(node, mo, &mut idx, &mut out);
    out
}

/// The base container var of a slot expression: peel `OpGetField` / `OpGetVector` down to the
/// leading `Var`. `Var(v) → v`; a non-projection / non-Var expression → `None`.
fn base_var_of(expr: &Value, mo: &MoveOps) -> Option<u16> {
    match expr.unspan() {
        Value::Var(v) => Some(*v),
        Value::Call(d, args) if *d == mo.op_get_field || *d == mo.op_get_vector => {
            args.first().and_then(|a| base_var_of(a, mo))
        }
        _ => None,
    }
}

/// Pass 2 of [`move_elide`]: over each op list, DROP a ready source's `OpDatabase` /
/// `OpCopyRecord` / `OpFreeRef`, and RETARGET every remaining construction op that writes into
/// the source (`OpSet*(s, …)`) onto that source's captured destination expression.
fn move_rewrite(node: &mut Value, ready: &HashSet<u16>, dest: &HashMap<u16, Value>, mo: &MoveOps) {
    // `Value::unspan`'s obligation, in its mutable form.  Measured over the 858-program
    // corpus: 567 values arrive here, 41 of them spanned around an arm (33 `Call`, 8 `Block`).
    //
    // ⚠ Those 41 were NOT misses, and an earlier version of this comment claimed they were.
    // The `_` arm ends in `node.for_each_child_mut(…)`, and that walk DESCENDS THROUGH a
    // `Span`, so a spanned node was already reached one level down — measured
    // behaviour-identical across 120 corpus programs with and without this peel.  It is kept
    // because it obeys the rule and reaches the right arm directly; it fixes nothing.
    //
    // The `if let Value::Call(…) = node` below reads the ORIGINAL binding ON PURPOSE: peeling
    // it as well would retarget the same call twice, once here and once when the trailing walk
    // reaches it.  That is the double-count that peeling only the scrutinee produced in
    // `sandbox::intrinsic_space` — a bound inflated from `24 · n²` to `36 · n²`.
    match node.unspan_mut() {
        Value::Block(b) => {
            b.operators.retain(|s| !move_drop(s, ready, mo));
            for op in &mut b.operators {
                move_rewrite(op, ready, dest, mo);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !move_drop(s, ready, mo));
            for op in &mut *ops {
                move_rewrite(op, ready, dest, mo);
            }
        }
        _ => {
            if let Value::Call(d, args) = node {
                // Only a construction op (not one of the three dropped ops) whose target is a
                // ready source gets its target rewritten to the destination slot.
                let retarget = *d != mo.op_copy_record
                    && *d != mo.op_database
                    && *d != mo.op_free
                    && matches!(args.first().map(Value::unspan),
                        Some(Value::Var(s)) if ready.contains(s));
                if retarget
                    && let Some(Value::Var(s)) = args.first().map(Value::unspan)
                    && let Some(dst) = dest.get(s).cloned()
                {
                    args[0] = dst;
                }
            }
            node.for_each_child_mut(&mut |c| move_rewrite(c, ready, dest, mo));
        }
    }
}

/// Retain-predicate for [`move_rewrite`]: is `stmt` a dropped op (the source's alloc, the
/// `OpCopyRecord`, or the source's free) for a ready move source?
fn move_drop(stmt: &Value, ready: &HashSet<u16>, mo: &MoveOps) -> bool {
    if let Value::Call(d, args) = stmt.unspan()
        && (*d == mo.op_database || *d == mo.op_copy_record || *d == mo.op_free)
        && let Some(Value::Var(s)) = args.first().map(Value::unspan)
    {
        return ready.contains(s);
    }
    false
}
