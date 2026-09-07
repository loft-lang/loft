
# Loft Language Reference

Loft is a statically-typed, imperative scripting language with null safety and built-in parallel execution.
Source files use the `.loft` extension. The language compiles to an internal bytecode representation
and can emit Rust code for host integration.

**Quick reference with common patterns and gotchas:** see the loft-write skill (`.claude/skills/loft-write/SKILL.md`).

---

## Contents
- [Naming Conventions (enforced by the parser)](#naming-conventions-enforced-by-the-parser)
- [Types](#types)
- [Scripts (a file with no `fn main`)](#scripts-a-file-with-no-fn-main)
- [Declarations](#declarations)
- [Operators](#operators)
- [Literals](#literals)
- [String formatting](#string-formatting)
- [Control flow](#control-flow)
- [Variables](#variables)
- [Vectors](#vectors)
- [Key-based collections (hash / index / sorted)](#key-based-collections-hash--index--sorted)
- [Structs and record initialization](#structs-and-record-initialization)
- [Methods and function calls](#methods-and-function-calls)
- [Assertions](#assertions)
- [Sizeof](#sizeof)
- [Polymorphism / dynamic dispatch](#polymorphism--dynamic-dispatch)
- [File structure](#file-structure)
- [External function annotations (`#rust`, `#iterator`)](#external-function-annotations-rust-iterator)
- [Operator definitions (internal)](#operator-definitions-internal)
- [Shebang](#shebang)
- [Summary of grammar (informal)](#summary-of-grammar-informal)
- [Best Practices](#best-practices)
- [Design decisions and constraints](#design-decisions-and-constraints)

---

## Naming Conventions (enforced by the parser)

| Construct              | Convention         | Examples               |
|------------------------|--------------------|------------------------|
| Functions, variables   | `lower_case`       | `my_fn`, `count`       |
| Types, structs, enums  | `CamelCase`        | `Terrain`, `Format`    |
| Enum values            | `CamelCase`        | `Text`, `FileName`     |
| Constants              | `UPPER_CASE`       | `PI`, `MAX_SIZE`       |
| Operator definitions   | `OpXxx` prefix     | `OpAdd`, `OpEqInt`     |

---

## Types

### Primitive types

| Type        | Description                                      |
|-------------|--------------------------------------------------|
| `boolean`   | `true` / `false`                                 |
| `integer`   | 64-bit signed integer end-to-end (stack, fields, arithmetic).  Range can be constrained with `limit(...)`; narrow widths (`u8`/`u16`/`i8`/`i16`/`i32`) keep compact storage.  Overflow yields null and continues (the spreadsheet model).  See "Arithmetic safety" below. |
| `float`     | 64-bit floating-point; literals contain a `.`    |
| `single`    | 32-bit float; literals end with `f`              |
| `character` | A single Unicode character                       |
| `text`      | A UTF-8 string; `len()` counts bytes             |

A type is **non-null by default**: a plain `integer` / `text` / `Row` is not a slot for
`null`, and the compiler says so wherever one is written — discharge it (`??`, `?`,
`match`) or declare the slot `integer?`. Add `?` to make it nullable: `integer?` holds a
value or `null`. (The old `not null` modifier is now the default and is **deprecated** —
it parses as a no-op and warns; delete it. See "Fields" below.)

How loudly depends on whether the slot has room for a null. Declaring a **local** `x: Row
= null` is refused outright. A **field**, a **return**, a **vector element** and a **call
argument** get a warning and the store proceeds, because the slot does reserve a null it
can hold and read back — with one exception: a narrow width (`u8`…`u32`) spends its whole
range on real values, so a null there is an error too. This applies to every type, records
and collections alongside the scalars.

**Non-null is a rule about writes, not a guarantee about reads.** A *fault* can still
leave the reserved pattern in a non-null slot: an integer overflow writes the sentinel
and the slot then reads `null`, and a non-null `float` can hold a `NaN` the same way
(the deliberate, bounded edge in
[DESIGN_DECISIONS C85](DESIGN_DECISIONS.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--)).
A width with no code to spare — `u8`, `i8`, `u16`, `i16` — cannot hold the pattern at
all, so the same fault leaves the type's default there instead. So `x == null` on a
non-null slot is not dead code.

#### Null representation

Loft uses in-band sentinel values to represent `null`. Each type has a dedicated sentinel:

| Type | Null sentinel | Notes |
|------|---------------|-------|
| `boolean` | byte `255` (@PLN17) | Three-state: `false`=0, `true`=1, `null`=255 — stored like a 2-variant plain enum.  `null` is held and distinguished everywhere a boolean lives (locals, params, returns, fields, vector/keyed elements); test it with `b == null`.  `null` coerces to `false` only in boolean *logic* (`if`/`while`/`!`/`&&`/`\|\|`); `==`/`!=` are raw, so `null == false` is `false`.  A `boolean not null` is 2-state (false/true).  `!b` is true for both `null` and `false` (both coerce to false). |
| `integer` | `i64::MIN` | Post-2c (0.9.0): 8-byte storage, sentinel moved from `i32::MIN`.  Accidental sentinel collisions effectively vanish at the i64 boundary. |
| `float` | `NaN` | IEEE 754: `NaN != NaN`, but `!f` correctly detects null |
| `single` | `NaN` (32-bit) | Same as `float` |
| `character` | `'\0'` (NUL) | The null character is not a valid loft character value |
| `text` | internal null pointer | Opaque; `!t` detects it; `len(t)` returns null |
| `reference` | record 0 | Opaque; `!r` detects it |
| plain `enum` | byte `255` **or** byte `0` | Limits plain enums to 255 variants.  **Two bytes mean absent, and every test accepts both:** an explicit `null` writes `255`, while zero-initialised storage and a read of an absent record produce `0`.  Variants are numbered from 1, so `0` is a variant of no enum — which is why the renderer has always shown both as `null`.  A `??` that recognised only `255` let `v[9] ?? d` and a field never set answer `null` through a coalesce. |
| narrow int (`u8`/`u16`/`i8`/`i16`) | top of the packed range | e.g. `i8::MIN` for `i8`; stored compactly in `Parts::Byte`/`Short` |
| `i32` / `integer size(4)` | `i32::MIN` | 4-byte storage via `Parts::Int`; widens to i64 on the stack |

**Arithmetic safety — the spreadsheet model (C80, [formal/operational.md](formal/operational.md)):**
integer arithmetic that can't produce a value — overflow (`i64::MAX + 1`,
`i64::MIN - 1`, `i64::MAX * 2`), or `/` / `%` by zero — yields **null and keeps
running**.  It does NOT trap or halt.  A bad calculation degrades that one value
(it becomes null); every later statement still executes (null is contagious — a
consumer of the null gets null too — but it *runs*).  This holds identically in
development, test, and production: a calculation fault never stops the run.

```loft
a = 9223372036854775807;     // i64::MAX
b = a + 1;                   // b = null  (overflow → null, NOT a wrapped value)
c = a / 0;                   // c = null  (divide-by-zero → null, execution continues)
print("done");               // ALWAYS reached
```

**`??` — a non-null fallback** (it is the null-fallback, not a trap-rescue):
`expr ?? default` yields `expr` when it is non-null, else `default`.  Use it to
turn a null result into a usable value at the spots that need one:

```loft
x = (a * b) ?? 0;            // overflow → null → x = 0
y = total / count ?? 0;      // count == 0 → null → y = 0
```

**Observability.** An *unguarded* divide-by-zero (no `??` / null-check) also
emits one Warn log, so an undefended fault is not invisible; a *guarded* site is
silent.  Overflow is silent (the null result is the signal — and silent overflow
is the Rust-release default, except loft's result is null rather than a wrapped
wrong number).  To trace faults, opt into the debug log level.  The compiler also
warns at the unguarded site (`consider a / b ?? 0`).

The null sentinel is `i64::MIN` for `integer` (`NaN` for `float`/`single`).
Narrow alias fields (`i32` / `u8` / `u16` / `i8` / `i16`) store their own narrow
sentinel but widen to i64 on the stack, so arithmetic is uniform.  Both the
interpreter and the native backend produce the same null on the same fault.

**History:** 0.9.0's C54.G-hybrid made overflow / div0 **trap** (a halt, with a
`??`-only discharge).  C80 (the spreadsheet model) reverses that to
null-and-continue everywhere — so `??` is now a plain fallback, not a trap mode.

**Binary file I/O caveat:** post-2c `f += <integer_expression>` on a
`BigEndian` / `LittleEndian` file writes **8 bytes**.  Pre-2c
wrote 4.  Writers of binary formats must add an explicit width
cast — `f += 2 as i32;` for a 4-byte u32 field, `f += 0 as u8;`
for a byte, `f += v as u16;` for a 2-byte field.  The GLB / PNG
writers in the stdlib were updated accordingly; custom binary
protocols need the same audit.

**`!value` asymmetry — read carefully:** the unary `!` operator reads as "is null
or default?" but the answer differs by type because the null sentinel is in-band.
For `boolean`, `!b` is true for **both** `null` and `false` (both coerce to false in
boolean logic) — so `!b` alone can't tell them apart; use `b == null` for that
(@PLN17 — boolean is now three-state, false=0 / true=1 / null=255).  For `integer`,
`0` is a valid non-null value — `!n` fires **only** for `i64::MIN`, not for `0`.  Code
ported from a boolean guard to an integer guard (or vice versa) silently changes
meaning:

```loft
flag: boolean = false;
if !flag { /* runs */ }     // catches both null and false

count: integer = 0;
if !count { /* skipped */ } // catches only null; zero passes through
```

The idiomatic "zero or null" check on an integer is `count == 0 or !count`,
or simply `count == 0` if the sentinel and zero should be treated the same.
This asymmetry is a deliberate, settled choice — see
[DESIGN_DECISIONS.md § C69](DESIGN_DECISIONS.md#c69--x-on-a-non-boolean-is-a-null-test-not-logical-not).
The compiler warns when `!` is applied to a statically `not null` operand
(`!x` there is always false, since the value can never be the null sentinel).

Integer ranges can be constrained with `limit`:
```
integer limit(-128, 127)   // fits in a byte
integer limit(0, 65535)    // fits in a short
```

The default library also defines convenient width-specific aliases:
```
u8    // integer limit(0, 255)              — 1 byte unsigned
i8    // integer limit(-128, 127)           — 1 byte signed
u16   // integer limit(0, 65535)            — 2 bytes unsigned
i16   // integer limit(-32768, 32767)       — 2 bytes signed
i32   // integer (explicit 32-bit)          — 4 bytes signed, -2147483647..=2147483647
      //                                      (i32::MIN is the null sentinel, not a value)
u32   // integer limit(0, 4_294_967_294)    — 4 bytes unsigned (post-2a)
```

**Nullable narrow fields reserve one code for null** (decision 2026-06-11,
#334).  The convention runs through every width:

- a nullable `u8` field holds **255** distinct values (`0 ..= 254`) — the
  byte's 256th code is the null sentinel;
- a nullable `u16` field holds **65 535** distinct values — one short code
  is the sentinel (the `+1` storage encoding reserves raw `0`);
- `u32` covers `0 ..= 4_294_967_294` — one 32-bit code reserved.

The reserved code is expressed in the effective `min`/`max` of the integer
type behind the field, and the read/write contract is symmetric: reading a
narrow field into an `integer` widens the stored sentinel to integer null,
and writing integer null (or `null`) stores the sentinel — a null always
round-trips, at every width.  `not null` on the field unlocks the full
range (256 / 65 536 / 2³²) when it never carries null.  Typical `u32` use:
RGBA pixels, large file offsets, bitmasks wider than i32.

**Migration note:** the `long` type keyword and the `l` literal
suffix (e.g. `42l`) were removed in 0.9.0.  There are no external
users of pre-0.9.0 loft, so no migration path is needed in
practice; the `loft --migrate-long <path>` CLI exists as an
internal utility should one become necessary.

### Composite types

| Type syntax                        | Description                                           |
|------------------------------------|-------------------------------------------------------|
| `vector<T>`                        | Dynamic array of `T`                                  |
| `hash<T[field1, field2]>`          | Hash-indexed collection of `T` on the given fields    |
| `index<T[field1, -field2]>`        | B-tree index (ascending/descending)                   |
| `sorted<T[field]>`                 | Sorted vector on the given fields                     |
| `spatial<T[x,y]>` / `spatial<T[x,y,z]>` | Spatial keyed collection, 1–3 coordinate axes, Morton/Z-order radix tree |
| `trie<T[field]>`                   | Text-keyed collection on ONE field — exact lookup, key order, and prefix |
| `reference<T>`                     | Reference (pointer) to a stored `T` record            |
| `reference<T>?`                    | The same pointer, allowed to be absent — a list terminator |
| `iterator<T, I>`                   | Iterator yielding `T` using internal state `I`        |
| `fn(T1, T2) -> R`                  | First-class function type                             |

The key fields are declared **inside** the angle brackets with the element type.
A `-` prefix on a field name means descending order:
```
sorted<Elm[-key]>           // single key, descending
index<Elm[nr, -key]>        // two keys: nr ascending, key descending
hash<Count[c, t]>           // compound hash key
```

**A key field may be a TUPLE**, which is a compound key spelled as one field. It behaves
exactly as its elements spelled out would: element 0 orders, element 1 breaks its ties, and
so on, nested tuples included. Look it up by writing the tuple.

```loft
struct Cell { pos: (integer, integer), name: text }

h: hash<Cell[pos]> = [Cell { pos: (1, 2), name: "a" }];
c = h[(1, 2)];              // a literal, a local, or a vector<(…)> element all work

s: sorted<Cell[pos]> = [];  // (1,2) before (1,9) before (2,0)
d: sorted<Cell[-pos]> = []; // `-` reverses the WHOLE tuple
```

A `trie<T[field]>` is the exception: it keys on the BYTES of ONE text field, so a tuple key
is refused with a message pointing at `sorted` / `index` / `hash`.

**Gotcha — iteration direction is declared on the struct, not on the query.**
A `-` prefix on a key field in `sorted<T[-key]>` or `index<T[-key]>` flips
the iteration direction of *every* query against that collection — plain
`for v in db.map`, range queries, and partial-key lookups all walk
descending instead of ascending.  Reading the query site alone never
reveals the direction: the `-` lives in the struct declaration, possibly
hundreds of lines away.  When reviewing a query, cross-check the index
declaration before reasoning about what "starts at X" means.
`index` and `sorted` answer identically for the same declaration — the `-` is applied
by the key comparator and by nothing else, so a range names its bounds in the
COLLECTION's key order too: on `[-key]` the walk runs high-to-low, which makes
`c["c".."a"]` the non-empty half and `c["a".."c"]` the inverted, empty one.
Regression guards in `tests/issues.rs` (`inc12_sorted_ascending_iterates_forward`,
`inc12_sorted_descending_iterates_backward`) lock the two directions on
otherwise-identical `sorted` structs, and
`tests/scripts/1267-a-descending-key-orders-the-index-once.loft` does the same for
`index`, pairing every row against its `sorted` twin.

### Enum types

Simple enums (value types):
```
enum Format {
    Text,
    Number,
    FileName
}
```

Simple enum values support all six comparison operators (`==`, `!=`, `<`, `<=`, `>`, `>=`).
The ordering follows declaration order (`Text < Number < FileName`).

Polymorphic enums (each variant has its own fields, stored as a record):
```
enum Shape {
    Circle { radius: float },
    Rectangle { width: float, height: float }
}
```

A variant's fields are read **directly** — `s.radius` — with `match` / `is` reserved for
*dispatch* rather than extraction ([C89](DESIGN_DECISIONS.md#c89)).  A field EVERY variant
declares shares one slot and reads correctly from any of them.

**A field only SOME variants declare answers for the variant the value holds** (loft#980).
The access resolves at compile time to the variant that declares it, and the tag is checked
at run time: on any other variant the read answers **null** — the same answer a hash miss
and an out-of-range index give — and a write to it is **ignored** rather than landing in
that variant's bytes.  The value's own fields are untouched and its tag never changes.

The access TYPE is unchanged, so the null is a value the type does not advertise, exactly
as for an out-of-range index; `warning[variant-field-unchecked]` names each such access and
the variants that have the field, because a silently ignored write is a lost write.  Reach
it per-variant instead:

```
if s is Circle { radius } { area = PI * radius * radius }
// or
match s {
    Circle { radius }            => …,
    Rectangle { width, height }  => …,
}
```

### Struct types

```
struct Argument {
    short: text,
    long: text,
    mandatory: boolean,
    description: text
}
```

Fields are declared as `name: type` with optional modifiers **after** the type:
- `limit(min, max)` — constrain an integer field to a range
- `not null` — **deprecated no-op** (fields are non-null by default; it parses but
  warns). Delete it; write the type as `T?` if the field should allow `null`.
- `= expr` — stored default value, applied when field is omitted in constructor.
  The expression may build a value of its own — `= [1, 2]`, `= "a" + "b"`, `= mk()`,
  `= P { … }`, `= [K { … }]` for a keyed field — and is evaluated once per
  construction. One form is refused, and says so: an expression that reads `$` **and**
  needs a temporary, since a `$`-reading default is built against the record at each
  construction site and so cannot be built once and shared.

  **Where a default reaches depends on whether it is a constant** (loft#876). A default
  that is a LITERAL — `= 1.5`, `= 7`, `= "hi"`, `= true` — is part of the type: it is
  carried on the stored schema, so it answers a `text as Struct` / `text as vector<Struct>`
  cast for a key the document omits or writes `null`, exactly as it answers a struct
  literal that leaves the field out. Any other default is *computed*, and the store layer
  that fills a cast has no evaluator to run it, so it applies **where the constructor
  runs** and not to a cast:

  ```
  struct D {
      height: float   = 1.5,     // constant → literal AND cast both give 1.5
      area:   float   = 1 + 2,   // computed → literal gives 3, a cast gives 0.0
  }
  ```

  A key the document actually carries always beats the default, either way. When a
  computed default must survive a cast, write the field into the JSON or assign it after
  the cast.
- `assert(expr)` / `assert(expr, message)` — runtime constraint checked on every write
- `computed(expr)` — calculated on every access, **not stored** in the record

A field has **two independent const axes** (@PLN40 const model) — `const` before the
NAME freezes the *binding* (the slot), `const` before the TYPE freezes the *value*
(the contents). They are opposites and compose into four quadrants:

| declaration | rebind `t.v = …` | append `t.v += …` | element `t.v[i] = …` | use |
|---|---|---|---|---|
| `v: T` | ✓ | ✓ | ✓ | plain mutable |
| `const v: T` (binding-const) | ✗ | ✓ | ✓ | **builder** — slot is write-once, contents grow in place |
| `v: const T` (value-const) | ✓ | ✗ | ✗ | **frozen value** — read-only contents, slot re-pointable |
| `const v: const T` | ✗ | ✗ | ✗ | fully immutable record |

- **`const` before the NAME — binding-const** (write-once at construction). The field is
  set in the struct literal (or takes its default), and any later `t.field = …` rebind is
  a compile error, but the *contents* stay mutable: `const v: vector<T>` allows `t.v[0]=x`
  and `t.v += […]`. This is the **builder** shape (grow a field in place after construction).
  Combining `const` with `computed(...)` is rejected (a computed field is already read-only).
- **`const` before the TYPE — value-const** (`v: const T`). The field's *value* is read-only,
  so every mutation THROUGH it is rejected — append `t.v += …`, element `t.v[i] = …`, and a
  nested write `t.r.x = …` reached via a value-const struct field. A whole-value **rebind**
  `t.v = other` is still allowed: it re-points the slot rather than mutating the frozen value.
  This is the **frozen record / immutable-value** shape (shared config, a compound key, a value
  handed to `par()` threads).
- **Scalar collapse.** A by-value scalar (`integer/float/single/boolean/character`) has no
  contents distinct from its binding, so BOTH axes make it fully immutable — `const n: integer`
  *and* `n: const integer` reject `t.n = …` and `t.n += …`. Example: `const id: integer`.

Value-const enforcement covers DIRECT writes (the write's LHS resolved at compile time). A
value-const value that escapes via a local (`x = t.v; x[i]=…`), a function return, or a
`vector<const T>` generic is not yet frozen through that laundering — that transitivity is
type-carried const, deferred to Phase 3.
(@PLN40; doc/claude/plans/40-const-fields/const-model-phase2.md.)

Defaults are applied in DECLARATION order, and `$` reads the record as it stands at that
moment. A field the construction site supplies is already written, whichever order the two
are declared in — which is what makes the `Object` example below work. A field left to its
OWN default is not: `struct S { a: integer = $.b, b: integer = 2 }` gives `a = 0`, because
`b`'s default has not run yet. Declare the field being read first.

In default/computed expressions, `$` refers to the record:
```
struct Object {
    name_length: integer = len($.name),   // stored default: computed at construction
    name: text
}

struct Circle {
    radius: float,
    area: float computed(3.14159 * $.radius * $.radius)   // recomputes on access
}
```

Example with all modifiers:
```
struct Point {
    r: integer limit(0, 255) not null,
    g: integer limit(0, 255) not null,
    b: integer limit(0, 255) not null
}
```

---

## Scripts (a file with no `fn main`)

A `.loft` file whose top level contains **loose statements** runs as a **script**: the
statements are collected into one synthesized `fn main` and run **once**, in source order,
sharing state. Nothing to opt into — `loft hello.loft` just runs it (@PLN13).

```loft
name = "world"
print("Hello, {name}!\n")

fn twice(x: integer) -> integer { x * 2 }   // defs stay top-level, and are HOISTED
print("{twice(21)}\n")                      // → 42, even though `twice` is declared above-or-below
```

- **`;` is optional between the script's TOP-LEVEL statements** — and only there. Inside a
  `{…}` block the normal rule applies (separate statements with `;`; the block-final one may
  omit it), so two `;`-less statements inside a `for` body are a parse error.
- **Top-level defs** (`fn` / `struct` / `enum` / `type`) stay top-level and are callable from
  the loose statements regardless of order.
- **A file is not a script** when it has a `fn main`, or contains only definitions (a library) —
  those are compiled exactly as before. A file with a mistyped def keyword (`funcion main()`)
  is also not treated as a script, so the parser's real "unknown keyword" error still surfaces
  instead of being buried inside a synthesized `main`.
- **`--script` forces script mode**; auto-detection is the default and can only affect source
  that loft rejects today.
- **Function signatures stay typed.** Parameter types are never inferred: the script → function
  step is where a programmer should think about types, and it is what buys the compile-time
  safety and the native/WASM speed.

The same desugar runs in the browser, so a script is exactly what the
[playground](../playground.html) executes — no install, no boilerplate.

## Declarations

### Functions

```
fn function_name(param: type, other: type = default_value) -> return_type {
    // body
}
```

- `pub` prefix makes a definition publicly visible (applies to functions, structs, and enums).
- `param: type = expr` gives a parameter a **default**, used when the call omits it.
  The expression may build a value of its own — `= []`, `= [1, 2]`, `= "a" + "b"`,
  `= mk()`, `= S { … }` — and may reference EARLIER parameters (`b: text = "x" + a`).
  It is evaluated per call, so each call gets its own value; adding a default is
  additive, so existing callers keep working.
- Parameters with a `&` prefix are a **live reference** to the caller's argument (in-out for any
  type): reads see the caller's current value, writes write the caller, field/element mutation
  mutates the caller. **Call WITHOUT `&` at the call site** — the reference comes from the
  parameter's TYPE, not a call-site operator: `fn inc(n: &integer){ n = n + 1 }` is called `inc(x)`,
  not `inc(&x)`. (See § References (`&`) for the full model; `&` is a binding marker, not a general
  operator.)
  - **Enforced**: a `&` parameter that is never mutated (directly or transitively through a called function) is a **compile error**. Drop the `&` if the parameter is read-only.
- A `const` before a parameter's type — `p: const T` — is **value-const**: a read-only
  borrow of the value (the immutable sibling of the `&T` mutable borrow). It is a
  compile-time check.
  - Every mutation THROUGH the parameter is an **error** — `p += …` (append), `p[i] = …`
    (element), `p.f = …` (field), and nested writes `p.a.b = …`. Reads are always allowed.
  - Re-pointing the local slot — `p = other` — **is** allowed for a compound type (it
    rebinds the function's own copy of the borrow, not the caller's value). The same is
    true of a value-const **local**: `x: const vector<T> = …`.
  - A by-value **scalar** (`integer/float/single/boolean/character`) and a `& const T`
    reference collapse to fully immutable — no `=` and no `+=` — because a scalar has no
    contents distinct from its binding and a `&` write goes straight to the referent.
  - The mirror axis, **binding-const** (`const` before the NAME — freezes the slot but
    leaves contents mutable), exists for **locals** (`const x = …`) and struct **fields**
    (`const v: T`). A binding-const *parameter* (`const p: T`) is not yet wired (Phase 1
    ships value-const params + binding-const locals/fields).
  - The check lives purely on the per-binding flags (`Variable.const_binding` /
    `Variable.value_const`); `Attribute.constant/mutable` on the function definition are
    NOT set for `const` user-defined-function parameters (that would break codegen).
    (@PLN40 const-model phase 1; doc/claude/plans/40-const-fields/const-model.md.)
- Default parameter values are supported.
- Functions without a `->` clause return `void`.
- A function body ending in an expression (without `;`) returns that value.

External (Rust-implemented) functions are declared without a body, followed by `#rust "..."`:
```
pub fn starts_with(self: text, value: text) -> boolean;
#rust "@self.starts_with(@value)"
```

### References (`&`)

A `&`-typed binding is a **live reference** to its source, not a copy. Every operation goes through
it to the source: a read sees the source's current value, a write writes the source, and a
field/element mutation mutates the source.

```
a = 3
b = &a        // b references a
b = 4         // writes a  →  a == 4
a = 9         // b sees a's value  →  b == 9
```

`&` is **not a general operator** — it appears only in a reference-*binding* position, and its
operand must be **addressable** (a variable, struct field, or vector element — never a temporary):

| Form | Allowed? | Meaning |
|---|---|---|
| `a = &b` | ✅ | `&b` as the whole assignment RHS — bind a reference to `b`. |
| `a: &T = b` | ✅ | the declared type says reference; `b` is the referent. |
| `a = &(1 + 2)`, `a = &f()` | ❌ error | operand is a temporary, not addressable. |
| `f(&x)`, `&x + 1`, `[&a]` | ❌ error | `&` used as a general operator / sub-expression. |
| `f(x)` where the param is `&T` | ✅ | the reference comes from the parameter's TYPE — call without `&`. |
| `&a = 4` (`&` on the assignment TARGET) | ❌ error | `&` is a bind-site marker, not an lvalue. |

So a `&` parameter is called by passing the variable directly (`f(x)`), and a `&` local is bound
with `a = &b` or `a: &T = b`. A `&` reference cannot outlive its source.

> **Status (2026-06, @PLN87):** every `&`-reference is live read- and write-through on both backends —
> a scalar local (`a = &b`, `a: &T = b`), a **struct field** (`b = &s.x`), a **vector element**
> (`c = &v[0]`), a **heap whole-value** (`p = &o`, aliases the struct — `p = o` without `&` still
> copies), and a `&` **function parameter** (`fn f(b: &integer)`, called `f(a)`). The addressable-operand
> check and the general-operator ban (`&` only as a binding RHS; `&`-params called without `&`) are in
> place. The edges are characterized too: a reference-to-reference works; a reference can't escape its
> source (no `&T` return type, no `&` in a collection literal, no `&T` struct field). The ladder is
> complete — see [plans/87-reference-default-binding.md](plans/87-reference-default-binding.md). The one
> deferral is full borrow checking (the formal spec's `ownership.md`).

### Constants

```
PI = 3.14159265358979;
```

Constants must be `UPPER_CASE` and are defined at file scope.

**A constant is an inlined expression, not a once-computed value.** The right-hand
side is substituted at every place the name is used, so it runs again for each
reference:

```
fn make() -> integer { println("EVAL"); 7 }
X = make();
fn main() { println("{X} {X} {X}"); }   // prints EVAL three times
```

For a literal or plain arithmetic this costs nothing and is invisible. For an
initialiser that opens a file, parses data, or connects to something, it is a
trap: the work happens once per use. A consumer wrote `FNT = load_bundled();`,
referenced it once per word while laying out text, and the browser ran out of
memory because the font was parsed hundreds of times per frame.

When the value must be computed once, use a function that caches it:

```
fn font() -> FontTable { … }   // parse on first call, keep the handle, return it
```

loft warns when a constant's initialiser calls a user function or a stdlib
function marked `#impure`. Set `LOFT_NO_CONST_EFFECT` to silence it.

A **vector** constant is different: it is pre-built once into the constant store and
referenced, not re-run. Its elements are built from their fields, so they must be
**flat** — scalars and text, and structs of those (`ITEMS = [It { a: 3, b: 4 }]`).
A field value may be computed from things already known at compile time, and is folded
before the element is built: `[BASE + 1, BASE * 2]`, `[-5]`, `["a" + "b"]` all work.

Three limits on what an initialiser may be, each rejected with the working idiom named:
a **struct-valued** constant (`P = Point { … }`), because a record cannot be
materialised at each use site; a vector constant whose element holds a **nested
record** — an inner collection, or a struct/enum field (`NEST = [[7], [8]]`,
`WS = [W { v: [2, 3] }]`) — because that data lives in a store of its own that no
field write describes; and a vector constant with an element value that is only known
at **run time** (`[seven(), 42]`), because there is nothing to pre-build. Wrap any of
them in a zero-argument function.

Text initialisers such as `A = "x" + "y";` and `A = "p{1 + 1}q";` are fine. An
all-literal one is folded to a single literal, so it is not rebuilt per use.

### Types and type aliases

```
type MyInt = integer;
type Coord = integer limit(-32768, 32767);
type Handler = fn(Request) -> Response;
type Pair = (integer, text);
```

Type aliases are purely compile-time substitutions — `Handler` and
`fn(Request) -> Response` are the same type.  Aliases for `fn(...)` types
and tuple types are supported (C55).

In library/default files, `size(n)` specifies the storage size in bytes:
```
pub type u8 = integer limit(0, 255) size(1);
```

### Library imports

```
use arguments;                     // wildcard: every pub name bare + `arguments::` qualifier
use arguments::*;                  // explicit wildcard (same as above)
use arguments::parse_args;         // selective: one name, bare
use arguments::(parse_args, Flag); // selective group — MULTIPLE names need parentheses
use arguments::Flag as Opt;        // alias an imported name (bare `Opt`)
use arguments::(Flag as Opt, parse_args);  // per-name aliases inside a group
use arguments as args;             // library alias → `args::parse_args` (qualifier only;
                                   //   does NOT bring names in bare)
```

Searches for `arguments.loft` in `lib/`, the current directory, directories from the
`LOFT_LIB` environment variable, and relative to the current script.
`use` declarations must appear at the top of the file, before any other declarations.
A qualified `lib::fn()` auto-loads the library, so an explicit `use` is optional for
qualified access.

**Multiple names from one library must be parenthesised.** `use lib::a, b;` (a flat
comma list) is a compile error; write `use lib::(a, b);`.

#### The import form decides whether your own name can clash (loft#1094)

**A bare `use lib;` brings every public name into THIS SOURCE's namespace, so declaring
one of them yourself is a redefinition and is refused.  A selective `use lib::(a, b);`
brings in only what it names, so every OTHER name in that library stays free for you to
declare** — and the two live side by side, reached unambiguously in each source that did
not import the other (the module scoping of @PLN102 C97).

```
use hex_body;                        struct Frame { … }   // refused: `Frame` came in bare
use hex_body::(Rig, rig_world_seg);  struct Frame { … }   // fine: only those two came in
```

Both are deliberate, and the second is the stronger position: **a selective import makes
you immune to the library GROWING a name.** A dependency adding a `Frame` in a later
release cannot collide with yours, because you never asked for it. With a bare import it
can, and that refusal is the point of the check.

It is worth knowing which form you wrote before relying on either. A consumer read the
refusal as a property of their package — "a floor bump would break my build loudly" —
when it is a property of the IMPORT, and their selective imports meant the two `Frame`s
had been live in one graph for some time with nothing to say so.

Where two same-named types do meet — one module imports the library bare while another
declares its own — the mismatch names both DECLARATION sites rather than printing the one
name twice.

#### A package's own module wins its own `use` (loft#976)

**Inside a package, `use <module>;` binds that package's own `src/<module>.loft`** when it
ships one. The module registers under `<package>::<module>`, so two packages' `catalogue`
modules are two live modules rather than one contested name.

It used not to be. A module's short name was one slot shared by the whole dependency
graph, and building a package reads every file under its `src/` — so whichever package
loaded first took the name, and the rest lost their own module:

```
dep/src/catalogue.loft   pub fn part_list() -> integer { 41 }
dep/src/dep.loft         use catalogue;
                         pub fn dep_answer() -> integer { part_list() + 1 }
con/src/catalogue.loft   pub fn part_list() -> integer { 99 }

dep_answer()  ->  100        // was: the consumer's number, from inside the dependency
dep_answer()  ->  42         // now: its own, in every consumer
```

Nothing in `dep` changed and nothing in `con` imported `catalogue`; a published, versioned,
tested package answered differently because of a file downstream, and which one lost was
decided by the CONSUMER's `use` order — visible to neither author.

Four things to know:

- **A declared dependency still wins.** If `loft.toml` names a dependency `catalogue`,
  `use catalogue;` is that dependency, not a local file of the same name. A package cannot
  accidentally shadow what it depends on.
- **`use self::<module>` is the same binding, written explicitly.** It stays, and it is
  what you write when a file is not inside a package but you want the guarantee spelled
  out. It also refuses to search outward, which a bare `use` still does when the package
  ships no module of that name.
- **Its qualifier is the alias.** `<package>::<module>` is not a typeable path, so write
  `use self::catalogue as cat;` (or `use catalogue as cat;`) to get `cat::part_list()`.
  ⚠ The short name gives NO qualifier: after `use self::catalogue;`, `catalogue::f()` is
  refused — the flat `catalogue::` slot is shared by the whole dependency graph, and
  staying out of it is what `self::` is for, so it is withheld rather than missing. Since
  loft#1043 the compiler says exactly that at the call site instead of *"Unknown
  library"*, which read as *the module is gone* and took a tree-wide rewrite to diagnose.
  ⚠⚠ **And an alias DOES take that shared slot**, so `as cat` re-enters the namespace
  `self::` kept you out of — pick a name no other package would plausibly use
  (`hexmesh_surfaces`), never the module's own name.
- **Two modules are not merged.** If both packages' modules declare the same public name
  and you call it bare with both in scope, that is an error naming both — pick one with an
  alias or a selective import. Silently taking one was the old behaviour and the reason
  for the change.
- **One FILE is one module, however many names reach it.** The `<package>::<module>` key
  is what keeps another package from taking the name, and it is also a second name for a
  file that may already be loaded under its bare one — a program OUTSIDE the package
  writes `use catalogue;` and gets the file flat, then a file INSIDE the package writes
  the same line and computes the qualified key, which is absent. Until loft#1080 that
  parsed the same file a SECOND time, and every consequence followed from the two copies:
  bare calls became ambiguous against a module the author never wrote (`src2::part_list`,
  the orphaned second source), and a native build emitted every duplicated function twice
  under one identifier, so the cdylib would not compile (55 × `error[E0428]`, each pair
  with an identical hash — a same-file collision, unlike loft#305's two different files).
  The loader now asks whether the FILE is loaded, by canonical path, and binds the second
  name to the source that exists. Two different files that merely share a module name are
  untouched: they are still two modules, and still an error when a bare call cannot pick.

### Shadowing and qualified names (`@PLN22`)

Definitions are scoped, not flat. A name resolves first in the current file, then
falls back to the imported-library and standard-library *prelude*:

- **Your definitions may shadow a prelude name.** `enum E { … }`, `struct File { … }`,
  `pub PI = 3` are all legal even though the stdlib defines `E` / `File` / `PI`. Your
  definition wins bare lookup; the original is still reachable as `std::E` (and a
  library's via `lib::Name`).
- **Built-in type keywords are reserved** and cannot be shadowed: `integer`, `float`,
  `single`, `text`, `boolean`, `character`, `vector`, `hash`, `sorted`, `index`,
  `radix`, `spatial`, `iterator`, `reference`, and the sized integers
  `i8`/`i16`/`i32`/`u8`/`u16`/`u32`. `struct integer { … }` errors with *"conflicts
  with a type"* (for `struct`, `enum`, and `type` alike).
- **A local may carry a function's name** — values and functions are separate
  namespaces, and the parentheses pick between them. `chr = 65` binds a local while
  `chr(65)` in the same scope still reaches the stdlib function; a bare `chr` reads
  the local once one is bound. This holds for every binding form — assignment, the
  typed local `chr: integer = 65`, a tuple-destructuring element, a parameter, a
  `for` variable, a struct field.

  It is what keeps a library's growth off its consumers (loft#852): every short verb
  a library exports — `turn`, `step`, `run`, `wait`, `next`, `open`, `send` — would
  otherwise become a word no consumer of that library may use as a local, taken away
  on someone else's release with nothing to announce it. Shadowing a name you rely on
  is still worth avoiding; it is just yours to decide, not a library's.

- **Two imported libraries may not both answer a bare name** (loft#788). When
  `use a;` and `use b;` each export `Chunk`, writing bare `Chunk` is an error naming
  both — *"`Chunk` is declared by more than one package here — write `a::Chunk` or
  `b::Chunk` to say which"* — because the alternative is a source line whose meaning
  depends on the order of the `use` block above it. It applies to every bare
  mention: a type, a function call, a constant.

  Reported where the bare name is USED, never at the `use` line: two libraries may
  share a name your program never writes bare, and that program keeps compiling.
  Qualifying (`a::Chunk`, `a::helper()`) always works, and a definition of your own
  still shadows both.

### Enum-scoped variants (`@PLN22`)

Variants belong to their enum, so **two enums may share a variant name**:

```loft
enum Color { Red, Green }
enum Light { Red, Amber }
```

A bare variant used as a **value** resolves from its type context — a `match`
subject, a typed declaration (`c: Color = Red`), a comparison (`c == Red`), a
function argument, a return position, or a struct-field type/default. With no
context, qualify it: `Color.Red` (or `Color::Red`). Defining a *new untyped
variable* directly from a bare variant is a deliberate error:

```loft
x = Red;            // error: bare variant 'Red' has no type here — qualify it as 'Color.Red'
x = Color.Red;      // ok
c: Color = Red;     // ok — the declared type supplies the context
```

This keeps a later `enum Light { Red, … }` from silently re-pointing an existing
bare assignment. The variant name remains usable as a **type / constructor**
(`Circle { … }`, `s: Circle`, `fn f(self: Circle)`), so struct-variant
construction is unaffected.

---

## Operators

Listed by precedence, loosest first — each level binds **tighter** than the one above
it, so a lower level groups last (outermost) and a higher level groups first (innermost):

| Precedence | Operators                              | Notes                   |
|------------|----------------------------------------|-------------------------|
| 0 (loosest)| `??`, `?? return`                      | null-coalescing / early return (C56) |
| 1          | `\|\|`, `or`                           | logical OR              |
| 2          | `&&`, `and`                            | logical AND             |
| 3          | `==`, `!=`, `<`, `<=`, `>`, `>=`       | comparison              |
| 4          | `\|`                                   | bitwise OR              |
| 5          | `^`                                    | bitwise EOR             |
| 6          | `&`                                    | bitwise AND — *infix only*; a **leading** `&` is the reference annotation, not an operator (see § References (`&`)) |
| 7          | `<<`, `>>`                             | bit shift               |
| 8          | `-`, `+`                               | addition/subtraction    |
| 9          | `*`, `/`, `%`                          | multiplication/division/modulo |
| 10         | `**`                                    | power — **right-associative** |
| 11 (tightest)| `as`                                 | type cast/conversion (right operand is a *type*) |

**Associativity.** All binary operators are **left-associative** (equal-precedence
operators group left-to-right: `a - b - c` is `(a - b) - c`) except `**` (power),
which is **right-associative**: `2 ** 3 ** 2` is `2 ** (3 ** 2) = 512`. Worked groupings:
`1 + 2 & 3` is `(1 + 2) & 3 == 3` (additive tighter than bitwise-and); `2 * 3 ** 2` is
`2 * (3 ** 2) == 18` (power tighter than multiply); `x as integer as float` is
`(x as integer) as float` (`as` left-associative).

**Comparisons do not chain (non-associative).** The comparison operators
(`==`, `!=`, `<`, `<=`, `>`, `>=`) are **non-associative**: writing two at the same level —
`a == b == c`, `1 < x < 10` — is a **compile error**, because left-associative grouping would
make it `(a == b) == c`, silently comparing a *boolean* to the third operand (the classic C
footgun). Parenthesise if you truly mean the boolean compare (`(a == b) == c`), or combine with
`&&` for a range test (`1 < x && x < 10`).

**A boolean and an integer are not comparable.** `true == 1`, `flag != 0` and the like are a
**compile error** — a `boolean` is `true`/`false`/`null`, not `0`/`1`, so the two are different
types (consistent with `bool < int`, which was always rejected). Convert explicitly if you
really mean it. (`b == null` on a `boolean?` is fine — `null` is not an integer.)

**Tuples compare lexicographically.** All six operators work between two tuples of the same
arity: the first element decides, and later elements are consulted only while the earlier
ones are equal — so `(1, 9) < (2, 0)` and `(1, 9) < (1, 10)`, while `(1, 9) < (1, 9)` is
false and `(1, 9) <= (1, 9)` is true. Elements are compared with their OWN operators, so
text compares by value (`(1, "abc") == (1, "abc")`) and a nested tuple recurses. An element
type with no such operator says so about the element — `(false, 1) < (true, 0)` reports
*"No matching operator `<` on `boolean` and `boolean`"*, because a tuple never invents an
ordering its elements do not have. Different arities are not comparable.
See [TUPLES.md § Comparison](TUPLES.md).

Unary operators: `!` (logical not), `-` (negation / sign), `~` (bitwise NOT). A unary prefix
binds **tighter than every binary operator** — the `-` is the **sign of its operand**, part of
the primary expression. So `-2 ** 2` is `(-2) ** 2 == 4` (read `-2` as the number
*negative two*), *not* `-(2 ** 2) == -4` as in Python/maths (which treat `-` as a weaker
operator). For a **literal** base this matches the "`-2` is a number" intuition and is silent;
for a **non-literal** base (`-x ** y`, `-f() ** y`) loft emits a **warning** nudging you to
parenthesise, since there the `-` reads as an operator on a subexpression. The grammar rule is
uniform either way — only the reminder is added. Wrap explicitly if you mean the negation to
apply last: `-(x ** 2)`.

`~x` computes the bitwise complement (all bits flipped): `~0 == -1`, `flags & ~32` clears bit 5.
Only defined for `integer`; use `as integer` to convert other types first.

Assignment operators: `=`, `+=`, `-=`, `*=`, `/=`, `%=`.

**Integer `/` and `%` with negatives.** Integer division **truncates toward zero**
(`-7 / 2 == -3`, not `-4`), and `%` returns the remainder that takes the sign of the
**dividend** (`-7 % 2 == -1`, `7 % -2 == 1`) — the C/Rust convention, so `a == (a / b) * b + a % b`
holds. When you want a remainder that wraps into `[0, n)` (the sign of the *divisor*, for
circular indexing), use `floor_mod`: `(-1).floor_mod(3) == 2`. Division or `%` by zero is a
**null**, never a fault (C80) — discharge it with `?? default`.

### The `??` operator (null-coalescing)

`lhs ?? rhs` evaluates to `lhs` if it is not null, otherwise evaluates to `rhs`:

```loft
name = record.optional_field ?? "unknown"
count = map_lookup ?? 0
first = a ?? b ?? c    // chains: first non-null of a, b, c
```

The operator is left-associative and chains: `a ?? b ?? c` is `(a ?? b) ?? c`.
If `lhs` has a statically-known `null` type (the bare `null` literal), `??` returns `rhs` directly.

**Result type — the discharge is only as complete as the fallback.** `a ?? b` yields `a` when
non-null else `b`, so it can still be null exactly when `b` can: the result type is the non-null
base **only if the fallback `b` is non-null**. A nullable fallback — a bare `null` literal, or a
`τ?`-typed expression — keeps the result `τ?`, so `y: integer = x ?? null` is a compile error (a
null would reach the non-null slot). A chain discharges to non-null iff its *last* fallback is
non-null: `x ?? (a / b) ?? 7` is `integer` (ends in `7`), while `x ?? null` is `integer?`. Discharge
into a non-null slot with a real default (`x ?? 0`), or keep the slot `τ?`.

**Note:** For complex LHS expressions (function calls, field chains), the compiler automatically
materialises the result into a temporary variable so the expression is evaluated exactly once.
Simple variable reads skip the temporary since they have no side effects.

### The `x?` operator (default-fallback) — @PLN116

Postfix `x?` discharges a nullable `x: T?` to `T` by falling back to `T`'s **default** when
`x` is null — pure sugar for `x ?? construct_default(T)`. Where `??` supplies *the default you
give*, `?` supplies *the default the type gives*; that pairing is the mnemonic (`??` = my
default, `?` = the type's). It relieves the `?? 0` / `?? ""` / `?? 0.0` boilerplate that loft's
own null-flow manufactures (DN3 makes float ops yield `float?`; index/map/field reads are
nullable by the C80 model).

```loft
x = (a / b)?                 // integer? → 0 on divide-by-zero
name = row.label?            // text?    → "" when the field is null
first = points[i]?          // Point?   → Point{} (every field defaulted) on out-of-bounds
colour = pixel.tint?         // Colour?  → the first-defined enum variant
```

Precedence: `?` binds **tightest** (like `.`/`[]`), tighter than `as` and every binary operator,
so `a.b?` is `(a.b)?`, `x? as T` is `(x?) as T`, and — because `??` lexes greedily over `?` —
`a ?? b?` is `a ?? (b?)`. Chaining works: `points[i]?.x` discharges then reads the field.

The **default** is the one `has_default(T)` / `construct_default(T)` predicate that also backs the
`S{}` zero value (one home per fact, [Goal E](GOALS.md)): scalar → `0`/`0.0`/`false`/`'\0'`,
`text` → `""`, collection → empty, a **bare** enum → its first-defined variant, record → `S{}` with
every field defaulted, nullable `U?` → `null`.

Two things have **no default**, so `x?` on them (and the matching `S{}`) is a **compile** error (a
static well-definedness check, fully consistent with "no *runtime* errors ever" — C80):

- a **bare reference / non-null `DbRef`**; and
- a **record with a bare (non-`Optional`) enum field that has no `= expr`**. An enum's 0 *is* its
  null value (variants are 1-based), so a non-null enum field may not silently zero-fill to it —
  and choosing a variant as a record's default is a real decision the author must make. Fix it by
  providing the field, giving it `= <variant>`, or typing it `E?` (which then defaults to `null`).
  So a *bare* enum discharges to its first variant, but an enum *field inside a record* needs an
  explicit choice before the record itself can default.

`x?` on an already-non-null operand is an identity plus a redundant-`?` warning (mirrors the
redundant-`??` lint).

**On the left of an assignment, the `?` is on the READ.** `place? op= e` writes `place`; the
`?` says which value to read when `place` is null. So `x? += 3` on a null `x` is `3` — the
accumulate-from-the-zero idiom — while the bare `x += 3` leaves it null, because a compound
assignment on a scalar or `text` PROPAGATES. For a **collection** the two spellings agree:
appending to a null collection builds the empty one first, so `b.d? += [r]` and `b.d += [r]`
mean the same thing. A plain `place? = e` has no read to discharge and means `place = e`.

```loft
hits: integer? = null;
hits? += 1                  // 1     — the `?` read the zero, the write landed in `hits`
misses: integer? = null;
misses += 1                 // null  — no `?`, so the null propagates through `+`
```

`(a ?? d) += e` is refused rather than guessed: it names two values and no place, so it takes
no assignment at all. A place whose **address calls a function** is fine — `w[idx()]? += 1`
calls `idx()` once, as [C92](DESIGN_DECISIONS.md) requires of every compound assignment.
Rule: `@FR-E-Asgn-Discharge`, [formal/operational.md](formal/operational.md).

### The `as` operator

Used for explicit type casts and conversions:
```
3.14 as integer          // numeric cast: float to integer (truncates toward zero) → integer
"json-text" as Program   // deserialize text as a struct
```

**`as` has two different jobs — a numeric cast vs. a fallible parse.** When the left
operand is a *number*, `as` reinterprets it (truncate/narrow/widen). When the left
operand is **text**, `as integer` / `as float` / `as single` is a **parse that can
fail**, so it types `integer?` / `float?` / `single?` — you get `null` on a bad parse
and must discharge it:
```
n = "42" as integer          // n : integer?   (NOT integer — the parse can fail)
n = "42" as integer ?? 0     // n : integer    (discharge with a default)
n = s as integer?            // n : integer?   (keep it nullable, discharge later)
```
(This is the @PLN25 `(N-Parse)` rule — a bad parse is a reachable fault like `÷0`/OOB.)

### Type-conversion rules — when does loft convert automatically?

Loft applies conversions in three modes: **implicit** (no annotation),
**format-only** (implicit, but only inside `"{…}"` interpolation), and
**explicit** (`as` required).  The mode depends on the types involved,
not on the context — which means you can predict what a conversion will
do by looking up the pair in this table:

| From → To                          | Mode          | Notes |
|------------------------------------|---------------|-------|
| Any type → `boolean` (in `if`, `!v`, `while`, `assert`) | Implicit | `false` and null are falsy; integer `i32::MIN` is falsy; every other value is truthy.  See § Pattern matching for the null-sentinel table.  **These four POSITIONS are the whole of it** — a `vector` passed where a `boolean` PARAMETER is declared stays an error, because there the coercion would hide a mistake rather than express one.  An EMPTY collection and a payload-less enum variant are values, so both are truthy; only null is falsy. |
| Integer ↔ `float` in arithmetic    | Implicit      | `3 + 1.5` is `4.5` — the integer widens to the float operand's width |
| Integer / `single` → `float`       | Implicit      | widening; `single` (32-bit) widens to `float` (64-bit) with no loss |
| Integer → `single`                 | Implicit      | `[1, 2]` is a valid `vector<single>` |
| `float` → `single`                 | Explicit `as` | NARROWING (64→32-bit loses precision).  A bare decimal literal is `float`; write a **`single` literal** with the `f` suffix (`1.0f`) or cast (`x as single`).  This is enforced element-wise: a `vector<single>` literal must be `[1.0f, 2.0f]` or `[a as single, …]` — `[1.0, 2.0]` (float literals) is a compile error ("would lose precision"), never a silent truncation |
| `i32` / narrow int → `integer`     | Implicit      | widening; a 4-byte `i32` (or `u8`/`u16`/`i8`/`i16`) widens into the 8-byte `integer` with no loss |
| `integer` → `u8`/`u16`/`i8`/`i16`/`u32`/`i32` | Explicit `as` at storage sites | NARROWING — a plain `integer` is 64-bit; writing one into a narrow **struct field**, local, parameter or return (any narrow storage) requires `as` ("cannot implicitly narrow integer to u16 … cast explicitly").  A **constant that provably fits** the target is exempt (`x: u16 = 5`, `f(200)`).  The check is range containment **or a drop in storage width**, so it covers every narrow alias — including `i32`, which the range half cannot see (see the row below).  (@PLAN48 / @P370 / loft#931) |
| `integer` → `i32` — why it needs the width half | Explicit `as` | `i32` spans the whole 32-bit range, which is the range a plain `integer` *reports*: the 64-bit value lives in an 8-byte slot the bounds do not describe.  So the two specs differ in `forced_size` alone and `[s.min,s.max] ⊆ [d.min,d.max]` holds for a pair whose storage drops 8 → 4 — the one alias whose NAME says "32 bits" was the one range containment never checked, and an `i32` field took `5000000000` and stored `705032704` in silence, on both backends (loft#931, fixed).  The **implicit** stores compare storage width; an **explicit** `as i32` keeps the range rule alone, so it stays spellable as the cure this diagnostic prescribes |
| `float` → integer                  | Explicit `as` | `pi as integer` truncates toward zero; preserves the current sentinel semantics |
| `text` → integer / float / single  | Explicit `as` — **a PARSE, types `τ?`** | `"42" as integer` is a fallible parse, so it types **`integer?`** (`float?` / `single?`), not `integer`. A non-numeric text yields `null`. You MUST discharge before storing into non-null: `"42" as integer ?? 0`, `s as integer?` (keep it nullable), or `match`. This is `(N-Parse)` — a bad parse is a reachable fault, exactly like `÷0` and out-of-bounds indexing (§ @PLN25). Contrast the *numeric* casts above (`float`→`integer`, width narrowing), which reinterpret an existing number rather than parse text |
| Integer / float / boolean → `text` | **Format-only** | `"n={m}"` renders the value inline; `t = m` with `t: text` is a compile error.  If you want the rendered form as a standalone text value, assign through interpolation: `t = "{m}"` |
| `character` → `integer` (codepoint)| Explicit `as` | `'a' as integer` yields 97 |
| `character` ↔ `text`               | See § String literals | Indexing vs. slicing asymmetry; concatenation via interpolation |
| `text` (of form `"VariantName"`) → plain enum | Explicit `as` | `"West" as Direction` — the name must match a declared variant |
| Struct-enum variant → parent enum  | Implicit on assignment | `p: Shape = Circle { r: 1.0 }` works without `as` |
| Struct-enum variant ← parent enum  | `match` only  | Recover the concrete variant via pattern matching; there is no direct downcast |
| `text` → struct / vector<T>        | Explicit `as` or `.parse` | `raw as Program` or `Program.parse(raw)` |

**Rule of thumb:** conversions that cannot fail (widening numeric,
struct-enum up-cast, rendering for display) are implicit.  Conversions
that can fail (narrowing, parsing) require `as` or `.parse` so the
failure point is visible at the call site.  The one special case is
"integer/float → text" — implicit only inside format strings, explicit
elsewhere — because loft treats format interpolation as a dedicated
rendering operation, not a general coercion.

**Narrowing an integer *value* (expression casts).** Writing `x as u8`
when the compiler cannot prove `x` fits `0..=255` is a **compile error**,
not a silent truncation.  Pick the form by what should happen to an
out-of-range value:

- **Fallback** — `x as u8 ?? d`.  A checked narrowing with a default: the
  value when it fits `u8`, otherwise `d` (which must itself fit `u8`).
  The result is a `u8`, so it drops straight into a `u8` field / return /
  local with no further cast.  `x as u8? ?? d` means the same thing; the
  `?`-free form is the natural one.
- **Mask** — `x & 0xFF`.  Keeps the low bits (wraps an out-of-range
  value).  The mask proves the range, so the result stores into a `u8`
  slot with no `as` at all — the idiom for byte packing, RGBA, and
  hashing.
- **Constrain the source type** — declare it `integer limit(0, 255)` (or
  `u8`).  A value that provably fits narrows implicitly, no cast.

Loft does **not** refine a value's range from an `if` guard — inside
`if x <= 255 { … }`, `x` is still a full `integer`.  Use one of the forms
above.

### Parsing (JSON → JsonValue tree → struct)

JSON support has two layers:

**1. `JsonValue` enum (preferred for new code).** `json_parse(text) -> JsonValue` returns
a typed tree covering all six RFC 8259 kinds (`JNull`, `JBool`, `JNumber`, `JString`,
`JArray`, `JObject`).  Malformed input returns `JNull`; the error trail is in
`json_errors()`.  Chained access (`v.field("k").item(0).as_text()`) is safe — every
intermediate failure produces `JNull`, never a trap.  Full surface reference in
[STDLIB.md § JSON](STDLIB.md).

```
v = json_parse(`{{"users":[{{"name":"Alice"}}]}}`);
name = v.field("users").item(0).field("name").as_text();   // "Alice"
reply = json_object([
  JsonField { name: "ok",    value: json_bool(true) },
  JsonField { name: "count", value: json_number(3.0) }
]);
text = reply.to_json();   // {{"ok":true,"count":3}}
```

**2. `Type.parse(text)` (legacy, transitional).**  Parses JSON or loft-native text
directly into a struct record.  `Type.parse(JsonValue)` is the preferred replacement
(shipped).

Works for plain structs AND struct-enums (P159).  Struct-enum JSON uses a
discriminant wrapper: `{"Circle":{"radius":3.14}}`.

A failed parse leaves the record at its type's zeros, so ASK whether it failed — either
surface answers, and both are cleared by the next parse:

```
user = User.parse(`{{"id":42,"name":"Alice"}}`);
scores = vector<Score>.parse(`[{{"value":10}},{{"value":20}}]`);
shape = Shape.parse(`{{"Circle":{{"radius":3.14}}}}`);   // struct-enum round-trip

if json_errors() != "" { log_warn(json_errors()); }   // the JSON surface
errs = user#errors;                                   // the record surface: TEXT, and
if errs != "" { log_warn(errs); }                     // reading it clears it
```

`record#errors` is a single text (newline-separated when a parse produced several), not a
collection — `for e in user#errors` iterates CHARACTERS, and because the read clears, it
iterates none at all.  Read it into a variable and test it, as above.

---

## Literals

| Kind             | Syntax examples                     |
|------------------|-------------------------------------|
| Integer          | `42`, `0xff`, `0b1010`, `0o17`      |
| Float            | `3.14`, `1.0`                       |
| Single           | `1.0f`, `0.5f`                      |
| Character        | `'a'`, `'😊'`                       |
| Boolean          | `true`, `false`                     |
| Null             | `null`                              |
| String           | `"hello world"`                     |
| Function ref     | `fn double_score`                   |
| Lambda (long)    | `fn(x: integer) -> integer { x * 2 }` |
| Lambda (short)   | `\|x\| { x * 2 }`                    |

A **function reference** (`fn <name>`) produces a `Type::Function` value whose runtime representation is the definition number of the named function.  The compiler resolves the name at **compile time** and errors if it does not exist or is not a function.  The value is 4 bytes (same as `integer`).

**Calling a fn-ref variable:** a variable or parameter of type `fn(T) -> R` can be called directly:

```loft
f = fn double_score           // type: fn(const Score) -> integer
x = f(some_score)             // calls double_score via f
```

**`fn(T) -> R` as a parameter type:**

```loft
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
result = apply(fn double_it, 5)
```

**Lambda expressions** produce an inline anonymous function at the expression level.
Two syntactic forms are available:

```loft
// Long form — all types explicit; always valid
fn(x: integer) -> integer { x * 2 }
fn(x: integer, y: integer) -> integer { x + y }

// Short form — types inferred from the expected type
|x| { x * 2 }
|x, y| { x + y }
|| { 0 }                        // zero parameters: uses the || token

// No context to infer from?  Use the LONG form — a `|x|` lambda takes no
// annotations of its own (neither `|x: integer|` nor a trailing `-> R`).
transform: fn(integer) -> integer = fn(x: integer) -> integer { x * 2 }
```

Short-form parameter types are inferred from the expected `fn(T1, T2) -> R` type
**wherever there is one** — the position does not matter, only that something names the
signature (loft#1067):

| where | example |
|---|---|
| a call argument | `takes(\|x\| { x * 2 })` |
| a named argument | `takes(f: \|x\| { x * 2 })` |
| a declared local | `a: fn(integer) -> integer = \|x\| { x * 2 }` |
| a struct-literal field | `H { f: \|x\| { x * 2 } }` |
| an element of `vector<fn(…)>` | `[\|x\| { x * 2 }, \|x\| { x + 1 }]` |
| a return position | `fn make() -> fn(integer) -> integer { \|x\| { x * 2 } }` |
| a parameter default | `fn takes(f: fn(integer) -> integer = \|x\| { x * 2 })` |
| a tuple member | `t: (fn(integer) -> integer, integer) = (\|x\| { x * 2 }, 1)` |
| a tuple member by assignment | `t.0 = \|x\| { x + 1 }` (nested too: `t.0.0 = …`) |

If inference is impossible — nothing in the context names a signature — the compiler
errors: *"Cannot infer type for lambda parameter 'x'; pass the lambda where the expected
type is known, or use fn(name: &lt;type&gt;) { ... }"*.

Its primary use is with the higher-order functions `map`, `filter`, and `reduce`, as well as the `par(...)` for-loop clause:

```loft
// Named fn-ref
fn double(x: integer) -> integer { x * 2 }
fn is_pos(x: integer) -> boolean { x > 0 }
fn add(a: integer, b: integer) -> integer { a + b }

doubled  = map(nums, fn double);        // [2, 4, 6, ...]
positive = filter(nums, fn is_pos);     // only positive elements
total    = reduce(nums, 0, fn add);     // sum

// Equivalent using lambdas (short form, types inferred)
doubled  = map(nums, |x| { x * 2 });
positive = filter(nums, |x| { x > 0 });
total    = reduce(nums, 0, |a, b| { a + b });

for a in items par(b=double(a), 4) { results += [b] }
```

**`map(v, fn f) -> vector<U>`** — applies `f` to every element; returns a new vector of the return type of `f`.

**`filter(v, fn pred) -> vector<T>`** — returns a new vector with only elements for which `pred` returns `true`.

**`reduce(v, init, fn f) -> U`** — left-folds: starts from `init`, applies `f(acc, elm)` for each element in order.

### Closures

A lambda that references variables from the enclosing scope is a **closure**.
The captured values are **copied into the closure record at definition time**
(value semantics, like Rust `move` closures).

```loft
greeting = "Hello"
greet = fn(name: text) -> text { "{greeting}, {name}!" }
greeting = "Bye"       // does NOT affect the closure
greet("world")         // "Hello, world!" — captured at definition time
```

**Cross-scope closures** — a function can return a closure to its caller.
The captured values travel with the lambda:

```loft
fn make_adder(n: integer) -> fn(integer) -> integer {
    fn(x: integer) -> integer { n + x }
}
add5 = make_adder(5)
add5(10)               // 15
```

**Capture rules:**
- Integers, floats, booleans, characters: copied by value at definition time —
  unless the closure MUTATES the capture, in which case the scalar is promoted to a
  shared heap cell and writes propagate both ways (plan-22; the
  single-closure accumulator pattern).  A mutated scalar may be captured by
  only ONE closure (C74, #314).  The captured binding may be a local **or a
  parameter**: a mutated parameter is copied into a hidden local first, so the
  closure's writes are visible for the rest of the function and the CALLER's value
  is untouched — a scalar parameter stays by-value (#685).  Mutating a `const`
  parameter through a closure is rejected, like any other write to it.
- Text: deep-copied (independent of original after capture); mutated text
  captures are cell-promoted like scalars, including from a parameter and
  including inside a function that itself returns `text` (#687).
- Struct references: the DbRef is copied — both point to the same store
  record while both are alive, and mutations from either side are visible to
  the other (#318/C75 bound such closures to the frame that owns the
  captures).
- Collections (`hash` / `vector` / `sorted` / `index`): captured by shared
  DbRef — the closure **borrows** the outer collection (like a struct
  reference).  Inside the closure the full surface works and every mutation
  persists to the shared collection (the outer scope keeps ownership): **look up
  by key / index** (`(h[k] ?? Row { … }).v`), **iterate** (`for e in h`),
  **point-assign** (`h[key] = value`), and **append** (`h += Row { … }`;
  vectors take the unambiguous `xs += [elem]` push form).  Two closures that
  capture the same collection both mutate the one shared store (@PLN93 / #511).

**Who frees a shared capture (#682).**  A struct-reference or collection capture
travels as a DbRef, so exactly one owner must reclaim the store.  Which one depends
on where the capture came from, and the language does the bookkeeping for you:

- Capturing a local the function **owns** hands ownership to the closure, so the
  store lives as long as the closure — that is what makes a factory (`fn make() ->
  fn(…)` returning a closure over a local) sound.
- Capturing a **parameter**, or a local that only views into something else
  (`ch = w.chunks[1]`, a `for ch in w.chunks` element), **borrows**: the store still
  belongs to its original owner, which outlives the closure.

Both directions are automatic and need no annotation.  What you can rely on is the
consequence: **passing a value into a function that captures it in a lambda never
damages your copy**, and a returned closure never reads a store that has been
freed.  Until #682 the first half was untrue — a captured parameter was freed when
the closure record died, and the caller's value went dangling, typically surfacing
as a crash much later in an unrelated function that touched the same value.

**Limitations:**
- A **collection cannot hold a capturing closure** — `vector<fn(…)>` and the keyed
  collections take non-capturing lambdas only, whatever their types (@P213/@P214: the
  closure record has no co-located layout yet).  One capturing element is enough to
  refuse the literal, and so is a collection of a STRUCT whose field holds one.
  A struct field on its own is fine: `Holder { f: fn(x: integer) -> integer { x + a } }`
  captures and calls normally — which is what makes the compiler's advice work, namely
  keep the captured state in a struct and store a non-capturing `fn` that reads it.
- A `&` parameter cannot be captured at all, in any shape (loft#1276).

`spatial` is not an exception to any of this: it stores a non-capturing `fn` field and calls
it (`for e in sp { e.f(21) }` answers), and it refuses a capturing one with the same message
every other collection gives.

See [THREADING.md](THREADING.md) § fn Expression for how function references are used with `par(...)`.

---

## String literals

Loft has two string literal syntaxes. Both support `{expr}` interpolation. The strict rules for
interpolation and value→text rendering (per-type forms, format specs, fault-safe `{a/b}` → `null(/0)`)
are [formal/formatting.md](formal/formatting.md).

### Double-quoted strings (`"..."`)

Single-line. Supports `\n`, `\t`, `\\`, `\"` escapes.

```
"hello {name}"           // interpolation
"line1\nline2"           // escape sequences
"literal {{braces}}"     // escape { } by doubling
```

### Backtick strings (`` `...` ``)

**Multi-line.** Bare `"` is literal inside backtick strings (no escaping needed).
Auto-strips leading indentation: the **first content line** sets the base, and that many
leading spaces come off every line that has them.  A blank line does not set the base,
a line indented less than the base comes out flush, and a TAB-indented line is left
alone (a tab is not a space, so there is nothing to count).  The first and last lines
are dropped when they contain only whitespace.

Interpolation is no exception — a block with `{…}` in it dedents exactly like one
without.  Until loft#990 it did not, which mattered most for the shape the feature
exists for: templates.

`{` opens an interpolation hole here as it does in `"…"`, so a literal brace is written
`{{` — which is what the `void main() {{` below is doing.

```
shader = `
  #version 330 core
  layout (location = 0) in vec3 aPos;
  void main() {{
      gl_Position = vec4(aPos, 1.0);
  }}
`;

msg = `Hello, {name}!
  You have {count} messages.`;   // holes -> NOT stripped: the two spaces survive
```

Use backtick strings for GLSL shaders, multi-line templates, or text containing `"`.
Embedded code brings its own braces, and every one of them has to be doubled — a bare `{`
opens an interpolation hole wherever it appears. Doubling keeps the strip working, because
`{{` is not a hole; a real `{expr}` is what switches it off.

**Gotcha — indexing a text yields `character`, slicing yields `text`.**  The two
operations on the same subject return different types:

```
txt = "hello";
c = txt[0];       // character ('h') — a single Unicode scalar value
s = txt[0..1];    // text ("h") — a one-character string
```

Practical consequences:
- `text + text` concatenates (`txt[0..1] + txt[1..2]` is `"he"`).
- `character + character` does **not** concatenate — `+` on characters is
  the arithmetic operator.  Build text from characters via interpolation:
  `"{c1}{c2}"` or `t = ""; t += "{c}"` in a loop.
  Returning interpolated characters from functions works correctly:
  `fn f() -> text { c = txt[0]; "{c}" }` returns `"h"`.
- `character == text` is a compile error today; format the character
  first: `"{c}" == some_text`.
- Vectors are consistent (`vec[0]` element, `vec[0..1]` `vector<T>`) —
  the text/character asymmetry is deliberate because `character` is a
  distinct scalar type, not a length-1 text.

When your code manipulates text character-by-character, prefer `txt[i..j]`
slicing (iterator-aware, stays in the text domain) over `txt[i]`
(produces a character you then have to convert back).

## String formatting

Both `"..."` and `` `...` `` strings support format specifiers using `{...}`:

```
"Value: {x}"             // embed variable
"Hex: {n:#x}"            // hexadecimal with 0x prefix
"Oct: {n:o}"             // octal
"Bin: {n:b}"             // binary
"Padded: {n:+4}"         // width 4, always show sign
"Signed float: {f:+.3}"  // `+` applies to float and single too
"Zero-padded: {n:03}"    // width 3, zero-padded
"Float: {f:4.2}"         // width 4, 2 decimal places
"Left: {s:<5}"           // left-aligned width 5
"Right: {s:>5}"          // right-aligned
"Center: {s:^7}"         // center-aligned
"{x:j}"                  // JSON output
// The flags before the width may be written in any order: `{f:+<8.3}` == `{f:<+8.3}`.
"{x:#}"                  // pretty-printed multi-line output
```

Escape `{` and `}` as `{{` and `}}`.

A spec tunes what the value RENDERS as, so the width, the alignment and the pad character
work on every type — a character, a vector, a struct and a record-carrying enum pad exactly
as text and numbers do:

```
"[{c:>5}]"        // a character   → [    x]
"[{v:>12}]"       // a vector      → [       [1,2]]
"[{p:*^16}]"      // a struct      → [***{r:1,g:2}****]
```

Two rules decide the edges. The flags that choose the RENDERING itself — `#` and `:j` — are
not field-shaping, so they combine with a width rather than competing with it. And a null
character renders as nothing, so `"{c:>3}"` on one is three pad characters: a width pads
whatever the value rendered as, and nothing is still a rendering.

For-expressions can be used inside strings to produce formatted lists:
```
"values: {for x in 1..7 {x*2}:02}"   // produces [02,04,06,08,10,12]
```

### Building a value instead of text

A format string normally joins everything into one `text`. When the type it is
assigned to says so, it **builds that type instead** — and the type is told which
bytes the author wrote and which came from a value.

A type opts in by defining `lit` plus one `hole_…` method per value kind it
accepts:

```loft
struct Query {
  const parts: vector<text>,      // the literal chunks
  const values: vector<text>,     // what was interpolated
}

fn lit(self: Query, s: text) { self.parts += [s] }        // author bytes
fn hole_text(self: Query, v: text?) { self.values += [v ?? ""] }
fn hole_int(self: Query, v: integer) { self.values += ["{v}"] }
```

Then a format string with that target type calls them, in source order:

```loft
name = "ada";
q: Query = "SELECT * FROM t WHERE name = {name}";
// calls q.lit("SELECT * FROM t WHERE name = ") then q.hole_text(name)
```

The target comes from the type you assign to, a struct field you initialise, a
function parameter, or a return type — there is no new syntax, and `text` behaves
exactly as before. So a builder function needs no local to route through:

```loft
fn where_name(name: text) -> Query { "SELECT * FROM t WHERE name = {name}" }
```

A string written inside a **hole** does not inherit the destination's type: a hole
is not the destination, so `q: Query = "{"seed"}"` passes `"seed"` to `hole_text`
as a value rather than building a second `Query` from it.

Why this exists: a value that has been rendered into text cannot be told apart
from text the author wrote. Keeping them separate is what lets a library build a
SQL statement, a shell command, an HTML fragment or a file path in which **an
interpolated value can never become syntax** — the value simply has no route into
the text, because the only path in is `lit`.

The method names, and which kind each hole uses:

| hole type | method |
|---|---|
| `text` (and `text?`) | `hole_text` |
| `integer` | `hole_int` |
| `float` | `hole_float` |
| `single` | `hole_single` |
| `boolean` | `hole_boolean` |
| `character` | `hole_character` |
| a struct or enum | `hole_<type name in method case>` — `SqlIdent` → `hole_sql_ident`, `Level` → `hole_level` |

A hole of your OWN type is what lets a builder treat one hole differently from
all the others. A SQL table name cannot be a bound parameter — no placeholder
stands for it — so a query builder has to put it in the statement itself, and
making it a type is what keeps that safe:

```loft
tbl = ident("orders");                              // null if it is not a name
q: SqlText = "SELECT id FROM {tbl} WHERE name = {n}";
```

`tbl` reaches `hole_sql_ident` and the builder writes it into the text; `n`
reaches `hole_text` and is bound. Nothing constructs a `SqlIdent` but `ident`,
which refuses anything that is not a name — so there is one place to check
rather than a rule to remember.

Rules worth knowing:

- **A missing `hole_…` is a compile error** naming the method to add. A value is
  never quietly rendered to text instead — that would undo the point.
- **`text?` is allowed** for `hole_text`, so an absent value stays distinct from
  the empty string. This is how a SQL builder tells NULL from `''`.
- **A format spec is refused** on a hole (`"{x:>8}"`), because the value is handed
  over rather than rendered, so there is nothing to format.
- **A string with no holes still builds the type** — an empty statement is still a
  statement.
- **A hole does not inherit the target type.** A string literal INSIDE a hole
  (`"{"seed"}"`) is ordinary text, not a second value of the target type — which
  also means a format string in argument position inside a hole
  (`"{ build("p{n}q") }"`) is text, so build it in a local first.

`tests/scripts/interpolation-hook.loft` is a complete worked example, and
`tests/fixtures/sqldb/sql/src/sql.loft` is a real one.

---

## Control flow

### If / else if / else

```
if condition {
    // ...
} else if other {
    // ...
} else {
    // ...
}
```

`if` can be used as an expression when both branches produce a value:
```
result = if x > 0 { x } else { -x }
```

### For loops

```
for item in collection {
    // item is each element
}
```

Ranges:
```
for i in 1..10 { }            // 1 to 9 (exclusive end)
for i in 1..=10 { }           // 1 to 10 (inclusive end)
for i in 0..2147483647 { }    // near-unbounded (break as needed)
```

Text iteration yields characters:
```
for c in some_text { }    // c: character
```

Filtered iteration:
```
for item in collection if item.active { }
```

Reverse iteration:
```
for i in rev(1..10) { }        // integer range in reverse (9, 8, 7, …, 1)
for x in rev(sorted_col) { }   // sorted / index collection in reverse key order
```

Inside a loop, the iteration variable supports several attributes using `#`:

| Attribute    | Meaning                                                                              |
|--------------|--------------------------------------------------------------------------------------|
| `v#index`    | For **text** loops: byte offset of the **start** of the current character.           |
|              | For **vector** and **sorted** loops: 0-based position of the current element.        |
|              | Not supported on **index** loops (compile error — use `#count` instead).             |
| `v#next`     | For **text** loops only: byte offset immediately **after** the current character.    |
| `v#count`    | Number of iterations completed so far (works on all collection types).               |
| `v#first`    | `true` for the first element only (works on all collection types).                   |
| `v#remove`   | Remove the current element — in a filtered loop or a plain one (see below).          |

**Collection type support matrix:**

| Attribute | `vector` | `sorted` | `index` | `hash` |
|-----------|----------|----------|---------|--------|
| `#first`  | ✓        | ✓        | ✓       | N/A — cannot iterate directly |
| `#count`  | ✓        | ✓        | ✓       | N/A |
| `#index`  | ✓ (0-based) | ✓ (0-based array position) | ✗ compile error | N/A |
| `#remove` | ✓ (filtered) | ✓ (filtered) | ✓ (filtered) | use `h[key] = null` |

**Gotcha — `#index` does not mean the same thing on text and vector.** On a text
loop `c#index` is a **byte offset** into the underlying UTF-8 (so it advances by
2–4 per non-ASCII character); on a vector or sorted loop `v#index` is a 0-based
**element position**.  Code that relies on `#index` being a counter — say
`if c#index == 5 { … }` — works on ASCII, then quietly stops working when an
emoji or accented letter is added.  When you want a 0-based character count
that matches vector semantics, use `c#count`; when you want byte offsets for
slicing (e.g. `txt[c#index..c#next]`), use `c#index`.

Text iteration example — `#index` and `#next` are consistent: `c#next == c#index + len(c)`,
with one exception.  A NUL character reports `len(c) == 0` (`character`'s null IS code
point 0, so the two are one value), while the walk still steps over its one byte — there
`c#next == c#index + 1`.  Slicing `txt[c#index..c#next]` stays correct either way; only
`len(c)` under-reports.  Guaranteeing forward progress is what stops a NUL from ending
the loop, or spinning it forever (loft#755).
```
// "Hi 😊!": H@0..1, i@1..2, ' '@2..3, '😊'@3..7, '!'@7..8
for c in "Hi 😊!" {
    // c#index = start byte of current character
    // c#next  = first byte of the next character
}
```

`v#remove` is only valid inside `for ... if ...` loops:
```
for v in x if v % 3 != 0 {
    v#remove;
}
```

It removes exactly one element, releases what that element owned, and — when the
collection is one member of a [linked collection group](DATABASE.md#removing-one-entry-with-eremove-loft903)
— takes the element out of every member.  Removing while walking backwards
(`for e in rev(c)`) is equally safe: the cursor lands on the next element in the
direction being walked.

**Mutation guard:** Appending to a collection while iterating over it is a compile error:

```
for e in v { v += [4]; }  // ERROR: Cannot add elements to 'v' while it is being iterated
```

This protects against infinite loops (vectors re-read their length each step) and data
corruption (sorted/index insertions invalidate stored iterator positions).

Exceptions:
- `e#remove` is safe and allowed — it adjusts the iterator position after removal, so every
  element is still visited. A filter chooses WHICH elements to drop; it is not what makes
  removal legal, and `for e in v { e#remove; }` empties the vector visiting each element once.
- Field accesses are not blocked: `db.items += x` is allowed even if `db.items` is iterated via a local variable.

### While loops

```
while <condition> { }
```

Repeats as long as the condition holds, and is the only unbounded loop loft has —
`while true { }` runs until something stops it, where a `for` over a range carries
its own upper bound. `break` and `continue` work inside it exactly as in a `for`.

A `while` has no loop VARIABLE, so it cannot be named by the labelled forms below:
there is no way to leave an outer `while` from inside an inner loop except a flag.
A labelled `x#break` does cross a `while` on its way out, so an inner `while`
nested in a `for x` can still leave that `for`.

### Break and continue

```
break
continue
```

Only valid inside a loop.

**Labelled break — `loop_var#break`.** To exit an *outer* loop from inside an inner
one, write `outerVar#break` using the loop variable's name.  This reuses the
`#attribute` syntax (see § Loop attributes — `#first`, `#count`, `#index`,
`#remove`) as a control-flow statement: `x#break` is not a property read but a
jump to just past the loop whose iterator is `x`.  A bare `break` always exits
the nearest enclosing loop.

```
for x in 1..5 {
    for y in 1..5 {
        if y > x       { break; }        // exits the inner y loop only
        if x * y >= 16 { x#break; }      // exits BOTH loops (jumps past x loop)
    }
}
```

**Gotcha (INC#18).** `x#break` looks like an attribute access but is a jump
instruction — it produces no value and cannot appear on the right of `=`.  The name
must be a real loop variable: an ordinary local currently crashes the compiler
rather than being diagnosed (loft#998).

**Labelled continue — `loop_var#continue`.** Symmetric to `x#break`: use
`x#continue` from inside an inner loop to skip the remainder of the current
*outer* iteration (named by `x`).  Semantics: jump back to the top of the
`x` loop and advance it by one step, abandoning any remaining inner-loop
iterations and any code between the inner loop's closing `}` and the end
of the outer body.  A bare `continue` still targets only the innermost
loop.

### Return

```
return value
return           // for void functions
```

The last expression in a block (without a trailing `;`) is automatically returned.

`?? return` (C56): if the left side of `??` is null, return from the function
immediately with the right-hand value:
```
id = param(req, "id") ?? return bad_request("missing id");
val = lookup(id)       ?? return;    // void function: return nothing
```

### Custom iterators (I13)

Any type with a `fn next(self: T) -> Item?` method can be used in a `for` loop.
Returning `null` from `next` terminates the loop:

```
struct Counter { current: integer, limit: integer }
fn next(self: Counter) -> integer? {
    val = self.current;
    self.current = val + 1;
    if val >= self.limit { return null; }
    val
}

c = new_counter(5);
for x in c { }    // iterates 0, 1, 2, 3, 4
```

`Item` is any type: a scalar, a `text`, a struct, a collection, or an enum.  Whatever it
is, the loop ends on the null of THAT type and on nothing else — a `0`, an `""` and a
`false` are ordinary elements, and the loop yields them.

Declare the item `Item?`, not `Item`.  Both run, but a non-null SCALAR return warns when
`next` hands it `null`, because a scalar's null is a reserved value of its own range.

Inside the body the loop variable is typed as the non-null `Item`: the loop has already
ended by the time `next` answers null, so the body never binds one.

`#count` and `#first` work; `#index` and `#remove` are not available.

### Parallel blocks (A15)

`parallel { }` runs each top-level expression concurrently, and continues once every one
of them has finished:

```
parallel {
    task_a();
    task_b();
}
// continues after both arms complete
```

No trailing `;` is required after the closing `}`.

Each expression is one ARM, and each arm runs against its own read-only view of the
program's state.  So an arm may READ an enclosing local, and may declare and use locals of
its own, but it may not write enclosing state — assigning an enclosing local, mutating one
through a reference, and capturing a function parameter are all compile errors, not silent
no-ops.  Copy a parameter into a local first if an arm needs its value.

An arm's result is discarded.  When you need the answers back, use `for x in xs par(y =
f(x), n) { … }`, which delivers each worker's value to the loop body.

Both backends run the arms concurrently.  (On `--native` this needed loft#1054: the block
used to compile to nothing there, so the arms never ran and the program exited 0 having
done none of the work.)

### Match expressions

Pattern matching dispatches on enum variants, scalar values, or struct types:

```
result = match direction {
    North | South => "vertical",
    East | West => "horizontal"
}
```

**Enum match:** each arm names a variant. All variants must be covered or a `_` wildcard
must be present. Or-patterns (`|`) combine variants into a single arm. Struct-enum arms
can destructure fields:

```
match shape {
    Circle { radius } if radius > 0.0 => PI * radius * radius,
    Circle { radius }                 => 0.0,
    Rect { width, height }            => width * height
}
```

Whether a destructured field is a **view of the subject** or a **copy** depends on the
field's type:

| payload type | writing through the binding |
|---|---|
| `text`, `vector`, and other heap values | updates the value being matched — a **view** |
| `integer`, `float`, `boolean` and other scalars | changes the binding only — a **copy** |

```
match e {
    Holder { items } => { items += "y"; }      // heap: `e`'s payload is now "…y"
    _ => { }
}

match n {
    Num { v } => { v = 9; }                    // scalar: `n` is unchanged
    _ => { }
}
```

The view rule holds for a `text` payload as well as a heap one (loft#673) and through
nested patterns (`Wrap { inner: Holder { items } }`).  To change a scalar payload, build
the variant again (`n = Num { v: 9 }`).

A whole-value BIND is a third case and always copies — `b = e.items; b += "y"` leaves
`e` alone (C86); the difference is that a pattern binding is not a bind, and you never
wrote the copy.  A subject that is a temporary (`match make_e() { … }`) has nothing to
write back to, so a write there updates only the arm's own view.

**Scalar match:** the subject is an integer, text, float, boolean, or character. Arms
are literal values, ranges, `null`, or `_`:

```
match score {
    null     => "absent",
    90..=100 => "A",
    80..90   => "B",
    1 | 2 | 3 => "low",
    _        => "other"
}
```

**Guard clauses:** any arm may have an `if` guard after the pattern. The guard is
evaluated when the pattern matches, with the arm's captures already bound, so it can
test them (`[a, _, _] if a > 10`, `(n, _, true) if n > 10`); if the guard is false,
matching falls through to the next arm. The one arm that refuses a guard is a **cursor**
match arm, where the pattern advances the shared cursor before the guard could be tested
— put the test inside the arm body there. Guarded arms do **not** count toward exhaustiveness — because the guard
can fail at runtime, the compiler cannot guarantee the arm will handle that variant.
Even if every variant has a guarded arm, a wildcard `_ =>` or an unguarded arm covering
each variant is still required:
```
match color {
    Red if is_bright   => "bright red",
    Green if is_bright => "bright green",
    Blue               => "blue",
    _                  => "other"       // required — Red and Green guards may fail
}
```

**JsonValue match:** the typed JSON tree returned by `json_parse(text)` is a
struct-enum, so pattern matching is the canonical way to dispatch on a parsed
JSON value.  Each arm names a variant; the destructured field exposes the
inner payload (`items` for `JArray`, `fields` for `JObject`, `value` for the
primitive variants).  A wildcard or `JNull` arm covers parse failures.
Assign the parse result to a variable first, as below — the inline form
`match json_parse(raw) { … }` compiles and dispatches correctly, but the
interpreter currently leaks one store per inline parse (the hoisted
temporary is never freed), so the assign-first form is the reliable one.

```
v = json_parse(raw);
match v {
    JObject { fields } => for f in fields { handle(f.name, f.value) },
    JArray  { items }  => for vi in items { handle_element(vi) },
    JNumber { value }  => log_info("scalar number: {value}"),
    JNull              => log_warn("parse error: {json_errors()}"),
    _                  => log_warn("unsupported root kind")
}
```

**Match is an expression:** it produces a value that can be assigned or returned. All
arms must produce the same type (or void).

### Slice patterns (matching a vector)

A match arm can also describe the **shape of a vector**: how many elements it has, what sits
at the front or the back, and what lies in between. Each element you name becomes a variable
inside the arm.

```
match v {
    []           => "empty",
    [only]       => "one: {only}",
    [first, ..]  => "starts with {first}",
    _            => "other"
}
```

**A slice pattern matches the whole vector.** `[a, b, c]` matches a vector of exactly three
elements — not the first three of a longer one. Use `_` for an element you do not need, and a
literal to require a particular value:

```
match v {
    [a, _, c] => a + c,      // exactly 3 elements; the middle one is ignored
    [1, x]    => x,          // exactly 2, and the first must be 1
    _         => 0
}
```

**`..` skips the middle.** Put it between the elements you care about. The vector may be any
length that leaves room for them:

```
match v {
    [first, .., last] => last - first,   // 2 or more elements
    _                 => 0
}
```

**`..name` also captures what it skipped**, as a `vector` you can return, index, or change:

```
match v {
    [head, ..tail] => len(tail),        // everything after the first
    _              => 0
}

match v {
    [first, ..mid, last] => len(mid),   // everything between the two ends
    _                    => 0
}
```

Order the arms from the most specific to the least: `[head, ..tail]` already matches every
vector with at least one element, so an arm below it that also starts with one element would
never be reached.

**Every slice pattern can fail, so a match on a vector needs a final catch-all.** No set of
slice arms is ever complete on its own, because some length always escapes them. End with `_`
or with a bare name, which binds the whole vector:

```
match v {
    [a, b] => a + b,
    whole  => len(whole)      // or `_ => 0`
}
```

Leaving it out is a compile error: *"match on vector is not exhaustive — a slice pattern can
fail (a length no arm matches); add a `_ =>` or a bare-binding final arm"*.

**What a binding gives you** depends on the element type, and follows the same view-or-copy
split as the rest of the language:

| element type | writing through the binding |
|---|---|
| a struct, or a nested `vector` | updates the element inside the matched vector — a **view** |
| `integer`, `float`, `boolean`, `character` | changes the binding only — a **copy** |
| `text` | changes the binding only — a **copy** |
| a `..name` rest, or a repetition's collected vector | a **fresh** vector, independent of the subject |

```
match ps { [p, ..] => { p.x = 99; }, _ => { } }   // struct element: ps[0].x is now 99
match ns { [n, ..] => { n = 77; },   _ => { } }   // scalar element: ns is unchanged
```

Note the one place this differs from an enum arm: a `text` **payload** of a variant is a view
(above), but a `text` **element** of a vector is a copy. Mutating a captured `..rest` never
touches the vector it came from, which is what makes it safe to return.

**Element patterns.** Beyond a name, a `_` and a literal, an element of a struct-enum vector
can be matched by its variant, and a run of elements can be collected into one binding. This is
what lets a single arm describe the shape of a token sequence:

| form | matches |
|---|---|
| `Kw { word }` | an element of that variant, binding its field |
| `(A { n } \| B { n })` | either variant — the branches must bind the **same** field name |
| `(x: Num)*` | zero or more `Num` elements, collected into `x` |
| `(x: Num)+` | one or more |
| `(x: Num)*(Comma)` | a run with a separator between items; the separator is not collected |
| `xs:integer*` | the same for a plain scalar element type |
| `(Kw { k } Op { o })?` | an optional group — if absent, its captures read `null` |

```
match toks {
    [Kw { word }, ..rest]        => "keyword {word}, {len(rest)} more",
    [(x: Num)*, End { e }]       => "{len(x)} numbers then {e}",
    _                            => "no match"
}
```

A variant element may itself be matched deeper — `[Box { inner: Num { n } }, ..]` binds `n`.
A slice arm takes an `if` guard like any other arm.

**Two limits worth knowing.** An element written **after** a `..` must be a plain name or `_`;
a literal or a variant pattern there does not parse (loft#1419). And a multi-pattern arm
(`A { r }, B { r } => …`) is for enum variants only — it does not accept slice patterns.

### `is` variant check

The `is` operator tests whether an enum value is a specific variant:

```loft
d = North;
if d is North { ... }       // true
assert(!(d is South));       // negation
```

For struct-enums, `is` can also capture variant fields into local variables:

```loft
s = Circle { radius: 3.14 };
if s is Circle { radius } {
  area = PI * radius * radius;   // radius is in scope here
}
// radius is NOT in scope here
```

Multiple fields:
```loft
if shape is Rect { width, height } {
  area = width * height;
}
```

With else:
```loft
if shape is Circle { radius } {
  area = PI * radius * radius;
} else {
  area = 0.0;
}
```

In loops:
```loft
for item in shapes {
  if item is Circle { radius } {
    total += radius;
  }
}
```

**Disambiguation:** `if s is Circle { radius } { body }` — the parser
uses lookahead to distinguish field capture `{ ident [, ident]* }` from
an if-body `{ statements }`.  If the `{` is followed by an identifier
then `,` or `}`, it is a field capture; otherwise it is the if-body.

---

## Variables

Variables are declared implicitly on first assignment. Their type is inferred:

**Struct assignment copies the record — but vector-element READS are
views.**  `a = b` for struct-typed VARIABLES deep-copies `b`'s record into
a fresh store owned by `a` — the two variables do NOT alias afterwards
(`b.v = 42` leaves `a.v` unchanged).  Reading a struct ELEMENT into a
local (`e = v[i]`) is different: it yields a dep-tracked VIEW of the
slot's record (`e.f = x` mutates `v[i]`, and a later write to `v[i]`
changes what `e` reads) — see the swap warning under § Vectors.  Other
explicit aliasing: `reference<T>` struct fields share by pointer (#328),
and closure captures of struct references share the live record (capture
rules above).

**A `reference<T>` field may be nullable, and the `?` costs it nothing.**
`next: reference<Node>?` is the linked-list terminator: the field keeps the
pointer's own bytes and spells absence inside them, so it is the same size
and still shares.  This works on a type that points back at ITSELF — a list,
a tree, a mutual pair — where a plain `Node?` field cannot, because that one
stores the record INLINE and a record cannot contain itself.  The two are
different types, not two spellings of one: `reference<Leaf>?` links, `Leaf?`
copies.
```
x = 42
name = "hello"
items = [1, 2, 3]
```

Variables may be explicitly initialized from expressions:
```
data = configuration as Program
```

---

## Vectors

```
v = [1, 2, 3]               // create with literal
v: vector<integer> = []     // empty vector with type annotation
buf: vector<single> = []    // empty vector of f32
v += [4]                    // append one element
v += [5, 6]                 // append multiple elements
for x in v { }             // iterate
v[i]                        // index; i>=len -> null, negative i counts from the end (see below)
v[start..end]               // slice range (end exclusive)
v[start..=end]              // slice range (end inclusive)
v[start..]                  // open-ended slice to end
v[..end]                    // open-start slice from 0 to end (exclusive)
[elem; 16]                  // repeat initializer: 16 copies of elem
[for n in 1..7 { n * 2 }]  // vector comprehension (builds [2, 4, 6, 8, 10, 12])
[for n in 1..10 if n % 2 == 0 { n }]  // comprehension with filter
```

**A vector can also be taken apart by `match`** — `[first, ..rest]`, `[a, .., z]` and the other
slice patterns are in [§ Slice patterns](#slice-patterns-matching-a-vector).

**Slices are iterators, materialised on assignment.**  `v[lo..hi]` can
be used in `for x in v[lo..hi] { … }` and wherever an iterator is
accepted, and assigning it to a local (`sub = v[lo..hi]` or
`sub: vector<T> = v[lo..hi]`) materialises a fresh vector.  It still
cannot be passed directly where a `vector<T>` **argument** is expected
("expected vector<integer>, got iterator<integer>") — materialise first
via a local, or a comprehension: `f([for x in v[lo..hi] { x }])`.

**Negative slice bounds count from the end (@P384).**  `v[2..-1]` is
"element 2 up to (not including) the last": on `[10, 20, 30, 40, 50]`
it yields `30, 40`.  `v[-2..]` yields the last two elements.  A
negative bound is shorthand for `len(v) + bound`, so `v[0..len(v) - 1]`
and `v[0..-1]` are the same slice.

**Scalar `v[i]` follows the same negative rule — mind the null-guard footgun.**
The full picture: `i ∈ [0, len)` → the element; `i ≥ len` → `null`; a **negative**
`i ∈ [-len, -1]` counts **from the end** (`v[-1]` is the last element, `v[-len]` the
first — the same rule as negative slice bounds above); `i < -len` → `null`.  Because a
negative index in range yields a real element, a *computed* index that can go negative
does **not** null-guard: `if v[i] { … }` and `v[i] ?? d` only catch `i ≥ len`, not a
`-1` "not-found" sentinel or a subtraction underflow.  Test `if i >= 0` first (or `?? d`
only after a `>= 0` check) when `i` may be negative.

**Struct elements: reads are views, writes are copies — never swap
in-place via a temp (#338).**  For a `vector<STRUCT>`, `tmp = v[j]` yields
a dep-tracked LINK to slot `j`'s record (writes through `tmp` mutate
`v[j]`); but `v[j] = v[k]` COPIES `k`'s record bytes into slot `j`'s
storage.  The classic swap therefore silently corrupts:

```loft
tmp = v[j];     // a view of slot j
v[j] = v[k];    // copies k's record INTO slot j — tmp now reads k's record
v[k] = tmp;     // writes k's record back: j's record is LOST, k's DUPLICATED
```

Swap through scalar temps per field, rebuild into a fresh vector
(selection sort instead of in-place insertion sort), or copy the record
explicitly through a fresh struct literal before overwriting the slot.

**A view lasts only as long as the place it names, and loft tells you when it
stops.**  `tmp = v[j]` (and a struct-typed field read, `w = o.inner`) is a link
into the container, so it stays valid only while that place does.  Where the
compiler can see the place will not survive the binding, it gives `tmp` its own
copy instead — taken at the bind, so it holds the value you bound — and says so:

```
advice: in `f`, `c` was copied out of `bx` because `bx` is reassigned while `c`
  is in use — a view names a place inside `bx`, and giving `bx` a new value
  leaves nothing for it to point at. Writes through `c` no longer reach `bx`.
```

Three things end the place: **removing** from the container (`v.remove(i)`
renumbers the rest), **writing a key field** through the view on a keyed
collection, and **reassigning the container itself** (`bx = T{…}`).  After the
copy, writes through the binding no longer reach the container — which is why it
is reported and not silent.  To keep writing through, re-read the view after the
change (`c = bx.v[0]`).

The copy happens only when you still USE the view after the change — *"while `c`
is in use"* is meant literally.  Finish with the view first and it keeps writing
through, so moving the last use above the removal is a second way out:

```loft
c = v[0];  c.n = 99;  v.remove(2);   // no copy — `c` is done before `v` changes
c = v[0];  v.remove(2);  c.n = 99;   // copied — `c` is used after `v` changed
```

Overwriting a place is not ending it: `o.inner = Box{…}` writes into the place
`o.inner` already occupies, so a view of it sees the new value and still writes
through.

**Write `&` and you get an error instead of a copy.**  `c = &v[0]` says *"I want a
live link"* — an ownership decision, not a hint — so loft will not quietly hand you a
copy.  Where it cannot honour the link, it refuses the program instead.  All three
things that end a place do this: removing from the container, writing a KEY field
through the reference, and replacing the container itself.

```
error: cannot remove from `v` while `c` references an element of it — a removal
  renumbers the remaining elements, so a write through `c` would no longer reach
  the element it names. Move the removal after the last use of `c`, or bind
  without `&` to work on a copy
```

The same refusal covers a **call**: passing an element of a container and the
container itself to a function that removes from it (`shift(v[2], v)`) is rejected,
whether or not the parameter is spelled `&` — a struct parameter names the caller's
element either way, so the write would be lost either way.  Pass the INDEX instead
and read the element again after the removal:

```loft
fn shift(idx: integer, all: &vector<Box>) { all.remove(0); all[idx - 1].n = 99; }
```

For a keyed collection the remedy is to re-insert rather than to reorder, because the
key write IS the thing that cannot be honoured — changing a key would leave the element
reachable by no key at all:

```loft
c = &s[30];  c.key = 5;      // refused
s[5] = s[30];  s[30] = null; // say it directly instead
```

**Why an error rather than some defined behaviour.**  loft may always drop an error
later, but never add one after the language freezes — so refusing keeps the door open.
If loft one day gains the machinery to honour these references properly, every program
that compiles today still compiles and the refused ones start working.  Had it shipped
the silent copy instead, that copy would be the contract forever.

Full rule + the reasoning:
[OWNERSHIP_MODEL.md § A view lasts as long as the thing it names](OWNERSHIP_MODEL.md#a-view-lasts-as-long-as-the-thing-it-names--and-loft-says-when-it-does-not).

**Empty vectors** require a type annotation so the compiler knows the element type.
Use `v: vector<T> = []` instead of the older `[for _ in 0..0 { default }]` pattern.

To remove elements while iterating, use `v#remove` inside a filtered loop (see [For loops](#for-loops)).

---

## Key-based collections (hash / index / sorted)

All three keyed collection types support single-element removal by assigning `null` to a subscript:

```loft
h[key] = null          // hash: remove element whose key field equals key
idx[nr, name] = null   // index: remove element by compound key
s[key] = null          // sorted: remove element by key field
```

Removing a key that is not present is a **no-op** (safe, no error).

`sorted` and `index` collections support forward and reverse iteration:
```loft
for v in sorted_col { }         // forward — visits elements in key order
for v in rev(sorted_col) { }    // reverse — visits elements in reverse key order
```

Lookup also returns `null` when an element is absent:
```loft
if h[key] { /* found */ }
elem = idx[42, "foo"]    // null if not present
```

**All keyed-collection subscripting is by KEY, never by position (C99).** On a
`sorted` / `index` / `hash`, `coll[k]` looks up the element whose key **equals**
`k` — not the element at position `k` — so `s[20]` finds key 20 and `s[1]`
returns `null` when there is no key 1 (it is not position 1). On an ordered
`sorted` / `spatial`, a **range** subscript is likewise key-addressed: `s[lo..hi]`
is a **key-range** query (`s[15..35]` selects the elements whose key is in
`[15, 35)`, not positions 15..35), and `spatial` uses the same form for proximity
(`xs[(x,y)..(x2,y2)]`). This is the same key-addressing as `s[k]` and
`s[k] = null` — it only *looks* like a vector's positional slice `v[1..3]`.
**Porting a positional `v[1..3]` to a sorted changes its meaning**; use the key
range you want, or iterate and count positions yourself.

**Gotcha (INC#2) — vector has a richer literal/build API than the keyed
collections.** All four collection types share `+=`, `for` iteration
(hash iterates via its internal ordered index), and subscript removal
(`h[key] = null`), but **comprehensions** (`[for x in c if p { … }]`)
produce only a `vector<T>`; there is no `sorted<T>` / `index<T>` / `hash<T>`
comprehension form — instead, build a vector literal and assign it to a
keyed-collection field, which does implicit conversion.  Inside a `for`
loop, `#index` is valid on vector and sorted but a compile error on index
collections.  When porting code between collection types, treat these gaps
as structural differences rather than bugs — they are intentional and not
planned to close.

**`spatial<T[x,y]>` / `spatial<T[x,y,z]>` (1–3 coordinate axes, @PLN48) is a
related keyed collection** backed by a Morton/Z-order radix tree.  It shares
`+=` append, `for` iteration (visited in the tree's natural Morton order —
no sort, unlike `hash`), `.len()`, and the full point subscript: `xs[x, y]`
reads the record at exactly that point (`null` when empty), `xs[x, y] = mob`
inserts-or-replaces, and `xs[x, y] = null` removes — the same three roles
`h[k]` has on a `hash`.  Note the point subscript takes the coordinates as
separate subscripts (`xs[3, 6]`), where the range forms below parenthesise
them.  Proximity queries use range-slice
syntax instead of new keywords or methods: `xs[(x1,y1)..(x2,y2)]` is the
bounding box and gives exactly what is inside it (loft#800), while
`xs[(x,y)..]` and `xs[(x,y)..:n]` walk OUTWARD from that point, nearest
first — two cursors seeded either side of the query, so `..:n` answers `n`
records from any origin and a query past every record still answers its
neighbours.  They used to be the Morton TAIL, where a record just behind the
query was never returned however close it was (loft#1002).  The walk is
APPROXIMATE: it orders by Morton distance, which jumps at quadrant
boundaries, so a truly-near point can arrive a little late.  Reach for a
symmetric box, `xs[(x-r, y-r)..(x+r, y+r)]`, when the answer must be exact.
See [STDLIB.md § Keyed collections](STDLIB.md#keyed-collections-hash--index--sorted)
for the full syntax table.

**`trie<T[field]>` keys on ONE text field** and shares the same radix tree as
`spatial` — only the key oracle above it differs, so `spatial` is coordinates and
`trie` is bytes.  It shares `+=` append, `for` iteration (in key order, no sort),
`.len()`, and the exact subscript `t[key]` (`null` when absent — never a
neighbour).

What it offers that `sorted<T[text]>` cannot is the **prefix**:

```
for w in words["kerk"..]    { … }   // every key beginning with "kerk", in key order
for w in words["kerk"..:20] { … }   // the first 20 of them
```

The prefix IS the query.  A `sorted` range needs a successor string
(`words["kerk".."kerl"]`) that the caller has to construct, gets wrong at a byte
boundary, and which answers a key INTERVAL rather than a prefix — so `t[a..b]` is
refused, and it names `sorted` as the kind that answers an interval.  Key order is
BYTE order and the terminator sorts before any byte, which is why `kerk` precedes
`kerkstraat` precedes `kerkweg`.  Exactly one key field: a trie orders one key's
bytes, so several keys have no order to share (use `sorted<T[a, b]>` for that).

### Two collections over one element type are ONE record set

Declare two collections over the same element struct in one struct and you get **two routes to
a single set of records**, not two collections. Filling either fills both:

```loft
struct Tile { k: integer, n: text }
struct Level { tiles: vector<Tile>, by_key: hash<Tile[k]> }

lvl = Level { };
lvl.tiles += [Tile { k: 7, n: "gate" }];
// len(lvl.by_key) is 1 — the hash is a VIEW of the same record
```

That is the point of the feature: the keyed view stays in step with the list with no code to
keep it in step, and it works whichever member you spell the insert through.

**When it happens — one line:** two or more collections over the same element type, in the same
struct **or struct-enum variant**, **at least one of them keyed** (`hash` / `sorted` / `index` /
`spatial` / `trie`). Field order does not matter, and neither does whether the vector's element
is nullable (`vector<Tile?>`). Two plain vectors over one element type are **independent** — a group needs a
keyed member.

**When you want them apart**, say so in the TYPES — there is no per-field opt-out:

| written this way | result |
|---|---|
| a second element **struct** (`vector<Chosen>` beside `hash<Tile[k]>`), fields identical | **independent** |
| the two collections in **different structs** | **independent** |
| both as **locals** rather than struct fields | **independent** |
| `type Chosen = Tile;` then `vector<Chosen>` | ⚠ **still one group** — an alias names the same type |

The second struct is the escape, and it costs a field copy to move between them:

```loft
struct Tile   { k: integer, n: text }
struct Chosen { k: integer, n: text }
fn to_chosen(t: Tile) -> Chosen { Chosen { k: t.k, n: t.n } }

struct Level { by_key: hash<Tile[k]>, picked: vector<Chosen> }
```

⚠ **Nothing at the declaration tells you which you got** — a group that did not form looks
exactly like an empty one, so `len(view) == 0` is the symptom to recognise. Details, teardown
and the record-ownership rules: [DATABASE.md § Clearing one member of a linked group](DATABASE.md).

---

## Structs and record initialization

Named form (recommended; type is explicit):
```
point = Point { x: 1.0, y: 2.0 }
```

Anonymous form (type is inferred from context):
```
point = { x: 1.0, y: 2.0 }
```

Fields not specified get their `= expr` default, or the zero value for their type.
Nullable fields default to `null`. A **bare (non-`Optional`) enum field is the one exception**: it
has no zero value (an enum's 0 is `null`, which a non-null field may not hold), so omitting it is a
compile error — provide it, give it `= <variant>`, or type it `E?` (@PLN116).

**A nullable STRUCT-enum (`Shape?`) works** as a local, a parameter, a return, a struct FIELD and
a `vector<Shape?>` element — `= null`, `== null`, truthiness, `??`, `match` and reassignment in
both directions all behave (loft#1065, loft#1071). How absence is STORED follows the slot, as the
null model says it should: a handle carries the reference sentinel, while an inline slot is a
four-byte record pointer where absence is `0`.

Iterating one works too: in `for e in v { e == null }` the loop binding is a sub-reference to the
element slot, and the test reads that slot's word rather than a handle's sentinel.

Field access uses `.`:
```
point.x
arg.long.len()
```

### Shared field names

Field names are type-scoped, not globally unique.  Different structs and enum
variants can share a field name — the compiler resolves the correct field by
the type of the receiver:

```loft
struct Point { x: float, y: float }
struct Rect { x: float, y: float, w: float, h: float }

p = Point { x: 1.0, y: 2.0 };
r = Rect { x: 10.0, y: 20.0, w: 30.0, h: 40.0 };
p.x;   // 1.0 — Point's x
r.x;   // 10.0 — Rect's x (different offset, same name)
```

This also works between struct-enum variants:
```loft
enum Shape {
  Circle { radius: float, label: text },
  Square { side: float, label: text }
}
c = Circle { radius: 5.0, label: "big" };
s = Square { side: 3.0, label: "small" };
c.label;  // "big"
s.label;  // "small"
```

Verified: works in vectors (`pts[0].x`), function parameters, and across
struct/enum boundaries.  See `tests/scripts/23-field-overlap-structs.loft`
and `tests/scripts/24-field-overlap-enum-struct.loft`.

### Value structs (`value struct`)

A `value struct` is a struct with **value (copy) semantics** and **zero heap overhead** as a
field of another record or as a vector element (@PLN101).  Reading one out of a field or element
yields an independent copy — mutating the copy never writes back through the source:

```loft
value struct Point { x: integer, y: integer }

struct Path { points: vector<Point> }

p = Path { points: [Point { x: 1, y: 2 }, Point { x: 3, y: 4 }] };
q = p.points[0];   // q is a COPY of element 0
q.x = 99;          // p.points[0].x is still 1 — no aliasing
```

- **Copy, not alias.** A plain `struct` binding is a *view* (a later `p.points[0].x = 9`
  is seen through it); a `value struct` binding is a snapshot.  This is the whole difference —
  everything else (operators via `OpXxx` methods, `{v}` / `{v:spec}` formatting, `as`
  conversions) works exactly as for a reference `struct`.
- **Zero-cost inside records and vectors.**  Fields and elements are stored inline (no per-field
  or per-element allocation), and a read-only use (e.g. `for pt in p.points { s = s + pt.x }`)
  is left as a zero-cost view — a `vector<value struct>` allocates the same as the raw scalar
  layout, flat in the element count.  A standalone local may own its own store (negligible; a
  loop reuses the slot).
- **Non-null.**  A value struct is inline bytes with no null sentinel: `value struct?` is a
  compile error, and every value struct must be initialised (there is no null value struct).
- **Method params stay zero-copy.**  `self` / `both` on a value-struct method is passed by
  reference (no copy); the copy only happens when you *bind* a value struct out of a view.

Use a `value struct` for small wrapper types that must be as cheap as the raw field they wrap
(e.g. `DateTime { ms: integer }`, `Point`, `Color`) — where the copy semantics of a scalar are
what you want.  Use a reference `struct` when you want shared/aliased mutation or nullability.

### First-grade custom types — operators, formatting, `as` (@PLN99)

A user `struct` (or `value struct`) can behave exactly like a built-in across three surfaces —
this is what makes a wrapper type (`DateTime`, `Money`, `Colour`, a `Decimal`, a URL) ergonomic
rather than a bag of functions.  Mark the type and these functions `pub` to use them across a
`use` boundary.

- **Operators** — define `fn OpLt(self: T, other: T) -> boolean` (and `OpLe/OpGt/OpGe/OpEq/OpNe`),
  `fn OpAdd/OpMin/OpMul(self: T, …) -> …`, etc., and `a < b` / `a - b` dispatch them **directly**
  (not only inside `<T: Ordered>`).  `OpMin` is the `-` operator (subtraction), `OpAdd` is `+`.
  **Operators key on `(OpName, receiver type)`** — you cannot overload the *same* operator by the
  *second* operand's type (e.g. one `OpMin(T, T)` and one `OpMin(T, U)` collide); give the second
  form a named method instead.  A type with no such op errors as before (`dt + 5` stays a compile
  error — distinct-type safety is free).
- **Scope end** — define `fn OpDrop(self: T)` and it runs when the value's OWNER dies: the
  binding's own scope exit, the early-`return`/`break` paths, reverse-declaration order within a
  scope (@PLN125 arc B), and a REASSIGNMENT of the binding, which releases the record it
  displaces once the new value is computed.  Copying a droppable into a struct field, an enum payload or a
  collection element MOVES it — the source stops dropping, and the container's death releases
  what it holds, its own hook first and then its members (@PLN139).  A plain whole-value copy
  (`t = s`, the variable an `if` arm yields, `t = s; return t`) moves it the same way: the
  copy releases, the source does not; a copy off a PARAMETER leaves the caller as the owner
  ([INTERFACES.md § `OpDrop`](INTERFACES.md)).  Taking a value back OUT
  (`v.remove(i)`, `v[i] = other`) does not release it.  A drop **cannot fail** (C80 — no caller
  is left to tell), so it may not return and anything whose failure matters stays an explicit
  call (`tx.commit()` answers, the closing brace does not).  It receives only `self`, whose data
  is COPIED at construction, so its effect reaches the world (I/O, a `#c` handle it owns) rather
  than a caller's collection.  Full contract, including what it deliberately does NOT do:
  [INTERFACES.md § `OpDrop`](INTERFACES.md).
- **Indexing** — define `fn OpIndex(self: T, i: τ) -> υ` and `x[i]` dispatches it, so a matrix, a
  bitset, a row or a ring buffer reads as `x[i]` rather than `x.at(i)` (@PLN125 arc C).  The index
  type is whatever the method declares — a row addressed by column NAME takes a `text`.  An
  interface requires it as `op [] (self: Self, i: τ) -> υ`.  `OpIndex` READS: `x[i] = …` is refused
  (a type that must be written through offers a setter, `x.set(i, v)`).
- **Formatting** — define `fn to_text(self: T, spec: text) -> text`.  Then `"{x}"` calls it with
  `spec == ""` and `"{x:anything}"` passes `"anything"` raw — the type owns its whole spec
  vocabulary (the Python `__format__` model; core learns no date/money tokens).  *Known issue
  (#533): today the body must not be a bare tail `if` — bind the result to a local and return it
  (`r = if … else …; r`), else the branch mis-selects.*
- **Conversions** — define `fn OpConvTFromS(v: S) -> T` (e.g. `OpConvDateTimeFromText`), and
  `s as T` dispatches it: `"2026-07-08" as DateTime`, `"#ff0000" as Colour`, `"1.5" as Decimal`.
  With no matching conversion, `as T` is a clean compile error (not a silent mis-cast).

**When to reach for this — and which library types still should.**  A type earns
the full treatment when it hits all four axes `DateTime` does: **(1)** it is one
value or a small fixed bundle (so `value struct`'s zero-cost copy fits), **(2)**
it has arithmetic or ordering meaning (→ operators), **(3)** it has a canonical
text form (→ `to_text`), **(4)** it converts to/from a primitive or text (→ `as`).
`DateTime` / `Duration` (the `time` lib) are the shipped exemplars.  Prime
un-upgraded candidates in the current libraries, highest-leverage first:

- **`Colour`** — today a bare packed `integer` in `graphics` / `imaging` with
  hand-rolled `rgb()` / `color_r()` free functions (the exact pre-`DateTime`
  shape).  A `value struct Colour { packed: integer }` with blend/scale
  operators, a `{c:#}` → `#RRGGBB` `to_text`, and `Colour ↔ integer` / `↔ text`
  conversions packs **flat** in pixel buffers (no heap cost) and is the clearest
  next dogfood — it exercises every part of the machinery, as `DateTime` did.
- **`Vec2` / `Vec3` / `Rect` / `Point`** — today plain **heap** structs with
  `add3` / `scale3` / `dot3` free functions; as zero-cost value structs with
  `+` / `*` / `dot` operators they allocate flat in a vector, so the win lands
  exactly where you make thousands of them (meshes, particles, physics).
- **`Version`** (registry semver — hand-rolled parse + compare), **`Angle`**, and
  a units family (`ByteSize`, `Money` / `Decimal`) round out the list.

---

## Methods and function calls

Functions whose first parameter is named `self` can be called with dot syntax:
```
text.starts_with("prefix")
text.to_uppercase()
```

Otherwise they are called as free functions:
```
len(collection)
round(PI * 1000.0)
```

**Gotcha (INC#8) — method vs. free function is the stdlib author's choice.** The
language has no rule about which operations *should* be methods vs. free
functions; it depends entirely on whether the definition's first parameter is
`self`, `both`, or neither.  Measured, that names two behaviours rather than
three: a plain first-parameter name is free-ONLY and the method spelling is
refused by name, while **both `self` and `both` accept the method AND the free
spelling** — `find_fn` resolves a free call by receiver type, so `f(x)` reaches
a `self` method.  What separates them is registration, and it shows up in the
one place neither reaches: **a `self`/`both` method is not a fn-ref value**, so
it cannot be handed to `map`/`filter` or to a parameter of function type
(loft#1008 — wrap it in a lambda, `map(v, |q| { q.m(…) })`).  The
standard library makes this call per-function: `text.starts_with(s)` and
`text.find(s)` are method-only (`self: text`); `len(v)`, `abs(n)`, `round(x)`
are both-forms (`both: …`); `sum_of(v)` and `print(s)` are free-only.  A user
cannot predict the call form without looking it up.  When in doubt, try
free-function form first — the compiler's "Unknown field" vs. "method not
found" error makes the available form obvious.

**A `&` parameter calls like the value it references.**  `&` is how an argument is
PASSED, not a different type, so inside `fn f(v: &vector<integer>)` the name `v` is
the vector and every call form it supports works on it — `len(v)`, `v.len()`,
`size(v)`.  The same holds for `&text`, the keyed collections, a `&Struct` and a
`&integer`.  There is nothing to unwrap first (loft#824):

```loft
fn total(v: &vector<integer>) -> integer {
  v += [9];        // the append reaches the caller's vector
  len(v)           // …and the length is the vector's, not a reference's
}
```

Note the trade the `&` asks for: it earns its place only when the function writes
through it.  A helper that just reads is told *"Parameter 'v' has & but is never
modified; remove the &"* — drop the `&` and the by-value signature reads the same.

### The `both` parameter name

When the first parameter is named `both` instead of `self`, the function is
registered as **both** a method and a free function:

```loft
pub fn exists(both: File) -> boolean {
  both.format != Format.NotExists
}

// Can be called as:
f.exists()      // method syntax
exists(f)       // free function syntax
```

Use `both` when a function should be equally natural as either form.
`self` registers as a method only; a plain parameter name registers as a
free function only.

### Named arguments

Any parameter can be passed by name using `name: value` syntax.  Positional arguments
come first; once a named argument appears, all subsequent must be named.  Parameters
not provided must have a default value.

```
fn connect(host: text, port: integer = 8080, tls: boolean = true) -> text
connect("example.com")                         // all defaults
connect("example.com", tls: false)             // skip port
connect(host: "example.com", port: 443)        // all named
```

Both spellings of a call take names, including the method one — `cfg.render(dry: true)`
and `render(cfg, dry: true)` are the same call.  The receiver is argument 0, so naming
it (`render(self: cfg)`) is the one thing that does not work: it is already provided.

A default is an **expression**, evaluated at the call rather than stored as a constant.
It runs once per call, not at all when the caller supplies the argument, and it may read
a parameter declared before it:

```
fn window(rows: integer, height: integer = rows * 10) -> integer { height }
window(4)      // 40 — the default reads `rows`
window(4, 7)   // 7  — the default is not evaluated
```

A default is **not part of the function's type**.  Adding one to an existing function
keeps every direct call working, but a fn-ref of type `fn(integer) -> integer` stops
matching the moment a second parameter arrives however optional it is — so growing the
signature of something handed out as a VALUE is a breaking change ([INTERFACES.md](INTERFACES.md)).

---

## Assertions

```
assert(condition)
assert(condition, "message")
```

Panics at runtime if the condition is false.

---

## Sizeof

```
sizeof(integer)    // 8
sizeof(u8)         // 1 (packed field size)
sizeof(u16)        // 2
sizeof(MyStruct)   // sum of packed field sizes
sizeof(my_var)     // size of the variable's type
```

`sizeof(TYPE)` returns the packed byte size used when the type is stored as a struct
field or vector element. For range-constrained integer types (`u8`, `u16`, etc.) this
is the packed size (1 or 2 bytes), not the stack slot size. For polymorphic enums and
references, the size is computed at runtime from the actual variant.

---

## Random numbers

Three functions for pseudo-random integer generation. All use a thread-local PCG64 generator.

```loft
rand_seed(seed: integer)                   // seed the generator
rand(lo: integer, hi: integer) -> integer  // uniform in [lo, hi]; null if lo > hi
rand_indices(n: integer) -> vector<integer>// shuffled [0..n-1]
```

`rand_seed` makes sequences reproducible:

```loft
rand_seed(42);
a = rand(1, 100);  // same value every run with seed 42
```

`rand_indices` is the idiomatic way to randomly visit all elements of a collection:

```loft
rand_seed(7);
items = ["a", "b", "c"];
for i in rand_indices(len(items)) { println(items[i]) }
```

---

## Polymorphism / dynamic dispatch

For struct-enum types, multiple functions may share the same name if each handles a
different variant as its `self` parameter. Loft generates a dispatch wrapper automatically:

```
enum Shape {
    Circle { radius: float },
    Rect { width: float, height: float }
}

fn area(self: Circle) -> float { PI * pow(self.radius, 2.0) }
fn area(self: Rect) -> float { self.width * self.height }

c = Circle { radius: 2.0 };
c.area()   // dispatches to the Circle overload
```

If a variant has no implementation, the compiler emits a `Warning` at the variant's
definition site. To silence the warning deliberately, provide an **empty-body stub**:

```
fn area(self: Rect) -> float { }   // explicit skip — no warning emitted
```

A stub with an empty body `{ }` and a `self` parameter is treated as an intentional
no-op: it emits no warnings, is callable at runtime (returns null for its return type),
and suppresses the unused-`self` warning.

Note: ordinary (non-enum) function overloading by argument type is **not** supported —
two functions with the same name and different non-variant parameter types are a compile error.

---

## Generic functions

A single type variable `<T>` lets you write a function body once for any type:

```
fn identity<T>(x: T) -> T { x }
fn pick_second<T>(a: T, b: T) -> T { _x = a; b }
```

**Rules:**
- T must appear in the first parameter (directly or as `vector<T>`, etc.).
- Only one type variable is allowed.
- At the call site, T is inferred from the first argument's concrete type.
- The compiler creates a specialised copy per concrete type automatically.

**Allowed on T:** assign, return, store in variables.

**Disallowed on T (compile-time errors):**
- Arithmetic: `x + y` → *"generic type T: operator '+' requires a concrete type"*
- Field access: `x.field` → *"generic type T: field access requires a concrete type"*
- Method calls: `x.method()` → *"generic type T: method call requires a concrete type"*
- Match, cast, struct construction on T.

```
identity(42)      // T = integer → returns 42
identity("hi")    // T = text → returns "hi"
```

---

## File structure

A loft file may contain (in any order):
- `use <library>;` imports (must appear at the top)
- `pub` / non-`pub` function definitions
- Struct definitions
- Enum definitions
- Type aliases
- Top-level constants

---

## External function annotations (`#rust`, `#iterator`)

Used only in default/library files to bind loft declarations to Rust implementations:

```
pub fn len(self: text) -> integer;
#rust "@self.len() as i32"

pub fn env_variables() -> iterator<EnvVar, integer>;
#iterator "stores.env_iter()" "stores.env_next(@0)"
```

---

## Operator definitions (internal)

Operators are defined as functions named `OpXxx` in default files and linked to
infix/prefix syntax by the parser. Examples: `OpAdd`, `OpEq`, `OpNot`, `OpConv`, `OpCast`.

---

## Shebang

Loft scripts support a Unix shebang line for direct execution:
```
#!/path/to/loft-interpreter
fn main() { ... }
```

---

## Summary of grammar (informal)

`use` declarations must appear before any other top-level declarations in a loft file.

```
file         ::= { use_decl } { top_level_decl }
use_decl     ::= 'use' identifier ';'
top_level    ::= [ 'pub' ] ( fn_decl | struct_decl | enum_decl | type_decl | constant )
fn_decl      ::= 'fn' ident '(' args ')' [ '->' type ] ( ';' | block )
struct_decl  ::= 'struct' CamelIdent '{' field { ',' field } [ ',' ] '}'
enum_decl    ::= 'enum' CamelIdent '{' variant { ',' variant } '}'
variant      ::= CamelIdent [ '{' field { ',' field } '}' ]
field        ::= ident ':' type { field_mod }
field_mod    ::= 'limit' '(' expr ',' expr ')'
               | 'not' 'null'
               | 'default' '(' expr ')' | '=' expr
               | 'virtual' '(' expr ')'
type_decl    ::= 'type' CamelIdent '=' type ';'
constant     ::= UPPER_IDENT '=' expr ';'
block        ::= '{' { stmt } '}'
stmt         ::= expr [ ';' ]
expr         ::= for_expr | match_expr | 'continue' | 'break' | 'return' [ expr ]
               | assignment
match_expr   ::= 'match' expr '{' match_arm { ',' match_arm } '}'
match_arm    ::= pattern { ( '|' | ',' ) pattern } [ 'if' expr ] '=>' expr   // both: enum arms only
pattern      ::= '_' | 'null' | literal | range | CamelIdent [ '{' field_bind '}' ]
               | ident                       // whole-subject bind; vector subjects only
               | slice_pat
slice_pat    ::= '[' [ slice_elem { ',' slice_elem } ] ']'
slice_elem   ::= '_' | ident | literal | CamelIdent [ '{' field_bind '}' ]
               | '(' variant_alt { '|' variant_alt } ')'      // alternation, shared captures
               | '(' ident ':' CamelIdent ')' rep             // collect a run of one variant
               | ident ':' type rep                           // the same for a scalar element
               | '(' slice_elem { slice_elem } ')' '?'        // optional group, captures null
               | '..' [ ident ]                               // skip, optionally capturing
rep          ::= ( '*' | '+' ) [ '(' CamelIdent ')' ]         // optional separator variant
variant_alt  ::= CamelIdent [ '{' field_bind '}' ]
assignment   ::= operators [ ( '=' | '+=' | '-=' | '*=' | '/=' | '%=' ) operators ]
operators    ::= single { '.' ident [ '(' args ')' ] | '[' index ']' | '#' ident | '?' }
               { binary_op operators }   // '?' is the @PLN116 postfix default-fallback (tightest)
binary_op    ::= '??' | '||' | 'or' | '&&' | 'and'
               | '==' | '!=' | '<' | '<=' | '>' | '>='
               | '|' | '^' | '&' | '<<' | '>>'
               | '+' | '-' | '*' | '/' | '%' | '**' | 'as'   // 'as' right operand is a type
single       ::= '!' single | '-' single | '(' expr ')' | block | '[' vector_lit ']'
               | 'if' expr block [ 'else' ( single | block ) ]
               | 'for' ident 'in' range_expr [ 'if' expr ] block
               | CamelIdent [ '{' field_init { ',' field_init } '}' ]
               | ident | integer | float | single | string | character
               | 'true' | 'false' | 'null'
range_expr   ::= expr '..' [ '=' ] expr   // exclusive or inclusive end
               | expr '..'                 // open-ended
               | 'rev' '(' range_expr ')' // reverse
```

The `{ binary_op operators }` rule above is intentionally **flat** — it does not encode
how a chain like `a + b * c ?? d` groups. That grouping is fixed by the **precedence
ladder** (twelve levels, loosest `??` to tightest `as`) and **associativity** (every level
left-associative except `**`, which is right-associative) given in [§ Operators](#operators);
a unary prefix (`!`, `-`, `~` in `single`) binds tighter than every binary operator. The
parser realises this with a precedence-climbing walk (`OPERATORS` / `parse_operators`); the
two statements — this grammar and that table — together pin every expression's shape.

---

## Best Practices

### String comparisons containing `{` or `}`

All string literals in loft are format strings — any `{...}` is interpreted as a
format expression. When comparing formatted output against a string that contains
literal braces, escape both sides with `{{` and `}}`:

```loft
// WRONG — {r:128,g:0,b:64} tries to look up variable r with format spec 128,...
assert("{p}" == "{r:128,g:0,b:64}", "...");

// CORRECT — double braces produce literal { and }
assert("{p}" == "{{r:128,g:0,b:64}}", "...");
```

Similarly for JSON format output:
```loft
assert("{o:j}" == "{{\"key\":1}}", "json format");
```

### ~~Unique field names across all structs in one file~~ (resolved)

Field lookups are type-scoped: `determine_keys()` and `position()` receive the
struct type number and search only within that struct's field list. Two structs
in the same file **may** share a field name at different byte offsets without
causing errors. Verified by `tests/scripts/23-field-overlap-structs.loft` and
`tests/scripts/24-field-overlap-enum-struct.loft`.

### Ref-param vector append

`v += items` inside a `&vector<T>` function parameter propagates back to the
caller. Both bracket-form literals and vector expressions work:

```loft
fn fill(v: &vector<Item>, extra: vector<Item>) {
    v += extra;          // appended elements are visible to the caller
}

fn add_one(v: &vector<Item>, x: Item) {
    v += [x];            // bracket-form also works
}
```

Field-level mutations via a ref-param also work as expected:

```loft
fn ok_mutate(v: &vector<Item>, idx: integer, val: integer) {
    v[idx].value = val;  // field mutation via ref-param is visible
}
```

Without `&`, everything done to the CONTENTS is visible to the caller — element writes,
appends, removes, clears. A heap parameter aliases the caller's value
([formal/calls.md](formal/calls.md) `F-ParamHeap`, enumerated for containers as `F-ParamGrow`),
so growing it is a mutation of the shared store rather than a local act.

`&` buys exactly one thing: a WHOLE-VALUE replacement writes back. `v = [7, 7]` inside a plain
parameter gives that function a different vector and leaves the caller's alone
(`F-ParamRebind`); through a `&vector<T>` the caller sees the new vector. So reach for `&` when
the function REPLACES, not when it appends (loft#1251 — this paragraph previously claimed a
plain append was local to the callee, which the rules never said).

### Polymorphic text methods on struct-enum variants

Text-returning methods on struct-enum variants that use format strings work
correctly:

```loft
enum Shape {
    Circle { radius: float },
    Rect   { width: float, height: float }
}
fn describe(self: Circle) -> text { "r={self.radius}" }
fn describe(self: Rect)   -> text { "{self.width}x{self.height}" }
```

If a variant does not implement a method, declare an empty stub with `self` as the
first parameter to suppress the warning and return null:

```loft
fn describe(self: Circle) -> text { }   // stub: returns null, no warning
```

---

## Interfaces and bounded generics

Interfaces declare a set of required methods.  A type satisfies an interface
by defining the required methods — no `impl` declaration is needed (structural
satisfaction, like Go interfaces):

```loft
interface Comparable {
  fn less_than(self: Self, other: Self) -> boolean
}

struct Priority { value: integer }
fn less_than(self: Priority, other: Priority) -> boolean {
  self.value < other.value
}
// Priority now satisfies Comparable — no explicit declaration.
```

Bounded generics use `<T: InterfaceName>` to constrain the type variable:

```loft
fn find_min<T: Comparable>(v: vector<T>) -> T {
  result = v[0];
  for item in v {
    if item.less_than(result) { result = item; }
  }
  result
}
```

Operator interfaces use `op` syntax:

```loft
interface Summable {
  op + (self: Self, other: Self) -> Self
}
fn total<T: Summable>(a: T, b: T) -> T { a + b }
total(10, 20);  // integer satisfies Summable automatically
```

Multiple bounds: `<T: Ordered + Printable>`.

**Stdlib interfaces** (defined in `default/01_code.loft`): `Ordered`, `Equatable`,
`Addable`, `Numeric`, `Scalable`, `Printable`.  Built-in types (`integer`, `float`,
`text`) satisfy them automatically via their existing operator definitions.

Bounded generics work with for-loops, method calls, and operator dispatch
on all types including structs.

**Interpolating a type variable needs `Printable`.** Inside a generic only the
BOUNDS may be relied on — that is already true of a method call, a subscript and
an operator, and formatting is not an exception, because `"{v}"` picks its op
from the value's type and a template has no concrete one to pick from:

```loft
fn show<T>(v: T) -> text { "{v}" }             // refused, and says why
fn show<T: Printable>(v: T) -> text { "{v}" }  // renders every kind
```

A **collection** of a type variable needs no bound — `"{v}"` on a `vector<T>`
dumps through the schema, which renders elements from their storage, so
`fn showv<T>(v: vector<T>)` formats fine (loft#845).

---

## Design decisions and constraints

A complete list of open issues is in [PROBLEMS.md](PROBLEMS.md).

### Error handling: null + FileResult, no exceptions

Loft uses two mechanisms instead of exceptions:

**Null returns** for simple fallible operations — handled with `??`, `!`, or `if`:

```loft
name = config.get("user") ?? "anonymous";  // fallback
f = file("data.txt");
if !f.exists() { println("not found"); return; }   // guard
clip = audio_load("hit.wav");
if clip { audio_play(clip, 0.5); }         // graceful skip
```

**`FileResult` enum** for filesystem operations that need specific error reasons:

```loft
result = delete("temp.dat");
if result == FileResult.NotFound { println("already gone"); }
if result == FileResult.PermissionDenied { println("access denied"); }
if !result.ok() { println("delete failed"); }
```

`FileResult` variants: `Ok`, `NotFound`, `PermissionDenied`, `IsDirectory`,
`NotEmpty`, `Other`.  Used by `delete`, `move`, `mkdir`, `mkdir_all`, `rmdir`,
`set_file_size`.  `NotEmpty` is `rmdir`'s alone, and it has its own variant because
it is the one failure a caller can act on: empty the directory and retry.

There are no hidden exception paths — every function's failure mode is visible
at the call site.  `assert` and `panic` are for programmer errors (bugs), not
expected failures.  In production mode (`--production`), failed asserts are
logged instead of aborting.

### Relative paths resolve against the program's directory

A relative path passed to file I/O is resolved against the **program's own
directory** (the directory of the running `.loft` file), NOT the directory you
launched loft from.  So `file("map.png")` reads the `map.png` that sits next to
the program, wherever you run it from:

```loft
// program at  /home/me/game/level.loft
// run as      loft /home/me/game/level.loft   (from anywhere)
img = file("map.png").png();   // → /home/me/game/map.png
```

This rule is uniform across **every** file-touching path:

- loft's built-in I/O (`file`, `read`, `write`, `delete`, …) joins the path onto
  the program directory directly.
- a **native library** function that does its own file I/O (raw Rust
  `std::fs`, e.g. `imaging`'s `load_png`/`save_png`) sees the same anchor,
  because loft sets the process working directory to the program directory
  before running your code.  A library author therefore does **not** need a
  special path API — a plain relative path resolves where the program expects.

Opt out with the `#cwd` directive at the top of a file (or `LOFT_PATHS=cwd`),
which anchors *both* built-in and native I/O at the process working directory
instead — useful for a CLI tool that should read files relative to where the
user invoked it.

### Closure capture: copy-at-definition, mutable within copy

Captured variables are copied into the closure at definition time (value semantics,
like Rust `move`).  Mutations after capture are not visible inside the lambda, and
mutations inside the lambda are not visible outside.  However, the closure's own
copy persists across invocations:

```loft
counter = 0;
inc = fn() -> integer { counter += 1; counter };
inc();   // 1
inc();   // 2
inc();   // 3
counter; // still 0 — outer variable unchanged
```

### Variable scoping: shared name table per file

All functions in a `.loft` file share one variable name table.  In practice this
works transparently — the compiler tracks which function each variable belongs to,
so reusing the same parameter or local-variable name across functions (including
recursive functions with `const vector<T>` parameters and `for` loops) works
correctly.

The one rule to know: **loop variables are not block-scoped.**  A `for` loop
variable lives in the function's scope, so naming it the same as an existing local
in that function is a *compile-time error*, not a silent shadow:

```loft
fn f() {
  x = 0;
  for x in 0..3 { }   // error: loop variable 'x' shadows a local named 'x'
}                      //        — rename the loop variable (e.g. loop_x)
```

Rename the loop variable (the message suggests `loop_x`) or drop the dead outer
local.  The compiler reports this up front with a fix hint; there is no codegen
panic or hidden workaround to remember.

Two *loops* may share a name freely, at any element types — each `for` binds its
own variable, so nothing is carried from one to the next:

```loft
fn g() {
  for i in ["a", "b"] { println(i); }
  for i in 0..3 { println("{i}"); }   // fine — a different variable
  println("{i}");                      // 2 — the last loop's value
}
```

Reading the variable after the loop still works, and reads what the *last* loop
that bound the name left there.  Nested loops are the exception: `for i { for i
{ } }` is rejected, because the inner binding would take over `i` for the rest of
the outer body.

A local declared **inside** a loop body splits the same way, and for the same
reason — two adjacent loops doing different work want the same short name for the
same role:

```loft
fn h() {
  for x in as1 { e = x; print("a={e.v}\n"); }
  for y in bs  { e = y; print("b={e.w}\n"); }   // fine — a different variable
}
```

The split happens only where the two types differ; a local whose type is the same
in both loops stays ONE variable, which is what keeps an accumulator working:

```loft
total = 0;
for x in as1 { total = total + x.v; }
for z in cs  { total = total + z.c; }   // still the same `total`
```

A body local read after its loop keeps the value the last iteration left, and a
loop that never ran leaves it `null` — the same answer the loop variable gives.

### Hash collections: name the key (local or struct field)

A hash (and `sorted` / `index`) needs its key spelled out, because a bare `[]`
literal is ambiguous — it could be a vector or a keyed collection.  Give the key
either way and lookup, mutation, removal, and **iteration** all work, on both the
interpreter and `--native`:

```loft
struct Entry { name: text, value: integer }

fn main() {
  // As a local variable — the type annotation supplies the key:
  h: hash<Entry[name]> = [];
  h += [Entry { name: "x", value: 1 }];
  e = h["x"];                 // lookup — works
  h["x"] = null;              // remove — works
  for kv in h { }             // iteration — works

  // Equivalently, as a struct field (the field declaration supplies the key):
  t = Table { data: [] };
  t.data += [Entry { name: "y", value: 2 }];
}

struct Table { data: hash<Entry[name]> }
```

A `[…]` literal builds a keyed collection wherever the KEYED TYPE is known — a typed
local (`h: hash<Entry[name]> = [Entry { … }]`), a return type, a parameter, a struct
field, a field default. Standing alone with no such type in view it builds a
`vector<T>`, because that is all its elements can say.

The one unsupported form is a **generic-constructor expression**
(`h = hash<Entry[name]>()`) or a bare untyped `h = []` — neither names the key.
Use the annotation (`h: hash<Entry[name]> = []`) or a field declaration instead.

#### Assigning to the collection ITSELF, not to a key

`h[k] = null` removes ONE element.  Assigning to the **field** replaces the whole
collection, on every kind (`vector`, `hash`, `sorted`, `index`, `spatial`, `trie`):

```loft
t.data = [Entry { name: "y", value: 2 }];  // REPLACES — the old contents are freed
t.data = [];                               // empties it
t.data = null;                             // empties it too (see below)
t.data += [Entry { name: "z", value: 3 }]; // `+=` is the one that APPENDS
```

`= null` empties the collection rather than making it absent: a collection field holds
a record id / claim pointer where `0` already means *no records*, and nothing in that
encoding is left to mean *absent* rather than *empty*.  So `c == null` answers `false`
even straight after `c = null` — **test emptiness with `len(c) == 0`**
([loft#917](https://github.com/loft-lang/loft/issues/917) tracks the reader half).  The
`?` makes no difference here: only the SCALAR default flips to non-null, so `vector<T>`
and `vector<T>?` are one type with one layout and take the same clear.

### Generics: single type variable

Only one type variable `<T>` is allowed, inferred from the first argument.
Multiple type variables (`<T, U>`) are not supported.

**Without bounds:** only assign, return, and store are allowed on `T`.
**With bounds (`<T: Interface>`):** method calls and operators declared
in the interface are allowed on `T`.  See § Interfaces above.

### Text: comprehensive operations

The stdlib provides `starts_with`, `ends_with`, `find`, `contains`, `replace`,
`trim`, `split(char)`, `join(separator)`, `to_uppercase`, `to_lowercase`,
`len`, and slicing.  `split` and `join` are inverses:
`"a,b,c".split(',').join(",") == "a,b,c"`.

## See also
- [STDLIB.md](STDLIB.md) — Standard library API (math, text, collections, file I/O, logging, parallel)
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
