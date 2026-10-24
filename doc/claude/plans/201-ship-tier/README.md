<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN201 — the ship tier: a game's release build drops the fault checks its checked runs proved it never needed

## Status

Active.  Step 1 (the measurement column) is built: `bench/stats.py --lanes native,rust,ship`
and the portal's ship column.  Steps 2–4 are designed below, not built.

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
   third time under the probe, the libraries' loft code included, and reports `ship_ratio`
   beside the checked ratio.  A row whose ship hash differs is marked, never failed: a fault
   wraps there.  `make perf-portal ARGS="--lanes native,rust,ship"` puts the column on the
   portal page.
2. **The ledger.**  A checked run with `LOFT_LEDGER=<file>` records, per FAULT SITE, whether it
   faulted, and per function whether it ran.  A site is named by what survives a rebuild of
   the same source: the function, a hash of its body, the operator and its position in the
   body.  The interpreter and a native library record into the same file, so a game's
   development runs (interpreted routine, precompiled libraries) collect one ledger.  Many
   runs merge: a fault in any run marks the site.
3. **The ship build.**  `loft build --ship <ledger>…` compiles the program and every library
   from source.  At each fault site: the function's body hash matches the ledger, the
   function ran, and the site never faulted — the processor's operator; otherwise the checked
   form.  A function the ledger does not know keeps every check.  The probe's emission forms
   are reused, chosen per site instead of globally.
4. **Guards.**  A site that faulted keeps its check; a function edited since the ledger keeps
   all of its checks; on the ledger's own runs the ship binary answers what the checked
   build answers; a ledger of another program is refused with its reason.

## Open design question — how much coverage licenses a site

The recommended form above licenses a site when its FUNCTION ran and the site never faulted.
It is cheap: one flag per function in a ledger run, and nothing per operator.  It also
licenses a site in a branch the function never took.  The strict alternative counts each
site's own executions, which costs a counter per operator in ledger runs only.  A site in an
untaken branch then keeps its check, and the tier is exactly as safe as the testing's
coverage.  The choice is the owner's: the cheap form is assumed until it is decided.

## See also

[NATIVE.md § Optimisation tiers](../../NATIVE.md#optimisation-tiers--semantics-runs-performance-lanes-shipped-binaries) ·
[DESIGN_DECISIONS_VALUES.md @C120, @C129](../../DESIGN_DECISIONS_VALUES.md) ·
[NATIVE_SWITCHES.md `LOFT_RELEASE_PASS_PROBE`](../../NATIVE_SWITCHES.md) ·
[bench/README.md](../../../../bench/README.md) ·
[loft-lang/plans#201](https://github.com/loft-lang/plans/issues/201)
