# @PLN91 tester handoff — 2026-11-26 (b)

Resume point for the tester session (`tester`) after a `/clear`.  Peers: `porter` (the port,
branches tuxedo-pln204-int32 / tuxedo-pln204-next, writes as
`bridge:session_015fP1u2RTCyGqEvFJ5edvkL`), `merger` (joins and PRs,
`bridge:session_015fP1u1me7iBdHTmeHVyEbQ`).  The previous handoff is
`tester-handoff-2026-11-26.md`.

## First acts

1. Read `/var/tmp/p229-b3-time2.out` — the B3 run-time measurement, which was running at
   handoff (`b3_time.sh` in the old session's scratchpad: the gate run with the D1-built
   single-unit elaborator against the split one, `/var/tmp/p229-d1/elab-{single,split}-loft`,
   two rounds each, in `kr-try2`).  Round 1 single was 82.1 s.  B3's bar: the split tools no
   more than 15 % slower.
2. Read any porter message; their next try goes on **dc356f49d** (the try that measured
   075d704de, clean at 1634/2269), patch `git diff 075d704de <their commit>`, run as
   `kr-tools/try.sh <patch> dc356f49d`.
3. Finish and close @PLN229 (below).

## State

- **loft30-gate @ 0a699f8ed** (pushed): the whole @PLN229 stack.  The merger was given that
  sha as this branch's cut for the PR; `8acaebeae` (the porter's 11-19 WIP) is superseded —
  its three files carry 13–14 later commits.
- **Gate:** porter tip 075d704de = try dc356f49d, 1634/2269, 0 differ, 634 refused.  Every
  try since 3a46cc206 carries the @PLN229 stack and moved no verdict.
- **B3 is live:** `kr-tools/build_tools.sh` builds the elaborator with `LOFT_C_UNITS=1
  LOFT_C_UNITS_JOBS=8` (the old script: `kr-tools/tmp/build_tools.sh.before-b3`).  The unit
  objects are cached in `kr-try/selfhost/front/.loft/cache/units` (64 after the first split
  try).  First split try: 478 s against ~590 s before (cold cache).
- **N-Join** (a plain-enum match with a null arm binding a non-null local, no diagnostic;
  verified on both backends and on installed main): the owner decided other agents fix the
  Rust side; the porter drops its narrowing in `el_ex_match.loft` when the oracle has it.

## @PLN229 — what remains to close it

Built and on loft30-gate: A0–A3 (A3: `scripts/scaling-guard.sh`, nightly), R0–R2 and the R
gaps (`REPRODUCIBLE_OUTPUT.md`), B0–B2b, D0–D1 (`(R-CTable)`), and the `(R-CSplit)` fix
(02df966e1).  To do:

1. **Commit and push** the `CHANGELOG_TECHNICAL.md` entry ("Building a large program") — it
   is in this commit.
2. **Record B3's run time** (first act 1) in the plan issue.
3. **Update the plan issue body** (`gh issue edit 229 -R loft-lang/plans`): Status → done, with
   the measured results below, then **close it**.  Two things the plan named and did not
   build, to state at close: the `loft.toml` setting for the split mode (the env switch
   `LOFT_C_UNITS` is the interface; nothing asked for the setting), and option A of the
   release design (release only what a path may have assigned — scope pass, moves every
   gate-E image, sequenced with the porter; `plans/229-c-release-size.md`).

Measured results for the close (all on the elaborator, porter commit 36f3b0f19 unless named):

| What | Before | After |
|---|---|---|
| loft writing a 4000-function call chain's C | 35 s | 2.4 s |
| split build, gcc / peak memory (cold) | 114 s / 1.8 GB (pre-D1) | 54 s / 1.1 GB |
| one-unit build, gcc / peak memory (cold) | 202 s / 5.9 GB (pre-D1) | 88 s / 3.5 GB |
| rebuild after 3 small file changes (split, cached) | 106 s of gcc cold | 16 s |
| the gate's verdicts, split tools vs one unit | | identical (2486 / 20093 / 20398 rows) |

Open findings to carry into the close (not @PLN229 work): the largest unit is ONE function's
C (`assignment_inner`, 1703 lines); unparted functions still hold 14.7 MB of release runs
(`plans/229-c-release-size.md` § Open questions).

## Where things are

- Worktree `~/workspace/kr-tools/tmp/land-wt2` (detached, = loft30-gate tip): the @PLN229
  work.  Binary copies in its `target/release/` (`loft-b2`, `loft-b3`, `loft-d1`,
  `loft-pre-restructure`) can be deleted.  `target-fp/` is a frame-pointer build for
  profiling (PROFILING.md § loft's own compile time); delete when done.
- Push to loft30-gate only through `~/workspace/loft-keyed-ranges`' local branch
  (`git merge --ff-only <sha>`, then push), with no cycle running.
- Measurement dirs under `/var/tmp/p229-*` can be removed once @PLN229 is closed.
