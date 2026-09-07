<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# STRONG_POINTS.md — the strengths that win a person, and the gap that loses them

This doc is a **lens for evaluating work**, not a brag sheet. Use it to ask of
any change: *does this raise a strong point higher, or does it close the gap that
would turn off the exact person the strong point attracts?*

The core idea it turns on:

> **A strong point is a promise that draws a specific person in. The gap on that
> same topic is precisely what turns *that* person off, seconds after they
> arrive.** So the gap is not a footnote to the strength — it is the
> **highest-leverage work**, because it is load-bearing for the one audience the
> strength just won.

Someone picks loft *for* live data migration, then hits the rename gap on their
first refactor and leaves. Someone picks it *for* predictable memory, then hits a
store-lifetime surprise and concludes the promise was hollow. **A strength with
an open turn-off is worse than no claim at all** — it spends trust to attract
someone, then breaks it on contact. That is the failure this doc exists to
prevent.

**Reach a strength even when it isn't what you'd use loft for.** The entries here
are *not* the author's intended use cases. loft is built as a foundation for
lavition and games ([GOALS.md § Purpose](GOALS.md#purpose--what-loft-is-for)) —
yet **scripting** (point 9: embeddable + sandboxed) is a genuine strong point that
*falls out of the architecture* (the integrated store + the Rust-crate embedding),
and it earns real development not because anyone here writes scripts, but because
it **is** a strength and the work is to **reach** it — to make the promise actually
hold. A strong point is worth reaching by the plain fact that it is strong: it
wins a person who is **not you**, and that is reason enough. This is the same
[GOALS.md § Purpose](GOALS.md#purpose--what-loft-is-for) rule — *adoption is a
result, not a steering input* — turned into a build rule: you don't chase users,
but when the architecture hands you a real strength, you finish it so the people
who *do* want it aren't turned away at the door. A strength is reached, not
assumed; the **Raise it higher** line on each entry is the unfinished part of that
reach.

So each entry has four parts:

- **Strength** — what is genuinely good today.
- **Who it wins** — the person this promise attracts.
- **The turn-off** — the gap on the *same topic* that loses exactly that person
  right after they arrive. This is the mark we cannot miss.
- **Raise it higher** — the work that closes the turn-off and pushes the strength
  further, with a check to tell whether we're there.

This is the positive-space sibling of [GOALS.md](GOALS.md) (the aims A–F) and
[FORMALIZATION.md](FORMALIZATION.md) / [INCONSISTENCIES.md](INCONSISTENCIES.md)
(the rough spots). When a turn-off here is closed, delete it from this doc and
from its canonical home in the same change — a turn-off that reads as open when
it's fixed wastes the reader's caution, and one that reads as closed when it's
open is the exact trust-break above.

---

## At a glance — strength → the person it wins → the turn-off to close

| # | Strength | Who it wins | The turn-off that loses them |
|---|---|---|---|
| 1 | The integrated store | builders of data-heavy apps tired of the ORM seam | libraries on top don't meet the bar yet |
| 2 | Live data migration | prototypers who edit structs while the game runs | rename loses the field's data — on their first refactor |
| 3 | Predictable memory | people who want a memory model they hold in their head | a store-lifetime surprise makes the "no exceptions" promise hollow |
| 4 | Friction-free surface | refugees from Rust's lifetime ceremony | a needed shape is *refused*, or an error makes them serve the compiler |
| 5 | Soundness | people betting a product on "it won't corrupt my data" | latent UB that passes today and corrupts after a toolchain bump |
| 6 | One source, three backends | "write once, share a link" demo/tool authors | the browser build behaves differently from the desktop one |
| 7 | Approachable parallelism | people who want safe parallelism without race-hunting | they ship to web and `par` silently runs sequentially |
| 8 | Inherits Rust's ecosystem | people who need a crate (regex, http, decimal) *now* | the crate exists but isn't bound, and binding is painful |
| 9 | Embeddable + sandboxable | hosts running player scripts / mods | a script escapes, or its effects can't be rolled back |
| 10 | Ships and is legible | a newcomer who just discovered loft | they install, hit a library that fights them, and leave |
| 11 | The method is the moat | contributors who value a project that fixes the class | the rough-spot lists stop shrinking |
| 12 | Keeps running under fault | game/server authors who can't take a crash-on-fault | it isn't built yet (still halts); a degraded null can be hard to trace to its source |
| 13 | Layout control with a safety net | performance-conscious devs turned off by "only C++ is valid" | the cache win only shows on `--native`; or layout becomes a mandatory tax, not an optional power |

Read top-down: the turn-offs are roughly in trust-cost order — #1–#5 break the
promises that *are* loft's identity, so they cost the most to leave open.

---

## 1. The integrated store — the heap *is* the database

**Strength.** The schema is data (`Stores.types` / `Parts`, mutable at runtime), a
value **is** a self-describing record against it (`DbRef`, name-keyed), and
serialization is just reading the record. Language types, runtime values, heap
layout, and wire form are **one** representation — no object-relational seam to
glue or to drift. See [DATABASE.md](DATABASE.md) and
[GOALS.md § Why a language, not a store bolted on](GOALS.md#why-a-language-not-a-store-bolted-onto-an-existing-one).

**Who it wins.** Builders of data-heavy apps — servers, CRUD tools, ETL — who are
tired of keeping app objects and database rows in sync by hand.

**The turn-off.** They build the *app*, not just the store, and the libraries that
make a server or a CRUD tool pleasant **do not meet the bar yet**
([GOALS.md § goals at a glance](GOALS.md#the-goals-at-a-glance)). The substrate is
coherent; the product layer on top is unfinished, so the coherence they were
promised stops at the library boundary.

**Raise it higher.** Bring the consumer libraries (server, data) to
[LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md), so the integration the store
promises is felt all the way up. *Check:* a real data app builds and runs on HEAD
from registry libraries alone, with no hand-written seam.

---

## 2. Live data-structure migration

**Strength.** Edit a struct or enum *while the program runs* and the existing data
survives — add/remove/reorder a field (missing→default, unknown→ignored), add/
remove/reorder a variant (removed→null sentinel, never a wrong variant), widen a
numeric type (coercion). This is impossible for a bolt-on store (recompiling kills
the running program); loft does it because changing a struct is changing schema
*data*. Governing rule: **leniency is the feature** — only silent *wrong* data is
forbidden, silent *graceful* data is the preservation you want.

**Who it wins.** Prototypers and game devs who iterate structures live and refuse
to lose the running game's state to a recompile.

**The turn-off.** **Rename.** The first time they rename a field — an ordinary
refactor — it reads as delete+create and the value is lost. The single most
common edit hits the single real gap, on day one. (It warns rather than dropping
silently, but the data is still gone.)

**Raise it higher.** Build the **migration setter** for the old field (declares
old name + type, routes the value to the renamed field) — loft has no setter
concept yet, so this is its own small language feature
([GOALS.md § what survives an edit](GOALS.md#why-a-language-not-a-store-bolted-onto-an-existing-one)).
*Check:* a name-keyed round-trip survives a field rename with a migration setter,
locked by a regression test.

---

## 3. Predictable memory — the source is the truth

**Strength.** One rule, no exceptions: **a value's heap memory is freed when its
scope ends.** Write a vector in a block, it's freed at the block's end — full
stop. The cleverness (last-use, slot-vs-heap) stays in the implementation. The
reference count is *gone* (plan-57 / [@PLN2](https://github.com/loft-lang/plans/issues/2),
2026-06): no hidden counter decides death — the scope does. The stated aim is to
**surpass Rust on safe-AND-predictable**
([GOALS.md § Goal E](GOALS.md#goal-e--predictable-memory-the-programmers-model-is-the-truth)).

**Who it wins.** People who want a memory model small enough to hold entirely in
their head, where a surprise is a knowable, fixable bug — not "what did the
compiler decide?"

**The turn-off.** A single store-lifetime surprise — memory held past its scope,
or a divergence from the plain reading — makes the *whole* "no exceptions" promise
read as marketing. The promise is binary: one exception they have to learn and the
model is no longer the one they were sold. The crawler dogfood's survival guide
([loft#248](https://github.com/loft-lang/loft/issues/248)) is this turn-off made
literal.

**Raise it higher.** The hard assertion exists: `scopes.rs`'s `reclaim_guard` arms on
`cfg!(debug_assertions)` or `LOFT_STORE_GUARD`, and the nightly debug-assertions sweep
gates on it. What is missing is **reach** — the sweep covers 5 test targets, and
`[profile.dev.package.loft] debug-assertions = false` compiles the assert out of ordinary
test builds, so most of the corpus never arms the guard and drift can still land
unobserved. *Check:* the guard is silent corpus-wide **and** the assertion holds and keeps
holding.

---

## 4. Friction-free surface — the language serves the programmer

**Strength.** Reads like Python, statically typed with almost no annotations.
**No lifetime annotations, no `move`, no turbofish, no `Pin`, and no `usize` index
cast** — `v[i]` takes any integer, where Rust forces `i as usize` / `len() as i32`
all over the *common path*; loft has **one integer type and no special index type**,
so idiomatic code needs **zero `as`**. No syntax exists to feed the compiler's
analysis. When the compiler can't prove what it wants, it takes the cost itself;
warnings are the only channel back, and they're free to ignore
([GOALS.md § Goal F](GOALS.md#goal-f--friction-free-surface-the-language-serves-the-programmer-not-the-compiler)).

**Who it wins.** Refugees from Rust's borrow-checker ceremony who want the safety
without filling in proof obligations.

**The turn-off.** Two shapes, opposite directions. *Either* a pattern they need is
**refused** (a scalar mutated by several closures,
[#314](https://github.com/loft-lang/loft/issues/314)) and "rather miss a feature"
costs them the program they came to write — *or* a diagnostic crosses the line
into "restructure it so I can prove it," and the very ceremony they fled is back
in a new coat. Both lose the same person.

**Raise it higher.** Keep the line exact — an error that *bounds the language* is
fine; one that *bounds the user into serving the compiler* is forbidden — and
drive the type/conversion rough spots ([TYPING_RELATION.md](TYPING_RELATION.md)
R1–R3, the `*_hint` channels) to zero so fewer needed shapes get refused. *Check:*
no feature design reaches "…and the user must write X so the compiler can Y"; every
refusal hands back a supported path. *And:* idiomatic consumer code — a well-built
library used as intended (e.g. the hex-world) — contains **zero `as`**; any `as`
there means a width leaked that the types should have carried (the bar Rust's `usize`
breaks by construction). A rare *deliberate* narrowing at a write-time edge is the
allowed exception, not the common path.

---

## 5. Soundness — it won't corrupt your data

**Strength.** loft does not produce wrong or corrupt results from undefined
behaviour, pursued **by construction** (one invariant, enforced at the chokepoint,
bad state impossible) and backed by standing sanitizers (Miri, `stack_align_guard`,
ASan, fuzzing) that run in CI. A well-grounded safe-Rust crate **cannot segfault
loft** the way a C extension crashes CPython
([GOALS.md § Goal A](GOALS.md#goal-a--soundness-no-silent-corruption)).

**Who it wins.** People betting a product or a business on a floor that does not
silently corrupt their data — the AS/400 "software that does not fail for software
reasons" appeal.

**The turn-off.** **Latent UB**: a bug that passes every test today and surfaces as
silent corruption after a rustc/LLVM bump or on macOS-ARM's aligned reads. The
person who chose loft *for* this guarantee is exactly the one a single
corrupted-after-upgrade record loses forever — "passes today" is not the promise
they bought.

**Raise it higher.** Keep expanding sanitizer coverage (plan-54) to every surface
the consumers actually use (eval stack, store claim/copy/resize, vectors, fn-refs,
text), so the guarantee is continuous, not a one-time clean run. *Check:* the three
Goal-A checks (guard/asan green, `stack_align_guard` fires zero times corpus-wide,
nightly Miri green) hold **and keep holding** as code lands.

---

## 6. One source, three backends — and a one-command share

**Strength.** The same `.loft` source runs on three backends — interpreter
(instant startup), **native** via rustc (the shipping default, for speed), and
**wasm** for the browser — and `loft --html` turns a program into a single
shareable HTML+WebGL file. "Write once, runs in the browser and on your desktop"
is demonstrated (Brick Buster; the 24-demo gallery). See
[HTML_EXPORT.md](HTML_EXPORT.md), [NATIVE.md](NATIVE.md).

**Who it wins.** Authors of demos, toys, and tools who want one source and a
share-by-link deploy with no toolchain on the player's side.

**The turn-off.** The three backends are **three implementations**, so a program
that works on the desktop and behaves *differently* in the browser breaks the
"write once" promise at the worst moment — after they've shared the link. Backend
divergence (interpret vs `--native`, no error) is the project's single largest
re-derivation and a recurring class.

**Raise it higher.** Hold the differential sweep at **zero divergence** — run one
shared program set on interpret/native/wasm and require identical output *and*
diagnostics ([Goal D](GOALS.md#goal-d--cross-platform--cross-backend-parity)).
Every shared fact reified into one truth (one store, one schema, one IR) is one
fewer place the backends can disagree. *Check:* the differential run reports zero
differences; per-backend-green alone does not count.

---

## 7. Approachable parallelism

**Strength.** `par` / `par_light` give parallel loops with **store isolation** —
each worker has its own store, so there is no shared-mutable footgun to reason
about. Safe parallelism without a data-race surface. See [THREADING.md](THREADING.md).

**Who it wins.** People who want to speed something up across cores without
becoming concurrency experts or hunting races.

**The turn-off.** They prototype on the desktop where `par` is parallel, then ship
to the web — where `par` runs **sequentially**
([C3, accepted](DESIGN_DECISIONS.md#c3--wasm-par-runs-sequentially)) — and the
speedup they designed around silently isn't there. The accepted trade-off becomes a
broken expectation for anyone who picked loft *for* the parallelism *and* targets
the browser.

**Raise it higher.** Make the bound **loud, not silent** — surface WASM's
sequential `par` where the author meets it, not only in a design-decisions doc — and
keep the door open to real wasm threading. *Check:* a web program relying on `par`
for throughput gets a visible signal (doc, warning, or introspection) that it runs
sequentially, rather than discovering it by profiling.

---

## 8. Inherits the Rust ecosystem — and its stability

**Strength.** loft is built *on* Rust, so it binds a crate (`regex`, `reqwest`,
`rust_decimal`, …) instead of reimplementing it — and gets memory safety, maturity,
and `cargo`'s reproducible builds *with* the library. Because loft *is* Rust, the
binding is **in-language** (loft value ↔ Rust value, no C ABI), lower-friction than
CPython's C-API glue. See [BROADENING.md](BROADENING.md#lofts-genuine-differentiators).

**Who it wins.** Someone who needs a specific capability *now* — a regex engine,
an HTTP client, decimals, CSV — and expects "it's Rust underneath" to mean "I can
have it."

**The turn-off.** The crate exists on crates.io, but it **isn't bound yet** (crates
are bound one-per-need, the dogfood way, not wholesale) — so the person who came
for instant access hits "bindable, not bound," and the **binding ergonomics** are
the honest gap. "It inherits the whole ecosystem" reads as false the moment they
need a crate nobody has bound.

**Raise it higher.** Lower the cost of binding a crate until an author can do it
themselves in-session, and bind the high-demand crates as consumers ask. *Check:* a
representative crate goes from unbound to usable in loft in one focused sitting,
following the documented path — no compiler change required.

---

## 9. Embeddable and sandboxable

**Strength.** loft is a Rust crate you drop into an application to script logic,
hot-reload config, or run untrusted code — and the sandbox (`@PLN86`) admits player
scripts and mods with **admission-time validation**: capability, library, loop, and
recursion limits enforced before a script runs, switched on by a `[sandbox]` policy.
See [SANDBOX.md](SANDBOX.md). This is the **worked example of reaching a strength
you wouldn't use yourself** (see the framing above): loft is for games and
lavition, not for running other people's scripts — but the architecture makes
scripting strong, so it gets finished for the host that *does* want it.

**Who it wins.** A host — a game, an editor, a service — that wants to run player
scripts or mods without the script being able to harm the host or other players.

**The turn-off.** They need two guarantees, and only one ships. **Admission** limits
are enforced (checks S1–S5 green); the **runtime-containment** half — the
transactional store that rolls back a script's effects, and the `run_script`
fault-isolation boundary — is **post-v1**. A host that admits a script today cannot
yet *undo* what a misbehaving-but-admitted script did, which is exactly what
"run untrusted code safely" implies to them.

**Raise it higher.** Ship the runtime-containment half — the transactional world and
the `run_script` boundary — so a script's effects are reversible and a fault is
isolated. *Check:* a misbehaving admitted script's writes roll back cleanly and do
not touch the host or other sessions.

---

## 10. It ships, and the value is legible on contact

**Strength.** loft releases on a monthly cadence (current `2026.9.0`),
`loft install <name>` fetches published libraries end to end, and a newcomer meets
the value in a browser — [playground](https://loft-lang.org/loft/playground.html),
[gallery](https://loft-lang.org/loft/gallery.html), a playable game — with no
install. Libraries are written in loft (no C, no FFI glue) and **every doc page runs
as a test** ([GOALS.md § Goal B](GOALS.md#goal-b--release--legibility)).

**Who it wins.** The newcomer who just discovered loft and is deciding, in the first
ten minutes, whether it's real.

**The turn-off.** They get past the playground, reach for a library to build their
*own* thing, and it **fights them** — because the libraries on top do not meet the
bar yet, and the dogfood loop is paused on the soundness and structure floors. The
on-ramp delivers; the first real step off it stumbles, and a first impression
doesn't get a second try.

**Raise it higher.** Clear the two floors (soundness A, structure/packaging B) so the
dogfood loop resumes and the libraries reach [LIBRARY_CHECKLIST.md](LIBRARY_CHECKLIST.md).
The acceptance test is loft's own: **a thing is done when picking it up is *fun***,
not when it is feature-complete. *Check:* on a clean machine, a new user installs
loft, runs a first program, and `use`s a library to build something — from the docs
alone, with no fight.

---

## 11. The method is the moat

**Strength.** loft's most durable advantage is the **discipline that produces the
rest**: it engineers the *class* away instead of patching one site, debugs
matrix-first (a coherent explanation is a hypothesis, not a conclusion), and holds
its own reasoning to Goal E's law — *the stated thing must match reality.* This very
doc is that method applied to its own marketing: no strength stated without the
turn-off that guards it.

**Who it wins.** Contributors and serious adopters who can tell the difference
between a project that fixes bugs and one that retires bug *classes* — and who stay
for the second kind.

**The turn-off.** The discipline is only visible while it's *working*. If the
rough-spot catalogues ([FORMALIZATION.md](FORMALIZATION.md),
[INCONSISTENCIES.md](INCONSISTENCIES.md),
[STABILITY_ROADMAP.md](STABILITY_ROADMAP.md)) **stop shrinking**, the "we drive
deviations to zero" claim becomes a claim and nothing more — and the person who
stayed for the method is the first to notice.

**Raise it higher.** Keep the deviation lists moving toward zero release over
release, and keep writing them down — the strength is that the lists *exist* and
*shrink*. *Check:* each release's rough-spot count is lower than the last, or the
delta is named and owned.

---

## 12. Keeps running under fault — it degrades, it doesn't stop

**Strength.** A calculation that can't produce a value — `s / 0`, an overflow, an
out-of-bounds index, a deref of an absent value — yields **null** and the program **keeps
running**; it never halts the run or skips a later statement. Like a spreadsheet, one cell's
bad formula shows null in that cell and never stops the others recalculating — a fault is
*local*, it degrades one value, not the whole run. It does this with **no exception machinery**:
no stack unwinding, and no `finally` block to get wrong — a fault is a **value, not a thrown
control-flow event**, so it can't leave the half-cleaned-up state that is `try`/`catch`'s own
corruption surface. This inverts the half-true "if it compiles it works" (which really means "it
halts cleanly at the first logic fault"): loft's promise is **"it won't stop."** See
[DESIGN_DECISIONS.md C80](DESIGN_DECISIONS.md) and
[formal/operational.md § E-Uncomp](formal/operational.md).

**Who it wins.** Anyone building something that must **stay up** — and that is not just games.
A **game** dev iterating on something half-working (one bad mob doesn't freeze the world); a
**server** author who refuses an outage on one bad request; at the limit a **kernel** where
stopping *is* a dead machine. Termination is the larger failure for any long-running system, so
availability-as-a-default — not a framework bolted on — wins all of them.

**The turn-off.** Two, on the same topic. First, **it isn't built yet**: the runtime still
traps/halts on these faults in development (the old C66 behaviour, tracked as
[operational.md D-op-4](formal/operational.md)) — the promise is *decided, not delivered*.
Second, the inherent one: **silent degradation can hide the cause** — you see a null
*downstream*, not *where* it first arose, the mirror image of Rust's halt-at-the-source
pinpointing the bug. The person who loves "keeps running" is the same one who curses a null they
can't trace.

**Raise it higher.** Land D-op-4 (uncomputable → null + continue in every mode), then close the
second turn-off with the **opt-in debug log + null provenance** — when the programmer asks, show
*where* a degraded value first went null, so "keeps running" never costs them the root cause.
*Check:* a deliberate `s/0` deep in a call chain keeps the program running AND, with the debug
log on, names the originating site.

---

## 13. Layout control with a safety net — you place the data

**Strength.** You decide the **allocation topology**: group data that belongs together into
one store (one allocation, packed, one lifetime, freed as a unit) and split unrelated data into
another (independent lifetimes, no false coupling). The store **is** the arena and `DbRef` is
the index-pointer — and, unlike C++ or Zig, the ownership/`deps` discipline holds *across* the
grouping, so you keep the safety net while you author the layout. That combination is an empty
quadrant elsewhere: C#/Unity and GDScript hide layout entirely (the interleaved-allocation
pitfall), C++ gives layout but no enforced safety, and Rust gives both but fights hardest on the
shared-mutable object graphs games are made of. loft occupies **approachable + layout control +
enforced safety** together — data-oriented design as a default, not a framework bolted on. See
[OWNERSHIP_MODEL.md § the control story](OWNERSHIP_MODEL.md#why-this-shape--the-control-story-makers-call).

**Who it wins.** Performance-conscious developers — the "write Java like ANSI-C" and
data-oriented-design school — who want to control memory layout but are turned off by C++'s
no-safety bargain and by managed languages that hide layout altogether, and anyone uneasy with
the idea that only C++ is a valid engine language.

**The turn-off.** Two, opposite in shape. First, the *realized* cache win is a `--native`
story: on the interpreter, dispatch overhead masks locality, so someone who benchmarks the
layout control on the interpreter sees the control but not the speed and concludes it is
theatre. Second, **progressive disclosure** — if grouping and splitting stores is a decision the
programmer *must* make to write *correct* code, loft stops being "easier than Java" and becomes
a tax, losing the very approachability that wins the newcomer. The control has to be an optional
power over a sane naive default, never an upfront obligation.

**Raise it higher.** Keep layout an *optional* power — a beginner writes into a default store
and is correct with zero topology decisions — and prove the realized win on `--native` (the
~10k-body browser physics demo is the existence proof against "only C++"). *Check:* a beginner
program is correct with no store-placement decisions; and the physics demo shows the
cache/throughput win on `--native`, not merely the layout *control* on the interpreter.

---

## How to use this doc

When you pick up or finish a piece of work, run it past this lens:

1. **Which strong point does it touch?** If none, fine — not all work does.
2. **Does it close a turn-off, or raise the bar?** Closing a turn-off is the
   higher-leverage move, because it protects the trust the strength already spent
   to attract someone.
3. **Did you leave a *new* turn-off behind?** A change that strengthens one promise
   while quietly breaking another on a related topic is a net loss — the new gap
   loses the person the old strength won.
4. **If a turn-off is now closed, delete it here and in its canonical home**, in the
   same change.

The goal is never to *have* strong points — it is to make sure that the person each
one attracts is still glad they came, one step after they arrive.

## See also

- [GOALS.md](GOALS.md) — the seven goals A–G these strengths serve, each with a Check.
- [BROADENING.md](BROADENING.md) — loft's four genuine differentiators beyond games.
- [FORMALIZATION.md](FORMALIZATION.md) / [INCONSISTENCIES.md](INCONSISTENCIES.md) —
  the ranked rough spots (the negative-space companion to this doc).
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — features deliberately *cut* to protect
  a strength (read before re-proposing one).
- [CAVEATS.md](CAVEATS.md) — the concrete edge cases that bite today, with repros.
