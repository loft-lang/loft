// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I72 — Parallel execution runtime

//! Parallel execution: dispatch a worker function over vector elements using OS threads.
//!
//! Enforces @FR-C-Par — the clause is two phases, a PARALLEL map that may run its workers
//! in any order, then a SEQUENTIAL consume of `res[0], res[1], …` in INDEX order — and
//! @FR-C-Det: the thread count is a performance knob, never a semantic one, so position
//! `i` always receives `worker(v[i])` whatever order the threads finished in.
//!
//! When the `threading` feature is enabled, each `run_parallel_*` function spawns OS
//! threads.  When `threading` is disabled (e.g. under WASM), the loop body runs
//! sequentially in the caller's thread — same results, no parallelism.  That is @FR-C-Det
//! taken to its limit: N = "no threads at all" is still the same answer.
//!
//! ⚠ The guarantee is CONDITIONAL (@FR-C-Impure): it holds only for a PURE worker.  A
//! worker that touches shared mutable state has no defined behaviour — loft pins no
//! interleaving — and nothing here detects that.  See `scopes::is_par_safe`, which
//! classifies it but is wired to nothing.

use crate::database::{Call, Stores, WorkerStores};
use crate::keys::DbRef;
use crate::state::State;
use crate::vector;
use std::sync::Arc;
#[cfg(feature = "threading")]
#[cfg(all(feature = "threading", not(feature = "wasm")))]
use std::thread;

/// Shared bytecode + library for a single `parallel_for` dispatch.
type ProgramRefs = (Arc<Vec<u8>>, Arc<Vec<crate::database::Call>>);

/// Wrapper for `*mut u8` that is `Send + Sync` for cross-thread direct writes.
/// SAFETY: callers must ensure non-overlapping writes and join all threads.
///
/// Present in a build without threads too, so the dispatch it appears in has ONE
/// body rather than a second, unexercised copy — the sequential `map_workers`
/// simply runs that body on one thread.
struct SendMutPtr(*mut u8);
unsafe impl Send for SendMutPtr {}
unsafe impl Sync for SendMutPtr {}

/// Immutable interpreter context shared across all worker threads.
///
/// All three fields are `Arc`-wrapped so that spawning many workers only
/// increments a reference count per thread instead of deep-copying the
/// bytecode and library on every `parallel_for` call.
#[derive(Clone)]
pub struct WorkerProgram {
    pub bytecode: Arc<Vec<u8>>,
    pub library: Arc<Vec<Call>>,
    /// Cached library index of `n_stack_trace`; `u16::MAX` = not found.
    /// Copied into each worker's `State::stack_trace_lib_nr` so that
    /// `stack_trace()` works inside parallel workers (fix #92).
    pub stack_trace_lib_nr: u16,
    /// Raw pointer to the parent's `Data` and the function-position table.
    /// `data_ptr` may be null if the caller does not have stack_trace context;
    /// when non-null, each worker's `State::data_ptr` and `fn_positions` are
    /// populated so `stack_trace()` returns named frames inside workers
    /// (fix #92 — without this the worker frame is shown as `<worker>` with
    /// no function name, file, or line).  The pointer is borrowed from a
    /// `&Data` held by the spawning frame, which outlives `thread::scope`.
    pub data_ptr: *const crate::data::Data,
    pub fn_positions: Arc<Vec<u32>>,
    /// Source-line lookup table (bytecode position → source line) shared from
    /// the parent State.  Workers populate `State::line_numbers` from this so
    /// `stack_trace()` can resolve real source lines instead of always
    /// reporting line 0 (fix #92 follow-on).
    pub line_numbers: Arc<std::collections::BTreeMap<u32, u32>>,
}

// Safety: WorkerProgram is read-only after construction; Call is fn ptr (Send).
unsafe impl Send for WorkerProgram {}
unsafe impl Sync for WorkerProgram {}

impl WorkerProgram {
    /// Clone the shared program references for a new worker `State` — O(1).
    fn clone_refs(&self) -> ProgramRefs {
        (Arc::clone(&self.bytecode), Arc::clone(&self.library))
    }

    /// Create a worker `State` from this program, with `stack_trace_lib_nr` propagated.
    ///
    /// Returns a [`WorkerState`] rather than a bare `State` so a worker's HALT cannot be
    /// forgotten.  Every `run_parallel_*` variant builds its state here — thirteen sites —
    /// and each previously dropped the state, and with it any `runtime_error` an `assert`
    /// or `panic` had raised inside the worker.  Recording at the one construction point
    /// makes coverage a property of the mechanism instead of of remembering thirteen
    /// call sites; the block form was fixed by hand first and the other three families
    /// stayed silent, which is the argument for doing it here.
    fn new_state(&self, worker_stores: WorkerStores) -> WorkerState {
        WorkerState(Some(self.new_state_inner(worker_stores)))
    }

    fn new_state_inner(&self, worker_stores: WorkerStores) -> State {
        let (bytecode, library) = self.clone_refs();
        let mut state = State::new_worker(worker_stores, bytecode, library);
        state.stack_trace_lib_nr = self.stack_trace_lib_nr;
        // Fix #92: propagate Data ptr + fn_positions so stack_trace() inside
        // a parallel worker can resolve the worker's d_nr → name/file/line.
        state.data_ptr = self.data_ptr;
        state.fn_positions.clone_from(&*self.fn_positions);
        state.line_numbers = (*self.line_numbers).clone();
        state
    }
}

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
                // Workers never write the parent (C93 → compile error), so the shared
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
fn par_trace_enabled() -> bool {
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
struct WorkerTrace(Option<std::sync::Mutex<std::collections::BTreeSet<usize>>>);

impl WorkerTrace {
    fn new() -> Self {
        Self(par_trace_enabled().then(|| std::sync::Mutex::new(std::collections::BTreeSet::new())))
    }

    /// Record the current worker index — call once per task inside the pool.
    /// A build without threads has exactly one worker, so it records index 0 and
    /// reports `distinct_workers=1`: the honest answer, not an absent one.
    fn record(&self) {
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
    fn report(self, site: &str, n_rows: usize, threads: usize) {
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
fn rayon_pool() -> &'static rayon::ThreadPool {
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

// ── Plan-06 phase 3a — Stitch policy enum (DESIGN.md D1) ─────────────────────
//
// The unified runtime entry point that phase 3b lands will be
// `n_parallel_native(stores, program, fn_pos, input, threads, fn, stitch)`,
// inner-dispatching on the `Stitch` policy.  Today the runtime branches
// on three native fns (`n_parallel_for_native` / `_text_native` /
// `_ref_native`) selected at codegen time by inspecting the worker fn's
// return type; phase 3 collapses those into one polymorphic dispatcher
// parameterised by `Stitch`.
//
// Phase 3a (this commit) lands just the type — no dispatcher, no opcode,
// no production caller.  The variants exist so that phase 3b/3d/3e have
// a stable shape to compile against, and so that `OpParallel(stitch_id)`
// payload encoding can be designed with the final variant set in mind.
//
// Phase 4c shape (DESIGN.md D1b):
//   `Concat` (no payload — sizes come from `Data::fn_return_type` and
//   the caller's `DispatchMode`).  Phase 4c retired the transitional
//   `{ elem_size, ret_size }` payload that earlier phases needed.

/// Plan-06 phase 3a (DESIGN.md D1a) — selects the per-call stitch
/// policy that the unified `n_parallel_native` dispatcher will use.
///
/// Runtime enum (not compile-time generics) so codegen emits one
/// `OpParallel(stitch_id)` opcode regardless of policy; the policy is
/// selected at parse / scope-analysis time and hardcoded into the
/// opcode stream.
///
/// **Currently dead code** — phase 3b lands the dispatcher that
/// consumes this; until then no production caller exists.  The type
/// is here so that downstream phases (3d codegen-embedding, 3e
/// `Stitch::Reduce` runtime) have a stable shape.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stitch {
    /// Run workers, drop their results.  Used by the fused for-loop
    /// when the body never references `r`, and by future
    /// `par_for_each`.  Lands in plan-06 phase 7.
    Discard,

    /// Each worker accumulates over its slice via a user-supplied
    /// monoidal fold; main thread combines per-worker partials with
    /// the same fold fn.  Used by `par_fold(input, init, fold,
    /// threads) -> U` and by the fused for-loop when the body is a
    /// pure single-accumulator update (auto-detected at scope
    /// analysis).  Runtime lands in phase 3e; surface in phase 7e.
    ///
    /// `fold_fn` is the fold function's def_nr; `init` lives in the
    /// opcode payload separately (sized to `U`).
    Reduce { fold_fn: u32 },

    /// Bounded queue: workers push, parent body pops in completion
    /// order (per D1c — input order is NOT preserved).  Used by the
    /// fused for-loop when the body references `r` or has side
    /// effects.  Runtime lands in phase 7b.
    ///
    /// `capacity` is the queue depth in elements (typically
    /// `2 × threads` to absorb worker-burstiness without blocking).
    Queue { capacity: u32 },
}

#[cfg(test)]
mod stitch_tests {
    use super::Stitch;

    #[test]
    fn enum_size_is_at_most_8_bytes() {
        // Largest variant is Reduce { fold_fn: u32 } or Queue {
        // capacity: u32 } — both 4 bytes payload + 1 disc + 3
        // padding = 8.  Validates the opcode payload budget.
        assert!(
            std::mem::size_of::<Stitch>() <= 8,
            "Stitch enum size {} bytes — opcode payload budget is 8",
            std::mem::size_of::<Stitch>()
        );
    }

    #[test]
    fn variants_distinguishable_by_match() {
        // ARC.md A4 (closed 2026-05-07) — Stitch::Concat removed.
        // The materialised result-vector dispatch was retired when
        // every par return type routed through the Queue family.
        let cases = [
            Stitch::Discard,
            Stitch::Reduce { fold_fn: 42 },
            Stitch::Queue { capacity: 16 },
        ];
        let names: Vec<&str> = cases
            .iter()
            .map(|s| match s {
                Stitch::Discard => "discard",
                Stitch::Reduce { .. } => "reduce",
                Stitch::Queue { .. } => "queue",
            })
            .collect();
        assert_eq!(names, vec!["discard", "reduce", "queue"]);
    }
}

// ── Plan-06 phase 2 — store-rebase infrastructure ─────────────────────────────
//
// Today's `copy_from_worker` (src/database/allocation.rs:409) deep-copies
// every struct result from a worker's WorkerStores into the parent's
// result vector via `copy_block` + `copy_claims`.  For a 100K-element
// result of 256-byte structs that's 25.6 MB of memcpy per parallel call.
//
// Phase 2's `StoreRebase` replaces deep-copy with **store adoption +
// DbRef translation**: the worker's allocated stores are moved into
// the parent's allocations table (store ownership transferred); any
// DbRef referring to those stores from the worker's local namespace
// is translated to the parent's new store_nr.  Bytes never move.
//
// See plan-06 DESIGN.md D2.1 (slot-marker design) and DESIGN.md D11c
// (the per-field worker-own / parent-shared / cross-worker translation
// rule).

/// Translates DbRefs across the worker→parent adoption boundary.
///
/// Built once per `parallel_for` call: every worker-allocated store
/// is adopted into the parent (via `Stores::adopt_store`), recording
/// its worker-local store_nr → parent-side store_nr mapping in a
/// per-worker `StoreRebase`.  When the parent later inspects a DbRef
/// the worker handed back, it calls `StoreRebase::translate` to rewrite
/// the `store_nr` field.
///
/// The translation is per-worker: each worker's DbRefs come from a
/// different worker-local namespace, so the rebase map is constructed
/// fresh for each worker.
///
/// Currently unused at runtime — phase 2b's narrow path
/// (`copy_from_worker_unowned`) handles owned-free struct returns
/// without needing the rebase walk.  The full step 2b will use this
/// for owned-field structs (text, refs).
#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct StoreRebase {
    /// Maps worker-local `store_nr` → parent-side `store_nr` for stores
    /// adopted from one worker's WorkerStores.  A DbRef whose `store_nr`
    /// is **not** in the map either points at a parent-shared store
    /// (the original parent allocations the worker only read, store_nr
    /// < parent_store_count) or at a worker-local store that wasn't
    /// adopted (which would be a codegen bug).
    pub map: std::collections::HashMap<u16, u16>,
    /// Number of parent-side stores at the moment this rebase was
    /// constructed (before any worker output was adopted).  Set via
    /// `with_parent_count`; used by `translate` to apply the
    /// 3-category rule from DESIGN.md D11c:
    ///   - `db.store_nr` in `map` → worker-own, translate
    ///   - `db.store_nr < parent_store_count` → parent-shared, pass through
    ///   - otherwise → cross-worker (codegen bug); panic in debug,
    ///     log + pass through in release
    pub parent_store_count: u16,
}

#[allow(dead_code)]
impl StoreRebase {
    /// Construct an empty rebase map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct an empty rebase map with the parent-side store count
    /// captured at the moment of construction.  Adoption monotonically
    /// increases the parent's store count, so any pre-existing parent
    /// store has a `store_nr < parent_store_count` — which is how
    /// `translate` separates parent-shared refs from worker-own ones.
    #[must_use]
    pub fn with_parent_count(parent_store_count: u16) -> Self {
        Self {
            map: std::collections::HashMap::new(),
            parent_store_count,
        }
    }

    /// Record a worker-local → parent-side translation.
    pub fn add(&mut self, worker_local: u16, parent_nr: u16) {
        self.map.insert(worker_local, parent_nr);
    }

    /// Translate a DbRef from the worker's local namespace to the
    /// parent's namespace.  Implements the 3-category rule from
    /// DESIGN.md D11c:
    ///
    /// | `db.store_nr` shape | Category | Action |
    /// |---|---|---|
    /// | In `map` | Worker-own | Translate to parent-side `store_nr` |
    /// | `< parent_store_count` | Parent-shared | Pass through |
    /// | Otherwise | Cross-worker | Debug-panic; release-pass-through |
    ///
    /// The cross-worker case is a codegen bug — workers should never
    /// hand out DbRefs into a sibling worker's stores.  Phase 5's
    /// auto-light analyser tightens this further; until then we keep
    /// the release path defensive (pass through unchanged).
    #[must_use]
    pub fn translate(&self, db: &DbRef) -> DbRef {
        if let Some(&parent_nr) = self.map.get(&db.store_nr) {
            return DbRef {
                store_nr: parent_nr,
                rec: db.rec,
                pos: db.pos,
            };
        }
        if db.store_nr < self.parent_store_count {
            // Parent-shared (input store, stdlib const, parent-allocated
            // read-only).  Pass through.
            return *db;
        }
        // Cross-worker: codegen bug.  Debug builds panic so the bug
        // surfaces immediately; release builds log + pass through so a
        // released runtime degrades gracefully on a corner the test
        // suite missed.
        debug_assert!(
            false,
            "StoreRebase::translate cross-worker DbRef: \
             store_nr={} not in rebase map and not parent-shared \
             (parent_store_count={})",
            db.store_nr, self.parent_store_count
        );
        eprintln!(
            "loft: StoreRebase cross-worker DbRef store_nr={} (parent_store_count={}); passing through unchanged",
            db.store_nr, self.parent_store_count
        );
        *db
    }
}

/// Plan-06 phase 2 — type-driven recursive walk that translates every
/// DbRef field inside a record so the worker's store-local references
/// become valid in the parent's namespace after store adoption.
///
/// `record_ref` is a parent-side DbRef pointing at a record (typically
/// in an adopted worker output store) that needs its DbRef-shaped
/// fields translated.  `record_type` describes the layout used to
/// locate those fields: a `Type::Reference(struct_d, _)` looks up the
/// struct's attribute types from `data`, a `Type::Tuple(elems)` reads
/// `elems` directly.  Any other shape has no DbRef fields and the walk
/// returns immediately.
///
/// Cycles are broken via `visited` (keyed by `(store_nr, rec, pos)`).
///
/// Currently unused at runtime — phase 2b's reference-path switch is
/// the first consumer.  Lives here so it can be unit-tested in
/// isolation against synthetic Stores before the larger phase-2b
/// surgery wires it into `run_parallel_ref`.
#[allow(dead_code)]
pub fn rebase_walk_record(
    stores: &mut Stores,
    record_ref: &DbRef,
    record_type: &crate::data::Type,
    data: &crate::data::Data,
    map: &StoreRebase,
    visited: &mut std::collections::HashSet<(u16, u32, u32)>,
) {
    use crate::data::{Type, owned_elements};
    let key = (record_ref.store_nr, record_ref.rec, record_ref.pos);
    if !visited.insert(key) {
        return;
    }
    let elem_types: Vec<Type> = match record_type {
        Type::Reference(struct_d, _) | Type::Enum(struct_d, true, _) => data
            .def(*struct_d)
            .attributes
            .iter()
            .map(|a| a.typedef.clone())
            .collect(),
        Type::Tuple(elems) => elems.clone(),
        _ => return,
    };
    let owned: Vec<(usize, usize)> = owned_elements(&elem_types);
    for (offset, idx) in owned {
        let field_pos = record_ref.pos + offset as u32;
        let field_tp = &elem_types[idx];
        // `owned_elements` already decided membership, so the only question left is which
        // of the two owning shapes this is.  Restating its variant list here is what the
        // `_` arm used to guard with a `debug_assert` — absent from release builds and from
        // the ordinary debug build too, since the loft package turns debug assertions off.
        // One list, asked once, cannot drift.  Peeled for the same reason it is peeled
        // there: a nullable element holds the bare one's `DbRef` (@FR-L-Null).
        //
        // A text field is the one owning shape with nothing to translate: it holds a
        // `Str { ptr, len }` whose pointer is a raw heap address, not a store-relative
        // slot, and adoption does not move those bytes.
        if matches!(field_tp.base(), Type::Text(_)) {
            continue;
        }
        let store = &mut stores.allocations[record_ref.store_nr as usize];
        let cur: DbRef = store.read::<DbRef>(record_ref.rec, field_pos);
        let translated = map.translate(&cur);
        if translated != cur {
            store.write::<DbRef>(record_ref.rec, field_pos, translated);
        }
        // Recurse into the pointed-at record using the field's declared type so the next
        // layer's `owned_elements` call sees the right shape.
        if translated.store_nr != u16::MAX
            && (translated.store_nr as usize) < stores.allocations.len()
        {
            rebase_walk_record(stores, &translated, field_tp, data, map, visited);
        }
    }
}

/// Channel-based parallel: one u64 per row.
///
/// Plan-06 phase 3c: production paths now use `run_parallel_direct`
/// uniformly (it handles every inline byte width 1..=8 via per-worker
/// output slots).  This helper survives because `tests/threading.rs`
/// drives it directly to verify the parallel runtime returns one
/// `u64` per row in the right order.
///
/// # Panics
/// Panics if a worker thread panics.
#[allow(clippy::too_many_arguments)]
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
#[cfg_attr(not(feature = "threading"), allow(clippy::needless_pass_by_value))]
#[allow(dead_code)] // tested by tests/threading.rs but no production caller post-phase-3c
#[must_use]
pub fn run_parallel_raw(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    return_size: u32,
    n_threads: usize,
    extra_args: &[u64],
) -> Vec<u64> {
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return Vec::new();
    }
    let input_t = *input;
    let extras = extra_args.to_vec();
    let prog = Arc::new(program);
    let batches = parallel_workers(stores, n_threads, n_rows, |start, end, ws| {
        let mut state = prog.new_state(ws);
        let mut batch = Vec::with_capacity(end - start);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            batch.push(state.execute_at_raw(fn_pos, &row_ref, &extras, return_size));
        }
        (start, batch)
    });
    merge_batches(batches, n_rows, 0u64)
}

/// Parallel text returns: workers copy `Str` to owned `String` before state drops.
/// # Panics
/// Panics if a worker thread panics.
#[allow(clippy::too_many_arguments)]
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
#[cfg_attr(
    not(feature = "threading"),
    allow(clippy::needless_pass_by_value, dead_code)
)]
#[must_use]
pub fn run_parallel_text(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    n_rows: usize,
    n_hidden_text: usize,
    primitive_input_size: u32,
    tuple_input_types: Option<Vec<crate::data::Type>>,
) -> Vec<String> {
    if n_rows == 0 {
        return Vec::new();
    }
    let input_t = *input;
    let extras = extra_args.to_vec();
    let prog = Arc::new(program);
    let prim_in = primitive_input_size;
    let tuple_types_arc = tuple_input_types.map(Arc::new);
    // Plan-06 phase 1b: workers write text results into a per-worker
    // output Store slot — same path text always uses inside loft.
    // Each worker reserves a single record holding `row_count` u32
    // s_pos pointers (`set_str` interns each string into the slot's
    // store).  After join, the parent reads each adopted slot's
    // s_pos array, calls `get_str` to retrieve the bytes, and copies
    // them into the result `Vec<String>`.  The `Vec<String>` per-
    // thread accumulation is gone — replaced by N store slots.
    let batches = parallel_workers(stores, n_threads, n_rows, |start, end, mut ws| {
        let tuple_types_arc = tuple_types_arc.as_ref().map(Arc::clone);
        let row_count = end - start;
        // The s_pos array record needs 4 bytes for the record-size header
        // (fld 0..4) plus 4 bytes per row; writes start at fld 4 — writing
        // from fld 0 would overwrite the record's own header word.
        let array_words = (4 + row_count * 4).div_ceil(8) as u32;
        let slot = ws.add_output_slot(array_words);
        let mut state = prog.new_state(ws);
        let array_rec = state.database.allocations[slot.store_nr as usize].claim(array_words);
        for (local_idx, row_idx) in (start..end).enumerate() {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            // Deliver the element the way the worker's first parameter expects.
            // Without it a `vector<integer>` / range / text element was always
            // pushed as a 12-byte DbRef → garbage worker arg (text input: SIGSEGV).
            let arg = worker_row_arg(
                &state.database,
                &row_ref,
                prim_in,
                element_size,
                tuple_types_arc.as_deref().map(Vec::as_slice),
            );
            let s = state.execute_at_text(fn_pos, arg, &extras, n_hidden_text);
            let slot_store = &mut state.database.allocations[slot.store_nr as usize];
            let s_pos = slot_store.set_str(&s);
            slot_store.set_u32_raw(array_rec, 4 + (local_idx as u32) * 4, s_pos);
        }
        (
            start,
            row_count,
            slot.store_nr,
            array_rec,
            state.into_database(),
        )
    });
    let mut results = vec![String::new(); n_rows];
    for (start, count, slot_nr, array_rec, worker_db) in batches {
        let slot_store = &worker_db.allocations[slot_nr as usize];
        for local_idx in 0..count {
            let s_pos = slot_store.get_u32_raw(array_rec, 4 + (local_idx as u32) * 4);
            results[start + local_idx] = slot_store.get_str(s_pos).to_string();
        }
    }
    results
}

/// Plan-06 PRIORITY.md spine step 8d.0 — `Stitch::Queue` runtime for
/// reference / struct-enum-payload / vector return shapes.
///
/// Builds on `run_parallel_ref` but post-processes the per-worker
/// batches into a flat `Vec<DbRef>` ordered by input row, with each
/// DbRef rebased into the parent's store namespace via
/// `Stores::adopt_worker_excess` + `rebase_walk_record`.  Returns the
/// rebased refs alongside the list of adopted parent-side store_nrs
/// so `n_parallel_buf_drop_ref` (8d.1) can free them when the body
/// loop completes.
///
/// Adoption (vs. deep-copy) is the cost-saving move that makes Queue
/// for refs cheaper than the legacy Concat path: workers' output
/// stores are moved into the parent's allocations table — no
/// per-record memcpy — and DbRef fields are translated in place via
/// `rebase_walk_record` so cross-record references stay valid.
///
/// `ret_type` is the worker's declared return type
/// (`Type::Reference(_, _)`, `Type::Enum(_, true, _)`, or
/// `Type::Vector(_, _)`); used by `rebase_walk_record` to find DbRef
/// fields inside each adopted record.  Pass through unchanged from
/// the parser-side gate.
///
/// Currently `#[allow(dead_code)]` — 8d.1 is the first call-site
/// consumer (`n_parallel_queue_ref` native fn).
#[allow(dead_code, clippy::too_many_arguments)]
#[must_use]
pub fn run_parallel_queue_ref(
    stores: &mut Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    n_rows: usize,
    ret_type: &crate::data::Type,
    data: &crate::data::Data,
    n_hidden_dests: usize,
    primitive_input_size: u32,
    tuple_input_types: Option<Vec<crate::data::Type>>,
) -> (Vec<DbRef>, Vec<u16>) {
    if n_rows == 0 {
        return (Vec::new(), Vec::new());
    }
    let prim_in = primitive_input_size;
    let tuple_types_arc = tuple_input_types.map(Arc::new);
    // ARC.md A2: shared atomic dispenser for parent-namespace slot
    // indices.  Every worker's `database_named` call pulls a unique
    // index from this counter and records it in
    // `worker_allocated_indices`; the worker extends its own
    // `allocations` clone to fit.  Replaces 8d.3's fixed
    // `[off, off+SLOTS_PER_THREAD)` ranges with unbounded growth.
    // Cross-worker collisions are impossible because the atomic
    // dispenses globally-unique values.
    // Every store the parent already had before this dispatch.  A worker's result may
    // POINT at one of them — an identity worker (`fn w(x: Cell) -> Cell { x }`) returns the
    // element it was handed, whose record lives in the caller's own store — and such a
    // store is not the queue's to adopt.  The boundary is the same one `take_all_owned`
    // and `StoreRebase` already use (`parent_store_count`); the revive walk below did not
    // have it, so it adopted the CALLER's store and `par_buf_drop_ref` freed it: the
    // caller's vector then pointed into freed records, which read back correct bytes until
    // something reused the arena (visible only under `LOFT_POISON=1`).
    let parent_store_count = stores.allocations.len() as u16;
    let dispenser = stores.make_worker_slot_dispenser();
    let n_workers = n_threads.max(1).min(n_rows.max(1));
    let input_t = *input;
    let extras = extra_args.to_vec();
    let prog = Arc::new(program);
    let stores_immut: &Stores = &*stores;
    let prog_ref = &prog;
    let extras_ref = &extras;
    let dispenser_ref = &dispenser;
    // @PLN117 arc C — instrument the struct/ref/vector return path too, so the
    // memory-model harness can prove THIS (allocation + adoption + rebase heavy)
    // dispatch really crossed ≥2 workers under shared memory, not just the
    // simple `parallel_workers` path.
    let trace = WorkerTrace::new();
    let trace_ref = &trace;
    let batches: Vec<(Vec<(usize, DbRef)>, Stores)> = map_workers(n_workers, |t| {
        trace_ref.record();
        let start = t * n_rows / n_workers;
        let end = (t + 1) * n_rows / n_workers;
        // @PLN108 R4b: queue_ref borrows the parent read-only like every other
        // par worker — but with ZERO scratch stores.  Its output slots are
        // assigned by the shared `dispenser` (floor `parent_len + 1`); pre-seeded
        // scratch would push the worker's stack store into that range and the
        // dispenser would climb into a live slot ("Allocating a used store" —
        // probe 3).  Scratch-free ⇒ layout identical to the retired byte-copy
        // clone (borrows at `0..parent_len`, stack at `parent_len`), so the
        // dispenser math is unchanged.
        let mut ws = unsafe { stores_immut.clone_for_light_worker_with_scratch(0) };
        ws.stores.free_bits.clear();
        ws.stores.disable_slot_reuse = true;
        // The worker's stack store (allocated by
        // `prog.new_state(ws)` via `db.database(1000)`) MUST
        // land at `allocations.len()` (push-at-end), NOT
        // through the dispenser — the stack store is a
        // worker-local scratch buffer; routing it through
        // the parent-namespace dispenser would put parent at
        // risk of swapping that scratch buffer back in.
        // Leave `worker_slot_dispenser = None` until
        // `new_state` returns, then attach it for the
        // worker's user-code allocations.
        let tuple_types_arc = tuple_types_arc.as_ref().map(Arc::clone);
        let mut state = prog_ref.new_state(ws);
        state.database.worker_slot_dispenser = Some(Arc::clone(dispenser_ref));
        state.database.worker_allocated_indices.clear();
        let mut batch = Vec::with_capacity(end - start);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            // ARC.md A6.a — when the worker fn has hidden
            // caller-supplied destination params (added by
            // `ref_return` for Vector / Reference / Enum-payload
            // returns), allocate a fresh storage slot per
            // hidden arg in the worker's output store and pass
            // it as a 12-byte param after the input element.
            // The worker writes its result into the destination;
            // its DbRef survives in the worker's allocations
            // table and gets adopted + rebased back to parent
            // alongside the rest.
            // ARC.md A6.a — pre-allocate a real backing store
            // (NOT `state.database.null()`) for each hidden
            // destination.  `null()` returns a u32::MAX-sized
            // sentinel store: subsequent record claims silently
            // fail, so `out += [i]` writes go nowhere and the
            // returned vector is empty.  A 100-word real store
            // is enough seed; the standard claim path grows it
            // as the worker appends.
            let hidden_dests: Vec<DbRef> = (0..n_hidden_dests)
                .map(|_| state.database.database(100))
                .collect();
            // Every element reaches the worker the way its first parameter reads it.
            // This route used to answer `DbRef` for a text or wide element as well,
            // which SIGSEGV'd a struct-returning worker over `vector<text>`.
            let arg = worker_row_arg(
                &state.database,
                &row_ref,
                prim_in,
                element_size,
                tuple_types_arc.as_deref().map(Vec::as_slice),
            );
            let r = state.execute_at_ref(fn_pos, arg, &hidden_dests, extras_ref);
            // @PLAN59 witness-free (mirrors the plain-call
            // `OpFreeRefIfDistinct` pair): a worker whose tail built
            // its OWN store (literal return — the signature-time
            // buffer stays unwritten) leaves the pre-allocated dest
            // orphaned; free every dest the result did not adopt.
            for d in &hidden_dests {
                if d.store_nr != r.store_nr {
                    state.database.free(d);
                }
            }
            batch.push((row_idx, r));
        }
        (batch, state.into_database())
    });
    trace.report("run_parallel_queue_ref", n_rows, n_workers);
    let null_db = DbRef::NULL;
    let mut refs: Vec<DbRef> = vec![null_db; n_rows];
    let mut adopted: Vec<u16> = Vec::new();
    // Grow parent's allocations to fit the dispenser's high-water
    // mark, then iterate each worker's allocated-indices list and
    // swap the worker's slot at that index into parent.  The
    // dispenser's final value is one-past-the-last-index, so the
    // parent's allocations vec must reach that length.
    let high_water = dispenser.load(std::sync::atomic::Ordering::Relaxed) as usize;
    stores.grow_allocations_to(high_water);
    for (batch, mut worker_stores) in batches {
        // Swap each slot this worker allocated back into the parent.  A worker
        // may have allocated then freed a slot mid-call (e.g. FreeRef on an
        // intermediate temp); the swap is unconditional and `revive_record_chain`
        // below decides which slots stay active.  Store isolation (no
        // cross-thread slot aliasing) is the invariant enforced ONCE in
        // `Stores::swap_in_worker_slots` — each worker lists only its own indices.
        stores.swap_in_worker_slots(&mut worker_stores);
        for (i, src_ref) in batch {
            refs[i] = src_ref;
        }
    }
    // Revive every result chain.  Without this, parent reads from
    // worker-freed slots return uninitialised bytes (variant-
    // reassignment workers like `if neg { v = Fail{...} }` fall
    // into this case because their codegen unconditionally
    // `OpFreeRef`s every variant temp at end-of-fn).
    let mut visited: std::collections::HashSet<u16> = std::collections::HashSet::new();
    for r in &refs {
        if r.store_nr == u16::MAX {
            continue;
        }
        revive_record_chain(
            stores,
            r,
            ret_type,
            data,
            &mut visited,
            &mut adopted,
            parent_store_count,
        );
    }
    // Update `max` to cover any slots the workers allocated past
    // parent's pre-dispatch max.  Slots beyond `high_water` were
    // never dispensed, so they don't exist.
    if (high_water as u16) > stores.max {
        stores.max = high_water as u16;
    }
    (refs, adopted)
}

/// 8d.3 — mark a result record's slot (and any DbRef sub-field
/// chain) as active in the parent's namespace, after `mem::swap`
/// has moved worker data into parent.  Mirrors `rebase_walk_record`'s
/// recursion shape but with a "revive" action (clear `free` flag,
/// reset `ref_count`, clear `free_bits`) instead of "translate".
///
/// The worker may have freed the result store during scope
/// cleanup; the bytes are still in memory but the store is flagged
/// `free=true`, which would cause future parent allocations to
/// overwrite it.  Reviving sets `free=false, ref_count=1` so the
/// slot is owned by `par_ref_buffer_stack`'s adopted list — freed
/// later by `n_parallel_buf_drop_ref`.
fn revive_record_chain(
    stores: &mut Stores,
    record_ref: &DbRef,
    record_type: &crate::data::Type,
    data: &crate::data::Data,
    visited: &mut std::collections::HashSet<u16>,
    adopted: &mut Vec<u16>,
    parent_store_count: u16,
) {
    use crate::data::{Type, owned_elements};

    let store_nr = record_ref.store_nr;
    if !visited.insert(store_nr) {
        return;
    }
    // REVIVE and ADOPT are two different claims, and only the second is gated.
    //
    // Reviving is safe for any store the result chain reaches: a worker may have freed a
    // slot during its own scope cleanup, and the parent needs it live again to read the
    // result.  ADOPTING says "this store is the queue's, free it at `par_buf_drop_ref`" —
    // which is only true of a store the WORKER made.  A store the parent already owned is
    // reached whenever the worker returns something it was given (`fn w(x: Cell) -> Cell
    // { x }` returns the caller's own element), and adopting that handed the drop a store
    // the caller was still using: the caller's vector then pointed into freed records,
    // reading back correct bytes until something reused the arena — visible only under
    // `LOFT_POISON=1`.  The boundary is the one `take_all_owned` and `StoreRebase` already
    // use.
    let worker_made = store_nr >= parent_store_count;
    if (store_nr as usize) >= stores.allocations.len() {
        return;
    }
    {
        let store = &mut stores.allocations[store_nr as usize];
        store.unlock();
        if store.free {
            store.free = false;
        }
    }
    let wi = store_nr as usize / 64;
    let bi = store_nr as usize % 64;
    if wi < stores.free_bits.len() {
        stores.free_bits[wi] &= !(1u64 << bi);
    }
    if worker_made {
        adopted.push(store_nr);
    }

    // For plain `Type::Reference(struct_d, _)`, `attributes` lists
    // the struct's fields — safe to walk for owned DbRef sub-fields.
    // For `Type::Enum(_, true, _)`, `attributes` lists the parent
    // enum's *variants*, NOT the active variant's fields — walking
    // them as DbRef offsets reads garbage and segfaults.  Reaching
    // the active variant requires reading the discriminant + type
    // tag; deferred until a test with nested DbRef-in-variant
    // payload exposes the gap (the spine's current corpus has no
    // such test — Pass/Fail variants in `par_struct_to_struct_enum_t4`
    // hold only byte / integer / text, none owned-DbRef).
    let elem_types: Vec<Type> = match record_type {
        Type::Reference(struct_d, _) => data
            .def(*struct_d)
            .attributes
            .iter()
            .map(|a| a.typedef.clone())
            .collect(),
        Type::Tuple(elems) => elems.clone(),
        _ => return,
    };
    let owned: Vec<(usize, usize)> = owned_elements(&elem_types);
    for (offset, idx) in owned {
        let field_pos = record_ref.pos + offset as u32;
        let field_tp = &elem_types[idx];
        // The same one-list reading as `rebase_walk_record` above: membership is
        // `owned_elements`', and `text` is the shape that owns no store.
        // `text` again: a heap `String` pointer, not a store-relative slot.
        if matches!(field_tp.base(), Type::Text(_)) {
            continue;
        }
        let cur: DbRef =
            stores.allocations[store_nr as usize].read::<DbRef>(record_ref.rec, field_pos);
        if cur.store_nr != u16::MAX && (cur.store_nr as usize) < stores.allocations.len() {
            revive_record_chain(
                stores,
                &cur,
                field_tp,
                data,
                visited,
                adopted,
                parent_store_count,
            );
        }
    }
}

/// Parallel integer returns: one `i64` per row, original order.
///
/// Plan-06 phase 4b': the loft-surface `parallel_for_int` (string-based
/// dispatch) and its `n_parallel_for_int` native impl have been retired,
/// but this helper stays as a tested Rust-side API — `tests/threading.rs`
/// drives it directly to verify the parallel runtime semantics.  It's
/// equivalent to `run_parallel_raw` with `return_size=8`, but exists as
/// a separate fn for the no-extras / fixed-i64-return ergonomics that
/// the threading tests rely on.
///
/// # Panics
/// Panics if a worker thread panics.
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
#[cfg_attr(not(feature = "threading"), allow(clippy::needless_pass_by_value))]
#[allow(dead_code)] // tested by tests/threading.rs but no production caller post-phase-4b'
#[must_use]
pub fn run_parallel_int(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
) -> Vec<i64> {
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return Vec::new();
    }
    let input_t = *input;
    let prog = Arc::new(program);
    let batches = parallel_workers(stores, n_threads, n_rows, |start, end, ws| {
        let mut state = prog.new_state(ws);
        let mut batch = Vec::with_capacity(end - start);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            batch.push(state.execute_at(fn_pos, &row_ref));
        }
        (start, batch)
    });
    merge_batches(batches, n_rows, i64::MIN)
}

// ── Plan-06 ARC.md A5 — Stitch::Reduce runtime ──────────────────────────────
//
// Workers each accumulate over their slice of the input via a user-supplied
// monoidal fold; the main thread combines per-worker partials by repeatedly
// applying the same fold function (now treating the partials as the row
// values).  Returns the final scalar accumulator.
//
// V1 scope: integer accumulator + integer row type.  Both per-row and
// combine use the same `fold(acc: integer, row: integer) -> integer`
// signature.  Future extensions (heterogeneous T / R, Float / Single accs,
// text-fold) layer on top once the runtime + dispatch shape stabilises.
//
// The monoid contract — associativity + identity — is the user's
// responsibility; the runtime preserves left-to-right order within each
// worker, then combines partials in worker-completion order (matching
// rayon's task-queue ordering).

/// Plan-06 ARC.md A5 — `Stitch::Reduce` worker runtime.
///
/// Splits `input` (a `vector<integer>`) across `n_threads` workers; each
/// worker accumulates over its slice via `fold(acc, row) -> acc`, starting
/// from `init`.  Main thread combines per-worker partials with the same
/// `fold` (treating partials as the second arg).
///
/// `fold_fn_pos` is the bytecode position of the user-supplied fold fn,
/// declared as `fn fold(acc: integer, row: integer) -> integer`.  Both
/// arguments are i64; the `acc` slot is param 0, the `row` slot is param 1.
///
/// `extra_args` are additional context args appended after `acc` + `row`
/// in the worker's parameter list — currently always empty for `par_fold`
/// callers; reserved for future surface extensions.
///
/// # Panics
/// Panics if a worker thread panics.  Empty input returns `init` without
/// dispatching workers.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn run_parallel_fold(
    stores: &Stores,
    program: WorkerProgram,
    fold_fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    init: i64,
    n_threads: usize,
    extra_args: &[u64],
) -> i64 {
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return init;
    }
    let input_t = *input;
    let extras = extra_args.to_vec();
    let prog = Arc::new(program);
    // Each worker accumulates over its slice; returns its partial acc.
    let partials: Vec<(usize, i64)> =
        parallel_workers(stores, n_threads, n_rows, |start, end, ws| {
            let mut state = prog.new_state(ws);
            let mut acc = init;
            for row_idx in start..end {
                let row_ref = vector::get_vector(
                    &input_t,
                    element_size,
                    row_idx as i64,
                    &state.database.allocations,
                );
                // Read the row's i64 value.  V1 restricts row type to
                // integer; future extensions read other widths via the
                // existing read_primitive_at* family.
                let row_val = read_primitive_at(&state.database, &row_ref, element_size);
                // Build the extras vector for this call: [row, ...callsite_extras].
                // The fold fn's parameter list is `(acc, row, ...extras)` so
                // `row` slots in as the first "extra" after the primary `acc` arg.
                let mut call_extras = Vec::with_capacity(1 + extras.len());
                call_extras.push(row_val);
                call_extras.extend_from_slice(&extras);
                // execute_at_raw_primitive_input pushes (acc, ...extras) onto
                // the worker's stack frame; the fold fn reads acc as param 0,
                // row as param 1.  Returns the new acc as u64.
                let new_acc = state.execute_at_raw_primitive_input(
                    fold_fn_pos,
                    acc as u64,
                    8, // acc is i64
                    &call_extras,
                    8, // result is i64
                );
                acc = new_acc as i64;
            }
            (start, acc)
        });
    // Combine partials in worker-completion order.  Use a single-threaded
    // State borrowed back from the parent; for the v1 we do this on the
    // main thread (no `parallel_workers` indirection — small N).
    //
    // The combine call uses the SAME fold fn, with `partial_acc` as the
    // row argument.  Per the monoid contract, fold(a, b) == fold(b, a)
    // shouldn't matter (associative + identity), but the runtime
    // preserves left-to-right ordering for users who care.
    let mut sorted_partials: Vec<(usize, i64)> = partials;
    sorted_partials.sort_by_key(|(start, _)| *start);
    // Build a transient main-thread State for the combine pass.  The
    // worker programs already hold a reference to the bytecode + library
    // we need; reuse one.
    if sorted_partials.is_empty() {
        return init;
    }
    let mut combine_state = prog.new_state(unsafe { stores.clone_for_light_worker() });
    let mut acc = init;
    for (_, partial) in sorted_partials {
        let new_acc = combine_state.execute_at_raw_primitive_input(
            fold_fn_pos,
            acc as u64,
            8,
            &[partial as u64],
            8,
        );
        acc = new_acc as i64;
    }
    acc
}

// ── Plan-06 PRIORITY.md spine step 2 — Stitch::Discard runtime ──────────────
//
// Discard is the simplest of the three streaming Stitch policies: workers run,
// results are dropped on the floor, worker output stores deallocate at thread
// join.  No order preservation, no per-element allocation, no merge pass.
//
// Used by the fused for-par lowering when the body never references `r`, reached
// through the `par_discard` worker name (`parser/collections.rs`).  Self-tested in
// `tests/threading.rs::par_discard_*`.

/// Plan-06 spine step 2 — `Stitch::Discard` worker runtime.
///
/// Iterates `input` rows in parallel across `n_threads`, dispatching the
/// worker fn at `fn_pos` per row and **dropping** the return value.  The
/// worker is run for its side effects (e.g. `log_info`, host_io) — workers
/// must be par-safe (D8 rule, enforced by phase 5b').
///
/// The worker's return type is irrelevant: dispatch goes through
/// `execute_at_raw` with the caller-supplied `return_size`, and the result
/// is `let _`-bound.  `return_size = 0` is also valid (void worker — caller
/// passes 0 and the post-return stack drain is a no-op).
///
/// `extra_args` are extra context args that follow the row arg in the
/// worker's parameter list (matches the `execute_at_raw` convention used by
/// `run_parallel_direct` / `run_parallel_int`).
///
/// # Panics
///
/// Panics if a worker thread panics.  No worker output is collected, so a
/// non-panicking worker that produces an invalid value (e.g. `i64::MIN`
/// sentinel) is silently absorbed — the caller is expected to enforce
/// par-safety + side-effect-correctness via phase 5's analyser before
/// reaching this dispatcher.
#[allow(clippy::too_many_arguments)]
pub fn run_parallel_discard(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    return_size: u32,
    primitive_input_size: u32,
    tuple_input_types: Option<Vec<crate::data::Type>>,
    n_hidden_text: usize,
    n_hidden_dests: usize,
) {
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return;
    }
    // @PLN108 R2/R3: `parallel_workers` always borrows the parent read-only, so there is
    // no copy-vs-borrow choice left — this is the single path.
    let input_t = *input;
    let extras = extra_args.to_vec();
    let prog = Arc::new(program);
    let prim_in = primitive_input_size;
    let tuple_types_arc = tuple_input_types.map(Arc::new);
    let _: Vec<()> = parallel_workers(stores, n_threads, n_rows, |start, end, ws| {
        let tuple_types_arc = tuple_types_arc.as_ref().map(Arc::clone);
        let mut state = prog.new_state(ws);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            // Discard drops the result, so only the CALL has to be right — and a call
            // is right when the worker's parameter slots are filled the way its
            // signature reads them.  Two things decide that, and this route used to
            // get both wrong for every shape but one, invisibly: the results it drops
            // are the only thing most workers produce, so nothing could witness the
            // fault until a worker that also PRINTS put three rows through it and got
            // one wrong value three times (loft#987).
            //
            // The INPUT is `worker_row_arg`'s single answer.  It used to be decided
            // here, and only when the worker had no HIDDEN parameters: a wide row had
            // no `WorkerArg` spelling, so a tuple reaching a text- or heap-returning
            // worker fell to the `DbRef` arm and the worker read the pointer's bits
            // (loft#1055).
            //
            // The HIDDEN parameters: a text-returning worker is compiled with a
            // `__work_N` buffer parameter and a heap-returning one with a destination
            // parameter, both after the row.  Leaving them off shifts every slot the
            // worker reads — a text worker SEGFAULTED the interpreter.
            let arg = worker_row_arg(
                &state.database,
                &row_ref,
                prim_in,
                element_size,
                tuple_types_arc.as_deref().map(Vec::as_slice),
            );
            if n_hidden_text > 0 {
                let _ = state.execute_at_text(fn_pos, arg, &extras, n_hidden_text);
            } else if n_hidden_dests > 0 {
                // A real backing store per destination, never `null()`: the worker
                // claims records in it.  Nothing adopts them back — the worker's whole
                // `Stores` clone dies with the batch, which is what discard means.
                let dests: Vec<DbRef> = (0..n_hidden_dests)
                    .map(|_| state.database.database(100))
                    .collect();
                let _ = state.execute_at_ref(fn_pos, arg, &dests, &extras);
            } else {
                let _ = state.execute_at_raw_worker_arg(fn_pos, arg, &extras, return_size);
            }
        }
    });
}

// ── Plan-06 PRIORITY.md spine step 4 — Stitch::Queue runtime ────────────────
//
// Queue is the order-preserving streaming Stitch policy.  Workers run
// in parallel; their results are collected per-worker in input order
// and merged in start-index order for sequential consumption by the
// main thread's body.
//
// Used by phase 7's fused for-par when the body references the worker
// result, and by phase 10's value-position lowering (`let r =
// parallel_for(input, fn, threads); for x in r { … }`).  The key
// invariant: a result for row N appears in slot N of the merged Vec —
// `run_parallel_queue` is order-preserving, identical to today's
// `run_parallel_int` but with a configurable `return_size` and
// `extra_args` payload.
//
// This commit lands the Rust runtime + tests; codegen wiring lands
// in spine step 5 (value-position par lowering).  True streaming
// (workers running while the main thread consumes) is a follow-up
// once the API shape stabilises — for the MVP the runtime collects
// every batch before returning.

/// Plan-06 spine step 4 — `Stitch::Queue` worker runtime.
///
/// Iterates `input` rows in parallel across `n_threads`, dispatching the
/// worker fn at `fn_pos` per row and collecting each return value as a
/// `u64`.  The returned `Vec<u64>` has `len(input)` entries, with
/// element `i` holding the worker's result for input row `i` —
/// **order preserved** regardless of how the rayon pool dispatched
/// workers.
///
/// `return_size` controls how `execute_at_raw_*` reads the worker's
/// return off the stack: 1, 4, or 8 bytes.  Narrow returns are
/// zero-extended into the u64 slot.  Text and reference returns are
/// not supported here — they need the Concat path (step 8c/8d will
/// extend Queue once the encoding is settled).
///
/// `primitive_input_size` selects the input dispatch shape, mirroring
/// `run_parallel_direct`:
/// - `0`: DbRef input (worker reads slot 0 as a 12-byte `DbRef`).
/// - `u32::MAX`: text input (slot 0 is a 16-byte `Str`).
/// - `1..=8`: primitive input (worker reads slot 0 as a 1/4/8-byte
///   inline value — bool / i32 / i64 / single / character / enum-no-
///   payload).
/// - `9..=64`: wide-inline input (tuple, fn-ref, or any inline-typed
///   first-arg slot exceeding 8 bytes).  When the worker's first arg
///   is a tuple, `tuple_input_types` contains the per-element types
///   so the dispatcher can inflate text fields from a 4-byte heap
///   pointer to a 16-byte `Str` argument slot.
///
/// `extra_args` are extra context args that follow the row arg in the
/// worker's parameter list (matches `execute_at_raw`'s convention used
/// by `run_parallel_direct` / `run_parallel_int` /
/// `run_parallel_discard`).
///
/// # Panics
/// Panics if a worker thread panics.
#[cfg_attr(
    not(feature = "threading"),
    allow(clippy::needless_pass_by_value, dead_code)
)]
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn run_parallel_queue(
    stores: &Stores,
    program: WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
    extra_args: &[u64],
    return_size: u32,
    primitive_input_size: u32,
    tuple_input_types: Option<Vec<crate::data::Type>>,
) -> Vec<u64> {
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return Vec::new();
    }
    // @PLN108 R2/R3: `parallel_workers` always borrows the parent read-only (the win
    // routing's per-row-results workload needs), so this is the single path.
    let input_t = *input;
    let extras = extra_args.to_vec();
    let prog = Arc::new(program);
    let prim_in = primitive_input_size;
    // Wrap tuple types in an Arc so each worker thread shares the same
    // Vec instead of cloning it (mirrors `run_parallel_direct`).
    let tuple_types_arc = tuple_input_types.map(Arc::new);
    let batches = parallel_workers(stores, n_threads, n_rows, |start, end, ws| {
        let tuple_types_arc = tuple_types_arc.as_ref().map(Arc::clone);
        let mut state = prog.new_state(ws);
        let mut batch = Vec::with_capacity(end - start);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            // Covers every input shape the other families do, because it asks the
            // same question of the same function.
            let arg = worker_row_arg(
                &state.database,
                &row_ref,
                prim_in,
                element_size,
                tuple_types_arc.as_deref().map(Vec::as_slice),
            );
            batch.push(state.execute_at_raw_worker_arg(fn_pos, arg, &extras, return_size));
        }
        (start, batch)
    });
    merge_batches(batches, n_rows, 0u64)
}

// ── ARC.md A6.b — run_parallel_queue_fn (fn-ref returns) ────────────────────

/// Plan-06 ARC.md A6.b — fn-ref-returning par dispatch.  Each worker
/// writes its row's 20-byte fn-ref blob (8B i64 d_nr + 12B closure
/// `DbRef` per Rust's reordered layout) directly into a packed
/// `Vec<u8>` via `State::execute_at_raw_to`.  The result vector
/// holds `n_rows * 20` bytes; readers (the `n_parallel_buf_get_fn`
/// native fn) pull 20 bytes per row.
///
/// Scope: DbRef-input only (the canary `pick(s: const Score) -> fn(...)`
/// shape).  Wide-input + fn-ref-return is left on the legacy path
/// until a canary surfaces it — the dispatch ladder in
/// `run_parallel_queue` for `prim_in == u32::MAX` (text input) /
/// `> 8` (wide) / `1..=8` (primitive) only has the `execute_at_raw`
/// 8-byte-return shape; reusing them for fn-ref returns would need
/// new `_to`-returning variants of those entry points.
///
/// Bypasses the heap-result-vector + body's `get_field` indirection
/// that A6.b's L2 issue stems from: the body's `f(10)` →
/// `CallRef(b_var, [Int(10)])` reads `b_var`'s 20-byte slot directly,
/// and the parser emits `Set(b_var, Call(buf_get_fn, [idx]))` at the
/// top of each iteration to fill that slot from the packed buffer.
///
/// # Panics
/// Panics if a worker thread panics.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn run_parallel_queue_fn(
    stores: &Stores,
    program: &WorkerProgram,
    fn_pos: u32,
    input: &DbRef,
    element_size: u32,
    n_threads: usize,
    extra_args: &[u64],
) -> Vec<u8> {
    const FN_REF_SIZE: usize = 20;
    let n_rows = vector::length_vector(input, &stores.allocations) as usize;
    if n_rows == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u8; n_rows * FN_REF_SIZE];
    let buf_ptr = Arc::new(SendMutPtr(buf.as_mut_ptr()));
    let input_t = *input;
    let extras = extra_args.to_vec();
    let prog = Arc::new(program.clone());

    let _batches: Vec<()> = parallel_workers(stores, n_threads, n_rows, |start, end, ws| {
        let mut state = prog.new_state(ws);
        let buf_ptr = Arc::clone(&buf_ptr);
        for row_idx in start..end {
            let row_ref = vector::get_vector(
                &input_t,
                element_size,
                row_idx as i64,
                &state.database.allocations,
            );
            // Each row's 20-byte slot is disjoint across workers
            // (start..end ranges from `parallel_workers` are
            // non-overlapping), so concurrent writes via raw
            // pointer are safe.
            unsafe {
                let dst = buf_ptr.0.add(row_idx * FN_REF_SIZE);
                state.execute_at_raw_to(fn_pos, &row_ref, &extras, FN_REF_SIZE as u32, dst);
            }
        }
    });
    buf
}

/// Plan-06 phase 1 G3 — read the `&str` slice that the input row's
/// 4-byte text-pointer field points to.  Returns a `Str { ptr, len }`
/// borrowing into the input store's allocation; the worker's frame
/// receives this as a 16-byte slot.  Safe for the lifetime of the
/// par call because the parent stores are pinned (D2.0 read-only-
/// parent).
#[allow(dead_code)] // not threading: only the sequential branch uses it
fn read_text_at(stores: &crate::database::Stores, row_ref: &DbRef) -> crate::keys::Str {
    let store = &stores.allocations[row_ref.store_nr as usize];
    let base = store.base_ptr();
    // 4-byte u32 text-pointer at row.pos.  Unaligned read because `p`
    // comes from a `*mut u8` (row stride is 8 bytes but the pos offset
    // can land on any byte boundary inside a row).
    let text_rec = unsafe {
        let p = base.offset(row_ref.rec as isize * 8 + row_ref.pos as isize);
        p.cast::<u32>().read_unaligned()
    };
    let s = store.get_str(text_rec);
    crate::keys::Str::new(s)
}

/// Plan-06 phase 1 G2 — read a 1/4/8-byte primitive from a row
/// `DbRef`.  Returns the value zero-extended into a `u64` for
/// uniform passing to `execute_at_raw_primitive_input`.
#[allow(dead_code)] // not threading: only the sequential branch uses it
fn read_primitive_at(stores: &crate::database::Stores, row_ref: &DbRef, size: u32) -> u64 {
    let store = &stores.allocations[row_ref.store_nr as usize];
    let base = store.base_ptr();
    unsafe {
        let p = base.offset(row_ref.rec as isize * 8 + row_ref.pos as isize);
        // Unaligned reads — same rationale as `read_text_at`: row stride
        // is 8 bytes but `pos` can land on any byte boundary inside a row.
        match size {
            1 => u64::from(*p),
            4 => u64::from(p.cast::<u32>().read_unaligned()),
            _ => p.cast::<u64>().read_unaligned(),
        }
    }
}

/// Plan-06 phase 4d.A — wide-inline element reader for tuple,
/// fn-ref, and other 9..=64 byte first-arg types.  Returns a
/// stack-allocated `[u8; 64]` filled with `size` bytes copied from
/// the row record; bytes past `size` are zero.  Caller passes the
/// resulting buffer + `size` to `execute_at_raw_primitive_input_wide`.
///
/// Stays separate from `read_primitive_at` so the inline-fast-path
/// (1/4/8-byte primitives) keeps its `u64` channel — no regression
/// risk for primitive worker calls that already work today.
pub(crate) fn read_primitive_at_wide(
    stores: &crate::database::Stores,
    row_ref: &DbRef,
    size: u32,
) -> [u8; 64] {
    debug_assert!(
        size as usize <= 64,
        "wide read size {size} exceeds 64-byte cap"
    );
    let mut buf = [0u8; 64];
    let store = &stores.allocations[row_ref.store_nr as usize];
    let base = store.base_ptr();
    unsafe {
        let p = base.offset(row_ref.rec as isize * 8 + row_ref.pos as isize);
        std::ptr::copy_nonoverlapping(p, buf.as_mut_ptr(), size as usize);
    }
    buf
}

/// P189d — wide-inline reader for `vector<(T1, T2, …)>` worker inputs
/// where one or more elements need representation inflation between
/// in-vector storage and the worker's argument slot.
///
/// In-vector layout (`data::element_storage_size` — the STORAGE view):
///   - `text`: 4-byte heap-pointer (interns into the input store).
///   - `reference`: 12-byte `DbRef`.
///   - others: same as their `Context::Argument` width.
///
/// Worker-slot layout (`variables::size(_, Context::Argument)`):
///   - `text`: 16-byte `Str` (8B ptr + 8B len).
///   - others: same as in-vector for primitives.
///
/// For each element this function copies `element_size` bytes from
/// `row_ref + in_vec_offset` to `buf + arg_offset`, except for `Text`
/// where the 4-byte pointer is inflated to a `Str` via
/// `store.get_str(...)` — same path as `read_text_at`.
///
/// `elem_types` is the list of tuple element types (from the synthetic
/// `__tuple<…>` struct's attributes).  Bytes outside the populated
/// regions stay zero.
pub(crate) fn read_tuple_at_wide(
    stores: &crate::database::Stores,
    row_ref: &DbRef,
    elem_types: &[crate::data::Type],
) -> [u8; 64] {
    let mut buf = [0u8; 64];
    let store = &stores.allocations[row_ref.store_nr as usize];
    let base = store.base_ptr();
    // @PLN114 — the row is STORAGE, so its offsets and widths are the storage view.
    // This read used the STACK view, which was indistinguishable while the two
    // coincided (every integer 8 bytes); once narrow elements got their declared
    // width (`u8` = 1) the reader walked the row at the wrong stride and returned
    // garbage — a `par` over `vector<(u8,u16)>` summed to 1012184093813119760.
    let in_vec_offsets = crate::data::element_storage_offsets(elem_types);
    let mut arg_offset: usize = 0;
    for (i, t) in elem_types.iter().enumerate() {
        let in_off = in_vec_offsets[i];
        let in_sz = crate::data::element_storage_size(t);
        let arg_sz = crate::variables::size(t, &crate::data::Context::Argument) as usize;
        debug_assert!(
            arg_offset + arg_sz <= 64,
            "read_tuple_at_wide: tuple slot exceeds 64-byte cap"
        );
        unsafe {
            let src =
                base.offset(row_ref.rec as isize * 8 + (row_ref.pos as usize + in_off) as isize);
            if matches!(t, crate::data::Type::Text(_)) {
                // Inflate the 4-byte heap text-pointer into a 16-byte
                // Str for the worker's argument slot.
                let text_rec = src.cast::<u32>().read_unaligned();
                let s = store.get_str(text_rec);
                let str_val = crate::keys::Str::new(s);
                std::ptr::copy_nonoverlapping(
                    (&raw const str_val).cast::<u8>(),
                    buf.as_mut_ptr().add(arg_offset),
                    arg_sz,
                );
            } else if matches!(t, crate::data::Type::Function(_, _, _)) {
                // ARC.md A6.c — fn-ref vector elements are 4-byte
                // d_nr in storage (`element_size(Type::Function) = 4`)
                // but the worker's argument slot is 20 bytes
                // (`variables::size(.., Argument) = 8B i64 d_nr + 12B
                // closure DbRef`).  A plain memcpy of 4 bytes leaves
                // the remaining 16 bytes zero; the worker's OpCallRef
                // would then dereference closure `(0, 0, 0)` (a real
                // DbRef pointing into store 0) and SIGSEGV.
                //
                // vector<fn> can only store non-capturing functions
                // (capturing-lambda storage is part of the open 4d.C
                // closure-storage redesign), so the correct closure
                // representation here is the u16::MAX sentinel.
                let d_nr = src.cast::<u32>().read_unaligned();
                let d_nr_i64: i64 = i64::from(d_nr);
                std::ptr::copy_nonoverlapping(
                    (&raw const d_nr_i64).cast::<u8>(),
                    buf.as_mut_ptr().add(arg_offset),
                    8,
                );
                let sentinel = DbRef::NULL;
                std::ptr::copy_nonoverlapping(
                    (&raw const sentinel).cast::<u8>(),
                    buf.as_mut_ptr().add(arg_offset + 8),
                    12,
                );
            } else {
                // Plain memcpy — in-vector and worker-slot widths
                // match for primitives, references, and fn-refs.
                let copy_n = in_sz.min(arg_sz);
                std::ptr::copy_nonoverlapping(src, buf.as_mut_ptr().add(arg_offset), copy_n);
            }
        }
        arg_offset += arg_sz;
    }
    buf
}

/// Read one input row the way the worker's FIRST PARAMETER reads it — the single
/// place that decision is made.
///
/// A worker's slot 0 is filled from the row, and how depends only on the row's shape:
/// a text element arrives as a 16-byte `Str`, a 1/4/8-byte primitive as its value, a
/// wide (9..=64 byte) element such as a tuple as its bytes INLINE, and anything else
/// as the element's `DbRef`.  Nothing about the worker's RETURN type enters into it.
///
/// It lives here because it used to live in four places.  Each dispatch family wrote
/// its own ladder and each stopped at a different rung: `run_parallel_text` had no wide
/// arm at all (its comment claimed a wide element was "not constructible as a vector
/// literal today" — a five-row `vector<(integer, integer)>` literal falsifies it),
/// `run_parallel_queue_ref` had neither a wide nor a text arm, and `run_parallel_discard`
/// had a wide arm it could only take when the worker had no hidden parameters.  Every
/// gap ended at the same wrong answer, `WorkerArg::Ref`, so the worker read the row's
/// pointer bits as its value: wrong numbers for `vector<(integer, integer)>`, a worker
/// stack underflow for a 3-tuple, and a SIGSEGV for `vector<text>` into a
/// struct-returning worker (loft#1055).
///
/// `tuple_types` is `Some` for a tuple element, whose in-vector STORAGE layout differs
/// from its argument-slot layout (see [`read_tuple_at_wide`]); `None` reads the row's
/// bytes straight.
pub(crate) fn worker_row_arg(
    stores: &crate::database::Stores,
    row_ref: &DbRef,
    primitive_input_size: u32,
    element_size: u32,
    tuple_types: Option<&[crate::data::Type]>,
) -> crate::state::WorkerArg {
    // `u32::MAX` is the TEXT marker, not a width, so it has to be tested before any
    // size comparison.
    if primitive_input_size == u32::MAX {
        crate::state::WorkerArg::Text(read_text_at(stores, row_ref))
    } else if primitive_input_size > 8 {
        let buf = match tuple_types {
            Some(types) => read_tuple_at_wide(stores, row_ref, types),
            None => read_primitive_at_wide(stores, row_ref, element_size),
        };
        crate::state::WorkerArg::Wide {
            buf,
            size: primitive_input_size,
        }
    } else if primitive_input_size > 0 {
        crate::state::WorkerArg::Primitive {
            value: read_primitive_at(stores, row_ref, element_size),
            size: primitive_input_size,
        }
    } else {
        crate::state::WorkerArg::Ref(*row_ref)
    }
}

/// A worker's fatal halt, carried to the parent so the WHOLE program stops.
///
/// `assert` and `panic` are not exceptions and nothing here propagates one: a failing
/// assert sets `runtime_error` on the `Stores` it runs against, and the dispatch loop
/// halts when it sees that.  A worker runs against a CLONE of `Stores`, so its halt was
/// set on a copy that is dropped at join — the arm stopped and the program carried on,
/// silently, on both backends (loft#1053).
///
/// The contract is that a failed assertion stops the whole program, not one arm of it, so
/// the worker's already-built halt is handed back and the parent raises it through the
/// SAME path a main-thread assert takes.  No new error channel, and nothing a loft program
/// can observe or catch.
static WORKER_FATAL: std::sync::Mutex<Option<Box<crate::runtime_error::RuntimeError>>> =
    std::sync::Mutex::new(None);

/// Cheap gate for the dispatch loop, which asks after EVERY op.  A relaxed load costs
/// nothing measurable; taking the mutex there would not be free, and in the overwhelming
/// case there is nothing to take.
pub static WORKER_FATAL_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Whether a worker halted and the parent has not yet raised it.
#[must_use]
pub fn worker_fatal_pending() -> bool {
    WORKER_FATAL_PENDING.load(std::sync::atomic::Ordering::Relaxed)
}

/// A worker's `State`, which records its halt when it goes out of scope.
///
/// Deref, not inheritance: every existing `state.execute_*(…)` call site keeps working
/// unchanged, and the only added behaviour is at drop, where the worker is finished and
/// its `runtime_error` is either set or not.
pub(crate) struct WorkerState(Option<State>);

impl std::ops::Deref for WorkerState {
    type Target = State;
    fn deref(&self) -> &State {
        self.0
            .as_ref()
            .expect("worker state is present until taken")
    }
}
impl std::ops::DerefMut for WorkerState {
    fn deref_mut(&mut self) -> &mut State {
        self.0
            .as_mut()
            .expect("worker state is present until taken")
    }
}
impl Drop for WorkerState {
    fn drop(&mut self) {
        if let Some(s) = &self.0 {
            record_worker_fatal(&s.database);
        }
    }
}

impl WorkerState {
    /// Hand the worker's stores back to the parent, recording any halt FIRST.
    ///
    /// Two `run_parallel_*` variants return `state.database` so the parent can adopt the
    /// worker's stores.  That moves the field out from under the drop guard, which the
    /// compiler refuses — correctly, since it is exactly the path where a halt would go
    /// unrecorded.  Taking it explicitly keeps the recording and the hand-off in one place
    /// instead of leaving the guard silently bypassed.
    fn into_database(mut self) -> Stores {
        let s = self.0.take().expect("worker state taken once");
        record_worker_fatal(&s.database);
        s.database
    }
}

/// Record a worker's halt if it ended with one.  First writer wins: the message a user
/// reads must not depend on thread scheduling, and one arm's assertion is enough to stop
/// the program.
pub fn record_worker_fatal(db: &Stores) {
    if let Some(err) = db.runtime_error.clone()
        && let Ok(mut slot) = WORKER_FATAL.lock()
        && slot.is_none()
    {
        *slot = Some(err);
        WORKER_FATAL_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Forget any recorded halt, so a new run cannot inherit one.
///
/// `record_worker_fatal` is first-writer-wins and the dispatch loop is the only consumer,
/// so a halt recorded on a path that never reaches that loop — a worker whose stack
/// underflowed PANICS, and the panic unwinds past it — stays in the slot.  The next run
/// then raises someone else's fault as its own, which is how a text-element test came to
/// fail with a tuple worker's message.  Cleared at the entry point of every run rather
/// than at the panic, because "a run starts with no inherited halt" is the property, and
/// it holds however the previous one ended.
pub fn clear_worker_fatal() {
    WORKER_FATAL_PENDING.store(false, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut slot) = WORKER_FATAL.lock() {
        *slot = None;
    }
}

/// Take the recorded halt, if any, for the parent to raise as its own.
pub fn take_worker_fatal() -> Option<Box<crate::runtime_error::RuntimeError>> {
    if !worker_fatal_pending() {
        return None;
    }
    WORKER_FATAL_PENDING.store(false, std::sync::atomic::Ordering::Relaxed);
    WORKER_FATAL.lock().ok().and_then(|mut s| s.take())
}

/// Run N independent arms concurrently, each at a given bytecode position.
#[allow(dead_code)]
/// Each arm runs as a void function (no args, no return value).
/// Uses the same store-isolation model as `par()`: each arm gets a read-only snapshot.
/// # Panics
/// Panics if any arm thread panics.
// See `run_parallel_direct` for the threading-vs-non-threading split rationale.
// (dead_code already allowed above — only needless_pass_by_value is feature-gated.)
#[cfg_attr(
    any(not(feature = "threading"), feature = "wasm"),
    allow(clippy::needless_pass_by_value)
)]
pub fn run_parallel_block(
    stores: &Stores,
    program: WorkerProgram,
    arm_positions: &[u32],
    parent_snapshot: &Arc<Vec<u8>>,
) {
    if arm_positions.is_empty() {
        return;
    }
    #[cfg(all(feature = "threading", not(feature = "wasm")))]
    {
        let program = Arc::new(program);
        thread::scope(|s| {
            for &pos in arm_positions {
                let worker_stores = unsafe { stores.clone_for_light_worker() };
                let prog = Arc::clone(&program);
                let snapshot = Arc::clone(parent_snapshot);
                s.spawn(move || {
                    let mut state = prog.new_state(worker_stores);
                    state.execute_at_void_with_snapshot(pos, &snapshot);
                });
            }
        });
    }
    #[cfg(any(not(feature = "threading"), feature = "wasm"))]
    {
        // Sequential fallback (WASM or threading disabled).
        for &pos in arm_positions {
            let mut state = program.new_state(unsafe { stores.clone_for_light_worker() });
            state.execute_at_void_with_snapshot(pos, parent_snapshot.as_ref());
        }
    }
}
