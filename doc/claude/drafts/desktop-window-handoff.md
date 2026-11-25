<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# HANDOFF — tile-world-game on a fresh Linux desktop, and the desktop window's future

> **Status: handoff, 2026-11-24.**  What happened when the daily build and `tile-world-game`
> were set up from scratch on an Ubuntu (GNOME, Wayland) machine, the three defects it found,
> and a proposal for the desktop window on Linux.  Nothing below is built yet; each item says
> where it belongs.  Not a plan: the Wayland half becomes one (a `@PLN` issue) only on the
> owner's word.

## What was done

1. `loft` installed from main's daily build: `scripts/install_main_ci.sh` with
   `LOFT_DAILY_RUN=45241186122` (commit `6ac5b79`, 2026-11-23), into `~/.local`.  Main's
   newest run (`45413464426`, `f2035b35e`) had a **red `x86_64-unknown-linux-gnu` leg**, so the
   script's default (main's own commit) answered 3; the previous green run was taken by id.
2. `loft-lang/tile-world-game` cloned beside the checkout (`18429cf`, 2026-11-17),
   `./fetch-libs.sh` run, then `loft --native --lib libs main.loft`.
3. The game did not start; the two causes below.  With the first worked around it builds
   clean and stops at `screen::open`.

## Defect 1 — the game does not compile against `loft-libs-graphics` main

`fetch-libs.sh` takes every library at `main`.  `graphics` 0.14.0 (`e29845f`, 2026-11-20,
"the window is a gpu device") is a **declared break** (its `loft.toml`: a draw names its window
and textures, a drawing function takes a `Window` not a `const Window`).  The game, last
changed 2026-11-17, is still on the 0.13 API: **33 errors, all in `render3d.loft`**
(`const-to-plain-parameter` on `gl.win.*` / `gl.shader.*`, `Unknown field Texture.bind`,
`expected Window, got Shader on argument 2 of call to draw`).

- **Worked around locally**: the `loft-libs-graphics` packages in `libs/` (tiles, imaging,
  graphics, gridmesh, gpu, gpu_gl, gpu_record, scene3d) replaced by their tree at `1ab5e5b`,
  the last commit before 0.14 (`graphics` 0.13.4).  It then compiles with no errors.  A
  re-run of `fetch-libs.sh` undoes this.
- **Belongs to**: the game (a consumer, read-only from here) — move `render3d.loft` to 0.14
  (graphics README § Moving from 0.13), or pin `loft-libs-graphics` in `fetch-libs.sh` to a
  ref instead of `main`.  Report it to the game's stream.
- **The lesson for loft**: a consumer that fetches libraries by branch gets a declared break
  the day it lands.  Registry dependencies with version ranges (the game's own comment: "then
  the list becomes the game's loft.toml dependencies") would have held it on 0.13.

## Defect 2 — `screen` silently builds without a window when the X11 headers are missing

The game then printed `this machine has no screen loft can open (no display?)` and exited 0,
with `DISPLAY=:0` set and the display reachable (`xwininfo -root` answers).  A three-line
probe (`screen::open(64, 48, 4, "probe")` → `false`) reproduced it outside the game.

Cause: `loft-libs-game/screen/c/screen.c:457-460` (screen 0.2.2) enables its X11 section only
`#if __has_include(<X11/Xlib.h>)`.  This machine has `libX11.so.6` but no `libx11-dev`, so the
shim is compiled as the "no screen here" variant — without a warning — and `sc_open` answers 0.
The fix that works today is `sudo apt install libx11-dev` and deleting
`libs/screen/native-auto/` so the shim is rebuilt.

Why the headers are needed at all: libX11 is `dlopen`ed at run time, but the C still takes
`Display`, `Window`, `XImage`, `XEvent`/`XKeyEvent`, `ZPixmap`, the event masks and the `XK_*`
keysyms from `<X11/Xlib.h>`, `Xutil.h` and `keysym.h`, and types its ~25 function pointers with
them.

**Proposed fix (loft-libs-game, small):** declare the X11 subset in `screen.c` itself, as
`loft-libs-graphics/graphics/c/graphics_gl.c` already does for X11 + GLX (it includes no X11
header: `gx_fbconfig`, `gx_visual_info`, its own function-pointer table).  The X11 protocol
structs and keysym values are frozen, so this is safe, and it is what screen's macOS section
already does for AppKit ("building needs no SDK").  Independently: **never fall back
silently** — a Linux build that ends up without a backend should say so at build time
(`#warning`, or a line from the shim build), and `sc_open` failing on a machine that has
`DISPLAY` set should say why.  Guard: build the shim with the headers hidden and assert
`screen::open` (on-screen, under Xvfb) answers true.

## Defect 3 — main's daily Linux leg is red

Run `45413464426` (`f2035b35e`, the @PLN225/@PLN226 join): macOS and Windows green, Linux red.
Until it is green `make install-main` falls back to a local build.  Not investigated here.

## The proposal — is X11 the long-term desktop window on Linux?

**Today it works, through XWayland.**  Ubuntu's default session has been Wayland for years and
recent GNOME (49, Ubuntu 25.10) removed the Xorg session; X11 clients keep running through
XWayland, which GNOME keeps (started on demand).  Every desktop window loft's libraries open on
Linux is X11: `screen` (Xlib, 2D frames) and `graphics`/`gpu_gl` (Xlib + GLX).  None speaks
Wayland.

**What XWayland costs:** blurry windows under fractional scaling (rendered at 1x, stretched —
bad for pixel-art tiles); input through a translation layer (focus, repeat, keyboard-layout
edge cases); and Wayland-only sessions (kiosks, embedded compositors, some minimal desktops)
without XWayland at all.  The toolkits (GTK, Qt, SDL, GLFW) all treat Wayland as the primary
Linux backend now.

**Proposed order:**

1. **Now — headers vendored (Defect 2).**  Keeps X11 as the always-available fallback and
   removes the `-dev` package from every user's path.
2. **Later — a native Wayland backend, tried first, X11 the fallback**, in `screen` and in the
   GL window, in the libraries' own style (everything `dlopen`ed or vendored, no link-time
   dependency):
   - `libwayland-client.so.0` opened at run time; the `xdg-shell` protocol glue (the
     `wayland-scanner` output, a small C file) vendored in the library;
   - `screen`: frames into `wl_shm` buffers; GL: an EGL window surface (`wl_egl_window`) —
     `graphics_gl.c` already drives EGL for Android;
   - keys through `libxkbcommon.so.0`, opened at run time;
   - fractional scale honoured (`wp_fractional_scale_v1` / buffer scale), which is the point;
   - **decorations are the hard part**: GNOME draws none for Wayland clients
     (no `xdg-decoration` server side), so either `libdecor` opened at run time or a minimal
     title bar drawn by the library.
   - Size: comparable to the X11 section per library — a few hundred lines, not thousands.
3. **The alternative, not recommended:** open SDL3 at run time and let it do X11, Wayland,
   decorations, scaling and input everywhere.  Far less code, but a large external runtime
   dependency, against the libraries' rule that what they open is the OS's or vendored.

Which repos: `loft-libs-game` (`screen`) and `loft-libs-graphics` (`graphics`, `gpu_gl`).
Their tests need a Wayland compositor in CI (a headless `weston --backend=headless` or `cage`)
beside the existing Xvfb runs — note @P358: a GL test under `xvfb-run` on a Wayland session
reaches the real compositor unless `WAYLAND_DISPLAY` is unset, so a dual-backend library needs
that pinning in both directions.

## Open for the owner

- Report Defect 1 to the game's stream, or leave it to them?
- Step 1 (vendored X11 subset in `screen`): a `loft-libs-game` PR, per the library rule.
- Step 2: open a `@PLN` issue for the Wayland backend, or keep it here as a proposal?
