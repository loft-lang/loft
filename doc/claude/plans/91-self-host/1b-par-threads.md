<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 strand 1b — `par` on threads in the runner

A child document of [1b-fill-port.md](1b-fill-port.md), for its `par` row.  Status: **not
built.**  The runner runs a `par` loop's workers one after another (`natives.loft` `par_start` /
`par_return`, entered from `fill_run.loft`) and a `parallel { }` block's arms in turn
(`fill_run.loft` `par_block`).  The answers are right; the time is one core's: `par_text` on four
cores takes 3.0 s on the runner against the Rust interpreter's 1.08 s (task-clock equal to
elapsed — one CPU).  `bench_speed.py` pins every routine to one core, so it measures the
single-thread cost only.

## What the rules allow

[formal/concurrency.md](../../formal/concurrency.md):

- `(C-Par)` — the workers run in any order on N threads, the body reads the results in source
  order, and a struct or reference result is deep-copied into the result vector (`H-Copy`), so
  no worker's store escapes.
- `(C-Det)` — a PURE worker (it reads only its element and its arguments and changes no shared
  state) answers what the sequential loop answers, for every N: N is a performance knob.
- `(C-Impure)` — an impure worker's result is undefined; a data race is a program error.

So the sequential runner conforms, and a threaded one would too: for a pure worker nothing can
change, and an impure one is not protected.  Threading needs no new rule.

## What the C runtime already does

The runner is a loft program compiled by the C backend, and that runtime threads `par`
(`loft_par_run`, `src/c/loft_keyed.h`, [C_BACKEND.md](../../C_BACKEND.md) § par): each worker
thread gets a SHALLOW copy of the store table — the same store memory, every store that existed
before the fork pinned so no thread frees it — and writes only into stores it makes itself;
a record result is copied into the caller's result store after the join (`loft_par_adopt`).
That is exactly the model a pure worker needs: shared reads, private writes.  A site whose
worker answers a scalar, a text, nothing, or a struct record runs on threads.

## What the runner has to change

The runner's own interpreter is the impure part — its state is shared mutable state in the
sense `(C-Impure)` forbids:

1. **The `State` record is written on every operator**: the stack store and the frames,
   `code_pos` and `stack_pos`, and the caches filled on first use (the target cache and table,
   the keyed facts, the record images, the light natives, the interned texts, `table_loaded`).
   On shared store memory two threads writing one `State` race.  Each worker thread needs a
   `State` of its own — its own stack store and frames — reading the run image read-only
   (pinned, so safe), its caches rebuilt per thread or filled before the fork.
2. **The crt shim keeps process-wide globals the runner writes per operator**
   (`selfhost/crt/shim/crt.c`): the key buffer `push_keys` fills for a keyed lookup
   (`loft_crt_kv`), the show, read and write buffers (`loft_crt_show_buf`, `loft_crt_rbuf`,
   `loft_crt_wbuf`), the `#c` call cache (`loft_crt_cfn`), the sidecars.  The core runtime marks
   its per-thread state `LOFT_TLS`; the shim does not.  Each becomes `LOFT_TLS` or moves into
   the `State`.  The type tables loaded once (`loft_crt_types`, `loft_crt_tnames`) are read-only
   after the load and stay shared.
3. **A record result of the program's worker** is made in the worker thread's own stores, and
   `(C-Par)` / `H-Copy` want it deep-copied into the caller's result store after the join.
   `loft_par_adopt` does that for the compiled program's own types, not for the INTERPRETED
   program's records: the runner needs its own adopt over the program's type table.  A scalar
   or text result is copied out as a value.
4. **The shape.**  loft's `par` has no per-worker set-up, and a fresh `State` per row would cost
   more than the row.  The form that fits is one row per thread —
   `for t in 0..nth par(r = run_slice(t), nth)` — each row interpreting its slice of the
   program's rows (`parallel_workers`' split: rows `[t*n/nth, (t+1)*n/nth)`) and answering a
   struct that carries the slice's results, which the runtime threads as a struct result.

The same work serves the `parallel { }` block, whose arms Rust runs on threads with a copy of the
stores (1b-fill-port.md difference 10).

## Steps, each checked on its own

- **S1** — the shim's per-operator globals made `LOFT_TLS`; no behaviour change, the survey and
  `selfhost/tests` hold.
- **S2** — a `State` built for a worker from the image and the caller's `State`'s read-only
  parts; run every slice sequentially through it first (still one thread): the survey holds.
- **S3** — the slices on `par` threads for scalar and text results; a fold's partials combined
  in slice order (`(C-Det)`'s fold arithmetic, `run_parallel_fold`).
- **S4** — record results adopted into the caller's result store; the `parallel { }` block.

Each step's check: the survey (every corpus `par` program), the `par` cells of loft-suite run
through the runner at N = 1, 2 and 4 (`(C-Det)`: the same answer for every N), and
`par_text` / `par_map_float` / `par_records` timed on four cores.
