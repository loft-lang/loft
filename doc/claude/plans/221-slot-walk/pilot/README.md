<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 221 — The slot walk: the pilot

The record of the probe [the plan](../README.md) was designed on.  `scripts/slot_matrix.py`
has replaced it; the two scripts stay as the smallest form of the idea.

[`gen.py`](gen.py) writes the cells and [`run.sh`](run.sh) runs
them; counts and hand-written expected values only, no twin, no trace, no leak channel.

| Build | Cells | Failing | Distinct causes |
|---|---|---|---|
| `main` at the release build of 2026-11-18 | 114 | 30 | 4 |
| the branch carrying the fixes for loft#2573 and loft#2571 (`6ec7ddf15`) | 114 | 12 | 2, and 1 question |

- **It found the known defects unaided.**  loft#2573 (24 cells: every loop form over
  `sorted` and `index`, with every filler) and loft#2571 (the place of a collection append
  run twice per item).
- **It found two more that the fixes did not reach**, the same on the interpreter and
  `--native-c`:
  - a comprehension over a `sorted` or `index` source still runs the source once per step
    (`[for p in mk(c) { p.k }]`: five runs for three elements) — loft#2573's fix covered the
    `for` statement and not the comprehension;
  - a collection assigned to a field at a computed place addresses the place three times
    (`h.ws[at(c, 1)].items = [P { … }]`).
- **It raised a question the rules must answer.**  A compound write through a record a
  function returned (`box(c, h).n += 5`, where `box` returns `h.ws[0]`) leaves `h.ws[0]`
  unchanged, with no diagnostic.  Whether that is a write to a copy that should be refused,
  or a lost write, is for `binding.md` to say.
- **It showed two rule sentences that disagree.**  `(E-Left)`'s prose says `??` "evaluates
  both operands"; `(E-Coalesce)` and both backends evaluate the right side only when the
  left is null.

The plan's § Phase B as built carries these forward.

