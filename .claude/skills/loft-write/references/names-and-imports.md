<!-- reference for the loft-write skill: read when writing `use`, `pub`/`#package`, or choosing a name that may clash with a builtin -->
# loft-write: imports, visibility and builtin names

The names half of the [loft-write skill](../SKILL.md).  Read it when a file imports a library,
shares items with sibling files, or declares a name the stdlib or a library also uses.

## Imports

<!-- from library:arguments -->
```loft
use arguments;                       // the `arguments::` qualifier only
use arguments::*;                    // all pub names bare, for this file
use arguments::parse_args;           // selective: just one name, bare
use arguments::(parse_args, Flag);   // selective group — MULTIPLE names need parentheses
use arguments::Flag as Opt;          // alias an imported name (bare `Opt`)
use arguments::(Flag as Opt, parse_args);   // per-name aliases inside the group
use arguments as args;               // library alias → `args::parse_args` (qualified-only,
                                     //   does NOT import names bare)
```

- **Multiple names are parenthesised** — `use lib::a, b;` (flat comma list) is a hard error;
  write `use lib::(a, b);`.
- `use lib;` gives the `lib::` qualifier only (@C98); `use lib::*;` brings every `pub` name in
  bare.  An import serves this file only; `pub use lib::*;` / `pub use lib::(a);` also passes
  the names on to whoever imports this file (a package entry offering its modules).
- `use lib as m;` gives a qualifier alias only (`m::fn`); it does not wildcard-import.
- A qualified `lib::fn()` auto-loads the library — an explicit `use` is optional for qualified
  access.
- **`use` declarations come before any other declarations in the file.**

## Visibility: `pub` and `#package`

**Everything a file declares is private to that file**; `pub` shows it (an item, or one
field).  A file that is part of something bigger starts with **`#package`**: to the files of
its own directory and of the directory directly above it, it reads as if everything in it were
`pub` (fields, literals, variants, private functions, glob and selective imports).  Not two
levels up, not a subdirectory, not a sibling; the reading files declare nothing.  A package's
tests (anything inside `tests/`) see what its `src/` shares, so test internal steps directly
instead of making them `pub`.  Prefer it over `pub` on every field of a type shared inside one
component; keep `pub` for what the component offers outside.  A used file may say `#package`,
never `#cwd`/`#link`.

## Builtin names — shadowing rules

- **Definitions CAN shadow a stdlib name.**  `enum E`, `struct File`, `pub PI = 3;` are legal
  even though the stdlib has `E`/`File`/`PI`; your name wins bare resolution and `std::Name`
  still reaches the original.  A LIBRARY name is different: one brought in by `use lib::*;`
  cannot be redefined (an error naming the library's definition), while a bare `use lib;`
  leaves the unqualified name free (`lib::name` reaches the library's).
- **Built-in TYPE-KEYWORDS are reserved** — you cannot define `struct integer`, `enum vector`,
  etc. (it errors: "conflicts with a type").  Reserved: `integer`, `float`, `single`, `text`,
  `boolean`, `character`, `vector`, `hash`, `sorted`, `index`, `radix`, `spatial`, `iterator`,
  `i8`/`i16`/`i32`, `u8`/`u16`/`u32`, `reference`.  This holds for `struct`, `enum`, and `type`
  alike.
- **A function's name is free as a local.**  Values and functions are separate namespaces:
  `chr = 65` binds a local and `chr(65)` in the same scope still calls the function.  True for
  every binding form — assignment, the typed local, a tuple element, a parameter, a `for`
  variable, a struct field — and for library functions as much as stdlib ones, which is what
  stops a library's new `pub fn` from taking a word away from its consumers.  Method names
  (`map`/`filter`/`reduce`/`len`/`sorted`/…) are fine as locals; the method call still
  resolves.  The one exception is the literal `null` (the skill's error table).

## Reserved words with no implementation

`debug_assert` is a reserved keyword and does nothing yet — using it in any position is a
compile error naming the reservation.  Use `assert(cond, "message")`, and do not use
`debug_assert` as a variable name.  `sizeof(Type)` IS implemented (`sizeof(integer)` is 8).
