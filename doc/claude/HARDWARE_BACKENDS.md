<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Adding a hardware platform

A hardware platform is a device a loft program is built for as C: a handheld, a board, a
console.  This page is the method, with the Tanmatsu ([TANMATSU.md](TANMATSU.md)) as the
worked example.  Two layers take part:

1. **The processor:** loft's C backend, the `#width 32` regime when the core is 32-bit, and an
   emulator that runs the C.
2. **The device's parts** (sound, keys, display, random numbers): one section per platform in
   a library's C part, behind the library's one loft API.

## The rule: one API, platform sections only in C

- **The loft API is the same everywhere.**  The loft side of a library has no platform
  conditions: no `#if` in a `.loft` file, no per-platform function names.  A program written
  on a PC builds for the device unchanged.
- **Only the C part branches**, on the compiler's own platform macro (`ESP_PLATFORM`, `_WIN32`,
  `__wasm__`, ...).  Every section defines the same bound functions with the same meaning.
- **A missing part has a defined answer.**  A platform without a part answers the documented
  "none": `audio::start()` is false, `input::device_open()` is false,
  `random::seed_hw()` is false.  A program can always ask and go on.  A call that cannot be
  made on a target at all (one that needs files on wasm) is refused when the program is
  built, never at run time.
- **A device's own revisions are its SDK's.**  When batches of one device differ in a part (a
  panel with its own start-up code), the board support package tells them apart and sets the
  part up; a section calls the SDK and reads what it reports (a panel's size and format), and
  never names a revision itself.
- **The answers are the same.**  Computation shared by every platform lives outside the
  sections, so it is the same code everywhere.  `audio`'s mix is one function that every
  output layer calls; `input`'s terminal parser serves every platform's `device_feed`.

### The pattern

`audio/c/audio_out.c` and `input/c/keys.c` are the models:

```c
/* the bound functions, declared once */
int64_t ae_out_open(int64_t mode);
void ae_out_close(void);

#if defined(ESP_PLATFORM)
/* the device's SDK: a FreeRTOS task writing to the BSP's I2S channel */
#elif defined(__wasm__)
int64_t ae_out_open(int64_t mode) { (void) mode; return 0; }   /* none: a defined answer */
void ae_out_close(void) { }
#else
/* the desktop: miniaudio */
#endif

/* shared by every section */
int64_t ae_out_running(void) { return ae_out_on; }
```

- **Threads.**  A section that runs on its own thread (an audio callback, a device task)
  shares state with loft's calls.  Put one lock around every bound function and the same lock
  around the thread's work.  Make the lock a macro each section defines (`AE_LOCK()`).  The
  bound functions call unlocked bodies (`u_ae_*`), so one bound function calling another never
  takes the lock twice.
- **Tests on a machine without the part.**  Give the section a mode that runs the real code
  path into nothing:
  - `audio::start(true)` opens miniaudio's null device, which takes frames at real-time pace,
    so a test hears a clip end through the output thread alone;
  - `input::device_feed(bytes)` hands the terminal parser bytes as if they were typed.

## The processor

- **32-bit.**  If the core is 32-bit, write the program and its libraries under `#width 32`
  ([WIDTH32.md](WIDTH32.md)).  A 32-bit program answers the same on every backend, so the
  interpreter on a PC is the oracle for the device.
- **An emulator leg.**  Add the core to `scripts/c_sweep.py`'s `CROSS` table: a compiler for
  the target and an emulator to run it.  `--riscv32` is
  `zig cc -target riscv32-linux-musl` and `qemu-riscv32`.  A leg with no emulator can still
  compile (`-c`), which catches type and size mistakes.
- **No Rust.**  A C build loads no library's Rust crate (`native_build::c_target`).  A library
  that the device needs therefore has its device code in a `[c] shim`, never in `#native`
  Rust.

## Checking a platform section

The device's SDK is often not installed where the tests run.  Type-check the section against
the SDK's real headers anyway.

1. **Get the headers.**  Download the board support package (for the Tanmatsu:
   `badgeteam/badge-bsp` from the ESP component registry) into its own directory.
2. **Stand in for the rest.**  Write small headers for what the BSP includes from the SDK
   (`freertos/FreeRTOS.h`, `esp_err.h`, `driver/i2s_std.h`, ...).  Each declares only the
   types and functions the section uses, with the SDK's real signatures.
3. **Compile with the platform's macro.**  For example:

   ```sh
   gcc -m32 -DESP_PLATFORM -I<stand-ins> -I<bsp> -Wall -Wextra -fsyntax-only keys.c
   ```

   `-m32` gives the device's integer sizes.

This catches a wrong function name, argument order or type, and a missing include.  It
cannot catch a mistake in what the device does at run time.  That needs the device, or its
emulator: for the Tanmatsu, `virtual-tanmatsu-graceloader`.

## Checklist for a new platform

1. **The processor:** an emulator leg in `c_sweep.py`; every library the device needs passes
   its tests on it.
2. **For each part the device has** (output, input, entropy, display): a section in the
   library that owns that part, a check against the SDK headers, and a test that runs the
   shared code path without the device.
3. **For each part it lacks:** the defined "none" answer, and a test that asks for it.
4. **A page for the device**, like [TANMATSU.md](TANMATSU.md): the hardware as loft sees it,
   what runs there, what was verified and how, and what has not run on the device yet.
