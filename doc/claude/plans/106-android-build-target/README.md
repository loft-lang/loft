<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN106 — loft Android build target (`aarch64-linux-android`)

**Tracker:** [`loft-lang/plans#106`](https://github.com/loft-lang/plans/issues/106) — the
B2–B4 arc (APK packaging · EGL/GL surface · touch/IME). B0/B1 are this doc's §0–§1b.

## Status — ✅ SHIPPED / DONE (2026-07-15, on `tuxedo-work`)

The invariant held all the way through: a **build target is a descriptor over ONE
target-agnostic generated core** (`src/android.rs` is the chokepoint), so every capability
below runs an **UNCHANGED** loft program. All proven on the KVM-accelerated Android emulator
(x86_64 twin via `LOFT_ANDROID_TARGET=x86_64-linux-android`; ship target `aarch64-linux-android`).

| Phase | What shipped | Proof |
|---|---|---|
| **B0/B1** | invariant falsified-and-held; `--native-android` → bionic AArch64 `.so` | §0/§1b below |
| **B2** | NativeActivity `android_main` + signed APK packaging inside loft | `b2-spike/` |
| **B3** | GLES-3.0 backend for `lib/graphics` — clear → shapes/shaders → **text** | `b3-spike/*_golden.png` |
| **B4 touch** | `gl_mouse_*` ← android-activity `MotionEvent`s | `b4_touch_golden.png` |
| **B4 IME** | `gl_show_keyboard()` + `gl_key_pressed` ← `KeyEvent`s | `b4_ime_golden.png` |
| **audio** | oboe/AAudio via `audio_play_raw` (vector ABI fix) | `b4_audio_golden.png` |
| **deep recursion** | 512MB `android_main` stack (matches `NATIVE_MAIN_STACK`) | `b4_stack_golden.png` |

Commits `b270145b`→`17938565` (this session) on top of the B0–B2 arc. Reference for
**using** the target lives in [NATIVE.md § Android target](../../NATIVE.md#android-target);
the implementation is `src/android.rs` + the cfg-gated `android_gl.rs` in the graphics fixture.
**On the C backend:** the design that takes this target off rustc and onto the published
libraries' C shims is [c-backend.md](c-backend.md).
**Known follow-up (not blocking):** B4 IME covers `KeyEvent`s only — NativeActivity has no
`TextEvent`, so IME *composing* text would need a new `gl_text_input()` text-stream API.

Everything below §0 is the **HISTORICAL design record** (the hypothesis that was executed).

## 0. What B0 found (the falsification pass — run, not guessed)

Verified on this box with NDK r27c (`aarch64-linux-android`, API 24):
- **Lean core rlib cross-compiles clean** — `cargo rustc --lib --no-default-features
  --crate-type rlib --target aarch64-linux-android` → `libloft.rlib`, exit 0. An rlib
  build runs NO linker, so this isolates the codegen question: **the generated core is
  target-agnostic** (all the `cfg` gates are `wasm32`-vs-not, which Android takes the
  host side of; `cfg(unix)` covers Android too). Invariant holds.
- **The ONLY blockers are toolchain wiring**, exactly as §1 predicted: (a) the final
  `cdylib` `.so` link needs the NDK `clang` linker (the host `ld` rejects EM 183 /
  AArch64 objects — "file in wrong format"); (b) the FULL default-feature graph fails at
  ONE crate, `ring`'s build script (`cc-rs: failed to find tool
  "aarch64-linux-android-clang"`) — every other dep (`rustls`, `ed25519`,
  `curve25519-dalek`, `zip`, `tar`, `flate2`, `webpki`) cross-compiles. Both resolve once
  the NDK is installed; neither is a codegen fork.
- **End-to-end gate PASSED:** with the NDK, a trivial program's generated `.rs` links to
  `libb0.so` = `ELF 64-bit LSB shared object, ARM aarch64`, `NEEDED libc.so`/`libdl.so`
  (bionic, not glibc's `libc.so.6`). The full `libloft.so` (default features, incl.
  `ring`/TLS) also links — so ssh_home's networking chain is viable when B wires it.

## 1b. What B1 + B2 landed

`src/android.rs` (`AndroidTarget` descriptor — the §1 chokepoint, all Android knowledge in
one file) + a `--native-android [out.so]` flag in `main.rs`. It reuses the identical
`output_native_reachable` emit (no codegen path); the difference from `--native` is the
**runtime entry** and the toolchain.

- **B1** proved a headless `cdylib` `.so` cross-compiles + links with the NDK.
- **B2 (the runtime entry)** emits a real **`NativeActivity`**: `--native-android` wraps the
  emitted program (its `fn main` intact) in a small generated cargo crate whose crate root is
  the program plus a fixed `android_main` tail (via the `android-activity` crate, which
  exports `ANativeActivity_onCreate`) that runs `main()` and pipes the program's stdout into
  logcat. Cargo owns the `android-activity` dep tree and the lean `loft` cross-build
  (`--no-default-features`, `random`+`threading`; networking/TLS `ring` is a later phase per
  §5).
- **B2 (packaging)** produces a **signed, installable APK** directly, driven by the output
  extension: `loft --native-android app.apk` runs the NativeActivity `.so` build, then
  discovers the Android SDK (`ANDROID_HOME`) + a JDK and does `aapt2 link` (a generated
  code-less `NativeActivity` manifest, package `com.loft.<name>`) → add `lib/<abi>/` via `jar`
  → `zipalign` → `apksigner` (a per-tree debug keystore). An explicit `*.so` output still
  builds just the library (NDK only, no SDK).

**Proven end to end:** `loft --native-android app.apk` output installs + launches on a KVM
emulator; logcat shows `loft --native-android: android_main reached` + the program's own
output (`sum of squares 0..8 = 140`). Tests: `tests/android_target.rs` (NDK-gated build
test asserting the AArch64 ELF exports the entry symbols + an always-on no-NDK error test).
**To run:** `ANDROID_NDK_HOME=<ndk> ANDROID_HOME=<sdk> loft --native-android app.apk prog.loft`
(default output is a `.apk`; pass `app.so` for library only). Build the emulator twin with
`LOFT_ANDROID_TARGET=x86_64-linux-android` (override the API with `LOFT_ANDROID_API`).

## 1. The one invariant (the design is a hypothesis about this)

> **A loft build target is a *descriptor* — `(rust target triple, linker/toolchain, runtime
> entry shape, packaging)` — over ONE target-agnostic generated-Rust core. Android is a new
> descriptor, NOT a new codegen path.**

If this holds, the generated Rust that `--native` already emits compiles for
`aarch64-linux-android` with only *toolchain + entry + packaging* differences — no
per-target branching inside codegen. The whole plan rides on it, so **B0 exists to falsify
it** (design-protocol: probe the cleanest claim first). loft's existing targets are the
evidence for it: `--native` (host), `--native-wasm` (`wasm32-wasip2`), `wasi`, and the
configurable `build.target.*` family all reuse the same IR→Rust emission and differ only in
the descriptor fields above. Android is the fourth such descriptor.

**Re-assertion sites (design-protocol step 2 — where the triple/toolchain must agree, and
omission is a silent link failure, not a compile error):** (a) the rustc `--target`; (b) the
linker (NDK `clang` for that API level); (c) every non-`loft` dependency rlib must be built
for the *same* triple (the wasm path already learned this — `main.rs:3454` builds the
bridge's dep rlibs for wasm32 without cargo); (d) the crate-type (`cdylib` `.so`, not `bin`);
(e) the runtime entry (`android_main`, not `fn main`). Five sites → collapse them into one
**target descriptor struct** the whole build reads, so adding Android is *data*, not five
edits. This is the chokepoint; if the build wants a sixth `if target == android` branch,
the descriptor is wrong.

## 2. Current architecture (verified in `src/main.rs`, `src/native_utils.rs`)

- `--native` → emit Rust → invoke `rustc` for the host; `--native-wasm [out.wasm]` →
  `wasm32-wasip2`; `wasi`; `build.target.*` selects `native | html | wasi | …`.
- The dep-rlib cross-build precedent (`main.rs:3454`) already produces non-`loft` dependency
  rlibs for a *foreign* triple without cargo — Android reuses exactly this machinery for the
  NDK triple.
- `lib/graphics` native backend = glutin + winit + gl + fontdue (backend crate vendored at
  `tests/fixtures/libs/graphics/native/`, host GL via `src/native_utils.rs` / `src/wasm_gl.rs`).

## 3. The three gaps (from `../ssh_home/DESIGN.md` §7; none exist today)

1. **Cross-target + packaging.** No `aarch64-linux-android` target. Need: the NDK toolchain
   (target triple + NDK `clang` linker + sysroot), `cdylib` crate-type, and an APK wrapper
   (`android-activity` `android_main` entry, `AndroidManifest.xml`, the `.so` under
   `lib/arm64-v8a/`). Signing/`aapt`/`zipalign` via the Android SDK build-tools.
2. **`lib/graphics` Android backend.** Desktop = glutin+winit on X11/Wayland; Android = **EGL
   on `ANativeWindow`** via `android-activity`, a **GLES-3.0** subset. Because the stack is
   already winit-based, this is a re-target of the surface/context creation, not a rewrite —
   the `gl_*` API surface is unchanged.
3. **Input: touch + IME.** `gl_key_pressed` is is-key-down over 0–255 keycodes (no
   char/IME); no `Touch`/gesture events. winit already carries both — wire winit `Touch` +
   the IME/char events through `lib/graphics` so the soft keyboard (password entry) and
   tap/drag/pinch work. This gap is cross-target (helps desktop Unicode too).

## 4. Phased plan (each phase independently landable + dogfooded by ssh_home)

**Verification legend** (each phase is gated by ONE concrete, checkable artifact — mirrors
`../ssh_home/PLAN.md`): **(C)** compiles/links (a named `.so`/rlib exists); **(R)** runs on an
emulator/device (a specific `logcat`/stdout line); **(G)** golden PNG (`gl_screenshot` == a
committed golden under tolerance); **(I)** an input event produces a named on-screen effect.
A phase is DONE only when its gate passes on-device, not in prose.

- **B0 — spike / falsify the invariant. ✅ DONE (2026-07-14).** Confirmed: see §0. The
  exact steps that either compile or name the blocker (kept for reproducibility):
  ```sh
  # 0. one-time: rustup target + NDK (r26+); NDK provides the clang linker + sysroot
  rustup target add aarch64-linux-android
  export NDK=$ANDROID_NDK_HOME AT=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin
  # 1. capture loft's generated Rust for a TRIVIAL program (no GL, just prints)
  echo 'fn main() { print("hi\n"); }' > /tmp/b0.loft
  LOFT_KEEP_NATIVE_RS=1 loft --native /tmp/b0.loft   # note the /tmp/loft_native_*.rs path it prints
  # 2. cross-compile that .rs as a cdylib against the NDK clang linker
  rustc --target aarch64-linux-android --crate-type cdylib \
        -C linker=$AT/aarch64-linux-android24-clang \
        --extern loft=<libloft-aarch64.rlib> /tmp/loft_native_*.rs -o /tmp/libb0.so
  # 3. the load-bearing sub-question: does libloft ITSELF cross-compile?
  cargo build --release --lib --target aarch64-linux-android   # (with .cargo/config linker set)
  ```
  **Pass/fail:** step 3 producing `libloft.rlib` for the triple + step 2 producing `libb0.so`
  = **invariant holds** (blockers are toolchain wiring → proceed to B1). A codegen error that
  is host-specific (a `#[cfg(target_os)]` the generated Rust hard-codes, a host-only intrinsic)
  = **invariant is WRONG** → the plan changes (the generated core is not target-agnostic).
  Record the FIRST blocker either way (linker, `std`/`libc` symbols, `ring`/build-script host
  assumption, panic strategy). Do NOT build B1+ before B0 answers this. **If no NDK is available
  in the working env, B0 is a spec to run on a machine that has one — it is not skippable, it is
  the gate.**
- **B1 — the target descriptor + cross `rustc`. ✅ DONE (2026-07-14).** `src/android.rs`
  (`AndroidTarget`) + `--native-android` in `main.rs`; auto cross-builds the runtime rlib
  into `target/loft/android/`. **Verified:** a headless struct/fn/for-loop program links to a
  bionic AArch64 `.so` (`tests/android_target.rs`); interpreter/`--native`/`--native-android`
  agree on the same source. Host-tested with an NDK; no device/emulator yet.
- **B2 — runtime entry + APK packaging. ✅ DONE (2026-07-14).** `--native-android` emits a
  real `NativeActivity` (`android_main`, §1b) AND — with an `.apk` output — packages it into a
  signed, installable APK inside loft (SDK discovery → `aapt2`/`jar`/`zipalign`/`apksigner`,
  §1b). The hand-written spike wrapper + `build_apk.sh` are now generated/done by loft itself.
  Proven end to end — `loft --native-android app.apk` output installs + launches on a KVM
  emulator; logcat shows `loft --native-android: android_main reached` +
  `sum of squares 0..8 = 140`. The [`b2-spike/`](b2-spike/README.md) `run_emulator_test.sh`
  (now package-auto-detecting) remains the install/launch/logcat harness.
- **B3 — `lib/graphics` EGL/ANativeWindow backend. ✅ DONE (2026-07-14).** A real, unchanged
  loft GL program (`use graphics; gl_create_window; while gl_poll_events { gl_clear(rgb(255,
  128,0)); gl_swap_buffers }`) renders on the KVM emulator via `loft --native-android app.apk`
  — `screencap` golden = center pixel (255,128,0), **99.6 % `#ff8000`**. The port
  (`tests/fixtures/libs/graphics/native/`) is a small cfg-gated `android_gl.rs`: EGL/GLES-3.0
  on `app.native_window()`, `gl::load_with(eglGetProcAddress)` so all 45 `gl::` draw fns work
  unchanged, android-activity poll for `loft_gl_poll_events`; only ~5 glutin-specific sites
  changed, the desktop path untouched. The AndroidApp seam is a unified-rlib global
  (`loft_gl_android_set_app`, option B — the OS entry must live in loft's `.so`, so the entry
  hands the app over). Three integration fixes landed with it: native package as a unified
  rlib dep + `loft-ffi` patch (`src/android.rs`); `libc++_shared.so` bundled + linked (rodio's
  oboe is C++); and the generated `main` runs inline on android (not the big-stack thread) so
  the graphics pump is on the `android_main` ALooper thread (`src/generation/mod.rs`,
  target-gated). Since the loft website is WebGL2 = GLES 3.0, any website GL program is already
  GLES-safe → runs on Android unchanged. **Remaining polish:** text/shaders/textures goldens
  on-device (the `gl::` code is shared, so expected to work — golden them incrementally);
  audio-on-android (oboe links now; not yet exercised); a big android_main stack for
  deep-recursion programs.
- **B4 — touch + IME input.** winit `Touch`/gesture + char/IME → `lib/graphics`. **Verify:**
  tap-to-select, drag-scroll, pinch-zoom, and soft-keyboard password entry drive ssh_home.

## 5. Failure paths to probe (design-protocol — enumerate how it breaks)

- **`std` for `aarch64-linux-android`** — does the generated Rust's `std` usage (threads,
  files, net via the FFI crate) link against the NDK `libc`/`liblog`? (B0.)
- **The SSH FFI crate (`russh`) cross-compiles** — `ring`/crypto often need per-target asm;
  its build-script must target the NDK, not the host. (Mirrors the known consumer-native
  cdylib pain in [[zt-consume-native-loft-libs]].)
- **GLES subset** — desktop GL calls absent in GLES-3.0 must be avoided/emulated in the
  graphics lib; the golden must render identically or the tolerance must cover it.
- **`android-activity` lifecycle** — `onSaveInstanceState`/pause-resume vs loft's single
  `gl_poll_events` loop; the surface can be destroyed under the app (must recreate context).
- **NDK availability in CI** — the ASan/native CI images likely lack an NDK; the Android
  build is a *separate, opt-in* CI leg (like the daily Windows one), never a required check.

## 6. Non-goals (v1)

Public-key auth, IME beyond ASCII+password, GameActivity (NativeActivity/`android-activity`
is enough), Play-store packaging. Android is a *re-target of one source*, so anything that
would fork the loft program between Linux and Android is out of scope by construction.
