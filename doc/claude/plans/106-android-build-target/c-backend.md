<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Android on the C backend — the design

A child of [README.md](README.md) (@PLN106, the Android target) and the piece of
[@PLN203](../203-c99-default-backend.md) C6 still owed: `loft --native-android` built from the C
unit by the NDK's clang, with no rustc, no cargo and no Rust crate in its path.  It is the last
thing that holds the rustc writer in the tree (`91-self-host/main-switch.md`, "delete at the
Android flip").

**The invariant is the one @PLN106 held:** a build target is a DESCRIPTOR over one
target-agnostic unit.  The C unit `loft::c::emit_program` writes is compiled for Android
unchanged; everything Android-specific lives in one runtime header, one driver function and one
section of each library shim that touches the OS.

## What is measured, before any design

| fact | how it was measured |
|---|---|
| The emitted C unit compiles for Android **unchanged**: aarch64 at API 24 and API 21, and armv7 (32-bit) at API 24, zero errors; the `.so` needs only `libc`, `libm`, `libdl`. | a struct + vector + hash program's unit (1.45 MB, one file), `aarch64-linux-android24-clang -std=c99 -O1 -fPIC -shared -Dmain=loft_program_main`, NDK r27c |
| The C runtime's system headers all exist in bionic; it calls none of the libc functions Android gates behind an API level (`posix_spawn`, `getrandom`, `glob`, `aligned_alloc`, …). | the `#include` and call census of `src/c/*.h` |
| Every library that reaches the OS is already a C shim: `graphics` (`loft-libs-graphics-native/graphics/c/graphics_gl.c`), `audio` (`loft-libs-game/audio/c/audio_engine.c` + `audio_out.c`), `imaging`, `tiles`.  No Rust crate is left in a published library's native path. | their `loft.toml` `[c] shim` lines |
| A `[c] shim` is already compiled FROM SOURCE for a non-host target (`c_driver::build_wasm`, `shims_as_sources`). | `src/c_driver.rs` |
| Today's Android graphics, input and audio run only through a TEST FIXTURE (`tests/fixtures/libs/graphics/native/src/android_gl.rs`, Rust), on no branch of the published library: an installed `graphics` has never reached Android. | NATIVE.md § Android target |
| The shim's platform switch picks X11 on Android: `#if defined(__linux__) && !defined(__wasm__)`, and bionic defines `__linux__`. | `graphics_gl.c` line 97 |
| HTTPS in the C runtime is vendored BearSSL (`src/c/tls/loft_tls.c`), plain C built into an archive by `src/c/tls.rs`. | `src/c/loft_https.h` |
| This box can run the whole acceptance loop: NDK r27c, the SDK emulator, an x86_64 system image, KVM. | `~/Android/Sdk`, `/dev/kvm` |

So the port is not a code generator question.  It is four small pieces of C and one driver
path, and the packaging `src/android.rs` already does (aapt2, zipalign, apksigner) carries over
untouched.

## The build, end to end

```
prog.loft ──emit_program──▶ unit.c ─┐
                                    ├─ NDK clang  --target=<triple><api>  -fPIC -shared
src/c/android/loft_android_main.c  │   -DLOFT_ANDROID -std=c99, the host tier's flags
   (entry, logcat, files, the app) ─┤
<NDK>/sources/android/native_app_glue (compiled with -Dpthread_create=…: an 8 MiB app thread)
each [c] shim, compiled from source ┘   -llog -landroid -lEGL -lGLESv3 -lm -ldl
                                    │
                         libloft_android_app.so ──package_apk (unchanged)──▶ app.apk
```

- **The driver** (`src/c_driver.rs`): `build_android(c, out, shim_sources, ship, target)`, the
  sibling of `build_wasm`.  The toolchain comes from the `AndroidTarget` descriptor, which keeps
  every Android path: `detect()` and `linker()` already name the NDK clang wrapper for
  `<triple><api>`, and it serves as the C compiler unchanged.  The flags are the host build's
  tier plus `-fPIC -shared -DLOFT_ANDROID`.  No `-lc++_shared`: no C++ is left once oboe is gone.
- **The entry**: the unit's `int main(int argc, char **argv)` is renamed by the unit's own
  compile (`-Dmain=loft_program_main`, that object only), so the C writer changes nothing.  The
  entry's `android_main` calls it.
- **Packaging**: `AndroidTarget::package_apk` takes any `.so` and stays as it is.  The manifest,
  the keystore and the ABI directory are backend-agnostic.

## The four pieces of C

### 1. The runtime entry — `src/c/android/loft_android_main.c`

A C file of its own, compiled beside the unit (the driver writes it out of the binary).  It
owns the OS entry, the way `android-activity` owns it for the rustc path:

- `void android_main(struct android_app *app)`: keep `app` in a runtime global; move stdout
  and stderr onto two pipes whose readers write each line to logcat (tags `loft-stdout`,
  `loft-stderr`: the Rust tail's `__loft_android_redirect_stdio_to_logcat`, in C); prepare the
  files (§ 3); call `loft_program_main(1, argv)`; then drain lifecycle events until
  `APP_CMD_DESTROY`, so the process does not exit under the activity.  A program that stops
  the run (`exit`, a failed assertion) does the same: the runtime's `LOFT_EXIT` reaches the
  entry's `loft_android_exit`, which records the status and returns to `android_main`.  Ending
  the process instead reads to the OS as a crash, and it starts the app again.
- **logcat is not the output of record.**  A 5,000-line burst kept 3,603 lines under the
  default ring buffer and 4,646 under a 32 MiB one: the device's log socket drops writes under
  a burst.  So a build that is not `--ship` (`LOFT_ANDROID_STDOUT_COPY`) also keeps the whole
  stdout in `loft-stdout.txt` and the exit code in `loft-exit.txt`, in the app's files
  directory, and is debuggable, so `adb exec-out run-as <package> cat files/…` reads them: the
  same burst then reads byte for byte as the interpreter's.  A `--ship` build writes neither
  and is not debuggable.
- `struct android_app *loft_android_app(void)`: how a library shim reaches the window, the
  input queue and the activity.  This is the C form of the Rust `loft_gl_android_set_app`
  seam, turned around: the shim ASKS the runtime, so no library names the runtime's internals.
- `loft_program_directory()`: on Android `/proc/self/exe` is the VM launcher (`app_process64`),
  so the `__linux__` branch answers the app's files directory there instead.

### 2. The glue — the NDK's `android_native_app_glue.c`, compiled from the NDK

The NDK's `native_app_glue` (Apache-2.0, ~450 lines, in every NDK since r5) turns
`ANativeActivity_onCreate`'s callbacks into an event queue and runs `android_main` on a thread
of its own, holding the `ALooper`.  The graphics shim's poll must run on that thread: the
rustc path learned this as "`__run` runs INLINE on the `android_main` ALooper thread".

That thread starts with bionic's default stack of about 1 MiB.  A C-backend program on the
desktop has 8 MiB: the main thread's usual limit, and `LOFT_PAR_STACK` for a `par` worker.
**The glue is compiled from the NDK unchanged, with `-Dpthread_create=loft_android_glue_thread`
on that one object**: the entry's function sets 8 MiB on the attributes the glue built, then
calls the real `pthread_create`.  No copy of the glue is kept, so there is nothing to keep
current with the NDK.

Measured (A0): a recursion every frame of which stays live (each argument is read after the
call, so clang cannot turn it into a loop) needs more than 1 MiB and less than 2 MiB at 9,000
frames on x86_64.  It fails on the default stack and passes on 8 MiB.

Two alternatives are rejected:
- running `loft_program_main` on a second thread with a large stack: that moves the program off
  the looper's thread, and the graphics poll then waits on a looper it does not own;
- the Rust writer's 512 MiB (`NATIVE_MAIN_STACK`): the C runtime's own frames never needed it,
  and a reservation that large can fail on a 32-bit device, leaving the app thread unstarted.

### 3. Files — the program sees a normal directory

A loft program reads and writes paths, and a game loads its fonts and images by name.  The
entry makes that work unchanged:

- `chdir(app->activity->internalDataPath)` before `loft_program_main`, so a relative path lands
  in the app's own writable directory.
- The program's data files are packaged under the APK's `assets/` and extracted into that
  directory on the first launch after an install or an update.  A stamp file holds the APK's
  version code, so a later launch extracts nothing.  The reads go through `AAssetManager`
  (`app->activity->assetManager`), the only reader of a compressed APK entry.
- Which files are packaged is the one rule that needs the owner's decision (below).

### 4. The library shims — one `__ANDROID__` section each

- **graphics** (`graphics_gl.c`), a new `GG_ANDROID` back end beside `GG_X11`, `GG_COCOA` and
  `GG_BROWSER`, ported from the fixture's 427-line `android_gl.rs`:
  - EGL on `loft_android_app()->window`: an ES 3 context, `EGL_OPENGL_ES2_BIT` configs, a
    window surface.  GLES 3.0 is WebGL2, so the browser back end's shader rules apply as they
    are.
  - The surface follows the lifecycle: `APP_CMD_TERM_WINDOW` destroys it and
    `APP_CMD_INIT_WINDOW` makes it again, with the same context.  A frame with no window
    draws nothing.
  - Input from `app->onInputEvent`: a `MotionEvent` feeds `gl_mouse_*`, a `KeyEvent` feeds
    `gl_key_pressed`; `gl_show_keyboard()` is `ANativeActivity_showSoftInput`.
  - The platform switch gains `&& !defined(__ANDROID__)` on the X11 line.  Without it, Android
    compiles the X11 back end and fails at its first `dlopen("libX11.so.6")`.
- **audio** (`audio_out.c`): the desktop section is miniaudio, which supports Android itself
  (AAudio loaded at run time, OpenSL ES as the fallback).  Expected work: the build compiles it,
  and the `b4_audio` golden decides.
- **imaging, tiles** and every other pure-C shim compile unchanged; the driver's existing
  "a shim that does not compile for this target" message names any exception.
- **TLS**: `src/c/tls.rs` builds BearSSL for the triple, as it builds it for the host.  The
  trust store on Android is `/system/etc/security/cacerts/` (one PEM per file); the loader
  reads that directory under `__ANDROID__`.

## Selecting the path, and retiring the old one

- While both exist, `--native-android` builds through the C path whenever the C backend is
  selected (`c_backend_selected()`, which the C9 flip makes the default).
  `LOFT_NATIVE_BACKEND=rust` keeps the rustc path, a bisect switch with its one home in
  NATIVE_SWITCHES.md.
- At the flip, as main-switch.md orders it, the rustc path goes in one commit: the
  `android-activity` / `oboe` / `android_logger` dependencies, the generated cargo crate, the
  fixture's `android_gl.rs`, and the rustc writer itself.  NATIVE.md § Android target is
  rewritten for the C path, and its "fixture-only" warning goes with the fixture.

## How it is verified

The two oracles the project already trusts: the interpreter's answer, and @PLN106's goldens.

1. **A build test, on the nightly** (the first shard of the emulator job): `tests/android_target.rs`
   gains the C path.  Emit, NDK-compile for `aarch64` and `x86_64`, check the ELF header, check
   that `ANativeActivity_onCreate` and `android_main` are exported, and check the `NEEDED` list
   holds only system libraries.  GitHub's ubuntu image ships an NDK; the test stays opt-in
   (`LOFT_ANDROID_BUILD_TEST=1`) as it is now, and the job sets it.
2. **A differential run in the emulator, on the nightly (advisory)**: a `--android` mode of
   `scripts/c_sweep.py`.  It builds each headless corpus program as an APK, starts it on an
   x86_64 emulator (`adb install`, `am start`), reads the `loft-stdout.txt` and
   `loft-exit.txt` the entry kept (logcat drops lines) and compares them with
   `loft --interpret`'s output and status.  The programs that do not match yet are
   `scripts/c_sweep_android.baseline`, a list that only shrinks.  On GitHub: the ubuntu runner with KVM and
   the community emulator action; locally, this box.  Its wall time is measured before the job
   is wired, and a long run is sharded, never given a larger limit.
3. **The goldens, rebased onto the published libraries**: `b3_rects`, `b3_text`, `b3_render`
   (graphics), `b4_touch`, `b4_ime` (input), `b4_audio`, `b4_stack`.  Each is an emulator
   screenshot (`adb exec-out screencap`) of the same loft program, compared within a pixel
   tolerance fixed at A2 from the emulator's own run-to-run noise.  They now prove the
   published package, not a fixture.

## The phases, each with a gate

| phase | what | gate |
|---|---|---|
| **A0** | `build_android` in the driver; `src/c/android/loft_android_main.c` with the entry, logcat and the renamed `main`; the NDK's glue with the 8 MiB app thread | a headless program's APK prints, in logcat, exactly what `loft --interpret` prints; a recursion that needs more than the default 1 MiB stack passes, and fails without the change |
| **A1** | the build test (verification 1) and the emulator differential (verification 2) over the headless corpus | every headless corpus program agrees, 0 differ; the differ list is a committed baseline that only shrinks (main-switch.md phase 0's rule) |
| **A2** | `GG_ANDROID` in `graphics_gl.c`; the `__ANDROID__` guard on X11; audio through miniaudio | `b3_*`, `b4_touch`, `b4_ime` and `b4_audio` pass on the PUBLISHED `graphics` and `audio`; each library republished through its own CI (the `loft-library` skill) |
| **A3** | files: `internalDataPath`, assets packaged and extracted, the program directory | a program that loads a font and an image from its package draws `b3_text` with no path changed |
| **A4** | TLS for the triple and the Android trust store | an `https://` fetch answers the same in the emulator as on the desktop |
| **A5** | the flip: `--native-android` on the C path by default; the rustc Android path deleted with the rustc writer | one release cycle in which the Android build test and the nightly emulator run used only the C path |

A0 and A1 need no library change and prove the claim the rest stands on.  A2 is the largest
piece (the graphics back end), and the only one that changes a published library.

## Decisions (owner: the recommendations, taken)

1. **Which files go into the APK.**  Decided: a package's `[android] assets = [...]`
   list in `loft.toml` (directories or globs relative to the package), defaulting to the
   package's `assets/` directory when it has one, with each `use`d library's own declared
   assets added.  Nothing is guessed from string literals in the program.
2. **Whether armv7 (32-bit) ships.**  It compiles (measured), and loft's store is 32-bit
   already.  Decided: build it on request (`LOFT_ANDROID_TARGET`) but test only
   `aarch64` and the `x86_64` emulator twin; arm64 is every Android device sold since 2019.
3. **The minimum API.**  API 21 compiles (measured); 24 is today's default.  Decided: keep 24, since nothing measured needs a lower one.
4. **Who builds it.**  Decided: the gathering stream (loft3).  The work is the
   driver, one runtime header and two library shims, and the libraries are this stream's.
   The C writer's own change is the one `#define`.

## Open, to measure in its phase

- IME *composing* text: NativeActivity carries `KeyEvent`s only (@PLN106's known follow-up).
  A `gl_text_input()` stream needs a Java-side `InputConnection`, which a code-less APK does
  not have.  Out of scope here; recorded so it is not mistaken for a regression.
- `par` threads on a phone: the pthread pool is unchanged, but the core count and thermal
  limits differ.  A2's goldens run single-threaded first.
- APK size: the unit plus every shim, one ABI.  Measured at A0 and reported beside the rustc
  path's size.
