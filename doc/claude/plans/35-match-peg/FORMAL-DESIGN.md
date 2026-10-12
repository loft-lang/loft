// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# @PLN35 — Formal Design: what the strict spec must add for PEG match patterns

> **Read before building** (the user's directive). This is the design of the changes
> to loft's **strict formal definition** (`doc/claude/formal/`) that PEG match
> patterns require. It is the contract the [IMPLEMENTATION.md](IMPLEMENTATION.md)
> phases are built to satisfy; [README.md](README.md) is the informal design draft.
> The order is deliberate: **fix the rules first, then build to them** — per
> `formal/README.md`, "the code changes to match the rules."

---

## 0. Framing — spec-first, conformance-differential

`formal/` normally *describes existing code* and tracks **deviations** (code-breaks-rule)
down to zero. PLN35 is a **new** feature, so we use `formal/` in **spec-first** mode: we
write the rules as the **build-to target**, and there are *no* code-breaks-rule deviations
to open (nothing is implemented yet to break them). Instead each new rule is a **pending
obligation**, discharged when its IMPLEMENTATION phase lands and an oracle program pins it.

- **Conformance stays differential** (`operational.md` D-op-1): there is no second executable
  semantics; both backends must **agree** on value / null / halt / stdout / leak for a corpus
  of programs (the @PLN89 oracle, `tests/differential_oracle.rs` + `tests/oracle/*.loft`). Each
  PLN35 rule graduates a falsifying program into that corpus. This is tracked in
  `formal/VERIFICATION.md` ("rules to pin"), **not** in `formal/ROADMAP.md` (which tracks
  deviations to close).
- **PEG is unambiguous** (ordered choice, greedy repetition, first-match commit), so the
  differential contract is *tractable*: there is a single defined result per input, which is
  exactly what makes "both backends agree" a checkable property rather than a coin toss.

---

## 1. The ONE invariant PLN35 must not break

`matching.md`'s load-bearing guarantee, verbatim:

> "a `match` can never fall through to nothing at runtime, so there is no 'unmatched value'
> runtime error in loft's model; the exhaustiveness is discharged **statically**." (`M-Exhaust`)

**INV-Total (north star).** *Every `match` selects an arm at runtime; totality is proven at
compile time.* Today enums satisfy it by covering all variants or a trailing `_`.

PEG patterns are **inherently non-total** — `[a, b, c]` rejects a length-4 slice; `(a | b)`
rejects when neither matches; `V { f: Inner { … } }` rejects a different inner shape. So the
extension has exactly one job: **preserve INV-Total.** The only sound way is —

> **A match containing any non-total pattern is well-formed iff its final arm is total**
> (a `_`, a bare binding, or a complete enum-variant cover). Enforced statically.

This replaces `M-Exhaust` with the more general **`M-Total`** (§3.1). It is *the* design
constraint; every construct below is checked against it.

Two supporting invariants make the backtracking sound:

- **INV-Pure.** A **failed** pattern makes **no observable store change** — cursor and
  provisional bindings roll back atomically; the arm body runs **only** on commit at `=>`.
  (Formal rule `P-Atomic`.) Falsifier: a program where a partially-matched-then-failed
  alternative leaves a binding or a moved cursor visible to a later arm.
- **INV-Det.** Ordered-choice / first-match / greedy repetition select the **same** arm and
  bind the **same** values on **both backends** (oracle-pinned, D-op-1). Falsifier: any program
  where interp and `--native` pick different arms or different captures.

The three falsifiers above are the Phase-0 probes in IMPLEMENTATION.md; INV-Total's is the
one that must hold from the very first phase.

---

## 2. The clean result — captures need NO new type former; slices need NO new op

Reading the spec collapses the formal surface twice over:

1. **Capture typing lands entirely on existing formers.** Every PEG capture produces one of
   `τ`, `τ?`, or `vector<τ>`, unified by the **existing join `⊔`** and the **existing**
   nullable-join `(N-Join)` (`types.md`). No new `Type` constructor:
   | capture form | type | existing rule reused |
   |---|---|---|
   | `name:pat` (always taken) | τ (pat's result) | plain synthesis `(T-Syn)` |
   | `(a \| b)` both bind `n` | `τ_a ⊔ τ_b` | the join `⊔` (`types.md`); compile error if no join |
   | `n` bound in only some alternatives | `τ?` | `(N-Join)` — "optional iff some branch lacks it" |
   | `(a)?` captures | `τ?` | the optional former `(N-Opt)` |
   | `(a)*` / `(a)+` capture | `vector<τ>` | `(N-Dense)` vector |
   | `..rest` | `vector<τ>` (subject elem type) | `(N-Dense)` vector |
   This is the single strongest simplification: PEG capture typing is a set of **additive
   synthesis rules over the type relation loft already has**.

2. **Slice/vector backtracking reuses `iteration.md`'s cursor.** A slice cursor **is**
   `iteration.md`'s iterator `⟨i, src⟩` with `elem(src,i)` / `len(src)` (both `null` past the
   end, `I-Next`/`I-Done` — **never fault**). "Anchor" = save `i`; "revert" = restore `i` —
   pure `operational.md` `assign`/`if`/`loop`. **No new operational primitive for
   vectors/slices**, hence no new opcode (matches IMPLEMENTATION.md finding F2).

The **only** genuinely new operational primitive is the **iterator** cursor (L3.6): a source
you cannot random-access, so a failed alternative must **replay** pulled items — the
`Lexer::memory` + `links`-refcount model (`src/lexer.rs`). That is where — and *only* where —
`OpMatchAnchor` / `OpMatchRevert` enter the operational semantics.

**Consequence for sequencing:** L2 → L3.4 are *pure spec extensions over existing rules*;
L3.6 is the one phase that adds a new small-step primitive to `operational.md`.

---

## 3. Per-document changes

### 3.1 `formal/matching.md` — PRIMARY (scope grows: enum-dispatch → match + patterns)

The doc is today 106 lines, narrowly "enum-variant dispatch + payload binding." It becomes the
home of the **pattern-match relation**. (If it grows past ~250 lines, split the pattern relation
into a new `formal/patterns.md` that `matching.md`'s `M-Match` delegates to — decide at write
time.)

**New notation.** A **cursor** `κ = ⟨i, src⟩` (reuse `iteration.md`). The **pattern-match
relation**:

```
  ⟨pat, κ, σ⟩ ⇓ Match(binds, κ')          pat matches, consuming κ→κ', binding binds
  ⟨pat, κ, σ⟩ ⇓ Fail                        pat does not match; κ and σ UNCHANGED (INV-Pure)
```

**Generalize `M-Match`** (arm selection) to run the relation:

```
  (M-Match)   ⟨match v { pat₁ => b₁, … }, σ⟩ → ⟨bₖ[binds], σ⟩
              where k = the SMALLEST index with ⟨patₖ, ⟨0,v⟩, σ⟩ ⇓ Match(binds, κ') AND κ' is
              END (whole-input consumed) — a partial match is NOT a selection (see P-Whole).
```

**New pattern rules** (each a user-visible contract, both-backends):

```
  (P-Lit/P-Var/P-Wild/P-Bind)  unchanged from today's M-Unit/M-Variant/M-Wild, lifted into ⇓.
  (P-Seq)    ⟨[p₁ … pₙ], κ, σ⟩: run p₁ from κ→κ₁, …, pₙ from κ_{n-1}→κₙ; if ANY pᵢ ⇓ Fail, the
             whole sequence ⇓ Fail (κ unchanged — INV-Pure).  Binds = ⋃ binds_i.
  (P-Whole)  an ARM's pattern must consume the WHOLE input: κ' = ⟨len(src), src⟩.  A pattern that
             matches a proper PREFIX ⇓ Fail for arm-selection UNLESS it ends in `..rest`, which
             absorbs the remaining sub-slice.  (This is why today `[a,b,c]` needs exact length.)
  (P-Alt)    ⟨(a | b), κ, σ⟩: try a from κ; if Match, that; else (a ⇓ Fail) try b from the SAME κ.
             Ordered choice — FIRST success wins; if both Fail, ⇓ Fail.
  (P-Opt)    ⟨(a)?, κ, σ⟩: try a from κ; on Match(bs,κ') ⇓ Match(bs,κ'); on Fail ⇓ Match(bs↦null, κ)
             — succeeds with a's captures null, cursor UNMOVED.  (P-Opt never Fails.)
  (P-Rep)    ⟨(a)*, κ, σ⟩: greedily match a from κ→κ₁→…; on the first Fail at κ_m, ⇓ Match(collected,
             κ_m) — stop, cursor at the last success.  `(a)+` = a then (a)*.  A separator
             `*(s)` consumes s between iterations (not captured).  BOUNDED by len(src) for slices
             ⟹ terminates; for iterators, by a `max_lookahead` arm attribute (L3.6).
  (P-Cap)    ⟨name:p, κ, σ⟩: run p; on Match(bs,κ') ⇓ Match(bs ∪ {name ↦ p's result}, κ').
  (P-Rest)   ⟨..name, κ=⟨i,src⟩, σ⟩ ⇓ Match({name ↦ FRESH vector of src[i..len-t]}, ⟨len-t, src⟩)
             where t = the count of fixed patterns after the rest.  H-Alloc (a new store).
  (P-Multi)  a MULTI-PATTERN arm `pat_a, pat_b => body`: try pat_a from ⟨0,v⟩; if Match(+Whole)
             commit; else try pat_b; first whole-match commits.  (No new cursor work — P-Alt at
             arm granularity.)
  (P-Atomic) ⟨pat, κ, σ⟩ ⇓ Fail  ⟹  σ is UNCHANGED and κ is not advanced (INV-Pure).  Provisional
             captures written during a failed attempt are never observable — the arm body runs
             ONLY after a committed whole-match.
```

**Replace `M-Exhaust` with `M-Total`** (INV-Total):

```
  (M-Total)  define total(pat):
               total(_) = total(bare name) = true
               total(V) / total(V{f…}) = true iff every sub-pattern of the fields is total
               total(sequence / alternation-without-full-cover / optional-in-required-position /
                     repetition / length-constrained slice / literal) = false
             a match is EXHAUSTIVE (⟹ INV-Total) iff EITHER
               (enum subject) its total arms cover every variant,   OR
               its FINAL arm's pattern is total (a `_`, a bare binding, or a full variant cover).
             A match with any non-total pattern and no total final arm is a STATIC ERROR
               ("match is not exhaustive — a structural pattern can fail; add a `_` arm").
```

This **preserves** the promise "no unmatched-value runtime fault": a non-total arm may `⇓ Fail`
at runtime, but a total final arm always fires. `M-Wild` (the `_`-must-be-last rule) is unchanged.

### 3.2 `formal/grammar.md` (+ `LOFT.md` § Summary of grammar) — pattern grammar

`grammar.md` pins operator precedence and the non-CFG points; the pattern **productions**
belong in `LOFT.md`'s grammar summary, and `grammar.md` gains the **pattern-operator
precedence** (a second, small ladder, loosest first):

```
  pattern-level   form                                  binds
  ─────────────   ───────────────────────────────────   ────────────────────────────
   0  ` , `       multi-pattern arm separator            (loosest — whole alternatives)
   1  ` | `       alternation inside a group
   2  sequence    juxtaposition in `[ … ]`
   3  ` : `       capture  name:pat
   4  postfix      `?`  `*`  `+`  `*(sep)`               (tightest — bind to the nearest pat)
      prefix       `..name` (rest, only as a slice tail)
```

Add a **non-CFG note** (like `D-gram-2`, a decided edge): a `(…)` inside a pattern is an
alternation/optional/repetition **group**, distinguished from a tuple pattern by its operators
(`|`, `?`, `*`, `+`) — resolved by the same speculative parse the surface already uses. No CFG is
owed (tooling reuses the hand-written parser).

### 3.3 `formal/types.md` — capture typing (additive; no new former)

Add a **§ Pattern captures** with the synthesis rules from §2.1, each citing the former it reuses:

```
  (P-Cap-Ty)     Γ ⊢ (name:p) binds name : τ         where Γ ⊢ p's result ⇒ τ
  (P-Alt-Same)   both alternatives bind name : τ_a, τ_b   ⟹   name : τ_a ⊔ τ_b   (⊔ = the join;
                 if τ_a ⊔ τ_b undefined ⟹ STATIC ERROR "alternatives bind name at incompatible types")
  (P-Alt-Diff)   name bound in only some alternatives    ⟹   name : τ?           (via N-Join)
  (P-Opt-Ty)     captures inside (a)?                    ⟹   each promoted to τ?  (via N-Opt)
  (P-Rep-Ty)     the capture inside (a)* / (a)+          ⟹   vector<τ>            (via N-Dense)
  (P-Rest-Ty)    ..name over a vector<τ> subject        ⟹   name : vector<τ>
```

**Headline for the doc:** PEG captures introduce **no new type constructor** — they are a
new *source* of `τ` / `τ?` / `vector<τ>`, unified by the join already central to the integer /
nullability model. `match` remains a `τ?` **eliminator** (`(N-Match)` unchanged: a `null`/`x`
arm still discharges an optional).

### 3.4 `formal/binding.md` — capture aliasing (view vs fresh)

Add **§ Pattern captures** pinning which captures alias the subject:

```
  (P-Cap-View)   a SINGLE structural capture that names an INTERIOR place of the subject (a
                 struct field, a struct-typed element) is a VIEW (binding.md B-View / heap.md
                 H-View): it aliases WITHOUT `&`, and carries the subject's borrow-dep
                 (Deps::frame1(subject)) so both backends agree on free.
  (P-Cap-Fresh)  a `..rest` sub-slice and a repetition `(a)*` accumulator are FRESH vectors
                 (H-Alloc), INDEPENDENT of the subject (binding.md B-Copy / iteration.md I-Comp).
```

*Rationale (decision D-F2/D-F3, §4):* views for single interior captures match today's match
bindings (no copy, cheap); fresh vectors for `..rest`/repetition match `I-Comp`'s "fresh result
vector" and avoid a sub-slice-aliasing lifetime that neither backend models cleanly.

### 3.5 `formal/iteration.md` (+ `operational.md`) — cursors

- **Vector/slice cursor:** a *reference*, not new rules. Add one line to `iteration.md` §cursor:
  "the same `⟨i, src⟩` cursor backs **pattern matching** (matching.md) — anchor = save `i`,
  revert = restore `i`, both `operational.md` `E-Asgn`; a pattern read past the end is `null`
  per `I-Done`, never a fault."
- **Iterator cursor (L3.6 — the one new primitive):** add a **buffered-cursor** rule + the two
  ops to `operational.md` (or an `iteration.md` §):
  ```
    (P-Anchor)   OpMatchAnchor: push ⟨i, epoch⟩; while any anchor is live, next(it) APPENDS the
                 pulled item to a memo buffer (Lexer::memory model) instead of discarding it.
    (P-Revert)   OpMatchRevert: pop the anchor, rewind i to the anchored position (replaying from
                 the memo), and drop bindings written after epoch.  Buffer clears when the anchor
                 stack empties (links-refcount = 0).
    (P-IterBound) a repetition over an iterator is bounded by a `max_lookahead` arm attribute;
                 exceeding it is a defined runtime error (NOT a hang) — preserves termination.
    Limitation (documented, CAVEATS.md): a SIDE-EFFECTING pull (a generator that mutates external
                 state per item) cannot be reverted — matching over such a source is UB-by-contract,
                 the same assumption Lexer makes about its token stream.
  ```
  These are the only PLN35 additions to the small-step core; they are the two opcodes whose
  add-path IMPLEMENTATION.md Phase 7 details.

### 3.6 `formal/VERIFICATION.md` + `formal/ROADMAP.md`

- **VERIFICATION.md:** add a PLN35 block — one row per new rule (`P-Seq`, `P-Alt`, `P-Opt`,
  `P-Rep`, `P-Cap`, `P-Rest`, `P-Multi`, `P-Atomic`, `M-Total`, the `P-*-Ty` typing rules, the
  two iterator ops): the single falsifiable claim + the both-backends oracle program that pins it
  (`tests/oracle/35-*.loft`). This is the spec-first analogue of a deviation list.
- **ROADMAP.md:** one line noting PLN35 is spec-first (rules written ahead of code) — it creates
  **no deviations** (nothing yet breaks a rule); the obligations live in VERIFICATION.md until
  each phase lands.

### 3.7 `formal/capabilities.md` — one note

Pattern matching is **capability-neutral**: it introduces no new host surface, no I/O, no
ambient authority. A match over an iterator inherits the iterator's own admission (the pull is
the guarded operation, not the match). Add a one-line note so the area stays complete.

---

## 4. Formal decisions (each with its falsifier)

| id | decision | rationale | falsifier (obey-rule vs naive disagree) |
|---|---|---|---|
| **D-F1** | **Whole-consume default; `..rest` for prefix.** An arm matches iff its pattern consumes the *entire* subject (`P-Whole`); a prefix match is not a selection. | matches today's exact-length `[a,b,c]`; makes length part of the pattern, not a silent truncation. | `match [1,2,3,4] { [a,b,c] => X, _ => Y }` must pick `Y` (naive prefix-match picks `X`). |
| **D-F2** | **`..rest` + repetition captures are FRESH vectors** (H-Alloc), not sub-slice views. | consistent with `I-Comp`'s fresh result + `B-Copy`; avoids an interior-slice lifetime neither backend models. | mutate a captured `rest`; the subject must be UNCHANGED on both backends (naive view aliases). |
| **D-F3** | **A single interior capture is a VIEW** (`B-View`), as today. | cheap, matches current match bindings + `H-View`. | `V { inner }` then `inner.x = 9` ⇒ subject's `inner.x == 9` (the existing view contract). |
| **D-F4** | **Ordered choice, first-match, greedy repetition** (PEG, not regex/longest-match). | unambiguous ⟹ INV-Det tractable; mirrors the parser loft already ships. | `(a | ab)` on `"ab"`: PEG takes `a` then fails the rest; longest-match takes `ab`. Both backends must take `a`. |
| **D-F5** | **Struct-variant patterns only — tuple variants are a PERMANENT non-goal ([@C89](../../DESIGN_DECISIONS.md)).** | loft enum payloads are always named fields; positional `Num(i64)` / `Ok(a)` force match-to-read + a mitigation-syntax sprawl. | `Num(i64)` is a parse error, always; every example uses `Num { v: … }`. |
| **D-F6** | **Patterns read like grammar notation, not regex; readability > parser simplicity ([@C89](../../DESIGN_DECISIONS.md)).** | The PEG surface is the standardized readable parser notation (sequence, `\|`, `?`, `*`, `+`, grouping, named captures) a reader follows without training — regex/text is not, so it stays a library. loft pays extra parser logic for the readable surface. | falsifier: a spelling that needs training to read (regex-class density, a cryptic separator like `*(sep)`) fails the bar → re-spell it. |

---

## 5. Phase → formal-rule map (what each IMPLEMENTATION phase discharges + pins)

| IMPL phase | formal rules discharged | oracle program to add |
|---|---|---|
| **P0** design | `M-Total`, INV-Pure/INV-Det/INV-Total probes; docs 3.1–3.7 written | `35-invariant-*.loft` (the three falsifiers) |
| **P1** L2 nested | `P-Var`/`P-Bind` recursion, `P-Cap-Ty`, `P-Cap-View` | `35-nested-enum-match.loft` |
| **P2** L3.1 seq + `..rest` | `P-Seq`, `P-Whole`, `P-Rest`, `P-Rest-Ty`, `P-Cap-Fresh`, `M-Total` gate on slices | `35-sequence-rest.loft` |
| **P3** L3.7 multi-arm | `P-Multi`, `M-Total` union | `35-multi-pattern-arm.loft` |
| **P4** L3.2 alternation | `P-Alt`, `P-Alt-Same` (⊔), `P-Alt-Diff` (N-Join), `P-Atomic` (slice) | `35-alternation.loft` |
| **P5** L3.3 optional | `P-Opt`, `P-Opt-Ty` | `35-optional.loft` |
| **P6** L3.4 repetition | `P-Rep`, `P-Rep-Ty`, separator | `35-repetition.loft` |
| **P7** L3.6 iterator | `P-Anchor`, `P-Revert`, `P-IterBound`; the two opcodes | `35-iterator-match.loft` |

Each phase closes by graduating its oracle program (both backends agree) and ticking its
VERIFICATION.md rows. INV-Total is asserted from P0 and re-checked every phase.

---

## 6. Open formal questions

1. **Split matching.md?** If the pattern relation pushes matching.md past ~250 lines, extract
   `formal/patterns.md` (the `⇓` relation) and leave matching.md the `match`-expression + M-Total
   hub. Decide at write time (§3.1).
2. **`total()` precision.** Is a length-exact slice of all-total elements ever itself total (a
   fixed 0-length `[]`)? Edge; default is non-total (require `_`). Revisit only if a real
   consumer wants a total fixed-shape arm.
3. **Longest-partial error position** (README open-Q #3): the furthest `i` reached across reverts
   is a natural error anchor. Formalize as an *observability* note (like `E-Report`), not a
   value-affecting rule — deferred to P4.
4. **Commit points** (`~`/`!~`, README open-Q #1): not in the relation; add only if deep-revert
   errors prove unhelpful. Would be a new `P-Commit` rule.

---

## 7. See also

- [IMPLEMENTATION.md](IMPLEMENTATION.md) — the build plan these rules gate (phase↔rule map §5).
- [README.md](README.md) — informal design draft.
- `formal/matching.md` / `types.md` / `iteration.md` / `binding.md` / `operational.md` — the
  docs edited.
- `formal/README.md` — the spec-first / deviation discipline; `VERIFICATION.md` — the pinning worklist.
- design-protocol skill — the invariant-first, falsifier-per-claim lens this doc applies.
