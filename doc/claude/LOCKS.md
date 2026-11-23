<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Locks

Every lock loft keeps, why it is needed, and what would replace it.

The rule is `C-Publish` ([formal/concurrency.md](formal/concurrency.md) § Each thread's own
state): data more than one thread can write is, in order of preference,

1. **owned by a context** — the C runtime's `loft_cx`, the interpreter's `Stores` or `State`, a
   compile pass's own state — so only one thread ever touches it (`C-Own`);
2. **set before the first worker starts** and read-only afterwards;
3. **built per thread and merged after the join** (`C-Adopt`) — a log, a tally, a de-duplicated
   report;
4. **one atomic word** — a flag, a counter, a claim, a slot taken by compare-and-swap;
5. **a lock**, only where none of the above holds — and then it is listed here, with its reason.

A lock that sits here with "not needed" is owed: it is removed by moving its data to one of the
forms above.  A one-time publish (`OnceLock`, `LazyLock` of an immutable value, `env_once!`, the
C runtime's `loft_once_begin`) is not a lock: the one thread that sets it is the only one that
can wait, once per program.

Columns: **Threads** — who can touch it at the same time (*arms* are `parallel { }` arms; *test
threads* are several programs compiled or run in one process: `cargo test`, `loft test`).
**Instead** — the lock-free form and its cost.  **Visible** — what a user would notice.

## C runtime

| Lock | Guards | Threads | Why | Instead | Visible |
|---|---|---|---|---|---|
| `loft_pub_lock` in `loft_io.h` `loft_log_refusal_once` | the refused paths already reported | main, workers, arms | the port of Rust's `file_access::log_once` (`SAID`): one line per path per program | a set per context, de-duplicated when merged at the join | the line appears after the join; or once per worker without the merge |
| `loft_pub_lock` in `loft_process.h` (replay) | how often each recorded command has been replayed | main, arms | the port of Rust's `process_run` `USED`: a replay walks its recordings in order across the program | read the counts before workers start; per-context counts would replay an entry twice | — (kept) |
| `loft_pub_lock` fallback (`loft_pub_srw`, `loft_pub_mutex` in `loft_cc.h`) | every `LOFT_PUB_*` word and `loft_once_begin` | main, workers | only on a threaded build whose compiler has no atomics (not gcc, clang or MSVC — `LOFT_STRICT` included) | C11 `<stdatomic.h>` where present | none |
| `LOFT_EH_LOCK` (`loft_engine.h`) | the finished outbound HTTP requests | request threads, the host's pump | genuinely cross-thread: a worker parks its answer, the pump drains it | an atomic slot per request id scanned by the pump | none |

Everything else the C runtime shares is one atomic word (`LOFT_PUB_GET/SET/SWAP/ADD/CAS`): the
halt claim, the fatal flag, the process-group table the signal handler reads, the counters, the
lazily read switches and resolved symbols — or a one-time publish (`loft_once_begin`): the fault
log, the store dump and the allocation trace opened on first use, Winsock's start, the signal
handlers' install.

## Interpreter runtime

| Lock | Guards | Threads | Why | Instead | Visible |
|---|---|---|---|---|---|
| `Stores.logger` (`database/mod.rs`) | the run's logger: file, rotation, rate limits | main, workers, arms | one file, one rotation, one rate limit for every thread | **a logger per context buffering its records; the starter appends a worker's buffer after the join and alone writes the file** — more memory, a rate map per worker | worker lines arrive after the join, grouped per worker; a per-site rate limit becomes per worker; a crash mid-`par` must flush the buffers |
| `PRODUCTION_LOGGER` (`runtime_error.rs`) | the production logger a store fault finds | any thread at a locked-store write | a global stands in for the context | a `production` flag on the store, the log through the context (`C-Boundary`) | none; also stops test threads overwriting each other's |
| `REPORTING` (`runtime_error.rs`) | report a halting fault once, then exit | workers | a second worker must not print or exit mid-report | one atomic swap, as the C's `loft_halt_claimed` | none |
| `WORKER_FATAL` (`parallel.rs`) | the first worker halt, handed to the parent | workers → parent | carries an error across threads | each worker returns its error with its result; the parent raises the lowest index after the join | the reported halt is the lowest worker index, not the first in time — deterministic |
| `WorkerTrace` (`par_pool.rs`, trace only) | the pool indices a dispatch used | workers | a set filled from several threads | an atomic bit per pool index | none |
| `HostInputPump` state and condvar (`database/format.rs`) | stdin bytes a background reader drains | the reader, main, arms | stdin is one per process | a channel of chunks | none — program-wide by nature |
| `CONNECTIONS` (`database/sql_source.rs`) | the open SQLite handle per target | any thread that misses a lazy store | reuse across the program | a connection per `Stores`, opened before workers start | separate snapshots unless opened before the start |
| `QUERIES` (`database/sql_source.rs`) | queries per target (a test measure) | same | a shared count | per context, summed at the join | none |
| `STARTED`, `CURSORS` (`process_run.rs`) | started children and their line cursors | main, arms | a handle is a value an arm can read and wait on | handles owned per context, the table read-only while arms run — which would refuse `wait` from an arm | that refusal |
| `USED` (`process_run.rs`) | replay counts (the C row's twin) | main, arms | see the C row | see the C row | — |
| `KEPT` (`platform_process.rs`) | dropped children still running, for the orphan watcher | droppers, the watcher thread | a hand-off between threads | a channel to the watcher, which owns the list | none |
| `PIPE` (`platform_process.rs`) | stdin of the watcher that kills owned process groups | whichever thread starts or ends a child | one watcher per process | — | — (program-wide by nature) |
| `SAID` (`file_access`) | refused paths logged once per process | any thread resolving a path | de-duplication across threads | a set per context merged at the join | as the C row |
| `BY_TYPE`, `NAMES`, `BY_SITE`, `PEAK_SNAPSHOT` (`store_budget.rs`) | live bytes per type and site for the ceiling's refusal | every allocating thread — locked on **every** add, release, grow and shrink while a ceiling is set, and test runs set one | the refusal names what holds the memory | a tally per `Stores` adopted at the join, or a walk of the live stores when the refusal is built; names from `Stores::types` | the breakdown may omit workers still running |
| `LAZY_FETCH`, `LAZY_FETCH_REFUSAL` (`codegen_runtime.rs`) | lazy drivers of the rustc writer's programs | set by `init` before any thread | **not needed** | `OnceLock`, or gone with the writer (@PLN225) | none |

## Library loading and the FFI

| Lock | Guards | Threads | Why | Instead | Visible |
|---|---|---|---|---|---|
| `C_BINDINGS` (`c_call.rs`) | library index → `#c` binding — **locked and cloned on every interpreted `#c` call** | set before the run; read by every thread | a program's table in a process global | a `State` table set before the run, read-only after (`C-Boundary`) | none |
| `NATIVE_SIGS`, `SHARED_SIGS` (`extensions.rs`) | library index → bridge and signature | set at wiring; read through a thread-local copy | a program's table in a process global; the thread-local cache and a generation counter cover several programs per process | the table in `State`, read-only during the run — no lock, no thread-local, no generation | none |
| `SHARED_LABELS` (`call_frames.rs`), `STUB_SYMBOLS` (`compile.rs`) | names a fault or a missing native reports | set at wiring; read on a fault | same | `State` tables | none |
| `DECLARED_LIBS`, `LIB_SYMBOLS` (`c_libs.rs`) | the program's C libraries and their symbols | set at `register`; read on a miss | same; test threads replace each other's | `Data`/`State`, fixed before the run; the lazily tried flag as one atomic per entry | none |
| `NATIVE_REGISTRY`, `BRIDGE_REGISTRY`, `CX_BRIDGES`, `LOADED_LIBS`, `REFUSED_LIBS`, `LOAD_LOCK` (`c_libs.rs`) | what `dlopen` loaded and registered | loaders, wiring | a library is opened once per process | an append-only table published once per entry; `LOAD_LOCK` stays — an open must not run twice | — |
| `DEP_HEADERS` (`c_shim.rs`) | dependency headers per package for shim builds | one compile thread per program | a program's data in a process global | a `Data` field | none |
| `PLACEMENT` (`lib_placement/dispatch.rs`) | placed-library workers; **held across the whole call** | main, workers, arms | the wire has one request slot per placed worker | a slot or a worker per caller context (`C-Boundary`) | a placed library's own global state becomes one per caller context; its calls run side by side |

## Compiler and passes

| Lock | Guards | Threads | Why | Instead | Visible |
|---|---|---|---|---|---|
| `PassCell` (`data.rs`: range, non-null, copy-manifest, catch-up, trace, depth), `ScopedCell` (`data.rs`: the definitions, ownership-span, parameter-write and written-parameter memos), `LazyDriverCache`, `OpSetCache`, `NameOnlyCache` (`data.rs`) | memos and scratch about one program | the compiling thread; `Data` is shared read-only with the interpreter's `par` workers | only so `Data` stays shareable — never contended | a compile-session object the passes take by `&mut`, holding these memos (`C-Own`) | none |
| `FOUND` (`spent.rs`), `WHERE` and the counts (`rewrite_census.rs`), `SEEN` (`timeout.rs` give-up reports) | per-program compile state | the compiling thread; test threads race on them | process globals for a program's state | the compile session; census totals merged into a process total at exit | none; stops cross-talk between test threads |
| `NAMES` (`lexer.rs` `intern_file`), `POOL` (`diagnostics.rs`), `SEEN` (`ir_read.rs`) | interned file names, codes and block names as `&'static str` | every compile and the runtime | `'static` names need a process table | a per-`Data` interner (names live as long as the program) — a large change | none |
| `CACHE` (`registry_index.rs`), `INDEX_MEMO` (`install.rs`), `CACHE` (`native_utils.rs`) | parsed registry index; file hashes | compile and build threads, test threads | genuinely shared caches (a parse or a 14 MB hash once per process) | per compile session, paid once per program | none |

## Diagnostics, logs and trace sinks

All are off unless a `LOFT_*` switch arms them.

| Lock | Guards | Threads | Why | Instead | Visible |
|---|---|---|---|---|---|
| `SINK` (`fault_log.rs`), `SINK` (`native.rs` assert trace), `FILE` (`store/dump.rs`), `FILE` (`store_census.rs` alloc trace), `SINKS` (`bind_lowering.rs`), `FILE` (`state/codegen.rs`), `SINK` (`lexer.rs` tokens) | a trace or log file | any thread that writes a line | whole lines | one `write` per line to an `O_APPEND` file; or per context, flushed at the join | none with `O_APPEND`; with buffers, worker lines arrive later |
| `PROGRAM_LINES` (`store_census.rs`) | the line table workers read | main writes, workers read at start | hands data to workers | the worker's context, filled before it starts | none |
| `VERIFY` (`stack_verify.rs`), `TEXT_TL` (`state/text.rs`), `CALLS`, `SEEN` (`state/hosted_verify.rs`), `SITES` (`net_profile.rs`) | process-wide tallies and ledgers | main, workers | one verdict for all threads | per context, adopted at the join and printed by the main thread | findings grouped after the join |
| `Timing` (`platform.rs`) | build timing events | build threads | a shared summary | per build context | none |

## Engine, hosts and tooling

| Lock | Guards | Threads | Why | Instead | Visible |
|---|---|---|---|---|---|
| `HTTP_DONE` (`engine_host.rs`) | finished HTTP requests (the C `LOFT_EH_LOCK`'s twin) | request threads, the pump | cross-thread delivery | a channel whose receiver the host owns | none |
| `Handoff` (`wasm_threads.rs`) | rayon builders waiting for Web Workers | the page's main thread, workers | start-up hand-off | slots indexed by an atomic counter | none |
| `GameProc.output` (`repl.rs`) | a started game's output | two drain threads, the REPL | two producers | a channel of lines | none |
| `BREADCRUMB`, `INTERP_ENTRY`, `STOP_FLAG`, `BLOCKED` (`timeout.rs`) | what the watchdog reports and the loop it stops | main, the watchdog | values of several words read across threads | an atomic per field (interned strings as pointers) | fields may come from two adjacent checkpoints |
| `first_failure` (`c_shim.rs`, scoped) | the first failed shim compile | scoped compile workers | several workers report | each worker returns its failures; the minimum after the join | none |

## Open work

The locks marked **not needed**, and the hot ones, go first: `C_BINDINGS` (every `#c` call),
`store_budget` (every allocation under a ceiling), the wiring tables (`NATIVE_SIGS` with its
thread-local cache).  Then the
compile session that takes the `Data` memos (`PassCell`, `ScopedCell`) and the per-program
globals of the compiler.  The logger and the tallies follow the per-thread-then-merged form once
the order of their lines is agreed.  @PLN226 P4b carries them.
