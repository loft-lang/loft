<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# @PLN102 arc E — the `0 → 1` contract flip: gate + drift-gates + procedure

> **Status: DESIGN + gates BUILT (2026-07-19).** The flip itself is a **one-line**
> change (`manifest::CONTRACT_VERSION = 0 → 1`); the risk is entirely in *earning* it.
> This doc is the earn-it checklist (what must be true), the **CI drift gates** that
> make an omitted contract bump LOUD (versioning-decision.md item 4) — **now built and
> inert** (Gate 1 + Gate 2 below) — and the flip procedure. Reference:
> [versioning-decision.md](versioning-decision.md) (the pivot),
> [../../COMPATIBILITY.md § Before the flip](../../COMPATIBILITY.md).

## What the flip is (and why it is a one-way door)

`contract 1` is the 1.0 compatibility baseline. Today the mechanism ships **inert**
at `CONTRACT_VERSION = 0` (nothing declares `contract`, nothing gates). Flipping to
`1` starts the **absolute never-break promise** (arc A): from that commit on, no
functioning program may break — language, errors, libs, and their persisted data.
A wart shipped at the flip is **permanent** (live-with-it or engineer-around); an
error we *failed* to add is unaddable (the error surface is one-directional). So the
flip is gated on a surface-by-surface audit that fixes every regret while contract 0
still allows it.

## The invariant (design-protocol step 1)

> **Nothing declaring `contract 1` may observe a behaviour change after the flip
> commit.** Corollary for the drift gates below: any edit that *would* silently
> change a frozen surface must either (a) be caught pre-flip by the audit, or (b)
> after the flip, force a *contract bump* — never merge silently. The gate's job is
> to convert "silent" into "loud" for every surface that can drift mechanically.

## The gate — preconditions, by surface

The flip may land only when every row is ✅. Most are already there; the table is the
single reconciled view (supersedes the scattered "open" markers).

| Surface | Precondition | State (2026-07-19) |
|---|---|---|
| **Semantics** | INCONSISTENCIES.md High (silent-wrong) all fixed | ✅ *"All fixed — see CHANGELOG.md"* |
| **Semantics** | INCONSISTENCIES.md Medium/Low resolved-as-design-point (documented + regression-guarded) | ✅ all in the resolved table |
| **Errors** | Pre-freeze error surface maximally strict (add every error we might want — one-way) | ✅ Tier 0/1 fixed; E2-A + E2-B shipped (`tuxedo-e1-diag-codes`); sentinel collisions accepted (@C85) |
| **Errors** | Diagnostic machine-identity exists (codes, so prose stays improvable) | ✅ E1 shipped (codes are additive post-flip, so full back-fill is NOT a gate) |
| **Syntax** | The last in-flight syntax plans settled (they *define* what contract 1 is) | 🔶 **effectively settled** (query 2026-07-19: all syntax plans `future`/additive) — pending 2 owner rulings (@PLN15, @PLN91); see "Syntax-settled" below |
| **Stdlib / libs** | lib-audit worklist resolved (H1–H9) | ✅ done or consciously accepted (H8 = @C99) |
| **Stdlib / libs** | The dedicated unhurried lib pass (owner phase 2 — stdlib + core libs, equally permanent) | 🔶 the second half of the E gate |
| **Formats** | Layout/persistence identity distinguishes every semantically-distinct layout | ✅ **F9 built 2026-07-19** — `τ` vs `τ?` distinguished ([layout-nullability-identity.md](layout-nullability-identity.md)); deep raw-store gate grandfathered |
| **Formats** | Format sub-language (interpolation) warts fixed | ✅ E2-A (unescaped `}`) shipped; format-brace is the last known one |
| **Mechanism** | Drift gates make an omitted bump loud (layout-hash ⇒ bump; golden-corpus ⇒ classify) | ✅ **built + inert** (Gate 1 + Gate 2 below); the coverage-survey residual **also built 2026-07-20** — the E1 **code-set** gate (`tests/e1_code_set.rs`, the 4th surface) + the Gate-2 **HIGH corpus rows** (22→47 lines) → [flip-gate-coverage-gaps.md](flip-gate-coverage-gaps.md) |
| **Test hygiene** | The `code!` harness asserts the diagnostics loft *actually* emits (no tolerated-warnings filter) | ✅ **built 2026-07-19** — filter deleted + meta-lock ([test-hygiene-warnings.md](test-hygiene-warnings.md)) |

**"Syntax-settled" is not a vibe — make it a query.** The gate is "no open plan with
`subject:syntax`/`subject:language` that changes a *frozen* surface". Operationalise:
`./scripts/idx` (or `gh issue list -R loft-lang/plans --label status:active`) filtered
to language/syntax subjects; each must be closed **or** its remaining work explicitly
classified *additive* (safe to land post-flip). This list — reviewed with the owner —
is the literal flip trigger, not a judgement call.

**Query result (2026-07-19).** No `status:active` plan touches syntax — the whole open
language set is `future`/`parked`. The syntax-touching plans, classified:

| Plan | Syntax | Class for the flip |
|---|---|---|
| **@PLN37** [libs] Language features (CLOSED) | shipped: `type` aliases (C55), `?? return` (C56), `parallel { }` (A15).  Unshipped: `@get`/`@post`/`@ws` route decorators (C57), iterator protocol (I13) | **Additive** — all new forms, shipped or not; safe post-flip either way |
| **@PLN24** `#c` C-ABI binding | a new `#c "<sym>"` definition annotation | **Additive** |
| **@PLN15** cross-branch references | `ref struct T` + serialisable `&T` field refs | **⚠ owner decision** — see below; additive IFF inferred, breaking IFF declared |
| **@PLN91** self-hosting epic | parser operator/precedence → bundles (internal); configurable default int width | Bundle restructure semantics-preserving; int-width is an **opt-in** SBC profile, not a default change |

**Two edges need an explicit owner ruling before the flip** (the rest classify additive):
1. **@PLN15 — declared vs inferred referenceability.** The provisional design makes
   `&T` on an *unmarked* type a COMPILE ERROR (referenceability via `ref struct T`). That
   is a one-directional tightening → if it's part of contract 1 it MUST land pre-flip.
   The plan itself flags this "toll" as provisional and prefers **inferred+reported**
   (auto-detect + warn), which is additive/post-flip-safe. Owner picks: declared
   (pre-flip blocker) vs inferred (not a blocker). Scope to pin: whether `ref struct`
   would collide with EXISTING `&` uses (`fn f(o: &S)`, `b: &integer = a`) or only gate
   the new field-level `&T` reference.
2. **@PLN91 — configurable default integer width.** Confirm it stays a per-build **SBC
   profile** (opt-in), never a change to the default width — else it is a silent
   semantics break that must precede the flip.

**Verdict:** the syntax surface is effectively settled — every syntax plan is additive
or deferred; the gate is MET once the owner rules on the two edges above.

## The CI drift gates (versioning-decision.md item 4) — BUILT + inert

After the flip, the promise is only as strong as our ability to *notice* a silent
break. Two surfaces drift mechanically and need an automatic "you changed a frozen
thing → bump `contract` or revert" gate. Both are **inert pre-flip** (at
`CONTRACT_VERSION = 0` a bump is a no-op), so they were **built and proven now** and
simply become load-bearing at the flip — the ideal safe-step shape. Shared enforcement:
`scripts/check_contract_goldens.sh` (a `--self-test` proves the decision table without
git) + the non-blocking `contract-goldens` CI job.

### Gate 1 — layout-hash-changed ⇒ contract must bump

The store/file layout hash (@PLN97) *is* the persistence contract. A change to it is a
silent data break (open Q1, answered: yes, it bumps `contract`).

**Steps 1–2 PRE-EXISTED** (@PLN97 Phase B): `tests/layout_golden.rs` already pins the
layout of a representative corpus against a committed golden (`tests/golden/layout/corpus.txt`)
+ a `LAYOUT_ALGO_HASH` constant, re-blessed with `LOFT_BLESS_LAYOUT=1`, AND a
`layout_coverage_audit` ratchet that fails if the corpus misses any user-writable
storage kind (the "no blind spots" guarantee this design's falsification asks for).
**Step 3 BUILT 2026-07-19** (inert). **Step 4** is procedural (the flip runbook).

| # | Step | Proof | State |
|---|---|---|---|
| 1 | Golden layout baseline over a coverage-audited corpus | a field reorder / stride change flips the golden + `LAYOUT_ALGO_HASH` (the #477 class) | ✅ pre-existing (`layout_golden.rs`) |
| 2 | Drift is loud at commit time (re-bless forced on an intentional change) | `layout_golden` fails red on any layout move | ✅ pre-existing |
| 3 | **Couple to the contract** — a layout change may land only WITH a `CONTRACT_VERSION` bump (else a silent persistence break). `LAYOUT_CONTRACT` pins the contract-at-bless (`layout_golden.rs`); the git-diff rule (`scripts/check_contract_goldens.sh`) fails a golden change with no bump. **Inert while `CONTRACT_VERSION == 0`.** | self-test proves the decision table (`--self-test`); both git detectors verified; `layout_contract_pin_is_consistent` guards `LAYOUT_CONTRACT <= CONTRACT_VERSION`; non-blocking CI job in `api-compat.yml` | ✅ **built 2026-07-19** |
| 4 | **Flip-day arm.** No code change — at the flip the gate arms itself from `CONTRACT_VERSION = 1`. Documented in the flip runbook below. | n/a (procedure) | ⬜ at flip |

**Refinement noted (flip-day sharpening):** step 3's git rule flags ANY `corpus.txt`
change (a real layout move *or* an additive corpus entry) — conservative/fail-closed,
which is correct pre-flip. Post-flip, distinguish additive corpus growth from a genuine
existing-type reshape (reuse api-surface commit-5's layout axis, which already separates
`superset` from `changed`) so a coverage addition doesn't demand a spurious bump.

### Gate 2 — golden-corpus-output-changed ⇒ classify + bump  ✅ BUILT 2026-07-19

A behavioural break need not touch layout — a changed *observable output* on a curated
corpus is the general silent-semantics detector (shared with arcs A/C).

**Built** (`tests/behavior_golden.rs` + `tests/golden/behavior/{corpus.loft,corpus.out}`):
a curated corpus prints one labelled VALUE per frozen semantic surface (@C80/@C85 null
model, casts, shifts, `??`, vectors + comprehensions, match, text — VALUES only, never
error PROSE, so an improvable wording change never trips it). The output is pinned to a
golden (re-bless `LOFT_BLESS_BEHAVIOR=1`), the drift is loud at commit, and a second
test asserts `--native` == `--interpret` (the master invariant). The
`CONTRACT_VERSION` coupling is the SAME gate as Gate 1 (`scripts/check_contract_goldens.sh`
now lists both goldens), inert while `CONTRACT_VERSION == 0`.

| # | Step | Proof | State |
|---|---|---|---|
| 1 | Behavioural corpus + golden (VALUES + error codes, NOT prose) | `behavior_golden_interpret` fails on any output change | ✅ built |
| 2 | Drift loud at commit; both-backend parity asserted | `behavior_parity_native_equals_interpret` fails on divergence | ✅ built |
| 3 | **Couple to the contract** — a behaviour-golden change with no `CONTRACT_VERSION` bump is a silent semantics break (shared `check_contract_goldens.sh`). Classification (additive / bugfix / silent-break) stays a human review on the re-bless diff. | gate self-test proves the decision; inert at contract 0 | ✅ built |
| 4 | **Flip-day arm.** As gate 1 — live at the flip. | n/a (procedure) | ⬜ at flip |

**Classification (flip-day sharpening):** step 3's git rule is conservative — ANY
behaviour-golden change demands a bump post-flip. The finer *additive / bugfix /
silent-break* split (a bugfix re-bless is allowed without a bump) stays a human call on
the re-bless diff until the corpus is large enough to warrant an automated classifier.

> **Why advisory-first for both:** a required gate that can false-positive on a
> cosmetic change would block every PR the moment it lands. Both ship **non-blocking**
> (the `contract-goldens` job in `api-compat.yml`, sibling to api-compat); watch them be
> correct on real merges, *then* promote to required — the same inert-first ladder arc C
> used. Both are entirely in-repo and touch no `main` behaviour until the flip arms them.

## The flip procedure (the actual small steps, at the freeze)

When the gate table is all ✅ and the syntax-settled query is empty (owner-reviewed):

1. **Pre-flip dry run.** Run both drift gates in *would-this-bump* mode against
   `main`; confirm clean (no un-blessed layout/behaviour drift sitting on `main`).
2. **The one-line change.** `manifest::CONTRACT_VERSION: 0 → 1`. Nothing else in this
   commit. (`ContractCheck`'s `Drifted`/reject/accept arms were built inert in
   B-semantic and become live purely from this constant.)
3. **Bless the baselines as "contract 1".** Set `LAYOUT_CONTRACT = 1` and treat
   `tests/golden/layout/corpus.txt` + `tests/golden/behavior/corpus.out` as the
   frozen-at-1 references; promote the `contract-goldens` job to a required check from
   this commit.
4. **Flip the docs of record together:** COMPATIBILITY.md § Before-the-flip → After;
   RELEASE.md gains the flip runbook + the "how to ship a contract-2 change" epoch
   path; versioning-decision.md item 4 → ✅.
5. **Regression: a `contract`-declaring library round-trips.** A fixture lib with
   `contract = 1` loads clean; a synthetic `contract = 2` lib is *rejected below* by a
   `CONTRACT_VERSION = 1` runtime (the reject-below arm proven live, not inert).
6. **Announce** (arc A promise begins) — CHANGELOG + the never-break statement at the
   public intake (arc D, already landed).

Each step is independently revertible up to step 2; steps 3–4 are the point of no
return and land together in the flip PR.

## Falsification — how the flip could still be wrong (design-protocol steps 3–4)

- **A frozen surface with no drift gate.** ANSWERED (2026-07-19 survey →
  [flip-gate-coverage-gaps.md](flip-gate-coverage-gaps.md) Finding 1): the FOURTH ungated
  surface is the **E1 diagnostic CODE SET** — the `code!` harness strips the tag and no
  golden pins the codes, so a rename/removal is silent. Mitigation: build a code-set
  golden (S). Two thin secondary spots (bare-`null`/char rendering, binary-file scalar
  encoding) noted there. Everything else surveyed is gated or non-contract (JSON parser
  gated, JSON writer tooling-only, CBOR nonexistent, introspect/show-ownership/gendoc
  dev tools).
- **The corpus subset is too small** → a silent break slips through a gap in coverage.
  ANSWERED (→ [flip-gate-coverage-gaps.md](flip-gate-coverage-gaps.md) Finding 2): the
  corpus is honest but THIN — it omits structs, struct-enum payloads, keyed collections,
  the full rendering surface, the text stdlib, and the null-comparison/propagation
  keystone. None are *untested* (the both-backend assert suite covers them) but they are
  absent from the one contract-versioned artifact. Mitigation: the HIGH worklist there
  (rendering first) lands before the flip; MED/LOW grow the corpus additively post-flip.
- **"Syntax-settled" judged, not queried** → a moving surface frozen by accident.
  Mitigation: the flip trigger is the label query above, reviewed with the owner, not
  a feeling that "things seem quiet".
- **A bugfix mis-classified as additive** post-flip → a real break waved through.
  Mitigation: gate 2 step 3's three classes are exhaustive and each requires a label;
  the *silent-break* class is the default when unsure (fail-closed).

## See also
- [flip-gate-coverage-gaps.md](flip-gate-coverage-gaps.md) — the fourth-surface + corpus-completeness survey (the two falsification answers + the pre-flip worklist).
- [versioning-decision.md](versioning-decision.md) — the pivot; item 4 = these gates.
- [../../COMPATIBILITY.md](../../COMPATIBILITY.md) — the never-break policy (arc A) + § Before the flip.
- [formal-audit.md](formal-audit.md) / [lib-audit.md](lib-audit.md) — the surface-by-surface audit worklists (the gate's evidence).
- [layout-nullability-identity.md](layout-nullability-identity.md) — F9, a Formats-row precondition.
- [test-hygiene-warnings.md](test-hygiene-warnings.md) — the test-hygiene precondition.
- `src/manifest.rs` (`CONTRACT_VERSION`, `ContractCheck`) — the one-line flip site + the inert arms it arms.
