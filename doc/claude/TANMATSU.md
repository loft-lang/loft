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

An app is an ESP-IDF project built for the `esp32p4`, installed into `appfs` with
**badgelink** (`badgeteam/esp32-component-badgelink`, its `tools/`).  The launcher writes the
app there and lists it in its menu; the launcher itself is never touched.  **`idf.py flash` is
the command to avoid**: it writes the app over the launcher.

badgelink talks to the launcher over USB in **USB device mode**, not over the serial port:

1. Once per machine, the udev rule that lets it open the device without root:
   `sudo cp tools/60-badgelink.rules /etc/udev/rules.d/ && sudo udevadm control --reload-rules
   && sudo udevadm trigger`.
2. On the launcher's home screen, press the **purple diamond key** (second from the top right)
   **once**.  The icon at the top right turns from a bug into a USB symbol, and the P4 shows up
   as `16d0:0f9a` instead of its serial port.  A second press switches back to debugging mode.

```sh
badgelink.sh appfs list                                     # what is installed, read-only
badgelink.sh appfs usage                                    # how full appfs is
badgelink.sh appfs upload <slug> "<title>" <version> application.bin
badgelink.sh appfs crc32 <slug>                             # compare with the local file's CRC-32
badgelink.sh start <slug>
```

A started app boots with the port back in debugging mode, so its console is the P4's serial
port again while it runs.

## Reading an app's log

An app's console is the P4's USB serial port, and that port goes away and comes back on every
restart into an app.  `scripts/tanmatsu_log.py` follows both chips' ports by their serial
numbers, opens each again as soon as it is back, and appends every line, with the PC's time in
front, to `~/.cache/loft/tanmatsu/<serial>.log` (the serial is the chip's MAC):

```sh
scripts/tanmatsu_log.py                  # follow the ports until Ctrl-C
scripts/tanmatsu_log.py --install        # the same as a systemd user service, from now on
scripts/tanmatsu_log.py --tail           # the last lines of each chip's log
```

**ModemManager** opens every new port that looks like a modem and reads from it, taking lines
before the logger has them.  Keep it away from Espressif's ports once per machine:
`echo 'ATTRS{idVendor}=="303a", ATTRS{idProduct}=="1001", ENV{ID_MM_DEVICE_IGNORE}="1"' |
sudo tee /etc/udev/rules.d/61-esp-usb-jtag.rules && sudo udevadm control --reload-rules &&
sudo udevadm trigger`.

A loft app holds its start-up lines (`@boot`) until the program starts, so a reader that opens
the port while the board starts still gets them.

## A loft program as an app

`loft --native-esp <dir> prog.loft` writes an ESP-IDF project into `<dir>` and builds
`<dir>/build/application.bin`, the file badgelink uploads.  It needs ESP-IDF (the version
`src/esp_app.rs` names, installed for `esp32p4`, found through `IDF_PATH` or under
`~/.espressif`).

- **No Rust.**  The project holds the C file the C backend writes (the program and the runtime)
  and the `[c]` parts of the libraries the program uses.  A C build loads no library's Rust part
  ([HARDWARE_BACKENDS.md § The processor](HARDWARE_BACKENDS.md#the-processor)).
- **Assets.**  `--assets a,b` names the files to pack, relative to the program (a directory is
  packed whole; the default is `assets`).  The app serves them read-only under the same
  relative paths, so `file("assets/x.png")` finds on the device what it found on the PC.
- **The board.**  `app_main` starts NVS and the BSP (display in RGB565, keys, audio), mounts the
  assets, runs `main` and goes back to the launcher when it returns.
- **Timing.**  Each start-up step prints `@boot <µs since boot> <step>` on the console, and
  `@loft exit <n>` when the program ends.
- **Built for speed and size.**  The shipped tier (no call chain; `--native-debug` keeps it),
  `-O2`, a 1 kHz scheduler tick, a 64 KB main stack, no task watchdog and no radio.  ESP-IDF
  compiles with `-ffunction-sections -fdata-sections` and links with `--gc-sections`, so only
  the runtime functions the program reaches stay in the binary.

`tile-world-game/sheet.loft` is the worked example: Kenney's Tiny Dungeon sheet on the screen
until space is pressed, every step logged as `@t <µs> <event>`.  Its libraries' C parts are
`tiles.c`, `imaging_png.c`, `screen.c` and `input`'s `keys.c`, and with
`--assets assets/sheets/tiny-dungeon.png` the app is about 520 KB.

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

**On the device.**  `tile-world-game/sheet.loft`, built with `--native-esp` and installed
with badgelink, runs: the asset filesystem, `screen`'s display section, `input`'s key polling,
the console timing lines and the return to the launcher.  The steps that cost the most on that
run are the open work below.

**Open:**

- The BSP's start-up takes about 1.6 s of the app's launch; which part, and whether an app can
  skip what it does not use, is not measured yet.
- `screen`'s Tanmatsu section rotates and scales the frame one pixel at a time: showing one
  800x480 frame takes about 63 ms, which caps a game near 15 frames a second.
- The panel shows the restart's colours between an app's end and the launcher's first frame.
- The I2S output task (`audio`) has not run on the device.
- The screen's physical size, for a game that draws to scale: a loft call with a defined
  "unknown" answer, filled on the Tanmatsu from the panel's specification until the BSP
  reports it.
