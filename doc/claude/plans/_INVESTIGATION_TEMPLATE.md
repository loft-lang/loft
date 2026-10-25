<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Investigation-plan template

Copy this file to `<NN>-<slug>/README.md` when opening a plan whose
**primary deliverable is mechanism understanding** — characterising
a failure class, family of bugs, or unfamiliar subsystem before any
fix shape can be committed to.  Use the standard
[`_TEMPLATE.md`](_TEMPLATE.md) when the primary deliverable is a
feature ship instead.  The two shapes look different:

| | Standard plan | Investigation plan |
|---|---|---|
| Primary deliverable | Code change shipped to users | Mechanism understanding + fix-design decision |
| Reading order | Goal → Design → Sub-arcs → Implementation | Status → Probes → Cluster docs → Roadmap |
| Sub-files | DESIGN.md, IMPL.md, per-phase files | RESULTS.md, cluster-*.md, probes/ subdirectory |
| Length budget | 100-300 lines per file | 100-300 README + ~200 per cluster doc + probe headers |
| When promoted to tests | When phases ship | When mechanism is pinned and fix lands |

## Primary goal: loft stability, not single-bug closure

The point of an investigation plan is to make loft **stable** —
to close a failure CLASS so the next session doesn't keep
re-discovering siblings of the same shape.  Fixing the single
bug that prompted the investigation is rarely enough on its own.
Two settings make this acute:

- **Memory management.**  Hidden buffer aliasing, dep tracking,
  store ownership, and closure capture interact through several
  code paths at once.  A fix that closes the reported repro often
  leaves siblings — different shapes that hit the same broken
  mechanism — silently leaking, corrupting, or panicking on
  teardown.  The probe suite is the safety net: it catches the
  siblings the original repro never touched.  PLAN51 (5 clusters,
  62 probes) and PLAN52 (cluster II/IV/V iteration history) are
  both stability investigations whose probe coverage outlasted
  the one-line fix that opened them.

- **New rustc versions.**  Rust's UB definitions, optimiser
  behaviour, lifetime inference, and clippy lint set shift between
  toolchain releases.  A pattern that compiled cleanly under one
  toolchain can produce miscompiled native code, new lint
  failures, or borrow-checker rejections under the next.  Without
  the probe suite re-running on toolchain bumps, regressions
  surface as user reports months later instead of as a red CI on
  the bump commit.

Investigation plans pay for the higher up-front cost (probes +
cluster docs + per-cluster commit discipline) by leaving the
class CLOSED — provably, by the probe sweep — rather than just
the one shape that prompted the report.

**A plan is warranted ONLY when the scope is genuinely hard to
pin down** — when you can't yet write the fix because you don't
know what you're dealing with, and a probe-driven investigation
is the way to find out.  Once a bug's scope and root cause ARE
pinned, there is nothing left to investigate: **fix it** (with a
regression test).  Don't open a plan around a fix you could
already write.  And don't file a P-issue for a bug you understand
well enough to fix — filing only *documents a bug for later*;
fixing removes the need to.  If the goal is "stop seeing this
CLASS of bug in this subsystem" — siblings keep re-appearing and
the scope spans several interacting mechanisms — use the
investigation-plan shape.

**The investigation plan IS the future-documentation.**  Its
probes + cluster docs already record every shape and mechanism,
which is the whole reason filing exists.  So findings discovered
*during* the investigation are FIXED in-plan and recorded in the
cluster catalogue — never *also* filed as separate P-issues.
Double-documenting the same shape in PROBLEMS.md is redundant
(see § In-plan vs spinoff policy).

Investigation plans tend to have **more files** than standard plans
(probes + cluster docs + RESULTS + README).  This is justified
*proportionally to the investigation depth* — NOT a default.  A
shallow investigation belongs in `## Open work` on a reference doc;
a deep investigation earns this structure.

The canonical example is `plans/finished/51-hidden-buffer-aliasing/`
(closed 2026-05-29) — 5 clusters, 62 probes, full per-cluster
mechanism docs + 3-attempt fix-iteration journal in
[`cluster-II-latent-leak.md`](finished/51-hidden-buffer-aliasing/cluster-II-latent-leak.md).
Use it as a reference for the layout below.

---

# <NN> — <Investigation title>

## Status (REQUIRED)

State of the investigation: shape catalogue progress, mechanism
verification status per cluster, what's blocked.  Lead with the
status table:

| Stage | Status |
|---|---|
| A — Probe catalogue | 🟡 in progress / ✅ complete |
| B — Mechanism investigation | 🔴 not started / 🟡 N/M clusters verified / ✅ complete |
| C — Fix design (OPTIONAL) | ⏸️ pending Stage B / N/A — mechanism uniquely determines fix |
| D — Implementation | ⏸️ pending Stage C |

**Stage C is OPTIONAL.**  Skip C and go B → D when mechanism
analysis uniquely determines the fix shape; force C when multiple
fix options need comparing (refcount vs targeted vs hybrid).
PLAN51's clusters II/III/IV/V went B → D iteratively — each fix
landed as its own commit, pushed before the next began (see
§ Fix-application discipline).

Then one paragraph: what triggered this investigation, what the
scope is, what's already known going in.

## Goal (REQUIRED)

One sentence.  What this plan ships when complete.  For an
investigation plan, this is typically a fix design + a phased
implementation — NOT a vague "understand X better".

## In-plan vs spinoff policy (default: in-plan)

Findings discovered during the investigation are **fixed in-plan**
and recorded in the cluster catalogue — **not** filed as separate
PROBLEMS.md P-issues (the plan's probes + cluster docs already
document them; a P-issue would double-document the same shape).

**This no-file rule is scoped to the ACTIVE phase.  On closure it
INVERTS** — every finding still OPEN (a deferred / benign residual,
or a sibling not yet fixed) IS filed then, because the cluster
catalogue becomes a `finished/` closure RECORD, not a live tracker,
so an open finding needs a forward home: **PROBLEMS.md** for a bug,
**QUALITY.md `## Open work`** for a benign tradeoff — each citing the
now-`finished/` cluster doc as the repro landmark.  See
[§ Closing an investigation plan](#closing-an-investigation-plan-required).

Spin off as a separate P-issue / mini-plan ONLY when one of:

1. **Truly an edge case users won't hit** — e.g., parser refusal on
   a syntactically-invalid construct nobody writes deliberately.
2. **Needs its own investigation plan** — fix surface is large
   enough or touches enough unrelated subsystems that bundling
   would balloon the plan beyond reviewer-friendliness.

Default in-plan is safer because (a) cross-cluster overlap is
common (fixing X often closes Y too, but only if Y stays in the
probe-gate); (b) the cumulative probe coverage IS the regression
guard for the whole class.  See PLAN52's experience with cluster VI
(closure body) — was almost spun off; kept in-plan because the
fix likely shares the cluster I surface.

## Cluster catalogue (REQUIRED — replaces Sub-arcs)

The failure modes discovered during exploration, one row per
distinct cluster.  Each cluster gets its own `cluster-<id>-<slug>.md`
investigation doc.

| ID | Cluster | Severity | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| I | one-line shape | severity | both / interp / native | count | `cluster-I-<slug>.md` (link when created) |
| II | … | … | … | … | … |

## Probe suite (REQUIRED)

Each probe is a self-contained `.loft` file in `probes/` with
assertions that turn the failure into a deterministic test result.

The minimum viable index is a **flat list** keyed by probe number:
shape, cluster, status.  Use one row per probe regardless of role.

| File | Shape | Cluster | Status |
|---|---|---|---|
| `01-<slug>.loft` | <shape> | reference | passes — baseline |
| `02-<slug>.loft` | <shape> | <cluster id> | <fails with X / now PASS> |

**Optional A/B/C curation**: when the suite grows past ~15 probes,
the flat list becomes hard to scan and an explicit "primary probe per
cluster vs everything else" split helps.  PLAN51's experience (62
probes, formal A/B/C table) showed the curation overhead doesn't pay
back during implementation — investigators worked from cluster docs,
not the curation table.  Add the A/B/C split only when probe count
makes the flat list painful AND multiple investigators are reading
the suite cold.

**Probe naming**: `NN-<descriptive>.loft`.  Numeric ordering for
stable references; descriptive suffix.  Probes promote to
`tests/scripts/NN-<descriptive>.loft` when their cluster's fix
lands.

**Probe promotion gate**: a probe is graduation-ready only when it
passes ALL of:

1. **Assertions pass** (`probe NN PASSED` prints).
2. **Clean process exit** — no SIGSEGV / panic at teardown.  Run the
   probe and check the exit code; "PASSED prints" is not enough
   (PLAN51's probe 08 printed PASSED then SIGSEGV'd at process exit,
   corrupting the loft_suite run during graduation).
3. **No leak warning** — run with `LOFT_STORES=warn` (or rely on the
   loft_suite leak gate); zero `stores not freed` lines.
4. **Bounded runtime** — completes in seconds, not minutes.  An
   infinite loop in a graduated test hangs the suite (PLAN51's
   probe 22 hangs forever; graduation would have wedged CI).

A probe that passes assertions but fails any other gate stays in
`probes/` with a status note; substitute a representative variant
when graduating.

### Curated probe sets + runner script (REQUIRED at probe count ≥ 20)

Once the suite exceeds ~20 probes, running everything against
every fix attempt is impractical.  Curate into **named sets**
(A, B, … one per cluster + Set H for baselines + Set Z for
known-broken-skip), each ≤10 probes, plus a `probes/run_set.sh`
runner that takes the set letter.

PLAN52 found this discipline load-bearing: the runner localises
fix-validation to <30s per set vs minutes for the full sweep,
and **Set H (baselines that MUST always PASS)** catches scope-
handler / codegen regressions that the @PLAN51 history showed
routinely break adjacent things.  Default the runner to arm
the project's watchdog (e.g. `LOFT_TIMEOUT`) so probe hangs
self-terminate with a localised breadcrumb instead of needing
manual `pkill`.

The set definitions live in the plan README's "Probe suite"
section as a table mapping set-letter → probes → purpose →
current status.  Probe-set authoring is a Stage A deliverable,
not deferred to Stage D fix-validation.

## Reference ↔ problem pairings (REQUIRED if probes ≥ 5)

Each problem probe paired with its closest passing reference.
Diffing the pair is the diagnostic shortcut for understanding
each failure.

| Problem | Reference | What the diff reveals |
|---|---|---|
| 02 | 01 | <one-sentence summary of the divergence> |

This pairing structure makes Stage B's mechanism work concrete:
"trace probe X vs probe Y under `<env vars>` and identify the
divergent opcode emission".

## Tool gaps (OPTIONAL but recommended)

Tools added or verified during this plan's investigation work.

| Tool | Status | Used for |
|---|---|---|
| `LOFT_LOG=<key>` | New / Already-existed / Verified-suitable | <what it pinned> |
| Watchdog / timeout (`LOFT_TIMEOUT` + `LOFT_TIMEOUT_CLEAN_EXIT`) | Verified-essential | Probe-runner default; localises hangs to a `phase=…` breadcrumb |

Tools added during a plan are part of its output, not separate
work.  Closing the plan should leave the tools in tree.

## Status & next-session roadmap (REQUIRED)

For each cluster, the action needed to advance and its effort
estimate:

| Cluster | Mechanism status | Action needed | Effort |
|---|---|---|---|
| I | ✅ Fully understood | — | — |
| II | 🟢 Hypothesis verified by side evidence | Read <file:line>; pin opcode | <XS-L> |
| III | 🤔 Hypothesized | Trace probe X with `<env var>` | <XS-L> |

Then prose: aggregate effort estimate (Phase B-finish + Phase C
+ Phase D), recommended sequence, quickest-user-visible-win
callout.

### Per-step exit criteria (REQUIRED when fix sequencing has > 2 steps)

Each fix step in the sequence gets a binary exit criterion
mapped to probe-set runs.  The plan is provably closed iff the
final-step exit criteria hold — no informal "we think it's done".

Minimum form:

| # | Step | Exit criteria (mapped to probe sets) |
|---|---|---|
| 1 | Fix cluster X | Set X + Set H all PASS; project CI green |
| 2 | … | … |

The closing check at the bottom: all sets (except explicitly-
out-of-scope Z entries) PASS on every backend + project CI green
+ canary suite green.  Required when sequencing > 2 steps.

## Fixing a finding — probe + agent BEFORE code (REQUIRED)

An investigation plan exists **because its bugs are hard** —
complex-variant, multi-subsystem, with a non-obvious fix surface.
So a fix is **earned by a rigorous method, never written ad-hoc
from a first read of the code.**  The willingness to "just fix" an
encountered bug mid-investigation is the anti-pattern this rule
exists to stop.  For every non-trivial finding the sequence is:

1. **Probe the real shape first** (the edge-probe default —
   [plans/README § Edge-probe](README.md#edge-probe-before-fixing--the-lightweight-default-for-lofts-complex-variant-bugs)).
   Build/extend a probe corpus that maps the shape across its
   combinatorial region, on BOTH backends — not a single repro.
   Most loft bugs are complex-variant: a one-repro fix under-fixes
   the class or hides an unsound edge.  The probes ARE the
   verification harness AND the regression landmarks.  A first
   hypothesis is a thing to **refute with probes**, not a conclusion
   (e.g. "this bug = method-vs-function" turned out to be
   "de-NRVO'd vector return" only after the probes killed the first
   framing).

2. **Run a CODE-ONLY investigation agent before editing.**  Dispatch
   a code-only agent (read / grep / run only — it does NOT edit; you
   apply the fix) to pin the exact fix site(s), trace the downstream
   machinery the fix leans on, enumerate the hazards (double-free,
   ABA, native parity, regression in an adjacent system), and
   estimate scope.  This is where the hard fix surface gets mapped
   before a line is written — it catches the wrong-site /
   missed-hazard / under-scoped fix the cheapest way, and its
   per-probe oracle prediction becomes the post-fix check.

3. **Then fix + verify** against the FULL probe corpus on both
   backends + the agent's predicted oracle: the targeted probes flip
   to clean, the controls stay clean (no over-fix / double-free),
   project CI green.

**Don't dismiss without probing, either.**  "This is probably benign
/ a flake / out of scope" is a hypothesis too — probe it before
believing it (a "concurrency, not my change" verdict is only
trustworthy AFTER the mechanism probes + a standalone-vs-trigger
repro prove it).

**Don't claim a workaround you didn't run, either.**  "Bind it to a
local / use the non-X form" is a claim — VERIFY it on current HEAD,
both backends, before stating it (the workaround IS the agent's
primary "can you keep moving?" signal to other consumers).  A WRONG
workaround is worse than "none": it misleads whoever follows it and
burns the trust that is the signal's whole value.  Record the
verified workaround + a `wa:*` label — see
[ISSUE_TRACKING.md § Workarounds](../ISSUE_TRACKING.md#workarounds--the-agents-can-you-keep-moving-signal).

**The ONLY exception** to the agent step: the probe corpus already
pins a SINGLE fix site AND the change is XS — state both in the
commit.  **Probe-first is never skipped; never skip both.**

## Fix-application discipline (REQUIRED)

Fixes land **one cluster per commit, pushed before the next
cluster begins**.  For each cluster: edit → run Set <cluster> +
Set H locally → if both green, commit the fix on its own → push
→ only then start the next cluster.  Bundling two clusters into
one commit, or letting two cluster fixes accumulate locally before
pushing, both violate this rule.

Reasons:

- **Causality.**  A green Set X after the commit says cluster X's
  fix worked.  A green Set X *bundled with* cluster Y's fix says
  one of them worked; the other may be silently broken in a way
  Set X doesn't reveal.  Bundling collapses per-cluster signal.
- **Self-bisect.**  `git diff` between cluster commits is the
  bisect resolution this project actually has (real `git bisect`
  is prohibited; see CLAUDE.md § Debugging policy).  One commit
  per cluster keeps that resolution sharp; bundling collapses it
  to plan-granularity.
- **Rollback granularity.**  When a cluster fix turns out wrong
  (PLAN51 cluster II took 3 attempts to converge), reverting it
  without losing earlier cluster fixes is only possible when each
  fix is its own commit.
- **Set H is the regression canary at the cluster boundary.**
  Running Set H after each commit catches adjacent-system breakage
  (scope handler, codegen, store ops) at the cluster boundary
  where the cause is one commit deep — not after five clusters
  have piled up and the bisect surface is the whole plan.
- **Push for durability.**  Investigation plans span sessions;
  unpushed cluster fixes are one machine failure from gone, and
  the next session can't see the green checkpoint to resume from.
  Pushing also lets the user spot a wrong-cluster fix at the
  boundary instead of after the whole plan lands.  CLAUDE.md
  § Branch policy rule 2 already requires this on long-lived
  working branches.

**When a fix regresses, bisect by SITE — don't re-guess.**  A fix
that touches several sites and breaks something is the
complex-variant hazard turned inward: revert it wholesale and try a
different multi-site guess, and you learn nothing each round.
Instead apply the sites **one at a time** and re-run the matrix —
the site that is a no-op proves it is safe; the site that moves the
regression is the culprit, and *why* it is the culprit usually names
the real fix.  (PLAN51 cluster II took 3 attempts; plan-58's
outer-handle stride took 3 reverted multi-site attempts — then
applying the read-clamp *alone* proved it was a no-op for ≥4 strides,
isolating the storage clamp in one move and pointing straight at the
real parse-time fix.  Wholesale-revert-and-re-guess is the slow path;
one-site-at-a-time is the fast one — reach for it after the *first*
regression, not the third.)

**Exception.**  If an open PR on the branch is mid-review, hold
the next cluster's push until review concludes — CLAUDE.md
§ Branch policy rule 2 prohibits disturbing a branch with an
open PR.  Investigation plans normally run on a long-lived
working branch with no open PR, so this exception is rare; when
it applies, accumulate cluster commits locally and push them as
a batch when the PR closes.

**Anti-pattern to avoid.**  Opening all cluster docs, writing a
unified fix that touches scope handler + codegen + store ops in
one commit, running the full probe sweep, observing green, and
declaring the plan closed.  The sweep will pass; the next
regression will be unattributable; if one of the fix sites was
actually wrong, you've lost the ability to isolate it without
re-deriving the whole plan.  PLAN51's Cluster III "FIXED on
corruption" status while leaks under Cluster II persisted was a
mild version of this failure mode — the lesson generalises.

## Closing an investigation plan (REQUIRED)

Run [`_LIFECYCLE.md`](_LIFECYCLE.md)'s general steps (extract
reference content → trim README → reclassify ROADMAP → set the issue
`status:finished` + close it → repoint links; the dir stays in
place), **plus these two obligations that are specific to
investigation plans** — both must happen BEFORE the issue closes:

1. **File the still-open findings.**  The active-phase no-file rule
   (§ In-plan vs spinoff) inverts at closure: the cluster catalogue
   stops being a live tracker, so every finding that is still open
   needs a forward home.
   - A **deferred / benign residual** of the thesis → QUALITY.md
     `## Open work` (a known tradeoff, not a bug) — note any inert
     foundation already in the tree as a head-start.
   - A **sibling bug** not yet fixed → a **GitHub Issue**
     (`gh issue create`, the `bug_report` template; `sev:` + `area:` +
     a VERIFIED `wa:*` label) — see
     [ISSUE_TRACKING.md](../ISSUE_TRACKING.md).
   - Each cites the plan's cluster / probe doc as the repro
     landmark (the dir stays in place at close).  A finding whose FIX
     shipped needs no row — the fix + its test are the record.

2. **Promote permanent-guarantee probes → CI tests.**  Doc-probes
   under `probes/` are NOT run by CI.  Every probe that encodes a
   correctness GUARANTEE (not characterization / diagnostics) must
   have a CI test before close, so it can never silently regress
   (`tests/scripts/<NN>-*.loft` for cross-backend behaviour,
   `tests/*.rs` for in-process invariants).  Characterization probes
   (watermark counts, bug repros, reference↔problem pairings) stay in
   the plan's `probes/` as landmarks (the dir is not moved at close).
   A probe whose guarantee is already covered by an existing suite
   needs no new test; just record the mapping.

The closure record (the trimmed plan README) should list the
P-issues filed + the probe→test mapping, so a later reader sees
where each open thread went.

## Documentation (REQUIRED)

What this plan must update OUTSIDE itself when it finishes — the plan is not closed until every
line here is done (owner, 2026-10-25): the user docs and the reference pages that run, the
formal chapter whose deviation it closes (its `OPEN:` line), `DIAGNOSTICS.md` for a code,
`CHANGELOG.md`, the bars, the skills (`make skills-review`), the library docs.  One line per
item, each marked with its commit.  [`_TEMPLATE.md`](_TEMPLATE.md) § Documentation has the
usual set.

## See also (REQUIRED)

- Reference doc(s) the investigation touches.
- Other plans on related subsystems.
- Source files central to the investigation.

---

# Per-cluster investigation document template

Each `cluster-<id>-<slug>.md` follows this shape:

```markdown
# Cluster <ID> — <name>

**Severity (split by failure mode):**
- **Corruption / panic / hang:** <user impact, or NONE if leak-only>
- **Leak:** <store count per iter / NONE if no leak>

Track these separately.  "FIXED" must be qualified by which failure
mode is closed (PLAN51's Cluster III was marked FIXED on corruption
closure while leaks persisted under Cluster II — that conflation
caused two false-fix moments in the next session).

**Affected probes:** <list>
**Backend asymmetry:** <which backends fail>

## Mechanism (verified / hypothesized)

<concrete description; trace evidence cited as /tmp/<file>>

## Reference probe — XX (<name>, <status>)

<loft code>

**Lowered IR / behavior**: <key observation>

## Problem probe — YY (<name>, <status>)

<loft code>

**Lowered IR / behavior**: <key observation>

## The divergence

<one-paragraph explanation of what makes Y fail and X pass>

## What we know vs. don't

| | Status |
|---|---|
| <claim> | ✅ Verified via <evidence> / 🟢 Strong hypothesis / 🤔 Plausible / ❌ Unknown |

## Investigation tasks

1. <concrete action — read file:line, run trace, dump artifact>
2. …

## Fix surface

Options ranked by effort and scope.  Each option lists what it
fixes and what it doesn't.

## Fix iterations (REQUIRED when fix attempt count > 1)

A short journal of fix attempts that landed-then-retracted, or
landed-but-needed-follow-up.  Each entry: what was tried, what
assumption it corrected, why it was insufficient.  Two paragraphs
per attempt max.

The commit messages capture each landed change in isolation, but
the *sequence* (and why attempt N didn't suffice) is the load-
bearing context for future investigators who hit a similar shape.
PLAN51's Cluster II took 3 attempts (sentinel-only → free+sentinel
→ narrowed `is_hidden_buf_arg`); without this journal, the third
attempt's predicate would look arbitrary.

Drop this section if the fix landed on the first attempt.

## Why <backend that works> escapes

<contrast: what does the working backend do differently here?>
```

Length budget per cluster doc: **100-300 lines**.  Longer means
historical "failed approach" sections are accumulating — push
closed approaches into a single "Historical: previous attempts"
appendix at the bottom, summarising each in 2-4 lines.  Reference
content that drifts in should extract to the parent reference doc.
PLAN51's cluster-II doc hit 381 lines before pruning; the
appendix pattern would have held it under budget.

---

## Authoring notes (delete from your plan README)

**Length budget**: 100-300 lines for the README + 100-300 per
cluster doc + probe-file headers (typically 20-50 lines each).
A complete investigation plan is 50-80 files total in a deep
case; 10-20 in a shallow case.

**Probe-first discipline**: write probes BEFORE source reading.
The probe suite is the executable spec for what "understood" means.
A mechanism hypothesis is verified when a probe-pair diff confirms
it.  Source reading without a probe to ground it tends to
recursively explore code paths without convergence.

**Liberal probing**: add probes aggressively — missing a crucial
case (like PLAN51's `moros_map` probe surfacing a leak class no
synthetic probe caught) is the worst failure mode.  If an idea
occurs while reading another probe's output, write it; you can
only judge "does this add insight?" AFTER running it.  Variants
confirming known mechanisms don't need test-scripts promotion but
belong in `probes/`.  The cost of a redundant probe is ~10
minutes; the cost of a missed shape is the next session
re-discovering it from scratch.

**Real-library extraction**: include at least one probe extracted
from production code in your subsystem (the canonical loft case:
`gridmesh`, `moros_map`, `audience_crystal`).  Pure synthetic
probes can miss real-world shapes — PLAN51's probe 39 caught a
leak class that no synthetic probe surfaced.

**Verified-vs-hypothesized accountability**: every mechanism
statement in a cluster doc is either VERIFIED (with cited
evidence — trace file path, code line read, etc.) or
HYPOTHESIZED (marked with 🤔).  No middle ground.  When asked
"do we know what's going on?", the table answers honestly.

**Tools-as-needed**: don't write a debugging framework upfront.
Check the existing toolchain (CLAUDE.md inventory) for what's
already available.  Add the ONE tool you need DURING the
investigation, when its absence is blocking.  Other "nice-to-have"
tools should stay as ad-hoc eprintln patches reverted before
commit.

**When NOT to use this template**: if the failure class is
narrow enough that you can pin the mechanism in one source-
read session, just file a P-issue + fix.  Investigation plans
are for failure CLASSES with multiple sub-mechanisms.  PLAN51
qualified (5 clusters, 39 probes, mixed mechanisms); a single
crash bug doesn't.

**On closing an investigation plan**: see [`_LIFECYCLE.md`](_LIFECYCLE.md).
Probe migration to `tests/scripts/` happens as each cluster's
fix lands, NOT all at once.  The plan stays open during
phased implementation; closes when the last cluster's regression
is committed and the cluster doc moves to the relevant reference
doc (e.g. NATIVE.md `## Open work` for native-specific issues).
