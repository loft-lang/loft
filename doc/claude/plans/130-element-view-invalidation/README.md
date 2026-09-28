<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 130 — A copy nobody is told about, and a view that outlives what it names

**Status — CLOSED 2026-08-05. All findings resolved; no residual defect.** F1, F2, F4, F5, F6,
F8 fixed on both backends; F7 retracted (the doc was wrong, not the code); F3 reopened as
[loft#779](https://github.com/loft-lang/loft/issues/779) and closed as **F9** — disturbing a
container while a `&` reference into it is live is now a COMPILE-TIME ERROR
([`formal/binding.md` B-Ref-Reshape](../../formal/binding.md), design in
[F9-amp-link-survives-reshape.md](F9-amp-link-survives-reshape.md), boundary in
[probes/40-reshape-refusal/](probes/40-reshape-refusal/README.md)). Kept as the investigation
record; the contract itself lives in the reference docs below.

Tracker: [@PLN130](https://github.com/loft-lang/plans/issues/130) · opened from
[loft#774](https://github.com/loft-lang/loft/issues/774).

**Where the reference content lives now** — read these, not this file, for the contract:

| what shipped | its home |
|---|---|
| the view-invalidation contract (reshape / re-key / reassign → materialise + advice) | [OWNERSHIP_MODEL.md § A view lasts as long as the thing it names](../../OWNERSHIP_MODEL.md#a-view-lasts-as-long-as-the-thing-it-names--and-loft-says-when-it-does-not) · user-facing in [LOFT.md](../../LOFT.md) |
| the copy/view boundary it extends | [DESIGN_DECISIONS.md § C86](../../DESIGN_DECISIONS.md#c86--whole-value-heap-binds-copy-aliasing-is-a-last-use-elision-the-rustc-rule) |
| the `&` rule — a disturbance under a live reference is refused | [formal/binding.md § B-Ref-Reshape / B-Disturb](../../formal/binding.md) · the C79 principle behind it in [DESIGN_DECISIONS.md § C79](../../DESIGN_DECISIONS.md) |
| the copy-diagnostic model (the owner's five decisions, the three copy kinds) | [COPY_DIAGNOSTICS.md § The decided model](../../COPY_DIAGNOSTICS.md) |
| removal renumbers a vector, and a vector stays DENSE | [formal/collections.md § Col-Remove](../../formal/collections.md) |
| the METHOD (build the instrument before the fix) | [CODEGEN_METHOD.md](../../CODEGEN_METHOD.md) — the worked example stays below |
| `LOFT_STRICT_STORES`, `LOFT_DEBUG_F8`, `LOFT_COPY_MANIFEST` | [DEBUG.md](../../DEBUG.md) |
| the guards | see the probe→test mapping below |

**Probe → CI test mapping** (obligation 2 of `_INVESTIGATION_TEMPLATE.md § Closing`: a probe
encoding a correctness GUARANTEE must be CI-run, because `probes/` is not).  Characterization
probes — the watermark counts, the secret-copy catalogue, the reference↔problem pairings — stay
in `probes/` as landmarks:

| finding | guarantee probes | CI test |
|---|---|---|
| F1 — a view reassigned from a loop var destroyed the container | 30 | `tests/scripts/144-view-loopvar-reassign.loft` |
| F2 — a view live across a RESHAPE | 03–07 | `tests/scripts/145-view-materialised-on-reshape.loft` |
| F2 — and ONLY where it is live across one (F9 step 1) | 39 | `tests/scripts/148-view-liveness-across-reshape.loft` |
| F4 — a key-field write through a view | 28 | `tests/scripts/146-keyed-rekey-through-view.loft` |
| F3 — a `vector` stays DENSE | 01, 02 | `tests/scripts/200-vector-stays-dense.loft` |
| F7 — the C86 copy/view boundary, 30 cells | 09–14 | `tests/scripts/201-bind-copies-projection-views.loft` |
| F8 — a view live across a container REASSIGNMENT | 35, 36, 37 | `tests/scripts/774-view-outlives-reassigned-container.loft` |
| the producer × invalidator boundary — which binds VIEW vs COPY, what invalidates | 25, 27, 29, 31, 32, 33, 34 | `tests/scripts/147-view-producer-invalidator-boundary.loft` |
| F8 — the `par` / coroutine paths (measured at closure) | 37 cells E1/E2 | `tests/scripts/776-generator-heap-locals.loft` |
| F3/F9 — a reference whose container the CALLEE reshapes | 26, 38, **40** | `tests/parse_errors.rs::b_ref_reshape_*` (the refused shapes) + `tests/scripts/149-reference-survives-callee-reshape.loft` (the ones that must still compile) |

**Still-open findings, filed forward** (obligation 1: the in-plan no-file rule inverts at
closure).  Two remain, both legitimate resting states rather than unfixed bugs, and both live in
[COPY_DIAGNOSTICS.md § What remains open](../../COPY_DIAGNOSTICS.md) now that the model moved
there: the per-file/per-function ACCEPT surface has no syntax yet, and the 29-site uncovered copy
set is sized but not drained.  [QUALITY.md § Store-lifetime residuals](../../QUALITY.md#store-lifetime-residuals)
carries the tracking rows.

The third — F8's *"unmeasured"* `par`/coroutine cells — was **measured at closure and was not
clean**: it exposed two pre-existing native codegen defects (no generator holding a heap local
compiled; the generator tail was silently dropped). Both fixed, both backends. That is why an
unmeasured residual is not a resting state.

**Issues this plan closed** — `loft#774` (F8, the reassignment defect), `loft#775` (a field
alias outliving its owner) and `loft#778` (F1, the loop-var reassignment) are all fixed on both
backends and carry `fixed-pending-merge`; they close when the branch reaches `main`.
`loft#415`, `#426`, `#615` and `#664` were already closed and are cited as landmarks.


## Findings — what each resolved as

The plan's own bar, and the reason "allowed for now" is narrow: **FIXED** where behaviour was
wrong, **CORRECTED** where a statement was false, **STATED** only for a program that is already
correct and merely pays a cost. Copies may rest; wrong behaviour, misinformation and silent
copies may not.

| # | problem | resolved as |
|---|---|---|
| F1 | a view reassigned from a loop var DESTROYED the container (interp-only, silent, total) | **FIXED**, both backends |
| F2 | index-pinned views survive a shifting removal — wrong reads and cross-element corruption | **FIXED** — materialise + advice. The first cut keyed on the CONTAINER and was order-blind, so it also copied views already DEAD at the removal: a lost write plus untrue advice, both on the bar above. Made liveness-aware in F9 step 1 |
| F3 | a reference bound from an element loses its write after a shift | **FIXED as F9** — the program is REFUSED (B-Ref-Reshape). It had been signed off as STATED on a claim measurement contradicts; see § F3 below |
| F4 | re-keying a keyed element through a view makes it unreachable | **FIXED** — a key-field write is a reshape, reusing F2 |
| F5 | copies no diagnostic accounts for — the `exists()` family | **CORRECTED + STATED** — default-on advice naming the lever; measured 27/585 scripts |
| F6 | `LOFT.md` claimed a match capture is a view "whatever the field's type"; scalars copy | **CORRECTED** |
| F7 | claimed `c = v[i]` aliasing was a defect and every bind must copy | **RETRACTED** — all 30 cells conform to C86. The DOC was wrong; see § F7 |
| F8 | a view whose CONTAINER is reassigned reads the replacement — a genuine use-after-free on `--native` | **FIXED**, both backends |
| F9 | disturbing a container while a `&` reference into it is live | **FIXED** — refused at compile time, all three disturbances |

### F3 — the retraction that mattered

F3 was closed as STATED on *"F2's materialise covers it"*. Measured, it does not: the cell emits
**no advice at all** and the write vanishes. A lost write is exactly what the bar never allows,
so the sign-off was a finding refused rather than a finding resolved. It reopened as
[loft#779](https://github.com/loft-lang/loft/issues/779) and closed as F9.

The issue's own boundary table was then wrong in turn — its row A2 says a plain parameter
copies, and probe 40 cell X9 measures it writing 99 into the caller. So the refusal keys on the
ALIASING relation, not on the `&` token. **A filed boundary is a hypothesis, including the cells
it marks CONFORMANT.**

### F7 — the retraction, kept because the mistake is instructive

F7 proposed deleting B-View outright, on a one-cell reading. A 30-cell sweep showed every cell
already conforming to [C86](../../DESIGN_DECISIONS.md). The code was right; the DOC was wrong.
What stops the next one is `tests/scripts/201-bind-copies-projection-views.loft`, which pins the
whole boundary — and the habit of grepping the decision register before reversing documented
behaviour.

## Method — the worked example

The *rules* live in [CODEGEN_METHOD.md § build the instrument before the fix](../../CODEGEN_METHOD.md);
this is the evidence they were drawn from, kept here because a worked example belongs with the
work.

**The oracle did not merely stay quiet — it asserted the opposite.** `--report-copies` said
*"none — every structure copy is a move, a literal, or already borrowed"* on a program that
provably deep-copies. A report that says "clean" about a broken thing is worse than no report:
it is a false negative wearing a clean bill of health.

What replaced it, and what each step actually bought:

1. **Put the instrument where the fact is CREATED.** Copies are minted during emission, so
   `LOFT_COPY_MANIFEST` records them in the branch that emits them — the one place that cannot
   be wrong about whether a copy exists.
2. **Calibrate in BOTH directions, on known answers.** Two mis-installations were caught this
   way and neither by reading code: the plausibly-named `gen_set_first_ref_copy` fires zero
   times in the whole corpus, while the real emitter is `gen_set_first_ref_call_copy`. A dead
   path reads exactly like a live one.
3. **Survey — turn a bug into a distribution.** One repro says a shape exists; the sweep said
   how often and where, which was not where the first report landed.
4. **State the instrument's own coverage.** Until it is honed to every path, "found nothing"
   means *"nothing on the paths I watch"*.

The same discipline produced this plan's later corrections: a sweep over a rule's PRODUCERS
(not a boundary around the reported one) is what found D-bind-9, and running an "unmeasured,
not known-broken" residual is what found that `--native` could not compile any generator holding
a heap local.

## Cluster catalogue

| ID | Cluster | Severity | Backend asymmetry | Probes |
|---|---|---|---|---|
| I | silent copies — the inventory and its two mechanisms | wrong cost, no wrong value | both silent; native has no move at all | 10–14 |
| II | index-pinned views survive a shifting removal | **corruption + silent wrong read** | both identical | 01–09 |
| III | producer set — which binds mint a view | resolved (F7 boundary) | none | 25, 27, 31, 32 |
| IV | invalidator set — what else shifts | resolved (F2/F4/F8/F9) | none | 28–30, 33–35 |
| V | backend parity for the alias/move decision | perf (native) | **native-only gap** | 10, 11 |

**V is the one that did not fully close.** The interpreter has a last-use move for the
whole-record bind, gated on a raw parse-time appearance count; native emits `OpDatabase` +
`OpCopyRecord` unconditionally. Under decision 3 native must GAIN the move rather than the
interpreter lose it. Tracked as part of the uncovered copy set.

## Probe suite

`probes/` — 43 probes, every one run on **both** backends. Not CI-run (see the probe→CI mapping
above for the ones that encode a guarantee); these are the investigation's landmarks.

| range | what it holds |
|---|---|
| 01–09 | the view invariant: growth, shift, re-occupation, nesting, keyed and reassign baselines |
| 10–14 | the copy inventory — each proves its copy BY VALUE, so it stays meaningful when diagnostics change |
| 15–19 | the secret-copy catalogue (`run_set.sh secret`) |
| 20–24 | return-dep discriminators, buffer reuse, escape routes |
| 25–36 | the producer × invalidator matrix — the plan's main boundary |
| 37 | F8's boundary, 17 cells + the concurrency cells added at closure |
| 38 | the cross-frame residual, characterised as cells |
| 39 | F2's liveness boundary, 12 cells |
| [40](probes/40-reshape-refusal/README.md) | F9's refusal boundary, 25 cells, with the *before* column measured on the pre-fix binary |
| 41 | the `&`-alias sweep over the rule's PRODUCERS — what found D-bind-9 |
| 42 | the native coroutine boundary — what running one "unmeasured" residual found |

## What this plan cost, and what it left

Four things this investigation found that were **not** what it was opened to find, each because
a measurement was run rather than a claim trusted:

- **F7's retraction** — a doc, not the code, was wrong; the proposed "fix" would have deleted a
  correct rule.
- **loft#779** — a sign-off whose claim measurement contradicted, reopened and closed properly.
- **D-bind-9** — a rule closed on one of its three producers.
- **The native coroutine defects** — `--native` could not compile any generator holding a heap
  local, and silently dropped every statement after the last `yield` (a dropped side effect, not
  merely a leak). Both pre-existing on `main`, both found by running one residual this plan had
  recorded as *"unmeasured, not known-broken"*. Four faults in all, each with its own chokepoint
  and three of them only reachable once the one before it was fixed. Boundary
  [probes/42](probes/42-generator-heap-locals.loft); CI form
  `tests/scripts/776-generator-heap-locals.loft`, leak-clean on both backends.

The last one is the plan's own lesson turned on itself: **"no specific reason to expect
divergence" is not a measurement.**
