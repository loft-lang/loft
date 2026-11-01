<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft on the Tanmatsu

The Tanmatsu is a handheld computer with a keyboard, an 800x480 display and a speaker.
loft runs there as C: the program is written out by the C backend
([C_BACKEND.md](C_BACKEND.md)) and built into an ESP-IDF app.  This page is about this one
device.  How to add another device is in [HARDWARE_BACKENDS.md](HARDWARE_BACKENDS.md).

## The hardware, as loft sees it

| Part | What it is | What it means for a loft program |
|---|---|---|
| Processor | ESP32-P4: two 32-bit RISC-V cores (RV32IMAFC), single-precision FPU only | Write under `#width 32` ([WIDTH32.md](WIDTH32.md)): a 64-bit operation is several instructions, a `float` (double) is software |
| Memory | 768 KB of fast SRAM; 32 MB of PSRAM, slower | Compose a frame at 1x in SRAM and scale it up once (`tiles`) |
| Display | A MIPI-DSI panel, 800x480 on the current boards, mounted in portrait (480x800), framebuffer in PSRAM; the BSP sets it up and reports its size, colour format and rotation | `tiles::present(s, w, h, scale, rotate: true)` writes the rotated, scaled frame, sized from what the BSP reports |
| Audio | I2S, 16-bit stereo, into an ES8156 codec; one stream, no mixer in hardware | `audio` mixes every voice into that one stream |
| Keys | A full keyboard, read as PC scancodes and navigation keys | `input::input_tick_device()` |
| Random numbers | A hardware random source (`esp_fill_random`) | `random::seed_hw()` |

The board support package is `badgeteam/badge-bsp`, from the ESP component registry
(`bsp/audio.h`, `bsp/input.h`, `bsp/display.h`).  The Tanmatsu app template
(`Nicolai-Electronics/tanmatsu-template`) names the version an app uses.

**The BSP owns the differences between hardware batches.**  Boards from different batches can
carry a different part that needs its own start-up code, a display panel for example.  Nicolai
Electronics keeps the BSP working on every batch, so loft never sets up a part itself and never
asks which batch it runs on.  What follows from that:

- **Display code reads the panel from the BSP, never from constants.**  Width, height and
  colour format come from `bsp_display_get_parameters`, the rotation from
  `bsp_display_get_default_rotation`.  A new batch then needs a newer BSP and no loft change.
- **A game is told the screen's dimensions, not shielded from them.**  The graphics library
  passes the pixel size on (`Window::width()` / `height()` elsewhere), and `drawing` scales its
  fractions-of-the-paper scenes to whatever `size` it is given.  The PHYSICAL size, which a game
  needs to draw something the same size on every screen, is not available yet: the BSP reports
  pixels only, and no platform of the graphics library reports millimetres or density.

## What runs there

**The language.**
- `#width 32` makes a file's `integer` an `i32` and its `float` a `single`, and keeps its
  arithmetic in 32 bits.
- The C backend writes such a body with `int32_t` locals.
- A library can opt in on its own.  A 64-bit caller's arguments are narrowed where they enter
  it.

All of this is in [WIDTH32.md](WIDTH32.md).

**The libraries.**  Each has a C part with a Tanmatsu section (`#if defined(ESP_PLATFORM)`)
behind the same loft API it has everywhere.

| Library | On the Tanmatsu | Elsewhere |
|---|---|---|
| `random` | `seed_hw()` seeds from `esp_fill_random`; `Rand32` is xoshiro128**, 32-bit only | `/dev/urandom`, `rand_s` |
| `imaging` | lodepng (PNG decode and encode), no platform section | the same |
| `tiles` | 8-bit tile sheets, RGB565 surfaces, `present` with scale and rotation; no platform section | the same |
| `audio` | `start()` runs a FreeRTOS task that writes the mix to the BSP's I2S channel | miniaudio: ALSA, PulseAudio, CoreAudio, WASAPI |
| `audio_bus` | pure loft over `audio` | the same |
| `input` | `input_tick_device()` reads `bsp_input_read_scancode` / `_navigation_key` per key | a terminal in raw mode |
| `tile_world` | pure loft | the same |

## Verified, and how

**Off the device.** Every check here runs on an ordinary PC:

- **The 32-bit arithmetic:** the interpreter is the oracle.  A program under `#width 32` answers
  the same on every backend and every processor.
- **The C on a RISC-V 32-bit core:** `scripts/c_sweep.py --riscv32` builds each program with
  `zig cc -target riscv32-linux-musl` and runs it under `qemu-riscv32`.  `--m32` does the same
  for 32-bit x86.  Every library above passes its tests on both.
- **The Tanmatsu sections:** each library's `ESP_PLATFORM` code is type-checked against the real
  BSP headers, with small stand-ins for the FreeRTOS and ESP-IDF headers
  ([HARDWARE_BACKENDS.md § Checking a platform section](HARDWARE_BACKENDS.md#checking-a-platform-section)).

**Not yet run on a device.**  These are open:

- A loft program built into a Tanmatsu app: the ESP-IDF component that carries loft's C, its
  runtime and the libraries' C parts.
- The I2S output task and the key polling, which only real hardware (or
  `virtual-tanmatsu-graceloader`, with the `riscv32-esp-elf` toolchain) can run.
- A display layer that hands `tiles::present` to the panel, sized from the BSP's parameters.
- The screen's physical size, for a game that draws to scale: a loft call with a defined
  "unknown" answer, filled on the Tanmatsu from the panel's specification until the BSP
  reports it.
