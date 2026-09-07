---
name: design-protocol
description: >-
  Design Protocol 1 — a design is a testable HYPOTHESIS about an invariant, not a
  plan you execute. The DESIGN-mode counterpart to the engineering-rigor skill
  (which routes here for the design half). USE THIS whenever you are about to commit
  to a load-bearing design — a core representation, runtime, memory model, codegen, a
  public contract, a data format, or any algorithm that will carry weight: name the
  ONE invariant, count its re-assertion sites, build the cheapest probe that could
  FALSIFY each load-bearing claim, then build and validate against the written
  prediction. And for exact-invariant domains where you cannot even form the
  invariant (geometry, caching, hashing, store/memory lifetime, round-trips), flip to
  the constructive instrument — plot a concrete instance of the ANSWER and read the
  invariant off it. Reach for it when a design feels too clean, when you keep reaching
  for an approximation, or whenever you catch yourself about to build the first
  coherent design without probing it. ALSO reach for it BEFORE writing any routine,
  predicate, helper, or type-list that might already exist somewhere else — in another
  module of this project, or in a library or package the project can already use — and
  especially when PORTING or RE-TARGETING something that already works (a new backend, a
  new output format, a standalone or single-file version, a different language). That
  case does not feel like new work, which is exactly why it is where duplicate and
  subtly-wrong reimplementations come from. The skill covers how to find the existing
  implementation before you write a second one, and — as a system grows past what fits in
  one head — how to anchor that question on NAMED RULES cited at each enforcement site,
  so "does this already exist?" becomes a lookup instead of a search, and so two sites
  that implement one rule are distinguishable from two that merely look alike today.
user-invocable: true
---

# Design Protocol 1 — A Design Is a Testable Hypothesis

> **Transferable** — written so an agent in any codebase can read it cold and apply
> it. The examples are evidence, not prerequisites: skip them and the procedure still
> stands.

---

## The thesis

A design is **not a plan you execute**. It is a **hypothesis about an invariant** — a
claim that some single rule makes every case (including the ones you never tested)
behave correctly *for the same reason*. A hypothesis can be **wrong**, and you can
**test it** — the same epistemics as a bug's root-cause hypothesis, one level up:

| Bug | Design |
|---|---|
| has a *root-cause hypothesis* | has an *invariant hypothesis* |
| tested with a **boundary matrix** (probe the cells) | tested with **probes on its claims** (try to falsify each) |
| "I can't see the root" = "the matrix isn't finished" | "I can't name the invariant" = "it isn't a design yet" |
| the **fix** is the proof | the **build** is the proof |

The failure this prevents is **shipping a design as a confident assertion** — a story
that reads correct in prose and breaks at the first case the prose didn't look at.

## Why a probe, not more care

The generative capability is real — a coherent design, correct load-bearing
predictions. Its **characteristic failure is over-reach**: making the design
*cleaner / bigger / more unifying* than the domain warrants — and that failure is
**invisible to introspection**, because it presents as *elegance*. So brittleness here
is a **sight failure, not a values failure**: you are not *choosing* fragility, you
genuinely **cannot see** the false absorption from the desk. The cure is therefore
**not "more care"** (a values lever — it does nothing for a sight gap) but **better
eyes**: a test that can falsify the claim. *The design-probe is to a design what the
boundary matrix is to a bug.*

Earned the hard way. A real design's invariant was framed plausibly but **wrong**,
and it "also subsumed" an unrelated bug it actually didn't — both read as correct in
prose; re-reading caught neither, only probing the specific claims did. And the
inverse: a design shipped *without* probing, whose own plan said *"one chokepoint that
18 sites route through"* — a spray wearing the word **chokepoint** — shipped as 18
bypasses and a silent mismatch.

---

## The other half — when you cannot form the invariant at all

Over-reach is the failure of a design you **already have** — a candidate invariant,
too clean, that a falsifying probe can break. It assumes the generative step has
*succeeded*. There is a symmetric failure one move earlier, where it has **not**: you
cannot state a candidate invariant at all. Step 1 below names this state — *"a pile of
cases, not a design yet"* — but treats it as a stop sign. On one class of problem it
is the **main event**.

**Exact-invariant domains.** In geometry, caching / serialization, hashing, store /
memory lifetime, and protocol round-trips, the correct design is not an open space to
explore — it is a single **construction that already exists** and must be
**recovered**. There the characteristic failure is not over-reach but **failing to
generate the right candidate**: the mind reaches for an **approximation** (geometry —
fit a line, bucket the angles, smooth the wobble) or **chases the symptom** (a store
desync surfacing as three different corruptions). Both are *iterating toward an answer
that already exists exactly* — motion that never converges, because the target is a
point, not a region.

**Same epistemics, opposite instrument.** This is still sight, not willpower — you
cannot *will* the invariant into view, and "think harder" does nothing for the gap.
But the instrument here is **constructive, not falsifying**: a falsifying probe needs a
claim to break, and you have none. The generative instrument is to **plot a concrete
instance of the *answer*** — the exact target output / state for one specific input —
or build the **cheapest thing that *produces* it** (a throwaway prototype, a
round-trip test), then **read the invariant off it.** The construction you could not
name in the abstract is legible in one worked instance.

- *Caching.* A startup-cache desync was symptom-chased across manifestations until the
  **target store state after a round-trip** was plotted concretely — the invariant then
  read out at once (*the round-trip must reproduce every binding identically*, the
  synthetic wrappers included). A one-line fix behind a large discovery cost.
- *Geometry.* A grid wall-straightener resisted eight approximations (line-fit,
  angle-buckets) against an *"exact, not an approximation"* spec, until the construction
  was drawn as a **plotted end-result** (subdivide each cell into triangles; keep the
  band the boundary cuts, drop the full-in / full-out ones). Only pinpointable after a
  throwaway prototype *produced* it — then ported unchanged.

**When the answer already EXISTS — capture its artifact and diff.** The cheapest
"plot the answer" is a **working sibling that already produces the construction** for a
near-identical case. Then you neither prototype nor theorise: **capture the sibling's
artifact and your broken one's — into files, not glances — and `diff`.** The residual
divergence *is* the construction to recover; you read it **off the diff**, not out of
your head. Three captures sharpen it: BEFORE (your change off) → NOW (your change)
confirms you changed what you meant; NOW → PROVEN-sibling isolates the bug. The failure
this prevents is **symptom-chasing a near-miss**: holding a theory of the difference
*without* the capture, patching one manifestation, and finding the next — motion near a
point it never lands on. *Earned.* A borrowed store-delivery corrupted only under
allocation pressure; three theories held without a capture — a stride-type, a missing
wrapper op, a lifetime flag — were each wrong. Capturing the proven sibling (a simpler
shape that survives the same stress) beside the broken output showed the one real
difference in a line: the proven path delivered into a **canonical owned buffer**, the
broken one reused an incidental local **as** the buffer — a dep the lifetime analysis
never tracked as the return, so a later allocation reused its slot. The captured diff
landed on the point the three patches kept circling. (For a captured-artifact domain —
IR, serialized bytes, store state, protocol frames — this beats prototyping: the answer
is already running; make yours byte-equal to it.)

**A sharp distinction from the bug side.** The matrix law is *"the truth is in the
class, invisible in the instance"* — beware acting on one instance. This does **not**
contradict "plot one concrete instance," because the instances are opposites: an
instance of the **problem / symptom** is the trap (it hides the class it belongs to);
an instance of the **answer / end-result** is the instrument (you read the invariant —
and therefore the class — *off* it). One misleads, the other generates.

**Where this sits.** It runs *before* step 3's falsification and moves step 5's build
*forward*: for an exact-invariant domain the build is not the last probe but the
**first, generative** act — the construction is named *by* building the cheapest
version, that invariant is then probed (steps 3–4), and only then is the real one
built. The cheap prototype is where the design gets **pinned**, not merely confirmed.

---

## Write the doc before the code — the failure-paths are where the invariant surfaces

The design doc is **not** after-the-fact write-up. Writing it is the **generative act
that surfaces the invariant and the requirements** — cheaply, before code commits you to
an unstated one. Enumerating the **failure paths** (every way it breaks) and naming
**what must hold across the boundary** (the requirements) is exactly where the
load-bearing invariant becomes *nameable* (step 1 below). Code-first skips that
enumeration: you silently adopt the **narrowest invariant you happened to see** and then
discover the real one by **whack-a-mole** — each patch fixes one case and reveals the
next the prose never listed. The doc is the cheapest place to find the right invariant;
code is the most expensive place to discover it was the wrong one.

Earned. A native-link `StableCrateId` collision was "fixed" by **build-identity keying**
— rebuild the package so its shared deps *match* the consumer's — coded before the design
was written down. It fixed `libloading` (a flags mismatch), then `log` collided (a
*profile* mismatch), and the next would have been build-script reproducibility: a symptom
chased one collision at a time. The load-bearing invariant — *keep the **API identity**
(the C-ABI contract) separate from the **Rust part** (the package's private internals),
so the latter never enters the consumer's link* — only became visible once the **failure
paths were written down** and the two identities were named on the page. *Matching* is
whack-a-mole; the doc made *separation* legible, and writing it first would have skipped
the dead-end build entirely.

So for a load-bearing design: **write the doc first** — the failure paths, the
cross-boundary requirements, the candidate invariant — *then* code. Not a full spec; the
minimum that forces you to enumerate how it breaks and what must hold. The enumeration is
the instrument: the class you cannot see from the desk is legible the moment you are
forced to list its members. (This sits *before* step 1 — it is how you reach a nameable
invariant — and complements *the other half*: when you cannot state the invariant, plot a
concrete instance of the answer *or* write the failure paths down; both make the
construction legible.)

---

## The protocol

Run it when a design is **load-bearing** — an algorithm that will carry weight (a core
representation, runtime, memory model, codegen, a public contract) — or when something
*feels* fragile, **or when the domain is exact-invariant** (a construction to *recover*,
not a space to explore — see *The other half*). Not reflexively (see *Keep it light*).

1. **State the design as one invariant.** One sentence: *under what single rule does a
   case you never tested behave correctly, for the same reason the tested ones do?* If
   you cannot name it, it is not a design yet — it is a pile of cases; on an
   exact-invariant domain the way out is **generative, not more thinking** — plot a
   concrete instance of the *answer* and read the invariant off it. Naming it is the
   difference between a matrix that is *confirmatory* (evidence an invariant holds) and
   one that is *constitutive* (the cells are the only reason it works).

2. **Count the re-assertion sites — the prospective tell.** How many independent sites
   must re-state the invariant for the design to be correct? If the answer is **N > 1
   and omitting it at a site is silent** (a wrong result, not a compile error), **N is
   the brittleness, known now, before any code.** "One chokepoint that 18 sites route
   through" is a self-contradiction; the prediction has already failed. Two cures,
   attacking the two factors: **collapse N toward 1** (a real chokepoint — one place
   every path consults), or **make omission loud** (a type or guard that turns
   forgetting into a compile error). Drive `N × silence` toward zero.

3. **Probe each load-bearing claim — try to falsify it.** For every claim the design
   rests on ("X is subtractable", "the buffer is Y", "this also handles Z"), write the
   **cheapest test that could prove it false** — a throwaway probe, a targeted code
   read, one boundary case. *Expect to falsify.* Re-reading the prose is not a test;
   the prose is where the error hides.

4. **Attack your cleanest claim specifically (the over-unification guard).** The most
   dangerous error is the *elegant absorption* of a case that is not really in the
   family — the failure mode that presents as success. Every "…and it also handles X"
   is a claim to falsify, not a bonus to celebrate. Compressing genuinely-distinct
   cases under a false invariant is itself brittleness (*wider than the domain*); it
   breaks when the cases assert their real differences.

5. **Build it (new code) or read it fully (old code).** Either way you now have the
   **actual** shape to compare against the prediction. (Auditing an existing subsystem
   is the same protocol: predict what a *cohesive* version would cost, then read it —
   the gap diagnoses the substrate.)

6. **Validate against the written prediction — the build is the last probe.** A
   divergence (most often *bigger / more mechanisms than predicted*) is an **alarm, not
   a verdict**. Route it to the search: is the extra length *accidental* (N mechanisms
   for one family — a missed invariant; find it and the code collapses back toward the
   prediction), *essential* (genuinely N families, no invariant to find; the surprise
   just taught you a domain axis you couldn't see — record it), or *over-broad* (the
   rule is **correct** but rewrites more producers than the defect touches — a consumer
   already holds the per-case info, so narrow it to the broken path instead of
   normalising every producer)? No metric decides this; only understanding whether the
   cases share a deeper structure.

**The alarm gates the *decision*, it does not merely log.** Whichever step trips, the
fired alarm routes to the search *before* the approach is chosen. "Thread the fix
through all N sites" stays legal — but only ever as the search's *conclusion*, never
the default. The sharpest failure is not *missing* the alarm but **seeing it and
overriding it** for momentum. (When the alarm is a *measurement that jumped* — a
false-positive count that leapt — the gate is the ratchet: flag the divergent approach,
don't revert the clean one. See engineering-rigor § "Building a noisy check to clean".)

---

## The two ways to fail (teach both — they are symmetric)

| Failure | What it is | The cure |
|---|---|---|
| **Ignore the alarm** (under-unify, ship the spray) | N mechanisms for what is really **one** family — a missed invariant; *or* the alarm fired and lost to momentum | the alarm must **gate**, not log (step 6); count the sites (step 2) |
| **Obey it blindly** (over-unify) | **one** mechanism forced over genuinely **N** families — a false invariant that breaks when the cases assert their differences | **probe** the cleanest claim (steps 3–4) |

The probe is what distinguishes them: it tells you whether the cases *actually* share
the invariant. Under-unification is caught by counting sites; over-unification by
trying to falsify the unifying claim.

But over-unification has a **second face a falsifying probe misses**: a unifying claim
that is **true** — every case really fits — yet applied **wider than the defect**. A
correct universal rule rewritten across every *producer* to repair a fault that lives in
one *consumer* path is over-reach by **blast radius, not falsehood**; no probe fires,
because the claim holds, and the re-assertion count can be **N = 1** (one place adds it
to all), so step 2 stays silent too. The check here is **informational, not
falsifying** — before committing the universal form, look for a **special circumstance**
in the implementation context (a consumer / runtime that already holds the per-case
information) that lets the broken path *adapt* while the producers stay untouched. The
*correct* universal story is exactly what suppresses that look; the cue to run it is the
scope ballooning past the size of the failing domain — universal stays the right default,
but special circumstances still occur and have to be looked for, not assumed away.

---

## Before you write it — the sites you have to count include the ones you cannot see

Step 2 asks you to count an invariant's re-assertion sites. That presumes you can *find*
them, and the ones that matter most are the ones you have no reason to open: another
module, another file, a library the project already depends on. A duplicate
implementation is rarely a decision made badly. It is a decision **never made** — nothing
in the local task raised the question, so the choice was never on the table.

This is why more care does not help. Care operates on what is in front of you, and the
second implementation is not in front of you. The failure is not carelessness, and
resolving to be more careful next time does not touch it.

Two radii, asked *before* writing rather than reviewed after:

- **inside this codebase** — does this predicate, list, constant, or routine already exist?
- **outside it** — does a dependency, a library, or the package index already do this?

**Two implementations of one rule is a defect with a delay, not untidiness.** The copies
drift, and a drifted copy is worse than a copy: it looks like the thing it is not, so it
passes review and reads as authoritative while answering differently. Every site you leave
behind is a place the invariant can quietly stop holding.

**The case that gets missed is "same functionality, new target"** — porting to another
backend, another output format, a single self-contained artifact — because it does not
*feel* like new functionality. It feels like moving something that already exists, so the
question "does this exist?" reads as already answered. Notice what the constraint actually
forbids: it forbids *depending* on the original, and that is not a licence to *re-derive*
it. Port it, or generate the new form from it. Rewriting from the idea throws away every
correction the original accumulated — which is how a reimplementation comes out not merely
duplicated but **wrong**, and wrong in ways the original already knew about.

**You cannot answer either question from memory, and reading for it does not scale.** So
reach for an instrument; if the project has none, build the cheap one *before* you write
the code rather than after. What such a tool looks for:

- the same set of names in a membership test or match arm, at two or more sites
- one symbol name defined in more than one module
- near-duplicate bodies — the same call sequence under different local names
- for the outer radius: the dependency manifest, the package index, the library catalogue

That is minutes of scripting, and it pays twice if you **keep it and re-run it** — a
one-off answer rots, an instrument does not. But such a tool searches by **shape**, and
shape is the weaker anchor; the next section is the stronger one.

### As the system grows, anchor the question on the RULE, not on the code

Searching by shape has a **rising false-negative rate**. Early on, two implementations of
one rule tend to look alike — the same list, the same few lines — and a grep finds them. As
the system grows the same rule gets expressed in more ways: one site checks a type list,
another takes an early return, a third consults a table. They implement one rule and share
no text. A shape-matcher cannot see that, and neither can you, because by then the whole
picture no longer fits in one head — which is the condition the search was supposed to
compensate for.

What survives that growth is **the rule itself, named**. So write down what you design as a
named rule — the invariant, spelled once, somewhere that is not the code — and have every
site that enforces it *say which rule it obeys*. A citation costs a comment, and it is
written at the only moment the fact is reliably known: while you are making that site obey
the rule. Later, nobody has to reconstruct it.

**A name is not yet an anchor — it has to be a TAG.** "Named" is carrying more weight there
than it looks: for a citation to be findable *by a machine* the name must be unique and
unambiguous, and a bare rule name is usually neither.

- **prose collision** — the name reads as ordinary words, so a search returns discussion
  as well as enforcement, and you cannot tell the count from the noise;
- **prefix collision** — one rule's name begins another's, so searching the general rule
  silently sweeps in the specific ones (and a search for a specific one misses sites that
  spell it generally);
- **namespace collision** — two areas independently pick the same short name, and nothing
  in either says they are different rules.

So mint a **tag**: a sigil you reserve for this purpose, so it cannot occur by accident in
prose, plus a rule that makes a match *exact*. Sub-rules make prefixes hard to avoid — a
general rule and its refinements naturally share a stem — so the cheaper route is usually not
to forbid prefixes but to **specify the boundary**: a citation matches the tag only when the
next character cannot continue a tag. (Word-boundary matching often will not do this for you;
if your names contain `-` or `_`, those already read as boundaries.) Keep the tags in one
registry and let a cheap check enforce that every citation resolves to a registered tag, that
no tag is defined twice, and that the boundary rule holds. Those three are what make step 2's count
*trustworthy* rather than merely suggestive; without them the number is a lower bound you
cannot size.

The tag is also the **rename unit**. Without one, renaming a rule silently rots every
citation and nothing reports it. With one, the rename is mechanical and the check names the
sites you missed.

**Mint the tag when you write the rule.** Retrofitting tags across a corpus that already has
both rules and enforcement sites is the expensive path, and it is the path you are on from
the moment you decide the rules are obvious enough not to need them.

Three things fall out, and the third is not available any other way:

- **the search becomes a lookup** — *does this already exist?* is answered by grepping the
  rule's name, instead of guessing which shape someone else chose;
- **step 2's count comes for free** — an invariant's re-assertion sites *are* its citations,
  so the number you were told to count is a query rather than an audit, and it stays correct
  as the code moves;
- **the merge question gets arbitrated.** Above: *equality is evidence; sameness-of-rule is
  the claim.* A rule name **is** that claim, written down and reviewable. Two sites citing
  one rule are candidates to merge. Two sites citing different rules stay apart however
  identical their code looks today — so when one of them later has to change, there is
  nothing to untangle, and the too-early-abstraction trap is closed by construction rather
  than by taste.

Keep the citations **honest rather than complete**: a citation naming a rule that does not
exist is worth failing on from the first day, while *every rule has at least one citation*
can tighten slowly as coverage grows. And generate any index **from** the citations rather
than maintaining one beside them — a second copy of where the rules live is the same defect
this section is about, one level up.

**The symmetric error is the over-unification already named above**, arriving by a
different road. Do not merge two sites because their lists are equal *today*. Equality is
evidence; sameness-of-rule is the claim, and only the second licenses the merge. A merge
that couples two rules which must stay free to diverge is worse than the duplication —
that is the too-early-abstraction failure relocated, and it is expensive to undo precisely
because later work has to fight it. Duplication is cheap to undo once you can find it;
that asymmetry is what makes the instrument the right investment and the reflex to merge
the wrong one.

---

## Keep it light — the procedure must not consume the judgment it serves

The cheap, always-available sensor is the **tell**: *"this is longer / absorbs more
than I expected for what it does."* Code longer than it should be trips the worry
before any matrix, profile, or crash — because the robust design covers N cases with
**one** mechanism (it lands short) and the brittle one covers them with **N**
mechanisms (it accretes as bulk, then as runtime cost). This is **robustness by
subtraction**: the deep reason *the shorter version is usually the more robust one* —
correctness-by-construction from a single nameable invariant, won by removing cases,
not guarding them.

The tell has an **earlier, more visceral form than final length — the implementation
*stutters*.** When the build fights you (you reach for a workaround to make the chosen
mechanism fit, the scope grows *again* mid-build, you are on the third re-read of code
that still won't sit right), that friction is the approach working *against the grain* —
a solution that fits the domain rides the existing structure and goes smoothly. **Route
the stutter to the search** (the over-broad check above — is a special circumstance being
fought?), **not to pushing harder.** Reading the stutter as *"this is just hard"* rather
than *"the frame is wrong"* is the characteristic miss: the friction WAS the signal, and
it fires before the length does.

But the tell says **look**, not what you will find — sometimes length is **essential**
(genuinely N families, or an invariant deliberately spelled out so it is explicit
rather than hidden in cleverness). So the tell triggers the *search*; it never dictates
shortening.

The full write-down-predict-probe-validate procedure fires **only when the tell trips
on something load-bearing** — never reflexively per function. An ever-on checklist
would be friction and would burn the very capacity the essential-vs-accidental
judgment depends on. Writing the prediction down is a load **reducer**, not a tax: it
moves the prior onto the page so working memory is freed for the building and the
judgment. Design is intrinsically heavy — it holds the whole composition space at once
— so the page is where you put down what you would otherwise drown carrying.

---

## The residual

The protocol only covers axes you *know* to vary. The axis invisible at design time —
the composition no probe imagined — survives any discipline; only real use reveals it.
That is why the method has a second engine: **the dogfood loop** — real consumers, not
toys — is what converts an unknown axis into a known one, and each harvested lesson is
appended to the axis list your next design's matrix varies. This protocol makes the
*visible* axes safe; the dogfood loop grows what is visible.

---

## In the loft tree (bindings — skip outside it)

The body above is tree-agnostic. Working in the loft repo, these are the instruments it
tells you to reach for:

- The outer radius (dependency manifest / package index / library catalogue) is
  `make libcatalogue` + `loft install` — read the generated `doc/claude/LIBRARIES.md`,
  never a clone or installed copy.
- The named-rule register is `doc/claude/formal/` (`@FR-` tags); `scripts/rule_tags.py`
  is the citation instrument (`list` · `check` · `sites <tag>` · `dups`), and
  `formal/README.md` says how a rule is written and cited.
- The verification questions a finished design answers are
  `doc/claude/DESIGN_VERIFICATION.md § C1`; declined designs are recorded in
  `doc/claude/DESIGN_DECISIONS.md` — check it before re-proposing one.
