<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# Windows session — history

The record behind [WINDOWS_SESSION.md](WINDOWS_SESSION.md).

## Retired with the rustc writer (@PLN225)

Session priorities that concerned the rustc writer's `--native` (a package's Rust cdylib or
rlib linked against `libloft.rlib`), kept as they stood.  The pre-flight then also built
`cargo build --release --lib` — the unhashed `libloft.rlib` `--native` linked.  The argfile
pattern from PR #228 (`tests/native.rs`) was noted as a possible template for the G3 fix.

> **2026-05-30 blocker for ALL `--native` priorities below (G2/G3/G4-native).**
> On the local host these could not be exercised end-to-end: (1) the
> pre-existing `libloft.rlib` was a stale **gnu**-target build → `E0461` under
> the msvc `--native` compile (cleared by `rustup run
> stable-x86_64-pc-windows-msvc cargo build --release`); then (2) the rebuilt
> **unsigned** `loft.exe` is blocked by the host's **WDAC code-integrity
> policy** (CodeIntegrity 3077, "blocked by an application control policy").
> `cargo test` works (cached verdict); standalone `loft --native …` does not.
> **Code-sign `loft.exe` after each build** (user validates with a YubiKey)
> before attempting G2/G3/G4-native.  Detail: WINDOWS.md § The 2026-05-30
> native-execution wall.

### Priority 2 — ~~G2 LNK1181~~ — FIXED 2026-05-30 (windows-latest CI)

**Root cause found + fixed on `windows-validation` branch.**  Two versions of
`windows_x86_64_msvc` (0.48.5 top-level, 0.52.6 in graphics stack) were in
play; `build_script_native_lib_dirs` only harvested the top-level build root,
so the 0.52.6 lib dir was never passed as a linker `/LIBPATH`.  Fix:
`src/native_utils.rs::add_native_extern_flags` now also calls
`build_script_native_lib_dirs(rlib_path.parent())` for each native package's
own build dir.  Verified: `--check --lib lib tests/fixtures/p310_save_png.loft`
exits 0 on `windows-latest` (CI run 26690846366).

Note: validated on the GitHub runner (no WDAC), NOT on the local WDAC-blocked
host.

**Done in this change:** the `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean`
LNK1181 silent-skip branch is removed; `p310` now asserts `out.status.success()` on
every platform.  Remaining: full Windows `nextest` validation runs on this PR's
`windows-latest` CI leg.

### Priority 3 — ~~G3 transitive-rlib~~ — FIXED 2026-05-30

Root cause (proven): concurrency artefact in `src/extensions.rs::auto_build_native`.
`auto_build_native` runs `cargo build` with no `--locked`/`--frozen`, re-resolving the
full dependency tree on every on-demand build.  Under parallel nextest, concurrent
re-resolution races with a standalone `loft --check --lib lib` rustc link → ureq/rustls
rlibs transiently mid-rebuild → "required to be available in rlib format".  The earlier
"environmental / no longer reproduces" framing was WRONG: a cold CI run (rust-cache
"No cache found") still failed p310 on G3; a rust-cache trial did NOT help.

Fix: CI pre-builds all four native lib packages SEQUENTIALLY in `.github/workflows/ci.yml`
before the nextest step, so `auto_build_native` is a no-op during the suite.

Verified: CI run 26694041810 — 2268/2268 Windows, 0 failures; `p310` dropped from
~86-113s to 2.7s.  The `tests/codegen_emitter.rs` "required to be available in rlib
format" silent-skip branch is removed; `p310` asserts `out.status.success()` on every
platform.

Residual latent concern → @P388: the underlying unlocked re-resolution is still present
and can bite end users running parallel `loft --native`/`--check` invocations.
