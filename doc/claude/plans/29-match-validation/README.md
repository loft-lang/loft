<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN29 — Match expression validation

**Status: phase 01 closed (PR #207).  Phase 02 — @P209 (match guards
saw pattern bindings as uninitialised) closed 2026-05-04 in
`src/parser/control.rs::parse_scalar_match`.  Range patterns work.
Phase 02 wiring (matrix tests for range / guard / null patterns)
not yet started.**

Closed (2026-05-04):
- **Hang fix**: `expect_match_arm_arrow` recovers via
  `lexer.recover_to(&[",", "}", ";"])` after a missing `=>` so a
  malformed pattern (e.g. `x @ 1 | x @ 2 => …`) no longer spins
  the surrounding scalar/tuple/enum match loop indefinitely.
  Pinned by `plan18_at_binding_in_or_pattern_does_not_hang` in
  `tests/parse_errors.rs`.
  Earlier pre-flight observation that `1 | 2 | 3 => …` itself
  hung was incorrect — re-running shows that simple or-patterns
  (no `@`-binding) parse and execute correctly.  The actual hang
  trigger was specifically `x @ N | x @ M …` where
  `parse_match_pattern` doesn't recognise `name @ pattern` in the
  or-loop and silently fails, leaving the lexer parked.

Open: deciding whether to support `x @ pattern` *inside*
or-patterns (parser feature) or document it as an explicit
non-goal — same shape as @PLAN17 phase 03 (`Printable`
satisfaction add-vs-retract decision).  Phase 02+ of this plan
covers other cells but the hang fix removes the only known
DoS-class issue.

## Goal

Validate that `match` expressions dispatch correctly across every
**subject type** through every **pattern shape**, with
**interp/native byte-identical stdout** asserted by the cross-mode
harness from @PLAN14 phase 00.

## Why now — pre-flight survey

6 quick tests (2026-05-04), 2 hangs.  Both hangs are in the
**scalar-match or-pattern path**:

| Shape | Result |
|---|---|
| Scalar match with range pattern (`0..3 => "lo"`) | ✅ |
| Char match with range (`'a'..='m' => "first"`) | ✅ |
| Text match (literal arms) | ✅ |
| Enum match with field capture | ✅ |
| Scalar or-pattern (`1 \| 2 \| 3 => "small"`) | ❌ parser hangs (no output, OOM-class loop) |
| Or-pattern with `@` binding (`x @ 1 \| x @ 2 => …`) | ❌ parser hangs |

33% bug rate.  Both failures look related to P206-class issues
(parser loop on a token-mismatch path that doesn't advance) but
in different `parse_*_match` variants than the `=>`/`->` fix.
The repeated-`|`-then-arm shape is the trigger.

## The matrix

Two axes.

### Axis 1 — subject type

| ID | Subject type | Notes |
|---|---|---|
| S1 | **Scalar** — integer, character, float, boolean | `parse_scalar_match` |
| S2 | **Text** — `text` literals as patterns | `parse_scalar_match` (text branch) |
| S3 | **Plain enum** — `enum E { A, B, C }` | `parse_match` enum branch |
| S4 | **Struct enum** — `enum Shape { Circle { r }, Rect { w, h } }` | `parse_match` struct-enum branch |
| S5 | **Tuple** — covered by @PLAN14 phase 03 | Cross-reference |
| S6 | **Vector** — `match v { [first, ..] => … }` | `parse_vector_match` |

### Axis 2 — pattern shape

| ID | Pattern | Notes |
|---|---|---|
| P1 | Wildcard `_` | Total catch-all |
| P2 | Literal | Exact value |
| P3 | Binding (`name`) | Bare identifier captures |
| P4 | Range (`lo..hi`, `lo..=hi`) | Pre-flight ✅ on int + char |
| P5 | Or-pattern (`a \| b \| c`) | Pre-flight ❌ hangs |
| P6 | `@` binding (`x @ pattern`) | Pre-flight ❌ hangs in or-pattern context |
| P7 | Guard (`pattern if cond`) | Conditional arm |
| P8 | Null (`null`) | Nullable subject only |
| P9 | Nested | Tuple-in-struct, struct-in-tuple, etc. |

## Phase layout

| Phase | Subject rows | Pattern cols | Outcome |
|---|---|---|---|
| [00 — matrix freeze + harness wiring](00-matrix.md) | (table) | (table) | Frozen matrix; `tests/match_matrix.rs` binary; smoke test against a known-passing cell.  No production change. |
| 01 — fix the P5/P6 parser hangs | S1, S2 | P5, P6 | Active risk: pre-flight surfaced two hangs.  Likely a sibling of the P206 fix in `parse_scalar_match`'s or-pattern loop.  Fix + regression tests + un-ignore phase 01 cells. |
| 02 — basic dispatch coverage | S1–S4 | P1–P3 | Most should pass.  Cells exist mainly as a regression net. |
| 03 — range + guard + null | S1–S4 | P4, P7, P8 | Cross-mode equivalence on numeric ranges; guard short-circuit ordering; null subject handling. |
| 04 — nested + cross-cutting | S3, S4 | P9 | Nested struct-enum-in-struct, tuple-in-struct, struct-in-tuple combos. |
| 05 — vector match | S6 | P1–P9 | Vector-specific patterns (rest patterns, slice match if loft supports them). |
| 06 — freeze + doc | — | — | Update LOFT_CONTROL.md § Match where the matrix surfaces under-documented behaviour. |

## Pre-flight gate

If phase 01 surfaces only 1–2 P-issues across all 9 pattern × 6 subject
combinations (i.e. a low-yield matrix), close phases 02–05 as deferred
and leave the matrix table as documentation.  The 7-phase plan is the
high-yield path; the low-yield path is "shipped enough to verify, no
more cells needed."

## Acceptance for the whole plan

- Matrix in [00-matrix.md](00-matrix.md) fully populated.
- Every PASS cell has a `cross_mode!` test in `tests/match_matrix.rs`.
- Every CLOSED cell has a stable-diagnostic regression test.
- The two P206-class parser hangs from pre-flight are closed.

## Out of scope

- **Match exhaustiveness analysis** — already enforced; this matrix
  validates dispatch, not the analysis itself.
- **PEG-style match patterns** (MATCH_PEG.md, L3) — future work,
  not yet implemented.

## Cross-references

- [LOFT_CONTROL.md § Match](../../LOFT_CONTROL.md) — language reference.
- [TUPLES.md § T1.9](../../TUPLES.md) — tuple match specification.
- `src/parser/control.rs` — `parse_match`, `parse_scalar_match`,
  `parse_tuple_match`, `parse_vector_match`.
- [@PLAN14 phase 03](../finished/14-tuple-validation/03-closures.md) — tuple
  match cells (cross-reference).
