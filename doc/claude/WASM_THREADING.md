# WASM threading — the two-tier design and Node.js worker threads (W1.18)

Split out of [WASM.md](WASM.md): `par()` under WASM: sequential fallback, Web Worker parallelism, and testing it under Node.js worker threads.

## Threading in WASM — Two-Tier Design

> **⚠ Superseded implementation (@PLN117).** The hand-rolled Web-Worker dispatch
> described below (`worker_entry`, a manual `SharedArrayBuffer` control buffer,
> `LoftThreadPool`) was **never implemented** (`worker_entry` was a no-op stub) and
> has been **retired**. Browser `par()` now rides **loft's own rayon pool**
> (`src/wasm_threads.rs` + `doc/loft-thread.js`) — the same rayon scheduler the
> native backend uses — via `src/parallel.rs::with_pool`. rayon schedules; loft
> only supplies the threads, because a browser has no `thread::spawn`. The pool is
> plain wasm exports with no host imports, which is why the SAME runtime serves
> the wasm-bindgen gallery bundle and the raw `loft --html` one. Build with
> `make wasm-mt`; the COOP/COEP
> contract, the two-tier (isolated → threaded, else → sequential) fallback, and the
> runtime detection idea below all still hold. The current, proven picture is
> **§ Cargo Feature Gates → Build commands** above and
> `doc/claude/plans/117-browser-multithreading/`. The text below is kept for the
> two-tier design rationale only.

### Overview

Loft's `par()` loop attribute spawns OS threads with shared mutable access to the
Store heap. Browsers can replicate this using **Web Workers + SharedArrayBuffer**,
but only when specific HTTP headers are present. The design uses two tiers:

```
┌─────────────────────────────────────────────────────────────┐
│                    Runtime detection                         │
│                                                             │
│  if (crossOriginIsolated && SharedArrayBuffer) {            │
│      → Tier 2: Web Worker thread pool (real parallelism)    │
│  } else {                                                   │
│      → Tier 1: Sequential fallback (same results, no par)   │
│  }                                                          │
└─────────────────────────────────────────────────────────────┘
```

| Tier | Environment | `par()` behaviour | Build |
|---|---|---|---|
| **Tier 1** | `file://`, simple static hosts, Node.js | Sequential — loop body runs inline | `--features wasm` |
| **Tier 2** | Hosts with COOP/COEP headers, CDN | Real parallelism via Web Workers | `--features wasm-threads` |

### Tier 1 — Sequential fallback

When the `threading` feature is disabled, all parallel entry points execute the loop
body sequentially in the main thread. The loft program behaves identically — same
results, same side effects — just without parallelism.

**Rust changes in `src/parallel.rs`:**

```rust
// The public entry points used by fill.rs:

#[cfg(feature = "threading")]
pub fn run_parallel_int(/* ... */) { /* existing thread-pool or Web Worker impl */ }

#[cfg(not(feature = "threading"))]
pub fn run_parallel_int(
    state: &mut State,
    stores: &mut Stores,
    start: i32,
    end: i32,
    body_fn: usize,
) {
    // Sequential fallback: just run the loop body inline
    for i in start..end {
        state.push_int(i);
        state.call(body_fn, stores);
    }
}

// Same pattern for run_parallel_raw, run_parallel_text, etc.
```

**Bytecode generation (`src/state/codegen.rs`):**

No changes needed. `gen_for` already emits `OpParallelFor*` opcodes regardless of
target. The opcodes dispatch through `src/fill.rs` to the `parallel.rs` entry points,
which are swapped at compile time.

**Loft-side behaviour:**

- `par(threads: 4)` attribute is parsed and accepted but ignored — the loop runs
  sequentially.
- `fn <name>` references (used for parallel worker functions) still work — they are
  called inline.
- No runtime error or warning is emitted. Programs that use `par()` remain valid.

### Tier 2 — Web Worker parallelism

When `SharedArrayBuffer` is available, loft can use real parallel execution in the
browser. The WASM linear memory (which contains the Store heap) becomes a
`SharedArrayBuffer` that multiple Web Workers share.

**Why this maps directly to loft's `par()` model:**

| loft native (src/parallel.rs) | WASM + Web Workers |
|---|---|
| `std::thread::spawn(closure)` | Web Worker on shared WASM memory |
| `unsafe { copy_nonoverlapping }` into Store | Same pointers — Store is in shared linear memory |
| `mpsc::channel` for results | `Atomics.wait()` / `Atomics.notify()` |
| `thread_local! { RNG }` | Per-Worker TLS (each Worker gets its own) |
| `thread::available_parallelism()` | `navigator.hardwareConcurrency` |

**Required HTTP headers (server-side):**

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Without these headers, the browser blocks `SharedArrayBuffer` construction (Spectre
mitigation). This is why Tier 2 cannot work from `file://` URLs.

**Rust implementation** (as shipped — @PLN117; the sketch this section originally
carried used `wasm-bindgen-rayon`, which loft no longer depends on):

```rust
// src/wasm_threads.rs — under #[cfg(feature = "wasm-native-threads")]

// The page spawns the Web Workers and sequences start-up; rayon schedules.
loft_pool_new(n) -> handoff    // create the handoff, mark this thread non-blocking
loft_rayon_start_worker(h)     // each worker: claim one rayon thread and run it
loft_pool_build()              // install the global pool once the workers are up
```

Plain wasm exports with no host imports, driven from `doc/loft-thread.js`.  That is
what lets ONE pool serve both the wasm-bindgen gallery bundle and the raw
`loft --html` one, whose glue builds its own imports and would reject an extra
module.  `std::thread::spawn` does NOT become a Web Worker under the atomics target
feature — it is unsupported, which is exactly why the threads come from the host.

Alternatively, if loft's parallel model doesn't fit rayon's work-stealing pattern,
the worker pool can be managed directly:

```rust
// Direct Web Worker management via wasm-bindgen + js-sys
#[cfg(all(feature = "threading", feature = "wasm-threads"))]
mod wasm_workers {
    use js_sys::SharedArrayBuffer;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        type Worker;

        #[wasm_bindgen(constructor)]
        fn new(url: &str) -> Worker;

        #[wasm_bindgen(method, js_name = postMessage)]
        fn post_message(this: &Worker, msg: &JsValue);
    }

    /// Each worker receives:
    /// - The SharedArrayBuffer (WASM linear memory)
    /// - The function index to execute
    /// - The iteration range [start, end)
    /// - The base pointer into Store for writing results
    pub fn spawn_workers(
        worker_count: usize,
        fn_index: usize,
        start: i32,
        end: i32,
        store_base: *mut u8,
    ) {
        let chunk_size = (end - start) / worker_count as i32;
        // ... distribute work, wait for completion via Atomics
    }
}
```

**Worker script (`ide/src/loft-worker.js`):**

```js
// Loaded by each Web Worker
import init, { worker_entry } from '../pkg/loft_wasm.js';

self.onmessage = async ({ data }) => {
  if (data.type === 'init') {
    // Initialise WASM with shared memory
    await init(data.module, data.memory);
    self.postMessage({ type: 'ready' });
  } else if (data.type === 'run') {
    // Execute the parallel loop chunk
    worker_entry(data.fn_index, data.start, data.end);
    // Signal completion via Atomics (no postMessage needed for result —
    // results are written directly to shared Store memory)
    Atomics.store(data.signal, data.worker_id, 1);
    Atomics.notify(data.signal, data.worker_id);
  }
};
```

**Main thread coordination:**

```js
// wasm-bridge.js
async function initWasmThreaded(wasmUrl, workerCount) {
  // Compile module (shared across workers)
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));

  // Create shared memory (this is the WASM linear memory)
  const memory = new WebAssembly.Memory({
    initial: 256,       // 16 MB
    maximum: 16384,     // 1 GB
    shared: true        // ← SharedArrayBuffer under the hood
  });

  // Spawn worker pool
  const workers = [];
  for (let i = 0; i < workerCount; i++) {
    const w = new Worker('src/loft-worker.js', { type: 'module' });
    w.postMessage({ type: 'init', module, memory });
    workers.push(w);
  }

  // Wait for all workers to be ready
  await Promise.all(workers.map(w =>
    new Promise(resolve => {
      w.onmessage = (e) => { if (e.data.type === 'ready') resolve(); };
    })
  ));

  // Init main thread WASM with same shared memory
  await init(module, memory);

  return { workers, memory };
}
```

**Synchronisation pattern for `par()` loops:**

```
Main thread                          Workers
    │                                   │
    ├─ Prepare iteration ranges         │
    ├─ Write ranges to shared memory    │
    ├─ Atomics.store(signal, 0)  ──────→│ Workers wake, read range
    ├─ Atomics.wait(done, ...)          │ Execute loop body
    │                                   │ Write results to Store (shared memory)
    │                              ←────│ Atomics.store(done, 1)
    ├─ All workers done                 │
    ├─ Continue execution               │
    │                                   │
```

Results don't need to be "sent back" — they're written directly to the Store heap
in shared memory, exactly like the native `copy_nonoverlapping` pattern.

### Runtime detection and WASM loading

```js
// app.js — choose tier at startup
export async function initLoft() {
  const threaded = typeof SharedArrayBuffer !== 'undefined'
                && crossOriginIsolated;

  if (threaded) {
    console.info('[loft] Tier 2: parallel execution via Web Workers');
    const cores = navigator.hardwareConcurrency || 4;
    const { workers, memory } = await initWasmThreaded(
      'pkg/loft_wasm_bg.wasm', cores
    );
    return { threaded: true, workers, workerCount: cores };
  } else {
    console.info('[loft] Tier 1: sequential execution (no SharedArrayBuffer)');
    await initWasm('pkg/loft_wasm_bg.wasm');
    return { threaded: false, workerCount: 0 };
  }
}
```

**IDE status indicator:**

```js
// Show the user which tier is active
const badge = document.getElementById('threading-badge');
if (loftEnv.threaded) {
  badge.textContent = `⚡ ${loftEnv.workerCount} threads`;
  badge.title = 'Parallel execution enabled (SharedArrayBuffer)';
} else {
  badge.textContent = '1 thread';
  badge.title = 'Sequential mode — serve with COEP/COOP headers for parallelism';
}
```

### Deployment configurations

| Hosting method | Headers | Tier | `par()` |
|---|---|---|---|
| `file://index.html` | None | 1 | Sequential |
| `python -m http.server` | None | 1 | Sequential |
| Nginx / Apache with COOP+COEP | Yes | 2 | Parallel |
| Cloudflare Pages (custom headers) | Yes | 2 | Parallel |
| GitHub Pages | No (can't set headers) | 1 | Sequential |
| Vercel / Netlify (with `_headers`) | Yes | 2 | Parallel |

**Minimal dev server with headers** (`ide/serve.mjs`):

```js
import http from 'http';
import { readFileSync, existsSync } from 'fs';

const MIME = { '.html': 'text/html', '.js': 'text/javascript',
               '.wasm': 'application/wasm', '.css': 'text/css',
               '.json': 'application/json' };

http.createServer((req, res) => {
  const path = `ide${req.url === '/' ? '/index.html' : req.url}`;
  if (!existsSync(path)) { res.writeHead(404).end(); return; }

  const ext = path.substring(path.lastIndexOf('.'));
  res.writeHead(200, {
    'Content-Type': MIME[ext] || 'application/octet-stream',
    'Cross-Origin-Opener-Policy': 'same-origin',
    'Cross-Origin-Embedder-Policy': 'require-corp',
  });
  res.end(readFileSync(path));
}).listen(8080);

console.log('Loft IDE: http://localhost:8080 (threads enabled)');
```

### Build pipeline

```sh
# ide/build.sh — updated to produce both tiers

#!/bin/sh
set -e

echo "Building single-threaded WASM..."
wasm-pack build --target web --out-dir ide/pkg/st -- --features wasm --no-default-features

echo "Building multi-threaded WASM..."
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  wasm-pack build --target web --out-dir ide/pkg/mt -- --features wasm-threads --no-default-features

echo "Building base filesystem..."
node ide/scripts/build-base-fs.js

echo "Done. Serve with: node ide/serve.mjs"
```

The loader picks the right package at runtime:

```js
const pkg = loftEnv.threaded ? 'pkg/mt/loft_wasm.js' : 'pkg/st/loft_wasm.js';
```

### Memory considerations for shared WASM

- `WebAssembly.Memory({ shared: true })` requires a fixed `maximum` declared at
  compile time. For loft this should be generous (1 GB) since the Store heap can
  grow.
- Shared memory cannot be grown dynamically in some browsers — set initial size
  large enough for typical programs (~64 MB) to avoid frequent grows.
- Each Web Worker loads the full WASM module but shares the same linear memory.
  Per-worker overhead is ~1-2 MB for the WASM instance plus JS context.

### Test impact

| Test file | Tier 1 | Tier 2 |
|---|---|---|
| `tests/threading.rs` | **Skip** — tests Rust `std::thread` directly | **Skip** — same reason |
| `tests/scripts/22-threading.loft` | Runs (sequential) | Runs (parallel via Workers) |
| All other tests | Pass | Pass |

The `22-threading.loft` test verifies output correctness (sums, vector contents), not
execution speed or thread count. It passes under both tiers without changes.

---

## W1.18 — Node.js Worker Threads: Testing `par()` Outside the Browser

> **⚠ Retired (@PLN117).** This whole node-Worker approach (`worker_entry` +
> `LoftThreadPool` + `worker.mjs`/`parallel.mjs` + `initThreaded`, built with
> `--target nodejs`) has been **removed**. Its premise — "node avoids the browser's
> COOP/COEP obstacle" — does not survive contact with `wasm-bindgen-rayon`, which is
> `--target web` only and needs a **Web Worker** global that node lacks. Browser
> `par()` runs on loft's own pool (`src/wasm_threads.rs`), and the proof harness is the
> **browser**: `make wasm-mt` + `tests/wasm/par-thread-proof.{html,sh}` +
> `coi-server.py`. The COOP/COEP requirement is real and documented under
> **§ Cargo Feature Gates → Build commands** above. The section below is obsolete;
> kept only as a record of the abandoned node-worker attempt.

### Why Node.js, not the browser

The browser Tier 2 design requires `SharedArrayBuffer`, which demands `COOP`/`COEP` HTTP
headers (Spectre mitigation). That makes `file://` URLs and most dev servers ineligible,
and adds a server-side prerequisite to every CI run.

Node.js removes all of these obstacles:

| Capability | Browser | Node.js |
|---|---|---|
| `SharedArrayBuffer` | Requires COOP/COEP headers | Always available |
| `Atomics.wait()` on main thread | **Blocked** (main thread is not allowed) | **Allowed** |
| Worker spawning | `new Worker(url)` | `new Worker(__filename, { workerData })` |
| Shared WASM memory | `SharedArrayBuffer` via `.memory` | Same — identical API |
| CI without a browser | Not possible | Yes — just `node` |

The test for `19-threading.loft` is currently `#[ignore]` under WASM (WASM_SKIP) because
Tier 1 runs `par()` sequentially. W1.18 enables real parallel execution in Node.js by
implementing Tier 2 entirely within the existing `tests/wasm/` harness.

---

### Architecture

```
tests/wasm/
├── parallel.mjs           ← NEW: thread pool manager
├── worker.mjs             ← NEW: worker thread entry point
├── harness.mjs            — extended: detect CPU count, init pool
├── host.mjs               — extended: parallel_run / parallel_wait hooks
└── pkg/                   — wasm-pack output (wasm-threads feature build)
```

The WASM module is compiled **once** and shared across all workers via
`WebAssembly.Module`, which is structured-cloneable. The WASM linear memory is created
as a `SharedArrayBuffer`-backed `WebAssembly.Memory` (`{ shared: true }`) so every worker
operates on the same Store heap — exactly matching the native `thread::spawn` + shared
`Stores` model.

---

### Shared memory layout

One `Int32Array` on top of a separate small `SharedArrayBuffer` carries the control
signals (not inside the WASM heap itself, to avoid alignment issues):

```
Control buffer — Int32Array(N_WORKERS * 4 entries):

  offset 0..N:       per-worker command  (0=idle, 1=run, 2=exit)
  offset N..2N:      per-worker fn_index (bytecode entry point)
  offset 2N..3N:     per-worker start    (inclusive element index)
  offset 3N..4N:     per-worker end      (exclusive element index)
```

Results are written directly to the Store heap inside shared WASM memory — no transfer
needed, matching the native `copy_nonoverlapping` pattern.

A second `Int32Array(N_WORKERS)` — `doneSignal` — is used for completion notification:
each worker writes `1` and calls `Atomics.notify` when its chunk finishes.

---

### Worker entry point (`tests/wasm/worker.mjs`)

```js
import { receiveMessageOnPort, parentPort, workerData } from 'node:worker_threads';

const { module, memory, control, done, workerId } = workerData;

// Initialise WASM with the shared memory
const { instance } = await WebAssembly.instantiate(module, {
  env: { memory },
  // host bridge functions injected identically to harness.mjs
  loftHost: buildWorkerHost(),
});

const { worker_entry } = instance.exports;

// Signal ready
Atomics.store(done, workerId, 0);
parentPort.postMessage({ type: 'ready' });

// Work loop — park until signalled
while (true) {
  Atomics.wait(control, workerId, 0);           // sleep until cmd != 0
  const cmd = Atomics.load(control, workerId);
  if (cmd === 2) break;                          // exit

  const N = done.length;
  const fnIndex = Atomics.load(control, N      + workerId);
  const start   = Atomics.load(control, N * 2  + workerId);
  const end     = Atomics.load(control, N * 3  + workerId);

  worker_entry(fnIndex, start, end);             // write results to shared Store

  Atomics.store(done, workerId, 1);
  Atomics.notify(done, workerId);
  Atomics.store(control, workerId, 0);           // reset to idle
}
```

`buildWorkerHost()` supplies the same `loftHost` bridge as `host.mjs` but wired to a
per-worker `VirtFS` snapshot (read-only view of the main thread's virtual filesystem).

---

### Thread pool manager (`tests/wasm/parallel.mjs`)

```js
import { Worker } from 'node:worker_threads';
import { fileURLToPath } from 'node:url';

const WORKER_SCRIPT = fileURLToPath(new URL('./worker.mjs', import.meta.url));

export class LoftThreadPool {
  constructor(module, memory, nWorkers) {
    this.nWorkers = nWorkers;
    // Control buffer: 4 slots × N workers (command, fn_index, start, end)
    this.control  = new Int32Array(new SharedArrayBuffer(4 * nWorkers * 4));
    this.done     = new Int32Array(new SharedArrayBuffer(nWorkers * 4));

    this.workers = Array.from({ length: nWorkers }, (_, id) =>
      new Worker(WORKER_SCRIPT, {
        workerData: { module, memory, control: this.control,
                      done: this.done, workerId: id },
      })
    );
  }

  /** Wait for all workers to post { type: 'ready' }. */
  async waitReady() {
    await Promise.all(this.workers.map(w =>
      new Promise(resolve => {
        w.once('message', (msg) => { if (msg.type === 'ready') resolve(); });
      })
    ));
  }

  /**
   * Distribute a par() loop across all workers.
   * @param {number} fnIndex  — WASM function table index for the worker body
   * @param {number} total    — total number of elements
   */
  runParallel(fnIndex, total) {
    const chunkSize = Math.ceil(total / this.nWorkers);
    const N = this.nWorkers;

    for (let t = 0; t < N; t++) {
      const start = t * chunkSize;
      const end   = Math.min(start + chunkSize, total);
      Atomics.store(this.done,    t,         0);
      Atomics.store(this.control, N      + t, fnIndex);
      Atomics.store(this.control, N * 2  + t, start);
      Atomics.store(this.control, N * 3  + t, end);
      Atomics.store(this.control, t,          1);      // command = run
      Atomics.notify(this.control, t);
    }

    // Main thread waits for all workers
    for (let t = 0; t < N; t++) {
      Atomics.wait(this.done, t, 0);                   // wait until done[t] == 1
    }
  }

  /** Shut down all workers. */
  terminate() {
    for (let t = 0; t < this.nWorkers; t++) {
      Atomics.store(this.control, t, 2);               // command = exit
      Atomics.notify(this.control, t);
    }
    return Promise.all(this.workers.map(w => w.terminate()));
  }
}
```

---

### Harness integration (`tests/wasm/harness.mjs` — additions)

```js
import { LoftThreadPool } from './parallel.mjs';
import os from 'node:os';

// Build wasm-threads feature binary for threaded tests
// wasm-pack build --target nodejs --out-dir tests/wasm/pkg-mt \
//   -- --features wasm-threads --no-default-features

async function initThreaded(wasmPath, nWorkers = os.cpus().length) {
  // Fetch and compile the module once (structured-cloneable)
  const bytes  = readFileSync(wasmPath);
  const module = await WebAssembly.compile(bytes);

  // Shared memory — this becomes the Store heap
  const memory = new WebAssembly.Memory({
    initial:  256,    // 16 MB
    maximum:  16384,  // 1 GB
    shared:   true,
  });

  const pool = new LoftThreadPool(module, memory, nWorkers);
  await pool.waitReady();

  // Instantiate the main-thread copy
  const { instance } = await WebAssembly.instantiate(module, {
    env:      { memory },
    loftHost: createHost(new VirtFS(baseTree)),
  });

  // Register the pool so the host bridge can dispatch par() calls
  instance.exports.set_thread_pool_ptr(/* ptr to pool dispatch table */);

  return { instance, pool, memory };
}
```

The `parallel_run(fn_index, total)` and `parallel_wait()` calls from `fill.rs`
(via the WASM host bridge) route to `pool.runParallel(fnIndex, total)`.

---

### W1.18-6 — Build and test environment

#### Prerequisites

```bash
# 1. Install wasm-pack (if not already)
cargo install wasm-pack

# 2. Add the WASM target
rustup target add wasm32-unknown-unknown

# 3. Node.js v16+ (for Worker Threads + SharedArrayBuffer)
node --version   # must be >= 16
```

#### Building the threaded WASM module

The threaded build requires atomics, bulk-memory, and mutable-globals target
features.  These are passed via `RUSTFLAGS`:

```bash
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  wasm-pack build --target nodejs \
    --out-dir tests/wasm/pkg-mt \
    -- --features wasm-threads --no-default-features
```

This produces `tests/wasm/pkg-mt/loft_bg.wasm` with `SharedArrayBuffer`-backed
memory support.  The build is separate from the single-threaded `pkg/` build
(which uses `--features wasm` only).

A Makefile target is provided for convenience:

```bash
make wasm-mt     # builds pkg-mt/
```

#### Running the threaded test

Once `pkg-mt/` exists:

```bash
# Run the threading test through the WASM Worker Thread pool
node tests/wasm/suite.mjs --threaded 19-threading.loft
```

Or remove `19-threading.loft` from `WASM_SKIP` in `tests/wrap.rs` and run:

```bash
cargo test --test wrap wasm_dir
```

#### CI integration

Browser threading has its own workflow — **`.github/workflows/browser-threads.yml`**
(`Browser threading`), which runs the five headless gates nightly at 05:00 UTC and on
any PR touching the threading surface (`src/parallel.rs`, `src/wasm_threads.rs`,
`wasm-sync/`, `doc/loft-thread.js`, `tests/wasm/`, …).  It installs what the rest of
CI has no reason to carry: nightly + `rust-src` (the atomics std comes from
`-Z build-std`), `wasm-pack`, binaryen, and a headless Chrome.  Locally the same run
is `make par-gates`.

Two properties keep it honest:

- **A skipped gate is a CI failure.** Every gate exits 0 when a prerequisite is
  missing so a dev without a browser still gets a useful run; `scripts/par_gates.sh
  --ci` preflights the prerequisites and turns any `SKIP` into a red job, because a
  gate that skips itself green is the one way this can rot unnoticed.
- **The performance floors are calibrated, not disabled.** A runner has 4 vCPUs
  against the dev box's 24, so the workflow lowers the scaling and frame-rate floors
  (`POOLS`, `SPEEDUP_AT`, `MIN_SPEEDUP_PCT`, `THREADS`, `MIN_FRAME_RATIO_X10`) to what
  that hardware delivers — a `par` that stops parallelising still fails.

A red run is surfaced on every PR by ci.yml's `Nightly health (informational)` job.

#### Implementation status

| Step | Status | File |
|------|--------|------|
| W1.18-1 | ✓ Done | `src/parallel.rs` — `#[cfg(wasm+threading)]` branch |
| W1.18-2 | ✓ Done (stub) | `src/wasm.rs` — `worker_entry` export |
| W1.18-3 | ✓ Done | `tests/wasm/worker.mjs` — park/wake loop |
| W1.18-4 | ✓ Done | `tests/wasm/parallel.mjs` — `LoftThreadPool` |
| W1.18-5 | ✓ Done | `tests/wasm/harness.mjs` — `initThreaded()` |
| W1.18-6 | Pending | Remove from `WASM_SKIP` after `pkg-mt/` build verified |

---

### Rust-side WASM host bridge additions (W1.18)

Two new `extern "C"` imports are declared in `src/parallel.rs` under
`#[cfg(all(target_arch = "wasm32", feature = "threading"))]`:

```rust
#[wasm_bindgen]
extern "C" {
    /// Distribute fn_index over `total` elements using the JS worker pool.
    /// Blocks (via Atomics.wait in JS) until all workers complete.
    fn parallel_run(fn_index: u32, total: u32);
}

#[cfg(all(target_arch = "wasm32", feature = "threading"))]
pub fn run_parallel_raw(
    _stores: &mut Stores,
    _program: &[u8],
    fn_pos: u32,
    _input: &DbRef,
    _element_size: u32,
    _return_size: u32,
    n_elements: u32,
) -> Vec<u64> {
    // Dispatch to the JS worker pool; results are already in shared Store memory
    unsafe { parallel_run(fn_pos, n_elements); }
    // Return an empty Vec — caller reads results directly from shared Store
    vec![]
}
```

Results land in shared WASM linear memory (the Store heap), identical to the native
`copy_nonoverlapping` path — no serialisation, no transfer, no post-processing.

---

### `worker_entry` export (Rust)

A new `#[wasm_bindgen]` export gives workers their entry point:

```rust
/// Called by each JS worker to execute one chunk of a par() loop.
/// fn_pos:   bytecode position of the worker function
/// start:    first element index (inclusive)
/// end:      last element index (exclusive)
#[wasm_bindgen]
pub fn worker_entry(fn_pos: u32, start: u32, end: u32) {
    WORKER_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        for i in start..end {
            state.execute_at_raw(fn_pos, &DbRef::element(i), 4);
        }
    });
}
```

`WORKER_STATE` is a `thread_local!` — each Web Worker / Node.js worker thread gets its
own `State` instance backed by the shared `Memory`, matching `clone_for_worker()` in
native.

---

### Sequence diagram

```
Main thread                         Worker 0..N-1
    │                                    │
    ├─ compile module (once)             │
    ├─ create shared Memory              │
    ├─ spawn N workers via Worker()      │
    │    ←── { type: 'ready' } ─────────┤ workers init WASM + park on Atomics.wait
    │                                    │
    │  [par() loop begins]               │
    ├─ write fn_index, start, end        │
    ├─ Atomics.store(control[t], 1)      │
    ├─ Atomics.notify(control[t]) ──────→│ workers wake, call worker_entry()
    ├─ Atomics.wait(done[t], 0) (block)  │ results written to shared Store heap
    │                              ←─────│ Atomics.store(done[t], 1) + notify
    ├─ all done                          │
    ├─ read results from shared Store    │ workers park again on Atomics.wait
    ├─ continue execution                │
    │                                    │
    │  [test teardown]                   │
    ├─ Atomics.store(control[t], 2)      │
    ├─ Atomics.notify(control[t]) ──────→│ workers exit
    └─ pool.terminate()                  │
```

---

### Build target

A second wasm-pack build produces the threaded binary for W1.18 tests:

```sh
# Single-threaded (existing, all other WASM tests)
wasm-pack build --target nodejs --out-dir tests/wasm/pkg \
  -- --features wasm --no-default-features

# Multi-threaded (W1.18 tests only)
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  wasm-pack build --target nodejs --out-dir tests/wasm/pkg-mt \
  -- --features wasm-threads --no-default-features
```

`harness.mjs` selects `pkg-mt` for tests tagged `@threaded`; all other tests
continue using `pkg`.

---

### Test coverage

Once W1.18 lands, `19-threading.loft` is removed from `WASM_SKIP` in `tests/wrap.rs`:

```rust
// tests/wrap.rs — WASM_SKIP list (W1.18 removal)
// "19-threading.loft",   ← removed when W1.18 is complete
```

The threading test file exercises:
- `par()` with Form 1 worker (`double_score(a)`)
- `par()` with Form 2 method (`a.get_value()`)
- Result ordering (results must match sequential order)
- Multi-core count (`par(..., 4)` attribute)

---

### Implementation steps (W1.18)

| Step | File | Description |
|------|------|-------------|
| W1.18-1 | `src/parallel.rs` | Add `#[cfg(wasm+threading)]` branch: `parallel_run` import + `run_parallel_raw` stub |
| W1.18-2 | `src/lib.rs` | Export `worker_entry(fn_pos, start, end)` via `#[wasm_bindgen]` |
| W1.18-3 | `tests/wasm/worker.mjs` | Worker thread script: init WASM, park/wake loop, call `worker_entry` |
| W1.18-4 | `tests/wasm/parallel.mjs` | `LoftThreadPool` class: spawn, `runParallel`, `terminate` |
| W1.18-5 | `tests/wasm/harness.mjs` | `initThreaded()` helper; route `@threaded` tests to `pkg-mt` |
| W1.18-6 | `tests/wrap.rs` | Remove `19-threading.loft` from `WASM_SKIP`; add threaded build step |

**Effort:** H (as in ROADMAP.md — shared memory + Atomics protocol + WASM export plumbing)
**Design:** ✓ (this section)
