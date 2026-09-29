
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# @PLN35 — PEG-Style Match Patterns with Anchor-Revert Captures (L3)

> **Status: SHIPPED (phases 1–7 + PC1–PC5; #554/#558/#561/#562).**  The formal spec
> ([formal/matching.md](../../formal/matching.md), [formal/VERIFICATION.md](../../formal/VERIFICATION.md))
> is reconciled to SHIPPED; every PEG rule (P-Seq/Alt/Opt/Rep/Cap/Rest/Multi/Atomic) is
> verified on both backends via `tests/scripts/35*.loft`.  Extends the base
> match syntax ([LOFT_CONTROL.md](../../LOFT.md) § Match expressions) with sequence patterns,
> alternation, optionals, repetition, and multi-variable capture.  Backtracking is
> modelled on the existing `Lexer::link()` / `revert()` anchor mechanic so a
> partially-matched branch can be cleanly undone.  **Build plan:**
> [IMPLEMENTATION.md](IMPLEMENTATION.md).  **Strict-spec changes:**
> [FORMAL-DESIGN.md](FORMAL-DESIGN.md).  The D2 architecture bet (slice backtracking
> needs no new opcode) was CONFIRMED on both backends (`probes/p0-d2-backtrack.loft`) and
> carried through all phases.

---

## Reconciliation with loft reality (read before the design below)

The design in this doc was drafted against an idealized syntax; a full read of the current
`match` implementation ([IMPLEMENTATION.md § 1](IMPLEMENTATION.md) has the code anchors)
corrects several assumptions.  The examples further down use the idealized shapes — the
**real** loft forms are:

- **No tuple-style enum variants.**  loft has only STRUCT variants: write `Num { v: integer }`,
  not `Num(integer)`; `Neg { inner: … }`, not `Neg(Expr)`.  Every `Num(n)` / `Ident(name)`
  example below is shorthand for `Num { v: n }` / `Ident { name }`.  (Decision **D5** /
  [C89](../../DESIGN_DECISIONS.md): tuple variants are a PERMANENT non-goal — they force
  match-to-read plus a mitigation-syntax sprawl; loft reads enum payloads by name.)
- **Recursion goes through a collection, not an inline field.**  `Neg { inner: Expr }` fails
  type-layout (an inline self-reference has no size); an AST enum uses
  `Neg { inner: vector<Expr> }` (or a reference), as `445-generic-tree-walk.loft` models a tree
  with `vector<Node>`.
- **`match` has no IR node — it desugars entirely in the parser** into `If`-chains + ops.  So
  L2–L3.4 are parser-only work and both backends come for free; only iterator input (L3.6) adds
  opcodes.
- **Slice backtracking needs no new opcode** — a slice cursor is a `usize` index; anchor/revert
  = save/restore it.  **CONFIRMED** by the Phase-0 D2 probe on both backends.
- **Already shipped, reused as-is:** fixed-arity slice patterns `[a, b, c]` and single-level
  variant binding `V { field }`.  **Not yet implemented (this plan builds them):** nested
  patterns `V { f: Inner { x } }` (L2), named / `..rest` / sequence captures (L3.1), and the
  PEG operators `| ? * +`.

The formal contract for all of the above — and the invariant it must preserve (`M-Total`: a
match with any non-total pattern needs a total final arm, so a match never fails to select at
runtime) — is [FORMAL-DESIGN.md](FORMAL-DESIGN.md); the five load-bearing decisions **D1–D5**
are pinned there.

---

## Motivation

Current `match` arms are *point* patterns: one shape, a few bindings, a
guard.  Writing a parser, a token dispatcher, a protocol decoder, or a
structured-log matcher requires expanding a single "logical" pattern into
many arms or into imperative code that shadows what the language should be
able to say directly.

The target is for an arm to read like a canonical parser expression — a
sequence of sub-patterns, each of which may capture — while staying inside
`match`'s exhaustiveness and type-unification story:

```loft
match tokens {
    [ Let, Ident(name), Eq, expr:expr, Semi ]
        => bind(name, expr),

    [ If, cond:expr, Then, body:block, (Else else_body:block)? ]
        => if_node(cond, body, else_body),

    [ Fn, Ident(name), LParen, (params:ident_list)?, RParen, body:block ]
        => fn_node(name, params ?? [], body),

    _ => error("unrecognised form"),
}
```

Text matching is **not** handled by this extension.  Rich text patterns
go through the [REGEX.md](../../lib_plans/57-regex/README.md) library and integrate with `match`
by destructuring the returned `Match` struct — the existing match path,
no new syntax.  Keeping text out of this design is an explicit choice:
one text-pattern language (regex, in a library) is easier to learn than
two, and the library can offer PCRE-class features without bloating the
compiler.

The arms here bind *multiple* sub-results and each sub-pattern is itself
a pattern.  That capability applies to vectors, enum shapes, and
iterators only.

---

## Design principle: mirror `Lexer::link` / `revert`

The loft parser already solves this for token streams.  `lexer.link()`
returns a reference-counted anchor that keeps scanned tokens live in
`Lexer::memory`; `lexer.revert(link)` rewinds the cursor to that point and
drops the anchor.  Speculative parsing in `parser/objects.rs`,
`control.rs`, and `expressions.rs` is all written against this primitive.

**The proposal re-uses the same shape at match time**, generalised from
tokens to any forward-iterable input (vector slice, iterator, string):

| Lexer primitive                 | Match-pattern analogue                              |
|---------------------------------|-----------------------------------------------------|
| `lexer.link()`                  | `MatchCursor::anchor()` — snapshot position + bindings |
| `lexer.revert(link)`            | `MatchCursor::revert(anchor)`                       |
| `Lexer::memory` (ringed tokens) | Provisional-bindings slot block, scoped to anchor   |
| `links` refcount                | Nested-alternative depth — drives when to free      |

A branch that begins with an anchor and fails at any sub-pattern reverts
atomically: cursor position, provisionally bound variables, and any
intermediate state created by sub-patterns all roll back.

---

## Syntax

### Sequence pattern

Inside a slice / iterator match, a comma-separated list of sub-patterns
must each succeed in order.  A trailing `name:sub_pat` binds the result
of the sub-pattern to `name`.

```loft
[ Let, Ident(name), Eq, value:expr, Semi ]
```

### Alternation with capture

Parentheses plus `|` introduce an ordered choice.  If every alternative
binds the *same* name at the *same* type, the capture is promoted out of
the group:

```loft
( Ident(n) | Str(n) | Num(n) )         // all three bind `n` at its variant type
```

Capture positions hold *only* a name (or a sub-pattern) — no inline
transforms.  If a conversion is needed, write it in the arm body.

If alternatives bind different names, the union becomes nullable per-name:

```loft
( Add(x, y) | Neg(x) )    // x: i64, y: option<i64>
```

### Optional

`(... )?` attempts the group; on failure reverts and yields `null` /
`option<T>` for any captures inside:

```loft
( Else body:block )?      // body: option<block>
```

### Repetition

A capture followed by `*` or `+` collects into a vector.  Each iteration
runs under its own anchor so the final partial iteration (which caused
the loop to stop) reverts cleanly.  A separator may be supplied in
parentheses after the quantifier — the separator is consumed but not
captured.

```loft
[ LParen, (args:expr*(Comma))?, RParen ]     // args: vec<expr>
[ (items:line+)? ]                           // items: vec<line>, at least one
```

Each repetition reuses a *single* name; the previous "reuse the same
name twice in a sequence" shorthand is dropped because it hid the
aggregation rule.  If a first element needs distinct treatment from the
tail, use two names:

```loft
[ first:expr (Comma rest:expr)* ]            // first:expr, rest:vec<expr>
```

### Named capture on any sub-pattern

`name:<sub_pattern>` binds whatever the sub-pattern produces.  One sigil
per job:

| Form                | Meaning                                      |
|---------------------|----------------------------------------------|
| `name`              | single-element capture                       |
| `name:pat`          | sub-pattern, `name` bound to its result      |
| `..name`           | slice tail                                   |
| `name:pat*`         | repetition capture into vec                  |
| `name:pat+`         | repetition, at least one                     |
| `name:pat*(sep)`    | repetition with separator                    |

Type assertion is expressed through the sub-pattern form (`name:is<T>`
or a dedicated type pattern) or via a guard — not through a dedicated
capture sigil.

### Multi-pattern arm (cross-shape alternation)

A single arm may list several patterns separated by commas (line breaks
allowed).  The arm body runs when *any* pattern matches; captures from
the matching pattern are bound.  Use this when the same logical concept
arrives in more than one shape and a single-line `|` or-pattern cannot
unify the shapes:

```loft
match input {
    [ Verb(v), Obj(o) ],
    Parsed { verb: v, object: o }
        => dispatch(v, o),
}
```

Mixing a regex match into the same arm is done by compiling the regex
once and destructuring its result struct alongside the structural
patterns:

```loft
re_cmd = regex("^(\w+) (\w+)$")
match input {
    [ Verb(v), Obj(o) ],
    Parsed { verb: v, object: o },
    Raw(s) if (m := re_cmd.match(s)) != null => dispatch(m.group(1), m.group(2)),
    _ => fallback(),
}
```

Binding rules:

- Names present in every pattern with a common type are bound at that
  type in the arm body.
- Names present in only some patterns become `option<T>` (same rule as
  existing alternation).
- Each pattern is attempted in order under its own anchor; the first
  that matches commits, later patterns in the arm are not tried.
- Exhaustiveness treats the arm as non-total if any listed pattern is
  non-total — the same rule as guarded and repetition arms.

`|` inside a group remains the single-line same-shape alternation (it
reads tighter).  The comma-separated multi-pattern form is reserved for
the cross-shape case.

---

## Semantics — forward tracking with anchors

A match pattern is compiled to a small state machine over a `MatchCursor`:

```text
struct MatchCursor<'a> {
    input:     &'a [Value] | Iter<Value>,
    pos:       usize,
    bindings:  SlotBlock,          // provisional captures
    anchors:   Vec<Anchor>,        // stack of saved (pos, bindings_epoch)
}
```

The compiler lowers the five PEG-style operators to cursor ops:

| Operator        | Lowering                                                                 |
|-----------------|-------------------------------------------------------------------------|
| `a, b, c`       | run `a`, then `b`, then `c`; no anchor needed at the seq level itself.  |
| `(a \| b \| c)` | `anchor = cursor.anchor()`; try `a`; on fail `cursor.revert(anchor)`, try `b`; etc.  Commit = drop anchor. |
| `(a)?`          | `anchor = cursor.anchor()`; try `a`; on fail revert and bind captures to `null`. |
| `(a)*`          | loop: `anchor`, try `a`; on fail revert and break; on success push captures and continue. |
| `(a)+`          | one mandatory run of `a` followed by `(a)*`.                            |

Sub-patterns themselves — enum-variant match, scalar match, range,
nested struct — already have fail/succeed semantics in today's compiler.
The new layer is purely the *anchor stack* around them.

### Matching terminology

| Term              | Meaning                                           |
|-------------------|---------------------------------------------------|
| **Anchor**        | Saved `(pos, bindings_epoch)` checkpoint.         |
| **Commit**        | Drop the topmost anchor.  Captures become real.   |
| **Revert**        | Restore cursor and rewind bindings to the anchor. |
| **Bindings epoch**| Monotonic counter; rewinding drops slots ≥ epoch. |

A branch *commits* when it reaches the `=>` arrow without failing; only
then is the arm body entered with the full binding set visible.  No
partially-committed state is ever observable by user code.

### Bindings epoch and slot reuse

Every provisional capture is stored in a **reset-region** of the match
arm's slot block, much as `Lexer::memory` holds tokens only while links
are live.  On `revert`, slots written after the anchor's epoch are
logically cleared.  On commit, they graduate to the arm's normal slot
range.  Because alternatives at the same nesting level share the same
reset-region, there is no slot blow-up for wide alternations.

### Exhaustiveness

Sequence and repetition patterns never participate in exhaustiveness
checks — they are inherently open-ended.  Alternation participates only
when every alternative is itself an exhaustive form (e.g. a closed enum
variant set).  In practice this means an L3 arm that uses any sequence
or repetition operator must be followed by a wildcard arm; the
exhaustiveness checker treats the arm as non-total.

This is the same rule that already applies to guarded arms today and is
captured in [INCONSISTENCIES.md](../../INCONSISTENCIES.md) #26.

---

## Validated input shapes

The anchor mechanism is orthogonal to input shape.  The cursor only has
to satisfy three primitives:

1. `pos()` — a cheap, copyable position token.
2. `revert(p)` — after return, `peek`/`next` produce the same values as
   before the anchor.
3. Items consumed between anchor and revert must not cause side effects
   observable to user code.

Everything else — bindings-epoch rewind, slot-block rollback, arm
commit — lives on the match arm and is shape-independent.

| Shape | `pos()` | Revert cost | Memo needed | Notes / constraints |
|---|---|---|---|---|
| Vector / slice (`vec<T>`, `[Value]`) | `usize` index | 1 word | none | Slice is its own memory.  Simplest case; targeted by L3.1. |
| Iterator (`iterator<T>`, coroutine, channel) | buffer index | 1 word | yes — grows under anchors | Mirrors `Lexer::memory` + `links` refcount: pulled items stay while any anchor is live; buffer clears when anchor stack empties.  Iterator must be pure w.r.t. match — same rule Lexer applies to its token stream.  Targeted by L3.6. |

Bindings rollback (the slot-block epoch) is identical across both
shapes; only the cursor's `pos`/`peek`/`next` differ.  This is the same
abstraction split `Lexer` has today between `Lexer::memory` (shape-
specific) and the `links` refcount (shape-independent).

Text is deliberately absent from this table — it is served by the
[REGEX.md](../../lib_plans/57-regex/README.md) library, not by this extension.  A `vec<char>`
or a `char` iterator can still be matched as a vector/iterator shape
if someone wants structural character patterns, but that's a niche use
and regex is the intended answer.

### What would *not* work

- **Side-effecting iterators** whose pull itself mutates external state
  (e.g. a generator that fires a network call per item): revert cannot
  undo the side effect.  Match must document this, mirroring the same
  assumption Lexer makes about its input.
- **Unbounded-lookahead repetition on an infinite iterator** with a
  pattern that always matches: never terminates.  Same failure mode as
  any PEG; the pattern must have a terminating failure case.
- **Grapheme-cluster–level text matching**: current cursor is codepoint-
  indexed.  A grapheme-cluster cursor is a separate variant, deferred
  until a concrete need appears.

---

## Minimal examples — one per input shape

The two examples below use the *same* two opcodes
(`OpMatchAnchor`, `OpMatchRevert`).  Only the cursor underneath
differs — slice index or enum/field descent — which is the concrete
payoff of the shape-independence claim above.  A third example using
an iterator is deferred to the L3.6 phase.

### Numeric vector

```loft
// cmd: vec<i64>
// arm 0: literal 0, then one required i64
// arm 1: literal 1, then zero-or-more i64 collected into a vec
// arm 2: alternation of two literals, then two required i64
msg = match cmd {
    [ 0, value:i64 ]                   => "set " + text(value),
    [ 1, (arg:i64)* ]                  => "call/" + text(len(arg)),
    [ 2 | 3, x:i64, y:i64 ]            => "move " + text(x) + "," + text(y),
    _                                  => "unknown",
}
```

Anchor behaviour: each arm opens an anchor at pos 0.  If arm 0 matches
the `0` literal but the next element isn't an `i64`, `revert` rewinds to
pos 0 and the runtime falls through to arm 1.  Inside `(arg:i64)*`, each
iteration runs under its own anchor — the first non-`i64` element
reverts that iteration and exits the loop with whatever was already
collected.

### Enum struct (AST walk)

```loft
enum Expr {
    Num(i64),
    Neg(Expr),
    Add  { left:Expr, right:Expr },
    Call { name:text, args:vec<Expr> },
}

label = match e {
    Num(0)                                          => "zero",
    Neg(Num(n))                                     => "neg " + text(n),
    Add { left: Num(a), right: Num(b) }             => text(a + b),
    Call { name, args: [only:Expr] }                => name + "(1)",
    Call { name, args: [first:Expr, (rest:Expr)*] } => name + "(" + text(1 + len(rest)) + ")",
    _                                               => "other",
}
```

Anchor behaviour: nested patterns (`Num(n)` inside `Neg`, `Num(a)` /
`Num(b)` inside `Add` fields) reuse the same mechanism.  Each nested
sub-pattern opens an anchor on its own cursor — enum-variant tag cursor
for the outer shape, slice cursor for the `args` vector.  The two
`Call` arms differ only in their `args` length; the single-element
arm's anchor reverts if `args` has more than one element, so the
second `Call` arm gets its chance.

### Text (not in this design — via REGEX library)

The equivalent of the two examples above for text input is served by
the standalone regex library:

```loft
re = regex("^HTTP/(\d)\.(\d)$")
match re.match(line) {
    Some(m) => "http " + m.group(1) + "." + m.group(2),
    None    => "?",
}
```

Rich text matching (custom char classes, anchors, lookaround, non-
greedy, named groups, Unicode properties) all goes through the library.
See [REGEX.md](../../lib_plans/57-regex/README.md).

---

## Implementation phases

| Phase | Scope                                                                 |
|-------|-----------------------------------------------------------------------|
| **L3.1** | Sequence patterns on slice / vector input; named captures; L2 nested enum field patterns already required.  No alternation or optional. |
| **L3.2** | Alternation `(a \| b)` with anchor/revert; per-alternative capture unification. |
| **L3.3** | Optional `(...)?`; nullable promotion of captures.                |
| **L3.4** | Repetition `(...)*` and `(...)+`; per-iteration anchors; vector capture. |
| ~~**L3.5**~~ | ~~String-shaped patterns (backtick template).~~  **Withdrawn** — text matching goes through [REGEX.md](../../lib_plans/57-regex/README.md) instead, avoiding two text-pattern languages in the codebase. |
| **L3.6** | Iterator inputs (`match some_iter { ... }`) — anchors must spill to memory like `Lexer::memory`; bounded by a `max_lookahead` arm attribute. |
| **L3.7** | Multi-pattern arms (comma-separated patterns per arm).  Purely additive — no new cursor work; each listed pattern compiles as today, with the first-match commit wired into arm dispatch. |

Phases are strictly additive.  L3.1 alone already delivers the
"parser-rule-looking arm" without any backtracking.

### Ship order — across MATCH_PEG and REGEX

The recommended order of implementation, combining this design with
[REGEX.md](../../lib_plans/57-regex/README.md), places text capability first because it is the
smaller, library-scoped change with the highest immediate payoff:

| Step | Item | Why here |
|---|---|---|
| 1 | REGEX R1 (linear NFA, basic features) | Library-only; validates the lazy-loading mechanism from LAZY_STDLIB.md; immediate value for CLI / server / log work. |
| 2 | **L3.1** (sequence over slice/vec + named captures) | Highest-value structural extension; unlocks AST work. |
| 3 | **L3.7** (multi-pattern arm) | Purely additive, low cost; helps long arm lists read cleanly. |
| 4 | **L3.2 → L3.3 → L3.4** (alternation, optional, repetition) | Incremental; each useful on its own; shared anchor infrastructure with L3.1. |
| 5 | REGEX R2–R4 (named groups, Unicode properties, backtracking fallback) | As demand appears. |
| 6 | **L3.6** (iterator inputs) | Last because it is the most complex anchor/spill case, not because of any dependency — `iterator<T>` and coroutines are already shipped in both backends. |

Steps 1+2 alone deliver most of the user-visible value — the rest is
strictly optional and can be picked up as specific needs emerge.

**Parallel library evolution** — independent of compiler phases, can
ship at any time (see [Integration with `lib/lexer.loft`](#integration-with-liblexerloft)):

| Item | When it unlocks | Gated by |
|---|---|---|
| `Token` enum + `tokenise()` in `lib/lexer.loft` | Lexer integration Path A (tokenise-to-vec). | Nothing — ordinary library commit.  Lands alongside or before L3.1. |
| `tokens() -> iterator<Token>` in `lib/lexer.loft` | Lexer integration Path B (streaming). | L3.6 (coroutines already available, so no upstream gate). |
| `Iterator.cheap_revert()` protocol hook | L3.6 optimisation — avoids memo buffer for sources with native revert (lexer being the motivating case). | L3.6 exists and profiling shows the buffer matters. |

### Code-generation notes

- Anchor and revert compile to two new opcodes (`OpMatchAnchor`,
  `OpMatchRevert`).  Both take a slot-block index and push/pop on an
  anchor stack that lives on the `State::stack`.
- The bindings epoch is a u16 in the arm's slot block header; `revert`
  writes it back atomically.
- Sub-pattern compilation re-uses existing `generate_pattern_*` helpers
  from `state/codegen.rs`.  The L3 layer only wires failure jumps to
  `OpMatchRevert` rather than to the next arm.
- For repetition, the compiler emits a while-loop in bytecode; the loop
  header owns the anchor and restores on a failed iteration.

### Runtime overhead

Arms that use **no** L3 operators pay zero cost — the existing match
path is unchanged.  An arm with `k` alternation points pays
`O(k)` anchor pushes per attempt, each of which is two word writes
(pos, epoch).  Repetition is `O(iterations)` anchor pushes, but all
cheap; amortised cost is one word write per successful capture plus
one revert per loop exit.

---

## Interaction with existing features

| Existing feature                    | Interaction                                                                 |
|------------------------------------|------------------------------------------------------------------------------|
| L2 nested enum patterns            | Required prerequisite — sub-patterns inside seq must already work on fields. See PLANNING.md § L2. |
| Slice/vector patterns              | L3 sequence *is* the generalised slice pattern; `[a, b, ..rest]` compiles identically. |
| Tuple destructure ([TUPLES.md](../../TUPLES.md)) | Tuples are a fixed-arity sequence — a degenerate L3.1 case.                |
| Guards (`if` on arm)               | Guard runs *after* L3 captures are committed; failure is not revertable.   |
| Coroutines ([COROUTINE.md](../../COROUTINE.md)) | Already shipped (0.8.3), both interpreter and native (state-machine lowering).  Matching on an `iterator<T>` value uses L3.6's memoised cursor; no dependency blocker. |
| Regex library ([REGEX.md](../../lib_plans/57-regex/README.md)) | Complement — handles all text matching; returns a `Match` struct that destructures in arms here via existing struct-pattern support. |
| `lib/lexer.loft` (loft-level lexer) | Natural consumer.  Its `anchor()`/`revert(Anchor)` pair already matches the cursor contract.  Three integration paths — see below. |

---

## Integration with `lib/lexer.loft`

The loft-level lexer library in `lib/lexer.loft` already exposes the
cursor contract this design is modelled on: `anchor() -> Anchor` and
`revert(to: Anchor)` (lines 464 and 468 of that file) restore the
lexer's `{index, line, pos}` state atomically.  Typed probes
(`int()`, `identifier()`, `long_int()`, `get_float()`,
`constant_text()`, `matches(token)`) advance on success and return
`null` on failure.  That is exactly the shape an L3 sub-pattern
needs.

Three integration paths, in order of increasing compiler dependency:

| Path | Mechanism | Requires |
|---|---|---|
| **A — Tokenise to `vec<Token>`** | User calls `tokenise(lex)` once; match over the resulting vector. | L3.1 only. **Works today as soon as L3.1 ships.** |
| **B — Lexer as `iterator<Token>`** | `tokens() -> iterator<Token>` adapter; match streams through. | L3.6 only (coroutines / `iterator<T>` are already shipped in 0.8.3). |
| **C — Lexer as cursor protocol** | Match compiler invokes `lex.anchor()` / `lex.revert()` and uses typed probes as sub-patterns. | New design — **not recommended**.  Couples match to a specific object interface with no capability gain over Path B. |

Path A is the bootstrap: zero compiler work once L3.1 lands, covers
the vast majority of parser-style use cases, loses only streaming for
very large inputs.  Path B adds streaming once L3.6 is implemented
(`iterator<T>` itself is already available — coroutines shipped in
0.8.3, interpreter and native, so there is no upstream gate).  Path C
is listed for completeness; skipped.

### Library enhancements to `lib/lexer.loft`

Independent of MATCH_PEG phases — ordinary library evolution:

1. **Canonical `Token` enum.**  Add a public `Token` enum (`Ident(text)`,
   `Int(integer)`, `Long(long)`, `Float(float)`, `Str(text)`,
   `Char(character)`, `Punct(text)`, `Keyword(text)`, `Eof`) to
   `lib/lexer.loft`.  Gives all consumers a shared vocabulary instead
   of each defining their own.
2. **`tokenise(self: Lexer) -> vec<Token>`.**  Drain loop that calls
   `scan()` and emits one `Token` per step.  Enables Path A.
3. **`tokens(self: Lexer) -> iterator<Token>`.**  Streaming variant.
   Ships as `vec<Token>` alias until coroutines land, then upgraded to
   a real iterator — callers that write `for t in lex.tokens()`
   continue to work.

Scope: a single library commit, no language changes.  Unlocks Path A
immediately.

### Iterator-protocol enhancement (optional, for L3.6)

When an iterator wraps a source that already has cheap revert (the
lexer does), the memo-buffer mechanism described in [Validated input
shapes](#validated-input-shapes) is unnecessary — reverts can delegate
to the underlying source.

Optional addition to the iterator protocol:

```
trait Iterator<T> {
    fn next(self) -> option<T>
    fn cheap_revert(self) -> option<RevertHandle>  // default: null
}
```

If `cheap_revert` returns a handle, L3.6 uses it instead of buffering;
on a cursor `anchor`, it stores the handle; on `revert`, it passes the
handle back to the iterator to restore its internal position.  The
lexer's iterator adapter returns a handle wrapping `lex.anchor()`; its
`revert(handle)` calls `lex.revert(handle.anchor)`.

Pure optimisation — L3.6 works correctly without it, just with a
memo buffer.  Worth revisiting once L3.6 is on the critical path.

---

## Open questions

1. **Commit-points inside alternation.**  PEG normally has `~` / `!~` for
   "after this point, don't backtrack past here".  Do we expose it, or
   rely on the compiler inferring no-backtrack past the first committing
   token?  Proposal: start without explicit commit points; revisit if
   error messages from deep revert become unhelpful.

2. **Left-recursive patterns.**  Classical PEGs forbid them.  Loft match
   arms are not self-referential, so this does not arise for L3.1–L3.4.
   For L3.6 (iterator inputs) the same restriction applies — a pattern
   cannot call itself.

3. **Error reporting on partial match.**  When every arm fails, the
   user wants to see *where the longest partial match broke*.  The
   anchor stack naturally records this — the furthest `pos` reached
   across all reverts — so the runtime error can point to the exact
   token / character.  Design target for L3.2.

4. **Type of a captured sub-pattern.**  Inferred from the sub-pattern's
   result type.  Alternations must unify; if not, either promote to
   `option<T>` (per-alternative names) or raise a compile error
   (same-name, different types).

5. **Relationship to the regex library.**  All text matching goes
   through the standalone regex library — see [REGEX.md](../../lib_plans/57-regex/README.md).
   The two systems have disjoint domains: this extension handles
   structural sequences over vectors / enums / iterators; regex
   handles text.  They meet in user code when a regex `Match` result
   is destructured by an arm of a regular (or L3) match.

---

## See also

- [LOFT_CONTROL.md](../../LOFT.md) § Match expressions — base match syntax reference.
- [REGEX.md](../../lib_plans/57-regex/README.md) — standalone regex library for rich text matching;
  the intentional complement to this PEG extension.
- [TUPLES.md](../../TUPLES.md) — fixed-arity sequence captures.
- [PLANNING.md](../../PLANNING.md) — L2 nested-pattern backlog (prerequisite).
- [COROUTINE.md](../../COROUTINE.md) — iterator-valued inputs (L3.6).
- [LAZY_STDLIB.md](../../lib_plans/59-lazy-stdlib/README.md) — lazy-loading mechanism (regex is its
  first consumer).
- [COMPILER.md](../../COMPILER.md) § pattern lowering — where L3 compiles in.
- [INTERMEDIATE.md](../../INTERMEDIATE.md) — `OpMatchAnchor` / `OpMatchRevert` addition.
- [INCONSISTENCIES.md](../../INCONSISTENCIES.md) #26 — guarded-arm exhaustiveness precedent.
- `src/lexer.rs` § `link` / `revert` — the primitive this design mirrors.
- `lib/lexer.loft` § `anchor` / `revert` — loft-level lexer library that
  already exposes the same cursor contract; see
  [Integration with `lib/lexer.loft`](#integration-with-liblexerloft)
  above.
