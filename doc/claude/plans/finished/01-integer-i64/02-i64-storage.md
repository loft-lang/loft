# Phase 2 — C54.A: widen `integer` to i64 + range-packed storage

Status: **not started** — blocked by Phase 1 **AND tightly coupled with Phase 4**.

## Coupling discovered (2026-04-18 attempt)

A minimal approach — "redirect unbounded `integer` in `parse_type` to
return `Type::Long` so arithmetic picks up i64 registers" — was
attempted.  It compiles but fails to load the stdlib because
every `integer` / `long` overload pair in `default/01_code.loft`
collides:

```
pub fn abs(both: integer) -> integer { ... }   // line 145
pub fn abs(both: long)    -> long    { ... }   // line 213
```

Under the redirect, both signatures become `abs(long) -> long` and
the parser rejects the duplicate.

Same collision exists for every function overloaded between
`integer` and `long` — `min`, `max`, `sign`, `round`, `floor`,
`ceil`, etc.  ~50 `long`-specific stdlib declarations.

**Implication**: Phase 2 and Phase 4 (deprecate `long`) MUST land
together, OR the stdlib needs a transitional layer where `long`
functions stay for now but become no-ops that forward to their
`integer` counterparts.  Either way, `long` overloads get removed
as part of the Phase 2 landing.

Estimate revised: Phase 2 + Phase 4 combined ≈ 800-1000 minutes.

## Revised landing sequence

Single atomic session covering:

1. Widen `Type::Integer` representation OR redirect unbounded
   `integer` to `Type::Long` (pick one).
2. Sweep stdlib: remove all `long`-specific function overloads
   (their `integer` twins absorb them).
3. Update `pub type integer size(4)` → `size(8)`.
4. Run regen_fill_rs to update bytecode handlers.
5. Verify: interpreter + native + WASM parity, full workspace
   suite, `c54_*` tests un-ignored.
6. `.loftc` auto-invalidates via `CARGO_PKG_VERSION` bump.
7. Migration tool shipped on the same commit.

Sub-phases `02a-migration-tool-design.md` and `02b-stdlib-sweep.md`
open once the atomic session starts.

## What changes for the user

| Today | After Phase 2 |
|---|---|
| `integer` = i32 storage + i32 arithmetic | `integer` = **i64 storage** + i64 arithmetic |
| `i32` = explicit alias for the old default | `i32` = explicit narrow alias (4-byte storage, i64 arithmetic register) |
| `integer limit(0, 255)` = 1-byte storage | unchanged — 1-byte storage, widens to i64 on read |
| `long` / `10l` = i64 | still works; becomes a no-op alias.  Phase 4 deprecates. |
| Timestamp / checksum math overflows silently | headroom up to 2^63 |

Arithmetic behaviour does NOT change — Phase 1 already unified
overflow semantics.  Phase 2's value is pure **headroom + storage
optimisation for common small-magnitude literals**.

## Design

### Decouple arithmetic width from storage width

- **Arithmetic always i64.**  `OpAddInt`, `OpMulInt`, … operate on
  i64 registers.  `i32::MIN` stops being the arithmetic sentinel;
  `i64::MIN` replaces it.
- **Storage default for unbounded `integer` is 8 bytes** (i64).
  Bounded fields (`limit(lo, hi)`, `i8`, `u8`, `i16`, `u16`) keep
  their bit-packed storage.
- **Load widens, store narrows.**  Reading `i8` / `u8` / `i16` /
  `u16` / `integer limit(...)` sign- or zero-extends to i64.
  Writing narrows back with the existing `limit` range check.

### Bytecode-constant family (width-graded by magnitude)

Per QUALITY-history.md § 421-432:

| Opcode | Stream bytes | Range | Use case |
|---|---|---|---|
| `OpConstTiny` | 1 | −128 ..= 127 | `x = 0`, `x = 1`, loop bounds, indices |
| `OpConstShort` | 2 | −32 768 ..= 32 767 | small literals |
| `OpConstInt` | 4 | −2³¹ ..= 2³¹ − 1 | existing 4-byte literals |
| `OpConstLong` | 8 | full i64 | timestamps, large constants |

Each sign-extends into an i64 register on load.  The common case
(`x = 0`, `for i in 0..n`, `v[i]`) stores 1 byte after opcode —
~50 % bytecode-size saving on integer-heavy programs.

`OpConstTiny` and `OpConstShort` are NEW opcodes.  `OpConstInt`
already exists.  `OpConstLong` exists (`default/01_code.loft` line
185).  Keep all four.

## Critical files

### Register width + type surface

| File | Change |
|---|---|
| `src/data.rs:32` | `pub static I32: Type = Type::Integer(i32::MIN+1, i32::MAX as u32, false);` — widen the default `integer` type to the i64 range.  Rename this static to `I64` or add a new `I64` + leave `I32` for explicit `i32` alias. |
| `src/data.rs:172` | `Type::Integer(i32, u32, bool)` — extend min/max fields to i64 OR add a new `Type::Integer64(i64, u64, bool)` variant and route unbounded integers there.  **Second option is safer**: keep `Type::Integer` as the 4-byte-max-storage bounded variant, introduce `Integer64` as the unbounded default.  Decide in implementation. |
| `src/data.rs:529` | `Type::Integer(_, _, _) ... => 4` — default arm for unbounded → 8.  Depends on which struct is chosen above. |
| `src/data.rs:1846-1858` | Rust type selection for native codegen — adjust default arm to emit `i64` for unbounded. |

### Storage read/write

| File | Change |
|---|---|
| `src/store.rs:1115` | `get_int(&self, rec, fld) -> i32` → `get_int(&self, rec, fld) -> i64`.  Width of the stored slot stays 4 bytes for bounded, widens to 8 bytes for unbounded.  Read code differentiates by field metadata. |
| `src/store.rs:1124` | `set_int(&mut self, rec, fld, val: i32) -> bool` → `set_int(... val: i64) -> bool`.  Store narrows with range check; error if value doesn't fit bounded width. |
| `src/store.rs:1153` | `get_short` — signature stays, still reads u16, result widens into i64. |
| `src/store.rs:1167` | `set_short` — accept i64, narrow to u16 with range check. |
| `src/store.rs:1184` | `get_byte` — same pattern, u8 → i64. |
| `src/store.rs:1194` | `set_byte` — same pattern, i64 → u8 with range check. |
| `src/store.rs` | **New**: `get_int64`, `set_int64` for the 8-byte unbounded path.  OR collapse `get_int` into the 8-byte version and have bounded fields use `get_byte` / `get_short`. |

### Arithmetic handlers

| File | Change |
|---|---|
| `src/ops.rs:531-571` | `op_{add,sub,mul,div,rem}_int(v1: i32, v2: i32) -> i32` → `op_*_int64(v1: i64, v2: i64) -> i64`.  Given Phase 1 already routes through `checked_*`, the edit is a type widening + macro parameter update. |
| `src/ops.rs:313-405` | `op_*_long` family merges into the i64 path.  They're currently distinct only because the interpreter distinguished `integer` (i32) from `long` (i64) registers.  After Phase 2 both are i64 → duplication.  Phase 5 deletes the `_long` siblings. |
| `src/fill.rs:477-656` | Handler dispatch — each `OpAddInt` / `OpMulInt` / etc. swaps its `i32` pop/push for `i64` pop/push.  The stack representation becomes 8 bytes per integer slot (or stays 4 for bounded — stack slots are width-typed). |

### Bytecode constants

| File | Change |
|---|---|
| `src/fill.rs` | Add handlers for `OpConstTiny` (reads 1 byte, sign-extends to i64) and `OpConstShort` (reads 2 bytes, sign-extends).  `OpConstInt` already reads 4 bytes; promote its result to i64 on load. |
| `src/state/codegen.rs` | In the literal-emit path, pick the narrowest opcode by magnitude: `value in [-128..128)` → `OpConstTiny`; `in [-32768..32768)` → `OpConstShort`; `in i32 range` → `OpConstInt`; else `OpConstLong`. |
| `default/01_code.loft` | Add `OpConstTiny` and `OpConstShort` operator definitions.  Add their native-registry entries. |
| `src/native.rs` | Register native handlers for the new ops. |

### `.loftc` cache

`src/cache.rs:13-16` — magic `b"LFC1"` + SHA-256 of
`VERSION + BUILD_ID + source`.  No explicit format version field.

- **Option A — rely on version bump.**  Bumping `CARGO_PKG_VERSION`
  (e.g. to 0.9.0) changes the SHA-256 key; old caches fail-closed.
  Simplest.  Has the side effect of invalidating ALL caches across
  the codebase, not just integer-storage-sensitive ones.  Acceptable.
- **Option B — introduce magic `LFC2` + explicit format version.**
  More work, lets us keep `.loftc` for files that didn't touch
  integer storage.  Not worth it for a single release bump.

**Decide: Option A.**  Bump version, invalidate all caches.

### Persisted-database migration

If any persisted database stored unbounded `integer` as 4-byte values,
those DBs need migration.  Ship `loft --migrate-i64 <dbfile>`:

```
Read the database's schema block.
For each column declared `integer` (unbounded):
  - Read the 4-byte value.
  - Write to a new column with 8-byte storage.
  - If the old value was `i32::MIN` (null sentinel), write `i64::MIN`
    (new sentinel).
Rewrite the schema block with widened column widths.
Atomic rename.
```

Conservative dry-run flag.  Refuses to migrate if the schema looks
malformed.

Critical files for the migrator:
- `src/database/structures.rs` — schema read/write.
- New: `src/migrate_i64.rs` — the migration logic.
- `src/main.rs` — CLI `--migrate-i64 <path>` handler.

## Test plan (from QUALITY-history.md § 434-440)

Un-ignore and make pass on all three backends:

| Test | Purpose |
|---|---|
| `c54_i32_min_round_trip` | `-2_147_483_648 * 1` returns `-2_147_483_648`, not null |
| `c54_arithmetic_at_boundary` | boundary values (MAX, MAX-1, -1, 0, 1, MIN+1, MIN) round-trip |
| `c54_bounded_storage_preserved` | `integer limit(0, 255)` still 1 byte in the store |
| `c54_unbounded_storage_widens` | `integer` unbounded reads 8 bytes in the store |
| `c54_u8_times_u8_no_overflow` | `u8 * u8` in i64 register; result up to 65025 fits |
| `c54_loftc_cache_invalidated` | old `.loftc` from a pre-C54 build fails to load cleanly |
| `c54_migration_tool_roundtrip` | persisted DB with a `i32::MIN`-null column round-trips through migrator |
| `c54a_const_tiny_used_for_small_literals` | `x = 0` emits `OpConstTiny 0`, 1 byte after opcode |
| `c54a_const_short_used_for_small_literals` | `x = 1000` emits `OpConstShort 1000`, 2 bytes |
| `c54a_bytecode_size_savings_integer_heavy` | emit a loop with integer-heavy math; compare bytecode-length pre- and post-phase; assert ≥30 % reduction |

Plus cross-backend parity tests for every boundary value.

## Risks

1. **Opcode-count explosion.**  Adding `OpConstTiny` + `OpConstShort`
   while keeping `OpConstInt` + `OpConstLong` adds 2 opcodes.
   Current budget: 254 / 256.  This pushes to 256 / 256.  Phase 5
   reclaims 26 slots by deleting duplicate `Op*Long` — that happens
   AFTER Phase 4 deprecates `long`.  Phase 2 will briefly run at
   255-256 used.
2. **Stack-slot width.**  If stack slots are currently 4 bytes,
   widening to 8 bytes per integer slot doubles stack usage for
   integer-heavy code.  Measure before committing.  If it's a
   problem, introduce variable-width slots (already hinted at by
   the `OpVarInt` / `OpVarLong` split).
3. **Migration tool edge cases.**  Schema evolution, nullable
   columns, nested structs containing integer fields.  The
   migration tool needs to walk the full schema graph.  Budget
   separately if it outgrows ~300 LoC.
4. **WASM parity.**  WASM's i64 support is first-class
   (`wasm32-wasip2`).  Should be near-trivial.  Verify with a WASM
   boundary test.

## Budget

**480-720 minutes** across the phase.

Sub-phases that may open:
- `02a-migration-tool-design.md` if the migration tool exceeds
  ~300 LoC.
- `02b-fill-rs-i64-parity.md` if the `src/fill.rs` handler rewrite
  surfaces per-handler parity bugs.
- `02c-const-stream-encoding.md` if `OpConstTiny` / `OpConstShort`
  integration with the codegen emitter needs its own design.

## Deliverables

- All `c54_*` (non-G) tests un-ignored and passing.
- Migration tool committed with round-trip fixture.
- `.loftc` version bumped (via `CARGO_PKG_VERSION` bump).
- QUALITY.md C54.A entry: Closed.
- `RELEASE.md` 0.9.0 gate entry: C54.A complete.
