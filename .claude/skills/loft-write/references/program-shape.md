<!-- reference for the loft-write skill: read for an interface, a multi-line string, loop attributes, `main` with arguments, or the CLI -->
# loft-write: program shape and less common syntax

The syntax half of the [loft-write skill](../SKILL.md) that a short program rarely needs.  Read
it when you declare an interface or a bounded generic, write a multi-line backtick string, use
a loop attribute (`v#index`, `v#remove`), take command-line arguments in `main`, or run loft by
hand.

## Interfaces and bounded generics

```grammar
interface Comparable {
  fn less_than(self: Self, other: Self) -> boolean
}

fn find_min<T: Comparable>(v: vector<T>) -> T { ... }
```

Structural satisfaction: if the methods exist, the type satisfies the interface.
No `impl` block needed. Built-in types satisfy `Ordered`, `Equatable`, `Addable`,
`Numeric`, `Scalable`, `Printable` automatically, and EVERY type — a struct, vector,
tuple, enum — is `Equatable` (@C91).

## Backtick strings (`` `...` ``)

**Multi-line.** Supports `{expr}` interpolation. Bare `"` is literal.
Auto-strips common leading indentation (based on closing backtick column).

<!-- from tests/reference/skill-constants.loft -->
```loft
SHADER = `
  #version 330 core
  void main() {{
      gl_Position = vec4(0.0, 0.0, 0.0, 1.0);
  }}
`;   // a backtick string is a format string too: a literal brace is doubled
```

## Loop attributes

```grammar
for v in collection {
    v#index    // 0-based position (vector/sorted); byte offset (text)
    v#count    // iterations completed so far (all types)
    v#first    // true for first element (all types)
    v#next     // byte offset after current char (text only)
    v#remove   // remove current element (filtered loops only)
}
```

`#index` is NOT supported on `index` collections — use `#count` there.


## The entry point, and reading arguments

```grammar
fn main() { println("hi"); }                 // the usual form
fn main(args: vector<text>) {                // the invocation arguments
  for a in args { println(a); }              // `loft p.loft Ada Grace` → Ada, Grace
}
```

**One `vector<text>` parameter is the ONLY supported shape.** It works on both backends.
Any other spelling is a clean compile error (*"`main` takes no parameters, or one
`vector<text>` …"*; guard `tests/main_signature.rs`). There is
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

