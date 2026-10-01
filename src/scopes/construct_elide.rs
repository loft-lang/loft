// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! CONSTRUCTION move elision: a value copied into a field of a record under construction, or a
//! whole-vector field replacement, written into its destination instead of copied twice.

use super::move_elide::{MoveOps, collect_move_disturbed};
use crate::data::{Block, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

// The `op_` prefix is meaningful (operator def-numbers), so the same-prefix lint does not apply.
#[allow(clippy::struct_field_names)]
pub(super) struct ConstructOps {
    pub(super) op_database: u32,
    pub(super) op_free: u32,
    pub(super) op_append: u32,
    pub(super) op_prealloc: u32,
    pub(super) op_set_int4: u32,
    pub(super) op_get_field: u32,
    pub(super) op_clear: u32,
    pub(super) op_new_record: u32,
    pub(super) op_finish_record: u32,
    /// The fused scalar appends (@PLN157 § V-m, `OpPush<Kind>(container, val)`): a source's
    /// element build in ONE op, so it counts as building the source exactly as
    /// `OpNewRecord`/`OpFinishRecord` do, and retargets the same way.
    pub(super) op_push: Vec<u32>,
}

/// @PLN90 phase B (B1.3b) — the CONSTRUCT copy shape (`x.field += src`, lowered as a copying
/// `OpAppendVector(x.field, src)`), restricted to the **reorder-free** case: the destination
/// container already exists when `src` is built. `src` is a vector view over its own backing
/// wrapper `vdb` (`src = OpGetField(vdb, …)`); instead of building `src`'s elements into `vdb`
/// then copying the whole vector into `x.field`, retarget `src`'s element-build ops
/// (`OpNewRecord`/`OpFinishRecord`) DIRECTLY onto `x.field`, and drop `vdb`'s alloc/init/free,
/// `src`'s view-def + capacity `OpPreAllocVector`, and the `OpAppendVector` copy. No new op — the
/// retargeted appends grow `x.field` exactly as the copy did.
///
/// **Reorder-free guard (the safety line):** fire only when `x`'s allocation precedes `vdb`'s
/// (or `x` is a parameter, i.e. never `OpDatabase`'d here). The fresh-construction case
/// (`a = Bag { items: base }` — `a` built AFTER `base`) needs a build-order reorder and is NOT
/// handled here; it is left as a copy. `skip` receives ONLY the backings of sources actually
/// rewritten, so a skipped source keeps its free (no live-store suppression).
// The eight parameters are one act's worth of state: the code being rewritten, the four
// read-only sets that decide whether a source may move, and the two out-parameters the
// caller reads back.  Bundling them into a struct would put a name between each set and
// the predicate that reads it without removing anything.
#[allow(clippy::too_many_arguments)]
pub(super) fn construct_move_rewrite(
    code: &mut Value,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    mo: &MoveOps,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
    moved_into: &mut HashMap<u16, u16>,
) {
    // Pass 1 — pre-scan: per source capture the append destination, its backing wrapper, the
    // destination's container var, the `OpDatabase` encounter order (for the reorder guard) and
    // the control-flow position of the build and of the append (for the run-count guard).
    let mut sc = ConstructScan::default();
    construct_prescan(code, co, con_sources, &mut sc);
    let ConstructScan {
        db_order,
        db_path,
        dest,
        dest_path,
        ambiguous,
        vdb,
        container,
        last_bind,
        ..
    } = sc;

    // A destination TOUCHED between the source's build and the append cannot take the retarget:
    // the append would move ahead of that access.  `escaping` above guards the SOURCE being read
    // in between and this is its missing twin — measured, `seen = len(b.v); b.v += tmp` reported
    // the POST-append length, and `for x in c.v { t2 += [...] } c.v += t2` retargeted the loop's
    // appends onto the vector the loop was ITERATING and grew it without bound.
    let mut disturbed: HashSet<u16> = HashSet::default();
    collect_move_disturbed(code, mo, co.op_append, 1, &dest, &mut disturbed);

    // Ready = found a unique append destination + a backing wrapper, AND provably reorder-free.
    let ready: HashSet<u16> = con_sources
        .iter()
        .copied()
        .filter(|s| {
            !ambiguous.contains(s)
                && dest.contains_key(s)
                && vdb.contains_key(s)
                // A source READ between its build and the move (or grown by appends) can't be built
                // directly into the destination — leave it a copy.
                && !escaping.contains(s)
                && !disturbed.contains(s)
                // B1.3b handles a PURE append (`x.field += src`). If the field is CLEARED anywhere
                // it is a whole-vector REPLACE (`x.field = src`, an `OpClearVector` + append) — a
                // simple retarget would leave the clear stranded after the retargeted build (empties
                // the result), and the source may even READ the field (`s.v = s.v[1..]`). Leave the
                // replace to B1.3d (which moves the clear) or to a copy.
                && !field_is_cleared(code, &dest[s], co)
                // The rewrite leaves the element builds where the source is BUILT and DROPS the
                // append, so the program it produces runs the build as often as the append it
                // replaced only when the two sit in the same control-flow region.  Where they do
                // not, the count is simply wrong in whichever direction the region runs: an
                // append inside a loop ran once per turn and now runs once for the whole loop
                // (`for i in 0..3 { d.c += s }` grew `d.c` by one copy, not three), and an append
                // inside a branch NOT TAKEN did not run at all and now always does (`if false {
                // d.c += s }` appended anyway).  Both answer wrong with nothing said, on both
                // backends, because this is one shared IR pass (@FR-O-NoDiverge).
                // Equality of the whole path, not of its depth: two arms of one `if` are equally
                // deep and never run together.  @FR-O-Latest — a binding's ownership
                // lives at the LOOP DEPTH its assignment was taken at, which no type-level fact
                // can carry, so the depth has to be measured here.  loft#1243.
                && db_path.get(&vdb[s]) == dest_path.get(s)
                && container.get(s).and_then(|c| *c).is_some_and(|cvar| {
                    // The container must PRE-EXIST when the source is built. A FRESH `OpNewRecord`
                    // element (`[Chunk { … }]` → `_elm_N.field += src`) has no `OpDatabase` but is
                    // NOT pre-existing — it is defined later, so retargeting there is use-before-def.
                    if bad_containers.contains(&cvar) {
                        return false;
                    }
                    // `@FR-R-MoveLast` — a container BOUND after the source's backing exists is
                    // not the record the retargeted build lands in: a rebind allocates nothing,
                    // so the order check below read it as a parameter (loft#1741).
                    if let (Some(&bound), Some(&v)) = (last_bind.get(&cvar), db_order.get(&vdb[s]))
                        && bound > v
                    {
                        return false;
                    }
                    // container built before the source's backing (both locals), or a real param.
                    match (db_order.get(&cvar), db_order.get(&vdb[s])) {
                        (Some(&c), Some(&v)) => c < v,
                        (None, Some(_)) => true, // container is a parameter (never allocated here)
                        _ => false,
                    }
                })
        })
        .collect();
    if ready.is_empty() {
        return;
    }
    let vdbs: HashSet<u16> = ready.iter().map(|s| vdb[s]).collect();

    // Pass 2 — retarget the element builds + drop the wrapper / view / prealloc / copy.
    construct_rewrite_ops(code, &ready, &vdbs, &dest, co);
    // Report where each source's records now live, so its borrowers can be re-pointed there
    // (loft#1241).  `ready` already proved the container is a `OpGetField(Var(c), …)` base.
    for s in &ready {
        if let Some(Some(cvar)) = container.get(s) {
            moved_into.insert(*s, *cvar);
        }
    }
    // The moved-out owned store is the backing wrapper (`src` is a borrow of it) — suppress ITS
    // free. Only ready sources' backings are added, so a skipped source keeps its free.
    skip.extend(vdbs);
}

/// The base var of a `OpGetField(Var(x), …)` destination expression (`x`), else `None`.
fn get_field_base(expr: &Value, co: &ConstructOps) -> Option<u16> {
    if let Value::Call(d, args) = expr.unspan()
        && *d == co.op_get_field
        && let Some(Value::Var(x)) = args.first().map(Value::unspan)
    {
        return Some(*x);
    }
    None
}

/// Where a statement sits in the control flow: the chain of enclosing regions that run
/// zero, one, or MANY times — a loop body, an `if` arm, a parallel arm, an iterator's
/// step.  Every region gets its own id, so two statements carry the same path exactly
/// when each execution of one is an execution of the other.  That is the question
/// [`construct_move_rewrite`] has to answer before it may move a build and drop a copy.
type CfPath = Vec<u32>;

/// What [`construct_prescan`] gathers in one walk of a function body, for
/// [`construct_move_rewrite`] to filter.
#[derive(Default)]
struct ConstructScan {
    /// `OpDatabase` encounter index per var (first allocation wins) — the build-order guard.
    db_order: HashMap<u16, usize>,
    /// Where each var's `OpDatabase` sits: the site the retargeted element builds stay at.
    db_path: HashMap<u16, CfPath>,
    /// The one append destination expression per source.
    dest: HashMap<u16, Value>,
    /// Where that append sits: the site whose copy the rewrite DROPS.
    dest_path: HashMap<u16, CfPath>,
    /// Sources appended into two places — not the clean shape, so not rewritten.
    ambiguous: HashSet<u16>,
    /// Source → the backing wrapper it is a view of.
    vdb: HashMap<u16, u16>,
    /// Source → the destination's container var, when the destination is a field read.
    container: HashMap<u16, Option<u16>>,
    /// Per var, the `OpDatabase` count at its LATEST binding (`Set`) — a container bound after
    /// a source's backing is allocated is not the one the source's build would land in.
    last_bind: HashMap<u16, usize>,
    /// Walk state: the `OpDatabase` counter, the region-id counter, and the current path.
    idx: usize,
    next_region: u32,
    path: CfPath,
}

/// Pass 1 of [`construct_move_rewrite`]: gather the append destination / backing / container /
/// `OpDatabase` order — and the control-flow position of the build and the append — for every
/// construct source.
fn construct_prescan(node: &Value, co: &ConstructOps, con: &HashSet<u16>, sc: &mut ConstructScan) {
    match node.unspan() {
        Value::Call(d, args) => {
            if *d == co.op_database {
                if let Some(Value::Var(v)) = args.first().map(Value::unspan) {
                    sc.db_order.entry(*v).or_insert(sc.idx);
                    sc.db_path.entry(*v).or_insert_with(|| sc.path.clone());
                }
                sc.idx += 1;
            } else if *d == co.op_append
                && let Some(dst) = args.first()
                && let Some(Value::Var(s)) = args.get(1).map(Value::unspan)
                && con.contains(s)
            {
                if sc.dest.contains_key(s) {
                    // ⚠ NOT idempotent, and this function double-visits a spanned node: the
                    // scrutinee above is peeled while `for_each_child` below walks the ORIGINAL,
                    // and that walk sees through a `Span` itself.  A second visit of the SAME
                    // append lands here and reads as two appends, which silently disqualifies the
                    // var from the construct rewrite.  Measured over the 858-program corpus: 77
                    // first-appends in 45 files and exactly ONE mark, which is genuine — it
                    // survives binding the peel.  So the hazard does not fire today; it is one
                    // edit away from firing.  See `sandbox::intrinsic_space` for the shape biting.
                    sc.ambiguous.insert(*s); // appended into two places — not the clean shape.
                } else {
                    sc.container.insert(*s, get_field_base(dst, co));
                    sc.dest.insert(*s, dst.clone());
                    sc.dest_path.insert(*s, sc.path.clone());
                }
            }
        }
        // `src = OpGetField(vdb, …)` — the source's view over its backing wrapper.
        Value::Set(s, rhs) if con.contains(s) => {
            if let Value::Call(gd, gargs) = rhs.unspan()
                && *gd == co.op_get_field
                && let Some(Value::Var(vd)) = gargs.first().map(Value::unspan)
            {
                sc.vdb.insert(*s, *vd);
            }
        }
        _ => {}
    }
    if let Value::Set(v, _) = node.unspan() {
        let at = sc.last_bind.entry(*v).or_insert(sc.idx);
        *at = (*at).max(sc.idx);
    }
    // A node whose children do NOT run exactly once with it opens a region, so everything
    // below carries a path the statements outside cannot match.  Scoped to the whole node
    // rather than to the arms alone: an `if` CONDITION does run once, but charging it the
    // region too only ever withholds a rewrite, and this walk visits a span-wrapped node
    // twice (the ⚠ above), which arm-precise pushes could not survive.
    let region = matches!(
        node.unspan(),
        Value::Loop(_) | Value::If(_, _, _) | Value::Parallel(_) | Value::Iter(_, _, _, _)
    );
    if region {
        sc.path.push(sc.next_region);
        sc.next_region += 1;
    }
    node.for_each_child(&mut |c| construct_prescan(c, co, con, sc));
    if region {
        sc.path.pop();
    }
}

/// Pass 2 of [`construct_move_rewrite`]: DROP the wrapper/view/prealloc/copy statements and
/// RETARGET each ready source's element-build ops (`OpNewRecord`/`OpFinishRecord`) onto its
/// append destination.
fn construct_rewrite_ops(
    node: &mut Value,
    ready: &HashSet<u16>,
    vdbs: &HashSet<u16>,
    dest: &HashMap<u16, Value>,
    co: &ConstructOps,
) {
    match node {
        Value::Block(b) => {
            b.operators.retain(|s| !construct_drop(s, ready, vdbs, co));
            for op in &mut b.operators {
                construct_rewrite_ops(op, ready, vdbs, dest, co);
            }
        }
        Value::Insert(ops) => {
            ops.retain(|s| !construct_drop(s, ready, vdbs, co));
            for op in &mut *ops {
                construct_rewrite_ops(op, ready, vdbs, dest, co);
            }
        }
        _ => {
            if let Value::Call(d, args) = node {
                // An element-build op (NOT one of the dropped/excluded ops) whose target is a
                // ready source → retarget it onto the append destination.
                let retarget = *d != co.op_database
                    && *d != co.op_prealloc
                    && *d != co.op_append
                    && *d != co.op_free
                    && *d != co.op_set_int4
                    && matches!(args.first().map(Value::unspan),
                        Some(Value::Var(s)) if ready.contains(s));
                if retarget
                    && let Some(Value::Var(s)) = args.first().map(Value::unspan)
                    && let Some(dst) = dest.get(s).cloned()
                {
                    args[0] = dst;
                }
            }
            node.for_each_child_mut(&mut |c| construct_rewrite_ops(c, ready, vdbs, dest, co));
        }
    }
}

/// Retain-predicate for [`construct_rewrite_ops`]: the wrapper's alloc/init/free, the source's
/// view-def + capacity pre-alloc, and the `OpAppendVector` copy are all dropped.
fn construct_drop(
    stmt: &Value,
    ready: &HashSet<u16>,
    vdbs: &HashSet<u16>,
    co: &ConstructOps,
) -> bool {
    match stmt.unspan() {
        // `src = OpGetField(vdb, …)` — the view-def is dead once the builds retarget.
        Value::Set(s, _) => ready.contains(s),
        Value::Call(d, args) => {
            let a0 = args.first().map(Value::unspan);
            if *d == co.op_database || *d == co.op_set_int4 || *d == co.op_free {
                matches!(a0, Some(Value::Var(v)) if vdbs.contains(v)) // wrapper alloc/len-init/free
            } else if *d == co.op_prealloc {
                matches!(a0, Some(Value::Var(s)) if ready.contains(s)) // src capacity hint — omit
            } else if *d == co.op_append {
                // the copy: OpAppendVector(dest, Var(src), …) — src in arg1.
                matches!(args.get(1).map(Value::unspan), Some(Value::Var(s)) if ready.contains(s))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// @PLN90 phase B (B1.3c) — the FRESH-construction move-elision (`a = Bag { items: base }`, the
/// container built AFTER the source). Unlike B1.3b's field-append it needs a build-order REORDER:
/// hoist `a`'s allocation ahead of `base`'s build, retarget `base`'s build ops
/// (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord`) onto `a.field`, drop the backing wrapper +
/// the `OpAppendVector` copy. Runs AFTER [`construct_move_rewrite`], on the copies it left standing.
///
/// Conservative — operates on a FLAT top-level block only, and rewrites each construct that passes
/// EVERY guard: `a`'s construction is a contiguous run of statements immediately before the copy
/// that references only `a` or a **never-written parameter** (B1.4 — a param's value is constant,
/// so hoisting reads the same value; any other var could be a local built between `base` and `a`
/// → SKIP); the run contains `a`'s `OpDatabase`; and `a` is genuinely allocated AFTER the source's
/// backing (a real reorder). B1.4 also lifts the one-construct-per-fn cap — each safe construct is
/// rewritten independently (a cross-construct dependency is a non-`a`, non-param run var → SKIPs
/// that one). B1.4 (nested) walks EVERY block, so a construct inside an `if`/loop body is handled
/// too — its `a`-alloc + `base`-build + copy are flat within that block. Anything else stays a
/// copy. `skip` receives only the moved-out backings.
#[allow(clippy::too_many_arguments)]
pub(super) fn construct_fresh_rewrite(
    code: &mut Value,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) {
    // Apply the per-block reorder to the top block AND every nested block (if/loop bodies etc.).
    if let Value::Block(b) = code {
        fresh_rewrite_block(
            b,
            con_sources,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        );
    }
    code.for_each_child_mut(&mut |c| {
        construct_fresh_rewrite(
            c,
            con_sources,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        );
    });
}

/// Run the fresh-construction reorder over a SINGLE block's operators: rewrite each safe construct,
/// re-scanning after every rewrite (the reorder shifts indices), by earliest remaining copy (a
/// deterministic order); a guard-failing source is recorded so it is not retried (termination).
#[allow(clippy::too_many_arguments)]
fn fresh_rewrite_block(
    b: &mut Block,
    con_sources: &HashSet<u16>,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) {
    let mut failed: HashSet<u16> = HashSet::default();
    loop {
        let next = con_sources
            .iter()
            .copied()
            .filter(|s| !failed.contains(s))
            .filter_map(|s| {
                b.operators
                    .iter()
                    .position(|op| append_copy_of(op, s, co).is_some())
                    .map(|ci| (ci, s))
            })
            .min_by_key(|&(ci, _)| ci);
        let Some((_, src)) = next else {
            break;
        };
        if !try_fresh_one(
            b,
            src,
            co,
            function,
            written,
            bad_containers,
            escaping,
            skip,
        ) {
            failed.insert(src);
        }
    }
}

/// Attempt the fresh-construction reorder for ONE source `src` on the flat block `b`. Returns
/// `true` (and rewrites `b.operators` + records the moved-out backing in `skip`) iff every guard
/// holds; `false` otherwise, leaving `b` unchanged.
#[allow(clippy::too_many_arguments)]
fn try_fresh_one(
    b: &mut Block,
    src: u16,
    co: &ConstructOps,
    function: &Function,
    written: &HashSet<u16>,
    bad_containers: &HashSet<u16>,
    escaping: &HashSet<u16>,
    skip: &mut HashSet<u16>,
) -> bool {
    if escaping.contains(&src) {
        return false; // read between build and copy (or append-grown) → not safe to retarget.
    }
    // Copy index + destination expression (`a.field`) — the first copy of `src`.
    let mut ci = None;
    let mut dest = None;
    for (i, op) in b.operators.iter().enumerate() {
        if let Some(d) = append_copy_of(op, src, co) {
            ci = Some(i);
            dest = Some(d);
            break;
        }
    }
    let (Some(ci), Some(dest)) = (ci, dest) else {
        return false;
    };
    let Some(a) = get_field_base(&dest, co) else {
        return false;
    };
    if bad_containers.contains(&a) {
        return false; // a REASSIGNED container has a prior store the hoist does not retire.
    }

    // The source's backing wrapper (`src = OpGetField(vdb, …)`).
    let mut vdb = None;
    for op in &b.operators {
        if let Value::Set(s, rhs) = op.unspan()
            && *s == src
            && let Value::Call(gd, gargs) = rhs.unspan()
            && *gd == co.op_get_field
            && let Some(Value::Var(v)) = gargs.first().map(Value::unspan)
        {
            vdb = Some(*v);
        }
    }
    let Some(vdb) = vdb else {
        return false;
    };

    // `a`'s construction run: the contiguous block [ps..ci) whose statements all target `a`.
    let mut ps = ci;
    while ps > 0 && stmt_targets_var(&b.operators[ps - 1], a) {
        ps -= 1;
    }
    let run = &b.operators[ps..ci];
    // Guards: the run allocates `a`, references only `a` or a never-written param (dependency-safe
    // to hoist past `base`), and `a` is allocated AFTER the backing (else this isn't a reorder).
    if !run.iter().any(|s| call_is(s, co.op_database, a)) {
        return false;
    }
    if !run.iter().all(|s| run_var_ok(s, a, function, written)) {
        return false;
    }
    match b
        .operators
        .iter()
        .position(|s| call_is(s, co.op_database, vdb))
    {
        Some(vi) if vi < ps => {}
        _ => return false,
    }

    // Rebuild: [a-construction run] ++ [base's build, wrapper dropped + retargeted] ++ [rest].
    let ops = std::mem::take(&mut b.operators);
    let mut new_ops = Vec::with_capacity(ops.len());
    for op in &ops[ps..ci] {
        new_ops.push(op.clone());
    }
    for (i, op) in ops.into_iter().enumerate() {
        if (ps..=ci).contains(&i) {
            continue; // run hoisted above; the copy at `ci` is dropped.
        }
        if fresh_drop(&op, src, vdb, co) {
            continue;
        }
        let mut op = op;
        fresh_retarget(&mut op, src, &dest, co);
        new_ops.push(op);
    }
    b.operators = new_ops;
    skip.insert(vdb); // the backing wrapper is the moved-out owned store.
    true
}

/// If `op` is `OpAppendVector(dest, Var(src), …)` (the copy of `src` into a field), return `dest`.
fn append_copy_of(op: &Value, src: u16, co: &ConstructOps) -> Option<Value> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && let Some(Value::Var(s)) = args.get(1).map(Value::unspan)
        && *s == src
    {
        return args.first().cloned();
    }
    None
}

/// Does `stmt` write into variable `v` (its `Set` target, or a `Call`'s first arg)?
fn stmt_targets_var(stmt: &Value, v: u16) -> bool {
    match stmt.unspan() {
        Value::Set(s, _) => *s == v,
        Value::Call(_, args) => {
            matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == v)
        }
        _ => false,
    }
}

/// Is `stmt` the call `op` with first arg `Var(v)` (e.g. `OpDatabase(v, …)`)?
fn call_is(stmt: &Value, op: u32, v: u16) -> bool {
    matches!(stmt.unspan(), Value::Call(d, args) if *d == op
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == v))
}

/// Does every `Var` referenced anywhere in `stmt`'s value positions equal `only`? (A `Set`/`Call`
/// TARGET is `only` by construction; this checks the RHS/args carry no dependency on another var.)
/// Every `Var` in `stmt`'s subtree is either `a` or a never-written PARAMETER (whose value is
/// therefore constant, so hoisting `a`'s construction past `base`'s build reads the same value).
/// Any other var — a local that might be built between `base` and `a` — fails, so the construct
/// stays a copy. (`written` is the fn-wide over-approximation from [`collect_written`].)
fn run_var_ok(stmt: &Value, a: u16, function: &Function, written: &HashSet<u16>) -> bool {
    fn walk(node: &Value, a: u16, function: &Function, written: &HashSet<u16>, ok: &mut bool) {
        if let Value::Var(v) = node.unspan() {
            let v = *v;
            let allowed = v == a || (function.is_argument(v) && !written.contains(&v));
            if !allowed {
                *ok = false;
            }
        }
        node.for_each_child(&mut |c| walk(c, a, function, written, ok));
    }
    let mut ok = true;
    walk(stmt, a, function, written, &mut ok);
    ok
}

/// Retain-drop for [`construct_fresh_rewrite`]: the backing wrapper's alloc/len-init/free and the
/// source's view-def (`src = OpGetField(vdb, …)`).
fn fresh_drop(stmt: &Value, src: u16, vdb: u16, co: &ConstructOps) -> bool {
    match stmt.unspan() {
        Value::Set(s, _) => *s == src,
        Value::Call(d, args) => {
            (*d == co.op_database || *d == co.op_set_int4 || *d == co.op_free)
                && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == vdb)
        }
        _ => false,
    }
}

/// Retarget every source-targeting build op (`OpPreAllocVector`/`OpNewRecord`/`OpFinishRecord` —
/// NOT the dropped alloc/append/free/set-int4/get-field ops) onto the `dest` field. Unlike the
/// field-append path, the capacity `OpPreAllocVector` IS retargeted here: the fresh field is empty,
/// so pre-claiming its capacity is correct (it mirrors the source's own initial claim).
fn fresh_retarget(node: &mut Value, src: u16, dest: &Value, co: &ConstructOps) {
    if let Value::Call(d, args) = node {
        let hit = *d != co.op_database
            && *d != co.op_append
            && *d != co.op_free
            && *d != co.op_set_int4
            && *d != co.op_get_field
            && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == src);
        if hit {
            args[0] = dest.clone();
        }
    }
    node.for_each_child_mut(&mut |c| fresh_retarget(c, src, dest, co));
}

/// @PLN90 phase B (B1.3d) — the `a.field = base` whole-vector REPLACEMENT, a DOUBLE copy the
/// compiler lowers as `base → __p154_rhs → a.field` with an `OpClearVector` between:
///
/// ```text
///   <build base into __vdb>
///   __p154_rhs = null; OpAppendVector(__p154_rhs, base);   (copy 1 — base → temp)
///   OpClearVector(a.field);                                (clear a.field's old contents)
///   OpAppendVector(a.field, __p154_rhs);                   (copy 2 — temp → a.field)
/// ```
///
/// `base`'s copy target is a temp (`__p154_rhs`), so it is not a `MovePlan`; this rewrite detects
/// the idiom STRUCTURALLY. When `base` is a dead-after local (its own `__vdb` backing), build it
/// DIRECTLY into the cleared `a.field`: move the `OpClearVector` ahead of `base`'s build, retarget
/// `base`'s build ops onto `a.field`, and drop the temp + both copies + the wrapper. Both temp and
/// wrapper join `skip` (their frees are suppressed). Walks every block (nested `if`/loop bodies too);
/// conservative.
pub(super) fn construct_replace_rewrite(
    code: &mut Value,
    co: &ConstructOps,
    skip: &mut HashSet<u16>,
) {
    if let Value::Block(b) = code {
        replace_rewrite_block(b, co, skip);
    }
    code.for_each_child_mut(&mut |c| construct_replace_rewrite(c, co, skip));
}

/// Run the whole-vector-replacement rewrite over a SINGLE block's operators.
fn replace_rewrite_block(b: &mut Block, co: &ConstructOps, skip: &mut HashSet<u16>) {
    let mut failed: HashSet<usize> = HashSet::default();
    loop {
        // Find the earliest `copy 2` (`OpAppendVector(a.field, Var(rhs))`) preceded by the clear +
        // `copy 1`, not yet tried.
        let mut hit = None;
        for c2 in 2..b.operators.len() {
            if failed.contains(&c2) {
                continue;
            }
            let Some((field, rhs)) = append_field_temp(&b.operators[c2], co) else {
                continue;
            };
            // Preceding statement must be `OpClearVector(field)` for the SAME field.
            if !is_clear_of(&b.operators[c2 - 1], &field, co) {
                continue;
            }
            // And the one before that `OpAppendVector(Var(rhs), Var(base))`.
            let Some(base) = append_temp_src(&b.operators[c2 - 2], rhs, co) else {
                continue;
            };
            hit = Some((c2, field, rhs, base));
            break;
        }
        let Some((c2, field, rhs, base)) = hit else {
            break;
        };
        if !try_replace_one(b, c2, &field, rhs, base, co, skip) {
            failed.insert(c2);
        }
    }
}

/// One `a.field = base` replacement at `copy 2` index `c2`. Rewrites `b.operators` + records the
/// moved-out temp/wrapper in `skip` iff every guard holds; else returns `false` unchanged.
fn try_replace_one(
    b: &mut Block,
    c2: usize,
    field: &Value,
    rhs: u16,
    base: u16,
    co: &ConstructOps,
    skip: &mut HashSet<u16>,
) -> bool {
    let Some(a) = get_field_base(field, co) else {
        return false;
    };
    // `base` must be a dead-after LOCAL: it owns a backing wrapper `base = OpGetField(vdb, …)`, and
    // is not referenced after `copy 2` (the store transfers, so a later read would dangle).
    let mut vdb = None;
    let mut rhs_set = None;
    for (i, op) in b.operators.iter().enumerate() {
        match op.unspan() {
            Value::Set(s, r) if *s == base => {
                if let Value::Call(gd, ga) = r.unspan()
                    && *gd == co.op_get_field
                    && let Some(Value::Var(v)) = ga.first().map(Value::unspan)
                {
                    vdb = Some(*v);
                }
            }
            Value::Set(s, _) if *s == rhs => rhs_set = Some(i),
            _ => {}
        }
    }
    let (Some(vdb), Some(_)) = (vdb, rhs_set) else {
        return false;
    };
    if a == base || a == vdb {
        return false; // paranoia: the destination container must be independent of the source
    }
    if references_var_after(b, base, c2) {
        return false;
    }
    // `base`'s build start: the first statement that sets up its wrapper or references it. Everything
    // before it (`a`'s already-built value + null-inits) is kept; the clear is inserted there.
    let Some(bs) = b
        .operators
        .iter()
        .position(|op| fresh_drop(op, base, vdb, co) || refs_var(op, base))
    else {
        return false;
    };
    if bs >= c2 - 2 {
        return false; // base built after the field already exists but not as a real preceding build
    }
    // The container `a` must ALREADY EXIST at `base`'s build (we move the clear + build to `bs`, so
    // `a.field` must be valid there). If `a` is allocated in THIS block AFTER `base` (`fresh = …;
    // s = S{…}; s.field = fresh`), building into `a.field` at `bs` would hit an un-allocated `a` —
    // SKIP. A param / outer-scope `a` has no `OpDatabase` here → it pre-exists → fine.
    if let Some(a_db) = b
        .operators
        .iter()
        .position(|op| call_is(op, co.op_database, a))
        && a_db >= bs
    {
        return false;
    }
    let chain: HashSet<usize> = [rhs_set.unwrap(), c2 - 2, c2 - 1, c2].into_iter().collect();
    // `base`'s BUILD must not read the destination container `a` (a SELF-ASSIGN like
    // `s.v = s.v[1..]` builds the source by slicing `s.v` — moving the `OpClearVector` ahead of that
    // read would empty `s.v` before the slice copies it). Guard the build region [bs..c2) (excluding
    // the chain: copy1 / clear / copy2 legitimately reference `a`).
    if (bs..c2).any(|i| !chain.contains(&i) && refs_var(&b.operators[i], a)) {
        return false;
    }
    // `@FR-R-MoveLast` — nor may the region BIND `a`: the build lands in the `a.field` that exists
    // at `bs`, and a rebind before the store (`m9 = mo`, a call result, an arm of an `if`) puts a
    // different record there, so the built elements were lost with no diagnostic on both
    // backends — and on `--native` a FRESH `a` was used before its `let` (loft#1741).  An
    // `OpDatabase` of `a` after `bs` is refused above; a binding by assignment allocates nothing,
    // which is how it passed as "pre-existing".
    if (bs..c2).any(|i| !chain.contains(&i) && binds_var(&b.operators[i], a)) {
        return false;
    }

    let clear = b.operators[c2 - 1].clone();
    let ops = std::mem::take(&mut b.operators);
    let mut new_ops = Vec::with_capacity(ops.len());
    for (i, op) in ops.into_iter().enumerate() {
        if i == bs {
            new_ops.push(clear.clone()); // clear a.field BEFORE building base into it
        }
        if i < bs {
            new_ops.push(op);
            continue;
        }
        if chain.contains(&i) || fresh_drop(&op, base, vdb, co) {
            continue;
        }
        let mut op = op;
        fresh_retarget(&mut op, base, field, co); // base's build → a.field
        new_ops.push(op);
    }
    b.operators = new_ops;
    skip.insert(vdb);
    skip.insert(rhs);
    true
}

/// If `op` is `OpAppendVector(OpGetField(…), Var(rhs))` (copy INTO a field from a temp), return
/// `(field_expr, rhs)`.
fn append_field_temp(op: &Value, co: &ConstructOps) -> Option<(Value, u16)> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && let Some(field) = args.first()
        && get_field_base(field, co).is_some()
        && let Some(Value::Var(rhs)) = args.get(1).map(Value::unspan)
    {
        return Some((field.clone(), *rhs));
    }
    None
}

/// Is `op` `OpClearVector(field)` for the given `field` expression?
fn is_clear_of(op: &Value, field: &Value, co: &ConstructOps) -> bool {
    matches!(op.unspan(), Value::Call(d, args) if *d == co.op_clear
        && args.first().map(Value::unspan) == Some(field.unspan()))
}

/// Is `field` cleared (`OpClearVector(field)`) ANYWHERE in `node`'s subtree? A cleared destination
/// means the append is really a whole-vector REPLACE, not a pure `+=` append.
fn field_is_cleared(node: &Value, field: &Value, co: &ConstructOps) -> bool {
    fn walk(node: &Value, field: &Value, co: &ConstructOps, found: &mut bool) {
        if is_clear_of(node, field, co) {
            *found = true;
        }
        node.for_each_child(&mut |c| walk(c, field, co, found));
    }
    let mut found = false;
    walk(node, field, co, &mut found);
    found
}

/// If `op` is `OpAppendVector(Var(rhs), Var(base))` (copy INTO the temp from `base`), return `base`.
fn append_temp_src(op: &Value, rhs: u16, co: &ConstructOps) -> Option<u16> {
    if let Value::Call(d, args) = op.unspan()
        && *d == co.op_append
        && matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == rhs)
        && let Some(Value::Var(base)) = args.get(1).map(Value::unspan)
    {
        return Some(*base);
    }
    None
}

/// Does `op` reference `Var(v)` anywhere in its subtree?
fn refs_var(op: &Value, v: u16) -> bool {
    fn walk(node: &Value, v: u16, found: &mut bool) {
        if matches!(node.unspan(), Value::Var(x) if *x == v) {
            *found = true;
        }
        node.for_each_child(&mut |c| walk(c, v, found));
    }
    let mut found = false;
    walk(op, v, &mut found);
    found
}

/// Does `op` BIND `v` anywhere inside it — a `Set(v, …)`, at any depth?  `refs_var`'s twin for
/// the other half of a variable's life: a rebind names `v` as a target id, not as a `Var` node,
/// so a read-only walk passes over it (loft#1741).
fn binds_var(op: &Value, v: u16) -> bool {
    fn walk(node: &Value, v: u16, found: &mut bool) {
        if matches!(node.unspan(), Value::Set(x, _) if *x == v) {
            *found = true;
        }
        node.for_each_child(&mut |c| walk(c, v, found));
    }
    let mut found = false;
    walk(op, v, &mut found);
    found
}

/// Is `Var(v)` referenced in any operator strictly after index `idx`?
fn references_var_after(b: &Block, v: u16, idx: usize) -> bool {
    b.operators[idx + 1..].iter().any(|op| refs_var(op, v))
}
