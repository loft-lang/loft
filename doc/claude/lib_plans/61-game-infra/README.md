
# Game Infrastructure — Design

**Status:** ✅ **CLOSED 2026-08-26** (@PLN61, `status:finished`) — six of the seven
G-rows shipped, in the libraries rather than here: G1/G2 `graphics::create_sprite_sheet`
+ `draw_sprite`, G4 `shapes`' overlap tests, G5/G6 `graphics` audio + the `sfx_*` synths,
G7 `doc/brick-buster.html`. FFI.1-4 were superseded in implementation by
`lib_plans/74-ffi-dispatch`. **G3 tilemap rendering is the one remainder** and lives on
as its own ROADMAP row — its neighbours all ship, so it is a composition of `graphics`'
sprite sheet with `stage`'s 2-D batching rather than a new primitive. Kept as a closure
record.

Designs for all game infrastructure, audio, FFI, and tooling items on the
roadmap that don't yet have a dedicated design document.

---

## G1 — Sprite sheet loading

Load an atlas image (PNG) and define named rectangular regions.

```loft
use graphics;

pub struct SpriteSheet {
  texture: integer not null,     // GL texture ID
  width: integer not null,       // atlas width in pixels
  height: integer not null,      // atlas height in pixels
  sprites: vector<SpriteRect>
}

pub struct SpriteRect {
  name: text,
  x: integer not null,
  y: integer not null,
  w: integer not null,
  h: integer not null
}

// Load a sprite sheet from a PNG file.  Regions defined by a companion
// JSON file or added manually with add_sprite().
pub fn load_sheet(path: text) -> SpriteSheet {
  tex = gl_load_texture(path);
  // TODO: read atlas width/height from texture metadata
  SpriteSheet { texture: tex, width: 0, height: 0 }
}

pub fn add_sprite(self: SpriteSheet, name: text, x: integer, y: integer,
                  w: integer, h: integer) {
  self.sprites += [SpriteRect { name: name, x: x, y: y, w: w, h: h }];
}

// Find a sprite by name.  Returns null if not found.
pub fn find(self: const SpriteSheet, name: text) -> SpriteRect {
  for s in self.sprites {
    if s.name == name { return s; }
  }
}
```

**Native:** `gl_load_texture` already works.
**WebGL:** same texture upload path once GL6.4 is done.

---

## G2 — Sprite drawing

Draw a sprite region as a textured quad.  Two modes: 3D billboarded quad
facing the camera, or 2D screen-space overlay.

```loft
// Draw a sprite in screen space (2D overlay, pixel coordinates).
pub fn draw_sprite_2d(sheet: const SpriteSheet, sprite: const SpriteRect,
                      x: integer, y: integer, scale: float) {
  // Compute UV coordinates from sprite rect and atlas size
  u0 = sprite.x as float / sheet.width as float;
  v0 = sprite.y as float / sheet.height as float;
  u1 = (sprite.x + sprite.w) as float / sheet.width as float;
  v1 = (sprite.y + sprite.h) as float / sheet.height as float;
  // Draw a textured quad at screen position using a simple 2D shader
  gl_bind_texture(sheet.texture, 0);
  gl_draw_quad_2d(x, y, sprite.w * scale, sprite.h * scale, u0, v0, u1, v1);
}

// Draw a sprite as a billboard in 3D space (always faces camera).
pub fn draw_sprite_3d(sheet: const SpriteSheet, sprite: const SpriteRect,
                      pos: math::Vec3, size: float, camera: const Camera) {
  // Build a quad perpendicular to the camera view direction
  // Upload as a 2-triangle mesh with UV from sprite rect
}
```

**Requires:** A `gl_draw_quad_2d` native function (small — just a VAO with
4 vertices and dynamic UVs).  Or reuse the fullscreen quad path with custom
UV and position uniforms.

---

## G3 — Tilemap rendering

Render a 2D grid of tiles efficiently.  Tiles are sprite indices into a
sprite sheet.  The renderer batches all visible tiles into one draw call.

```loft
pub struct Tilemap {
  width: integer not null,       // grid columns
  height: integer not null,      // grid rows
  tile_size: integer not null,   // pixels per tile
  tiles: vector<integer>,        // sprite index per cell (row-major)
  sheet: SpriteSheet
}

pub fn tilemap(w: integer, h: integer, tile_px: integer, sheet: SpriteSheet) -> Tilemap {
  Tilemap {
    width: w, height: h, tile_size: tile_px, sheet: sheet,
    tiles: [for _ in 0..w * h { 0 }]
  }
}

pub fn set_tile(self: Tilemap, col: integer, row: integer, sprite_idx: integer) {
  self.tiles[row * self.width + col] = sprite_idx;
}

// Render visible tiles.  cam_x/cam_y is the camera offset in pixels.
pub fn draw(self: const Tilemap, cam_x: integer, cam_y: integer,
            screen_w: integer, screen_h: integer) {
  // Compute visible tile range
  col0 = cam_x / self.tile_size;
  row0 = cam_y / self.tile_size;
  col1 = (cam_x + screen_w) / self.tile_size + 1;
  row1 = (cam_y + screen_h) / self.tile_size + 1;
  // Batch all visible tiles into one vertex buffer, one draw call
  gl_bind_texture(self.sheet.texture, 0);
  for row in row0..row1 {
    for col in col0..col1 {
      if row >= 0 and row < self.height and col >= 0 and col < self.width {
        idx = self.tiles[row * self.width + col];
        if idx > 0 {
          sprite = self.sheet.sprites[idx];
          // Emit quad vertices for this tile
          draw_sprite_2d(self.sheet, sprite,
                         col * self.tile_size - cam_x,
                         row * self.tile_size - cam_y, 1.0);
        }
      }
    }
  }
}
```

**Optimization:** For large maps, batch all quads into a single VBO and draw
once.  Rebuild the VBO only when tiles change.

---

## G4 — 2D collision detection

Simple collision shapes for game objects.  No physics engine — just overlap
testing.

```loft
pub enum Shape {
  Rect(RectShape),
  Circle(CircleShape)
}

pub struct RectShape {
  x: float not null, y: float not null,
  w: float not null, h: float not null
}

pub struct CircleShape {
  x: float not null, y: float not null,
  r: float not null
}

// Test if two shapes overlap.
pub fn collides(a: const Shape, b: const Shape) -> boolean {
  match a {
    Rect(ra) => match b {
      Rect(rb) => rect_rect(ra, rb),
      Circle(cb) => rect_circle(ra, cb)
    },
    Circle(ca) => match b {
      Rect(rb) => rect_circle(rb, ca),
      Circle(cb) => circle_circle(ca, cb)
    }
  }
}

fn rect_rect(a: const RectShape, b: const RectShape) -> boolean {
  a.x < b.x + b.w and a.x + a.w > b.x and
  a.y < b.y + b.h and a.y + a.h > b.y
}

fn circle_circle(a: const CircleShape, b: const CircleShape) -> boolean {
  dx = a.x - b.x; dy = a.y - b.y;
  dist_sq = dx * dx + dy * dy;
  dist_sq < (a.r + b.r) * (a.r + b.r)
}

fn rect_circle(r: const RectShape, c: const CircleShape) -> boolean {
  // Nearest point on rect to circle center
  nx = clamp(c.x, r.x, r.x + r.w);
  ny = clamp(c.y, r.y, r.y + r.h);
  dx = c.x - nx; dy = c.y - ny;
  dx * dx + dy * dy < c.r * c.r
}
```

Pure loft — no native code needed.

---

## G5 — Audio: sound effect playback

Play short audio clips (WAV/OGG).  Native uses a simple audio library;
WebGL uses Web Audio API.

```loft
// Load an audio clip.  Returns a handle (>= 0) or -1 on failure.
pub fn audio_load(path: text) -> integer;
#native "loft_audio_load"

// Play a loaded clip.  volume: 0.0–1.0.
pub fn audio_play(clip: integer, volume: float);
#native "loft_audio_play"

// Stop a playing clip.
pub fn audio_stop(clip: integer);
#native "loft_audio_stop"
```

**Native backend:** `rodio` crate (pure Rust, cross-platform).
**WebGL backend:** `AudioContext.createBufferSource()` via web-sys.

The native function signature is identical for both — the `#native` dispatch
selects the right implementation at compile time.

---

## G6 — Audio: background music with crossfade

Layer on top of G5.  One music track plays at a time; switching crossfades.

```loft
// Start playing background music.  Loops until stopped or replaced.
pub fn music_play(path: text, volume: float);

// Crossfade to a new track over `duration` seconds.
pub fn music_crossfade(path: text, volume: float, duration: float);

// Stop music with a fade-out.
pub fn music_stop(fade_seconds: float);
```

Implemented in loft on top of `audio_load`/`audio_play` — two clips, a
timer, and volume interpolation per frame.

---

## G7 — First playable demo game

**Frame loop:** The browser game loop uses the frame-yield design described in
[WASM_FRAME_YIELD.md § Frame Yield](../../WASM_FRAME_YIELD.md#frame-yield--browser-game-loop-via-interpreter-suspension).
The interpreter pauses at `gl_swap_buffers()` and JavaScript resumes it on
each `requestAnimationFrame`.  Loft game code is identical on native and browser.

A simple game that proves the full pipeline: loft → renderer → WebGL →
browser.  Something a person can play in 30 seconds.

**Candidate: Brick Buster clone**
- One paddle, one ball, rows of colored bricks
- Uses: sprite sheet (G1/G2), collision (G4), audio (G5), renderer (R1)
- Input: left/right arrow keys or mouse
- Win condition: all bricks destroyed
- ~200 lines of loft code

**Why Brick Buster:**
- Exercises 2D rendering, input, collision, and audio
- Simple enough to write in one session
- Immediately recognizable — anyone knows how to play
- The finished game is a single `.html` file shareable via URL

---

## GL6.6 — Keyboard + mouse input via DOM events

The existing native input functions (`gl_key_pressed`, `gl_mouse_x`, etc.)
need WebGL equivalents.

**Native (existing):** winit event polling, key state tracked per frame.

**WebGL:**

```rust
// In webgl.rs — register DOM event listeners on the canvas
fn init_input(canvas: &HtmlCanvasElement) {
    // keydown/keyup → update key state HashMap
    // mousemove → update mouse_x, mouse_y
    // mousedown/mouseup → update mouse_button
    // wheel → update scroll_delta
}
```

The loft-side API is unchanged:
```loft
pub fn gl_key_pressed(key: integer) -> boolean;
pub fn gl_mouse_x() -> float;
pub fn gl_mouse_y() -> float;
pub fn gl_mouse_button() -> integer;
```

Key codes use the same constants (`KEY_W`, `KEY_A`, etc.) on both backends.
The WebGL backend maps DOM `event.code` strings to the same integer codes.

**Implementation (JavaScript side, in gallery.html):**

```javascript
const keys = new Set();
let mouseX = 0, mouseY = 0, mouseBtn = 0;

canvas.addEventListener('keydown', e => {
  keys.add(mapKeyCode(e.code));
  e.preventDefault();
});
canvas.addEventListener('keyup', e => keys.delete(mapKeyCode(e.code)));
canvas.addEventListener('mousemove', e => {
  const r = canvas.getBoundingClientRect();
  mouseX = e.clientX - r.left;
  mouseY = e.clientY - r.top;
});
canvas.addEventListener('mousedown', e => mouseBtn |= (1 << e.button));
canvas.addEventListener('mouseup', e => mouseBtn &= ~(1 << e.button));
canvas.tabIndex = 0;  // make canvas focusable for keyboard events

function mapKeyCode(code) {
  // Map DOM code strings to loft key constants
  // ASCII letters: 'KeyA'→65, 'KeyB'→66, etc.
  // Special: ArrowUp→128, ArrowDown→129, ArrowLeft→130, ArrowRight→131,
  //          ShiftLeft/Right→132, ControlLeft/Right→133
  if (code.startsWith('Key')) return code.charCodeAt(3);
  if (code.startsWith('Digit')) return code.charCodeAt(5);
  const special = { ArrowUp:128, ArrowDown:129, ArrowLeft:130, ArrowRight:131,
                    ShiftLeft:132, ShiftRight:132, ControlLeft:133, ControlRight:133,
                    Space:32, Enter:13, Escape:27, Tab:9 };
  return special[code] || 0;
}

loftHost.gl_key_pressed = (code) => keys.has(code);
loftHost.gl_mouse_x = () => mouseX;
loftHost.gl_mouse_y = () => mouseY;
loftHost.gl_mouse_button = () => mouseBtn;
```

**Depends on:** Frame yield (FY.1) — without yielding between frames, DOM events
are never processed.  With frame yield, the browser handles events between
`resume_frame` calls.

**Unlocks:** Example 21 (keyboard camera), and all future interactive games.

---

## W1.1 — Single-file HTML export

`loft --html game.loft` produces a self-contained HTML file with the WASM
binary base64-encoded inline.

```
┌─── game.html ────────────────────────────┐
│ <!DOCTYPE html>                          │
│ <html>                                   │
│ <body>                                   │
│   <canvas id="game" width="800" ...>     │
│   <script>                               │
│     const wasmBytes = atob("AGFz...");   │
│     // instantiate WASM, bind WebGL      │
│     // call main()                       │
│   </script>                              │
│ </body>                                  │
│ </html>                                  │
└──────────────────────────────────────────┘
```

**Build steps** (in loft's main.rs):
1. Compile the .loft file to WASM (`--native-wasm` internally)
2. Base64-encode the .wasm binary
3. Embed in an HTML template with a small JS loader (~30 lines)
4. Write the .html file

The JS loader:
- Decodes the base64 WASM
- Creates a WebGL2 canvas
- Instantiates the WASM module with the WebGL host bridge
- Calls the exported `main()` function
- Runs `requestAnimationFrame` loop

**Result:** One file.  Drop it on any web server or open locally.

---

## FFI.1 — Generic type marshaller

> **Superseded by [`../25-ffi-dispatch/`](../74-ffi-dispatch/README.md)** (2026-05-27).
> The dedicated plan resolves the linking decision (bridge lives library-side,
> generated by a `#[loft_native]` proc-macro) and supersedes the design below
> + FFI.3.  Kept here as the originating sketch.


Currently, each `#native` function needs a hand-written Rust wrapper that
unpacks loft types from the stack.  The marshaller auto-generates this from
the loft function signature.

**Design:**

Parse the `#native` function's parameter types and generate the unpacking
code at compile time:

```
fn my_func(a: integer, b: text, c: vector<single>) -> integer;
#native "my_func"
```

Generates (conceptually):
```rust
extern "C" fn bridge_my_func(state: &mut State) {
    let a: i32 = state.pop_int();
    let b: &str = state.pop_text();
    let (c_ptr, c_len): (*const f32, u32) = state.pop_vector();
    let result = my_func(a, b, c_ptr, c_len);
    state.push_int(result);
}
```

**Type mapping:**

| Loft type | Rust ABI | Pop/Push |
|---|---|---|
| `integer` (plain, i64) | `i64` | `pop_long` / `push_long` |
| narrow int (`i32`/`u8`/…) | `i32` | `pop_int` / `push_int` |
| `float` | `f64` | `pop_float` / `push_float` |
| `single` | `f32` | `pop_single` / `push_single` |
| `boolean` | `bool` | `pop_bool` / `push_bool` |
| `text` | `(*const u8, usize)` | `pop_text` / `push_text` |
| `vector<T>` | `(*const T, u32)` | `pop_vector` / `push_vector` |
| `Canvas` etc. | `DbRef` | `pop_ref` / `push_ref` |

**Implementation:** In `src/native.rs`, replace the hand-written dispatch
table with a generated one.  The generator reads the `#native` declaration's
parameter types and emits the bridge function.

### Design decision (2026-05-27 — perf framing + @PLAN48 integer width)

Refines the above after the @PLAN48 integer-width work hit the hand-written
arm-explosion in `src/extensions.rs::dispatch_call` (72 `(&[ArgT], ret)` arms,
each `transmute`-ing the fn pointer; every new signature/width needs a loft-core
edit).

- **Integer width:** the table above is outdated — there is no `long`, and a
  plain loft `integer` is **64-bit → `i64`** (per @PLAN48 / @P370), not `i32`.
  Only an explicit narrow alias (`i32`/`u8`/…) maps to a smaller width.  The
  generated marshaller must cast loft's 8-byte cell to the impl's **real**
  param/return width at the boundary (`args[i].as_i64() as i32` for an i32 impl),
  derived from the impl signature — so any width works with no loft-core arm.
- **Perf framing (why generate, not abstract at runtime):** `--native` already
  links the library C-style — direct typed calls, args in registers, **zero
  marshal** — and is the perf path; this work does **not** touch it.  The marshal
  exists only on `--interpret`, which `dlopen`s `.so`s at runtime (cannot link)
  and is the non-perf, tree-walking path.  So optimise the generated marshaller
  for **maintenance**, while keeping calls **direct-typed** (no slower than the
  current hand-arms).
- **Chosen mechanism:** a `#[loft_native]` proc-macro reads each native fn's
  **real Rust signature** and generates a **direct-call** dispatch entry;
  `loft-ffi-build` (already source-scans `#native` → `loft_register!`) wires them
  per-library.  This deletes the 72-arm match (this is FFI.3) and lets libraries
  own their own FFI typing — new signatures never touch loft-core.
- **Rejected:** a uniform `LoftCell` arg-array shim (adds per-call indirection
  for no benefit on the non-perf path) and `libffi` (C dependency + per-call
  prep) — both strictly slower than generated direct calls.
- **Sequencing:** @PLAN48 P1b adds the ~20 `(I64, …)` arms by hand as a one-time
  unblock; this work generates them and removes the recurrence.

---

## FFI.2 — Generic cdylib loader

Instead of hand-registering each native function in `register_v1()`, scan
the shared library's export table automatically.

```rust
fn load_native_lib(path: &str) -> HashMap<String, extern "C" fn(...)> {
    let lib = libloading::Library::new(path)?;
    let register: fn(&mut Registry) = lib.get(b"loft_register")?;
    let mut registry = Registry::new();
    register(&mut registry);
    registry.functions
}
```

The native library exports one function `loft_register` that calls
`registry.add("name", fn_ptr)` for each function.  No more manual `reg!`
macros.

---

## FFI.3 — Eliminate per-function glue

With FFI.1 (type marshaller) and FFI.2 (auto-loader), the hand-written
bridge functions in `native.rs` and `extensions.rs` can be deleted.  Each
native function is just:

```loft
fn my_func(a: integer, b: text) -> integer;
#native "my_func"
```

And the Rust side is just:
```rust
#[no_mangle]
pub extern "C" fn my_func(a: i32, text: *const u8, text_len: usize) -> i32 {
    // pure Rust logic, no loft internals
}
```

---

## FFI.4 — Zero-boilerplate native function guide

A documentation page in `doc/claude/PACKAGES.md` with:

1. How to write a Rust function
2. How to declare it in loft with `#native`
3. How to build a `cdylib` crate
4. How to register it via `loft.toml`
5. Complete working example (3-function library)

---

## W-warn — Developer warnings

Implemented in the parser's second pass.  Each warning is a simple pattern
check; no flow analysis needed.

| Warning | Detection | Location |
|---|---|---|
| Comparison always true/false | `expr op null` where `expr_not_null` flag is set | `operators.rs` (partially exists) |
| Unnecessary parentheses | `if (expr) {` — outer parens around if condition | `control.rs` |
| Empty loop/if body | `Block` with zero operators or only `Value::Null` | `control.rs` |
| Shadowed variable | `change_var_type` called with different type | `variables/mod.rs` |
| Unused import | `use lib;` with no `lib::` references in the file | `parser/mod.rs` |
| Identical if/else branches | Deep-compare `then_val` and `else_val` in `Value::If` | `control.rs` |
| Division by literal zero | `OpDivInt/Float/Long` with `Value::Int(0)` or `Value::Float(0.0)` as divisor | `operators.rs` |

Each warning is gated behind `!self.first_pass` and emits via the existing
`diagnostic!(lexer, Level::Warning, ...)` mechanism.  No new infrastructure
needed.
