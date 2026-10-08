<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 102 — Stability contract for scale

## Status

**Open — arc E is the critical path.**  Most of the mechanism has LANDED: arcs
A (policy), B-mechanical + B-semantic (constraint parser + `contract` loader),
C (the recommended-idiom steer+fold MVP, steps 1–6), D (public bug-intake MVP), and
the C1 compat-gate MVP (api-surface + two-axis diff) are all shipped.  What remains is
**arc E — the `0 → 1` pre-freeze flip** and its few unbuilt pieces, with small-safe-step
designs now written: [flip-gate.md](flip-gate.md) (the gate checklist + the CI drift
gates + the flip procedure), [test-hygiene-warnings.md](test-hygiene-warnings.md)
(remove the tolerated-warnings filter), and [layout-nullability-identity.md](layout-nullability-identity.md)
(F9).  Deferred/external: arc B-registry Tier-2, arc C step 7, arc D step 5.  This is
the wide-release bar's **gate 5**
([STABILITY_ROADMAP.md § The wide-release bar](../../STABILITY_ROADMAP.md)).  Its
opening condition — *"open one when gate 1 is in sight"* — fired **2026-07-10**.  As of
that date gate 1 is **sealed pending merge**: its last structural item (the Cluster C
`copy_claims` fold) landed on `tuxedo-cluster-c`, its fuzz/sanitizer proof stands
(@PLN53/@PLN54 closed), and the nightly DA + `stack_align_guard` gates were widened to
the whole in-process interpreter corpus — so gate 5 is now the genuinely-active next
gate, not merely "in sight."  Gates 2 (@PLN25) and 3 (@PLN28 / @PLN36) are already
closed.

The plan is opened **because the failure mode it prevents is already live**, not as a
speculative hardening.  See § Why now.

**Design refinement (2026-07-10):** arc B splits into a *mechanical* half (bind upper
bounds/ranges, loudly reject unparseable — closes the verified silent failure,
independent of any policy) and a *semantic* half (what a bound *means*), and the
semantic half depended on the **language-versioning decision** (the pivot), which is now
**DECIDED** — [versioning-decision.md](versioning-decision.md): a monotone integer
`contract` version, calver kept for release tags, `1.0` == contract 1.  See § Phase
ordering.

**Timing (2026-07-10):** the language's **type surface is now feature-complete on `main`
and the last syntax changes are in flight** — so the versioning pivot and arc E (the 1.0
line) are no longer far off: "what is frozen" is about to have a concrete answer, which
is exactly when the compatibility axis must be decided.  **Status update:** arc
B-**mechanical** is now **IMPLEMENTED** (a real constraint parser: binds `>=`/`<=`/`>`/`<`/`=`
and comma ranges, grandfathers a bare `>=`, rejects the unparseable via
`VersionCheck::Malformed`; `src/manifest.rs`, plan matrix + both-parser-path fixtures).
**Status update (2026-07-19):** B-semantic + A + C-MVP + D-MVP + the C1 compat-gate MVP
have since landed; only **arc E** (the pre-freeze flip) and the deferred Tier-2/external
pieces remain — see the § Sub-arcs table and [flip-gate.md](flip-gate.md).

## Goal

Ship a stated compatibility contract for loft, plus the mechanism that enforces it —
expressible version bounds, a deprecation channel, a public bug-intake path, and a
declared 1.0 line — so that a loft release cannot silently break the libraries and
programs built on it.

## Effort + design

- **Effort:** L (multi-arc: policy + mechanism)
- **Design:** ~ (partial — arc B specified by the matrix below; A / C / E need design)
- **Last touched:** 2026-07-10

## Why now — the failure mode is live, not hypothetical

`hex_terrain 0.1.0` fails its own registry-validation test with `0 land cells`.  The
library uses the plain-bind write-through idiom throughout:

```loft
th = t.tr_h;      // intent: alias the terrain's height vector
th[i] = value;    // intent: write through to t.tr_h
```

loft now **copies on plain bind** ([DESIGN_DECISIONS.md § @C86](../../DESIGN_DECISIONS.md)
— H-Copy: whole-value heap binds copy; aliasing is a last-use elision), so the writes
land in a throwaway copy, `t.tr_h` stays all-zeros, every cell classifies as sea, and
the island has no interior.  **It does not crash.  It computes a plausible wrong
answer.**

`graphics` hit the identical class and was migrated to `&self.data`.  `hex_terrain` was
never migrated.  Both declare `loft = ">=0.8"`, so nothing guarded either of them.

As of 2026-07-10 `hex_terrain` is the **sole** red leg on `registry-validation` (the
other former failure, `graphics`, was a CI-provisioning gap — missing `libasound2-dev`
— now fixed), which makes it the clean isolated exemplar for this plan.  Note that
**none of this plan's arcs fix `hex_terrain` itself**: arc B would let it *declare*
incompatibility, arc C would *warn* its author, but the library still computes a wrong
answer until it is **republished** with the `&self.data` idiom (external — loft-libs-game).
The plan removes the *silence*, not the library bug.

[GOALS.md](../../GOALS.md) holds the platform to the opposite standard, and names it as
the thing loft is trying to win back from the AS/400:

> backward compatibility was a contract the maker kept: the platform never broke its
> users; the cost of change was paid by the maker, not the customer.

A compatibility promise with a deprecation channel is the mechanism that would have
caught this before publication.

## Composition matrix — Stage A

Arc **B** extends an existing operation (`manifest::check_version`), so it carries a
matrix.  Arcs **A / D / E** are policy and documentation with no composition surface,
and arc **C** gains its matrix once its warning shape is designed — stated here rather
than left silent.

`check_version` (`src/manifest.rs:513`) strips a leading `>=` and parses the remainder
numerically with `unwrap_or(0)`, so **every non-`>=` form degrades to `0.0.0` and always
passes**.  Probed against the current interpreter, `2026.7.1`:

| `loft = ` in `loft.toml` | today | required after arc B |
|---|---|---|
| `">=0.8"` | ACCEPT | ACCEPT |
| `">=9999.0"` | **REJECT** | REJECT |
| `"<=0.1"` | ACCEPT | **REJECT** — upper bound must bind |
| `"<2026.0.0"` | ACCEPT | **REJECT** |
| `"=0.1"` | ACCEPT | **REJECT** — exact pin must bind |
| `"^0.9"` | ACCEPT | **REJECT** (or support caret; decide in arc B) |
| `">=0.8, <2027"` | *unrepresentable* | ACCEPT — ranges must parse |
| `"garbage"` | ACCEPT | **REJECT LOUDLY** — a constraint you don't honour is a category-S silent failure |

The `">=9999.0"` row is the **positive control**: it is the one cell that fires today's
gate (`"Package X requires loft R but interpreter is C"`), which proves the harness can
fail and that the other rows' silence is a real accept, not a dead probe.  Accept cells
proceed past the version gate into downstream symbol resolution; the gate's own failure
is that distinct fatal.

Two consequences, both load-bearing for this plan:

1. A library **cannot** declare that it does not work with loft ≥ X.  There is no
   syntax that binds, so `hex_terrain` could not have protected itself even if its
   author had known.
2. Under **calendar versioning** (`2026.7.1`), a `>=0.8` lower bound is *permanently*
   vacuous — every future loft satisfies it.  Every library in the registry carries one.

Cells graduate to `tests/` as arc B's regression suite (both backends: the check runs in
the parser, so `--interpret` and `--native` must agree on accept/reject — an
accept/reject divergence here is a Goal-D violation).

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — Compatibility policy: what *is* a breaking change, per surface | [../../COMPATIBILITY.md](../../COMPATIBILITY.md) | **✅ DRAFTED + STRENGTHENED** (2026-07-10). Owner set an **absolute** promise: at contract 1 no functioning program breaks — **language + errors + libs** (+ its data) — whatever we do; a deviation is a *bug*, not a managed change. Additive-only; deprecation = soft steering (never warn-then-remove); the rare unavoidable change is contract-keyed (edition-style). A **ratchet**: every addition from 1 on is frozen too, so we add carefully. |
| **B-mechanical** — bind upper bounds + ranges + exact pins; loud rejection of unparseable constraints; grandfather a bare `>=` as "unknown compatibility" | `src/manifest.rs` (`check_version` → `VersionCheck`), `src/parser/mod.rs` (loader), matrix unit test + `testpkg_badconstraint`/`testpkg_upperbound` fixtures | **✅ IMPLEMENTED** (2026-07-10) |
| **B-registry** — never-break **leg 2**: a **curated validation pass**, not passive storage and not a fully-automatic gate. Automation prepares the evidence — API verification (public surface must be an additive superset), old-tests regression, a **sandboxed all-target run** (verify, don't trust a claimed pass), and it **stores more than the author made**: the registry's OWN behavioural + API baseline **and the built interpret/native/wasm/html artifacts** (durable vs a weakened test suite AND toolchain drift — the built forms stay runnable). A **human makes the acceptance decision** (is the baseline *adequate*? worth the perpetual commitment?). On a break, **preserve the old version first** then accept the new as a distinct opt-in version. **NEW status vs today:** the gate's API-compat verdict (drop-in superset vs breaking epoch) is *recorded + surfaced per version transition* — today a bump only says the lib *changed*, not *whether* it's backwards-compatible; this is a real addition to the registry data model, and what makes migration *informed*. See [COMPATIBILITY.md § Two populations](../../COMPATIBILITY.md). | external — loft-lang/registry | Open — the real infra + curation cost |
| **B-semantic** — the `contract` integer + loader semantics (reject-below / accept-in-range / warn-above) | `src/manifest.rs` (`CONTRACT_VERSION`, `ContractCheck`, `check_contract`), `src/parser/mod.rs` (loader), `[package] contract` field, unit + `testpkg_contract_*` fixtures | **✅ IMPLEMENTED** (2026-07-10). Name + baseline **ratified**: field `contract`, baseline **1**. Ships at `CONTRACT_VERSION = 0` (inert — nothing declares `contract` yet); the **0 → 1 flip lands after the last open syntax changes settle** (they define what contract 1 is) |
| **C** — Recommended-idiom channel (**not** deprecation-toward-removal): "never break" + no usage telemetry makes the surface a one-way ratchet, so a steer can never reach a removal — it signposts a nicer idiom (warnings only, [Goal F](../../GOALS.md)) and warns the *author* of a contract-keyed semantic change (worked example = @C86 / `hex_terrain`). The engineer-around is **folding**: keep the callable name, fold its implementation onto the new primitive (interface grows, duplicate code does not) — see [COMPATIBILITY.md § Folding](../../COMPATIBILITY.md). | **✅ DESIGNED** (2026-07-16) → [recommended-idiom-channel.md](recommended-idiom-channel.md). Q3 resolved (a provenance gate on the CALLER — fire only on owned/entry source, never on imported-dependency source, so the steer always reaches whoever can act); delivery decided (dev-compile primary + registry-scan secondary); the `#superseded`+fold-lint mechanism + a 6-step inert-first ladder specified; contract-keyed semantic changes (@C86's class) routed to C4, not folded. **✅ MVP LANDED (2026-07-16): steps 1–6** (the `#superseded` attribute, the owned-source provenance gate + steer, the enforced fold lint, and the first real dogfood folds `text.contains → find` and `sum_of → sum`) all shipped, suite byte-identical when inert. **Step 7 deferred** (type/field steers + the registry migration-scan — needs arc B-registry). |
| **D** — Public bug-intake path: the fix-not-file discipline is internal and does not reach strangers | [ISSUE_TRACKING.md](../../ISSUE_TRACKING.md) · [public-bug-intake.md](public-bug-intake.md) | **✅ MVP LANDED** (2026-07-16). The public-facing FORM already existed (minimal-repro `bug_report.yml` + `config.yml` chooser + `CONTRIBUTING § Reporting a bug`); this built the INTERNAL side. **Steps 1–4 landed:** the **triage bridge** (public report → acknowledge/`needs-triage` → reproduce + minimise both-backend → fix-not-file → close, never `wontfix` a regression) in [ISSUE_TRACKING.md § The public intake bridge](../../ISSUE_TRACKING.md); the **never-break promise** at the intake (`bug_report.yml` intro + `CONTRIBUTING` + new root `SUPPORT.md`); **repo-routing** (`config.yml` chooser); the **acknowledgement discipline** (the queryable `needs-triage` label, `.github/LABELS.md`). Reconciliation-with-fix-not-file + failure-paths in [public-bug-intake.md](public-bug-intake.md). **Step 5 (`loft report` bundle helper) deferred** until the manual path proves friction. |
| **E** — The 1.0 line: what is frozen vs still moving, **and the pre-freeze AUDIT** | [RELEASE.md](../../RELEASE.md), [COMPATIBILITY.md § Before the flip](../../COMPATIBILITY.md), [INCONSISTENCIES.md](../../INCONSISTENCIES.md) | Open — **the critical path.** The `0→1` flip is a ONE-WAY DOOR gated on a thorough surface-by-surface audit (semantics · syntax · errors · stdlib · formats): fix every wart we'd regret while contract 0 still allows. A miss is permanent (live-with-it, engineer-around). Anchor on INCONSISTENCIES.md (High=silent-wrong=must-fix). This is the largest remaining 1.0 work item. **Dual-phase (owner): (1) close the open plans → settle the language [underway], then (2) a dedicated unhurried pass on the LIB side (stdlib + core libs — equally permanent).** Only then is the flip earned. **Worklists PREPPED:** [lib-audit.md](lib-audit.md) (stdlib surface — 4-agent survey) + [formal-audit.md](formal-audit.md) (language basis + error surface — 6-agent survey over `formal/*` + the error surface). Both share **the null-sentinel keystone**; the formal/error half adds float-`==`, ~~compound-assign double-eval~~ (**F2 FIXED + LANDED 2026-07-16**, `tuxedo-f2-impl-steps`), `&&`/`||` short-circuit, the `&v` copy-vs-alias spec contradiction, the format sub-language, layout persistence, and the diagnostic-identity fix. Each item worked as a design decision — alternatives presented + conversion set enumerated; the error surface is one-directional (drop, never add → be strict now). A 2026-07-17 **code-eval sweep** of the rebased stack fixed F2's two residual gaps + a fold-lint false-negative, and filed the remaining kinks — **K1 enum-variant `const` = a pre-freeze decision** (enforce now or never), K2–K4 cleanups — with small-step designs in [code-eval-followups.md](code-eval-followups.md). |

**Build plan for arcs B-registry + C (the never-break mechanism):**
[compat-gate-build.md](compat-gate-build.md) — component-by-component with steps + code-points (C1
API-surface diff = keystone · C2 the registry gate · C3 the new per-version compat status · C4
contract-keying [mostly done] · C5 the folding discipline), a build order, and the falsification list.

**Pre-freeze test-hygiene (arc E) — remove the tolerated-warnings list.**  `tests/testing.rs` carries
a **`is_runtime_warning` filter** that silently ABSORBS a fixed set of warnings so the
diagnostic-comparison harness never flags them as unexpected output — currently *"division/modulus may
produce null"*, *"`v[i]`/`s[i]` may produce null"*, *"field …"*, *"`&` on parameter …"*, and *"`not
null` is deprecated"*.  A test therefore PASSES **without asserting** these, so a semantic change (the
null-flow default-on cutover, the `&`-lint, the `not null` retirement, now the arc-C steer) can quietly
change *what warns* and no test notices.  **Task:** delete the tolerated list and make each affected
`code!` fixture CORRECT to the current semantics — either explicitly `.warning("…")`-assert the warning
it should now produce, or fix the `.loft` so it no longer emits one.  This is the warning-surface half
of arc E's *"be strict now"* one-way-door audit: the test suite should describe the diagnostics loft
*actually* emits today, not tolerate a frozen legacy set.  (End-to-end warning coverage already lives
in `tests/runtime_warnings.rs` / `tests/steer_warning.rs`; this is about the `code!` harness's silent
filter.)

## Phase ordering

Refined 2026-07-10 to separate the two halves of arc B and to surface the pivot the
plan's own open questions had left implicit.

1. **B-mechanical first — the sharpest evidence, smallest surface, no policy
   dependency.**  Replace `check_version` with a real constraint parser: bind upper
   bounds / ranges / exact pins, and **reject an unparseable constraint LOUDLY** (a
   constraint accepted-but-not-honoured is the verified category-S silent failure).
   Grandfather a bare `>=` lower bound as *"unknown compatibility"* (warn, do not
   reject) so the ~20 already-published libraries keep loading.  The matrix is the spec;
   both backends (the check runs in the parser).  This closes the *silent* half of the
   gap **before any policy is stated**, and gives `registry-validation` something real
   to check.
2. **The language-versioning decision — THE PIVOT.  ✅ DECIDED** →
   [versioning-decision.md](versioning-decision.md).  A monotone integer `contract`
   version (increments iff loft makes a *silent* breaking change), separate from the
   calver release tag; libraries declare a contract range; `1.0` == contract 1.  A
   version bound is only *meaningful* against such an axis — calver's `>=0.8` is
   permanently vacuous.  B-mechanical binds bounds without this; B-semantic points that
   same parser at the `contract` integer and adds the loader semantics
   (reject-below / accept-in-range / warn-above).
3. **A — policy before the rest of the mechanism. ✅ DRAFTED** →
   [../../COMPATIBILITY.md](../../COMPATIBILITY.md).  B-semantic enforces a rule; the
   rule ("what *is* a breaking change, per surface") now exists — the
   silent/loud/additive trichotomy per surface, what the maker owes per tier, and the
   per-surface detectors that make a misclassification a CI failure.  Unblocks C (it
   defines what must be deprecated) and E (it defines "breaking" for whatever E freezes).
4. **C after A** — the deprecation channel needs A's definition of "breaking" to know
   what it must warn about.  C is what would actually have caught `hex_terrain`; B only
   lets a library defend itself after the fact.  Its own open question (Q3, *whose*
   obligation is the warning) is the thorniest design in the plan — run the
   design-protocol.
5. **D in parallel** — independent of A/B/C; no dependencies.
6. **E last** — the 1.0 line is a statement about A (and the versioning decision), so it
   cannot precede them.

## Open design questions

1. **What does semver mean when the schema is data and the heap layout *is* the
   contract?**  @PLN97 shipped a formal memory/file layout contract with a layout-identity
   hash.  Is that hash the compatibility key for the store surface — i.e. does a layout-hash
   change *define* a breaking change there?  **✅ ANSWERED by the versioning decision:**
   yes — a layout-hash change is a silent store-surface break, so it bumps the `contract`
   integer, and a CI gate "layout hash changed ⇒ contract must bump" is what makes an
   omitted bump loud for that surface (see [versioning-decision.md](versioning-decision.md)
   failure path 1).
2. **Does the language version itself keep calendar versioning? — THE PIVOT.
   ✅ DECIDED 2026-07-10 → [versioning-decision.md](versioning-decision.md).**  A
   monotone integer `contract` version, separate from the calver release tag, that
   increments iff loft makes a *silent* breaking change; libraries declare the contract
   range they were tested against; **`1.0` == contract 1** (so the pivot and arc E are
   the same milestone).  An integer suffices — probed: the feature-floor case (needing a
   later-added symbol) already fails LOUDLY, so only the *silent* breaking-change class
   needs a version axis, which collapses semver to major-only.  Both sub-choices
   **RATIFIED 2026-07-10**: field name `contract`, baseline `1` — with the `0 → 1` flip
   landing after the last open syntax changes settle (see versioning-decision.md § The
   two sub-choices).
3. **Whose obligation is a deprecation warning?**  @C86 changed the meaning of code that
   libraries had *already shipped*.  A warning at loft-build time reaches the loft team;
   a warning at library-compile time reaches the author; a warning at consumer-compile
   time reaches the wrong person.  Where does it fire?  **✅ ANSWERED (2026-07-16) →
   [recommended-idiom-channel.md](recommended-idiom-channel.md):** a **provenance gate on the
   CALLER** — the steer fires iff the source *making the call* is the compilation's entry project
   (owned), never a resolved dependency or the stdlib.  This reaches the right person structurally:
   a library author building their lib is the entry (fires); a consumer importing it has the lib as
   a dependency (silent on the lib's internal idioms, but their own code still steered).  Load-bearing:
   a consumer DOES re-parse library source, so the gate is required, not optional.  @C86's *semantic*
   change is not this channel — it is contract-keyed (C4); arc C's fold is for supersets only.
4. **Is `loft = ">=0.8"` grandfathered?**  Every published library carries a vacuous
   bound.  Does arc B rewrite them, reject them, or treat a bare lower bound as
   "unknown compatibility"?  *Leaning: treat a bare `>=` as "unknown compatibility" —
   warn, do not reject* — so B-mechanical can land without breaking the ~20 published
   libraries that all carry one; a hard reject would strand every existing consumer the
   day it merges.  Malformed / unparseable constraints still reject loudly (that is the
   silent-failure fix); only the *legacy bare lower bound* is grandfathered.
5. **How does this interact with the registry `verified` mark?**  See
   [LIBRARY_CHECKLIST.md](../../LIBRARY_CHECKLIST.md).  Should `verified` assert
   "tested against loft X" rather than a point-in-time pass?

## Cross-arc dependencies

- **@PLN97** (layout contract, closed) — supplies the layout-identity hash; the natural
  compatibility key for the store/file surface.  Question 1 above.
- **PKG.7 — `loft.lock`** ([PACKAGES.md § Open work](../../PACKAGES.md)) — a lockfile pins a
  working set; complementary to a compat range, not a substitute (a lock protects an
  existing consumer, a range protects a *new* resolve).
- **`registry-validation` CI** — the enforcement point, and the detector that motivated this
  plan: it had never gone green, and `hex_terrain` was why.  It is green now (since
  2026-07-26), so it enforces rather than accuses.  Arc B gives it something to check.
- **@PLN78** (loft distribution / self-update) — a self-updating binary needs a stated
  compatibility promise before it can be trusted to update under a user's project.
- **[Goal B](../../GOALS.md)** (release & legibility) and **[Goal F](../../GOALS.md)**
  (warnings are the only channel that may bill the programmer).

## See also

- [GOALS.md](../../GOALS.md) — the AS/400 aspiration this plan defends; the two floors.
- [STABILITY_ROADMAP.md](../../STABILITY_ROADMAP.md) — the wide-release bar; gate 5 is this
  plan, gate 1 is the Cluster C fold that precedes it.
- [DESIGN_DECISIONS.md § @C86](../../DESIGN_DECISIONS.md) — the plain-bind copy decision that
  `hex_terrain` did not survive.
- [PACKAGES.md](../../PACKAGES.md) / [PKG_REGISTRY.md](../../PKG_REGISTRY.md) — the manifest
  and registry that arc B changes.
- [LIBRARY_CHECKLIST.md](../../LIBRARY_CHECKLIST.md) — the per-library quality bar.
- `src/manifest.rs:513` (`check_version`) · `src/parser/mod.rs:7593` (the call site) — arc B's
  two touch points.
- Tracking issue: [@PLN102](https://github.com/loft-lang/plans/issues/102).
