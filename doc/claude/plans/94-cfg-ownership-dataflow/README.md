<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 94 — CFG + dataflow fixpoint for free/ownership facts

## Status

**✅ DONE / MERGED — landed on `main` via PR #524 (`9b38e1b6`), closed 2026-07-08.** Phases 0–5 all landed; `LOFT_OWN_ORACLE=check` runs three
gates (Check A fact-disagree/A1b, over-free Check B, definite-leak scan) at **0 RED across the whole
corpus**, each with a firing true-positive. Remaining items are non-blocking refinements: a Coq/Lean
mechanisation of the (discharged) formal prose, and the self-contained A1b catch (deferred — its
correctness criterion collapses onto the shipped materialisation decision; A1b is caught by Check A).
Out of scope by declaration: the P4/VH codegen cutover + perf fork.

**H-tier END-STATE REACHED — Phases 0–5 all landed.** The oracle runs BESIDE the shipped analysis on
every `cargo test` (`tests/ownership_oracle.rs`), a PURE OBSERVER (SI-1: shipped codegen byte-identical;
reached only via `LOFT_OWN_ORACLE`, nothing in the compile path consumes it). `LOFT_OWN_ORACLE=check`
now catches **both directions on the default path** — over-free (Check A, the A1b catch, RED on
`LOFT_NO_A1B`; and the promoted Check B `run_over_free_check`, an unconditional free of a dep-carrying
view) and definite under-free/leak (the promoted `check-leak` scan) — three independent gates, each
with a firing true-positive and 0 false positives across ~521 files. A machine-checkable-soundness
proof skeleton is in `formal/ownership.md`. **Merged to `main` via PR #524** (`9b38e1b6`); the
`tuxedo-pln94-ownership-dataflow` branch is removed. **What's still open** (refinements, not the end-state): the formal lemma
(local transfer soundness) is now DISCHARGED (`formal/ownership.md`, over-free property given the
O-\* rules; only a Coq/Lean rendering remains); the self-contained A1b catch stays DEFERRED — a second
build (2026-07-07) fired correctly on the `LOFT_NO_A1B` UAF but had 55 FPs on correct code, sharpening
the blocker to **materialisation-awareness** (deep-copy-vs-shallow-ref in container-insert ops), not
base resolution (`n_mk` appending a `vector<u8>` deep-copies = safe; `n_h` appending `vector<ref(E)>`
stores refs = UAF, structurally identical IR) — reverted, A1b stays on Check A (see PHASE4_DESIGN).
The `n_choose` fact-disagree residual is now RESOLVED: a
`var = src` copy INTO a var re-minted via `OpDatabase` (a `Call`, invisible to the `Set`-only transfer)
owns — so `check` is now **0 RED across the entire corpus** (806 files + 54 fuzz), and
`85-struct-copy-return-owned` joined the clean corpus. The `check-dev` over-free Check B is **now PROMOTED** onto `check` (its
true-positive is `LOFT_OWN_INJECT_FREE_BORROWED`); `check-dev` retains only the exit-state Check C as a
second opinion. **Check D** (2026-09-05, the `@FR-O-Override` walk, QUALITY-history.md B7q) also runs on
`check`: every free op — in any of the five spellings `OpSets::frees` names — whose first argument is a
never-free (`skip_free`) binding is a RED in a spelling the backends do not intercept and a NOTE in one they
drop; the admissible exception is a staged text temp (`Function::is_staged_text_temp`) freed by the pass
that staged it.  Its true-positive is `LOFT_OWN_INJECT_FREE_SKIPFREE=<var>`; 0 RED / 0 NOTE over 1247 files. **Check A's
true positive is injected too** since the `@FR-O-Oracle` walk (2026-09-05, QUALITY-history.md B7r):
`LOFT_OWN_INJECT_FACT_OWNED=<var>` forces the shadow's fact to `Owned`.  The A1b disagreement it used
to assert was two derivation defects meeting on one fixture — the oracle's "minted ⇒ Owned regardless"
shortcut and the shadow's private, drifted base translation — both closed; the shadow now reads
`use_analysis::structural_arg_base`, the one translation home, and Check A reads 0 over 1247 files.
`LOFT_OWN_ORACLE=own` also prints one `RETSUM` line per heap-returning function, the deps proxy
`return_adopts_fresh_store` beside the oracle's return class. The leak scan's **adopted-owned class is now PROMOTED** too (an NRVO return buffer
`__ref_*` a function owns + frees but never `OpDatabase`-mints — 0 FP across 829 files, its own
`LOFT_OWN_INJECT_DROP_FREE=__ref_1` true-positive). Remaining leak gaps by design: conditional/`Join`
leaks (the runtime leak-check's class — coexistence) and closure bodies (frees on a different codegen
clock). Out of scope (declared): P4/VH codegen cutover + the perf fork (Open q4). Progress by phase:

- **Phase 0 ✓** — falsification gate PASSED: `LOFT_NO_A1B` produces a backend-consistent wrong plan
  the interp-vs-native oracle passes (`len=0` on both), and an independent static check catches it.
  The oracle is not redundant.
- **Phase 1 ✓** — a structured CFG over the Value-IR (`src/ownership_cfg.rs`) + a monotone worklist
  dataflow fixpoint, validated on reaching-defs across every control-flow shape (straight-line,
  if-join, single/nested loop, break-out, early-return-in-loop). 5 unit tests; SI-3 (bounded
  convergence) enforced by assert.
- **Phase 2 (ownership fact) COMPLETE — 3.1 ✓ / 3.2 ✓ / 3.3 ✓ / 3.4a ✓ / 3.4 ✓ / 3.5 ✓.** Forward
  flow-sensitive `OFact = Bottom|Owned|Borrowed(base)|Join(base)`; transfer reuses `ownership_of` for
  structural RHS, resolves `Var` RHS flow-sensitively, and consumes callee `return_ownership`
  summaries at call sites (independently of the shipped classifier). Shadow-diff vs `ownership_of`
  splits AGREE / PRECISION (mine ⊏ B's `Join`) / DISAGREE (must be 0 for soundness). **3.4a
  (2026-07-07): the capture probe's lone disagreement was a real UNSOUNDNESS in the oracle, not a B
  fork.** The 3.3 call-arm guard mis-routed the primitive structural ops `OpGetField`/`OpNewRecord`
  (Function-typed, empty native) to the summary path, so a projection local `xs = OpGetField(vdb,…)`
  read `Owned` where it must read `Borrowed(vdb)`. Excluding structural ops
  (`classifies_structurally`) fixed it; **all 22 "3.3 divergences" were this one bug** — the corpus
  is now **DISAGREE=0 across 712 fns**, with the precision win and interproc independence intact. The
  cross-check's payoff landed inward — an independent impl caught the oracle's own gap. Genuine
  op-tail families (coroutines, `par`) remain but surface no divergence on this corpus.
- **Phase 2 COMPLETE — 3.5 ✓ (2026-07-07): fuzzer soundness sweep + A1b payoff.** The `own`-mode
  shadow-diff is **DISAGREE=0 across the @PLN85 ownership fuzzer's 54 cells** (9 shapes × 2 values × 3
  churn, both backends, SI-2 verified). The sweep first flagged the `local_source` (`#462`
  conditional-local-view) shape — a SECOND real unsoundness distinct from 3.4a: a `??` null-coalesce
  temp (`skip_free`, never freed) read `Owned` because the transfer defaulted its `= null`
  declaration to `Owned` where B skips `= null` sentinels. Fixed (skip `Null` owns entries). And the
  **A1b payoff is demonstrated**: on the canonical `85-…-uaf.loft` the oracle is clean under the
  correct default (`n_h disagree=0`) and **flags** the wrong plan under `LOFT_NO_A1B` (`__ref_1:
  mine=Join / B=Owned`). Two oracle unsoundnesses found + fixed via the inward cross-check; the
  framework works end-to-end.
- **Phase 4 DONE ✓ (design: [`PHASE4_DESIGN.md`](PHASE4_DESIGN.md)).** `LOFT_OWN_ORACLE=check` runs
  the consistency checks BESIDE the shipped analysis — **both flag, neither replaces the other**
  (overhead ~1.6%, gated-off zero). Check A (shadow-diff — the A1b catch, RED on `LOFT_NO_A1B`, clean
  on the fix) is the independent over-free cross-check; a SELF-CONTAINED A1b invariant was probed +
  FALSIFIED before coding (base `65535` unresolved on both safe and unsafe returns), then a second
  build (2026-07-07, `OpFreeRefIfDistinct(_, return)` + escaped-view) fired on the UAF but hit 55 FPs on
  correct code — the blocker is materialisation (deep-copy-vs-ref), not base resolution; reverted, A1b
  stays on Check A. Check B (over-free, `run_over_free_check`) is now PROMOTED onto `check` (0 FP, own injected
  positive control `LOFT_OWN_INJECT_FREE_BORROWED`); the exit-state Check C stays gated `check-dev` as a
  second opinion. The fact-precision insight (use the post-codegen type dep) drove them 153 → 0.
- **Phase 5 DONE ✓ — landed beside.** `tests/ownership_oracle.rs` runs the oracle on every `cargo
  test`: clean-corpus + fuzzer-hook (54 cells) + RED-on-`LOFT_NO_A1B` + SI-1 observer + SI-2
  backend-identity. Phase 5.4: the machine-checkable-soundness proof skeleton in `formal/ownership.md`
  (obligation ledger; one open lemma = local transfer soundness).
- **Under-free / leak (Check C) — BUILT + PROMOTED ✓ ([`CHECK_C_UNDERFREE_DESIGN.md`](CHECK_C_UNDERFREE_DESIGN.md)).**
  The definite-leak scan (`run_leak_scan`) now runs on the DEFAULT `check` path over TWO owned-store
  classes: a MINTED (`OpDatabase`) var AND an ADOPTED work-ref (`__ref_*`/`__rref_*`) NRVO return
  buffer a function owns + frees but never `OpDatabase`-mints (a type-dep phantom like `__retbuf` has
  no store — excluded because it never appears as a `Var` arg). Transferred = returns (closed
  transitively through the dep) ∪ consumes/captures (not closed). Drove its own `check-leak` ratchet
  **927 → 0** (the `__retbuf` phantom was ~889), then folded in the adopted class (0 FP across 829
  files, no ratchet needed) — two injected-free positive controls prove it fires (not vacuous). BY-DESIGN
  GAPS (not FPs): conditional/`Join` leaks (the runtime leak-check's class — coexistence), closure
  bodies (frees on a different codegen clock). The dev-tier + ratchet workflow (a check that RAISES the count gets its own flag, never a
  revert) is distilled into a nudge in the engineering-rigor skill.

The remaining approximations this replaces still ship: `src/use_analysis.rs` analysis **A**
(position-proxy, valid only outside loops) and analysis **B** (`Owned/Borrowed/Join`, flow-insensitive
join of ALL defs). **Per-step detail, how to run, and the resume checklist live in
[`IMPL.md`](IMPL.md).** This plan does not change the language, the deps representation, or the
"never reject" contract — only how (and how precisely) the facts are computed and cross-checked.

## Goal

Compute every store-lifetime fact (liveness/last-use, `Owned/Borrowed(base)/Moved`, `Join`)
as the least fixpoint of a monotone transfer function over each function's control-flow graph,
consumed by one oracle every free/own/elision site reads — replacing the position-proxy and
the flow-insensitive join, and making O-Deps + O-NoDiverge structural.

## Effort + design

- **Effort (tiered):** **VH** for the full replacement (P0–P4); **H** for the oracle end-state
  (P0–P3 — build the framework, run it beside forever, no cutover, no fast-path perf constraint);
  **MH** for first value (a liveness-only oracle that already catches A1b-class divergences without
  being complete). The cost driver is the **per-op transfer-function long tail** over loft's
  irregular op surface + modeling loft's (non-Rust) ownership semantics + the P4 cutover — NOT the
  (textbook) CFG/lattice/fixpoint machinery. The oracle-first ordering lets most of the safety value
  land at H (or MH for the first catch) and defers the VH tail indefinitely.
- **Design:** ~ (this README is the skeleton; the lattice + transfer functions get their own
  `DATAFLOW.md` in Phase 0 before any code)
- **Last touched:** 2026-07-07

## The invariant (the design hypothesis)

> Every store-lifetime codegen decision — **free placement**, **own-vs-borrow
> materialisation**, and **move-elision eligibility** — is a pure function
> `f(program-point, dataflow-state)`, where the dataflow-state is the **least fixpoint of a
> monotone transfer function over the function's CFG**. Computed once per function, consumed
> everywhere, backend-agnostic.

This is the **recover-a-known-construction** case, not an open design space: monotone dataflow
(lattice + transfer functions + worklist fixpoint) is textbook, and rustc's MIR borrow-checker
(+ Polonius) is the reference. The design work is *adapting* that construction to loft's
structured IR, not inventing one. The falsifiable core is small and gets probed in Phase 0.

## What actually makes it hard (the crux — it is NOT the dataflow machinery)

rustc's borrow-checker is a **pure analysis over a fixed program with a reject valve**. loft's
ownership analysis is a **self-referential analysis-AND-rewrite whose output is the program it
must be correct about — and it cannot reject.** Three facets, in order of bite:

1. **Analysis ↔ emitted-code mutual recursion.** The ownership fact *decides* the free / copy /
   move; but a move-elision rewrite *changes* the store lifetimes, which changes the fact. Precise
   ⇒ a **joint fixpoint of facts-and-emitted-code**, not a one-way pass. rustc never has this —
   borrowck observes fixed MIR and errors; drop elaboration (the transform) runs after, separately.
2. **No reject valve, self-referential truth.** rustc rejects when unsure (safe fixed default);
   loft must emit *some* code, and "correct" is defined relative to the code it is emitting — a
   wrong fact is a UAF that surfaces only when that exact code runs, not a catchable error.
3. **Shared mutation removes the exclusivity rule.** "Aliasing XOR mutability" is what makes rustc's
   check *local and decidable*. loft allows many live mutable aliases of one owned store, so the
   free-safety proof is over an unbounded aliasing graph, not one borrow at a time.

The current position-proxy + flow-insensitive-join are precisely the **dodge**: one conservative
pass decides-once/rewrites-once, so it never re-analyses after its own rewrite. @PLN94's difficulty
*is* re-introducing precision, which brings the joint fixpoint back. **This is why the oracle form
is materially easier:** an oracle *observes* already-emitted IR (frees/copies present) and checks
consistency — it does not rewrite, so facet 1 vanishes entirely. It stays *rustc-shaped* hard
(facets 2–3: the op-tail transfer functions + the shared-mutation aliasing invariant) but not
*joint-fixpoint* hard. That is the real reason P0–P3 is **H** and the full P0–P4 replacement is **VH**.

## Why at all — and the honest trade

loft's ownership half has taken **rustc's hardest job** (decide every free without UB) while
removing rustc's safety valve — **it never rejects**. So **completeness is load-bearing for
soundness**: an incomplete fact is not a user-fixable compile error, it is a miscompile / leak
(`formal/ownership.md:64-66`). The A1b UAF (@PLN90) — which *shipped latently in the native
backend* — is the existence proof that a heuristic substrate can harbour that class; rustc
structurally cannot emit it because its dataflow is precise at the join. This plan buys that
structural immunity.

The trade is real and this plan is **status:future** for a reason: it re-engineers a subsystem
that already works and is oracle-green. The **trigger** to unpause is one of — (a) the
ownership half's A1b-class incompleteness risk is judged to exceed the migration cost; (b) a
feature needs precision the position-proxy cannot give (e.g. loop-carried borrows, partial
moves); or (c) the perf fork below resolves toward "the fixpoint is the shipped analysis." Until
then it is the *validation oracle's* structural backstop, not a rewrite in flight.

## Beside the current analysis — oracle first (the recommended entry point)

Introduce the fixpoint **beside** the current analyses, not as a replacement — and make its
*first* role a permanent, independent **completeness oracle**, with cutover an optional later
decision. The reason is sharp: the existing differential oracle (@PLN89) compares **interp vs
native**, but both backends read the *same* ownership fact — so a fact that is wrong *but
consistently* wrong passes (this is exactly how A1b shipped latently in native). A flow-sensitive
fixpoint run beside the shipped analysis is a **second, independent source of truth about the
fact itself** — the missing "is the fact right?" check, not "do the two backends agree on it?".

This ordering de-risks the whole plan and delivers value before any cutover:

- **Shipped codegen is untouched** — the oracle only observes and alarms on divergence, so there
  is zero regression risk to the fast path.
- **Value from a PARTIAL fixpoint** — even liveness-only, or just the `Join` arm-meet, run as an
  oracle immediately starts flagging A1b-class divergences; it need not be complete enough to
  *ship*, only precise enough to *disagree correctly*.
- **Cutover / retirement becomes optional and evidence-based** — after the fixpoint has run beside
  for a while (green ⇒ the shipped fact is complete; red ⇒ it caught a latent hole), *then* decide
  whether it earns a place in the fast path (the perf fork, Open question 4). The old analysis is
  retired only if and when that decision lands.

So the coexistence is not just migration scaffolding — permanent coexistence (cheap shipped
analysis + fixpoint oracle) is a legitimate *end state* that fits loft's "validation, not proof"
stance and strengthens the very safety net the rustc evaluation identified as load-bearing.

**The principle, generalised: the most important evaluations deserve MORE THAN ONE independent
implementation.** A single implementation run two ways (interp vs native reading the *same*
ownership fact) cannot catch a wrong-but-consistent fact — that is exactly how A1b shipped latently
(Step 0.2). A *second, independent* implementation of the evaluation can, because two independent
computations of the same fact MUST agree, and every disagreement is a real finding in one of them.
Early, the divergences point at the *newer/less-complete* implementation (a friendly first target);
as it matures, a residual divergence indicts the *shipped* one — which is the A1b catch. This is not
specific to ownership: any evaluation load-bearing for correctness (free-placement, type-resolution,
layout) earns an independent cross-check for the same reason. **Demonstrated already:** Phase 3.3's
independent call handling surfaced 22 concrete divergences vs the shipped classifier; adjudicating
them (3.4a) proved **all 22 were a single unsoundness in the NEW implementation** — the friendly
first-target case, exactly as the principle predicts — a projection local mis-routed to `Owned`.
The cross-check produced the work-list AND, on adjudication, pinned it to one root cause the newer
impl owned; guessing would not have found it. (A residual divergence indicting the *shipped* impl —
the A1b catch — is the later, matured-oracle case; not yet reached.)

## Design decisions (recommendations; the forks are Open questions below)

- **Structured dataflow over the Value-IR — NOT a MIR basic-block lowering.** loft's control
  flow is structured (if / match / `Loop` / break / return — no gotos; `for` desugars to
  `Loop`). A syntax-directed dataflow with an explicit **fixpoint at `Loop` nodes** and a
  **meet at if/match arm-joins** is sufficient and far cheaper than lowering to basic blocks.
  break/return propagate an abnormal-exit state to the enclosing loop-exit / function-exit
  (standard structured abstract interpretation). Explicit CFG stays the fallback iff
  irreducible control flow ever appears (it should not).
- **The lattice.** Per-variable state over `⊥(unreachable) ⊑ {Dead, Owned, Borrowed(base-set),
  Moved} ⊑ ⊤`, meet at joins. `Join` (owner depends on which arm ran) stops being the
  all-defs join and becomes a genuine **per-path lattice value** at the arm-meet; its runtime
  witness (`OpBindOrCopy`) is emitted only where the meet is genuinely `Owned ⊔ Borrowed`.
- **Liveness is the core; compute it first.** Backward liveness feeds *both* free-placement
  (dead ⇒ free) and move-elision (last-use). It is the exact-construction kernel — get it
  right and validated before the forward ownership pass rides on it.
- **Interprocedural stays a per-fn summary.** Keep the existing memoised `return_ownership`
  summary as the callee contract; the intra-procedural fixpoint consumes callee summaries at
  call sites (like rustc's per-fn signatures). No whole-program dataflow.
- **Migration is shadow-mode, one consumer per commit — never big-bang.** The old analyses
  keep shipping until each consumer is proven against the new fact via the @PLN89 oracle. This
  is the whole safety story (see Phase ordering).

## Composition matrix — Stage A

The "cells" are the dataflow's **correctness surface**, not a new language feature. Grid,
each a `/tmp` probe first, the differential oracle (old-vs-new fact + runtime value/leak/poison)
is pass/fail, graduate to `tests/scripts/`:

`{liveness, ownership, Join}` × `{straight-line, if/match-join, single loop, nested loop,
break-out-of-loop, early-return-in-loop, loop-carried borrow, the A1b temp-subject-borrow-return
shape, the v[i]??d Join shape, mutate-through-shared-alias (@PLN93)}` × `{interpret, native}`.

Done = every cell green on both backends **and** the new fact strictly subsumes the old
(same where the old was right, correct where the old was incomplete — the A1b class).

## The re-assertion sites it must collapse (design-protocol § count the sites)

The plan's success = all of these read the **one** dataflow solution (O-Deps realised, not
aspired). Phase 0 must produce the *complete* inventory; the ones already known:

- **Free placement** — `scopes::get_free_vars` (`src/scopes.rs:3157-3420`, the
  `owns`/`in_ret`/`skip_free`/`captured_ref` gate).
- **Own-vs-borrow materialisation** — `state/codegen.rs:1753`, `:3700`; `generation/dispatch.rs:53`,
  `:213`, `:466`; `generation/mod.rs:1099` (all via `ownership_of`, gated `join_own_enabled`).
- **Move-elision / borrow-inline** — `scopes::move_elide` (`:454`) reading `MovePlan`;
  `scopes::elide_borrows` (`:307`) reading `ElidePlan`.
- **Displaced-owned strip** — `scopes::run_scan_phase` via `displaced_owned_slots`.

That is N ≫ 1 sites re-reading facts that two producers approximate; the plan is exactly the
"one chokepoint N sites route through" made real.

## Sub-arcs

Concrete, executable per-step gates for the oracle end-state (P0–P3) live in
[`IMPL.md`](IMPL.md) — Step 0 (falsify the oracle's reason to exist) through Step 5 (land the
oracle beside), each green on both backends with shipped output byte-identical.

| Item | Concern | Status |
|---|---|---|
| **P0** — Characterise + equivalence harness + `DATAFLOW.md` (lattice/transfer on paper) + tractability falsification | design gate | Open |
| **P1** — Structured-CFG walker + **backward liveness**, shadow-diffed vs the position-proxy | code | Open (dep: P0) |
| **P2** — **Forward ownership** dataflow (`Owned/Borrowed/Moved` + per-path `Join`), consuming callee summaries | code | Open (dep: P1) |
| **P3** — Run the fixpoint **beside** as an independent completeness oracle (default-on shadow, alarm on divergence); triage every old-vs-new disagreement | validation | Open (dep: P2) |
| **P4** — *Optional, evidence-gated:* cut over consumers one-per-commit (free → own → elision), retire the proxy/insensitive-join, reconcile `formal/ownership.md` | code | Open (dep: P3 + perf fork) |

## Phase ordering

1. **P0 — the falsification gate.** Complete the re-assertion-site inventory; write `DATAFLOW.md`
   (lattice, transfer functions per IR op, join/loop rules); build the differential harness
   (compute old fact + stub new fact, diff at every site on the corpus + `program_ownership`
   fuzzer, both backends); hand-work the fixpoint on the A1b, `v[i]??d`-Join, nested-loop, and
   early-return-in-loop shapes to prove monotonicity + termination. **Gate:** if a consumer needs
   info the lattice can't carry, or the fixpoint isn't monotone on loft's real shapes, STOP and
   redesign before any migration code.
2. **P1 — liveness first.** Cheapest exact-construction kernel; shadow it against the current
   last-use, feed nothing yet.
3. **P2 — ownership forward pass** on the same CFG.
4. **P3 — coexist as the independent oracle (a valuable end state on its own).** Run the fixpoint
   beside the shipped analysis (default-on shadow), diffing at every decision site on the corpus +
   `program_ownership` fuzzer, both backends. Triage every divergence: new-more-precise ⇒ an
   A1b-class latent hole in the *shipped* fact (win — fix + graduate a regression); new-wrong ⇒ fix
   the transfer function. Shipped codegen is untouched, so this phase can be the stopping point.
5. **P4 — optional, evidence-gated cutover + formalise.** ONLY once the oracle has run
   green-or-caught for a while AND the perf fork (Open q4) resolves toward "the fixpoint can ship":
   route each consumer to the new fact one commit at a time (byte-identical-or-strictly-better,
   poison+leak both backends), retire the proxy/insensitive-join, and move `formal/ownership.md`
   from "validated" to "structural". Otherwise the fixpoint stays a permanent oracle and this phase
   is not taken. Keep the `LOFT_POISON`/`LOFT_UAF` runtime backstops either way.

## Open design questions

1. **Structured dataflow vs explicit basic-block CFG.** Recommend structured; P0 must audit
   `break`/labeled-break/`Loop` desugaring to confirm no irreducible flow.
2. **Per-variable vs per-path/per-field granularity.** Recommend per-variable first (sound,
   matches today). Per-path (partial moves — move `x.a` while `x.b` lives) is a *precision*
   follow-on, not a soundness need under loft's copy-default; gate on demonstrated demand.
3. **Unify the elision half onto the same CFG?** Recommend yes — compute liveness once and feed
   both, retiring the position-proxy entirely (removes a whole fragile substrate; the elision
   half gains loop-precision for free).
4. **Perf: is the fixpoint the *shipped* analysis, or the *oracle*?** A worklist fixpoint costs
   more per compile than today's single pass ("pays no `Position` clone, stays byte-identical",
   `use_analysis.rs:206-208`). Fork: (a) ship the fixpoint; or (b) ship a cheaper conservative
   pass and run the fixpoint as the **machine-checkable completeness oracle** in CI/fuzz/verify
   — which fits loft's "validation, not proof" stance and could make the fixpoint valuable even
   if it never enters the fast path. Measure in shadow mode before deciding.

## Cross-arc dependencies

- **@PLN85** (store-lifetime retirement) — defines the `Own` fact this plan makes flow-sensitive.
- **@PLN89** (differential oracle) — the equivalence gate that makes shadow-mode migration safe;
  hard dependency for P0/P3.
- **@PLN90** (copy-diagnostics / move-elision) — a primary *consumer*; its `MovePlan`/`ElidePlan`
  become dataflow queries.
- **@PLN93** (collection capture) — supplies the shared-mutation cells the ownership lattice must
  model (many non-owning aliases of one owned store).

## See also

- Implements the beacon in [`../../OWNERSHIP_MODEL.md`](../../OWNERSHIP_MODEL.md) ("a sound,
  complete, statically-computed ownership system … from which every codegen decision derives
  mechanically") and would move [`../../formal/ownership.md`](../../formal/ownership.md) from
  *validated-complete* to *structurally-complete*.
- Current substrate: [`../../../src/use_analysis.rs`](../../../../src/use_analysis.rs) (analyses A + B),
  [`LIFETIME.md`](../../LIFETIME.md), [`DEPS_INVENTORY.md`](../../DEPS_INVENTORY.md).
- Tracked as [`@PLN94`](https://github.com/loft-lang/plans/issues/94); promoted from the
  "usage-analysis vs rustc" evaluation.
