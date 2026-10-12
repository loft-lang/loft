<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 7 — Cleanup and documentation

Status: **done 2026-07-07** — see § Delivered.

## Delivered (2026-07-07)

- **COMPILER.md § Diagnostics** rewritten as the three-layer model
  (positions/spans → @C66 runtime errors → renderers), matching the real
  code (`DiagEntry`, `Value::Span`/`source_spans`, `runtime_error::
  RuntimeError`, `diagnostic_render::render_pretty_all`, `LOFT_ERRORS`).
- **CHANGELOG.md** user-facing entry under `2026-07` (carets, suggestions,
  concrete type-mismatch, match-pattern check; runtime faults keep @C66
  never-abort).  **CHANGELOG_TECHNICAL.md** per-phase closeout entry.
- **CLAUDE.md** `LOFT_LOG` quick reference gained `LOFT_ERRORS` + the real
  diagnostic toggles (`LOFT_NO_WARN_RUNTIME`, `LOFT_NO_HINT_NOT_NULL`,
  `LOFT_FORMAT_BARE_NULL`, `LOFT_DEV_SOFT_HALT`).  `LOFT_BT` from the 7.4
  table was **never shipped** — dropped.
- **CAVEATS.md** — new "native build has no loft-source position on runtime
  faults" entry (the pc→source map is interpreter-only).

Adjusted from the original spec (stale assumptions):
- **7.6/7.9 dir move retired.** The current convention keeps a finished
  plan dir in place as its own closure record (`_LIFECYCLE.md`); the legacy
  `finished/NN-…` move is not done.
- **7.7 dead-code sweep — nothing to remove.** `Diagnostics::add` (no-pos)
  has 3 legitimate callers (unknown-file `Fatal`, two `data.rs` invariant
  sites); no retired `panic=error` helpers remain after the @C66 reframe.
- **7.8 smoke script — not added.** The `error_messages` golden corpus +
  `baselines_are_locked_in` already assert caret + source-line on every
  case in CI; a separate `grep` script would duplicate that.

## Goal

Phases 1-6 land the machinery and the visible improvements.  Phase 7
ties off the loose ends so a future contributor (or future Claude)
inherits a coherent design instead of a sprawl of TODOs:

- `doc/claude/COMPILER.md` gains a Diagnostics section.
- `doc/claude/PROBLEMS.md` retires resolved entries.
- `doc/claude/CAVEATS.md` retires resolved caveats.
- `doc/claude/CLAUDE.md` `LOFT_LOG` table mentions source-line
  resolution.
- `CHANGELOG.md` (user-facing) and `CHANGELOG_TECHNICAL.md` get
  proper entries.
- Stale tests and dead helpers from earlier "panic = error"
  patterns are deleted.

## Steps

### 7a — `COMPILER.md` § Diagnostics

New top-level section in `doc/claude/COMPILER.md`:

```
## Diagnostics

Loft errors flow through three layers:

1. **Spans** — every fault-prone IR node carries a `Position`
   (file/line/col).  See `Value::Span` in src/data.rs.  Stored at
   codegen time in `Definition.source_spans` keyed by bytecode pc.
2. **Diagnostic types** — `DiagEntry { level, message, file, line,
   col }` for compile-time; `RuntimeError { kind, position, op_pc,
   backtrace }` for runtime.  Compile-time diagnostics flow through
   `Diagnostics`; runtime errors surface from `State::execute`.
3. **Renderers** — `to_string_compact` for the test harness;
   `render_pretty` for users.  Switch via `LOFT_ERRORS=compact|pretty`.

User-facing rule: every error knows its source position.  Anything
that panics in the runtime is an interpreter bug.
```

Plus per-section diagrams cribbed from each phase's design doc.

### 7b — Retire PROBLEMS.md / CAVEATS.md entries

Phase 0's site survey recorded which entries were "error message is
useless" or "div-by-zero silently propagates".  After phase 4 + 6,
each is closed:

- Each closed entry moves under PROBLEMS.md's "Resolved" or
  CAVEATS.md's "Closed" subsection with the closing commit's hash.
- New caveats discovered during phase 4-6 (e.g. the `--native` path
  doesn't get phase-3's pc→source map) get a new CAVEATS.md entry
  with a forward reference to NATIVE_DEBUG.md.

### 7c — `CLAUDE.md` updates

Two small edits:

1. The `LOFT_LOG` quick reference table gets a row noting that
   `crash_tail:N` now prefixes each line with the source location:
   ```
   game.loft:88:14 [pc 0x12ab OpDivInt] in fn n_run_battle
   ```
2. The "Debug logging" section gets a one-line addition: "see
   `LOFT_ERRORS=pretty|compact` for the user-facing renderer".

### 7d — `CHANGELOG.md` (user-facing)

Single entry under the next version:

```
- Errors now show `file:line:col` with the offending source line and
  a caret.  Set `LOFT_ERRORS=compact` for the old single-line format.
- Runtime faults (divide by zero, index out of bounds, null
  dereference, narrowing-cast overflow) report a typed error with a
  source position and a stack trace, instead of crashing or
  silently producing a null sentinel.
- Many "name not found" errors now suggest a near-match.
```

### 7e — `CHANGELOG_TECHNICAL.md`

Per-phase entries with commit hashes, opcode changes (none in this
plan), and the bench numbers.

### 7f — Dead-code sweep

After phase 4 some helpers become unused:

- The `i64::MIN`-as-null sentinel propagation in
  `op_div_int_nullable` etc. — kept (still used by `??`).
- Helpers that printed bare `panic!` messages from `fill.rs` —
  retired; the `RuntimeError` path replaces them.
- `Diagnostics::add` (no-position variant, `src/diagnostics.rs:82`)
  — audit for remaining callers; phase 1 should have moved them all
  to `add_at`.  If any remain in phase 7, they're either compile-time
  invariant violations (legitimate) or missed sites (fix).

`cargo +nightly udeps` (or a manual sweep) confirms no dead helpers
remain.

### 7g — Final acceptance

Run end-to-end:

1. Pick 5 cases from `tests/error_messages/cases/` at random.
2. Run them with `cargo run --bin loft -- <case>.loft 2>&1`.
3. Verify each output:
   - Names the file, line, and column.
   - Shows the source line with a caret.
   - Has a clear, concrete message.
   - Where applicable, suggests a fix (`note: did you mean …?` or
     `note: integer division by zero produces …`).

If any case fails the bar, return to the appropriate earlier phase
and fix.

## Atomic landing sequence

| # | Step | Test |
|---|---|---|
| 7.1 | Add `COMPILER.md § Diagnostics` (spans, types, renderers — pulled from each phase doc) | `tests/doc_hygiene.rs` green (no broken links, no orphan headings) |
| 7.2 | Move resolved entries in `PROBLEMS.md` to "Resolved" with closing commit hash | `doc_hygiene` green |
| 7.3 | Move resolved entries in `CAVEATS.md` to "Closed"; add new "native codegen has no source map" caveat with forward ref to NATIVE_DEBUG.md | `doc_hygiene` green |
| 7.4 | Update `CLAUDE.md` `LOFT_LOG` table row (crash_tail prefixes loc) and add `LOFT_ERRORS` mention | `doc_hygiene` green; manual review of CLAUDE.md table |
| 7.5 | Add `CHANGELOG.md` user-facing entry | `doc_hygiene` green |
| 7.6 | Add per-phase `CHANGELOG_TECHNICAL.md` entries with commit hashes and bench numbers | `doc_hygiene` green |
| 7.7 | Dead-helper sweep (audit `Diagnostics::add` no-position callers; remove retired panic helpers) | `cargo build --release` zero warnings; clippy green |
| 7.8 | 5-case end-to-end smoke script `scripts/smoke_error_messages.sh` runs 5 random cases and `grep`s for caret + source line | New CI step runs the script; non-zero exit fails the build |
| 7.9 | Move `doc/claude/plans/07-error-messages/` → `doc/claude/plans/finished/07-error-messages/`; update plans README index | `find doc/claude/plans -name '07-error-messages' -path '*/finished/*'` returns one hit |

## Acceptance

- COMPILER.md § Diagnostics exists and is accurate.
- PROBLEMS.md / CAVEATS.md / CLAUDE.md updated.
- CHANGELOG.md and CHANGELOG_TECHNICAL.md entries written.
- Dead helpers removed.
- 5-case end-to-end smoke test passes.
- `make ci` green.
- Plan moved to `doc/claude/plans/finished/07-error-messages/`.

## Risks

| Risk | Mitigation |
|---|---|
| Doc updates drift from code (writer reads the spec, not the source) | Phase 7 is the last phase; the writer reads the actual code (which is now stable) and the design docs as a cross-check. |
| `LOFT_ERRORS=compact` is needed by an undocumented test that breaks under pretty default | The test harness sets `LOFT_ERRORS=compact` explicitly in `tests/common/`; user-facing default is pretty.  Any test that breaks indicates a missing harness setting and gets fixed. |
| Future contributor adds a new fault site without a `RuntimeError` kind | The fault-site list in 04-runtime-error-kinds.md is enumerable; phase 7's COMPILER.md § Diagnostics references it as the single source of truth.  A linter rule (clippy or a custom check) is overkill — the single source is enough. |
| Future contributor adds a new fault site without wiring the 4d/4e.1 swap tables and the 4e.2 warning message | Phase 4f's "defense-coupling rule" (per 2026-05-11 evaluation): each new fault kind ships its Nullable peer + adds itself to the swap tables + provides its 4e.2 warning message in the same commit.  Phase 7 adds the cross-check to the contributor checklist in `CONTRIBUTING.md` (or `CLAUDE.md § Adding a new fault site` if no CONTRIBUTING). |

## New env-var additions to document in 7.4

The @PLAN04 family ships several new env-var / CLI flag toggles
that 7.4 must add to `CLAUDE.md` and `LOFT_LOG`-adjacent tables:

| Flag | Effect | Phase |
|---|---|---|
| `LOFT_NO_WARN_RUNTIME=1` / `--no-warn-runtime` | Suppress the 4e.2 undefended-fault-site warning | 4e.2 |
| `LOFT_FORMAT_BARE_NULL=1` | Suppress the 4e.3 `(reason)` suffix in format-string output (bare `null`) | 4e.3 |
| `LOFT_NO_HINT_NOT_NULL=1` / `--no-hint-not-null` | Suppress the 4h `not null` field-reminder hint | 4h |
| `LOFT_BT=full` | Render the full backtrace under a runtime-error diagnostic (default: top 3 frames) | 4g.1 |
| `LOFT_DEV_SOFT_HALT=1` / `--dev-soft-halt` | Demote dev-mode raises to log-and-continue (matches production semantics) so a single run surfaces every fault site | 4g.3 |

The toggles cluster — 7.4 introduces a `## Diagnostic toggles`
section that lists them together with their default (always
ON for the warning/hint/distinct-token paths; OFF for soft-halt
and full-bt) and a one-liner for when to flip each.
