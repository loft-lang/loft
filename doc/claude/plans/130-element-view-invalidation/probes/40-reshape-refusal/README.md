<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Probe 40 — the boundary of the reshape-under-reference REFUSAL (@PLN130 F9, loft#779)

One cell per file, because a compile-time refusal ends the whole program — cells cannot share
one. `run.sh` compiles each on BOTH backends and prints its value line or its error.

Every `want` below was **hand-computed before the fix**, and the *before* column was measured on
the pre-fix binary. Agreement between two binaries is not a pass, so each cell asserts a
distinctive value (`n` 11/22/33, `tag` 111/222/333) rather than a shape.

## What the cells hold fixed, and what they move

`A` the `&` spelling · `B` where the removal sits (this frame / one call down / two calls down) ·
`C` whether the viewed element MOVES · `D` liveness of the reference at the removal ·
`E` local `&` link vs. reference passed as an argument.

| cell | shape | before | after |
|---|---|---|---|
| **S1** | `c = &v[0]; v.remove(2); c.n=99` — element does not move | 11, write lost | **refused** |
| **S2** | `c = &v[2]; v.remove(0); c.n=99` — element moves 2→1 | 33, write lost | **refused** |
| **S3** | `c = &v[0]; c.n=99; v.remove(2)` — link DEAD at the removal | 99 | **99** (liveness is the condition) |
| **S4** | `c = v[0]` (PLAIN bind) `; v.remove(2); c.n=99` | 11 + advice | **11 + advice** (F2, untouched) |
| **S5** | removal BEFORE the bind | 99 | **99** |
| **S6** | `&` link, no removal at all | 99 | **99** |
| **S7** | `&` link into `v`, removal from `w` | 99 | **99** |
| **S8** | `c = &o.inner` (field, not element), removal from `v` | 99 | **99** |
| **S10** | READ (not write) through the link after the removal | 11 | **refused** — a use is a use |
| **S11** | removal inside an `if` branch | 11 | **refused** |
| **S12** | removal inside a loop | 11 | **refused** |
| **S13** | two links, one dead one live | 77/22 | **refused** (names `live`, not `dead`) |
| **X1** | **loft#779's own repro** — `shift(v[2], v)`, callee removes | 33, write lost | **refused** |
| **X2** | `t = v[2]; shift(t, v)` — same, view bound first | 33, write lost | **refused** |
| **X3** | callee removes an element BELOW the view (probe 38 C1) | 99 — **worked** | **refused** |
| **X4** | X1 with the `&` DROPPED from the view parameter | 33, write lost | **refused** |
| **X5** | callee removes from `v`; the reference is into `w` | 99 | **99** |
| **X6** | callee writes but never removes | 99 / 7 | **99 / 7** |
| **X7** | the removal is TWO calls down | 33, write lost | **refused** (call-graph closure) |
| **X8** | callee removes from its OWN local vector | 99 | **99** |
| **X9** | plain `Box` parameter, no removal anywhere | 99 | **99** |
| **X13** | `c = &v[0]; drop_last(v); c.n=99` — local link, callee removal | 99 | **refused** |
| **X14** | as X13 with the element MOVING | 33, write lost | **refused** |
| **X15/X16** | the severity question: does the stale write resurface? | 44/444 | refused (they are X4/X1 plus an append) |

## The three cells that decided the shape of the fix

**X9 — a PLAIN struct parameter aliases exactly like a `&` one.** `fn w(t: Box) { t.n = 99 }`
called as `w(v[2])` writes 99 into the caller's `v`; loft's own `warn_redundant_amp` advice says
so in as many words. So X4 is X1's lost write without the `&` spelling, and refusing only the
`&` form would mean an author who takes loft's advice and drops the `&` trades a compile error
for a silent lost write. The cross-frame refusal therefore keys on the ALIASING relation, not on
the token. loft#779's own table said the opposite (*"A2 … plain param copies (@C86), so nothing
to lose"*) — that row is measurement-contradicted, and X9 is the measurement.

**S4 — a plain LOCAL bind is the other way round.** It does not alias across a reshape, because
@PLN130 F2 materialises it and says so. So producer 1 (a link in this frame) stays `&`-only.
The two halves are not inconsistent: at a parameter there is no bind site to materialise at.

**X3 — the refusal costs a program that works today.** The removal is below the view, so the
element never moves and the write lands. It is refused anyway, because the rule is about an OPEN
reference and not about whether this particular removal would have invalidated it — and because
deciding otherwise needs the removal index and the view index, which are usually dynamic. The
same-function twin (S1) is already broken today, so keeping the two answers consistent is worth
more than saving X3.

## X15/X16 — the severity, measured rather than assumed

Measured on the PRE-FIX binary (both are refused now, being X4 and X1 with an append bolted on).
The answer is a lost update, not corruption: the stale write lands in the vacated slot, and
appending afterwards overwrites it, so the value never resurfaces. Both spellings, both backends.
That is what makes the refusal the right resolution rather than an urgent runtime repair — no
program silently reads a torn record, they only silently miss an update.

## Lower bound, deliberate

A callee reached only through a **runtime fn-ref** has no static call edge, so the closure does
not see it and that shape keeps today's behaviour. Safe direction: the refusal simply does not
fire. The `LOFT_BIN` hook on `run.sh` re-runs the whole matrix against the installed mainline
binary, which has no refusal — that is the before/after oracle for the *before* column.
