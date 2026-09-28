# Phase 5 — C54.E: reclaim 32-bit-arithmetic opcodes

Status: **not started** — blocked by Phase 2 (the widen) and Phase 4
(stdlib sweep clears `long` references).

## What gets deleted

After Phase 2, every `integer` slot is i64 and the `Op*Long`
arithmetic family becomes duplicate.  The inventory from Part B
(QUALITY-history.md § 464-474 + exploration findings):

### Arithmetic (delete)

| Opcode | Defined in | Line |
|---|---|---|
| `OpAddLong` | `default/01_code.loft`, `src/ops.rs` | 313 (ops.rs) |
| `OpMinLong` (subtraction) | same | 323 |
| `OpMulLong` | same | 333 |
| `OpDivLong` | same | 343 |
| `OpRemLong` | same | 353 |
| `OpAddLongNn` | `src/ops.rs` | 367 |
| `OpMinLongNn` | same | 373 |
| `OpMulLongNn` | same | 379 |
| `OpDivLongNn` | same | 385 |
| `OpRemLongNn` | same | 395 |
| `OpNegLongNn` | same | 405 |

### Bitwise (delete)

| Opcode | Line |
|---|---|
| `OpLandLong` (logical AND) | 411 |
| `OpLorLong` (logical OR) | 421 |
| `OpEorLong` (exclusive OR) | 431 |
| `OpSLeftLong` (shift left) | 443 |
| `OpSRightLong` (shift right) | 458 |

### Comparison (delete)

| Opcode | Purpose |
|---|---|
| `OpEqLong` | == |
| `OpNeLong` | != |
| `OpLtLong` | < |
| `OpLeLong` | <= |

### Conversion (delete — redundant after widen)

| Opcode | Why redundant |
|---|---|
| `OpConvLongFromInt` | identity — `integer` IS i64 |
| `OpCastIntFromLong` | identity |
| `OpCastLongFromSingle` | merge into `OpCastIntFromSingle` |
| `OpCastLongFromFloat` | merge into `OpCastIntFromFloat` |
| `OpConvFloatFromLong` | merge into `OpConvFloatFromInt` |
| `OpConvBoolFromLong` | merge into `OpConvBoolFromInt` |

### Misc (delete)

| Opcode | Why |
|---|---|
| `OpAbsLong` | `OpAbsInt` works on i64 registers after Phase 2 |
| `OpNegateLong` | merge with `OpNegateInt` |
| `OpMinSingleLong` (whatever this is) | audit; likely redundant |
| `OpGetLong` | `OpGetInt` covers 8-byte reads after Phase 2 |
| `OpSetLong` | same — `OpSetInt` |
| `OpVarLong` | same — `OpVarInt` |
| `OpPutLong` | same — `OpPutInt` |
| `OpFormatLong` | merge with `OpFormatInt` |
| `OpFormatStackLong` | same |
| `OpConvLongFromNull` | merge with `OpConvIntFromNull` |

### Keep — stream payload width

`OpConstLong` (8-byte literal stream encoding) STAYS.  Stream-payload
width is a separate concern from register width; large literals
(timestamps, bitmask constants) benefit from an 8-byte encoding.
`OpConstTiny` / `OpConstShort` / `OpConstInt` also stay (Phase 2
adds the small ones).

**Total deletions: ~28 opcodes.**

Current budget: 254/256.  Phase 2 added 2 (`OpConstTiny`,
`OpConstShort`) → 256/256.  Phase 5 removes ~28 → ~228/256.  Net
reclaim: ~26 slots.

## Critical files

### `default/01_code.loft`

Each deleted opcode has:
- a `fn Op<Name>(...) -> <type>;` declaration,
- a `#rust "<body>"` implementation annotation,
- possibly an operator-dispatch entry (e.g. in the `OpLt` / `OpLe`
  overload tables for long-vs-int comparison).

Delete all three parts for each retired opcode.  Keep the
conversions needed for explicit `i32` (narrow alias) vs default
(i64) round-trips.

### `src/ops.rs`

Delete the functions listed above (lines 251–480 most of them).
Their callers in `src/fill.rs` need to stop dispatching to them.

### `src/fill.rs`

Remove the dispatch arms for each retired opcode.

### `src/native.rs`

Remove the native registry entries.

### `src/parser/operators.rs`

Today, operators like `+` dispatch to either `Op*Int` or `Op*Long`
based on operand type.  After Phase 2, both operand types are i64 →
the dispatch always picks `Op*Int`.  Simplify the dispatch table.
Reference: today's grep for `OpAddLong` shows where in
`operators.rs` (or `parser/operators.rs`) the dispatch happens.

## Pre-flight check

Before any deletion:

```bash
grep -rn "\bOp\(Add\|Min\|Mul\|Div\|Rem\|Eq\|Ne\|Lt\|Le\|Gt\|Ge\|Land\|Lor\|Eor\|SLeft\|SRight\|Abs\|Negate\|Neg\|Conv\|Cast\|Get\|Set\|Var\|Put\|Format\)Long\b" src/ default/ lib/ tests/ > /tmp/long_refs.txt
wc -l /tmp/long_refs.txt
```

Any user-code reference means either the Phase 4 stdlib sweep missed
a site or a user program needs migration.  Fix before deleting.

## Opcode renumbering

After deletion, gaps open up in the opcode numbering.  Options:

1. **Renumber** — pack retained opcodes into a contiguous range.
   Gives the cleanest budget and sets up future additions.
   Invalidates every `.loftc` cache.  Acceptable — Phase 2 already
   bumped the format, so users have rebuilt caches.
2. **Leave gaps** — retained opcodes keep their IDs.  No `.loftc`
   re-bump needed.  Budget reads "228 used, 28 gaps, 256 max."

**Decide: Renumber.**  The O1 superinstruction peephole work needs
contiguous numbering to simplify its dispatch table.  Since Phase 2
already invalidated caches, the marginal cost of a second invalidation
is near zero.

Implementation: define a mechanical renumber pass — given the retired
list, rewrite every opcode ID in `default/01_code.loft` and
`src/fill.rs`'s dispatch table.  Regression-test against the
bytecode-snapshot tests under `tests/dumps/`.

## Test plan (from QUALITY-history.md § 472-474)

Un-ignore:

| Test | Purpose |
|---|---|
| `c54e_opcode_budget_reclaimed` | Opcode count after Phase 5 ≤ 230 |
| `c54e_long_arithmetic_still_works` | Programs that went through `--migrate-long` run correctly (uses integer path now) |
| `c54e_loftc_pre_c54_invalidated` | Old `.loftc` referencing dropped opcodes fails cleanly (not silent wrong-result) |
| `c54e_no_op_long_references_remain` | `grep` confirms stdlib / lib / tests have zero `Op*Long` references |

Plus the bytecode-snapshot test suite (`tests/dumps/*.txt`)
regenerates with new opcode IDs — regression-check by manual
comparison of one or two dumps.

## Risks

1. **Missed reference to a retired opcode.**  Runtime error with
   opaque message.  Mitigation: pre-flight grep.
2. **Wrong merge of a conversion opcode.**  E.g. if `OpConvFloatFromLong`
   subtly differs from `OpConvFloatFromInt` in rounding semantics, a
   merge introduces a bug.  Mitigation: audit each merged pair before
   deletion.
3. **Bytecode-snapshot test churn.**  Every `.txt` dump under
   `tests/dumps/` shifts.  Mitigation: regenerate in bulk as part of
   the phase; manual spot-check.

## Budget

**180-240 minutes**.  Mostly mechanical, but the opcode renumbering
pass + bytecode snapshots refresh is fiddly.

Sub-phases:
- `05a-opcode-renumber-pass.md` if renumbering + snapshot regeneration
  outgrow ~200 LoC.
- `05b-conversion-opcode-merge.md` if a merged conversion op surfaces
  semantic differences.

## Deliverables

- ~28 opcodes deleted across `default/01_code.loft`, `src/ops.rs`,
  `src/fill.rs`, `src/native.rs`, `src/parser/operators.rs`.
- Opcode budget reduced from 256/256 (post-Phase-2) to ~228/256.
- Bytecode-snapshot tests regenerated.
- 4 un-ignored tests passing.
- `ROADMAP.md` § Deferred indefinitely: O1 superinstruction peephole
  moved from "blocked on opcode budget" to "unblocked".
- QUALITY.md C54.E entry: Closed.
