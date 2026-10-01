// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! INLINE call results: whether a call's struct or collection result is fresh and owned, so the
//! argument or arm that holds it can be lifted into a temp that frees it.

use super::Scopes;
use crate::data::{Block, Data, DefType, Deps, Type, Value};
use crate::variables::Function;

impl Scopes<'_> {
    /// EXPERIMENT (D-clo-14) — the dep list a lifted collection `Join` return carries: it
    /// NAMES the caller variable the return may still be aliasing, which is what makes the
    /// owner decidable at run time by store identity.  `Deps::none()` (owned) otherwise.
    fn lift_deps(base_witness: u16) -> Deps {
        if base_witness == u16::MAX {
            Deps::none()
        } else {
            Deps::frame1(base_witness)
        }
    }

    /// loft#1176 — does a monomorph whose tail is a call THROUGH A FN-REF hand back a
    /// store this caller must own?
    ///
    /// [`Definition::monomorph_return_is_fresh`] reads the callee's body and answers
    /// `false` for a tail like `f(x)`: what comes back is the fn-ref target's fact, and
    /// inside the monomorph `f` is a runtime value with no definition to ask.  The fact
    /// is not unreachable, only unreachable from THERE — at this call site the caller
    /// wrote which closure it passed, so `fnref_target` resolves it and the same
    /// body-shaped question is put to the definition that actually runs.
    ///
    /// **The resolved target must pass BOTH ownership reads, and the pair is not
    /// redundant.**  `returns_borrowed_view` is the deps proxy and catches a lambda
    /// handing back its own PARAMETER; it does not catch one handing back a CAPTURE,
    /// because the dep then names the hidden `__closure` attribute and a hidden attr is
    /// read as "not a borrow" (loft#1114).  `monomorph_return_is_fresh` is what closes
    /// that: the capture arrives as a place read rooted at `__closure`, which is an
    /// argument, so the body-shaped proof refuses it.  Measured — the concrete twin
    /// `fn once_p(x: P, f: fn(P) -> P) -> P { f(x) }` lifted on the deps proxy alone and
    /// CORRUPTED the captured record on both backends (`formal/closures.md` D-clo-9);
    /// this route declines that same shape.
    ///
    /// Every unresolved position answers `false`, which leaves the leak that was already
    /// there.  That is the direction this whole gate takes when it cannot name what it
    /// would be freeing: a wrong `true` frees a store the caller still holds.
    /// A monomorph whose return sites are direct CALLS delivers a fresh store when every
    /// one of those callees does — loft#1273.
    ///
    /// `fn add<T: Addable>(a: T, b: T) -> T { a + b }` lowers its tail to
    /// `Call(n_OpAdd, …)`, and the user's `OpAdd` mints a record. `monomorph_return_is_fresh`
    /// cannot see that — a callee's ownership is not a fact this body carries — so the
    /// result was never lifted and one record was retained per call, unbounded in a loop,
    /// while the bound spelling (`r = add(…); r.v`) was clean all along.
    ///
    /// The three questions are the fn-ref twin's, for the same reasons: the target must have
    /// a BODY, must not return a borrowed view — a callee handing back its own argument
    /// would make the lift free the caller's record — and must itself be proven fresh, so
    /// the proof stays positive and one unreadable link refuses the chain.
    ///
    /// A delegate that is itself an instance is asked the same question, so a generic that
    /// forwards to another generic (`diff2<T>(a, b) { diff(a, b) }`) is decided by the chain
    /// rather than refused at its first link.  [`DELEGATION_DEPTH`] is the cycle guard mutual
    /// recursion needs: past it the answer is `false`, the direction every gate here takes
    /// when it cannot name what it would be freeing.
    /// No `self`: unlike the fn-ref twin, which resolves a closure through the caller's
    /// `fnref_target`, the target here is written in the IR and only `Data` is needed.
    ///
    /// A target that returns a BORROW of one of its parameters is fresh here too, where the
    /// instance COPIES what it is handed (loft#1820).  An instance whose published return
    /// carries no dep hoists a call tail into a `__ret_N` typed with that return, and a
    /// record `Set` into an owned temp copies a borrowed source.  The hoist happens whenever
    /// free ops follow the tail, and a target taking a `__retbuf` guarantees one: the
    /// instance passes its own work ref and releases it after the call.  So `diff(p, q)`
    /// over `fn OpMin(self: P, o: P?) -> P { self }` hands its caller a copy of `p`.  Declined,
    /// that copy was owned by nobody, one record per inline call, while `(p - q).x` — which
    /// hands back `p` itself and is read as the borrow it is — was clean.
    fn monomorph_delegated_return_is_fresh(data: &Data, def: &crate::data::Definition) -> bool {
        Self::delegated_return_is_fresh_at(data, def, 0)
    }

    fn delegated_return_is_fresh_at(
        data: &Data,
        def: &crate::data::Definition,
        depth: usize,
    ) -> bool {
        let null_ref = data.def_nr("OpNullRefSentinel");
        let Some(targets) = def.monomorph_direct_call_return_targets(null_ref) else {
            return false;
        };
        let instance_copies = def.returned.depend().is_empty();
        targets.iter().all(|&d_nr| {
            if d_nr as usize >= data.definitions() as usize {
                return false;
            }
            let target = data.def(d_nr);
            target.code != Value::Null
                && if target.returns_borrowed_view() {
                    instance_copies && target.attr_names.contains_key("__retbuf")
                } else {
                    target.monomorph_return_is_fresh(null_ref)
                        || (target.is_instance()
                            && depth < DELEGATION_DEPTH
                            && Self::delegated_return_is_fresh_at(data, target, depth + 1))
                }
        })
    }

    fn monomorph_fnref_return_is_fresh(
        &self,
        val: &Value,
        data: &Data,
        def: &crate::data::Definition,
    ) -> bool {
        let null_ref = data.def_nr("OpNullRefSentinel");
        let Some(slots) = def.monomorph_fnref_return_slots(null_ref) else {
            return false;
        };
        let Value::Call(_, args) = val.unspan() else {
            return false;
        };
        slots.iter().all(|&slot| {
            // A parameter's variable slot IS its argument position; a hidden argument the
            // lowering appends sits after every visible one, so a slot in range names the
            // value the caller wrote.
            let Some(arg) = args.get(slot as usize) else {
                return false;
            };
            let Value::Var(fn_var) = arg.unspan() else {
                return false;
            };
            let target = match self.fnref_target.get(fn_var).copied() {
                Some(d) if d != u32::MAX => data.def(d),
                _ => return false,
            };
            target.code != Value::Null
                && !target.returns_borrowed_view()
                && target.monomorph_return_is_fresh(null_ref)
        })
    }

    /// Check whether a scanned argument at position `arg_idx` is an inline
    /// struct-returning call that needs lifting to a temporary variable.
    /// Returns the struct definition number if lifting is needed, None
    /// otherwise.
    ///
    /// Skips lifting when the outer call's return type depends on this argument
    /// (i.e. the result borrows from the argument's store).  Freeing the lifted
    /// temp at scope exit would be use-after-free in that case.
    /// loft#721 — may the aggregate a `CallRef` returns be LIFTED into an owned
    /// temp (and therefore freed)?
    ///
    /// The direct-call branch answers this from the callee's definition; a
    /// `CallRef`'s callee is a runtime value, so the fn-ref TYPE is all that is
    /// available at the call — and it is not enough. Measured: a closure that
    /// calls a struct-returning function and one that hands back a borrowed
    /// element present the SAME signature (`ret_deps=[1]`, one param), because
    /// the dep index lives in the callee's attribute space and says nothing on
    /// its own. Lifting on the type alone frees a borrowed element: a
    /// use-after-free that only `LOFT_POISON=1` makes visible.
    ///
    /// So resolve the target definition instead — `fnref_target` records which
    /// definition each fn-ref variable was assigned — and then ask the SAME
    /// canonical question the direct call asks, `returns_borrowed_view()`.
    /// An unknown or ambiguous target answers `None`: the result is not lifted,
    /// which is the pre-existing behaviour (it leaks, and a leak is recoverable
    /// where a premature free is not).
    fn callref_owned_return(
        &self,
        val: &Value,
        data: &Data,
        function: &Function,
        outer_call: u32,
    ) -> Option<Type> {
        let Value::CallRef(v_nr, _) = val.unspan() else {
            return None;
        };
        let d_nr = match self.fnref_target.get(v_nr).copied() {
            Some(d) if d != u32::MAX => d,
            // loft#1659 — an UNRESOLVED target (a fn-typed parameter, a slot two lambdas were
            // assigned to).  Nothing static says whose store its record result is
            // (@FR-O-Unknown), and not lifting left it to the frame's hand-up list: one
            // store per call, held to frame exit.  The lift binds it the way the bound
            // spelling does, through `OpBindFnRefResult` — the call's own return decides
            // adopt (minted during the call) or copy (a capture or an argument) — so the
            // temp OWNS on both arms and its scope-exit free is the release (@FR-O-Owner).
            // Records only: `opaque_callref_bind` is the bind that makes that free right,
            // and it answers only for a record.
            _ => {
                let Type::Function(_, ret, ..) = function.tp(*v_nr).base() else {
                    return None;
                };
                let (returned, opt) = ret.peel_optional();
                self.pending_join_witness.set(u16::MAX);
                return match returned {
                    Type::Reference(d, _) => {
                        Some(Self::reopt(opt, Type::Reference(*d, Deps::none())))
                    }
                    Type::Enum(d, true, _) => {
                        Some(Self::reopt(opt, Type::Enum(*d, true, Deps::none())))
                    }
                    _ => None,
                };
            }
        };
        let def = data.def(d_nr);
        // loft#1245 — the INVARIANT: a call's returned store is ADOPTED by the caller only
        // where the callee minted it, and COPIED otherwise.  Which of the two happened is a
        // runtime fact for a `??` return, so the @P290 bracket decides it per execution —
        // and for that it needs a witness for every place the return could borrow FROM.
        //
        // A fn-ref can borrow from two places, and only one of them is witnessable: its
        // ARGUMENTS (which `protectable_ref_args` names) and its CAPTURES (which nothing at
        // the call site can name).  So the lift is admissible exactly when the witness set
        // is COMPLETE and the fn-ref captures nothing.
        if def.code == Value::Null {
            return None;
        }
        // `returns_borrowed_view()` is the deps PROXY (@FR-O-Proxy), and for a closure whose
        // return is a `??` it is right about one arm and wrong about the other: the dep names
        // the parameter or the capture the SUBJECT arm hands back, so the whole return reads
        // as a borrow and the DEFAULT arm's minted store is left owned by nobody — one store
        // per call, to frame exit, which a loop turns into the 65535-store ceiling
        // (loft#1248).
        //
        // @FR-O-Oracle is the authority, and this is the chokepoint that should read it —
        // the same sentence the direct-call branch below already acts on.  A `Join` lifts
        // only where the bind that follows is the runtime guard `OpBindOrCopy`: the witness
        // has to be NAMEABLE, and the statement has to be one that BINDS.  `outer_call ==
        // u32::MAX` is the bare-statement lowering, which gets no such bind, so there the
        // conservative no-lift stands and costs the mint arm's leak instead.
        //
        // ⚠ The relaxation reaches exactly as far as the GUARD does, and no further.  The
        // arms below also answer for the collection returns, and a lifted collection gets a
        // scope-exit free with nothing deciding per execution whether the store is the
        // caller's — `callref_join_first_bind`, which is what emits `OpBindOrCopy` on both
        // backends, answers only for a `Reference` / record `Enum`.  Widening past that
        // would trade this leak for the over-free in the other direction, which is the
        // trade the whole gate exists to refuse.
        let record_return = matches!(
            def.returned().peel_optional().0,
            Type::Reference(_, _) | Type::Enum(_, true, _)
        );
        let own = crate::use_analysis::ownership_of(data, self.d_nr, val);
        let join_lifts = record_return
            && matches!(own, crate::use_analysis::Own::Join { base }
                if base != u16::MAX && outer_call != u32::MAX);
        // A fn-ref borrows from two places and only one of them is witnessable at the call:
        // its ARGUMENTS, which `protectable_ref_args` names, and its CAPTURES, which nothing
        // at the call site can.  So a lift is admissible two ways, and needing EITHER is what
        // keeps both halves closed at once:
        //
        //   * `join_lifts` — the oracle named a WITNESS and the statement BINDS, so
        //     `OpBindOrCopy` decides per execution whose store it is (loft#1248);
        //   * `witnessed_lifts` — the witness set is COMPLETE and the fn-ref captures
        //     nothing, so there is nothing borrowed for the lift to free (loft#1245).
        //
        // The capture test is separate and unconditional because `returns_borrowed_view()`
        // is FALSE for one: a capture reaches the return through `__closure`, a hidden
        // attribute, and a hidden-only dep otherwise reads as *"the callee minted this"*.
        // A complete witness set says nothing there either — it reads complete VACUOUSLY for
        // a call whose arguments are all scalars, having witnessed nothing.
        // loft#1248's decline, narrowed to the capture that cannot be RESOLVED: the callee's
        // body names the offset its `??` subject reads, and one offset over a variable assigned
        // once is a witness as good as an argument's (D-clo-7).  A collection's chosen arm is
        // COPIED into `__retbuf`, so a capture-reading collection closure returns an owned
        // store and takes the witnessed route like any other.
        let blocks = crate::use_analysis::callref_capture_blocks(data, self.d_nr, val);
        // …and the witnessed route reaches exactly as far as a GUARDED release does.  The
        // bracket refuses the SOURCE-free of a record the callee handed back
        // (`do_copy_record`), so a record temp holds its own store by the time the scope-exit
        // free runs; a collection `Join` gets the identity free below, which names the base.
        // A collection the callee answers as a raw VIEW of its argument (a keyed field, an
        // index read — `Own::Borrowed`) has neither: the lift's `OpFreeRef` releases whatever
        // store the temp names, and that store is the caller's — `t = h(bag)` with
        // `h = fn(q) { q.m }` emptied `bag.m` after one call, both backends, where the release
        // still answered correctly.
        let collection_join = !record_return
            && matches!(own, crate::use_analysis::Own::Join { base } if base != u16::MAX);
        let witnessed_lifts = !blocks
            && (record_return || collection_join)
            && crate::use_analysis::protectable_ref_args(data, self.d_nr, val).1;
        if blocks && !join_lifts {
            return None;
        }
        if def.returns_borrowed_view() && !join_lifts && !witnessed_lifts {
            return None;
        }
        // loft#1257 — and the collection arms need the oracle for the OPPOSITE reason: to
        // stop lifting, not to start.  A collection return is delivered through a HIDDEN
        // buffer, so its dep names only hidden attributes and `returns_borrowed_view()`
        // answers false — *"the callee minted into its own buffer, the caller adopts"*.
        // Right when the closure mints, wrong when its `??` hands back the caller's
        // argument, and the proxy cannot tell those apart because they are the same call:
        // `fn(q: vector<integer>?) -> vector<integer> { q ?? [7, 8] }` reached the lift, and
        // the lifted temp then EMPTIED the caller's own vector — `len(some)` reached 0 after
        // five iterations, with nothing saying so.
        //
        // A `Join` whose base the bracket can NAME is exactly *"this may be that caller
        // variable"*.  The `Reference` / `Enum` arms may still lift it, because
        // `OpBindOrCopy` settles it per execution; a collection has no such guard, so here
        // the answer is to decline.
        //
        // ⚠ THIS IS A TRADE AND THE COST IS MEASURED: the MINT arm of the same closure goes
        // back to leaking one store per call (peak 4 -> 403 at N=400), which at scale is a
        // store-table abort.  Taken deliberately — a leak announces itself and a container
        // silently emptied does not, which is why `silent-wrong` outranks `sev:`
        // (`.github/LABELS.md`).  It costs only the JOIN shape: a pure mint classifies
        // `Owned`, or `Borrowed` of a hidden buffer with no nameable base, and loft#1177's
        // cells are all pure mints and keep their lift.
        //
        // The closure is a WITNESSED lift, and `OpFreeRefIfDistinct` is the right shape for
        // it — built and measured here.  It fixes `--native` and leaves the interpreter
        // wrong, because on that side the damage is not the free but the RE-SET: one
        // iteration is correct, two are not, so the transition-free on `__lift_N`'s
        // reassignment releases the borrowed store before any scope-exit free runs.  Both
        // halves are needed, and only the decline is correct on both backends today.
        //
        // The fn-ref variable's own type is the declared shape; the definition is
        // the authority on what it returns.
        let _ = function;
        let mut base_witness = u16::MAX;
        let (returned, opt) = def.returned().peel_optional();
        if !matches!(returned, Type::Reference(_, _) | Type::Enum(_, true, _))
            && let crate::use_analysis::Own::Join { base } =
                crate::use_analysis::ownership_of(data, self.d_nr, val)
            && base != u16::MAX
        {
            // loft#1257 — the IDENTITY route, and the reason this is no longer a decline.
            // Declining cost the mint arm one store per call (389 live at N=400, a
            // store-table abort at scale).  @FR-O-Oracle already says what a `Join` means at
            // run time — *"adopt iff the value's store ≠ base's store"* — and the dep NAMES
            // that base, so the owner is decidable by store IDENTITY with no witness slot.
            // `ownership.md` D-own-16 closed the same sentence one shape over.
            //
            // The base rides on the temp's TYPE (`lift_deps` below), which does both halves at
            // once: a non-empty dep keeps `state/codegen.rs`'s unconditional pre-Set free from
            // being emitted at all — the RE-SET that left the interpreter wrong when an earlier
            // attempt guarded only the scope-exit free — and `get_free_vars` then emits
            // `OpFreeRefIfDistinct(__lift_N, base)` there.  One guarded free per evaluation.
            if !crate::keys::lift_join_witness_enabled() {
                return None;
            }
            base_witness = base;
        }
        // The witness is handed to the next `new_lift_var` — and ONLY where an arm below
        // actually answers with a type, so a return that lifts nothing cannot leave it
        // standing for an unrelated temp.
        let lifted = match returned {
            Type::Reference(d, _) => Some(Self::reopt(opt, Type::Reference(*d, Deps::none()))),
            Type::Enum(d, true, _) => Some(Self::reopt(opt, Type::Enum(*d, true, Deps::none()))),
            // loft#1177 — a COLLECTION return is the same question with the same answer, and
            // it was missing: the arms named the two aggregate shapes a closure was known to
            // return and `_ => None` read as *"nothing else needs owning"*, which a
            // store-backed collection contradicts.  A lambda handing back a `vector` / keyed
            // collection used INLINE (`len(g(7))`) therefore had its store owned by nothing —
            // one leaked record per call, where the bound form `r = g(7)` was always clean.
            // The dep list is rebuilt empty for the same reason the two arms above rebuild
            // theirs: `returns_borrowed_view` has already refused a callee that hands back a
            // view, so what reaches here is a store the caller must own.
            Type::Vector(inner, _) => Some(Self::reopt(
                opt,
                Type::Vector(inner.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Hash(d, k, _) => Some(Self::reopt(
                opt,
                Type::Hash(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Sorted(d, k, _) => Some(Self::reopt(
                opt,
                Type::Sorted(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Index(d, k, _) => Some(Self::reopt(
                opt,
                Type::Index(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Radix(d, k, _) => Some(Self::reopt(
                opt,
                Type::Radix(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            Type::Trie(d, k, _) => Some(Self::reopt(
                opt,
                Type::Trie(*d, k.clone(), Self::lift_deps(base_witness)),
            )),
            // Everything else is a value the caller does not own a store for — a scalar
            // lives in the slot, and a `text` is freed by its own delivery path.
            _ => None,
        };
        self.pending_join_witness.set(if lifted.is_some() {
            base_witness
        } else {
            u16::MAX
        });
        lifted
    }

    /// loft#879 — the shape question `inline_struct_return` asks ("does this call
    /// hand back a store the caller must own, and of what shape?") is about the
    /// BASE type: `Optional(τ)` is a compile-time wrapper over τ's own runtime
    /// layout (@PLN25), so `-> C?` allocates and delivers exactly what `-> C`
    /// does.  Matching the arms below on the unpeeled type therefore answered
    /// "not liftable" for every optional aggregate return, and the result got a
    /// bare stack-pop (`FreeStack`) that never freed the store — one leaked
    /// record per call, unbounded in a loop, on the interpreter.
    ///
    /// The arms peel for the match; this puts the wrapper back on the temp's
    /// type, so a lifted temp is typed exactly like the hand-correct bound form
    /// (`x = pick(1)` → `x: optional(reference(C))` + a scope-exit `OpFreeRef`)
    /// that has always been the clean spelling.  Keeping the `Optional` matters:
    /// the temp may legitimately hold the null sentinel, and it is the bound
    /// form — not the non-optional one — that proves this type flows correctly
    /// through slot assignment, `get_free_vars`, and both backends' codegen.
    pub(super) fn reopt(was_optional: bool, tp: Type) -> Type {
        if was_optional { Type::optional(tp) } else { tp }
    }

    /// loft#1118 — may an `ncc` block that DOES carry a dep be lifted anyway?
    ///
    /// The empty-dep test is the conservative reading of "nobody else owns this", and it
    /// refuses the shape a NULLABLE PARAMETER produces: a callee whose return may be the
    /// argument or may be a store it minted answers `Own::Join`, whose dep names that
    /// argument. The block then stayed inline with the minted store owned by nothing — one
    /// leaked record per evaluation, unbounded in a loop.
    ///
    /// Lifting is safe exactly when the bind that follows is the RUNTIME guard rather than
    /// a static bet. It is: a lifted `__lift_N` is a dense `Reference`, so the heap
    /// first-bind dispatch reaches it and emits `OpBindOrCopy`, which adopts the arm where
    /// the callee minted (making the scope-exit free right) and materialises the arm where
    /// the value is the witness's store (leaving the caller's argument intact). This asks
    /// the same `Own::Join` question of the same value, so the lift cannot fire where that
    /// guard would not.
    ///
    /// **The subject must be a call to a LOFT-DEFINED function**, and that is the whole of
    /// the narrowing rather than a detail. loft's IR spells every operator as a
    /// `Value::Call`, so "the subject is a call" also matches an element read (`t[p] ?? d`
    /// is `OpGetVector`) — a view INTO a container the caller still owns, where the lift
    /// hands the temp a free that reaches inside it. Measured, not hypothetical: admitting
    /// the read made the ownership fuzz gate's `local_source` cell answer WRONG on
    /// `--native` with the two backends diverging. Only the FIRST statement is read, too:
    /// the block's default arm is often a call of its own (`t[p] ?? dflt()`), and asking
    /// `any` operator re-admits the very cell this excludes.
    ///
    /// A join whose witness the bracket cannot name keeps the conservative no-lift, which
    /// costs the leak that was already there rather than a free nothing protects.
    fn ncc_join_is_witnessed(&self, val: &Value, data: &Data) -> bool {
        if !crate::keys::join_own_enabled() {
            return false;
        }
        let Value::Block(bl) = val.unspan() else {
            return false;
        };
        // The subject is the block's first REAL statement, not its first.  A REUSED
        // `__ncc_N` opens its block with an overwrite `OpFreeRef`, which is not a subject:
        // it shifts the `Set` to second, and a predicate reading `first()` then answers
        // "not a user call" for a block that is one.  Whether a given spelling reuses the
        // temp is a numbering property, so the set of spellings this hides is not stable
        // enough to name here —
        // `tests/scripts/1118b-an-inline-join-lifts-in-every-statement-context.loft` is
        // the measurement, one cell per statement context.
        //
        // Skipping a LEADING FREE cannot re-admit the `t[p] ?? dflt()` cell the narrowing
        // above excludes: the first non-free statement still has to be a `Set` of a
        // loft-defined call.
        let subject = bl
            .operators
            .iter()
            .map(Value::unspan)
            .find(|op| !matches!(op, Value::Call(d, _) if data.def(*d).name() == "OpFreeRef"));
        let subject_is_user_call = match subject {
            Some(Value::Set(_, rhs)) => match rhs.unspan() {
                Value::Call(fn_nr, _) => data.def(*fn_nr).is_loft_defined(),
                Value::CallRef(_, _) => true,
                _ => false,
            },
            _ => false,
        };
        subject_is_user_call && self.join_is_witnessed(val, data)
    }

    /// Is this value a `Own::Join` — borrow-or-mint, settled only per execution — whose borrow
    /// arm the @P290 bracket can NAME?
    ///
    /// That is the question deciding whether the bind following a lift is a runtime GUARD
    /// (`OpBindOrCopy`: adopt the minted arm, materialise the borrowed one) or a static bet.
    /// A lift may fire wherever the answer is yes, and must not where it is no — there the
    /// conservative no-lift costs the leak that was already there, rather than a free that
    /// protects nothing.
    ///
    /// One home, because the lift, the deps strip and both backends' `OpBindOrCopy` all read
    /// it: a second spelling of the same question could only agree by accident.
    fn join_is_witnessed(&self, val: &Value, data: &Data) -> bool {
        crate::keys::join_own_enabled()
            && matches!(
                crate::use_analysis::ownership_of(data, self.d_nr, val),
                crate::use_analysis::Own::Join { base } if base != u16::MAX
            )
    }

    /// Does EVERY arm of this `ncc` block yield a store the frame would own?
    ///
    /// ⚠ The keyed lift needs this and the reference arm does not, because for a keyed result
    /// `bl.result.depend()` comes back EMPTY even when an arm is a bare local: `f() ?? d`
    /// reads as owned and lifting it freed the caller's `d` — a use-after-free where the
    /// defect was a leak, caught by the over-free cell rather than by reading (loft#1157).
    ///
    /// An arm the @P290 bracket can NAME is a view of something a variable still holds, which
    /// is exactly the wrong thing to free; an arm it cannot name minted its own store.  The
    /// SUBJECT reaches the tail as `Var(__ncc_N)`, so its own assignment is substituted in —
    /// the temp is a name, and the question is about what the call behind it produced.
    pub(super) fn ncc_arms_are_all_owned(bl: &Block, data: &Data) -> bool {
        let Some(last) = bl.operators.last() else {
            return false;
        };
        let subject = bl.operators.iter().find_map(|op| match op.unspan() {
            Value::Set(v, val) if !matches!(val.unspan(), Value::Null) => Some((*v, val.as_ref())),
            _ => None,
        });
        let mut arms: Vec<&Value> = Vec::new();
        Self::ncc_tail_arms(last, &mut arms);
        if arms.len() < 2 {
            return false;
        }
        arms.iter().all(|a| {
            let effective = match (a.unspan(), subject) {
                (Value::Var(v), Some((sv, val))) if *v == sv => val,
                _ => *a,
            };
            crate::use_analysis::view_root_slots(data, effective).is_none()
        })
    }

    /// The terminal value of each arm of an `ncc` block's tail `if`.
    fn ncc_tail_arms<'a>(val: &'a Value, out: &mut Vec<&'a Value>) {
        match val.unspan() {
            Value::If(_, t, f) => {
                Self::ncc_tail_arms(t, out);
                Self::ncc_tail_arms(f, out);
            }
            Value::Block(b) if b.operators.len() == 1 => Self::ncc_tail_arms(&b.operators[0], out),
            other => out.push(other),
        }
    }

    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn inline_struct_return(
        &self,
        val: &Value,
        data: &Data,
        outer_call: u32,
        function: &Function,
    ) -> Option<Type> {
        // loft#721 — a closure call is lifted only when its target definition is
        // known AND that definition does not return a borrowed view.
        if let Some(tp) = self.callref_owned_return(val, data, function, outer_call) {
            return Some(tp);
        }
        // loft#879 — a null-coalesce (`??`) lowers to an `ncc` value-block that
        // assigns the subject to a `__ncc_N` temp and yields either that temp or
        // the default arm's `__ref_N`.  The temp is `skip_free` (the block's
        // result ALIASES it, so freeing at the block would dangle the value the
        // consumer reads), which leaves the subject's store owned by nothing when
        // the block is used INLINE — one leaked record per evaluation,
        // unbounded in a loop.  Binding it first (`x = pick(1) ?? C{}`) has always
        // been clean because the `Set` gives the store an owner; lifting here
        // rewrites the inline spelling into exactly that bound form.
        //
        // REFERENCE results only.  A text ncc temp is already freed in place by
        // the @PLN85 skip_free-orphan pass ([`collect_consumed_ncc_text`]) and a
        // vector one by its own delivery path; both measured clean, and lifting
        // them too would free what those mechanisms free.
        // An EMPTY dep list licenses the lift because it says the value is owned.  It says
        // that for a `Call`; for a `CallRef` it says nothing.  `fnref_result_type` maps the
        // callee's return deps through the caller's ARGUMENTS and drops every index naming a
        // HIDDEN attribute, on the stated grounds that the value then arrives owned — and
        // `__closure` is a hidden attribute.  So a lambda returning a value it CAPTURED
        // hands the caller an empty dep list for a store the outer scope still owns, and
        // lifting it emits a free that reaches into that scope: the capture is released
        // while the variable it came from is still live, and the next read of it answers
        // garbage (loft#1114).
        //
        // The witnessed-`Join` route stays open to a `CallRef`, because there the bind that
        // follows is the runtime guard rather than a static bet.  Declining the other route
        // costs the leak that was already there, which is the direction this gate has always
        // taken when it cannot name what it would be freeing.
        let subject_is_call_ref = match val.unspan() {
            Value::Block(bl) if bl.name == "ncc" => {
                match bl.operators.first().map(Value::unspan) {
                    Some(Value::Set(_, rhs)) => match rhs.unspan() {
                        // Only a CAPTURING fn-ref can hand back a store the caller's scope
                        // owns; a fn-ref carrying no captures has nothing to borrow FROM,
                        // so its empty dep list means what it says and the lift stands.
                        // The fn-ref type's own deps are exactly that question, and they
                        // name the closure the call reads.
                        Value::CallRef(fn_var, _) => !matches!(
                            function.tp(*fn_var),
                            Type::Function(_, _, d, ..) if d.is_empty()
                        ),
                        _ => false,
                    },
                    _ => false,
                }
            }
            _ => false,
        };
        if let Value::Block(bl) = val.unspan()
            && bl.name == "ncc"
            && let (Type::Reference(d_nr, dep), opt) = bl.result.peel_optional()
            && ((dep.is_empty() && !subject_is_call_ref) || self.ncc_join_is_witnessed(val, data))
        {
            return Some(Self::reopt(opt, Type::Reference(*d_nr, Deps::none())));
        }
        // loft#1157 — and the KEYED kinds, which that carve-out never named.  Its reasons are
        // PER ITEM and both are about a mechanism that exists elsewhere: text is freed in place
        // by the skip_free-orphan pass, a vector by its own delivery path.  A keyed `??` has
        // NEITHER, so used inline its subject's store is owned by nothing — one retained record
        // per evaluation, unbounded in a loop, while the bound spelling
        // (`a = f() ?? []`) was clean all along because the `Set` gives the store an owner.
        // Lifting rewrites the inline spelling into exactly that bound form.
        //
        // Same ownership gate as the reference arm: an EMPTY dep list is what says the value is
        // owned, and a capturing fn-ref subject is excluded for the reason `subject_is_call_ref`
        // gives — its empty dep list means nothing.
        if let Value::Block(bl) = val.unspan()
            && bl.name == "ncc"
            && crate::parser::vectors::is_keyed(&bl.result)
            && bl.result.depend().is_empty()
            && !subject_is_call_ref
            && Self::ncc_arms_are_all_owned(bl, data)
        {
            return Some(bl.result.without_deps());
        }
        // @P297 — a USER struct-returning call (`n_*` with a body) passed
        // directly as a call argument is wrapped in `Value::Span` by
        // `parse_call` (and re-wrapped by `scan`), so the argument reaching
        // here is `Span(Call(...))`.  Unspan before matching this branch or the
        // lift never fires and the call-result temporary leaks — the same
        // pitfall `scan_set` was patched for under @P198 (`value.unspan()`).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            // loft#879 — peel `Optional` before asking the shape question; see
            // [`Self::reopt`], which puts it back on the temp.
            let (returned, opt) = def.returned.peel_optional();
            // #549 — a generic monomorph (`t_…`) whose return SHAPE is a concrete
            // aggregate (`f<T>(x) -> (integer,integer)` / `-> Struct` / `-> Enum`)
            // leaks its result store when used inline or discarded: the caller
            // lifts+frees an `n_` aggregate return (below) but historically not a
            // `t_` one, so the fresh store the monomorph allocated via `__retbuf`
            // was orphaned (both backends).  Extend the lift to `t_` — BUT a
            // monomorph LOSES its return dep during specialization, so the
            // dep-based ownership guards here cannot tell a fresh-owned return
            // from a borrowed-arg one (`id<T>(x) -> T { x }` reads as empty-dep =
            // owned and would DOUBLE-FREE if lifted).  The reliable "delivers a
            // fresh owned aggregate" signal a monomorph keeps is the `__retbuf`
            // NRVO parameter: a concrete-aggregate return gets it at signature
            // finalization; a borrowed-reference return never does.  So gate the
            // `t_` extension on `__retbuf`, leaving every `n_` case untouched.
            // loft#1066 — a monomorph with NO `__retbuf` still delivers a fresh store when
            // its body allocated one, and nobody freed it: one leaked record per inline
            // call, N calls leaking N stores, on both backends.  `monomorph_return_is_fresh`
            // answers from the BODY the question the deps cannot — substitution gives every
            // monomorph of `-> T` the same empty dep, so `{ x }` and `{ y: T = x; y }` are
            // indistinguishable to `returns_borrowed_view` and lifting on that would double
            // free the first.  The proof is positive and under-approximating: a shape it
            // cannot read stays unlifted, which costs the leak that was already there.
            // loft#1176 — a tail that is a call THROUGH A FN-REF is such a shape read from
            // the wrong frame, and it is decided by [`Self::monomorph_fnref_return_is_fresh`]
            // ALONE, ahead of every gate below.  That ordering is the fix rather than a
            // tidy-up.  The gates below all read facts carried by THIS
            // callee's signature, and none of them can carry the caller's closure: `-> P`
            // says the same thing whether the closure mints, hands back the caller's own
            // argument, or hands back a record it CAPTURED.  Reached through the `n_` arm
            // the last of those was lifted and freed — the captured record answered
            // another value on the next read and garbage once the scope ended, on both
            // backends — while `__retbuf`'s exemption made it worse: `{ f(x) }` never
            // delivers INTO that buffer, so the premise that the lifted temp is the
            // caller's own allocation is simply false here.
            // loft#1484 — and a monomorph whose return BORROWS a visible parameter, which is
            // the shape the `t_` gate above was written when it could not exist.  #549's
            // reason is *"a monomorph LOSES its return dep during specialization, so the
            // dep-based ownership guards cannot tell a fresh-owned return from a
            // borrowed-arg one"* — D-call-13 gave the instance its deps back from the
            // oracle, so `id<T>(x) -> T { x }` now publishes `["x"]` and reads as the borrow
            // it is.  Unlifted, an inline `idg(q).a = 99` wrote straight through the returned
            // DbRef into the caller's own record, where the concrete twin — lifted through
            // the `__retbuf` exemption below — copies at its `Set` and stays independent
            // (`(F-Ret)`: *the concrete twin is the oracle for the instance*).
            let monomorph_returns_a_borrow =
                (def.name.starts_with("t_") || def.is_instance()) && def.returns_borrowed_view();
            let lift_owned_return = if def.has_fnref_return_site() {
                self.monomorph_fnref_return_is_fresh(val, data, def)
            } else {
                // A free member of an overload set (`f_…`, @PLN162) is a free function in
                // every respect but its key, and lifts as one.  So does a declared METHOD
                // (`t_…`): its return deps are its own, never an instance's substituted ones,
                // so the ownership oracle below decides it exactly as it decides `n_`.  The
                // `t_` clause after this was written when an instance was keyed `t_` too; a
                // method's call reached it and was lifted only with a `__retbuf`, so a
                // method returning a loop's element through its hidden buffer — called in
                // the free spelling, `get(w, i).a` — left one store unowned per call.
                def.name.starts_with("n_")
                    || def.is_free_overload()
                    || def.is_method()
                    || monomorph_returns_a_borrow
                    || ((def.name.starts_with("t_") || def.is_instance())
                        && (def.attr_names.contains_key("__retbuf")
                            || def.monomorph_return_is_fresh(data.def_nr("OpNullRefSentinel"))
                            // loft#1273 — a tail that DELEGATES (`a + b` is `Call(n_OpAdd)`)
                            // is a shape the callee's own body settles.
                            || Self::monomorph_delegated_return_is_fresh(data, def)))
            };
            if lift_owned_return && def.code != Value::Null {
                // The same `returns_borrowed_view()` question its struct-enum sibling below
                // asks, and for the same reason: an EMPTY return dep (or one naming only a
                // hidden work-ref) is a store the callee minted and the caller adopts, while
                // a dep naming a VISIBLE parameter is a BORROW — lifting that and freeing
                // the temp releases the caller's own record while the variable holding it is
                // still live.  A function delegating to one that borrows its argument is how
                // that is reached without any borrow appearing at the call site.
                //
                // A `__retbuf` callee is exempt, and the exemption is what the borrow means
                // there: it delivers INTO the buffer the caller allocated, so the lifted temp
                // is that buffer and freeing it releases the caller's own allocation rather
                // than the argument.  Declining for those instead orphans one buffer per
                // evaluation — measured on the dense delegating twin, which was correct
                // before this gate and has to stay correct after it.
                //
                // `returns_borrowed_view()` is the deps PROXY (@FR-O-Proxy), and it is
                // deliberately not the last word: a callee that mints into its buffer on one
                // path and returns a parameter on another carries a dep naming that
                // parameter, so the proxy calls it a borrow while the value the caller
                // actually receives is owned.  `ownership_of` is the oracle (@FR-O-Oracle)
                // and this is the chokepoint that should read it.
                //
                // `Owned` lifts.  A `Join` lifts only where the bind that follows is the
                // runtime guard — the witness has to be nameable, and the statement has to
                // be one that BINDS.  `outer_call == u32::MAX` is the bare-statement
                // lowering, where the lifted temp gets no `OpBindOrCopy` on the interpreter,
                // so the free would run on the borrow arm too and release the caller's own
                // record; there the conservative no-lift stands and costs the mint arm's
                // leak instead.  `Borrowed` never lifts.
                let own = crate::use_analysis::ownership_of(data, self.d_nr, val);
                let lift_by_oracle = match own {
                    // @PLN155 phase 2 — lifting gives an unbound result a NAME so the sweep
                    // can free it; declining on `Unknown` would leave it unnamed and leak,
                    // which is why this arm keeps `Owned`'s answer.  `Borrowed` never lifts.
                    crate::use_analysis::Own::Owned | crate::use_analysis::Own::Unknown => true,
                    crate::use_analysis::Own::Join { base } => {
                        outer_call != u32::MAX && base != u16::MAX
                    }
                    crate::use_analysis::Own::Borrowed { .. } => false,
                };
                // An inline-unbound call whose result is a struct-like heap store —
                // a `Reference` or a record ENUM, the two spellings `heap_def_nr`
                // answers for — binds its result to nothing, so nothing frees it.
                // Lifting it into a `__lift_N` gives `get_free_vars` a name to emit
                // the `OpFreeRef` against.  The lifted temp keeps the spelling it
                // arrived with.
                //
                // Three ways to be the caller's to free, and the second is the one a
                // dep list reads backwards:
                //   - EMPTY dep (`fn mk() -> H { Bytes{…} }`) — fresh, owned.
                //   - a dep naming the HIDDEN `__ref_N`/`__retbuf` the callee
                //     delivered through.  That reads as a borrow and is not one: the
                //     buffer is the CALLER's own allocation, so the lift's copy-path
                //     free (`0x8000` source-free) claims it exactly as the bound
                //     `h = f()` case does.  Declining here orphans one store per
                //     evaluation (#490 kt=65 on native, loft#1202 on both backends).
                //   - the ORACLE says owned where the deps proxy cannot (@FR-O-Oracle).
                // A dep naming a VISIBLE parameter (`fn field_of_arg(d) -> H { d.value }`)
                // IS a borrow: lifting it would dangle the caller's own argument.
                //
                // ⚠ Asked ONCE for both spellings on purpose.  These were two arms, and
                // the record-enum one carried only `!returns_borrowed_view()` — so a
                // struct-enum callee delivering through a `__retbuf` fell through the
                // second bullet above and leaked, while its `Reference` twin did not.
                if returned.heap_def_nr().is_some()
                    && (!def.returns_borrowed_view()
                        || def.attr_names.contains_key("__retbuf")
                        // loft#1484 — the monomorph twin of the `__retbuf` exemption beside
                        // it, and it is the SAME reason rather than a second one.  What makes
                        // that exemption safe is not the buffer but the guarded copy the
                        // lift's own `Set` emits for a borrow-returning callee: it adopts
                        // when the store that came back is fresh and deep-copies when it is
                        // the argument's, which is decided per execution by identity.  A
                        // monomorph has no buffer and the same guard, so the same lift is
                        // safe — and it is the only thing that gives the inline spelling
                        // anywhere to put the copy.
                        || monomorph_returns_a_borrow
                        || lift_by_oracle)
                {
                    return Some(Self::reopt(opt, returned.with_deps(&Deps::none())));
                }
                // Plan-57: a user fn returning a CAPTURING closure (`fn(...) -> T`
                // whose fn-ref carries a fresh closure record) leaks its result temp
                // when used directly as a call argument — `apply(make())` left the
                // `__closure_*` record at rc 1 and the cell uncollected.  Lift it like
                // the Reference / struct-enum cases so `get_free_vars`' fn-ref arm
                // emits the `OpFreeRef`; codegen frees the closure DbRef at offset+8
                // and `free_named` cascades to the captured `__cell_*`.  NOT guarded on
                // `dep.is_empty()` — a capturing closure's dep IS the cell (e.g.
                // `function([], integer, [1])`), so guarding would skip the very case.
                // A non-capturing return carries the null closure sentinel → the free
                // is a safe no-op; a borrowed fn-ref copy is marked `skip_free`
                // elsewhere, so only a freshly produced closure is lifted here.
                if let Type::Function(params, ret, _, consts) = returned {
                    return Some(Self::reopt(
                        opt,
                        Type::Function(params.clone(), ret.clone(), Deps::none(), *consts),
                    ));
                }
            }
            // loft#792 — the same lift for a body-less NATIVE global that MINTS a
            // record.  `type_of(x)` / `type_named(n)` lower to `n_reflect_type` /
            // `n_type_named`, which allocate a `TypeInfo` store; passed straight as a
            // call argument the value was bound to nothing, so nothing freed it.
            // Binding it to a local first leaked nothing, which is what made this read
            // as a reflection quirk rather than the missing lift it is.  Native frees
            // the record through its own drop path, so the leak was interpreter-only —
            // and it CASCADES: with a callee that returns a struct holding a freshly
            // built vector, that vector leaked once per call after the first, so a loop
            // calling `f(type_of(x))` grew the heap without bound.
            //
            // The bound is what keeps this sound.  A native has no body to read, so
            // lift only where the answer cannot be anything but a fresh record: the
            // return names a concrete STRUCT, carries no dep, and no parameter has a
            // type that could have supplied one.  That excludes every view-returning
            // native in the stdlib — `hash_sorted(h: reference, …) -> reference` and
            // `parallel_buf_get_ref(i) -> reference` both hand back a borrow, and both
            // return the untyped `reference` rather than a named struct.  The
            // `JsonValue` constructors are struct-ENUMs and keep their own arm above.
            if lift_owned_return
                && def.code == Value::Null
                && let Type::Reference(d_nr, dep) = returned
                && dep.is_empty()
                && data.def_type(*d_nr) == DefType::Struct
                && !def
                    .attributes()
                    .iter()
                    .any(|a| matches!(a.typedef.base(), Type::Reference(p, _) if p == d_nr))
            {
                return Some(Self::reopt(opt, Type::Reference(*d_nr, Deps::none())));
            }
        }
        // @P393 (t9) — a loft-source fn OR `t_` method returning an OWNED vector
        // BY VALUE (empty dep) is de-NRVO'd when its body builds the result with
        // >=2 distinct element-temps (`01a3f24f` in `ref_return`): the signature
        // is `n_f() -> vector<T>` / `t_..split() -> vector<text>`, with no hidden
        // `__vdb`/`__ref` buffer param.  Used inline-unbound (`len(split(x))`,
        // `split(x).join(y)`) the by-value temp gets no scope-exit free on the
        // interpreter → its store leaks (native codegen already frees it).  Lift
        // it like the Reference / struct-Enum / Function cases above so
        // `get_free_vars` emits the `OpFreeRef`.  A SEPARATE block from the
        // `n_`-gated one above because `split` is a `t_` method — broadening that
        // block's guard would also change the Reference/Enum/Function arms' scope.
        // Gated on `dep.is_empty()`: the NRVO'd hidden-param return carries dep
        // `["??"]` (caller already frees `__ref`) and a borrowed view carries
        // `[self]`, so both are excluded — no double-free, no UAF.  `val.unspan()`
        // because `parse_call`/`parse_method` Span-wrap the call (arg AND
        // receiver = arg0).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            if def.is_loft_defined() {
                // loft#879 — peel `Optional` for the match, restore it on the temp
                // ([`Self::reopt`]).  The dep guards keep reading the BASE's dep,
                // which is where a borrowed view records itself.
                let (returned, opt) = def.returned.peel_optional();
                match returned {
                    Type::Vector(elem, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Vector(elem.clone(), Deps::none())));
                    }
                    // @PLN85 p188 — a discarded (or inline-unbound) owned KEYED
                    // collection return (`build() -> sorted<T[k]>`, and the
                    // index/hash/spatial siblings) leaks its by-value store exactly
                    // like the vector case above — the `Drop` lift binds it to a
                    // `__lift_N` temp so `get_free_vars` emits the store's
                    // `OpFreeRef` (both backends).  Empty dep = OWNED (fresh); a
                    // borrowed view / NRVO'd hidden-buffer return carries a
                    // non-empty dep and is excluded (no double-free / UAF).
                    Type::Sorted(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Sorted(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Index(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Index(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Hash(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Hash(*d, keys.clone(), Deps::none())));
                    }
                    Type::Radix(d, keys, dep) if dep.is_empty() => {
                        return Some(Self::reopt(
                            opt,
                            Type::Radix(*d, keys.clone(), Deps::none()),
                        ));
                    }
                    Type::Trie(d, key, dep) if dep.is_empty() => {
                        return Some(Self::reopt(opt, Type::Trie(*d, key.clone(), Deps::none())));
                    }
                    _ => {}
                }
            }
        }
        // Native-constructor calls arrive BARE when chained onto a builtin
        // (`v.keys().len()`) and Span-wrapped when passed as a call argument or
        // method receiver (`jt(json_parse(x), n)`, `json_parse(x).field(n)`), so
        // match through the span — an unlifted native-constructor temp owns a
        // fresh store nothing ever frees (#490).
        if let Value::Call(fn_nr, _) = val.unspan() {
            let def = data.def(*fn_nr);
            // loft#879 — peel `Optional` for the match, restore it on the temp
            // ([`Self::reopt`]).
            let (returned, opt) = def.returned.peel_optional();
            // Native struct-enum constructors: no body (code == Null), return type
            // is a struct-enum with empty dep (allocates a new store, doesn't borrow).
            // Accessors carry dep=[0] after parser dep-inference and are skipped here.
            if def.code == Value::Null
                && let Type::Enum(d_nr, true, dep) = returned
                && dep.is_empty()
            {
                return Some(Self::reopt(opt, Type::Enum(*d_nr, true, Deps::none())));
            }
            // Native vector-returning fns (e.g. `keys()`, `fields()` on
            // JsonValue) allocate a fresh vector store that the caller owns.
            // Without lifting, the chained call `v.keys().len()` leaks the
            // intermediate vector — same mechanism as the struct-return case.
            if def.code == Value::Null
                && let Type::Vector(elem, dep) = returned
                && dep.is_empty()
            {
                return Some(Self::reopt(opt, Type::Vector(elem.clone(), Deps::none())));
            }
        }
        None
    }
}

/// How many generic instances deep [`Scopes::monomorph_delegated_return_is_fresh`] follows a
/// chain of delegating tails before it answers `false`.  The bound is the cycle guard mutual
/// recursion needs (`f<T>` forwarding to `g<T>` forwarding to `f<T>`), not a measured depth.
const DELEGATION_DEPTH: usize = 8;
