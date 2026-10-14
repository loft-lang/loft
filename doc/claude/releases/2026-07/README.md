<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-07` — candidate release blockers

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

The gate is **stability + public face**, not features (see § Safety gate). As
of 2026-07-04 the filed-bug tracker is **empty** (`loft-lang/loft` has 0 open
issues; highest is #501, all closed), so the list below is the concrete
must-close set before the `2026-07` tag — grouped hard-blocker → public-face →
ecosystem → hygiene. The `2026-07` branch already exists; rebase it onto the
current `main` tip before working the list.

**A. Safety gate — hard blockers (a crash/leak slips the tag, no exceptions):**

- [x] **D-key-1 — parser crash on a value-position keyed slice.** ✅ FIXED
  2026-07-04. A keyed range slice (`x = idx[lo..hi]`) panicked at
  `src/variables/mod.rs:524` (`set_loop` with no active loop); a partial-key
  match (`x = idx[k1]` on a multi-key index) panicked later at codegen
  ("Iter should have been rewritten"). Both are a `for`-only iterator used in a
  value position — now a clean diagnostic. Fix: a save/restore `iterable_context`
  flag around the iterable parse in `parse_in_range`, `set_loop` tolerates the
  missing loop, and `parse_key` rejects both subscript forms in a value position.
  Guards: `tests/parse_errors.rs::keyed_{range_slice,partial_key}_in_value_position_is_error`
  + `tests/scripts/502-keyed-slice-for-only.loft` (the legit for/comprehension/
  exact-lookup paths, both backends). full parse_errors (157) + wrap (51) green.
- [x] **Re-verify the full safety suite on the tag candidate** — ✅ RUN
  2026-07-04 on the branch tip (D-key-1 landed). `cargo nextest run --release
  --no-fail-fast`: **2603 / 2606 passed, 182 skipped**; ownership fuzz gate
  2/2, `LOFT_POISON=1` leak suite 49/49, the D-key-1 area poison-clean on both
  backends. **No SIGSEGV, no signal crash, no compile/link failure, no `stores
  not freed`.** The 3 failures are all ENVIRONMENTAL, not safety-gate items and
  not regressions. ✅ `error_messages::baselines_are_locked_in` **FIXED**
  2026-07-04 — it was a network-dependent flake (a `use unknown_module`
  auto-install attempt prints a `[registry] … Dns Failed` line offline / a
  registry-not-found line online, neither in the golden); `normalise()` now strips
  `[registry]` lines, so the baseline is deterministic across CI / offline / the
  sandbox (suite green offline). Remaining 2 are browser-only: `html_wasm` /
  `html_asyncify` (need node + a headless browser — the brittle class
  §CI-hardening addresses); re-run on a browser CI runner to clear before tag.
  None block the safety gate.
- [x] **Zero-leak gate** — ✅ no `stores not freed` warnings across the suite
  run above (both backends); the `LOFT_POISON` leak suite is green.

**B. Public face — the website must not ship stale or broken (elevated 2026-07-04):**

- [ ] **🟢 BLOCKER — Brick Buster `--html` render: BOTH root causes FIXED; pending
  merge + rebuild.** Two layered bugs, both fixed:
  1. **Vector-arg host-import elision** (commit e8e13234, `src/generation/`) — the
     `--html` codegen dropped every `#native` call taking a `vector` (`gl_upload_vertices`
     / `gl_upload_canvas` / `gl_set_mat4`), so no buffers reached WebGL (blank canvas).
     Fixed → geometry draws. Guard `tests/html_gl_imports.rs` + `tools/wasm_imports.mjs`
     (browser-free import-table check).
  2. **Canvas mutations lost** (the "6-colour" residual) — root-caused with a direct GL
     texture probe: every `Canvas` draw method did `d = self.data; d[i]=…`, which @C86
     H-Copy COPIES, so the whole software rasterizer silently drew nothing (the atlas /
     text textures were blank). The maker surfaced a formal contradiction (binding.md
     `B-HeapAlias` "heap aliases" vs heap.md `H-Copy` "heap copies"); a both-backends
     boundary matrix proved H-Copy, and that `&`-write-through was silently ignored for
     a vector lvalue. **Fixed by making `& vector` a writable alias** (commit 348a37f5 —
     a genuine language feature: `d = &self.data; d[i]=…` writes through, non-owning;
     plain bind still copies). binding.md corrected (`B-Copy`/`B-Ref-Alias`/`B-View`);
     guard `tests/scripts/503-vector-reference-alias.loft`. Graphics library adopts it in
     **PR loft-libs-graphics#10** (6 Canvas sites → `&self.data`); proven end-to-end (a
     red|blue `fill_rect` now reads back blue). Also fixed a pre-existing no-`main`
     test-runner crash surfaced en route (commit 0d967c62).
  **Remaining (merge-gated, no code left):** land the loft compiler changes → merge
  graphics PR#10 + republish graphics → rebuild `doc/brick-buster.html` (`make game`) in
  a graphics-enabled env → confirm the render gate passes (≥20 colours) → commit the fresh
  hero. Keep NOT committing a rebuilt hero until that gate is green. Original
  investigation below.
- [ ] ~~**🔴 render REGRESSED; hero cannot be freshly rebuilt.**~~ Investigated 2026-07-04 (network + headless chromium): the
  committed `doc/brick-buster.html` (last good build ~2026-06-13) renders
  correctly (headless render gate PASSES, 128 distinct colours), but **`make game`
  from current source produces a bundle whose canvas renders BLANK** — it clears
  to the background colour and draws NO sprites/content (render gate: `canvas.blank`,
  1 distinct colour, 6 s wait, SwiftShader on). Confirmed a real regression, not a
  timing/GPU artefact, by rendering old-vs-new side by side. **So the staleness was
  a symptom: the hero was frozen at the last working build because rebuilding breaks
  it.** **ROOT CAUSE CONFIRMED 2026-07-04 (NOT @PLN25 — predates it):** the
  `--html`/wasm codegen **drops every `#native` host-import CALL that takes a
  `vector` argument.** Proof — the wasm import table of the last-good build lists
  `loft_gl_upload_vertices` (vertex buffer, `vector<single>`), `loft_gl_upload_canvas`
  (atlas texture pixels, `vector<integer>`), and `loft_gl_set_mat4` (matrix uniform,
  `vector<float>`); a fresh build's import table is MISSING exactly those three,
  while every scalar-arg GL fn (`gl_draw`, `gl_bind_texture`, `gl_set_uniform_float`,
  …) survives. Runtime confirms it: instrumenting the fresh page shows `loft_gl_draw`
  fires (n=6) but `loft_gl_upload_vertices` is **never called** — so `ss_vao` is never
  populated and every `gl_draw(ss_vao, 6)` no-ops. No vertices, no atlas texture, no
  matrices ever reach WebGL → clear works, nothing draws. **The buffers aren't handled
  wrong — the buffer-upload calls are ELIDED.** Fix lives in the `--html` host-import
  codegen (`src/generation/` — the reachability / host-import declaration path,
  `reachable_functions` at `generation/mod.rs:345`): a host import with a `vector<T>`
  param must marshal it to the `(ptr, count)` the `loft-gl-wasm.js` side already
  expects (`new Float32Array(mem, ptr, count)`), not drop the call. **Do NOT commit a
  rebuilt hero until this is fixed** (I did not — kept the working committed build;
  broken rebuild saved at `/tmp/bb-fresh.html`). Once fixed, rebuild + the render gate
  must pass, then the freshness gate (§CI-hardening) keeps it honest. **Structural
  fix still applies: build+deploy the bundle from CI instead of committing it.**
- [ ] **Brick Buster visual gate — via the CPU Canvas atlas, not the browser.**
  Every sprite (ball / paddle / bricks / power-ups / particles) is drawn by
  `build_atlas()` → `graphics::Canvas` (`fill_rect`/`fill_circle`/`vline`/`hline`,
  no 3D — verified 2026-07-04): the **software rasterizer, zero GL**. So golden-test
  `build_atlas().save_png()` (fuzzy-MAE, headless, deterministic) — it exercises
  exactly the drawing primitives that regress (GL is stable; the primitives inside
  it break), at ~one PNG diff, no display. **Ready-made recipe (de-risked this
  session):** the harness already exists — `graphics`'s `native/tests/gold.rs`
  (`gold_compare(example, gold_name, max_abs, mean_abs)`, runs the example under
  `--interpret` so no GL/native-compile, decodes both PNGs, fuzzy-MAE compare,
  `UPDATE_GOLD=1` to capture). Add an `examples/25-brick-buster-atlas.loft` (copy
  `build_atlas()` verbatim + `at.save_png("25-brick-buster-atlas.png")`) and a
  `brick_buster_atlas_matches_gold()` case. **WHERE it must run:** the graphics
  library is *extracted* to `loft-libs-graphics` and its `graphics.loft` depends on
  the `math`/`mesh`/`scene` packages, so rendering needs a **working graphics
  install** — build it in the graphics library's own **library-CI** (or a monorepo
  job that `loft install graphics` first). It CANNOT run in an offline monorepo
  checkout (no registry ⇒ `use graphics` can't resolve its deps — confirmed
  2026-07-04). Keep one browser WebGL confirmation (`make test-html-render`) as a
  release-time check, but it is not the gating one.
- [ ] **Gallery** — regenerate and verify `doc/gallery.html` examples run
  (`scripts/build-gallery-examples.loft`, `gallery-examples.js`); GALLERY_CI
  green (both bundles instantiate — the `gallery` job's Node probe already gates
  js↔wasm consistency).

**CI hardening (this cycle — retires the recurring "broke the WASM bundle /
stale website" class; theme-aligned with `2026-07` library hardening):**

- [x] **The missing freshness gate — LANDED 2026-07-04.** The Node
  instantiate-probe (`tests/html_wasm.rs` for `--html`; the `gallery` job's
  "Probe committed wasm/js pair") already catches js↔wasm *consistency* but not
  *freshness*: a stale-but-consistent bundle passes (proven by the 12-day-stale
  `doc/brick-buster.html` being green). Rather than a rebuild-then-`git diff
  --exit-code` (which risks *flaking* on wasm build non-determinism — the exact
  brittleness that killed the last attempt), the gate is `scripts/check_bundle_fresh.sh`
  + an advisory `bundle-fresh` CI job: **git-only, deterministic** (no build), and
  **PR-scoped** — it fails only a change set that edits a bundle's source
  (`tools/brick-buster/25-brick-buster.loft`) without rebuilding the bundle in the
  same set, so it can never flake and never blocks an innocent PR. Tested on real
  history (flags the current staleness; passes a clean set; skips on an
  unresolvable base). Advisory now; **promote to a required check once item B
  rebuilds the currently-stale `doc/brick-buster.html`**. The gallery wasm bundle
  (whole compiler → wasm) stays covered by the job's rebuild + Node-instantiate,
  not this gate (its "source" is all of `src/`).
- [x] **Move the flaky browser render off the blocking PR path — LANDED
  2026-07-04.** The GPU/headless-browser-flaky binaries (`html_render` =
  Chrome + SwiftShader WebGL; `html_asyncify` = headless-page asyncify resume) are
  now excluded from the per-PR `test` run (a `pull_request`-only filter addition)
  and run in a new **nightly + push-to-main, Linux-only, retry-tolerant** step,
  mirroring the differential-oracle pattern already in the same job. The
  DETERMINISTIC node-based `html_wasm` instantiate-probe (the LinkError /
  import-mismatch catch) STAYS on the PR path — it does not flake. Net: the per-PR
  browser surface is now deterministic-only; the brittle pixel render is visible
  daily but never blocks a PR. *(The Canvas atlas golden that replaces its everyday
  value is the ready-to-build gate scoped in item B above.)*

**C. Library ecosystem — a coherent registry on day one:**

- [x] **Merge PR #18 + republish — DONE 2026-07-04.** PR #18
  (`lima-default-random-0.3.0`) merged to `loft-libs-core` main (commit 2a8aed5b,
  by jjstwerff). Registry republished via `registry_maintain.sh` (commit `bb191e7`
  on `loft-lang/registry`): `arguments` 0.1.3, `random` 0.3.0, `regex` 0.2.1,
  `cbor` 0.1.1, **and** `crypto` 0.3.5 (also stale) — 5 GitHub releases cut, each
  sha256-verified against its tarball, the index re-signed (trust-gate verified,
  the re-sign foot-gun avoided) and pushed. Coverage check now **0 findings across
  21 libraries** — the July registry is coherent.
- [x] **`loft install` smoke — VERIFIED out of band 2026-07-04.** Every check the
  install makes is confirmed against registry `main` directly (bypassing the stale
  raw-CDN): the index carries all 5 new versions (GitHub API), `loft-keygen verify`
  passes on the pushed `index.json` + `index.json.sig` ("signature valid"), and each
  tarball's sha256 matched its entry at publish. The literal `loft install
  <lib>@<version>` is gated only on raw-CDN propagation (~1h) — a time delay, not an
  action; re-run once `raw.githubusercontent.com` catches up (or point a fresh cache
  at the API index).

**D. Release hygiene:**

- [x] **CHANGELOG `2026-07`** — ✅ DRAFTED 2026-07-04 (review before tag). Leads
  with the **breaking type migration** (`text as int/float/single` → `τ?` fallible
  parse, with the `?? default` upgrade recipe), then the retired store-lifetime bug
  class, dense vectors, `&`-binding, the sandbox, browser/`--html`, and a fixes
  round-up incl. the four migrated libs. Verify the prose against the final scope
  before tagging.
- [ ] Bump `Cargo.toml` `2026.6.0` → `2026.7.0`; run the § Per-release ship
  checklist (tag, crates.io publish, per-OS binaries + stdlib checksums).

**Explicitly NOT `2026-07` blockers** (recorded so they are not re-litigated on
release day): open plans/features (@PLN86/87/88/90/91, …) — the release is
stability-gated, not feature-gated; the formal **D-op-1/2** operational
meta-deviations (differential-vs-definitional conformance, not bugs); the
**P54 auto-wrap diagnostic-drop** gap (medium diagnostic-quality, not a crash —
the two-stage `Struct.parse(json_parse(t))` form reports correctly); and the
other demo apps (server / game-client / scene) which ship on their own cadence.
