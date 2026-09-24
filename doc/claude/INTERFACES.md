
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
- [Implementation steps](#implementation-steps)
  - [I1 — Lexer: add `interface` keyword](#i1--lexer-add-interface-keyword)
  - [I2 — Data: add `DefType::Interface` and `Definition.bound`](#i2--data-add-deftypeinterface-and-definitionbound)
  - [I3 — Parser first pass: parse interface declarations](#i3--parser-first-pass-parse-interface-declarations)
  - [I4 — Parser first pass: parse `<T: Bound>` syntax](#i4--parser-first-pass-parse-t-bound-syntax)
  - [I5 — Type resolution: validate interface bodies](#i5--type-resolution-validate-interface-bodies)
  - [I6 — Satisfaction checking at instantiation](#i6--satisfaction-checking-at-instantiation)
  - [I7 — Allow bounded method calls on T](#i7--allow-bounded-method-calls-on-t)
  - [I8 — Operator interfaces](#i8--operator-interfaces)
  - [I9 — Standard library interfaces](#i9--standard-library-interfaces)
  - [I10 — Diagnostics](#i10--diagnostics)
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
   first parameter, matching loft's existing method convention. Operator
   interfaces use the `OpCamelCase` naming the stdlib already uses internally.

---

## Syntax

### Interface declaration

```loft
interface Comparable {
    fn less_than(self: Self, other: Self) -> boolean
}

interface Printable {
    fn to_text(self: Self) -> text
}
```

⚠ The name here is `Comparable` on purpose: the SHIPPED `Ordered` is keyed to `OpLt`, the
operator form, so a type defining `less_than` does not satisfy it. The method form is for
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

To satisfy a SHIPPED interface the method is the operator's own name, and one definition
serves both the bound and the bare operator:

```loft
fn OpLt(self: Priority, other: Priority) -> boolean { self.value < other.value }

// Priority now satisfies Ordered, and `a < b` works on it as well.
```

If a method is missing, the compiler reports an error at the call site, naming the one to
write:

```
error: 'Priority' does not satisfy interface 'Ordered': missing OpLt
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

Loft operators dispatch via the `OpCamelCase` naming scheme. An interface
can declare operator requirements using the same names:

```loft
interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
}

interface Ordered {
    fn OpLt(self: Self, other: Self) -> boolean
    fn OpGt(self: Self, other: Self) -> boolean
}
```

Inside a generic body bounded by `Addable`, `x + y` is allowed and resolves
to the `OpAdd` implementation for the concrete type. This requires a small
change to the existing generic body type-checking: when an operator is applied
to `T`, check if the operator name is declared in the bound interface before
emitting the "operator requires concrete type" error.

A built-in operator interface is **satisfied automatically** if the concrete
type already has the relevant operator defined — no `fn OpAdd(self: Priority)`
stub is required if `+` already works on `Priority`.

**Note:** `fn less_than` (method form) and `fn OpLt` (operator form) are
two separate ways to express the same capability. Prefer the method form for
readability in user-defined interfaces; the operator form for stdlib interfaces
that hook into loft's operator dispatch.

---

## Arithmetic in generic bodies

This section details the four cases that arise when `T` participates in
arithmetic, and how each is handled by the interface design.

### Case 1 — Same-type binary operators: `T op T -> T` or `T op T -> boolean`

The common case: `total + item`, `a < b`, `x == y`. Both operands are `T`;
the result is either `T` or a concrete type (`boolean`).

```loft
interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
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
    fn OpMul(self: Self, factor: float) -> Self
}
fn scale_all<T: Scalable>(v: vector<T>, factor: float) -> vector<T> {
    [item * factor for item in v]
}
```

The satisfaction check (I6) matches the second parameter as a concrete type,
not `Self`. At the operator dispatch site (I8), when `x * factor` is
encountered with `x: T` and `factor: float`, the interface's `OpMul(Self, float)`
signature matches and the call is allowed.

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
    fn OpAdd(self: Self, other: Self) -> Self
    fn OpDiv(self: Self, divisor: integer) -> float
}
fn average<T: Averageable>(v: vector<T>) -> float {
    total = v[0];
    for item in v[1..] { total = total + item; }
    total / len(v)
}
```

In the generic body, `total / len(v)` has type `float` (the declared return
type of `OpDiv`), not `T`. Step I8 must propagate the return type from the
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

For cases where null-as-zero is not acceptable, declare a `zero()` factory
method in the interface (see open question Q4 and Q6 below).

### Compound assignment

`total += item` desugars to `total = total + item` in loft's parser, so it
is handled by `OpAdd` automatically. No separate treatment needed.

### Unary operators

Declared with `self` only, no second parameter:

```loft
interface Negatable {
    fn OpNeg(self: Self) -> Self
}
fn negate_all<T: Negatable>(v: vector<T>) -> vector<T> {
    [-item for item in v]
}
```

I8 handles unary operators identically to binary ones: map the operator token
to its `OpCamelCase` name (`-` unary → `"OpNeg"`), check the bound, allow if
declared.

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
| render in `"{x}"` | **yes** | `fn to_text(self: T) -> text` (@PLN99) |
| arithmetic / comparison | **yes** | `fn OpAdd(self: T, other: U) -> V`, `OpEq`, `OpLt`, … |
| `for x in <value>` | **yes** | a `next(self) -> τ?` on the type — a struct iterates like a collection |
| bounded generics | **yes** | structural satisfaction, no `impl` block |
| **receive the parts of `"{…}"`** | **yes** | `fn lit(self: T, s: text)` + `fn hole_<kind>(self: T, v: τ)` — the target type decides (@PLN124). A hole may be a scalar OR a value of a named type, whose kind is its own name in method case (`SqlIdent` → `hole_sql_ident`) |
| **associated types** | **yes** | `type Rows: Cursor` in an interface body; `Self.Rows` in its signatures (@PLN125 arc A) |
| **`x[i]` indexing** | **yes** | `fn OpIndex(self: T, i: τ) -> υ`; an interface requires it with `op []` (@PLN125 arc C) |
| **run at scope end** | **yes** | `fn OpDrop(self: T)` — runs when the value's OWNER dies (@PLN125 arc B, @PLN139) |
| **lease a copy** | **yes** | `fn OpCopy(self: T)` — runs on the new structure a copy makes (@PLN163 P4) |

No gaps are left in the measured set. Each arc landed **inert first** — the
contract declared, every existing program proved byte-identical in IR and native
Rust, before any new behaviour was routed through it. That ordering is what kept
each language change from being a rewrite: the proof that nothing changed is a
smaller and much earlier step than the feature.

## Running at scope end — `OpDrop`

A type that defines `OpDrop` runs it when a scope that OWNS one lets it die —
the shape RAII gives a transaction, a file handle or a C resource:

```loft
struct Tx { tag: text, done: boolean }

fn commit(self: Tx) -> boolean { …; self.done = true; return true; }

fn OpDrop(self: Tx) {
  if !self.done { rollback(self.tag); }
}

fn work() {
  t = begin("orders");
  …                       // no commit on this path
}                         // <- rollback runs here
```

**The rule:**

> A drop runs when the value's OWNER dies. Taking a value back out of its owner
> does not run one.

For a value held in a plain binding, the owner is that binding, and the drop runs
exactly where the binding's own `OpFree*` runs — the same scope exit, the same
early-exit paths. loft already computes that. The ownership model decides, per
binding, whether this scope owns a value and whether it dies here, which is what
emits `OpFreeRef`; a returned or borrowed value is already excluded, and the
early `return` / `break` / return-out-of-a-loop cases are already handled there
(loft#731 exists because a hand-rolled version of exactly those went wrong). So
the hook DERIVES from the borrow model rather than sitting beside it, and there
is one answer to "when does this run", not two that can drift.

### Putting one in a container

Copying a droppable into a struct field, an enum payload or a collection element
is a **move**: the container becomes the owner. The value you copied FROM no
longer drops, and the container's death releases what it holds.

```loft
struct Handle { id: integer }
fn OpDrop(self: Handle) { close(self.id); }

struct Session { h: Handle }

fn open_session() -> Session {
  h = acquire();
  return Session { h: h };   // `h` no longer drops — the Session owns it now
}

fn work() {
  s = open_session();
  …
}                            // <- close() runs here, once
```

What moves is the value and the RESPONSIBILITY to release it: `h` is spent after
that line, and reading it is `error[read-after-move]` — read it as `s.h` instead. Without that move
the resource was released twice over: once by the source at its own scope end,
and never by the container. That is invisible while both die in the same scope,
and it is a use-after-free the moment the container outlives the source, which is
what `open_session` above does.

**A reassignment releases what it displaces.** `s = S { h: acquire() }; s = S { h: acquire() }`
runs the hook on the first record's handle at the second assignment — after the new value
has been computed, so `s = grow(s)` still finds the old resource live while `grow` runs —
and so does a rebind from a call, to `null`, or inside a loop of a local declared outside
it. The hook used to run at scope end only, and the first handle was never closed
(loft#1362).

**Whether a line copies or moves is read off that line.** A value the function OWNS
— a local it bound to a fresh value — MOVES when it is placed: bound (`h2 = h`),
written into a field, appended, or returned. The new structure releases it, and the
old name is spent: reading it afterwards is `error[read-after-move]`, which names the
line the value moved on. Anything else placed into a new structure is a COPY — a
parameter (the caller still owns it), a member of a container (`s.h`, `v[i]`), a
captured variable — and a copy of a droppable is `error[copy-of-droppable]` unless
the type says how a copy gets its own lease, below. Passing a value as an argument,
a `&` link and a view (`x = s.h`) make no second structure and are always legal.
The rules are `formal/heap.md` `(H-Move)`, `(H-Spent)` and `(H-Copy-Refuse)`.

### Copying one — `OpCopy`

A type that CAN have two live copies — a reference-counted buffer, a read-only handle
that can be reopened — declares `fn OpCopy(self: T)`. A copy then copies the bytes
and runs `OpCopy` on the NEW structure, which takes its own lease there; each copy is
released once, at its own death.

```loft
struct Buf { id: integer }
fn OpDrop(self: Buf) { release(self.id); }
fn OpCopy(self: Buf) { retain(self.id); }   // the copy holds a reference of its own

fn keep(b: Buf) {
  mine = b;          // a copy: `OpCopy` runs on `mine`
}                    // <- `release` runs for `mine`; the caller's `b` releases later
```

Like `OpDrop`, it takes only `self` and answers nothing. A struct whose members
declare `OpCopy` gets a synthesized cascade: its own `OpCopy` first, then its
members'. A copy is refused if ANY droppable inside the type lacks `OpCopy`. A move,
a vector's growth, a view and an argument run no hook — and neither does a copy the
compiler skips altogether together with its release, which it may: never rely on the
hook running for a particular copy, only on each copy that exists holding a lease.

A container releases in this order:

1. its own `OpDrop`, if it has one — a wrapper may still need what it wraps, the
   way a connection says goodbye over the socket it is about to close;
2. then its fields, in reverse declaration order, each through its own type;
3. a collection field element by element, in element order.

Nesting needs no special case: each type releases its own members, so a struct
inside a struct inside a vector releases once, at the outermost owner's death.

What follows from all of this, and is worth knowing before you reach for it:

- **A drop cannot fail.** loft has no runtime errors (C80) and a rollback at
  scope end can fail for real — the connection dropped, the server went away —
  with no caller left to tell. So `OpDrop` may not return, and the compiler
  refuses one that tries. **Anything whose failure matters stays an explicit
  call**: `tx.commit()` answers, the closing brace does not. That asymmetry is
  the design.
- **A drop reaches the world, not its caller's data.** It receives only `self`,
  and construction COPIES the data — `Tx { journal: j }` holds a copy of `j` — so
  a drop cannot write back into a caller's loft-side collection. Its effect is
  I/O, or a resource it owns (a `#c` handle). That is exactly the intended use:
  libpq's `PQexec("ROLLBACK")` at a closing brace.
- **Order within a scope is reverse-declaration**, matching the existing free
  order, or a statement would outlive the transaction it belongs to.
- **A binding written inside an `if` block is hoisted to the function scope** —
  that is where loft frees it, so that is where it drops. A `for` body is a scope
  of its own, so a droppable made per iteration drops per iteration.
- **A value that was never created never drops.** The free is null-tolerant and
  a drop is not, so the call is guarded by the same liveness test the free
  performs internally.
- **In a library, the hook may be private** — and usually should be. Nothing
  calls `OpDrop` by name, so `pub` only widens the surface. The hook is looked up
  through the source that declares the TYPE, which is what makes a private one
  reachable: a library's symbols are module-scoped (@PLN102 C97), and both askers
  run after parsing, when the current source is the main program.

**Three things a drop does NOT do.** Each is a deliberate boundary, not an
omission:

- **Taking a value OUT of its owner does not release it.** `v.remove(i)`,
  `v[i] = other` and an overwritten FIELD `o.s = other` do not release the value that
  goes away: it leaks, and the program is otherwise correct. (A reassigned LOCAL is
  different — its record has one owner, the local, and the release runs there.) Releasing there would mean the runtime's free
  cascade calling back into your loft code, inside the one operation the heap
  invariant rests on, for a hook that by contract can neither fail nor answer. If
  you churn a collection of live resources, release the old element yourself
  before you replace it. The reasoning is
  [DESIGN_DECISIONS.md § C111](DESIGN_DECISIONS.md).
- **A keyed collection does not release its records.** A `hash` / `sorted` /
  `index` shares its records with the collection it is indexed from, so releasing
  through one would release somebody else's element. Keep droppables in a plain
  `vector`, or release them explicitly.
- **A value moves once.** `a = C { h: h }; b = C { h: h }` moves `h` into `a`, and
  the second line reads a spent name: `error[read-after-move]`. Build the second
  container from its own value, or give it a copy the type can lease (`OpCopy`).

**When to reach for it.** A drop pays for itself when the value owns something
the program cannot see — a `#c` handle, a lock, a file — and the release is
unconditional. It does not pay for loft-side data, which the ownership model
already frees. And it is a poor fit for anything whose release ORDER matters
against an explicit call: a scope end runs after the function body, so a cursor
whose connection is shut inside that body is still live when the shut happens.
@PLN138's `lazy_fetch` closes its cursor explicitly for exactly that reason.

Shipped as @PLN125 arc B, with the owner rule and the container cascade added by
@PLN139 (loft#849). `tests/scripts/pln125-b-drop.loft` is the hook's behaviour
matrix, with two unrelated consumers (a transaction and a lease) because a hook
with one user is a hook whose invariant is untested;
`tests/scripts/139-drop-cascade.loft` is the container half. The design reasoning
is [plans/23-db-clients/LIFETIME_AND_PROCEDURES.md](plans/23-db-clients/LIFETIME_AND_PROCEDURES.md)
and [plans/139-drop-cascade.md](plans/139-drop-cascade.md).

## Indexing — `x[i]` on a library type

A type that defines `OpIndex` is subscripted like a built-in collection:

```loft
struct Ring { data: vector<integer>, start: integer }

fn OpIndex(self: Ring, i: integer) -> integer {
  n = len(self.data);
  if n == 0 { return 0; }
  return self.data[(self.start + i) % n] ?? 0;
}

r = Ring { data: [10, 20, 30], start: 1 };
r[0]        // 20 — the ring's own offset, not `data[0]`
```

This is the `OpAdd` / `OpEq` precedent and nothing more: `x[i]` lowers to the
two-argument method call `OpIndex(x, i)`, so argument conversion, the heap-return
buffer and the ownership deps all apply because it IS a method call.

- **The index type is whatever the method declares.** `fn OpIndex(self: Row, name:
  text) -> integer` gives a row addressed by column name. There is no requirement
  that a subscript be an integer.
- **Out of range is the TYPE's answer**, not the language's — `OpIndex` is an
  ordinary method, so it decides what a miss means.
- **An interface can require it**, with the operator sugar spelled `op []`:

  ```loft
  interface Indexable {
    op [] (self: Self, i: integer) -> integer
    fn count(self: Self) -> integer
  }

  fn total<I: Indexable>(x: I) -> integer {
    s = 0;
    for i in 0..x.count() { s += x[i]; }
    return s;
  }
  ```

  Inside a generic ONLY the bounds may be relied on, so an unbounded `<I>` cannot
  be subscripted even when every type it is used with defines `OpIndex`.

**`OpIndex` reads.** `x[i] = …` is refused, and the message says so: a writing
counterpart is a separate decision — it needs its own method, and a decision about
whether `x[i] += 1` may then read-modify-write — so a type that must be written
through offers a setter (`x.set(i, v)`).

Shipped as @PLN125 arc C; `tests/scripts/pln125-c-index.loft` is the behaviour
matrix.

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
| Operator requirements | Not natively expressible | Via `OpCamelCase` method names |
| Multiple bounds | `[T A ∩ B]` | `<T: A + B>` — supported |
| Default methods | No | No |

The dispatch model (no vtables, static specialisation) aligns with Go 1.18+
generic constraints rather than classic Go interface values.

---

## Standard library interfaces

⚠ **The block below is the DESIGN, not what `default/01_code.loft` contains.** The file
ships a narrower set, and the reference's Generics chapter copied this list and so promised
operators the compiler refuses. What actually ships is:

```loft
pub interface Ordered   { op < (self: Self, other: Self) -> boolean }
pub interface Equatable { op == (self: Self, other: Self) -> boolean }
pub interface Addable   { op + (self: Self, other: Self) -> Self }
pub interface Numeric   { op * (self: Self, other: Self) -> Self
                          op - (self: Self) -> Self }              // unary negation
pub interface Scalable  { fn scale(self: Self, factor: integer) -> integer }
pub interface Printable { fn to_text(self: Self) -> text }
```

so `-` is not in `Addable`, `+` and `/` are not in `Numeric`, `Scalable` takes an INTEGER
factor through a method and answers `integer` rather than `Self`, and no built-in type
satisfies `Scalable` at all. The derived spellings come free: `>`/`<=`/`>=` from `<`, and
`!=` from `==`. `tests/scripts/the-reference-bounds-permit-what-it-lists.loft` holds the
permitted half and `tests/parse_errors.rs`'s `generic_bound_*` family the refused half. A
binary `a - b` under `Numeric` is REFUSED (loft#1274, fixed): `-` desugars to the same
`OpMin` name at both arities, and bound satisfaction now compares the SIGNATURE rather than
the name, so the unary negation no longer answers for the binary spelling. No built-in
interface offers binary subtraction — write it against a concrete type.

The design as originally written:

```loft
// Comparison.  `>`, `<=` and `>=` all DERIVE from `<`, so one operator is the whole bound.
pub interface Ordered {
  op < (self: Self, other: Self) -> boolean
}

// Equality.  `!=` derives from `==` for the same reason.
pub interface Equatable {
  op == (self: Self, other: Self) -> boolean
}

// Addition, returning the same type.
pub interface Addable {
  op + (self: Self, other: Self) -> Self
}

// Multiplication, and UNARY negation.
pub interface Numeric {
  op * (self: Self, other: Self) -> Self
  op - (self: Self) -> Self
}

// BINARY subtraction — a bound of its own, see the note below.
pub interface Subtractable {
  op - (self: Self, other: Self) -> Self
}

// Integer scaling, as a METHOD rather than `op *` — see the note below.
pub interface Scalable {
  fn scale(self: Self, factor: integer) -> integer
}

// Text conversion — for generic print/log helpers.
pub interface Printable {
  fn to_text(self: Self) -> text
}
```

Built-in types (`integer`, `single`, `float`, and for the two comparison bounds `text` and
`boolean`) satisfy these through their existing operator definitions.

**The list is narrower than it looks, and deliberately so.** Each bound names the FEWEST
operators the derivations need: `Ordered` carries only `<` because `>`, `<=` and `>=` are
derived from it, and `Equatable` only `==` for the same reason. An interface demanding every
spelling would break every user type that implements the minimum.

⚠ **`Numeric`'s `-` is UNARY negation; binary subtraction is `Subtractable`.** The two share
one desugared name — `-` becomes `OpMin` at either arity — so they are two SIGNATURES of one
name, and a bound-method stub is keyed by `(name, arity)` to keep them apart. Write
`<T: Subtractable>` for `a - b`, `<T: Numeric>` for `-a`, and `<T: Numeric + Subtractable>`
for both; the last is two interfaces on one variable that each name `OpMin`, which is the
shape that needed the arity in the key (loft#1275).

Subtraction is a separate bound rather than a third requirement on `Numeric` because adding
one would take satisfaction away from every user type that provides `OpMul` and unary `OpMin`
today — `(G-Sat)` is structural, so a new requirement is a breaking change
([COMPATIBILITY.md](COMPATIBILITY.md)).

**Your own interfaces may declare both arities, in ONE interface or one per interface.** An
interface is a set of SIGNATURES (`(G-Iface)`), so

```loft
interface SubNeg {
  op - (self: Self, other: Self) -> Self
  op - (self: Self) -> Self
}
```

compiles and both arities are reachable from a `<T: SubNeg>` body. Two interfaces each
declaring one arity work too, including when both generics spell their type variable `T` — a
generic header binds its own type variable (`(G-Gen)`, loft#1300 / loft#1301).

⚠ **A CONCRETE type still provides one arity of a name, not both.** A method key carries the
receiver and the method name and no arity, so `fn OpMin(self: Money)` and
`fn OpMin(self: Money, other: Money)` collide as a redefinition. A user type therefore
satisfies `Numeric` or `Subtractable`, not both — which is the other half of why they are
separate bounds. Asking a type for the arity it does not have is a compile error naming the
interface (*"'Money' does not satisfy interface 'Subtractable': missing OpMin"*), not a
silently dropped operand.

`Scalable` scales through a `scale` METHOD rather than `op *` for the neighbouring reason: it
would share `Numeric`'s `*` at the SAME arity, and same-name same-arity requirements from two
interfaces are one stub, so the second is taken to agree with the first.
`text` satisfies `Ordered` and `Equatable`. No extra declarations are needed.

**Stdlib functions converted from native to bounded-generic loft** (depends on I8):

| Function | Bound | Notes |
|---|---|---|
| `sum_of<T: Addable>` | `Addable` | first-element init; null for empty vector |
| `min_of<T: Ordered>` | `Ordered` | first-element init; null for empty vector |
| `max_of<T: Ordered>` | `Ordered` | first-element init; null for empty vector |

---

## Implementation steps

Each step is independently compilable and testable. Steps I1–I6 are the core;
I7–I10 add usability and standard library support.

---

### I1 — Lexer: add `interface` keyword

**File:** `src/lexer.rs`

Add `"interface"` to the `KEYWORDS` static slice. After this step,
`interface` is tokenised as `Token("interface")` instead of
`Identifier("interface")`, making it available as a reserved keyword for
the parser.

**Test:** parsing a file that uses `interface` as an identifier should produce
a keyword-conflict error (same as `struct`, `fn`, etc.).

---

### I2 — Data: add `DefType::Interface` and `Definition.bound`

**Files:** `src/data.rs`

**2a.** Add a new variant to `DefType`:

```rust
pub enum DefType {
    // ... existing variants ...
    /// An interface declaration: a named set of required method signatures.
    /// Child definitions (via parent links) are the required method stubs.
    Interface,
}
```

**2b.** Add a `bound` field to `Definition`:

```rust
pub struct Definition {
    // ... existing fields ...
    /// For Generic functions: the def_nrs of all required interfaces (empty = no bounds).
    pub bounds: Vec<u32>,
}
```

Initialise `bounds` to `vec![]` in `Definition`'s constructor. Using a `Vec`
from the start means multiple bounds (`<T: A + B>`) requires no data model
change later — only the parser needs extending.

**Conflict detection:** if two bounds in `bounds` declare a method with the
same name but different signatures, emit an error at the `fn` declaration site:
`"interfaces A and B both declare method foo with conflicting signatures"`.
This is checked once when the bounds are resolved in the second pass.

**Test:** `Definition` constructs with `bounds = vec![]` without affecting
existing behaviour. A generic function with two bounds stores two entries.

---

### I3 — Parser first pass: parse interface declarations

**File:** `src/parser/definitions.rs`

Add a `parse_interface(&mut self) -> bool` method, called from
`parse_file`'s top-level loop alongside `parse_struct`, `parse_enum`, etc.

```
interface Ident { fn_signature* }

fn_signature = "fn" Ident "(" param_list ")" [ "->" type ] ";"
               // no body — ends with ";" or "}"
```

First pass actions:
1. Consume `interface`.
2. Read the interface name (must be `CamelCase`; emit error otherwise).
3. Call `data.add_def(name, pos, DefType::Interface)` to register it.
4. Parse each method signature. For each:
   - Call `data.add_def(method_name, pos, DefType::Function)` with
     `parent = interface_def_nr`.
   - Store parameter types and return type in the `Definition.attributes`
     or `Definition.returned` fields (same layout as a regular function stub).
   - `Self` in parameter/return types is stored as `Type::Unknown(interface_def_nr)`
     as a placeholder; it is resolved to the concrete type at instantiation.
5. Skip the body (no second-pass IR generation for interfaces).

**Test:** a file with a valid interface declaration parses without error.
An interface with a duplicate name emits the existing "already defined" diagnostic.

---

### I4 — Parser first pass: parse `<T: Bound>` syntax

**File:** `src/parser/definitions.rs`, inside `parse_function`.

The existing generic parsing detects `<T>` at the function name:

```rust
// Current code (simplified):
if lexer.has_token("<") {
    type_var_name = lexer.identifier();
    lexer.token(">");
    is_generic = true;
}
```

Extend this to optionally read `: A + B + ...` after the type variable:

```rust
if lexer.has_token("<") {
    type_var_name = lexer.identifier();
    let mut bound_names: Vec<String> = vec![];
    if lexer.has_token(":") {
        bound_names.push(lexer.identifier());       // first bound
        while lexer.has_token("+") {
            bound_names.push(lexer.identifier());   // additional bounds
        }
    }
    lexer.token(">");
    is_generic = true;
    // ...
    if !bound_names.is_empty() {
        self.pending_bounds = bound_names;   // resolved in second pass
    }
}
```

In the second pass, resolve each name in `pending_bounds` via
`data.def_nr(&name)` and push the result into `definition.bounds`. If any
name does not resolve to a `DefType::Interface`, emit "unknown interface".
After all bounds are resolved, run conflict detection (see I2).

**Test:** `fn foo<T: Ordered>(...) { ... }` stores one bound.
`fn foo<T: Ordered + Printable>(...) { ... }` stores two bounds.
`fn foo<T>(...) { ... }` stores zero bounds. Unknown interface name errors.

---

### I5 — Type resolution: validate interface bodies

**File:** `src/typedef.rs`, inside `actual_types` or a new `check_interfaces`.

After type resolution, iterate over all `DefType::Interface` definitions.
For each required method (child definitions with the interface as parent):

- Resolve all `Type::Unknown(interface_def_nr)` (the `Self` placeholder) to
  a sentinel that the satisfaction checker in I6 will substitute.
- Validate that all other types in the signature are known and concrete.
- Emit errors for unresolved types in interface bodies.

No bytecode is generated for interface definitions themselves.

**Test:** an interface with an unknown type in a method signature emits a
clear "unknown type" error. An interface with all valid types passes silently.

---

### I6 — Satisfaction checking at instantiation

**File:** `src/parser/definitions.rs`, inside the generic specialisation logic.

Currently, when a generic function `fn foo<T>(...)` is called with a concrete
`T = Point`, the compiler looks for or creates a specialised copy named
`foo_Point`. Extend this to also run a satisfaction check:

```rust
fn check_satisfaction(
    data: &Data,
    concrete_type: u32,   // def_nr of the concrete struct/enum
    bound: u32,           // def_nr of one required interface
    call_pos: &Position,
    diagnostics: &mut Diagnostics,
) {
    // Collect required method signatures from the interface's children.
    for child in data.children_of(bound) {
        let concrete_fn = data.find_method(child.name, concrete_type);
        if concrete_fn == u32::MAX {
            diagnostics.error(
                call_pos,
                &format!(
                    "{} does not satisfy interface {}: missing fn {}",
                    data.def(concrete_type).name,
                    data.def(bound).name,
                    child.name,
                )
            );
        } else {
            // Check return type and param types match (with Self → concrete_type).
        }
    }
}

// Call once per bound, per (concrete_type, generic_fn) pair:
for &bound in &definition.bounds {
    check_satisfaction(data, concrete_type, bound, call_pos, diagnostics);
}
```

Cache results per `(concrete_type, generic_fn)` pair to avoid re-checking on
every call. The cache key covers all bounds together; if any bound fails the
whole instantiation fails.

**Test:** calling `max_of([Priority{...}])` where `Priority` has `less_than`
compiles cleanly. Calling `max_of([Thing{...}])` where `Thing` lacks
`less_than` emits the "does not satisfy" error.

---

### I7 — Allow bounded method calls on T

**File:** `src/parser/control.rs` or `src/parser/objects.rs` — wherever
method calls on generic `T` currently emit the
`"generic type T: method call requires a concrete type"` error.

When `x.method(args)` is encountered and `x` is of generic type `T`:

1. Collect `definition.bounds` for the enclosing generic function.
2. Search each bound's children for a method named `method`. Stop at the
   first match.
3. If found in any bound: allow the call. The method resolves to the concrete
   implementation when the specialised copy is compiled.
4. If not found in any bound: emit the existing "method call requires a
   concrete type" error, listing all bounds that were searched.

**Test:** inside `fn find_max_and_log<T: Ordered + Printable>`, both
`result < item` and `item.to_text()` compile. A method not in either bound
still errors.

---

### I8 — Operator interfaces

**File:** `src/parser/operators.rs` — wherever operators on generic `T`
currently emit the `"operator '+' requires a concrete type"` error.

When an operator expression is encountered with a `T`-typed operand, the
procedure is:

1. Map the operator token to its `OpCamelCase` name
   (e.g. `+` → `"OpAdd"`, unary `-` → `"OpNeg"`).
2. Search **each bound** in `definition.bounds` for a child method named
   `"OpAdd"` (or the relevant name). Stop at the first match.
3. If not found in any bound: emit the existing "operator requires concrete type" error.
4. If found: validate the operand types against the interface signature:
   - For binary operators: the right-hand operand must match the declared
     second-parameter type (either `Self` → `T`, or a concrete type like
     `float`). Emit a type-mismatch error if they differ.
   - For unary operators: no second operand to check.
5. Determine the result type from the interface method's declared return type:
   - `Self` in return position → `T` (the generic type variable).
   - Any concrete type (e.g. `boolean`, `float`) → that concrete type.
   Emit the operator as a `Call(op_nr, args)` IR node with this result type.

This result-type propagation is the key addition over the boolean allow/deny
check: the generic body's type checker needs to know whether `x / y` produces
`T` or `float` in order to type-check subsequent expressions.

This requires the operator-name mapping (operator token → `OpCamelCase`)
which already exists in `src/parser/operators.rs`. It only needs to be
made accessible at the check site.

**Covered cases:**
- `T + T -> T` — same-type binary, Self return
- `T < T -> boolean` — same-type binary, concrete return
- `T * float -> T` — mixed-type binary, Self return
- `T / T -> float` — same-type binary, concrete return
- `-T -> T` — unary, Self return
- `T += T` — desugars to `T = T + T` before this stage; handled by OpAdd

**Test:** inside `fn sum_of<T: Addable>`, `total = total + item` compiles
and the result has type `T`. Inside `fn average<T: Averageable>`,
`total / len(v)` compiles and the result has type `float`. Inside
`fn id<T>` (no bound), `total = total + item` still errors.

---

### I9 — Standard library interfaces

**File:** `default/01_code.loft`

Add interface declarations at the top of the file, before the operator
definitions they describe:

```loft
pub interface Ordered {
    fn OpLt(self: Self, other: Self) -> boolean
    fn OpGt(self: Self, other: Self) -> boolean
}

pub interface Equatable {
    fn OpEq(self: Self, other: Self) -> boolean
    fn OpNe(self: Self, other: Self) -> boolean
}

pub interface Addable {
    fn OpAdd(self: Self, other: Self) -> Self
}

pub interface Printable {
    fn to_text(self: Self) -> text
}
```

Convert the currently-native `sum_of`, `min_of`, `max_of`, `any_of`, `all_of`
from native Rust implementations to bounded generic loft functions where
feasible. Those that require operator access (`sum_of`, `min_of`, `max_of`)
depend on I8 landing first.

**Test:** existing tests for these stdlib functions pass unchanged.
A new test shows a user-defined type satisfying `Ordered` and being passed
to `max_of`.

---

### I10 — Diagnostics

**Files:** `src/diagnostics.rs`, satisfaction check in I6.

Polish the error messages from the satisfaction check:

```
error[I01]: type `Priority` does not satisfy interface `Ordered`
  --> example.loft:14:5
   |
14 |     max_of(priorities)
   |     ^^^^^^ `Ordered` required by this bound on `T`
   |
   = missing: fn OpLt(self: Priority, other: Priority) -> boolean
   = missing: fn OpGt(self: Priority, other: Priority) -> boolean
   = help: add `fn OpGt(self: Priority, other: Priority) -> boolean { ... }`
```

Also add a diagnostic for using an interface name as a type
(`x: Ordered = ...`) with a clear "interfaces cannot be used as types" message.

**Test:** a deliberately unsatisfied call produces the formatted multi-line
error. Using an interface as a variable type produces the specific message.

---

## Open questions

**Q1: Multiple bounds** — resolved. `<T: A + B>` is supported from the start.
`Definition.bounds` is `Vec<u32>` from I2 onward; the parser (I4) reads
`+`-separated names in a loop; satisfaction (I6) and lookup (I7, I8) iterate
over all bounds. The incremental cost over a single-bound design is ~40 lines.

**Q2: Operator method naming in interfaces** — requiring users to write
`fn OpLt(self: Self, other: Self) -> boolean` is consistent with internals
but surprising to users expecting `<` syntax. Consider allowing
`op < (self: Self, other: Self) -> boolean` as syntactic sugar in interface
bodies that desugars to `fn OpLt`. This is a purely cosmetic change that
can be added without altering the data model.

*Mitigation:* Add `op <op> (self: Self, ...) -> T` sugar in `parse_interface`
(`src/parser/definitions.rs`) that maps the operator token to its `OpCamelCase`
name and stores it as an ordinary method stub. Zero data model impact; the
desugaring happens before any downstream step sees the signature.

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

*Mitigation (phase 2):* After the primary `T.OpMul(concrete)` lookup
succeeds, allow declaring `fn OpMul(factor: float, self: Self) -> Self` in the
interface with `factor` as the first parameter — but this requires either
commutativity to be declared explicitly in the interface, or a second-pass
fallback lookup. Add a design note before implementing to avoid ambiguity with
existing overload resolution.
