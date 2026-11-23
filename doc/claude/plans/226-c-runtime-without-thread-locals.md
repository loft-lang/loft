<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 226 — The C runtime without thread-locals

## Status

Open, `status:next` on [loft-lang/plans#226](https://github.com/loft-lang/plans/issues/226).
Design stated by the owner; no phase started.

## Goal

The C runtime and the C it emits keep no state in thread-locals: `LOFT_TLS` and every
`static LOFT_TLS` declaration are gone, and `par`, `par_fold` and `parallel { }` answer as
they do today on every host, threads or none.

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

## Phases

| Phase | What | Validated by | Effort |
|---|---|---|---|
| **P0** | Probe: one `par` program's unit rewritten by hand with the context argument (a worker's context made as `clone_for_light_worker` makes it); the time of a call-heavy sequential program and a `par` one against today's. | The hand-written unit answers as today's under TSan; the times recorded here. | S |
| **P1** | Call-scoped scratch becomes locals of the call that uses it (JSON, reflection walks, `show`, equality, pack, upgrade). | Those headers name no `LOFT_TLS`; their suites (`json*`, `reflect*`, `store_upgrade`, `eq*`) unchanged on the C, under `par` too. | M |
| **P2** | The store context: every emitted function and store-touching runtime helper takes it first; main's is the program's, a worker's is made by the main thread before it starts (the common stores borrowed read-only, its own writable) and holds its frames and fault state. | `git grep LOFT_TLS -- src/c/loft_rt.h src/c/loft_store.h` empty; `tests/par_*.rs`, `runtime_errors` (worker faults), `lazy_sql_source` (contained faults) green on the C; TSan run of the `par` corpus clean. | L |
| **P3** | Emitted state: the work-buffer parks and slots (`body.rs`, `split.rs`) as fields of the store context; the hosted unit's bridge state on the call. | No emitted `LOFT_TLS`; `c_emit`, `hosted_unit`, `stdlib_unit` green. | M |
| **P4** | Server, web, SSH, FFI log, hints: owned by the thread that runs them. | Their suites green; `git grep LOFT_TLS -- src/c` empty. | M |
| **P5** | Remove `LOFT_TLS` itself, the C11 floor's one reason (`C_BACKEND.md`: `_Thread_local`), and "a strict C99 build runs on one thread". | `cc -std=c99 -pedantic -DLOFT_STRICT` builds a `par` unit that runs on threads; full suite and the threading CI legs green. | S |

**Order:** P0 → P1 (independent) → P2 → P3 → P4 → P5.

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
