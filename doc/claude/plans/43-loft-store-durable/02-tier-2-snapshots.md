<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — Tier 2: double-buffered atomic snapshots

**Status:** Open

## Goal

Ship `DurabilityMode::SnapshotEvery(interval)`: periodic
atomic snapshots via POSIX rename, with `msync(MS_SYNC)`
discipline.  Bounded loss = one interval's worth of writes.

Tier 2 is the right choice for any data where ~5-30 sec of
loss is acceptable but full reconstruction is too expensive.
First consumer: TTT v5 multiplayer session state (turn-based
game; players don't issue moves faster than once per second
on average).

## What ships

### `DurabilityMode::SnapshotEvery` variant

```rust
pub enum DurabilityMode {
    IntegrityOnly { /* phase 01 */ },

    SnapshotEvery {
        interval: Duration,
        on_corruption: Box<dyn Fn(&Path) -> io::Result<()>>,
    },
}
```

### `lib/store_durable/` package

The Tier 2 mechanics live in a new loft package, NOT in
`src/store.rs`.  Reasons:

- Adds non-trivial dependencies (background thread or
  tick-driven snapshot loop).
- Optional opt-in for consumers who don't need it.
- Lets the implementation evolve without touching the
  core runtime.

Package layout:

```
lib/store_durable/
├── loft.toml
├── README.md
├── src/
│   └── store_durable.loft       # public API
└── native/
    ├── Cargo.toml
    └── src/
        └── lib.rs               # snapshot scheduler + msync
```

The native crate exposes:

```rust
#[no_mangle] pub extern "C" fn n_durable_snapshot_install(
    cell: &UnsafeCell<Stores>,
    store_nr: u16,
    interval_ms: u32,
);

#[no_mangle] pub extern "C" fn n_durable_snapshot_force(
    cell: &UnsafeCell<Stores>,
    store_nr: u16,
);
```

The loft side wraps these as `install_snapshot(store, interval)`
+ `force_snapshot(store)`.

### On-disk layout — two slots + checkpoint pointer

For a logical store at `/.../sessions.store`, the files are:

```
sessions.A.store         ← generation N
sessions.B.store         ← generation N+1 (in-progress)
sessions.checkpoint      ← 4 bytes: "A" or "B"
```

Snapshot cycle:

1. Read `checkpoint` → e.g., "A" → next slot is "B".
2. Memcpy in-memory store contents into `sessions.B.store`.
3. Write the tail marker (phase 00 format) + tail CRC.
4. `msync(B-fd, MS_SYNC)` → durable on disk.
5. Write "B" to `checkpoint.tmp`, `fsync`, `rename` →
   `checkpoint`.
6. Stop using "A"; "B" is the new active.

Step 5's atomic rename is the linearisation point: any
crash before rename leaves "A" still active; any crash
after leaves "B" active.  Never a torn state.

### Snapshot loop

A background tick (in the consumer's main loop or a
spawned worker) calls `force_snapshot(store)` every
`interval`.  The loop is single-threaded — only one
snapshot in flight at a time.  If the previous snapshot
hasn't finished when the next tick fires, the tick is
skipped (logged once).

For lib/server-driven daemons (the indexer, TTT v5), the
snapshot tick can piggyback on the server's accept loop:
check elapsed time after each request, snapshot if due.
No separate thread needed.

### Open path

```rust
DurabilityMode::SnapshotEvery { interval, on_corruption } => {
    let checkpoint_path = path.with_extension("checkpoint");
    let active_slot = read_checkpoint(&checkpoint_path)?;  // "A" or "B"
    let active_path = path.with_extension(format!("{active_slot}.store"));
    match Self::validate_integrity(&active_path)? {
        StoreIntegrity::Clean => Self::open(&active_path),
        StoreIntegrity::Corrupt(reason) => {
            // Fall back to the OTHER slot (it's the previous
            // generation; older but still valid).
            let other_slot = if active_slot == "A" { "B" } else { "A" };
            let other_path = path.with_extension(format!("{other_slot}.store"));
            match Self::validate_integrity(&other_path)? {
                StoreIntegrity::Clean => {
                    // Repair: point checkpoint at the good slot.
                    write_checkpoint(&checkpoint_path, other_slot)?;
                    Self::open(&other_path)
                }
                StoreIntegrity::Corrupt(_) => {
                    // Both slots bad — full rebuild.
                    on_corruption(path)?;
                    Self::open_durable(path, mode)
                }
            }
        }
    }
}
```

The "fall back to the other slot" path is the recovery
contract: at most one slot can be torn at any moment; the
other is always the previous good generation.

## Critical files

| Path | Action |
|---|---|
| `src/store.rs` | EXTEND DurabilityMode enum + open_durable arm |
| `lib/store_durable/loft.toml` | NEW package manifest |
| `lib/store_durable/src/store_durable.loft` | NEW public API (~80 lines) |
| `lib/store_durable/native/src/lib.rs` | NEW native impl (~150 lines): snapshot scheduler, msync, atomic rename |
| `lib/store_durable/README.md` | NEW package docs |
| `tests/store_durable_tier2.rs` | NEW: snapshot timing, atomicity, fallback to other slot |

## Existing functions / utilities to reuse

- Phase 01's `Store::open_durable` skeleton — extended
  with the SnapshotEvery arm.
- `lib/server`'s host-bridge pattern for the native crate
  (Stores access via `cell: &UnsafeCell<Stores>`).
- `Store::ptr` + raw memcpy for the snapshot copy (phase
  03 may add a copy-on-write optimisation).

## Test surface

`tests/store_durable_tier2.rs`:

- Open with SnapshotEvery(50ms); make 100 writes; sleep
  200ms; verify on-disk store reflects writes.
- Force snapshot mid-test; kill the process; verify
  recovery loads the snapshot.
- Manually corrupt the active slot's tail marker; restart;
  verify fallback to the other slot.
- Manually corrupt BOTH slots' tail markers; restart;
  verify on_corruption callback fires.
- Snapshot ticks while previous snapshot in flight: verify
  the tick is skipped + logged (no double-snapshot
  collision).
- Pathological case: snapshot interval shorter than
  snapshot duration → verify no fd leak, no corruption.

## Acceptance

- `cargo test --test store_durable_tier2` passes.
- `lib/store_durable/` package builds (cdylib for native,
  loft package metadata).
- Snapshot cycle on a 1 MB store: < 5 ms wall-clock per
  snapshot on SSD.
- Bounded loss verified: write 100 records, snapshot at
  50ms, write 50 more, kill at 75ms, restart → exactly
  100 records present.
- No regression for non-durable Store paths.
- TTT v5 design doc (in `plans/39-tic-tac-toe/`)
  updated to reference Tier 2 as its persistence layer
  (the consumer hookup ships in phase 05).

## Risks

| Risk | Mitigation |
|---|---|
| `msync(MS_SYNC)` semantics differ across Linux / macOS / Windows | Test matrix in phase 04 covers all three; per-OS code paths in the native crate where needed |
| Atomic rename fragility on network filesystems (NFS) | Document: Tier 2 requires local POSIX-compliant FS.  Network FS users get Tier 1 only.  Phase 06 doc records this. |
| 2× disk space for the two slots | Document.  For game-server data (typically MB-scale) this is negligible.  For multi-GB stores, Tier 3 with truncated WAL may be more efficient. |
| Snapshot tick contends with main-loop work | Single-threaded design; consumer controls when ticks happen.  No surprise contention. |
| Write-hot stores spend most of their time in snapshots | If snapshot duration > 50% of interval, log a warning + suggest tuning the interval (or upgrading to Tier 3) |

## Cross-references

- [Phase 01 — Tier 1 IntegrityOnly](01-tier-1-integrity.md) — same DurabilityMode enum
- [Phase 03 — Tier 3 WAL](03-tier-3-wal.md) — next tier up
- [`lib/server/loft.toml`](https://github.com/loft-lang/loft-libs-net/blob/main/server/loft.toml) — package layout precedent
- [Plan-32 TTT v5](../39-tic-tac-toe) — first Tier 2 consumer
