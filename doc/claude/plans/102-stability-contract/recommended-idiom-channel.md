<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 arc C — the recommended-idiom channel (steer + fold), safe small steps

> **Status: design (2026-07-16), buildable.** The full design for arc C — the "recommended-idiom
> signposting" channel and the folding discipline — with a safe, inert-first step ladder. Read
> [COMPATIBILITY.md § Deprecation is soft steering](../../COMPATIBILITY.md) and [§ Folding](../../COMPATIBILITY.md)
> for the *why*; the [README arc table](README.md) maps **C = C5**. This resolves the plan's
> thorniest open question (Q3: *whose* obligation is the warning). No code yet — the doc is the
> generative act (design-protocol § "write the doc before the code").

## What arc C is (and is not)

loft's promise is **absolute never-break + no usage telemetry**, which makes the callable surface a
**one-way ratchet**: you can *add* and *steer*, but you can never *prove* zero holdouts, so you can
never *remove*. So a "deprecation" here can never reach a removal — it is **recommended-idiom
signposting**, not a path to a break. Arc C is that channel. It has exactly two jobs:

1. **Steer** — a warning that says *"there is now a nicer way to write this"* (Goal F: the only
   channel allowed to bill the programmer, and even it bills nothing you must act on). The old idiom
   keeps working, identically, forever.
2. **Fold** — the collectible win of a steer is not surface shrinkage (impossible) but **fewer
   independent implementations**: reimplement the old name as a thin shim over the new primitive and
   delete the old code. *Every steer ships with its fold, or it buys nothing but more surface.*

Arc C is **not** the compat gate (that is C1, done, `loft api-surface`), **not** the registry
(B-registry / C2-C3), and **not** the contract-keyed behaviour split (C4). It is the smallest of the
mechanism arcs — one attribute, one gate, one lint — but it carries the plan's hardest *design*
question, Q3.

## The one invariant (design-protocol step 1)

> **A steer fires exactly when the compilation's OWNED source (the entry project) uses a
> `#superseded` idiom — never when the idiom-use is in imported-dependency or stdlib source; and
> every `#superseded("use Y")` symbol is implemented as a shim over Y.**

Two halves, each a single carried fact so it cannot be re-derived per site:

- **The fire-site half (Q3):** the gate reads ONE fact — *is the source currently being compiled the
  entry project, or a dependency?* — which loft already carries per source (`Definition.source`;
  `MAIN_SOURCE` = the entry, `STD_SOURCE` = stdlib, a `use`d library = its own resolved source, tagged
  a dependency by the package-path-prefix map). The steer fires iff the CALLER's source is owned.
- **The fold half (C5):** `#superseded("use Y")` on symbol X is a machine-checkable promise that X's
  body is a shim over Y. A `make ci` lint enforces it, so a steer cannot ship un-folded.

## Q3 — whose obligation, resolved (the crux)

**The question** (README open-question 3): @C86 changed the meaning of code libraries had *already
shipped*. A warning at **loft-build** time reaches the loft team (they already know); at
**library-compile** time reaches the author (who can act); at **consumer-compile** time reaches the
wrong person (a consumer cannot fix the library). *Where does it fire?*

**The answer is a provenance gate on the CALLER, not the callee.** A steer fires only when the source
*making the call* is the compilation's entry project (`owned`), never when it is a resolved
dependency or the stdlib. This reaches the right person **structurally**, in every case:

| Who is compiling | Whose source is `owned` (entry) | Steer on a superseded idiom fires? | Right person? |
|---|---|---|---|
| A user writing a program | their program | **yes** — on their own old-idiom use | ✅ they can act |
| A **library author** building/testing their lib | the library | **yes** — on the lib's old-idiom use | ✅ they can act |
| A **consumer** importing that library | their program (the lib is a dependency) | **no** on the lib's internal idiom; **yes** on the consumer's own code | ✅ never nagged for a lib they can't fix |
| loft's own `make ci` on the stdlib | the stdlib is `STD_SOURCE`, exempt | **no** (stdlib already warning-exempt) | ✅ n/a |

The load-bearing observation that makes this non-trivial: **a consumer DOES re-parse a library's
`.loft` source** (libraries ship as source for the interpreter + cross-target). Without the gate, a
library's internal use of an old idiom would warn *the consumer* on every build — the exact
"wrong-person" failure. The gate suppresses it because the library's source is not `MAIN_SOURCE` in
the consumer's compilation. This is the same shape as the existing `self.default` stdlib exemption,
generalised from two-way (stdlib / not) to the provenance already carried per source (entry /
dependency / stdlib).

**Delivery (the README's second half of C's open status).** The steer's primary channel is the
**dev-time compile warning** — the author sees it on `loft run` / `loft check` / `loft test` of their
own project or library. That is sufficient and correctly-targeted by the gate above. A **secondary**
channel — the registry migration-scan (arc B-registry) — statically scans every *published* library
for superseded-idiom use, giving the OWNER the ecosystem map ("which public libs still use old idiom
X") to **prioritise which folds are worth doing** and to target an author-facing nudge. Two channels,
distinct audiences: dev-compile → the code's author (act now); registry scan → the loft owner
(prioritise). Neither ever reaches a consumer about a dependency's internals.

**The contract-keyed variant (@C86's actual class) is C4, not the MVP.** @C86 was a *semantic* change
(plain-bind now copies), which **cannot be folded** (the old behaviour is not expressible over the
new — folding's limit, below). A semantic change is handled by the **contract-keyed escape valve**
(C4 / leg 3): the old behaviour is preserved under the declared `contract`, and the author is steered
to the new idiom *when they bump contract*. That author-alert reuses arc C's SAME provenance gate and
emission, but its trigger is a contract bump, not a bare `#superseded`. So the MVP builds the
**superset idiom steer**; the contract-keyed author-alert is a thin C4-triggered reuse of it,
cross-referenced, built when the first genuine keyed change lands (same "when genuinely needed" rule
as C4's behaviour split).

## The re-assertion count (design-protocol step 2) — N = 1 everywhere

- **Steer emission:** ONE chokepoint — call resolution (`call_nr` / `call`, `src/parser/mod.rs`),
  the same single point every fn/method/operator call routes through (the site the call-arg N-Store
  check already lives at). It reads the resolved def's `superseded` field + the caller's provenance.
  No per-call-form spray.
- **Provenance:** ONE carried fact (`self.data.source` vs `MAIN_SOURCE` / dependency), not
  re-derived. Omitting it cannot silently mis-target — the gate is one predicate.
- **The fold check:** ONE `make ci` pass over the parsed `Data`.

So `N × silence` = 0: there is one emission site, one gate, one lint. (The one honest widening: a
superseded *type/field* use — not a call — would need a second emission point at type-resolution.
The MVP scopes to fn/method/operator steers, the common "use method Y instead" case; type steers are
a later increment reading the *same* provenance fact — see step 7.)

## Falsification — how it breaks (design-protocol steps 3–4)

- **Claim: "the gate reaches the right person in every case."** Probed in the Q3 table above; the
  load-bearing sub-claim ("a consumer re-parses library source, so the gate is required") is
  VERIFIED against the tree (libraries distribute `.loft` source, parsed at load). Falsification
  target for the build: a fixture where a *dependency's* source uses a `#superseded` idiom must emit
  **zero** steers when compiled by a consumer, and the SAME source must emit the steer when compiled
  as its own entry (the author case). Both are positive controls in step 3.
- **Claim: "every superseded symbol can be folded."** FALSE in general — a semantically-*different*
  replacement cannot be folded (folding's limit). So `#superseded` carries a contract: **the
  successor Y is a behavioural superset of X.** The fold lint enforces that X is *implemented via* Y
  (a structural check), but it CANNOT prove semantic equivalence — the author asserts the superset.
  This bounds the attribute's honest use: a semantic change is contract-keyed (C4), never a bare
  `#superseded`. The lint catches the *mechanical* failure (a steer with no fold); the superset claim
  is the author's, documented at the marker.
- **Claim: "a new steer is never a breaking change" (why the channel survives the freeze).** A
  `#superseded` marking adds a *warning* to code that used to compile clean. Per
  [COMPATIBILITY.md § Warnings are not a covered surface](../../COMPATIBILITY.md) (owner ruling
  2026-07-14: warnings are never a contract breakage), a new warning breaks nothing — so the steer
  channel is the one path that stays open *past* 1.0 (Goal F). This is load-bearing: it is *why* arc
  C can keep operating after the freeze while every other surface is frozen. Falsification target: a
  build that treats warnings as errors is opting into churn the contract does not owe — so the steer
  must be a `Level::Warning`, never an error, and must be silenceable per the standard warning
  toggles.
- **Over-unification guard (step 4) — the cleanest claim: "arc C is one attribute + one gate + one
  lint."** Attacked: (a) type/field steers need a second emission point → scoped OUT of the MVP
  (step 7), not absorbed falsely. (b) The contract-keyed author-alert (@C86's class) *looks* like the
  same channel but has a different trigger (a contract bump, not a bare marker) and cannot fold →
  routed to C4, not absorbed. (c) A steer whose successor is in a *different* library (cross-package
  fold) — the fold lint must resolve Y across the loaded `Data`, and if Y is unresolvable the marker
  is rejected at parse (a dangling steer is a build error on the AUTHOR's side, never a consumer's).
  Each near-absorption is named and either scoped out or handled, not waved in.

## The safe small steps (the commit ladder)

Inert-first, each step complete + verifiable before the next (PLANNING.md § goal 5). The channel is
**inert until the first symbol is marked `#superseded`**, so most steps are byte-identical for every
existing program — the risk is concentrated in one place (the provenance gate, step 3) and the proof
in the dogfood (step 6).

| # | Step | What lands | Verify (positive control) | E |
|---|---|---|---|---|
| 1 | ✅ **LANDED (2026-07-16). The `#superseded` attribute — PARSE + STORE, inert.** `Definition.superseded: String` (**empty = not superseded**, matching the `cap`/`native` empty-string-sentinel convention — chosen over `Option<String>` to match the sibling def-level fields + keep the string round-trip simple). **Payload is the BARE successor symbol** (`#superseded "write_through"`, NOT a free-text phrase like `"use write_through"`) — locked in so step 4's lint resolves Y directly with no phrase-parsing, and the future warning renders "…use `write_through`…". Parsed in `definitions.rs` beside `#null_safe` (author-available, not stdlib-only; a missing string is a parse error). Full round-trip mirroring @PLN46's `null_safe`: `DEF_SUPERSEDED` store slot (text ref at offset 152; the three trailing bools shift +4, stride 155→159) + JSON (`ir_schema`) + the `ir.loft` mirror + `ir_schema_gen`. **Nothing reads the field** → byte-identical bytecode + native Rust (proven: HEAD~1 vs HEAD introspect diff empty). | ✅ `superseded_survives_store_round_trip` (parse → store → JSON); layout asserts updated; a marked fn round-trips, an unmarked one is `""`; introspect byte-identical on both backends | S |
| 2 | ✅ **LANDED (2026-07-16). The provenance predicate.** Two methods on `Data`: `source_is_owned(source) = (source == MAIN_SOURCE)` (the general fact — entry vs a `2..` dependency vs `STD_SOURCE`) and `caller_source_is_owned() = source_is_owned(self.source)` (the current-compile-source form step 3 reads at a call site). Names a fact loft already carries; **no behaviour change** (no caller yet). Boundary documented on `source_is_owned`: loft's entry is a single `MAIN_SOURCE` file + `use`d libs; a multi-source entry package would need the package-path-prefix map. | ✅ `source_ownership_distinguishes_entry_dependency_stdlib`: an entry def (MAIN_SOURCE) → owned; a def parsed at source 2 (what a `use`d lib gets) → NOT owned; stdlib → NOT owned; `caller_source_is_owned()` tracks the current parse source | S |
| 3 | ✅ **LANDED (2026-07-16). Steer emission — GATED, the one risky step.** At `call_nr` (the shared call chokepoint), a resolved call FROM OWNED source (`self.data.caller_source_is_owned()`) to a `superseded`-marked def emits `Level::Warning` *"`X` is superseded — use `Y` (the old form keeps working)"* (`X` shown with the `n_` prefix stripped). Second-pass + `report` only (fires once per call site); strings cloned before the diagnostic to keep `&self.data`/`&mut self.lexer` disjoint. Gate `LOFT_NO_STEER` (`keys::steer_enabled`, default **on**, opt-out), mirroring `nullflow_enabled`; inert until a symbol is marked (suite byte-identical). Both backends share the parser + the pre-branch diagnostic render, so they surface it identically — NOTE it is a parse-time diagnostic, so a whole-program WARM-cache hit skips it (like every inline parse warning; the author sees it on every cold compile, `LOFT_NO_CACHE=1` forces cold). | ✅ `steer_fires_from_owned_source_silent_from_dependency` (parser unit, backend-agnostic — owned warns, source-2 silent, SAME mechanism); `tests/steer_warning.rs` (steer fires on interpret AND `--native`; `LOFT_NO_STEER=1` silences) | **M** |
| 4 | ✅ **LANDED (2026-07-16). The fold lint (C5.2).** `use_analysis::superseded_fold_diagnostics` — a pass over `Data` (wired into the compile path beside `warn_dead_stores`; runs in `make ci`) over every `#superseded "Y"` symbol X **in owned source** (same provenance gate — a consumer never sees a dependency's fold issues): (a) resolve Y (X's own source, then `STD_SOURCE`) — unresolvable ⇒ hard `Level::Error`, the compile fails (a dangling steer never ships); (b) X's body must CALL Y (`code.any_node` for `Call(y_nr,…)`) — un-folded ⇒ advisory `Level::Warning` (promote to a hard `make ci` gate once the surface is clean). Inert until a symbol is marked ⇒ suite byte-identical. | ✅ `fold_lint_flags_dangling_and_unfolded_superseded` (folded → clean · un-folded → warns · dangling → errors); end-to-end: a dangling successor exits non-zero, an un-folded one warns + runs | S–M |
| 5 | ✅ **LANDED (2026-07-16). Default-on confirmation + docs.** Inertness MEASURED — the full suite ran with steps 1–4 wired in and NO symbol marked: 2977 passed, the only fails the 3 known heavy-serial flakes, zero new warnings (byte-identical). The mechanism (`#superseded` attribute + the owned-source steer + the enforced fold) is documented in **[COMPATIBILITY.md § Folding](../../COMPATIBILITY.md)** (the canonical home — LOFT.md has no definition-attribute section, its `#…` are loop attributes); the **`LOFT_NO_STEER`** toggle is in `CLAUDE.md`'s diagnostic-toggle reference. | ✅ suite zero new warnings (inert); COMPATIBILITY.md § Folding + the toggle ref updated | S |
| 6 | ✅ **LANDED (2026-07-16). The first REAL steer (dogfood).** Folded **`text.contains` → `find`** (`default/03_text.loft`): `contains(self, value) { self.find(value) != null }`, marked `#superseded "find"` (the native `#rust` body deleted — the shim runs on both backends). A textbook superset (`contains ≡ find != null`). **Blast radius = ZERO test failures** (full suite 2979/2979): its ~14 owned `.loft` callers now warn but don't fail (the script runner ignores warnings without `--deny-warnings`), and it has no `code!` callers. Two mechanism refinements this needed: the fold lint now also checks **`STD_SOURCE`** (so loft's `make ci` enforces its own stdlib folds, not just entry-project ones), and the steer + fold lint resolve/render **method** successors (`t_<LEN><Type>_<succ>`) via a shared `Definition::display_name()` (`t_4text_contains` reads as `contains`). *Finding → FIXED:* the scout's first pick `sum_of → sum` was blocked — a stdlib fn could not call a **generic** stdlib fn (`sum<T: Addable>`) from its own source (worked fine from a user program). Root cause: `parse_call`'s generic-instantiation block was gated on `!self.default`, skipping the stdlib parse. **Fixed** — generic instantiation is now caller-source-agnostic (the guard removed; inert for the current stdlib, proven byte-identical; regression `stdlib_fn_can_call_a_generic` + end-to-end on both backends). With that unblocked, **`sum_of → sum` is now also folded** (the scout's #1 pick): `sum_of(v) { sum(v, 0) }` `#superseded "sum"`, `__add_int` dropped, defined *after* `sum` (a forward-ref to a generic is a separate still-open limitation). Needed one more fold-lint refinement — the shim check matches the successor by **user-facing name**, since a call to a GENERIC successor targets its monomorphisation (`t_<Type>_sum`), not the `n_sum` template. Blast radius: the 2 `code!` regressions migrated to `sum`; the 2 `tests/scripts` aggregate tests now `@EXPECT_WARNING` the steer (honest, not silently tolerated). Suite green. | ✅ suite 2979/2979; the folded `contains` still passes every test; owned callers steer to `find`; the fold lint is green on `contains`; dependency-source libs do not warn | **M** |
| 7 | **(Later) type/field steers + the registry migration-scan.** Extend emission to a superseded *type/field* use at type-resolution (same provenance fact, second read point). Wire the registry scan (arc B-registry) to report which *published* libs use each `#superseded` idiom — the owner's fold-prioritisation map + author-facing nudge. Defer until the MVP proves out and B-registry exists. | a superseded type used in owned source warns; the scan lists public-lib users of a marked idiom | M |

**Shape:** keystone risk in **step 3** (the provenance gate — the Q3 resolution in code) and proof in
**step 6** (the first real fold, end-to-end, with the dependency-source-silent positive control).
Steps 1–2 are inert plumbing; 4–5 make the discipline enforceable; 7 is the ecosystem tie-in,
deferred. The whole MVP is steps 1–6, entirely in this repo.

## Folding's limit (the boundary the design must respect)

Folding works **only when Y is a genuine superset** — same observable semantics, nicer form. So:

- `#superseded` is for **preference steers over supersets** — "use `write_through` instead of the
  plain-bind idiom", "use `floor_mod` instead of the manual wrap". The old and new produce identical
  observable results; the fold is a pure refactor.
- A **semantic** replacement (the old behaviour is *not* expressible over the new — @C86) is NOT an
  arc-C steer. It is **contract-keyed** (C4 / leg 3): loft carries both behaviours, keyed on the
  declared `contract`; the author sees the new idiom only when they bump contract. Arc C's channel is
  *reused* to deliver that author-alert (same owned-source gate), but the trigger and the "fold" (=
  contract-keying) are C4's, built when the first keyed change is genuinely needed.

Stating this boundary is load-bearing: it stops the elegant over-absorption of @C86 into the bare
`#superseded` channel, where it would be a lie (the fold cannot exist).

## First applications + the pre-freeze survey (2026-07-18)

Before applying `#superseded` broadly, we surveyed both the stdlib and the whole `loft-libs-*`
ecosystem for genuine candidates. **The finding is that clear-cut candidates are rare — and that
scarcity is the point.** Needing many marks at pre-freeze would be a *smell*: it says the surface you
are about to freeze is full of idioms you already regret and are steering away from, when the
pre-freeze window (arc E) exists to *fix* those (remove/rename while contract 0 allows), not ship them
and paper over with steers. A clean surface needs almost no steering.

**The load-bearing distinction the survey surfaced: FOLD ≠ SUPERSEDE.** Many functions are thin shims
over a more general one (`ln`→`log(x, E)`, `clamp`→`min(max(...))`, `contains`→`find(v) != null`,
`exists`→`file(p).format != NotExists`, the lib native-wrappers). Folding an implementation onto a
primitive is good DRY — but it is *not* a reason to steer callers away from the name. You supersede
only when the new form is **strictly better AND the old adds no distinct concept**; you fold for
implementation reuse regardless. `ln`/`clamp`/`contains` are all fold-yes, steer-no.

The three real cases:

| Symbol | Verdict | Why |
|---|---|---|
| `sum_of` → `sum` (stdlib) | **keep marked** | `sum(v, init)` is a strict *generalization* (superset, same return type, `sum(v, 0)` costs ~nothing); `_of` adds no concept. The one defensible stdlib supersession. |
| `contains` → `find` (stdlib) | **mark REMOVED** | `find` is not a generalization of `contains` — it returns a *position*, `contains` a *boolean*; different questions. Steering the membership test to `find(v) != null` is more verbose + less clear. A fold, wrongly marked as a supersede. |
| `regex` — 4 spellings → clean verbs `search`/`matches`/`split_on` (loft-libs-core) | **first ecosystem mark** | regex exposed *four* spellings for two ops: pattern-first free fns (`regex::find`/`regex::split`) **and** `regex_`-prefixed methods (`line.regex_find`/`line.regex_split`). The prefix was a mechanical dodge: bare `find`/`split` as `text` methods collide with the stdlib literal `text.find`/`text.split` — and **@C95 blocks reusing the name even as an argument-type overload** (verified: `Cannot redefine 'split'`). Consolidated on the *conventional* regex verbs as clean methods — `search` (cf. JS `String.search`, Python `re.search`), `matches`, `split_on` — and superseded all four old spellings → the two clean verbs. A genuine *pre-freeze API cleanup* that `#superseded` implements. |

**Naming under the collision constraint (why the clean form is a method with a *conventional* verb, not
a prefix).** The instinct "just call it `find`" fails: the stdlib owns `text.find`/`text.split` as the
*literal* operations, so a regex method cannot reuse those names (@C95, hard error — not even a
by-argument overload). The mechanical fix (`regex_find`) reads badly. The clean fix is the verb every
regex API already uses: `search` for find-position and `split_on` for pattern-split (only `split` needs
a distinguishing word; `search` and `matches` are collision-free bare names). This keeps the *method*
ergonomics the caller wants (`line.search(p)`) without the prefix. The lesson generalizes: when a clean
name is taken by a neighbouring literal operation, prefer the domain's **conventional verb** over a
namespacing prefix.

**Mechanism validated end-to-end** on the regex case: a consumer calling *either* old spelling from
OWNED source — `regex::find(...)` or `line.regex_find(...)` — gets `warning: `find` (resp.
`regex_find`) is superseded — use `search` (the old form keeps working)`, the program still returns the
correct result, and `LOFT_NO_STEER=1` silences it. That is the whole channel exercised on one clean,
real case — not a scattering of marks. (The same PR also fixed a latent @PLN110 bug the audit surfaced:
`split_iter` used `len()`, a character count, as a *byte* slice bound → now `size()`.)

## See also

- [COMPATIBILITY.md](../../COMPATIBILITY.md) — § Deprecation is soft steering · § Folding · § Two
  populations · § Warnings are not a covered surface (why the channel outlives the freeze).
- [compat-gate-build.md](compat-gate-build.md) — the sibling build plan; arc C is its **C5** (C1 done,
  C2/C3 = B-registry, C4 = contract-keying). This doc is C5 in full.
- [README.md](README.md) — the arc table + Q3 (open-question 3), resolved here.
- [versioning-decision.md](versioning-decision.md) — the `contract` axis C4's keyed-change alert rides.
- [GOALS.md](../../GOALS.md) Goal F — warnings are the only channel that may bill the programmer.
- Code-points: `Definition.superseded` (`src/data.rs`) · attribute parse (`src/parser/definitions.rs`,
  beside `#pure`/`#native`) · emission at `call_nr` (`src/parser/mod.rs`) · provenance
  (`Definition.source`, `MAIN_SOURCE`/`STD_SOURCE`, `def_library`) · the fold lint (a `make ci` pass
  over `Data`, cf. `LOFT_NO_DEAD_STORES`).
