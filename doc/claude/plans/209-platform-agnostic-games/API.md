<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 209 — the game API, frame tier

The loft surface arc F builds ([README.md](README.md)): what a game writes against so it runs on
the Tanmatsu, in a page and on a desktop without knowing which.  A design, not yet built; every
signature here is a proposal the F-phases check.

## The rules it follows

1. **State lives in objects the caller holds** ([API_SURFACE.md § S6](../../API_SURFACE.md)).  A
   screen, a sound output, a save place, an input state are values; the game owns them, passes
   them, and they end when it drops them (`OpDrop` closes what the host opened).  No library keeps
   a current screen, a current window or a latched event; no call reaches state the caller cannot
   see.  An integer handle stays only inside an object, where the HOST owns the thing (a window, a
   voice).
2. **A cache is as if it is not there** ([@PLN207](../207-memo-cache/README.md)).  Decoding a tile
   sheet, rasterising a glyph, converting a frame's pixels: each is a pure function of its inputs,
   cached by `#memo` where it is worth it.  No cache object, no handle into a cache, no "load once
   then look up" API: a game calls the function again and only gets faster.
3. **Ask, never detect.**  The machine is described by what it has (`Caps`), never by a name.
4. **The game sees its frame, not the display.**  Pointer and touch arrive in frame pixels; the
   screen scales, letterboxes, rotates and masks.  A game that draws 208×160 never learns that
   the panel is 480×800 portrait.
5. **One thread.**  An object is used from the thread that opened it; another thread is refused
   with a message, as graphics' `Window` is.
6. **Optimise freely per platform, invisibly.**  Each platform's section may take every shortcut
   its hardware offers — a pixel accelerator, a native format, a SIMD path, a format kept
   internally — as long as the game cannot tell except by speed.  The check is the same as the
   cache's: the frame `capture` answers and every value a call returns are identical with the
   shortcut on or off.

## Pixels: RGBA everywhere, packed per destination

A game sees one pixel format: 32-bit RGBA — PNG RGBA assets, `Canvas`, and every frame it shows.
What a destination wants is produced behind the API, at two points:

- **Assets, when the game is built for a target.**  A packer turns the RGBA sources into the
  target's format — RGB565 for the Tanmatsu, dithered once there; untouched on a desktop or a
  page.  A pure function of the source and the target, so its output is kept by key.
- **Frames, when `show` presents.**  The screen converts, scales and rotates in one pass — on
  the Tanmatsu by the ESP32-P4's pixel accelerator where it does the conversion, with a fixed
  ordered dither so nothing shimmers between frames.

So a game's frame is byte-identical on every machine before it is presented: only the presenter
differs, which is what the F-phases compare.  RGBA is stored packed, four bytes a pixel
(`vector<u32>`) — not one loft `integer` per pixel, as `Canvas` keeps it today.  `tiles` may keep
RGB565 internally on the Tanmatsu (rule 6); its API takes and answers RGBA.  Palette effects
(colour cycling) are an explicit `tiles` feature — an indexed sheet and a palette object — not a
pixel format.

## Assets: declared once, reworked by the ship build

A game names its assets in `loft.toml`, beside `[[embed]]`
([HTML_EXPORT.md](../../HTML_EXPORT.md) § Putting a file there), and loads them by the same path
on every machine:

```toml
[[asset]]
path    = "art/hero.png"     # what the game passes to load_sheet / load_texture
quality = "exact"            # exact | reduced | compressed — how far a target may rework it
                             # (a mesh: `reduced` with `max_error = 0.01`, in model units)
```

A development run reads the RGBA source.  **`--ship` for a target reworks every declared asset
into that target's best form** and carries the result under the same path, so no game line
changes:

| Target | What `--ship` makes of a texture |
|---|---|
| the Tanmatsu | RGB565, dithered once; sheets packed into atlases; scaled down to what its tier draws |
| a page | the source, or a smaller lossless encoding; atlases |
| a desktop, frame tier | the source |
| a GPU target (arc G) | mipmaps, and the GPU's compressed format where `quality` allows (BC on a desktop, ASTC / ETC2 on a phone or console) |

**Meshes** are reworked the same way, for the GPU targets only (the Tanmatsu draws no 3D): a
`[[asset]]` naming a `.glb` may be simplified for a target's screen and tier — fewer triangles,
the same silhouette — by an edge-collapse routine beside `glb` / `mesh3d` in loft-libs-assets.
Its bound is geometric: the simplified surface stays within a declared distance of the source.

Each rework is a pure function of the source, the target and the quality, kept by that key, so a
second ship build redoes nothing that did not change.

This is the one place a shipped build may differ from a development build, and only by what the
asset declares: `exact` is never changed in its pixels (pixel art, fonts, UI), `reduced` may lose
colour depth and size, `compressed` may use a GPU's lossy formats.  The ship build checks each
reworked asset against its source within the bound its quality allows, and refuses an asset
over it with the asset's path and the measured difference.

## Packages

| Package | Holds | Depends on |
|---|---|---|
| `machine` | `Caps`, `Event`, the `KEY_*` names, `Saves`, `Clock` | nothing |
| `geom` | the math types (`vec2` … `Mat4`) and pure geometry, moved from `graphics` (F10) | nothing |
| `canvas` | `Canvas` (RGBA) and its drawing, moved from `graphics` (F10) | `geom` |
| `tiles` | tile sheets, maps and sprites drawn into a `Canvas`; sheets and maps as objects, not integer handles (exists, re-shaped) | `canvas` |
| `screen` | `Screen`: shows a frame, answers its events | `machine` |
| `input` | `Bindings`, `Actions`: events to actions | `machine` (no longer `graphics`) |
| `audio` | `Audio`, `Clip`, `Voice` (exists, re-shaped) | `machine` |

`graphics` re-exports `geom`, `canvas` and `machine`'s `Event` and keys, so no 0.11 caller breaks;
in arc G it opens its window through the same platform sections.

## `machine` — what the machine has

```loft
pub enum Tier { Low, Mid, High }
pub enum Shape { Rect, Round { pub diameter: integer } }

pub struct Display {
  pub width: integer,          // pixels, as the panel is mounted: 480 x 800 on a Tanmatsu
  pub height: integer,
  pub density: integer,        // pixels per inch, 0 when the host does not say
  pub shape: Shape,
  pub refresh: integer,        // frames per second, 0 when unknown
}

pub struct Caps {
  pub displays: vector<Display>,   // empty: a machine with no screen (a server, a test)
  pub keys: boolean,
  pub pointer: boolean,
  pub touch: boolean,
  pub gamepads: integer,
  pub sound: boolean,
  pub tier: Tier,
}

// What this machine has, asked of the host now.  A value: the game holds it, and asks again when
// it wants to know whether something was plugged in.
pub fn caps() -> Caps;
```

`Tier` is the one performance knob: `Low` a small device drawing on the CPU, `Mid` a page or a
modest desktop, `High` a desktop with a GPU.  A game reads it once and picks its frame size and
detail; it never measures the machine itself.

**Events** — graphics 0.10's `enum Event`, moved here unchanged and widened by the gamepad
(open question 4), so every library speaks one form.  Positions are in the frame's pixels.

```loft
pub enum Event {
  KeyDown { pub key: integer, pub mods: integer, pub repeat: boolean },
  KeyUp { pub key: integer, pub mods: integer },
  Text { pub content: text },
  PointerDown { pub button: integer, pub x: float, pub y: float },
  PointerUp { pub button: integer, pub x: float, pub y: float },
  PointerMove { pub x: float, pub y: float },
  Wheel { pub delta: integer, pub x: float, pub y: float },
  TouchBegin { pub id: integer, pub x: float, pub y: float },
  TouchMove { pub id: integer, pub x: float, pub y: float },
  TouchEnd { pub id: integer, pub x: float, pub y: float },
  Resize { pub width: integer, pub height: integer },   // the DISPLAY changed; see Screen.refit
  Close                                                   // the user asked to quit
}
```

Keys are the named `KEY_*` constants (`KEY_UP`, `KEY_A`, `KEY_ENTER`, `KEY_BACK`); a game never
writes a number.  A Tanmatsu's keys arrive as the same names.

**Saves** — a place to keep a game's data, by name.

```loft
pub struct Saves { place: text }                   // a directory, a page's storage key, an SD path
pub fn open_saves(game: text) -> Saves?;            // null where the machine keeps nothing
pub fn read(self: const Saves, name: text) -> vector<u8>?;
pub fn write(self: const Saves, name: text, data: const vector<u8>) -> boolean;
pub fn names(self: const Saves) -> vector<text>;
```

A desktop keeps them under the user's data directory, a page in its storage, the Tanmatsu on its
SD card.  `write` replaces a save whole or not at all.

**Clock** — fixed-rate time for a game loop; the `fixstep` library's accumulator over it.

```loft
pub struct Clock { hz: integer, start_us: integer, ticks: integer }
pub fn clock(hz: integer) -> Clock;
pub fn due(self: Clock) -> integer;     // ticks to run since the last call (0 or more)
```

## `screen` — one frame on one display

```loft
pub enum Fit { Pixel, Smooth }   // Pixel: a whole-number scale, letterboxed; Smooth: fill, keep aspect

pub struct Frame {
  pub width: integer,
  pub height: integer,
  pub fit: Fit,
}

// The frame a game asks for, adjusted to `d`: the largest whole-number scale of `want_w` x
// `want_h` that fits, its orientation turned to the display's, and on a round display the
// largest frame inside the circle.  A pure function of its arguments.
pub fn frame_for(d: const Display, want_w: integer, want_h: integer) -> Frame;

pub struct Screen { handle: integer }
fn OpDrop(self: Screen) { … }      // closes the window or releases the panel

// Show frames of `f` on display `d`.  On a desktop a window titled `title`, sized to the frame
// scaled; on a device the whole display; in a page the canvas.  Null where it cannot open.
pub fn open_screen(d: const Display, f: const Frame, title: text = "loft") -> Screen?;

// A screen of no display: frames are kept in memory, for tests and headless runs.
pub fn offscreen(f: const Frame) -> Screen;

pub fn poll(self: const Screen, wait_ms: integer = 0) -> boolean;   // false once closed
pub fn events(self: const Screen) -> vector<Event>;                  // since the last call
pub fn show(self: const Screen, c: const Canvas);                    // the frame, RGBA
pub fn frame(self: const Screen) -> Frame;
pub fn refit(self: Screen, f: const Frame);        // after a Resize: the game picks a new frame
pub fn capture(self: const Screen) -> Canvas;      // what was last shown, at frame size
```

`show` converts the frame to the display's pixel format, scales it and presents it.  It runs
every frame, so it is not cached: the work is the conversion, and it is cheap at frame size.

## `input` — events to actions

```loft
pub struct Bindings { … }                           // action name -> keys, buttons, touch zones
pub fn bindings_for(c: const Caps) -> Bindings;     // the defaults for what the machine has
pub fn bind(self: Bindings, action: text, key: integer);
pub fn bind_axis(self: Bindings, axis: text, minus: integer, plus: integer);

pub struct Actions { … }                            // this tick's state, and the last tick's
pub fn actions() -> Actions;
pub fn feed(self: Actions, events: const vector<Event>, b: const Bindings);   // once per tick
pub fn held(self: const Actions, action: text) -> boolean;
pub fn pressed(self: const Actions, action: text) -> boolean;   // went down this tick
pub fn released(self: const Actions, action: text) -> boolean;
pub fn axis(self: const Actions, axis: text) -> float;          // -1.0 .. 1.0
```

Input reads only the events it is fed: no polling of a device behind the game's back, so a
replayed event list is a replayed game.

## `audio` — one output, clips as values

```loft
pub struct Audio { handle: integer }
fn OpDrop(self: Audio) { … }                         // stops the output
pub fn open_audio(rate: integer = 44100) -> Audio?;  // null without sound: the game plays silent

pub struct Clip { pub rate: integer, pub channels: integer, pub samples: vector<i16> }
pub fn load_clip(path: text) -> Clip?;               // WAV / Ogg; decoding is #memo (path, stamp)

pub struct Voice { audio: integer, id: integer }     // one playing clip, owned by the engine
pub fn play(self: const Audio, c: const Clip, volume: float = 1.0, pan: float = 0.0,
            looping: boolean = false) -> Voice;
pub fn stop(self: const Voice);
pub fn set_volume(self: const Voice, v: float);
pub fn playing(self: const Voice) -> boolean;
```

A `Voice` names one playing; once it ends, its calls act on nothing — the rule the engine already
keeps.

## What is cached, and how

| Work | Function | Key | Why it is safe |
|---|---|---|---|
| decode a tile sheet or image | `tiles::load_sheet(path)` → `#memo decode_png(path, stamp)` | path, file stamp | pure given the stamp; the sheet is copied out |
| decode a sound | `audio::load_clip(path)` → `#memo decode_audio(path, stamp)` | path, stamp | same |
| rasterise a glyph | `Font.glyph(code, size)` → `#memo raster(tables, code, size)` | the font's tables, code, size | pure; the bitmap is copied out |
| lay out a text run | `text::layout(font, s, width)` → `#memo` | font tables, text, width | pure |

Until `#memo` is built these functions compute every time — slower, and otherwise the same, which
is the rule.  A library never offers a `GlyphCache`, a `SheetStore` or an `unload`.

## A game's loop

```loft
use machine;
use screen;
use input;
use audio;
use tiles;

fn main() {
  c = machine::caps();
  d = c.displays[0] ?? return;
  f = screen::frame_for(d, 208, 160);
  s = screen::open_screen(d, f, "tile world") ?? return;
  a = audio::open_audio();                    // may be null: then the game is silent
  b = input::bindings_for(c);
  act = input::actions();
  saves = machine::open_saves("tile-world");
  world = load_world(saves);                  // the game's logic package: draws nothing
  view = canvas::canvas(f.width, f.height, 0);
  t = machine::clock(60);
  while s.poll(1) {
    act.feed(s.events(), b);
    for i in 0..t.due() { world.step(act); }
    draw_world(world, view, c.tier);          // the game's frame-tier view
    s.show(view);
  }
  store_world(saves, world);
}                                             // the screen, the audio and the saves end here
```

The same `world` and `step` serve a GPU view (arc G): only `draw_world` and `show` differ.

## Open questions

1. **Several displays** — `open_screen` takes a `Display`, so the API allows it; no host opens two
   yet.
2. **Touch zones in `Bindings`** — screen regions as virtual keys, for a touch-only machine.
3. **Gamepads** — `GamepadButton` / `GamepadAxis` events, and `Bindings` defaults for them.
4. **`Close` and `poll`** — `Close` arrives and `poll` still answers true, so a game can ask
   "save first?"; whether `poll` turns false on its own after a second `Close` is open.
