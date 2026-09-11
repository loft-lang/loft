<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/ownership.md — the `deps` ownership / borrow system (strict; register at `OPEN: 0`)

**Catalogue:** @F21 (references `&T`), @I60 (deps / lifetime tracker) — Goal E. Roadmap: @PLN85, @PLN87.

> **Rules then deviations** (see [README](README.md)). The rules below are loft's
> ownership model.  The register is back at **`OPEN: 0`** — `D-own-40` and `D-own-41` both
> opened and CLOSED 2026-09-11, below.  It read `OPEN: 0` from 2026-07-04 until then, and what
> moved it was not a new defect but a VALIDATION of loft#1517 against these rules: the zero had
> been re-measured against its oracle, which covers the JOIN family and does not ask whether a
> witness should EXIST at all.  A zero that an oracle cannot disturb is only as strong as the
> questions that oracle asks.  The five original `D-own-*`
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
  (O-Derived)   FREE PLACEMENT IS DERIVED, NOT DECIDED.  Free a local iff it owns its store
                and does not transfer it out — once, at scope exit.  No per-site heuristic.
  (O-Complete)  PER BINDING, PER PATH, COMPLETE.  Every binding, including every `match`/`if`
                arm — a set-and-reconcile, not a single-variable structural walk.
```

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
*binding surface* declines the program ([binding.md](binding.md) B-Ref-Reshape, C79 revisited
2026-08-05). That is a decision about what `&` MEANS, not a lifetime the checker failed to
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
hand drifted four times, each found by a leak or an abort on one backend only (QUALITY.md
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
read (loft#1336 follow-up, QUALITY.md B7v).

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
right at run time by the distinctness guard (D-own-32, QUALITY.md B7r).

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

⚠ **And the "seventeen decide a free" above is a HAND count the gate could not reproduce.**
Re-measured 2026-09-03, after `scripts/o_proxy_check.py` was given a decidable predicate for
*"does this site reach a free?"*: of 24 positive sites **6 reach one, and all 6 discharge the
veto**. Two of those six were undischarged until that run — `scan_set`'s displaced-owned dep
strip and `gen_set_first_ref_var_copy`'s move — and both now consult it.

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

⚠ **The pass corrected one of its own conclusions, which is why it is worth writing down.**
`parse_field_iteration` looked like a `free` site and its own prose said so — *"a
borrow/skip_free binding owns no allocation"* — so it was first declared `free` and given the
veto. The differential probe then reported **8 of 1119 corpus files** reaching it with a
`skip_free` binding, i.e. a live behaviour change rather than a latent guard. Reading the
mechanism settled it: the frees that follow are of FRESH per-field bindings
(`copy_variable` + `remap_var_deep`), never of the binding tested — so the veto does not
belong there, and the site is `copy`. **A site's own comment is not a measurement**, and the
prose there still overstates its filter.

⚠ This does **not** re-open `D-own-1` (CLOSED: *"every free/copy/move reads `deps`"*). That
remains true in the letter — these sites do read `deps`. What was never true is the
implication that reading `deps` is *sufficient*.

---

## Deviations

**OPEN: 0.**  `D-own-40` (`(O-Witness)` armed for locals whose assignments do NOT mix, and
`(O-Owner)` broken while one was) and `D-own-41` (a detach by in-place re-allocation, wiping the
value being copied in) both opened and CLOSED 2026-09-11, below; `D-own-39` opened and CLOSED
2026-09-10.  Every earlier deviation this doc has carried is closed; the
record is in [ownership-history.md](ownership-history.md).  `D-own-38` (loft#1388) was
closed by `(O-Witness)`: every release a captured local owes is now by STORE IDENTITY, with the
hand-off at the closure build placed ahead of it for `(O-Detach)`'s ordering.  One shape keeps
a store — a closure capturing a VECTOR inside a loop, because the witness is record-typed — and
the entry records why the obvious widening is not taken: it answers wrong on `--native`.

> **A zero here is a claim to re-measure, and this is what its oracle covers.**  The join
> family is pinned by three files: `1323-every-arm-of-a-value-branch-has-its-own-binding`
> (every arm KIND of a bound value branch, the two shapes loft#1320 declined, the `??` hoist
> of a call, each on the 65535-store ceiling with a borrow-direction cell beside it),
> `1321-a-joined-binding-copies-what-a-plain-bind-copies` (the copy face, both spellings,
> the return position) and `1320-…` (the fn-ref arm that opened it).  Held FIXED, and
> therefore NOT measured by them: a fn-ref whose TARGET cannot be resolved — a fn-typed
> parameter or field — which is loft#1327 and reads `Owned` at every site that frees on the
> oracle; a `&` binding as an arm (it keeps the alias it had — a struct-`Enum` variable arm
> was in this list until 2026-09-04, when the `@FR-B-Copy` walk gave it the copy lowering
> a struct's has, `tests/scripts/a-struct-enum-whole-value-bind-copies-like-a-struct.loft`);
> and the `--native` release of a displaced store on a fn-ref re-bind of a USER local, which
> is loft#1328 and which the `??` hoist sidesteps by releasing in the IR.

### D-own-41 — OPENED AND CLOSED (2026-09-11): the interpreter detached a local by re-allocating its store IN PLACE, wiping the value being copied into it

`(O-Detach)` — DETACH AFTER THE READS — names *"the free, sentinel or RE-ALLOCATION that stops
[a binding] naming its current store"* and requires it after every read of that binding by the
value being assigned.  A NULLABLE heap local reassigned from a call whose return BORROWS a
parameter that reads it answered the type's ZERO on `--interpret` and correctly on `--native`:

```loft
fn keep(s: K) -> K { s }
c: K? = mk();  c = keep(c ?? K { x: 9 });     // --interpret 0, --native 7
```

Two lines, no loop, no literal, **live on `main`** — and a wrong value with no diagnostic, which
makes it the more serious half of the loft#1517 arc even though it was found second.

The interpreter DID defer the re-allocation past the call, which is what the rule asks.  It then
performed it IN PLACE on the local's own store: `OpDatabase(c)` cleared the record and the
`OpCopyRefOrNull` after it copied from the call's result — which was that same store, because
`keep` hands back what it was given.  `--native` has a runtime same-store passthrough for exactly
this (`generation/dispatch.rs`'s `PASSTHROUGH`, whose comment cites this rule), added when the
backends were wrong the other way round (loft#1017b).  So `(O-NoDiverge)` was broken by a fix that
had only ever been applied to one side.

⚠ **That is a hiding place, not bad luck, and it generalises past this entry.**  This is one
question answered by two decoders — the usual shape — but here the two decoders are the two
BACKENDS, and the instrument that normally finds that shape is a differential run comparing them.
Repairing one backend and not the other therefore breaks the very cross-check that would report
it, and the defect becomes invisible to the @PLN89 oracle by construction: the oracle asks whether
the two agree, and the repair is what made them disagree.  A same-store, same-rule fix landed in
`generation/dispatch.rs` or `state/codegen.rs` alone should be read as an OPEN deviation in the
other until measured, whichever direction it goes.  Measured cost of not doing that here: a
two-line wrong value shipped for a cycle, found only because an unrelated arming defect was being
corrected on top of it.

**Two reasons it stayed invisible**, and they are the reusable part:

- The condition that forces a FRESH destination was gated on the local being **WITNESSED**, and
  `(O-Detach)` carries no such qualifier — a guard narrower than the rule it cites, the third
  instance of that shape in two days.  A witnessed local took the fresh-store path and was
  correct, which is also why loft#1517's misclassification MASKED this: the locals that reach
  this path were being witnessed for a false reason, and that reason was load-bearing.
- Its other half, `value.reads_var(v)`, is SYNTACTIC and could not see the case at all.  A `??`
  (and an `if`-valued arm) HOISTS the read of the local into a temporary in a PRIOR statement —
  which is the lowering `(O-Detach)` itself prescribes — so the assignment arrives as
  `Set(c, Call(keep, [__lift_1, …]))` and mentions `c` nowhere.  **The hoist moved the READ, not
  the VALUE**: `__lift_1` is bound to `c` and names c's store, and its dep list says so
  (`["__ref_p2_1", "c"]`).  A predicate written from the source spelling matches nothing here and
  looks inert.

**CLOSED** by the ORACLE's borrow BASE: a `Borrowed`/`Join` whose base may hold `v`'s store —
`base == v`, or `base`'s dep list names `v`.  The `??` hoist temp carries exactly that dep
(`__lift_1` reads `["c", "__ref_p2_1"]`), because the hoist moved the READ and not the VALUE.

⚠ **Two other facts were tried first and BOTH are wrong, in OPPOSITE directions, and all 70
targeted tests across eight guards were green on each.**  Recorded so neither is retried:

- **`value.reads_var(v)` widened into the gate.**  Answers NO for the defect (hoisted) and YES
  for `s = grow(s)`, which NEEDS the in-place destination — one store leaked per loop pass,
  `303-ref-reassign-free.loft`, `R3×4` and `N3×4`.
- **`Def::returns_borrowed_view`** (the return's dep names a visible param).  True for a callee
  that MINTS its record and merely carries a dep through a `text` field —
  `fn grow(x: N3) -> N3 { N3 { name: x.name + "x", v: x.v + 1 } }` reads as borrowing and hands
  back a fresh store — so it leaks the same population.  A return type's dep list is not an
  ALIASING fact.

And the middle is narrow rather than the edges merely being wrong: the first cut over-reached into
the WIPE (`1184-a-view-assigned-back-onto-its-own-source`, `len 0` where 8 is right, six cells,
plus a `??` default arm's mint released twice), the narrowing over-reached back into the LEAK, and
the oracle-alone form does not separate them either — **1184's failing cells and the defect share
the signature `reads=false, Borrowed`**.  What separates them is only the base: the defect's base
carries a dep naming `v`; 1184's bases (`w`, `d`, `target`) have empty dep lists and are genuine
views of ANOTHER binding, which is why that population needs the in-place destination — it is
assigning a view back onto its own source.

⚠ **All of it was found by an env-gated probe inside the gate, after two wrong answers reasoned
from predicates that were in scope.**  `(O-Detach)`'s three candidate facts are indistinguishable
by reading; they separate on one line of `eprintln!`.  And ⚠ **a plain run shows none of it**:
`--interpret`, `--native` and `--tests` on `303` are all clean, because only `wrap`'s in-process
SUITE run arms the leak gate (`--tests` skips it).  Two of us checked 303 the natural way and
concluded it was fixed.

Guard `tests/scripts/a-reassignment-from-a-borrowing-call-does-not-wipe-its-own-source.loft`, 11
cells.  Its sharp control is `test_the_callee_returns_the_other_parameter`: the callee's return
borrows, but not the parameter carrying the local, so it says the condition is *"the result may be
the destination's own store"* and not *"the callee borrows something"* — and it is where the
remaining conservatism (a fresh allocation bought for nothing) is visible as a choice.

### D-own-40 — OPENED AND CLOSED (2026-09-11): `(O-Witness)` was armed where its premise is false, and `(O-Owner)` broke while it was

`(O-Witness)` conditions the runtime witness on the local's assignments MIXING ownership — *"one
assignment hands a heap-record local a store of its own and another hands it a VIEW"*.  Two
measured populations carry a witness with no mix at all, so the release placement `(O-Override)`
then vetoes is replaced by one that does not cover the same deaths.  This is the ownership half
of loft#1517; [heap.md](heap.md) `D-heap-6` is the `(H-Drop)` half and carries the guard.

- `x: SE = A { k: 3 }; x = a` — a CONSTRUCTION and a whole-value COPY.  Classification arm (e)
  makes a record construction `Owned` (fresh, `(O-Owner)`).  For the copy the rule is
  `binding.md (B-Copy)`, which names this spelling outright — *"a heap WHOLE-VALUE … a
  struct-enum `c = e`: the bound variable is INDEPENDENT"* — so it is a second record the
  binding alone names, which is owning.  (The dataflow arm that carries it is not cited here:
  (c′)'s `reminted` test is about the LOCAL being `OpDatabase`'s arg-0, and a struct-enum
  literal mints into a work-ref instead, so naming that arm would be an attribution rather
  than a reading.)  Two owning assignments, no view, and a witness.
- `c: K? = K { x: 7 }; c = keep_k(c ?? …)` where `fn keep_k(s: K) -> K { s }` — a CONSTRUCTION
  and a call whose return BORROWS its parameter.  `(O-Move)` is explicit for that case: *"if the
  return borrows a parameter, the return type records it and the caller COPIES to obtain its own
  store"*.  So the local owns after the call, which is measured rather than read off the rule —
  `c.x = 99` leaves the source at `7` on both backends, where a `(B-View)` projection writes
  through.  Two owning assignments again.

The misclassification is one `Other` read by two sites for two different questions (D-heap-6 has
the mechanism).  What belongs HERE is the consequence for these rules, and there are two:

**`(O-Owner)` is broken for as long as the witness is armed.**  A construction delivers through a
work-ref, and `scan_set`'s hand-off disarm — which writes the sentinel into that work-ref so the
binding becomes the store's one claimant, `D-heap-5`'s cure — is conditioned on
`proxy_says_owned`, which a witnessed local is not.  So the work-ref and the local both name the
store: *"every heap store has exactly one owner at any moment"* does not hold on that path.
⚠ `witness_set_kind`'s own comment reasons FROM that state — the store is *"the work-ref's and
not solely the local's"* — and concludes the local does not own.  The rule says the state should
not exist; the disarm should fire.  Measured: with the witness suppressed the disarm DOES fire
and the local adopts the store outright, so `(O-Owner)` holds for every non-witnessed spelling.

**The cure ORDER was settled by these rules, not by a design call.**  `D-heap-6` recorded it as
an open question — either the other release path mis-frees a local the witness was covering, or
such a local must stay witnessed.  `(O-Witness)` and `(O-Move)` decide it: the local must NOT be
witnessed, so the witness on it is this deviation and the other path's wrong answer (`0` where
`7` is right) is the defect to fix FIRST.  *"Keep the witness because removing it breaks
something"* was never an available answer.  That second defect is `D-own-41` below, it turned out
to be LIVE on `main` independently, and fixing it is what let the arming correction land.

**CLOSED 2026-09-11** by `heap.md` D-heap-6's Cure A: `is_view_of_storage` answers `false` where
`construction_work_ref` names a work-ref, so a construction is no longer counted as the view half
of a mix.  `(O-Owner)` follows from the same change — an unwitnessed local reaches the hand-off
disarm, so the work-ref stops naming the store and the single-owner property holds on that path
again.  Guard: `a-record-local-reassigned-after-a-literal-build-releases-what-it-displaces.loft`,
whose `test_mixed` and `test_null_then_call_field` cells are the two directions a cure could
over-reach in.

⚠ **No gate saw this, and the reason is structural — closing it changes nothing there.**
`(O-Override)`'s gate is `ownership_cfg`'s Check D (`LOFT_OWN_ORACLE=check`), which reds on a FREE
of a never-free binding.  Run over both guards it reported `clean — 0 RED`, because nothing frees
illicitly: what was missing was a DROP.  `(H-Drop)`'s own ⚠ says a drop's two failures are not ordered the way a
free's are, and this is that asymmetry showing up in the instruments — the free side has a
checker and the drop side has only per-guard traces.  A gate for `(H-Drop)`'s three deaths is
the gap, and it would have caught both populations above.

### D-own-39 — OPENED AND CLOSED (2026-09-10): a per-path hand-off to a SHARED destination

`(O-Complete)` makes the ownership fact per binding and PER PATH.  A whole-value copy written
inside a branch arm moves a release on the runs that take that arm and on no others, and the
hand-off is recorded once for the statement — so the arm that did not run leaves its source
with nothing to release it, and the resource is never released at all, silently, on both
backends.

The ARM-LIFT half of this is CLOSED (loft#1514, `scopes::handoff_target`): a value `if`/`match`
lifts each arm's value into a temp of its own, and because each temp is null until its own arm
assigns it, keeping the release with the SOURCE and stopping the DESTINATION is correct on
every path with no runtime witness.  Guard
`a-branch-arm-releases-the-source-the-other-arm-took.loft`, 19 cells, 12 moving.

What stays open is the case where the arms assign one SHARED local:

```loft
a = mk(1); b = mk(2); x = mk(3);
if c { x = a; } else { x = b; }        // c=true releases 3 and 1, never 2
```

The per-arm rule does not reach it and must not: `x` genuinely owns what it took on the path
that ran, so stopping `x` would lose THAT release instead of restoring the other.  Neither side
can be chosen statically, which is the whole of the deviation — `(O-Complete)`'s "per path"
has no representation at a destination that is shared across the paths.

✓ **CLOSED by MATERIALISING the path fact** (loft#1515): `Scopes::handed_off` is one boolean
per source, false at entry, set where the copy runs, read at the source's scope-end release and
cleared when the SOURCE is reassigned.  Not a new mechanism — loft#1200's `local_owns` already
materialises sole ownership for the free it guards, and this is the same answer for the drop.
Guard `a-shared-destination-releases-the-arm-that-did-not-run.loft`, 14 cells, 12 moving.

⚠ **Two cures were measured and rejected, and both fail in ways that reading them does not
show.**  A DISTINCTNESS WITNESS (`if !OpEqRef(a, x) { … }`) rests on the two naming one record,
and `binding.md (B-Copy)` makes the plain bind COPY — so it is true on every path and would run
the cascade where the copy already owns the resource, this deviation with the sign flipped.
And giving `drop_transferred` the per-path intersect-merge `owned_refs` has turns every lost
hook here into a DOUBLED one, which `(H-Drop)`'s own clause rules out: a drop has no safe
direction.  That merge is INERT on its own besides — measured byte-identical on 19 cells,
because the seed's whole-body walk and the statement-level re-arm both arm the fact before
`scan_if` descends into the arms.

⚠ **What the fix does NOT reach, and the cell that says so.**  A struct-ENUM local loses its
hooks at an UNCONDITIONAL bind (loft#1517) — `a: SE = A{…}; x: SE = A{…}; x = a` releases
neither the displaced record nor the copy, with no branch in sight — so the guard's `c_enum`
cell moves to ONE hook of three rather than three.  It is kept for exactly that: a fix here has
a place to be scored, and the residual is visible rather than implied.

✓ **The RETURNED value branch CLOSED 2026-09-11** (loft#1515 shape 2), and it needed no runtime
witness at all — where the shared-destination half had to materialise the path fact, here the
ARM *is* the path.  `scopes::move_join_hooks_into_arms` reads what each arm hands out —
a MEMBER of a local, that local WHOLE, or nothing — and gives each arm the hooks its own path
owes: the arm handing `src.h` out gets the `(skip, depth)` cascade over everything else, the arm
handing nothing out gets the whole record, the arm handing the record out gets nothing.  It is
`free_record_in_omitting_arms`'s shape (loft#1476) one level over.

**Only the HOOK moves; the `OpFreeRef` stays in the common sweep** — a store is freed once
whichever arm ran, and splitting that would free it per path.  `(H-Drop)` and `(H-Free)` want
the same placement here for opposite reasons.

Two rungs were needed and the first is not landable alone.  `classify_ret_promotion`'s
`wrong_shape_for_buffer` refuses a candidate that does not have the return's own SHAPE, and it
opened on `Type::Vector` while the rule it cites carries no such qualifier — so a `Two` local
was renamed onto a `CfH` buffer, `is_argument` became true, and `(H-Drop)`'s parameter clause
declined the cascade on EVERY path.  Widening it alone trades the lost release for a double,
which for a DROP is not a safer direction.

The rewrite is IDEMPOTENT by a marker on the node (`materialized_view_return_armed`), because
the scan runs in more than one phase over a body the previous phase installed: without it the
second phase injects a second copy of every hook and the delivering arm releases twice —
measured, not anticipated.

Guard `1515-a-join-return-releases-each-arms-source-once.loft`, nine cells, both backends
byte-identical and clean under `LOFT_POISON=1`.  Three are load-bearing: `nested_with_sibling`
(the delivering arm skips `p.a` and must still release `p.b` AND `q` — the only cell an
offset-only skip fails), `two_fields` (the delivering arm keeps the sibling member, which is
what a whole-cascade suppression loses), and `loop_before_return` (the back edge).  `two_exits`
is the control: two separate `return` statements are two sweeps and were always right.

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
