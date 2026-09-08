<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Roadmap

Open work items grouped by value category, with explicit dependencies and effort estimates.

The methodology behind this file (categories, no-time-projections, features-need-plans) lives in [`plans/README.md` § Roadmap workflow](plans/README.md#roadmap-workflow).  This file holds only the work tables.

| Legend | Meaning |
|---|---|
| **S / R / G / F / U / C / Q / N** | Value category (silent-failure / regression / goal / foundation / UX / clean / quality / niche).  Default reading order — pick from the highest tier with open work. |
| **E** column | XS = Tiny · S = Small · M = Medium · MH = Med-High · H = High · VH = Very High · L = Large multi-arc |
| **Design** column | ✓ = detailed · ~ = partial · — = needs design |

| Companion file | Purpose |
|---|---|
| [`RELEASE.md`](RELEASE.md) | What MUST be true before tagging |
| [`PLANNING.md`](PLANNING.md) | Priority-ordered backlog (next-best pickup) |
| [`plans/README.md`](plans/README.md) | docs-vs-plans rule + plan workflow + roadmap workflow + value categories |

**Project goal:** browser games anyone can play via a shared link.  Native OpenGL is supported for desktop enthusiasts; server/multiplayer comes after the single-player browser experience works.

---

## Where we are & the highest-leverage next work (2026-07-10)

> **Update 2026-07-10 — the plan board is EMPTY; the remaining work was never ticketed.**
> Zero plans carry `status:active` / `status:next` / `status:progress`, and the loft repo has **zero
> open bug issues**.  That second number is real — loft runs **fix-don't-file** (CLAUDE.md § Bug-filing
> policy) under a feature freeze, so a known defect is never parked and nothing accumulates — but it is
> **not the defect ledger**.  The known remainder is a set of deliberate, scoped deferrals living in
> each open plan's residual list (e.g.
> [plans/25-nullable-sequences/RESUME.md](plans/25-nullable-sequences/RESUME.md)) and in the
> [STABILITY_ROADMAP](STABILITY_ROADMAP.md) queue.  Read those, not the issue count.  The empty *plan
> board* is real too: @PLN25/@PLN28/@PLN36/@PLN85/@PLN90/@PLN94/@PLN97–101 all closed and the residue
> was never ticketed.  The top picks now are:
>
> 1. **Cluster C / H10 — fold `copy_claims` onto the keystone** (executable plan:
>    [STABILITY_REDFLAG_REMEDIATION § Cluster C / H10](STABILITY_REDFLAG_REMEDIATION.md#cluster-c--h10--fold-copy_claims-source-enumeration-onto-the-keystone);
>    tracking row: [STABILITY_ROADMAP](STABILITY_ROADMAP.md)).
>    The **sole remaining item of the wide-release bar's gate 1** — the gate the roadmap calls *"the
>    definition of stabilized, not one item among five."*  Gate 1's other half, the fuzz-proof, now
>    stands: @PLN53 + @PLN54 both CLOSED 2026-07-10 (#547).  Fully designed, **S per copy helper**.
>    Deliberately a **work item, not a plan** (the design is settled; a plan issue would be a pointer).
> 2. **Gate 5 — the stability contract** (compat promise, expressible version bounds, deprecation
>    channel, public bug-intake path, the 1.0 line).  Its opening condition ("when gate 1 is in
>    sight") has **fired**, and the failure mode it prevents is already live: `hex_terrain 0.1.0`
>    silently computes a wrong answer against current loft.  Opened as
>    **[@PLN102](https://github.com/loft-lang/plans/issues/102)** (`status:next`).
> 3. **Un-mute the nightlies** (effort S).  `ci.yml` and `registry-validation.yml` have **no failure
>    notifier at all** — `ci.yml` carries the 3-OS matrix and the differential oracle (Goal D's whole
>    Check) and went red 6 of 10 scheduled nights with nobody told.  Reuse `miri.yml`'s `notify` job.
>    (The `graphics` runner one-liner this item also called for is done — `registry-validation.yml`
>    installs `libasound2-dev`.)  A gate nobody watches is not a gate — that is how
>    `registry-validation` was mistaken for a DNS flake for days.
>
> **`registry-validation` is green** (since 2026-07-26; all 34 package legs pass, 08-07 through
> 08-09 consecutively). Both faults that held it red are fixed — the workflow installs
> `libasound2-dev` for `graphics`, and `hex_terrain` no longer fails on plain-bind write-through
> vs C86 H-Copy. The **differential oracle is green too** — all 30 corpus programs agree across
> interpret / native / wasm at `4c80e706` (2026-08-09), with both positive controls passing, so
> the native/wasm accept-reject divergence on a match-arm tail call is no longer reproducing.
> It runs `--ignored`, outside the per-PR set, so it needs re-running to stay current.
>
> Lower-friction picks that remain valid: **`loft search` CLI** (effort S — the
> advertised-but-unimplemented registry discovery command; spec:
> [PKG_REGISTRY](PKG_REGISTRY.md) § Open work) and **lazy auto-use**
> ([lib_plans/59-lazy-stdlib](lib_plans/59-lazy-stdlib) — auto-load a registered library on first use
> of its trigger method; note the tracker row for it is marked closed/superseded, so re-premise before
> picking up).

> **Historical (2026-06-19) — FFI bridge migration done.**  The local plan
> [`lib_plans/74-ffi-dispatch`](lib_plans/74-ffi-dispatch) is **COMPLETE**: all 7 loft-lang native
> libraries migrated to `#[loft_native]` generated bridges + re-published to the registry (signed,
> merged); the legacy ~98-arm interpreter marshaller is deleted (bridge-only dispatch); the `#306`
> bridge ref-return bug is fixed.  Libraries are **discoverable in-repo** — auto-generated
> `LIBRARIES.md` + CLAUDE.md hooks + a CI staleness gate + a registry `validate.py` docs
> gate.  **Tracker mismatch:** the issue `@PLN74` ("[libs] FFI dispatch") is still OPEN /
> `status:future` — it is a migration stub pointing at `lib_plans/future/25-ffi-dispatch/`.  Either
> close `@PLN74` or stop calling the work complete; do not read the two numbers as the same plan.

The 2026-07 cycle shipped as `2026.7.1` (stability + type safety); the current release is
`2026.8.0`.  The **H-register is drained**
([STABILITY_ROADMAP.md](STABILITY_ROADMAP.md) — H3/H5/H6/H7/H8 all done) and the **GitHub issue
tracker holds 4 open bugs** (2026-08-09) — under fix-don't-file a low count means "few
deferrals", not "no bugs"; the ledger is in the docs, see the digest above.  Three of the four
came from the crawler consumer, which is where most bug flow now originates.  The stability roadmap itself is **not** drained either: gate 1's
Cluster C fold remains, and gate 5 is now [@PLN102](https://github.com/loft-lang/plans/issues/102).
Under the warm feature freeze
(below), **in-scope** = library enablement + optimisations + stabilisation;
**gated** = new language features.  Current top picks, by theme — this is a pointer
digest; the detail lives in the linked homes (no catalogue is duplicated here).

**✅ Much of each theme's FOUNDATION already ships** — the items below are
increments on top, not greenfield.  Already working today (verified 2026-06-17
against the CHANGELOG + `lib/` + the test suite): the **engine host** run modes
(`run` / `run_local` windowed-no-server / `run_client` / `post` / `stop`); the
**REPL** (`loft repl`) and **`loft introspect`**; the **program cache** (on by
default — the headline startup win); the **library system** with toolchain-free
installs and ~12 working libs (`graphics`, `imaging`, `gridmesh`, `shapes`,
`input`, `server`, `web`, `html`, `markdown`, `game_protocol`, `world`, `time`);
**const-store Phase A** (heap-backed `CONST_STORE`); **native-package C-ABI
linking** (`@PLN26`); plus the branch-review viewer / tracker index / markdown
renderer.  So *windowed graphics + a multiplayer protocol + a REPL* are present
now — what's open below is the increments.

| Theme | Open increment (the part NOT yet shipped) | Scope | Home |
|---|---|---|---|
| **Performance / startup** (serves live-prototyping) | precompiled-stdlib fast-start (`@PLN52` — **DELIVERED** via @PLN11 arc D/D2b, opt-in `LOFT_STDLIB_CACHE`); const-store Phase B/C (`@PLN82`); the wasm-vs-native gap | in-scope | [PERFORMANCE.md § Open work](PERFORMANCE.md); `@PLN82` |
| **Native robustness** | ~~shared-store dispatch → a C-ABI `LoftStore` handle~~ — **gh #389 CLOSED**.  Live item: the **differential-oracle divergence** on `main` (native/wasm reject a match-arm tail call the interpreter accepts) | in-scope (stabilisation) | [NATIVE.md § Open work](NATIVE.md) |
| **Library system** (the dogfood track) | LSP, a game-client lib, viewer generalisation, regex Phase 1 (pure-loft NFA) — *graphics / imaging / server / markdown / world / game_protocol / **regex** (v0.2.0: matches/find/split **+ match_groups/replace**) already ship*.  Also: migrate `hex_terrain` off the plain-bind write-through idiom (see gate 5) | in-scope | [lib_plans/README](lib_plans/README.md); the `[libs]` `@PLN` issues |
| **Friend-readiness / UX** | first-time tutorial + more day-to-day ergonomics — *REPL / `introspect` / the IDE editor slices already ship* | mixed | ROADMAP § U + § "Near-term focus" below |
| **Games / engine** (the north star) | the UDP state-sync channel (05a) — a **deferred sub-item of the now-CLOSED `@PLN18`**, at [`plans/18-engine-host/05a-udp.md`](plans/18-engine-host/05a-udp.md) — scriptable scenes, fuller browser game UI — *the run modes + graphics rendering + the multiplayer protocol already ship* | parallel-agent lane + partly gated | ROADMAP § G; `plans/18-engine-host/` |
| **Coroutines** (native iterator *sources* — Rust fns yielding into `iterator<T>`) | the native iterator source (P327) | gated (language feature) | [COROUTINE.md](COROUTINE.md); PLANNING § CO1 |
| **Stability instruments** | program-level fuzzing (`@PLN53`) — **✅ harness shipped + CLOSED**; sanitizer-coverage expansion (`@PLN54`) — **✅ stack shipped + CLOSED** (only S9 cdylib-boundary ASan spun out, toolchain-blocked) | ✅ both closed | STABILITY_ROADMAP step 9; plans 53/54 |
| **CI / tooling** | docs-only-PR matrix-skip fix (STABILITY_ROADMAP row 11 — risky; validate via a docs-only test PR) | in-scope | STABILITY_ROADMAP row 11 |

The default reading order stays the value-category tables below (S → R → G → F → U
→ C → Q → N); this digest just surfaces the current top pick across them, and
[PLANNING.md](PLANNING.md) remains the priority-ordered next-best-pickup backlog.

---

## Feature freeze — heading into the 2026-07 cycle (added 2026-06-07)

Loft is entering a **warm feature freeze** to stabilise toward a release we can trust.  Scope for this cycle is deliberately narrow:

- The **REPL** (interactive read-eval-print loop) is the **last new language feature** before the freeze — built on the `repl` branch.
- After the REPL, the **only** new-feature work allowed is **making libraries work on loft** (the library system; see [`lib_plans/README.md`](lib_plans/README.md)).
- **Optimizations are allowed** — libraries depend on performance, so speed/footprint work stays in scope.
- **Everything else is fixing existing language features** — stabilisation and bug-fixing, lowering the bug count.
- **New features resume only once we have confidence that all language features work.**  Until then, nothing new beyond the REPL and library-enablement.

This is the sequencing view; the per-cycle ship gate and the monthly branch model live in [`RELEASE.md § Release cadence`](RELEASE.md#release-cadence).

## Near-term focus — friend-readiness (added 2026-05-13)

Loft has crossed the threshold where a developer friend with
some Rust / ML experience can be invited to try it.  The
following items are flagged as **gates for "tier-2
friend-readiness"** — a friend who'd follow a tutorial and
write something small.

The user explicitly named this as a near-term goal: "I can
start to really advise friends to try it out."  Items below
are the residual gaps; see the per-item rows in the value
categories below for full detail.

| Pri | Gate | Status | Source |
|---|---|---|---|
| — | **@PLN28 — better error messages** — ✅ **DONE 2026-07-07** (all phases delivered: `file:line:col` + caret across parser/type/runtime, did-you-mean suggestions, concrete type-mismatch + match-pattern check, closeout doc). Friends' first WTF moment is a type error; tighter messages compound into trust. | Complete; phase-4 has 2 non-blocking polish slices deferred (finer format-null tokens, `= note:` renderer) | [`plans/28-error-messages/`](plans/28-error-messages) |
| 2 | **@PLN42 — tracker-tag indexer** — `@P-id` / `@PLAN-id` convention + scanner + CLI + viewer integration.  Replaces grep-based tag lookups with O(1) JSON queries.  Shrinks Claude per-task token usage; feeds @PLN50 (eagleviewer) newcomer landing's status buckets. | ⏸ **PARKED** — core shipped + in daily use (phases 0-6 + 9); tail deferred/gated (06 closeout waits on @PLN50; 07a WebSocket gated on `lib/fs_watch`; 08 multi-project open, appetite) | [`plans/42-tracker-index/`](plans/42-tracker-index) |
| 3 | ~~**DX.2 — CI: package + native tests**~~ — **✅ DONE** (verified 2026-07-07): the per-PR `ci.yml` nextest run already covers native (`binary(native)`) + package (`binary(wrap)::library_suite`) tests, plus the ASan gate + nightly registry-validation. @PLN36 CLOSED (`status:finished`). | Done | [`plans/36-developer-experience/`](plans/36-developer-experience) |

What ships ALREADY for friend-readiness (don't reblock on these):

- ✅ TextMate grammar (`syntaxes/loft.tmLanguage.json`)
- ✅ VS Code extension (`editors/vscode/`) + IntelliJ plugin
- ✅ Examples directory (7 standalone programs at `examples/`)
- ✅ "Learn loft in 30 minutes" tutorial (`doc/learn-loft.md`)
- ✅ Closures (@PLAN22 trio @P259/P260/P261, shipped 2026-05-13)
- ✅ Native backend CI-gated production
- ✅ Coroutines (0.8.3)
- ✅ Server lib + WebSocket (lib/server)

What's deliberately deferred past the tier-2 gate (don't
block here):

- LSP server (LSP.1+) — months of work; tier-3 enabler.
- Outbound HTTP client — in `lib_plans/06-web-services/`;
  tier-3 (any friend writing a real network app).
- Package registry (PKG.REG) — in `PACKAGES.md § Open work`;
  tier-3 (real package consumption).
- Plan-23 event loop — needed for games (tier 3).

**Total tier-2 sprint estimate:** H (items 1+2+3 combined).
Item 4 (tables) adds MH on top but elevates polish.

---

## S — Silent failure / data-loss prevention

Features that "appear to work" but don't, or that lose data without indication.  HIGHEST priority because invisible to users.  See [plans/README.md § Value categories](plans/README.md#value-categories--what-kind-of-value-not-just-how-much) for why S sits above R.

> **Reconciled 2026-07-10.** Four of the five rows below were CLOSED plans still being presented as
> highest-priority open work.  Only the Q1 row is genuinely open.

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| ~~Q*~~ | **JSON parse-error diagnostics (Q1) — CLOSED 2026-08-20.**  Both halves of the old row are now false, re-measured on both backends: the auto-wrap `Struct.parse(text)` path DOES report (`line 1:33 path:addr.zip`), and a successful parse leaves `json_errors()` empty rather than carrying the previous call's error.  The two spellings differ in which half of the answer they give — one-stage names the position, `Struct.parse(json_parse(text))` names the types (`Addr.zip: expected JNumber, got JString`).  Pinned by the JSON chapter's own cells. | S-M | ✓ | QUALITY.md#open-work--actionable-summary |
| ~~(cross)~~ | ~~Match validation~~ — **✅ CLOSED** (@PLN29 `status:finished`) | M | ✓ | plans/29-match-validation/README.md |
| ~~(cross)~~ | ~~Struct-enum validation~~ — **✅ CLOSED 2026-07-09** (@PLN30, delivered) | M | ✓ | plans/30-struct-enum-validation/README.md |
| ~~(cross)~~ | ~~Keyed collection validation~~ — **✅ CLOSED 2026-07-09** (@PLN31, superseded) | M | ✓ | plans/31-collection-validation/README.md |
| ~~(cross)~~ | ~~Integer width discipline~~ — **✅ CLOSED** (@PLN1 `status:finished`; `integer` is i64 end-to-end) | M | ✓ | plans/1-integer-width-discipline/README.md |

---

## R — Regression / release-blocker

Known broken behavior that gates the next release.  Currently empty — bug-fix workflow lands these as P-issues directly (PROBLEMS.md + test + commit), not as plans.  Items would surface here when a regression accumulates beyond a single fix.

*(no entries today)*

---

## G — Goal-enabling

Directly enables loft's core use case: browser games anyone can play via shared link, multiplayer, native-game debugging.

### Native game debugging

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| NDB.0 | `--native-debug` flag — DWARF in `--native` builds | XS | ✓ | plans/34-native-debug/README.md |
| NDB.1 | `.loft.map` source map + `loft-gdb.py` / `loft-lldb.py` plugins | M | ✓ | plans/34-native-debug/README.md |
| NDB.2 | DWARF rewrite — point `.debug_line` / `.debug_info` directly at `.loft` | MH | ✓ | plans/34-native-debug/README.md |

### Multiplayer + protocol stack

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Event-loop abstraction (client + server protocol) | MH | ✓ | plans/32-event-loop/README.md |
| (cross) | Protocol-validation vehicle (TIC_TAC_TOE — v1/v2/v3/v5 shipped, v3.5/v4/v6 gated on @PLN32 YIELD.2) | M | ✓ | plans/39-tic-tac-toe/README.md |
| (cross) | First real-game milestone — multi-client hex editor | M | ✓ | plans/33-multiplayer-editor/README.md |
| SRV.1 | Plain HTTP routing + middleware | M | ✓ | lib_plans/future/08-server/README.md |
| SRV.2 | HTTPS with static PEM certificates | S | ✓ | lib_plans/future/08-server/README.md |
| SRV.3 | WebSocket support | S | ✓ | lib_plans/future/08-server/README.md |
| SRV.4 | Authentication: JWT, session, API key | M | ✓ | lib_plans/future/08-server/README.md |
| SRV.5 | ACME / Let's Encrypt automatic certs | M | ✓ | lib_plans/future/08-server/README.md |
| SRV.6 | CORS, rate limiting, static files | M | ✓ | lib_plans/future/08-server/README.md |
| SRV.G | Game loop: ws_poll, broadcast, ConnectionRegistry | M | ✓ | lib_plans/future/08-server/README.md |
| GC.1 | WebSocket client + GameEnvelope protocol | M | ✓ | lib_plans/64-game-client/README.md |
| GC.2 | Lobby + matchmaking | S | ✓ | lib_plans/64-game-client/README.md |
| GC.3 | Fixed-timestep game loop | S | ✓ | lib_plans/64-game-client/README.md |
| GC.4 | Client-side prediction + reconciliation | M | ✓ | lib_plans/64-game-client/README.md |
| GC.5 | WASM script loading + Ed25519 verification | M | ✓ | lib_plans/64-game-client/README.md |
| GC.6 | Shared game logic + Tic-Tac-Toe demo | M | ✓ | lib_plans/64-game-client/README.md |

### Browser game UI + scriptable scenes

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| W2 | Editor shell (CodeMirror 6 + Loft grammar) | M | ✓ | lib_plans/62-web-ide/README.md |
| W3 | Symbol navigation (go-to-def, find-usages) | M | ✓ | lib_plans/62-web-ide/README.md |
| W4 | Multi-file projects (IndexedDB) | M | ✓ | lib_plans/62-web-ide/README.md |
| W5 | Docs & examples browser | M | ✓ | lib_plans/62-web-ide/README.md |
| W6 | Export/import ZIP + PWA offline | M | ✓ | lib_plans/62-web-ide/README.md |
| SC.1 | Scene script API — hooks for hex enter/exit/interact | M | ✓ | lib_plans/65-scriptable-scenes/README.md |
| SC.2 | IDE panel in scene editor | M | ✓ | lib_plans/65-scriptable-scenes/README.md |
| SC.3 | In-browser compile + hot-reload | M | ✓ | lib_plans/65-scriptable-scenes/README.md |
| SC.4 | Script sandbox — limited API | S | ✓ | lib_plans/65-scriptable-scenes/README.md |
| SC.5 | Built-in script templates | S | ✓ | lib_plans/65-scriptable-scenes/README.md |
| SC.6 | Script sharing via scene JSON | S | ✓ | lib_plans/65-scriptable-scenes/README.md |

### Game rendering

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Graphics library bundle (2D canvas + GLB + OpenGL + WebGL) — low-level `gl_*` shipped; high-level renderer designed | H | ✓ | lib_plans/58-graphics/README.md |

---

## F — Foundation

Unblocks 2+ downstream plans.  Lattice points in the dependency graph.

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| PKG.REG | Central package registry MVP — `loft install <name>` | M | ✓ | PACKAGES.md § Open work |
| PKG.7 | Lock file (`loft.lock`) for reproducible builds | S | ✓ | PACKAGES.md § Open work |
| PKG.EXTRACT | Move `lib/*/` out into per-family GitHub repos | L | ✓ | lib_plans/12-library-extraction/README.md |
| FFI.1 | Generic type marshaller from `#native` signature | MH | ✓ | lib_plans/61-game-infra/README.md |
| FFI.2 | Generic cdylib loader — scan exports, HashMap | S | ✓ | lib_plans/61-game-infra/README.md |
| FFI.3 | Eliminate per-function glue in native.rs | M | ✓ | lib_plans/61-game-infra/README.md |
| FFI.4 | Docs: zero-boilerplate native function guide | S | ✓ | lib_plans/61-game-infra/README.md |
| LSP.1 | `loft-lsp` MVP — diagnostics + outline + hover | M | ✓ | lib_plans/63-lsp/README.md |
| LSP-CLIENT | `loft-lsp-bridge` sidecar + viewer code intelligence — rust-analyzer / loft-lsp / jdtls | L | ✓ | lib_plans/66-viewer-lsp-bridge/README.md |
| (cross) | Lazy stdlib loading — trigger-based pay-for-what-you-use | M | ✓ | lib_plans/59-lazy-stdlib/README.md |
| **REGEX.0** | regex MVP — `#native` cdylib bridge to Rust `regex` crate.  **SHIPPED as `regex` v0.2.0** (loft-libs-core/regex; matches/find/split **+ `match_groups` + `replace`**).  Next: Phase 1+ (pure-loft NFA) | S | ✓ | lib_plans/57-regex/README.md |
| **TIME.1** | `DateTime` value type (i64 epoch-ms, JS-`Date`-aligned) + built-in `{dt:…}` formatting + pure-loft `lib/time` operations — unblocks the `training` app's date-indexed B8–B10 routines; broadly useful Data/ETL gap | H | ~ | lib_plans/21-datetime/README.md |
| **GFX.PORTABLE** | Make the `Renderer`/`Scene` layer the complete backend-portable rendering contract (portable shaders, scene-level custom materials + render-target/post-process passes; no script reaches raw `gl_*`) — prerequisite for a native GPU backend (wgpu → Vulkan/Metal) and thus native Android/iOS | H | ~ | lib_plans/72-renderer-backend-boundary/README.md |

---

## U — Ease of use

First-time-user experience, daily ergonomics, IDE polish.

### First-time experience + tutorial

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Better error messages — `file:line:col` + caret + suggestion | M | ✓ | plans/28-error-messages/README.md |
| SH.1 | TextMate grammar for `.loft` | S | ✓ | plans/36-developer-experience/README.md |
| SH.2 | VS Code extension (grammar + snippets + run task) | S | ✓ | plans/36-developer-experience/README.md |
| DX.1 | Quick-start `examples/` directory at repo root | XS | ✓ | plans/36-developer-experience/README.md |
| DX.3 | "Learn loft in 30 minutes" walkthrough page | S | ✓ | plans/36-developer-experience/README.md |
| DX.2 | CI: add package tests + native tests to workflow | XS | ✓ | plans/36-developer-experience/README.md |

### Day-to-day ergonomics

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| P2 | REPL / interactive mode | M | ✓ | @PLN12 — plans/12-repl-and-introspection/README.md (**FINISHED** 2026-06-08; store-resident successor → @PLN14) |
| W-warn | Developer warnings (Clippy-inspired) | M | ✓ | lib_plans/61-game-infra/README.md |
| W-qual | Warning quality — stop nagging users about safe code (short-circuit guard recognition, `#null_safe` annotation, entry-guard inference, ASCII-peephole) | MH | ✅ CLOSED | plans/46-warning-quality/README.md |
| L1 | Error recovery after token failures | M | ✓ | (needs plan promotion) |
| (cross) | Branch-aware doc + code review viewer (loft binary) | M | ✓ | plans/35-branch-review-viewer/README.md |

### IDE editing surface

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| LSP.2 | `loft-lsp` editing — completion, def, refs, rename, semantic tokens, code actions | MH | ✓ | lib_plans/63-lsp/README.md |
| LSP.3 | `loft-dap` MVP — DAP server for interpreter-mode debug | MH | ✓ | lib_plans/63-lsp/README.md |
| IDE.ECLIPSE | Eclipse plugin via LSP4E (LSP.1 features) | S | ✓ | lib_plans/63-lsp/README.md |
| IDE.JETBRAINS | JetBrains plugin via LSP4IJ (LSP.1 features) | S | ✓ | lib_plans/63-lsp/README.md |
| IDE.NEOVIM | Neovim docs + `nvim-lspconfig` snippet | XS | ✓ | lib_plans/63-lsp/README.md |

---

## C — Clean features

Language correctness, removes special cases.  (Validation matrices that catch silent-failure variants live in S above; this section holds clean-feature work that doesn't primarily prevent silent failure.)

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| P54 | First-class `JsonValue` enum; old text-based JSON gone | MH | ✓ | QUALITY.md#open-work--actionable-summary |
| (cross) | L3 PEG-style match patterns (sequence / alternation / capture) | MH | ✓ | plans/35-match-peg/README.md |
| A8 | Slicing, open-ended ranges, partial-key match on sorted/index | M | ✓ | plans/38-sorted-slice/README.md |
| C52 | Stdlib name clash: warning + `std::` prefix | M | ✓ | (needs plan promotion) |
| C53 | Match arms: library enums + bare variant names | M | ✓ | (needs plan promotion) |
| (cross) | `const` struct fields (write-once-at-construction) — closes INCONSISTENCIES.md § 33 | M | ✓ | plans/40-const-fields/README.md |
| I12 | Interfaces: factory methods (`fn zero() -> Self`) | S | ✓ | (needs plan promotion) |
| (cross) | Standalone regex library Phase 1+ (pure-loft NFA + backtracking fallback engine — replaces the cdylib bridge transparently) | MH | ✓ | lib_plans/57-regex/README.md |

---

## Q — Internal quality

Performance, refactor, internal cleanup with clear payoff.

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Native codegen follow-ups (yield-from + generic text-return + fill.rs auto-gen) | XS-M per item | ✓ | NATIVE.md § Open work |
| (cross) | Performance follow-ups (P1-P3 interpreter / N1-N3 native / W1 wasm) | S-MH per item | ✓ | PERFORMANCE.md § Open work |
| O4 | Native: direct-emit local collections | M | ✓ | PERFORMANCE.md § Open work (N1) |
| O5 | Native: omit `stores` from pure functions | M | ✓ | PERFORMANCE.md § Open work (N2) |
| A12 | Lazy work-variable initialization | M | ✓ | PLANNING.md (no PERFORMANCE.md design yet) |
| O2 | Stack raw pointer cache | M | ✓ | PLANNING.md (no PERFORMANCE.md design yet) |
| @P393 | Vector store-lifetime watermark — function-local vectors free at scope-end not last-use; literal-init double-allocates.  Stage A: verified **no leak** (exit gate passes); benign watermark + noisy `LOFT_STORES=warn` floor.  Quickest win = raise heuristic threshold (XS) | S-M (XS heuristic / S cluster II / M cluster I) | Stage A ✓; Stage B/C pending (design call) | plans/2-vector-store-watermark/README.md |

### Constant store deferred-tail

The CS.B / CS.C1-C3 work items are deferred (Phase A + D shipped; Phase
B + C remain).  Trigger conditions and design content live in
[`plans/DEFERRED.md`](plans/DEFERRED.md) and the const-store plan in
`plans/deferred/`.  ROADMAP carries them only when the trigger fires.

---

## N — Niche / opportunistic

Small specific features, low-priority items.

| ID | Title | E | Design | Source |
|---|---|---|---|---|
| (cross) | Game asset pipeline (prototype → artist polish → integration) | M | ✓ | lib_plans/60-asset-pipeline/README.md |
| (cross) | Web services — HTTP client + URL handling + auth + SSE/WS | M-H per arc | ✓ | lib_plans/06-web-services/README.md |
| C57 | Route decorator syntax (`@get`, `@post`, `@ws`) | H | ✓ | plans/37-server-features/README.md |
| I13 | Iterator protocol (`for msg in ws` via `fn next`) | MH | ✓ | plans/37-server-features/README.md |
| AOT | Auto-compile libraries to native shared libs | M | ✓ | (needs plan promotion) |
| A4 | Spatial index operations | M | ✓ | (needs plan promotion) |

### Deliverables (no plan needed — single-action items)

| ID | Title | E | Notes |
|---|---|---|---|
| G3 | Tilemap rendering (grid-based 2D, batched draw) — generic `lib/tilemap` | M | Brick Buster has level_brick dispatcher as own tilemap; generic library still open |
| G7.P | 🌐 Brick Buster on itch.io | S | Optional demo deploy; `--html` works, GH Pages already serves |
| MP.P | 🌐 Moros multiplayer — DM + players share live scene | S | Hosted-server demo |

---

## Stability gate (stability contract)

Every item below must be checked off before the project claims its stability bar — no "appears fixed" exceptions.  Cross-references the safety gate in [`RELEASE.md`](RELEASE.md).

- [ ] **PROBLEMS.md** has zero open `**High**` severity entries
- [ ] All `⚠️ Appears fixed but unverified` flags definitively closed via real-world testing (not just regression guards)
- [ ] **valgrind clean** on a debug build of `tests/scripts/50-tuples.loft` and the full brick-buster game (`25-brick-buster.loft`) for 5+ minutes of play
- [ ] `make ci` green on Linux, macOS Intel, macOS ARM, Windows
- [ ] All `~~Fixed~~` PROBLEMS.md entries removed (history lives in CHANGELOG.md)
- [ ] `doc/claude/INCONSISTENCIES.md` reviewed: each entry resolved or explicitly accepted in LOFT.md / CHANGELOG.md
- [ ] Pre-built binaries on the GitHub release for all four platforms
- [ ] HTML reference and PDF up to date and linked from the release page
- [x] **[@PLAN53 sanitizer-CI-lever](plans/finished/53-sanitizer-ci-lever/README.md)** — Miri / ASan / guard CI stack live on `main`; 5 UB clusters fixed; Wave-2 coverage continuing in [@PLN54](plans/54-sanitizer-coverage-expansion/README.md) (CLOSED 2026-05-31)

---

## Deferred indefinitely

| ID | Title | Notes |
|---|---|---|
| O1 | Superinstruction peephole rewriting | Opcode table full (254/256) |

---

## Demo applications — independent lifecycles

Demo apps ship on their own cadence and do **not** gate any language work.  Per [`RELEASE.md` § Explicitly out of scope here](RELEASE.md#explicitly-out-of-scope-here).  If a demo surfaces a language-side bug, the fix lands under the relevant plan — but the demo's own scope never blocks anything.

| Demo | State |
|---|---|
| **Brick Buster** | Shipped 2026-04-25 ([brick-buster.html](https://loft-lang.org/loft/brick-buster.html)).  itch.io publication optional. |
| **Moros editor — native** | Shipped 2026-04-22 (plans/finished/03-native-moros-editor/).  `make editor-dist` builds a self-contained `dist/moros-editor/`. |
| **Moros editor — web** | Designed, not built.  ~20 sprints (MO.1–MO.13).  Lives in `../moros/doc/claude/` + PLANNING.md MO.* once PKG.EXTRACT lets the libraries iterate independently. |
| **Web IDE** (W2–W6) | G category above (Browser game UI section). |
| **Server / game-client / scene scripting libraries** | G category above (Multiplayer + protocol stack and Browser game UI sections). |

---

## All open plans — index by category

Comprehensive list of every open plan across `plans/` and `lib_plans/`, tagged by primary value category.

For per-phase status (what's shipped, what's in flight, what's blocked) **read the plan README directly** — it's the source of truth.  This index gives you the plan name, remaining effort, dependencies, and a one-line "what is this plan about" descriptor.

### S — Silent failure / data-loss prevention

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/29-match-validation/`](plans/29-match-validation) | M | **✅ CLOSED** (@PLN29 `status:finished`) | Subject type × pattern shape matrix |
| [`plans/30-struct-enum-validation/`](plans/30-struct-enum-validation) | M | **✅ CLOSED 2026-07-09 (delivered)** | Variant payload × dispatch context matrix — feature shipped + ~135 tests; matrix demoted to docs per the plan's own gate |
| [`plans/31-collection-validation/`](plans/31-collection-validation) | M | **✅ CLOSED 2026-07-09 (superseded)** | Motivating panic gone; hash/sorted/index validated cross-mode; spatial folds into @PLN48 |
| [`plans/47-binary-io-validation/`](plans/47-binary-io-validation) | M | **✅ CLOSED 2026-07-09 (delivered)** | Value type × format × access-pattern matrix; absorbs @P289 — scalar/struct/char/bool/narrow-int round-trip shipped; variable-width-field compile-time rejection; 32-cell `tests/binary_io_matrix.rs` harness |
| [`plans/53-program-level-fuzzing/`](plans/53-program-level-fuzzing) | H | **✅ CLOSED 2026-07-10 (harness shipped)** | Harness delivered + merged (#542): F1 raw-source fuzzer + oracle, F2 keyed-container generator, F3 interp≡native differential, F4 arena-poison cleared — all gated by in-process `cargo test`, both backends.  Continuous at-scale runs (F1.4/F2.5) + OSS-Fuzz onboarding (F5) closed-by-decision (appetite-gated, no concrete trigger); `F5-DESIGN.md` kept for a future re-open |
| [`plans/54-sanitizer-coverage-expansion/`](plans/54-sanitizer-coverage-expansion) | M | **✅ CLOSED 2026-07-10 (sanitizer stack shipped)** | S1 macOS-ARM leg · S2 TSan · S3 `LOFT_POISON` · S5 Miri store-unit + debug-asserts · S6 native-ASan · S7 notifier all green on `main`; S4 LSan `detect_leaks=1` unblocked + green (@PLN85 fixed its leak); S8 MSan deferred (one-line reason).  Only S9 (mixed-boundary C71 cdylib ASan) open — toolchain-blocked (curve25519 `E0463`), spun out to @PLN11 N5 / its own plan |

### G — Goal-enabling

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/34-native-debug/`](plans/34-native-debug) | XS-MH | — | NDB.0 / NDB.1 / NDB.2 — GDB / LLDB integration for `--native` |
| [`plans/32-event-loop/`](plans/32-event-loop) | MH | **@P213 v4** (compiler bug) | Bidirectional event-loop abstraction (client + server) |
| [`plans/33-multiplayer-editor/`](plans/33-multiplayer-editor) | M | **plans/32 TIC_TAC_TOE v2 ground layer** (now active) | First real-game milestone |
| [`plans/39-tic-tac-toe/`](plans/39-tic-tac-toe) | M | **@PLN32 YIELD.2** + **@PLAN22 phase 2** | Protocol-validation vehicle.  v1/v2/v3/v5 shipped; v3.5/v4/v6 parked (2026-05-11) waiting on infra |
| [`lib_plans/58-graphics/`](lib_plans/58-graphics) | H (multi-arc) | — | Low-level GL + renderer abstraction |
| [`lib_plans/62-web-ide/`](lib_plans/62-web-ide) | M per W item | **lib_plans/63-lsp LSP.1** + **PACKAGES.md § Open work R1 workspace split** | Browser IDE (W2-W6) |
| [`lib_plans/future/08-server/`](lib_plans/future/08-server) | M-MH per SRV | — | HTTP / WS / static-file server library |
| [`lib_plans/64-game-client/`](lib_plans/64-game-client) | M | **plans/future/23 EVENT_LOOP** + cooperates with 08-server / 32-tic-tac-toe | `game_client` library design |
| [`lib_plans/65-scriptable-scenes/`](lib_plans/65-scriptable-scenes) | M-S per SC | **lib_plans/62-web-ide W2** + moros editor MO.* + script-target build mode | User-authored scene scripts (SC.1-SC.6 + SC.P) |
| [`plans/6-audience-generative-art/`](plans/6-audience-generative-art) | M | — | Audience-driven plant/crystal growth demo via shared URL (SHIPPED) |
| [`plans/51-bumper-airplanes/`](plans/51-bumper-airplanes) | M | reuses **plans/6-audience-generative-art** substrate + dryopea editor output | Successor audience demo — twin-strip-controlled airplane/bumper-car hybrids fly a static extruded-hex world; bounce physics, smoke-pot trails, off-axis-only player scoring (anti-coordination) |

### F — Foundation

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`lib_plans/61-game-infra/`](lib_plans/61-game-infra) | M-MH per item | **✅ CLOSED 2026-08-26 (6 of 7 G-rows shipped)** | FFI.1-4 — third-party native extensions.  The 2D-game rows landed in the libraries rather than here: G1/G2 `graphics::create_sprite_sheet` + `draw_sprite`, G4 `shapes` overlap tests, G5/G6 `graphics` audio + `sfx_*`, G7 `doc/brick-buster.html`.  G3 tilemap is the only remainder and has its own row below |
| [`lib_plans/63-lsp/`](lib_plans/63-lsp) | M (LSP.1) / MH (LSP.2/3) | — | LSP.1 unblocks 4 IDE plugins + browser IDE |
| [PACKAGES.md § Open work](PACKAGES.md#open-work) | S-M | — | PKG.7 + PKG.REG (format itself already shipped) |
| [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction) | L | **PACKAGES.md § Open work PKG.REG** | Multi-release execution arc |
| [`lib_plans/59-lazy-stdlib/`](lib_plans/59-lazy-stdlib) | M | **✅ CLOSED 2026-07-09 (superseded)** | Re-premised to `use`-loaded `lib/*` (crypto precedent); trigger-registry not built |
| [`lib_plans/57-regex/`](lib_plans/57-regex) | MH (Phase 1+) | — | **Phase 0 SHIPPED** (`regex` **v0.2.0** at loft-libs-core/regex; matches/find/split **+ match_groups/replace**).  Next: Phase 1+ (pure-loft NFA + backtracking fallback).  Unblocked @PLN42 phase 07 scan.loft + check_doc_drift.sh ports |
| [`plans/43-loft-store-durable/`](plans/43-loft-store-durable) | M | cooperates with **plans/42-tracker-index/07** + **plans/39-tic-tac-toe** + **plans/6-audience-generative-art** | Three-tier opt-in durability for loft mmap stores: IntegrityOnly (indexer), SnapshotEvery (TTT v5 sessions), WAL (audience demo).  Index is cheap test bed; game servers are critical consumers |
| [`lib_plans/67-process/`](lib_plans/67-process) | M | — | `lib/process/` subprocess primitive — closes the indexer / viewer bash-wrapper dependency (dogfood-driven by @PLN42 + @PLAN35) |
| [`lib_plans/68-fs-watch/`](lib_plans/68-fs-watch) | M | — | `lib/fs_watch/` file-event watcher — prerequisite for @PLN42 phase 07a WebSocket-push daemon (inotify on Linux, kqueue on macOS, ReadDirectoryChangesW on Windows) |
| [`lib_plans/69-cache/`](lib_plans/69-cache) | S | — | `lib/cache/` mtime-invalidated read-through cache — viewer hot-path optimisation (re-reads `index/tags.json` per request today) |

### U — Ease of use

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/28-error-messages/`](plans/28-error-messages) | M | — | `file:line:col` + caret + suggestions across parser / type / runtime / native |
| [`plans/finished/35-branch-review-viewer/`](plans/finished/35-branch-review-viewer) | M | (closed 2026-05-14) | Frozen loft binary serves branch-aware doc + code review dashboard via SSH-forwarded HTTP.  Shipped: dashboard / file render (markdown via new `lib/markdown` lib + line-numbered code) / diff + commit views with hunk colouring / `[Rendered ¦ Diff vs main]` toggle / `/tag/<bare>` tracker-tag references / `@P-id` autolinks in body text / image refs through `/raw/`.  Drove the seven-bug native arc @P262→@P269 closure as collateral. |
| [`plans/42-tracker-index/`](plans/42-tracker-index) | S | — | `@P-id` / `@PLAN-id` tag convention + scanner + CLI + viewer integration.  Tier-1 lookup tool for both Claude and humans |
| [`plans/36-developer-experience/`](plans/36-developer-experience) | XS-S per item | — | SH.* / DX.* / NT.* — DX grab-bag (some shipped) |
| [`plans/44-viewer-discoverability/`](plans/44-viewer-discoverability) | XS per item | — | Three XS viewer cleanups: site header, page_landing sections, route-graph drift sentry |
| [`plans/12-repl-and-introspection/`](plans/12-repl-and-introspection) | M | ✓ | `loft>` interactive prompt + IR/Rust/slot-table CLI (`@PLN12`, **FINISHED** 2026-06-08).  Shipped: result echo, multi-line, error recovery, `:`-commands, value-`:vars`, line editing + history, auto-resume, identifier + member Tab completion.  Store-resident successor → @PLN14. |

### C — Clean features

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/35-match-peg/`](plans/35-match-peg) | MH | — | L3 PEG-style match patterns (cooperates with regex lib) |
| [`plans/38-sorted-slice/`](plans/38-sorted-slice) | M | **✅ CLOSED 2026-07-09 (delivered)** | A8 — slicing / open-ended ranges / partial-key match on sorted/index; shipped + per-sub-feature tests |
| [`plans/40-const-fields/`](plans/40-const-fields) | M | **✅ CLOSED 2026-07-16 (delivered)** | `const` struct fields (write-once-at-construction) — closes INCONSISTENCIES.md § 33; shipped all 8 steps + boundary matrix + hex_world dogfood |

### Q — Internal quality

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [NATIVE.md § Open work](NATIVE.md#open-work) | XS-M per item | — | N8b.3 yield-from + N8c.1/2 generic text-return audit + N20a/b fill.rs auto-gen |
| [PERFORMANCE.md § Open work](PERFORMANCE.md#open-work) | S-MH per item | P1 blocked on opcode-table capacity | 7 optimization designs (P1-P3 interpreter / N1-N3 native / W1 wasm) |
| Release performance pass (`@PLN158`) | M-MH | api-surface baselines; profile corpus | Per-routine benches over stdlib + published libraries, pure-Rust/C# reference twins, drift vs previous release; report read by `M-perf-pass` per release.  **Prototype shipped**: the drawing library's `bench/` (bench.loft + bench.rs twin + compare.py, hash-validated lanes — loft#1426); @PLN158 generalizes it |
| [`plans/45-doc-hygiene-autofix/`](plans/45-doc-hygiene-autofix) | M | — | `make plan-move` + `make doc-fix` — atomic directory-move with link-rewriting; closes the PR-212-style cascade of 3-5 fix-up commits per move |
| [`plans/52-stdlib-fast-start/`](plans/52-stdlib-fast-start) | M-MH | **CLOSED — delivered by @PLN11 arc D/D2b/E** | Precompiled-stdlib cache — hash-validated on-disk parsed stdlib, deserialize-on-startup instead of re-parsing `default/*.loft` per invocation.  Built under @PLN11 (store-backed IR) as the opt-in `LOFT_STDLIB_CACHE` (`startup_cache.rs` + `cache.rs`); tests `d2b_stdlib_cache.rs` / `arc_e_program_cache.rs`.  Miri-safe serde variant was mooted (mmap route chosen; Miri solved by `cached_default()`) |
| [`plans/2-vector-store-watermark/`](plans/2-vector-store-watermark) | S-M | kindred to finished PLAN51/52 store-lifetime class; soundness-floor A ([GOALS.md](GOALS.md)) | @P393 investigation — function-local vectors free at scope-end not last-use (cluster I) + literal-init double-alloc (cluster II).  Stage A ✓ both backends: **verified no leak**, benign watermark.  Stage B (source root-cause) + Stage C (design call: do-nothing-heuristic vs last-use-free) pending |

### N — Niche / opportunistic

| Plan | E | Depends on | Notes |
|---|---|---|---|
| [`plans/37-server-features/`](plans/37-server-features) | S-H per item | — | C55/C56/A15/I13/C57 — language features for server / game-client |
| [`plans/48-spacial-index/`](plans/48-spacial-index) | M | — | `spatial<T[x,y]>` / `spatial<T[x,y,z]>` Morton/Z-order radix spatial index |
| [`plans/49-dryopea/`](plans/49-dryopea) | H | — | dryopea sci-fi free-build / tower-defence game (consumer project) |
| [`plans/50-eagleviewer/`](plans/50-eagleviewer) | M | — | Generic branch-aware code + docs review viewer (extracted from loft viewer) |
| [`lib_plans/19-gridmesh/`](lib_plans/19-gridmesh) | M | — | `gridmesh` — chunk-local, bounded-extent grid→mesh primitives (active) |
| [`lib_plans/60-asset-pipeline/`](lib_plans/60-asset-pipeline) | M | — | Game asset workflow |
| [`lib_plans/06-web-services/`](lib_plans/06-web-services) | M-H per arc | — | JSON / HTTP client / auth / WebSocket / SSE clients |
| [`lib_plans/70-viewer-generalisation/`](lib_plans/70-viewer-generalisation) | M | — | `lib/viewer/` — extract the loft branch-review viewer as a project-agnostic library (Java + moros projects as initial customers) |
| [`lib_plans/71-terrain-heightmap/`](lib_plans/71-terrain-heightmap) | M | **✅ CLOSED 2026-08-26 (superseded)** | `terrain` — slope-based height-map generation.  `hex_terrain` 0.1.1 ships the need by another model (`tt_rise`/`tt_steep`, priority-flood hydrology); `md_slope` and `emit_slope_face` exist in no library |
| [`lib_plans/73-universal-editor/`](lib_plans/73-universal-editor) | H | — | `hex_*` universal hex-world editor libraries (moros extraction; dryopea + indie consumers) |
| [`lib_plans/74-ffi-dispatch/`](lib_plans/74-ffi-dispatch) | MH | F | FFI generated-dispatch — `#[loft_native]` proc-macro generates per-fn marshal bridges, deletes the ~98-arm `dispatch_call`; libraries own their FFI typing (supersedes 05-game-infra FFI.1/FFI.3) |
| [`lib_plans/75-physics-2body/`](lib_plans/75-physics-2body) | M | — | `physics_2body` — shared rigid-body collision + integrator for moros / dryopea / bumper-airplanes (sphere/AABB pairwise; no N-body stacking) |
| G3 tilemap rendering | S | — | The one row left from `lib_plans/61-game-infra` when it closed: draw a grid of sprite-sheet cells in one batch.  Its neighbours already ship — `graphics` has the sheet and the sprite draw, `stage` has 2-D batching — so this is a composition rather than a new primitive, and belongs in whichever of the two grows it |
| [`lib_plans/76-particles/`](lib_plans/76-particles) | S | — | `particles` — ribbon trails + point-burst particles (two-flavour scope) for dryopea + bumper-airplanes |
| [`lib_plans/77-test-deps/`](lib_plans/77-test-deps) | S | F | `loft test --deps` — transitive dep-tree test walker driven by loft.toml + loft.lock; wired into the unified library CI as a final regression-catch step (COMPLETE: T1-T3 + T6 2026-05-28, T4 `--lock` + T5 `--skip` + `--strict-deps` 2026-08-25) |
| [`lib_plans/78-loft-distribution/`](lib_plans/78-loft-distribution) | MH | **DONE 2026-07-31** | `loft` binary distribution + self-update — `install.sh` bootstrap, `loft self-update` (resolve → verify against the signed index → replace), `loft verify-self`, and the toolchain's own registry entry.  Reference content moved to [RELEASE.md](RELEASE.md) + [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md) |

### Deferred plans

Deferred plans don't appear on ROADMAP — their trigger index lives
in [`plans/DEFERRED.md`](plans/DEFERRED.md).  When a trigger fires,
the plan moves back to `future/` and ROADMAP gains a row.

### Cross-tracker dependency chains worth noting

- **lib_plans/59-lazy-stdlib → lib_plans/57-regex Phase 3** (lazy-loading wire-up; bridge MVP (Phase 0) ships independently)
- **PACKAGES.md § Open work PKG.REG → lib_plans/12-library-extraction** (registry → execution of monorepo split)
- **PACKAGES.md § Open work R1 + lib_plans/63-lsp LSP.1 → lib_plans/62-web-ide** (workspace split + LSP server → browser IDE)
- **plans/32-event-loop → lib_plans/64-game-client** (protocol abstraction → client library)
- **plans/32-event-loop → plans/33-multiplayer-editor** (depends transitively via plans/39-tic-tac-toe v2 ground layer)
- **(cross-mode harness shipped by closed @PLAN14) → plans/future/15/16/18/19/20** (the validation-matrix toolchain feeds 5 sibling validation plans — all S category)
- ~~**plans/54-sanitizer-coverage-expansion S3 (`LOFT_POISON`) → plans/53-program-level-fuzzing F4**~~ — **RESOLVED**: S3 shipped (store + stack poison-at-RESERVE), so F4 is unblocked
- **C57 / I13 (in plans/37-server-features) → lib_plans/future/08-server route decorators + iterator protocol** (language features prerequisite for server API ergonomics)

### Features still needing plan promotion

ROADMAP rows that still cite a flat reference doc as Source rather than a plan.  Promote when next-up work surfaces:

- **L1** Error recovery after token failures (U)
- **AOT** Auto-compile libraries to native shared libs (N)
- **C52** Stdlib name clash: warning + `std::` prefix (C)
- **C53** Match arms: library enums + bare variant names (C)
- **I12** Interfaces: factory methods (C)
- **A12, O2** Performance items (Q) — would fold into PERFORMANCE.md § Open work if their designs grow
- **A4** Spatial index operations (N)
