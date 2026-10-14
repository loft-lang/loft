<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN35 — Sub-rule invocation & the parser-combinator layer (design + steps)

> **The keystone.** The core PEG operators ([IMPLEMENTATION.md](IMPLEMENTATION.md) P1–P7) make a
> `match` recognize a *fixed* shape. This layer lets a pattern element **invoke a named sub-rule**
> — a user function that parses a sub-structure and advances the cursor — which turns the matcher
> into a real recursive-descent grammar. It is governed by [@C89](../../DESIGN_DECISIONS.md) (reads
> like grammar, never forced) and adds one new invariant, **INV-Static** (every parser/grammar
> error is a compile-time diagnostic; a compiling grammar cannot fault at runtime).
>
> Depends on: P4 (backtracking / the cursor). Composes with P7 (iterator input) for streaming.
> Formal home: extends [FORMAL-DESIGN.md](FORMAL-DESIGN.md); this doc is the sub-feature's own
> design + build plan.

---

## 1. The idea in one example

```loft
// grammar:  fn_decl := 'fn' IDENT '(' parameters ')' block
fn parse_fn(cur: &Cursor<Token>) -> option<FnDecl> {
  match cur {
    [ Fn, Ident { name: n }, LParen, params: parameters, RParen, body: block ]
        => FnDecl { name: n, params: params, body: body },
  }
}
```

`parameters` and `block` are **other sub-rule functions**. A function name in a pattern position
reads exactly like a grammar nonterminal — the arm *is* the production. That is the readable-grammar
payoff at full strength, and it revives the draft's `expr:expr` / `body:block` examples (they were
sub-rule references all along).

The two moves that make it work:

1. **`match` can take a `Cursor<T>`, not only a `vector<T>`.** Matching over a cursor consumes a
   *prefix* and advances it (whereas matching over a bare vector is whole-consume, `P-Whole`). A
   sub-rule is just `fn(&Cursor<T>) -> option<R>` whose body is a `match` over the cursor — so rules
   **compose**: each one matches, invokes other rules, and returns.
2. **A pattern element `name: rule` invokes `rule(cur)`**, binds `name` to its result, and threads
   the cursor. The matcher owns anchor/revert, so a later failure rewinds past whatever the sub-rule
   consumed. The caller never sees the plumbing.

---

## 2. The Cursor protocol

```loft
// A cursor over a token stream. The MATCHER owns anchor/revert; a sub-rule only ADVANCES it.
struct Cursor<T> {
  source:   vector<T>,    // a buffered iterator for the streaming case (P7)
  pos:      integer,      // current index
  farthest: integer,      // monotonic high-water mark — for error reporting (§5)
}
```

Contract:

- **peek / next / at_end** — a sub-rule reads with `cur.peek()` (⇒ `T?`) and advances with
  `cur.next()`; `cur.at_end()` is the end test. Reads past the end are `null` (`H-Index`), never a
  fault.
- **anchor / revert are matcher-only.** A sub-rule never reverts; it only advances on success. The
  *matcher* takes an anchor before an alternative / sub-rule and reverts on failure — for a slice
  that is save/restore of `pos` (no new op); for a stream it is the P7 memo buffer. So a failed
  sub-rule leaves the cursor exactly where it was (`INV-Pure`), and the sub-rule author writes no
  rollback code.
- **purity** — a sub-rule must be *pure* (advance the cursor + return a value; no external effect),
  so a reverted attempt is invisible. Enforced statically (§4).

Recommended over the functional alternative (`fn(rest) -> option<Parsed<T>>` returning value +
remaining): the `&Cursor` form mirrors `lib/lexer.loft`'s existing probes (`int()`, `identifier()`,
`constant_text()` — advance-on-success / null-on-failure), needs no per-rule packaging, and puts
rollback in the one place that already does backtracking (the matcher). (Generic `Cursor<T>` needs
loft generic-struct support — see Risks.)

---

## 3. Sub-rule invocation — `name: rule`

At a pattern position, `name: rule` where `rule` resolves to a function of type
`fn(&Cursor<T>) -> option<R>` desugars (parser-side, no new op) to:

```text
  a = cursor.anchor()
  r = rule(cursor)                 // the sub-rule advances the cursor on success
  if r == null { cursor.revert(a); <fail this arm / try next> }
  else         { name = r; drop a; <continue with the next element> }
```

`name` binds at type `R` (the sub-rule's success type), directly usable in the arm body — no
`Some`/`Ok` wrapper to unwrap (the anti-`Ok(a)`, [@C89](../../DESIGN_DECISIONS.md)). A bare `rule`
(no capture) is allowed when you only need it to advance the cursor. This composes with the P-*
operators: `rule*` (repeat a sub-rule), `( rule )?` (optional sub-rule), `a: ruleA | b: ruleB`
(choose a sub-rule).

---

## 4. INV-Static — every parser/grammar error is compile-time

**INV-Static — a PEG match that COMPILES cannot produce a match/parser fault at runtime.** This
extends loft's existing discipline (`M-Total` discharges exhaustiveness statically; `E-Uncomp` makes
÷0/OOB/overflow yield null+continue, never a trap). The failure-mode ledger — every grammar/parser
bug lands parse-side:

| hazard | caught at | mechanism |
|---|---|---|
| sub-rule doesn't fit the cursor protocol | **compile** | signature check: `fn(&Cursor<T>) -> option<R>` |
| capture type mismatch (`name`) | **compile** | existing type inference (`name : R`) |
| non-exhaustive arms | **compile** | `M-Total` (already static) |
| **left recursion** (rule reaches itself without consuming) | **compile** | well-formedness pass (§4.1) |
| **non-consuming repetition** (`p*` where `p` may match empty) | **compile** | well-formedness pass (§4.1) |
| **impure sub-rule** (backtracking can't undo it) | **compile** | purity pass (§4.2) |
| cursor past end / no read | runtime, **safe** | null + continue (`H-Index`) — not a fault |

The two genuinely new static analyses:

### 4.1 Termination / well-formedness pass

Build the **sub-rule reference graph** (each rule = a function whose body is a cursor-`match`;
edges = the sub-rules it references) and compute, per pattern and per rule, a **min-consumption**
property (`consumes ≥ 1` vs `may match empty`). Then reject, at compile time:

- **Left recursion** — a cycle in the graph reachable without an intervening consume
  (`"rule `expr` is left-recursive via `term` → `expr`"`).
- **Non-consuming repetition** — a `p*` / `p+` whose body `p` may match empty (`"repetition body
  may match empty — it would loop"`).

**Analyzability is the key constraint:** min-consumption is derivable when a rule's body is a
cursor-`match` (its shapes tell you what it consumes). For an *opaque* sub-rule (arbitrary body), it
is not — so to hold INV-Static strictly, an opaque sub-rule is allowed in a **consuming position but
NOT under `*` / `+`** (where a wrongly-assumed consume would loop). A runtime progress-guard would
patch this but breaks never-at-runtime, so it is a compile-time refusal instead.

### 4.2 Purity pass

A sub-rule must be pure (advance the cursor + return; no I/O, no external mutation) so a reverted
attempt is unobservable. Reuse loft's **capability / effect analysis** (`capabilities.md`,
`src/sandbox.rs` — the same machinery that gates a sandboxed closure): a sub-rule that reaches a host
effect is a compile error (`"sub-rule `x` is not pure — matching may backtrack over it"`).

---

## 5. Reporting contract (parse errors are values, never faults)

A failed parse takes the `_` arm (or a null sub-rule result) and returns a **named error value**
(`enum ParseResult { Ok { … }, Err { at: Span, msg: text } }` — struct variants, not `Ok/Err`
tuples). Reporting is a collaboration, each part where it belongs:

- **lexer → spans on tokens.** `lib/lexer.loft`'s tokenizer stamps each `Token` with a span
  (`{ line, col, byte }`) — the same state loft's own lexer tracks. The matcher stays
  position-agnostic (it tracks a cursor *index*); a problem's position is the span of the token at
  that index.
- **matcher → the farthest-failure index.** The cursor's `farthest` field is a monotonic high-water
  mark — the deepest position reached across *all* backtracks (never rewound by a revert). It is
  PEG's "where did the longest partial match break." When the `_` arm fires, the parser reads
  `cur.farthest` → the token's span → points the error there. (Collecting the *expected set* —
  "expected `)` or `,`" — is a later refinement layered on the same field.)
- **renderer → the caret.** Reuse loft's own diagnostic renderer (@PLN28: `file:line:col` + source
  line + caret, `LOFT_ERRORS=pretty`); expose it as a stdlib call `report(span, msg)`. A loft
  parser's errors then look identical to loft's own.
- **author → the message, and cuts for precision.** Farthest-failure gives a decent automatic
  error; a **cut** (`~`, the draft's open Q #1) after a committing token turns a later failure into
  a *definite* reported error (`"expected `)` to close the parameter list"`) instead of a silent
  backtrack that loses the position. Cut is an optional refinement (§6 phasing).

---

## 6. Formal rules (extend [matching.md](../../formal/matching.md) / [FORMAL-DESIGN.md](FORMAL-DESIGN.md))

```
  (P-Rule)      ⟨name: rule, κ, σ⟩ ⇓ Match({name ↦ v}, κ')     when rule(κ) ⇓ v (non-null), κ→κ' advanced
                ⟨name: rule, κ, σ⟩ ⇓ Fail                        when rule(κ) ⇓ null (κ, σ unchanged — INV-Pure)
  (P-Cursor)    match over a Cursor<T> consumes a PREFIX and advances it; match over a bare vector<T>
                is P-Whole (a fresh cursor, whole-consume).
  (INV-Static)  a match that TYPE-CHECKS + passes well-formedness (§4.1) + purity (§4.2) has NO
                runtime match/parser fault: it selects an arm (M-Total) and reads captures directly.
  (WF-Term)     the sub-rule graph has no left-recursive cycle, and every `*`/`+` body consumes ≥1;
                a violation is a STATIC error (driver-agreement: reject on --dump/--interpret/--native).
  (WF-Pure)     every sub-rule is pure (capabilities.md effect analysis); a violation is a STATIC error.
```

`P-Rule` inherits `P-Atomic` (a failed sub-rule leaves κ/σ unchanged). Reporting is *observability*
(like `E-Report`), not a value-affecting rule: `cur.farthest` is the furthest κ across reverts.

---

## 7. Implementation steps (PC1–PC5, after the core P-phases)

Each step is independently shippable and verified on **both backends** (`--interpret` + `--native`),
leak-checked, with driver-agreement for the static rejects.

### PC1 — Cursor as a first-class value — **DONE (fixed-arity), both backends**

**Landed (option (c), no generics).** Since generic structs are unsupported, a cursor is a
CONCRETE struct with a `vector<T>` source field + an integer field named `pos` — recognised by
shape (`cursor_shape`), no `Cursor<T>` type needed; the token type comes from the pattern variants.
`match <cursor> { [pat] }` PREFIX-consumes: `parse_cursor_match` reads source + pos into temps, sets
`match_cursor` so `read_slice_elem` offsets forward reads by `pos` and the length gate flips from
`len == fixed` (whole) to `pos + fixed <= len` (prefix), then advances `cursor.pos` by the consumed
count on a match (writing the caller's cursor directly — a struct is a DbRef).  A plain `vector<T>`
is unchanged (whole-consume).  Guard `tests/scripts/35q-cursor-match.loft` (sequential consume, field
reads relative to pos, end-of-input, vector-unaffected; cross-mode + leak).  **Deferred to a
follow-up:** repetition / `..rest` / tail-from-end in cursor mode (the offset+prefix interplay needs
more than the fixed-arity gate); today only fixed-arity forward patterns prefix-consume.

**Original goal.** `match` accepts a `Cursor<T>` subject and consumes a prefix; the slice-match
desugaring routes through the same cursor.
**Design.** Introduce the `Cursor<T>` type + `peek`/`next`/`at_end` + matcher-only `anchor`/`revert`.
Matching a bare `vector<T>` = wrap in a fresh cursor at 0, whole-consume (`P-Whole`); matching a
`Cursor<T>` = prefix-consume, advance.
**Code points.** `src/parser/control.rs` (`parse_match` `:2620`, the arm-chain build) — the internal
index temp from P-Seq becomes the cursor's `pos`; add the `Cursor<T>` subject arm. Cursor type in
stdlib (`default/*.loft`) or a runtime struct. Anchor/revert = save/restore `pos` (slice) reusing
the P4 mechanic; the P7 memo for streams.
**Verify.** `match cur { [A, B] => … }` over a Cursor consumes exactly 2 and leaves `pos` at 2;
matching a vector still whole-consumes. Both backends; leak-clean.

### PC2 — Sub-rule invocation `name: rule` — **DONE (whole-pattern), both backends**

**Landed (first cut).** In a cursor match, a slice element `[ name: rule ]` where `rule` names a
`fn(cursor) -> Node` is a sub-rule invocation (`peek_subrule_capture` — an `n_<rule>` fn returning a
`Reference`; disjoint from `name: Variant` sub-patterns, which have no `n_<...>` fn).  It desugars to
the proven hand-form `r = rule(cursor); if r != null { name = r; … }`: the sub-rule prefix-matches
over the SAME cursor (PC1 semantics — advances `pos` on a match, leaves it on a miss, returns null on
a miss), so the call HOISTS above the arm if-chain (`prechain`, a match-level binding visible to every
arm — a cond sub-block scopes it away on native) and runs once; the arm gates on `name != null`
(`OpNeRef(name, OpNullRefSentinel())` — the ref path the hand-form uses, NOT `OpRefIsNull`, which
misreads a fn-returned null struct on the interpreter).  `multi_alt` skips the fixed-length gate +
pos-advance — the sub-rule owns the pos.  A sub-rule may consume MANY tokens (pos jumps by its width),
and sequential top-level matches walk the stream, so recursive descent composes today.  Guard
`tests/scripts/35r-subrule-invoke.loft` (1- and 2-token sub-rules, hit/miss/wildcard, sequential
walk; both backends, leak-clean) + `parse_errors::subrule_mixing_deferred`.  **DEFERRED:** mixing a
sub-rule with fixed elements in the same pattern (`[ Fn, params: rule, RP ]`) — needs the running-pos
+ revert (a variable-width sub-rule advances by a runtime amount, so later elements read at a runtime
pos); today a sub-rule must be the WHOLE slice pattern.  Match bindings leak to the enclosing scope
(pre-existing), so reusing a binding name across sub-rules of different return types collides — use
distinct names.

**Original goal.** The `parse_fn` example (§1) parses and runs.
**Design.** At a pattern element, resolve an identifier that names a function of type
`fn(&Cursor<T>) -> option<R>` as a sub-rule; desugar to the anchor/call/null-check/bind/thread of §3.
**Code points.** `src/parser/control.rs` pattern parsing (the `parse_pattern` element loop from P1)
— add the sub-rule-call case; signature check via `data.def`/`typedef.rs`. Emit `Value::Call` +
cursor threading + the fail-jump (reuse the P4 backtracking jump).
**Verify.** The `[ Fn, Ident{name:n}, LParen, params: parameters, RParen ]` arm binds `params` from
the sub-rule; a sub-rule that fails mid-arm reverts the cursor and the arm fails/falls through. Both
backends; nested/recursive rules (expr→term→factor→expr-in-parens) terminate.

### PC3 — Termination / well-formedness pass (§4.1) — **DONE (cycle detection), both backends**

**Landed.** PC2 records a sub-rule edge `(enclosing_rule → invoked_rule, site)` on pass 2
(`subrule_edges`); a post-parse pass (`check_subrule_termination`, run from both `parse` and
`parse_str`) builds the graph and rejects any CYCLE as left recursion, naming the path (`sub-rule
`expr` is left-recursive (expr -> term -> expr): …`).  For the current surface this is exactly right
and complete: every PC2 invocation `[ name: rule ]` is a WHOLE pattern, so the sub-rule runs at
cursor position 0 (nothing consumed) and its call is hoisted unconditionally — so ANY cycle recurses
forever (no base-case arm can intervene; confirmed — a self-recursive rule blows the stack at 10k+
frames without the guard).  Detection is a 3-colour DFS (`find_subrule_cycles`), deterministic node
order, one report per back-edge; the diagnostic points at the invocation site.  Guard
`parse_errors::subrule_left_recursion` + `pc3-*` probes (self-rec + mutual A→B→A rejected; acyclic
DAG wrap→leaf compiles + runs both backends).  **When mixing lands (PC2 follow-up)** a consuming edge
becomes possible, so the min-consumption refinement (reject only cycles with no intervening consume;
non-consuming `*`/`+`; opaque sub-rule under `*`/`+`) is the future extension — today all edges are
non-consuming, so reject-all-cycles is both sound and not over-restrictive (no terminating cycle is
expressible yet).

**Original goal.** A left-recursive or empty-looping grammar is a **compile error**, not a runtime hang.
**Design.** After two-pass parsing, walk the sub-rule graph; compute min-consumption; reject
left-recursion + non-consuming `*`/`+`; refuse opaque sub-rules under `*`/`+`.
**Code points.** A new well-formedness analysis (new module under `src/parser/`, run post-parse over
the def graph); emit diagnostics through the standard path.
**Verify.** `tests/parse_errors.rs`: a left-recursive rule rejects (driver-agreement); `p*` with an
empty-matching `p` rejects; a well-formed grammar compiles. Hand-check the min-consumption on a
mutually-recursive grammar.

### PC4 — Purity pass (§4.2) — **DONE, both backends**

**Landed.** An invoked sub-rule must be pure, or it is a compile error.  Runs in the same post-parse
pass as PC3 (`check_subrule_wellformedness`): for each invoked sub-rule (callee), `scopes::
sub_rule_is_pure` descends the call graph and rejects it if it reaches an OBSERVABLE / non-
deterministic / concurrent effect.  Confirmed hazard: with the unconditional hoist, an impure
sub-rule's `print` fires even when the arm MISSES (`[noisy] side effect!` before `r=-1`).
**Category calibration (the crux):** loft's effect analysis annotates the impure builtins, but
`#impure(parent_write)` covers record construction AND the cursor advance (`OpNewRecord` / `OpSetInt`
/ `OpFinishRecord`) — every sub-rule uses it internally — so `parent_write` CANNOT disqualify; only
`host_io` / `io` / `prng` / `par_call` do.  An un-annotated native builtin is pure (every observable
builtin is annotated `#impure`, so the un-annotated natives are pure reads / arithmetic / record
ops).  A recursion cycle breaks optimistically (PC3 rejects left recursion separately); an indirect
`CallRef` is conservatively impure.  Guard `parse_errors::subrule_impure_rejected` (direct + a
transitive helper both rejected) + `35r` extended with a sub-rule that calls a PURE helper (allowed,
runs).  **Deferred:** writing EXTERNAL shared state (vs the result record / cursor) is a `parent_write`
too, so backtracking over such a write is not caught at this category granularity — needs a
local-vs-external write distinction (the deps/ownership analysis).

**Original goal.** An impure sub-rule is a compile error.
**Design.** Reuse the capability/effect analysis to require sub-rule purity.
**Code points.** `src/sandbox.rs` / the `capabilities.md` machinery (`mark_lambda_sandboxed` and
kin); hook the purity check at the sub-rule reference site.
**Verify.** A sub-rule doing I/O (or host mutation) used in a pattern rejects; a pure one compiles.
Both backends / driver-agreement.

### PC5 — Reporting (§5) — **DONE (farthest primitive + reporting-as-a-value), both backends**

**Landed (the compiler-essential core).** The PEG farthest-failure primitive: a cursor may carry an
OPTIONAL `farthest: integer` field (opt-in — a plain `{ src, pos }` cursor is unaffected), and the
match maintains it as a monotonic high-water mark `farthest = max(farthest, new_pos)` at every
advance.  It is a MAX (never rewound by a backtrack) and rides the SHARED cursor, so it propagates
through sub-rules automatically — after a failed parse `cursor.farthest` names the deepest token
reached.  Detected in `parse_cursor_match` (`match_cursor_farthest`), emitted beside the pos-advance.
"Never a trap" already holds (a failed match returns null); PC5 adds the POSITION.  Reporting is then
a VALUE built with existing features: read `src[farthest]` and format a message (guard
`35s-farthest.loft` — `parse error at token 1: expected identifier, found `(`` — both backends,
leak-clean; plain-cursor unaffected).  **The library layer (parts a/c/d) is a convention, not a
compiler feature:** spans on tokens are the user's token type (like `lib/lexer.loft`'s
`line`/`pos`/`Anchor` + `at()`), and `report(span, msg)` is string interpolation over `farthest`
(the `lib/parser.loft` `log.error("…{lexer.at()}")` pattern) — no new runtime hook needed.
**Deferred:** `farthest` is the deepest CONSUMED position, not the deepest ATTEMPTED (a whole-pattern
arm that matches a prefix then fails does not advance, so its partial reach is not recorded) — precise
farthest-failure needs updating on every read, and the authored-cut (`~`) refinement is future work.

**Original goal.** A failing parse yields a lexer-quality diagnostic (`file:line:col` + caret)
pointing at the farthest token — as a value, never a trap.
**Design.** (a) `lib/lexer.loft` tokens carry a `Span`; (b) thread the monotonic `farthest` through
the desugared match; expose `cur.farthest`; (c) expose the @PLN28 renderer as `report(span, msg)`;
(d) *optional* cut (`~`) for authored messages.
**Code points.** `lib/lexer.loft` (Token span + `tokenise`); `src/parser/control.rs` (update
`farthest` on each failure jump); the @PLN28 error renderer (locate the render entry; expose a
stdlib wrapper); grammar for `~` if cut is in scope.
**Verify.** A malformed input produces an error at the correct token with a caret, identical on both
backends; the farthest-failure position matches the hand-computed longest-partial-match. Cut (if
built): a failure past a cut is a definite `Err`, not a backtrack.

---

## 8. Dependencies & phase placement

PC1–PC2 need P4 (backtracking / the cursor). PC5 needs `lib/lexer.loft` tokens with spans and the
@PLN28 renderer. Streaming sub-rules (large inputs) want P7 (iterator input). So the natural order is
**P1–P4 core → PC1–PC2 sub-rules → PC3–PC4 the INV-Static passes → P7 streaming → PC5 reporting**
(reporting last because good errors want the whole thing working). PC3/PC4 gate *shipping* the
sub-rule layer — INV-Static is not optional once sub-rules exist.

## 9. Risks / open questions

| risk | mitigation |
|---|---|
| **Generic `Cursor<T>`** needs loft generic-struct support | confirm generic structs (interfaces.md monomorphization) cover it; else a built-in cursor per token type, or an opaque cursor handle the match passes implicitly |
| **Opaque sub-rule min-consumption** undecidable | forbid opaque sub-rules under `*`/`+` (§4.1); rules whose body is a cursor-match are always analyzable |
| **Purity analysis precision** (false positives) | start conservative (reject anything reaching a host effect); widen as capabilities.md already does for closures |
| **Cut vs pure-PEG** (does `~` complicate backtracking?) | ship PC1–PC5 without cut first; add cut only if farthest-failure errors prove too coarse (draft open Q #1) |
| **Left-recursion is a real ergonomic limit** (expression grammars love it) | document the standard rewrite (left-recursion → repetition: `expr := term (op term)*`), which the PEG `*` expresses naturally |

## 10. See also

- [FORMAL-DESIGN.md](FORMAL-DESIGN.md) — the formal spec changes (P-Rule / INV-Static / WF-* extend it).
- [IMPLEMENTATION.md](IMPLEMENTATION.md) — the core P1–P7 build plan this layers onto.
- [EXAMPLES.md](EXAMPLES.md) — parser examples (the §1 `parse_fn` shape).
- [C89](../../DESIGN_DECISIONS.md) — reads-like-grammar / never-forced / no-mitigation-sprawl.
- `lib/lexer.loft` — the cursor-protocol precedent (`anchor`/`revert` + typed probes) + token spans.
- `capabilities.md` / `src/sandbox.rs` — the effect analysis PC4 reuses.
