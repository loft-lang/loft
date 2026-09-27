<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 173 — Code shape: functions that fit, files that hold one subject

## Status — DONE 2026-09-26

Every phase shipped.  `clippy::too_many_lines` fires on every function in `src/`, each
function over the bar carries its own `#[expect]`, and the release beat splits the worst
file each cycle.  The burn-down itself is not this plan: it is the `M-file-split` row,
done once per release.

The rules live where they are used: [CODE.md § Functions](../CODE.md#functions)
(function length, the `expect` and its reason, `inherited`), DEVELOPMENT.md's
*Validation Against CODE.md* table, [RELEASE.md § File split per
release](../RELEASE.md) (the beat, the pick, the payoff check), and the `split-file`
skill (where the seams are).

## Goal

A function a reader can hold, and a file that holds one subject.  Functions are held to the
bar on every PR; files are brought down on the release beat, never per PR.

## What shipped

| Rule | Delivered |
|---|---|
| **C1** | `clippy::too_many_lines` left the crate-root `#![allow]` of `src/lib.rs` and `src/main.rs`; the generated `src/ir_schema_gen.rs` exempts its one long function instead of the file (`tools/ir_schema/extract.py` emits it) |
| **C2** | every function over the bar carries `#[expect(clippy::too_many_lines, reason = …)]`; 252 new, measured over five clippy legs (CI's three, debug assertions on, the browser wasm rlib); the 58 existing per-function `#[allow]`s became `#[expect]`, and the 18 of them that no longer fired were removed |
| **C3** | `reason = "inherited"` marks the debt; six functions carry a real reason.  `make clippy-review ARGS=--lengths` counts them without a clippy run (at close: 293, of which 287 inherited) |
| **F1** | `M-file-split`, a gate row on the release checklist; RELEASE.md § File split per release |
| **F2** | the `split-file` skill: split or carve, a pure-move PR, its own move-share check |
| **F3** | `scripts/file-sizes.py`: `impl` and inline `mod` blocks are transparent (a `#[cfg(test)]` module stays one section); `--pick N` ranks by split value — the file minus its largest item — and prints a first grouping by name stem.  `parser/control.rs` now reads SPLIT (was KEEP at 97 % `impl Parser`); the pick on the closing tree is `scopes.rs`, `parser/mod.rs`, `parser/control.rs` |

Falsified during the build: removing the crate allow made 252 functions fire as warnings
(errors under `-D warnings`); the 18 `expect`s on functions that had shrunk were reported
as `unfulfilled_lint_expectations`, which `-D warnings` also fails.

## Out of scope

- `too_many_arguments` and `struct_excessive_bools`: a different rule.
- Docs: their ceiling and burn-down are @PLN172.

## See also

- [`loft-lang/plans#173`](https://github.com/loft-lang/plans/issues/173) — `@PLN173`, the
  issue this plan is.
