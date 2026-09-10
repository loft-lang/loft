<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Windows support — verified state, known gaps, and a VM-validation runbook

> **Looking for "what to do when you finally get Windows access for a day"?**
> See [WINDOWS_SESSION.md](WINDOWS_SESSION.md) — priority-ordered checklist,
> time budget, pre-flight steps.  This doc is the reference; the session
> doc is the action plan.

## Honest compatibility statement

The CI matrix runs the full test suite on `windows-latest`
(`.github/workflows/ci.yml`) **daily (03:00 UTC schedule) and on every
push-to-main — NOT on every PR**: the Windows leg costs ~30 min and almost
never regresses independently of the Linux/macOS legs that DO run per-PR.  On
a PR the required `Test (windows-latest)` context is a non-blocking placeholder
and the non-required **`Windows (daily)`** job mirrors the latest daily result
(green = last daily passed, red = it failed) so the state is visible without
re-validating — a red there never blocks the merge, it is a nudge to open a
focused Windows-fix session.  A set of tests are also **skipped or
`#[ignore]`d on Windows** because they hit failures no contributor has been
able to reproduce or diagnose without a real Windows machine.  So the
accurate claim today is:

| Path | Windows status |
|---|---|
| **`--interpret`** (single program, no native libs) | ✅ **Verified** — the bulk of the suite runs + passes on Windows CI |
| **`--interpret`** with a `#native` library (dlopen the cdylib) | ⚠️ **Mostly** — imaging etc. pass; multi-lib + networking caveated below |
| **`--native`** linking a native library (rlib link) | ✅ **Verified 2026-05-30 (CI)** — G2 fixed (per-package link-search harvest in `native_utils.rs`); G3 fixed (CI pre-build eliminates concurrent `auto_build_native` re-resolution race; CI run 26694041810: 2268/2268 Windows, 0 failures); `--check --lib lib` exits 0 on `windows-latest`.  `tests/codegen_emitter.rs::p310_graphics_vector_ffi_checks_clean` LNK1181 + G3 silent-skip branches removed; `p310` asserts `out.status.success()` on every platform.  Residual latent concern: @P388 (end-user parallel invocations). |
| **`--native`** linking a native library (C-ABI dylib link, @PLN26 ph.4) | ✅ **Verified 2026-06-15 (focused CI) — now the DEFAULT on every host.**  The package's cdylib is linked by C-ABI (sealing its Rust crate graph, killing the StableCrateId class).  Windows links the DLL through its import library — `-l dylib=<stem>` wants `<stem>.lib`, so the arm copies the cdylib's `<stem>.dll.lib` → `<stem>.lib`; **no RPATH** (the DLL is staged beside the binary, `native_utils::stage_native_dlls`).  `native_cabi_enabled()` returns `true` everywhere; `LOFT_NATIVE_CABI=0` is the escape hatch back to the legacy rlib link.  Verified in focused CI (not a VM): `win-cdylib.yml` job `win-cdylib-cabi` on `windows-latest`, `native_crate_package_links_and_runs_via_cabi` PASS (36/36).  Two Windows-only gaps were fixed en route: the `loft --native test` path now propagates loft's own build-script `OUT_DIR`s (windows-targets `windows.X.lib`, was `LNK1181`), and the import-lib naming above. |
| **Server networking** (`server` bind/accept) | ✅ **Verified 2026-05-29 (CI), re-verified 2026-05-30 (local host)** — the v2 probe (PR #228) un-ignored `v2_single_client_completes_game` on Windows and it PASSED; the other 9 P229b ignores were dropped in the follow-up.  `@P229b` closed without code change (incidental fix in a recent Rust toolchain or transitive dep update).  Independently re-confirmed on a real Windows host (rustc 1.96.0 MSVC): all 10 P229b tests across `multiplayer_v{2,3,5}.rs` pass (v2: 3, v3: 2, v5: 5) when each suite runs in isolation. |
| **`parallel { … }`** (`--interpret`) | ✅ **Verified 2026-05-30 (local host)** — `tests/scripts/80-parallel-block.loft` + `81-parallel-outer-vars.loft` (the @P245 outer-var snapshot guard) both exit 0; `tests/threading.rs` 47/47 pass incl. the `par_ref_buffer_stack_*` worker-stack-snapshot cells. |
| **`parallel { … }`** (`--native`) | ✅ **Verified 2026-05-30 (CI, windows-latest)** — `80-parallel-block.loft` + `81-parallel-outer-vars.loft` compiled + executed under `--native`, exit 0 (CI run 26689698213).  G4 fully closed. |

RELEASE.md *intends* to ship a `x86_64-pc-windows-msvc` binary with a
"hands-on smoke test before publishing."  The `--native` and server paths
are now CI-verified.  The two silent-skip branches in
`tests/codegen_emitter.rs` (LNK1181 and "required to be available in rlib
format") are dropped in this change; `p310` now guards the fix on every
platform.  Remaining: the full Windows `nextest` regression suite runs
automatically on this PR's `windows-latest` CI leg.

This doc is the runbook to close that gap.  The user can spin up a Windows
VM and work each gap → reproduce → capture the real error → fix → un-skip.

## Why these can't be closed from Linux/macOS

Every gap is a *Windows-toolchain* or *Windows-OS-semantics* issue
(MSVC linker search paths, `TIME_WAIT`/exclusive-port semantics, rlib
discovery under the MSVC target).  They do not reproduce on Linux or macOS,
and CI gives only a post-mortem log — not an interactive shell.  A VM with
the MSVC toolchain is the missing piece.

## Windows VM prerequisites

- Rust stable with the **`x86_64-pc-windows-msvc`** target (the default on a
  Windows host) + the **MSVC build tools** (`link.exe`).
- `cargo-nextest` (the suite uses it) and `gh` if validating against CI.
- **No `mold`** — the repo forces `-fuse-ld=mold` only on Linux; confirm the
  Windows build uses the default MSVC linker (check `.cargo/config.toml`
  guards by target).
- Clone, then `cargo build --release` + `cargo build --release --lib` (the
  unhashed `libloft.rlib` the `--native` path links — see
  `reference_native_rlib_rebuild`).
- **Pin the toolchain target.**  If both `-gnu` and `-msvc` toolchains are
  installed, build the rlib with the SAME target `--native` compiles against
  (msvc), or E0461 fires (see § The 2026-05-30 native-execution wall).  Use
  `rustup run stable-x86_64-pc-windows-msvc cargo build --release` to be
  explicit.
- **Native execution needs a runnable, policy-permitted `loft.exe`.**  On a
  host with WDAC / Smart App Control in Enforce, a freshly-built unsigned
  `loft.exe` is blocked at load (CodeIntegrity 3077).  `cargo test` still
  works (cached verdict), but standalone `loft --native …` requires the
  binary to be code-signed with a trusted cert after each build.

## Known gaps

### Engine-host kernel on Windows — PROBED 2026-06-11 (the focused windows-probe loop)

The `windows-probe` branch + `.github/workflows/windows-probe.yml` (the
non-PR platform-debug loop, user-directed) answered the @PLN18 questions the
unix-gated lifecycle suite leaves open.  Warm-cache round time: ~40 s.

**Verified working on `windows-latest`:**
- **The kernel SERVES**: listen (the std-bind fallback), WS upgrade, event
  echo over world state, drift-free 10 ms ticks — the whole serve path
  (`tests/windows_probe.rs::probe_kernel_serves_on_windows`).
- **Sequential rebind is instant**: close-listener → rebind the same port
  succeeds after **~72 µs** (no TIME_WAIT penalty for listeners).

**Measured constraints:**
- **No bind overlap**: a second plain bind on a held port → `AddrInUse`
  (the first listener unaffected).  The S5 swap's unix shape (SO_REUSEPORT
  overlap, rollback-by-default because the old build never stops
  listening) does NOT port as-is.
- **Windows swap design (from the numbers)**: a TWO-PHASE handover —
  new build restores the world and signals *restored* (pre-bind READY
  variant) → old closes its listener + signals GO → new binds (~72 µs)
  and serves.  Rollback: no *restored* within the deadline → the old
  build never closed, keeps serving; post-GO child death → the old
  REBINDS (~72 µs) and resumes.  The freeze window grows only by the
  close-to-bind gap (negligible); the choreography gains one round-trip.

**New bug found (and FIXED 2026-06-11 — never Windows-specific):**
- **`parse_str` died on any `use` clause** — resolving a `use` halts the
  current file and later RESUMES it by re-opening its NAME, which for a
  virtual source (`<win-probe>`, REPL snippets, live-reload's
  `"<live-reload>"`) is not an openable path on ANY platform (the Linux
  cell failed identically — the probe merely found it first).  Fix: the
  lexer registers `parse_string` sources by name and `switch` re-serves
  them from memory (mirroring the wasm VIRT_FS branch).  Regression:
  `tests/parse_str.rs` (cross-platform) + the windows-probe serve test
  (the Windows leg — VALIDATED on windows-latest 2026-06-11, run
  27380012429: parse_str + use + serve all green).

**Round 2 — PROBED 2026-08-24** (`tuxedo-windows-probes`; dispatch with
`gh workflow run windows-probe.yml --ref <branch> -f tests="--test windows_probe"`
— run 32749933258, 7/7 green in 10.5 s).  Two of the three remaining questions
are answered, and one of them named a defect:

- **The grandchild IS orphaned — FIXED 2026-08-24.**  `stop_game` reaches a
  `--native` game's real server through `killpg`, which is `cfg(unix)`; the
  Windows path had only `child.kill()`.  Measured: after the child is killed the
  grandchild still holds its port (`probe_child_kill_reaches_the_grandchild`).
  So a stopped game kept serving, and the next launch met its own port taken.
- **`taskkill /T` is the cure, and no Job Object is needed** — it walks the
  parent link and terminates the grandchild (`SUCCESS: The process with PID N
  (child process of PID M) has been terminated`), releasing the port
  (`probe_taskkill_tree_reaches_the_grandchild`).  **The ordering is the
  finding**: it needs the child ALIVE to walk from, so it runs BEFORE
  `child.kill()`.  Run it after and there is no tree left to walk.
- **UDP beside TCP on one port: BOUND** — the 05a auto-path's shape works.  A
  SECOND UDP bind on that port is `AddrInUse`, and the TCP listener keeps
  accepting throughout (`probe_udp_beside_tcp_on_one_port`).  Different
  protocols do not contend, so nothing is owed here.

**Remaining unprobed**: the full flip/reload/rebuild lifecycle (its virtual-name
blocker is fixed, so it is now reachable).  And one NEIGHBOUR of the orphan
class, deliberately not fixed alongside it because it was not measured: the swap
ROLLBACK in `engine_host.rs` kills its handover target with a bare
`child.kill()`, which orphans that target's own grandchild the same way — on
BOTH platforms, since the target is put in its own process group precisely so a
group kill cannot reach it.


### ~~G5~~ — LSP `file://` URIs built with backslashes are invalid JSON — FIXED 2026-07-25

- **Symptom:** the Windows nightly's 11 `lsp_transport` timeouts (600s), all and
  only the tests that set a workspace `rootUri`.
- **Root cause:** `format!("file://{}", path.display())` on Windows yields
  `file://C:\Users\…` — the backslashes are invalid JSON escapes (`\U`), so loft's
  strict `json::parse` rejects the whole `initialize` message, the server skips it,
  never replies, and the client blocks. On POSIX the path is forward-slashed, so it
  works. The server EMITTED URIs the same broken way, so real Windows editors got
  malformed `file://C:\…` back — not a test-only bug.
- **Fix:** the platform-agnostic pair `loft::lsp::path_to_uri` / `uri_to_path`
  (`src/lsp.rs`). ALWAYS convert a path↔URI through them — never `format!("file://…")`
  or `strip_prefix("file://")` by hand. `path_to_uri` renders `C:\a\b` as
  `file:///C:/a/b` (never a backslash → always valid JSON); `uri_to_path` inverts it
  to native separators. Verified on the `windows-probe` custom CI.
- **Confirmed on the real leg 2026-07-26:** daily run `30190084089` on `d1a5840c` —
  `Test (windows-latest)` ran the full 53 minutes and PASSED, whole run green. The
  preceding daily (`30146048916`, sha `06fb917c`) failed and predates this fix, so
  the red→green transition is unambiguous rather than inferred. Note the real leg
  only runs on push-to-main and the daily schedule; a PR's "Windows (daily)" check
  merely mirrors the last scheduled run, so it stays red on a PR until a new one
  lands.

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

### ~~G4~~ — `parallel { … }` worker stack snapshot (`@P229`) — FULLY CLOSED 2026-05-30

- **Interpreter half VERIFIED CLEAN (2026-05-30, local host):** `81-parallel-outer-vars.loft`
  exits 0 under `--interpret`; `tests/threading.rs` 47/47 pass incl. the
  `par_ref_buffer_stack_*` worker-stack-snapshot cells.
- **Native half VERIFIED CLEAN (2026-05-30, windows-latest CI run 26689698213):**
  `tests/scripts/80-parallel-block.loft` + `81-parallel-outer-vars.loft`
  compiled + executed under `--native` on the GitHub `windows-latest` runner,
  exit 0.  The runner does NOT enforce the WDAC code-integrity policy that
  blocked the local host, so freshly-built unsigned binaries run normally.
- **`@P229` G4 is fully closed.**  Both interpreter and native parallel paths
  are verified on Windows.  Details in PROBLEMS.md `@P229`.

## The 2026-05-30 native-execution wall

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

## Previously fixed Windows-only issues (for context)

- **The daily's five Windows failures were two code defects, not five test-gating
  decisions (fixed 2026-09-10, probed via the windows-probe loop).**  `main` had been red
  on `Test (windows-latest)` with 5 of 4716 failing, each on both attempts.

  **`pid_alive` answered `None` on Windows**, so the dead-only scratch sweep fell back to
  the age rule and three tests asserting decidability failed.  Its doc said liveness was
  "unknowable on a non-unix host", but the test's own cfg structure said otherwise: the
  pid-1/EPERM cell is already `#[cfg(unix)]` while the other three assertions are left
  general, so the unix-only part had been separated deliberately and the rest was a
  cross-platform contract Windows never implemented.  It is implementable with no new
  dependency — `unsafe extern "system"` on kernel32, as `src/android.rs` already declares
  its imports.

  ⚠ **`WaitForSingleObject` is the obvious spelling and it is INERT here.**  It compiled,
  it typechecked against `x86_64-pc-windows-msvc`, and the probe reported WAIT_FAILED
  (`0xffffffff`) for EVERY openable process, live or dead, because
  `PROCESS_QUERY_LIMITED_INFORMATION` does not grant SYNCHRONIZE — so every openable pid
  would have answered `None` and all three tests would still be red.  Use
  `GetExitCodeProcess`, which needs no extra right.  What the runner reported:

  | pid | `GetExitCodeProcess` | `SYNCHRONIZE`+`Wait` |
  |---|---|---|
  | own process, pid 4 (System), a live child | `259` (STILL_ACTIVE) | `0x102` WAIT_TIMEOUT |
  | a child that exited with 7 | `7` — the handle still opens | `0x0` WAIT_OBJECT_0 |
  | pid 0, `u32::MAX-1`, an unused 999999 | `OpenProcess`=NULL, err 87 | same |

  Both discriminate; `GetExitCodeProcess` wins on least privilege.  Its STILL_ACTIVE
  ambiguity (a process exiting WITH code 259 reads alive) is accepted because it errs
  toward not reclaiming, and the sweep DELETES what it calls dead.  pid 0 needs no special
  case as it does on unix, where 0 addresses a process GROUP: Windows reports it as no
  process.

  **`verify_self::manifest_path_escapes` asked `is_absolute()`, which is unix-shaped while
  the rule is not.**  Its own doc states the hazard platform-neutrally — "`root.join
  ("/etc/x")` REPLACES the root" — but on Windows `Path::new("/etc/x").is_absolute()` is
  FALSE, because an absolute path there needs a drive prefix, while `join` still keeps only
  the prefix: `<drive>:\bundle` joined with `/etc/x` is `<drive>:\etc\x`.  So the one home
  of the bundle escape rule waved through the very escape it exists to refuse, on the one
  host where the predicate disagreed with the rule — a real hole, not a test fixture.  Asked
  as a COMPONENT walk now (`Prefix | RootDir | ParentDir`), which also catches the
  drive-relative `C:x` and the UNC shapes.  Measured: identical to the old predicate on
  ZERO of 22 shapes on unix, so the whole effect is on Windows.

  ⚠ **A unix-passing guard for a Windows-only defect is VACUOUS and must say so.**  On unix
  `is_absolute()` already answered `/etc/loft-evil` correctly, so every portable cell of the
  new guard passes under the pre-fix predicate; only its four `#[cfg(windows)]` cells carry
  the change, and the falsification happened on windows-probe rather than in the file.

  **The fifth, `doc_hygiene::every_test_binary_matches_a_subject`, is load-dependent and
  its cause is still unknown.**  It reported `scripts/test_subjects.sh failed: ` with stderr
  EMPTY — a message naming nothing — while a probe of the identical command passed (exit 0,
  zero unmatched binaries, bash 5.3.15 Cygwin), and neither the script nor the `tests/*.rs`
  set changed since the daily's commit.  Two failures wore one exit code: the script
  REFUSING prints the binary name on stdout, the shell failing to RUN prints nothing.  The
  guard now retries only the empty-empty shape (a genuine finding has stdout, so the retry
  cannot mask one) and its message carries the exit code and both streams, so the next
  occurrence is evidence rather than a dead end.

- **A `[c]` shim published by rename imported a name nobody published
  (fixed 2026-08-04, probed via the windows-probe loop).**  A program using a
  package with a `[c] shim` died with `STATUS_DLL_NOT_FOUND` (`0xC0000135`)
  before `main`, both streams empty.  The class: **a linker records the name it
  was GIVEN, not the name you rename to.**  loft builds the shim to
  `<stem>.<pid>.tmp` and renames it over the final name so the publish is
  atomic; `--out-implib` writes the import library during that build and records
  the DLL name from `-o`, so the `.lib` said `.tmp`.  The rename moved the file
  and left the recorded name behind, and every binary linking that `.lib` copied
  the temporary in as the thing to ask the loader for.  Read straight off the
  program's PE import table on the runner: it asked for
  `lcshim_shim_<key>.8496.tmp` while `lcshim_shim_<key>.dll` sat in the same
  directory.  **The fix is a staging DIRECTORY with the artifacts already
  carrying their FINAL names**, then a rename into place: the recorded name
  follows the BASENAME (a staging path does not leak into it), and the rename
  still lands in the same directory, so it stays atomic.
  This is the same class as the macOS install-name bug, which is why
  `platform.rs` already says *every artifact loft publishes by rename needs
  this* — but the Windows arm is not an install name at all, so
  `install_name_args` returning empty off macOS left it open.  macOS keeps its
  explicit `-install_name` regardless: it bakes in the whole `-o` PATH, so the
  staging directory would otherwise leak into it.
  **Two hypotheses died on the way, both cheaply**: the shim imports only
  `KERNEL32` and the UCRT, so the MinGW runtime (`libgcc_s_seh-1`,
  `libwinpthread-1`) was never involved and `-static-libgcc` had already done
  its job; and `-Wl,--soname,<final>` fixes nothing because **PE ignores it** —
  verified side by side with the staging shape (`.tmp`+rename → imports `.tmp`,
  exit 127 · `--soname` → imports `.tmp`, exit 127 · built under the final name
  → imports the real DLL and runs).  The lesson worth keeping: **a missing
  import names no name**, so guessing is unbounded and reading the import table
  is the only instrument that converges.
- **#460 skip missed on Windows — verbatim vs plain path representation
  (fixed 2026-07-02, probed via the windows-probe loop).**  The class: on
  Windows `fs::canonicalize` returns an extended-length `\\?\D:\…` verbatim
  path, and a verbatim path never equals or prefix-matches its plain twin
  (`VerbatimDisk` vs `Disk` prefix components), for `Path::starts_with` and
  string prefix checks alike.  `abs_file` deliberately sheds the prefix
  (@P296), but the `lib_dirs` canonicalization in `src/main.rs` re-introduced
  it, so every path derived from `lib_dirs` (use-candidates, the package dirs
  recorded into `pending_native_compile`) was verbatim while the entry path
  was plain — the #460 entry-package skip compared plain vs verbatim,
  `starts_with=false`, and the entry package auto-native-compiled after all
  (`entry_package_is_never_auto_native_compiled` red on `windows-latest`
  since it landed in #464; earlier same-class instance: P244's malformed
  verbatim concat in `auto_build_native`).  The rule: **one path
  representation everywhere — every canonicalized path entering the shared
  path space goes through `strip_verbatim_disk` (src/main.rs)**.  Probe
  evidence (runs 28602820918 / 28603951574): before,
  `pkg_dir=\\?\D:\…\selfpkg starts_with=false`; after,
  `pkg_dir=D:\…\selfpkg starts_with=true`, no `native-auto/`, both entries
  clean.
- **`@P229b` — server cannot bind on Windows (closed 2026-05-29; re-verified
  on a local host 2026-05-30).** The 10
  `multiplayer_v{2,3,5}.rs` scenarios marked `#[cfg_attr(target_os = "windows",
  ignore = "P229b…")]` are all un-ignored.  The v2 probe (PR #228 commit
  `baa9c3e2`) un-ignored `v2_single_client_completes_game` and CI showed it
  PASSING on `windows-latest` — the 2026-05-21 "bind-then-drop race"
  hypothesis was incorrect; @P229b incidentally resolved in some recent
  Rust toolchain or transitive dep update.  Other 9 dropped in the same
  cycle.  Linux + macOS still pass; Windows now matches.  WINDOWS_SESSION.md
  Priority 1 was executed on a real Windows host (rustc 1.96.0 MSVC, 2026-05-30):
  all 10 P229b tests pass (`multiplayer_v2` 3/3, `multiplayer_v3` 2/2,
  `multiplayer_v5` 5/5), and no `cfg_attr(windows, ignore)` gate remains in
  those files.  Note: running several `multiplayer_v*` test binaries
  concurrently (plus a parallel single-test instance) once made
  `v2_late_join_independent_games` exceed its 60s client timeout (empty
  stdout, exit `None`) — a CPU/port-contention artifact, not a bind failure
  (a real @P229b bind error surfaces via `diagnose_listen_failure`, not a
  silent client timeout); the test passed on isolated re-run.
- `@P332` — `install::install_one` return on Windows (fixed 2026-05-26).
- `@P333` — `/tmp/` hard-coded paths in two lib fixtures → CWD-relative
  (fixed 2026-05-26).
- Heap-corruption-style failures: TESTING.md § (Windows `STATUS_HEAP_CORRUPTION`
  caught real out-of-bounds that Linux's allocator slack hid — a case where
  Windows CI was *more* sensitive and *useful*).

## Closing a gap — the loop

1. Reproduce on the VM with the command in the gap's "VM validation".
2. Capture the *real* error (CI only shows a post-mortem; the VM gives a shell).
3. Apply the fix; re-run; confirm green on Windows.
4. **Remove the skip/ignore** (the gate is the lie — deleting it is the proof).
5. Move the gap to "Previously fixed"; update PROBLEMS.md / TESTING.md.
6. G2, G3, and G4 are closed; the `--native` row in § compatibility is ✅.
   The codegen_emitter skip removal is done; CI run 26694041810 (2268/2268 Windows)
   confirms all three gaps closed.  Residual latent concern: @P388.

## See also

- `.github/workflows/ci.yml` — the 3-OS matrix.
- `src/native_utils.rs` — `build_script_native_lib_dirs`, `add_native_extern_flags`
  (the `--native` link/rlib-discovery the gaps live in).
- PROBLEMS.md `@P229` · RELEASE.md (Windows binary intent).
