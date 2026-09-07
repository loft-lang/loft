---
name: loft-ship
description: >-
  Write, bridge, verify, and publish a loft library so it behaves IDENTICALLY on all four
  targets — the interpreter, --native, --native-wasm, and --html (browser). Use this skill
  whenever the user is creating, packaging, cross-compiling, or publishing a loft library or
  package; mentions loft.toml, the target matrix, a wasm bridge, `loft new` / `loft install`,
  the registry, the sha256/signature, or making a library "work in the browser" / "work in
  wasm"; or is debugging native-vs-wasm-vs-interpret divergence in a library. Trigger it even
  when the user doesn't say "all four platforms" — any loft library that has to ship beyond the
  interpreter belongs here, because the cross-target parity gate and the wasm-bridge pitfalls
  are exactly where this work silently goes wrong.
---

# loft-ship — one source, four targets, identical behavior

A loft library ships to **four targets**, and the runtime — not the consumer — picks the
variant ([PACKAGES.md § Target matrix](../../../doc/claude/PACKAGES.md)):

| | Interpreter | `--native` | `--native-wasm` | `--html` (browser) |
|---|---|---|---|---|
| Pure loft / `#rust` inline | ✓ | ✓ | ✓ | ✓ |
| `#native` external (Rust) | ✓ rlib | ✓ rlib | ✓ wasm rlib | **✗ → needs a `wasm.bridge` crate** |

**The master invariant: `interpret == native == native-wasm == browser`.** A library is not
shipped until it produces the *same observable result* on every target it claims. That
invariant is the whole job — everything below serves it.

## Do this first: the Tier gate (it decides how much work you're in for)

Before scaffolding anything, classify the library — this is the single most leverage-saving
step, because the two tiers differ by an order of magnitude:

- **Tier 1 — pure loft, or `#rust` inline only.** The matrix is ✓ across all four columns:
  the compiler emits the variant for each target automatically. **There is no bridge to
  build.** Shipping = write → `loft.toml` → cross-mode test → publish. Most libraries are
  here; keep them here (prefer `#rust` inline over `#native` external whenever the Rust is
  small) precisely so you never enter Tier 2.
- **Tier 2 — `#native` external Rust bindings.** The browser column has no automatic path:
  you must hand-build a **`wasm.bridge` crate** (host-import module + JS/WASI host shim). This
  is where the real cost and nearly all the bugs live. Only pay it when the binding genuinely
  can't be `#rust` inline (it calls a real host capability: crypto, sockets, the DOM, files).

State which tier you're in out loud — it sets expectations and stops you from over-building a
Tier 1 library or under-estimating a Tier 2 one.

## The workflow

1. **Scaffold** — `loft new <name>` (or copy a sibling library's `loft.toml`). See
   [LIBRARY_AUTHORING.md](../../../doc/claude/LIBRARY_AUTHORING.md) for the full author
   narrative.
2. **Write the loft surface** — the `.loft` API. For naming/types/format-strings/known-bugs,
   use the **`loft-write` skill** (this skill is about *shipping*, that one is about *writing
   `.loft`*). Keep the public surface clean per
   [LIBRARY_CHECKLIST.md](../../../doc/claude/LIBRARY_CHECKLIST.md) (Goals A–F apply per
   library; the stdlib is just the library every program imports).
3. **Declare the target matrix in `loft.toml`** — there is NO `targets =` field; the real
   knobs are `[build] default-targets` / `[build.target.<name>]` and `[test] targets`
   (PACKAGES.md § Build targets), plus a `.wasm_exempt` marker for a justified wasm
   opt-out (LIBRARY_CHECKLIST.md). Run the suite per target with
   `loft --tests --native-wasm <dir>` (and its siblings). Tier 2 adds a
   `[wasm.bridge]` block. Don't claim a target you haven't passed the parity gate on — a
   claimed-but-broken target is worse than an honestly-omitted one.
4. **(Tier 2 only) Build the wasm bridge** — the host-import module, the host shim, asyncify
   for any suspending call, and the value marshalling across the boundary. This is its own deep
   procedure with non-obvious traps → **read `references/wasm-bridge.md` before writing a
   single bridge line.**
5. **Run the parity gate** (below) — the definition of done.
6. **Publish** — build per target, sign, submit to the registry. The signing step has a
   foot-gun that breaks *every* install if missed → **read `references/publish.md`.**

## The parity gate — the definition of "shipped"

Run the library's own tests on **every target it claims**, and confirm the *results match* —
not just that each exits 0. Bound any `--native`/wasm run with `LOFT_TIMEOUT` (the build can
hang). The gate, in order of how often each catches a real bug:

1. `--interpret` — the baseline (fastest; the reference result).
2. `--native` — compiled Rust. Interp/native divergence is the #1 library bug class.
3. `--native-wasm` — headless wasm (WASI). Catches VirtFS / no-threading assumptions.
4. `--html` (Tier 2) — the browser bridge. Catches the host-shim + asyncify mistakes.

A target passes only when its result *equals the interpreter's*. "Native works" or "the suite
is green" does **not** close a wasm/browser divergence — that divergence **is** the bug (it's
the same failure mode the stability red-flags chase: two generators reading one IR, drifting).
Add a leak check where the library allocates (`LOFT_STORES=warn`, `LOFT_NATIVE_LEAK_CHECK`).

## Pitfall index — the things that actually cost hours (hard-won, @PLN84)

These are not theoretical; each one bit a real ZT library this cycle. Skim them before you
start, not after you're stuck.

- **The re-sign foot-gun (publish).** Editing the registry `index.json` *without re-signing*
  it breaks **all** `loft install`s — every install fails with "registry index
  signature INVALID" (`src/install.rs` verifies before anything else). Every registry PR must re-sign. → `references/publish.md`.
- **Asyncify for suspending calls (bridge).** A bridge function that yields/awaits (a socket
  read, a frame yield) needs asyncify wiring; `yield_frame()` only *sets a flag*, it does not
  trigger a suspend — a dedicated suspend import (e.g. `loft_web.ws_yield`) does. → bridge ref.
- **Boundary-marshalling drift (bridge).** The value marshalling across the wasm boundary
  is where silent corruption hides; validate it with a round-trip *value* check, not "it
  didn't crash."
- **Index staleness (publish).** `loft install` reads `index.json` via the raw-GitHub CDN
  and keeps a local ~1h-TTL cache of its own (`~/.loft/registry/`). A just-published version may not resolve as `@latest` immediately — pin the exact
  version to verify, and don't conclude "publish failed" from a stale edge.
- **Claiming the browser column for a `#native` lib without a bridge.** The matrix says ✗
  there for a reason — it will compile-fail or silently no-op. Either build the bridge or drop
  the `--html` claim.

## References (read on demand — don't inline them)

- `references/wasm-bridge.md` — the Tier-2 `[wasm.bridge]` recipe: host-import module, host
  shim (`host.js` browser / Node headless), asyncify, the marshalling traps, and the @PLN84
  history. Read it
  the moment a library needs the browser target.
- `references/publish.md` — the registry publish runbook: `loft.toml` targets, per-target
  build, Ed25519 sign + **re-sign**, sha256/size, the CDN gotcha. Read it before any registry
  submission.
- Canonical depth lives in the repo docs — this skill is the *procedure + the gate + the
  traps*; point into [PACKAGES.md](../../../doc/claude/PACKAGES.md),
  [WASM.md](../../../doc/claude/WASM.md), [HTML_EXPORT.md](../../../doc/claude/HTML_EXPORT.md),
  [PKG_REGISTRY.md](../../../doc/claude/PKG_REGISTRY.md),
  [REGISTRY_SUBMIT.md](../../../doc/claude/REGISTRY_SUBMIT.md) for the full reference rather
  than duplicating them.
