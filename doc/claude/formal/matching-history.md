<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# formal/matching-history.md — the deviation register for [matching.md](matching.md)

> **The rules are next door.**  [matching.md](matching.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The rules doc carries the CURRENT state (how many are open,
> and which); everything below is the record behind it.

## Deviations carried by matching.md until 2026-09-30

The register's status line, as it read: OPEN: 0 — the 2026-09-29 rule-led walk of the pattern rules found `D-match-7` to `-13`,
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

The PEG extension landed in 350e660c (#554), 3fda4e1e (#558), 50cc4c18 (#561) and a37917ff
(#562).  Its banner said SHIPPED over all fourteen rules until 2026-09-25, when `D-match-6`
named the three it did not cover.  The conformance bullet read *"opens no deviation"* for as
long as `(P-Rest)`'s `t` was refused outright: no oracle case had spelled a rest with anything
after it.  The arm rule's "bare binding" wording stood for two months without the code ever
having it — corrected 2026-09-07, with the silence that hid it (an arm naming nothing was
skipped without a diagnostic).

- **D-match-18 — CLOSED 2026-10-06 (@PLN186 step 2).** `(P-Multi)` on a plain-struct subject,
  `P { x: 0, y: v } | P { x: v, y: 0 } => v`, was refused (*"Unknown variable 'v'"*).  The
  refusal was not about `|`: a struct arm read a field's bare-name pattern (`y: v`) as a VALUE,
  against `(P-Point)` — refused when no `v` was in scope, and a silent comparison with an outer `v`
  when one was (loft#1885).  Closed by both halves: the struct arm binds a bare name through the
  variant arm's `field_pattern_rename`, and its further patterns are parsed by the tuple arm's
  `parse_pattern_alternatives` — each its own arm with its own bindings (nothing hoisted when an
  arm lists alternatives) and literal-field test.  Alongside, an arm after a total struct pattern
  is refused by name (`@FR-M-Wild`), where the loop's `break` left it to "Expect token }".
  Guards: `tests/scripts/a-struct-or-pattern-arm-links-its-bindings.loft`,
  `a-struct-pattern-arm-refusals.loft`.

- **D-match-20 — CLOSED 2026-10-06 (@PLN186 step 4).** `(P-Alt)` inside a tuple element,
  `(Circle { r } | Square { r }, k) => r + k`, was refused (*"expected ',' between tuple pattern
  elements"*).  A tuple element that lists alternatives (a `|` before the `,` or `)` that ends it)
  now takes the slice element's alternation without its parentheses: a tag disjunction, each name
  read from whichever variant matched, `τ?` where only some bind it, and the names joined to the
  arm's pending set so its end restores them.  Guards:
  `tests/scripts/a-tuple-element-lists-alternatives.loft` and
  `…-element-alternative-binds-a-name-at-another-type.loft`.

- **D-match-19 — CLOSED 2026-10-06 (@PLN186 step 3).** `(P-Multi)` over a tuple subject,
  `(Circle { r }, Square { s }) | (Square { s }, Circle { r }) => r * 10.0 + s`, was refused
  (*"Expect token =>"*).  Each further tuple pattern now parses into its own bindings and
  conditions, a shared name is copied into the first pattern's slot (the enum arm's linking), and
  each alternative runs the body taken BEFORE the first pattern's bindings fold into it — the
  first build cloned the folded body, so the second alternative re-ran the first's bindings and
  read `r` and `s` from the swapped positions, hidden by a symmetric `r * s`.  Guards:
  `tests/scripts/a-tuple-or-pattern-arm-links-its-bindings.loft` (order-sensitive bodies) and
  `…-alternative-binds-a-name-at-another-type.loft` (`@FR-P-Alt-Same`).

- **D-match-17 — OPENED AND CLOSED 2026-10-05.** `(P-Point)`: a field sub-pattern over a
  `reference<T>` field, and a plain-struct sub-pattern on any struct field (loft#1870).  The
  variant test asked the inline-enum type alone, so `Add { l: Lit { v: 0 }, r }` over
  `l: reference<Expr>` fell to the scalar path, which BUILT a `Lit` and tested that record:
  the arm never matched on `--interpret` (silently — `0 + x` was never simplified),
  `--native` refused its own output (`E0605`, `DbRef as u8`), and the binding form
  (`l: Lit { v }`) did not parse.  `H { q: P { v: 0 } }` failed the same way on an INLINE
  struct field, though a top-level `P { v: 0 }` over a `P` subject always worked.
  `pattern_variant_enum` now also answers a reference to a struct-enum, and
  `parse_field_sub_pattern` runs a plain struct's own field loop (`parse_struct_sub_pattern`,
  `parse_match_struct_arm`'s).  Seven cells on both backends under strict stores; guard
  `a-field-sub-pattern-asks-the-record-a-reference-field-points-at.loft`.
- **D-match-16 — OPENED AND CLOSED 2026-10-02.** `(M-Match)`: a struct-enum or struct subject
  that was not a variable was spliced into every arm test and every field binding, so a call
  ran once for the variant and again for each field read.  `match next(lx) { Num { v } => … }`
  tested the first token's variant and read the second token's payload, on both backends and
  on main, with nothing reported; a lookup in a `match` subject (`match pa_get(m, key)`) ran
  its lookup and its copy twice.  The subject is now bound once unless it is a place, which
  reads the same value each time (`tests/scripts/a-match-evaluates-its-subject-once.loft`).
  The binding had been held back for a free-order constraint `(H-FreeAny)` had already lifted.
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

## Deviations carried by matching.md until 2026-09-29

Closed entries moved here from the rules chapter's register (RELEASE.md § 5b), as written.

- **D-match-6 — OPENED AND CLOSED 2026-09-25 (loft#1678).** `(P-IterBound)` promised a bound and
  a defined error; there was neither.  A `match` over an iterator MATERIALISES its subject before
  the patterns run (`collect_iterator_subject`; `tests/scripts/35p-iterator-match.loft` states the
  design), and the pull had no counter, so an endless source filled memory on both backends —
  2 GB in 18 s on `--native` — until something outside the program killed it.  Closed in the
  pull: it counts what it appends and, past `max_lookahead`, stops with `panic`, the language's
  defined error, naming the `match`'s line and the bound.  `max_lookahead` is one million by
  default, `LOFT_MAX_LOOKAHEAD` overrides it (`0` = no bound), read at compile time so both
  backends bake the same constant.  The two decisions the issue named (what the error is, what
  the bound is) were taken on the rule's own words — "a DEFINED runtime error" — and stand to be
  overruled.  The endless source must yield something a native generator runs LAZILY: an
  endless loop yielding RECORDS runs eagerly on `--native` and never reaches the `match`
  (`coroutines.md` Conformance, COROUTINE.md § CL-9), which is `(G-Next)`'s gap, not this bound.
  Guard: `tests/exit_codes.rs` `a_match_over_an_iterator_stops_at_max_lookahead` (the stop, the
  bound and `0`, both backends, on the process); falsified by hand against `ee5faae15` — the
  endless cell aborts on allocation interpreted (exit 134) and is killed by the timeout on
  `--native` (exit 143), where this tree stops with exit 1 and the message.

  Not settled by that closure, and carried from the entry as it was opened: `(P-Anchor)` and
  `(P-Revert)` name a memoising cursor — `OpMatchAnchor`, `OpMatchRevert` — that does not exist
  in `src/`; the shipped design materialises the subject.  Whether the two rules are restated to
  the materialising design or the cursor is built is the owner's call.

- **D-match-1 — OPENED AND CLOSED 2026-09-04 (loft#1343).** `(M-Bool)` did not exist, and the
  edge it names was answered wrong: a boolean match spelling both arms was lowered with the
  scalar match's typed-null fall-through, so its type read nullable and a `-> text` function
  returning it warned `(N-Store)` on both backends.  Closed in the scalar match parser: both
  literal arms, unguarded, make the last arm the fallback.  Guard
  `tests/scripts/1343-a-boolean-match-with-both-arms-is-exhaustive.loft` +
  `tests/boolean_match_exhaustive.rs` (the warning stream, which no corpus channel scores);
  falsified at `dd46146c` — five warnings → none on both backends.

- **D-match-2 — OPENED AND CLOSED 2026-09-07.**  `(M-Unit)` says an arm's pattern names a variant
  of the subject's enum.  An arm naming something else was refused only when the name resolved to
  SOME other definition; a name that resolved NOWHERE — a typo, or a variant renamed since the arm
  was written — was skipped in silence, its body still parsed and type-checked, and the subject
  fell to whatever arm came next.  With a `_` present that is a wrong ANSWER with no diagnostic on
  either backend: `match c { Red => "red", Grean => "GREEN", _ => "other" }` answered `"other"` for
  `Colour::Green`.  The gate was on the PATTERN name resolving; it is now on the SUBJECT enum being
  resolved (`valid_enum && e_nr != u32::MAX`), which is the condition the skip actually exists for
  — a cross-package forward reference read on pass 2 (#375).  Guard
  `tests/scripts/a-match-arm-names-a-variant-that-exists.loft`, three arms, falsified at
  `a192aecbd` (all three compiled and ran there, zero diagnostics).  The same walk corrected
  `(M-Total)`'s enum bullet, which offered "bare binding" as a covering arm form — an element
  pattern the ARM grammar does not have, and the reading that makes the silent skip look
  deliberate.

- **D-match-3 — OPENED AND CLOSED 2026-09-07.**  `(P-Rest)` binds `..name` to `src[i .. len−t]`
  with `t` counting the fixed elements AFTER the rest, and the parser refused every `t > 0`
  spelling: `[a, ..mid, z]` reported *"a named rest `..mid` must be the last slice element"*.  The
  refusal — not the rule — was the defect, because the UN-NAMED gap `[a, .., z]` was already legal
  and already bound `z` from the end: the language could match the shape and not name what was in
  it.  Nor was anything missing underneath.  The arm gate is `head + tail <= len`, the tail is read
  at negative indices, and `hi = len − tail_len` was computed inside the refused branch itself, so
  the diagnostic stood in front of a correct lowering; `materialize_named_rest` already takes
  runtime bounds (the repetition path passes them, native-verified by `35o-tail-elements`).  Closed
  by deleting the diagnostic.  Guard
  `tests/scripts/a-named-rest-leaves-room-for-the-elements-after-it.loft` — head 0/1/2 × tail 1/2,
  the empty middle (`lo == hi`) and the under-length subject that must NOT match, scalar / text /
  struct / struct-enum elements, `(H-Alloc)` independence, the reuse cell, and a guarded arm — each
  asserted beside the un-named spelling on the same subject, so a future drift between the two
  paths fails here.  Falsified at `4e5725a2c` (both backends refuse, exit 0 → 1).

- **D-match-4 — OPENED 2026-09-08, CLOSED 2026-09-12 (loft#1419).**  `(P-Rest)`'s `t` counts
  *fixed patterns*, and `(P-Point)` makes a unit variant, a struct variant, a literal, `_` and
  a bare binding all point patterns.
  Only `_` and a bare binding are accepted after a `..`: the element loop takes `has_identifier()`
  there, so `[Kw { word }, .., End { e }]` binds the name `End` and then chokes on `{`, reporting
  four cascading *"Expect token ,"* messages that name nothing.  Pre-existing and independent of
  the rest's spelling — it reproduces on the un-named gap and on a literal alike.  Closing it means
  reading a tail sub-pattern at a NEGATIVE index, so the tail length must be known before the
  sub-pattern is parsed; the repetition path already does exactly that
  (`35o-tail-elements.loft`), which is the precedent to follow.  Workaround (verified, both
  backends): bind a bare name and destructure it in a nested `match`.

  **CLOSED 2026-09-12** — and it closed the way the entry predicted, by knowing the tail length
  before the sub-pattern is parsed.  Re-measured on both backends: the entry's own
  `[Kw { word }, .., End { e }]` binds `let 9`, the literal tail `[1, .., 9]` matches, and the
  bare-name control still binds — so the cure admitted the two forms `(P-Point)` names without
  disturbing the two that already worked.  Guard
  `tests/scripts/1419-a-fixed-pattern-after-a-rest-is-a-tail-element.loft`, which also holds the
  named-rest spelling, a two-fixed tail and a head control; falsified at `d9850164`.

- **D-match-5 — OPENED AND CLOSED 2026-09-14.**  `(P-Point)` makes a unit variant and a struct
  variant point patterns over ONE value, and a TUPLE element is one value — but the tuple
  element loop offered only `_`, a bare binding, a literal and a nested tuple, so a capitalised
  name in an enum-typed element fell through to the BINDING branch: `(KFire, KWall) => …` bound
  two locals named `KFire` and `KWall` and matched EVERY pair, on a plain enum and on a
  struct-enum alike, on both backends, with no diagnostic.  Nothing else in the language reads a
  capitalised name as a variable (`Foo = 5` is refused as unknown), which is what made the
  silence a wrong ANSWER rather than a spelling the author could have meant.  Found by @PLN162
  step 0, whose acceptance program is a central `match` on a pair of variants; the design's own
  `match (a, b) { (Fireball(f), IceWall(w)) => … }` did not parse, and the nearest loft
  spelling answered wrong.  Closed by routing an enum-typed element through the slice head's
  own path (`peek_is_variant_subpattern` → `parse_field_sub_pattern`), so `(Fire, Wall {
  hp })` tag-tests each position and binds its payload — a heap payload as a VIEW, as at a
  top-level arm; a capitalised name that is no variant is refused by name, the `(M-Unit)`
  refusal one level down, and one over an element with no variants says so.  Re-measured on
  both backends: five pairs of a plain enum, an or-pattern inside an element, a struct-enum
  pair by tag and by payload, a variant beside a literal and beside a guarded scalar, a
  nested-record write landing through the tuple and through the elements of a
  `vector<Entity>`, and the lower-case binding control.  Guards
  `tests/scripts/a-tuple-pattern-names-a-variant.loft` and
  `tests/scripts/a-tuple-pattern-refuses-a-name-that-is-no-variant.loft`, falsified at
  `00272ff49`.  Two spellings stay refused, exactly as at a top-level arm: an or-pattern
  between STRUCT-enum variants, and the qualified `Kind.KFire`, which reports *"'Kind' is not
  a variant of Kind"* in both places — a message to sharpen, not a rule.
