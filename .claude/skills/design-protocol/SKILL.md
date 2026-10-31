---
name: design-protocol
description: >-
  A design is a testable HYPOTHESIS about one invariant, not a plan to execute. USE THIS
  before committing to a load-bearing design — a representation, the memory or ownership
  model, codegen, a public contract, a data format — and BEFORE RECOMMENDING a rule or
  design in answer to a question ("which way should this work?", "what would option 2
  entail?"): answering feels like advice, so the probe gets skipped, and an approved
  recommendation is built on. Also when deciding what a construct MEANS (what a bind, copy,
  drop or return does), when one rule collects its third exception or its defect list
  re-cuts instead of shrinking, and before writing or porting code that may already exist
  (another module, a library, an implementation for another target). The method: name the
  invariant, count the sites that re-assert it, build the cheapest probe that could falsify
  each load-bearing claim — for a recommendation, its premise about the current code — then
  build and validate against the written prediction. Where you cannot state the invariant
  (geometry, caching, store lifetime, round-trips), construct a concrete instance of the
  answer and read the invariant off it; for a meaning, table the candidate rules over the
  real use-case programs.
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

The characteristic failure of a capable designer is **over-reach**: making the design
*cleaner / bigger / more unifying* than the domain warrants. It is **invisible to
introspection**, because it presents as *elegance* — you genuinely **cannot see** the false
absorption from the desk, so "more care" does nothing for it. The cure is **better eyes**: a
test that can falsify the claim. *The design-probe is to a design what the boundary matrix is
to a bug.* (Re-reading the prose of a design that "also subsumed" an unrelated bug caught
nothing; probing that one claim did.)

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

## When you RECOMMEND it — the answer is the commitment

A design recommended in answer to a question is committed the moment the asker approves it:
they build on the recommendation, not on its probes, and an approved wrong rule costs a round
trip plus everything done under it. Answering does not feel like designing — nothing is being
edited, no routine is being written — so the cues that normally start this protocol do not
fire. That is the gap, and it is where a confident rule gets past both of you.

So before you recommend, run step 3 early: build the cell that would falsify the rule, and run
it. Aim it at the **premise** the recommendation rests on, which is usually a claim about how
the current system behaves ("X only happens when Y", "this value is reachable only through Z",
"the owner of this is always the one that outlives the other"). The asker can weigh your
options; they cannot check your premise about the code — that part is yours. A premise that
turns on a NAME in the code (an "owned" flag, a "borrowed" marker, an "adopted" store) is the
riskiest kind: the name says what the code decides, not what the program can still reach.

When you present several options, probe the premise each one rests on, not only your
favourite's. And say in the answer what you measured and what you assumed: an assumption named
is one the asker can challenge before approving it, and one left implicit is approved with
the rest.

---

## The two ways to fail (symmetric — debugging shares them)

| Failure | What it is | The cure |
|---|---|---|
| **Ignore the alarm** (under-unify, ship the spray) | N mechanisms for what is really **one** family — a missed invariant; *or* the alarm fired and lost to momentum | the alarm must **gate**, not log (step 6); count the sites (step 2); in a bug, the matrix showing the sibling cases |
| **Obey it blindly** (over-unify) | **one** mechanism forced over genuinely **N** families — a false invariant that breaks when the cases assert their differences | **probe** the cleanest claim (steps 3–4) |

The probe is what distinguishes them: it tells you whether the cases *actually* share the
invariant. Under-unification is the failure of the lazy, over-unification the failure of the
clever; both are brittleness, and both are invisible without the matrix or the probe. The same
pair is *under-reach* and *over-reach* when fixing a bug.

But over-unification has a **second face a falsifying probe misses**: a unifying claim that is
**true** — every case really fits — yet applied **wider than the defect**. A correct universal
rule rewritten across every *producer* to repair a fault that lives in one *consumer* path is
over-reach by **blast radius, not falsehood**; no probe fires, because the claim holds, and the
re-assertion count can be **N = 1** (one place adds it to all), so step 2 stays silent too. The
check is **informational, not falsifying** — before committing the universal form, look for a
**special circumstance** in the implementation context (a consumer / runtime that already holds
the per-case information) that lets the broken path *adapt* while the producers stay untouched.
The *correct* universal story is exactly what suppresses that look; the cue to run it is the
scope ballooning past the size of the failing domain — or the build *stuttering* (§ Keep it
light). Universal stays the right default, but special circumstances still occur and have to be
looked for, not assumed away.

---

## The other half — when you cannot form the invariant at all

Over-reach is the failure of a design you **already have** — a candidate invariant, too clean,
that a falsifying probe can break. There is a symmetric failure one move earlier: you cannot
state a candidate invariant at all. Step 1 names this state — *"a pile of cases, not a design
yet"* — but on one class of problem it is the **main event**.

**Exact-invariant domains.** In geometry, caching / serialization, hashing, store / memory
lifetime, and protocol round-trips, the correct design is not an open space to explore — it is
a single **construction that already exists** and must be **recovered**. There the
characteristic failure is **failing to generate the right candidate**: the mind reaches for an
**approximation** (fit a line, bucket the angles, smooth the wobble) or **chases the symptom**
(a store desync surfacing as three different corruptions). Both iterate toward an answer that
already exists exactly — motion that never converges, because the target is a point, not a
region.

**Same epistemics, opposite instrument.** A falsifying probe needs a claim to break, and you
have none. The generative instrument is to **plot a concrete instance of the *answer*** — the
exact target output / state for one specific input — or build the **cheapest thing that
*produces* it** (a throwaway prototype, a round-trip test), then **read the invariant off it.**
The construction you could not name in the abstract is legible in one worked instance: a cache
desync resolved the moment the target store state after a round-trip was written out, and a
wall-straightener that resisted eight approximations yielded once a prototype drew the exact
end-result.

**When the answer already EXISTS — capture its artifact and diff.** The cheapest "plot the
answer" is a **working sibling that already produces the construction** for a near-identical
case. Then you neither prototype nor theorise: **capture the sibling's artifact and your broken
one's — into files, not glances — and `diff`.** The residual divergence *is* the construction to
recover. Three captures sharpen it: BEFORE (your change off) → NOW (your change) confirms you
changed what you meant; NOW → PROVEN-sibling isolates the bug. The failure this prevents is
**symptom-chasing a near-miss**: holding a theory of the difference *without* the capture,
patching one manifestation, and finding the next (three wrong theories of one corruption, where
the captured diff showed the real difference in a line). For a captured-artifact domain — IR,
serialized bytes, store state, protocol frames — this beats prototyping: the answer is already
running; make yours byte-equal to it.

**A sharp distinction from the bug side.** The matrix law is *"the truth is in the class,
invisible in the instance"*. This does **not** contradict "plot one concrete instance," because
the instances are opposites: an instance of the **problem / symptom** is the trap (it hides its
class); an instance of the **answer / end-result** is the instrument (you read the invariant —
and therefore the class — *off* it).

**Where this sits.** It runs *before* step 3's falsification and moves step 5's build
*forward*: for an exact-invariant domain the build is the **first, generative** act — the
construction is named *by* building the cheapest version, that invariant is then probed (steps
3–4), and only then is the real one built. The cheap prototype is where the design gets
**pinned**, not merely confirmed.

---

## When the design is a MEANING — decide from the use cases

Both instruments above assume a correct answer exists. What a user-facing construct *means*
— what a bind, a copy, a drop, a return does — has none: it has the programs people write
with it, and they pull in different directions. So list those programs (lifetimes that nest
and cross, resources that can and cannot be shared, the move-like uses), run each today, and
table every candidate rule's result per program; decide in the rows where they disagree. Do
not draw the options from the rule text or the defect list — both encode the current model.

**When the patches point the other way.** A local fix answers *"how do I make this case right
under the current rule?"*, which strengthens the rule even when the rule is wrong. Signs: the
fix infers an intent the language cannot state; one rule collects exceptions; the defect list
re-cuts instead of shrinking; each option grows its own machinery. At the third exception to
one rule, stop patching and walk the use cases.

---

## Write the doc before the code — the failure-paths are where the invariant surfaces

The design doc is **not** after-the-fact write-up. Writing it is the **generative act that
surfaces the invariant and the requirements** — cheaply, before code commits you to an unstated
one. Enumerating the **failure paths** (every way it breaks) and naming **what must hold across
the boundary** is exactly where the load-bearing invariant becomes *nameable*. Code-first skips
that enumeration: you silently adopt the **narrowest invariant you happened to see** and then
discover the real one by **whack-a-mole** — each patch fixes one case and reveals the next. (A
link collision "fixed" by making two builds *match* collided again on the next mismatch; only
writing the failure paths down showed the invariant was *separation* of the two identities.)

So for a load-bearing design: **write the doc first** — the failure paths, the cross-boundary
requirements, the candidate invariant — *then* code. Not a full spec; the minimum that forces
you to enumerate how it breaks and what must hold. The enumeration is the instrument: the
class you cannot see from the desk is legible the moment you are forced to list its members.
This sits *before* step 1 and complements
*the other half*: when you cannot state the invariant, plot a concrete instance of the answer
*or* write the failure paths down; both make the construction legible.

---

## Before you write it — the sites you have to count include the ones you cannot see

Step 2 asks you to count an invariant's re-assertion sites. That presumes you can *find* them,
and the ones that matter most are the ones you have no reason to open: another module, another
file, a library the project already depends on. A duplicate implementation is rarely a
decision made badly; it is a decision **never made** — nothing in the local task raised the
question, and care operates only on what is in front of you.

Two radii, asked *before* writing rather than reviewed after:

- **inside this codebase** — does this predicate, list, constant, or routine already exist?
- **outside it** — does a dependency, a library, or the package index already do this?

**Two implementations of one rule is a defect with a delay, not untidiness.** The copies
drift, and a drifted copy is worse than a copy: it looks like the thing it is not, so it
passes review and reads as authoritative while answering differently.

**The case that gets missed is "same functionality, new target"** — porting to another
backend, another output format, a single self-contained artifact — because it feels like
moving something that already exists, so "does this exist?" reads as already answered. The
constraint forbids *depending* on the original; that is not a licence to *re-derive* it. Port
it, or generate the new form from it. Rewriting from the idea throws away every correction the
original accumulated — which is how a reimplementation comes out not merely duplicated but
**wrong**, in ways the original already knew about.

**You cannot answer either question from memory, and reading for it does not scale.** Reach
for an instrument; if the project has none, build the cheap one *before* you write the code.
What it looks for:

- the same set of names in a membership test or match arm, at two or more sites
- one symbol name defined in more than one module
- near-duplicate bodies — the same call sequence under different local names
- for the outer radius: the dependency manifest, the package index, the library catalogue

That is minutes of scripting, and it pays twice if you **keep it and re-run it** — a one-off
answer rots, an instrument does not. But it searches by **shape**, the weaker anchor.

### As the system grows, anchor the question on the RULE, not on the code

Searching by shape has a **rising false-negative rate**. Early on, two implementations of one
rule tend to look alike and a grep finds them. As the system grows the same rule gets expressed
in more ways: one site checks a type list, another takes an early return, a third consults a
table. They implement one rule and share no text, and by then the whole picture no longer fits
in one head — the condition the search was supposed to compensate for.

What survives that growth is **the rule itself, named**. Write down what you design as a named
rule — the invariant, spelled once, somewhere that is not the code — and have every site that
enforces it *say which rule it obeys*. A citation costs a comment, written at the only moment
the fact is reliably known: while you are making that site obey the rule.

For a machine to find a citation, the name must be a **tag** — a reserved sigil, matched
boundary-exact, kept in one registry with a check that every citation resolves and no tag is
defined twice; the `formal-rules` skill is that mechanism. Beyond it: the tag is also the
**rename unit** (without one, renaming a rule silently rots every citation); **mint it when you
write the rule**, because retrofitting tags across a corpus that already has rules and sites is
the expensive path; keep citations **honest rather than complete** (a citation to a missing rule
fails from day one, while *every rule cited* can tighten slowly); and generate any index **from**
the citations, never beside them — a second copy is this section's defect one level up.

Three things fall out, and the third is not available any other way:

- **the search becomes a lookup** — *does this already exist?* is answered by grepping the
  rule's name, instead of guessing which shape someone else chose;
- **step 2's count comes for free** — an invariant's re-assertion sites *are* its citations,
  so the number you were told to count is a query rather than an audit, and it stays correct
  as the code moves;
- **the merge question gets arbitrated.** *Equality is evidence; sameness-of-rule is the
  claim.* A rule name **is** that claim, written down and reviewable. Two sites citing one rule
  are candidates to merge. Two sites citing different rules stay apart however identical their
  code looks today — so when one later has to change, there is nothing to untangle, and the
  too-early-abstraction trap is closed by construction rather than by taste.

**The symmetric error is the over-unification already named above**, arriving by a different
road. Do not merge two sites because their lists are equal *today*; only sameness-of-rule
licenses the merge. A merge that couples two rules which must stay free to diverge is worse
than the duplication — expensive to undo precisely because later work has to fight it.
Duplication is cheap to undo once you can find it; that asymmetry is what makes the instrument
the right investment and the reflex to merge the wrong one.

---

## Keep it light — the procedure must not consume the judgment it serves

The cheap sensor is the **tell** — *"this is longer / absorbs more than I expected"* — and the
two-layer rule (cheap probe always, full procedure only when the tell trips on something
load-bearing) is `engineering-rigor` § The tell and § Keep it light. Two points are
design-specific:

- **Robustness by subtraction.** The robust design covers N cases with one mechanism and lands
  short; the brittle one accretes as bulk, then as runtime cost. Correctness-by-construction
  from a single nameable invariant is won by removing cases, not guarding them — the deep reason
  the shorter version is usually the more robust one.
- **The build stutters before the length shows.** You reach for a workaround to make the chosen
  mechanism fit, the scope grows *again* mid-build, the third re-read still won't sit right: the
  approach is working against the grain. Route the stutter to the over-broad check (§ The two
  ways to fail), not to pushing harder; reading it as *"this is just hard"* is the miss.

Design is intrinsically heavy — it holds the whole composition space at once — so writing the
prediction down is a load **reducer**: the page carries what you would otherwise drown carrying.

The axes no probe imagined survive any of this; walking the use cases (§ When the design is a
MEANING) moves many to design time, and real use finds the rest (`engineering-rigor` § The
residual).

---

## In the loft tree (bindings — skip outside it)

The body above is tree-agnostic. Working in the loft repo, these are the instruments it
tells you to reach for:

- A recommendation's premise is probed like any cell: a `/tmp` program per shape, hand-computed
  values, `--interpret` and `--native`, the controls beside it. loft#2273 put that rule here: a
  rule approved on the premise "an owned capture is reachable only through closures" was wrong,
  and a five-minute control cell showed it after the approval.
- The incidents behind the examples in the body (the caching and geometry recoveries, the
  store-delivery diff, the `StableCrateId` collision, loft#2273 in full) are in
  [`DESIGN_PROTOCOL-history.md`](../../../doc/claude/DESIGN_PROTOCOL-history.md).
- The outer radius (dependency manifest / package index / library catalogue) is
  `make libcatalogue` + `loft install` — read the generated `doc/claude/LIBRARIES.md`,
  never a clone or installed copy.
- The outer radius cuts both ways here (the dogfood policy): when no library implements a
  routine, prioritize building it INTO a library over burying it in the current project.
  Proving it first inside a side project is fine, but the end state of a routine worth keeping
  is available to more projects than this one — a project-local routine is a duplicate someone
  else has not written yet.
- The named-rule register is `doc/claude/formal/` (`@FR-` tags); `scripts/rule_tags.py`
  is the citation instrument (`list` · `check` · `sites <tag>` · `dups` · `registers` ·
  `coverage` · `worklist` — `make rule-coverage`, `make rule-worklist`), and
  `formal/README.md` says how a rule is written and cited.
- A construct's use cases are measured as one program per shape, run on `--interpret` and
  `--native` (a batch lets one wrong release corrupt the next cell). The full treatment —
  procedure, the copy-lease worked example, the evidence behind the patch signs — is
  `doc/claude/DESIGN_PROTOCOL.md` § Deciding what a construct means.
- The verification questions a finished design answers are
  `doc/claude/DESIGN_VERIFICATION.md § C1`; decided and declined designs are recorded in
  the register `doc/claude/DESIGN_DECISIONS.md` indexes (one `DESIGN_DECISIONS_<SUBJECT>.md`
  per subject) — check it before re-proposing one; `./scripts/idx next-decision` names the
  id for a new entry, which is kept by `@C<n>` citations (a code site, a doc site, one guard
  under `tests/`) and `make decisions` lists what it still owes — DESIGN_DECISIONS_RULES.md
  § Using the register.
