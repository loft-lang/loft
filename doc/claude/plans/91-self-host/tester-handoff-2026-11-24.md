# @PLN91 tester handoff — 2026-11-24 ~01:00

Resume point for the tester session (`tester`).  Peers: `porter` (the self-host port, branches
tuxedo-pln204-int32 / -next), `merger` (loft3, joins and PRs, writes as
`bridge:session_015fP1u1me7iBdHTmeHVyEbQ`).

## State

- **Parity reached and enforced.**  Every IR-agreeing program agrees on contract, state and
  reasons; since c4197ed04 (`elab_gate.py`) an `agree` requires all four, with no exception (a
  not-derived fact, a one-sided contract function, a silent layer all differ).
- **Gate at the porter's 128eaca93** (promoted to int32): 1409 of 2139 agree, 0 differ; refused
  programs' definitions 13 774 of 22 819 agree (tester try baee497a4).
- **Landed on loft30-gate today** (tester): P1 buffer_exits, rebinds/user_named fixes (two Rust
  contract-report defects), @FR-B-Ref-Repoint, the agree merge, width32 *fact*, the
  register-then-refuse sites (refused lambda 7201cfb57; vector element / refused default /
  expected empty literal 6711b88de), the one-sided-definitions gate signal d8c7fa070, the
  per-compiler C split 42b939636, worklists refusal-worklist.md (+ the rule, fcf040e53).
- **PR in flight:** merger joined loft30-gate 35bc43dc2 + porter 128eaca93 + tuxedo-pln225 onto
  tuxedo-pr2622-next @cb9c3e2f7.  Open with merger: the two receipts I had armed
  `FALSIFY_NATIVE=rust` (G-Mono 1841, L-Apply a-fn-ref-call) can no longer run (pln225 R2
  refuses the rustc backend); under @PLN225 R4 both are writer-only and RETIRE (delete the two
  tests/falsified patches, remove the `@falsified-by` blocks, keep `@falsified-at`).  I offered to
  make the edits; waiting for their answer.

## Running at handoff (scratchpad `/tmp/claude-1000/-home-jurjen-workspace-loft/0d286d8f-82b3-45a8-832e-9c2f6b57f143/scratchpad`)

1. The porter's tip **f6244c9eb** (dfc0ee38e chain_of + loft30-gate 35bc43dc2) as a try against
   922051ab5 → `pnext-try.out`.  Clean = chain_of and the 18 width32 definitions back, nothing
   else lost; then tell the porter (they send it to merger as a fix for the PR).
2. After it, the numbers-pool capture with try-baee497a4's tools → `np/shift2.txt`,
   `np/verify2.txt` (the next R9 sites, ranked).

## In progress: R10, the 32-bit regime's lowering

LOCAL commit **63dae8fb6** in ~/workspace/loft-front (NOT gated, NOT landed): the `#width 32`
refusal lifted (el_definitions) and **W-Name** (el_types `type_of`: in a regime file `integer`
without `limit` → `i32`, `float` → `single`).  Next, in order, each a Rust routine to mirror:
- **W-Op** — el_ex_call.loft `binary` (after `builtin_op`): when the file is under the regime and
  both operands are `is_width32` integers, wrap the result in `OpNarrow32` (not for `& | ^`) and
  type it `i32` forced 4 (operators.rs ~6823); the compound assignment the same (~1930).
- **I-Join / W-Lit** — Rust wraps the 32-bit join and the right operand in `OpRangeDefault`
  (the first two diffs of the harness).
- **W-Count** — `count_type()` (forced-4 i32 in the regime) at the port's ~48 `i32_spec()`
  counter sites; **W-Call**, **W-Store** (width32.rs).
Harness: `w32/run.sh` (env `FILES=…` for a subset; the elaborator from source, so no tool
build), 5 of the 10 regime programs have a Rust image; tiny probe `w32/p/a.loft`.

## Next after R10

R9: take the fresh ranking (`np/shift2.txt`); the tuple-vector registration (52 shifts) needs the
whole `__tuple<…>` record registered.  The refusal worklist's R1–R8 are the porter's (R2 heap
constants in progress there).

## Tooling notes learned today

- Never edit kr-tools/try.sh while a try runs (bash reads it as it goes); stop the try first.
- Two queued jobs both waiting on `run/try.pid` can start together; chain them in one job.
- Land from a clean worktree: `LAND_TREE=<scratch>/land-wt ~/workspace/kr-tools/land.sh <sha…>`;
  docs from ~/workspace/loft-docs.
