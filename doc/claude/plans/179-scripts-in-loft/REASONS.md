<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN179 — reasons to reach for another tool (driven to zero)

One row per reason a port MET to prefer Python or bash over loft — recorded when met, never
guessed, struck only when the port that met it re-measures green on that axis
([README § The bar](README.md)).  `OPEN` is the plan's headline number.

**OPEN: 3**

| # | met by | axis | the reason | the fix it needs | status |
|---|---|---|---|---|---|
| 1 | `scripts/script_census` (strand 1) | performance | start-up of a `hello`, INSTALLED binary, idle box, 2026-09-30, 5 runs: `--interpret` 15–22 ms (Python 15–19, bash 5) — at the bar; the DEFAULT path (native, warm cache) 32–37 ms — 2× Python.  The from-source binary's 78–84 ms was rule 4 (program cache off), not the language | the warm-native start-up: where the ~30 ms goes (`parse_default` is 5 ms of it), and the `--interpret` pin in a script's `#!` line ([MODES.md](MODES.md)) so a short script meets the bar today | open — narrowed to the native path |
| 2 | `scripts/script_census` (strand 1) | clarity | tokenising a line needs a character loop: `split` takes one separator and `find` answers bytes against `len`'s characters | a regex-shaped split in the stdlib, or the `regex` library measured for it | open |
| 3 | `scripts/script_census` (strand 1) | behaviour | the interpreter segfaults at `OpFreeText` on `argv_call(self.result[n - 1] ?? "")` inside a struct method that also appends to `self.result`, once enough heap is behind it; `--native` runs it; four other spellings run; no extraction reproduces it yet | [loft#1773](https://github.com/loft-lang/loft/issues/1773) — the census binds the element to a local first, marked to fold back | open |
