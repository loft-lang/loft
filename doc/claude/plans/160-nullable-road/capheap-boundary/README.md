<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The loop-capture leak behind `captured-local-rebind` — loft#1483

`captured-local-rebind`'s divergence has a one-line cure (`var_tp.base()` at
`src/parser/objects.rs:3715`, peeling the `?` so a nullable local takes its dense twin's in-place
road).  Applying it turns a shipped guard RED, and these cells are why.

Run each with `LOFT_STRICT_STORES=1` on both backends.  Values are correct everywhere; the channel
that moves is the leak.

⚠ **The first version of this table was WRONG, and the number that corrects it was printed on
the same line I read.** `NEVER FREED: 1 stores … kt=81 C×2` carries TWO counts: the STORE count
and, after the type, the RECORD count.  I read the store count, called the leak "bounded at 1",
and published that.  The records are what accumulate — one store because `OpDatabase` on a
non-null slot reuses it and appends.  Corrected by the peer stream and re-measured here:

| cell | stores | RECORDS |
|---|---|---|
| `no-closure.loft` — loop-scoped local, no closure | 0 | 0 |
| `closure-in-loop.loft` — the same plus a closure capturing it (3 iterations) | 1 | **C×2** |
| `closure-in-loop.opt.loft` — its NULLABLE twin | 0 before the peel | **C×2** after |
| `one-iteration.loft` — the same shape, one iteration | 0 | 0 |
| `wa-minting-call.loft` — literal replaced by `mk(5 + i)` | 0 | 0 |
| `wa-hoisted-local.loft` — local declared outside the loop | 0 | 0 |
| `fifty-calls.loft` — 50 calls × 3 iterations | 1 | **C×100** |

**It scales on BOTH axes.**  Within a call: 2 → `C×1`, 3 → `C×2`, 5 → `C×4`, 9 → `C×8`,
17 → `C×16`, i.e. `C×(n-1)`.  Across calls: 50 calls × 3 iterations → `C×100`.  So it grows
without bound in a long loop or a long-running program, which is why loft#1483 is not the `sev:low`
I first filed it as.

**The reading, and it is the second time this plan has produced it.**  The nullable spelling was
accidentally leak-free: it declines the in-place hint, so it takes a work-ref road that frees
correctly.  The DENSE spelling — the one the rules would call the oracle — carries the defect, and
converging the two inherits it.  Exactly the shape of loft#1482, where the dense half aliased and
the nullable half had already been fixed.

So a divergence is not evidence about WHICH half is right.  `@FR-N-Shape` says only that the two
must agree; what settles which answer they should agree ON is a rule about the thing itself —
`(F-Ret)` for loft#1482, the store-lifetime rules for this one.

## ⚠ A leak sweep on `LOFT_STRICT_STORES` alone reads a PANIC as clean

`LOFT_STRICT_STORES=1` implies `LOFT_NO_SLOT_REUSE` (`keys.rs`: *"which needs the no-reuse
guarantee"*), so a leak-FREE program that churns more than 65535 stores aborts with *"store table
exhausted"* instead of finishing.  A sweep that greps its output for `strict-store` — the obvious
thing to grep — sees no such line and scores that file **clean**.

Measured while sweeping the 45 files the `@FR-N-Shape` peel changes: 1 of the 45
(`1320-a-branch-joined-binding-frees-the-arm-that-minted`) aborts that way.  Under an oracle that
keeps slot reuse it is clean and not close to a leak — `LOFT_STORES=timeline` reports *986546
allocs, 986544 frees, peak 9 concurrently-live — NO leak*.  So the blind spot hid nothing here, but
it would have hidden anything.

**So a leak sweep needs two greps, not one**: the `strict-store` line AND `store table exhausted`,
with the second re-measured under `LOFT_STORES=timeline` or `LOFT_NATIVE_LEAK_CHECK` (both keep
reuse).  The same applies to any sweep that treats "no leak line" as a pass.
