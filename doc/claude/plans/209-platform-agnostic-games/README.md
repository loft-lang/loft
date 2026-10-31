<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 209 — Platform-agnostic game libraries

## Status

Next — designed, nothing built.  Issue: [@PLN209](https://github.com/loft-lang/plans/issues/209).
The small-machine stack (`screen`, `tiles`, `input`, `audio` in loft-libs-game and
loft-libs-graphics-tiles-rect) runs on the Tanmatsu and an X11 desktop; this plan carries it to
the browser, Windows and macOS first (arc F), then puts `graphics` on the same platform layer and
gives it a GPU boundary that Metal, Vulkan and a console backend can sit under (arc G, which takes
over [@PLN72](https://github.com/loft-lang/plans/issues/72)).

| Phase | State |
|---|---|
| F1–F12 | Open |
| G1–G4 | Open — the 3D work (graphics, stage, the moros editor) continues alongside arc F; G reshapes its boundary |

## Goal

A game written against loft's game libraries runs unchanged on every supported machine, and
knows only two things about the machine: the screen it draws to (size, shape, orientation,
density) and how much drawing it can afford (a performance tier).

## Why

- **Short-term game writing is for small machines.**  A Tanmatsu game is cheap to write, and the
  same game as a web page is what reaches people.  That needs one source to run on both, and on a
  desktop for development.
- **The small-format game is a product, not a reduced port.**  The games are meant to look cool
  eventually, in 3D; on the way there, their frame-tier versions already serve old-school gamers
  with good games.  So a frame-tier view is held to the same bar as a game's GPU view — complete
  and fun to play — and it ships first.
- **Today a game is tied to the machines its author tested.**  `screen` opens nothing in a browser,
  on Windows or on macOS; keys come from two places (`input`'s `c/keys.c` and graphics'
  `Window.key_down`); the X11 window is written twice (`screen/c/screen.c`, graphics'
  `c/graphics_gl.c`).
- **The GPU games are tied to GL.**  moros' `editor_client.loft`, dryopea's `ground_gl.loft` /
  `entity_gl.loft` and MED's `planet_view.loft` carry their own `#version 330 core` shaders,
  uniforms set by name (57 in moros), GL state calls and fixed window sizes — measured by the
  survey in § Where the games stand.  None of that moves to Metal, a console or a machine with no
  GPU.

## The shape

The frame tier's loft API — packages, objects, what is cached — is designed in
[API.md](API.md).

**Ask, never detect.**  A library answers what the machine HAS, never which machine it is:

| Question | Answer |
|---|---|
| the screen | width and height in pixels, orientation, pixel density, its shape (a rectangle, or round with a diameter) |
| input | has keys, has a pointer, has touch, has a gamepad — and the events, in one form |
| drawing budget | a performance tier: `low` (a CPU frame, a small device), `mid`, `high` |
| storage | a place to keep saves, by name |

A game that branches on these runs on a machine nobody wrote it for.  No library exposes a
platform name to a game.

**Two tiers, one platform layer.**

- **Tier F — the frame.**  The game draws a frame on the CPU (`tiles`, `Canvas`, text) and
  `screen` shows it.  Runs everywhere, including the Tanmatsu, which has no 3D GPU.
- **Tier G — the GPU.**  `graphics` and `stage`.  Desktops and the browser today.
- **The platform layer** under both: display, input events, audio output, time, storage — one C
  section per machine (the [HARDWARE_BACKENDS.md](../../HARDWARE_BACKENDS.md) rule: one loft
  API, platform sections only in C).  Each machine's window and input are written once.

## Where the games stand

| Game | Tier | Platform knowledge it carries |
|---|---|---|
| tile-world-game | F (`screen`, `tiles`, `input`, `audio`) | raw key codes (`game.loft:122`), a fixed 208×160 frame |
| dryopea | G, graphics 0.9 `gl_*` | own GLSL and GL enums, a 960×720 window used as the layout size |
| moros (editor client) | G, graphics 0.9 `gl_*` | own GLSL, 57 named uniforms, depth toggles, a Linux font path |
| MED (planet view) | G, graphics 0.9 `gl_*` | own GLSL, bare key numbers, a 1100×800 window |
| hexbody | none (renders a canvas to PNG) | a `/tmp` path |

tile-world-game is the model arc F generalises; the G games move when arc G gives them a
boundary to move to.

**Every game gets a small-format version.**  Most of a game is its rules — the world, the units,
the turns, the economy — and those do not need 3D.  So a game keeps its logic in packages that
draw nothing, and has one or more VIEWS over the same state: a frame-tier view that runs on the
Tanmatsu and in a page, and a GPU view where the game wants one.  Dryopea is the first: it is
fine without fancy graphics, and its small-format version is F11's target.  moros' simulation
already stands apart (`moros_sim` uses `graphics` only for its math types and `Canvas`); MED and
the planned games follow the same split.  The games are their own agents' to change — this
stream provides what the split needs (F10) and verifies it on a game's testbed; the game's agent
moves its code.

## Arc F — the frame tier on every machine (first)

Every phase is verified on the interpreter and the C backend; a browser phase on the C writer's
page (`--native-c --html`) as well as the interpreter's.  The Tanmatsu is checked by the method
[TANMATSU.md](../../TANMATSU.md) § Verified, and how describes.

**F1 — capabilities in `screen` (S).**  `screen` answers the table above for the machine it opened
on (X11 and the Tanmatsu), and `open` takes the frame size the game picked from them.
- Goes red when: an offscreen screen of a given display answers other than the values the test
  set; the Tanmatsu answers anything but 480×800 portrait, `low`, keys and no pointer.

**F2 — `screen` in the browser (M).**  A canvas: the RGB565 frame converted and drawn with
`putImageData`, scaled with nearest-neighbour, keys and pointer from the page's events.  Both
writers' pages.
- Goes red when: a scripted frame read back from the headless page differs by one byte from the
  same frame offscreen (graphics' `browser/check.sh` pattern); a dispatched key does not arrive.

**F3 — `screen` on Windows and macOS (M).**  Windows: a GDI window and `StretchDIBits`.  macOS: an
`NSWindow` and a bitmap through the Objective-C runtime, as graphics G11 opens its window.
- Goes red when: the window's pixels read back differ from the offscreen frame; measured on the
  `windows-probe.yml` and `macos-probe.yml` runners.

**F4 — one `input` (S).**  `input` reads the platform layer's events on every machine — the
window's keys, the Tanmatsu's keys, the page's events, the pointer and touch — through `screen`,
not through graphics' `Window` or a second key reader.  Key codes are named (`KEY_UP`), never
numbers in a game; bindings carry a default per capability (a Tanmatsu has no mouse).
- Goes red when: an injected event sequence (XTest, a dispatched page event, the Tanmatsu's
  scancodes replayed) gives a different action state on any machine.

**F5 — screen size and shape (S).**  A game asks the display and picks its frame; `screen` scales
by a whole number, letterboxes, rotates into a portrait panel, and masks a round display.
- Goes red when: a matrix of displays (landscape, portrait, square, round; small to 4K) places the
  frame anywhere but the hand-computed rectangle.

**F6 — the performance tier (S).**  `tier()` answers per machine, from a declared table first; a
game chooses detail from it (frame size, effects, particle counts).
- Goes red when: a machine answers another tier than its table row; a demo draws the same frame
  for two tiers.

**F7 — saves and frame pacing (S).**  `save(name, bytes)` / `load(name)`: a file under the user's
data directory on a desktop, the page's storage in a browser, the SD card or flash on the
Tanmatsu.  One frame clock: `poll` waits for the display's next frame where the machine has one.
- Goes red when: a save written then read answers other bytes on any machine; a 60 Hz loop drifts
  by more than a frame per minute against the clock.

**F8 — the portability lint (S).**  A gate, not a report: a game package that names a platform
(a target check, an absolute OS path, a bare key number, GLSL in tier-F code) fails its library
CI with the line and the capability to ask instead.
- Goes red when: a planted platform check in a fixture game passes the gate.

**F9 — a reference game on every machine (S).**  tile-world-game (or a successor) moved onto
F1–F7, published, and built for the Tanmatsu, as a web page and for three desktops from one
source.
- Goes red when: any build of it differs from the others in a scripted run's frames.

**F10 — game logic apart from its view (M).**  What a game's logic needs leaves the display
libraries: the math types (`vec3`, `Mat4`, …) and the pure geometry move from `graphics` to a
library that draws nothing, which `graphics` re-exports so no caller breaks.  The portability
gate (F8) gains a rule: a package a game marks as logic may not depend on `screen`, `graphics`,
`stage` or `tiles`.
- Goes red when: `moros_sim`, the `hex_*` libraries or a fixture logic package still pull in a
  display library; a 0.11 graphics caller of the math types stops compiling.

**F11 — Dryopea in the frame tier (M, with dryopea's agent).**  Dryopea's rules on a logic
package, and a frame-tier view (`tiles` or `Canvas`, sized from F5's display) beside its GL view,
running on the Tanmatsu, as a web page and on a desktop.  This stream supplies the libraries and
reports gaps; dryopea's agent moves the code.
- Goes red when: a scripted game played through both views ends in a different state; the
  frame-tier build does not start on any of the five machines.

**F12 — a hex world editor in the frame tier (M).**  An editor for `hex_world` worlds — terrain,
places, ways — drawn with `tiles` / `Canvas`, so it runs on the Tanmatsu, in a page and on a
desktop.  It writes the world in `hex_world`'s stored format, the one the 3D games read, so a
world made in the small editor is the world the 3D version loads later.
- Goes red when: a world saved by the editor and loaded by a 3D game's loader answers another
  `hex_world` state than the editor held; a scripted edit session gives a different world on any
  machine.

## Arc G — the GPU tier on the same layer

The 3D work does not wait for arc F: graphics, stage and the moros editor keep being built — the
editor is a large gain on its own.  What arc G orders is the BOUNDARY: G1 and G2 reshape where the
GPU tier meets the platform layer, and are taken up when arc F's layer exists and before a second
GPU backend is written.  Takes over @PLN72, whose `render.loft` and
`gl_*` premises graphics 0.10 already removed.

**G1 — one platform layer under `graphics` (M).**  graphics' window, input and present use arc F's
platform sections; graphics' own X11, Cocoa and browser windows go.  Windows' GL context (graphics
G10) is written here, once.
- Goes red when: graphics' suite and its 0.9 pixel comparisons change on any machine.

**G2 — the GPU boundary (L).**  `Shader` / state calls become pipelines (shader, vertex layout,
blend, depth, cull together), render passes (a target, how it is loaded and stored) and uniform
blocks bound by slot from a typed loft struct — the model Metal, Direct3D 12, Vulkan and the
console APIs share.  GL and WebGL2 implement it by setting state when a pipeline is bound.
- Goes red when: stage and every graphics example draw other pixels than on 0.11.

**G3 — the shader decision, then Metal (L).**  One shader source for every backend — open question
1.  Metal is the first second backend: two very different APIs expose a bad boundary fastest.
- Goes red when: a scene draws other pixels on Metal than on GL beyond a measured tolerance.

**G4 — further backends (L each).**  Vulkan (Linux, Android, Steam Deck); a console backend in a
private repo, implementing the same C interface under its maker's licence; Direct3D only when Xbox
is a target, else through ANGLE.

## Open design questions

1. **The shader source** (G3).  (a) Vulkan-style GLSL compiled ahead of time to SPIR-V and
   translated to MSL / HLSL / GLSL ES, outputs committed as derived files; (b) shaders in a subset
   of loft, compiled by loft to each target's language and to C for a software path; (c) a fixed
   set of materials the library provides, no game shaders.  @PLN72's WGSL + `naga` is out: a Rust
   crate, against the C-only direction.
2. **The tier table** (F6) — declared per machine, measured at start-up, or both.
3. **Round displays** (F5) — mask only, or a shape a game can ask for its layout.
4. **Gamepads** (F4) — in F4, or when a machine with one is a target.

## Cross-arc dependencies

- [@PLN204](https://github.com/loft-lang/plans/issues/204) — the Tanmatsu: its display, keys and
  audio sections are arc F's device half.
- [@PLN208](https://github.com/loft-lang/plans/issues/208) — graphics' object API; its G10
  (Windows GL) moves into G1 here.
- [@PLN72](https://github.com/loft-lang/plans/issues/72) — absorbed as G2/G3.
- [@PLN203](https://github.com/loft-lang/plans/issues/203) — the C backend: every phase is checked
  on it, and the browser pages are the C writer's.

## Documentation

- [HARDWARE_BACKENDS.md](../../HARDWARE_BACKENDS.md) — the platform layer and the capability
  questions, as the method for a new machine.
- [TANMATSU.md](../../TANMATSU.md) — what runs there after F1–F7.
- The `screen`, `input`, `audio`, `tiles` READMEs and guides — the capability API, one example game
  loop; and a "writing a game for every machine" guide in loft-libs-game.
- `make libcatalogue` regenerated; [ROADMAP.md](../../ROADMAP.md) row; @PLN72 closed into this
  plan.
- The library CI's portability gate (F8) named in
  [LIBRARY_AUTHORING.md § The library contract](../../LIBRARY_AUTHORING.md).

## See also

[HARDWARE_BACKENDS.md](../../HARDWARE_BACKENDS.md) · [TANMATSU.md](../../TANMATSU.md) ·
[PACKAGES_C_BINDING.md](../../PACKAGES_C_BINDING.md) ·
[@PLN208 graphics objects](../208-graphics-objects-c/README.md)
