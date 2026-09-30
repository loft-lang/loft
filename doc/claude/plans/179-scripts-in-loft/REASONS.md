<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN179 — reasons to reach for another tool (driven to zero)

One row per reason a port MET to prefer Python or bash over loft — recorded when met, never
guessed, struck only when the port that met it re-measures green on that axis
([README § The bar](README.md)).  `OPEN` is the plan's headline number.

**OPEN: 2**

| # | met by | axis | the reason | the fix it needs | status |
|---|---|---|---|---|---|
| 1 | `script_census.loft` (strand 1) | performance | start-up: a `hello` on the from-source binary is 78–84 ms against Python's 13–17 and bash's 4 (idle box, 2026-09-29); `LOFT_TIMING=1` puts ~110 ms of a cold run in `parse_default`, and the program cache (on only for an INSTALLED binary — PERFORMANCE.md § Which loft am I measuring) brings that parse to ~6 ms | an idle-box measurement of the installed path first, then whatever the remaining gap names — the bar is Python's `hello` | open |
| 2 | `script_census.loft` (strand 1) | clarity | tokenising a line needs a character loop: `split` takes one separator and `find` answers bytes against `len`'s characters | a regex-shaped split in the stdlib, or the `regex` library measured for it | open |
