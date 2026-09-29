# formal/matching-history.md — the deviation register for [matching.md](matching.md)

> **The rules are next door.**  [matching.md](matching.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The rules doc carries the CURRENT state (how many are open,
> and which); everything below is the record behind it.

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
