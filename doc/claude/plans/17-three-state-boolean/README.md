<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 17 — Three-state boolean (true / false / null)

**Status — DONE 2026-06-10 (both backends).**  Reference for the shipped semantics
lives in [`LOFT.md` § Null representation](../../LOFT.md) and the design decision in
[`DESIGN_DECISIONS.md` § @C73](../../DESIGN_DECISIONS.md).  This file is a closure
record; the design *history* (Stage-A characterization, the decision-B reversal, the
interpreter spike, the field-null attempt+revert) is in [SPIKE.md](SPIKE.md).
Tracked as [@PLN17](https://github.com/loft-lang/plans/issues/17).

## What shipped

`boolean` was the only common-value scalar whose zero-value collided with its null
sentinel (`null` *was* `false`).  It is now **three-state** — `false`=0, `true`=1,
`null`=255 (byte) — held and distinguished everywhere a boolean lives: locals, params,
returns, tuples, struct fields, vector/keyed-collection elements, closure captures.  A
`boolean not null` stays 2-state.  Both backends produce byte-identical results;
regression: [`tests/scripts/292-pln17-three-state-boolean.loft`](../../../../tests/scripts/292-pln17-three-state-boolean.loft).

**Semantics (design A — integer-consistent), recorded in DESIGN_DECISIONS § @C73:**
- `==` / `!=` are **raw** → `null == false` is `false`; `b == null` is the null test.
- Truthiness (`if`/`while`/`!`/`&&`/`||`) **coerces** `null → false`.
- `??` / `?? return` work (null-check is `== 255`, not truthiness) → `false ?? x` keeps `false`.
- Supersedes the **#256 guard cluster** (which *rejected* `null`/`??`/`== null` on boolean).

## How it works (one line each)

- **Storage = plain-enum byte.** A boolean field/element byte IS its `u8` form
  (0/1/255), read/written like a 2-variant enum — so it inherits enum's end-to-end
  null handling, including serialization (null field omitted; true/false rendered).
- **Interpreter:** `OpConvBoolFromNull` producer; boolean op operands read as `u8`
  (reading 255-as-`bool` is UB); truthiness ops coerce (`@v != 1`); `eq_bool`/`ne_bool`
  raw; `OpGetBoolean`/`OpSetBoolean` for field/element storage.
- **Native:** the `u8`(storage) / `bool`(expression) two-form split via `rust_type`
  (mirrors `text`'s String/Str `Context` split); `narrow_int_cast` + operand-wrap +
  `output_test_predicate` coercion + if-arm `bool_unify`; FFI/runtime helpers
  (`n_assert`/`n_set_store_lock`/`n_json_bool`/extern-decls/direct-call) coerce at the
  external-Rust `bool` boundary; `infer_type(CallRef)` resolves a fn-ref's return type.

## Phase outcomes

| Sub-arc | Outcome |
|---|---|
| A — Stage A matrix | Done — current behaviour measured (SPIKE.md) |
| B/C — representation + truthiness chokepoint | Done — both backends |
| D / G256 — `==`/`!=` raw, `== null`, `??`, retire #256 cluster | Done |
| E — native u8/bool two-form split + FFI/runtime seams | Done — the gating piece |
| Field/element null storage | Done — `OpGetBoolean`/`OpSetBoolean`, plain-enum model |
| fn-ref/closure dispatch returning boolean (native) | Done — `infer_type(CallRef)` (caught by the final verification pass) |
| F/H — serialization + docs | Done — LOFT.md + DESIGN_DECISIONS @C73 + loft-write skill |

## Headline insights (kept for the record)

- **The clean fix was net-negative.** Unifying boolean storage with plain-enum storage
  let several special-cases be *deleted* (the `OpEqInt(OpGetByte)`/`if{1}else{0}` wraps,
  the `collections.rs` two-level destructure) — robustness by subtraction.
- **The matrix only covers axes you think to vary.** "Complete" was claimed twice before
  the final from-scratch verification probe surfaced the boolean-returning-fn-ref native
  bug — no existing test exercised it.  That probe graduated into test 292.
- **Reverting beat forcing.** The field-null attempt hit a SIGSEGV from a shared codec;
  reverting (not shipping corruption) let the next pass find the blocker was a
  special-case to *delete*, not code to add.

## Known non-goal (separate, not boolean-specific)

An *omitted* field defaults to the zero value on construction (`S{}` → bool false / int 0)
but to null on `parse` — and this construction-vs-parse asymmetry affects integer too.
Left to its own investigation (binary-I/O validation, [plans/future/43](../47-binary-io-validation/README.md)).

## See also

- [`LOFT.md` § Null representation](../../LOFT.md) — the shipped semantics (reference home).
- [`DESIGN_DECISIONS.md` § @C73](../../DESIGN_DECISIONS.md) — the design decision; § @C69 (adjacent, `!x` on non-boolean).
- [SPIKE.md](SPIKE.md) — design history (characterization, reversal, spike, field-null attempt).
