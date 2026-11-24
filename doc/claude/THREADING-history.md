<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# THREADING-history.md — dated measurements behind THREADING.md

The timeline behind [THREADING.md](THREADING.md): baselines and host-relative checks recorded while Plan-06 (typed par) was built.

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

## Retired with the rustc writer (@PLN225)

The rustc writer's per-thread context and its `parallel { }` twin, kept as they stood.

| Implementation | Its context | A worker's |
|---|---|---|
| rustc writer | `cell: &UnsafeCell<Stores>`, first argument of every emitted function; its call frames, generator table and fn-ref buffers are `Stores::frames` and `Stores::native` | the interpreter's worker view |

`run_parallel_block`'s native twin is `codegen_runtime::n_parallel_block_native`, which
takes one Rust closure per arm instead of one bytecode position per arm.  The generator
emits every variable the arms assign at the top of EVERY closure, at its type's default:
each arm is a separate top-level expression, so `sum = 0;` and the loop that reads `sum`
are two different arms, and the interpreter gives the second one an entry-value copy
through the parent's stack snapshot.  Private per closure, which is the isolation the
construct promises.  A `&`-link local is left out — a raw pointer has no honest default,
so an arm reading a sibling's link fails to compile rather than dereferencing a made-up
address.

Adding a new `n_parallel_queue_<X>` also meant a `_native` mirror in `src/codegen_runtime.rs`.

The @PLN108 read-only-sharing paragraph as it stood in THREADING.md while the rustc writer's
dispatchers shared the interpreter's template:

**Read-only sharing (@PLN108, shipped 2026-07-17 — interpreter).** The per-worker byte-copy above is pure conservatism: a worker's captured parent state is read-only (@PLN102 @C93 — a `ParentWrite` from a worker is a compile error), so every parent store is provably unwritten for the par's lifetime, and the dispatcher joins all workers before the borrowed parent drops. So `run_parallel_discard` / `run_parallel_queue` now **BORROW** the parent stores read-only (`clone_for_light_worker`: shares each store's `ptr`, `read_only:true`, `borrowed:true` ⇒ `Drop` skips dealloc) instead of copying — a copy-elision, no semantic change. There is **exactly ONE clone path** — always the read-only borrow. The original design auto-selected it by heap size (`par_share_for`, a 2 MB threshold, `LOFT_PAR_SHARE=0`/`=1`); the redesign retired both, so no data-dependent switch decides how a `par` shares (`src/parallel.rs`: *"no byte-copy, no size heuristic, no second `thread::scope` dispatcher"*). Net: a par over a large read-only structure no longer pays a copy of the whole session heap per worker (measured flat vs 53× growth). Safety is compiler-carried (the dispatcher's `&Stores` signature proves parent-unwritten) + `read_only` runtime write-panic, and it is **ASan + TSan clean** (a positive-control race fires, so the clean run is non-vacuous). `--native` par is no longer separate: `src/codegen_runtime.rs`'s dispatchers call the same `parallel::parallel_workers` template, so both backends share the borrow.
