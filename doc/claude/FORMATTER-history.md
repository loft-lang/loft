<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# Loft Code Formatter — history

Record companion of [FORMATTER.md](FORMATTER.md).  What the formatter WAS: the Rust
formatter that `loft fmt` replaced.  Nothing here holds now.

---

## The Rust formatter (`src/formatter.rs`, deleted)

Status line it carried: "shipped — implemented in `src/formatter.rs` (T2-0, 2026-03-16).
`loft --format file.loft` formats in-place; `loft --format-check file.loft` exits 1 if not
canonical."  The parser-driven `loft fmt` (`tools/fmt/`, written in loft) replaced it; the
Rust file is gone (`git log --diff-filter=D -- src/formatter.rs`), and the binary answers
`unknown option: --format`.

## Invocation (removed)

```
loft --format file.loft          # format in-place (overwrites)
loft --format-check file.loft    # exit 1 if file differs from formatted output
```

Or as a pipe:
```
loft --format -             # read stdin, write stdout
```

---

## Implementation (removed)

### `src/formatter.rs` — standalone tokenizer + state machine

The formatter uses its **own scanner** (`fn scan`) that tokenizes loft source into a `Tok`
enum — completely independent of the parser's `lexer.rs`.  This keeps formatting self-contained
and allows formatting files that do not yet fully parse.

Public API:
```rust
pub fn format_source(source: &str) -> String
pub fn check_source(source: &str) -> bool
```

Both functions normalize CRLF line endings at entry so they behave identically on Windows
and Unix.

Internal types:
```rust
enum Tok { Word(String), Int(String), Flt(String), Str(String), Chr(String),
           Sym(String), Comment(String), Newline, Blank }

enum Ctx { Block, StructDef, ArgList, ArrayLit, StructLit }

struct Fmt {
    depth: usize,
    prev: String,           // last emitted token text (or sentinel like "unary")
    ctx: Vec<Ctx>,
    next_brace_is_block: bool,
    out: String,
}
```

`next_brace_is_block` is set by all block-opening keywords (`fn`, `if`, `else`, `for`,
`while`, `loop`, `match`) and by `->`, so that `{` always opens a `Block` context when
following a keyword, regardless of what the immediately preceding token was.

### `src/main.rs`

`--format` / `--format-check` / `--format -` flags are parsed before the normal execution
path and handled without loading the standard library.

### `src/lib.rs`

`pub mod formatter;` exposes the formatter for integration tests.

---

---

## Rules of the Rust formatter (some overlap `loft fmt`; the live set is in FORMATTER.md)

### Formatting rules

#### Indentation

- 2 spaces per depth level (matches existing codebase convention)
- Depth increases after `{`, decreases before `}`

#### Blank Lines

| Context | Rule |
|---|---|
| Between top-level items (`fn`, `struct`, `enum`, `type`, `use`, constant) | 1 blank line |
| Inside a function body | preserve at most 1 blank line |
| Inside struct/enum body | no blank lines |

#### Braces `{ }`

Opening brace **always** on the same line as the header.
Closing brace **always** on its own line, at the enclosing indent.

```loft
fn foo(x: integer) -> integer {
  x + 1
}
```

**No inline blocks** — even a one-liner body is expanded:
```loft
// input:   if x > 0 { return x; }
// output:
if x > 0 {
  return x;
}
```

This is the single most important rule: it eliminates all "should I inline this?" decisions.

#### Semicolons `;`

Semicolons terminate statements and are followed by a newline at the same depth.
The formatter does **not** insert missing semicolons (that remains a parse error).

#### Commas `,`

| Context | Rule |
|---|---|
| Function parameter list `fn f(a, b)` | space after comma, stay on one line; wrap if line > 80 cols |
| Function call `f(a, b)` | space after comma, stay on one line; wrap if line > 80 cols |
| Struct literal `Point { x: 1, y: 2 }` | space after comma, stay on one line |
| Struct/enum definition body | comma + newline (each field on its own line) |
| Array literal `[1, 2, 3]` | space after comma, stay on one line; wrap if line > 80 cols |

Trailing commas in struct/enum definitions are **stripped** (the formatter enforces
"no trailing comma" as the canonical style, matching the existing test files).
Trailing commas in call/param/array contexts are also stripped.

The heuristic for "struct/enum definition body vs struct literal" is purely syntactic:
- In a `struct Name {` or `enum Name {` header → definition mode (multi-line)
- After `=`, after a type name in expression position → literal mode (single-line)

**Line-length wrapping for param/call/array lists (> 80 cols):** emit each element on
its own line at `depth+1`, with the closing `)` or `]` on a new line at `depth`.
This handles long function signatures without needing the full Wadler/Lindig algorithm.
```loft
// short — stays on one line:
fn add(a: integer, b: integer) -> integer {

// long — each param on its own line:
fn process(
  source: vector<Record>,
  target: vector<Record>,
  options: Options,
) -> Result {
```

#### Spaces Around Tokens

| Token(s) | Rule |
|---|---|
| Binary operators `+ - * / % == != < > <= >= && \|\| & \| ^ << >> ?? as` | 1 space before and after |
| Assignment operators `= += -= *= /= %=` | 1 space before and after |
| `->` (return type) | 1 space before and after |
| `=>` | 1 space before and after |
| `:` in type annotation `field: type`, `param: type` | no space before, 1 space after |
| `::` (path separator) | no spaces |
| `.` (field/method access) | no spaces |
| `(` in call | no space before |
| `[` in index | no space before |
| `,` | no space before, 1 space after (except before newline) |
| `!` (unary not) | no space after |
| `-` (unary minus) | no space after |
| `#` (loop attribute) | no space before or after |
| `?` | no space before |

#### Keywords with Trailing Space

`fn`, `pub`, `struct`, `enum`, `type`, `use`, `if`, `else`, `for`, `in`, `return`,
`break`, `continue`, `loop`, `as`, `and`, `or`, `not`

Exception: `else if` → 1 space between `else` and `if`.

#### Comments

Line comments (`// ...`) are preserved verbatim.
A comment on its own line keeps its current indentation depth (re-indented to `depth`).
A trailing comment (end of a line with code) is separated from the code by 2 spaces.

Block comments (`/* ... */`) are not currently used in loft and are not handled.

---

## Context Stack

To decide inline-vs-multiline and spacing, the formatter maintains a stack of contexts:

```
enum Ctx {
    Block,          // inside { } of fn/if/for/loop body — multi-line
    StructDef,      // inside struct/enum declaration — multi-line, comma+newline
    ArgList,        // inside ( ) of fn call or declaration — single-line
    ArrayLit,       // inside [ ] — single-line unless overflow
    StructLit,      // inside { } of struct/enum constructor — single-line
    FormatExpr,     // inside { } of a format string — passthrough
}
```

Context is pushed on `{`, `(`, `[` and popped on the matching closer.
Determining the context at `{`:
- Stack top is `Block` or `ArgList` and prev token is `)` or `->` type → `Block`
- Prev token is identifier or type keyword → `StructLit`
- After `struct`/`enum` keyword path → `StructDef`
- Inside a `CString` format expression → `FormatExpr`

---

## Token Reconstruction

The formatter scans the token stream and, for each token, emits:

1. Any pending newlines / indentation
2. Any spacing before the token (based on rules above)
3. The token text
4. Any comment that was attached to this line

The token → text mapping:

| LexItem | Output |
|---|---|
| `Integer(n, false)` | decimal `n` |
| `Integer(n, true)` | `0x{N:X}` for hex constants, else decimal |
| `Long(n)` | `{n}l` |
| `Float(f)` | shortest round-trip decimal |
| `Single(f)` | `{f}f` |
| `Token(s)` | `s` as-is |
| `Identifier(s)` | `s` as-is |
| `CString(s)` | `"{s}"` with interior `{...}` expressions re-formatted |
| `Character(c)` | `'c'` |
| `LineComment(s)` | `// {s}` |

---

## Example: Before and After

**Input (messy):**
```loft
fn   add( a:integer,b:integer)->integer{return a+b;}
struct  Point{x:float,y:float}
fn dist(p:Point)->float{ let d=p.x*p.x+p.y*p.y; d.sqrt() }
```

**Output (formatted):**
```loft
fn add(a: integer, b: integer) -> integer {
  return a + b;
}

struct Point {
  x: float,
  y: float
}

fn dist(p: Point) -> float {
  let d = p.x * p.x + p.y * p.y;
  d.sqrt()
}
```

---

## File Layout in `src/formatter.rs`

```
fn scan(src: &str) -> Vec<Tok>          // standalone scanner

impl Fmt {
    fn process(&mut self, tokens: &[Tok])
    fn handle_sym(&mut self, s: &str, tokens: &[Tok], i: &mut usize)
    fn close_brace(&mut self, tokens: &[Tok], i: &mut usize)
    fn need_space(&self, tok: &str) -> bool
    fn push_ctx(&mut self, ctx: Ctx)
    fn pop_ctx(&mut self)
    fn emit(&mut self, s: &str)
    fn newline(&mut self)
}

pub fn format_source(source: &str) -> String
pub fn check_source(source: &str) -> bool
```

---

## Non-Goals (Explicitly Out of Scope)

- **Full optimal line-length wrapping (Wadler/Lindig)**: the simple > 80-col heuristic on
  param/call/array lists covers the common cases; arbitrary expression wrapping is not attempted
- **Semantic analysis**: the formatter works on tokens alone; it does not type-check; files
  that do not yet compile can still be formatted
- **Multi-file project formatting**: `loft --format` takes one file at a time; shell
  globbing (`loft --format src/*.loft`) handles projects

## In Scope (revised from initial draft)

- **Import sorting**: consecutive `use` lines are sorted alphabetically within each contiguous
  block; a blank line between `use` statements starts a new block (preserving intentional grouping).
  This is safe because loft does not allow two `use` statements to import the same identifier — if
  they did, the second would already be a compile error.
- **Trailing comma stripping**: trailing commas after the last element in struct/enum definitions,
  call lists, and array literals are removed. Adding trailing commas is not done (requires
  knowing whether the grammar permits them in each position).

---

## Its tests (`tests/format.rs`, removed)

`tests/format.rs` — 11 tests using `include_str!` wrapped in `lf()` for CRLF safety:

| Test | Type | File(s) |
|---|---|---|
| `roundtrip_comments` | roundtrip | `comments.loft` |
| `roundtrip_struct_def` | roundtrip | `struct_def.loft` |
| `normalize_messy` | normalize | `messy.loft` → `messy.loft.fmt` |
| `format_check_already_formatted` | check_source | `comments.loft` |
| `format_check_needs_formatting` | check_source | `messy.loft` |
| `roundtrip_unary_minus` | roundtrip | `unary_minus.loft` |
| `roundtrip_range_ops` | roundtrip | `range_ops.loft` |
| `roundtrip_binary_literals` | roundtrip | `binary_literals.loft` |
| `roundtrip_if_for_blocks` | roundtrip | `if_for_blocks.loft` |
| `roundtrip_adjacent_words` | roundtrip | `adjacent_words.loft` |
| `normalize_else_same_line` | normalize | `else_same_line.loft` → `else_same_line.loft.fmt` |

Golden files live in `tests/format/`. `.gitattributes` enforces `eol=lf` on checkout so
`include_str!` yields `\n`-only strings on every platform.

---
