<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 173 — Code shape: functions that fit, files that hold one subject

## Status

**Proposed (2026-09-26).**  Owner rulings that this plan builds on:

- `too_many_lines` applies to **new code**.  A function that cannot reasonably be split
  stays out of it — per function, with a reason.  A whole file never does.
- The worst files are split **one or two per cycle**, on the release beat, until nothing
  is left over the bar.  The bar is a ratchet, not a big-bang cleanup.

Tracks [@PLN173](https://github.com/loft-lang/plans/issues/173).

## The problem, measured

`src/` on 2026-09-26: **70 Rust files over 1200 lines, 31 over 3000, 18 over 6000**.  The
top of the list is `src/scopes.rs` (26,892 lines, 554 fns), `src/parser/mod.rs` (23,422),
`src/parser/control.rs` (20,034), `src/data.rs` (13,599), `src/generation/hoist.rs`
(12,467), `src/main.rs` (12,447).

Three things let it get there.

1. **The split signal is blind to `impl` blocks.**  `file-sizes.py` calls a file one
   subject when its largest section is over 50 % of it, and it sections a Rust file on
   `impl` as if that were a topic.  `parser/control.rs` is "97 % `impl Parser`", so the
   report says KEEP.  In Rust an `impl` block is a container, not a subject; the language
   lets one `impl Parser` be spread over any number of files.
2. **The function-length rule is stated and not enforced.**  CODE.md § Functions says "no
   functions longer than ~50 lines", DEVELOPMENT.md says new code triggering
   `too_many_lines` refactors in its own commit step — and `src/lib.rs` and `src/main.rs`
   put `clippy::too_many_lines` in the crate-wide `#![allow]`.  That also makes the 54
   per-function `#[allow(clippy::too_many_lines)]` under `src/` dead.
3. **Nothing stops growth.**  `M-file-sizes` is a report read once per cycle.  A PR can
   add 400 lines to a 20,000-line file and no gate notices.

Why it matters more here than in a human-maintained tree: a 20k-line file is roughly 250k
tokens.  No agent reads it whole — it greps, edits a slice, and never sees the neighbouring
functions.  That is a manufacturing site for the class of bug the formal-rule tags exist to
catch: code that is locally right and globally inconsistent.

## The rules (the contract, one line each)

| # | Rule | Where it lives | Checked by |
|---|------|----------------|-----------|
| C1 | `clippy::too_many_lines` fires on every function; no crate-wide allow | `src/lib.rs`, `src/main.rs` | `make ci` (clippy, `-D warnings`) |
| C2 | A function that cannot reasonably be split carries `#[expect(clippy::too_many_lines, reason = "…")]` with a reason a reader can check | CODE.md § Functions | clippy (`expect` fails when the lint no longer fires, so a stale exemption is removed by the gate, not by memory) |
| C3 | An exemption whose reason is `inherited` is a debt marker: the split campaign removes it when it touches the file; new code never gets one | CODE.md § Functions | `make clippy-review` (lists live-but-unexplained suppressions; `inherited` counts as unexplained) |
| F1 | A code file over `CODE_BAR` (1200) may not grow, net, in a PR; a file under it may not cross it | DEVELOPMENT.md gate table | `A-file-sizes` (new; base-relative, like `docs-lint-gate`) |
| F2 | Each cycle splits the report's top pick, one PR per file, moves only | RELEASE.md § Monthly file split | `M-file-split` (new checklist row, gate class) |
| F3 | A split PR is a pure move: no signature, behaviour or comment change; `make ci` green; `git diff -M --color-moved` reads as moves | `.claude/skills/split-file` | the skill's own check + PR review |
| F4 | The report lists files by split value, not by size; `impl` blocks are transparent | `scripts/file-sizes.py --pick` | `M-file-sizes` (existing report row, now also the payoff check on last cycle's pick) |

C1–C3 are the "new code" half.  F1 stops the bleeding.  F2–F4 are the burn-down.

## Phase 1 — functions (one PR, mechanical)

1. Remove `clippy::too_many_lines` from the `#![allow]` in `src/lib.rs` and `src/main.rs`.
2. Run the three CI clippy legs (`make clippy-review ARGS="--legs all"` already knows
   them).  Every function that now fires gets
   `#[expect(clippy::too_many_lines, reason = "inherited @PLN173")]`.  Nothing is
   refactored in this PR — that is what the split campaign is for.  The existing 54
   `#[allow(clippy::too_many_lines)]` become `#[expect]` in the same pass (clippy-review's
   throwaway-worktree trick, made permanent for this one lint).
3. Functions that are long **by nature** — per-opcode `match` arms, the generated
   `fill.rs` body, a diagnostics table — get a real reason instead of `inherited`:
   `reason = "one arm per opcode; splitting would hide the table"`.  The owner's rule: the
   exemption is per function, never per file, and the reason must survive the deletion
   test from DOC_QUALITY.md (delete the incident, does the rule remain?).
4. CODE.md § Functions gains C2 and C3 as written above and drops the "~50 lines" number
   in favour of "the `too_many_lines` bar" (commit the goal, never the position —
   DOC_QUALITY rule 5).  DEVELOPMENT.md's gate table row "Function length" changes from
   *skip pre-existing* to *pre-existing ones carry `#[expect(… "inherited")]`; a function
   you touch loses its `inherited` tag — split it or write the real reason*.

**Falsified when:** a new 120-line function without `expect` fails `make ci`; an `expect`
on a function that has shrunk under the bar fails `make ci`; `make clippy-review` lists
every `inherited` exemption and the count is the campaign's function-side scoreboard.

## Phase 2 — the report picks (one PR, `scripts/file-sizes.py`)

1. **`impl` transparency.**  Section Rust files on items *inside* `impl`/`mod` blocks (fns,
   nested types), not on the block.  A file's "largest section" is then its largest
   function, and a 20k-line file of 500 small functions reads as SPLIT, which is the
   truth.
2. **Split value, not size.**  Rank by `lines × fns_over_bar_share` — the files where many
   comparable functions share one file come first.  On today's tree that puts
   `scopes.rs` (554 fns) ahead of `parser/control.rs` even though control.rs is bigger
   only in one respect, and it puts `generation/hoist.rs` (245 sections, largest 5 %)
   near the top.  Generated files stay excluded.
3. **`--pick N`** prints the N files for this cycle with the seam the script can see
   (which types each fn touches, which fns call which — a first grouping the skill
   refines).  `--pick` writes nothing; the `M-file-split` tick records the pick's commit
   in `releases/<cycle>/`, the same way every tick records its commit.
4. **`--ratchet`** compares the working tree against a base ref: exit 1 if any code file
   over `CODE_BAR` has more lines than at base, or any file crossed the bar.  Moved text
   is not growth: the check runs on `git diff --numstat -M` so a split PR that adds three
   new files and shrinks one passes.  This is `A-file-sizes`, wired next to
   `docs-lint-gate` in ci.yml and in `make ship`.

**The knob the owner may want to turn:** F1 as written allows zero net growth of an
over-bar file.  A five-line bug fix in `scopes.rs` then needs to carve five lines out —
the intended pressure, but blunt.  The alternative is a small allowance (say 40 lines net
per PR) which keeps fixes cheap and still bounds drift to a few hundred lines per cycle,
which one split PR removes many times over.  Default in this plan: **zero**, because an
allowance is a number in prose that reads as a measurement, and because the skill's
*carve* mode (below) makes the five-line case a two-minute job.

**Falsified when:** a PR adding 10 lines to `scopes.rs` fails; the same PR that also
moves one 200-line `impl` block to `scopes/lookup.rs` passes; a PR touching only files
under the bar passes untouched.

## Phase 3 — the doer (one skill, `.claude/skills/split-file`)

A skill, not a script: moving `impl` blocks is mechanical, choosing seams is not, and the
agent already does this well from a tight brief.  The skill (draft alongside this plan)
has two modes:

- **split** — one file from `--pick`, into `dir/<subject>.rs` per cluster, the original
  keeping the type, the shared helpers and a module header listing the parts.
- **carve** — one subject out of an over-bar file the builder must touch anyway, so F1
  passes.  Same rules, smaller scope, same PR as the fix is *not* allowed: carve first,
  merge, then fix, so the move never mixes with a behaviour change under squash-merge.

Both end in a pure-move PR whose title is `split <file> into N parts` or
`carve <subject> out of <file>`, with the skill's own move-share check (`git diff -M
--stat` plus a count of non-move hunks; more than a handful fails the skill) run before
`make ci`.

Seam guidance the owner can set in the skill in one line each (the agent picks a
defensible seam otherwise, and a re-split later is cheap because moves are cheap):

- `src/parser/` — by grammar production or by AST node kind?
- `src/scopes.rs` — by lookup / declaration / lifetime, or by the type the scope holds?
- `src/generation/hoist.rs` — by rewrite (its formal doc `formal/rewrites.md` already names them).

## Phase 4 — the beat (RELEASE.md, checklist)

- **`M-file-split`** (gate class, cadence `mid`): this cycle's pick is split and merged;
  ticked with the PR link and the pick's commit.  Two picks when the previous cycle's
  landed clean, one otherwise.
- **`M-file-sizes`** stays a report row and gains the payoff check the bug review already
  does: did last cycle's split land, and did the file it came from stay under its new
  size?  A split whose parts have regrown is a seam chosen wrong; re-open the seam, do not
  add a third part.
- **`A-file-sizes`** (automatic, `mid pre`): the ratchet, F1.
- RELEASE.md gets a § *Monthly file split* beside § *Monthly bug review*, same
  "one cycle, one file, one PR" shape.

## Horizon

At one or two splits per cycle, the 18 files over 6000 lines are gone in about a year; the
ratchet means the 70 over 1200 never get worse meanwhile, and every over-bar file touched
for other reasons loses a subject on the way (carve).  The function side shrinks the
`inherited` count on the same beat: a split PR removes the tag from every function it moves
that it can split on the way, and leaves a real reason on those it cannot.  `make
clippy-review` and `make file-sizes --all` are the two scoreboards; neither number is
written into a doc.

## Out of scope

- Refactoring for its own sake inside a split PR — a split moves code and changes nothing
  else.  Anything found on the way becomes an issue.
- Docs: their ceiling and burn-down are @PLN172.
- `too_many_arguments` (89 suppressions), `struct_excessive_bools` (19): different rule,
  different plan.  This one is about length.
