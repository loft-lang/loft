<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLAN01 — Integer → i64 + safe arithmetic (C54)

## Status — DONE 2026-04-21

`integer` is i64 end-to-end across interpreter / native / WASM.
Overflow + div/mod by zero **trap** with source location (no longer
silent wrong results); `??`-discharge preserves the
`x = (a*b) ?? default` idiom by suppressing the trap when the op
is the immediate LHS of `??`.  `Type::Long` + the `long` keyword
+ the `l` literal suffix have been removed; 34 duplicate `Op*Long`
opcodes reclaimed.

Reference for the post-@PLAN01 surface lives in:

- [`../../../LOFT.md`](../../../LOFT.md) — `integer` description
  (line ~57), null sentinels per type (line ~72), arithmetic
  safety C54.G-hybrid (line ~82), `??` discharge idiom (line ~94),
  legacy-note migration guidance (line ~115), binary-file I/O
  caveat (line ~121), `!value` asymmetry (line ~129), narrow
  alias table (line ~155), `--migrate-long` CLI (line ~170).
- [`../../../INTERMEDIATE.md`](../../../INTERMEDIATE.md) —
  `Type::Integer(IntegerSpec)` enum (line ~84) with the
  Type::Long removal note, Integer Storage Size table including
  Parts::ShortRaw (line ~113), opcode-table state (line ~456,
  34 slots reclaimed by phase 5).
- [`../../CAVEATS.md`](../../../CAVEATS.md) § C54 — post-migration
  caveats (binary writers, cdylib FFI layout, memory footprint).
- [`../../QUALITY.md`](../../../QUALITY.md) § C54 — kept as design
  reference; status flipped to LANDED 2026-04-21 via @PLAN01.

This file is the closure record; phase + execution-archaeology
files in this directory remain as historical record (PHASE_2C_*,
CATEGORY_C_*, FINISH_MIGRATION, INCREMENTAL_PLAN, CODEGEN_AUDIT,
2c_migration, etc.).

### Phase outcome

| File | Phase | Outcome |
|---|---|---|
| [00-null-enforcement-audit.md](00-null-enforcement-audit.md) | Phase 0 — audit `not null` enforcement; decide G vs G′ | DONE — 7/11 holes found; **G-hybrid chosen** (trap default, null inside `??`).  G′ deferred until H1-H4 close. |
| [01-checked-arith.md](01-checked-arith.md) | Phase 1 — C54.G-hybrid: trap on bare overflow, null inside `??` | DONE — commit `925ee36`.  5 Int Nullable opcodes + `??`-context dispatch.  Long Nullable folded into Phase 5. |
| [02-i64-storage.md](02-i64-storage.md) | Phase 2 — C54.A: widen `integer` to i64 + `--migrate-i64` tool | DONE.  All three backends carry i64 integer end-to-end with narrow storage via `Parts::{Byte, Short, Int}` for alias fields. |
| [03-u32-type.md](03-u32-type.md) | Phase 3 — C54.C: `u32` stdlib type | DONE via increment 2a.  Works for values up to `u32::MAX - 1` via the wide-limit-to-Long rule. |
| [04-deprecate-long.md](04-deprecate-long.md) | Phase 4 — C54.B: remove `long` + `l` suffix; stdlib/tests/lib sweep | DONE — commits `3e976b3`..`0c46abb`.  `Type::Long` variant + keyword + literal suffix removed. |
| [05-opcode-reclamation.md](05-opcode-reclamation.md) | Phase 5 — C54.E: delete duplicate `Op*Long` arithmetic opcodes | DONE — 34 opcodes removed across rounds 10b.1–10b.4 + 10d (commits `5b2c89c`, `fd09612`, `cb0644c`, `e5a4988`, `3b34f89`).  OPERATORS table 268 → 234. |
| [06-spec.md](06-spec.md) | Phase 6 — document the new arithmetic invariant | DONE — CHANGELOG + LOFT.md (Primitive types + Null representation) + CAVEATS.md § C54. |
| [FINISH_MIGRATION.md](FINISH_MIGRATION.md) | Post-migration hardening (A / B / C / D / E) | A1/A2 (asserts + audit), B (opcode dedup), C (Type::Long + keyword removal), E (docs) all shipped.  D (persisted-DB migration tool) NOT NEEDED (no external users, no pre-2c databases). |

### Coupling discovery — Phase 2 + Phase 4 had to land atomically

Discovered 2026-04-18 during a minimal Phase 2 attempt: widening
unbounded `integer` to share representation with `long` collapsed
every `fn f(integer)` ↔ `fn f(long)` overload pair in the stdlib
into a duplicate-definition error (~50 sites: `abs`, `min`, `max`,
`round`, `sign`, …).  The stdlib sweep (Phase 4's scope) was
therefore a PREREQUISITE for the widen (Phase 2's scope), not a
follow-up.  Both shipped in a single atomic commit landing.

This pattern — "the breaking change requires the migration sweep
in the same commit" — is now memorialised in the Migration tools
ground rule and applied generally.

### Phase 5's reclamation unblocks future O1 work

With `integer` collapsed to i64, the `Op*Long` arithmetic family
became duplicate.  Phase 5 deleted them and reclaimed 34 opcode
slots.  The opcode budget is currently 254/256 used (per
INTERMEDIATE.md line 454); the reclaimed slots feed any future
O1 superinstruction peephole work that needs new opcodes.

## Follow-up holes filed by Phase 0

The audit surfaced 7 pre-existing null-enforcement gaps orthogonal
to C54.  They do NOT block any C54 phase; tracked here for future
enforcement work.  A future C54.G′ migration (null-on-overflow
everywhere) depends on H1-H4 closing.

| ID | Hole | Probes |
|---|---|---|
| H1 | `not null` field write runtime check | probes 01, 02, 03 |
| H2 | `not null` function parameter runtime check | probes 04, 05 |
| H3 | `-> T not null` return narrowing runtime check | probe 06 |
| H4 | array/hash index null/bounds runtime check | probe 09 |

Each opens its own sub-phase only when prioritised
(`07-enforcement-H1-field-writes.md`, etc.).  Probes live under
`probes/` in this directory.

## Non-goals

- **C54.F tagged-null format on small-board targets.**  Hanging
  prerequisite for G′ on 32-bit microcontrollers; loft doesn't
  target those yet.  Defer until a concrete board is picked.
- **Saturating arithmetic as a user-selectable mode.**  Explicitly
  rejected in the design (QUALITY-history.md § 561).
- **Auto-widening type system** (`i32 + i32 → i64` Python-style).
  C54.A is the capped instance; wider type-level widening is a
  separate conversation.
- **C54.D Rust-style literal suffixes** (`42i32`).  Closed-by-
  decision in [`../../DESIGN_DECISIONS.md`](../../../DESIGN_DECISIONS.md)
  § C54.D.

## Provenance

- Design captured: `doc/claude/QUALITY-history.md § 392-567` (2026-03 to
  2026-04).  Decision tree (G vs G′): QUALITY-history.md § 479-557.
- Initiative opened 2026-04-18 on branch `int_migrate`.
- Closed 2026-04-21 across the phase commits listed in the
  Phase outcome table above.

## See also

- [`../../../LOFT.md`](../../../LOFT.md) § Primitive types +
  § Null representation — user-facing reference for the post-
  @PLAN01 arithmetic semantics
- [`../../../INTERMEDIATE.md`](../../../INTERMEDIATE.md) §
  Type Enum + § Integer Storage Size — IR and storage reference
- [`../../CAVEATS.md`](../../../CAVEATS.md) § C54 — post-migration
  caveats
- [`../../QUALITY.md`](../../../QUALITY.md) § C54 — design history
- [`../../DESIGN_DECISIONS.md`](../../../DESIGN_DECISIONS.md) §
  C54.D — closed-by-decision (no Rust-style literal suffixes)
- [`../../CHANGELOG_TECHNICAL.md`](../../../CHANGELOG_TECHNICAL.md)
  — per-phase shipped manifest
- [`../02-narrow-collection-elements/`](../02-narrow-collection-elements)
  — sibling plan that addressed the narrow-vector storage gap
  surfaced post-@PLAN01
- `src/data.rs::IntegerSpec` — the named-struct carrier replacing
  the old `Type::Integer(i32, u32)` 4-tuple
- `src/database/types.rs::Parts::{Byte, Short, ShortRaw, Int, Long}`
  — the storage variants
- `tests/issues.rs` — C54 regression net (the QUALITY.md design
  list of ~30 tests, all un-ignored at plan close)
