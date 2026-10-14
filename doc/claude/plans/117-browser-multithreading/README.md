<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 117 — Browser multi-threading (real Web Worker threads for `par` / `par_fold`)

Tracks [`loft-lang/plans#117`](https://github.com/loft-lang/plans/issues/117) (`@PLN117`).

## Status

**Open — INTEGRAL threading done and gated in CI: `par` runs on Web Worker threads in EVERY
browser build (`loft --html` and the gallery), on ONE loft-owned runtime.** A
loft `par`, run through `compile_and_run`
in a real (headless) browser, **dispatches across multiple Web Worker threads over shared wasm
memory** with correct results (proven for primitive, struct+text, and vector returns) and a
never-break sequential fallback. The load-bearing risk (does rayon actually build, link, and
parallelise loft's dispatch in a browser?) is **resolved**. What is proven, landed on
`tuxedo-extract-function`, and pushed:

- **Build + link + pool** (`src/wasm_threads.rs`, `doc/loft-thread.js`; `make wasm-mt`).
- **Native dispatch seam** (`src/parallel.rs::with_pool`) — wasm uses the global rayon pool,
  native its private pool; behaviourally identical on native (threading suite 47/47).
- **In-browser dispatch:** `distinct_workers=4`, `par_sum=36023`, matching the native
  `--interpret` **and** `--native` reference (`36023`). ~1.8× wall-clock here (interpreter
  overhead dominates 48 elems / 4 workers).
- **Arc-D fallback:** with no pool started, the same `par` runs sequentially
  (`distinct_workers=1`), succeeds, and gives the correct value — **it never breaks**.
- **Arc-C memory model:** the read-only-share model holds under shared linear memory —
  allocation-heavy struct+text AND vector-return `par` match the native reference on every
  rep across 4 workers (30-rep race hunt clean); the wasm allocator lock-guards DLMALLOC.
- **Scaling (E2):** a CPU-heavy `par` speeds up near-linearly with the pool — par-time 3154ms (1
  worker) → 612ms (8 workers, 5.2×; 2.9× at 4), value stable, ~2× the native interpreter.
- **UI responsiveness (E1):** a `requestAnimationFrame` loop running one heavy `par` per frame
  delivers ~3× more frames (and ~½ the worst-case jank) with the pool than sequential — the win is
  throughput (each frame's par finishes faster; the main thread still blocks on the rayon join).
- **Headless gates:** `tests/wasm/par-thread-proof.sh` (dispatch + fallback), `par-memory-proof.sh`
  (memory model, both return shapes), `par-scaling-bench.sh` (scaling), `par-ui-responsive.sh`
  (UI); `coi-server.py` is the COOP/COEP host.

**The toolchain finding (load-bearing).** A wasm only threads if its memory is SHARED + IMPORTED
with a maximum and lld's synthesized TLS / heap-base globals survive as exports. This toolchain's rustc does **not** auto-emit those from `+atomics`, so `make wasm-mt`
passes the full link-arg set explicitly (`--shared-memory --max-memory --import-memory
--export=__heap_base --export=__wasm_init_tls/__tls_size/__tls_align/__tls_base`, plus loft's own
`--export=__stack_pointer`). Drop any one and the bundle silently builds a **non-shared** memory →
workers die at runtime with *"Memory could not be cloned"*. `--target web` is mandatory (the old
`wasm-mt` target used `--target nodejs`, which cannot drive the worker bootstrap; **node also has
no Web Worker global**, so the browser — not node — is the proof environment).

**B1 — `loft --html` threaded export: DONE.** A self-contained single-file page (wasm inlined as
base64 in a non-module `<script>`, no wasm-bindgen) dispatches `par` across Web Workers.
`loft --html` picks the threaded runtime when the program has a reachable `par`; `--threads` /
`--no-threads` override. It links an atomics std through a **sysroot assembled from the
`build-std` output** so the `rustc`-direct pipeline, its single loft copy, and the wasm-bridge
crates are unchanged (design decision D1=b — the option the pre-implementation design had ranked
second). Gate `tests/wasm/html-thread-proof.sh`: threaded `distinct_workers=8`, plain host
`distinct_workers=1`, `--no-threads`, all `par_sum=36023` = the interpreter.

**One runtime everywhere — `wasm-bindgen-rayon` is gone.** loft owns its `par` runtime end to end
(`src/wasm_threads.rs` + `doc/loft-thread.js` + `wasm-sync/`): rayon still schedules, loft supplies
the threads, because a browser has no `thread::spawn`. The pool is plain wasm exports with **no
host imports at all** — the host sequences start-up anyway — which is what lets the same runtime
serve the raw `--html` bundle and the wasm-bindgen gallery bundle, whose glue builds its own
imports and would reject an extra module. The gallery was migrated onto it and all four gates
re-pass, scaling included (8w = 5.3×, slightly better than under wasm-bindgen-rayon).

**Three things the POC falsified before any pipeline surgery** (see
[`DESIGN-integral-threads.md`](DESIGN-integral-threads.md) § Phase 0 result): `rayon/web_spin_lock`
is mandatory and its `wasm_sync` had to become loft's own (the browser main thread may not block,
and rayon locks there on every join); pool start-up must be two-phase and host-driven (one-shot
deadlocks); and each worker needs its own shadow stack + TLS block installed from JS, plus an
`--export=__stack_pointer` link-arg.

**CI wiring — DONE.** The five gates run on a runner, not just this box:
`.github/workflows/browser-threads.yml` (`Browser threading`) installs what they need —
nightly + `rust-src`, `wasm-pack`, binaryen, headless Chrome — builds the release loft and
`make wasm-mt`, then runs `scripts/par_gates.sh --ci`. It fires **nightly at 05:00 UTC**
(clear of the 03:00 daily matrix and 04:00 miri) and **per PR on the threading surface**
(`src/parallel.rs`, `src/wasm_threads.rs`, `src/wasm.rs`, `src/native_utils.rs`, `wasm-sync/`,
`doc/loft-thread.js`, `tests/wasm/`, `Makefile`, `Cargo.toml`); the nightly is what covers the
broad files (`src/state/mod.rs`, `src/generation/`, `src/main.rs`) that are too hot to put on a
per-PR trigger for a ~40-minute job. Two decisions make it a real gate rather than a green
no-op:

- **A SKIP is a failure in CI.** Each gate exits 0 on a missing prerequisite so a dev without a
  browser still gets a useful run — on the runner everything is installed, so "skipped" can only
  mean the provisioning broke. `--ci` preflights the five prerequisites and fails on any `SKIP`.
- **Measurement is calibrated to the runner, not switched off.** 4 vCPUs against this box's 24,
  so the workflow sets `POOLS="1 2 4"`, `MIN_SPEEDUP_PCT=150`, `THREADS=4`,
  `MIN_FRAME_RATIO_X10=15` (defaults, i.e. this box: `1 2 4 8` / 250 / 8 / 20). A `par` that stops
  parallelising still fails, and **every value assertion is untouched**. `WAIT_SCALE=3` stretches
  the wait for each page's report line; the gates now exit as soon as it lands (they used to sleep
  a fixed 15–34s per cell), so the cap costs nothing when things are healthy.

The **first CI run found two more** — both sampling artefacts of a small, contended machine, each
reproduced locally under `taskset -c 0-3` before being touched (all values were correct on the
runner throughout):

- `par-memory-proof` asserted `min_workers >= 2` — EVERY one of 12 reps concurrent. On 4 vCPUs
  rayon handed one rep its whole (small) workload on a single worker. The page now reports both
  ends of the spread and `CONCURRENCY_STAT` picks which is asserted: `min` (this box, strict) vs
  `max` (CI — concurrency demonstrated across the reps). The per-rep value check, which is what
  actually catches a race, is unchanged.
- `par-ui-responsive` got 4 frames against 3 — too small to be a ratio at all. `UI_WORK=15000` /
  `UI_WINDOW=9000` sample lighter work over a longer window: 77 frames vs 35 (2.2×) on 4 CPUs.
  Separate names from the bench's `WORK` so retuning one cannot silently retune the other.

**Green on the runner** (run 30042663620): all five gates pass — dispatch and `--html` across 4
workers, scaling 100/159/224% over 1/2/4, UI 33 frames vs 19 sequential, every value equal to the
interpreter's. The frame floor is set at 1.3× rather than just under that 1.7×: this runs nightly,
and a gate that reddens on runner noise gets ignored, which costs more than the tenths it buys.

A red run surfaces on every PR through ci.yml's `Nightly health (informational)` job, which now
reports `browser-threads.yml` beside miri and the daily matrix.

Steps 0–4, arcs A/B/C/D, B1 (gallery **and** `--html`), and all of Track 4 (E1/E2/E3) are done.
This README is the single source of truth for phase status.

The **default** (non-threaded) `--target web` gallery bundle is still built `--features wasm`
without `threading`, so `par` there takes `parallel.rs`'s sequential fallback — a threaded gallery
is `make gallery-mt`, deployed on a COOP/COEP host. A `loft --html` page needs no such choice: it
picks the threaded runtime from the program itself.

## Goal

Browser **games** — loft's core use case — and apps like **routing** need genuine
multi-threading. Make `par(...)` and `par_fold` dispatch across Web Worker threads in the
browser (off the main thread, UI stays responsive, scales with core count), with a clean
fall-back to today's sequential behaviour when the host can't provide cross-origin isolation.

## Effort + design

- **Effort:** L — runtime + build wiring + JS glue + a real deployment constraint (COOP/COEP).
- **Value:** G (goal-enabling: browser games / multiplayer / routing all depend on it).
- **Design:** ready — the step-by-step plan (small gated steps + exact code points) is in
  [`DESIGN.md`](DESIGN.md); this is wiring + re-proving the memory model under shared memory,
  not new invention.

## Why it exists

`par` is loft's data-parallel primitive; on `--native` it runs OS threads via rayon
(`std::thread::spawn`, `thread::scope`). In the browser it currently does **nothing parallel** —
`src/parallel.rs` (lines 7–9): *"When `threading` is disabled (e.g. under WASM), the loop body
runs sequentially in the caller's thread — same results, no parallelism."* For a routing solver
or a game's per-entity update, that means the whole workload runs on the main thread and freezes
the page while it does. The infrastructure to fix it already exists but is unwired:

- `Cargo.toml`: a `wasm-threads` feature exists (at plan-open it pulled `wasm-bindgen-rayon`).
- `Makefile:951`: a **nodejs** multi-thread bundle (`tests/wasm/pkg-mt/`) is built with it.
- `src/wasm.rs::worker_entry`: a **no-op stub** (W1.18-2 TODO) — an older hand-rolled-worker
  design; the intended model is the rayon/`wasm-bindgen-rayon` scheduler (`parallel.rs:140`,
  *"so the two scheduler stories converge"*).

So the browser (`--target web`) build just never opts in.

## The two threading models this must cover

1. **`par(...)` for-loop** — `run_parallel_*` in `src/state/mod.rs` / `src/parallel.rs`
   (`.into_par_iter()` under `#[cfg(feature = "threading")]`), including the @PLN108
   read-only-store-share optimisation.
2. **`par_fold` / `parallel_fold`** — the parallel-fold builtin (`parse_par_fold`,
   `parallel::run_parallel_fold`).

Both are rayon-backed, so both ride the same wasm-pool wiring. **Coroutines** (COROUTINE.md) are
cooperative single-thread state machines — they are *not* threaded and stay as-is, but must keep
working correctly on the main thread alongside the worker pool.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A** — threaded browser bundles | build with threading; the page starts the pool (`await init(); startLoftWorkers(...)`) before any `par` | **DONE for BOTH.** Gallery: `make wasm-mt` / `gallery-mt` (full link-arg set, → `doc/pkg-mt`, no clobber of the committed single-threaded gallery); `playground.html`/`gallery-run.html` start loft's pool when `crossOriginIsolated` (inert on the single-threaded bundle — proven both ways); `make serve` sends COOP/COEP. **`loft --html`** chooses the threaded runtime from the program itself and inlines the bootstrap, staying one self-contained file |
| **B** — dispatch over the browser rayon pool | route `run_parallel_*` and `par_fold` over the page's global pool; ONE scheduler, not two | **DONE + proven** (`with_pool` seam; in-browser `distinct_workers=4`, value matches native). The hand-rolled `worker_entry` stub is gone — and so is `wasm-bindgen-rayon`: the pool is loft's own (`src/wasm_threads.rs`), shared by every browser build |
| **C** — memory model under SHARED memory | re-prove the @PLN108 read-only-share model (`clone_for_light_worker`, borrowed parent stores, `read_only` write-panic) now that the Store heap is a *shared* `SharedArrayBuffer` across workers, under atomics | **DONE + proven.** Invariant (worker reads shared read-only parent, writes only own scratch) holds across 5 verified re-assertion sites — @C93 static, the release-active read-only `assert!`, the read-only borrow, join-before-drop, and the **lock-guarded wasm DLMALLOC** (concurrent worker alloc is serialized). Empirical: `par-memory-proof.{html,sh}` runs allocation-heavy struct+text AND vector-return `par` across 4 Web Workers, every rep == native ref, real ≥4-worker concurrency, 30-rep race hunt clean. `par_share_for`/copy-path is gone (@PLN108 left ONE borrow path) |
| **D** — cross-origin isolation + fallback | COOP/COEP hosting contract; runtime `crossOriginIsolated` detection; **graceful fallback to sequential** when unavailable | **DONE + proven** — `coi-server.py` sends COOP/COEP, `html-plain-server.py` deliberately does not; on a non-isolated host the SAME threaded `--html` bundle runs `distinct_workers=1` with the same value and never crashes. Contract documented in WASM.md |
| **E** — verify + measure | in-browser proof a `par` runs OFF the main thread + scales; an automated gate; benchmark vs native + vs sequential-wasm | **E1 + E2 + E3 DONE.** Five headless gates: `par-thread-proof.sh` (dispatch + fallback), `par-memory-proof.sh` (memory model), `par-scaling-bench.sh` (scaling — par-time 1w→8w = 3154→612ms, 5.2×; 2.9× at 4w; ~2× the native interpreter), `par-ui-responsive.sh` (E1 — a rAF loop delivers ~3× more frames + ~½ the worst-case jank threaded vs sequential), and `html-thread-proof.sh` (the `loft --html` bundle: threaded, non-isolated fallback, and `--no-threads`, each against the interpreter's value). All value-stable, all run in CI by `.github/workflows/browser-threads.yml` (nightly + threading-surface PRs), where a skipped gate is a failure |

## Phase ordering

1. **A** first — a `--target web` bundle that builds with `wasm-threads` and whose JS glue
   spins up a worker pool is the prerequisite everything else is verified against. Prove the
   pool initialises (`crossOriginIsolated === true`, N workers) before touching dispatch.
2. **C alongside B** — the memory model must be re-proven *as* dispatch is wired, not after:
   shared linear memory changes the copy-vs-borrow assumptions, and a data race here is silent
   corruption (an **S**-category hazard). Build the falsification instrument (a positive-control
   race that MUST fire) first, like @PLN108 did on native.
3. **B** — wire the pool once A+C hold; the `.into_par_iter()` paths should light up under the
   pool with little change, but `clone_for_worker`'s freed-slot reinit and the locked-store
   asserts need checking under atomics.
4. **D** runs throughout (the fallback path must exist from A so nothing ever hard-breaks) and
   is the deployment gate for real hosts.
5. **E** graduates as each arc lands; the pkg-mt harness is the CI-able proxy for the browser.

## Open questions

1. **rayon-via-wasm-bindgen-rayon vs the hand-rolled `worker_entry`.** ANSWERED: neither.
   rayon does drive `.into_par_iter()` in the browser, so `worker_entry` is deleted — but the
   scheduler needed no wasm-bindgen at all. loft supplies the threads itself
   (`src/wasm_threads.rs`), which is what lets ONE pool serve the `--html` bundle as well as the
   gallery, and `wasm-bindgen-rayon` is dropped.
2. **@PLN108 borrow model under shared memory.** On native, the read-only-share borrows parent
   stores because the dispatcher joins all workers before the parent drops. Under a shared
   `SharedArrayBuffer`, is the same lifetime argument sound, or does shared memory + atomics
   force back to the copy path (and if so, is the copy even affordable given wasm memory
   limits)? This is the load-bearing correctness question.
3. **Thread count + pool lifecycle.** One pool per page (init once) vs per-`par`; how
   `navigator.hardwareConcurrency` maps to the `threads` argument of `par(b=f(a), N)`; what
   happens when a game wants a persistent worker set across frames.
4. **Fallback UX.** When `crossOriginIsolated` is false, run sequential — but should it *warn*
   (a page that expected parallelism silently runs 1×), and where (console vs a loft-level hook)?

## Cross-arc dependencies

- **[@PLN108 read-only store sharing](../108-share-read-only-stores/README.md)** — the memory
  model arc C must re-prove under shared wasm memory; its native ASan/TSan gate is the template
  for the wasm race-detection gate.
- **THREADING.md** — the `par` / `par_fold` dispatch contract (worker rules, `clone_for_worker`,
  `run_parallel_*`); reference home, updated as arcs land.
- **WASM.md / BROWSER_INTEROP.md** — the browser build + host-interaction surface; the COOP/COEP
  contract (arc D) documents here.
- **`tests/wasm/` harness** — `pkg-mt` build (Makefile) + `suite.mjs --threaded`; the automated
  proxy for the in-browser gate.
- **Coroutines (COROUTINE.md)** — not threaded, but must keep working on the main thread; a
  regression check, not an arc.

## See also

- `src/parallel.rs` (dispatch + the threading/non-threading split), `src/state/mod.rs`
  (`run_parallel_*`, `clone_for_worker`), `src/wasm.rs` (`worker_entry` stub).
- `Cargo.toml` (`wasm-threads` feature), `Makefile` (`--target web` gallery build + `pkg-mt`).
- [`loft-lang/plans#117`](https://github.com/loft-lang/plans/issues/117) — the tracking issue.
