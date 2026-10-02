<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Operators on your own types — page draft

**Status: DRAFT (P0)** for the @F catalogue entry P6 publishes.  It describes the design in
[RULES.md](RULES.md), which is not built yet.  Lookup keys: every operator token below and every
method name in the table.

---

## Operators on your own types

Your type can use `<`, `+`, `[]` and the other operators.  Write a method with the name the
table gives, and start it with `operator` instead of `fn`: it is a function where you define it
and an operator where you use it.

| You write | loft calls | Define |
|---|---|---|
| `a < b`, `a <= b`, `a > b`, `a >= b` | `a.compare(b)` | `operator compare(self: T, other: T) -> Ordering` |
| `a + b` | `a.plus(b)` | `operator plus(self: T, other: U) -> V` |
| `a - b` | `a.minus(b)` | `operator minus(self: T, other: U) -> V` |
| `a * b` | `a.times(b)` | `operator times(self: T, other: U) -> V` |
| `a += b` (and `-=`, `*=`) | `a = a.plus(b)` | nothing more |
| `x as integer`, `x as Point` | `x.to_integer()`, `x.to_point()` | `operator to_integer(self: T) -> integer` |
| `"…" as T` (into your type) | `"…".to_t()` | `operator to_t(self: text) -> T`, in `T`'s package |
| `for e in x` | `x.next()` | `operator next(self: T) -> E?` |
| `"{x}"` | `x.to_text()` | `operator to_text(self: T) -> text` |
| `"{x:spec}"` | `x.to_text(spec)` | `operator to_text(self: T, spec: text) -> text` |

```loft
pub value struct Money { cents: integer }

pub operator plus(self: Money, other: Money) -> Money { Money { cents: self.cents + other.cents } }
pub operator compare(self: Money, other: Money) -> Ordering { self.cents.compare(other.cents) }

total = Money { cents: 250 } + Money { cents: 125 };   // calls plus
cheap = total < Money { cents: 500 };                  // calls compare
```

## What defining one changes

- **Each operator calls exactly the `operator` definition in the table.**  Nothing else runs.
- **`compare` gives all four comparisons.**  `a <= b` is `a.compare(b) != Greater`; `compare` is
  called once.  `sort()`, `min_of` and every generic that needs an order use it too.
- **The left operand decides.**  `a + b` calls the `plus` of `a`'s type, picked by the type of `b`.
  `b + a` is a different call, and it is refused if `b`'s type has no matching `plus`: `3 * w`
  does not turn into `w * 3`.
- **The answer is the method's answer.**  If `minus` returns `V?`, then `a - b` is a `V?`.
  An operator never adds an error of its own.
- **`a += b` is `a = a.plus(b)`.**  There is no separate method for it.
- **Only the type's own package defines them.**  An `operator` on someone else's type, or on a
  built-in type, is refused where you write it.
- **Without `operator` it is an ordinary method.**  A `fn plus` does not make `+` work; the
  compiler's refusal of `a + b` points at it and says to add `operator`.
- **You can still call it by name**: `a.plus(b)` is the same call as `a + b`.

## What stays as it is

- **`==` and `!=`** compare the values field by field, for every type.  Two `Money` values with
  equal `cents` are equal.  `compare` answering `Equal` does not change what `==` says.
- **`!x`** asks whether `x` is present.

## Not available yet

These have their names reserved and are refused with a pointer to this page: `at` and `set_at`
(`x[i]`), `slice` (`x[a..b]`), `key_range` (a keyed slice), `power` (`**`), and `bit_and`,
`bit_or`, `bit_xor`, `bit_not`, `shift_left`, `shift_right`.

`operator equals` is refused for good: `==` compares the fields of every type, always.  A
comparison of your type's own is a named method — `same_second(self, other)` — called by name.

## When the compiler refuses an operator

`a < b` on a type with no `operator compare` says what to define, with its exact signature.  An
`operator` with a name outside the table, the wrong parameters, or on a type from another package
is refused where it is defined, with the signature the form needs.

`for e in x` and `"{x}"` need `operator next` and `operator to_text` too: a plain `fn next` or
`fn to_text` is an ordinary method, and the refusal names it.
