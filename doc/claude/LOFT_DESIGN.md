# Loft Generics, Interfaces, Best Practices and Design Decisions

Polymorphism, generic functions and structs, best practices, interfaces and bounded generics, and the design decisions the language keeps.  Part of the language reference: [LOFT.md](LOFT.md).

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

shapes: vector<Shape> = [Shape::Circle { radius: 2.0 }, Shape::Rect { width: 3.0, height: 4.0 }];
for s in shapes { println("{s.area()}"); }   // each element runs its own variant's `area`
```

A call on a value typed as the ENUM must reach a body for every variant. When one has
none, the call is a compile error: *"call of `area` on `Shape` is not exhaustive — missing:
Tri"*. Two cures: write the missing `fn area(self: Tri)`, or write one fallback on the enum
itself, `fn area(self: Shape)`, which every variant without its own body reaches. A call on
a value the compiler knows to be one variant (`c = Circle { radius: 2.0 }; c.area()`) needs
only that variant's body.

An **empty-body stub** also covers a variant:

```
fn area(self: Rect) -> float { }   // covers Rect; answers the return type's default
```

A stub with an empty body `{ }` and a `self` parameter is treated as an intentional
no-op: it emits no warnings, is callable at runtime (it answers its return type's default —
`""` for `text`, `0` for `integer`), and suppresses the unused-`self` warning.

### Multiple dispatch — one name, a definition per parameter types

The same name may have several ordinary functions that differ in their parameter types.
The call picks the definition whose parameters fit its arguments:

```
fn width(x: integer) -> integer { x }
fn width(s: text) -> integer { len(s) }
width(7)        // 7
width("abcd")   // 4
```

The rules:

- Every parameter counts, not only the first, so `hit(f: Fireball, w: IceWall)` and
  `hit(f: Fireball, c: Crate)` are two definitions.
- The most specific definition wins: a variant beats its enum, so `hit(a: Entity, b: Entity)`
  answers every pair that nothing more specific takes.
- A value held as the enum is dispatched on the variant it holds at run time.  Such a call
  needs every candidate definition to return the same type; otherwise it is refused
  (*"`val(Entity)` is decided by the runtime variant, but its definitions return different
  types"*).
- Ambiguity is checked over every combination of variants a call can meet.  Two definitions
  that fit one combination equally well are a compile error that names the combination and
  both definitions.
- Two definitions with the same parameter types are refused (*"Cannot redefine"*).
- A short lambda with untyped parameters (`|v| { v + 1 }`) cannot be passed where the name
  has several definitions: nothing tells it which parameter type to take.  Write the typed
  form, `fn(v: integer) -> integer { v + 1 }`.

Catalogue: @F20 (variant-based dispatch), @F122 (one name, a definition per parameter combination).

---

## Generic functions

A single type variable `<T>` lets you write a function body once for any type:

```
fn identity<T>(x: T) -> T { x }
fn pick_second<T>(a: T, b: T) -> T { _x = a; b }
```

**Rules:**
- T must appear in a parameter (directly or as `vector<T>`, etc.) — any parameter; one it
  appears in nowhere is refused, since a call has nothing to infer it from.
- A header may declare several type variables, each with its own bounds —
  `fn pair_up<K: Printable, V: Printable>(k: K, v: V) -> text`; each is inferred from the
  parameters that name it, and each binding is checked against its own variable's bounds.
- At the call site, T is inferred from the arguments; two parameters naming T must receive
  one type (`same<T>(a: T, b: T)` called with an integer and a text is refused, naming both).
- A generic may share its name with other definitions — concrete ones, other generics, a
  method: a call reaches the most specific one that takes it (a concrete definition over a
  generic, `vector<T>` over `T`, `<T: A + B>` over `<T: A>`), and two nothing ranks are
  refused naming both.
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

### Generic structs

A struct may declare type variables, and `Box<integer>` names an **instance** — an ordinary
struct whose fields are the template's with every variable replaced, laid out and behaving
exactly as its hand-written twin `struct BoxInteger { v: integer }` would:

```
struct Box<T> { v: T }
b: Box<text> = Box { v: "hi" };   // the annotation names the instance
c = Box { v: 1 };                 // or the field values bind it: a Box<integer>
fn twice(b: Box<integer>) -> integer { b.v * 2 }
twice(c)                          // one type: the inferred instance is the named one
```

- A literal takes its instance from the type expected of it (a binding's annotation, a
  parameter, a return type) or else binds each variable from the field values whose declared
  types name it, as a call binds a generic function's.  A variable no value binds (`Box {}`,
  `Box { v: null }`, an empty `[]` for a `vector<T>` field) is refused naming the variable;
  one variable given two types by two fields is refused naming both fields.
- The bare template name is not a type: `b: Box` is refused — name the arguments, `Box<integer>`.
  The argument count must match the header's.
- `Box<integer>` and `Box<integer?>` are two instances, as are `Box<integer>` and `Box<u8>`.
- A type variable is a type only inside its own header.
- A generic function takes an instance (`fn get<T>(b: Box<T>) -> T { b.v }`), builds one
  (`fn wrap<T>(x: T) -> Box<T> { Box { v: x } }`) and binds `T` through it — and a variable
  no argument binds may be bound by a callback's result: `map_grid<T, U>(g: Grid<T>, f: fn(T)
  -> U) -> Grid<U>` called as `map_grid(tiles, |t| { t.height })` is a `Grid<integer>`; methods work as
  on any struct (`fn at<T>(self: Grid<T>, i: integer) -> T?`), and a concrete method on one
  instance beside the generic one takes that instance.
- A struct may name itself at its own variables through a collection or a pointer
  (`kids: vector<Tree<T>>`, `next: reference<Node<T>>?`); an inline `next: Node<T>?` is
  refused as its twin's is, and a mention at other arguments (`Bad<vector<T>>` inside
  `Bad<T>`) is refused at the declaration.  A generic struct may be named above its
  declaration, as any struct may.
- An `enum` may declare type variables too: `enum Shape<T> { Dot { at: T }, Empty }`.
  `Shape<integer>` is an enum with its own variants; a variant literal takes its instance
  from its payload or its annotation (a unit variant only from the annotation:
  `e: Shape<integer> = Empty`), `match` reads it, and `Shape<integer>::Dot` names one of the
  instance's variants as a type — a set over those dispatches on the variant.  Generic code
  takes one as it takes a generic struct: `fn get<T>(s: Slot<T>, d: T) -> T { match s {
  Full { v } => v, Hole => d } }` reads the payload, `fn wrap<T>(x: T) -> Slot<T> { Full {
  v: x } }` builds a variant, `is` tests one.
- A library's generic structs and enums cross `use` whole: a consumer instantiates them at the
  library's types and at its own (`Grid { cells: [Mine { n: 5 }], w: 1 }`), the library's
  generic functions, methods, field checks and defaults reach every instance, and the
  consumer's own generics take them.  `loft api-surface` lists the template (`Grid<T>`), not
  its instances; the debugger shows a local's instance beside a literal that names the
  template (`g: Grid<integer> = Grid{cells:[1,2],w:1}`).

Rules: [formal/interfaces.md](formal/interfaces.md) `(G-Gen)`, `(G-Mono)`, `(G-Check)`; catalogue: @F25.

Rule: [formal/interfaces.md](formal/interfaces.md) `(G-Type)`.


## Best Practices

### String comparisons containing `{` or `}`

All string literals in loft are format strings — any `{...}` is interpreted as a
format expression. When comparing formatted output against a string that contains
literal braces, escape both sides with `{{` and `}}`:

<!-- from tests/reference/format-braces-refused.loft -->
```loft
// WRONG — {r:128,g:0,b:64} opens an interpolation: `r` with a format spec, not text
assert("{p}" == "{r:128,g:0,b:64}", "...");
```

A doubled brace is a literal brace:

<!-- from tests/reference/format-braces.loft -->
```loft
// CORRECT — double braces produce literal { and }
assert("{p}" == "{{r:128,g:0,b:64}}", "...");
```

Similarly for JSON format output:
<!-- from tests/reference/format-braces.loft -->
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

<!-- from tests/reference/ref-params.loft -->
```loft
fn fill(v: &vector<Item>, extra: vector<Item>) {
    v += extra;          // appended elements are visible to the caller
}

fn add_one(v: &vector<Item>, x: Item) {
    v += [x];            // bracket-form also works
}
```

Field-level mutations via a ref-param also work as expected:

<!-- from tests/reference/ref-params.loft -->
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
the function REPLACES, not when it appends (guard
`tests/scripts/1251-a-heap-parameter-is-shared-not-copied.loft`).

### Polymorphic text methods on struct-enum variants

Text-returning methods on struct-enum variants that use format strings work
correctly:

<!-- from tests/reference/methods.loft -->
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

<!-- from tests/reference/methods.loft -->
```loft
fn name(self: Dot) -> text { }   // stub: answers the type's default (""), no warning
```

---

## Interfaces and bounded generics

Interfaces declare a set of required methods.  A type satisfies an interface
by defining the required methods — no `impl` declaration is needed (structural
satisfaction, like Go interfaces):

<!-- from tests/reference/interfaces.loft -->
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

<!-- from tests/reference/interfaces.loft -->
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

<!-- from tests/reference/interfaces.loft -->
```loft
interface Summable {
  op + (self: Self, other: Self) -> Self
}
fn total<T: Summable>(a: T, b: T) -> T { a + b }
```

and `integer` satisfies it automatically:

<!-- from tests/reference/interfaces.loft -->
```loft
assert(total(10, 20) == 30, "integer satisfies Summable automatically");
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

<!-- from tests/reference/interfaces-refused.loft -->
```loft
fn show<T>(v: T) -> text { "{v}" }             // refused, and says why
```

With the bound, the same body renders every kind:

<!-- from tests/reference/interfaces.loft -->
```loft
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

<!-- from tests/reference/error-handling.loft -->
```loft
name = config.get("user") ?? "anonymous";  // fallback
f = file("tests/reference/no-such-file.txt");
if !f.exists() { seen = "not found"; }             // guard
```

**`FileResult` enum** for filesystem operations that need specific error reasons:

<!-- from tests/reference/error-handling.loft -->
```loft
result = delete("tests/reference/never-created.dat");
if result == FileResult.NotFound { seen = "already gone"; }
if result == FileResult.PermissionDenied { seen = "access denied"; }
if !result.ok() { seen += " — delete failed"; }
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

```grammar
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

### Closure capture: the value at definition, the write shared

A captured scalar is taken by value when the closure is formed: a later write to the
outer variable is not seen inside (`(L-CapScalar)`, `formal/closures.md`).  A write the
CLOSURE makes to that scalar, though, lands in the outer variable, and the next call sees
what the previous one wrote (`(L-CapWrite)`); a captured struct or vector is shared in
both directions (`(L-CapHeap)`).  So a counter closed over is a counter the caller can
read:

<!-- from tests/reference/closures.loft -->
```loft
counter = 0;
inc = fn() -> integer { counter += 1; counter };
assert(inc() == 1, "1");
assert(inc() == 2, "2");
assert(inc() == 3, "3 — each call sees what the previous one wrote");
assert(counter == 3, "3 — the closure's write landed in the outer variable (L-CapWrite)");
```

### Variable scoping: shared name table per file

All functions in a `.loft` file share one variable name table.  In practice this
works transparently — the compiler tracks which function each variable belongs to,
so reusing the same parameter or local-variable name across functions (including
recursive functions with `const vector<T>` parameters and `for` loops) works
correctly.

**A local bound inside a block ends at that block's `}`** (`formal/binding.md` `(B-Scope)`,
as in Rust).  An `if` / `else` arm, a loop body, a `match` arm body and a bare `{ }` are
blocks; a local a statement binds there is gone after the `}`, and reading it is an error —
also when every arm binds it:

<!-- from tests/reference/scoping-refused-block.loft -->
```loft
fn f(c: boolean) -> integer {
  if c { w = 5; } else { w = 6; }
  w                        // error[local-out-of-scope]: `w` was bound inside a block that has ended
}
```

Either make the block's value the binding, or bind before the block:

<!-- from tests/reference/scoping.loft -->
```loft
fn g(c: boolean) -> integer {
  w = if c { 5 } else { 6 };   // the block's value is the binding
  w
}
fn h(c: boolean) -> integer {
  w = 0;                   // bound before the block, assigned inside
  if c { w = 5; }
  w
}
```

For several values, bind a tuple: `(qx, qw) = if c { (0.0, 1.0) } else { (a, b) };`.

The loop variable follows the same rule: it is bound by the `for` header, lives in the
loop's body, and is gone after the loop — `for i in 0..3 { } i` is `local-out-of-scope`, and so
is `i#index` after the loop.  A destructured `for (a, b) in …` binds both names the same way.
Carry a value out through a local bound before the loop:

<!-- from tests/reference/scoping.loft -->
```loft
fn last_of(v: vector<integer>) -> integer {
  last = 0;
  for x in v { last = x; }
  last                     // the last element, or 0 when `v` is empty
}
```

Naming a loop variable the same as an existing local of the function is still a
*compile-time error*, not a silent shadow:

<!-- from tests/reference/scoping-refused-shadow.loft -->
```loft
fn f() {
  x = 0;
  for x in 0..3 { }   // error: loop variable 'x' shadows a local named 'x'
}                      //        — rename the loop variable (e.g. loop_x)
```

Rename the loop variable (the message suggests `loop_x`) or drop the dead outer
local.

Two *loops* may share a name freely, at any element types — each `for` binds its
own variable, so nothing is carried from one to the next:

<!-- from tests/reference/scoping.loft -->
```loft
fn g2() -> text {
  out = "";
  for i in ["a", "b"] { out += i; }
  for i in 0..3 { out += "{i}"; }   // fine — a different variable
  out
}
```

Nested loops are the exception: `for i { for i { } }` is rejected, because the inner
binding would take over `i` for the rest of the outer body.

A local declared **inside** a loop body splits the same way, and for the same
reason — two adjacent loops doing different work want the same short name for the
same role:

<!-- from tests/reference/scoping.loft -->
```loft
fn h2(as1: vector<A>, bs: vector<B>) -> text {
  out = "";
  for x in as1 { e = x; out += "a={e.v} "; }
  for y in bs  { e = y; out += "b={e.w} "; }   // fine — a different variable
  out
}
```

A local that must outlive its loop — an accumulator — is bound before it:

<!-- from tests/reference/scoping.loft -->
```loft
total = 0;
for x in as1 { total = total + x.v; }
for z in cs  { total = total + z.c; }   // the same `total`, bound before both loops
```

### Hash collections: name the key (local or struct field)

A hash (and `sorted` / `index`) needs its key spelled out, because a bare `[]`
literal is ambiguous — it could be a vector or a keyed collection.  Give the key
either way and lookup, mutation, removal, and **iteration** all work, on both the
interpreter and `--native`:

<!-- from tests/reference/keyed-collections.loft -->
```loft
struct Entry { name: text, value: integer }
```
<!-- from tests/reference/keyed-collections.loft -->
```loft
struct Table { data: hash<Entry[name]> }
```

Either declaration supplies the key, and every keyed operation follows it:

<!-- from tests/reference/keyed-collections.loft -->
```loft
// As a local variable — the type annotation supplies the key:
h: hash<Entry[name]> = [];
h += [Entry { name: "x", value: 1 }];
e = h["x"];                 // lookup — works
h["x"] = null;              // remove — works
for kv in h { }             // iteration — works

// Equivalently, as a struct field (the field declaration supplies the key):
t = Table { data: [] };
t.data += [Entry { name: "y", value: 2 }];
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

<!-- from tests/reference/keyed-collections.loft -->
```loft
t.data = [Entry { name: "y", value: 2 }];  // REPLACES — the old contents are freed
replaced = len(t.data) == 1 and t.data["x"] == null and (t.data["y"].value ?? -1) == 2;
t.data = [];                               // empties it
emptied = len(t.data) == 0;
```

`= null` empties the collection rather than making it absent: a collection field holds
a record id / claim pointer where `0` already means *no records*, and nothing in that
encoding is left to mean *absent* rather than *empty*.  So `c == null` answers `false`
even straight after `c = null` — **test emptiness with `len(c) == 0`**
([loft#917](https://github.com/loft-lang/loft/issues/917) tracks the reader half).  The
`?` makes no difference here: only the SCALAR default flips to non-null, so `vector<T>`
and `vector<T>?` are one type with one layout and take the same clear.

### Generics: type variables

A header declares one or more type variables (`<T>`, `<K: Ordered, V: Printable>`), each
inferred from the parameters that name it — any parameter, not only the first.  A variable no
parameter names is refused at the declaration.

**Without bounds:** only assign, return, and store are allowed on `T`.
**With bounds (`<T: Interface>`):** method calls and operators declared
in the interface are allowed on `T`.  See § Interfaces above.

Rules: `(G-Gen)`, `(G-Check)`; catalogue: @F25, @F26.

### Text: comprehensive operations

The stdlib provides `starts_with`, `ends_with`, `find`, `contains`, `replace`,
`trim`, `split(char)`, `join(separator)`, `to_uppercase`, `to_lowercase`,
`len`, and slicing.  `split` and `join` are inverses:
`"a,b,c".split(',').join(",") == "a,b,c"`.
