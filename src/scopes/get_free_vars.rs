// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! WHICH variables a scope exit frees: `get_free_vars` picks each local's release — plain,
//! identity-guarded or skipped — from the facts the scan gathered.

use super::backings::{member_backing, outer_collection_backing};
use super::capture_adoption::{
    closure_records_of_source, link_written_closure_records, owning_record_locals,
    record_leaves_frame,
};
use super::capture_builds::{
    adoption_build_is_conditional, capture_adoption_owns_free, escaping_record_holds,
    escaping_record_holds_buffer, free_unless_record_built, join_capture_witness,
    reassigned_join_capture_slot,
};
use super::free_vars::Delivered;
use super::tuple_members::{MemberFacts, tuple_owned_elem_frees};
use super::witness::release_witness;
use super::{Scopes, call};
use crate::data::{Data, Type, Value, v_if, v_set};
use crate::fxhash::FxHashSet as HashSet;
use crate::variables::Function;

/// The members of a TUPLE local whose values live in a backing registered in an OUTER scope,
/// as `(member index, backing)`: a vector member's `__vdb_N`, or the `__ref_N` a whole-tuple
/// bind copied a record member into (loft#1361).  Both are minted at the function's head so the
/// store is reused, so a tuple in a block or a loop body released them after the block had moved
/// on (loft#1588) — the tuple twin of [`outer_collection_backing`].  Only a local the program
/// declared, never an argument backing, and a member whose ONE dep is that backing.
fn outer_tuple_backings(
    function: &Function,
    v: u16,
    exited: &std::collections::HashSet<u16>,
) -> Vec<(u16, u16)> {
    let Type::Tuple(elems) = function.tp(v).base() else {
        return Vec::new();
    };
    if function.is_compiler_generated(v) {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (idx, e) in elems.iter().enumerate() {
        if let Some(b) = member_backing(function, e)
            && !exited.contains(&b)
            && !function.is_argument(b)
        {
            out.push((idx as u16, b));
        }
    }
    out
}

/// @PLN94 TEST-ONLY: the var name whose scope-exit free `get_free_vars` drops (injecting a genuine
/// leak, the `check-leak` true-positive gate), or `None`. Cached — ONE env read per process, so the
/// production (unset) path pays nothing per-free. Never set outside tests.
fn inject_drop_free() -> Option<&'static str> {
    use std::sync::OnceLock;
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| std::env::var("LOFT_OWN_INJECT_DROP_FREE").ok())
        .as_deref()
}

/// @PLN94 TEST-ONLY: the BORROWED var name whose scope-exit free `get_free_vars` is forced to emit
/// (injecting a genuine OVER-free — an unconditional `OpFreeRef` of a dep-carrying view), the
/// over-free-check (`run_over_free_check`) true-positive gate, or `None`. Cached like
/// [`inject_drop_free`]; the production (unset) path pays nothing. Never set outside tests.
fn inject_free_borrowed() -> Option<&'static str> {
    use std::sync::OnceLock;
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| std::env::var("LOFT_OWN_INJECT_FREE_BORROWED").ok())
        .as_deref()
}

/// @FR-O-Override TEST-ONLY: the NEVER-FREE var name for which `get_free_vars` emits a
/// witness-guarded free against ITSELF — `OpFreeRefIfDistinct(v, v)`, a run-time no-op (one store on
/// both sides) that the IR nevertheless NAMES as a free of a never-free binding, which
/// `ownership_cfg`'s Check D must report.  The check's true-positive gate; cached like
/// [`inject_drop_free`]; never set outside tests.
fn inject_free_skipfree() -> Option<&'static str> {
    use std::sync::OnceLock;
    static V: OnceLock<Option<String>> = OnceLock::new();
    V.get_or_init(|| std::env::var("LOFT_OWN_INJECT_FREE_SKIPFREE").ok())
        .as_deref()
}

impl Scopes<'_> {
    /// Has @FR-O-Oracle DERIVED an owner fact for `v`, rather than defaulted to one?
    ///
    /// The question `owns_freeable_store` cannot ask from `deps` alone, and the one phase 0
    /// measured: `Own::Owned` with `OwnEvidence::Derived` or `Minted` is a positive answer,
    /// while `Fallback` (nothing to read) and `Own::Unknown` (read, could not conclude) are
    /// not.  A parameter answers `Borrowed` of itself and is excluded by the carve-out above
    /// before this is reached.
    fn oracle_derived_owner(
        &self,
        function: &crate::variables::Function,
        data: &Data,
        v: u16,
    ) -> bool {
        if usize::from(v) >= function.count() as usize {
            return true; // not a binding this can ask about — keep the existing answer
        }
        let defs = crate::use_analysis::function_defs(data, self.d_nr);
        let (own, evidence) =
            crate::use_analysis::ownership_evidence_with(data, self.d_nr, v, &defs);
        // The plan's refusal set, and NOT a wider one: *a free whose licence is proxy-only,
        // with no oracle agreement*.  So the refusal is `Fallback` (nothing to read) and
        // `Own::Unknown` (read, could not conclude) — the two the census calls `proxy-alone`
        // and `no-answer`.
        //
        // ⚠ An oracle DISAGREEMENT (`Borrowed`/`Join`) is deliberately NOT refused, and
        // getting that wrong is measured: refusing it too made the rung reject 9349 frees
        // instead of ~1600, and shrinking the proxy-only set by 77 % then moved the deny cost
        // by nothing at all, because the disagreements were carrying it.  Those are mostly
        // ownership-TRANSITION frees reading the `owned_refs` memo (@FR-O-Latest), a fact this
        // question cannot see — the census's own doc says so — so calling them unlicensed is
        // this predicate exceeding what it knows.
        !matches!(own, crate::use_analysis::Own::Unknown)
            && !matches!(evidence, crate::use_analysis::OwnEvidence::Fallback)
    }

    /// Which variables this scope must free on the way out, as the `OpFreeRef` /
    /// `OpFreeText` / per-element ops to emit before leaving it.
    ///
    /// Enforces @FR-O-Derived — free placement is DERIVED, not decided: a local is freed
    /// iff it OWNS its store and does not transfer it out, once, at scope exit — and
    /// @FR-O-Owner, the single-owner invariant that makes "once" correct.  There is no
    /// per-site heuristic here; every arm answers from a carried fact.
    ///
    /// `ret_var` and `return_sources` are what "does not transfer it out" means in
    /// practice: a store handed to the caller is the caller's to free (@FR-O-Move), so its
    /// scope-exit free is suppressed.  `return_sources` is PATH-LOCAL — a source dead on a
    /// sibling path is absent from it and is still freed by that path's own sweep, which is
    /// what keeps @FR-O-Complete true without a global "skip" stamp that would over-suppress
    /// and leak.
    ///
    /// ⚠ Ownership is read here from a carried fact, but "empty deps" is only a PROXY for
    /// it (loft#723) — see [`crate::variables::Function::is_skip_free`], the second fact
    /// that vetoes the proxy for a borrow whose dep list was never populated.
    ///
    /// @C88 — this gate stays dep-derived; it is not simplified into "emit more frees and let
    /// a free be idempotent".  Simplifying it means promoting the ownership oracle to authority.
    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn get_free_vars(
        &mut self,
        function: &mut Function,
        data: &Data,
        to_scope: u16,
        tp: &Type,
        ret_var: u16,
        delivered: &Delivered,
    ) -> Vec<Value> {
        let Delivered {
            sources: return_sources,
            arm_dropped,
            moved_out,
        } = delivered;
        // loft#1628 — a holder's hook runs only where it holds a record other than the one
        // this return hands out.
        let unless_moved = |h: u16, hook: Option<Value>| match moved_out {
            Some((x, holders)) if holders.contains(&h) => hook.map(|hook| {
                v_if(
                    Value::Call(data.def_nr("OpNeRef"), vec![Value::Var(h), Value::Var(*x)]),
                    hook,
                    Value::Null,
                )
            }),
            _ => hook,
        };
        let scope_debug =
            crate::env_once!(std::env::var("LOFT_LOG").as_deref() == Ok("scope_debug"));
        let mut ls = Vec::new();
        // The conditional releases of loft#1464, kept apart so they can go FIRST.  Each reads a
        // closure-record local to decide whether that record's cascade already owes this store,
        // and the sweep's own `OpFreeRef` of that record would destroy the answer: the two
        // backends do not even agree on what a freed slot reads back as (the interpreter leaves
        // it standing, `--native` nulls `store_nr`), so a guard placed after it would decline on
        // one and double-free on the other.
        let mut guarded: Vec<Value> = Vec::new();
        // The owners whose drop a conditional release below already carries — one hook per
        // store, on the FIRST release of it (a view and its backing free one store).
        let mut guarded_hooks: std::collections::HashSet<u16> =
            std::collections::HashSet::default();
        let vars = self.variables(to_scope);
        if scope_debug {
            eprintln!(
                "[get_free_vars] fn={} to_scope={to_scope} scope={} vars={vars:?} ret_var={ret_var} \
                 return_sources={return_sources:?}",
                data.def(self.d_nr).name,
                self.scope
            );
        }
        // @PLN85 A.1 part i — the directly-owned heap BUFFER of EACH transferred
        // return arm (the union-of-arms SET, not just `returned_var`'s single
        // var) is freed by the caller, so suppress its scope-exit free here.
        // PATH-LOCAL: `return_sources` is this return's set, so a source dead on
        // a sibling path is absent and still freed by that path's sweep (no
        // global `skip_free` stamp — that would over-suppress and leak the
        // dead-path allocation).  The dep buffer of a BORROWING terminal
        // (`_vec_N["__vdb_N"]`) is handled in the `leaves_frame` computation below.
        //
        // The SET drives suppression ONLY for the heap-buffer block (Vector /
        // Reference / Enum / keyed).  TEXT and TUPLE returns keep their own,
        // mature free paths (`OpFreeText` + the B5-L3 `__ret_N` text hoist;
        // per-element tuple frees) — a `match`-returning-text whose owned
        // `__work_N` buffer is alive but unused on a sibling arm MUST still be
        // freed, and short-circuiting the whole iteration on `return_sources`
        // would leak it (the enum-vector param-return `show()` regression).
        //
        // loft#1150 — `.base()`, because the block this set drives is *"Vector / Reference /
        // Enum / keyed"* and a `τ?` IS one: `@FR-L-Null` gives it the same layout and the same
        // store.  Asked bare, an `Optional(Hash)` arm buffer failed the suppression and took
        // an UNCONDITIONAL scope-exit free on top of the conditional one loft#1142 emits — so
        // the store being returned was freed and the caller read a dead record.  That is the
        // fifth site where this same list has drifted short by the wrapper (`is_dbref` here
        // and at D-own-13, `deps_mut`, `is_keyed`, `depend`); `is_dbref`'s own doc records
        // that it drifts when restated, and it drifts when asked BARE too.
        let suppress_source = |function: &Function, v: u16| {
            return_sources.contains(&v) && crate::data::is_dbref(function.tp(v).base())
        };
        // loft#1443's second half — the closure records this frame writes OUT through a
        // `&fn(…)` link.  A write through a link DELIVERS, exactly as a `return` does
        // (`(B-Ref-Uniform)`: a `&τ` variable is used exactly like a τ variable), so the
        // caller holds the record and this frame must not free it.  Computed once here
        // rather than per variable: the reading walks the body.
        let link_delivered = link_written_closure_records(data, function, self.d_nr);
        let exited: std::collections::HashSet<u16> = vars.iter().copied().collect();
        for v in vars {
            if v == ret_var || suppress_source(function, v) {
                continue;
            }
            // `@FR-L-CapKeep` — a closure record another live fn-ref still names is that
            // name's to release: the record local lets go of it before its own release.
            if self.closure_keep.gated && self.closure_keep.records.contains(&v) {
                let mut holders = self.closure_keep_live(&[]);
                let (link_pre, links, link_post) = self.closure_keep_links(function);
                holders.extend(links);
                if !holders.is_empty() {
                    let sentinel = Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new());
                    ls.extend(link_pre);
                    ls.push(Self::closure_keep_unless_shared(
                        &Value::Var(v),
                        &holders,
                        &[],
                        Value::Null,
                        &v_set(v, sentinel),
                        data,
                    ));
                    ls.extend(link_post);
                }
            }
            // `@FR-H-Drop`, the scope-end clause — a VECTOR local whose elements live in a
            // backing registered in an OUTER scope (its `__vdb_N`, or the `__ref_N` buffer a call
            // delivered it through, both minted at the function's head so the store is reused)
            // is released HERE, at its own scope's end, and emptied.  Left to the backing, a loop
            // body's vector released its elements during the NEXT pass, at the re-mint, and the
            // last pass's after the loop (loft#1565); a call-delivered one never, because the
            // backing's release looks for a binding that is out of scope by then.  Emptying it
            // is what keeps the re-mint and the backing's own release from running the hooks a
            // second time — `clear` runs none (`H-Drop-Not`).  The release is the backing's
            // (`scope_end_drop`), so a hand-off that stopped it still stops it.
            // A CAPTURED local's store is the closure record's to release (`@FR-L-CapOwn`,
            // `capture_adoption_owns_free`), at every scope's end as at the function's: released
            // here as well, its hooks ran twice (loft#1606).
            if let Some(backing) =
                outer_collection_backing(function, v, &exited, self.bind_backing.get(&v).copied())
                && !capture_adoption_owns_free(data, function, &self.capture_build_backing, v)
                && let Some(hook) = self.scope_end_drop(function, backing, data)
            {
                ls.push(hook);
                ls.push(call("OpClearVector", v, data));
            }
            // The same clause for a TUPLE local's members (loft#1588).  A vector member is
            // released and emptied as above.  A record member's backing is released, freed and
            // reset to the null sentinel, as a loop's call buffer is: the next pass then mints
            // afresh and the function's own sweep of the backing finds nothing.  Declined where a
            // variable of a scope that stays OPEN views the backing — freeing it would free a
            // record in use.  A temp of a block that already ended (the literal's own `_vec_N`)
            // is still in `var_scope`, and is not a viewer.
            for (idx, backing) in outer_tuple_backings(function, v, &exited) {
                let viewed = self.var_scope.iter().any(|(&x, s)| {
                    x != backing
                        && x != v
                        && !exited.contains(&x)
                        && (self.stack.contains(s) || *s == self.scope)
                        && function.tp(x).depend().contains(&backing)
                });
                if viewed {
                    continue;
                }
                let hook = self.scope_end_drop(function, backing, data);
                if matches!(function.tp(backing).base(), Type::Vector(_, _))
                    || function.name(backing).starts_with("__vdb_")
                {
                    // Emptied even where a hand-off stopped the hook: the copy that took the
                    // release holds the elements now, and a re-mint of this backing on the next
                    // pass must not release them again.
                    if hook.is_some() || self.drop_transferred.contains(&backing) {
                        ls.extend(hook);
                        ls.push(Value::Call(
                            data.def_nr("OpClearVector"),
                            vec![Value::TupleGet(v, idx)],
                        ));
                    }
                } else if matches!(function.tp(backing).base(), Type::Reference(_, _))
                    && let Some(hook) = hook
                {
                    ls.push(hook);
                    ls.push(call("OpFreeRef", backing, data));
                    ls.push(v_set(
                        backing,
                        Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                    ));
                }
            }
            // loft#1336 / @FR-O-Witness — a witnessed local's store is released through its
            // witness, which names it only while the local still holds it; the local itself
            // is never-free.  A returned local is skipped above like any other: the store is
            // handed up, and the witness is not released for it.
            if let Some(&w) = self.owner_witness.get(&v) {
                // @FR-L-CapOwn, loft#1464 — the second route to the same missing release.  A
                // witnessed local that a closure record ADOPTED gives its store up at the
                // build (`built_here` resets the witness to the sentinel there), so on a run
                // that skips a CONDITIONAL build neither party holds it: the witness names
                // nothing and the cascade never ran.  Same conditional release as the
                // suppression site below, declining by store identity where the witness is
                // about to release the store itself.
                if adoption_build_is_conditional(&self.capture_build_backing, v)
                    && let records = owning_record_locals(
                        data,
                        function,
                        self.d_nr,
                        &self.capture_build_backing,
                        v,
                    )
                    && !records.is_empty()
                {
                    guarded.push(free_unless_record_built(v, &records, Some(w), None, data));
                }
                ls.push(release_witness(
                    w,
                    unless_moved(w, self.witness_hook(function, data, v, w)),
                    data,
                ));
                continue;
            }
            // on=4 iteration scratch (`hash_scratch`): a `return` out of an exposed loop
            // bypasses the loop epilogue's OpFreeScratch, so free the dedicated scratch
            // store here at scope exit too.  OpFreeScratch is conditional (frees only a
            // read-only source's dedicated store) and rec==0-guarded; the epilogue nulls
            // the var on the complete/break paths, so the two never double-free.
            // (expose-iteration-scratch.md Open question A.)
            if function.name(v).contains("hash_scratch") {
                ls.push(call("OpFreeScratch", v, data));
                continue;
            }
            // T1.3: tuple scope exit — free owned elements in reverse index order.
            if let Type::Tuple(elems) = function.tp(v) {
                let elems = elems.clone();
                ls.extend(tuple_owned_elem_frees(
                    &elems,
                    v,
                    data,
                    function,
                    MemberFacts {
                        call_mints: self.tuple_call_mint.get(&v),
                        handed: Some(&self.drop_transferred),
                        moved: Some(&self.tuple_moved),
                    },
                    None,
                ));
                ls.extend(self.tuple_handle_frees(v, function, data));
                continue;
            }
            if matches!(function.tp(v).base(), Type::Text(_)) {
                // @PLN25 slice (c): peel `Optional` — a `text?` local owns the same heap
                // text as `text` and must be freed identically (else its interval is not
                // extended and the slot allocator aliases it — the `text? = text?` copy
                // read back empty).
                // @PLAN52 cluster I iteration 2 (2026-05-30): honor skip_free
                // for text vars too.  The file-level "Text exception"
                // doc-comment ("OpFreeText is always emitted ... regardless
                // of deps") remains true for borrowed-from-parameter text
                // (`dep` non-empty, not skip_free).  The new rule only
                // suppresses OpFreeText for an EXPLICITLY-set skip_free text
                // temp — used by the `__ncc_N` null-coalesce temp at
                // `src/parser/operators.rs::build_null_coalesce_default` so
                // the present-path Str outlives the block scope.  Native
                // emit's `needs_ncc_materialise` (in `output_block`)
                // materialises an owned String inside the block tail so the
                // outer consumer takes ownership cleanly on both backends.
                if !function.is_skip_free(v) {
                    ls.push(call("OpFreeText", v, data));
                }
            }
            // P193: include keyed collections (Sorted/Hash/Index/Radix)
            // — `gen_set_first_keyed_null` allocates a fresh store via
            // `OpDatabase` for each local-var keyed collection, so each
            // needs scope-exit `OpFreeRef`.  Without this they leak as
            // "Stores not freed at program exit".
            if let Type::Reference(_, dep)
            | Type::Vector(_, dep)
            | Type::Enum(_, true, dep)
            | Type::Sorted(_, _, dep)
            | Type::Hash(_, _, dep)
            | Type::Index(_, _, dep)
            // @PLN25 slice (c): peel `Optional` so an owned `vector?`/`reference?` local is
            // still freed at scope exit (same reasoning as the `text?` case above).
            | Type::Radix(_, _, dep) | Type::Trie(_, _, dep) = function.tp(v).base()
            {
                // H2 step 5 (DEPS_INVENTORY): the declared return type's
                // dep list is DEF-space — attr indices from `ref_return`,
                // plus tagged callee-frame notes (a returned fn-ref's
                // closure work var, `Deps::callee_frame1`).  Decode per
                // entry; the historical positional guess (in-range = attr
                // index, out-of-range = frame var) is retired.
                let def = data.def(self.d_nr);
                let ret_borrows_v = def.returned.deps_ref().is_some_and(|deps| {
                    deps.entries().any(|e| match e {
                        crate::data::DepEntry::Attr(a) => {
                            (a as usize) < def.attributes.len()
                                && function.var(&def.attributes[a as usize].name) == v
                        }
                        crate::data::DepEntry::CalleeFrame(w) => w == v,
                    })
                });
                // @PLN85 A.1 part i — `v` is the dep BUFFER of a borrowing
                // return-source terminal (`_vec_N["__vdb_N"]`, where `_vec_N` is
                // in `return_sources` and its dep names `v`): the buffer's store
                // is what the caller adopts, so suppress it here.  Path-local for
                // the same reason as the directly-owned case above.
                let backs_return_source = return_sources
                    .iter()
                    .any(|&src| src != v && function.tp(src).depend().contains(&v));
                // `leaves_frame` is really *"does `v` leave this frame"*, and until loft#1443 the
                // return was the only way out — a `&fn(…)` parameter did not compile.  Now it
                // does, and a closure record written through one is delivered to the caller,
                // which frees it at ITS scope exit under the fn-ref that received it.  Without
                // this term the callee freed the record it had just handed over, and its
                // cascade took the capture with it: the caller then called a closure over
                // `0xDEADBEEF`.  Invisible without `LOFT_POISON=1`, because a freed arena slot
                // still reads back the bytes it held — every cell of the loft#1443 guard passed
                // on stale data.
                // A record whose link delivery the frame decides at run time
                // (`@FR-L-CapKeep`) stands down above where the caller holds it, and is
                // released here on the runs that did not deliver it.
                let leaves_frame = ret_borrows_v
                    || backs_return_source
                    || (link_delivered.contains(&v) && !self.closure_keep.decides_link(v))
                    || ret_var != u16::MAX && function.tp(ret_var).depend().contains(&v)
                    // …and a CLOSURE RECORD this return delivers.  `ret_borrows_v` decodes the
                    // declared return's `CalleeFrame` note, and that note is published once per
                    // lambda and OVERWRITTEN — so where a function builds more than one it names
                    // the last one BUILT rather than the ones the return can hand out.  A `match`
                    // whose arms each build a closure has one record per arm; the note named one
                    // and the frame freed the rest, cascading into everything they captured, so
                    // the caller called a live lambda over a released capture (loft#1474).
                    //
                    // [`record_leaves_frame`] is the same question asked of the VALUES in return
                    // position, which is where loft#1444 already moved this for the fn-ref
                    // variable's own free; the record local's free was left reading the note.
                    // Gated on the local actually being a closure record so this stays a
                    // statement about `@FR-L-CapOwn` and not a new suppression for every
                    // reference local.
                    || (matches!(function.tp(v), Type::Reference(r, _)
                            if data.def(*r).name.starts_with("__closure_"))
                        && record_leaves_frame(data, function, self.d_nr, v));
                // H2 step 5 (DEPS_INVENTORY): the BLOCK-RESULT type's deps were
                // read here for years under the positional guess.  That read is
                // RETIRED: the declared-return (`ret_borrows_v`, a TYPED decode),
                // returned-var, and return-source-backing checks above decide the
                // suppression.  A debug sentinel used to scream when the old read
                // would have "decided alone" (`tp.depend()` names `v` while
                // `leaves_frame` is false), on the theory that such a case would need
                // the read re-added.  It does NOT: every firing is a FALSE positive
                // of the retired POSITIONAL decode — a field / enum-field / match-
                // arm return that COPIES its source into the caller's retbuf
                // (`return fv_c.pts`, `match e { Filled{items} => items }`), so the
                // local source `v` is correctly freed at scope exit AFTER the copy.
                // Re-adding the read would instead SUPPRESS that free and LEAK the
                // source.  Verified on the seven firing cases (450, 508, repro_p365,
                // four 85-store-lifetime-*) — value + leak + LOFT_POISON + the DA
                // store-free asserts all clean, both backends — so the read stays
                // retired and the sentinel is removed (the reliable checks subsume
                // every TRUE return source; the positional read only added noise).
                // Work-refs (`__ref_N` / `__rref_N`) carry their own var
                // in the dep list (`src/parser/mod.rs:1924-1928`) so the
                // standard `dep.is_empty()` gate skips them.  But work-
                // refs allocated to back ref-returning calls accumulate
                // unfreed stores when:
                //   - `gen_set_first_ref_call_copy`'s `0x8000` doesn't
                //     fire (e.g. when the callee MIGHT return a DbRef
                //     aliasing one of its args), or
                //   - the call-site reuses the same work-ref slot across
                //     loop iterations and `OpDatabase`'s `clear+claim`
                //     leaves the store marked `free` from the previous
                //     iteration even while live data lives in it.
                // Free them explicitly at function exit so the leak-check
                // at `src/state/debug.rs:1045` doesn't trip.  Skip when
                // the work-ref participates in the return chain.
                let is_work_ref = {
                    let n = function.name(v);
                    n.starts_with("__ref_") || n.starts_with("__rref_")
                };
                // @P302 — a keyed-collection local backed by its OWN store
                // carries a self-dep `[v]` (added by the `s = []` clear path
                // so a later `s += …` re-inits in place).  That self-dep is an
                // ownership marker, not a borrow — treat it like `dep.is_empty()`
                // so the store is freed at scope exit.  Mirrors the fn-ref
                // ownership rule below.  Keyed-only + exact self-dep; `leaves_frame`
                // still suppresses returned keyed locals.
                // D-own-16 residual — a nullable heap local BOUND FROM A PARAMETER and later
                // reassigned from a minting call owns its store on some paths and borrows on
                // others (`d: S? = p; if c { d = mint(d) }`), and the dep list is
                // flow-INsensitive so it reports the borrow forever: the scope-exit free is
                // suppressed and every mint leaks.  @FR-O-Latest is a per-RUN fact, and here it
                // is decidable at runtime WITHOUT a witness slot, because the dep NAMES the
                // variable this local might still be aliasing — distinct stores mean the local
                // minted its own.  A static strip cannot do this: on the not-taken branch the
                // local still holds the caller's store and freeing it is a use-after-free two
                // frames up, which is why `displaced_owned_slots` excludes arguments.
                //
                // Restricted to an ARGUMENT dep on purpose.  A parameter's slot is stable for
                // the frame (or has an entry stash, below); an arbitrary local dep can itself be
                // freed or reassigned before this scope ends, and then the comparison names a
                // store that is already gone.
                // The one-argument borrow is `Function::borrows_one_argument`, the spelling both
                // backends' displacement frees read (@FR-O-NoDiverge); this sweep asks it of a
                // RECORD local only, and never of a never-free one (@FR-O-Override).
                let borrow_witness = if function.borrows_one_argument(v)
                    && matches!(
                        function.tp(v).base(),
                        Type::Reference(_, _) | Type::Enum(_, true, _)
                    )
                    && !function.is_skip_free(v)
                {
                    // A REBINDABLE parameter's slot stops naming the caller's store once it is
                    // rebound, so compare against the @PLN87 entry stash that still does.
                    Some(function.rebind_orig(dep[0]).unwrap_or(dep[0]))
                } else {
                    None
                };
                // @FR-O-Proxy asks free — this is the scope-exit sweep's own licence, and it
                // frees more bindings than every other site in the compiler together.  The
                // @FR-O-Override veto is consulted in `emit` below, which is the same
                // conjunction spelled across two statements.
                //
                // ⚠ It is NOT `Scopes::owns_freeable_store`, and @PLN155 phase 3 measured that
                // the hard way: the ladder was attached to that predicate and its gate NEVER
                // FIRED, because the sweep does not go through it.  That predicate is the
                // licence for the null-arm / keyed leg; THIS is the licence for the sweep.
                let owns = (function.proxy_says_owned(v)
                    || self.lift_join_witness.contains_key(&v)
                    || borrow_witness.is_some()
                    || (dep.len() == 1
                        && dep[0] == v
                        && crate::parser::vectors::is_keyed(function.tp(v))))
                    // @PLN155 phase 3, `LOFT_OWN_FREE=deny` — the rung: the proxy is not
                    // enough, the ORACLE must also have derived an owner fact.  Off by
                    // default; it exists to measure what refusing those frees costs.
                    && (!crate::keys::own_free_deny()
                        || self.oracle_derived_owner(function, data, v));
                // Plan-57 Phase B (Mechanism B), widened by #323: a
                // Reference-typed capture — a boxed `__cell_<T>` AND, per
                // P260's storage rule, any plain struct capture — is OWNED
                // by the closure record, not the defining frame.  The
                // record stores the capture's 12-byte DbRef and
                // `free_named`'s cascade (allocation.rs) walks every DbRef
                // field when the record dies, so the defining-frame
                // `OpFreeRef` here is redundant — and actively harmful: for
                // an ESCAPED closure (factory return) it frees a slot the
                // live closure still references, the next allocation reuses
                // it, and the closure silently corrupts the new occupant
                // (#323; interp only *appeared* sound through slot-reuse
                // luck).  Suppress it; the cascade is the sole owner for
                // escaping AND in-frame captures (in-frame: the fn-ref's
                // own scope-exit free triggers the cascade).
                //
                // The handover is only sound where this free EXISTS to be
                // suppressed — `owns` — so the record's cascade must reach
                // no further.  #682 is what happens when it does: a captured
                // PARAMETER never enters this sweep (see `variables()`: "never
                // return function arguments") and a projection local is
                // `owns == false`, yet the cascade freed both, destroying the
                // caller's store.  `mark_borrowed_captures` recomputes this same
                // verdict once every function is scanned and marks those captures
                // borrowed on the record, which is what stops the cascade.
                //
                // `capture_adoption_owns_free` is where the rule itself lives — this is
                // its first consumer, `check_ref_leaks` its static mirror, and
                // `ownership_cfg`'s leak oracle the third.  It is the SIXTH site in the
                // drifted-list family the loft#1150 note above enumerates (`is_dbref` here
                // and at D-own-13, `deps_mut`, `is_keyed`, `depend`), which is why it is a
                // call and not a `matches!` written out again.
                let captured_ref =
                    capture_adoption_owns_free(data, function, &self.capture_build_backing, v);
                // @PLN94 TEST-ONLY over-free injection (never set in production): force the scope-exit
                // free of a NAMED borrowed var (owns=false) so the over-free check has a firing
                // true-positive. Subject to the same !leaves_frame/!skip_free/!captured guards as a real free.
                let inject_free = inject_free_borrowed() == Some(function.name(v));
                let emit = (owns || is_work_ref || inject_free)
                    && !leaves_frame
                    && !function.is_skip_free(v)
                    && !self.free_transferred.contains(&v)
                    && !captured_ref;
                // @FR-L-CapOwn, loft#1464 — the release the frame just gave up is taken over by
                // a cascade that runs only if the closure BUILD executes.  The suppression is a
                // static fact about the function; the cascade is a per-RUN one, and where the
                // build sits in a branch the two disagree on every run that skips it — the
                // store is then freed by nobody.  Keep the frame's release there and make it
                // conditional on the record existing, which is the same currency @FR-O-Witness
                // uses for a local whose assignments mix ownership: a fact only the run knows,
                // read off a slot at the moment the answer is needed.
                if !emit
                    && captured_ref
                    && adoption_build_is_conditional(&self.capture_build_backing, v)
                    && let records = owning_record_locals(
                        data,
                        function,
                        self.d_nr,
                        &self.capture_build_backing,
                        v,
                    )
                    && !records.is_empty()
                {
                    // The release this stands in for is the frame's whole one, hook included
                    // (`@FR-H-Drop`): on a run that skips the build nothing else releases the
                    // elements.  The hook is the OWNER's — a vector local viewing its backing
                    // (`w` over `__vdb_N`) frees the same store — and it runs once, ahead of
                    // whichever of the two is released first.
                    let owner = match function.tp(v).depend().as_slice() {
                        [] => Some(v),
                        [b] => Some(*b),
                        _ => None,
                    };
                    let hook = owner
                        .filter(|o| guarded_hooks.insert(*o))
                        .and_then(|o| self.scope_end_drop(function, o, data));
                    guarded.push(free_unless_record_built(v, &records, None, hook, data));
                }
                if function.is_skip_free(v) && inject_free_skipfree() == Some(function.name(v)) {
                    ls.push(Value::Call(
                        data.def_nr("OpFreeRefIfDistinct"),
                        vec![Value::Var(v), Value::Var(v)],
                    ));
                }
                if scope_debug && !emit {
                    // Every conjunct of `emit`, because the question this line is read to
                    // answer is WHICH of them declined.  Three of the six used to be
                    // printed and `owns` / `captured_ref` / `free_transferred` were not,
                    // so a suppression by one of those read as a suppression by any of
                    // them — the reader then re-derives the verdict by hand, which is how
                    // loft#1487 was first attributed to the wrong predicate.
                    eprintln!(
                        "[scope_debug] NOT freeing '{}' (var={v}, scope={}, to_scope={to_scope}): \
                         dep_empty={} owns={owns} is_work_ref={is_work_ref} leaves_frame={leaves_frame} \
                         skip_free={} free_transferred={} captured_ref={captured_ref}",
                        function.name(v),
                        self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        dep.is_empty(),
                        function.is_skip_free(v),
                        self.free_transferred.contains(&v),
                    );
                }
                if emit {
                    if scope_debug {
                        eprintln!(
                            "[scope_debug] freeing '{}' (var={v}, scope={})",
                            function.name(v),
                            self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        );
                    }
                    // when `v` is a `__ref_*` / `__rref_*` work-ref
                    // that was passed to a user-fn call whose Reference
                    // result lives on as `witness`, emit the runtime-
                    // conditional `OpFreeRefIfDistinct(v, witness)` — it
                    // is a no-op in the adoption case (v and witness
                    // share a store) and a real free in the fresh-store
                    // case (distinct stores, placeholder orphaned).
                    // Falls through to plain `OpFreeRef` when no pairing
                    // was recorded.
                    // @PLN125 arc B — the scope-end hook fires where the BINDING's life
                    // ends, which is not always where the STORE is released.  A value
                    // delivered through a caller-side return buffer has two variables
                    // naming one record: the buffer (`__ref_N`, function-scoped and
                    // reused) and the witness the author actually bound.  The author's
                    // binding is the one that ends — per loop iteration, at its own
                    // scope — so the witness drops and the buffer never does.  Getting
                    // that backwards is a rollback that runs once for a loop that opened
                    // a transaction on every pass.
                    let is_buffer = is_work_ref && self.paired_witness.contains_key(&v)
                        || self.witness_buffer.values().any(|bs| bs.contains(&v));
                    if let Some(&jw) = self.lift_join_witness.get(&v) {
                        // loft#1257 — free the lifted collection return only where it is NOT
                        // the caller's own store.  A `Join` is owned on one arm and a borrow
                        // on the other and they are the SAME call, so nothing static separates
                        // them; the store number does.
                        if let Some(hook) = unless_moved(v, self.scope_end_hook(function, v, data, arm_dropped)) {
                            ls.push(hook);
                        }
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(jw)],
                        ));
                    } else if is_work_ref && let Some(&witness) = self.paired_witness.get(&v) {
                        // `OpFreeRefIfDistinct` declines where the two alias, so it is
                        // sound only where somebody ELSE releases the store then: the
                        // witness itself (a local that adopted the buffer and frees at its
                        // own exit), or the CALLER, when the witness — or a binding that
                        // borrows it — is what this return hands out.  A witness that never
                        // frees (a `??` hoist that binds the call for its block) and is not
                        // handed out leaves this free as the store's sole release, so it is
                        // plain: `r = mk(i) ?? d` in a loop held the buffer's last record for
                        // the frame's life once `r` stopped owning what it only borrowed.
                        let handed_out = witness == ret_var
                            || return_sources.iter().any(|&s| {
                                s == witness || function.tp(s).depend().contains(&witness)
                            });
                        if !function.is_skip_free(witness) || handed_out {
                            ls.push(Value::Call(
                                data.def_nr("OpFreeRefIfDistinct"),
                                vec![Value::Var(v), Value::Var(witness)],
                            ));
                        } else {
                            ls.push(call("OpFreeRef", v, data));
                        }
                    } else if is_work_ref
                        && escaping_record_holds_buffer(
                            data,
                            function,
                            self.d_nr,
                            &self.capture_build_backing,
                            v,
                        )
                    {
                        // loft#1446 — a closure record that LEAVES this frame adopted the store
                        // this buffer minted, so the record's cascade is that store's release
                        // and the frame owes nothing for it.
                        //
                        // There is nothing conditional to emit: which store the record took is
                        // settled statically, and `OpFreeRefIfDistinct(buffer, local)` below
                        // cannot express it.  That guard tests the buffer against the LOCAL, so
                        // a local reassigned after the build reads as "distinct" and the free
                        // fires on exactly the store the escaped closure is still reading.
                    } else if is_work_ref
                        && let Some(&witness) = self.literal_buffer.get(&v)
                        && (witness == ret_var
                            || return_sources.contains(&witness)
                            // loft#1439 — the record ADOPTED the capture (so the frame emits no
                            // free for the local) and that record LEAVES the frame (so its
                            // cascade will free what it holds).  Both halves are load-bearing:
                            // without the first, a capture the record only borrows loses its
                            // sole release; without the second, a record left behind dies with
                            // no free of its own — the fn-ref type carries `Deps::frame1` so the
                            // sweep skips it — and its cascade never runs.
                            || (capture_adoption_owns_free(
                                data,
                                function,
                                &self.capture_build_backing,
                                witness,
                            ) && escaping_record_holds(data, function, self.d_nr, witness)))
                    {
                        // loft#1317 — the buffer an inline record literal minted, whose store
                        // the local it was aliased into is now HANDING TO THE CALLER.  This
                        // free is forced (`is_work_ref`) and so ran even though `leaves_frame`
                        // suppressed the local's own: `fn f() -> S? { c: S? = S { x: 5 }; c }`
                        // returned a released store on both backends, right by luck on an
                        // ordinary build and `0xDEADBEEF` under `LOFT_POISON=1`.
                        //
                        // Conditional on the local being a RETURN source, and that condition is
                        // the whole of the rule.  Where the local is NOT returned, this plain
                        // free is the store's only release — the local may be captured (the
                        // record adopted it and the frame emits nothing), or carry no free of
                        // its own at all — and making it conditional strands the store.
                        // Measured both ways: `1181-a-captured-struct-…` leaks a `Circle` and
                        // `810-method-return-buffer` an `M` when this arm fires unconditionally.
                        //
                        // `OpFreeRefIfDistinct` then answers the two return shapes at run time:
                        // the local still names the buffer's store (alias -> decline, the caller
                        // owns it), or it was reassigned since (differ -> free, the literal
                        // store is dead).
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(witness)],
                        ));
                    } else if let Some(buffers) = self.witness_buffer.get(&v).cloned() {
                        // @P378(a) — `v` is an inner-scoped witness whose store
                        // is the outer `__ref_N` buffer (adoption).  Skip the
                        // per-iteration free when they still alias so the
                        // buffer stays reserved across iterations; the buffer's
                        // own function-exit OpFreeRef releases it once.
                        //
                        // The FREE is skipped in the adoption case; the DROP is not.
                        // The store surviving into the next iteration is a reuse
                        // optimisation, and the value it held is over either way.
                        if let Some(hook) = unless_moved(v, self.scope_end_hook(function, v, data, arm_dropped)) {
                            ls.push(hook);
                        }
                        // Several buffers — one per arm of the value branch `v` was bound
                        // from — free only where `v` aliases NONE of them; the single-buffer
                        // case is the one op.
                        if let [buffer] = buffers[..] {
                            ls.push(Value::Call(
                                data.def_nr("OpFreeRefIfDistinct"),
                                vec![Value::Var(v), Value::Var(buffer)],
                            ));
                        } else {
                            let mut free = call("OpFreeRef", v, data);
                            for &buffer in buffers.iter().rev() {
                                free = v_if(
                                    Value::Call(
                                        data.def_nr("OpDistinctStore"),
                                        vec![Value::Var(v), Value::Var(buffer)],
                                    ),
                                    free,
                                    Value::Null,
                                );
                            }
                            ls.push(free);
                        }
                    } else if let Some(w) = borrow_witness {
                        // Free ONLY when the local no longer names what its dep names.
                        if let Some(hook) = unless_moved(v, self.scope_end_hook(function, v, data, arm_dropped)) {
                            ls.push(hook);
                        }
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(w)],
                        ));
                    } else if inject_drop_free() == Some(function.name(v)) {
                        // @PLN94 TEST-ONLY positive control (never set in production; one cached env
                        // read/process): drop the scope-exit free for the NAMED owned var, injecting
                        // a genuine leak. The `check-leak` scan must go RED on it — the true-positive
                        // gate. Mirrors LOFT_NO_A1B / LOFT_STORE_GUARD_INJECT.
                    } else if let Some(c) =
                        join_capture_witness(data, function, &self.capture_build_backing, v)
                    {
                        // loft#1721 — one arm of a join a closure record adopted: released
                        // unless the capture holds this store (`join_capture_witness`).
                        if let Some(hook) = unless_moved(v, self.scope_end_hook(function, v, data, arm_dropped)) {
                            ls.push(hook);
                        }
                        ls.push(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(c)],
                        ));
                    } else if let Some((w, pos)) = reassigned_join_capture_slot(
                        data,
                        self.database,
                        function,
                        self.d_nr,
                        &self.capture_build_backing,
                        v,
                    ) {
                        // loft#1725 — the same release for a capture REASSIGNED after its
                        // build, witnessed by the record's slot rather than the local.  A
                        // record never built (the build sat on a path that did not run) holds
                        // nothing, and the arm is the frame's outright.
                        if let Some(hook) = unless_moved(v, self.scope_end_hook(function, v, data, arm_dropped)) {
                            ls.push(hook);
                        }
                        let held = Value::Call(
                            data.def_nr("OpGetDbRef"),
                            vec![Value::Var(w), Value::Int(i32::from(pos))],
                        );
                        let unless_held = Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), held],
                        );
                        let built = v_if(
                            Value::Call(data.def_nr("OpRefIsNull"), vec![Value::Var(w)]),
                            Value::Boolean(false),
                            Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(w)]),
                        );
                        ls.push(v_if(built, unless_held, call("OpFreeRef", v, data)));
                    } else {
                        // The type's scope-end hook, immediately BEFORE the free that ends
                        // the value's life — unless `v` is a buffer whose witness already
                        // ran it.
                        if !is_buffer
                            && let Some(hook) = unless_moved(v, self.scope_end_hook(function, v, data, arm_dropped))
                        {
                            ls.push(hook);
                        }
                        ls.push(call("OpFreeRef", v, data));
                    }
                }
            }
            // A generator HANDLE owns its coroutine frame, and through it every heap local
            // the generator body allocated.  `Type::Iterator` was absent from the heap block
            // above, so no scope carried a free for one: a generator whose consumer stopped
            // early never reached the tail where its own `OpFreeRef`s live, and the vector it
            // was walking stayed allocated for the rest of the program (loft#835).  Stopping
            // early is ordinary code — iterating until a match is found and breaking is the
            // main reason to reach for a generator at all.
            //
            // Ownership is unconditional here because a handle is never a view of somebody
            // else's store, so `Type::Iterator` carries no dep list to consult.  A handle
            // being RETURNED is already skipped by the `v == ret_var` test at the top of the
            // loop, and a parameter never enters this sweep, so the caller keeps its own.
            // Freeing an already-exhausted handle is safe: the frame carries a generation
            // stamp the free checks, so a stale handle cannot reach a recycled slot.
            //
            // A handle MOVED into a field or an element (loft#1585) is the container's to free.
            // One moved in a branch arm is freed on the paths that did not move it.
            if matches!(function.tp(v).base(), Type::Iterator(_, _))
                && !function.is_skip_free(v)
                && !self.drop_transferred.contains(&v)
            {
                let free = call("OpFreeRef", v, data);
                ls.push(match self.handed_off.get(&v) {
                    Some(&flag) => v_if(Value::Var(flag), Value::Null, free),
                    None => free,
                });
            }
            // free the closure DbRef embedded at offset+4 in a fn-ref slot.
            // The 16-byte fn-ref stack slot is reclaimed by FreeStack, but the closure
            // store record at offset+4 must be explicitly freed via OpFreeRef.
            if let Type::Function(..) = function.tp(v) {
                // fn-ref variables OWN their closure store. The dep list
                // tracks captured variables, not store borrowing. Always
                // emit OpFreeRef unless the fn-ref is the return value.
                // H2 step 5: the declared return's closure-work-var note is
                // a TAGGED callee-frame entry — decode it (a raw `contains`
                // never matches the tagged value, and an explicit
                // `return adder;` reaches here with an empty block-result
                // dep list, so the closure record would be freed under the
                // escaping fn-ref).
                let ret_carries = data.def(self.d_nr).returned.deps_ref().is_some_and(|d| {
                    d.entries()
                        .any(|e| matches!(e, crate::data::DepEntry::CalleeFrame(w) if w == v))
                });
                // …and the fn-ref this return actually DELIVERS, which the declared type's
                // note does not name: it is published once per lambda and OVERWRITTEN, so the
                // last lambda BUILT wins wherever a function makes more than one.  A closure
                // returned through a local, with another built after it, was therefore freed
                // under the escaping fn-ref — and freeing it cascades into everything it
                // captured, which is how the caller read a released record (loft#1444).
                // `return_sources` is the path-local fact this frame already has, and it names
                // the value rather than the build order.
                // …and the fn-ref that carries a record OUT through a `&fn(…)` link, the
                // route loft#1443 opened.  The free below is what TRIGGERS the capture
                // cascade, so emitting it for a fn-ref whose record the caller now holds
                // destroys the closure the caller was just handed — `h = fn() { d.a };
                // out = h;` kept the record (the heap sweep's own link term) and then had
                // it taken by this one.  Asked THROUGH `closure_records_of_source`, the one
                // home `link_written_closure_records` itself reads, so the two cannot
                // disagree about which record a fn-ref holds — restating it here as a dep
                // walk of its own is what let this side read the list in a space the other
                // side never does.
                let mut carried: Vec<u16> = Vec::new();
                closure_records_of_source(data, function, v, &mut carried);
                let link_carries = matches!(function.tp(v), Type::Function(..))
                    && carried
                        .iter()
                        .any(|w| link_delivered.contains(w) && !self.closure_keep.decides_link(*w));
                // A return's dep on the fn-ref says its value was computed FROM it; only a
                // returned fn-ref can carry the closure out.  Asked for a name whose release
                // is decided by store identity (`@FR-L-CapKeep`), where reading `"{g()}"`
                // as a delivery left the record `g` alone holds unreleased.
                let keep_holder = self.closure_keep.gated && self.closure_keep.holders.contains(&v);
                // De Morgan of `!(keep_holder && !is_fn)`, which clippy reads as non-minimal.
                // Written this way round on purpose: the dep test leads, and the exception
                // reads as "unless this is a kept holder that is not itself a function".
                let dep_delivers = tp.depend().contains(&v)
                    && (!keep_holder || matches!(tp.base(), Type::Function(..)));
                let leaves_frame = dep_delivers
                    || ret_carries
                    || link_carries
                    || v == ret_var
                    || return_sources.contains(&v);
                // The free above is what TRIGGERS the capture cascade (see the
                // `captured_ref` note earlier in this function: the frame's own free of a
                // captured cell is suppressed, so the record's cascade is its sole
                // owner).  That makes this fn-ref's scope the lifetime of everything it
                // captured — which is only right while the fn-ref is not scoped TIGHTER
                // than the record it points at.
                //
                // A closure written in a nested block is exactly that: the binding lives
                // in the block, the `___clos_N` record and the `__bx_<n>` cell are frame
                // vars one scope out.  Freeing through the binding at the block's end
                // then cascaded into a cell the frame still reads —
                // `fn f(n) { { b = fn(k) { n = n + k }; b(1); } n + 4 }` read
                // 0xDEADBEEF under LOFT_POISON, and plausible-looking stale bytes
                // without it.  Both backends, every scalar type, and a captured LOCAL as
                // readily as a parameter.
                //
                // The record carries its own scope-exit free (it is a plain OWNS frame
                // var, not a capture, so nothing suppresses it), and that one runs at the
                // right time.  So when the record sits in a different scope, leave the
                // cascade to it.  A fn-ref with no local record — a parameter, a call
                // result — keeps its free: nothing else would ever release it.
                let record_outlives = function.tp(v).depend().iter().any(|&r| {
                    r != v
                        && (r as usize) < function.count() as usize
                        && self.var_scope.get(&r) != self.var_scope.get(&v)
                });
                let emit = !leaves_frame && !function.is_skip_free(v) && !record_outlives;
                if emit {
                    if scope_debug {
                        eprintln!(
                            "[scope_debug] freeing closure of fn-ref '{}' (var={v}, scope={})",
                            function.name(v),
                            self.var_scope.get(&v).copied().unwrap_or(u16::MAX),
                        );
                    }
                    // `@FR-L-CapOwn` — a fn-ref no local record releases (a call result, a
                    // parameter) holds a record that LEFT its frame, and the release it took
                    // over runs the hooks as well as freeing the store.  Which lambda it holds
                    // is a run-time fact, so the cascade is dispatched on its `d_nr`
                    // (loft#1609).  A local record releases through itself and needs no op.
                    let local_record = carried
                        .iter()
                        .any(|&r| r < function.count() && function.name(r).starts_with("___clos_"));
                    // `@FR-L-CapKeep` — in a frame whose names are compared by store, the
                    // fn-ref stands down where a record or another live name holds the same
                    // store, and is otherwise the record's LAST name: its release is the whole
                    // of one, hooks included, whether or not a local record built it.
                    let keep = self.closure_keep.gated && self.closure_keep.holders.contains(&v);
                    if keep {
                        ls.extend(self.closure_keep_stand_down(v, function, data));
                    }
                    if (keep || !local_record) && data.any_closure_drop() {
                        ls.push(call("OpDropFnRef", v, data));
                    }
                    ls.push(call("OpFreeRef", v, data));
                }
            }
        }
        // @P376 follow-up — no const-param unlock emitted anymore (matching
        // the dropped function-entry lock in `parser/expressions.rs`).  See
        // there for the rationale: compile-time const checks already cover
        // every mutation path, and the function-entry lock was a
        // false-positive trigger on iteration over a const-param's hash
        // field.  Par-worker `read_only` clones still enforce immutability
        // independently.
        // scope_debug: also report Reference vars in var_order whose scope is NOT in
        // the current chain — these are "orphaned" vars that should never happen after
        // the A5.6 block-pre-registration fix.
        if scope_debug {
            let chain: HashSet<u16> = {
                let mut s = HashSet::default();
                let mut sc = self.scope;
                let mut pos = self.stack.len();
                loop {
                    if sc == 0 {
                        break;
                    }
                    s.insert(sc);
                    if sc == to_scope {
                        break;
                    }
                    if pos == 0 {
                        break;
                    }
                    pos -= 1;
                    sc = self.stack[pos];
                }
                s
            };
            for &v in &self.var_order {
                if v == ret_var {
                    continue;
                }
                let v_scope = *self.var_scope.get(&v).unwrap_or(&0);
                if v_scope == 0 {
                    continue;
                }
                if !chain.contains(&v_scope)
                    && function.tp(v).is_heap_owned()
                    && !function.is_skip_free(v)
                {
                    eprintln!(
                        "[scope_debug] ORPHANED heap var '{}' (var={v}): \
                         its scope={v_scope} is not in the chain to to_scope={to_scope}",
                        function.name(v),
                    );
                }
            }
        }
        // @PLN87 P2.1 — function exit (`to_scope == 1` is the body scope; every
        // `return` and the tail both free up to it).  For each rebindable heap
        // param, free its CURRENT store iff it differs from the caller-supplied
        // original (witness) captured at entry: a wholesale-reassigned param
        // points at a callee-owned FRESH store (freed here), while an
        // un-reassigned or field-only-mutated param still equals its witness
        // (`OpFreeRefIfDistinct` no-ops, the caller owns + frees it).  The
        // runtime distinctness check makes this sound across conditional and
        // repeated rebinds.  Arguments are deliberately excluded from the normal
        // `variables()` sweep ("never return function arguments"), so this is the
        // sole site that frees a rebound param.  Loop scopes / nested blocks use
        // `to_scope >= 2`, so a `break`/`continue`/block-exit never fires this.
        if to_scope == 1 {
            let free_distinct = data.def_nr("OpFreeRefIfDistinct");
            for (param, orig) in function.rebind_params() {
                ls.push(Value::Call(
                    free_distinct,
                    vec![Value::Var(param), Value::Var(orig)],
                ));
            }
            // `(G-Hold)`, loft#1708 — a generator HOLDS every handle it was handed (its caller
            // took a hold for the frame, `retain_shared_handles`), and gives it back at its end.
            // An abandoned frame gives it back through `free_coroutine` instead.
            if matches!(data.def(self.d_nr).returned.base(), Type::Iterator(_, _)) {
                for param in 0..function.count() {
                    if function.is_argument(param)
                        && matches!(function.tp(param).base(), Type::Iterator(_, _))
                    {
                        ls.push(call("OpFreeRef", param, data));
                    }
                }
            }
        }
        if !guarded.is_empty() {
            guarded.append(&mut ls);
            ls = guarded;
        }
        ls
    }
}
