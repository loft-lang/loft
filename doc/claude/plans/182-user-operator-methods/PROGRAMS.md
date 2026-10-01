<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN182 P0 — four programs in the proposed spelling

**Status: on paper (P0).**  Written against [RULES.md](RULES.md), which none of this compiles
against yet.  Each use is tagged with the ONE rule that gives it meaning; the falsification is a
use with no rule, a use with two, or a `time` operation that reads worse than today.

## 1. `time` — the required adopter

Today (loft-libs-game `time` 0.3.2): 20 `fn Op…` on `DateTime` and `Duration`, plus
`minus(self: DateTime, span: Duration)` and `negate(self: Duration)`.  Proposed:

```loft
pub value struct DateTime { ms: integer }
pub value struct Duration { ms: integer }

// order — one method per type replaces OpLt/OpLe/OpGt/OpGe          (Op-Order)
pub operator compare(self: DateTime, other: DateTime) -> Ordering { self.ms.compare(other.ms) }
pub operator compare(self: Duration, other: Duration) -> Ordering { self.ms.compare(other.ms) }

// equality — OpEq/OpNe compared `ms`, the only field: structural `==` answers the same,
// so both go and nothing replaces them                                  (Op-Eq)

// arithmetic                                                           (Op-Back, Op-Left)
pub operator minus(self: DateTime, other: DateTime) -> Duration { Duration { ms: self.ms - other.ms } }
pub operator minus(self: DateTime, span: Duration) -> DateTime { DateTime { ms: self.ms - span.ms } }  // exists today
pub operator plus(self: DateTime, span: Duration) -> DateTime { DateTime { ms: self.ms + span.ms } }
pub operator plus(self: Duration, other: Duration) -> Duration { Duration { ms: self.ms + other.ms } }
pub operator minus(self: Duration, other: Duration) -> Duration { Duration { ms: self.ms - other.ms } }
pub operator times(self: Duration, factor: integer) -> Duration { Duration { ms: self.ms * factor } }
pub operator negate(self: Duration) -> Duration { Duration { ms: 0 - self.ms } }                       // exists today

// conversions — today OpConvIntFromDateTime / OpConvDateTimeFromInt / OpConvDateTimeFromText (Op-Conv)
pub operator to_integer(self: DateTime) -> integer { self.ms }
pub operator to_date_time(self: integer) -> DateTime { DateTime { ms: self } }          // foreign self, own target
pub operator to_date_time(self: text) -> DateTime { … the parse today's OpConvDateTimeFromText does … }

// formatting — today a plain `fn to_text(self: DateTime, spec: text)`                  (Op-Fold)
pub operator to_text(self: DateTime, spec: text) -> text { … unchanged body … }
pub operator to_text(self: Duration, _spec: text) -> text { … unchanged body … }
```

`negate` stays a PLAIN method under the subset (Q10): `time` calls it by name and never writes
`-w`, so marking it is not "in use".  Shown marked above as the placed spelling.

| use | rule | today |
|---|---|---|
| `a < b`, `a >= b` on dates | Op-Order (`compare`, one call) | `OpLt` / `OpGe`, four functions |
| `a == b`, `a != b` | Op-Eq (structural; `ms` only) | `OpEq` / `OpNe` |
| `b - a` → `Duration` | Op-Left: `DateTime.minus(DateTime)` | `OpMin` |
| `d - days(3)` → `DateTime` | Op-Left: `DateTime.minus(Duration)` — today a plain `fn`, now `operator` | `minus` called by name |
| `d + hours(2)` | Op-Left: `DateTime.plus(Duration)` | `OpAdd` |
| `w + w2`, `w - w2` | Op-Left on `Duration` | `OpAdd` / `OpMin` |
| `w * 3` | Op-Left: `Duration.times(integer)` | `OpMul` |
| `3 * w` | refused: `integer` has no `times(Duration)`; write `w * 3` | refused too |
| `-w` | Op-Back: `operator negate` (today a plain `fn`) | `negate` called by name |
| `d += days(1)` | Op-Compound: `d = d.plus(days(1))` | same through `OpAdd` |
| `dates.sort()` | Op-Bound: `Ordered` met by `compare` | met by `OpLt` |
| `dates.sort_by(fn(x: DateTime) -> integer { x.ms })` | an ordinary call (README Q11) | not available |
| `dt as integer` | Op-Conv: `DateTime.to_integer()` | `OpConvIntFromDateTime` |
| `1_700_000_000_000 as DateTime` | Op-Conv: `integer.to_date_time()`, defined in `time` (own target) | `OpConvDateTimeFromInt` |
| `"2026-07-08T12:00:00Z" as DateTime` | Op-Conv: `text.to_date_time()` | `OpConvDateTimeFromText` |
| `"{dt}"`, `"{dt:iso}"` | Op-Fold: `to_text(self, "")` / `to_text(self, "iso")` | plain `fn to_text` |
| `dt as text` | Op-Conv: `to_text(self, "")`, `DateTime` having only the spec form | plain `fn to_text` |

The migration is 20 `fn Op…` + 3 `fn OpConv…` + 2 plain `to_text` → 6 arithmetic and order
`operator`s, 3 conversions and the 2 `to_text`s marked; `minus(DateTime, Duration)` and
`negate` exist today as plain `fn`s and change only their keyword, and their callers by name keep
working if (Op-Name) holds.  Every operation reads the same
at its use; the definitions read better (`compare` once, not four).  A breaking release: the
`fn Op…` names go (P5).

## 2. `Grid` — element read and write

```loft
pub struct Grid { w: integer, h: integer, cells: vector<float> }

pub operator at(self: Grid, x: integer, y: integer) -> float? {                  // Op-Back (at)
  if x < 0 || y < 0 || x >= self.w || y >= self.h { return null; }
  self.cells[y * self.w + x]
}
pub operator set_at(self: Grid, x: integer, y: integer, v: float) {            // Op-Back (set_at)
  if x >= 0 && y >= 0 && x < self.w && y < self.h { self.cells[y * self.w + x] = v; }
}
```

| use | rule |
|---|---|
| `g[2, 3]` → `float?` | Op-Back (`at`), Op-Result (the `?` is the method's) |
| `g[2, 3] = 1.5` | Op-Back (`set_at`, the value last — Op-Shape) |
| `g[2, 3] += 1.0` | Op-Compound over `at` + `set_at` — P3 decides (`at` answers `float?`) |
| `g[2]` | refused: no `at` of one index (Op-Shape names the arity it has) |
| `g[0..2]` | refused: `Grid` has no `slice` (Op-Slice) |

## 3. A multi-key lookup — keyed `[]` with two keys

```loft
pub struct Fare { from: text, to: text, cents: integer }
pub struct Fares { by_route: hash<Fare[from, to]> }

pub operator at(self: Fares, from: text, to: text) -> integer? {                 // Op-Back (at)
  f = self.by_route[from, to];
  if !f { return null; }
  f.cents
}
```

| use | rule |
|---|---|
| `fares["AMS", "BER"]` → `integer?` | Op-Back (`at` with two `text` keys) — the built-in multi-key lookup `self.by_route[from, to]` stays the built-in's |
| `fares["AMS"]` | refused (Op-Shape: arity) |

Element `[]` and keyed `[]` are one rule for a user type: what the brackets hold is the method's
parameter list.  The built-in distinction (position vs key) is the built-in collection's own.

## 4. A keyed slice — placed, not allowed

```loft
pub struct Log { at: sorted<Entry[ms]> }

// Proposed shape, REFUSED in the first subset (Op-KeyRange):
//   pub operator key_range(self: Log, lo: integer?, hi: integer?) -> iterator<Entry, …>
// What a program writes today, and keeps writing:
for e in log.at[t0..t1] { … }          // the built-in sorted's own keyed slice (Slice-KeyedIter)
```

| use | rule |
|---|---|
| `for e in log[t0..t1]` | Op-KeyRange: refused until a user type needs it and a loft function can answer an iterator |
| `for e in log.at[t0..t1]` | Slice-KeyedIter (built-in), unchanged |
| `log[t0..t1]` as a value | Op-Slice would apply if `Log` defined `slice`; a type defines one of the two |

## The count

Every use above maps to exactly one rule.  `time` needs nothing outside the first subset; `Grid`
and `Fares` need only `at` / `set_at`; the keyed slice is placed and refused, and its built-in
twin is unaffected.
