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

| cell | leaked |
|---|---|
| `no-closure.loft` — loop-scoped local, no closure | 0 |
| `closure-in-loop.loft` — the same plus a closure capturing it | **1** |
| `closure-in-loop.opt.loft` — its NULLABLE twin | 0 before the peel, **1** after |
| `one-iteration.loft` — the same shape, one iteration | 0 |
| `wa-minting-call.loft` — literal replaced by `mk(5 + i)` | 0 |
| `wa-hoisted-local.loft` — local declared outside the loop | 0 |
| `fifty-calls.loft` — 50 calls to the leaking function | **1** — bounded |

**The reading, and it is the second time this plan has produced it.**  The nullable spelling was
accidentally leak-free: it declines the in-place hint, so it takes a work-ref road that frees
correctly.  The DENSE spelling — the one the rules would call the oracle — carries the defect, and
converging the two inherits it.  Exactly the shape of loft#1482, where the dense half aliased and
the nullable half had already been fixed.

So a divergence is not evidence about WHICH half is right.  `@FR-N-Shape` says only that the two
must agree; what settles which answer they should agree ON is a rule about the thing itself —
`(F-Ret)` for loft#1482, the store-lifetime rules for this one.
