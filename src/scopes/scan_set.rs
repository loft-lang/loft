// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! One ASSIGNMENT: `scan_set` decides what a `Set` owns, borrows, releases and witnesses.

use super::backings::{
    bind_backing_of, construction_work_refs, delivered_work_ref, member_backing,
};
use super::buffers::tail_calls;
use super::capture_builds::adopted_work_refs;
use super::disturbance::{disturbance_via, report_materialised_view};
use super::drops::drop_hook;
use super::handles::{handle_rhs_kinds, tag_handle_leaves};
use super::handoff::{copy_moves_drop_from, per_path_stops, var_copy_owns};
use super::join_rewrite::branch_tail_vars;
use super::places::keyed_payload_view;
use super::returns::projection_root;
use super::tuple_members::{
    MemberFacts, branch_tuple_call_mints, tuple_call_mints, tuple_owned_elem_frees,
};
use super::witness::{
    WitnessSet, displaces_owned_through_fresh_callee, mints_a_store_the_target_does_not_hold,
    release_witness, witness_points_at, witness_set_kind,
};
use super::{Adopts, BindShape};
use super::{Scopes, call};
use crate::data::{Data, Deps, Type, Value, v_if, v_set};
use crate::fxhash::FxHashMap as HashMap;
use crate::variables::Function;

/// Perform scope analysis on all currently known functions.
/// One scan pass for [`check`]: scan `orig_code`/`orig_vars`, prepend the
/// lift-var null-inits, apply the result to `def`, run the debug ref/leak checks,
/// and set each variable's scope.  Runs once normally; plan-57 cluster I re-runs
/// it with a non-empty `confined` map (`__vdb`/local → block scope) so a confined
/// store registers — and therefore frees — at its block exit (`put_scope`).
/// loft#721 — map each fn-ref VARIABLE to the definition assigned to it.
///
/// A non-capturing lambda is stored as a bare definition number, a capturing one
/// as `FnRef(d_nr, closure)`; both forms are searched.  A variable that receives
/// two different definitions maps to `u32::MAX` (ambiguous), and a caller that
/// cannot name one definition must not lift — see `callref_owned_return`.
/// @PLN130 F2 — the container variable an element/field read ultimately reads OUT of.
///
/// One line, because the derivation is [`crate::use_analysis::projection_container_var`]'s:
/// the EMITTER that copies a materialised view reads the same home
/// (`generation::container_element_base`), and a walk that named a container the emitter
/// cannot is a copy the compiler reports and does not make.
fn base_container_var(value: &Value, data: &Data) -> Option<u16> {
    crate::use_analysis::projection_container_var(data, value)
}

/// The specific keyed collection's store type — what `OpReplaceKeyed` and a keyed
/// `OpNewRecord` dispatch on.  The same registration the parser's `keyed_known_type` makes,
/// which is idempotent for a type the program already uses.
fn keyed_type_id(database: &mut crate::database::Stores, data: &Data, tp: &Type) -> Option<u16> {
    let tp = tp.peel_link();
    let content = match tp {
        Type::Sorted(td, _, _)
        | Type::Hash(td, _, _)
        | Type::Index(td, _, _)
        | Type::Radix(td, _, _)
        | Type::Trie(td, _, _) => data.def(*td).known_type(),
        _ => return None,
    };
    if content == u16::MAX {
        return None;
    }
    Some(match tp {
        Type::Sorted(_, key, _) => database.sorted(content, key),
        Type::Hash(_, key, _) => database.hash(content, key),
        Type::Index(_, key, _) => database.index(content, key),
        Type::Radix(_, key, _) => database.spatial(content, key),
        Type::Trie(_, key, _) => database.trie(content, key),
        _ => return None,
    })
}

/// The backing each VECTOR member of a tuple literal lives in, by member index — read off the
/// member's own block, whose result type names it ([`member_backing`]).  `None` where `rhs` is
/// not a tuple literal.
fn tuple_member_backings_of(rhs: &Value, function: &Function) -> Option<HashMap<u16, u16>> {
    let Value::Tuple(members) = rhs.unspan() else {
        return None;
    };
    let mut out = HashMap::default();
    for (idx, m) in members.iter().enumerate() {
        if let Value::Block(b) = m.unspan()
            && matches!(b.result.base(), Type::Vector(_, _))
            && let Some(backing) = member_backing(function, &b.result)
        {
            out.insert(idx as u16, backing);
        }
    }
    Some(out)
}

impl Scopes<'_> {
    /// Is `v = value` a copy off a PARAMETER written in a branch arm — a copy that stops `v`
    /// itself, with a per-path flag (loft#1515) to record that it ran?  Such a `v` holds a record
    /// of its own (`@FR-B-Copy`) whose release is the caller's on the path that copied and the
    /// release of `v`'s earlier record on the path that did not.  An ownership verdict read off
    /// the parameter calls the copy a view, which cannot say both, so the displaced release and
    /// the ownership memo read this copy as owned and leave the release to the flag.
    fn copy_flagged_on_target(
        &self,
        function: &Function,
        data: &Data,
        v: u16,
        value: &Value,
    ) -> bool {
        matches!(value.unspan(), Value::Var(src)
            if self.per_path_pairs.contains(&(v, *src))
                && per_path_stops(function, data, v, *src) == Some(v)
                && self.handed_off.contains_key(&v))
    }

    /// Is `v = value` a copy off a PARAMETER into a local the loft#1336 owner witness releases?
    /// The copy is a store of the local's own (`@FR-B-Copy`), so the witness names it, but its
    /// resource is the caller's (`(H-Drop)`), so the witness must not hook it.  The local's
    /// `__hoff_` flag records which of the two the witnessed record is.
    fn witnessed_copy_stops_itself(
        &self,
        function: &Function,
        data: &Data,
        v: u16,
        value: &Value,
    ) -> bool {
        self.owner_witness.contains_key(&v)
            && self.handed_off.contains_key(&v)
            && matches!(value.unspan(), Value::Var(src)
                if per_path_stops(function, data, v, *src) == Some(v))
    }

    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn scan_set(
        &mut self,
        ov: u16,
        value: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Value {
        assert_ne!(
            ov,
            u16::MAX,
            "Incorrect variable in {} fn {}",
            function.file,
            function.name
        );
        if let Some(s) = self.var_scope.get(&ov)
            && self.scope != *s
            && !self.stack.contains(s)
        {
            if crate::env_once!(std::env::var("LOFT_LOG").as_deref() == Ok("scope_debug")) {
                eprintln!(
                    "[scope_debug] copy trigger: var={ov} name='{}' \
                     registered_scope={s} current_scope={} stack={:?} value={value:?}",
                    function.name(ov),
                    self.scope,
                    self.stack,
                );
            }
            if let Some(&existing_copy) = self.var_mapping.get(&ov) {
                // Replace the mapping only if the existing copy's scope has exited.
                if let Some(&copy_scope) = self.var_scope.get(&existing_copy)
                    && copy_scope != self.scope
                    && !self.stack.contains(&copy_scope)
                {
                    self.var_mapping.insert(ov, function.copy_variable(ov));
                }
            } else {
                self.var_mapping.insert(ov, function.copy_variable(ov));
            }
        }
        let v = *self.var_mapping.get(&ov).unwrap_or(&ov);
        // A handle view given a member's handle on one arm and one of its own on another
        // (loft#1585) records which in the arm itself: after the `Set` nothing tells them apart.
        let tagged;
        let value = if self.handle_views.contains(&v)
            && let Some(&flag) = self.handed_off.get(&v)
            && handle_rhs_kinds(value, data) == (true, true)
        {
            tagged = tag_handle_leaves(value, flag, data);
            &tagged
        } else {
            value
        };
        // A scope copy of a witnessed local (`copy_variable` above) is the same binding under
        // a new id: it keeps the witness and the never-free mark, or its own Sets would go
        // back to the static frees the witness replaced.
        if v != ov
            && let Some(&w) = self.owner_witness.get(&ov)
            && !self.owner_witness.contains_key(&v)
        {
            function.set_skip_free(v);
            function.set_owner_witness(v, w);
            self.owner_witness.insert(v, w);
        }
        // `@FR-O-Complete` / `@FR-B-Copy` — a REASSIGNMENT of a heap local from a value
        // branch is lowered to the statement form, `if c { x = a } else { x = b }`, so each
        // arm's `Set` gets the lowering a single bind of that tail has: a whole variable is
        // COPIED, a projection views, a call is copied or adopted by its own arm.  Bound as
        // one value, the branch handed the local the chosen arm's STORE: `x = if c { a }
        // else { b }` on an owned `x` aliased `a` (a write through `x` reached it) and then
        // freed it as its own at scope exit.  The FIRST bind keeps its per-arm lift
        // (`lift_join_arm_tails`), whose temps the binding borrows — a binding assigned
        // elsewhere cannot borrow them (`@FR-O-Latest`), which is exactly why the
        // reassignment is written out per arm instead.
        //
        // Except a first bind of a type that owns a DROPPABLE (`formal/heap.md` (H-Move)): the
        // lift keeps each arm's release with the arm's SOURCE and makes the binding a borrow, so
        // the binding cannot hand the value on — `y = x`, `S { h: x }`, `v += [x]` and `return x`
        // each stopped a binding that released nothing while the source still did, and one
        // resource was released twice.  Written out, the binding OWNS what its arm moved into it,
        // as the author's own `if c { x = a } else { x = b }` does.  Not where an arm's source
        // outlives the loop the bind runs in: that places one name on every iteration, which
        // `(H-Spent)` refuses, and until that error exists the lift's keep-with-the-source is the
        // answer that releases once ([`Self::arm_source_outlives_loop`]).  Arms that hand back a compiler temp
        // (a `??` hoist, a literal's work-ref) keep the value form: the join they express is a
        // runtime fact (`Own::Join`), not a copy — except a call arm the parser gave an owner for
        // the value form, which is written out as the call itself, because that owner served the
        // join.  RECORDS only: a vector keeps the value form, which already copies the chosen arm.
        let writes_out = (self.var_scope.contains_key(&v)
            || (data.type_owns_droppable_anywhere(function.tp(v))
                && !self.arm_source_outlives_loop(value, function)))
            && Self::is_value_branch(value)
            && !matches!(function.tp(v), Type::RefVar(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            );
        // Recorded by the address of the value node, the node inside the code this scan was
        // handed, for `rewrite_written_out` — with every arm shape the rewrite accepts, a
        // projection arm beside an owned call included: the rescan's analyses see that
        // projection's assignment, which this scan's could not.
        if writes_out && Self::sink_set_into_arms(v, ov, value, function, data, true).is_some() {
            self.written_out
                .push((std::ptr::from_ref(value).addr(), ov));
        }
        if writes_out
            && let Some(sunk) = Self::sink_set_into_arms(v, ov, value, function, data, false)
        {
            // The written-out arms are copies the author could have spelled, so they get what
            // that spelling gets, BEFORE the first arm is scanned: a per-path flag for each source
            // a copy hands off (loft#1515 — minted here, because the pre-scan pass saw one
            // `Set(v, <branch>)` and no copies), and a binding whose type no longer names those
            // sources, because the first arm reads that type to decide whether it owns the record
            // it displaces.
            for src in branch_tail_vars(value) {
                // The identity arm hands nothing over, and the binding does not view itself.
                if src == v || src == ov {
                    function.make_independent(v, src);
                    continue;
                }
                let Some(stopped) = per_path_stops(function, data, v, src) else {
                    continue;
                };
                self.per_path_pairs.insert((ov, src));
                self.per_path_pairs.insert((v, src));
                if stopped == src {
                    self.mint_handoff_flag(function, src);
                    if var_copy_owns(function, data, v, src) {
                        function.make_independent(v, src);
                    }
                } else {
                    // A parameter arm stops the binding itself, so the flag is the binding's —
                    // under both of its ids, the one the pre-scan saw and the one this scan uses.
                    let flag = self.mint_handoff_flag(function, ov);
                    self.handed_off.insert(v, flag);
                }
            }
            return self.scan(&sunk, function, data);
        }
        // #316 — capture BEFORE put_scope below: an ownership-transition free
        // only applies to a REassignment.
        let was_in_scope = self.var_scope.contains_key(&v);
        // Read before the retirement below clears it: a generator handle moved into a
        // container (loft#1585) is the container's, and its rebind must not free it.
        let handle_moved = self.drop_transferred.contains(&v);
        // The record this reassignment DISPLACES is released through its hook before the
        // new value lands — read here, while `owned_refs` still describes the previous
        // assignment.
        let displaced = if was_in_scope && *value != Value::Null {
            let rhs_owned = matches!(self.ref_rhs_ownership(value, data), RefRhs::Owned)
                || self.copy_flagged_on_target(function, data, v, value);
            self.displaced_drop(v, rhs_owned, function, data)
        } else {
            None
        };
        // An UNCONDITIONAL reassignment retires the hand-off of the record it displaces:
        // what `v` holds from here on is its own to release again.  A reassignment inside a
        // deeper scope (one arm of a branch, a loop body) is not certain to run, so the
        // hand-off stays — the leak direction, never a second release.
        //
        // Scope depth only stands in for "certain to run", and an arm lift temp is where the
        // stand-in fails: its Set IS the per-path copy its hand-off was recorded for
        // (`arm_lift_temps`, `@FR-O-Complete`), and a `??` arm is a bare `Insert` that opens
        // no scope of its own, so that Set arrives at the temp's own scope depth.
        if was_in_scope
            && *value != Value::Null
            && self.var_scope.get(&v) == Some(&self.scope)
            && !self.arm_lift_temps.contains(&v)
        {
            self.drop_transferred.remove(&v);
        }
        // A redundant re-init `Set(v, Null)` for an already-in-scope var is
        // elided (Reference/Vector/Enum/Text locals don't need re-null-ing).
        // EXCEPTION (@P302): keyed collections — `s = []` lowers to
        // `Set(s, Null)`, which on a reassignment is a genuine CLEAR (codegen
        // emits an in-place `OpDatabase`).  Eliding it left the old contents
        // intact (silent no-op) and leaked `s`'s store.  Let keyed Set-Null
        // through so codegen's keyed reassign arm clears in place.
        if self.var_scope.contains_key(&v)
            && *value == Value::Null
            && !crate::parser::vectors::is_keyed(function.tp(v))
        {
            return Value::Insert(Vec::new());
        }
        // #316 — ownership-transition free.  When this var's latest scanned
        // assignment gave it an OWNED store and this reassignment installs a
        // BORROW, the merged static type already carries deps, so codegen's
        // dep-empty pre-Set free never fires and the owned store is orphaned
        // (`chosen = m_none(); chosen = pool[i] ?? m_none()` leaked one store
        // per call).  Emit the free here, in the IR, before the new value
        // lands.  Depth guard: only at the loop depth that owned the store —
        // inside a deeper loop the free would re-run on iterations 2+ and
        // release the previous iteration's VIEWED store.
        let mut transition_free: Option<Value> = None;
        // The same transition for a TUPLE: reassigning a tuple local installs new
        // elements over the old ones, and without this the previous turn's owned
        // element stores are named by nobody.  Invisible until a call-site return
        // buffer stopped being pre-allocated by the caller (loft#1085): before that
        // the callee ADOPTED the caller's `__ref_N` store, so the caller's own
        // scope-exit free covered every iteration's element and one store served
        // the whole loop.  A first-iteration free is a no-op — the entry null-init
        // leaves the elements at the sentinel, which `free` ignores.
        if was_in_scope
            && let Type::Tuple(elems) = function.tp(v)
            && !value.reads_var(v)
            && !value.reads_var(ov)
        {
            let elems = elems.clone();
            let mut frees = tuple_owned_elem_frees(
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
            );
            frees.extend(self.tuple_handle_frees(v, function, data));
            // What the tuple holds after this rebind is its own again.
            let mut resets: Vec<(u16, u16)> = self
                .tuple_moved
                .iter()
                .filter(|((t, _), _)| *t == v)
                .map(|(&(_, i), &f)| (i, f))
                .collect();
            resets.sort_unstable();
            frees.extend(
                resets
                    .into_iter()
                    .map(|(_, f)| v_set(f, Value::Boolean(false))),
            );
            if !frees.is_empty() {
                transition_free = Some(Value::Insert(frees));
            }
        }
        // loft#1601, @FR-G-Hold — a keyed collection rebound releases the generator frames its
        // records hold before they are cleared, through its type's walk; the records run no
        // hook (`(H-Drop-Not)`).
        if transition_free.is_none()
            && was_in_scope
            && data.keyed_holds_generator(function.tp(v))
            && !value.reads_var(v)
            && !value.reads_var(ov)
            // Its own store, not a view of another's: a keyed local lists itself.
            && function.tp(v).depend().iter().all(|&d| d == v || d == ov)
        {
            let walk = data.def_nr(&data.keyed_frames_name(function.tp(v)));
            if walk != u32::MAX {
                let live = Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(v)]);
                transition_free = Some(v_if(
                    live,
                    Value::Call(walk, vec![Value::Var(v)]),
                    Value::Null,
                ));
            }
        }
        // A generator HANDLE owns its frame (@FR-G-Hold), so reassigning one releases the frame it held
        // — as its scope end would have (loft#835).  Nothing did: `g = steps(1); g =
        // steps(5)` kept the first frame, and every heap local it owned, to program exit.
        // Not for a view of a member (`skip_free`), nor for a handle moved into a container
        // before this line, nor where the right-hand side reads the handle it replaces.  One
        // moved in a branch arm, or a view (`handle_views`), is released on the paths where
        // its flag says it owns.
        if transition_free.is_none()
            && was_in_scope
            && matches!(function.tp(v).base(), Type::Iterator(_, _))
            && !function.is_skip_free(v)
            && !handle_moved
            && !value.reads_var(v)
            && !value.reads_var(ov)
        {
            let free = call("OpFreeRef", v, data);
            transition_free = Some(match self.handed_off.get(&v) {
                Some(&flag) => v_if(Value::Var(flag), Value::Null, free),
                None => free,
            });
        }
        // loft#1126 / @FR-O-Latest — the ownership-transition free for the OTHER
        // reassignment shape: `v = f(…, v, …)` with `v` at `f`'s hidden
        // return-buffer position, where `f` mints a store of its own and never
        // delivers into the buffer.  The hand-off is not taken, so `v`'s current
        // store is displaced with nothing left naming it.
        //
        // Codegen cannot decide this one.  `v` is the function's hidden
        // return-buffer PARAMETER, so `state/codegen.rs`'s `is_hidden_buf_arg`
        // reads "argument → the CALLER owns that store → never free" — the
        // BINDING-level reading.  @FR-O-Latest says ownership is a property of the
        // LATEST ASSIGNMENT: the caller's store is gone the moment this function
        // assigns `v`, and what `v` holds from then on is this function's to
        // release.  `owned_refs` is that fact (@FR-O-Oracle memoised per path and
        // per loop depth), and it lives here, so the free is emitted here.
        //
        // `--native` reaches the same verdict at runtime through its entry-buffer
        // witness (`_rb_w_<name>`, `generation/mod.rs`), which is why only the
        // interpreter reported the leak — an @FR-O-NoDiverge asymmetry, with the
        // interpreter on the deviating side.  Its own guarded free is a no-op after
        // this one: the `OpFreeRef` emitter resets a freed Var to the null
        // sentinel, so the store it captures is already NULL.
        //
        // `@FR-O-Buffer` — NOT where the buffer has an ENTRY WITNESS.  Then the rebind's own
        // free (`generate_set`, native's `_rb_w_` twin) releases the displaced store AFTER the
        // call, declining on the entry store and on a store the callee filled in place, and
        // it is the one home for it.  A free here runs BEFORE the call that is handed `v` as
        // its buffer: a callee that fills a live buffer then filled the freed store on the
        // interpreter, which does not reset `v`, and the caller read whatever took that slot
        // next (loft#1703).  It was also unguarded, so a caller-handed store died with it.
        if transition_free.is_none()
            && was_in_scope
            && matches!(function.tp(v), Type::Reference(_, _) | Type::Enum(_, true, _))
            // @FR-O-Proxy asks free — the ownership-TRANSITION free, releasing the store `v`
            // is about to stop naming.  The proxy is unsound alone, so it is asked with its
            // @FR-O-Override veto as one question and never separately.
            && function.proxy_says_owned(v)
            && self.owned_refs.get(&v) == Some(&self.loops.len())
            && displaces_owned_through_fresh_callee(value, v, ov, data)
            && function.entry_witness(v).is_none()
        {
            transition_free = Some(call("OpFreeRef", v, data));
        }
        // A `??` hoist that OWNS its subject (a record from a call, `parser/operators.rs`)
        // is a function-scoped work-ref re-bound on every pass of a loop, and the store it
        // displaces is released HERE, in the IR, rather than left to codegen's pre-Set free:
        // the interpreter emits that free for a dep-empty owned Reference and `--native` does
        // not for a fn-ref re-bind, so without this op the displaced stores were held to
        // frame exit on one backend only (`@FR-O-NoDiverge`).  The op is a no-op on the first
        // pass (the slot holds the sentinel) and the interpreter's own pre-Set free then finds
        // the slot already reset.
        if transition_free.is_none()
            && was_in_scope
            && function.name(v).starts_with("__ncc_")
            // @FR-O-Override, consulted first: a hoist the parser marked never-free (a
            // projection subject) releases nothing here.
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            // @FR-O-Proxy asks free — the ownership-TRANSITION free of the store the hoist
            // is about to stop naming, with its @FR-O-Override veto as one question.
            && function.proxy_says_owned(v)
        {
            transition_free = Some(call("OpFreeRef", v, data));
        }
        let mut witness_snapshot: Option<Value> = None;
        // loft#1128 — the PATH-SENSITIVE half of the same fact.  `owned_refs` above is
        // intersect-merged at every join (@FR-O-Complete), so an assignment inside ONE `if`
        // arm answers "not owned on every path" and the branch above emits nothing: sound,
        // and incomplete — the store that assignment minted is displaced here with nothing
        // naming it, one per call.  The runtime witness carries the same @FR-O-Latest fact
        // per RUN, so the free is emitted GUARDED instead of not at all.  `--native` has
        // reached this answer all along through its entry-buffer witness `_rb_w_<name>`,
        // which is why only the interpreter reported the leak.  Stands down under an entry
        // witness for the reason the free above does.
        if transition_free.is_none()
            && was_in_scope
            && let Some((buf, flag)) = self.rbuf_witness
            && buf == v
            && matches!(
                function.tp(v),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            // @FR-O-Proxy asks free — the same transition free, emitted GUARDED on the
            // runtime witness where the static fact is sound but incomplete.
            && function.proxy_says_owned(v)
            && displaces_owned_through_fresh_callee(value, v, ov, data)
            && function.entry_witness(v).is_none()
        {
            transition_free = Some(v_if(
                Value::Var(flag),
                call("OpFreeRef", v, data),
                Value::Null,
            ));
        }
        // loft#1200 — the displaced-store free for a nullable heap-record local, GUARDED by
        // the per-local witness.  See `nullable_locals_that_displace` for why the guard is a
        // runtime flag and not a predicate: the local's first store is shared with a work-ref
        // that frees it too, and no static site separates that iteration from the rest.
        // Keyed by the ORIGINAL var `ov`, which is what built the map: the flags are minted
        // from `orig_code` before the scan, while a second local of the same NAME in a sibling
        // scope is split into a var of its own by the time it reaches here (`v=19 ov=3`).
        // Looking it up by `v` found nothing for that half, so its rebinds got no guarded free
        // at all and the displaced store fell to whatever other site was left (loft#1522).  The
        // two locals never overlap in time — sibling scopes — so the one flag answers for
        // whichever is live, which is exactly what @FR-O-Latest says it records.
        if transition_free.is_none()
            && was_in_scope
            && let Some(&flag) = self.local_owns.get(&ov)
            && mints_a_store_the_target_does_not_hold(value, v, ov, data)
        {
            transition_free = Some(v_if(
                Value::Var(flag),
                call("OpFreeRef", v, data),
                Value::Null,
            ));
        }
        // A record ENUM is the second spelling of a struct-like heap store, and this
        // transition — and the `owned_refs` tracking below that licenses it — reads the
        // same @FR-O-Latest fact for both.  The two blocks above already pair the
        // spellings; these two did not, so a record-enum local that OWNED a store and is
        // then assigned a VIEW never freed what it displaced (loft#1202).
        if was_in_scope
            && matches!(
                function.tp(v),
                Type::Reference(_, d) | Type::Enum(_, true, d) if !d.is_empty()
            )
            // @FR-O-Proxy asks free — @FR-O-Override applies here as at every other free
            // site: a witnessed local (loft#1336) releases through its witness only.
            && !function.is_skip_free(v)
            && self.owned_refs.get(&v) == Some(&self.loops.len())
            && matches!(
                self.ref_rhs_ownership(value, data),
                RefRhs::View
            )
            // `value` is pre-scan IR: reads may name the original id (`ov`)
            // or the remapped one (`v`) — guard against both.  A self-
            // reading borrow (`x = x.next`, #328) keeps its owned store
            // until scope exit — a bounded, documented residual of this
            // conservatism (LIFETIME.md § Ownership-transition free).
            && !value.reads_var(v)
            && !value.reads_var(ov)
        {
            // loft#1510 / D-heap-4 — this releases a store the frame minted, so where the
            // mint is a CONSTRUCTION whose backing work-ref is known, the type's cascade runs
            // first and the work-ref is disarmed: the hand-off to this view-typed binding was
            // vetoed (`drop_handoff_node`), so the work-ref still owns the drop, and without
            // the sentinel its scope-end cascade would run on the store freed here.  The
            // sentinel also makes a loop's next `OpDatabase` mint fresh instead of reusing
            // the freed slot.  A backing this scan could not track (a join dropped it) keeps
            // the bare free — losing the hook, never doubling it.
            transition_free = Some(match self.construction_backing.get(&v) {
                Some(&w) => {
                    let mut ops = Vec::with_capacity(3);
                    if let Some(hook) = drop_hook(function, v, data) {
                        ops.push(hook);
                    }
                    ops.push(call("OpFreeRef", v, data));
                    ops.push(v_set(
                        w,
                        Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                    ));
                    Value::Insert(ops)
                }
                None => call("OpFreeRef", v, data),
            });
        }
        // Track the LATEST assignment's ownership for this var.  Through `base()`: a nullable
        // record local holds the same record behind a nullability marker (`@FR-L-Null`), and
        // the memo is what [`Self::displaced_drop`] reads for it; the transition free above
        // keeps its own bare test and is unchanged by the wider memo.
        if matches!(
            function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) {
            // A copy off a parameter whose per-path flag decides its release is recorded as
            // owned: `v` holds a copy of its own, and whether that record's release is `v`'s is
            // the flag's to answer, per path.  Recorded as a view, the arm that copied and the
            // arm that did not disagreed at the join, so a later rebind released neither
            // (heap-history.md `D-heap-7`).
            if self.copy_flagged_on_target(function, data, v, value)
                || matches!(self.ref_rhs_ownership(value, data), RefRhs::Owned)
            {
                self.owned_refs.insert(v, self.loops.len());
            } else {
                self.owned_refs.remove(&v);
            }
        }
        // loft#1510 / D-heap-4 — remember which work-ref backs the record a CONSTRUCTION
        // just delivered to a binding that will not run its scope-end drop (the hand-off is
        // vetoed for it, see `drop_handoff_node`).  The owned→view transition free reads
        // this to run the cascade and disarm the work-ref; any other assignment retires the
        // pairing (@FR-O-Latest — the fact belongs to the latest assignment).
        //
        // loft#1513 — where the hand-off IS taken (the binding owns, so its scope-end
        // drop+free covers the record — the same predicate as `drop_handoff_node`'s Set arm,
        // negated, so the two cannot drift), the work-ref must also stop NAMING the store:
        // its scope-end `OpFreeRef` still claimed it, and whenever the binding is declared
        // BEFORE the work-ref (a rebind, `r = CfH{…}; r = mk().h`) that bare free ran before
        // the binding's hook, which then read freed memory.  The sentinel makes the binding
        // the store's ONE claimant on this path; a path that skips this Set leaves the
        // work-ref holding its record, and its own scope-end free still covers that.
        //
        // Only where the binding's type carries a HOOK (`drop_hook`): the disarm exists to
        // keep that hook off freed memory, and for a hookless type the double claim is a
        // benign no-op free — while the sentinel is not free there: it defeats the work-ref's
        // in-place reuse (`OpFreeRefIfDistinct` reads "distinct" against a sentinel and
        // frees, so a comprehension minted per PASS instead of rebuilding one store —
        // `value_struct_alloc`'s O(1) promise measured it at N cycles).
        let mut handoff_disarm: Vec<Value> = Vec::new();
        // A handle view's flag says what it holds NOW (`@FR-O-Latest`): a view of a member's
        // handle, or a frame of its own.  Every other flagged variable is retired to `false`
        // at its assignment, further down.
        if self.handle_views.contains(&v)
            && let Some(&flag) = self.handed_off.get(&v)
            && let (view, own) = handle_rhs_kinds(value, data)
            && view != own
        {
            handoff_disarm.push(v_set(flag, Value::Boolean(view)));
        }
        match delivered_work_ref(value, function, data) {
            Some(w) if w != v && !function.proxy_says_owned(v) => {
                self.construction_backing.insert(v, w);
            }
            Some(w) if w != v => {
                // …and where the construction READS the binding: the reuse rebuilds the
                // work-ref's store in place on the next pass, and after this `Set` that store
                // IS the binding's, so a literal computed from the binding would read the
                // record its own re-init just cleared (`(E-Asgn)`: `s = S { n: s.n + 1 }.f()`
                // in a loop answered 1 for 7).  Only a self-reading construction gives the
                // reuse up.
                if drop_hook(function, v, data).is_some()
                    || value.reads_var(v)
                    || value.reads_var(ov)
                {
                    handoff_disarm.push(v_set(
                        w,
                        Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                    ));
                }
                self.construction_backing.remove(&v);
            }
            _ => {
                self.construction_backing.remove(&v);
            }
        }
        // D-heap-3 (loft#1506) — remember the member a view-bound local NAMES, so a
        // `return e` can hand its release to the copy it publishes.  Retired from BOTH
        // ends: writing `v` replaces what it views, and rebuilding a BASE leaves every
        // view of it naming a record that no longer holds what the path said.
        self.view_backing.remove(&v);
        self.view_backing.retain(|_, (base, _)| *base != v);
        if let Some((base, path)) = projection_root(value, data.def_nr("OpGetField"))
            && base != v
            && !function.is_argument(base)
        {
            self.view_backing.insert(v, (base, path));
        }
        // loft#1588 — which backing each vector member of this value lives in, where the
        // assignment runs in the variable's own scope (a first binding registers it there).
        if matches!(function.tp(v).base(), Type::Tuple(_)) {
            match tuple_member_backings_of(value, function) {
                Some(now) if self.var_scope.get(&v).is_none_or(|s| *s == self.scope) => {
                    self.tuple_member_now.insert(v, now);
                }
                _ => {
                    self.tuple_member_now.remove(&v);
                }
            }
        }
        // loft#1511 — remember which elements of a tuple-literal RHS were minted by their
        // own call, for the element frees at reassignment and scope exit.
        if matches!(function.tp(v), Type::Tuple(_)) {
            let joined = if matches!(value.unspan(), Value::If(..)) {
                branch_tuple_call_mints(value, function, data, data.def_nr("OpNullRefSentinel"))
            } else {
                tuple_call_mints(value, function, data).map(|m| (m, Vec::new()))
            };
            if let Some((_, disarms)) = &joined {
                handoff_disarm.extend(disarms.iter().cloned());
            }
            match joined.map(|(m, _)| m) {
                Some(mut m) => {
                    // loft#1532 — an element a later member assignment writes is handed to the
                    // tuple ALONE here: its claimant is disarmed after the `Set`, with the other
                    // hand-off disarms, and it is paired as a sole owner.  See
                    // `written_tuple_members`.
                    if let Type::Tuple(elems) = function.tp(v).base() {
                        let elems = elems.clone();
                        let mut idxs: Vec<u16> = m.keys().copied().collect();
                        idxs.sort_unstable();
                        for idx in idxs {
                            let owned = !tuple_owned_elem_frees(
                                &elems,
                                v,
                                data,
                                function,
                                MemberFacts::NONE,
                                Some(idx as usize),
                            )
                            .is_empty();
                            if owned
                                && self.written_tuple_members.contains(&(ov, idx))
                                && let Some(claim) = m.get_mut(&idx)
                                && let Some(w) = claim.take()
                            {
                                handoff_disarm.push(v_set(
                                    w,
                                    Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                                ));
                            }
                        }
                    }
                    // The members this statement minted are the tuple's own: a copy that took an
                    // EARLIER member's release does not reach them.  Retired only where the
                    // refill is certain to run, as a record's hand-off is (above).
                    if self.var_scope.get(&v) == Some(&self.scope) {
                        for b in m.values().flatten() {
                            self.drop_transferred.remove(b);
                        }
                    }
                    self.tuple_call_mint.insert(v, m);
                }
                None => {
                    self.tuple_call_mint.remove(&v);
                }
            }
        }
        // remember the scope of the variable
        let mut depend = Vec::new();
        for d in function.tp(v).depend() {
            // Skip deps that reference variables from another function's scope
            // (e.g., closure work vars embedded in a fn-ref return type).
            if d >= function.count() {
                continue;
            }
            // loft#1135 — a LOOP VARIABLE reserves its own slot, in its own scope.
            //
            // This prefix exists for a dep whose only other mention is its FREE: a lift var
            // assigned inside a conditional arm needs its slot reserved along every path, or
            // the path that skips the arm reads an uninitialised one.  A `for` loop's
            // variable has no such gap — its header assigns it unconditionally, every
            // iteration, inside the loop.
            //
            // `e`'s type carries ONE dep list while `e` may be assigned in two disjoint
            // scopes (`Function::depend` replaces rather than accumulates), so the surviving
            // dep is whichever assignment parsed LAST.  Two `for` loops over the same keyed
            // type give the FIRST loop's `e` a dep on the SECOND loop's collection, and
            // pre-initialising it here put a `Set(c#1, Null)` — and `var_scope[c#1]` — inside
            // the first loop's body.  The second loop then read its own variable as
            // out-of-scope and copied it, leaving the original a keyed local nothing writes
            // and nothing reads: a store-backed slot, allocated by that init and freed by
            // nobody.  One orphan per program, on `--interpret`.
            //
            // The dep list being wrong is the defect above this one; not reserving a slot for
            // a loop variable is right regardless of which dep names it.
            //
            // ⚠ The predicate is `was_loop_var` and NOT "assigned somewhere in the body",
            // which is the first thing to reach for and is wrong: a lift var IS assigned, in
            // a conditional arm, which is exactly the case the prefix is for.  Measured —
            // `890-consumed-lift-double-free.loft` returned a garbage record under
            // `LOFT_POISON`.
            if !self.var_scope.contains_key(&d) && !function.was_loop_var(d) {
                depend.push(d);
                self.put_scope(d);
                self.var_order.push(d);
            }
        }
        let first_binding = !self.var_scope.contains_key(&v);
        if first_binding && matches!(function.tp(v).base(), Type::Tuple(_)) {
            self.tuple_depth.insert(v, self.loops.len());
        }
        if first_binding {
            self.register_binding(v, function);
        }
        if matches!(function.tp(v).base(), Type::Vector(_, _)) {
            match bind_backing_of(value, function, data) {
                Some(b) => {
                    self.bind_backing.insert(v, b);
                }
                None => {
                    self.bind_backing.remove(&v);
                }
            }
        }
        // When a Reference variable is assigned from a user-function call,
        // codegen has two sub-paths (state/codegen.rs gen_set_first_at_tos /
        // gen_set_first_ref_call_copy), keyed on the SAME carried adopt-vs-copy
        // fact `Definition::return_adopts_fresh_store()` (Cluster A.3,
        // OWNERSHIP_MODEL row 102):
        // - adopts_fresh_store == false → the return is tied to a passed
        //   buffer/param (a visible arg it aliases, or a hidden ref_return
        //   work-ref the caller reuses); gen_set_first_ref_call_copy deep-copies
        //   into a FRESH store `v` owns.
        // - adopts_fresh_store == true → the return is genuinely fresh (empty
        //   dep or the `["??"]` one-buffer marker); `v` adopts the callee's
        //   store (the callee's `__ref_N` store IS the returned struct's store),
        //   OR the callee minted a different fresh store and the caller's
        //   `__ref_N` pre-alloc is orphaned.
        // This reads the precise carried adopt-vs-copy fact rather than the
        // coarse "callee has any visible ref param" proxy `has_ref_params` the
        // 11 sites used to re-derive (A.3): a callee with a ref param that
        // returns a *fresh* store (`fn mk_from(seed) -> Box { Box { v: [...] } }`)
        // now adopts instead of wastefully deep-copying, while a hidden
        // work-ref return (`fn render(p) -> Canvas { cv = …; cv }`, dep
        // `["cv"]`) still copies — the coarse proxy lumped both as "copy".
        // P198 — most operators are wrapped in Value::Span by the parser
        // for diagnostics.  Unwrap before pattern-matching so the
        // deep-copy / make_independent logic fires for Span(Call(...))
        // assignments — without this, OpFreeRef is never emitted for the
        // freshly-allocated store and Database N leaks at scope exit
        // (e.g. tests/scripts/95-alias-copy.loft Database 3 leak).
        let unspanned_value = value.unspan();
        // loft#759 — a `&` parameter is `RefVar(Reference|Enum)`, so the bare
        // `matches!` above read it as "not a record" and skipped this whole
        // block.  Peel the `&` for the record-KIND question (the loft#753 /
        // loft#740 shape: one question, one peel) and carry the `&`-ness
        // separately, because the two halves below want opposite answers:
        //
        // - the deep-copy dep-strips are LOCAL-only.  They exist because
        //   `gen_set_first_ref_call_copy` copies the callee's buffer into a
        //   fresh store `v` owns — but a set THROUGH a `&` never reaches that
        //   path (codegen writes the returned DbRef straight into the caller's
        //   slot with `SetStackRef`), and `v` is a parameter the caller owns,
        //   so stripping its deps would only make the callee free it.
        // - the witness pairing is exactly what the `&` case needs, and needs
        //   it whatever `adopts_fresh_store` says.  With no copy in between,
        //   the buffer the callee filled IS what the caller now holds, so the
        //   scope-exit `OpFreeRef(__ref_N)` freed the record the caller went
        //   on reading and writing through.
        let publishes_through_ref = matches!(function.tp(v), Type::RefVar(_));
        // Asked BARE on purpose, not through `base()`: the two strips below make `v` an
        // OWNER, which is right only where a copy is emitted, and a `-> S?` callee's
        // delivery does not yet materialise what a `-> S` one does — a capture, a parameter's
        // element, a witnessed local's view are handed up raw (loft#1337's selector copies
        // only what this frame frees).  Peeled, a nullable local bound from such a call
        // became an owner of a store it only viewed and freed it (the loft#1181 capture).  The
        // nullable spelling copies through the join guard instead (`nullable_join_first_bind`,
        // whose own strip sits below), which copies exactly the borrow it can witness.
        let mut record_target = crate::use_analysis::first_bind_shape(function.tp(v));
        while let Type::RefVar(inner) = record_target {
            record_target = inner.base();
        }
        // The WITNESS PAIRING below asks *did the callee adopt the buffer I handed it, or
        // mint its own?*, and that question is the same for a vector: a `vector<T>` is
        // delivered through a `__ref_N` exactly as a record is, and its per-iteration free
        // has the same two cases to tell apart.  Without it a vector local bound from such
        // a call got a PLAIN `OpFreeRef`, which in the adoption case releases the caller's
        // own buffer — reused every iteration, so the next one wrote into a freed store
        // (loft#1201; the record spelling beside it was already correct).
        //
        // The two dep-STRIPS in this block do NOT generalise and keep the record-shaped
        // test they had: both exist for `gen_set_first_ref_call_copy`, the Reference-only
        // deep-copy path, and a vector never reaches it.
        let record_shaped = matches!(
            record_target,
            Type::Reference(_, _) | Type::Enum(_, true, _)
        );
        let vector_shaped = matches!(record_target, Type::Vector(_, _));
        // loft#1245 — BOTH spellings, because this decision and codegen's copy-or-adopt
        // one have to name the SAME set of callees (see below), and codegen now reaches a
        // `CallRef`.  While this read `Value::Call` alone the two disagreed for a fn-ref
        // bind: codegen deep-copied into a store `v` owns and the deps stayed, so
        // `get_free_vars` emitted no `OpFreeRef` and every copy leaked.
        // @PLN157 § V-af (`@FR-O-Buffer`, per path under `@FR-O-Complete`) — the right-hand
        // side is a value BRANCH whose arms
        // end in buffer-delivering calls: `v = if c { mk(i) } else { mk2(i) }`.  Each arm's
        // call delivers through a hidden buffer of its own and `v` adopts whichever ran, so
        // every arm's buffer is `v`'s witness — the same pairing a direct call takes below,
        // applied per tail call.  Without it `v`'s per-iteration free released the buffer's
        // store, the buffers stayed unreused, and every call minted a store (16 % of the
        // consumer's `smooth` row).  Records only: a vector's alias rules are the direct
        // call's own case.  Whether the callee adopts a fresh store or fills the one it is
        // handed makes no difference here: a branch binds the arm's `DbRef` as it is, with
        // none of the copy a direct call's set lowering interposes, so `v` aliases the
        // buffer either way.
        if record_shaped
            && !publishes_through_ref
            && crate::keys::join_buffer_witness_enabled()
            && matches!(unspanned_value, Value::If(_, _, _))
        {
            let v_scope = self.var_scope.get(&v).copied().unwrap_or(u16::MAX);
            for call in tail_calls(unspanned_value) {
                let Value::Call(fn_nr, args) = call else {
                    continue;
                };
                if (*fn_nr as usize) >= data.definitions.len()
                    || !data.def(*fn_nr).is_loft_defined()
                {
                    continue;
                }
                for arg in args {
                    let Value::Var(av) = arg.unspan() else {
                        continue;
                    };
                    let n = function.name(*av);
                    if !(n.starts_with("__ref_") || n.starts_with("__rref_")) || *av == v {
                        continue;
                    }
                    let av_scope = self.var_scope.get(av).copied().unwrap_or(u16::MAX);
                    if v_scope <= av_scope && v_scope != u16::MAX {
                        self.paired_witness.entry(*av).or_insert(v);
                    } else if v_scope != u16::MAX && av_scope != u16::MAX && v_scope > av_scope {
                        let buffers = self.witness_buffer.entry(v).or_default();
                        if !buffers.contains(av) {
                            buffers.push(*av);
                        }
                    }
                }
            }
        }
        // loft#1643 — a NULLABLE record local whose one real bind follows its null on every
        // pass (`x: T? = null; if c { x = mk() }`, the spelling `(B-Scope)` requires) is a
        // first bind of the call's answer exactly as the bare local's is, so it takes that
        // bind's witness PAIRING: its free declines against the call's buffer by identity.  Only the pairing: the dep strips below stay
        // bare-record-only for the reason `record_target` gives.
        let nullable_first_adopt = !record_shaped
            && !publishes_through_ref
            && matches!(function.tp(v), Type::Optional(_))
            && matches!(
                function.tp(v).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && self.null_led_first.contains(&ov);
        if (record_shaped || vector_shaped || nullable_first_adopt)
            && matches!(unspanned_value, Value::Call(_, _) | Value::CallRef(_, _))
            && let Some(fn_nr) = crate::use_analysis::callee_of(data, self.d_nr, unspanned_value)
            // A loft-defined callee — an `n_` global OR a `t_` method / generic
            // monomorph (@PLN85 generic-tuple-return-fix.md — a generic tuple return
            // is a `t_<Type>_<fn>` monomorph; without `t_` the adopts-fresh /
            // OpFreeRefIfDistinct pairing was skipped and the caller freed the
            // aliased return with a plain OpFreeRef, orphaning its text fields).
            // This decision and codegen's copy-or-adopt one have to name the SAME set
            // of callees, which is why the predicate lives in one place (loft#810).
            && data.def(fn_nr).is_loft_defined()
        {
            let adopts_fresh_store = data.def(fn_nr).return_adopts_fresh_store();
            // @PLN164 B1 (`@FR-O-Move`) — a plain local bound from a callee that returns
            // its own promoted local takes the SAME pairing a fresh-adopting callee's result
            // takes: the deps are stripped below (the local OWNS the store the callee
            // minted) and its free is guarded by identity against the call's buffer.  One
            // home decides it for the two backends' bind arms too.
            let adopts_minted =
                crate::use_analysis::adopts_minted_at_bind(data, function, v, unspanned_value);

            // @PLN85 `local_source` over-free fix (LOFT_JOIN_OWN): `v` holds an OWNED
            // store (this adopts-fresh call) that a later borrow/join reassignment
            // displaces. Strip `v`'s declared deps so it is OWNED everywhere — the
            // owned path then deep-copies the borrow into `v`'s store and frees it at
            // scope exit; without this the displaced owned store is orphaned (it was
            // bound to `v`, not to the source retbuf the cleanup guards) and leaks.
            if record_shaped
                && !publishes_through_ref
                && self.displaced_owned.contains(&ov)
                && !function.tp(v).depend().is_empty()
                // @FR-O-Proxy asks free.  @FR-O-Override vetoes it at every site that frees
                // on it, and stripping the deps IS such a site: `get_free_vars` reads the dep list, so
                // emptying it here is what makes the scope-exit sweep emit `OpFreeRef(v)`.
                // A binding the parser marked never-free keeps its deps and its store.
                //
                // ⚠ NOT [`crate::variables::Function::proxy_says_owned`], and the difference is
                // the whole reason it stays apart (@PLN155 phase 1): that predicate is
                // `empty && !veto`, while this is `!empty && !veto`.  The negated proxy here is
                // not an ownership answer at all — it asks *"is there a dep list left to
                // strip"*, which is a question about WORK TO DO.  Only the veto is this rule's
                // obligation, and it is read on its own for that reason.
                && !function.is_skip_free(v)
            {
                let deps: Vec<u16> = function.tp(v).depend().clone();
                for d in deps {
                    function.make_independent(v, d);
                }
            }
            if record_shaped && !adopts_fresh_store && !publishes_through_ref {
                if let Some(base) = crate::use_analysis::view_elision_bind(
                    data,
                    self.d_nr,
                    function,
                    v,
                    unspanned_value,
                    &crate::use_analysis::BodyFacts {
                        read_only: &self.read_only_locals,
                        multi_assigned: &self.multi_assigned,
                        assigned: &self.assigned,
                    },
                ) {
                    // @PLN157 § V-g — the copy is unobservable here, so the dep STAYS (a
                    // view) and the local takes the COLLECTION join's route: `get_free_vars`
                    // releases the callee's per-execution minted store by identity against
                    // the argument at scope exit, and a re-Set in scope releases the store
                    // it displaces the same way.  Both backends read the mark and deliver
                    // the result directly instead of copying.
                    function.mark_view_elided(v);
                    let w = function.rebind_orig(base).unwrap_or(base);
                    self.lift_join_witness.insert(v, w);
                    let slot_live = match self.lift_decl_depth.get(&v) {
                        Some(&depth) => depth < self.loops.len(),
                        None => true,
                    };
                    if was_in_scope && transition_free.is_none() && slot_live {
                        transition_free = Some(Value::Call(
                            data.def_nr("OpFreeRefIfDistinct"),
                            vec![Value::Var(v), Value::Var(w)],
                        ));
                    }
                } else {
                    // codegen will take gen_set_first_ref_call_copy —
                    // OpConvRefFromNull +
                    // OpDatabase + lock-args + OpCopyRecord deep-copy into a
                    // FRESH store owned by `v`.  Strip v's declared deps so
                    // get_free_vars emits OpFreeRef at scope exit; otherwise
                    // the parser's "borrows from arg N" inference suppresses
                    // emission and the deep-copied store leaks (the
                    // `dep_empty=false` path in scopes.rs:906).
                    let deps: Vec<u16> = function.tp(v).depend().clone();
                    for d in deps {
                        function.make_independent(v, d);
                    }
                }
            }
            // `adopts_fresh_store == true` call whose result is assigned
            // to a Reference variable `v`.  At runtime the callee either:
            //   - **adopts** the placeholder (writes into the passed
            //     `__ref_N` and returns the same DbRef) — then `v`
            //     and `__ref_N` share a store;
            //   - **allocates fresh** (e.g. `return map_empty()` or
            //     `T.parse(text)` with an internal fresh alloc) —
            //     then `v`'s store and `__ref_N`'s placeholder store
            //     are distinct, and the placeholder is orphaned.
            //
            // The compiler cannot resolve the choice statically: a
            // single callee (`map_from_json`) branches both ways on
            // `json == ""`.  Both patterns must work.
            //
            // Plain `OpFreeRef(__ref_N)` at scope exit is wrong in
            // the adoption case when `v` flows into the enclosing
            // function's return — the placeholder free happens
            // BEFORE the caller reads `v`, corrupting `v`'s shared
            // store.  Unconditionally skipping the free is wrong in
            // the fresh-store case — placeholder orphaned.
            //
            // Record `__ref_N → v` in `paired_witness`.  At scope
            // exit, `get_free_vars` emits `OpFreeRefIfDistinct(__ref_N,
            // v)` instead of `OpFreeRef(__ref_N)`: the runtime
            // store-nr comparison settles the two cases per execution
            // path (match → skip; differ → free).
            //
            // loft#759 — a set THROUGH a `&` parameter needs the same pairing
            // whatever `adopts_fresh_store` says.  For a LOCAL target the flag
            // decides whether a deep copy stands between the buffer and `v`
            // (`!adopts_fresh_store` copies, so the two are always distinct and
            // the pairing would be a no-op).  No copy stands in the `&` case,
            // so the buffer and the caller's slot alias whenever the callee
            // returned the buffer it was handed — the majority shape, and the
            // one `file()` has (`result = File{..}; result`).
            // ⚠ A VECTOR pairs whatever `adopts_fresh_store` says, and the asymmetry is
            // the whole point.  The flag means *the callee mints its own store rather than
            // filling the one I passed*, so for a RECORD its false case is safe on its own:
            // `gen_set_first_ref_call_copy` interposes a deep copy, and `v` and the buffer
            // cannot alias.  A vector has no such copy path — it is PutRef-ALIASED to the
            // work-ref argument — so there the false case is the one where they DEFINITELY
            // alias, and a plain `OpFreeRef(v)` releases the caller's own buffer.  Hoisted
            // out of a loop and reused every iteration, that buffer is then written after
            // the free (loft#1201).  `OpFreeRefIfDistinct` answers both cases at run time
            // and is conservative in the direction that matters: it frees exactly as the
            // plain free did when the stores DIFFER, and only skips when they alias.
            self.pair_call_buffers(
                v,
                unspanned_value,
                function,
                BindShape {
                    adopts: if adopts_fresh_store {
                        Adopts::Fresh
                    } else if adopts_minted {
                        Adopts::Minted
                    } else {
                        Adopts::Neither
                    },
                    publishes_through_ref,
                    vector_shaped,
                },
            );
        }
        // @PLN85 over-free class — a VECTOR return-buffer (the hidden SRet arg,
        // NRVO'd into a source local like `best`) bound from a call that
        // DETERMINISTICALLY returns its OWN buffer (`!return_adopts_fresh_store()`,
        // e.g. `best = rows(b, __ref_1)` where `rows` returns its `__retbuf`) is
        // PutRef-ALIASED to the work-ref arg `__ref_1` — no deep copy (unlike the
        // Reference path above, which `gen_set_first_ref_call_copy` copies). A plain
        // `OpFreeRef(__ref_1)` at scope exit then whole-store-frees the RETURNED
        // buffer. Pair `__ref_1 → ov` so the free becomes
        // `OpFreeRefIfDistinct(__ref_1, ov)`: a no-op since they alias (the caller
        // owns + frees the returned buffer). Restricted to the return-buffer ARG —
        // a dead vector LOCAL (e.g. `other`) is not an argument, so it keeps its
        // plain free (its borrowed source must still be released, else it leaks).
        // Extended to Reference (struct) + struct-Enum return-buffers too: a struct
        // retbuf `ns = sim_new_gen_s(…, __ref_1)` that ADOPTS the buffer-returning
        // callee's `__ref_1` has the SAME plain-`OpFreeRef(__ref_1)` over-free (#462's
        // sim_descend driver, tp=194). The witness-pair is conservative —
        // `OpFreeRefIfDistinct(__ref_1, ov)` frees `__ref_1` exactly as the plain free
        // did when they are DISTINCT (the deep-copy case), and only skips when they
        // alias (the adopt case, the bug) — so this can only fix, never regress.
        if matches!(
            function.tp(ov),
            Type::Vector(_, _) | Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && function.is_argument(ov)
            && data
                .def(self.d_nr)
                .attributes
                .iter()
                .any(|a| a.hidden && a.name == function.name(ov))
            && let Value::Call(fn_nr, args) = unspanned_value
            && data.def(*fn_nr).name.starts_with("n_")
            && data.def(*fn_nr).code != Value::Null
            && !data.def(*fn_nr).return_adopts_fresh_store()
        {
            for arg in args {
                let av = match arg {
                    Value::Var(a) => Some(*a),
                    Value::Set(a, _) => Some(*a),
                    _ => None,
                };
                if let Some(av) = av {
                    let n = function.name(av);
                    if n.starts_with("__ref_") || n.starts_with("__rref_") {
                        self.paired_witness.entry(av).or_insert(ov);
                    }
                }
            }
        }
        // loft#1317 — an inline record literal bound to a NULLABLE local mints into a
        // `__ref_N` work-ref and then ALIASES it into the local: `c: S? = S { x: 5 }` lowers
        // to `c = { OpDatabase(__ref_1); OpSetInt(__ref_1, …); __ref_1 }`, so the two names
        // hold ONE store.  The dense twin never gets here — `OpDatabase` builds straight into
        // `c` and there is no buffer — which is why only the nullable spelling had the fault.
        //
        // A work-ref's scope-exit free is FORCED (`is_work_ref` in `get_free_vars`), so it
        // runs even where `leaves_frame` suppressed the local's own.  A returned nullable record was
        // therefore handed back through a store this frame had already released:
        // `fn f() -> S? { c: S? = S { x: 5 }; c }` answered `0xDEADBEEF` under `LOFT_POISON=1`
        // on both backends, and the right value on an ordinary build, which is why it stood.
        //
        // Pairing the buffer with the local turns that free into
        // `OpFreeRefIfDistinct(__ref_1, c)`, and the run-time comparison answers all four
        // combinations — the same trade the call-shaped pairings above take:
        //
        //   returned, local still names the store  -> alias  -> decline; the caller owns it
        //   returned, local reassigned since       -> differ -> free; the literal store is dead
        //   not returned, still named              -> alias  -> decline; `OpFreeRef(c)` above
        //                                                       it already released the store
        //   not returned, reassigned since         -> differ -> free
        //
        // So it frees exactly where the plain free did whenever the stores differ, and only
        // declines where the plain free was releasing a store someone else still owns.
        //
        // The SAME two names, the other way round, where the local is INNER-scoped — a loop
        // body — and the buffer is the function's: that is @P378(a)'s shape, and it takes
        // @P378(a)'s answer (`witness_buffer`).  The local dies once per iteration, and a plain
        // free there releases the buffer's store while the buffer keeps naming it; the next
        // pass re-mints through `OpDatabase`, which REUSES the slot's store in place — a number
        // the free handed back and another record has since taken.  `for … { o = O { opt: S {
        // n: 6 } }; y: S? = S { n: 3 }; }` wrote the second iteration's literal over `o`'s
        // record on both backends, and nothing reported it: the store was live, just not the
        // buffer's.  With the pairing, the local's free is `OpFreeRefIfDistinct(y, buffer)`:
        // declined while they alias (the buffer keeps its store, reuses it in place, frees it
        // once at exit), a real free where the local moved on.  Every arm of a value branch
        // minted a buffer of its own and the local adopted whichever ran, so the pairing
        // carries them all and the free declines against each.
        if matches!(
            function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) {
            let mut adopted: Vec<u16> = Vec::new();
            adopted_work_refs(unspanned_value, function, data, &mut adopted);
            let v_scope = self.var_scope.get(&v).copied().unwrap_or(u16::MAX);
            for av in adopted {
                if av == v || v_scope == u16::MAX {
                    continue;
                }
                // The witness must outlive the buffer, or native's `let` for it has fallen
                // out of scope by the time the buffer's free runs — the same condition the
                // call-shaped pairing above states at length.
                let av_scope = self.var_scope.get(&av).copied().unwrap_or(u16::MAX);
                if v_scope <= av_scope {
                    self.literal_buffer.entry(av).or_insert(v);
                } else if av_scope != u16::MAX {
                    let buffers = self.witness_buffer.entry(v).or_default();
                    if !buffers.contains(&av) {
                        buffers.push(av);
                    }
                    // loft#1522 — the same pairing the scope-exit free asks about, carried to
                    // the one fact BOTH backends read for the DISPLACEMENT free.  Recorded
                    // here so the two cannot drift: a local gains the veto exactly when it
                    // gains the guarded scope-exit free.
                    function.mark_buffer_witnessed(v);
                }
            }
        }
        // @PLN130 F2 — an element view that is live across a RESHAPE of its container cannot
        // stay an alias.  `remove` renumbers the positions in the container's store and the
        // view is a `DbRef` pinned to one, so it silently starts naming a different element:
        // measured, a pure READ answered `44/444` where its element held `33/333`, and a
        // write tore a live record (`99/444` — n from the stray write, tag from the real
        // element).  No detector sees it: nothing is freed and the pointer is live.
        //
        // Strip the container dep so the binding materialises into a store it owns (the same
        // F1 arm in `state/codegen.rs` picks it up, and native's generator already
        // materialises off empty deps).  The alias is LOST for this binding, so say so —
        // constraint 2, where rustc errors on a use-after-move loft copies and warns.
        //
        // Only fires for a view that is still USED after the reshape. A view whose last use
        // precedes it is not at risk, and materialising it would lose a write that lands
        // today — see `collect_views_to_materialise`.
        let mut collection_copy: Option<(u16, i32)> = None;
        let mut keyed_copy: Option<(u16, i32)> = None;
        // Asked once, here: the arm below clears the never-free mark this reads, because a
        // materialised binding is no longer a view.
        let keyed_view = keyed_payload_view(function, v);
        if (matches!(
            function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
        ) || keyed_view)
            && let Some(cause) = self.views_to_materialise.get(&v).copied()
            && let Some(container) = base_container_var(unspanned_value, data)
            // Every dep this binding carries names the container being disturbed — a VIEW
            // test, not an ownership one, and deliberately not the empty-deps proxy: what
            // makes the binding a view here is the walk's answer plus `base_container_var`,
            // and a dep would only be restating it.  A payload binding written
            // `if sh is Holder { inner }` carries NO dep where its `match` twin does, and one
            // with nothing to strip still has a mark to lift and emitters to steer, so both
            // must pass.  A binding whose deps name something ELSE is declined: that one is
            // not a view of what was disturbed.
            && function.tp(v).depend().iter().all(|d| *d == container)
        {
            let vname = function.name(v).to_string();
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
            // The binding has stopped being a view, so the NEVER-FREE mark it was given as
            // one goes with the deps (@FR-O-Override marks a borrow; this is no longer one).
            // A `match`/`is` payload binding (`_mv_<field>_N`) is marked by the parser, and
            // stripping only the deps left it owning a store nothing released — one leaked
            // record per call on `--native`, measured.  Two facts, one statement: an owner
            // frees what it owns.
            function.clear_skip_free(v);
            // ...and the VECTOR arm below gives it back, for a different fact: there the
            // local names a buffer that owns the store, so the clear must precede the set or
            // the local frees the buffer's store at scope exit.
            // A COLLECTION view needs the copy EMITTED, where a record view only needs its
            // dep stripped.  The record bind reads the deps at emit time and copies; the
            // collection bind decides copy-vs-view at PARSE time (`classify_vec_bind`'s
            // `depend().is_empty()`, the `(B-View-Base)` citation), which runs before this
            // pass, so a strip here arrives too late to be heard.  The buffer + refill is the
            // shape a whole-vector copy already takes (`ArmBind::CopyVector`): a `__lift_N`
            // that owns its store for the function's life and is refilled in place, so a
            // materialise inside a loop costs one store rather than one per iteration.
            // A KEYED payload binding is copied the same way through the keyed twin of the
            // refill, `OpReplaceKeyed` (loft#1664): a collection bind is decided at PARSE time,
            // so a scope-pass strip alone would leave the binding a view of a store its
            // subject no longer owns.
            if keyed_view && let Some(ktp) = keyed_type_id(self.database, data, function.tp(v)) {
                let tp = function.tp(v).without_deps();
                let tmp = self.new_buffer_var(function, &tp);
                function.set_skip_free(v);
                keyed_copy = Some((tmp, i32::from(ktp)));
            }
            if let Type::Vector(inner, _) = function.tp(v).base().clone() {
                let wrapper = format!("main_vector<{}>", inner.name(data));
                if data.name_type(&wrapper, data.def(self.d_nr).source) != u16::MAX
                    && let Some(elem) = data.vector_element_type(&inner, self.database)
                {
                    let tp = Type::Vector(inner.clone(), Deps::none());
                    let tmp = self.new_buffer_var(function, &tp);
                    // The local NAMES the buffer; the buffer owns the store and frees it once
                    // at function exit.  Left owning, the local's own scope-exit `OpFreeRef`
                    // releases the buffer's store — harmless at function scope and fatal in a
                    // LOOP, where the next iteration refills a freed store and the read comes
                    // back poisoned (`rec=3735928559`).  This is the same borrow the snapshot
                    // witness takes, for the same reason (@FR-O-Borrow: naming a store is not
                    // owning it).
                    function.set_skip_free(v);
                    collection_copy = Some((tmp, i32::from(elem)));
                }
            }
            let fname = data.def(self.d_nr).original_name();
            // The container the ADVICE names is the one that was DISTURBED, carried on the
            // walk's own answer.  `cname` above is the one the binding's deps name, which the
            // strip needs and the sentence does not: for a binding that views two containers
            // they are different, and only one of them was reassigned.
            let cname = function.name(cause.container).to_string();
            let via = disturbance_via(data, &cause);
            report_materialised_view(
                cause.reported_cause(),
                &vname,
                &cname,
                &fname,
                via.as_deref(),
            );
        }
        // Companion to the !adopts_fresh_store (deep-copy) branch above for the
        // var-to-var deep-copy path.  When `Set(v, Var(src))` and
        // both are References to the same struct, codegen takes
        // `gen_set_first_ref_var_copy` (state/codegen.rs:1025-1033)
        // which OpConvRefFromNull + OpDatabase + OpCopyRecord
        // deep-copies src into a FRESH store owned by `v`.  This
        // path is hit by the I13 iterator protocol's hidden
        // `__iter_obj_N = c` setup (parser/collections.rs:209).
        // Strip v's declared deps so get_free_vars emits OpFreeRef.
        //
        // Through `base()` on both sides: `S?` is the same storage behind a nullability
        // marker (@FR-L-Null), and both emitters copy the nullable spelling of this bind
        // exactly as the dense one (`gen_set_first_ref_var_copy` reads `base()`).  Asked
        // bare, a nullable local reassigned from a value branch kept the join deps the parser
        // typed it with once the branch was written out per arm, and the per-arm copies
        // then read as borrows: an alias on both backends, where the dense twin copied.
        if let Value::Var(src) = unspanned_value
            && var_copy_owns(function, data, v, *src)
        {
            // @PLN130 F1 — this strip is LOAD-BEARING FOR NATIVE, which is why the obvious
            // narrowing does not work.  Skipping it when both sides are borrows fixes the
            // interpreter (probe 30 goes green) and BREAKS native (`len 0 want 3`, the same
            // destruction the other way round): the emptied deps are exactly what makes the
            // native generator materialise `_own_store_k` at the bind, so leaving them makes
            // native alias the container instead.  Measured, then reverted.
            //
            // The real fix therefore cannot be "stop promoting the borrow" — it has to
            // materialise at the BIND for both backends, which is what native already does
            // as a side effect of this strip. See @PLN130 § F1 + F2 design.
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
        }
        // loft#1320 — a value joined from BRANCH ARMS whose tails are fn-ref `??` calls.  The
        // joined binding carries every arm's dep and so reads as a borrow, which is right for
        // the arm that hands back a caller's store and leaves the arm that MINTED with no
        // owner.  Give each such arm its own owner: rewrite the tail call into the BOUND
        // spelling on a temp declared in THIS statement's scope, so the branch borrows from
        // the temp and the temp frees by store identity against its one base (or, for a
        // record, owns unconditionally through `OpBindOrCopy`).  `(O-Complete)` asks for the
        // fact per binding, per path; this gives each path a binding.
        // A value BLOCK that ends in a local record (`t = { s }`) hands the binding that local as an
        // arm would, and `(B-Copy)` copies it the same way: without it the block spelling
        // aliased what the bare `t = s` copies (loft#1752).
        let rewritten_arms;
        // A RECORD only: a collection tail is lifted through `OpReplaceVector`, which reads an
        // absent vector as empty (`[for z in v { z }]` over a `vector<integer>?` element).
        let local_tail = if let Value::Block(bl) = value.unspan()
            && matches!(bl.operators.last().map(Value::unspan), Some(Value::Var(_)))
        {
            bl.result.base().heap_def_nr().is_some()
        } else {
            false
        };
        let value: &Value = if (Self::is_value_branch(value) || local_tail)
            && self.arm_tails_need_binding(value, v, data, function)
        {
            let mut rw = value.clone();
            // The temps live where the BINDING lives: a binding declared outside a loop and
            // re-Set inside it still names the arm's store after the loop, so a temp scoped
            // to the statement would be freed under it.
            let home = self.var_scope.get(&v).copied().unwrap_or(self.scope);
            // `(H-Materialise)` promises the author is TOLD when a view is copied out of its
            // container, and for a branch- or discharge-valued right-hand side the copy is
            // made here rather than by the deps strip above — so the report is owed here too.
            // Keyed on the lift actually TAKEN under the walk's gate, never on the walk's
            // answer alone: a sentence that asserts "writes through `c` no longer reach `v`"
            // over a binding that still aliases is worse than the silence it replaces, which
            // is the measured reason loft#1401's second cure was backed out.
            if self.lift_join_arm_tails(&mut rw, home, v, function, data)
                && let Some(cause) = self.views_to_materialise.get(&v).copied()
            {
                let via = disturbance_via(data, &cause);
                report_materialised_view(
                    cause.reported_cause(),
                    function.name(v),
                    function.name(cause.container),
                    &data.def(self.d_nr).original_name(),
                    via.as_deref(),
                );
            }
            rewritten_arms = rw;
            &rewritten_arms
        } else {
            value
        };
        // A branch JOIN hands over each arm's construction the way a single construction does
        // above: the binding adopts the one that ran, and the other arms' work-refs hold
        // nothing, so every one is disarmed (`construction_work_refs`).  Decided HERE, after the
        // arm lift, because the lift can turn the binding into a borrow of its per-arm temps —
        // `x = a ?? H {…}` lifts `a` — and then the binding releases nothing, so a work-ref
        // disarmed before the lift was released by nobody.  After it, this reads the same
        // ownership fact the drop hand-off reads once the statement is scanned.
        if delivered_work_ref(value, function, data).is_none()
            // @FR-O-Proxy asks free — the answer places the binding's hook and free as the
            // store's one claimant (the disarm above does the same for a single construction),
            // and the @FR-O-Override veto rides inside `proxy_says_owned` as one question.
            && function.proxy_says_owned(v)
            && drop_hook(function, v, data).is_some()
        {
            for w in construction_work_refs(value, function, data) {
                if w != v {
                    handoff_disarm.push(v_set(
                        w,
                        Value::Call(data.def_nr("OpNullRefSentinel"), vec![]),
                    ));
                }
            }
        }
        if first_binding {
            self.binding_now.push(v);
        }
        // The records this statement's prefix will release (below), known before the value is
        // scanned so the rebuild inside it does not release them a second time.
        let prefix_from = self.prefix_released.len();
        let built_before = captures_built_in_value(value, data);
        if !built_before.is_empty()
            && built_before
                .iter()
                .all(|(_, c)| self.owner_witness.contains_key(c))
        {
            self.prefix_released
                .extend(built_before.iter().map(|(rec, _)| *rec));
        }
        let scanned = self.scan(value, function, data);
        self.prefix_released.truncate(prefix_from);
        if first_binding {
            self.binding_now.pop();
        }
        // loft#1588 — the backings a tuple's vector members live in take the TUPLE's turn in the
        // sweep, just ahead of it.  Scanning the literal placed each one at its own temp
        // (`_vec_N`, D-heap-21), which for a REFILL is this statement rather than where the
        // tuple was declared; reading them after the scan is what lets this have the last word.
        if matches!(function.tp(v).base(), Type::Tuple(_))
            && let Some(now) = tuple_member_backings_of(value, function)
        {
            let mut now: Vec<u16> = now.into_values().collect();
            now.sort_unstable();
            for b in now {
                if let Some(bpos) = self.var_order.iter().position(|&x| x == b) {
                    self.var_order.remove(bpos);
                    let at = self.var_order.iter().position(|&x| x == v).unwrap_or(bpos);
                    self.var_order.insert(at, b);
                }
            }
        }
        // Flatten: if the scanned value is Insert([preamble..., final_call]),
        // hoist the preamble out so the IR becomes
        // Insert([preamble..., Set(v, final_call)]) instead of
        // Set(v, Insert([preamble..., final_call])).
        // This keeps Set(v, Call(...)) as a bare Call, which codegen's
        // gen_set_first_at_tos can handle correctly.
        // The scanned value may carry its source position: a `Span` around the `Insert` hid the
        // shape from the flatten (the `unspan` contract), so a call bound after an argument's
        // preamble reached the bind dispatch as `Set(v, Insert(…))` and never had its result
        // copied or adopted per `@FR-O-Move` — a callee answering its by-value parameter was
        // ALIASED and then freed as the local's own (loft#1884).  The position stays on the
        // final value.  Not where the scan already reads the value as a construction DELIVERED
        // through the call (`delivered_work_ref`, loft#1575's `s = me(Bx { … })`): the binding
        // adopts that work-ref's store and the work-ref is disarmed, so the bind must stay the
        // plain adopt — flattened, the split would copy and the disarmed store would leak.
        let delivered = delivered_work_ref(value, function, data).is_some();
        let scanned = match scanned {
            Value::Span(b)
                if !delivered && matches!(&b.1, Value::Insert(ops) if ops.len() >= 2) =>
            {
                let (pos, inner) = *b;
                let Value::Insert(mut ops) = inner else {
                    unreachable!("matched as an Insert above")
                };
                if let Some(last) = ops.pop() {
                    ops.push(Value::Span(Box::new((pos, last))));
                }
                Value::Insert(ops)
            }
            other => other,
        };
        let (mut ls, mut set_value) = if let Value::Insert(mut ops) = scanned {
            if ops.len() >= 2 {
                let final_val = ops.pop().unwrap();
                (ops, final_val)
            } else {
                (Vec::new(), Value::Insert(ops))
            }
        } else {
            (Vec::new(), scanned)
        };
        // loft#1106 — a NULLABLE heap local first-bound from a call whose return may
        // borrow an argument.  `S?` is `Optional(Reference(S))`, and the shape questions the
        // heap first-bind dispatch asks are asked against the BARE type, so the nullable
        // spelling of the same storage never reached the deps strip: the local kept the
        // argument's dep, read as a permanent borrow, and nothing freed the store the callee
        // minted on its other arm.  Both backends bind it through the runtime join guard,
        // which leaves the local owning a store either way — so the deps have to go, or the
        // free that guard exists to make correct is never emitted.
        //
        // The collection materialise decided above, emitted here because it wraps the SCANNED
        // right-hand side: `OpReplaceVector(buffer, <the view>, elem)` clears the buffer and
        // refills it from what the view names, and the local then binds the buffer.  Writes
        // through the local land in the copy, which is what `(B-View)`'s materialise means and
        // what the advice already told the author.
        if let Some((tmp, ktp)) = keyed_copy {
            let view = std::mem::replace(&mut set_value, Value::Null);
            set_value = Value::Insert(vec![
                Value::Call(
                    data.def_nr("OpReplaceKeyed"),
                    vec![view, Value::Var(tmp), Value::Int(ktp)],
                ),
                Value::Var(tmp),
            ]);
        }
        if let Some((tmp, elem)) = collection_copy {
            let view = std::mem::replace(&mut set_value, Value::Null);
            set_value = Value::Insert(vec![
                Value::Call(
                    data.def_nr("OpReplaceVector"),
                    vec![Value::Var(tmp), view, Value::Int(elem)],
                ),
                Value::Var(tmp),
            ]);
        }
        // Asked against `set_value`, the SCANNED right-hand side, not the raw one: `scan`
        // has just LIFTED any argument the @P290 bracket could not name into a temp, and the
        // witness the join resolves is that temp.  Read before the lift the same call answers
        // "no nameable witness" and the strip declines, while codegen — which only ever sees
        // the scanned form — emits the guard anyway.  Then the local owns a store with no
        // free: one leaked record per call on the minting arm, from the two readers of ONE
        // predicate disagreeing about which value they were reading.
        //
        // loft#1248 — and the same sentence for a bind from a CLOSURE call, which reaches
        // neither this strip's sibling above nor the `Value::Call` strip earlier in this
        // function: both are keyed on the call spelling that names its definition, and a
        // `CallRef` names a runtime value.  So a closure whose return may be its argument or
        // may be a store it minted kept the argument's dep, read as a permanent borrow, and
        // the minted arm's store was owned by nobody — one per call, to FRAME exit, which a
        // loop turns into the 65535-store ceiling.
        if crate::use_analysis::nullable_join_first_bind(
            data,
            self.d_nr,
            function.tp(v),
            &set_value,
        )
        .is_some()
            || crate::use_analysis::callref_join_first_bind(
                data,
                self.d_nr,
                function.tp(v),
                &set_value,
            )
            .is_some()
        {
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
        } else if let Some(base) = crate::use_analysis::callref_collection_join_base(
            data,
            self.d_nr,
            function.tp(v),
            &set_value,
        ) && !function.is_skip_free(v)
            && !function.is_argument(v)
        {
            // loft#1257 / loft#1320 — the COLLECTION twin of the strip above, and it goes
            // the other way: the dep STAYS, because it names the witness.  A collection has
            // no `OpBindOrCopy`, so the local may hold the caller's store or one the closure
            // minted, and only the store number can say which.  `get_free_vars` frees it by
            // identity at scope exit; a RE-Set of a named local releases the store it is
            // about to stop naming the same way, before the new value is computed.  A lift
            // temp gets no transition free: its only Set runs once per scope and the scope's
            // own exit already freed the slot.
            //
            // The witness is the store the base named AT THE BIND.  Where the base is
            // assigned once and this local has one base, the base variable itself still
            // names that store at every later free, so it is the witness.  Where it does not
            // — the base is reassigned in this function, or the local is bound at two sites
            // from two different bases — the witness is a SNAPSHOT of the base taken beside
            // the bind (`@FR-O-Latest`: the fact belongs to the assignment, and here it is
            // carried by a slot the way @PLN87's entry stash carries a rebindable
            // parameter's).  Comparing against the LIVE base instead freed a caller's store
            // on the other site's arm (sum 4034 for 12500), and against a base already
            // re-pointed it could free whatever store reused the slot; comparing against the
            // snapshot, two stale numbers still agree and decline.
            // A parameter with any `Set` is rebound after entry (its bind is the call), so
            // it cannot stand witness for the store it named at the bind.
            let base_rebound = function.is_argument(base) && self.assigned.contains(&base);
            let stable = !self.multi_assigned.contains(&base)
                && !base_rebound
                && self.callref_join_bases.get(&v).is_none_or(|b| b.len() <= 1);
            let w = if stable {
                function.rebind_orig(base).unwrap_or(base)
            } else {
                let wit = self.snapshot_witness_for(v, base, function);
                witness_snapshot = Some(v_set(wit, Value::Var(base)));
                wit
            };
            self.lift_join_witness.insert(v, w);
            // A re-Set releases the store it displaces, guarded the same way — where the slot
            // is LIVE.  A named local's earlier Set in the same scope chain left it live; a
            // lift temp's slot is live only if the temp was created outside the innermost
            // loop that re-runs this Set, since a temp inside it is freed at that loop body's
            // exit and would be freed twice.
            let slot_live = match self.lift_decl_depth.get(&v) {
                Some(&depth) => depth < self.loops.len(),
                None => true,
            };
            if was_in_scope && transition_free.is_none() && slot_live {
                transition_free = Some(Value::Call(
                    data.def_nr("OpFreeRefIfDistinct"),
                    vec![Value::Var(v), Value::Var(w)],
                ));
            }
        } else if self.callref_delivers_collection(function.tp(v), &set_value, data)
            // @FR-O-Proxy asks free — read negated, *"this still looks like a borrow"*:
            // stripping the deps is what makes `get_free_vars` emit the free, so the site is
            // a free site and consults @FR-O-Override like every other.
            //
            // ⚠ NOT [`crate::variables::Function::proxy_says_owned`] — the sibling strip site
            // above carries the reason: `!empty && !veto` asks whether there is a dep list to
            // strip, not whether the binding owns its store (@PLN155 phase 1).
            && !function.tp(v).depend().is_empty()
            && !function.is_skip_free(v)
            // loft#1333 — and NOT for a MIXED binding, one another path assigns a borrow.
            // The deps this would strip are that other path's, not this delivery's: the two
            // arms share one binding, so declaring it the owner of this buffer also declares
            // it the owner of the store the borrow arm merely views, and the displacement
            // free then released it.  @FR-O-Complete says the fact is per BINDING and per
            // PATH; where one static site cannot be both, the rule names the direction —
            // a retained buffer is recoverable, a premature free is not.
            && !function.has_borrow_arm(v)
        {
            // A collection a closure hands back is DELIVERED — copied by the callee into the
            // buffer minted for that call — unless it is a raw view (`returns_borrowed_view`)
            // or a `Join`, which the two arms above own.  The type inherited from the callee
            // still names the ARGUMENT the tail borrowed before the delivery copied it, so the
            // local read as a borrow and the buffer was freed by nobody: `t = h(bag)` with
            // `h = fn(q: Bag) -> vector<integer> { q.items }` held one store per call.  The
            // binding owns the buffer; say so, and `get_free_vars` frees it (`@FR-O-Move`).
            let deps: Vec<u16> = function.tp(v).depend().clone();
            for d in deps {
                function.make_independent(v, d);
            }
        }
        // Prepend dependency initializations.
        let mut prefix = Vec::new();
        // #316 — the ownership-transition free runs FIRST: before the dep
        // inits, the hoisted RHS preamble, and the Set itself, so the owned
        // store is released before any part of the new value is computed.
        if let Some(free) = transition_free {
            prefix.push(free);
        }
        // …and the witness snapshot is written AFTER it, so the free compares against the
        // store the PREVIOUS bind named and the snapshot then names this bind's base.
        if let Some(snap) = witness_snapshot {
            prefix.push(snap);
        }
        for d in depend {
            if d == v {
                continue;
            }
            if matches!(function.tp(d), Type::Text(_)) {
                prefix.push(v_set(d, Value::Text(String::new())));
            } else {
                prefix.push(v_set(d, Value::Null));
            }
            self.put_scope(d);
        }
        // loft#1128 — keep the runtime witness in step with what `v` now holds.  A call that
        // DELIVERS into the buffer (`v` at the callee's buffer position and the callee does
        // NOT adopt a fresh store) leaves `v` holding whatever it held, so the flag is left
        // alone; every other assignment sets it to the same @FR-O-Latest verdict `owned_refs`
        // records statically.
        let mut witness_update = match self.rbuf_witness {
            Some((buf, flag)) if buf == v && !delivers_into_buffer(value, v, ov, data) => {
                let owned = matches!(self.ref_rhs_ownership(value, data), RefRhs::Owned);
                Some(v_set(flag, Value::Boolean(owned)))
            }
            _ => None,
        };
        // loft#1200 — the per-local witness records SOLE ownership, which is a narrower
        // question than `owned_refs`'s.  An inline mint into a work-ref is `Owned` and still
        // not solely owned: the work-ref frees it too.  Only a MINTING CALL hands the local a
        // store nothing else names, so that is the one shape that sets the flag true.
        // By `ov` for the reason the guarded free above is: the map's keys are original vars.
        if let Some(&flag) = self.local_owns.get(&ov) {
            let sole = mints_a_store_the_target_does_not_hold(value, v, ov, data);
            witness_update = Some(match witness_update {
                Some(prev) => Value::Insert(vec![prev, v_set(flag, Value::Boolean(sole))]),
                None => v_set(flag, Value::Boolean(sole)),
            });
        }
        // loft#1515 — this copy is one of the per-path hand-offs, so record that it RAN, on the
        // side it stops.  That side's scope-end release reads the flag: on this path the other
        // side owns the resource, on the path that did not copy the stopped side owes its own.
        // A copy off a parameter into a witnessed local sets the local's flag on every path it
        // runs, branch or not: the witness names the copy and must not hook it.
        let stops_target = self.copy_flagged_on_target(function, data, v, value)
            || self.witnessed_copy_stops_itself(function, data, v, value);
        // A copy that takes a FLAGGED local's release takes its flag too, read before this
        // statement writes the source's: the record is not the destination's to release on
        // exactly the paths it was not the source's — a copy off a parameter made in a branch
        // arm and handed on after it, or a record a branch arm already handed to another copy.
        let inherited = match value.unspan() {
            Value::Var(src)
                if !stops_target
                    && *src != v
                    && *src != ov
                    && copy_moves_drop_from(function, data, v, *src, false) == Some(*src) =>
            {
                match (self.handed_off.get(src), self.handed_off.get(&v)) {
                    (Some(&from), Some(&to)) => Some(v_set(to, Value::Var(from))),
                    _ => None,
                }
            }
            _ => None,
        };
        let inherits = inherited.is_some();
        if let Some(write) = inherited {
            witness_update = Some(match witness_update {
                Some(prev) => Value::Insert(vec![prev, write]),
                None => write,
            });
        }
        if let Value::Var(src) = value.unspan()
            && (stops_target || self.per_path_pairs.contains(&(v, *src)))
            && let Some(stopped) = per_path_stops(function, data, v, *src)
            && let Some(&flag) = self.handed_off.get(&stopped)
        {
            witness_update = Some(match witness_update {
                Some(prev) => Value::Insert(vec![prev, v_set(flag, Value::Boolean(true))]),
                None => v_set(flag, Value::Boolean(true)),
            });
        }
        // `@FR-O-Latest` — assigning a flagged variable retires its hand-off: what it holds
        // from here on is its own to release, whichever arm ran before.  Except the copy that
        // has just set the variable's OWN flag, a copy off a parameter: what it holds from here
        // on is the caller's.
        if !stops_target
            && !inherits
            && !self.handle_views.contains(&v)
            && let Some(&flag) = self.handed_off.get(&v)
        {
            witness_update = Some(match witness_update {
                Some(prev) => Value::Insert(vec![prev, v_set(flag, Value::Boolean(false))]),
                None => v_set(flag, Value::Boolean(false)),
            });
        }
        // loft#1336 / @FR-O-Witness — keep the OWNER WITNESS naming exactly the store the
        // local minted and still holds.  Three shapes, read off the value being assigned:
        //
        // - the local MINTS and the value does not read it: release the witnessed store
        //   first (the store it is about to stop naming), then point the witness at what the
        //   local now holds;
        // - the local MINTS from a value that READS it (`c = mk(c.x)`, a materialised
        //   `x = x.inner`): @FR-O-Detach — the old store stays live until the value is
        //   computed, so the release comes AFTER the `Set`, and by store identity, because
        //   a callee that filled the store the local already owned displaced nothing;
        // - anything else — a view, a join, a null, a store somebody else owns: the local
        //   stops owning, so release the witnessed store where the local no longer names it.
        //   A view INTO the witnessed store (`x = x.inner` over an owned `x`) keeps it, and
        //   the witness keeps naming it, so scope exit still frees it exactly once.
        //
        // Identity, not a flag, so both backends read one fact from the IR and the two
        // sentinels — a witness that never took a store beside a local holding none — compare
        // EQUAL and release nothing.
        let mut witness_ops: Vec<Value> = Vec::new();
        // @FR-O-Latest — the HAND-OFF, and it comes FIRST.  A closure build in this value
        // makes the record the owner of the store its capture names AT THE BUILD, so from
        // here the frame owes nothing for it and the witness gives it up — a clear, never a
        // free: `free_named`'s cascade owns it now.  Ahead of the guarded release below,
        // because for an INLINE closure argument the capture and the target are the SAME
        // local (`s = build(|i| { s.a + i })`): the record adopts the old store and the local
        // moves to a new one in one statement, so a release asked before the clear would free
        // the store the record has just taken.  That is `(O-Detach)` in ownership clothing,
        // and it is what made dropping the veto answer `a=1` where `a=2` is right.
        let built_here = captures_built_in_value(value, data);
        // ...and the record this build is about to OVERWRITE gives up what it already holds.
        // A build inside a loop reaches the same work-ref every pass and `OpDatabase` records
        // into the store it already names, so the capture slot is rewritten and the store it
        // held is orphaned — a 5-pass loop kept four (loft#1388).  `free_named`'s cascade is
        // the adopted store's releaser, so freeing the record here is what hands it back; on
        // the first pass the work-ref is `Null` and this is a no-op.
        //
        // Gated on EVERY capture in the build having a witness, and that gate is the whole
        // soundness argument: a capture without one is still owned by the FRAME, so the
        // cascade and the frame would free one store between them.  Measured exactly there —
        // a VECTOR capture can hold no witness (`heap_def_nr` is `None` for a vector, so the
        // record-typed witness variable cannot be made), and with the release ungated its
        // loop answered `1,2` where `4,5` is right, on `--native` alone.  `(O-Derived)`: one
        // decision, one home.
        if !built_here.is_empty()
            && built_here
                .iter()
                .all(|(_, c)| self.owner_witness.contains_key(c))
        {
            for (rec, _) in &built_here {
                // `@FR-L-CapOwn` — the record gives up what it adopted through its own
                // cascade, emitted here; the store free releases only the record.
                if let Some(hook) = super::drops::drop_hook(function, *rec, data) {
                    prefix.push(hook);
                }
                prefix.push(call("OpFreeRef", *rec, data));
            }
        }
        // loft#1628, `@FR-H-Drop` — a REBIND of a witnessed local that still holds a record it took
        // over from a plain local (`x: H = s.h ?? b; …; x = mk(9)`) displaces THAT record, whose
        // lease `(H-Move)` gave to `x`: its hook runs here, after the new value has landed, and
        // the local's `__hoff_` flag keeps its scope-end hook from running it a second time.  The
        // store is still freed at the local's scope end, with its call buffer, exactly as before.
        if was_in_scope && self.owner_witness.contains_key(&v) {
            for h in self.witness_aliases.get(&v).cloned().unwrap_or_default() {
                let Some(hook) = drop_hook(function, h, data) else {
                    continue;
                };
                let held = function.add_temp_var(
                    &format!("__hdsp_{}_{}", function.name(v), function.name(h)),
                    &Type::Boolean,
                );
                self.var_scope.insert(held, 0);
                self.var_order.push(held);
                let flag = self.mint_handoff_flag(function, h);
                prefix.insert(
                    0,
                    v_set(
                        held,
                        Value::Call(data.def_nr("OpEqRef"), vec![Value::Var(v), Value::Var(h)]),
                    ),
                );
                witness_ops.push(v_if(
                    Value::Var(held),
                    v_if(
                        Value::Var(flag),
                        Value::Null,
                        Value::Insert(vec![hook, v_set(flag, Value::Boolean(true))]),
                    ),
                    Value::Null,
                ));
            }
        }
        // loft#1628, `(H-Move)` — the frame's own locals a witnessed local may ALIAS: a value
        // branch whose arm is a plain local hands that local's record over as it is, so at a
        // return of `v` it is that local's record the caller takes.  Read at the return by
        // [`return_moved_holders`].
        if self.owner_witness.contains_key(&v) {
            let mut arms = Vec::new();
            crate::use_analysis::join_var_arms(value, &mut arms);
            for h in arms {
                // @FR-O-Proxy asks free — a holder recorded here has its hook RUN at a rebind of
                // `v` and guarded at a return, so only a local that owns its record may be one:
                // a view arm (`__ncc_N` over `s.h`) holds no lease, and running its hook would
                // release the container's member.  The @FR-O-Override veto rides inside
                // `proxy_says_owned` as one question.
                if h != v
                    && !function.is_argument(h)
                    && matches!(function.tp(h).base(), Type::Reference(_, _))
                    && function.proxy_says_owned(h)
                {
                    let slot = self.witness_aliases.entry(v).or_default();
                    if !slot.contains(&h) {
                        slot.push(h);
                    }
                }
            }
        }
        if let Some(&w) = self.owner_witness.get(&v) {
            let kind = {
                let d_nr = self.d_nr;
                let defs = self
                    .fn_defs
                    .get_or_insert_with(|| crate::use_analysis::function_defs(data, d_nr));
                witness_set_kind(value, v, ov, function, data, d_nr, &mut |val| {
                    crate::use_analysis::ownership_of_with(data, d_nr, val, defs)
                })
            };
            let guarded_release = v_if(
                Value::Call(
                    data.def_nr("OpDistinctStore"),
                    vec![Value::Var(w), Value::Var(v)],
                ),
                release_witness(w, self.witness_hook(function, data, v, w), data),
                Value::Null,
            );
            match kind {
                // `(H-Drop)`: the record a reassignment displaces is released AFTER the new
                // value has been computed — so a mint releases what the witness held exactly as
                // a mint that reads the local does.  Every copy into a witnessed local lands in
                // a fresh store (`(O-Witness)`), so the identity guard declines nothing here.
                WitnessSet::Mint | WitnessSet::MintReading => {
                    witness_ops.push(guarded_release);
                    witness_ops.push(witness_points_at(w, v, data));
                }
                WitnessSet::Other => witness_ops.push(guarded_release),
            }
        }
        if prefix.is_empty()
            && ls.is_empty()
            && witness_update.is_none()
            && witness_ops.is_empty()
            && displaced.is_none()
            && handoff_disarm.is_empty()
        {
            Value::Set(v, Box::new(set_value))
        } else {
            // The snapshot of the displaced record is taken FIRST — before the transition
            // free releases its store and before any part of the new value is computed —
            // and its hook runs LAST, after the new value has landed, so a right-hand side
            // that reads the old value (`s = grow(s)`) still finds the resource live.
            let (mut all, post) = match displaced {
                Some((pre, post)) => (pre, post),
                None => (Vec::new(), Vec::new()),
            };
            all.append(&mut prefix);
            all.append(&mut ls);
            all.push(Value::Set(v, Box::new(set_value)));
            // loft#1513 — the moment the binding has adopted the construction's store, the
            // work-ref that delivered it stops naming it, so its scope-end free cannot race
            // the binding's hook.
            all.extend(handoff_disarm);
            // The witness's release BEFORE the flag writes: a guarded release is of the record
            // the local held before this Set, and its hook reads THAT record's `__hoff_` flag,
            // which the writes below set or retire for the record the local holds now.
            all.append(&mut witness_ops);
            all.extend(witness_update);
            all.extend(post);
            Value::Insert(all)
        }
    }

    /// #316 — classify the (pre-scan) RHS of a `Set` into Reference var `v`.
    /// Only two shapes are provably OWNED: a user-fn call whose declared
    /// return carries no visible-attribute dep (the callee materialises /
    /// owns its result), and a same-struct `Var` copy (codegen deep-copies
    /// both first assignment and reassignment).  A `Block` whose result type
    /// carries deps is a view — unless a dep names `v` itself (the new value
    /// might point into the store about to be freed).
    /// The free-side owned-vs-view verdict for a Reference reassignment RHS,
    /// read from the CANONICAL `ownership_of` oracle (@PLN90 D-own-1 — the last
    /// per-site ownership re-derivation, folded onto the one fact the delivery
    /// side already reads).  Owned → track the var as owned.  Borrowed AND Join →
    /// View: a reassignment whose new value is a borrow OR a runtime join
    /// DISPLACES the var's prior owned store (freed by the #316 transition-free),
    /// and the var must NOT be tracked as owned afterward (a join might be a
    /// borrow — tracking it owned would over-free the NEXT transition).  So Join
    /// folds to View, not "don't-track" — the p462 conditional
    /// `chosen = t[i] ?? m_none()` reassign needs the prior-store free.
    /// loft#854 — the memoised form. `ownership_of` recomputes the whole-function
    /// summary per question; `scan_set` asks once per assignment, which made a
    /// function with n assignments cost n whole-function walks.
    fn ref_rhs_ownership(&mut self, value: &Value, data: &Data) -> RefRhs {
        let d_nr = self.d_nr;
        let defs = self
            .fn_defs
            .get_or_insert_with(|| crate::use_analysis::function_defs(data, d_nr));
        match crate::use_analysis::ownership_of_with(data, d_nr, value, defs) {
            // `Unknown` keeps `Owned`'s answer here (@PLN155 phase 2): to DECLINE instead,
            // this site would need a third `RefRhs` — "do not decide" — and every consumer of
            // `RefRhs` an arm for it.  That is phase 2b's question, not a transcription.
            crate::use_analysis::Own::Owned | crate::use_analysis::Own::Unknown => RefRhs::Owned,
            crate::use_analysis::Own::Borrowed { .. } | crate::use_analysis::Own::Join { .. } => {
                RefRhs::View
            }
        }
    }

    /// Does binding this fn-ref call hand the binding a collection store of its own — one the
    /// callee COPIED into the buffer minted for this call, or minted outright?  False for a
    /// call that is not a resolved, capture-free fn-ref, for a callee answering a raw VIEW of
    /// its argument (a keyed field, an index read — `returns_borrowed_view`), and for a `Join`,
    /// whose owner is decided per execution by the arms above.  The fallback is *"keep the
    /// borrow"*: a shape this cannot read costs the leak it already had, never a free of the
    /// caller's store.
    fn callref_delivers_collection(&self, tp: &Type, value: &Value, data: &Data) -> bool {
        let base_tp = tp.base();
        if !(matches!(base_tp, Type::Vector(_, _)) || crate::parser::vectors::is_keyed(base_tp)) {
            return false;
        }
        let Value::CallRef(v_nr, _) = value.unspan() else {
            return false;
        };
        let d_nr = match self.fnref_target.get(v_nr).copied() {
            Some(d) if d != u32::MAX => d,
            _ => return false,
        };
        let def = data.def(d_nr);
        // The `["??"]` marker is the callee's own word that its return is a `Join` — the
        // buffer on one arm, the argument on the other — so it is excluded by name as well
        // as by the oracle's verdict.
        def.code != Value::Null
            && !def.returns_borrowed_view()
            && !def.returned().depend().contains(&u16::MAX)
            && !crate::use_analysis::callref_capture_blocks(data, self.d_nr, value)
            && !matches!(
                crate::use_analysis::ownership_of(data, self.d_nr, value),
                crate::use_analysis::Own::Join { .. }
            )
    }

    /// The witness SLOT for a collection local bound from a closure's `Join` return where the
    /// base variable cannot stand witness itself — one per local, written beside every bind
    /// of that local from that bind's base.  A borrow of the base's type (never freed, never
    /// allocated for), homed where the local lives so it is readable at every free of it.
    fn snapshot_witness_for(&mut self, v: u16, base: u16, function: &mut Function) -> u16 {
        if let Some(&w) = self.snapshot_witness.get(&v) {
            return w;
        }
        self.lift_counter += 1;
        let name = format!("__wit_{}", self.lift_counter);
        let tp = function.tp(base).base().with_deps(&Deps::frame1(base));
        let wit = function.add_temp_var(&name, &tp);
        function.set_skip_free(wit);
        let home = self.var_scope.get(&v).copied().unwrap_or(self.scope);
        self.var_scope.insert(wit, home);
        self.var_order.push(wit);
        self.lift_vars.push(wit);
        self.snapshot_witness.insert(v, wit);
        wit
    }
}

/// The capture variables a value's closure BUILDS write into their record.
///
/// The record owns the store each names from the build on (`free_named`'s cascade is its
/// releaser), so a capture with an owner witness gives that store up there.  Read off the
/// VALUE rather than the statement, because an INLINE closure argument builds inside the very
/// assignment that moves the local on.
fn captures_built_in_value(value: &Value, data: &Data) -> Vec<(u16, u16)> {
    let set_dbref = data.def_nr("OpSetDbRef");
    fn walk(node: &Value, set_dbref: u32, out: &mut Vec<(u16, u16)>) {
        if let Value::Call(d, args) = node.unspan()
            && *d == set_dbref
            && let (Some(Value::Var(rec)), Some(Value::Var(c))) = (
                args.first().map(Value::unspan),
                args.get(2).map(Value::unspan),
            )
        {
            out.push((*rec, *c));
        }
        node.unspan()
            .for_each_child(&mut |c| walk(c, set_dbref, out));
    }
    let mut out = Vec::new();
    walk(value, set_dbref, &mut out);
    out
}

/// Does this call DELIVER its result into `v`'s existing store?
///
/// The complement of [`displaces_owned_through_fresh_callee`] on the same two facts: `v` sits
/// at the callee's hidden buffer position AND the callee does not adopt a fresh store, so it
/// writes through the buffer it was handed.  `v` then holds exactly what it held before, which
/// is why the runtime ownership witness must be left UNCHANGED rather than recomputed from the
/// call's return (loft#1128).
fn delivers_into_buffer(value: &Value, v: u16, ov: u16, data: &Data) -> bool {
    let Value::Call(fn_nr, args) = value.unspan() else {
        return false;
    };
    let def = data.def(*fn_nr);
    if def.return_adopts_fresh_store() {
        return false;
    }
    let Some(buf_idx) = def.hidden_return_buffer_attr() else {
        return false;
    };
    args.get(buf_idx)
        .is_some_and(|a| matches!(a.unspan(), Value::Var(w) if *w == v || *w == ov))
}

/// #316 — what kind of store does the RHS of a `Set` into a Reference var
/// yield?  Derived from the `ownership_of` oracle (@PLN90 fold): `Own::Owned`
/// maps to `Owned`, `Borrowed`/`Join` to `View` — the oracle's `_ => Owned`
/// fallback means there is no third "unprovable" case at this site.
enum RefRhs {
    /// A store the variable will own (safe to free on a later transition).
    Owned,
    /// A borrowed view into someone else's store (must never be freed).
    View,
}
