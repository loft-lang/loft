<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan lifecycle checklist — closing or deferring

Use this when a plan exits the active state.  **Lifecycle state lives on the
[`loft-lang/plans`](https://github.com/loft-lang/plans) issue's `status:*` label,
not on the directory.**  We no longer move plan directories: an existing local
`plans/<N>-<slug>/` dir stays where it is and its README becomes the closure (or
defer) record in place.  The `finished/` / `deferred/` / `future/`
subdirectories are a **legacy archive** from the old local-numbering era — read
them, but don't add to them.

Two outcomes share most of the procedure:

- **Closing** — all phases shipped → issue `status:active` → `status:finished`,
  then close the issue.
- **Deferring** — remaining work has a **concrete trigger**; issue stays open; record
  the trigger. If a floor **shipped** (some phases delivered, rest paused) →
  `status:parked`; if **nothing** shipped (all phases still to start) → `status:future`.

If remaining work has no concrete trigger, the design moves to the decision register
[`../DESIGN_DECISIONS.md`](../DESIGN_DECISIONS.md) — an index: the entry goes in the subject's
`DESIGN_DECISIONS_<SUBJECT>.md` under the id `./scripts/idx next-decision` names
([`../DESIGN_DECISIONS_RULES.md`](../DESIGN_DECISIONS_RULES.md)).  "Will get to it later" is not
a trigger; "when 3+ template-path bugs accumulate" is.

## Pick the outcome

| Situation | Outcome |
|---|---|
| All phases shipped | **Close** — `status:finished`, close the issue |
| Some phases shipped, others paused with concrete trigger | **Partial defer** — `status:parked`, issue stays open; Status table grows SHIPPED / DEFERRED rows.  Canonical: @PLN43 (Tier 1 shipped), @PLN82, @PLN80. |
| No phases shipped, all paused with concrete trigger | **Full defer** — `status:future`, issue stays open |
| Some phases shipped, others abandoned without trigger | Close the shipped portion as above; remaining design moves to `DESIGN_DECISIONS.md` |
| Waiting on a date or "appetite" with no concrete signal | Keep `status:future` (date-bound) or move design to `DESIGN_DECISIONS.md` (appetite-bound).  Don't defer. |

## Steps 1-3 — Per shipped phase, extract reference content

These steps run for **every shipped phase**, regardless of close
vs defer.  Skip for phases being deferred (they keep their design
content in the plan README).

### Step 1 — Tag each section

Read the plan README + each phase file.  Tag every section as one
of:

- **REFERENCE** ("how things work today after this ships") → moves out
- **CLOSURE RECORD** ("what was done, when, what was retracted, why") → stays
- **HISTORICAL ARCHAEOLOGY** ("what was tried that didn't ship") → stays as historical record (don't move into reference; reference is for current truth)

### Step 2 — Pick the reference home

Two shapes:

- **CREATE-AND-MOVE** — no comprehensive reference doc exists.
  Create `doc/claude/<NAME>.md` (or extend `lib/<name>/<doc>.md`
  for library-scoped reference) with the extracted content.
  Canonical example: `31-html-export → HTML_EXPORT.md`.
- **TRIM-ONLY** — reference doc already covers shipped state.
  Trim plan README + cross-link to the existing home.
  Canonical example: `04-slot-assignment-redesign → SLOTS.md`.

### Step 3 — Trim plan README to closure-record shape

Target ~50-150 lines.  Lead with:

> **Status — DONE YYYY-MM-DD** (or **SHIPPED** for partial defers)
> Reference for the post-plan content moved to `<path>`.  This file
> is a closure record only.

Drop pre-shipping content (Goal-as-motivation, Why-this-is-an-
initiative, Ground rules, Verification gates, Risks).  Keep
Status block, per-phase outcome table (compressed), the technical
insight or P-issue table that's the headline value, See also
cross-links.

For partial defers, the plan README also keeps **full design
content for deferred phases** alongside the closure-record for
shipped ones — see @PLN82's Phase B + Phase C sections as the
canonical shape.

## Steps 4-6 — Common to close + defer

> **Investigation plans — two extra obligations BEFORE the move** (the
> `plans/*/` shape with `probes/` + a cluster catalogue).  See
> [`_INVESTIGATION_TEMPLATE.md` § Closing an investigation plan](_INVESTIGATION_TEMPLATE.md#closing-an-investigation-plan-required):
> (1) **file the still-open findings** — the active-phase no-file rule inverts at
> closure, so a deferred/benign residual → QUALITY.md `## Open work` and an
> unfixed sibling bug → PROBLEMS.md, each citing the plan's cluster doc;
> (2) **promote permanent-guarantee probes → CI tests** (doc-probes aren't
> CI-run).  A FIXED finding needs no row — the fix + its test are the record.

### Step 4 — Reclassify ROADMAP rows

For each ROADMAP row touched by the plan:

- **Shipped parts** — remove from ROADMAP (closure lives in
  CHANGELOG + git history per the maintenance rule).
- **Deferred parts that are roadmap-tracked** — leave on ROADMAP;
  update plan path to `plans/deferred/<NN>-<slug>/`.
- **Deferred parts that are trigger-only** — remove from ROADMAP.
  The DEFERRED.md row is enough.

For the "Plans index by category" subsection at the bottom of
ROADMAP.md, move/remove rows accordingly.

### Step 5 — Set the issue label + close the issue (THE lifecycle state)

The `loft-lang/plans` issue's `status:*` label IS the lifecycle state — this
step replaces the old directory move.  **Do not `git mv` the local dir**: it
stays in place and its README (now the closure / defer record) travels with it.
The `status:*` labels are `active` / `future` / `finished` (`gh label list
--repo loft-lang/plans`).

```bash
# Closing — all phases shipped:
gh issue edit <N> --repo loft-lang/plans \
  --remove-label status:active --add-label status:finished
gh issue close <N> --repo loft-lang/plans \
  --comment "Closed: all phases shipped (<commit/PR>). Reference → <doc home>."

# Deferring — paused with a concrete trigger (issue stays OPEN):
gh issue edit <N> --repo loft-lang/plans \
  --remove-label status:active --add-label status:future
#   then record the trigger in the issue body + the plan README Status block
```

There are no per-state tables in `plans/README.md` / `lib_plans/README.md` to
edit — tracking lives on the issue (`_TEMPLATE.md` step 3).  A deferred plan
additionally records its **trigger** in [`DEFERRED.md`](DEFERRED.md).

**Closing is automated at release.**  Put a close directive in the PR that
finishes the plan — `Closes @PLN<n>` (cross-repo `Fixes #N` can't reach the
plans repo) — and on merge to `main` the `close-plans` workflow runs
`scripts/close-shipped-plans.sh`, doing the `status:finished` + close above for
you.  So the manual `gh` here is the *fallback* (or for an out-of-band close);
`scripts/audit-stale-plans.sh` is the drift sweep — it runs **daily** in the
nightly checks (`miri.yml` → `stale-plans-audit`) and makes two passes.  It
*warns* about a plan left `status:active` after its work shipped, because that is
a judgement (the work may have landed via a PR this repo's history cannot see).
It **fails** on a CLOSED plan still carrying a live label, because that is not:
the state and the label contradict each other outright.  That second pass exists
because the warning form was not enough — @PLN48 (`status:future`) and @PLN102
(`status:next`) sat wrong for a month, closed by the automation, which removed
only `status:active` and reported success anyway.  `close-shipped-plans.sh` now
strips every live label; the audit catches what it cannot reach — hand-closes,
and PRs that used `Refs` instead of `Closes`.  See
[RELEASE.md § Closing plans when the release merges](../RELEASE.md#closing-plans-when-the-release-merges).

### Step 6 — Repoint design links to the reference home

The plan path does NOT change (no move), so there are no path-only fixes — but
after Steps 1-2 move reference content out, incoming links that pointed at the
plan for *design / how-it-works* must follow it to the new home.

```bash
grep -rn "plans/<N>-<slug>" CLAUDE.md doc/claude/ --include="*.md"
```

Rewrite each match:
- **Design / how-it-works links** → reference home (the
  `doc/claude/<NAME>.md` location).
- **Closure-record links** → keep pointing at the plan README (history,
  what-shipped) — its path is unchanged.

Verify `scripts/check_doc_drift.sh` reports no broken plan paths
afterwards.

## Lifecycle after defer

A deferred plan keeps `status:future` (issue open) while ANY remaining phase has
a concrete trigger.

- **Trigger fires** → set the issue back to `status:active`; remove the
  DEFERRED.md row; add ROADMAP rows; repoint design links per Step 6 if
  reference content had moved.
- **Remaining phases ship** → re-apply this checklist, this time closing.
- **Remaining phases reclassified as won't-do** → design moves to
  `DESIGN_DECISIONS.md`; close the issue (`status:finished`) noting the partial
  delivery; remove the DEFERRED.md row.

## Pitfalls

1. **"Will get to it later" is not a trigger.**  If the trigger is
   "when contributor appetite arrives" without a concrete signal
   (a bug pattern, user-request shape, count threshold), the work
   belongs in DESIGN_DECISIONS.md.
2. **Half-done phases corrupt the Status table.**  Either close
   the phase properly (extract its reference content) or roll it
   back (drop it from "shipped" status).  Don't leave "Phase 3 is
   sort of done."
3. **Step 4 (ROADMAP reclassification) is the most-forgotten
   step.**  Shipped parts staying on ROADMAP is the most common
   drift; an audit then thinks they're still open.
4. **Step 6 (link rewrite) is the second-most-forgotten.**  Run
   `scripts/check_doc_drift.sh` after every close/defer to catch
   broken paths.
5. **Stale "removed in plan-X" notes.**  When you retire a feature
   (Type::Long, text_code, .loftc, forwarding_smoke.rs), update
   the docs that mentioned it as current.  Drift script flags
   common shapes.

## Cross-links

- [`README.md § Three workflows`](README.md#three-workflows-for-todo-items--pick-the-lightest-that-fits) — when to use a plan vs the lighter `## Open work` flow.
- [`README.md § Light flow lifecycle`](README.md#light-flow-lifecycle---open-work-in-reference-docs) — close + defer for the light flow (no directory move; just edit a row).
- [`_TEMPLATE.md`](_TEMPLATE.md) — for new plans.
- [`DEFERRED.md`](DEFERRED.md) — trigger index this checklist feeds.
- `scripts/check_doc_drift.sh` — drift detection for path / shipped-claim staleness.
