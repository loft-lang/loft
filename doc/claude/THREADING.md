
# Threading Interface

## Contents
- [Current State](#current-state)
- [`fn` Expression](#fn-expression)
- [`parallel_for` Call Rewriting](#parallel_for-call-rewriting)
- [Runtime](#runtime)
- [Compiler Validation Summary](#compiler-validation-summary)
- [`par(...)` Parallel For-Loop Syntax](#par-parallel-for-loop-syntax)

---

## Current State

The public API for parallel execution is the `par(...)` for-loop clause.  The internal functions `parallel_for_int`, `parallel_for`, and `parallel_get_*` are declared without `pub` in `default/01_code.loft` and must not be called directly from user code.

Function references (`fn <name>`) are now fully first-class (T1-1 complete): they can be stored in variables of type `fn(T) -> R`, passed as parameters, and called directly via `f(args)`. See the [`fn` Expression](#fn-expression) section for details.

### `par(...)` Parallel For-Loop (public API)

See the [par(...) Parallel For-Loop Syntax](#par-parallel-for-loop-syntax) section below.

### Internal Primitives (not public)

#### `parallel_for_int`

```loft
fn parallel_for_int(func: text, input: reference,
                    element_size: integer, threads: integer) -> reference
```

Legacy internal: function name is a runtime string (no compiler check), return type is always `integer`, element size must be supplied manually.

#### `parallel_for` (compiler-checked, internal)

```loft
fn parallel_for(input: reference, element_size: integer, return_size: integer,
                threads: integer, func: integer) -> reference
```

Emitted by the compiler when rewriting `par(...)` clauses.  The user-facing form is the `par(...)` clause; this function is not callable directly.

Worker rules: see `par(...)` Parallel For-Loop Syntax below.

---

## `fn` Expression

A `fn <name>` expression in value position produces a `Type::Function(args, ret)` value.  The runtime representation is the definition number (`d_nr`) stored as an `i32`.

```loft
f = fn double_score;   // type: fn(const Score) -> integer
                       // runtime value: d_nr of double_score
```

**Compile-time resolution:**
- Tries `n_<name>` first (user function naming convention).
- Falls back to bare `<name>` (methods, operators).
- Emits a diagnostic error if neither resolves.
- The `Type::Function` carries full argument type and return type metadata.

**No new bytecode opcode** — compiles to `OpInt(d_nr)`.

**Callable fn-ref variables (T1-1, complete):** A local variable or parameter of type
`Type::Function` can be called directly: `f(args)`. `parse_call` detects the
`Type::Function` case and emits `Value::CallRef(var_nr, args)` instead of `Value::Call`.
At bytecode generation `generate_call_ref` emits `OpCallRef` (op_code 252). The runtime
looks up the entry point in `State::fn_positions` and dispatches via `fn_call`.

`fn(T) -> R` is also a valid parameter type, enabling higher-order functions:
```loft
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
```

---

## `parallel_for` Call Rewriting

The parser special-cases calls to `parallel_for` in `parse_call` (similar to `assert`).  After collecting the argument list, it calls `parse_parallel_for` which:

1. Verifies `types[0]` is `Type::Function(args, ret)` (produced by `fn <name>`).
2. Verifies `types[1]` is `Type::Vector(T, _)`.
3. Checks worker return type is a supported primitive.
4. Validates extra arg count == worker's extra param count.
5. Computes `element_size = database.size(T.known_type)` (actual inline storage size).
6. Computes `return_size` (1/4/8 bytes).
7. Emits `Value::Call(n_parallel_for_d_nr, [input, elem_size, return_size, threads, func])`.

The internal native function `n_parallel_for` (registered in `native.rs::FUNCTIONS`) has the loft declaration:

```loft
fn parallel_for(input: reference, element_size: integer, return_size: integer,
                threads: integer, func: integer) -> reference;
```

`input` is listed first so that `gather_key` in `generate_call` does not misread the integer `func` d_nr as a key count.

---

## Runtime

### `execute_at_raw` (state.rs)

```rust
pub fn execute_at_raw(&mut self, fn_pos: u32, arg: &DbRef, return_size: u32) -> u64
```

Sets up the same `[arg: DbRef][return-addr u32::MAX]` stack layout as `execute_at`.  After execution, pops the result using the correct width:

| return_size | pop method | Rust type |
|---|---|---|
| 8 | `get_stack::<u64>()` | `i64`/`f64` bit pattern |
| 4 | `get_stack::<u32>()` | `i32` bit pattern |
| 1 | `get_stack::<u8>()` | `bool` as 0/1 |

### `run_parallel_raw` (parallel.rs)

```rust
pub fn run_parallel_raw(
    stores, program, fn_pos, input, element_size, return_size, n_threads
) -> Vec<u64>
```

Generalisation of `run_parallel_int`.  Each worker calls `execute_at_raw` and stores the raw bits in a `u64`.  The main thread assembles results in order.

### `n_parallel_for` (native.rs)

Pops (reverse declaration order): `func`, `threads`, `return_size`, `element_size`, `input`.  Calls `run_parallel_raw`, then builds the result vector:

| return_size | store method |
|---|---|
| 8 | `set_long(rec, fld, bits as i64)` |
| 4 | `set_int(rec, fld, bits as i32)` |
| 1 | `set_byte(rec, fld, 0, bits as i32)` |

### A build without threads still runs `par` (@PLN117)

`threading` off does not mean "no `par`" — it means one thread runs the same
dispatch.  The whole `n_parallel_*` family is registered in **both** builds and
the runtime bodies are shared; the only thing that differs is one function per
dispatch shape:

| shape | with threads | without |
|---|---|---|
| most dispatches | `parallel_workers` — rayon over N slices | one call over the whole range |
| `run_parallel_queue_ref` | `map_workers` — rayon over worker indices | a sequential `map` |

That is the whole difference, and it is why the two agree: `make check-no-threading`
runs the threading scripts on both builds and diffs the output.

This used to be a **silent wrong answer**, which is worse than the missing feature
it looked like.  The family's entries in `native::FUNCTIONS` were all
`#[cfg(feature = "threading")]`, so a build without it registered none of them and
the interpreter's `par` called functions that were not there — returning garbage,
with no error.  `make wasm` ships exactly that configuration, so every `par` in the
browser playground was quietly wrong.  Two sequential dispatch bodies also existed
as *duplicates* of the threaded ones; being unreachable, one of them had gone
wrong unnoticed.  There is now one body per shape.

### Nested `par` — a worker inherits its parent's context

A worker's `Stores` is a light view of its parent's, and it carries the parent's
`ParallelCtx` (bytecode + library + `Data`).  That is what lets a `par` inside a
`par` worker dispatch in turn: the nested dispatch needs the program it is running,
and a worker has no `State::execute()` of its own to install one.

The pointers are safe for a worker for the same reason they are safe for the thread
that set them — **the parent joins its workers before its own `execute()` returns** —
and they are read-only: a worker only reads the bytecode, library and `Data` its
parent is already running.

Inheriting it is the whole mechanism; there is no depth limit and no per-level
bookkeeping.  rayon nests the dispatches on the same pool, so depth K costs no extra
threads.  Before this, the interpreter aborted the run (*"parallel_queue called
outside State::execute()"*) while `--native` computed the right answer — the same
program meaning two different things depending on the backend, and in the browser
(which runs the interpreter) a page that hung.  Locked in by `tests/par_nested.rs`
(depth 2, depth 3, and a text return — each against a hand-computed value, on both
backends) and by the nested cell in `tests/wasm/par-thread-proof.sh`.

### Where the threads come from (@PLN117)

One seam decides it: `parallel.rs::with_pool`.

| Build | Threads | Pool |
|---|---|---|
| native | OS threads | loft's private rayon pool (`rayon_pool()`) |
| browser | Web Workers | the page's GLOBAL rayon pool, installed by `wasm_threads::loft_pool_build` |
| browser, pool not started | none | rayon's single-threaded fallback — `par` runs sequentially, same values |

rayon schedules in every case, so `par` and `par_fold` behave identically
everywhere.  The browser difference is only *where the threads come from*: wasm
has no `thread::spawn`, so the page spawns Web Workers and each one claims a
rayon thread through `loft_rayon_start_worker` (`src/wasm_threads.rs`, host half
in `doc/loft-thread.js`).  Both browser bundles — `loft --html` and the
wasm-bindgen gallery — link that same runtime.

Two things about a browser thread are not like a native one:

- **The main thread may not block.** `memory.atomic.wait32` throws there, and
  rayon takes a lock on the calling thread on every join, so the whole build uses
  `rayon/web_spin_lock` over loft's own `wasm-sync/` (spin instead of park on
  that one thread).
- **A worker starts on the main thread's shadow stack**, because every
  `WebAssembly.Instance` copies the same initial globals.  The host moves each
  worker onto its own stack and TLS block before it runs any wasm.

Details and the in-browser proofs: [WASM.md](WASM.md) §
Threading, and `doc/claude/plans/117-browser-multithreading/`.

**Proving it stays true.** Browser threading cannot be checked by the Rust suite —
it needs a real browser, a COOP/COEP host and a threaded bundle — so five headless
gates carry it: `make par-gates` locally, and the `Browser threading` workflow
(`.github/workflows/browser-threads.yml`) nightly plus on any PR touching the
threading surface.  They measure dispatch (worker count), the shared-memory model,
scaling, UI responsiveness, and the `loft --html` bundle, each against the value the
interpreter produces.  In CI a gate that SKIPs for a missing prerequisite **fails**
(`scripts/par_gates.sh --ci`): the one way this could rot is by quietly not running.

**The nightly toolchain here is a long-term dependency, not a temporary one — do not plan
a stable-only migration around it.**  A threaded browser bundle needs std rebuilt with
`+atomics`, which needs `-Z build-std`, which is nightly.  Checked 2026-08-06: `build-std`
is an accepted 2026 project goal whose scope for the cycle is only *accept the RFCs
([#3874](https://github.com/rust-lang/rfcs/pull/3874) / #3875) and begin implementation*,
with the stabilisation PR still an open task on
[rust#155363](https://github.com/rust-lang/rust/issues/155363) and Cargo-team review
bandwidth named as a delivery risk.  And that goal targets custom / tier-three targets and
targets with **no** pre-compiled std — not our case, which is rebuilding a tier-2 target's
shipped std to flip one target feature.  The mechanism that would cover it (`build-std.when`,
rebuilding when a *target modifier* changes) is a follow-up to those RFCs, so it sits behind
them.  Treat `+atomics` on stable as years out, and keep the nightly leg budgeted.

Note also that stabilisation would not, by itself, buy loft anything: `std::thread::spawn`
does not become a Web Worker under the atomics feature ([WASM.md](WASM.md) § Threading), so
the threads would still have to come from the host — which is what @PLN117 already built.

---

## Compiler Validation Summary

| Check | Location | Error |
|---|---|---|
| `fn <name>` names an existing function | `parse_fn_ref` | `"Unknown function '{name}'"` |
| `fn <name>` resolves to a `DefType::Function` | `parse_fn_ref` | `"'{name}' is not a function"` |
| First `parallel_for` arg is `Type::Function` | `parse_parallel_for` | `"first argument must be a function reference (use fn <name>)"` |
| Second arg is `Type::Vector` | `parse_parallel_for` | `"second argument must be a vector"` |
| Extra arg count matches worker | `parse_parallel_for` | `"wrong number of extra arguments"` |
| Every captured argument but the element is a scalar | `parse_parallel_worker_fn` | `"captured argument '<c>' is a reference (<T>)…"` |
| A worker does not write captured state | `parse_parallel_worker_fn` | `"writes captured '<c>' — a par worker's captured state is READ-ONLY"` |
| Worker's first argument IS the loop element | `parse_parallel_worker_fn` | `"its first argument is the loop element '<a>'"` |
| Worker declares a parameter to receive it | `parse_parallel_worker_fn` | `"it declares no parameters"` |
| Worker's first parameter accepts the element type | `parse_parallel_worker_fn` | `"receives the loop element, but expected <T>, got <U>"` |

**The worker's RETURN type is not restricted.** This table carried a
*"worker return type '…' must be integer, float, or boolean"* row until 2026-09-01; no such
diagnostic exists in `src/`, and `text`, `character`, a struct and `vector<integer>` were all
measured working on both backends. Struct and text results are deep-copied out of the
worker's store ([heap.md](formal/heap.md) `H-Copy`), which is what makes them safe to return.

---

## `par(...)` Parallel For-Loop Syntax

The `par(b=worker(a), N)` clause on a `for ... in` loop is a shorthand that runs the worker in parallel over the source and iterates the results in the source's materialised order — deterministic for an ordered source (`vector` / range / `iterator` / `text` / `sorted`), but **non-deterministic for a `hash`** (see the § below).

### Syntax

```loft
for a in <iterable> par(b=<worker_call>, <threads>) {
    // body — b holds the worker result for element a
}
```

`<iterable>` may be any for-loop source: a `vector<T>`, an integer range
(`0..n`), an `iterator<T>`, text (iterates characters), or a keyed collection
(`hash` / `sorted` / `index` / `spatial`).  Sources that are not already a flat
vector are materialised into one (`materialise_iter_for_par` for ranges /
iterators / text via the comprehension lowering; `materialise_keyed_for_par`
for keyed collections) before the queue dispatcher partitions it across
threads.  A **hash** uses an *unsorted* bucket walk for par (`hash_unsorted`)
since the queue has no use for the hash's key order.  **This walk is NOT
deterministic:** a par loop over a hash yields its results in an order that
**varies run to run** (verified, both backends) — unlike sequential `for x in h`,
which is stably key-ordered.  The queue is order-preserving relative to the
materialised input, but for a hash that materialised order is *itself* unstable,
so **the par result order over a hash is non-deterministic.**  Ordered sources
stay deterministic — a `vector` / range / `iterator` / `text` par preserves the
source order, and a `sorted` par preserves key order (both verified, stable
across runs).  So if you accumulate hash-par results into a vector and need a
stable, reproducible order, par over a `sorted` (or sort the results) instead of
a `hash`.

Two worker call forms are supported:

| Form | Example | Description |
|---|---|---|
| Form 1 | `func(a)` | Global/user function; `a` is the loop element |
| Form 2 | `a.method()` | Method on the element type |

**`a` is not optional and not a placeholder.** The dispatcher passes each element to the
worker itself, so the first argument is the only thing it can be — the loop element,
written out.  Anything else there names a value the worker never receives, and until
loft#1060 it was accepted silently: `func(5)` and `func(other)` ran `func(a)`, and
`func(a.n)` handed the worker the whole record to reinterpret as its parameter's type.
All three are refused now, as is a worker declaring no parameter to receive the element,
and a worker whose first parameter type cannot take it — the check the sequential
`b = func(a)` always made.  Read what you need INSIDE the worker (`func(a)` then `a.n`),
and pass anything else after the element as a context argument.

Form 3 (`c.method(a)` — captured receiver) is detected but not yet implemented.

### Desugaring

```
par_len   = length(vector)
par_results = parallel_for(vector, elem_size, return_size, threads, fn_d_nr)
for b#index in 0..par_len {
    b = parallel_get_T(par_results, b#index)
    <body>
}
```

### Supported return types

`integer`, `float`, `single`, `boolean`, inline `enum`, `text`, and `struct`/reference types.
Extra context arguments are forwarded to workers: `par(b = scale(a, mult), N)` — here `mult`
is an extra argument beyond the element `a`.  **An extra context argument must be a SCALAR**
(`integer` / `float` / `single` / `boolean` / `character` / inline `enum`) — this is load-bearing:
each extra is pushed to the worker as a raw `i64` (`state/mod.rs` `run_parallel_*`, "Push each
extra as a raw i64"), so a struct / vector / text context value is **not** a forwardable extra
arg.  Larger state is not passed as an argument at all — the worker **captures** it (read-only;
see § Multi-threading Safety), which is also cheaper than copying it into every call.
Struct returns use deep-copy (`copy_block` + `copy_claims`) to transfer worker-created
data inline into the result vector; field access on the loop variable works directly.

### Limitations

- Any for-loop iterable is accepted as input (vector, range, `iterator<T>`,
  text, keyed collections) — non-vector sources are materialised first.
- Form 3 (`c.method(a)` — captured receiver) is not yet supported.
- The worker function may not write to shared state.

### Element Size

Element size is computed from `self.database.size(element_type.known_type)` — the actual inline struct field size (e.g. 4 for `Score{value:integer}`, 8 for `Range{lo,hi:integer}`), NOT `size_of::<DbRef>()`.

### Multi-threading Safety

A worker does **not** get a semantically-independent copy of everything to mutate. Captured parent state is **read-only** (@PLN102 C93 — a `ParentWrite` from inside a worker is a compile error, `scopes.rs`), so it is logically **shared**, not owned per-worker; only the worker-mutated stores (the element buffer, the return buffer) need to be per-worker. `Stores::clone_for_light_worker()` implements that: because captured state is *provably unwritten*, a worker **reads the captured stores directly** rather than slicing a large read-only structure into every job. Freed store slots (`.free == true`) are replaced with fresh unlocked `Store::new(100)` instances so that `State::new_worker → Stores::database` can safely re-initialise them without hitting the "Write to locked store" debug assert.

**Read-only sharing (@PLN108, shipped 2026-07-17 — interpreter).** The per-worker byte-copy above is pure conservatism: a worker's captured parent state is read-only (@PLN102 C93 — a `ParentWrite` from a worker is a compile error), so every parent store is provably unwritten for the par's lifetime, and the dispatcher joins all workers before the borrowed parent drops. So `run_parallel_discard` / `run_parallel_queue` now **BORROW** the parent stores read-only (`clone_for_light_worker`: shares each store's `ptr`, `read_only:true`, `borrowed:true` ⇒ `Drop` skips dealloc) instead of copying — a copy-elision, no semantic change. There is **exactly ONE clone path** — always the read-only borrow. The original design auto-selected it by heap size (`par_share_for`, a 2 MB threshold, `LOFT_PAR_SHARE=0`/`=1`); the redesign retired both, so no data-dependent switch decides how a `par` shares (`src/parallel.rs`: *"no byte-copy, no size heuristic, no second `thread::scope` dispatcher"*). Net: a par over a large read-only structure no longer pays a copy of the whole session heap per worker (measured flat vs 53× growth). Safety is compiler-carried (the dispatcher's `&Stores` signature proves parent-unwritten) + `read_only` runtime write-panic, and it is **ASan + TSan clean** (a positive-control race fires, so the clean run is non-vacuous). `--native` par is no longer separate: `src/codegen_runtime.rs`'s dispatchers call the same `parallel::parallel_workers` template, so both backends share the borrow.

### Example

```loft
fn double_score(r: const Score) -> integer { r.value * 2 }
fn get_value(self: const Score) -> integer { self.value }

fn main() {
    q = make_score_items();   // [10, 20, 30]

    // Form 1: global function
    sum = 0;
    for a in q.items par(b=double_score(a), 4) {
        sum += b;   // b = 20, 40, 60  → sum = 120
    }

    // Form 2: method
    total = 0;
    for a in q.items par(b=a.get_value(), 1) {
        total += b;  // b = 10, 20, 30  → total = 60
    }
}
```

### Cache-line contention — what the design guarantees, and what is still unmeasured

By construction the plain return paths share no writable cache line between
cores: parent stores are borrowed READ-ONLY (`clone_for_light_worker`), every
worker's writes land in its own scratch stores (4 × 1 024-word stores allocated
inside its own rayon task), `parallel_workers` gives thread `t` exactly one
contiguous range `[t·n/T, (t+1)·n/T)` with no stealing inside it, and results are
collected per worker into that worker's own `Vec` and copied into place
sequentially after the join by `merge_batches` — so no two cores write
neighbouring result slots.

Four points are NOT yet established by measurement and are recorded as open in
[plans/par-contention-measurement.md](plans/par-contention-measurement.md)
(a read-only review of 2026-09-15; nothing there was run):

- **F1** — the record-returning path (`run_parallel_queue_ref`) hands every worker
  one `Arc<AtomicU16>` slot dispenser, and `Stores::database_named` clones the
  `Arc` and does a `fetch_add` on EVERY named allocation: three writes to one
  shared line per store a worker mints.  True sharing, contended per element if a
  worker allocates a store per element.
- **F2** — the same branch grows a worker's `allocations` with placeholders up to
  every index the dispenser handed out, other workers' included: O(T × total)
  placeholder pushes in the worst case.
- **F3** — the dispenser is `u16` and `fetch_add` wraps silently; a record-returning
  `par` whose results persist past the join may exceed ~65 k dispensed indices.
  Whether the `#306` cap fires first, or a wrapped low index collides with a parent
  store, is a correctness question with a cell to write on both backends.
- **F4** — whether any read path on a borrowed store writes shared memory (a claim
  check, a budget counter, a statistics field).  TSan-clean does not answer it:
  TSan reports races, not benign shared atomic writes.

Each needs a probe with a POSITIVE CONTROL (a deliberately contended cell) under
`perf c2c` before a number is recorded here; a clean result without that control
proves nothing.  If padding ever becomes relevant, note that some CPUs, Apple
M-series included, use 128-byte cache lines.  The fixed contiguous ranges also
leave threads idle when the per-element cost is skewed (F5, an observation for a
future design discussion, not a defect).

### `par_fold` — accumulator shorthand

Plan-06 A5 added a shorthand for the pure-fold pattern (every
element folds into a scalar accumulator with no per-iteration
state):

```loft
total = par_fold(items, 0, |acc, e| acc + e.value, 4)
```

The parser lowers it to the builtin `n_parallel_fold`
(`Parser::parse_par_fold`, `src/parser/builtins.rs`), which both backends
back with `run_parallel_fold` in `src/parallel.rs` (A5 + A5b).  The
`Stitch` enum in that file is NOT what backs it: it is a dead placeholder
(`QueueStitch` is the live dispatch enum) and its own comment says so.
The fused `for ... in ... par(...) { sum += b }` form is the user-facing
alternative; the parser auto-detects pure-fold bodies and routes them
through the same runtime.

### Design — `par(...)` over any `for`-iterable (partially shipped)

**Status (2026-06-06, #270).**  `par(...)` now **accepts every for-iterable** —
the vector-only gate is gone.  Shipped via the **Materialise** path (class 3
below) generalised to all non-vector sources: ranges / `iterator<T>` / text go
through `materialise_iter_for_par` (reusing `build_comprehension_code`), keyed
collections through `materialise_keyed_for_par`.  The **zero-allocation fast
paths remain future work**: the Range split (`parallel_for_range`, class 1) and
Fusable-map (class 4) are NOT yet implemented — a range currently materialises
into a temp `vector<integer>` rather than partitioning `[lo,hi)` directly.  The
rest of this section is the optimisation roadmap for those fast paths.

**Goal.** `par(...)` should accept anything a `for` statement accepts —
integer ranges, keyed collections, text, `map`/`filter` chains, custom
`.next()` iterators — not only `vector<T>`.  **Principle:** accept
everything; **materialise a temporary vector only when it is absolutely
needed** (the iterator cannot be partitioned natively).  Normally a
`par(...)` should run without allocating an intermediate collection.

**The constraint that makes it vector-only today.** The runtime
(`parallel_for` → `run_parallel_raw`, `src/parallel.rs`) partitions work
into **contiguous index ranges** (`[start,end)` per thread) and fetches
element `i` via `vector::get_vector(input, elem_size, i)` — i.e. it needs
**O(1) random access by index** over a known row count with a uniform
element size.

⚠ **`elem_size` is the width of the SLOT the container holds, not of what
the element points at**, and the two part company for a nested collection:
a `vector<vector<T>>` stores a 4-byte record index per row whatever `T` is.
`par_elem_size` reached it through `type_elm`, which resolves a
`vector<integer>` element to `integer` and answered 8 — so the worker
strode twice per row, saw rows 0 and 2, and then read past the end, where
C80 hands back the element's default rather than faulting. `vector<text>`
was the one inner type that worked, because `text`'s db size is 4 by
coincidence. A wrong stride here is silent by construction: the runtime has
no row identity to check the fetch against (loft#1033).  The parser enforces this at
`src/parser/collections.rs:1855` (`"par(...) requires a vector<T> input"`).
A `for`-iterable is otherwise one of: an index-addressable vector; a
counted range; or a **sequential cursor** (keyed B-tree/hash `OpStep`,
text byte-cursor, coroutine `OpCoroutineNext`, custom `.next()`) that has
no random access.

**Approach — classify the input, pick the cheapest partition.**  Replace
the vector-only gate with a classifier routing each iterable to one of
these paths (the cheaper the better; materialise is the last resort):

| Class | Iterables | Partition strategy | Temp vector? |
|---|---|---|---|
| **Index** | `vector<T>`, tuple-vectors, struct vector-fields | by index range (the current path) | no |
| **Range** | integer ranges `lo..hi`, `..=`, reverse | split `[lo,hi)` into N sub-ranges | **no** (fast path) |
| **Fusable map** | `map(src, g)` where `src` is Index/Range and `g` is pure | partition `src`; worker computes `f(g(a))` | **no** (fusion) |
| **Materialise** | keyed (`sorted`/`hash`/`index`/`spatial`), `filter(...)`, comprehensions, finite custom iterators | drive the for-cursor ONCE into a temp `vector<T>`, then Index path | **yes** — the "absolutely needed" case |
| **Reject / opt-in** | coroutine generators, infinite / side-effecting `.next()`; `#fields` (compile-time unroll) | not partitionable / no runtime iteration | diagnostic (or opt-in materialise) |

1. **Range — no vector (the headline win).**  `for i in lo..hi par(b =
   f(i), N)` lowers to a new runtime entry **`parallel_for_range(lo, hi,
   return_size, threads, fn) -> reference`**: partition `[lo,hi)` into N
   contiguous sub-ranges; each worker runs the counted body over its
   sub-range, calling `f(i)` for each integer `i`; results are collected
   by global index (`i - lo`).  The worker element is the loop integer —
   no input vector is allocated.  Reverse / `..=` ranges adjust the
   bounds.  Mirrors `run_parallel_raw` but advances a counter instead of
   `get_vector`.  Reuses `clone_for_worker`, the result stitch
   (`parallel_buf_get*`), and the order-by-index guarantee.

2. **Index — unchanged.**  The current `parallel_for(vec, elem_size, …)`
   path for vectors / struct-fields / tuple-vectors.

3. **Materialise — the explicit fallback.**  For FINITE, re-readable
   sequential iterables (keyed collections, `filter` chains,
   comprehensions), generalise the existing `materialise_keyed_for_par`
   (`collections.rs:1542`, already used for keyed-collection `par`) into
   `materialise_for_par(iterable) -> vector<T>`: drive the for-iterator
   once (the same `OpIterate`/`OpStep` / `filter` lowering the sequential
   `for` uses), appending into a temp vector, then run the Index path.
   Keyed collections and `map`/`filter` already do exactly this — this
   only makes the fallback general.  The temp vector frees at loop-scope
   exit and (post-P5/P6) is compact and reuses freed space.

4. **Fusable map — avoid the vector.**  For `for x in src.map(g) par(b =
   f(x), N)` where `src` is Index- or Range-partitionable and `g` is
   pure, FUSE: partition `src` directly and have each worker compute
   `f(g(a))` — no materialisation.  (`filter` cannot fuse — its variable
   output count breaks the by-index result layout — so it materialises.)
   This delivers the "function without a vector" ideal for the common
   map-over-vector/range case.

5. **Non-partitionable.**  Coroutine / `iterator<T>` generators and
   side-effecting custom `.next()` are inherently sequential (poll-based,
   stateful) and may be unsafe to replay.  `par` over them either
   (a) opt-in materialises if the generator is finite and yields values
   (drive to exhaustion into a temp vector — documented memory caveat),
   or (b) emits a clear diagnostic ("par over a generator requires
   materialisation; collect into a vector first").  `#fields` is a
   compile-time unroll (no runtime iteration) — diagnose.

**Runtime additions.**
- `parallel_for_range(lo, hi, return_size, threads, fn)` in
  `src/parallel.rs` (+ `n_parallel_for_range` in `src/native.rs` + the
  `default/01_code.loft` declaration) — range partition + worker dispatch
  + result buffer.
- `parallel_get_*` / `parallel_buf_get*` result gathering and the
  `return_size` logic are element-type-agnostic and **unchanged**.

**Parser changes (`src/parser/collections.rs`).**
- Replace the `Type::Vector`-only gate in `parse_parallel_for_loop`
  (~1855) with the classifier; the keyed pre-materialisation at
  1303-1326 becomes one branch of the general `materialise_for_par`.
- Range branch emits `parallel_for_range` (worker element = the range's
  integer loop var).
- Fusion branch detects `map(partitionable_src, pure_g)` and rewrites the
  worker to `f(g(a))` over `src`.
- Reuse `for_type` (`control.rs:2853`) for the loop-var / element type in
  every branch.

**Invariants preserved.**
- **Ordering:** results are delivered in iteration order (by global
  index) on every path, so `par` stays a drop-in for the sequential
  `for`.
- **Read-only workers:** `clone_for_worker` keeps the worker view
  read-only.  The Materialise pre-pass and any fused `map`/`filter`
  transform run on the MAIN thread before the parallel region — so a
  side-effecting transform is never silently parallelised (it
  materialises sequentially or is rejected).
- **Clamping:** empty / singleton inputs and `threads > rows` clamp as
  today (`threads.min(n_rows.max(1))`); an empty range yields an empty
  result.

**Phasing (each independently shippable).**
- **P1** — Range fast-path (`parallel_for_range`): the biggest, cleanest
  win and the documented gap (`for i in 1..n par(…)`).
- **P2** — Generalise `materialise_for_par` to all finite sequential
  iterables (keyed already done; add `filter` / comprehension / finite
  custom iterators).
- **P3** — Map-fusion optimisation (no temp for `map`-over-partitionable).
- **P4** — Diagnostics for non-partitionable generators + `#fields`
  (opt-in materialise where safe).

**Tests.**  Extend `tests/scripts/22-threading.loft`: `par` over a range
(sum + transform), over a keyed collection, over `v.map(g)`, over
`v.filter(p)`; assert results == the sequential `for` over the same
iterable (order AND values), cross-mode (interp + native); a generator
`par` diagnostic test; `store_memory()` to confirm the materialise
fallback frees its temp.

### Post @PLAN06 surface (closed 2026-05-09)

Plan-06 collapsed the 7-variant `par` runtime + 3-fn native
dispatch into one store-stitch path.  Every parallel worker
now writes its output into a per-worker output Store; the main
thread stitches per-worker stores into a single result Store.
There is no separate `par_light(...)`; the parser decides light
vs full path from the worker's effect signature.  See
[CHANGELOG_TECHNICAL.md § Plan-06 (typed-par redesign) closed
2026-05-09](CHANGELOG_TECHNICAL.md) for the per-A-step shipped
manifest, and [§ Dispatcher inventory](#dispatcher-inventory-when-adding-a-new-return-shape)
below for the post-@PLAN06 dispatcher set.

---

## Dispatcher inventory (when adding a new return shape)

`src/parallel.rs` exposes 5 distinct `pub fn run_parallel_*`
dispatchers for the par worker runtime.  They diverge structurally,
not just in result-buffer shape — see ARC.md A8's deferral rationale
for why a unifying trait collapse was considered and rejected.

| Dispatcher | Return shape | `Stores` borrow | Worker primitive | Per-row execute call | Per-thread state | Merge step |
|---|---|---|---|---|---|---|
| `run_parallel_queue` (line 1251) | `Vec<u64>` (i64 / float / 8B prim) | `&Stores` | `parallel_workers` | `execute_at_raw_worker_arg` | none | `merge_batches(…, 0u64)` |
| `run_parallel_text` (line 585) | `Vec<String>` | `&Stores` | `parallel_workers` | `execute_at_text` (single shape) | per-worker output store slot via `add_output_slot` + `s_pos` array record | iterate slots, `get_str` per row |
| `run_parallel_queue_ref` (line 669) | `(Vec<DbRef>, Vec<u16>)` | `&mut Stores` | raw rayon (`pool.install` + `into_par_iter`) | `execute_at_ref` with caller-pre-allocated hidden destination stores | `worker_slot_dispenser` (atomic) + `worker_allocated_indices.clear()` + `n_hidden_dests` claim | `mem::swap` stores at allocated indices into parent + `revive_record_chain` graph walk |

**Whatever else a dispatcher does, it does not decide how the row reaches the worker.**
`parallel::worker_row_arg` answers that for all of them, and `WorkerArg` is the vocabulary:
`Text` (16-byte `Str`), `Primitive` (1/4/8-byte value), `Wide` (9..=64 bytes inline — a
tuple), `Ref` (12-byte `DbRef`).  The rule is that the row's SHAPE picks the spelling and
the worker's RETURN type never enters into it.

That was four hand-written ladders until loft#1055, each stopping at a different rung, and
every gap ended at the same wrong answer — `WorkerArg::Ref`, so the worker read a pointer's
bits as its value.  `run_parallel_text` had no wide arm at all, `run_parallel_queue_ref`
had neither a wide nor a text arm, and `run_parallel_discard` had a wide arm it could only
take when the worker had NO hidden parameters — which made a tuple into a text-returning
worker answer wrong, a 3-tuple underflow the worker stack, and `vector<text>` into a
struct-returning worker SIGSEGV.  A new dispatcher gets this right by calling that
function; it cannot get it right by copying a neighbour.
| `run_parallel_queue_narrow` (today `run_parallel_int`, line 927) | `Vec<i64>` packed via narrow stride elsewhere | `&Stores` | `parallel_workers` | `execute_at` (i64) | none | `merge_batches(…, i64::MIN)` |
| `run_parallel_queue_fn` (line 1347, cfg-gated) | `Vec<u8>` (packed 20-byte fn-ref blobs) | `&Stores` | `parallel_workers` | `execute_at_raw_to(fn_pos, …, dst)` writing through `SendMutPtr` to disjoint slots in a pre-allocated buffer | none | no merge — buffer filled in-place |

Plus 3 non-Queue dispatchers: `run_parallel_discard` (Stitch::Discard,
no buffer), `run_parallel_fold` (Stitch::Reduce, scalar accumulator),
`run_parallel_block` (`parallel { arm; arm }` — internal, not a row
loop).

`run_parallel_block`'s native twin is `codegen_runtime::n_parallel_block_native`, which
takes one Rust closure per arm instead of one bytecode position per arm.  The generator
emits every variable the arms assign at the top of EVERY closure, at its type's default:
each arm is a separate top-level expression, so `sum = 0;` and the loop that reads `sum`
are two different arms, and the interpreter gives the second one an entry-value copy
through the parent's stack snapshot.  Private per closure, which is the isolation the
construct promises.  A `&`-link local is left out — a raw pointer has no honest default,
so an arm reading a sibling's link fails to compile rather than dereferencing a made-up
address.

### When to add a new dispatcher vs extend an existing one

- **Adding a return shape that fits an existing impl** (e.g. another
  ≤ 8B primitive) → add a new route in `src/parser/collections.rs::
  build_parallel_for_ir` pointing at `n_parallel_queue` (or
  `_narrow` if the value width is < 8B).  No new `run_parallel_*` fn.
- **Adding a return shape with a NEW Stores buffer-stack type** →
  add a new buffer stack in `src/database/mod.rs` (per-type, not
  polymorphic — see the rationale at lines 215-265), a new
  `n_parallel_queue_<X>` native fn in `src/native.rs`, a new
  `_native` mirror in `src/codegen_runtime.rs`, and either reuse
  one of the existing `run_parallel_*` shapes or add a new one if
  the per-thread state / merge truly diverges.
- **Don't try to unify** the existing 5 dispatchers under one trait.
  The shape differences (`&Stores` vs `&mut Stores`, parallel_workers
  vs raw rayon, per-row execute signature, per-thread state, merge
  step) are structural.  See ARC.md A8 for the full audit.

## See also
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/state/`, store cloning for workers
- [STDLIB.md](STDLIB.md) — `par(...)` parallel for-loop user-facing API
- [PLANNING.md](PLANNING.md) — A1 (parallel workers: extra args + text/ref returns)
- [plans/finished/06-typed-par/](plans/finished/06-typed-par) — closure record for the typed-par redesign (closed 2026-05-09)
- [THREADING_SAFETY.md](THREADING_SAFETY.md) — safety analysis of the parallel worker system (Part 1, P1-R1…R5)
- [THREADING_SAFETY_COROUTINES.md](THREADING_SAFETY_COROUTINES.md) — safety analysis of coroutines, stores and strings (Part 2, P2-R1…R10)
- [THREADING_PAR_LIGHT.md](THREADING_PAR_LIGHT.md) — `par_light` lightweight parallel for-loop design
- [THREADING-history.md](THREADING-history.md) — the dated measurement timeline behind this doc (Plan-06 phase 0 baseline)
