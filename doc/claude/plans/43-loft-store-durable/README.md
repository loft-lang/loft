<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN43 — LOFT_STORE_DURABLE — three-tier opt-in durability for mmap stores

**Status (updated 2026-07-07):** Phases 00, 01, **and 01b all SHIPPED** to `main`.
Phase 00 + 01 (foundation + Tier 1 `IntegrityOnly`) via
[PR #219](https://github.com/loft-lang/loft/pull/219) (commit `d494edc`); Phase 01b (the
loft-callable `store_durable_check` / `store_durable_seal` binding + the
`store_durable_loft.rs` round-trip test) via [PR #220](https://github.com/loft-lang/loft/pull/220)
(commit `b307ef03`), stdlib binding refined through [PR #225](https://github.com/loft-lang/loft/pull/225).
Verified 2026-07-07 (`store_durable_loft.rs` 2/2 green, callable both backends). **Phases 02–06
stay in `future/`** until their driver consumers (TTT v5 / @PLN6 audience demo) need them —
consumer-gated, not blocked. Full promotion to active deferred until phase 02 begins; the plan
issue [loft-lang/plans#43](https://github.com/loft-lang/plans/issues/43) stays OPEN (real phases
remain).

**Substrate update (2026-06-09):** the @PLN16 debugger landed a **revertible store
change journal** — a record-change write-ahead log
([STORE_JOURNAL.md](../16-debugger/STORE_JOURNAL.md)).  For Tier 3's purposes it
*is* the WAL primitive; **Tier 3 (phase 03) builds on it rather than a parallel WAL**
— see [§ Convergence](#convergence-with-the-pln16-store-change-journal-tier-3s-substrate).
The "done safely" rationale is not re-argued here — it is homed in
[GOALS.md § Purpose](../../GOALS.md#purpose--what-loft-is-for) (the AS/400
single-level-store / "software that doesn't fail" aim).

A `Store::open_durable` API + three opt-in durability tiers
on top of loft's existing mmap-backed `Store` primitive
(`src/store.rs`).  Each consumer picks the tier that matches
its data's value:

- **Tier 1 — IntegrityOnly**: signature + checksum + tail
  marker; on corruption, fall back to caller-supplied
  rebuild.  Cheap; ideal for derived data (the indexer).
- **Tier 2 — SnapshotEvery(interval)**: double-buffered
  atomic snapshots via POSIX rename; `msync` per snapshot.
  Bounded loss = one interval's worth.  Ideal for
  turn-based game state (TTT v5 sessions).
- **Tier 3 — WAL**: write-ahead log + `fsync` per record +
  periodic checkpoints.  Zero-loss for committed writes.
  Ideal for real-time game state (@PLN6 audience demo).

## Drivers

The plan exists because three consumers face an "OS hard kill"
failure mode with very different stakes:

1. **Training-port (Python→loft) store** (`personal/training`,
   `loft-migration` branch) — the migration's Phase 2 store
   engine currently persists via a file-snapshot write→reload
   round-trip because `Store::open_durable` doesn't exist yet
   (see that branch's `MIGRATION.md` § "Loft capability gaps"
   #2).  Underlying data (Strava streams, Garmin activities,
   OSM features) is re-derivable from cached JSON, so Tier 1
   with a "re-run sync→store" rebuild callback is the right
   tier.  **This is the first real consumer; it unblocks the
   training port's persistence story and replaces today's
   file-snapshot scaffolding.**

2. **Plan-37's loft-native indexer daemon** (test bed) — can
   lose its entire `tags.store`, recover via 0.85 sec full
   rescan from the filesystem.  Gets Tier 1 essentially for
   free; just needs an integrity check so a half-written
   file doesn't crash the daemon.  Opts in when phase 08 of
   @PLN42 lands.

3. **Future game servers** (TTT v5 in `plans/future/32-…`,
   @PLN6 audience-generative-art) — hold session state,
   world state, decay timers, audience contributions that
   ARE the source of truth.  An OS-kill mid-game means
   players lose their moves; an audience-demo crash means
   the projector loses every contribution since the last
   restart.  Need Tier 2 or Tier 3 depending on tick rate.

The shared `Store` foundation makes a unified durability
plan cheaper than per-consumer ad-hoc solutions.  Each
consumer opts into the tier it needs.

The user's [evaluation prompt](../42-tracker-index/README.md)
that opened this plan: "the current task is cheap to recover,
but the eventual game server will lose valuable data in the
same event.  So this is lightly tied to the index."  The
2026-05-26 training-port re-evaluation extended the rationale:
the Python→loft port is the *immediate* driver for Tier 1 (the
indexer is still planned), so phases 00 + 01 ship first to
unblock it.

## Architecture

### Foundation — integrity at the file level

Every durable store starts with the same on-disk shape:

```
[8B signature: "DStoreV1"][2B tier-id][2B flags][4B header-CRC]
[N×8B records, each with leading record-CRC][...]
[16B tail marker: "DStoreCommit\0\0\0\0"][8B last-write-ns]
```

- **Signature**: 8 bytes; allows future format evolution.
- **Tier-id + flags**: which durability layer wrote this.
- **Header CRC**: catches corruption of the header itself.
- **Per-record CRC**: catches torn writes within the file.
- **Tail marker**: present iff the last write completed
  cleanly.  Absent → file was killed mid-write → fall back
  to recovery.
- **Last-write-ns**: timestamp of the last clean commit;
  useful for "is this snapshot fresh enough?" checks.

The existing `src/store.rs::SIGNATURE = "StoreV01"` stays as
the marker for non-durable stores; durable stores use a
different signature so a tool can tell at-a-glance which it
is.

### Tier 1 — IntegrityOnly

```rust
let store = Store::open_durable(
    path,
    DurabilityMode::IntegrityOnly { on_corruption: rebuild_callback },
)?;
```

On open:
1. Validate signature + header CRC + tail marker.
2. If anything fails → call `rebuild_callback` (the consumer
   provides this; for the indexer it's "do a full rescan").
3. If validation passes → mmap as usual.

Writes proceed exactly as today.  No `msync` discipline;
trust the OS page cache.  Worst-case loss = page cache
contents on hard power-off.

### Tier 2 — SnapshotEvery(interval)

```rust
let store = Store::open_durable(
    path,
    DurabilityMode::SnapshotEvery {
        interval: Duration::from_secs(5),
        on_corruption: rebuild_callback,
    },
)?;
```

Two slots on disk:

```
.state/
├── world.A.store
├── world.B.store
└── world.checkpoint    ← 4 bytes: "A" or "B"
```

Background thread (or main-loop tick) every `interval`:
1. Determine inactive slot (the one NOT named in `checkpoint`).
2. Memcpy in-memory store into the inactive slot's file.
3. `msync(MS_SYNC)` to force durability.
4. Write inactive slot name to `.checkpoint.tmp`,
   `fsync`, atomic `rename` → `checkpoint`.

Worst-case loss = `interval` seconds of changes.  POSIX
rename atomicity means the file system never sees a
half-committed state.

### Tier 3 — WAL

```rust
let store = Store::open_durable(
    path,
    DurabilityMode::WAL {
        wal_path: ".state/world.wal".into(),
        snapshot_every_n_writes: 1000,
        group_commit_window: Duration::from_millis(2),
        on_corruption: rebuild_callback,
    },
)?;
```

Each write:
1. Append record to `wal_path`: `[8B seq][4B record-len][record bytes][4B record-CRC]`.
2. `fsync(wal_fd)` — within `group_commit_window`, the daemon
   batches multiple writes into one fsync (latency tradeoff).
3. Apply write to in-memory store.
4. If write count since last snapshot > `snapshot_every_n_writes`:
   trigger Tier-2-style snapshot; on success, truncate WAL.

Recovery on open:
1. Load latest snapshot (Tier 2 path).
2. Replay WAL records since snapshot's seq.
3. Resume.

Per-write cost on modern SSD: 0.1-1 ms with grouped commit
(amortised across the group).  Single-fsync cost without
grouping: 1-10 ms.  Acceptable for any turn-based game; for
60Hz action games the group-commit window keeps it
bounded.

## Convergence with the @PLN16 store change journal (Tier 3's substrate)

@PLN16 built a **store change journal**
([STORE_JOURNAL.md](../16-debugger/STORE_JOURNAL.md)) — a record-change WAL that
is, for Tier 3, the same primitive.  **Tier 3 builds on it; we do not write two
WALs.**  What landed and how it maps to this plan:

- **Two artifacts, not one `wal_path`.** An **index store** (a `Store` holding one
  growing fixed-stride entry array — RAM *or* mmap, so mmap is optional and free) + a
  **blob file** (always a file; the variable-length payload).  The split keeps the
  index typed and mmappable and the payload a dumb append stream.
- **Commit ordering** — blob payload first, index entry last; recovery trusts the
  index to its last complete entry.  This is Tier 3's seq/checkpoint rule, already
  encoded.
- **Revertible — a superset of Tier 3's recovery-replay WAL.** Each entry records the
  `before` *and* `after` bytes of a `(store_nr, rec, off, len)` region, so the log
  replays **forward** (redo / recovery — Tier 3's use) *and* **backward** (undo — the
  debugger's use).  Tier 3's `[seq][len][bytes][crc]` is after-only; `before/after`
  also gives Tier 3 **mid-batch rollback** for free.
- **Determinism guard** (probe #2, confirmed): `claim` is a pure function of allocator
  state, so a replayed insert lands at its recorded position with **no `DbRef`
  remap** — the basis for whole-record replay.

**What Tier 3 (phase 03) adds on top:** `fsync` + `group_commit_window`, the
periodic checkpoint + truncate, and the on-disk **integrity** the Tier-0/1 foundation
requires — a per-entry CRC + the `DStoreV1` tier signature on the index store (the
journal's entry is `op | store_nr | rec | off | len | blob_at`; extend with a CRC
column).

**Open delta to resolve when phase 03 starts:** the journal's entry is
**per-record-change** (fine-grained, revertible); Tier 3's spec assumed a
**per-store-record write** (coarse).  The fine-grained form subsumes the coarse one
— confirm the 24-byte entry + blob append sustains 60Hz game-write throughput, or add
a coarse "whole-record" op variant for the high-rate path.

## Ground rules

- **Foundation in core, tiers in a package.**  The signature
  + checksum + tail marker live in `src/store.rs`
  (Tier-0/1 baseline).  Tier 2 + 3 machinery lives in a
  new `lib/store_durable/` package — opt-in for consumers
  who need it; doesn't bloat the runtime for those who don't.
- **Consumer chooses the tier at open-time.**  No
  per-write override.  Keeps the API small.
- **The filesystem is the recovery contract.**  Tier 1
  consumers MUST supply a rebuild callback that reconstructs
  from authoritative sources.  Tier 2 + 3 reconstruction
  is the durability layer's responsibility.
- **No transaction semantics.**  Each write is independent;
  no rollback, no isolation between concurrent writers.
  A future plan can add MVCC if it surfaces as a real need.
- **No cross-process coordination.**  Single-writer
  assumption (the daemon).  Multi-process coordination is a
  deliberate non-goal.

## Phases

| # | Phase | Effort | What ships |
|---|---|---|---|
| 0 | [Foundation: integrity + tail marker on existing Store](00-foundation.md) | XS | `src/store.rs` gains `DStoreV1` signature variant + per-record CRC scaffolding (no behavior change for non-durable stores) |
| 1 | [Tier 1: IntegrityOnly + auto-rescan hook](01-tier-1-integrity.md) | S | `Store::open_durable(.., IntegrityOnly { on_corruption })` API; integrity validation on open; corruption → callback fires; first consumer = @PLN42 indexer |
| 1b | [Loft-callable binding for `open_durable`](01b-loft-binding.md) | XS-S | Native fns `store_durable_check(path)` + `store_durable_seal(path)` exposed in stdlib so loft consumers (training port first) can use the Phase 01 API without a Rust callback wrapper |
| 1c | Path-backed hash binding for dryopea (shipped on `aid_dryopea`) | XS-S | Native fn `store_persist_bind(h: hash, path: text) -> boolean` — re-roots a hash's Store at a file path so mutations are durable via mmap.  Fresh-path branch snapshots current bytes with a padded tail-free-block; existing-path branch loads on-disk contents via `Store::open`.  Tests in `tests/store_persist_loft.rs` + `tests/scripts/store_persist_smoke.loft`.  STDLIB.md § "Path-backed hash storage". |
| 2 | [Tier 2: double-buffered snapshots](02-tier-2-snapshots.md) | M | `lib/store_durable/` package; `SnapshotEvery(interval)` mode with two-file atomic rotation + `msync` discipline |
| 3 | [Tier 3: WAL + grouped commit](03-tier-3-wal.md) | M-MH | WAL append + fsync + checkpoint + truncate; `group_commit_window` to amortise fsync cost across batches. **Builds on the @PLN16.J store change journal** (record/apply/revert already exist) — adds fsync/group-commit + checkpoint/truncate + per-entry CRC; see [§ Convergence](#convergence-with-the-pln16-store-change-journal-tier-3s-substrate) |
| 4 | [Stress test — `kill -9` × 1000 across all tiers](04-stress-test.md) | S | `tests/store_durable_kill.rs` runs an injection harness that spawns a daemon, kills it mid-write, validates recovery semantics per tier |
| 5 | [First-consumer opt-in](05-consumer-optin.md) | S | @PLN42 (tracker-index) phase 08 selects Tier 1; TTT v5 design doc updated to declare Tier 2 dependency; @PLN6 audience demo design doc references Tier 3 |
| 6 | [Closeout — DESIGN_DECISIONS + STDLIB.md + close issue](06-closeout.md) | XS | "C-… durability tier choice" decision recorded; STDLIB.md `Store::open_durable` doc; issue → `status:finished` + closed |

Total estimated effort: **H** (sum of per-phase letters above).
Sequencing: phases 0-3 are foundation+impl (must ship in
order); 4 is the cross-tier validation; 5+6 wire downstream.

## First slice — Phases 00 + 01 as one PR

Phases 00 and 01 ship together on branch
**`store-durable-phase1`** (the substantial-plan exception to
[CLAUDE.md § Branch policy rule 4](../../../../CLAUDE.md)
default of "one general working branch") and merge as a single
focused PR.

**Why bundled:** Phase 01's `Store::open_durable` API depends
directly on Phase 00's `detect_format` + `validate_integrity`
primitives — landing 00 alone exposes types nothing uses, and
landing 01 needs 00 underneath.  Combined effort is ~S+
(XS + S), which fits one PR cleanly.

**Scope (concrete deliverables):**

| Lands | File |
|---|---|
| `StoreFormat` / `StoreIntegrity` / `CorruptReason` enums | `src/store.rs` |
| `Store::detect_format`, `Store::validate_integrity` | `src/store.rs` |
| `crc32c = "0.6"` dep (fallback: ~50-line hand-rolled poly if dep conflicts) | `Cargo.toml` |
| `DurabilityMode::IntegrityOnly { on_corruption }` | `src/store.rs` |
| `Store::open_durable(path, mode)` | `src/store.rs` |
| Drop impl writes tail marker + single `msync` on clean close | `src/store.rs:180` |
| `tests/store_durable_format.rs` + `tests/store_durable_tier1.rs` | new |
| `DATABASE.md` § "Durable stores" subsection | doc |

**Out of scope for this PR** (deferred to later slices):

- Tier 2 (snapshots) — phase 02, waits for TTT v5.
- Tier 3 (WAL) — phase 03, waits for @PLN6 audience demo.
- `kill -9` stress harness — phase 04, runs across all tiers.
- Indexer phase-08 opt-in — phase 05, waits for @PLN42 to
  reach its consumer phase.  (The training port, on its own
  `loft-migration` branch, is the first opt-in instead.)
- DESIGN_DECISIONS + plan closeout — phase 06.

**Implementation gotchas the plan glosses over:**

1. **`Store::open` panics on unknown signature** today
   (`src/store.rs:297-301`).  `open_durable` must NOT route
   through that assertion for the `DStoreV1` signature.
   Cleanest fix: extract a private
   `open_with_format(path, expected_format)` helper, and have
   both `open` and `open_durable` call it with the right
   expectation.  Phase 00's `detect_format` provides the
   signature dispatch.
2. **Fresh-file path**: when the target file doesn't exist,
   Phase 01's pseudo-code fires `on_corruption(path)`.  For
   brand-new databases the callback must accept "rebuild" as
   "create empty + populate from authoritative sources."
   `STDLIB.md` doc note must make this explicit so consumers
   aren't surprised by callback semantics on first run.
3. **Drop on panic isn't guaranteed** — that's the whole point
   of Tier 1's "tail marker absent → rebuild" path, but
   `STDLIB.md` must spell out: don't use Tier 1 for data you
   can't re-derive.

**Promotion timing:** The plan stays in `plans/future/`.
Phases 00 + 01 land with `**Status:** Landed in <PR#>`; the
directory promotes to `plans/38-loft-store-durable/` only when
phase 02 (Tier 2 snapshots) starts.

## Acceptance — full plan

- `Store::open_durable(path, DurabilityMode::IntegrityOnly{..})`
  detects all of: missing signature, wrong tier-id, header CRC
  mismatch, missing tail marker → calls `on_corruption` once.
- `DurabilityMode::SnapshotEvery(5s)` writes a snapshot every
  5 sec; killing the process between snapshots loses ≤ 5 sec
  of writes; killing during a snapshot leaves the previous
  generation valid.
- `DurabilityMode::WAL{..}` recovery: every write that
  returned successfully to the caller is present after
  `kill -9 + restart` (verified by `tests/store_durable_kill.rs`).
- The training port (`personal/training`'s `loft-migration`
  branch) opts into Tier 1 as the first real consumer;
  rebuild-on-corruption successfully re-derives the store
  from cached Strava/Garmin/OSM JSON.  (Phase 01 acceptance.)
- The indexer (@PLN42 phase 08), when landed, opts into Tier 1;
  full-rescan-on-corruption succeeds in ≤ 2 sec.
- TTT v5 design doc + @PLN6 audience demo design doc cite
  Tier 2 / Tier 3 as their persistence layer.
- All 7 phases close → plan moves to
  `plans/finished/38-loft-store-durable/`.

## Risks

| Risk | Mitigation |
|---|---|
| `msync(MS_SYNC)` interaction with the kernel page cache differs across Linux / macOS / Windows | Tier 2 + 3 implementations are gated behind a feature flag per OS; test matrix in phase 04 covers all three explicitly. |
| Tier 3 fsync latency dominates throughput on slow disks | `group_commit_window` + per-tier benchmark in phase 04; document expected throughput floors so consumers can pick the right tier |
| Recovery tests are slow (each iteration spawns a process + kills it) | Phase 04's harness uses fork/exec primitives where available; runs 1000 iterations in < 2 minutes target |
| Plan-37 indexer's `tags.store` schema collides with Tier 1 layout requirements | Foundation (phase 00) freezes the layout BEFORE indexer phase 08 commits to it; coordinated via the indexer plan's design |
| Game-server plans grow needs that exceed Tier 3 (e.g., MVCC, multi-writer) | Out of scope; file as a future plan if it surfaces.  Tier 3 covers single-writer durability, which is the biggest win and matches the lib/server WebSocket-driven server architecture. |
| The foundation-in-core / tiers-in-package split adds API friction | The package re-exports the foundation types so consumers see one cohesive API.  Documented in STDLIB.md. |

## Out of scope (deferred / non-goals)

- **Multi-writer / MVCC.**  Single-writer (the daemon) is the
  loft server pattern; concurrent writers belong in a future
  plan with its own design.
- **Distributed replication.**  Not needed for any current or
  near-term loft consumer.  If a multiplayer server ever
  needs cross-machine durability, that's its own arc.
- **Encryption-at-rest.**  Defer until a security-driven
  consumer surfaces.
- **Transactional semantics across multiple stores.**
  Per-store durability only; cross-store atomicity is a
  separate plan.

## Cross-references

- [`src/store.rs`](../../../../src/store.rs) — the existing
  Store primitive this plan extends.
- `personal/training` repo (`loft-migration` branch),
  `MIGRATION.md` § "Loft capability gaps" #2 — the immediate
  Tier 1 consumer.  External to this repo; not linked.
- [`plans/42-tracker-index/`](../42-tracker-index/README.md)
  — the test-bed consumer (Tier 1, second opt-in).
- [`plans/42-tracker-index/08-multi-project-deploy.md`](../42-tracker-index/08-multi-project-deploy.md)
  — phase that opts into Tier 1.
- [`plans/39-tic-tac-toe/`](../39-tic-tac-toe) — TTT v5
  multiplayer; Tier 2 consumer.
- [`plans/6-audience-generative-art/`](../6-audience-generative-art)
  — audience demo; Tier 3 consumer.
- [`lib/server/src/server.loft`](https://github.com/loft-lang/loft-libs-net/blob/main/server/src/server.loft)
  — the server pattern these durability tiers complement.
- [DATABASE.md](../../DATABASE.md) — Stores schema + DbRef
  semantics; durable stores live within this layer.
- [plans/16-debugger/STORE_JOURNAL.md](../16-debugger/STORE_JOURNAL.md) — the
  store change journal: the revertible record-change WAL substrate Tier 3 builds on
  (§ Convergence).
