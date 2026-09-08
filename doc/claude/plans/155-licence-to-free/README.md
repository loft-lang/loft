<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 155 — The licence to free: one home, fail-closed, and the refusal where every free ends up

Tracker: [@PLN155](https://github.com/loft-lang/plans/issues/155).

## Status

**Queued (`status:next`) — nothing built.  Arc A and phase 0 are independent of every
other phase and are what decide whether the rest is worth doing.**  Sequenced behind
[@PLN153](../153-null-rules-one-home/README.md): both campaigns rewrite `scopes.rs`, and the
bug review's payoff check can attribute a fall only when one campaign moves at a time.  The
ownership MODEL is not reopened here — [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) stands,
and so does `deps` as the carried fact.  What this plan changes is **who is allowed to free**:
today that is derived four different ways, concluded in one function and emitted in another,
and the derivation that decides it fails toward *permitted*.

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
- **Last touched:** 2026-09-07.

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
| **A** — the reassessment instrument: the four campaign gates as a command | § Arc A | reproduces the 2026-09-07 ranking from measurements alone; fed the pre-2026-08 bands it must NOT name generic/monomorph (whose keystone paid off) | Open |
| **0** — probe: how many frees are licensed by the PROXY alone? | § Phase 0 | its own control — an injected proxy-only free (`LOFT_OWN_INJECT_FACT_OWNED` precedent) moves the count; a category reading 0 is shown reachable before it is believed | Open |
| **1** — one home for *may this binding's store be freed here* | `Scopes::owns_freeable_store` | `scripts/introspect_diff.sh` byte-identical over the corpus; `o_proxy_check.py`'s `N of M reach a free` control does not collapse | Open |
| **2** — fail closed: `Own::Unknown` | `use_analysis.rs:2355` (the code names the cure) | an unnamed IR spelling DECLINES instead of freeing (`make falsify` vs the pre-loft#1248 build); corpus diff differs only in the cells phase 0 predicted, written down first | Open |
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
   `Tuple` are the formers with no number today, and without one, condition 3 is an opinion;
4. **a walk already tried and measured no effect** — from the QUALITY.md walk records and the
   review's payoff column.

A class passing all four earns a plan; passing only 1–3 earns a rule-led walk; passing only 2
earns a queue entry.  **The gate that must stay loud is 4**: a class whose last keystone has an
unfilled payoff row is not a candidate, it is an unfinished measurement — which is exactly
where keyed collections sits (15.5 % and RISING, `Scopes::owns_freeable_store` landed 2026-09,
row blank).  A report that ranked it first would be reading three gates out of four.

### Phase 0 — the probe that can kill the plan

An env-gated census over the corpus classifying every emitted free by the fact that licensed
it: oracle-agreeing · proxy + veto · **proxy alone** · `owned_refs` only.  If proxy-alone is a
handful, phases 2–4 are not worth their cost and the plan closes with the number as its
product; the census stays as the guard that says when that changes.

### Phase 3 — the ladder, and what `deny` may do

`report` first (default, one release), then `deny`.  The terminal behaviour is a design call,
not an implementation detail — see question 3.

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
- `make bug-review`, `scripts/rule_predicate_audit.py`, `scripts/ir_walker_audit.py`,
  `scripts/o_proxy_check.py`, `scripts/introspect_diff.sh`, `make falsify`,
  `LOFT_OWN_ORACLE=check` — the instruments.
- [@PLN155](https://github.com/loft-lang/plans/issues/155) — this plan's issue.
