<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/grammar.md — concrete grammar & operator precedence (strict)

**Catalogue:** @I58 (parser), @F37 (operator precedence). Context-sensitive points: @F35 (string interpolation), @F12 (struct literals).

> **Rules then deviations** (see [README](README.md)). This area pins what the informal
> grammar in [LOFT.md § Summary of grammar](../LOFT.md) leaves out — **operator
> precedence and associativity** — and records where the surface is *not* context-free.
> Rough spot #4 from [FORMALIZATION.md](../FORMALIZATION.md), which the runtime-scoped
> red-flag map structurally misses.
>
> Ground truth: the precedence table `OPERATORS` (`src/parser/mod.rs:375`) and the
> precedence-climbing walk `parse_operators` (`src/parser/operators.rs:446`). The full
> statement/expression shapes stay in [LOFT.md](../LOFT.md); this doc adds the one fact
> that lives *only* in the parser.

## Notation

- A binary operator **binds tighter** than another if it groups first: in `a + b * c`,
  `*` binds tighter, so it reads `a + (b * c)`.
- **Left-associative**: equal-precedence operators group left-to-right — `a - b - c` is
  `(a - b) - c`.

---

## Rules

### Operator precedence — twelve levels, loosest first (the source order)

```
  level   operators                          (each row binds TIGHTER than the one above)
  ──────  ─────────────────────────────────
   0      ??                                 ← loosest (groups last / outermost)
   1      ||   or
   2      &&   and
   3      ==   !=   <   <=   >   >=          (comparison — NON-associative; no chaining)
   4      |                                  (bitwise or)
   5      ^                                  (bitwise xor)
   6      &                                  (bitwise AND — infix `&`, see below)
   7      <<   >>                            (shifts)
   8      +    -                             (additive)
   9      *    /    %                        (multiplicative)
  10      **                                 (power — RIGHT-associative; all others left)
  11      as                                 ← tightest (cast; groups first / innermost)
```

```
  (G-Prec)   for binary operators, level N binds tighter than level N-1.  Parsing is
             precedence-climbing: `parse_operators(p)` parses operands at level p+1, so a
             higher level groups deeper.
  (G-Assoc)  binary levels are LEFT-associative, EXCEPT `**` (power = RIGHT-assoc), `??`
             (default-fallback = RIGHT-assoc) and the comparison level 3 (NON-associative —
             chaining is rejected):
                 2 ** 3 ** 2  ==  2 ** (3 ** 2)  ==  512       (matches maths / most languages)
                 x ?? y ?? d  ==  x ?? (y ?? d)                 (same VALUE either way; see below)
                 x as integer as float  ==  (x as integer) as float   (`as` stays left-assoc)
                 a == b == c   → COMPILE ERROR (would be `(a == b) == c`, a bool vs c compare;
                                parenthesise, or use `&&` for a range: `1 < x && x < 10`)
```

**In words.** loft has one precedence ladder, twelve rungs. The closer to the bottom
(`as`, `**`, `*`), the tighter an operator grips its operands; the closer to the top
(`??`, `||`), the later it groups. Operators group left-to-right, with **three exceptions**:
`**` (power) is **right**-associative, so `2 ** 3 ** 2` is `2 ** (3 ** 2) = 512` — matching
maths and most languages (it was left-associative = 64 before; the maker should not carry a
surprise there); `??` is **right**-associative; and the **comparison** level is
**non-associative** — `a == b == c` / `1 < x < 10` are rejected at parse time, because
left-associative grouping would silently compare a boolean to the third operand.
Parenthesise, or use `&&` for a range test.

`??`'s grouping is the one exception a program cannot OBSERVE as a value: the operator is
associative, and either grouping answers the first present operand, evaluating the operands
left to right and stopping there. What it changes is the SHAPE the rest of the language then
reads. Left-associated, `x ?? y ?? d` is `(x ?? y) ?? d`, whose subject is not a variable and
is therefore hoisted into a compiler temp — and that temp VIEWS the operand it chose, so the
destination bound to it shared that operand's store where `(B-Copy)` gives it a copy
(loft#1612, `binding.md` D-bind-50). Right-associated, every operand is an ARM of a plain
`if`, and an arm's bind is the copy — which is why the two-operand spelling was always right.
`LOFT_COALESCE_LEFT_ASSOC=1` parses the left-associated form again, as the bisect step.

### Postfix `?` is NOT a rung on the ladder — it is a primary suffix (@PLN116)

```
  (G-Post-Default)  postfix `?` (the default-fallback, types.md `(N-Default)`) is a SUFFIX on
                    a PRIMARY expression, not a level in the table above.  It binds TIGHTER
                    than level 11 (`as`) and than every binary operator, alongside `.` and
                    `[]`, and chains with them left-to-right:
                        a.b?           ≡  (a.b)?
                        points[i]?.x   ≡  ((points[i])?).x        discharge, THEN read
                        x? as T        ≡  (x?) as T
                        a + b?         ≡  a + (b?)
  (G-Lex-Coal)      `??` is lexed by MAXIMAL MUNCH, so a `??` is never two `?` tokens:
                        a ?? b?        ≡  a ?? (b?)               (never `a ? (? b?)`)
```

**In words.** `?` and `??` look like a pair but sit at **opposite ends** of the ladder: `??`
is the loosest thing in the language (level 0) and `?` is tighter than the tightest rung.
That is deliberate — `??` takes a whole expression on its left, while `?` is a suffix that
grabs only the primary it is stuck to, exactly like `.` and `[]`.

The consequence is the one trap in the pair: **on a binary result they need opposite
parenthesisation.** `a / b ?? 0` discharges the division and needs no parens; `a / b?` does
*not* discharge the division — it is `a / (b?)`, so the division is still undischarged and
the whole expression is null. The discharging form is `(a / b)?`. So `?` is a strict
improvement over `?? <default>` on a **primary** (`row.label?`, `points[i]?`, `x?` — no
parens, strictly shorter) and a *wash* on a binary result, where it costs the parens `??`
does not need. Anything steering a maker from `??` to `?` must respect that split.

**Falsifying program** (obeying `(G-Post-Default)` and reading `?`/`??` as symmetric disagree):

```loft
a = 10;  b = 0;
a / b ?? 0      // 0
(a / b)?        // 0
a / b?          // null   ← reading `?` as "loose like ??" predicts 0
```

### Prefix `&` is NOT an operator — it is the reference annotation

```
  (G-Amp-Infix)   an INFIX `&` is the bitwise-AND operator (level 6): `a & b`.
  (G-Amp-Prefix)  a PREFIX `&` is the reference-type annotation, parsed in the primary
                  expression — NOT a unary operator.  `&a` at a binding makes the bound
                  variable a link (see [binding.md](binding.md)); a prefix `&` anywhere
                  else is a parse error (binding.md B-Ref-AnnotationOnly), with the one
                  exception that a `reference<τ>` FIELD takes it (B-Ref-StoredRef).
  (G-Amp-Disambig) the two are told apart by POSITION: a `&` with a left operand is
                  infix bitwise-and; a leading `&` is the reference annotation.  "Leading"
                  means leading the whole BINDING, not merely leading an operand: in
                  `b = 1 + &a` the `&` leads an operand and is still rejected, because a
                  prefix `&` is legal only where it annotates a binding.
```

**In words.** `&` does double duty. Between two values (`a & b`) it is bitwise-and, an
ordinary operator at level 6 (looser than `+`, so `1 + 2 & 3` is `(1+2) & 3`). At the
*front* of a binding (`b = &a`) it is the reference annotation from `binding.md`, and it
is the only place a leading `&` is allowed. The parser disambiguates purely by position.

### Pattern-operator precedence (@PLN35, SHIPPED)

> **@PLN35 · SHIPPED.** This ladder was written spec-first, ahead of the code; the code landed
> with phases 1–7 + PC1–PC5 ([matching.md § Rules — PEG patterns](matching.md)) and parses these
> groupings — verified both backends. The pattern
> *productions* live in [LOFT.md § Summary of grammar](../LOFT.md); this pins their operator
> precedence, the one fact that lives only in the parser. Design:
> [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

A pattern has its own small precedence ladder (separate from the expression ladder above), loosest
first:

```
  pattern level   form                              binds
  ─────────────   ───────────────────────────────   ──────────────────────────────────
   0  ` , `        multi-pattern arm separator        (loosest — separates whole alternative shapes)
   1  ` | `        alternation inside a group
   2  sequence     juxtaposition inside `[ … ]`
   3  ` : `        capture  name:pat
   4  postfix       `?`  `*`  `+`  `*(sep)`           (tightest — bind to the nearest pattern)
      prefix        `..name`  (rest — only as a slice tail)
```

```
  (G-Pat-Prec)   for pattern operators, level N binds tighter than level N-1: `a:V | b:W` is
                 `(a:V) | (b:W)`; `x:p*` is `x:(p*)`.
  (G-Pat-Group)  a `(…)` inside a pattern is an alternation / optional / repetition GROUP, told apart
                 from a tuple pattern by its operators (`|`, `?`, `*`, `+`) — the same speculative
                 parse the surface already uses (a DECIDED EDGE like D-gram-2; no CFG is owed,
                 tooling reuses the hand-written parser).
```

**In words.** Inside a pattern, `,` (separating whole alternative shapes in a multi-pattern arm) is
loosest, then `|` (ordered choice), then a sequence of sub-patterns, then `:` (a capture); the
postfix quantifiers `? * +` bind tightest to the pattern right before them — with **no** parens
around a **single** element (`Num { value: n }*`, `Ident { name }?`), so `( … )` groups only a
multi-element sequence or an alternation (`( Colon, Ident { name: ty } )?`). `..name` is a prefix
rest, allowed only as the last element of a slice. A parenthesised pattern is a *group* (not a
tuple) when it carries a pattern operator — resolved by the same position-based, deliberately
non-context-free parse this doc already documents for `&` and struct-vs-block.

---

## Deviations

**OPEN: 0.**  Every deviation this doc has carried is closed; the record is in
the companion [grammar-history.md](grammar-history.md).

## Conformance

The precedence rules are checkable by evaluation: `1 + 2 & 3 == 3` (additive tighter than
bitwise-and), `2 ** 3 ** 2 == 512` (RIGHT-assoc `**`; guarded by
`tests/issues.rs::power_is_right_associative`), `x as integer as float` (left-assoc `as`). The
two decided edges have no falsifier *to drive to zero* — they are accepted shapes: the
positional `&` rule is enforced by binding.md's `pln87_amp_*` parse-error tests, and the
non-CFG surface is the parser's own behaviour (COMPILER.md documents the disambiguation points).
