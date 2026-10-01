<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# LIBRARY_DOC_REVIEW-history.md — the timeline of [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md)

The protocol and the current watermarks are next door; this file keeps what each pass
found and how the table came to its shape.

## Watermark notes carried by LIBRARY_DOC_REVIEW.md until 2026-10-01

Each row's notes accumulated one clause per pass; the working table keeps only what holds
now.  The rows as they stood:

| library | reviewed through | at commit | notes |
|---|---|---|---|
| `default` | 2026-10 | `892ff4743` | @STD-001..012 authored across text / collections / JSON / files-IO; docs read while tagging. 2026-09: the only change since is two compiler-internal ops (`OpDistinctStore`, `OpRefAlias`), non-`pub` and undocumented by design — the reference chapter's promises are unchanged. 2026-09-04 re-read: loft#1348 removed every tracker tag from the published descriptions and rewrote the `min`/`max` section, `sum_of` and `File.format` for their readers; `Walkable`, `tree_walk`'s cap edge and the three interfaces documented; the `assert_eq` and `03_text` maintainer notes moved off the page. Each authored behaviour sentence was probed on both backends and holds 2026-10: every published section read against the release and each authored sentence probed on both backends (the docs pass; `.doc_review_ledger` 264/264); since then `len`/`size` of an absent text answer 0, `log10`/`log2` are exact at powers, `StackFrame.arguments` lists the parameters, and only `///` lines are published (loft#1808) — each probed; no `pub fn` line moved |
| `lib/git` | 2026-10 | `892ff4743` | @GIT-001..005 tagged to live uses in `scan.loft` + `refresh.loft`; 13 pub fns read while tagging. 2026-09: a query that cannot be ASKED now halts instead of answering `""` (loft#1061) and the doc above `git_query` and `branch` moved with the code 2026-10: one maintainer comment re-pointed from a design-decision id to the plan that records the decision; no `pub fn` moved |
| `lib/lexer.loft` | 2026-10 | `892ff4743` | @LEX-001 (matches/test/identifier), @LEX-002 (anchor/revert backtracking) — both tagged to live uses in `parser.loft` (`function`, `object`), exercised by the `16-parser` doc test; format-protocol/comment fns still owe examples (need a non-rendered demo). 2026-09: `Anchor.start` (a revert can land at a token's START), `split_token` (maximal munch undone for a nested `>>`) and `offset` (a stalled-loop guard) — each documented with its reason and with live callers in `lib/parser.loft`. 2026-09-04: `Lexer` itself documented (the token/keyword tables must be set before the first parse); no `pub fn` moved 2026-10: one keyed read discharges with `?? []` (no entry means no token starts there); no `pub fn` moved |
| `lib/parser.loft` | 2026-10 | `892ff4743` | First review. One `pub fn` (`parse`), doc read against the body and found current; @PAR-001 tags the doc test `tests/docs/16-parser.loft`, which is the clearest call site there is. Its prelude load names `default/01_code.lav`, an extension this repo has never had — filed as loft#1339, not fixed here because dropping it renumbers `cur_file`. 2026-09-04 after loft#1339: the phantom `.lav` prelude load is gone, with the comment saying why a real one would regress `parse`'s count; no `pub fn` moved 2026-10: the grammar grew `while`, `match` with guards, both lambda forms, `::` names and the optional `;` after a block (loft#1800); `parse`'s doc still states its contract, and `tests/docs/16-parser.loft` now asserts both what it parses and the listed constructs it does not |
| `lib/code.loft` | 2026-10 | `892ff4743` | First review. 24 `pub fn`, no doc comment on any of them and no module header; header written naming what `Code` is, what `cur_arg` switches, and which half is reached. `deferred` in `examples-exempt.tsv`: the emitter half has no call site to cite. Two defects it hides — `null_value` emits `Boolean`, `blocks` is popped but never pushed — filed as loft#1340. 2026-09-04 re-read after loft#1340: `add_block`/`add_loop`/`add_if` record the header they open, `end_if` spans are consistent, `null_value` emits `Null`; `Code` and `Structure` documented. Still `deferred` — the emitter has no caller to cite 2026-10: the `as Block` / `as If` / `as Loop` downcasts are now guarded by an `is` test and two keyed reads discharge with `?? 0` / `?? []`; the module header and type descriptions still describe it. Still `deferred` |
| `lib/testlib.loft` | 2026-10 | `892ff4743` | First review. `exempt` in `examples-exempt.tsv` — a fixture for `tests/docs/17-libraries.loft` and `tests/diagnostic_reach.rs`, deliberately trivial, so a call site teaches nothing its signature does not. Docs read; nothing stale. 2026-09-04: `Point` and `Bag` documented as the fixtures they are; no `pub fn` moved 2026-10: no source change beyond the join; nothing stale |
| `lib/audience_crystal` | 2026-10 | `892ff4743` | @ACR-001..003 tagged to the `01-editor-helpers` test (picking inverse, incr editor loop, erase) 2026-10: the `gridmesh` dependency moved to 0.2.3 with `use gridmesh::*`; one maintainer comment restated without its history; no `pub fn` moved |
| `lib/engine_host` | 2026-08 | `7786d28c` | @EHK-001..004 tagged to CI-spawned audience-demo kernels (run loop, broadcast, sync lanes, run_client drain); 37 pub fns read while tagging |

`7786d28c` is the commit that authored every in-tree worked-example tag (squash-merged
PR #971, 2026-08-18) — the 2026-08 pass *was* that tagging, so it is the pass's real
end ref rather than the `(bootstrap)` placeholder these rows carried first.

## The placeholder rows removed in 2026-08

Six placeholder rows had gone stale by 2026-08: `lib/html` and `lib/markdown` had moved
out to the `loft-libs-docs` repo, two more were spelled differently from the tree they
named, and two were prose standing in for a list — a four-library cell, and a catch-all
"registered libs (…)" row that named six of thirty-four.  That is why a row now means a
review happened, and the backlog is derived.

## Type descriptions — 2026-09 (loft#1342)

The 2026-09 pass found the published distribution documents its functions and not
its types: 34 `pub struct`/`pub enum` declarations carried no description, 33 of them
named in other public signatures of their own library (`Stage` in 111 of `stage`'s).
Every one now has a one-line description directly above its declaration, on the
`doc-types-2026-09` branch of each library repo that had one — `loft-libs-graphics`,
`-world`, `-net`, `-core`, `-assets` and `-docs` (`game` and `plugins` had none);
parse-checked as 19 packages, 0 errors. The count is taken over the REGISTRY's
published surface, so the `34 type` figure `make libraries-review` prints moves only
once those branches merge and the libraries republish: the branches are the fix, the
release is the owner's step.

## Older dated lines moved from LIBRARY_DOC_REVIEW.md on 2026-10-01

- **Why the examples gate only advises in a library repo.**  Before the gate was tiered it bit
  in both directions: `exindex` landed in loft on 2026-08-18 and reddened loft-libs-game's next
  PR for a file it never touched (last green run 2026-08-17), and switching a library
  checkout's branch turned loft's own run red with two dangling tags.
