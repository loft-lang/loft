<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# The loft Workbench — a multi-language, server-backed browser IDE

> **Identity:** a design doc for the *combined* browser IDE. It is the multi-language
> generalisation of [`IDE.md`](IDE.md) (@PLN16 M5e, the server-backed loft IDE) fused with
> [`66-viewer-lsp-bridge`](../../lib_plans/66-viewer-lsp-bridge/README.md) (the multi-LSP
> client) and extended past loft to **Rust and C**.
> **Status:** design proposal. Nothing new built yet; the loft half is largely *shipped*
> (serve.rs slices 1–5, `loft-lsp`/`loft-dap`) — the Workbench is mostly *assembly + one
> generalisation*, not green-field. To become a plan it needs a `loft-lang/plans` issue
> (not filed — needs the owner's go-ahead).
> **Not** the serverless WASM playground ([`62-web-ide`](../../lib_plans/62-web-ide/README.md));
> see § 3 and § 10.

## 1. The thesis

> **A server that runs native tools, and a browser that is the interface to those tools.**

That is the whole product. The important thing is that **we already have that server — it
just drives one tool.** `src/serve.rs` (@PLN16 M5e, *landed*) is a local HTTP + WebSocket
server whose browser front-end edits source, sets breakpoints, and runs/steps/inspects — by
sending **one JSON message per WebSocket frame** into the **same `crate::rpc::handle` engine
driver** the terminal `--rpc` uses. The browser holds no interpreter and no language logic;
the real loft engine runs locally and streams state back.

The Workbench is that exact pattern **generalised from one tool / one language to N tools /
three languages**. Nothing in the model changes. You keep the transport and the "one message
= one native-tool call" invariant, and you replace the single hard-wired loft engine with a
**dispatch table of native adapters keyed by file extension**, spanning three capability
planes (intelligence, execution/debug, build). Adding a language becomes *registering a
trio of adapters* — never touching the shell, the transport, or the protocol envelope.

This doc's job is to (a) inventory what already exists, (b) settle the one real architectural
fork (server-backed vs serverless), (c) define the generalisation seam, and (d) give a
small-step, independently-landable build spine.

## 2. What already exists (the pieces we are combining)

Almost every part is built or designed; the Workbench is mostly assembly. Grounded in the
investigation of the tree:

| Piece | What it gives | Role in the Workbench | Status |
|---|---|---|---|
| **`src/serve.rs`** (@PLN16 M5e slices 1–6) | HTTP+WS server on `127.0.0.1`; editor + dual console + debugger panels + test runner + REPL + game launcher; `writeFile` sandbox | **The shell + transport + loft execution/debug plane.** The whole server we generalise. | **Landed** (1–5; 6a) |
| **`src/rpc.rs` `handle()`** | "one message ⇄ one engine method", transport-agnostic — already runs over stdio-NDJSON (`--rpc`) *and* WebSocket (`--serve`) | **The protocol spine.** The verb/event envelope every plane rides. | **Landed** |
| **`loft-lsp` / `loft-dap`** (@PLN63, shipped this session) | native LSP + DAP for `.loft` (hover/def/refs/rename/format/completion/diagnostics; breakpoints/step/variables **VE**/stack **SF**/data-bp **DB**/reverse **RX**) | **The loft intelligence + debug adapters.** Ready today — this *unblocks* @PLN66 phase 03. | **Shipped** |
| **The viewer** (`tools/viewer/`, plan-35) | router on `req.path`; file-tree (`/tree`), breadcrumbs, page chrome; **Markdown render** (`markdown::render`); **git-aware** routes (`/commit/<sha>`, `/diff/<path>`, `/raw/`) backed by a git bridge (`refresh.sh` → `state/*.json`) | **The browse/tree half + Markdown preview + git surface**, grafted onto serve.rs's edit half. | **Runs** (`make view`) |
| **@PLN16 M5e slice 6** (`launchGame`/`reload`) + [`58-graphics`](../../lib_plans/58-graphics/README.md) + @C71/N9 | launch a program in a **native OpenGL window**; hot-swap one function into the *running* game over the shared store | **The live-authoring plane** (§ 9): run the game, edit scripts on the fly. | 6a landed; 6b/6c gated on @C71/N9 |
| [`65-scriptable-scenes`](../../lib_plans/65-scriptable-scenes/README.md) + [`64-game-client`](../../lib_plans/64-game-client/README.md) | visual scene/world definition + the game client runtime | **The world editor** (§ 9). | Design |
| **`loft-lsp-bridge`** (@PLN66, designed) | multi-LSP fan-in (rust-analyzer + loft-lsp + jdtls) over 3 IPC layers; bridge "intelligence" (warm pool, multiplex, cache, debounce, crash-recovery, extension routing, tracing) | **The multi-language INTELLIGENCE plane** (Layers A/B/C). Absorbed as a sidecar. | **Design only** |
| **`62-web-ide`** (@PLN62, designed) | serverless in-browser WASM loft interpreter; CodeMirror 6 editor; loft Lezer grammar; in-WASM `get_symbols` | **NOT this product** — the offline sibling. Donates only the **CM6 editor + loft grammar**. | **Design only** |
| loft's `rustc`/`cargo` plumbing (`src/main.rs`, `src/native_utils.rs`, `src/native_lib.rs`) | loft already shells to `rustc`/`cargo` heavily for `--native` (generate Rust → compile with the user's rustc → link the runtime rlib) | **Precedent + reusable build-space assumption** for the Rust build plane. | **Shipped** |

Two facts from the transport investigation pin the shape:

- **`loft-lsp`/`loft-dap` run native and are proxied over WebSocket — not compiled to WASM.**
  They are blocking-stdin binaries; the browser has no stdin/spawn. The *loft library* they
  link (`loft::lsp`, `loft::rpc`, the interpreter) *does* compile to wasm (three worlds:
  `--html`, `--native-wasm`, `make wasm`), so an in-browser variant is a *future* option after
  rewriting their stdio `main` into a wasm-bindgen call API — a bounded but real rewrite,
  explicitly out of scope here.
- **The server-proxy path already exists and is proven** — serve.rs *is* "native engine,
  WebSocket to browser." Adding LSP-framed or DAP-framed endpoints is the same move applied to
  new tools.

## 3. The one architectural decision — server-backed, not serverless

The investigation surfaced **three inconsistent stories** for browser language-intelligence,
and they must be collapsed to one:

1. **@PLN62's in-WASM `get_symbols`** — serverless, no LSP, symbols baked into the WASM.
2. **"`loft-lsp` compiled to WASM"** — an undesigned, unbacked cross-reference.
3. **@PLN66's server-side bridge over WebSocket** — designed, but requires a local server.

**Your ask resolves it by construction.** The Workbench must host **rust-analyzer, rustc,
clangd, gcc, and lldb** — native tools with *no in-browser WASM form*. You cannot run gcc in a
browser tab. Therefore the Workbench is **server-backed** (story 3), and @PLN62's serverless
playground stays a *separate product* (zero-install teaching/demo, offline via a service
worker, no real toolchain). They share only the CM6 editor and the loft grammar.

This is not a compromise — it is forced by the toolset, and it mirrors M5e's own honest seam:

> **The browser holds no toolchain. The native tools (loft engine, rust-analyzer, rustc,
> clangd, gcc, lldb) run locally against the real filesystem. The browser edits source, shows
> results, and sends intents — it never compiles, analyses, or executes anything itself.**

Native GUI output (a loft game window, a Rust/C windowed or TUI program) renders in its **own
native surface**, exactly as M5e keeps the game window native; the browser *controls* it
(run/stop/reload) but does not display its pixels. There is no frame-streaming surface to
build or secure.

## 4. The core model — one server, N adapters, three capability planes

The browser speaks **one protocol to one local server**. The server owns a **dispatch table
keyed by file extension → language → an adapter trio**. Every language contributes three
adapters, one per capability plane:

| Plane | Browser sees | loft | Rust | C |
|---|---|---|---|---|
| **Intelligence** (LSP) | hover, go-to-def, refs, completion, diagnostics, rename, format, outline | `loft-lsp` | `rust-analyzer` | `clangd` |
| **Execution + Debug** (DAP-shaped) | run, breakpoints, step, variables, watch, call-stack, reverse | loft engine via serve.rs / `loft-dap` (VE/SF/DB/RX) | `cargo run` + `codelldb`/`lldb-dap` | `gcc`→exe + `lldb-dap`/`gdb`-DAP |
| **Build** (compiler CLI) | check/compile, test, artifacts, streamed diagnostics | `loft compile` / `loft --native` | `cargo` / `rustc` | `gcc` / `clang` |

**Adding a language = registering one trio.** The shell, the transport, the protocol envelope,
and the panels never change. That single seam is what turns three bespoke integrations into
"a functional project."

### Topology

```
 ┌──────────────── Browser (thin IDE, CodeMirror 6) ────────────────┐
 │  file-tree (viewer) · tabs · editor · gutter breakpoints          │
 │  toolbar [Run ▶ Build Test Debug]  · dual console · debug panels  │
 │  vars/watch/stack (VE·SF·DB·RX) · REPL                            │
 └───────────────────────────▲──────────────────────────────────────┘
             A: one WebSocket │  {kind: lsp|debug|build, file, …}  (localhost / SSH-forward)
                              ▼
 ┌──────── Gateway:  `loft ide --serve <port> --lib …` (serve.rs, generalised) ────────┐
 │  router by {kind, ext} → adapter table                                              │
 │  • Execution/Debug + Build + FS  ── owned here (ReplSession for loft; spawns         │
 │    cargo/rustc/gcc; speaks DAP to lldb-dap; drives loft-dap in-process)              │
 │  • Intelligence (LSP) ── proxied ↓                                                   │
 └───────────────────────────▲──────────────────────────────────┬──────────────────────┘
        the project on disk   │ B: Unix socket (length-prefixed  │ spawns build/debug
        (src/ lib/ tests/)     │    JSON, @PLN66)                 │ children + native
                              ▼                                   ▼ windows (game/TUI)
 ┌──── loft-lsp-bridge (sidecar, @PLN66; warm pool, multiplex, cache) ────┐
 │           C: stdio JSON-RPC  (Content-Length framing)                  │
 │      ┌──────────────┬──────────────┬──────────────┐                    │
 │      ▼              ▼              ▼                                     │
 │  rust-analyzer   loft-lsp        clangd     (+ jdtls later)             │
 └────────────────────────────────────────────────────────────────────────┘
```

## 5. The unification insight — M5e and @PLN66 are the *same* server

@PLN16 M5e's rpc/WebSocket server and @PLN66's `loft-lsp-bridge` are the **same architecture**
— thin browser, local server, native tools, "one message = one native call" — *designed twice*
for two different reasons (M5e: loft run/debug/test/game; @PLN66: multi-language read-only
intelligence for the viewer). The Workbench **merges them into one gateway** that multiplexes
**both protocol families over the single browser WebSocket**, tagged by `kind`:

- `kind:"lsp"` frames (intelligence) → forwarded down **Layer B** to the sidecar bridge, which
  routes by extension to the right stdio LSP server (Layer C). This is @PLN66 verbatim.
- `kind:"debug"` / `kind:"build"` / run frames → handled **in the gateway**: the loft
  `ReplSession` (today's serve.rs path) for `.loft`; a spawned `cargo`/`gcc` build child and a
  `lldb-dap` DAP child for `.rs`/`.c`. This is M5e's protocol extended per-language.

**Why keep the bridge a separate sidecar** (rather than fuse it in-process): @PLN66's
rationale holds unchanged — the **warm pool survives IDE restarts** (rust-analyzer's ~30 s
indexing is paid once, not every `Ctrl-C`), it has an **independent release cycle**, and it
**multiplexes across tabs** (two tabs share one rust-analyzer). The gateway stays simple
(execution/build/FS + a dumb LSP proxy over Layer B); all LSP concurrency/crash-recovery lives
in the sidecar. This also side-steps serve.rs's single-threaded-blocking limit for the
intelligence plane early (§ 7, seam 4).

Net: **one browser WebSocket → one gateway → (a) an in-gateway execution/build/FS plane and
(b) a sidecar LSP fan-in → N native tools.** loft already implements every loft leg; Rust and
C are new adapter trios behind the same seams.

## 6. The editor — CodeMirror 6, settled

CodeMirror 6, consistently chosen by @PLN62, @PLN66 (05-browser-editor), *and* M5e's editor
graduation note. serve.rs today ships a raw `<textarea>` + line gutter (slice 3); the
Workbench upgrades that pane to **CM6 with an LSP-over-WebSocket client**
(`@codemirror/lsp-client` / `codemirror-languageserver`) pointed at the gateway's `kind:"lsp"`
frames. Grammars: reuse @PLN62's loft Lezer grammar; Rust and C get community CM6 grammars.
Monaco (`monaco-languageclient`) is the heavier alternative — **CM6 is the recommendation** for
bundle size, modular plug-ins, and co-hosting with the `--html` build, matching both prior
design decisions. All assets are **bundled, not CDN-loaded** (offline posture + no external
network, matching @PLN62's PWA stance). Panels are grafted: editor/console/debug/test/REPL from
serve.rs, file-tree/breadcrumbs from the viewer.

## 7. The seams to close (concrete work)

From the serve.rs investigation, six generalisation seams — each already *named* in M5e's own
"Library vs project" section, so this is not new discovery:

1. **Multi-file / workspace.** `writeFile` is sandboxed to *one canonical path*
   (`set_workspace_file`). Widen it to a **workspace root** (canonicalise-under-root check).
   M5e already flags this as **THE library-development blocker**, so the work is scoped and
   motivated. Add the viewer's file-tree + breadcrumbs + tabs, and thread `{file}` through every
   event so panels route per-file.
2. **Per-language backend behind the verb set.** The rpc verbs are already language-agnostic in
   *shape*; the single seam is the hard-wired `ReplSession`. Replace it with the § 8 adapter
   table (loft = today's engine as the first impl).
3. **Multiplex LSP + debug/build on one socket.** Tag frames by `kind`; route in the gateway.
   Testable in the `tests/rpc.rs` / `tests/serve.rs` shape.
4. **Async / multi-client gateway.** serve.rs is blocking single-threaded (fine for one tab).
   Mitigation: push LSP concurrency into the **sidecar bridge from WB1**, so the gateway stays
   simple until a dedicated hardening phase (WB7) makes it per-connection/async.
5. **Frontend: inline Rust string → served static asset.** `GET /` already exists; swap
   `render_shell`'s token-replaced inline HTML for a served CM6 app bundle.
6. **Session model.** One `ReplSession` for the process → a session/engine **per language (and
   per open file/workspace)**, owned by the adapter.

## 8. Build spine — small, independently-landable phases

Each phase: a gate you can demo, what it **reuses**, what it **builds**. Every phase leaves a
usable IDE. Tests follow the shipped shape: drive the messages over a socket and assert JSON
(`tests/rpc.rs` / `tests/serve.rs`) + a headless-chromium smoke.

| Phase | Gate (demo) | Reuses | Builds |
|---|---|---|---|
| **WB0 · Gateway + CM6 shell** | Open browser, edit + Run a `.loft` file in **CodeMirror**, output streams — parity with today's textarea IDE. | serve.rs slices 1–2 whole | Served static CM6 app (seam 5); CM6 + loft grammar wiring |
| **WB1 · loft intelligence** | Hover a loft symbol → signature; go-to-def jumps; live diagnostics/completion/rename/format — all in-browser. | `loft-lsp` (shipped); @PLN66 Layers B/C pattern; CM6 lsp-client | `kind:"lsp"` multiplex on the WS (seam 3); the sidecar bridge + Layer-B hookup (@PLN66 ph.00 + ph.03) |
| **WB2 · loft debug** | Gutter breakpoint → pause; step; **variables tree (VE)**, **multi-frame stack (SF)**, **data breakpoint (DB)**, **reverse (RX)** — in CM6. | serve.rs slice 4 + the loft-dap advanced engine (VE/SF/DB/RX, shipped) | CM6 debug UI re-skin (panels exist) |
| **WB3 · multi-file workspace** | Open a multi-file loft package, edit across files, run the whole **suite**. | viewer `/tree` + breadcrumbs; M5e `runSuite` (package-aware) | Widen `writeFile` sandbox → workspace root (seam 1); tree/tabs/ per-file routing |
| **WB4 · the adapter abstraction** | loft still works end-to-end, now through a `LanguageAdapter` indirection; the dispatch table has **one** entry. | everything | The `LanguageAdapter` trait (intelligence + debug + build) + extension→trio table (seams 2, 6) — **the load-bearing generalisation** |
| **WB5 · Rust** | Open `.rs` → hover/def/diagnostics (rust-analyzer); **Build/Run** via cargo; breakpoint + step via lldb-dap. | @PLN66 ph.01 (rust-analyzer end-to-end); loft's `rustc`/`cargo` plumbing; a stock DAP client | The Rust trio + `codelldb`/`lldb-dap` DAP bridging |
| **WB6 · C** | Open `.c`/`.h` → clangd intelligence; **Build/Run** via gcc; breakpoint + step via lldb-dap/gdb-DAP. | WB5's DAP bridge; @PLN66 Layer-C slot | The C trio + `gcc`/`clang` invocation |
| **WB7 · bridge intelligence + hardening** | @PLN66 acceptance metrics: cold-start ≤ 2 s, hover ≤ 50 ms P95, crash-recovery < 1 s; multi-tab share; async gateway; security review. | @PLN66 ph.02 wholesale | Warm pool / multiplex / cache / debounce / crash-recovery / tracing; per-connection async (seam 4) |
| **WB8 · workspace surface (Markdown + Git/GitHub)** | Click a `.md` → **rendered preview** beside the source; a **Source Control** panel shows changed files + diffs; stage/commit/push; open a PR. | viewer `markdown::render` + `/diff`,`/commit` routes + `refresh.sh` git bridge (§ 10) | Read→write git verbs; `gh` for PRs; a Markdown-preview pane |
| **WB9 · live-authoring plane (run the game, edit on the fly) + world editor** | **Game ▶** runs a full native game; edit a script + **Reload** hot-swaps it into the *running* game; a visual **world/scene editor** places entities that scripts drive. | M5e slice 6 (`launchGame`/`reload`); [`58-graphics`](../../lib_plans/58-graphics/README.md); [`65-scriptable-scenes`](../../lib_plans/65-scriptable-scenes/README.md) | The hot-swap loop needs **@C71/N9**; the world editor is @PLN65 (§ 9) |

WB0–WB3 already deliver a materially better **loft** IDE (shippable on its own). WB4 is the
pivot; WB5–WB6 the multi-language payoff; WB7 the "done right" bar @PLN66 sets. **WB8** (docs +
git) reuses the viewer almost wholesale and can land early — it depends on nothing above WB3.
**WB9** is the lavition payoff and the one phase with a *hard* external dependency (@C71/N9 for
true hot-swap; the world editor is its own plan). WB8/WB9 are ordered last only because WB0–WB7
establish the multi-language spine; WB8 in particular could be pulled forward.

## 9. The live-authoring plane — run the game while editing its scripts

Beyond the three *source-oriented* planes (§ 4) sits a fourth that operates on a **running
program**: the lavition loop. It is M5e slice 6 promoted to a first-class plane, and it drops
onto the same server with no new model — a live game is just the execution plane pointed at a
**long-lived native process** instead of a one-shot run.

- **Run a complete game.** `launchGame {file}` runs the program in its **own native OpenGL
  window** (the [`58-graphics`](../../lib_plans/58-graphics/README.md) /
  [`64-game-client`](../../lib_plans/64-game-client/README.md) runtime). *6a is landed* — the
  game runs as a real child process, its stdout streamed into the console; one game at a time;
  killable. The browser controls it (run/stop) and reads `gameState`; **it never renders the
  game's pixels** — the honest seam from M5e, unchanged.
- **Edit scripts on the fly.** With the game running, edit a function and **Reload**
  hot-swaps *just that function* into the live game over the **one shared store**
  (per-function interpret on the compiled baseline). This is the *edit → see it in the running
  game → breakpoint it* loop. **Dependency:** the real hot-swap (6b/6c) needs the **@C71/N9**
  execution model — a *design, not yet a build*; faking it via full-restart would ship the
  workaround as the feature, so it waits for that build. Until then WB9 delivers 6a (run +
  observe + restart-to-apply), and true in-place hot-swap lands when @C71/N9 does.
- **Debug the running game.** A breakpoint in frame logic pauses the game and drops the *same*
  variables/watch/stack panels (VE/SF/DB/RX) — debugging *is* "run with breakpoints", including
  in a live game.

**The world / scene editor — emergent, not pre-specified.** A visual editor that places
entities and edits a world/scene definition scripts then drive is on the roadmap, but it is
**not designed top-down here.** Following loft's dogfood loop (build a real consumer → harvest
the lessons → generalise into the tooling), the scene editor is **emerging from the crawler
consumer** — the `story` roguelike that is already the slice-6 game and the debugger's dogfood
driver. It is not there yet but is close; its real shape is being discovered by the crawler's
actual world-building needs rather than invented in the abstract. The Workbench's job is to
**reserve the seam** — a scene/world panel alongside the editor, driven by the same
one-message-one-method protocol — and **adopt the crawler's emergent design** when it lands.
[`65-scriptable-scenes`](../../lib_plans/65-scriptable-scenes/README.md) is its eventual
canonical home; the harvested editor generalises *into* that plan, it does not precede it.

## 10. Workspace surface — Markdown preview + Git/GitHub

Two **workspace-wide** surfaces (not per-language planes) that the viewer already provides
**read-only**; the Workbench reuses them and adds the **write** half — so this phase is almost
entirely assembly.

**Markdown — reuse the viewer's renderer.** The viewer already renders `.md` neatly via the
`markdown` registry library (`markdown::render`) inside the page chrome. Reuse it directly:
opening a `.md` file shows a **rendered preview** pane beside (or in place of) the source
editor. Rendering stays **server-side** (loft `markdown::render` → HTML streamed to the
browser), consistent with "the browser renders state, it never computes" — no JS Markdown
library, and it **dogfoods loft's own Markdown lib**. Bonus: the IDE becomes self-documenting —
the loft docs render in place.

**Git / GitHub — promote the viewer's git bridge from read to read-write.** The viewer is
already git-aware: `refresh.sh` snapshots git into `state/*.json` (branch, ahead/behind,
changed-files name-status, commits, per-file diffs) and the `/commit/<sha>` + `/diff/<path>`
routes render them. That is the **read** half of source control, already built. The Workbench
adds the **write** half as protocol verbs, and — since loft has no subprocess primitive (the
same reason the viewer shells to `refresh.sh` and @PLN66 uses a Rust sidecar) — the git/`gh`
operations run **native in the gateway**, each verb one native call, same invariant:

| verb | native call | surface |
|---|---|---|
| `gitStatus` / `gitDiff` | the existing snapshot | **Source Control** panel: changed files + inline diffs |
| `gitStage` / `gitUnstage` / `gitCommit` | `git add` / `reset` / `commit` | stage + commit from the panel |
| `gitPush` | `git push` | push the feature branch |
| `ghPr` / `ghChecks` | `gh pr create` / `gh pr checks` | open a PR, watch CI |

**Branch policy is honoured in the UI, not just the CLI.** The panel surfaces branch + PR
state and commits/pushes to a **feature** branch freely (a safety habit), but branch-creation,
PR-open, and merge stay **explicit user actions** — the IDE never auto-branches or auto-merges,
mirroring the repo's own [branch policy](../../../../CLAUDE.md). This makes the Workbench a
place to *review and land* a branch (the viewer's original purpose) as well as author it.

## 11. Boundaries / non-goals

- **Not the serverless playground** ([@PLN62](../../lib_plans/62-web-ide/README.md)) — the
  offline, zero-install sibling; shares only CM6 + the loft grammar. The Workbench never runs a
  toolchain in the browser.
- **No game/GUI pixels in the browser.** Native GUI/TUI programs (a loft game, a windowed or
  terminal Rust/C program) render in their own native surface; the browser sends control
  intents and reads status (M5e's honest seam, generalised).
- **Local-first, single-user.** Binds `localhost`; remote use is an explicit SSH port-forward
  (the `make view` pattern). The **build plane runs arbitrary `rustc`/`gcc`/binaries = arbitrary
  code execution** — acceptable locally, an explicit **non-goal** to expose as a hosted
  multi-tenant service without a real sandbox story.
- **Java (jdtls)** is in @PLN66 but outside your stated loft+Rust+C scope; it drops in later as
  a fourth trio (intelligence-only unless a Java debug/build adapter is added).
- **In-browser WASM `loft-lsp`** — a *future* variant for the offline sibling, after rewriting
  its stdio `main`; not this product.

## 12. Risks

| Risk | Mitigation |
|---|---|
| **Position translation** (LSP UTF-16 vs loft byte offsets vs CM6 columns) | @PLN66's known bug magnet; normalise in the bridge; pin with a translation test per language. |
| **Two protocol families on one socket** | Strict `kind` tagging + a per-`kind` router; cover with the `tests/serve.rs` drive-and-assert shape. |
| **serve.rs blocking single-thread** | Push LSP concurrency into the sidecar from WB1; defer the gateway async rewrite to WB7. |
| **Debug-adapter heterogeneity** (loft-dap is in-process; lldb-dap/codelldb are external DAP servers) | The gateway already frames DAP for `loft-dap`; reuse that framing to speak real DAP to the external adapters. |
| **Toolchain not installed** (rust-analyzer/clangd/lldb absent) | Per-language capability probe at startup; graceful degradation + a clear "install X" message (@PLN66's fallback pattern). |
| **Security of the build plane** | Local-only bind; workspace-sandboxed FS; bounded runs (`--max-steps`, `LOFT_TIMEOUT`); no hosted mode without a sandbox. |

## 13. Prerequisites (status)

- **`loft-lsp` / `loft-dap`** — **SHIPPED** (@PLN63, this session). The loft leg is *ready*;
  this **unblocks @PLN66 phase 03** (which was gated on LSP.1).
- **serve.rs slices 1–5** — **LANDED**. Slice 6 (game hot-swap) waits on @C71/N9 and is **not**
  on the Workbench's critical path.
- **`loft-lsp-bridge`** (@PLN66) — designed, unbuilt; WB1/WB5/WB6 build phases 00–02.
- **Viewer generalisation** ([@PLN70](../../lib_plans/70-viewer-generalisation/README.md), if
  present) — optional; nice for reusing the tree/shell, not blocking.
- **CM6 + community grammars** — bundled npm assets (no CDN; offline posture).

## 14. Cross-references

- [`IDE.md`](IDE.md) — @PLN16 M5e, the server-backed **loft** IDE this generalises; its
  protocol table, safety model, and slice history are the loft plane verbatim.
- [`PROTOCOL.md`](PROTOCOL.md) — the wire contract (`one message ⇄ one engine method`) every
  plane rides.
- [`66-viewer-lsp-bridge`](../../lib_plans/66-viewer-lsp-bridge/README.md) — the multi-LSP
  sidecar (Layers A/B/C, bridge intelligence) absorbed as the intelligence plane; phases
  00–02 are WB1/WB5/WB6/WB7's backbone.
- [`63-lsp`](../../lib_plans/63-lsp/README.md) + [`DAP.md`](../../lib_plans/63-lsp/DAP.md) /
  [`DAP_ADVANCED.md`](../../lib_plans/63-lsp/DAP_ADVANCED.md) — the shipped `loft-lsp`/`loft-dap`
  and the VE/SF/DB/RX advanced debug surface.
- [`62-web-ide`](../../lib_plans/62-web-ide/README.md) — the serverless sibling; the editor +
  grammar donor, *not* this product.
- [`58-graphics`](../../lib_plans/58-graphics/README.md),
  [`64-game-client`](../../lib_plans/64-game-client/README.md),
  [`65-scriptable-scenes`](../../lib_plans/65-scriptable-scenes/README.md) — the native game
  runtime (§ 9) and the eventual home of the crawler-emergent world editor.
- The `story` roguelike ("crawler") — the sibling dogfood consumer M5e already names as the
  slice-6 game and the debugger's driver; the scene editor is being harvested from its
  real world-building work (§ 9). *(Separate repo — referenced in prose, not linked.)*
- `src/serve.rs`, `src/rpc.rs` — the running gateway + protocol the Workbench extends.
- [`70-viewer-generalisation`](../../lib_plans/70-viewer-generalisation/README.md) — extracts a
  reusable viewer engine; optional donor for the Workbench shell/tree.
