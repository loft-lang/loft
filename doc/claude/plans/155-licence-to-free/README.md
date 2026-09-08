<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 155 — The licence to free: one home, fail-closed, and the refusal where every free ends up

Tracker: [@PLN155](https://github.com/loft-lang/plans/issues/155).

## Status

**Active — arc A and phase 0 landed 2026-09-08.  Arc A does NOT confirm the plan's premise;
phase 0 confirms it on a different measurement and the plan continues.**  @PLN153 closed the
same day, so the sequencing hold is lifted.  **Next: phase 3** — the refusal at the
free, on a `report → deny` ladder.  Phase 2b's refutation is what points there: the fail-open
cannot be closed at the VERDICT, because at three of four readers the permissive answer is
load-bearing or unmeasurable.  The ownership MODEL is not reopened
here — [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) stands, and so does `deps` as the
carried fact.  What this plan changes is **who is allowed to free**: today that is derived
four different ways, concluded in one function and emitted in another, and the derivation
that decides it fails toward *permitted*.

**What arc A measured, and what it means for the rest of this plan.**  `make campaign-review`
on the 2026-09-08 population (642 bugs, #246-#1467) ranks ownership/free as a **walk, not a
campaign** — it fails two of the four gates:

| gate | ownership/free reads | the plan's premise said |
|---|---|---|
| 1 trend | **falling -3.0pp vs peak 19.3 %** (16.3 % in the last band; -3.1pp with @PLN153's own finds screened out) | "flat at its own peak", measured 2026-09-07 with 5 bands |
| 2 spellings | **`Enum+Reference+Vector` hand-spelled at 26 sites**, one `@FR-O` rule cited from 12 files — the widest of any class | one fact in four representations ✓ |
| 3 chokepoint | **`Type::RefVar` 78 % opaque** (584 of 751 shape-resolving functions cannot see a `&τ`) — the number the plan said did not exist | "no chokepoint for the LICENCE" ✓ |
| 4 walk tried | **unjudged — the fall is 2.8 bugs in a 109-bug window** | "the walk was tried and measured no effect" |

Gates 2 and 3 hold and are the strongest readings in the table.  Gates 1 and 4 do not, and
both for the same reason: **the seven ownership walks landed 2026-09-04/05 and their payoff
window has not passed.**  Four days is not a measurement — a 2.8-bug difference in a
109-bug window separates nothing — so "the walk measured no effect" is not yet a fact, it is
an unfinished measurement of exactly the kind gate 4 exists to refuse.  The report says so
rather than ranking the class.

**Phase 0 answers what the gates could not, and it does NOT kill the plan.**  `make
licence-census` over the whole 1412-file corpus: **6412 of 80 325 emitted frees (8.0 %) rest
on the deps PROXY alone**, at **2478 distinct `function:binding` sites, 804 of them
author-named bindings rather than compiler temps.**  The kill criterion was *a handful*; this
is not one.  Phases 1-4 are worth their cost, and they are worth it on a measurement of the
COMPILER rather than on a bug share that a three-day window cannot read.

| the fact that licensed the free | frees | share |
|---|---|---|
| `oracle-derived` — the oracle read the binding's own definitions and answered `Owned` | 45 192 | 56.3 % |
| `minted` — `OpDatabase` minted a store into it; the strongest positive fact | 17 555 | 21.9 % |
| `oracle-disagrees` — the oracle answers `Borrowed`/`Join` and a free was emitted anyway | 9 349 | 11.6 % |
| **`proxy-alone` — empty deps, and the oracle had NOTHING to read** | **6 412** | **8.0 %** |
| `veto` — a `skip_free` binding, whose contract is no ownership-derived free at all | 1 817 | 2.3 % |

Phase 0's own finding about the population, which shapes phase 2: the largest family in
`proxy-alone` is `__ref_N` (1579 of 2478 sites) — the NRVO/return buffers, which the oracle's
`Value::Var` arm *deliberately* cannot classify (*"a store the function minted in place with
no `Set` at all … else a parameter"*).  So the fail-open is not an accident at the margins; it
is load-bearing for the return-buffer machinery, and phase 2's `Own::Unknown` has to give
those a positive answer rather than merely refuse them.

⚠ **What the number is NOT.**  It is not 6412 wrong frees.  It counts what each licence RESTS
on, not what is broken: a `__ref_N` buffer really is owned, and the oracle simply cannot see
it.  The claim phase 0 supports is the plan's premise — that the licence is derived from a
proxy with no positively-derived owner fact behind it at one free in twelve — and nothing
stronger.

## Goal

Every emitted free derives its licence from one positively-derived owner fact, the shapes that
fact cannot name are denied rather than granted, and a free without a licence is refused at the
point every free ends up (`OpSets::frees`, the five spellings).

## Effort + design

- **Effort:** H — seven arcs, none straddling a PR.
- **Design:** ~ — the four facts are named (`formal/IMPLEMENTATIONS.md` § The variable-lifetime
  map); the licence's ONE home, the deny ladder's terminal behaviour and the heap half are the
  design calls (§ Open design questions).
- **Value category:** S (silent failure).  An over-free reads another record's bytes and a leak
  reaches the store ceiling; both answer without saying anything.
- **Last touched:** 2026-09-08 (arc A, phases 0, 1, 2a, 2b landed).

## Why this family, measured

`make bug-review` (2026-09-07, 586 bugs in four bands): ownership/free is **37 of the last 200
(18.5 %), flat at its own peak** and the second-largest class after null/sentinel — while
`@FR-O` is the **most-cited rule family in the repo** (13 of 14 rules, 225 sites) and five
walks landed on it in one day (QUALITY.md B7q–B7v).  Cited everywhere, still firing: the
`@FR-L-Null` signature the review scored NO EFFECT.  The full argument, with the four
representations of the one fact and the three sites where the licence fails open, is on
[@PLN155](https://github.com/loft-lang/plans/issues/155); it is not restated here.

## Composition matrix — Stage A

The axes a licence cell varies.  Written as `/tmp` probes on `--interpret` first, hand-computed
per cell, graduated to `tests/scripts/155-*.loft`; `python3 scripts/matrix_axes.py file <guard>`
is run on every guard and the axes it reports unreached are cells still to build.

| axis | domain |
|---|---|
| binding kind | local · parameter · struct field · vector element · tuple member · capture · return slot · keyed member |
| ownership route | mint (literal, `OpDatabase`) · copy · projection (`v[i]`, `s.f`, variant payload) · call return (owned / borrowed / join) · join arm · owner witness (mixed) · `CallRef` (closure) |
| free spelling | `OpFreeRef` · `OpFreeRefTag` · `OpFreeRefIfDistinct` · `OpFreeRefOrHandUp` · `OpFreeText` · the scope-exit sweep · the pre-`Set` displaced free |
| licensing fact | `deps` proxy · `is_skip_free` veto · `owned_refs` (latest assignment + loop depth) · oracle verdict |
| nullability | `τ` · `τ?` — a nullable view local is the heap local it is (@PLN153 phase 4) |
| loop depth | 0 · inside a loop · nested |
| backend | `--interpret` · `--native` |

## Sub-arcs

`Verify` names the comparison that would go RED if the arc were done wrong.

| Item | Source | Verify | Status |
|---|---|---|---|
| **A** — the reassessment instrument: the four campaign gates as a command | § Arc A | reproduces the 2026-09-07 ranking from measurements alone; fed the pre-2026-08 bands it must NOT name generic/monomorph (whose keystone paid off) | ✅ **done** — `scripts/campaign_review.py`, `make campaign-review`.  Control: `--control` PASSES (no class is named a PLAN on the pre-2026-08 population), and it FAILS when a gate is mis-wired — proved by making gates 3 and 4 pass on an unmeasured reading, which named generic/monomorph and tuple |
| **0** — probe: how many frees are licensed by the PROXY alone? | § Phase 0 | its own control — an injected proxy-only free (`LOFT_OWN_INJECT_FACT_OWNED` precedent) moves the count; a category reading 0 is shown reachable before it is believed | ✅ **done — 8.0 %, the plan is NOT killed.**  `LOFT_OWN_ORACLE=census` + `make licence-census`.  Both controls PASS, in the two directions a bucket can move; no category reads 0, so nothing had to be shown reachable |
| **1** — one home for *may this binding's store be freed here* | `Scopes::owns_freeable_store` | `scripts/introspect_diff.sh` byte-identical over the corpus; `o_proxy_check.py`'s `N of M reach a free` control does not collapse | ✅ **done — the narrow-widths outcome: one PAIR, four questions.**  `Function::proxy_says_owned` folds six sites; three are shown to ask a different question and stay apart.  `introspect_diff.sh` **IDENTICAL 1412/1412**; the control went 9 of 29 → 9 of 30 after the check was taught the folded spelling — it read 3 of 23 first, which is the collapse the verify exists to catch |
| **2a** — the verdict exists and every reader disposes of it | `use_analysis.rs:2355` (the code names the cure) | an unnamed IR spelling DECLINES instead of freeing (`make falsify` vs the pre-loft#1248 build); corpus diff differs only in the cells phase 0 predicted, written down first | ✅ **done** — `Own::Unknown`, 20 readers disposed.  Prediction (EMPTY diff) written first and **falsified twice**, each falsification naming a reader that silently inherited the permissive answer; **IDENTICAL 1412/1412** once both were fixed |
| **2b** — a reader DECLINES on `Unknown` | § Phase 2b | `make falsify` vs the pre-loft#1248 build; the leak each decline trades for, measured per reader | ✅ **done — all four candidates REFUTED, and none lands.**  `LOFT_OWN_DECLINE=<name>` is the instrument that says so: `witness` is a WRONG VALUE on both backends, `owned-slot` moves three files' emit with every channel unchanged (unverified, not safe), `collection` and `join` are inert |
| **3** — the refusal at the free, on a `report → deny` ladder | § Phase 3 | full gate + corpus green under `deny`; hand-computed position × free-spelling × backend matrix; guard falsified against a pre-phase build | Open |
| **4** — the heap half (`@FR-H-Free`, `-FreeTwice`, `-FreeLIFO`, `-FreeNull`) | `formal/heap.md` | double-free, free-null and LIFO-order cells red before the arc, green after, both backends | Open |
| **5** — re-measure | `make bug-review` | the ownership/free row after this plan's watermark, plus the keyed-collection keystone's own row | Open |

### Arc A — the reassessment instrument

The question *"which family earns a campaign next?"* is asked repeatedly and answered today by
reading four instruments by hand.  Arc A makes it a report joining them, one row per mechanism
class, one column per gate:

1. **rising or peak-flat** in `scripts/bug-review.py`'s class table;
2. **one fact in ≥2 representations** with no predicate answering both — from
   `rule_predicate_audit.py` (shape-shared lists) and `rule_tags.py dups` (rule-shared sites);
3. **no chokepoint by construction** — `ir_walker_audit.py`'s opacity screen, which today
   answers only for `Optional` (728 functions discriminate on a `Type` variant, 359 opaque).
   Parameterising it by type former is the arc's one code change: `RefVar` (234 sites) and
   `Tuple` are the formers with no number today, and without one, condition 3 is an opinion.
   **Landed** — `ir_walker_audit.py former <Name>`, over the 751 functions that resolve a
   shape by naming a `Type` variant:

   | former | sees (peel or arm) | descends | opaque | opaque share |
   |---|---|---|---|---|
   | `Optional` (`τ?`) | 407 | 6 | 338 | 45 % |
   | `RefVar` (`&τ`) | 156 | 11 | **584** | **78 %** |
   | `Tuple` (`(τ, …)`) | 135 | 12 | 604 | 80 % |
   | `Rewritten` | 37 | 15 | 699 | 93 % |

   `Optional` is the one former with a chokepoint, and it is the one a campaign has already
   been run on — which is the reading that makes the screen worth generalising.  The peel
   verbs are per former and deliberately not shared: `base()` strips an `Optional` and leaves
   a `&` exactly where it was.  Two of the other three carry the same finding in their own
   doc — `Type::unrewritten` names loft#943, `Type::peel_link` names loft#753;
4. **a walk already tried and measured no effect** — from the QUALITY.md walk records and the
   review's payoff column.

A class passing all four earns a plan; passing only 1–3 earns a rule-led walk; passing only 2
earns a queue entry.  **The gate that must stay loud is 4**: a class whose last keystone has an
unfilled payoff row is not a candidate, it is an unfinished measurement — which is exactly
where keyed collections sits (15.5 % and RISING, `Scopes::owns_freeable_store` landed 2026-09,
row blank).  A report that ranked it first would be reading three gates out of four.

### Phase 0 — the probe that can kill the plan — RUN, and it does not

An env-gated census over the corpus classifying every emitted free by the fact that licensed
it: oracle-agreeing · proxy + veto · **proxy alone** · `owned_refs` only.  If proxy-alone is a
handful, phases 2–4 are not worth their cost and the plan closes with the number as its
product; the census stays as the guard that says when that changes.

**Built** as `LOFT_OWN_ORACLE=census` (`ownership_cfg::run_licence_census`), summed over the
corpus by `scripts/licence_census.py` / `make licence-census`.  The verdict is in § Status:
8.0 % of emitted frees, 2478 sites.  Two things about the instrument are worth carrying
forward.

**The oracle had to publish its EVIDENCE, not only its verdict.**  `ownership_of`'s
`Value::Var` arm answers `Own::Owned` for a var with no definition and no mint — a FALLBACK,
not a derivation, and indistinguishable from a real `Owned` in the return value.  That
distinction IS the census, so `use_analysis::ownership_evidence` publishes it
(`Derived` · `Minted` · `Parameter` · `Fallback`) rather than the census re-deriving *"does
this var have a definition"* at its own site — which would have been a second spelling of the
question `ownership_of` already owns, the exact defect this plan exists to remove.

**The decisive bucket moves in only ONE direction, and the reason is structural.**  The
existing over-free injector (`LOFT_OWN_INJECT_FREE_BORROWED=bview`) moves `oracle-disagrees`
5 → 6, so the census is not vacuous — but it cannot move `proxy-alone`, because such a binding
is by definition one the proxy already licenses, so its free is EMITTED rather than suppressed
and there is nothing for an injector of suppressed frees to force.  The control that reaches
it is the drop direction: `LOFT_OWN_INJECT_DROP_FREE=__ref_1` takes `proxy-alone` 1 → 0.
`make licence-census ARGS=--control` runs both and requires both to move.

### Phase 3 — the ladder, and what `deny` may do

`report` first (default, one release), then `deny`.  The terminal behaviour is a design call,
not an implementation detail — see question 3.

## Phase 1 — one home for the PAIR, and the three questions that are not it (2026-09-08)

The plan offered two outcomes — *"either it grows to every free spelling or the spellings are
shown to ask different questions (the narrow-widths outcome: one type list, three questions)"*.
The answer is both, and the split is where the value is.

**The census.**  `o_proxy_check.py -v` names nine sites that conclude ownership on the proxy
AND reach a free.  Read side by side, six spell the identical conjunction —
`tp(v).depend().is_empty() && !is_skip_free(v)` — the two obligations @FR-O-Proxy says must
never travel apart:

| site | what it frees |
|---|---|
| `parser/control.rs` arm-return | the match arm's own backing store |
| `scopes.rs` ×3 (`scan_set`) | the ownership-TRANSITION free, plain · `??`-hoist · witness-guarded |
| `scopes.rs` drop hook | the `__disp_N` drop-cascade lift |
| `state/codegen.rs` move-elision | `src`'s store, moved into `v` and released by `v`'s sweep |

They are now one predicate, `Function::proxy_says_owned` — on `Function` rather than on
`Scopes`, because that is what every one of the six has: the parser holds `self.vars`, the
interpreter's codegen holds `stack.function`, and `Scopes` is handed one.
`Scopes::owns_freeable_store` becomes that pair plus the obligation that IS its own — the
sweep frees a frame's bindings, so a user parameter belongs to the caller.

**The three that stay apart, and why.**  Each is marked at its own site, because a reader who
lands there is the person who needs to know:

- `tuple_owned_elem_frees` reads the proxy off a tuple **ELEMENT's** type and the veto off the
  **CONTAINER** binding.  Two subjects, where a predicate over one `v` has one.  Folding it
  would have to invent a dep list for the element, which is exactly what a tuple does not have.
- The two dep-**STRIPPING** sites in `scan_set` read `!depend().is_empty() && !is_skip_free(v)`.
  That is not the pair negated (`!(empty && !veto)` is `!empty || veto`) — it is a different
  conjunction, and its first half is not an ownership answer at all: it asks *"is there a dep
  list left to strip"*, a question about WORK TO DO.  Only the veto is this rule's obligation
  there, which is why it is read on its own.

**A second spelling of the veto, deleted.**  `Function` exposed the never-free flag as both
`is_skip_free` and a bare `skip_free`, with five callers on the second — and @FR-O-Override's
contract is written in a doc block on the first only, so a reader of the second never met it.
One reader now.

**What the control caught, which is the reason it is in the verify.**  Folding six textual
proxy reads into one predicate took `o_proxy_check.py` from **9 of 29 reach a free** to **3 of
23**.  That reads like an improvement and is a BLINDING: the check's job is *a site that frees
on the proxy consults the veto*, and a site that has discharged the obligation by construction
is still such a site — one the check had simply stopped seeing.  Teaching it that
`proxy_says_owned(v)` IS a proxy read (and discharges the veto) restores the population to
**9 of 30**, one more than before, which is the predicate itself.  A fold that shrinks its own
instrument's population has not been verified; it has removed the verifier.

## Phase 2a — the verdict exists, and the prediction that failed twice found what it was for (2026-09-08)

`Own` had three verdicts and a permissive default: `classify`'s `CallRef` arm answered `Owned`
— the one verdict that licenses a free — both for a fn-ref whose target it cannot resolve and
for a callee returning a borrow whose base the caller cannot NAME.  The arm's own comment names
the cure: *"`Own::Unknown` — forcing each caller to decide rather than defaulting to the
permissive value — is what would make that attempt safe."*

**2a is the variant and the disposals, and nothing else.**  Every reader disposes of `Unknown`
explicitly, and every one disposes of it exactly as it disposed of `Owned` — so the emitted
program is unchanged.  That is the point rather than a limitation: phase 2's product is that a
reader must now SAY what it does with an underivable verdict, and that a new reader cannot
inherit the permissive answer by writing `_ =>`.  What a reader SHOULD do is a separate
decision each, and folding those in would spend the byte-identical gate exactly where it is
most needed.

**The prediction was written first, and it failed twice.**  Both failures are the whole value
of the phase, because each named a reader that had been silently inheriting the fail-open:

1. **`use_analysis::text_return_risk`** — `Own::Owned => Some("owned-by-value")` with a
   `_ => None` beneath it.  `Unknown` fell to `None`, meaning *no risky delivery site*, and a
   `text` return lost its caller buffer for a frame-local `__ret_N` — a promotion decision
   flipping on a verdict nobody chose.  9 corpus files.
2. **`use_analysis::callref_collection_join_base`** — and its own comment is the finding:
   *"`Own::Owned` is also this arm's FALLBACK for a base the summary could not name."*  That
   sentence describes `Unknown`; letting it fall to `_ => None` dropped the witness and with it
   an `OpFreeRefIfDistinct`, one leak per evaluation.  3 more files.

Neither is reachable by the compiler's exhaustiveness check — both are `_ =>` arms — which is
exactly why the corpus prediction had to exist.  **A behaviour-preserving refactor whose
prediction cannot fail is not verified.**  Both are now spelled `Own::Owned | Own::Unknown`,
and `introspect_diff.sh` reads **IDENTICAL 1412/1412**.

**Five more readers absorbed it silently and were caught by transcription**, not by the gate:
`matches!(own, Own::Owned)` in three `mints` tests, the displaced-slot filter's `s.prior`, and
the shadow lattice.  Same defect, found by reading rather than by measuring — which is why the
measurement is what the phase is graded on.

**The census sharpened, and phase 0's headline was 133 frees too optimistic.**  A `no-answer`
bucket, kept OUT of `oracle-disagrees` (whose documented meaning is *the oracle answers
Borrowed/Join*, which `Unknown` is not).  Predicted before the run: the total unchanged,
`oracle-disagrees` not growing, `minted`/`veto` untouched, and the new bucket carved out of
`proxy-alone` and/or `oracle-derived`.  All four hold, and the last is sharper than predicted —
**133 frees, carved entirely out of `oracle-derived`**: they had been counted as positively
derived owner facts while the oracle had no answer at all.

⚠ **`no-answer` reads 0 on the 40-file sample** the baseline was taken from, which is the
alphabetically-first corpus files and stdlib-dominated.  By this plan's own rule a category
reading 0 is a claim to check, and it was: the bucket fires on every fn-ref-heavy guard
(1335 → 4, 1323 → 4, 1245b → 4, 387 → 1).  The zero was the sample, not the category.

## Phase 2b — every candidate refuted, and the refutations disagree with each other (2026-09-08)

Phase 2a left four readers marked as candidates for DECLINING on `Unknown`.  `LOFT_OWN_DECLINE=<name>`
flips one at a time, opt-in, default empty, and the A/B is one binary against itself over the
1412-file corpus.  **None of the four lands, and the three reasons are different — which is the
result, because a single reason would have been a property of `Unknown` rather than of the
readers.**

| candidate | emit | measured verdict |
|---|---|---|
| `witness` — `scopes`' `__ret` witness | 4 files | **WRONG VALUE, both backends.**  Guard 1335 reads `rr.x == 99` where the mapper must give 81; guard 1323 also fails on the interpreter |
| `owned-slot` — `scan_set`'s owned set | 3 files | every channel UNCHANGED — value, exit and leak, both backends, under `LOFT_STRICT_STORES`.  **Unverified, not safe** |
| `collection` — `callref_collection_join_base` | none | **inert** on this corpus |
| `join` — `Own::join` absorbing | none | **inert** on this corpus |

**The `witness` result overturns phase 2a's own note.**  That site's comment hand-compensates
for the fail-open by asking the callee, and 2a called it *"the first reader to separate"* —
the reasoning being that `Unknown` states outright what the arm was inferring.  It is wrong:
the hand-compensation is not a workaround for the fail-open, it is the mechanism that makes the
site correct, and declining removes it.  Only the measurement says so, and the note is left in
place beside the refutation because a plan that quietly deletes its wrong predictions cannot be
audited.

**`owned-slot` is the interesting non-result.**  It moves three files' emitted code while every
channel reads identically — which is not a pass.  It is the shape `introspect_diff.sh`'s own doc
warns about (*"a changed emission that happens to compute the same values is still a change
nobody asked for"*) and the shape that bit this branch earlier the same day: eight files' emit
moved, all green, and three were leaking under a channel nothing gated.  Landing it needs the
three files shown individually right.

**`collection` and `join` are inert, so they cannot be scored at all**, and by
`ownership-history.md`'s own doctrine — *"a guard that cannot fail proves nothing"* — an inert
change is not landable however plausible.  Both stay behind the switch as the A/B for shapes
this corpus does not reach; building those by hand is phase 3's matrix.

⚠ **The A/B needs a WRAPPER per candidate, not `introspect_diff.sh --env`.**  That flag applies
the environment to BOTH binaries — it exists for *same env, two builds* — so using it for *one
build, two envs* makes both sides decline and reports IDENTICAL for every candidate.  All four
read IDENTICAL that way, and the finding *"every candidate is inert"* was one step from being
recorded.  The cure is the same positive control as everywhere else in this plan: before
trusting an instrument, show the thing it measures actually moves it — one file, one command.

**What phase 2 therefore concludes.**  `Own::Unknown` earns its place as a verdict a reader
must dispose of (2a), and **no reader should decline on it today** (2b).  The fail-open is not
a single mistake that can be closed at the verdict; at three of these four sites the permissive
answer is either load-bearing or unmeasurable, and phase 3's refusal must therefore sit at the
FREE rather than at the verdict — which is where the plan already puts it.

## Phase ordering

A first: it is independent, it is what phase 5 re-runs, and it is the cheapest thing here.
0 before 1–4 (it can kill them).  1 before 2 (the home has to exist before the verdict that
feeds it changes).  2 before 3 (a refusal over a fail-open verdict refuses the wrong frees).
4 can interleave with 3.  5 last, and only after a bug-review window has passed.

## Open design questions

1. **Is the licence ONE predicate over all five free spellings?**  The witness-guarded pair
   (`OpFreeRefIfDistinct`, `OpFreeRefOrHandUp`) is a no-op where the placeholder aliases its
   witness, so a path walk reads one as CLEARING a pending free.  The narrow-widths precedent
   says assume separate questions until each site is read, and record the split rather than
   merging it.
2. **Where does the refusal live?**  `@FR-O-NoDiverge` says a decision is spelled once —
   `scopes` decides and writes `OpFreeRef`, the emitters translate.  But the displaced free is
   stashed per backend (`state/codegen.rs`'s `OpFreeRefIfDistinct`, `generation/dispatch.rs`'s
   `_old`/`_disp`), so either the refusal sits before both or it is two checks reading one
   predicate.
3. **What does `deny` DO?**  A hard error would refuse programs the compiler compiles today,
   which [COMPATIBILITY.md](../../COMPATIBILITY.md) governs.  The proposal is that `deny`
   **declines the free** — a reported leak rather than an abort — so the terminal failure is
   the safe direction, with an `abort` mode for the gate.  A leak and a UAF are both wrong; only
   one of them is recoverable, and the store ceiling already reports the leak.
4. **Does `Own::Unknown` carry a base?**  `Join`'s readers need the witness; if `Unknown` needs
   one too it is a fourth verdict, and if it does not, `callref_join_first_bind`'s three readers
   each need an explicit arm for it.
5. **Does arc A belong here?**  Kept, because phase 5 is the arc that reads it.  If it grows
   past a report it becomes its own plan.

## Cross-arc dependencies

- **@PLN153** (`status:active`) — sequencing: its phases 4/5 tails and its phase-6 re-measure
  come first.  Its phase-4 finding (a nullable view local IS the heap local it is) is the
  nullability row of this plan's matrix.
- **@PLN139** (finished) — the double-move lint counts hand-offs with the same predicate that
  suppresses the source's own drop; that predicate must not drift from the licence this plan
  gives one home.
- **@PLN102** (finished) — arc C / COMPATIBILITY.md owns question 3's one-directional flip.
- **@PLN152** (`status:next`) — independent; no shared surface.

## See also

- [formal/ownership.md](../../formal/ownership.md) — the `@FR-O` rules;
  [formal/heap.md](../../formal/heap.md) — the `@FR-H` rules phase 4 gives a home;
  [formal/IMPLEMENTATIONS.md](../../formal/IMPLEMENTATIONS.md) § The variable-lifetime map — the
  four facts, and the checker that reported clean over nothing.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the north star this plan does not reopen;
  [QUALITY.md](../../QUALITY.md) B7q–B7v — the five walks whose no-effect is this plan's premise.
- [BUG_REVIEW.md](../../BUG_REVIEW.md) — the ownership/free rows this plan is measured by;
  [STABILITY_METHOD.md](../../STABILITY_METHOD.md) § The rule-led walk — the cheaper tier, and
  when arc A should route a class there instead.
- `make campaign-review` (arc A's own report, `--control` for its negative control),
  `make licence-census` (phase 0's census, `--control` for its two injection controls),
  `make bug-review`, `scripts/rule_predicate_audit.py`, `scripts/ir_walker_audit.py`,
  `scripts/o_proxy_check.py`, `scripts/introspect_diff.sh`, `make falsify`,
  `LOFT_OWN_ORACLE=check` — the instruments.
- [@PLN155](https://github.com/loft-lang/plans/issues/155) — this plan's issue.
