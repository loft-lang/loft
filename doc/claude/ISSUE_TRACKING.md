<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Issue tracking — investigations in files, open bugs in GitHub Issues

**Status: DRAFT (2026-06).**  Pilot done (#246/#247); the rest of the migration is
gated on approval.  Rationale + evaluation: this is the multi-project answer —
loft / dryopea / lavition / `loft-lang/*` all have bug-filing needs, and discrete
bugs are a commodity that GitHub Issues serves better than N per-repo markdown
files, while **investigations** (which work, and which GitHub can't hold) stay in
files.

## The split — what lives where

| Kind | Home | Why |
|---|---|---|
| **Investigations** (multi-phase, probe-driven bug *classes*) | files: `plans/*/` + `probes/` + cluster catalogue | working artifacts GitHub can't hold (probe corpora, cluster docs); agent-grepped; versioned with the code |
| **Open bugs** (discrete: repro / severity / workaround / fix-path) | **GitHub Issues**, per repo | commodity records; cross-repo refs; triage / search / milestones; external discoverability (Goal B); one uniform tool across all repos |
| **Closed bugs** (the FIXED record) | `PROBLEMS.md` (frozen archive) + git history + the regression test | the fix + its test ARE the record; keep the history greppable |
| **Design entries** (the big `###` P-sections, e.g. @P213) | reference docs (`doc/claude/*.md`) | these are design docs, not bug rows — they don't belong as one-line Issues |
| **Feature / infra catalogue** (`@F<n>` / `@I<n>`, @PLN92) | **GitHub Issues in `loft-lang/features`** — the issue body (incl. its ` ```loft ` example) is the single source of truth | `index/features.json` = committed snapshot (`make features-fetch`); `doc/features/` + `tests/docs/features/*.loft` = one-way GENERATED shadow (`make features-gen`). **Never edit the shadow** — hand-edits fail the `features-check` drift guard and are overwritten by the next regen; a stale example (e.g. one the language has since outlawed) is fixed by editing the ISSUE, then fetch + gen + commit |
| **Benign tradeoffs / open work** (not defects) | `QUALITY.md § Open work` | a known tradeoff with a fix mapped, not a bug |
| **Plan sub-tasks** (the dependency-ordered work-items WE decompose from a plan + fix in sequence) | the plan doc (`plans/*/ROADMAP.md`) | self-created, self-fixed, transient; **don't file them as GitHub Issues** |

**An Issue is earned by being *surfaced*, not by being *planned*.**  A defect
found in the wild — especially one that blocks, recurs, or another repo hits —
becomes a GitHub Issue (commodity record, external discoverability, cross-repo
refs).  But when WE decompose a plan into its phases and fix them ourselves in
sequence, those phases stay **inside the plan doc** (`plans/<NN>/ROADMAP.md`).
Filing our own plan decomposition into the tracker just pollutes the issue list
with rows nobody outside the plan needs — the plan's DAG already tracks them, and
they close as the plan advances.  (Promote one to a real Issue only if it escapes
the plan: it blocks unrelated work, another repo hits it, or it's handed off.)

**Investigations PRODUCE Issues.**  The closure rule
([`plans/_INVESTIGATION_TEMPLATE.md § Closing`](plans/_INVESTIGATION_TEMPLATE.md#closing-an-investigation-plan-required))
now files **GitHub Issues** (not PROBLEMS.md rows) for the still-open findings +
sibling bugs.  The file-based deep-dive emits commodity bug records into the
commodity tool; the active-phase no-file rule is unchanged.

## Convention (apply in EVERY repo — this is what makes multi-project pay off)

Without one shared convention, N repos' Issues drift exactly like N PROBLEMS.md
files would.  The win is the *uniform* convention, not GitHub itself.

- **Templates** — copy `.github/ISSUE_TEMPLATE/{bug_report,feature_request}.yml`
  into each repo (dryopea, lavition, `loft-libs-*`).
- **Title** — `[<area>] <one-line>`.  For any issue migrated from PROBLEMS.md,
  keep the legacy `@P###` token in the title so OLD doc references still grep
  (`gh search issues "@P396"`).  The canonical way docs *reference* an issue is the
  indexed `@GH###` token (below), not the bare gh number.
- **Reference token: `@GH###`** (the indexed tracker) — docs reference a gh issue
  as `@GH<number>` (e.g. `@GH247`), NOT bare `#247` / `loft#247`.  The `@`-prefix
  makes it an INDEXED token exactly like `@P###` / `@PLAN###`: `scan.loft` finds
  every reference + backlink **fully offline**, and `@GH247` maps to a
  deterministic URL (`github.com/<repo>/issues/247`) with no `gh` call.  Token
  families: `@P###` = legacy/closed (PROBLEMS.md archive), `@PLAN###` = plans,
  `@GH###` = live issues.  Optional validation (does it exist / is it closed) is a
  bolt-on `make index-gh` (`gh issue list --json number,state`), not a
  prerequisite.  Cross-repo: bare `@GH###` = this repo; a qualified spelling for
  other repos (`@GH:<repo>:<n>`) is TBD and the less-common case.
- **Labels** — meanings live in [`.github/LABELS.md`](../../.github/LABELS.md)
  (the glossary so other agents don't dig into the loft source to learn what
  `area:codegen` covers) + each label's GitHub `description` (the inline gloss).
  Beyond GitHub's defaults `bug`/`enhancement`/…:
  - severity: `sev:high` / `sev:medium` / `sev:low`
  - **workaround** (created): `wa:clean` / `wa:partial` / `wa:none` — see
    [§ Workarounds](#workarounds--the-agents-can-you-keep-moving-signal)
  - area: `area:codegen` / `area:closures` / `area:store-lifetime` /
    `area:parser` / `area:native` / `area:wasm` / `area:stdlib` /
    `area:packages` / …
  - cross-cutting: `both-backends`, `needs-design`
  - triage-state: `attention` (stuck after 2+ tries), `design` (blocked on a
    user decision), `by-design` (closed — intended, cites a `DESIGN_DECISIONS`
    `C##`)
  - lifecycle: `fixed-pending-merge` (fixed on the working branch, awaiting the
    merge to `main` — see [§ Issue lifecycle](#issue-lifecycle--what-each-state-means-read-this-before-picking-work))
- **Where a bug's issue lives — file it in the repo whose source fixes it.**
  GitHub closing keywords auto-close **only same-repo** issues; a cross-repo
  `Fixes owner/repo#N` *links* but does **not** auto-close, and the
  `fixed-pending-merge` lifecycle (apply-on-push → close-on-merge → strip-on-close,
  the `.github/workflows/{apply,strip}-fixed-pending-merge.yml` pair) is **per
  repo**.  So the routing rule is mechanical — *where does the edit land?*:
  - compiler / runtime / in-loft stdlib (a `build.rs` or `src/` fix, e.g.
    `@GH274`) → `loft-lang/loft`.
  - an extracted library's own code (e.g. a `graphics` text-metrics fix in
    `native/src/text.rs`, `@GH252`) → that library's **chunk repo**
    (`loft-lang/loft-libs-<chunk>`) — NOT the loft repo, and NOT the read-only
    snapshot mirror under `tests/fixtures/libs/<pkg>/` (editing the mirror is
    drift; `sync-fixtures.sh --check` fails).
  Then `Fixes #N` is same-repo → closes on merge → the lifecycle workflows
  (carried by **every** repo's `.github/workflows/`, copied from this pair) just
  work.  A bug filed in the wrong repo is **re-homed** — re-file in the owning
  repo, close the original with a pointer — never closed by a cross-repo `Fixes`,
  which won't fire.  After a library fix, the loft-side fixture re-sync
  ([LIBRARY_AUTHORING.md § 5d](LIBRARY_AUTHORING.md)) is a **separate** reviewable
  commit in loft, not the issue's closer.
- **Cross-repo** — a bug in repo A that *blocks* repo B → an Issue in A, referenced
  from a `blocked-by`-labelled tracking Issue in B (`loft-lang/loft#247`).  The
  dogfood loop (moros / dryopea drive loft) lives on these links.
- **Roadmap** — a `gh` Project board across the orgs for "which release bundles
  which consumer-driven work"; ROADMAP.md can't span orgs.

## A repro carries the base it was measured on — name it

A repro written from a work branch silently depends on every OTHER fix that branch holds, and
the reader who tries it on `main` gets a different symptom. Twice in one day (2026-08-28) that
cost a false start between the two checkouts:

* **loft#1135** ("two generators leak one store") was filed with *"values are correct on both
  backends throughout"*. True on the branch it was measured on, which had loft#1130 fixed. On
  `main` the same program does not leak at all — it fails an assertion, because the yielded
  keyed-collection LITERAL is corrupted by #1130 and the lookup answers null. A reader would
  spend the first hour on the wrong defect.
* **loft#1139**'s workaround (`t = mk(); v += [(t.0, t.1)]`) compiles on any base and answers
  WRONG on one without loft#1134, because the rebuilt member goes through the tuple-element
  write that #1134 fixes.

So, in the body of any issue whose repro was run from a work branch:

* say which base it was measured on (`measured on <branch> @ <sha>`), and
* if it needs another fix to be reachable at all, name that issue — *"needs #1130; without it the
  assertion fails first"*.

And when a repro's symptom does not match its report, **suspect the base before the report**:
re-run it on `main` and on the filer's branch tip before concluding the issue is wrong. The
cheap isolating move is to apply the other issue's own WORKAROUND — that is what separated
#1135 from #1130 in one run.

### The CONTRAST cell carries the base too — and it is the one nobody re-measures

The cases above are about the cell that BREAKS. A differential claim — *"X leaks, its twin does
not, so the axis is D"* — rests just as hard on the cell that WORKS, and that half fails
differently: the broken cell reproduces everywhere, so the issue looks solid, while the twin
that makes it a contrast is flat only on the filer's tree.

loft#1248 read *"the non-capturing twin and the named twin are both flat — the axis is the
capture, not the shape."* Measured at N=400, one loop per run:

| cell | `main` | pre-#1245 (the fix's own parent) | post-#1245 |
|---|---|---|---|
| capturing | 404 | 404 | 404 |
| non-capturing twin | 403 | 403 | 5 |
| named twin | 4 | 4 | 4 |

The leak is 404 on every tree — a `main` defect, correctly filed. But loft#1245 moved the twin
from 403 to 5, and *only* the twin. On `main` the two cells read 404 and 403, so the axis the
issue names is not derivable there at all. The sibling checkout re-measured it, found both
lambdas leaking, and had every reason to read the report as misfiled.

So for a differential claim:

* **measure the contrast cell on the tree the READER will be on**, not only on yours — it is
  load-bearing evidence, not background;
* **prefer the fix's own PARENT as the control over `main`.** `ee1993019` isolates loft#1245;
  `main` confounds it with the ~90 other commits on the branch, which is why the parent column
  above is the one that settles it;
* if the contrast needs a fix to exist at all, say so — the issue is then *under-witnessed*, not
  misfiled, and those want opposite responses from a reader.

Related: [DEBUG.md](DEBUG.md) § matrix-first, and
STABILITY_METHOD.md § *When a filed issue names which route is broken*.

## Workarounds — the agent's "can you keep moving?" signal

A bug's workaround is the **primary thing the loft agent communicates to others**
(moros / dryopea / library authors / users): *can you route around this, or are
you blocked?*  Every bug carries both halves:

- a **`### Workaround` section** in the body — what to do today, OR what you tried
  that did NOT work;
- a **`wa:*` label** (the queryable metadata):
  - `wa:clean` — a simple idiomatic alternative, **verified working**;
  - `wa:partial` — a workaround exists but is awkward / loses the intended
    behaviour, **verified working**;
  - `wa:none` — nothing works; this **blocks** whoever hits it.

**THE RULE: a workaround is VERIFIED or it is not claimed.**  Run it on current
HEAD, both backends, and put the command + result in the section.  An **unverified
or WRONG workaround is worse than `wa:none`** — a downstream developer who follows
a confident-but-broken workaround loses time *and* stops trusting the signal,
which is the signal's entire value.  Same rigor as the probe-first fix rule: a
claim is a thing to *verify*, not assert.  (Pilots: #246 `wa:none` — the rebind
was tested, yields `[]`; #247 `wa:partial` — the non-capturing alternative was run
on both backends, printed `10`/`7`.)

**Triage:** `gh issue list --label "wa:none"` is the blocked-set — often more
urgent than raw `sev:`, because severity is "how bad when hit" and `wa:` is "can
you avoid being hit."

## The public intake bridge (arc D)

*A stranger's report enters the fix-not-file flow — without reintroducing a backlog.*

loft's internal rule is **fix, don't file**: whoever finds a bug fixes it (repro
warm, paths loaded), so the tracker never grows a backlog.  That rule is right —
but it is a rule for people who **can** fix.  A stranger cannot; for them, filing
is the *only* move.  The public bug template
([`.github/ISSUE_TEMPLATE/bug_report.yml`](../../.github/ISSUE_TEMPLATE/bug_report.yml))
is their door in; this bridge is how what they file becomes a warm, reproducible
fix-input the internal discipline consumes.

**Reconciliation (the load-bearing claim).**  fix-not-file forbids *the finder who
can fix* from filing instead of fixing — its rationale is *repro warm, no
re-derivation to re-pay later*.  A stranger has no fix ability, so their file is
not that forbidden move; it is the **input** fix-not-file consumes.  The bridge is
the seam: the stranger files (warm to *them*), the maintainer reproduces +
minimises (re-warming it *internally*), then fix-not-file runs unchanged.  So the
public intake **extends** fix-not-file's reach to bugs no insider found; it does
not weaken it, and it is a queue with a standing consumer, not a backlog.

**The bridge — a public report → the fix flow:**

1. **Acknowledge — it never vanishes.**  Every public report gets a triage
   response (a label + a reply).  This is the *acknowledgement promise*: a report
   left sitting with no maintainer response is itself a lapse, because the
   never-break promise (step 5) rings hollow if reports vanish.  It is the standing
   consumer, **not an SLA number** — `gh issue list --label needs-triage` is the
   un-drained intake, and nothing there should sit unacknowledged.
2. **Label — the maintainer triages; the reporter can't.**  A public report arrives
   carrying only `bug` (the [access model](#access-model--transparent-by-default-mutation-gated-by-role)
   gives the public no label write).  Triage adds **`needs-triage`** on arrival,
   then — once reproduced — the `sev:` / `area:` / `wa:` that make it a *ripe* bug
   (§ Item lifecycle), and removes `needs-triage`.  If it doesn't reproduce or the
   repro is incomplete, it goes to **`status:unclear`** with a *specific* ask back
   to the reporter — not silence.
3. **Reproduce + minimise to a both-backend repro.**  The template requires a
   minimal repro; triage's job is to shrink it to the *actionable core* — the
   smallest program that fails, checked on **both backends** (the smaller the
   program, the faster the fix).  This minimisation *is* the internal re-warming
   fix-not-file needs; the arc-B version axis answers "does it still reproduce on
   `main`?" so an already-fixed report closes with a pointer, not silence.
4. **Fix-not-file — a maintainer who CAN fix picks it up warm.**  From here the
   normal flow runs unchanged: matrix-first investigate → fix → regression test →
   verify both backends → `Fixes #NNN` (§ Resolving an issue).  The stranger's file
   and the insider's fix are the two halves of one `file → fix` pipeline.
5. **Close with the fix — never `wontfix` for a regression.**  A report that an
   upgrade broke a working program is a **top-priority regression** (the never-break
   promise, arc A — [COMPATIBILITY.md](COMPATIBILITY.md)); the promise *forbids*
   closing it `wontfix` / "managed change".  Other outcomes close as usual
   (`by-design` citing a `DESIGN_DECISIONS.md` `C##`, `duplicate` citing the
   canonical issue) — always a pointer, never silence.

**Stated at the intake.**  The promise and the routing are one click from "New
issue", so a reporter of a regression files it as the bug it is (not a timid "is
this expected?") and a library bug reaches the right repo: the public form's intro
+ [`CONTRIBUTING.md § Reporting a bug`](../../CONTRIBUTING.md) + the
[`config.yml`](../../.github/ISSUE_TEMPLATE/config.yml) chooser + the
GitHub-surfaced [`SUPPORT.md`](../../SUPPORT.md) all carry them.  Design + the full
failure-path enumeration this bridge closes:
[plans/102-stability-contract/public-bug-intake.md](plans/102-stability-contract/public-bug-intake.md).

## Issue lifecycle — what each state means (read this before picking work)

**`open` means there is work to be done.**  The tracker's open set is the agent's
worklist; nothing closed or in the pending state is a pick-up candidate.

| State | Meaning | Is it a pick-up? |
|---|---|---|
| **open**, no blocking label | a real defect with work remaining | **yes** — investigate + fix |
| **open** + `design` / `needs-design` | open, but **blocked on a decision** — the next move is a design call (often the user's) | no — surface options, don't grind a fix |
| **open** + `fixed-pending-merge` | **done** on the working branch; the only step left is the merge to `main` (not the agent's action) | **no** — it's finished, just in transit |
| **closed** | terminally resolved: `by-design` / `duplicate` / `wontfix` (a non-fix outcome, no merge pending), **or** the fix reached `main` (auto-closed by `Fixes #NNN`) | no |

So the agent's actionable worklist = **open AND NOT `fixed-pending-merge` AND NOT
`design`/`needs-design`** (the last two need a decision first).

**Why `fixed-pending-merge` instead of closing on the working branch.**  `main` is
the release branch; a fix that lives only on a long-lived working branch is **not
in `main`**.  Manually closing it would make the tracker say "fixed" while the
released code still has the bug — and the eventual merge's `Fixes #NNN` would then
close-an-already-closed issue, the **close ↔ reopen ping-pong** we explicitly
avoid.  The pending label keeps the issue **honestly open** (work is *not* awaiting
the agent) until the merge auto-closes it in one clean transition.

## Resolving an issue (the close half)

Filing is half the loop; closing is the other half.

- **Reference the issue in the fixing commit** — `Fixes #NNN` / `Closes #NNN` in
  the commit (or PR body); GitHub auto-closes it when that lands on the default
  branch.
- **On a working branch with no PR, do NOT close manually.**  After pushing, add
  the **`fixed-pending-merge`** label (and a comment naming the fixing commit +
  the regression test).  The `Fixes #NNN` line closes it when the branch merges to
  `main` — one transition, no ping-pong.  Manual `gh issue close` is reserved for
  **terminal non-fix outcomes** (`by-design` → cite a `DESIGN_DECISIONS.md` `C##`;
  `duplicate` → cite the canonical issue; `wontfix`) — **and the park-and-close of a
  `deferred` idea** (close-reason *not planned*, keep the `status:deferred` label, point
  at its design doc + un-defer trigger; reopenable, not terminal — see
  [§ Parking a deferred idea](#parking-a-deferred-idea--close-it-into-its-design-doc-dont-hoard-it-open)).
- ⚠ **An issue fixed as a SIDE EFFECT of another issue's commit will never auto-close.**
  `fixed-pending-merge` is right for it — the fix really is on the branch — but the close is
  driven off the `Fixes #NNN` trailer, and no commit names it. So the label is a claim the
  automation cannot honour, and the issue sits open through the merge that fixed it. Give it a
  trailer on a real commit (the doc or register entry that records the cross-issue closure is a
  fine home) rather than hand-closing, which the rule below reserves for non-fix outcomes.
  The audit is one line and worth running before a PR — it reads the whole board against the
  whole history, so a hand-applied label with no trailer shows up as the only row:

  ```bash
  gh issue list --state open --limit 80 --json number,labels \
    --jq '.[] | select([.labels[].name] | index("fixed-pending-merge")) | .number' | sort > /tmp/fpm
  git log --all --format=%B | grep -oE 'Fixes #[0-9]+' | grep -oE '[0-9]+' | sort -u > /tmp/fixed
  comm -23 /tmp/fpm /tmp/fixed      # labelled, but nothing closes it
  ```

  Sort BOTH lexically (plain `sort`, not `sort -n`) — `comm` compares byte-wise and silently
  reports garbage on a numerically-sorted input.
- **Correct the labels the fix invalidated**, not just add `fixed-pending-merge`.
  A label is a claim about the issue's CURRENT state, and fixing it settles several
  of them at once: `needs-design` comes off the moment the design question is
  answered, `needs-triage` once it is triaged, `blocked-by` once the blocker lands,
  `status:*` moves to what it is now waiting on.  Add what triage never supplied —
  a missing `sev:` / `area:` is normal on a report written by a consumer, who has
  the repro but not the subsystem.  This matters because the labels are the QUERY
  surface: `gh issue list --label needs-design` is read as the design backlog, and a
  solved issue sitting in it sends the next agent to re-answer a question that has
  an implementation.  Cheapest at fix time, when what changed is still in hand.
- **Judge the CONTRACT axis, in the fixing commit.**  Write a `Contract: settled` or
  `Contract: strained` trailer beside `Fixes #NNN`, plus one line of why
  ([.github/LABELS.md § `contract:`](../../.github/LABELS.md)).  *Settled* = the formal
  rules and the existing tests already gave the right answer and the fix makes that
  promise hold; *strained* = closing it EXTENDED a rule, changed a documented surface,
  or needed a design call.  **This is the one moment the answer exists** — it is what the
  fix turned out to need, which nobody could know when the bug was filed — and over a
  month the settled : strained ratio is the convergence signal the contract-1 decision
  reads (`make bug-review` § 5).  `.githooks/commit-msg` asks for it while you type, and
  the push then applies the label itself — the run that labels the issue
  `fixed-pending-merge` reads the same commits for the trailer, and warns when a `Fixes #N`
  arrives without one.  `scripts/contract_labels.py` is the backstop for what that run
  cannot see (a push over 20 commits, a trailer amended in later): it names the fixes on a
  branch that went without and applies the labels.  Absence counts as UNJUDGED, never as
  settled.
- **A fix needs a regression** — link the `tests/scripts/NNN` / `tests/*.rs` that
  locks it in.  A `fixed-pending-merge` issue with no regression is a re-opening
  waiting to happen.
- **Re-verify the workaround on resolve** if the issue had one — a fix can make a
  `wa:partial`/`wa:none` moot; keep the record accurate.
- **Don't file a bug you fix in the same change** — the fix + its test ARE the
  record (CLAUDE.md § Bug-filing policy).

## Item lifecycle — the `status:` axis (bugs + enhancements)

Every item carries a `status:*` label = **what it is waiting on**, on one shared
axis.  Bugs take the **short** path; enhancements take the **full** path — because a
*want* must clear value-vs-cost before it is committed, where a *fault* is committed
to by default.  (An enhancement is a small plan; the outcomes below are the plan
outcomes at issue scale — see *Beyond bugs — the unified model* below.)

**Intake gate (well-formedness).**  An item is not actionable until its body makes
the gap explicit — a **bug**: *expected vs observed* (+ a reproducer); an
**enhancement**: *what works now* vs *what you want from us*.  Until then it sits at
**`status:unclear`** (blocked on **information**).

**Intermediate (open):**

| `status:` | waiting on | bug | enhancement |
|---|---|---|---|
| `unclear` | information | ✓ | ✓ |
| `need-approval` | a decision — needs the **cost** beside the value: effort, systems changed, risks | — *(a fault is fixed by default; `needs-design` / `attention` cover the rare fix that needs a design call)* | ✓ |
| `approved` | doing it — **maintainer greenlit it** (the go decision; design may still need pinning) | — | ✓ |
| `designed` | doing it — approved AND the design is pinned, ready to code | — *(a bug's design is the inline investigation)* | ✓ |

**Resolution:**

| outcome | bug | enhancement | closes? · register |
|---|---|---|---|
| implemented / fixed | fixed | implemented | yes, on merge · `fixed-pending-merge` interim |
| **deferred** | — *(edge: a parked low-sev bug)* | ✓ — **parked in its design doc, closed not-planned**, un-defer trigger (reopen on fire) | **yes** (not-planned; keep the `status:deferred` label) · idea → its canonical design doc — see [§ Parking a deferred idea](#parking-a-deferred-idea--close-it-into-its-design-doc-dont-hoard-it-open) |
| declined | `wontfix` / `by-design` | `rejected` | yes · rejected → DESIGN_DECISIONS.md |

A **bug has 2 terminals** (fixed / declined); an **enhancement 3** (implemented /
deferred / rejected) — the extra `deferred` is the *want* that can wait.  An
enhancement **links out to a `@PLN` / `plans/<NN>/` lazily** — only when the design
grows phase-worthy (usually at `status:designed`); no renumber, the issue stays the
lightweight capture.  Feature requests use the `feature_request` template; the plan /
ROADMAP carries the design + sequencing.

### Parking a deferred idea — close it into its design doc, don't hoard it open

`deferred` is a **want that can wait** — but a want with no present consumer does not
belong in the open set, which is the agent's worklist (§ Issue lifecycle).  Left open,
deferred items grow a tail of "someday" rows that read as work-remaining and never
shrinks.  So the disposition for a deferred idea is to **move the idea to its canonical
design doc and close the issue not-planned** — closed ≠ deleted: a closed issue is
searchable, linkable, and one-click reopenable when its trigger fires.  Three steps, in
order:

1. **Split active debt from the parkable idea — verify ground truth first.**  A
   "deferred" row often bundles a present fragility (already manifesting) with a
   trigger-gated future want, and a sub-item may have quietly shipped since filing.
   Re-read the code + the design doc before disposing.  *Worked example #389:* its
   "Part 2" (cdylib + `[native]` linking) had **already shipped** — regression test and
   all; the issue body was stale — and its "Part 1" (raw `*mut Stores` → a `LoftStore`
   handle) was **active robustness debt**, not an idea.  Only what is *both*
   trigger-gated *and* has no present consumer is parkable.
2. **Re-home the non-parkable parts.**  Active debt goes to its forward-work register —
   a [STABILITY_HOTSPOTS.md](STABILITY_HOTSPOTS.md) H-entry, a `ROADMAP.md` slot, or an
   open `status:approved` enhancement — where it stays tracked; it is *not* parked-closed.
   *#389 Part 1 → hotspot H9.*
3. **Park the idea + close not-planned.**  Write the design + a **crisp un-defer trigger**
   into the canonical design doc (the one that already owns the area — `NATIVE.md § Open
   work`, a plan, `ROADMAP.md`), then `gh issue close --reason "not planned"` with a
   comment pointing at that anchor and restating the trigger.  **Keep the
   `status:deferred` label on the closed issue** so the parked-idea bank stays one query
   away: `gh issue list --state closed --label status:deferred`.  *#388 → NATIVE.md @PLN26
   ph.1 row; trigger: a consumer must call a symbol two un-renameable packages both export.*

The trigger is what makes this safe rather than lossy: a park **without** a concrete
"un-defer when X" is how an idea is lost; a park **with** one is just work that hasn't
been scheduled.  This supersedes the older "deferred stays open" wording — the idea's
home is its design doc, not an open row.

### Ripeness — a `status:` is earned by its data, not assigned

A `status:` claims the ticket holds the data that state requires.  **Evaluate the
ticket against the entry criteria before promoting it; if the data isn't there it is
*not ripe* and stays at the lower state.**  (A label assigned without the data is the
same failure as a fix asserted without the matrix.)

- **leave `unclear`** — the gate is filled (bug: *expected vs observed* + a repro;
  enh: *now vs wanted*).  A ticket also stays `unclear` whenever it is not yet
  *decidable* — e.g. value is clear but the cost can't be assessed until an upstream
  design/scope call is made (mark that with `needs-design`).
- **enter `need-approval`** — the body holds **all three**: (1) **value** (now-vs-wanted);
  (2) **cost** — the systems/files changed, a rough effort, the risks/blast-radius;
  (3) a **decidable proposal** — a decider can pick implement / defer / reject *without
  first doing design exploration*.  Missing any → not ripe → stays `unclear`.
- **enter `designed`** — `need-approval` passed *and* approved *and* the design settled
  (scope chosen, approach pinned).
- **`deferred` / `rejected`** are the *decision* — made on the `need-approval` data, so
  that data must be present to defer or reject *informedly*, never by default.

### Designing — `needs-design` is earned by its use cases

`needs-design` is **earned, not assigned** — like every status, by data, and its data
is **the use cases.**  A ticket that doesn't yet state its use cases isn't
`needs-design`; it's `unclear` — you can't see what it's *for*, let alone that it
needs a design call.  So before a ticket earns `needs-design`, the **use cases must be
included**, and they must pose a real scope/approach decision (usually a tension the
breaking cases expose).  Earning the state comes first; the design pass then resolves
it.

Clearing it — `needs-design` → `designed` — is the design pass, which works **both
halves of the use-case matrix**:

1. **Use cases served** — what the design makes possible (the *want*).
2. **Use cases broken or changed** — what existing, legitimate behaviour it affects.

The second half is the one that does the work: it is the matrix discipline applied to
*design*, and **the breaking cases reveal the real boundary** (the matched scope —
*no wider*).  A design that lists only what it serves is the "no-wider" failure
waiting to ship.  #255 is the worked example — "switch the path anchor from cwd to the
program" looked decided until the breaking case (a CLI tool resolving the *user's*
file) exposed **two kinds** of relative path, and the matched design became
single-anchor + a one-line opt-in, not a global switch.  Until that pass is run the
ticket stays `needs-design`; cost can't be assessed against a scope the breaking cases
haven't bounded.

### The done-gate — `fixed` / `implemented` is earned, not declared

Ripeness guards the way *in*; the **done-gate guards the way out**.  Before a ticket
earns `fixed` (bug) or `implemented` (enhancement), evaluate the *result* against two
checks — their failure has a loud symptom: **the requester immediately files a slight
variation** (the production-time form of *"the un-generalized remainder is the same
bug, unfinished"* — found by the user instead of by us).

1. **Class coverage** — did the fix enforce the **invariant / whole class**, not just
   the filed repro?  A narrower fix leaves siblings and the requester files one.  (The
   matrix protocol's "no narrower," checked at closure: the `i16` case, the nested one,
   the other backend, the other context.)
2. **Intent match** — does the result deliver **what the requester actually wanted**,
   not just what they literally typed?  (#255: the literal complaint was *wrong*; a
   perfect fix of the words would have missed the real want.)

Operational test — **"would the requester read this result and file a slight
variation?"**  *"…but what about &lt;sibling&gt;"* → class miss; *"…but that's not what
I meant"* → intent miss.  Either → **not done**: widen the fix, or re-scope to the
intent, before closing.  Any residual that legitimately can't be closed now (a real
*separate* sibling, a verification you couldn't run) is **named on the ticket**, not
left silent — that is the difference between a tracked follow-up and a slip.

## The work queue — what's workable, and the dual flow

A goal that says *"work the queue"* acts only on items that are **workable now**, and
the predicate **differs by type** (the bug/enhancement duality).  Items needing a
human decision are **surfaced, not churned**, and **the loop ends when no workable
item remains** — even with open items left, because the rest are in your court or
parked.

| type | workable when… | the agent does | not workable → surface to you |
|---|---|---|---|
| **bug** | **ripe** — well-formed (expected/observed + repro), not `needs-design` / `attention` | matrix-first investigate (code-only agent) → fix → regression → verify both backends → `fixed-pending-merge` | `status:unclear` (info/repro) · `needs-design` / `attention` (a design call) |
| **enhancement** | **`status:approved`** (or `designed`) — you greenlit it | implement within the approved scope (pin the design first if not yet `designed`) → `fixed-pending-merge` | `status:unclear` (clarify/scope) · `status:need-approval` (**your decision**) · `status:deferred` (parked) |

The two **reasons work can be done** are different by design: a *bug* is workable
because it is **ripe** (a fault, ready to fix); an *enhancement* because it is
**approved** (a want, greenlit).  That is the duality — and `status:approved` is the
trigger **you** set to move an enhancement from your court to the agent's.

**Termination + report.**  When nothing is workable, **stop** — do not read "open
items remain" as "incomplete."  Report the remainder by *why it's yours*: *N awaiting
your approval · M need a design call · K parked (deferred)*.

**Goal phrasings:**
- **"work the queue"** — fix every ripe bug + implement every `status:approved`
  enhancement, then stop and report what's in my court.
- **"work the bugs"** — ripe bugs only.
- **"work #NNN"** — a single item, still gated on it being workable.

### The mirror — "what's blocked on you?"

`"work the queue"` is the **agent** half of the loop; the **maintainer** half is its
inverse.  Ask:

> **"What's blocked on you — highest-leverage first?"**

It returns **decisions and authorizations, not status** — only the things the
maintainer's attention is the bottleneck for — **ranked by how much each unblocks**, so
one spare minute goes to the item that frees the most.  **Format matters: lead with
the ONE highest-leverage item in full** — the decision it needs + the minimum to make
it — then **a one-line summary of each of the rest**, never twenty detailed rows.  One
thing to act on now; the others at a glance, so the landscape is visible without
making the maintainer process all of it.  (This is the reporting norm for the whole
workflow, not just this question — long detailed lists spend the maintainer's
attention on reading instead of deciding.)

What lands in the maintainer's court (the "surface to you" column above, plus the
out-of-band gates):
- **decisions** — approve a `need-approval` enhancement (set `status:approved`), pick a
  scope on a `needs-design` item, reclassify, set priority;
- **authorizations** — force-push, open a PR, merge, run an interactive command only
  the maintainer can (a login, a real host);
- **information** — answer the question a `status:unclear` item is blocked on;
- **external actions** — a change in another repo, or a resource only the maintainer has.

Together the two halves are the **combined workflow**: *work the queue* drains what the
agent can do; *what's blocked on you* surfaces, ranked, exactly what it can't — so
nothing stalls silently and the maintainer's time goes to the highest-leverage call.

## Beyond bugs — the unified model (plans · lib-plans · enhancements)

**Initial design (2026-06) — draft.**  Bugs were the pilot; the same split
generalises to **every** work item, with one addition: **Subject** (the
consumer/deliverable) as the primary axis — the multi-project answer a per-repo
`ROADMAP.md` structurally can't give.

### As built (2026-06) — where the plans actually live

The rest of this section is the *design rationale*; here is the **current reality**
(it supersedes the draft where they differ):

- **Plans live as Issues in [`loft-lang/plans`](https://github.com/loft-lang/plans)** —
  the central, public, cross-ecosystem overview (the generic *org* home, not
  `loft-lang/loft`).  Distributing plans across individual product repos would lose
  the overview; centralising the tracking keeps it built-in.  The labelled issue
  list **is** the overview today.
- **`@PLN<N>` is the canonical plan identity** = that repo's issue number
  (`@PLN3` → `loft-lang/plans/issues/3`).  Simple, globally unique, scales to every
  subject — the tag *is* the number.
- **`@PLAN<NN>` is now only a legacy *dir-pointer*** ("design lives in
  `plans/<NN>/`"), carried in the issue body; per-tree and harmless because the
  `@PLN` number is the real key.  Active + future local dirs are being migrated
  to `@PLN` issues under **@PLN27** (`finished/` keeps its `@PLAN<NN>` refs).
- **Dimensions ride on labels** for now — `subject:*` (the primary cut) +
  `status:*` (lifecycle); **every plan issue MUST carry exactly one of each**
  (`subject:{loft,libs,audience}` × `status:{active,future,finished}`).  A gh
  **Project board** (the richer field schema below) is a later browser add-on;
  the labelled list already gives the overview.
- **Design stays in each code repo's `plans/` dir**; the loft-lang issue links to it.

### Why a clean view at all — the resonance test

The plans view is **not a management board.**  Its job is to be a *resonance
surface*: when someone who **shares the sensibility** sees the list, they grasp
**what kind of project this is — at a glance.**  It is the builder's counterpart to
a game playable in the browser — legible-on-contact for the right people, never a
funnel.  (The project is built for its own sake on a long horizon — see
[GOALS.md § Goal F "Grounding"](GOALS.md); adoption is a *consequence*, not a goal.
This view exists so the people who *would* resonate **can**, not to convert anyone
who wouldn't.)

**So the success test is character-legibility, not work-organization:** the board
succeeds when a kindred mind reads it and feels *"that's my kind of project"* — not
when "everything is tracked."  Every field and view decision is measured against
that, and read back through this lens the choices below already serve it.

The list must transmit three things at a glance, and **only** these:
- **coherence** — one idea, many expressions, visibly serving a single vision →
  this is why **Subject is the primary axis**;
- **depth / ambition** — serious, long-horizon, the hard plumbing genuinely being
  done → the **Value / Effort / Driven-by** framing carries it;
- **taste** — what is valued, and that it is valued for real → the **"why"** on each
  item, and the narrative.

Three things kill it — each already the reason for a design choice:
- a **ticket-dump** buries the soul under mechanics → keep it clean; *per-phase
  status + numeric priority stay off the board*;
- a **grab-bag** hides the single idea → *Subject-first + the dependency DAG keep
  the coherence visible*;
- **mechanics-without-why** shows no sensibility → *Value / Driven-by / the "why"
  are first-class, not optional*.

So **"clean" is not aesthetics — it is the medium.**  Clutter isn't just noise; it
is the project's character made *illegible*.

### The split, generalised

| Item type | Design home (files) | State home (gh Project) |
|---|---|---|
| **bug** | repro + probes + the regression test | the Issue + its fields |
| **plan** | grows with maturity: a `PLANNING.md` section (backlog) → a `plans/<NN>/` (loft) or `lib_plans/<NN>/` (libs) directory with phases (active) — see [`_PLAN_TEMPLATE`](plans/_PLAN_TEMPLATE.md) | a Project item |

Design in files; **state + sequencing + dependencies in one cross-org gh
Project**.  loft + libs plans are the **blueprint exemplars**; other subjects
replicate the `_PLAN_TEMPLATE` shape in their own repos.  Thin subjects (moros,
the demos) stay **unpadded** — a board that invents placeholder plans for an empty
subject is the same drift in a new costume.

#### Two kinds of item, not four — the PEP lesson

PEP (Python's enhancement-proposal process) already settled this: what PEP calls a
*proposal* is **the unit of intentional change**, and a one-paragraph one and a
multi-phase one are the *same kind* — size only changes length.  loft's "plan" vs
"enhancement" was never a real type split; it was **size + maturity wearing two
hats** (ROADMAP's own "features needing *plan promotion*" is a maturity transition
*within* one kind, not a conversion between kinds).  loft's word for it is **plan**
(matching `@PLAN` / `plans/` / `_PLAN_TEMPLATE`), so the Type axis is just **`bug`
vs `plan`**:

- a **bug** is a *fault report* (reactive — reality diverged from the spec);
- a **plan** is *intentional change* (proactive), whose **design home grows with
  maturity** — a `PLANNING.md` section while it's a backlog sketch, a `plans/<NN>/`
  directory with phases once active.  A "plan" is thus any maturity; Status tells
  you which.

Three things follow, the way PEP does them:

- **One number space (lazily).**  A plan carries one identity for life.  We do
  *not* renumber dormant catalog items: a backlog plan keeps its lightweight
  `PLANNING.md` ID; **on activation it earns an `@PLAN<NN>` number + a directory.**
  Promotion stops being a rename — it's just "the same item grew a home."
- **Identity on the board — the `@P###` trick, reused.**  Putting a plan on the
  board does **not** renumber it or rewrite a single reference.  Exactly as the bug
  migration did with `@P###`, the gh Issue's **title embeds `@PLAN<NN>`**
  (`[loft] @PLAN48 integer-width discipline`), so `gh search issues "@PLAN48"`
  resolves to the Issue while the `@PLAN` index keeps resolving doc references to
  the **dir**.  The gh number `#N` is plumbing (`Fixes #N`, closing) — **`@PLAN<NN>`
  stays primary**, the same way `@P###` stays primary for a migrated bug.  Because
  the `@PLAN` token is flatten-proof (the index re-resolves wherever the dir sits),
  the directory **need not move** and no `plans/*` path or `@PLAN` ref is touched;
  the `future/`/`deferred/` subdir becomes a vestigial hint with the **board
  authoritative for state**, and any `plans/*` *path* links flatten lazily, on
  touch, or never.
- **Status carries the terminal outcome**, because loft files each outcome
  *differently*: **shipped** (closure-record in the dir) · **declined** →
  `DESIGN_DECISIONS.md` (loft's closed-by-decision register *is* a declined-plan
  archive) · **superseded** (link to the replacement) · **deferred / withdrawn**.  A
  flat "done" would erase the distinction the routing depends on.
- **No Standards/Process/Informational "Kind" axis.**  PEP needs it; loft doesn't —
  it already routes Process (DEVELOPMENT.md) and Informational (DESIGN_DECISIONS.md,
  the reference docs) *out* of `plans/` by filing location.  Importing the axis
  would be cosplay.

What loft does **not** take from PEP: the Draft→Accepted acceptance ceremony (that
exists for a multi-stakeholder council; solo+agent needs only `backlog → active →
shipped/declined`).

**Investigation is not a board type.**  An investigation is a *file-level flavour*
of a plan — it additionally follows
[`_INVESTIGATION_TEMPLATE`](plans/_INVESTIGATION_TEMPLATE.md) for its clusters /
probes.  gh sees `Type: plan`; the "(investigation)" parenthetical lives in the
title + the README header.  Its specialness — it *produces* bugs — surfaces as
**outgoing links** to the Issues it spawned, which is data, not a type.  (Same for
"validation" / "feature" sub-kinds: title parenthetical + the Area field slice
them; none earns a structured board value.)

### What's good to track — the field schema

**Track a field iff** it is (1) **state** or a **relationship** (changes over
time, or links items), (2) something you **triage / sequence / group** by, (3)
**not derivable** from the file or another field, and (4) meaningful **across** the
board.  Design content, write-once-never-filtered facts, and second copies of what
the file already holds stay in the file.

| Field | Shape | Applies to | Why it earns a slot |
|---|---|---|---|
| **Type** | select · bug / plan | all | picks the lifecycle + the design home (a plan's home grows `PLANNING.md` → directory with maturity) |
| **Subject** | select · loft / libs / moros / dryopea / bumper-plane / audience / lavition / … | all | the **primary axis** — which consumer/deliverable |
| **Status** | select · backlog / next / active / deferred / shipped / declined / superseded | all | the lifecycle **+ terminal outcome** (shipped · declined→`DESIGN_DECISIONS` · superseded) — the **single authority** replacing dir-location + the README tables |
| **Area** | select · codegen / closures / store-lifetime / parser / native / wasm / stdlib / packages / … | all | subsystem; **unifies with the bug `area:*` labels** — one taxonomy for "all closure work" |
| **Milestone** | select · 0.9.0 / 1.0.0 / 1.1+ / ‹game› | all | release bundling — the cross-repo "which release ships this" that drove the move |
| **Depends-on** | linked items / @refs | all | the dependency **DAG** — ROADMAP's hand-drawn chains made real; powers a *derived* "blocked" view |
| **Effort** | select · XS / S / M / MH / H | plan | sizing, for sequencing |
| **Value** | select · Correctness / Enabling / Polish / Quality | plan | *why it matters / what kind*; doubles as coarse priority (board **order** refines) |
| **Driven-by** | select · ‹subject› | plan | the **dogfood link** — which consumer demanded this language/lib work |
| **Trigger** | text | deferred items | what un-blocks / revives it (the concrete defer-trigger) |
| **Severity** | select · high / medium / low | bug | how bad when hit (the existing `sev:`) |
| **Workaround** | select · clean / partial / none | bug | the "can you keep moving?" signal (the existing `wa:`) |
| **Repo** | select · loft / loft-libs-* / moros / dryopea / lavition | all (optional) | cross-org ownership + click-through (often implied by Subject) |
| **Owner** | assignee | all (optional) | who's on it — low value solo, grows with contributors |

**Deliberately NOT on the board** (lives in files, or derived):

- **Per-phase status** → the plan README is the source of truth (ROADMAP already
  says "read the plan README directly").  Mirroring phases is a guaranteed-drift trap.
- **Numeric priority (P0–P3)** → use the board **order**; a number drifts against
  the order it is meant to encode.
- **"Blocked" boolean** → **derived** (Depends-on has an open item), not hand-set.
- **Design / probes / repro / dates** → the dir + git (gh stamps created/updated
  automatically — enough to power a "stale" view).

#### Driven-by — the field the dogfood loop earns

loft's whole development model is *build a consumer → harvest the language lesson →
fix the language*.  **Driven-by** makes that loop **queryable**: "show every loft
plan dryopea is waiting on", "if we cut bumper-plane, which language work loses its
justification", "what lessons did moros harvest into 1.0".  It is distinct from
Depends-on (a *blocking* edge) — Driven-by is a *motivation* edge: the consumer
that would notice if this work vanished.  Without it, the consumer→language
dependency the whole project runs on is invisible to the tracker.

#### Open calibration — Value granularity

The draft collapses ROADMAP's eight bands (**S/R/G/F/U/C/Q/N**) to four:
**Correctness** (S + R), **Enabling** (G + F), **Polish** (U + C), **Quality**
(Q); **N**iche becomes "low board order", not a band.  Four sorts cleanly and
still answers *must-fix / unlock / nicety / refactor*; the finer eight remains
available.  **Decision point — keep four or the eight.**

### Access model — transparent by default, mutation gated by role

**Read is open; write is role-gated.**  The default is **public** for everything
with public value; a repo, board, or item goes private *only* when there is little
in it for the public (early scratch, a throwaway prototype, an idea not yet worth
showing) — a **usefulness** gate, not a secrecy one.  Building the games in the
open *is* the adoption story (the consumer→language dogfood loop, visible — Goal B),
so a game in development stays public.

| Who | Read | Write |
|---|---|---|
| **Public** (no role) | all public repos · Issues · the board | **only** open an Issue + comment — no label / field / Status / Milestone edits, cannot change a plan |
| **Triage** role | + | + label / set fields / close / reopen, *without* code write — the lever for a trusted non-code helper |
| **Write / admin** | + | everything |

GitHub has **no per-field public-write** — access is all-or-nothing by role, and the
public has none.  So "not all fields publicly changeable" is the *default*: every
structured field (`sev:` / `area:` / `wa:`, Status, Milestone, all Project fields)
is maintainer-only without configuring anything.

- **Plans are doubly read-only to the public.**  A plan is a *maintainer-authored*
  Issue (titled `@PLAN<NN>`) + a directory: the public can **comment** on the Issue
  and **view** the dir; changing the dir is a **PR you merge**.  Viewed + commented,
  never changed.
- **One public board** for the public-value ecosystem (loft / libs / games /
  demos) — public **views** the roadmap; editing fields or order needs project
  write.  The rare low-public-value item stays **off** it (a private note/repo)
  until it earns a place — and mind that a public board's draft cards are
  world-visible, so don't park a not-ready-to-show idea there.

> **Transition note.** "PROBLEMS.md" / "P-issue row" references elsewhere in the
> docs (plans/README, DEVELOPMENT.md, …) are repointed to GitHub Issues as they're
> touched.  PROBLEMS.md is frozen to OPEN bugs — it's the closed/historical archive.

## Migration plan

| Step | Action | Status |
|---|---|---|
| 1 | **Pilot** — file @P396/@P397 as Issues (#247/#246), drop from PROBLEMS.md | ✅ done |
| 2 | Create the `sev:*` / `area:*` / `wa:*` / cross-cutting labels | ✅ done (17: sev:/area:/wa:/regression/flaky/blocked-by/hit-by:) |
| 2.5 | **`@GH###` indexed tracker** (the one remaining CODE task — `tools/indexer/src/scan.loft`): (a) add `@GH<n>` to the tag tokeniser + a deterministic issue URL in `scripts/idx`; (b) in `tag_is_valid`, ALSO accept a `@P<n>` that appears in PROBLEMS.md's **freeze-banner `@P→#` map** (so the 7 migrated tags resolve instead of reading as broken — fully offline, the banner IS the map); (c) optional `make index-gh` validation bolt-on (`gh issue list --json number,state`). | ✅ done (04576e74) — @GH<n> tokenised + indexed; migrated @P resolve via the freeze-banner map; `idx tag:@GH247`/`gh:247` print the URL; index_hygiene green |
| 3 | Migrate the OPEN PROBLEMS.md rows → Issues | ✅ done — @P391→#248, @P389→#249, @P384→#250, @P351→#251, @P340→#252 (+ pilots #246/#247) |
| 4 | Freeze PROBLEMS.md — closed/historical record; FIXED rows + `###` design entries stay | ✅ done — freeze header + `@P→#` map; 0 open rows left |
| 5 | Flip the meta-doc filing rule "→ PROBLEMS.md" → "→ GitHub Issue" | ✅ done — CLAUDE.md (bug-filing + doc index + reading-by-goal), `_INVESTIGATION_TEMPLATE § Closing`, `plans/README § workflows` |
| 6 | USER_FACING.md — **KEEP** (revised: it's a curated *release-note-worthy deferred-work* tracker — features + user-visible bugs/perf + a showcase track — NOT a bug mirror, so not redundant).  Mark downstream-visible gh Issues with the `user-facing` label; USER_FACING.md cross-links them. | ✅ `user-facing` label created; cross-link as items arise |
| 7 | Apply the template + labels in dryopea / lavition / `loft-libs-*` as each needs bug-filing | ☐ ongoing |

## Agent note

`gh issue list/view/create/search` is the bug-layer interface; `idx` + files keep
serving plans/investigations.  Repros stay in-repo (commits, `tests/scripts/`,
`probes/`), so fixing a bug still has its context local — the Issue is the
tracker, the repo holds the artifacts.  The grep-ability lost on open bugs is
small; the grep-ability kept on everything that matters for *fixing* is total.
