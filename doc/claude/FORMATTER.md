
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Loft Code Formatter

`loft fmt` — a canonical, opinionated formatter for `.loft` source, in the spirit of `gofmt`:
one style, no configuration.  It is a loft program (`tools/fmt/whole.loft`) run by the
interpreter through the `loft::host` call API.  The design and its open steps are in
[plans/formatter-parser-driven.md](plans/formatter-parser-driven.md); the retired Rust formatter
is in [FORMATTER-history.md](FORMATTER-history.md).

---

## Invocation

```
loft fmt <file…>            # print the formatted source (default; file untouched)
loft fmt --write <file…>    # rewrite in place; prints `formatted <file>` per changed file
loft fmt -w <file…>         # same as --write
loft fmt --check <file…>    # exit 1 if any file differs; lists them on stderr
loft fmt -                  # stdin -> stdout
```

- `--check` and `--write` together: error, exit 1.
- No file: usage line, exit 1.  Unknown option (including `--help`): `unknown option`, exit 1.
- An unreadable file: message on stderr, exit 1, the remaining files are still processed.
- `LOFT_FMT_WIDTH=<n>` sets the width budget (default 100).
- The formatter source is embedded in the binary (`include_str!`); the stdlib `default/`
  directory is resolved as for `loft run` (beside the binary, else the source tree).
- The LSP `textDocument/formatting` runs the same program (`src/lsp.rs`), so editor and CLI
  produce identical output.  A tidy buffer yields no edit.

---

## How it is built

`src/main.rs::run_fmt_command` compiles `tools/fmt/whole.loft` once
(`Program::from_source_with_stdlib`) and calls its `format(text) -> text` per file.
`whole.loft` is the only file the binary uses.  It is self-contained and has three layers:

1. **Lossless lexer** (`lex`) — every byte belongs to one token: words, operators, `//`
   comments, whitespace, newlines.  Strings (`"…"` with `{expr}` slots and `{{`/`}}`), backtick
   strings and char literals are ONE opaque token; the formatter never rewrites their
   interior.  `#directive` is one token.
2. **Bracket tree** (`parse`) — `[]`, `{}`, `()` nest into `Group` nodes; all else is a `Leaf`.
   Error-tolerant: an unmatched close is a leaf, an unterminated group has an empty close.
3. **Renderer** — `render_stmts` (statements, one source line per output line, re-indented),
   `render_range` (one line of tokens, spacing by `sp`), `render_block`, `render_container`.
   Whitespace is re-derived; comments are kept.  Layout is direct string building, not a
   `Doc` algebra.

Classification of a `{…}` group is syntactic, confirmed against its body: a struct or enum
definition (`struct`/`enum` before the name) and a struct literal (`CamelName { field: … }`,
no top-level `;`, at least one top-level `:`) are DATA containers; everything else (fn, `if`,
`for`, `match` arms, `-> Type {`, a bare `{`) is a BLOCK.  An `interface` body is a forced block.

### The prototype files

`cst.loft`, `roundtrip.loft`, `fmt.loft`, `rules.loft` in `tools/fmt/` are the step-by-step
groundwork (lossless lexer, tree, a rule-free `Doc`/`Group` engine with a stubbed transparent
`Group` resolver, a first container-rule cut).  Each carries its own copy of the lexer, runs
standalone (`LOFT_FMT_FILE=<file> loft --interpret <step>.loft`), and is NOT used by `loft fmt`.
`whole.loft` supersedes them.

---

## Invariants

For any input `x` (the harness in `whole.loft`'s `main`):

- **Idempotent**: `format(format(x)) == format(x)`.
- **Token stream preserved**: the non-trivia tokens of `format(x)` equal those of `x`, except
  that a trailing `,` before a closer may be added or removed.
- **Comments preserved**: the `//` comment count is unchanged.
- **Empty stays empty**; otherwise the output ends in exactly one newline.

Run the self-check on one file (prints `OK …` or `FAIL !IDEMPOTENT !SEMANTICS !COMMENTS`):

```
LOFT_FMT_FILE=$PWD/tests/scripts/01-integers.loft loft --interpret tools/fmt/whole.loft
```

Set `LOFT_FMT_PRINT=1` to print the formatted text instead.

---

## Rules applied

### Indentation and blank lines

- 2 spaces per level.
- Statement line structure is preserved: one source line stays one output line (a
  `a; b` line stays a line).  Blank runs collapse to one blank line; leading blanks are dropped.

### Blocks `{ }`

- Opening brace on the header line, closing brace on its own line at the enclosing indent.
- A `fn` body and an `interface` body always break.
- Any other block with at most one `;`, no nested `{…}` and no `//` comment stays inline as
  `{ expr }` when it fits the width; otherwise it breaks.  An empty body is `{}`.

### Data containers (struct/enum definitions, struct literals, `[…]`)

- A struct or enum definition always breaks: one field or variant per line, trailing comma.
- Otherwise a container stays inline when it fits the width and nothing forces a break.
  Forced break: a `//` comment inside, or a `[…]` holding a struct literal (`Name { … }`).
- A broken container has one element per line and a trailing comma, except a pure-scalar
  `[…]` vector that overflows: its elements wrap into rows filled to the width.
- A same-line trailing comment after an element's comma stays on that line; leading comments
  keep their own lines.
- A `(…)` list, a comprehension `[for … { … }]` and a match slice pattern (`*`/`+` repetition
  or `..rest`) break on width only, one element per line, and take no trailing comma after
  the last element.

### Spacing

- One space around binary and assignment operators, `->`, `=>`, `??`, `&&`, `||`.
- No space before `,` `;` `:` `)` `]`, none after `(` `[`; one space after `,` `;` `:`.
- No space around `.` and `::`; none before `(` or `[` in a call or index.
- Unary `-` `+` `*` `&` (after an operator, opener, `,` `;` `:` or `return`) and `!` hug their
  operand.
- Generic brackets are tight: `vector<T>`, `sorted<K>`, `Name<T>`; `<` after any other token is a
  comparison and is spaced.
- `#rust"…"` hugs its string; a repetition postfix `)*` / `)+` hugs the paren.

### Comments

`//` comments are kept verbatim.  A trailing comment stays at the end of its line, one space
after the code.  A comment alone on a line is re-indented.

---

## Not implemented

Live plan: [plans/formatter-parser-driven.md](plans/formatter-parser-driven.md) (Step 4 rules
"keep landing").  The formatter does NOT yet:

- align consecutive trailing comments to a common column;
- break, join or wrap statements and expressions (only the containers above wrap);
- insert or normalise blank lines between top-level items;
- sort `use` lines, or strip trailing commas in data containers (it adds them when breaking);
- run the `Doc`/`Group` layout engine of `fmt.loft` (a prototype; `whole.loft` renders directly).

Not every stdlib file is canonical yet: `loft fmt --check default/*.loft` lists
`default/01_code.loft` and `default/02_files.loft`.

---

## Tests

| Test | Covers |
|---|---|
| `tests/host_call.rs::formatter_dogfood` | struct definition expands |
| `tests/host_call.rs::formatter_enum_variant_if_body_is_a_block` | `if x == E.A { … }` is a block, no `;,` |
| `tests/host_call.rs::formatter_qualified_variant_before_block_is_not_a_struct_lit` | `Light::Point { … }` and `-> a::B { … }` bodies are blocks |
| `tests/host_call.rs::formatter_width_counts_characters_not_bytes` | width is measured in characters |
| `tests/lsp_transport.rs::formatting_returns_a_whole_document_edit_and_noops_when_tidy` | LSP formatting = one whole-document edit; tidy buffer = none |

The corpus check is the `whole.loft` self-check above run over `tests/scripts/*.loft` and
`default/*.loft`; no make target or CI step runs it.

---

## See also
- [LOFT.md](LOFT.md) — the syntax the formatter must preserve
- [CODE.md](CODE.md) — style conventions
- [plans/rust-host-call-api.md](plans/rust-host-call-api.md) — the `loft::host` API `loft fmt` uses
- [FORMATTER-history.md](FORMATTER-history.md) — the retired Rust formatter
