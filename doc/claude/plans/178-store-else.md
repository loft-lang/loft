<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 178 — A store's failure arm: `place = value else { … }`

## Status

Open — design settled, no implementation.  Tracker:
[@PLN178](https://github.com/loft-lang/plans/issues/178).  The ruling is
[DESIGN_DECISIONS.md C130](../DESIGN_DECISIONS.md#c130--a-store-carries-its-own-failure-arm-else-after-the-assignment);
this file carries the phases and the matrix, not the rationale.

## Goal

An assignment statement may end in `else { … }`, and the block runs exactly when the store did
not take — an absent element or key, a null view at any step of the chain, a narrow slot the
value did not fit, a lock fault — with the place spelled once; a dropped write with no `else`
logs one Warn line, in every build kind; a store in condition position is refused.

## Effort + design

- **Effort:** M
- **Design:** ✓ (C130)
- **Last touched:** 2026-09-28

## What is already known

- **Today's behaviour is the rule's, and a re-read is not a status.**  `v[9] = 2; if !v[9]`
  prints "missed" on both backends only because the re-read answers the out-of-range null;
  `v: vector<integer?>; v[1] = null; if !v[1]` reports a landed write as missed (measured
  2026-09-28 on the release binary, both spellings in the C130 entry).
- **The status temp exists.**  `src/parser/fit.rs` mints `__fit_N` for a narrow store followed
  by `if !place` and conditions the `if` on it; `same_place` is the comparison that names the
  place, and the seven shapes its header lists as NOT fused (a call in the index, a tuple
  element, a keyed element, …) are the ones `else` must cover without a re-spelled place.
- **Both backends decide "absent" at one site each** — `State::vec_get_or_raise` and
  `Stores::vec_get_or_raise_runtime` — and every typed setter opens with the `rec == 0` test
  the `OpSet*` templates in `default/01_code.loft` carry.  The native rewrites that write an
  element WITHOUT a `DbRef` decide it on their own path: the hoisted header and base
  (`vec_set_at`, `vec_set_hoisted_or_raise_runtime`), the slice fill (`fill_hoisted`), the
  record-view address (`rec_set`) and the guarded index chain.  Each is a cell below.
- **`else` is read at one parser site** (`src/parser/control.rs`, after an `if`'s true block).
- **A store in condition position was accepted, and the backends disagreed on it** — the
  interpreter read a stack byte as the flag (`if x = 5 { … }` then printed a pointer-sized
  number for `x`), `--native` failed in rustc.  Refused at `Parser::convert_condition_at`
  on 2026-09-28 (phase 0, shipped with C130); the guard is
  `tests/scripts/a-store-in-condition-position-is-refused.loft`.
- **Two interpreter panics seen on the way were NOT the language's** — *Store access out of
  bounds … the reference is corrupt* on `if v[9] = 2` and on two write-and-test pairs over a
  `vector<integer?>`.  Both reproduced exactly when a binary built from one branch ran with
  the OTHER branch's `default/01_code.loft` checked out (one extra `fn Op…` declaration
  shifts every later entry of the positional `fill::OPERATORS` table), and never with a
  matching stdlib (0 of 60 runs, `LOFT_POISON=1` and `LOFT_STRICT_STORES=1` clean).  A
  stdlib that does not match its binary is accepted silently today; refusing it at load is
  a finding for the build-id family (`build.rs` already names the cross-checkout class),
  not this plan's.

## Composition matrix — Stage A

Every cell is a `/tmp` probe on `--interpret` first, hand-computed, asserting the value, the
length and the leak gate, then graduated to `tests/scripts/178-store-else.loft` and run on
both backends.  `python3 scripts/matrix_axes.py file <guard>` reports the axes reached.

| Axis | Values |
|---|---|
| index shape | literal in range, `len`, `len + 1`, `-1`, `-len`, `-len - 1`, integer null, a variable, a loop counter through negative values, a call in the index (`w[bump()]` — the read must run once) |
| element type | integer, `u8`, `u8?`, `integer?`, float, text, record, heap-owning record, struct-enum, vector |
| store spelling | plain, compound (`+=`), whole-record literal and variable, a field through the element, a nested field path, a view then a write, a tuple element |
| container | local vector, parameter, `&` alias, field, a call's result, LINKED vector, hash, sorted, index; a keyed miss at the first step and at a later step of a chain |
| right-hand side | literal, variable, a call, `a / b` yielding null, `if c { 1 } else { 2 }` (the inner `else` binds to the `if`), a `??` chain |
| the arm | none, `else {}`, `else { flag = true }`, `else { return }`, `else { continue }` in a loop |
| statement context | straight line, an `if` arm, a loop body, a hoisted loop, a fill loop, a guarded-chain grid, a coroutine body, a `par` body |
| fault kind | out of range, null index, absent key, null view, fit failure (non-nullable narrow), lock fault, and the LANDED cells for each (the arm must not run) |

## Sub-arcs

| # | Phase | Verify | E |
|---|---|---|---|
| 0 | **Refuse a store in condition position** — `if place = v` is a compile-time refusal naming the cure (`==`, or the store on its own line) | `tests/scripts/a-store-in-condition-position-is-refused.loft` (eight `@EXPECT_ERROR` cells: `if` / `else if` / `while` / `assert`, element in and out of range, local, field, compound), falsified against ef4fd49 | **Shipped 2026-09-28** |
| 1 | **Rules first** — `(H-Write-Else)` in `formal/heap.md` beside `(H-WriteOOB)`, the dropped-write clause on `(E-Report)`, `(E-Uncomp-Seen)` restated with `else` as its form; each cited by the site that will enforce it | `scripts/rule_tags.py check` and `registers`; the chapters' `OPEN:` counts re-measured | XS |
| 2 | **Interpreter: `else` after a store** — the parser reads a trailing `else` on an assignment statement, the setter answers whether it resolved a cell, the block is conditioned on that through the `__fit_N` temp; the fit fusion re-targets to the same path for `place op= e else` | the Stage A matrix green on `--interpret`; `loft introspect` byte-identical for every store WITHOUT an `else` (the codegen skill's refactor gate) | M |
| 3 | **Native: every setter path answers the status** — the `OpSet*` twins and each rewrite that writes without a `DbRef` (hoisted header and base, slice fill, record-view address, guarded chain) | the same script on `--native`; `LOFT_TRACE_BASE` / `LOFT_TRACE_RECPTR` / `LOFT_TRACE_CHAIN` and a `--native-emit` grep confirm each rewrite fired on its cell — a green cell a rewrite declined measures nothing | M |
| 4 | **The Warn line** — an unguarded dropped write logs once, naming the place, the index or key and the length; a store with an `else` logs nothing; mode-independent | a test with the logger attached asserts one line per unguarded dropped write and none per guarded; a loop dropping a thousand writes shows the logger's own suppression, not a thousand lines; `LOFT_FORMAT_BARE_NULL`-style deployments unaffected | S |
| 5 | **Docs and the manual** — LOFT.md's narrow-slot section moves to `else` (its sample is a program, @PLN176), `if !place` is documented as the accepted older form, CAVEATS/CONTROL rows if any, CHANGELOG | `make ci`'s doc gates (`features-check`, the @PLN176 sample runner); `make rule-coverage` unchanged or up | S |

## Phase ordering

0 → 1 → 2 → 3 → 4 → 5.  Phase 0 is independent and may land first on its own; phases 2 and
3 are one comparison apart (the same script on the other backend) and must not be merged into
one phase, because a rewrite that declined on native leaves a cell green for the wrong reason.

## Open design questions

1. **The lock fault** — C130 lists `(H-WriteLocked)` as "did not take".  Where the write is
   provable at compile time it is a static reject today; the `else` is only ever the runtime
   half.  Confirm no site turns a static reject into a runtime arm.
2. **The log line's name** — `write_dropped`, alongside `divide_by_zero`; it is a log line,
   not a diagnostic code, so DIAGNOSTICS.md gains no row.
3. **wasm / `--html`** — the parser is shared, so the interpreter path covers them; the
   library cdylib path is phase 3.  Verify on the loft-ship matrix once, in phase 5.
4. **A coroutine or `par` body** — the status temp is a frame local; confirm the temp lands
   in the right frame under both.

## Cross-arc dependencies

- @PLN152 step 5 shipped `(E-Uncomp-Seen)` and `fit.rs`; this plan re-targets that machinery
  and must keep its guards green.
- @PLN176: the LOFT.md sample is a program, so the manual's `else` example is a test.
- @PLN157's lean tier: phase 3 must not add a check to a store without an `else`, or the
  perf-portal rows move.

## See also

- [DESIGN_DECISIONS.md C130](../DESIGN_DECISIONS.md#c130--a-store-carries-its-own-failure-arm-else-after-the-assignment) — the ruling.
- [DESIGN_DECISIONS.md C80](../DESIGN_DECISIONS.md#c80--the-spreadsheet-fault-model-nothing-stops-a-running-calculation) — nothing stops a running calculation.
- [formal/heap.md](../formal/heap.md) `(H-WriteOOB)` / `(H-WriteNull)`; [formal/operational.md](../formal/operational.md) `(E-Uncomp-Seen)` / `(E-Report)`.
- `tests/scripts/a-write-to-an-absent-element-lands-nowhere.loft` — the standing guard for the write that lands nowhere; this plan's matrix extends its axes with the arm.
- [LOGGER.md](../LOGGER.md) — the level configuration that is the project-wide switch.
