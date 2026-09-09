<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 150 — A fn-ref call's return ownership is a per-run fact, and there is no channel for it

Tracker: [@PLN150](https://github.com/loft-lang/plans/issues/150).

## Status

**CLOSED 2026-09-09.**  The question is answered on both backends and across the whole destination
axis the issue's Scope names, with the guard falsified once per backend.  The channel the plan
asked for exists: the per-run MINTED-or-BORROWED verdict, computed where it always was and carried
one hop to the caller's bind.

Two things routed OUT rather than done: loft#1487 (a `??` inside a lambda leaks its unused fallback
arm) is a different mechanism — it reproduces with no forwarder at forwarding depth 0 — and the
nine unaudited `is_struct_returning_call` sites are the general free-source LICENCE question, which
is [@PLN155](../155-licence-to-free/README.md)'s, not fn-ref return ownership.

No keystone registered, deliberately: `ownership/free` already carries two this cycle (@PLN155 at
\#1479, @PLN160 at \#1485), and a third would leave all three unjudgeable, since each one's BEFORE
window would contain the others' effect.

## ⚠ The issue's own premises are STALE — read this first

The plan was filed against loft#1185 and loft#1186 with a table measured on
`tuxedo-stability-impact`.  **Both bugs are now CLOSED and one of the two candidate channels has
SHIPPED.**  Re-measured here on `e4c7db584`:

| the issue says | actually |
|---|---|
| loft#1185 / loft#1186 open, this plan is "the mechanism they need" | both CLOSED |
| candidate channel 1: a monotonic `Store::alloc_serial` snapshot | **shipped** — `store.rs:387`, used by `release_fnref_bufs` and `cr_fnref_minted` |
| "OWNED (today) → use-after-free … both backends" | ⚠ **both are affected, in DIFFERENT SHAPES** — see below.  This row first read "native is CLEAN", which was true of the shape measured at the time (a LOCAL destination) and false in general: native froze the capture for a FIELD destination.  A backend reading is only as wide as the destinations it was taken over |

What is actually live is named in `formal/closures-history.md`, in D-clo-12's own closing entry:

> ⚠ **The BOUND spelling is not closed**: `{ r = f(v); r }` binds before returning, so the tail is
> a `Var` and no tail-shaped rule reaches it — 4 use-after-free reads either side.  … @PLN150.

That is this plan's real scope, and even that entry's "either side" no longer holds.

## The boundary, re-measured (18 cells, `cells/`)

Axes: forwarder spelling (tail `{ f(v) }` / bound `{ r = f(v); r }`) × lambda tail (capture /
join `cap ?? P{…}` / mint) × forwarding depth (0, 1, 2).  Value read from **stdout only** — the
leak warning goes to stderr, and merging them hid every join row's value on the first pass.

**Two independent defects, both INTERPRETER-ONLY** (native is clean in all 18):

**A — the bound spelling's use-after-free.**  `bound_capture_d1/d2`, `bound_join_d1/d2` read
`13 0 0 0` against a wanted `13 13 13 13`, with 4 strict-store violations.  The capture cell also
reports `cap 0` — **the caller's own capture is destroyed.**  Boundary: bound spelling AND a
capture-reaching tail AND depth ≥ 1.  Controls that hold: every `d0`, every `tail_*`, every
`mint_*`.

**B — the join arm leaks one store**, in BOTH spellings at ALL depths, values correct, bounded at
1 regardless of iteration count.

### The value channel cannot see defect A

Under `LOFT_STRICT_STORES=1` the same cells read `13 13 13 13 | cap 13` — correct — because that
mode implies `LOFT_NO_SLOT_REUSE` and the freed record keeps its bytes.  The defect is visible only
in the violation channel (4) and under `LOFT_POISON=1`, which answers
`cap -2401053088876216593`.  `LOFT_STORES=timeline` says 12 allocs / 10 frees, NO leak — so it is
a double-USE, not a leak.  Three instruments, three different answers, and only two of them see it.

## Root cause, pinned to one flag

`LOFT_TRACE_COPY` on both spellings, capture = store 2:

```
TAIL (correct)    src=(2,1) dst=(5,1) free_source=FALSE   ← fwd1 materialises a fresh temp
                  src=(5,1) dst=(4,1) free_source=true    ← main consumes that temp: right
BOUND (broken)    src=(2,1) dst=(4,1) free_source=TRUE    ← main frees the CAPTURE
                  src=(2,1) dst=(2,1) free_source=true    ← src==dst, alias no-op, reads freed bytes
```

`COPY_FREE_SOURCE` (`keys.rs`, `0x8000`) means *"the source is a callee's fresh temporary nobody
else frees"*.  Whether a fn-ref-forwarded result IS that **is a per-run fact** — the callee answers
it with a capture on one arm and its own mint on the other.  That is the plan's thesis, in one bit.

The tail spelling escapes only because `materialize_view_return` inserts a genuine fresh temporary,
so freeing the source is right there.

## Why native is clean and the interpreter is not

Not the guard: the free-source test in `codegen_runtime.rs` and in `state/io.rs` is **byte-identical**,
down to `is_free_protected()`.  The difference is the emitted shape around it.  Native wraps the
bind in an adopt-vs-copy runtime test —

```rust
if _src.store_nr == u16::MAX || _src.store_nr == _dst.store_nr { … adopt … }
else { var_r = OpDatabase(cell, _dst, 81); OpCopyRecord(cell, _src, var_r, 32849); }
```

— and performs **one** copy in the whole run (from the stack store, where the source-free is
refused).  `State::copy_ref_or_null` has no such test: it reads `dst` and calls `do_copy_record`
unconditionally, four times, and the first one frees the capture.

**So this is one notion with two spellings again** — the same shape @PLN160 kept finding, here
between the two BACKENDS rather than between `τ` and `τ?`.

## The wider finding: 11 sites decide this, 2 ask the canonical question

`is_struct_returning_call` answers *"is the RHS a call"*.  `call_return_frees_source` is what the
tree itself calls *"the canonical answer to exactly this question — it was written for this bit
(loft#981/#982)"*, and loft#1140 is the record of the bare form shipping a bug.  Audited:

| decides "free the source" | guarded by `call_return_frees_source` |
|---|---|
| 11 sites | **2** |

Unguarded: `expressions.rs:4374`, `expressions.rs:5477`, `operators.rs:970`, `vectors.rs:5049`,
`vectors.rs:5084`, `objects.rs:4946`, `collections.rs:1701`, `mod.rs:10846`, `mod.rs:11404`.

⚠ **Not all nine are defects** — @PLN155's rule applies: *equal today is not the same rule*.  A site
whose destination is provably fresh and whose source is a literal needs no per-run answer.  The
list is a queue to READ, not a count of bugs.

## The fix — the channel, built where the answer already was

Defect A is CLOSED.  The cure is not at the return, where both the issue and D-clo-12 pointed, but
at the **caller's bind**, and it needed no new machinery:

`release_fnref_bufs` already computes *"did this frame MINT the store it is handing back, or did it
borrow one that predates the call?"* — the `alloc_serial` stamp against the snapshot taken when the
call began — and then threw the answer away.  `State::fnref_borrowed_return` carries it one hop to
`copy_ref_or_null`, which clears `COPY_FREE_SOURCE` for a source that predates the call.  Set only
on a borrowed return, consumed by the next bind, so nothing stale outlives the value it describes.

**Why this is the channel and not another trade.**  The issue's own table shows every STATIC
reading buying one defect with the other — OWNED gives a use-after-free on the capture arm, BORROW
leaks one store per call on the mint arm.  Re-measured over all 18 cells after the fix:

| | before | after |
|---|---|---|
| capture arm, bound spelling, depth 1–2 | `13 0 0 0`, 4 violations | `13 13 13 13`, 0 |
| mint arm (the control) | clean | **still clean — 0 leaks** |
| every cell, both backends | disagree on 4 cells | **agree on all 18** |

That is the whole point of a per-run answer: it closes one arm without opening the other.

Guard: `tests/scripts/1185b-a-forwarded-fnref-result-bound-before-return-is-not-the-callers.loft`,
2 cells + 3 controls, **falsified at `31c4e04da`** — interpret exit 1 → 0, 2 assertion failures →
0; native INERT, which is the correct reading for a backend-divergence guard since only one side
can move.

## The NATIVE half — found by finishing the destination axis

The first pass covered two of the four destinations the issue's Scope names (*local, rebind,
return, forwarded*).  Filling in the rest found a **live defect on the other backend**:

| destination | interpret | native |
|---|---|---|
| local, rebind, return | correct | correct |
| **field** — `b.p = fwd(s, 1)` | correct | **`13 0 0 0 \| cap 0`** |

Same question, same bound-spelling/capture-arm shape, opposite backend — and the controls place it
exactly: `field + tail` and `field + mint` are clean on both, so it is neither the destination
alone nor the forwarding alone.

**The cause is the same sentence twice.**  `cr_fnref_minted` stands where both halves of the answer
are in scope — the returned `DbRef` and the `alloc_serial` snapshot — computes MINTED-or-BORROWED,
registers the mint, and *returned* on the borrow.  Exactly what `release_fnref_bufs` did on the
interpreter.  The cure is the same one hop: `FNREF_BORROWED` carries the verdict to
`OpCopyRecord`, which declines the source-free for a store that predates the call.

```
BROKEN   src=#0 dst=#4 free_src=TRUE    ← copies the CAPTURE into the lift and frees it
WORKING  src=#0 dst=#4 free_src=FALSE   ← the tail spelling materialised a real temp first
```

**Two backends, one question, and each had a test the other did not.**  The interpreter was correct
for a field destination and wrong for a local; native was correct for a local and wrong for a
field.  Neither backend's own suite could see its gap, because the shape that exposes it was only
ever exercised on the other one.  The guard now falsifies on BOTH — `31c4e04da` for the
interpreter half, `ace157ea4` for the native half, each INERT on the side the other fixed.

## Defect B is filed, not fixed — loft#1487

The `??`-in-a-lambda leak is a different mechanism and the measurement says so: it reproduces with
**no forwarder at all**, at forwarding depth 0, so it is not the fn-ref return channel.  Bounded at
one store for 1, 4 or 20 iterations; both backends; values correct throughout; clean workaround.
It is D-clo-13's residual on the arm that is NOT taken — when the fallback wins its store becomes
the result and is freed with it, and when the borrow arm wins nothing owns the minted fallback.

## Still open from the audit

The 11-sites-2-guarded table above is a queue to READ, not a count of bugs.  This plan fixed the
one site its own defect ran through; the other nine want the @PLN155 treatment — read what each
asks, cite the rule, and leave a note where two look alike and must stay apart.
