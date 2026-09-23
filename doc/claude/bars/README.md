<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Bars — what other languages express, measured against loft

A **bar** is a ruler. It takes one language, asks one question of it, and turns the answer
into small loft programs that either run green on both backends or are refused with a named
diagnostic. It is **never a plan and never a gate**: nothing in a bar is a commitment to
implement, and no bar can fail `make ci`.

They live in a directory because there will be more of them, and because a bar is only worth
its length if someone can tell, a year later, what it *produced*. That is what this file is:
the index, the disposition vocabulary, and the record of what each bar has been worth.

## The bars

| bar | the question it asks | measured | entries |
|---|---|---|---|
| [OCAML_BAR.md](OCAML_BAR.md) | what can the type system SAY? generics, closures, patterns, dispatch, nullability | 2026-09-21 | 28 |
| [LUA_BAR.md](LUA_BAR.md) | what does Lua's ABSENCE of declarations buy a game scripter, and is loft's typed answer as short at the use site? | 2026-09-21 | 22 |

Julia has been engaged with, but not as a bar — see § Candidates.

## Disposition — the one word each entry carries

The bars grew three inconsistent ways of saying what becomes of a gap: a heading suffix
(`(regression floor)`, `(design-negotiable)`, `(WONTFIX, pinned)`), a table verdict
(PASS / FAIL / PARTIAL / BLOCKED) and prose. The verdict says what the compiler DID; the
disposition says what the project INTENDS, and they are different questions — a FAIL can be
a won't-do and a PASS can be a floor worth gating.

One word, in the `disposition` column of each bar's Evaluation table:

| word | means | `where` points at |
|---|---|---|
| `floor` | loft does this; the row exists so it cannot regress | the `@F` entry or the test that gates it |
| `shipped` | was a gap, now closed | the issue or plan that closed it |
| `open` | a defect the measurement surfaced, still open | `loft#N` |
| `planned` | a real gap, carried by a named plan | `@PLN N` |
| `low` | a real gap, nobody has scheduled it, nobody has asked for it | — |
| `won't` | declined | the `DESIGN_DECISIONS.md` entry carrying the rationale |

**A `won't` never carries its own rationale.** `DESIGN_DECISIONS.md` is the declined-features
register and is the one home for that reasoning; a bar links to the C-entry and says nothing
more. OCAML_BAR's own § *The document's own claims, corrected* reached this conclusion before
this file existed — *"WONTFIX belongs in DESIGN_DECISIONS.md"* — and it is written down here
so the next bar starts from it.

**`low` is deliberately reason-free.** A gap nobody has asked for needs no essay; writing one
invents a justification for a decision that was never taken. If a reason exists, the row is a
`won't` and the reason belongs in the register. The honest distinction is *"declined"* versus
*"not yet wanted"*, and conflating them is how a backlog turns into an apology.

## What the bars have been worth

Both were measured on 2026-09-21. Between them they surfaced **ten defects, and all ten are
closed**:

| from | issue | what it was |
|---|---|---|
| OCAML_BAR F2 | [loft#1577](https://github.com/loft-lang/loft/issues/1577) | per-value dispatch definitions on a plain enum never selected — `silent-wrong` |
| OCAML_BAR B4 | [loft#1578](https://github.com/loft-lang/loft/issues/1578) | a self-referencing lambda was an internal compiler error |
| OCAML_BAR A5b | [loft#1579](https://github.com/loft-lang/loft/issues/1579) | the cycle refusal's own cure did not work — `reference<E>` over an enum |
| correcting the docs | [loft#1580](https://github.com/loft-lang/loft/issues/1580) | `==` on a `value struct` compared identity — `silent-wrong` |
| correcting the docs | [loft#1581](https://github.com/loft-lang/loft/issues/1581) | a concrete `!=` ignored a user `OpEq`, so `a == b` and `a != b` were both true — `silent-wrong` |
| LUA_BAR (harness) | [loft#1572](https://github.com/loft-lang/loft/issues/1572) | a `sorted` over a record-backed element stacked a duplicate key |
| LUA_BAR LB2 | [loft#1583](https://github.com/loft-lang/loft/issues/1583) | a lazily compiled loop generator with a boolean local did not build on `--native` |
| LUA_BAR LD3 | [loft#1584](https://github.com/loft-lang/loft/issues/1584) | `x = <nullable record> ?? return` did not build on `--native` |
| LUA_BAR LB1 | [loft#1585](https://github.com/loft-lang/loft/issues/1585) | a generator could not be held in a vector or a struct field |
| LUA_BAR LB3/LB4 | [loft#1586](https://github.com/loft-lang/loft/issues/1586) | an endless `while` generator ran eagerly on `--native` until the memory cap killed it |

Three of those are `silent-wrong` — the freeze axis — and two were found not by a probe but
by correcting the reference pages the probes were written from. That second route is worth
naming: a bar makes someone read the documentation adversarially, and that reading found
defects the probes did not.

**A bar also produces features, not only fixes.** The Julia comparison (§ Candidates) is the
clearest case: it produced `@F122`, multiple dispatch, shipped.

## Staleness — a measured bar is a snapshot

A bar's Evaluation table records what the compiler did **on one day**. Every defect above has
since been fixed, so both tables now UNDER-report loft. Each bar therefore states its
measurement date, and each row that is known to have moved says so.

Spot-checked 2026-09-23 against `tuxedo-pr1632-red`, interpreter:

| row | table says | today |
|---|---|---|
| OCAML_BAR A5b | FAIL — `reference<Shape>` field refuses `&e` | **builds**, and with it A5, the recursive forms of C1 and C6, and tier E are no longer blocked |
| OCAML_BAR B4 | internal compiler error | a named refusal naming the cure (*"for recursion declare a file-scope `fn go(…)`"*) |
| OCAML_BAR F2 | silently answers the fallback for all nine combinations | a named refusal naming the cure (*"'Rock' is a value of the plain enum 'Hand', not a type … take a 'Hand' and 'match' on its value"*) |

Four rows checked, three moved. **The re-measure is owed and is not done here** — this file
records the obligation rather than pretending the tables are current, because a register that
silently inherits stale verdicts is worse than no register.

## Adding another

A bar is one language, one question, entries that are runnable loft with an expectation line,
measured on both backends inside a memory cap before any prose is written. A probe that cannot
parse is a FAIL only when **no current spelling expresses the capability** — four of
OCAML_BAR's original FAILs were the probe's own spelling.

But **the rationale does not go in the new file.** It goes in the subject it belongs to
([SUBJECTS.md](../SUBJECTS.md)), and the bar links there. A bar is evidence about one language;
the position is the project's and is stated once. Adding a fourth lens must not add a fourth
rationale — which is why there is no Julia file here even though the Julia comparison produced
more than either of these two.

**Julia.** Engaged with as a design comparison rather than a bar:
[plans/162-multiple-dispatch/](../plans/162-multiple-dispatch/) weighed multiple dispatch,
generated functions, broadcasting and world age against loft's commitments, under the rule
*adopt the constructs that exploit compile-time type knowledge; refuse the ones that demand
runtime type flexibility*. @PLN162 is FINISHED with deviations at OPEN: 0, and multiple
dispatch ships as `@F122`. The one axis a Julia *bar* would add is array and numerical
expressiveness, and [BROADENING.md](../BROADENING.md) § Domain fit already judges that domain
bindable rather than a gap — so it is a `low`, and the reason is recorded there rather than
argued here.
