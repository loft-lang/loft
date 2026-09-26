<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library plans — LEGACY (absorbed into `plans/`)

> **Closed to new work (2026-06-19).** Library plans are no longer separate.
> **All new plans — including library work — are flat files under
> [`../plans/`](../plans/)** (`plans/<n>-<slug>.md`, where `<n>` is the
> [`loft-lang/plans`](https://github.com/loft-lang/plans) `@PLN<n>` issue
> number), in the unified numbering there.  This directory is a legacy archive
> being migrated to `plans/`: an entry that already maps to a `@PLN` issue moves
> + renumbers to it; one not on gh yet gets an issue created first, then
> renumbers.  **Do not add new `lib_plans/` dirs.**  ⚠ Two landed anyway after this was written — @PLN141
(2026-08-18) and @PLN144–147 (2026-08-19, since moved to `plans/`) — because the
loft-plan-workflow skill's binding table still routed *library* plans here.  That row is
fixed; if a new `lib_plans/` dir appears, check whether some other doc is still pointing at
this directory rather than assuming the author ignored this line.

**Distinct from `plans/`.**  `plans/` tracks core-language and
compiler / runtime initiatives (validation matrices, codegen
arcs, language features).  `lib_plans/` tracks **library**
initiatives — `server`, `game_client`, graphics / OpenGL,
regex, package format, asset pipeline, web examples, IDE.
Libraries are ergonomic surfaces built on top of the language;
their planning has different acceptance criteria (API shape,
example coverage, downstream consumer impact) than core-language
work.

## Companion indexes — every parked item is discoverable

- **[`DEFERRED.md`](DEFERRED.md)** — internal index of every parked
  library plan, deferred design decision, and "noted but not now"
  item specific to libraries.  Each row carries an explicit
  `Trigger to unpause:` value.
- Cross-cuts with `plans/`: when a library plan blocks on a
  core-language gap, the row points at the relevant `plans/`
  P-issue or future plan that has to land first.

## Conventions

- A new library initiative opens as a [`loft-lang/plans`](https://github.com/loft-lang/plans)
  issue (`@PLN<n>`, labelled `subject:libs`) — no local slot.  A big multi-phase
  design may add an optional local dir named for the issue (`<n>-slug/README.md`
  stating the goal, phase layout, ground rules + a first phase file
  `00-<first-phase>.md`); small ones live in the issue alone.
- Existing `NN-slug/` dirs predate this and are kept as-is (numbering was
  open-order, independent of `plans/`).
- Every phase plan file begins with `Status: open | in-progress |
  done` so a fresh session can orient quickly.
- When an initiative is fully closed (all phases committed, no open follow-ups),
  set its issue `status:finished` and close it; the local dir (if any) stays in
  place as a closure record.  See [`../plans/_LIFECYCLE.md`](../plans/_LIFECYCLE.md).
- When an initiative is intentionally paused — well-described, no driving bug or
  feature, picked up only when triggered — set its issue `status:future` (keep it
  open).  A deferred library plan differs from a finished one: it's not done,
  it's parked.  Its README + issue must state the **trigger** that would unpause
  it.
- The `future/` / `deferred/` / `finished/` subdirectories are a **legacy
  archive** (closure records only); state now lives on the issue's `status:*`
  label.  The long-term direction is still to drain planned work to zero by
  shipping each plan through to `status:finished`.  May take years; that is fine.
- `deferred/` = **won't do absent a concrete trigger** —
  different intent.  May never run.
- Work flows in one direction: `future/` → current →
  `finished/`, never back to "indefinitely parked."  Promote
  from `future/` as initiatives close.

### Closing a plan — documentation must move out

Same rule as `plans/`.  Apply the 6-step procedure in
[`../plans/_LIFECYCLE.md`](../plans/_LIFECYCLE.md);
the rule's full specification is in
[`../plans/README.md § Closing a plan`](../plans/README.md#closing-a-plan--documentation-must-move-out).

For library plans specifically, the natural reference home
when a plan closes is **inside the library itself**:
- `lib/<name>/README.md` — top-level library reference.
- `lib/<name>/src/*.loft` — inline doc comments on
  user-facing types and functions.
- Per-feature deep-dive docs go alongside the code they
  describe, not in `doc/claude/`.

The `finished/<NN>-<slug>/README.md` keeps only the closure
record; reference content moves to `lib/<name>/`.  Other
docs link to the library, not to the closed plan.

### Authoring a new library plan

Open a [`loft-lang/plans`](https://github.com/loft-lang/plans) issue
(`@PLN<n>`, labelled `subject:libs`) — the issue is the plan, no local slot.
Use [`../plans/_TEMPLATE.md`](../plans/_TEMPLATE.md) for the issue body's shape; a
big design may add an optional local `<n>-slug/README.md` dir.  Library-specific
concerns (downstream consumer compatibility, `lib.toml` schema, etc.) fold into
the Sub-arcs / Open questions sections.

## Ground rule — library plans never break downstream consumers

A library plan's job is to evolve a library's API + implementation
without surprising the consumers (loft programs that import the
library, example galleries, downstream embedders).  Every phase,
and every step within a phase, must:

- Preserve every currently-green test across the full suite,
  including library-specific test scripts.
- Preserve every currently-correct downstream example unless
  the phase explicitly migrates it.
- Either ship a new API or a no-op refactor — never a
  break-now-fix-later bargain on a published surface.

When a phase migrates a downstream example (e.g., the moros
editor moves to a new server API), the migration lands in the
SAME commit as the API change — the example becomes part of
the phase's deliverable, not a follow-up TODO.
*(Superseded: consumer applications are read-only to the loft stream, and a
library change stays additive so no consumer needs migrating —
[LIBRARY_AUTHORING.md § The library contract](../LIBRARY_AUTHORING.md).)*

## Ground rule — file pre-existing bugs surfaced during a library hunt

Same rule as `plans/`: a library phase fixing one feature
routinely surfaces *other* bugs while probing variants — file
those P-issues before the phase closes, not later.  See
[CLAUDE.md § Bug-filing policy](../../../CLAUDE.md#bug-filing-policy--mandatory).

Library-specific notes:
- Bugs in the library code itself stay in PROBLEMS.md (single
  cross-cutting issue tracker).  No separate library issue
  tracker.  *(Superseded: a library bug is filed in its
  `loft-libs-<chunk>` repo's tracker — [LIBRARY_AUTHORING.md § 5e](../LIBRARY_AUTHORING.md).)*
- Bugs in the **core-language layer** that block a library
  phase get a P-id and the library phase pauses (or works
  around with an annotation pointing at the P-id).

## Current library initiatives

Maximum 2-3 library plans in flight at a time.  When a current
plan closes, promote the next-highest-priority library plan
from `future/` into this section.

| Dir | Initiative | Status |
|---|---|---|
| [`19-gridmesh/`](19-gridmesh) | `gridmesh` — shared toolkit of chunk-local, bounded-extent grid→mesh primitives (spatial-index neighbour queries, halo-aware gather, bounded mesh accumulation keyed by owning cell, wired-in dirty-chunk index → per-chunk rebuild, parallel chunk build via `par(...)`).  Extracted from the `audience_crystal` prototype; destined for `moros_render` (per-chunk world meshes with neighbour-pattern wall placement + edge rounding, replacing the global flat-quad `build_hex_meshes`).  Toolkit not framework — each consumer supplies its per-cell rule. | **Active.** Phase A + B done 2026-05-21 (coord + spatial-index primitives; `SegMesh` accumulator; `ChunkField`/`ChunkInput` + halo + wired-in dirty index; G1 incremental per-chunk buckets → O(dirty) rebuild).  **Crystal C1 done** — chunk-driven `SegMesh` build, SET-equivalent to the legacy build cross-mode (`tests/scripts/130`).  In progress: render-group (tile) layer G2 (tunable `group_dim`) + crystal two-level incremental reuse C3 + flat-cost tuning bench.  moros Phase C to follow. |
| [`21-datetime/`](21-datetime) | `time` library + (bonus, deferred) `DateTime` value type — pure-loft date/time operations over `integer` epoch-milliseconds: parse ISO, add days/weeks/seconds, day/second difference, weekday, ISO week, start-of-day/week, fixed-offset local day, and `format_*` text renderers (`YYYY-MM-DD`, `HH:MM`, ISO, weekday name).  UTC + fixed-offset only — no IANA tz / DST.  Civil-calendar math (Hinnant) in pure loft → works identically on interpret / `--native` / wasm with zero core changes.  Driver: the `training` app (`../personal/training`) is date-indexed and its B8–B10 routines are blocked on dates alone. | **Active 2026-05-25.**  Basics built first to unblock the trainer app; the distinct built-in `DateTime` type + built-in `{dt:…}` format specifiers + `js_sys::Date` wasm delegation are kept as the plan's "full datetime support" tail (arcs A/C). |
| [`12-library-extraction/`](12-library-extraction) | Library extraction — execution arc for PKG.EXTRACT.  Move `lib/*/` packages out of the main loft repo into per-family external GitHub repos, each consumable via the package registry.  Includes prerequisite work to **drain the compiler crate of library code** (src/native.rs n_sha256 / web symbols; `lib/imaging/native/` style cdylib paths that today route back through the main crate) — the loft compiler crate must contain ZERO library code or library blueprints, only language core, runtime, codegen, and stdlib symbols.  This direction is the REVERSE of @P321a (crypto-in-codegen_runtime) and the @P321c attempts — those added library code to the compiler crate; this plan removes it. | **Active 2026-05-23 (moved from `future/`).**  Same-day reason: @P321c diagnosis surfaced a recurring pull toward routing library `#native` symbols through `src/codegen_runtime.rs`, replaying the closure-class trap (crypto already shipped that way as @P321a).  Activating this plan reframes the goal: instead of adding more library code to the compiler crate, drain what's already there.  Blocked on PKG.REG (registry MVP, [PACKAGES.md § Open work](../PACKAGES.md#open-work)) for the external-repo move; the prerequisite arc (Phase 1 in the plan README — drain `src/native.rs` of library symbols) is unblocked and starts now. |

## Future library initiatives

Library plans we intend to do — paused, not abandoned.  Each
future plan is a real commitment to finish; the long-term
direction is to drain `future/` to zero by completing each
plan into `finished/`.  Ready-to-resume — pre-flight already
done, design already drafted.  Promote one into "Current" when
a current library plan closes.

| Dir | Initiative | Pre-flight status |
|---|---|---|
| [`57-regex/`](57-regex) | Standalone regex library.  Phase 0 = MVP `#native` cdylib bridge to the Rust `regex` crate (small wrapper exposing `regex(...) -> Regex` / `match_at` / `find_all` / `replace`); Phase 1+ = pure-loft NFA + backtracking-fallback engine per the original design.  Driver: @PLAN37 phase 07's `scan.loft` consolidation and the eventual `check_doc_drift.sh` port both compress dramatically once regex is available — `scan_line()` alone drops from ~150 lines of hand-rolled character walking to a single pattern.  See [bash-scripts evaluation in phase 07 doc](../plans/42-tracker-index/07-loft-native-scanner.md#bash-scripts-evaluation--what-else-benefits-from-loft). | Design fully drafted (Phase 0 specced; was briefly Active 2026-05-18 → demoted to `future/` 2026-05-20 with no phase work started).  `scan.loft` is the first scheduled consumer.  Ready to resume — promote into "Current" when there's room and intent. |
| [`58-graphics/`](58-graphics) | Graphics library bundle — covers 2D RGBA drawing, 3D mesh representation, scene management, multi-backend rendering (desktop OpenGL, browser WebGL, GLB file export).  Four companion files: README.md (top-level library design, opcode integration, performance hooks); IMPLEMENTATION.md (ordered checklist — canvas → GLB → OpenGL → WebGL, GLB-first because it needs no GPU / window / SDK); RENDERER.md (high-level scene-driven PBR layer on top of the low-level `gl_*` API); GALLERY.md (web example gallery + unified rendering across native / WebGL / GLB from one API). | Design (was `doc/claude/{OPENGL,OPENGL_IMPL,RENDERER,WEB_EXAMPLES}.md`).  Low-level `gl_*` API already covers current use cases; the renderer layer is "designed, not scheduled".  Pre-flight: phase ordering and most architectural decisions locked in.  Implementation in 4 staged backends per IMPLEMENTATION.md. |
| [`59-lazy-stdlib/`](59-lazy-stdlib) | Pay-for-what-you-use stdlib cold start.  **Re-premised (2026-05-31):** rather than build a bespoke trigger registry, ship new stdlib modules as `use`-loaded `lib/*` packages — the mechanism already ships (~20 libs use it; `crypto` already moved its natives out of always-loaded core).  A program pays a module's parse cost only when it does `use <name>`. | Design re-premised on `scripting` branch; **no new machinery to build** for the forward-looking win.  REGEX (lib_plans 01) ships as `lib/regex` (loaded via `use regex;`).  Existing `default/*` core (json/stacktrace/coroutine) stays put — extraction costs more than the ~9 % cold-start it saves.  See the plan's Decision section. |
| [`60-asset-pipeline/`](60-asset-pipeline) | Game asset pipeline — three-phase workflow: (1) Claude builds prototype with procedural placeholder sprites / sounds via `fill_rect` / `build_atlas()` etc.; (2) artist creates real assets in external tools (Aseprite for pixel art, Audacity / sfxr for sound, etc.); (3) integration via `load_sprite_sheet()` and friends — code tries the PNG first, falls back to procedural placeholder if missing.  Lets a game look polished early without blocking on art. | Design (was `doc/claude/PIPELINE.md`).  Some loader entry points already exist; the documented workflow is end-to-end designed but not yet exercised across a complete real game.  Brick Buster + moros editor are the natural first consumers. |
| [`61-game-infra/`](61-game-infra) | Game infrastructure grab-bag — designs for items that don't yet have a dedicated design doc.  Sub-arcs: G1-G7 (sprite-sheet loading, drawing, tilemap rendering, 2D collision, sound effects, background music with crossfade, first playable demo game); GL6.6 (keyboard + mouse input via DOM events); W1.1 (single-file HTML export — also covered by HTML_EXPORT.md); FFI.1-FFI.4 (generic `#native` marshaller, generic cdylib loader, eliminate per-fn glue, zero-boilerplate native-fn guide); W-warn (Clippy-inspired developer warnings). | Design (was `doc/claude/GAME_INFRA.md`).  Several items roadmap-scheduled for 0.8.6 (FFI.1-4, W-warn).  This is a kitchen-sink doc; over time individual arcs may split into their own plans. |
| [`06-web-services/`](06-web-services) (@PLN19, [tracker](https://github.com/loft-lang/plans/issues/19)) | Client-side web services library — long-term plan covering JSON (shipped), HTTP client (shipped as the `web` registry package in loft-lang/loft-libs-net), and the broader scope toward "fully functioning": URL handling, auth helpers (Bearer / Basic / OAuth2 / cookies / signed requests), request/response refinements (streaming, multipart, content negotiation, conditional requests, retry/backoff, connection pooling), real-time push (SSE client, WebSocket client), transport/TLS (custom CAs, mTLS, proxies, DNS overrides), diagnostic tooling (mock client, recording, curl dump), and adjacent formats (form-urlencoded, YAML, MessagePack).  Three-file split: README.md (overview + scope + 8-step ship order), JSON.md (shipped reference + future extensions sketch), HTTP_CLIENT.md (locked-in `HttpResponse` + `http_*` design). | Mixed status (was `doc/claude/WEB_SERVICES.md`).  JSON shipped (`Type.parse` + `:j`); HTTP client + WebSocket client shipped as `web` in [loft-lang/loft-libs-net](https://github.com/loft-lang/loft-libs-net); future expansions sketched only.  Server-side (HTTP server, WebSockets) shipped as the `server` package in [loft-lang/loft-libs-net](https://github.com/loft-lang/loft-libs-net/tree/main/server); the fuller framework surface (TLS termination, ACME, RBAC) was declined — see [08-server](future/08-server) (CLOSED) and [DESIGN_DECISIONS.md § C84](../DESIGN_DECISIONS.md#c84--server-ships-as-minimal-tcpws-primitives-not-a-fully-featured-http-framework). |
| [`62-web-ide/`](62-web-ide) | Fully serverless single-origin browser IDE for loft — zero-server, runs from `file://` or any static host.  Full Rust→WASM interpreter, CodeMirror 6 editor, problems panel, console, outline, go-to-definition / find-usages, multi-project IndexedDB storage, docs + examples browser, one-click ZIP export, PWA offline.  Roadmap series W1-W6.  Architecture: pre-bundled JS shell + `wasm-pack`-built loft binary; structured diagnostics in `src/diagnostics.rs`, thread-local output buffer in `src/fill.rs`, virtual filesystem in `src/parser/mod.rs`, JS bridge in `src/wasm.rs`. | Design (was `doc/claude/WEB_IDE.md`).  Deferred past 1.0 per ROADMAP § 1.0.0.  W2-W6 have `✓` design markers; LSP.1 is a prerequisite (the IDE consumes `loft-lsp` for diagnostics + symbol nav).  PACKAGES R1 workspace split is also a prerequisite (for the cdylib WASM target). |
| [`future/08-server/`](future/08-server) | `server` library — HTTP + WebSocket server for loft. The original "fully featured HTTP framework" design (`App`/routing/middleware/JWT+session auth/TLS+ACME/CORS/rate-limiting) was **declined, not built**; the library shipped as minimal TCP/WS primitives (apps route with their own `match`). | **CLOSED (design declined) 2026-07-02.** Shipped in [loft-lang/loft-libs-net `server`](https://github.com/loft-lang/loft-libs-net/tree/main/server) as a minimal single-file TCP+WS server (`listen`/`next`/`respond*`, single-client WS, multi-client `run(on_event)`/`broadcast`). Declined framework design recorded at [DESIGN_DECISIONS.md § C84](../DESIGN_DECISIONS.md#c84--server-ships-as-minimal-tcpws-primitives-not-a-fully-featured-http-framework). |
| [`63-lsp/`](63-lsp) | `loft-lsp` (LSP server) + `loft-dap` (DAP debug adapter) — protocol-agnostic editor integration that unlocks first-class support across every modern IDE (VSCode, Eclipse, JetBrains, Helix, Neovim, Sublime, the future browser IDE, Emacs).  ROADMAP series: LSP.1 (MVP, 0.8.6), LSP.2 (full editing surface, 0.9.0), LSP.3 (DAP interpreter-mode debug, 0.9.0), IDE.ECLIPSE / IDE.JETBRAINS / IDE.NEOVIM (1.0.0 — thin plugins). | Design (was `doc/claude/LSP.md`).  No `loft-lsp` / `loft-dap` binaries in tree yet — pure future plan.  Browser IDE (lib_plans/07-web-ide) consumes loft-lsp once it ships; native debugging (plans/25-native-debug) is the GDB/LLDB complement.  REPL plan (plans/12-repl-and-introspection) shares the introspection surface. |
| [`64-game-client/`](64-game-client) | `game_client` library — multi-player game client.  ROADMAP series GC.1-GC.6: GC.1 WebSocket client + GameEnvelope protocol, GC.2 lobby + matchmaking, GC.3 fixed-timestep game loop, GC.4 client-side prediction + server reconciliation, GC.5 WASM script loading + Ed25519 verification (hot-swap untrusted scripts), GC.6 shared game logic with Tic-Tac-Toe demo.  Companion to lib_plans/08-server (server-side library). | Design (was `doc/claude/GAME_CLIENT_LIB.md`).  `lib/game_client/` doesn't exist yet — pure future plan.  Early `Dispatcher` / `run_game_loop` sketches in this design are SUPERSEDED by plans/23-event-loop's bidirectional handler model; the GC.x sub-features remain valid (they're features the EventLoop layer hosts).  Co-design with plans/24-multiplayer-editor + plans/39-tic-tac-toe (parked 2026-05-11 — which validate the protocol GC.6 demoes). |
| [`65-scriptable-scenes/`](65-scriptable-scenes) | Scriptable scenes — users author loft scripts that drive scene behavior (hex enter/exit/interact in moros editor; analogous hooks elsewhere), edit them in an in-browser IDE, hot-reload without restart, share via scene JSON.  Consolidates 7 ROADMAP rows (SC.1-SC.6 + SC.P) that previously cross-cited each other with no doc home.  Includes 7-phase ship order, sandbox design (script-target build mode disables `use server` / `use file_io` etc.), 5 open design questions (script API style, hook signature evolution, script-to-script comm, resource limits, save state). | Plan-only; no implementation.  Scheduled for 1.0.0 (IDE + multiplayer block).  Depends on lib_plans/07-web-ide (W2 IDE editor shell) + moros editor MO.* milestones (which ship on demo-app cadence). |
| [`66-viewer-lsp-bridge/`](66-viewer-lsp-bridge) | Viewer LSP bridge — sidecar Rust binary `loft-lsp-bridge` consuming rust-analyzer / loft-lsp / jdtls; gives `make view` real multi-language code intelligence (hover, jump-to-def, references, diagnostics).  Three IPC layers (browser↔viewer WebSocket, viewer↔bridge length-prefixed JSON over Unix socket, bridge↔servers stdio JSON-RPC).  Bridge intelligence (warm pool, multiplex, document cache, debounce, crash recovery, structured tracing) is the differentiator that justifies the Rust sidecar.  Designed 2026-05-13 in response to "tooling colleagues will judge by" framing.  CLIENT side; pairs with [`63-lsp/`](63-lsp) (loft-lsp SERVER, consumed by phase 03). | Pure design.  Phase 0 scaffolds the binary + Layer-B socket protocol; phase 1 ships rust-analyzer end-to-end; phase 2 builds the bridge intelligence; phases 3-5 layer in loft-lsp / Java / browser polish; phase 6 closeout.  ~2 quarters of focused work.  Acceptance includes 5 quality metrics (cold start ≤ 2 s warm, hover P95 ≤ 50 ms, multi-language even-handedness, transparent crash recovery, log-surfacing UI). |
| [`71-terrain-heightmap/`](71-terrain-heightmap) | Terrain height-map — slope-based generation, game-agnostic. Artist paints ground TYPES carrying a slope (rock = steep cliffs/mountains, grass = gentle hills, field/floor = flat, water variants meander/current/rapids/waterfall = drop profiles), pins one low point (road exit / waterway / sea shore); a multi-source Dijkstra (Eikonal `\|∇h\| = slope`) integrates the slopes outward to compute every tile's height — replacing the hand-set `RaiseHex`/`slope_path` tedium. The height-FIELD producer that feeds lib-plan 19 gridmesh Phase C meshing. Two consumers: **dryopea** (sci-fi free-build / tower-defence — likely first, far less sculpting) + **moros** (RPG terrain) — both need believable hill sides from painted slope, so the solver is a shared primitive (toolkit, not per-game). | Design drafted 2026-05-21; no code. Only schema add is `md_slope`/`md_drop` on `MaterialDef`; `Hex.h_height` + 3D extrusion already exist. T1 palette → T2 headless solver + cross-mode tests → T3 colored-ground editor test (no buildings) → T4 auto slope-faces → T5 incremental re-solve / FMM. Validate dryopea-first → moros next. |
| [`72-renderer-backend-boundary/`](72-renderer-backend-boundary) | Make the high-level `Renderer`/`Scene` layer (`render.loft`/`scene.loft`) the COMPLETE backend-portable rendering contract, so one GPU backend beneath it serves desktop + web + mobile and no script reaches into raw `gl_*`.  Closes two gaps: (A) embedded GLSL shaders → a portable representation (**WGSL + `naga`** is the all-worlds answer: emits SPIR-V/MSL/HLSL/GLSL-ES, covers Vulkan/Metal/D3D/desktop-GL/WebGL2/WebGPU, pure Rust); (B) custom-shader + framebuffer/post-process escape hatches that currently bypass the Renderer into raw `gl_*`.  **Prerequisite** for a native GPU backend (recommended: a single **wgpu** backend → Vulkan/Metal/D3D/GL/WebGPU, NOT hand-written Vulkan+Metal) and therefore for native Android/iOS.  Surfaced 2026-05-25 evaluating Vulkan/Metal backends ([`future/02-graphics/` § Native mobile backends](58-graphics/README.md#native-mobile-backends-android--ios--evaluation-2026-05-25)). | Design drafted 2026-05-25; no code.  Boundary located + gaps enumerated; shader-IR recommendation (WGSL+naga) settled.  The native wgpu backend is a separate follow-on plan blocked on this.  The trainer's near-term phone path (WebGL-in-webview) needs neither. |
| [`73-universal-editor/`](73-universal-editor) | Universal hex-world editor + library extraction — a coherent set of loft libraries (`hex_grid` / `hex_map` / `hex_render` / `hex_stencil` / `hex_editor` / `hex_entity`) that together provide a **universal editor for hex-world games**.  Each game (moros, dryopea, future indies) consumes the same substrate + registers per-game palette / item / wall semantics on top.  Slice-based extraction from moros: the existing `lib/moros_*` packages are rough-but-unit-tested seed material; each slice (L1-L7) lifts a clean substrate into a neutrally-named shared package, adapts moros to consume it, and lets the second consumer (dryopea) surface bugs.  Strategic unlock for indie / strike-path users — the suite becomes a hex-world game engine, not "moros's neighbour."  Companion to lib_plans/12-library-extraction (which governs WHERE packages live; this plan governs WHAT packages exist + their shape).  Driver: dryopea's plan 06 (editor-to-stencil pipeline) explicitly relies on L1-L6 landing. | Design drafted 2026-05-27; no code.  Phase L0 (architecture spike) is the next concrete step.  Ready when the trigger fires — dryopea plan 06 S1 trigger fires, OR indie / strike-path interest surfaces, OR the substrate is worth extracting on its own merits. |
| [`74-ffi-dispatch/`](74-ffi-dispatch) | FFI generated-dispatch — replace the ~98-arm hand-written interpreter marshal (`src/extensions.rs::dispatch_call`) with a per-function bridge each native library generates from its own Rust signatures (`#[loft_native]` proc-macro).  New signatures/widths never touch loft-core; libraries own their FFI typing.  Promotes FFI.1/FFI.3 out of the `05-game-infra` kitchen-sink.  Only touches the `--interpret` dlopen marshal — `--native`'s direct typed calls are untouched. | Design drafted 2026-05-27 (key linking decision settled by inspection: the bridge lives library-side because native crates are pure-Rust + `loft-ffi` and dlopen'd).  Phases F1-F5.  No code. |
| [`75-physics-2body/`](75-physics-2body) | `physics_2body` — shared rigid-body physics primitives.  Pairwise sphere/AABB collision + integrator; one body vs geometry + one body vs one body (not full N-body piles).  Driven by cross-project audit: three games (moros, dryopea, bumper-airplanes) need the same surface; `lib/moros_sim/collide.loft` becomes the initial population.  Companion to lib_plans/12 Phase 7p (cross-cutting primitives extracted before moros migration). | Slot created 2026-05-28; API sketch + 5-phase implementation outline + open questions filed.  Resumes when consumer-stall lifts. |
| [`76-particles/`](76-particles) | `particles` — ribbon trails + point-burst particles.  Two flavours only (intentionally narrow): ring-buffered trails for plane smoke / scramble exhaust, and short-lived point bursts for score confetti / explosions.  Driven by cross-project audit: dryopea + bumper-airplanes both need both flavours; without a shared package each project would copy-and-fork. | Slot created 2026-05-28; API sketch + 4-phase implementation outline filed.  Resumes when consumer-stall lifts. |
| [`78-loft-distribution/`](78-loft-distribution) | Loft binary distribution + signed installer (`curl \| sh`) + `loft self-update` + key rotation.  Closes the security loop with @PLAN12 § Phase 6.7 — without 30, the advisory channel surfaces "loft 0.8.4 is yanked" but the user has no mechanical fix path.  Five phases (30.1 reproducible builds → 30.2 signed registry entries for binary tarballs → 30.3 installer → 30.4 self-update → 30.5 key rotation + LTS).  Same Ed25519 trust root as the library registry; bootstrap chain matches `rustup`. | **DONE 2026-07-31.**  Delivered in one pass, not five phases: the package registry had already built the signing, key rotation, per-target `BinaryEntry` and advisory feed that four of them proposed.  What remained was an entry, a bootstrap script, and self-replacement. |
## Deferred library initiatives

Library plans well-described but intentionally paused — picked
up only when a concrete trigger arrives.  Distinct from
`future/`, which is "we will finish this, eventually" (real
intent to ship).  Deferred items are "we won't do this unless
something specific changes" — they may never run, and that is
acceptable.

| Dir | Initiative | Trigger to unpause |
|---|---|---|
| _(empty)_ |  |  |

## Finished library initiatives

| Dir | Initiative | Closed |
|---|---|---|
| [`finished/23-http-session-auth/`](finished/23-http-session-auth) | HTTP response headers + cookie-jar `HttpSession` + `base64_encode`/`decode` on `lib/web`.  Unblocked native Garmin login (training port A2). | 2026-05-27 |
| [`finished/29-library-wasm-bridges/`](finished/29-library-wasm-bridges) | Library-owned `--html` extensions — each library carries `wasm/src/lib.rs` (Rust bridge) + `wasm/host.js` (JS host imports) + `[wasm.bridge]` manifest section.  Drained `lib/imaging`'s wasm-bridge from 4 compiler/tooling locations; pattern documented in [PACKAGES.md § Wasm bridges](../PACKAGES.md#wasm-bridges-library-owned---html-extensions). | 2026-05-29 |
