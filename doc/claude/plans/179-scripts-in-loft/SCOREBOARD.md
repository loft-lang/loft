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
| `…/35-match-peg/rest-store-lifetime/gen_probes.py` → `gen_probes` (33 probe files + a manifest, committed output; the twin's `--tracked` mode) | same on all four channels, on both loft backends; a one-byte wrong port goes red on `files` | 34 ms → 24 ms interpreted, 16 ms native | 171 / 125 → 155 / 127 | **equal**: a struct where Python had a tuple loop, backtick literals where it had triple quotes, the same doubled braces in both | — |
| `scripts/wasm_bundle_stamp.sh` → `scripts/wasm_bundle_stamp` (one SHA-256 over 8 named files + `default/*.loft`, through the `crypto` library) | same on all three channels, on both loft backends | 12 ms → 25 ms interpreted, 47 ms native — **slower**: a `cat | sha256sum` pipeline is under loft's own start-up | 27 / 8 → 24 / 16 | **longer**: a glob is a `list_dir` + filter, and `set -e` is an `assert` per file | 1, 6, 8, 9 |
| `tests/dump_ignored_tests.py` → `tests/dump_ignored_tests` (584k lines of Rust scanned for `#[ignore]`, the committed baseline reproduced) | same on all three channels, on both loft backends | 298 ms → 5.9 s interpreted, 1.7 s native — **slower, 20× and 6×**: 91 % of the interpreted run is `File.lines()`, a per-character loft loop | 58 / 33 → 87 / 66 | **longer**: regex without capture groups (the reason and the name re-found by hand), a local walk, an eleven-line header as eleven prints | 005, 011, 013, 014 |

How to read a row: **behaviour** is the twin's verdict (stdout, stderr, exit status and the
written files, by content where the original's numbering follows an unordered `find`);
**performance** compares the original as it runs with the port in the mode its `#!` line
names; **clarity** is loft ≤ original in non-comment lines, and a longer port names why in
the reasons column rather than in prose.
