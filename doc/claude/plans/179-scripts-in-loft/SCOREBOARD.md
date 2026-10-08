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
| `doc/claude/plans/35-match-peg/rest-store-lifetime/gen_probes.py` → `gen_probes` (33 probe files + a manifest, committed output; the twin's `--tracked` mode) | same on all four channels, on both loft backends; a one-byte wrong port goes red on `files` | 34 ms → 24 ms interpreted, 16 ms native | 171 / 125 → 155 / 127 | **equal**: a struct where Python had a tuple loop, backtick literals where it had triple quotes, the same doubled braces in both | — |
| `scripts/wasm_bundle_stamp.sh` → `scripts/wasm_bundle_stamp` (one SHA-256 over 8 named files + `default/*.loft`, through the `crypto` library) | same on all three channels, on both loft backends | 12 ms → 25 ms interpreted, 47 ms native — **slower**: a `cat | sha256sum` pipeline is under loft's own start-up | 27 / 8 → 24 / 16 | **longer**: a glob is a `list_dir` + filter, and `set -e` is an `assert` per file | 1, 6, 8, 9 |
| `tests/dump_ignored_tests.py` → `tests/dump_ignored_tests` (584k lines of Rust scanned for `#[ignore]`, the committed baseline reproduced) | same on all three channels, on both loft backends | 221 ms → 655 ms interpreted — **slower, 3.0×** since `lines()` runs its compiled loft body (@PLN181, finding 013; before it 298 ms → 5.9 s interpreted, 1.7 s native — 91 % was the per-character loft loop); native not re-measured | 58 / 33 → 87 / 66 | **longer**: regex without capture groups (the reason and the name re-found by hand), a local walk, an eleven-line header as eleven prints | 005, 011, 013, 014 |
| `scripts/opl_points.py` → `scripts/opl_points` (200k OPL lines on stdin → the point corpus; the twin's `--stdin` mode) | same on all three channels, on both loft backends | 297 ms → 1.62 s interpreted — **slower, 5.5×** since `split` runs its compiled loft body (@PLN181, finding 013), measured on 200k lines replicated from the committed `sample.opl`; what remains is the port's own per-field loop and `%hex%` unescape (before it, on the original input: 870 ms → 8.8 s interpreted, 1.4 s native — 73 % was `text.split`); native not re-measured | 71 / 37 → 97 / 74 | **longer**: the `%hex%` unescape is a hand loop where Python had `re.sub` with a group, and half-to-even rounding is spelled out | 011, 013 |
| `scripts/check_bundle_fresh.sh` → `scripts/check_bundle_fresh` (the first script over `run`: `git diff --name-only` against a declared artefact→source table; four recorded cases — an unrelated change, a source without its page, both, an unresolvable base — through `--replay`) | same on all three channels in every case, on both loft backends | 122 ms → 39 ms interpreted, 70 ms native — **faster**, with both sides paying the same bash `git` shim | 67 / 27 → 79 / 38 | **longer**: a `Bundle` struct where bash had a tab-separated `printf` row (a gain in names), and a four-line membership loop where bash had `grep -qxF` | 017 |
| `scripts/check_contract_goldens.sh` → `scripts/check_contract_goldens` (the contract-goldens gate: CONTRACT_VERSION read from the manifest, three `git diff` calls, a decision table with its own `--self-test`; three cases, two of them recorded) | same on all three channels in every case, on both loft backends — after the original lost its `grep -P`, which BSD grep has not, so the gate had been red on macOS | 376 ms → 183 ms interpreted, 172 ms native — **faster** (the self-test, no git: 38 ms → 133 ms, start-up) | 93 / 58 → 113 / 83 | **longer**: the version is read with a character loop where bash had one regex capture, and `… \|\| exit 1` is a sentinel and an `if` | 011, 019 |
| `scripts/rule_predicate_audit.py` → `scripts/rule_predicate_audit` (208 Rust files scanned for repeated `matches!` type-lists; `--near`, `--min N` twinned too) | same on all three channels in every mode, on both loft backends — after the original's `glob` was pinned to sorted order, since readdir order printed a different report per filesystem | 110 ms → 130 ms interpreted, 200 ms native — **equal**: a `find` per `matches!` and a byte scan of the 600-character window | 88 / 55 → 227 / 177 | **longer, 3×**: two regexes with groups and a bounded non-greedy window spelled as scans, a `frozenset` as a hand-sorted vector, `a ^ b` as a merge, first-seen order beside a hash | 011, 015 |

How to read a row: **behaviour** is the twin's verdict (stdout, stderr, exit status and the
written files, by content where the original's numbering follows an unordered `find`);
**performance** compares the original as it runs with the port in the mode its `#!` line
names; **clarity** is loft ≤ original in non-comment lines, and a longer port names why in
the reasons column rather than in prose.
