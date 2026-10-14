<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Fix-line wording — prototype (copy notice)

The load-bearing part of @PLN131 is not the rewriting machinery, it is whether **one line can
carry a condition precisely enough for a veteran to affirm it in a second**. This prototypes
that line against the real cases @PLN130 F5 measured, and reports what is missing.

## The structure: the condition is a FIELD, not prose

The first attempt put the condition inside the sentence:

```
fix: if `src` is not needed afterwards, drop that use — this becomes a move
```

It reads fine and is wrong for the job. A click has to affirm the condition, so the condition
must be a thing the UI can *show as the thing being affirmed* — not a clause the reader has to
extract. Hoisting it out:

```
fix  build it in place                       h = Holder { v: [1, 2, 3] }     [move]
fix  drop the later use of `src`             needs: `src` unused after here  [move]
```

Structured, this is:

```
{ kind: "mechanical" | "conditional",
  title:     "drop the later use of `src`",
  condition: "`src` is not used after this point",   // conditional only
  edit:      <rewrite>,
  concept:   "move", concept_ref: <catalogue id> }
```

That answers Q2 as a side effect: an LSP code action renders `title` on the lightbulb and
`condition` in the confirm step, and the CLI prints them on one line. One shape, two surfaces.
It also makes the two hard rules checkable — a conditional suggestion with an empty
`condition` is malformed and can be rejected in a test.

## Rendered against the measured cases

**Case 1 — construct from a surviving local** (@PLN130 F5's `av1`; both fixes exist):

```
advice: copy of vector<integer> — `src` is used again after this point
  fix  build it in place                    h = Holder { v: [1, 2, 3] }        [move]
  fix  drop the later use of `src`          needs: `src` unused after here     [move]
```

**Case 2 — append from a surviving local** (`ap1`: `c1.data += c2.data`):

```
advice: copy of Bag — `c2` is used again after this point
  fix  drop the later use of `c2`           needs: `c2` unused after here      [move]
```

Note what changed: **only one fix**. You cannot "build it in place" for an append — the
mechanical rewrite does not exist for this shape. So the fix set is **shape-dependent**, and
a suggestions feature that assumes every diagnostic has a mechanical option is wrong. This is
the second-commonest avoidable shape in the corpus, so it is not an edge case.

**Case 3 — append from a parameter** (corpus `148 seg_mesh_append`, `dst.kinds += src.kinds`):

```
advice: copy of SegMesh — `src` is used again after this point
  fix  drop the later use of `src`          needs: `src` unused after here     [move]
```

Here the "later use" is the next two lines of the same function (`dst.values += src.values`).
The condition is *false* — the author obviously needs them — so the honest outcome is that
this site has **no offerable fix**, and the notice should stand alone. Offering a fix whose
condition is visibly false trains people to stop reading conditions, which is precisely how
the click becomes unsafe.

## Two things must exist before this line is any good

1. **The condition cannot name the use.** It says *"after here"* because the analysis has
   `last_use_pos` as a traversal index, not a source span — so it cannot say *"`src` is used
   again at line 12"*. That difference decides whether a veteran affirms in one second or goes
   hunting. Carrying the surviving use's LOCATION through `VerdictRow` is a small, specific
   piece of work and it is a prerequisite, not a polish item.

2. **The concept has no door.** `[move]` should link to the capability, but the feature
   catalogue has 105 entries and **none** covering move/copy/ownership semantics, and
   `LOFT.md` has no copy-vs-move section (one passing `@C86` mention). The plan's own rule says
   a door onto nothing is worse than no door — so either a catalogue entry is written first,
   or the handle ships without a link and the teaching half of @PLN131 does not happen yet.

## What the prototype settles

- The condition belongs in its own field; that is what makes one-click affirmation honest,
  and it gives the LSP and CLI one shared shape.
- Not every diagnostic has a mechanical fix — the append shape has only a conditional one.
- A site whose condition is visibly false should offer **nothing**. Suppressing a bad
  suggestion matters more than emitting a good one, because credibility is what makes the
  click safe.
- The wording is cheap. The two prerequisites above are the actual work.
