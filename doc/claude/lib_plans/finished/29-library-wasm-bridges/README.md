<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 29 — library-owned wasm bridges (`--html` extension model)

**Status — DONE 2026-05-29.**  All four phases (W1+W2+W3+W4) shipped
in a single day on `libraries4` (commits `1c12d7ec` → `a49b2345` →
`b1831793` → `cd89c84b` → this commit).

Reference for the shipped pattern lives in
[PACKAGES.md § Wasm bridges](../../../PACKAGES_TARGETS.md#wasm-bridges-library-owned---html-extensions) —
the three-part library layout (`wasm/src/lib.rs` + `wasm/host.js` +
`[wasm.bridge]` manifest), the rustc-direct compile recipe, the
`LOFT_WASM_EXTENSIONS` self-registration mechanism, and the
canonical `lib/imaging` example.  This file is a closure record only.

## What shipped

The compiler / tooling crate names ZERO library symbols anywhere
related to wasm bridges.  Each phase removed one source of
library-specific knowledge from a shared location:

| Phase | What | Commit |
|---|---|---|
| **W1a + W1b** | Parallel `loft-imaging-wasm` crate at `lib/imaging/wasm/` + `[wasm.bridge]` manifest declaration in `lib/imaging/loft.toml` (inert; existing routing still used `src/wasm_imaging.rs`) | `1c12d7ec` |
| **W1c + W1d** | Manifest reader populates `Data::wasm_bridge_routes`; codegen reads dynamic lookup (replaces hard-coded `WASM_BRIDGE_FNS`); `--html` driver invokes `rustc --crate-type rlib` directly (not `cargo build` — see PACKAGES.md § Why rustc-direct); `src/wasm_imaging.rs` deleted (kept generic `asset_exists` as `src/wasm_assets.rs`); `WASM_BRIDGE_ALLOWLIST` deleted | `a49b2345` |
| **W2** | Library JS host imports (`imaging_query` / `imaging_copy_rgb` / `imaging_save`) moved from `doc/loft-gl-wasm.js` to `lib/imaging/wasm/host.js`; new `[wasm.bridge].host_js` manifest key; driver concatenates each library's host.js into the HTML preamble; dispatch loop applies `LOFT_WASM_EXTENSIONS` callbacks to imports.  `host_asset_exists` + `decodeLoftAssets` stayed (library-agnostic) | `b1831793` |
| **W3** | `tools/wasm_repro.mjs` discovers `lib/*/wasm/host.js` at startup, evals each, runs the same dispatch loop as the production preamble.  Inline imaging stubs removed | `cd89c84b` |
| **W4** | PACKAGES.md `## Wasm bridges` section + Target matrix gains `--html` column + plan README trimmed to closure record + @P321(c) cross-link updated | this commit |

## Verification

- `cargo test --release --test html_wasm wasm_library_suite -- --nocapture`
  shows `wasm[node] imaging/14-image.loft: ok` (10/10 wasm runs).
- `cargo test --release --test extraction_hygiene` — 4/4 green; the
  bare-name grep covers wasm bridges with no carve-outs.
- Full `./scripts/find_problems.sh` sweep: zero failures.
- Standalone repro: extract wasm from a fresh `--html` bundle, drop
  `map.png` alongside, `node tools/wasm_repro.mjs /tmp/img.wasm`
  → `loft_start: OK`.

**Net effect**: adding a new wasm-bridge library now requires editing
exactly three files inside `lib/<X>/`: `wasm/src/lib.rs`,
`wasm/host.js`, `loft.toml::[wasm.bridge]`.  Zero `src/`, `doc/`, or
`tools/` touches.

## Second-consumer audit (2026-05-29)

| Library | Bridge needed? | Why / why not |
|---|---|---|
| `lib/imaging` | YES (shipped) | PNG codec; store-mutating |
| `lib/graphics` | NO — separate mechanism | `loft_gl_*` extern imports via `doc/loft-gl-wasm.js`'s OpenGL→WebGL2 case (PACKAGES.md § OpenGL).  Not store-mutating. |
| `lib/server` | N/A | Native sockets — browser can't run by construction (in `LIB_PKGS_WASM_SKIP`). |
| `lib/web` | NO | `ureq` HTTP client doesn't apply in browser; `fetch()` would change the loft API too. |
| `lib/world` | NO | Pure-loft + file I/O; browser FS limitation is on the loft side via `host_fs_*`, not library-specific. |
| Others (`markdown`, `html`, `time`, …) | NO | Pure-loft or cross-target via stdlib bridges. |

Conclusion: `imaging` is the only motivator today.  Second consumers
(likely an audio library bridging Web Audio, or a `lib/fs_browser`
shim) would arrive as separate plan slots and reuse the pattern.

## Reconciling with plan 12 + plan 25

- **Plan 12** drains library `n_*` symbols from the compiler crate.
  Plan 29 does the analogous drain for the wasm-bridge surface.
- **Plan 25** generalised the INTERPRETER FFI dispatch via
  `loft-ffi-build` + `loft_register_bridges!`.  Plan 29 is the WASM
  analogue: a per-library wasm-extension crate registered through
  the package manifest.

## See also

- [PACKAGES.md § Wasm bridges](../../../PACKAGES_TARGETS.md#wasm-bridges-library-owned---html-extensions) — design reference.
- [PROBLEMS.md @P321](../../../PROBLEMS.md) — the bug that triggered this plan (browser-WASM dimension); now FIXED.
- [`../../12-library-extraction/`](../../12-library-extraction) — sibling drain of compiler-crate library code on the non-wasm side.
- `lib/imaging/loft.toml` + `lib/imaging/wasm/{Cargo.toml,src/lib.rs,host.js}` — the canonical implementation.
