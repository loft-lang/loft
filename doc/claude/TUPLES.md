
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Tuple Design

> **Status: completed in 0.8.3.** T1.1–T1.7 implemented; tuple-returning
> functions (T1.4) and LHS destructuring remain deferred.

Tuples are anonymous, fixed-arity, stack-allocated compound value types for
returning multiple values without defining a named struct.

---

## How a tuple RETURN travels

A tuple returned from a function stays a value: under `--native` it is a real Rust
tuple (`-> (f64, f64, f64)`, returned in registers), and the interpreter carries it
in the frame. Two shapes are the exception and travel as a **`Reference` to the
synthetic `__tuple<…>` struct** instead — the same record a stored tuple uses:

1. **Any element with a lifetime concern** — Text, Reference, Vector, Enum-struct, a
   keyed collection, RefVar, or a nested tuple containing one (`data::has_lifetime_concern`).
   Those elements need store-side ownership tracking, which the existing
   `ref_return` / `text_return` transfer machinery already provides for struct returns
   (@P234).
2. **A `par(...)` worker's return wider than 8 bytes.** Par dispatch carries a worker
   result home through per-route buffers that cover ≤8-byte primitives, text, fn-refs
   and references and nothing else, so a bare `(integer, integer)` has nowhere to ride.
   Boxing puts it on the reference route. This is decided per FUNCTION, from the set of
   defs seen as a worker (`Parser::par_worker_defs`), not per tuple shape.

The distinction is worth keeping honest because the boxing is not free: it claims and
frees a store record on **every call**. loft#808 measured a `(float, float, float)`
helper at 728 ms boxed against 129 ms as a value on `--native-release` — the identical
arithmetic, ~5.6x, entirely from crossing a function boundary. Boxing every tuple
return is what that issue was.

A tuple containing a `Type::Function` element is never boxed by the size rule (P196):
the synthetic-struct wrapping breaks at the assignment site, where the field type stays
a bare tuple while the value has become a reference.

---

## The two spellings of one tuple type

Those two ways of travelling are two **spellings of the same loft type**, and both are
reachable from ordinary source:

| | `Type::Tuple(elems)` — STACK | `Reference(__tuple<…>)` — STORED |
|---|---|---|
| runtime | elements on contiguous stack slots | a 12-byte DbRef at the elements' bytes |
| `.0` | `OpTupleGet` / Rust `.0` | `OpGetFloat(ref, 8)` — read at the offset |
| native | `(f64, f64, f64)` | `DbRef` |
| ownership | none — copied by value | a store, with deps and frees |
| written by | a literal, a value-tuple return | a `vector<(…)>` element, a boxed return |

A `vector<(…)>` loop variable carries the stored spelling (`for_type`, P189b) so element
access reads at offsets instead of decoding DbRef bytes as values. So does a
heap-carrying tuple return. Both then arrive wherever such a value is *used* as the plain
type — a call argument, a `return`, a declared local, a vector append.

**`Parser::convert` is where the two meet.** It answers such a pair by UNBOXING the value:
`unbox_tuple_from_dbref` reads each element at its stored offset and rebuilds the stack
tuple, exactly as `v[i]` already does. Ask `Parser::unboxes_stored_tuple` rather than
comparing the two spellings by hand.

Accepting the pair as merely EQUAL would be worse than rejecting it — the slot would take
12 bytes of DbRef where the reader expects the elements, so the conversion has to be a
real one. That is also why a caller must not keep classifying on the pre-`convert` type:
`parse_return`'s delivery classifier did, saw a `Reference` where the value had become a
stack tuple, and materialised it with `OpCopyRecord`, which read the tuple's own float
bytes as a DbRef (loft#822).

A `vector<(…)>` element is written the same way it is read — per element, at the
synthetic struct's own offsets (`emit_tuple_set_ops`), for both `v += [t]` and `v[i] = t`.
The write picks its arm from the SOURCE's representation, never from the spelling the slot
happens to carry: a stack tuple writes element by element, a source that is already a
promoted `Reference(__tuple<…>)` copies the record in one op.

Note that `v[i] = t` evaluates its index ONCE even though the three element writes each
carry the address expression — `hoist_index_arg` binds it to a local first, so
`v[bump(c)] = (1.0, 2.0, 3.0)` calls `bump` once, as the same write to a `vector<integer>`
does.

---

## Comparison

`==`, `!=`, `<`, `<=`, `>`, `>=` all work between two tuples of the same arity. Ordering is
**lexicographic**: the first element decides, and later elements are consulted only while
the earlier ones are equal.

```loft
(1, 9) < (2, 0)      // true  — the first element decides
(1, 9) < (1, 10)     // true  — it ties, so the second decides
(1, 9) < (1, 9)      // false — identical is not strictly less
(1, 9) <= (1, 9)     // true
(1, "abc") == (1, "abc")   // true — text compares by VALUE, not by identity
```

The comparison lowers to the ELEMENT types' own operators (`Parser::tuple_compare`), not to
a tuple opcode. Three things follow, and they are the reason it is built that way:

- Every element type that can already be compared can be compared inside a tuple — text by
  value, a scalar enum by discriminant, a nested tuple by recursing into the same rule.
- An element type with no such operator reports **itself**: `(false, 1) < (true, 0)` says
  *"No matching operator `<` on `boolean` and `boolean`"*, naming the element the author has
  to change rather than the tuple around it. A tuple never invents an ordering its elements
  do not have.
- Both backends inherit it with nothing to add.

Each operand is evaluated **once**. Every element read names its side again, so a side that
does work — a call, an index read — is bound to a local first; a side that is already a
tuple local is used as it stands.

Both spellings compare the same way: a `vector<(…)>` element is unboxed to its stack form
first, so a loop variable compares against a literal, a local, or another element. This is
also why the lowering runs BEFORE `call_op`'s operator loop — that loop would match
`OpEqRef` for two stored tuples and answer whether they are the same record, not whether
they hold the same values.

Comparing tuples of different arity is not a tuple comparison at all: it falls through to
`No matching operator '==' on '(integer, integer)' and '(integer, integer, integer)'`,
which says more than an arity count would.

---

## As a collection key

A tuple key field in `hash<T[k]>`, `sorted<T[k]>` or `index<T[k]>` is a **compound key
spelled as one field**: it behaves exactly as its elements spelled out as separate key
fields, in order. `sorted<Cell[pos]>` with `pos: (integer, integer)` orders on element 0 and
breaks ties on element 1, the same as `sorted<Cell[x, y]>`; a nested tuple flattens the same
way, so the flat element order is the comparison order all the way down. `-` reverses the
whole tuple. Look one up by writing the tuple — `h[(3, 4)]` — from a literal, a local, or a
`vector<(…)>` element.

`Stores::key_contents_for_field` is the one place a key field's arity is decided, and it has
to stay that way. **Both** the comparison descriptors (`determine_keys_for`) and the lookup's
stack arity (`get_keys`) read it, and `read_key` pops one stack value per entry — so a list
one short leaves a key value on the stack and the very next `get_stack::<DbRef>()` reads it
as the collection. That is loft#720 (`spatial<T[x,y]>`, whose arity list had gone missing),
and a tuple key reproduced it exactly: `h[(3, 4)]` looked up in store #4. Deriving the arity
twice is what let the two disagree.

Not expanding it was worse than the panic, because it was quiet: the whole tuple took the
catch-all descriptor, so only element 0 was ever compared, and a `sorted` holding three cells
at `(1,9)`, `(2,0)` and `(1,2)` reported `len == 3`… after silently dropping one of the two
that shared an element 0.

`trie<T[field]>` is the exception, and refuses a tuple key. It keys on the BYTES of one text
field, and an order-preserving byte encoding of a tuple is a format decision (it would reach
the stored/paged trie image), not a descriptor change — so the refusal names `sorted` /
`index` / `hash` instead.

---

## Syntax

```loft
// Type notation — two or more element types
(integer, text)
(float, float, boolean)

// Function return
fn min_max(v: vector<integer>) -> (integer, integer) {
    (min_val, max_val)
}

// Literal
t = (1, "hello")

// Element access (zero-based integer literal)
a = foo()       // a: (integer, text)
b = a.0         // integer
c = a.1         // text

// Element assignment
a.0 = 5
a.0 += 3

// LHS deconstruction (deferred)
(lo, hi) = min_max(values)
```

---

## Memory layout

Each tuple is a contiguous stack region. Elements are laid out in declaration
order, each naturally aligned. Total size = sum of `element_size(T_i)`.

Text elements (`text`, `text not null`) use full `String` (24 bytes) when owned
by the tuple. Argument-passed text uses `Str` (16 bytes) per the standard
calling convention.

---

## Known limitations

| ID | Issue | Resolution |
|---|---|---|
| SC-1 | Text element use-after-free on return | Caller-allocated slots |
| SC-2 | Text double-free on tuple copy | Deep-copy text elements |
| SC-5 | LIFO store violation on scope exit | Reverse element free order |
| SC-7 | `not null` inaccessible for tuple integers | `integer not null` annotation |

Plan-14 closure (2026-05-11) validated 40 cells across 5 element
types (E1, E1n, E2, E3, E5) and 3 destinations (D1 local, D2 stack,
D3 struct field) under the cross-mode harness — every cell asserts
byte-identical output between interpreter and `--native`.  The two
shapes that were briefly gated behind P-issues are both closed
(@P250 loop-body destructure and @P251 fn-ref tuples in struct
fields, both fixed 2026-05-11 on both backends — see PROBLEMS.md);
no workaround is needed for either.

E4 (closure element) calling — `t.0(args)` and `s.field.0(args)` —
was the open phase-03 / phase-05 question; the layered fix (20-byte
fn-ref tuple-element layout in six codegen sites + `__fn_ref_tmp`
skip_free) closed it for D1 / D2 (@P249), and the struct-field call
shape closed with @P251.

---

## Non-goals

Named tuple fields, single-element tuples, tuple iteration,
whole-tuple formatting, variadic tuples — all compile errors.  Use
named structs or element-by-element access instead.

**Tuples in struct fields** were a non-goal in 0.8.3; Plan-06
phase 4d LIFTED that restriction and Plan-14 phase 05 validated
the lift across the matrix (5 + 1-omitted D3 cells green).  Tuple
struct fields now lay out their elements inline using the
synthetic `__tuple<…>` struct's positions; access via `s.field.i`
goes through the same `Type::Tuple` arm of `set_field_check` /
`get_val` that the matrix exercises.

---

## Deferred work

- **`&tuple` with owned elements** — per-element DbRef expansion is
  still pending; tuple values are passed by value or via the
  synthetic `__tuple<…>` struct's DbRef.
- **The `vector<(…)>` read cursor still materialises a copy** — the work-ref
  `unbox_tuple_from_dbref` reads through is born with `Deps::none()`, which both backends
  read as "owner", so every `vector<(…)>` element read allocates and frees a record where
  three loads would do. This is a PERFORMANCE residue: the two *correctness* failures filed
  with it (loft#823) are closed, and neither was a tuple defect.

  Their root causes are worth keeping straight, because the filed scope named neither.
  `v[oob] ?? d` answered uninitialised bytes on `--native` because the materialise arm
  allocated before asking whether the element was there, and absence has two spellings —
  see [DATABASE.md § DbRef](DATABASE.md). `g = p` inside `for p in v` SIGSEGV'd because the
  O-B1 last-use move transferred a store the loop variable never owned; that one is not
  tuple-specific at all — a `vector<struct>` loop does the same, see
  [PERFORMANCE.md § Status after O-B1](PERFORMANCE.md).

  Making the cursor a real BORROW would retire the remaining allocation, and still needs the
  dep to survive `scopes.rs` — a design pass rather than a patch. `Deps::none()` conflating
  *owns* with *nobody said* is the fact underneath it.

T1.4 (tuple-returning functions), tuple LHS destructuring, and
tuple patterns in match all shipped in 0.8.3 (T1.9) and were
re-validated by Plan-14's 40 cells.  T1.8c (struct-ref tuple
element semantics) closed in Plan-14 phase 04 with the MOVE-
semantics decision.

---

## See also

- [LOFT.md](LOFT.md) — language reference
- [INTERMEDIATE.md](INTERMEDIATE.md) — `Type`/`Value` enums, stack layout
- [SLOTS.md](SLOTS.md) — slot assignment for owned elements

---

# Tuple Destructuring in `match` (T1.9)

Design for `match` expressions whose subject is a `Type::Tuple`.

## Contents
- [Current State and Dependencies](#current-state-and-dependencies)
- [Goals](#goals)
- [Syntax](#syntax)
- [Element Pattern Forms](#element-pattern-forms)
- [Exhaustiveness](#exhaustiveness)
- [IR Lowering](#ir-lowering)
- [Implementation Plan](#implementation-plan)
- [Edge Cases](#edge-cases)
- [Test Plan](#test-plan)

---

## Current State and Dependencies

`Type::Tuple` is fully implemented (T1.1–T1.7, 0.8.3). The following works today:

```loft
t: (integer, text) = (42, "hello")
(a, b) = t              // LHS destructuring — works
x = t.0                 // element access — works
t.1 = "world"           // element assignment — works
```

A destructuring LHS is a **binding position**, so its names follow the same rule as
`name = expr`: a name that a definition also uses still mints a local.

```loft
(a, trim) = pair()      // `trim` is a local, as `trim = 7` always was
(a, chr) = pair()       // so is `chr` — and `chr(65)` still calls the function
```

Both forms answer identically, which is the rule to keep: a binding position mints a
local whatever else carries that name, and the function stays reachable as a call —
values and functions are separate namespaces (loft#852, [LOFT.md § Shadowing and
qualified names](LOFT.md)). Before loft#756 only `name = …` was recognised as a
binding, so a destructured element resolved to the definition instead and the author
got *"Tuple destructuring requires plain variable names"* about a name that is exactly
that, plus an arity error counting the names that had been dropped.
`Parser::at_binding_name` is the single predicate every binding form reads — three of
them now: `name = …`, the typed local `name: T = …`, and the destructured element. The
typed local was described as living there from the day loft#756 closed, but was in fact
a second predicate spelled beside it at ONE of the three call sites, and loft#1079 is
what that cost: a `both:` function registers a definition under its raw spelling as well
as `n_<name>`, the site WITHOUT the extra predicate matched it first, and
`exp: integer = 5` failed to parse while `exp = 5` was fine. Folding it in is what stops
the sites disagreeing again.

An annotated destructuring LHS — `(a, b): (integer, text) = t` — is not a form loft has;
it is refused for every name, including one nothing else declares. The annotation goes
on the tuple being destructured, as in the block above.

`parse_match` dispatches on the subject type; a `Type::Tuple` subject goes to
`parse_tuple_match`.  A `&(…)` binding takes a tuple pattern too, in both of its
representations (`formal/tuples.md` `(T-Ref-Rep)`): the subject becomes the tuple of its
element reads through the reference — `TupleGet` for a stack-backed tuple, the record's own
fields for a `__tuple<…>` RECORD — and a heap member read that way borrows the binding.

**Dependency on T1.8:** Tuple-returning functions (`-> (A, B)`) are deferred as T1.8.
Tuple match on a function call result (`match foo() { ... }`) requires T1.8a first.
Tuple match on a local variable, parameter, or literal is independent and can land now.

---

## Goals

- Allow `match` to destructure and dispatch on tuple subjects.
- Support all element pattern forms that exist for scalar match: wildcards, binding
  variables, literals, ranges, or-patterns, and `null`.
- Support nested tuple patterns for tuple-valued elements.
- Exhaustiveness: a compile error when no arm is total (catches all cases).
- Guards work the same way as for enum and struct match: the captures are assigned
  before the guard runs, so a guard can test them.

---

## Syntax

```
tuple-match     ::= 'match' tuple-expr '{' tuple-arm+ '}'

tuple-arm       ::= tuple-pattern [ guard ] '=>' expression

tuple-pattern   ::= '_'                                  // total wildcard
                  | '(' elem-pattern { ',' elem-pattern } ')'

elem-pattern    ::= '_'                                  // element wildcard
                  | identifier                           // binding variable
                  | literal                              // exact match
                  | literal '..' literal                 // range (exclusive)
                  | literal '..=' literal                // range (inclusive)
                  | elem-pattern '|' elem-pattern        // or-pattern
                  | '(' elem-pattern { ',' elem-pattern } ')'  // nested tuple
                  | 'null'                               // null match
                  | Variant [ '{' field { ',' field } '}' ]  // enum element: tag test, payload bound

guard           ::= 'if' expression
```

Arms are separated by newlines; `;` is also accepted as a separator.

### Examples

```loft
t: (integer, text) = (42, "hello")

// Basic binding and wildcard
match t {
    (0, msg)  => println("zero: {msg}")
    (n, "")   => println("empty text at {n}")
    (n, msg)  => println("{n}: {msg}")
}

// Range on first element — DESIGNED, DOES NOT PARSE TODAY (see the gap list below)
match t {
    (0..10, _) => println("single digit")
    (10..100, name) => println("two digits: {name}")
    _ => println("large or negative")
}

// Or-pattern in element position — DESIGNED, DOES NOT PARSE TODAY (see below)
match t {
    (1 | 2 | 3, _) => println("one, two, or three")
    (0, _) | (_, "") => println("zero or empty")  // ERROR — arm-level | not supported
    _ => println("other")
}

// Nested tuple
coords: ((float, float), boolean) = ((1.0, 2.0), true)
match coords {
    ((0.0, 0.0), _)    => println("origin")
    ((x, y), true)     => println("active: {x},{y}")
    ((x, y), false)    => println("inactive: {x},{y}")
}

// Guard
match t {
    (n, msg) if n > 100 => println("large: {msg}")
    (n, msg)            => println("normal: {n} {msg}")
}
```

### Element patterns that are DESIGNED but do not parse yet (verified 2026-08-09)

Both forms below are shown as working in the examples above; neither is implemented.

The gap stayed invisible because tuple patterns fall through the seam between the two
doc-tests that should cover them: `tests/docs/29-match.loft` has 33 `match` expressions
and **no tuple arm** (and no range arm either), while `tests/docs/28-tuples.loft` has
**no `match` at all**. Nothing in `tests/scripts/` covers a tuple element pattern either.
The pair below is the coverage that would have caught it.

| written | today |
|---|---|
| `match t { (0..10, _) => … }` | `error: Expect token }` |
| `match t { (1 \| 2 \| 3, _) => … }` | `error: Expect token \|` |

Neither is a range/or limitation as such: the same forms work on a **scalar** subject —
`match n { 0..10 => "single digit", _ => "big" }` prints `single digit`. What is missing is
the element position inside a tuple pattern.

### What is NOT supported (T1.9 scope)

- **Or-patterns at arm level**: `(1, _) | (2, _) => ...` — only `|` inside an element
  position is supported (reuses existing scalar or-pattern). A full arm-level or-pattern
  requires restructuring the arm-building loop (deferred to T1.10 if needed).
- **Rest patterns**: `(first, ..)` — tuple arity is fixed and known at compile time;
  no `..` rest needed (unlike vector/slice match). A `..` in any position is refused by
  name, and the message says what to write instead: *"a `..` rest pattern is not
  supported in a tuple pattern — a tuple's arity is fixed, so write every position, using
  `_` for the ones you do not bind"*. A pattern listing MORE elements than the subject has
  is refused the same way, naming the subject's arity.
  (Until [loft#832](https://github.com/loft-lang/loft/issues/832) both shapes HUNG the
  parser instead of being rejected.)
- **Guards**: `(a, _, _) if a > 10 => …` — supported on every tuple and vector/slice arm.
  The arm's captures are assigned before the guard runs, so the guard can read them; a
  false guard falls through to the next arm, and a guarded arm never counts toward
  exhaustiveness. The single exception is a **cursor** arm, where the pattern advances the
  shared cursor as part of binding: a guard failing after that would leave the cursor
  consumed and the next arm reading from the wrong position, so the combination is refused
  and the test belongs in the arm body.
- **Match on tuple-returning function calls**: `match foo() { ... }` — requires T1.8a
  (tuple function return convention). The subject must be a tuple variable or parameter.

---

## Element Pattern Forms

### Wildcard `_`

No condition generated. No binding. The arm does not become total from this element
alone, but a `_` at every position (or a top-level `_` arm) makes the arm total.

### Binding variable `name`

No condition generated. A new variable `name` of the element's type is declared and
bound to `tuple_var.i` at the start of the arm body.

```loft
(n, msg) =>          // n bound to t.0, msg bound to t.1
```

Binding a name that already exists in scope re-uses that variable if the type matches;
otherwise creates a new one (same rule as LHS destructuring).

A `_` identifier is treated as a wildcard, not a binding (consistent with the rest of
the language).

### Literal `42`, `"hello"`, `true`

Generates an equality condition: `tuple_var.i == literal`.

The literal type must be compatible with the element type (same rules as scalar match).

### Range `lo..hi` / `lo..=hi`

Generates a range condition: `tuple_var.i >= lo && tuple_var.i < hi` (exclusive) or
`tuple_var.i >= lo && tuple_var.i <= hi` (inclusive).

Reuses `parse_match_pattern` which already handles ranges for scalar match.

### Or-pattern `p1 | p2 | p3`

Generates an or-condition: `cond(p1) || cond(p2) || cond(p3)`.

Reuses the existing or-pattern infrastructure in `parse_match_pattern`.

### `null`

Generates a null check. Only valid for nullable element types.

### Nested tuple `(p1, p2)`

When the element type is itself a `Type::Tuple`, the pattern may be a nested
`(...)`. Recursive call to the element-pattern parser for each sub-element.

Generates a conjunction of sub-element conditions, ANDed into the outer arm condition.

### Enum variant `Variant` / `Variant { fields }`

When the element type is an enum (plain or struct-enum), a capitalised name is a VARIANT
pattern, never a binding: `(Fire, Wall)` tag-tests both positions, and `(Fire { id }, Wall
{ status })` also binds each payload field by name, in scope for the guard and the arm.  The
same view-or-copy rule as a top-level arm applies — a record or other heap payload is a view
of the matched value, a scalar payload is a copy (LOFT.md § Match expressions).  Plain-enum
variants take an or-pattern (`(Fire | Ice, _)`); struct-enum variants do not, so spell those
as separate arms.  The element and the top-level arm share one lowering
(`parse_field_sub_pattern`), which is what `(P-Point)` in `formal/matching.md` asks for: a
unit or struct variant is a point pattern over ONE value, and a tuple element is one value.

A capitalised name that is not a variant of the element's enum is refused by name (*'Bogus' is
not a variant of Kind*), as it is at a top-level arm; over an element with no variants at all
the message names the element's type.  Before this held, such a name fell through to the
binding branch and the arm matched every tuple in silence — see `formal/matching.md`
D-match-5.

---

## Exhaustiveness

Tuple values cannot be enumerated (unlike enum variants), so exhaustiveness is checked
via a simpler rule:

> **A tuple match is exhaustive if at least one arm is *total*.**

An arm is *total* when:
- It is a bare `_` wildcard, OR
- Every element position is either `_` or a plain binding variable (no condition).

A guarded arm (`if cond`) is never total — the guard may fail at runtime.

If no arm is total, a **compile-time error** is emitted:
```
error: tuple match is not exhaustive — add a wildcard arm `_` or an all-binding
       arm `(a, b, ...)` to cover all cases
```

This matches the exhaustiveness model for scalar match and vector/slice match.

---

## IR Lowering

### Subject storage

The subject tuple is stored in a temp variable to avoid re-evaluation and to give
`TupleGet` a stable `var_nr`:

```rust
let v = self.create_unique("match_subj", subject_type);  // u16 var_nr
self.vars.defined(v);
// preamble: Set(v, subject_expr)
```

### Per-arm IR shape

Each arm lowers to a block:

```
[
  // 1. Check arm condition (conjunction of element conditions)
  If(cond,
    Block([
      // 2. Bind variables: Set(name_var, TupleGet(v, i)) for each binding
      Set(n_var, TupleGet(v, 0)),
      Set(msg_var, TupleGet(v, 1)),
      // 3. Optional guard as inner if
      If(guard_cond, body, <next_arm>)
    ]),
    <next_arm>
  )
]
```

When no element conditions exist (total arm), the outer `If` is omitted and the body
runs unconditionally (with only bindings + guard if present).

### Example lowering

```loft
match t {
    (0, msg) => println("zero: {msg}")
    (n, _)   => println("{n}")
}
```

Lowers to:

```
tmp_v = t                          // Set(v, subject)
If(
  OpEqInt(TupleGet(v, 0), 0),      // t.0 == 0
  Block([
    Set(msg_var, TupleGet(v, 1)),  // bind msg
    println("zero: {msg_var}")
  ]),
  Block([
    Set(n_var, TupleGet(v, 0)),    // bind n (wildcard on t.1 — no binding)
    println("{n_var}")
  ])
)
```

### Nested tuple lowering

```loft
match coords {                    // coords: ((float, float), boolean)
    ((0.0, 0.0), _) => "origin"
    ((x, y), active) => "{x},{y},{active}"
}
```

Lowers to:

```
tmp_v = coords
If(
  OpAndBool(
    OpEqFloat(TupleGet(TupleGet(v, 0), 0), 0.0),   // coords.0.0 == 0.0
    OpEqFloat(TupleGet(TupleGet(v, 0), 1), 0.0)    // coords.0.1 == 0.0
  ),
  "origin",
  Block([
    Set(x_var,      TupleGet(TupleGet(v, 0), 0)),  // bind x
    Set(y_var,      TupleGet(TupleGet(v, 0), 1)),  // bind y
    Set(active_var, TupleGet(v, 1)),               // bind active
    "{x_var},{y_var},{active_var}"
  ])
)
```

**Note on nested TupleGet:** `TupleGet(TupleGet(v, 0), 0)` reads element 0 of the inner
tuple. The codegen for `TupleGet` already handles this by computing the byte offset of
the element within the outer tuple's stack slot. A nested `TupleGet(inner, i)` where
`inner` is another `TupleGet` chains the offset correctly.

### A nested element is a RECURSION, and it needs one helper per direction

A tuple's stack layout FLATTENS — `element_stack_size(Tuple)` sums one
`aligned_stack_step` slot per element, so `((1,4),5)` puts its leaves at exactly the
offsets `(1,4,5)` would. Every site that moves a WHOLE tuple therefore walks leaves,
and a tuple-typed element is simply a recursion into the same walk.

There are two directions and one helper each, both in `src/state/codegen.rs`:

| direction | helper | used by |
|---|---|---|
| push leaves onto the eval stack | `emit_tuple_var_push_recursive` | `TupleGet`, and `generate_var`'s `Type::Tuple` arm |
| pop leaves into a variable's slots | `emit_tuple_var_pop_put` / `emit_tuple_put_ops` | `set_var`, tuple literals |

**Route every new whole-tuple site through them — never re-inline the per-element
match.** The same defect has now landed three times from exactly that: @P212 (a tuple
literal containing a tuple panicked "unsupported elem" in `gen_set_first_at_tos`),
then the `TupleGet` side, and then loft#817 — `generate_var` kept a hand-rolled copy
of the push loop that had no `Type::Tuple` case, so **reading a whole nested tuple
variable was an ICE** (`r = ((1,4),5); r`). That ICE is what bounded loft#816's return
hoist, which is why a nested tuple return with a store local silently answered the
zero initialiser on native while the interpreter was right. The arm is now a
delegation, and the emitted bytecode for every flat shape is byte-identical to before.

---

## Implementation Plan

All changes are in `src/parser/control.rs`.

### Step T1.9-1 — Dispatch in `parse_match`

In the subject-type match inside `parse_match`:

```rust
Type::Tuple(_) => {
    return self.parse_tuple_match(subject, &subject_type, code);
}
```

This goes in the existing dispatch block alongside the vector and scalar dispatches
(lines ~324–337).

### Step T1.9-2 — `parse_tuple_match` (new function)

```rust
fn parse_tuple_match(
    &mut self,
    subject: Value,
    subject_type: &Type,
    code: &mut Value,
) -> Type {
    let Type::Tuple(elem_types) = subject_type.clone() else { unreachable!() };
    let arity = elem_types.len();

    // Store subject in temp — gives TupleGet a stable var_nr.
    let v = self.create_unique("match_subj", subject_type);
    self.vars.defined(v);

    self.lexer.token("{");
    let mut result_type = Type::Void;
    struct TupleArm { cond: Option<Value>, bindings: Vec<Value>, guard: Option<Value>, body: Value }
    let mut arms: Vec<TupleArm> = Vec::new();
    let mut has_wildcard = false;
    let match_pos = self.lexer.pos().clone();

    loop {
        if self.lexer.peek_token("}") { break; }

        if self.lexer.has_token("_") {
            // Total wildcard arm
            has_wildcard = true;
            let guard = self.parse_optional_guard();
            self.lexer.token("=>");
            let mut body = Value::Null;
            let bt = self.expression(&mut body);
            result_type = self.merge_types(result_type, bt);
            arms.push(TupleArm { cond: None, bindings: vec![], guard, body });
        } else {
            // Tuple pattern arm
            self.lexer.token("(");
            let mut arm_cond: Option<Value> = None;
            let mut arm_bindings: Vec<Value> = Vec::new();

            for i in 0..arity {
                if i > 0 { self.lexer.token(","); }
                self.parse_tuple_elem_pattern(
                    v, i as u16, &elem_types[i].clone(),
                    &mut arm_cond, &mut arm_bindings,
                );
            }
            self.lexer.token(")");

            // A pattern with no conditions is total.
            if arm_cond.is_none() {
                has_wildcard = true;
            }

            let guard = self.parse_optional_guard();
            // Guarded arm — even if all-binding, not total.
            if guard.is_some() && arm_cond.is_none() {
                has_wildcard = false;  // guard may fail
            }

            self.lexer.token("=>");
            let mut body = Value::Null;
            let bt = self.expression(&mut body);
            result_type = self.merge_types(result_type, bt);
            arms.push(TupleArm { cond: arm_cond, bindings: arm_bindings, guard, body });
        }

        self.lexer.has_token(";");
    }

    self.lexer.token("}");

    if !has_wildcard && !self.first_pass {
        diagnostic_at!(
            self.lexer, match_pos, Level::Error,
            "tuple match is not exhaustive — add a wildcard arm `_` or \
             an all-binding arm `({})` to cover all cases",
            elem_types.iter().map(|_| "_").collect::<Vec<_>>().join(", ")
        );
    }

    // Build if-chain from last arm to first.
    let mut chain = Value::Null;
    for arm in arms.into_iter().rev() {
        let arm_body = if arm.bindings.is_empty() && arm.guard.is_none() {
            arm.body
        } else {
            let mut stmts = arm.bindings;
            let body = if let Some(guard) = arm.guard {
                // guard failure falls through to chain (the following arms)
                v_if(guard, arm.body, chain.clone())
            } else {
                arm.body
            };
            stmts.push(body);
            v_block(stmts, result_type.clone(), "tuple arm")
        };

        chain = if let Some(cond) = arm.cond {
            v_if(cond, arm_body, chain)
        } else {
            arm_body
        };
    }

    let preamble = Value::Set(v, Box::new(subject));
    *code = v_block(vec![preamble, chain], result_type.clone(), "tuple_match");
    result_type
}
```

### Step T1.9-3 — `parse_tuple_elem_pattern` (new function)

```rust
fn parse_tuple_elem_pattern(
    &mut self,
    tuple_var: u16,
    idx: u16,
    elem_type: &Type,
    arm_cond: &mut Option<Value>,
    arm_bindings: &mut Vec<Value>,
) {
    let elem_val = Value::TupleGet(tuple_var, idx);

    if self.lexer.has_token("_") {
        // Wildcard — no condition, no binding.
        return;
    }

    // Nested tuple pattern: (p1, p2, ...) when elem_type is Type::Tuple.
    if self.lexer.peek_token("(")
        && matches!(elem_type, Type::Tuple(_))
    {
        if let Type::Tuple(inner_types) = elem_type.clone() {
            // Create a temp var for the inner tuple element so we can TupleGet from it.
            let inner_v = self.create_unique("match_inner", elem_type);
            self.vars.defined(inner_v);
            arm_bindings.push(Value::Set(inner_v, Box::new(elem_val)));
            self.lexer.token("(");
            for (i, inner_type) in inner_types.iter().enumerate() {
                if i > 0 { self.lexer.token(","); }
                self.parse_tuple_elem_pattern(
                    inner_v, i as u16, inner_type, arm_cond, arm_bindings,
                );
            }
            self.lexer.token(")");
            return;
        }
    }

    // Check if the next token is a plain identifier (binding variable).
    // An identifier is a binding if it is lower_case and not a keyword or literal.
    if let Some(name) = self.try_parse_binding_identifier(elem_type) {
        // Binding — no condition; add Set statement.
        let var_nr = self.vars.add_variable(&name, elem_type, &mut self.lexer);
        self.vars.defined(var_nr);
        arm_bindings.push(Value::Set(var_nr, Box::new(elem_val)));
        return;
    }

    // Scalar pattern: literal, range, or-pattern, null.
    // Store elem_val in a temp var for parse_match_pattern (which needs a var_nr).
    let tmp = self.create_unique("elem_tmp", elem_type);
    self.vars.defined(tmp);
    arm_bindings.push(Value::Set(tmp, Box::new(elem_val)));

    let (pat_cond, _) = self.parse_match_pattern(elem_type, tmp);
    *arm_cond = Some(match arm_cond.take() {
        None => pat_cond,
        Some(c) => self.cl("OpAndBool", &[c, pat_cond]),
    });
}
```

**`try_parse_binding_identifier`**: peeks at the next token. Returns `Some(name)` if it
is a lower-case identifier that is not a keyword and does not look like the start of a
literal or operator. Returns `None` if the next token is a literal, `null`, `(`, or a
keyword. This distinguishes `(n, msg) =>` (bindings) from `(0, "foo") =>` (literals).

Heuristic: if `lexer.peek()` is `LexItem::Identifier(name)` → binding. If it is
`LexItem::Integer`, `LexItem::Text`, `LexItem::Boolean` → literal (use scalar pattern).

### Step T1.9-4 — `parse_optional_guard` helper (extract or inline)

Guards are already parsed in `parse_match` with `self.lexer.has_token("if")`.  Either
inline the same pattern in `parse_tuple_match` or extract a small helper.

---

## Edge Cases

| Case | Behaviour |
|---|---|
| Subject is not a `Type::Tuple` | Existing dispatch; no change |
| Arm arity ≠ subject tuple arity | Compile error "expected N elements in pattern, found M" |
| Binding name already used in the same arm | Compile error "duplicate binding in tuple pattern" |
| Nested tuple element accessed in guard | Works — bindings are emitted before guard evaluation |
| All-`_` arm (`(_, _, ...)`) | Total; satisfies exhaustiveness |
| Guarded all-binding arm | Not total — guard may fail; still need an unguarded arm |
| `null` subject | TupleGet on a null variable — same as accessing a null tuple today (debug assert); document as UB until T1.8b adds null-safety |
| Text element bound in arm | Works; lifetime same as LHS destructuring binding |
| Tuple element that is itself a struct enum | Scalar `match` on that element is not in scope of T1.9 — use a separate outer `match` |
| Match on `match` result (chained) | Works — outer match dispatches to `parse_tuple_match` normally |

---

## Test Plan

New test file `tests/tuple_match.rs` or additions to `tests/match.rs`:

| Test | Coverage |
|---|---|
| `tuple_match_binding` | `(n, msg) =>` — all bindings, exhaustive |
| `tuple_match_literal` | `(0, "x") =>` — exact literals on both elements |
| `tuple_match_wildcard` | `(0, _)` and `_` arm — exhaustive via wildcard |
| `tuple_match_range` | `(1..10, _) =>` — range on first element |
| `tuple_match_or_elem` | `(1 \| 2 \| 3, _) =>` — or-pattern in element position |
| `tuple_match_guard` | `(n, msg) if n > 0 =>` — guard, with fallthrough arm |
| `tuple_match_nested` | `((x, y), true) =>` — nested tuple pattern |
| `tuple_match_three` | `(a, b, c) =>` — three-element tuple |
| `tuple_match_as_expr` | `val = match t { ... }` — match produces a value |
| `tuple_match_not_exhaustive` | No total arm → compile error |
| `tuple_match_guarded_not_total` | Guarded all-binding arm → still requires unguarded arm |
| `tuple_match_arity_mismatch` | `(a, b, c)` on `(integer, text)` → compile error |
| `tuple_match_null_elem` | `(null, _)` on nullable element → works |
| `tuple_match_in_function` | Tuple match as function return value |

### Reference loft script

```loft
// tests/docs/28-tuples.loft — new section

fn classify(t: (integer, text)) -> text {
    match t {
        (0, _)            => "zero"
        (1..10, "")       => "small-empty"
        (1..10, s)        => "small: {s}"
        (n, msg) if n < 0 => "negative: {n} {msg}"
        (n, msg)          => "{n}: {msg}"
    }
}

fn main() {
    assert(classify((0, "x"))   == "zero",          "zero case");
    assert(classify((5, ""))    == "small-empty",   "small-empty");
    assert(classify((5, "hi"))  == "small: hi",     "small binding");
    assert(classify((-3, "no")) == "negative: -3 no", "guard");
    assert(classify((99, "ok")) == "99: ok",        "fallback");

    // Nested tuple match
    coords: ((float, float), boolean) = ((0.0, 0.0), true)
    result = match coords {
        ((0.0, 0.0), _) => "origin"
        ((x, y), true)  => "active {x},{y}"
        ((x, y), false) => "inactive {x},{y}"
    }
    assert(result == "origin", "nested tuple: {result}");
}
```

---

## See also
- [TUPLES.md](TUPLES.md) — Full tuple design; T1.8a/b for function-return convention
- [LOFT.md](LOFT.md) § Match expressions — match syntax reference;
  L2 nested field patterns tracked in PLANNING.md
- [PLANNING.md](PLANNING.md) — T1 backlog
- `src/parser/control.rs` — `parse_match`, `parse_scalar_match`, `parse_vector_match`
- `src/data.rs` — `Type::Tuple`, `Value::TupleGet`, `Value::TuplePut`
