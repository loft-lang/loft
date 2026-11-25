<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 226 — loft without thread-locals

## Status

**DONE 2026-11-24** — closed, `status:finished`, on
[loft-lang/plans#226](https://github.com/loft-lang/plans/issues/226).  No state is a thread-local:
`git grep 'thread_local!' -- src` and `git grep LOFT_TLS -- src/c` find none, guarded by
`tests/par_contexts.rs::no_state_is_a_thread_local`.  Where the reference content lives now:

- the design, each implementation's context and where each kind of state lives —
  [THREADING.md § Each thread's own state](../THREADING.md#each-threads-own-state);
- the C runtime's store context, `struct loft_cx` — [C_BACKEND.md](../C_BACKEND.md);
- the rules `C-Own`, `C-Shared`, `C-Adopt`, `C-Publish` — [formal/concurrency.md](../formal/concurrency.md)
  § Each thread's own state;
- every lock that remains, with its reason — [LOCKS.md](../LOCKS.md).

The rest of this file is the closure record.

| Phase | State |
|---|---|
| P0 | done: measured below |
| P2a | done (f048ff16e): every C runtime and emitted function takes `loft_cx *cx` first; fixed ABIs declare theirs |
| P2b | done: the state of `loft_text.h`, `loft_store.h`, `loft_rt.h`, `par`, the FFI claim log and the hosted bridge is fields of `struct loft_cx` (`src/c/loft_cx.h`); workers', hosted units' and HTTP workers' contexts made as below; TSan clean on the `par` corpus |
| P3 | done: the parks are main-thread statics (below) |
| P1 | done: the B-tree hints, hash scratch, image checks, JSON errors and null sentinel, key seeds, the typed parse's walk and errors, the upgrade walk and the engine's message deadline are context fields too (unguarded: a header's switch is set at different points of a unit, and a field costs words) |
| P4 | done: the URL loader's state is context fields; the `net` component keeps its own context per caller context (below) |
| P5 | done: `LOFT_TLS` is gone (`git grep LOFT_TLS -- src/c` names only the TLS transport's `LOFT_TLS_UNAVAILABLE`); a strict C99 `par` unit runs on threads |
| P4b | done: `git grep 'thread_local!' -- src` is empty.  Moved: the compile passes' memos into `Data` (`PassCell`; the span memos, the definitions and the two parameter-write summaries, as `ScopedCell`; the copy manifest), a run's checks and reports into `Stores` (`checks`, `run`, `render`, `hosted_cx`), the bind notes and text yields into `State`, the C emission options into `UnitShape`; a native call's library index, bridge mark, claim log and calling interpreter into `Stores`, the interpreter's FFI callbacks finding their `Stores` in the store handle as the C does (`HOST_ABI` 8: the coroutine table's `retain` and `exhausted` take the `Stores`); the wired natives' signatures, bridge labels and stub symbols into one per-program table (`Stores::wired`), in place of two locked process tables, their per-thread caches and a generation counter; a text read's reference state into `Stores`, the hosted verify's twin depth and the stack and op censuses into `State`; the store instruments (`LOFT_ALLOC_TRACE`'s site and worker, the stack store, the bytes moved) into a per-context `store_census::Tap` each store of the context shares, a store's relocations (`LOFT_VERIFY_STACK`) and its discarded-write scratch into the `Store`; the index-walk hints into `Stores` (the C's `cx->bt_hints`); a parse's manifest errors into the `Parser`; the crash context into `Stores` (`crash_report::CrashCx`, with the spans and the store-trace names), a fatal signal reporting every running interpreter from a table of atomic slots, a panic reported with its loft position by the interpreter or compile step that catches it (`State::dispatch`, `Parser::parse_file`, `compile::byte_code`), the REPL's quiet a process choice of the CLI (`crash_report::quiet_panics`); the `#c` bindings into the program's wired natives; a debug session's output capture into the states it builds (`Stores::output`, `ReplSession::new_state`); a radix measurement's touched offsets into the read `Store`; the emulated Windows host a process choice (tests re-run in a child with the switch, `file_access::on_emulated_windows`); the engine host's kernel, client (the browser's too), swap, world and sync classes and the live rebuild into the program's `Stores` (`engine_host::EngineCx`), its frame masks from one published key and one counter; the live-reload host into the running `State` (`State::reload`); the web library's pack buffer and the browser's last message, body and headers into `Stores::web` (the C's `cx->pk_*`); the page's files into the parser's lexer (`Lexer::add_page_files`) and the run's `Stores::page_files`, a page run's print into `Stores::output`, the swap root into the engine context; the page's game and debug sessions into handles the page holds and passes to every call (`wasm_entry::Game`, `wasm_debug::Debugger`, the owner's choice of a page-API change), the staged swap snapshot into the `Game` and from there into the started program's engine context, the `par` tracer's browser lines into `Stores::par_trace`; the core layer kept free of interpreter types (`Stores::host_state` untyped, the wired natives held as `Any` and read through `extensions::wired`, the bridge labels a table of their own); the debug channel's breakpoints, subscriber, queued breakpoints and mailbox into the program's engine context (`live_control::DebugCx`), the hooks taking it; six locks removed ([LOCKS.md](../LOCKS.md)); the rustc writer's runtime, though @PLN225 R7 deletes it: a generated program's library callbacks find their `Stores` in the store handle (`native_call::handle_word`; on a 32-bit host the program's one published `Stores`), its shadow stack and lazy-driver window in `Stores::frames` (`call_frames::NativeFrames`, the emitted push, site and guards taking `cell`), its generator table and fn-ref buffers in `Stores::native` (`codegen_runtime::NativeRun`), a driver's contained fault an unwind payload of its own (`call_frames::ContainedFault`, which the CLI's panic hook stays silent for), a browser page's program published for its panic hook in one atomic word while `loft_start` runs, the live tier's parked interpreter in the program's engine context (`EngineCtl::live`, taken out for a dispatch so a breakpoint set during its pause is queued) and the flip and breakpoint hooks taking that context; `codegen_runtime::make_loft_store` and its callbacks deleted (no caller).  Verified on the writer un-refused in a scratch tree: frames, `stack_trace()`, nested generators, fn-ref record results, overflow, `assert` frames, `par`, a faulting lazy driver and a live flip print what the interpreter and the previous build print.  Found on the way, not a `thread_local!`: a library cdylib's id tables (`loft_shared_init`) are the artifact's process globals, so two programs in one process that load one artifact overwrite each other's numbering — `repl_session` under `cargo test` aborts in `loft_shared_init` (nextest runs each test alone); `C-Boundary` says the tables belong to the caller's context |

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

- `doc/claude/C_BACKEND.md` — the C11 floor and `LOFT_TLS` paragraph; the store context: done
  (the store-context paragraph).
- `doc/claude/THREADING.md` — the C runtime's worker model beside the interpreter's: done
  (§ Each thread's own state, with the plan's decisions).
- `doc/claude/formal/concurrency.md` — Conformance: the C's worker context as the
  implementation of C-Par / C-Arms: done (§ Each thread's own state).
- `src/c/loft_cc.h` header comment (the three modes): done; `CHANGELOG_TECHNICAL.md`: done
  (§ No state is a thread-local).

## See also

- [formal/concurrency.md](../formal/concurrency.md) — C-Par, C-Det, C-Arms.
- [THREADING.md](../THREADING.md) § Nested `par` — the interpreter's worker carries its own
  `Stores` view, explicitly; § Each thread's own state.
- `Stores::clone_for_light_worker`, `par_pool::parallel_workers` — the worker view the
  rustc writer's code is handed.
- [225-retire-rustc-writer/](225-retire-rustc-writer/README.md).
