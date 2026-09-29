<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ownership.md — the `deps` ownership / borrow system (strict; register at `OPEN: 0`)

**Catalogue:** @F21 (references `&T`), @I60 (deps / lifetime tracker) — Goal E. Roadmap: @PLN85, @PLN87.

> **Rules then deviations** (see [README](README.md)). The rules below are loft's
> ownership model.  The register stands at **`OPEN: 0`** (below); a zero that an oracle cannot
> disturb is only as strong as the questions that oracle asks.  The five original `D-own-*`
> deviations remain CLOSED on the **shipped path** (@PLN85 store-lifetime,
> @PLN87 the `&` law both landed), validated by the @PLN89 differential oracle + the
> `program_ownership` fuzzer. This is *validation, not a machine-checked proof*: the
> `Join` fact still resolves through a runtime witness, and the pre-fact shape-scans
> survive under opt-out as differential-control machinery. The residual is not a
> correctness deviation but the *substrate* — the fact is computed flow-INSENSITIVELY.
> [@PLN94](../plans/94-cfg-ownership-dataflow/) has now built the flow-SENSITIVE
> replacement as an independent oracle that runs BESIDE the shipped analysis (a machine
> check on every `cargo test`, `tests/ownership_oracle.rs`); its abstract-interpretation
> soundness — the over-free half — is now PROVED given the rules here (the local-transfer
> lemma discharged case by case). See §"Machine-checkable soundness" below; only a Coq/Lean
> rendering of the prose remains.
>
> The rules are loft's borrow checker. **Rust is the reference model.** Beacon + rationale:
> [OWNERSHIP_MODEL.md](../OWNERSHIP_MODEL.md); the typed-`deps` design:
> [DEPS_INVENTORY.md](../DEPS_INVENTORY.md). This doc is the **checker** (lifetimes /
> free placement); the **surface** (`&τ`, reference-default) is [binding.md](binding.md).

## Notation

- **owner** — the binding or slot responsible for freeing a heap store. Exactly one at a
  time.
- **borrow / alias** — a value that *refers to* a store it does not own (a parameter, a
  field/element read, a `&τ` link). It must not free, and must not outlive its source.
- **`deps`** — the per-binding list recording what a binding borrows from. (Today a
  `Vec<u16>`; see D-own-3.) An EMPTY list is read as "owner" at many sites — that reading
  is `O-Proxy`, and it is a stand-in for the oracle rather than the oracle itself.
- **the oracle** — `use_analysis::ownership_of`, which computes own-vs-borrow for a VALUE
  from the IR. `O-Oracle`. It does not consult `deps`.
- **the never-free override** — a per-binding flag (`is_skip_free`) that forbids emitting a
  free for that binding at all. `O-Override`.
- **the latest-assignment memo** — `Scopes::owned_refs`, the oracle's answer for a binding's
  most recent assignment, tagged with the loop depth at which it was taken. `O-Latest`.
- **transfer / move** — handing ownership to another binding (e.g. a return). The giver
  stops owning; it must not free what it moved.

---

## Rules

> The model is **sound** (no use-after-free, no double-free, no leak) **and complete**
> (computed for *every* binding, every path). The five invariants:

```
  (O-Owner)     SINGLE OWNER.  Every heap store has exactly one owner at any moment.
  (O-Move)      MOVE ON RETURN.  A returned heap value's ownership transfers to the
                caller's binding; the callee never frees what it transfers.  If the return
                *borrows* a parameter, the return type records it (`{Attr(param)}`) and the
                caller COPIES to obtain its own store.
  (O-Opaque)    A RETURN TYPE THAT CANNOT RECORD.  O-Move puts the obligation on the return
                type, which presumes a type able to carry it.  A fn TYPE cannot: `fn(τ) -> σ`
                is the whole of what an author may spell, so a call through a fn-typed
                PARAMETER arrives with empty deps whatever its target does.  Empty deps are
                then read as *"the callee minted this"* — the one reading that licenses a
                free — and the caller releases its own argument on the arm where the closure
                handed it back.  Where the type cannot record, the caller assumes the return
                BORROWS every heap argument it passed.
  (O-Borrow)    BORROW TRACKING.  A value aliasing another (param / field / element / `&τ`)
                carries the source in its `deps`; the borrower is skip-free; the single
                owner frees once.
  (O-Borrow-Scalar)  EXCEPT a `&` link at a SCALAR local, which owns no store and so has no
                free decision to record: its `deps` stay EMPTY and the obligation it does
                carry — that the place it names outlives it — is recorded on the TARGET
                (`Variable::amp_linked_by`), read by the slot allocator and by nothing else.
                Two channels, each answering one question.
  (O-Derived)   FREE PLACEMENT IS DERIVED, NOT DECIDED.  Free a local iff it owns its store
                and does not transfer it out — once, at scope exit.  No per-site heuristic.
  (O-Buffer)    A HIDDEN RETURN BUFFER IS THE CALLER'S STORE, WITNESSED BY THE LOCAL
                THAT ADOPTS IT.  The `__ref_N` a caller passes for a callee's record
                result belongs to the caller, and the callee either fills it and hands
                it back or mints its own and hands that back — which, no static bit can
                say (O-Opaque).  So the result local's free is guarded by STORE IDENTITY
                against the buffer (`OpFreeRefIfDistinct(v, __ref_N)`), declining exactly
                when the callee handed the buffer back, and the buffer's own free at
                frame exit is what releases it then.  Every arm of a value branch is such
                an adoption, so a local bound from a branch is witnessed by EVERY arm's
                buffer and its free declines against each (O-Complete, per path).  A
                result reached any other way — a lift temp with a plain free, a local
                assigned twice — has no such witness.
                The callee reads the same fact from its side.  A local it PROMOTES onto
                the buffer IS that parameter, so it holds the caller's store when the
                caller handed a live one and a store the callee minted when the caller
                handed the null sentinel — and only the second is the callee's to
                release.  No static bit separates them, so the callee snapshots the store
                it was handed at entry (its ENTRY WITNESS) and every free it emits of the
                promoted local — the displaced store at a rebind, the store left behind
                at an exit that answers another — declines on that store.
                A buffer that IS A DESTINATION PLACE is the exception, and the only one:
                where (R-Place) hands a call the place its result is going — the place
                EXISTS at the call and no argument reaches it — the buffer is not a
                store of the caller's at all.  It is never freed, at frame exit or
                anywhere, and it needs no identity guard, because there is no second
                store for the result to be distinct FROM: the result names the
                destination.  The clause is here rather than in (R-Place) because it is
                this rule that says what a buffer is; without it the two describe
                different objects.
  (O-LazyBuffer) A HIDDEN RETURN BUFFER IS MINTED WHERE IT IS FIRST USED.  (O-Buffer) says
                whose store a buffer is and when it is released; it does not tie the MINT
                to function entry.  The store is minted in front of each statement that
                hands the buffer to a callee, behind a null test — at most once per
                activation, and only on a path that makes the call.  A path that does not
                leaves the slot at the null sentinel, and every exit's free of the sentinel
                is a no-op (H-FreeNull), so no release moves.  A buffer named only by a
                free or an identity test is never read, and is never minted.
  (O-Complete)  PER BINDING, PER PATH, COMPLETE.  Every binding, including every `match`/`if`
                arm — a set-and-reconcile, not a single-variable structural walk.
  (O-ViewField) A FIELD OF A RETURNED RECORD MAY BE A VIEW.  Where a heap field of a
                function's result is assigned from a value that has an OWNING
                destination in a store the frame does not own — a parameter's store,
                or one handed up to the caller — so that the place outlives the frame,
                the field may be delivered as a VIEW of that place instead of a copy of
                its own, and the return type records the borrow on the field as it
                records a whole-value borrow (O-Move).
                THE CALLEE'S HALF is a fact at each EXIT, on EVERY path to it (O-Complete):
                the last change to that container was the append of an element whose
                field took a copy of the value, and after that copy neither the value —
                nor a store it views, nor a view of it — nor that field, nor the
                container changed.  The view is that element's field; where the paths
                appended DIFFERENT elements it is the field of the container's LAST
                element, which is the appended one on each of them.  The value views
                only stores the FRAME owns — a parameter's store can arrive under a
                second name the frame cannot watch — and has exactly ONE destination
                outside the frame, or the place is undecided.  A `?`-discharged element
                is not such a value: its absent arm is a record of the frame's own
                (B-View), so a view of the container would dangle there.  An exit that
                writes the field NOT AT ALL delivers a null view, which reads as the
                empty vector it replaces; "not at all" is PROVEN by accounting every
                write the exit makes to its record, never assumed from the absence of
                one spelling — a vector literal fills by element pushes and a record
                literal's vector field by element appends.  A leaf is a plain
                `vector<T>`: a `text` is read through its own getters and a keyed
                collection through its keys, neither through the field slot.
                THE SITES' HALF is decided over EVERY call site of the function at once:
                each site binds the result to a local whose bind DOMINATES its reads and
                that only READS the field — a use that answers a VALUE: a length, a null
                test, a `const` argument.  An ELEMENT read answers a PLACE and is not a
                read, however read-only the op that answers it, because the site may
                write through what it answered.  The local reaches its last read with no
                disturbance (B-Disturb — a removal renumbers, a growth relocates) of the
                container the view names between the call and that read, in the caller
                or through what it calls, and that span is judged as an UPPER bound: a
                naming of the container is a disturbance unless what it reaches is shown
                to move nothing (a sibling field, a claim in the store, a fixed-width
                field write, a read of the watched field itself, a call whose own
                disturbance summary does not reach it).  One site that stores, appends
                to, iterates, rebinds, returns or hands the field to a by-value parameter
                declines the function whole, and every site keeps the owning copy.
                "Every call site" means every site the compiler sees whole (R-Escape): a
                result that escapes the unit — a library's exported function — never
                carries a view.  It keeps its record, since the boundary that
                materialises a tuple writes FIELDS and a reference is not one.  A
                declined view is a copy, never a stale read.
  (O-One-Kind)  THE STORE IS THE ONE KIND OF HEAP OBJECT.  Every value that reaches the
                heap is a `DbRef` naming a store: identity is the store, and a free
                releases a store (`Stores::free`), on both backends.  An optimisation may
                change what a store's BYTES are — a foreign span over bytes the program
                does not own (@PLN174), a hash-entry arena inside the store (@PLN135) — or
                remove a temporary store outright (R-ValueRecord, O-ViewField, R-Place,
                R-InPlaceLiteral, R-ElemFirst) or reuse one per call site.  It may not add
                an object with an identity or a release of its own.  The admission test is
                reuse: a variant that every existing read routine serves unchanged is a
                store; one that needs its own reads, or its own handle, is a second kind
                (C125).
```

**`(O-ViewField)` in words** (@PLN164 C5, written before the phase is cut).  The natural
`parse_poly` writes its points into the op it appends to the scene AND into the `Mark` it
returns, and the caller only reads `ps_p.pts` to pass it on; a programmer who knows the
store model returns the op's index instead.  The compiler may not change that contract,
so the rule lets the returned field BE the op's vector — a reference in the value tuple
`(R-ValueRecord)` already returns for scalar records — exactly where every caller could
not tell the difference: read-only, and no growth of `sc.ops` between the call and the
last read.  The three parts are the three ways it could be wrong: the source dying with
the frame (a dangling view), a site that writes or keeps the field (a lost write, a copy
the caller expected), and a disturbance between the call and the read (a stale view).
Every part declines to the copy the code emits today, which is the direction
`(O-Move)` already points.  Admitted under C122: the contract is
semantics, not representation, so a returned view needs only its conditions; the library
API is the boundary, and a construction that does not escape it may be rewritten freely.

The measurements that shaped these conditions, each with the cell that bought it, are in
[ownership-history.md](ownership-history.md) § What the build measured.

**In words.** One thing owns each piece of heap, and it's the only thing that frees it.
When you return a heap value you *give it away* (the function stops owning it); if you
only return a view into an argument, the type says so and the caller makes its own copy.
Anything that just borrows is tracked but never frees. Crucially, *where* to free is
**computed** from these facts, not guessed per code-site — and it's computed for **every**
binding on **every** branch, not just the easy ones.

**`(O-Complete)`'s "every path" has two halves, and a set-and-reconcile of OWNERSHIP covers
only one.**  The other is what the binding HOLDS on the paths that never assigned it — the
null a local first assigned inside a branch carries on the other arm, and outside the loop
that first bound it — and which store a binding that ADOPTS a compiler buffer's record is
freeing at its own scope exit when that scope is inside the buffer's.  Both were measured
short of the rule for the NULLABLE spellings alone (D-own-33): `needs_pre_init` names the
locals that get the null and the hoist, and it must peel `Optional`; a literal's `__ref_p2_N`
adopted inside a loop body takes the pairing a call's `__ref_N` already has (`witness_buffer`,
@P378(a)) so one owner frees once; and the free-source licence of a keyed join reaches every
`match` arm, not only an `if`'s.  The rule did not change: the code did, at those three homes.
A fourth home, @PLN157 § V-af: a local bound from a value BRANCH whose arms end
in buffer-delivering calls (`v = if c { mk(i) } else { mk2(i) }`) adopts whichever arm's
`__ref_N` ran, so EVERY arm's buffer is its witness — the same pairing a direct call takes,
per tail call — and the local's scope-exit free is the multi-buffer `OpDistinctStore` ladder.
The branch binds the arm's `DbRef` as it is, with none of the copy a direct call's set
lowering interposes, so whether the callee adopts a fresh store or fills the buffer it is
handed makes no difference to the pairing.  Before it a branch paired nothing, which was
correct (the local freed a store minted per call) and forfeited the buffer reuse; the
falsifier is the positive control `LOFT_NO_RETBUF_WITNESS_GATE=1` under `LOFT_STRICT_STORES=1`
on the one-cell probe — clean with the pairing, a use-after-free without it, both backends.
Switch `LOFT_NO_JOIN_BUFFER_WITNESS`; site `scopes.rs` (the branch block above the direct
call's pairing), `scopes::tail_calls`.
The VECTOR spelling of a bound value branch has its home where the vector copy has its home —
the parser's bind selector, not the post-parse lift — and had none until D-own-35: every
value-branch bind of a vector local aliased the chosen arm, and `x = s.v ?? va` viewed a
projection `x = s.v` copies.  `Parser::sink_vec_bind_into_arms` writes such a bind out per arm
and classifies each tail by the same selector; a promoted return buffer keeps the value form.

**`O-Detach` is about ORDER, and it is the one rule here that a correct ownership FACT cannot
save you from.** Every other rule answers *who owns this store*; this one answers *when may the
lowering act on that answer*. A binding whose ownership is computed perfectly is still read as
`null` if the sentinel that detaches it is emitted before the expression that reads it — which is
what `p = mk(p.a + 1)` did on a heap parameter, on both backends, with nothing reported
(loft#1312). The same order appears three more times: the `--native` adopt-vs-copy guard cleared
the destination while the source still named that store; the reassignment path in `codegen.rs`
avoids it by asking `Value::reads_var` and deferring the free; and the @PLN87 P2.1 literal
lowering avoids it by hoisting the field reads into temporaries. A collection LITERAL is the
same rule at the build's own detach — the `=` repoint, a field's clear — and had no home until
D-own-36: `v = [v[1], v[0]]` read the emptied result; `Parser::snapshot_read_destination` now
hoists its reads onto a copy taken before the first write, the comprehension's cure given one
home. Declining the detach is NOT a third option — that is what D-own-16's open half does, and
it trades a wrong answer for a retained store rather than resolving the order; `--native` did
exactly that for a value-`if` right-hand side (D-own-36's second face) until it counted the
shape among those that produce a store.

**This is an INTERNAL system — it never rejects a program it can compile.** loft has no
user-facing borrow checker; the user writes naively and the compiler always finds a valid
lowering, copying when it cannot prove an alias is safe
([OWNERSHIP_MODEL.md § Internal and invisible](../OWNERSHIP_MODEL.md)).
That makes **`O-Complete` the load-bearing invariant**: an incomplete fact is not a compile
error the user fixes — it is a miscompile or a leak. So the failure mode to fear here is
*incompleteness* (D-own-2), not just unsoundness — the analysis must be **total**.

The single carve-out is not in this doc's rules at all, and stays that way: where NO correct
lowering exists — an explicit `&` reference whose place the program then destroys — the
*binding surface* declines the program ([binding.md](binding.md) B-Ref-Reshape, C79 revisited). That is a decision about what `&` MEANS, not a lifetime the checker failed to
prove, so it changes nothing here: these rules still never reject, and the deps fact is still
computed for every binding on every path. A program this doc's rules would have handled fine is
never refused.

### The mechanism — one fact, derived everywhere

```
  (O-Deps)      every store-lifetime codegen decision — free placement, adopt-vs-copy,
                move-vs-clone, drop — DERIVES MECHANICALLY from the single `deps` fact.
                If a decision is re-derived by a codegen condition, that is the bug.
  (O-NoDiverge) because both backends translate the SAME `deps` facts, the interpreter and
                `--native` cannot diverge.  (This is the soundness side of
                [operational.md](operational.md)'s shared contract: O-NoDiverge is *why*
                E-Op/E-Trap agree across backends.)
```

**In words.** Every "do I free / copy / move this store?" question is *answered by reading
a carried fact*, never re-worked out in the code generator. And because both backends read
the same answer, they can't disagree — which is exactly what makes the operational rules
hold on native as well as interp.

**Where a decision still lives in a backend, it is spelled ONCE.**  The displacement free at
a heap reassignment is the one store-lifetime decision both code generators still make
themselves, and its fact-reading half is `Function::owns_displaced_store` (the store-backed
kinds, the empty-dep proxy or the one-argument borrow `Function::borrows_one_argument`, the
`O-Override` veto, the capture exclusion, the detach) — read by the interpreter's `owned_ref`,
native's `owned_ref_reassign` and the scope-exit sweep alike.  Two lists kept "verbatim" by
hand drifted four times, each found by a leak or an abort on one backend only (QUALITY-history.md
B7s); one predicate cannot.  What stays per backend is only what IS per backend — the
interpreter's hidden-buffer-argument exclusion, native's declared-local and store-producing
right-hand-side conditions.  A fact both backends need belongs in the IR; one they cannot
share belongs behind one predicate.

### The facts that answer it — there are four, and `deps` is not the oracle

`O-Deps` above is written as though `deps` were the single source of truth. It is not, and
the gap between that sentence and the implementation is where this subsystem's bugs live
(loft#723 is the worked example). Four distinct carried facts answer ownership questions,
and a decision that reads the wrong one is wrong in the silent direction:

```
  (O-Oracle)    the own-vs-borrow answer for a VALUE is computed by ONE oracle
                (`use_analysis::ownership_of`), from the IR: a store mint is Owned, a
                projection is Borrowed(base), a call resolves through the callee's return
                summary.  It does NOT read `deps` — the two are INDEPENDENT derivations of
                the same question.  A chokepoint reads the oracle rather than re-deriving.
  (O-Proxy)     an EMPTY `deps` list is a cheap PROXY for "this binding owns its store",
                and it is UNSOUND ALONE: a borrow whose dep list was never populated also
                reads empty, so the proxy answers "owner" for a borrower.  A site that
                FREES on the proxy MUST also consult O-Override — otherwise it frees a
                store someone else owns.
  (O-Override)  a binding may carry an explicit never-free flag
                (`variables::Function::is_skip_free`).  Its contract is "no free DERIVED
                FROM OWNERSHIP is ever emitted for this binding" — in ANY of a free's five
                spellings (`OpFreeRef`, `OpFreeRefTag`, `OpFreeText`, `OpFreeRefIfDistinct`,
                `OpFreeRefOrHandUp`; `use_analysis::OpSets::frees` is the one home of that
                list, and a matcher spelling its own goes blind to the next one), from the
                scope-exit sweep, a transition free, a pre-`Set` free or a move alike — so
                it VETOES the proxy and the sweep alike.  It exists BECAUSE O-Proxy is
                unsound alone.  The ONE admissible free of a never-free binding is the
                release the MARKING pass places itself, on a fact of its own rather than on
                the proxy: a STAGED TEXT TEMP (`Function::is_staged_text_temp` — a `??`
                subject, a return-delivery stage) is never-free for the sweep because its
                value outlives the block the sweep would free it in, and the pass that
                staged it frees it after the statement that copied the value out.
                `ownership_cfg`'s Check D (`LOFT_OWN_ORACLE=check`) is the gate: a free of
                any other never-free binding, by any live spelling, is a RED.
  (O-Unknown)   the oracle may answer that it DERIVED NOTHING (`use_analysis::Own::Unknown`),
                and that answer is neither a free licence nor a refusal: it obliges the
                reader to DECIDE.  Two shapes reach it — a `CallRef` whose target cannot be
                resolved, and a callee returning a borrow whose base the caller cannot NAME —
                and both answered `Owned` before, the one verdict that licenses a free, so a
                reader inherited the permissive value by writing `_ =>`.  It carries no base
                by definition: there is nothing to witness, so a reader needing a witness
                must read it as "no answer" and never as a `Borrowed` with a missing one.
  (O-Latest)    ownership is a property of the LATEST assignment to a binding, at the LOOP
                DEPTH at which that assignment was taken (`Scopes::owned_refs`, a memo of
                O-Oracle plus that depth).  A type-level `deps` list can express neither,
                so the ownership-TRANSITION free — freeing the store a binding is about to
                stop pointing at — reads THIS and not `deps`.  The same sentence decides
                which store a CLOSURE RECORD takes over the free of: the one its capture
                named AT THE BUILD (`Scopes::capture_build_backing`), not the one the
                local's dep names at scope exit (loft#1324).
  (O-Detach)    DETACH AFTER THE READS.  A binding's DETACH — the free, sentinel or
                re-allocation that stops it naming its current store — is sequenced AFTER
                every read of that binding by the value being assigned to it.  A lowering
                that must detach early hoists those reads into temporaries first; one that
                cannot hoist defers the detach past the assignment.  Detaching before the
                reads is never admissible, and DECLINING the detach to avoid the hazard
                trades the wrong answer for a retained store rather than resolving it.
  (O-Witness)   A LOCAL WHOSE ASSIGNMENTS MIX OWNERSHIP CARRIES A RUNTIME OWNER WITNESS.
                A binding carries ONE dep list, flow-insensitively, and it records
                whichever assignment parsed LAST — so where one assignment hands a
                heap-record local a store of its own (a whole-value copy, a minting call)
                and another hands it a VIEW, no static reading is right for both.  Such a
                local is given a hidden witness `__own_<name>` naming the store it minted
                for as long as it still holds it, and the sentinel otherwise.  The witness
                is maintained in the IR at every assignment — a mint points it; a view
                releases it by STORE IDENTITY (`OpDistinctStore`) after the value is
                computed, per O-Detach; a mint that reads the local releases the same way
                — and released at scope exit.  The local itself is never-free
                (O-Override), and both backends translate the one fact (O-NoDiverge).  A
                projection bound into such a local is always a VIEW: binding.md's
                materialise clause does not apply to it, and a binding that clause names
                is never witnessed.  Every copy into it lands in a FRESH store, never in
                place — the slot may hold a view.
```

**In words.** There is a real oracle, and it is not the dep list. `deps` is a cheap
stand-in for it that is right most of the time and wrong for a borrow nobody recorded a dep
for; `is_skip_free` is the patch that makes the stand-in safe at a free site; and
`owned_refs` carries the two things a type cannot — *which* assignment, and *how deep in
loops* it happened.  And where even that is not enough — a local that OWNS after one
assignment and VIEWS after another, in a loop that runs both — the ownership is a per-RUN
fact, and `O-Witness` carries it in a slot beside the local: the walker `cur: Node? = a;
while cur != null { cur = cur.next }` frees the copy it started from at the first rebind
and nothing at the last, whichever iteration that is (loft#1336).  Because it is a fact the
EMITTERS read, it must survive the startup cache like `skip_free` does: it was maintained in
the IR and restored by no snapshot field, so a WARM program-cache run emitted the pre-witness
copy arm and wrote a copy INTO the record the local was viewing.  `__own_<name>` is now the
tenth stored variable field, and the cache format version is bumped so a stale bundle is not
read (loft#1336 follow-up, QUALITY-history.md B7v).

**And the copy `O-Move` asks for is ELIDED where no program can observe it** (@PLN157
§ V-g).  A record local bound once from a call whose return borrows a value-const,
never-rebound ARGUMENT, and read only through projections, keeps the view —
`use_analysis::view_elision_bind`, one predicate for `scan_set`'s strip and both
backends' copy arms — and releases the callee's per-execution minted store by identity
against that argument at scope exit, the collection join's route (loft#1257).  The rule
is unchanged: a written local, an escaping one (a call argument, a literal element, a
return, a capture), a nullable one, a rebound or non-const base, all still copy.  The mark
(`view_elided`) is the eleventh stored variable field, for the reason `__own_<name>` is the
tenth — measured before it was stored: a warm `LOFT_PROGRAM_CACHE=1` run re-emitted the
copy beside the stored identity free.

⚠ **`(O-Oracle)`'s interprocedural half has a failure mode of its own: it can lose the
callee's answer on the way back to the caller.** The summary is stated in the CALLEE's
parameter space, and delivering it means naming the caller value the returned store may lie
in. Where that translation gives up, the honest answer is *"a borrow whose base I cannot
name"* — but a `CallRef` answered `Owned`, the one verdict that licenses a free, so the
caller released a container it still held and the next read came back `null` with nothing
said (loft#1318). Three shapes reached it: an argument that is itself a CALL (the structural
walk stops at one, on the ground that the caller lifts it into a temp first — which a
BORROW-returning callee is not), a keyed lookup (`m[k].v`, missing from the oracle's own
projection set, so a view read as a mint), and a hidden `__retbuf` (refused as "not a
parameter the author wrote", though the caller allocates that buffer and passes it).

**The rule that decides all three is one sentence: a translation that cannot name a base must
not upgrade the verdict.** Naming the base is always preferable to declining — the base is
what `(O-Oracle)`'s run-time test compares against, so a named one keeps the mint arm's free
as well — but between an unnameable base and `Owned` there is no trade: `Owned` is the
over-free direction, and a leak is recoverable where a premature free is not.

**And the translation itself has ONE home.**  `use_analysis::structural_arg_base` carries the
hidden-parameter rule, the delivery-buffer exception and the projection-root walk, and both
derivations of the fact read it — the oracle, and the @PLN94 flow-sensitive shadow that
cross-checks it (Check A).  The shadow's own copy, written to *mirror* the oracle's, carried
none of loft#1318's three fixes and reported the oracle's CORRECT answers as disagreements;
what the shadow keeps independent is the FLOW, never the translation.  The oracle's answer for
a VARIABLE is likewise the join of ALL its definitions — a store `OpDatabase` minted into it
makes it Owned only where nothing else defines it (the retbuf a `materialized_view_return`
fills), and a variable minted once and then rebound by a call that may hand back its argument
is a `Join`, not `Owned`; reading the mint alone was the upgrade this paragraph forbids, held
right at run time by the distinctness guard (D-own-32, QUALITY-history.md B7r).

`(O-Borrow-Scalar)` is that principle applied one rule earlier, and it was written after the
alternative shipped a defect.  `deps` answers *what does this binding borrow from*, and a
heap link's answer happens to serve a second question — *how long must the source live* —
because the two coincide there.  A SCALAR link has no answer to the first: it owns no store,
so recording the target in its `deps` would say "this is a heap borrow" to every one of the
38 readers below, while leaving the second question with no channel at all.  It had none, and
the slot allocator — which ends a local's live range at its last use BY NAME — handed a
linked local's slot to a later local: the interpreter read, and WROTE, the usurper's bytes
through the link, while native was correct because a link there is a raw pointer to a Rust
local the compiler keeps alive (loft#1627, `sev:high`, silent, both a wrong value and an
`(O-NoDiverge)` divergence).  The cure is a second channel rather than a wider first one,
for the reason the paragraph below gives.

⚠ **The reason to write this down is that the choice is currently invisible.** 38 functions
test `depend().is_empty()`; some legitimately want the proxy (they are asking "is this a
view?", not "may I free it?"), some memo the oracle, and some free. Nothing in the source
distinguishes them, so a reader cannot tell a site that correctly reads one fact from a site
that reached for the wrong one — and both compile. `O-Proxy`'s "MUST also consult
O-Override" is the first checkable obligation in that space.

**Seventeen of the 38 decide a free, and six of those consult the override.** Measured in
the 2026-09 bug review ([BUG_REVIEW.md](../BUG_REVIEW.md)), which converted the largest
group: `Scopes::owns_freeable_store` is now the one home for *"may this function free the
store this source names?"*, discharging both obligations together — the `is_skip_free` veto
and the carve-out that a user PARAMETER belongs to the caller while the promoted NRVO buffer
is the one argument that is really a local. It replaced three copies inside `free_vars`,
extended separately by loft#688, loft#1022 and loft#1078; the keyed copy never gained the
promoted-buffer half at all.

⚠ **And the obligation was holding by ACCIDENT, not by construction.** Before the fold, none
of those three consulted `is_skip_free` — and the corpus never noticed, because no
`skip_free` binding currently reaches them (measured over `tests/scripts` and `tests/docs`,
zero hits). A site that frees on the proxy and happens never to meet a marked binding is
indistinguishable from one that asks correctly, which is the same invisibility this section
is about, one level down.

**The seventeen the gate cannot decide lexically now DECLARE what they ask**, which is the
cure the ⚠ above names — the choice is written at the site instead of inferred from it. Each
proxy site carries one of four verdicts, and the census is countable:

| declares | sites | what the empty dep list decides there |
|---|---|---|
| `free`   | 9 | ownership, and a free follows — **@FR-O-Override is required with it** |
| `copy`   | 8 | copy-vs-alias / materialise-vs-view; a wrong answer costs a copy, never a release |
| `alloc`  | 4 | whether to ALLOCATE or null-init a store — the opposite direction from a free |
| `oracle` | 3 | an independent derivation that drives no emission (@PLN94, witness accounting) |

A declaration is a claim, so the gate contradicts it where it can: a site declaring anything
but `free` while a free IS visible in the region it gates is reported rather than trusted.
What it cannot do is catch a site that declares `copy` and frees somewhere the region cannot
see — that residual risk is real, and it is a much smaller one than *"nothing in the source
distinguishes them, and both compile."*

⚠ This does **not** re-open `D-own-1` (CLOSED: *"every free/copy/move reads `deps`"*). That
remains true in the letter — these sites do read `deps`. What was never true is the
implication that reading `deps` is *sufficient*.

---

## Deviations

**OPEN: 0.**  Every deviation this doc has carried is closed; the record — `D-own-53` back to
the first — is in [ownership-history.md](ownership-history.md).  One shape keeps a store: a
closure capturing a VECTOR inside a loop, because the witness is record-typed; the obvious
widening answers wrong on `--native` (`D-own-38`).

> **A zero here is a claim to re-measure, and this is what its oracle covers.**  The join
> family is pinned by three files: `1323-every-arm-of-a-value-branch-has-its-own-binding`
> (every arm KIND of a bound value branch, the two shapes loft#1320 declined, the `??` hoist
> of a call, each on the 65535-store ceiling with a borrow-direction cell beside it),
> `1321-a-joined-binding-copies-what-a-plain-bind-copies` (the copy face, both spellings,
> the return position) and `1320-…` (the fn-ref arm that opened it).  Held FIXED, and
> therefore NOT measured by them: a fn-ref whose TARGET cannot be resolved — a fn-typed
> parameter or field — which is loft#1327 and reads `Owned` at every site that frees on the
> oracle; a `&` binding as an arm (it keeps the alias it had — a struct-`Enum` variable arm
> copies as a struct's does, `tests/scripts/a-struct-enum-whole-value-bind-copies-like-a-struct.loft`);
> and the `--native` release of a displaced store on a fn-ref re-bind of a USER local, which
> is loft#1328 and which the `??` hoist sidesteps by releasing in the IR.


The full register — every entry, open and closed, with its dates and issue numbers — is
the companion [ownership-history.md](ownership-history.md).

## Machine-checkable soundness — the @PLN94 flow-sensitive oracle (proof skeleton)

The register above is **validation, not a machine-checked proof**: the shipped fact is computed
flow-INSENSITIVELY (the join of all defs) and the `Join` case discharges through a runtime witness.
[@PLN94](../plans/94-cfg-ownership-dataflow/) builds the flow-SENSITIVE replacement — a monotone
dataflow fixpoint (`src/ownership_cfg.rs`) run BESIDE the shipped analysis as an independent oracle,
never driving codegen (SI-1). Being a textbook abstract interpretation, that oracle is the piece that
CAN carry a machine-checked proof. This section states the obligations and discharges them — the
substantive lemma (4), local transfer soundness, is now proved case by case below (hand-written prose;
a Coq/Lean rendering is the only rigour polish left). The result: the flow-sensitive fact is
**over-free-sound given the O-\* rules**, so a green check is a proof-backed over-free-freedom
certificate on every program where the oracle and shipped analysis agree.

**What is proved, and what is not.** The target is the **over-free** class only — no free of a store
the fact does not own (⇒ no use-after-free, no double-free of that store). It is **NOT** a no-leak
proof (under-free is a disjoint class the shipped leak-check owns — @PLN94's coexistence finding:
`LOFT_NO_JOIN_OWN` leaks past this oracle but not past the leak detector). And it proves the
**oracle**, not the codegen: the shipped path inherits the certificate only where the two agree,
which is why they run beside forever.

**(1) The abstract domain — DISCHARGED.** `OFact = ⊥ | Owned | Borrowed(b) | Join(b)` with meet `⊔`
is a join-semilattice (finite height ≤ 3), and `refines` is its partial order. *Proof:*
`ofact_meet_is_a_join_semilattice` + `ofact_refines_marks_precision_and_flags_the_unsound_direction`
(unit tests, `src/ownership_cfg.rs`).

**(2) The concrete property.** In [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩` with the
[heap.md](heap.md) store `H`, define at each program point the relation *owns(v)* = the var whose
binding is responsible for freeing `v`'s store (per **O-Owner**: exactly one). A free of `v` is
**sound** iff `owns(v) = v` at that point (**O-Derived**). Over-free = a sound-fact says `Owned`
where concretely `owns(v) ≠ v`.

**(3) The Galois connection.** `γ(Owned) =` { states where `owns(v)=v` }; `γ(Borrowed(b)) =` { states
where `v` aliases `b`'s store, `owns(v)=owns(b)≠v` } (**O-Borrow**); `γ(Join(b)) = γ(Owned) ∪
γ(Borrowed(b))` (runtime-dependent); `γ(⊥) = ∅`. `α` is the pointwise best abstraction. Obligation:
`γ` is monotone w.r.t. `refines` and `⊔` is its sound join — *straightforward from (1); to write.*
The join of two borrows with DIFFERENT bases is not `Join` of either — `Borrowed(a) ⊔
Borrowed(b)` is covered by no `Join(b)` — so the domain carries a base-less top below `⊤`:
`γ(Join(∅)) = γ(Owned) ∪ ⋃ₓ γ(Borrowed(x))`, spelled `Join { base: u16::MAX }`.  A reader of
it has no witness to compare against, so it decides per run by COPYING, the arguments
bracketed (@P290) so the copy's source-free releases only a store the callee minted.  The
implementation takes it at the CALL boundary (`use_analysis::returns_one_of_several_args`):
inside a body a second "base" may be the function's own literal backing (`q ?? [7, 8]`),
which the lattice cannot tell from a parameter, while a callee's return deps name exactly the
parameters a caller must witness (loft#1550, `D-own-45`).

**(4) Local soundness of the transfer — DISCHARGED for the over-free property (given the O-\* rules).**
The property the over-free check needs is **no false `Owned`**: wherever the fixpoint reports
`st(v)=Owned` at a site the check trusts (a free, a return), `v` genuinely owns a store there — and a
non-`Owned` fact authorizes no free. This is *weaker* than full `σ'∈γ(f)` (a `Borrowed`-where-owned
fact is a leak-direction imprecision, out of scope) and is exactly what obligation (2) defines as
over-free. The transfer (`ownership_dataflow`) is per-var — `st'[var]=f(rhs,st)` — so prove per RHS
shape that `f=Owned ⇒ owns'(var)=var`, and that a non-`Owned` `f` authorizes no free:

- **(a) `OpDatabase(var,…)` / a record `OpNewRecord` → `Owned`.** heap.md `alloc` mints a FRESH store;
  **O-Owner** ⇒ its unique owner is `var`. `owns'(var)=var`. ∎ *(independent)*
- **(b) projection `OpGet*(base,…)` → `Borrowed(root)`.** A non-`Owned` fact — authorizes no free; and
  it correctly names the view's owner (`borrow_base`'s root, **O-Borrow**). ∎ *(independent)*
- **(c) bare `var = u` → `st[u]`.** When `st[u]=Owned`, `u` owns a store, and `var=u` — a MOVE, or a
  non-move alias materialised as `OpCopyRecord` (a/e) — leaves `var` owning one either way, so
  `owns'(var)=var`. When `st[u]` is `Borrowed`/`Join`, `f` authorizes no free. The
  `unwrap_or_else(ownership_of)` boundary (`u` absent — a parameter) yields `Borrowed(u)`, non-`Owned`.
  ∎ *(the `Owned` sub-case is independent; see the bridged gap below for the moved SOURCE)*
- **(c′) bare `var = u` where `var` is `OpDatabase`-re-minted on some path → `Owned`** (the `reminted`
  rule, taking precedence over (c)). If `var` is the arg-0 of an `OpDatabase` anywhere in the body then
  **O-Owner** gives `var` a fresh store on that path, so `var` is a materialised OWNED local; a
  whole-value `var = u` copy into it is a materialised copy (`OpCopyRecord` at codegen, as a/e) that
  owns a fresh store — `owns'(var)=var`. NARROW by construction: it fires ONLY for a bare `Var` RHS,
  never a projection (a `OpGet*` view stays `Borrowed(root)` per (b)), so no borrowing view is
  manufactured `Owned` — the property that keeps the A1b returned-view catch intact. ∎ *(independent —
  rests on O-Owner + the C86 copy-materialisation, like (c)'s `Owned` sub-case)*
- **(d) non-native call `var = f(args)` → `call_own`.** `f=Owned` only when the callee
  `return_ownership` is `Owned` = *returns a fresh store* (**O-Move**), so `owns'(var)=var`; a
  `Borrowed(argᵢ)` return authorizes no free. Sound by induction over the call graph — the callee
  summary is (4) applied to `f`; the recursion back-edge is `Borrowed(⊤)`, never `Owned`, so no false
  `Owned` is manufactured. ∎ *(inductive over the call graph)*
- **(e) else.** Record → `Owned` (fresh, O-Owner). Scalar/literal → `Owned`, but it owns no heap store,
  so a "free" of it is a no-op — no over-free. Native op → `call_ownership` (as d). ∎
- **(f) `= null` skip.** No store minted; a free of a null DbRef is a no-op. ∎ *(independent)*
- **(g) self-borrow `Borrowed(var) → Owned`.** The @P302 self-dep `[s]` is an ownership marker (re-init
  in place), not a borrow — **O-Owner** ⇒ `owns(s)=s`, so `Owned` is correct. ∎ *(independent)*
- **(h) the meet `IN[b] = ⊔ₚ OUT[p]`.** `IN=Owned` **only if every** predecessor's `OUT=Owned`
  (`Owned⊔Borrowed=Join`, not `Owned`), so `owns=var` on every incoming path — no false `Owned` from a
  join. **O-Complete**: no arm is dropped. ∎ *(independent, from (3)'s sound join)*

**The one bridged gap (the over-reach guard — honest).** `OFact` has no `Moved` state, and the
transfer does NOT kill a moved-out *source*: after `var = u` (a move), `u`'s fact stays `Owned` though
`u` no longer owns its store. So the FACT is not a full sound abstraction for moved sources. The CHECK
is over-free-sound anyway, because **O-Move** forbids the shipped plan from *freeing* a moved source —
the sole site where the stale `u=Owned` could authorize a bad free never arises. This, plus the
interprocedural induction (d), is where the over-free guarantee rests on the O-\* rules rather than the
fact alone; the INDEPENDENT part is the flow-sensitive structure — the meet, the structural-op
classification, the self-borrow/null carve-outs. A disagreement in the rule-relative cases is exactly
what the shadow-diff (Check A) surfaces — which is why the two run beside forever.

**(5) Fixpoint soundness — from (1)+(4), DISCHARGED.** The round-robin least fixpoint over the CFG
converges (**≤ n+2** passes, asserted SI-3), and by (4)'s local soundness + monotonicity + Tarski, the
per-block OUT-state soundly over-approximates the concrete ownership at every reachable point (given
the O-\* rules). *Bound, convergence, and — with (4) now discharged — the soundness step all hold.*

**(6) The check corollary.** With (5): if the oracle's check is **GREEN** — Check B finds no
unconditional `OpFreeRef(v)` with `st(v) = Borrowed`, and Check A finds no fact the shipped analysis
disagrees with in the unsound direction — then the emitted plan performs **no over-free** of the
covered classes. Contrapositive is the A1b catch: the `LOFT_NO_A1B` plan returns a store the fact
reads `Join`/`Borrowed` while the shipped fact reads `Owned` → RED (verified end-to-end,
`tests/ownership_oracle.rs`), a wrong plan every runtime gate passes.

**(7) Coexistence conclusion.** The proven oracle is a machine-checked *certifier*: on every program
where oracle and shipped analysis agree (empirically: 505-corpus + 54-cell fuzzer + 377 scripts, all
0 RED), the program carries a proof-backed over-free-freedom certificate; a residual disagreement
indicts one side for adjudication. This upgrades the register's *"validated"* to *"the flow-sensitive
fact is over-free-sound given the O-\* rules; the shipped analysis is certified per-program by the
proven oracle running beside it."*

**Obligation ledger.** DISCHARGED: (1) lattice; (3) `γ` sound-join (used in 4h); **(4) local transfer
soundness for the over-free property (the substantive lemma) — proved case by case above; the
flow-sensitive structure (4a,b,f,g,h) independently, the interprocedural summary (4d) by induction,
and the one moved-source staleness bridged by O-Move**; (5) fixpoint bound/convergence (SI-3) +
soundness step; (6) the check corollary; backend fact-identity (SI-2, `tests/ownership_oracle.rs`);
no-crying-wolf at corpus + fuzz scale (empirical). REMAINING (rigour polish, not a gap): a
machine-checked (Coq/Lean) rendering of this prose — the argument is complete but hand-written. OUT OF
SCOPE: no-leak (under-free — the leak detector's + the `check-leak` scan's class); the `Join`
runtime-witness discharge (a separate `OpFreeRefIfDistinct` lemma); proving the shipped 8-mechanism
analysis directly (the certifier sidesteps it).

## Conformance

- **A fn-ref call's return records the argument it borrows, for EVERY kind (`O-Move`,
  loft#1335)** — `fn pick(x: integer, bag: Bag) -> hash<K[k]> { h = fn(q: Bag) -> hash<K[k]>
  { q.m }; h(bag) }` records `bag` (attribute 1) as what its return borrows, for a keyed, an
  optional keyed and an optional struct return exactly as for the vector control.  On the
  four-shape list it replaced, the keyed return recorded attribute 0 — the scalar `x` — which
  in a join with a local unioned attribute-space with frame-space deps (the debug-assertions
  gate's `dep-space violation`), and would have read as an owned store had the lift not
  re-derived the answer.  Guard `frame_vars::a_fn_ref_return_borrows_the_argument_in_the_callers_space_for_every_kind`,
  a FACT test, falsified on the list.

- **A `??` default-arm mint is released once (`O-Borrow`, loft#1322)** — `_vec_N` and
  `__vdb_N` name one store, and exactly one of them frees it: at the vector and keyed kinds,
  empty and non-empty defaults, nested, bound outside a closure, and at the store ceiling.
  ⚠ **No value can carry this verdict**, which is why the guard is split: a second free of an
  already-freed store is a no-op (`free_named` returns) and `LOFT_STRICT_STORES` does not flag
  it either, so `tests/scripts/1322-…` pins the SHAPES and `tests/redundant_free.rs` reads the
  only channel there is — `LOFT_TRACE_DB`'s `already_free=true` count. That file carries a
  `the_harness_can_see_a_redundant_free` cell for the same reason: without one, a trace that
  stopped reporting would read exactly like a clean tree.
  ⚠ **The flags this decision sets are CUMULATIVE ACROSS PASSES**, and that is what makes
  "does the view's free run?" a real question. A capture subject reads empty deps on pass 1 and
  takes the view model (`skip_free`), then reads non-empty deps on pass 2 and arrives at the
  other arm; silencing the record there too leaves the store with no owner at all. Asked as
  `is_skip_free`, which is which arm the variable ENDED in rather than which one a pass took.
  ⚠ **A residual with the same rule and a different pair of names:** a closure record is
  released through BOTH the fn-ref value and its `___clos_N` local, so its cascade runs twice
  and the second pass finds the capture's store gone. Pre-existing, and it is the cell that
  proves the instrument above works.

- **An opaque fn-ref return borrows its arguments (`O-Opaque`, loft#1327)** — a closure called
  through a fn-typed PARAMETER leaves the caller's vector intact over 300 calls with a filler
  allocation between them, at the nullable and dense parameter spellings and across a branch
  over both arms, while the same call in the frame that OWNS the closure and a named function
  passed through the same parameter are unchanged. Both backends. Guard
  `tests/scripts/1327-an-opaque-fn-ref-return-may-be-its-argument.loft`, 6 cells.
  ⚠ Declining a free normally trades an over-free for a leak; here it costs nothing, because
  the fn-ref call's own runtime buffer already owns what the closure minted. Measured at 70 000
  default-arm calls past the store table, both backends — not assumed.
  ⚠ Still open, and the parser cannot see it: a fn-ref LOCAL assigned two DIFFERENT lambdas is
  opaque in the same way, and the same free reaches it. `Scopes::fnref_target` is where that
  fact lives, one pass later than the typing this fixes.

- **A rebind from a call frees what it displaces, in BOTH call spellings (`O-NoDiverge`,
  loft#1328)** — `x = m(i)` in a loop over a fn-ref `m` completes at 70 000 iterations on both
  backends, at a nullable reference, a dense one and a keyed collection, with the direct-call
  spelling and a destination-reading callee as controls. Guard
  `tests/scripts/1328-a-fn-ref-rebind-frees-the-store-it-displaces.loft`, 6 cells.
  ⚠ The exit-leak gate cannot see this class: the frame frees everything, so a 1000-iteration
  cell reports no leak on either backend and the defect is entirely in the PEAK. The 65 535-store
  table is what turns that watermark into an accept/reject split — the channel a guard can
  actually score, and the reason these cells count past it.

- **The vector destination, at both spellings and both nullabilities (`O-NoDiverge`,
  loft#1329)** — loft#1328's sibling, and it reached the store ceiling three separate ways,
  each a different reader of ONE fact. `--native`'s displaced-free gate listed
  `Reference | Enum | keyed` and not `Vector`, where the interpreter's twin has carried the
  bare `Vector` arm all along. Then BOTH backends declined for a FORWARDING fn-ref, so they
  agreed and the shared fact was short: a capturing lambda's assignment is a block that mints
  the closure record, writes each capture into it and yields the `FnRef`, and a capture that
  is itself a fn-ref is an `FnRefDnr` argument of that write — so `fnref_target_in`'s tree
  walk saw two definition numbers and returned "names TWO targets", the answer reserved for a
  slot two lambdas were assigned to. A capture is a payload, not a candidate; the target is
  what the right-hand side YIELDS. Guard
  `tests/scripts/1329-a-fn-ref-vector-rebind-frees-the-store-it-displaces.loft`, 8 cells, 5 of
  which fail on `a8c0b74d`; the other 3 are its controls.
  ⚠ The NULLABLE spelling could not be measured at all until a third reader was fixed: a
  lambda declaring `-> vector<τ>?` aborted the compiler on the H5 two-pass contract, because
  the between-passes buffer sweep asked *"does this deliver a collection?"* through
  `ret_promo_base` (which peels `Optional(Vector)`) and then rejected the same definition on
  the RAW type, so pass 2 GREW the attribute. Its native twin — the fn-ref dispatch deciding
  whether to MINT that buffer — read the raw type too, and handed the callee `DbRef::NULL`
  to deliver through.
  ⚠ And `owned_ref`'s bare `Vector` arm carried a claim that this closed by measuring:
  *"a nullable vector already releases through its own path, and widening would free twice."*
  It does not. `x: vector<τ>? = null` grew the peak 1:1 with the iteration count on both
  backends, and the peel is safe because an `Optional` destination routes to the
  runtime-GUARDED post-free, which `free_displaced` no-ops on a same-store, free-protected or
  stack-record ref. A carve-out's own safety claim is a measurement, not a premise.

- **A closure record's suppression names the store it holds (`O-Latest`, loft#1324)** — a
  capture REASSIGNED after the build keeps the build-time store inside the closure (42 while
  the variable reads 52) and leaves no store unfreed: straight-line, twice over, in a 200-turn
  loop, at the `|x|` spelling, beside an untouched second capture, and for a closure that
  ESCAPES its defining frame. Both backends. Guard
  `tests/scripts/1324-a-reassigned-capture-suppresses-the-store-the-record-holds.loft`,
  12 cells.
  ⚠ It closed a use-after-free as well as the filed leak, and that is what says the cure had
  to NAME the right store rather than decline: the suppression was reading
  `function.tp(v).depend()`, which for a reassigned capture names the store the local holds
  NOW, so the store the record ADOPTED kept its frame-exit free and an escaping closure read
  it released. Declining to suppress stops the leak and leaves that half exactly where it was.

- **A call-shaped argument names its base (`O-Oracle`, loft#1318)** — a fn-ref `??` whose
  argument is `pick(vs, 0)`, `m[k].v` at the hash / sorted / index kinds, `vs[0]`, or a value
  delivered through the caller's own `__ref_N` buffer keeps the caller's container intact over
  five borrows, while the mint arm of the same closure still costs no store per call at 70 000
  iterations and a MINTING argument (`g(mk())`) is still adopted. Both backends. Guard
  `tests/scripts/1318-a-call-shaped-argument-names-the-store-a-fn-ref-may-hand-back.loft`,
  14 cells, 8 of which fail on `b1bd3212`; the other 6 are its controls.
  ⚠ Its two interpolated cells are interpolated deliberately: `s += g(vs[0])[1]` and
  `c = g(pick(vs, 0))[1]` are CORRECT on the broken build, so the accumulate and bind
  spellings of the same read score nothing. Statement context is an axis here.

This area's "falsifying programs" are the store-lifetime bugs themselves — each is a
program where the derived-free invariant (O-Derived) or completeness (O-Complete) fails
and a store leaks, double-frees, or a backend diverges. The area is **formal when OPEN
reaches 0**: when every store-lifetime decision is one `deps` read (O-Deps) over a complete,
typed fact, the bug class is closed by construction and `binding.md`/`types.md`'s
`deps`-fused rough spots (the `Deps`-in-`Type` fusion) resolve with it.
