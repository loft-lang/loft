<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — the language-versioning decision (the pivot, open Q2)

> **⚠ STRENGTHENED 2026-07-10 by the absolute-compat commitment — read this first.** After
> this doc was written the owner set a stronger rule: **at contract 1, loft never breaks a
> functioning program (language + errors + libs), whatever we do; a deviation is a bug, not
> a managed breaking change** ([COMPATIBILITY.md](../../COMPATIBILITY.md)). Two concrete
> effects on the mechanism below, which is otherwise intact:
> 1. **The contract does NOT "count silent breaking changes"** — post-1 there *are* none
>    (they are bugs, fixed stop-the-world). It is a monotone **capability floor** (+ a
>    behavior-epoch key for the rare escape-valve change). The framing below of "increments
>    on a silent break" is superseded; a silent break is now a *bug*, and the contract only
>    rises for additive capability epochs.
> 2. **A bare `contract = N` is a lower bound (`>= N`, forward-open), NOT "tested-at N /
>    warn on newer."** Forward-compat is guaranteed, so a newer loft never drifts; `Drifted`
>    fires only for an *explicit* upper bound. (The mechanism's bare case was flipped
>    `=`→`>=` — `check_contract`.)
> 3. **The `0 → 1` flip is gated on a thorough pre-freeze audit** of every surface (see
>    COMPATIBILITY.md § Before the flip), not only "after the open syntax changes."
>
> The rest of this doc is the original reasoning (monotone integer, calver for releases,
> `1.0 == contract 1`, "an integer suffices because feature-floor fails loudly") — all still
> correct; only the two semantics above are refined by the stronger promise.

**Status: DECIDED 2026-07-10** (ratification of the two cosmetic sub-choices below is
the user's — the substance is committed). This resolves the pivot that arc B-semantic
and arc E (the 1.0 line) both wait on. Written design-protocol-first: the invariant, the
failure paths, the falsified alternatives, then the mechanism.

## The decision, in one line

**loft gets a monotone integer `contract` version — separate from the calendar release
tag — that increments if and only if loft makes a *silent* breaking change; libraries
declare the contract range they were tested against; `1.0` is contract 1.**

- **Releases stay calendar-versioned** (`2026.7.1`) — the tag you install; it
  communicates recency and cadence and is unchanged.
- **The contract surface gets its own integer**, `contract`, decoupled from the release
  tag. It counts *silent breaking changes* to loft's contract (language
  syntax/semantics, stdlib API meaning, store/heap layout, on-disk + wire format,
  package format). Additive and loud-failing changes do **not** bump it.
- **1.0 == contract 1** (ratified). Everything before the syntax/type freeze is contract
  0 — "unstable, no compatibility promise." When the surface freezes (imminent — the type
  surface is feature-complete on `main`, the last syntax changes in flight), `CONTRACT_VERSION`
  flips 0 → 1 and the promise begins. The versioning pivot and arc E are the **same
  milestone**. **The mechanism ships now at 0 (inert); the flip to 1 lands *after* the
  open syntax changes** — see § The two sub-choices.

## The invariant (design-protocol step 1)

> **The `contract` integer increments if and only if a change would make
> already-shipped library code compile-and-run to a *different result* — a silent
> breaking change. Every other change (additive, or one that fails loudly) leaves it
> untouched.**

So a library's declared bound stays valid *exactly* as long as loft's silent contract is
unchanged — that is the single rule under which a library never tested against a future
loft still behaves correctly for the same reason the tested ones do: either the contract
is unchanged (safe), or it moved and the library is told.

## Why an integer is *sufficient* — the load-bearing claim, probed

The obvious objection: an integer that bumps only on breaking changes cannot express a
**feature floor** ("I need a stdlib fn added in a later loft"). Semver's minor/patch
exists for exactly that. Do we need it?

**No — because the feature-floor case already fails LOUDLY, so it needs no version
guard.** Probed against the interpreter (2026-07-10): a program using a symbol loft does
not have fails at compile with a hard error, not a silent wrong answer:

```
error: Unknown function some_function_added_in_a_future_loft   (rc=1)
error: Unknown field text.method_from_the_future               (rc=1)
```

The class this whole plan exists to kill is the **silent** one — old code that still
compiles and runs but now does the *wrong* thing (@C86 plain-bind copy → `hex_terrain`'s
`0 land cells`). That is precisely the *breaking-change* class, and a monotone
breaking-change integer is exactly the axis that captures it. The loud class is already
safe without any version bound. So minor/patch buys nothing here: **collapse semver to
major-only, i.e. a single integer.** (This is why option B below reduces to option A.)

## Semantics — how a bound is checked

A library declares `contract = K` (or a range). Let `E` be loft's current contract:

| relation | meaning | loader action |
|---|---|---|
| `E == K` | exact epoch match | **accept** |
| `E < K` | loft is *older* than the library's epoch (the library may rely on post-K semantics) | **reject** — "requires loft contract ≥ K, this is E" |
| `E > K` | loft made ≥1 silent break since K; the library may misbehave | **warn** via the deprecation channel (arc C) — "tested against contract K; loft is at E; republish" |

A bare `contract = K` means **"tested at epoch K"** and warns on a newer loft — the
*honest* default, because forward-compatibility is exactly the thing @C86 silently
assumed and broke. An author who has re-tested widens to a range (`">=K"`, or
`">=K, <=E"`) to assert forward-coverage and clear the warning. This puts the cost of
change on the maker (republish/re-test), never silently on the consumer — the GOALS.md
standard.

Ranges reuse **arc B-mechanical's parser unchanged** (`>=`/`<=`/`>`/`<`/`=` + comma), now
over the contract integer instead of the release version. That is the whole point of the
B split: the mechanism already shipped; B-semantic only points it at the right axis.

## Failure paths (design-protocol — enumerate how it breaks)

1. **Maintainer forgets to bump `contract` on a silent break.** THE real brittleness:
   the integer is only as honest as the discipline that increments it, and a semantic
   break (@C86-shaped) changes no hash and throws no error, so nothing *automatically*
   catches the omission. Mitigation, and the tie to the other arcs:
   - The @PLN97 **layout-identity hash** already detects store/heap-layout breaks
     mechanically → a CI gate "layout hash changed ⇒ contract must bump" makes omission
     loud *for that surface* (answers open Q1: a layout-hash change *is* a contract
     bump).
   - The **golden-behavior corpus** (a curated set of programs whose stdout is pinned)
     turns a silent semantic break into a diff → "golden output changed ⇒ classify:
     intended break ⇒ bump contract." This is arc A/C work; the versioning decision
     *names it as required*, it does not hand-wave it away.
   - Residual after both: a semantic break in an *un-corpus'd* shape can still ship
     without a bump. That is the dogfood loop's job to surface, and it is strictly
     smaller than today's "no axis at all."
2. **A break to surface X, a library that only uses surface Y.** The single integer is
   **conservative**: it warns library Y even though the break can't touch it. Named
   residual — see below. Accepted because the alternative (per-surface versions) makes
   the *author* track which of five surfaces they touch, which is the wrong cost.
3. **Existing libraries carry no `contract`.** All ~20 declare only `loft = ">=0.8"`.
   Treated as **contract-unknown**: they keep loading (grandfathered), and the registry
   nudges a `contract` at republish; at 1.0 the registry can *require* `contract = 1`
   for new submissions. No flag-day break.
4. **`E < K` on a genuinely forward-only library.** Correct to reject — an old loft
   cannot honour a newer contract. This is the one hard-reject; it is the symmetric,
   honest twin of the warn-on-newer default.

## Falsified alternatives (design-protocol step 4 — attack the clean claims)

- **Full semver for releases (drop calver).** Abandons a deliberate project choice and
  forces *every* release to be classified major/minor/patch — the exact per-release
  breaking-change judgement calver was chosen to avoid. Rejected; calver stays for
  releases.
- **Two numbers: a language-semver + calver.** The minor/patch of the language-semver
  buys the feature-floor expressiveness — which the probe above shows is **not needed**
  (feature-missing fails loudly). So the minor/patch is complexity with no payoff:
  collapse to major-only = the single integer. This is *option B reducing to option A*,
  and the reduction is *earned by the probe*, not assumed.
- **Per-surface contract versions (5 axes).** Maximally precise (a wire break doesn't
  warn a pure-syntax library), but it makes the author answer "which surfaces do I
  touch?" — genuinely hard (does using a vector touch store layout?). Wrong cost on the
  wrong party. The single conservative integer + the layout hash for the one
  accidentally-easy-to-break surface is the right trade. Rejected as over-broad for the
  author.
- **Reinterpret the existing `loft = "…"` field as the contract.** Every published lib
  says `loft = ">=0.8"` meaning the *release* 0.8; reinterpreting silently changes what
  those bounds mean. Rejected — the contract is a **new** field; the release-version
  bound stays grandfathered.

## The named residual (don't hide it)

The single integer **over-warns**: a break confined to surface X warns every library
declaring an older contract, including those that only use surface Y and are genuinely
fine. This is deliberate — it is a *warning*, not a hard reject (Goal F), the author
clears it with a one-line re-test + republish (widen to a range), and it trades a little
noise for the author never having to enumerate surfaces. The precise-but-heavy
alternative (per-surface) was rejected for that reason. The over-warn is the honest
price of the simplest axis, and it fails safe (warn, not silent-wrong).

## What arc B-semantic builds — ✅ IMPLEMENTED 2026-07-10 (except the CI gates)

1. ✅ `manifest::CONTRACT_VERSION` — loft's current contract, `0` pre-freeze (the
   language surface is still settling), to become `1` at the 1.0 freeze. Separate from
   `CARGO_PKG_VERSION`.
2. ✅ The `[package] contract = "..."` manifest field (`Manifest::contract`).
3. ✅ `manifest::check_contract` + the `ContractCheck { Ok | TooOld | Drifted | Malformed }`
   outcome, folding the constraint into a `[lo, hi]` window (bare integer = exact
   "tested-at", `>=K` opens the ceiling), with the loader (`parser/mod.rs`) rejecting
   `TooOld`, warning on `Drifted` (the arc-C channel — loads, does not reject), and
   rejecting `Malformed`. Unit test `arc_b_semantic_contract_check` + integration
   fixtures `testpkg_contract_future` (too-new → fatal) / `testpkg_contract_ok`
   (current → loads). The `Drifted` warn arm is inert until `CONTRACT_VERSION > 0`.
4. ⬜ **Still open** — the CI gates that make an omitted bump loud: layout-hash-changed
   ⇒ contract bump; golden-corpus-output-changed ⇒ classify + bump (shared with arcs
   A/C). This is the failure-path-1 mitigation; it lands with the policy arcs.

## The two sub-choices — RATIFIED 2026-07-10

Both ratified by the owner:

1. **The field + concept name → `contract`.** (Chosen over `edition` to avoid the
   Rust-`edition` connotation — Rust editions are *multiplexed, additive* opt-ins the
   compiler supports simultaneously, the opposite of a monotone compatibility floor.)
   Already the field name in `manifest.rs`.
2. **The baseline integer → 1.** The compatibility contract starts at **1** (not a
   pre-1.0 `0`): contract 1 is the 1.0 baseline. **Sequencing:** the mechanism ships now
   with `CONTRACT_VERSION = 0` (inert — no existing library declares `contract`, so
   nothing gates), and the **0 → 1 flip lands *after* the last open syntax changes
   settle** — those changes are part of defining what contract 1 *is*, so declaring the
   baseline before they land would freeze a moving contract. The flip is a one-line,
   deliberate follow-up at the freeze. A library tested against the pre-1.0 language may
   carry `contract = "0"`, which then drifts (warns) once loft is at contract 1.

## See also

- [README.md](README.md) — the plan; § Phase ordering (this decision is step 2, the
  pivot) and open question 2.
- [../97-layout-contract/](../97-layout-contract/) — the layout-identity hash; the
  automatic detector for the store surface (failure path 1) and the answer to open Q1.
- [../../DESIGN_DECISIONS.md § @C86](../../DESIGN_DECISIONS.md) — the plain-bind copy: the
  archetypal *silent* break this axis exists to make declarable.
- [../../GOALS.md](../../GOALS.md) — Goal F (warnings are the only channel that may bill
  the programmer) governs the `E > K` warn; the AS/400 standard (cost of change paid by
  the maker) governs the republish-to-clear model.
