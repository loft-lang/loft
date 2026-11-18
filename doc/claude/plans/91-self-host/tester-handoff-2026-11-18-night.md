# @PLN91 — the tester's handoff, 2026-11-18 night

Supersedes [tester-handoff-2026-11-18.md](tester-handoff-2026-11-18.md) for the state and the
try-gate; that file still describes the cycle scripts.  Read
[porter-ledger.md](porter-ledger.md) § What the testing side is for FIRST: it is the working
agreement with the owner.

## State

- Porter tip **4d46402e8** on `tuxedo-pln204-int32`: gate E **1151 of 2086** agree, 0 differ, no
  definition drop (`dropped.txt` at 0).  First pass: 10942 of 16204 functions agree.
- A try (979ff1cf4) on that tip was waiting for the tip's cycle; the watcher answers it.
- `loft30-gate` holds the tester's commits; the porter merges it at each promotion.
- The porter is on the hoisted default ([hoisted-defaults.md](hoisted-defaults.md)).

## The standing order

1. Stay AHEAD: at least two prepared subjects beyond the one the porter is on.  A prepared
   subject is a page here plus a probe in [probes/](probes/), run on the Rust front end on both
   backends AND on the port's current build (the message says which cases the port fails).
2. FIX a defect that blocks the port or that the port would mirror — matrix first, the Rust
   change announced to the porter (functions and new form) before it lands.
3. FILE every other defect found on the way, measured on a fresh main first (`make
   install-main`, then the repro on `~/.local/bin/loft`).

Prepared and unused: [linked-group-literal.md](linked-group-literal.md),
[vector-slice.md](vector-slice.md).  In use: [keyed-field-write.md](keyed-field-write.md) — its try
(979ff1cf4) uncovered a keyed local as the subject of a record tail, which keeps `__retbuf`
where Rust renames the buffer (the `keyed_*` cases of `probes/record-tail-default.loft`).  The buffer is at its minimum: pick the
next from the programs one refusal from agreeing (`kr-tools/try_moves.py`, the try answers'
"lost a refusal and still refused" list).

## Open with the owner

**"A vector local bound from a later-declared function's vector result"** (2 programs alone, 1
with one other) is an accident of Rust's pass order, not a rule: `own_fresh_binds` reads a
callee's result type as the pass meets it, so the same function emits its two frame owners in
another ORDER when its callee is declared below it.  The port decides it order-free and
refuses the programs by name (`el_refhome.loft`).  Put to the owner: make Rust order-free
(recommended), have the port copy the dependence, or leave it refused.  No page or probe is
prepared until that is decided; the reduction is two functions calling `above()` / `below()`
after `p = [9]`.

## Today's Rust-side work

- **loft#2561** fixed on `loft30-gate` (6709fe1ae … 7332c08aa): a `-> S?` result read out of a
  held store is tested for absence unless its source is proven present.  Guard:
  `tests/scripts/2561-…loft`.  The port follows it since 4d46402e8.  The issue stays open
  until the branch reaches main.
- Filed, not fixed: **loft#2562** (C backend, a returned keyed-collection FIELD of a local
  record is freed twice), **loft#2563** (a linked group's keyed member given a keyed value
  leaves the other members empty; `silent-wrong`).
- Failing on the branch and not from this work: `index_hygiene_clean` (design decisions 161
  and 162 have no guard).
- Not run: rustc-native on the loft#2561 change; the guard is in `tests/scripts/`, not yet a
  language-suite cell.

## The try-gate (mailbox: loft#2560, closed and locked on purpose)

- Porter side: `scripts/try_gate.sh` posts the working tree as a bundle in a comment and
  returns at once.  Tester side: `kr-tools/try_poll.sh`, started by the watcher.
- A CLEAN try is acknowledged automatically with what it moved (`kr-tools/try_moves.py`).  A
  try with ISSUES gets an automatic "do not promote this one yet"; the watcher prints
  `TRYPOLL ISSUES … ANALYSE` and the analysis is the tester's, sent as a message in the
  porter's three-part shape (clean or not; each definition that stopped agreeing with the
  refusal and source line; each differ with the first differing line of both sides).
- A try waits while its base is not gated (`TRYPOLL WAITING`), may stack on an earlier clean
  try that did not touch `src/`, and tries are serialised by `kr-tools/try.lock`.

## This box

- Watcher: `kr-tools/auto.sh | grep --line-buffered -E "^REPORT|FAILED|REBASED|MERGE|NOT FOUND|BASELINE|DID NOT BUILD|^MAINCHECK|SYNC|^TRYPOLL"`
  as a Monitor; it expires every 30 minutes — re-arm.
- Worktrees: `loft-keyed-ranges` is the cycle's (never edit it); `loft-front` is for Rust work
  and its tests; `loft-docs` is for docs and probes.  Land with
  `kr-tools/land.sh <sha…>` from `loft-front`, or `LAND_TREE=~/workspace/loft-docs kr-tools/land.sh <sha…>`;
  it refuses a tree holding other local commits.
- After a full test selection in `loft-front` (`find_problems.sh --changed` on a parser change
  runs 6500 tests and leaves 80 GB): `cargo clean --profile dev`.
- Per-tip results are in `kr-tools/<tip>/`; the tester's report of a tip is
  `tester-report.txt` there.  The porter cannot read that directory: everything it needs goes
  in a message or in a doc on `loft30-gate`.
