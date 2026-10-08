// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Owner WITNESSES: the locals whose assignments mix owning and borrowing, the runtime witness
//! that releases by store identity, and the reassignments that displace a store the target owns.

use super::backings::delivered_work_ref;
use super::call;
use super::capture_builds::capture_build_backings;
use super::disturbance::Disturbance;
use crate::data::{Data, Type, Value, v_set};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// Does `node` contain a `Value::Loop` anywhere inside it?
///
/// The `for` lowering wraps its loop in a block (`{#For block … loop {#For loop …} }`), so the
/// statement an enclosing block holds is a `Block`, not the `Loop` itself.
/// Does `node` contain a fn-ref call anywhere?  The cheap structural gate on
/// [`mixed_ownership_locals`]'s oracle walk — the fact it computes is read only where a
/// `CallRef` delivers a collection.
fn contains_callref(node: &Value) -> bool {
    if matches!(node.unspan(), Value::CallRef(_, _)) {
        return true;
    }
    let mut found = false;
    node.for_each_child(&mut |c| {
        if !found && contains_callref(c) {
            found = true;
        }
    });
    found
}

/// Which heap LOCALS are assigned a BORROW on one path and an owned value on another?
///
/// The single ownership fact such a binding carries cannot be right for both.  Its `deps` come
/// out EMPTY — the owned arm contributes none and the join drops the borrow's — so @FR-O-Proxy
/// answers "owned" and the displacement free releases a store the borrow arm only borrowed.
/// Measured: `if c { r = field_of(q) } else { r = fnref(7) }` in a loop freed `q`'s store on
/// the iteration that took the other arm, on both backends (loft#1333).
///
/// **A FOREIGN base is the borrow that counts, not any non-owned verdict.** `x: vector<τ> = []`
/// followed by `x = fnref(i)` also reads as non-owned on its first assignment, and there the
/// base is the `__vdb_N` that very literal minted — storage `x` is the sole user of, released
/// by that temp's own scope-exit free.  Marking such a binding withdraws the strip loft#1329
/// needs, and every cell of its guard then exhausted the store table.  What separates them is
/// whether the base is the binding's own literal backing
/// ([`crate::variables::owns_literal_backing_store`]) or storage reached through somebody
/// else — a call's return buffer carrying a view of the callee's argument, which is the c3i
/// shape this closes.
///
/// **Why MIXED and not merely viewed.** A local every path views already carries a dep and
/// declines the free on its own; it is the disagreement that produces the empty list.  Keeping
/// the condition to mixed bindings is what stops this from suppressing frees that are correct.
pub(super) fn mixed_ownership_locals(
    code: &Value,
    function: &Function,
    data: &Data,
    d_nr: u32,
) -> Vec<u16> {
    let mut viewed: HashSet<u16> = HashSet::default();
    let mut owned: HashSet<u16> = HashSet::default();
    fn walk(
        node: &Value,
        data: &Data,
        d_nr: u32,
        function: &Function,
        viewed: &mut HashSet<u16>,
        owned: &mut HashSet<u16>,
    ) {
        if let Value::Set(t, val) = node.unspan() {
            match crate::use_analysis::ownership_of(data, d_nr, val) {
                // @PLN155 phase 2 — membership of `owned` licenses a free downstream, so this
                // IS a reader where declining on `Unknown` is the conservative direction.
                // Kept joined for 2a; a candidate for 2b, with the leak it would trade for.
                // @PLN155 phase 2b — DECLINE withholds membership, and membership licenses a
                // free downstream, so this is the conservative direction and the trade is a
                // leak.  `Owned` is unaffected: only the verdict with no derivation behind it
                // stops licensing.
                //
                // Measured, and NOT landable on that measurement: it moves the emit of three
                // corpus files (1183, 1332, 1333) while every channel — value, exit and leak,
                // both backends, under `LOFT_STRICT_STORES` — reads identically.  That is
                // unverified rather than safe, and it is the shape `introspect_diff.sh`'s own
                // doc warns about: a changed emission that happens to compute the same values
                // is still a change nobody asked for.  Landing it needs the three files shown
                // individually right, not a green suite.
                crate::use_analysis::Own::Unknown if crate::keys::own_declines("owned-slot") => {}
                crate::use_analysis::Own::Owned | crate::use_analysis::Own::Unknown => {
                    owned.insert(*t);
                }
                crate::use_analysis::Own::Borrowed { base }
                | crate::use_analysis::Own::Join { base } => {
                    // A base that is the binding's OWN literal backing is not a foreign
                    // borrow: `x: vector<τ> = []` lowers to a read of the `__vdb_N` this very
                    // literal minted, and that temp's own scope-exit free releases it.  Any
                    // other base is storage reached through somebody else — a call's return
                    // buffer carrying a view of the callee's argument, or a named local.
                    if base != u16::MAX
                        && (base as usize) < function.count() as usize
                        && !crate::variables::owns_literal_backing_store(function.name(base))
                    {
                        viewed.insert(*t);
                    }
                }
            }
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, data, d_nr, function, viewed, owned));
    }
    // The fact is read at ONE site — `callref_delivers_collection`'s strip — so a body with no
    // fn-ref call cannot need it, and the oracle walk below is not free: it asks
    // `ownership_of` per assignment, and a big vector literal is thousands of them
    // (`issue854_a_vector_literal_compiles_in_linear_time` went from seconds to over a minute
    // before this gate).  The structural pre-check is what keeps the cost on the bodies that
    // can actually be wrong.
    if !contains_callref(code) {
        return Vec::new();
    }
    walk(code, data, d_nr, function, &mut viewed, &mut owned);
    let mut out: Vec<u16> = viewed.intersection(&owned).copied().collect();
    out.sort_unstable();
    // ⚠ Deliberately NOT filtered on an empty dep list.  This runs BEFORE the scan, where the
    // view arm's dep is still on the binding — the empty list is what the scan PRODUCES and
    // what this exists to prevent, so testing for it here would drop every var that matters.
    out.retain(|&v| {
        v < function.count()
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
            )
            && !function.is_argument(v)
    });
    out
}

/// Which nullable heap-record LOCALS are reassigned from a call that mints?
///
/// The pre-scan answer to *"is a runtime ownership witness worth a slot here?"*, asked once
/// per function in the shape [`displaces_return_buffer`] already uses.  A body that reaches no
/// such site pays nothing.
///
/// **Why a witness and not a predicate.** A nullable RECORD return gets no delivery buffer —
/// `-> S?` is a synthetic `__nullable<S>` carrying its own delivery, and giving it a buffer as
/// well leaks one record per call — so every call MINTS and the caller owes the release of
/// what it displaces.  A static free at the reassignment cannot be placed, because the local's
/// FIRST store is normally an inline mint into a work-ref (`c: S? = S { x: 5 }` lowers to
/// `c = { Object -> __ref_p2_1 }`): the local and that work-ref name ONE store, and freeing
/// through the local double-frees it against the work-ref's own scope-exit free.  One static
/// site cannot separate the first iteration from the rest, which is what `formal/ownership.md`
/// D-own-16 records; the flag answers it per RUN.
/// Which nullable heap-record LOCALS hold a PROJECTION VIEW on some assignment — a field
/// read (`d = q.inner`), a vector-element read (`d = vs[i]`), a tuple element — a store the
/// local only borrows and never owns (@FR-O-Owner)?
///
/// Such a local is marked never-free (@FR-O-Override).  Its single-ARGUMENT dep otherwise
/// reads as ownership at the D-own-16 `borrows_one_argument` residual (`state/codegen.rs`,
/// this file's scope-exit `borrow_witness`, `generation/dispatch.rs`), which then frees the
/// store the local DISPLACES at a reassignment — the caller's nested store, or a local's
/// field — a store the local only VIEWED.  A view owns nothing, so the proxy that licenses
/// that free is wrong and @FR-O-Override vetoes it.
///
/// A DIRECT projection — a field read (`OpGetField`), a vector-element read (`OpGetVector`
/// &c) or a tuple element — ALIASES its base (@FR-B-View / @FR-B-View-Depth): the local
/// holds a store it does not own.  A whole-value bind of a heap variable COPIES (@FR-B-Copy,
/// pE) and a CALL that returns a borrowed view is COPIED into the local by the set-lowering
/// (@FR-F-Ret, loft#1346) — both mint the local a store of its own, so neither is matched
/// here; the set is exactly the ops in [`crate::use_analysis`]'s projection set plus a tuple
/// read.  The mixed-ownership shapes that own a store are excluded by the caller: a
/// solely-owned minting call by the loft#1200 runtime flag ([`nullable_locals_that_displace`]),
/// a view+mint mix by the owner witness ([`owner_witness_locals`], loft#1336), and a
/// MATERIALISED view (its container is disturbed while it is live, so it takes its own copy,
/// @FR-B-View) by `views_to_materialise`.
pub(super) fn nullable_view_locals(code: &Value, function: &Function, data: &Data) -> Vec<u16> {
    // Cost gate: restrict the walk to bodies that actually declare a nullable heap-record
    // local (the only thing this marks).
    let has_candidate = (0..function.count()).any(|v| {
        matches!(function.tp(v), Type::Optional(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && !function.is_argument(v)
            && !function.is_captured(v)
    });
    if !has_candidate {
        return Vec::new();
    }
    let projections = &data.op_sets().projections;
    let mut out: Vec<u16> = Vec::new();
    let mut walk = |node: &Value| {
        if let Value::Set(t, val) = node.unspan()
            && !out.contains(t)
            // A DIRECT projection that ALIASES: a tuple element, or one of the projection
            // reads.  NOT a bare `Var` (copies, @FR-B-Copy) and NOT a user/native call
            // returning a borrow (copied into the local, @FR-F-Ret / loft#1346).
            // A tagged slot read through its tag (`if <present> { <projection> } else {
            // nullref }`, the bind of `x = o.opt`) is the projection its present arm is.
            && match crate::use_analysis::through_null_arm(data, val).unspan() {
                Value::TupleGet(_, _) => true,
                Value::Call(fn_nr, _) => projections.contains(fn_nr),
                _ => false,
            }
        {
            out.push(*t);
        }
    };
    code.walk(&mut |n| walk(n));
    out.retain(|&v| {
        v < function.count()
            && matches!(function.tp(v), Type::Optional(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && !function.is_argument(v)
            && !function.is_captured(v)
            && !function.is_compiler_generated(v)
            // A local whose DEP LIST is empty is not a view, whatever its defining statement
            // looks like — something detached it deliberately, and the only honest reading of
            // an owner-with-no-dep is that it owns.  `make_independent` is that something: a
            // write to a KEY field through a keyed element view materialises the local
            // (@PLN130 F4), and from there it owns the copy and must free it.
            //
            // Classifying on the defining SHAPE alone saw the projection and marked it
            // never-free, so no copy was made, the key write reached the collection, and the
            // element was left reachable by NO key — the exact defect F4 exists to prevent.
            // It stayed invisible while these locals were non-null; `@FR-Col-Lookup` giving a
            // keyed lookup its `?` (loft#1450) brought them into this classifier for the first
            // time and the shape test could not tell an owner from a view.
            && !function.tp(v).depend().is_empty()
    });
    out
}

pub(super) fn nullable_locals_that_displace(
    code: &Value,
    function: &Function,
    data: &Data,
    null_led_first: &HashSet<u16>,
) -> Vec<u16> {
    // loft#1934, `@FR-N-Road` — while a `τ?` record return takes the buffer, a nullable local's
    // bind takes the dense arm (`use_analysis::first_bind_shape`), which answers this question
    // by store identity.  A flag beside it is a second release mechanism whose writes that arm
    // never emits: two `y: S?` loops shared one stale flag and freed a buffer still in use.
    if crate::keys::nullable_ret_buffer() {
        return Vec::new();
    }
    fn walk(node: &Value, seen: &mut HashSet<u16>, out: &mut Vec<u16>, data: &Data) {
        if let Value::Set(t, val) = node.unspan() {
            // A SECOND assignment is what displaces; the first allocates.
            if !seen.insert(*t)
                && mints_a_store_the_target_does_not_hold(val, *t, *t, data)
                && !out.contains(t)
            {
                out.push(*t);
            }
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, seen, out, data));
    }
    let mut out = Vec::new();
    let mut seen = HashSet::default();
    walk(code, &mut seen, &mut out, data);
    // @FR-O-Proxy asks free — the locals this returns are the ones whose DISPLACED store is
    // released, so the proxy's answer is what licenses that free.  A local whose earlier binds
    // are nulls in front of the minting one on every pass displaces nothing: that bind is a
    // first bind (`@FR-O-Move`) and takes the first bind's pairing (loft#1643).
    out.retain(|&v| {
        !null_led_first.contains(&v)
            && v < function.count()
            && matches!(function.tp(v), Type::Optional(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            // @FR-O-Proxy asks free — the pair, asked once ([`Function::proxy_says_owned`]),
            // plus the parameter carve-out this site shares with the scope-exit sweep.
            && function.proxy_says_owned(v)
            && !function.is_argument(v)
    });
    out
}

/// Which heap-record LOCALS have assignments that MIX ownership — at least one that hands
/// the local a store of its own and at least one that hands it a view?
///
/// The pre-scan answer to *"is an OWNER WITNESS worth a slot here?"* (loft#1336,
/// `@FR-O-Witness`), asked once per function like [`nullable_locals_that_displace`].
///
/// **Why a witness and not the dep list.** A binding carries ONE dep list, flow-insensitively,
/// and it records whichever assignment parsed LAST: `cur: Node? = a; cur = cur.next` leaves
/// `cur` reading as a borrow for the whole frame, so the store the copy minted is released by
/// nobody — and the inverse order leaves it reading as an owner while it holds a view, so the
/// view's record is freed as if it were the local's.  Neither static answer is right for both
/// assignments; the witness answers per RUN, by store identity.
///
/// A local with only owning assignments keeps the static free placement (the proxy is right
/// for it), and one with only views has nothing to release.  Excluded on purpose: a
/// parameter (its entry stash is the witness, `Function::rebind_orig`), a captured local (a
/// closure reads the capture-time `DbRef`, @FR-L-CapHeap — the record takes over the free), a
/// loop variable (bound by `Iter`, not by a `Set`), and the compiler's own temporaries.
pub(super) fn owner_witness_locals(
    code: &Value,
    function: &Function,
    data: &Data,
    d_nr: u32,
    materialised_views: &HashMap<u16, Disturbance>,
) -> Vec<u16> {
    let mut defs: Option<crate::use_analysis::Defs> = None;
    let mut minted: HashSet<u16> = HashSet::default();
    let mut viewed: HashSet<u16> = HashSet::default();
    fn walk(
        node: &Value,
        function: &Function,
        data: &Data,
        d_nr: u32,
        defs: &mut Option<crate::use_analysis::Defs>,
        minted: &mut HashSet<u16>,
        viewed: &mut HashSet<u16>,
    ) {
        if let Value::Set(t, val) = node.unspan()
            && (*t as usize) < function.count() as usize
            && !function.name(*t).starts_with("__")
            && !function.is_argument(*t)
            && !function.is_captured(*t)
            && !function.was_loop_var(*t)
            && !matches!(function.tp(*t), Type::RefVar(_))
            && function.tp(*t).base().heap_def_nr().is_some()
        {
            match witness_set_kind(val, *t, *t, function, data, d_nr, &mut |v| {
                let defs =
                    defs.get_or_insert_with(|| crate::use_analysis::function_defs(data, d_nr));
                crate::use_analysis::ownership_of_with(data, d_nr, v, defs)
            }) {
                WitnessSet::Mint | WitnessSet::MintReading => {
                    minted.insert(*t);
                }
                WitnessSet::Other => {
                    // A view of another variable's storage.  A null or a store nobody names
                    // is not a VIEW and does not make the ownership mixed on its own.
                    if is_view_of_storage(val, function, data) {
                        viewed.insert(*t);
                    }
                }
            }
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, function, data, d_nr, defs, minted, viewed));
    }
    walk(
        code,
        function,
        data,
        d_nr,
        &mut defs,
        &mut minted,
        &mut viewed,
    );
    // @FR-O-Latest / @FR-O-Witness — a CAPTURED local reassigned after its build has mixed
    // ownership too, and between the same two parties the witness exists for: the closure
    // record owns the store the capture named AT THE BUILD, the frame owns every store the
    // local is given afterwards.  No static reading is right for both — `(O-Latest)` says a
    // type-level `deps` list can express neither which assignment nor the loop depth — so the
    // release has to be by STORE IDENTITY, which is exactly what this witness does.  Without
    // one, `owns_displaced_store`'s per-BINDING `!is_captured` veto is asked at two sites that
    // are statically identical and is right at only one of them (loft#1388).
    let builds = capture_build_backings(data, function, code);
    for &v in &builds.reassigned_after_build {
        // Both spellings of "the record took this local's store": a STRUCT capture names the
        // local outright, a COLLECTION capture names a view whose backing local is this one.
        // The backing needs the witness for the same reason and, once the record's own
        // release is emitted (`fn_ref_with_closure`'s pre-build free), for a sharper one: a
        // local left without one is still owned by the FRAME, so the two would free the same
        // store between them — measured as the vector loop answering `1,2` where `4,5` is
        // right, on `--native` alone.
        if (function.is_captured(v) || builds.backing.values().any(|b| *b == v))
            && !function.is_argument(v)
            && !function.name(v).starts_with("__")
            && !function.was_loop_var(v)
            // RECORD kinds only, and that boundary is measured rather than assumed.  A vector
            // witness was built — a `Vector`-typed `__own_` handle, admitted here and released
            // by the same identity guard — and it answers WRONG: the vector-capture loop read
            // `1,2` where `4,5` is right, on `--native`, because the witness releases a store
            // the record still holds.  A leak is the better trade, so the vector-capture loop
            // keeps one store (values right on both backends) until the release the record
            // owes is decided per SLOT rather than per local.
            && function.tp(v).base().heap_def_nr().is_some()
            && !materialised_views.contains_key(&v)
        {
            minted.insert(v);
            viewed.insert(v);
        }
    }
    let mut out: Vec<u16> = minted
        .intersection(&viewed)
        .copied()
        .filter(|v| !materialised_views.contains_key(v))
        .collect();
    out.sort_unstable();
    out
}

/// Does this value bind a VIEW of storage some other binding owns — a projection, a
/// call answering a borrow, a join?  The `viewed` half of [`owner_witness_locals`].
///
/// A CONSTRUCTION into its own work-ref is NOT one, and answering `true` for it fabricated the
/// VIEW half of a mix that does not exist (loft#1517, `formal/heap-history.md` D-heap-6).  The work-ref
/// is the compiler's own temp for this very literal, so no other BINDING owns what it names, and
/// the local adopts the store outright ([`Scopes::scan_set`]'s hand-off disarm) — which is also
/// `@FR-O-Owner`'s single owner, and it only holds once the local is NOT witnessed.
///
/// [`witness_set_kind`] answers `Other` for such a value on purpose: while the work-ref still
/// names the store it is not SOLELY the local's, which is the right answer to *"may the witness
/// POINT here?"*.  Reading that one `Other` as a view is what made `x: SE = A { k: 3 }; x = a`
/// — two OWNING assignments by `(B-Copy)` — carry a witness, after which
/// `Scopes::displaced_drop` declined on `@FR-O-Override` and neither record's `OpDrop` ran.
/// Two sites, two different questions, one answer: this is the `viewed` half only.
fn is_view_of_storage(value: &Value, function: &Function, data: &Data) -> bool {
    if delivered_work_ref(value, function, data).is_some() {
        return false;
    }
    match value.unspan() {
        Value::Null => false,
        Value::Call(nr, args) if args.is_empty() && data.def(*nr).name() == "OpNullRefSentinel" => {
            false
        }
        // A whole-value copy of a heap record never views (@FR-B-Copy).
        Value::Var(_) => false,
        Value::Call(_, _)
        | Value::CallRef(_, _)
        | Value::Block(_)
        | Value::Insert(_)
        | Value::TupleGet(_, _) => true,
        // A tagged slot read through its tag (`Parser::emit_nullable_slot_read`, the bind
        // of `x = o.opt` under `@FR-L-Null-Which`) answers as its present arm does.
        Value::If(_, _, _) => {
            let seen = crate::use_analysis::through_null_arm(data, value);
            !matches!(seen.unspan(), Value::If(_, _, _)) && is_view_of_storage(seen, function, data)
        }
        _ => false,
    }
}

/// What a `Set` hands a witnessed local (see [`owner_witness_locals`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum WitnessSet {
    /// The local MINTS a store of its own and the value does not read the local.
    Mint,
    /// The local MINTS a store of its own from a value that READS it, so the store it
    /// displaces must stay live until the value is computed (@FR-O-Detach).
    MintReading,
    /// A view, a join, a null, or a store somebody else owns: the local stops owning.
    Other,
}

/// Classify the value assigned to witnessed local `v` (`ov` is its pre-scan id).
///
/// MINTS means *both emitters give the local a store of its own here*: a whole-value copy of
/// another heap variable (@FR-B-Copy, `gen_set_first_ref_var_copy` / native's record-copy
/// arm); a loft-defined callee whose return does NOT adopt a fresh store, which both emitters
/// deep-copy into the local (`state/codegen.rs`'s call-copy arm, `generation/dispatch.rs`'s
/// `OpCopyRecord` arm); a value the oracle calls `Owned` — a minting call, an inline mint —
/// unless that mint lands in a work-ref that frees it itself (the `c = { Object → __ref_N }`
/// literal, whose store is the work-ref's and not solely the local's).
///
/// A PROJECTION is never a mint here, whatever its deps: @FR-B-View's materialise clause
/// (a view live across a reshape of its container) is decided from the binding's FINAL dep
/// list at codegen, which one `Set` cannot see mid-scan, so a witnessed local is kept out of
/// that clause altogether — [`owner_witness_locals`] excludes every binding
/// `collect_views_to_materialise` names, and both emitters decline the materialise arm for a
/// witnessed local.  The two mechanisms never meet on one binding.
///
/// The fallback is `Other`, and it is the SAFE direction: a value this does not recognise as
/// a mint is treated as a view, so the witness is released where the local stops naming it
/// and never pointed at a store the local does not own — a retained store is recoverable, a
/// premature free is not (@FR-O-Oracle's rule for an unnameable base).
pub(super) fn witness_set_kind(
    value: &Value,
    v: u16,
    ov: u16,
    function: &Function,
    data: &Data,
    d_nr: u32,
    own: &mut impl FnMut(&Value) -> crate::use_analysis::Own,
) -> WitnessSet {
    let reads = value.reads_var(v) || value.reads_var(ov);
    let mints = match value.unspan() {
        Value::Var(src) => {
            *src != v
                && *src != ov
                && (*src as usize) < function.count() as usize
                && function.tp(*src).base().heap_def_nr().is_some()
        }
        Value::Call(nr, args) if args.is_empty() && data.def(*nr).name() == "OpNullRefSentinel" => {
            false
        }
        Value::Call(_, _) | Value::CallRef(_, _) => {
            let copied_by_both =
                crate::use_analysis::callee_of(data, d_nr, value).is_some_and(|fn_nr| {
                    data.def(fn_nr).is_loft_defined()
                        && !data.def(fn_nr).return_adopts_fresh_store()
                });
            // @PLN155 phase 2 — `Unknown` keeps `Owned`'s answer at all three `mints` tests
            // below, and it must be SPELLED: a bare `Own::Owned` silently answers "does not
            // mint" for a value the oracle could not read, which is a different decision from
            // the one this site has been making.
            copied_by_both
                || matches!(
                    own(value),
                    crate::use_analysis::Own::Owned | crate::use_analysis::Own::Unknown
                )
        }
        Value::Block(b) => {
            let into_work_ref = b.operators.last().is_some_and(|last| {
                matches!(last.unspan(), Value::Var(r)
                    if (*r as usize) < function.count() as usize
                        && (function.name(*r).starts_with("__ref_")
                            || function.name(*r).starts_with("__rref_")))
            });
            !into_work_ref
                && matches!(
                    own(value),
                    crate::use_analysis::Own::Owned | crate::use_analysis::Own::Unknown
                )
        }
        Value::Insert(_) => matches!(
            own(value),
            crate::use_analysis::Own::Owned | crate::use_analysis::Own::Unknown
        ),
        _ => false,
    };
    if !mints {
        WitnessSet::Other
    } else if reads {
        WitnessSet::MintReading
    } else {
        WitnessSet::Mint
    }
}

/// Release the store an owner witness names and reset the witness to the sentinel — as ONE
/// unit, because `OpFreeRef` of a variable does not reset its slot on the interpreter, and a
/// witness left naming a freed store would release whatever the allocator hands that slot
/// next.
pub(super) fn release_witness(w: u16, hook: Option<Value>, data: &Data) -> Value {
    // loft#1510 / `(H-Drop)` — the witness releases a store the FRAME minted, so the type's
    // cascade runs first: the bare free released the resource without its hook.  Guarded on
    // the witness holding a record (`ConvBoolFromRef` reads the sentinel's `rec == 0` as
    // false), so a witness that took no store hooks nothing.  Exactly one mechanism claims a
    // witnessed store — the witnessed local never drops (view-typed) and the call's hidden
    // buffer keeps a bare, hook-less free — so the hook here cannot double.  The hook is the
    // caller's to pass (`Scopes::witness_hook`): a store the frame minted can hold a record
    // whose resource it does not own, and the free below runs either way.
    let mut ops = Vec::with_capacity(3);
    if let Some(hook) = hook {
        ops.push(hook);
    }
    ops.push(call("OpFreeRef", w, data));
    ops.push(v_set(
        w,
        Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
    ));
    Value::Insert(ops)
}

/// Point owner witness `w` at the store local `v` now holds — an ALIAS, where a plain
/// `Set(w, Var(v))` would copy the record (@FR-B-Copy).
pub(super) fn witness_points_at(w: u16, v: u16, data: &Data) -> Value {
    v_set(
        w,
        Value::Call(data.def_nr("OpRefAlias"), vec![Value::Var(v)]),
    )
}

/// Does this call hand back a store the target does NOT already hold?
///
/// The complement of [`displaces_owned_through_fresh_callee`]'s NRVO shape.  There the target
/// sits at the callee's return-buffer attribute, so the callee fills the store the target
/// already owns and nothing is displaced.  Here the callee MINTS: it has no return buffer at
/// all — which is every nullable RECORD return — or it was handed someone else's.
///
/// The target must not be READ anywhere in the call: the free is emitted before the
/// assignment, so a call that still reads the old value would be handed a freed store.
pub(super) fn mints_a_store_the_target_does_not_hold(
    value: &Value,
    v: u16,
    ov: u16,
    data: &Data,
) -> bool {
    let Value::Call(fn_nr, args) = value.unspan() else {
        return false;
    };
    let def = data.def(*fn_nr);
    if !def.is_loft_defined() || !def.return_adopts_fresh_store() {
        return false;
    }
    let names_v = |x: &Value| matches!(x.unspan(), Value::Var(w) if *w == v || *w == ov);
    if def
        .hidden_return_buffer_attr()
        .is_some_and(|i| args.get(i).is_some_and(names_v))
    {
        return false;
    }
    args.iter().all(|a| !a.reads_var(v) && !a.reads_var(ov))
}

/// Does this body ever displace a store held by the return-buffer parameter `v`?
///
/// The pre-scan answer to *"is a runtime ownership witness worth a slot here?"* — asked once
/// per function so the witness exists before the first assignment that has to maintain it, in
/// the same shape `displaced_owned` and `collect_views_to_materialise` already use.  A body
/// that never reaches such a site pays nothing.
pub(super) fn displaces_return_buffer(code: &Value, v: u16, data: &Data) -> bool {
    fn walk(node: &Value, v: u16, data: &Data) -> bool {
        if let Value::Set(t, val) = node.unspan()
            && *t == v
            && displaces_owned_through_fresh_callee(val, v, v, data)
        {
            return true;
        }
        let mut found = false;
        node.unspan().for_each_child(&mut |c| {
            if !found {
                found = walk(c, v, data);
            }
        });
        found
    }
    walk(code, v, data)
}

pub(super) fn displaces_owned_through_fresh_callee(
    value: &Value,
    v: u16,
    ov: u16,
    data: &Data,
) -> bool {
    let Value::Call(fn_nr, args) = value.unspan() else {
        return false;
    };
    let def = data.def(*fn_nr);
    if !def.return_adopts_fresh_store() {
        return false;
    }
    // WHICH attribute is the return buffer is `Definition::hidden_return_buffer_attr`'s
    // question — the same answer the substitution that put `v` there read.
    let Some(buf_idx) = def.hidden_return_buffer_attr() else {
        return false;
    };
    let names_v = |x: &Value| matches!(x.unspan(), Value::Var(w) if *w == v || *w == ov);
    if !args.get(buf_idx).is_some_and(names_v) {
        return false;
    }
    // Pre-scan IR can still name the ORIGINAL slot, so both spellings count as a read.
    args.iter()
        .enumerate()
        .all(|(i, a)| i == buf_idx || (!a.reads_var(v) && !a.reads_var(ov)))
}
