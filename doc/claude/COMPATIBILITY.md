<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# COMPATIBILITY.md — the promise: at contract 1, loft does not break your program

> **Status: arc A of [@PLN102](https://github.com/loft-lang/plans/issues/102) (the
> stability contract), 2026-07-10.** This is the *policy*. Its mechanism is the
> `contract` version ([plans/102-stability-contract/versioning-decision.md](plans/102-stability-contract/versioning-decision.md));
> arc C is how a deprecation *steers*; arc E is which surface is frozen at 1.0.

## The promise (absolute)

> **When loft introduces `contract` 1 (the 1.0 freeze), any loft program that FUNCTIONS
> at that moment keeps running — identically — on every future loft, whatever we do. A
> deviation is not a "breaking change" to be managed; it is a BUG, and it is fixed before
> any other work.**

This is the AS/400 standard [GOALS.md](GOALS.md) names — *"the platform never broke its
users; the cost of change was paid by the maker, not the customer"* — taken at full
strength. It is the floor loft exists to be: a substrate that *sometimes* breaks a working
program betrays the whole pitch. So this is not a versioning scheme with escape hatches; it
is a commitment, and the rest of this document is how the commitment is kept.

**It is stronger than the usual "semver + deprecation window."** There is no
warn-for-a-cycle-then-remove for anything a program depends on: you do not remove a stdlib
function a working program calls, ever. Evolution is **additive**. When a change would make
a functioning program deviate, that change is wrong — reverted or reworked — not shipped
with a migration note.

## What is covered — language, errors, and libs (and the data underneath)

The promise covers **everything a functioning program can observe** — enumerated explicitly
so the scope is not quietly narrowed:

1. **The language** — both *syntax* (how code is written) and *semantics* (what it means and
   how it runs: evaluation order, coercion/narrowing, null/`??`, ownership/free timing a
   program can observe).
2. **Errors** — the frozen surface is the error **boundary**: *which* programs error versus
   compile (and, for a runtime fault a program branches on, *which* fault fires — its kind via
   `??`, not its wording). Error **text is NOT a frozen surface** — the prose stays freely
   improvable forever; no program contract depends on the exact words (our goldens are internal
   tests that update with the prose). The boundary is **one-directional** (§ below): the set of
   inputs we error on can only *shrink* post-freeze — we may stop erroring on something (a program
   that used to fail now compiles), never start erroring on a program that used to compile. *(Owner
   decision 2026-07-13: "the error texts are not frozen; we freeze when we throw an error, and can
   only limit the space we error for.")*
   The **stable machine handle** for a diagnostic is its **code** — a kebab-case kind slug rendered
   `error[text-parse-may-fail]: …`, not its prose (@PLN102 arc-E E1). A tool that must recognise a
   specific diagnostic keys on the code; the prose behind it stays freely improvable. Codes are
   assigned incrementally (a site without one yet renders `error: …` as before) — adding a code is
   additive, never a breaking change.
3. **The libs** — the standard library (`default/*.loft`) **and** published libraries: a
   program calling a stdlib function or a `use`d library keeps working.

And the surfaces a running program *depends* on, so "keeps running" includes "its data and
dependencies keep working":

4. **Store / heap layout** — a store persisted by an older loft keeps reading correctly
   (@PLN97's layout-identity hash is the authority).
5. **On-disk + wire format** — serialized data, the IR codec, any protocol: old data keeps
   reading to the same values.
6. **Package format** — `loft.toml`, the package layout, the registry: existing packages
   keep resolving.

**Warnings are NOT a covered surface — a new warning is never a breaking change.** Unlike the
error *boundary* (item 2, frozen), the warning stream is entirely non-contractual: neither
*which* code warns nor the warning text is frozen, so adding a warning to a program that used to
compile clean is **additive, not a regression**. The program still compiles and runs identically
— a warning changes only what is *printed alongside*. Warnings exist to flag *potential* problems,
and as loft learns of more of them (a new nullability nudge, a new lint, a domain-fault hint) it
must stay free to warn about them, forever — the [GOALS.md](GOALS.md) Goal F channel, the one path
allowed to bill the programmer, stays open past 1.0 precisely because a warning breaks nothing. A
build that treats warnings as errors, or pins warning output, is opting into churn the contract
does not owe it. *(Owner decision 2026-07-14: "warnings are never a contract breakage — they are
there to warn programmers about potential problems; we can learn more of them and have to warn
about those.")*

## The promise is a ratchet — everything we add is forever

The promise does not stop at the 1.0 baseline. **Every addition from contract 1 onward joins
the frozen contract and gets the same treatment**: a feature shipped at contract 2 is, from
that moment, something a contract-2 program may rely on forever — loft will not take it away
or change what it does. The reliable surface only *grows*; it never shrinks or shifts.

This is the promise stated as a **benefit**, not only a constraint: a program or library
author can build on *any* capability loft has ever shipped and know it will still be there,
behaving identically, for the life of their program. That is the AS/400 gift — you never
have to chase the platform.

It also sets the standing discipline for evolution: **because every addition is permanent, we
add carefully.** The pre-freeze audit below is not a one-time gate before 1.0 — its rigor
applies to *each* new capability epoch, because each is a forever-commitment the moment it
ships. loft accretes slowly and deliberately; a feature added in haste is a wart carried for
life. Rarely and well beats often and regretted.

## The classification — additive, or a regression

Under the promise there are only two kinds of change to a covered surface:

| | What it is | Status |
|---|---|---|
| **Additive** | every program, error, and store valid before is valid *and behaves identically* after; something new is available | **allowed** — ship it |
| **Regression** | a functioning program deviates: a different result, a store that reads wrong, an error that no longer fires (or now fires), a removed/renamed symbol | **a bug** — fixed before other work |

The old draft's middle category — a "managed breaking change" with a deprecation window —
is **gone for covered surfaces.** A *loud* regression (a removed function → `Unknown
function`) is still a regression: the program that called it stops working. Loudness makes
it *safe to detect*, not *acceptable to ship*. The only thing loudness buys is that our
CI and the developer catch it immediately — which is why the detectors below matter.

## The error surface is one-directional — you can drop an error, never add one

Errors have an asymmetry the other surfaces do not, and it flips the pre-freeze disposition:

- **Dropping an error is safe** (loosening). A program that used to be *rejected* now compiles
  or runs — no program that *functioned* is broken, because the rejected one never functioned.
  So loft may always become *more* permissive.
- **Introducing an error is a break** (tightening). A program that compiled and ran now fails —
  that is exactly a functioning program breaking. So after the freeze loft may **never** become
  *less* permissive on a covered surface.

Therefore the frozen error set is the **maximum** loft will ever have; it can only shrink. That
**inverts the disposition for errors**: everywhere else the pre-freeze mandate is "improve, then
you can't"; for errors it is **"be strict now, because you can always relax later but never
tighten."** The audit question for the error surface is precisely *"do we need **more** errors?"*
— every place loft is *too permissive* (silently accepts something dubious, or produces a
plausible-wrong value where it should reject or fault) is a **last-chance-to-add**: add the error
while contract 0 allows, convert the programs it catches, and the freeze then locks in a strict
floor we can only ever loosen. (Caveat: a *runtime* fault a program can **catch** is observable
both ways — dropping one can change a program that handled it — so the clean "drop is always
safe" rule is sharpest for **compile-time** errors; runtime faults still follow the general
"functioning program unchanged" test.)

This is why the silent-wrong findings from the lib and formal audits (`text as integer` → null,
integer overflow → null, a classifier true on `""`) are really **missing-error** findings: the
fix is usually *to add a diagnostic or a fault*, and adding it is the one-way door.

**But the *first* resolution of a would-be-error is a rewrite to correct function, not an
error.** Because contract 1 lets loft only ever *drop* errors, a situation that would "normally"
(in another language) fault is handled — wherever it sensibly can be — by **defining a
correct-functioning semantic**: the C80 spreadsheet model, where a divide-by-zero or an
out-of-range read yields the reserved null and the program keeps running instead of halting.
Erroring is the *narrower* choice, reserved for what cannot be given a sane defined behavior (and,
being a compile-time add, is the safe one-way door above). **This is why the pre-freeze checking
is so intensive right now:** every would-be-error situation must be decided *now* — rewritten to
function correctly, or (last chance) turned into an error — so that once frozen the contract holds
and nothing a program does falls into an undefined-or-newly-erroring gap. *(Owner decision
2026-07-14: "if we encounter more situations that would normally be errors we do rewrites so it
functions correctly as a contract — that is why we do so much checking right now.")*

## Deprecation is soft steering, never warn-then-remove

A deprecation under this promise **steers** toward a better idiom; it never announces a
coming break. The old idiom keeps working **forever**. So a deprecation warning says *"there
is now a nicer way"* — free to ignore ([GOALS.md](GOALS.md) Goal F: warnings are the only
channel that may bill the programmer, and even this one bills nothing you must act on). It
never means *"this will be removed."* That is arc C's channel, and arc C inherits this
constraint: it warns, it does not threaten.

## Folding — how we engineer around never-remove

"Never break" **plus no usage telemetry** makes the callable surface a **one-way ratchet**: you
can add and steer, but you can never prove zero holdouts, so you can never remove. Deprecation-
toward-removal is therefore *structurally impossible* here — a "deprecation" never reaches a
removal, so it is really **recommended-idiom signposting**, not a path to a break. (The word
"deprecation" survives only for familiarity; it never means here what it means elsewhere.) You can
entice users onto a nicer method, but you will never *know* the last holdout migrated — so the old
method stays forever. The way out is not removal; it is **folding** — we *engineer around* the
constraint. The cost splits across two axes, and only one of them has to grow:

- **The interface** (the callable name / idiom) grows forever — genuinely. The lever is never
  removal; it is **discipline at the door**: because every addition is permanent, add slowly and
  rarely. The ratchet pays the cost *upfront* (add carefully) instead of downstream (deprecate
  later) — this is the operative meaning of "we add carefully."
- **The implementation** does **not** have to grow. *"Never drop the old one"* means never drop the
  callable **name**, not carry its independent **code** forever. When a nicer primitive arrives,
  **fold** the old idiom onto it: reimplement the old name as a thin shim over the new primitive and
  delete the old implementation. The permanent *surface* stops meaning permanent *duplicate code*.

So the collectible win of a steer is the **fold** — fewer independent implementations — never
surface shrinkage. Every recommended-idiom steer should ship *with* its fold, or it buys nothing but
more surface to maintain.

**The mechanism — `#superseded` (@PLN102 arc C).** loft ships this as a callable attribute.
`#superseded "Y"` on a symbol X (a fn / method / operator) declares X superseded by the successor
symbol `Y` (a **bare name**, e.g. `#superseded "write_through"`). Three things follow, all built:

- **The steer.** A call to X *from owned source* — the entry project being compiled, never a
  re-parsed dependency or the stdlib — emits `advice: `X` is superseded — use `Y` (the old form
  keeps working)`. It is `advice`, not `warning`, and the tier rule says why: ignoring it cannot
  produce a wrong result, because the old form goes on behaving identically
  ([DIAGNOSTICS.md](DIAGNOSTICS.md) — a diagnostic gates CI iff ignoring it can be wrong).
  This caller-provenance gate means the steer reaches only whoever can act on it:
  the author of the *calling* code, never a consumer nagged about a dependency's internal idiom.
  Default-on; **`LOFT_NO_STEER`** opts out. The old form keeps working, identically, forever — a
  signpost, never a removal. The capability's catalogue entry is `@F109`.
- **The fold, enforced.** A lint (`superseded_fold_diagnostics`, run on every compile and in
  `make ci`) checks each `#superseded "Y"` symbol in owned source: `Y` must resolve — an
  unresolvable successor is a **hard compile error**, so a *dangling* steer never ships — and X's
  body must *call* `Y` (X is a thin shim over the successor, not independent code; an un-folded steer
  is an advisory warning). A steer therefore cannot ship without its fold.
- **Inert until used.** The channel does nothing until a symbol is actually marked, so adding it
  changed no existing program (the whole suite is byte-identical).

A *semantic* replacement (the old behaviour is **not** expressible over the new — see *Folding's
limit* below) is **not** a `#superseded` steer; it is **contract-keyed** (the escape valve below),
which reuses this same owned-source gate but keys the author-alert to a `contract` bump instead of a
bare `#superseded`.

**Folding's limit.** It works only when the new primitive is a genuine **superset** — same
observable semantics, nicer form. A semantically-*different* replacement cannot be folded (the old
behavior is not expressible over the new), so there you carry both — which is exactly why a semantic
replacement must be rare and **contract-keyed** (the escape valve below), never casual.

**The one visibility we do have: the registry.** We can statically scan every published library and
know precisely which *public* libs still use an old idiom. That cannot authorize removal (private
programs are invisible, so zero is unprovable), but it maps the public ecosystem's migration state —
enough to prioritize *which* folds are worth doing and to target the author-facing steer.

## The escape valve for the genuinely unavoidable — key it on the contract

Some change is occasionally forced (a security fix; an API whose old behavior is actively
harmful). The promise still holds, by the AS/400 method: **do not break old programs — keep
their old behavior, keyed on the `contract` they declared.** A program built for contract 1
runs with contract-1 behavior *even on a later loft that changed it* — loft carries both,
edition-style. The new behavior is what a program gets when it declares the newer contract.
This is deliberate, rare, and documented; it is the *only* way an observable behavior ever
changes, and even then no existing program sees the change. A regression that slips through
*unintentionally* is not this — it is a bug, fixed stop-the-world.

## Two populations, one promise — fold ours, host theirs

The promise covers two populations the owner relates to differently, because **he cannot compel or
limit others** — only guarantee what he himself controls:

- **loft's own surface (language + stdlib)** — the owner owns the implementation, so never-break
  here is additive + **folding** (above): keep the name, fold the code onto the new primitive.
- **Everyone else's libraries** — the owner owns *neither* the implementation *nor* the authors. He
  cannot make an author migrate, maintain, or republish, and he does not try. His lever is the one
  thing he *does* control — the **registry** — and it is enough: **host every published version
  forever** (append-only; a version is never deleted), and let the **runtime** run each artifact
  under the `contract` it declared (the escape valve above). An abandoned contract-1 library stays
  downloadable and runs correctly on any later loft, with *zero* action from its author — and a new
  program may still depend on it.

So never-break rests on three legs, each a thing the owner actually controls — not a request made of
anyone else:

1. **loft's surface** — additive + fold (owner owns the code).
2. **The registry** — permanent hosting (owner owns the host): every published artifact stays
   available forever. *This* is the registry's primary never-break role; the migration-scan
   (§ Folding) is secondary.
3. **The runtime** — contract-keying: loft honors each artifact's declared contract forever.

Together they carry any working program and its dependencies: the deps stay downloadable (2), loft
still runs them correctly (3), and loft's own surface dropped nothing (1). Availability is hosting,
correctness is contract-keying — the owner compels no one and still breaks no one. (Hosting covers
the *published* ecosystem; a private artifact its user vendored is held by that user and is carried
by leg 3 alone.)

**Leg 2 is active work, not passive storage.** "Host forever" is *enforced* by an automated
compatibility gate at submission (arc B-registry, `pr-validate`) — this is the real cost of the
promise on our side. Each new library version is checked against the last accepted one on two axes:

- **API verification** — the public surface must be an additive **superset**; a removed or changed
  public symbol / signature is a contract break.
- **Old-tests regression** — the previous version's own test suite must still pass against the new
  implementation; a failure is a break. (As strong as the library's tests + declared surface — the
  gate cannot see behaviour the author never pinned.)

On a clean submission the new version supersedes as a compatible successor. **On a detected break the
old version is preserved *first* — snapshot its API/test baseline, keep it hosted — before the new
version is accepted as a distinct, opt-in breaking version.** So a consumer pinned to the old version
always keeps a working copy, and the break becomes a *choice*, never a surprise. This holds library
authors to the *same* never-break discipline loft holds for itself: the gate catches the break
automatically, instead of asking every author to be careful.

**And the gate must *own* its baseline, not borrow the author's.** Even the two checks above are not
enough, because the author's tests are the *floor*, not the reference: they may be weak, incomplete,
or weakened over time, so a gate that only re-runs what the author shipped is only as honest as the
author's diligence. So at submission the registry does two more things:

- **Verify, don't trust** — run the library itself in a sandbox, on every target, and record the
  result. A claimed pass is not a pass; the registry produces the pass.
- **Store more than the author made** — capture and keep the registry's OWN compatibility baseline:
  a behavioural / characterization snapshot + the pinned API surface, **and the built cross-target
  artifacts** (interpret / native / wasm / html). Future never-break checks reference *that*, not the
  author's current tests. This is durable two ways: against a later-weakened test suite (the baseline
  is ours, frozen at acceptance), and against **toolchain drift** — an old library stays runnable
  because its *built* forms are preserved, not only its source, so a future loft that can no longer
  rebuild it from source can still serve what was verified working.

This is the deeper cost of the promise: verification infrastructure (a sandboxed, all-target runner)
plus storage *richer than the submission* — the registry keeps more than any author ever made.

**So admission is a validation *pass*, not a fully automatic gate.** The automation does the
mechanical work — the API diff, the sandboxed all-target run, the baseline capture — but it only
verifies what is mechanically checkable. Whether the captured baseline *adequately pins* the
library's contract (a *passing* test suite is not a *sufficient* one), whether the code is
trustworthy to serve forever, and whether the perpetual host-verify-serve commitment is worth taking
on at all — these are judgments. So the registry is **curated**: automation prepares the evidence, a
human makes the acceptance decision. Admitting a library into the never-break guarantee is a
deliberate act — because acceptance, like every addition under this promise, is permanent (the same
ratchet as loft's own surface).

**Our own published libraries are the hybrid — and there the mechanism is a *signal*.** loft's own
libs (`web`, `server`, `graphics`, …) live in the registry and pass the same curated gate as
anyone's, so they are *hosted* like a third party's — but we also own their code, so we can *fold*
them like our own surface. That gives us a lever third parties do not have: we can trigger the
versioning mechanism **deliberately** — mark a new version as a breaking epoch, key a new behaviour on
the `contract` — **not because we want to break** (we never do: we fold, host, and key, so no consumer
is harmed) **but as a signal.** It is arc C's recommended-idiom signpost delivered *through the
mechanism itself*: the new epoch *tells* the ecosystem "the preferred idiom moved here," while every
existing consumer keeps its working old version untouched. The same mechanism a third party is
*caught* by, we use *on purpose* — safe precisely because we control both the library and the
language, so we can guarantee the signal costs no one a break.

**This *is* loft's own deprecation mechanism** — the honest one under this promise, not the
warn-then-remove kind (there is none). To deprecate an idiom in our own surface or libraries is to
**signal** the successor (the versioning epoch / contract-keyed behaviour / recommended version) and
**fold** the old onto the new, while the old keeps working forever. Signal + fold *is* the
deprecation: it steers without threatening and consolidates without removing. So "deprecated" here
means *"superseded, still supported, implemented over the new primitive"* — never *"going away."*

We are *allowed* to do this; we do not *want* to. Even a break-free signal spends a new epoch and
taxes every consumer's attention, so the standing preference is **pure-additive — no signal at all**,
and this mechanism is held in reserve: reached for reluctantly, rarely, and only when a successor
genuinely earns the steer. Having the lever is not a reason to pull it.

## What it means for the programmer — old works; migration is yours, and informed

Under all of this the contract never moves *under a program*: it keeps working, unchanged, on every
later loft — that is the whole promise, and it holds with the programmer doing nothing. The only cost
of *staying* is **opportunity**: an old version may lack **new features** — and, in the rare
unavoidable case, a **security fix**. We try hard to keep even security fixes *additive* and backport
them into the old epoch, so staying safe never *requires* migrating; a *breaking* security fix is the
one place that can fail, and we treat it as the emergency it is, not a routine break.

Nothing *pushes* the programmer forward — **migration is their choice and their work**, taken when a
newer version is worth it to them. And we make that choice *informed*: the same gate that enforces
never-break already knows whether a newer version is an additive **superset** (a drop-in upgrade) or a
breaking **epoch** (needs code changes), so the registry can **indicate the API-compatibility of the
target** before they move. The programmer never has to guess whether an upgrade is safe — we have
already computed it.

This is **new status the registry does not carry today.** Right now a new version indicates only
*that* the library changed (a bump); it says nothing about *whether* the change is API-backwards-
compatible. The design adds a per-version-transition **verdict** — additive superset (drop-in) vs
breaking epoch — that the gate computes and the registry **records and surfaces**. That is a real
addition to the registry's data model, part of the arc-B-registry cost, not a reframing of metadata
we already have; and it is what turns "migration is yours" into "migration is yours *and informed*."
The field is **computed automatically by the gate on import** — derived from the API diff at publish,
never author-declared — so it stays honest (the registry produces the verdict; an author cannot
*claim* compatibility) and always current (set on every version imported, no manual upkeep).

## The `contract` integer under this promise

Because forward-compatibility is now **guaranteed**, the `contract` version is simpler than
a breaking-change tracker:

- It is a **monotone capability floor.** A program/library declares `contract >= N` — "I
  need loft's contract-N capabilities." Since loft never breaks forward, **loft at contract
  E ≥ N always runs it** (accept); **E < N** is a clean reject ("needs a newer loft").
- A **bare `contract = N` is `>= N`** (forward-open) — *not* "tested-at N, warn on newer."
  There is nothing to warn about on a newer loft, because a newer loft does not break it.
  *(This supersedes the original versioning-decision default; the mechanism's bare case was
  flipped `=`→`>=` accordingly.)*
- Where the escape valve above is ever used, the contract additionally **keys which behavior
  a program gets** — so the integer a program declares is both its capability floor and its
  behavior epoch.
- **Contract 0 is pre-1.0 — the only era with no promise.** Until the freeze, every surface
  may move; this whole document takes effect at the `0 → 1` flip.

## Before the flip: the pre-freeze audit (the one-way door)

The `0 → 1` flip is **irreversible** — every wart it freezes is frozen forever. So the flip
is **gated on a thorough, surface-by-surface examination** of everything the promise will
cover, done while contract 0 still lets us change it. Anything we would not want to live
with permanently is fixed *before* the flip, not grandfathered into the contract.

**The stakes: a miss is permanent.** If the audit overlooks a wart and it ships in contract
1, we do **not** break it later to fix it — that would break the very programs this promise
protects. We **live with it and engineer around it**: the wart stays, and a better idiom is
added *alongside* it (additive), the way every long-lived platform carries the mistakes it
froze and grows past them without removing them. That is the whole reason the audit must be
exhaustive — the cost of a miss is not a later fix, it is a permanent scar the language
carries forever. Post-freeze, "engineer around, never break" is the standing rule for any
wart we discover too late.

This audit is the critical-path work of the 1.0 line (arc E), not a formality:

- **Syntax** — every construct reviewed for a shape we would regret; the last in-flight
  syntax changes settled and reviewed as part of this.
- **Semantics** — every observable rule (evaluation order, coercion/narrowing, null/`??`,
  ownership/free timing) examined for a wart we would be stuck with.
- **Errors** — which errors fire, their identity, and any content a program can observe —
  reviewed *before* they become frozen (this surface is new to the audit as of the owner's
  scope note).
- **Stdlib** — every public name, signature, default, and behavior — naming consistency,
  missing coverage, wrong defaults — because a bad name or default is permanent after the
  freeze.
- **Store/heap layout, wire format, package format** — every format reviewed for anything we
  would not want to keep readable forever.

**The audit's ledger already exists in part:** [INCONSISTENCIES.md](INCONSISTENCIES.md)
catalogues known asymmetries and tensions by severity — every entry is a candidate
"fix-or-consciously-accept before the freeze," and a **High** (silent-wrong) entry is a
must-fix. The audit resolves that ledger, sweeps each surface for what it misses, and only
then is the flip earned. Consciously-accepted items are recorded as deliberate (a
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) entry), so the freeze is a decision, not an
oversight.

### The road to contract 1 is two phases — language, then libs

The audit is not one rushed pass; reaching the freeze is **dual-phase**, and the order is
forced by the work:

1. **Close the open plans → settle the language.** You cannot audit a surface still in
   motion, so the language side (syntax, semantics, errors) is settled *first* by landing the
   in-flight feature/language plans. This phase is underway. As each plan closes its surface
   stops moving and becomes auditable.
2. **Then a dedicated, unhurried pass on the lib side.** The standard library and the core
   published libraries are an *equally permanent* surface — every public name, signature,
   default, and behavior is frozen forever (§ The promise is a ratchet). They get their **own
   focused phase**, not a rushed tail on the language work: naming consistency, coverage gaps,
   wrong defaults, the method-vs-free-function asymmetries in
   [INCONSISTENCIES.md](INCONSISTENCIES.md) — all decided while contract 0 still permits it.

   **The freeze scope is everything published — no core-subset carve-out** (owner, 2026-07-20):
   the whole distribution is frozen at contract 1, not a hand-picked core with the rest left
   experimental. **The validation gate that earns that freeze is games.** We build real games that
   work *on* the libraries first; the games dogfood every surface, expose the wrong defaults and
   coverage gaps while contract 0 still lets us fix them, and only once the games run well on a
   library is that library proven ready to freeze forever. Games-first ([GOALS.md](GOALS.md)) is
   therefore the lib-side readiness bar, not a parallel track — a library nobody built a game
   against is not yet proven, and an unproven surface is not frozen.

Only when both phases are done — the language settled and audited, the lib side deliberately
gone over — is the `0 → 1` flip earned. This is *why* the freeze is not imminent despite the
type surface being feature-complete: the language nearing done is phase 1; the libs are phase
2, and they are given their time.

**One thing IS measurable before the freeze, and it is not resilience — it is whether the
standard has stopped MOVING.** The `contract:` axis ([.github/LABELS.md](../../.github/LABELS.md))
records, for every bug we state we have fixed, whether closing it needed the written standard to
change: `contract:settled` = the formal rules and the tests already gave the right answer and the
fix makes that promise hold; `contract:strained` = it took a rule EXTENDED, a documented surface
changed, or a design call. `make bug-review` § 5 reports the monthly ratio.

That is the convergence evidence the freeze decision has been missing. It is one of the freeze
gates below, which come in two kinds, interleaved rather than grouped. Three ask whether what we
FOUND is settled — `silent-wrong`, `contract:strained`, and open deviations. Two ask how much we
LOOKED — rule coverage, and the light pass. A gate of the first kind can be met by a quiet month;
only the second kind can say the quiet was earned:

- **`silent-wrong` → 0** is the per-bug blocker — no known wrong answer may be frozen into the
  contract. Ask it rather than reading a status here:
  `gh issue list -R loft-lang/loft --state open --label silent-wrong`. ⚠ This line read *"true
  on any given day, and true today"* while two were open, which is the same rot as a committed
  position figure: a status asserted in prose is stale the moment it is written, and it reads
  as measured rather than as stale. ⚠ And a `fixed-pending-merge` entry is a fix on a BRANCH,
  not on `main` — whether the gate counts those as met is a decision rather than a
  measurement, and the freeze binds what main ships.
- **`contract:strained` → 0, SUSTAINED over a window long enough to be evidence**, is the
  convergence gate — the standard has stopped moving. Only this one can say the blockers are
  truly gone rather than currently absent.
- **Rule coverage at or above its contract-1 floors** — `make rule-coverage`, currently
  **70 % of `@FR-` rules carrying a code annotation and 40 % an active guard** — is the
  *coverage* gate: enough of the written standard has been checked against the implementation
  that the unwalked surface has been looked at deliberately rather than left to chance — a
  LOOKED gate, where the three found-gates can all be met without anyone having looked. ⚠ These
  are **minimum thresholds, not targets** — informed owner estimates (2026-09-17) of the least
  that could earn the freeze, expected to move, and the walk continues past them. Crossing them
  is necessary and never sufficient, for the same reason the checklist minimum is: it measures
  the rules we have written, and a rule nobody has written yet is not counted by anything.
  ⚠ **The distance overstates the WORK.** Many tail rules are simple enough that the existing
  tests already validate them fully — so no new exception is waiting to be found, and closing
  the gap is an annotation at a site that already holds, not verification to do. Evidence,
  2026-09-17: five rules drawn from the untouched tail (`F-Escape`, `E-And`, `F-Args`, `M-Bool`,
  `Col-Copy`, each with neither an annotation nor a guard) were probed on the interpreter and
  every one behaved exactly as written. Read a shortfall as labelling debt until a probe says
  otherwise. ⚠ Five rules took SIX probes: `Col-Copy`'s first tested a `vector` where the rule
  is stated for KEYED collections, and passed — a probe landing beside a rule rather than on it
  reads exactly like a verification, which is the light pass's dominant hazard.
- **Open deviations as few as they can be made** — `python3 scripts/rule_tags.py registers`
  counts them; no figure is written here. A deviation is a *written rule the code does not
  obey*, so freezing on top of one promises the rule and ships the exception. "As few as
  possible" rather than zero on purpose: some will be closed by an owner ruling that the rule
  was wrong, and a few may be knowingly carried with their reason recorded — what must not
  happen is carrying one nobody decided about.
- **A light detection pass over every rule** — ⚠ **this instrument does not exist yet**, and
  naming it here is the point. It is deliberately *lighter* than
  [STABILITY_METHOD.md § The rule-led walk](STABILITY_METHOD.md): the walk splits one rule into
  the questions its sites ask, finds each question's one home and verifies the related cases —
  deep work per rule, and it took the most-changed and most-error-prone rules FIRST. That
  ordering is why the remainder is the low-yield tail, and why it wants a different
  instrument: **read each remaining rule once and ask only whether it looks semantically
  wrong or unenforced**, producing suspicions to triage rather
  than citations to land. The walk is how a suspicion gets resolved; this pass is how the tail
  gets looked at ALL, before a freeze makes every unlooked-at path a permanent promise.
  ⚠ **It runs immediately before the contract-1 decision, not continuously** (owner,
  2026-09-17). Run early it reads a surface still moving and has to be run again; its whole
  value is being the LAST look before the one-way door. So it is the final gate to clear, not
  a task to start now — and nothing else on this list waits for it.

⚠ **A bug count cannot substitute for it, and reading one as the other is the specific mistake
this axis exists to prevent.** `silent-wrong` ran at 33 % of everything filed in August while
every fix examined was `contract:settled` — the audits were productive and the standard held.
Those are opposite readings of the same number. **Owner's call (2026-08-28): track this
statistic through the next month rather than declaring on a few days of it** — the mechanism is
working, it has not yet had time to settle. Expect the first weeks to be mostly UNJUDGED, which
the report prints beside the ratio and never folds into either side; that column is what to watch
first.

**Clear the blockers first; measurement comes after the freeze.** The path to contract 1 is a bounded
job — get rid of *everything* that blocks the freeze (the open plans, the language + lib audits) — and
**new feature development is held off until those blockers are worked through**. This pre-freeze work is
*convergence to* the standard, not measurement *of* it: with the surface still moving there is nothing
fixed to measure against, so **until the freeze we measure nothing**. Only *after* contract 1 does real
development on the now-frozen standard measure its resilience — and any gap it surfaces is handled the
additive way (fold / host / contract-key — the escape valve above), never by breaking. So the checklist
minimum (audits green, plans closed, suite passing) is *necessary, not sufficient*: it gets us to the
door, and the freeze is declared only when the blockers are truly gone and the converted consumers are
stable again — deliberately weeks away, never on the strength of a passing board alone.

### What a falling bug rate does and does not license

The dogfood consumers are building and hitting far fewer bugs than they used to. That is real,
it is the point of Goal C, and **it is not evidence that the freeze is earned** — because it is a
measurement of the paths *those consumers walk*. Games-first is the right lib-side readiness bar
and it has the same shape: it proves the surface a game exercises, and a game exercises what its
author needed.

The freeze does not bind only the walked paths. **At contract 1 every behaviour that works
becomes a permanent promise**, including behaviour on paths nobody has taken yet — the same
feature at another type, in another position, under a `?`, on the other backend. An outlier is
not a rare shape; it is an *unwalked* one, and the next project's author walks a different set
than ours ([GOALS.md § Useful is not stable](GOALS.md)).

That asymmetry is what makes the unwalked surface the expensive part:

- A defect on a walked path is found by somebody running a program, and while we are at
  contract 0 it is simply fixed.
- A defect on an unwalked path is **frozen on the day the contract is declared**. After that it
  can only be handled additively — fold, host, or contract-key — never fixed in place, because
  fixing it is exactly the breakage the promise forbids. A `silent-wrong` is the sharpest case
  and CLAUDE.md's filing policy already calls it *the FREEZE axis*: a `sev:low` edge that
  answers quietly wrong cannot be frozen into the contract, while a `sev:high` crash can,
  because a crash tells you.

So the readiness question is not *"are our programs stable?"* — they are — but **"would the next
program be?"**, and no consumer metric can answer it. A falling bug rate says our reach has been
hardened; it is silent about everything outside our reach, which is the majority of what the
contract will bind.

**The honest bound, stated so nobody plans against a fantasy.** The matrix of
feature × type × position × nullability × backend is not enumerable, and this will never be a
proof. What is achievable is to get materially closer to a language that does not let the next
user down — and the instrument for that is the one thing we have that is stated over a *whole
domain* rather than over the programs we happen to own: the formal rules. A corpus can only
cover what somebody already wrote; a rule covers every case it quantifies over, so walking one
reaches cells no program in the tree does. That is the practice in
[STABILITY_METHOD.md § The rule-led walk](STABILITY_METHOD.md), and the position marker to read
against this section is a command rather than a figure: **`make rule-coverage`**, whose contract-1
floors are **70 % of rules carrying a code annotation and 40 % an active guard** — minimums for
the freeze, not targets — and which says how many rules short of each the tree is today.

⚠ **No position figure is quoted here, and that is the point.** This sentence carried
**179 of 255** for a year — overstating the uncovered share at nearly double — and read as a
measured position rather than as stale, in the one section where being pessimistic about
readiness costs the most. ⚠ And read the command's answer as work LEFT, never readiness: the
walk takes the most-changed and most-error-prone rules first, so what remains is the low-yield
tail, and a bare fraction cannot express that in either direction.

This does not add a checklist item to the two phases above; it says what the phases are *for*.
The checklist minimum stays necessary-not-sufficient, and the judgement it is not sufficient for
is precisely this one — whether the surface we are about to promise forever behaves on the paths
we have not walked.

## Per-surface — additive is the path; here is what a regression looks like

| Surface | Additive (allowed) | Regression (a bug) |
|---|---|---|
| **Language syntax** | a new construct that leaves existing parses unchanged | old source no longer parses; a new reserved word colliding with an identifier; a precedence/associativity change that re-groups valid code |
| **Language semantics** | a new capability with no effect on existing programs | same source, different runtime result (the C86 class); changed evaluation order, overflow, null, coercion a program observes |
| **Errors** | a *new* error for previously-*undefined*/UB input; better human-only prose no program observes | an error that used to fire no longer does (or vice-versa); a changed error identity/type a program catches; changed observable content |
| **Stdlib API** | a new function/method/type; a new optional parameter | removing/renaming a public function; a signature change that breaks existing calls; a *different result* for the same inputs |
| **Published libs** | a new library; a new library version | a registry/resolution change that stops an installed library loading; see the note below on library authors' own obligation |
| **Store/heap layout** | a versioned layout that still reads old stores | a layout change that makes an older store read wrong (@PLN97 hash changes) |
| **On-disk + wire** | new optional fields with defaults; a new message type old readers ignore | old data/messages read to different values |
| **Package format** | a new optional `loft.toml` field (e.g. `[package] contract`, added additively) | removing/renaming a required field; a layout change that stops old packages resolving |

**Published libraries have two obligations, not one.** loft promises the *language +
stdlib + runtime* a library is built on will not break it. A **library author** owes the
same promise to *their* consumers — and declares the `contract` they target so loft can
honor it. The registry-validation gate is where a library that broke its own consumers (or
that loft would have broken, before this promise) is caught.

## Bug fixes — the one careful line

A fix whose old behavior was a **crash, a fault, or undefined behavior** is always allowed:
a program that crashes was not *functioning*, so nothing that functions relies on it.

A fix that changes the **observable result of a functioning program** is a **regression**,
even when the old result was "wrong" — because the program functioned, and the promise is to
that program, not to our sense of correctness. The reconciliation is the escape valve: if
the behavior genuinely must change, key the new behavior on a newer contract and keep the old
one for old programs. "We fixed a bug" is never a licence to change what a working program
does.

## Making a regression impossible to ship silently

The promise cannot rest on remembering to check. Each surface has a detector that turns a
regression into a **CI failure** — because a *silent* regression (right-looking wrong answer)
is the one a human review misses:

| Surface | Detector |
|---|---|
| Store / heap layout | @PLN97 layout-identity hash (a changed hash with no keyed migration → fail) |
| On-disk + wire | format-version + round-trip golden tests |
| Language + stdlib + **errors** semantics | a **golden-behavior corpus** — pinned program → (output **and** diagnostics); any drift between releases is a regression to fix or an escape-valve keying to justify |
| interp ↔ native | the @PLN89 differential oracle |
| published libs | `registry-validation` — every published library re-run against loft@main |

The layout hash, the oracle, and registry-validation exist; the golden-behavior corpus (now
explicitly including **diagnostics**, per the errors surface) is the open CI work, shared
with arcs C/E. **Named residual:** a regression in a shape no corpus covers can still slip —
the dogfood loop is the backstop that turns an unseen shape into a corpus cell, and when one
does slip it is a bug fixed before other work, per the promise.

## How this fits @PLN102

- **The `contract` version** (implemented) is the capability floor + behavior key this
  policy drives.
- **Arc C — deprecation** is *soft steering only* here (never warn-then-remove), plus the
  keying mechanism for the escape valve. This policy sets that constraint; arc C builds the
  channel within it.
- **Arc E — the 1.0 line** decides *which* surface is frozen (what is in the promise vs
  marked experimental) at contract 1. This policy says what the promise *means* for whatever
  E includes.

## Open decisions (owner's call)

1. **The 1.0 freeze scope (arc E).** The promise is absolute for what is *in* it; E draws
   the line between frozen and still-experimental at contract 1.
2. **How far "observable errors" reaches.** Where a program can read raw error content vs
   only an error's identity/type — the tighter that surface, the more error *prose* stays
   improvable. Depends on loft's error-handling model; pin it as arc C designs the channel.
3. **The escape valve's ergonomics.** How a contract-keyed behavior split is written and
   tested (edition-style) — designed when first genuinely needed, not before. **In design
   (proactively):** [plans/113-contract-keyed-semantics/README.md](plans/113-contract-keyed-semantics/README.md)
   (@PLN113), the mechanic for the first *post*-contract-1 semantic change. The @PLN110
   `len/size(text)` flip is its **worked example, not a customer** — the flip lands *before* contract 1
   (a free break, #587), so it needs no keying.

## See also

- [plans/102-stability-contract/README.md](plans/102-stability-contract/README.md) — the
  plan; this is arc A.
- [plans/102-stability-contract/versioning-decision.md](plans/102-stability-contract/versioning-decision.md)
  — the `contract` axis (bare = `>=` under this promise).
- [plans/113-contract-keyed-semantics/README.md](plans/113-contract-keyed-semantics/README.md) —
  @PLN113, the edition-style escape valve that keys a semantic split on the declared `contract`.
- [GOALS.md](GOALS.md) — the AS/400 standard and Goal F.
- [DESIGN_DECISIONS.md § C86](DESIGN_DECISIONS.md) — the archetypal silent regression.
- [plans/97-layout-contract/](plans/97-layout-contract/) — the layout-identity hash.
- [RELEASE.md](RELEASE.md) — release cadence + calendar versioning (the release tag, kept
  distinct from `contract`).
