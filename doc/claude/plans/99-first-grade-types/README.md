<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN99 — First-grade custom types: direct operator dispatch + custom-format hook

> **Status — DONE / SHIPPED 2026-07-08.** All arcs landed on `main` (PR #532) and are proven
> end-to-end by a real published consumer (`time 0.2.0`). User-facing reference:
> [LOFT_DATA.md § First-grade custom types](../../LOFT_DATA.md). Per-arc build detail: [STEPS.md](STEPS.md).

## What shipped

A user library `struct` behaves like a built-in across three surfaces — the general capability
that makes every wrapper type (`DateTime`, money, colour, a `Decimal`, a URL) first-grade with one
investment, no per-type core feature:

- **Arc A — direct concrete operator dispatch.** `a < b` / `a - b` / `a == b` on two concrete
  user-struct operands resolve the type's own `fn OpLt`/`OpMin`/`OpEq` (not only inside
  `<T: Ordered>`). Included the **precedence completion** — `Data::find_op_method` tries the exact
  `t_<len><Type>_Op<Name>` method BEFORE the built-in coercion / reference-identity loop, so a
  user operator is no longer shadowed. Regression `tests/scripts/511-first-grade-operators.loft`.
- **Arc B — the `{x}` / `{x:spec}` custom-format hook.** A struct's own `to_text(self, spec)`
  drives interpolation for ANY struct; the type owns its spec vocabulary; the built-in numeric
  grammar is untouched. Regression `512-first-grade-format.loft`.
- **Arc C — user conversions `x as T`.** `"lit" as T`, `var as T`, `structA as B` dispatch a user
  `fn OpConv<T>From<S>`; no match → a clean `Unknown cast` error (was a silent mis-cast).
  Regression `513-first-grade-conversions.loft`.
- **A5 — null equality on nullable struct refs.** `s == null` on an `Optional(Reference)` variable
  lowers to `OpRefIsNull` (was always-false); `??` and `== null` now agree. Regression
  `514-null-equality-struct-ref.loft`.
- **Arc D (value structs) — SHIPPED as [@PLN101](../101-value-structs/README.md)** — the zero-cost
  half (value semantics + zero heap overhead inside records / vectors).

**Acceptance:** `tests/scripts/515-datetime-first-grade.loft` (the three surfaces composing on a
real `DateTime`) landed, and the production consumer — **`loft-libs-game/time` 0.2.0**, a
first-grade `DateTime`/`Duration` value struct — is **built, both-backend tested, and published +
signed to the registry**. That is the "prove it on a real library" close condition, met.

## Known bugs the consumer surfaced (tracked separately, NOT reopening this plan)

Building `time 0.2.0` on the shipped hook exposed two defects, filed as ordinary bugs (owned by the
`../loft2` stream) and worked around in the library:

- **#533** — a **tail `if` in a `to_text(self, spec)` mis-selects its branch** (both backends, any
  struct). This is in Arc B's hook; the fix removes the workaround (bind-if-to-a-local) the lib and
  LOFT.md currently note. Per policy a shipped-feature bug is a bug-fix, not a reopened plan.
- **#534** — native codegen fails to unify `text` `if`/`else` arms mixing a `String` and a
  `&str`/match arm (E0308). A general `area:native` codegen bug, not @PLN99-specific.

## Closed design questions

- Distinct-type safety (`dt + 5` rejects) + generic-context operators were **free** (a struct is a
  distinct `Type::Reference`); Arc A added only the DIRECT path.
- `-` is `OpMin`; operators key on `(OpName, receiver type)` — no overload by the second operand.
- The format hook's home is a general language capability, now documented in LOFT.md (not a `time`
  detail).

## See also

- [LOFT_DATA.md § First-grade custom types](../../LOFT_DATA.md) — the user-facing reference (operators /
  `to_text` / `as`), including the #533 workaround note.
- [STEPS.md](STEPS.md) — the per-arc implementation detail + probes (historical record).
- [@PLN101](../101-value-structs/README.md) — Arc D (value structs, zero-cost).
- [`@PLN8`](https://github.com/loft-lang/plans/issues/8) — the DateTime library this unblocked
  (delivered as `time 0.2.0`).
- Formal: `formal/interfaces.md`, `formal/formatting.md` (0 open deviations).
