<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 108 — Share read-only parent stores across par workers

Tracker: [@PLN108](https://github.com/loft-lang/plans/issues/108) · `subject:loft` · `status:active`

> **RE-OPENED — 2026-07-17.** The shipped core (S0–S10, below) wired the borrow into only
> `discard`/`queue` behind the size heuristic, so **../routing cannot reproduce the win** — its
> reduction par (`run_parallel_fold`) still copies (measured 1→71 ms, 0→61 MB, both backends).
> Re-open consumes the § Deferred items (**rayon reconciliation**, other queue variants, native
> analogue, threshold) under one goal: **one clone, one spawn primitive, always sharing — no copy
> path, no heuristic.** Design + steps: **[single-implementation.md](single-implementation.md).**
>
> **Original close note (interpreter core — 2026-07-17):** S0–S10 done and gate-green; borrow
> shipped default-ON via the size heuristic, win confirmed (par_ms flat vs heap, 53×), ASan + TSan
> clean. Retained below as the closure record of the first cut.

## Status

Open — design ready, no implementation. This is the deferred **phase 5** of the
legacy typed-par plan ([`finished/06-typed-par/`](../finished/06-typed-par/README.md)):
`parallel.rs:99` calls it "Phase 5's `Arc<Store>` rewrite makes every path light by
default." The auto-light *heuristic* shipped ([`05-auto-light.md`](../finished/06-typed-par/05-auto-light.md));
the parent-store **sharing** it was meant to unlock did not. The live par dispatch
(Queue family + heavy) still `clone_for_worker()`s a **byte-copy of every active
parent store per worker** — the "slicing a large read-only structure per job is real
work" a par consumer reported. Filed as its own plan (not extending the legacy dir)
because it is a **memory-model change (loft priority #1)**, not a spot fix; ASan/TSan
are load-bearing gates, not optional.

## Goal

Let a par worker read a provably-unwritten captured parent store **directly**,
eliminating the per-worker byte-copy — a copy-elision with no semantic change.

## Effort + design

- **Effort:** M (Option A) → H if it escalates to Option B.
- **Design:** ✓ (detailed below; A-vs-B decision is the one open question).
- **Last touched:** 2026-07-17.

## Composition matrix — Stage A

**No new composition surface** — this is a copy-elision of the memory model, not a new
value / type / operation, so the standard both-backends value-matrix does not apply.
The acceptance instrument is instead a **data-race + lifetime gate matrix** (§ Gates):
ASan for the borrow-lifetime/UAF axis, TSan for the shared-read race axis (each with a
firing positive control), plus the existing par order/value tests unchanged on both
backends — a data race is the one fault loft cannot null-out, so the gate *is* the spec.

## Why it is safe to share — the proof already exists

A par worker's captured parent state is **read-only** by @PLN102 **@C93**: a
`ParentWrite` from inside a worker is a *compile error* (`scopes.rs`; "par() worker …
captured state is READ-ONLY"). So every parent store a worker sees is **provably
unwritten for the lifetime of the par**. The current per-worker byte-copy is therefore
pure conservatism — nothing the worker does could observe a divergence between its copy
and the shared original. Sharing does not *weaken* an invariant; it *rests on* one the
compiler already enforces. Two facts make the lifetime sound:

- **`thread::scope`** — every dispatcher spawns workers inside a scope and **joins them
  before the scope ends** (`parallel_workers`, `run_parallel_queue`). The parent
  `Stores` outlives every worker by construction; a borrowed pointer cannot dangle.
- **Synchronous par** — the main thread blocks in the join; it does **not** run (so
  cannot mutate a parent store) while workers borrow. (Must VERIFY no dispatcher
  materialises/adopts a parent store mid-par before the join — step 4.)

## Current state (what exists, what's live)

| Piece | State | Note |
|---|---|---|
| `clone_for_worker()` (`allocation.rs:1277`) | **LIVE default** | byte-copies every active store (`clone_locked_for_worker` → `alloc` + `copy_nonoverlapping`); `Store::new(100)` for freed slots |
| `borrow_locked_for_light_worker()` (`store.rs:1292`) | exists, **dead** | SHARES `self.ptr`; `read_only:true`, `borrowed:true` (`Drop` does NOT dealloc — `store.rs:243`). Tests: `borrow_locked_reads_original_data`, `borrow_locked_write_panics` |
| `clone_for_light_worker(pool_slice)` (`allocation.rs:1361`) | exists, **dead** | borrows parents + a pre-allocated pool for worker-owned stores |
| `run_parallel_light` (`parallel.rs:1467`) | exists, **dead** | no live caller; off the `parallel_workers` template pending this rewrite |
| `Store.ptr: *mut u8` | raw owned buffer | `Store: Send`, **NOT `Sync`** (`store.rs:2352`) |

So the sharing machinery is **built and tested** — parked, not the default, waiting for
a clean lifetime story. This plan provides it.

## Two options

### Option A — revive the existing unsafe borrow (smaller, reuses tested infra)

Route the live dispatch through `clone_for_light_worker` (borrow parents + pool) instead
of `clone_for_worker`. The safety already lives in the flags: `read_only:true` (a worker
write panics, guarded by `borrow_locked_write_panics`) + `borrowed:true` (no
double-free). Needs `unsafe impl Sync for Store` — **justified only because the shared
access is read-only** (workers call `addr()` only, `store.rs:2350`); the write-panic
guard is the runtime backstop.

- **Cost:** small — wire an existing path; no `Store` representation change.
- **Risk:** safety is *contract-carried* (the `unsafe` borrow trusts @C93 + `thread::scope`
  + read-only), not *type-carried*. A future edit that mutates a parent mid-par, or lets
  a worker outlive the scope, reintroduces UAF/race silently. Mitigated by the write-panic
  guard + the ASan/TSan gates — but it is trust, not proof.

### Option B — `Arc<Store>` rewrite (the clean end-state `parallel.rs:99` names)

Make a parent store's buffer **`Arc`-shareable**, so sharing is safe *by construction* —
`Arc` owns the buffer, frees it once when the last worker + parent drop it, and
`Arc<T>: Sync` for read-only access. No `borrowed` bookkeeping, no hand-justified
`unsafe impl Sync`.

- **Cost:** larger — `Store.ptr: *mut u8` becomes an `Arc`-backed buffer for the shared
  read-only range (touches allocation/free/word-addressing), or wrap the parent range as
  `Arc<Store>` in `WorkerStores` (pushes `Arc<Store>` through the `allocations` access
  surface — wide, since `Stores` is shared main+worker).
- **Risk:** wide blast radius on the store representation; but safety is *type-carried*
  (the borrow checker + `Arc` prove no UAF / no double-free) — strictly better than A.

## Sub-arcs — the migration steps (A first, behind a flag)

| Step | Item | Status |
|---|---|---|
| 1 | **Bench the copy cost** — a par loop over a large read-only captured structure; measure per-worker `clone_for_worker` time. The win to beat; if the copy is a rounding error for the consumer's shapes, stop here. | Open |
| 2 | **Wire Option A behind `LOFT_PAR_SHARE` (default OFF)** on ONE dispatcher (`n_parallel_discard` — no return-stitch). Keep `parallel_store_is_read_only_in_workers`, `par_discard_does_not_grow_parent_stores` green. | Open |
| 3 | **`unsafe impl Sync for Store`** with a comment pinning the justification (read-only shared access + write-panic guard) and naming this plan. | Open |
| 4 | **VERIFY no dispatcher mutates a parent mid-par** — audit each `run_parallel_*` for a lazy materialise/adopt of a parent store between spawn and join; if any, move it BEFORE the spawn. | Open |
| 5 | **Run the gates** (§ Gates) — ASan + TSan (with positive control) + full suite both backends. | Open |
| 6 | **Bench the win** vs step 1 (copy gone; wall-clock over the large read-only shape). | Open |
| 7 | **Flip `LOFT_PAR_SHARE` default-on** once gates + bench pass; extend from `discard` to the Queue family; unify `run_parallel_light` into the `parallel_workers` template (`parallel.rs:99`). | Open |
| 8 | **Decide A-vs-B** — if step 4's audit or the TSan run shows the contract-carried safety is fragile, escalate to **Option B** (`Arc<Store>`) as its own follow-on. Otherwise A stands. | Open |

## Gates (mandatory — this is the data-race class)

- **ASan** (`scripts/asan.sh -E 'binary(threading)'`) — the borrow-lifetime / UAF check.
  Baseline **47/47 green** (2026-07-17); the change must stay 47/47.
- **TSan** (`-Zbuild-std` + target-scoped `-Zsanitizer=thread`, @PLN54 S2) — the
  shared-read race check. A data race is the one fault loft cannot null-out
  (DESIGN_DECISIONS.md), so a clean TSan run over the threading suite is the acceptance
  bar, with a **positive control** (temporarily let a worker write a parent → TSan must
  fire) so the clean run is non-vacuous.
- Full suite on **both backends** (interpreter + native).

## Invariants the change must not break (acceptance checklist)

- A worker **never writes** a parent store (@C93 compile error + `read_only` runtime panic).
- Parent `Stores` **outlive** every worker (`thread::scope` join before drop) — no dangling borrow.
- No **double-free** of a shared buffer (`borrowed:true` ⇒ `Drop` skips dealloc; Option B: `Arc`).
- No dispatcher **mutates a parent** between spawn and join (step-4 audit).
- Result **ordering + values unchanged** (copy-elision, not a semantics change) — existing order/value threading tests stay green on both backends.
- ASan 47/47 + TSan clean (with a firing positive control).

## Open design questions

1. **A vs B** — resolved by steps 4 + 5: contract-carried (A) stands unless the audit or
   TSan shows it is fragile, then escalate to type-carried (B). Records into
   `DESIGN_DECISIONS.md` when decided.

## Cross-arc dependencies

- **@PLN54** (sanitizer coverage) — supplies the ASan + TSan gates this plan's acceptance rests on.
- **@PLN102 @C93** (captured state is read-only) — the compiler-enforced invariant the whole safety argument rests on.

## See also

- [`finished/06-typed-par/05-auto-light.md`](../finished/06-typed-par/05-auto-light.md) — the shipped phase-5 auto-light heuristic (the sibling this completes).
- [`finished/06-typed-par/DESIGN.md`](../finished/06-typed-par/DESIGN.md) — the plan's store-per-worker model.
- [THREADING.md](../../THREADING.md) § Multi-threading Safety — now describes this intended model (read-only shared, the deep-copy conservative).
- [DESIGN_DECISIONS.md](../../DESIGN_DECISIONS.md) @C93 — captured state is read-only (the proof).
- [`54-sanitizer-coverage-expansion`](../54-sanitizer-coverage-expansion/README.md) — the ASan/TSan gates this depends on.
- Tracker: [@PLN108](https://github.com/loft-lang/plans/issues/108).
