<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 21 — Prebuilt native libraries (no Rust toolchain to *use* a loft library) · `@PLN21` ([loft-lang/plans#21](https://github.com/loft-lang/plans/issues/21))

## Status

**MERGED TO MAIN (2026-06-13 — PR [#370](https://github.com/loft-lang/loft/pull/370) →
`9f43e769`).** Phases 1–6 + 4b are on the release branch. The whole in-loft pipeline is done:
produce (`loft build-native` — hand-written AND auto-compiled libs, resolved from the handed
package path) → install (host-matching prebuilt fetch) → load (`prebuilt/<triple>/`, fp-gated)
→ validate (proactive `runtime-libs` check) → diagnose (humane load/build errors) → surface
(`loft install` "Native:" block) → and a fallback source build that is reproducible
(`--locked` when a `native/Cargo.lock` is shipped) yet safely retries without `--locked` when
that lock is platform-incomplete (the Windows `cfg(windows)`-deps case that the merge's CI
caught).

**Scope correction (toolchain-stability eval, this work).** Prebuilt *distribution* is sound
for **hand-written** native libs (they link loft-ffi's C ABI → loft-ffi-versioned,
rustc-independent); **auto-compiled** libs `extern crate loft` and are loft-build + rustc-locked,
so they are NOT distributable on `loft_ffi_fp` — `build-native` reports `loft_build_fp` + rustc
for them, and a pure-loft lib is already toolchain-free by interpretation. See
[§ The boundary of this claim](#the-boundary-of-this-claim--it-holds-for-hand-written-native-not-auto-compiled-corrected-2026-06-13).

**Remaining is registry/CI integration only** — the producer *publish glue* (artifacts →
release assets → `index.json binaries`), the submit-CI gates (Phase 5/6), one end-to-end
workflow run (blocked on `prebuild-native.yml` reaching the default branch) + a manylinux glibc
baseline, and — only if auto-native distribution is pursued — the consumer fetch path gated on
`loft_build_fp`. Each needs the live registry to exercise, not local code.

The enabling invariant was *proven* this session by the cdylib re-key work
([CHANGELOG_TECHNICAL] / `cache::loft_ffi_fingerprint`): a registry cdylib is
**loft-ffi-versioned, not loft-versioned, and rustc-independent**. That makes shipping a
prebuilt binary *sound* — this plan turns the proof into distribution.

Tracked as [`@PLN21`](https://github.com/loft-lang/plans/issues/21) (loft-lang/plans) —
the cross-ecosystem id (it touches the registry every library publishes through).

## Goal — invariant

> **Using a native loft library must not require a Rust toolchain.** `use graphics`
> on a fresh machine resolves a **prebuilt cdylib** for the host platform and `dlopen`s
> it — no rustc, no cargo, no `~90s` compile. Building from source is the *fallback*,
> not the default.

Today the opposite holds: the first `use graphics` triggers `auto_build_native` →
`cargo build` of the package's native crate (rustc + cargo + the native dev headers
like `libasound2-dev`/GL), costing ~90s and a full toolchain. That is the barrier this
plan removes.

## Why this is sound now (the evidence, proven 2026-06-13)

The hard question for shipping someone-else's binary is *"is it compatible with my
loft?"* This session answered it, end to end:

- **A cdylib links loft-ffi (the C ABI), never `libloft.rlib`.** Verified by inspection:
  `nm -D` on `libloft_random.so` and `libloft_graphics_native.so` shows **zero loft
  undefined symbols**; their only NEEDED libs are system ones (`libc`, `libGL`,
  `libasound`, …). All loft callbacks arrive at runtime via the loft-ffi `#[repr(C)]`
  function-pointer table when loft `dlopen`s the lib.
- **So compatibility is keyed on loft-ffi, not loft.** Landed as
  `cache::loft_ffi_fingerprint` (a build-time hash of `loft-ffi/src`, stamped by
  `build.rs` into `LOFT_FFI_FINGERPRINT`). A cdylib built against loft-ffi vX is valid
  for *any* loft using loft-ffi vX — matrix-verified (libloft change → cdylib HIT;
  loft-ffi change → cdylib rebuilt).
- **rustc version is a non-issue for the cdylib.** `extensions.rs:2420` already decided
  this deliberately ("NO rustc-version guard here … ANY rustc builds it correctly"). A
  cdylib is `dlopen`d over C, statically embeds its own std, and exposes only
  `extern "C"`/`repr(C)`. A prebuilt built with rustc 1.89 loads into a loft built with
  rustc 1.95 unchanged. **Do NOT put rustc in the validity key** — it would manufacture
  staleness that doesn't physically exist and re-create the per-release churn we just
  removed. The rustc guard belongs only to the rlib-linking paths (`cache.rs:485`,
  `native_lib.rs:806`, `main.rs:5360`).

The one consequence to honour: the distributed artifact is the **cdylib** (`.so`/`.dll`/
`.dylib`), **not the rlib** (the PACKAGES.md format sketch showing `libgraphics.rlib` is
a trap — an rlib *is* SVH-locked to its rustc, E0514).

### The boundary of this claim — it holds for HAND-WRITTEN native, NOT auto-compiled (corrected 2026-06-13)

The `nm -D` evidence above is from **hand-written** native libs (`libloft_random.so`,
`libloft_graphics_native.so`) — and for them the claim is sound: they link **only** loft-ffi's
clean `#[repr(C)]` surface (`LoftRef`/`LoftValue`/`LoftStr` + the callback table; loft-ffi has
**zero** `use loft::`/`extern crate loft`), so a hand-written prebuilt is genuinely loft-ffi-
versioned and rustc-independent. Ship those keyed on `loft_ffi_fp`.

The **auto-compiled** native libs (the @PLN11/Phase-4b path: `glb`, `mesh3d`, `shapes`) are a
**different ABI contract** and the universal framing does NOT cover them — verified by reading a
generated cdylib's own source (`native-auto/loft_auto_<lib>.rs`):

- It begins `extern crate loft; use loft::database::Stores; use loft::keys::DbRef; use
  loft::ops; use loft::codegen_runtime::*;` — i.e. it **statically embeds all of `libloft.rlib`**
  (12.7 MB vs 435 KB for the loft-ffi-linked hand-written lib; the "zero undefined loft symbols"
  reading is because the rlib is linked *in*, not absent).
- It operates on the host's **`Stores`/`Store`/`DbRef`** by shared-memory pointer (the N9/C71
  shared-store model). Those are `#[derive(...)]` with **no `#[repr(C)]`** (keys.rs:202,
  database/mod.rs:209, store.rs:128) → **repr(Rust)** layout, which rustc may reorder and does not
  guarantee stable across versions. Even `LibArg.dbref` (a repr(Rust) `DbRef` inside a repr(C)
  `LibArg`) carries a rustc-determined inner layout.

So an auto-compiled cdylib is valid **only for a byte-identical loft build** (same source **and**
rustc). Its real compatibility key is **`loft_build_fingerprint`** (the `libloft.rlib` content
hash — which folds in both), and the local cache already gates on exactly that
(`native_lib.rs:970`). `build-native` now reports `loft_build_fp` + the rustc string for this
branch (NOT `loft_ffi_fp`), so a publish/consume path cannot mislabel it as widely portable.

**Consequence for distribution.** A pure-loft library is **already toolchain-free by
interpretation** — "no rustc to *use* it" is met without any prebuilt; native is a *host-matched
optimization*. A cross-machine auto-native prebuilt is therefore (a) unsound on `loft_ffi_fp`,
and (b) near-useless even on `loft_build_fp` (every loft point-release or rustc bump invalidates
it). So @PLN21's prebuilt **distribution** should be scoped to **hand-written** native libs;
`build-native`'s auto branch is best understood as a *loft-release-CI* tool (loft ships per-
version auto-native cdylibs gated on `loft_build_fp` + triple), not a library-author registry
artifact. The producer workflow + the (still-unbuilt) consumer `fetch_prebuilt` should likewise
target hand-written libs only.

## Significance — a source-language-agnostic native surface

Because the artifact is keyed on the **C ABI** (loft-ffi) + `dlopen` + a *versioned*
fingerprint — none of which know the source language — this is more than a Rust-ecosystem
feature. A "loft library binary" is **any C-ABI `.so` implementing the loft-ffi contract**:
Rust ergonomically today (the macros), C / C++ / Zig directly once a `loft-ffi.h` lands. It
broadens the [BROADENING.md § differentiator #4](../../BROADENING.md) thesis from "inherits
Rust's ecosystem" to "**inherits every native ecosystem, via the C ABI — toolchain-free**."
Where CPython / Node bind native code through *their own* ABI + a runtime build, loft's is
the bare C ABI + a content fingerprint: runtime-version- *and* language-independent.

And it does **not** trade away the stability differentiator, because **stability is being
*well-grounded*, not the source language**: a decades-hardened C library (sqlite, zlib,
libpng) is as stable as safe Rust — and much of Rust's "native" surface is Rust *wrapping
those very C libraries*. The prebuilt model just distributes the stable substrate (mature C,
Rust-over-C, or pure Rust) uniformly; the filter stays *well-grounded*. The binary structure
merges anything that speaks the contract; what *earns* the guarantee is grounding.

## What already exists (don't rebuild)

- **The package format defines `prebuilt/<target-triple>/`** (PACKAGES.md:82, *"avoids
  requiring Rust toolchain on consumer machine"*).
- **The resolution flow specifies it** (PACKAGES.md:910: *"Check prebuilt/ for current
  target; if missing/stale → cargo build"*).
- **The WASM path already does the per-target prebuilt check**
  (`native_utils.rs:653`, `prebuilt/wasm32-wasip2/`).
- **A native prebuilt-load hook exists** — `resolve_native_lib` (`extensions.rs:2272`)
  returns a prebuilt `.so` before falling to `auto_build_native`, but only from one
  platform-agnostic `native/<lib>` path, not the per-triple `prebuilt/<host-triple>/` dir.
- **The registry index already reserves the per-platform slot** —
  `registry_index.rs:79-87`: `Version.binaries: BTreeMap<String, BinaryEntry{url, sha256}>`,
  keyed by target triple (resolver deferred to *this* plan; PKG_REGISTRY.md:235 — *"when
  present, consumer skips local cargo build"*). Phase 4/5 wire it, not invent it.
- **dlopen failure is already non-fatal** — `extensions.rs:111-115`:
  `libloading::Library::new` failure is an `eprintln!` + silent continue (the load is
  optional). That is exactly the hook to enrich with classified diagnostics (Phase 3).
- **A host-triple helper exists** — `cache::target_triple()` (`cache.rs:212`) builds
  `<arch>-<os>-<family>` from `std::env::consts` at runtime (private; needs the libc
  refinement in Phase 1's C1).
- **The native fingerprint sidecar exists** — `auto_build_native` stamps
  `<dir>/release/.loft-build-fp` with `loft_ffi_fingerprint()` and validates via
  `native_artifact_fingerprint_matches` (this session's work). A prebuilt reuses the same
  sidecar mechanism, unchanged.

## The design — tiered native resolution

```
resolve_native_lib(pkg, stem):
  1. prebuilt/<host-triple>/<lib>  with a matching loft-ffi fp   → dlopen
  2.   dlopen fails → validate WHY (the dynamic linker IS the validator):
         missing RUNTIME system lib  → actionable error; building won't help
         loft-ffi / glibc mismatch   → fall to (3)
  3. source-build (auto_build_native + loft_ffi_fingerprint gate) → dlopen
  4.   no toolchain / missing build dev-libs → clear, actionable error
```

### Requirement A — system-wide library validation

- **The dynamic linker is the authoritative validator.** loft already `dlopen`s; on
  failure `dlopen`/`LoadLibrary` reports *exactly* what is missing
  (`libasound.so.2: cannot open shared object file`, `version 'GLIBC_2.38' not found`,
  `undefined symbol`). Validation = *attempt the load, catch the failure, respond* — not
  a hand-rolled pre-flight that can drift from reality.
- **The load-bearing distinction: a missing RUNTIME lib cannot be fixed by building.**
  A source-built cdylib links the *same* runtime lib, so it fails to load identically.
  So the tier model is not "prebuilt else build" — there is a real third outcome:
  *needs a system lib, and only the user installing it resolves it.*
- **Packages declare their runtime libs** (manifest, below) so loft turns the raw linker
  error into *"graphics needs libGL — `apt install libgl1`"* (Goal F: never hand the
  programmer a cryptic failure). Split is clean: `glb`/`mesh3d`/`random`/`imaging` need
  only `libc` (fully self-contained); `graphics` adds `libGL`/`libasound` at runtime.

### Requirement B — reliable on-demand build (the fallback)

`auto_build_native` is the path; "reliable" has concrete teeth:

- **Fingerprint-gated** — `loft_ffi_fingerprint` (this session) already ensures it
  builds only when genuinely needed, not the old per-profile churn.
- **Toolchain/dev-lib diagnostics** — distinguish "no rustc" from "missing
  `libasound2-**dev**`" (the *build*-time header, vs the runtime lib above) and name the
  fix.
- **Determinism gap to close** — the repo gitignores `Cargo.lock`, so the on-demand
  build re-resolves deps each run (the `@P388` comment flags this). A reliable build
  wants a pinned/locked resolution so two machines produce the same cdylib.

### Manifest schema — two dep sets

`loft.toml [native]` declares **both**, and the registry surfaces both:

```toml
[native]
crate        = "loft-graphics"
runtime-libs = ["libGL.so.1", "libasound.so.2"]   # validation + prebuilt install hint
build-deps   = ["libgl-dev", "libasound2-dev"]    # rustc + dev headers, for the fallback
```

- `prebuilt/<target-triple>/` ships the **cdylib** (`.so`/`.dll`/`.dylib`), each stamped
  with the `loft-ffi` fingerprint it was built against (the existing `.loft-build-fp`
  sidecar mechanism).
- Registry view: *"graphics: runs with libGL; builds with libGL-dev + glutin."*

## Design decisions (probe before building)

- **C1 — the prebuilt triple form.** A cdylib's portability key is `(arch, os, libc-on-linux)`,
  not the loose `<arch>-<os>` the PACKAGES.md sketch shows: a glibc `.so` won't load on a
  musl host, and `cache::target_triple()` (`env::consts`) doesn't capture libc. **Decision:**
  stamp the *authoritative* host triple in `build.rs` from `env::var("TARGET")` (available to
  build scripts) → `LOFT_BUILD_TARGET` (e.g. `x86_64-unknown-linux-gnu`); name prebuilt dirs
  by it. **Backstop:** even a wrong pick is *safe* — Phase-3's trial `dlopen` catches a
  glibc/abi mismatch and falls to source build; triple precision is efficiency, not
  correctness. *Probe:* confirm a gnu cdylib `dlopen`-FAILS cleanly (not UB) on a musl host.
- **C2 — the fp travels WITH the binary, two homes.** Local/tarball prebuilt → a co-located
  `prebuilt/<triple>/.loft-build-fp` sidecar (reuses `native_artifact_fingerprint_matches`).
  Registry binary → a new `loft_ffi_fp` field on the index `BinaryEntry`, so `loft install`
  rejects an incompatible binary *before* download. *Probe:* an fp mismatch must skip the
  prebuilt, never load it.
- **C3 — a missing runtime lib is terminal, not a build trigger.** Proven by mechanism: a
  source build links the *same* runtime lib, so it fails identically. The validation must
  decide build-vs-error *at resolve time* (a trial `dlopen`), not after a wasted ~90s build.
  *Probe:* a prebuilt needing an absent lib emits the install hint and does NOT build.

## Phases (designed)

### Phase 1 — Per-triple prebuilt resolve (the load hook) · S — **SHIPPED 2026-06-13**
**Goal:** loft loads `prebuilt/<host-triple>/lib<stem>.<ext>` when present and fp-valid, before any build.
**Landed:** `build.rs` stamps `LOFT_BUILD_TARGET`; `cache::host_triple()` reads it;
`resolve_native_lib` checks `prebuilt/<triple>/` (fp-gated via the `.loft-build-fp` sidecar)
before the legacy `native/` path and `auto_build_native`; a hit emits a `prebuilt` timing
event. Probe ✓ both cells: installed prebuilt → loads with **no build**; corrupted sidecar
fp → skipped, falls through to source build. (The wasm-path helper-extraction was dropped —
a wasm *rlib* and a native *cdylib* validate differently, so sharing was net-negative.)
1. **Authoritative host triple** — stamp `LOFT_BUILD_TARGET` in `build.rs` (`env::var("TARGET")`); the building loft *is* the host, so this is exact (C1). Expose `cache::host_triple()` reading it (replaces the lossy runtime `target_triple()` for this use).
2. **Shared helper** — lift the wasm logic (`native_utils.rs:652`, `pkg_dir/prebuilt/<target>/<file>`) into `fn prebuilt_lib(pkg_dir, triple, stem) -> Option<PathBuf>`.
3. **Hook `resolve_native_lib`** (`extensions.rs:2272`) — BEFORE the existing `native/<filename>` check: `prebuilt_lib(pkg_dir, host_triple, stem)`; if found AND `native_artifact_fingerprint_matches(prebuilt_dir, loft_ffi_fingerprint())` → return it; on fp mismatch → skip (never load a wrong-ABI cdylib), fall through to `native/<filename>` → `auto_build_native`.
4. **Prebuilt sidecar** — a prebuilt dir is `prebuilt/<triple>/lib<stem>.so` + `prebuilt/<triple>/.loft-build-fp` (the loft-ffi fp it was built against); reuse the existing sidecar reader unchanged.

   **Probe:** drop a locally-built `librandom.so` + a matching `.loft-build-fp` into `~/.loft/registry/random-0.1.0/prebuilt/<triple>/`; a `use random` run LOADS it (no `cdylib miss` in the ledger, no `cargo build`). Corrupt the sidecar → it falls through and rebuilds.

### Phase 2 — Manifest schema: `runtime-libs` / `build-deps` · S — **SHIPPED 2026-06-13**
**Goal:** packages declare the two dep sets validation + diagnostics need.
**Landed:** `manifest.rs` gains `runtime_libs` / `build_deps: Vec<String>` parsed from
`[native] runtime-libs` / `build-deps` (comma-list via `split_list`); unit test
`parses_native_runtime_and_build_deps` ✓; PACKAGES.md `prebuilt/` sketch corrected to the
cdylib + `.loft-build-fp` sidecar. Consumer wiring (store in `Data`, read in validation)
moves to Phase 3, where it's used.
1. **Struct** (`manifest.rs:36-46`) — add `runtime_libs: Vec<String>`, `build_deps: Vec<String>`; init `Vec::new()` in `default()` (`manifest.rs:79`).
2. **Parse** (`manifest.rs:90-125`, before the `_ => {}` catch-all) — a flat comma-list is least-friction for the hand-rolled scanner: `("native","runtime-libs") => manifest.runtime_libs = value.split(',').map(trim).collect()`, same for `build-deps`. Example `runtime-libs = "libGL.so.1, libasound.so.2"`.
3. **Consume** (`parser/mod.rs::apply_manifest_side_effects`, ~5907) — store both on the native-package record in `Data` (keyed by stem) so Phase 3 reads `runtime_libs` for the hint.
4. **Doc fix** — PACKAGES.md:82 `prebuilt/.../libgraphics.rlib` → `lib<stem>.so` (cdylib); document the two fields + the per-triple sidecar.

   **Probe:** unit test on `read_manifest` — `[native] runtime-libs = "a.so, b.so"` → `vec!["a.so","b.so"]`.

### Phase 3 — Validation + diagnostics (the dlopen-failure path) · M — **SHIPPED 2026-06-13**
**Goal:** a load failure becomes an actionable message, and build-vs-error is decided correctly (C3).
**Landed (all three):**
- **Diagnostics** — `extensions::dlopen_diagnostic` classifies the load-site error: missing
  system lib (names it + "install it; building won't help"), glibc-too-old,
  undefined-symbol/ABI — replacing the raw `eprintln`.
- **Proactive C3 check** — `first_missing_runtime_lib` reads the manifest's `runtime-libs`
  and `dlopen`s each at the TOP of `resolve_native_lib`; a missing one emits the install
  hint and returns `None` BEFORE loading a doomed prebuilt or spending ~90s on a build that
  can't load. (Read the manifest directly from `pkg_dir/loft.toml` — no `Data` wiring
  needed.) Empty `runtime-libs` → no check, no cost.
- **Build-deps diagnostics** — `auto_build_native`'s failure arm splits into "cargo ran but
  failed" (names the declared `build-deps`) vs "cargo couldn't start" (no toolchain).

Unit-tested (`dlopen_diag_tests`, **5 cells**: classifier ×3, missing-runtime-lib detect,
build-deps hint); `n0_fingerprint` regression ✓; `fmt`/`clippy` clean. The trial-`dlopen`
of an *un*declared-dep prebuilt is covered by the existing load-site diagnostic, so the
declared-libs proactive check + the load-site classifier together close C3.
1. **Trial-load in resolve** — when Phase 1 finds an fp-valid prebuilt, `libloading::Library::new` it *there* (cheap). Classify `Err`:
   - missing-lib (`cannot open shared object file`) → **terminal**: extract the lib name, cross-ref the manifest `runtime-libs`, emit *"`<pkg>` needs `<lib>` at runtime — install it"*, return `None`-with-diagnostic, **do not build** (C3).
   - `version 'GLIBC_x' not found` → message + fall to source build (a local build targets the host glibc).
   - `undefined symbol` / other → defensive: skip prebuilt, fall to `auto_build_native`.
2. **Enrich the load site** (`extensions.rs:111-115`) for the non-prebuilt path too — same classifier on the final `eprintln!`.
3. **Build-path diagnostics** — in `auto_build_native`, on a failed `cargo build`: rustc-not-found → "install Rust"; `pkg-config`/linker dev-lib error → cross-ref `build-deps` → "install `<dev-lib>`".

   **Probe:** a prebuilt whose `runtime-libs` names an absent lib → loft emits the hint and spends **no** time building (timed).

### Phase 4 — CI build matrix + publish (produce the binaries) · M — **PRODUCER PRIMITIVE + WORKFLOW SCAFFOLD 2026-06-13**
**Goal:** each native library ships per-triple, fp-stamped cdylibs in the registry.
**Landed (producer):** `loft build-native [pkg-dir]` (`main.rs`) compiles a package's native
crate via `auto_build_native` — running **no program** (a graphics lib needs no display) —
and prints the cdylib path, stem, host triple, and `loft_ffi_fp` (machine-readable for a
publish script). Verified locally on `random` (builds `libloft_random.so`, reports the fp).
`.github/workflows/prebuild-native.yml` (manual-`workflow_dispatch`, non-disturbing) mirrors
`release.yml`'s matrix, checks out a library repo, runs `build-native` per target, and
uploads each cdylib + emits its `binaries[<triple>]` index-entry JSON to the job summary.
**Remaining (the two untestable-locally refinements, noted in the workflow):** (a) a real
end-to-end CI run; (b) linux on an **old-glibc/manylinux** base for portability; (c) the
**publish glue** — a final job / `registry_maintain.sh` turning the artifacts into release
assets and writing the `binaries` entries into the package's `index.json`.
**Landed (install-side — the testable Rust half):** `BinaryEntry` gains `loft_ffi_fp:
Option<u64>` (parsed from the index, stored as a string for u64 precision);
`install::fetch_prebuilt` runs after extraction — when `Version.binaries[host_triple]`
exists AND its `loft_ffi_fp == loft_ffi_fingerprint()`, it downloads the cdylib into
`prebuilt/<triple>/lib<stem>.<ext>` (stem from the package manifest's `[library] native`),
verifies the sha256, and writes the `.loft-build-fp` sidecar — exactly where Phase 1 looks.
Best-effort: any miss (no entry / fp mismatch / offline / download or hash failure)
silently leaves the source path. `fmt`/`clippy` clean; install + registry_index suites pass
(no regression). This closes the install→load loop with Phase 1.
**Remaining (producer-side — infra, needs a real registry + CI to validate):** the
per-triple CI build matrix (mirror `release.yml`; linux on an old-glibc/manylinux base;
`auto_build_native` emits the cdylib + fp), and the `registry_maintain.sh` publish step
that uploads each cdylib as a release asset and writes the `binaries[<triple>] = {url,
sha256, loft_ffi_fp}` index entries. Untestable locally — scoped, not yet built.
1. **Per-triple build workflow** — mirror `release.yml`'s matrix (`x86_64`/`aarch64` × linux/macos/windows). Linux in an **old-glibc (manylinux-style) container**; macOS sets an old `MACOSX_DEPLOYMENT_TARGET`. Each job builds the package's `native/` crate via a loft carrying the target loft-ffi → `auto_build_native` emits `lib<stem>.<ext>` + `.loft-build-fp` (fp is free).
2. **Publish** — extend `scripts/registry_maintain.sh`: upload each cdylib as a release asset; add `binaries["<triple>"] = {url, sha256}` to the version (slot exists, `registry_index.rs`) + a new `loft_ffi_fp` on `BinaryEntry` for pre-download validation.
3. **`loft install`** (`install.rs:87`) — when `Version.binaries[host_triple]` exists AND `loft_ffi_fp == loft_ffi_fingerprint()` → download the cdylib into `~/.loft/registry/<pkg>-<ver>/prebuilt/<triple>/` + write the `.loft-build-fp` sidecar (so Phase 1 finds it); else the source path.

   **Probe:** a test index entry with a `binaries` slot + matching fp → `loft install` downloads + sidecars it → a later `use` loads without building.

### Phase 4b — Producer covers AUTO-COMPILED native too · S — **SHIPPED 2026-06-13**
**Gap found** by running `build-native` over the whole local registry: it built the four
hand-written-native libs (graphics / random / server / web) but **skipped the pure-loft
compute libs** (glb / gridmesh / mesh3d / shapes). Those are not "interpreted-only" — loft
auto-compiles a library's pure-loft API to a `loft_auto_<dir>` cdylib (exporting
`loft_shared_<fn>` wrappers) by **default** ("libraries compile, scripts interpret";
`parser/mod.rs:5922`, via `native_lib::cached_or_build_shared_cdylib`). `build-native` only
drove the *hand-written* `native/`-crate path (`auto_build_native`), so the auto-compiled libs
never got a prebuilt — they'd compile from source on first use (needs rustc), losing the
toolchain-free win exactly for the compute-heavy libraries that most want it.
**Landed:** `build-native` gains a second branch — when `manifest.native` is absent but
`[package] name` is present, it parses the library the `use`-resolution way (`use <name>;`
over the stdlib), runs `scopes::check`, computes `native_lib::library_export_set`, and calls
`native_lib::cached_or_build_shared_cdylib` to emit the `loft_auto_<dir>` cdylib + fp — then
reports `cdylib:`/`stem:`/`triple:`/`loft_ffi_fp:` exactly like the hand-written branch, so the
producer workflow ships it as a `prebuilt/<triple>/` the same way. A dir with no `[package]
name` now gets a clean "not a loft package" error instead of the old "no native stem".
**The load-bearing finding:** the codegen reads slot/scope assignments, so the minimal parse
setup **must run `scopes::check(&mut p.data)` before the build** — without it the generated
Rust references undeclared locals (`var_me`, the method receiver) and rustc rejects it. The
run path does this at `main.rs` just before its own auto-native loop; the `build-native` branch
mirrors it. (Caught exactly as the matrix-first method predicts: the first build *failed*
cleanly with `cannot find value var_me`, pointing straight at missing pre-codegen analysis.)
**Verified (cold builds, both arms):** `loft build-native` on glb/mesh3d/shapes (mesh3d/shapes
had no `native-auto` at all → true cold rustc compile) each produced a valid
`libloft_auto_<lib>.so`; the regenerated `.rs` now *declares* `var_me` (`let mut var_me: f32`);
the hand-written `random` arm still reports `loft_random`; `/tmp` → clean non-package error.
`fmt`/`clippy` clean.

**Producer verified end-to-end against a real library repo (2026-06-13).** A faithful local
replay of the producer workflow's build job (clone `loft-lang/loft-libs-graphics`, run
`build-native` on the `shapes` subdir, sed-extract `cdylib:`/`triple:`/`loft_ffi_fp:`, checksum)
caught a **real bug in the auto-native branch**: `use <name>` resolves a *same-named registry*
package, not the handed checkout, so `library_export_set` (filtering defs by `pkg_str` prefix)
saw none of them → "no native-compilable public functions" → the workflow `exit 1`s. The local
unit tests passed only by coincidence (they pointed at `~/.loft/registry/<name>-<ver>`, exactly
where `use <name>` resolves). **Fixed:** canonicalize the handed path and push its *parent* to
`p.lib_dirs`, so `use <name>` resolves `<parent>/<name>` (and sibling monorepo deps) before the
registry fallback; the registry-install layout (`shapes-0.2.0` ≠ name `shapes`) still misses
lib_dirs and falls through to the registry as before. Regression matrix (all pass): registry
auto-native (glb), hand-written (random → `loft_random`), fresh checkout (shapes → built).

**The bigger correction (toolchain-stability eval, 2026-06-13):** an auto-native cdylib
`extern crate loft`s — it embeds libloft and shares repr(Rust) `Stores`/`DbRef`, so it is
loft-build + rustc-locked, **not** loft-ffi-versioned. See [§ The boundary of this
claim](#the-boundary-of-this-claim--it-holds-for-hand-written-native-not-auto-compiled-corrected-2026-06-13).
`build-native`'s auto branch now reports **`loft_build_fp` + `rustc:`** (not `loft_ffi_fp`), so
it can't be mislabeled. The recommendation: scope @PLN21's prebuilt **distribution** to
hand-written libs; the gaps below only matter if auto-native distribution is later pursued as a
loft-release-CI artifact.

**Two gaps remain for a true ship-and-consume of an AUTO-native prebuilt** (both belong to the
publish/consume glue, not the producer):
1. **No consumer fetch path.** `install.rs::fetch_prebuilt` only handles hand-written libs — it
   `return false`s when `[library] native` is absent, so an auto-native prebuilt is never
   downloaded/placed. Phase 1's `resolve_native_lib` likewise keys off the manifest stem. (A
   consumer must gate on `loft_build_fp`, not `loft_ffi_fp` — see the correction above.)
2. **Layout-dependent cdylib identity.** `auto_cdylib_stem` derives from the *directory*
   basename, so the producer (checkout dir `shapes`) builds `libloft_auto_shapes.so` while a
   consumer (installed `shapes-0.2.0`) looks for `libloft_auto_shapes_0_2_0.so`. The fix is a
   layout-independent identity (package name+version from `loft.toml`, the auto analog of the
   hand-written `[library] native` stem) — touches the run-path cache filename, so verify both
   "run from source checkout" and "run from registry install" find/build/load the same cdylib.

**Producer GitHub-Actions run is still blocked** on `prebuild-native.yml` not being on loft's
default branch (`workflow_dispatch` requires it there). The reusable `workflow_call` entry can
be referenced `@<branch>` from a library repo's own CI without that, but its `Checkout loft`
step pins the default branch — so until Phase 4b reaches `main`, a CI run would build a
pure-loft lib with a loft that lacks the auto-native branch. Path to a real run: land the
workflow + Phase 4b on `main`, then `gh workflow run prebuild-native.yml -f
library_repo=loft-lang/loft-libs-graphics -f package_subdir=graphics`.

### Phase 5 — Registry surfacing · S — **SURFACING DONE 2026-06-13**
**Goal:** the runtime requirement + available prebuilts are visible.
**Landed (the `loft install` surfacing):** `InstallReport` gains a `surface: Vec<String>`;
`install_one` populates a per-package "Native:" block via `prebuilt_status(host, available,
installed)` — "prebuilt installed for `<triple>` — no Rust toolchain needed" / "no prebuilt
for `<triple>` (available: …) — builds from source (needs rustc)" / "a `<triple>` prebuilt
exists but was built against a different loft-ffi" — plus a "runtime libraries: …" line from
the manifest's `runtime-libs`. `format_report` renders it. `prebuilt_status` is pure and
unit-tested (`prebuilt_status_branches`, 4 cells); install suite passes; `fmt`/`clippy` clean.
**Remaining (the submit-CI gate — registry infra):** the REGISTRY_SUBMIT.md gate that
validates each `binaries` entry's `sha256` + `loft_ffi_fp` and requires `runtime-libs` to be
declared when a prebuilt's `objdump -p` NEEDED list has a non-`libc` entry — needs the
registry submit pipeline, like Phase 4's publish glue.

### Phase 6 — Build determinism (the fallback's reliability) · M — **SHIPPED 2026-06-13**
**Goal:** the source-build fallback is reproducible across machines.
**Landed:** `auto_build_native` passes `--locked` to the `cargo build` **when the package
ships a `native/Cargo.lock`** — cargo then uses the pinned resolution and refuses to drift,
so two machines produce the same cdylib bytes. The `@P388` note that `--locked` was
unavailable applied to the loft repo's *own* gitignored locks; a published registry package
commits its native lock (the packaging convention this enables). Matrix-verified on a /tmp
copy of `random`: **(A)** a freshly-generated lock → `build-native` succeeds (`--locked`
doesn't break a valid build); **(B)** a stale lock (a dep added to `Cargo.toml` after the
lock) → `cargo: cannot update the lock file … because --locked was passed` → the build
fails, proving `--locked` is in effect (and the Phase 3 build-failure diagnostic fires on
top). `fmt`/`clippy` clean.
**Remaining (convention + gate, not loft code):** package authors commit `native/Cargo.lock`
in the tarball, and the reproducible-build submit gate (REGISTRY_SUBMIT gate 3) gains the
deterministic native-build check — registry-side, like the Phase 4/5 infra tails.

## Order & critical path

`1` and `2` are independent and small — land first (prebuilts become *loadable* + *declared*).
`3` builds on `1`+`2` (trial-load + the `runtime-libs` hint). `4` produces what `1` loads
(depends on `1`; the index slot already exists). `5` depends on `2`+`4`. `6` is independent.
**Critical path to "no rustc to use graphics": `1 → 4`** (a load path + one published prebuilt);
`2`/`3` make the failure modes humane, `5`/`6` are polish.

## Starting distribution — the publish glue (remaining critical-path step, 2026-06-14)

Producer (`loft build-native` + `prebuild-native.yml`) and consumer (`fetch_prebuilt` +
`resolve_native_lib`) both ship; the gap is the wiring that gets a built cdylib INTO the
registry index.  Verified prereqs (2026-06-14): `graphics-v0.1.0` exists in
`loft-lang/loft-libs-graphics`, but that repo has **no prebuild caller workflow yet**.  Minimal
end-to-end to seed the first prebuilt (graphics) — scoped to **hand-written** native libs (the
auto-compiled, `loft_build_fp`-locked case stays out, [§ The boundary of this
claim](#the-boundary-of-this-claim--it-holds-for-hand-written-native-not-auto-compiled-corrected-2026-06-13)):

1. **Build + attach the cdylibs to the release.**  The workflow's `gh release upload` fires only
   on `workflow_call` with `publish: true`.  Two ways:
   - *one-off (proves the loop now):* `gh workflow run prebuild-native.yml -f
     library_repo=loft-lang/loft-libs-graphics -f package_subdir=graphics` builds the 4 cdylibs as
     artifacts (each job prints `{url, sha256, loft_ffi_fp}`); then download + `gh release upload
     graphics-v0.1.0 <cdylibs>`.
   - *repeatable:* add the ~10-line caller (LIBRARY_PUBLISH.md § 4b) to each library repo so a
     version tag auto-builds + attaches.
2. **Add the index `binaries` entry.**  Under the version in `index.json`: `"binaries": {
   "<triple>": { "url", "sha256", "loft_ffi_fp" }, … }` (already parsed by
   `registry_index::BinaryEntry`).  Sign + push with `scripts/registry-sign.sh` (shows the diff,
   verifies, signs).
3. **Verify the consume path.**  `loft install graphics` on a clean host → `fetch_prebuilt`
   matches the host triple + `loft_ffi_fp`, downloads + sha256-checks the cdylib, `dlopen`s it —
   no rustc.
4. **Automate (make it routine):** the per-repo caller workflow; a tool that collects the
   job-summary `binaries` entries into the index; the submit-CI gates (Phase 5 — sha256/fp +
   `runtime-libs` when `objdump -p` NEEDED has a non-`libc` entry); the manylinux glibc baseline
   (Phase 6 / Risks).

**Timing:** seed prebuilts right AFTER cutting the loft release that activates the trust root —
same `loft_ffi_fp` everywhere, so the seeded `binaries` match the released loft.

## Risks / open questions

- **glibc baseline** — prebuilts must be built on an OS old enough that a newer host's
  forward-compatible glibc loads them (the manylinux problem). Decide the baseline per
  platform; this is a build-host choice, *independent of rustc*.
- **Panic across the FFI boundary** — loft-ffi must `catch_unwind`/abort at the edge
  (UB otherwise). A correctness invariant the prebuilt path inherits; audit loft-ffi.
- **The C-ABI discipline is the load-bearing invariant** — rustc-independence holds only
  while loft-ffi stays strictly `repr(C)`/`extern "C"` with no leaked Rust types. If that
  ever broke, rustc *would* matter. `loft_ffi_fingerprint` catches loft-ffi changes; an
  `api-lint`-style check that loft-ffi's public surface is C-ABI-only would guard the
  invariant directly.
- **Windows/macOS specifics** — `.dll` import semantics and macOS `@rpath`/codesigning
  for a downloaded `.dylib` (Gatekeeper) need their own validation in Phase 3/4.

## Connections

- **PACKAGES.md** — the `prebuilt/` format + resolution flow (the design home).
- **PKG_REGISTRY.md** — the publish/install pipeline (Phase 4/5 land here).
- **`cache::loft_ffi_fingerprint` / `extensions::resolve_native_lib`** — the loft-side
  load + validity gate (Phase 1/3).
- **GOALS.md Goal F** (friction-free) + the "loft inherits Rust's ecosystem via
  in-language bindings, minus the crash surface" thesis — this plan is what makes that
  ecosystem usable without a toolchain.
