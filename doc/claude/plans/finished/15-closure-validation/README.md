<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN15 — Closure validation: capture × storage matrix

**Status: SHIPPED 2026-05-12 — plan closed.**
All 6 phases (00 + 01 + 02 + 03 + 04 + 05 + 06) landed in one
session.  Reference home: `tests/closure_matrix.rs` (22 cells)
plus `tests/leak.rs::p15_phase0[345]_*_no_leak` (5 leak guards).

**Per-phase summary**:

| Phase | Coverage | Cells / guards | Outcome |
|---|---|---|---|
| 00 | Harness wiring + smoke | 3 cells | No production change. |
| 01 | C0 (non-capturing) × D1/D2/D3/D4 | 5 cells | Pins @P214 closure (vector-of-non-capturing). |
| 02 | C1 + C5 (basic-type captures) × D1/D2/D3 | 6 cells | Pins @P213's `Parts::ChildRec` layout-widening + @P215 nested-name resolution. |
| 03 | C2 (text capture) × D1/D2/D3 | 3 cells + 2 leak guards | Disposed LIFETIME.md "NOT YET HANDLED" claim — no leak; documentation drift, not a runtime bug. |
| 04 | C3 (Reference capture) × D1/D2/D3 | 3 cells + 2 leak guards | No DbRef-in-closure-record leak; no read-after-free. |
| 05 | C6 (nested closures) × D1/D2/D3 | 3 cells + 1 leak guard | D3 included (matrix's "deferred" was conservative); nested non-capturing inner pattern works on both backends. |
| 06 | Doc closeout | — | LIFETIME.md "Implementation path" trimmed; ROADMAP.md / USER_FACING.md / 6-audience-generative-art cross-refs updated; plan moved to `finished/`. |

**Bug yield**: 0 new P-issues filed.  All gaps the plan was
designed to surface (closure-DbRef leak, move-vs-copy semantics
gap analogous to T1.8c) turned out to be non-issues — the
underlying support landed earlier through @P213/P214/P215/P227 and
@PLAN15 confirmed it via systematic regression coverage.

**Out of scope** (deliberately; tracked in matrix + loft-write skill):

- C7 (vector capture) — closing as non-goal per the matrix.
- C1+/D4 (vector of CAPTURING closures) — known restriction;
  failure mode is unstable (interp panic + native E0308).  No
  CLOSED-cell parse_errors test added because there's no clean
  parse-time diagnostic to pin.  When the language adds first-
  class generic fn-refs (or a clean parse-time rejection), the
  CLOSED cells graduate to FIX cells in a follow-up.
- D5 (tuple element) — covered by `plans/finished/14-tuple-validation`.

**Reference home for matrix details**: see [`00-matrix.md`](00-matrix.md).

## Goal

Validate that closures (`Type::Function(args, ret, dep)`) round-trip
correctly through every meaningful **capture composition** and every
**storage destination**, with **interp/native byte-identical stdout**
asserted by the cross-mode harness already in place from @PLAN14
phase 00.

The driving question: "given a closure that captures shape C, stored
in destination D, called and observed — does the interpreter and the
native build agree on the read-back value, byte for byte?"  A green
matrix means: every cell has a test that runs in interp and native,
and the cross-comparison passes.

This plan inherits all infrastructure from @PLAN14: the `cross_mode!`
macro, the `tests/common/cross_mode.rs` harness, the `#[ignore]`
discipline, and the P-id filing rules.  The only new artefacts are
the closure-specific matrix, phase plans, and cell tests in a new
`tests/closure_matrix.rs` binary.

## Why now

Closures shipped end-to-end in 0.8.3 but the test surface has grown
ad-hoc — there's no systematic check that, for example, "a closure
that captures a `text` field of a struct, stored in a vector, called
through `map`, then released" works the same under interp and native.
That permutation was never written down as a cell.

Two known gaps point at concrete bug-yield potential:

1. **LIFETIME.md flagged a closure-DbRef leak** (the "Function
   (`Type::Function`) — NOT YET HANDLED" annotation).  Phase 03
   investigated this with capture-of-text + storage-in-struct-field
   cells under `tests/leak.rs`-style 100-iteration assertions and
   confirmed the leak does NOT manifest — closure records free at
   scope exit via standard local-cleanup (D1) or `Parts::ChildRec`
   cascade (D3).  Disposed as documentation drift; LIFETIME.md
   updated 2026-05-12.  No P-issue filed.
2. The loft-write skill records: *"Capturing closures in
   `vector<fn(...)>` is supported only for non-capturing lambdas or
   when all elements are the same closure type."*  That's a real
   restriction that the matrix pins (cells either pass or match
   the exact diagnostic).

Per the project's bug-hunt policy (memory: `feedback_proactive_bug_hunting`), 
extending the matrix is the way to find compiler bugs we don't know
about yet.  Plan-14 phase 01 found 2 P-issues in 15 cells; @PLAN15
likely surfaces a comparable rate.

## The matrix

Two axes.  Every cell is `PASS:test_name`, `FIX:phase`, or
`CLOSED:reason` with a DESIGN_DECISIONS.md cross-reference.

### Axis 1 — capture composition

| ID | Capture shape | Notes |
|---|---|---|
| C0 | **Non-capturing** — `\|x\| { x + 1 }` | Closure record is empty / null DbRef.  Baseline. |
| C1 | **Single basic-type capture** — `let n = 5; \|x\| { x + n }` | One scalar in the closure record. |
| C2 | **Single text capture** — `let s = "tag"; \|x\| { "{s}: {x}" }` | Owned text element; lifetime risk for closure-leak path. |
| C3 | **Single Reference capture** — `let p = make_p(); \|dx\| { p.x + dx }` | DbRef element; dep tracking. |
| C4 | **Single tuple capture** — `let t = (3, 7); \|x\| { t.0 + x }` | Cross-references @PLAN14 phase 03 (E4 closure-element). |
| C5 | **Multiple captures (mixed)** — `let n = 5; let s = "k"; \|x\| { … }` | Two captures, disjoint types. |
| C6 | **Nested closure capture** — `let inner = \|x\| { x*2 }; \|y\| { inner(y)+1 }` | Captures another closure. |
| C7 | **Vector capture** | Out of scope — known restriction; keep parser error stable via a CLOSED cell with the exact diagnostic. |

### Axis 2 — storage destination

| ID | Destination | Today |
|---|---|---|
| D1 | **Local variable** — `f = \|x\| { … }; f(3)` | Primary tested path |
| D2 | **Direct stack** — function arg (`f(my_lambda)`), return value, inline `(\|x\| {…})(3)` | Partial coverage |
| D3 | **Struct field** — `struct S { cb: fn(integer) -> integer }`, instantiate, call `s.cb(arg)` | LIFETIME.md says struct-field closures work but the closure-leak gap may bite here |
| D4 | **Vector element** — `vector<fn(integer) -> integer>` | Restricted: non-capturing lambdas only, OR all elements the same closure type.  Cells split into PASS (non-capturing, monomorphic) and CLOSED (capturing-heterogeneous → exact diagnostic).  Verified failure mode: shorthand `\|x\|` triggers "No common type function([unknown(0)], void, [])"; explicit `fn(x: T) { … }` form panics in interp ("Write to locked store") and rejects in native (`(u32, DbRef)` vs `i64`).  Pinned by `par_vec_of_capturing_fns_t4` ignored canary in `tests/threading_chars.rs`. |
| D5 | **Tuple element** | Covered by @PLAN14 phase 03 — cross-reference, no new cells here |

### Cell key

Each cell `(C, D)` has one of:
- **PASS** — covered by an existing or new test.  Test reference recorded.
- **FIX** — implement + add test in the matching phase below.
- **CLOSED** — documented as design rejection.  Cell test asserts
  the parser error stays exactly as today.

## Phase layout

| Phase | Capture rows | Destination cols | Outcome |
|---|---|---|---|
| [00 — matrix freeze + harness wiring](00-matrix.md) | (table) | (table) | Frozen matrix; new `tests/closure_matrix.rs` binary; reuses `cross_mode!`.  No production change. |
| 01 — non-capturing baselines (C0) | C0 | D1, D2, D3, D4 | All non-capturing-closure cells green.  Establishes the harness shape against the simplest case. |
| 02 — basic-type captures (C1, C5) | C1, C5 (multi-mixed) | D1, D2, D3 | Single-scalar and multi-scalar captures.  D4 stays CLOSED for capturing closures (matches the loft-write skill restriction). |
| 03 — text captures (C2) | C2 | D1, D2, D3 | The active risk — closure-leak gap (LIFETIME.md).  Phase 03 either confirms the leak via `tests/leak.rs` and files a P-issue, or surfaces that the leak is benign for these cells.  Decision recorded in TUPLES-style "Decision" section. |
| 04 — Reference captures (C3) | C3 | D1, D2, D3 | DbRef-in-closure-record dep tracking; surfaces any move-vs-copy semantics gap analogous to @PLAN14 T1.8c. |
| 05 — nested closures (C6) | C6 | D1, D2 | Closure-capturing-closure; verifies that the captured closure's own dep list propagates. |
| 06 — freeze + doc | — | — | LIFETIME.md already updated (phase 03 closeout 2026-05-12).  Phase 06 trims the legacy "Implementation path" steps in LIFETIME.md (the 4-step plan that already shipped via @P213/P215/P227), updates PLANNING.md + CHANGELOG_TECHNICAL.md, moves plan to `finished/`. |

## Acceptance for the whole plan

- Matrix in [00-matrix.md](00-matrix.md) fully populated — no
  "unknown" cells.
- Every PASS cell has a `cross_mode!`-driven test in
  `tests/closure_matrix.rs` that runs green under
  `cargo test --release --test closure_matrix -- --ignored`.
- Cross-mode equivalence is mandatory (same contract as @PLAN14).
- Every CLOSED cell has a corresponding negative test in
  `tests/parse_errors.rs` (or similar) asserting the diagnostic
  stays stable.
- LIFETIME.md `Type::Function` row updated to reflect actual
  freed-or-not status after phase 03 lands.

## Out of scope

- C7 vector-of-capturing-closures.  Known restriction;
  documented in the loft-write skill.  Cell stays CLOSED.
- D5 tuple-element closures.  Covered by @PLAN14 phase 03.
- Closures returned across thread boundaries.  Plan-06 phase 4d
  owns that surface.
- Closures as iterator subjects (`for v in closure(...)`).
  Closure → iterator coercion isn't a language feature.

## Risks

| Risk | Mitigation |
|---|---|
| LIFETIME.md gap (closure DbRef not freed) is genuine and surfaces as a leak in phase 03 | Plan phase 03 has an explicit "Decision" section: confirm-and-fix or confirm-and-document.  Either way, ship a regression test that pins the chosen behaviour. |
| C6 (nested closure) reveals dep-list propagation bugs that cascade into phase 04 (Reference captures) | Phase 06 inserts a sub-phase that addresses the cross-cell pattern before the dependent phases ship.  This mirrors @PLAN14's "FIX phase 04 reference cells if T1.8c fix lands" pattern. |
| Vector-of-closure restriction (D4 + C1+) is fragile under future generic-fn-ref work | The CLOSED cells assert the exact diagnostic; if it shifts unexpectedly we catch it at test time, not at user-report time. |
| Plan-15 cells balloon test runtime past @PLAN14's already-heavy ~2-3 min matrix | All cells `#[ignore]`d under `tuple_matrix`-equivalent tag.  Run with `cargo test --release --test closure_matrix -- --ignored`.  Only run on demand or in a dedicated CI lane. |

## Cross-references

- [LIFETIME.md § Function](../../../LIFETIME.md) — the closure-leak gap
  this plan validates.
- [LOFT_LITERALS.md § Closures](../../../LOFT_LITERALS.md) — language reference.
- [loft-write skill § Higher-order functions](../../../../../.claude/skills/loft-write/SKILL.md)
  — capture semantics summary.
- [@PLAN14 phase 03](../14-tuple-validation/03-closures.md) — tuple
  cells with closure elements; closure-in-tuple is the D5 cell here.
- [@PLAN14 README](../14-tuple-validation/README.md) — same matrix
  template and cross-mode contract.
- `src/data.rs::Type::Function` — definition.
- `src/scopes.rs:578` — tuple/closure scope-exit gate (T1.8c
  neighbourhood).
