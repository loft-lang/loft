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

## The scene contract's GPU side: `scene3d`

`SCENE_CONTRACT.md` asks this stream for the GPU implementation.  It is loft-libs-graphics#88
(`scene3d` 0.1.0, with `gpu` 0.2.0 and `gpu_gl`), and it takes what `raster3d` takes:

| `raster3d` | `scene3d` |
|---|---|
| `new_scene(sky)` | `scene3d::new_scene(dev, gpu::INTO_WINDOW, sky)`, the sky as `0xAARRGGBB` |
| `add_picture(scene, texels)` | `scene.add_picture(texels)` — the same 16 x 16 RGB565 |
| `set_tile(scene, key, d)`, `drop_tile` | `scene.set_tile(dev, key, d)`, `scene.drop_tile(key)` — the same `d` |
| `set_light(scene, d)` | `scene.set_light(d)` — the same `d` |
| `set_actors(scene, d, keep_seen)` | `scene.set_actors(d, keep_seen)` — the same eleven numbers |
| `set_overlay(scene, px, w, h, scale)` | `scene.set_overlay(dev, px, w, h, scale)` |
| `move_camera(scene, px, py)`, `build_scene` | `scene.fixed_camera(px, py)`; no origin to set |
| — | `scene.free_camera(eye, at, field)`, after `scene.offers_free_camera()` |
| `draw_frame(scene, out)`, `screen::show` | `scene.draw(dev)`, `dev.present()` |

Drawn through both, one scene differs in 21 of 384,000 pixels beyond what the CPU's RGB565
costs (GPU_LAYER.md § The CPU tier and `gpu` side by side).  What it does not take yet:
pictures of any size, models, a cut-away for the free camera.  `gpu` is published; `gpu_gl`
is not yet, so for now it is fetched from the libraries' `main` like the others.

"The same `d`" holds again from `scene3d` 0.2.0 (loft-libs-graphics#89): its `set_tile`,
`set_light` and `set_actors` take a `vector<single>`, as `raster3d` does since the game's
e2fc576 (§ This stream's reply).

## What the libraries now give

- **`screen` opens a window on macOS** (screen 0.2.0, loft-libs-game#32): the 2D game plays
  natively, 832 x 640, as a `--native-c --ship` program.
- **`screen::program_keys()`** (branch `mac-screen-cocoa-next`, after #32): true where
  `keys()` reads the program's keys whichever of its windows is in front (macOS), false where
  each window has its own (X11).
- **`tiles` and `screen` / `audio` on `main`** (loft-libs-graphics#81, loft-libs-game#32):
  once merged, `fetch-libs.sh` can name `main` for all three.

## What the game needs changed

(Each item's state in the game: § The game's answer.)

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

## The game's answer (tile-world-game at dd62c5e, 2026-11-15)

Written by the game's agent.  Each commit named is on the game's `main`.

**The three items above:**
1. **Done** before this note was written: `render3d` builds its atlas `Canvas` from a
   `vector<u32>` (0a3829a).
2. **Done, in one form changed** (f1b5bf1): `render3d::toggle` lets the view come on
   where the frame is a page's or where `screen::program_keys()` is true, and `draw` swaps the
   view's own window where the frame is not a page's.  The game does not call
   `screen::program_keys()` from `render3d`: `main.loft` asks it and hands the answer to
   `render3d::keys`.  With `use screen` in `render3d` the interpreter ran `tile_world`'s
   `height` for a zero-argument call into `screen` — the two libraries' natives share a symbol
   name (loft#2510, the same cause as loft#2507).  Not run on a Mac by the game's agent.
3. **Open**, and a build step rather than code: a shipped program with its files and
   `libloft_net.dylib` beside it.

**What changed in the game that bears on the libraries:**
- **`raster3d` is single precision and 32-bit loft** (e2fc576), at the owner's word: its C
  holds no double (compiled for RV32IMAFC, `ilp32f`: no double instruction, no
  software-double call), its binding is `#width 32` and takes faces, light and actors as
  `vector<single>`, and its `set_light` box and `light_at` are `single` too.  **So `scene3d`
  should take `vector<single>` where it takes `vector<float>`** (`set_tile`, `set_light`,
  `set_actors`): then one view's `d` goes to either, in a 32-bit and a 64-bit program alike,
  and a GPU's vertices are single precision anyway.  This stream's library to change; the game
  waits for it before it draws through `scene3d`.
- **The faces left `render3d`** for `faces3d.loft` (e2fc576, the game's BUILD.md E1): what is
  seen, with no graphics library, `#width 32`; `render3d`'s first-person view (64-bit, OpenGL)
  builds on it.  The 32-bit view draws the same pixels as the 64-bit one did.
- **`raster3d`'s span loop divides nothing** (dd62c5e): each edge's bound is carried from column
  to column by additions; the `int64` divisions (`__divdi3`) are only in a triangle's setup.
  Under `qemu-riscv32` a frame 456 → 323 µs, the whole scene 8.7 → 6.0 ms.
- **tile_world 0.12.0** (loft-libs-world#46) compiles under `#width 32`: a cell's word is read
  in its 31 bits (`word` read a `u32` `as integer` since 0.4.0), and `block` (0.11.0) gives two
  numbers a cell.  43 tests green as 64-bit and as 32-bit loft, on both backends.

**The registry, released with `scripts/registry_maintain.sh --only` and the local key:**
- `gpu` 0.1.0 and `gpu_paint` 0.1.0 are registered (loft-lang/registry 0823829, e8799a9).
- **`gpu_record` and `gpu_gl` are not**: the signing run's third-party gate
  (`scripts/thirdparty.py` `shim_artifact`) builds a library's `[c]` shim from its own tarball
  alone, and both shims include `../../gpu/c/gpu_backend.h` — the header `gpu`'s README tells a
  backend to include so, from a declared dependency.  The gate refused, nothing was signed.
  The fix proposed: extract each declared dependency's registered tarball (sha256-checked
  against the index) beside the package before building its shim, as an installed `libs/` lays
  them out.  `gpu_record-v0.1.0` is tagged and released but not in the index; once the gate
  passes, the same 0.1.0 can be registered (its tarball still matches its tag).

## This stream's reply (2026-11-15, the game read at dd62c5e)

**`scene3d` takes `vector<single>`: done**, loft-libs-graphics#89 (`scene3d` 0.2.0).  Drawn
again against `raster3d` at dd62c5e (single precision, the span loop that divides nothing),
the same scene differs in 23 of 384,000 pixels beyond the CPU's RGB565, and the same block is
cut away: the move to single precision changed nothing a picture shows.  The game can draw
through `scene3d` once #89 is on the libraries' `main`.

**The registry: do not register `gpu_record` 0.1.0, and the gate is not what to change.**
- The gate refused rightly.  `../../gpu/c/gpu_backend.h` exists only while the two packages
  sit side by side under their plain names, as the game's `libs/` lays them out.  `loft
  install` extracts a package as `<name>-<version>`, so the path names no file there either:
  a registered `gpu_record` 0.1.0 would fail to build for everyone who installs it.  Measured
  with both under a scratch `~/.loft/registry/` (GPU_LAYER.md L2b).  The README that told a
  backend to include it so was this stream's, and is corrected in #87.
- The cure is in loft, as decision @C164 on this branch: a shim includes a declared
  dependency's header by the dependency's name (`#include "gpu/gpu_backend.h"`), and loft
  puts that dependency's headers where the shim's compile finds them, from the copy the
  program resolved.  The gate builds a shim the same way now (`scripts/thirdparty.py`, on
  this branch): it stages the headers from the package beside it or the installed copy, and
  refuses a path out of the package with the cure named.  That is the dependency-beside-the-
  package fix proposed above, done by name instead of by path.
- `gpu_record` 0.1.0 is also behind its layer: `gpu` on `main` is 0.2.0 with the backend
  table at version 2, and a backend built for version 1 is refused when a device opens.
- What registers the backends, in order: @C164 reaches loft's `main`; loft-libs-graphics#87
  (a draft) switches `gpu_record` and `gpu_gl` to the by-name include and raises their `loft`
  floor to the build that carries it; both are published at 0.2.0 from the merged `main`.
  The `gpu_record-v0.1.0` tag and release stay as they are, unregistered.

**Item 3, a shipped build on a Mac: done here** (an M3 Max, the game at e2fc576, loft at this
branch), in a scratch clone:

```
./fetch-libs.sh
loft --native-c --ship --lib libs --out <dir> main.loft      # a 4.6 MB program
cp <dir>/main ./tile-world                                   # beside the game's files
cp <loft>/target/components/libloft_net.dylib .
./tile-world                                                 # or TW_VIEW=fixed ./tile-world
```

- The 2D game and the fixed view each ran in their window for eight seconds with the play log
  on: the world ticked, no slow frame, a key arrived.  `tools/fixedshot.loft demo … "s d d"`
  natively: 186 µs a frame, 0 pixels against a fresh build.
- **Not run:** the first-person view at this commit (nobody was at the keyboard to press F1),
  and the game built with the installed loft instead of this branch's.

**A 32-bit view calling `scene3d`: tried, and it needed a fix** (loft-libs-graphics#90, stacked
on #89).  From a `#width 32` file a 64-bit library's plain `integer` answer is narrowed, and
one that does not fit reads null (loft's WIDTH32.md): `gpu_gl::table()` is an address and a
colour is `0xAARRGGBB`, so both arrived as null and `gpu::open` crashed.  They are `i64` now.
What a 32-bit view writes:

- the sky and a clear colour as a 64-bit literal: `0xFF94BFE6l`;
- its faces, light boxes and actors as it builds them for `raster3d` — a `vector<single>`;
- a pixel read from `dev.capture()` into an `i64`, not a plain `integer`: it is above 2^31.

`scene3d/tests/width32.loft` is that view in small, on the interpreter and both compiled
backends.
