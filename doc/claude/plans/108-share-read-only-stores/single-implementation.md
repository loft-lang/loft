<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN108 re-open — one par-worker implementation (single core + thin wrappers)

Tracker: [@PLN108](https://github.com/loft-lang/plans/issues/108) · `subject:loft` · re-opened 2026-07-17 · **R1–R5 COMPLETE 2026-07-17**

> **Status:** all steps done. The clone model is now **ONE** (`clone_for_light_worker`, always a
> read-only borrow); `clone_for_worker` + `clone_locked_for_worker` are deleted. Both backends
> share via the single `parallel_workers` chokepoint; the win reproduces on both (routing fold
> flat vs heap). Race gates green on the rayon path (ASan 0 UAF, TSan 0 races with a verified-firing
> positive control). Net **−515 lines** in `src/` across R1–R4b (105 ins / 620 del). Ready to land.

Picks up the § Deferred items the closed core parked — **rayon reconciliation**, **other
queue variants**, **the native analogue**, **drop the threshold** — and unifies them under
one goal: **there is exactly one way a par worker gets its stores, and it always shares.**

## Why re-open (the consumer proof)

The shipped core (S0–S10) wired the read-only borrow into `run_parallel_discard` and
`run_parallel_queue` only, behind the `par_share_for` size heuristic. **../routing cannot
reproduce the win** — measured on the merged binary, both backends:

```
par(b = spin(a), 8) { total += b }   heap 0 → 61 MB   par_ms 1 → 71   (still copying)
```

Routing's par is a **reduction** → `run_parallel_fold`, which never got the borrow. This is
a textbook *chokepoint-became-a-spray*: the sharing was bolted onto 2 leaf dispatchers, but
the ~7 variants (`fold`/`int`/`text`/`raw`/`queue_fn` + the copy branch of `discard`/`queue`)
all funnel through **one** helper — `parallel_workers` — that still byte-copies. Fix the
spray by making the chokepoint the only path.

## The invariant (one sentence)

> Every par worker, on **either backend**, borrows the parent stores **read-only** (raw-ptr
> share, `read_only:true` + `borrowed:true`, @C93 forbids parent writes) and owns **only** its
> own writable scratch — produced by the **single** clone `clone_for_light_worker`, dispatched
> by the **single** primitive `parallel_workers`; there is no copy path and no heuristic.

If a case you never tested is correct, it is correct because it went through that one clone
and that one primitive — not because a variant remembered to opt in.

## Re-assertion sites — the prospective tell (design-protocol step 2)

The bug is that the sharing invariant must currently be re-stated at **N > 1** sites and
omitting it at one is **silent** (a correct-but-slow result, no error) — exactly the failure
that shipped. Count them:

| Concern | Today (N sites, silent on omission) | Target |
|---|---|---|
| **Clone model** | 2 — `clone_for_worker` (copy) **and** `clone_for_light_worker` (borrow) | **1** — light only |
| **Spawn primitive** | 3 — `parallel_workers` (rayon+copy) + `run_parallel_discard_shared` + `run_parallel_queue_shared` (`thread::scope`+borrow) | **1** — `parallel_workers`, always light |
| **Opt-in heuristic** | 1 — `par_share_for` (≥ 2 MB) choosing copy vs borrow per call | **0** — borrow is always ≥ as fast |
| **Result shape** | ~7 `run_parallel_*` each re-inlining slice+dispatch+stitch | **3 thin wrappers** (discard / map / reduce) over the one primitive |

Driving `N × silence → 0`: collapse N to **1** real chokepoint (every path consults it), so
omission is not "silently slow" — it is *impossible*, there is no other path to take.

## The single core + the wrapper (the shape the owner asked for)

**One implementation** — `parallel_workers` (already the shared chokepoint: both backends'
`run_native_workers_*` call `crate::parallel::parallel_workers`, and every interp variant does
too). It slices `n_rows` across `threads`, clones **light** per worker, runs a caller-supplied
worker-body closure, returns per-batch results:

```rust
parallel_workers(stores, n_threads, n_rows, |start, end, ws| { /* run rows start..end */ })
```

**The wrapper** — result-shape is *not* a second implementation; it is a thin adapter over the
one core, generic over the return `R`:

- **discard** — call, ignore the `Vec<()>`. (`run_parallel_discard`)
- **map** — call, `merge_batches` into an ordered `Vec<R>`. (`run_parallel_{int,text,raw,queue}` —
  they differ only in `R` = `u64` / `String` / `Value` / `DbRef`, already generic.)
- **reduce** — call with a per-batch accumulator, combine partials. (`run_parallel_fold`/`int`-sum.)

The *backend* difference (bytecode `State::execute_at_*` vs a compiled Rust closure `F`) is the
**worker-body closure passed in**, never a second copy of the machinery.

## The load-bearing claim, and why rayon reconciliation is now possible

The closed core used `thread::scope` for the borrow path because `clone_for_light_worker` takes
`pool_slice: &mut [Store]` — a **disjoint mutable scratch slice per worker** — which the explicit
`for t { pool.slice_mut(t) }` loop provides but a rayon `Fn` closure (shared capture) cannot.
Three facts (verified in-tree 2026-07-17) dissolve that blocker:

- **`WorkerStores: Send`** (`database/mod.rs:597` `unsafe impl Send`) — a *borrowed* one is the
  same struct, so it moves into a rayon task exactly like the copy one does today.
- **The borrow is a raw-ptr share** (`store.rs::borrow_locked_for_light_worker`: `ptr: self.ptr`,
  `read_only:true`, `borrowed:true`) — no `Arc`, no lock; the parent's buffers are simply pointed
  at and never freed by the worker. rayon's `pool.install(|| par_iter.collect())` **scopes the
  parent's lifetime** (blocks until every task joins) exactly as `thread::scope` does.
- **`WorkerPool` is vestigial** — its own doc says the binary has no callers (`#[allow(dead_code)]`),
  and `clone_for_light_worker` **allocates fresh scratch** (`Store::new(cap)`) rather than reusing
  pool buffers. So `pool_slice` degrades to *"how many scratch stores, at what capacity"* — a count
  + default, self-allocated per call. **Drop the `&mut [Store]` parameter → the disjoint-slice
  requirement disappears → light-worker composes with rayon.**

**Consequence: the heuristic dies too.** `par_share_for` existed because the *old* borrow path
paid a per-call `thread::scope` OS-thread spawn (~200 µs × N), so for frequent small pars the
copy-on-rayon was cheaper — the regression S3 measured. On **rayon + light**, there is no thread
spawn and the per-worker cost is *borrow N store headers + allocate a tiny scratch* (µs, heap-size
independent). Borrow is then **always ≤ copy**, so there is nothing left to choose: no threshold,
no `LOFT_PAR_SHARE` flag.

## Delete list (the end state has these GONE)

- `Stores::clone_for_worker` (the byte-copy clone) — nothing calls it once `parallel_workers` borrows.
- `run_parallel_discard_shared`, `run_parallel_queue_shared` (the second spawn path).
- `par_share_for`, `PAR_SHARE_MIN_BYTES`, `LOFT_PAR_SHARE` (the heuristic + its override).
- `WorkerPool` + `slice_mut` + the `pool_slice` parameter of `clone_for_light_worker`.
- Any `run_parallel_*` that is a pure return-type fork of `map`/`reduce`/`discard` (fold into the
  three wrappers); keep only what a genuinely distinct result shape needs (e.g. `queue_ref` adopt).

## Falsification points / probes (probe-first — expect to be wrong somewhere)

1. **Self-allocated scratch is sufficient.** Prove a worker that allocates (not just reads) still
   works when its scratch is self-allocated per rayon task, not pool-seeded — a par whose body
   builds a vector/text per row, both backends, leak-clean (`LOFT_STORES=warn`).
2. **No small-par regression.** The case the heuristic protected: a tight loop of many small pars
   over a large live heap. Measure rayon+light vs today's copy — must be **≤**, not just the big-heap
   win. If it regresses, the per-task scratch allocation is the cost; pool it back via a rayon
   thread-local, *not* by resurrecting the heuristic.
3. **`queue_ref` / adopt survives the borrow.** Workers returning a `DbRef` whose store is *adopted*
   into the parent — the one result shape that writes back. Confirm adoption still moves worker-owned
   (non-borrowed) stores and never the borrowed parent ones.
4. **Races.** Re-run the shipped gates: ASan `binary(threading)` (UAF/leak) + TSan with the firing
   positive control. Read-only borrow + @C93 should keep 0 races; a data race here means a worker
   wrote a borrowed store — a @C93 hole, file it.
5. **Both backends identical.** The routing fold probe flat vs heap on `--interpret` **and**
   `--native`; every `tests/threading.rs` shape green on both.

## Verification (the win, restated as the acceptance test)

`bench/par_copy_probe.loft` (the reduction shape) → **par_ms flat vs heap on both backends**, i.e.
routing reproduces the win it currently cannot. Graduate the probe to `tests/threading.rs` as a
guard (co-located bench stays for the timing number). H-close only when the flat curve holds on
both backends with ASan+TSan green.

## Steps (small, safe, one observable each)

| # | Step | Observable |
|---|---|---|
| R0 | Bench baseline on the merged binary (done above) — fold copies, 1→71 ms. | the number to move |
| R1 | ✅ **DONE** — `clone_for_light_worker(&self)` self-allocates its scratch (4 × 1024-word stores, `Store::new` already free+init'd — byte-equivalent to the old `WorkerPool` seed); dropped `pool_slice`. The 2 live dispatchers (`discard_shared`/`queue_shared`) drop `WorkerPool::new`. (The dead `run_parallel_light` + `WorkerPool` + `pool_tests` stay — deleted in R3.) | **DONE** — probe 1 (allocating-worker queue par, borrow==copy==941472, leak-clean via `LOFT_STORES=warn`, both backends); threading suite **47/47** both default AND `LOFT_PAR_SHARE=1` (forced borrow through the self-allocated clone); clippy `-D warnings` clean. |
| R2 | ✅ **DONE** — `parallel_workers` (the single helper BOTH backends funnel through) always borrows via `clone_for_light_worker`, never `clone_for_worker`. Safe on rayon exactly as on `thread::scope`: `pool.install` blocks until every task `collect`s, so no borrow of `stores` outlives the call; `WorkerStores: Send`; workers never write the parent (@C93). | **DONE** — **acceptance test met**: routing probe goes FLAT vs heap on **both** backends — native **1→71 ms → 1→2 ms** (0→61 MB), interpret 4→4, `total` identical. Probe 2 (200 small pars over 61 MB): **20 ms native** (copy ≈ 14 s) — no small-par regression, the heuristic is confirmed unnecessary. Suite 47/47; probe 1 leak-clean both backends; clippy clean. |
| R3 | ✅ **DONE** — deleted the redundant second path + the heuristic + dead code: the `par_share_for` branches in `run_parallel_discard`/`queue` (they now always use the borrowing `parallel_workers`), `par_share_for` + `PAR_SHARE_MIN_BYTES` + `active_clone_bytes` (`LOFT_PAR_SHARE` gone), `run_parallel_discard_shared` + `run_parallel_queue_shared` (the `thread::scope` path), and the dead `run_parallel_light` + `WorkerPool` + `pool_tests`. **Net −440 lines.** `clone_for_worker` deletion moved to R4 — it's still used by the *direct-clone* dispatchers (`run_parallel_block` = `par {arm;arm}`, `run_parallel_queue_ref`) + the fold-combine + `not(threading)` fallbacks, which R4 collapses. | **DONE** — suite 47/47; routing still flat (native 3→1 ms); `LOFT_PAR_SHARE` ignored (always shares); clippy `-D warnings` clean (`--all-features` + default); all feature combos compile. |
| R4 | ✅ **DONE (6 of 7 sites)** — switched the direct-clone dispatcher `run_parallel_block` (`par{arm;arm}` — void arms, so it now gets the borrow win too) + the sequential/combine `clone_for_worker` sites (fold-combine, `not(threading)` fallbacks, `queue_fn`) to `clone_for_light_worker`. **`clone_for_worker` is now down to ONE caller: `queue_ref`.** | **DONE** — suite 47/47; probe 1 leak-clean; routing still flat. Probe 3 **fired** on queue_ref (see R4b). |
| **R4b** | ✅ **DONE** — root cause (probe 3, confirmed by reproducing `"Allocating a used store #8"`): the default light clone appends **4 self-allocated scratch stores**, pushing the worker's stack store from index `parent_len` to `parent_len + 4`, so the shared output-slot dispenser (floor `parent_len + 1`) eventually dispenses the *live* stack-store index → the used-store assert fires. Fix: a **scratch count** on the light clone; queue_ref calls it with **0** — layout then identical to the retired byte-copy clone (borrows `0..parent_len`, stack at `parent_len`), dispenser math unchanged. With queue_ref switched, `clone_for_worker` + its sole helper `clone_locked_for_worker` have **zero callers → deleted**. **Clone model is now ONE.** Net −82 lines. | **DONE** — queue_ref acceptance probe (worker returns `vector<integer>`): total/count **invariant to a 0→24 MB parent heap**, leak-clean (`LOFT_STORES=warn`); `vector<text>` owned-sub-DbRef adoption leak-clean; suite **47/47** both backends; full suite green (only the known `wasm_debug_relay` env flake); clippy `-D warnings` clean (`--all-features` + default). Committed `bed597ac`. |
| R5 | ✅ **DONE** — race gates re-run on the **rayon** path (R2 moved the borrow off the now-deleted `thread::scope`). **ASan** (`scripts/asan.sh -E 'binary(threading)'`, pinned nightly-2026-05-28): **47/47, 0 UAF/heap-overflow.** **TSan** (`-Zsanitizer=thread -Zbuild-std`, isolated `target/tsan`): a fresh **positive control** (env-gated unsynchronised static-byte RMW inside the rayon `parallel_workers` closure) **FIRES** — TSan reports exactly one race, in `parallel_workers…run_parallel_int::{closure#0}` (non-vacuous); control **off** ⇒ **0 races, 47/47**. Control reverted after the run. Read-only borrow + @C93 (no parent writes) ⇒ race-free, as predicted. Win locked by `par_queue_ref_adopts_and_rebases` (adoption path — fails loudly if scratch is ever re-added to the queue_ref clone) + `par_*_does_not_grow_parent_stores` (no store leak) + `bench/par_copy_probe.loft` (the flat-vs-heap timing artifact). | **DONE** — 0 races both control states as predicted; ASan clean; guards green. |

## Discovered (pre-existing, unrelated) — native ref-return par codegen bug

A native (`--native`) par whose worker **returns a vector/reference** fails to *compile*
the generated Rust (`E0605` non-primitive cast; `native compilation failed (codegen bug)`).
Reproduces on a 4-line probe; **not** caused by @PLN108 (none of R1–R5 touch codegen — native
never calls `clone_for_worker`, and non-ref native pars compile + run fine, e.g. routing's).
The interp path is correct. File separately against native codegen (ref-return par lowering).

## See also

- [README.md](README.md) § Deferred — the parked items this re-open consumes.
- `src/parallel.rs` (`parallel_workers`, the `run_parallel_*` family, `WorkerPool`) ·
  `src/database/allocation.rs` (`clone_for_worker` / `clone_for_light_worker`) ·
  `src/store.rs` (`borrow_locked_for_light_worker`) · `src/codegen_runtime.rs` (native `run_native_workers_*`).
- `../routing/PLAN-PERF.md` § 18 — the consumer that blocks on this.
- [THREADING.md](../../THREADING.md) · [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) (@C93 no-parent-write).
