<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN82 — Constant Store — Phase B + Phase C deferred

## Status

| Phase | What | State |
|---|---|---|
| **A** — P127 fix: heap-backed `CONST_STORE`, vector constants pre-built in `byte_code()`, `OpConstRef` opcode, long strings in `CONST_STORE`, `text_code` buffer retired | Closes P127 | **SHIPPED.**  Reference for the post-Phase-A surface lives in [DATABASE_STORES.md § Constant store (`CONST_STORE`)](../../DATABASE_STORES.md#constant-store-const_store) and [INTERMEDIATE.md § Bytecode State](../../INTERMEDIATE.md#bytecode-state--srcstate). |
| **D** — `.loftc` bytecode cache: file format caches bytecode + stores + const_refs + function positions; SHA-256 cache key from source content + version; `byte_code_with_cache()` skips the `def_code()` loop on cache hit; `src/cache.rs` module | Skip stdlib re-parse | **SHIPPED then RETIRED.**  Removed in @PLAN01 (integer-i64 migration) — its cache key missed stdlib edits and there were no external users yet.  Revisit the full-bytecode-cache design if/when Phase C demands it. |
| **B** — `mmap`-backed `CONST_STORE` cache file | Zero-copy load on subsequent runs | **DEFERRED.**  Cache files are 5-10 KB — mmap overhead (syscall + page-table setup) exceeds memcpy savings at this size.  Becomes worthwhile only when Phase C embeds a large stdlib cache. |
| **C** — WASM pre-compiled stdlib (`Data` + bytecode + `CONST_STORE` as static `include_bytes!`) | Skip ~100 ms stdlib re-parse on every WASM page load | **DEFERRED.**  Requires `Data` struct serialisation across 130+ public members + recursive enums (`Value`, `Type`).  MH effort.  Trigger: contributor appetite for `Data` serialisation work, OR demonstrated need for sub-100 ms WASM cold-start past what `include_bytes!` + parse achieves. |

This plan stays in `deferred/` because Phases B + C remain.  Phase A
+ Phase D are recorded above and in their reference homes (DATABASE.md
+ INTERMEDIATE.md for A; CHANGELOG / git log for D's removal).

Trigger detail in [`../../DEFERRED.md`](../DEFERRED.md) row for
`28-const-store` (CS.B / CS.C1-C3 ROADMAP rows track Phases B + C).

---

## Phase B — Memory-mapped constant store (deferred)

### Motivation

The constant store is read-only and its contents are deterministic
— the same source program always produces the same constant data.
On systems that support memory-mapped files, the constant store can
be written to disk alongside the program and mapped on subsequent
runs, skipping reconstruction entirely.

### Platform support

| Platform | mmap available | Approach |
|---|---|---|
| Native (Linux/macOS/Windows) | Yes (`mmap` feature, `mmap-storage` crate) | File-backed `Store::open()` |
| WASM (browser) | No | Heap-backed `Store::new()` — build every run |
| WASM (WASI) | Possible (future) | Could use virtual FS |

The `mmap` feature is already default-enabled for native builds and
absent from WASM builds (`Cargo.toml:27`).  The constant store
would use the same feature gate.

### File format

The constant store file is a raw Store image — the same byte layout
that `Store::open()` already reads.  Starts with the `SIGNATURE`
(`0x53746f31` = "Sto1") header, followed by records exactly as
they appear in memory.  No versioning beyond the signature; if
the store format changes, the cache is invalidated.

### Cache invalidation

Valid only if:
1. The source file(s) haven't changed since the cache was written.
2. The loft interpreter version matches.

**Strategy:** store SHA-256 of source + version string in the first
record.  On load, compute the hash and compare; mismatched discards
the cache and rebuilds.

Alternatively use file modification timestamps (simpler, less
robust) or always rebuild and let the OS page cache handle repeated
runs (simplest; defers mmap to the bytecode cache milestone).

### Lifecycle with mmap

```text
First run:
  State::new()
    → allocate CONST_STORE as heap Store
  byte_code()
    → populate CONST_STORE with vector/string constants
    → lock store
  execute()
    → opcodes read from locked CONST_STORE
  exit
    → write CONST_STORE to disk if mmap feature enabled

Subsequent runs (cache hit):
  State::new()
    → CONST_STORE = Store::open("program.loftc")  (mmap, zero-copy)
    → verify hash
    → lock store
  byte_code()
    → skip constant building (const_ref DbRefs read from stored definitions)
  execute()
    → opcodes read from mmap'd CONST_STORE — pages faulted in on demand
```

### Implementation notes

- `Store::open()` already handles mmap-backed stores with `MmapStorage`.
- The mmap'd store is `locked = true` and has `file: Some(MmapStorage)`
  — `Drop` skips deallocation (line 103-106 in `src/store.rs`).
- `Str` pointers into the constant store remain valid because the
  mmap mapping persists for the program lifetime.
- Parallel workers clone stores for thread safety, but the constant
  store is read-only and could be shared without cloning (add
  exception to `clone_for_worker`).

---

## Phase C — WASM fast startup (deferred)

### Current WASM startup cost

```text
compile_and_run() — called on every page load / playground run
  1. Populate VirtFS with include_str! source text        ~0 ms (static data)
  2. Parse 5 stdlib files × 2 passes                      ~50-100 ms
  3. Parse graphics lib files × 2 passes                   ~20-50 ms
  4. Parse user file × 2 passes                            ~5-20 ms
  5. Scope analysis                                        ~5 ms
  6. Bytecode generation                                   ~10 ms
  7. Execute                                               variable
```

Steps 2-3 are the bottleneck: the standard library is re-parsed
from source text on every invocation.  The constant store helps
step 7 but doesn't touch the real cost.

### Fastest possible startup: pre-compiled WASM image

The ultimate optimisation is to include the compiled standard
library (Data definitions + bytecode + constant store) as static
data in the WASM binary, not the source text.  Startup would be:

```text
compile_and_run()
  1. Deserialize pre-built Data + bytecode + constants     ~1-5 ms
  2. Parse user file only (2 passes)                       ~5-20 ms
  3. Scope analysis (user code only)                       ~1 ms
  4. Bytecode generation (user code only)                  ~2 ms
  5. Execute                                               variable
```

This requires serialising `Data`, `State.bytecode`, and the constant
store into a binary format that can be `include_bytes!` into the
WASM binary.

### Why CONST_STORE is the prerequisite

With constants in a Store (a flat byte buffer), serialisation is
trivial — the Store's `ptr` buffer is already the serialised form.
Deserialising is just pointing at the static bytes.  Without the
constant store, vector constants live as IR trees in the Data
structure, which would need a complex serialiser for the recursive
`Value` enum.

### Implementation levels

| Level | What | Startup cost | Effort |
|---|---|---|---|
| 0 (current) | Re-parse everything from source | ~100-200 ms | — |
| 1 (Phase A — shipped) | Constants pre-built, still parse stdlib | ~90-180 ms | Done |
| 2 (Phase C) | Data + bytecode + constants as static binary | ~10-30 ms | MH |
| 3 (incremental) | Cache user compilation in IndexedDB | ~5-20 ms repeat | H |

Level 2 is the big win — requires the constant store as
prerequisite because the Store byte buffer is the natural
serialisation format for constant data.

### Static byte inclusion (level 2 sketch)

```rust
// Generated at build time by a build.rs script or offline tool
const STDLIB_DATA: &[u8] = include_bytes!("../generated/stdlib.bin");
const STDLIB_CONST_STORE: &[u8] = include_bytes!("../generated/constants.store");
const STDLIB_BYTECODE: &[u8] = include_bytes!("../generated/bytecode.bin");
```

The constant store bytes can be used directly as a Store buffer by
setting `Store.ptr` to the static data address (zero-copy, like
mmap).  The `borrowed` flag (A14.1) prevents `Drop` from
deallocating static memory.

### Outstanding work for Phase C

- Serialise `Data` (130+ public members, recursive `Value` /
  `Type` enums) into a binary format.  Choose between serde
  (adds dep) and hand-written (more control).  This is the
  bulk of the effort.
- Build-time script that compiles the stdlib once and emits
  the three `include_bytes!` artefacts.
- WASM startup path that branches on "static data available?"
  and skips the stdlib parse.
- Cache invalidation strategy for the static artefacts (likely
  rebuild-on-version-bump rather than runtime check, since the
  artefacts are baked into the WASM binary).

---

## See also

- [DATABASE_STORES.md § Constant store (`CONST_STORE`)](../../DATABASE_STORES.md#constant-store-const_store)
  — Phase A reference (allocation, what lives there, reference-site
  codegen, lifetime + safety)
- [INTERMEDIATE.md § Bytecode State](../../INTERMEDIATE.md#bytecode-state--srcstate)
  — `State.const_refs` and `OpConstRef` dispatch
- [PROBLEMS.md § P127](../../PROBLEMS.md) — the original bug Phase A
  closed (file-scope vector constants crashed when referenced in
  functions because IR Var(0)/Var(1) collided with caller variables)
- [DEFERRED.md](../DEFERRED.md) row `28-const-store` — trigger
  for Phases B + C
- [ROADMAP.md](../../ROADMAP.md) — CS.B / CS.C1-C3 rows that
  schedule the deferred phases
- `src/database/mod.rs::CONST_STORE` — the `pub const u16 = 1`
- `src/state/mod.rs::State::const_refs` — the per-d_nr DbRef table
- `src/store.rs::Store::open` — mmap entry point for Phase B
