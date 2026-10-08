<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# First-contact work — the outside view, made concrete

**What this is.** A design for the presentation / first-run work proposed by an
external review (2026-07-24). The review carried the *visitor's* perspective —
what a first-time reader sees in five minutes — which is the one thing an agent
deep in language internals cannot reconstruct for itself. This doc keeps that
perspective and turns each item into a **decision**, not a restated task.

**Not a plan directory.** Per [plans/README.md](plans/README.md), most items here
are bug-fix / operational / light-TODO shaped and belong in direct commits. Only
T2 (gendoc SEO) is plausibly plan-shaped, and a plan's identity is a tracker issue
number — the owner's to mint. Promote later if it earns it.

**Tone constraint that governs every item:** understated-honest. State facts once,
plainly. No superlatives, no credential adjectives. The performance page showing
the interpreter losing to CPython stays exactly as it is. Restraint is the signal.

---

## Inspection — what the review got right, and four corrections

Verified against the working tree, 2026-07-24. The review's items were checked
against source rather than the release binary, which moved three of them.

| Item | Verdict |
|---|---|
| T0.1 release-binary warning | **Confirmed** (source), and the proposed fix fails its own acceptance test — see C1 |
| T0.2 line offset + missing snippet | **Confirmed, and re-localized** — see C2 |
| T0.3 `int` has no did-you-mean | **Confirmed** — `error: Undefined type int`, no suggestion, snippet renders fine |
| T1.1 badge / dead link | **Confirmed** — badge hardcoded `2026.7.1`; README:136 linked a non-existent dir |
| T1.2 chunk-repo contradiction | **Resolved** — examples never pushed (claim removed); the chunk README is the stale side, not the main one |
| T1.3 duplicate Time page | **Confirmed, worse** — see C3 |
| T2.1 no meta tags | **Confirmed** — `doc/index.html` has zero description / OG / Twitter tags |
| T4.1 root clutter | **Confirmed for tracked files, but mis-framed** — see C4 |

### C1 — the once-per-user marker fails the acceptance test it is given

T0.1 proposes a state marker (`~/.loft/native-notice-shown`) and then asks for
*"fresh container, unzip release, run twice → warning at most once"*. A fresh
container has no `~/.loft`, so a marker fires **on every first run in every
container** — exactly the CI/demo/Docker path where the nag is most visible. The
marker also adds writable-state on a path that must work when `$HOME` is unset or
read-only.

**Decision: gate on intent, not on state.** Say nothing on the default path;
explain only when the user asked for native (`--native`, or a native-only
operation). Rationale in one line: someone who downloaded a release binary chose
*not* to install a toolchain, so "install Rust and rebuild" is not an action they
want — it is not a warning, it is a non-sequitur. No new state, no new file, and
it passes the container case the marker fails.

Keep the full explanation in `--help` and in the release `QUICKSTART.md`.
Record as a `DESIGN_DECISIONS.md` entry (the review asks for this either way).

### C2 — T0.2 is the bare-script wrapper, not the `default/` prelude

The review guessed prelude concatenation. It is not: an explicit `fn main()`
reports correctly, and only *bare* scripts are shifted.

| source | reported | snippet |
|---|---|---|
| explicit `fn main()`, error on line 2 | `2:3` ✓ | yes |
| bare script, error on line 1 | `2:1` ✗ | **no** |
| bare script, error on line 2 | `4:1` ✗ | **no** |

Bare-script lines map to roughly **2×** the true line, so the fault is in the
synthetic-`main` wrapper's span mapping, not the prelude. That also **confirms the
review's "two symptoms, one cause" hunch**: the computed line does not exist in the
file, so snippet lookup finds nothing and renders silently. Fix the mapping and the
snippet returns for free — do not chase the missing snippet separately.

The release binary reported `:5:2` where source reports `:4:1`; the offset is not a
fixed constant, which is further evidence it scales with the wrapper rather than a
fixed prelude length.

### C3 — there are three Time pages, not two

`doc/22-time.html`, `doc/32-time.html`, **and** `doc/stdlib-time.html`. Decide one
canonical page before merging, or the nav gains a third entry the next time
`gendoc` runs.

### C4 — root clutter: the two biggest files are invisible, and two stray dirs are not

The review measured a local checkout. A visitor sees the **GitHub file list**, which
shows *tracked* files only, sorted **alphabetically with directories first** — file
size is irrelevant to what they see.

- `result.txt` (311 KB) and `isolated_stair.glb` (302 KB) are **untracked**. They are
  the two largest things in the directory and no visitor ever sees them. Ignore.
- The tracked offenders are as the review says: `.lint_comments_baseline` (100 KB),
  `Makefile` (79 KB), `DRAWING.md` (53 KB).
- **Missed: `dz/` and `f8e2e/`** — two stray directories committed by accident in the
  *same* PR (#463, @PLN86 sandbox v1). `dz/dz.loft` is a divide-by-zero probe;
  `f8e2e/` is a sandbox e2e fixture. `grep` finds **no reference** to either from any
  `.rs`, `.toml`, `Makefile`, `.sh` or doc. They are safe to delete, and they sit
  near the top of the alphabetical listing with names a visitor cannot interpret.
- Root has **44 tracked entries** (27 dirs + 17 files). The problem is count and
  legibility, not bytes.

`docs` is a symlink to `doc/` — intentional (Pages), leave it.

---

## Design per item

Ordered by the review's tiers. Each entry states the decision; the acceptance test
is the definition of done.

### ~~T0.1~~ — DONE 2026-07-24
Silent on the default path at all four fallback sites; `--native` explains. Recorded
as [DESIGN_DECISIONS.md @C102](DESIGN_DECISIONS.md). Verified in a release-shaped
layout (no source tree, `rustc` absent): zero notices on two consecutive runs.
*The first attempt at verifying this was vacuous* — `rustc` was still on the test
`PATH`, so native simply succeeded; the calibration check (`which rustc` on the
stripped PATH) is what caught it.

### ~~T0.2~~ — DONE 2026-07-24 (with a residual, filed)
`script_desugar_mapped` now returns a line map beside the generated source, and
`Diagnostics::remap_lines` puts every diagnostic back into the user's coordinates
right after the parse. Both symptoms fixed by the one change, as predicted: the
2-line repro reports `:2:` **with** the snippet, a 1-line script reports `:1:`,
both backends (`tests/script_mode.rs::t02_*`, map unit-tested in
`script::tests::t02_line_map_tracks_the_source`).

**Residual → loft#625.** A script that *hoists a def* still shows a wrong line, but
through a **different, pre-existing** bug: the desugar map is provably correct
(unit-tested), and the generated source reproduces the same lag when run directly
as a plain file with an explicit `fn main()`. Bisected: it needs a call to a def
**plus** a preceding statement. Not scope-crept into T0.2.

### ~~T0.3~~ — DONE 2026-07-24
Alias table `data::builtin_type_alias`, consulted before edit distance — which
**cannot reach this class at all**: `suggest_similar_capped` returns `None` at ≤ 3
characters (`int`, `str`, `i64`, `f64`) and caps distance at 2, so `bool`→`boolean`
(3), `char`→`character` (5) and `string`→`text` (unrelated) all fail. The table is
the whole mechanism.

Two things fell out. `DefType::Type` joined the candidate set, so a mistyped
**builtin** (`intger`, `bolean`, `charater`) now suggests for the first time — it
never had candidates before. And the hardcoded `'string'` special case, which
existed in **three** places, folded into the one table.

Suggestion only; declined as a language change with the author's rationale in
[DESIGN_DECISIONS.md @C103](DESIGN_DECISIONS.md). Note `i32`/`u32`/`i8`/`i16`/`u8`/`u16`
are **legal** and correctly absent from the table.

### ~~T1.1~~ — DONE 2026-07-24, **except one claim that needs Jurjen**
Dynamic version badge (verified 200). `make linkcheck` / `make linkcheck-external`
(`scripts/linkcheck.sh`) — relative links are offline and exact; the external pass
is opt-in and stays OUT of `make ci` (it would make the build depend on other
people's uptime). It skips inline code spans (`OPERATORS[opcode](state)` is not a
link) and test fixtures (which hold deliberately dead links). **323/323 links now
resolve**, including external.

Fixed: the dead "full example list" link; two CHANGELOG links to `v0.1.0`, a
version that was never tagged; and a stale `lib/server` link (that library moved
to `loft-libs-net/server` in @PLAN12).

**BLOCKED, and bigger than a link — see T1.2.** The README's graphics claims are
not backed by anything in the org (details below); fixing the *link* was safe,
fixing the *claim* is a status question only the owner can answer.

### ~~T1.2~~ — RESOLVED 2026-07-24 by the owner: the examples were never pushed

Owner's answer: **the 24 graphics examples were never pushed.** The claim is
removed — not softened — everywhere it appeared:

| was | now |
|---|---|
| README pillar: "24 interactive graphics demos, from hello-triangle to PBR with shadows" | the pillar is **Brick Buster**, which is real and playable |
| README § "Graphics examples": "ships 24 progressive examples" + a table of 7 files | section deleted |
| `doc/gallery.html` crumb: "25+ live demos" | "live WebGL demos written in Loft" |
| index tile: tag "24 demos", "hello-triangle to PBR" | tag "WebGL", describes what is actually there |

**One correction to my own earlier write-up.** I reported the chunk repo as
evidence that `graphics` was blocked. Checking the registry index shows
`graphics` and `imaging` **are published** (alongside `input`, `server`,
`markdown`, and 17 more), so `loft install graphics` works and the README's
package table was right all along. The **chunk repo's** README is the stale
one — it still says "TODO (blocked on `Type::Reference` codegen forwarding)".

So the two READMEs disagreed for two *different* reasons, and I had only found
one: the main README overclaimed the **examples**, and the chunk README
underclaims the **library**.

**Chunk README fixed 2026-07-24** (direct commit to `loft-libs-graphics` —
`main` is unprotected and this is a doc-only correction, the lightest sanctioned
path per [plans/README.md](plans/README.md)):
[`c405962`](https://github.com/loft-lang/loft-libs-graphics/commit/c405962f8a6b6def094aa6d28743d490ff1761b7).

**All four rows were stale, not two.** The registry index and the repo's own tags
say `graphics` v0.5.0 (nine releases), `imaging` v0.2.1, `shapes` v0.3.0,
`gridmesh` v0.1.2 — the README claimed the first two were blocked and the other
two were still at v0.1.0. Versions are now taken from those two sources rather
than edited in place, the install snippet lists all four, and the design-doc link
points at `loft-lang/loft`.

So the pair is now consistent from both ends: loft no longer overclaims the
**examples**, and the chunk repo no longer underclaims the **library**.

### ~~T1.3~~ — Time DONE 2026-07-24; ordering DEFERRED with a reason

The three Time pages are **not duplicates** — they are different content that
shared a nav label: `tests/docs/22-time.loft` is the builtin `now`/`ticks`,
`32-time.loft` is the extracted `time` *library*, and `stdlib-time` is the API
page. Both `.loft` files declared `@NAME: Time`. Fixed by renaming the library
one's `@NAME` to `Time library`, so the nav and `<title>` are distinct. No merge,
no page deleted — merging would have destroyed real content.

**Two pages now, not three** (@PLN149 step 9, 2026-08-29): the library one moved into
the `time` package as `docs/01-getting-started.loft` and is published as
`doc/lib-time-guide.html`, off the library's card rather than the language nav. What is
left under Time is `22-time` (the builtin `now` / `ticks`) and `stdlib-time` (the API),
which were never the pair that confused anyone.

**Lexer/parser reordering deferred.** `gather_topics()` orders by the numeric
filename prefix and `gendoc` has no topic-grouping mechanism, so reordering means
*renaming* `15-lexer` / `16-parser` — 38 references, many in finished plan
documents that are historical records and should not be rewritten. The cost is
wrong for a nav nicety. If it is worth doing, the right shape is a grouping
mechanism in `gendoc` (T2-shaped code), not a rename.

### ~~T2.1–T2.2~~ — DONE 2026-07-24
`page_html` is the single template, so the meta block lands at one chokepoint:
`description`, `canonical`, the five `og:*`, `og:site_name` and four `twitter:*`,
with titles as `Enums — Loft, a language for small browser games`. `sitemap.xml`
(65 pages) + `robots.txt` are emitted by `generate_sitemap`, which **derives the
page list from `doc/*.html`** rather than a hand-kept table, so a new page cannot
be silently missing. `print.html` is excluded — it is a one-page rendering of
pages already listed, and indexing it would offer a duplicate of the whole site
under one URL.

Descriptions are not scraped: a topic page uses its own `@TITLE`, a stdlib
section its hand-written one-liner. Two things worth knowing:

- **The landing page had no head of its own.** `write_index` builds its `<head>`
  separately from `page_html`, so the single most-shared page would have been the
  one page still missing every tag. It now has a hand-written description, and its
  own title (`Loft — a language for small browser games`) because the sub-page
  shape stutters there (`Loft — Loft, a language…`).
- **17 pages are hand-written, not generated** (`playground`, `gallery`,
  `install`, `roadmap`, `00-vs-rust`, …), so `gendoc` never touched them.
  `playground.html` and `gallery.html` — the two loft owns and that people
  actually share — got the block by hand. The rest are listed below as the
  residual.

### T2.3 — off-repo **[needs Jurjen]**
Search Console + Bing submission; GitHub **social preview image** (this is what
renders on every HN/Discord/Slack share — highest reach per minute of the whole
document); repo topics.

### ~~T2.4~~ — DONE 2026-07-24 (poster + three more instances of the claim)

The Brick Buster tile now carries a static poster, so the card shows the demo
before WebGL warms — and to crawlers and slow connections, which never run it at
all. Keyed by demo (`images/poster-<key>.png`) and hidden when absent, so adding
a poster for a future demo is dropping a file in, with no code change.

**No new asset was generated.** I tried the in-repo headless capture first
(`tools/html_render_check.mjs --screenshot`, which already drives headless Chrome
— exactly the tooling the review pointed at). It reproducibly captures a
partial, dimmed frame: 6 distinct colours, three brick rows, no title text, at
both 8 s and 20 s. So `doc/brick-buster.html` renders only a partial frame in
headless Chrome — worth knowing, since that is roughly what a preview bot sees.
The existing `showcase-title.png` was an **unused orphan** and is an authentic,
fully-rendered title card, so it was renamed into place rather than duplicated.
(`showcase-powerups.png` is still an unused orphan.)

**Rendering the page found three more instances of the removed claim** that
grepping the source had not:

- Three of the four section headings (*Fundamentals*, *Lighting & scenes*,
  *Advanced rendering*) had **no demos behind them** — bare headings over empty
  space, which reads as broken rather than as not-written-yet. A section with no
  cards now hides itself, and reappears on its own when a demo lands in its
  range; the taxonomy is not deleted.
- The intro claimed the demos were *"Adapted from the LearnOpenGL tutorials"* —
  a body of work that was never pushed.
- The footer's "Source on GitHub" pointed at `lib/graphics/examples/README.md`,
  which does not exist; it now points at the Brick Buster source.

The lesson worth keeping: for a page whose content is data-driven, *rendering it*
finds what grepping the template cannot.

### T2 residual — pages `gendoc` does not own
`brick-buster.html` is regenerated by `make game` (via `loft --html`), so its
`<title>` is `25-brick-buster` — a filename, on the page you would share to show
the game off. Fixing it means changing the **exporter** (`src/main.rs`), and that
needs a decision first: an exported page belongs to the USER's game, so loft must
NOT inject its own `og:image` there (every shipped game would preview as Brick
Buster). A neutral title + viewport is probably right; a loft-branded share card
is not.

Still without meta: `install`, `roadmap`, `docs`, `examples`, `report`,
`00-performance`, `00-vs-rust`, `00-vs-python`, `kernel-*`, `gallery-run`,
`crystal-editor`, and a stale `stdlib-random.html` (its title is still the old
`Loft - Random` format, i.e. `gendoc` no longer regenerates it — likely an
orphan worth deleting).

### ~~T3.2~~ — DONE 2026-07-24
"How loft is built" moved above "Packages & libraries" (it is the most newsworthy
section in the file and sat second-from-last). The one-file claim became an
**invitation**: *"The whole game is one file — read all 1,983 lines."*

The number is the proof, so it must not rot:
`doc_hygiene::readme_brick_buster_line_count_is_current` parses the README's
claim and compares it to `wc -l` on the game. Verified it fails when the number
is wrong — a guard nobody has seen fail is not a guard.

### T3.1 / T3.4 — DRAFTED, awaiting approval **[needs Jurjen]**
Both staged under [drafts/](drafts/) rather than published, because both are the
owner's to approve:

- [drafts/provenance-paragraph.md](drafts/provenance-paragraph.md) — two wordings
  of the fourth-language paragraph, plus the one judgement call worth making
  consciously ("forty years of programming" is the only credential-shaped phrase
  in it). Deliberately **not** placed in the README: unapproved biographical
  wording about a real person is not an agent's call.
- [drafts/why-loft-is-the-way-it-is.md](drafts/why-loft-is-the-way-it-is.md) —
  seven entries written from repo sources (`GOALS.md`, `DESIGN_DECISIONS.md`),
  each citing where the reason comes from, plus three marked `[needs Jurjen]`
  where the repo has the mechanism but not the *why* — notably **LGPL**, which
  the repo never justifies anywhere, and which every evaluator asks about.

### T3.3 — animated hero — **blocked on the game not rendering, not on tooling**

`ffmpeg` is now installed (7.0.2 static, `~/.local/bin/ffmpeg`, no sudo needed),
so encoding is no longer a blocker. The remaining one is upstream of capture:
**Brick Buster draws a static, partial canvas here and never advances.**

Five approaches, all the same result — headless + wall-clock, headless +
`Emulation.setVirtualTimePolicy`, a real (non-headless) Chrome on an Xvfb
display, a fresh `make game` rebuild, and click-then-Space. Attribution, not
guesswork:

- `requestAnimationFrame` runs at **61 ticks/s** — the browser loop is healthy.
- Input **is** received: a CDP click + Space changes the canvas exactly once
  (title → play field), then it never changes again.
- The play state renders **3 of 5 brick rows, no ball, no score**.
- No console errors, no exceptions.

**The project's own gate reproduces it**, which is what turns this from "my
capture is wrong" into a finding: `tests/html_render.rs` fails with
`canvas.blank — canvas has only 6 distinct colors (threshold 20)` on a
freshly-built artifact.

*(One instrument was invalid and is worth flagging: hashing `canvas.toDataURL()`
measures nothing on a WebGL canvas without `preserveDrawingBuffer` — it returns
blank regardless. `Page.captureScreenshot`, which composites, is the honest
signal.)*

**Two questions for the owner, in order:**

1. **Does Brick Buster still render correctly in a real browser?** If yes, this
   is a SwiftShader/software-GL limitation and the animated hero simply needs a
   manual recording (spec below). If no, the shipped game page has a real
   regression — which matters more than the hero.
2. **Is the render gate actually running anywhere?** It skips in `make ci`
   whenever the wasm rlib is newer than the bundle — the normal dev-machine state
   — and the job that runs it (`ci.yml`, `binary(html_render)`) is PR-triggered,
   so it has not run on this branch. A gate that skips locally and only fires on
   a PR is close to unwatched.

**Manual recording spec** (unchanged, and now only the capture is missing):
~5 s mid-play showing ball motion, brick breaks, a powerup drop and multiball;
the game's own 800×600 canvas, no desktop chrome; GIF for the README. Keep the
PNG — OpenGraph does not animate, so `SITE_OG_IMAGE` stays as it is. With
`ffmpeg` present, `ffmpeg -i clip.webm -vf "fps=15,scale=800:-1,palettegen"` /
`paletteuse` turns a screen recording into the GIF in one step.

### ~~T3.5~~ — DONE 2026-07-24, routed differently than proposed
`examples/README.md` added. The review suggested routing game-seekers to the
**gallery**; B2 established the gallery holds one demo and graphics is blocked
(T1.2), so it routes to **Brick Buster** instead — which demonstrably works, is
playable in one click, and whose source is the best thing in the repo to read.

Each of the seven CLI examples now links its **playground twin** (every one has an
`ex_*` key, verified present in `doc/examples.js`), so a reader can run any of
them without installing anything. The graphical example is deliberately NOT added:
it would need `loft install graphics`, which the chunk repo says is blocked.

### ~~T4.1~~ — DONE 2026-07-24
Deleted the `dz/` and `f8e2e/` strays; `DRAWING.md` → `doc/claude/` (3 inbound
links rewritten, one of which was already broken); both baselines → `scripts/`,
beside the only scripts that read them (one `BASELINE=` line each; both verified
`--check` green after the move). Root: **44 → 39** tracked entries.

### T4.2 — Discussions **[needs Jurjen]**
Enable before any launch post; link from README + `SUPPORT.md`.

---

## First-use walkthrough (2026-07-24) — done as a visitor, not from the list

After B1–B4 landed I ran the actual first-use path in a release-shaped sandbox
(no source tree, no `rustc` on `PATH`) rather than working further down the list.
Three things showed up that the external review had not, because they only appear
when you *do* it:

### ~~FU.1~~ — a mistyped path dead-ended. FIXED
`loft examples/helo.loft` said `fatal: Unknown file:examples/helo.loft` — no
space after the colon, no suggestion, with `hello.loft` sitting in the same
directory, followed by a redundant "aborting due to 1 previous error". Mistyping
the path is one of the *commonest first actions* anyone takes.

Now: `no such file: examples/helo.loft — did you mean 'hello.loft'?`, reusing the
same `suggest_similar_capped` cap the function/type suggestions use, over `.loft`
siblings in the same directory. A name with no near neighbour gets no invented
suggestion. Guarded by `script_mode::mistyped_file_path_suggests_the_neighbour`.

### ~~FU.2~~ — `--help` buried the two things a newcomer wants. FIXED
162 lines, opening with `--path` / `--project` / `--lib` / `--log-conf`. The
usage header did not mention that **bare `loft` starts the REPL** — that was on
line 64. Added a *Getting started* block naming the two likely actions (run a
file, start the REPL) before the option wall, and put the REPL in the usage
header. It also states plainly that a release interprets by design, which is the
same message [@C102](DESIGN_DECISIONS.md) removed from the runtime path.

### ~~FU.3~~ — a failed REPL input warned about earlier, correct code. FIXED
```
loft> x = 5
loft> prnt("hi")
Error: Unknown function prnt — did you mean 'print'? at <repl>:1:1
Warning: Variable x is never read at <repl>:1:4
```
`x` **was** read — by the very input that failed to compile. A beginner's single
typo produced two messages, one of them false and pointing at a line they wrote
correctly, at a column inside a different line.

**Mechanism** (matrix, `src/repl.rs`): an input runs as `fn replmain_N() { <replayed
body> <input> }`, and `map_input_lines` subtracts the prefix with `.max(1)` — so
*every* line of the replayed body clamps onto the user's line 1. Three prior
bindings gave three false warnings, all at `1:4`. The success path drops the
generation's diagnostics wholesale, so this failure path was the only route by
which any warning reached the user at all — and what it surfaced was the replay's,
never the user's.

**Fix**: `map_input_lines` now drops a prelude-region **non-error** instead of
clamping it onto line 1, and keeps a prelude-region **error** (imprecise line and
all — swallowing it would leave the REPL rejecting input while saying nothing, and
a bad position beats silence). Guarded by
`repl_session::a_failed_input_does_not_warn_about_the_replayed_session`, calibrated
by disabling the filter and confirming the three warnings return.

## Sequencing — five batches, not twenty PRs

`CLAUDE.md` says bundle subjects into one CI cycle; a PR per item would burn ~30
minutes of CI each for changes that are individually minutes of work. Batch by
*risk class*, so a red gate points at one kind of thing:

| Batch | Contents | Risk |
|---|---|---|
| ~~**B1 — first-run bugs**~~ | ~~T0.1, T0.2, T0.3~~ — **DONE 2026-07-24** (residual loft#625) | Code + tests; the only batch touching the parser |
| ~~**B2 — truthfulness**~~ | ~~T1.1, T1.3, T4.1~~ — **DONE 2026-07-24**; T1.2 now evidenced and blocked on the owner | Docs, `gendoc` ordering, file moves/deletes |
| ~~**B3 — discoverability**~~ | ~~T2.1, T2.2~~ — **DONE 2026-07-24**; T2.4 blocked on T1.2 | `gendoc` output; verify a sample `<head>` |
| ~~**B4 — story**~~ | ~~T3.2, T3.5~~ — **DONE 2026-07-24**; T3.1/T3.4 drafted for approval | README + new page, no code |
| **B5 — owner** | T1.2, T2.3, T3.1 wording, T3.3, T4.2 | Outside the repo |

B1 first (two are plain bugs and all three are "first five minutes"). B2 next — a
README that lies undoes everything after it. Then B3. B4/B5 in any order.

**Cross-cutting gates.** Every generated-site change: `cargo run --bin gendoc` +
`make gallery`, then check a sample page's `<head>` and run `make linkcheck`. Every
diagnostic change: `--interpret` **and** `--native` covered by tests. Everything
through `make ci`. Remove items from this file as they land.

## Non-goals

No new language features, no new pillars. No superlatives, no credential
adjectives, no weakening of the honest benchmark presentation. Do **not** silently
make `int`/`str` legal — suggestion only, unless a design decision is taken.

## See also

- [PLANNING.md](PLANNING.md) — the backlog and house rules this follows.
- [plans/README.md](plans/README.md) — why most of this is *not* a plan directory.
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — check before re-litigating; T0.1
  lands an entry here.
