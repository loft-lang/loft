<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `par` write contention and dispatcher scaling — the measurement task

**Status (2026-09-15):** received as a handoff (a read-only review of
`origin/tuxedo-work-2026-09-15` at `f9d025964`, made from a blobless clone — **nothing
in it was run or measured**).  Its docs item, D1, is DONE in the commit that added this
file: `THREADING.md` § *Cache-line contention* states what the design guarantees and
lists F1–F4 as open, the two P1 sections are marked historical with today's flow, and
the `par_fold` paragraph and the `Stitch` comment in `src/parallel.rs` name the real
backing (`n_parallel_fold` → `run_parallel_fold`; `QueueStitch` is the live dispatch
enum).  The five measurements below are NOT done.  Every site the review cites was
checked to exist on this tree (`parallel_workers`, `merge_batches`,
`run_parallel_queue_ref`, `make_worker_slot_dispenser`, `clone_for_light_worker`,
`Stores::database_named`'s dispenser branch); `f9d025964` itself is not in this clone.
Run them on a quiet box — `perf c2c` and 8-thread probes are exactly the load this
14 GiB machine cannot take beside a build (its `/tmp` is RAM).

## Scope and ground rules

- **This is a measurement and verification task, not an implementation task.**
  Each item ends in a finding: confirmed with numbers, or refuted.  A fix is a
  design question for the owner and waits for the owner's go.
- **Both backends.**  `--native` par calls the same `parallel::parallel_workers`
  template (`src/codegen_runtime.rs`), so a finding on one backend is checked on
  the other.
- **Falsify every probe.**  Every probe gets a positive control that shows the
  instrument can see the effect: a deliberately contended cell, or a known
  collision.  A clean result without that control proves nothing.
- **Bugs go to GitHub issues** (loft-lang), not to a `.md` register.
- **Pin the bench.**  Use the hash-pinned bench discipline from PERFORMANCE.md:
  record the hash, the machine, and a before/after range, not a single number.

## Background — what `par` is today

- **Syntax:** `for a in src par(b = worker(a), N) { body }`.  The first worker
  argument is the loop element (loft#1060).  Extra arguments are scalars only.
  Captured parent state is read-only; a `ParentWrite` from a worker is a compile
  error (@PLN102 C93).
- **Parent stores:** workers borrow them read-only through
  `Stores::clone_for_light_worker`, which is the only clone path (@PLN108).
- **Worker allocations:** each worker gets 4 × 1024-word scratch stores,
  allocated inside its own rayon task.
- **Partitioning:** `parallel_workers` (`src/parallel.rs`, threaded variant near
  line 149) gives thread `t` exactly one range, `[t·n/T, (t+1)·n/T)`.  There is
  no stealing inside a range.
- **Result collection:** each worker pushes to its own `Vec`.  `merge_batches`
  copies them into place sequentially after the join, so no two cores write
  neighbouring result slots.
- **Assessment:** the plain return paths look free of false sharing by
  construction.  The findings below concern the record-returning path and the
  documentation.

## F1 — shared atomic writes per store allocation in `run_parallel_queue_ref`

**Where.**
- `src/parallel.rs`, `run_parallel_queue_ref` (near line 868) creates
  `stores.make_worker_slot_dispenser()`, an `Arc<AtomicU16>` shared by all
  workers.
- `src/database/allocation.rs`, `Stores::database_named` (near line 677), runs
  this on every named allocation in a worker:

  ```rust
  let slot = if let Some(dispenser) = self.worker_slot_dispenser.clone() && … {
      let idx = dispenser.fetch_add(1, Ordering::Relaxed);
  ```

**Why it matters.**  Each named allocation writes the shared `ArcInner` three
times: the `Arc::clone` increment, the `fetch_add`, and the decrement when the
clone drops.  The strong count and the atomic sit in the same cache line, so
every worker writes one shared line.  If a record-returning worker allocates a
store per element, that line is contended on every element.  This is true
sharing, not false sharing, but the stall mechanism is the same.

**Task.**
1. Build a probe: a record-returning worker over 10⁵–10⁶ elements, at 1, 2, 4
   and 8 threads.
2. Positive control: a variant that allocates no store per element.
3. Run `perf c2c record` / `perf c2c report` on Linux.  Record whether the
   `ArcInner` line appears as a HITM hotspot, and plot wall-clock scaling
   against thread count for both variants.
4. Report the numbers.  **Do not change the dispenser.**  Possible directions
   for the owner, not to be implemented: borrow instead of cloning the `Arc`,
   and dispense index blocks per worker.

## F2 — placeholder growth for other workers' indices (hypothesis)

**Where.**  The same branch of `database_named`:

```rust
while self.allocations.len() <= idx as usize {
    self.allocations.push(Store::new(100));
}
```

**Why it matters.**  A worker extends its own `allocations` up to every index
the dispenser handed out, including indices that belong to other workers.  With
T workers each allocating per element, each worker may push placeholders for
roughly all stores allocated so far across all workers.  That is O(total) per
worker, so O(T × total) placeholder work overall.  Whether `Store::new(100)`
allocates eagerly (100 words, 800 B) decides whether this costs memory or only
a `Vec` push.

**Task.**
1. Read `Store::new` and determine whether it allocates.
2. Instrument the placeholder count per worker in the F1 probe.
3. Report the peak RSS and the placeholder count against thread count.  If the
   count grows with T for a fixed input, this is a scaling issue independent of
   F1.

## F3 — `u16` slot space under the dispenser (verify correctness)

**Where.**  The dispenser is an `AtomicU16`, and `fetch_add` wraps silently.
`database_named` caps slot 65535 (#306, "Fail loudly") after the index is
chosen.

**Question.**  Can a record-returning `par` whose results persist past the join
(`worker_allocated_indices`, swapped back at join) exceed about 65k dispensed
indices?  If so, check what happens:
- Does the #306 cap fire on the wrapping thread?
- Can another worker receive a wrapped low index before any thread panics,
  colliding with a parent store (floor `parent_len + 1`)?

**Task.**
1. Build a cell with a record-returning `par` over ~70 000 elements, on both
   backends.
2. Expected: a loud, deterministic failure, or success if stores are reused.
3. A silent wrong value or a SIGSEGV is an issue to open, with the cell as its
   guard.  Also check whether the non-dispenser path
   (`self.allocations.len() as u16`) has the same truncation.

## F4 — contention on the read-only borrow (verify)

**Question.**  Does any read path on a borrowed store
(`borrow_locked_for_light_worker`) write shared memory: a claim check, a budget
counter (`store_budget`), a statistics field, a lazy-binding touch?  TSan
reports races, not benign shared atomic writes, so the current TSan-clean
result does not answer this.

**Task.**
1. Build a `par` whose workers only read a large captured structure (≥ 50 MB),
   at 1–8 threads.
2. Run `perf c2c`.  The expectation is no HITM on parent store memory or on
   `Store` headers.
3. For the positive control, a worker that writes its own scratch in a tight
   loop should show only worker-local lines.

## F5 — skewed per-element cost (observation only)

Fixed contiguous ranges keep locality, but they leave threads idle when the
cost is concentrated in one range.  Measure one skewed cell (for example,
element cost ∝ index²) at 4 threads and compare it with the uniform cell.
Record the result in PERFORMANCE.md or the plan tracker.  **No change** — this
is input for a future design discussion.

## D1 — documentation drift in `doc/claude/THREADING.md` (DONE 2026-09-15)

- **P1 is outdated.**  The "P1 Architecture Summary" and "P1 What is Safe"
  sections described `clone_for_worker` with a full `clone_locked` byte copy per
  worker, and `run_parallel_direct` writing through a shared `out_ptr`.  Both are
  now marked historical, with today's borrow-and-batch flow drawn beside them and a
  pointer to § Multi-threading Safety.
- **Contention note.**  § *Cache-line contention* states what the design
  guarantees (parent stores read-only, worker writes worker-local, results batched)
  and lists F1–F4 as open, with F5 as the observation; the 128-byte cache line of
  Apple M-series is noted for the day padding matters.  Numbers are added there as
  each measurement lands.
- **The `Stitch` comment.**  Traced: no production caller consumes `Stitch`;
  `QueueStitch` (`src/native.rs`) is the live dispatch enum and `par_fold` lowers to
  `n_parallel_fold` → `run_parallel_fold`.  THREADING.md's *"backed by
  `Stitch::Reduce`"* was the wrong text and is corrected; the enum comment now names
  what superseded it.

## Deliverable

1. **A report for the owner:** one table per finding (F1–F5) with the cell, the
   control, the backend, the thread counts, the numbers and the verdict
   (confirmed or refuted).
2. **Issues:** one issue per confirmed defect (F3 in particular), each with its
   guard cell.
3. **A docs commit:** a commit for D1 only (done).  Docs are the only code change
   in scope.
4. **Open questions:** a list for the owner, including the fix directions for F1
   and F2 if they are confirmed.
