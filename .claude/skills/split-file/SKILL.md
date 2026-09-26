---
name: split-file
description: >-
  Split one over-bar source file in the loft repo into files that each hold ONE
  subject, or carve the small subjects out of a file that is mostly one. Use it for the
  release checklist's M-file-split row (the pick from `make file-sizes ARGS=--pick`,
  twice a month on the release cadence), and when asked to "split", "break up" or
  "carve out" a file. File size is never limited per PR — this is release work. The result is always a PURE MOVE in its own
  PR: no signature, behaviour or comment changes. Not for refactoring — a long function
  found on the way is split only if the split is itself a move; otherwise it keeps or
  gains a `#[expect(clippy::too_many_lines, reason = …)]` and the finding becomes an
  issue.
user-invocable: true
---

# Split a file

The rules are the code-shape rows of CODE.md § Functions and DEVELOPMENT.md's gate table
(plan @PLN173, `doc/claude/plans/173-code-shape.md`).  This skill is the judgment part:
where the seams are.

## Start from the pick

```bash
make file-sizes ARGS="--pick 2"        # this release's files, with the seams the script can see
make file-sizes ARGS="--all"           # every file over the bar
```

`--pick` and the `M-file-split` row arrive with @PLN173 phases 2 and 4; until then take
the file from `--all` (or the owner's choice).

Take the top pick (the owner may name another).  **One file per PR.**

## Two modes

**split** — the whole file.  Output is `path/<name>/` (or sibling files) with one file per
subject, the original keeping the type definition(s), shared private helpers, and a module
header that lists the parts and what each holds.

**carve** — the file is mostly one subject with small ones beside it: move the small ones
out and stop.  Same rules, same pure-move PR.  A move never shares a squash-merge with a
behaviour change.

## Find the seams

1. List the fns and what each one touches: the `self` fields it reads, the types in its
   signature, the fns it calls.  `--pick` prints a first grouping; refine it.
2. A subject is a cluster that reads the same fields and calls each other more than it
   calls out.  Name it by what it does for a caller (`lookup`, `declare`, `lifetime`),
   never by when it was written or which plan added it.
3. Where a formal doc already names the parts (`formal/rewrites.md` for `hoist.rs`, the
   grammar for `parser/`), use those names — the docs and the tree should agree.
4. Seam guidance the owner has set is in this section; follow it before your own:
   <!-- owner: one line per file, e.g. "src/parser/: by grammar production" -->

Two or more clusters of comparable size → split.  One cluster is the file and the rest is
small → carve the small ones out and stop; a file that is one long subject is a long
chapter, not a defect.

## The move

- `impl X` blocks can be spread over files: each part gets its own `impl X { … }`.
  Helpers used by more than one part stay in the original with `pub(super)`; a helper used
  by one part moves with it.
- Every part starts with the copyright line and a one-paragraph header saying what the
  part is for (DOC_QUALITY: why to use it, not why it was written).  Add `mod` lines to
  the original.
- Change **nothing else**: not a signature, not a `pub`, not a comment, not a name.  A
  long function you pass is left as it is, or gains
  `#[expect(clippy::too_many_lines, reason = "…")]` with a reason a reader can check.  An
  `inherited` reason on a function you move must be replaced — split it (if that is a move
  too) or write the real reason.  Anything else you notice becomes an issue, not a diff.
- `rustfmt` runs; nothing is hand-formatted.

## Prove it

```bash
git diff -M --color-moved=dimmed-zebra --stat origin/main    # should read as moves
git diff -M origin/main | grep -cE '^[+-][^+-]'               # non-move lines: a handful (mod lines, headers) at most
make ci                                                       # green, both clippy legs
make file-sizes ARGS="--all"                                  # the source shrank, no part is over the bar
```

If the non-move count is more than the `mod` lines and part headers account for, something
changed that should not have; find it and revert it.

## The PR

Title `split <file> into N parts` or `carve <subject> out of <file>`; body is the parts and
one line each on what they hold.  Nothing else in the PR.  Tick `M-file-split` with the
PR link and the pick's commit.
