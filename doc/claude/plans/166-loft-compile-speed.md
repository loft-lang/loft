<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 166 — loft's own compile speed: instrument it, attribute it, gate it

## Status — DONE 2026-09-26

Every sub-arc shipped.  loft's own compile is measured by phase, attributed by module, and
guarded by an exact allocation count; on the 12 826-line corpus it takes **48 % fewer
instructions** (5 984 M → 3 112 M) and **59 % fewer allocations** (medium corpus 2 551 492 →
1 047 203) than when the plan was filed, with every corpus dump byte-identical at each step.

The reference content lives in [PERFORMANCE.md](../PERFORMANCE.md): § *Front-end speed —
how it is measured and guarded* (the timing split, the bench, the count gate and why it
counts rather than times, the stdlib-cache reuse), § *F1* (every cut with its measurement,
and the leads left), § *N6* (the source-keyed native fast path).  The engine profile's
oracle row is [PROFILE_ORACLE.md § Engine](../PROFILE_ORACLE.md).  CODE.md § Hot-path
conventions carries the four rules the cuts turned into.

## Goal

Make loft's own compile/analysis speed **measurable by phase, attributable to a named
mechanism, and protected by a gate that cannot drift with the machine it runs on.**

## What shipped

| Item | Delivered |
|---|---|
| **A1** | `LOFT_TIMING` splits the front end: `parse_user`, `scopes`, `lints`, `front_end` |
| **A2** | `bench/frontend/frontend.py` — cold / warm / edit loop, interleaved, `--counts`, `--self-test` |
| **A3** | `profile.sh --engine` by module, and the engine profiler's first oracle row |
| **B1** | the edit loop reuses the stdlib cache, pinned by the manifest's `stdk` line (edit loop, tiny: 573 M → 88.5 M instructions) |
| **B2** | definition lookups borrow their key instead of allocating it |
| **B3** | a warm `--native` run execs its binary off the source key, no parse (113 M → 4.8 M instructions) |
| **B4** | `env_once!`, the Fx hasher, child links, in-place token compares (−35 %) |
| **B5** | `Position.file` is an `Arc<str>` (−12 %); loft#1685, a per-process release order, found and fixed |
| **B6** | the definition index keyed by name with an address memo, the bytecode tables on Fx, a split borrow in the scope pass (−5.8 %) |
| **B7** | `Lexer::peek` borrows, the parse-time lints read the body in place, operator tokens spelled on the stack, the retired fault-site walk skipped (−3.0 % on the joined tree) |
| **C1** | `tests/frontend_counts.rs`, the exact allocation ratchet; sabotage receipt: one `to_string()` in `Data::def_nr` → +639 819 on medium → FAIL |

## Cross-arc dependencies

- **@PLN52** (finished) — the whole-program cache whose warm path this plan measured.
- **@PLN82** (parked) — constant store; its Phase C is the in-browser stdlib re-parse.
- **@PLN158** — release performance pass; library runtime, not loft's own front end.

## See also

- [`loft-lang/plans#166`](https://github.com/loft-lang/plans/issues/166) — `@PLN166`, the
  issue this plan is.
