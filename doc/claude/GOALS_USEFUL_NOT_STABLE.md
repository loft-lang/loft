<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# GOALS — useful is not stable

Split from [GOALS.md](GOALS.md) by subject: why dogfood cannot measure stability, and how to rank the unwalked cells.

---

## Useful is not stable — the gap dogfood cannot measure

Goal C's fraction says the language is **useful**: real programs build and run, and the
consumers keep finding it worth using. That is a real measurement, and it is not a stability
measurement — because a consumer exercises the paths *its author happened to walk*.

Those paths get hard through use. A shape that moros or dryopea leans on daily has been hit
from every angle a working program hits it from, and its bugs were found years ago by the
cheapest possible instrument: somebody running the program. **Heavy use is a test suite nobody
had to write.**

Everything else is an outlier — and an outlier is not a rare shape, it is an *unwalked* one.
The same feature at a different type, in a different position, under a `?`, on the other
backend, in a `return` instead of a local. Nothing about it is exotic; it is simply that no
program we have has needed it yet. **A new project or a new user hits those on their first
day**, because their program is not our program, and what is peripheral to us is central to
them. The measured form of this: `hash<S[k]>? = [S { … }]` is refused today, and the reason is
not that anyone decided it should be — it is that no consumer has ever written a nullable keyed
literal.

**So the gap this work bridges is between *"works on the paths we walk"* and *"works on the
paths someone else will walk."*** Every instrument aimed at the corpus measures the first. The
bug windows say so directly: they are dominated by `hit-by:loft`, and BUG_REVIEW.md's own
reading is that *"the sample is entirely self-generated, so it measures our reach and not the
language"*. A corpus can only ever cover what somebody already wrote.

**A rule can cover what nobody has written yet**, and that is why the rules are the instrument
rather than a nicety. `formal/`'s rules are stated over a whole domain — *every* `τ`, *every*
collection kind, *every* position — so walking one asks about cells no program in the tree
reaches. That is the concrete link from this section to the method:
[STABILITY_METHOD.md § The rule-led walk](STABILITY_METHOD.md).

Two consequences worth stating outright, because both look like poor use of time under a
capability lens and are the point under a stability one:

- **A cell that works is a result.** Proving that a rule holds on nine types across three
  routes converts thirty unwalked paths into walked ones. Nothing shipped, and the language
  got more stable, because the next person to walk them is no longer the first.
- **An unwalked path's bug is worth more than its frequency suggests** — but which unwalked
  path matters is decided by DEPTH, not by rarity, and the next paragraph is the whole ranking.

### Not every unwalked cell is worth the same — rank by how contrived it is

The full matrix contains a great many cells that only exist because a programmer *shoehorned*
something in: four features composed in a way nobody would arrive at without deliberately
pushing at the edges of the language. **A problem in one of those is survivable** — the person
who built that cell knows they are at the boundary and has the context to back out — so it does
not set the queue.

⚠ **Depth ranks URGENCY; it never licenses a refusal.**  What decides whether a shape must work
is [`formal/`](formal/README.md), and its doctrine is *the rules do not change to match the
code; the code changes to match the rules*.  **If the definition gives a clear picture of what
to implement, implement it — right away, whatever the cell's depth.**  A contrived shape the
rules cover is a shape that must work; refusing it would be a deviation, not a decision, and
[CLAUDE.md](../../CLAUDE.md) § Debugging policy already says to read the spec *before shipping a
refusal* for exactly this reason.  Nor is silence from the rules a licence: an edge the rules
cannot EXPRESS means the rule wants extending, and that is the next question rather than the end
of one.

A refusal is right only where the rules do not settle it **and** there is a positive reason to
bound the language — the shape works only through fragile machinery no programmer need pays for.
That is § Stability trumps features, and it is a deliberate design decision recorded in
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md), never a default that deep cells fall into.

The cell that matters is the opposite one: **a casual user tries something obvious and hits a
wall immediately.** They are not exploring the boundary, they do not know there is one, and the
wall is the first thing the language ever told them. That is the failure this work exists to
prevent, and it is worth more than a deep bug in every dimension that counts — it costs a user
rather than an afternoon.

**This ranking is already operational — it is what the `wa:` labels measure**, and recognising
that is better than inventing a second axis beside them. A contrived cell has an obvious
workaround *by construction*: the programmer composed four features to get there, so **doing the
simple thing instead is always available to them**. That is `wa:clean`. A casual user who hits a
wall on an obvious shape has nowhere simpler to go — they already wrote the simple thing — which
is `wa:none`, *"blocks whoever hits it"*.

So the ranking reads off the tracker rather than off a judgement:

- **`wa:none` — the case this work exists for.** Nobody reaches it by pushing at the edges;
  they reach it by writing the obvious thing. Top of the queue regardless of `sev:`.
- **`wa:partial` — awkward or lossy escape.** The user gets out, but pays, and had to learn
  the language has an edge here.
- **`wa:clean` — usually the contrived end**, and usually survivable, so it waits.  It still
  gets fixed when the rules say the shape must work; it is later in the queue, not exempt.

⚠ **`wa:clean` does not by itself prove a cell is contrived**, and the implication only runs one
way: contrived ⇒ a workaround exists, not the reverse. loft#1122 was as shallow as a cell gets —
a bare variant in a tuple member — and carried a `wa:clean`; the workaround only existed because
a fix had landed hours earlier, and before that the same cell was `wa:none`. So read `wa:none` as
decisive and `wa:clean` as weak evidence to be checked against the shape.

**This is why a VERIFIED workaround in every issue is load-bearing rather than a courtesy.** It
is the datum that ranks the issue — the filing policy already requires one
([CLAUDE.md](../../CLAUDE.md) § Bug-filing), and the reason is this section. An issue filed
without it is not merely less helpful; it is unranked.

When a rule walk enumerates the related cases (step 4 of
[the rule-led walk](STABILITY_METHOD.md)), the ordering follows the same rule: the cells to
build first are the ones a programmer reaches without knowing the language has edges. A shallow
cell that fails is the finding; a deep cell that fails is a note.

This is the same doctrine as § Stability trumps features, read from the other end: that section
says do not add a shape you cannot afford to keep working; this one says the shapes you already
promised must work everywhere they are promised, not only where they are used.
---
