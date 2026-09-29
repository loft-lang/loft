<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/grammar-history.md — the deviation register for [grammar.md](grammar.md)

> **The rules are next door.**  [grammar.md](grammar.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — (was 4). D-gram-3 (`**` right-assoc) + D-gram-1 (precedence-doc) closed in code/
doc; D-gram-2 + D-gram-4 resolved as **decided edges** (the spec-may-adjust outcome the roadmap
predicted — they leave formal/ rather than being driven to zero).

> **D-gram-1 (CLOSED) — the written grammar now states precedence + associativity.**
> [LOFT.md § Operators](../LOFT.md#operators) carries the full twelve-level ladder (the stale
> table is fixed: `**` added at level 10, `as` moved to 11) with an explicit associativity
> statement (all left-assoc except `**` right-assoc) and the unary-binds-tightest note
> (`-2 ** 2 == 4`). [LOFT_DECLARATIONS.md § Summary of grammar](../LOFT_DECLARATIONS.md) no longer collapses operators
> into one undefined `op`: it enumerates `binary_op` and cross-references § Operators for the
> grouping. The "read-the-parser tax" (FORMALIZATION.md rough spot #4) is paid down — the two
> user-facing statements pin every expression's shape, matching `OPERATORS` / `parse_operators`.

> **D-gram-2 (RESOLVED — decided edge) — the surface is deliberately not context-free.**
> The speculative backtracking (type-vs-variable, `S { … }`-vs-block) + lexer interpolation
> modes are accepted on purpose: they buy real ergonomics and no consumer needs a CFG (tooling
> reuses the hand-written parser, which IS the spec). Decided, not chased → [DESIGN_DECISIONS.md
> C82](../DESIGN_DECISIONS_SYNTAX.md#c82--lofts-surface-is-deliberately-not-context-free).

> **D-gram-4 (RESOLVED — decided edge) — `&` stays one token, disambiguated by position.**
> Infix `&` is bitwise-and; a *leading* `&` is the reference annotation. Prefix `&` is a parse
> error in every non-binding position, so the positional rule is **total** — like Rust, one `&`
> token is kept. Decided → [DESIGN_DECISIONS.md
> C81](../DESIGN_DECISIONS_SYNTAX.md#c81---stays-one-token-disambiguated-by-position-bitwise-and-vs-reference).
>
> "Total" was claimed here from 2026-07-24 on the strength of A1 (binding.md D-bind-7), which
> closed the bare-statement position only. It became true on 2026-08-09 with binding.md's
> **D-bind-10**: until then a `&` that was the LAST operand of an expression (`b = 1 + &a`,
> `b += &a`, a block-final `1 + &a`, `S { x: &a }`) compiled, because the guard peeked only the
> token AFTER the operand — which proves nothing precedes the `&`. The claim outran the
> enforcement by one half of the question.

## Carried by grammar.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [grammar.md](grammar.md) now states only what is open.

### the status line formal/README.md's area table carried until 2026-09-04

**0 open** — the 12-level precedence ladder written; the prefix-`&`/infix-`&` overload + non-CFG surface resolved as decided edges (C81/C82)

