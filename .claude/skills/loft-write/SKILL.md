---
name: loft-write
description: >-
  Reference for writing correct loft code. Use it whenever you write, edit or review any
  .loft source — a program, a library, a doc example, a language-suite cell or a throwaway
  /tmp probe — and when a probe fails to compile or warns, before assuming the compiler is
  wrong. Covers types, syntax, structs, closures and their captures, collections, naming
  rules, known bugs with their workarounds, the lints that fire on small programs, and an
  error-to-fix table.
user-invocable: false
---

# Loft Language Writing Reference

## Top traps

Each of these either compiles to a program that answers wrong without a word, or fails a
compile in a way that sends you looking in the wrong place.

1. **A whole-value bind COPIES** — `w = r.v; w[0] = 99` leaves `r.v` unchanged.  Bind
   `w = &r.v` to write through (§ Copy or view).
2. **A struct-element bind is a VIEW** — `t = v[j]` names slot j, so swapping struct elements
   via a temp duplicates one record.  Swap field by field, or build a new vector.
3. **`len(t)` counts characters, `t[a..b]` counts bytes** — cut by a character count with
   `t.char_slice(a, b)` (§ Text pitfalls).
4. **`v[-1]` is the last element; only `i >= len(v)` answers null** — a `-1` sentinel reads a
   real element.  Test `i >= 0` before trusting a null guard.
5. **Uncomputable arithmetic answers null and keeps running** — `÷0` and overflow yield null.
   Defend with `?? fallback`, or `x?` for the type's zero.
6. **`a / b?` is `a / (b?)` and answers null** — postfix `?` binds tightest, `??` loosest.
   Write `(a / b)?`.
7. **A local ends at its block's `}`** — reading it after is `local-out-of-scope`.  Bind it
   before the block, or make the block's value the binding (§ Variable scoping).
8. **Match arms use `=>` and are comma-separated** — `->` is the return arrow; a missing comma
   reads as `Expect token ,`.
9. **`fn` only at file scope** — inside a body write a lambda (`|x| { … }`, `fn(x: T) { … }`).
10. **`text`, not `string`; `integer` (64-bit), not `long`** — literals take no `l` suffix.
11. **A bare variant (`c = Red`) needs one owning enum** — when two enums share `Red`, write
    `Color.Red` unless the context picks one (§ Enums).
12. **`text as integer` is `integer?`** — a parse can fail; write `s as integer ?? 0`.

Before reimplementing stdlib or library functionality, run `make libcatalogue` and read the
generated `doc/claude/LIBRARIES.md`, plus `loft install <name>`; stdlib signatures are in
[STDLIB.md](../../../doc/claude/STDLIB.md).  Library code is written readable first — a slow
routine is an engine finding, not a cue for a fast-path rewrite
([LIBRARY_AUTHORING.md § The library contract](../../../doc/claude/LIBRARY_AUTHORING.md#the-library-contract), item 7). <!--noindex-->

---

## Naming conventions — enforced by the parser

| Construct | Convention | Examples |
|-----------|-----------|---------|
| Functions, variables | `lower_case` | `my_fn`, `item_count` |
| Types, structs, enums, variants | `CamelCase` | `Point`, `Color`, `Red` |
| Constants (file-scope) | `UPPER_CASE` | `PI`, `MAX_SIZE` |
| Operator definitions | `OpXxx` prefix | `OpAdd`, `OpEqInt` |

The parser **rejects** code that violates these rules.  `OpXxx` names are the stdlib's
operator table only (@C132); to put an operator on your own type write `operator compare` /
`plus` / … ([references/expressions.md](references/expressions.md)).

---

## Primitive types

| Type | Description | Null sentinel |
|------|-------------|--------------|
| `integer` | **64-bit** signed int (the base integer type; i64 at rest) | `i64::MIN` |
| `float` | 64-bit float; literal must contain `.`: `1.0` | `NaN` |
| `single` | 32-bit float; literal suffix `f`: `1.0f` | `NaN` (32-bit) |
| `boolean` | `true` / `false` (2-state by default under DN1; **`boolean?`** adds `null` → three-state, @PLN17) | `boolean?` null is byte `255` (test with `b == null`; `!b` is true for both `null` and `false`; `null == false` is `false`) |
| `character` | Single Unicode char; literal: `'a'`, `'😊'`; `c as integer` → codepoint | `'\0'` |
| `text` | UTF-8 string (primary string type) | internal null pointer |

`now()` / `File.size` return a full-range `integer`.  **Narrow / sized integers are aliases of
`integer`** (defined in `default/01_code.loft`), for byte-width-sensitive file / wire formats:
`i8` `size(1)`, `i16`/`u16` `size(2)`, `i32`/`u32` `size(4)`.  Cast with `x as i32` etc.  Use
plain `integer` for ordinary arithmetic — it has the full 64-bit range.

---

## Null, `?` and casts — the short version

- **Every scalar, `text` and vector is non-null by default**; append `?` to allow null
  (`integer?`, `text?`, `vector<T>?`).  Storing null into a plain slot warns.
- **`x ?? d`** supplies the default you give; **`x?`** supplies the type's own (`0`, `""`,
  empty, every field defaulted, an enum's first variant).  `?` binds tightest, `??` loosest.
- **A text parse is checked**: `s as integer` types `integer?`; write `s as integer ?? 0`,
  `(s as integer?)?`, or keep it `integer?`.  `(s as integer)?` is refused.
- **`s as Circle` and `t as Color` answer a default on a miss and warn** — check with `is`
  first, or write `as Circle?`.
- **`if v == null { return …; }` narrows** `v` to present for the rest of the block.

Read [references/null-and-casts.md](references/null-and-casts.md) when a slot may hold null,
when you discharge a `τ?` or parse text, or when the default `x?` picks matters.

### Idioms for tree-walking and lookup code — reach for these before writing a `??`

- **Bind a variant or leave:** `args = match c { NdCall { args } => args, _ => return null };`
  — loft's `let … else` ([LOFT_CONTROL.md § Match](../../../doc/claude/LOFT_CONTROL.md#match-expressions)).
  Inside one block, `if c is NdCall { args } { … }`.
- **Bind a nullable or leave:** `x = lookup(k) ?? return null` (also `?? break`, `?? continue`).
- **An index needs no `??` when the compiler trusts it:** a constant (`args[0]`, `v[-1]`),
  `v[len(v) - 1]`, a `for j in 0..len(v)` variable, or an index a guard proved (`if i < len(v)`).
  `n = len(v); v[n - 1]` and `v[x#index]` are not trusted — write `v[len(v) - 1]`, read `x`
  ([LOFT_DATA.md § Vectors](../../../doc/claude/LOFT_DATA.md#vectors)).
- **Discharge the rest by intent:** `v[i]!` when an invariant guarantees it (halts otherwise),
  `v[i]?` for the type's default, `v[i] ?? d` for your own default.
- **Order:** `v.sort()`, `v.sort_by(fn)` for a one-off order; `sorted<T[-key]>` when records
  are added over time and must stay ordered.

---

## Field modifiers (struct fields only)

**Read [fields.md](fields.md) before declaring a struct field with a modifier** — `limit`, a default, `virtual`, and the two `const` axes (binding-const `const v: T` for a builder, value-const `v: const T` for a frozen value).

---

## Variable declarations

Variables declared by assignment — type is inferred from the initialiser.

<!-- from tests/reference/skill-declarations.loft -->
```loft
x = 5;
s = "hello";
f = 3.14;
```

Explicit type annotations (sometimes required for empty collections):
<!-- from tests/reference/skill-declarations.loft -->
```loft
v: vector<integer> = [];
n: integer? = null;             // a null needs a NULLABLE type; `null as integer` is a
                                // compile error (cannot cast null to non-null integer)
```

### `const` variables

<!-- from tests/reference/skill-declarations.loft -->
```loft
const x = 5;       // immutable local — reassignment is a compile error
const t = "hello";  // works for any type
```

---

## Constants

The `const` keyword is accepted at file scope as well, alongside the
bare-name form.  Both produce the same file-scope immutable constant:

<!-- from tests/reference/skill-constants.loft -->
```loft
TAU      = 6.28318530717958;    // bare-name form
const E  = 2.71828182845905;    // const-keyword form
pub const MAX_SIZE = 256;       // pub + const combine for exported constants
```

Constants must be `UPPER_CASE`.  Prefer the `const` form when you want the immutability intent
visible at the declaration site (the bare-name form relies on the UPPER_CASE convention as the
only visual signal); both forms behave identically.

**UPPER_CASE locals get `advice[upper-case-local]` unless declared `const`.**  Inside a
function body, `FOO = 5;` emits advice telling you to add `const` or rename to `lower_case` —
"UPPER_CASE means immutable" holds at every scope.

---

## Functions

```grammar
fn name(param: Type) -> ReturnType { body }
fn name(a: integer, b: integer = 0) -> integer { a + b }  // default param
pub fn exported() { }   // pub = publicly visible
```

Parameter modifiers:
- `const T` — immutable (compile error to assign to it inside function)
- `&T` — a **live reference** to the caller's argument: reads see the caller's current value,
  writes write the caller, field/element mutation mutates the caller. **Call WITHOUT `&`** — the
  reference comes from the param's TYPE: `fn inc(n: &integer){ n = n + 1 }` is called `inc(x)`, not
  `inc(&x)` (`&` is a binding marker, not a general operator). `&` to a temporary or in a general
  expression is an error; `v[i]` and `s.field` pass directly.
- **`&` that is never mutated is a compile error** — drop it if the param is read-only
- Omit modifier — pass by value/copy (heap values still alias: field/element mutation propagates)

A local reference is bound with `a = &b` (or `a: &T = b`); the operand must be addressable (a
variable / struct field / vector element). Full model: [LOFT_DECLARATIONS.md § References](../../../doc/claude/LOFT_DECLARATIONS.md).

A function body ending in an expression (no `;`) returns that value. Functions without `->` return `void`.

### `self` — one name, both call spellings

Name the first parameter `self` and the function is callable as a method AND as a
free function:

<!-- from tests/reference/methods.loft -->
```loft
pub fn exists(self: Doc) -> boolean {
  self.format != "missing"
}
```

Both spellings call it:

<!-- from tests/reference/methods.loft -->
```loft
// Can be called as:
a = f.exists();      // method syntax
b = exists(f);       // free function syntax
```

Don't name a first parameter `both` — it warns (`both-receiver-deprecated`), and a library's
CI denies warnings.

---

## Imports

Multiple names are parenthesised — `use lib::(a, b);`, never `use lib::a, b;` — and `use`
declarations come before any other declaration in the file.  `use lib;` gives the `lib::`
qualifier only; `use lib::*;` brings every `pub` name in bare.

Read [references/names-and-imports.md](references/names-and-imports.md) for aliases, `pub use`,
`#package` visibility, and which builtin names you may shadow.

**Read [library-api.md](library-api.md) before writing `use` of a library.**  Never guess a
signature: for a program, read `.loft/api/<name>.api` (or `loft api <name>`); to see what exists,
`.loft/api/_available.api` / `loft api --registry`; when working ON a library in this repo,
`make libcatalogue` then `doc/claude/LIBRARIES.md`.

---

## Composite types

| Syntax | Description |
|--------|-------------|
| `vector<T>` | Dynamic array |
| `hash<T[field]>` | Hash-map keyed by `field` on struct `T` |
| `index<T[field, -field2]>` | B-tree index; `-` = descending |
| `sorted<T[field]>` | Sorted collection |
| `reference<T>` | Pointer to a stored `T` record |
| `fn(T1, T2) -> R` | First-class function type |

---

## Structs

<!-- from tests/reference/skill-declarations.loft -->
```loft
struct Item { name: text, count: integer }
```
<!-- from tests/reference/skill-declarations.loft -->
```loft
item = Item { name: "foo", count: 0 };
item.count += 1;
```

A record with some fields changed is one expression, `..base` last:

<!-- from tests/reference/skill-declarations.loft -->
```loft
more = Item { count: 5, ..item };       // a copy of `item` with `count` written; `item` keeps its value
```

`d = Unit { hp: 0, ..u }` is `d = u; d.hp = 0;`: the named fields are written as assignments (a
`text` or collection field releases what the copy brought; a `const` field is refused), each
reads the BASE, and the base must be an `Item` and present (discharge an `Item?` first).  A
variant updates the same way when the base's type IS that variant (`..b` with `b: Bin`, or
`..(n as Bin)` inside `if n is Bin`); a base typed as the enum is refused.

Field names may overlap across structs — lookups are type-scoped.

**A field may hold its own type** — `struct Tree { key: integer, left: Tree?, right: Tree? }`,
`enum Expr { Num { v: integer }, Add { l: Expr, r: Expr } }`.  It is a CHILD: a value stored
out of line, copied on write, released by `t.left = null`, compared and rendered as a nested
value (@C159).  Never write a one-element `vector<T>` or a `reference<T>` to break the cycle:
`reference<T>` is for ONE node shared by two places.  A struct field on the cycle must be `T?`
unless a nullable hop or an enum variant ends it.  Building a list by prepending
(`h = List { v: i, next: h }`) copies the whole list each time; write into the tail instead.

---

## Tuples

**Read [tuples.md](tuples.md) before writing a tuple** — the type notation, element access and
assignment, destructuring, nesting, and the five restrictions (no single-element tuple, no
named fields, no iteration or whole-tuple formatting, non-null scalar elements, no compound
assignment on a tuple left-hand side).

---

## Enums

Simple enum (value type, no fields):
<!-- from tests/reference/skill-declarations.loft -->
```loft
enum Color { Red, Green, Blue }
```
<!-- from tests/reference/skill-declarations.loft -->
```loft
c = Color.Red;                 // qualify when DEFINING a new variable
// ordering follows declaration order: Red < Green < Blue
```

**A bare variant is the variant of the one enum that owns that name.**  `c = Red` works
when only `Color` has `Red`.  Two enums MAY share a variant name (`enum Color { Red }` +
`enum Berry { Red }`); then the context picks one — a `match` subject, a typed target
(`c: Color = Red`), a comparison (`c == Red`), a parameter, a return position, a field's
type, or the other arms of the same `if` / `match` — and with no context the shared name is
refused with an error that names the qualification to write (`Color.Red`).  The full rule:
`doc/claude/LOFT_DECLARATIONS.md` § Enum-scoped variants.

Struct-enums (variants with fields), per-variant methods and the JSON round-trip:
[references/expressions.md](references/expressions.md).

---

## Vectors

```grammar
empty: vector<integer> = [];
nums  = [for i in 0..10 { i * 2 }];    // comprehension
items = [1, 2, 3];

v += [element];      // append
v += other_vec;      // concatenate
len(v);              // length
v[i];                // index read
```

**Empty vectors** need a type annotation so the compiler knows the element type.

**Slices are iterators, materialised on assignment.** `sub = arr[lo..hi]` (annotated or not) builds a fresh vector, and negative bounds count from the end (`arr[2..-1]`, @P384).  A slice passes directly where a `vector<T>` argument is expected (`total(a[1..3])`).

**`v[i]` with a possibly-negative index does NOT null-guard.** Scalar indexing counts from the end for negative `i`, exactly like slices: `v[-1]` is the LAST element, `v[-len]` the first — NOT `null`.  Only `i >= len` (and `i < -len`) yield `null`.  So `if v[i] { … }` and `v[i] ?? d` catch an over-range index but NOT a negative one — a `-1` "not-found" sentinel or a `a - b` underflow silently reads a real element from the end.  When `i` can go negative, test `if i >= 0` FIRST (that `>= 0` check is not redundant with a later null-guard).

### Copy or view

**A struct-typed projection is a VIEW; a whole value or a collection field COPIES** (@C86)
([formal/binding.md](../../../doc/claude/formal/binding.md) `(B-View)` / `(B-Copy)`; the
other view cases are `(B-View-Base)` and `(B-View-Depth)` there).

- `e = v[i]` or `s = o.inner`, where the element or field IS a struct, names that slot: writes
  through `e` reach `v[i]`.  So **never swap `vector<STRUCT>` elements via a temp** —
  `tmp = v[j]; v[j] = v[k]; v[k] = tmp;` copies k into j while `tmp` still views j, losing j's
  record and duplicating k's.  Swap scalar fields one by one, or build a fresh vector.
- `w = v` or `w = r.v` (a collection field) is an independent copy, so **never capture a vector
  into a local in order to write through it** — every write lands in the copy:

```grammar
// BAD — `results` is a COPY; all three writes are discarded:
results = self.results;
results[i] = "true";               // lost
results[i] = Row { id: 1 };        // lost
results += [Row { id: 2 }];        // lost

// If you need a local at all, bind a live reference with `&`:
results = &self.results;           // aliases the field
results[i] = "true";               // propagates
```

Reads through a plain copy are correct, so only the writes vanish, arbitrarily far from where
the value is later found missing.  The compiler warns on the simple shape (§ Known bugs).

---

## Hash collections

A hash works as a struct field or as a plain local:

<!-- from tests/reference/skill-declarations.loft -->
```loft
struct Entry  { key: text, value: integer }
struct Table  { data: hash<Entry[key]> }
```
<!-- from tests/reference/skill-declarations.loft -->
```loft
t = Table { data: [] };
t.data += [Entry { key: "x", value: 1 }];
e = t.data["x"];
if e == null { seen = "not found"; }
else { e.value += 1; }
assert((t.data["x"].value ?? -1) == 2 and seen == "", "found, and written through the view");
t.data["x"] = null;   // remove entry
t.data += [Entry { key: "b", value: 2 }, Entry { key: "a", value: 1 }];
for row in t.data { seen += row.key; }  // iterates in key order (C60)
assert(seen == "ab" and t.data["x"] == null, "removed, then walked in key order");
```

(`key` as the key-field name is fine; a standalone `data: hash<Entry[k]> = [];`
local is fine too.)

---

## Interfaces and bounded generics

Structural: a type satisfies `interface Comparable { … }` when the methods exist — no `impl`
block; bound a generic with `fn find_min<T: Comparable>(v: vector<T>) -> T`.  Every type is
`Equatable`.  Declaring one: [references/program-shape.md](references/program-shape.md).

---

## Operators

| Precedence | Operators | Notes |
|-----------|-----------|-------|
| 0 (lowest) | `??` | null-coalescing |
| 1 | `\|\|`, `or` | logical OR |
| 2 | `&&`, `and` | logical AND |
| 3 | `==`, `!=`, `<`, `<=`, `>`, `>=`, `is` | comparison, variant check |
| 4–7 | `\|`, `^`, `&`, `<<`, `>>` | bitwise |
| 8 | `+`, `-` | |
| 9 | `*`, `/`, `%` | |
| 10 | `as` | type cast/conversion |
| — (tightest) | postfix `?` | default-fallback — a **suffix**, not a rung: binds tighter than `as`, like `.` / `[]` |

Unary: `!` (logical not / null check), `-` (negation), `~` (bitwise NOT, integer only)
Assignment: `=`, `+=`, `-=`, `*=`, `/=`, `%=`

```grammar
name = record.field ?? "default"   // null-coalescing
name = record.field?               // …or the TYPE's default ("" for text)
x as i32                           // cast to a 4-byte sized integer
flags & ~32                        // bitwise NOT — clears bit 5
```

**`==` compares content for every type**; `&a == &b` asks whether two names are one record.
Operators on your own type (`operator plus` …) and the identity rule in full:
[references/expressions.md](references/expressions.md).

### `is` variant check

<!-- from tests/reference/skill-is-and-match.loft -->
```loft
if d is North { hit = true; }      // boolean check
assert(!(shape is Rect));          // negation

// field capture — binds variant fields as locals scoped to the if-body
if shape is Circle { radius } {
  area = PI * radius * radius;
}

// multiple fields + else
if shape is Rect { width, height } {
  area = width * height;
} else {
  area = 0.0;
}
```

A unit variant has no fields and so no capture list: the `{` after `d is North` opens the
block, whatever the block starts with.

---

## String / text literals

### Double-quoted strings (`"..."`)

Single-line. Supports `{expr}` interpolation and `\n`, `\t`, `\\`, `\"` escapes.

<!-- from tests/reference/skill-strings.loft -->
```loft
println("hello {name}");
println("hex={n:#x}");            // hex with 0x prefix
println("float={f:4.2}");         // width 4, 2 decimal places
println("json={o:j}");            // JSON format
println("pretty={o:#}");          // pretty-printed multi-line
println("padded={n:>5}");         // right-align width 5
println("zero={n:03}");           // zero-padded width 3
println("{{literal braces}}");    // escape { } by doubling — see the warning below
```

**A literal `{` in a string is an interpolation hole.** `"[{r:1,g:2}]"` is read as a hole
named `r` with the spec `1,g:2`, and the error names the FORMATTER, not the brace. Any text
containing braces — JSON, a rendered struct, a code sample — must double them: `{{` and `}}`.

A spec pads and aligns any type, but `0N` zero-padding is for numbers only:
[references/expressions.md](references/expressions.md) § Format specs.

### Backtick strings (`` `...` ``)

Multi-line, interpolated, common indentation stripped; a literal brace is doubled there too:
[references/program-shape.md](references/program-shape.md).

Use `println()` for line-oriented output and `print()` for output without a newline.

---

## Control flow

```grammar
if cond { } else if cond { } else { }
result = if x > 0 { x } else { -x }    // if as expression

// null check
if !x { }            // x is null (or false for boolean)
val = a ?? b         // null-coalescing

while cond { }
return expr;
break;
break expr;          // break with value (requires non-void function)
```

---

## For loops

```grammar
for i in 0..n { }         // exclusive: 0 to n-1
for i in 0..=n { }        // inclusive: 0 to n
for item in collection { }
for c in some_text { }    // character iteration
for item in col if item.active { }   // filtered iteration
for i in rev(0..n) { }    // reverse range
```

### Loop attributes

`v#index`, `v#count`, `v#first`, `v#next` and `v#remove` (filtered loops) inside a `for`:
[references/program-shape.md](references/program-shape.md).
---

## Match

<!-- from tests/reference/skill-is-and-match.loft -->
```loft
match color {
    Red   => { said = "red"; },
    Green | Blue => { said = "cool"; },
    _     => {},
}
```
<!-- from tests/reference/skill-is-and-match.loft -->
```loft
match shape {
    Circle { radius } => { said = "r={radius}"; },
    Rect { width, height } if width == height => { said = "square"; },
    _ => {},
}
```

Match is an expression — all arms must produce the same type (or void).  Matching a tuple, a
variant payload or a slice, and the scalar-hole advice:
[references/expressions.md](references/expressions.md) § Match beyond a plain enum.

---

## Higher-order functions

Two lambda syntaxes, each with a clear job:

- **Shorthand `|x|`** — types inferred from the call-site context
  (e.g. the `map`/`filter` signature).  Use inside higher-order
  calls where the types flow in.
- **Explicit `fn(...)`** — full type annotations.  Use when the
  lambda is stored in a local variable, or anywhere types can't
  be inferred.  Omit `->` for void-returning lambdas (`-> void`
  is not valid syntax — there is no `void` type).

<!-- from tests/reference/skill-hofs.loft -->
```loft
fn double(x: integer) -> integer { x * 2 }
```
<!-- from tests/reference/skill-hofs.loft -->
```loft
doubled  = map(nums, double);             // named function ref — bare name, no `fn` prefix
positive = filter(nums, |x| { x > 0 });  // inferred lambda
total    = reduce(nums, 0, |a, b| { a + b });

// Method form on vectors
doubled  = nums.map(|x| { x * 2 });
evens    = nums.filter(|x| { x % 2 == 0 });

// Typed lambda stored in a local: use the explicit fn(...) form.
emit = fn(x: integer, y: integer) { total += x + y; };
emit(1, 2);
```

**Type annotations on `|x|` shorthand are rejected by design** (@C62,
DESIGN_DECISIONS_SYNTAX.md) — when you need types, write `fn(name: <type>) { ... }`.

---

## Variable scoping — a block's locals end at its `}`

**A local ends at the `}` of the block that bound it — an `if` arm, a loop body, a
`match` arm, a bare `{ }` — and so does a loop variable** (`formal/binding.md`
`(B-Scope)`, rustc's rule; LOFT_DESIGN.md § Variable scoping).  Reading it after is
`error[local-out-of-scope]`, also when every arm binds it.  Bind it before the block
(`x: T? = null; if c { x = mk(); }`, `last = 0; for v in xs { last = v; }`) or make the
block's value the binding (`x = if c { a } else { b }`, a tuple for several).

Within one scope a name keeps ONE type; a name in a different function is free.  The
violations are clean compile-time errors with fix hints, on both backends:

- **Re-typing a live name** → `x = 1; … x = "hi";` is `Variable 'x' cannot change type
  from integer to text` — and so is assigning another type to an OUTER local inside a
  block.  Two sibling blocks may each bind `x` at their own type, because the first
  `x` has ended.  Re-assigning the *same* type is fine (`x = 1; x = 2`).
- **A loop variable named like an existing local** →
  `loop variable 'x' shadows a local named 'x' — rename the loop variable
  (e.g. loop_x)`.  Rename it, or drop the dead outer local.
- **Nested same-name loops** → `for i { for i { … } }` is rejected.
- **Loop variables are inference-only** — `for i: integer in …` does not parse
  (`loop variable 'i' is type-inferred from the iterable`).  Drop the annotation.

Sequential loops may reuse a name at different element types — `for i in ["a","b"] {…}`
then `for i in 0..3 {…}` compiles — so short names (`i`, `e`, `n`) are fine.

**Unused variable = warning, not an error** — the program still runs (exit 0).
Use `_` for an unused loop variable to keep the build warning-clean.

---

## Text pitfalls

**There is no `substr` / `substring`.** A part of a text is a slice:

<!-- from tests/reference/text-slices.loft -->
```loft
s = "abcdef";
a = s[1..3];                // "bc" — from byte 1 up to, not including, byte 3
b = s.char_slice(1, 3);     // "bc" — the same, counting CHARACTERS instead of bytes
```

**Pick by where the numbers came from**, and this is the commonest text bug in this stack:
`len(t)` counts CHARACTERS, `t.size()` and `s[a..b]` count BYTES (a File's `f.size` is a field). A character count used as a byte
range fits fewer characters than it measured, so the text comes back SHORT — silently, and
only once the text stops being ASCII. `"héllo"[0..4]` is `"hél"`; `"héllo".char_slice(0, 4)`
is `"héll"`. Use `char_slice` whenever a character count becomes a cut (fitting a label,
wrapping, a caret, truncating); use `s[a..b]` only for numbers from `find` / `rfind` / `size`
/ `byte_at`.

**`character == text` is a compile error** — use `"{c}" == t` to compare as text.

(Reassigning a `text` parameter works — `msg = msg + "!"` inside the callee is fine;
the parameter is the callee's own binding.)

**Prefer `h += expr`** over `h = h + expr` for text building.

---

## File I/O patterns

**Read [file-io.md](file-io.md) before any program that opens a file.**  It covers path resolution
(a relative path is PROGRAM-relative, `#cwd` for CLI tools), UTF-8 text files (`content`, `lines`,
`write`), and binary files (`f#format`, `f#read as <type>`, `f += (x as i32)`, `f#next`,
`read_bytes` / `write_bytes`).  Two rules to carry without opening it: `f.content()` is
UTF-8-only (`text?`, null on a binary file), and a scalar write is 8 bytes unless cast to its
width (`f += (n as i32)`).

---

## Known bugs and workarounds

- **`Too few parameters on n_<fn> (got …, need …)`** is a compiler panic (a hidden-buffer ABI
  mismatch), not a mistake in your code — reduce it to a minimal repro and file it
  (`gh issue create`).
- **`Allocating a used store`** — a store is being reused while still held; check parallel
  blocks and store lifetimes.  A field named `key` is NOT the cause; that works.
- **The lost-write lint misses guarded writes.**  It warns on `w = v` followed by an unread
  `w[i] = …`, but a bound guard such as `if i < len(w)` counts as a read and silences it.
  Treat it as a backstop: bind `w = &v` whenever you mean to write through (§ Copy or view).

---

## Known error messages → fixes

| Error message | Fix |
|--------------|-----|
| `Variable <x> is never read` | A **warning** (program still runs, exit 0) — use the variable, or name an unused loop var `_` |
| `The value parameter 'p' receives is never read` / `Dead assignment — … a parameter's assignment stays in this function` | A plain parameter is the callee's own local: the assignment reaches no caller.  Write `p: &T` if the caller is meant to see it, else a local (DIAGNOSTICS.md `never-read` / `dead-assignment`) |
| `Indexing a non vector` | You indexed a scalar (`x = 5; x[0]`) — index a vector/collection, not a single value |
| `Cannot assign <T> to a field of type null` | A local named `null` (a literal keyword) — rename it |
| `Undefined type string — did you mean 'text'?` | Use `text`, not `string` |
| `Undefined type long — did you mean 'integer'?` | `integer` is already 64-bit; literals are plain (`86400000`, not `86400000l`) |
| `variant-cast-default` / `enum-parse-default` (warnings) | The cast answers a default on a miss — check first (`if s is Circle`), or write `as V?` / `?? default` |
| `Unknown function say` | Use `println()` |
| `Cannot pass a literal or expression to a '&' parameter` | Assign to a named variable first, then pass it. `v[i]` and `s.field` work directly. |
| `match arm separator is \`=>\`, not \`->\`` | Replace `->` with `=>` in the arm. |
| `Expect token ,` in a `match` | Separate the arms with commas — newline-separated arms are refused. |
| `'fn' definitions must be at file scope, not inside a function or block` | Move the helper fn out of the enclosing fn body.  Lambdas (`|x| { … }` or `fn(x: T) { … }`) are the only function-shaped values allowed inside a fn body. |
| `ambiguous variant 'Red' (a variant of Color, Berry)` | Qualify it (`Color.Red`), or give it a typed context |
| `error[local-out-of-scope]` | The local was bound inside a block that has ended — bind it before the block, or make the block's value the binding |
| `` `&` has nothing to bind here: this place holds the record itself `` | A `&` into a plain record field or element — assign the record (`d.o = p`, a copy) or declare the field `reference<T>`; `d.r = &p` on a `reference<T>` field stores the pointer |
| `` the base after `..` is a `E`, which may hold any of its variants `` | Name the variant: `..(base as V)` inside `if base is V`; for a generic enum, `match` on the base and build `V` from its fields |
| `` the base after `..` may be absent `` / `` must be a `S`, the literal's own type `` | Discharge the base first (`..(b ?? …)`), and give the literal the base's type |
| `` `E.V` is a variant of a generic enum, whose type arguments come from the target `` | Bind it to a typed local (`x: E<integer> = V`) or pass it where an `E<…>` is expected |
| `advice[match-scalar-hole]` | A scalar `match` with no `_`: add `_ =>` for the values the arms do not name, or `LOFT_NO_SCALAR_HOLE` for a deliberate hole |
| `` `Node.next: Node` makes every Node contain another Node — make it `Node?` `` | A struct recursing through non-null fields has no finite value: write `next: Node?` |
| `compound assignment is not supported for tuple destructuring — use (a, b) = expr instead` | Rebuild the tuple: `(a, b) = (a + 1, b + 2)` — or update each element directly. |

Compiler faults (`Too few parameters …`, `Allocating a used store`) are under § Known bugs.

---

## The entry point, and reading arguments

`fn main()` or `fn main(args: vector<text>)` — no other shape compiles, and there is no
`args()` builtin.  A file with no `main` is a library.  The CLI flags (`--native`,
`--native-wasm`, `--path`): [references/program-shape.md](references/program-shape.md).
---

## Pre-flight checklist

- [ ] Every Top trap checked against the code — above all a bind you write through (`&`?),
      a text cut (characters or bytes?), an index that can go negative, and arithmetic that
      can divide by zero
- [ ] Every local read after its block is bound before the block
- [ ] All `use` imports come before any other declaration
- [ ] No loop variable shares a name with a plain local in the same function
- [ ] No `character == text` comparisons — use `"{c}" == t`
- [ ] The probe ran on `--interpret` and every warning it printed is read, not skimmed
