<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 227 — `par` on Web Workers in a page the C backend builds

## Status

**Finished 2026-11-25**; it reaches `main` through loft#2634, and
[loft-lang/plans#227](https://github.com/loft-lang/plans/issues/227) is closed.  The design is
[THREADING.md](../THREADING.md) § the page the C backend builds and decision
[C3](../DESIGN_DECISIONS_PLATFORM.md#c3--wasm-par-runs-sequentially); this file is the record.

| Phase | State |
|---|---|
| P0 | done: the three facts below, each measured |
| P1 | done: `platform::wasm_threads_sysroot`, `make wasi-threads-sysroot` |
| P2 | done: `loft_pool_start`, `loft_pool_run`, the dispatch in `loft_par_run` (`src/c/loft_keyed.h`) |
| P3 | done: `loftCPool` (`doc/loft-thread.js`); copies before decodes (`loft-wasi.js`, `loft-deliver.js`, `loft-web.js`) |
| P4 | done: `c_driver::WasmHost::ThreadedPage`, chosen in `--html` when the unit calls `loft_par_run` |
| P5 | done: `html-thread-proof.sh` passed all four claims on CI (loft#2634's second run: 4 workers, fallback and `--no-threads` correct, the matrix as the interpreter); the `browser-threads` job links the page with wasi-sdk's own clang (`make wasi-sdk`) |

## Goal

A page the C backend builds (`loft --html`) runs `par` on Web Workers, as decision
[C3](../DESIGN_DECISIONS_PLATFORM.md#c3--wasm-par-runs-sequentially) says a page that uses `par`
does.  Without it, a C page runs every `par` row on the main thread: the right answer
(`@FR-C-Det`), at single-thread speed, so a developer reads the wrong speed off their page.

Done when `tests/wasm/html-thread-proof.sh` passes on the C page:

1. on a cross-origin-isolated host the page dispatches `par` across at least 2 workers;
2. on a host without isolation the same page runs on 1 worker;
3. a `--no-threads` page runs sequentially;
4. every value equals the interpreter's.

## P0 — what was measured

1. **The page's main thread cannot wait.**  A module on shared memory whose exported function
   runs `memory.atomic.wait32`, called from the page's main thread in headless Chromium on a
   cross-origin-isolated host: `RuntimeError: Atomics.wait cannot be called in this context`.  So
   on the main thread a `pthread_join` traps, and so does a contended lock in the libc
   (`malloc`'s, `stdout`'s), which waits once its short spin runs out.  This shapes the design.
2. **The toolchain exists.**  clang 22 and wasi-sdk-34's `wasm32-wasip1-threads` sysroot link
   a pthread program (`-pthread`, imported shared memory).  wasmtime runs it with 4 threads
   (`-W threads,shared-memory -S threads`).  Debian's and Ubuntu's `wasi-libc` packages carry
   no threads variant; wasi-sdk's sysroot archive does.
3. **The runtime has the rest.**  The C runtime already runs `par` on POSIX and Windows threads,
   each worker on a store context of its own (`@FR-C-Own`, @PLN226).  It already runs a slice on
   the calling thread when a thread cannot start.  What a page lacks is the threads build, a host
   for the libc's thread spawn, and a main thread that never waits.

## The design

- **A pool, started before `main`.**  A Worker that a page creates begins running only after the
  main thread returns to the event loop, and a `par` cannot return there in the middle.  So the
  workers are started once, before the program runs.  The page calls the module's
  `loft_pool_start(n)` (`n` = `navigator.hardwareConcurrency`, at most `LOFT_POOL_MAX`).  That
  function `pthread_create`s `n` pool threads, and the page waits until each worker reports that it
  is up.  A pool thread waits for a job (a worker may wait) and never ends, so its exit never
  takes a libc lock while the main thread runs.
- **The main thread never waits** (P0.1).  On the main thread, a `par` of `t` slices splits
  its rows into `min(t, pool)` slices.  It hands every slice to a pool worker, the first one
  included, and spins on each worker's `done` word.  While it spins it calls the page's
  `loft_page.pool_idle`, which shows the workers' output.  It runs no slice itself, so it never
  contends for a lock with a worker.  The results are adopted as on a native host (`@FR-C-Adopt`).
- **Elsewhere nothing changes.**  A `par` on a worker (a nested one) and a `parallel { }` block
  on a page start threads through `pthread_create`.  The page refuses that spawn, so the
  existing fallback runs each slice or arm on the calling thread, in order: the same answer.  A
  page that is not cross-origin isolated starts no pool (`loft_pool_start` is not called), and
  every `par` runs on the main thread, as it does today.
- **A worker's host.**  A worker instantiates the same module on the same memory and calls its
  `wasi_thread_start`.  Its WASI answers the clocks, randomness and `sched_yield`; its standard
  output and error go to a shared queue, which the main thread drains before each write of its
  own, while it spins, and when the program ends.  So a `par` body's lines come before anything the
  program prints after the `par`, as on a native host.  Files and the page's other host calls
  answer "not supported" on a worker and say so once in the console.
- **Shared memory.**  The memory is imported, shared, at most 1 GiB, as the rustc writer's
  threaded page had it.  A view onto a `SharedArrayBuffer` is refused by `TextDecoder.decode`
  and `crypto.getRandomValues`, so the page's readers copy first (`loftTextDecoder`).

## P1 — the threads toolchain

- `loft::platform::wasm_threads_sysroot()`: the first sysroot that holds
  `lib/wasm32-wasip1-threads/libc.a`: `LOFT_WASI_THREADS_SYSROOT`, then the one the WASI build
  already uses, then `$WASI_SDK_PATH/share/wasi-sysroot`, `/opt/wasi-sdk/share/wasi-sysroot`,
  and `~/.loft/toolchains/wasi-sysroot`.
- A probe: a small pthread program links under `wasm32-wasip1-threads` with that sysroot.
- `make wasi-threads-sysroot` installs wasi-sdk's sysroot under `~/.loft/toolchains/`.
- No threads sysroot: the page builds unthreaded and says so in one line, naming the cure.  This
  matches C3's "where that runtime is not available".

## P2 — the C runtime's page pool

- `LOFT_PAGE_POOL` (defined for a threaded page build) turns `LOFT_THREADS` on for
  `wasm32-wasip1-threads`.
- `loft_pool_start` (exported), the pool thread's loop, and the main thread's dispatch and spin in
  `loft_par_run`, in `src/c/loft_keyed.h` beside the native thread code.
- `loft_set_par_trace` (exported): `?loftTrace=1` makes each `par` print
  `distinct_workers=N`, the number of pool workers its slices ran on.

## P3 — the page script

- `doc/loft-thread.js` gains the C half: the `wasi.thread-spawn` import (allowed only while
  `loft_pool_start` runs), the worker source that instantiates the module and calls
  `wasi_thread_start`, and the output queue.
- `doc/loft-wasi.js` decodes and fills through copies on shared memory, and drains the queue.

## P4 — the page build

- `--html` builds the C unit threaded when it calls `loft_par_run` (`--threads` / `--no-threads`
  override): `--target=wasm32-wasip1-threads -pthread`, `-Wl,--import-memory,--shared-memory,
  --max-memory=1073741824`, `-DLOFT_PAGE_POOL`.
- asyncify runs with `--enable-threads`; only the main thread unwinds.

## P5 — the gates and CI

- `tests/wasm/html-thread-proof.sh` builds the C page (it no longer needs nightly) and keeps its
  four claims.
- The `browser-threads` workflow installs the threads sysroot (`make wasi-threads-sysroot`, cached).

## Documentation

- [x] Decision C3 (`DESIGN_DECISIONS_PLATFORM.md`), its record in `DESIGN_DECISIONS-history.md`.
- [x] `THREADING.md` § the page the C backend builds; `WASM.md` and `HTML_EXPORT.md` § threads.
- [x] `CHANGELOG.md`: a page runs `par` on Web Workers.

## Not in this plan

- `--native-wasm` threaded: a WASI host needs flags to run a threaded module
  (`wasmtime -W threads,shared-memory -S threads`), so a threaded default would stop a plain
  `wasmtime prog.wasm` from running it.
- `parallel { }` arms on pool workers, and a nested `par` spread over the pool.
- Files from a `par` body on a pool worker: the page filesystem lives in the main thread's script.
