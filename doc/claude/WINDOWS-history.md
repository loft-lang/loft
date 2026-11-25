<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# Windows support — history

The record behind [WINDOWS.md](WINDOWS.md).

## Retired with the rustc writer (@PLN225)

Windows gaps of the rustc writer's `--native` route, which linked a package's Rust cdylib or
rlib against `libloft.rlib`.  The writer is retired (`@C170` in
[DESIGN_DECISIONS_PLATFORM.md](DESIGN_DECISIONS_PLATFORM.md)); the sections are kept as they
stood.

The compatibility rows they backed:

| **`--native`** linking a native library (rlib link) | ✅ **Verified 2026-05-30 (CI)** — G2 fixed (per-package link-search harvest in `native_utils.rs`); G3 fixed (CI pre-build eliminates concurrent `auto_build_native` re-resolution race; CI run 26694041810: 2268/2268 Windows, 0 failures); `--check --lib lib` exits 0 on `windows-latest`.  `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean` LNK1181 + G3 silent-skip branches removed; `p310` asserts `out.status.success()` on every platform.  Residual latent concern: @P388 (end-user parallel invocations). |
| **`--native`** linking a native library (C-ABI dylib link, @PLN26 ph.4) | ✅ **Verified 2026-06-15 (focused CI) — now the DEFAULT on every host.**  The package's cdylib is linked by C-ABI (sealing its Rust crate graph, killing the StableCrateId class).  Windows links the DLL through its import library — `-l dylib=<stem>` wants `<stem>.lib`, so the arm copies the cdylib's `<stem>.dll.lib` → `<stem>.lib`; **no RPATH** (the DLL is staged beside the binary, `native_utils::stage_native_dlls`).  `native_cabi_enabled()` returns `true` everywhere; `LOFT_NATIVE_CABI=0` is the escape hatch back to the legacy rlib link.  Verified in focused CI (not a VM): `win-cdylib.yml` job `win-cdylib-cabi` on `windows-latest`, `native_crate_package_links_and_runs_via_cabi` PASS (36/36).  Two Windows-only gaps were fixed en route: the `loft --native test` path now propagates loft's own build-script `OUT_DIR`s (windows-targets `windows.X.lib`, was `LNK1181`), and the import-lib naming above. |

### ~~G2~~ — `--native` `windows-targets` link search path (`LNK1181`) — FIXED 2026-05-30

- **Root cause:** a diamond dependency pulls TWO versions of
  `windows_x86_64_msvc`.  The graphics native stack (winit/glutin/rodio/cpal)
  pulls 0.52.6, built under `lib/graphics/native/target/release/build/…`; the
  top-level stack pulls 0.48.5, built under the top-level
  `target/release/build/…`.  `build_script_native_lib_dirs` was only invoked
  on the TOP-LEVEL build root (`src/main.rs:3199`), so the 0.52.6 crate's
  link-search dir (`…\windows_x86_64_msvc-0.52.6\lib`) was never added →
  its `windows.0.52.0.lib` was passed to the linker as a bare filename with
  no `/LIBPATH` → `LNK1181: cannot open input file 'windows.0.52.0.lib'`.
  The 0.48.5 copy linked fine because its dir WAS harvested.
- **Fix (`windows-validation` branch):** `src/native_utils.rs::add_native_extern_flags`
  now also harvests `rustc-link-search` dirs from each native package's OWN
  `<profile>/build/*/output` (via `build_script_native_lib_dirs(rlib_path.parent())`)
  and adds `-L native=<dir>` for each.  This supplies the missing
  `…\windows_x86_64_msvc-0.52.6\lib` LIBPATH.
- **Verified:** CI run 26690846366 — `loft --check --lib lib tests/fixtures/p310_save_png.loft` exits 0 (was 1 / LNK1181).
- **Open follow-up:** `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean`
  still carries a silent-skip branch that matches `LNK1181`.  Now that the
  link succeeds, this branch masks a passing case.  Remove it and assert the
  link succeeds on Windows; run a full Windows nextest suite to confirm no
  regressions.

### ~~G3~~ — `--native` multi-lib transitive-rlib not found (`ureq`/`rustls`) — FIXED 2026-05-30

- **Root cause (proven, not the earlier cache/environmental theory):** concurrency
  artefact in `src/extensions.rs::auto_build_native`.  `auto_build_native` runs
  `cargo build --release` with no `--locked`/`--frozen`; each on-demand native-package
  build re-resolves the full dependency tree ("Locking 169 packages / Adding ureq
  v2.12.1 / Blocking waiting for file lock on package cache").  Under parallel nextest,
  many tests triggered `auto_build_native` concurrently; the concurrent re-resolution
  churned the shared `~/.cargo` + target dirs while a standalone `loft --check --lib
  lib` rustc link needed ureq/rustls rlibs from `target/release/deps` → they were
  transiently mid-rebuild / wrong form → "required to be available in rlib format".
  The `taiki-e/install-action@nextest` "bash startup failure" flake amplified the
  failure by triggering a concurrent cargo-install fallback — why G3 correlated with
  truncated CI runs.  The earlier "G3 no longer reproduces / environmental" claim was
  WRONG: a genuinely cold CI run (rust-cache logged "No cache found") still failed p310
  on G3; a rust-cache trial did NOT help.
- **Fix:** CI now pre-builds all four native lib packages (graphics, web, server,
  imaging) SEQUENTIALLY in a "Pre-build native lib packages" step in
  `.github/workflows/ci.yml`, BEFORE the parallel nextest Test step.  `auto_build_native`
  finds the rlibs already present and is a no-op during the suite — no concurrent
  re-resolution.  Side diagnostic improvement: `src/main.rs` also dumps the rustc
  invocation on the "required to be available in rlib format" error (previously only
  on E0460/E0463).
- **Verified:** CI run 26694041810 — full Windows suite ran UNTRUNCATED, 2268/2268
  passed, 0 failures; `p310_graphics_vector_ffi_checks_clean` dropped from ~86-113s
  to 2.7s (mechanistic proof `auto_build` is now a no-op).  ubuntu + macOS also green.
- **codegen_emitter skip branches removed:** the `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean`
  silent-skip branches for LNK1181 and "required to be available in rlib format" are
  removed; `p310` asserts `out.status.success()` on every platform.
- **Residual latent concern → @P388:** the underlying `auto_build_native` unlocked
  re-resolution is still present and can bite end users running parallel `loft
  --native`/`--check` invocations (same concurrent-cargo race, same rlib-format error).
  Builds are also non-deterministic (may pick newer dep versions).  Fix direction:
  `--locked`/`--frozen` + committed `Cargo.lock` per lib native package, and/or
  serialise `auto_build_native`.

### The 2026-05-30 native-execution wall

The 2026-05-30 local-host session validated the interpreter + server paths
but could **not** exercise any `--native` path end-to-end.  Two layered
blockers, both **environmental** (not loft bugs):

1. **`E0461` — stale gnu-target rlib.**  The host had both
   `stable-x86_64-pc-windows-gnu` and `-msvc` toolchains installed (msvc
   default), and the pre-existing `target/release/libloft.rlib` was a
   **gnu-target** build, while `--native` compiles against **msvc**:
   `error[E0461]: couldn't find crate 'loft' with expected target triple
   x86_64-pc-windows-msvc`.  No `rust-toolchain.toml` pins the target.
   *Cleared* by an explicit `rustup run stable-x86_64-pc-windows-msvc cargo
   build --release` (rebuilds both `loft.exe` and the rlib for msvc).
   Lesson for G2/G3 work: confirm the rlib's target triple matches the
   `--native` compile target before trusting any link-stage diagnosis.
2. **WDAC code-integrity policy blocks the rebuilt, unsigned `loft.exe`.**
   With the msvc rlib in place, every freshly-built `loft.exe` (release AND
   the `--native` temp binaries) is refused at load:
   `CodeIntegrity` event **3077** — *"did not meet the Enterprise signing
   level requirements or violated code integrity policy (Policy ID:
   {0283ac0f-fff1-49ae-ada1-8a933130cad6})"*; the user-facing error is
   *"This file is blocked by an application control policy."*  Confirmed it
   is **not** a toolchain issue: the older **debug** `loft.exe` (also local,
   also unsigned) still runs — the policy keys on the file **hash /
   reputation**, so rebuilding produces a new hash with no cached-good
   verdict and is blocked.  `cargo test` continues to work because those
   test binaries already carry a cached verdict.  Smart App Control is in
   **Enforce** (`VerifiedAndReputablePolicyState = 1`); disabling SAC is a
   one-way door (requires a Windows reset to re-enable) and was declined.
- **To unblock native validation on this host:** code-sign each freshly
  built `loft.exe` with a trusted cert after every build (the user validates
  with a YubiKey-backed signing key), then re-run the `--native` checks.

**Note (2026-05-30 follow-up):** Although the local host remained WDAC-blocked,
all `--native` gaps (G2 root-cause + fix, G3 concurrency root-cause + CI pre-build
fix, G4-native verification) were addressed on the GitHub `windows-latest` CI
runner, which does NOT enforce WDAC and runs freshly-built unsigned binaries
normally.  CI run 26689698213 confirmed G4-native clean; CI run 26690846366
confirmed the G2 fix (per-package link-search harvest) makes `--check --lib lib`
exit 0; CI run 26694041810 confirmed G3 fixed (2268/2268 Windows, 0 failures;
`p310_graphics_vector_ffi_checks_clean` dropped from ~86-113s to 2.7s).
G2, G3, and G4 are all closed.  The `tests/codegen_emitter.rs` silent-skip
branches for LNK1181 and "required to be available in rlib format" are removed;
`p310` is the cross-platform regression guard.  Residual latent concern: @P388
(end-user parallel `auto_build_native` race).
