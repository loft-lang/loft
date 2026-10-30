# Loft Operators and Conversions

Operators and their precedence, operators on your own types, the null discharges (`??`, `x?`,
`x!`), the null test that narrows, `as` and the conversion rules, and parsing JSON into a
struct.  Part of the language reference: [LOFT.md](LOFT.md).

---

Listed by precedence, loosest first — each level binds **tighter** than the one above
it, so a lower level groups last (outermost) and a higher level groups first (innermost):

| Precedence | Operators                              | Notes                   |
|------------|----------------------------------------|-------------------------|
| 0 (loosest)| `??`, `?? return`, `?? break`, `?? continue` | null-coalescing / early exit (C56) |
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

**`==` compares what a value holds, for every type (@C91).** Two structs are equal when their
fields are, a vector when its elements are (in order), a `hash` / `sorted` / `index` / `spatial` /
`trie` when it holds the same records whatever order they were inserted in, a struct-enum value
when the variant and its fields are; a `reference<T>` field is followed to the record it names,
and a cyclic value terminates.  `b = a` copies (@C86), so `a == b` is `true` afterwards.  A type's
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
*"No matching operator '<' on 'boolean' and 'boolean'"*, because a tuple never invents an
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
**null**, never a fault (@C80) — discharge it with `?? default`.

## Operators on your own type — `operator compare`, `plus`, `minus`, `times`, …

A type gets `<`, `<=`, `>` and `>=` from one method written with `operator` in place of `fn`:

<!-- from tests/reference/operator-compare.loft -->
```loft
value struct Date { ms: integer }
pub operator compare(self: Date, other: Date) -> Ordering { self.ms.compare(other.ms) }

fn test_a_date_is_ordered_by_its_operator_compare() {
  d1 = Date { ms: 1 };
  d2 = Date { ms: 2 };
  assert(d1 < d2, "d1.compare(d2) == Less");
  assert(!(d1 >= d2), "d1.compare(d2) != Less");
  dates = [d2, d1];
  dates.sort();                // and every generic bounded by `Ordered`
  assert(dates[0] == d1, "sorted by compare");
}
```

`operator plus`, `minus`, `times`, `divided_by` and `remainder` give `+`, `-`, `*`, `/` and `%`
the same way, `operator negate` the unary `-x`, and `a += b` is `a = a.plus(b)`.  The LEFT operand's type decides, chosen by the right operand's
type among its definitions: `d - span` and `d - other_date` may be two `minus`, and `3 * w` is
refused where `w * 3` is not.

`x as T` calls `operator to_t(self: S) -> T`, the target's name in snake case
(`operator to_date_time(self: text) -> DateTime` parses a date text `as DateTime`); a conversion
INTO your type may take a foreign `self`.  `operator to_text` and `operator next` give `"{x}"`
and `for e in x` the same way.  `==` and `!=` compare the fields of every type, always — no type
redefines them, and a comparison of a type's own is a named method; `!x` is the presence test
on every type.
`[…]` belongs to the built-in collections (a type reads through a named method, `x.at(i)`), and
`**` and the bit operators a type cannot define yet.  An interface names the same methods:
`interface Ordered { operator compare(self: Self, other: Self) -> Ordering }`.

Each form is one call of the LEFT operand's method, operands in the order they are written.
A method declared with plain `fn` is an ordinary method, and the operator on that type is
refused saying so; a function NAMED for an operator (`fn OpLt`, `fn OpAdd`) is ordinary too.  `x.compare(y)` and `Ordering.then` are in the stdlib for every built-in ordered type,
so a field-by-field order is `self.a.compare(o.a).then(self.b.compare(o.b))`.  The rules:
[formal/operators.md](formal/operators.md).

## The `??` operator (null-coalescing)

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
`τ?`-typed expression — keeps the result `τ?`, so `y: integer = x ?? null` warns that a null
reaches the non-null slot (`(N-Store)`). A chain discharges to non-null iff its *last* fallback is
non-null: `x ?? (a / b) ?? 7` is `integer` (ends in `7`), while `x ?? null` is `integer?`. Discharge
into a non-null slot with a real default (`x ?? 0`), or keep the slot `τ?`.

**Note:** For complex LHS expressions (function calls, field chains), the compiler automatically
materialises the result into a temporary variable so the expression is evaluated exactly once.
Simple variable reads skip the temporary since they have no side effects.

## A null test that leaves proves the value is there

After a test that stops the code when a value is absent, the value reads as present for the
rest of the block, with no `??` needed (`formal/types.md` `(N-Guard)`):

<!-- from tests/reference/null-guard.loft -->
```loft
fn shout(s: text) -> text {
  "{s}!"
}

fn describe(s: text?) -> text {
  if s == null { return "nobody"; }   // the absent case has left
  shout(s)                            // s reads as text here
}

fn greet(name: text?) -> text {
  assert(name != null, "a name");     // a failed assert stops the program
  shout(name)                         // name reads as text here
}
```

The test must be about one name (`v != null`, `if v`, or `v == null` on its failing side), and the
proof lasts until the block ends or the name is assigned again.  The same tests also prove an
index in bounds (`assert(i < len(v))` before `v[i]`), a divisor non-zero and a square root's
operand non-negative.

## The `x?` operator (default-fallback) — @PLN116

Postfix `x?` discharges a nullable `x: T?` to `T` by falling back to `T`'s **default** when
`x` is null — pure sugar for `x ?? construct_default(T)`. Where `??` supplies *the default you
give*, `?` supplies *the default the type gives*; that pairing is the mnemonic (`??` = my
default, `?` = the type's). It relieves the `?? 0` / `?? ""` / `?? 0.0` boilerplate that loft's
own null-flow manufactures (DN3 makes float ops yield `float?`; index/map/field reads are
nullable by the @C80 model).

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
static well-definedness check, fully consistent with "no *runtime* errors ever" — @C80):

- a **bare reference / non-null `DbRef`**; and
- a **record with a bare (non-`Optional`) enum field that has no `= expr`**. An enum's 0 *is* its
  null value (variants are 1-based), so a non-null enum field may not silently zero-fill to it —
  and choosing a variant as a record's default is a real decision the author must make. Fix it by
  providing the field, giving it `= <variant>`, or typing it `E?` (which then defaults to `null`).
  So a *bare* enum discharges to its first variant, but an enum *field inside a record* needs an
  explicit choice before the record itself can default.

`x?` on a value whose type cannot hold null (a non-null `u8` field, say) is an identity plus
`warning[redundant-default-fallback]` (mirrors `redundant-coalesce` for `??`).

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
calls `idx()` once, as [@C92](DESIGN_DECISIONS.md) requires of every compound assignment.
Rule: `@FR-E-Asgn-Discharge`, [formal/operational.md](formal/operational.md).

**`e!` takes no default:** the value when present, a halt when absent, its message quoting
the operand (@C160, `(N-Present)`). For a read an invariant guarantees — a table entry by its
own index — where any default would hide a corrupt input. Written against the operand (`x!`;
`!x` with a space is a negation), and it reaches a reference or a bare enum, which `?` cannot:

<!-- from tests/reference/null-operators.loft -->
```loft
p = rows[i]!;               // present, or a halt: "rows[i] is null"
```

## The `as` operator

Used for explicit type casts and conversions:
```
3.14 as integer          // numeric cast: float to integer (truncates toward zero) → integer
"json-text" as Program   // deserialize text as a struct
```

**`as` has two different jobs — a numeric cast vs. a fallible parse.** When the left
operand is a *number*, `as` reinterprets it (truncate/narrow/widen). When the left
operand is **text**, `as integer` / `as float` / `as single` is a **parse that can
fail**, so a bare cast — which asserts it cannot — is a compile error
(`error[text-parse-may-fail]`). Say what a bad parse becomes:
```
n = s as integer             // COMPILE ERROR — a parse cannot be asserted
n = s as integer ?? 0        // n : integer    (a bad parse gives the default)
n = s as integer?            // n : integer?   (null on a bad parse, discharge later)
```
(`(N-Cast)` in [formal/types.md](formal/types.md) — a bad parse is a reachable fault like `÷0`/OOB.)

## Type-conversion rules — when does loft convert automatically?

Loft applies conversions in three modes: **implicit** (no annotation),
**format-only** (implicit, but only inside `"{…}"` interpolation), and
**explicit** (`as` required).  The mode depends on the types involved,
not on the context — which means you can predict what a conversion will
do by looking up the pair in this table:

| From → To                          | Mode          | Notes |
|------------------------------------|---------------|-------|
| Any type → `boolean` (in `if`, `!v`, `while`, `assert`) | Implicit | `false` and null are falsy (an `integer` holding its null, `i64::MIN`, is falsy); every other value is truthy.  See [LOFT.md § Null representation](LOFT.md#null-representation) for the null-sentinel table.  **These four POSITIONS are the whole of it** — a `vector` passed where a `boolean` PARAMETER is declared stays an error, because there the coercion would hide a mistake rather than express one.  An EMPTY collection and a payload-less enum variant are values, so both are truthy; only null is falsy. |
| Integer ↔ `float` in arithmetic    | Implicit      | `3 + 1.5` is `4.5` — the integer widens to the float operand's width |
| Integer / `single` → `float`       | Implicit      | widening; `single` (32-bit) widens to `float` (64-bit) with no loss |
| Integer → `single`                 | Implicit      | `[1, 2]` is a valid `vector<single>` |
| `float` → `single`                 | Explicit `as` | NARROWING (64→32-bit loses precision).  A bare decimal literal is `float`; write a **`single` literal** with the `f` suffix (`1.0f`) or cast (`x as single`).  This is enforced element-wise: a `vector<single>` literal must be `[1.0f, 2.0f]` or `[a as single, …]` — `[1.0, 2.0]` (float literals) is a compile error ("would lose precision"), never a silent truncation |
| `i32` / narrow int → `integer`     | Implicit      | widening; a 4-byte `i32` (or `u8`/`u16`/`i8`/`i16`) widens into the 8-byte `integer` with no loss |
| `integer` → `u8`/`u16`/`i8`/`i16`/`u32`/`i32` | `?? d`, a `τ?` destination, or a mask | NARROWING — a plain `integer` is 64-bit; writing one into a narrow **struct field**, local, parameter or return (any narrow storage) that the compiler cannot prove fits is refused, and so is a bare `as` there: "narrowing cast from integer to u16 may not fit at runtime; use `u16?` for a checked cast (value or null), give it a fallback with `as u16 ?? <value>`, or mask it first, `(<value> & 65535) as u16`".  A **constant that provably fits** the target is exempt (`x: u16 = 5`, `f(200)`).  The check is range containment **or a drop in storage width**, so it covers every narrow alias — including `i32`, which the range half cannot see (see the row below).  (guard `loft-suite/attic/scripts/931-i32-narrowing-is-checked.loft`) |
| `integer` → `i32` — why it needs the width half | Explicit `as` | `i32` spans the whole 32-bit range, which is the range a plain `integer` *reports*: the 64-bit value lives in an 8-byte slot the bounds do not describe.  So the two specs differ in `forced_size` alone, and `[s.min,s.max] ⊆ [d.min,d.max]` holds for a pair whose storage drops 8 → 4 — range containment alone would let an `i32` field take `5000000000` and store `705032704`. <!-- doc-lint: ok --> The **implicit** stores therefore compare storage width; an **explicit** `as i32` is refused the same way (`i32?`, `as i32 ?? <value>` or a mask are the cures) |
| `float` → integer                  | Explicit `as` | `pi as integer` truncates toward zero; preserves the current sentinel semantics |
| `text` → integer / float / single  | Explicit `as ?? d` or `as τ?` — **a PARSE** | `s as integer` is a fallible parse, so the bare cast is refused (`text-parse-may-fail`). Say what a bad parse becomes: `s as integer ?? 0` (a default) or `s as integer?` (`integer?`, null on a bad parse). This is `(N-Cast)` — a bad parse is a reachable fault, exactly like `÷0` and out-of-bounds indexing. Contrast the *numeric* casts above (`float`→`integer`, width narrowing), which reinterpret an existing number rather than parse text |
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
  value).  The mask proves the range where it meets the store, so
  `b: u8 = x & 0xFF` needs no `as` — the idiom for byte packing, RGBA, and
  hashing.  The proof belongs to the expression, not to a variable:
  `m = x & 0xFF` makes `m` an `integer`, and `b: u8 = m` is refused.  Mask
  at the store, or declare `m: u8 = x & 0xFF`.
- **Constrain the source type** — declare it `integer limit(0, 255)` (or
  `u8`).  A value that provably fits narrows implicitly, no cast.

Loft does **not** refine a value's range from an `if` guard — inside
`if x <= 255 { … }`, `x` is still a full `integer`.  Use one of the forms
above.

## Parsing (JSON → JsonValue tree → struct)

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

if json_errors() != "" { log_warn(json_errors()); }   // the JSON surface: one text
for e in user#errors { log_warn(e); }                 // the record surface: a vector<text>
```

`x#errors` is a `vector<text>` with one entry per value that did not fit in the parse that
built the value `x` holds — every mismatch, in both parse forms — empty after a parse that
succeeded: test it with `len(x#errors) > 0`.  A value that did not fit leaves its field at
the type's default; the values around it are read.  Reading it
changes nothing, and each of `user`, `scores` and `shape` above answers for its own parse
(@C148).  The errors go where the WHOLE value goes — bound, rebound, passed, returned from a
function, copied to another variable.  A value built any other way has none, and neither has a
part copied out of a parsed value or a parsed value stored into a field or an element.  Ask a
variable: `f()#errors` does not parse, and a variable that is neither a record nor a vector is
refused.  The one-step `Type.parse(text)`, the `text as T` cast and the staged
`Type.parse(json_parse(text))` all fill it; `json_errors()` carries the same mismatches as one
text.  A field with no default cannot be left out: a record whose bare enum field declares no
`= <variant>` is refused by `text as T` and `T.parse(…)` (`text-parse-may-leave-a-field-unfilled`)
— except a `JsonValue` field, which takes `JNull` (`@FR-D-Json`, STDLIB.md § JsonValue surface),
and the checked `text as T?` answers null when the text leaves that field out, sets it to
`null`, or names a variant the enum does not have.  Guards:
`loft-suite/rules/types/D-NoEnumF/a-checked-parse-answers-null-for-an-enum-field-it-cannot-fill.loft`,
`loft-suite/attic/scripts/a-record-errors-read-answers-for-its-own-parse.loft`,
`loft-suite/attic/scripts/a-vector-errors-read-answers-for-its-own-parse.loft`.

---
