---
name: engineering-rigor
description: >-
  The shared discipline for any non-trivial change — fixing a bug OR
  designing/refactoring a load-bearing algorithm. USE THIS when about to fix a
  non-trivial bug (crash, silent corruption, wrong result), design or refactor a
  load-bearing algorithm (core representation, runtime, memory, codegen, a public
  contract or data format), when a change feels fragile or "longer than expected
  for what it does", or when reaching for an approximation in an exact-invariant
  domain (geometry, caching, hashing, round-trips) and can't pin the exact
  construction — ESPECIALLY when a fix already looks obvious (that's the cue, not
  the exception). It is a SPEED tool, not a brake: a coherent explanation is a
  hypothesis, not a conclusion, and the fastest route to a fix that sticks is the
  cheapest probe that could prove your idea wrong (a timing, a grep of a count, one
  --emit-and-look — seconds). So the moment you can state a cause or a fix, that
  fluency is the cue to GATHER first, not act — which also lets you fix the SOURCE
  and delete the whole class, not just this instance. Then build the instrument
  that makes the CLASS visible (boundary matrix, falsification probe, usage
  sentinel), find the ONE invariant, enforce it at the chokepoint (no narrower, no
  wider), verify against what you wrote down. And when a diagnostic reports a
  symptom but not its cause, or repeated probes won't converge (non-monotonic,
  "sometimes it happens"), treat that as a blind instrument, not a hard bug —
  upgrade the tool to ATTRIBUTE the effect to its cause before guessing at a fix.
  ALSO applies BEFORE you write: a predicate, type list, helper or routine that may
  ALREADY EXIST elsewhere in this project or in a library it can use — and especially
  when PORTING or RE-TARGETING something that already works, which does not feel like
  new work. A duplicate is a decision NEVER MADE rather than one made badly, so more
  care cannot find it; an instrument can.
  Routes to the matrix-first protocol and Design Protocol 1.
user-invocable: true
---

# Engineering Rigor — See the Class Before You Act

## The one idea

**A coherent explanation is a hypothesis, not a conclusion — most of all an
elegant one.** Whether you are fixing a bug or designing a system, the failure is
the same: you act on the first story that fits, and that story was built from the
*one instance in front of you*. It is correct for that instance and silent about
the cases you could not see — so it breaks on them.

This is a **sight discipline, not a willpower one.** You do not ship fragility on
purpose. You ship it because a single instance cannot show you the class it
belongs to. So the cure is never *"be more careful"* (a values lever — it does
nothing for a sight gap). The cure is **better eyes**: build the instrument that
makes the whole class visible, then act on what it shows. The instrument is the
organ of sight; the rest of this skill is how to build it and what to do once you
can see.

## This gets you to a fix FAST — the cheap probe is the accelerator, not the brake

Read this as a **speed tool.** The fastest route to a fix that's actually right is
**the cheapest thing that could prove your idea wrong** — a single timing, a `grep`
of a count, one `--emit`-and-look, a cold/warm run. *Seconds.* Reach for it like a
cherry: it's right there, it's cheap, and it hands you **certainty** — which is what
lets you fix once and move on.

A guessed fix only *feels* faster. A story you can describe but haven't probed is a
hypothesis you've mistaken for a conclusion, and the wrong ones cost the real time —
the rework, the redone PR, the CI round, the spent trust. **The 30-second probe is
the fast lane precisely because being wrong is the slow one.**

And the **bigger cherry:** because the instrument shows you the *class*, you fix the
**source** — the chokepoint the whole family flows through — not the instance. You
don't fix *this* bug; you **delete the category**, so the siblings and the future
runs that would have hit the same root never come back. A guessed instance-fix
leaves the source intact and the siblings keep arriving. Two prizes for a few
seconds of looking: a fix that sticks, and a class of bugs that stops happening.

> **You can state a fix → that's the cue to grab the cheapest thing that could kill
> it.** Run that first; reaching for the probe *is* reaching for the fix.

This first cheap probe is the always-on move, not the heavier build-the-instrument
work below. Skipping it is the most common way rigor fails in practice — not a
botched step, but never reaching for the instrument because a fix was already in
hand.

## Two modes, one shape

The discipline has two faces depending on whether you are *explaining behavior
that exists* or *choosing behavior that doesn't yet*. They are the **same shape
one level apart** — learn the column you're in, but know it's one method.

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

1. **Don't act on the first read.** A coherent story — especially an elegant
   one-line fix or a clean unifying abstraction — is the *signal you haven't
   earned the action yet*. Real bugs are complex-variant; real designs carry
   weight you haven't probed. The clean story is usually the part of the picture
   you haven't looked at.

2. **Build the instrument — it is your eyes.** Debug: a boundary matrix of `/tmp`
   probes, varying **one axis per probe** (type-kind, construction-path, depth,
   null, backend — and, when the fault is in the *transform* rather than the
   value: **cardinality** (N of a thing coexisting), **ordering/position**,
   **cross-pass consistency** — the composition axes). Design: the cheapest test that could
   *falsify* each load-bearing claim — a probe, a targeted read, one boundary
   case. Control-flow (*which code actually runs*): a **usage sentinel** — route
   the uses through one chokepoint and make it loud (its own § below). Provenance
   (*where did this effect come from?* — a leak, a wrong field, a bad value with no
   named producer): an **attribution** instrument — stamp the cause into the
   system's own diagnostics (§ The blind-instrument tell).
   *"I can't see the root / can't name the invariant yet"* means **the
   instrument isn't finished** — never a license to act on the one case in hand.
   When you cannot form a candidate invariant *at all* and the domain is
   **exact** (geometry, caching, hashing, store/memory lifetime, round-trips — the
   design is a single construction to *recover*, not a space to explore), the
   unfinished instrument is **constructive, not falsifying**: there is no claim to
   break, so plot a concrete instance of the *answer* (the exact target
   output/state) — or build the cheapest thing that *produces* it — and **read the
   invariant off it**. Mind the symmetry with step 3: an instance of the
   **problem/symptom** is the trap (it hides its class); an instance of the
   **answer/end-result** is the instrument (you read the class *off* it). (§ *The
   other half* in the `design-protocol` sibling skill.)

3. **The truth is visible in the class, invisible in the instance.** The shared
   mechanism behind a family of "different" symptoms shows up *in the matrix* and
   in no single repro. The false unification in a design ("…and it also handles
   X") shows up *under a probe* and in no re-reading of the prose. This is *why*
   step 2 is non-negotiable: the thing you're hunting is a property of the class,
   so you must be able to see the class. **An irreducible minimal repro is itself
   a reading:** when you cannot shrink below *N coexisting* constructs (N loops, N
   declarations, N allocations), stop shrinking — the bug *is* the coexistence (a
   resource / slot / store / accumulation fault), not any one instance, and
   chasing a one-instance repro there is the matrix aimed down the wrong axis.

4. **Find the ONE invariant; enforce it at the chokepoint — no narrower, no
   wider.** If the project keeps a named-rule register or formal spec, look the
   invariant up there BEFORE deriving it — a rule already written settles the
   question, and the citation is cheaper than the re-derivation.
   *Narrower* (a per-case/per-type patch; an N-way spray) leaves the
   siblings broken — the same problem, unfinished. *Wider* (re-resolving more
   than the failing region; unifying genuinely distinct cases under a false
   invariant) drags blast radius and is its own brittleness. The proportionate
   move enforces *exactly* the invariant the whole failing region violates.
   *The chokepoint is often a **data structure**, not a code path.* When the fix
   keeps wanting another condition / branch / special-case to re-derive a fact — or
   you are simply **stuck, the change wanting more tests than it should** — that
   thicket is the signal the *structure* is wrong, not your logic (sight, not
   willpower: suspect the structure, don't redouble effort). Fold the fact INTO the
   structure so the special case becomes the normal case (Linus's "good taste";
   "data dominates") — every site then *reads* it instead of re-deriving. You can't
   design the perfect structure upfront, so this is recurring, not a one-time win:
   recognize it, signal it, evolve. Full story:
   [STABILITY_METHOD.md § The trigger](../../../doc/claude/STABILITY_METHOD.md).

5. **Verify against what you wrote down**, not against the action's own momentum.
   Debug: re-run the full matrix on **every mode the result can diverge across**
   — for a system with more than one execution path (e.g. an interpreter and a
   compiler), each path, because cross-mode divergence is real. Design: compare
   the built thing to the **written prediction**. An external commitment is what
   makes the check honest — without it you grade your own homework.

## A third instrument: the usage sentinel ("which code actually runs?")

The matrix varies **inputs** to make a behavior-class visible. Some questions aren't
about inputs — they're about **control flow**: which code actually runs, which
sites route through a point. There the obvious instrument lies: a **static count**
(`grep` the call sites) **over-counts** — it sees dead and live uses with the same
eyes, so it can't say which still *fire*.

What can is a **usage sentinel** — route every use through one observable chokepoint
and make it loud (count it + name the caller, or trip under a flag). One run turns a
control-flow *guess* into a runtime *fact*: the chokepoint rule (step 4) aimed at
**observability**, not enforcement. The same fact answers three questions — removal is
only one:

| Use | Sentinel question | The answer you want |
|---|---|---|
| **Removal** | is anything still using this? | **none** — zero across the whole suite → safe to delete |
| **Debug** | does the path I *think* runs actually run? | **the expected sites fire** — catches a fast path that's never taken, a handler that's secretly dead |
| **Design** | is my chokepoint the *sole* route every site takes? | **all sites, only here** — the runtime form of one-home-per-fact |

**Silence is evidence only after a positive control.** A silent sentinel reads the
same whether the consumer is dead *or the sentinel sits on a dead path* — so prove it
*can* fire first. Removal gets this free (the whole-suite run is the control — if
anything used it, you'd see it); debug and design must show the chokepoint fires for a
known-live case before trusting a zero. Skip it and you call a handler "dead code"
when you've only proven "my probe never reached it."

This is one face of a wider law: **the instrument is part of the system, so validate
it before you trust its reading.** Two properties must hold, and each fails silently
as a "bug" that isn't one:

- **Calibration** — *can it report the truth for this question?* A dead-path sentinel
  can't (above); neither can a **stale build artifact** — a matrix cell reflects the
  *binary you ran*, not the source you edited, and a self-hosted compiler links
  several lenses (debug rlib, release rlib, wasm rlib, fixture cdylibs) that can each
  lag the source independently. So **a surprising cell is a stale-artifact suspect
  before it is a bug**: rebuild the lens behind that cell and re-read before believing
  it. (Step 5's cross-mode verify is this property as a precondition — each mode is a
  separate lens that must be in focus on its own.)
- **Non-intrusiveness** — *does observing change the result?* For codegen / slot /
  layout / aliasing faults it does: adding a `print`, a temporary, or one more
  reference shifts the codegen and *moves* the bug. When an added observation flips
  the outcome, that perturbation **is** the boundary — you've localized to
  codegen-sensitivity, and bisecting *what minimal edit flips it* often names the
  mechanism. Then read from **outside the run** (IR / bytecode / trace dumps, the
  variable table, the generated source), never an in-program probe that re-runs the
  compiler you are trying to inspect.

## A fourth lens: when verified parts fail *together* — read the shared medium

The matrix varies inputs to one part; the sentinel traces one control path. A third
shape is neither: each part **passes alone** yet they **corrupt each other in
composition**. Re-auditing the parts is wasted motion — they are locally correct; the
bug is in the **medium they share**, which sits in no part's contract.

The cause is structural: **verification is local and instantaneous; a real system is
embedded and historical.** A part is checked against its own spec at a moment, but it
sits in a shared medium and carries accumulated state — and local-instantaneous
checking is blind to embedded-historical coupling. The composition failure is that
hidden channel finally becoming visible. (Physical engineering lives this: separately
sound components fail the next one through shared air pressure, shock, static buildup,
or a tear propagating through shared material — the channel no datasheet lists.)

So when isolated-correct parts fail together:

1. **Read the boundary contract first.** A mismatch *there* — one part's output vs the
   next's expected input, each verified against a different spec — is composition
   failure you can **audit**; it is not hidden. Rule it out before hunting a medium.
2. **If the contracts hold, enumerate the shared medium — don't re-audit the parts.**
   The channels: mutable state, a **namespace / counter**, a slot / store,
   **ordering**, the **build artifact** — the cardinality / ordering / cross-pass axes
   *are* this medium. Then ask the conserved-quantity question: **what accumulates,
   and where does it discharge?** (a desync building in a shared counter until two
   siblings collide is the software form of charge building until it arcs).
3. **Fix by owning the medium.** Every engineering field manages the medium, not the
   parts — a ground strap, a damper, pressure equalization. The software form is
   Goal E (`doc/claude/GOALS.md`): give the shared thing **one owned home** (a
   per-loop-keyed name instead of a global counter) so the channel can't carry
   contention — or make the coupling explicit. The stale-artifact trap above is the
   same lens: a shared medium (the build) out of sync, coupling source and test.

## Building a noisy check to clean? Ratchet it, don't churn

The matrix and sentinel are *static* instruments; constructing an analysis/check that starts noisy
needs a *growing* one — a **ratchet**. Gate it off the default path, pin its false-positive count as a
one-way baseline, drive it **down** — never write-measure-**revert** (exploration accumulates behind
the flag; progress is a number). A change that *raises* the count is a different, harder approach:
give it its **own** flag + baseline, don't revert the clean tier. And **0 findings can be vacuous** —
a check needs a positive control (an *injected* fault it must flag), not just clean-on-correct.

## The two ways to fail (symmetric — both modes share them)

| Failure | What it is | Caught by |
|---|---|---|
| **Under-reach** | N mechanisms for what is really **one** family (a spray, a patch per case) — *or* the alarm fired and lost to momentum | counting the re-assertion sites; the matrix showing the sibling cases |
| **Over-reach** | **one** mechanism forced over genuinely **N** families — a false invariant that breaks when the cases assert their differences | a probe that tries to *falsify* the unifying claim |

The instrument is what **distinguishes** them: it tells you whether the cases
actually share an invariant. Under-reach is the failure of the lazy; over-reach
is the failure of the clever. Both are brittleness; both are invisible without
the matrix/probe.

## The tell (cheap, always-on sensor)

The robust version covers N cases with **one** mechanism, so it lands *short*. The
brittle version covers them with **N** mechanisms, so it *accretes* — and the bulk
is the first thing you feel: **"this is longer / more than I expected for what it
does."** Treat that as the alarm to *look* — before any matrix, profile, or crash.

But the tell says **look, not shorten.** Sometimes length is *essential* (genuinely
N families, or an invariant spelled out so it's explicit rather than hidden in
cleverness). The tell triggers the search for a missing invariant; finding one →
shorter + robust; honestly finding none → the length is essential, accept it. The
alarm did its job by making you check.

## The second always-on sensor — "has this already been written?"

*The tell* above fires on code you have **written**. This one fires **before** you write
it: the moment you are about to add a predicate, a type list, a helper, or a routine, ask
whether it already exists — in another module here, or in a library the project can
already use.

It costs seconds, and it is not something care can replace. A duplicate implementation is
rarely a decision made badly; it is a decision **never made** — the other copy lives in a
file the current task gave you no reason to open, so the question never arose. Care
operates on what is in front of you, and the second copy is precisely what is not.

Two moments carry most of the risk, and neither of them feels like design:

- **fixing at a site** — adding the check *here too*, rather than asking where the check
  lives. That is how one rule ends up with N implementations that then drift apart, and a
  drifted copy is worse than a copy: it looks like the thing it is not.
- **porting or re-targeting** — a new backend, an output format, a self-contained build.
  It reads as *moving* something that already exists, so *does this exist?* feels already
  answered. Notice what the constraint actually forbids: it forbids **depending** on the
  original, which is not a licence to **re-derive** it.

Answer it with an instrument rather than memory — and if the project has none, the cheap
one is minutes of scripting. The full treatment, including what such a tool looks for, how
to anchor the question on named rules once the system outgrows one head, and the symmetric
error of merging two things that are merely equal *today*, is the **`design-protocol`
sibling skill**, § *Before you write it*.

## The dual sensor — "does this notion have a second spelling?"

The sensor above asks whether the thing you are about to write already exists. Its dual asks
whether the thing you are about to MATCH arrives more than one way. A notion the system treats as
one — a projection, a null, a borrow, an identifier, a unit — can reach your input in two
representations, and a matcher keyed on one is blind to the other **silently**: the missing
representation shares no token with the one you search for, so the blindness cannot be found from
the symptom. Searching for what you DO match returns every site that gets it right.

The normal appearance of it is therefore a matcher that is **right about every case it can see**,
which is why care does not catch it and why a passing suite is not evidence. The check is a
question before you write the predicate — *what else IS one of these?* — and the proof is the
other representation, constructed by hand, arriving where the matcher is not looking.

Where a project has more than a couple of these, the answer belongs in an instrument rather than
in attention: a screen that lists every site resolving the notion one way and says which of them
also handle the other. That is a short script, and it ranks a backlog that otherwise reads as
uniformly suspect.

## The blind-instrument tell — upgrade the eyes, don't guess

*The tell* above fires on your **code**; this one fires on your **instrument**. You
have probed the same question two or three times and it will not resolve: the boundary
is **non-monotonic** ("sometimes it leaks, sometimes not"), or the tool reports the
*effect* but not its *cause* ("leaked — from where? the site reads `0`"), or the label
it hands you is misleading (a symptom tagged with a stale name that sends you hunting
the wrong object). That is not a hard bug — it is a **blind instrument**, and staring
harder or spraying more probes only spends time at the same blind altitude.

The reflex it triggers is a question, not a probe: *"why can't what I already have
attribute this?"* — then build the missing eyes. The instrument here is **attribution**
(the sentinel's sibling, aimed at *data* instead of control): make each effect record
its own provenance — where a store was allocated, which pass wrote a field, what
produced a value — so a symptom points back at its cause with **no inference**. Prefer
upgrading the **system's own diagnostic** over a throwaway probe: that upgrade
**compounds** — it attributes this instance *and every future one* — so it earns its
keep even when this one bug wouldn't justify a bespoke matrix (the exception to *keep it
light* below).

Two guards. Building it is a small **design** act (§ Which mode am I in — you have
crossed into DESIGN), so hold it to the **calibration** bar (§ the third instrument): a
provenance field that reads `0`/empty is vacuous, not truth — stamp it at the real
allocation/write chokepoint and prove it fires for a known case before you trust a
blank. And distinguish this from *change altitude* below: stalled because the **frame**
is wrong → go deep on one case; stalled because you **cannot see** → fix the eyes. Ask
whether a clearer reading would decide it — if yes, you are blind, so build the reading
first.

## When iterating stalls: change altitude — the pivot always exists

The tell says *look*; this says **where** once looking-broad has stalled. A few
unresolving probes is fine — iterate freely. The rule bites only at
**non-convergence**: you're retrying and the evidence is piling up that the *frame*
is off (*actionable ≠ true* — a category becoming useful is not it becoming
correct). Both instinctive moves fail here for one reason — another **retry** stays
at the wrong category's altitude, and **more breadth** (extra matrix cells) spreads
you thinner across that same shallow level; neither drops to the facts the category
summarized.

**Plan B is depth on ONE case.** Take the **simplest-looking** one and go all the
way down — the IR, the emitted code, the exact values, one variable's real lifetime
— asking: *is this really the category, or have I skated over a detail?* Simple =
least noise, so the missing detail is visible; one case understood **completely**
decides rather than suggests.

It is the highest-leverage move when stuck, and it **compounds**: the detail doesn't
just settle this case, it **opens plans C, D, E** — the real mechanism, once seen,
generates approaches that were invisible while you guessed. So a good engineer knows
*when* to pivot and trusts **a way out always exists — you just have to look for
it.** Perseveration (retrying plan A past its stall) is the failure; a block is only
the signal to drop altitude. Sight, not willpower: stuck means you can't *see* the
case yet — go see one.

## Keep it light — the procedure must not consume the judgment it serves

There are **two layers, and only the second is expensive.** The always-on layer is
free and reflexive: the *tell* (this is longer than I expected) **and the cheap
disproving probe** (you can state a fix → grab the seconds-long thing that could
kill it). Both fire constantly, cost almost nothing, and are the accelerator
above — reach for them every time. The *full* build-the-instrument procedure (a
complete boundary matrix; a falsification suite; a usage sentinel with a positive
control) is the expensive part, and it fires **only when the cheap layer says the
thing is load-bearing** — never reflexively per function. An ever-on full
checklist is friction that burns the very attention the essential-vs-accidental
judgment needs. Writing the prediction / matrix down is a load *reducer*: it moves
the prior onto the page so working memory is free for the building and the
judgment. So: **the cheap probe is never skipped; the full matrix is never
reflexive.** The one exception to "expensive": an instrument that **compounds** —
upgrading a production diagnostic to attribute effects to their cause (§ The
blind-instrument tell) — is usually cheap to add and pays off on *every* future
instance, so it is worth building even when this single bug wouldn't justify a
bespoke matrix.

## Which mode am I in? (and they compose)

- **A discrepancy exists** — something crashes, corrupts, or returns the wrong
  answer → **DEBUG.** You are explaining behavior.
- **You are choosing behavior** — a new algorithm, a refactor, a data format, a
  public contract → **DESIGN.** You are committing to an invariant.
- **They compose.** A bug fix that touches a load-bearing algorithm is *both*: the
  matrix finds the class, and the proportionate fix is itself a small design whose
  invariant you should be able to name. When an investigation reveals the real fix
  is a substrate change, you've crossed from debug into design — pick up the design
  column without dropping the matrix. And when that substrate change is *out of the
  current change's scope* (a store-lifetime or representation rework), rigor is to
  **finish the localization and route it** — a well-localized issue with the matrix
  attached — not to force a patch the matrix says is too wide; a blind fix to a
  substrate you haven't earned the design for re-introduces the very class you were
  removing.

## The residual (why this isn't enough on its own)

The instrument only covers axes you *know* to vary. The composition no probe
imagined survives any discipline; only real use reveals it. That is the second
engine: the **dogfood loop** (real consumers, not toys) converts an unknown axis
into a known one, and each harvested lesson widens the next matrix. This skill
makes the *visible* axes safe; the dogfood loop grows what is visible.

## Keep `git diff main` usable — one branch, rebase often

For a refactor the sharpest instrument is **`git diff main`**: it shows your delta (am I
going the right direction?) and lets you read the known-good baseline without
touching your tree (`git show origin/main:f` — never `git checkout … -- f`, which
silently discards uncommitted work; undo a probe with the inverse edit, and commit
before anything that rewrites the working tree). That instrument only works
while you stay on **one** branch held **close to main** — build everything there (mixed
topics are fine) and **rebase on `origin/main` often**, not once at the end (with review in
flight on the branch — an open PR — coordinate before rebasing or force-pushing). Per-topic
branches, or a branch left to diverge, poison the diff with merge noise AND make overlapping
work un-mergeable later: a squash upstream vs your original commits is a guaranteed conflict,
then a multi-commit rebase from hell. Do not create a branch unless the user explicitly asks;
if the branch has drifted from main, rebase before continuing. (Full rationale + cautionary
tale: `doc/claude/DEVELOPMENT.md` § "Stay close to main — rebase rigorously".)

## Go deeper — route here, don't reinvent

This skill is the **synthesis + the router**, and everything above is
**tree-agnostic — the discipline is identical in any codebase.**

**The DESIGN half is its own skill — universal, not repo-specific.** When a design
is load-bearing, run the **`design-protocol` sibling skill** (Design Protocol 1 — A
Design Is a Testable Hypothesis): name the invariant, count the re-assertion sites,
probe to falsify, build, validate against the written prediction. Its **§ The other
half** covers the move one step earlier — when you cannot form a candidate invariant
at all (an exact-invariant domain): the constructive instrument, plot the
answer-instance and read the invariant off it. The skill is self-contained and
tree-agnostic; the full DESIGN depth (evidence, the two failure modes, worked
examples) lives there.

**The DEBUG-method depth and the worked references are repo-specific.** The routes
below point into *this* repository (loft); carrying this skill into another tree
(e.g. loft2) means keeping the body + the `design-protocol` sibling unchanged and
repointing these links at that tree's equivalents — its debugging policy, its test
layout.

- **DEBUG method** — `CLAUDE.md` § Debugging policy (matrix-first: the boundary
  matrix, `scripts/matrix_axes.py` for the axes you held fixed, the
  chokepoint-invariant rule, and read `doc/claude/formal/` FIRST when the fix has
  a choice in it).
- **What to vary in the matrix** — `doc/claude/plans/README.md` § The composition
  axes.
- **DEBUG mechanics (loft)** — the `loft-debug` skill: `LOFT_LOG` presets, dump
  files, `--interpret`-first seeing loop, native-env traps that fake failures.
- **DESIGN reference** — `doc/claude/DESIGN_VERIFICATION.md` § C1: the six
  verification questions (name-the-invariant, consequence/cause ratio,
  cost-of-next-case, one-home-per-fact, subtraction-not-a-guard, matched-to-domain)
  and the countable brittleness form.
- **Heavyweight investigations** — `doc/claude/plans/_INVESTIGATION_TEMPLATE.md`.
- **The deep why** — `doc/claude/GOALS.md` Goal E: robustness by *subtraction* —
  the reason the short version is usually the robust one.
