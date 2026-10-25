<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN201 — the ship tier: a game's release build drops the fault checks its checked runs proved it never needed

## Status

Active.  Step 1 (the measurement column) and step 2 (the ledger) are built; `--ship` is the
shipped build on every compiled target, every check kept (NATIVE.md § Optimisation tiers).
Steps 3–4 are designed below, not built.

## Goal

A game is developed against PRECOMPILED libraries and an interpreted or quickly compiled
routine under test, every fault check on: a null from an overflow, a zero divisor or a cast
out of range is answered and reported, because a value a developer cannot explain is the
thing the checks exist to prevent (@C129).  Most of the logic ends up in libraries.

The product that ships is a different binary.  It is compiled ONCE MORE from every source it
uses — each library recompiled inside the game's build, never linked as a published binary,
because the checks to drop are inside the library code and only a recompile can drop them —
and every fault check its own checked runs proved it never needed is gone.  Precompiled
binaries serve quick development only; a release build never uses one.

That is the tier `NATIVE.md § Optimisation tiers` names as not yet existing (owner,
2026-09-15): "a build, not a switch, and licensed by the testing's fault-free ledger", kept
open by @C120 and @C129.  A library published to the registry always keeps its checks.

## What the tier may drop, and what it never drops

| dropped where the ledger licenses it | always kept |
|---|---|
| integer overflow checks on `+ - *`, negation, `<<` | a store's bounds and lifetimes (memory safety) |
| the null tests the CHECKS add around those operators | a null the program writes or tests itself (`??`, `?`, `== null`): null semantics, not fault protection |
| the zero-divisor raise of a division whose divisor never was zero | a vector index out of range (it reads another record's memory) |
| a cast's out-of-range test | file and network errors (they come from outside the program) |

The measurement instrument, `LOFT_RELEASE_PASS_PROBE=1`, drops the left column everywhere at
once.  It is the CEILING of the tier: the speed the shipped binary reaches when every site is
licensed.

## Steps

1. **Measure the ceiling (built).**  `stats.py --lanes native,rust,ship` builds each program a
   third time under the probe, the libraries' loft code included and their Rust linked into
   the binary (`--static`, as step 3 builds it), and reports `ship_ratio`
   beside the checked ratio.  A row whose ship hash differs is marked, never failed: a fault
   wraps there.  `make perf-portal ARGS="--lanes native,rust,ship"` puts the column on the
   portal page.
2. **The ledger (built).**  A run with `LOFT_LEDGER=<file>` is the ordinary checked run on
   either backend, with every fault site instrumented: `compile::close_program` first runs
   `opt::ledger_sites`, which rewrites each site `OpAddInt(a, b)` into the builtin
   `__ledger(site, kind, a, b)` (`src/ledger.rs`).  That call answers what the operator answers
   — a zero divisor reports as it does — and counts the site as run, and as FAULTED when the
   processor's answer would differ: an overflow, a null operand the processor would not carry,
   a zero divisor.  The sites are the probe's: integer `+ - * / %`, negation, `& | ^`, a right
   shift by a constant.  A site is named by the function's definition name, a hash of its body
   (call targets by name, positions left out: a function added elsewhere moves no hash, an
   edit to the body moves its own) and the operator's pre-order ordinal, named before any
   whole-program rewrite, so a body `leaf_inline` copies keeps its callee's names and both
   backends write the same file.  The counts merge at exit (an `atexit` handler, so `exit()`
   and a failing run are recorded) into a sorted text file, one site per line, under a lock
   file beside it: runs add up, on either backend, from any number of processes.  A ledger
   run neither reads nor writes the program cache; `--ship` is never one.  Not recorded yet,
   so such a site keeps its check: casts, float comparisons, and code that runs precompiled
   (the compiled stdlib, a library's cdylib).  Guards: `tests/ledger.rs`.
3. **The ship build.**  `loft --ship <ledger>…` compiles the program and every library
   from source, and links the libraries' Rust STATICALLY: a library loaded as a shared object
   reaches its thread-locals through `__tls_get_addr` and is called through a bound slot,
   which cost web `pack_u8` 43 % and random `rand` 23 % against the static link.  At each fault site: the function's body hash matches the ledger, the
   function ran, and the site never faulted — the processor's operator; otherwise the checked
   form.  A function the ledger does not know keeps every check.  The probe's emission forms
   are reused, chosen per site instead of globally.
4. **Guards.**  A site that faulted keeps its check; a function edited since the ledger keeps
   all of its checks; on the ledger's own runs the ship binary answers what the checked
   build answers; a ledger of another program is refused with its reason.

## Open design question — how much coverage licenses a site

The ledger counts each SITE's own runs, so step 3 can apply either rule.  The cheap form
licenses a site when its FUNCTION ran and the site never faulted, which also licenses a site
in a branch the function never took.  The strict form licenses a site only when the site
itself ran and never faulted: a site in an untaken branch keeps its check, and the tier is
exactly as safe as the testing's coverage.  Recommended: the strict form, now that its
counts cost nothing extra.  The choice is the owner's.

## Cross-arc dependencies

@PLN200 builds a second code generator (ANSI C).  The tier is defined on the IR, so it holds
for both: a fault site is named by the function, its body hash and the operator's position,
never by the emitted text.  One ledger then licenses a ship build on either backend.  The C
generator implements the same two tiers as the Rust one: the CHECKED tier, where every fault
answers null exactly as the interpreter does, and the SHIP tier, where a check the ledger
licenses is dropped.  Only the text each generator emits for an operator differs.  The C backend validates the
generator only: it gets no benchmark lanes and no twins of the bench routines (owner), so
the ship tier is measured on the Rust backend alone.

## See also

[NATIVE.md § Optimisation tiers](../../NATIVE.md#optimisation-tiers--semantics-runs-performance-lanes-shipped-binaries) ·
[DESIGN_DECISIONS_VALUES.md @C120, @C129](../../DESIGN_DECISIONS_VALUES.md) ·
[NATIVE_SWITCHES.md `LOFT_RELEASE_PASS_PROBE`](../../NATIVE_SWITCHES.md) ·
[bench/README.md](../../../../bench/README.md) ·
[loft-lang/plans#201](https://github.com/loft-lang/plans/issues/201)
