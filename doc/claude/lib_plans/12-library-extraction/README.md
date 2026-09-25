<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library extraction — `lib/*/` → external repos

Move the `lib/*/` packages out of the main loft repository into
per-family external GitHub **chunks**, each consumable via the
package registry.

This is the **execution arc** for **PKG.EXTRACT** in
[ROADMAP.md](../../ROADMAP.md).  The infrastructure (registry MVP,
lock file, format) is the sibling
[PACKAGES.md § Open work](../../PACKAGES.md#open-work).  The durable
"how it works" reference — inventory, stdlib boundary, chunk
topology, dependency graph, per-chunk template, CI path, open
questions — lives in [REFERENCE.md](REFERENCE.md).  **This README is
status + the forward path only.**

## Status

**FINISHED 2026-06-14.**  The drain goal is met: every shared-registry package
is extracted to its `loft-libs-*` chunk and installable; the monorepo carries no
`lib/<pkg>/` directory for any extracted package.  The last three — `html`,
`input`, `markdown` — completed Stage B on 2026-06-14 (`tools/viewer` migrated to
the registry `markdown`, the `lib/` dirs removed, gates green).  Two items are
**out of this plan's registry-drain scope** and routed to their canonical homes:

- **`engine_host`** → the engine/lavition layer (@PLN18 `run_local`/`run`) — it
  is engine core, not a shared-registry library; it leaves `lib/` when the engine
  layer firms up.
- **Prebuilt-native cdylib distribution** (toolchain-free consumers of the native
  chunks) → **@PLN21**.

`audience_crystal` *stays* by design (the documented monorepo-paired fixture),
and the loose `lib/*.loft` files are monorepo-internal, not extracted packages.

Trigger (2026-05-23): @P321c showed the project kept re-adding library code to
the compiler crate; the plan reframed to **drain what's there** and extract by
chunk.

Shipped so far:

- **Decoupling done** (Phases 1–2): crypto + web drained from
  `src/native.rs`; `[native.functions]` manifest is the declarative
  source of truth, `loft-ffi-build` generates the `loft_register!`
  list, `tests/extraction_hygiene.rs` locks the boundary.
- **Registry code complete** (Phase 3, PKG_REGISTRY.md R1–R9):
  `loft package` / `install` / `search` / `info`, lockfile, signing.
- **Trust root BOOTSTRAPPED with `K_real`** (PR #371, 2026-06-14) — the
  interim-`K_tmp` plan is superseded: `TRUSTED_PUBLIC_KEYS` now embeds
  **three independent keys** (1 software laptop + 2 on-card YubiKeys), the
  live index is signed, and `scripts/registry-sign.sh` is the review-then-sign
  path ([REGISTRY_BOOTSTRAP.md](../../REGISTRY_BOOTSTRAP.md)).  Fully active on
  the next loft release.
- **`loft-libs-core` SHIPPED** (Phase 4): `arguments`, `random`,
  `crypto` published + installable end-to-end; their `lib/` dirs are
  removed from the monorepo (residual `lib/random/native/target/`
  build cache cleaned 2026-05-28).
- **`loft-libs-net` SHIPPED (A+B)** (Phase 6 + 6b): `web`, `server`,
  `game_protocol` 0.1.0 (initial) → v0.1.1 patch release 2026-05-31
  (registry PR #5, all three validator gates green including
  reproducible-build re-check); Stage B 2026-05-31 (PR #238 squash
  commit `eff4b01`) removed `lib/web/` + `lib/server/` +
  `lib/game_protocol/` from the monorepo with consumer migration
  (`tests/integration/`, `tools/audience-demo*/`, `tools/viewer/`
  swung to registry-version deps).  Native cdylib build redirects
  to `~/.loft/build-cache/<pkg>-<ver>/` via the shared
  `extensions::native_target_root` helper.
- **`loft-libs-graphics` SHIPPED (A+B)** (Phase 5 + 5b):
  `shapes` 0.2.0, `gridmesh` 0.1.1, `graphics` 0.1.0, `imaging` 0.1.0
  all on registry; monorepo `lib/{shapes,gridmesh,graphics,imaging}/`
  removed (`doc-updates` commits `93ce34a` + `21af961`, 2026-05-31).
  Imaging ships interpreter + native only — wasm bridge deferred
  pending a `loft-host-ffi` crate publish (no current `--html`
  consumer affected).  Consumer migration: `lib/moros_render`,
  `lib/moros_sim`, `lib/audience_crystal`, `tools/audience-demo`
  swapped from path-deps to registry versions with lockfile pins.
  `src/wasm.rs::BUNDLED_LIB_FILES` repointed to fixture clones;
  `tests/extraction_hygiene.rs::FORBIDDEN_LIBRARY_SYMBOLS_MANUAL`
  gained 57 graphics + imaging symbols.
- **`loft-libs-docs` created + `html`/`markdown`/`input` extracted** (2026-06-14):
  new domain chunk [loft-lang/loft-libs-docs](https://github.com/loft-lang/loft-libs-docs)
  for general document/markup libs — `html 0.1.0` + `markdown 0.1.0`; `input 0.1.0`
  → `loft-libs-game` (depends on `graphics`).  GitHub releases cut; registry entries
  + sign pending (`registry_maintain.sh`).  **Stage-B remaining:** migrate
  `tools/viewer` (uses markdown + input) to registry deps, then remove
  `lib/{html,markdown,input}/` from the monorepo.  Surfaced + fixed the
  `loft package` monorepo-URL bug (`[package] repository` field) — see
  [PACKAGES.md](../../PACKAGES.md) / [REGISTRY_SUBMIT.md](../../REGISTRY_SUBMIT.md).
- **Consumer-side surface SHIPPED** (PR #238): `loft new <name>`
  scaffolder (with `--native` / `--chunk` flags); Phase 6.6
  auto-install on `use` + `loft pin` sidecar + walk-up
  `loft.toml` + `loft list-installed`; Phase 6.7 signed
  advisory feed + `loft audit`; Phase 6.8 `loft update`;
  Phase 6.11 `loft bundle export/import` + `file://` URLs;
  Phase 6.12 `tests/fixtures/libs/` + `scripts/sync-fixtures.sh`.
- **Author UX surface SHIPPED** (PR #238): Phase 6.16
  `loft publish` (emit-entry MVP); Phase 6.7a `loft yank`
  (emit-blocks MVP for typed status + advisories.json row);
  `LIBRARY_AUTHORING.md` end-to-end author guide; Phase 6.14
  `loft doc` + canonical `library-ci.yml` gh-pages step;
  Phase 6.15 `scripts/gen_library_catalog.py` catalog generator.

Remaining for the externalised **native** libs (`graphics`, `imaging`, `web`,
`server`) — the "no rustc to *use* them" piece:

- **Native prebuilt distribution** (@PLN21).  Today a consumer without a prebuilt
  compiles the library's cdylib from source on first use (needs rustc + dev
  headers).  @PLN21's producer (`loft build-native` + `prebuild-native.yml`) and
  consumer (`fetch_prebuilt` + `resolve_native_lib`) both ship; the open step is
  the **publish glue** that seeds `index.json binaries[<triple>]` from a release's
  cdylib assets (per-repo caller workflow → `gh release upload` → signed index
  entry).  Scoped to hand-written native libs (auto-compiled libs are
  loft-build-locked).  Concrete steps: [plans/21 § Starting distribution](../../plans/21-prebuilt-native-libs/README.md#starting-distribution--the-publish-glue-remaining-critical-path-step-2026-06-14).
  This is what makes an extracted native chunk fully toolchain-free for consumers.

Recently corrected (this README was stale before 2026-05-26):

- **Phase 3.5b (path-dep resolution) is DONE**, not TODO —
  `manifest::extract_path_dep` is wired into the resolver
  ([`src/parser/mod.rs:4570`](../../../../src/parser/mod.rs)) with unit
  tests in [`src/manifest.rs`](../../../../src/manifest.rs).
- **The graphics/imaging blocker ("task #67", `&ref`-arg store
  forwarding) is RESOLVED** — @P321c's native fix emits
  `to_loft_ref(...)` at the call site
  ([`src/generation/mod.rs:2402`](../../../../src/generation/mod.rs));
  graphics + imaging both pass `native_library_suite` (not skipped).
  So Phase 5 is no longer codegen-blocked.
- **`imaging` migrated to the manifest pattern** (2026-05-26):
  `[native.functions]` + `build.rs` + generated `loft_register!`;
  redundant `#native` annotations + dead `generated.rs` removed.

### Remaining `lib/` inventory → destinations (recorded 2026-06)

Ten packages are still in-repo, and they do **not** all go to the shared
registry — the destination depends on what *kind* of code it is:

| Still in `lib/` | Where it goes |
|---|---|
| ~~`html`, `input`, `markdown`~~ | **✅ EXTRACTED 2026-06-14** — `html`/`markdown` → `loft-libs-docs`, `input` → `loft-libs-game`; published + installable, `lib/` dirs removed (Stage B), `tools/viewer` migrated to the registry `markdown`. |
| ~~`moros_map`, `moros_render`, `moros_sim`, `moros_editor`, `moros_ui`~~ | **✅ MOVED 2026-06-14** to the moros project's own repo (`workspace/moros/lib/`) — game-specific code belongs with the game, not the shared registry.  All 5 packages + their tests left `lib/`; loft keeps fixture clones of the 3 it exercises in its own feature tests (`tests/fixtures/libs/{moros_map,moros_editor,moros_render}`, the same pattern as the graphics/imaging extraction) — the leak, `--html`/WASM, GLB-exit-code, and per-library-namespace (@P379) guards retarget to those.  The substrate-lift framing (hex/chunk → `hex_grid`, already in `loft-libs-world`) was the *option* moros never adopted; it shipped with its own `Hex`/`Chunk` and a possible later migration to the shared `hex_grid` is the moros project's call. |
| `engine_host` | **Engine-layer kernel** (@PLN18 `run_local`/`run`/`run_client`) — belongs with the engine, not the shared registry; exact destination settles as the engine / lavition layer firms up. |
| `audience_crystal` | **Stays** as the monorepo-paired test/demo fixture (the standing exception in § Goal) — the prototype that seeded `gridmesh`. |

Rule of thumb: **shared → registry, game-specific → the game's own repo, engine
core → the engine.**  This is what "drain the compiler crate of *all* library
code" means in practice — it does not force game code into a neutral published
package; it just gets it out of the loft repo into the right home.

## Goal

Every `lib/*/` package (except the monorepo-paired `audience_crystal`)
lives in an external chunk repo, consumable via `loft install`, with
the compiler crate carrying **zero library code** AND the monorepo
carrying **no `lib/<pkg>/` directory for any extracted package** —
verified by the extraction-hygiene gate, identical behaviour across
interp / native / WASM, and `make ci` green with all consumers
resolving extracted packages through the registry rather than via
`path = "../<pkg>"`.

**Two-stage per-chunk workflow** (codified 2026-05-31, see [Phase
6.5 § Bringing a chunk to all-green CI — checklist](#phase-65--green-ci-across-chunks-done-chunk-side-2026-05-31)):

- **Stage A — green extraction:** chunk PR (canonical CI + 6r
  re-clean + warning sweep + tests) → tag + tarball + GitHub
  release → registry PR (all three validator gates green) →
  registry PR merged.  The library is now in the registry catalog
  ("in the manifest") and `loft install <pkg>` works end-to-end.
- **Stage B — monorepo cleanup:** consumers migrated from
  `path = "../<pkg>"` to the registry-version dep
  (`<pkg> = ">=0.1"`); `lib/<pkg>/` deleted; `make ci` green.
  Lands as a SEPARATE PR from Stage A.

Stage A without Stage B is "the library is published but the
monorepo still depends on its in-tree copy" — a real release for
external users, but not the end state.  All three chunks
(loft-libs-core, loft-libs-net, loft-libs-graphics) have completed
both stages as of 2026-05-31.

## Next steps — small verifiable increments

Ordered by readiness.  Each is independently verifiable.  The
PR #238 closure (2026-05-31) shipped every 6.x consumer/author UX
phase + net Stage B; what remains is the longer-arc graphics/world/
moros extraction work plus the closure ritual.

> **Quality gate (2026-07 cycle):** before any remaining `lib/` library is
> extracted or published it must pass the library rules
> ([LIBRARY_CHECKLIST](../../LIBRARY_CHECKLIST.md)).  The per-library worklist —
> inventory, what's checked, and progress — is [stabilization.md](stabilization.md).

1. ~~Migrate `server`/`graphics` to the `[native.functions]` manifest
   pattern.~~  **DONE + superseded** — the manifest was a transitional
   step; the libraries now use the **clean source-scan pattern**
   (bare `#native` in `.loft` → `loft-ffi-build::generate_register_from_loft`
   scans the sources → register list; no manifest).  See
   [REFERENCE.md § Clean libraries](REFERENCE.md#clean-libraries--the-principle).
   `server`/`imaging`/`web` are clean; `graphics` is the documented
   register exception (`CLEAN_REGISTER_EXCEPTIONS`); a parser error
   forbids redundant `#native "n_<fn>"`.  Enforced by the
   `native_libraries_follow_clean_binding_pattern` hygiene gate.
1r. **Phase 6r — re-clean the already-extracted external repos**
   (`loft-libs-core`: random; `loft-libs-net`: web/server).  They
   were extracted *before* the clean pattern landed, so they still carry
   the old manifest / hand-written `loft_register!` and stripped
   `#native` annotations.  Re-clean = **re-sync the now-clean monorepo
   `lib/<name>/` into each external repo** (clean `.loft` + source-scan
   `build.rs` + drop manifest) and bump their `loft-ffi-build` dep to
   `0.2`.  Prerequisites: (a) `loft-ffi-build 0.2.0` on crates.io —
   **published**; (b) bare-`#native` parser support on `main` —
   **landed** (#220 `clean native binding pattern` merged 2026-05-26,
   followed by #221/#223/#224).  **Status:** `loft-libs-core` re-clean
   landed via PR #2 (2026-05-30) — bundled with the YAML refresh and
   the arguments warning sweep.  `loft-libs-net`'s re-clean **landed
   2026-05-31** in net PR #2 (omnibus: canonical YAML + per-symbol
   6r re-clean — 9 `tcp_*` sites stripped to bare `#native`, 16 `ws_*`
   sites kept because the loft fn name genuinely differs from the
   native symbol; warning sweep — 16 `_ = self;` insertions across
   web/server; new `web/tests/byte_at.loft` — 5 tests).  Shipped to
   the registry as `web/server/game_protocol 0.1.1`; gate-3
   reproducible-build re-check green on all three.

   **Per-symbol decision rule** (learned from loft-libs-core 2026-05-30):
   the re-clean **only applies where the `#native "n_<fn>"` string is
   redundant** — i.e. where the symbol equals the function name (loft's
   default).  Where the symbol genuinely differs from the function name
   (`fn sha256_native … #native "n_sha256"` — the loft fn is `sha256_native`
   so it wraps the native `n_sha256`), the explicit string is the
   correct form and must stay.  Crypto fell into this second bucket
   and was left alone; random fell into the first and was re-cleaned.
   When opening a Phase 6r PR, audit each `#native "X"` annotation
   individually — don't sweep blindly.

   **Pre-PR validation — `scripts/verify_external_libs.sh`:** mirrors each
   chunk's `library-ci.yml` (`loft --interpret test` + `loft --native
   test` per package) but builds loft from the **current working tree**
   instead of cloning `main`.  So a loft change the external repos depend
   on (a parser feature, a codegen fix) can be confirmed green *before*
   opening the unblocking `libraries → main` PR.  `--src net=/path`
   validates a local clone (a PR branch) instead of the published repo;
   `--interpret` / `--native` select one step.
2. **Phase 6.5 — green the chunk-repo CI** (`loft-libs-core/graphics/net`
   + registry `pr-validate.yml`): canonical `library-ci.yml` does
   `apt-get install mold` → clone+build loft from source → `loft test`
   + `loft --native test` per package; drop the speculative
   binary-download URL.  *Verify:* each repo's workflow green on push +
   PR; a deliberate one-package-break PR lands red naming the package.
2t. **Phase 6t — library test self-sufficiency** (BLOCKS Phase 5
   and Phase 6w; 6r and 6.5 can run in parallel).  Today 4 of 11
   libraries are not independently testable post-extraction
   because critical coverage lives in monorepo Rust harnesses, not
   in `lib/<name>/tests/`.  Close the gaps before any chunk extracts
   that depends on them.  See [§ Phase 6t detail](#phase-6t--library-test-self-sufficiency)
   below.  *Verify:* for each library, deleting `tests/<harness>.rs`
   from the monorepo leaves the library's regression coverage
   intact (each `lib/<name>/tests/` directory plus, where needed,
   `lib/<name>/native/tests/` for Rust-side integration tests).
3. **Phase 5 — extract `graphics` + `imaging` into
   `loft-libs-graphics`** (codegen-unblocked; gated on 6t for
   `graphics` rendering-regression coverage).  Follow the
   [per-chunk template](REFERENCE.md#per-chunk-extraction-template).
   *Verify:* chunk CI green standalone + `loft install graphics imaging`
   into a scratch dir; monorepo `make ci` green after the Stage-B swap.
4. **Phase 3.6 — stdlib drain**: Image/Pixel/Format types →
   `lib/imaging/`; `escape_html` → new `lib/html/`; path helpers →
   `02_files.loft` (rename from `02_images.loft` — done 2026-05-28; path
   helpers `dir`/`basename`/`join(text,text)`/`resolve` move from
   `03_text.loft` still pending).  *Verify:* `make ci` green;
   `default/*.loft` line count drops (~2,500 → ~2,000).
5. **Phase 7a — split moros into shared `lib/world/` + moros-specific**
   (monorepo-internal; hex addressing, `wall.loft`, `overland.loft`,
   geometry move in).  *Verify:* moros demos render identically;
   every `lib/moros_*/src/*.loft` `use world;` resolves.
   **Status:** partial — `lib/world/` package created with sparse
   32x32-chunk model + save/load (389 lines, smoke test green);
   `lib/wall.loft` + `lib/overland.loft` folded into
   `lib/world/src/` 2026-05-28 (no consumers existed at lib/ root,
   so the move is non-breaking).  Remaining: mark wall/overland items
   `pub`, wire them into the `world.loft` entry, and migrate
   `lib/moros_*` to `use world;` for the hex / chunk primitives they
   currently duplicate locally.

After 7a + 6.5 + 6t: Phase **6w** (extract `loft-libs-world`), then
**7p** (rename + design new cross-cutting slots), **7b** (move moros
libraries into the existing `moros` project), **7c** (bootstrap
`dryopea` — [@PLAN46](../../plans/49-dryopea/README.md)),
**8** (final monorepo cleanup + `audience_crystal` test dir).

`6w` extracts `lib/world` (now subsuming MapFile schema + the
spatial primitives 7a folded in), so dryopea and bumper can both
`loft install world` rather than re-implementing chunk math.  This
is the cross-project payoff for the 7a work.

Phase 5 (graphics + imaging extraction) is codegen-unblocked but
gated on 6t-Tier-2 (gold-image regression must live in
`lib/graphics/native/tests/` before graphics ships externally).

## Cross-project consumers — moros / dryopea / bumper-airplanes

Three game consumers drive the library catalog: **moros** (single-
player RPG, `lib/moros_*` packages), **dryopea**
([@PLAN46](../../plans/49-dryopea/README.md), sci-fi tower-
defence / free-build; depends on `lib_plans/20-terrain-heightmap` +
`lib_plans/19-gridmesh` Phase C; reuses `lib/server` + `lib/web`),
and **bumper-airplanes**
([@PLAN50](../../plans/51-bumper-airplanes/README.md),
audience demo on `origin/bumper_plane`; loads dryopea-authored
MapFiles; proposes `lib/physics_2body`).

### Shared-library matrix

| Library / planned slot | moros | dryopea | bumper |
|---|---|---|---|
| `lib/graphics` | ✓ | ✓ | ✓ |
| `lib/imaging` | ✓ | ✓ | ✓ |
| `lib/server` | — | ✓ | ✓ |
| `lib/web` | — | ✓ | ✓ |
| `lib/game_protocol` | — | ✓ | ✓ |
| `lib/world` (after 7a) | ✓ | ✓ | ✓ |
| `lib/gridmesh` | ✓ | ✓ | ✓ |
| `lib_plans/20` terrain-heightmap | — | ✓ | ✓ (palette → height) |
| `lib/moros_editor` (as authoring tool) | ✓ | ✓ | ✓ |
| `lib/moros_map` (hex/chunk addressing) | ✓ | overlaps `world` | overlaps `world` |
| `lib/moros_sim/collide.loft` | ✓ | similar shape | similar shape |
| `lib/physics_2body` (NEW slot) | — | ✓ | ✓ |
| `lib/particles` (NEW slot) | — | ✓ (explosions, exhaust) | ✓ (trails, confetti) |
| MapFile JSON schema (cross-project contract) | source | consumer | consumer |

### What the overlap implies

**1. Phase 7a is a cross-project unlock, not a moros-internal
cleanup.**  Folding hex/chunk addressing + `wall.loft` +
`overland.loft` into `lib/world/` is what lets THREE projects share
one chunked-map substrate.  Today dryopea + bumper would each
re-implement chunk math after extraction; after 7a they consume
`lib/world` directly.

**2. `lib/moros_editor` is misnamed for its actual scope.**  Bumper
loads "a saved dryopea editor MapFile (or future stencil-pipeline
output)" and dryopea reuses moros's editor + spawn/items markers.
One editor authors maps for three games.  After 7a folds the
spatial primitives into `lib/world/`, the editor logically becomes
`lib/world_editor/` (or `lib/level_editor/`) — a `moros_` prefix on
an editor used by dryopea and the audience demo is documentation
lying about scope.  Rename should happen **before** Phase 7b
(moros migration) ships externally — otherwise three projects fork
the same editor under three names.

**3. The MapFile JSON schema is an implicit cross-project contract,
currently undocumented and buried inside `lib/moros_map`.**  Before
any of the three games extracts, the schema should be promoted to
a stable, versioned format in `lib/world/`, with a single
`world::load_mapfile(path)` entry point.  Otherwise dryopea +
bumper fork their parsers as soon as they leave the monorepo.

**4. `lib/physics_2body` cannot be PLAN50-private.**  PLAN50's
sub-arc 4 introduces it for plane bounce + stall, but dryopea also
needs 3D continuous physics (hover vehicle, enemy ballistics,
scramble-rocket trajectory) and moros's `lib/moros_sim/collide.loft`
is a 2D-ish degenerate case of the same surface.  Allocate a
`lib_plans/` slot so the API design is shared from the start;
otherwise PLAN50 ships bespoke physics and dryopea rewrites it.

**5. The PLAN50 multiplayer QoS layer is reusable.**  Sight-range
filter + three-tier rate-LOD + bounce-forecast is a general-purpose
broadcast primitive ("ship pose state to N receivers, throttle by
distance, send discrete events as forecasts").  Dryopea's planet
multiplayer + co-op base defence wants the same shape.  Belongs in
`lib/server` (or as a thin layer in `lib/game_protocol`) — not in
PLAN50 application code.  Add as an `## Open work` row in
[`../future/08-server/`](../future/08-server).

**6. Particles are a near-certain duplicate.**  Bumper has smoke
trails + score confetti.  Dryopea wants explosions + scramble-rocket
exhaust.  If PLAN50 ships particles as application code, dryopea
copies-and-modifies.  A `lib/particles` slot — ribbon trails +
point-burst particles, two flavours — is cheap to design once.

### Missing lib_plans/ slots (gaps surfaced by this analysis)

| Slot | Status | Why | Blocks |
|---|---|---|---|
| [`lib_plans/75-physics-2body/`](../75-physics-2body/README.md) | **FILED** 2026-05-28 | Shared collision + integrator API for moros / dryopea / bumper | PLAN50 sub-arc 4; dryopea phase 4 (vehicle); Phase 7b clean moros migration |
| [`lib_plans/76-particles/`](../76-particles/README.md) | **FILED** 2026-05-28 | Shared trails + point-burst particles for dryopea + bumper | PLAN50 phase 3 (trails); dryopea phase TBD (explosions) |
| MapFile schema in `lib/world/` (covered by Phase 7a) | DESIGNED below | Cross-project contract; today buried in `lib/moros_map` | Phase 6w (`loft-libs-world` extraction) |
| [`lib_plans/future/08-server/` § Gap 8](../future/08-server/README.md#gap-8--per-recipient-broadcast-qos-sight--rate-lod--forecast) — broadcast QoS layer | **FILED** 2026-05-28 | Sight + rate-LOD + forecast pattern from PLAN50 phase 7; reused by dryopea | dryopea multiplayer; Phase 6r (re-clean already-extracted `loft-libs-net`) |
| Editor rename — `lib/moros_editor` → `world_editor` family (covered by [`lib_plans/73-universal-editor/`](../73-universal-editor/README.md) L1-L5) | EXISTING SLOT | Editor authors maps for three games, not just moros | Phase 7b moros migration (rename **before** moros leaves) |

The two new `lib_plans/` slots (26, 27) carry their own API
sketches + phase breakdowns + open questions; the editor rename
is subsumed by the existing universal-editor plan (slot 24).  The
two designs that live inline in this plan (MapFile schema below,
canonical `library-ci.yml` below) are tied to specific phases here
(7a and 6.5 respectively) and don't merit their own slots.

## Why a separate plan from PACKAGES.md

PACKAGES.md § Open work = INFRASTRUCTURE (registry, lock file,
format) — one focused arc.  This plan = EXECUTION (which library
extracts when, consumer migration, version-sync) — spans multiple
releases, one chunk per release window.  Standing up each chunk repo
(org perms, branch protection, CI secrets, release tagging) is real
admin work; **chunk-extraction phases are serialised, not batched**,
with ≥1 minor release of soak between consecutive chunks.

## Phase summary

| Phase | Scope | Depends on | Status |
|---|---|---|---|
| 1 | Drain library symbols from `src/native.rs` | — | **DONE** 2026-05-24 |
| 2 | Compile-time native-registry aggregator (`[native.functions]` + `loft-ffi-build`) | 1 | **DONE** 2026-05-24 |
| 3 | PKG.REG + cdylib loader | PACKAGES.md | **code complete** 2026-05-24 (bootstrap via `K_tmp`) |
| 3.5a | Dry-run lib without monorepo consumers (crypto) | 1–2 | **DONE** 2026-05-24 |
| 3.5b | Real path-dep resolution (`extract_path_dep` wired) | 3.5a | **DONE** (`src/parser/mod.rs:4570`) |
| 3.5c | Dry-run libs with consumers | 3.5b | superseded — core/net extracted for real |
| 3.6 | Stdlib drain (Image→imaging, `escape_html`→`html`, path helpers→`02_files`) | 1c | **partial** — `escape_html`→`lib/html/` DONE 2026-05-27 (Image/Pixel already in `lib/imaging`); `02_images.loft`→`02_files.loft` rename DONE 2026-05-28; path-helper consolidation (`dir`/`basename`/`join(text,text)`/`resolve` move from `03_text.loft` → `02_files.loft`) remains |
| 4 | Extract `loft-libs-core` (arguments, random, crypto) | 1–3 + 3.5 | **SHIPPED (A+B)** — Stage A 2026-05-24; Stage B (monorepo `lib/{arguments,random,crypto}/` removed) by 2026-05-28 |
| 5 | Extract `loft-libs-graphics` (graphics, imaging, gridmesh, shapes) | 4 + [`../02-graphics/`](../58-graphics) | **SHIPPED (A+B)** — Stage A: shapes 0.2.0 (2026-05-28), gridmesh 0.1.1 (2026-05-31), graphics 0.1.0 + imaging 0.1.0 (registry PR #7 merged 2026-05-31 17:48 UTC); Stage B SHIPPED via 5b row below. |
| 5b | `loft-libs-graphics` Stage B — remove monorepo `lib/{shapes,gridmesh,graphics,imaging}/`; migrate consumers (`lib/moros_*` + `lib/audience_crystal` + `tools/audience-demo`) to registry-version deps | 5 + 6.6 + 6.12 | **SHIPPED** 2026-05-31 (doc-updates commits `408686d` consumer migration + `93ce34a` monorepo cleanup + `21af961` fmt).  Both prerequisites cleared earlier in the session: (1) `loft-ffi-macros 0.1.0` published to crates.io 15:59 UTC; (2) graphics warning sweep complete (148→0 unique source warnings + bind-local + len-guard restructure pattern documented).  Stage A tarballs prepared via `loft package` + pushed to chunk repo + GitHub releases at `graphics-v0.1.0` / `imaging-v0.1.0`; registry PR #7 (`https://github.com/loft-lang/registry/pull/7`) merged after all 3 validator gates green.  Imaging Stage A ships interpreter + native only — wasm bridge deferred pending a `loft-host-ffi` crate publish (no current `--html` consumer affected; follow-up listed at the bottom).  Consumer migration: 4 loft.tomls swapped from path-deps to `">=0.1"`; loft.lock files pinned to registry SHA + version; sibling moros_* deps stay path-resolved (Phase 7-series).  Monorepo cleanup: 93 tracked + 2235 untracked artifact files removed across the 4 dirs; `src/wasm.rs::BUNDLED_LIB_FILES` repointed to `tests/fixtures/libs/{graphics,shapes}/src/*.loft`; `tests/extraction_hygiene.rs::FORBIDDEN_LIBRARY_SYMBOLS_MANUAL` gained 57 graphics + imaging native symbols; `scripts/sync-fixtures.sh` PINNED_REFS extended + per-(chunk, ref) clone bug fixed.  Surfaced @P390 (self-slice-assign drops element values; worked around in `draw_bezier` with temp local).  Verified: `cargo fmt --check` + `cargo clippy --all-targets -D warnings` + full nextest suite (`find_problems.sh --bg`) + extraction_hygiene 4/4 all green. |
| 6 | Extract `loft-libs-net` (server, web, game_protocol) | 4 + [`../08-server/`](../future/08-server) | **SHIPPED (A+B)** — Stage A 2026-05-24/31; Stage B 2026-05-31 (monorepo `lib/web/` + `lib/server/` + `lib/game_protocol/` removed) |
| 6b | `loft-libs-net` Stage B — remove monorepo `lib/web/` + `lib/server/` + `lib/game_protocol/`; migrate `tools/audience-demo*` + `tools/viewer` + `lib/audience_crystal` to registry-version deps (audience_crystal has a `loft.toml`; the `tools/` scripts rely on 6.6 auto-install) | 6 + 6.6 + 6.12 + 6b-prep (native build from registry) | **SHIPPED** 2026-05-31.  Native-build redirect (`extensions::native_target_root` shared helper drives both `auto_build_native`'s cargo invocation + `native_utils::add_native_extern_flags`'s rlib lookup) lets registry-installed cdylibs build into `~/.loft/build-cache/<pkg>-<ver>/`.  Script relocation: `lib/game_protocol/examples/` → `tests/integration/multiplayer/`; `lib/server/tests/server.loft` → `tests/integration/p244_smoke.loft`; `tests/integration/loft.toml` + `loft.lock` pin web/server/game_protocol from registry.  Consumer loft.tomls added: `tools/audience-demo/`, `tools/audience-demo-50/`, `tools/viewer/`.  `extraction_hygiene::manifest_native_functions_cover_drained_libraries` updated — web's 19 symbols pinned in `FORBIDDEN_LIBRARY_SYMBOLS_MANUAL`.  v5_t5_world_tick_and_decay `#[ignore]`'d pending Phase 6w (extract `loft-libs-world`).  All 49 wrap + 22 doc-hygiene + multiplayer v2 (3) + v3 (2) + v5 (4 + 1 ignored) + p244 codegen_emitter + extraction_hygiene (4) tests green |
| 6.5 | Green CI across every chunk + registry repo (canonical `library-ci.yml`); subsumes parked tasks #61/#62/#63 | 4–6 | **DONE (chunk side)** — all three chunks on canonical YAML + at least one patch release published: `loft-libs-core` (PR #2 2026-05-30); `loft-libs-net` v0.1.1 (PRs #2 + #3 + registry #5, 2026-05-31); `loft-libs-graphics` gridmesh 0.1.1 (PR #1 + registry #6, 2026-05-31).  Remaining: registry `pr-validate.yml` already aligned (registry #4 multi-package homepage fix merged 2026-05-31); macOS / Windows matrix expansion is the only chunk-CI delta still on the to-do list (currently Linux-only by design until a Linux baseline soaks) |
| 6.6 | Auto-install on `use` — parser auto-installs from registry when an unresolved `use X;` matches a name in the signed index; one-line announcement on cold cache, silent on cache hit; works WITHOUT a `loft.toml` (script mode) and WITH one (project mode); offline opt-out via `LOFT_OFFLINE=1`/`--offline`/`LOFT_NO_AUTO_INSTALL=1`.  Plus three companion CLIs: `loft pin <script>` (sidecar lockfile), walk-up `loft.toml` detection (project-mode resolution), `loft list-installed` (cache query helper) | 3 (registry MVP), 6.5 | **SHIPPED** 2026-05-31 (libraries7 commits `449d8eb` auto-install + `5702633` pin sidecar + `d0fb2f9` walk-up + `d949379` list-installed).  Resolution chain: user_installed → sidecar_lockfile → project_lockfile (walk-up) → registry_installed (cwd lockfile, script-mode fallback) → auto_install → flat fallbacks.  Design at [registry-resolution.md § Phase 6.6](registry-resolution.md#phase-66--auto-install-on-use-proposed-2026-05-31); harvest into PACKAGES.md § Auto-install behaviour deferred to Phase 6.13 |
| 6.7 | Security advisory channel — typed `status` field on registry entries (severity tier + advisory ID + summary), separate `advisories.json` feed with 24h TTL, loft binary checks installed versions against the feed on every invocation and refuses-or-warns by severity | 6.6 (auto-install — yanked-fix paths invoke the same machinery) | **SHIPPED (loft-binary side)** 2026-05-31 (libraries7 commits `7b79b7b` parser+classifier + `b4479a3` TTL loader + sig verify + `37bf154` `loft audit` CLI + `<new commit>` parser hook + LOFT_SECURITY_OVERRIDE).  Severity-tiered runtime check fires from probe_sidecar_lockfile / probe_project_lockfile / probe_registry_installed; critical aborts with exit 3, high/low warn, deprecated notes.  Registry-side schema bump (typed `status`, `advisories.json` hosting) is still 6.7a + a `loft-lang/registry` PR.  Design at [security.md § Phase 6.7](security.md#phase-67--security-advisory-channel-proposed-2026-05-31) |
| 6.7a | Author-side yank workflow — CLI helper `loft yank <pkg>@<ver> --reason ...` that drafts the registry PR adding the typed `status` entry + the `advisories.json` row; GHSA cross-reference enforcement in `tools/validate.py`; documented author flow in `REGISTRY_SUBMIT.md` | 6.7 (the advisory schema this writes against) | **SHIPPED (emit-blocks MVP)** 2026-05-31.  `loft yank <pkg>@<ver> --severity <tier> --advisory <id> --summary "..." --affected "<range>" --fixed-in "<ver>"` emits both edits the registry PR needs: (1) typed `status` block to splice into the affected version's index.json entry; (2) the cross-referenced row for advisories.json's `advisories[]` array.  JSON-escaped summary handles quotes + backslashes.  Severity validation rejects invalid tiers.  Auto-PR-open (clone registry + splice + `gh pr create`) is the next iteration. |
| 6.8 | `loft update` command — `loft update` refreshes lockfile to latest active versions of all declared deps; `loft update <pkg>` targets one package; project-mode explicit upgrade path that pairs with the 6.7 yank channel | 6.6 (lockfile primitives), 6.7 (advisory feed for "is the new version safe to pick") | **SHIPPED** 2026-05-31.  Walks project (walk-up `loft.toml`) or cwd `loft.lock`; respects per-dep range; skips yanked via `find_best_version`; `--dry-run` / `--check` / scoped-by-pkg modes.  `--major` (range bump) deferred to a follow-up.  Design at [registry-resolution.md § Phase 6.8](registry-resolution.md#phase-68--loft-update-command-proposed-2026-05-31) |
| 6.11 | Offline bundle support — `loft bundle export <pkgs> <outdir>` + `loft bundle import <indir>` + `LOFT_REGISTRY_URL=file://` resolution; stale-advisory thresholds (`LOFT_ADVISORY_MAX_AGE`, `LOFT_ADVISORY_STALE_REFUSE`); makes air-gapped / regulated-environment / classroom-lab deployments first-class | 6.6, 6.7 | **SHIPPED (core)** 2026-05-31.  `loft bundle export --all` / `--packages X,Y,Z <outdir>` writes index + advisories + tarballs + manifest.json; `loft bundle import <indir>` verifies sha256 per tarball + extracts to `~/.loft/registry/`; `http_get_bytes` now handles `file://` URLs so `LOFT_REGISTRY_URL=file://...` is a drop-in mirror.  Stale-advisory thresholds + transitive auto-resolve for `--packages` are follow-ups.  Design at [offline.md § Phase 6.11](offline.md#phase-611--offline-bundle-support-proposed-2026-05-31) |
| 6.12 | Loft-developer offline test loop — `tests/fixtures/libs/` + `scripts/sync-fixtures.sh`; bundled-fixture pattern that survives Stage B's `lib/<pkg>/` removal; eliminates "loft contributor needs internet to run tests" failure mode; mock-registry fixture for testing registry-resolution code paths | Stage B for each chunk (the gap this closes only appears once `lib/<pkg>/` is removed) | **SHIPPED (scaffolding + pure-loft fixtures)** 2026-05-31.  `scripts/sync-fixtures.sh` clones pinned tags + copies sources; `--check` mode for drift detection.  Initial population: arguments, shapes, gridmesh, game_protocol (pure-loft only; native-cdylib packages stay in monorepo until their chunk's Stage B).  `tests/fixtures/mock-registry/` has `index.json` + `advisories.json` for offline resolution-path tests (4/4 passing).  Doc-hygiene gate verifies fixture-dir structural integrity.  Native-cdylib fixtures (random / web / server / crypto / imaging) land when their Stage B is closer.  Design at [offline.md § Phase 6.12](offline.md#phase-612--loft-developer-offline-test-loop-proposed-2026-05-31) |
| 6.13 | Documentation harvest + close-out — extract design content from this plan into permanent reference docs (PACKAGES.md / PKG_REGISTRY.md / new authoring docs); create user-facing onboarding docs (INSTALL.md / SECURITY.md / PUBLISHING.md / USING_LIBRARIES.md); retire stale in-monorepo `lib/<pkg>/` references; CLAUDE.md table surgery; reference audit sweep; split plan into `README.md` (retrospective) + `LANDING_LOG.md`; move to `lib_plans/finished/12-library-extraction/`.  The closure ritual that prevents the plan from "just stopping" with valuable design content trapped in a finished doc no one reads | All previous 6.x phases shipped (6.5 + 6.6 + 6.7 + 6.7a + 6.8 + 6.11 + 6.12 + 6.14 + 6.15 + 6.16) + Stage B done for all three chunks (core ✓ already; 5b + 6b pending) | **IN PROGRESS** — [LANDING_LOG.md](LANDING_LOG.md) seeded 2026-05-31 with every shipped commit through Phase 6.12; PR #238 closure entry (2026-05-31, squash `eff4b01`) records every 6.x consumer + author-UX phase landing on main.  LIBRARY_AUTHORING.md author-facing walkthrough shipped same day.  Remaining closure work: permanent-doc harvest into PACKAGES.md / PKG_REGISTRY.md, CLAUDE.md table surgery, reference-audit sweep, move to `lib_plans/finished/`.  Fires when only longer-arc items (5/5b/6w/7-series) remain. |
| 6.14 | Library documentation pipeline — chunk-repo HTML doc generation; analogue of `cargo run --bin gendoc` for libraries that live OUTSIDE the monorepo; per-version published to a per-chunk gh-pages site OR aggregated at `loft-lang.org/libraries/<name>/<ver>/`.  After Stage B, the existing monorepo `gendoc` has no library source to read; this fills that gap | 5b + 6b (Stage B work that removes monorepo lib sources), Stage A complete for the library being documented | **SHIPPED (loft-binary side + CI template)** 2026-05-31.  `loft doc <path>` was already wired as PKG.8 — reads `<pkg>/loft.toml` + `<pkg>/src/*.loft` + optional `<pkg>/docs/*.loft` topic pages, emits HTML to `<pkg>/doc/`.  Verified against the gridmesh fixture: `doc/index.html` + `doc/api-general.html` generated cleanly.  `library-ci.yml.example` gained a "Generate per-package docs" step + a tag-gated "Publish docs to gh-pages" step (URL pattern `loft-lang.github.io/<chunk>/<pkg>/<ver>/`).  Chunk-repo rollout (apply the new YAML) is a per-chunk PR; lands incrementally.  Cross-package link generation + `<pkg>/latest/` redirects are follow-ups.  Design at [library-docs.md](library-docs.md) |
| 6.15 | Library catalog page generator — `scripts/gen_library_catalog.py` that pulls `index.json` from the registry and writes `doc/library-catalog.md` (and an HTML view at `loft-lang.org/libraries`); auto-update via CI on registry change; one page listing every published library with one-liner descriptions, current active versions, license, and link to docs (Phase 6.14) | 6.7 (status info for yanked entries), 6.14 (doc URLs to link to) | **SHIPPED** 2026-05-31 → **RETIRED 2026-07-02** (a stale, un-CI-guarded duplicate of the agent-facing `doc/claude/LIBRARIES.md`; `gen_library_catalog.py` + `doc/library-catalog.md` deleted).  Python script reads `index.json` (live HTTPS, file://, or local path) and emits a categorised markdown table — packages sorted within each category; one-liner description + latest active version + link to chunk-repo homepage.  `--check` mode for CI drift detection.  Verified against live registry + the mock-registry fixture.  Registry-side CI auto-update + HTML rendering at `loft-lang.org/libraries` are follow-ups.  Design at [closure.md § Library catalog generator](closure.md#phase-615--library-catalog-page-generator-proposed-2026-05-31) |
| 6.16 | `loft publish` command — CLI helper that reads `loft.toml`, verifies CI green at the tag, computes sha256 + size, opens a registry PR via the GitHub API (or generates the PR body for manual `gh pr create`); the missing piece that closes the authoring-friction gap vs `cargo publish` | 6.5 (CI), 6.6 (install primitives reused by some flows) | **SHIPPED (emit-entry MVP)** 2026-05-31.  Re-packages locally via `package::package_create` (deterministic); auto-detects chunk repo from `git remote get-url origin`; verifies the GitHub release at `<pkg>-v<ver>` carries the expected asset (via `gh release view`); emits the `index.json` entry block ready for paste into a registry PR.  `--dry-run` skips the GH verification.  Auto-PR-open (clone registry + splice index.json + `gh pr create`) is the next iteration; MVP closes the friction gap by eliminating manual sha256 + size computation. |
| 6w-w | Retire every `lib/*/.allow_warnings` opt-out — clean each library's warnings until the gate runs strict everywhere | 6.5 (gate landed) | **partial — 2026-05-31** — `lib/graphics/` retired via warning sweep (doc-updates `177453e`, 148 unique source warnings → 0 + opt-out file deleted); `lib/shapes/` retired by removal (Phase 5b-5, dir gone).  Remaining monorepo libs with `.allow_warnings`: `lib/moros_map/` (4), `lib/moros_editor/` (6), `lib/moros_render/` (31), `lib/audience_crystal/` (40), `lib/moros_ui/` (155), `lib/moros_sim/` (173) — total 409 unique warnings across 6 libs.  Lint improvements (doc-updates `639d546`) make these sweeps easier — position attribution now points at the right function; AND-conjunction guards (`if a<len(u) and b<len(v)`) lift automatically.  Surfaced @P390 along the way (self-slice-assign drops element values; PROBLEMS.md row 390 with workaround). |
| 6t | Library test self-sufficiency — Tier 1 gridmesh script copies, Tier 2 `graphics_gold.rs` port, Tier 3 `multiplayer_v{2,3,5}.rs` port, Tier 4 `loft test --deps`, Tier 5 (NEW) coverage gaps with no Rust-harness home (`imaging` / `world` / `markdown`) | 4–6 | **partial** — Tiers 1+2+4 DONE; Tier 3 OPEN; Tier 5 OPEN; blocks Phase 5 (`imaging`), Phase 6r re-clean (Tier 3), Phase 6w (`world`) |
| 6w | Extract `loft-libs-world` (world, Phase-7a-expanded) | 7a + 6.5 + 6t | OPEN — M |
| 7a | Split moros: shared spatial primitives → `lib/world/` (cross-project unlock — feeds dryopea + bumper + moros; subsumes MapFile schema promotion) | 4 | **partial** — `lib/world/` shipped (sparse 32x32 model + save/load, smoke test green); `lib/wall.loft` + `lib/overland.loft` folded into `lib/world/src/` 2026-05-28; remaining: `pub` markers + moros migration + MapFile schema doc |
| 7p | Cross-cutting primitives extracted before moros leaves: `lib/moros_editor` → `lib/world_editor` rename + `lib_plans/NN-physics-2body` + `lib_plans/NN-particles` slots filed; sequencing matters more than effort | 7a | **OPEN** — XS (rename) + M (new slot designs) |
| 7b | Move moros libraries into the existing `moros` project | 5 + 6w + 7a + 7p | OPEN — MH |
| 7c | Bootstrap `dryopea` project | [@PLAN46](../../plans/49-dryopea/README.md) | OPEN — S |
| 8 | Final monorepo cleanup + `audience_crystal` test dir | 7b | OPEN — S |

Phase 7a is monorepo-internal (no user-visible change) and can land
before the registry work; 6w then interleaves with the remaining
chunk extractions once 7a is stable.


## Phase detail

Open-phase design content is split by topic across companion
files (this README is status + phase summary + cross-project
context only).

| File | Phases covered | Topic |
|---|---|---|
| [ci-and-warnings.md](ci-and-warnings.md) | 6.5, Bringing-a-chunk checklist, 5b, 6b, 6w-w | Canonical `library-ci.yml`, per-chunk omnibus pattern, Stage A/B sequencing + per-chunk Stage B execution, `.allow_warnings` ratchet |
| [registry-resolution.md](registry-resolution.md) | 6.6, 6.8 | Auto-install on `use` (Python-style for scripts; Cargo-style for projects); `loft update` command |
| [security.md](security.md) | 6.7 | `advisories.json` signed feed, typed severity tiers, classifier fail/warn behaviour, verify-on-recompile timing |
| [authoring.md](authoring.md) | 6.7a, 6.16 | Author-side workflow — `loft yank` (advisory submission) + `loft publish` (registry PR helper) |
| [offline.md](offline.md) | 6.11, 6.12 | `loft bundle export/import`, `LOFT_REGISTRY_URL=file://`, stale-advisory thresholds, loft-developer fixture pattern |
| [library-docs.md](library-docs.md) | 6.14 | Chunk-repo HTML doc generation pipeline (post-Stage B replacement for the in-monorepo `gendoc`) |
| [closure.md](closure.md) | 6.13, 6.15 | Documentation harvest + close-out ritual; library catalog page generator |
| [LANDING_LOG.md](LANDING_LOG.md) | (chronological — all phases) | Per-commit landing record.  Becomes the finished plan's log on close.  Append-only; never reorder. |
| [stdlib-drain.md](stdlib-drain.md) | 3.6 | Scope hygiene + CVE-surface lever; what stays embedded permanently |
| [test-coverage.md](test-coverage.md) | 6t (Tiers 1-5) | Per-library test self-sufficiency; multiplayer harness port; @P389 cross-package-link blocker |
| [moros-split.md](moros-split.md) | 7a, 7p, 6w, 7b, 7c, 8 | Cross-project consumer thread: shared `lib/world/`, physics/particles slots, moros + dryopea project moves, final monorepo cleanup |

Shipped phases' build records live in git history + CHANGELOG;
the docs above are for OPEN phases only.

## See also

- [REFERENCE.md](REFERENCE.md) — inventory, stdlib boundary, chunk
  topology + dependency graph, store-allocating cdylib pattern, CI
  path, per-chunk extraction template, open questions.
- [PACKAGES.md § Open work](../../PACKAGES.md#open-work) — registry +
  format infrastructure (prerequisite).
- Sibling library plans: [`../02-graphics/`](../58-graphics),
  [`../08-server/`](../future/08-server), [`../21-datetime/`](../21-datetime);
  game plan [@PLAN46 dryopea](../../plans/49-dryopea/README.md).
- [ROADMAP.md](../../ROADMAP.md) — PKG.EXTRACT milestone placement.
