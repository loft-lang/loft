<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN35 — What an actual loft parser looks like (target-syntax examples)

> Illustrative **target** syntax — not yet runnable; the PEG operators land per the phases in
> [IMPLEMENTATION.md](IMPLEMENTATION.md). Struct-variants only ([C89](../../DESIGN_DECISIONS.md)).
> The bar (D-F6 / C89): **each arm should read like the grammar production it implements**, using
> `|` / `?` / `*` / `+` on *named* elements — no regex training required.

## Token model — unit variants for fixed tokens, named fields for data

```loft
enum Token {
  // fixed tokens: UNIT variants (no payload) — they read as grammar terminals
  Let, Return, Move, Stop, Say,
  Eq, Semi, Comma, LBrack, RBrack, Colon,
  // data tokens: NAMED fields (never Ident(text) — always Ident { name })
  Ident { name: text },
  Num   { value: integer },
  Str   { s: text },
  Eof,
}
```

## Example A — an AST evaluator (structural dispatch — the sweet spot)

Recursion goes through a **collection** field (an inline self-referential field will not lay out):

```loft
enum Expr {
  Num  { value: integer },
  Neg  { operand: vector<Expr> },          // one child
  Add  { operands: vector<Expr> },         // [left, right]
  Call { name: text, args: vector<Expr> },
}

fn eval(e: Expr) -> integer {
  match e {
    Num  { value }                           => value,
    Neg  { operand: [ inner ] }              => -eval(inner),
    Add  { operands: [ left, right ] }       => eval(left) + eval(right),
    Call { name: "double", args: [ only ] } => 2 * eval(only),
    _                                        => 0,
  }
}
```

Each arm reads like the case it handles — *Neg of inner*, *Add of left and right*, *a `double`
call of one argument*. This is L2 (a nested pattern in a field) + L3.1 (slice-element captures).

## Example B — a token-stream command parser (grammar side by side)

```
grammar:   command := 'move' NUM NUM | 'say' STR | 'stop'
```
```loft
fn parse_command(ts: vector<Token>) -> Command {
  match ts {
    [ Move, Num { value: x }, Num { value: y } ] => Command.Move { x: x, y: y },
    [ Say,  Str { s } ]                          => Command.Say  { text: s },
    [ Stop ]                                     => Command.Stop,
    _                                            => Command.Unknown,
  }
}
```

The three arms line up one-to-one with the three alternatives of the grammar rule.

## Example C — optional and repetition

```
grammar:   param   := IDENT ( ':' IDENT )?
           numlist := '[' NUM* ']'
```
```loft
fn parse_param(ts: vector<Token>) -> Param {
  match ts {
    [ Ident { name }, ( Colon, Ident { name: ty } )? ] => Param { name: name, ty: ty ?? "any" },
    _                                                   => Param { name: "?", ty: "any" },
  }
}

fn parse_numlist(ts: vector<Token>) -> vector<integer> {
  match ts {
    [ LBrack, Num { value: n }*, RBrack ] => n,          // n : vector<integer>
    _                                          => [],
  }
}
```

`?` is the optional and `*` the repetition — the very `?` / `*` you would write in the grammar, on
*named tokens* (not regex classes). A quantifier binds directly to a **single** element with no
parens (`Num { value: n }*`); `( … )` groups only a multi-element sequence, as in the two-token
optional `( Colon, Ident { name: ty } )?` above. The capture types fall straight out of loft's
existing model: `ty : text?` (optional → discharge with `??`), `n : vector<integer>` (repetition
→ a vector). No `Some`/`Ok` wrapper to unwrap — the anti-`Ok(a)`.

## Example D — recursive descent (honest about the edges)

The PEG match recognizes the **shape** at a position and captures; **rule recursion is ordinary
function calls** — there is no in-pattern rule reference like `expr:expr`, so you capture tokens
and recurse. This keeps `match` a shape-recognizer, not a parser generator.

```loft
// grammar:   stmt := 'let' IDENT '=' expr | 'return' expr | expr
fn parse_stmt(ts: vector<Token>) -> Stmt {
  match ts {
    [ Let, Ident { name }, Eq, ..rhs ] => Stmt.Let      { name: name, value: parse_expr(rhs) },
    [ Return, ..e ]                    => Stmt.Return   { value: parse_expr(e) },
    _                                   => Stmt.ExprStmt { e: parse_expr(ts) },
  }
}
```

Edges to know — the honest limits of the design as scoped:

- **A tail element is a bare name** — `[ Let, Ident{name}, Eq, ..mid, last ]` is fine and binds
  `mid` to everything between, but `[ …, ..mid, Semi ]` is not: a variant sub-pattern or literal
  after a `..` is not parsed (loft#1419, `formal/matching.md` D-match-4). Destructure the bound
  name in a nested `match`, or use the L3.6 **iterator** input where the cursor stops before
  `Semi` and the caller continues from there (the natural streaming-parser model).
  *(The rest itself was tail-only until 2026-09-07; `(P-Rest)`'s `t` had always said otherwise.)*
- **No in-pattern rule reference** — `expr:expr` (match the `expr` sub-grammar inline) is not a
  feature; write `parse_expr(rhs)`.
- **Whole-consume** — an arm matches the ENTIRE slice unless it ends in `..rest`, so `[ Stop ]`
  means *exactly one* token.

## Verdict against the bar

The arms read like the productions they implement — the `move`/`say`/`stop` alternatives,
`IDENT ( ':' IDENT )?`, `'[' NUM* ']'` — with `|` / `?` / `*` on named tokens and no regex
training. The cost lands exactly where [C89](../../DESIGN_DECISIONS.md) said it would: extra parser
logic (whole-consume, bare-name tail elements, function-call recursion) bought for a readable surface.
