<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Plan-57 closure design

The thesis is delivered — @P393's watermark warnings are gone (cluster II), the
I/III straight-line watermark is fixed (reclaim is default), and the rc-removal
tail-end (follow-up #1) is complete.  This is the checklist to **close the plan
cleanly**: promote the probes that are permanent guarantees into CI tests, file
the sibling bugs the probing surfaced, get the docs (in-plan and out) honest, and
close the issue (`status:finished`).

Execute the workstreams in order **B → A → C → D** (file bugs first so the probes
have a forward home, then tests, then docs, then the close).  One commit per
workstream; `make ci` green before the close.

---

## Workstream B — file the sibling bugs (do first)

These are **sibling discoveries** (a different subsystem from the store-watermark
thesis), so per the plan's own policy ([§ sibling bugs are discoveries](../README.md))
they are filed as P-issues — exactly as @P394/@P395 already were.

The no-file rule for *thesis* findings is scoped to the **active** phase; **on
closure it inverts** — every cluster finding still OPEN gets a forward home now,
because the cluster catalogue becomes a `finished/` record, not a live tracker.
So **Cluster III Route 2 is also filed at close** (see B.4) — the only difference
from a sibling bug is the home: it's a benign tradeoff, so QUALITY.md `## Open
work` (not a PROBLEMS.md bug row).  The FIXED clusters (I-a, I-b, II) need no
filing — their fixes + tests are the record.

**B.0 — re-verify each crash still reproduces on current HEAD** (rc removal +
reclaim-default may have incidentally fixed one).  Run the probe; only file what
still crashes.  If one is now fixed, record it as fixed in the corpus instead.

| # | Bug | Repro (probe) | Site | Notes |
|---|---|---|---|---|
| B.1 | **Returning a tuple that contains a vector crashes** | `probes/bugs/` Bug 1 | `store.rs:1374` | position-independent; literal AND built-mutably; both backends — check |
| B.2 | **Storing a capturing closure into a collection crashes** (the un-handled inverse of `P257`) | `probes/closure-collection/02_closure_into_collection.loft` | `store.rs:1385` | non-capturing closures + named fn-refs in a vector work; workaround = bind the element |
| B.3 | **`vv[0] += [2]` (nested-vec element-compound-assign) hits a codegen assertion** | parallel battery residual (`probes/bugs/` note) | `data.rs:3036` | fires OUTSIDE `parallel {}` too — a standalone codegen bug |
| B.4 | **Cluster III Route 2** (thesis residual, benign) | `probes/cluster-I/` + `cluster-III-…md` | `scopes.rs` (inert `confine_reassign_safe`/`multi_store`) | filed at close to **QUALITY.md `## Open work`** (benign watermark tradeoff, not a PROBLEMS.md bug); the inert foundation is the head-start for a future fixer |

For each still-reproducing bug: minimal reproducer + expected/observed per backend,
severity tier, workaround, and a pointer to the probe as the repro landmark in
[PROBLEMS.md](../../PROBLEMS.md); mirror user-visible rows to
[USER_FACING.md](../../USER_FACING.md); save the repro to `tests/scripts/`
(guarded) or `/tmp/p_followups/`.  (`parallel {}` Bug 2 is NOT filed — its
soundness floor already shipped as `reject_unsound_parallel_captures` +
`tests/scripts/170`; the *feature* is tracked in lib_plans, not as a bug.)

---

## Workstream A — promote probes that are permanent guarantees into CI tests

Doc probes under `probes/` are **not** run by CI.  Anything that is a correctness
GUARANTEE (not characterization/diagnostics) must have a CI test so it can never
silently regress.  Audit:

| Corpus | Guarantee | CI test | Action |
|---|---|---|---|
| rc-removal (closures 01–13) | closures correct without rc | `tests/closure_cell_ownership.rs` + `closure_matrix` + `mut_closure_matrix` | **A.1** — drop the now-retired `RC_OFF=1` env from `closure_cell_ownership.rs` (free is unconditional by default; the env is a no-op) and re-comment so it reads as a default-mode ownership guard, not an RC_OFF probe |
| nrvo-inline-leak (01–09) | unbound heap-temp freed; NRVO'd / borrowed NOT over-freed | `tests/scripts/174-inline-temp-free.loft` | ✅ covered (incl. negative controls) — **A.2** verify 174 still names all 4 lift arms + the borrowed/NRVO controls |
| const-pin | const never freed under unconditional free | `tests/scripts/175-const-pin-no-free.loft` | ✅ covered |
| cluster-I + top-level `01–19` | watermark bounded by reclaim | `tests/watermark.rs` (`Stores::peak`) | ✅ covered |
| bugs/ (parallel soundness) | unsound parallel captures rejected | `tests/scripts/170` | ✅ covered (floor) |
| closure-collection `01` | collection→closure rejected (`P257`) | — | **A.3** (optional, low value) add a compile-error test for the intended rejection |
| closure-collection `02`, bugs/ Bug 1 | (crashes — unfixed) | — | regression added WHEN B.2 / B.1 are fixed; until then the probe + P-issue are the record |
| lib-test-file-concurrency `01` | file-close correct under Phase C | harness-verified (full suite) | stays a doc probe — it writes fixed `/tmp` names, so it can't be a `tests/scripts/` file without re-introducing the very concurrency race it documents |

Net: coverage is essentially complete; the only required code change is **A.1**.

---

## Workstream C — documentation

### In-plan
- **C.1 `README.md`** — update the Status table (Stage D) to the final state
  (reclaim default + rc removed); add the ✅ CLOSED banner (see Workstream D).
- **C.2 `RESULTS.md`** — confirm it reflects the shipped end-state, not a mid-investigation snapshot.
- **C.3 `cluster-III-reassignment-pin.md`** — mark **Route 2** as the ACCEPTED benign
  residual (exit-safe, below the warn floor, fix mapped, foundation
  `confine_reassign_safe`/`multi_store` inert in `scopes.rs`); link the QUALITY.md pointer (C.7).
- **C.4 `fix-design-store-lifetime.md`** — mark the "Tail-end experiment (disable
  rc)" section **DONE** → point to `probes/rc-removal/` + the Phase A/B/C commits.

### Outside the plan
- **C.5 stale rc references** — DEBUG.md, COMPILER.md, DEVELOPMENT.md,
  PERFORMANCE.md, THREADING.md, CHANGELOG_TECHNICAL.md describe `ref_count` /
  `inc_rc` / `dec_rc` / `OpIncRc` as LIVE.  Update each to past tense / "removed
  plan-57 Phase C".  **THREADING.md** specifically: the shared rc counter was a
  cross-thread contention point — now gone (a parallel win worth stating).
- **C.6 `LIFETIME.md`** — the canonical lifetime doc.  Rewrite the store-lifetime
  section to the new model: **single-ownership free at scope end + closure-record
  cascade owns captured cells + `Store.pinned` for const/global; no ref-count.**
- **C.7 `QUALITY.md`** — add a `## Open work` pointer for **Cluster III Route 2**
  (B.4).  QUALITY.md rather than a PROBLEMS.md bug row because it's a **benign
  watermark tradeoff**, not a correctness defect — NOT because of any no-file rule
  (that rule is active-phase only; on closure all open findings get filed — here
  the right home is just QUALITY.md, not PROBLEMS.md).
- **C.8 `GOALS.md` Goal E** — note that rc removal advanced Goal E (the
  programmer's model is the truth; no hidden counter decides lifetimes) and
  update its Check/status.
- **C.9 `CHANGELOG_TECHNICAL.md` [Unreleased]** — one entry covering the arc:
  Phase A/B/C (free unbound heap temps; closure-cell cascade ownership; delete
  `ref_count`/`inc_rc`/`dec_rc`/`OpIncRc` + the `pinned` flag), opcode renumber
  note, and the lib-test temp-CWD isolation.  No user-facing CHANGELOG line needed
  (no behaviour change — except "memory is now predictable", a Goal-E note).
- **C.10 `PROBLEMS.md` / `USER_FACING.md`** — the rows from Workstream B.

### Follow-up routing (confirm, don't re-document)
- **C.11** parallel-capture FEATURE (#2) — confirm it's referenced in
  `lib_plans` 08-server / 10-game-client (its real consumer); the soundness floor
  already shipped.
- **C.12** nightly backend-parity sweep (#3) — confirm it's tracked in
  GUARDS.md § Backend divergence as the Goal-D standing detector.

---

## Workstream D — close-out mechanics

- **D.1** Add the close-out banner to the plan `README.md` (the dir stays in
  place — no move):
  > ✅ **CLOSED 2026-06.** @P393 resolved (cluster II) + reclaim default (I/III
  > straight-line) + rc removed (follow-up #1, Phases A/B/C).  Tracked elsewhere:
  > Cluster III Route 2 → QUALITY.md; parallel feature → lib_plans 08/10; nightly
  > parity sweep → TESTING.md; sibling bugs B.1–B.3 → PROBLEMS.md.
- **D.2** Set the issue label + close it: `gh issue edit <N> --repo
  loft-lang/plans --remove-label status:active --add-label status:finished`,
  then `gh issue close <N>`.  See [`../../_LIFECYCLE.md`](../_LIFECYCLE.md).
- **D.3** `ROADMAP.md` — remove the 57 row from the active section.
- **D.4** `make ci` green; commit the closure.  (The plan path is unchanged, so
  no link repointing is needed beyond reference content moved out in Workstream C.)

---

## Sequenced checklist

1. **B.0** re-verify the 3 crashes → **B.1–B.3** file the survivors (PROBLEMS.md + USER_FACING.md).
2. **A.1** de-RC_OFF `closure_cell_ownership.rs`; **A.2** confirm 174; (A.3 optional).
3. **C.1–C.4** in-plan docs → **C.5–C.10** outside docs → **C.11–C.12** confirm routing.
4. **D.1–D.4** banner, close the issue (`status:finished`), ROADMAP, `make ci`, commit.

Estimated effort: **S–M** — mostly doc edits; the only code is A.1 (test hygiene)
and the B-bug repros.  No behaviour change ships from the closure itself.
