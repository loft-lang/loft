<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN179 — the scoreboard: every port against its original, three verdicts each

One row per port, measured by `make script-twin` ([README § The bar](README.md)): the four
behaviour channels, the best wall clock of each side, lines with and without comments, and
the reasons the port met ([REASONS.md](REASONS.md)).  A port is listed the day its twin first
runs green; a row is re-measured when either side changes.  Strand 6 renders this as a page
beside `00-vs-python.html` once the pairs live under `tests/comparisons/scripts/`.

| original → port | behaviour | performance (orig → port, best of 2) | lines total / non-comment | clarity | reasons |
|---|---|---|---|---|---|
| `fuzz/seed_program_source.sh` → `fuzz/seed_program_source` (3883 files copied) | same on all four channels, on both loft backends | 17.1 s → 1.15 s interpreted, 1.5 s native | 21 / 10 → 34 / 24 | **longer**: a hand-written walk and a two-call copy | 4, 5 |

How to read a row: **behaviour** is the twin's verdict (stdout, stderr, exit status and the
written files, by content where the original's numbering follows an unordered `find`);
**performance** compares the original as it runs with the port in the mode its `#!` line
names; **clarity** is loft ≤ original in non-comment lines, and a longer port names why in
the reasons column rather than in prose.
