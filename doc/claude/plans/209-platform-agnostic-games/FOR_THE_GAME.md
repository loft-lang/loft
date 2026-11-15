<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 209 — for tile-world-game's agent: the game on a Mac

What [GPU_LAYER.md](GPU_LAYER.md) track P found that is the game's own to change.  This
stream does not edit the game; each line was tried in a scratch copy of the game's `main`
(dc27cf0) on an Apple-silicon Mac, with loft at `tuxedo-pr2438-post`.

The game's agent reads this as `HANDOFF_FROM_LOFT.md`, on the branch
`loft-handoff-2026-11-15` of the game's repo: the same findings, read against the game at
`0a3829a`, with the state of the loft fixes and what the GPU layer will ask of its view.  At
that commit the game already builds its `Canvas` from a `vector<u32>` (item 1 below).

## What the libraries now give

- **`screen` opens a window on macOS** (screen 0.2.0, loft-libs-game#32): the 2D game plays
  natively, 832 x 640, as a `--native-c --ship` program.
- **`screen::program_keys()`** (branch `mac-screen-cocoa-next`, after #32): true where
  `keys()` reads the program's keys whichever of its windows is in front (macOS), false where
  each window has its own (X11).
- **`tiles` and `screen` / `audio` on `main`** (loft-libs-graphics#81, loft-libs-game#32):
  once merged, `fetch-libs.sh` can name `main` for all three.

## What the game needs changed

1. **`render3d.loft`, the atlas upload** builds a `Canvas` from a `vector<integer>`, which
   graphics 0.12 refuses (`Canvas.data` is a `vector<u32>`).  Declare `data: vector<u32>`,
   append `0 as u32` and `(graphics::rgba(…) & 4294967295) as u32`.  Without it the game does
   not compile against graphics' `main`.
2. **The first-person view beside the frame, until the GPU layer gives it one window** (the
   owner's choice, 2026-11-15).  Two changes, both asking what the machine has:
   - `toggle`: the view may come on where the frame is a page's, **or** where
     `screen::program_keys()` is true — the view then opens a window of its own
     (`graphics::open_window` or `open_fullscreen`) and the keys keep arriving through
     `screen`.  On X11 the gate stays shut: the view's window would take the keys.
   - `draw`: after the view is drawn, `gl.win.swap()` where the frame is NOT a page's
     (`!tiles::on_display(g.surface)`).  In a page `swap` hands the browser a frame, which
     `screen::poll` already does.
   The hearts and the note (`overlay`) are drawn on the game's surface, so in this form they
   show in the 2D window, not over the view.
3. **A shipped program reads its files beside the executable**, not in the working directory,
   and needs `libloft_net.dylib` (the `web` library's component, `make components`) beside
   it: build with `--out` into the game's folder, or copy both there.

## Measured in the scratch copy

- The 2D game: `show` 2.7 ms a frame at 832 x 640.
- The first-person view in its own 960 x 720 window: the same picture from a `--native-c` and
  a `--ship` build, 6.7 ms a frame shipped.
