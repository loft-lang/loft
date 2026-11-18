# @PLN91 — the tester's handoff, 2026-11-18

The tester runs every corpus measurement for the porter (owner's division, `handoff-2026-11-17.md`
§ Working mode): the porter ports and probes single programs and promotes `tuxedo-pln204-int32`;
the tester gates each tip, reports, and lands baselines on `loft30-gate`.  Aiding the porter comes
first; the scope pass in loft (`scope-frees.md`) fills the gaps.

## State at handoff

- Porter tip **6ec20bb03** verified: gate E **1092 of 2191** agree, 0 differ.  Its real-gate
  baseline landed (cb97cdabb).  Nothing promoted since.
- The only standing definition drop is R-SameRead/a-slice-match-reads-its-subject-once::vector<Tok>
  (a type-registration ORDER difference: Rust registers `vector<vector<byte>>` with `vector<byte>`,
  the port last; reported, the porter takes it when that program's other refusals lift).
- Scope gate (`scope_survey.py`): **135 of 2191** programs, **19152 of 35643** definitions agree,
  baseline pinned at suite fac3652.
- Front layer (`run_image_survey.py --front`): 50 of 1291 programs run from the loft front end alone;
  its two faults are IR passes after the scope pass (R-ExpPow, R-TailSelf).

## The machinery (this box: `~/workspace/kr-tools`, outside the repo)

- **Watcher**: arm `kr-tools/auto.sh | grep --line-buffered -E "^REPORT|FAILED"` as a Monitor (it
  expires every 30 min: re-arm).  `auto.sh` starts a `cycle.sh` systemd unit per new porter tip.
- **A cycle** (`cycle.sh` → `tip_test.sh`): builds the tip in the cycle worktree
  `../loft-keyed-ranges`, runs the dev gate with `--functions-against` (FUNCTION DROP = a pinned
  definition stopped agreeing), the skip sweep (`skip_sweep.py`: families that alone block programs,
  skipped; staleness judged by definition, `beside` noise separated), the where-list (`where.py`:
  source lines of the top 10 families), merges the tip into the worktree's `loft30-gate`, triggers
  the real gate on GitHub; `gh_baseline3.sh` waits for it and merges the baseline commit.
- **Per tip, by hand**: read `kr-tools/<tip>/gate.txt`, diff `verdicts.tsv` against the previous tip
  (refused→agree / agree→differ / changed refusal sets), check every claim of the porter's message,
  reduce any differ to its first differing line and source (drive `elab_gate.py`'s own code: a hand
  invocation of `elab` does not reproduce the gate's environment), then SendMessage the porter.
- **Other tools**: `never_arms.py` (Rust-image scans), `trend.py`, `family.py`, `next_moves.py`.

## Rules learned the hard way

- Work in `../loft-front` (detached HEAD); land a commit by cherry-picking into
  `../loft-keyed-ranges`'s local `loft30-gate` **only when no `kr-cycle-*` unit runs**, then push.
  `gh_baseline3.sh` merges and pushes without fetching: a push from elsewhere breaks it.
- **Never rebase `loft30-gate`**: it holds the cycle's merges; a rebase flattens the porter's sha.
- Never edit `selfhost/src` while a build of it is under four minutes old; edit a running script by
  writing a new file and `mv`.
- The real gate builds its tools as a development build (`LOFT_ELAB_TOOL_BUILD=dev`) and one at a
  time (`LOFT_ELAB_BUILD_JOBS=1`) under `pipefail`: the elaborator's optimised compile passed 5.5 GB
  in cc1 and no longer fits the 7 GB runner.
- A definition header's name runs to ` type=` (generic instances hold spaces).

## Next

1. Keep gating the porter's tips (first priority).
2. Scope pass, from the gate's ranking: the closure records' drop functions (≈280 definitions —
   Rust keeps or removes each capture's release by ownership: `closure_keep.rs`,
   `capture_adoption.rs`; design first), then `@FR-H-TextReplace` (`text_replace.rs`, an
   interprocedural analysis, 24 definitions), then a returned value block's frees (`insert_free.rs`).
3. Open: loft#2545 (an `if` with a forward call and `[]` arms types void on pass 1) — filed, unfixed.
