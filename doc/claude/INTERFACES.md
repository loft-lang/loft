
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Interfaces — Design and Implementation Plan

> **Status: implemented (I1–I9).**  P136 (use-after-free in a bounded for-loop over a
> struct vector) is fixed — re-verified clean on both backends, poison-clean, 2026-07-05.
> `tests/scripts/86-interfaces.loft` is the guard suite; note its
> `test_bounded_for_loop_struct` is still **commented out** behind a now-stale "crashes"
> note and should be re-enabled (the case it guards passes). Strict spec:
> [formal/interfaces.md](formal/interfaces.md).

Structural interfaces for loft: implicit satisfaction, static dispatch only.
Primarily motivated by enabling bounded generic functions (`<T: Ordered>`).

---

## Contents

- [Motivation](#motivation)
- [Design principles](#design-principles)
- [Syntax](#syntax)
- [Semantics](#semantics)
- [Operator interfaces](#operator-interfaces)
- [Arithmetic in generic bodies](#arithmetic-in-generic-bodies)
- [What is out of scope](#what-is-out-of-scope)
- [Comparison to Go interfaces](#comparison-to-go-interfaces)
- [Standard library interfaces](#standard-library-interfaces)
- Implementation steps — [INTERFACES-history.md](INTERFACES-history.md)
- [Open questions](#open-questions)

---

## Motivation

Loft's current single-`<T>` generics are opaque: no arithmetic, method calls,
field access, or comparisons are allowed on a generic `T`. This forces generic
algorithms to be either reimplemented per type or written as native Rust functions.

The most painful gap is bounded generics — functions like `max_of`, `min_of`,
and user-defined sort comparators that need `T` to be comparable. All of these
currently live in native Rust or are duplicated per concrete type in the stdlib.

A second gap is generic consumers: a function that accepts "any comparable
collection element" has no way to express that today.

Interfaces fix this by adding **compile-time constraints** on `T`. No runtime
overhead is introduced — the compiler creates a specialised copy per concrete
type (as it already does for generics), and the constraint is verified at
the call site.

---

## Design principles

1. **Implicit satisfaction (structural)** — a type satisfies an interface by
   having the required methods. No `impl Interface for Type` declaration is
   needed. This matches loft's existing dispatch model (writing
   `fn area(self: Circle)` automatically participates in the `Shape` dispatch
   wrapper without any explicit declaration).

2. **Static dispatch only** — interfaces are constraints on generic type
   parameters, not first-class values. `x: Ordered` as a variable type is
   a compile error. There are no vtables, no heap-allocated interface values.

3. **`Self` in interface bodies** — within an interface declaration, `Self`
   is a placeholder for the concrete type that will satisfy the interface.
   At instantiation, every `Self` is substituted with the actual concrete type.

4. **Multiple bounds with `+`** — `<T: A + B + C>` is supported. Bounds are
   `+`-separated after the `:`. The data model stores them as `Vec<u32>` from
   the start; satisfaction is checked for each bound independently.

5. **Methods only** — interface method signatures use `self: Self` as the
   first parameter, matching loft's existing method convention. An operator
   requirement is spelled `op ⊕`, and a type meets it with its `operator` method
   (formal/operators.md).

---

## Syntax

### Interface declaration

```loft
interface Comparable {
    fn less_than(self: Self, other: Self) -> boolean
}

interface Printable {
    operator to_text(self: Self) -> text
}
```

⚠ The name here is `Comparable` on purpose: the stdlib's `Ordered` asks for `op <`, which a
type meets with `operator compare`, so a type defining `less_than` does not satisfy it. The method form is for
interfaces you declare yourself — see the Note under § Bound checking.

`interface` is a new top-level keyword. Each method is a bare signature
(no body). `Self` is the only type variable allowed inside the interface body.

**A type variable's name is yours to reuse.** A bound's methods are keyed to the
type VARIABLE, not to the name it is spelled with, so `fn render<T: Printable>` in
one library and `struct T` in a consumer are unrelated — the consumer keeps its own
`to_text`, and the library's generic keeps working. Before loft#1153 they shared one
namespace, so a bound declaring a common method reserved it against every struct
spelling that variable's name, and neither library author could see it happen.

### Bounded generic function

```loft
fn max_of<T: Comparable>(v: vector<T>) -> T {
    result = v[0];
    for item in v {
        if result.less_than(item) { result = item; }
    }
    result
}
```

The bound is written as `<T: InterfaceName>`, or with multiple bounds as
`<T: A + B>`. Inside the function body, any method declared in any of the
listed interfaces may be called on values of type `T`. All other restrictions
on `T` remain (no field access, no arithmetic unless the bound includes the
relevant operator interface).

```loft
fn find_max_and_log<T: Ordered + Printable>(v: vector<T>) {
    best = max_of(v);
    log_info(best.to_text());
}
```

### Satisfying an interface

No declaration is required. Any type that has all the required methods
automatically satisfies the interface:

```loft
struct Priority { value: integer }
fn less_than(self: Priority, other: Priority) -> boolean {
    self.value < other.value
}

// Priority now satisfies Comparable — no explicit declaration.
max_of([Priority{value: 3}, Priority{value: 1}, Priority{value: 7}])
```

A stdlib interface whose members are operators is met by the `operator` method each one
names, and one definition serves both the bound and the bare operator (formal/operators.md
`(Op-Bound)`):

```loft
operator compare(self: Priority, other: Priority) -> Ordering { self.value.compare(other.value) }

// Priority now satisfies Ordered, and `a < b` works on it as well.
```

If a method is missing, the compiler reports an error at the call site, naming the one to
write:

```
error: 'Priority' does not satisfy interface 'Ordered': missing `operator compare(self: Priority, other: Priority) -> Ordering`
```

---

## Semantics

### Satisfaction check

A concrete type `C` satisfies interface `I` if, for every method signature
`fn m(self: Self, p1: T1, ...) -> R` declared in `I`, there exists a
function `m` visible at the call site whose first parameter type is `C`,
whose remaining parameters match `T1, ...` (with `Self` replaced by `C`),
and whose return type matches `R` (with `Self` replaced by `C`).

The check is performed when a bounded generic function is instantiated —
i.e. when `max_of(v)` is first encountered with a concrete `T`. The check
happens once per concrete type per function; subsequent calls with the same
`T` skip the check.

### Dispatch inside the generic body

When the compiler specialises a bounded generic function for a concrete `T`,
method calls `x.m(...)` where `m` is declared in the bound interface are
resolved to the concrete function `m` for type `T`. This is the same
process as ordinary method resolution — no new dispatch mechanism is needed.
The specialised copy of the function body is compiled with concrete types
substituted throughout, exactly as the existing generic specialisation does.

### Visibility

Satisfaction is checked using functions that are in scope at the **call site**,
not at the interface definition site. A type defined in library A can satisfy
an interface defined in library B as long as both are visible to the caller.

---

## Operator interfaces

An interface declares an operator requirement as the `operator` member that meets it:

```loft
interface Joinable {
    operator plus(self: Self, other: Self) -> Self
}

struct Money { cents: integer }
operator plus(self: Money, other: Money) -> Money { Money { cents: self.cents + other.cents } }

fn join<T: Joinable>(a: T, b: T) -> T { a + b }
```

Inside a generic body bounded by `Joinable`, `a + b` is allowed and calls the concrete
type's operator.  A type meets the member with its `operator` method of that name, at the
member's own signature (`operator times(self: Self, k: float)` by `operator times(self: T, k:
float)`); a built-in type with the stdlib's own.  The symbol spelling (`op +`) is refused in an
interface body, naming the member to write.  The reserved forms — the bit operators, `**` —
have no `operator` method yet, and an interface cannot require `[…]` at all (it belongs to the
built-in collections).

A member written `operator` (`operator to_text(self: Self) -> text`, as `Printable`
declares) is met only by an `operator` definition, never by a plain `fn` of the name
(`(Op-Iface)`).

---

## Arithmetic in generic bodies

This section details the four cases that arise when `T` participates in
arithmetic, and how each is handled by the interface design.

### Case 1 — Same-type binary operators: `T op T -> T` or `T op T -> boolean`

The common case: `total + item`, `a < b`, `x == y`. Both operands are `T`;
the result is either `T` or a concrete type (`boolean`).

```loft
interface Addable {
    operator plus(self: Self, other: Self) -> Self
}
fn sum_of<T: Addable>(v: vector<T>) -> T {
    result = v[0];
    for item in v[1..] { result = result + item; }
    result
}
```

Inside the generic body, the type of `result + item` is determined by the
interface method's return type (`Self` → `T`). Step I8 reads the declared
return type from the interface method signature and uses it as the expression
type in the IR, rather than emitting the "operator requires concrete type"
error.

### Case 2 — Mixed-type binary operators: `T op concrete -> T`

Sometimes the second operand is a fixed concrete type, not another `T`:
`distance * 2.0`, `count + 1`. This is expressible by declaring the concrete
type explicitly in the interface method signature:

```loft
interface Scalable {
    operator times(self: Self, factor: float) -> Self
}
fn scale_all<T: Scalable>(v: vector<T>, factor: float) -> vector<T> {
    [item * factor for item in v]
}
```

The satisfaction check (I6) matches the second parameter as a concrete type,
not `Self`. At the operator dispatch site (I8), when `x * factor` is
encountered with `x: T` and `factor: float`, the interface's `op * (Self, float)`
signature matches and the call is allowed.  A program's type meets it with `operator
times(self: T, factor: float) -> T`.

Concrete types on the **left** side (`concrete op T -> T`) are not supported
in phase 1 — operator dispatch always starts from the `self` position.

### Case 3 — Operators with non-Self return type: `T op T -> concrete`

Some operators produce a widened or different type: average computation needs
`T / T -> float`, a hash function needs `T -> integer`. These are declared
with a concrete return type in the interface:

```loft
interface Hashable {
    fn hash(self: Self) -> integer
}
interface Averageable {
    operator plus(self: Self, other: Self) -> Self
    operator divided_by(self: Self, divisor: integer) -> float
}
fn average<T: Averageable>(v: vector<T>) -> float {
    total = v[0];
    for item in v[1..] { total = total + item; }
    total / len(v)
}
```

In the generic body, `total / len(v)` has type `float` (the declared return
type of the `op /` member), not `T`. Step I8 must propagate the return type from the
interface signature to the IR expression type. `Self` in the return position
is replaced with `T`; any concrete type is used as-is.

### Case 4 — Zero / identity element

A recurring problem in generic arithmetic is initialisation: `total = 0`
does not type-check when `total: T`. Loft's null-propagation model provides
a natural solution: **null is the universal zero sentinel for arithmetic**.

```loft
fn sum_of<T: Addable>(v: vector<T>) -> T {
    result = v[0];              // null if vector is empty
    for item in v[1..] { result = result + item; }
    result                      // null for empty vector
}
```

If the vector is empty, `v[0]` returns the null sentinel for `T`
(`i32::MIN` for integer, `NaN` for float, etc.), and the loop body never
executes. The caller receives null, which is consistent with loft's standard
pattern for "no value" results.

For cases where null-as-zero is not acceptable, take the starting value from the
caller — `fn sum_of<T: Addable>(v: vector<T>, identity: T) -> T` (Q6 below).  A
`zero()` factory in the interface is refused: a method returning `Self` needs a
`self: Self` first parameter to name the type (Q4).

### Compound assignment

`total += item` is `total = total + item` (`(Op-Compound)`), so it reaches the `op +`
member with no separate treatment.

### Unary operators

Declared with `self` only, no second parameter:

```loft
interface Negatable {
    operator negate(self: Self) -> Self
}
fn negate_all<T: Negatable>(v: vector<T>) -> vector<T> {
    [-item for item in v]
}
```

A unary member is checked exactly as a binary one is: a program's type meets it with its
`operator negate`, a built-in number with the stdlib's.

---

## What is out of scope

- **Dynamic dispatch / interface values** — `x: Ordered = my_priority` is not
  supported. Interfaces are constraint annotations, not types.
- **Composite interfaces / interface inheritance** — `interface A extends B` is
  not supported; declare the methods directly in the interface that needs them.
- **Default method implementations** — no bodies inside interface declarations.
- **Naming a companion type outside its interface** — an associated type is in scope
  (§ Associated types below), but `Self.<Name>` is spellable only inside the declaring
  interface's body, so a generic cannot write `S.Rows` in its own signature.
- **Implementing an interface for a type you didn't define** — satisfaction is
  structural; if the method exists with the right signature, it counts. There is
  no orphan rule.

All of the above can be added later. The implementation steps below are
designed to avoid closing off these extensions prematurely.

---

## How first-grade a library type is — measured

"Can a library define a type that behaves like a built-in one?" is the question
behind most requests for language features, and loft answers it in more places
than it gets credit for. Measured on the current tree, not recalled:

| capability | today | how |
|---|---|---|
| render in `"{x}"` | **yes** | `operator to_text(self: T) -> text` (formal/operators.md `(Op-Fold)`) |
| order (`< <= > >=`) | **yes** | `operator compare(self: T, other: T) -> Ordering` (formal/operators.md) |
| `+ - *` | **yes** | `operator plus` / `minus` / `times`, `a += b` included (formal/operators.md) |
| `x as T` | **yes** | `operator to_<t>(self: S) -> T` — `to_date_time` answers `DateTime` (`(Op-Conv)`) |
| `==` / `!=` | **yes, structural** | by the fields, for every type at every depth; no type redefines them (@C134) — a comparison of its own is a named method |
| `for x in <value>` | **yes** | `operator next(self: T) -> τ?` on the type — a struct iterates like a collection |
| bounded generics | **yes** | structural satisfaction, no `impl` block |
| **receive the parts of `"{…}"`** | **yes** | `fn lit(self: T, s: text)` + `fn hole_<kind>(self: T, v: τ)` — the target type decides (@PLN124). A hole may be a scalar OR a value of a named type, whose kind is its own name in method case (`SqlIdent` → `hole_sql_ident`) |
| **associated types** | **yes** | `type Rows: Cursor` in an interface body; `Self.Rows` in its signatures (@PLN125 arc A) |
| `x[i]` indexing | **no, by decision** | `[…]` belongs to the built-in collections; a type reads through a named method, `x.at(i)` (C132, see below) |
| **run at scope end** | **yes** | `fn OpDrop(self: T)` — runs when the value's OWNER dies (@PLN125 arc B, @PLN139) |
| **lease a copy** | **yes** | `fn OpCopy(self: T)` — runs on the new structure a copy makes (@PLN163 P4) |

One row is a NO by decision rather than a gap: `x[i]` (below).  Each arc landed **inert first** — the
contract declared, every existing program proved byte-identical in IR and native
Rust, before any new behaviour was routed through it. That ordering is what kept
each language change from being a rewrite: the proof that nothing changed is a
smaller and much earlier step than the feature.

**Running at scope end — `OpDrop`** — see [INTERFACES_DROP.md](INTERFACES_DROP.md).

---

## Indexing — a library type reads through a named method

`[…]` belongs to the built-in collections — `vector`, `text`, the keyed collections.  A
library type reads an element through a NAMED method it defines like any other, and a bound
can require that method:

```loft
struct Ring { data: vector<integer>, start: integer }

fn at(self: Ring, i: integer) -> integer {
  n = len(self.data);
  if n == 0 { return 0; }
  return self.data[(self.start + i) % n] ?? 0;
}

r = Ring { data: [10, 20, 30], start: 1 };
r.at(0)     // 20 — the ring's own offset, not `data[0]`
```

The index type is whatever the method declares (a row addressed by column name takes a
`text`), and out of range is the TYPE's answer.  `x[i]` on a program's type — and `x[a, b]`,
`x[a..b]`, `x[i] = v` — is one refusal naming that cure, and an interface cannot require
`op []`, since no type could meet it.  A type defined `fn OpIndex` for `x[i]` until C132
retired it with every other `fn Op…` (@PLN182 Q10); `tests/scripts/pln125-c-index.loft` keeps
that arc's cases running through `at`, and `tests/scripts/996-opindex-composite-subscript.loft`
pins the refusal.

## Associated types — an interface that names a companion type

An interface can name a **type that goes with** the implementor, not only a set of
methods. A `sql` connection and the cursor it produces are one contract, and
without this the cursor has to become state ON the connection — which means a
connection can hold only one, and the type system cannot say so.

That example is not hypothetical: it is what `tests/fixtures/sqldb` did, across
four real drivers, and @PLN138 has since moved it onto this feature.
`tests/fixtures/sqldb/two_cursors.loft` is the worked consumer — two cursors from
one connection, interleaved and nested, over a contract that names no backend.

```loft
interface Cursor {
  fn width(self: Self) -> integer
}

interface Source {
  type Rows: Cursor                     // the companion, and what it must satisfy
  fn open(self: Self) -> Self.Rows      // named in a signature as `Self.<Name>`
  fn label(self: Self) -> text
}
```

An implementor declares the methods, as always — there is no `impl` block, so
there is nowhere to write the companion down, and nowhere is needed:

```loft
struct FileRows  { w: integer }
struct FileSource { path: text }

fn width(self: FileRows) -> integer { return self.w; }
fn open(self: FileSource) -> FileRows { return FileRows { w: len(self.path) }; }
fn label(self: FileSource) -> text { return "file"; }
```

**The rule in one sentence:** an associated type is a type variable owned by the
interface — inside a generic it dispatches through its declared bounds exactly as
`<T: I>` does, and at instantiation it binds to the one concrete type the
implementor's methods agree on, which must satisfy those bounds.

So a generic may hold the companion and call the bound's methods on it, and it
binds per implementor:

```loft
fn first_width<S: Source>(s: S) -> integer {
  r = s.open();        // r is S's own companion — FileRows here
  return r.width();    // authorised by `type Rows: Cursor`
}
```

Three consequences worth stating:

- **The companion is INFERRED from the implementor's signature**, read back
  through the interface's: where the interface writes `Self.Rows`, the
  implementor writes a concrete type. Return and parameter positions are both
  read, and they must AGREE — an implementor whose `open` yields one type while
  its `feed` takes another is refused rather than resolved by declaration order.
  What is read is the SHAPE and not the lifetime: the implementor's return type
  carries a dep list indexed in its own frame, non-empty exactly when its producer
  returns a record from a nested call rather than constructing one inline, and
  those indices name unrelated caller locals once substituted into a monomorph.
  So the binding is recorded with its deps stripped — the same answer loft#666
  needed for a type used as a hint.
- **The bound is checked per monomorph**, because the companion is per
  implementor. A companion missing one of the bound's methods is a compile error
  naming the implementor, the associated type, the companion, the bound and the
  method — three of those are invisible at the call site, where the reader only
  wrote `first_width(s)`.
- **A bound-less `type Held` still binds.** Nothing of the companion's own is
  callable — no bound, no methods — but the interface that named it can take it
  back (`fn keep(self: Self, h: Self.Held)`), and that round trip is what an
  un-bounded companion is for.

There is **no runtime cost**: generics are static-dispatch and specialise per
concrete type, so an associated type is a compile-time name and the monomorph is
byte-identical to the same body written against the concrete types by hand. It is
not dynamic dispatch and not a trait-object system — it names a type in a
contract, it does not choose an implementation at run time.

Two rules on the syntax: the name is a type name and is enforced **CamelCase**,
and `Self.<Name>` is only spellable inside the declaring interface's body —
elsewhere a `.` after a type is not this construct.

Shipped as @PLN125 arc A; `tests/scripts/pln125-a2c-companion.loft` is the
behaviour matrix.

## Interpolation targets — receiving the parts of `"{…}"`

A format string does not have to become one flat `text`. When the string's TARGET
TYPE defines the methods below, the parser hands that type the literal chunks and
the interpolated values **separately**, instead of appending them into a text
buffer:

```loft
fn lit(self: T, s: text)              // a literal chunk the AUTHOR wrote
fn hole_text(self: T, v: text?)       // an interpolated VALUE
fn hole_int(self: T, v: integer)
fn hole_float(self: T, v: float)
fn hole_single(self: T, v: single)
fn hole_boolean(self: T, v: boolean)
fn hole_character(self: T, v: character)
```

So

```loft
q: SqlText = "SELECT id FROM t WHERE name = {name}";
```

lowers to `q.lit("SELECT id FROM t WHERE name = "); q.hole_text(name);`, with the
accumulator itself as the value of the expression. `text` is unchanged — a target
that does not define `lit` formats exactly as it always did.

**`lit` is the whole test for whether the hook applies.** A type that can accept
the author's literal bytes is a type that can be built. The `hole_*` methods it
goes on to define say which value kinds it takes.

**A hole may also be a value of a NAMED type**, struct or enum, and its kind is the
type's own name in the case a loft method is spelled in — an acronym run breaks at
the last capital:

```loft
fn hole_sql_ident(self: SqlText, v: SqlIdent?)   // SqlIdent / SQLIdent -> hole_sql_ident
fn hole_level(self: Trace, v: Level)             // Level               -> hole_level
```

The name is DERIVED rather than chosen, so a target and the parser cannot disagree
about what a type's hole is called, and the diagnostic can name the exact method to
add. This is what lets a target hold something apart from both a literal and a
bound value: a SQL table name really is syntax, so `SqlText` puts it inline — and
the safety then rests on the TYPE, because nothing builds a `SqlIdent` but its
validating constructor. One method, one constructor, one place to audit.

**Two refusals, and they are the point:**

- **A kind the target does not define is a compile error** naming the method to
  add — never a quiet fall back to text, which would put a value back on the path
  this exists to close.
- **A spec on a hole is refused.** `{v:>8}` has already decided how the value
  should look, and the target wants the VALUE. A target that needs formatting
  receives the hole as data and renders it itself.

Why the target type carries this and not the type system: only the parser knows
where a literal ends and a hole begins, and that boundary is gone by the time any
value exists. Neither a type nor a `const` can carry it, which is why this is a
parser hook rather than a library convention. `Parser::interpolation_target` reads
the target off the one expected-type channel that already carries `lambda_hint` /
`enum_hint` / `vector_hint`, so it is a fifth shape on that channel rather than a
sixth side-channel.

The per-kind `hole_*` form is **deliberate and stays**.  Collapsing it into one
generic `hole<T>` was evaluated and DECLINED ([C110](DESIGN_DECISIONS.md), kept by
[C126](DESIGN_DECISIONS.md)).  An unbounded `hole<T>` accepts every type, deleting the
per-kind opt-in.  A BOUNDED one (`<T: SqlHole>`) keeps the compile error — a type that has
not opted in is refused naming the method to add — but hands the opt-in list to every
implementor, where the per-kind form keeps it with the target's author: the refusal above
is auditable by reading ONE target's method list.  The cost is paid once per target type
by a library author and never by a consumer.

(It was recorded as @PLN125 arc A's A4 until that arc shipped and showed the two
are not the same gap — an associated type names a COMPANION, while collapsing
`hole_*` needs a type variable in a later PARAMETER.) Catalogued as
[`@F94`](https://github.com/loft-lang/features/issues/94); the design reasoning is
[plans/23-db-clients/INTERPOLATION_HOOK.md](plans/23-db-clients/INTERPOLATION_HOOK.md).

## Comparison to Go interfaces

| Property | Go interfaces | loft interfaces (this design) |
|---|---|---|
| Satisfaction | Implicit / structural | Implicit / structural (same) |
| Dynamic dispatch | Yes — interface values carry a vtable | No — bounds only, no vtables |
| Interface as a type | `var x io.Reader = ...` | Not allowed |
| Generic bounds | `[T interface{ M() }]` (Go 1.18+) | `<T: Interface>` (same concept) |
| Operator requirements | Not natively expressible | `op ⊕` members, met by `operator` methods |
| Multiple bounds | `[T A ∩ B]` | `<T: A + B>` — supported |
| Default methods | No | No |

The dispatch model (no vtables, static specialisation) aligns with Go 1.18+
generic constraints rather than classic Go interface values.

---

## Standard library interfaces

What `default/01_code.loft` ships, each member spelled as the definition that meets it
(`(Op-Iface)`):

```loft
pub interface Ordered      { operator compare(self: Self, other: Self) -> Ordering }
pub interface Equatable    { }                     // every type meets it: `==` is structural
pub interface Addable      { operator plus(self: Self, other: Self) -> Self }
pub interface Subtractable { operator minus(self: Self, other: Self) -> Self }
pub interface Numeric      { operator times(self: Self, other: Self) -> Self
                             operator negate(self: Self) -> Self }
pub interface Scalable     { fn scale(self: Self, factor: integer) -> integer }
pub interface Printable    { operator to_text(self: Self) -> text }
```

A built-in type meets them with the stdlib's own `operator` definitions (`integer`, `single`
and `float` define `compare`, `plus`, `minus`, `times`, `negate`, `divided_by` and
`remainder`; `text` and `character` define `compare`), and a program's type with its own —
`integer` meets `Ordered` the way a `Date` does.  A generic body keeps the built-in operator
underneath for a built-in type: each `operator` member brings the symbolic member it stands
for (`OpLt` beside `compare`), which the monomorph lowers to `OpLtInt` at `integer` and to
`a.compare(b) == Less` at a program's type.  `Equatable` declares nothing and is met by every
type, because `==` is structural for all of them (@C134).

So `-` is not in `Addable`, `+` and `/` are not in `Numeric`, `Scalable` takes an INTEGER
factor through a method and answers `integer` rather than `Self`, and no built-in type
satisfies `Scalable` at all.  The four order forms all read `compare`, and `!=` derives from
`==`.  `tests/scripts/the-reference-bounds-permit-what-it-lists.loft` holds the permitted half
and `tests/parse_errors.rs`'s `generic_bound_*` family the refused half.

**The list is narrower than it looks, and deliberately so.** Each bound names the FEWEST
operators its forms need: `Ordered` carries only `compare` because all four order forms read
it, and `Equatable` nothing at all.  An interface demanding every
spelling would break every type that implements the minimum.

⚠ **`Numeric`'s `-` is UNARY negation (`negate`); binary subtraction is `Subtractable`
(`minus`).**  Write `<T: Subtractable>` for `a - b`, `<T: Numeric>` for `-a`, and `<T: Numeric
+ Subtractable>` for both.  Subtraction is a separate bound rather than a third requirement on
`Numeric` because adding one would take satisfaction away from every type that meets
`Numeric` without it — `(G-Sat)` is structural, so a new requirement is a breaking change
([COMPATIBILITY.md](COMPATIBILITY.md)).

**Your own interfaces may declare both arities, in ONE interface or one per interface.** An
interface is a set of SIGNATURES (`(G-Iface)`), so

```loft
interface SubNeg {
  operator minus(self: Self, other: Self) -> Self
  operator negate(self: Self) -> Self
}
```

compiles and both are reachable from a `<T: SubNeg>` body.  Two interfaces each declaring one
work too, including when both generics spell their type variable `T` — a generic header binds
its own type variable (`(G-Gen)`).  The symbol spelling `op -` is
refused in an interface body, naming the `operator` member to write.

**A built-in number provides both arities**, so it satisfies `Numeric + Subtractable`; a
program's type provides the binary one (`operator minus`) and not yet the unary one.  Asking a
type for a member it does not meet is a compile error naming the interface and what to write
(*"'Money' does not satisfy interface 'Subtractable': missing `operator minus(self: Money,
other: Money) -> Money`"*), not a silently dropped operand.

`Scalable` scales through a `scale` METHOD rather than `op *` for the neighbouring reason: it
would share `Numeric`'s `*` at the SAME arity, and same-name same-arity requirements from two
interfaces are one stub, so the second is taken to agree with the first.
`text` satisfies `Ordered` and `Equatable`. No extra declarations are needed.
Every type satisfies `Equatable` (`(G-Sat-Eq)`, C91): a struct, `value struct`, struct-enum,
vector or keyed collection with no `op ==` of its own is compared by content in the monomorph,
exactly as a concrete `a == b` on it is; a tuple element by element, nested tuples included.

**Stdlib functions converted from native to bounded-generic loft** (depends on I8):

| Function | Bound | Notes |
|---|---|---|
| `sum_of<T: Addable>` | `Addable` | first-element init; null for empty vector |
| `min_of<T: Ordered>` | `Ordered` | first-element init; null for empty vector |
| `max_of<T: Ordered>` | `Ordered` | first-element init; null for empty vector |

---

**Implementation steps (I1–I10)** (implemented) — the record is in [INTERFACES-history.md](INTERFACES-history.md).

---

## Open questions

**Q1: Multiple bounds** — resolved. `<T: A + B>` is supported from the start.
`Definition.bounds` is `Vec<u32>` from I2 onward; the parser (I4) reads
`+`-separated names in a loop; satisfaction (I6) and lookup (I7, I8) iterate
over all bounds. The incremental cost over a single-bound design is ~40 lines.

**Q2: Operator method naming in interfaces** — resolved.  An interface spells an operator
requirement as the definition that meets it, `operator compare(self: Self, other: Self) ->
Ordering`, and a type meets it with its `operator` method, never by a function's name (C132,
@PLN182 Q13).

**Q3: Interface visibility / `pub`** — should interfaces follow the same
`pub` / non-`pub` visibility rules as functions? Recommended: yes, using the
existing `pub_visible` field on `Definition`.

*Mitigation:* Reuse `pub_visible` on `Definition` unchanged. `parse_interface`
checks for a leading `pub` token and sets the flag exactly as `parse_function`
does. No new field or mechanism required.

**Q4: `Self` in return position** — `fn create(x: integer) -> Self` (a
factory method with no `self` parameter) is probably not useful at this stage
and complicates the `Self` substitution. Restrict `Self` to appear only when
`self: Self` is the first parameter in phase 1.

*Mitigation (phase 1):* In the I5 validation pass, emit
`"factory methods (Self in return without self parameter) are not yet supported"`
if `Self` appears in the return type but no `self: Self` first parameter is
present. This makes the restriction explicit rather than silently producing
wrong code. The caller-supplied-identity overload
(`fn sum_of<T: Addable>(v: vector<T>, identity: T) -> T`) is the recommended
workaround for the empty-collection case (see Q6).

*Mitigation (phase 2):* Track a separate `Self` substitution for parameterless
factory methods keyed by the call-site's concrete type. Requires no data-model
change; only extends the substitution logic in I6.

**Q5: Interfaces in the doc generator** — `gendoc` (`src/documentation.rs`)
will need a rendering path for `DefType::Interface`. Deferring to after the
feature lands; add a stub that omits interfaces from HTML output until then.

*Mitigation:* Add a guard in the `documentation.rs` rendering loop that
silently skips `DefType::Interface` definitions (the same pattern used for
any unhandled variant). This prevents a panic on the first `cargo run --bin
gendoc` run after I2 lands. A proper interface section (name, signatures,
known implementing types) can be added as a follow-up without touching any
other step.

**Q6: Zero/identity element for generic arithmetic** — the first-element
initialisation pattern (`result = v[0]; for item in v[1..]`) is loft-idiomatic
and returns null for empty collections, which is consistent with null
propagation elsewhere. However, some algorithms need an explicit zero:
an empty-safe `sum_of` that returns 0 (not null) for an empty vector.
Two paths exist:

- **Relax Q4** and allow factory methods without `self`: `fn zero() -> Self`.
  Then `Addable` gains `fn zero() -> Self`, and `sum_of` calls `T.zero()`
  for its initial value. Requires extending `Self` substitution to cover
  parameterless functions.
- **Caller-supplied identity**: add an overload
  `fn sum_of<T: Addable>(v: vector<T>, identity: T) -> T`
  where the caller passes the zero value. No language change needed.

*Mitigation:* Ship the caller-supplied-identity overload in phase 1 alongside
I9. Add it next to the first-element form in `default/01_code.loft`. This
covers the empty-safe use case with no language change. Revisit the factory
method form (`fn zero() -> Self`) in phase 2 after Q4 is relaxed.

---

## Phase 1 gaps

### Left-side concrete operand (`concrete op T -> T`)

`2.0 * my_t_value` is not supported in phase 1. Operator dispatch always
starts from the `self` position, so the left operand must be of type `T`.

*Mitigation (phase 1):* Document as a known limitation. Most cases can be
rewritten using commutativity: `my_t_value * 2.0`. Where commutativity does
not hold, the user defines a helper method instead of relying on operator
syntax.

*Decided (@PLN182 Q4):* nothing commutes.  `2.0 * w` is refused, and since `float` is not
the program's type (`(Op-Home)`) it cannot be made to work; write `w * 2.0`.
