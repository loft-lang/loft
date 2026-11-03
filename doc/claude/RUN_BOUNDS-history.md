<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# RUN_BOUNDS — history

The incidents behind the bounds [RUN_BOUNDS.md](RUN_BOUNDS.md) states.

## Why a time bound is not a memory bound (loft#796)

A corrupted length ends in a bad dereference on one run and an unbounded ALLOCATION on the next:
loft#796 reached 59.6 GiB in seconds, and the global OOM killer took two unrelated agent sessions
with it.  Test runs (`--tests` / `loft test`) carry the 2 GiB store-heap ceiling since.

## Why `LOFT_MAX_OPS` defaults to 4e9 (loft#919)

The hang guard is a count, so it cannot tell a long run from a hung one: at 100M it tripped
legitimate library tests and reported them as infinite loops, which read the debug-assertions
gate as known-red (loft#919).
