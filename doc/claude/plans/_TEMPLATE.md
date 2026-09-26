<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan template

Copy this file to `<N>-<slug>.md`, or to `<N>-<slug>/README.md` when the plan has
companion files — **flat**, no `future/` subdir — when opening a new plan.  **`<N>` is the plan's
[`loft-lang/plans`](https://github.com/loft-lang/plans/issues) issue number**
(`@PLN<N>`), NOT the next local directory integer.  State lives on that issue, not the
directory; there is **no ROADMAP row** (the overview is derived from the loft-lang
issues).  Full procedure: the loft-plan-workflow skill (Procedure A — NEW MODEL).

The sections below are the canonical shape.  Sections marked
**(REQUIRED)** must be present in every plan; sections marked
**(OPTIONAL)** depend on plan shape.

## Before you copy — pick the right shape

Per [`README.md § Three workflows`](README.md#three-workflows-for-todo-items--pick-the-lightest-that-fits):

- **Bug fix** — PROBLEMS.md row + regression test + commit.
  No plan, no Open work entry.
- **Light TODO** — add a row to `## Open work` in the relevant
  `doc/claude/<NAME>.md` reference doc.  This is the default;
  most TODOs fit here even when they take several sessions.
- **Standard plan** — this template.  For genuinely multi-phase
  feature work or fix arcs that benefit from their own directory:
  explicit phasing, design-before-implementation discipline,
  cross-arc dependencies, multiple sub-files (DESIGN.md, ARC.md,
  per-phase files).  **Primary deliverable: code shipped.**
- **Investigation plan** — see
  [`_INVESTIGATION_TEMPLATE.md`](_INVESTIGATION_TEMPLATE.md).
  For plans whose first phases are "characterize a failure
  class" before fix design — probes/ subdirectory + per-cluster
  investigation docs + verified-vs-hypothesized accountability.
  **Primary deliverable: mechanism understanding + fix-design
  decision.**  Canonical example: `plans/finished/51-hidden-buffer-aliasing/`.

If your TODO fits in one row of a reference-doc table with one
sentence of design, you don't need a plan — close this template
and add the row.

---

# <NN> — <Plan title>

## Status (REQUIRED)

This is the **single source of truth** for what's shipped / open /
deferred / blocked.  Other docs (the loft-lang issue, DEFERRED.md,
downstream plans) carry the plan name + dependencies but not the
per-phase status — readers who want the exact state come here.  No
duplication, no sync.

One paragraph.  State of the world today + what this plan will
change.  Examples:

- "Open — design ready, no implementation yet.  Routes here
  per the docs-vs-plans rule."
- "Mixed: JSON shipped (`Type.parse`); HTTP planned for 1.1+.
  Three-file split: README (overview), JSON.md (shipped
  reference), HTTP_CLIENT.md (planned design)."
- "Pointer-plan: full design lives at <!--noindex-->
  [`../../../FOO.md`](../../../FOO.md); this plan tracks <!--noindex-->
  open work as actionable rows."
- "Pure future plan; pre-flight 50% bug yield expected."

## Goal (REQUIRED)

One sentence.  What this plan ships when complete.  Avoid
strategy / advertising language; that lives on the loft-lang issue
or in RELEASE.md.

## Effort + design (OPTIONAL — recommended)

State the `E` (effort) and `Design` here so readers get sizing without
leaving the plan.  (The `loft-lang/plans` issue may mirror them, but this
README is the source.)

- **Effort:** XS / S / M / MH / H / VH
- **Design:** ✓ (detailed) / ~ (partial) / — (needs design)
- **Last touched:** YYYY-MM-DD (auto via `git log -1`)

## Composition matrix — Stage A (REQUIRED for plans that add or extend a value, type, or operation)

*Before* the implementation phases, enumerate the feature's cells against the
**composition axes** ([README § The composition axes](README.md#the-composition-axes--the-dimensions-a-matrix-varies))
— the axes your change actually touches — and write them as `/tmp` probes first, on
`--interpret`.  This matrix is the spec: the feature is done not when the demo runs
but when **every cell is green on both backends**, and the probes graduate to
`tests/scripts/` as its regression suite.

The bug class this prevents is the one plan-58 shipped — an invariant re-derived in
several code paths, validated only where the derivations happened to coincide.  So
the design half of the same discipline: give each fact the feature introduces **one
home** every path consults ([Goal E](../GOALS.md) — source is the truth — applied to
the feature's *representation*, not just its memory).  With single-home invariants
the off-diagonal cells cannot disagree; the matrix is how you *prove* it.

Skip this section only for plans with no new composition surface (pure refactor,
pointer-plan, docs) — and say so in one line when you do.  Silence reads as "matrix
done", not "matrix N/A".

## Sub-arcs (REQUIRED if multi-phase; OPTIONAL otherwise)

Status table.  One row per arc / phase / sub-feature.  Each
row links back to the design source if reference content
lives elsewhere (pointer-plan shape) or to the phase file
if it lives here.

**`Verify` names the comparison that would go RED if this phase were done wrong** —
a test file, a both-backends matrix cell, a byte-identical `introspect` diff, a
round-trip fixture, a rendered image.  `Source` says where the design is; `Verify`
says how you would find out it is wrong, and they are different questions.

Fill `Verify` **when you cut the phase**, not when you implement it.  An empty cell
does not mean verification is pending — it means the phase is not cut yet, because
nothing about it could fail on its own.  That is the lower bound in the
loft-plan-workflow skill (§ Cutting a phase): a phase that ends with something built
and called by nobody is green by construction.  An `H`/`VH` phase usually fails the
*other* bound — too big to have a half-done state with anything exact to compare
against; split it until each part has its own comparison.

| Item | Source | Verify | Status |
|---|---|---|---|
| **A** — short title | (link) | `tests/scripts/NN-<slug>.loft`, both backends | Open / In-flight / Shipped |
| **B** — short title | (link) | `introspect` byte-identical vs A | Open / Blocked on X |

## Phase ordering (OPTIONAL — for multi-arc plans)

Suggested sequence when this plan unpauses.  Numbered list,
each item one or two sentences.  Note dependencies between
arcs.

## Open design questions (OPTIONAL)

Questions that need answers before implementation starts.
Numbered list.  Each question's resolution becomes a
decision recorded in `DESIGN_DECISIONS.md` or absorbed
into the design content here.

## Cross-arc dependencies (OPTIONAL)

When this plan depends on or cooperates with sibling plans,
list them here as a bullet list with one-line rationale.
Helps the dependency graph stay visible.

## See also (REQUIRED)

- Reference doc(s) that this plan implements / extends.
- Sibling plans that cooperate or that block this one.
- The `loft-lang/plans` issue (`@PLN<N>`) that tracks this plan, and any source
  bug/enhancement issue it was promoted from.

---

## Authoring notes (delete from your plan README)

**Length budget:** 100-300 lines.  Plans longer than 300
lines tend to be reference content that should move to
`doc/claude/<NAME>.md` or be split.

**File names within the plan dir:** `README.md` is required.
Sub-files for distinct concerns: `IMPL.md`, `DISCUSSION.md`,
`PROTOCOL.md`, etc.  Multi-file plans should match the
EVENT_LOOP / OPENGL / WEB_SERVICES precedent: each sub-file
has a single concern.

**On opening a new plan** (new model — full procedure in the loft-plan-workflow skill):
1. Create the plan's [`loft-lang/plans`](https://github.com/loft-lang/plans/issues)
   issue — its number is `@PLN<N>`, and the issue **is** the plan.  No local
   slot.  Only a **big multi-phase design** adds an optional local dir, named for
   the issue: `plans/<N>-<slug>/README.md` (or `lib_plans/<N>-<slug>/`), `<N>` =
   the issue #, using this template's shape.  Small plans live in the issue alone.
2. Fill in Status + Goal first; link the loft-lang issue + carry `@PLN<N>` in the body.
3. **Label the loft-lang issue**: `subject:*` (loft / libs / audience) + `status:*`
   (future / active / finished).  (No `plan` label — every issue in the `plans`
   repo IS a plan, so the marker carried no information; retired 2026-06-14.)
   **No ROADMAP row, no plans/README table row** — tracking lives on the issue.
4. **Do NOT add a CLAUDE.md doc index entry by default.**  Add ONLY if the plan
   introduces a NEW top-level reference concept with no doc-root home (vanishingly rare).

**On closing or deferring a plan:** see [`_LIFECYCLE.md`](_LIFECYCLE.md).
