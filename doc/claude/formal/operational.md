<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/operational.md — small-step semantics for the stable core (strict)

**Catalogue:** @F38 (arithmetic safety), @F1 (null model), @F3 (scalar core) — the Goal-D backend contract. Roadmap: @PLN28, @PLN89.

> **Rules then deviations** (see [README](README.md)). This is a small-step evaluation
> relation for loft's **stable scalar core**, written for one purpose: to be the **shared
> contract both backends must satisfy**. Today the interpreter (`src/state/`) *is* the
> de-facto spec and the native generator (`src/generation/`) is a separate implementation
> kept in agreement only by tests — so a disagreement is a test gap, not a definitional
> error. These rules turn that around: a program where the two backends step differently
> is **by definition** a bug in whichever one disobeys.
>
> Rough spot #3 from [FORMALIZATION.md](../FORMALIZATION.md). Pinned-behaviour sources:
> [LOFT.md § null](../LOFT.md) (in-band sentinels) and [LOFT.md § Arithmetic safety](../LOFT.md)
> (overflow / divide-by-zero — under C80, these yield **null and continue**; `??` is the
> null-fallback). Scope note: this file covers the scalar core (values, arithmetic, the
> uncomputable→null discipline, evaluation order, assignment, `if`, sequencing). The rest of
> the operational semantics is split into sibling files (all 2026-07-04): [heap.md](heap.md)
> (store alloc/read/write/copy/free), [iteration.md](iteration.md) (`for` + combinators),
> [coroutines.md](coroutines.md) (generators), [concurrency.md](concurrency.md) (`par`),
> [calls.md](calls.md) (function call/return + parameter binding), [matching.md](matching.md)
> (`match` + exhaustiveness; + PEG patterns and the two new ops `OpMatchAnchor` / `OpMatchRevert`,
> @PLN35 SPEC-FIRST), [tuples.md](tuples.md), [closures.md](closures.md) (lambdas /
> closures / fn-refs), [formatting.md](formatting.md) (`"{x}"` interpolation + value→text
> rendering), and [interfaces.md](interfaces.md) (interfaces + generics — a static/typing area).
> Every sibling file is now at **0 own deviations** (closures' D-clo-1/2 closed 2026-07-04;
> formatting + interfaces written 2026-07-05). The operational contract is now written across the
> whole family — nothing is left to "the interpreter is the spec" except the differential-oracle
> meta-deviation (D-op-1) itself.

## Notation

- `σ` — the **store/environment** (variable ⟼ value, plus the heap).
- `⟨e, σ⟩` — a **configuration**: expression `e` to evaluate in store `σ`.
- `⟨e, σ⟩ → ⟨e', σ'⟩` — one **small step**.
- `v` — a **value**: an `integer` (64-bit), `float`, `boolean`, `character`, `text`, a
  heap reference, or **`null`**.
  (There is no trap/halt step in the core: an uncomputable result is the value `null`, not a
  halt — see `E-Uncomp`. The only runtime halts are the *explicit* `panic`/`assert` in
  dev/test, which are statements outside this scalar core.)

---

## Rules

### Values and null

```
  (E-Val)    a value v does not step (it is a normal form).
  (E-Null)   `null` is a value, represented IN-BAND by a specific reserved bit-pattern per
             scalar width — `integer` = `i64::MIN`; a narrow int = its top value (`u8` = 255);
             `float`/`single` = a reserved NaN; `character` = codepoint 0; a reference =
             `nullref`.  That pattern is a REAL, OBSERVABLE value, and it is RESERVED: it is
             EXCLUDED from the non-null range of `τ` (so a non-null `integer` is
             `[i64::MIN+1, i64::MAX]`, symmetric).  No legitimate non-null value equals it.
             Both backends MUST agree on the abstract value AND on the reserved pattern per
             width (the pattern is part of the observable contract, not a private encoding).
```

**In words.** A value is "done" — it doesn't evaluate further. `null` is a real value, not a
separate state; each scalar **width** reserves ONE bit-pattern for it (an `integer` null is
the smallest `i64`; a `u8` null is 255; a `float` null is a reserved NaN). That pattern is
**in-band and observable** — it is a value in the same slot — so the non-null value **range
excludes it** and no real value can silently be confused with null. (This corrects the earlier
claim that "how a backend encodes the sentinel is its business": the encoding is in-band, so it
IS observable and part of the frozen contract — see the null-sentinel keystone,
[plans/102-stability-contract/keystone-null-model.md](../plans/102-stability-contract/keystone-null-model.md).)
The cost — a *nullable* narrow type cannot store its one reserved value (`u8?` has no 255,
`integer?` no `i64::MIN`, `character?` no `'\0'`) — is a deliberate, documented limitation of
the in-band model, not a silent one.

### Evaluation order — left to right

```
  (E-Left)   in a binary form `e₁ op e₂` (op NOT short-circuiting), reduce e₁ to a value
             first, then e₂:
                 ⟨e₁, σ⟩ → ⟨e₁', σ'⟩   ⟹   ⟨e₁ op e₂, σ⟩ → ⟨e₁' op e₂, σ'⟩
                 ⟨v₁ op e₂, σ⟩ → ⟨v₁ op e₂', σ'⟩   when   ⟨e₂, σ⟩ → ⟨e₂', σ'⟩
  (E-And)    `e₁ && e₂` reduces e₁ first; if e₁ is false the whole form is false and e₂ is
             **NOT** evaluated (short-circuit); otherwise the form reduces to e₂.
  (E-Or)     `e₁ || e₂` reduces e₁ first; if e₁ is true the whole form is true and e₂ is
             **NOT** evaluated; otherwise the form reduces to e₂.
```

**In words.** Operands evaluate left first, then right — so any side effects (a call that
mutates the store) happen in source order. Both backends must use this order. The **only**
exception is the short-circuiting logical operators `&&`/`||` (and their `and`/`or` spellings):
they reduce the left operand, and evaluate the right operand *only* when the left has not
already decided the result — verified on both backends. Every other binary op (arithmetic,
comparison, `??`) evaluates both operands under E-Left.

### Arithmetic — uncomputable yields null (the spreadsheet model)

```
  (E-Op)        ⟨v₁ op v₂, σ⟩ → ⟨v, σ⟩          where v = v₁ op v₂ is representable
  (E-Uncomp)    ⟨v₁ op v₂, σ⟩ → ⟨null, σ⟩       where the result is NOT computable — `v₁ op v₂`
                                                overflows the type, or op is `/`/`%` with
                                                v₂ = 0.  The result is **null**; evaluation
                                                CONTINUES (it never halts).
                                                FLOAT and SINGLE are the exception, because
                                                for them "not computable" is decided by IEEE
                                                and not by this rule: `5.0 / 0.0` is `inf`, a
                                                REPRESENTABLE value, so it is not null.  Only
                                                what IEEE makes a NaN is — `0.0 / 0.0`, and
                                                `x % 0.0` — since the float null IS the NaN
                                                (E-Null).  Overflow follows the same line:
                                                `1.0e308 * 10.0` keeps `inf`.
  (E-Uncomp-NN) ⟨v₁ op v₂, σ⟩ → ⟨default(τ), σ⟩ where the result is not computable AND the
                                                target type τ cannot HOLD null (a non-nullable
                                                declared range).  (E-Uncomp) answers null, and
                                                a slot that cannot hold null cannot take that
                                                answer, so the next best thing is τ's DEFAULT —
                                                the value the program would have had if nobody
                                                had assigned: `0` for every integer width whose
                                                range admits it, else the bound nearest zero.
                                                NEVER a value derived from the machine
                                                representation.  A wrapped, truncated or
                                                reinterpreted result is the WORST answer
                                                available: it is in range, type-correct, and
                                                unrelated to the computation, so nothing
                                                downstream can tell it from a real one.  The
                                                processor's behaviour is an implementation
                                                detail and never the semantics.
  (E-NullArg)   any op with a `null` operand produces `null` (null is CONTAGIOUS),
                EXCEPT the two families below.
                COMPARISONS are DEFINITE against the reserved pattern and UNIFORM across
                every scalar type:
                  `null == null` → true;  `v == null` / `null == v` → false (v non-null);
                  `!=` is the exact complement of `==`;
                  equality holds for `integer`, `character`, `float`, `single`, `boolean`
                  and `text` alike;
                  ordering (`<` `>` `<=` `>=`) places `null` at the LOW extreme —
                  `null < v` → true, `v < null` → false, `null < null` → false —
                  the SAME for every ORDERED type: `integer`, `character`, `float`,
                  `single`, `text`.  `boolean` carries equality without ordering
                  (see below), so the ordering half does not reach it.
  (E-Truthy)    a TRUTHINESS position reads `null` as `false` and yields a DEFINITE
                two-state boolean.  The positions are the `if` / `while` / `assert`
                condition, `!e`, and BOTH operands of `&&` / `||` — and nothing else:
                elsewhere a `null` boolean stays null under (E-NullArg).  So
                `true && null` is `false`, not `null`, and the result of `&&`/`||`
                never holds the boolean null sentinel.
```

**In words.** Arithmetic gives the obvious result when it fits. When it *can't* — overflow,
divide/modulo by zero — it yields **null** and the program **keeps running**; it does not
halt.  Where the value has to land somewhere that cannot hold null — a non-nullable declared
range — it takes that type's **default** instead (E-Uncomp-NN), which is `0` wherever the
range admits it.  What it is never allowed to be is whatever the hardware happened to leave
in the register: a wrapped or truncated result is in range, type-correct and unrelated to the
computation, which makes it indistinguishable from an answer.  Null says "this did not
happen"; the default says "this did not happen and I could not tell you"; a wrapped number
says nothing at all, and says it convincingly.  *"I could not tell you"* is the half
(E-Uncomp-Seen) removes: an author who cares writes `if !place` on the next line and is
told, while the value the slot holds is untouched — so the default stays the default for
every program that says nothing. Comparisons are the first exception to contagion: they let you *test* for null
(`x == null`) and give a **total order** with null sorting first, and this is **uniform across
scalar types** — `null == null` is always true, never type-dependent. (`float`/`single` null was
a NaN, so `null == null` used to be false and ordering unordered — deviation D-op-null-1, CLOSED
by keystone step 2 (2026-07-10); both are now uniform with the integer/char behavior.)

**`boolean` has equality but no ordering, and the ordering clause used to claim it anyway.**
`null == null`, `null == false` and `!=` all answer per the rule on a `boolean?`, but `<` on two
booleans is REFUSED at compile time — *"No matching operator '<' on 'boolean' and 'boolean'"* —
because no `OpLtBool` exists and `default/01_code.loft`'s `Ord` interface lists
`integer`/`single`/`float`/`text` and deliberately not `boolean`. That is a decision, not a gap:
`false < true` is a convention a program should have to spell out. The rule listed `boolean` in
the ordering clause for its whole life regardless — an over-claim measured and corrected by the
`@FR-E-NullArg` walk (2026-08-29), which is the kind of thing an uncited rule accumulates.

**The second exception is truthiness (E-Truthy).** `&&`, `||`, `!` and the `if`/`while`/`assert`
condition are not contagious — they read a null operand as `false` — which is
[DESIGN_DECISIONS.md C73](../DESIGN_DECISIONS.md)'s three-state boolean, and is why `&&`/`||`
type as the non-null `boolean`. It is a genuinely separate exception from comparisons and was
simply missing here, so E-NullArg read as forbidding behaviour the language ships and documents.
See D-op-6 for what that cost.

**A `match` arm GUARD is deliberately NOT one of those positions, and that is not an oversight
to file.** It REFUSES a nullable — *"guard must be boolean, got boolean?"* — where the
neighbouring `if` coerces. The rule says "and nothing else" for exactly this reason: a guard
chooses between ARMS, so a null silently reading as "skip this arm" picks a different branch
with nothing said, while an `if`'s two outcomes are both written at the site. The cure is
spellable (`mb() ?? false`, `mb() == true`) and the diagnostic names it. Every listed position
was verified on both backends; the guard's refusal was too.

**Float division by zero is `inf`, and that is a decision, not an oversight.** `(E-Uncomp)`
read as covering it for years and the code never did; the carve-out above was added by the
`@FR-E-NullArg` walk (2026-08-29) after the rule sent it hunting a bug that was not there —
the third time an incomplete rule in this family produced a false positive. The reason is
recorded at `OpDivFloat` in `default/01_code.loft`: forcing NaN (loft#983) made ONE expression
answer two things — `inf` inline, where the `*Nullable` peer is emitted, and `null` once bound
or returned — split division from float OVERFLOW, which keeps `inf` in every position, and
made `a / b ?? 0.0` select the peer that never yields null, so the idiom every numeric library
uses to defend a divide guarded nothing. Integer `/0` stays null: an integer has no `inf` to
answer with.

**Under all of it is the spreadsheet model** ([DESIGN_DECISIONS.md C80](../DESIGN_DECISIONS.md)):
a cell that can't compute shows null and never stops the other cells. A fault is *local* — it
degrades one value, never the whole run. The same holds for every uncomputable step (an
out-of-bounds index, a deref of an absent value): null, continue.

**Float `==` is exact, never epsilon.** For two *non-null* floats, `==`/`!=` compare the IEEE
values exactly — `1.0 == 1.0000000001` is **false**, `0.1 + 0.2 == 0.3` is **false** (the sum is
`0.30000000000000004`), and `!=` is the exact complement of `==`. There is no tolerance band. The
ordering operators (`<` `<=` `>` `>=`) agree with it: among non-null floats they form a **total
order** — NaN cannot occur (it is represented as null, D-op-null-1), so exactly one of `a < b`,
`a == b`, `a > b` holds for every pair, and no value is ever both `<` and `==` its neighbour.
`single` (32-bit) behaves the same. Verified both backends.

### Integer division and modulo — truncate toward zero, dividend-sign remainder

```
  (E-Op-IntDivMod)   refines (E-Op) for integer `/` and `%`: `/` TRUNCATES toward zero and
                     `%` returns the remainder with the SIGN OF THE DIVIDEND, so
                     `a == (a / b) * b + a % b` holds for every non-zero `b`:
                       -7 / 2  == -3      7 / 2  == 3       -7 / -2 == 3
                       -7 % 2  == -1      7 % -2 == 1       -7 % -2 == -1
                     (E-Uncomp still governs `b = 0`: both `/` and `%` yield null.)
```

**In words.** Integer `/` drops the fractional part toward zero rather than flooring toward
`-∞` — `-7 / 2` is `-3`, not `-4` — and `%` always carries the sign of the **dividend**, its
matching remainder (`-7 % 2` is `-1`, `7 % -2` is `1`). This is the C/Rust convention, one of
two legitimate, self-consistent choices (the other floors, as Python/Ruby do) — so it is a
place a backend could silently diverge (Rust's native `/`/`%` already truncate; an
interpreter or a future backend implemented against a floor-toward-`-∞` intuition would not),
which is exactly why it belongs in the shared contract rather than being left "the obvious
result when it fits." The stdlib `floor_mod(a, b)` is the companion for the cyclic case that
genuinely wants the divisor's sign and a `[0, b)` result — `floor_mod(-7, 2) == 1` — so
circular indexing (`grid[(i - 1).floor_mod(w)]`) never silently reads a negative index the
way a bare `%` would. [DESIGN_DECISIONS.md C94](../DESIGN_DECISIONS.md) (commit `a2eaba66`);
verified both backends.

### `??` — a non-null fallback (no trap mode)

```
  (E-Coalesce)   ⟨e ?? d, σ⟩ → ⟨v, σ⟩   if  e → v  with v ≠ null
                 ⟨e ?? d, σ⟩ → ⟨d, σ⟩   if  e → null
```

**In words.** `??` supplies a fallback for a null: `(a * b) ?? 0` is "a*b, or 0 if it couldn't
compute." There is **no** context-dependent "trap-suppression mode" any more — an op yields
null whether or not it sits under `??` (C80); `??` just decides what to do with that null.
(This is what closes the old D-op-3.)

### `x?` — a fallback to the TYPE's default (@PLN116)

```
  (E-Default)   ⟨e?, σ⟩ → ⟨v, σ⟩                        if  e → v  with v ≠ null
                ⟨e?, σ⟩ → ⟨construct_default(τ), σ'⟩    if  e → null
                                                        where τ is e's STATIC type and, for a
                                                        record / collection τ, σ' is σ extended
                                                        with a FRESHLY constructed value
```

**In words.** `x?` is `x ?? <the default the type gives>`, so it reduces exactly like
`(E-Coalesce)` except that the fallback is not written at the site — it comes from the type.
`construct_default` and its domain `has_default` are defined once in
[types.md § Defaults](types.md); `x?` on a type with no default does not reach this rule at
all, because it is rejected at compile time.

Two things this rule is pinning that are easy to get wrong:

- **The fallback is chosen by the STATIC type, not the runtime value.** There is no value to
  inspect — `e` reduced to `null`, which carries no type. So `(E-Default)` is the one
  evaluation rule that reads the typing derivation, and a backend must therefore resolve the
  default at compile time. (`(E-Coalesce)` needs no such thing: its fallback is an
  expression already in the program.)
- **The default is a FRESH value, never a shared one.** For a record or collection, each
  evaluation of `e?` constructs its own — `points[i]?` twice yields two distinct `Point{}`s.
  A shared singleton would alias: mutating one discharge's result would be visible through
  another's, which the ownership model ([ownership.md](ownership.md)) forbids for a value
  the expression owns.

**Falsifying program.** The two discharges sit at opposite ends of the precedence ladder
([grammar.md `(G-Post-Default)`](grammar.md)), so on a *binary* result they need opposite
parenthesisation — the one place they are not interchangeable by eye:

```loft
a = 10;  b = 0;
a / b ?? 0      // 0     — `??` is LOOSEST, so it discharges the division
(a / b)?        // 0     — same, parenthesised
a / b?          // null  — `?` is TIGHTEST: this is `a / (b?)`, and the DIVISION is undischarged
```

### Observability — report a fault only where it is UNGUARDED

```
  (E-Report)   an UNGUARDED uncomputable divide/modulo-by-zero ALSO emits a Warn-level
               log (`divide_by_zero`) — the "no guard" signal — before yielding null.
               A GUARDED site (the operand of `??` / a following null-check) emits the
               silent `*Nullable` op and reports NOTHING (the guard owns the null).
               Integer OVERFLOW is silent at every site (the null IS the signal — also
               the rustc-release default); the value is null, never a wrapped wrong answer.

  (E-Uncomp-Seen)  where (E-Uncomp-NN) applies — the target type cannot hold null, so the
               slot took `default(τ)` and the null IS NOT the signal — the failure is
               nonetheless OBSERVABLE, by `!place` in the condition of the `if` that is the
               statement immediately after the store.  It answers true exactly when
               (E-Uncomp-NN) fired for that store and false otherwise, and it changes
               NOTHING else: the slot holds the same `default(τ)` it holds with no test
               written, and the store is then a GUARDED site under (E-Report).
               One statement further apart the observation is not available, because the
               only thing that could carry it is the place, and the place is storage —
               a status bit beside every element would cost the density the narrow widths
               are declared for.  Absent the test, nothing is produced.
```

**In words.** The fault stays a *value*, never a halt (E-Uncomp), but loft is not blind to it:
an uncomputable you did **not** defend — a bare `a / 0` — also writes one Warn log so it is not
invisible, while a site you *explicitly* defended (`a / b ?? 0`) is silent because you already
said how to handle it. Overflow is silent everywhere — common enough that a per-site log would
be spam, and the null result already shows it. The Warn is **silent on a default CLI run** (no
logger attached) and surfaces when a logger is — which is how a test *validates* the fault fired
(see `runtime_logging.rs::prod_divide_by_zero_logs_and_continues`). The opt-in `--dev-soft-halt`
debug flag still surfaces these recoverable faults (uniformly: div0, overflow, OOB) for one-shot
breakage triage — it is an explicit debugging tool, NOT a dev/test/prod mode, so it does not break
E-Uncomp's mode-independence.

### State steps

```
  (E-Var)    ⟨x, σ⟩ → ⟨σ(x), σ⟩
  (E-Asgn)   ⟨x = v, σ⟩ → ⟨v, σ[x ↦ v]⟩                 (the LHS place reduces first —
                                                        left-to-right — THEN the RHS, by E-Left)
  (E-Seq)    ⟨v ; s, σ⟩ → ⟨s, σ⟩
  (E-IfT)    ⟨if true { s } else { t }, σ⟩ → ⟨s, σ⟩      (and E-IfF for false)
```

**In words.** A variable steps to its stored value; an assignment reduces its left-hand
place first (left-to-right), then its right side, then updates the store; a sequence drops a
finished statement; an `if` picks the branch
its (already-evaluated) condition selected. Standard — pinned here only so both backends
share them.

### Compound assignment — the place evaluates exactly once

```
  (E-Asgn-Compound)   ⟨place op= e, σ⟩ steps as:
                      1. reduce place's ADDRESSING sub-expressions — index exprs, a
                         container-producing call — to values, EXACTLY ONCE, binding
                         each to a hoisted temp slot t̄ (left-to-right, by E-Left);
                      2. ⟨t̄, σ⟩ → ⟨v₁, σ⟩                       (read through t̄)
                      3. ⟨e, σ⟩ → ⟨v₂, σ⟩                       (reduce the RHS)
                      4. ⟨v₁ op v₂, σ⟩ → ⟨v, σ⟩ or ⟨null, σ⟩    (by E-Op / E-Uncomp)
                      5. σ' = σ[t̄ ↦ v]                          (write through the SAME t̄)
                      ⟨place op= e, σ⟩ → ⟨v, σ'⟩
```

**In words.** `place op= e` is not the naive desugaring `place = place op e` — that would
reduce `place`'s addressing sub-expressions **twice** (once to read, once to write), and if
one of them is side-effecting or divergent (`w[next()] += 5` where `next()` returns 1 then 2),
the read and the write land on **different slots** — a plausible-looking, silently wrong
result with no error. Instead, the addressing sub-expressions — an index, or a call that
produces the container/struct being indexed — are evaluated **exactly once** and bound to a
temp; both the read and the write go through that same temp. This holds for **every** place a
compound assignment can target, including a struct field at a nonzero offset reached through
a call-index (`w[idx()].y += 5` calls `idx()` once, and reads/writes the same element's `y`)
and nested indices (`m[i()][j()] += n` evaluates `i()` once and `j()` once, not per read/write
or per nesting level). Contrast `E-Asgn`: a plain `x = v` already writes its place once, so
there is no double-eval to close. [DESIGN_DECISIONS.md C92](../DESIGN_DECISIONS.md); verified
both backends — `tests/scripts/pln102-f2-place-once.loft`.

### Compound assignment through a discharge — the `?` is on the READ

```
  (E-Asgn-Discharge)  ⟨place? op= e, σ⟩  →  ⟨place op= e, σ'⟩   where
                      σ' = σ[place ↦ construct_default(τ)]   when σ(place) = null
                      σ' = σ                                 otherwise
                      — and `place` is addressed EXACTLY ONCE, by (E-Asgn-Compound).
                      A discharge is not a place: `place?` is a VALUE, and what the `?`
                      says on the left of an assignment is which value to READ when the
                      place is null.  The write lands in `place`.
                      `place? = e` therefore means `place = e` — a plain assignment has
                      no read to discharge.
                      An explicit `(a ?? d)` names two values and no place; it takes no
                      assignment at all.  ENFORCED at the assignment dispatcher, above
                      `assign_var_nr` — reached any later, a `text` target is an ICE
                      rather than a diagnostic (loft#1212).
```

**In words.** `x? += 3` on a null `x` is `3`, not null: the `?` picked what to read, the
statement still writes `x`. It is the accumulate-from-the-zero idiom, and it composes with
the once-only rule above — the place's addressing is evaluated once for the read, the
discharge and the write together. For a COLLECTION the discharge is what `op=` already
does (appending to a null collection builds the empty one first), so `b.d? += [r]` and
`b.d += [r]` agree; for a scalar or `text` they differ, because a bare `op=` PROPAGATES
(`(N-Prop)`: `null + 3` is null) and that difference is the whole reason to write the `?`.
Delivering the discharge reads the place a second time — once to ask whether it is null,
once to write the default in — so it is read through the temp (E-Asgn-Compound) already
binds, never off the spelling the author wrote. `w[idx()]? += 1` therefore calls `idx()`
**once**, and the discharge and the operation land on the element that one call named; read
off the original spelling instead it calls twice and reads one element while writing the
next, which is the corruption the once-only rule exists to prevent.
[DESIGN_DECISIONS.md C92](../DESIGN_DECISIONS.md), [types.md (N-Default)](types.md);
verified both backends — `tests/scripts/1205-a-discharged-place-writes-through-to-its-place.loft`.

**A discharge INTERIOR to the place is a different question, and this rule does not reach it.**
`h.i?.x = …` discharges `h.i` and then names `.x`, so the target is an interior place
(`H-View`) and the statement is an ordinary write — there is no `place?` at the top for the
rule above to rewrite. What still has to hold is that the write RESOLVES to the binding it
reaches: `h.i?.x = …` roots at `h` exactly as `h.i.x = …` does, so
[binding.md (Const-Value)](binding.md) keeps rejecting it on a `const h`. While the resolver
stopped at the discharge and answered "no binding at all", it did not — a `const` parameter
was mutated in silence and the caller saw the new value, on both backends, with no diagnostic
(loft#1211). One home answers what a discharge was applied to for both questions, so which
place a discharged TARGET was reading and which binding a write THROUGH one reaches cannot
drift apart; verified both backends —
`tests/scripts/1211-a-const-binding-holds-through-an-interior-discharge.loft`,
`tests/scripts/1211b-a-place-behind-an-interior-discharge-is-written.loft`.

---

## Deviations

**OPEN: 2.**
- **D-op-1** — there is no shared operational semantics — the interpreter is the spec
- **D-op-2** — interpreter/native divergences are test-caught, not definition-caught

The full register — these entries in full, plus every closed one with its dates and
issue numbers — is the companion [operational-history.md](operational-history.md).

## Conformance

The pinned rules are checkable directly: `5 / 0` is **null** and execution continues (an
unguarded site also logs a `divide_by_zero` Warn — **only with a logger attached**, i.e.
`--log-conf`/a `log.conf` beside the script; `raise_recoverable` is a no-op when
`database.logger` is `None`, so a bare run shows nothing and that silence is not a
counter-example to this rule); `a + 1` at `a = i64::MAX` is **null** and
continues; `(i64::MAX + 1) ?? 0` is `0` (E-Coalesce); `integer` null is `i64::MIN`.
(E-Truthy) is checkable the same way and needs the RAW compare to see it: `b: boolean =
true && maybe()` is `false`, so `b == false` is true and `b == null` is false — `!b` cannot
tell those apart, because `!` is itself a truthiness position. Guard
`tests/scripts/a-boolean-operator-answers-a-definite-boolean.loft`.
D-op-1/D-op-2's falsifier is any program where the interpreter and `--native` disagree —
e.g. #433's cbor `read_value` (interp `20`, native E0308 pre-fix). When the rules become the
shared oracle, that disagreement is the definitional error, and this doc is the definition it
fails against.
