<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Revalidating the shipped libraries

Does this loft change break a published library, and can each library be released as it
stands?  `scripts/revalidate_libs_local.sh` answers both, locally, for the whole registry.
The incidents behind each rule here are in [DEVELOPMENT-history.md](DEVELOPMENT-history.md).

## Why a work branch needs it

`revalidate-libs.yml`, the CI gate that compiles every published library against this loft,
runs on `pull_request` and on `push` to `main` only — never on a work branch.  A language
change that retro-breaks a shipped library is invisible for as long as the branch is
unmerged.  **Run the script after any `src/**` or `default/**` change a library could
notice** — a resolution rule, a diagnostic, a lint.

## What it does

For each published package at its latest non-yanked version it reads the matrix from
`../loft-registry/index.json` through `scripts/revalidate_matrix.py` (the workflow's own
reader, so the two cannot pick different packages or versions), extracts the release TAG with
`git archive` so the sibling clones are never written to, runs the suite, and on a failure
re-classifies it exactly as the workflow does.  `--native` adds the native suites; naming
packages restricts the run to them.

Never run a library's suite inside a consumer's or a library's own tree instead: `loft test`
there writes `native-auto/` and `.loft/` and is not read-only (CLAUDE.md § Dogfood loop).

## Its two verdicts

- **COMPILE-BREAK** — a language change retro-breaking a shipped library.  The freeze forbids
  it, and it is your change's question to answer.
- **`NOT RELEASE-READY`** — a library carries warnings, so its own CI (`LOFT_DENY_WARNINGS=1`)
  refuses a release cut from it.  This is a fact about the ecosystem that was true before you
  started; the closing line says so, so a red here never sends you hunting a regression you
  did not cause.  It is red on warnings EXISTING, not on the set growing, because "can it
  ship" is a question about the absolute state.

Both are carried in the exit code.  A clean run reads `N pass, 0 runtime/env, 0 skipped,
0 COMPILE-BREAK` and exits 0.  **If yours does not, read the rows: a number whose zero is
not zero measures nothing.**

On CI the release-ready verdict is the `Release-ready` step inside each library's matrix leg
in `revalidate-libs.yml`.  It is **advisory and must never become a required check**: a
library's warning debt is information, never a veto on a loft PR.  On a release it blocks,
which is `release-gate.yml`'s standing rule — informational on a diff, blocking on a release.

## Reading a result you can trust

1. **`--self-test` before trusting a green.**  It injects a compile break and a runtime break
   and asserts the two are reported DIFFERENTLY, and checks the shared matrix policy on inputs
   each of its rules has to act on.
2. **Refresh the inputs first.**  The script reads a CLONE of the registry index, and a stale
   one measures superseded versions: a stale clone reports phantom COMPILE-BREAKs that CI on
   the same commit does not.  The first line prints
   `registry index: <sha> publish: <pkg>-<ver> (<date>)`; read that date before the verdict,
   and `git -C ../loft-registry fetch` if it is not today's.  Agreement between two agents on
   one box is not independent evidence when the clone is shared; CI on the same commit is.
3. **A SKIP is not a pass.**  A package whose release tag the local library clone lacks is
   reported "tag absent" and not run, so a registry that has moved silently SHRINKS the gate.
   The script pre-flights the skips above the table; the failure mode is reading only the
   bottom line.  Fetch the library clones with the index.  In a scratch layout,
   `git clone --bare` copies are enough, because the script only runs `git archive` on them.
4. **A scratch index disables the date line.**  A directory holding only an `index.json`
   prints `registry index: not a git checkout (unknown date)`.  There the freshness evidence
   is the version column: every row names the version it picked; check those against the
   registry's latest.

## Is a COMPILE-BREAK yours?

Only an A/B says.  Build `origin/main` in a worktree of its own and run the gate there for the
named packages, with the same index both times, or the comparison measures the index.
`$siblings` is the parent directory of the script's checkout, so symlink `../loft-registry` and
`../loft-libs-*` next to that worktree (a missing clone reports the SKIP that reads as green);
a directory holding only a fresh `index.json` is enough for the registry.  Settle the residue
on ONE tree: `git archive <pkg>-v<ver>` into a scratch directory once and run both binaries
against it with `loft --interpret --tests tests`, which also rules out the two runs extracting
differently.  Before reporting a break as news, read the issue body and the commit's
`Contract:` trailer: a strained contract often names its blast radius already.
