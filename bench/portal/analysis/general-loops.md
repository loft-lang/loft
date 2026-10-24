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

## Design: `(R-FaultFlag)`, mechanisms 1 and 3

**Built** (`generation/fault_flag.rs`): `06_newton_sqrt` 1.67× → 1.14× of Rust (priced 1.12×),
hash unchanged.  Across the bench it admits 5 statements.  The declines, ranked: 462 name a
call (196 of them `OpLengthCharacter`, a text walk; the rest user and stdlib functions) and 230
a store READ (`OpGetVectorNullable`, `OpGetText`, `OpLengthVector`, `OpGetInt`, …).  A read has
no effect a rerun can observe, and a call to a function whose own body passes the walk has
none either, so those two clauses are the next reach.

**Unit.** A `for` statement (the IR's `For block`: the counter seed, the range end, the loop).
Taking the whole statement, not the bare loop, lets the rerun restart from the seed and lets
the unroll see the seed.

**Admission** (a walk over the block, declining with a reason under `LOFT_TRACE_FAULT_FLAG`):

1. Every node is a literal, a `Var`, a `Set` of a local, a block, an `if`, a loop, a
   `break` / `continue` that stays inside the statement, a `Drop`, or a call to an OPERATOR
   whose `#rust` template touches no store: once `s.raise_recoverable(…)` is taken out, it
   names no `s.`, `stores`, `cell` or `crate::`.  A user function, a store read, a text, a
   `return` or a `yield` declines: the statement must have no effect but its locals.
2. Every local the statement writes is a scalar (`integer`, `float`, `single`, `boolean`,
   `character`) whose address nothing takes.
3. At least one node has a fault form (below), or the copy buys nothing.

**Emission.**

```
{ static __FFM_n: AtomicU32 = AtomicU32::new(0);      // this site's reruns
  let __ffs_n = (var_a, var_b,);                       // the written locals declared before it
  let mut __ff_n: bool = ops::fault_flag_skip(&__FFM_n);
  if !__ff_n { <the statement, FAST> }
  if __ff_n { ops::fault_flag_rerun(&__FFM_n); (var_a, var_b,) = __ffs_n; <the statement, as today> } }
```

`fault_flag_skip` is true under `--dev-soft-halt`, where an integer overflow REPORTS, and once
a site has rerun 8 times: a loop whose nulls are common pays its fast copy at most eight
times.

**The fault forms**, each answering today's value whenever it leaves the flag clear:

| node | fast form |
|---|---|
| `if not-null(n) { n } else { d }` (the lowering of `??` and `?`) | `__ff \|= is-null(n); n` |
| a template's `s.raise_recoverable(k)` | `ops::fault_flag_raise(&mut __ff, k)` |
| `+ - *` and negation on integers, checked or `Nullable` | wrapping, `__ff` set on a null operand or an overflow |
| `&`, `\|`, `^` on integers | the operator, `__ff` set on a null operand |

Every other node is emitted exactly as today, so the fast copy differs only where a fault
form stands in.

**The unroll.** Inside the fast copy, a `for` over `0..C` (literal `C` in `1..=64`, the
counter seeded at its start) whose body is straight-line (no loop, `break`, `continue` or
`return`, at most 48 nodes) is emitted as `C` copies of the body, each with the counter set.

**Invariant.** The fast copy's only effect is on locals, and those are restored before a
rerun.  So a set flag leaves no trace, and a clear flag means every fault form answered
exactly what today's form answers.
