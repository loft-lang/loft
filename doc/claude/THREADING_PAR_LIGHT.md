# `par_light` — Lightweight Parallel For-Loop

A variant of `par(...)` that eliminates the per-thread `clone_for_worker()` cost
by borrowing the main thread's stores read-only and using a pre-allocated store pool
instead of cloning.

---

## Contents

- [Motivation](#motivation)
- [Constraint — Non-recursive workers](#constraint--non-recursive-workers)
- [Core Design](#core-design)
  - [Shallow locked borrow](#shallow-locked-borrow)
  - [Pre-allocated store pool](#pre-allocated-store-pool)
  - [`clone_for_light_worker`](#clone_for_light_worker)
- [Compiler Analysis](#compiler-analysis)
  - [Call-graph reachability](#call-graph-reachability)
  - [Store-count computation (M)](#store-count-computation-m)
  - [Validation errors](#validation-errors)
- [Loft Syntax](#loft-syntax)
- [Runtime Changes](#runtime-changes)
  - [`WorkerPool`](#workerpool)
  - [`run_parallel_light`](#run_parallel_light)
  - [`State::new_light_worker`](#statenew_light_worker)
- [Implementation Steps](#implementation-steps)
- [Safety Analysis](#safety-analysis)
- [See also](#see-also)

---

## Motivation

Every `par(...)` call today pays `clone_for_worker()` once per worker thread.
That function deep-copies every active `Store` buffer:

```
clone_for_worker()
  for each active slot s:
    s.clone_locked_for_worker()     ← full byte copy of s.ptr[0..s.size*8]
  + types.clone()                   ← schema Vec
  + names.clone()                   ← schema Vec
```

Cost: **O(N\_threads × total\_store\_bytes)** of heap allocation and memcopy, paid
before any worker executes a single bytecode instruction.

For workloads where:
- The worker only **reads** data from the input stores (never writes to shared state)
- The worker allocates at most **M** new stores, where M is bounded and statically
  known (no recursive allocations)
- The return type is a primitive or small struct

…the entire deep copy is unnecessary.  `par_light` eliminates it.

---

## Constraint — Non-recursive workers

A worker is eligible for `par_light` if and only if **no store allocation occurs on
any cycle in the worker's call graph**.

Concretely: the compiler builds the call graph of all functions transitively reachable
from the worker.  It then checks every cycle (directly or mutually recursive function
set).  If any function on a cycle contains `OpNewRef` (store allocation), `par_light`
is rejected for that worker with a clear diagnostic.

Workers that allocate stores in non-recursive (leaf or tree-shaped) calls are
accepted.  The maximum number of simultaneously live stores across all such paths is
computed as **M** (the pool size per worker thread).

Examples:

```loft
// ACCEPTED — allocates a store but no recursion
fn summarise(r: const Batch) -> Summary {
    s = new Summary;    // allocates 1 store
    s.total = r.count;
    s
}

// ACCEPTED — calls helper that allocates; neither function is recursive
fn helper(r: const Row) -> Stats { s = new Stats; ... s }
fn process(r: const Row) -> integer { st = helper(r); st.value }

// REJECTED — recursive function allocates a store
fn build_tree(depth: integer) -> Node {
    n = new Node;            // store allocation inside a recursive function
    if depth > 0 {
        n.left = build_tree(depth - 1);   // recursive call
    }
    n
}
```

---

## Core Design

### Shallow locked borrow

Instead of copying a store's buffer, `par_light` creates a **shallow locked borrow**:
a `Store` struct that shares the main thread's backing buffer pointer but has
`locked = true` to block all writes.

```rust
impl Store {
    /// Create a read-only view of this store for a light worker.
    ///
    /// # Safety
    /// Caller must ensure:
    /// 1. The original `Store` outlives all threads that hold the borrow
    ///    (guaranteed by `thread::scope`).
    /// 2. No one writes to the original buffer while the borrow exists
    ///    (guaranteed by main thread being blocked in `thread::scope`).
    pub unsafe fn borrow_locked_for_light_worker(&self) -> Store {
        Store {
            ptr:       self.ptr,          // shared pointer — O(1), no copy
            claims:    HashSet::new(),    // no claim tracking for borrowed stores
            size:      self.size,
            free:      false,
            locked:    true,              // all writes blocked
            free_root: self.free_root,
            #[cfg(debug_assertions)]
            generation: self.generation,
            #[cfg(feature = "mmap")]
            file: None,                   // mmap not shared
        }
    }
}
```

Cost per store: **one struct copy (~48 bytes), zero heap allocation**.
Compare to `clone_locked_for_worker`: O(store.size × 8) bytes of heap allocation +
memcopy.

Because workers have `locked = true`, any write attempt panics in debug builds and is
silently discarded in release builds — the existing `locked` enforcement path covers
this without new code.

The borrowed `Store` **must not be `Drop`ped** in the normal way (its `Drop` impl calls
`dealloc` on `ptr`, which belongs to the main thread).  This is handled by a
`ManuallyDrop<Store>` wrapper in the light worker's `Stores`, or by a sentinel `free =
true` that suppresses the dealloc in `Drop`.

### Pre-allocated store pool

The main thread owns a `WorkerPool` containing `n_workers × M` fresh `Store` objects.
These are allocated once (at first `par_light` call or at interpreter startup) and
**reused** across `par_light` invocations by calling `store.init()` to reset each one.

Worker `i` gets exclusive access to the slice
`pool.stores[i × M .. (i+1) × M]`.
Because thread indices are disjoint and `thread::scope` prevents overlap between
invocations, no synchronisation is needed.

The worker's `Stores` is constructed with:
- Slots `0 .. main.max`: shallow locked borrows of main stores (read-only input data)
- Slots `main.max .. main.max + M`: pool stores for the worker's own allocations,
  all marked `free = true` initially

The existing `find_free_slot` / `free_bits` bitmap mechanism allocates pool stores in
LIFO order, exactly as it does today.  No changes to allocation logic are needed.

### `clone_for_light_worker`

New method on `Stores`:

```rust
/// Produce a light-worker view: main stores are borrowed read-only; pool stores
/// provide allocation capacity.
///
/// # Safety
/// `pool_slice` must remain valid and exclusively owned by this worker for the
/// duration of the thread scope.
pub unsafe fn clone_for_light_worker(
    &self,
    pool_slice: &mut [Store],
) -> WorkerStores {
    let mut allocations: Vec<Store> = self.allocations[..self.max as usize]
        .iter()
        .map(|s| {
            if s.free {
                // Freed main-thread slot: create a tiny sentinel (no allocation).
                Store::new_freed_sentinel()
            } else {
                // Active main-thread slot: shallow locked borrow.
                // SAFETY: covered by thread::scope contract (see above).
                unsafe { s.borrow_locked_for_light_worker() }
            }
        })
        .collect();

    // Append pre-allocated pool stores as free slots available to the worker.
    for store in pool_slice.iter_mut() {
        store.init();          // reset to empty
        store.free = true;
        allocations.push(/* move out of pool slice — see pool design */);
    }

    // Build free_bits: main-thread freed slots + all pool slots.
    let free_bits = build_free_bits(&allocations, self.max);

    WorkerStores::new(Stores {
        types:              self.types.clone(),   // schema (small, immutable)
        names:              self.names.clone(),   // schema (small, immutable)
        allocations,
        max:                self.max + pool_slice.len() as u16,
        free_bits,
        files:              Vec::new(),
        scratch:            Vec::new(),
        last_parse_errors:  Vec::new(),
        parallel_ctx:       self.parallel_ctx.clone(),   // inherited — see below
        logger:             self.logger.clone(),
        had_fatal:          false,
        #[cfg(not(feature = "wasm"))]
        start_time:         self.start_time,
        #[cfg(feature = "wasm")]
        start_time_ms:      self.start_time_ms,
        call_stack_snapshot: Vec::new(),
    })
}
```

**Cost**:
- `borrow_locked_for_light_worker` per active slot: O(active\_stores) struct copies, no heap
- `init()` per pool slot: O(M) zeroing operations (pool stores already allocated)
- `types.clone()` + `names.clone()`: O(schema\_size) — modest, unavoidable
- **Zero** large buffer copies — all store data stays in main-thread memory

Compare to `clone_for_worker`: O(N\_threads × Σ store\_sizes) buffer copies.

---

## Compiler Analysis

### Call-graph reachability

At `par_light(b = worker(a), N)` parse time, the compiler:

1. Resolves the worker function (`n_<worker>`).
2. Does a depth-first walk of the call graph (all `Value::Call`, `Value::CallRef`,
   `Value::Method` nodes reachable from the worker's body).
3. Detects cycles using a visited set.
4. For every function on a cycle: scans its body for `OpNewRef`.  If found → error.

This is already possible with the existing `Data` / `Value` IR — no new infrastructure
needed.  The walk is bounded by the size of the program.

### Store-count computation (M)

After confirming no recursive allocations, compute `M`:

```
M = max simultaneously-live reference-type slots in any execution path
    through the worker's call tree (excluding cycles).
```

Practically: a DFS through the acyclic call graph, tracking the count of
simultaneously live `reference`-type variables at each point (similar to existing
live-range analysis in `src/variables/validate.rs`).  Take the maximum across all
paths.

M is typically 0–3 for real workloads.

The pool pre-allocates `M + 1` stores per worker thread (the `+1` is for the
worker's execution stack store, which is always needed).

### Validation errors

| Condition | Error message |
|---|---|
| Worker calls itself (directly recursive) + allocates | `"par_light: worker '<name>' allocates a store inside a recursive call; use par() instead"` |
| Mutually recursive functions + allocation on cycle | `"par_light: recursive cycle through '<f1>' → '<f2>' allocates a store; use par() instead"` |
| Return type is `text` | `"par_light: text return requires par() (use par_light only for primitive or struct returns)"` |

---

## Loft Syntax

`par_light` is a separate loop clause, distinct from `par`:

```loft
for a in vector par_light(b = worker(a), N) {
    // b holds the worker result for element a
    // syntax identical to par(...) — only the clause keyword differs
}
```

The same two worker call forms are supported as in `par`:

| Form | Example |
|---|---|
| Form 1 | `worker(a)` — global/user function |
| Form 2 | `a.method()` — method on element type |

The compiler desugars `par_light` identically to `par`, except it emits
`n_parallel_for_light_d_nr` instead of `n_parallel_for_d_nr`.

---

## Runtime Changes

### `WorkerPool`

New struct, owned by `State` (or passed in to `run_parallel_light`):

```rust
pub struct WorkerPool {
    /// Flat store buffer: n_workers × stores_per_worker stores.
    /// Worker i owns stores[i * spw .. (i+1) * spw].
    stores: Vec<Store>,
    stores_per_worker: usize,
    n_workers: usize,
}

impl WorkerPool {
    pub fn new(n_workers: usize, stores_per_worker: usize, store_capacity: u32) -> Self {
        let total = n_workers * stores_per_worker;
        let stores = (0..total).map(|_| Store::new(store_capacity)).collect();
        WorkerPool { stores, stores_per_worker, n_workers }
    }

    pub fn slice_mut(&mut self, worker_idx: usize) -> &mut [Store] {
        let spw = self.stores_per_worker;
        &mut self.stores[worker_idx * spw .. (worker_idx + 1) * spw]
    }
}
```

The pool is created once.  Between `par_light` invocations each store is reset with
`init()` inside `clone_for_light_worker` — no re-allocation.

### `run_parallel_light`

Drop-in for `run_parallel_direct` / `run_parallel_raw` for the light case.

```rust
pub fn run_parallel_light(
    stores: &Stores,          // borrowed read-only; outlives scope by thread::scope contract
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    return_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    out_ptr: *mut u8,
    n_rows: usize,
    pool: &mut WorkerPool,
) {
    let threads = n_threads.max(1).min(n_rows);
    let program = Arc::new(program);
    let out = Arc::new(SendMutPtr(out_ptr));

    thread::scope(|s| {
        for t in 0..threads {
            let start = t * n_rows / threads;
            let end   = (t + 1) * n_rows / threads;
            // SAFETY: thread::scope ensures stores outlives all threads.
            let worker_stores = unsafe {
                stores.clone_for_light_worker(pool.slice_mut(t))
            };
            let prog    = Arc::clone(&program);
            let out_t   = Arc::clone(&out);
            let input_t = *input;
            let extras  = extra_args.to_vec();
            let ret_sz  = return_size as usize;

            s.spawn(move || {
                let mut state = prog.new_state(worker_stores);
                for row_idx in start..end {
                    let row_ref = vector::get_vector(
                        &input_t, element_size,
                        row_idx as i32, &state.database.allocations,
                    );
                    let val = state.execute_at_raw(fn_pos, &row_ref, &extras, ret_sz as u32);
                    unsafe {
                        let dst = out_t.0.add(row_idx * ret_sz);
                        std::ptr::copy_nonoverlapping(
                            (&raw const val).cast::<u8>(), dst, ret_sz,
                        );
                    }
                }
            });
        }
    });
}
```

### `State::new_light_worker`

No change needed — `WorkerStores` produced by `clone_for_light_worker` is structurally
identical to one produced by `clone_for_worker`.  `State::new_worker` accepts it
unchanged.  The only runtime difference is that borrowed store slots have `locked =
true`, which is already enforced by the existing write-guard path.

---

## Implementation Steps

Each step is independently testable.

### Step L1 — `Store::new_freed_sentinel` and `borrow_locked_for_light_worker`

Add the two new `Store` constructors.  Add a unit test that:
- Creates a `Store`, writes some data, calls `borrow_locked_for_light_worker`
- Verifies reads return the same data
- Verifies writes panic (debug) or are silently discarded (release)
- Verifies `Drop` of the borrow does not free the buffer

**Pass**: unit test green.

### Step L2 — `WorkerPool`

Add `WorkerPool` struct and `new` / `slice_mut` methods.  Add a unit test that:
- Creates a pool for 4 workers × 3 stores each
- Each worker's `slice_mut` is disjoint
- After `init()`, each pool store can `claim` and `free` normally

**Pass**: unit test green.

### Step L3 — `clone_for_light_worker`

Add `Stores::clone_for_light_worker`.  Add a unit test that:
- Creates a `Stores` with two active stores containing known data
- Calls `clone_for_light_worker` with a 2-store pool
- Verifies the worker can read all original data
- Verifies the worker can `database()` into pool slots and use them
- Verifies writing to borrowed slots panics/is discarded

**Pass**: unit test green.

### Step L4 — `run_parallel_light`

Add the `run_parallel_light` function.  Verify with an existing `par_int` test vector
by running it through `run_parallel_light` with a pool.  Assert identical results to
`run_parallel_direct`.

**Pass**: results identical to `par()` for a simple integer worker.

### Step L5 — Compiler call-graph analysis

Add `check_light_worker(worker_fn_nr, data) -> Result<usize, String>`:
- Returns `Ok(M)` (stores_per_worker) or `Err(diagnostic)`.
- Unit tests: accepted worker, directly recursive worker, mutually recursive cycle.

**Pass**: unit tests for all three cases.

### Step L6 — Parser: `par_light` clause

Wire `par_light(...)` in `parse_parallel_for` (or a sibling function):
- Parse like `par(...)` but call `check_light_worker` and emit
  `n_parallel_for_light_d_nr`.
- Attach `M` and `n_threads` to the emitted call so the runtime can allocate the pool.

**Pass**: `par_light` parses, compiles, and produces correct results on the standard
`par` examples (`tests/threading.rs`).

### Step L7 — Performance benchmark

Add a benchmark comparing `par()` vs `par_light()` on a large vector (≥ 100k elements)
with an integer-returning worker.  Measure wall time and compare.

Expected: `par_light` is measurably faster when total store bytes are large (the gain
scales with total active store buffer size, not element count).

**Pass**: benchmark runs; result documented in PERFORMANCE.md.

---

## Safety Analysis

| Risk | Mitigation |
|---|---|
| Borrowed `Store` outlives main-thread buffer | `thread::scope` join guarantees all workers finish before `clone_for_light_worker` returns and before main-thread `Stores` can be dropped |
| Worker writes to a borrowed store | `locked = true` → panic in debug, silent discard in release (existing path, no new code) |
| Two workers share a pool slice | `pool.slice_mut(t)` hands out disjoint slices by construction; `thread::scope` prevents reuse during the scope |
| Worker borrows `Drop`s main buffer | Borrowed `Store` must either be `ManuallyDrop` or have a sentinel that skips `dealloc` in `Drop`.  Step L1 tests this explicitly |
| Compiler misses a cycle | Call-graph DFS is exhaustive over all `Value::Call` / `Value::CallRef` nodes; no inlining heuristic needed |
| M under-counted | `+1` safety margin in pool allocation; pool-exhaustion falls back to fresh `Store::new` with a debug warning |

---

## See also

- [THREADING.md](THREADING.md) — `par(...)` design; thread safety in [THREADING_SAFETY.md](THREADING_SAFETY.md)
- [PLANNING.md](PLANNING.md) — A14 item for this feature
- [PERFORMANCE.md](PERFORMANCE.md) — benchmark data once Step L7 is complete
- `src/parallel.rs` — existing `run_parallel_direct` / `run_parallel_raw`
- `src/store.rs` — `Store::new`, `clone_locked_for_worker`, `locked` enforcement
- `src/database/allocation.rs` — `clone_for_worker`, `find_free_slot`, `free_bits`
