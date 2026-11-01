<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 1a — the parser in loft

Stage 1 of [§ The path](README.md#the-path--stages-and-gates): loft's front end written in
loft, its image byte-identical to the Rust front end's `--emit-image`.  This doc is the
design of the parser half (lexer, syntax, elaboration) and the order it is built in.

## The shape — three layers

```
source ─▶ C lexer ─▶ Toks ─▶ syntax (PEG) ─▶ Syn arena ─▶ elaboration ─▶ Data + IR (ir.loft) ─▶ image
          src/c/loft_lex.h    selfhost/src/syntax.loft      selfhost/src/elab*.loft
```

1. **The lexer is the C runtime's** (owner, 2026-11-01): `src/c/loft_lex.h` ports
   `src/lexer.rs`'s loft mode, reached through the `crt` shim (`crt::lex`) and decoded into
   `Toks` by `selfhost/src/lex.loft` — one `Tok` enum value per token, with parallel `line`
   and `col` vectors.  `src/lexer.rs` interleaves with the parser inside string holes (the
   parser sets `Mode::Formatting` / `Mode::Code`); the C lexer reproduces those switches
   itself, so the whole file is lexed once, before parsing.
2. **The syntax layer is PEG cursor matches** (@PLN35, `formal/matching.md`): one function
   per grammar rule, `fn rule(p: Syntax) -> integer?`, whose body is a `match p { [ … ] => … }`
   over the token cursor.  It builds a SYNTAX tree and decides nothing semantic.
3. **The elaboration layer is the port of `src/parser/`'s semantics**: it walks the syntax
   tree in source order, twice (the Rust parser's `first_pass` and second pass), and makes
   the same `Data` definitions, the same variable numbering and the same IR the Rust parser
   makes while it reads tokens.

**Why the split.**  A PEG rule may run and then be abandoned (an alternative fails after it),
so a rule must be pure: `(P-Atomic)` promises a failed attempt is invisible, and the purity
check (@PLN35 PC4) accepts only writes to the cursor and to records the rule builds.  The Rust
parser's semantics write shared state on every token — a definition registered, a variable
numbered, a type joined, a diagnostic added — so they cannot sit inside a rule that may be
backtracked.  Nodes a failed alternative built are garbage in the arena and nothing points at
them.

**The rule of the port still holds:** the same `Data`, the same variable tables, the same IR
and the same diagnostics.  The syntax tree is the one structure the Rust parser does not
have, and it is internal: nothing outside the front end sees it.

## Syntax decisions that read semantics — each becomes a general node

The Rust parser takes a few SYNTACTIC decisions from semantic facts.  A survey of
`src/parser/` (2026-11-01, every `link()`/`revert()` and every `def_type`/`vars` read on a
syntax path) found each can be deferred: the syntax layer parses a node general enough to hold
every reading, and elaboration picks the reading with the fact the Rust parser read.

| Construct | Rust reads | Syntax node |
|---|---|---|
| `Name { … }` struct literal or a block after an expression (`objects.rs:2930`, `3102`) | `def_type(Name)`, `vars.name_exists` | `Path { body }` when the body is `ident:` / `"str":` / empty, never in a control head (`in_control_head`) |
| `{ f: v }` with no name (`control.rs:977`) | the target's type | `AnonRecord`, told from a typed local `{ x: T = … }` by `ident : Type =` |
| `Sh.Dot { r: 1 }` (`fields.rs:23`) | the variant's fields | a member followed by a brace body |
| `name<T>(…)` (`objects.rs:828`) | not a variable, not a type | call with type arguments |
| `Name<…>` in a type (`definitions.rs:4823`, `4886`) | `TypeTemplate` | type with arguments |
| `sizeof(…)`, `type_name(…)` argument (`control.rs:23103`) | `paren_type_ahead` | `TypeOrExpr` |
| `a.b(…)` method or a `fn` field called (`fields.rs:754`) | attribute `b` is a `Routine` | `MemberCall` |
| statement end after a block-like expression (`operators.rs:2108`, `2498`) | the left operand is `Void` | the block-like-statement rule: `if`/`for`/`while`/`match`/block end the statement |
| `x#index` (`objects.rs:992`) | `vars.name_exists` | postfix `#ident` |
| `v is V { a, b }` captures (`control.rs:14140`) | the variant has fields | a capture list when another `{` follows its `}` |
| a pattern identifier: binding, constant or variant (`control.rs:6555`, `11616`, `12701`) | the subject's type | one pattern grammar: `ident`, `ident : pat`, `Path ({…})?`, literal, range, group `* + ?`, `..rest`, `|` |
| `x[…]` index forms (`fields.rs:1613`) | the receiver's type | one index node `[args (.. =? expr?)? (: expr)?]` |
| format spec `{x:w}` (`objects.rs:4140`, `3603`) | the width names a variable; the type owns `to_text` | the spec's tokens kept as written, interpreted in elaboration |

## The token cursor and the arena

```loft
pub struct Syntax {
  src: vector<Tok>,          // the cursor's source: the first vector field
  pos: integer,              // the cursor
  farthest: integer,         // (P-Cursor) PC5's high-water mark — where a parse error points
  nodes: vector<Syn>,        // the arena; a node's children are indices into it
}
```

A rule answers the index of the node it built, or null when it does not match.  Children are
held as arena indices, never as `&v[i]` into a vector that grows (README § Sub-arcs, loft#2163).
Token positions stay in `Toks`; a node records the index of its first token, which names its
line and column.

## What the language lacks — fixed in the language first

Measured on `91-self-host @ ad9811d2b`, the shipped PEG cursor match cannot hold a grammar:

1. **A sub-rule must be the whole slice pattern** (`control.rs` `parse_subrule_slice_element`):
   `[LParen, e: expr, RParen]` is refused.  A production is a sequence of tokens and rules,
   so this is the first fix: **P-Rule in a sequence** — fixed elements and sub-rule calls
   mixed, each fixed element read at the running position, the cursor restored when the arm
   fails (`(P-Atomic)`).
2. **Every cycle of sub-rule calls is refused as left recursion** (PC3), even one that
   consumes a token before it recurses — `primary` calling `expr` inside its `[LParen]` arm.
   With (1), an edge after a consumed element is a consuming edge; WF-Term rejects only a
   cycle of non-consuming edges, as SUBRULE-DESIGN § 4.1 specifies.
3. **A sub-rule must answer a struct** (`peek_subrule_capture` asks for a `Reference`).  The
   arena form answers `integer?`; a scalar sub-rule is gated on `!= null` as a struct one is.
4. **Repetition and option of a sub-rule** (`[(xs: arg)*]`, `[(t: ty)?]`) — after 1–3, measured
   on the grammar as it is written.

Each is a compiler fix with its suite cell under `rules/matching/`, both backends, falsified —
never a hand-written cursor save and restore in the parser.

## Gates, in order

| Gate | Measures | Instrument |
|---|---|---|
| **T** tokens | the C lexer's stream equals `src/lexer.rs`'s, per file | `LOFT_TRACE_TOKENS=<file>` (the Rust stream the parser scanned) against `selfhost/front/tokens.loft`; `selfhost/front/tokens_gate.py` counts files that agree |
| **S** syntax | every corpus file parses with no error; the tree's tokens, in order, are the file's tokens | the syntax tree printer, round-tripped against the token stream |
| **E** elaboration | the image equals `--emit-image`, per definition | `selfhost/compare/`; HOW MANY programs agree |

The corpus for every gate: `default/*.loft`, `tests/scripts/`, `tests/docs/`, `lib/`, and the
language suite (`../loft-suite-91`).

## Order of work

1. Gate T: the C lexer and its gate (in progress).
2. The language fixes 1–3 above.
3. The syntax layer: definitions (`fn`, `struct`, `enum`, `type`, `interface`, `use`,
   constants, `#` attributes), types, statements, expressions by precedence
   (`operators.rs`'s table), patterns, string holes and specs, lambdas.  Gate S.
4. Elaboration, pass by pass, in the Rust parser's order — definitions first, then the
   bodies; gate E per definition so a partial port is measured.

## Open

- Diagnostics are part of the port (same text, same position), measured after gate E on the
  programs `tests/parse_errors.rs` and the suite's refusal cells carry.
