
# Loft Language Reference

Loft is a statically-typed, imperative scripting language with null safety and built-in parallel execution.
Source files use the `.loft` extension. The language compiles to an internal bytecode representation
and can emit Rust code for host integration.

**Quick reference with common patterns and gotchas:** see the loft-write skill (`.claude/skills/loft-write/SKILL.md`).
This page states the language as it stands; behaviour the language has since dropped is
recorded in [LOFT-history.md](LOFT-history.md), not here.

---

## Contents
- [Naming Conventions (enforced by the parser)](#naming-conventions-enforced-by-the-parser)
- [Types](#types)
- [Scripts (a file with no `fn main`)](#scripts-a-file-with-no-fn-main)
- [Declarations, files and grammar](#declarations-files-and-grammar) → [LOFT_DECLARATIONS.md](LOFT_DECLARATIONS.md)
- [Operators](#operators) → [LOFT_OPERATORS.md](LOFT_OPERATORS.md)
- [Literals, closures, strings and formatting](#literals-closures-strings-and-formatting) → [LOFT_LITERALS.md](LOFT_LITERALS.md)
- [Control flow and variables](#control-flow-and-variables) → [LOFT_CONTROL.md](LOFT_CONTROL.md)
- [Collections, structs and calls](#collections-structs-and-calls) → [LOFT_DATA.md](LOFT_DATA.md)
- [Generics, interfaces, best practices and design decisions](#generics-interfaces-best-practices-and-design-decisions) → [LOFT_DESIGN.md](LOFT_DESIGN.md)
- [Limits](#limits)

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

A file that starts with `#width 32` (or every file but the standard library under
`--width 32`) reads `integer` as `i32` and `float` as `single`, keeps arithmetic on 32-bit
values 32-bit, and spells the 64-bit forms `i64` / `123l` and `f64` / `2.5d`:
[WIDTH32.md](WIDTH32.md).
| `character` | A single Unicode character                       |
| `text`      | A UTF-8 string; `len()` counts characters, `size()` bytes |

A type is **non-null by default**: a plain `integer` / `text` / `Row` is not a slot for
`null`, and the compiler says so wherever one is written — discharge it (`??`, `?`,
`match`) or declare the slot `integer?`. Add `?` to make it nullable: `integer?` holds a
value or `null`. (The old `not null` modifier is now the default and is **deprecated** —
it parses as a no-op and draws `advice[not-null-deprecated]`; delete it. See "Struct types" below.)

How loudly depends on whether the slot has room for a null. A **local**, a **field**, a
**return**, a **vector element** and a **call argument** get a warning and the store
proceeds, because the slot does reserve a null it can hold and read back — with one
exception: a narrow width (`u8`…`u32`) spends its whole range on real values, so a null
there is an error. This applies to every type, records and collections alongside the
scalars ([formal/types.md](formal/types.md) `(N-Store)`).

**Non-null is a rule about writes, not a guarantee about reads.** A *fault* can still
leave the reserved pattern in a non-null slot: an integer overflow writes the sentinel
and the slot then reads `null`, and a non-null `float` can hold a `NaN` the same way
(the deliberate, bounded edge in
[DESIGN_DECISIONS @C85](DESIGN_DECISIONS_VALUES.md#c85--overflow-arithmetic-types-non-null-the-game-keeps-running-dont-force-integer-on-every--)).
A width with no code to spare — `u8`, `i8`, `u16`, `i16` — cannot hold the pattern at
all, so the same fault leaves the type's default there instead. So `x == null` on a
non-null slot is not dead code.

#### Null representation

Loft uses in-band sentinel values to represent `null`. Each type has a dedicated sentinel:

| Type | Null sentinel | Notes |
|------|---------------|-------|
| `boolean` | byte `255` (@PLN17) | Three-state: `false`=0, `true`=1, `null`=255 — stored like a 2-variant plain enum.  `null` is held and distinguished everywhere a boolean lives (locals, params, returns, fields, vector/keyed elements); test it with `b == null`.  `null` coerces to `false` only in boolean *logic* (`if`/`while`/`!`/`&&`/`\|\|`); `==`/`!=` are raw, so `null == false` is `false`.  A plain `boolean` is non-null: storing `null` into one warns (`(N-Store)`); declare `boolean?` for the third state.  `!b` is true for both `null` and `false` (both coerce to false). |
| `integer` | `i64::MIN` | 8-byte storage; the sentinel sits at the far edge of the i64 range. |
| `float` | `NaN` | IEEE 754: `NaN != NaN`, but `!f` correctly detects null |
| `single` | `NaN` (32-bit) | Same as `float` |
| `character` | `'\0'` (NUL) | The null character is not a valid loft character value |
| `text` | internal null pointer | Opaque; `!t` and `t == null` detect it.  `len` / `size` take a non-null `text`, so passing a `text?` warns — discharge it first (`len(t?)`) |
| `reference` | record 0 | Opaque; `!r` detects it |
| plain `enum` | byte `255` **or** byte `0` | Limits plain enums to 255 variants.  **Two bytes mean absent, and every test accepts both:** an explicit `null` writes `255`, while zero-initialised storage and a read of an absent record produce `0`.  Variants are numbered from 1, so `0` is a variant of no enum — which is why the renderer has always shown both as `null`.  A `??` that recognised only `255` let `v[9] ?? d` and a field never set answer `null` through a coalesce. |
| narrow int (`u8`/`u16`/`i8`/`i16`) | top of the packed range | e.g. `i8::MIN` for `i8`; stored compactly in `Parts::Byte`/`Short` |
| `i32` / `integer size(4)` | `i32::MIN` | 4-byte storage via `Parts::Int`; widens to i64 on the stack |

**Arithmetic safety — the spreadsheet model (@C80, [formal/operational.md](formal/operational.md)):**
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

**Binary file I/O caveat:** `f += <integer_expression>` on a
`BigEndian` / `LittleEndian` file writes **8 bytes**, because `integer` is 64-bit.
A binary format with narrower fields needs an explicit width cast —
`f += 2 as i32;` for a 4-byte field, `f += 0 as u8;` for a byte, `f += v as u16;`
for a 2-byte field.

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
[DESIGN_DECISIONS.md § @C69](DESIGN_DECISIONS_VALUES.md#c69--x-on-a-non-boolean-is-a-null-test-not-logical-not).
The compiler warns (`redundant-null-negation`) when `!` is applied to a value whose type
keeps no code for null — a non-null `u8` / `i8` / `u16` / `i16` / `u32` / `limit` slot —
since `!x` there is always false; the one exception, directly after a store, is below.

Integer ranges can be constrained with `limit`:
```
integer limit(-128, 127)   // fits in a byte
integer limit(0, 65535)    // fits in a short
```

A `limit` type is a narrow integer exactly as `u8` is: a value the compiler cannot prove in
range is refused at the store, the call and the literal, and the refusal names the cure —
write what the value becomes when it does not fit, `x ?? 0`; make the slot nullable,
`integer limit(0, 7)?`, which takes the value as a checked cast and holds `null` when it
does not fit; or mask it, `x & 7`.  An `if` range check is not a cure: loft does not narrow
a value's type after a test.  The one way an
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
round-trips, at every width.  A plain (non-null) `u8` / `u16` field has no
null to encode and uses the full range (256 / 65 536); `u32` keeps its top
code out of range either way ([formal/types.md](formal/types.md) `(N-Reserve)`).  Typical `u32` use:
RGBA pixels, large file offsets, bitmasks wider than i32.

**A NON-nullable narrow slot has no code for a failure, so the store says it.**  `u8`,
`i8`, `u16`, `i16`, `u32` and every `limit(lo, hi)` range use every code they have, so a
value that does not fit takes the type's default and is indistinguishable from a computed
one — `x: u8 = 250; x += 10` answers `0`, and so does `x: u8 = 250; x -= 250`.  An author
who cares about that edge gives the store an `else` (see *A store that did not take*
below):

<!-- from tests/reference/store-else.loft -->
```loft
health: u8 = 250;
health += 10 else { note += "the boost did not fit"; };
```

The older spelling still works: `!health` on the `if` that is the very next statement
answers whether the store fit.  The value is unchanged either way — the slot still takes
`0`, exactly as it does with nothing written.  Two rules bound the `if !` form:

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

### A store that did not take

A write lands nowhere when its place is not there — an index past the end, a key the
collection does not hold, a field reached through a null — and the program continues
(the spreadsheet model).  An assignment statement may end in `else { … }`, and that block
runs exactly when the write did not land; the place is spelled once:

<!-- from tests/reference/store-else.loft -->
```loft
scores[7] = 70 else { missed += 1 };
board.by_name["ann"].points += 1 else { missed += 1 };
board.by_name["bob"].points += 1 else { missed += 1 };
```

The same block runs when a narrow slot cannot hold the value (above) and when the store is
locked (`#lock`), and never when the write landed — a stored `null` landed.  The block may
`break`, `continue` or `return` like any other.  A dropped write with no `else` logs one
warning line (`index_out_of_bounds` for an index, `write_dropped` for a key or a null view)
and the program goes on; with an `else`, the block is the whole answer and nothing is
logged.  A store is never a condition: `if v[1] = 2` is refused — write `==`, or put the
store on its own line.

### Composite types

| Type syntax                        | Description                                           |
|------------------------------------|-------------------------------------------------------|
| `vector<T>`                        | Dynamic array of `T`                                  |
| `hash<T[field1, field2]>`          | Hash-indexed collection of `T` on the given fields    |
| `index<T[field1, -field2]>`        | B-tree index (ascending/descending)                   |
| `sorted<T[field]>`                 | Sorted vector on the given fields                     |
| `spatial<T[x,y]>` / `spatial<T[x,y,z]>` | Spatial keyed collection, 1–3 coordinate axes, Morton/Z-order radix tree |
| `trie<T[field]>`                   | Text-keyed collection on ONE field — exact lookup, key order, and prefix |
| `reference<T>`                     | Reference (pointer) to a stored `T` record; assigning re-points it; the record lives as long as the store of its holder — the compiler keeps it there, and refuses the few shapes it cannot keep (a holder that is an element of a parameter's collection, two parameters naming one record) |
| `reference<T>?`                    | The same pointer, allowed to be absent.  For a tree or a list that OWNS its nodes, a `T?` field is the child ([§ Recursive types](#recursive-types--a-child-or-a-link)) |
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
`loft-suite/attic/scripts/1267-a-descending-key-orders-the-index-once.loft` does the same for
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
*dispatch* rather than extraction ([@C89](DESIGN_DECISIONS_SYNTAX.md#c89--no-tuple-style-enum-variants-a-matcher-reads-like-grammar-and-is-never-forced)).  A field EVERY variant
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
  construction. It may also read the record through `$`, building from it as freely:
  `= "{$.id}: {$.label}"`, `= $.label.to_uppercase() + "!"`, `= [$.label]`,
  `= if $.price > 10 { Big } else { Small }`, `= describe($.label)`. Such a default is
  lowered into a function of the record and called with the record as it stands.
  The same default fills the field when a store written before the field existed is
  upgraded (`loft layout upgrade`, [DATABASE.md § the store upgrade](DATABASE.md#a-changed-definition-the-store-upgrade)).

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

Value-const covers every VIEW of the value: a write through a local bound to it (`q = t.r;
q.x = 5`), a loop variable over its elements or a `&` link is refused like a direct one. A
COPY out of it is the reader's own — `x = t.v` copies a vector and a function return is a
value, so writing `x[i]` leaves `t.v` unchanged ([formal/binding.md](formal/binding.md)
`(Const-Value)`).

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

Example with the modifiers together:
```
struct Point {
    r: integer limit(0, 255),
    g: integer limit(0, 255) = 128,
    b: integer limit(0, 255) assert($.b != 7),
    grey: integer computed(($.r + $.g + $.b) / 3)
}
```

### Recursive types — a child or a link

A field may hold its own type: a tree, an expression, a list.  The field holds a **child**, a
value like any other field's — no extra syntax, and the compiler stores it out of line
because it cannot be inline (@C159, [formal/layout.md](formal/layout.md) `(L-OutOfLine)`).
An enum's other variants end its recursion, so a variant field needs no `?`:

<!-- from tests/reference/recursive-types.loft -->
```loft
struct Tree {
  key: integer,
  left: Tree?,
  right: Tree?,
}

enum Expr {
  Num { v: integer },
  Add { l: Expr, r: Expr },
  Neg { e: Expr },
}
```

A child behaves as an inline field ([formal/binding.md](formal/binding.md) `(B-OutOfLine)`):
a read is a view, a write copies, a null write releases the subtree, and `==`, `{t}`, `:j`
and `Tree.parse` treat it as a nested value.  For one node in two places, use a **link**,
`reference<T>`, which shares:

<!-- from tests/reference/recursive-types.loft -->
```loft
t.left = Tree { key: 4 };   // copied into t's store
t.right = t.left;           // a copy: the two subtrees are independent
t.right.key = 9;
t.left = null;              // the old subtree is released
```

A struct whose recursion runs through non-null fields only has no finite value, and is
refused with `` `Bad.next: Bad` makes every Bad contain another Bad — make it `Bad?` ``
(`(T-Recursive)`).  A cycle through a tuple member stays refused: write `reference<T>` there.

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

Operators and precedence, operators on your own types, the null discharges (`??`, `x?`, `x!`), the narrowing null test, `as`, the conversion rules and parsing JSON: [LOFT_OPERATORS.md](LOFT_OPERATORS.md).

## Literals, closures, strings and formatting

Literals, closures, string literals and formatting: [LOFT_LITERALS.md](LOFT_LITERALS.md).

## Control flow and variables

`if`, loops, `break`/`continue`, `return`, custom iterators, parallel blocks, `match`, slice patterns, `is`, and variables: [LOFT_CONTROL.md](LOFT_CONTROL.md).

## Collections, structs and calls

Vectors, key-based collections, structs and record initialization, methods and call spellings, assertions, `sizeof`, random numbers: [LOFT_DATA.md](LOFT_DATA.md).

## Generics, interfaces, best practices and design decisions

Polymorphism, generic functions and structs, best practices, interfaces and bounded generics, design decisions and constraints: [LOFT_DESIGN.md](LOFT_DESIGN.md).

## Limits

One number bounds a program's structure: **300** (@C149).  At 300 everything works, on both backends;
the 301st is refused at compile time with a message naming the limit.

- an expression nested 300 levels deep — each bracket, call argument, block, branch, vector
  or struct element, chained operator (`a + b + c` is three levels at `c`) and postfix step
  (`a.b.c`, `x.f().g()`) is one level;
- 300 fields in a struct, 300 parameters to a function, 300 variables bound in a function
  (the compiler's own temporaries are not counted), 300 elements in a tuple.

The limits do not multiply: 300 variables each holding a 300-element tuple is refused by the
next limit, not guaranteed by these.  Collection sizes, the number of definitions, functions and
files are not limited.  Two technical caps stand apart:

- an enum holds at most 254 variants, so a variant is stored in one byte;
- one function's stack frame — its parameters, variables and the values its expressions hold
  while they run — holds at most 64 KiB.  Only large tuples reach it; a struct or a vector
  lives in a store and does not count.

A function at the count fits its frame whatever its variables hold, short of 300 tuples of 28
integers each, so the count is the limit a function meets.

Guards: `loft-suite/attic/scripts/a-nesting-within-the-bound-compiles.loft`,
`the-counts-within-the-limit-compile.loft` and their refused twins,
`the-variable-limit-arrives-before-the-frame-cap.loft`, `tests/limits.rs`.  The per-commit
copies prove each relation under LOWERED limits (`// @LIMIT:`, `// @FRAME_LIMIT:`;
`LOFT_LIMIT` and `LOFT_FRAME_LIMIT` for a `loft` run), which can only lower them; the same
files at 300 and 64 KiB are `tests/limits_full/`, run nightly.

## See also
- [STDLIB.md](STDLIB.md) — Standard library API (math, text, collections, file I/O, logging, parallel)
- [COMPILER.md](COMPILER.md) — Lexer, parser, two-pass design, IR, type system, scope analysis, bytecode
