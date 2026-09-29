# Loft Control Flow and Variables

Conditionals, loops, `break`/`continue`, `return`, custom iterators, parallel blocks, `match`, slice patterns, `is`, and variables.  Part of the language reference: [LOFT.md](LOFT.md).

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

Rules: [formal/operational.md](formal/operational.md) `(E-IfT)`; the condition is a truthiness position, `(E-Truthy)`.

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
collection is one member of a [linked collection group](DATABASE_INDEXES.md#removing-one-entry-with-eremove-loft903)
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

<!-- from tests/comparisons/loops.loft -->
```loft
while !done() { step(); }  // while works

while true {           // infinite until a break
    if should_exit() { break; }
    step();
}
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
must be a real loop variable: an ordinary local is refused with a message naming the
loop variables that CAN be written (`n` is not a loop variable … write a plain `break`, or
`i#break`); guard `tests/parse_errors.rs`.

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

Rules: [formal/calls.md](formal/calls.md) `(F-Return)`, `(F-Ret)` (the value returned is a fresh, independent value); `?? return` is `(E-Coalesce)` with a return in the fallback.

### Custom iterators (I13)

Any struct or struct-enum with a `fn next(self: T) -> Item?` method can be used in a `for`
loop.  Returning `null` from `next` terminates the loop:

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

`#count` and `#first` work.  `#index` is refused at compile time: it is the position you give
to `v[i]` to read the same element again, and an iterator's values have no such position —
`#count` numbers them.  `#remove` is not available either.

The loop walks a COPY of the iterator value, as any bind of a struct copies: after
`for x in c { }` the caller's `c` is where it was, and `c.next()` starts from there.

A program may also define `fn exhausted(self: T) -> boolean` for its own iterator type.  The
built-in `exhausted(gen)` answers only for a generator (`iterator<T>`, nullable included); for
any other type the call reaches the program's definition, or is refused when there is none.

Rules: [formal/iteration.md](formal/iteration.md) `(I-Next)`, `(I-Done)` — `next` is asked once per step and `null` is the end.

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

Both backends run the arms concurrently (guard `tests/scripts/1054-parallel-block-arms-run.loft`).

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

`Rect { width: w, height }` binds `width` as `w` — a bare lowercase name after a field is a
binding under that name, even when a variable of that name is in scope (`@FR-P-Point`).  A
literal, a range or `_` in that position tests the field instead (`Circle { radius: 0 }`,
`Circle { radius: 1..4 }`).

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
| `xs:integer*` | a plain scalar element type: every element matches, so this is a typed `..xs` — what follows it is read from the end, `[xs:integer*, last]` |
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

An element written **after** a `..` takes the same forms as one before it — a name, `_`, a
literal or a variant pattern (`[Kw { word }, .., End { e }]` binds `e` from the last element;
guard `tests/scripts/1419-a-fixed-pattern-after-a-rest-is-a-tail-element.loft`).
**One limit worth knowing.** A multi-pattern arm (`A { r }, B { r } => …`) is for enum
variants only — it does not accept slice patterns.  A name only some of its patterns bind
(`A { r }, B { s } => …`) is nullable in the body — `null` when a pattern without it matched
(guard `tests/scripts/a-name-only-some-listed-patterns-bind-is-nullable.loft`).

### `is` variant check

The `is` operator tests whether an enum value is a specific variant:

<!-- from tests/reference/is-check.loft -->
```loft
d: Dir = North;
if d is North { hit = true; }       // true
assert(!(d is South));       // negation
```

For struct-enums, `is` can also capture variant fields into local variables:

<!-- from tests/reference/is-check.loft -->
```loft
s = Circle { radius: 3.14 };
if s is Circle { radius } {
  area = PI * radius * radius;   // radius is in scope here
}
// radius is NOT in scope here
```

Multiple fields:
<!-- from tests/reference/is-check.loft -->
```loft
if shape is Rect { width, height } {
  area = width * height;
}
```

With else:
<!-- from tests/reference/is-check.loft -->
```loft
if shape is Circle { radius } {
  area = PI * radius * radius;
} else {
  area = 0.0;
}
```

In loops:
<!-- from tests/reference/is-check.loft -->
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
then `,` or `}`, it is a field capture; otherwise it is the if-body.  A capture binds each field
under its own name; to bind one under another name (`radius: r`), use `match` — `is Circle
{ radius: r }` is refused with that advice.

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

Rules: [formal/binding.md](formal/binding.md) `(B-Copy)` (a plain bind copies), `(B-View)` (a struct-typed projection is a view).
