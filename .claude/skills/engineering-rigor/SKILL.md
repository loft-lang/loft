---
name: engineering-rigor
description: >-
  The discipline for fixing a non-trivial bug or reshaping load-bearing code, in any
  codebase. USE THIS before you fix a crash, a silent corruption, a wrong answer, a leak or
  a disagreement between two implementations that should agree — and STOP AND LOAD IT when:
  the fix already looks obvious; a second fix for the same symptom has failed; probes do not
  converge or a diagnostic shows a symptom but not its cause; a failure happens "sometimes";
  or you are about to write a helper, predicate or type list that may already exist, or port
  working code to another target. It is a speed tool, not a brake: a coherent explanation is
  a hypothesis, so state the cause, then run the cheapest probe that could prove it wrong (a
  count, one look at what the tool actually produced, one timing). Then build the instrument
  that shows the whole CLASS (boundary matrix, falsification probe), find the ONE invariant,
  enforce it at the chokepoint (no narrower, no wider) and verify against what you wrote
  down. Routes to the project's debugging policy and to design-protocol for the design half.
user-invocable: true
---

# Engineering Rigor — See the Class Before You Act

## Start here — you are here because…

- **the fix already looks obvious** → state the cause, then run the cheapest probe that could kill it (§ The one idea; loop step 1).
- **a second fix for the same symptom failed** → stop fixing; build the boundary matrix that shows the class (§ The shared loop, steps 2–3).
- **probes do not converge, a diagnostic shows a symptom but not its cause, or it fails "sometimes"** → the instrument is blind: build attribution (§ The blind-instrument tell); if you can see but the frame is wrong, go deep on one case (§ Change altitude).
- **each part passes alone but they fail together** → read the boundary contract, then the shared medium (§ When verified parts fail together).
- **you are unsure which code actually runs** → a usage sentinel (§ The usage sentinel).
- **the fix keeps wanting one more special case** → the structure is wrong, not your logic (loop step 4).
- **you are about to write a helper, predicate or type list, or port code to a new target** → check it does not already exist (§ Has this already been written?).
- **you are choosing behaviour, not explaining it** → the `design-protocol` sibling skill.

## The one idea

**A coherent explanation is a hypothesis, not a conclusion — most of all an elegant one.**
The first story that fits was built from the *one instance in front of you*; it is right for
that instance and silent about the cases you could not see, so it breaks on them. You do not
ship fragility on purpose — a single instance cannot show you its class. So the cure is never
*"be more careful"* (a values lever, useless against a sight gap); it is **better eyes**: build
the instrument that makes the whole class visible, then act on what it shows.

This is a **speed tool**. The cheapest thing that could prove your idea wrong — one timing, a
`grep` count, one look at what the tool actually emitted — costs seconds and buys certainty, so
you fix once. A guessed fix only feels faster; being wrong is the slow lane. And because the
instrument shows the *class*, you fix the **source** — the chokepoint the family flows through —
and delete the category instead of this instance.

> **You can state a fix → that is the cue to grab the cheapest thing that could kill it.**
> Skipping this probe — never reaching for the instrument because a fix was already in hand —
> is the most common way rigor fails in practice.

## Two modes, one shape

The discipline has two faces depending on whether you are *explaining behavior that exists*
or *choosing behavior that doesn't yet*. They are the **same shape one level apart**.

| Step | DEBUG — explain existing behavior | DESIGN — choose future behavior |
|---|---|---|
| The thing you have | a symptom (crash, wrong result, corruption) | a problem + a candidate design |
| The trap | fix the first cause you find | build the first coherent design |
| The hypothesis | a **root cause** | an **invariant** |
| The instrument (your eyes) | a **boundary matrix** — vary one axis per probe | **falsification probes** — try to break each claim |
| Where the truth hides | in the *class*, invisible in any one repro | in the *class*, invisible in the prose |
| The act | fix at the chokepoint, enforce the invariant | build the chokepoint every site derives from |
| Verify against | the full matrix, on every mode the result can diverge across | the written prediction |

## The shared loop (run it in either mode)

1. **Don't act on the first read.** A coherent story — especially an elegant one-line fix or a
   clean unifying abstraction — is the *signal you haven't earned the action yet*. Real bugs
   are complex-variant and real designs carry weight you haven't probed; the clean story is
   usually the part of the picture you haven't looked at.

2. **Build the instrument — it is your eyes.** Debug: a boundary matrix of throwaway probes,
   varying **one axis per probe** (type-kind, construction-path, depth, null, backend — and,
   when the fault is in the *transform* rather than the value: **cardinality** (N of a thing
   coexisting), **ordering/position**, **cross-pass consistency** — the composition axes).
   Design: the cheapest test that could *falsify* each load-bearing claim. Control-flow (*which
   code actually runs*): a **usage sentinel** (§ below). Provenance (*where did this effect come
   from?*): an **attribution** instrument (§ The blind-instrument tell).
   *"I can't see the root / can't name the invariant yet"* means **the instrument isn't
   finished** — never a license to act on the one case in hand. When you cannot form a
   candidate invariant at all and the domain is exact (geometry, caching, lifetime,
   round-trips), the instrument turns constructive: build a concrete instance of the *answer*
   and read the invariant off it (`design-protocol` § The other half).

3. **The truth is visible in the class, invisible in the instance.** The shared mechanism
   behind a family of "different" symptoms shows up *in the matrix* and in no single repro;
   the false unification in a design shows up *under a probe* and in no re-reading of the
   prose. **An irreducible minimal repro is itself a reading:** when you cannot shrink below *N
   coexisting* constructs (N loops, N declarations, N allocations), stop shrinking — the bug
   *is* the coexistence (a resource / slot / accumulation fault), and chasing a one-instance
   repro there aims the matrix down the wrong axis.

4. **Find the ONE invariant; enforce it at the chokepoint — no narrower, no wider.** If the
   project keeps a named-rule register or formal spec, look the invariant up there BEFORE
   deriving it — a rule already written settles the question. *Narrower* (a per-case patch; an
   N-way spray) leaves the siblings broken. *Wider* (re-resolving more than the failing region;
   unifying distinct cases under a false invariant) drags blast radius. The proportionate move
   enforces *exactly* the invariant the whole failing region violates.
   *The chokepoint is often a **data structure**, not a code path.* When the fix keeps wanting
   another condition / branch / special case to re-derive a fact — or you are stuck, the change
   wanting more tests than it should — suspect the structure, not your logic. Fold the fact
   INTO the structure so the special case becomes the normal case ("good taste"; "data
   dominates"); every
   site then *reads* it instead of re-deriving. No structure is right upfront, so this recurs:
   recognize it, signal it, evolve.

5. **Verify against what you wrote down**, not against the action's own momentum. Debug:
   re-run the full matrix on **every mode the result can diverge across** — for a system with
   more than one execution path (an interpreter and a compiler), each path. Design: compare the
   built thing to the **written prediction**. An external commitment is what makes the check
   honest — without it you grade your own homework.

## The usage sentinel — "which code actually runs?"

The matrix varies **inputs**. Some questions are about **control flow**: which code actually
runs, which sites route through a point. There a **static count** (`grep` the call sites)
**over-counts** — it sees dead and live uses alike. A **usage sentinel** routes every use
through one observable chokepoint and makes it loud (count it + name the caller, or trip under
a flag). One run turns a control-flow guess into a runtime fact:

| Use | Sentinel question | The answer you want |
|---|---|---|
| **Removal** | is anything still using this? | **none** — zero across the whole suite → safe to delete |
| **Debug** | does the path I *think* runs actually run? | **the expected sites fire** — catches a fast path never taken, a handler secretly dead |
| **Design** | is my chokepoint the *sole* route every site takes? | **all sites, only here** — the runtime form of one-home-per-fact |

**Silence is evidence only after a positive control.** A silent sentinel reads the same whether
the consumer is dead *or the sentinel sits on a dead path*. Removal gets the control free (the
whole-suite run); debug and design must show the chokepoint fires for a known-live case before
trusting a zero.

**Validate any instrument before you trust its reading** — it is part of the system. Two
properties, each failing silently as a "bug" that isn't one:

- **Calibration** — *can it report the truth for this question?* A dead-path sentinel can't;
  neither can a **stale build artifact** — a cell reflects the *binary you ran*, not the source
  you edited, and each build output (debug, release, each target, each fixture) can lag the
  source independently. So **a surprising cell is a stale-artifact suspect before it is a
  bug**: rebuild the output behind it and re-read. (Loop step 5's cross-mode verify needs
  this as a precondition: each mode is a separate lens that must be in focus on its own.)
- **Non-intrusiveness** — *does observing change the result?* For codegen / slot / layout /
  aliasing faults it does: a `print`, a temporary or one more reference moves the bug. When an
  added observation flips the outcome, that perturbation **is** the boundary — bisect *which
  minimal edit flips it*. Then read from **outside the run** (IR / bytecode / trace dumps, the
  generated source), never an in-program probe that re-runs the compiler you are inspecting.

## When verified parts fail *together* — read the shared medium

Each part **passes alone** yet they **corrupt each other in composition**. Re-auditing the
parts is wasted motion: **verification is local and instantaneous; a real system is embedded
and historical.** The bug is in the **medium they share**, which sits in no part's contract —
as separately sound physical components fail each other through shared air pressure, static
buildup or a tear through shared material.

1. **Read the boundary contract first.** One part's output vs the next's expected input, each
   verified against a different spec, is a composition failure you can **audit**. Rule it out
   before hunting a medium.
2. **If the contracts hold, enumerate the shared medium.** Mutable state, a **namespace /
   counter**, a slot / store, **ordering**, the **build artifact** — the cardinality / ordering
   / cross-pass axes *are* this medium. Ask: **what accumulates, and where does it discharge?**
   (a desync building in a shared counter until two siblings collide).
3. **Fix by owning the medium.** Give the shared thing **one owned home** (a per-loop-keyed
   name instead of a global counter) so the channel can't carry contention, or make the
   coupling explicit. The stale-artifact trap is the same lens: the build, out of sync.

## Building a noisy check to clean? Ratchet it, don't churn

An analysis that starts noisy needs a *growing* instrument — a **ratchet**. Gate it off the
default path, pin its false-positive count as a one-way baseline, drive it **down** — never
write-measure-**revert** (exploration accumulates behind the flag; progress is a number). A
change that *raises* the count is a different approach: give it its **own** flag + baseline.
And **0 findings can be vacuous** — a check needs a positive control (an *injected* fault it
must flag), not just clean-on-correct.

## The tell (cheap, always-on sensor)

The robust version covers N cases with **one** mechanism, so it lands *short*; the brittle one
covers them with **N**, so it *accretes*. The bulk is the first thing you feel: **"this is longer
/ more than I expected for what it does."** Treat that as the alarm to *look* — before any
matrix, profile, or crash. It says **look, not shorten**: sometimes length is essential
(genuinely N families, or an invariant spelled out rather than hidden in cleverness). Finding the
missing invariant → shorter + robust; honestly finding none → accept the length.

Failing by **under-reach** (N mechanisms for one family) and by **over-reach** (one mechanism
forced over N families) are symmetric, and only the matrix or a falsifying probe tells them
apart — `design-protocol` § The two ways to fail.

## Has this already been written? — the sensor before you write

The tell fires on code you **wrote**; this one fires **before**: about to add a predicate, a
type list, a helper or a routine, ask whether it already exists — in another module, or in a
library the project can use. A duplicate is a decision **never made**: the other copy lives in a
file the task gave you no reason to open. Two moments carry the risk: **fixing at a site**
(adding the check *here too* — one rule, N drifting copies) and **porting or re-targeting**
(the constraint forbids *depending* on the original, not a licence to *re-derive* it). Answer it
with an instrument, not memory. Full treatment — what such a tool looks for, anchoring on named
rules, and the symmetric error of merging things merely equal *today* — `design-protocol`
§ Before you write it.

## The dual sensor — "does this notion have a second spelling?"

Its dual asks whether the thing you are about to MATCH arrives more than one way. A notion the
system treats as one — a projection, a null, a borrow, an identifier, a unit — can reach your
input in two representations, and a matcher keyed on one is blind to the other **silently**: the
missing form shares no token with the one you search for. So such a matcher is **right about
every case it can see**, and a passing suite is not evidence. Ask before writing the predicate —
*what else IS one of these?* — and prove it with the other representation, constructed by hand,
arriving where the matcher is not looking. With more than a couple of these, a short screen that
lists every site resolving the notion one way, and which also handle the other, ranks the
backlog.

## The blind-instrument tell — upgrade the eyes, don't guess

You have probed the same question two or three times and it will not resolve: the boundary is
**non-monotonic** ("sometimes it leaks"), the tool reports the *effect* but not its *cause*
("leaked — from where? the site reads `0`"), or its label misleads (a stale name that sends you
hunting the wrong object). That is a **blind instrument**; more probes spend time at the same
blind altitude. Ask *"why can't what I already have attribute this?"* and build **attribution**
(the sentinel's sibling, aimed at *data*): make each effect record its provenance — where a
store was allocated, which pass wrote a field, what produced a value — so a symptom points at
its cause with no inference.

Prefer upgrading the **system's own diagnostic** over a throwaway probe: it **compounds** — it
attributes this instance *and every future one*, is usually cheap, and so earns its keep even
when this bug alone would not justify a bespoke matrix (the one exception to *keep it light*).
Building it is a small design act, so hold it to the **calibration** bar: a provenance field
reading `0`/empty is vacuous — stamp it at the real chokepoint and prove it fires for a known
case. Stalled because you **cannot see** → fix the eyes; stalled because the **frame** is wrong
→ change altitude.

## When iterating stalls: change altitude — the pivot always exists

A few unresolving probes is fine. The rule bites at **non-convergence**: evidence piles up that
the *frame* is off (*actionable ≠ true*). Another **retry** stays at the wrong category's
altitude and **more breadth** spreads you thinner across it. **Plan B is depth on ONE case.**
Take the **simplest-looking** one and go all the way down — the IR, the emitted code, the exact
values, one variable's real lifetime — asking *is this really the category, or have I skated
over a detail?* One case understood **completely** decides rather than suggests, and the detail
**opens plans C, D, E** that were invisible while you guessed. Perseveration is the failure; a
block is only the signal to drop altitude.

## Keep it light — the procedure must not consume the judgment it serves

**Two layers, and only the second is expensive.** The always-on layer is free and reflexive:
the *tell* and the cheap disproving probe — reach for them every time. The full
build-the-instrument procedure (a complete matrix, a falsification suite, a sentinel with a
positive control) fires **only when the cheap layer says the thing is load-bearing** — never
reflexively per function; an ever-on checklist burns the attention the essential-vs-accidental
judgment needs. Writing the prediction or matrix down is a load *reducer*: it moves the prior
onto the page and frees working memory. **The cheap probe is never skipped; the full matrix is
never reflexive** (the exception: a compounding diagnostic, § The blind-instrument tell).

## Which mode am I in? (and they compose)

- **A discrepancy exists** — something crashes, corrupts, or answers wrong → **DEBUG.**
- **You are choosing behavior** — a new algorithm, a refactor, a data format, a public
  contract → **DESIGN** (`design-protocol`).
- **They compose.** A fix touching a load-bearing algorithm is both: the matrix finds the
  class, and the proportionate fix is a small design whose invariant you should be able to
  name. When the real fix is a substrate change *out of the current change's scope*, finish the
  localization and **route it** — a well-localized issue with the matrix attached — rather than
  force a patch the matrix says is too wide; a blind fix to a substrate you haven't earned the
  design for re-introduces the class you were removing.

## The residual (why this isn't enough on its own)

The instrument covers only axes you *know* to vary; the composition no probe imagined survives
any discipline. Walking a construct's real use cases before deciding (`design-protocol` § When
the design is a MEANING) moves many of those axes to design time; the rest need the second
engine, the **dogfood loop** — real consumers, not toys — which converts an unknown axis into a
known one, each harvested lesson widening the next matrix. This skill makes the *visible* axes
safe; the dogfood loop grows what is visible.

## Keep `git diff main` usable — one branch, rebase often

For a refactor the sharpest instrument is **`git diff main`**: it shows your delta and lets you
read the known-good baseline without touching your tree (`git show origin/main:f` — never `git
checkout … -- f`, which silently discards uncommitted work; undo a probe with the inverse edit,
and commit before anything that rewrites the working tree). It works only while you stay on
**one** branch held **close to main** — build everything there (mixed topics are fine) and
**rebase on `origin/main` often**. Per-topic branches, or a branch left to diverge, poison the
diff with merge noise and make overlapping work un-mergeable later (a squash upstream against
your original commits is a guaranteed conflict). Follow the repository's own branch and
pull-request policy.

## Go deeper — route here, don't reinvent

Everything above is **tree-agnostic**. The DESIGN half is the **`design-protocol` sibling
skill**: name the invariant, count the re-assertion sites, probe to falsify, build, validate
against the written prediction; its § The other half covers the constructive instrument for
exact domains. The DEBUG-method depth and worked references are repo-specific and live in the
bindings below; carrying this skill to another tree means repointing them at that tree's
debugging policy and test layout.

## In the loft tree (bindings — skip outside it)

- **DEBUG method** — `CLAUDE.md` § Debugging policy (matrix-first: the boundary
  matrix, `scripts/matrix_axes.py` for the axes you held fixed, the
  chokepoint-invariant rule, and read `doc/claude/formal/` FIRST when the fix has
  a choice in it).
- **What to vary in the matrix** — `doc/claude/plans/README.md` § The composition
  axes.
- **DEBUG mechanics (loft)** — the `loft-debug` skill: `LOFT_LOG` presets, dump
  files, `--interpret`-first seeing loop, native-env traps that fake failures.
- **The build outputs that lag independently** (calibration) — the debug rlib, the release
  rlib, the wasm rlib and the fixture cdylibs; `make check-rlib` is the pre-flight for the
  first, and the `code-quality-validator` agent names the rebuild for the others.
- **DESIGN reference** — `doc/claude/DESIGN_VERIFICATION.md` § C1: the six
  verification questions (name-the-invariant, consequence/cause ratio,
  cost-of-next-case, one-home-per-fact, subtraction-not-a-guard, matched-to-domain)
  and the countable brittleness form.
- **Heavyweight investigations** — `doc/claude/plans/_INVESTIGATION_TEMPLATE.md`.
- **The deep why** — `doc/claude/GOALS.md` § Stability trumps features: robustness by
  *subtraction* — remove the mechanism rather than guard it; the reason the short version is
  usually the robust one.  Goal E there is the "own the medium" step: one owned home for a
  shared thing.
- **The cheapest probe in loft** — `loft introspect prog.loft` (IR, bytecode and the native
  Rust in one capture), `--native-emit`, a `LOFT_LOG` preset; the `loft-debug` skill has the
  question-to-instrument table.
- **When the special case wants to become the normal case** — `doc/claude/STABILITY_METHOD.md`
  § The trigger: the full story of evolving the representation.
- **Branches here** — one hostname-prefixed branch held close to `main`; never create one
  without the owner's ask; with a PR open on it, push nothing to it and stack further work on
  `<host>-pr<N>-next`, rebasing onto `main` after the merge (CLAUDE.md § Branch policy, rules
  3–5; `doc/claude/DEVELOPMENT.md` § "Stay close to main — rebase rigorously"; a rebase onto a
  squash: `doc/claude/JOINING.md`).
