<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — tester handoff, 2026-11-20

The tester of the @PLN91 port (gate E): gate the porter's tips and tries, analyse every result
to *what to code*, keep THREE untouched prepared subjects ahead of the porter (owner, 11-20)
plus one in reserve, and fix the Rust defects the work turns up (matrix first, chokepoint,
guard cell, both backends).  The rules of the role:
[porter-ledger.md § What the testing side is for](porter-ledger.md); the earlier state:
[tester-handoff-2026-11-19-late.md](tester-handoff-2026-11-19-late.md).

## Peers

| who | list name (address with `SendMessage`) | what |
|---|---|---|
| the porter | "Front-end lambda, iterator match handoff" [58066d] | ports the elaborator; work branch `tuxedo-pln204-next`, tested line `tuxedo-pln204-int32`; its handoff `handoff-2026-11-20.md` |
| loft3 | "Parallel threading test" [a82a0d] | joins and PRs to main; places suite cells (with `make falsify`) |
| this role | "loft-7b" (the name changes with every session; `ListAgents` prints it) | |

Session names change after a restart.  Check `ListAgents` and reply to an incoming message's
`from`.  The porter said a stale bridge address answered HTTP 409.

## State at the hand-over

- **Tested line** `tuxedo-pln204-int32` = **e27ec2076**: 1236 of 2114 agree, 0 differ.  Its cycle
  ran at 09:04 and its GitHub baseline landed as c5d2957ae.
- **Ungated, the porter's stack:** a15f52718 (CLEAN, 1239), 59085462f (1256, 2 differ),
  f9a55db69 (1267, 3 differ), then d74aba07d (not tried).  It cannot be promoted until three
  fixes are in.  The analyses are on loft#2560, comments 7034215804 and 7034354115:
  - `u32` checked cast: the dn4cast's max bound is a 64-bit literal (`int_literal`,
    operators.rs:16);
  - `(v as i32?) ?? 0` over a bare variable gets NO `__ncc` holder in Rust, on either pass.
    The port's pass 1 makes one when another `??` holder follows.  (Latent beside it: an
    append item `[(v as i32?) ?? 0]` drops the cast.)
  - R-SliceBorrow `n_rebind`: a slice of a by-value text parameter takes no deps, also after
    the function rebinds the parameter (its `__tp_` copy has deps; the slice must not).
  - Op-Order: a stray `compare` definition when one operator is overloaded on its second
    operand (`(self: A, other: B)` then `(self: A, other: A)`), old, shown by @C132.
- The newest try directory is `kr-tools/try-f9a55db69`.  The porter's fixes for the four
  points above arrive as its next try.  `ls -dt kr-tools/try-*` lists by date; the names sort
  by sha.

## Backlog (tracker `~/workspace/kr-tools/backlog.md`)

Untouched, each with a page and a probe judged on --interpret, --native-c and --native-rust
(the porter's next picks are the early tuple return, early-tuple-return.md, sent, and "a vector
match answering a record"):

1. slice-single-alternation.md: `[(A { n } | B { n })]` (6 programs, 3 sole)
2. ref-tuple-param.md: `fn swap(p: &(integer, integer))` (13 programs, 2 sole)
3. keyed-range-of-a-field.md: `for s in sdb.map[15..25]` (5 programs, 1 sole)
4. *reserve:* vector-field-narrow.md: a narrow vector field from a vector place, plus the
   struct-first row order (latent in 14 refused programs)

On every porter event, check what it took (its log since the last try, and the refusal sites
by `git grep` at its tip) and refill to three from the ranking (`where.tsv` of the newest try:
sole programs ×2 + programs met / 3).

## Rust fix of 2026-11-20

| issue | what | state |
|---|---|---|
| #2594 (main) | a record local in a block, first bound to a literal built in the function's buffer and then rebound from calls, leaked every displaced store and double-freed when the rebind read it (`@FR-B-Copy`); fixed by a pre-scan move (`literal_moves_in`): the store moves into the local, the buffer is reset to null | fixed on loft30-gate a672248b1; guard placed by loft3 in loft-suite fb83f71.  The fix is in `src/scopes/`, so gate E (oracle before the scope pass) does not see it; the scope gate (`scope_survey.py`) gives the same verdicts with the old and the new Rust (its 3 programs: 23 definitions unscoped at "a compiler-made reference", 0 differ).  Listed in scope-frees.md for when the scope pass reaches that local |
| #2591 (silent-wrong, main) | a name a slice pattern's alternation captured (single-element, multi-element, optional group) kept naming the capture after the arm (`@FR-B-Scope`); a capture named like the subject looped `parse_match_inner` into OOM | fixed on loft30-gate f406c3312; guard cell placed by loft3 in loft-suite 302a24b (falsified on 203d1bf6c) |

## Instruments changed

- `kr-tools/build_tools.sh`: try.sh and tip_test.sh build the two gate tools through it.  It
  caches by (loft binary, selfhost/, default/), so a promoted tip's cycle reuses its try's
  tools.  It holds a box-wide lock (one gcc at a time) and keeps its scratch on disk
  (`/var/tmp/kr-build-*`).  Cache: `kr-tools/tool-cache/`, 6 newest kept.
- `kr-tools/tmp_clean.sh`: called at the end of every try and cycle.  It removes this user's
  loft leftovers in /tmp older than 3 hours.  The gate run's own temporaries go to `$T/tmp`,
  removed after.  /tmp is RAM on this box.  The owner refused a systemd timer for it.
- `kr-tools/subj/`: `cases.sh PROBE helper,… [TOOLS]` splits a probe one function per case
  (a `chk_<case>` checker rides along) and prints the port's verdict per case.
  `probe.py FILE --tools DIR --show fn` prints Rust's form and the port's diff; a refusal at
  a signature (line 0) is attributed.  The splits go to `subj/cases/c.*`, 10 kept.
- `kr-tools/answers/<sha>.md` → `try_answer.sh <sha> <file>` posts an analysis on loft#2560.

## Habits the day taught

- **Bound every probe run:** `( ulimit -v 2000000; timeout 60 loft … )`.  A probe that hit
  loft#2591 made the Rust front end allocate without bound and OOM-killed the session twice.
  The session was down 01:00–08:50, the watcher with it, and the porter's tries waited all
  night.
- **One heavy job at a time:** before an elaborator build, a cargo build or a native probe,
  check `systemctl --user list-units 'kr-*'`.  A running `kr-try-*` or `kr-cycle-*` means wait.
- **`--native` is the C backend.**  The rustc writer is `--native-rust`.  A three-backend
  judgment is `--interpret` + `--native-c` + `--native-rust`.  Small probes build in about a
  second on the rustc writer (the prebuilt rlib), so mutate a callee's body to prove a run.
- One watcher only: `pgrep -af kr-tools/auto.sh` must show one process.  Stop an old Monitor
  task before arming a new one.
- A corpus dump before and after a Rust fix (`kr-tools/order-free/dump_all.py`, scratch on
  disk, the SAME working directory for both runs: import programs resolve their libraries
  relative to it) is what lets a parser fix land without the full suite.
