<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/dispatch-history.md — the deviation register for [dispatch.md](dispatch.md)

> **The rules are next door.**  [dispatch.md](dispatch.md) states what must always be true of
> overload selection; this file is its TIMELINE.  How each rule landed in the tree — the
> implementation form, the decisions taken and their dates — is
> [plans/162-multiple-dispatch/RULES.md](../plans/162-multiple-dispatch/RULES.md), and the rules
> moved from there to dispatch.md on 2026-10-02 (@PLN182 P1 needed one to cite).

## Deviations carried by plans/162-multiple-dispatch/RULES.md until 2026-10-02

OPEN: **0**.

- **D-disp-1 — CLOSED 2026-09-15** (opened 2026-09-14, narrowed the same day by step 13).
  What remained after step 13 was the `self` set over variants, in two facets, and the owner
  decided both in one sentence: *the refusal is the same shape as the match without `_`, and
  the enum-level definition is the `_` variant — every case with no more specific
  implementation.*
  - **The enum-level member is `_`.**  `kind(self: Fireball)` beside `kind(self: Entity)` — or
    beside a free `kind(e: Entity)` — lived under different keys (`t_8Fireball_kind`,
    `t_6Entity_kind`, `n_kind`), so nothing made them one set: no bare dispatcher existed,
    @F20 yielded to the enum-level definition, and a receiver held at the enum answered the
    enum-level body whatever its variant — on both backends, with no diagnostic, even with
    every variant implemented.  `Parser::join_enum_lattice_sets` now makes every definition
    of one name whose first parameter is an enum or one of its variants ONE overload set
    (once the file's definitions are known, before @F20 asks whether a set owns its
    dispatch), so `Disp-Specific` ranks the variant above the enum and step 13's dispatcher
    builds the `match`.  A group with no enum-level member stays @F20's, whose dispatcher
    already is that `match`.
  - **A missing variant is `match` without `_`.**  With `tag(self: Fireball)` and
    `tag(self: IceWall)` declared and `Crate` left out, a call through `Entity` was a
    WARNING at `Crate`'s declaration and an EMPTY value at run time.  It is now refused at
    the call, in `M-Exhaust`'s shape (*call of `tag` on `Entity` is not exhaustive —
    missing: Crate; add … or a fallback `fn tag(self: Entity)`*), by
    `Parser::refuse_uncovered_variants` at `call_nr`, where both spellings and every
    receiver spelling arrive.  A method only some variants have, called only on those
    variants, dispatches nothing and says nothing — the warning fired for it too.
  The warning was the leniency that let a half-built set run; at contract 0 turning it into
  the refusal is the owner's call, taken.  Measured over 1 554 corpus files and 3 331 more
  (the repo's fixture libraries, tools and docs, and nineteen sibling projects): the only
  programs refused are the two that pinned the old behaviour; `813-variant-bounded-generic`
  forms a set and answers unchanged.  Guards: `tests/scripts/a-method-at-the-enum-is-the-wildcard-for-variants-without-their-own.loft`,
  the refusals in `tests/parse_errors.rs`, and `tests/oracle/35-dispatch-self-set.loft` held
  to its `match` twin.  A `self: Fireball?` member beside `self: Entity` is `Disp-Ambiguous`
  for a `Fireball` held at the enum, exactly as the free pair is (nullability and the enum
  are incomparable abstractions).

- **D-disp-2 — CLOSED 2026-09-15** (opened the same day, found closing D-disp-1).  A
  NULLABLE enum position stayed static (step 13: *null has no variant*), so an `Entity?`
  holding a `Fireball` reached the definition its STATIC type selects — behind the `(N-Store)`
  warning where that definition takes a dense `Entity`, and in SILENCE where it takes
  `Entity?` — while the canonical `match` over the same value takes the `Fireball` arm
  (measured on both spellings, a trailing parameter, a loop over call results, and two
  positions: `hit(Entity?, Entity)` answered the fallback for a `(Fireball, IceWall)` pair).
  A nullable position is now dynamic whenever the call HAS a static selection: a present
  value's variant decides as a dense one's does, and a null — which has no variant — reaches
  that static selection, exactly what it reached before.  The dispatcher declares each
  nullable position at the nullability the static selection declares there, so the call into
  it is checked as the direct call was (the same `(N-Store)` warnings at the same sites with
  the same text, measured), and a `Disp-World` rebuild recovers what the call routed from the
  specialisation's spelling.  A call with no static selection keeps its nullable positions
  static and is answered as before.  Guards:
  `tests/scripts/a-nullable-enum-argument-is-dispatched-on-its-variant.loft`, the nullable
  cells of the two step-13 guards (which pinned the static answer, now flipped), and
  `tests/live_world.rs` (`a_nullable_dynamic_site_keeps_its_null_leaf_across_a_rebuild`,
  which fails with the spelling step removed).

