<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# General loop mechanisms, priced (2026-10-24)

Owner: optimisations are GENERAL, for every loop and every operator, never fitted to one
construct ([par-rows.md](par-rows.md) was removed for that).  These prices ask what the general
mechanisms buy.  Each is a hand form of the emitted Rust (`bench/portal/hand_price.sh`, laptop,
pinned, three interleaved runs, hash unchanged in every row).

## `06_newton_sqrt` (1.67×): the `?` on a float division

`guess = (guess + (x / guess)?) / 2.0`, 50 rounds per call.

| form | ms | vs Rust |
|---|---|---|
| as emitted (`?` as a select) | 21.4 | 1.67× |
| the constant loop unrolled, select kept | 21.2 | no gain: LLVM already removes the loop overhead |
| `?` as a cold branch | 17.4 | the test leaves the dependency chain |
| loop plain, a sticky NaN flag, the exact function rerun when it is set | 16.5 | |
| the flag form, and the constant loop unrolled | 14.3 | 1.12× |
| no NaN test at all (not admissible, the ceiling) | 12.8 | 1.00× |

The whole gap is the discharge.  As a select it sits on the critical path of a serial division
chain, and it keeps LLVM from fully unrolling the 50 rounds, so the CPU cannot overlap one
call's divisions with the next call's (the ceiling has 58 `divsd`, the emitted form 10).

## `02_sum_loop` (1.81×): checked integer operators on a loop-carried local

`acc = ((acc ^ i) + (i & 7)) & 16777215`.

| form | ms | |
|---|---|---|
| as emitted (checked ops) | 5.39 | |
| plain ops, a whole-loop overflow flag, exact rerun | 4.24 | −21 % |
| blocks of 1024 with a flag and a block rerun | 4.56 | the block structure costs |
| plain ops (the ceiling) | 2.91 | Rust 2.87 |

In a three-operator chain the overflow flag costs as much as the chain.  A static proof is
the better tool here: after `& 16777215`, `acc` lies in `[0, 2^24)`, so a loop-carried range
fact (`@FR-R-Range` over a local whose every write is bounded) makes every operator plain.

## The mechanisms, in order of reach

1. **A sticky fault flag over a loop that writes only locals.** Run the body plain: a
   discharge or a null-producing operator ORs a flag instead of branching or selecting.  Read
   the flag once after the loop, and when it is set rerun that loop exactly from the saved
   live-in locals.  It reaches every loop with a `?` / `??` (about 1,000 discharges in the
   consumers and libraries) and every checked operator.  Combined with the unroll below:
   −33 % on `newton_sqrt`.
2. **A loop-carried range fact** for integer locals whose every write is bounded: all of
   `sum_loop`'s gap, and the same shape as `14_stdlib_vector/index_read` (1.94×).
3. **Full unroll of a constant loop of at most 64 rounds** with a straight-line body.  It pays
   only once the body is branch- and select-free (mechanism 1): then LLVM overlaps calls.
