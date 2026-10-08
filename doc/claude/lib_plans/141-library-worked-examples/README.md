<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN141 — Worked examples for the current libraries

> Tracker: [loft-lang/plans#141](https://github.com/loft-lang/plans/issues/141)
> (`subject:libs`, `status:finished`). Origin: dryopea's
> `docs/EXAMPLES.md` gate (commits `9cf8a01` + `1f4c35f`, 2026-08-17) — this idea
> rolled out across the loft library + feature ecosystem. Built on branch
> `mac-install-dylib-fix` (alongside @PLN142).

## Status

**CLOSED — SHIPPED 2026-08-25.** Every phase is done: **A** probe · **B** acronym
registry · **C** shared gate + indexer ingestion (both follow-ups settled) · **C2** the
feature catalogue, **116 of 116 cited** · **D** the per-library rollout · **E**
`loft-libs-world`'s twelve · **(last)** the convention doc + CI ratchet.

**297 worked-example tags across the ecosystem**, and the convention is wired into every
library's CI. The reader's guide is
[LIBRARY_AUTHORING.md § 2](../../LIBRARY_AUTHORING.md); the standing home for the tail is
the monthly pass in [LIBRARY_DOC_REVIEW.md](../../LIBRARY_DOC_REVIEW.md).

### Why it closes with work still visible

Two things were outstanding, and neither is this plan's to finish:

- **Three PRs are unmerged** — [`game#10`](https://github.com/loft-lang/loft-libs-game/pull/10),
  [`plugins#3`](https://github.com/loft-lang/loft-libs-plugins/pull/3),
  [`assets#5`](https://github.com/loft-lang/loft-libs-assets/pull/5). All three are green
  and all three read READY under `make examples-progress`. That is **landing, not
  building**, and a plan does not gate on a merge clock.
- **New packages owe verdicts** — `assets`, `drawing`, `text2d`, `tween` in the
  distributed repos, plus `html`, `markdown` and three in-tree files. These landed AFTER
  the rollout, which is **the ratchet working, not the rollout being incomplete**: a repo
  is not done once, it is done per package, and packages keep arriving. A plan that stays
  open until no package ever owes a verdict can never close.

**The tail was handed over only after checking the receiver actually fires.** `make
libraries-review` now names all nine, and [LIBRARY_DOC_REVIEW.md](../../LIBRARY_DOC_REVIEW.md)
step 3 is the standing instruction to clear one or two per pass.

⚠ **That check nearly passed for the wrong reason, and the fix is the plan's last
finding.** The review aid reads a locally-built snapshot for its package list, and the
one on this machine was five days old — so it omitted every package that had landed
since, which is precisely the set most likely to owe a review, because a new package has
never had one. It said nothing about that: the report above the omission was confident
and complete-looking. Handing the tail to it in that state would have dropped four
verdicts on the floor. The aid now **states its worklist's age** and warns past two days
(`scripts/check_doc_drift.sh`), so the next reader is told what it did not look at.

### Historical — how the first distributed library landed

> Kept because the sequencing findings below are reusable; superseded as a STATUS report
> by the table above. `arguments` was the first distributed library and its PR has long
> since merged, so read this for the method, not for where the rollout stands.

Stdlib + in-tree libraries done; the distributed-library **gate is now
wired into every library's CI** (Phase-last CI ratchet, see below); the **convention
page is done** (LIBRARY_AUTHORING.md § 2a). First distributed library **`arguments`
DONE** — pushed as `loft-libs-core` branch `mac-worked-examples` (rebased onto
`origin/main`, PR not opened). Four `@ARG-001..004` worked examples (`parse`
declare→parse→query lifecycle, `optional` glued-only value, `enable_help` opt-in +
required-bypass, `required` parse-failure error idiom): def tests in
`arguments/tests/02-worked-examples.loft`, the four `// Example:` citations in
`arguments/src/arguments.loft`, and the generated repo-root `examples-index.tsv`.
Both per-library validations pass: green in library mode
(`EXAMPLES_REPO_ROOT=../loft-libs-core EXAMPLES_CITE_ROOTS="arguments/src
arguments/tests"` — the same two env values `library-ci-reusable.yml` sets), and
deleting the demonstrator file makes all four citations `dangling`. `loft test` and
`loft test --native` are both 17/17. The other five packages in that monorepo
(`cbor`, `crypto`, `random`, `regex`, `zttext`) run the same CI step **vacuously
green** — the opt-in ratchet working as designed, so one library adopting the
convention cannot redden its neighbours.

**⚠ THE RATCHET ARMED ITSELF, AND TWO MERGED LIBRARY `main`s WENT RED — 2026-08-18.**
loft **#971 merged the gate to `main` at 13:39**, and `library-ci-reusable.yml` builds
loft from `loft-ref` (**default `main`**) and runs `check_doc_drift.sh examples` +
`examples-index` as **gating** steps on `push: [main]` **and on `pull_request`**. Every
repo that adopted the convention BEFORE the gate was live had therefore never been
validated by it. Two were wrong, and neither is a bug in the work — both are the same
sequencing hazard, now measured rather than predicted:

| repo | gate | fault | cause |
|---|---|---|---|
| `loft-libs-game` @ `main` | `examples` | 13 × `unregistered: @FIX-0NN` | `fixstep` shipped 13 tags + 16 citations; `FIX` is not in `main`'s `scripts/example_repos.tsv` |
| `loft-libs-game` @ `main` | `examples-index` | `missing: examples-index.tsv` | the repo defines tags and has no index file |
| `loft-libs-graphics` @ `main` | `examples-index` | `stale` — all five `@GFX-00N` rows off by **+1 line** | PR #26 committed the test file and the index it was generated from in ONE commit, with a line added to `worked-examples.loft` in between |

Both verified against pristine `origin/main` trees (`git archive` into a scratch dir,
gate run from a detached `origin/main` worktree of loft), and both fixes verified to
turn the gate green: the six registry rows take `loft-libs-game` from `DRIFT — 13
action items` to `ok — 13 citation(s) resolve`, and `write-examples-index` takes
`loft-libs-graphics` from `stale` to `ok — 17 tag(s), all current`. `loft-libs-core`,
`loft-libs-net` and loft's own `main` are green; `loft-libs-plugins` and
`loft-libs-assets` are green on `main` (their tags are still on the unmerged branch).

**And the gate itself was wrong in library CI — found by opening the PRs.** Every
library's `examples-index` step failed, whatever the repo contained. `library-ci-
reusable.yml` checks loft out INSIDE the workspace (`path: loft-src`) and points
`EXAMPLES_REPO_ROOT` at that same workspace, so `examples_defs_in_tree` walked loft's
own tree and indexed `@STD`/`@GIT`/`@LEX`/`@ACR`/`@EHK` as the library's — the
regenerated index carried rows like `loft-src/tests/scripts/945-…`, which no library
could ever commit, so the committed copy read `stale` **forever**. Invisible in this
repo, because loft scanning itself has nothing to exclude; invisible on the `examples`
step, which iterates CITATIONS from the package-scoped `EXAMPLES_CITE_ROOTS` and never
walks the tree. The fix is a path exclusion — *the gate's own checkout is never part of
the repo under test* — filtered by PATH rather than by the name `loft-src`, so it holds
wherever the checkout is nested. Reproduced in the real CI shape (a workspace holding
the library plus a `loft-src/` running the gate) before and after, and pinned as the
scanner self-test's **fifth rule**, an ABSENCE: remove the filter and the self-test
fails naming the contaminant row. That the self-test needed a fifth rule at all is the
same lesson a third time — **a repo-agnostic gate has to be exercised in the shape it
runs in, not only in the repo that hosts it.**

**The merge order is now a HARD constraint, not a preference.** Because the gate also
fires on `pull_request`, a library PR carrying tags for an unregistered acronym is red
**in its own PR** and cannot merge. So:

1. **loft first** — the six rows (`FIX` `INP` `TIM` `GLB` `MSH` `PAB`) land on loft
   `main`. Nothing else can go green until they do.
2. `loft-libs-graphics` — regenerate `examples-index.tsv` (independent of 1; `GFX` is
   already registered).
3. `loft-libs-game` / `-plugins` / `-assets` — the `worked-examples` PRs. game's also
   supplies the `examples-index.tsv` its `main` is missing.

**What this says about the mechanism.** The opt-in ratchet is what let the rollout
proceed one package at a time, and it worked exactly as designed — a library with no
citations stays vacuously green. What it does NOT cover is a library that opted in
while the gate was still on an unmerged branch: for that window the ratchet was a
promise, not a check, and the promise was kept by hand. The generalisable rule is in
LIBRARY_AUTHORING § 2a — *register the acronym in the same pass that writes the first
tag; if your CI is green and you never touched the registry, that is not evidence* —
and the second half of it is now earned too: **re-run the gate against every already-
adopted repo the day it goes live**, because that is the only moment the promises are
converted into checks.


**`loft-libs-graphics` is MERGED AND PUBLISHED — the convention's second complete
repo, and the first one to reach consumers.** `shapes` (`@SHP-001..003`), `imaging`
(`@IMG-001..004`) and `graphics` (`@GFX-001..005`) joined `gridmesh`; PRs #26 + #27
merged, and **graphics 0.5.3 / gridmesh 0.2.1 / imaging 0.2.2 / shapes 0.4.1** are
signed into the registry (commit `ca7bb7f`, index + `.sig` in one commit, trust gate
OK, 0 findings across 36 packages). Verified end to end rather than assumed: all four
install pinned from a clean project, and a consumer program built on the published
`imaging 0.2.2` decodes a 2×2 RGBA fixture to four correct pixels on BOTH backends
(pre-fix: five, channels shifted a byte).

**Publishing is what makes a citation reach anyone.** A `// Example:` line travels to
consumers through `loft api` / `.loft/api/<name>.api`, which are generated from the
published source's doc comments — confirmed by installing into a fresh project and
reading the citations back out of its stubs. The tarball carries `tests/` too, so the
demonstrator itself lands in `~/.loft/registry/<pkg>-<ver>/` beside the code that
cites it. Until a release goes out, the convention is a private note.

**Three gates, not one — each caught something the previous one could not.** This is
the operational lesson of the release, and it cost two extra CI cycles to learn:

| gate | what runs | what it caught |
|---|---|---|
| `loft test` (+ `--native`) | the INSTALLED loft | the ordinary suite |
| library CI | `LOFT_DENY_WARNINGS=1 loft test` | `pct = 0;` before a file-handle block — a `dead-assignment` **warning**, invisible without the flag |
| `registry_maintain.sh` | `<loft-checkout>/target/release/loft --interpret --tests tests` | a local named `now` (a stdlib fn) — *"Cannot redefine function 'now' as a variable"* |

The last two rows both report version `2026.8.0` and disagree on the same program.
The publish gate is the right authority — a published library must parse under
whatever loft its CONSUMERS hold, not the one on the publishing machine — so its
command is now on the LIBRARY_AUTHORING § 3 pre-release checklist, to be run BEFORE
tagging rather than discovered after the release PR merges.

**`loft-libs-net` and `loft-libs-core` are MERGED AND RELEASED** (net PRs #18–#20;
core PRs #29–#31, releasing arguments 0.2.1 / cbor 0.1.4 / crypto 0.3.8 / random
0.3.1 / regex 0.3.0 / zttext 0.1.1). Both carried a follow-up commit worth naming,
because it is a trap any repo in this rollout walks into: **adding a `description`
to `loft.toml` makes `loft publish` authoritative and OVERWRITES the registry's
existing catalogue line.** Three of net's descriptions were replaced by shorter ones
written during the pass — the publish did not fail, and it was caught only by
reading the publish diff. The old text knew things the new did not (which crate
backs the library, `ssh`'s **"Native-only"**), so the fix was to merge the two, not
revert. The general rule: a package with no `description` field has its catalogue
text living only in the index, and the first `loft.toml` to declare one wins.

**`loft-libs-core` was the third complete repo, and the one where the
rollout paid in CI health rather than in bugs.** `@RND-001..003` (random),
`@RGX-001..004` (regex), `@CBR-001..004` (cbor) and `@ZTX-001..004` (zttext) joined
`arguments` and `crypto`: 6 tagged, 0 todo on `mac-worked-examples`.

**This repo's `main` was already RED when the rollout reached it, in two packages,
and neither red was about examples.** `random` had `for i in 0..200` with `i` never
read — a `never-read` WARNING, and library CI runs `LOFT_DENY_WARNINGS=1`. `regex`
was red because loft grew `shadowed-by-method` (loft#940) on 2026-08-16 and it fires
on exactly the two functions that library had ALREADY deprecated for that reason:
`find` / `split`, kept as `#superseded` spellings because deleting them is an API
break. Both fixed here — the second by ASSERTING the warning with `@EXPECT_WARNING`
rather than suppressing it, so the annotation goes red and asks to be removed if the
shadow ever disappears. Worth naming the shape: a new diagnostic can turn a
correctly-written published library red with no green path except an API break, and
the assert-don't-suppress move is the one that keeps the information while restoring
the gate.

`cbor` carried a third kind of debt the pass surfaced: a tracked `.allow_warnings`
opting the whole package out of the warning gate, for a `v[i]` lint that no longer
fires. Removed — it was silently covering every FUTURE warning too. It also had no
README and no `[package] description` (the registry's catalogue line), and neither
did `zttext`; both written.

What the four new packages owe examples for is worth recording as a pattern, because
it is the same one three times: **the library offers two doors and nothing in the
types says which one a call site needs.** `random` has a shared global generator and
an owned stream (@RND-001); `regex` has a stdlib method spelling and a library
function that a bare call cannot reach — `find("[0-9]+", "abc123")` compiles and
answers null where `"abc123".search("[0-9]+")` answers 3 (@RGX-001); `cbor` has
`CBytes` and `CText` for the same payload bytes, which is a different record on the
wire (@CBR-002). `zttext` is the fourth shape, the gridmesh one: an ENGINE, where
what needs demonstrating is the shape of a correct call — an edit is a VALUE you
have to keep, and `insert_text(d, 0, "x", 0);` as a statement compiles, runs and
throws the document away (@ZTX-001).

**`loft-libs-game` is READY TO PR — the fourth complete repo, and the first whose
first package adopted the convention BEFORE the rollout arrived.** `fixstep` was
written with `@FIX-001..013` from the start (a door-per-test file, `01-doors.loft`);
this pass added `@TIM-001..005` (`time`) and `@INP-001..004` (`input`) on branch
`worked-examples`, reaching 3 tagged / 0 todo.

**And that early adoption is exactly what surfaced a hole in the mechanism: an
acronym can carry tags in a library repo while being absent from
`scripts/example_repos.tsv`, and nothing anywhere says so.** `FIX` was
unregistered; the gate reports `unregistered` — one of its three hard-drift faults —
so pointed at that repo it failed 13 times. It has not failed in CI only because
**this whole plan is still on branch `mac-install-dylib-fix` and not on loft's
`main`**, and the library-CI examples step comes from `library-ci-reusable.yml@main`.
That is a merge-ordering constraint, not a detail: the moment this branch lands,
every acronym already carrying tags in an adopting repo must be in the registry or
that repo's CI goes red on a file nobody touched. `FIX`, `INP` and `TIM` are
registered here.

**`input` shipped a ⚠️ PARKED banner to consumers for two months.** Its source and
its test both opened by declaring the library blocked on loft#248 and gated out of
the suite by `LIB_PKGS_SKIP`. Both blockers (#248 and #266) were fixed and the skip
list emptied on 2026-06-04 — and `input 0.2.0` was published after that, tarball and
all, telling every reader the library does not run. The convention found it the
ordinary way: writing an example means running the suite, and the suite passed.
Replaced with what is true plus the one real limit (`input_tick` polls a live GL
context and is the only function CI cannot reach).

**A committed `loft.lock` made the local gate disagree with the one that decides.**
`input/loft.lock` pinned `graphics 0.4.2`, which feeds a nullable into `rgba`'s
non-null parameter — a WARNING, so `LOFT_DENY_WARNINGS=1` failed locally on both its
test files while CI was green throughout, because CI resolves fresh and got 0.5.5
where that line reads `?? 0`. The lock did not buy reproducibility, it bought a local
red that CI hides — the mirror image of the usual failure, and worse, because a
developer cannot pass a gate the merge button never runs. `-core`, `-net` and
`-graphics` all already ignore `*/loft.lock`; this repo is the fourth to align, and
`*/native-auto/` went with it.

What the two new packages owe examples for is the ecosystem's most common shape in
its purest form: **the type is a bare `integer` or a bare `text` standing for a
convention.** In `time` a time, a span, a bucket key and a day count are all
`integer`, so `deadline - 3` moves a reminder three MILLISECONDS while reading like
three days @TIM-001; `parse`'s null means *not shaped like a date* and never *not a
real date*, so `"2026-13-45"` answers 2027-02-14 past the documented `!result` check
while `as DateTime` turns prose into the epoch @TIM-002; `days_between` counts
MIDNIGHTS CROSSED, so a 26-hour span answers 2 where a 46-hour span answers 1
@TIM-003; `to_local` shifts the INSTANT and `local_day` answers a bucket KEY that
formats with a `Z` it does not mean @TIM-004; and an ISO week number is meaningless
beside `year()`, which invents `2021-W53` and splits a real week in half @TIM-005. In
`input` a name is a bare `text` that is never declared, so a misspelt action reads
`false` — what an unpressed one reads — and a misspelt axis reads `0.0`, which is
also what a centred axis and two cancelling keys read @INP-002; a button argument is
a mask tested `& != 0`, so `MB_LEFT|MB_RIGHT` means *either* and its `just_pressed`
MISSES the right button pressed while the left is held @INP-003; the TICK consumes an
edge rather than the query, so two ticks in one frame hand the press to nobody
@INP-001; and a rebind inherits the key tables, which is what stops a held key being
stranded and also fires an edge the player never made @INP-004.

**Four more doc claims wrong in the very functions being documented** — stale-doc-
per-library is now five for five. `parse` and `combine` both said "null on malformed
input" when they reject a mis-shaped string and never an impossible date;
`days_between`'s "whole calendar days" reads as elapsed time when it counts
boundaries; `seconds_between` had no doc at all and truncates toward zero.
`input/README.md` was a nine-line stub and is written; `time`'s README carried the
same `parse` claim.

**`loft-libs-plugins` is READY TO PR — the fifth complete repo, and the smallest:
one package, four tags.** `pluginabi` (`@PAB-001..004`) owes them because **a frame
is `vector<u8>` and every payload is `text`**, and the signatures never say that the
text must be BASE64. `request(OP_APPLY_OP, "hello", "")` is not an error — it is
silent truncation to `"hell"`, because that prefix is one whole base64 quantum and
the trailing `o` is an incomplete one, dropped; five characters reach the plugin as
three bytes, the frame decodes, and `check_request` admits it @PAB-001. Only
`reply_is_ok` classifies a reply: `reply_out_b64` answers `""` for a failure AND for
a success carrying an empty payload, `reply_err_code` answers `""` for a success, so
neither field is a verdict in either direction @PAB-002. `check_request` validates
the ENVELOPE and `""` is its PASS — it returns the code to reply *with*, so it reads
backwards from every boolean guard; a known op carrying bytes no plugin could load
passes, an `arg` sent for an operation that reads none passes, and a REPLY frame
handed to it comes back `unknown-op` because a reply decodes perfectly and simply
has no `op` @PAB-003. That last one is recorded as the front door's honest limit
rather than changed: the error codes are a published closed set, and a host chasing
version skew when a frame is going the wrong way is a documentation problem, not an
excuse to break the vocabulary. @PAB-004 TAGS the existing whole-plugin test instead
of writing a second — what a protocol library owes a reader is not a signature but
the shape of a whole exchange, and that test already is one.

**`loft-libs-assets` is READY TO PR — the sixth complete repo, and the one where a
single defect showed itself differently through two doors.** `mesh3d`
(`@MSH-001..005`) and `glb` (`@GLB-001..004`), 2 tagged / 0 todo.

The pairing is the finding worth keeping. An out-of-range triangle index — the
easiest mistake to make in an index-addressed mesh builder — reaches `mesh3d`'s
`mesh_to_floats`, which SKIPS the missing vertex, so the buffer comes out **30
floats where 36 were meant**: short by one *vertex*, not one triangle, so it no
longer divides into triangles and every float after the gap is read as part of the
wrong one @MSH-001. The same mesh through `glb`'s `save_glb` is copied VERBATIM: a
byte-perfect GLB container carrying index 5 against an accessor that declares three
vertices @GLB-003. One defect, two exports, two unrelated-looking symptoms, and
neither door says a word — which is why both examples end by handing the reader the
same three-line validity check.

The rest of `mesh3d` is the bare-type pattern again, at its purest: a stride that
travels to the consumer by hand (6 vs 8, both `vector<single>`, and the buffer is
sized by TRIANGLES so a cube's 24 stored vertices become 36 emitted ones) @MSH-002;
a face whose direction is claimed TWICE, once by the stored vertex normal and once
by the winding of `add_quad`, with nothing checking they agree — `plane` reverses
its corners on purpose, and the natural order gives a geometric normal of -y beside
a stored +y @MSH-003; `mat4_mul(A, B)` applying B first, so the same rotate and
translate in the two orders land at (0,0,11) and (10,0,1) @MSH-004; and every
degenerate input answering ZERO rather than an error, so `mat4_look_at(eye, eye,
up)` builds a matrix that maps every point to the origin @MSH-005. `glb`'s own
shape is that **its entire public surface is two functions that return nothing** —
so the file is the only evidence @GLB-001, `save_glb` drops every non-geometry part
of a scene @GLB-002, and an empty mesh writes a valid container around a glTF whose
`"count":0` accessors the format forbids @GLB-004.

**Repo hygiene, twice more.** `glb/tests/light_glb.loft` wrote two `.glb` files and
never deleted them, unlike every other test in that repo — so the suite left
artifacts in the working tree on every run, and both were sitting untracked in the
checkout when the rollout arrived. Fixed, with `*/tests/*.glb` ignored for an
aborted run. Both `loft-libs-game` and `loft-libs-assets` also gained the
`.loft/` + `*/native-auto/` + `*/loft.lock` ignores the other four repos already
had — the rollout keeps finding this because it is the first pass that runs every
gate in a checkout somebody else set up.

**`loft-libs-world` is DONE — the seventh and last repo, and the hold was wrong
about one thing.** It had been held because `hex_edge/README.md` was modified and
unstaged: a consumer's finding from moros#10 (a caller must not come to rest exactly
at the fraction `sweep_path` returns, because that point is on the bisector and the
next `hex_at` may round to the far side, putting the character through the wall),
ending with an open design question for the owner. The hold read that as work in
progress. **Its mtime was 2026-07-28 — three weeks old.** Not work in progress:
work STRANDED, one careless `git checkout` from gone, and the correct handling was
to commit it unchanged as its own commit rather than to keep tiptoeing around it.
The design question stays open and is recorded as what blocks `hex_edge`'s verdict
row. Generalised: *"would anyone lose anything"* is the right question, and the
answer for an untracked file that has not moved in three weeks is **yes — by
leaving it there**.

**What the last repo added to the convention.** Fourteen packages, ~478 public
functions, all geometry — the tier where a call site teaches most. Tagged
`hex_grid` (`@HXG-001..006`, the tier-1 package every other one is expressed in)
and `hex_world` (`@HXW-001..005`); the other twelve are recorded **`deferred`**,
never `exempt`, each row naming what its example would teach and what unblocks it.
Two carry real ordering constraints (`hex_fit` follows `hex_form`, `hex_draw`
follows both) and one is blocked on the owner's `sweep_path_skin` decision.

The finding worth carrying: **one repository can hold two lattices that share a
type and a spelling.** `hex_grid`'s `(q, r)` are odd-r OFFSET with parity-dependent
diagonal deltas; `hex_world`'s `(q, r)` are AXIAL with no parity term at all. Both
are `(integer, integer)`, both spelled `q` and `r`, both in `loft-libs-world`, and
nothing catches a coordinate handed from one to the other. `@HXG-001` and
`@HXW-002` are the two halves of that, and the deeper one is `@HXG-001`: within a
SINGLE module, `hex_round` answers axial while every other function takes offset,
so the mistake lands you on the **west neighbour** — a real cell, adjacent, with
every downstream call still working. That is the failure mode this convention
exists for, and no signature in either package hints at it.

**`imaging` is the row that pays for the whole plan: writing @IMG-002 found a
silent decode bug that had shipped.** `decode_png` handed the png crate's raw
output buffer through and `n_load_png` re-cut it into three-byte `Pixel`s — correct
only for 8-bit RGB. A 2×2 RGBA file came back as **5** pixels with the channels
shifted one byte (`(10,20,30) (255,40,50) (60,255,70) …` — alpha smeared into
colour); a 2×2 greyscale as **1**; a 2×2 palette as **0**, against a header still
saying 2×2. So `len(data) != width * height` and the first thing any caller does —
`data[y*w+x]` — read garbage or null. RGBA is the default for anything with
transparency, so this covered most PNGs in the wild, and nothing anywhere reported
it. Fixed by normalising to 8-bit and folding all four remaining colour types to
RGB, returning `None` rather than a buffer whose length disagrees with the header;
five 2×2 fixtures pin each fold to hand-computed values. `png()` now honours
`load_png`'s boolean too, so a decode failure reaches the caller as the null its own
doc-example always promised, instead of a non-null 0×0 `Image` that *passes* the
documented null check and then silently yields nothing. **The bug was reachable only
by asking what an example should assert** — the library's own suite round-tripped
its own encoder's output, which is always 8-bit RGB, so every colour type it did not
write was untested and unmentioned.

`graphics` (`@GFX-001..005`): a colour is a bare `integer` and a canvas a flat
`vector<integer>`, so the compiler sees none of the contract. The packing is
0xAARRGGBB and `rgb(255,0,0)` is `0xFFFF0000` — **not** the `0xFF0000` a reader
reaches for, whose alpha byte is 0, so it stores fine, reads back as red, and
composites to nothing @GFX-001. The solid primitives STORE rather than composite, so
a half-transparent `fill_rect` replaces what was under it with a colour that merely
records "half transparent"; `blend_pixel` is the door @GFX-002. `get_pixel` answers 0
off the canvas and 0 is a colour the canvas can hold, so the answer is not a bounds
test in either direction — the example's three-sample scan has the colour test
calling 2 "outside" where the size test calls 1 @GFX-003. Span ends are exclusive and
a reversed span draws nothing and says nothing @GFX-004. And `save_png` reads its
colour type off the PIXELS, so one alpha-0 literal in an otherwise opaque picture
turns the file RGBA and that pixel into a transparent hole — asserted by reading the
IHDR byte back off disk, so the claim is about the file, not the buffer @GFX-005.

`shapes` (`@SHP-001..003`): every predicate is STRICT, so tiles laid edge to edge
never collide and a mover pushed out by exactly its penetration depth is done
@SHP-001; the depths say HOW FAR and never WHICH WAY, so applying `+ox` where `-ox`
was meant is a well-typed call that pushes deeper in, and the smaller axis is the
minimal correction @SHP-002; and `rect_circle_overlap` is a real circle test whose
bounding-box substitute grows an invisible square shoulder on every corner @SHP-003.

**Coverage gap recorded, not papered over:** `graphics`'s `gl_*` half needs a window
and has no CI demonstrator, so it carries no tags rather than tags pointing at a test
that cannot exercise them — the same call as `server`'s multi-client event model.

**Three tooling defects surfaced by this repo**, all filed rather than folded into
the rollout. (0) `--native-wasm` cannot build ANY program that `use`s a package with
a `[native] crate` — the emitter calls `loft::native_call::build_store`, which the
wasm build gates out behind `native-extensions` (E0433). The control that pins it:
pure-loft `shapes` builds AND runs on that target from the same tree. loft#967, and
`imaging`'s README now marks the target blocked rather than claiming it. That one
nearly slipped past, because **`loft test --native-wasm` silently ignores the flag**
and runs the interpreter — its footer says so, but a reader scanning for a green
`test result:` line reads it as a wasm pass. (1) `loft test` does **not** pick up an
edit to a library's own `native/src/*.rs`: `auto_build_native` reuses the cached
cdylib whenever the loft-ffi/RUSTFLAGS/codegen fingerprint matches and never stats
the crate's Rust sources, so the imaging fix read as "no effect" through two runs and
only took hold after a hand `cargo build --release`. A false GREEN, and CI is immune
(a clean checkout has no artifact to reuse), which is what makes it a local trap that
survives review — filed as loft#965. (2) The examples gate's def scanner let a
CITATION's prose define the tag it merely mentioned; fixed below.

**`loft-libs-net` is READY TO PR — the first complete repo under the convention.**
`make examples-progress REPO=../loft-libs-net` reads *3 tagged, 1 exempt, 0 deferred,
0 todo* on branch **`worked-examples`** (the plan's no-host-prefix name; pushed, PR
not opened — that needs an explicit ask). Tier 2 (`crypto`, `server`, `web`) is
complete, and finishing the repo took `ssh` and `game_protocol` with it.

**`game_protocol` is the convention's first `exempt`, and it is the row that proves
the verdict column earns its keep.** Every public function is
`msg_<kind>(args) -> GameEnvelope`, the arguments landing verbatim in named fields of
a struct literal; no call order, no encoding, no return value that can mean something
other than it says — its own tests are field-echo assertions, which is what an API a
signature already explains looks like. Untagged, it is indistinguishable from a
package nobody opened; the recorded reason is the whole difference, and it lives in
`examples-exempt.tsv` beside the code rather than in a plan doc nobody reads at
review time.

`ssh` (`@SSH-001..002`): every method self-guards, so a refused connection still
answers a `Session` from which `login`/`open_shell` answer false, `recv` answers "",
and `send`/`resize`/`close` do nothing — right for a terminal app that must not die
mid-frame, but a caller who skips `ok()` runs the entire session ladder, sends every
keystroke, reads an endless empty stream and is never told @SSH-001. And `recv` is a
raw BYTE stream: bound the loop with `size`, never `len`, because `byte_at` indexes
bytes while `len` counts characters, so a `len`-bounded loop drops the tail of every
multi-byte sequence — silently, since both are integers and the loop simply ends
early @SSH-002.

`server` (`@SRV-001..003`): `listen` HALTS on a lost port and that is the design —
a `Server` built on a failed bind is indistinguishable from a healthy one at every
call site, so the process reports itself up and serves nobody, invisibly from
outside too, because a readiness probe reaches whoever DID win the bind;
`try_listen` + `bound()` is the recoverable door @SRV-001 (tagged on the smoke test
that already demonstrated it, rather than writing a second). `header` folds case —
the client picks its own, so a hand-rolled `starts_with("Origin:")` misses the next
one — and rejoins a value's own colons, which a hand-rolled `split(':')[1]` truncates
off an Origin's port @SRV-002. And the return value **says who answered**:
`serve_range` declines with false when there is no `Range` header, the only case the
caller must handle (ignore it and the client waits for a reply nobody sent, or gets
two), while `serve_data` takes every branch and is therefore always true and always
final @SRV-003.

`web` (`@WEB-001..003`): the `pack_*` builder exists because interpolation cannot
carry a zero byte — codepoint 0 is the `character` null sentinel, so `"{a}{b}{c}"`
silently drops every NUL and a binary header with a small integer field is mostly
NULs; the example packs `01 00 02` and shows the interpolated form measuring **2**
bytes, and pins that `pack_take` MOVES @WEB-001. An out-of-range `byte_at` answers
`-1`, which matters *because* 0 is a byte the buffer legitimately carries: `== 0` as
a bounds test reads a real NUL field as end-of-frame **and** an overrun as a NUL
field, wrong in both directions @WEB-002. And a handle is not a connection —
`ws_handler` answers null for a malformed URL only, so a well-formed URL with nobody
listening still yields a usable handle (reconnecting is the library's job) and a
caller reading non-null as "connected" reports itself up against a server that does
not exist; `send`'s false is what reports the link down @WEB-003.

All six run with **no network**, like the rest of those packages' CI tests. @WEB-003
is the pleasing case: its contract is that a down server is *not* an error, so having
no server is exactly what makes it demonstrable. @SRV-002/003 build a `Request` by
hand rather than accepting one off a socket, because what they pin is a DECISION the
request layer makes — which header matched, whether the helper already answered — not
the bytes that reach a client; that is the half a caller gets wrong and the half a
single-process test can hold honestly.

**Stale docs caught in the very functions being documented — one per library so far,
four for four.** `crypto`'s README table called `sha256_b64`'s return a hex digest
when it is base64. `web`'s `tests/byte_at.loft` header said out-of-range returns `0`
when it returns `-1` — and `0` is the one wrong answer to state there, since it is a
legitimate byte value. And `ssh` shipped the bug in its own **documented idiom**: the
header example and `recv`'s doc-comment both wrote `for i in 0..len(out) { byte_at(i,
out) }`, a byte-indexed loop bounded by a CHARACTER count — the exact
`strict-index-text` shape loft lints for, in the library whose whole payload is a
byte stream. All fixed, each pinned by the assertion that found it. The mechanism is
worth naming: writing a worked example forces a value to be *pinned*, and pinning it
is what exposes prose that had drifted. This is the monthly review's failure mode
(a doc that still resolves but no longer matches) arriving at the cheapest possible
moment — and at four for four it is not incidental, it is the second thing the
convention buys.

**Noted coverage gap:** `server`'s multi-client event model (`run` / `poll_event`,
the mutually-exclusive `WsEvent` flags, the pre-split `<msg_id>:<payload>` wire form,
and disconnects absorbed **silently** — the surprising one) owes an example and has
no demonstrator that CI runs; a real one needs a concurrent client+server, which the
package's own smoke test already records as out of reach today. Deliberately left
untagged rather than tagged against a test that does not exercise it.

Third **`crypto` DONE** — the second package on `loft-libs-core`'s
`mac-worked-examples` branch (pushed, PR not opened). Six `@CRY-001..006` worked
examples in `crypto/tests/worked-examples.loft`, cited from the twelve functions
they document. The reason this library owes them is one fact about its surface:
**every parameter and return is `text`**, so the type checker cannot tell a message
from the base64 of a message, a hex digest from a base64 one, a 32-byte seed from a
64-byte key, or a `""` that means *it failed* from a `""` that means *it was empty*.
The six: the two hashing doors (`sha256` text-in/HEX-out vs `sha256_b64` base64 both
ways — and only the base64 door can see a non-UTF-8 byte string at all) @CRY-001;
sign `base64_encode(msg)`, never the message — `"test"` is *itself* valid base64, so
the bare text signs three other bytes **and verifies against itself**, which is why
the mistake is invisible inside one program and only a peer ever sees it @CRY-002;
ES256 determinism (RFC 6979 — two signatures differing is a bug, not entropy) plus
the flat JOSE encodings (bare scalar, `x || y` with no `0x04`, `r || s` not DER), so
check lengths 32/64/64 @CRY-003; a DH output is not a key — run it through HKDF,
where `info` is what separates two keys from one shared secret, and the one-shot is
exactly Extract-then-Expand @CRY-004; the 16-byte tag rides *inside* the sealed blob
and `""` from `open` means AUTHENTICATION FAILED — the same `""` an authentic empty
plaintext answers @CRY-005; `enc` is fresh per seal and must travel with the
ciphertext, while `info` AND `aad` must both match and neither failure says which
@CRY-006. Both validations pass (green in library mode; removing the demonstrator
dangles all six, exit 1), `loft test` / `loft test --native` are 78/78, and a
flipped expected value goes red. The pass also caught a real doc defect in the
function @CRY-001 documents: the README's API table called `sha256_b64`'s return a
hex digest when it is base64 — fixed in its own commit. `loft-libs-core` now reads
2 tagged / 4 TODO (`cbor`, `random`, `regex`, `zttext`).

Second distributed library **`gridmesh` DONE** — pushed as `loft-libs-graphics`
branch `tuxedo-worked-examples` (PR not opened). Five `@GRM-001..005` worked
examples in `gridmesh/tests/worked-examples.loft`, cited from the seven functions
they document: the edit→collect→rebuild→**clear** cycle (@GRM-001), `cell_ixs`
(emit) vs `halo_ixs` (read-only, owned by the neighbour) (@GRM-002), cell indices
surviving a removal uncompacted while a repaint gets a fresh one (@GRM-003), a
dirty GROUP listing its CLEAN member chunks so their cached meshes are copied
(@GRM-004), and the `step_x`/`step_y` parity pair + `idx_at`'s `-1`-not-null
sentinel (@GRM-005). What a toolkit owes an example is the SHAPE of a correct
call, not a single signature — every one of the five is a contract a caller can
get wrong while type-checking. Both validations pass (gate green in library mode;
removing the demonstrator dangles all five, gate exit 1) and `loft test` /
`loft test --native` are 25/25; a flipped expected value goes red, so neither
channel is vacuous. `hex_grid` (the other tier-1 library) is **held**: its
monorepo `loft-libs-world` has uncommitted work in the tree, and the rollout's
one gate is a clean, current checkout.

**What "clean" has to mean, refined by `loft-libs-net`.** That repo's tree was not
pristine either — but every entry was `.loft/` build residue, including one *tracked*
cache file (`server/tests/.loft/cache/…`) that a test run had deleted. Build cache is
not someone's work, so the rollout proceeded there and simply never staged those
paths; `loft-libs-world`'s modified `README.md` and lock files are a different thing
and the hold stands. The distinguishing question is not `git status` being empty, it
is whether a human or another agent would lose anything. (`loft-libs-core` fixed its
own version of this by ignoring `.loft/` by directory NAME rather than one row per
place someone noticed — `loft-libs-net` still tracks a cache file and wants the same
one-line fix, filed here rather than folded into a docs branch.)

Mechanism complete: Phase A (probe), Phase B foundation + **acronym registry
broadened** to the distributed monorepos, Phase C indexer ingestion, and the
**shared gate made repo-agnostic + run from `library-ci-reusable.yml`** so a
`loft-libs-*` repo's own citations are validated in its CI with no per-repo copy
(loft self-check byte-identical, synthetic-lib probe green/dangling/duplicate). Tagged so far:
`@STD-001..012` (stdlib), `@GIT-001..005`, `@LEX-001..002`, `@ACR-001..003`,
`@EHK-001..004` (in-tree libraries), `@ARG-001..004` + `@CRY-001..006`
(`loft-libs-core`), `@GRM-001..005` + `@SHP-001..003` + `@IMG-001..004` +
`@GFX-001..005` (`loft-libs-graphics`, complete), `@SRV-001..003` +
`@WEB-001..003` + `@SSH-001..002` (`loft-libs-net`, complete),
`@FIX-001..013` + `@TIM-001..005` + `@INP-001..004` (`loft-libs-game`,
complete), `@PAB-001..004` (`loft-libs-plugins`, complete), `@MSH-001..005` +
`@GLB-001..004` (`loft-libs-assets`, complete). Remaining: `loft-libs-world`
(held). **The distributed libraries are this stream's
to roll out** — they are shared code with their own validated contract (each
`library-ci.yml` + the register's recorded `api`), not a per-agent private tree, so
loft authors their tags in the canonical monorepo (per `loft-registry/index.json`;
work only in a clean, current checkout). Tier 1 is done except `hex_grid`
(held — dirty monorepo); next: tier 2, `crypto`/`server`/`web`. The genuine wait-for-their-agent case is
only the first-class *apps* (`dryopea`/`crawler`/`moros`), which loft merely cites.

The loft **stdlib** was the starting library
(source 3, in-repo, loft's own gate — the cleanest arrow): twelve functions across
four clusters carry `// Example: @STD-0NN` citations — text (`starts_with_at`,
`chr`, `join`), collection aggregates (`min_of`, `max_of`, `sum`, `tree_walk`), JSON
(`json_parse` navigation @STD-007, the `json_array`/`json_object` builders @STD-008,
the `struct_from_jsonvalue`/`struct_to_json` round-trip @STD-009), and files/IO (the
missing-vs-empty `content`/`read_bytes`/`list_dir` `T?` contract @STD-010, `lines()`
CRLF-normalisation @STD-011, the `FileResult`/`ok()` classify idiom @STD-012) —
resolving to tagged tests (two examples reuse existing clear tests —
`445-generic-tree-walk.loft` for `tree_walk`, `562-file-read-missing-null.loft` for
the null contract — the rest live in `945-stdlib-worked-examples.loft` and the
filesystem-isolated `946-stdlib-file-worked-examples.loft`, all green on both
backends). The gate is `scripts/check_doc_drift.sh examples` (dangling + duplicate),
wired into the `all` run that CI already blocks on — proven red on both a dangling
citation and a duplicate tag. All four stdlib clusters are covered.

**Cross-repo resolution now works (Phase B + first-class-app source).** The acronym
registry lives at `scripts/example_repos.tsv` (acronym → repo → git url → branch).
Projects are assumed checked out as **siblings** in one parent dir, so a repo's
checkout is simply `../<repo>` (no per-entry path navigation; the current repo is
scanned in place). A `// Example: @AAA-###` citation is resolved by acronym:
- the owning repo's tag may sit above **any `fn`** — a `test_*` OR a real function in
  a first-class application's own source (dryopea tags both `tests/` and `src/`);
- a repo with a **local checkout is validated offline against it** (preferred) and
  the check emits the real git blob link (verified live: `@DRY-001` →
  `dryopea/blob/main/tests/25_m3_the_ground_gl.loft#L43`);
- a cross-repo tag whose repo is **not** checked out is reported `unvalidated` (a
  warning, not a hard failure — keeps CI deterministic) with the link still emitted.

Follow-up: hard *online* validation without a checkout (fetch a published per-repo
tag index) — today the offline local-checkout path is the validated one.

## Goal

For a library function whose **correct use is not obvious from its signature +
doc**, point the reader at a **real call site** instead of a prose snippet. Three
sources, in priority order:

1. **The library's own tests** — a test that already demonstrates the function,
   made findable by a stable tag.
2. **A real use in a first-class application** (`moros`, `dryopea`, `crawler`) or
   its tests — the strongest example, because it is the function doing a real job.
3. **A loft in-repo demo application or example program** — `examples/*.loft` and
   the `gallery` / `game` / `play` / `crystal-editor` / `native-editor` demos. For a
   **stdlib function or a language feature** this is often the best *and* cleanest
   source: it is a real program a user runs, and it lives in the loft tree itself, so
   it resolves through loft's own gate with **no cross-repo arrow** (unlike source 2).

The same applies to the **loft feature catalogue** (`@F`/`@I` issues): a language
feature also wants a tag pointing at how it is used **in real life**, on top of the
synthetic run-snippet the catalogue already generates (Phase C2).

The example is a *pointer to working code*, never a snippet in a comment: a snippet
rots silently at the first signature change; a tagged test cannot drift from working
code because it **is** working code (dryopea `EXAMPLES.md § Why not a snippet`).

**Scope discipline (the heart of this plan).** Tag a function **only where a real
call site teaches more than its signature does**. A one-line accessor, a function
whose usage is self-evident from its type — no example; the doc already suffices.
No retroactive sweep of every `pub fn`. This mirrors dryopea's opt-in ratchet, and
is what keeps the gate from going red on hundreds of trivially-obvious functions the
day it lands.

**When no clear example exists yet, write one — in retrospect.** The two sources
above may need to be **created**, not merely cited: if a non-obvious function or
feature has no test that shows it used *correctly*, authoring that test is part of
the deliverable (dryopea `EXAMPLES.md` rule 4 — write a new test if none of the
existing ones is CLEAR). The signal for *which* functions owe one is concrete, and
is the motivation for this plan: **a reader — often an AI agent — that knows a
function/feature exists but cannot use it effectively from its doc alone, and only
succeeds once pointed at an application that already uses it, is exactly the case
that owes a worked example.** Lift the pattern from that real usage into a tagged
test, so the next reader never needs the pointer. This is not a contradiction of the
ratchet: the ratchet says don't tag the *obvious*; this says for the *non-obvious*,
a missing example is a gap to fill, not a reason to skip.

## The mechanism (as built — adapted from dryopea)

- **Tag family `@AAA-###`** — `@`, three-letter acronym, hyphen, three digits
  (`@STD-001`). Distinct from loft's `@P`/`@PLN`/`@F`/`@GH` families (none has that
  hyphen). One acronym per repo, claimed **ecosystem-globally**.
- **Citation** — `// Example: @STD-001` on a comment line directly above the
  function it documents (stdlib fn in `default/`, library fn in `lib/`).
- **Definition** — the tag sits in a comment block directly above the `fn` it names
  (a blank line breaks the block). The `fn` may be a `test_*` **or a real function
  in a first-class application's own source** — a live use documents as well as a
  test does.
- **Acronym registry** — `scripts/example_repos.tsv`: `acronym → repo → git_url →
  branch`. Projects are assumed checked out as **siblings** in one parent dir, so a
  repo's checkout is `../<repo>`.
- **Gate** — `scripts/check_doc_drift.sh examples` (part of the `all` run CI's
  doc-hygiene job already blocks on). Faults: `dangling` (cited, no fn carries it),
  `duplicate` (one tag on two fns in this repo), `unregistered` (acronym not in the
  registry) — all drift; `unvalidated` (cross-repo tag whose sibling isn't checked
  out) — warning only, link still emitted. Proven red on dangling + duplicate.
  **The FIRST tag in a definition block defines it.** An example's prose routinely
  names a sibling ("the failure path, see @ARG-004"), and letting a later mention win
  read that block as defining the *sibling* — surfacing as a `dangling` on the block's
  own tag plus a `duplicate` on the one it mentioned, both pointing away from the real
  mistake (found on the `arguments` rollout, the first library whose examples
  cross-reference each other). Verified a no-op on every existing tree: the def scan is
  byte-identical over loft, `loft-libs-graphics`, `loft-libs-net` and `dryopea`.
  **A block containing an `// Example:` line is a CITATION and defines nothing** —
  not before that line, not after it. The first half was always there (dryopea's
  `crossref` fixture pins it: a file may name a tag in prose and then cite it without
  claiming it). The second half was missing, and a citation is prose whose
  continuation lines routinely name a second tag — `// Example: @GFX-005 … one
  alpha-0 pixel (@GFX-001) makes the file RGBA` read as *defining* @GFX-001 above
  `save_png`, surfacing as `duplicate: @GFX-001` against the innocent real
  definition. The same misdirection the first-tag rule removed, arriving from the
  citation side. Verified a no-op on every existing tree (def scan byte-identical
  over loft 26, dryopea 21, `loft-libs-core` 10, `loft-libs-net` 8, and the three
  zero-tag repos).
  **Self-test built** (`check_doc_drift.sh examples-selftest`, run inside `all`, so
  loft's doc-hygiene job gates on it and library CI is untouched): five temp-dir
  fixtures pinning all four rules — defines, first-tag-wins, blank-breaks-the-block,
  citation-block. Not committed under the repo, because the scanner walks every
  `*.loft` and committed fixtures would inject their fake `@TST` tags into loft's own
  index. Each rule was reverted in turn and each goes red naming its own fixture —
  including one fixture that needed a second pass: first-tag-wins was written with
  both tags on ONE line, where `match()` only ever sees the first, so it passed
  whatever the rule was. **The real trees cannot pin these rules** (they are the trees
  the rules were tuned against), which is exactly why reverting the old
  `Example:`-cancels rule left the whole ecosystem green and only a fixture in
  *another repo* caught it.
  *Not yet built:* `orphan` and opt-in `uncovered` — deferred.

### Design decision — which way the cross-repo arrow points

A library must **not** gain a code dependency on its consumers, and a first-class
app is not a dep in the library's `loft.toml`. So the two sources resolve
differently, and this split is load-bearing:

| source | tag DEFINED in | citation lives on | resolved by (as built) |
|---|---|---|---|
| library's own test | this repo's `tests/` | the fn's doc comment | `check_doc_drift.sh examples`, scanned in place |
| loft in-repo demo / example program | this repo's `examples/` / `tools/` | the stdlib fn / feature issue | same — same tree, no cross-repo arrow |
| real use in a first-class app | the app's `tests/` **or its `src/`** | the stdlib / library fn | the registry maps the acronym to `../<repo>`; validated offline against that sibling checkout, git link emitted |

The gate only ever checks a citation against the repo its **acronym** names, and a
foreign repo is consulted **read-only** via its sibling checkout — never a build
edge, so every library still builds and tests standalone. A cross-repo tag whose
sibling isn't checked out is a warning, not a failure, so CI stays deterministic.
The demo-program row is the case that needs no foreign repo at all: a loft
`examples/` program documenting a stdlib function or feature is in the **same tree**
as what it documents — the preferred source for stdlib + feature examples.

## Phases

Cut per the two-bounds rule (each can go red on its own, for a real reason).

### Phase A — Probe: does the cross-repo pointer work, and does it stay inert? — DONE

Done differently than first sketched (no `arguments`/`examples.sh` port): the probe
rode the real gate. A temporary `// Example: @DRY-001` in loft's stdlib **validated
against the sibling `../dryopea` checkout** and emitted the git blob link; `@DRY-999`
went `dangling`; `@MOR-001` (moros not checked out) stayed an `unvalidated` warning
without failing. Confirms all three: the cross-repo pointer resolves, it produces a
real link, and it is inert (read-only, never a build edge) when the foreign repo is
absent.

### Phase B — The acronym registry — DONE (foundation), still to broaden

Built as `scripts/example_repos.tsv` (a loft-repo file for now, not yet the
`loft-registry` shared home — see Open questions). loft's own: `STD` · `GIT`
(in-tree `lib/git`) · `LEX` (in-tree `lib/lexer`) · `ACR` (in-tree
`lib/audience_crystal`) · `EHK` (in-tree `lib/engine_host`) — one repo may own
several acronyms. First-class apps: `DRY` dryopea · `CRW` crawler · `MOR` moros.

**Distributed libraries registered (claim staked, tags not authored yet).** With
the sibling monorepos pulled, the library acronyms are entered — mapped to their
**monorepo** checkout, not a per-package repo (the design the checkout layout
forced, below): `loft-libs-core` → `ARG` arguments · `CRY` crypto · `RND` random ·
`loft-libs-graphics` → `GFX` graphics · `GRM` gridmesh · `SHP` shapes ·
`loft-libs-net` → `GMP` game_protocol · `SRV` server · `WEB` web ·
`loft-libs-world` → `HXG` hex_grid · `HXT` hex_terrain · `HXW` hex_world. These
rows **stake the ecosystem-global acronym**; none of the monorepos carries a
worked-example tag yet — except `ARG`, whose four are now authored (Status). Still to
add when reached: `MKD` markdown (no sibling checkout found — deferred), the newer
`imaging`/`ssh`/`cbor`/`regex`/`zttext` packages, and `FTR` the feature catalogue
(Phase C2). An acronym is only needed the moment a package authors its first tag: the
per-package CI step is vacuously green without one, so an unregistered package cannot
go red before it opts in.

**These libraries ARE this stream's to roll out** — they are shared code between
agents (not one consumer-agent's private tree, the way dryopea/crawler/moros are),
each guarded by its **own validated contract** (its `library-ci.yml` gate + the API
signatures recorded in the register). So the `edit-only-this-repo` dogfood rule that
holds for a first-class *app* does **not** apply here: this stream authors the
libraries' worked-example tags directly, in the library's canonical location. The
one operational caveat is ordinary git-safety — work only in a **clean, current**
checkout (`loft-libs-graphics`/`-net` were clean when surveyed; `loft-libs-world`/
`-core` were dirty/behind, so leave those until clean rather than risk a concurrent
edit).

- **Design decision — monorepo, not per-package (registry `repo` = the monorepo).**
  The registry's `../<repo>` assumption was written for one repo per package
  (`../gridmesh`). Reality is monorepos: `gridmesh` lives at
  `../loft-libs-graphics/gridmesh`. So the `repo` column names the **monorepo** and
  several acronyms map to it. This needs **no gate change**: `examples_defs_in_tree`
  scans the whole monorepo for the acronym's tag and captures a **repo-relative**
  path, so the emitted git link is correct
  (`…/loft-libs-graphics/blob/main/gridmesh/tests/x.loft#L12`).
- **Canonical location = the monorepo subdir, per the register.** The authority for
  where a library lives is `loft-registry/index.json`: each package entry carries a
  `homepage` (`…/loft-libs-graphics/tree/main/gridmesh`) and every version a
  `subpath` (`gridmesh`) + release `url` on the monorepo. So a worked-example tag is
  authored in that canonical tree — never in loft's `lib/<name>` stubs (empty
  scaffolding) nor an installed/`~/.loft/` copy (a lagging cache). The register also
  records each version's `api` signatures — the contract the tag documents against.
- **Still to do:** a duplicate-**acronym** guard (two repos claiming one acronym) —
  the gate today guards duplicate *tags* within a repo, not acronym collisions across
  the registry.

### Phase C — Shared gate + indexer ingestion — DONE

- **Done:** the gate is one shared script — `scripts/check_doc_drift.sh examples` —
  not a per-library copy; it already resolves cross-repo citations and **emits the
  git link** (the "carry a real link" half of what the indexer owes).
- **Done:** loft's tag indexer (`make index` / `scripts/idx`) now ingests
  `@AAA-###`, so `scripts/idx tag:@STD-011` resolves both the citation and the
  demonstrating fn. Added one branch to `tools/indexer/src/scan.loft`'s per-byte
  `scan_line` dispatch (three uppercase letters, hyphen, three digits; the interior
  hyphen disambiguates from `@P`/`@PLAN`/`@F`/`@GH`, and it is checked AFTER `@F`/`@I`
  so `@FTR-001` is not eaten by the `@F` branch). `tag_is_valid` already accepts the
  shape via its fallthrough, so `idx broken` stays `[]`; the query side (`scripts/idx
  tag:…`) is a generic bucket lookup and needed no change. Verified: all 16
  `@AAA`-shaped tags in the tree index (12 `@STD` + the plan's `@DRY`/`@MOR`/`@FTR`
  mechanism examples), no accidental matches, `idx broken` + `broken-links` clean.
- **Done (2026-08-25):** online validation of a cross-repo tag when its sibling is not
  checked out — `EXAMPLES_ONLINE=1` reads the owning repo's published
  `examples-index.tsv`. Opt-in, and it may CONFIRM a tag but never REFUTE one, because
  the file it reads is the one LIBRARY_AUTHORING.md is retiring. See § What is actually
  left.
- **Closed by measurement (2026-08-25):** the `~/.loft/`-style hidden-root crawl does not
  reproduce here. It was inherited from dryopea's `grep --exclude-dir='.*'`; this gate
  uses `find . -not -path './.*'`, which prunes only a hidden directory directly under
  the scan root.

### Phase C2 — Worked examples for the feature catalogue (S) — **DONE (116 of 116 cited)**

**The mechanism half is built, self-tested and proved end to end (2026-08-18); what
remains is the tracker content.** A feature's citation lives in its ISSUE BODY, reaches
the tree as the generated shadow `doc/features/F###.md`, and is Markdown — so the
citation scanner, which grepped `// Example:` in `*.loft` only, could never have seen
one. Built:

- **A second citation source**, `examples_cited_in_tree()`, scoped to
  `EXAMPLES_FEATURE_DOCS` (default `doc/features`). Inert where the directory is absent,
  which is every library — the same opt-in ratchet as the rest of the mechanism. A
  `dangling` feature citation now names the feature doc and line
  (`doc/features/F104.md:42`).
- **Its own self-test**, `examples-cite-selftest`, five rules, **two of them ABSENCES**:
  `## Example` is a heading and not a citation (81 feature docs carry one, so a scanner
  that stopped requiring the colon would have every one of them citing whatever tag
  appeared underneath), and the `//` code form shown INSIDE a fenced block is
  documentation OF the convention, not a use of it. It earned its place on the first
  run: `- **Example:** @TST-103` did not match, because the pattern allowed one run of
  markdown markers and that line has two.
- **`FTR` registered**, with the rule beside it: a feature whose real use is already
  demonstrated cites THAT acronym, and only a demonstrator authored *for* a feature is
  an `FTR`.
- Proved end to end against a real feature doc: a citation resolves to
  `tests/scripts/946-…:15`, a bad one goes red naming the doc, and removing it is clean
  again.

**THE FIRST SLICE NEEDS NO NEW TESTS, and that is the phase's first finding.** Eleven of
the first twelve citations point at demonstrators that already existed — `@STD-010/011/012`
for **F40** (file I/O), `@STD-007/008/009` for **F42** (JSON), `@STD-005/006` for **F26**
(interfaces + bounded generics), `@STD-003/004` for **F6** (vector aggregates) and
`@STD-001` for **F97** (`len` vs `size`, whose reader's mistake is exactly the byte offset
that example scans). **The catalogue's gap was the POINTER, not the example** — which makes
the rollout far cheaper than "author one in retrospect" implied, and says the two halves of
@PLN141 were building the same thing from opposite ends without a link between them.

**`@FTR-001`/`@FTR-002` are authored, for F104 `store_reclaim()` — and writing them found
the doc's calibration sentence to be false.** The doc tells a reader how to decide whether
the call is worth making: *"Read `store_memory()` first — its `tail%` is exactly what this
call returns."* Measured across eight shapes whose tails ran from 13 % to 60 %, the store
lands at **`tail 11%` every single time**: 11 % is a growth reserve the allocator keeps, so
the bytes handed back are `tail_before − 11 %` of the resulting capacity and never the whole
tail. At a 13 % tail the report suggests ~36 KB and the call returns **5 832 bytes** — a 6×
over-promise, and it is worst exactly where the decision is marginal. `tail%` ranks
candidates; it does not size the return.

**And a second fact the docs do not carry: what you get back depends on WHERE you dropped,
not how much.** Same 4000 records, same 2000 dropped:

| drop | free-blk | mergeable | after reclaim: free-blk | bytes back |
|---|---|---|---|---|
| contiguous 2000 | 2004 | **1997** | **7** | 25 400 |
| scattered 2000 | 2002 | **5** | **1997** | 16 312 |

`mergeable` counts adjacent free neighbours that never coalesced, so it measures how
CONTIGUOUS the drop was — and it is exactly the part `store_reclaim` can fix. A scattered
drop leaves the store in ~2000 pieces permanently, because the live records between the
holes are what keeps them apart. So the report says, before the call, which kind of drop
was made. **My first hypothesis had this backwards** (I predicted scattered drops would be
the mergeable ones) and the probe corrected it — recorded because the wrong version is the
intuitive one.

**THE FIRST SLICE IS APPLIED (2026-08-18).** Twelve pointers across six features, edited
into the loft-lang/features ISSUE BODIES — the canonical home, never the shadow — and
pulled through with `make features-fetch && make features-gen`:

| feature | cites |
|---|---|
| F6 vector aggregates | `@STD-003` `@STD-004` |
| F26 interfaces / bounded generics | `@STD-005` `@STD-006` |
| F40 file & directory I/O | `@STD-010` `@STD-011` `@STD-012` |
| F42 JSON | `@STD-007` `@STD-008` `@STD-009` |
| F97 `len` vs `size` | `@STD-001` |
| F104 `store_reclaim()` | `@FTR-001` `@FTR-002` |

`check_doc_drift.sh` reads 28 citations, all resolving; `make features-check` is green.
**F97 is the sharpest of the twelve** — the feature is about characters against bytes, and
`@STD-001` is a scanner calling `starts_with_at` at a BYTE offset, which is the exact
mistake the feature exists to prevent. That pairing existed in the repo for weeks with
nothing joining the two.

**The negative gate is proved on real content, not a fixture:** removing the `@FTR-001`
tag from its demonstrator turns the gate red naming the feature doc AND the citation text
(`doc/features/F104.md:43:Example: @FTR-001 — …`), and restoring it is clean. So a feature
that cites a demonstrator someone later deletes cannot stay quietly wrong.

**`store_reclaim`'s own doc comment is corrected too** (`default/02_files.loft`), since it
was the source of the false calibration and it now cites `@FTR-001`/`@FTR-002` — the
numbers in the prose cannot drift from the test without the gate saying so.

**Still to do:** the other ~111 features. The first slice says the rollout is mostly a
READING task rather than an authoring one — find the demonstrator that already exists,
and only author an `@FTR` where none does.

**THE SECOND SLICE IS APPLIED (2026-08-18) — eight entries, and four of them described
loft as copy-by-default.** The pairing pass was again mostly READING: `@GIT-001..005` for
**I117** (git natives — the entry's own "first consumers" section already named the two
programs the tags sit in), `@EHK-001..004` for **I85**, `@RGX-001` for **F47**,
`@SSH-002` for **F97**, `@RND-002` for **F106**, `@TIM-001` for **F95**. Two new
demonstrators were authored, `@FTR-003` / `@FTR-004`, because the rest of the slice was a
correction and a correction needs a test that can go red.

| feature | cites | what the entry said |
|---|---|---|
| F21 references `&T` | `@FTR-003` | *"without `&`, the function only sees a copy and your original is left alone"* |
| F22 closures | `@EHK-001` `@FTR-004` | *"it keeps behaving the same even if those values change afterwards"* |
| F47 imports | `@RGX-001` | (nothing about resolution order — added) |
| F95 value structs | `@TIM-001` `@FTR-003` | *"hand one to somebody and they cannot change yours"* |
| F97 `len` vs `size` | `@SSH-002` | (second citation, a real byte stream) |
| F106 copy/move | `@RND-002` `@FTR-003` `@FTR-004` | *"**Two** rules decide whether two names share data"* — a call is not one of them |
| I85 engine-host natives | `@EHK-001..004` | (locator named only the Rust file; the typed surface added) |
| I117 git natives | `@GIT-001..005` | — |

**The measured rule, both backends** (the matrix is `@FTR-003`):

| | callee writes a FIELD / ELEMENT | callee REPLACES the binding |
|---|---|---|
| `integer` / `text` | — | lost; `&` → lands |
| `vector` | **lands, `&` or not** | lost; `&` → lands |
| `struct` | **lands, `&` or not** | lost; `&` → lands |
| `value struct` | **lands, `&` or not** | lost; `&` → lands |

So `&` buys a scalar write-back and whole-binding replacement, nothing else — and a
closure capture splits the same way (`@FTR-004`): a scalar is a snapshot, a struct or
collection is shared in both directions, which is what makes the audience-demo kernel's
handler (`@EHK-001`) able to append to the world it captured.

**The finding that ties the four together: loft's own compiler already says it.**
`advice[slow-reference-parameter]` reads *"field mutation already propagates to the caller
without it"* and its fix line is tagged `[reference · @F21]` — the diagnostic sends the
reader to the entry that denied it. A catalogue is checked against the code it describes
only where something crosses the two; nothing did here, and the entries were internally
consistent all the way to the wrong model. **Where a diagnostic names a doc, that pairing
is a cheap oracle: read the doc it points at and check it agrees.**

**A test can pin a defect as a promise.** Three tests asserted the dead-assignment warning
that fires on `s = 10; f = fn() { s }; s = 20;` — and each of them ALSO asserted the value
that proves the capture read the 10 (`add_base(5) == 15`). A self-contradicting test reads
as coverage. Fixed in the same session (see below), which is what made `@FTR-004` writable
at all: the demonstrator tripped the false positive it was written to describe.

**Fixed here: the dead-assignment lint counted a closure capture as no read** (`warning` tier,
so it gates a library's CI). `Variables::track_write` compares `uses` against `uses_at_write`,
and the capture site deliberately does not touch `uses` — with the comment *"Do NOT call
var_usages — that would interfere with the dead-assignment check"*. The narrow cure is the
one that cannot regress a true positive: skip the report for a variable a closure captured
(`!var.captured`), which stays silent only where a read exists that the counter cannot see.
A genuinely dead pair BEFORE the capture still warns — that boundary is the test.

**THE THIRD SLICE (2026-08-18) — F2, F5, F38, and a promise that is an INTEGER promise.**
`@MSH-005` (mesh3d's degenerate camera) and `@TIM-002` (a date the user typed) were the
demonstrators already in the tree; `@FTR-005` was authored because the correction needed a
gate. **F38 said a calculation that cannot give a real answer produces `null` — "the way a
spreadsheet shows an error in one cell while the rest of the sheet still works".** Measured,
both backends:

| | value | `?? fallback` fires? |
|---|---|---|
| `integer x / 0`, `x % 0`, overflow | null | yes |
| `float 1.0 / 0.0` | `+inf` | **no** |
| `float 0.0 / 0.0` | null (NaN is the null float) | yes |
| `float 1.0e308 * 10.0` | `inf` | **no** |

So whether `x / d ?? 0.0` defends anything depends on whether the NUMERATOR happened to be
zero — which is not what the line says, and is exactly why `normalize3`'s guard is sound
(a zero-length vector zeroes the numerator too) one component away from where it is not.
F2 gained the same warning from the `??` side, and F5 gained the rule the pairing exposed:
**whether a cast can refuse is the TARGET type's choice** — a `value struct` has no null, so
`"next tuesday" as DateTime` is total and answers the epoch.

**And a defect the row could not have found from the docs: loft#983.** The raw quotient
reads as `null` once bound to a local or returned, while the same division keeps its
infinity inline, through `??`, as an argument, and into a vector — one rule applied in some
destinations and not others, identically on both backends. Narrowed by the next probe: a
bound float OVERFLOW keeps the infinity, so it is the DIVIDE op's null-producing path that
normalises, not the bind. `@FTR-005` asserts only the guarded shapes, so it stays honest
whichever way #983 is settled.

**THE FOURTH SLICE (2026-08-18) — F1, F3, F12, and a modifier that is a storage width.**
`@IMG-004` and `@GFX-003` (graphics/imaging) and `@MSH-002` (mesh3d) were the demonstrators
already in the tree; `@FTR-006` was authored for the settled half of F12.

**F12 listed five field modifiers as if they were one kind of thing.** Measured:

| modifier | when it is checked |
|---|---|
| `const` | compile time — assigning the field is an error |
| `assert(...)` | RUN time — `field constraint failed on <Type>.<field>` stops the program |
| `= default` / `computed(...)` | not checks at all: one fills an omitted field, one recomputes on access and stores nothing |
| `limit(lo, hi)` | **never** — it selects the STORAGE WIDTH (`u8` *is* `integer limit(0, 255)`) |
| `not null` | **never** — deprecated, no effect |

An out-of-range write to a `limit` field is silently dropped (1-byte), silently wrapped
(2-byte), or aliased onto `lo` at exactly `lo + 256` — `Store::set_byte` guards
`val <= min + 256` against a range that holds `min..=min+255`, and discards the `false` it
returns when the write is refused. Filed **loft#984** (the fix is a design choice: refuse at
compile time like `u8` does, raise like `assert` does, or store null). `@IMG-004` had
measured the construction half in a published library first — doubling a bright channel
stores 0, so brightening turns the brightest pixels black, in a well-formed image with
nothing on stderr.

**And the oracle fired a second time.** `advice[not-null-deprecated]` reads *"`not null` is
deprecated and has no effect"* and its fix line is tagged `[struct records · @F12]` — the
entry it points at listed `not null` as a constraint. Two for two: **where a diagnostic names
a catalogue entry, read that entry.** It is now the cheapest known way to find a wrong one.

**F1's title promised "in-band sentinels" and its body never said what they are.** Added the
table (integer's most-negative, NaN, `false` for a boolean, NUL for a character, a one-NUL
text, record 0, enum 255) with the consequence that a value which IS the sentinel reads as
absent — `false` being the one that bites. `@GFX-003` is the same shape one level up in a
shipped library: `get_pixel` answers 0 off the canvas and 0 is a colour the canvas holds.
**F3 listed `single` in its title and never mentioned it in the body**; cited from `@MSH-002`,
whose GPU upload buffer is a `vector<single>`.

**THE FIFTH SLICE (2026-08-18) — F18, F23, F96, chosen by the ORACLE rather than by
reading.** Every diagnostic fix line carries a `[concept · @Fnn]` ref, so
`grep -rn 'concept_ref: "@' src/` ranks the catalogue by how much the compiler already says
about each entry: F16 (8 diagnostics), F1 (7), F97 (6), F109 (6), F21 (5), F2 (5), F12 (5),
F106 (5), F18 (4)… Working that list top-down is cheaper than reading entries at random,
and it is what found F12 and F21.

**F18's guarantee stops at the first call.** `const` on a parameter refuses every write
THROUGH it — `Cannot modify const parameter 'a'` — and does not stop the parameter being
handed to a callee that writes:

```loft
fn bump(a: Account) { a.balance = 999; }
fn describe(acct: const Account) -> text { bump(acct); "…" }   // accepted, both backends
// the caller's 500 becomes 999
```

Not filed: @PLN40 owns it, and its `const-model.md` **rule 4 names the wrong spelling** —
"a `const` value may be passed to a `const` param but not a `&` param". `&` is not what
lets a callee write; a plain struct parameter names the caller's record just as well (the
slice-2 finding). Recorded there as a scope correction to steps 4–5: gate every writable
argument position, not just `&`. The entry now says what `const` buys and what it does not.

**F96 drew the line in the wrong place.** It said "types with no default (a struct without
defaults for its fields) are a compile error"; measured, a record without field defaults
discharges to every field's own zero (nested records, vectors and text included). What has
no default is a FIELD whose type has none — a plain `enum` with no chosen variant — and the
compiler already names exactly that. `@FTR-007` pins the table.

**F23 gained the pairing that explains why it matters:** `@ZTX-004` (zttext takes `resolve`
and `measure` as function parameters, so cross-target layout parity is a consequence of the
signature) and `@EHK-001`. Its "keep it for later" claim was re-measured across all four
storage shapes — local, vector element, struct field, argument — including a CAPTURING
closure in a struct field, which works on both backends; the `@EHK-001` demonstrator's
comment still blamed loft#313 for that, closed in June, and now says what the capture is
actually for.

**THE SIXTH SLICE (2026-08-18) — F16, F100, F109, and a whole lint family that a test
run cannot see.** The oracle picked them (F16 has 8 diagnostics pointing at it, F109 6,
F100 2). Two demonstrators authored: `@FTR-008` (the stdlib supersedes its own `sum_of`
with the general `sum`) and `@FTR-009` (the copy-mutate write).

**loft#985 — the post-scope-check lints do not run under `loft test`.** One block in
`src/main.rs` runs `warn_copies`, `warn_dead_stores` (lost-write), `warn_double_move`,
`warn_lost_temp_writes` and `superseded_fold_diagnostics` on the PROGRAM path. Under
`--tests` none of it runs — **including a hard error**:

```loft
fn doubled(v: integer) -> integer { scaled(v, 2) }
#superseded "no_such_thing"
fn test_it() { assert(doubled(21) == 42, "still answers"); }
```

`loft --tests` → `ok. 1 passed`, with the steer still saying *use `no_such_thing`*; the same
file with a `main` → `error[superseded-unknown-successor] … the steer would ship dangling`,
exit 1. `warning[never-read]` DOES fire under `--tests`, which is what makes the split hard
to notice. It is the exact hole the lint was written for: @PLN107's motivating case is a
published canvas that shipped every primitive as a no-op, and a library is checked by
`LOFT_DENY_WARNINGS=1 loft --interpret --tests tests`.

**F100 said the wrong tier.** "Advice-tier, so it never fails a build" — both halves are
`warning` (`lost-write`, `never-read`), which is the right tier by loft's own rule and the
opposite of what the entry promised. Corrected, with the real message text and the loft#985
limit; `@FTR-009` asserts the wrong RESULT rather than the diagnostic, because the diagnostic
is exactly what a test run does not get.

**F16 gained the two facts a reader needs and it had neither:** a declared result must arrive
on every path (a fall-through is a compile error, not a zero), and a result you IGNORE is
discarded in silence — which `@ZTX-001` is the real use of, an editing engine of pure
functions where a discarded return is an editor that does not type.

**F109 was accurate** — its two checks are real, and both were measured rather than assumed
(a missing successor is an error, an un-folded shim a warning). The gap was only the pointer,
plus the loft#985 caveat. **A near-miss worth recording: the first read of the
missing-successor probe was `head -6` and showed only the advice, which read as "the rule is
not enforced". It is — the error was four lines further down.** Truncating a probe's output
is the same failure as a vacuous channel: read the whole thing, and the exit code.

**Still to do:** the other ~92 features, plus two pairings this slice deliberately did NOT
make. **F109 `#superseded`** was one of them and is now cited from `@FTR-008` (the stdlib's
own `sum_of` → `sum`); a second, cross-repo citation would still be worth having — an
`@RGX-005` in loft-libs-core that calls `regex::find`, asserts the steer, and asserts the
shim answers what `search` does. **F43 is the catalogue's one unauthored stub** and is
deliberately so (@PLN92: "random, deferred as a library") — it is the single entry the
shadow generator skips, so `doc/features/` holds 116 pages for 117 issues.

**One diagnostic gap noticed while measuring, not filed:** replacing a whole binding
inside a callee (`fn f(s: S) { s = S { … } }`) loses the write silently — no `lost-write`,
no dead-store. It is the one cell of the parameter matrix where the wrong model produces a
wrong answer, and the natural home is @PLN107's dead-store lint.

**Slice 7 — F17, F7, F37, and one class of bug behind all three.** Picked by the oracle
(the diagnostics that name each entry), and every one of them fired again: three separate
diagnostics point a reader at an entry that does not contain what the diagnostic says.

**F17 was a fix, not a correction.** `render(cfg, dry: true)` compiled and
`cfg.render(dry: true)` was a parse error — the same function, the same argument, the
same default. `parse_call` collected `name: value`; `parse_method` never did, though
`call_with_named` already took `is_method` and resolved names against the callee's own
attributes. The gap mattered most exactly where loft sends an author to close it:
`advice[trailing-boolean-parameters]` says *"give them defaults so callers pass only what
they change"* and tags the fix `@F17`, and on a method only a named argument can change a
flag that is not a PREFIX. Take the advice and the only spelling left was `f(false, true)`
— the shape the advice complained about, now with a default in front of it.

**The fix passed the codegen gate exactly**: `c.show(loud: true)` now emits IR + native
Rust byte-identical to `show(c, loud: true)`, which was already correct, so the working
bytecode was a SOURCE SHAPE rather than something to hand-write.

**And it was WRONG on the first attempt, in a way only the matrix caught.** With
`parse_method` fixed, `c.show(loud: true)` worked when `show` was declared ABOVE the caller
and still failed below it — a fix that makes a program's legality depend on declaration
order is worse than no fix. Pass 1 cannot resolve a forward-declared method, so the call
never reaches `parse_method`; it reaches a FALLBACK that consumes the argument list
without resolving anything, and that fallback had not been taught about named arguments.
There were three such fallbacks, all copies of one loop. Two are now one
(`skip_remaining_args`, which the `Type::Unknown` receiver path calls instead of its own
copy), and the third — the index list — had the identical hole: **`grid.cells[1, 2]`, a
compound-key lookup, parsed above its declarations and reported `Expect token ]` below
them.**

*Generalises:* **a fallback that consumes what it cannot resolve is invisible until a
spelling it never learned walks into it, and it fails as an ORDERING dependency rather
than as a missing feature.** The tell is a matrix cell that moves when you move the
declaration and nothing else. Fixing the parse loop and not the fallback would have
shipped a feature that works in the file you tested and not in the file you wrote.

Error recovery improved with it: `s.nosuch(width: 3)` was five cascading errors with no
`Unknown field` among them, and is now that one message.

**F17's compatibility sentence was false at the boundary that matters.** *"Adding a new
optional setting later never breaks the calls people already wrote"* — measured, a direct
call is additive (`scale(5)` keeps answering 10), and a function used as a VALUE breaks:
`expected fn(integer) -> integer, got fn(integer, integer) -> integer`, and a bound fn-ref
`expects 2 argument(s), got 1`. A default is not part of the type, so a fn-ref caller must
be rewritten to the new arity and pass the "optional" argument by hand. Growing the
signature of something handed out as a value IS a breaking change, and @F23 (functions as
values) is a first-class feature of the same catalogue.

**F17 also surfaced a lint that told the author to delete a live parameter** — the same
shape as the closure-capture false positive fixed the slice before. `fn window(rows:
integer, height: integer = rows * 10)` warned *"Parameter rows is never read"* with the
fix line *"drop the parameter `rows` — and its callers' argument"*; taking it deletes what
the default reads. A default is parsed against TEMPORARY variables that are removed before
the real parameter slots exist, so the read landed on a throwaway. It is answered off the
stored signature instead (name → argument index → every attribute's default), which also
covers a default lifted into a function of its own. A parameter nobody reads still warns,
and one sitting beside a default-reader is still named individually.

**F7 called an index a collection, and that is the sentence loft ships a diagnostic to
correct.** Two `hash<Person[…]>` fields on one struct are not two collections: one append
through either is visible through both and `len` moves on both — measured in both
directions, so there is no primary. `advice[linked-group-double-fill]` says exactly this
(*"a second route to `by_name`'s records, not a collection of its own"*) and both its fix
lines are tagged `[keyed collections · @F7]`. Worth having as a CAPABILITY, not only a
hazard: a multi-index table costs one field declaration and no extra records. The entry
also never showed the plural its own TITLE promises — `hash<T[keys]>` takes a compound key,
read `grid.cells[1, 2]` — and never said that `+=` on a key already present REPLACES,
though its example only ever appends a fresh one.

**F37 is accurate in nine shapes and deviates in exactly one.** Hand-computed against the
convention a reader brings: `**` is right-associative (512), `+` binds tighter than `<<`
(32), `and` tighter than `or`, `/` and `%` truncate toward zero (-3, -1, 1), `>>` is
arithmetic (-4). The single deviation is the one loft warns about — a leading `-` is a
SIGN, so `-x ** 2` is `(-x) ** 2` = 9 where reading it aloud gives -9 — and the same symbol
with a left operand goes the other way (`0 - x ** 2` IS -9). `DIAGNOSTICS.md` tags that
warning `@F37`; the entry did not mention it. **The warning is deliberately silent when the
base is a LITERAL** (the code says so: *"-2 IS a number"*), which leaves the shortest
spelling of the trap unmarked — `-3 ** 2` is 9 and `-1.5 ** 2.0` is +2.25, both silently.
Left as designed and written into the entry, because the value is the same either way and
the reasoning is stated; what was missing was the reader knowing it.

**Not fixed, filed instead: an EMPTY struct literal cannot be written above its
declaration.** `d = Dir { };` with `struct Dir` below it is `Expect token ;`, while
`Dir { by_name: [] }` in the same position is fine — a fourth member of the same
forward-reference class, with a different mechanism (the literal, not an argument list).
Both backends, both binaries, pre-existing.

**A trap this slice re-learned the expensive way:** the first run of the new demonstrator
reported `Unknown field Cell947.x` and it read as a forward-reference gap. It was a NAME
COLLISION — `Cell947` was already declared 400 lines up for `@FTR-003`. A shared
demonstrator file has one namespace; the suffix convention does not make a name unique.
The compound-index gap beside it WAS real, which is what made the misreading comfortable:
grep the file for the name before adding a type to it.

**Slice 8 — F29, F33, F35, and the forward-reference class claims a fifth member.**

**F29's "every possibility has to be covered" is a statement about ENUMS.** Measured,
exhaustiveness is enforced two different ways and only one stops a build: a missing enum
variant is a hard error naming it, while an OPEN domain (integer, text) with no `_`
compiles and answers `null` for anything no arm covers. What reports it is the
nullable-into-non-null check, so it needs a non-null CONTEXT — returning the match into a
declared `text` warns, and binding the same match to a local and printing it says nothing
at all. Writing `_` is what makes the result non-nullable. The `null` arm is real and
catches a genuine absence (a vector overrun lands there); its price is @F1's in-band
sentinel, so the most negative integer cannot be told from "absent".

**F33 was missing the two facts a caller needs before writing a `par` at all**, and both
are strong. Results reach the body **in SOURCE order, not completion order** — proved by
giving item 1 the most work and item 8 the least so the workers finish backwards, on both
backends; that is what decides whether `results += [b]` is writable, and the entry's own
example only ever does `total += b`, which is order-blind and cannot show it. And a
worker's captured arguments **must be scalars**, enforced at compile time: `text` counts
as a reference so even a literal `"x"` is refused, a captured struct the worker WRITES
earns a second error naming the data race, and the loop ELEMENT is the documented
exception. So "no manual thread handling" is not a promise to trust — the racing shape
does not compile. Speed measured twice each way: ~2.0x at 2 workers, ~3.2x at 4, ~4.7x at 8.

**F35's entry never mentioned `{{` / `}}`**, while `format-unescaped-brace` is an ERROR
whose fix line is tagged `@F35` and spells the cure — the oracle again. And the two
mistakes are answered unequally: a stray `}` gets that coded mechanical fix, a stray `{`
gets `Formatter error` plus a cascade (loft#989).

**F35's real finding is the dedent, and it took three rounds to state correctly.** The
rule was not the "common leading indentation" LOFT.md described: it was
*(closing-backtick column − 1)* spaces, removed from a line only if that many of its
leading bytes were all spaces. So a line indented LESS kept everything while its siblings
lost theirs (two lines four spaces apart in the source came out level), and a TAB-indented
line was never dedented. **Then the amended entry example failed to dedent at all, and the
difference was the `{name}` in it: one interpolation anywhere switched the strip off
completely, trailing blank line included (loft#990).** The strip amount came from the
CLOSING backtick and a hole made the lexer emit text before that column was known — so the
dedent served the block with no values in it and silently stopped serving the template,
which is the shape the feature is advertised for.

**loft#990 is fixed on `main`, and the slice's own test went RED on the rebase** — which is
the lesson worth keeping. The cells pinned the broken rule as a promise (`size == 19` for a
held block, `size == 16` for the uneven one), so the fix that made them right made them
fail. The base is now the FIRST CONTENT LINE, interpolation is no exception, and only the
TAB edge survives. *A test written against measured-but-wrong behaviour is a tripwire on
the fix, not a guard — say in the cell which side of a known issue it is pinning.*

*Generalises:* **the doc example you write to illustrate a fix is itself a probe — run it.**
The interpolation finding exists only because the amended entry example was executed
before publishing rather than after. Two more doc defects fell out of the same habit:
LOFT.md's `msg` example sits directly under the sentence describing the strip and is not
stripped, and **its flagship `shader` example does not compile at all** — `void main() {`
opens a hole, so a GLSL block needs every brace doubled. Doubled, it compiles AND dedents
(because `{{` is not a hole), which is now what the doc shows.

**The fifth member of slice 7's forward-reference class, and this one had four sites.**
A `par` loop whose worker is declared BELOW it was `Expect token ;` on the following line:
the loop was read as a VALUE, because the recovery path taken when the worker cannot be
resolved left `*code = Value::Null` instead of a Void statement. Its comment said "errors
already reported", true on pass 2 and false on pass 1, where an unresolved forward
reference is the ordinary state — the same misconception as `skip_remaining_args`'s
"consume a trailing `(…)` to avoid cascading parse errors". Fixing the one exit was not
enough: three more recovery exits in `parse_parallel_for_loop` had it too, which surfaced
only when the demonstrator declared the element STRUCT below as well.

*Generalises:* **when a recovery path is wrong for pass 1, look for its siblings before
declaring the fix done** — one exit fixed and three left is a fix that works on the probe
and fails on the file. And the counted-axes rule earned its keep again: the matrix needed
declaration order moved for the WORKER and for the ELEMENT TYPE, not just one of them.

**A baseline test was pinning the broken recovery as a promise** (third time in this plan).
`36_par_worker_writes_parent` locked the spurious `Expect token ;` cascade — and its name
was a lie: the source has no worker CALL, so it died on the parse long before any parent
write. Split into `36_par_worker_is_not_a_call` (which now locks the real message) and a
new `53_par_worker_writes_parent` that reaches the @C93 data-race error the name promised.

**Filed, not fixed:** loft#987 (`par` with an EMPTY body fails to compile on `--native`;
`n_parallel_discard` is declared one parameter short AND its generated body is empty, so
"fixing" the arity would turn a loud error into a silent no-op — the
[[panic-noop-on-native]] shape), loft#988 (a float-returning `par` worker declared below
its loop mistypes a `+=` accumulator; `t = t + b` is the verified workaround and a plain
call to the same function is fine, which is what makes it par-specific), loft#989, loft#990.

### Phase C2 — the original design

The same mechanism applies to the **feature catalogue** (`loft-lang/features`,
`@F<N>` feature / `@I<N>` infra), not just library functions — a reader learning a
language *feature* is best served by seeing it used **in real life**, not only by a
synthetic snippet.

- **What exists today:** the generator promotes the **first** ` ```loft ` fence in a
  feature issue into a RUN test (`tests/docs/features/*.loft`). That proves the
  feature compiles and runs, but it is a teaching snippet — it does not show the
  feature carrying real work.
- **What this adds:** a feature issue gains an `Example:` citation to a `@AAA-###`
  tag naming a **real** test or first-class-app usage that exercises the feature.
  Same tag family, same indexer (Phase C), same `dangling`/`orphan` gate. The two
  are complementary and both kept: the first-fence RUN test is the minimal
  compiles-and-runs gate; the tag is the pointer to idiomatic real-life use. The
  **preferred source for a feature** is a loft in-repo demo/example program (source 3
  above) — same tree, no cross-repo arrow. Where a feature has no test or demo
  showing real use, **author one in retrospect** — the same rule as the library
  rollout.
- **Where the citation lives:** in the **issue body** (canonical), never in the
  generated shadow (`index/features.json`, `doc/features/`, `tests/docs/features/`)
  — the `features-check` drift guard fails on hand-edits. Edit the issue, then
  `make features-fetch && make features-gen`.
- **Acronym:** the catalogue gets its own — `FTR` — registered in Phase B alongside
  the libraries', since a feature tag shares the ecosystem-global namespace.
- **Validation (can go red):** `make features-check` stays green; a feature citing a
  deleted test goes `dangling` through the same gate; `idx tag:@FTR-001` resolves
  the real-usage test.

### Phases D… — Per-library rollout, one library per phase (S each)

**Started with the loft stdlib** (`@STD`, source 3, in-repo): text
(`starts_with_at`, `chr`, `join`, @STD-001..003), collection aggregates
(`min_of`/`max_of` @STD-004, `sum` @STD-005, `tree_walk` @STD-006 — the last reusing
an existing test), JSON (`json_parse` navigation @STD-007, the
`json_array`/`json_object` builders @STD-008, the struct↔JSON round-trip @STD-009),
and files/IO (the missing-vs-empty `content`/`read_bytes`/`list_dir` contract
@STD-010 — reusing `562-file-read-missing-null.loft` — `lines()` CRLF-normalisation
@STD-011, the `FileResult`/`ok()` classify idiom @STD-012). The four stdlib clusters
are covered; the rollout moves to the registered libraries below.

**In-tree libraries shipping.** Each anchors its tags on **real, exercised code** —
a live consumer or a CI-run test — never a prose snippet:
- **`lib/git`** (`@GIT-001..005`, registered `GIT`→loft): tags on live uses in two
  consumers run every `make index` / `make view` — `scan.loft` (`tracked_files`
  @GIT-001) and `refresh.loft` (`ahead_behind` @GIT-002, `log`-vs-`head`-date
  @GIT-003, `changed` rename→new-path @GIT-004, `numstat` `-1`-is-binary @GIT-005).
- **`lib/lexer`** (`@LEX-001..002`, `LEX`→loft): the core cursor idiom — `matches`
  (consume-if-equal) vs `test` (peek) + `identifier` — tagged on `parser.loft`'s
  `function` grammar rule (@LEX-001), plus the **backtracking protocol**
  `anchor`/`revert` (@LEX-002) tagged on `parser.loft`'s `object` rule, which
  anchors the cursor, attempts an object literal, and rewinds on the first
  non-matching field. Both are exercised by the `16-parser` doc test (which parses
  a function and an object literal). (The format-protocol / comment functions owe
  examples too, but their only current demo is the *rendered* `15-lexer` doc test,
  which must not carry a tag — a noted coverage gap for a non-rendered demo.)
- **`lib/audience_crystal`** (`@ACR-001..003`, `ACR`→loft): tagged on the
  `01-editor-helpers` test (run both backends by `wrap.rs`) — `hex_at_world`
  picking inverse @ACR-001, the `crystal_incr_new`/paint/rebuild/`full_mesh` +
  `crystal_mesh_to_lines` stride-10 loop @ACR-002, `crystal_incr_erase`
  edit-then-rebuild @ACR-003.
- **`lib/engine_host`** (`@EHK-001..004`, `EHK`→loft): the @PLN18 server kernel,
  a sequence-sensitive API — tagged on the audience-demo kernels that CI spawns
  (`engine_host_audience.rs`, `engine_host_udp.rs`): `run` loop-skeleton with
  closures-as-args @EHK-001, `broadcast`-to-all vs `send`-to-one @EHK-002, the
  `fast_lane`/`fast_lane_keyed` sync-lane declaration *before* `run` @EHK-003, and
  `run_client` + the `client_sync_next`/`client_sync_payload` drain loop @EHK-004.

All resolve through `idx` and the gate; each host program / test still runs.

**All the distributed libraries are this stream's to roll out.** They are shared
code between agents, each guarded by its own validated contract (its `library-ci.yml`
gate + the register's recorded `api`), so — unlike a first-class *app* — this stream
authors their worked-example tags directly, in the canonical monorepo tree (per the
register; see Phase B). The in-tree libraries went first only because they were the
nearest to hand (`lib/git` — done — anchoring live uses in `scan.loft` +
`refresh.loft`, no test authored); the priority order below then covers the
distributed monorepos. The one gate is git-safety: roll out only in a **clean,
current** checkout, and leave a dirty/behind monorepo (e.g. `loft-libs-world` was
39 behind) until it is clean. The genuine wait-for-their-agent case is narrower than
first written: it is the first-class *apps* (`dryopea`/`crawler`/`moros`), whose own
agents author their `@DRY`/`@CRW`/`@MOR` tags — loft only *cites* those.

One library per phase (the skill's one-call-site-at-a-time shape). For each: tag the
highest-value functions from its own tests and, where it exists, a real consumer
usage — and where **no** clear demonstrating test exists, **author one in retrospect**
(lifting the pattern from a first-class app that already uses the function correctly)
rather than skipping the function; the shared `check_doc_drift.sh examples` gate
covers it (no per-library `examples.sh` to wire).
- **Validation per library:** gate green on that library **and** a deliberately
  deleted test makes a citation dangle.
- **Priority order** (non-obvious usage first, where a real call site pays most):
  1. **DONE** — `arguments` (`@ARG-001..004`), `gridmesh` (`@GRM-001..005`),
     `hex_grid` (`@HXG-001..006`) —
     stateful/geometry APIs where the shape of a correct call is the whole
     question. Confirmed by the two done: every tag is a contract a caller can get
     wrong while type-checking (a forgotten `clear_dirty`, a halo cell emitted
     twice, a stale cell index, a group rebuilt whole, a `-1` read as null).
  2. **DONE** — `crypto` (`@CRY-001..006`), `server` (`@SRV-001..003`), `web`
     (`@WEB-001..003`): protocol/sequence APIs where the order of calls matters.
     The tier sharpened what "gettable wrong while type-checking" means. For
     `crypto` the whole surface is `text -> text`, so the *encoding* is the
     contract and the compiler is blind to all of it. For `server` and `web` it is
     the RETURN VALUE that carries the contract — `serve_range`'s false, `send`'s
     false, `byte_at`'s `-1`, a non-null `WsHandler` — each a well-typed answer
     that means something other than what a caller assumes.
  3. **DONE for `loft-libs-graphics`** — `graphics` (`@GFX-001..005`), `shapes`
     (`@SHP-001..003`) and `imaging` (`@IMG-001..004`), which closed that repo.
     Remaining across the ecosystem: `markdown`, and `game_protocol` (already
     **exempt**). `loft-libs-core` is complete as of this pass.
     What tier 3 added to "gettable wrong while type-checking": the type is not
     merely uninformative, it is a *bare integer standing for a convention* — which
     byte of a colour is alpha, which end of a span is included, whether an answer
     of 0 means "transparent" or "not there". `imaging` also showed the tier's own
     payoff: asking what an example should ASSERT is what surfaced a shipped
     decoder bug that the library's round-trip suite structurally could not see.
  4. **DONE for `loft-libs-game`** — `time` (`@TIM-001..005`) and `input`
     (`@INP-001..004`) joined the `fixstep` (`@FIX-001..013`) that arrived already
     adopted. Tier 4's addition to "gettable wrong while type-checking": the type is
     a bare `integer` or a bare `text` naming a convention that is never declared —
     a milliseconds-vs-days unit, a bucket key that is not an instant, an ISO year
     that is not the calendar one, an action name no compiler ever sees.
  5. **DONE for `loft-libs-plugins` and `loft-libs-assets`** — `pluginabi`
     (`@PAB-001..004`), `mesh3d` (`@MSH-001..005`) and `glb` (`@GLB-001..004`).
     Tier 5's addition: a defect can show itself DIFFERENTLY through two doors of the
     same ecosystem, so an example belongs on each — one out-of-range vertex index
     shears a GL buffer through `mesh3d` and writes a byte-perfect but invalid glTF
     through `glb`, and neither symptom points at the other.
  6. `hex_terrain`, `hex_world`, in-repo `moros_*` — opportunistic (opt in when a file
     is finished; the ratchet only goes up), and blocked behind `loft-libs-world`'s
     hold.

  Off the priority list but done, because finishing a REPO is what earns a PR:
  `ssh` (`@SSH-001..002`) and `game_protocol` (**exempt**), which together closed
  `loft-libs-net`. Priority orders which library pays most; the PR unit decides which
  ones you actually finish first.

### Landing the rollout — one branch per repo, PR when the repo is complete

**The PR unit is the REPO, not the library.** A per-library PR buys nothing: the
gate is opt-in, so a half-adopted repo is already green, and each PR costs a full
CI cycle to land a doc-comment and a test file. Batching by repo is the same
bundle rule that governs plan closes, and it makes the PR mean something a
reviewer can judge — *this repo has adopted the convention* — instead of
*one more function got a comment*.

**One branch per repo, continued across sessions and hosts.** If a rollout branch
already exists, commit the next package onto **it** — never fork a second. A new
one is named `worked-examples`, with **no host prefix**: the branch belongs to the
work, not to the agent, and a host prefix is exactly what produces two
half-finished rollout branches in one repo when the next session runs on another
machine. (The two in flight predate this and keep their names:
`mac-worked-examples` in `loft-libs-core`, `tuxedo-worked-examples` in
`loft-libs-graphics`.) **Rebase onto `origin/main` before each package's commit** —
these repos publish from `main` while the rollout runs, so one package's worth of
divergence is the most that should ever need reconciling.

**Done, defined so it can be checked.** Every package (a directory with a
`loft.toml`) must carry one of three verdicts:

| verdict | recorded where | means |
|---|---|---|
| **tagged** | the generated `examples-index.tsv` lists ≥1 tag under `<pkg>/` | it has its worked examples |
| **exempt** | `examples-exempt.tsv` (repo root, hand-written) + a reason | no function here teaches more from a call site than from its signature — the convention's own no-retroactive-sweep clause, made explicit |
| **deferred** | `examples-exempt.tsv` + a reason naming what unblocks it | one does owe an example, not in this pass; it returns via the monthly by-hand review |

**Silence is not a verdict.** An untagged, unlisted package is TODO and holds the
PR — which is the whole point: without a recorded *exempt*, "all libs done" would
be indistinguishable from "nobody looked at the rest", and the rollout would ship
a repo whose quiet packages had never been judged. Zero TODO ⇒ open the PR.

```bash
make examples-progress REPO=../loft-libs-graphics   # run it ON the rollout branch
```

The report reads the working tree, so on `main` a package whose tags live on the
rollout branch reads TODO — check out the branch first. It **exits 0 always, is
not part of `all`, and library CI never runs it**: a half-adopted repo must stay
green, because the opt-in ratchet is what lets the rollout proceed one package at
a time without reddening a neighbour.

**Completion is frozen to the packages present when the branch opens.** A package
added mid-rollout adopts later through the same ratchet. Without that, a 14-package
repo (`loft-libs-world`) is a moving target that never converges.

**Where the ecosystem stands** (`make examples-progress REPO=…`):

| repo | branch | state |
|---|---|---|
| `loft-libs-graphics` | *(merged)* | **DONE + PUBLISHED** — 4 tagged, 0 todo; graphics 0.5.3 / gridmesh 0.2.1 / imaging 0.2.2 / shapes 0.4.1 in the registry |
| `loft-libs-net` | *(merged)* | **DONE + PUBLISHED** — 3 tagged, 1 exempt (`game_protocol`), 0 todo |
| `loft-libs-core` | *(merged)* | **DONE + PUBLISHED** — 6 tagged, 0 todo |
| `loft-libs-game` | `worked-examples` | **PR OPEN** ([#10](https://github.com/loft-lang/loft-libs-game/pull/10)) — 3 tagged (`fixstep`, `input`, `time`), 0 todo |
| `loft-libs-plugins` | `worked-examples` | **PR OPEN** ([#3](https://github.com/loft-lang/loft-libs-plugins/pull/3)) — 1 tagged (`pluginabi`), 0 todo |
| `loft-libs-assets` | `worked-examples` | **PR OPEN** ([#5](https://github.com/loft-lang/loft-libs-assets/pull/5)) — 2 tagged (`glb`, `mesh3d`), 0 todo |
| `loft-libs-world` | *(merged)* | **DONE** ([#15](https://github.com/loft-lang/loft-libs-world/pull/15) MERGED) — **14 tagged, 0 exempt, 0 deferred, 0 todo** — Phase E complete |

**The blocker is gone.** All four waited on loft
[#973](https://github.com/loft-lang/loft/pull/973) — the six registry rows **and** the
fix for the gate bug that reddened every library's `examples-index` step — because the
gate fires on `pull_request`, so none of them could go green before it merged. It has
MERGED, and `loft-libs-world` merged behind it. The three still open are open for their
own reasons now, not for that one; re-run each PR's checks before assuming otherwise
(measured 2026-08-25).

`loft-libs-net` was the convention's first complete repo, which is what makes the
"the PR unit is the REPO" rule reviewable rather than theoretical — a reviewer is
handed *this repo has adopted the convention*, with a recorded reason for the one
package that has nothing to demonstrate. `loft-libs-graphics` is the second, and it
reached zero TODO without needing the exempt column at all: each of its four
packages had something a signature could not say. **Opening a PR needs an explicit
ask** — the plan's "zero TODO ⇒ open the PR" sets the readiness bar, not the
permission.

### Phase E — `loft-libs-world`'s remaining twelve (S each) — **DONE, 12 of 12** (PR [#15](https://github.com/loft-lang/loft-libs-world/pull/15) MERGED)

The rollout's PR unit is the REPO, and `loft-libs-world` reached zero TODO the way
the mechanism allows: two packages **tagged**, twelve recorded **`deferred`** in
`examples-exempt.tsv`. That is a legitimate verdict, not a gap — but twelve is most
of the repo, and a verdict file beside the code is a good home for a reason and a
poor one for a work queue. So the queue lives here, and this plan stays open until
it is empty.

**Row 1 `hex_form` is DONE** (`@HXF-001..007`, acronym registered, `make
examples-progress` reads *3 tagged, 0 exempt, 11 deferred, 0 todo*) — and it paid the
convention's usual dividend in its sharpest form yet. The tag that had to say *"the
reader accepts exactly what the writer emits"* found that the reader did not. Its own
comment claimed strictness — *"anything it cannot round-trip must be refused rather
than repaired"* — and a nineteen-cell acceptance matrix showed the claim wrong seven
ways: a trailing space, an extra field, junk after the header, a non-integer length, an
empty length, an unnormalised `h0` and a non-integer `h0` were all admitted and then
re-written differently, so the byte-diff gate the whole format exists for could be
defeated by whitespace. Six were spelling; **`len x` was repaired to `len 0`** — a side
that is not there, admitted into the model without a word. Fixed at the one chokepoint
by a re-spelling identity (a field is an integer only if printing the parse reproduces
the original characters), a pure narrowing; `hex_form` 0.1.1. The same pass also
retired a test that could not fail — the corner-invariance assertion compared
`form_canon_text` of two *identical* forms.

**The generalisable half:** a doctrine sentence in a source comment is a CLAIM until a
worked example makes it a test, and the claim is likeliest to be false exactly where it
is most confidently written. `hex_fit`'s `draft_read` (row 8) states the same doctrine
over the same `word_int(…) ?? 0` helper and carries the same gap in its `wall` line —
so the finding was filed as a queue item rather than a one-off, and **row 8 collected it**:
ten repairs there against seven here, one of them a wall pointing somewhere else.

**Row 2 `hex_place` is DONE** (`@HXP-001..006`, acronym registered; *4 tagged, 10
deferred*), and it produced the same shape of finding from the opposite direction —
not a claim that was false, but a claim whose UNITS were unstated. `pose_residual` is
documented as *"~machine ε in float"*, which reads like a constant. Swept against the
evaluation point it is not: one 30-degree pose gives exactly `0.0` at (10,7),
`2.3e-13` at (1000,700), `0.0` again at (10000,7000) and `2.3e-10` at (1e6,7e5) — a
jagged, deterministic function of the point. So the natural way to calibrate a
tolerance — measure it once, near the origin, where the thing under test lives —
**reads exactly zero and concludes the transform is exact.** The comment now says
which half is load-bearing (*AT ONE WORLD POINT*), and the example pins the contrast
rather than any single value.

The row also delivered the cross-citation the table asked for: `@HXP-001` and
`@HXG-003` are one question at two scales — one wall between two hexes must resolve to
one stored slot; one seam between two *stencils* should not be stored at all — and each
names the other. The def scanner's first-tag rule keeps a sibling mention a mention;
verified by running the gate for both packages rather than assuming it.

**Row 3 `hex_roof` is DONE** (`@HXR-001..006`; *5 tagged, 9 deferred*), and its finding
is the same shape a third time, in its mildest and most useful form: a claim that was
TRUE but priced nothing. `roof_cone`'s comment says clamping at `dmax` *"is what makes an
eave LEVEL on a quantised footprint"* — correct, and silent about the cost. Swept across
`dmax` on a 29-cell quantised disc:

| `dmax` | `eave_spread` | `roof_ponds` | |
|---|---|---|---|
| ≤ 3.00 | 0 | 3–9 | level, and it leaks |
| 3.25 | 0.25 | 1 | both wrong at once |
| ≥ 3.50 | 0.5+ | 0 | drains, and the eave wobbles |

**No setting gets both**, and the sweep says why: the innermost *boundary* cell sits at
radius 3.00 while the outermost *interior* cell reaches 3.46, so no single radius
separates the two sets. `roof_hip` gets both because it never measures a radius — the
boundary is ring 0 by definition. The comment now carries the cost beside the cure.

**Row 4 `hex_way` is DONE** (`@HXY-001..006`; *6 tagged, 8 deferred*), and it is the first
row whose finding is a **wrong answer in shipped code** rather than a wrong sentence about
it. `track_offset` is the function the whole package exists for — a way stores one exact
centreline and every rail, kerb and platform face is this call at a different `d`. Its arc
branch read the sweep direction the wrong way up, so **every turn's offset landed on the
far side of the way from its straights**: a left rail that runs down the left of the
straight and the right of the bend, with a `2d` jump where they meet.

**What makes it a @PLN141 finding rather than an ordinary bug is what could not see it.**
The property everyone checks is equidistance, and equidistance holds *exactly* on the
wrong side — measured at 1.0000000000000009 for a requested 1.0. So:

| checker | verdict on the broken code |
|---|---|
| `hex_way`'s own conformance suite (`01-hex-way.loft`) | green |
| crawler's dedicated `I-EQUI` gate, over a straight+arc+straight track, 4 offsets × 21 samples | green |
| `track_distance` from any point of the offset | exactly `d` |
| the gap at the joint | **2d** |

The number that sees it is not a distance to the line at all — it is the distance between
the end of one offset segment and the start of the next. `@HXY-001` sweeps eight widths ×
two turn directions × two sides and asserts that gap is zero, and it also pins the
agreement between `seg_curvature`'s sign and which rail is the shorter one — the invariant
the bug violated. Fixed in `hex_way` 0.1.1 (one sign), NOT republished. Restoring the bug
turns two of the six examples red and leaves the conformance suite green, which is the
whole thesis in one run.

**Consumers, for the record** (reported here, not in their trees): moros' `hex_editor`
offsets only straights, so it was never affected; crawler's `platformtest` offsets a single
arc and calls the result "concentric — the right answer" without ever checking the side, so
its platform was on the wrong side of the curve and nothing said so.

**The row's second finding is a number the module header rounds off.** The header claims
rasterising a band "bottoms out around 0.5 hex widths". Swept, the floor is not one number —
it is the spacing of the cell centres *across* the way, so it is a function of the heading:
**1.5 down a row, 0.866 down a column**, a factor of `sqrt(3)` apart. 0.5 hex widths is the
best case quoted as the case. Measured, a 0.1-wide footpath and a 1.4999-wide lane along a
row rasterise to the *identical* 17 cells. The same anisotropy returns end-on in `@HXY-006`:
the shortest tread `way_steps` can carry is the spacing of the flight's own cells —
`sqrt(3)` down a row, 1.5 down a column — so there is no tread that is fine enough for every
heading, and 1.5 is exactly right along one and a double riser along the other. That
function's own comment already knew this for treads; the module header had forgotten it for
widths, one screen away.

**And a vacuous instrument, caught in the act.** The first tread sweep read *worst riser =
0* for every column-heading flight, which looks like a clean staircase and is not: at that
halfwidth the column footprint is **disconnected** — only every other row has a cell at
`x = 0` — so the riser instrument had no neighbouring pairs to compare and reported a
perfect result over an empty set. `@HXY-006` therefore asserts `adjacent_pairs(...) > 0`
before it measures anything. See [[absent-warning-is-not-a-pass]]: scoring on a channel
that can be silently empty ranks the most broken cell best.

**Row 5 `hex_shape` is DONE** (`@HXS-001..009`; *7 tagged, 7 deferred* — the repo is half
tagged), and it moved the finding one level up: from *the code is wrong* to **the package's own
INSTRUMENTS report pass on the exact failures they were built to catch.** Two of them, in
different modules, found the same way — by running the measurement on inputs nobody had run it
on.

| instrument | built to catch | reads on correct/broken input |
|---|---|---|
| `wall_along_max` | the "picket comb" — edges marked ACROSS the wall instead of along it, `\|dot\| ~ 1` | **0.97 on a correct in-between wall.** Only 3 of its 25 edges are above 0.9, but the statistic is the MAX |
| `set_connected` | a wall with a gap in it | **true on a ring with a cell removed** — a broken ring is still one connected chain, just not a loop |

Neither is a bug in the function: `wall_along_max` measures the wobble correctly and
`set_connected` answers connectivity correctly. What was wrong is that each was written up as
*the* verdict. On the twelve EXACT headings the along test tops out at 0.5 or 0.866 and nothing
reaches 0.9, so whoever calibrated it was right — and never ran it on the other twelve, where a
shallow line genuinely has to cross the vertical edge between two cells of the same row and
that edge's dot is `cos(13.9°)`. The verdict that holds for all 24 is the CHAIN property
(`wall_chain_ends == 2`, `wall_chain_branches == 0`), exact and tolerance-free; for the ring it
is `flood_outside` + `leak_count`, which on one cell removed from twenty-two reports the
outside reaching **all twenty-seven** courtyard cells. Both comments now say which half is
load-bearing, and `@HXS-004`/`@HXS-007` pin the contrast rather than the number.

**The package's own thesis, made numeric.** Every primitive takes a CONTINUOUS parameter the
lattice cannot represent continuously, so every answer is split — which is the row's brief and
it held everywhere it was checked:

- an arc's **centre** is exact; its **radius** is a grid. Over 0.5–4.5 world units there are
  exactly FOUR admissible radii (shells 0, 12, 36, 48), and `arc_fill(…, 16, …)` draws shell 12
  and reports 12 — the 16 is stored nowhere and nothing says it was discarded (`@HXS-001`).
- a run's **line** is exact; **which end you called the start** is not — the field cannot store
  an orientation, so `wall_read_run` returns `d` or `d + 12` with the other endpoint as anchor.
  Writing `d = 0` reads back `12`, so the obvious round-trip assertion `read.d == d` fails on
  the very first direction (`@HXS-005`).
- twelve directions are **exact** and twelve are **1.1021° off**, and only three values of `N`
  occur across all 24 with no pair commensurable — which is the integer reason `WALL_W` is a
  world-unit constant and not a count of lattice rows (`@HXS-002`).
- a legal run length depends on the **starting corner**: `wall_min_p` is 1 from one vertex of a
  hex and 2 from the next, for the same direction (`@HXS-003`).
- the twelve box angles are **two families of six** — 33 cells against 31, related by no exact
  map (`@HXS-006`).

**A stale number is a claim too, and one change left three.** Moving the in-between direction
vector from `N = 21` to `N = 39` updated the design block that argued for it and left three
downstream comments quoting the old consequences: the angle error as "~4.11 degrees" (measured
1.1021), and the δ = 0 class as "the N=21 in-between" (measured 39). All three are one screen
apart from the block that superseded them. The lesson is narrow and repeatable: **when a value
changes, grep for its CONSEQUENCES, not for the value** — 4.11 does not contain the string 21.

Coverage went from 4 of 83 public functions entered to **all 83**, which is what pushed the
last two examples (`@HXS-008` the mouse's three stacked snaps, `@HXS-009` the door as an
annotation) into existence: they were not on the plan's brief and the coverage hole is what
named them.

**Row 8 `hex_fit` is DONE** (`@HXI-001..008`; *8 tagged, 6 deferred*) — the first row taken
out of the table's ORDER, because row 1 unblocked it and it inherited a filed finding. It
repaid that twice over: the finding arrived exactly where row 1 said it would, and a second
one was waiting underneath it that nothing had predicted.

**The predicted one first.** `draft_read` states row 1's doctrine — *"the reader refuses
anything else rather than repairing it"* — over the same `word_int(…) ?? 0` helper. A
nineteen-cell acceptance matrix admitted **ten** texts `draft_write` cannot emit and re-wrote
each of them differently. Six were spelling (a trailing space, an extra field, `+3`, `003`, an
empty field from a double space, a `wall` line placed between two `side` lines). The other
was not: `wall d x a -6 b 9 p 1` parsed as `d 0` — where row 1's `len x` was *a side that is
not there*, this is **a wall pointing somewhere else**, and `a x` moves it instead. Same
chokepoint, same re-spelling identity, plus the two structural rules the writer obeys (nine
fields; the wall line LAST). A pure narrowing: `d 99` and `p -1` still read, because a
spelling gate is not a doorstep — which became `@HXI-004`.

**And the one nothing predicted, which is the row's real finding.** The format's premise is
that a `diff` means *the model changed*, and that needs every model to have exactly ONE text.
The form half has had that since `form_canon`. The RUN half never did, and a run has exactly
two spellings — `(d, A, p)` and `(d + 12, B, p)` — because A-to-B and B-to-A mark the same
edges and the field stores no orientation. Measured over all 24 directions, both spellings of
each wall:

| measurement | result |
|---|---|
| edges the two spellings differ in | **0** of 48 |
| spellings of a wall that survive the text round trip | exactly **one**, 24 of 24 walls — never both, never neither |
| `draft_write` == `draft_rebuild_text` | 24 of 48 |
| the doorstep admits it | 48 of 48 |

So an author who wrote the wall from its other end got *the model changed* out of a byte diff
on a model that had not changed at all — and by law C1's own wording (*fits? must agree with
whether the model round-trips; a FALSE ACCEPT is the dangerous direction*) **half of every
legal run is a false accept**. The cure is the canon the form half already had:
`draft_canon` / `draft_canon_text`, with the canonical spelling **read off the field** — it is
what the reader answers, so it cannot drift from the reader the way a rule computed beside it
could. All 48 spellings now canonicalise to one text, stable across two chunk geometries.

**What hid it is worth more than the bug.** The workshop's own round-trip gate covers this
exact call and is green, because it builds its expected text from `wall_read_run`'s answer
rather than from what an author would write — a workaround for the defect, written into the
gate, in the one place that would otherwise have caught it. Row 5 found instruments calibrated
on a sample that excluded their failure cases; this is the next form of the same thing: **a
gate that has already routed around the defect it was built to find.** The question to ask of
a passing gate is therefore not only *what does it measure* but **"what does it CONSTRUCT
rather than take from the caller?"** — every such step is a place the caller's mistake cannot
reach.

A third, smaller finding: `arc_fit_n`'s comment says the author "sees both candidates' cost",
and the API carried only one of them. `arc_snap_n` / `arc_snap_residual` supply the other —
what an unrefused radius actually draws — validated against `arc_fill` over every radius 0..64
rather than against the shell formula it would otherwise share with `arc_fits`. The prices are
not close: `N = 35` is offered 36 at a residual of 1 and would silently have drawn **12**, an
error of 23, because 12 and 36 are neighbouring shells. hex_fit 0.1.1, not republished.
Coverage 8 of 30 functions entered → all 37.

**Row 9 `hex_draw` is DONE** (`@HXD-001..007`; *9 tagged, 5 deferred*), and its finding is the
first one in Phase E that could never have been true. `surface_miter`'s comment states a
RECOVERY CHECK — *"the intersection of the two averaged lines must land on the **exact model
corner**, or the surfaces have drifted off the shape they were fitted to"* — and nothing had
ever evaluated it. Measured on the 5×4 cottage it misses by 0.479 world units at every corner;
swept over plan sizes 2..8 the miss is 0.465..0.827, and it is **sometimes outside the plan
rectangle and sometimes inside it**, so it is not even an offset a caller could correct for.

**The reason is structural rather than numerical, and that is what makes it a @PLN141 finding.**
These surfaces recover the face of the wall AS DRAWN, and the drawn footprint is the plan
quantised to cells. So the recovered corner is an exact rational of the lattice — every swept
value is a multiple of 3/4 across the rows — while the plan's half-depth is an irrational
multiple of `sqrt(3)/2`. The two agree only at the origin. **The claim compares numbers from two
different systems**, which is the same disease as `hex_grid`'s two lattices sharing a spelling
(`@HXG`/`@HXW`) and `hex_way`'s treads, one level up: not a wrong value but a wrong KIND.

A second claim was orientation-luck, and it had a green test on it. `surface_fitted_spread`'s
comment said the fitted quad's spread *"is exactly 0"*, and the conformance suite asserted
`== 0.0` — at one orientation. Over 12 orientations × 4 sides, **32 read exactly 0 and 16 read
2.22e-16**: the derivation is exact (integer sums, rational means) but the span is projected
through world coordinates, and where that arithmetic does not cancel the round trip costs an
ulp. Both the comment and the conformance assertion now compare against the number that means
something — flat against the STRIP's own band, fifteen orders of magnitude larger — and the
tolerance-free exactness stays where it belongs, on the integer cross product in
`surface_heading`.

**Three claims held, and measuring them was still worth it**: the two wrong ways to write the
exactness test reject 16 of 24 and 12 of 24 surfaces, exactly as recorded; a band-of-cells wall
would eat 16 of the cottage's 27 floor cells; the shortened gable ridge rolls its end over by
`sqrt(3)/4` = 0.375 m. The edge-count undercount held too and came out **sharper** than
recorded: the same three-directions shortcut reads 19 in `hex_grid`'s neighbour order and 17 in
`hex_field`'s, both plausible beside the true 38 — a second instance of the convention hazard
this repo warns about in four separate comments.

**And the row caught its own instrument being vacuous, mid-probe.** Measured over the massing
alone, the two ridge rules agree to the last bit — because no cell of the massing is past the
ridge's end. The entire difference lives in the halo ring `grow_ring` adds. That is row 4's
tread sweep again ([[absent-warning-is-not-a-pass]]), and the example now asserts the halo is
there before it measures anything. Coverage 3 of 26 public functions entered → all 26, and the
last two came from reading the coverage list rather than the brief.

**Row 6 `hex_terrain` is DONE** (`@HXT-001..008`; *10 tagged, 4 deferred*), and the fixture
that unblocked it turned out to be the cheapest possible one: a hand-authored ramp,
`h(c,r) = 100c + 10r`, 500 m tiles, a sea column and one pit — every number in the eight
tags is arithmetic that can be redone on paper, which is what made the finding legible.

**The finding is a NAME, and the name is the load-bearing part of the doc.** The README and
the module header both call the package's headline invariant **window independence**:
*"every sample is a pure function of `(terrain, types, params, rivers, x, y)`"*. Read as
written it is true, and the conformance suite holds it — by building the same world from the
same seed twice and comparing samples, which measures DETERMINISM. Read as named it says the
answer does not depend on the window you generated in, and the consumer who reads it that way
is exactly the one tiling a large world.

Measured on one authored ramp in a 5×5 and an 8×8 window:

| measurement | 5×5 | 8×8 |
|---|---|---|
| accumulation at cell (2,1) | 5 tiles | 11 tiles |
| that cell is water (`tp_acc_min` = 6) | no | **yes** |
| its relief | 154.0 | **0.0** (water takes none) |
| interior sample points that differ | — | **24 of 27**, by up to **15.1 m** |
| river courses | **0** | **5** |

Hydrology is a global pass and accumulation is a catchment property, so widening the map
moves the rivers and the fine surface follows through the blend kernel. There is nothing to
fix in the code — accumulation cannot be local — so the cure is the doc: which half is pure,
which half is global, and the rule that follows (*generate the overland map once at its full
extent; a window is a unit of sampling, never a unit of generation*).

**Two more contracts that a signature cannot carry, and both are exact.** A cell's own centre
reads **99.1034 m** where 100 m was authored, because the blend kernel has radius `1.02·tile`
and therefore always overlaps the six neighbours — that overlap IS the seamless merge, and
`100/(1 + 6w)` with `w = (1 − 1/1.02²)²` is its exact price, with the six neighbours picking
up exactly what the cell lost. And a pit authored at 5 m returns from hydrology at **110.05**
— its lowest rim plus the flood's epsilon — reclassified as a LAKE, with nothing in the record
saying a pass has run.

**A method note this row adds.** The finding was reached by ATTRIBUTION, not by inspection:
the two windows were compared after each pass in turn (authored → hydrology → relief), which
put the divergence on `terrain_relief_pass` reading a wetness flag that hydrology had set from
a global count. Comparing only the endpoints would have said *"the surface differs"* and named
nothing. Same instrument the profiler lesson asks for — measure where the effect ENTERS, not
that it exists.

**Row 7 `hex_body` is DONE** (`@HXB-001..008`; *11 tagged, 3 deferred*), and it is the row with
a **shipped wrong answer** in it — the second in Phase E after `hex_way`'s offset, and this one is
silent in a way that reaches a consumer's collision layer.

`rig_world_seg` composes joints by ADDING ANGLES, so it reads neither `oz` nor the stored revolute
axis. Hand it a rig hinged about `+y` — perfectly admissible, built through the library's own
`rig_bone3` — and it answers as though that hinge were about `+z`:

| joint value | the 2-D walk says | the bone actually is | apart |
|---|---|---|---|
| 0.25 | (3, 2) | (3, 0, −1.5) | **2.5** |
| 0.75 | (3, −2) | (3, 0, 2.5) | **3.2** |

on a bone **2 long**. And `bone_obb` / `bone_shape_has` are built on that same walk, so the
collision proxy boxes empty space and `I4`'s containment — *a proxy never misses an overlap* —
quietly stops holding. The predicate that answers the question, `rig_planar`, already existed and
its own comment names it (*"the question `rig_world_seg` implicitly asks of every rig it is
handed"*) — but the three functions that ask it implicitly never mentioned it. **A precondition
known to the author and absent from the call site is not a precondition**, and that is this row's
generalisable half.

**The second finding is the family's pattern for the third time.** Above `rig_read` stood *"STRICT
PARSER — accepts exactly what `rig_write` emits … A lenient reader would void the byte diff."* It
was wrong nine ways — `len 1.0`, `+1.5`, `01.5`, `1.50`, `1.5e0`, an empty field, a trailing space,
an extra field, and `len x` → `len 0`, which in `hex_form` was *a side that is not there* and here
is a BONE that is not. Same chokepoint, same cure (a re-spelling identity plus a field count),
hex_body 0.3.1. **Three packages, three independent authors of the same sentence, three times
wrong** — which retires any doubt that the doctrine sentence is where to look first.

The narrowing exposed two hand-written fixtures in the package's own suites, one of them the
CONTROL of the unknown-record test: it had been passing on a text the reader was about to stop
taking. That is the third time in this plan that a fix's blast radius landed on a test written to
prove something else.

**And one claim corrected rather than enforced.** The design block said the parser refuses *"a bone
out of order, or referring to a forward/absent parent"*. It refuses the first and not the second —
and it is right not to: a forward parent round-trips FAITHFULLY, so it is not a spelling question,
and `rig_admissible` is the doorstep that catches it. That is exactly the split `hex_fit`'s
`@HXI-004` draws, so the same reasoning now appears in both packages, and `@HXB-008` measures both
halves — including what posing an inadmissible rig does, which is read a parent frame that does not
exist yet and get zeros.

**Row 10 `hex_field` is DONE** (`@HXL-001..012`; *12 tagged, 2 deferred*) — the repo's largest
surface, 82 public functions, and the one the table had flagged as *"a phase, not a row"*. It was
a row. The tags do not track functions; they track the package's load-bearing promises, and there
were twelve.

**The blocker dissolved the same way rows 6 and 7's did.** The table said row 10 waited on
*"choosing which golden fixture is small enough to read inside a test"* — the package has a real
oracle, a Python implementation whose golden JSON it must reproduce byte for byte. The answer was
not to shrink that fixture. It was to write one: a hex traces to six named lattice points,
`(-1,-1) (0,-2) (1,-1) (1,1) (0,2) (-1,1)`, and shoelaces to exactly 12. The entire
representation fits in one assertion, and the two hexes and the ring of six that follow are the
smallest forms carrying an adjacency and a hole. **Three for three now: "small enough to assert
against" has meant WRITTEN BY HAND every time, and a shrunken real thing has never been the
answer.**

**THE FINDING IS A PREMISE, and it is the first one in Phase E that was true of the author's
shape and false of the package's own doctrine.** `validate` — *"what must hold before a renderer
or mesh may consume it"* — read `outer != 1`, exactly one outer loop. That is a property of a
CONNECTED form. This package's README says the opposite about itself in its own section heading:
*"Bounded chunks on purpose … the same code serves a 32×32 world chunk and a one-off tower
window"*, and *"forms spanning chunks are traced per chunk with a halo and stitched"*. A chunk
holding two buildings, or one building the chunk edge cut in half, is therefore the ORDINARY case
— and every such map was refused with code 5 while `trace` had produced it correctly and the
exact integer area agreed: two disjoint hexes are two loops of 12, and 24 is 24. Measured on
three shapes (two hexes, two donuts, two columns), all refused, all correct.

The cure is where it gets interesting, because *weakening a gate until it passes* is the exact
anti-pattern row 5 was about. `outer >= 1` does lose something real — a hole wound the same way as
its outer loop, compensated by an extra outer loop, so the signed areas still sum. Deciding that
needs each loop's NESTING, which the sign count never actually computed; the old rule only appeared
to check it because it forbade the multi-component case outright. So the honest split is: code 5
means *no* outer loop, `outline_count(v)` is exposed, and a caller who KNOWS its form is one piece
keeps the stronger property by asserting on it. **The general shape: when a check both refuses
correct data and catches something real, the answer is to separate the two claims and give the
stronger one to the caller who has the knowledge it needs — not to pick one.**

**Five more, all one family — a rule enforced on part of its surface:**

| what | the number that shows it |
|---|---|
| a material past the slot DELETED the wall it was setting | the slot is a `u8` and the narrowing cast is CHECKED, so `?? 0` behind it wrote the one value that means NO WALL: `edge_set_mat(e, …, 300)` read back as open ground, and 257 did too, so it was not a low-byte truncation anyone could reason about |
| `validate` never checked for a repeated vertex, which its README listed | two hexes emitted as two circuits joined at their shared corners: twelve legal hex-edge segments, shoelace exactly `12 × 2`, one positive loop — every other check passes it |
| `stencil_rotate` / `stencil_mirror` carried each edge's MATERIAL and dropped its SURFACE | six turns returned the cells and materials exactly and the geometry as 0, under a comment reading *"A stencil must never lose data"*. The whole surface half of `EdgeSet` — five public functions — was uncovered by the suite |
| `doc_read` took `w`/`h` on trust | a 32-byte file claiming 4000×4000 allocated sixteen million cells, plus heights, labels, edges and layers over the same extent, and only then reported a missing section. Reverting the fix now trips the **2 GiB store ceiling at 1.9 GiB** — the ceiling is the measurement |
| the `EDGE` section copied the source layer's vector into a length derived from the FIELD | an `EdgeSet` at another extent has a different stride, so every material landed on a DIFFERENT edge: the file loaded with code 0, the wall count was right, and the walls were somewhere else |

**And a units finding, the `hex_place` shape again.** `form_hexdisk(w, n)` counts hex STEPS;
`form_circle(w, radius)` and `form_octagon(w, apothem)` measure LATTICE WORLD UNITS, the ones
`x = k·√3/2, y = m/2` defines, in which a hex has circumradius 1. So one hex step is √3 ≈ 1.732
world units, and the same `3` asks for 13 cells or 37. The README's own scale section said every
threshold was *"dimensionless — hex steps or pure ratios"*, which sends a reader to the wrong one
of the two. Both units are now named at both call sites, with the conversion.

**Two side effects worth recording, because both generalise.**

`hex_fit`'s `@HXI-007` (row 8) had **pinned the erasure as behaviour** — *"256 ERASES the wall — it
does not wrap to 0 by luck"* — and row 10's fix turned that test red. That is the convention
noticing its own subject move, which is the whole point of pinning; the tag's thesis (a material id
is nominal, so its refusal offers nothing) survived unchanged and only the measurement behind it
moved. **A worked example that documents a defect as behaviour is a bookmark, and the day the defect
is fixed the bookmark is what tells you.**

And a scratch file: `file(path)` opens an existing file to **APPEND**. A test that died before its
own `delete` therefore handed the next run a header with a header in front of it — so the failure
landed in a *different* test from the one that caused it, which is how a five-minute check becomes
an hour. Both `tmp()` helpers in this package now clear the path before naming it. **A test's
cleanup running only on the success path is not cleanup.**

**And the row's seventh finding was upstream.** Both `stencil_rotate` and `stencil_mirror`
opened with `if !seen { return st; }` — a stencil with no occupied cells has no bounding box to
fit, so it came back unturned. That path is essentially never taken, and it leaked a `Stencil`
**on every call**: 2000 rotations plus 2000 reflections left 4000 stores unfreed, in a published
library, in the operation an editor performs constantly. The cause is a loft ownership seam — a
function whose return paths disagree about ownership (the by-value parameter on one, a freshly
built value on the other) never frees the fresh one, both backends — filed as **loft#982** with a
twelve-row boundary matrix and a verified clean workaround, and the same seam as loft#978 in the
opposite direction (that one over-frees a view; this one under-frees a fresh value). The library
took the workaround: a zero-turn copy for the rotation, a `flip` flag for the reflection, measured
at 4000 stores before and zero after.

**Why it took a worked-example pass to see a leak that had been there all along:** `loft test`
runs the store-leak check only under `--interpret`, and it reports at PROGRAM exit — which a test
suite never reaches with a stencil still live. The leak was visible only from a standalone probe,
and the reason to write one was that @HXL-007 needed `edgeset_equal` over a rotated stencil, which
is what put a rotation in a loop in the first place. **The tag did not find the leak; wanting to
assert the tag did.**

**A harness note, from this row's own non-vacuity pass.** The first run of the
restore-the-pre-fix-code channel reported *no failures for all six reverts* — because the loop was
missing the line that applied the revert. An empty result set read as "nothing broke". The pass was
re-run with two guards: fail loudly if the patch script errors, and `cmp` the file against the
fixed copy and skip the case if the revert was a no-op. **Prove the harness can fail — and for a
harness that reports by ABSENCE, that means asserting the mutation actually happened, not just that
the run finished.**

**Row 11 `hex_recover` is DONE** (`@HXV-001..009`, acronym registered; *13 tagged, 1 deferred*),
and its blocker dissolved by being MEASURED rather than scoped. The table said row 11 waited on
*"deciding what a fast, readable subset of that census looks like — the full one is a long-running
test"*. There is no subset to choose, because two different censuses had been conflated. The open-
ended one — push the level ladder up and see where injectivity stops — is hexbody's exploration and
belongs there. The one a CALLER needs is finite and already fixed by the build: **is `draw`
injective over the set this build matches against?** That set is 119 forms, and `index_build`
decides it for the whole space at once (two candidates sharing a digest overwrite each other in the
map, so the build counts the clashes) in **18 ms** — which the row's own fix then took to **2 ms**.
*"A census is slow"* was a property of a question nobody was asking.

**THE FINDING IS A LIMIT INHERITED FROM THE CODE IT REPLACED, and it is the first in Phase E where
the step that makes an answer trustworthy is the step that throws it away.** `rebuild_construct` is
the package's headline: it reads `(h0, lens, turns)` off the field's own convex hull, *"enumerating
nothing … in O(cells), independent of how large the admissible space is"*, and then re-draws once —
*"the construction PROPOSES a form; one re-draw CONFIRMS it. That keeps `ρ = 0` a measured fact
rather than a claim."* The redraw went into `FW/FH/FQ0/FR0`, a constant 25×25 window whose own
comment reads *"a compact window: **every level-1 shape** is a handful of cells around the anchor"*
— true of the enumeration, and false of the thing built to replace it. `form_fill` clips to its
chunk silently, so:

| form | cells | verdict | ρ |
|---|---|---|---|
| heading 0, side 12 | 91 | R1 | 0 |
| heading 0, side 13 | 105 | **R2** | 2 |
| heading 2, side 13 | 105 | **R2** | **196** |
| heading 3, side 7 | **85** | **R2** | 3 |

Every one of those had the correct form sitting in `rb_form` already; `rebuild_construct_text`
returned `""`.

**Three numbers say what could not see it.** First, `ρ = 2` on a 105-cell field reads like an
almost-perfect fit — the natural reading of a small residual is *"nearly a stencil"*, and it was
measuring the distance from the anchor to the edge of a constant. Second, `ρ = 196` on that same
105-cell field is **more unexplained cells than there are cells**, which is a number that cannot
mean what its name says: the clip moved the translation-normalisation origin by one, and one is
the wrong parity, so not a single cell of 105 lined up (105 + 91 = 196, hand-computed and
confirmed). Third and sharpest, the **85**-cell shape refused while the 105-cell one passed. The
limit was never a size. An odd (vertex-direction) heading covers two rows per unit of side and an
even one covers one, so the same stencil turned reached the window at half the length — and a
stencil and its orientation-image are ONE stencil by this package's own law **I**. A recovery that
depends on which way a stencil faces is not a recovery.

Fixed at the one chokepoint: the window is now DERIVED from the form (`fit_chunk`, from
`form_poly_k`/`form_poly_m` — every `head_step` moves `m` by a multiple of 3, so `r` is bounded
exactly and `q` within one, with a one-cell pad). All three redraw sites take it, so `rebuild_with`
and `index_build` lose the same latent trap the day `LEVEL` is raised. It is a pure narrowing and
it is *faster*: the derived window is far smaller than the constant for the shapes that ship —
70 728 fewer cells scanned across the candidate set, index build 18 ms → 2 ms. `hex_recover` 0.1.1,
API additive.

**The invariant the tags pin is not the fix.** On the verified path the redraw must CONTAIN the
field — so ρ counts only what the hull ADDS and never what the chunk dropped. `@HXV-003` asserts
that at all twelve headings, and carries the old 25×25 window inline as its own CONTROL: the same
containment check against it must report a drop at every one of the twelve, or the assertion
proves nothing. Which makes the residual readable at last: a ring of six scores **ρ = 1**, exactly
the centre cell its hull adds — the smallest positive value there is, and a flat refusal, because
no grammar form has a hole. `ρ` is a count, not a distance; it does not shrink towards a match.

**A second, upstream finding, and it is loft#982 again — one row later, in a different package.**
`hex_form`'s `form_canon` opened with `if n == 0 { return f; }` — the by-value parameter on one
path, a freshly built `Form` on the other — and leaked one store per call on the path that IS
taken. Every enumerator calls it once per admitted form, so the census leaked 1182 `Form`s on a
single `candidate_forms()`. Row 10 filed the seam from `hex_field`'s stencil transforms; row 11
found the second victim in the family without looking for it. Same verified workaround (build a
fresh value on every path), measured 100 leaked → 0, `hex_form` 0.1.2. **The generalisable half is
the search key, not the bug: `return <by-value struct param>;` beside a `return <fresh>;` is a
grep, and it is worth running across a repo the first time the seam bites it.**

**A note on where the row's questions came from.** The productive question was the one rows 1–10
converged on — *"what is the nearest call that looks identical and is not?"* — and this package
answers it in its own comments: three digests, all `HexSet -> vector<integer>`, and the file says
using the wrong one *"reported 17 false law F failures on a 10-entry corpus before the distinction
was drawn."* `@HXV-001` is that paragraph made executable; it also pins one digest as literal text
(`640400.642003.643600`, the unit triangle's three cells at `(0,0)`, `(2,0)`, `(1,3)` under
`(k+400)*1600 + (m+400)`), because *"EXACT, NOT HASHED"* is only a claim until a number you can
derive by hand appears in a test.

**Non-vacuity, nine for nine.** One targeted library mutation per tag — `field_exact` aliased to
`field_norm`, the index keyed on the cell count, the constant window restored, the hull refusing
4 sides, the verify accepting any residual, the cyclic test frozen at shift 0, the mirror test not
reversing, `ri_fills` not counting, `forms_at_level` reading `<=` — each ran the suite and each
turned it red naming its OWN test. The harness errors out if a mutation pattern is not found and
`cmp`s the file to prove the edit landed (row 10's lesson, applied rather than re-learned).
Coverage 17/46 → **46/46**.

**Row 12 `hex_edge` is DONE** (`@HXE-001..011`, acronym registered), and **Phase E is
complete**: `make examples-progress REPO=../loft-libs-world` reads *14 tagged, 0 exempt,
0 deferred, 0 todo*. `examples-exempt.tsv` is now empty of verdict rows.

**The blocker was an owner decision, and the measurement refuted BOTH options it offered.**
The queue held row 12 on *"whether `hex_edge` should offer a `sweep_path_skin(…)` … or every
caller keeps its own skin"* — the resting-position trap moros#10 reported: a caller must not
come to rest exactly at the fraction `sweep_path` returns, because that point is on the
bisector and the next `hex_at` may round to the far side. Two things came out of measuring it
rather than reasoning about it.

**First, "may round" is wrong — it rounds to the far side EVERY time.** Over all six
directions, a head-on stop leaves the position at exactly `t = 0.5`, `hex_at` there names the
FAR cell six times out of six, and the very next sweep then reports `dir = -1, t = 1.0`: the
whole segment clear, straight through the wall. It is not a rounding coin-flip that bites
sometimes; it is deterministic, and the consumer's *"collision does not work"* is the exact
truth — it worked for one step.

**Second, the skin both options depend on has no correct value.** The smallest backoff that
works is not a geometric clearance, it is a float-resolution floor, so it scales with where in
the world you are standing:

| where | smallest skin that works |
|---|---|
| at the origin | 1e-15 |
| ~1.7e3 world units out | 1e-11 |
| ~1.7e6 world units out | 1e-9 |

A constant calibrated at the origin fails 18 of 30 cases at 1.7e3 units out. So *"the library
offers a skin"* and *"every caller keeps its own"* are the same wrong answer wearing two hats
— neither party can pick the number, and moros's 1 cm is safe only because it is enormously
larger than the floor at moros's extent, which is luck rather than reasoning.

**The exact option neither branch considered: `sweep_path` already RETURNS the cell.** It
hands back `(t, cq, cr, dir)`, and the ambiguity exists only because the next call throws
`(cq, cr)` away and re-derives it from a float. So the cure is to thread it —
`sweep_path_from(e, cq, cr, …)`, added as a purely additive `pub fn` with `sweep_path` kept
byte-identical in behaviour as the `hex_at` wrapper over it. Pressing on into the wall then
answers `t = 0` in the cell you are in, which is the honest answer, and it needs no tolerance
anywhere. Verified across all six directions and at four extents: exact from the origin out
to 3e6 cells. `hex_edge` 0.2.0.

**And the far-field limit, which is the honest part.** Past roughly 5e6 cells the threaded
version degrades too — so it is not a property of the API choice. TWO in-algorithm
explanations were tested and BOTH refuted: scaling the epsilons relative to the coordinate
magnitude made it *worse* (8 of 30 failing at 1e3 cells, where the shipped code is clean), and
recentring the bisector solve on the current cell's centre — which does make the crossing
parameter exactly 0.5 at every extent, against a shipped drift of 8.6e-9 at 1e8 cells — moved
the boundary not at all. What survives is measured directly: the offset a double position can
recover at that magnitude loses 2.9e-10 at 3e6 cells and 6.4e-10 at 1e7, crossing the code's
1e-9 tolerance exactly where the walk-throughs start. **The position itself is the limit, and
the answer is to recentre the world, not the arithmetic.** No fix was shipped for it, because
the fix would have been for something that is not the cause.

**A second finding, and it is left as BEHAVIOUR rather than fixed, because fixing it is
another owner decision.** `material_set_solid` is documented as the dynamic-material
mechanism — *"a level-crossing barrier, a door, a portcullis … flipping the table entry
retargets every edge already carrying this id"* — and **movement does not read it**.
`passable` and `sweep_path` take an `EdgeSet` and no `Materials` at all, so lowering `solid`
to false leaves `passable` false and the sweep stopping at exactly the same `t = 0.25`.
Raising the portcullis does not open it. Meanwhile `sight_clear` *is* material-aware, so the
package has one query that honours the material vector and one that cannot see it; of the six
transmission terms, two (`opacity`, `height`) change an answer here and four are data for a
consumer. `@HXE-010` pins the asymmetry as behaviour — a bookmark, in row 10's sense — and
the API question (should movement take a `Materials`?) is recorded, not answered.

**Three hypotheses were tested and refuted in this row and none became a finding.** Besides
the two above, `sight_clear`'s 0.2-unit sampled walk was checked against the claim in its own
comment — *"a hex inradius is 0.866, so consecutive samples cannot skip a cell"* — over 600
sight lines and 7644 cell transitions: **zero** non-adjacent hops, and zero disagreements with
the exact sweep over 400 crossing lines. The claim is true and now measured. Worth recording
because the row's write-up would read better with a fourth defect in it, and there wasn't one.

**Non-vacuity, eleven for eleven**, each mutation naming its own test. Coverage 25/40 →
**40/40**.

**A harness note.** The first full both-backend sweep reported `hex_roof` red on native while
the mutation harness was running concurrently in the same repo — the harness rewrites a source
file eleven times, and a native build that reads a tree mid-write fails for reasons that have
nothing to do with the change under test. Re-run serially it is green, as are all fourteen
packages on both backends. **Never read a full-suite verdict that was taken while something
else was editing the tree.**

**A method note worth keeping.** All twelve rows found their defect in the tag that had to
demonstrate the package's most confidently stated promise — a claim that was
false (`hex_form`), one whose units were unstated (`hex_place`), one that was true and
priced nothing (`hex_roof`), one the code simply did not implement (`hex_way`), two
CHECKS that answered *pass* on what they were written to fail (`hex_shape`), a
premise the package enforced on one half of its own text and not the other (`hex_fit`),
one that was true of the shape its author had in mind and false of the shapes the
package exists to hold (`hex_field`), one true of the CONSTRUCTION and false of the
ROUTINE, because the step that verifies it kept a limit from the code it replaced
(`hex_recover`), and one hedged as *"may"* that turns out to happen every time
(`hex_edge`).
That is not luck: a worked example is the first thing that ever evaluates such a sentence
against the code, and the sentences most worth working are the ones written with the most
certainty.

**And a second note, about what these packages have in common.** Every finding so far is a
call that produces a plausible wrong answer while passing every cheap check — a parse that
repairs, a residual that reads zero, a roof that still sheds water, a rail that is exactly
the right distance from the wrong side. So the productive question for every row after it
is not *"what does this function do"* but **"what is the nearest call that looks
identical and is not, and what number distinguishes them?"** In `hex_roof` that number
already existed and was simply not the one anyone read (`eave_spread`); in `hex_way` it had
to be constructed, because a joint gap is not a quantity anyone had thought to name.

**The sharpest form of the rule, from row 4:** the property a package is *most* careful to
guarantee is the one least able to catch its own violation, because everyone — the library's
tests and the consumer's gate alike — checks the guarantee and nobody checks what the
guarantee is silent about. Equidistance does not pick a side.

**And row 5 turns it on the checks themselves.** A gate is calibrated on the inputs its author
had, and its threshold silently inherits that sample: `wall_along_max` separates a chain from a
comb beautifully on the twelve headings anyone tried it on, and not at all on the twelve they
did not. So the question to ask of any instrument is **"what is the widest input it was
calibrated against, and what does it read just outside that?"** — which is a sweep, and it is
the same sweep rows 3 and 4 needed for a parameter. An instrument is a claim with a number
attached, and it decays exactly like the prose does.

Nothing here is `exempt`. This is geometry, the tier where a call site teaches most
and a signature carries least; every row below is a package that owes an example.

| # | package | pub fns | what its example must teach | blocked on |
|---|---|---|---|---|
| ~~1~~ | ~~`hex_form`~~ | 53 | **DONE** — `@HXF-001..007`. Rules **C1–C5** worked one by one; writing them found and fixed a reader that repaired seven texts it documents as refused | — |
| ~~2~~ | ~~`hex_place`~~ | 17 | **DONE** — `@HXP-001..006`. The shared edge, order-freeness, levels, seating, the seam error and arbitration; `@HXP-001` and `@HXG-003` now name each other | — |
| ~~3~~ | ~~`hex_roof`~~ | 15 | **DONE** — `@HXR-001..006`. The distance-source taxonomy, and the eave/drainage trade-off a quantised footprint forces on a point source | — |
| ~~4~~ | ~~`hex_way`~~ | 20 | **DONE** — `@HXY-001..006`. Found and fixed an offset that put every arc on the far side of the way (0.1.1), and measured the quantisation floor the header rounds off: 1.5 down a row, 0.866 down a column | — |
| ~~5~~ | ~~`hex_shape`~~ | 68 | **DONE** — `@HXS-001..009`. The split answer confirmed on all three primitives, and two of the package's own instruments found reading *pass* on what they were built to fail. Coverage 4/83 → 83/83 | — |
| ~~6~~ | ~~`hex_terrain`~~ | 20 | **DONE** — `@HXT-001..008`. The scale boundary worked on a hand-authored ramp, and the package's headline invariant found to be determinism wearing the name of extent-independence: same content, two window sizes, 0 rivers against 5. Coverage 24/26 → 26/26 | — |
| ~~7~~ | ~~`hex_body`~~ | 28 | **DONE** — `@HXB-001..008`. Two-bone arms at quarter turns were the fixture, and they found a 2-D walk that answers for a spatial rig 3.2 units away — collision proxy included — plus the family's third strict-reader claim. Coverage 21/32 → 36/36 | — |
| ~~8~~ | ~~`hex_fit`~~ | 27 | **DONE** — `@HXI-001..008`. Row 1's finding was waiting exactly where it was filed (ten repairs, one of them a wall pointing elsewhere), and under it a wall with two legal names of which only one survives the trip — with the workshop's own gate constructing its way around it. Coverage 8/30 → 37/37 | — |
| ~~9~~ | ~~`hex_draw`~~ | 23 | **DONE** — `@HXD-001..007`. The analytic surface confirmed on both families, and a stated recovery check found unsatisfiable: the miter it compares is a lattice rational, the plan corner it compares against is not. Coverage 3/26 → 26/26 | — |
| ~~10~~ | ~~`hex_field`~~ | 82 | **DONE** — `@HXL-001..012`. The fixture question answered itself again (one hex, two hexes, a ring of six), and under it a validator refusing the multi-form chunk the package exists to trace, plus a material write that DELETED the wall it was setting and an extent taken on trust that allocated 16 M cells from a 32-byte file. 0.1.1, format unchanged byte for byte | — |
| ~~11~~ | ~~`hex_recover`~~ | 33 | **DONE** — `@HXV-001..009`. The blocker dissolved on measurement: the census a caller needs is the 119 forms this build matches against, decided in 2 ms. Under it, a verify step that clipped its own redraw into a window sized for the enumeration it replaced — refusing an 85-cell shape while passing a 105-cell one, and reporting 196 unexplained cells in a 105-cell field. 0.1.1; plus loft#982's second victim upstream (`hex_form` 0.1.2). Coverage 17/46 → 46/46 | — |
| ~~12~~ | ~~`hex_edge`~~ | 39 | **DONE** — `@HXE-001..011`. The owner decision was refuted on both branches: the skin either option needs has no correct value (1e-15 at the origin, 1e-9 at 1.7e6 units out), and `sweep_path` already returns the cell the ambiguity comes from throwing away. `sweep_path_from` threads it — exact, no tolerance, 0.2.0. Plus a portcullis whose raising opens nothing, pinned as behaviour. Coverage 25/40 → 40/40 | — |

**The order is the table's order, and it is not arbitrary.** Rows 1–5 are unblocked
and small; 6–7 need one fixture chosen; 8–9 are genuinely downstream of 1 and cannot
be written first; 10–11 need a scoping decision about what a readable test is; 12 is
the only one waiting on a person. **Rows 1–10 are done**, and every fixture question
answered itself the moment it was asked: a hand-authored ramp for row 6, two-bone arms at
quarter turns for row 7, and for row 10 one hex, two hexes, and a ring of six. None needed
a generated world, and row 10 is the case that settles it — the package with a *real*
oracle, a Python golden JSON, and the readable test turned out to be the one that names its
six vertices in the source. **"Small enough to assert against" means WRITTEN BY HAND every
time it has come up; a shrunken real thing has never once been the answer.** That is now
three for three and can be used as a prior rather than re-derived on row 11.

**Row 10 also retired the "it is a phase, not a row" worry.** 82 public functions did not
need splitting, because the tags do not track functions — they track the package's
load-bearing PROMISES, and there were twelve. Coverage 69/89 → 84/89 came out of writing
them rather than being aimed at.

**`hex_edge`'s block is the healthy case, not a stall:** the convention asks *what does a
caller get wrong*, and when the answer depends on an unmade API decision, writing the
example anyway would pin the wrong behaviour into a test. Row 11 needs no one — only a
decision about what a fast, readable subset of a census looks like.

**Definition of done:** `make examples-progress REPO=../loft-libs-world` reads
*14 tagged, 0 exempt, 0 deferred, 0 todo*. **Reached 2026-08-18.**

### Phase (last) — Convention doc + CI ratchet (S) — CI RATCHET DONE

**The shared gate now runs in every library's CI, from a single source.** The
distribution question had an answer already in the tree: every `loft-libs-*` repo's
`library-ci.yml` is a thin caller of the reusable workflow
`loft-lang/loft/.github/workflows/library-ci-reusable.yml@main`, which checks out
the library at `$GITHUB_WORKSPACE` **and** loft into `loft-src/`. So the shared gate
(`loft-src/scripts/check_doc_drift.sh`) and the acronym registry
(`scripts/example_repos.tsv`) are ALREADY present in every library CI run — "libraries
share it, not a per-repo copy" is satisfied natively, with no `loft-registry` migration.

Built (loft-side only, no library-repo edit):
- `check_examples` made **repo-agnostic** via three env knobs that all default to
  loft's own self-check byte-for-byte: `EXAMPLES_REPO_ROOT` (repo under test, `.`),
  `EXAMPLES_CITE_ROOTS` (dirs grepped for citations, `default lib`), `EXAMPLES_REGISTRY`
  (for test isolation). The script cd's to the loft root at startup, so the registry
  and cross-repo link logic stay loft-anchored while the citation + local-def scan
  follow `REPO_ROOT`. Three repos resolve in place: the repo-under-test (its own
  acronym), the **loft host repo** (its acronyms — `STD`/`GIT`/… — always available at
  `.`, since the gate runs from inside loft even when loft is checked out as `loft-src`
  in a library CI, so a library citing a loft `@STD` tag hard-validates to the loft
  blob link, and a missing one is real `dangling` drift), and a foreign sibling
  checkout `../<repo>` (an `unvalidated` warning only when genuinely absent, e.g. an
  app repo like `moros`).
- `library-ci-reusable.yml` gained a **gating** per-package step running that gate with
  `REPO_ROOT=$GITHUB_WORKSPACE` + `CITE_ROOTS="<pkg>/src <pkg>/tests"`. **Vacuously
  green for a package with no citations**, so it is safe on all libraries from day one
  and only begins gating once a package opts in by authoring a citation — the ratchet,
  enforced by construction rather than by a switch-off-within-a-week promise.

Validated: loft's own `check_doc_drift.sh examples` output is byte-identical before/after
(no self-check regression), and a synthetic library tree proved `ok` / `dangling` /
`duplicate` / `unregistered` all fire correctly in library mode.

**Published per-repo index — where each tag lives, without a checkout.** Each repo
carries a generated `examples-index.tsv` at its root: one row per `@AAA-###`-tagged fn
— `tag ⇥ file:line ⇥ fn ⇥ git-blob-link` — so a reader (or loft's cross-repo `idx`)
learns where a tag resolves without cloning. It is GENERATED, never hand-edited:
`make examples-index` (or `scripts/check_doc_drift.sh write-examples-index`) writes it;
`check_doc_drift.sh examples-index` VERIFIES the committed copy is current (fail-on-diff,
the `features-check` pattern), wired into both loft's `all` gate and the per-package
library-CI step. Line numbers churn, so the index lives WITH the code it indexes — the
central `example_repos.tsv` stays one stable row per acronym. **Auto-created on commit**
by a `.githooks/pre-commit` hook (regenerates + stages it, so every PR carries a current
index); the CI verify is the backstop for a commit made without the hook installed.
Vacuously "no index needed" for a repo with no tags, so it never reddens a library
before it opts in.

**Convention page — DONE.** The shared page the libraries point at instead of
re-explaining the tag family is [LIBRARY_AUTHORING.md § 2a — Worked examples](../../LIBRARY_AUTHORING.md#2a-worked-examples--point-at-a-real-call-site)
(a section, not a new top-level doc): the scope discipline, the citation/definition
two-halves + acronym registry, and the note that the gate already runs in every
`loft-libs-*` CI via the reusable workflow and is vacuously green until a package opts
in. It cross-links to [LIBRARY_DOC_REVIEW.md](../../LIBRARY_DOC_REVIEW.md) for the
staleness/quality half.

### Monthly by-hand review — the ongoing home (DONE, first pass 2026-08)

The automated `check_doc_drift.sh examples` gate only sees a citation that
*dangles* or *duplicates*. Two failures need a human: a doc that still resolves
yet no longer matches the code (**staleness**), and an example that is valid but
no longer the clearest (**quality**). Those are handled by a monthly by-hand pass
at the release beat — [../../LIBRARY_DOC_REVIEW.md](../../LIBRARY_DOC_REVIEW.md),
driven by `scripts/doc-review.sh` (coverage + citation inventory + a
changed-since-watermark worklist), wired into [RELEASE.md](../../RELEASE.md)'s
monthly cadence. It is a hygiene **ratchet, never a release blocker** — the
watermark bounds each pass to what moved. This is where the per-library rollout's
staleness/quality upkeep lives once a library is tagged.

## Open questions

- ~~**Registry home**~~ — RESOLVED: stays `scripts/example_repos.tsv` in the loft
  repo. A migration to `loft-registry` is unnecessary because the library CI already
  reaches the loft tree — every `loft-libs-*` `library-ci.yml` is a thin caller of the
  reusable workflow in loft, which checks loft out into `loft-src/`. So the single
  copy in loft IS the shared copy every library reads; no second home is needed.
- **Online cross-repo validation** — a sibling checkout validates offline today; a
  repo not checked out only warns. A published per-repo tag index (one small file
  fetched by raw URL) would let it hard-validate offline-of-clone. (Phase C follow-up.)
- **`// Example:` spelling** — kept dryopea's spelling for one shared indexer.
- **In-repo `lib/moros_*`** — first-class-app-internal libs; lower priority than the
  registered/sibling libraries a broad audience consumes.

## See also

- dryopea `docs/EXAMPLES.md` — the origin design + the `examples.sh` gate this adapts.
- dryopea `plans/26-the-fixed-step` — first library shipped under the convention.
- The loft-side gap this plan closes: `scripts/idx` + `make index` recognise
  `@P`/`@PLN`/`@F`/`@GH` today, not `@AAA-###`, and crawl only the loft tree.
- Feature catalogue (Phase C2): `loft-lang/features` issues + the
  `make features-fetch/gen/check` shadow; the generator's first-` ```loft `-fence
  RUN test is the compiles-and-runs gate the real-usage tag complements.
