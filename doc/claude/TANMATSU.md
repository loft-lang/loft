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

## Getting a Tanmatsu running

Everything in this section reads from the device and writes nothing to it.  The one step
that changes the device, installing an app, is in [Apps](#apps).

**Connect it.**  Use a USB-C cable that carries data and switch the device on.  It shows up
as two Espressif "USB JTAG/serial debug unit" ports (USB ID `303a:1001`) behind a small hub
inside the device:

| Chip | What it is | Flash |
|---|---|---|
| ESP32-P4 | the main processor: runs the launcher and every app | 16 MB |
| ESP32-C6 | the radio: Wi-Fi 6, Bluetooth 5 LE, 802.15.4 | 8 MB |

Both ports are the chips' own USB peripheral, built into the silicon, so they answer
whatever firmware is running.  Which one is `/dev/ttyACM0` is not fixed; use the stable
names under `/dev/serial/by-id/`, or ask with `esptool chip-id` (below).

**Permission.**  The ports belong to the `dialout` group:
`sudo usermod -aG dialout $USER`, then log in again.  Until then, `sg dialout -c "<cmd>"`
runs one command with the group.

**esptool.**  Version 5 or newer, which knows the P4's current revisions:
`sudo apt install pipx && pipx install esptool` (`sudo pipx install --global esptool` for
every user).  A distribution's own `esptool` package can be too old for the P4.

**Ask the chips** — each of these only reads.  Each restarts the chip into its ROM
bootloader and back, so the device reboots:

```sh
esptool --port /dev/ttyACM0 flash-id            # chip, revision, flash size
esptool --port /dev/ttyACM0 get-security-info   # secure boot, flash encryption
```

With secure boot and flash encryption both off, a backup is a plain copy of the flash, and
writing it back restores the device exactly.  Never run `write-flash`, `erase-flash` or any
`espefuse` command while exploring: an eFuse write can never be undone.

**Back up both chips before anything else:**

```sh
esptool --port <P4 port> read-flash 0 ALL p4.bin && esptool --port <P4 port> verify-flash 0 p4.bin
```

The C6 needs the P4 held still.  While the launcher runs, it restarts the radio when the
radio goes quiet, which drops the C6's USB port a few seconds into a long read.  Park the P4
in its ROM bootloader first, read the C6, then restart the P4:

```sh
esptool --port <P4 port> --after no-reset chip-id     # the P4 stays in its bootloader
esptool --port <C6 port> read-flash 0 ALL c6.bin && esptool --port <C6 port> verify-flash 0 c6.bin
esptool --port <P4 port> chip-id                      # restarts the P4 normally
```

Keep the images and their `sha256sum` off the machine.  Restoring one is
`esptool --port <port> write-flash 0 <image>`.

**The flash layout** — the partition table at `0x8000`, as the launcher ships it (and as
the app template's `partition_tables/esp32p4-16mb.csv` declares it):

| P4 partition | Offset | Size | Holds |
|---|---|---|---|
| `nvs`, `otadata`, `phy_init` | `0x19000` | 28 KB | settings, which launcher slot boots, radio calibration |
| `ota_0`, `ota_1` | `0x20000`, `0x220000` | 2 MB each | the launcher, two slots so an update can switch |
| `appfs` | `0x420000` | 8 MB | the installed apps |
| `locfd` | `0xC20000` | 3.9 MB | a FAT file area |

The C6 holds its own firmware (`factory`, 2 MB), an `appfs`, Zigbee storage and a 1.9 MB
SPIFFS area.

## Apps

An app is an ESP-IDF project built for the `esp32p4`.  The Tanmatsu app template
(`Nicolai-Electronics/tanmatsu-template`) names the ESP-IDF version and the BSP version an
app uses.  Its `make install` uploads `application.bin` with **badgelink**, which talks over
USB to the running launcher:

```sh
badgelink.sh appfs upload application "<title>" 0 application.bin
badgelink.sh start application
```

The launcher writes the app into `appfs` and lists it in its menu, beside the launcher, which
is never touched.  **`idf.py flash` is the command to avoid**: it writes the app over the
launcher itself.

## A loft program as an app

A loft program goes onto the device as C.  The C backend writes the program and the runtime
as one C file.  A C build loads no library's Rust part ([HARDWARE_BACKENDS.md § The
processor](HARDWARE_BACKENDS.md#the-processor)), so an app holds only that file and the
libraries' `[c]` parts.  `tile-world-game/sheet.loft`, Kenney's Tiny Dungeon sheet on the
screen until space is pressed, needs four of them: `tiles.c`, `imaging_png.c`, `screen.c`
and `input`'s `keys.c`.  It logs each step as `@t <microseconds> <event>` on the console,
which is the USB serial port on the device.

What is built so far:

- The program runs on the interpreter and through the C backend on a PC, with the same
  answers.
- The ESP-IDF project around it is not built yet ([Not yet run on a device](#verified-and-how)).
  That project is the C file plus the four C parts, an `app_main` that starts the BSP and runs
  the program, and the template's partition table.
- Building it needs ESP-IDF installed.  The template's `make prepare` installs it into the
  project, and `make eim-sdk` installs it through Espressif's installation manager.

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
