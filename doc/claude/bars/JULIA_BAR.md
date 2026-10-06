# JULIA_BAR — what Julia lets a program SAY, measured against loft

**What this document is:** a bar, not a plan. It lists things Julia expresses directly that
are a SEMANTIC gain — the program states more, or a mistake is caught — and whether loft does.
Constructs whose only gain is speed are out of scope (§ Out of scope). Every entry is a small
program that either runs green on both backends or is refused with a named diagnostic; nothing
here is a commitment to implement.

**Audience:** the coding agent. The method, the expectation line and the scoring are
[OCAML_BAR.md § 0](OCAML_BAR.md#0-how-probes-work)'s; read that first.

**Relation to @PLN162.** Julia was first engaged as a design comparison:
[plans/162-multiple-dispatch/](../plans/162-multiple-dispatch/) adopted multiple dispatch
(`@F122`, entry E3 below) and deferred generated functions and broadcasting to a separate
proposal. This bar measures what that comparison did not: the rest of Julia's expressiveness,
under the same rule — *adopt what exploits compile-time type knowledge; refuse what demands
runtime type flexibility* — and with speed-only constructs left to the engine (`(Perf-Cure)`).

---

## Evaluation

Measured at `58601f08a`, every probe on both backends, from a scratch directory. <!-- doc-lint: ok -->
**The two backends agreed on every cell.**  A probe whose spelling is wrong for a capability loft
has is scored in the real spelling, and the row names it.

| entry | measured | what answered | disposition | where |
|---|---|---|---|---|
| A1 distinct alias refuses a swap | FAIL | `type Meters = float` is transparent: `speed(t, d)` compiles and answers `0.2` for `5.0` — the swap is silent | `low` | — |
| A2 alias arithmetic | **PASS** | as written; the alias names the intent but checks nothing (A1) | `floor` | this probe |
| A3 wrapper struct refuses a swap | **PASS** | *"expected Meters, got Seconds on argument 1 of call to speed"* | `floor` | this probe |
| A4 wrapper struct operators | **PASS** | the spelling is `operator plus`, not a free `fn op_add` | `floor` | [LOFT.md § Operators on your own type](../LOFT.md) |
| A5 unit arithmetic | **PASS** | `Meters / Seconds → MetersPerSecond` through `operator divided_by`, dispatched on both operand types | `floor` | this probe |
| A6 unit mismatch refused | **PASS** | *"No matching operator '+' on 'Meters' and 'Seconds'"* | `floor` | this probe |
| B1 range as a parameter | FAIL | `fn total(r: range)` — *"Expect token )"*: a range exists only inside `for` and a slice | `low` | — |
| B2 range as a local | FAIL | `r = 2..5` — *"Expect token ;"* | `low` | — |
| B3 range with a step | FAIL | `(0..10).step(3)` — *"Expect token )"*; no step spelling exists | `low` | — |
| B4 range reversed | **PASS** | the spelling is `rev(1..4)` | `floor` | this probe |
| B5 comprehension over two sources | FAIL | `[x * 10 + y for x in 0..2 for y in 0..3]` — *"Expect token ]"* | `low` | — |
| B6 one loop over a grid | FAIL | `for (x, y) in (0..3, 0..2)` — *"Expect token )"* | `low` | — |
| C1 package extension | FAIL | no surface: a library cannot ship glue that loads only when a second library is present | `low` | — |
| D1 elementwise operator | FAIL | `a .+ b` — *"Expect a field name"*; a comprehension says it today | `low` | — |
| E1 keyword arguments | **PASS** | `area(h: 3.0, w: 2.0)` and a default | `floor` | this probe |
| E2 iteration over a user type | **PASS** | a generator: `fn count(c: Counter) -> iterator<integer>` | `floor` | [LOFT.md](../LOFT.md) `iterator<T, I>` |
| E3 multiple dispatch | **PASS** | `fn area(self: Circle)` / `fn area(self: Rect)` dispatch on the runtime variant | `shipped` | `@F122`, @PLN162 |
| E4 multiple return, destructured | **PASS** | `(lo, hi) = minmax(v)` | `floor` | this probe |

**Score (capability, both backends):** A 5/6 · B 1/6 · C 0/1 · D 0/1 · E 4/4.

**What the score says.** Units of measure already have Julia's semantics in loft — a wrapper
struct refuses a swap (A3), carries the unit through arithmetic (A5) and refuses a mixed
operation (A6); what it lacks is a light spelling, since every value is built `Meters { v: … }`
and read `.v`, and the light spelling loft has, a `type` alias, checks nothing (A1).  The other
gap is ITERATION SPACES: a range is syntax, not a value (B1–B3), and a two-dimensional space
has to be hand-nested (B5, B6) — the shape every grid library in the registry writes.

### Open defects

None: every FAIL is a capability gap the program can see (a refusal) except A1, which compiles
by design — a `type` alias is transparent — and is recorded as the gap it is, not as a defect.

### Spellings the probes got wrong

- **An operator on a type is `pub operator plus(self: T, other: T) -> T`** (`minus`, `times`,
  `divided_by`, `remainder`), not a free function.
- **A range runs backwards with `rev(a..b)`.**
- **Multiple dispatch is one `fn` per variant type, `self` first**; it is not written as a
  `match` over a tuple of plain-enum values (that form is OCAML_BAR F2's fallback).

---

## Out of scope — speed spelled by the author

Julia's value-parameterised types (`SVector{3, Float64}`, `Val{N}`), `@inbounds` / `@simd`,
and broadcasting's loop FUSION are not entries here.  Their gain is speed, and loft closes a slow
routine in the engine first and never asks the author for a faster spelling it cannot name in an
`advice` ([formal/performance.md](../formal/performance.md) `(Perf-Cure)`, `(Perf-Teach)`).
D1 measures only broadcasting's NOTATION — that an operation is elementwise — which is a
statement about the program, not about its speed.

---

## Tier A — Distinct types and units of measure

Julia expresses distinct numeric types as one-field wrapper structs, and units of measure as
types the arithmetic carries (Unitful.jl): `10u"m" / 2u"s"` is `5.0 m s⁻¹`, and `1u"m" + 1u"s"`
is an error.

### A1_distinct_alias_refuses_a_swap

```loft
// @BAR: refuse "Meters"
type Meters = float;
type Seconds = float;
fn speed(d: Meters, t: Seconds) -> float { d / t }
fn main() { d: Meters = 10.0; t: Seconds = 2.0; assert(speed(t, d) == 5.0, "swapped"); }
```

### A2_alias_arithmetic

```loft
// @BAR: pass
type Meters = float;
type Seconds = float;
type MetersPerSecond = float;
fn speed(d: Meters, t: Seconds) -> MetersPerSecond { d / t }
fn main() { v: MetersPerSecond = speed(10.0, 2.0); assert(v == 5.0, "v {v}"); }
```

### A3_wrapper_struct_refuses_a_swap

```loft
// @BAR: refuse "Seconds"
struct Meters { v: float }
struct Seconds { v: float }
fn speed(d: Meters, t: Seconds) -> float { d.v / t.v }
fn main() { d = Meters { v: 10.0 }; t = Seconds { v: 2.0 }; assert(speed(t, d) == 5.0, "swapped"); }
```

### A4_wrapper_struct_operators

```loft
// @BAR: pass
struct Meters { v: float }
pub operator plus(self: Meters, other: Meters) -> Meters { Meters { v: self.v + other.v } }
fn main() { s = Meters { v: 1.5 } + Meters { v: 2.0 }; assert(s.v == 3.5, "sum {s.v}"); }
```

### A5_unit_arithmetic_with_wrappers

```loft
// @BAR: pass
struct Meters { v: float }
struct Seconds { v: float }
struct MetersPerSecond { v: float }
pub operator divided_by(self: Meters, other: Seconds) -> MetersPerSecond {
  MetersPerSecond { v: self.v / other.v }
}
fn main() { v = Meters { v: 10.0 } / Seconds { v: 2.0 }; assert(v.v == 5.0, "v {v.v}"); }
```

### A6_unit_mismatch_refused

```loft
// @BAR: refuse "Seconds"
struct Meters { v: float }
struct Seconds { v: float }
pub operator plus(self: Meters, other: Meters) -> Meters { Meters { v: self.v + other.v } }
fn main() { s = Meters { v: 1.0 } + Seconds { v: 2.0 }; assert(s.v == 3.0, "mixed"); }
```

---

## Tier B — Iteration spaces

Julia's ranges are values: `r = 2:5` can be stored, passed and asked (`length(r)`, `step(r)`);
`0:3:9` steps, `reverse(r)` runs backwards.  A grid is one iteration space —
`[f(x, y) for x in a, y in b]`, `CartesianIndices((3, 2))`.

### B1_range_as_parameter

```loft
// @BAR: pass
fn total(r: range) -> integer { s = 0; for i in r { s += i; } s }
fn main() { assert(total(1..4) == 6, "total"); }
```

### B2_range_as_local

```loft
// @BAR: pass
fn main() { r = 2..5; s = 0; for i in r { s += i; } assert(s == 9, "s {s}"); }
```

### B3_range_step

```loft
// @BAR: pass
fn main() { s = 0; for i in (0..10).step(3) { s += i; } assert(s == 18, "s {s}"); }
```

### B4_range_reversed

```loft
// @BAR: pass
fn main() { t = ""; for i in rev(1..4) { t += "{i}"; } assert(t == "321", "t {t}"); }
```

### B5_comprehension_over_two_sources

```loft
// @BAR: pass
fn main() {
  v = [ x * 10 + y for x in 0..2 for y in 0..3 ];
  assert(len(v) == 6 && v[5] == 12, "v {v}");
}
```

### B6_one_loop_over_a_grid

```loft
// @BAR: pass
fn main() { n = 0; for (x, y) in (0..3, 0..2) { n += x * 10 + y; } assert(n == 63, "n {n}"); }
```

---

## Tier C — Library composition

Julia's package extensions are glue code a library ships that loads only when a second named
package is also installed — `audio_bus` positioning sounds through `graphics` without either
depending on the other.

### C1_package_extension

No single-file probe can express it: the capability is a manifest section (`[extensions]` in
`loft.toml`, naming the second package and the glue module).  Scored FAIL because no such
section exists.

---

## Tier D — Notation

Julia's broadcasting dot says an operation is ELEMENTWISE: `a .+ b`, `f.(xs)`.

### D1_elementwise_operator

```loft
// @BAR: pass
fn main() { a = [1, 2, 3]; b = [10, 20, 30]; c = a .+ b; assert(c == [11, 22, 33], "c {c}"); }
```

---

## Tier E — Already expressed (regression floor)

### E1_keyword_arguments

```loft
// @BAR: pass
fn area(w: float, h: float = 1.0) -> float { w * h }
fn main() { assert(area(h: 3.0, w: 2.0) == 6.0, "kw"); assert(area(4.0) == 4.0, "default"); }
```

### E2_user_type_iteration

```loft
// @BAR: pass
struct Counter { n: integer }
fn count(c: Counter) -> iterator<integer> { for i in 0..c.n { yield i; } }
fn main() { s = 0; for i in count(Counter { n: 4 }) { s += i; } assert(s == 6, "s {s}"); }
```

### E3_multiple_dispatch

```loft
// @BAR: pass
enum Shape { Circle { r: float }, Rect { w: float, h: float } }
fn area(self: Circle) -> float { 3.0 * self.r * self.r }
fn area(self: Rect) -> float { self.w * self.h }
fn main() {
  s: Shape = Rect { w: 2.0, h: 3.0 };
  c: Shape = Circle { r: 1.0 };
  assert(s.area() == 6.0 && c.area() == 3.0, "dispatch");
}
```

### E4_multiple_return_destructured

```loft
// @BAR: pass
fn minmax(v: vector<integer>) -> (integer, integer) {
  lo = v[0]; hi = v[0];
  for x in v { if x < lo { lo = x; } if x > hi { hi = x; } }
  (lo, hi)
}
fn main() { (lo, hi) = minmax([4, 1, 9, 3]); assert(lo == 1 && hi == 9, "{lo} {hi}"); }
```
