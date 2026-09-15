
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

## Plan-06 phase 0 baseline

Recorded 2026-04-25 on the loft project's primary CI host.
Workload: `bench/11_par/bench.loft` — 100 K-element vector,
50-iteration Newton's-method sqrt per element, 4 worker threads.

| Column | Time | Notes |
|---|---|---|
| python | 33ms | `multiprocessing.Pool(4)` — 4 worker processes |
| loft-interp | 44ms | `par(items, work, 4)` — 4 threads (real parallel) |
| loft-native | 12ms | **single-threaded** — G4: `n_parallel_for_native` ignored the thread count and ran sequentially. |
| loft-wasm | `-` | par codegen rejects today (G3 — fixed by phase 1) |
| rust | 4ms | `std::thread::spawn × 4` — std-only, range-partitioned |

### Phase 1a / 1b / 1c delta (G4 closed across all three native paths)

Recorded 2026-04-25 after `n_parallel_for_native` /
`_text_native` / `_ref_native` were rewritten to use
`thread::scope` × N + `clone_for_worker`.  Same workload, same
host:

| Column | Time | Δ vs phase 0 |
|---|---|---|
| loft-native | **6ms** | -50 % (sequential 12ms → parallel 6ms — G4 closed) |

The bench measures primitive-return par; the text and ref native
paths use the same parallelization shape (per-thread
`Vec<(idx, value)>` batches + main-thread merge — text writes via
`set_str`, ref deep-copies via `Stores::copy_from_worker`'s
graft).  All three native dispatches now real-parallel.

The native column sits within ~2× of rust's 3-4ms — remaining
overhead is per-worker `Stores::clone_for_worker()` + result
merge on the main thread.  Phase 2 (rebase pass retiring
`copy_block` for ref returns) and the per-worker output Store
work should reclaim the rest.

Plan-06 phase 1's bench gate: loft-interp within ±5 % of 44 ms;
loft-native ≤ ~5 ms (further closure work ahead).
Subsequent phases assert ±5 % regression on both columns.

### ARC.md A1 host-relative check (2026-04-30)

A1 retired the heavy `parallel_execute_and_collect` and its
`run_parallel_direct` / `_ref` helpers (commit `b9ad7af`).  The
phase-0 absolute timings above were recorded on a different host;
on the development workstation used for A1, both `main` and
`roadmap-lsp-eclipse` HEAD (post-A1) measure in the same regime:

| Column | main (5-sample median) | branch post-A1 (5-sample median) | Δ |
|---|---|---|---|
| loft-interp | ~98ms | ~101ms | +3 % (within noise; ±5 % gate PASS) |
| loft-native | n/a (@P199 — `n_parallel_for_native` codegen E0499) | n/a (same) | — |

The loft-native column cannot be measured today: native compilation
of `bench/11_par/bench.loft` hits the @P199 double-borrow on
`format_float(&mut s, t_5float_round(stores, …), …)`, which exists
identically on `main`.  ARC step A7 closes @P199.  Once A7 lands the
native column re-enables and gates with ±5 % against a fresh
host-relative baseline.

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
- See `par_light(...)` and thread safety sections below

---

This document records safety analyses of the runtime memory model with
design-level mitigations for each identified risk.

- **Part 1** covers the parallel worker system (`src/parallel.rs`,
  `src/database/allocation.rs`, `src/store.rs`).
- **Part 2** covers the coroutine system (`src/state/mod.rs`, `CoroutineFrame`,
  `stack_bytes`, `text_owned`).

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
- [P2 Architecture Summary](#p2-architecture-summary)
- [P2 What is Safe](#p2-what-is-safe)
- [P2-R1 — Text argument `Str` dangles on first resume](#p2-r1--text-argument-str-dangles-on-first-resume)
- [P2-R2 — `String` objects leaked at exhaustion](#p2-r2--string-objects-leaked-at-exhaustion)
- [P2-R3 — Text locals have implicit "never freed" invariant](#p2-r3--text-locals-have-implicit-never-freed-invariant)
- [P2-R4 — `text_positions` inconsistent across yield/resume](#p2-r4--text_positions-inconsistent-across-yieldresume)
- [P2-R5 — Store-backed `Str` dangles on record delete](#p2-r5--store-backed-str-dangles-on-record-delete)
- [P2-R6 — Compiler check for `yield` inside `par()` missing](#p2-r6--compiler-check-for-yield-inside-par-missing)
- [P2-R7 — Exhausted frames never freed](#p2-r7--exhausted-frames-never-freed)
- [P2-R8 — `DbRef` locals outlive their store across suspension](#p2-r8--dbref-locals-outlive-their-store-across-suspension)
- [P2-R9 — `e#remove` on a generator iterator corrupts unrelated records](#p2-r9--eremove-on-a-generator-iterator-corrupts-unrelated-records)
- [P2-R10 — Yielded `Str` value lifetime is not enforced at the consumer](#p2-r10--yielded-str-value-lifetime-is-not-enforced-at-the-consumer)
- [Part 2 Summary Table](#part-2-summary-table)

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

## Part 2 — Coroutines, Stores, and Strings

---

## P2 Architecture Summary

A suspended `CoroutineFrame` lives in `State::coroutines: Vec<Option<Box<CoroutineFrame>>>`,
entirely outside the `Store`/`Stores` system.  It holds two Rust-heap structures
that reference loft memory:

- **`stack_bytes: Vec<u8>`** — raw byte copy of the generator's stack locals at
  the last suspension point.  For text variables this encodes inline `String`
  objects (`ptr + len + cap`).  For text arguments and yielded text values it
  encodes `Str { ptr: *const u8, len: u32 }` pointing into external storage.
- **`text_owned: Vec<(u32, String)>`** — designed to hold owned copies of all
  dynamic text slots after serialisation (SC-CO-1/SC-CO-8 mitigations), with the
  `u32` being the byte offset within `stack_bytes` to patch on resume.

`text_owned` is always empty in the current implementation.  The full
serialisation path (`serialise_text_slots` / `free_dynamic_str`) is described in
COROUTINE.md but is not yet implemented (deferred as CO1.3d).

Store records are referenced only via `DbRef` values serialised as raw bytes into
`stack_bytes`.

### Coroutine lifecycle

```
OpCoroutineCreate  →  frame.stack_bytes = raw copy of arg bytes; text_owned = []
                       (arg Str pointers unowned — dangles if caller frees text)
OpCoroutineNext    →  raw copy of stack_bytes → live stack; no Str patching
OpYield            →  raw copy of live stack → stack_bytes; no text serialisation
OpCoroutineReturn  →  stack_bytes.clear(); text_owned.clear()
                       (String objects in stack_bytes are dropped as raw bytes — LEAK)
```

---

## P2 What is Safe

**Re-entrant advance detection** — `active_coroutines.contains(&idx)` prevents a
running generator from being advanced again.  The `Vec<usize>` correctly tracks
all simultaneously active frames under `yield from` nesting (SC-CO-3, SC-CO-9
resolved). ✓

**Stack base relocation** — `frame.stack_base = self.stack_pos` is reset at every
resume, so restored bytes always land above the caller's current stack top.
Caller locals pushed after creation are never overwritten (SC-CO-7 resolved). ✓

**Null iterator guards** — `coroutine_next` and `coroutine_exhausted` check
`store_nr != COROUTINE_STORE || rec == 0` before touching any frame. ✓

**`text_owned` offset is `u32`** — `Vec<(u32, String)>` gives 4 GB headroom;
no truncation for deep frames (SC-CO-12 resolved). ✓

---

## P2-R1 — Text argument `Str` dangles on first resume

### Description

**Severity: critical — use-after-free**

`coroutine_create` copies argument bytes verbatim from the live stack into
`stack_bytes`:

```rust
std::ptr::copy_nonoverlapping(src, stack_bytes.as_mut_ptr(), args_size as usize);
// text_owned stays empty — CO1.3d will handle text serialisation.
```

Text arguments are passed as `Str { ptr: *const u8, len: u32 }` — a zero-copy
reference into the **caller's** owned `String`.  After `OpCoroutineCreate`, the
caller continues executing normally.  When the caller's text variable goes out of
scope, `OpFreeText` frees the `String`.  The `Str` bytes copied into `stack_bytes`
now hold a dangling pointer.

On the first `OpCoroutineNext`, those bytes are copied back to the live stack:

```rust
std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len());
```

The generator executes with a dangling `Str` in its parameter slot.  Any read of
that text parameter is a use-after-free.

Static string literals (`Str.ptr` into `text_code`) are permanently live and
are not affected.

### Mitigation design

**M6-a — Implement `serialise_text_slots` at `OpCoroutineCreate`**

The COROUTINE.md design (CO1.3d) already specifies the fix: after copying the
argument bytes, call `serialise_text_slots` to transfer ownership of every
dynamic-text `Str` slot:

```rust
// In coroutine_create, after the raw byte copy:
let text_owned = serialise_text_slots(
    &mut stack_bytes,
    &def.text_arg_slots,   // (byte_offset, Type) pairs for text parameters
    &mut self.database,
);
frame.text_owned = text_owned;
```

`serialise_text_slots` must:
1. Read each `Str` slot from the bytes.
2. Skip null `Str` (ptr == STRING_NULL) and static `Str` (ptr inside `text_code`).
3. Call `s.str().to_owned()` to make an independent `String`.
4. Call `database.free_dynamic_str(ptr)` to release the original allocation
   (matching `OpFreeText` semantics).
5. Write a `Str` pointing to the owned buffer into `stack_bytes`.
6. Record `(offset as u32, owned_string)` in `text_owned`.

**M6-b — Implement the pointer-patch step in `coroutine_next`**

Before copying `stack_bytes` to the live stack, patch each `text_owned[i].1`
buffer address back into the corresponding slot in `stack_bytes`:

```rust
for (offset, s) in &frame.text_owned {
    let patched = Str::new(s.as_str());
    write_str_at(&mut frame.stack_bytes, *offset as usize, patched);
}
```

The `String` buffer address is stable as long as the `String` is not pushed or
grown between the patch and the copy.  No extra allocation is required.

**M6-c — Implement `free_dynamic_str` in `Stores` / `State`**

This function must match how `OpFreeText` releases a dynamic string in `text.rs`.
The mechanism depends on whether the runtime uses a scratch/side-table of
`String` objects or direct heap addresses.  Determine the correct call and add it
to `database::Stores` or as a `State` helper before CO1.3d lands.

---

## P2-R2 — `String` objects leaked at exhaustion

### Description

**Severity: high — memory leak on every generator with text locals that yields**

`coroutine_return` clears the frame's saved state:

```rust
frame.text_owned.clear();
frame.stack_bytes.clear();
```

`Vec<u8>::clear()` drops the `Vec`'s own backing allocation but treats the
contained bytes as plain scalars — no element destructor is called.  If
`stack_bytes` encodes `String` objects (text local variables are stored inline
on the stack as `String { ptr, len, cap }` structs), those `String`s' internal
heap allocations are **never freed**.

This affects every generator that:
1. Has at least one text local variable, **and**
2. Yields at least once before exhausting (so `stack_bytes` was written with
   live `String` bytes at the last `OpYield`).

Additionally, at the moment `OpCoroutineReturn` runs, the live stack holds the
most recently restored `stack_bytes` at `[stack_base, stack_top)`.  After
`stack_pos = stack_base`, those `String` objects are abandoned on the stack
without `drop` being called.  Both leak paths affect the same set of programs.

### Mitigation design

**M7-a — Explicitly drop `String` objects before clearing `stack_bytes`**

Before `frame.stack_bytes.clear()` in `coroutine_return`, walk every text slot
in `frame.text_owned` (or, if CO1.3d is not yet complete, walk the known text
slot offsets from the function definition) and call `drop` on each `String`
object encoded at that offset:

```rust
for (offset, owned) in frame.text_owned.drain(..) {
    drop(owned);   // Rust RAII frees the String's internal allocation
}
// The live-stack String objects also need to be dropped here;
// after CO1.3d they are always reflected in text_owned so the
// loop above covers both saved and live copies.
frame.stack_bytes.clear();
```

Once CO1.3d (`serialise_text_slots`) is implemented, `text_owned` always holds
all live `String` allocations for the frame, so this drain is the complete fix.
Until CO1.3d lands, a separate walk over the function definition's text-slot
layout is needed to cover the live-stack copies as well.

**M7-b — Add a test for text-local generator exhaustion**

Add a test that creates a generator with a `text` local, yields once, and then
breaks the `for` loop to force `OpCoroutineReturn` with a populated frame.  Run
it under Valgrind or with the Rust allocator's leak-detection feature to confirm
no allocation escapes.

---

## P2-R3 — Text locals have an implicit "never freed between yield and resume" invariant

### Description

**Severity: high — fragile; becomes use-after-free when CO1.3d lands**

`coroutine_yield` raw-copies live stack bytes into `stack_bytes` without
serialising text:

```rust
// Serialise locals (integer-only path — no text_owned handling yet).
let mut locals_bytes = vec![0u8; locals_len];
// ... raw copy ...
frame.stack_bytes = locals_bytes;
// text_owned stays empty — CO1.3d will handle text serialisation.
```

A text local variable on the stack is a `String { ptr, len, cap }` struct.  The
raw copy saves the struct's fields, including the internal heap pointer.  On
resume, those bytes are written back to the live stack; the internal pointer is
the same, and the heap allocation was never freed — so the resume is currently
safe.

The invariant holding this together is: **the generator's `String` allocations
are never freed between yield and resume**.  This holds today because:
- The compiler does not emit `OpFreeText` for generator locals before `OpYield`
  (text locals persist across yields).
- Nothing else claims those heap addresses in the interim.

This invariant breaks in two future scenarios:

1. **When CO1.3d (`serialise_text_slots`) is partially implemented:** if
   `free_dynamic_str` is called on the original allocation at yield time (step 4
   of M6-a) before the pointer-patch step (M6-b) is also in place, the resume
   path will write the freed pointer to the live stack — explicit use-after-free.
   CO1.3d must land atomically; partial implementation is more dangerous than no
   implementation.

2. **If a future optimisation reuses "off-stack" memory** between a yield and
   the matching resume, the old `String` bytes restored from `stack_bytes` would
   contain a pointer to reused memory — silent data corruption.

### Mitigation design

**M8-a — Implement CO1.3d atomically**

The serialisation (yield), pointer-patch (resume), and drop (exhaustion) steps
are a single unit of work.  Track them as one task in the implementation plan; do
not merge a partial implementation that calls `free_dynamic_str` without also
implementing the pointer-patch in `coroutine_next` (M6-b) and the `String` drain
in `coroutine_return` (M7-a).

**M8-b — Add a compile-time marker for CO1.3d incomplete state** (✓ implemented)

`coroutine_yield` now contains a `debug_assert!` that fires when `text_positions`
contains any live String slot in the generator's locals range `[base..value_start)`.
The implementation uses `text_positions` (already maintained for S27) rather than
a `text_slot_count` field, avoiding new struct fields:

```rust
let text_local_count = self.text_positions.range(locals_range.clone()).count();
debug_assert!(
    text_local_count == 0,
    "P2-R3: coroutine_yield: {text_local_count} live text local(s) in stack \
     range [{base}..{value_start}). CO1.3d (serialise_text_slots) is not yet \
     implemented; the raw-bytes copy saves heap pointers that could dangle on \
     resume.  See SAFE.md § P2-R3 and PLANNING.md § S25.",
);
```

This turns a silent mis-feature into a loud early failure during development.
Test: `expressions::coroutine_text_local_survives_yield` (ignored until CO1.3d lands).

---

## P2-R4 — `text_positions` inconsistent across yield/resume

### Description

**Severity: medium — debug detector gives wrong results for generators with text locals**

`State::text_positions: BTreeSet<u32>` (debug-only) tracks the absolute
stack-byte positions of live `String` objects.  `OpFreeText` asserts an entry
exists and removes it; `OpText` / `append_text` insert entries.

`coroutine_yield` rewinds `stack_pos` to `base + value_size` but does **not**
remove `text_positions` entries for the frozen text locals at
`[base, value_start)`.  `coroutine_return` also does not remove them.

Consequences:

- **False clean-up:** `free_stack` at the consumer level scans `text_positions`
  for entries in the rewound range.  Orphaned entries in that range are silently
  removed, masking a missing `OpFreeText` for unrelated code at the same stack
  positions.
- **False double-free miss:** After exhaustion, the text locals leak (P2-R2) but
  their `text_positions` entries remain.  A future `OpFreeText` on an unrelated
  variable that happens to land at the same absolute stack position will find an
  entry and succeed, hiding a missing free.
- **Opposite case on resume:** If `coroutine_yield` *had* removed the entries and
  `coroutine_next` did not re-add them, `OpFreeText` inside the resumed generator
  body would hit the `assert!(remove, "double free")` path on the first free of
  a text local after resume — a false double-free panic.

### Mitigation design

**M9-a — Remove text-local entries from `text_positions` at yield; restore on resume**

In `coroutine_yield` (debug path):
1. Collect all `text_positions` entries in `[base, value_start)`.
2. Remove them from `text_positions`.
3. Store the removed set in `frame.saved_text_positions: BTreeSet<u32>`.

In `coroutine_next` (debug path):
1. Re-insert `frame.saved_text_positions` into `text_positions`.
2. Clear `frame.saved_text_positions`.

In `coroutine_return` (debug path):
1. Clear `frame.saved_text_positions` without reinserting (the allocations are
   being dropped; no future `OpFreeText` should fire for them).

This keeps `text_positions` a correct snapshot of the live stack at all times.

---

## P2-R5 — Store-backed `Str` dangles on record delete

### Description

**Severity: medium — silent data corruption; harder to trigger than P2-R1**

When a generator reads a text field from a store record, the value is computed as
`Str { ptr: store.ptr + rec*8 + 8, len }` — a zero-copy pointer directly into
the store's raw allocation.  If this `Str` is live in a local at a `yield` point,
`stack_bytes` encodes the raw bytes of that pointer.

If, between the yield and the next resume, the consumer:
1. Deletes the record (`database.free(r)` / `store.delete(rec)`), OR
2. Frees the entire store (`database.free(db_ref)`)

...and that store word is subsequently reclaimed by a new `claim()` (reused for
different data), the `Str.ptr` in `stack_bytes` now points to unrelated bytes.
On resume the generator reads a corrupted string — **silent data corruption** with
no panic or warning in either debug or release builds.

This is an extension of SC-CO-2 (DbRef lifetime) to text specifically.  The
danger is less visible than with DbRef: a `DbRef` is obviously a pointer that
needs lifetime management, whereas a `Str` looks like a plain value.  The window
of vulnerability is also longer for generators than for ordinary functions because
suspension can span many consumer iterations.

### Mitigation design

**M10-a — Document the invariant and add a debug-mode guard**

Extend the SC-CO-2 documentation in COROUTINE.md with the text-specific variant:

> Any `Str` value derived from a store record field (via `store.get_str()` or
> equivalent) must be treated as a borrow of the store's memory.  If such a
> `Str` is live at a `yield` point, the caller must not delete the backing
> record or free the store before the generator is exhausted or the local is
> overwritten.

In debug builds, add a range check in `coroutine_yield`: for each `Str` slot in
`stack_bytes`, verify the pointer does not fall within any known live store
allocation.  This is a heuristic (cannot cover all store memory without full
pointer provenance), but catches the common case of a recently-obtained field
reference.

**M10-b — Long term: deep-copy store-derived text on yield**

Extend CO1.3d's `serialise_text_slots` to treat store-derived `Str` values
(pointer within `Stores::allocations[*].ptr` range) as dynamic text: copy the
string bytes into an owned `String`, replace the raw store pointer with a pointer
into the owned buffer.  This eliminates the class entirely at the cost of an
allocation per yielded store-text local.

---

## P2-R6 — Compiler check for `yield` inside `par()` missing

### Description

**Severity: medium — uncontrolled panic or silent wrong result**

SC-CO-4 in COROUTINE.md requires the compiler to reject `yield` expressions and
generator function calls inside `par(...)` bodies.  No such check exists in
`src/parser/`.

A `COROUTINE_STORE` DbRef (`store_nr == u16::MAX`) produced inside a `par(...)`
body belongs to the main thread's `State::coroutines` table.  Worker `State`
instances are initialised with `coroutines: vec![None]` (one null sentinel).  If
a worker receives a DbRef with `rec >= 1` and calls `coroutine_next`, it indexes
into its own `coroutines` with an out-of-bounds index — Rust panics.

If `rec == 1` and the worker happens to have allocated a coroutine at index 1,
the worker silently advances the *wrong* frame, producing incorrect results with
no error.

### Mitigation design

**M11-a — Add compiler check in `parse_parallel_for` and `par(...)` body parsing**

In `parse_parallel_for` (and in the `par(...)` body parser), add a flag
`inside_par_body: bool` to the parser context.  When `inside_par_body` is true:
- `yield` and `yield from` emit a diagnostic error.
- Any call to a function with return type `iterator<T>` emits a diagnostic error.

**M11-b — Add a runtime guard as defence-in-depth**

In `coroutine_next`, check whether the `COROUTINE_STORE` DbRef's `rec` is within
bounds of `self.coroutines`:

```rust
if idx >= self.coroutines.len() {
    panic!(
        "coroutine DbRef (rec={idx}) out of range — \
         iterator<T> values must not cross thread boundaries"
    );
}
```

This converts the Rust out-of-bounds panic into a clearly attributed error
message.

---

## P2-R7 — Exhausted frames never freed

### Description

**Severity: low — memory growth; no correctness impact**

`coroutine_return` marks the frame `Exhausted` and clears `stack_bytes` /
`text_owned`, but keeps the `Box<CoroutineFrame>` in `State::coroutines`.  The
slot is never set to `None`.

There is also no finalizer for `COROUTINE_STORE` DbRef variables: when a
generator's DbRef goes out of scope, the frame it points to is not freed.  The
`free_coroutine(idx)` helper exists in the design (COROUTINE.md Phase 1) but is
not called from anywhere in the implementation.

For programs that construct many generators over their lifetime (e.g., a
generator factory called in a loop), `State::coroutines` grows without bound —
one `Box<CoroutineFrame>` per generator invocation, each holding at minimum the
`CoroutineFrame` struct overhead even after exhaustion.

### Mitigation design

**M12-a — Free exhausted frames from the `for`-loop exit path**

The `for ... in gen { }` desugaring knows when the loop exits (either by
exhaustion or by `break`).  At loop exit, emit `OpFreeCoroutine(gen_slot)` which
calls `free_coroutine(idx)` to set the slot to `None`.  This covers the common
case without requiring a general garbage collector.

**M12-b — Free exhausted frames from `exhausted()` calls**

`exhausted(gen)` is often called immediately after a `next()` that returned null.
Optionally, `coroutine_exhausted` could call `free_coroutine(idx)` lazily when
it first observes `Exhausted` status.  This handles the `explicit-advance` API
path (`a = next(gen); if exhausted(gen) { ... }`).

**M12-c — Long term: reference counting for `COROUTINE_STORE` DbRefs**

A general solution requires tracking how many DbRef copies of each coroutine
index exist.  When the count reaches zero, `free_coroutine` is called.  This
mirrors the standard approach for heap-allocated objects.  Not planned for the
initial coroutine implementation but required before 1.0.

---

## P2-R8 — `DbRef` locals outlive their store across suspension

### Description

**Severity: medium — silent data corruption or wrong results; no panic**

SC-CO-2 is acknowledged in COROUTINE.md as "caller responsibility" but the
risk is qualitatively worse for generators than for ordinary functions.

An ordinary function holds a `DbRef` only for its call duration.  A generator
holds `DbRef` locals across an arbitrary number of consumer iterations — the
suspension window can span hundreds or thousands of `next()` calls.  During that
window the consumer (or any other code) may:

1. **Free the record** (`database.free(r)` on the store that owns the record).
   The store slot is returned to the free list and claimed for a new record.  The
   generator resumes and reads/writes the *new* record's data through the old
   `rec` offset.
2. **Free the entire store** (`database.free(db_ref)` on the DbRef).  The store
   index is recycled.  On resume the generator resolves the old `store_nr` to a
   completely different store.
3. **Resize or relocate a record** (`store.resize(rec, new_size)`).  The record
   moves to a new word offset; the old `rec` in the frame is now stale.

All three produce silent data corruption: no assertion fails in release builds,
and the debug-build store lock is not triggered because the frame accesses data
through the *old, now-reused* coordinates, not through a locked-store path.

The risk is highest for generators that iterate over a collection while holding
a `DbRef` to a live record from that same collection (e.g., a generator that
lazily processes records one at a time while the caller can also delete them).

### Mitigation design

**M13-a — Document the suspension lifetime rule in loft language docs**

Extend LOFT.md and the generator chapter of STDLIB.md with an explicit rule:

> A `DbRef` stored in a generator local (including parameters of reference type)
> must remain live and unmodified for the entire lifetime of the generator.
> Freeing or resizing the backing record or store between `next()` calls produces
> undefined behaviour.

**M13-b — Add a debug-mode generation-counter guard on stores**

Add a `generation: u32` field to `Store`.  Increment it on every `claim`,
`delete`, and `resize` that changes the store's live-record set.  When
`coroutine_create` or `coroutine_yield` saves a `DbRef` into `stack_bytes`,
also save `(store_nr, generation_at_save)` into a new `frame.store_generations`
field.  On `coroutine_next`, verify that each saved store's current generation
equals the recorded value; if not, emit a runtime diagnostic:

```
runtime warning: coroutine resumed with stale DbRef — store N was modified
  (generation at save: 7, current: 9)
```

This is a heuristic: a generation match does not prove the specific `rec` is
still valid, but a mismatch is a definite violation.  Cost is O(distinct
store_nr count in the frame) per resume — negligible.

**M13-c — Compiler warning for mutable store access during generator suspension**

Long term: if the compiler can see that a generator holds a `DbRef` local of
type `T` and the consumer code between two `next()` calls contains a `free` or
structural mutation of a `T`-typed store, emit a warning.  This is a flow
analysis and is appropriate for a later compiler pass.

---

## P2-R9 — `e#remove` on a generator iterator corrupts unrelated records

### Description

**Severity: medium — silent store corruption in debug and release**

SC-CO-11 in COROUTINE.md states the compiler must reject `e#remove` on a
generator-typed iterator.  Verification is needed to confirm this check is
implemented.

The corruption mechanism: `e#remove` is lowered to an opcode that calls
`database.remove(db_ref)` using the iterator's DbRef.  For a store-backed
collection iterator, `db_ref` points to a real record in a real store — remove
deletes that record.  For a coroutine iterator, the DbRef encodes
`store_nr == COROUTINE_STORE (u16::MAX)` and `rec == frame_index`.  Passing
this to `database.remove`:

```rust
// database/search.rs — remove() resolves store_nr into allocations[store_nr]
// u16::MAX overflows the allocations Vec, causing an out-of-bounds panic in
// debug builds.  In release builds it wraps to allocations[u16::MAX % len],
// deleting an arbitrary record in a real store.
```

In release builds `u16::MAX % allocations.len()` selects a real store, and
`rec` is the frame index (a small integer like 1 or 2).  Word offset 1 or 2 in
an arbitrary store is almost certainly an occupied record header.  Marking it
free corrupts the store's free list silently.

### Verification step

Check `src/parser/collections.rs` and `src/parser/fields.rs` for the `e#remove`
path.  Confirm whether there is an early return or diagnostic when the iterator
type is `iterator<T>` (i.e., when the DbRef would have `store_nr == COROUTINE_STORE`
at runtime).

### Mitigation design

**M14-a — Compiler-level rejection (SC-CO-11 as specified)**

In the parser, at the point where `e#remove` is resolved, check whether the
loop's iterator type is a generator (identified by the function's return type
being `iterator<T>` backed by `OpCoroutineCreate`).  If so, emit:

```
error: `e#remove` is not valid on a generator iterator;
       generators do not back a store — use a collection if removal is needed.
```

This is a compile-time error with zero runtime cost.

**M14-b — Runtime guard as defence-in-depth**

In `database::remove` (or the opcode that calls it), add a check:

```rust
debug_assert!(
    db.store_nr != COROUTINE_STORE,
    "remove() called with a COROUTINE_STORE DbRef (rec={}); \
     e#remove is not valid on a generator iterator",
    db.rec
);
```

In release builds, return immediately if `db.store_nr == COROUTINE_STORE` rather
than indexing into `allocations` with `u16::MAX`.  This prevents the release-build
store corruption even if the compiler check (M14-a) is missing.

---

## P2-R10 — Yielded `Str` value lifetime is not enforced at the consumer

### Description

**Severity: low — caller confusion; no runtime bug under normal use**

When `OpYield` slides the yielded value bytes to `frame.stack_base`, a `Str`
value in the yielded type is represented as raw `{ ptr: *const u8, len: u32 }`
bytes.  The `ptr` points to the generator's `String` object (currently on the
abandoned-but-live part of the stack above `stack_pos`, or after CO1.3d into a
`text_owned` buffer).

The consumer receives a `Str` reference.  Under normal use — reading the value
synchronously in the loop body and not storing it beyond the current iteration —
the pointer is valid.  The generator's `String` is not freed until the frame is
exhausted (or until CO1.3d causes `free_dynamic_str` on yield).

The lifetime guarantee breaks in two subtle cases:

1. **Consumer stores the `Str` past the next `next()` call.**  On the next
   advance the generator resumes and may reassign or free the underlying
   `String`.  The consumer's saved `Str.ptr` now points to freed or overwritten
   memory.
2. **Consumer passes the `Str` into a function that stores it in a database
   record via `set_str`.**  `set_str` copies the bytes into the store, which is
   safe.  But if the function stores the raw `Str` struct (not the content), the
   same dangling-pointer risk applies.

Unlike P2-R1 through P2-R5, this is not a bug in the implementation — it is the
expected ownership model for `Str` values.  It is called out because generators
make the lifetime window less obvious: the `Str` appears to come from the loop
variable (a value), but it actually references the generator's internal storage.

### Mitigation design

**M15-a — Document the yielded-value ownership rule** *(done — COROUTINE.md CL-7)*

The ownership rule is documented as Known Limitation CL-7 in `COROUTINE.md`:

> A `text` value produced by `yield` is a zero-copy reference into the
> generator's frame.  It is valid only for the duration of the current loop
> body (or until the next `next()` call for explicit-advance code).  To keep
> the text beyond a single iteration, copy it: `stored = "{value}"` or
> pass it to a function that calls `set_str`.

**M15-b — Enforce the lifetime via CO1.3d pointer invalidation**

Once CO1.3d (`serialise_text_slots`) is implemented, `OpYield` will replace
the raw `String.ptr` in the yielded value bytes with a pointer into a
`text_owned` buffer.  At the *next* `OpYield`, that `text_owned` entry is
replaced with a new buffer.  In debug builds, zero-out the old `text_owned`
buffer before replacing it:

```rust
// In the text_owned update path of serialise_text_slots at yield:
#[cfg(debug_assertions)]
for byte in old_owned.as_bytes_mut() { *byte = 0xDD; }
```

This turns use-after-next into an immediate read of `0xDD...` bytes rather
than silently reading stale content, making the bug visible during testing.

**M15-c — Loft type system: `iter_text` reference type (long term)**

Long term, a distinct `iter_text` type (or a lifetime annotation on the loop
variable) could let the compiler reject assignments that outlive the current
iteration.  This is a language design question outside the scope of the
initial coroutine implementation.

---

## Part 2 Summary Table

### SC-CO cross-reference

| SC-CO | Description | Resolution status |
|---|---|---|
| SC-CO-1 | Dynamic `Str` in `stack_bytes` dangles | **Not implemented** (CO1.3d, see P2-R1, P2-R3) |
| SC-CO-2 | `DbRef` locals outlive store | See P2-R8 (S28 ✓); design in M13-a/b/c; CL-2b added for store-backed Str (P2-R5) |
| SC-CO-3 | Re-entrant advance | ✓ Implemented |
| SC-CO-4 | `yield` inside `par(...)` | ✓ Compiler error + runtime guard (P2-R6 M11-a/b) |
| SC-CO-5 | Serialisation cost O(depth) | Documented; accepted |
| SC-CO-6 | Advancing exhausted generator | ✓ Null pushed; frames freed on exhaustion (S26 ✓, P2-R7 done) |
| SC-CO-7 | Fixed `stack_base` clobbered | ✓ Implemented |
| SC-CO-8 | Original dynamic `String` leaked after `to_owned()` | **Not implemented** (CO1.3d, see P2-R2) |
| SC-CO-9 | Scalar active-coroutine tracker wrong for `yield from` | ✓ Implemented |
| SC-CO-10 | Yielded `text` value `Str` not serialised | **Not implemented** (CO1.3d, see P2-R3) |
| SC-CO-11 | `e#remove` on generator iterator | See P2-R9; verification + design (M14-a/b) |
| SC-CO-12 | `text_owned` `u16` offset truncation | ✓ Fixed (`u32`) |

### Risk priority

| Risk | Severity | Effort | Short-term fix | Long-term design |
|---|---|---|---|---|
| P2-R1 — Text arg `Str` dangles | **critical** | L † | Debug assert if any text arg at create (M8-b) | Implement `serialise_text_slots` at create (M6-a, M6-b, M6-c) |
| P2-R2 — `String` objects leaked at exhaustion | **high** | XS † | Drain `text_owned` before `stack_bytes.clear()` (M7-a) | CO1.3d complete makes M7-a sufficient; add leak test (M7-b) |
| P2-R3 — Implicit "never freed" invariant | **high** | L † | Debug assert on text slots present (M8-b) | Implement CO1.3d atomically (M8-a) |
| P2-R4 — `text_positions` inconsistency | **medium** | S | ✓ S27: save/restore entries on yield/resume in debug (M9-a) | Same |
| P2-R5 — Store-backed `Str` dangles | **medium** | S / S | ✓ M10-a: CL-2b in COROUTINE.md + debug pointer-range guard in `coroutine_yield` | Deep-copy store-derived text in `serialise_text_slots` (M10-b, via CO1.3d P2-R3) |
| P2-R6 — No compiler check for `yield` in `par()` | **medium** | S | ✓ M11-a + M11-b: `in_par_body` flag + compiler error + S23 runtime guard | Same |
| P2-R7 — Exhausted frames never freed | **low** | M | ✓ S26: `coroutines[idx] = None` on exhaustion (M12-a) | Reference counting for `COROUTINE_STORE` DbRefs (M12-c) |
| P2-R8 — `DbRef` locals outlive store across suspension | **medium** | M / XL | ✓ S28: generation-counter guard in debug (M13-a/b) | Compiler flow-analysis warning (M13-c) |
| P2-R9 — `e#remove` on generator corrupts store | **medium** | XS | Runtime guard in `database::remove` (M14-b) | Compiler rejection at `e#remove` resolution (M14-a) |
| P2-R10 — Yielded `Str` lifetime not enforced at consumer | **low** | XS / XL | ✓ M15-a done: CL-7 added to COROUTINE.md | Poison old buffer in debug after CO1.3d (M15-b) |

**Implementation dependency:** P2-R1, P2-R2, and P2-R3 all resolve together when
CO1.3d (`serialise_text_slots`) is implemented.  That work must land atomically
(M8-a): implementing the `free_dynamic_str` call without simultaneously
implementing the pointer-patch in `coroutine_next` turns the currently-safe
implicit invariant into an explicit use-after-free.

---

## See also

- [THREADING.md](THREADING.md) — `par(...)` syntax, `parallel_for` desugaring, worker rules
- [COROUTINE.md](COROUTINE.md) — coroutine design, SC-CO safety concerns, implementation phases
- [DATABASE.md](DATABASE.md) — `Stores`, `Store`, `DbRef`, locking API
- [INTERNALS.md](INTERNALS.md) — `src/parallel.rs`, `src/store.rs`, `src/state/mod.rs`
- [PROBLEMS.md](PROBLEMS.md) — Known bugs and open issues

---


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

- See `par(...)` design and thread safety sections above
- [PLANNING.md](PLANNING.md) — A14 item for this feature
- [PERFORMANCE.md](PERFORMANCE.md) — benchmark data once Step L7 is complete
- `src/parallel.rs` — existing `run_parallel_direct` / `run_parallel_raw`
- `src/store.rs` — `Store::new`, `clone_locked_for_worker`, `locked` enforcement
- `src/database/allocation.rs` — `clone_for_worker`, `find_free_slot`, `free_bits`
