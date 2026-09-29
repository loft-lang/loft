<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN88 — the default `integer` is i64 end-to-end (close formal types.md D2)

**Issue:** [loft-lang/plans#88](https://github.com/loft-lang/plans/issues/88) ·
**Spec:** [formal/types.md D2](../formal/types.md) · [TYPING_RELATION.md § R2](../TYPING_RELATION.md) ·
**Roadmap:** [formal/ROADMAP.md](../formal/ROADMAP.md) C1.

This is the **audit** that precedes the change — the instrument that makes the whole i32-assuming
class visible before any edit (CLAUDE.md matrix-first / loft-codegen "prove it first"). It
inventories every integer-width site, classifies each, and names the keystone. Implementation
follows the audit, rung by rung, on both backends.

---

## The reframe (why this is NOT "widen `Value::Int` to i64")

types.md's D2 feared the bottom layer was *"`Value::Int` must carry i64"*. The audit shows that
would be the **wrong** change — it bloats every integer IR node to 8 bytes and throws away a
deliberate compact encoding. The actual model is already half-right:

- **The runtime is already i64.** The eval stack and every arithmetic op operate on
  `*get_stack::<i64>()` (`src/fill.rs`: `op_add_int(v1, v2)` with `v1 = *s.get_stack::<i64>()`).
  Large values already compute correctly (`3000000000 * 3 == 9000000000`, verified).
- **`Value::Int(i32)` vs `Value::Long(i64)` is a VALUE-SIZE encoding, not a type.** Both carry
  the same wide `Type::Integer`; the lexer's own words: *"the distinction is only how many bytes
  of bytecode the literal consumes."* Small values use the compact 4-byte `Int`; large values use
  `Long`. `9000000000` already works (held as `9000000000i64`, a `Long`).
- **The read side already widens.** Every consumer reads `Value::Int(n)` as `i64::from(*n)`
  (`ir_store.rs`, `parser/mod.rs`, `generation/`) — `Int` is *already* treated as a compact i64.

**So the keystone is one helper, not an IR-node widening:**

> **`int_const(v: i64) -> Value`** — returns `Value::Int(v as i32)` when `v` fits i32, else
> `Value::Long(v)`. Use it at every site that turns a *user integer value* into an IR constant.
> Small values stay compact (`Int`, 4 B); large values route to `Long` instead of truncating into
> `Int`. This is exactly "i32 in the IR, outside the i64."

The real work is in the **type model** (the `IntegerSpec` bounds + the two templates) and a
**bounded audit** of which `Value::Int` constructions carry user values vs compiler metadata.

---

## Audit — the site inventory, classified

### A. `IntegerSpec` bounds — `min: i32`, `max: u32` → **both i64** (the D2 core)

`src/data.rs:71` — `IntegerSpec { min: i32, max: u32, not_null, forced_size }`. The i32/u32
bounds **cannot represent the i64 range**, which is the entire D2 residual: "is this the full
integer?" must ride on `forced_size == None` instead of on the bounds (`signed32` max = i32::MAX,
`wide` max = u32::MAX-as-sentinel). ~**170 sites** reference `min`/`max`/`usable_*`/`forced_size`.

- **Already half-migrated (the tell):** `usable_min(..) -> i32` (data.rs:222) but
  `usable_max(..) -> i64` (data.rs:234). The max edge is i64, the min edge is i32 — widening
  `min` to i64 makes the pair consistent and is the first mechanical rung.
- **Bound encodings to retire:** `is_wide`, `is_signed32_template`, `is_wide_template`
  (data.rs:297-315) compare bound literals (`u32::MAX`, `i32::MAX`) as width sentinels; once the
  bounds are true i64 ranges these become range comparisons (`max == i64::MAX`).
- `range()` (the value count) needs **i128** — a full-i64 count overflows i64.
- **Status:** types.md layer 1 — *"mechanical, builds clean, not the obstacle."* Narrow-gated, so
  the storage casts type-check; the trap is the *silent* `as i32` (see C).

### B. The two integer templates → **unify (`signed32` → `wide`)**

`src/data.rs:33` `I32 = signed32()` (the keyword `integer`, i32 range, `forced_size: None`) vs
`data.rs:43` `I64 = wide()` (i64 range). `src/typedef.rs` `"integer" => I32`. The
`forced_size == None` guard in `is_narrowing_int` (parser/mod.rs) treated them as interchangeable;
pure range containment correctly says `wide ⊄ signed32`, so a `(I-Join)`-widened `wide` local
assigned to an `integer` destination flags *"cannot narrow integer to integer"*.

- Unify: `"integer" => wide()`; collapse `__cell_long` into `__cell_integer`
  (`src/parser/vectors.rs`); fix `is_wide`/`is_signed32_template`/`is_wide_template`; the name()
  sites (data.rs:1637/2120/4726 — `is_signed32_template() => "integer"`) re-point at the wide
  template.
- **Status:** types.md layer 2 — *"builds clean."*

### C. `Value::Int(_)` constructions — METADATA (safe) vs USER VALUE (the risk)

`Value::Int(…)` is **overloaded**. The 27 `Value::Int(<expr> as i32)` sites split into two classes:

| class | examples | i64 risk? | action |
|---|---|---|---|
| **Compiler metadata** | `Value::Int(fn_d_nr as i32)`, `Int(nr as i32)` (type/field/def number), `Int(pos.line as i32)`, `Int(return_size as i32)`, `Int(elm_size)`, `Int(d_nr as i32)`, `Int(c as i32)` (char code) | **No** — def numbers, line numbers, sizes, type tags, char codes are inherently small (fit i32). | **Leave as `Value::Int`.** These are NOT user integers; widening them would be the bloat we're avoiding. |
| **User integer value** | `const_eval.rs:118` `OpConvIntFromFloat → Value::Int(*a as i32)` (a folded `float as integer` can be large); any const-fold result; computed defaults / bounds emitted as a constant | **Yes** — can exceed i32, truncates silently. | **Route through `int_const(i64)`.** |

- **The read side is already correct** (`i64::from(*n)` everywhere) — only construction truncates.
- **The bounded part:** the user-value set is small (the const-fold results in D + a handful of
  bound/default emissions). The metadata set is the majority and stays compact — this is what
  keeps i32 efficient.

### D. `const_eval.rs` — folds in **i32 with `wrapping_*`** → fold in **i64**

`src/const_eval.rs` — 17 integer rules of the shape
`("OpMulInt", [Value::Int(a), Value::Int(b)]) => Some(Value::Int(a.wrapping_mul(*b)))`. They:

- **read** only `Value::Int` (miss `Value::Long` operands — a large operand simply isn't folded,
  falls to the i64 runtime, which is why it's not a *live* bug today: `100000 * 100000` and
  `9000000000` both print right), and
- **fold in i32 and wrap** — a small-operand / large-result fold (`100000 * 100000`) *would* wrap
  if it ever folded both i32 operands; today it's saved only by const-fold's narrow application
  (comprehension sizing, not general RHS).
- **Action:** read both `Int`/`Long` as i64, compute in i64 (the runtime semantics — trap/null on
  real i64 overflow, not wrap), emit via `int_const`. This is the const-time twin of the i64
  runtime; the `as i32` at line 118 (float→int) goes through `int_const` too.

### E. Storage ops — `min: const i16` (byte/short), `set_i32_raw` (the boundary)

`default/01_code.loft:825` `OpGetByte(v1, fld, min: const i16)` — the `min` is the **narrow
field's** declared min (a byte/short field's bound fits i16), used to offset the null sentinel
(`get_byte(rec, fld, i32::from(@min))`). For *narrow* fields this is correct and safe.

- **The truncation types.md hit** (`db.byte(-9223372036854775807, …)`) is a **wide source min
  reaching a narrow op** — a codegen path that, with the model unified, would route a wide
  integer's `min = i64::MIN+1` into a byte/short bound `as i32`. **Verify-and-gate:** the storage
  selector must never pick a narrow op for a wide-typed source; the wide source uses the 4/8-byte
  int/long path with `i64::MIN` null. This is the rung where the last attempt reverted — silent
  store corruption — so it gets a probe-matrix cell on **both backends** before it's declared
  closed. `Parts::Byte(n, b)` / `Short(n, b)` carry a field **position** `n` (i32 index), not a
  value bound — those are metadata, safe.

### F. Lexer Int-vs-Long — already correct (the model to mirror)

`src/lexer.rs:1128` `ret_number`: `r <= i32::MAX → LexItem::Integer (Value::Int)`,
`i32::MAX < r <= i64::MAX → LexItem::Long (Value::Long)`, `> i64::MAX → error`. This **is**
`int_const` for the literal path — extend the same compact-or-wide selection to const-fold and
codegen constant emission (C, D).

---

## Plan — rungs (each proven standalone on `--interpret` AND `--native` before the next)

> **Progress (formalize4) — D2 CLOSED by reconciliation; user-visible side done.** The audit
> RESPLIT the plan around [DESIGN_DECISIONS C83](../DESIGN_DECISIONS_OWNERSHIP.md#c83--the-internal-representation-follows-the-user-visible-contract-never-widen-storage-for-implementation-convenience):
> the **storage rework is OFF-PATH** (the compact `Int`/`Long` encoding is the intended design —
> never blanket-widen for convenience), so rungs **2, 3, 6 are NOT pursued**. What matters is the
> **user-visible** side: a large value a user can OBSERVE must never `as i32`-truncate.
> - **Keystone (rung 1) DONE:** `Value::int_const(v: i64)` (data.rs) — compact `Int` or wide `Long`.
> - **Construction sites (rung 5):** audited both backends. The ONE user-visible truncation found
>   was the `float as integer` const-fold — routed through `int_const` (`9e9 as integer` →
>   `9000000000`, was a clipped i32). Every other path already preserves i64 (the parser promotes
>   overflowing folds + wide locals to `Long` *before* fold_op's i32 arithmetic; `2147483647 + 1
>   == 2147483648`, no wrap). So no further construction site needs `int_const` today.
> - **Guards:** `tests/scripts/438-integer-i64-user-visible.loft` (storage round-trips) +
>   `439-integer-i64-cast-and-fold.loft` (cast + overflow-fold), both backends.
> - **types.md D2 closed by reconciliation** (the spec records the compact encoding as conformant).
>   This plan's user-visible deliverable is complete; reopen a targeted rung only if a NEW
>   user-visible truncation is found.

The original 7-rung storage-migration plan below is retained as the audit record; rungs 2/3/6
(the IR/bounds/storage-selector rework) are **declined per C83** — not the path.

1. **Keystone:** ✅ `Value::int_const(v: i64) -> Value` (data.rs) — compact `Int` if it fits i32,
   else `Long`. First use: the `float as integer` fold (const_eval.rs, audit C/D overlap).
2. **Type model (B):** `"integer" => wide()`; unify the predicates + `__cell_*`; fix the name()
   sites. *Builds clean; suite green* is the gate (no IR change yet — values still fit i32 ranges).
3. **Bounds (A):** `IntegerSpec.min`/`usable_min` → i64; `range()` → i128; retire the literal
   width-sentinels. Ripple the ~35 i32-assuming storage-boundary casts to i64.
4. **const_eval (D):** fold in i64, emit via `int_const`.
5. **User-value `Value::Int` sites (C):** route each through `int_const`; leave metadata sites.
6. **Storage selector (E):** the boundary-matrix rung — prove no wide min reaches a narrow op;
   the silent-truncation cell stays red until fixed, on both backends.
7. **Graduate** the falsifiers to `tests/scripts/` + `tests/issues.rs`; close types.md D2.

## What success looks like (the falsifiers)

- `d2_signed_narrowing_i8_to_u8_needs_cast` stays green (the D3/D5 close, unaffected).
- A `wide` local (`(I-Join)`-widened) assigned to an `integer` destination **does not** error
  "cannot narrow integer to integer" (B).
- A large value stored into a narrow field is a clean trap/null, **never** a silent `as i32`
  truncation — the cell types.md reverted on (E), green on both backends.
- The default `integer` round-trips `i64::MAX`/`i64::MIN+1` through every path; small integers
  still emit the compact `Value::Int` (size-checked — the efficiency guarantee).
