<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# loft on the Tanmatsu

The Tanmatsu is a handheld computer with a keyboard, an 800x480 display and a speaker.
loft runs there as C: the program is written out by the C backend
([C_BACKEND.md](C_BACKEND.md)) and built into an ESP-IDF app.  This page is about this one
device.  How to add another device is in [HARDWARE_BACKENDS.md](HARDWARE_BACKENDS.md).

Why a 32-bit device: such hardware is becoming efficient and affordable at once, so it has a
clear future in industry and in user devices, for low-cost, low-energy and retro computing
([BROADENING.md](BROADENING.md)).

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
(`bsp/audio.h`, `bsp/input.h`, `bsp/display.h`).  loft pins 2.0.0, the version the launcher
runs (`src/esp_app.rs`): it drives the boards' second display panel (SW39480800PN03-01-1,
through `mipi_dsi_abstraction` 2.0), and its own code is 1.6.4's, so the start-up steps
`loft_esp.h` runs one by one are the same.  Against 1.6.4 on ESP-IDF v6.0.3 the display starts
as fast and a 192x176 frame at scale 2 takes 30 us more (11.31 ms); the app is 1.2 KB larger.

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
- **Files, found from what names them.**  Without `--assets`, the app packs each file beside
  the program that something names: a text constant of the program (`"kinds.json"`, or the
  `/theme.json` a path built at run time leaves), a path written in a file already packed
  (`theme.json` naming its sheets), or a directory and an extension the program writes as
  constants of their own, which is what `"{dir}/levels/{name}.json"` leaves in the C
  (`/levels/` and `.json`).  What nothing names stays out.  The app serves the files read-only
  under the same relative paths, so `file("assets/x.png")` finds on the device what it found on
  the PC.  `--assets a,b` names them by hand instead: files, directories, `*` inside a name,
  `!` to take a match out again.
- **The board, found from what calls it.**  `app_main` starts NVS, then the BSP's parts one by
  one: each part the app's C calls (a library's file and what it includes), and the ones a
  program reaches without a BSP call (the clock, the SD card's supply, power, the LEDs).  The
  motion sensor, the environment sensors and the two expansion ports stay off for an app that
  calls none of them; the motion sensor alone takes more than a second to start.  The screen is
  black as soon as the display is up.  When `main` returns, the backlight goes off and the app
  restarts into the launcher.
- **The report.**  `<dir>/loft-esp-report.txt` says why each file and board part is in, what the
  image holds by origin (the program, the runtime, each library's C file, each ESP-IDF
  component), and its largest functions, read from the linker's map.  It is the place to start
  when an app is too large.
- **Timing.**  Each start-up step prints `@boot <µs since boot> <step>` on the console, held
  until the program starts, and `@loft exit <n>` when it ends.
- **Built for size where it runs once, for speed where it loops.**  The shipped tier (no call
  chain; `--native-debug` keeps it).  loft's C is built with `-Os -DLOFT_SIZE`, where the
  runtime's hot helpers are offered for inlining rather than forced; the libraries' C, where
  the pixels are drawn and the pictures decoded, keeps `-O2`.  ESP-IDF compiles with
  `-ffunction-sections -fdata-sections` and links with `--gc-sections`, so only what the program
  reaches stays in the binary; its assertions keep their checks without their messages, and it
  logs warnings only.  A 1 kHz scheduler tick, a 64 KB main stack, no task watchdog, no radio,
  and no test of all of PSRAM at every start.

`tile-world-game/sheet.loft` is the smallest example: Kenney's Tiny Dungeon sheet on the screen
until space is pressed, every step logged as `@t <µs> <event>`.  Its libraries' C parts are
`tiles.c`, `imaging_png.c`, `screen.c` and `input`'s `keys.c`, and the app is about 510 KB.
`tile-world-game/main.loft`, the game, builds whole for the P4 at about 1.8 MB, 740 KB of it its
files.

## Memory and storage for a larger game

**Where an app lives while it runs.**  An app runs from the flash it is installed in: its code
executes in place through the chip's 128 KB cache, and its packed files are read in place.
Neither is copied into RAM, so code that does not run and files that are not read cost flash,
not memory.  RAM holds what the program builds:

| Memory | Size | Holds |
|---|---|---|
| flash, `appfs` | 8 MB, shared by every installed app | the app: code and packed files |
| internal RAM | 768 KB, fast | the stack, allocations under 16 KB, loft's 64 KB arena for temporary values |
| PSRAM | 32 MB | allocations of 16 KB and more: decoded sheets and sounds, levels, loft's stores |

The two limits for a larger game are flash for what it stores and PSRAM for what it has decoded
at once.

**Where a game's data can live.**

| Place | Room | Read | Notes |
|---|---|---|---|
| inside the app | the free part of `appfs` | in place, no copy | what `--native-esp` does now; a change to any file means uploading the app again |
| a data pack as its own `appfs` entry | the same 8 MB | in place, no copy | updated without the code; needs a reader in `loft_esp.h`, which does not exist yet |
| the internal FAT area (`locfd`) | 3.9 MB | copied into RAM | writable: saved games, the level editor; `badgelink fs upload` writes it |
| an SD card | gigabytes | copied into RAM, slowest | needs a card; the same file calls as the FAT area |
| a server, over Wi-Fi | unlimited | over the network | loft serves stores over HTTP ranges ([REMOTE_STORES.md](REMOTE_STORES.md)); the radio adds size and start-up time |

loft's lazy stores ([LAZY_STORES.md](LAZY_STORES.md)) fetch what a collection is missing when it
is read, which fits a game's area held as a store image and loaded when the player enters it.
They have not run on the device.

**Code per scene.**  Code runs in place, so a scene's code that is not running takes no RAM, and
the question is only whether all of it fits in flash: `tile-world-game`'s program is about
500 KB of it.  Loading code at run time is possible: Espressif's `elf_loader` component runs a
relocatable ELF from PSRAM on the P4.  For loft that needs a scene written as a module of its own
that calls the main app's runtime, while the C backend writes one file per program, so it is
compiler work no game needs yet.  Two other ways exist: a game split into apps with a restart
between them (about half a second, the state carried in a file), and behaviour held as data, as
`tile-world-game` already holds its kinds and rules, so an area is mostly new data.

**The way to grow a game.**

1. Keep the code in the app; running in place keeps the code that is not running out of RAM.
2. Give each area its own files (its levels, sheets and sounds), loaded when the player enters
   it and freed when the player leaves, so the RAM a game needs is one area's, not the game's.
3. Read every file by a path relative to the program.  loft's file calls go through ESP-IDF's
   VFS, so the game's code is the same wherever a file is stored: in the app until the app
   nears the free part of `appfs`, then in an `appfs` data pack, the FAT area or an SD card.
4. Saved games and the editor need the FAT area, whatever holds the rest.

Not measured yet: how long entering an area takes (decoding its sheets and sounds), and how much
PSRAM one area needs.

## The radio, the network and the launcher's settings

**The radio chip.**  The ESP32-C6 runs `Nicolai-Electronics/tanmatsu-radio`, a fork of Espressif's
esp-hosted-mcu, and talks to the P4 over SDIO.  The C6 does the radio work: Wi-Fi and its WPA
handshake, the Bluetooth controller, 802.15.4, LoRa (an SX126x on the C6, reached through
`tanmatsu-lora`) and infrared transmit (`tanmatsu-infrared`, NEC codes).  TCP/IP, TLS and
WebSocket run on the P4, and so does the Bluetooth host (NimBLE).  esp-hosted's "network split", which gives the C6
a TCP/IP stack of its own, is off in the Tanmatsu build (`ESP_HOSTED_NETWORK_SPLIT_ENABLED`).  The
launcher checks the radio firmware's version by exact match and offers to replace one it does not
expect, so a radio firmware reaches every Tanmatsu only through the launcher.

**Wi-Fi is configured once, in the launcher.**  The launcher stores networks with `wifi-manager` in
the NVS namespace `wifi` (up to 255 slots: SSID, password, and identity and username for enterprise
networks).  An app shares that NVS partition (`nvs` at `0x19000`) and joins with the launcher's
sequence: `wifi_remote_initialize()` (`tanmatsu-wifi`), `wifi_connection_init_stack()`, then
`wifi_connect_try_all()`.

**The launcher's settings are the device's settings.**  `tanmatsu-settings` keeps them in the NVS
namespace `system`; an app reads them through the same component and writes none of them:

| Keys in `system` | What |
|---|---|
| `disp.brightness`, `kb.brightness`, `led.brightness`, `speaker.volume`, `hp.volume` | the owner's brightness and volume |
| `timezone`, `tz` (a POSIX rule: `setenv("TZ")` + `tzset()`), `ntp` | the clock |
| `owner.nickname`, `owner.bday.day`, `owner.bday.mon`, `owner.theme` | the owner |
| `lora.freq`, `.sf`, `.bandwidth`, `.codingrate`, `.power`, `.offset`, `.autooffset`, `.ldro` | the LoRa parameters the launcher sends to the radio |
| `repo.server`, `repo.baseuri`, `fw.patch`, `fw.welcome` | the app store and the launcher's own state |

The launcher also keeps `app_fav`, `app_usage`, `appfs`, `plugins` and one `plugin_<slug>` namespace
per launcher plugin.  The radio chip has an NVS of its own: its board revision and country, and
ESP-IDF's cached Wi-Fi state.  An app's own saved data belongs in a file in the FAT area, not in NVS.

**The network on the device.**  `web`, `server` and `ssh` do not run on the Tanmatsu yet.  The C
runtime compiles its sockets out under `ESP_PLATFORM` (`loft_cc.h` defines no `LOFT_POSIX_HOST`
there), and their natives are bound to the build machine's `libloft_net.so`, so a call stops the
program at run time.  The socket layer itself builds against ESP-IDF's lwIP with two changes:
`<sys/poll.h>` for `<poll.h>`, and a stand-in for `gai_strerror`.  The way there, Wi-Fi validation
included: [@PLN213](https://github.com/loft-lang/plans/issues/213).

## Processors

| Processor | What runs there |
|---|---|
| P4, two fast cores | the loft program, its libraries' C, the `audio` mix task (not pinned to a core) and, with networking, TCP/IP and TLS |
| P4, low-power core | nothing of loft's |
| C6 | the radio firmware above |

On the device a `par` loop runs its rows in order: `loft_cc.h` turns `LOFT_THREADS` on only for POSIX
hosts and Windows.  Using the second core, and moving network work to the C6, are optional work after
measurement: [@PLN213](https://github.com/loft-lang/plans/issues/213) tier C.

## Games for the Tanmatsu

The Tanmatsu suits games that are decided by thought rather than reflexes: its full keyboard
is a roguelike's and a strategy game's input, and a game with time to spare per move or tick can
spend it on software rendering.  The three larger loft games, as their trees stand:

| Game | What it is | How it draws | On the Tanmatsu |
|---|---|---|---|
| dryopea | a sci-fi tower defence on a fixed tick (`fixstep`), a strategy game by its design | `graphics::Canvas`, the library's software rasteriser in loft; GL only to put the canvas on the screen | closest: Canvas compiles for the C backend, so only the frame's way to the screen (`screen`) and the keys (`input`) need the Tanmatsu path.  Open: whether Canvas, drawing per pixel in loft, is fast enough on the P4 |
| crawler | a ZAngband-style hex roguelike; time advances with the distance travelled, turning is free | a 2D egocentric view, a character map (`ovmap`), and first-person 3D through GL shaders | the game fits; its 3D needs a renderer for a machine without a GPU (below).  4.9 MB of sprites: the asset inspection takes what is named, and a larger set goes per area |
| a 2.5D dogfighter ([@PLN214](https://github.com/loft-lang/plans/issues/214)) | arcade air combat with PvP over the network, planes nearer the camera drawn bigger | sprites rendered from a 3D model when the game is built, rotated and shrunk while it runs; no 3D on the device | planned as the Tanmatsu's network test: several Tanmatsus against one server, each reporting its performance figures |
| moros | an RPG whose game world is still to be built; the tree holds its tabletop rules and cards and the lavition hex-world editor | the editor through GL shaders and voxel meshes | the RPG, once it exists, fits as crawler does and shares crawler's world model (`moros_map`, `moros_render`); the editor stays a desktop tool |

**One world, drawn at more than one fidelity.**  A game keeps its world in packages that draw
nothing, and each machine draws that world with the renderer it can afford: the views over one
state, and the drawing-budget tier a game asks for, that
[@PLN209](plans/209-platform-agnostic-games/README.md) sets out.  For the hex worlds of crawler
and moros the world description is the hex field itself (cells, heights, walls, floors and
ceilings, materials, the things standing in it), and the renderers over it differ in fidelity,
not in what they show:

| Fidelity | Renderer | Where |
|---|---|---|
| high | GPU meshes with the game's shaders (@PLN209's tier G) | desktop, browser |
| low | a column renderer in C: walls as vertical spans and floors through a precomputed division table, so perspective-correct texturing needs no division per pixel (the method tinygpus' DMC-1 uses on the MCH2022 badge's iCE40), drawn at 400x240 or below and scaled up | the Tanmatsu's CPU, later its FPGA |
| lowest | the character map | any terminal |

A hex world is the case the column renderer is good at: its walls are vertical and its floors and
ceilings flat.  It also suits the panel: a column of the landscape picture is a row of the
portrait panel, the direction `screen` already writes.  The renderer's span list is what an FPGA
would take over, so a board that adds one keeps the game, the API and the low-fidelity renderer's
front half.

A shader written once (@PLN209's shading subset of loft) has a C build that runs on the CPU; for
the low tier the game names a small set of materials (textured, vertex colour, lit, lightmapped)
that both renderers implement, rather than a shader the low tier cannot afford.

**The order.**  dryopea first, as the cheapest real game to bring over (its Canvas through
`screen`, its keys through `input`, its frame time measured on the device); then crawler's column
renderer behind the graphics API, walls and floors first; then moros' game world on the same
renderer.  The games are their own agents' to change: this stream builds the libraries and the
renderer and checks them on a library's testbed.

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
  for 32-bit x86.  Every library above passes its tests on both.  No CI workflow runs either
  sweep yet: they become nightly jobs, with `scripts/width64_census.py` reporting where 64-bit
  work is paid (a report, not a gate), once a game demo runs on the device at 60 fps
  ([@PLN204](https://github.com/loft-lang/plans/issues/204) S8).
- **The Tanmatsu sections:** each library's `ESP_PLATFORM` code is type-checked against the real
  BSP headers, with small stand-ins for the FreeRTOS and ESP-IDF headers
  ([HARDWARE_BACKENDS.md § Checking a platform section](HARDWARE_BACKENDS.md#checking-a-platform-section)).

**On the device.**  `tile-world-game/sheet.loft`, built with `--native-esp` and installed
with badgelink, runs: the asset filesystem, `screen`'s display section, `input`'s key polling,
the console timing lines and the return to the launcher, black at both ends.  On that run the
display is up about 0.4 s after the app starts; showing one 192x176 frame at scale 2 takes
11 ms with `screen`'s row-at-a-time path (`loft-libs-game` branch `tuxedo-screen-esp-present`,
not yet published), where the pixel-by-pixel path took 63 ms.

**Open:**

- `tile-world-game`'s game has not run on the device.  It draws at the panel's own 800x480
  through `tiles`' camera (zoom 3) and prints `@t frame` lines with its draw, copy and show
  times; on a PC the rasteriser takes 1.7 ms for such a frame, and the device's time, sound and
  area switch are unmeasured.
- The I2S output task (`audio`) has not run on the device.
- From reset to the app's start the panel shows blue, before any of the app's code runs.
- The P4's pixel accelerator (PPA) can scale and rotate a frame in hardware, which `screen`
  does on the CPU; the chip's ROM has an inflater (`tinfl_decompress`) lodepng could use; the
  runtime's hash seed reads `/dev/urandom`, which ESP-IDF does not have, where the hardware
  random number generator is (`esp_fill_random`).
- `loft_esp_nvs` (`loft_esp.h`) erases the whole NVS partition when it is full or of a newer format.
  That partition is the launcher's, with its Wi-Fi networks and settings; an app must never erase it
  ([@PLN213](https://github.com/loft-lang/plans/issues/213) T0.4).
- The network libraries on the device, and the build-time refusal a native without C on the target
  owes ([@PLN213](https://github.com/loft-lang/plans/issues/213) T0.3, T3–T5).
- The screen's physical size, for a game that draws to scale: a loft call with a defined
  "unknown" answer, filled on the Tanmatsu from the panel's specification until the BSP
  reports it.
