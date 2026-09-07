<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# GOALS.md — what loft is for, and the goals that serve it

loft is not the goal. loft is the **foundation**: the lowest layer of plumbing.
The real goal is the **libraries and tools built on top of it** — lavition (the
engine), the hex-world library, the editor, the games. loft exists so those can
be built, picked up, and enjoyed.

Its technical north star is **stability**. This document splits the work into
**seven goals (A–G)**. Each goal carries a **Check**: a command to run, or a fact to
observe. The point is to *measure* progress, not claim it. A Check stays the same
over time; its result does not — run it to see where you stand today.

## Purpose — what loft is for

One drive runs through every layer: **do the hard plumbing yourself, deeply, so
someone else can just pick it up and have fun.**

**Why — to aid the people who make games.** Making a game is hard enough already —
the feel, the systems, the content, the art. The tools should not pile on. But today's
engines are built **by programmers, from a programmer's point of view**: their
abstractions, their ceremony, their notion of "correct" all reflect how a *programmer*
thinks, and the maker has to learn that worldview before building anything. loft inverts
the default — it is built from the **maker's** side of the screen. The programmer's
concerns (memory, types, errors, lifetimes) are carried *by the language*, out of sight,
so the person making the game thinks about the *game*, not about thinking like a
programmer. That is the whole reason loft exists.

- **loft** handles memory, types, and the store, so you write the logic with no
  ceremony.
- the **hex-world library** handles terrain, walls, collision, and rendering, so
  you make a world game without building world infrastructure.
- the **editor** handles authoring, so you shape worlds.
- the **server** handles networking, so a shared game just works.

These are not separate projects. They are one drive, repeated. The destination is
lavition's identity: not "the hex-world engine" but *the engine where the hard
parts are already done.*

**The acceptance test: a thing is done when picking it up is *fun*.** Not
feature-complete — fun. A library can ship every feature and still be a fight to
use. That library is not done.

The fun is the point in itself, not a way to attract a crowd. We build the idea
because it is worth building. Adoption is a **result, not a steering input**: real
value reaches the people who share the idea in its own time. Every decision is
made on two things only — staying true to the idea, and depth.

### The deeper aim — software you can trust and forget about

The fun rests on something quieter: **a floor that does not betray you.** You can
prototype fearlessly only because the substrate will not corrupt your data or fall
over under an edit. So beneath "fun" sits an older aim — **software that does not
fail for software reasons** — and it has a proof of existence.

**The mechanism is mental load.** A game developer's attention is already spent on
the game — the feel, the balance, the content. Every unit the *language* demands to
keep things *correct* — manual memory, lifetime puzzles, null checks,
`try`/`catch`/`finally`, "what if this fails" — is a unit stolen from the creative
work. So an early, generating choice: loft **carries the correctness load itself,
instead of handing it to the programmer.** That one intent produces the design
decisions — ownership is internal ([DESIGN_DECISIONS.md C79](DESIGN_DECISIONS.md): the
compiler finds a valid free/copy, never a borrow error to answer); a fault degrades to
null and the program keeps running ([C80](DESIGN_DECISIONS.md): no exception machinery
to get right); null is implicit, not a `Result` to thread. The language holds the
invariants; the developer builds the game. **Fun is what a low correctness-load feels
like from the inside.**

**It is *timing*, not the mere presence of load.** What the principle forbids is load at
the **wrong moment**: at *runtime* (the running game must not stop — C80's whole job) or in
the *common path* (the ordinary code you write all day). A rare, deliberate **write-time**
acknowledgement — `x as u8` at the point you *choose* to throw bits away — is fine: you are
in the editor, with the headspace, doing it on purpose. The test is **frequency in idiomatic
code**: a well-built library, used as intended, should need **zero `as`**. The archetypal
failure is Rust's `usize` — indexing must be pointer-sized, a toolmaker's memory fact, so
**everyone who writes `v[i]` is forced to cast** (`i as usize`, `len() as i32`) all over the
common path. That is the wrong-moment tax, structural and unavoidable. loft refuses it with
**one integer type and no special index type**: `v[i]` takes any integer, indexing produces
no `as`, and the pointer-width fact stays the *language's* problem. (That is the maker-centric
reason for the one-integer model — [@PLN88](https://github.com/loft-lang/plans/issues/88) —
not just tidiness.)

The IBM midrange machines (System/38, AS/400, today's IBM i) were *loved* by the
people who programmed them, for an unglamorous reason: they did not fail — not the
hardware, and, more tellingly, not the software. A program written in 1988 still
runs unchanged on modern hardware. The database, the language, and the store were
one dependable abstraction — something you could build a business on and then
*forget about*. That is the feeling we lost, and the one to gain back.

That dependability was structure, not luck. The system was **designed, not
assembled**: one team built the OS, the database, the language, and the machine
interface as a single thing, so the seams where modern software breaks — a library
and its caller drifting apart, app objects versus database rows, a dependency three
levels down shifting underneath — mostly did not exist. The machine enforced its own
invariants, so whole classes of failure were *impossible by construction*, not
caught after the fact. And backward compatibility was a contract the maker kept: the
platform never broke its users; the cost of change was paid by the maker, not the
customer.

We lost this without noticing the price. The industry traded **designed
integration** for **assembled-from-parts composition** — for velocity and reuse —
and every glued-on part is a seam, and every seam a way to fail. Modern software is
unreliable in large part because it is assembled rather than designed.

loft's bet is to win that reliability back **without the cage.** The AS/400 bought
its stability partly with lock-in — closed, proprietary, conservative. The hard,
unsolved problem is to keep the dependability of the integrated machine while being
**open, statically typed, and live-editable**: the heap *is* the database, the
language woven through the store, and it still does not fall over. (That integration
is the same one the next subsection argues from the live-migration side.) Only one
method earns it, and it is the one this document describes — reliability **by
construction, not by testing**: find the one invariant, enforce it at the
chokepoint, make the bad state *impossible* rather than merely absent. **Goal A**
(soundness) and **Goal E** (predictable memory) are this aim turned into goals;
bug-by-bug is hopeless against this target — you reach it only by retiring whole
classes of failure, one substrate at a time.

And the bet covers more than the lines loft's authors type: a system fails through
what it is *made of*, not only what was written for it — so the **dependency
surface** has to be dependable too, and "assembled from parts" is where modern
software bleeds. loft's answer is the substrate. It inherits the Rust ecosystem
([BROADENING.md](BROADENING.md) § differentiators #4), so a **well-grounded** crate
brings its memory safety and battle-testing *into* the stack: a bound safe-Rust
crate cannot segfault loft the way a buggy C extension hard-crashes a Python process,
and `cargo` gives reproducible builds where Python has wheel/ABI roulette. So
*"never fails for software" includes never being failed by a dependency* — which is
exactly why crates are bound **well-grounded** (the reliability filter — you inherit
stability only from crates that have it) and one per real need, not wholesale.

The cage was also *unfamiliarity*: the AS/400's gift came in a closed, proprietary
world. So loft wears a modern, Rust-shaped surface — the on-ramp that platform never
had. That surface is the hardest constraint, not the easiest: it constantly tempts
the matching substrate — compiled layout, a borrow checker, serialization as a
separate representation — each of which would quietly kill the integrated,
schema-as-data store the reliability depends on. The long, careful work is holding a
familiar surface over an alien-but-dependable substrate, because the alternative —
rushing to *look* like Rust — *is* Rust underneath, and loses the goal. The years
were not overhead; they were the goal taken seriously.

### The destination is BORING — a tool you notice only when it is missing

"Trust and forget about" has an end state, and it is worth naming because it does not
look like success from the outside: **loft should be boring.** Not impressive, not
clever, not the interesting part of anyone's stack — just a thing that works, reached
for without thought, and noticed only in its **absence**: the moment you go to use it
and it is not there, and you miss it. Until then it should say nothing and get out of
the way.

That is a claim about **maturity, not modesty.** A young tool is exciting because it
is still surprising you, and every surprise is a thing you had to learn, work around,
or repair. Arriving at boring means the surprises are gone. So "nothing interesting
happened" is the report we want, and a release whose headline is *fewer things you
have to think about* is the release that moved furthest.

It follows from the deeper aim rather than being a separate idea. Software you can
forget about is, by definition, software that does not ask for attention — and every
mechanism above is already this principle in another form: a fault degrades to null and
the program keeps running ([C80](DESIGN_DECISIONS.md)) instead of demanding a handler;
ownership is internal ([C79](DESIGN_DECISIONS.md)) instead of asking you to prove
lifetimes; the correctness load sits in the compiler instead of in your head. Each one
removes an occasion for loft to be interesting at you.

It binds the **tools**, not only the language, and that is where it is easiest to
violate:

- **Say nothing when nothing needs acting on.** A line that reports everything is fine
  is a line the reader learns to skip — and the day it says something else, they skip
  that too. Silence on success is what makes the exceptional case visible.
- **Never make our roadmap the user's problem.** Plan numbers, phase names and
  not-yet-implemented apologies belong in the code and the plans, never in output. A
  user asking a question deserves the answer, not our backlog.
- **Explain at the moment of failure, not before it.** The full account of what went
  wrong and what to do earns its space when something IS wrong; the same text on a
  healthy run is a lecture.
- **No ceremony.** Banners, progress theatre and self-congratulation are the tool
  making itself the subject.

The test to apply to any surface: *would a user who does not care about loft notice
this?* If yes, and nothing is wrong, remove it.

### Legible cost — you keep the performance-critical decisions

There is a second failure that "does not fail for software reasons" has to cover, and it
is the one that generated loft: code that is **correct at low scale and fatal at
production**. A construction runs fine on a developer's small dataset, then breaks the
day real load arrives — an N+1 access pattern, an unbounded cache, a GC that never paused
until the allocation rate was real. The code was not wrong; it was wrong *at a scale the
source did not reveal*, because the language hid the decisions that determine cost. The
worst case is **interleaved on-the-fly allocation** — objects scattered across the heap as
they are created, invisible until the working set outgrows cache, and *unactionable* even
once diagnosed, because a managed language gives no lever to dictate placement.

So loft draws a line by **performance-criticality**: a performance-critical decision is
never abstracted away — **allocation topology (group related data into one allocation,
split unrelated data into another) stays visible and in the programmer's hands** — while
the bookkeeping that is *not* performance-critical is automated, but **deterministically**
(freeing at scope/owner death, not a tracing collector; a copy by a legible rule, not a
silent deep copy). What loft refuses is *hidden, nondeterministic* machinery — a tracing
GC, a surprise reallocation. This is the layout/cost sibling of **Goal E**: E makes *when*
a value dies match the source; this makes *where* it lives, and *what it costs*, the
programmer's to see and set. The mechanism is [OWNERSHIP_MODEL.md § the control
story](OWNERSHIP_MODEL.md#why-this-shape--the-control-story-makers-call); the realized
speed of it is a `--native` concern (the interpreter's dispatch overhead masks locality),
so today loft lets you *express* the right layout and `--native` turns that into machine
cost.

### Why a language, not a store bolted onto an existing one

A key reason loft is a *language* and not an in-memory data store added to Rust
or C++: **live data-structure migration**. A programmer edits a game's structs
and enums *while the game runs*, and the existing data survives the edit — the
current game stays alive, and as much of its state as possible is preserved.

That capability cannot be added later; it demands that **the value, its in-memory
layout, its stored form, and its serialization are one representation.** A store
bolted onto an existing language cannot give it — for a structural reason, not an
effort one:

- In Rust / C++ / C# a value's layout is the **compiled native layout** — fixed
  offsets and raw pointers frozen into the binary at compile time. The type
  system has no runtime-mutable notion of "the schema"; serialization is a
  *second*, separate representation (serde / reflection) that mirrors the type by
  hand and drifts from it. To migrate live data when a struct changes you must
  serialize out, **recompile** (you cannot redefine a running program's struct),
  and read back — and the recompile *kills the running game*, the very thing you
  were protecting.
- In loft the **schema is data** (`Stores.types` / `Parts`, mutable at runtime),
  the value **is** a self-describing record against that schema (`DbRef`,
  position-independent, name-keyed), and serialization is just reading the
  record. Changing a struct is changing schema *data*; the existing records
  migrate by being re-read through the new schema — name-keyed and lenient,
  **while the game runs**, because no compiled-in layout fights it.

The four things a bolt-on keeps separate — language types, runtime values, heap
layout, wire form — are **one** representation here, and that unification has to
be the foundation; it cannot be added on top. It is the same principle loft2
turns on the compiler's own data (the IR *becoming* store-resident), and it is
what makes lavition's rapid prototyping rapid: iterate the data structures
without losing the test game's state.

**What survives an edit** (the observable form — a name-keyed round-trip through
loft's native serialization):

| edit | survives? |
|---|---|
| add / remove / reorder a field | ✅ — missing→default, unknown→ignored, name-keyed |
| add / remove / reorder an enum variant | ✅ — a removed variant → **null sentinel**, never a wrong variant |
| widen a field's numeric type | ✅ — coercion |
| **rename** a field or variant | ❌ today — reads as delete+create; the planned fix is a **migration setter** for the *old* field (declares old name + old type, body routes it to the renamed field) |
| scalar ↔ struct / incompatible type | ⚠️ — defaults |

The principle that governs every extension: **leniency is the feature, not a
hazard** — it is what lets the schema change while old data reads. The only thing
forbidden is silent *wrong* data (a fail-soft that fabricates a plausible value);
silent *graceful* data (a default or a null) is exactly the preservation we want.

The one real gap is **rename**, and the planned mechanism is a **migration
setter** for the old field: a setter named for the *old* field (declaring its old
type, so the parser knows how to read the serialized value) whose body routes the
value to the renamed field — and which also generalizes to retype, split, or
computed migration.  Its *absence* is the signal: an incoming field with no
matching field **and** no setter raises a **warning** (Goal F — the consequence
of the programmer's own rename, free to ignore), so a forgotten rename is never a
*silent* data drop.  loft has no setter concept today, so this is a small new
language feature — its own plan when picked up, out of scope here.

### Minimum syntax, maximum value

loft is, at heart, a **thought experiment: how much value can a *limited* syntax
deliver?** The small surface is not a constraint the language fights — it is the
point. Capability is meant to be **derived from a compact, learnable core** (the
store, the type system, reachability) rather than **added as more syntax**, so the
language a maker must hold in their head stays small while what they can express
keeps growing. This serves the maker-side purpose directly: every keyword the maker
does *not* have to learn is ceremony that never reaches them.

The tell that it is working is when a capability **falls out of the architecture**
instead of needing its own grammar:

- **Three visibility tiers from one keyword.** `pub` plus reachability already
  yields *private* / *sealed* (a value-exposed non-`pub` type — read, but not
  nameable or constructable: the factory pattern) / *public*, with no `sealed`
  keyword to learn.
- **Every keyed collection is a type, not a new syntax.** `hash<T[k]>`,
  `sorted<T[k]>`, `index<T[k]>`, `spatial<T[x,y]>` are one shape over the one store,
  and a spatial range query *reuses range syntax* — `xs[(0,0)..(10,10)]` — rather
  than inventing a query language.
- **The whole fallible-value story is `?` and `??`.** No exceptions, no
  `try`/`catch`; a computation yields a value or `null`, and `??` supplies the
  default at the point of use.
- **A tool as involved as the compatibility check needs no new surface.** A
  library's public API is *already* fully described by `pub` + the type graph, so
  `loft api-surface` derives the whole thing (@PLN102 C1) without adding a keyword.

The design rule this implies: **a feature earns new syntax only when it genuinely
cannot fall out of the core** — prefer deriving it from what is already there (fold
it in — [COMPATIBILITY.md](COMPATIBILITY.md) § Folding), and reach for new grammar
last, not first. The honest cost is **implicitness**: economy can trade an explicit
declaration for emergent behaviour (the sealed tier is *implied* by a signature, not
stated), so the discipline is to keep each derivation **legible** — economy in
service of a maker who can still predict what the code does, never cleverness for its
own sake.

The seven goals below are **foundation goals**: loft must be sound (A), shipped and
clear (B), capable (C), portable (D), predictable (E), friction-free (F), and fast
enough to pull its weight (G). A
crack in the foundation becomes a crack in everything above it. The same bar holds
for the libraries on top — and they are the real end. The shift up is now underway:
the libraries are **extracted, versioned, and installable** through the
[`loft-lang/registry`](https://github.com/loft-lang/registry) (Goal B's structure
floor — see [The two floors](#the-two-floors--why-dogfood-is-paused-and-when-it-resumes)),
so the foundation has largely finished its job. What the libraries do **not** all
meet yet is the next bar up — **per-library quality**: each one, on every target,
good enough to be fun ([LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md): cross-target
parity + the registry `verified` mark). That quality bar — not extraction — is the
work moving up.

---

## The goals at a glance

| | Goal | In one line |
|---|---|---|
| **A** | Soundness | no silent corruption — now, and across toolchain bumps |
| **B** | Release & legibility | it ships, and the value is legible on contact — libraries readable as the teaching corpus |
| **C** | Capability | real consumers keep building and running |
| **D** | Parity | identical on every OS × backend |
| **E** | Predictable memory | the source is the truth — *surpass Rust here* |
| **F** | Friction-free | the language serves the programmer, never the compiler |
| **G** | Performance | every routine within the bar of its industry reference twin |

All seven point at one destination: **boring** — a tool noticed only in its absence
(§ [The destination is BORING](#the-destination-is-boring--a-tool-you-notice-only-when-it-is-missing)).
Soundness, parity and predictable memory remove the surprises; friction-free removes
the ceremony. A release whose headline is *fewer things you have to think about* is
the one that moved furthest.

The bar covers loft (the foundation, nearly there) *and* the libraries on top
(the real end, which fall short today). Each goal carries a **Check** — something
to run or observe — so progress is measured, not claimed.

---

## North star

**loft is stable when it is correct now AND stays correct as the world changes
underneath it** — new rustc/LLVM toolchains, new platforms, new ways consumers
write code. The failure mode we guard against is **latent undefined behaviour
(UB)**: a bug that passes every test on today's toolchain, then shows up as silent
data corruption after a compiler upgrade or on a stricter platform. "Passes today"
is not "stable."

## Stability trumps features

**The rule.** A feature must be needed enough to cover what it costs — and the
cost is counted in the currency that matters here: brittle constructions, bug
surface, and implementation complexity. When the value does not cover the cost,
we **limit the feature**: forbid the expensive shape at compile time with an
error that names the supported path, and keep the cheap shape. Features serve
stability, never the reverse. This is a default to keep in mind, not a hard
line — each cut is recorded in
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) precisely so it stays cheap to
reevaluate when a real consumer brings new evidence of need.

**How to apply it.** When a bug investigation reveals that a language shape only
works through fragile machinery — an ownership story with no defined owner, a
parse-order-sensitive decision, a repair per case — ask first: *what programmer
need pays for this?* Consult the dogfood consumers. If every real program is
content with a simpler shape, cut the expensive shape instead of repairing it.
The rejection must hand the user the supported path (Goal F draws the exact
line: an error that **bounds the language** is fine; an error that makes the
user **serve the compiler** is not). Goal E's subtraction principle is the same
move on the implementation side — remove the mechanism rather than guard it.

**Worked example.** [#314](https://github.com/loft-lang/loft/issues/314): a
scalar captured by several closures, one of them mutating it, only worked
through shared heap cells with "first death wins" ownership — refcounting-shaped
complexity with no counter behind it. No consumer needed the shape (the kernel
program that surfaced it preferred a struct, which also makes the sharing
visible in the source). Decision: reject the multi-closure shape at compile
time; the single-closure accumulator — one record, one owner, sound — stays.

**Check.** Every limited feature has a [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md)
entry naming the cost it avoided and the supported alternative, and its
rejection diagnostic is pinned by a test.

## The two engines (both required; neither finds the other's bugs)

- **Dogfood — "is loft useful?"** Build real programs in loft (the branch-review
  viewer, the tracker indexer, `lib/markdown`, the games moros/dryopea). Learn
  what the language is missing, fix it, and ship the result. This finds the bugs
  users hit *today*: missing features, awkward ergonomics, feature-interaction
  bugs, design inconsistencies. (See CLAUDE.md § "Development cadence".)
- **Sanitizer — "is loft safe?"** Use tools to detect UB — Miri, the homegrown
  `stack_align_guard`, and fuzzing — covering bad alignment, use-after-free,
  uninitialised reads, aliasing, and leaks, whether or not the output looks
  correct today. This finds the correct-today, corrupt-tomorrow bugs that
  dogfooding cannot see.

These feed four goals: **A** (soundness) is the sanitizer engine; **C**
(capability) and **B** (release) are the dogfood engine; **D** (parity) uses
sanitizer-style differential testing but serves both.

---

## Goal A — Soundness (no silent corruption)

**Definition.** loft never produces wrong or corrupt results from undefined
behaviour, and stays that way across toolchain upgrades. This is a *continuous*
property: it is met by a sanitizer that keeps running and keeps catching new UB,
not by one clean run.

**Check.**
- `ci.yml`'s per-PR `guard` and `asan` jobs are green on `main` (and on the PR
  under review).
- `cargo test --features stack_align_guard` fires zero times across the whole
  corpus (every test binary, not just `issues`).
- The nightly `miri.yml` gate is green.

Healthy when all three hold *and keep holding* as new code lands.

---

## Goal B — Release & legibility

**Definition.** loft actually **ships** on a regular cadence, and its value is
**clear the moment you meet it** — installable, with a clean starting path — so the
people who would value it *can* find and recognise it. A language that never
reaches a stable release, or that only its authors can run, never gives those
people the chance. Adoption itself is a *result, not a goal* (see Purpose); this
goal is met by the starting path *existing*, not by a user count. The same legibility
governs *developing* loft: everything needed to work on it lives in the repo — the
source, the docs, and the executable skills — so any coding agent can continue it, and
the project has no single point of failure. See [BUS_FACTOR.md](BUS_FACTOR.md).
And it governs the **libraries' own source**: they double as loft's teaching corpus —
an open-source project heads-on, where every part of a library stays as readable as
possible.  What this refuses by name is the "fast pass" pattern (readable code shadowed
by an optimized twin nobody can follow): a slow routine is fixed in the engine or in its
own loft algorithm, and a native rewrite is a recorded per-routine edge case
([formal/performance.md](formal/performance.md) `(Perf-Cure)`).

**Check.**
- A release tag exists within the project's release cadence (`git tag` → latest),
  each with a CHANGELOG.md entry.
- `loft install <name>` resolves and fetches a published library, end to end,
  against the registry.
- On a clean machine, a new user can install loft, run a first program, and `use`
  a library — from the docs alone, with no extra guide.

**A reading, not a target.** Whether anyone outside the project publishes a library
is *observed, not chased*. It tells you the value is landing; it is not a number to
optimise. Removing it as a steering input keeps the work pure (true to the idea,
plus depth — see Purpose).

---

## Goal C — Capability via dogfood

**Definition.** loft keeps gaining the features and ergonomics that real programs
need, and the main consumer programs keep building and running.

**Check** — the consumer build matrix; each row is a command, scored `N/total`:
- the branch-review viewer builds and runs on HEAD;
- the tracker indexer (`make index`) succeeds;
- the `lib/markdown` suite passes;
- the games moros / dryopea build and run against current loft;
- the last release's CHANGELOG has a consumer-driven harvest section.

Never "finished"; it reads as a fraction, and a falling fraction is the alarm.

---

## Useful is not stable — the gap dogfood cannot measure

Goal C's fraction says the language is **useful**: real programs build and run, and the
consumers keep finding it worth using. That is a real measurement, and it is not a stability
measurement — because a consumer exercises the paths *its author happened to walk*.

Those paths get hard through use. A shape that moros or dryopea leans on daily has been hit
from every angle a working program hits it from, and its bugs were found years ago by the
cheapest possible instrument: somebody running the program. **Heavy use is a test suite nobody
had to write.**

Everything else is an outlier — and an outlier is not a rare shape, it is an *unwalked* one.
The same feature at a different type, in a different position, under a `?`, on the other
backend, in a `return` instead of a local. Nothing about it is exotic; it is simply that no
program we have has needed it yet. **A new project or a new user hits those on their first
day**, because their program is not our program, and what is peripheral to us is central to
them. The measured form of this: `hash<S[k]>? = [S { … }]` is refused today, and the reason is
not that anyone decided it should be — it is that no consumer has ever written a nullable keyed
literal.

**So the gap this work bridges is between *"works on the paths we walk"* and *"works on the
paths someone else will walk."*** Every instrument aimed at the corpus measures the first. The
bug windows say so directly: they are dominated by `hit-by:loft`, and BUG_REVIEW.md's own
reading is that *"the sample is entirely self-generated, so it measures our reach and not the
language"*. A corpus can only ever cover what somebody already wrote.

**A rule can cover what nobody has written yet**, and that is why the rules are the instrument
rather than a nicety. `formal/`'s rules are stated over a whole domain — *every* `τ`, *every*
collection kind, *every* position — so walking one asks about cells no program in the tree
reaches. That is the concrete link from this section to the method:
[STABILITY_METHOD.md § The rule-led walk](STABILITY_METHOD.md).

Two consequences worth stating outright, because both look like poor use of time under a
capability lens and are the point under a stability one:

- **A cell that works is a result.** Proving that a rule holds on nine types across three
  routes converts thirty unwalked paths into walked ones. Nothing shipped, and the language
  got more stable, because the next person to walk them is no longer the first.
- **An unwalked path's bug is worth more than its frequency suggests** — but which unwalked
  path matters is decided by DEPTH, not by rarity, and the next paragraph is the whole ranking.

### Not every unwalked cell is worth the same — rank by how contrived it is

The full matrix contains a great many cells that only exist because a programmer *shoehorned*
something in: four features composed in a way nobody would arrive at without deliberately
pushing at the edges of the language. **A problem in one of those is survivable** — the person
who built that cell knows they are at the boundary and has the context to back out — so it does
not set the queue.

⚠ **Depth ranks URGENCY; it never licenses a refusal.**  What decides whether a shape must work
is [`formal/`](formal/README.md), and its doctrine is *the rules do not change to match the
code; the code changes to match the rules*.  **If the definition gives a clear picture of what
to implement, implement it — right away, whatever the cell's depth.**  A contrived shape the
rules cover is a shape that must work; refusing it would be a deviation, not a decision, and
[CLAUDE.md](../../CLAUDE.md) § Debugging policy already says to read the spec *before shipping a
refusal* for exactly this reason.  Nor is silence from the rules a licence: an edge the rules
cannot EXPRESS means the rule wants extending, and that is the next question rather than the end
of one.

A refusal is right only where the rules do not settle it **and** there is a positive reason to
bound the language — the shape works only through fragile machinery no programmer need pays for.
That is § Stability trumps features, and it is a deliberate design decision recorded in
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md), never a default that deep cells fall into.

The cell that matters is the opposite one: **a casual user tries something obvious and hits a
wall immediately.** They are not exploring the boundary, they do not know there is one, and the
wall is the first thing the language ever told them. That is the failure this work exists to
prevent, and it is worth more than a deep bug in every dimension that counts — it costs a user
rather than an afternoon.

**This ranking is already operational — it is what the `wa:` labels measure**, and recognising
that is better than inventing a second axis beside them. A contrived cell has an obvious
workaround *by construction*: the programmer composed four features to get there, so **doing the
simple thing instead is always available to them**. That is `wa:clean`. A casual user who hits a
wall on an obvious shape has nowhere simpler to go — they already wrote the simple thing — which
is `wa:none`, *"blocks whoever hits it"*.

So the ranking reads off the tracker rather than off a judgement:

- **`wa:none` — the case this work exists for.** Nobody reaches it by pushing at the edges;
  they reach it by writing the obvious thing. Top of the queue regardless of `sev:`.
- **`wa:partial` — awkward or lossy escape.** The user gets out, but pays, and had to learn
  the language has an edge here.
- **`wa:clean` — usually the contrived end**, and usually survivable, so it waits.  It still
  gets fixed when the rules say the shape must work; it is later in the queue, not exempt.

⚠ **`wa:clean` does not by itself prove a cell is contrived**, and the implication only runs one
way: contrived ⇒ a workaround exists, not the reverse. loft#1122 was as shallow as a cell gets —
a bare variant in a tuple member — and carried a `wa:clean`; the workaround only existed because
a fix had landed hours earlier, and before that the same cell was `wa:none`. So read `wa:none` as
decisive and `wa:clean` as weak evidence to be checked against the shape.

**This is why a VERIFIED workaround in every issue is load-bearing rather than a courtesy.** It
is the datum that ranks the issue — the filing policy already requires one
([CLAUDE.md](../../CLAUDE.md) § Bug-filing), and the reason is this section. An issue filed
without it is not merely less helpful; it is unranked.

When a rule walk enumerates the related cases (step 4 of
[the rule-led walk](STABILITY_METHOD.md)), the ordering follows the same rule: the cells to
build first are the ones a programmer reaches without knowing the language has edges. A shallow
cell that fails is the finding; a deep cell that fails is a note.

This is the same doctrine as § Stability trumps features, read from the other end: that section
says do not add a shape you cannot afford to keep working; this one says the shapes you already
promised must work everywhere they are promised, not only where they are used.
---

## Goal D — Cross-platform + cross-backend parity

**Definition.** loft behaves identically on **ubuntu / macOS-ARM / windows** and
across the **interpret / native / wasm** backends. No result depends on the
platform or the backend. macOS-ARM matters most here: it requires aligned memory
reads, so an unaligned read is a real fault there. The three backends are three
separate implementations of the same language.

**Check.**
- The 3-OS CI matrix (`ci.yml`) is green on `main`.
- A differential run executes one shared set of programs on interpret / native /
  wasm and checks for **identical output and diagnostics** — zero differences.
  Per-backend green is *not* enough; the backends must agree on the same input.

---

## Goal E — Predictable memory (the programmer's model *is* the truth)

**Definition.** The runtime's memory behaviour matches the **plain reading of the
source**: a value's heap memory is freed when its **scope** ends, with **no
exceptions the programmer has to learn**. The model is small enough to hold fully
in your head — write a vector inside a block, and it is freed at the end of that
block, full stop.

This is **different from Goal A.** A program can be perfectly *sound* (no
corruption) and still hold far more memory than the source suggests, because the
runtime quietly keeps it alive. Goal A asks "is it safe?"; Goal E asks "**can the
programmer predict it?**"

**Why it's a goal, and the bar it sets.** The appeal of this kind of control is
that the memory model is small, the source is the truth, and when something goes
wrong the fault is the programmer's — knowable and fixable. Rust buys safety with a
static analysis the programmer must reason about, so debugging often becomes "what
did the compiler decide?". loft makes a different bet: safety from a **runtime
discipline** instead of a static proof. That lets the *rule* be the whole model.
The aim, stated plainly:

> **On the single axis of safe AND predictable (the programmer stays in control),
> surpass Rust** — not at performance, concurrency, or ecosystem, but here,
> because Rust cannot separate safety from its hidden machinery and loft can.

What makes or breaks it: the **rule the programmer sees stays exceptionless** — "a
value dies when its scope dies". All the cleverness — last-use analysis, reference
handling, deciding slot vs heap — lives **only in the implementation, out of
sight**. The moment the *rule* grows an "except when captured / iterated /
reassigned…" that the programmer must memorise, loft has rebuilt Rust's hidden
complexity in a new form. When loft's behaviour surprises the plain reading of the
source, that is a **loft bug** — never "harmless", never worked around in docs.

**Check.**
- `LOFT_STORE_GUARD=1` is **silent across the corpus** — no block-scoped vector
  store is freed later than the block it belongs to. (Detector shipped; see
  [plans/2-vector-store-watermark/fix-design-store-lifetime.md](plans/2-vector-store-watermark/fix-design-store-lifetime.md).)
- That guard is promoted to a `#[cfg(debug_assertions)]` assertion, so the rule
  cannot quietly regain exceptions as new code lands — the guard *forbids* the
  drift, it does not merely report it.

Healthy when the guard is silent corpus-wide *and* the assertion holds *and keeps
holding*. A guard that starts firing is the alarm that the model and the runtime
have drifted apart.

**Progress — reference counting removed** (plan-57,
[`@PLN2`](https://github.com/loft-lang/plans/issues/2), closed 2026-06). The store
reference count is gone (`ref_count` / `inc_rc` / `dec_rc` / `OpIncRc` deleted;
`Store.pinned` for const/global). It is replaced by a single-ownership free at scope
end plus a closure-record cascade. This advances Goal E directly: **no hidden
counter decides when a value dies — the scope does**, which is the plain reading of
the source. The reference count was the clearest case of sound-looking machinery
that hid the real lifetime; removing it makes the lifetime visible.

---

## The method mirrors the goals

Goal E's law — *the stated model must match reality; a divergence is a bug, fixed
by **removing** hidden machinery, not by adding cleverness* — is also how loft is
**built**, not only what it ships. The same law, one level up, governs our own
reasoning:

- an investigation's stated thesis must match its contents — don't slip an
  unrelated bug's fix into it (that makes the record disagree with what it claims
  to be);
- a bug verdict must match the bug's **verified** shape — don't claim "zero blast
  radius" over code you haven't probed. An unchecked confidence does the same
  thing the old reference count did: it lays a clean story over a reality you
  didn't check.

The edge-probe-first habit and the sibling-bug scope hygiene
([plans/README.md § Edge-probe](plans/README.md#edge-probe-before-fixing--the-lightweight-default-for-lofts-complex-variant-bugs))
are this same rule turned on our own claims. A team that tolerates hidden machinery
in its own reasoning cannot credibly ship a language whose whole promise is
no-hidden-machinery.

One related heuristic: when you hit the same wall again and again, the cause is
usually not your current attempt but an **old conservative mechanism** that was
correct before you had today's information and has outlived the gap it covered (the
store reference count is the model case). The move is to find that old assumption
and **narrow it with what you now know**, not to pile cleverness on top.

### Bugs hide things — clear them before trusting the model

A bug is not just a broken spot. It **hides everything downstream of it**, and —
worse — it can make broken things look fine. Examples are routine:

- a `parallel {}` block that silently did nothing on `--native` made test-80 and
  test-81 *pass* — "the asserts held" hid that native ran no arm at all;
- a read-only-store crash hides what a heap mutation would actually do;
- an over-retained store under the old reference count *never crashes*, so a wrong
  free site looks correct.

So clearing bugs is a **precondition** for verifying Goal E, not a separate task:
you cannot confirm "the model is the truth" through a bug that hides what the
runtime really does. This is why the standing detectors (the sanitizer for A,
`LOFT_STORE_GUARD` for E, a differential backend sweep for D) earn their place:
they turn hidden behaviour into a visible signal, so the model becomes checkable.

### Don't tolerate re-derivation patterns — engineer the class away

Some bugs are not a broken spot but a **broken pattern**: shared state that several
routines each work out *independently*, instead of reading it from one place. When two
of those routines disagree, the result is wrong — yet no single routine is at fault, so
the failure is **silent**. It slips past [Goal A](#goal-a--soundness-no-silent-corruption),
and it often shows up only as a **backend disagreement** — interpret says one thing,
`--native` another ([Goal D](#goal-d--cross-platform--cross-backend-parity)). That
hidden, re-derived state is the same shape of machinery
[Goal E](#goal-e--predictable-memory-the-programmers-model-is-the-truth) removes from the
memory model: a counter or convention that quietly decides an outcome the source never
names.

Three examples from loft's own internals:

- **variable slots** — slot numbers were once worked out per pass; two passes could
  land on different numbers and collide.
- **the store reference count** — a hidden counter, not the scope, decided when a value
  died (removed in plan-57; see Goal E above).
- **native pre-evaluation identity** — the collect walk and the emit walk each
  re-derived a hoisted sub-expression's name from a codegen counter, and disagreed when
  the counter drifted (the `PreEvalSet` work; see
  [COMPILER.md § Synthesised-identity stability](COMPILER.md#synthesised-identity-stability--the-counter-coupling-hazard)).

When we find a pattern like this we do **not** tolerate it. Tolerating wears four
respectable disguises, and we refuse all of them: **patch one site** (the siblings stay
broken); **add a workaround or a guard** (a new rule someone must maintain — itself fresh
friction, against [Goal F](#goal-f--friction-free-surface-the-language-serves-the-programmer-not-the-compiler));
**file it for later** (you re-pay to re-derive the scope you understand right now); or
**add an assertion and move on** (both derivations stay alive, so it returns).

We engineer the **class** away, on the same four-step arc the slot and reference-count
work used:

1. **Observe** the brittleness — name the state being re-derived, and the silent failure
   it causes.
2. **Reify** it — give the state one explicit home (the slot table; `PreEvalSet`), so the
   implicit thing becomes a value you can hold and inspect.
3. **Prove coverage** — check that *every* routine can be served from that one home,
   **without changing behaviour yet**. This is a gate, not a courtesy: cutting over before
   coverage is proven breaks the one consumer that cannot yet read from it. A standing
   detector that reports the remaining gaps (`LOFT_STORE_GUARD` for memory; the pre-eval
   drift report for identity) is how this stays measurable.
4. **Cut over** — the routines *read* the one home, and the second derivation is
   **deleted**. The class is gone only when nothing re-derives the state, because then
   there is nothing left to disagree.

The definition of done is step 4: until the second derivation is deleted, the bug can
come back. Making the hidden state **visible** (steps 1–2) is diagnosis; making it the
**only** state (step 4) is the cure — the constructive form of Goal E's law that a
divergence is fixed by *removing* hidden machinery, not by adding cleverness.

#### The largest instance — the two backends, and where the arc must stop at step 3

The biggest re-derivation in the whole project is the **interpreter vs the native
backend**: two implementations that each derive "what this program means" from the same
IR. Its silent-failure symptom is exactly [Goal D](#goal-d--cross-platform--cross-backend-parity)
backend divergence — interp says one thing, `--native` another, no error. The pattern is
*fractal*: #272 was a tiny re-derivation **inside** native (collect vs emit, pre-eval
identity from a counter) that surfaced as a full parity break (`inline=true` vs `false`).
The small disagreement *was* the large one — fixing the sub-derivation restored parity.

The **native-library work** (`@PLN11` "Data as a store" + store-backed IR; the C71
shared-store dispatch in [NATIVE.md § N9](NATIVE.md#n9--native-library-shared-store-dispatch-c71))
is this same arc applied to the substrate, not a one-off fix:

- **Reify** — data and the IR *become a store*, one substrate both backends index into,
  instead of each holding its own representation.
- **Cut over** — a native library and its interpreted caller share **one** store, not two
  copies kept in sync. The N9 open item *"binary schema, no source re-parse"* is the same
  move: re-parsing source to recover a library's types is a second derivation of the
  schema; the binary schema is the reified single truth both sides read.

But here the arc **cannot reach step 4 at the top level**: the three backends are
deliberately three implementations (interp for debug/wasm, native for speed — Goal D), so
"make native read the interpreter's execution" is impossible *by design*. This is exactly
the case the arc's fallback covers — *when you cannot delete the second derivation, make
its violation loud.* That is what the **differential backend sweep** is (Goal D's Check:
"identical output and diagnostics — zero differences"): the standing detector for the one
re-derivation we are forced to keep. So the native-library work and this principle are one
effort from two directions — **every piece of shared state reified into one truth (one
store, one schema, one IR, one node-identity) is one fewer place the backends can silently
disagree**, shrinking the irreducible remainder down to just the execution strategy, small
enough that the differential sweep can actually guard it.

---

## Goal F — Friction-free surface (the language serves the programmer, not the compiler)

**Definition.** No syntax, annotation, or blocking error exists just to feed the
*compiler's* analysis. The programmer writes what expresses intent. The compiler's
internal needs — lifetime tracking, confinement proof, slot assignment — are the
**implementation's** problem and stay invisible. When the compiler cannot prove
what it wants, it **takes the cost itself** (a missed optimisation, a postponed
feature); it does **not** hand the programmer a form to fill in. Warnings are the
one allowed channel: they describe the consequences of the programmer's *own*
choices, and they are **free to ignore**.

**Why — the Rust grievance, stated plainly.** Rust bought safety by pushing its
analysis onto the syntax: `'a` lifetimes, `move`, turbofish, `Pin` — ceremony that
serves the borrow checker, not the author. Once that syntax ships, it cannot be
removed; Rust's long-running ergonomics effort shows how hard that is to undo. loft
refuses the first step: **rather miss a feature than add friction.** This is paid
for by Goal E's bet — safety from a runtime discipline, not a static proof. No
static proof means **no proof obligations to put on the user**. E removes the
machinery from the *model*; F removes it from the *syntax* — the same coin.

**The friction test.** For any syntax or error, ask: *does this serve the
programmer or the compiler?*
- a type on a signature (documents intent), a warning about an unused value (the
  programmer's choice, ignorable) — serves the programmer: **keep**;
- a lifetime annotation, a `move`, a "restructure it so I can prove it" error —
  serves the compiler: **refuse** — infer it, default it, or drop the feature.

**Missing a feature is the preferred side — and is not the same as friction.**
Refusing an operation the language **cannot do safely** (the unsound-capture error
in `parallel{}` → "use `for par`") is *missing a feature*, not pushing work onto
the user. It says "this isn't available yet, here is the supported path" — never
"annotate X so I can allow it." The line is exact: an error that **bounds the
language** is fine; an error that **bounds the user into serving the compiler** is
the friction F forbids.

**Grounding.** F's deeper reason is the [Purpose](#purpose--what-loft-is-for) — *do
the hard plumbing so it is fun to pick up*. That makes friction **fatal, not
cosmetic**: a Goal-F violation means the plumbing isn't finished, so whoever picked
it up gets a *fight* instead of fun, and leaves. The crawler dogfood made this
literal — its "survival guide" of store-lifetime workarounds (C1/C3/C4/C18 →
[loft#248](https://github.com/loft-lang/loft/issues/248)) *is* the plumbing not yet
done.

**Check.** No feature design ever reaches "…and the user must write X so the
compiler can Y." When it does, the *feature* is wrong, not the user — infer X,
default it, or cut it. The store-confinement analysis is the worked model: zero
user-facing surface, and a **silent fallback** to a higher watermark when it cannot
prove confinement — the programmer never learns the analysis exists.

**F extends to the OUTPUT, not just the syntax (user-stated, 2026-07-31).** Friction
is also every line a tool prints that the reader did not need. So the same test governs
what a command SAYS: silence when nothing needs acting on, no plan numbers or
not-yet-implemented apologies aimed at users, the full explanation reserved for the
moment something is actually wrong, and no ceremony. A tool that reports its own good
health teaches people to skip the line where it eventually reports the opposite.  See
§ [The destination is BORING](#the-destination-is-boring--a-tool-you-notice-only-when-it-is-missing).

**F beyond the compiler — the engine takes the fastest path (user-stated core
value, 2026-06-10).** The same test governs the whole lavition surface, not just
syntax: *we do not bother the developer with details where they don't help/aid
them* — and no developer wants slow network traffic, so speed is never a knob.
The engine-host transport stack is the worked model (@PLN18 05a): the kernel
negotiates UDP per client *inside* the WS handshake (an `X-Loft-UDP` response
header — no loft code on either side touches it), `sync_send` rides the fastest
path that client supports, web pages stay on WS from the very same call, and a
silent keepalive timeout falls the path back — meaning-code never branches on
transport. The bulk channel (05c, parked) applies it one level deeper:
broadcast vs unicast vs WS is **measured per seat** and picked automatically.
The pattern generalizes: defaults = the fastest correct path; what surfaces is
read-only introspection (`udp_bound`), never a switch the developer must set to
get speed.

**Relation.** F is **orthogonal** to the useful → safe → predictable axis: it limits
the user-friction cost of delivering any of A–E. It is closest to E (both keep
hidden machinery from leaking out), but E guards the **memory model** while F guards
the **whole syntax** — and, per the paragraph above, the engine surface too.

---

## Goal G — Performance (every routine pulls its weight)

**Definition.** loft is a **fast implementation**, measured — not against its own
previous release, which compares to nothing outside the project, but against
**reference implementations in industry-standard languages** (pure Rust; C#
admissible).  Every public routine of the stdlib and of a shipped library keeps its
native time within a stated per-class bar of its reference twin, on a fixed workload
whose lanes are proven output-hash-equal before any comparison is admitted.  The
contract is [formal/performance.md](formal/performance.md) (`Perf-Like` /
`Perf-Weight` / `Perf-Twin` / `Perf-Cure`); the model harness is the drawing
library's `bench/` (loft#1426), and @PLN158 generalizes it.

**Scope.** This is a *within-the-bar* goal, and it amends nothing in Goal E's axis
statement: loft does not claim to surpass Rust at performance — it claims not to fall
behind the industry by more than the stated bar, per routine, measured per release.
And the CURE discipline is what keeps G from eating B: the twin measures and never
ships, so getting fast never costs the libraries their readability.

**Check.**
- `make release-checklist` → `M-perf-pass` read on the cycle (the per-library
  `bench/compare.py` tables, hashes agreeing, ratios within the bar).
- `python3 scripts/rule_tags.py check` green with `formal/performance.md`'s
  deviation count shrinking — each `D-perf-n` names a routine class below the bar.
- `make profile` attribution on any flagged routine: a loft hot loop matching its
  reference's is an ENGINE deviation (like D-perf-1), never a library rewrite.

**Relation.** G is the measured half of Purpose § *Legible cost*: that section keeps
performance-critical *decisions* in the programmer's hands; G keeps the *implementation
underneath* those decisions competitive, so a maker who did everything right is not
slow anyway.

---

## The two floors — why dogfood is paused, and when it resumes

The dogfood loop normally sets the agenda (CLAUDE.md § "Development cadence"). It is
**paused right now, on purpose** — because the loop did its job and hit two walls:

- it **kept surfacing instability** → the **soundness floor** (Goal A);
- it **fought the library/package structure** → the **structure floor** (Goal B's
  packaging half).

Building a game on either un-cleared floor means building on a base that can still
shift under you. So Goal C (the game work especially) waits on A and B — *by
choice, not neglect*.

The risk of a deliberate pause is that open-ended floors make it permanent by
drift: soundness can always take one more sanitizer leg, packaging one more polish.
So each floor has an **explicit resume bar, tied to what a game actually needs** —
not "all of A" or "all of B":

- **Soundness floor — cleared when:** the sanitizer gate is green on `main` **and**
  the Miri/ASan set covers the surfaces the games use (eval stack, store
  claim/copy/resize, vectors, fn-refs, text). *Not* "every Goal-A coverage leg
  shipped."
  **Reading (2026-08-09): the gate is RED, and coverage is still narrower than the Checks
  state.** @PLN85 closed and every *tracked* store-lifetime bug is fixed, but three things
  keep this from reading MET.
  (i) **The nightly is failing now.** `miri.yml` on `main` was green 2026-08-05 through
  2026-08-08 and went red on 2026-08-09: the ASan interpreter leak gate fails on *both*
  ubuntu and macOS, and the debug-assertions sweep fails with it. This is the floor's own
  Check, so until it is green again the floor cannot clear. A fresh regression, not a
  standing state — the failing run swept `8350a2ab`; the current tip is not yet swept.
  (ii) **Coverage is narrower than the Checks above state**, though less so than it was.
  The `stack_align_guard` sweep runs 16 in-process test binaries — but that is 16 of 179,
  and `tests/data_structures.rs` runs wholly in-process yet is not among them, so a sweep
  described as covering "the corpus" does not. `LOFT_STORE_GUARD=1` (Goal E's Check) *is*
  wired now: `miri.yml`'s debug-assertions sweep sets it, and its enforced twin — the
  `reclaim_guard` `assert_eq!` in `scopes.rs` — hard-gates there through
  `cfg(debug_assertions)`. But it reaches 5 targets (`--lib --test issues --test wrap
  --test strings --test frame_vars`), and `[profile.dev.package.loft] debug-assertions =
  false` compiles the assert out of ordinary test builds, so most of the corpus never arms
  it. (iii) **The class is not yet retired by construction**: the fuzz/sanitizer corpora
  that must prove it are now standing (@PLN53 + @PLN54 both closed 2026-07-10), so the
  **one** remaining step is the Cluster C / H10 fold — land it and the silence becomes
  proof. Tracked in [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md).
- **Structure floor — cleared when:** the libraries a game depends on (graphics,
  game_client / game_protocol, server) are extracted, installable, and
  version-stable through the registry. *Not* "the whole package toolchain
  polished."
  **Reading (2026-08-09): MET, and the caveat that used to qualify it is closed.**
  The [`loft-lang/registry`](https://github.com/loft-lang/registry) carries 34 signed
  (`index.json` + Ed25519 `.sig`), per-version-`sha256` packages with dependency resolution
  (`server` → `web >=0.1`; `hex_terrain` → `hex_grid`) — `graphics`, `game_protocol`, `server`,
  the hex-world stack, `web`, `crypto` (the zero-trust lib), assets, docs — each on its own
  semver track. `loft install <name>` resolves and fetches end to end, including transitive
  deps. Extraction + installability + version-stability read true.
  **`installable` now means a working artifact, not just resolve + fetch.** The nightly
  `registry-validation` gate passes every one of the 34 package legs, and has been green on
  `main` on most nights since 2026-07-31 (08-07, 08-08 and 08-09 consecutively). Both faults
  that used to hold it red are fixed: the workflow installs `libasound2-dev`, so `graphics`
  builds `--native` on a clean runner, and `hex_terrain` validates green against current
  loft. That closes the ecosystem rot this caveat recorded — including the C86 H-Copy
  compatibility break, which is the exact failure the wide-release bar's **gate 5** exists
  to prevent.

The structure floor's bar reads MET without a caveat, and the soundness floor is close
on substance but currently **red on its own gate**, so **the pause is at its end, not its
middle** — held open by a regression to clear, not by a body of work still to do. When
both read true the dogfood loop sets the
agenda again — and the work above it is no longer *extracting* libraries but raising
each to the **per-library quality bar** (cross-target parity + `verified`,
[LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md)) — a Goal-C-and-up concern, not a
foundation floor. Until the soundness floor confirms, A stays the foundation work
*because* C and D asked for it.

---

## How the goals relate

```
       useful ───────────► safe ───────────► predictable
   C (capability)      A (soundness)      E (predictable memory)
   B (release)         D (parity)             the programmer's model
        │                                     is the truth
        └── paused until the soundness floor (A) and
            structure floor (B) clear, then resumes as agenda-setter

   F (friction-free surface) ── orthogonal: the user-friction cost
        of delivering any of A–E must stay near zero
   G (performance) ── orthogonal: each routine within the bar of its
        industry reference twin, without costing B's readable libraries
```

Dogfood makes loft *worth using*. The sanitizer makes it *safe to keep using*. Goal
E makes it *predictable to reason about* — safe is not enough if the programmer
cannot hold the memory model in their head. A (*no corruption*) and E (*no
surprise*) are different properties: Rust achieves the first but not the second, and
separating them is where loft aims to surpass it. Goal F sits across all of them:
each of A–E must be delivered **without billing the programmer** in syntax or proof
obligations.

## See also

- [CLAUDE.md](../../CLAUDE.md) § "Development cadence — the dogfood loop" — Goal C.
- [plans/finished/53-sanitizer-ci-lever/](plans/finished/53-sanitizer-ci-lever/README.md)
  — Goal A: the sanitizer lever and the stack-alignment work.
- [plans/53-program-level-fuzzing/](plans/53-program-level-fuzzing/README.md)
  — Goals A/D: program-level fuzzing feeds both the UB sweep and the cross-backend
  differential.
- [plans/54-sanitizer-coverage-expansion/](plans/54-sanitizer-coverage-expansion/README.md)
  — Goal A continuing: remaining sanitizer-coverage items.
- [plans/2-vector-store-watermark/fix-design-store-lifetime.md](plans/2-vector-store-watermark/fix-design-store-lifetime.md)
  — **Goal E**: the predictable-memory fix design + `LOFT_STORE_GUARD` detector
  (free a vector store at its scope, decoupled from the slot).
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction/README.md) — Goal B's structure floor: the package-ecosystem extraction.
- [PKG_REGISTRY.md](PKG_REGISTRY.md) / [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md) — Goal B's registry on-ramp.
- [API_SURFACE.md](API_SURFACE.md) — Goals F + B at the named-API level: verifying the language/stdlib **and** the libraries for dup/confusable/undocumented/footgun functions ([LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md) is the per-library bar).
- [ROADMAP.md](ROADMAP.md) / [PLANNING.md](PLANNING.md) — feature backlog feeding Goal C.
