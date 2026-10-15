// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I72 — Parallel execution runtime: the worker pool a compiled program's `par` runs on

//! The worker pool: split a row range over N workers, each borrowing the parent stores
//! read-only with its own scratch, and collect their results in worker order.  A compiled
//! program's `par` runs on it directly (`codegen_runtime`), and the interpreter's worker
//! programs (`parallel`) run on it too — so it sits in the runtime a compiled program links,
//! and the interpreter's half does not (@PLN191).

#![allow(unused_imports)]
use crate::database::{Stores, WorkerStores};
use std::sync::Arc;
#[cfg(all(feature = "threading", not(feature = "wasm")))]
use std::thread;

// ── Plan-06 phase 1 step 4.5 — shared run_parallel_* template ─────────────────
//
// The run_parallel_* variants share a uniform shape:
//
//   1. Slice `n_rows` across N rayon tasks (the work-stealing pool).
//   2. Each worker BORROWS the parent stores read-only (`clone_for_light_worker`,
//      @PLN108) + a small self-allocated writable scratch, an Arc-bumped reference
//      to the WorkerProgram, and a row range `start..end`.
//   3. Each worker computes some R from its row range and returns it.
//   4. The main thread collects `Vec<R>` in worker-id order.
//
// `parallel_workers` captures that scaffolding; the per-variant `run_parallel_*`
// fns are thin wrappers that pass a closure describing the per-worker work.
// @PLN108: there is exactly ONE clone path (always the read-only borrow) — no
// byte-copy, no size heuristic, no second `thread::scope` dispatcher.

/// Common scope-spawn-collect scaffolding for the parallel runtime.
///
/// Spawns N rayon tasks (N = `min(n_threads, n_rows)`), each running
/// `f(start, end, worker_stores) -> R` over its row range with a read-only
/// `clone_for_light_worker` borrow of the parent.  Returns `Vec<R>` in
/// worker-id order.
///
/// Used by both the interpreter dispatchers in this file and the native
/// codegen dispatchers in `src/codegen_runtime.rs`.  The interpreter
/// path needs an `Arc<WorkerProgram>`; it constructs its own and the
/// closure captures it.  The native path constructs no extra context;
/// it just runs a Rust closure per row.
///
/// Implementation notes:
/// - `f` is captured by reference inside the spawn closures.  The
///   spawn closures are `move`, so they capture `&f` by `Copy`; the
///   `Sync` bound ensures concurrent calls are safe.
/// - Each worker gets its own `WorkerStores` via `clone_for_light_worker`
///   (read-only borrow of the parent + self-allocated scratch); the closure
///   may call `add_output_slot` / `take_slot` as needed and include them in
///   its `R` payload.
/// - `n_threads` is clamped to `min(n_threads, n_rows)`; for `n_rows == 0`
///   the caller is expected to short-circuit before calling.
#[cfg(feature = "threading")]
pub(crate) fn parallel_workers<R, F>(
    stores: &Stores,
    n_threads: usize,
    n_rows: usize,
    f: F,
) -> Vec<R>
where
    R: Send,
    F: Fn(usize, usize, WorkerStores) -> R + Sync,
{
    // Plan-06 phase 1.5 — use rayon's global work-stealing pool
    // instead of `std::thread::scope` per-call.  Per-call cost drops
    // from ~200 µs (thread spawn × N) to ~5 µs (task submission ×
    // N).  Nested par submits onto the same pool — no thread-count
    // explosion at depth K.  Matches the browser's
    // wasm-bindgen-rayon model so the two scheduler stories converge.
    use rayon::prelude::*;
    let threads = n_threads.max(1).min(n_rows.max(1));
    // @PLN117 step 0 — parallelism falsification instrument (see `WorkerTrace`):
    // when armed, prove this `par` dispatched across ≥2 workers.
    let trace = WorkerTrace::new();
    let trace_ref = &trace;
    let out = with_pool(|| {
        (0..threads)
            .into_par_iter()
            .map(|t| {
                trace_ref.record();
                let start = t * n_rows / threads;
                let end = (t + 1) * n_rows / threads;
                // @PLN108 R2 — the SINGLE clone: always borrow the parent read-only
                // (self-allocated scratch, R1), never byte-copy.  Safe on rayon exactly
                // as on `thread::scope`: `pool.install` blocks until the parallel iterator
                // has `collect`ed every task, so no worker (nor its read-only borrow of
                // `stores`) outlives this call, and `stores` outlives `pool.install`.
                // Workers never write the parent (@C93 → compile error), so the shared
                // read-only buffers are race-free.  `WorkerStores: Send` (unsafe impl),
                // so a borrowed one moves into a rayon task like the copy did.
                let worker_stores = unsafe { stores.clone_for_light_worker() };
                f(start, end, worker_stores)
            })
            .collect()
    });
    trace.report("parallel_workers", n_rows, threads);
    out
}

/// @PLN117 step 0 — runtime arm switch for the parallel-worker tracer.  The
/// wasm harness has no environment variables, so it flips this through a wasm
/// export (`set_par_trace_workers`) before running the instrument.
static PAR_TRACE_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Arm or disarm the parallel-worker tracer at runtime (@PLN117 step 0).
pub fn set_par_trace(on: bool) {
    PAR_TRACE_ON.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// @PLN117 step 0 — is the parallel-worker tracer armed?  Armed by the runtime
/// switch, or (native only) by `LOFT_TRACE_PAR_WORKERS` read once.  Off ⇒ the
/// dispatch chokepoint stays allocation-free.
pub(crate) fn par_trace_enabled() -> bool {
    if PAR_TRACE_ON.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    #[cfg(not(feature = "wasm"))]
    {
        use std::sync::OnceLock;
        static ON: OnceLock<bool> = OnceLock::new();
        *ON.get_or_init(|| std::env::var_os("LOFT_TRACE_PAR_WORKERS").is_some())
    }
    #[cfg(feature = "wasm")]
    {
        false
    }
}

/// @PLN117 step 0 — per-dispatch collector of the DISTINCT rayon worker indices
/// that ran a `par`, so the harness can prove a dispatch really crossed ≥2
/// workers (and that it can FAIL — `par(..., 1)` reports 1).  Used at EVERY
/// dispatch chokepoint (`parallel_workers` and `run_parallel_queue_ref`) so the
/// count is reported no matter which return-shape path a `par` takes.
///
/// Non-perturbing: `new()` allocates only when the tracer is armed; when off,
/// `record()` is a single `Option` branch and nothing is heap-allocated.
pub(crate) struct WorkerTrace(Option<std::sync::Mutex<std::collections::BTreeSet<usize>>>);

impl WorkerTrace {
    pub(crate) fn new() -> Self {
        Self(par_trace_enabled().then(|| std::sync::Mutex::new(std::collections::BTreeSet::new())))
    }

    /// Record the current worker index — call once per task inside the pool.
    /// A build without threads has exactly one worker, so it records index 0 and
    /// reports `distinct_workers=1`: the honest answer, not an absent one.
    pub(crate) fn record(&self) {
        if let Some(seen) = &self.0 {
            #[cfg(feature = "threading")]
            let index = rayon::current_thread_index().unwrap_or(usize::MAX);
            #[cfg(not(feature = "threading"))]
            let index = 0usize;
            seen.lock().unwrap().insert(index);
        }
    }

    /// Emit the distinct-worker count for `site` after the join.  Native →
    /// stderr; wasm → the program output (wasm stderr is dropped), where the
    /// browser harness reads it back from `compile_and_run`'s JSON.
    pub(crate) fn report(self, site: &str, n_rows: usize, threads: usize) {
        if let Some(seen) = self.0 {
            let indices = seen.into_inner().unwrap();
            let summary = format!(
                "loft: LOFT_TRACE_PAR_WORKERS {site} n_rows={n_rows} threads={threads} \
                 distinct_workers={} indices={indices:?}",
                indices.len(),
            );
            #[cfg(not(any(
                feature = "wasm",
                all(target_arch = "wasm32", not(target_os = "wasi"))
            )))]
            eprintln!("{summary}");
            #[cfg(feature = "wasm")]
            crate::wasm::output_push(&format!("{summary}\n"));
            // @PLN117 — the raw browser build (`loft --html`) has no stderr, so
            // the tracer would report into nothing.  Send it down the page's own
            // print path instead, which is what makes an in-browser `par` gate
            // able to READ how many workers actually ran.
            #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
            {
                let line = format!("{summary}\n");
                crate::loft_host_print(line.as_ptr(), line.len());
            }
        }
    }
}

/// Plan-06 phase 3b.1 — merge per-thread batches into one ordered vector.
///
/// Each `parallel_workers` call returns a `Vec<(start, batch)>` where every
/// batch's `i`-th element belongs at row `start + i` of the result.  This
/// helper writes the merge sequentially into a vector pre-filled with
/// `default` and returns it.
///
/// Used by `run_parallel_raw`, `run_parallel_text`, `run_parallel_int`, and
/// the codegen-runtime `run_native_workers_*` triple — five sites that
/// previously inlined the same 5-line loop.
///
/// `R: Clone` because `vec![default; n_rows]` clones the seed once per row.
/// For owning types (`String`, `Vec<…>`) the default is typically empty.
pub(crate) fn merge_batches<R: Clone>(
    batches: Vec<(usize, Vec<R>)>,
    n_rows: usize,
    default: R,
) -> Vec<R> {
    let mut results = vec![default; n_rows];
    for (start, batch) in batches {
        for (offset, val) in batch.into_iter().enumerate() {
            results[start + offset] = val;
        }
    }
    results
}

/// Run `f` on the rayon pool that drives loft's parallel dispatch (@PLN117 step 2).
///
/// - **Native:** install onto loft's private, lazily-built pool (`rayon_pool`).
/// - **Browser:** run `f` directly so the `.into_par_iter()` inside it dispatches
///   over the GLOBAL rayon pool the page installed — loft's own
///   (`wasm_threads::loft_pool_build`) or, for the wasm-bindgen bundle,
///   `initThreadPool(n)`.  A private `ThreadPoolBuilder` cannot spawn under wasm
///   — there are no OS threads to build on — so the global pool is the only one
///   with real workers.  Before the pool is installed (or when the host is not
///   cross-origin-isolated so the page never built one), the global pool has a
///   single thread and `f` runs sequentially on the caller: the never-break
///   sequential fallback (@PLN117 arc D).  Same bounds as `ThreadPool::install`,
///   so it is a drop-in at every call site.
#[cfg(feature = "threading")]
pub(crate) fn with_pool<R, F>(f: F) -> R
where
    R: Send,
    F: FnOnce() -> R + Send,
{
    #[cfg(not(browser_pool))]
    {
        rayon_pool().install(f)
    }
    #[cfg(browser_pool)]
    {
        f()
    }
}

/// Lazily-initialised private rayon pool for the native backend.  Uses rayon's
/// default thread count.  Not built under wasm — the browser arm of `with_pool`
/// uses the page's global pool instead (a private builder can't spawn OS threads
/// that don't exist in the browser).
#[cfg(all(feature = "threading", not(browser_pool)))]
pub(crate) fn rayon_pool() -> &'static rayon::ThreadPool {
    use std::sync::OnceLock;
    static POOL: OnceLock<rayon::ThreadPool> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .build()
            .expect("rayon pool init failed")
    })
}

/// Run `f` once per worker index and collect the results — on the pool when loft
/// was built with threads, sequentially when it was not.
///
/// [`parallel_workers`] is the same idea for dispatches that let this module
/// build the worker's view of the stores.  `run_parallel_queue_ref` builds its
/// own (it hands the worker's whole `Stores` back so the parent can adopt the
/// records it allocated), so it maps the indices through here instead.  Either
/// way, whether loft has threads is decided in ONE place per dispatch shape —
/// the caller is written once and reads the same in both builds.
#[cfg(feature = "threading")]
pub(crate) fn map_workers<R, F>(n_workers: usize, f: F) -> Vec<R>
where
    R: Send,
    F: Fn(usize) -> R + Sync + Send,
{
    use rayon::prelude::*;
    with_pool(|| (0..n_workers).into_par_iter().map(&f).collect())
}

/// Sequential [`map_workers`] for a build without threads: same slicing, same
/// order, same results — just one thread doing all of it.
#[cfg(not(feature = "threading"))]
pub(crate) fn map_workers<R, F>(n_workers: usize, f: F) -> Vec<R>
where
    F: Fn(usize) -> R,
{
    (0..n_workers).map(f).collect()
}

/// Sequential equivalent of `parallel_workers` for the `not(threading)` path.
/// Runs the closure once with `start = 0`, `end = n_rows`, and a single
/// `clone_for_light_worker()` snapshot.  Returned `Vec<R>` always has length 1.
#[cfg(not(feature = "threading"))]
pub(crate) fn parallel_workers<R, F>(
    stores: &Stores,
    _n_threads: usize,
    n_rows: usize,
    f: F,
) -> Vec<R>
where
    F: FnOnce(usize, usize, WorkerStores) -> R,
{
    let worker_stores = unsafe { stores.clone_for_light_worker() };
    vec![f(0, n_rows, worker_stores)]
}
