
# Loft Language Reference

Loft is a statically-typed, imperative scripting language with null safety and built-in parallel execution.
Source files use the `.loft` extension. The language compiles to an internal bytecode representation
and can emit Rust code for host integration.

**Quick reference with common patterns and gotchas:** see the loft-write skill (`.claude/skills/loft-write/SKILL.md`).
This page states the language as it stands; a limitation the language has since lost is
recorded in [LOFT-history.md](LOFT-history.md), not here.

---

## Contents
- [Naming Conventions (enforced by the parser)](#naming-conventions-enforced-by-the-parser)
- [Types](#types)
- [Scripts (a file with no `fn main`)](#scripts-a-file-with-no-fn-main)
- [Declarations, files and grammar](#declarations-files-and-grammar) → [LOFT_DECLARATIONS.md](LOFT_DECLARATIONS.md)
- [Operators](#operators)
- [Literals, closures, strings and formatting](#literals-closures-strings-and-formatting) → [LOFT_LITERALS.md](LOFT_LITERALS.md)
- [Control flow and variables](#control-flow-and-variables) → [LOFT_CONTROL.md](LOFT_CONTROL.md)
- [Collections, structs and calls](#collections-structs-and-calls) → [LOFT_DATA.md](LOFT_DATA.md)
- [Generics, interfaces, best practices and design decisions](#generics-interfaces-best-practices-and-design-decisions) → [LOFT_DESIGN.md](LOFT_DESIGN.md)

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
[DESIGN_DECISIONS C85](DESIGN_DECISIONS_VALUES.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--)).
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

<!-- from tests/reference/types-null-arithmetic.loft -->
```loft
a = 9223372036854775807;     // i64::MAX
b = a + 1;                   // b = null  (overflow → null, NOT a wrapped value)
c = a / 0;                   // c = null  (divide-by-zero → null, execution continues)
print("done");               // ALWAYS reached
```

**`??` — a non-null fallback** (it is the null-fallback, not a trap-rescue):
`expr ?? default` yields `expr` when it is non-null, else `default`.  Use it to
turn a null result into a usable value at the spots that need one:

<!-- from tests/reference/types-null-arithmetic.loft -->
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

<!-- from tests/reference/types-null-arithmetic.loft -->
```loft
flag: boolean = false;
if !flag { ran += 1; }       // catches both null and false

count: integer = 0;
if !count { skipped += 1; }  // catches only null; zero passes through
```

The idiomatic "zero or null" check on an integer is `count == 0 or !count`,
or simply `count == 0` if the sentinel and zero should be treated the same.
This asymmetry is a deliberate, settled choice — see
[DESIGN_DECISIONS.md § C69](DESIGN_DECISIONS_VALUES.md#c69--x-on-a-non-boolean-is-a-null-test-not-logical-not).
The compiler warns when `!` is applied to a statically `not null` operand
(`!x` there is always false, since the value can never be the null sentinel).

Integer ranges can be constrained with `limit`:
```
integer limit(-128, 127)   // fits in a byte
integer limit(0, 65535)    // fits in a short
```

A `limit` type is a narrow integer exactly as `u8` is: a value the compiler cannot prove in
range is refused at the store, the call and the literal, and the refusal names the cure —
write what the value becomes when it does not fit, `x ?? 0`, or take the checked cast
`x as integer limit(0, 7)?`, which is `null` when it does not fit.  The one way an
out-of-range value reaches such a slot at run time is the slot's own arithmetic stepping past
its range (`b += 253`), and then it takes the type's default — the value nearest zero in the
range — never a wrapped one.

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

**Nullable narrow fields reserve one code for null** (#334).  The convention runs through every width:

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

**A NON-nullable narrow slot has no code for a failure, so `!` reads it beside the
store.**  `u8`, `i8`, `u16`, `i16`, `u32` and every `limit(lo, hi)` range use every code
they have, so a value that does not fit takes the type's default and is
indistinguishable from a computed one — `x: u8 = 250; x += 10` answers `0`, and so does
`x: u8 = 250; x -= 250`.  An author who cares about that edge writes the test on the
line after the store:

<!-- from tests/reference/types-null-arithmetic.loft -->
```loft
health: u8 = 250;
health += 10;
if !health { seen = "the boost did not fit — health is {health}"; }
```

The value is unchanged — the slot still takes `0`, exactly as it does with no test
written — and `!health` answers whether the store fit.  Two rules bound it:

- **The `if` must be the very next statement.** Past that there is nothing left to carry
  the status but the slot itself, and writing it there would cost every element of a
  `vector<u8>` the byte the type was chosen for.  Move the line and the check goes back
  to meaning *is this null*, which on such a type is always false — the compiler says so
  (`redundant-null-negation`), so a moved line reports itself rather than going quiet.
- **Only where the type has no code of its own.** `integer`, `i32` and every nullable
  spelling (`u8?`) keep a sentinel, so `!x` already reads their failure anywhere, and
  nothing about them changes.

`?? <value>` is the other half of the same edge and works in the same place: it names
what the slot takes instead of the type's default (`health = (health + 10) ?? 255`).

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

<!-- from tests/reference/tuple-keys.loft -->
```loft
struct Cell { pos: (integer, integer), name: text }
```

With the key a tuple, the lookup takes a tuple and the ordering compares the whole of it:

<!-- from tests/reference/tuple-keys.loft -->
```loft
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
*dispatch* rather than extraction ([C89](DESIGN_DECISIONS_SYNTAX.md#c89--no-tuple-style-enum-variants-a-matcher-reads-like-grammar-and-is-never-forced)).  A field EVERY variant
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

<!-- from tests/reference/scripts.loft -->
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

## Declarations, files and grammar

Functions, references, constants, type aliases, imports, shadowing and qualified names, file structure, `#rust` annotations, the shebang and the informal grammar: [LOFT_DECLARATIONS.md](LOFT_DECLARATIONS.md).

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

**`==` compares what a value holds, for every type (C91).** Two structs are equal when their
fields are, a vector when its elements are (in order), a `hash` / `sorted` / `index` / `spatial` /
`trie` when it holds the same records whatever order they were inserted in, a struct-enum value
when the variant and its fields are; a `reference<T>` field is followed to the record it names,
and a cyclic value terminates.  `b = a` copies (C86), so `a == b` is `true` afterwards.  A type's
own `OpEq` replaces this; `x == null` still tests presence.  Whether two names are **one record**
is asked with `&` on both sides — `&a == &b` / `&a != &b` — and `&` on one side only is refused.
`0.0 == -0.0`, and a float key agrees: `0.0` and `-0.0` are one entry of a keyed collection.
Rule: [formal/operational.md](formal/operational.md) `(E-Eq)`.

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

<!-- from tests/reference/null-operators.loft -->
```loft
name = record.optional_field ?? "unknown";
count = map_lookup ?? 0;
first = a ?? b ?? c;    // chains: first non-null of a, b, c
```

The operator is right-associative and chains: `a ?? b ?? c` is `a ?? (b ?? c)` — the same value
either way (the first non-null operand, evaluated left to right and stopping there), and the
grouping that makes every operand an arm whose bind COPIES (loft#1612, `formal/grammar.md`
(G-Assoc)).
If `lhs` has a statically-known `null` type (the bare `null` literal), `??` returns `rhs` directly.

**Result type — the discharge is only as complete as the fallback.** `a ?? b` yields `a` when
non-null else `b`, so it can still be null exactly when `b` can: the result type is the non-null
base **only if the fallback `b` is non-null**. A nullable fallback — a bare `null` literal, or a
`τ?`-typed expression — keeps the result `τ?`, so `y: integer = x ?? null` stores null into the
non-null slot and is warned (*"a nullable `integer?` is stored into the local `y` of the non-null
type `integer` — it becomes null there"*). A chain discharges to non-null iff its *last* fallback is
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

<!-- from tests/reference/null-operators.loft -->
```loft
x = (a / b)?;                // integer? → 0 on divide-by-zero
name = row.label?;           // text?    → "" when the field is null
first = points[i]?;          // Point?   → Point{} (every field defaulted) on out-of-bounds
colour = pixel.tint?;        // Colour?  → the first-defined enum variant
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

<!-- from tests/reference/null-operators.loft -->
```loft
hits: integer? = null;
hits? += 1;                 // 1     — the `?` read the zero, the write landed in `hits`
misses: integer? = null;
misses += 1;                // null  — no `?`, so the null propagates through `+`
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
fail**.  A bare `"42" as integer` is refused (`error[text-parse-may-fail]`), because a
bare cast claims it cannot fail.  Say what a bad parse gives:
```
n = "42" as integer          // refused: error[text-parse-may-fail]
n = "42" as integer ?? 0     // n : integer    (0 on a bad parse)
n = s as integer?            // n : integer?   (null on a bad parse, discharge later)
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
| `integer` → `u8`/`u16`/`i8`/`i16`/`u32`/`i32` | Explicit `as` at storage sites | NARROWING — a plain `integer` is 64-bit; writing one into a narrow **struct field**, local, parameter or return (any narrow storage) requires `as` ("cannot implicitly narrow integer to u16 … cast explicitly").  A **constant that provably fits** the target is exempt (`x: u16 = 5`, `f(200)`).  The check is range containment **or a drop in storage width**, so it covers every narrow alias — including `i32`, which the range half cannot see (see the row below).  (guard `tests/scripts/931-i32-narrowing-is-checked.loft`) |
| `integer` → `i32` — why it needs the width half | Explicit `as` | `i32` spans the whole 32-bit range, which is the range a plain `integer` *reports*: the 64-bit value lives in an 8-byte slot the bounds do not describe.  So the two specs differ in `forced_size` alone and `[s.min,s.max] ⊆ [d.min,d.max]` holds for a pair whose storage drops 8 → 4 — the one alias whose NAME says "32 bits" was the one range containment never checked, so an `i32` field would take `5000000000` and store `705032704` in silence.  The **implicit** stores compare storage width; an **explicit** `as i32` keeps the range rule alone, so it stays spellable as the cure this diagnostic prescribes |
| `float` → integer                  | Explicit `as` | `pi as integer` truncates toward zero; preserves the current sentinel semantics |
| `text` → integer / float / single  | Explicit `as` — **a PARSE: `as τ?` or `as τ ?? d`** | A text parse can fail, so a bare `"42" as integer` is refused (`error[text-parse-may-fail]`: *"a bare cast asserts it cannot"*); the same holds for `float` and `single`. Write `s as integer?` to keep the result nullable (a non-numeric text gives `null`), or `s as integer ?? 0` to supply the fallback. This is `(N-Parse)` — a bad parse is a reachable fault, exactly like `÷0` and out-of-bounds indexing (§ @PLN25). Contrast the *numeric* casts above (`float`→`integer`, width narrowing), which reinterpret an existing number rather than parse text |
| Integer / float / boolean → `text` | **Format-only** | `"n={m}"` renders the value inline; `t = m` with `t: text` is a compile error.  If you want the rendered form as a standalone text value, assign through interpolation: `t = "{m}"` |
| `character` → `integer` (codepoint)| Explicit `as` | `'a' as integer` yields 97 |
| `character` ↔ `text`               | See § String literals | Indexing vs. slicing asymmetry; concatenation via interpolation |
| `text` (of form `"VariantName"`) → plain enum | Explicit `as` / `as E?` | `"West" as Direction`; a literal naming no variant is refused, a runtime text naming none answers the first variant and warns unless written `as E?` (null) or followed by `?? <variant>` (`@C131`) |
| Struct-enum variant → parent enum  | Implicit on assignment | `p: Shape = Circle { r: 1.0 }` works without `as` |
| Struct-enum variant ← parent enum  | `as V` / `as V?` | `if s is V { s as V }` is checked; an unchecked `s as V` warns and a miss answers `V` with its fields defaulted, `s as V?` answers null (`@C131`) |
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

<!-- from tests/reference/json-values.loft -->
```loft
v = json_parse(`{{"users":[{{"name":"Alice"}}]}}`);
name = v.field("users").item(0).field("name").as_text();   // "Alice"
// every intermediate failure produces JNull, never a trap

match v {
  JObject { fields } => { kind = "object with {len(fields)} field"; },
  JArray { items }   => { kind = "array of {len(items)}"; },
  _                  => { kind = "other"; }
}
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

## Literals, closures, strings and formatting

Literals, closures, string literals and formatting: [LOFT_LITERALS.md](LOFT_LITERALS.md).

## Control flow and variables

`if`, loops, `break`/`continue`, `return`, custom iterators, parallel blocks, `match`, slice patterns, `is`, and variables: [LOFT_CONTROL.md](LOFT_CONTROL.md).

## Collections, structs and calls

Vectors, key-based collections, structs and record initialization, methods and call spellings, assertions, `sizeof`, random numbers: [LOFT_DATA.md](LOFT_DATA.md).

## Generics, interfaces, best practices and design decisions

Polymorphism, generic functions and structs, best practices, interfaces and bounded generics, design decisions and constraints: [LOFT_DESIGN.md](LOFT_DESIGN.md).


## See also
- [STDLIB.md](STDLIB.md) — Standard library API (math, text, collections, file I/O, logging, parallel)
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
