<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 226 — loft without thread-locals

## Status

Open, `status:active` on [loft-lang/plans#226](https://github.com/loft-lang/plans/issues/226).

| Phase | State |
|---|---|
| P0 | done: measured below |
| P2a | done (f048ff16e): every C runtime and emitted function takes `loft_cx *cx` first; fixed ABIs declare theirs |
| P2b | done: the state of `loft_text.h`, `loft_store.h`, `loft_rt.h`, `par`, the FFI claim log and the hosted bridge is fields of `struct loft_cx` (`src/c/loft_cx.h`); workers', hosted units' and HTTP workers' contexts made as below; TSan clean on the `par` corpus |
| P3 | done: the parks are main-thread statics (below) |
| P1 | done: the B-tree hints, hash scratch, image checks, JSON errors and null sentinel, key seeds, the typed parse's walk and errors, the upgrade walk and the engine's message deadline are context fields too (unguarded: a header's switch is set at different points of a unit, and a field costs words) |
| P4 | done: the URL loader's state is context fields; the `net` component keeps its own context per caller context (below) |
| P5 | done: `LOFT_TLS` is gone (`git grep LOFT_TLS -- src/c` names only the TLS transport's `LOFT_TLS_UNAVAILABLE`); a strict C99 `par` unit runs on threads |
| P4b | in progress: 35 `thread_local!` blocks in 20 files remain (`git grep -c 'thread_local!' -- src`).  Moved: the compile passes' memos into `Data` (`PassCell`; the span memos, the definitions and the two parameter-write summaries, as `ScopedCell`; the copy manifest), a run's checks and reports into `Stores` (`checks`, `run`, `render`, `hosted_cx`), the bind notes and text yields into `State`, the C emission options into `UnitShape`; a native call's library index, bridge mark, claim log and calling interpreter into `Stores`, the interpreter's FFI callbacks finding their `Stores` in the store handle as the C does (`HOST_ABI` 8: the coroutine table's `retain` and `exhausted` take the `Stores`); the wired natives' signatures, bridge labels and stub symbols into one per-program table (`Stores::wired`), in place of two locked process tables, their per-thread caches and a generation counter; a text read's reference state into `Stores`, the hosted verify's twin depth and the stack and op censuses into `State`; the store instruments (`LOFT_ALLOC_TRACE`'s site and worker, the stack store, the bytes moved) into a per-context `store_census::Tap` each store of the context shares, a store's relocations (`LOFT_VERIFY_STACK`) and its discarded-write scratch into the `Store`; the index-walk hints into `Stores` (the C's `cx->bt_hints`); a parse's manifest errors into the `Parser`; five locks removed ([LOCKS.md](../LOCKS.md)).  Open, by group: the rustc writer's runtime, gone with @PLN225 (`codegen_runtime`; `call_frames`' shadow stack and lazy-driver flag; `extensions`' `CURRENT_STORES` for `native_call`); the engine host and live reload; wasm and the browser; the crash report and the store-trace names a fault reads (`crash_report`, `store_trace`); the REPL's panic silence; the debug session's output capture; the emulated-host file tests |

### Decisions taken while building it

- **One struct, like `Stores`.**  Every per-thread variable is a field of `struct loft_cx`, named
  without its `loft_` prefix (`loft_stores` → `cx->stores`).  The header holds by value only the
  types it defines (`loft_text`, `loft_ref`, the jump buffer, the hosted locals entry, the show
  callback); a field pointing at a runtime type names its struct tag.
- **A worker's context is made by its caller** in `loft_par_job_init`, before the thread starts:
  `loft_cx_fresh`, its table the copy, `no_park` set.  `loft_cx_end` releases what a context
  holds besides its table — what a thread's exit leaked before.
- **The parks are the main thread's.**  A park slot is a plain static; `loft_wpark_take`,
  `loft_park_take` and the puts all check `cx->no_park`, so a `par` worker and a hosted bridge
  call never touch one.  (A per-thread slot was always empty on a worker before: same answers.)
- **A hosted unit's context belongs to the `Stores` that calls it** (`HostedTable::cx_of`,
  `host_abi::HostedContexts`), as its table view does: an interpreter `par` worker's `Stores` is
  its own, and so is its context.  The entry is appended to `HostedTable`, so `HOST_ABI` stays.
- **A library native's callback finds its caller's context in the store handle.**  The loft-ffi
  ABI fixes `opaque` as the store number, and both the C helper and the Rust crate read its low
  16 bits alone (`loft_ffi_store_nr`); the context pointer rides above them
  (`loft_ffi_handle`).  It needs 64-bit pointers, asserted at compile time.  The interpreter's
  own callbacks (`extensions.rs`, `CURRENT_STORES`) can carry theirs the same way in P4b.
- **A Rust thread-local becomes a field of what it describes.**  A compile pass's memo lives in `Data` behind a `PassCell` (a `Mutex`, because `Data` is shared with `par` workers; cloned empty), a run's checks and one-time reports in `Stores` (`RefCell`, a worker's `Stores` its own), an emission's options in `UnitShape`.  A flag read from the environment is `env_once!`.
- **The TLS archive keeps its C ABI** (`loft_tls.c`, `loft_ssh.c`, compiled apart): its
  prototypes take no context.
- **A component has a context of its own per caller context.**  The caller's own cannot be
  passed: the component is compiled with its own switches, so its struct is laid out its own
  way.  `libloft_net` exports `loft_cx_make_v1` / `loft_cx_end_v1`; its typed symbols and its
  bridges (`loft_register_bridges_cx_v1`) take that context first.  A C unit keeps one per
  component in its own context (`loft_component_cx`, ended by `loft_cx_end`), the interpreter
  one per `Stores` (`c_libs::CX_BRIDGES`, kept in `HostedContexts`).  The interpreter's own bit
  above the store number in `opaque` (`CTX_RETURN_STORE`) is why the context is an argument and
  not packed in the handle.
- **A component's cache key names its export list** (`component.rs` `EXPORTED`): a change of it
  rebuilds the library, where a stale one lacked the new symbols.

## Goal

loft keeps no state in thread-locals: the C runtime and the C it emits (`LOFT_TLS`, 178
lines) and the Rust runtime and interpreter (`thread_local!`, 85 blocks in 44 files —
`git grep -c 'thread_local!' -- src`).  `par`, `par_fold` and `parallel { }` answer as they do
today on every host, threads or none.

Thread-locals, like reference counts, are almost never needed and hide bugs (owner): state
that outlives the run, the test or the worker it belongs to.  The Rust ones exist to avoid
adding a field to `Stores` (`NATIVE.md` § N8b: "Thread-local storage avoids modifying
`Stores`") — the call frames and depth (`call_frames.rs`), the native coroutines, the
lazy-driver flag, the bridge markers — while every compiled function already receives its
`Stores` and every `par` worker already has its own.  Each becomes a field of the `Stores` (or
`State`) it belongs to, and the C ports that design (C_BACKEND.md § Port, never reinvent).

## The design (owner)

**A store is claimed in the main thread.  A worker starts its own stores and reads the common
ones.  Nowhere in that design is a thread-local needed.**

It is the formal model's own shape (`formal/concurrency.md`):

- `C-Par` — a worker is a pure function of its element and the context arguments.  It READS
  the parent's frame and heap, which nothing writes while the workers run, and every store it
  makes is its own; only its result crosses back, deep-copied (`H-Copy`) into a store the
  main thread claimed.
- `C-Arms` — an arm of a `parallel` block runs against its own copy of the frame at entry,
  writes none of the enclosing state, and whatever it makes is gone when it ends.

So there are two kinds of state, and neither is per-thread in the C sense:

| State | Owner | Where it lives |
|---|---|---|
| The common stores, the program's frames, its fault state | the main thread | plain globals; a worker only reads the stores |
| A worker's own stores, its frames, its fault jump, its scratch | the worker | its own store context, made by the main thread before it starts the worker and passed to every function the worker runs |

## What the thread-locals hold today

`git grep -c LOFT_TLS -- src/c` measures it (178 lines).  By kind:

| Kind | Sites | What replaces it |
|---|---|---|
| The store table (`loft_stores`, `loft_nstores`, allocation counters, hint tables, the block pool, park slots) | `loft_store.h`, `loft_rt.h` | fields of the store context |
| Frames, the call chain, the depth count, the fault jump, the test jump, held texts, the lazy-driver jump | `loft_rt.h` | fields of the store context |
| Scratch of ONE runtime call (JSON parse errors, the reflection and record-reference walks, `show`'s closers, equality jobs, pack buffers, upgrade tables) | `loft_parse.h`, `loft_json.h`, `loft_upgrade.h`, `loft_image.h`, `loft_keyed.h`, `loft_rt.h` | a local of that call (on the stack, or allocated and freed by the call) |
| Per-function work-buffer parks the emitter writes (`static LOFT_TLS loft_text` slots, `body.rs`, `split.rs`) | emitted code | fields of the store context (a park per function slot) |
| Rust: call frames and depth, crash report, lazy-driver flag, bridge markers, store census, key caches, engine host, extensions (`thread_local!`) | `src/call_frames.rs`, `crash_report.rs`, `engine_host.rs`, `extensions.rs`, `database/allocation.rs`, `keys.rs`, `store_census.rs`, … | fields of the `Stores` or `State` they describe; the writer's (`codegen_runtime.rs`) go with @PLN225 R7 |
| Server, web and SSH connection tables | `loft_server.h`, `loft_web.h`, `loft_ssh.h`, `loft_net.h` | the server's own state, owned by the thread that runs the server loop: a global, or a value the loop holds |
| The hosted unit's bridge answer and generator text (`hosted.rs`) | emitted code | the bridge call's own stack, or the `Stores` the interpreter hands the call |
| The FFI call log, B-tree hints, hash-seed state | `loft_ffi_host.h`, `loft_btree.h`, `loft_hash.h` | per call, or main's global where only main writes |

## How the code reaches its stores: as the rustc writer and the interpreter do

Both other implementations already run `par` without a thread-local, the same way:

- **The stores are an argument.**  Every function the rustc writer emits takes them first
  (`cell: &UnsafeCell<Stores>`, `src/generation/`); the interpreter's `State` holds its own
  `Stores`.  Nothing reaches a store through a global.
- **A worker is handed its own `Stores`.**  `Stores::clone_for_light_worker` makes it before
  the worker starts: every store of the parent BORROWED, locked read-only
  (`borrow_locked_for_light_worker`), plus a few writable scratch stores the worker allocates
  into and grows from.  `par_pool::parallel_workers` hands that view to the worker closure as
  its `cell`; the results merge on the main thread (`par_pool::merge_batches`) into the stores
  the main thread owns.
- **Nested `par`** inherits the parent's context the same way (THREADING.md § Nested `par`).

The C follows it: **every emitted function and every runtime helper that touches a store takes
the store context (`loft_stores_t *`) as its first argument.**  The main thread's context is
the program's; a worker's is made by the main thread before the worker starts (the common
stores borrowed read-only, its own stores writable), and the frames, the fault jump, the
depth count and the work-buffer parks are fields of that context.  P0 measures the cost of the
argument on a call-heavy program against today's thread-local reads; the rustc writer pays it
on every call already.

## P0 — measured

A
recursive function, a zero-argument runtime function and two `par` worker threads, each worker
handed its own context made by the caller before the thread starts, written in strict C11
(`cc -std=c11 -pedantic -Wall -Wextra`): no warning, and a ThreadSanitizer run reports nothing;
each thread's count is its own.  On 39 088 169 calls of a recursive function, a depth count in
a context argument runs 0.183 s, the same count in a plain global 0.182 s, none at all 0.094 s
(`-O3`, Apple clang, arm64).  The argument costs what a global costs.

**The shape (owner: nothing hidden).**  `cx` is written at every call and in every signature,
as the rustc writer writes `cell` — never supplied by a macro, which would hide the argument
as a thread-local hides the state.

## Phases

| Phase | What | Validated by | Effort |
|---|---|---|---|
| **P0** | Probe: one `par` program's unit rewritten by hand with the context argument (a worker's context made as `clone_for_light_worker` makes it); the time of a call-heavy sequential program and a `par` one against today's. | The hand-written unit answers as today's under TSan; the times recorded here. | S |
| **P1** | Call-scoped scratch becomes locals of the call that uses it (JSON, reflection walks, `show`, equality, pack, upgrade). | Those headers name no `LOFT_TLS`; their suites (`json*`, `reflect*`, `store_upgrade`, `eq*`) unchanged on the C, under `par` too. | M |
| **P2** | The store context: every emitted function and store-touching runtime helper takes it first; main's is the program's, a worker's is made by the main thread before it starts (the common stores borrowed read-only, its own writable) and holds its frames and fault state. | `git grep LOFT_TLS -- src/c/loft_rt.h src/c/loft_store.h` empty; `tests/par_*.rs`, `runtime_errors` (worker faults), `lazy_sql_source` (contained faults) green on the C; TSan run of the `par` corpus clean. | L |
| **P3** | Emitted state: the work-buffer parks and slots (`body.rs`, `split.rs`) as fields of the store context; the hosted unit's bridge state on the call. | No emitted `LOFT_TLS`; `c_emit`, `hosted_unit`, `stdlib_unit` green. | M |
| **P4** | Server, web, SSH, FFI log, hints: owned by the thread that runs them. | Their suites green; `git grep LOFT_TLS -- src/c` empty. | M |
| **P4b** | The Rust thread-locals: each a field of the `Stores` / `State` it describes (call frames and depth first — the C's frames port them). | `git grep 'thread_local!' -- src` empty; the full suite on both backends; `runtime_errors`, `par_*`, `lazy_sql_source` on the interpreter. | M |
| **P5** | Remove `LOFT_TLS` itself, the C11 floor's one reason (`C_BACKEND.md`: `_Thread_local`), and "a strict C99 build runs on one thread". | `cc -std=c99 -pedantic -DLOFT_STRICT` builds a `par` unit that runs on threads; full suite and the threading CI legs green. | S |

**Order:** P0 → P1 (independent) → P2 → P3 → P4 → P4b → P5.

## Cost

Each rule of formal/concurrency.md § Each thread's own state, measured 2026-11-23 against
f048ff16e (the context argument in place, the state still in thread-locals): arm64 macOS, the
median of nine alternated runs, each program's answer checked.

| Case | Program | Thread-locals | Contexts |
|---|---|---|---|
| B1 — a worker's context made and ended | 3 000 `par` loops, 8 rows, 4 threads (`--native-c`) | 105.8 ms | 99.6 ms |
| B2 — a component call | 40 M `web_natives::byte_at` (`--native-c`) | 22.9 ms | 22.4 ms |
| B3 — a hosted bridge call | 2 M `clamp` through the compiled stdlib (`--interpret`), less start-up | 130.3 ms | 130.3 ms |
| B4 — the compile passes' memos behind a lock | `--emit-c` of 947-feature-worked-examples, less start-up | 544.5 ms | 544.1 ms |

- **B1 first read 12 % slower** (108.7 against 96.7 ms): the context held the transient
  arena's 64 KiB first block by value and `loft_cx_fresh` zeroed it for every worker.  The block
  is now allocated by the first transient value a thread builds; a context is 4.4 KiB.
- **Start-up read 100 ms slower, and was not this work.**  `cache::loft_build_fingerprint`
  locates the rlib through `cargo_layout::refresh_runtime_uplift`, which asked `same_file` before
  the modification times — and `same_file` compares content when the uplift is a copy, so every
  start read two 87 MB rlibs (90 ms of kernel time, 175 MB resident).  Fixed: the times first.
  What remains in this checkout (about 70 ms) is `classic_deps_is_debris` checking the identity
  of each per-unit build directory, 3 048 of them after weeks of builds; a fresh tree has a
  handful, and an installed `loft` has none of that layout.
- **Loops at once** (`C-Nest`): two `par` loops in two `parallel` arms and a `par` loop nested in
  a worker answer the hand-computed values on both backends, and twenty ThreadSanitizer runs of
  the C report no race.

## Cross-arc dependencies

- **@PLN225** (retire the rustc writer): its ship-tier call-depth count (`--ship` keeps every
  fault check) is a plain static in a unit without threads; in a unit whose `par` runs on
  threads the count waits for P2, where it is a field of the store context.
- `@C170`: the C must answer as the interpreter does; nothing here changes an answer.

## Documentation

- `doc/claude/C_BACKEND.md` — the C11 floor and `LOFT_TLS` paragraph; the store context.
- `doc/claude/THREADING.md` — the C runtime's worker model beside the interpreter's.
- `doc/claude/formal/concurrency.md` — Conformance: the C's worker context as the
  implementation of C-Par / C-Arms.
- `src/c/loft_cc.h` header comment (the three modes), `CHANGELOG_TECHNICAL.md`.

## See also

- [formal/concurrency.md](../formal/concurrency.md) — C-Par, C-Det, C-Arms.
- [THREADING.md](../THREADING.md) § Nested `par` — the interpreter's worker carries its own
  `Stores` view, explicitly.
- `Stores::clone_for_light_worker`, `par_pool::parallel_workers` — the worker view the
  rustc writer's code is handed.
- [225-retire-rustc-writer/](225-retire-rustc-writer/README.md).
