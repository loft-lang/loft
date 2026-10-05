<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library documentation review — the by-hand protocol

> Origin: [@PLN141](lib_plans/141-library-worked-examples/README.md) (worked
> examples). Run for **every release**, before the tag: it is part 1 of the release's
> documentation validation ([RELEASE.md § Documentation validation — required for EVERY
> release](RELEASE.md)): it runs in full halfway through the cycle (`M-doc-validation-mid`)
> and over what moved since then before the tag (`M-doc-validation`); both rows block the tag
> until they are done.  The automated `check_doc_drift.sh examples` gate additionally blocks dangling or
> duplicate citations inside loft, and only advises in a library repo (below).

## Why a by-hand pass exists

> ⚠ **The gate BLOCKS inside loft and ADVISES in a library repo**, and the asymmetry is
> deliberate. `examples` and `examples-index` are the only two checks that span
> repositories: a library's CI checks loft out as `loft-src` and runs *loft's* script
> against the library, so the rules arrive from whatever loft `main` happens to be. A
> blocking gate fails in both directions: a new rule in loft reddens a library's next PR
> for a file that PR never touched, and switching a *library checkout's branch* turns
> loft's own run red with dangling tags, a failure with no bad commit in either repo. A gate whose rules change
> under you, from a repo you do not control, lands its red on whoever opens the next PR.
>
> It follows this repo's own diagnostic rule: **a diagnostic gates if and only if ignoring
> it can produce a wrong result** ([CLAUDE.md](../../CLAUDE.md) § Two diagnostic tiers). A
> dangling doc citation is a broken link; it cannot. Inside loft — which owns the generator
> *and* the feature-doc citations, with no cross-repo coupling — it still gates, and the
> scanner's own selftests gate everywhere, because a scanner that stops following its
> documented rules is loft's bug whoever runs it.
>
> **Advisory does not mean quiet.** The findings go to the library PR's job summary in
> full — the same place a failing test writes its excerpt, so it is as visible as a red
> tick without being able to block a merge.
>
> **Advisory costs you a local pass/fail, so one command gives it back:** `make
> examples-preflight REPO=<library>` gates the citation faults CI reports and exits
> non-zero on them, without demanding the index, which is not committed.
> `EXAMPLES_GATE=hard` restores full blocking for a repo that wants it.
>
> ⚠⚠ **`examples-index.tsv` is not committed in a library repo at all — CI builds it.**
> The index is DERIVED and its generator is in loft, so a committed copy there can only
> rot: it cannot be regenerated where it sits, and "regenerate it" names a command that
> repo does not have. CI emits it per run (`check_doc_drift.sh emit-examples-index`),
> folds it into the job summary and uploads it as an artifact. **A derived file that is
> never committed cannot be stale** — that retires the failure mode rather than
> downgrading it. loft keeps its own committed copy: loft owns the generator, and a
> greppable offline index is what the agent development model runs on.


The automated gate (`scripts/check_doc_drift.sh examples`) catches the two failures a
machine can see: a worked-example tag that **dangles**
(cited, but no test carries it) or **duplicates** (one tag on two functions). It
cannot see the two failures that actually rot a library's docs over time:

- **Staleness** — a `///` doc comment that still resolves and still reads
  cleanly, but does not describe what the function *does now* (a parameter
  changed meaning, the return contract shifted, a behaviour note went out of
  date). The prose is internally consistent, so nothing flags it; only a human
  reading it against the current body notices.
- **Example quality** — a cited example that is valid and runs, but is not
  the *clearest* demonstration: a better real call site has since appeared in a
  consumer, or the tagged test drifted to exercise an edge case rather than the
  common path.

Both need judgment. This protocol is the monthly pass that supplies it, kept
cheap by a **watermark + changed-since worklist** so each month reviews what
actually moved, not all ~350 public functions.

## Cadence and scope

- **When:** every release, before tagging it. Libraries are not release-coupled for
  *publishing* (RELEASE.md § What forces a release), but their docs are reviewed with
  every release.
- **Who:** one reviewer per pass — a human, or an agent steered through the steps
  below. Splitting libraries across passes is fine; the watermark carries state.
- **What:** the whole distribution — the loft stdlib (`default/`), the in-tree
  libraries (`lib/*`), and every package in the registry. `make libraries-review`
  names the population and says which part of it this pass owes; you never pick
  the list by hand.
- **The other half of the pass:** the feature catalogue (`@F`/`@I`) is reviewed with
  the same release through `make features-review SINCE=<previous release tag>`, with
  `make features-check` as its pre-flight — the previous tag is its watermark. Same two
  questions, and the same requirement: both halves are part of `M-doc-validation-mid` and `M-doc-validation`, so
  run both and treat the union as one worklist.

## The pass — per library

### 0. Pre-flight (automated — must be green first)

```bash
scripts/check_doc_drift.sh examples     # no dangling / duplicate citations
make features-check                     # feature-catalogue shadow in sync (if touched)
make libcatalogue                       # the catalogue builds without breakage
```

`make libcatalogue` is a hard precondition for step 1, not just hygiene: the library
aid reads the snapshots it writes, and those are a **local build by design** (@PLN112 —
a committed copy silently lagged `origin/main`). Skip it and the "what moved" answer is
last month's, which is worse than no answer.

Green here is **necessary, not sufficient** — it means no cheap failure remains,
so the manual budget goes entirely to staleness and quality.

### 1. Generate the worklist

Two steps, coarse then fine. First, which libraries does this pass owe anything?

```bash
make libraries-review
```

It answers the only two questions a program can: what is **structurally missing** (a
library with no watermark row, so it has never been reviewed; one whose source cites no
worked example and carries no `examples-exempt.tsv` verdict; a watermark row naming a
library that does not exist), and which reviewed libraries have **moved** since the
commit their watermark records — with the commit count and how many `pub fn` lines
changed, so "three commits, zero signatures" can be read and dismissed in seconds. It
is a report: it never fails, and it never judges whether a doc is *good*.

Then, for each library the aid put on the list, the per-function worklist:

```bash
scripts/doc-review.sh --since <its-watermark-commit> <library-tree>
```

It prints three things: a **coverage** count (a health signal, not a target —
most functions are self-evident and correctly carry no example), the **citation
inventory** (every `// Example:` site), and the high-signal part — every public
function whose **signature changed** since the last review. A changed signature
is the number-one source of a stale doc.

### 2. Re-read the changed docs (staleness)

For each function on the changed-since list, read its `///` doc against its
current body: do the description, each parameter's meaning, the return contract,
and any inline example still match what the code does **now**? Doc edits are XS —
**fix drift on the spot**.

Then the README and the guide, the same way: **run every example in them**, and measure
every claim that names a target, a number or a limitation. The defect met most is an
example written as `use lib;` followed by a bare name — a bare `use` brings in only the
`lib::` qualifier (C98), so it does not compile; the cure is `use lib::*;`. The next is a
limitation the code has since outgrown: "does not build on `--native-wasm`", "is not
drawn", "import it in this order". A limitation is a claim — run it before keeping it.

### 3. Fill the highest-value coverage gap (examples)

From the *uncited* public functions, pick the ones a reader "knows exists but
cannot use from the signature alone" — the ratchet's rule (@PLN141 § Scope
discipline).  **This step is where @PLN141's tail lives now that the plan is
closed:** a package that lands after a rollout owes a verdict like any other, and
step 1's "owe a worked-example verdict" list is what surfaces it.  ⚠ That list is
built from a local snapshot, so a stale one omits exactly the newest packages —
the aid states its age and warns past two days; refresh with `make libcatalogue`
before trusting the worklist. Author a worked example for **one or two per pass** (a tagged test,
or a citation to a real consumer call site). The ratchet only goes up; there is
no sweep, and a function whose use is self-evident is left alone.

### 3a. Write a guide for a library that owes one

**This step is where @PLN149's tail lives now that the plan is closed.**  The four
documentation tiers, the guide contract and the rendering all shipped; what remains is
CONTENT, one library at a time — and it never finishes, because a package published next
month owes a guide too.  A plan that cannot reach a done state is a standing practice
wearing a plan's clothes, so it lives here instead.

Step 1's aid reports the queue rather than a document listing it:

```
  -- guides (Tier 1: docs/*.loft, run by the library's own CI on both backends) --
    16 have one · 25 owe one · 0 not measured
```

⚠ **Three states, and the third is the one that matters.**  The `guide` flag is recorded
by `scripts/refresh-unreleased.py`, whose cache reuses any entry whose sha has not moved —
so an entry written before the field existed carries no flag.  Counted as *no guide* it
invents a worklist; counted as *has one* it hides real gaps.  Neither is a measurement, so
"not measured" is its own count and names the refresh.  The flag backfills on the next
ordinary run (it is one directory listing, independent of the API extractor), so the third
state should be 0 in practice; a non-zero one means the snapshot has not been refreshed.

Write **one per pass**, and pick by who is reached: a library other packages depend on is
one more readers arrive at, and the aid's dependants column ranks that.  The contract is
[LIBRARY_AUTHORING.md § 2c](LIBRARY_AUTHORING.md) — five parts, `main` calls every section,
green on both backends with identical output.  Two rules that are easy to skip and are what
make a guide worth trusting:

- **Measure every asserted number before writing it.**  A guide is a running program, so a
  recalled value is a red run at best and a confidently wrong page at worst.
- **Falsify it.**  Invert one load-bearing assertion, confirm the file goes red, restore.
  A section whose asserts cannot fail reads as verified and is not — which is exactly the
  defect that made `main` calling every section a rule.

### 4. Spot-check example quality (freshness)

For a **rotating handful** of already-cited functions (the inventory from step
1), open the cited test: is it still the *clearest* demonstration of the common
path? Has a new consumer usage become a better example? Re-point or improve if
so — a worked example is only worth its tag while it teaches.

### 5. Record and route findings

Fix XS/S (doc edits, a clearer example) in the same pass. File M+ — a function
whose doc drift reveals an actual behaviour bug, or a gap that needs design — as
a GitHub issue per the [bug-filing policy](../../CLAUDE.md) (`Fixes #N`), never
inline. Don't scope-creep the review into unrelated fixes.

### 6. Bump the watermark

Update the library's row in the table below — `reviewed through` to this cycle, `at
commit` to the ref this pass ended on (`git rev-parse --short HEAD` in that library's
repo). A library reviewed for the first time gets a **new row**; that is what moves it
out of the aid's "never reviewed" list.

Leaving `at commit` empty is not free: next month the aid can only say *reviewed, but
no commit recorded — nothing to diff against*, and that library falls back to a full
re-read. The watermark is the entire mechanism that keeps a quiet month cheap.

## Watermarks

"Reviewed through" = the last monthly pass that read this library's docs; "at
commit" = the ref that pass ended on — the baseline `make libraries-review` diffs
against to say what has moved since.

**A row means a review happened.** The libraries that have *not* been reviewed are
not listed here — `make libraries-review` derives that backlog from the actual
population (the in-tree trees plus every package in the registry snapshot), so a
placeholder row would be a second list of the same fact, drifting the moment a
library is published or renamed.

The **key column is machine-read** — one library per row, spelled exactly as the aid
keys it: the path for an in-tree tree (`default`, `lib/git`, `lib/lexer.loft`), the
package name for a published one (`graphics`). A key that matches nothing is reported
as a STALE ROW rather than ignored.

| library | reviewed through | at commit | notes |
|---|---|---|---|
| `default` | 2026-10 | `a80e26737` | @STD-001..012 authored across text / collections / JSON / files-IO. Every published section read against the release and each authored sentence probed on both backends (`.doc_review_ledger` 264/264), including `len`/`size` of an absent text answering 0, `log10`/`log2` exact at powers, `StackFrame.arguments` listing the parameters, and only `///` lines published (loft#1808) |
| `lib/git` | 2026-10 | `892ff4743` | @GIT-001..005 tagged to live uses in `scan.loft` + `refresh.loft`; 13 pub fns read. A query that cannot be ASKED halts instead of answering `""` (loft#1061), and the doc above `git_query` and `branch` says so |
| `lib/lexer.loft` | 2026-10 | `892ff4743` | @LEX-001 (matches/test/identifier), @LEX-002 (anchor/revert backtracking) — both tagged to live uses in `parser.loft` (`function`, `object`), exercised by the `16-parser` doc test; format-protocol/comment fns still owe examples (need a non-rendered demo). `Lexer`, `Anchor.start`, `split_token` and `offset` documented with their reasons |
| `lib/parser.loft` | 2026-10 | `a80e26737` | One `pub fn` (`parse`); @PAR-001 tags the doc test `tests/docs/16-parser.loft`, the clearest call site there is. Its doc states the contract of the grammar it has — `while`, `match` with guards, both lambda forms, `::` names, the optional `;` after a block (loft#1800) — and the doc test asserts both what it parses and the listed constructs it does not |
| `lib/code.loft` | 2026-10 | `892ff4743` | 24 `pub fn`; the module header names what `Code` is, what `cur_arg` switches, and which half is reached; `Code` and `Structure` documented. `deferred` in `examples-exempt.tsv`: the emitter half has no call site to cite |
| `lib/testlib.loft` | 2026-10 | `892ff4743` | `exempt` in `examples-exempt.tsv` — a fixture for `tests/docs/17-libraries.loft` and `tests/diagnostic_reach.rs`, deliberately trivial, so a call site teaches nothing its signature does not. `Point` and `Bag` documented as the fixtures they are |
| `lib/docs.loft` | 2026-10 | `3cbc126fd` | `exempt` in `examples-exempt.tsv` — one unfinished function writing heading-only pages. Comments restated as rules instead of the loft#1339 story; its gating nullable warnings (and two in `lib/lexer.loft`) discharged |
| `lib/process` | 2026-10 | `b3aa068a5` | 12 `pub` items read against `src/process_run.rs` and probed: `Run.code`'s `128 + n` / `-1` hold; `hole_float` promised a "shortest" form that `1.0e20` does not get, and `args` now says a `-`-led word refuses the command even after `--`. `deferred` in `examples-exempt.tsv` until a @PLN179 port calls it |
| `lib/audience_crystal` | 2026-10 | `f29effbf3` | @ACR-001..003 tagged to the `01-editor-helpers` test (picking inverse, incr editor loop, erase) |
| `lib/engine_host` | 2026-08 | `7786d28c` | @EHK-001..004 tagged to CI-spawned audience-demo kernels (run loop, broadcast, sync lanes, run_client drain); 37 pub fns read while tagging |
| `arguments` | 2026-10 | `06aa45e` | Every doc probed on both backends; three untrue promises fixed in code (0.2.4): a value option takes the next word whatever it looks like, `-h`/`-V` go to the program's own option, a short-only option is queried by its short name |
| `cbor` | 2026-10 | `06aa45e` | "Never crashes" was false on deep nesting; `decode` refuses past `MAX_DEPTH` (0.1.9). Header no longer promises phases that shipped or a `to_cbor` that does not exist |
| `crypto` | 2026-10 | `06aa45e` | Malformed base64 decoded to other bytes at every door; strict decode, each door refuses (0.3.11). The `+=` CAUTION on `base64_to_bytes` was stale and is gone. Two `pub` TEST-ONLY HPKE fns remain a surface question |
| `random` | 2026-10 | `06aa45e` | `RandStream` described; `rand_indices`' negative case named (0.3.4) |
| `regex` | 2026-10 | `06aa45e` | `regex_find`'s supersession named the wrong `search`; roadmap's FFI-gap cause was stale (0.4.2) |
| `zttext` | 2026-10 | `06aa45e` | Guide written and falsified; `zttext_version()` answers the manifest; design pointers name where the notation lives (0.1.3) |
| `assets` | 2026-10 | `efd8ed7` | Guide and @PAK-001..003 written; README example did not compile under C98 and the v2026.8.0 hang note was obsolete; `blob_put`'s `+=` claim was stale (0.2.4). Found loft's text-key replace duplicating (fixed in loft) |
| `glb` | 2026-10 | `63e6b03` | Guide written; README claimed camera nodes that are never written (0.1.4). A spot light now carries the `spot` object KHR_lights_punctual requires and its node's translation (0.1.5) |
| `mesh3d` | 2026-10 | `efd8ed7` | Guide written; `mat4_scale` is per-axis, not uniform; the bare-import claim predated C98 (0.1.3) |
| `drawing` | 2026-10 | `1868d3c` | Guide and @DRW-001..003 written; the README's example did not compile under C98, and its rasteriser import order, `--native-wasm` and "Fronds / gradients / checks not drawn" claims were stale. 0.4.0's Brush/Lock rows read against the parser (`flip` defaults to 0); the corpus re-measured with the `Lock` brush: all 36 of the crawler's scenes byte-identical on both backends (0.4.0) |
| `graphics` | 2026-10 | `98b270f` | README listed `mesh3d` and `glb` as sub-modules it does not pass on; every listed function exists, the wasm PNG and blend claims measured (0.9.5) |
| `gridmesh` | 2026-10 | `98b270f` | Guide written; its read-only functions take `const`, so a rule holding the field `const` can call `idx_at`; plan history out of the comments (0.2.4). Found loft's false avoidable-copy advice on a field or parameter copy (fixed in loft) |
| `imaging` | 2026-10 | `98b270f` | README only: `--native-wasm` builds and decodes as the interpreter does; @IMG-002 described as RGBA with alpha carried (0.4.2) |
| `shapes` | 2026-10 | `98b270f` | Guide written; a depth is the distance out, not the part inside (0.5.2) |
| `stage` | 2026-10 | `98b270f` | README read against the source: all 100 listed names exist, the test-backed claims each have their test; guide linked (0.18.6) |
| `text2d` | 2026-10 | `98b270f` | Guide and @T2D-001..003 written; `write_text("")` answered -1 and a scale below 1 measured unlike it drew (0.4.3) |
| `tween` | 2026-10 | `98b270f` | Guide and @TWN-001..003 written; the README's fixstep example did not compile under C98 (0.1.2). Found loft's misleading bare-variant import advice (fixed in loft) |
| `fixstep` | 2026-10 | `658f0a0` | Guide written: a frame clock banking its remainder, a rate through a `Bank`, a cooldown firing once. The `--native-wasm` row was "not yet exercised"; measured, it answers as the interpreter does (0.1.3) |
| `input` | 2026-10 | `658f0a0` | Guide written, driven headless through `input_tick_from_state`; the repo README listed `input` as "(planned)" (0.2.4) |
| `time` | 2026-10 | `b24a332` | Read at 0.4.0, the @PLN182 P5 release (operator definitions; loft-libs-game#16). Every README claim measured on both backends; its bare `use time;` example did not compile — the C98 qualifier and, under it, no operator of the type (D-opr-1, fixed in loft); the README imports the names. Floored on the dated daily `>=2026.10.20261002` |
| `game_protocol` | 2026-10 | `c3d22a8` | Guide written (JSON on the wire with `:j` and `GameEnvelope.parse`); the README promised framing and ack/retransmit the package does not have — it is message types and constructors (0.1.5) |
| `ssh` | 2026-10 | `c3d22a8` | Guide written, runs with no sshd; the README's loop bounded `recv` bytes by `len` — the mistake @SSH-002 warns against (0.1.3) |
| `server` | 2026-10 | `fdcd652` | Read at 0.7.3 (`next` is an `operator next`; loft-libs-net#29). Its three `use server;` examples iterated `for req in srv`, unreachable through a bare `use` (D-opr-1, fixed in loft); they import the names, and all five README blocks compile warning-clean. Floored on `>=2026.10.20261002` |
| `web` | 2026-10 | `c3d22a8` | `byte_at`'s argument order was reversed in the README; the browser `fetch()` backend described as shipped, with what the browser does differently; `pack_u32_le` keeps the low 32 bits of a 64-bit `integer` (0.4.3) |
| `hex_body` | 2026-10 | `c9ddda6` | Guide written; history in README and source restated (0.3.3) |
| `hex_draw` | 2026-10 | `c9ddda6` | Guide written; `place_opening` replaces the wall's material — its comment said the material was kept (0.1.2) |
| `hex_edge` | 2026-10 | `c9ddda6` | Guide written; a material does not open a gate — `passable` reads only the surface mark; `SURF_NONE` slot is i32, not u16 (0.2.2) |
| `hex_field` | 2026-10 | `c9ddda6` | Status said stencils, the document format and the edge layer were "landing next"; all ship. `tests/08-hex-grid-parity.loft` keeps the restated lattice equal to `hex_grid` (0.1.4) |
| `hex_fit` | 2026-10 | `c9ddda6` | Guide written; comments cited a `tests/fit.loft` the package lacks (0.1.3) |
| `hex_form` | 2026-10 | `c9ddda6` | README and USAGE cited hexbody's SPEC / ROUNDTRIP.md, unreachable from the package (0.1.6) |
| `hex_grid` | 2026-10 | `c9ddda6` | Five packages build on it, not "four"; `hex_field` and `hex_world` do not (0.1.3) |
| `hex_place` | 2026-10 | `c9ddda6` | Guide written; USAGE omitted the seat_* family and five more functions (0.1.2) |
| `hex_recover` | 2026-10 | `c9ddda6` | Guide written; comments cited a `tests/trip.loft` the package lacks (0.1.4) |
| `hex_roof` | 2026-10 | `c9ddda6` | Guide written; `roof_match` answering `ROOF_UNKNOWN` dropped the closest ridge's far end — fixed in code and guarded (0.1.6) |
| `hex_shape` | 2026-10 | `c9ddda6` | Guide written; the wall-direction error is 1.1021°, the header said 4.11° (0.1.4) |
| `hex_terrain` | 2026-10 | `c9ddda6` | Guide written; the README example was refused under C98 (0.1.5) |
| `hex_way` | 2026-10 | `c9ddda6` | Guide written; the README listed six of seven examples and half the surface (0.1.3) |
| `hex_world` | 2026-10 | `c9ddda6` | Guide written; the README named two consumers that do not use it. Ships `src/overland.loft` and `src/wall.loft`, which fail `--check` and nothing reaches — left for the owner to remove (0.2.2) |
| `pluginabi` | 2026-10 | `b4dcdb4` | Guide written (a counter plugin and its host over frames); both README examples were refused under C98, the loft#1491 leak note was obsolete (clean under `LOFT_STORES=warn`), and the header's example spelled the error code `unknown_op` (0.1.5) |
| `html` | 2026-10 | `13205f6` | @HTM-001..002 written: escaping is one pass but not idempotent (escape once, at the output), and the apostrophe keeps a single-quoted attribute closed. "`&` is escaped first" described a replace chain the function is not (0.1.2) |
| `markdown` | 2026-10 | `13205f6` | @MKD-001..003 written: a non-empty `base_dir` routes relative links through `/file/` (undocumented until now), a heading's text is raw with `render`'s slug, `html_escape` leaves `'`. Only `@P<n>` / `@PLAN<n>` mentions are linked (0.2.4) |

Each pass's notes, row by row, are in [LIBRARY_DOC_REVIEW-history.md](LIBRARY_DOC_REVIEW-history.md).

### Type descriptions owed a release

Every `pub struct`/`pub enum` in the published distribution has a one-line description
directly above its declaration on the `doc-types-2026-09` branch of each library repo
that had one — `loft-libs-graphics`, `-world`, `-net`, `-core`, `-assets` and `-docs`
(loft#1342).  `make libraries-review` counts over the REGISTRY's published surface, so
its `type` figure moves only once those branches merge and the libraries republish:
the branches are the fix, the release is the owner's step.

## What this is NOT

- **Not a gate on its own aids.** `make libraries-review` and `make features-review` are
  reports that never fail; what blocks the release is the checklist row
  `M-doc-validation-mid` and `M-doc-validation`, ticked once the reads they list are done.
- **Not a full re-sweep.** The watermark + changed-since worklist bound each
  pass to what moved. A month with no library changes is a five-minute pass.
- **Not a coverage mandate.** A low citation count is healthy when the uncited
  functions are self-evident. The target is "every *non-obvious* function has a
  *current, clear* example", never "every function has one".

## See also

- [@PLN141](lib_plans/141-library-worked-examples/README.md) — the worked-example
  mechanism (tag family, `check_doc_drift.sh examples`, `idx` ingestion).
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the docs themselves should read.
- [RELEASE.md](RELEASE.md) — the monthly cadence this pass rides.
- `scripts/doc-review.sh` — the per-function worklist generator invoked in step 1.
- `make libraries-review` — the per-library worklist that picks what step 1 drills into
  (`scripts/check_doc_drift.sh libraries-progress`); `make features-review` is its
  feature-catalogue twin.
