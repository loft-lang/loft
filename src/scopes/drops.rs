// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! The DROP a value's type runs: its scope-end hook, and the release a reassignment owes for the
//! value it displaces — a vector rebuilt or rebound, a promoted return buffer refilled, a record
//! rebuilt in place.

use super::backings::{call_buffer_of, literal_backing_of};
use super::capture_adoption::owning_record_locals;
use super::{Scopes, call};
use crate::data::{Data, Type, Value, v_if, v_set};
use crate::fxhash::FxHashSet as HashSet;
use crate::variables::Function;

impl Scopes<'_> {
    /// The hook a release through witness `w` runs for witnessed local `v`: the type's cascade
    /// over the record `w` names, unless that record's release is not this frame's — a copy off
    /// a parameter, recorded on `v`'s `__hoff_` flag.  One home for the four places a witness
    /// hooks (a mint's release of what it replaces, a release where the local stops owning, an
    /// in-place rebuild, scope exit), because a copy the witness names reaches every one of them.
    pub(super) fn witness_hook(
        &self,
        function: &Function,
        data: &Data,
        v: u16,
        w: u16,
    ) -> Option<Value> {
        let hook = drop_hook(function, w, data)?;
        Some(match self.handed_off.get(&v) {
            Some(&flag) => v_if(Value::Var(flag), Value::Null, hook),
            None => hook,
        })
    }

    /// The type's scope-end hook for `v`, unless a MOVE-copy already released `v`'s
    /// store — see [`collect_drop_transferred`].  `@FR-H-Drop`: the owner's scope-end
    /// clause.  One home for the rule, because
    /// both emission sites (the buffer-adoption leg and the ordinary one) must agree:
    /// a drop that runs on a released store is a use-after-free either way.
    pub(super) fn scope_end_drop(&self, function: &Function, v: u16, data: &Data) -> Option<Value> {
        // Two facts stop a release, and both are read.  `drop_transferred` holds the
        // hand-offs made on EVERY path, so a variable in it releases nothing.  A variable a copy
        // in a branch arm stops (loft#1515: its source, or its destination when the copy is off
        // a parameter) keeps its release and guards it on the flag that records whether that
        // copy ran — the copy never enters the static set, so a later unconditional hand-off of
        // the same variable still reaches it here.
        if self.drop_transferred.contains(&v) {
            return None;
        }
        let handed = self.handed_off.get(&v).copied();
        let guard = |d: Option<Value>| match handed {
            Some(flag) => d.map(|d| v_if(Value::Var(flag), Value::Null, d)),
            None => d,
        };
        guard(drop_hook(function, v, data))
    }

    /// The scope-end hook for `v` at THIS exit, or `None` where the exit has already placed it.
    ///
    /// One home for the two ways a hook can already be accounted for at a sweep: the per-arm
    /// rewrite of loft#1515 shape 2 put it inside the arms of a join, where the arm is the
    /// path; and the ordinary case, where [`Self::scope_end_drop`] answers with whatever skip
    /// this exit's own copy earned. Kept together because every one of the four release legs
    /// in [`Self::get_free_vars`] must ask the same question — a leg that asked only the
    /// second would run a hook the arms already ran.
    pub(super) fn scope_end_hook(
        &self,
        function: &Function,
        v: u16,
        data: &Data,
        arm_dropped: &HashSet<u16>,
    ) -> Option<Value> {
        if arm_dropped.contains(&v) {
            return None;
        }
        self.scope_end_drop(function, v, data)
    }

    /// [`Self::displaced_drop`] for a statement-level `OpDatabase(v, tp)` that REBUILDS a
    /// local's record in place.  A construction of a local not yet in scope displaces
    /// nothing; one of a local already in scope is asked the owner question — a first
    /// build after the declaration's null placeholder owns nothing yet, and the snapshot
    /// is null-safe, so a first loop iteration releases nothing either.
    /// `@FR-H-Drop`, the reassignment clause, for a VECTOR local: the literal backings a rebind
    /// of `v` displaces, each released through its hook and freed, and set to the sentinel so no
    /// later sweep reaches it again — or nothing, where the statement is not such a rebind.
    ///
    /// Every spelling of a vector rebind (a literal, a call, a copy, `[]`) points `v` at a
    /// backing of its own and leaves the one it held to its scope-end sweep.  The candidates are
    /// every literal backing `v` is bound to anywhere ([`vector_literal_backings`]) except the
    /// new one: the one it holds now is live, and any other is already the sentinel, which the
    /// guard skips.  A candidate handed off on every path releases nothing here, as at its
    /// scope end ([`Self::scope_end_drop`]).  The caller places the result at the statement's
    /// END — after the new value is built, and before anything after the statement runs.
    pub(super) fn vector_rebind_release(
        &self,
        stmt: &Value,
        function: &Function,
        data: &Data,
    ) -> Vec<Value> {
        let Value::Set(ov, rhs) = stmt.unspan() else {
            return Vec::new();
        };
        let v = *self.var_mapping.get(ov).unwrap_or(ov);
        if !self.var_scope.contains_key(&v)
            || !matches!(function.tp(v).base(), Type::Vector(_, _))
            || function.is_argument(v)
            || function.is_captured(v)
            || function.is_compiler_generated(v)
        {
            return Vec::new();
        }
        let Some(candidates) = self.vector_backings.get(ov) else {
            return Vec::new();
        };
        let new_backing = literal_backing_of(rhs, function, data);
        let mut out = Vec::new();
        for &b in candidates {
            if Some(b) == new_backing || !self.var_scope.contains_key(&b) {
                continue;
            }
            let Some(release) = self.scope_end_drop(function, b, data) else {
                continue;
            };
            out.push(release);
            out.push(call("OpFreeRef", b, data));
            out.push(v_set(
                b,
                Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new()),
            ));
        }
        out
    }

    /// `@FR-H-Drop`, the reassignment clause, for a vector local that may hold a CALL's result:
    /// `(before, after)` the rebinding statement, or `None` where it is not such a rebind.
    ///
    /// A call-delivered vector is released through the local that names it (`drop_hook`'s
    /// delivered binding), so once the local names the new value nothing names the old one.
    /// Before the statement, a local whose value lives in one of its call buffers (store
    /// identity) is aliased into a displaced temp.  Where the statement's own call is handed that
    /// same buffer, the buffer is detached, so the callee mints a fresh one rather than clearing
    /// the elements still owed their release.  After the new value, the temp's elements are
    /// released, every buffer sharing its store is set to the sentinel (the sweep would free it
    /// again), and the store is freed.
    pub(super) fn vector_call_rebind(
        &mut self,
        stmt: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Vec<Value>)> {
        let Value::Set(ov, rhs) = stmt.unspan() else {
            return None;
        };
        let v = *self.var_mapping.get(ov).unwrap_or(ov);
        let Type::Vector(elem, _) = function.tp(v).base().clone() else {
            return None;
        };
        let nr = data.drop_cascade_nr(data.collection_def_nr(&elem));
        if nr == u32::MAX || function.is_captured(v) || function.is_compiler_generated(v) {
            return None;
        }
        if function.is_argument(v) || !self.var_scope.contains_key(&v) {
            return None;
        }
        // The census names the buffers `v` is bound through; the value can live in any buffer of
        // its type, because a call that reads its own destination rotates two buffers
        // (`OpPutRef`) to keep them apart.  Every one of them is asked, since a buffer left
        // naming the store released here would hand that store to the next call.
        if self.vector_call_buffers.get(ov).is_none_or(Vec::is_empty) {
            return None;
        }
        let want = function.tp(v).base().without_deps();
        let buffers: Vec<u16> = (0..function.count())
            .filter(|&b| {
                function.name(b).starts_with("__ref_")
                    && self.var_scope.contains_key(&b)
                    && function.tp(b).base().without_deps() == want
            })
            .collect();
        if buffers.is_empty() {
            return None;
        }
        let incoming = call_buffer_of(rhs, function);
        let tp = function.tp(v).base().without_deps();
        self.lift_counter += 1;
        let tmp = function.add_temp_var(&format!("__vdisp_{}", self.lift_counter), &tp);
        self.var_scope.insert(tmp, self.scope);
        self.var_order.push(tmp);
        let sentinel = || Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new());
        let distinct = |a: u16, b: u16| {
            Value::Call(
                data.def_nr("OpDistinctStore"),
                vec![Value::Var(a), Value::Var(b)],
            )
        };
        let mut pre = vec![v_set(tmp, sentinel())];
        for &b in &buffers {
            let mut take = vec![v_set(
                tmp,
                Value::Call(data.def_nr("OpRefAlias"), vec![Value::Var(v)]),
            )];
            if Some(b) == incoming {
                take.push(v_set(b, sentinel()));
            }
            pre.push(v_if(distinct(v, b), Value::Null, Value::Insert(take)));
        }
        let live = Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(tmp)]);
        let mut post = Vec::new();
        // A detached buffer is handed back the store its callee minted instead, so the next
        // rebind finds the local's value in it and the sweep frees it.
        if let Some(b) = incoming {
            post.push(v_if(
                Value::Call(data.def_nr("OpRefIsNull"), vec![Value::Var(b)]),
                v_set(
                    b,
                    Value::Call(data.def_nr("OpRefAlias"), vec![Value::Var(v)]),
                ),
                Value::Null,
            ));
        }
        post.push(v_if(
            live,
            Value::Call(nr, vec![Value::Var(tmp)]),
            Value::Null,
        ));
        for &b in &buffers {
            post.push(v_if(distinct(b, tmp), Value::Null, v_set(b, sentinel())));
        }
        post.push(call("OpFreeRef", tmp, data));
        post.push(v_set(tmp, sentinel()));
        Some((pre, post))
    }

    /// `@FR-L-CapOwn` for a collection captured by a record built in a LOOP: the statements
    /// that detach the frame's backing from the store the record just adopted (loft#1610).
    ///
    /// A loop-body vector's backing (`__vdb_N`) is minted once, at the function's head, and
    /// refilled on every pass.  A record that ADOPTS the capture takes that store over, so from
    /// the build on the store is the record's and the backing no longer names anything the
    /// frame owns.  Left naming it, the next pass's literal cleared and refilled the store the
    /// previous pass's record still held, which then released the NEW pass's elements, and the
    /// frame released the last pass's again.  Set to the sentinel, the next literal mints a
    /// fresh store and every frame release of the backing finds nothing.  A record that BORROWS
    /// (one confined to the pass, `CaptureBuilds::pass_confined`) leaves the store the frame's,
    /// and is not named by [`owning_record_locals`].
    pub(super) fn adopted_backing_detach(
        &self,
        stmt: &Value,
        function: &Function,
        data: &Data,
    ) -> Vec<Value> {
        let set_dbref = data.def_nr("OpSetDbRef");
        let builds = &self.capture_build_backing;
        let mut out = Vec::new();
        stmt.walk(&mut |n| {
            let Value::Call(d, args) = n.unspan() else {
                return;
            };
            if *d != set_dbref {
                return;
            }
            let (Some(Value::Var(rec)), Some(off), Some(Value::Var(x))) = (
                args.first().map(Value::unspan),
                args.get(1),
                args.get(2).map(Value::unspan),
            ) else {
                return;
            };
            // Only a record with a drop cascade releases what it adopted when its rebuild
            // displaces it (`displaced_drop`'s snapshot: the hook, then the store).  A record
            // with nothing to drop is rebuilt in place and frees nothing, so there the frame's
            // holder stays the store's release — detached, the store would leak.
            let cascade = function
                .tp(*rec)
                .base()
                .heap_def_nr()
                .is_some_and(|r| data.drop_cascade_nr(r) != u32::MAX);
            if !cascade || !builds.rebuilt_in_loop.contains(x) {
                return;
            }
            // A collection capture: a view over the literal's backing, which is the store.
            // A struct capture: the pooled buffer a call delivered it through, which the next
            // pass's call clears and refills.
            let holders: Vec<u16> = match builds.backing.get(x) {
                Some(&b) => {
                    if function.name(b).starts_with("__vdb_")
                        && owning_record_locals(data, function, self.d_nr, builds, b).contains(rec)
                    {
                        vec![b]
                    } else {
                        Vec::new()
                    }
                }
                None => {
                    if owning_record_locals(data, function, self.d_nr, builds, *x).contains(rec) {
                        self.witness_buffer.get(x).cloned().unwrap_or_default()
                    } else {
                        Vec::new()
                    }
                }
            };
            // The holder lets go only while it still names the store the record adopted, read
            // back out of the record's own capture field.  The detach runs after the WHOLE
            // statement, and a build inside the statement that rebinds the capture
            // (`v = build_v(|i| v[0] + i)`) leaves the holder naming the NEW value by then.
            for b in holders {
                let adopted = Value::Call(
                    data.def_nr("OpGetDbRef"),
                    vec![Value::Var(*rec), off.clone()],
                );
                let distinct =
                    Value::Call(data.def_nr("OpDistinctStore"), vec![Value::Var(b), adopted]);
                let sentinel = Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new());
                let detach = v_if(distinct, Value::Null, v_set(b, sentinel));
                if !out.contains(&detach) {
                    out.push(detach);
                }
            }
        });
        out
    }

    /// `@FR-H-Drop`, the reassignment clause, for a vector local promoted onto the RETURN
    /// BUFFER: `(before, after, waits)` a statement that refills it, or `None`.
    ///
    /// The promoted local IS the caller's buffer, so every rebind refills that one store in
    /// place — a call copies into it or is handed it, a literal or `[]` clears it
    /// (`OpClearVector`) and appends — and the old elements go with no hook (`(H-Drop-Not)`).
    /// They are copied into a fresh vector before the statement (`OpAppendVector` takes their
    /// heap along) and released from the copy after the new value, which `waits` for the
    /// statement's end when the statement is a clear that its literal fills after.  The FIRST
    /// statement to reach the buffer is its initial fill and displaces nothing: the buffer a
    /// caller hands in is emptied at entry and may still hold what the caller already released.
    pub(super) fn promoted_vector_refill(
        &mut self,
        stmt: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Vec<Value>, bool)> {
        let (v, waits) = match stmt.unspan() {
            Value::Set(v, _) => (*self.var_mapping.get(v).unwrap_or(v), false),
            Value::Call(d, args) if *d == data.def_nr("OpClearVector") => {
                let Some(Value::Var(v)) = args.first().map(Value::unspan) else {
                    return None;
                };
                (*v, true)
            }
            _ => return None,
        };
        let Type::Vector(elem, _) = function.tp(v).base().clone() else {
            return None;
        };
        if !function.is_argument(v) || !self.is_promoted_ret_buffer(function, data, v) {
            return None;
        }
        let nr = data.drop_cascade_nr(data.collection_def_nr(&elem));
        if nr == u32::MAX || self.promoted_filled.insert(v) {
            return None;
        }
        let elem_kt = data.def(elem.heap_def_nr()?).known_type();
        let tp = function.tp(v).base().without_deps();
        self.lift_counter += 1;
        let tmp = function.add_temp_var(&format!("__vsnap_{}", self.lift_counter), &tp);
        self.var_scope.insert(tmp, self.scope);
        self.var_order.push(tmp);
        let live = |x: u16| Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(x)]);
        let pre = vec![
            v_set(tmp, Value::Null),
            v_if(
                live(v),
                Value::Call(
                    data.def_nr("OpAppendVector"),
                    vec![
                        Value::Var(tmp),
                        Value::Var(v),
                        Value::Int(i32::from(elem_kt)),
                    ],
                ),
                Value::Null,
            ),
        ];
        let post = vec![
            v_if(
                live(tmp),
                Value::Call(nr, vec![Value::Var(tmp)]),
                Value::Null,
            ),
            call("OpFreeRef", tmp, data),
            v_set(
                tmp,
                Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new()),
            ),
        ];
        Some((pre, post, waits))
    }

    pub(super) fn in_place_rebuild(
        &mut self,
        stmt: &Value,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Vec<Value>)> {
        // A literal into a local promoted onto the return buffer rebuilds behind a guard — a
        // record the buffer already holds is written in place, an absent one is minted
        // (`parse_object`, @PLN157 § V-d) — and both replace what the local held.
        let stmt = match stmt.unspan() {
            Value::If(_, kept, rebuilt) if matches!(kept.unspan(), Value::Null) => rebuilt,
            _ => stmt,
        };
        let Value::Call(d, args) = stmt.unspan() else {
            return None;
        };
        if *d != data.def_nr("OpDatabase") {
            return None;
        }
        let Some(Value::Var(ov)) = args.first().map(Value::unspan) else {
            return None;
        };
        let v = *self.var_mapping.get(ov).unwrap_or(ov);
        if !self.var_scope.contains_key(&v) {
            return None;
        }
        // A construction OWNS what it builds, on every iteration.
        let mut ops = self.displaced_drop(v, true, function, data);
        if self.var_scope.get(&v) == Some(&self.scope) {
            self.drop_transferred.remove(&v);
        }
        // …and what it built is the latest assignment's record, so the next reassignment
        // displaces a record this frame owns (@FR-O-Latest).  A plain local records this at
        // its first `Set`; a local promoted onto the return buffer is first built by this
        // guarded statement instead, and without the record `s = S {…}; s = S {…}` released
        // nothing for the record it displaced.  Recorded AFTER the snapshot above, which is
        // what keeps the caller's offered record at entry untouched.
        if matches!(
            function.tp(v).base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) {
            self.owned_refs.insert(v, self.loops.len());
        }
        // What the rebuilt store holds from here on is its own to release again, whatever an
        // earlier copy moved out of it — `@FR-O-Latest` for a store rebuilt in place, which is
        // no `Set` for `scan_set` to retire.  After the snapshot, which read the flag.
        if let Some(&flag) = self.handed_off.get(&v) {
            let reset = v_set(flag, Value::Boolean(false));
            ops = Some(match ops {
                Some((pre, mut post)) => {
                    post.push(reset);
                    (pre, post)
                }
                None => (Vec::new(), vec![reset]),
            });
        }
        ops
    }

    /// The IR that releases, through its type's hook, the record `v` is about to stop
    /// holding — `(before, after)` the displacing statement — or `None` where `v` owns no
    /// droppable record.  `@FR-H-Drop`: the reassignment clause.
    ///
    /// `OpDrop` runs *"when the value's OWNER dies"* (INTERFACES.md), and a reassignment is
    /// that death for the record it displaces: its store is freed (a displaced free) or
    /// rebuilt in place, and either way the resource it held is gone with no hook run.  A
    /// file opened into a local that is later reassigned was never closed (loft#1362).
    ///
    /// The release is taken on a SNAPSHOT: a fresh temp is deep-copied from `v` before the
    /// statement (null-safe — nothing is copied from an absent local), and the hook runs on
    /// the temp after it.  The copy is what makes the order free of hazards: an in-place
    /// rebuild (`s = S {…}` lowers to `OpDatabase(s, tp)` on the existing store) overwrites
    /// the old bytes before anything after the statement could read them, and a right-hand
    /// side that reads `v` must still see the resource live.  It is a copy of a record with
    /// a droppable member — a handle, not data — so its cost is where drops are.
    ///
    /// The owner predicate is the transition free's: `v` is in scope, its record is OWNED
    /// (the dep-empty proxy, `@FR-O-Override`'s never-free, the oracle's latest-assignment
    /// fact), its drop was not handed off on every path (`drop_transferred`; a hand-off made on
    /// some paths only is read off its flag when the snapshot is taken), it is no witnessed
    /// mixed-ownership local, no argument and no capture.  A view's record is somebody
    /// else's resource and is never released here — which is why, inside a LOOP, the
    /// latest-assignment fact from outside the loop is trusted only when THIS assignment
    /// owns too (`rhs_owned`): on the second iteration the displaced record is the one this
    /// statement built, and a view assigned here would otherwise be copied and released as
    /// if it were owned.
    pub(super) fn displaced_drop(
        &mut self,
        v: u16,
        rhs_owned: bool,
        function: &mut Function,
        data: &Data,
    ) -> Option<(Vec<Value>, Vec<Value>)> {
        let owned_here = match self.owned_refs.get(&v) {
            Some(depth) => *depth == self.loops.len() || rhs_owned,
            None => false,
        };
        // @FR-O-Proxy asks free — the hook is a release, and it follows only where the empty
        // dep list says `v` OWNS the record; the proxy carries its @FR-O-Override veto as one
        // question, so this reads the pair negated rather than two separate escape clauses.
        // D-heap-7 — the local `classify_ret_promotion` renamed onto the return buffer is an
        // argument by slot only: this frame minted what it holds, and its assignments displace
        // a record exactly as a plain local's do (`a = mk(1); a = mk(5); return a` never
        // released the first).
        if !owned_here
            || (function.is_argument(v) && !self.is_promoted_ret_buffer(function, data, v))
            || function.is_captured(v)
            || !function.proxy_says_owned(v)
            || self.drop_transferred.contains(&v)
            || self.owner_witness.contains_key(&v)
        {
            return None;
        }
        let d = function.tp(v).base().heap_def_nr()?;
        if data.drop_cascade_nr(d) == u32::MAX {
            return None;
        }
        let kt = data.def(d).known_type();
        let tp = function.tp(v).base().without_deps();
        self.lift_counter += 1;
        let name = format!("__disp_{}", self.lift_counter);
        let disp = function.add_temp_var(&name, &tp);
        function.mark_inline_ref(disp);
        // A CLOSURE record's snapshot is homed at the FUNCTION body (1), its null-init hoisted
        // there (`lift_vars`), as `__blk_N` is: the record is rebuilt inside its lambda's
        // `fn_ref_with_closure` block, whose value is the `FnRef` it ends in, and the sweep's
        // second visit (a no-op on the sentinel) made at that block's end stood after the value
        // (loft#1606: `()` on native, a garbage fn-ref on the interpreter).  Every other snapshot
        // keeps its statement's scope: homed at the function, a generator's would become a heap
        // temp its TAIL releases when a `match` drains it.
        let closure_record = function.name(v).starts_with("___clos_");
        self.var_scope
            .insert(disp, if closure_record { 1 } else { self.scope });
        self.var_order.push(disp);
        if closure_record {
            self.lift_vars.push(disp);
        }
        let live = Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(v)]);
        let snapshot = Value::Insert(vec![
            Value::Call(
                data.def_nr("OpDatabase"),
                vec![Value::Var(disp), Value::Int(i32::from(kt))],
            ),
            Value::Call(
                data.def_nr("OpCopyRecord"),
                vec![Value::Var(v), Value::Var(disp), Value::Int(i32::from(kt))],
            ),
        ]);
        // A source whose per-path copy ran has handed this record's resource to that copy
        // (loft#1515): no snapshot on that path, so the release after the statement finds
        // nothing.  The flag is read HERE, before the statement, because the same statement
        // resets it to `false` before that release runs.
        let take = v_if(live, snapshot, Value::Null);
        let take = match self.handed_off.get(&v) {
            Some(&flag) => v_if(Value::Var(flag), Value::Null, take),
            None => take,
        };
        let pre = vec![v_set(disp, Value::Null), take];
        let mut post = Vec::new();
        if let Some(hook) = drop_hook(function, disp, data) {
            post.push(hook);
        }
        post.push(call("OpFreeRef", disp, data));
        // Back to the TRUE sentinel: the sweep visits the temp again at scope end, and a
        // freed reference that still reads `rec != 0` would run the hook a second time on
        // whatever the allocator has since put in that slot.  On the sentinel both the
        // hook's liveness test and the sweep's free are no-ops.
        post.push(v_set(
            disp,
            Value::Call(data.def_nr("OpNullRefSentinel"), Vec::new()),
        ));
        Some((pre, post))
    }
}

/// @PLN125 arc B — the scope-end hook `v`'s type declares, as a call on `v`.
///
/// > **A drop runs exactly where the value's own `OpFree*` runs — the same binding, the
/// > same scope exit, the same early-exit paths — and never anywhere else.**
///
/// That is the whole design, and phrasing it that way is what makes it small: loft already
/// COMPUTES the fact.  The ownership model decides per binding whether this scope owns the
/// value and whether it dies here, which is what puts an `OpFreeRef` in this list; a
/// returned or borrowed value is already excluded, and the early-`return`, `break` and
/// return-out-of-a-loop paths are already handled here (loft#731 exists because a hand-
/// rolled version of exactly those went wrong).  So the drop DERIVES from the borrow model
/// rather than sitting beside it: there is one answer to "when does this run", not two that
/// can drift.
///
/// Scope is honest and narrow: a **binding this scope owns**.  A droppable that is a FIELD
/// of another record is released by that record's cascade, which is not this list, so it
/// does not fire — a hook that ran from two different mechanisms would be the drift this
/// design exists to avoid.
///
/// **The free is null-tolerant and a drop is not**, which is the one place "where the free
/// runs" needed sharpening.  `OpFreeRef` on a slot that was never written is a no-op — it
/// checks `rec == 0` and returns — so the emitter has never had to know whether a binding
/// actually holds anything.  A drop is a USER call, and running it on an unwritten slot
/// runs the author's rollback against a record that does not exist:
///
/// ```loft
/// if n > 0 { t = Tx { … } }     // the else path never writes `t`
/// ```
///
/// printed `[drop null]` before the guard.  So the call is wrapped in the same liveness
/// test the free performs internally (`OpConvBoolFromRef` IS `rec != 0`), which makes the
/// rule *where the free runs, on a value that exists*.  The same guard settles the aliasing
/// case for free: a caller-side `__ref_N` return buffer that the callee did not adopt is
/// null here and correctly does not fire, while one that WAS adopted never reaches this
/// branch at all (it takes the `OpFreeRefIfDistinct` pairing above).
pub(super) fn drop_hook(function: &Function, v: u16, data: &Data) -> Option<Value> {
    // @PLN139 — the CASCADE, not the bare hook: for a type that owns droppable members it
    // is the synthesized function that runs the type's own hook and then releases what it
    // owns, and for every other type it IS the bare hook (`Data::drop_cascade_nr`), so a
    // program with no containers is unchanged.
    let nr = match function.tp(v).base() {
        // A struct-enum binding is a heap record exactly as a `Reference` one is — it just
        // carries a discriminator at its head — so it drops the same way. Reading only
        // `Reference` here is why an enum's cascade was synthesized and then never called
        // (@PLN139 stage D).
        Type::Reference(d, _) | Type::Enum(d, true, _) => data.drop_cascade_nr(*d),
        // `@FR-H-Drop` / D-heap-13 (loft#1551) — and reading only those two is why a COLLECTION a call
        // answers released nothing: its backing is the callee's return buffer, typed as the
        // bare collection, so this gate turned it away.  `(H-Move)` makes `d = mkv()` a move
        // and `(H-Drop)` owes its elements a release at the owner's scope end.
        //
        // The cascade called is the COLLECTION's own (`collection_def_nr`), not the wrapper
        // record's, and that is measured rather than stylistic: handing the wrapper's
        // cascade this binding's DbRef releases on `--native` and silently does nothing on
        // `--interpret`, because a `vector<T>` binding's address is the wrapper record's on
        // native alone (`@1,8` there, `@1,12` here — the discrepancy
        // `Stores::clear_vector_release` carries, and why IT asks the store's SHAPE rather
        // than an offset).  A collection cascade walks `self`, so one IR releases on both.
        Type::Vector(elem, _) => data.drop_cascade_nr(data.collection_def_nr(elem)),
        // loft#1601, @FR-G-Hold — a keyed collection whose records hold a generator releases
        // their frames through its type's walk; its records run no hook (`(H-Drop-Not)`).
        keyed if data.keyed_holds_generator(keyed) => data.def_nr(&data.keyed_frames_name(keyed)),
        _ => return None,
    };
    if nr == u32::MAX {
        return None;
    }
    // D-heap-13 — the cascade runs over the BINDING the buffer delivered to, where there is
    // one.  `(H-Drop)` names the owner, and for a collection a call answers that is the
    // author's local: `(H-Move)` places the fresh result where it is produced.  It also has
    // to be the local for the release to happen at all on the INTERPRETER, and that is
    // measured: `OpDatabase`'s reuse arm claims a FRESH record in the cleared store there
    // while native re-establishes record 1, so after the callee fills it the caller's buffer
    // variable still names the record it held before the call — empty — and the binding names
    // the delivered one.  On native the two are one address, so the subject changes nothing.
    let subject = match function.tp(v).base() {
        // The collection arm releases through the BINDING and only where there is one: a
        // buffer nobody bound handed its value somewhere that releases it (a field, an
        // argument), and cascading over the buffer as well releases twice — measured on
        // `b.v = mkv()`, which doubles on `--native` where the field's place IS the buffer.
        Type::Vector(_, _) => delivered_binding(function, v)?,
        _ => v,
    };
    let live = Value::Call(data.def_nr("OpConvBoolFromRef"), vec![Value::Var(subject)]);
    Some(Value::If(
        Box::new(live),
        Box::new(Value::Call(nr, vec![Value::Var(subject)])),
        Box::new(Value::Null),
    ))
}

/// `@FR-H-Drop` / D-heap-13 — the local a return BUFFER delivered its collection to: the one
/// whose type borrows `buffer` and is a collection itself.  `(H-Move)` places a fresh call
/// result where it is produced, so that local is the owner whose scope end the hook belongs to.
///
/// `None` unless there is exactly one.  Two bindings naming one buffer cannot both be the
/// owner `(H-Drop)` asks for, and releasing through a guess would double a drop — which
/// `(H-Drop)` rules out more firmly than it rules out losing one — so the ambiguous case
/// keeps the buffer as the subject and behaves as it did before.
fn delivered_binding(function: &Function, buffer: u16) -> Option<u16> {
    // A hidden RETURN buffer only (`__ref_N`).  A local's own `__vdb_N` backing already
    // releases through itself, and redirecting THAT onto a binding that borrows it releases
    // twice — measured on `d = mkv(); e = d`, the shape D-heap-13 warns is not a control.
    if !function.name(buffer).starts_with("__ref") {
        return None;
    }
    let mut found = None;
    for x in 0..function.count() {
        if x == buffer
            || !matches!(function.tp(x).base(), Type::Vector(_, _))
            || !function.tp(x).depend().contains(&buffer)
        {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(x);
    }
    found
}
