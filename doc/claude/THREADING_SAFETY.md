# Threading safety — parallel workers and the combined issue index

Split out of [THREADING.md](THREADING.md).

This document records safety analyses of the runtime memory model with
design-level mitigations for each identified risk.

- **Part 1** (this file) covers the parallel worker system (`src/parallel.rs`,
  `src/database/allocation.rs`, `src/store.rs`).
- **Part 2** covers the coroutine system (`src/state/mod.rs`, `CoroutineFrame`,
  `stack_bytes`, `text_owned`); it lives in
  [THREADING_SAFETY_COROUTINES.md](THREADING_SAFETY_COROUTINES.md).

---

## Contents

### Part 1 — Parallel Workers and Store Allocation
- [P1 Architecture Summary](#p1-architecture-summary)
- [P1 What is Safe](#p1-what-is-safe)
- [P1-R1 — Silent data loss in release builds](#p1-r1--silent-data-loss-in-release-builds)
- [P1-R2 — `out_ptr` lifetime not type-enforced](#p1-r2--out_ptr-lifetime-not-type-enforced)
- [P1-R3 — `claims` HashSet overhead in locked clones](#p1-r3--claims-hashset-overhead-in-locked-clones)
- [P1-R4 — `max` cascade panic with freed mid-slots](#p1-r4--max-cascade-panic-with-freed-mid-slots)
- [P1-R5 — No Rust-type-level proof of non-aliasing](#p1-r5--no-rust-type-level-proof-of-non-aliasing)
- [Part 1 Summary Table](#part-1-summary-table)

### Part 2 — Coroutines, Stores, and Strings
- [P2 Architecture Summary](THREADING_SAFETY_COROUTINES.md#p2-architecture-summary)
- [P2 What is Safe](THREADING_SAFETY_COROUTINES.md#p2-what-is-safe)
- [P2-R1 — Text argument `Str` dangles on first resume](THREADING_SAFETY_COROUTINES.md#p2-r1--text-argument-str-dangles-on-first-resume)
- [P2-R2 — `String` objects leaked at exhaustion](THREADING_SAFETY_COROUTINES.md#p2-r2--string-objects-leaked-at-exhaustion)
- [P2-R3 — Text locals have implicit "never freed" invariant](THREADING_SAFETY_COROUTINES.md#p2-r3--text-locals-have-an-implicit-never-freed-between-yield-and-resume-invariant)
- [P2-R4 — `text_positions` inconsistent across yield/resume](THREADING_SAFETY_COROUTINES.md#p2-r4--text_positions-inconsistent-across-yieldresume)
- [P2-R5 — Store-backed `Str` dangles on record delete](THREADING_SAFETY_COROUTINES.md#p2-r5--store-backed-str-dangles-on-record-delete)
- [P2-R6 — Compiler check for `yield` inside `par()` missing](THREADING_SAFETY_COROUTINES.md#p2-r6--compiler-check-for-yield-inside-par-missing)
- [P2-R7 — Exhausted frames never freed](THREADING_SAFETY_COROUTINES.md#p2-r7--exhausted-frames-never-freed)
- [P2-R8 — `DbRef` locals outlive their store across suspension](THREADING_SAFETY_COROUTINES.md#p2-r8--dbref-locals-outlive-their-store-across-suspension)
- [P2-R9 — `e#remove` on a generator iterator corrupts unrelated records](THREADING_SAFETY_COROUTINES.md#p2-r9--eremove-on-a-generator-iterator-corrupts-unrelated-records)
- [P2-R10 — Yielded `Str` value lifetime is not enforced at the consumer](THREADING_SAFETY_COROUTINES.md#p2-r10--yielded-str-value-lifetime-is-not-enforced-at-the-consumer)
- [Part 2 Summary Table](THREADING_SAFETY_COROUTINES.md#part-2-summary-table)

### Combined
- [All Issues — Quick Reference](#all-issues--quick-reference)
- [See also](#see-also)

---

## All Issues — Quick Reference

Effort scale: **XS** < 4 h · **S** 1–2 d · **M** 3–5 d · **L** 1–2 wk · **XL** > 2 wk.
Where two values are shown, the first is the short-term fix and the second is the
full long-term design.

| ID | Severity | Effort | Key files | Short-term action |
|---|---|---|---|---|
| **P1-R1** | medium | S | `store.rs`, `parser/expressions.rs` | Remove `#[cfg(debug_assertions)]` guard on auto-lock; promote dummy-buffer to panic |
| **P1-R2** | low/medium | XS / S | `parallel.rs` | `// SAFETY:` comment + debug assert; replace `spawn` with `thread::scope` |
| **P1-R3** | low | XS | `store.rs`, `database/allocation.rs` | `clone_locked_for_worker` omitting `claims` |
| **P1-R4** | medium | XS / M | `database/allocation.rs` | LIFO debug assert (XS); free-bitmap replacing cascade (M) |
| **P1-R5** | low | M / L | `database/allocation.rs`, `parallel.rs`, `keys.rs` | `WorkerStores` newtype (M); `DbRef` origin tag (L) |
| **P2-R1** | critical | L † | `state/mod.rs` (`coroutine_create`) | Debug assert if text args present; implement `serialise_text_slots` at create |
| **P2-R2** | high | XS † | `state/mod.rs` (`coroutine_return`) | Drain `text_owned` before `stack_bytes.clear()` |
| **P2-R3** | high | L † | `state/mod.rs` (`coroutine_yield`, `coroutine_next`) | Debug assert on text slots; implement CO1.3d atomically |
| **P2-R4** | medium | S | `state/mod.rs`, `data.rs` (`CoroutineFrame`) | Save/restore `text_positions` set on yield/resume (debug only) |
| **P2-R5** | medium | S / S | `state/mod.rs`, `store.rs` | Document rule; pointer-range heuristic in `coroutine_yield` |
| **P2-R6** | medium | S | `parser/collections.rs`, `state/mod.rs` | `inside_par_body` flag + out-of-bounds guard in `coroutine_next` |
| **P2-R7** | low | M | `fill.rs`, `state/mod.rs`, `state/codegen.rs` | `OpFreeCoroutine` emitted at for-loop exit |
| **P2-R8** | medium | M / XL | `store.rs`, `database/`, `state/mod.rs` | Generation counter on `Store`; save+check in frame (M); flow analysis (XL) |
| **P2-R9** | medium | XS | `parser/fields.rs`, `database/search.rs` | Compiler rejection of `e#remove` on generator; guard in `remove()` |
| **P2-R10** | low | XS / XL | docs | Document ownership rule; `iter_text` type (XL language design) |

† P2-R1, P2-R2, and P2-R3 share the CO1.3d implementation (combined effort **L**, 1–2 weeks).
  They must land atomically — partial implementation is more dangerous than none.

---

## Part 1 — Parallel Workers and Store Allocation

---

## P1 Architecture Summary

**HISTORICAL — the audit below was written against the byte-copy design
(`clone_for_worker` + `clone_locked`, one deep copy of every active store per
worker, `run_parallel_direct` writing through a shared `out_ptr`).  That design
was retired by @PLN108 (2026-07-17) and @PLN117; the current mechanism is
described in § Multi-threading Safety above, and its risks are the P1-R rows,
which stay as the record of what was checked.**  Today's flow:

```
main thread Stores
    └── clone_for_light_worker()    — BORROW: every parent store shared read-only
                                       (captured state is provably unwritten, C93)
            ├── parent slots  → shared ptr, read_only:true, borrowed:true (Drop skips dealloc)
            └── freed slots   → Store::new(100)  (fresh, so a worker may re-initialise it)
    └── one rayon task per worker (`parallel_workers`, contiguous range per thread)
            ├── 4 × 1 024-word scratch stores minted INSIDE the task
            ├── State::new_worker(worker_stores, …)
            └── results pushed to the worker's own Vec
    └── join, then `merge_batches` copies every batch into place sequentially
```

The record-returning path (`run_parallel_queue_ref`) additionally hands each
worker a shared `Arc<AtomicU16>` slot dispenser so that stores a worker mints
survive the join under distinct indices (`worker_allocated_indices`); its
contention and its `u16` ceiling are the open measurements in § Cache-line
contention.


---

## P1 What is Safe

**HISTORICAL — these six properties were verified for the byte-copy design.
Under the borrow (§ Multi-threading Safety) the first three are replaced by the
read-only borrow's own guarantees (compiler-carried parent-unwritten, `read_only`
runtime write-panic, ASan + TSan clean), the `out_ptr` write of the fourth by
per-worker batches, and the last two still hold as written.**

### Store memory is fully deep-copied

`clone_locked` (`store.rs`) does:

```rust
std::ptr::copy_nonoverlapping(self.ptr, ptr, self.size as usize * 8);
```

Every record, including string data, is copied into a fresh independent
allocation.  Strings are stored inline in the store as 32-bit word-offsets; after
byte-copy the offsets resolve correctly against the clone's own `ptr`.  Workers
never access the original store's memory.

### Worker-owned allocations never overlap cloned stores

When a worker function creates a new struct it calls `Stores::database` on its
private `Stores`.  At clone time `self.max` equals the original value (say *N*)
and `allocations.len() == N`, so the first worker allocation pushes a fresh
unlocked store at index *N* — beyond all locked clones at `0..N-1`.

### Locked clones enforce read-only access in debug builds

Every active store given to a worker is marked `locked: true`.  In debug builds,
`addr_mut`, `claim`, and `delete` all `debug_assert!(!self.locked)` and panic
immediately on any write attempt.  This converts accidental mutations into
fail-fast panics during development.

### Non-overlapping direct writes are safe

`run_parallel_direct` writes results via `out_ptr.add(row_idx * ret_sz)`.
Thread *t* owns indices `[t * n_rows / threads, (t+1) * n_rows / threads)`.
The last thread ends at `(threads * n_rows) / threads == n_rows`, so all ranges
tile `[0, n_rows)` without gaps or overlap.  All threads are joined before the
caller reads the buffer.

### `WorkerProgram` sharing is safe

Bytecode, text, and library are wrapped in `Arc` and never mutated after
construction.  The manual `unsafe impl Send + Sync` is justified by the
read-only invariant.

### Result collection is sequential

Channel-based paths send batches to the main thread after joining workers.
`copy_from_worker` (the store-graft deep-copy used for struct returns) is called
sequentially on the main thread — no concurrent store access.

---

## P1-R1 — Silent data loss in release builds

### Description

If a worker function writes to a field of its locked cloned input store in a
release build, the write is silently discarded into a thread-local 256-byte
dummy buffer.  The worker's computation may then observe stale or wrong data and
return incorrect results **with no error or panic**.

The auto-locking of `const` arguments is currently guarded by
`#[cfg(debug_assertions)]` (`parser/expressions.rs`), so release builds never
auto-lock.  A buggy worker that is supposed to be read-only can silently corrupt
results in production while passing all debug-mode tests.

### Mitigation design

**M1-a — Enable auto-locking unconditionally for `const` worker arguments**

Remove the `#[cfg(debug_assertions)]` guard on the auto-lock insertion in
`parse_code` and `expression` (the two sites that emit `n_set_store_lock` for
`const` parameters and local const variables).

Locking a store is a single flag write.  The only reason it was gated to debug
was to avoid the branch cost on every call; profiling shows the overhead is
negligible compared to the cost of a parallel dispatch.

**M1-b — Promote the write-to-locked-store path in release to a runtime error**

Change the release-build silent-discard path in `addr_mut` from a dummy buffer
return to an explicit `panic!` (or structured error).  The dummy buffer was
added to keep release builds from segfaulting; once M1-a ensures const stores
are always locked, no legitimate code path should hit it.  Making it a visible
failure removes the silent-corruption window.

**M1-c — Add a release-mode integration test**

Add a `#[test]` that compiles and runs with `--release` and asserts the result
of a `par(...)` loop whose worker accidentally writes to its input equals the
expected value.  If M1-a and M1-b are in place the test would panic with a
clear message instead of returning the wrong answer.

---

## P1-R2 — `out_ptr` lifetime not type-enforced

### Description

`run_parallel_direct` accepts a raw `*mut u8`.  The safety invariant — the
buffer must remain live until all threads are joined — is upheld today because
`thread::join` is called before the function returns.  But the Rust type system
does not enforce this.  A future refactor that moves or removes the join (e.g.
to allow early cancellation or deferred result collection) could introduce a
data race or use-after-free with no compile-time warning.

### Mitigation design

**M2-a — Wrap the output slice in a scoped-thread lifetime**

Replace `thread::spawn` with `std::thread::scope` (stabilised in Rust 1.63).
Scoped threads borrow data from the enclosing stack frame, so the compiler
enforces that the buffer outlives all threads:

```rust
std::thread::scope(|s| {
    for t in 0..threads {
        let out_slice = &mut out[t_start * ret_sz .. t_end * ret_sz];
        s.spawn(move || {
            // write into out_slice — lifetime enforced by scope
        });
    }
    // scope end: all threads joined here by the compiler
});
```

This eliminates `SendMutPtr`, the manual join loop, and the lifetime comment.
The only cost is that scoped threads cannot be detached, which is not a
requirement here.

**M2-b — Short term: add a safety comment with an invariant assertion**

Until M2-a lands, add a `// SAFETY:` block above every `SendMutPtr` use stating
the join invariant explicitly, and a `debug_assert!` after the join loop
verifying all handles are consumed.

---

## P1-R3 — `claims` HashSet overhead in locked clones

### Description

`clone_locked` copies `self.claims` (the set of all live record word-offsets,
used by `validate()`).  Workers with locked stores never call `validate()` and
never mutate claims, so the clone is wasted memory — O(*records*) allocation per
worker per `par(...)` call.

For programs with many long-lived records this can add measurable allocation
pressure when spawning many workers.

### Mitigation design

**M3-a — Skip `claims` in worker clones**

Add a constructor parameter or a dedicated `clone_for_worker` method on `Store`
that omits the `claims` clone:

```rust
pub fn clone_locked_for_worker(&self) -> Store {
    Store {
        claims: HashSet::new(),   // empty — workers never validate
        locked: true,
        free_root: 0,
        // … rest same as clone_locked
    }
}
```

`clone_for_worker` in `Stores` would call this variant instead of `clone_locked`.
The existing `clone_locked` (used elsewhere) is unchanged.

---

## P1-R4 — `max` cascade panic with freed mid-slots

### Description

**Reproducer:**
1. Original `Stores` has slots: 0 = live, 1 = freed, 2 = live (so `max = 3`,
   `allocations[1].free = true`).
2. `clone_for_worker` produces: 0 = locked_clone, 1 = fresh_free, 2 = locked_clone,
   `max = 3`.
3. Worker calls `database()` → pushes slot 3 (new, unlocked, `max = 4`).
4. Worker calls `free(slot_3)` → marks slot 3 free, cascade: `max` tries `4 → 3`,
   slot 2 has `free = false` (it is a locked clone), cascade stops at `max = 3`.
5. Worker calls `database()` again → `self.max (3) >= allocations.len() (4)` is
   false, so it calls `allocations[3].init()`.  Slot 3 was already freed (step 4
   set `free = true`), so `init()` succeeds and `max` becomes 4 again.

Wait — step 5 actually works if the slot is truly free.  The real failure path is
when the cascade overshoots into a locked-clone slot:

- Original: 0 = live, max = 1.  Worker slot push: max = 2 (slot 1 = worker).
- Worker frees slot 1: `max = 1`, cascade: slot 0 is `free = false` → stops.
  OK so far.
- Worker pushes again: `max (1) >= len (2)` → false → `allocations[1].init()`.
  Slot 1 (previous worker allocation, now freed) has `free = true` → assert
  passes.  `max = 2`.  OK.

The failing case requires the cascade to reach a slot with `free = false` that
is *below* `max` in the worker's view.  That happens when a worker frees its
*only* worker-created slot and the cascade hits slot `max-1` which is a locked
clone:

- Original: 0 = live (locked), `max = 1`.  Worker creates slot 1 (`max = 2`).
  Worker frees slot 1: `max → 1`.  Cascade checks slot 0: `free = false` →
  stops.  `max = 1`.
- Worker creates slot 1 again: `max (1) < len (2)` → `init()` slot 1.
  Slot 1 is `free = true` (from step above) → assert passes.  OK.

So the cascade itself is safe in the current logic.  The real panic would be:
- Worker frees a slot that has `store_nr < max - 1` (non-LIFO), triggering the
  LIFO debug assert `"Double free store"` or the `al == self.max - 1` check
  causing `max` *not* to decrement.

The LIFO-order requirement on `free()` is documented but not enforced in non-debug
builds.  When native-codegen code (`OpFreeRef`) frees stores out of order, `max`
stalls, slots leak, and subsequent `database()` calls eventually try to allocate
a slot that `free == false`.

### Mitigation design

**M4-a — Enforce LIFO order via a debug-build audit log**

The existing `LOFT_STORE_LOG` env-var logs alloc/free events.  Add a
`debug_assert` in `free_named` that verifies the freed slot equals `self.max - 1`
(strict LIFO), and emit the full alloc/free trace to a thread-local buffer on
violation so the error message shows which store broke ordering.

**M4-b — Replace LIFO scan with a free-bitmap**

Replace the `while max > 0 && allocations[max-1].free { max -= 1; }` cascade
with a bitset (`u64` array) tracking which slots are free.  `database` finds the
lowest free bit; `free` sets the bit.  `max` tracks the highest live slot for
boundary checks:

```
free_bits: [u64; MAX_STORES / 64]  — bit set = slot is free
max: u16                            — highest ever used index + 1
```

`database`:
1. Find lowest set bit in `free_bits` below `max` (first reuse slot).
2. If none, grow `max` and use the new slot.
3. Clear the bit, set `store.free = false`.

`free`:
1. Set bit for `store_nr`.
2. If `store_nr == max - 1`, trim `max` down to the highest cleared bit.

This eliminates the LIFO requirement entirely, makes store reuse O(1), and
removes the fragile cascade logic.  A worker creating and freeing stores in any
order would work correctly.

**M4-c — Short term: document the LIFO invariant prominently**

Until M4-b lands, add a `// INVARIANT: free() must be called in LIFO order`
comment in `free_named`, and assert it in debug builds (`al == self.max - 1`).

---

## P1-R5 — No Rust-type-level proof of non-aliasing

### Description

The architecture relies on:
1. The loft compiler enforcing `const` on worker arguments.
2. The runtime store lock catching violations in debug builds.
3. Convention that worker functions "may not write to shared state".

Rust's type system does not prevent a worker closure from capturing a `*mut`
pointer to main-thread data and writing through it, nor does it prevent a worker
from holding a `DbRef` whose `store_nr` belongs to the main thread.

This is acceptable for the current architecture but is an invariant that can
silently break if the parallel dispatch is extended (e.g. to allow workers to
receive mutable references for output accumulation).

### Mitigation design

**M5-a — Encode worker-store ownership in a newtype**

Introduce a `WorkerStores(Stores)` newtype that:
- Can only be constructed by `clone_for_worker` (private constructor).
- Exposes only `&Stores` (immutable) to the main thread after workers finish,
  never `&mut`.
- Is `Send` but not `Sync`, ensuring it cannot be shared across threads.

Worker closures receive `WorkerStores`; they can allocate into their private
portion but cannot be handed a raw pointer back to the main thread's stores.

**M5-b — Mark `DbRef` values from main-thread stores with a lifetime or tag**

Long term: add a `origin: StoreOrigin` field to `DbRef` (or an index range
`[0, worker_base)` vs `[worker_base, …]`) so that the runtime can assert in
debug mode that a worker does not store a main-thread `DbRef` into a result that
will be merged back, bypassing the `copy_from_worker` deep-copy path.

---

## Part 1 Summary Table

| Risk | Severity | Effort | Short-term fix | Long-term design |
|---|---|---|---|---|
| P1-R1 — Silent write-discard in release | **medium** | S | Remove `#[cfg(debug_assertions)]` guard on auto-lock | Promote dummy-buffer path to panic (M1-b), add release integration test (M1-c) |
| P1-R2 — `out_ptr` lifetime not type-enforced | **low/medium** | XS / S | ✓ S29: `thread::scope` (M2-a) + `// SAFETY:` comment (M2-b) in `run_parallel_direct` | Done |
| P1-R3 — `claims` cloned into locked workers | **low** | XS | ✓ S29: `clone_locked_for_worker` omits `claims` (M3-a) | Done |
| P1-R4 — LIFO violation stalls `max` / panic | **medium** | XS / M | ✓ S29: free-bitmap M4-b supersedes LIFO assert; non-LIFO frees now safe | Done |
| P1-R5 — No type-level non-aliasing proof | **low** | M / L | ✓ S30: `WorkerStores` newtype (M5-a) | `DbRef` origin tagging (M5-b) remains long-term |

---

## See also

- [THREADING.md](THREADING.md) — `par(...)` syntax, `parallel_for` desugaring, worker rules
- [THREADING_SAFETY_COROUTINES.md](THREADING_SAFETY_COROUTINES.md) — Part 2 (coroutines)
- [DATABASE.md](DATABASE.md) — `Stores`, `Store`, `DbRef`, locking API
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/store.rs`, `src/state/mod.rs`
