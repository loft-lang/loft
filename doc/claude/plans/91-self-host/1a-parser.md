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

## What the language needed — fixed in the language

A production is a sequence of tokens and rules, so the PEG cursor match was extended before the
grammar was written (`formal/matching.md` `(P-Cursor)`, `(P-Rule)`, `(WF-Term)`, `(WF-Pure)`;
cells under the suite's `rules/matching/P-Cursor` and `P-Rule`):

1. A sub-rule anywhere in a pattern: `[LParen, e: expr, RParen]`, read in segments, the rule
   called only when its arm is tried, the cursor put back when the arm fails.
2. A sub-rule group: `(args: expr)*(Comma)`, `+`, `(r: ret)?`.
3. Recursion through a consumed token is admitted; only a cycle of calls that can run before a
   token is consumed is left recursion.
4. A rule answers any type, so the arena form (`-> integer?`) is a rule.
5. A cursor match's mode covers its patterns, not its arm bodies (loft#2322).

A look-ahead is written as a PREDICATE rule — a function that answers the position without
consuming, or null (`brace_next`, `no_else`, `empty_literal_allowed`).

## Open

- **A rule answering a `vector`, captured in a match that is a function's tail**, loses the
  match's value on the interpreter and is refused by the C backend ("an integer expected, a Void
  found") when the rule also takes a hidden work buffer.  Minimal shape: `[k: kw, (tv: tvars)?,
  LParen] => 42` returns 0; the IR binds the match value nowhere and assigns the capture's
  conditional free (`if OpDistinctStore(tv, __ref_1) if OpDistinctStore(tv, __ref_2)
  OpFreeRef(tv)`, `get_free_vars.rs`'s witness-buffer arm) to `__ret_1`.  Hand-written twins
  (the capture a plain local, its assignment inside a condition) are right, so the trigger is
  the capture being a PATTERN binding (`pattern_binding`, an arm-scoped loop variable) that
  adopts the callee's return buffer.  Repro: the scratch copy of the syntax layer before its
  list rules answered `SList` nodes.  The grammar's list rules answer nodes, so it does not
  block the parser; it is this branch's own defect (the sub-rule lowering is new) and is fixed
  before the branch merges.
- **Diagnostics** are part of the port (same text, same position), measured after gate E on the
  programs `tests/parse_errors.rs` and the suite's refusal cells carry.
- **Scripts** (`src/script.rs`, a textual desugar before parsing) are not ported yet: gate S
  counts the file the driver would rewrite as a failure.

## Gates, in order

| Gate | Measures | Instrument |
|---|---|---|
| **T** tokens | the C lexer's stream equals `src/lexer.rs`'s, per file | `LOFT_TRACE_TOKENS=<file>` (the Rust stream the parser scanned) against `selfhost/front/tokens.loft`; `selfhost/front/tokens_gate.py` counts files that agree |
| **S** syntax | every corpus file parses with no error; the tree's tokens, in order, are the file's tokens | the syntax tree printer, round-tripped against the token stream |
| **E** elaboration | the image equals `--emit-image`, per definition | `selfhost/compare/`; HOW MANY programs agree |

The corpus for every gate: `default/*.loft`, `tests/scripts/`, `tests/docs/`, `lib/`, and the
language suite (`../loft-suite-91`).

## Where it stands

| Gate | Measured | Remaining |
|---|---|---|
| T tokens | 1845 of 1850 files | four `"\0"` literals (a one-NUL text is loft's null text), one script |
| S syntax | 1836 of 1843 files | seven refusal cells, all `@EXPECT_ERROR` files |
| E image | not started | — |

The syntax layer is `selfhost/src/syntax.loft` (the rules, one per production of
[1a-grammar.md](1a-grammar.md)); gate S's runner is `selfhost/front/parse.loft`.

## Order of work

1. Elaboration, pass by pass, in the Rust parser's order — definitions first, then the
   bodies; gate E per definition so a partial port is measured.
2. The script desugar, then diagnostics.
