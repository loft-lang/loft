// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! A scope EXIT: `free_vars` builds the releases a block, a loop pass or a return runs on the way
//! out, around the value leaving with it.

use super::capture_adoption::{
    capture_store_adopters, free_record_in_omitting_arms, link_written_closure_records,
    record_store_leaves_frame,
};
use super::handoff::promoted_ret_buffer;
use super::insert_free::{
    any_text_return_buffer, expr_ends_in_return, free_copied_text_sources, text_return_buffer_for,
};
use super::returns::{
    collect_return_sources, is_heap_return_type, is_value_return_type, move_join_hooks_into_arms,
    return_copies_view_holders, return_copies_whole_local, return_has_non_source_arm,
    return_has_null_arm, return_moved_holders, returned_var_null_unified,
};
use super::{Scopes, call};
use crate::data::{Data, Deps, Type, Value, v_if, v_set};
use crate::fxhash::FxHashSet as HashSet;
use crate::variables::Function;

/// The capture slots a closure record's build writes, as byte offsets.
///
/// Read off the builds themselves (`OpSetDbRef(record, offset, capture)`) rather than from the
/// record's attribute table, so the offsets are exactly the ones the emitted code used.
fn record_capture_slots(code: &Value, record: u16, set_dbref: u32) -> Vec<i32> {
    let mut out = Vec::new();
    code.walk(&mut |n| {
        if let Value::Call(d, args) = n.unspan()
            && *d == set_dbref
            && let Some(Value::Var(r)) = args.first().map(Value::unspan)
            && *r == record
            && let Some(Value::Int(off)) = args.get(1).map(Value::unspan)
            && !out.contains(off)
        {
            out.push(*off);
        }
    });
    out
}

impl Scopes<'_> {
    /// Is `v` the hidden NRVO return buffer promoted onto an argument slot, rather
    /// than a user parameter?
    ///
    /// A true parameter belongs to the caller and the callee may free none of it; the
    /// promoted buffer is a LOCAL that `classify_ret_promotion` renamed onto the hidden
    /// `__retbuf` attribute and `become_argument`ed.  It holds the store the CALLER handed
    /// or, when the caller handed the null sentinel, one this function minted — only the
    /// second is this frame's, and the entry witness (`Function::entry_witness`) tells the
    /// two apart per run wherever a free of it is emitted.  The un-renamed `__retbuf`
    /// placeholder is left out — no local was promoted onto it (loft#688).
    pub(super) fn is_promoted_ret_buffer(&self, function: &Function, data: &Data, v: u16) -> bool {
        promoted_ret_buffer(data, self.d_nr, function, v)
    }

    /// Enforces @FR-O-Derived: free placement is DERIVED, not decided — a local is freed
    /// iff it owns its store and does not transfer it out, once, at scope exit.  A
    /// per-site heuristic anywhere else in codegen is the bug that rule names.
    /// Does `d_nr`'s published return BORROW the closure it carries?
    ///
    /// The one question that decides whether a store this function mints and hands back will
    /// have an owner at the call site.  `published_ret_type` keeps the `__closure` index only
    /// where the caller is meant to read the result as a borrow, so its presence here IS the
    /// caller's reading — asked off the same attribute list, so the two cannot drift.
    fn return_borrows_closure(data: &Data, d_nr: u32) -> bool {
        if d_nr == u32::MAX || d_nr >= data.definitions() {
            return false;
        }
        let def = data.def(d_nr);
        let Some(idx) = def.attributes().iter().position(|a| a.name == "__closure") else {
            return false;
        };
        u16::try_from(idx).is_ok_and(|i| def.returned.depend().contains(&i))
    }

    /// May this function free the store the source `v` names — or does it belong to
    /// someone else?
    ///
    /// Enforces @FR-O-Proxy. The empty dep list is the cheap PROXY for "this binding owns
    /// its store" and the rule says it is unsound alone, so the two obligations it names
    /// are discharged together and in one place: the `O-Override` veto (`is_skip_free`,
    /// whose contract is "no ownership-derived free, in any spelling, for this binding"), and
    /// the carve-out that a user PARAMETER belongs to the caller while the promoted NRVO
    /// buffer is the one argument that is really a local this function minted.
    ///
    /// One home because it was written three times inside [`free_vars`](Self::free_vars) —
    /// once for a record source, once for a keyed one, once for the arm that disagrees
    /// about ownership — and each copy had to be extended on its own when a new shape
    /// arrived (loft#688, then loft#1022, then loft#1078 restating the same carve-out a
    /// third time). The keyed copy never gained the promoted-buffer half at all.
    ///
    /// The override consult is new here and is a GUARD rather than a fix: measured across
    /// `tests/scripts` and `tests/docs`, no `skip_free` binding currently reaches any of
    /// these sites, so today the obligation holds by accident. It now holds by
    /// construction.
    /// @FR-O-Proxy asks free — this IS the free question, asked once for every caller.
    fn owns_freeable_store(
        &self,
        function: &crate::variables::Function,
        data: &Data,
        v: u16,
    ) -> bool {
        // @FR-O-Proxy asks free — this IS the scope-exit sweep's licence.
        //
        // The proxy and its veto are ONE question and are asked as one
        // ([`crate::variables::Function::proxy_says_owned`], @PLN155 phase 1); what this
        // adds is the third obligation, which is this site's alone — the sweep frees a
        // FRAME's bindings, and a user parameter belongs to the caller.
        function.proxy_says_owned(v)
            && (!function.is_argument(v) || self.is_promoted_ret_buffer(function, data, v))
    }

    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn free_vars(
        &mut self,
        is_return: bool,
        expr: &Value,
        function: &mut Function,
        data: &Data,
        tp: &Type,
        to_scope: u16,
    ) -> Vec<Value> {
        // loft#1476 — a closure record the return delivers on SOME path is exempt from the
        // frame's free on ALL of them, because `record_leaves_frame` is one static answer to a
        // per-run question.  The run that does not deliver it then leaks the record and the
        // capture its cascade would have taken.  Put the release INSIDE the arms that do not
        // hand it out: the arm IS the path, so no runtime witness is needed, and on that path
        // nothing escaped holding the record.
        //
        // Restricted to a record that ALONE adopts its store.  Where several adopt one store the
        // record left behind may be the static owner while the delivered one borrows, and
        // freeing it would cascade into a capture the escaping record still holds — that half
        // needs per-run OWNERSHIP, not just a per-run free, and stays open on loft#1476.
        let rewritten;
        let expr = {
            let mut copy = expr.clone();
            let mut touched = false;
            if is_return && self.d_nr != u32::MAX {
                let (adopters, _) =
                    capture_store_adopters(data, function, &self.capture_build_backing);
                let set_dbref = data.def_nr("OpSetDbRef");
                let null_ref = data.def_nr("OpNullRefSentinel");
                let body = data.def(self.d_nr).code();
                // A record handed out through a `&fn(…)` LINK never appears in the tail, so the
                // tail cannot witness that it was delivered.  `arm_value_delivers_record` reads
                // the RETURN route only and would answer "this arm omits it" for every arm —
                // and the release would then destroy what the link just gave the caller.
                // `record_leaves_frame` unions both routes for exactly this reason; here the
                // link half has to be subtracted rather than asked, because no arm can speak
                // for it.  Measured: `give_both_ways` (a frame delivering by BOTH routes at
                // once) read 0xBEEF under `LOFT_POISON=1`.
                let link_delivered = link_written_closure_records(data, function, self.d_nr);
                // Every closure-record LOCAL this frame owns, not only the ones that entered
                // an adopter group: a record capturing a PARAMETER adopts nothing (the store is
                // the caller's) and so appears in no group, yet it is delivered by the same
                // branch and owes the same per-arm release.  Gating on the groups left those
                // records with no free anywhere, which `check_ref_leaks` reports as a leak.
                for v in 0..function.next_var() {
                    if function.is_argument(v) {
                        continue;
                    }
                    let Type::Reference(rec_def, _) = function.tp(v) else {
                        continue;
                    };
                    if !data.def(*rec_def).name.starts_with("__closure_")
                        || !record_store_leaves_frame(data, function, self.d_nr, v)
                        || link_delivered.contains(&v)
                    {
                        continue;
                    }
                    // Nulling is owed only where another record can hold the SAME store: there
                    // the cascade would reach a capture the delivered record still needs.  A
                    // sole adopter — and a record that adopts nothing at all — wants the plain
                    // release, whose cascade is either correct or empty.
                    let shared = adopters
                        .values()
                        .any(|g| g.len() > 1 && g.iter().any(|(l, _, _)| *l == v));
                    let mut release: Vec<Value> = Vec::new();
                    if shared {
                        for off in record_capture_slots(body, v, set_dbref) {
                            release.push(Value::Call(
                                set_dbref,
                                vec![
                                    Value::Var(v),
                                    Value::Int(off),
                                    Value::Call(null_ref, Vec::new()),
                                ],
                            ));
                        }
                    }
                    release.push(call("OpFreeRef", v, data));
                    free_record_in_omitting_arms(&mut copy, v, function, tp, &release);
                    touched = true;
                }
            }
            if touched {
                rewritten = copy;
                &rewritten
            } else {
                expr
            }
        };
        // loft#1515 shape 2 — a join-return's sources are released once for every path alike,
        // and that single sweep cannot be right for a join: the arm that hands `src.h` out
        // owes `src`'s OTHER members, the arm that does not owes the whole record.  Move the
        // HOOKS into the arms, where the arm IS the path, and leave the frees where they are.
        let arm_rewritten;
        let mut arm_dropped: HashSet<u16> = HashSet::default();
        let expr = if is_return {
            let mut copy = expr.clone();
            arm_dropped = move_join_hooks_into_arms(&mut copy, function, data);
            if arm_dropped.is_empty() {
                expr
            } else {
                arm_rewritten = copy;
                &arm_rewritten
            }
        } else {
            expr
        };
        // D-heap-7, `(H-Move)` — a return that COPIES a whole local of this frame onto the return
        // buffer (`return b` after an earlier `return a` took the buffer) moves that local's
        // release to the copy, exactly as a copied member's does (`return_copy_out`).  Its own
        // scope-end hook released it a second time, once here and once at the caller.
        if is_return
            && let Some(v) = return_copies_whole_local(expr, function, data, &self.view_backing)
        {
            arm_dropped.insert(v);
        }
        // loft#1623, `(H-Move)` — the same move one indirection down.  Where the returned value
        // is a joined binding that BORROWS its arms' temps, it is those temps the return hands
        // out, so their hook belongs to the caller and not to this exit.
        if is_return {
            for w in return_copies_view_holders(expr, function, data, &self.join_holders) {
                arm_dropped.insert(w);
            }
        }
        let moved_out = if is_return {
            return_moved_holders(
                expr,
                function,
                data,
                &self.owner_witness,
                &self.witness_aliases,
            )
        } else {
            None
        };
        let ret_var = returned_var_null_unified(expr, data.def_nr("OpNullRefSentinel"));
        // @PLN85 cluster II / A.1 part i (OWNERSHIP_MODEL row 100, invariant #5
        // "per binding, per path, complete") — the return-source SET, not the
        // single `returned_var`, drives free-suppression.  `returned_var`
        // collapses an arms-differ `match`/`if` to `u16::MAX`, which would free
        // every arm's transferred buffer at scope exit (the freed-return bug,
        // #405 / probe 05).  `collect_return_sources` is the union of every arm's
        // terminal var — the values this return transfers to the caller.
        //
        // The suppression is PATH-LOCAL: it is computed per `free_vars`
        // invocation and consumed only by THIS scope-exit's `get_free_vars`, so a
        // variable transferred on this return path is suppressed here while the
        // SAME variable, dead on a sibling path (an early `return e` vs a tail
        // `return [..]`, or the `null` arm of a nullable return), is still freed
        // by its own path's sweep.  A single global `skip_free` bit cannot encode
        // that path-dependence — marking the source do-not-free everywhere
        // over-suppresses and LEAKS the dead-path allocation (repro_p365's
        // `via_local`/`nested`, 25-nullable's `maybe_row`).  So the set is passed
        // down, not stamped onto the variable.
        let mut null_arm_record_sources: Vec<u16> = Vec::new();
        let return_sources: HashSet<u16> = if is_return {
            let mut sources = Vec::new();
            collect_return_sources(expr, data, &mut sources);
            // A nullable return (`if b { Struct{} } else { null }`) leaves the
            // present arm's work-ref placeholder orphaned on the null path.  When
            // a null arm is reachable, do NOT SET-suppress a Reference/Enum work-
            // ref source — hand it to the standard work-ref free path so the
            // orphan is freed.  See `return_has_null_arm`.
            let null_sentinel_nr = data.def_nr("OpNullRefSentinel");
            if return_has_null_arm(expr, null_sentinel_nr) {
                // @PLN85 P4-records: a record source with a reachable NULL arm
                // is a runtime JOIN — transferred to the caller on the present
                // path, an orphan (its preamble null-init ALLOCATES on interp)
                // on the null path.  The old design freed it unconditionally
                // (no orphan, but the present path returned a FREED store off
                // the eval stack — the poison-visible UAF).  Keep it
                // SUPPRESSED here and record it; the return leg below hoists
                // the value to `__ret_N` and emits
                // `OpFreeRefIfDistinct(src, __ret_N)` — the runtime decides.
                //
                // Not a PARAMETER: its store is the caller's (calls.md F-ParamHeap), so
                // the null arm of `if c { s } else { null }` has nothing of this frame's
                // to release for it — paired with the return it freed the caller's
                // record on every `null` answer, both backends.  A parameter REBOUND in
                // this body may hold a store of its own; that one is released by
                // identity against its entry stash at scope exit (`rebind_orig`).
                // ...and not a VIEW of one either, for the same reason and by the same rule.
                // `match v { [a, ..] => a, _ => null }` over a `vector<t>` PARAMETER binds
                // `a` to an element of the CALLER's store: the present arm hands that
                // element straight back and the null arm never assigns `a` at all, so the
                // conditional free is inert on both paths.  Inert but not harmless — `a`
                // lives in the match block while this free is emitted at the RETURN, so
                // native scoped the Rust `let` to the block and refused the program it had
                // just generated (E0425, loft#1415).  The interpreter never saw it: frame
                // slots have no block scope.
                //
                // `borrows_one_argument` is the exception the rule needs.  A nullable local
                // BOUND from a parameter and later reassigned from a minting call owns a
                // store on some paths and borrows on others (@FR-O-Latest, the D-own-16
                // shape), and this free is its ONLY one — `get_free_vars` skips every return
                // source.  So that one stays in, and the runtime comparison decides.
                // The PROMOTED return buffer is the one argument that is a local this frame
                // mints into (loft#1078): a `-> S?` local renamed onto it (loft#1934) keeps its
                // conditional free, and the entry witness below stops it releasing the store the
                // caller handed in.
                let d_nr = self.d_nr;
                let store_is_the_callers = |function: &Function, v: u16| {
                    ((function.is_argument(v) && !promoted_ret_buffer(data, d_nr, function, v))
                        || function
                            .tp(v)
                            .depend()
                            .iter()
                            .any(|&d| function.is_argument(d)))
                        && !function.borrows_one_argument(v)
                };
                // Through `base()`: `@FR-L-Null` gives `t?` the same storage as `t`, so it is
                // the same binding and owns its store the same way.  Asked BARE, the one
                // shape this set exists for fell past it — a local that may be ABSENT is
                // exactly what a null-arm return is about, and `d: S?` is
                // `Optional(Reference)`, not `Reference`.  It got no conditional free, and
                // nobody else frees a return source (`get_free_vars` skips them all), so
                // every store the minting path handed up was owned by nobody: one record per
                // call, unbounded (loft#1422).  The dense spelling beside it was always
                // freed, which is what hid it.
                // The free lands where the RETURN is, so it may only name a variable that is
                // live there.  A match-arm binding lives in the arm's block while the return
                // sits outside it, and the interpreter tolerated the mismatch — frame slots
                // have no block scope — while native scopes the Rust `let` to the block and
                // refused the program (E0425, loft#1415).  `variables(to_scope)` is the
                // existing one home for *which variables are live at this scope*, so ask it
                // rather than restate the scope walk.
                let live_here = self.variables(to_scope);
                for &v in &sources {
                    if matches!(
                        function.tp(v).base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    ) && (live_here.contains(&v)
                        // An argument is live in every scope, and `variables` lists none; the
                        // promoted buffer is the argument this asks about (loft#1934).
                        || promoted_ret_buffer(data, self.d_nr, function, v))
                        && !store_is_the_callers(function, v)
                    {
                        null_arm_record_sources.push(v);
                    }
                }
                // loft#936 — a COLLECTION source is the same runtime join, one
                // level down.  `_vec_N` is a VIEW into a backing record
                // (`vector<T>["__vdb_N"]`), so it is the BACKING var that owns
                // the store and that `get_free_vars` suppresses (via
                // `backs_return_source`).  That suppression is only correct on
                // the arm that actually delivers the store: on the null arm the
                // caller gets the sentinel and the entry-allocated backing
                // record is owned by nobody — one orphan per CALL, so a loop
                // calling such a function leaked until the 65,535-slot store
                // table was exhausted.  `OpFreeRefIfDistinct` compares
                // `store_nr`, and the view shares the backing record's store, so
                // the delivering arm still transfers it untouched.
                for &v in &sources {
                    if !crate::parser::vectors::is_collection(function.tp(v)) {
                        continue;
                    }
                    for d in function.tp(v).depend() {
                        if d != v
                            && !function.is_argument(d)
                            && !null_arm_record_sources.contains(&d)
                        {
                            null_arm_record_sources.push(d);
                        }
                    }
                }
            }
            // @PLN85 F2 (the fuzzer's catch: `t[i] ?? N{..}`) — a
            // MULTI-source return mixing an OWNED record work-ref arm with
            // view arms is a runtime JOIN even WITHOUT a null arm: the owned
            // arm's store transfers only when ITS branch ran, else its
            // (interp-allocating) preamble store orphans.  The CALL-default
            // twin never hits this — a Call arm contributes no source var, so
            // the work-ref keeps its plain free.  Route the owned sources
            // through the same hoist + `OpFreeRefIfDistinct` leg the null-arm
            // join uses; view sources (non-empty deps) stay suppressed as
            // borrows.
            //
            // loft#1078 — the promoted NRVO buffer is one of those owned sources,
            // and `is_argument` alone excluded it.  `fn pick(c) -> S { w = S{a:7};
            // if c { S{a:9} } else { w } }` renames `w` onto the hidden buffer, so
            // both arms name an owned candidate and exactly one wins; on the arm
            // that does NOT deliver `w`, the store `w` minted is returned by nobody
            // and freed by nobody — one orphan per call, which is the same u16
            // store-table exhaustion loft#688 and loft#1022 each closed for their
            // own shape.  The carve-out is the one those two already established:
            // a user PARAMETER belongs to the caller and the callee frees none of
            // it, while the promoted buffer is the one argument that is really a
            // local this function minted (`is_promoted_ret_buffer` — hidden attr,
            // renamed off `__retbuf`).  It reaches here rather than loft#688's leg
            // because that leg excludes anything in `sources`, and the owning arm
            // puts it there — the identical reason loft#1022 restated the carve-out
            // in its own gate.
            if sources.len() > 1 {
                for &v in &sources {
                    if matches!(
                        function.tp(v),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    ) && self.owns_freeable_store(function, data, v)
                        && !null_arm_record_sources.contains(&v)
                    {
                        null_arm_record_sources.push(v);
                    }
                }
            }
            // loft#688 — the NRVO return buffer is an ARGUMENT, and `variables()`
            // stops at scope 0 ("never return function arguments"): a true
            // parameter belongs to the caller, so the callee must not free it.
            // An NRVO buffer breaks that premise.  It is a promoted LOCAL — the
            // rename in `classify_ret_promotion` gives the hidden `__retbuf` attr
            // the local's name and `become_argument`s it — and when the caller
            // handed the null sentinel THIS function mints its store with
            // `OpDatabase`.  When a sibling return path delivers a different store,
            // the minted one is returned by nobody and freed by nobody: one orphan
            // per call, which exhausted the 65,535-entry store table after 65,535
            // calls and was invisible below that.  When the caller handed a LIVE
            // store (a pooled record buffer, `reuse_record_buffers`) that store is
            // the caller's, and the free is guarded by the entry witness.
            //
            // So treat it like any other candidate source that may or may not be
            // the returned store: route it through the same hoist +
            // `OpFreeRefIfDistinct` leg the null-arm join uses.  On the path that
            // DOES return it, `return_sources` holds it and it is skipped here; on
            // a path that returns something else the runtime comparison frees it,
            // and a buffer not yet minted on this path is the null sentinel, which
            // `free` ignores.
            //
            // A promoted buffer is exactly an argument whose attribute is HIDDEN
            // and no longer called `__retbuf`: `ref_return` renames the attr to the
            // local's name but leaves `hidden` set, and a user-declared parameter
            // is never hidden.  The un-renamed `__retbuf` placeholder is left alone
            // — no local was promoted onto it, so it holds no store of ours.
            //
            // loft#1096 — and a COLLECTION return's promoted buffer is left alone too,
            // because the premise above ("a buffer not yet minted on this path is the
            // null sentinel, which `free` ignores") is false for it.  The leg reads the
            // buffer as a local THIS function mints; that is true where the caller's
            // work-ref reaches the call as a bare `OpInitRef` sentinel, which is what a
            // record work-ref does.  A collection work-ref does not:
            // `codegen::gen_set_first_vector_null` gives an owned vector local
            // `OpInitRef` + `OpDatabase`, so the buffer arrives ALIVE, and the callee's
            // own `OpDatabase` then only clears it in place (`alloc_record_at` reuses a
            // live slot rather than minting beside it).  So there is never a distinct
            // callee-minted store to reclaim here, and the store this free reached was
            // always the CALLER's — which the caller still names and still frees at its
            // own scope exit.  A `-> vector<T>` function with a `null` arm therefore
            // handed the next call a freed record to `OpClearVector`, and a loop faulted
            // on its second iteration on both backends (`@FR-O-Owner`: a free is for a
            // store the value OWNS).
            let collection_return = crate::parser::vectors::is_collection(
                data.def(self.d_nr).returned.ret_promo_base(),
            );
            for v in 0..function.count() {
                if function.is_argument(v)
                    && !sources.contains(&v)
                    && !null_arm_record_sources.contains(&v)
                    && matches!(
                        function.tp(v),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                {
                    let n = function.name(v);
                    if n != "__retbuf"
                        && !collection_return
                        && let Some(&a) = data.def(self.d_nr).attr_names.get(n)
                        && data.def(self.d_nr).attributes()[a].hidden
                    {
                        null_arm_record_sources.push(v);
                    }
                }
            }
            // loft#1022 — the THIRD shape of the same runtime join, after the null arm
            // above and loft#688's promoted buffer: a return whose arms disagree about
            // OWNERSHIP.  `fn pick(bx, take) -> P { if take { bx.p } else { P { x: 9 } } }`
            // types as `P["bx"]` — a view — while the else arm mints its own store in a
            // work-ref.  `collect_return_sources` is the UNION of the arms, so the
            // work-ref lands in `return_sources` and its scope-exit free is suppressed on
            // EVERY path; on the borrowing path nothing returns it and nothing frees it.
            // One orphan per call, unbounded in a loop, and the store the entry preamble
            // allocated leaks even when the owning arm never runs.
            //
            // The suppression's own comment calls itself PATH-LOCAL, and it is — across
            // separate `return` statements.  A single `return` whose value is a JOIN puts
            // both arms in one set, which is the case the path-locality argument does not
            // cover.  So route it to the same hoist + `OpFreeRefIfDistinct` leg the other
            // two use and let the runtime decide: on the arm that delivers the store the
            // comparison matches and the free is a no-op, on the borrowing arm the stores
            // differ and the orphan is released.
            //
            // The gate needs BOTH conditions, and the loop a third.
            //
            // A RECORD return only.  A collection or text return carries its own mature
            // machinery — loft#936's backing-store comparison and the B5-L3 text hoist —
            // and its `__vdb_N` backing is a `Type::Reference` too, so a test on the
            // SOURCE's type alone claims it and re-routes a return that was already
            // correct (repro_p365's `nested` leaked its backing under exactly that).
            //
            // And a genuine BORROWING ARM, not merely a return type that carries deps.
            // A record LITERAL whose fields alias locals (`TableDef { columns: cols,
            // indexes: ixs }`) has deps too, and every one of its arms delivers the
            // source — nothing can be orphaned, so hoisting it to `__ret_N` and freeing
            // the original released the vectors the returned copy still names, and the
            // sqldb round trip read back zero indexes.
            //
            // And in the loop: only a source THIS function owns.  A source that is
            // itself a borrow carries deps naming what it views, and freeing that would
            // release the CALLER's store — an over-free where the defect is a leak.
            // loft#1142 — the FOURTH shape, and the one that needed the gate widened rather
            // than a new leg.  A KEYED return orphans differently from a record one: the
            // record case above needs an arm that is NOT a source, because that is the arm
            // whose path leaves the source unreturned.  A keyed join can have EVERY arm a
            // source and still orphan, because each arm's `__kvb_N` buffer is ALLOCATED
            // before the branch is tested — `scan_if`'s pre-init prefix emits `Set(v, Null)`
            // for each, and for a keyed local that is not a cheap null but an `OpDatabase`
            // store.  Exactly one arm runs; the rest are minted and freed by nobody, one
            // store per call and unbounded in a loop.  So the condition is *more than one
            // owned source*, not *an arm that is not a source*, and the leg is the same:
            // hoist the join to `__ret_N` and let `OpFreeRefIfDistinct` decide at runtime,
            // which is the only thing that can — which arm ran is not a static fact
            // (@FR-O-Complete: the ownership fact is per binding and PER PATH, and
            // `get_free_vars` was answering it per FUNCTION by suppressing every
            // `return_source` at once).
            //
            // Fixing the ALLOCATION instead would close only half of it: the leak
            // reproduces identically with named locals minted before the `if`
            // (`m = [..]; p = [..]; if c { p } else { m }`), where no pre-init runs at all.
            //
            // The gate needs an owned keyed source AND a way for it not to be returned.
            // "More than one source" covers the two shapes that actually leak — every arm a
            // minted buffer, and every arm a named local — while a PARAMETER arm counts
            // toward that number without being freeable itself: `if c { x } else { [lit] }`
            // has one owned source and still orphans it on the `x` path, which is the cell
            // that showed the first version of this gate was short.  `return_has_non_source_arm`
            // is the record leg's spelling of the same idea and is kept beside it, for an arm
            // that names no source at all.
            // A source that is itself a BORROW names what it views, and freeing that
            // releases the CALLER's store — an over-free where the defect is a leak.
            // `owns_freeable_store` is that question's one home; this leg asked it with a
            // parameter carve-out of its own that never gained the promoted-NRVO-buffer
            // half the record legs above carry.
            let owned_keyed_source = |v: u16| {
                crate::parser::vectors::is_keyed(function.tp(v))
                    && self.owns_freeable_store(function, data, v)
            };
            let keyed_join = crate::parser::vectors::is_keyed(tp.base())
                && (sources.len() > 1 || return_has_non_source_arm(expr, &sources))
                && sources.iter().any(|&v| owned_keyed_source(v));
            if keyed_join {
                for &v in &sources {
                    if !null_arm_record_sources.contains(&v) && owned_keyed_source(v) {
                        null_arm_record_sources.push(v);
                    }
                }
            }
            if matches!(tp.base(), Type::Reference(_, _) | Type::Enum(_, true, _))
                && return_has_non_source_arm(expr, &sources)
            {
                for &v in &sources {
                    if null_arm_record_sources.contains(&v)
                        || !matches!(
                            function.tp(v),
                            Type::Reference(_, _) | Type::Enum(_, true, _)
                        )
                        || !self.owns_freeable_store(function, data, v)
                    {
                        continue;
                    }
                    null_arm_record_sources.push(v);
                }
            }
            sources.into_iter().collect()
        } else {
            HashSet::default()
        };
        // `(F-ParamRebind)`, loft#1871 — a rebound record parameter this return MAY hand out:
        // `get_free_vars` leaves it to this leg, which releases it on the runs where the value
        // returned is another store.  `r(p, n - 1)` after `p = S { … }` returns `p`'s own store
        // from the deepest frame and a fresh one from every other, so the answer is per run.
        let rebound_sources: Vec<(u16, u16)> = if is_return && to_scope == 1 {
            function
                .rebind_params()
                .into_iter()
                .filter(|(p, _)| *p == ret_var || return_sources.contains(p))
                .collect()
        } else {
            Vec::new()
        };
        // Kept for the null-arm join leg below, which releases its sources after this sweep.
        let join_arm_dropped = arm_dropped.clone();
        let mut ls = self.get_free_vars(
            function,
            data,
            to_scope,
            tp,
            ret_var,
            &Delivered {
                sources: return_sources,
                arm_dropped,
                moved_out,
            },
        );
        // @PLN85 P4-records — at a RETURN site, a record work-ref's store may
        // BE the returned store: a named local adopts the arm's fresh Object
        // (`v: E = Pass{..}; if c { v = Fail{..} }; v` — two candidate stores,
        // one winner at runtime), and an unconditional OpFreeRef frees the
        // winner too — the caller then reads a freed store (silently stale
        // without LOFT_POISON; the par t4 catch).  Make every record work-ref
        // free at a return CONDITIONAL on not being the returned store — for
        // an unrelated work-ref the stores are distinct and the free runs
        // exactly as before.
        if is_return
            && ret_var != u16::MAX
            && (ret_var as usize) < function.count() as usize
            && matches!(
                function.tp(ret_var),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
        {
            let free_nr = data.def_nr("OpFreeRef");
            let free_if = data.def_nr("OpFreeRefIfDistinct");
            for op in &mut ls {
                // ANY record-typed local's store may BE the returned store —
                // not only a parser-minted work ref: a NAMED local aliased
                // into the hidden return-buffer param (`best = cand` — the
                // NRVO buffer keeps raw-alias Sets by design) had its
                // unconditional free kill the returned store on the
                // reassigned path (the 150-i306 `choose` shape; poison read
                // the caller's field as 0xDEADBEEF).  Distinct stores free
                // exactly as before.
                if let Value::Call(d, args) = op
                    && *d == free_nr
                    && let Some(a0) = args.first()
                    && let Value::Var(w) = a0.unspan()
                    && matches!(
                        function.tp(*w),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                {
                    *op = Value::Call(free_if, vec![Value::Var(*w), Value::Var(ret_var)]);
                }
            }
        }
        // The B5-L3 wrap (Set(__ret_N, expr); free ops; Return(Var(__ret_N)))
        // must not fire when `expr` is already a `Return` or contains one
        // at its tail — otherwise we'd emit `let _ret = return …` (E0308 in
        // native).  Recurse through `Insert` (which scopes wraps Return in
        // for free-vars cleanup) and `Block`.
        let expr_is_terminal = expr_ends_in_return(expr);
        // @PLN85 P4-records — a null-arm record return: hoist the value to a
        // `__ret_N` temp, then free each suppressed JOIN source CONDITIONALLY
        // (`OpFreeRefIfDistinct(src, __ret_N)`): the present arm returns the
        // source's store (not distinct → kept, transferred to the caller); the
        // null arm returns the sentinel (distinct → the preamble-allocated
        // placeholder is freed — no orphan).  Runs FIRST: the fast path would
        // otherwise emit `Return(expr)` with the sources leaking on the null
        // path (frees suppressed above), and the legacy path would re-open the
        // eval-stack UAF.
        if is_return
            && !expr_is_terminal
            && (!null_arm_record_sources.is_empty() || !rebound_sources.is_empty())
        {
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            // loft#1186 / @PLN150 — when this function's PUBLISHED return names its
            // `__closure`, its callers read the result as a borrow, and the not-distinct leg
            // then leaves the store the callee minted owned by nobody: the callee does not
            // free it (it is the value it returns) and the caller will not (it borrows).
            // `OpFreeRefOrHandUp` is `OpFreeRefIfDistinct` with an owner on that leg.  The
            // distinct leg is identical, so a function whose return does not borrow keeps
            // exactly the op it had.
            let free_if = data.def_nr(if Self::return_borrows_closure(data, self.d_nr) {
                "OpFreeRefOrHandUp"
            } else {
                "OpFreeRefIfDistinct"
            });
            let mut result = Vec::with_capacity(ls.len() + null_arm_record_sources.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            let distinct = data.def_nr("OpDistinctStore");
            let join_arm_dropped = &join_arm_dropped;
            for &src in &null_arm_record_sources {
                // D-heap-7, `(H-Drop)` — the free below releases a source only on the paths that
                // did not return it, and a record's release is its HOOK as well as its store: this
                // leg ran no hook, so the arm that returned `a` lost `b`'s release entirely
                // (`return a ?? b`, `return if c { a } else { b }`).  Guarded by the same store
                // comparison the free makes, so the two cannot disagree about the path, and placed
                // after the value is computed, which is where the source's life ends.
                if !join_arm_dropped.contains(&src)
                    && let Some(hook) = self.scope_end_drop(function, src, data)
                {
                    result.push(v_if(
                        Value::Call(distinct, vec![Value::Var(src), Value::Var(tmp)]),
                        hook,
                        Value::Null,
                    ));
                }
                let free = Value::Call(free_if, vec![Value::Var(src), Value::Var(tmp)]);
                // `@FR-O-Buffer` — the promoted buffer may still hold the store the CALLER
                // handed, which this frame never frees; the entry witness says so per run.
                // Composed with the hook above rather than replacing it: the two guard DIFFERENT
                // failures — the hook is a release this leg would otherwise lose, the witness
                // stops this frame freeing a store it does not own — so either alone is a bug.
                result.push(match self.entry_witness {
                    Some((buf, w)) if buf == src => v_if(
                        Value::Call(
                            data.def_nr("OpDistinctStore"),
                            vec![Value::Var(src), Value::Var(w)],
                        ),
                        free,
                        Value::Null,
                    ),
                    _ => free,
                });
            }
            // A rebound parameter's own store (distinct from the one the caller handed, which
            // its entry witness names) is released unless it is the value returned.
            for &(param, orig) in &rebound_sources {
                result.push(v_if(
                    Value::Call(distinct, vec![Value::Var(param), Value::Var(orig)]),
                    Value::Call(
                        data.def_nr("OpFreeRefIfDistinct"),
                        vec![Value::Var(param), Value::Var(tmp)],
                    ),
                    Value::Null,
                ));
            }
            result.append(&mut ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        }
        // @PLN85 poison-green — a bare-Var tail is free-safe (its slot holds
        // the value) EXCEPT a `&τ` place ref (`RefVar`): returning it DEREFS
        // the place DbRef at the Return, after `ls`'s frees released the
        // source store (the @PLN87 L3/L4 live-read shapes under LOFT_POISON).
        // Exclude it from the fast path so it takes the B5-L3 hoist below —
        // `Set(__ret_N, Var(r))` performs the deref BEFORE the frees.
        // (Text-inner RefVars are EXCLUDED from the exclusion: a `&text` tail
        // is the promoted out-BUFFER returned per the text-return contract —
        // the buffer lives in the caller, so returning it raw is free-safe,
        // and hoisting it emitted a native `Str::new(&local_String)` dangle.)
        let var_is_place_ref = !ls.is_empty()
            && matches!(expr, Value::Var(v)
                if matches!(function.tp(*v), Type::RefVar(inner)
                    if !matches!(inner.base(), Type::Text(_))));
        // @PLN85 Class B2 — a plain text LITERAL tail reads none of the
        // to-be-freed locals and has no side effects, so `ls (frees); return
        // "lit"` is correct and copy-free.  The B5-L3 text hoist below would
        // otherwise mint an OWNED `__ret_N = "lit"` copy (skip_free) that the
        // caller consumes-and-leaks — the leak that appears whenever a
        // text-returning fn with ANY freeable local returns a literal (the
        // p54 match-of-literals / json-classify family).  Returning the literal
        // directly is exactly what a fn with NO frees already emits.
        //
        // @PLN85 corpus — the null-text sentinel `OpConvTextFromNull()` (an
        // early `return null` in a `-> text?` fn) is the SAME free-safe shape:
        // it lowers to `Str::new(STRING_NULL)`, a borrowed static that owns no
        // allocation and aliases none of the to-be-freed locals.  Treated as a
        // literal it returns directly (matching native's `return
        // Str::new(STRING_NULL)`); routed through the B5-L3 text hoist it minted
        // an OWNED skip_free `__ret_N` copy of the sentinel that a non-copying
        // caller (`f(-1) == null`) never freed → append_text orphan.
        let expr_is_null_text_sentinel = matches!(expr.unspan(),
            Value::Call(d, args) if args.is_empty() && *d == data.def_nr("OpConvTextFromNull"));
        // `return ta` where `ta` is an owned text LOCAL and the function holds a hidden
        // `&text` buffer that is not `ta`: deliver through the buffer and free the local
        // (@FR-F-Ret / @FR-F-Call).  The bare-Var fast path below returns the local's slot
        // as-is, which is right for a scalar and for the buffer itself, and hands up a
        // view of an orphan for a `String` nothing frees — a lambda whose one buffer went
        // to `tb` returned `ta` that way, one orphan per call (loft#1357).
        if is_return
            && let Value::Var(v) = expr.unspan()
            && !function.is_argument(*v)
            && !function.is_skip_free(*v)
            && matches!(function.tp(*v).base(), Type::Text(_))
            && !matches!(function.tp(*v), Type::RefVar(_))
            && let Some(buf) = any_text_return_buffer(function, data, self.d_nr)
            && buf != *v
        {
            let mut result = Vec::with_capacity(ls.len() + 3);
            result.push(v_set(buf, Value::Var(*v)));
            result.push(call("OpFreeText", *v, data));
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(buf))));
            return result;
        }
        if ls.is_empty()
            || ((matches!(expr, Value::Null | Value::Var(_) | Value::Text(_))
                || expr_is_null_text_sentinel)
                && !var_is_place_ref)
        {
            if is_return && !expr_is_terminal {
                ls.push(Value::Return(Box::new(expr.clone())));
            } else if matches!(expr, Value::Null) {
                // skip
            } else {
                ls.push(expr.clone());
            }
        } else if expr_is_terminal {
            // expr is already a `Return(...)` (or a `Block`/`Insert(...)` ending
            // in one) — the cleanup was emitted alongside it by the inner Return
            // arm's free_vars call.  Re-emitting `ls` here would duplicate every
            // OpFreeText/OpFreeRef (and tack on a dead `Return(Null)`).  Just
            // propagate the terminal as-is.  #549 bug 2: a terminal *Block* must
            // hit this dedup BEFORE the `Value::Block` insert_free arm below —
            // an explicit `return (owned_text, …)` at a body tail is processed by
            // both the `Value::Return` scan arm AND `convert`'s is_body_return
            // tail sweep; the first makes the synthetic tuple block terminal, and
            // without ordering this check first the second re-ran `insert_free`,
            // emitting a second `OpFreeText` on the owned element (double free
            // under `-C debug-assertions=on`; text.rs:334).
            return vec![expr.clone()];
        } else if let Value::Block(bl) = expr {
            return self.insert_free(bl, &ls, is_return, data, function);
        } else if is_return && is_value_return_type(tp) && !expr_is_terminal {
            // B5-L3: when a value-returning function's tail expression is a
            // non-Block, non-Var, non-Null value (If/Match/Call etc.) and
            // there are free ops to run before return, save the expression's
            // value to a temp, run the free ops, then return the temp.  The
            // old path inserted the expression as a discarded statement and
            // emitted Return(Null) — interpreter bytecode got away with it by
            // reading the expression's result from top-of-stack via Return's
            // `value` bytes, but native codegen produced `let _ = expr; ...;
            // return 0` and dropped the function's actual return value.
            // Skip when expr is already a `Value::Return(...)` — wrapping
            // would generate `let _ret = return …` (E0308 in native).
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            free_copied_text_sources(&mut result, expr, &ls, function, data);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && !expr_is_terminal
            && ret_var == u16::MAX
            && is_heap_return_type(tp)
            && !matches!(expr, Value::Var(_))
            && expr.is_place_read(data)
        {
            // loft#754 — the B5-L3 rule for a HEAP return (`vector` / record /
            // struct-enum) whose tail is a PLACE read.  `is_value_return_type`
            // names only the scalars and the text branch below only text, so
            // such a tail with pending frees reached the fall-through and was
            // emitted as a DISCARDED statement plus a fabricated
            // `Return(Null)`.  The interpreter read the value off eval-stack
            // top and answered correctly; native emitted
            // `let _ = expr; …; return DbRef::NULL`, so
            // `fn f(w) -> vector<u8> { if … { return []; } w.items[0].bytes }`
            // handed back an EMPTY vector — silently, and on one backend only.
            // (rustc even flagged it as `unused_must_use` on the dropped
            // element read.)
            //
            // The hoist states the interpreter's own order in the IR: evaluate
            // the tail, run the frees, return the captured value.
            //
            // A PLACE read is the whole class, and the bound is load-bearing in
            // both directions.  Only a place leaves its value on the eval stack
            // alone — it allocates nothing and writes no return buffer — so
            // only a place can be dropped by a `Return(Null)`; and `Set(tmp,
            // place)` is a bare `DbRef` copy, which is why the hoist adds no
            // ownership.  A CALL tail already delivers through its hidden
            // buffer, and hoisting one instead engaged the store-transfer
            // machinery (`protect_store_frees` + `CopyRefOrNull`) around a
            // borrowed argument, which over-froze the caller's store
            // ("Delete on locked store", `return-borrow-of-mutated-arg`).  A
            // bare `Var` is excluded because the fast path above already
            // returns it directly.
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && !expr_is_terminal
            && ret_var == u16::MAX
            && is_heap_return_type(tp)
            && !has_return_buffer(self.d_nr, data)
            && tail_is_value_call(expr, data)
        {
            // loft#793 — the CALL sibling of loft#754, in the one regime that
            // fix's "a CALL tail already delivers through its hidden buffer"
            // does not cover: a NULLABLE heap return (`-> S?`,
            // `-> vector<S>?`, `-> StructEnum?`).  Only a DENSE heap return is
            // given a hidden `__retbuf` argument — both reservation sites
            // (`parser/mod.rs`, `parser/definitions.rs`) gate on
            // `Reference | Vector | Enum(_, true, _)`, which `Optional` is not
            // — so for a nullable one the callee's value comes back ONLY as
            // the call's own return value.
            //
            // Dropped, the fall-through emitted the call as a DISCARDED
            // statement plus a fabricated `Return(Null)`.  The interpreter
            // read the value off eval-stack top and answered correctly, so
            // the whole class was invisible there; native — and any
            // `StaticCall` into a library's compiled half, which is why it
            // surfaced across a library boundary first — returned the null
            // sentinel.  `fn f() -> S? { return mk(); }` answered null,
            // silently, with the record left leaked.
            //
            // Hoist the call into `__ret_N` and return that, and make each
            // pending record/vector free CONDITIONAL on not being the hoisted
            // store: a callee with a return buffer may deliver a fresh store
            // OR chain the work ref this frame passed in, and only the
            // runtime knows which.
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let free_nr = data.def_nr("OpFreeRef");
            let free_if = data.def_nr("OpFreeRefIfDistinct");
            for op in &mut ls {
                if let Value::Call(d, args) = op
                    && *d == free_nr
                    && let Some(Value::Var(w)) = args.first().map(Value::unspan)
                    && matches!(
                        function.tp(*w),
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
                {
                    *op = Value::Call(free_if, vec![Value::Var(*w), Value::Var(tmp)]);
                }
            }
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            result.append(&mut ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && matches!(tp.base(), Type::Text(_))
            && !expr_is_terminal
            && !matches!(expr.unspan(), Value::Null)
            && let Some(buf) = text_return_buffer_for(expr, function, data, self.d_nr)
        {
            // @FR-F-Ret / @FR-F-Call — an owned text return is delivered through the
            // CALLER's hidden `&text` buffer, never as a view of a local of this frame,
            // and the frame frees every local it owns when it drops.  The block tail
            // already writes that buffer (`text_return` promotes it); an EARLY
            // `return <call>` / `return <view>` / `return a ?? b` used to reach the
            // `__ret_N` hoist below instead, which copied the value into a frame-local
            // `String` that nothing freed — one orphan per call, unbounded in a loop
            // (loft#1338).  Write each arm of the value into the buffer this function
            // already holds from its caller (per arm, so native's arm types stay
            // uniform), free the frame-local temps the copy drained, run the scope-exit
            // frees, and return the buffer.  The buffer is one the value does not
            // READ: `"{x}-{n}"` written into `x` would clear `x` before rendering it.
            let mut delivered = expr.clone();
            crate::parser::Parser::push_text_arms_into(
                &mut delivered,
                buf,
                data.def_nr("OpCreateStack"),
            );
            let mut result = Vec::with_capacity(ls.len() + 3);
            result.push(delivered);
            free_copied_text_sources(&mut result, expr, &ls, function, data);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(buf))));
            return result;
        } else if is_return
            && matches!(tp.base(), Type::Text(_))
            && !expr_is_terminal
            && !matches!(expr.unspan(), Value::Null)
            && let Some(buf) = any_text_return_buffer(function, data, self.d_nr)
        {
            // The value reads every buffer this function holds (`rest[0..3]` where `rest` IS
            // the promoted buffer; a `match` arm that yields the work text), so it cannot be
            // written into one directly — clearing the buffer first would destroy what is
            // being rendered.  STAGE it: copy into a frame-local temp, move the temp's bytes
            // into the buffer, free the temp, run the frees, return the buffer.  Before this
            // the temp itself was returned and orphaned, one `String` per call (loft#1357).
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            function.set_skip_free(tmp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 4);
            result.push(v_set(tmp, expr.clone()));
            free_copied_text_sources(&mut result, expr, &ls, function, data);
            result.push(v_set(buf, Value::Var(tmp)));
            result.push(call("OpFreeText", tmp, data));
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(buf))));
            return result;
        } else if is_return && matches!(tp.base(), Type::Text(_)) && !expr_is_terminal {
            // The residual of the arm above: a text-returning function that holds NO
            // hidden `&text` buffer (a literal tail, or a tail whose promotion the
            // targeted pass declined) and whose value the value reads every buffer of.
            // Save the value's text to a `__ret_N` temp, run the free ops, then return
            // the temp.  The temp's String holds an OWN copy (`OpAppendText` copies
            // bytes), so the frees do not dangle the returned Str.  Its own scope-exit
            // `OpFreeText` is suppressed (`skip_free`): the caller copies the bytes on
            // return, and the String is ORPHANED — this is the one delivery that
            // violates @FR-F-Call's "owned locals freed", kept only where no buffer
            // exists to deliver through (`use_analysis::text_return_orphan_risk` is
            // the predicate that hands such a function a buffer, so a leak here means
            // that predicate did not see this return).
            //
            // Native codegen also needs the wrap (otherwise the call result is
            // dropped + `return null` returns the typed null sentinel), and then
            // collapses `Set(__ret, call); …; Return(__ret)` back to `return
            // Str::new(call(...))` in `output_block`, dropping the temp — which is
            // why native never orphans here.
            self.ret_temp_counter += 1;
            let name = format!("__ret_{}", self.ret_temp_counter);
            let tmp = function.add_temp_var(&name, tp);
            function.set_skip_free(tmp);
            self.var_scope.insert(tmp, self.scope);
            self.var_order.push(tmp);
            let mut result = Vec::with_capacity(ls.len() + 2);
            result.push(v_set(tmp, expr.clone()));
            free_copied_text_sources(&mut result, expr, &ls, function, data);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Var(tmp))));
            return result;
        } else if is_return
            && let Type::Tuple(elems) = tp
            && elems.iter().any(|e| matches!(e, Type::Text(_)))
            && !expr_is_terminal
            && let Value::Tuple(orig_elems) = expr.unspan()
            && orig_elems.len() == elems.len()
        {
            // @P329: tuple-of-text return — when an element is a non-literal
            // expression (typically a Call returning text that borrows from
            // a local), hoist it to a `__ret_text_N` temp before running
            // scope frees.  `Set(__ret_text_N, elem)` lowers to OpAppendText
            // (line 526 / src/state/codegen.rs), which deep-copies bytes
            // into the temp's owned String.  The frees then run safely; the
            // returned tuple's text elements point to temp Strings that
            // outlive the function's scope (skip_free marks them so the
            // function epilogue leaves the allocation for the caller's
            // AppendText to consume on return — same pattern as the
            // single-text B5-L3 branch above, generalised across tuple
            // elements).
            //
            // Without this, a function shape like
            //   fn f<T: Printable>(x: T) -> (text, text) { (x.to_text(), "x") }
            // returns a tuple whose element 0 Str points into the caller's
            // (function-local) __work_1 buffer; the scope's OpFreeText runs
            // BEFORE the Return, invalidating the Str — the caller reads
            // empty / garbage bytes.  See PROBLEMS.md @P329.
            let mut new_elems = Vec::with_capacity(orig_elems.len());
            let mut pre_ops = Vec::new();
            for (elem_expr, elem_type) in orig_elems.iter().zip(elems.iter()) {
                let unspanned = elem_expr.unspan();
                if matches!(elem_type, Type::Text(_))
                    && !matches!(unspanned, Value::Text(_) | Value::Var(_) | Value::Null)
                {
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_text_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, &Type::Text(Deps::none()));
                    function.set_skip_free(tmp);
                    self.var_scope.insert(tmp, self.scope);
                    self.var_order.push(tmp);
                    pre_ops.push(v_set(tmp, elem_expr.clone()));
                    new_elems.push(Value::Var(tmp));
                } else {
                    new_elems.push(elem_expr.clone());
                }
            }
            let mut result = Vec::with_capacity(pre_ops.len() + ls.len() + 1);
            result.extend(pre_ops);
            result.extend(ls);
            result.push(Value::Return(Box::new(Value::Tuple(new_elems))));
            return result;
        } else if is_return
            && !expr_is_terminal
            && matches!(
                tp,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
            && matches!(expr.unspan(), Value::If(c, _, _)
                if matches!(c.unspan(), Value::Insert(ops)
                    if ops.first().is_some_and(|o| matches!(o,
                        Value::Set(v, _) if function.name(*v).starts_with("__lift_")))))
        {
            // @P378(b) — a heap-returning tail `If` whose CONDITION lifted a
            // ref-temp (`__lift_N = call()`; the branches were not unified to a
            // shared work-ref, so `ret_var == u16::MAX`) and which has frees to
            // run before returning.  The fall-through (below) inserts the If as
            // a discarded statement + `Return(Null)`: interpret reads the value
            // off eval-stack-top, but native returns the null DbRef sentinel
            // (keys.rs:251 OOB in the caller).  A `Set(var, If)` save-to-temp
            // does NOT work (native voids the if/else branches → E0308); only
            // `Return(If(...))` value-emits them.  So PRESERVE the Return(If):
            // pull the condition's lift-preamble out, evaluate the boolean to a
            // value-typed temp, run the frees (the lift's OpFreeRef), then
            // `Return(If(Var(cond_tmp), t, f))` — the if/else stays the Return's
            // value-expression and the lift is freed before (not after) it.
            let Value::If(cond, t, f) = expr.unspan().clone() else {
                unreachable!()
            };
            let mut result = Vec::new();
            // split `Insert([__lift = …, …, bool_expr])` into preamble + bool.
            let bool_expr = if let Value::Insert(ops) = cond.unspan() {
                let mut ops = ops.clone();
                let last = ops.pop().expect("non-empty condition Insert");
                result.extend(ops);
                last
            } else {
                (*cond).clone()
            };
            self.ret_temp_counter += 1;
            let cname = format!("__cond_{}", self.ret_temp_counter);
            let cond_tmp = function.add_temp_var(&cname, &Type::Boolean);
            self.var_scope.insert(cond_tmp, self.scope);
            self.var_order.push(cond_tmp);
            result.push(v_set(cond_tmp, bool_expr));
            result.extend(ls);
            result.push(Value::Return(Box::new(v_if(Value::Var(cond_tmp), *t, *f))));
            return result;
        } else {
            // @PLN85 poison-green — the block-VALUE variant of the B5-L3 rule:
            // a non-Void block's exit frees run AFTER the tail expression but
            // BEFORE the enclosing consumer copies the value out
            // (`test_value = { mk()[0] }` — the text tail borrows the
            // block-local vector's element bytes; the block-exit OpFreeRef
            // poisons them before the Set's byte copy).  Hoist the value to a
            // temp: `Set(__blk_N, expr)` deep-copies the bytes (OpAppendText)
            // while the source store is still live, the frees run, and the
            // temp is the block's value.  Text-typed only — the one shape
            // where the Set IS a deep copy; record/vector block values keep
            // their existing paths.
            // (A branch tail — if/match — is EXCLUDED: its arms are unified
            // to write the assignment target directly, so the hoist's Set
            // would append the branch's value a SECOND time onto what the arm
            // already wrote — `if c { null } else { "error" }` read back
            // "errorerror".  Branch tails keep their existing arm-delivery.)
            if !is_return
                && !ls.is_empty()
                && matches!(tp.base(), Type::Text(_))
                && !matches!(expr, Value::Null | Value::Var(_))
                && !Self::tail_is_branch(expr)
                && !expr_is_terminal
            {
                self.ret_temp_counter += 1;
                let name = format!("__blk_{}", self.ret_temp_counter);
                let tmp = function.add_temp_var(&name, tp);
                // @PLN85 n3 — register the hoist temp at the FUNCTION BODY scope
                // (1), not `self.scope` (this nested block's scope, where `__blk_N`
                // is the tail value and so is EXCLUDED from `get_free_vars` as the
                // block's `ret_var` → its owned String leaked, e.g.
                // `test_value = { a = Item{name:"x"}; b = a; a.name }`).  The block
                // value is delivered to the outer consumer by COPY (OpAppendText),
                // never moved, so the temp stays owned and must be freed.  Its
                // `InitText` is hoisted to the function root (the `lift_texts`
                // mechanism below), so its `OpFreeText` must fire exactly ONCE at
                // function exit — registering at scope 1 makes the function-exit
                // sweep (`get_free_vars(to_scope = 1)`) emit it, matching the
                // root-level init and avoiding a per-iteration double-free in loops.
                // The hoist only fires for `!is_return` blocks (always nested,
                // scope >= 2), so `__blk_N` is never a function return value — the
                // function-exit free can never free a value the caller adopts.
                self.var_scope.insert(tmp, 1);
                self.var_order.push(tmp);
                self.lift_texts.push(tmp);
                let mut result = Vec::with_capacity(ls.len() + 2);
                result.push(v_set(tmp, expr.clone()));
                result.append(&mut ls);
                result.push(Value::Var(tmp));
                return result;
            }
            // Whether anything runs BETWEEN the value and the return.  `ls` holds
            // this scope's frees; the `insert` below puts the value in front of
            // them, so a non-empty `ls` here is exactly "frees follow the value".
            let frees_follow = !ls.is_empty();
            ls.insert(0, expr.clone());
            if is_return {
                // P236: when `expr` is an `If/Match` whose unified
                // tail var is known (returned_var(expr) recurses
                // through If — see line 1454), emit
                // `Return(Var(ret_var))` instead of the legacy
                // `Return(Null)` pattern.  The legacy pattern relied on
                // OpReturn(value=N) reading from eval-stack top —
                // bytecode-only, native discards the if/else's value
                // and returns the typed null sentinel.  After Step 2
                // unification (parser/control.rs::unify_if_branches_work_refs)
                // every branch of the if/else writes to the SAME work-
                // ref via OpDatabase + per-field SetInt; the if/else
                // statement leaves all writes in place; native
                // `return var___ref_N` then returns the active
                // branch's value correctly.
                // loft#1097 — `ret_var` cannot carry the sentinel when the tail has a
                // NULL arm and the var it unified onto is a COLLECTION.
                // `returned_var_null_unified` folds a null arm into its sibling's var
                // on the premise it states itself: *"the work-ref null-inits at
                // function entry and a null arm never allocates into it, so
                // `Return(Var(v))` yields the same null the sentinel did"*.  That holds
                // for a RECORD work-ref, which `gen_set_first_ref_null` sentinel-inits.
                // It is false for a collection: `gen_set_first_vector_null` gives an
                // owned vector local `OpInitRef` + `OpDatabase`, and a PROMOTED buffer
                // arrives alive from the caller — so on the null path `Var(v)` is a
                // live, populated vector.  `f(-1) == null` answered FALSE while
                // `len(f(-1))` answered 2, with no diagnostic: @FR-E-Null says the
                // sentinel is a real observable value, and calls.md's `(F-Return)` says
                // a body ending in an expression returns THAT expression — here the
                // expression was demoted to a statement and its value dropped.
                //
                // Hoist the tail's value to a temp instead — the shape the null-arm
                // RECORD join above already uses — so the frees still run between the
                // value and the return while the arm's own answer, sentinel included,
                // is what comes back.  Like loft#957's temp below it is deliberately
                // NOT registered in `var_scope`: it holds the value being transferred
                // to the caller, and a scope-exit free of it would free what the caller
                // adopts.
                // The premise fails for any var the caller HANDS IN as well, record or not:
                // a `-> S?` local promoted onto the caller's buffer arrives as that live
                // record, and a null arm folded onto it answered the record (loft#1934 —
                // `return if c { d } else { null }` never answered null once `S?` took the
                // buffer).
                let null_arm_needs_the_value = ret_var != u16::MAX
                    && frees_follow
                    && !expr_is_terminal
                    && (ret_var as usize) < function.count() as usize
                    && (crate::parser::vectors::is_collection(function.tp(ret_var))
                        || function.is_argument(ret_var))
                    && return_has_null_arm(expr, data.def_nr("OpNullRefSentinel"));
                if null_arm_needs_the_value {
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_tail_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, tp);
                    ls[0] = v_set(tmp, expr.clone());
                    ls.push(Value::Return(Box::new(Value::Var(tmp))));
                } else if ret_var != u16::MAX {
                    ls.push(Value::Return(Box::new(Value::Var(ret_var))));
                } else if frees_follow
                    && *tp != Type::Void
                    && !expr_is_terminal
                    // A CALL or a value BRANCH — the two producers whose result no
                    // binding holds.  A call is the shape `chain_site_set_shape`
                    // promotes into `__retbuf` when promotion does run; a branch is
                    // the shape whose arms `returned_var` could not unify onto one
                    // var, which for a `-> fn(…)` return is every branch, because the
                    // arms yield fn-ref CONSTANTS and there is no work-ref to unify.
                    // A wider test is not a safer one: an earlier cut allowed any
                    // non-null tail and so fired on a COROUTINE's body, whose tail is
                    // the `while` loop itself.  That wrapped the loop into
                    // `__ret_tail_1 = loop { … }`, and the state-machine lowering then
                    // could not see the captured parameters — four `coroutine_matrix`
                    // cells failed to compile with `cannot find value var_n`.  So the
                    // widening names the branch and stops there, and an `iterator`
                    // return stays excluded outright for the same reason: a coroutine
                    // does not return its body's value.
                    && (matches!(
                        expr.unspan(),
                        Value::Call(_, _) | Value::CallRef(_, _) | Value::If(_, _, _)
                    )
                    // …or the return type is a fn-ref, whatever the tail's shape.  This one
                    // is keyed on the TYPE and not on the node because it CANNOT be keyed on
                    // the node: a non-capturing lambda lowers to a bare `Value::Int` holding
                    // its def-number, indistinguishable from the integer 733.  loft#1470 is
                    // that cell — `return fn() -> integer { 5 };` with a free in the frame
                    // emitted the def-number as a DISCARDED statement and fabricated
                    // `return null`, so `--native` returned the typed null sentinel and the
                    // caller read `internal error: invalid fn-ref`.  The interpreter answered
                    // correctly by accident, off eval-stack top, which is the same accident
                    // loft#957 names one carve-out earlier in this list.
                    || matches!(tp.base(), Type::Function(..)))
                    && !matches!(tp.base(), Type::Iterator(_, _))
                {
                    // loft#957 — the same eval-stack reliance P236 names above, for
                    // the case where the value lives in NO variable: a tail
                    // `return <call>` whose callee is a bodiless `#rust` native
                    // returning a collection (`read_bytes`, `list_dir`).  Return
                    // promotion never runs for it — there is no local candidate to
                    // promote — so nothing binds the result, and the legacy shape
                    // lowered to `read_bytes(p); free …; return null`.  The
                    // interpreter answered correctly by accident, its `OpReturn`
                    // taking the eval-stack top the call happened to leave there;
                    // native emitted the typed null sentinel and the bytes were
                    // gone, silently, on a backend `loft test --interpret` cannot
                    // see.  P236 could reuse a variable that already existed; here
                    // there is none, so give the value one and return THAT.
                    //
                    // Gated on frees actually following: with nothing between the
                    // value and the return, the existing shape already emits
                    // `return <expr>` directly and needs no temp.  The temp is
                    // deliberately NOT registered in `var_scope` — it holds the
                    // value being transferred to the caller, so a scope-exit free
                    // of it would free what the caller adopts.
                    self.ret_temp_counter += 1;
                    let name = format!("__ret_tail_{}", self.ret_temp_counter);
                    let tmp = function.add_temp_var(&name, tp);
                    ls[0] = v_set(tmp, expr.clone());
                    ls.push(Value::Return(Box::new(Value::Var(tmp))));
                } else {
                    ls.push(Value::Return(Box::new(Value::Null)));
                }
            }
        }
        ls
    }
}

/// loft#793 — does this function receive a hidden return BUFFER argument?
///
/// Only a DENSE heap return is given one: both reservation sites
/// (`parser/mod.rs`, `parser/definitions.rs`) gate on
/// `Reference | Vector | Enum(_, true, _)`.  A NULLABLE heap return (`-> S?`)
/// is not one of those, so it has no buffer and its value can travel back only
/// as the call's own return value — which is what makes a dropped call tail a
/// silently-null answer there rather than a delivered one.
fn has_return_buffer(d_nr: u32, data: &Data) -> bool {
    data.def(d_nr).attributes().iter().any(|a| {
        a.hidden
            && matches!(
                a.typedef,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
    })
}

/// loft#793 — the tail of `expr` is a CALL that carries the block's VALUE, the
/// shape the legacy `Return(Null)` fall-through drops.
///
/// The typed-NULL producers are excluded — `OpNullRefSentinel` and the
/// `OpConv<T>FromNull` family.  For those `Return(Null)` is exactly right, and
/// hoisting one is actively wrong: a `-> StructEnum?` fall-through null is
/// `OpConvEnumFromNull`, whose native form is the `255u8` discriminator
/// sentinel, so binding it to a `DbRef`-typed temp emitted `255u8 as DbRef`
/// (E0605).
fn tail_is_value_call(expr: &Value, data: &Data) -> bool {
    match expr.tail().unspan() {
        Value::Call(d, _) => {
            let name = data.def(*d).name();
            name != "OpNullRefSentinel"
                && !(name.starts_with("OpConv") && name.ends_with("FromNull"))
        }
        Value::CallRef(_, _) => true,
        _ => false,
    }
}

/// Does this return have an arm that yields something OTHER than one of its
/// `return_sources` — a genuine BORROWING arm (loft#1022)?
///
/// `collect_return_sources` is the UNION of the arms' terminal VARS, so a work-ref that
/// only one arm delivers still lands in it and its scope-exit free is suppressed on
/// every path.  That is right when every arm delivers a source and wrong when one arm
/// hands back a borrow instead: `if take { bx.p } else { P { x: 9 } }` yields a field
/// access on the first arm, which is no variable at all, and the second arm's work-ref
/// is then owned by nobody.
///
/// Answers false for a return with no join in it — one path cannot orphan the value it
/// is itself delivering — and false when every arm's terminal is a source, which is the
/// shape a record literal aliasing locals has.
/// What a scope exit HANDS OUT, and so must not release — the two path-local facts
/// [`Scopes::get_free_vars`] needs about the value leaving with it.
///
/// Both are per RETURN SITE rather than per function: a `break`, a `continue` and an
/// ordinary block exit deliver nothing and pass the empty set.
#[derive(Default)]
pub(super) struct Delivered {
    /// The arm buffers this return delivers, whose scope-exit free the caller owns.
    pub(super) sources: HashSet<u16>,
    /// loft#1515 shape 2 — locals whose HOOK this return already placed inside the arms of
    /// its join, so the common sweep must not run it again. Their FREE is untouched: a store
    /// is freed once whichever arm ran. See [`move_join_hooks_into_arms`].
    pub(super) arm_dropped: HashSet<u16>,
    /// loft#1628 — the local a return copies out and the holders that may hold its record, whose
    /// hooks run only where they hold a DIFFERENT record.  See [`return_moved_holders`].
    pub(super) moved_out: Option<(u16, Vec<u16>)>,
}
