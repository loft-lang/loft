# Loft Declarations, Files and Grammar

Functions, references, constants, type aliases, imports and name resolution, plus file structure, `#rust`/`#iterator` annotations, the shebang and the informal grammar.  Part of the language reference: [LOFT.md](LOFT.md).

---

## Declarations

### Functions

```
fn function_name(param: type, other: type = default_value) -> return_type {
    // body
}
```

- `pub` prefix makes a definition publicly visible (applies to functions, structs, and enums).
- Some names are **reserved** and cannot name a program's function: the words `assert`, `panic`,
  `sizeof` and `debug_assert`, which the language keeps for meanings of its own, and the name of a
  standard-library function that is not a method (`log_info`, `parallel_for`, …).  Names the
  compiler only lowers for built-in types stay open: a program may define `sort`, `insert`,
  `map`, `next` or `exhausted` for its own types, and a call on those types reaches it.
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
  - A **view** of the value is read-only too: a loop variable over its elements
    (`for e in p { e.x = 1 }`), an element or field bound to a local (`e = p[0]`, `q = p.inner`,
    `e = &p[0]`), a loop over such a view, and `e#remove` in a loop over it are all errors that
    name the view and the value it views. A COPY is the reader's own and stays writable: a
    scalar or `text` read out (`n = e.x; n += 1`) and a whole-value bind (`v = p; v += […]`).
    One over-approximation: a view local later rebound to a fresh value (`e = p[0]; e = T{…};
    e.x = 1`) is still refused — the check follows the bind, not the flow.
  - Passing the value (or a view of it) to a **`&` parameter** is an error: the callee may write
    it. Passing it to a `const` parameter is always allowed.
  - Passing it to a plain **record or collection parameter** is reported unless that parameter is
    declared `const` — decided by the signature, not by whether the callee happens to write
    (DESIGN_DECISIONS.md C124): the error `const-to-plain-parameter`. A read-only helper says so:
    `fn total(v: const vector<T>)`. The
    standard library's readers (`len`, `sum`, `join`, the `JsonValue` accessors, …) declare their
    parameters `const`; its writers (`clear`, `seek`, the `store_load*` targets) do not. `text`
    and scalar parameters take their own copy and are not affected.
  - A **function reference** is judged by its TYPE the same way: `fn(const T)` declares a
    parameter the functions it holds may not write, and a `const` value passed through a
    reference whose type does not say so is the same warning. A function whose parameter is plain
    cannot stand where `fn(const T)` is expected (an argument, a field, a return); the other
    direction is free. A local, an `if` or a `match` that may hold functions with different
    `const` parameters promises only the `const` they all declare.
  - The elements of a `const` collection handed to `map`, `filter`, `any`, `all`, `count_if` or
    `reduce` reach the callback on the same terms: a short `|p|` callback over one gets a `const`
    parameter, and a named function whose parameter is plain is the warning.
  - A lambda's `const` parameter is read-only in its body, and a closure that captures a `const`
    value — or a view of one — cannot write it.
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

A standard-library function implemented in Rust is declared without a body, followed by
`#rust "..."` — in `default/` only; a library binds Rust with `#native` (C87):
```
pub fn starts_with(self: text, value: text) -> boolean;
#rust "@self.starts_with(@value)"
```

Rules: [formal/calls.md](formal/calls.md) `(F-Call)`, `(F-Args)`, `(F-Arity)`, `(F-Return)`; a stub body answers the type's default (`(N-Default)`).

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

The KEYED collections are not an exception. A `&` alias to a `hash` / `sorted` / `index` /
`trie` / `spatial` (`a = &h; a += [rec]`) is a live link, so the append lands in `h` and both
names read the same length; a `&` keyed PARAMETER appends through to the caller's collection
the same way. A keyed collection is already a store handle, so the `&`-free spelling reaches
the caller's collection too — the two spellings differ only in whether the bind copies, and
for a keyed collection neither does. Guards: `tests/scripts/1433-a-keyed-alias-is-a-link-not-a-copy.loft`
(the alias, every kind, both names read) and `tests/scripts/1445-a-keyed-parameter-appends-through-its-link.loft`
(the parameter). Everything else on this page — `insert` / `reverse` / `sort` / `reserve` and
the generics over `vector<T>` (`sum`, `min_of`, `max_of`) — goes through a `&` as stated.

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

<!-- from tests/reference/skill-constants.loft -->
```loft
TAU      = 6.28318530717958;    // bare-name form
const E  = 2.71828182845905;    // const-keyword form (P246, 2026-05-11)
pub const MAX_SIZE = 256;       // pub + const combine for exported constants
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
referenced, not re-run.  Binding it to a local **copies** it, like any whole-value bind
(`v = NAMES; v += ["c"]` leaves `NAMES` as declared), while an index, a `len` or a `for`
over the constant reads it in place.  A function that writes through a parameter handed the
constant (`grow(NAMES)`) is refused at run time — a parameter aliases its argument, and a
constant cannot be written; bind it first. Its elements are built from their fields, so they must be
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

Catalogue: @F46; guard: `tests/scripts/78-type-aliases.loft`, `tests/scripts/167-a-size-is-written-in-a-type-alias.loft`.

### Library imports

```
use arguments;                     // the `arguments::` qualifier only: `arguments::parse_args(…)`
use arguments::*;                  // every public name, bare, for THIS file
use arguments::parse_args;         // selective: one name, bare
use arguments::(parse_args, Flag); // selective group — MULTIPLE names need parentheses
use arguments::Flag as Opt;        // alias an imported name (bare `Opt`)
use arguments::(Flag as Opt, parse_args);  // per-name aliases inside a group
use arguments as args;             // library alias → `args::parse_args` (qualifier only)
pub use arguments::*;              // import AND pass on: whoever imports this file gets them
pub use arguments::(Flag);         // pass on some
```

Searches for `arguments.loft` in `lib/`, the current directory, directories from the
`LOFT_LIB` environment variable, and relative to the current script.
`use` declarations must appear at the top of the file, before any other declarations.
A qualified `lib::fn()` auto-loads the library, so an explicit `use` is optional for
qualified access.

**Multiple names from one library must be parenthesised.** `use lib::a, b;` (a flat
comma list) is a compile error; write `use lib::(a, b);`.

#### What an import brings in, and what it passes on (@C98)

The rules are Rust's:

- **A bare `use lib;` brings in no name** — only the `lib::` qualifier.  A program that
  only writes bare imports is immune to a library growing a name: every library name it
  uses is spelled `lib::name`, so a new name in the library cannot collide with its own.
- **`use lib::*;` and `use lib::(a, b);` bring names in bare, for this file only.**  A
  file that imports this one does not receive them.
- **`pub use lib::*;` and `pub use lib::(a, b);` also pass them on.**  This is how a
  package's entry file offers the names of its modules:

  ```
  // graphics.loft — the package entry
  pub use math::*;      // Vec3, …: a consumer's `use graphics::*;` receives them
  pub use mesh::*;
  use scene_internal;   // for this file only
  ```

  `pub use lib;` is refused: a qualifier is not a name an importer can receive.  So is a
  bare `use self::module;`: a package's own module has no short qualifier (see below), so
  that form would bind nothing — write `use self::module::*;` or `use self::module as m;`.

A name that does not resolve because of how it was imported says so, and names both
cures: `unknown type Pt — it is in geo, and a bare use geo; brings in only the geo::
qualifier: write geo::Pt, or import the name with use geo::(Pt);`.  The same holds for a
library in between that uses a name without passing it on.

Guards: `tests/scripts/c98-a-bare-use-brings-in-only-the-qualifier.loft`,
`tests/scripts/c98-an-import-serves-its-file-and-pub-use-passes-it-on.loft`,
`tests/scripts/c98-a-plain-use-in-a-library-passes-nothing-on.loft`.

#### The import form decides whether your own name can clash (loft#1094)

**A wildcard `use lib::*;` brings every public name into THIS SOURCE's namespace, so
declaring one of them yourself is a redefinition and is refused.  A selective
`use lib::(a, b);` brings in only what it names, and a bare `use lib;` brings in none, so
every OTHER name in that library stays free for you to declare** — and the two live side
by side, reached unambiguously in each source that did not import the other (the module
scoping of @PLN102 C97).

```
use hex_body::*;                     struct Frame { … }   // refused: `Frame` came in bare
use hex_body::(Rig, rig_world_seg);  struct Frame { … }   // fine: only those two came in
use hex_body;                        struct Frame { … }   // fine: only `hex_body::` came in
```

**A selective or bare import makes you immune to the library GROWING a name.** A
dependency adding a `Frame` in a later release cannot collide with yours, because you
never asked for it. With a wildcard import it can, and that refusal is the point of the
check.

Where two same-named types do meet — one module imports the library with a wildcard while
another declares its own — the mismatch names both DECLARATION sites rather than printing
the one name twice.

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
  staying out of it is what `self::` is for, so it is withheld rather than missing, and
  the compiler says exactly that at the call site (guard `tests/module_name_clash.rs`).
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
  the same line and computes the qualified key, which is absent. The loader asks whether
  the FILE is loaded, by canonical path, and binds the second name to the source that
  exists (guard `tests/imports.rs`). Two different files that merely share a module name are
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

<!-- from tests/reference/enum-variants.loft -->
```loft
enum Color { Red, Green }
enum Light { Red, Amber }
```

A bare variant used as a **value** resolves from its type context — a `match`
subject, a typed declaration (`c: Color = Red`), a comparison (`c == Red`), a
function argument, a return position, or a struct-field type/default. With no
context, qualify it: `Color.Red` (or `Color::Red`). Defining a *new untyped
variable* directly from a bare variant is a deliberate error:

<!-- from tests/reference/enum-variants-refused.loft -->
```loft
x = Red;            // error: ambiguous variant 'Red' (a variant of Color, Light) — qualify it, e.g. 'Color.Red'
```

A qualified name, or a declared type, supplies the context:

<!-- from tests/reference/enum-variants.loft -->
```loft
x = Color.Red;      // ok
c: Color = Red;     // ok — the declared type supplies the context
```

This keeps a later `enum Light { Red, … }` from silently re-pointing an existing
bare assignment. The variant name remains usable as a **type / constructor**
(`Circle { … }`, `s: Circle`, `fn f(self: Circle)`), so struct-variant
construction is unaffected.


## File structure

A loft file may contain (in any order):
- `use <library>;` imports (must appear at the top)
- `pub` / non-`pub` function definitions
- Struct definitions
- Enum definitions
- Type aliases
- Top-level constants

Guard: `tests/scripts/88-imports.loft` (a `use` after a declaration is refused).

---

## External function annotations (`#rust`, `#iterator`)

Used only in the standard library (`default/`) to bind loft declarations to Rust
implementations; anywhere else they are refused, and a library binds Rust with `#native` and a
native crate instead ([PACKAGES.md § Function binding model](PACKAGES.md#function-binding-model), C87):

<!-- from default/03_text.loft -->
```loft
pub fn trim(self: text) -> text[self];
#rust"@self.trim()"
```

---

## Operator definitions (internal)

Operators are defined as functions named `OpXxx` in default files and linked to
infix/prefix syntax by the parser. Examples: `OpAdd`, `OpEq`, `OpNot`, `OpConv`, `OpCast`.

Their home is `default/01_code.loft`; the parser's dispatch table names each `OpXxx` it lowers to, and a misspelt one fails the stdlib to load, so `make ci` keeps this section.

---

## Shebang

Loft scripts support a Unix shebang line for direct execution:
<!-- from tests/reference/shebang.loft -->
```loft
#!/usr/bin/env loft
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @SCRIPT — run top to bottom, not through --tests
// @PAGE: LOFT_DECLARATIONS.md § Shebang — a Unix shebang on the first line is skipped by the lexer, so the
// file runs both as `loft shebang.loft` and, once executable, as `./shebang.loft`.

greeting = "hi from a script with a shebang";
assert(greeting.starts_with("hi"), "the line after the shebang ran")
println(greeting)
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
left-associative except `**`, which is right-associative) given in [§ Operators](LOFT.md#operators);
a unary prefix (`!`, `-`, `~` in `single`) binds tighter than every binary operator. The
parser realises this with a precedence-climbing walk (`OPERATORS` / `parse_operators`); the
two statements — this grammar and that table — together pin every expression's shape.
