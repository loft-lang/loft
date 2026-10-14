// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Debug checks after the scan: every reference local that should be released is, and no call
//! argument allocates a store nothing owns.

use super::capture_adoption::{closure_record_leaves_frame, link_written_closure_records};
use super::capture_builds::{
    CaptureBuilds, capture_adoption_owns_free, capture_build_backings, escaping_record_holds_buffer,
};
use super::returns::{collect_return_sources, returned_var_null_unified};
use crate::data::{Data, Type, Value};
use crate::fxhash::FxHashSet as HashSet;
use crate::variables::Function;
#[cfg(debug_assertions)]
use std::collections::BTreeMap;

/// Walk `ir` and panic if any `Call` or `CallRef` argument directly contains a
/// `Set(ref_var, Null)` for an owned Reference (dep empty).
///
/// Such a nested allocation would place `ConvRefFromNull` on the eval stack
/// *between* call arguments, corrupting the arg layout and producing a garbage
/// `y` value inside the callee — the root cause of the A5.6 "Incorrect store"
/// bug.  `scan_args` in scopes.rs is responsible for bubbling these out; if it
/// misses a case this check catches it at compile time.
#[cfg(debug_assertions)]
pub(super) fn check_arg_ref_allocs(ir: &Value, function: &Function, fn_name: &str) {
    fn check_args(args: &[Value], function: &Function, fn_name: &str) {
        for a in args {
            if let Value::Insert(ops) = a
                && ops.len() >= 2
                && let Value::Set(v, val) = &ops[0]
                && matches!(val.as_ref(), Value::Null)
                && function.tp(*v).is_heap_owned()
            {
                panic!(
                    "[check_arg_ref_allocs] Set('{name}', Null) for owned heap type \
                     is nested inside a Call/CallRef argument in '{fn_name}'. \
                     This corrupts the CallRef arg layout (A5.6). \
                     scan_args in scopes.rs must bubble it out.",
                    name = function.name(*v),
                );
            }
            walk_check(a, function, fn_name);
        }
    }
    fn walk_check(ir: &Value, function: &Function, fn_name: &str) {
        match ir {
            Value::Call(_, args) | Value::CallRef(_, args) => {
                check_args(args, function, fn_name);
            }
            Value::Set(_, inner) => walk_check(inner, function, fn_name),
            Value::If(cond, t, f) => {
                walk_check(cond, function, fn_name);
                walk_check(t, function, fn_name);
                walk_check(f, function, fn_name);
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &bl.operators {
                    walk_check(op, function, fn_name);
                }
            }
            Value::Insert(ops) => {
                for op in ops {
                    walk_check(op, function, fn_name);
                }
            }
            // Every value-WRAPPING node, not a chosen subset: the shape this walker
            // reports — a `Set(v, Null)` nested inside a call argument — can sit under any
            // of them, so one omitted wrapper is a silently missed diagnostic.
            Value::Return(inner) | Value::Drop(inner) | Value::Yield(inner) => {
                walk_check(inner, function, fn_name);
            }
            _ => {}
        }
    }
    walk_check(ir, function, fn_name);
}

/// @PLN85 cluster II — the SET version of `returned_var`: every terminal var a
/// return expression can yield, INCLUDING all arms of an `If`/`match` (which
/// `returned_var` collapses to `u16::MAX` when the arms differ). These are the
/// function's "return-source" locals — their heap store is transferred to the
/// caller, so the callee must not free them at scope exit.
/// Every variable ANY `return` in this body can hand back — including an EARLY return
/// nested mid-block, which the two helpers below cannot see.
///
/// Both of them answer for the body's TAIL value: `returned_var_null_unified` keeps the
/// last operator's answer and `collect_return_sources` takes a block's last non-free
/// result.  So `if a { return a?; } x` reports only `x`, and the work var the guard arm
/// delivers looks like a local nobody frees — which is exactly what it is NOT, because
/// returning it transfers it to the caller.  `check_ref_leaks` asserted on that shape as a
/// leak (a generic instantiated at a STRUCT, where the return is a heap ref).
fn collect_all_return_vars(expr: &Value, data: &Data, out: &mut Vec<u16>) {
    if let Value::Return(inner) = expr.unspan() {
        collect_return_sources(inner, data, out);
    }
    expr.walk(&mut |v| {
        if let Value::Return(inner) = v {
            collect_return_sources(inner, data, out);
        }
    });
    // …and through the value STAGED into a returned variable.  `free_vars` reads its
    // suppression set off the return EXPRESSION (`collect_return_sources` there), and the
    // B5-L3 hoist then rewrites that expression to `Set(__ret_N, <expr>); …;
    // Return(Var(__ret_N))` — so by the time this mirror runs, the IR names only the temp
    // and every source the emitter suppressed sits one assignment away.  Closing the set
    // under "assigned into a transferred var" is what keeps the two readings the same
    // question; without it a work-ref the emitter deliberately left unfreed reads here as a
    // leak (`match p { [a, ..] => a, _ => null }` over a `vector<S?>`, whose payload copy
    // lands in a `__ref_N` the hoist hides).
    //
    // The closure is the emitter's own rule, not a wider one: `collect_return_sources`
    // unions an `If`'s arms exactly as `free_vars` does, so a source live on one path only
    // is credited on that path and its sibling orphan is still released by the
    // `OpFreeRefIfDistinct` the hoist emits beside the `Set`.
    let mut i = 0;
    while i < out.len() {
        let staged = out[i];
        i += 1;
        expr.walk(&mut |v| {
            if let Value::Set(target, rhs) = v
                && *target == staged
            {
                collect_return_sources(rhs, data, out);
            }
        });
    }
}

/// Recursively collect every variable freed by `OpFreeRef` in `ir`.
/// Used by `check_ref_leaks` to verify no Reference variable is leaked.
fn collect_freed_vars(ir: &Value, free_ops: &[u32], result: &mut HashSet<u16>) {
    ir.walk(&mut |n| {
        if let Value::Call(d_nr, args) = n
            && free_ops.contains(d_nr)
            && let Some(Value::Var(v)) = args.first().map(Value::unspan)
        {
            result.insert(*v);
        }
    });
}

/// Block-tail temps whose store is ADOPTED (moved) into a freed assignment LHS,
/// so freeing the LHS frees them — no `OpFreeRef` of the temp is emitted, and
/// none should be (it would double-free the shared record).
///
/// The shape is `Set(lhs, Block[…, Var(v)])`: the block allocates a fresh record
/// into `v` and yields it, and `lhs = <block>` PutRef-aliases that record into
/// `lhs` (the empty-dep adopt — e.g. the `#reading file` surface temp behind
/// `q = f#read as S`).  `v`'s free responsibility transfers to `lhs`, so when
/// `lhs` is freed, `v` is covered.
///
/// Narrow by construction: it matches only a BLOCK-valued RHS (a plain
/// `lhs = v` COPY has an RHS of `Var(v)`, not `Block`, and its `v` is freed
/// separately, so `v` is already in `freed` and never reaches the leak assert).
/// It credits `v` only when `lhs` is in `freed`, so it cannot mask a genuine
/// leak where the adopting LHS itself is never freed.
fn collect_adopted_block_results(ir: &Value, freed: &HashSet<u16>, result: &mut HashSet<u16>) {
    ir.walk(&mut |n| {
        if let Value::Set(lhs, rhs) = n
            && freed.contains(lhs)
            && let Value::Block(bl) = rhs.unspan()
            && let Some(Value::Var(v)) = bl.operators.last().map(Value::unspan)
        {
            result.insert(*v);
        }
    });
}

/// Whether `v`'s store is released through the variables it VIEWS: every binding of `v` ends
/// in a record that lives in some freed variable's store (or a parameter's, which the caller
/// frees), so `v` owns no store of its own and a free of `v` is not owed.  The root is found
/// through a block's or insert's tail, BOTH arms of an `if` (a `??` discharge answers the
/// element on one arm and a default record on the other), a projection's base (`items[0]`
/// lives in the store of `items`' record), and another local whose own bindings resolve the
/// same way.
///
/// Two shapes printed `check_ref_leaks`'s text-work-deps warning (*"Store will leak at
/// runtime"*) about a store that IS released, because the warning asks only what the deps
/// are NAMED: a tuple local a `&(…)` link names is BUILT in its work ref
/// (`t = { OpDatabase(__ref_1); …; __ref_1 }`, `tuples.md (T-Ref-Rep)`) — 12 times in the
/// loft#1673 guard — and `e = make().items[0] ?? P {}` VIEWS either the container's work ref or
/// the default's.
///
/// Answers `false` for any binding it cannot root — a call, a mint, a literal — which keeps the
/// warning: the case it exists for, a struct that COPIED a text yet kept the work ref's dep,
/// owns a store of its own.
#[cfg(debug_assertions)]
fn is_a_freed_backing(
    ir: &Value,
    v: u16,
    function: &Function,
    data: &Data,
    freed: &HashSet<u16>,
) -> bool {
    let mut visiting = HashSet::default();
    view_rooted_in_freed(ir, v, function, data, freed, &mut visiting)
}

#[cfg(debug_assertions)]
fn view_rooted_in_freed(
    ir: &Value,
    v: u16,
    function: &Function,
    data: &Data,
    freed: &HashSet<u16>,
    visiting: &mut HashSet<u16>,
) -> bool {
    if !visiting.insert(v) {
        return false;
    }
    let mut rhss = Vec::new();
    ir.walk(&mut |n| {
        if let Value::Set(lhs, rhs) = n
            && *lhs == v
        {
            rhss.push((**rhs).clone());
        }
    });
    if rhss.is_empty() {
        return false;
    }
    rhss.iter()
        .all(|rhs| rooted(rhs, ir, v, function, data, freed, visiting))
}

/// Member `i` of the tuple LITERAL a binding ends in, or `None` for any other right-hand side.
#[cfg(debug_assertions)]
fn tuple_literal_member(rhs: &Value, i: u16) -> Option<Value> {
    match rhs.unspan() {
        Value::Tuple(elems) => elems.get(i as usize).cloned(),
        Value::Block(bl) => bl.operators.last().and_then(|t| tuple_literal_member(t, i)),
        Value::Insert(ops) => ops.last().and_then(|t| tuple_literal_member(t, i)),
        _ => None,
    }
}

#[cfg(debug_assertions)]
fn rooted(
    rhs: &Value,
    ir: &Value,
    v: u16,
    function: &Function,
    data: &Data,
    freed: &HashSet<u16>,
    visiting: &mut HashSet<u16>,
) -> bool {
    match rhs.unspan() {
        Value::Var(d) if *d == v => false,
        Value::Var(d) => {
            freed.contains(d)
                || function.is_argument(*d)
                || view_rooted_in_freed(ir, *d, function, data, freed, visiting)
        }
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|t| rooted(t, ir, v, function, data, freed, visiting)),
        Value::Insert(ops) => ops
            .last()
            .is_some_and(|t| rooted(t, ir, v, function, data, freed, visiting)),
        Value::If(_, a, b) => {
            rooted(a, ir, v, function, data, freed, visiting)
                && rooted(b, ir, v, function, data, freed, visiting)
        }
        // A `null` binding holds no store at all, in either spelling.
        Value::Null => true,
        Value::Call(d, _) if data.def(*d).name() == "OpNullRefSentinel" => true,
        // A destructured member (`m = __ref_3.1`) views whatever the tuple's literal put there.
        Value::TupleGet(base, i) => {
            let mut members = Vec::new();
            ir.walk(&mut |n| {
                if let Value::Set(lhs, rhs) = n
                    && lhs == base
                {
                    members.push(tuple_literal_member(rhs, *i));
                }
            });
            !members.is_empty()
                && members.iter().all(|m| {
                    m.as_ref()
                        .is_some_and(|m| rooted(m, ir, v, function, data, freed, visiting))
                })
        }
        // A nullable element read (`OpGetVectorNullable`, `OpVectorRefNullable`) is a projection
        // exactly as its non-null twin is, so the twin is what is asked: the shared predicate
        // names only the four the borrow walk needs, and a list of the nullable spellings here
        // would miss the next one.
        Value::Call(d, args)
            if crate::use_analysis::is_projection_op(data, *d)
                || data
                    .def(*d)
                    .name()
                    .strip_suffix("Nullable")
                    .map(|twin| data.def_nr(twin))
                    .is_some_and(|t| {
                        t != u32::MAX && crate::use_analysis::is_projection_op(data, t)
                    }) =>
        {
            args.first()
                .is_some_and(|base| rooted(base, ir, v, function, data, freed, visiting))
        }
        _ => false,
    }
}

/// What a function's lowered body says about who releases each local's store — the facts a
/// STATIC leak mirror needs before it may call a local with no frame free a leak.
///
/// One home for two mirrors of `get_free_vars`: [`check_ref_leaks`] (the debug-build assert)
/// and `ownership_cfg`'s leak scan (`LOFT_OWN_ORACLE=check-leak`).  Each suppression leg the
/// emitter has grown had to be taught to each mirror separately, and the scan, holding only the
/// first, reported the other legs' stores as leaks: the literal buffer behind an escaping
/// nullable capture, a closure written out through a `&fn` link, a `_read_N` moved into its
/// destination.
pub(crate) struct FrameReleases {
    pub(crate) freed: HashSet<u16>,
    pub(crate) adopted: HashSet<u16>,
    pub(crate) ret_deps: HashSet<u16>,
    pub(crate) direct_ret_var: u16,
    pub(crate) built_with: CaptureBuilds,
    pub(crate) link_delivered: Vec<u16>,
    pub(crate) fn_def_nr: u32,
    /// `(R-Place)` buffers whose release their adopting local carries (`place_result`).
    pub(crate) placed: HashSet<u16>,
}

impl FrameReleases {
    pub(crate) fn of(
        ir: &Value,
        function: &Function,
        data: &Data,
        fn_name: &str,
        ret_type: &Type,
    ) -> Self {
        // Every op that FREES its first argument counts as that var's free
        // site: the plain scope-exit free, the @P317 tag-checked free, and
        // the witness-pair conditional free (`OpFreeRefIfDistinct` — how a
        // ref-returning call's work ref is released when it doesn't alias
        // the assigned var; the armed-corpus sweep's ~130 "no OpFreeRef"
        // false positives were all this shape).
        // ⚠ `OpFreeRefOrHandUp` belongs here for the same reason the other three do, and its
        // absence is what made this assert fire on `n___lambda_3`'s `__ref_p2_1` — a store that
        // IS released, by an op this list had never heard of.  loft#1186 added it (D-clo-13) as
        // `OpFreeRefIfDistinct` with an owner on the not-distinct leg, on the EMIT side only:
        // three files emit it and nineteen name its sibling, so every matcher keyed on the op
        // NAME went blind to the new spelling at once.  A free-op list is a claim about a
        // NOTION — "this op releases its first argument" — and each new spelling of that notion
        // has to arrive here too, or the assert reports a leak the compiler does not have.
        let sets = data.op_sets();
        let free_ops: Vec<u32> = sets
            .unconditional_ref_frees
            .iter()
            .chain(sets.conditional_ref_frees.iter())
            .copied()
            .collect();
        let mut freed: HashSet<u16> = HashSet::default();
        collect_freed_vars(ir, &free_ops, &mut freed);

        // A block-tail temp adopted into a freed LHS (`q = f#read as S`, whose
        // `#reading file` surface temp `_read_N` moves its record into `q`) has no
        // OpFreeRef of its own and must not — `q`'s free covers it.  Credit it so
        // the leak assert below does not false-positive on the moved-from source.
        let mut adopted: HashSet<u16> = HashSet::default();
        collect_adopted_block_results(ir, &freed, &mut adopted);

        // H2: `ret_type` deps are ATTRIBUTE indices — translate each to its
        // frame var through the attribute name before pooling with the
        // frame-space deps below (the old code inserted them raw, so an attr
        // index colliding with an unrelated var number silently suppressed a
        // leak report).
        let fn_def_nr = data.def_nr(fn_name);
        let mut ret_deps: HashSet<u16> = HashSet::default();
        for raw in ret_type.depend() {
            match crate::data::DepEntry::decode(raw) {
                crate::data::DepEntry::Attr(a) => {
                    let a_idx = a as usize;
                    if fn_def_nr != u32::MAX && a_idx < data.def(fn_def_nr).attributes().len() {
                        let av = function.var(&data.def(fn_def_nr).attributes()[a_idx].name);
                        if av != u16::MAX {
                            ret_deps.insert(av);
                        }
                    }
                }
                // H2 step 5: a tagged callee-frame note IS a frame var — pool
                // it directly (the untagged value was silently dropped by the
                // attr-range guard before, so a returned closure's work var
                // could surface as a false leak report).
                crate::data::DepEntry::CalleeFrame(w) => {
                    ret_deps.insert(w);
                }
            }
        }
        // The directly-returned variable (e.g. the owned struct constructed by a function
        // whose return type is Reference) passes ownership to the caller — no FreeRef is
        // emitted for it and that is correct.  Exclude it so check_ref_leaks does not
        // false-positive on `fn foo() -> S { S { ... } }`.
        let direct_ret_var = returned_var_null_unified(ir, data.def_nr("OpNullRefSentinel"));
        // Transitive: if the returned variable depends on another variable, that
        // variable's store must also survive — include it in ret_deps.
        if direct_ret_var != u16::MAX {
            for d in function.tp(direct_ret_var).depend() {
                ret_deps.insert(d);
            }
        }
        // …and every variable an EARLY return hands back, which the tail-value helper above
        // cannot see: `if a { return a?; } x` reports only `x`, so the guard arm's work var
        // read as a local nobody freed.  Returning it IS the transfer, wherever the return sits.
        let mut early_ret: Vec<u16> = Vec::new();
        collect_all_return_vars(ir, data, &mut early_ret);
        for v in early_ret {
            ret_deps.insert(v);
            for d in function.tp(v).depend() {
                ret_deps.insert(d);
            }
        }

        // The build facts the free EMITTER decided on, read off the body before the scope pass
        // rewrote it (`Scopes::capture_build_backing`).  Recomputed from the lowered body they
        // answer about a different program: loft#1715's null-capture rewrite moves the
        // capture's backing, and the mirror then reported the buffer the emitter had rightly
        // left to the closure record.  A body restored from the IR cache carries none, and
        // the lowered body is the only fact left there.
        let built_with = function
            .capture_builds()
            .cloned()
            .unwrap_or_else(|| capture_build_backings(data, function, ir));
        let link_delivered = link_written_closure_records(data, function, fn_def_nr);
        let placed = placed_buffers_released_by_their_local(ir, data, function);
        FrameReleases {
            freed,
            adopted,
            ret_deps,
            direct_ret_var,
            built_with,
            link_delivered,
            fn_def_nr,
            placed,
        }
    }

    /// Does something other than a frame-exit free release `v`'s store, by a leg the free
    /// emitter itself uses?  Each leg is a CALL to the emitter's own predicate, never a
    /// restatement of it.
    pub(crate) fn explains(&self, data: &Data, function: &Function, v: u16) -> bool {
        // #323: a heap local a closure record ADOPTS is owned by that record (which
        // stores its 12-byte DbRef; `free_named`'s cascade frees it when the record
        // dies), so `get_free_vars` emits no frame-exit free for it and "unfreed" is not
        // "leaked" here.
        //
        // The same call the emitter makes, not the rule written out again.  This mirror
        // going out of step with it is exactly how loft#1308 stayed hidden, and a mirror
        // that knows only the `is_captured` half calls the BACKING local of a collection
        // capture a leak — which no closure captured by name.
        if capture_adoption_owns_free(data, function, &self.built_with, v) {
            return true;
        }
        // …and the BUFFER that minted such a store, which the emitter releases through the
        // same cascade by a predicate of its own (loft#1446's leg in `get_free_vars`).  Asked
        // of the buffer rather than of the capture's name, because a buffer names ONE store
        // for its whole life while a capture local reassigned after the build names two — and
        // asked here because a mirror that knows only `capture_adoption_owns_free` reports the
        // literal buffer behind an escaping nullable capture as a leak (`n: C39? = C39 { … };
        // fn() -> integer { … n.a … }`, whose `__ref_p2_N` the record's cascade frees).
        if escaping_record_holds_buffer(data, function, self.fn_def_nr, &self.built_with, v) {
            return true;
        }
        // …and a closure record this frame WRITES OUT through a `&fn(…)` link, which the
        // emitter suppresses on the same reading (`link_delivered` in `get_free_vars`): a
        // write through a link delivers exactly as a `return` does, so the caller holds the
        // record and the frame owes no free.  The third suppression leg this mirror has had
        // to learn, and the reason each is a CALL to the emitter's own predicate rather than
        // a restatement of it.
        // A record written out through a link is skipped whether or not `(L-CapKeep)` decides
        // the delivery at run time: on a frame where the write always happens the emitter owes
        // no free, and where it does not, the free it emits is guarded by store identity — a
        // static mirror can assert neither.
        if self.link_delivered.contains(&v) {
            return true;
        }
        // …and a closure record that leaves the frame with a record holding it through a
        // captured fn-ref (loft#1869): `fn plus1(k) { dbl = fn(a) { a * k }; fn(a) { dbl(a)
        // + 1 } }` returns the outer record, whose cascade releases `dbl`'s.  The emitter's
        // own predicate, not a restatement — the mirror had only the delivered-store half,
        // through the return's deps, and called the held record a leak.
        if self.fn_def_nr != u32::MAX
            && closure_record_leaves_frame(data, function, self.fn_def_nr, v)
        {
            return true;
        }
        if v == self.direct_ret_var {
            return true; // ownership transferred to caller
        }
        if self.adopted.contains(&v) {
            return true; // moved into a freed LHS — that free covers this store
        }
        // `(R-Place)` — a call result built where it will live: the buffer and the local the
        // call delivered into are ONE record, and `place_result` collapses their exit pair into
        // `OpFreeRecordIn(local)` on a path that never stored it and nothing on a path whose
        // `OpMoveRecord(local, …)` did.  The buffer carries no free of its own by design.
        if self.placed.contains(&v) {
            return true;
        }
        false
    }
}

/// The `(R-Place)` buffers (`__ref_N = OpPlaceRecord(host, tp)`) whose adopting local — the
/// one a call delivered into through that buffer — is released by `OpFreeRecordIn` or consumed
/// by `OpMoveRecord` somewhere in the body.  A buffer whose local neither releases nor moves is
/// left out, so a dropped release still reads as a leak.
fn placed_buffers_released_by_their_local(
    ir: &Value,
    data: &Data,
    function: &Function,
) -> HashSet<u16> {
    let place = data.def_nr("OpPlaceRecord");
    let mut buffers: HashSet<u16> = HashSet::default();
    ir.walk(&mut |n| {
        if let Value::Set(v, rhs) = n.unspan()
            && matches!(rhs.unspan(), Value::Call(d, _) if *d == place)
        {
            buffers.insert(*v);
        }
    });
    if buffers.is_empty() {
        return buffers;
    }
    let (free_in, mv) = (data.def_nr("OpFreeRecordIn"), data.def_nr("OpMoveRecord"));
    let mut released: HashSet<u16> = HashSet::default();
    let mut adopter: Vec<(u16, u16)> = Vec::new();
    ir.walk(&mut |n| match n.unspan() {
        Value::Call(d, args) if *d == free_in || *d == mv => {
            if let Some(Value::Var(l)) = args.first().map(Value::unspan) {
                released.insert(*l);
            }
        }
        Value::Set(l, rhs) => {
            if let Value::Call(_, args) = rhs.unspan() {
                for a in args {
                    if let Value::Var(b) = a.unspan()
                        && buffers.contains(b)
                    {
                        adopter.push((*b, *l));
                    }
                }
            }
        }
        _ => {}
    });
    // …and a loop buffer (`(R-Place)`'s loop clause) claimed in a LOCAL host whose own
    // store is freed whole: the block is released with that store, which is why the
    // placement leaves the buffer's exit frees empty.
    let whole = data.def_nr("OpFreeRef");
    let mut freed_whole: HashSet<u16> = HashSet::default();
    ir.walk(&mut |n| {
        if let Value::Call(d, args) = n.unspan()
            && *d == whole
            && let Some(Value::Var(s)) = args.first().map(Value::unspan)
        {
            freed_whole.insert(*s);
        }
    });
    let mut hosted_in_freed_store: HashSet<u16> = HashSet::default();
    ir.walk(&mut |n| {
        if let Value::Set(b, rhs) = n.unspan()
            && buffers.contains(b)
            && let Value::Call(_, args) = rhs.unspan()
            && let Some(Value::Var(h)) = args.first().map(Value::unspan)
            && !function.is_argument(*h)
            && (freed_whole.contains(h)
                || function
                    .tp(*h)
                    .depend()
                    .iter()
                    .any(|s| freed_whole.contains(s)))
        {
            hosted_in_freed_store.insert(*b);
        }
    });
    adopter
        .into_iter()
        .filter(|(_, l)| released.contains(l))
        .map(|(b, _)| b)
        .chain(hosted_in_freed_store)
        .collect()
}

/// After scope analysis, assert that every Reference variable that should be
/// freed has a corresponding `OpFreeRef` somewhere in `ir`.
///
/// A variable "should be freed" when:
/// - Its type is `Reference(_, dep)` with `dep.is_empty()`
/// - It is not a function parameter (scope > 0)
/// - It is not marked `skip_free`
/// - It is not in the function's return-type dependencies
///
/// Only compiled in debug builds; the check panics rather than emitting a
/// diagnostic so that the failure is visible immediately during development.
#[cfg(debug_assertions)]
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn check_ref_leaks(
    ir: &Value,
    function: &Function,
    data: &Data,
    fn_name: &str,
    ret_type: &Type,
    var_scope: &BTreeMap<u16, u16>,
) {
    let releases = FrameReleases::of(ir, function, data, fn_name, ret_type);
    let FrameReleases {
        freed, ret_deps, ..
    } = &releases;
    for (&v, &scope) in var_scope {
        if scope == 0 {
            continue; // function parameter — caller frees
        }
        if (v as usize) >= function.count() as usize {
            continue; // variable belongs to outer scope — not our problem
        }
        if function.is_skip_free(v) {
            continue;
        }
        if releases.explains(data, function, v) {
            continue;
        }
        if let Type::Reference(_, dep) = function.tp(v) {
            // LOFT_REF_LEAK_WARN=1 downgrades the assert to a warning so a
            // debug build can still RUN a program with a known leak shape
            // (e.g. to chase a separate runtime corruption past compile).
            //
            // @FR-O-Proxy asks oracle — this CHECKS for a missing free, it never emits or
            // suppresses one, so the empty dep list is read as evidence about a program the
            // scope pass has already finished lowering.  A wrong answer here costs a
            // diagnostic, never a release.
            let warn_only = crate::env_once!(std::env::var("LOFT_REF_LEAK_WARN").is_ok());
            if warn_only && !(!dep.is_empty() || ret_deps.contains(&v) || freed.contains(&v)) {
                eprintln!(
                    "[check_ref_leaks] WARNING: Reference variable '{}' (var_nr={v}) in \
                     function '{fn_name}' has no OpFreeRef (scope {scope}) — store leak.",
                    function.name(v),
                );
            } else {
                assert!(
                    !dep.is_empty() || ret_deps.contains(&v) || freed.contains(&v),
                    "[check_ref_leaks] Reference variable '{}' (var_nr={v}) in function \
                     '{}' has no OpFreeRef — it is in scope {scope} but was never freed. \
                     This is likely a scope-registration bug: the variable was registered \
                     in an inner block scope that is not reachable from function-exit cleanup.",
                    function.name(v),
                    fn_name
                );
            }
            // warn about variables with deps that are only text-return work refs.
            // These deps are spurious (struct copies the text), but OpFreeRef is still
            // skipped, causing a store leak at runtime.
            if !dep.is_empty()
                && !ret_deps.contains(&v)
                && !freed.contains(&v)
                && !is_a_freed_backing(ir, v, function, data, &freed)
                && dep.iter().all(|d| {
                    function.name(*d).starts_with("__ref_")
                        || function.name(*d).starts_with("__rref_")
                })
            {
                eprintln!(
                    "[check_ref_leaks] Warning: Reference variable '{}' (var_nr={v}) in \
                     function '{}' has only text-work deps {:?} — likely spurious. \
                     Store will leak at runtime.",
                    function.name(v),
                    fn_name,
                    dep.iter()
                        .map(|d| function.name(*d).to_string())
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
}
