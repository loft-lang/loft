<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 209 — the game API, 2D tier

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
4. **The game sees its view, not the display.**  It draws and is pointed at in units of its own
   choosing (208×160 for a game of 16×16 tiles); the screen maps those onto the display's own
   pixels — scaled, letterboxed, rotated, masked — and every primitive is rasterised there, at
   the display's resolution (§ Drawing at the machine's resolution).  A game that draws in a
   208×160 view never learns that the panel is 480×800 portrait or the page 1900×1000.
5. **One thread.**  An object is used from the thread that opened it; another thread is refused
   with a message, as graphics' `Window` is.
6. **Optimise freely per platform, invisibly.**  Each platform's section may take every shortcut
   its hardware offers — a pixel accelerator, a native format, a SIMD path, a format kept
   internally — as long as the game cannot tell except by speed.  The check is the same as the
   cache's: the frame `capture` answers and every value a call returns are identical with the
   shortcut on or off.

7. **The library offers, the game chooses.**  Several displays, gamepads, touch, a round screen:
   each is supported well enough to use, and whether a game uses it is the game's decision.
   Nothing here makes that choice for it — and making it is cheap: the defaults already cover
   the common case (`bindings_for` maps keys, a gamepad and touch zones to the same actions;
   `view_for` fits any display), so a game that does nothing still plays everywhere, and taking
   up a capability is a few lines, not a subsystem.

## Drawing at the machine's resolution

There is no frame of a fixed small size.  A game draws **primitives** onto a **surface**, and
each surface rasterises them at its own resolution: a page at the canvas's device pixels, the
Tanmatsu at its 800×480 panel, a desktop at the window's size, an image at the size it is asked
for.  A 16×16 picture is source art, not a unit of the screen: a sprite is placed between
pixels, turned and stretched, and lands on the display's pixels exactly where its transform puts
it.

- **The view.**  The game chooses a coordinate space (`View`: its width and height in units, for
  tile-world 208×160, one unit per source pixel).  Positions, sizes and pivots are fractional —
  fixed point, 1/256 of a unit — so a figure glides, hops and leans smoothly at any resolution.
  The screen maps the view onto the surface with one transform (the largest fit, letterboxed;
  the Tanmatsu's panel rotation in it); the game never sees that transform.
- **The primitives.**  Each is a pure description, drawn in call order:
  - `sprite(sheet, tile, at, pivot, turn, scale_x, scale_y, flips, shade, ghost)`: one tile of a
    sheet as a textured quad, turned by any angle about its pivot and stretched along its axes;
  - `quad(sheet, tile, corners, shades)`: the general textured quad, four corners anywhere and a
    shade at each (a ground tile bent to its corner lifts and lit by its corners: what
    `draw_ground` does at whole pixels today); a sprite is a quad;
  - `fill` and `outline` of a rectangle or a quad in one colour, and `text` from a font sheet.
- **How a pixel is coloured.**  A surface pixel shows the source texel its centre maps back to
  (nearest), so pixel art stays crisp at every scale and angle and a turned figure is a true
  rotation of its texels; transparency is the texel's, and a shade or the ghost's checkerboard is
  applied per texel, so they turn and stretch with the sprite.
- **C holds only what depends on the platform; everything else is loft.**  The drawing
  primitives are a general library, not a game's: what to draw, where the camera goes and when
  it zooms is the game's loft.  And of the library, only what differs between machines is C —
  putting pixels on a panel, the page's GPU calls, the P4's pixel accelerator.  The rasteriser
  is the same on every machine, so its home is loft, compiled through the C backend; the C one
  `tiles` carries now is the reference that port is measured against, pixel for pixel.
- **Backends, one per kind of machine**, each implementing every primitive (rule 6):
  - **the CPU rasteriser**, in a library's C: the reference.  It draws a desktop window, an
    image, an off-screen test surface and the Tanmatsu's panel.  The Tanmatsu rasterises at its
    panel's own pixels too: a view of 208×160 shown about three times larger places and turns
    a figure in steps of a third of a unit, not of a whole one.  A machine may take a simpler
    route where its CPU needs it — simpler arithmetic (fixed-point steps along a span), never a
    coarser resolution — checked against the reference within a stated tolerance;
  - **WebGL**, in a page: the sheets as textures, each primitive a quad drawn at the canvas's
    resolution, nearest-sampled, the shade and the ghost in its shader;
  - later a GPU backend on a desktop, and the P4's pixel accelerator for what it can do (fills,
    copies, conversion; it turns by quarter turns only).
- **One call, as precise as the machine allows.**  A loft call means the same drawing
  everywhere; how exactly it is rasterised is the backend's choice.  A GL-capable machine draws
  the exact rotation, at its full resolution; a slow one may move pixels a cheaper way (whole
  rows and columns sheared, texels in blocks) where that keeps its frame rate, and says so in
  its tolerance.  The game never asks which it got.
- **Simple assets, a capable engine.**  The show comes from the engine — native resolution,
  true rotation, smooth sub-unit motion, a camera that zooms, light and effects on the GPU —
  over small, limited art (16×16 tiles), never from asking the art for more.
- **What stays exact.**  For one surface size the reference's pixels are a pure function of the
  primitives, so tests compare them byte for byte; a GPU backend is compared with the reference
  at the same size, pixel by pixel, within a tolerance measured once (where a texel edge falls
  on a pixel centre).  What is the same on every machine is the drawing, not the pixels.

**Pixels.**  Sources are 32-bit RGBA (PNG).  A packer turns them into what a target draws from —
RGB565 sheets for the Tanmatsu, dithered once; untouched for a page or a desktop — a pure
function of the source and the target, kept by key.  Palette effects (colour cycling) are an
explicit `tiles` feature — an indexed sheet and a palette object — not a pixel format.

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
| a desktop, 2D tier | the source |
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
| `tiles` | tile sheets, maps and the 2D primitives (sprites, quads, fills, text) drawn on a `Surface` at its own resolution, by the CPU rasteriser or a GPU backend; sheets and maps as objects, not integer handles (exists, re-shaped) | `canvas` |
| `screen` | `Screen`: a view on a display, drawn at the display's resolution, and its events | `machine` |
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

**Events** — graphics 0.10's `enum Event`, moved here and widened by the gamepad, so every
library speaks one form.  Positions are in the frame's pixels.

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
  GamepadButton { pub pad: integer, pub button: integer, pub down: boolean },
  GamepadAxis { pub pad: integer, pub axis: integer, pub value: float },   // -1.0 .. 1.0
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

## `screen` — a view on one display

```loft
pub enum Fit { Pixel, Smooth }   // Pixel: whole display pixels per unit, letterboxed; Smooth: fill, keep aspect

pub struct View {
  pub width: integer,             // the game's units across and down
  pub height: integer,
  pub fit: Fit,
}

// The view a game asks for, adjusted to `d`: `want_w` x `want_h` units, its orientation turned to
// the display's, and on a round display the largest view inside the circle.  A pure function of
// its arguments.
pub fn view_for(d: const Display, want_w: integer, want_h: integer) -> View;

pub struct Screen { handle: integer }
fn OpDrop(self: Screen) { … }      // closes the window or releases the panel

// Draw views of `v` on display `d`, at the display's resolution.  On a desktop a window titled
// `title`; on a device the whole display; in a page the canvas.  Null where it cannot open.
pub fn open_screen(d: const Display, v: const View, title: text = "loft") -> Screen?;

// A screen of no display, `w` x `h` pixels kept in memory: for tests and headless runs.
pub fn offscreen(v: const View, w: integer, h: integer) -> Screen;

pub fn poll(self: const Screen, wait_ms: integer = 0) -> boolean;   // false once closed
pub fn events(self: const Screen) -> vector<Event>;                  // since the last call
pub fn begin(self: Screen) -> Surface;             // a new picture, the view's units mapped on
pub fn present(self: Screen);                      // what was drawn since `begin`, on the display
pub fn view(self: const Screen) -> View;
pub fn refit(self: Screen, v: const View);         // after a Resize: the game picks a new view
pub fn capture(self: const Screen) -> Canvas;      // what was last presented, at its own pixels
```

A `Surface` is what `tiles`' primitives draw on (§ Drawing at the machine's resolution): a
screen's, or an image's (`tiles::image_surface(view, w, h)`, read back as RGBA).  `present`
hands the picture to the display; on a page it is the GPU's frame, on the Tanmatsu the panel's.

## `input` — events to actions

```loft
pub struct Bindings { … }                           // action name -> keys, buttons, touch zones
pub fn bindings_for(c: const Caps, actions: const vector<text>) -> Bindings;
                     // the game's actions bound to everything the machine has: keys, the first
                     // gamepad, and touch zones laid over the frame — rebind only to differ
pub fn bind(self: Bindings, action: text, key: integer);
pub fn bind_axis(self: Bindings, axis: text, minus: integer, plus: integer);
pub fn bind_pad(self: Bindings, action: text, button: integer);
pub fn bind_stick(self: Bindings, axis: text, stick_axis: integer);
pub fn bind_zone(self: Bindings, action: text, x: integer, y: integer, w: integer, h: integer);
                                                    // a frame region as a key, for touch

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
  v = screen::view_for(d, 208, 160);
  s = screen::open_screen(d, v, "tile world") ?? return;
  a = audio::open_audio();                    // may be null: then the game is silent
  b = input::bindings_for(c, ["left", "right", "jump", "menu"]);
  act = input::actions();
  saves = machine::open_saves("tile-world");
  world = load_world(saves);                  // the game's logic package: draws nothing
  t = machine::clock(60);
  while s.poll(1) {
    act.feed(s.events(), b);
    for i in 0..t.due() { world.step(act); }
    draw_world(world, s.begin(), c.tier);     // sprites and quads in the view's units
    s.present();
  }
  store_world(saves, world);
}                                             // the screen, the audio and the saves end here
```

The same `world` and `step` serve a GPU view (arc G): only `draw_world` and `show` differ.

## Open questions

1. **`Close` and `poll`** — `Close` arrives and `poll` still answers true, so a game can ask
   "save first?"; whether `poll` turns false on its own after a second `Close` is open.

Several displays (one `Screen` per `Display`), gamepads and touch zones are supported, and each
game decides whether it uses them (rule 7).
