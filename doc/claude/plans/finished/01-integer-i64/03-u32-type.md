# Phase 3 — C54.C: add `u32` as a stdlib type

Status: **blocked — explored, deferred pending Phase 2**.

## What was attempted (2026-04-18)

Tried to land `u32` without the full Phase 2 widen.  Two minor
compiler changes landed as prerequisites:

- `src/lexer.rs::ret_number` — accepts integer literals up to `u32::MAX`
  (was: rejected anything > `i32::MAX` without the `l` suffix).  This
  lets `limit(0, 4294967294)` parse in user code.
- `default/01_code.loft` — tried `pub type u32 = integer limit(0, 4294967294) size(4);`.

## What didn't work

Values > `i32::MAX` silently wrap when stored in a 4-byte integer slot.
Probe: `x: u32 = 3000000000;` prints `-1294967296` (signed i32
interpretation of the bit pattern).  The `limit(...)` clause doesn't
enforce at write time — storage is still i32 regardless of the
declared max.

For `u32` to round-trip:

1. **Storage**: the 4-byte slot must be interpreted as *unsigned* when
   read for an `integer limit(0, X)` where `X > i32::MAX`.
2. **Arithmetic register**: widening to i64 so `u32 + u32` doesn't
   overflow at 2^31 (today traps under G; ideal would be "headroom to
   2^32").
3. **Write narrowing**: a range check at store time that rejects
   values outside `[0, u32::MAX]`.

Items 1-3 are exactly Phase 2's scope (`02-i64-storage.md`).  Without
Phase 2, `u32` is a leaky abstraction and the stdlib alias was
deliberately NOT added.

## Keeping the lexer change

The `ret_number` change is net-positive on its own — it unblocks
downstream Phase 2 work and improves error messages for large
literals ("out of range (exceeds u32::MAX)" vs the old generic
"Problem parsing integer").  Committed separately.

## Unblocks

Phase 2 (C54.A).  Once i64 register arithmetic + unsigned-read at
bounded storage lands, revisit this phase: the stdlib alias and all
5 test probes from QUALITY-history.md § 452-453 should land in an afternoon.

## Budget (post-Phase-2)

60-120 minutes.  Same estimate as before, just gated on Phase 2
being complete.

## Why this is small

Post-C54.A, every arithmetic register is i64 and integer sizes are
schema-declared.  `u32` is a one-line alias added to the stdlib:

```loft
pub type u32 = integer limit(0, 4_294_967_294) size(4);
```

The sentinel reservation (one short of 2³²) mirrors the existing
`u8 = integer limit(0, 255) size(1)` pattern — one value reserved
for null.  Users needing the full 2³² range write `u32 not null`.

`u32` closes the "RGBA pixels wrap negative" trap.  Today a pixel
coordinate or colour channel computed via `r * 256 * 256` can exceed
`i32::MAX` and land on `i32::MIN` as null; post-Phase-1 it traps;
post-Phase-3 it's simply a `u32` that holds the value.

## Design

The existing loft type-alias mechanism (`pub type T = <declaration>`)
supports this directly.  Reference: `default/01_code.loft:10`:

```loft
pub type long size(8);
```

Parallel for `u8`:

```loft
pub type u8 = integer limit(0, 255) size(1);
```

(If `u8` doesn't already exist in that form, add it as part of this
phase — it's the template for `u32`.)

## Critical files

| File | Change |
|---|---|
| `default/01_code.loft` | Add `pub type u32 = integer limit(0, 4_294_967_294) size(4);` next to the existing `u8` / `u16` / `i16` / `i32` aliases.  Audit that `u8` / `u16` exist in the expected form; add if missing. |
| `src/parser/definitions.rs` | Verify `limit(0, 4_294_967_294)` parses correctly — the upper bound exceeds `i32::MAX`.  Post-C54.A this should already work because all arithmetic is i64; but the `limit(...)` parser may still parse the bounds as i32.  If so, widen the parser's limit parsing to accept i64 bounds. |
| `default/01_code.loft` | Audit existing `integer limit(0, 255)` / `limit(-128, 127)` declarations for the size inference rule — this should NOT regress when limits widen to i64-expressible values. |
| `doc/claude/LOFT.md` | Add `u32` to the primitive-types section, next to `u8` / `u16` / `i8` / `i16` / `i32`. |

## Test plan (from QUALITY-history.md § 452-453)

Un-ignore:

| Test | Purpose |
|---|---|
| `c54c_u32_rgba_round_trip` | `r = 255; g = 128; b = 64; packed = r * 256 * 256 + g * 256 + b` round-trips as `u32` |
| `c54c_u32_arithmetic_promotes` | `u32 + u32` → i64 arithmetic register; result not truncated to u32 until stored |
| `c54c_u32_not_null_full_range` | `u32 not null` accepts value `4_294_967_295` |
| `c54c_u32_size_is_4` | `stores.types[u32_def].size == 4` |
| `c54c_u32_sentinel_value` | `u32` with value `4_294_967_295` (the reserved sentinel) reads as null, not as the max value |

Plus an optional `moros_render` probe: RGBA pixel math on a small
image renders correct values at boundary pixels (e.g. (255, 255, 255,
255)).

## Budget

**60-120 minutes**.  This is the smallest phase.  If it isn't trivial
after Phase 2, something has gone wrong — likely the `limit(...)`
parser doesn't accept bounds > `i32::MAX`.  Fix that parser gap as
part of the phase; don't sub-phase it.

## Non-goals

- `u64`.  Not today.  `u64`'s max exceeds `i64`-arithmetic capacity,
  requiring either BigInt or a separate u64-native arithmetic path.
  Out of scope for C54.
- `i64` as a type alias separate from `integer`.  Post-Phase-2 they
  are synonymous.
- Operator overloading for unsigned arithmetic.  `u32 - u32` that
  underflows traps under G (Phase 1 behaviour) — that's correct, not
  a bug.

## Deliverables

- `pub type u32 = ...` in `default/01_code.loft`.
- 5 un-ignored tests passing.
- LOFT.md type-reference section updated.
- QUALITY.md C54.C entry: Closed.
