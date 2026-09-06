<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/matching.md — semantics for `match` (strict)

**Catalogue:** @F3 (enum/match core), @PLN89 (differential oracle).

> **Rules then deviations** (see [README](README.md)). This is the relation for the `match`
> expression — enum-variant dispatch with payload binding. It is the second control form
> [operational.md](operational.md) pins only half of (`if`, not `match`). It extends
> operational.md (control flow, expressions) and [heap.md](heap.md) (an enum value is a tagged
> heap value; a variant pattern reads its payload). Every rule is a **user-visible contract**
> verified on both backends.
>
> A `match`'s headline guarantee is **compile-time exhaustiveness**: a `match` that forgets a
> variant does not compile. That is a promise to the user, checked before the program runs.
>
> **@PLN35 extension (SHIPPED):** the § *Rules — PEG patterns* below adds sequence / alternation /
> optional / repetition / capture patterns, built in phases 1–7 + PC1–PC5 (350e660c #554, 3fda4e1e
> #558, 50cc4c18 #561, a37917ff #562) and verified on both backends. It generalises this
> exhaustiveness guarantee to `M-Total`, so the promise survives patterns that can *fail*.

## Notation

Uses [operational.md](operational.md)'s `⟨e, σ⟩ → ⟨e', σ'⟩`. An enum value `v` has a **variant
tag** and, for a struct-payload variant, named payload **fields**. A `match` is
`match e { pat₁ => b₁, …, patₙ => bₙ }`; a pattern is a unit variant `V`, a struct-payload
variant `V { f₁, … }`, or the wildcard `_`.

---

## Rules

### `match` is an expression that selects the first matching arm

```
  (M-Match)   ⟨match e { pat₁ => b₁, … }, σ⟩ → ⟨e', σ⟩          when e → e'   (scrutinee first)
              ⟨match v { pat₁ => b₁, … }, σ⟩ → ⟨bₖ[binds], σ⟩
                where k is the SMALLEST index whose patₖ matches v, and binds is patₖ's bindings.
  (M-Expr)    match is an EXPRESSION: every arm body bᵢ has the match's result type, and the
              selected arm's value is the whole match's value (feeds directly into `r = match …`).
```

**In words.** `match` first reduces the scrutinee to a value, then picks the **first** arm (top to
bottom) whose pattern matches, binds that pattern's variables, and evaluates its body — the body's
value **is** the match's value, so `r = match c { … }` is normal (verified: it returns `100`).
Only the selected arm runs.

### Patterns — unit, struct-payload with field binding, wildcard

```
  (M-Unit)     pattern V     matches an enum value whose variant is the unit variant V.
  (M-Variant)  pattern V { f₁, …, fₘ }  matches a value whose variant is V, BINDING each fⱼ to the
               corresponding payload field of v (by name), in scope for that arm's body.
  (M-Wild)     pattern _     matches ANY value; it is the catch-all.  It must be the LAST arm —
               an arm after `_` is a STATIC error (unreachable).
```

**In words.** A unit variant (`Dot`) matches by tag alone; a struct-payload variant
(`Circle { r }`, `Box { w, h }`) matches by tag AND binds its payload fields by name into the
arm, so `Circle { r } => r * r` uses the matched value's `r` (verified: `25` for `r = 5`). The
wildcard `_` matches everything and is the default — it must come last, because any arm written
after it could never run (loft rejects that at compile time).

### Exhaustiveness is checked at compile time

```
  (M-Exhaust)  a match on an enum must cover EVERY variant — each variant by its own arm, or a
               trailing `_`.  A match missing a variant is a STATIC ERROR
               ("match on E is not exhaustive — missing: …"), NOT a runtime fault.
  (M-Bool)     a match on a BOOLEAN whose arms name `true` and `false`, each unguarded, is
               exhaustive the same way: no value falls through, so nothing about the match is
               nullable.  A guarded arm can fail and is not part of the domain.
```

**In words.** The compiler proves a `match` handles every case: if you add a variant to an enum,
every `match` that forgot it stops compiling with a precise "missing: …" message (verified). This
is the load-bearing guarantee — a `match` can never fall through to nothing at runtime, so there
is no "unmatched value" runtime error in loft's model; the exhaustiveness is discharged
statically, before the program runs.

`(M-Bool)` is the same guarantee for the one scalar whose whole domain a match can spell.  It
was written after the compiler answered the other way: a wildcard-less scalar match carries a
typed-null fall-through for the value no arm matched, and a `match w { true => …, false => … }`
kept it — so its join read as nullable and `-> text { match w { … } }` warned
nullable-into-non-null on both backends, whatever the arms held, while the same choice
spelled as an `if` was quiet (loft#1343).  The last arm is the fallback now, as a trailing
`_` would be.

---

## Rules — PEG patterns (@PLN35, SHIPPED)

> **@PLN35 · SHIPPED.** Like everything above, the rules in THIS section are shipped semantics,
> pinned by the oracle: phases 1–7 + PC1–PC5 of the PEG match-pattern extension in
> [../plans/35-match-peg/](../plans/35-match-peg/) landed (350e660c #554, 3fda4e1e #558, 50cc4c18
> #561, a37917ff #562) and every named rule below — `P-Seq`, `P-Alt`, `P-Opt`, `P-Rep`, `P-Cap`,
> `P-Rest`, `P-Multi`, `P-Atomic` — is verified passing on both backends via
> `tests/scripts/35*.loft` (worklist: [VERIFICATION.md § matching.md — PEG patterns](VERIFICATION.md)).
> Overview + phase↔rule map: [../plans/35-match-peg/FORMAL-DESIGN.md](../plans/35-match-peg/FORMAL-DESIGN.md).

PEG patterns generalise a *point* pattern (unit/struct variant, `_`) to a **sequence** that may
branch (`|`), skip (`?`), repeat (`*`/`+`), and **capture** sub-results — over a vector/slice or an
iterator. The load-bearing constraint is that they must **preserve `M-Exhaust`**: a structural
pattern can *fail*, so totality is re-secured by requiring a total final arm (`M-Total`).

### The pattern-match relation

An input is walked by a **cursor** `κ = ⟨i, src⟩` — [iteration.md](iteration.md)'s iterator: an
index `i` into a source `src`, with `elem(src,i)` / `len(src)` **null past the end, never a fault**
(`I-Done`). The relation:

```
  ⟨pat, κ, σ⟩ ⇓ Match(binds, κ')     pat matches, consuming κ→κ', binding binds
  ⟨pat, κ, σ⟩ ⇓ Fail                  pat does not match — κ and σ UNCHANGED (P-Atomic / INV-Pure)
```

```
  (P-Point)  a unit variant V, struct variant V{f…}, literal, `_`, or bare binding is a POINT
             pattern over one value (today's M-Unit/M-Variant/M-Wild lifted into ⇓).  A struct /
             variant FIELD may itself be a pattern (nested) — the recursion this extension adds.
  (P-Range)  a RANGE pattern over a SCALAR value v (a non-enum subject — integer / character):
             `a..=b` (inclusive) matches iff a ≤ v ≤ b; `a..b` (half-open) matches iff a ≤ v < b —
             the upper bound is EXCLUSIVE.  A POINT pattern (one value, no extra cursor advance);
             Fail otherwise.  Verified both backends: `2..=5` matches 2 and 5; `2..5` excludes 5.
  (P-Seq)    ⟨[p₁ … pₙ], κ⟩: run p₁ from κ→κ₁, …, pₙ from κ_{n-1}→κₙ; ANY pᵢ ⇓ Fail ⟹ the whole
             sequence ⇓ Fail (κ unchanged).  binds = ⋃ᵢ binds_i.
  (P-Whole)  an ARM's sequence pattern must consume the WHOLE input (κ' = ⟨len(src),src⟩); a proper
             PREFIX ⇓ Fail for arm-selection UNLESS the sequence ends in `..rest`, which absorbs the
             remainder.  (This is why `[a,b,c]` needs exact length today.)
  (P-Alt)    ⟨(a | b), κ⟩: try a from κ; if Match, that; else try b from the SAME κ.  Ordered choice
             — FIRST success wins; both Fail ⟹ Fail.
  (P-Opt)    ⟨(a)?, κ⟩: try a; on Match(bs,κ') that; on Fail ⟹ Match(bs↦null, κ) — succeeds with a's
             captures null, cursor UNMOVED.  (P-Opt never Fails.)
  (P-Rep)    ⟨(a)*, κ⟩: greedily match a from κ→κ₁→…; on the first Fail at κ_m ⟹ Match(collected, κ_m).
             `(a)+` = a then (a)*.  A separator `*(s)` is consumed between iterations, not captured.
             BOUNDED by len(src) for slices ⟹ terminates; for iterators, by `max_lookahead` (P-IterBound).
  (P-Cap)    ⟨name:p, κ⟩: run p; on Match(bs,κ') ⟹ Match(bs ∪ {name ↦ p's result}, κ').
  (P-Rest)   ⟨..name, κ=⟨i,src⟩⟩ ⟹ Match({name ↦ a FRESH vector of src[i .. len−t]}, ⟨len−t, src⟩),
             t = fixed patterns after the rest (H-Alloc — a new store, independent of src).
  (P-Multi)  a MULTI-PATTERN arm `pat_a, pat_b => body`: try pat_a from ⟨0,v⟩ (whole-match); else
             pat_b; the FIRST whole-match commits.  (P-Alt at arm granularity — no new cursor work.)
  (P-Guard)  a GUARDED arm `pat if cond => body`: run `pat` from κ; on Match(binds,κ') evaluate
             `cond` under σ extended with `binds`.  cond true ⟹ the arm commits (Match(binds,κ'));
             cond false ⟹ the arm ⇓ Fail — exactly as if `pat` had not matched (P-Atomic keeps the
             provisional binds invisible) — and selection moves to the NEXT arm.  The guard is the
             only way an already-matched pattern can still reject its arm.
  (P-Atomic) ⟨pat,κ,σ⟩ ⇓ Fail ⟹ σ UNCHANGED, κ not advanced (INV-Pure).  Provisional captures from a
             failed attempt are NEVER observable — the arm body runs ONLY after a committed whole-match.
```

**The parentheses above are METANOTATION, and the concrete syntax has two spellings.**  `⟨(a)*⟩`
says *"a repetition of the pattern a"*; it does not say the source contains a `(`.  A VARIANT
element is written with the parens — `[ (x: Num)*, ..rest ]`, `[ (Kw { k })? ]` — and a SCALAR
element is written without them, as a bare capture with the suffix on the type:
`[ xs:integer* ]`.  There is no parenthesised scalar form and no bare variant form; each kind
takes exactly one of the two, and the wrong one is a parse error that reports `Expect token ,`
rather than naming the spelling.  Reading `(a)*` as literal syntax is what a first reader does —
it cost four wrong probes in the walk that added this note (QUALITY.md B8i) — so the two forms
are written out here beside the rule they instantiate.

**In words.** A pattern either matches — moving the cursor forward and binding names — or fails,
leaving everything exactly as it was. A sequence runs its parts in order and fails as a whole if any
part fails; an arm's sequence must line up with the *entire* input unless it ends in `..rest`.
Alternation tries its branches left to right and takes the first that works; an optional either
matches or quietly binds its captures to null without moving; a repetition matches greedily and
stops at the first failure, collecting what it got. Crucially, a *failed* attempt is invisible — no
half-bound name, no half-moved cursor leaks to the next arm (`P-Atomic`), which is what makes
backtracking safe.

### `M-Exhaust` generalises to `M-Total` (the invariant this extension must not break)

```
  (M-Total)  total(pat):
               total(_) = total(bare name) = true
               total(V) / total(V{f…}) = true  iff every field sub-pattern is total
               total(sequence | alternation-not-covering | optional-in-required-pos | repetition |
                     length-constrained slice | literal | range) = false
               total(pat if cond) = false — a GUARD can reject, so a guarded arm NEVER secures
                     totality, whatever its pattern.
             ENFORCEMENT splits on whether coverage is DECIDABLE:
               • ENUM subject — the variant set is finite + known, so coverage IS checked.  A variant
                 counts as covered only by a TOTAL arm (bare `Variant` / `_` / bare binding), NEVER by
                 a guarded or otherwise non-total arm.  A variant left uncovered with no `_` is a
                 STATIC ERROR ("match on T is not exhaustive — missing: X; add the missing variants or
                 a `_ =>` wildcard").
               • SCALAR subject (integer / character — an unbounded domain) — coverage is NOT decidable
                 and NOT required.  With no total final arm the match MAY select no arm at runtime; by
                 the C80 spreadsheet model ([DESIGN_DECISIONS.md C80](../DESIGN_DECISIONS.md)) it then
                 yields **null**, so the match's result type is **nullable** (`τ?`) — no error, the null
                 surfaces at the use site (the null-flow discipline).  This is the one place a `match`
                 "falls through", and it falls through to null, never to a fault.
```

**In words.** For an ENUM subject this keeps loft's promise that a `match` never falls through to
nothing: the compiler requires the arms to cover every variant (a variant counts only when a TOTAL
arm names it — a guard does not), or a final `_`; otherwise the program does not compile. For a
SCALAR subject (integer / character) coverage cannot be decided, so it is not required — a match with
no total final arm may select nothing at runtime and then yields **null** (the C80 model), which makes
its result type nullable. So a `match` still never faults on a fall-through: on an enum it cannot fall
through at all, and on a scalar it falls through to null. For a pure-enum match, nothing changes from
`M-Exhaust`.

### Iterator inputs add the only new operational primitive

For a **vector/slice**, anchor/revert is just save/restore of `i` — pure
[operational.md](operational.md) assignment, **no new op**. For an **iterator** (a source that
cannot be re-indexed), a failed alternative must *replay* pulled items, so two ops are added — the
`Lexer::memory` + `links`-refcount model (`src/lexer.rs`):

```
  (P-Anchor)    OpMatchAnchor: push ⟨i, epoch⟩; while any anchor is live, next(it) APPENDS the pulled
                item to a memo buffer instead of discarding it.
  (P-Revert)    OpMatchRevert: pop the anchor, rewind i (replaying from the memo), drop bindings
                written after epoch.  The buffer clears when the anchor stack empties (refcount 0).
  (P-IterBound) a repetition over an iterator is bounded by `max_lookahead`; exceeding it is a
                DEFINED runtime error (never a hang) — preserving termination.
```

A side-effecting pull (a generator that mutates external state per item) cannot be reverted;
matching over such a source is UB-by-contract (documented in [../CAVEATS.md](../CAVEATS.md)) — the
same assumption `Lexer` makes about its token stream.

Captures follow [types.md § Pattern captures](types.md) (no new type former — `τ` / `τ?` /
`vector<τ>` via the join) and [binding.md § Pattern captures](binding.md) (a single interior capture
is a view; `..rest` / repetition are fresh vectors); the pattern grammar + precedence are in
[grammar.md § Pattern-operator precedence](grammar.md).

---

## Deviations

OPEN: **0** (a *rules* doc — it shrinks operational.md's D-op-1, adds no code deviation).

- **D-match-1 — OPENED AND CLOSED 2026-09-04 (loft#1343).** `(M-Bool)` did not exist, and the
  edge it names was answered wrong: a boolean match spelling both arms was lowered with the
  scalar match's typed-null fall-through, so its type read nullable and a `-> text` function
  returning it warned `(N-Store)` on both backends.  Closed in the scalar match parser: both
  literal arms, unguarded, make the last arm the fallback.  Guard
  `tests/scripts/1343-a-boolean-match-with-both-arms-is-exhaustive.loft` +
  `tests/boolean_match_exhaustive.rs` (the warning stream, which no corpus channel scores);
  falsified at `dd46146c` — five warnings → none on both backends.

- **PEG patterns are SHIPPED (@PLN35)** — the *Rules — PEG patterns* § opens **no** deviation: the
  shipped implementation (phases 1–7 + PC1–PC5, [plans/35-match-peg](../plans/35-match-peg/))
  conforms to the stated rules, verified both backends. Each rule is pinned by the @PLN89 oracle in
  [VERIFICATION.md § matching.md — PEG patterns](VERIFICATION.md).
- **Conformance is differential** — `match` dispatch is enforced across the two backends by the
  @PLN89 oracle (D-op-1): `20-nested-enum-match` and `07-enum-match-dispatch` carry struct-payload
  variants, recursive walks, and matches whose arms return different variants, precisely because
  the native tag dispatch + payload layout differ from the interpreter's. A divergence in which
  arm fires, or in a bound payload value, is caught there.
- **Exhaustiveness is a STATIC judgment** — so it also participates in the oracle's
  *driver-agreement* facet (D-op-2): `--dump` / `--interpret` / `--native` must agree that a
  non-exhaustive match is rejected.

## Conformance

- **Arm selection + payload bind (`M-Variant`)** — `match Sh::Circle { r: 5 } { Dot => 0,
  Circle { r } => r*r }` is `25`.
- **Wildcard default (`M-Wild`)** — `match C::D { A => 1, _ => 0 }` is `0`; an arm after `_` is a
  compile error.
- **Exhaustiveness (`M-Exhaust`)** — `match c { A => 1 }` over `enum C { A, B }` does NOT compile
  ("missing: B"); adding a `B => …` arm or a trailing `_` makes it compile.
- **As an expression (`M-Expr`)** — `r = match c { A => 100, B => 200 }` binds `r` to the arm's
  value (`100`).

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on which
arm a `match` selects, on a bound payload value, or on whether a match is exhaustive is the
definitional error this doc names.
