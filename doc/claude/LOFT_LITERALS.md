# Loft Literals, Closures, Strings and Formatting

Literals, closures, string literals and string formatting.  Part of the language reference: [LOFT.md](LOFT.md).

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
| Function ref     | `double_score` (the bare name)      |
| Lambda (long)    | `fn(x: integer) -> integer { x * 2 }` |
| Lambda (short)   | `\|x\| { x * 2 }`                    |

A **function reference** — a function's bare name used as a value (`f = double_score`) — produces a `Type::Function` value whose runtime representation is the definition number of the named function.  The compiler resolves the name at **compile time** and errors if it does not exist or is not a function.  The value is 4 bytes (same as `integer`).

**Calling a fn-ref variable:** a variable or parameter of type `fn(T) -> R` can be called directly:

<!-- from tests/reference/fn-refs.loft -->
```loft
f = double_score;             // type: fn(const Score) -> integer
x = f(some_score);            // calls double_score via f
```

**`fn(T) -> R` as a parameter type:**

<!-- from tests/reference/fn-refs.loft -->
```loft
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
```

and the bare name is what a parameter of `fn` type takes (`fn double_it` is refused):

<!-- from tests/reference/fn-refs.loft -->
```loft
result = apply(double_it, 5);  // `fn double_it` is refused: use the bare name
```

A parameter of a function type may be `const` — `fn(const Score) -> integer` — which says the
functions the reference holds only read it, so a `const` value may be passed through it.  A
function whose parameter is plain is refused where `fn(const T)` is expected; one whose parameter
is `const` fits a plain `fn(T)` slot as well.

**Lambda expressions** produce an inline anonymous function at the expression level.
Two syntactic forms are available:

```grammar
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
signature (guard `tests/scripts/1067-lambda-expected-type.loft`):

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

Its primary use is with the higher-order functions `map`, `filter`, and `reduce`, as well as the `par(...)` for-loop clause.  A function is passed by its bare name; the older `fn double` spelling is refused with the cure named:

<!-- from tests/reference/fn-refs.loft -->
```loft
// Named fn-ref
fn double(x: integer) -> integer { x * 2 }
fn is_pos(x: integer) -> boolean { x > 0 }
fn add(a: integer, b: integer) -> integer { a + b }
```

A bare name and a lambda are interchangeable wherever a function is expected, `par` included:

<!-- from tests/reference/fn-refs.loft -->
```loft
doubled  = map(nums, double);           // [2, -4, 6]
positive = filter(nums, is_pos);        // only positive elements
total    = reduce(nums, 0, add);        // sum
assert("{doubled}" == "[2,-4,6]" and "{positive}" == "[1,3]" and total == 2, "by name");

// Equivalent using lambdas (short form, types inferred)
doubled  = map(nums, |x| { x * 2 });
positive = filter(nums, |x| { x > 0 });
total    = reduce(nums, 0, |a, b| { a + b });
assert("{doubled}" == "[2,-4,6]" and "{positive}" == "[1,3]" and total == 2, "by lambda");

for a in items par(b=double(a), 4) { results += [b] }
```

**`map(v, fn f) -> vector<U>`** — applies `f` to every element; returns a new vector of the return type of `f`.

**`filter(v, fn pred) -> vector<T>`** — returns a new vector with only elements for which `pred` returns `true`.

**`reduce(v, init, fn f) -> U`** — left-folds: starts from `init`, applies `f(acc, elm)` for each element in order.

### Closures

A lambda that references variables from the enclosing scope is a **closure**.
The captured values are **copied into the closure record at definition time**
(value semantics, like Rust `move` closures).

<!-- from tests/reference/closures.loft -->
```loft
greeting = "Hello";
greet = fn(name: text) -> text { "{greeting}, {name}!" };
greeting = "Bye";       // does NOT affect the closure
assert(greet("world") == "Hello, world!", "captured at definition time");
```

**Cross-scope closures** — a function can return a closure to its caller.
The captured values travel with the lambda:

<!-- from tests/reference/closures.loft -->
```loft
fn make_adder(n: integer) -> fn(integer) -> integer {
    fn(x: integer) -> integer { n + x }
}
```

The returned closure keeps `n` after `make_adder` has returned:

<!-- from tests/reference/closures.loft -->
```loft
add5 = make_adder(5);
assert(add5(10) == 15, "the captured `n` survives the return");
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
  captures).  A capture of a `const` value, or of a view of one, is read-only
  inside the closure as well (loft#1540).
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
- A `&` parameter IS capturable, and what the closure captures is the POINTEE: a `&S` or
  `&vector<τ>` is shared (the same DbRef), a `&integer` / `&text` is copied at creation
  like any scalar.  The one refusal is a WRITE to a captured `&` scalar — the copy could
  not reach the caller — and it is named, with the cure (`local = p; …; p = local;`).
  Guard `tests/scripts/1276-a-closure-captures-a-ref-parameter.loft`.

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

Rules: [formal/formatting.md](formal/formatting.md) `(F-Interp)`, `(F-Escape)`.

### Backtick strings (`` `...` ``)

**Multi-line.** Bare `"` is literal inside backtick strings (no escaping needed).
Auto-strips leading indentation: the **first content line** sets the base, and that many
leading spaces come off every line that has them.  A blank line does not set the base,
a line indented less than the base comes out flush, and a TAB-indented line is left
alone (a tab is not a space, so there is nothing to count).  The first and last lines
are dropped when they contain only whitespace.

Interpolation is no exception — a block with `{…}` in it dedents exactly like one
without (guard `tests/scripts/990-backtick-dedent-with-holes.loft`).

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

<!-- from tests/reference/interpolation-values.loft -->
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

<!-- from tests/reference/interpolation-values.loft -->
```loft
name = "ada";
q: Query = "SELECT * FROM t WHERE name = {name}";
// calls q.lit("SELECT * FROM t WHERE name = ") then q.hole_text(name)
```

The target comes from the type you assign to, a struct field you initialise, a
function parameter, or a return type — there is no new syntax, and `text` behaves
exactly as before. So a builder function needs no local to route through:

<!-- from tests/reference/interpolation-values.loft -->
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

<!-- from tests/reference/interpolation-values.loft -->
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
