# Preventing "Failed to grow table" in the deployed browser pages

## Two separate browser pipelines to protect

The repo ships **two independently-built browser artefacts** that land
on GitHub Pages, and each can go stale on its own:

| Artefact | Who uses it | Built by | Make target |
|---|---|---|---|
| `doc/pkg/loft_bg.wasm` + `doc/pkg/loft.js` | `gallery-run.html`, `playground.html` | `wasm-pack build --target web` | `make gallery` |
| `doc/brick-buster.html` | The featured "click-to-play" arcade game | `loft --html` against a `wasm32-unknown-unknown` libloft.rlib + wasm-opt | `make game` |

⚠ **A THIRD artefact is derived from the same wasm build and goes stale the same way, but by
a subtler route: `index/target_surface.json`.**  `make surface-gen` asks a **prebuilt** wasm
rlib which builtins exist, and the rebuild it needs is NOT `make wasm`, which builds the
wasm-bindgen package in the table above.  The rlib it reads comes from

```
cargo build --release --target wasm32-wasip2          --lib --no-default-features --features random
cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random
```

Run `surface-gen` without those and it records builtins as UNAVAILABLE IN THE BROWSER and
commits that as derived truth.  Measured 2026-09-24: run straight after `make wasm`, it
re-added `store_load_url` and `store_load_url_trusted` to the unavailable list — silently
reverting a fix made on the same branch that morning, when a join had made the identical
mistake by hand.  With both rlibs built first it answers *"2 of 111 builtins unavailable"* and
the file is byte-identical to what was committed.

**Same file, same command, opposite answers, decided only by build ORDER.**  That is what makes
this one dangerous rather than merely annoying: the generator's two failure modes — a real
surface change and a stale read — are indistinguishable in the diff, so the diff is not
evidence on its own.  Read a builtin BECOMING unavailable as a suspected stale read first: a
builtin does not usually leave the browser.  `make ci` gets the order right (it builds both
rlibs before `gen_target_surface.py --check`); a hand-run is where it bites, and CLAUDE.md's
*"rebuild that rlib first"* does not say which rebuild is meant.

⚠ **`gallery.html` makes no DIRECT reference to the bundle — it reaches it one page down.**  It
is an index linking to `gallery-run.html?example=…`, which is where `./pkg/loft.js` is actually
imported.  The row above therefore names the page that HOLDS the relationship rather than the
entry a reader arrives at; it was LOOSE rather than wrong, since the gallery does consume the
bundle, through that link.  The distinction is about AIM: a render check pointed at
`gallery.html` loads no wasm and passes while the bundle is broken, which is the exact failure
this document exists to prevent.  Measured 2026-09-16: 0 occurrences of `pkg/` in
`gallery.html`, against 3 in each of `gallery-run.html` and `playground.html`.

Both pipelines produce a wasm/js pair that must agree internally.  The
failure mode is identical: the browser aborts with

```
LinkError: WebAssembly.instantiate(): Failed to grow table
```

or

```
Function import mismatch at 'loft_io' imports
```

because the JS glue and the wasm binary came from different builds of
the source tree.  **Every single time we have seen it** the cause was
a partial or stale rebuild landing in either `doc/pkg/` or
`doc/brick-buster.html` — either on disk during local dev or in git
when a PR skipped the rebuild step.

## Defence layers

The layers below split in two.  Most are STRUCTURAL — they ask whether the
artefacts exist, agree with each other and are served — and each is
individually sufficient against the failure they were built for: a wasm/js
pair that does not agree.  One is BEHAVIOURAL: it runs the page.  That one
was added last, because the structural ones could not see the failure it
catches.

⚠ **A structural layer cannot see a bundle that loads cleanly and draws
nothing, and one of them asserted that it could.**  Every step of
`make gallery` was structural — files present, glue and wasm from one
build, every asset HEAD-200 — so it printed `gallery ready` over a page
whose canvas held one flat colour (measured 2026-09-16, loft#1545).  No
layer opened a browser on the gallery path: the only browser render in CI
was `tests/html_render.rs`, and it loads `doc/brick-buster.html`.  Layer 4
sits on the very page that rendered blank — `doc/gallery-run.html`, where
the guard at `:279-289` wraps the instantiation itself — and it still could
not fire: it is keyed to `LinkError: Failed to grow table`, the STALENESS
symptom, so a current bundle instantiates cleanly, sets `loftReady`, enables
the Run button and draws nothing, with the condition never true.  The COUNT
of layers was never the measure; what each one tests is.

**Closed by step 7 of 8** (`scripts/gallery_render_check.sh`), which opens
every listed example in headless Chrome and asserts the page's own status is
not a failure state.  That is the signal no structural step can reach:
`gallery-run.html` reports compile and runtime errors into its DOM and never
to `console.error`, so a console-only check records a clean run while the
page shows the error in red.

⚠ **A colour threshold is NOT the cure, and this is the part worth keeping:
the failure was INVERSION, not absence.**  `tests/html_render.rs` does drive
the paged asset route — `25-brick-buster.loft` reads its atlas through
`assets::prefetch` / `blobs_path` — and it PASSED for as long as the defect
existed.  The shipped page scored **767 distinct colours** with its sprite
atlas entirely missing, because `load_atlas` falls back to procedural
drawing and reports the failure in WORDS.  An absent test is a gap anyone
can see; a covering test that returns green is false assurance, and that is
why this survived.  No value of `--canvas-min-colors` repairs it — the check
is blind to CONTENT at every colour count.  What works is asserting on the
program's OWN output, which required `resume_frame` to carry per-frame text:
a long-lived program otherwise says everything into a buffer that is read at
a moment which never comes.

### 1. `make gallery` — local one-shot verify-and-rebuild

```
make gallery
```

Cleans `doc/pkg/`, rebuilds via `wasm-pack`, verifies the files the
gallery imports — `doc/loft-rt.js` among them, which supplies the page's
host, and the sprite pack under `doc/assets/`, without which an example
compiles and runs and draws an empty atlas — checks timestamps on
`loft.js` and `loft_bg.wasm` agree (the canonical staleness signal),
starts a transient http.server, HEAD-probes every URL the gallery loads,
opens every listed example in headless Chrome, and only prints
`gallery ready` if every step passes.

Fails with `[N/8] ... FAIL ...` pinpointing the stage so the
developer can fix it before pushing.

### 2. PR CI gate (`.github/workflows/ci.yml::gallery` job)

Every pull request runs **both** `make gallery` and `make game` on a
clean ubuntu runner — the two pipelines share the wasm target cache
so the incremental cost is small.  A PR cannot merge if either build
fails, if any asset served by a local http.server returns non-200,
or if the Brick Buster HTML sanity-check fails.

This catches the case where a developer changes loft source, the
graphics library, or `lib/graphics/examples/25-brick-buster.loft`
without rebuilding the corresponding browser artefact.

### 3. Release-time rebuild (`.github/workflows/release.yml::docs` job)

The Pages-deploy job now runs `make gallery` and `make game` in
sequence before publishing.  Pages therefore always serves a wasm
bundle + JS glue generated from the same commit — regardless of what
was committed in `doc/pkg/` or `doc/brick-buster.html`.  This is the
last line of defence if stale files slipped past PR review somehow.

### 3b. Build stamp (`scripts/wasm_bundle_stamp.sh`)

`make wasm` writes `doc/pkg-src.stamp` — a hash of the sources the
bundle is built from — and the two browser tests
(`engine_host_connector::browser_kernel_one_script_differential` and
`::s6_browser_swap_under_living_page`) recompute it and FAIL when it
disagrees.

Those two tests LOAD the committed bundle rather than building one, so
without the stamp they report on whatever was last committed instead of
on the tree under test.  That is loft#1189: the bundle was a year older
than the source and predated `client_loop`'s call to `kernel_swap_step`,
a native the browser kernel did not supply — and both tests were green
the whole time.  They fail rather than skip, because a skip reads the
same as a pass to everything downstream.

⚠ The stamp covers the files that decide what the browser kernel and the
two fixture pages do, not the whole build input, and the script's header
says why: an exact stamp reddens these tests on every commit touching
`src/`, which ends in either a skipped test or a 2 MB binary recommitted
several times a day.  It catches drift at the scale that actually
happened.

### 4. Runtime guard (`doc/gallery-run.html::initLoft`)

If a mismatch ever reaches a browser despite the above, the gallery
now translates the cryptic `LinkError: Failed to grow table` into:

> The gallery's WASM bundle and JS glue are out of sync (classic
> "failed to grow table" error). This usually means the deployed
> build is stale.  On a local clone, run `make gallery` to rebuild
> both together.  If you are on the deployed site and still see
> this, please file an issue.

The user sees an actionable message, not a browser internal.

## Why not just `.gitignore doc/pkg/`?

An obvious further step is to stop committing the generated bundle
entirely and only build it at deploy time.  That would also work —
and is the cleanest long-term answer — but it breaks two current
workflows:

- `make serve` immediately after `git clone` with no other setup.
- Forking the repo and browsing `doc/` locally via `file://` URLs
  without running any build.

Removing `doc/pkg/` from git would require everyone to run
`make gallery` before `make serve`, and fork users would see broken
404s.  If the CI layers above prove insufficient in practice, moving
to an ignored-but-rebuilt-on-deploy model is the cleanest next step.

## Recap — what happens on each event

| Event | What catches a broken browser artefact |
|---|---|
| Dev edits, runs locally | `make gallery` + `make game` on demand |
| Suite runs the browser tests | `doc/pkg-src.stamp` — they refuse a bundle built from another tree |
| Dev opens a PR | CI `gallery` job runs **both** `make gallery` and `make game` |
| PR merged to main | CI `gallery` re-runs post-merge |
| Tag pushed, Pages deploys | Release workflow runs `make gallery` + `make game` before `gh-pages` deploy |
| User opens the deployed page | Runtime `explainLoadError` surfaces the classic LinkError as an actionable message in `gallery.html` |

## See also

- `Makefile` — `gallery` target (7-step recovery pipeline)
- `doc/claude/GAME_TESTING.md` — the same approach used for per-example snapshot tests
- `doc/claude/BRITTLE.md` — section 3 (thread/build-glue plumbing) has a similar pattern: raw pointers threaded through multiple sites, easy to desync.
