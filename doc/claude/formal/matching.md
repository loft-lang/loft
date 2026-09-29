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

**A NULLABLE subject or element names the same variants.** `E?` is `E`'s layout with the
reserved discriminant 0 for absence (`layout.md` `(L-Null)` / `(L-Enum)`), so `V` and `V { f }`
are asked of a `vector<E?>` element exactly as of a dense one, and an ABSENT value matches no
variant arm — a variant is 1 or above. That is `(M-Unit)` / `(M-Variant)` read literally, and
it needs no null test of its own. What the compiler did instead was ask `Type::Enum` of the
WRAPPER: `[Id { x }]` over a `vector<Tok?>` was a parse error naming nothing, and the unit
spelling `[Id]` degraded into a bare-name BINDING that matched every element, absent ones
included (loft#1410 — `Parser::pattern_variant_enum` is the one home that answers this
question now).  A subject that is absent falls to the `_` arm, and where there is none the
match answers null, which `(N-Store)` then reports at the slot it reaches.

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

> **@PLN35 · SHIPPED, with one exception named below.** Phases 1–7 + PC1–PC5 of the PEG
> match-pattern extension in [../plans/35-match-peg/](../plans/35-match-peg/) landed (350e660c
> #554, 3fda4e1e #558, 50cc4c18 #561, a37917ff #562), and `P-Seq`, `P-Alt`, `P-Opt`, `P-Rep`,
> `P-Cap`, `P-Rest`, `P-Multi`, `P-Atomic` are verified passing on both backends via
> `tests/scripts/35*.loft` (worklist: [VERIFICATION.md § matching.md — PEG patterns](VERIFICATION.md)).
>
> ⚠ **`P-Anchor`, `P-Revert` and `P-IterBound` are NOT among them, and are not shipped** —
> they describe a memoising cursor that was never built, and the ops they name exist nowhere in
> `src/`.  The shipped design materialises an iterator subject into a vector instead, which
> leaves `(P-IterBound)`'s bound absent and an endless source unbounded: **D-match-6**,
> loft#1678.  The banner said SHIPPED over all fourteen rules until 2026-09-25; a section
> banner covers the rules a reader then reads under it, so it has to name its exceptions.
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
             PREFIX ⇓ Fail for arm-selection UNLESS the sequence CONTAINS a rest, which absorbs
             whatever the fixed elements around it do not take (`P-Rest`'s `t` counts the ones
             AFTER it, so the rest need not be last).  (This is why `[a,b,c]` needs exact length
             today.)
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
             A slice holds AT MOST ONE rest: a second has no length of its own (the first takes
             every element the fixed ones leave), so it is a STATIC ERROR.
  (P-Rep-Scalar) a SCALAR repetition `xs:T*` names the vector's own element type T, so every
             element matches it and (P-Rep)'s greedy run would take all of them.  It is instead
             the slice's ONE rest, typed: `xs:T*` ≡ `..xs`, and `xs:T+` ≡ `..xs` with ≥ 1
             element.  Whatever follows it is (P-Rest)'s fixed tail, read from the END — a
             literal, `_`, a bare name, a variant: `[xs:integer*, last]` binds last = v[len−1].
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
it cost four wrong probes in the walk that added this note (QUALITY-history.md B8i) — so the two forms
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
                 counts as covered only by a TOTAL arm (bare `Variant` / `_`), NEVER by a guarded or
                 otherwise non-total arm.  A variant left uncovered with no `_` is a STATIC ERROR
                 ("match on T is not exhaustive — missing: X; add the missing variants or a `_ =>`
                 wildcard"), and an arm naming something that is NOT a variant of the subject's enum
                 is a static error at the arm (M-Unit) rather than an arm that never fires.
               • SCALAR subject (integer / character — an unbounded domain) — coverage is NOT decidable
                 and NOT required.  With no total final arm the match MAY select no arm at runtime; by
                 the C80 spreadsheet model ([DESIGN_DECISIONS.md C80](../DESIGN_DECISIONS.md)) it then
                 yields **null**, so the match's result type is **nullable** (`τ?`) — no error, the null
                 surfaces at the use site (the null-flow discipline).  This is the one place a `match`
                 "falls through", and it falls through to null, never to a fault.
```

**In words.** For an ENUM subject this keeps loft's promise that a `match` never falls through to
nothing: the compiler requires the arms to cover every variant (a variant counts only when a TOTAL
arm names it — a guard does not), or a final `_`; otherwise the program does not compile.

> ⚠ **A bare binding is an ELEMENT pattern, never an enum ARM.** `total(bare name) = true` above is
> about a POINT pattern inside a sequence — `[a, b] => a + b` binds two elements — and the ARM
> position has no such form: the grammar is `pattern ::= '_' | 'null' | literal | range |
> CamelIdent [ '{' field_bind '}' ]` (LOFT.md § Grammar), so `match c { A => 1, other => 2 }` does
> not bind `c` to `other`; `other` is read as a variant name and there is no variant of that name.
> The enum-subject bullet said "bare binding" for two months and the code never had it — corrected
> 2026-09-07, together with the silence that hid it: an arm whose name resolved NOWHERE was
> skipped without a diagnostic, so a misspelled or renamed variant fell to `_` and the program
> answered the wildcard's value on both backends
> (`tests/scripts/a-match-arm-names-a-variant-that-exists.loft`). For a
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

**How it is built.**  `(P-Anchor)` and `(P-Revert)` describe a memoising cursor, and the ops they
name were never built: the shipped design MATERIALISES the subject into a buffer and runs the
vector-match machinery over it.  For a source without side effects the two agree on every
observable — every item a revert would replay is in the buffer — and `(P-IterBound)` is enforced
by bounding that materialise (D-match-6).

A side-effecting pull (a generator that mutates external state per item) cannot be reverted;
matching over such a source is UB-by-contract (documented in [../CAVEATS.md](../CAVEATS.md)) — the
same assumption `Lexer` makes about its token stream.

Captures follow [types.md § Pattern captures](types.md) (no new type former — `τ` / `τ?` /
`vector<τ>` via the join) and [binding.md § Pattern captures](binding.md) (a single interior capture
is a view; `..rest` / repetition are fresh vectors); the pattern grammar + precedence are in
[grammar.md § Pattern-operator precedence](grammar.md).

---

## Deviations

OPEN: **0** — the 2026-09-29 rule-led walk of the pattern rules found `D-match-7` to `-13`,
each a refusal of a program those rules define, and all seven closed the same day; `D-match-14`
and `-15` opened and closed with them.  `D-match-6` opened and closed 2026-09-25; `D-match-5` closed 2026-09-14;
`D-match-4` closed 2026-09-12.

The walk's finding is one shape seven times: each rule held on its own and was refused where two
of them COMPOSE — a guard or a field sub-pattern on a multi-pattern arm, a capture inside an
alternation, a capture a later pattern alone binds, a heap field under a repetition, a tail after
a scalar repetition, an iterator of tuples, a tail before a rest.  Each was a refusal worded
*"not yet supported"*, *"(yet)"* or as a plan phase, which is how the register read `OPEN: 0`
over them: a refusal worded as pending work was never entered as a deviation.  Behind two of
them sat a quiet wrong answer the refusal had kept out of sight — a tuple element read as garbage
(`D-match-13`), and a second rest that dropped the first one's binding (`D-match-14`).

Cursor matches (a struct with a `vector` source and a `pos`, consumed as a PREFIX) and sub-rule
invocation `[ name: rule ]` are shipped but have no rules here yet; `D-match-15`'s cursor half
follows `(P-Seq)` as a prefix reading of it.

- **D-match-15 — OPENED AND CLOSED 2026-09-29.** `(P-Seq)` × `(P-Rep)` × `(P-Rest)`: a fixed
  tail after a variant repetition, with a `..rest` after the tail, was refused ("cannot combine
  with `..rest` (yet)"), and so was any tail after a repetition in a cursor match.  By `(P-Seq)`
  the tail follows the run and the rest takes what is left; the lowering only knew a tail read
  from the END, which `(P-Whole)` makes right when nothing follows it.  With a rest after it, or
  in a cursor match, the tail is now read from the run's end and the rest (or the cursor) starts
  after it (`tests/scripts/a-tail-after-a-repetition-follows-the-run.loft`).
- **D-match-14 — OPENED AND CLOSED 2026-09-29.** `(P-Rest)` × `(P-Rest)`: a second rest in one
  slice was accepted and the FIRST one's binding dropped without a word — `[..a, ..b]` bound only
  `b`, to every element, and `a` read as an unknown variable.  Found while settling D-match-12.
  A second rest (or a scalar repetition beside one) is now refused at parse time
  (`two_rests_are_refused`, `scalar_rep_rest_is_a_second_rest` in `tests/parse_errors.rs`).
- **D-match-13 — OPENED AND CLOSED 2026-09-29 (loft#1737).** A `match` over an `iterator<τ>` was
  refused for every `τ` but a scalar, text or struct-enum, where the iterator-input rule
  materialises the subject whatever its element type.  Three defects, one composition each: the
  stream buffer carried its own copy of the comprehension's append and wrote a `vector<τ>`
  element as a scalar (it now appends through the comprehension's `append_element_ops`); a slice
  element over a TUPLE read the element's DbRef instead of unboxing it — garbage on
  `--interpret`, `DbRef as (i64, i64)` on `--native`; and `[(a, b), ..]` was read as an
  alternation, where `(G-Pat-Group)` makes a `( … )` with no pattern operator a tuple pattern
  (`@FR-P-Point`).  `tests/scripts/a-match-streams-an-iterator-of-any-element.loft`.  A
  generator yielding `(integer, text)` stays refused on `--native` — by the GENERATOR, not the
  match (coroutines-history.md D-cor-2).
- **D-match-12 — OPENED AND CLOSED 2026-09-29 (loft#1736).** A `..rest` or a non-literal element
  after a scalar repetition `xs:T*` was refused, over a question the rules had not answered: read
  as `(P-Rep)`'s greedy run, `xs` takes every element and any tail never matches — which the
  shipped literal tail already contradicted, `[xs:integer*, 9]` binding the middle exactly as
  `[..xs, 9]` does.  The owner took the reading the code shipped (2026-09-29): `(P-Rep-Scalar)`
  makes the run the slice's one rest, typed, so a tail of any form is `(P-Rest)`'s `t`, and a
  `..rest` after it is the second rest `(P-Rest)` refuses.  The scalar path is now parsed AS a
  rest rather than beside one (`tests/scripts/a-scalar-repetition-is-a-typed-rest.loft`).
- **D-match-11 — OPENED AND CLOSED 2026-09-29 (loft#1735).** `(P-Rep-Ty)` collects a capture
  inside `(a)*` into a `vector<τ>`; a HEAP field under a repetition (`[(Grp { items })*]`) was
  refused.  Admitting it answered every value right and freed the SUBJECT: the per-element read of
  a heap field is a view into the subject's store, and typed as an owned value its scope-end free
  released the subject's record each iteration, on both backends — a use-after-free a value check
  on the projection alone did not see.  The read now carries the borrow dep a whole element's read carries, and each field
  is deep-copied into the projection (`@FR-H-Alloc`)
  (`tests/scripts/a-repetition-collects-a-heap-field-per-element.loft`).
- **D-match-10 — OPENED AND CLOSED 2026-09-29 (loft#1734).** `(P-Multi)` is `(P-Alt)` at arm
  granularity, so a capture only some listed patterns bind is `τ?` (`(P-Alt-Diff)`), null when
  another pattern matched — as a single-element alternation already answered.  It was refused,
  and so was a capture inside a later pattern's field sub-pattern (`D-match-8`'s refusal).  A
  name a later pattern adds now gets a shared slot of its own, every pattern that lacks a name
  stores null into it, and a name some pattern lacks is typed `τ?` before the arm body is
  parsed.  Guard `tests/scripts/a-name-only-some-listed-patterns-bind-is-nullable.loft`.
- **D-match-9 — OPENED AND CLOSED 2026-09-29.** `(G-Pat-Prec)`'s own example, `a:V | b:W`, did
  not parse: an alternation branch was read as a variant name and failed on the `:`.  A branch's
  capture now joins the alternation's capture unification as a whole-element entry, typed by
  `(P-Alt-Same)` / `(P-Alt-Diff)`.  Guard
  `tests/scripts/an-alternation-branch-may-capture-the-element-it-matched.loft`.
- **D-match-8 — OPENED AND CLOSED 2026-09-29.** `(P-Point)` lets a field be a pattern; in a
  multi-pattern arm that was refused in the first listed pattern and failed to parse in a later
  one.  Each listed pattern's sub-patterns are now its own branch condition.  A sub-pattern in a
  later pattern that BINDS a name was refused by name; `D-match-10` made that name `τ?`.
  Guard `tests/scripts/a-listed-pattern-may-test-a-field.loft`.
- **D-match-7 — OPENED AND CLOSED 2026-09-29.** `(P-Guard)` × `(P-Multi)`: a guard on a
  multi-pattern arm was refused.  Each listed pattern's arm now carries it beside its own
  bindings, and a guarded arm covers nothing (`(M-Total)`).  Guard
  `tests/scripts/a-guard-on-a-multi-pattern-arm-holds-for-the-pattern-that-matched.loft`.

- **PEG patterns are SHIPPED (@PLN35)** — the shipped implementation (phases 1–7 + PC1–PC5,
  [plans/35-match-peg](../plans/35-match-peg/)) conforms to the stated rules on both backends,
  with no standing exception since `D-match-4` closed 2026-09-12. Each rule is pinned by the
  @PLN89 oracle in
  [VERIFICATION.md § matching.md — PEG patterns](VERIFICATION.md).  This bullet read *"opens no
  deviation"* for as long as `(P-Rest)`'s `t` was refused outright, which is the shape of claim
  the rule-led walk exists to re-measure: a conformance line is only as strong as the oracle
  under it, and no oracle case had ever spelled a rest with anything after it.
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
- **An arm names a real variant (`M-Unit`)** — `match c { Red => …, Grean => …, _ => … }` over
  `enum Colour { Red, Green, Blue }` does NOT compile ("'Grean' is not a variant of Colour"),
  whether or not a definition of that name exists elsewhere, and whether or not a `_` would have
  absorbed the subject.
- **A guard covers nothing (`M-Total`)** — `match c { A if x => 1, B => 2 }` over `enum C { A, B }`
  does not compile ("missing: A"), and `match c { _ if x => 1 }` does not compile ("missing: A, B");
  a false guard moves selection to the next arm with no binding left behind (`P-Guard`,
  `P-Atomic`).
- **Scalar fall-through is null (`M-Total`)** — `match n { 1 => 10, 2 => 20 }` with `n = 9` is
  `null`, and the match's type is nullable; no error, no fault.
- **Exhaustiveness (`M-Exhaust`)** — `match c { A => 1 }` over `enum C { A, B }` does NOT compile
  ("missing: B"); adding a `B => …` arm or a trailing `_` makes it compile.
- **As an expression (`M-Expr`)** — `r = match c { A => 100, B => 200 }` binds `r` to the arm's
  value (`100`).
- **A rest need not be last (`P-Rest`)** — `match v { [a, ..mid, z] => … }` over `[1,2,3,4,5]`
  binds `a=1`, `mid=[2,3,4]`, `z=5`; over `[1,2]` it MATCHES with `mid` empty; over `[1]` it does
  not match at all (`head + tail` is 2).  `mid` is a fresh vector, so mutating it leaves `v`
  untouched.  A tail element is any point pattern `(P-Point)` admits — a bare name, `_`, a
  literal, or a variant sub-pattern: `[Kw { word }, .., End { e }]` and `[1, .., 9]` both
  match (D-match-4, closed 2026-09-12).
- **A tuple element is a point pattern (`P-Point`)** — `match (a, b) { (Fire, Wall { status })
  => … }` tag-tests both positions and binds `status` as a view of `b`'s payload; a swapped
  pair falls to `_`, and `(Bogus, _)` over an enum element is a compile error naming the enum
  (D-match-5, closed 2026-09-14).

D-op-1's falsifier applies: any program where the interpreter and `--native` disagree on which
arm a `match` selects, on a bound payload value, or on whether a match is exhaustive is the
definitional error this doc names.
