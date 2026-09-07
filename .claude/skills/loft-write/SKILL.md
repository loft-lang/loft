---
name: loft-write
description: Reference for writing correct loft code. Apply whenever writing, editing, or reviewing .loft files. Covers types, syntax, known bugs, workarounds, naming rules, and error→fix table.
user-invocable: false
---

# Loft Language Writing Reference

Always consult this before writing or reviewing `.loft` files.

**Before reimplementing stdlib-or-library functionality, run `make libcatalogue` and read the generated `doc/claude/LIBRARIES.md` (built on demand, git-ignored), plus `loft install <name>` — a registered library may already provide it. Stdlib function signatures: [doc/claude/STDLIB.md](../../../doc/claude/STDLIB.md).** <!--noindex-->

---

## Naming conventions — enforced by the parser

| Construct | Convention | Examples |
|-----------|-----------|---------|
| Functions, variables | `lower_case` | `my_fn`, `item_count` |
| Types, structs, enums, variants | `CamelCase` | `Point`, `Color`, `Red` |
| Constants (file-scope) | `UPPER_CASE` | `PI`, `MAX_SIZE` |
| Operator definitions | `OpXxx` prefix | `OpAdd`, `OpEqInt` |

The parser **rejects** code that violates these rules.

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

**There is no `long` type and no `l` literal suffix** — both were
removed.  `integer` is already 64-bit (i64), so integer literals are
plain (`86400000`, not `86400000l`), and `now()` / `File.size` return a
full-range `integer`.  Writing `long` or `10l` is a parse error
(*"Undefined type long"* / *"Expect token ;"*).

**Narrow / sized integers are aliases of `integer`** (defined in
`default/01_code.loft`), for byte-width-sensitive file / wire formats:
`i8` `size(1)`, `i16`/`u16` `size(2)`, `i32`/`u32` `size(4)`.  Cast with
`x as i32` etc.  Use plain `integer` for ordinary arithmetic — it has
the full 64-bit range.

**Uncomputable arithmetic → null, and the program KEEPS RUNNING** (C80, the
spreadsheet model): divide / modulo by zero, and integer overflow
(`i64::MAX + 1`), are uncomputable, so they yield `null` and execution CONTINUES
— no trap, no halt, identically in dev / test / production.  Any arithmetic
landing exactly on `i64::MIN` also reads as `null` (the in-band sentinel).
Defend a site with `?? <fallback>` to turn the null into a value; an *unguarded*
divide-by-zero additionally logs a Warn (overflow is silent — the null is the
signal).  A function returning `integer` may `return null` (it yields `i64::MIN`,
detectable with `!result`).

**Prefer `x?` when the fallback is just the type's zero (@PLN116).** Postfix `?`
discharges `T?` to `T` using **the type's own** default, so `?? 0` / `?? 0.0` /
`?? ""` / `?? Pixel{r:0,g:0,b:0}` boilerplate collapses to one character.  `??`
supplies *the default you give*; `?` supplies *the default the type gives* — that
pairing is the mnemonic.

```loft
name  = row.label?          // text?   → ""      (was: ?? "")
first = points[i]?          // Point?  → Point{} (every field defaulted)
count = (a / b)?            // integer? → 0      (parens — see below)
```

Defaults: scalar → `0`/`0.0`/`false`/`'\0'`, `text` → `""`, collection → empty,
record → every field defaulted, nullable `U?` → `null`, bare enum → its
**first-declared** variant.

Three things to know before reaching for it:

- **`?` binds TIGHTEST, `??` binds LOOSEST** — opposite ends.  On a *binary*
  result that flips the parens: `a / b ?? 0` discharges the division, but
  `a / b?` is `a / (b?)` and evaluates to **null**.  Write `(a / b)?`.  So `?` is
  a clear win on a **primary** (`row.label?`, `v[i]?`, `x?` — no parens) and a
  wash on arithmetic.
- **On an enum it is positional.** `c?` is the first-*declared* variant, so
  reordering the enum silently changes it.  Use `?? Colour.Red` when the variant
  matters.
- **Two types have no default, so `x?` is a COMPILE error** (never a runtime one):
  a bare reference, and a record with a bare enum field lacking `= <variant>`.
  The error names the fixes; `??` or `match` is the fallback.

**At a text parse, put `?` after the CHECKED cast:**

```loft
n = (s as integer?)?     // integer — checked cast, then default ⇒ 0 on a bad parse
n = s as integer ?? 0    // integer — the assert-or-default form
n = s as integer?        // integer? — the nullable CAST (that `?` belongs to the type)
n = (s as integer)?      // COMPILE ERROR — a bare `as` on text is an assertion
```

A bare `text as integer` is an *assertion* the compiler refuses (a parse can't be
proven), so `(s as integer)?` fails on the inner cast — not because `?` is
unavailable.  Write the checked cast first.

**Parsing text is fallible, so `text as integer` types `integer?`** (`as float` →
`float?`, `as single` → `single?`).  `"42" as integer` is a **parse**, not a numeric
cast — a non-numeric string yields `null`, so the compiler types the result nullable
and **rejects storing it into a non-null slot**.  Discharge at the parse site:
`n = s as integer ?? 0` (default), `n = s as integer?` (keep it nullable), or `match`.
This is the same reachable-fault rule as `÷0` / out-of-bounds indexing (@PLN25
`(N-Parse)`).  A *numeric* `as` (`3.14 as integer`, `x as u8`) is not a parse and does
not add `?` on its own (a narrowing numeric cast that can miss uses `as u8?`).

**Nullability defaults (@PLN25 DN1) — every scalar and vector is NON-NULL by default.** A plain
`integer` / `float` / `single` / `boolean` / `character` / `text` (and `vector<T>`) field, local, or
return REJECTS `null`: storing `null` — or an undischarged `τ?` from a fit-failing op (parse, `/`,
`v[i]`) — into it is a compile error. To ALLOW null, append `?` to the type (`integer?`, `text?`,
`vector<T>?`). A `vector<T>` field defaults to `[]` (empty, non-null). **The old `not null` modifier
is a RETIRED accepted no-op** — it now means what the default already is, so don't write it; write
`?` when (and only when) you want the slot to hold null.

**`text` vs `string`:** The canonical string type is `text`. Using `string` in struct fields causes errors.

---

## Field modifiers (struct fields only)

```loft
struct Point {
    const id: integer,           // binding-const PREFIX: write-once at construction
    const cells: vector<Cell>,   // BUILDER: slot write-once, `cells += […]` still grows it
    coord: const Coord,          // value-const on the TYPE: frozen value (contents read-only)
    x: float,                    // non-null by default (DN1) — no `not null` needed
    r: integer limit(0, 255),
    label: text = "default",
    area: float virtual($.x * $.y),
    note: text?,                 // append `?` to ALLOW null
}
```

Modifiers: `limit(min, max)`, `default(expr)` / `= expr`, `virtual(expr)`.  (`not null` still parses
but is a retired no-op — see the DN1 defaults note above; use `?` for the nullable case.)

**Two const axes** (@PLN40) — `const` before the NAME freezes the *binding*; `const` before the
TYPE freezes the *value*.  They are opposites and compose:

| declaration | rebind `t.v=` | append `t.v+=` | element `t.v[i]=` | use |
|---|---|---|---|---|
| `v: T` | ✓ | ✓ | ✓ | plain |
| `const v: T` | ✗ | ✓ | ✓ | **builder** (grow in place) |
| `v: const T` | ✓ | ✗ | ✗ | **frozen value** (rebindable slot) |
| `const v: const T` | ✗ | ✗ | ✗ | fully immutable |

- **binding-const** (`const v: T`, prefix): write-once slot, contents stay mutable — the builder
  shape.  Reach for it when a field is assembled once then grown/edited in place (`const cells:
  vector<Cell>` with `cells += […]`).  `const virtual(...)` is rejected; works on enum-variant fields.
- **value-const** (`v: const T`, on the type): the value is read-only — append / element / nested
  writes (`t.v+=`, `t.v[i]=`, `t.r.x=` through it) are rejected; a whole-value rebind `t.v = other`
  is allowed.  Reach for it on a genuine value/record (shared config, a compound key, a `par()`-shared
  value).  Enforced for DIRECT writes; laundering via a local/return/generic is Phase 3 (not yet).
- **scalar collapse**: a by-value scalar (`integer/float/…`) freezes fully under EITHER axis —
  `const n: integer` and `n: const integer` both reject `t.n=` and `t.n+=`.

**Owner policy** (libraries): const every never-reassigned field.  Use `const v: T` for builder fields
you still append to; `v: const T` for true value/record fields; whole-value keys/config → both.

---

## Variable declarations

Variables declared by assignment — type is inferred from the initialiser.

```loft
x = 5;
s = "hello";
f = 3.14;
```

Explicit type annotations (sometimes required for empty collections):
```loft
v: vector<integer> = [];
n: integer? = null;             // a null needs a NULLABLE type; `null as integer` is a
                                // compile error (cannot cast null to non-null integer)
```

### `const` variables

```loft
const x = 5;       // immutable local — reassignment is a compile error
const t = "hello";  // works for any type
```

---

## Constants

The `const` keyword is accepted at file scope as well, alongside the
bare-name form.  Both produce the same file-scope immutable constant:

```loft
PI       = 3.14159265358979;    // bare-name form
const E  = 2.71828182845905;    // const-keyword form (P246, 2026-05-11)
pub const MAX_SIZE = 256;       // pub + const combine for exported constants
```

Constants must be `UPPER_CASE`.  Prefer the `const` form when you
want the immutability intent visible at the declaration site
(the bare-name form relies on the UPPER_CASE convention as the only
visual signal); both forms behave identically.

**UPPER_CASE locals get `advice[upper-case-local]` unless declared
`const`.**  Inside a function body, `FOO = 5;` (an assignment with an
UPPER_CASE name and no `const` keyword) emits advice telling you to
add `const` or rename to `lower_case`.  The convention "UPPER_CASE means immutable"
is enforced by this check at every scope.

---

## Functions

```loft
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
  expression is an error.
- **`&` that is never mutated is a compile error** — drop it if the param is read-only
- Omit modifier — pass by value/copy (heap values still alias: field/element mutation propagates)

A local reference is bound with `a = &b` (or `a: &T = b`); the operand must be addressable (a
variable / struct field / vector element). Full model: [LOFT.md § References](../../../doc/claude/LOFT.md).

A function body ending in an expression (no `;`) returns that value. Functions without `->` return `void`.

**Nested fn definitions are forbidden.**  `fn` declarations must be at file
scope.  Code like

```loft
fn outer() {
    fn inner() { ... }   // PARSE ERROR
}
```

produces *"'fn' definitions must be at file scope, not inside a function or
block"*.  Move helper fns alongside the caller, not inside it.  Lambdas
(`|x| { ... }` or `fn(x: T) { ... }`) are the only "function-shaped" thing
allowed inside a fn body.

---

## Imports

```loft
use arguments;                       // wildcard: all pub names bare + `arguments::` qualifier
use arguments::*;                    // explicit wildcard (same as above)
use arguments::parse_args;           // selective: just one name, bare
use arguments::(parse_args, Flag);   // selective group — MULTIPLE names need parentheses
use arguments::Flag as Opt;          // alias an imported name (bare `Opt`)
use arguments::(Flag as Opt, parse_args);   // per-name aliases inside the group
use arguments as args;               // library alias → `args::parse_args` (qualified-only,
                                     //   does NOT import names bare)
```

- **Multiple names MUST be parenthesised** — `use lib::a, b;` (flat comma list) is a
  hard error; write `use lib::(a, b);`.  (@PLN22 Phase 4.)
- `use lib as m;` gives a qualifier alias only (`m::fn`); it does not wildcard-import.
- A qualified `lib::fn()` auto-loads the library — an explicit `use` is optional for
  qualified access.

**`use` declarations must appear before any other declarations in the file.**

### Finding a library's API (do this BEFORE writing `use` calls)

Libraries live OUTSIDE the project (`~/.loft/registry/<name>-<version>/`,
`~/.loft/lib/<name>/`), so the project tree alone does not show what they
export.  Discovery surface, nearest first:

1. **`.loft/api/<name>.api`** in the project — generated public-API stubs
   (signatures + doc comments) for every locked dependency.  Written by
   `loft install` / `loft update` / `loft pin`; read these first.
2. **`.loft/api/_available.api`** — the registry CATALOG: every package you
   could `loft install` (name, latest version, one-line description), written
   alongside the per-dep stubs.  Read this to see what EXISTS, not just what's
   installed.
3. **`loft api`** — list every library reachable from the cwd (project deps,
   installed registry packages, user libraries) with their source paths.
4. **`loft api <name>`** — print one library's full public surface.
5. **`loft api --registry`** — print the whole installable catalog on demand
   (the live form of `_available.api`).  The catalog is cached ~1h; add
   **`--refresh`** to force a re-fetch (e.g. after a package was just published
   or its description changed).
6. **`loft search <query>`** / **`loft info <name>`** — query the registry
   for libraries not installed yet; `loft install <name>` fetches one and
   refreshes the stubs.

Never guess a library function's signature: check the stub or `loft api`
output, and read the real source at the path they name when you need the
implementation.

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

```loft
struct Item { name: text, count: integer }
item = Item { name: "foo", count: 0 };
item.count += 1;
```

Field names may overlap across structs — lookups are type-scoped.

---

## Tuples

Anonymous, fixed-arity, stack-allocated compound values.  Use them to
return multiple values without naming a struct.  Shipped in 0.8.3
(T1.1–T1.11); see [doc/claude/TUPLES.md](../../../doc/claude/TUPLES.md).

```loft
// Type notation — two or more element types (single-element tuples
// are not allowed; use the bare type instead).
t: (integer, text) = (3, "hi");

// Literal
pair = (1, "hello");

// Element access (zero-based integer literal — NOT a variable index).
a = pair.0;        // integer
b = pair.1;        // text

// Element assignment
pair.0 = 5;
pair.0 += 3;

// Destructuring
(lo, hi) = min_max(values);

// Function return
fn classify(t: (integer, text)) -> text {
    match t {
        (0, _)   => "zero",
        (n, msg) => "{n}: {msg}",
    }
}

// Nested tuples — chain the `.N` accessors.
nested: ((integer, integer), boolean) = ((1, 2), true);
inner_first  = nested.0.0;       // 1
inner_second = nested.0.1;       // 2
flag         = nested.1;          // true
```

**Restrictions:**
- **No single-element tuples** — `(integer)` is just `integer`.
- **No named tuple fields** — use a struct.
- **No tuple iteration / whole-tuple formatting** — access elements one
  by one.
- **Scalar elements are NON-NULL by default (@PLN25 DN1)** — a tuple
  `integer` / `text` / … element rejects `null`; append `?` to the element
  type (`(integer?, text)`) to allow it. (`not null` is a retired no-op.)
- **Compound assignment on tuple LHS is rejected** — `(a, b) += (1, 2)`
  is a compile error; rewrite as `a += 1; b += 2;` or rebuild the tuple.

Comparing a tuple-element `character` against a literal (`t.0 == 'a'`)
works on both backends — the old P207 native E0308 was fixed 2026-05-04;
no cast or destructure workaround is needed.

---

## Enums

Simple enum (value type, no fields):
```loft
enum Color { Red, Green, Blue }
c = Color.Red;                 // qualify when DEFINING a new variable
// ordering follows declaration order: Red < Green < Blue
```

**Bare variants need a type context (@PLN22 Phase 1).**  A bare `Red` resolves only
against a known enum — a `match` subject, a typed target (`c: Color = Red`), a
typed reassignment / field, a parameter, a return type, an `==` LHS, or an `is`
subject.  Defining a NEW untyped variable from a bare variant (`c = Red`) is a hard
error — qualify it (`c = Color.Red`) or annotate (`c: Color = Red`).  Match arms and
the other context positions still use the bare form (`match c { Red => … }`).  Two
enums MAY share a variant name (`enum Color { Red }` + `enum Berry { Red }`); the
bare `Red` resolves by its context enum.

Struct-enum (each variant has fields; polymorphic dispatch via methods):
```loft
enum Shape {
    Circle { radius: float },
    Rect   { w: float, h: float },
}
fn area(self: Circle) -> float { 3.14159 * self.radius * self.radius }
fn area(self: Rect)   -> float { self.w * self.h }

s = Circle { radius: 2.0 };
a = area(s);   // dispatches to correct variant
```

Plain enums cannot have methods — use struct-enum variants for polymorphic dispatch.

Trailing commas in variant field lists are accepted: `Circle { radius: float, }`.

JSON round-trip: `"{shape:j}"` produces `{"Circle":{"radius":3.14}}`; `Shape.parse(json)` reconstructs the correct variant.

---

## Vectors

```loft
empty: vector<integer> = [];
nums  = [for i in 0..10 { i * 2 }];    // comprehension
items = [1, 2, 3];

v += [element];      // append
v += other_vec;      // concatenate
len(v);              // length
v[i];                // index read
```

**Empty vectors** need a type annotation so the compiler knows the element type.

**Slices are iterators, materialised on assignment.** `sub = arr[lo..hi]` (annotated or not) builds a fresh vector, and negative bounds count from the end (`arr[2..-1]`, @P384).  A slice still cannot be passed directly where a `vector<T>` **argument** is expected — assign to a local first, or pass the array with index bounds.

**`v[i]` with a possibly-negative index does NOT null-guard.** Scalar indexing counts from the end for negative `i`, exactly like slices: `v[-1]` is the LAST element, `v[-len]` the first — NOT `null`.  Only `i >= len` (and `i < -len`) yield `null`.  So `if v[i] { … }` and `v[i] ?? d` catch an over-range index but NOT a negative one — a `-1` "not-found" sentinel or a `a - b` underflow silently reads a real element from the end.  When `i` can go negative, test `if i >= 0` FIRST (that `>= 0` check is not redundant with a later null-guard).

**Never swap `vector<STRUCT>` elements in-place via a temp (#338).** `tmp = v[j]` is a VIEW of slot j (not a copy), but `v[j] = v[k]` COPIES into the slot — so `tmp = v[j]; v[j] = v[k]; v[k] = tmp;` silently loses j's record and duplicates k's. Swap scalar fields one by one, or build a fresh vector (selection instead of in-place insertion sort).

---

## Hash collections

A hash works as a struct field or as a plain local:

```loft
struct Entry  { key: text, value: integer }
struct Table  { data: hash<Entry[key]> }

t = Table { data: [] };
t.data += [Entry { key: "x", value: 1 }];
e = t.data["x"];
if e == null { /* not found */ }
else { e.value += 1; }
t.data["x"] = null;   // remove entry

for e in t.data { … }  // iterates in key order (C60)
```

(`key` as the key-field name is fine; a standalone `data: hash<Entry[k]> = [];`
local is fine too.)

---

## Interfaces and bounded generics

```loft
interface Comparable {
  fn less_than(self: Self, other: Self) -> boolean
}

fn find_min<T: Comparable>(v: vector<T>) -> T { ... }
```

Structural satisfaction: if the methods exist, the type satisfies the interface.
No `impl` block needed. Built-in types satisfy `Ordered`, `Equatable`, `Addable`,
`Numeric`, `Scalable`, `Printable` automatically.

---

## The `both` parameter name

Name the first parameter `both` instead of `self` to register a function as
both a method and a free function:

```loft
pub fn exists(both: File) -> boolean { both.format != Format.NotExists }
// f.exists()  — method
// exists(f)   — free function
```

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

```loft
name = record.field ?? "default"   // null-coalescing
name = record.field?               // …or the TYPE's default ("" for text)
x as i32                           // cast to a 4-byte sized integer
flags & ~32                        // bitwise NOT — clears bit 5
```

**`?` and `??` are at opposite ends of this table**, so they parenthesise
oppositely on a binary result: `a / b ?? 0` discharges the division, `a / b?` is
`a / (b?)` and yields **null** — write `(a / b)?`.

### `is` variant check

```loft
if d is North { ... }              // boolean check
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

---

## String / text literals

### Double-quoted strings (`"..."`)

Single-line. Supports `{expr}` interpolation and `\n`, `\t`, `\\`, `\"` escapes.

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

**A format spec tunes what the value RENDERS as, so it works on every type**, not just
numbers and text: `{c:>5}` pads a character, `{v:>12}` a vector, `{p:*^16}` a struct. The
flags that choose the rendering (`#`, `:j`) are separate from width and alignment and combine
with them.

### Backtick strings (`` `...` ``)

**Multi-line.** Supports `{expr}` interpolation. Bare `"` is literal.
Auto-strips common leading indentation (based on closing backtick column).

```loft
SHADER = `
  #version 330 core
  void main() {
      gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
  }
`;
```

Use `println()` for line-oriented output and `print()` for output without a newline.

---

## Control flow

```loft
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

```loft
for i in 0..n { }         // exclusive: 0 to n-1
for i in 0..=n { }        // inclusive: 0 to n
for item in collection { }
for c in some_text { }    // character iteration
for item in col if item.active { }   // filtered iteration
for i in rev(0..n) { }    // reverse range
```

### Loop attributes

```loft
for v in collection {
    v#index    // 0-based position (vector/sorted); byte offset (text)
    v#count    // iterations completed so far (all types)
    v#first    // true for first element (all types)
    v#next     // byte offset after current char (text only)
    v#remove   // remove current element (filtered loops only)
}
```

`#index` is NOT supported on `index` collections — use `#count` there.

---

## Match

```loft
match color {
    Red   => println("red"),
    Green | Blue => println("cool"),
    _     => {},
}

match shape {
    Circle { radius } => println("r={radius}"),
    Rect { w, h } if w == h => println("square"),
    _ => {},
}

// Tuple match — destructure into element patterns.
match (3, 7) {
    (0, _)   => "zero",
    (n, m)   => "{n},{m}",
}
```

Match is an expression — all arms must produce the same type (or void).

**The arm separator is `=>`, never `->`.**  `->` is the lambda /
function return-type arrow; using it in a match arm produces a clear
diagnostic ("match arm separator is `=>`, not `->`") — but only because
the parser was hardened against it.  Older drafts of TUPLES.md showed
`->` for arms; that was always wrong.  If a `match` arm in your code
uses `->`, fix it before running anything.

**Scalar-match arms need commas between them**; enum and tuple match
also accept newline-separated arms.  When in doubt, comma-separate —
it's universally accepted.

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

```loft
fn double(x: integer) -> integer { x * 2 }

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

**Type annotations on `|x|` shorthand are rejected by design
(see `doc/claude/DESIGN_DECISIONS.md § C62`).**  If you need
types, switch to `fn(name: <type>) { ... }` — the shorthand
exists specifically *because* the types are inferred; adding
annotations collapses the distinction between the two forms.

---

## Variable scoping — one type per name, per function

Variable names are **per-function**, and loft has **no block scoping**: a
`for`/`if` body does not open a fresh binding — every local (loop variables
included) lives in the enclosing function's scope for the whole function.  So the
rule that matters is narrow: *within one function, a name maps to a single
slot + type.*  The same name in a **different** function is completely free.

All of the violations below are **clean compile-time errors with fix hints** —
never a codegen panic or silent corruption, on either backend:

- **A name reused with a different type in one function** → error, even across
  disjoint blocks: `if … { x = 1 }  if … { x = "hi" }` →
  `Variable 'x' cannot change type from integer to text`.  Re-assigning the *same*
  type is fine (`x = 1; x = 2`).
- **A loop variable named like an existing local** →
  `loop variable 'x' shadows a local named 'x' — rename the loop variable
  (e.g. loop_x)`.  Rename it, or drop the dead outer local.
- **Nested same-name loops** → `for i { for i { … } }` is rejected: the inner
  binding would take over `i` for the rest of the outer body.
- **Loop variables are inference-only (@P345)** — `for i: integer in …` does not
  parse (`loop variable 'i' is type-inferred from the iterable — remove the
  ': <type>' annotation`).  Drop the annotation.

**`for` loops are the exception to the one-type-per-name rule** (loft#915): each
`for` binds its own variable, so two loops in one function may reuse a name at
different element types — `for i in ["a","b"] {…}` then `for i in 0..3 {…}`
compiles.  Reading the variable after the loop still works and gives the *last*
loop's value.  You therefore do **not** need per-function loop-variable prefixes;
short names (`i`, `e`, `n`) are fine and read better.

**Unused variable = warning, not an error** — the program still runs (exit 0).
Use `_` for an unused loop variable to keep the build warning-clean.

---

## Reserved words with no implementation

`debug_assert` is a reserved keyword and does nothing yet — using it in any position is a
compile error naming the reservation. Use `assert(cond, "message")`, and do not use
`debug_assert` as a variable name. `sizeof(Type)` IS implemented (`sizeof(integer)` is 8).

## Builtin names — shadowing rules (@PLN22 Phase 2)

- **Definitions CAN shadow a stdlib/library name.**  `enum E`, `struct File`,
  `pub PI = 3` are legal even though the stdlib has `E`/`File`/`PI`; your name wins
  bare resolution and `std::Name` still reaches the original.
- **Built-in TYPE-KEYWORDS are reserved** — you cannot define `struct integer`,
  `enum vector`, etc. (it errors: "conflicts with a type").  Reserved: `integer`,
  `float`, `single`, `text`, `boolean`, `character`, `vector`, `hash`, `sorted`,
  `index`, `radix`, `spatial`, `iterator`, `i8`/`i16`/`i32`, `u8`/`u16`/`u32`,
  `reference`.  This holds for `struct`, `enum`, and `type` alike.
- **A function's name is free as a local** (loft#852).  Values and functions are
  separate namespaces: `chr = 65` binds a local and `chr(65)` in the same scope
  still calls the function.  True for every binding form — assignment, the typed
  local, a tuple element, a parameter, a `for` variable, a struct field — and for
  library functions as much as stdlib ones, which is what stops a library's new
  `pub fn` from taking a word away from its consumers.
- **One builtin name still breaks as a local variable name** — the literal `null`
  (assigning to it errors: `Cannot assign integer to a field of type null — use 'as
  null' to cast explicitly`).  Method names (`map`/`filter`/`reduce`/`len`/`sorted`/…)
  are fine as locals; the method call still resolves.

---

## Text pitfalls

**There is no `substr` / `substring`.** A part of a text is a slice:

```loft
s = "abcdef";
s[1..3]                 // "bc" — BYTE positions
s.char_slice(1, 3)      // "bc" — CHARACTER positions
```

**Pick by where the numbers came from**, and this is the commonest text bug in this stack:
`len` counts CHARACTERS, `size` and `s[a..b]` count BYTES. A character count used as a byte
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

### Path resolution — relative means program-relative

A **relative** path resolves against the **program's own directory**
(`source_dir()` — source dir under `--interpret`, exe dir under `--native`), not
the process cwd.  So `file("assets/x")` loads the asset bundled beside the
program on every backend, regardless of launch directory.  **Absolute paths are
untouched.**  The file builtins (`file`, `exists`, `read_file`/`write_file`,
`delete`/`move`/`mkdir`, image loads) all resolve this way — so **don't hand-roll
`"{source_dir()}/{path}"` joins** in normal loft; just pass the relative path.
(Do the explicit join only when handing a path to a *non-loft* consumer, e.g. a
native asset loader that reads the filesystem directly.)

A program that must resolve a *user-supplied* relative path against the **working
directory** (CLI tools — `loft tidy.loft data.csv`) declares `#cwd` as the
file-top directive (before the first declaration).  `LOFT_PATHS=program|cwd`
overrides per-invocation.

### Text files (UTF-8)

```loft
f = file("path/to/file.txt");
content = f.content();         // full text content (UTF-8)
lines = f.lines();             // vector<text> of lines

out = file("output.txt");
out.write(content);

dir = file("some/directory");
for ef in dir.files() {
  path = ef.path;
}

if f.exists() { }
size_bytes = f.size;            // integer (i64) — works for any file
```

**`f.content()` is UTF-8-only** (`-> text?`).  On a binary file it
returns **null** with a warning.  For non-text data use
`read_bytes(path) -> vector<u8>` / `write_bytes`, or the structured
binary idiom below.

### Binary files (structured reads and writes)

Set `f#format` to `LittleEndian` or `BigEndian`, then use `f#read`
for reads and `f += value` for writes.  `#next` seeks to an
absolute byte offset.  All file-handle operations should live
inside a `{ ... }` scope block so the handle flushes/closes at
block exit:

```loft
// --- Read a 12-byte GLB header ---
{
  f = file("model.glb");
  f#format = LittleEndian;
  magic   = f#read as i32;            // 0x46546C67 = 'glTF'
  version = f#read as i32;
  total   = f#read as i32;            // declared file length
  // Seek past the header + JSON data to a later chunk:
  f#next = (20 + json_len) as integer;
  bin_len = f#read as i32;
}

// --- Write a binary chunk-structured file ---
{
  f = file("model.glb");
  f#format = LittleEndian;
  f += (0x46546C67 as i32);   // 4 bytes: i32 magic
  f += (2 as i32);            // 4 bytes: i32 version
  f += (32 as u8);            // 1 byte (ASCII space)
  f += "chunk of text";       // raw UTF-8 bytes
  f += my_float_vector;       // vector<single> → 4 bytes per element
}
```

Notes:
- **Prefer `f#read as <type>` (no parens) for fixed-width reads.**
  The byte count is inferred from the type — `as i32` reads 4
  bytes, `as u8` reads 1, `as u16` reads 2, `as integer` reads 8.
  The legacy `f#read(n) as T` form still works but the `(n)` must
  match the type's storage width exactly or the runtime panics in
  `src/database/io.rs:276`.  The inferred form makes that mismatch
  impossible.  `as text` still needs `f#read(n) as text` because
  text has no fixed width.
- **`s.field = f#read` (no `as T`) infers width from the LHS field's
  declared type** — symmetric with `f += s.field`.  For a struct
  `S { a: i32, b: u8, c: u16 }`, both sides become:
  `f += s.a; f += s.b; f += s.c` to write, `s.a = f#read; s.b = f#read;
  s.c = f#read` to read.  Changing a field's declared type
  (`i32` → `i64`) automatically updates both sites at the next compile —
  no manual cast edits needed.
- **Always cast scalar writes to the intended width.**  Bare
  `f += int_var` writes 8 bytes (loft stores integers as i64).  To
  write 4 bytes use `f += (int_var as i32)`; for 1 / 2 bytes use
  `as u8` / `as u16`.  Strongly-typed struct fields (`u8`, `u16`, or a
  range-limited `integer limit(0, 255)`) write at their declared width
  automatically.
- `f += expr` appends `expr` to the file, respecting the `#format`
  endianness.  `text` → raw bytes, `vector<T>` → each element in
  sequence at its declared width.
- `f.size` returns an `integer` (i64); compare with `0`.
- `f#next = offset as integer` seeks.  Reading position advances
  automatically after each `f#read` — don't manually advance it
  between sequential reads.
- **Whole-buffer reads: `read_bytes(path) -> vector<u8>`** (and
  `write_bytes(path, v)`), from the stdlib.  Reach for the `f#read`
  loop only for structured, offset-driven access.

Example binary reader/writer patterns live in
`tests/fixtures/libs/graphics/src/glb.loft` (writer) and
`tests/fixtures/libs/graphics/tests/glb.loft` (reader).

---

## Nullable defaults, the copy-write hazard, and null-checking reads

**A plain `vector<T>` field is non-null and defaults to `[]`** (@PLN25 DN1) — declare
it plainly; write `vector<T>?` only when null must be distinct from empty.  The
`not null` modifier still parses but is a retired no-op — don't add it.

**Never capture a vector into a local in order to write through it.**  A
whole-value bind COPIES the heap value (C86), so every write lands in the
copy and is silently lost — indexed writes, whole-element assigns and
appends alike:

```loft
// BAD — `results` is a COPY; all three writes are discarded:
results = self.results;
results[i] = "true";               // lost
results[i] = Row { id: 1 };        // lost
results += [Row { id: 2 }];        // lost

// If you need a local at all, bind a live reference with `&`:
results = &self.results;           // aliases the field
results[i] = "true";               // propagates
```

Reads through a plain copy are correct, which is what makes this
expensive to find: `len`, indexing and field reads all behave, and only
the writes vanish — arbitrarily far from where the value is later found
missing (loft#670).  The compiler warns on the simple shape (`w = v`
followed by an unread `w[i] = …`), but a bound-guard such as
`if i < len(results)` counts as a read and silences it, so the diagnostic
is a backstop, not a guarantee.

**Reading a maybe-null element `v[i]`, pick by contract:** `v[i] ?? sentinel` when a
non-null default is meaningful (changes behaviour on OOB); `if i < len(v) { v[i] }`
when the access is index-bounded (works on a struct field directly — no capture
needed); `x = v[i]; if x != null { … }` when the consumer's contract is
"missing → null".

---

## Known error messages → fixes

| Error message | Fix |
|--------------|-----|
| `Too few parameters on n_<fn>` | Per-function name collision — give the loop/local a name distinct from the function's params; avoid `for` in `const vector<T>` recursive fns |
| `Variable <x> is never read` | A **warning** (program still runs, exit 0) — use the variable, or name an unused loop var `_` |
| `Indexing a non vector` | You indexed a scalar (`x = 5; x[0]`) — index a vector/collection, not a single value |
| `Cannot assign <T> to a field of type null` | A local named `null` (a literal keyword) — rename it |
| `Undefined type string` | Use `text`, not `string` |
| `Allocating a used store` | A store is being reused while still held — check parallel blocks / store lifetimes (a `key` field name is NOT the cause; that works) |
| `Unknown function say` | Use `println()` |
| `Cannot pass a literal or expression to a '&' parameter` | Assign to a named variable first, then pass it. `v[i]` and `s.field` work directly (P160). |
| `match arm separator is \`=>\`, not \`->\`` | Replace `->` with `=>` in the arm.  (P206 — was a parser hang before the recovery helper landed.) |
| `'fn' definitions must be at file scope, not inside a function or block` | Move the helper fn out of the enclosing fn body.  Lambdas (`|x| { … }` or `fn(x: T) { … }`) are the only function-shaped values allowed inside a fn body. |
| `compound assignment is not supported for tuple destructuring — use (a, b) = expr instead` | Rebuild the tuple: `(a, b) = (a + 1, b + 2)` — or update each element directly. |
| Native E0308 on `t.0 == 'a'` where `t` is `(character, …)` | Fixed (P207, 2026-05-04) — if seen on an old build, update loft; current builds compile this on both backends. |

---

## The entry point, and reading arguments

```loft
fn main() { println("hi"); }                 // the usual form
fn main(args: vector<text>) {                // the invocation arguments
  for a in args { println(a); }              // `loft p.loft Ada Grace` → Ada, Grace
}
```

**One `vector<text>` parameter is the ONLY supported shape.** It works on both backends.
Any other spelling is a clean compile error (*"`main` takes no parameters, or one
`vector<text>` …"* — loft#1172 made the old silent-garbage fill a refusal). There is
no `args()` builtin.

A file with no `main` is a library: the interpreter runs nothing, and `--native`
compile-checks it rather than linking.

## CLI invocation

```bash
loft --path /path/to/repo/ file.loft                        # interpreter
loft --native --path /path/to/repo/ file.loft               # compile + run native
loft --native-wasm out.wasm --path /path/to/repo/ file.loft # compile to wasm
```

`--path` takes the directory containing `default/`; a trailing slash is optional.

---

## Pre-flight checklist

- [ ] No loop variable shares a name with a plain local in the same function (loops may share names with each other)
- [ ] No nested `fn` definitions — helpers live at file scope
- [ ] No `arr[lo..hi]` passed as `vector<T>` argument
- [ ] No local named `null`
- [ ] All `use` imports appear before any other declarations
- [ ] No `long` type / no `l` literal suffix — `integer` is i64; literals are plain (`86400000`), `f.size` compares with `0`
- [ ] String type in struct fields is `text`, not `string`
- [ ] No `character == text` comparisons — use `"{c}" == t`
- [ ] Never reassign a text parameter — copy to local first
- [ ] `v[i]` and `s.field` can be passed directly as `&` parameters
- [ ] Match arm separator is `=>`, never `->` (the parser used to hang on this — P206)
- [ ] No single-element tuples; `(integer)` is just `integer`
- [ ] Tuple element access uses an integer literal (`t.0`, not `t.i`)
- [ ] No compound assignment on a tuple LHS (`(a, b) +=` is rejected)
