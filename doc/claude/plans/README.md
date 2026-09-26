# Plans

Multi-phase initiatives that span more than one session — core language,
compiler/runtime, **and library** work, unified here.

> **Current convention — flat, in `plans/`, numbered to the issue (2026-06-19).**
> A plan is a **single flat file**, `plans/<n>-<slug>.md`, with its phases as
> sections inside that one file — *not* a directory of per-phase files.  `<n>` is
> the plan's [`loft-lang/plans`](https://github.com/loft-lang/plans) issue number
> (`@PLN<n>`): every plan has a gh issue (the cross-ecosystem id), and the local
> file is named to match it.
>
> - **Every new plan lands here in `plans/`, whatever its source** — core,
>   runtime, or library.  There is no separate `lib_plans/` for new work;
>   `lib_plans/` is legacy and being absorbed.
> - **New plan** (e.g. a library scope): create the gh issue first
>   (`gh issue create --repo loft-lang/plans`, with a required `status:*` +
>   `subject:*` label), read its number `n`, then write `plans/<n>-<slug>.md`.
> - **Migrating an existing local plan** (a `lib_plans/` dir, or an un-issued
>   `plans/` dir): if it already maps to a `@PLN` issue, move + renumber to that
>   number; **if it does not exist on gh yet, create the issue, then renumber to
>   it.**
> - The existing numbered **directories** below predate this and remain as
>   legacy closure records.  An investigation plan needing a probe corpus may
>   keep an adjacent `<n>-<slug>/` dir (with `probes/`) — the one exception to
>   flat.

## The rule — docs vs plans

**Documentation about how things work** (architecture, runtime
semantics, data structures, language reference, API surface)
lives at `doc/claude/*.md` — or, when scoped to a single
library, inside that library (e.g. `lib/<name>/README.md`).
This is the reference layer: anyone reading or modifying the
code reads these.

**Future work — things that need to be built** lives in
`plans/` (core language / compiler / runtime) or `lib_plans/`
(library work).  This is the actionable layer: anyone
planning the next session looks here.

These connect through the **roadmap**: every row in
[`../ROADMAP.md`](../ROADMAP.md) eventually points at a plan
in `plans/` or `lib_plans/` (loose features without a plan
home become the exception, not the rule).

**Bug fixes are the explicit exception** — they land directly
via a GitHub Issue + a regression test + a focused commit, no
plan required.  The plan path is reserved for major
development that benefits from explicit phasing, multi-
session sequencing, or design-before-implementation
discipline.

When a `doc/claude/*.md` reference doc has open follow-up
work mixed into the architecture content, add an
`## Open work` section IN THAT REFERENCE DOC and link
ROADMAP rows directly at it (e.g.
[NATIVE.md § Open work](../NATIVE.md#open-work),
[QUALITY.md § Open work](../QUALITY.md#open-work--actionable-summary)).
Single source of truth, no indirection — pointer-plans were
tried (33/35/lib-11) and shown to be over-engineering.

## Three workflows for TODO items — pick the lightest that fits

| Flow | Where the TODO lives | When to use |
|---|---|---|
| **Bug fix** | A focused fix + regression test + commit.  A [**GitHub Issue**](../ISSUE_TRACKING.md) only if you're **not** fixing it now (it blocks you, or it's M+ — see [CLAUDE.md § Bug-filing policy](../../../CLAUDE.md#bug-filing-policy--mandatory)) | Single root cause, fits in one commit, no design choices, no multi-phase sequencing.  **The default** — most bugs are neither a plan nor even an Issue; they're just fixed. |
| **Light — `## Open work` section** | A `## Open work` section in the relevant `doc/claude/<NAME>.md` reference doc | The normal flow.  TODO is co-located with the architecture it touches; one row per item; closure is "remove the row + update the reference content."  Used by NATIVE.md, PERFORMANCE.md, PACKAGES.md, QUALITY.md today. |
| **Plan — a [`loft-lang/plans`](https://github.com/loft-lang/plans) issue (`@PLN<n>`)** | The issue is the plan; its number is the id.  No local slot.  A big multi-phase design may add an optional local dir named for the issue (`plans/<n>-<slug>/`), but the issue stays canonical. | Multi-session initiative with explicit phasing, design-before-implementation discipline, cross-arc dependencies, or a long arc that needs its own document space. |
| **Investigation plan — a `loft-lang/plans` issue + (for big ones) a local `<n>-<slug>/` dir with `probes/` + per-cluster docs** | Probes + cluster docs + verified-vs-hypothesized accountability.  Canonical example: PLAN51 (62 probes, 5 clusters, 12-commit fix arc). | Failure CLASS with multiple sub-mechanisms; needs probe-driven mechanism investigation BEFORE the fix design is clear.  See § When a problem should escalate to an investigation plan below. |

The light flow is the default.  Promote to a plan only when
the work is genuinely multi-phase and benefits from its own
directory — most TODOs don't, even ones that take several
sessions.

### Edge-probe BEFORE fixing — the lightweight default for loft's complex-variant bugs

Distinct from (and lighter than) a full investigation **plan**: before fixing any
non-trivial bug — *especially* one touching a **subsystem intersection** — spend a
few minutes **edge-probing to find the bug's real shape**.  Most loft bugs are
*complex-variant*: they live at the crossings of subsystems (tuple × vector,
parallel × parent-var capture, store-lifetime × block-scope), and those crossings
are **combinatorial** — a bug is a *region* of condition-combinations, not a point.
A single repro is one point in that region.  Fixing from it alone tends to either
(a) close the symptom and miss sibling points, or (b) be **unsound** for points the
repro never exercised (e.g. plan-57's U3: "block-confined ⇒ freeable" held for
every case checked and failed for the one block-result case left unprobed — a
shippable corruption).

So a batch of throwaway `.loft` probes (`--interpret`, vary one condition each)
costs minutes and tells you the real boundary (the tuple-return bug = the *return*
path only; local tuples fine; the `parallel {}` bug = a *write*-to-parent-local
corruption, **not** reads — which P245 already fixed and `tests/scripts/81` guards)
and which edges a naive fix would corrupt.  **Keep the probes** — they become
permanent regression landmarks
(`plans/2-vector-store-watermark/probes/`).  The *fix* still happens (don't
file — see [CLAUDE.md § Bug-filing policy](../../../CLAUDE.md#bug-filing-policy--mandatory));
you just characterise the region first.  This lowers the bar to probe-first vs
fix-from-one-repro.

*Why clear them at all, not just the reported one:* a left bug is a **veil** — it
blinds you downstream and can make broken things look fine (a `--native`
`parallel{}` no-op once made test-80/81 *pass*).  Clearing bugs is the
precondition for verifying the model holds anywhere — see
[GOALS.md § Bugs hide things](../GOALS.md#bugs-hide-things--clear-them-before-trusting-the-model).

**Inside an investigation plan this stops being the lightweight default and
becomes a hard RULE — and it adds a code-only investigation-agent step** (the
bugs there are hard by definition: multi-subsystem, non-obvious fix surface).
The sequence is **probe → code-only investigation agent → fix → verify against
the probe corpus**, never an ad-hoc fix from a first read.  Full rule:
[`_INVESTIGATION_TEMPLATE.md § Fixing a finding`](_INVESTIGATION_TEMPLATE.md#fixing-a-finding--probe--agent-before-code-required).

### The matrix is how you *see* the root — and the proportionate fix is the invariant

Edge-probing isn't only for *scoping* a fix; the completed probe matrix is what lets
you **see the general picture** — the one mechanism behind a family of "different"
bugs — that you would otherwise overlook.  The pattern is reliable: the unifying
root becomes visible *after* the matrix exists and gets missed *before* it.  In
plan-58 the `single` SIGSEGV, `boolean` overlap, `i32` corruption, and comprehension
skew were **one** `vector<T>`↔`vector<vector<T>>` handle-stride conflation — obvious
in the 34-cell matrix, invisible in any single repro, and "fixed" three different
wrong ways while the matrix was still incomplete.  So treat **"I can't see the root
yet" as "the matrix isn't finished," never as license to patch the one case in
hand.**  (Some readers reach the pattern by intuition with no data; if you don't,
the matrix is your eyes — build it first, then the root is as visible to you as it
was to them.)

And the fix must be **proportionate to the problem, not the symptom.**  A multi-line
fix special-cased to one *type*, in a language whose types are near-identical, is a
shape-mismatch — it signals you are patching downstream of the shared chokepoint
every type flows through.  Find the **invariant** the whole class violates and
enforce *exactly* that: no **narrower** (a per-type patch leaves siblings broken)
and no **wider** (re-resolving the type drags blast radius — plan-58's `db_type`
attempt was the right *unify* instinct aimed at the wrong dimension; it should have
enforced the stride invariant, not re-derived the type).  An **un-generalized
remainder** (plan-58's accepted ≥4 over-reservation) is the *same bug, unfinished* —
"benign" is making peace with a fix proportionate to one type, not to the problem.
The sanitizer **guard** worth building is just that invariant made executable
(*every vector handle strides by 4*) — see [GOALS.md](../GOALS.md) two-engine model.

### The composition axes — the dimensions a matrix varies

A matrix is only as good as the axes it crosses, and the axes are **not** invented
per bug or per feature — they are loft's **fixed degrees of freedom**, the things any
new value, type, or operation must compose with.  This is the one canonical list;
the **debug** matrix ([CLAUDE.md § Debugging policy](../../../CLAUDE.md) step 2, to
*bound a bug*) and the **feature-plan Stage A** ([`_TEMPLATE.md`](_TEMPLATE.md), to
*bound a feature*) both vary against it.  Cross the axes your change actually
touches; a weak matrix that exercises the easy axes and skips the load-bearing one
(plan-58 ran wide elements and skipped narrow) hides exactly the cell that crashes.

1. **Type-kind** of the element/operand — wide scalar (`i32`/`f32`/`bool`/`char`),
   **narrow scalar** (`i8`/`i16`/`u8`/`u16` — distinct stride, where plan-58 split),
   `text`, struct, enum, vector, tuple, closure / fn-ref.  Plus the **null** of each.
2. **Construction path** — literal, comprehension, function return, append/push,
   copy (assignment / element-copy), default / zero-init.  (plan-58's four clusters
   were *one* bug spread across four of these.)
3. **Storage context** — local slot, struct field, vector element, global,
   const/static, argument, captured-by-closure.
4. **Access** — read vs store, *every* index/position (not just `[0]`), length
   afterwards — a probe that checks only `[0]` with no length passes *on* corruption.
5. **Nesting depth** — 1 / 2 / 3+ (where the `vector<T>`↔`vector<vector<T>>` type-id
   divergence surfaced).
6. **Null / sentinel** — the null representation *per type* (`f32` NaN → wild rec-id
   vs `i64::MIN` → harmless; the @P380 axis).
7. **Backend** — `--interpret` vs `--native` (vs WASM where relevant); divergence
   between the two is its own hazard, verified at the end (step 7).
8. **Transform / accumulation** — when the fault is in the *compiler pipeline*, not
   the value, axes 1–7 (each the shape of one value) never reach it.  Vary instead:
   **cardinality** (N of a construct *coexisting* — e.g. many `par` loops in one fn,
   where per-loop slot/store assignment collides only past a threshold — the #282
   class); **ordering / position** (the same construct mid-body vs at the tail); and
   **cross-pass consistency** (the two-pass parser: a value numbered or typed
   differently on pass 1 vs pass 2 — the result-var counter-desync class).  Tell:
   if the minimal repro *won't shrink* below N coexisting items, the coexistence
   **is** the bug — stop hunting a one-instance repro.

The **design** use is this same list run *forward in time*.  Before building a
feature, its cells against these axes are its conformance spec: it is done not when
the demo runs but when **every cell is green on both backends**, and those probes
become its regression suite.  The bug class plan-58 shipped — one invariant
(*a handle is 4 bytes*) re-derived in four code paths, validated only where the
derivations happened to coincide — is *structurally* a set of matrix cells the
feature never crossed.  The representation that makes the class **impossible** is
[Goal E](../GOALS.md) (source is the truth) applied to construction: give each fact
the feature introduces **one home** every path consults, and the off-diagonal cells
have nothing to disagree about.

### Sibling bugs are discoveries to *record*, not cases to *fix in-place*

Edge-probing one bug routinely surfaces **other** bugs at adjacent crossings —
that's a *good* sign (the probing is working), and it will happen in every
investigation.  plan-57's store-lifetime probing surfaced both the tuple-return
crash *and* the `parallel {}` capture bug.  The trap is to then fix the sibling
**on the spot, inside the current investigation**, because it looks simple.  Don't.

**Why it's a problem** (steerable — not a disaster, just a thing to catch early):

1. **The sibling gets a fix-grade decision on discovery-grade evidence.**  It
   skips the very probe-first rigor the investigation exists to enforce.  Concrete:
   the `parallel {}` bug was handed a confident verdict ("reject *any*
   enclosing-scope reference, zero blast radius") off a coarse 4-row table — and a
   test *already in the repo* (`tests/scripts/81`, the P245 guard) disproved it
   (parent-var **reads** are legal and tested; only **writes** corrupt).  A fix
   built on that verdict would have broken a passing test.  That is the exact
   "fix-from-one-repro under-fixes a complex-variant bug" trap the section above
   warns about — re-entered, ironically, *during* a disciplined investigation.
2. **It contaminates the investigation's record and its landmark set.**  An
   investigation is a clean account of *one* thesis (plan-57 = store lifetime /
   [Goal E](../GOALS.md)).  Bolt an unrelated bug's fix into it and the probes stop
   telling you which landmarks are thoroughly mapped and which are half-probed
   imports — the ledger lies about its own reliability.

**The steer:** when a sibling bug surfaces, **record the discovery** (one note:
shape + minimal repro + "investigated separately") and give it its **own** scoped
edge-probe before any fix — same rigor, separate ledger.  This is just the
[DEVELOPMENT.md route-to-canonical-home rule](../DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan)
applied to investigations: a discovered bug that is not part of the active thesis
goes to its own home unless it **shares a fix site** with the bug you're on.  The
discovery note stays in the investigation's log (faithful record of what the
probing found); the *case* and the *fix* live in their own scope.

This whole discipline is not a separate process rule — it is
[Goal E](../GOALS.md#the-method-mirrors-the-goals) applied to our own reasoning:
the stated model (the investigation's thesis, a bug's verdict) must match the
verified reality, and a divergence is fixed by removing the gloss, not asserting
past it.  We hold our own claims to the exceptionless-transparency standard loft
holds its memory model to — because a team that tolerates hidden machinery in its
reasoning cannot credibly ship a language whose promise is no-hidden-machinery.

### Preserve a failed/partial attempt as a diff + hash — don't revert it to a summary

When an attempt **works partially or fails informatively**, the instinct to
`git`-revert it is a trap: it discards the one artifact that teaches — the actual
code, whose behaviour under the tracer you can re-examine — and leaves only a
degraded prose summary.  This plan's own history is the proof: two cluster-I/III
reverts came back as *misdiagnoses* (rc, then "timing") precisely because the work
was thrown away and re-derived from a summary.  **We learn from failures; we do
not hide them.**

So instead of reverting, **preserve the attempt as a diff pinned to the exact
build hash, inside the plan** (`experiments/<name>.diff` + a `.md` with
`git checkout <hash> && git apply <diff>`, what it does, and the *real* lessons —
see [`57-vector-store-watermark/experiments/`](2-vector-store-watermark/experiments)).
A future session re-applies it verbatim and studies the live behaviour rather than
re-deriving from a degraded copy.  Don't ship a known-broken or oracle-fooling
state to `main`; the diff is the durable, re-appliable record, and the working
code can stay uncommitted in-tree to continue from.

### Grade the signal before you trust the result

Hastiness is not impatience — it is **mis-calibrated trust**: grading a
measurement's *result* without grading the *signal*.  A signal lies in three
distinct ways, each needing its own check: it measures the **wrong thing** (a
proxy, not the property), it is **confounded** (buffering or mixed streams reorder
it), or it is **stale** (read off a state that has since changed).  Before any
conclusion or action: *is this signal reliable, and is it current?*

Two corollaries:

- **A fooleable oracle is a liability, not a safety net.**  For any test or guard,
  ask "can this pass while the thing it is meant to catch is broken?"  If yes, it
  is not an oracle — it manufactures false confidence.
- **The loop is the work; the fix is its byproduct.**  A session that preserves and
  reliably measures attempts, rules wrong explanations out, and *locates* the
  constraint has succeeded even before code lands — the ruled-out ledger is the
  durable asset, and the fix falls out of it.

### When a problem should escalate to an investigation plan

Just *fixing* the bug (focused fix + regression + commit, after the edge-probe
above) handles the overwhelming majority of bug reports — a plan is only for when
the **scope is genuinely hard to pin down** (you can't yet write the fix because
you don't know what you're dealing with).  Once scope + root cause are pinned,
there's nothing to investigate: fix it.  Escalate to an investigation plan ONLY
when one of these signals fires.  Each signal carries an action.

The escalation triggers below all serve one underlying goal: making
loft **stable** as a class, not just closing the reported shape.
Memory-management bugs (closure capture, dep tracking, store
ownership) and toolchain shifts (new rustc, new clippy) both
routinely leave siblings behind when fixed one-at-a-time.  See
[`_INVESTIGATION_TEMPLATE.md` § Primary goal](_INVESTIGATION_TEMPLATE.md#primary-goal-loft-stability-not-single-bug-closure)
for the full framing.

#### Early signals (before opening the fix branch)

| Signal | Action |
|---|---|
| Bug description names a SHAPE, not a function ("X returning Y in a Z-typed context", versus "fn `foo` crashes on null") | Stay P-issue; add the shape as a precondition note on the row.  Watch for siblings. |
| Recent commits in the same subsystem closed similar shapes (`git log --oneline src/<area>` shows ≥2 cluster-fixes in last 30d) | Strong signal — escalate.  The next bug is the 3rd of a class; the catalogue starts paying back. |
| The mechanism explanation requires ≥3 interacting code paths to articulate | Escalate.  One PROBLEMS.md row can't carry the cross-references; the investigation plan's per-cluster docs do. |

#### Mid-fix signals (after attempt #1)

| Signal | Action |
|---|---|
| Fix passes the original repro but regresses OTHER tests | Escalate **immediately** — mechanism isn't pinned. |
| Fix would force `#[ignore]` on another existing test to ship in isolation | Escalate — the two tests are the same class.  **This is the highest-confidence single signal.** |
| Investigation agent reports "multiple sub-mechanisms" or "couldn't pin a single root cause" | Escalate; the agent has done the cataloguing work for you. |

#### Late-fix signals (after attempts #2 + #3)

| Signal | Action |
|---|---|
| A "way-forward matrix" with ≥3 paths and no clear winner | Escalate now; you're choosing fix design without a mechanism understanding. |
| Fix-attempt N corrupts something DIFFERENT from fix-attempt N-1 | Escalate; symptom-chasing without convergence. |
| Effort estimate exceeds 1 week | Escalate; the work is multi-session by definition. |

#### Cost calibration

A P-issue costs ~30 seconds to file.  An investigation plan costs
~1-2 days to set up (template + probes + initial mechanism
notes).  The break-even is roughly the **3rd fix attempt** — if
you've burned 3 sessions on shape-by-shape fixes, the investigation
cost has already been paid in lost work.

#### One-line decision rule

> **"Would I need to `#[ignore]` an existing test to ship this fix?"**
> If yes, it's not a P-issue — it's a failure class.  Open the
> investigation plan, catalogue the shapes, and ship a fix that
> closes all of them together.

That single rule would have triggered PLAN51 about a week earlier
than it actually opened (the @P377 fix attempts carried
`#[ignore]` on tests 139/140 for weeks before PLAN51 emerged).
The investigation-plan shape adds friction up front; the rule
above is what makes that friction trip at the right moment.

## Roadmap workflow

[`../ROADMAP.md`](../ROADMAP.md) is the work-list view: every
open work item, grouped by value, with dependencies and
effort.  This section documents how it's organized — the
methodology lives here so ROADMAP itself stays tight.

### Value categories — what KIND of value, not just how much

ROADMAP rows are grouped by **value category**, not by release
milestone (0.8.5 / 0.8.6 / etc.).  Eight categories in default
reading order (read top to bottom; pick from the highest tier
that has open work):

| Tag | Meaning | Examples |
|---|---|---|
| **S** | **Silent failure / data-loss prevention** — features that "appear to work" but don't; corruptions that have no error message; data loss without indication.  HIGHEST priority because invisible to users and erodes trust most | Validation matrices (catch backend divergence), JSON-correctness sweeps, closure-DbRef leak, native-vs-interpreter parity gates |
| **R** | **Regression / release-blocker** — known broken behavior, PROBLEMS.md High-severity, gates the next tag | (none today; bugs land as P-issues, not plans — would surface here if a regression blocked a release) |
| **G** | **Goal-enabling** — directly enables loft's core use case: browser games anyone can play via shared link, multiplayer, native-game debugging | Server / game-client libraries, scriptable scenes, multiplayer protocol stack, web IDE, graphics library, native debug |
| **F** | **Foundation** — unblocks 2+ downstream plans (lattice point in the dependency graph) | Lazy stdlib, package registry, FFI generic marshaller, LSP server MVP, library extraction |
| **U** | **Ease of use** — first-time-user experience, daily ergonomics, IDE polish | Better error messages, REPL, syntax highlighting, VS Code extension, tutorial, LSP editing surface, developer warnings |
| **C** | **Clean features** — language correctness, removes special cases | Mutable closures (cleaner capture), match-PEG (cleaner pattern syntax), sorted slicing (removes special-case), JsonValue (replaces text-based JSON), error recovery, factory methods |
| **Q** | **Internal quality** — performance, refactor, internal cleanup with clear payoff | Performance follow-ups, native codegen follow-ups, retire-scratch, const-store completion |
| **N** | **Niche / opportunistic** — small specific features, low-priority items | Route decorators, asset pipeline, HTTP client, tic-tac-toe (protocol-only validator), AOT auto-compile |

**Why S is its own category, separate from R:**

A regression (R) is a **known broken** behavior — we have a
test failure, a panic, or a wrong output we can document and
gate releases on.  A silent failure (S) is **invisibly broken**
— the program runs to completion, returns "successfully," but
the result is wrong / data is lost / state is corrupted.

S is more urgent than R because:
- Users hit S without warning (no error message to file)
- Data loss without indication is the worst class of bug
- "Appears to work" hides decay in the codebase
- Trust erodes faster from one silent corruption than from a
  loud crash

The validation matrices (plans 14 / 15 / 16 / 18 / 19 / 20)
are precisely S-prevention work: every {input × backend × API
shape} cell is exercised end-to-end with byte-identical
output assertions.  They're cataloged here as S, not C, because
their value is preventing silent divergence.

**Why named categories instead of V1/V2/V3:**

The previous V1/V2/V3 collapsed two distinct dimensions —
"importance" and "what kind of work" — into one ranking.
Named categories separate them: a plan in **G** (goal-enabling)
ranks above a plan in **U** (ease-of-use) because of WHAT the
work delivers, not just because someone declared it more
important.  Re-ranking happens when categories change (rare),
not when calendar perception shifts.

**Why this works better than time-based grouping:** value
categories stay stable across the project's life.  A goal-
enabling item is goal-enabling whether it ships this month
or next year.  Milestone groupings (0.8.5, 0.8.6, ...) imply
calendar timelines that constantly drift; named categories
don't need that maintenance.

**Effort estimates, not time projections.**  ROADMAP has an
`E` column (XS / S / M / MH / H / VH / L) calibrated to
relative work, not calendar time.  Time projections are
inaccurate at both ends — items projected as "weeks" have
shipped in days; items projected as "quick" have taken
weeks.  Effort buckets are stable; calendar projections
aren't, so we don't make them.

### Maintenance rule

When an item completes, **remove it from ROADMAP**.  Completed
work belongs in CHANGELOG.md (user-facing notes) +
CHANGELOG_TECHNICAL.md (contributor detail) + git history.
Keeping closed rows in ROADMAP turns the work list into a
log; we already have logs.

### Features need plans

Every feature row in ROADMAP should cite a plan in its `Source`
column — or be small enough for direct PROBLEMS.md + commit
(the bug-fix path).

The plan-cadence rule: any FEATURE (multi-step, has design
choices, touches multiple files) goes through the plan
path even if small.  Small bug fixes, deliverables (demo
deploys, single-action items), and operational changes
(CI tweaks, doc fixes) can stay on ROADMAP without plans
or land directly with no ROADMAP row at all.

When a feature row still cites a flat reference doc
(PLANNING.md, INTERFACES.md) rather than a plan, that's a
**plan promotion candidate**.  Promote it to a plan when it
surfaces as next-up work.

### How to read the roadmap

The work-table sections (V1 / V2 / V3) are the **what to
do next**.  Within each value tier, items are loosely
grouped by theme — pick whichever theme is closest to what
you're touching.

The "All open plans — index by value" section is the
**comprehensive view across both trackers** (`plans/` +
`lib_plans/` + their deferred subdirs).  Single place to
read for "what's open, what depends on what, how valuable."

The "Cross-tracker dependency chains" subsection captures
the cross-plan arrows: which plan unblocks which.  Useful
for picking next-up work.

The "Features still needing plan promotion" subsection
tracks ROADMAP rows that should have plans but don't yet —
the next-up promotion candidates.

### Closing a plan — documentation must move out

When a plan ships (issue → `status:finished`, then closed — the dir stays in
place, see [`_LIFECYCLE.md`](_LIFECYCLE.md)), its **documentation content must
move to its proper home** in the reference layer:

- Library-scoped reference content → `lib/<name>/README.md`
  (or other library-internal docs).
- Project-wide reference content → `doc/claude/*.md`
  (architecture, runtime semantics, language / API surface).

**A closed plan's README is a closure record only** — git history pointer,
commit chain, P-issues filed/closed, lessons learned.  It is NOT a place that
other docs link to for ongoing reference.

**Why this matters:** retaining links to closed
plans across the docs creates drift.  A future contributor
clicking through to a closed plan finds a closure record
where they expected design content; the actual design has
moved elsewhere or the design has been superseded by the
implementation itself.  Links to closed plans rot fastest
because nothing in the project keeps them honest.

**How to apply on close:** see
[`_LIFECYCLE.md`](_LIFECYCLE.md) for the 6-step procedure shared
by close + defer.  Two shapes: **CREATE-AND-MOVE**
(`31-html-export → HTML_EXPORT.md`) and **TRIM-ONLY**
(`04-slot-assignment-redesign → SLOTS.md`).

This rule applies retroactively: a closed plan being linked
from current docs is a cleanup signal — either the reference
content never moved out, or the link wasn't updated when it did.
`scripts/check_doc_drift.sh` catches the common shapes
automatically.

### Authoring a new plan

Open an issue in [`loft-lang/plans`](https://github.com/loft-lang/plans).  The
issue **is** the plan; its number is the canonical id, referenced as `@PLN<n>`
(see [CLAUDE.md § Tracker tags](../../../CLAUDE.md)).  There is **no local plan
slot** — small plans live entirely in the issue.

Use [`_TEMPLATE.md`](_TEMPLATE.md) for the issue body's shape: Status / Goal /
Effort / Sub-arcs / Phase ordering / Open questions / Cross-arc dependencies /
See also.  A **big multi-phase design** that needs its own document space may add
a local file named for the issue — `plans/<n>-<slug>.md`, or `plans/<n>-<slug>/README.md`
when it has companion files — but the issue stays the canonical home and the file is
optional.  Length budget: 100-300 lines;
longer content belongs in `doc/claude/*.md`.

(Existing `plans/<NN>-<slug>/` dirs predate this and are kept as-is; only *new*
plans skip the slot.)

### Deferring a plan

When some/all phases can't reach completion in the current arc
but the remaining work has a **concrete trigger**, move to
`deferred/`.  Procedure in [`_LIFECYCLE.md`](_LIFECYCLE.md) —
shared with closing (Steps 4-6 are identical).

Partial defers (some phases shipped, others paused) grow a
SHIPPED / DEFERRED Status table at the top of the plan README.
Canonical shapes: @PLN82, @PLN80.

If remaining phases have no concrete trigger, the design moves to
[`../DESIGN_DECISIONS.md`](../DESIGN_DECISIONS.md), not
`deferred/`.  "Will get to it later" is not a trigger.

### Light flow lifecycle — `## Open work` in reference docs

The light flow follows the same lifecycle as plans, just with a
single row instead of a directory.

**Open** — add a row to the relevant reference doc's
`## Open work` table (create the section if it doesn't exist;
see NATIVE.md / PERFORMANCE.md / PACKAGES.md / QUALITY.md for
canonical shape).  Add a ROADMAP row tagged with value category
+ link directly at the section.

**Work** — when implementing, edit the reference doc's
architecture content directly (it's the same file).  Cross-link
between the Open work row and the section it touches if useful.

**Close** — when shipped:
- Remove the row from `## Open work`.
- Update the surrounding architecture content to reflect the new
  reality (the same edit that closed the work usually does this).
- Remove the ROADMAP row.
- Closure record lives in the commit message + CHANGELOG_TECHNICAL.md;
  no separate closure-record file.

**Defer** — if work is paused with a concrete trigger:
- Annotate the row inline (e.g. `**Blocked on X** — unpauses
  when Y happens`) or move the trigger detail to
  [`DEFERRED.md`](DEFERRED.md) if it's cross-cutting.
- Keep the row in `## Open work` (it's still tracked work, just
  paused).

**Promote to plan** — if the row grows into a multi-phase
initiative, copy `_TEMPLATE.md` and migrate.  Don't promote
prematurely; multi-row clusters often stay light if each row is
independent.

The recent collapses of pointer-plans 33 / 34 / 35 / lib-11 into
NATIVE.md / PERFORMANCE.md / QUALITY.md / PACKAGES.md `## Open
work` sections are the canonical examples.

## Companion indexes — every parked item is discoverable

Two files complement this README; together they ensure deferred
work is never silently dropped.

- **[`DEFERRED.md`](DEFERRED.md)** — trigger index for the
  `plans/deferred/` plans only.  Each row gives the concrete signal
  that should re-activate the plan.  Future plans (`plans/future/`)
  live on [`../ROADMAP.md`](../ROADMAP.md), not here — they're
  paused with intent to finish, not awaiting a trigger.
- **[`../USER_FACING.md`](../USER_FACING.md)** — the subset of
  DEFERRED.md that downstream users would notice if shipped, with
  release-note language, workarounds, and severity tiers
  (S0 / S1 / S2 / S3).  S0 items are release-blocking; S1 items
  must ship within two releases of being filed.

**Pre-release ritual:**

```bash
# 1. Every parked test (lock-in regression net):
cargo test --release -- --ignored 2>&1 | grep "^test " | head -50
#    Items now passing → un-ignore + add release note.

# 2. Every parked doc trigger:
grep -r "Trigger to unpause:" doc/claude/
#    Walk the list, refresh `Last reviewed:` lines.

# 3. USER_FACING.md status pass:
#    Every open row gets shipped / still-deferred / dropped tag.
```

### Closed-work hygiene rule

DEFERRED.md and USER_FACING.md are **open-queue documents**.
When an item closes, its row is **removed entirely** — closed
work lives in the right place for its shape: git history (commit
message), regression tests, plan READMEs (architectural lesson),
PROBLEMS.md (closed P-id entries stay for cross-reference),
CHANGELOG.md (user-facing notes).  Five homes, each correct for
its information shape.

The grep target `grep -r "Trigger to unpause:" doc/claude/` should
always show only currently-actionable items.

**Sole exception**: USER_FACING.md's "Closed-by-decision" section
is a permanent record of explicit non-goals so future contributors
find the decision before re-proposing.

### Validation-first default

Default discipline: finish validation plans before shipping new
feature work.  When validation matrices stop yielding bugs (a
phase closes with 0-1 P-issues across 5+ cells), the matrix is
mature — promote that bandwidth to feature / showcase work.
Override only when USER_FACING.md surfaces an S0 item or an S1
item that's been deferred for two releases.

## Conventions

- A new plan opens as a [`loft-lang/plans`](https://github.com/loft-lang/plans)
  issue (`@PLN<n>`) — no local slot.  A big multi-phase design may add an
  optional local file named for the issue (`<n>-slug.md` from `_TEMPLATE.md`, or a
  `<n>-slug/` dir with a README and `00-<first-phase>.md` when it has companions);
  small plans live in the issue alone.
- Existing `NN-slug/` dirs predate this (number = monotonic open-order, not
  priority) and are kept as-is.
- Phase files begin with `Status: open | in-progress | done`.
- Closing a plan: set the issue `status:finished` and close it; the local dir
  stays in place as a closure record (apply [`_LIFECYCLE.md`](_LIFECYCLE.md)).
- Paused-with-trigger plans (**never started**): set the issue `status:future` (keep it
  open) + add a row to [`DEFERRED.md`](DEFERRED.md) (apply [`_LIFECYCLE.md`](_LIFECYCLE.md)).
- Paused-with-trigger plans that **shipped a floor** (some phases delivered, the rest
  deferred pending a driver consumer): set the issue `status:parked` — the accurate state
  between `status:active` (in progress) and `status:future` (not started). The plan README's
  `## Status` block carries the phase-by-phase state + the re-activation trigger. (e.g. @PLN43
  store-durability: Tier 1 shipped, Tiers 2/3 parked pending game consumers; @PLN82 const-store
  is a candidate for the same.)

## Ground rule — plans never allow regressions

**A plan's job is to split work into manageable chunks that can
each land cleanly without introducing new problems.**  That is the
entire point of a plan vs. an ad-hoc fix.  Every phase, and every
step within a phase, must:

- Preserve every currently-green test across the full suite.
- Preserve every currently-correct user-facing behaviour.
- Either ship a new invariant or be a no-op refactor — never a
  degrade-now-fix-later bargain.

When a step surfaces a scope surprise (e.g. a prerequisite was
wrong, a shared code path breaks under the new invariant, a
previously-undocumented consumer exists), the plan document is
updated BEFORE the next commit lands.  The chunks may shrink, a
new sub-phase may be added, or the initiative may pause until the
surprise is understood — **but no regression ships as "we'll fix
it in the next phase"**.

Single-commit fixes outside a plan may exceptionally trade a
regression for a critical fix (documented explicitly in the commit
message).  Plans never — their entire raison d'être is the
discipline of no-regression progress.

Corollary: when a plan's acceptance criteria lists a condition like
"full test suite green" before proceeding, that condition is
binding.  A step that violates it gets reverted (not amended) and
the plan is re-scoped.  The 2026-04-21 P184 Phase 0 attempt (bulk
4-tuple extension, then reverted when test failures surfaced) is
the canonical example of this discipline in action.

## Ground rule — file pre-existing bugs surfaced during a phase

A plan phase fixing one bug or implementing one cell routinely
surfaces *other* bugs while probing variants, reading code, or
comparing backends — sibling shapes, latent issues flagged in
comments, symptoms unrelated to the active fix.

**File those P-issues before the phase closes, not later.**  See
[CLAUDE.md § Bug-filing policy](../../../CLAUDE.md#bug-filing-policy--mandatory)
and [DEVELOPMENT.md § Bug-filing During a Hunt](../DEVELOPMENT.md#bug-filing-during-a-hunt--mandatory)
for the full policy.  Plans-specific notes:

- The phase's commit message lists every P-id filed and every
  P-id closed in this phase.
- New P-issue rows in [PROBLEMS.md](../PROBLEMS.md) are part of
  the phase's deliverable, not a follow-up TODO.
- Follow-ups belong to their own future phase or session — do not
  scope-creep the active fix to "while I'm here, also fix X".  One
  fix per commit; one follow-up per row.

The May 2026 @P211 hunt is the canonical example: the original
P-issue was native `yield text`, but the diagnostic probes
surfaced @P217 (text accumulator), @P218 (format-with-param in
generator body), @P219 (vector-for-yield), @P220 (`""` in
`vector<text>`) and @P221 (server-side BufReader).  All five were
filed in the same commit window; none were lost.  The @P217
follow-up hunt then surfaced @P222 / @P223 (narrower self-concat
shapes) — same rule applied.

### Per-plan status lives with the plan — not on ROADMAP

A plan's lifecycle state (active, next, future, parked, finished) is its issue's `status:*`
label; its per-phase detail is the Status block of its file, when it has one.

ROADMAP's "All open plans — index by category" tables carry only
the stable parts of each plan: name, remaining effort (E),
dependencies, and a one-line "what is this plan about" descriptor.
Per-phase status (what's shipped, what's in flight, what's
blocked) lives in the plan file's Status block — that's the
single source of truth.

Why: per-phase status changes every time a phase ships or is
deferred.  Mirroring it on ROADMAP doubled the edit cost and
created a recurring drift surface (manual audit 2026-05-09 caught
@PLAN14 phase 08 deferral missing, @PLN28 phase 1 status stale).
ROADMAP's job is "which plans exist + how big + what blocks them";
the plan README's job is "where it stands today."

## Plan sets — where four plans are one piece of work

Most plans stand alone. When several are cut from one design, the set needs an entry point, or
a reader meets phase `F7a` with no idea why it exists. One set today:

### The 2-D game stack — @PLN144 · @PLN145 · @PLN146 · @PLN147

**What it is for.** `graphics` ships a complete immediate-mode GL surface and nothing above it,
so every game re-implements the scene graph, the text field and the widgets by hand —
`tools/brick-buster/25-brick-buster.loft` is **1983 lines for a Breakout clone**. These four
plans build the layer a game author writes against.

**Scope: 2-D games at any scale — not a 3-D engine.** The 2.5-D half is a *sprite presentation*
of a 3-D world (a hex or grid footprint, sprites standing up from it). Lighting, fog and
background blur are in; meshes and camera projection are not. Targets: interpreter, `--native`,
`--html`, `--native-android`. Full 3-D and broad standards integration are deliberate
non-goals *for these four* — see @PLN144 § Goal.

| Plan | Arcs | Phases | One line |
|---|---|---|---|
| **[@PLN144](144-2d-stage/README.md)** the 2-D stage ✅ **SHIPPED** | A scene ✅ · P presentation ✅ · L light ✅ · G paths *(deferred)* | 17 | A retained **flat** node array batched by a *merge-adjacent, never-reorder* rule, presenting a 3-D world through three knobs — sprite **origin**, `layer` + `depth`, projected position |
| **[@PLN145](145-authoring-libs/README.md)** text, tweens, widgets ✅ **SHIPPED** | B text ✅ · C tweens ✅ · D widgets ✅ | 11 | What you write a game *with*: a font that works headless, text that costs no upload, property tweens, and a widget kit that turned out to be **someone else's to publish** rather than ours to extract |
| **[@PLN146](146-content-delivery/README.md)** content + delivery ✅ **SHIPPED** | E audio ✅ · F assets ✅ · W drawing ✅ | 18 | The pack **is a loft store** on a dumb file server, range-read; plus sprites authored in loft rather than Python, and browser audio that loops, pans and seeks where it used to be a stub |
| **[@PLN147](147-content-editor/README.md)** the editor | S T U V · X sprites | 16 | An in-browser editor whose invariant is that **it edits the same store the game loads**, so editor↔runtime agreement is structural; arc X adds sprite + animation editing |

**Why four and not one.** It was one plan of 40 phases until 2026-08-19, and the test that split
it is whether phases can **fail together**. @PLN144's arcs share a gate family — pixels, batch
counts, upload counts — so a regression in `A3`'s upload path reddens `P2` and `P4`, which is
how both of those findings surfaced. @PLN146's gates are a byte-range log and a headless-Chrome
audio handle; neither can redden the other or the stage.  (Its phase count is **18**, not the
19 this table carried — a miscount the plan's own tables never had.) Phases that cannot fail together are a
programme, not a plan.

**Where to start — @PLN144, @PLN145 and @PLN146 are all closed, so the set's open half is
@PLN147** (`A0`, `B0` and `D0b` were the other no-dependency phases and have all shipped).
@PLN147's `X1` extends @PLN146 `W2`'s oracle gate across targets and its `X5` bakes through
`W6`'s scene-to-pack route, so arc W is the half to read first.

⚠ **`D0b` is the one to read before starting either.** It was the set's cheapest phase — one
probe program — and it did not do what it was cut to do: it was scoped on *"three consumers
wrote their own input layer"*, the true count was **one**, and instead of retiring the `input`
dependency it retired the *reason to doubt it* and returned a named structural limit
(a modifier is not expressible) that `D1` and `D2` could then plan around. Two of the three
miscounted rows came from judging a file by its **name**. It is the set's standing argument for
spending the first probe on the **premise** rather than on the task.

**The through-lines**, which is what makes the set readable as one piece:

1. **The world is 3-D; the view is a presentation of it.** The stage learns nothing about hexes
   or 3-D, and co-op replicates the *world* rather than the scene for the same reason.
2. **Never reorder.** 2-D correctness is painter's algebra, so the batcher merges only adjacent
   runs — which is why the *packer* decides the batch count, and why atlas assignment is a
   content decision rather than a renderer one.
3. **Bake at pack time, not at run time.** Blur, premultiplication, collision proxies, animation
   frames: the runtime keeps knowing nothing about how the art was made.
4. **Prefer a gate that already exists.** `A4`'s pick is `T2`'s gate; `P1`'s occlusion table is
   `T3`'s; `W2`'s Python oracle is `X1`'s. One fact checked by two consumers cannot drift.
5. **Probe an unadopted dependency before depending on it.** `input` and `shapes` are published
   with no consumers, so `D0b` and `F7a` ask why for the cost of a compile.

**Shared companions** live with @PLN144 because it is the oldest, not because it owns them:
[`PRIOR_ART.md`](144-2d-stage/PRIOR_ART.md) (what moros, dryopea, crawler, hexbody and crew_punk
already built, and the library-integration audit), [`RENDERER.md`](144-2d-stage/RENDERER.md)
(doctrine inherited from crawler's orphaned renderer design), and
[`PRESENTATION.md`](144-2d-stage/PRESENTATION.md).

**Not in these plans, on purpose:** co-op lives in
[`lib_plans/64-game-client`](../lib_plans/64-game-client/README.md); the sandbox boundary is a
rule in [LIBRARY_AUTHORING.md](../LIBRARY_AUTHORING.md) § 2d, because it is a property of an API
rather than a phase.

## Where to look for plans by state

The **`loft-lang/plans` issue's `status:*` label is the source of truth** —
duplicate per-state tables in this README rotted faster than they helped, and
directories are no longer moved between states.  Query by label:

```bash
gh issue list --repo loft-lang/plans --label status:active    # in progress now
gh issue list --repo loft-lang/plans --label status:parked    # floor shipped, remaining phases paused (trigger in the plan README)
gh issue list --repo loft-lang/plans --label status:future    # not started; planned / paused (trigger in DEFERRED.md)
gh issue list --repo loft-lang/plans --label status:finished  # delivered / closed
```

A plan's local `plans/<N>-<slug>/` dir (if it has one) lives at the top level
regardless of state — read its README `## Status` block for the in-place detail.
The `future/` / `deferred/` / `finished/` subdirectories are a **legacy archive**
from the old local-numbering era (closure records only); new plans are not added
to them.

Each plan directory contains a `README.md` with Status block, Goal,
and per-phase index — that's the per-plan source of truth.

For a work-list view across plans, see `../ROADMAP.md`: every active
plan has rows there tagged with value category + dependencies.
Deferred plans live in `DEFERRED.md` (trigger index).  Finished
plans have a one-line closure note in their README pointing at the
reference home.

Run `scripts/check_doc_drift.sh roadmap` to verify every active plan
is on ROADMAP at the right path and no finished/deferred plan has
crept into ROADMAP as an action item.

## One-off plans elsewhere

Per-session ephemeral plans not tied to a multi-phase initiative
live under `~/.claude/plans/` (flat, generated filenames).  Those
are not committed to the repo.
