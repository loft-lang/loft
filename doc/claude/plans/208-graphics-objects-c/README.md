<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 208 — graphics 0.10: an object API on a C shim

## Status

Active on loft-libs-graphics branch `firewall02-graphics-c` (from `origin/main` `d265125`).
Issue: [@PLN208](https://github.com/loft-lang/plans/issues/208).

| Phase | State |
|---|---|
| G0 | Done — a `[c] shim` and `[native] crate` coexist; the shim answers on all three backends (a scratch probe, not committed) |
| G1 | Done — `Font` in loft (`31d41be`), its metrics pinned to fontdue's (`392e212`); 1532 of 1536 comparison cells agree, the four others a symbol font where fontdue misread its Mac Roman table |
| G2 | Done — `save_png` through imaging (`16818a4`); six canvases identical to 0.9's files.  It surfaced an imaging defect, fixed as imaging 0.5.1: lodepng's `auto_convert` wrote few-colour images as palettes |
| G3 | Done — `enum Event` with public variant fields (`16818a4`).  The suite is 108/108 on the interpreter, `--native` and the C; G1–G3 and imaging 0.5.1 go to main together in loft-libs-graphics#67 |
| G4 | Done — `Window` over X11 + GLX (`b4ed3ce`): `tests/window.loft` 3/3 on the three backends under Xvfb, the suite 111/111; `compare/window.loft` reads back 0.9's pixels for five clears |
| G5–G11 | Open |

## Goal

graphics 0.10 replaces 0.9's integer font handles and its hidden current window and current event
with objects the caller holds — `Window`, `Event`, `Font` — and its natives become a `[c] shim`, so
the package runs on the interpreter, `--native` and the C backend with no Rust crate.

## Why

- Library CI runs every package on the C backend, which cannot load graphics' crate (winit,
  glutin, gl, fontdue, png, image, rodio): its C leg is red.
- The owner's rule is no hidden state — short-lived objects that come and go (and a cache only
  when it is invisible, [@PLN207](../207-memo-cache/README.md)).  0.9 keeps its state out of sight: a
  font table behind an integer, a current window `gl_poll_events()` reads, a latched current event
  `gl_event_x()` reads.  A loft library cannot hold such a table at all — a file's top level
  declares no variables — so 0.9's font API has no loft spelling, by design.
- The owner rewrites the games on the new API.  0.9.12 stays published; `api_compatible_with`
  rises to `0.10.0`, so `loft update` keeps each consumer on 0.9 until it moves.

## Shape

```loft
w = open_window(800, 600, "game") ?? return;   // Window: the OS window and its GL context
f = load_font("ui.ttf") ?? return;             // Font: parsed TrueType tables, a loft value
while w.poll() {
  for ev in w.events() {                       // Event: an enum, its fields in its variant
    match ev {
      KeyDown { key, mods } => press(key, mods),
      Resize { width, height } => relayout(width, height),
      _ => {}
    }
  }
  width = f.measure("Score", 24.0);
  w.swap();
}                                              // OpDrop closes the window at scope end
```

What the language gives this, measured on all three backends: `OpDrop` at the owner's death
(scope end, an early `?? return`, a vector's death, reverse declaration order), a copy that moves,
enum variants with fields under `match`.  Its deliberate limits shape the API
([INTERFACES_DROP.md](../../INTERFACES_DROP.md)):

| Limit | API answer |
|---|---|
| Taking a value OUT of its owner (`o.tex = other`, `v[i] = other`) does not release it | Replacing a texture is a call that releases the old one |
| A keyed collection never releases its records | Host objects live in vectors |
| A struct holding a droppable cannot be copied | Host objects stay out of the game's simulation state |
| A drop cannot fail or answer | Closing cannot fail; anything that can is an explicit call |

The shim enforces what the language cannot: a texture or buffer dropped after its window closed
is a no-op, and a GL call from a thread other than the window's is refused loudly.

## The shim

`graphics/c/graphics_gl.c`, bound by `#c` declarations.  It opens the platform's libraries at run
time — `libX11.so.6` and `libGL.so.1` on Linux, `user32`/`gdi32`/`opengl32` on Windows, the
Objective-C runtime and Cocoa on macOS — and declares the few types it uses itself, so it builds
with loft's fixed flags and no development package ([PACKAGES_C_BINDING.md § Declaring the
library](../../PACKAGES_C_BINDING.md)).  `#c` takes no scalar float, so a float parameter
travels as a `vector<float>` built by the loft wrapper; public signatures keep their floats.

## Phases

Each phase compares the new path with 0.9 and can go red on its own.  Pointers into
loft-libs-graphics are at `d265125`: `graphics/src/graphics.loft` (G.loft),
`graphics/native/src/lib.rs` (G.rs), `graphics/native/src/text.rs` (T.rs).

**G0 — probe: a shim beside the crate (XS).**  One `#c` function in a `[c] shim` while
`[native] crate` stays; call it on the three backends.  Goes red when the manifest or a backend
refuses the pair — then the switch is all-at-once and G1–G3 move with G9.

**G1 — `Font` in loft (M).**  A TrueType reader over the bytes `read_bytes` answers: `head`
(unitsPerEm), `hhea` (ascender, descender, lineGap), `maxp`, `hmtx`, `cmap` formats 4 and 12,
`kern` format 0.  `measure`, `ascent`, `line_height` follow fontdue's formulas (T.rs:27-82):
`scale = size / unitsPerEm`; width = Σ advance·scale + kern(prev, c)·scale; line height =
`ceil((ascender − descender + lineGap)·scale)`.  `load_font` answers null for a missing or
unparseable file.
- Pointers: G.loft:1388-1430 (the 0.9 declarations), G.rs:2517 / 2536 / 2244 / 2255.
- Goes red when: a comparison program, run against 0.9 on the interpreter, measures a fixed
  string list on both fixture fonts at four sizes and any metric differs by more than 1e-4 px;
  `tests/kerning.loft`, `font_ascent.loft`, `text_height.loft` move to the new API with their
  assertions unchanged.

**G2 — `save_png` through imaging (S).**  `Canvas.save_png` packs `0xAARRGGBB` into RGBA bytes
and calls imaging (published 0.5.0, a C shim over lodepng); RGB when every pixel is opaque, RGBA
otherwise (G.rs:2014).
- Pointers: G.loft:878-888; `imaging/src/imaging.loft:129`.
- Goes red when: the PNGs 0.9 and 0.10 write for the canvas fixtures decode to different pixels
  or a different colour type.

**G3 — `Event`, headless (S).**  `enum Event` with one variant per 0.9 tag (G.rs:180-192) and its
fields; `Window.events()` drains the shim's queue.  Without a window there is no queue to drain.
- Pointers: G.loft:1300-1387, G.rs:168-270.
- Goes red when: `tests/input_events.loft`, moved to the new API, fails; the constants it pins
  (`EV_*`, `MOD_*`, `KEY_*` codes) change.
- Milestone: after G1–G3 the library's C leg is green — the 5 failures today are exactly
  `load_font`, `save_png` and `next_event`.

**G4 — the Window on Linux (M).**  `open_window` / `open_fullscreen` / `poll` / `swap` / `close`
over X11 + GLX (a 3.3 core context through `glXCreateContextAttribsARB`), `OpDrop` closing it.
`swap` forces alpha to 1 before presenting (G.rs:643); `poll` applies a resize after the pump
(G.rs:594).
- Goes red when: under Xvfb, a program that opens a window, clears to a colour and reads the
  framebuffer back answers a different pixel than 0.9 does.

Comparisons against 0.9 live in `graphics/compare/`, one file each, run by hand on the interpreter
and `--native` (the C cannot load 0.9's crate) under `xvfb-run -s "-noreset …"` — an X server that
loses its last client resets and refuses the next window while it does.  The 0.10 path runs first in
each file: 0.9's destroy leaves its dead context bound to the thread.  The shim keeps libX11 and
libGL loaded (`RTLD_NODELETE`); unloading Mesa leaked 10 MiB of address space per window.

**G5 — the GL calls, one group per step (M each).**  Shaders and uniforms · buffers, instancing,
indices · state (blend, cull, depth, scissor, viewport) · framebuffers and textures · draw modes,
line width, point size.  `clear` keeps 0.9's `0xAARRGGBB` packing (G.rs:669).
- Goes red when: a scene per group, rendered by 0.9 and by the shim under Xvfb, differs in the
  screenshot.  One group per commit, so a red names its group.

**G6 — input (S).**  X11 key, button, motion, wheel, configure and close events into `Event`; the
key codes are 0.9's table (G.rs:275-327: ASCII lower-cased, arrows 128-131, F1-F12 135-146, …).
- Goes red when: events injected under Xvfb (`xdotool`) arrive as a different `Event` list than
  the table says.

**G7 — text rasterizing in loft (M).**  Quadratic outlines from `glyf`, coverage per pixel,
`Font.rasterize` and `Canvas.draw_text` (G.loft:1438).  The glyph cache is the `Font`'s own until
@PLN207 can key a whole `Font` cheaply.
- Goes red when: the alpha bitmaps differ from fontdue's (T.rs:84-162) by more than a measured
  tolerance on the fixture strings; `examples/gold-text-fixture.loft` still matches its gold.

**G8 — textures and audio (S each).**  `Window.load_texture` decodes PNG through imaging (JPEG,
which no consumer uses, is dropped).  Audio, which no consumer uses, moves to loft-libs-game's
`audio` library (published first) or leaves graphics.

**G9 — release 0.10.0 (S).**  The crate deleted, `[native]` gone, `api_compatible_with =
"0.10.0"`, the `loft` floor raised to the release G1's `Font` needs (2026.10.0 fails its font
tests on `--native`), the README and the guide on the new API.  Goes red when: the library CI fails on any
backend.

**G10 — Windows (M), G11 — macOS (L).**  Win32 + WGL; Cocoa through the Objective-C runtime.
Each repeats G4–G6's comparisons on its own runner.

## Open design questions

- A second window: 0.9 had one; the object API allows several, and the shim's per-window state
  makes it cheap — but the GL context is current per thread.  Decide when a game asks.
- Font rendering on the GPU (an atlas texture) belongs to `Window`, built from `Font.rasterize`.

## Documentation

- graphics `README.md`, `docs/01-getting-started.loft` and every `@GFX-` worked example on the new
  API; the crate's notes leave with it.
- `loft-libs-graphics/graphics/loft.toml` — `api_compatible_with`, the shim, no `[native]`.
- loft: `PACKAGES_C_BINDING.md` gains graphics as the dlopen-a-platform-library example;
  `LIBRARY_BRANCHES.md` lists the branch by itself (generated nightly).
- `CHANGELOG.md` (a library release note), the games' migration note in each game's own repo.

## See also

[INTERFACES_DROP.md](../../INTERFACES_DROP.md) · [PACKAGES_C_BINDING.md](../../PACKAGES_C_BINDING.md) ·
[API_SURFACE.md § S6](../../API_SURFACE.md) · [@PLN207](../207-memo-cache/README.md) ·
[C_BACKEND.md](../../C_BACKEND.md)
