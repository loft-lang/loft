
# Web Example Gallery & Unified Rendering Design

Design for presenting loft graphics examples as interactive web pages and
unifying the rendering backend across native OpenGL, WebGL, and GLB export.

---

## Project goals

### Primary: browser games for the wider public

Loft's main graphics goal is enabling **small games that run in a browser**.
The audience is the general public — people who click a link and immediately
play, with no install, no toolchain, no GPU driver.  WebGL2 in every modern
browser is the delivery platform.

This means:

- **WebGL is the first-class target.**  Every rendering feature must work in
  the browser.  If something works natively but not in WebGL, it is incomplete.
- **The game loop, input, audio, and rendering all run in WASM+WebGL.**
  The loft interpreter (or native-compiled WASM) drives the game; the browser
  provides the display surface and input events.
- **Share and play in one click.**  A loft game compiles to a single `.html`
  page (or a small set of static files) that anyone can host on GitHub Pages,
  itch.io, or any web server.

### Secondary: native OpenGL for enthusiasts

Desktop rendering via native OpenGL is supported for developers who want:

- **Low-latency, full-screen rendering** without browser overhead.
- **LearnOpenGL-style tutorials** that teach graphics programming concepts
  with direct GL access (the existing 23 examples serve this role).
- **Offline tools** — headless GLB export, procedural texture generation,
  batch rendering pipelines.

Native OpenGL is not a prerequisite for browser games.  The two paths share
the same loft-level API but compile to different backends.

### Design consequence

Every abstraction is designed **WebGL-first, native-compatible**:

| Decision | Rationale |
|---|---|
| Shader version patching (330 → 300 es) | One source, both targets |
| No GL extensions beyond WebGL2 core | Browser must work |
| Frame loop via `render.frame()` not `while true` | Maps to `requestAnimationFrame` |
| Input via `gl.key_pressed()` not OS-specific APIs | Maps to DOM events |
| Audio via a future `audio.loft` module | Maps to Web Audio API |
| Asset loading via virtual filesystem | Maps to fetch + IndexedDB |

---

## Contents

- [Example gallery](#example-gallery)
- [Unified rendering abstraction](#unified-rendering-abstraction)
- [Scene-level API](#scene-level-api)
- [Low-level GL abstraction](#low-level-gl-abstraction)
- [Backend implementations](#backend-implementations)
- [Migration path](#migration-path)
- [Implementation plan](#implementation-plan)

---

## Motivation

### Problem 1: Examples are invisible

The 23 graphics examples only run on machines with a display, a Rust toolchain,
and the native graphics library compiled.  There is no way for a casual visitor
to see what loft can do without cloning the repo and building everything.

### Problem 2: Three rendering paths with no shared abstraction

| Path | API | Runs on | State |
|---|---|---|---|
| GLB export | `glb::save_scene_glb()` | headless / any viewer | working |
| Native OpenGL | `gl_create_shader`, `gl_draw`, etc. | desktop with GPU | working |
| WebGL | — | browser | not started |

Each path has its own calling convention.  A program that renders live on
desktop cannot export a GLB, and neither can run in a browser.  Adding WebGL
as a third independent path would triple the maintenance burden.

### Solution

1. A **web gallery** that presents each example as a page with source, description,
   and either a live WebGL canvas or a static screenshot.
2. A **unified rendering abstraction** (`render.loft`) that sits above all three
   backends, so one program can render natively, in WebGL, or export GLB with
   no code changes.

---

## Example gallery

### Structure

```
docs/examples/
  index.html              — gallery index with thumbnails and descriptions
  01-hello-window.html    — per-example page
  02-hello-triangle.html
  ...
  assets/
    screenshots/          — PNG screenshots of each example (generated offline)
    style.css             — shared gallery styling
    gallery.js            — thumbnail grid, search, category filter
```

### Per-example page layout

```
┌─────────────────────────────────────────────────┐
│  ← Back to Gallery          01 - Hello Window   │
├────────────────────────┬────────────────────────┤
│                        │                        │
│   Live WebGL canvas    │   Source code           │
│   (or screenshot if    │   (syntax highlighted,  │
│    no WebGL support)   │    scrollable)          │
│                        │                        │
├────────────────────────┴────────────────────────┤
│  Description (from file header comments)        │
│  Loft constructions used (from header)          │
│  Controls: [Run] [Stop] [Reset] [Export GLB]    │
└─────────────────────────────────────────────────┘
```

### Generation

A build script (`scripts/build-gallery.py` or `.loft`) reads each example file,
extracts the header documentation, generates the HTML pages, and optionally
captures screenshots via headless rendering.

For examples that use the unified rendering API (see below), the WebGL canvas
is live — the loft WASM runtime executes the example in the browser.  For
examples that use low-level GL calls directly, a static screenshot is shown
with a note that native execution is required.

### Index page

Grid of cards, one per example.  Each card shows:
- Thumbnail (screenshot or placeholder)
- Number and title
- One-line description
- Category badge (basics / lighting / textures / advanced / scene)

Categories are derived from the example number ranges:
- 01-04: Basics (window, triangle, shaders, textures)
- 05-09: Transforms & Lighting
- 10: 2D Canvas
- 11: Scene Graph / GLB
- 12-16: Advanced Rendering (lights, depth, blending, culling, shadows)
- 17-19: Post-Processing, PBR, Complete Scene
- 20-23: Textures, Input, Wireframe, Cleanup

---

## Unified rendering abstraction

### Design principles

1. **One program, three outputs** — the same scene description renders natively,
   in WebGL, or exports to GLB without code changes.
2. **Progressive disclosure** — simple scenes use `render.render_loop(scene)`;
   custom effects drop down to `gl.*` calls that work on both native and WebGL.
3. **No shader code in user programs** — built-in PBR shader handles materials,
   lights, and shadows.  Custom shaders are opt-in for advanced users.
4. **Canvas-first for 2D** — 2D drawing stays on `Canvas` with `draw_text`,
   `fill_rect`, etc.  Canvases upload to textures via `gl_upload_canvas`.

### Architecture

```
User code
    │
    ├── Scene API (scene.loft)        ← declarative: meshes, materials, lights
    │       │
    │       ▼
    ├── Renderer (render.loft)        ← drives the render loop, owns shaders/FBOs
    │       │
    │       ▼
    ├── GL abstraction (gl.loft)      ← thin wrapper: same API for native + WebGL
    │       │
    │       ├── Native backend         ← glutin + gl crate (existing)
    │       └── WebGL backend          ← web-sys WebGl2RenderingContext
    │
    └── GLB export (glb.loft)         ← file output, no GPU needed
```

---

## Scene-level API

Reuses the existing types from `scene.loft` — no new types needed.

```loft
use scene;
use render;

fn main() {
  s = scene::Scene { name: "demo" };
  // ... add meshes, materials, nodes, lights, camera ...

  // Option A: render live (native or WebGL, auto-detected)
  r = render::create(800, 600, "Demo");
  r.run(s, cam);
  r.destroy();

  // Option B: export GLB
  glb::save_scene_glb(s, "demo.glb");

  // Option C: both
  r = render::create(800, 600, "Demo");
  for _ in 0..5000 {
    if !r.frame(s, cam) { break }
  }
  r.destroy();
  glb::save_scene_glb(s, "demo.glb");
}
```

### Renderer struct

```loft
pub struct Renderer {
  width: integer not null,
  height: integer not null,
  // Internal: shader handles, shadow FBO, uploaded mesh cache
  pbr_shader: integer not null,
  shadow_shader: integer not null,
  shadow_fbo: integer not null,
  shadow_tex: integer not null,
  shadow_size: integer not null,
  mesh_vaos: vector<integer>,
  mesh_counts: vector<integer>,
  start_time: integer not null
}

// Create renderer with window (native) or canvas (WebGL).
pub fn create(width: integer, height: integer, title: text) -> Renderer

// Render one frame. Returns false when close requested.
pub fn frame(self: Renderer, scene: const Scene, camera: const Camera) -> boolean

// Convenience: render loop until close.
pub fn run(self: Renderer, scene: const Scene, camera: const Camera)

// Seconds since creation.
pub fn elapsed(self: Renderer) -> float

// Destroy resources and close window.
pub fn destroy(self: Renderer)
```

### Built-in rendering pipeline

`frame()` executes:

1. **Upload** — on first call, flatten each mesh to VAO (cached in `mesh_vaos`)
2. **Shadow pass** — if scene has directional lights, render depth from light POV
   into shadow FBO using orthographic projection sized to scene bounding box
3. **Color pass** — for each node: bind material uniforms, bind mesh VAO,
   set MVP from camera, draw with PBR shader that samples shadow map
4. **Swap** — `gl_swap_buffers()` (native) or `requestAnimationFrame` return (WebGL)

---

## Low-level GL abstraction

For examples that need custom shaders or multi-pass rendering, a thin `gl`
module wraps both native OpenGL and WebGL2 behind identical loft functions.

### Current native-only functions → unified

| Function | Native impl | WebGL impl |
|---|---|---|
| `gl.create_window(w, h, title)` | glutin + winit | `document.getElementById` + `getContext("webgl2")` |
| `gl.create_shader(vert, frag)` | `glCreateProgram` | `createProgram` |
| `gl.upload_vertices(data, stride)` | `glBufferData` | `bufferData` |
| `gl.draw(vao, count)` | `glDrawArrays` | `drawArrays` |
| `gl.set_uniform_mat4(prog, name, mat)` | `glUniformMatrix4fv` | `uniformMatrix4fv` |
| `gl.set_uniform_vec3(prog, name, x,y,z)` | `glUniform3f` | `uniform3f` |
| `gl.set_uniform_float(prog, name, val)` | `glUniform1f` | `uniform1f` |
| `gl.set_uniform_int(prog, name, val)` | `glUniform1i` | `uniform1i` |
| `gl.bind_texture(tex, unit)` | `glBindTexture` | `bindTexture` |
| `gl.upload_canvas(data, w, h)` | `glTexImage2D` | `texImage2D` |
| `gl.clear(color)` | `glClear` | `clear` |
| `gl.swap_buffers()` | glutin swap | no-op (rAF handles it) |
| `gl.poll_events()` | winit poll | check close flag |
| `gl.destroy_window()` | drop context | no-op |

### Shader differences

| Feature | OpenGL 3.3 | WebGL2 (GLSL ES 3.0) |
|---|---|---|
| Version line | `#version 330 core` | `#version 300 es` |
| Precision | not needed | `precision mediump float;` required |
| Attributes | `in` | `in` (same) |

The renderer auto-prepends the correct version/precision header based on
the active backend.  User-written shaders use `#version 330 core` and the
WebGL backend patches them.

---

## Backend implementations

### Native (existing)

`lib/graphics/native/src/lib.rs` — glutin + gl crate.  Already working.
Functions registered via `#native` annotations in `graphics.loft`.

### WebGL (new)

`lib/graphics/native/src/webgl.rs` (or separate wasm-only crate).

Implementation via `web-sys`:
- `WebGl2RenderingContext` for all GL calls
- `HtmlCanvasElement` for the drawing surface
- `requestAnimationFrame` via `wasm_bindgen::closure::Closure` for the frame loop
- Keyboard/mouse events via `addEventListener` on the canvas

Compile with: `cargo build --target wasm32-unknown-unknown --features webgl`

The `#native` functions in `graphics.loft` compile to either the native or
WebGL implementation based on the target.  From loft's perspective, the API
is identical.

### GLB (existing)

`lib/graphics/src/glb.loft` — pure loft, no GPU.  Already working with
scene graph, materials, lights, and KHR_lights_punctual.

---

## Migration path

### Phase 1: Renderer layer (no WebGL yet)

Build `render.loft` on top of the existing native GL functions.  The renderer
compiles built-in PBR + shadow shaders, manages FBOs, and exposes
`create/frame/run/destroy`.  Examples 11 and 19 become ~20 lines each.

**Deliverable:** `render.loft` + updated examples using `render.run()`.

### Phase 2: Static gallery

Generate `docs/examples/index.html` and per-example pages from the .loft files.
Screenshots captured via headless rendering or manual.  Source code shown with
syntax highlighting.  No live execution yet.

**Deliverable:** Static HTML gallery deployable to GitHub Pages.

### Phase 3: WASM language examples

Wire up the existing `compile_and_run()` WASM entry point to a browser page.
The 30 language examples (tests/docs/) run live in the browser — text output
shown in a console panel.  No graphics yet.

**Deliverable:** Language example pages with live execution.

### Phase 4: WebGL backend

Implement the WebGL2 backend behind the same `gl.*` API.  The renderer and
all examples that use the unified API work in the browser.  Examples that use
raw `gl_*` calls need the version/precision patching.

**Deliverable:** Live WebGL rendering of graphics examples in the gallery.

### Phase 5: Export integration

Add [Export GLB] button to each gallery page — runs the example's scene
construction in WASM, serializes to GLB via `glb.loft`, and triggers a
browser download.

**Deliverable:** Browser-based GLB export for any scene example.

---

## Implementation plan

| Step | What | Files | Depends on |
|---|---|---|---|
| W1 | `render.loft` — Renderer struct, built-in shaders | `lib/graphics/src/render.loft` | — |
| W2 | `render.frame()` — shadow + PBR passes | `render.loft` | W1 |
| W3 | Update 11-scene-graph to use renderer | example | W2 |
| W4 | Gallery build script + index.html | `scripts/`, `docs/examples/` | — |
| W5 | Per-example HTML pages (static) | `docs/examples/` | W4 |
| W6 | Screenshot capture (headless or manual) | `docs/examples/assets/` | W5 |
| W7 | WASM build + language example runner | `ide/`, `src/wasm.rs` | — |
| W8 | WebGL2 backend (`webgl.rs`) | `lib/graphics/native/src/` | — |
| W9 | Shader version patching (330 → 300 es) | `render.loft` or native | W8 |
| W10 | Live WebGL in gallery pages | `docs/examples/` | W8, W5 |
| W11 | GLB export button in gallery | `docs/examples/` | W7 |

Steps W1-W3 deliver the renderer abstraction (desktop-only).
Steps W4-W6 deliver the static gallery (no execution).
Steps W7-W11 deliver the interactive web experience.

---

## WebGL compatibility matrix — all 24 examples

Completed items (GL6.1–GL6.5, GAL.2) deliver the WebGL2 bridge and gallery page.
The remaining work is phased by what blocks each example.

### Phase 1: Frame yield + 18 interactive examples (FY.1–FY.3, GAL.3)

The frame-yield design ([WASM_FRAME_YIELD.md § Frame Yield](../../WASM_FRAME_YIELD.md#frame-yield--browser-game-loop-via-interpreter-suspension))
suspends the interpreter at `gl_swap_buffers` and resumes on `requestAnimationFrame`.
With this, all examples that use a render loop and don't need file I/O work immediately.

| Example | Status | Notes |
|---------|--------|-------|
| 01 Hello Window | Ready | Clear + event loop only |
| 02 Hello Triangle | Ready | GLB export path skipped |
| 03 Shaders | Ready | Vertex colors, stride=10 |
| 05 Transformations | Ready | Animated rotation |
| 06 Coordinate Systems | Ready | Multiple cubes, perspective |
| 07 Camera | Ready | Orbiting camera |
| 08 Basic Lighting | Ready | Phong shading |
| 09 Materials | Ready | Cube + sphere, PBR uniforms |
| 12 Multiple Lights | Ready | Multi-light shader |
| 13 Depth Testing | Ready | Depth fog |
| 14 Blending | Ready | Alpha blend, depth mask |
| 15 Face Culling | Ready | Backface culling toggle |
| 16 Shadow Mapping | Ready | 2-pass, depth FBO |
| 17 Post-Processing | Ready | FBO + fullscreen quad |
| 18 PBR | Ready | Cook-Torrance, 5×5 grid |
| 19 Complete Scene | Ready | Full pipeline showcase |
| 22 Wireframe | Ready | Lines + points modes |
| 23 Cleanup | Ready | Resource lifecycle |

GLB export: examples 02–19 accept `--mode glb` via `arguments()`.  In WASM
`arguments()` returns an empty vector, so the GLB path is never taken — only
the interactive render loop runs.  No code changes needed.

### Phase 2: Keyboard + mouse input (GL6.6 — 1 more example)

| Example | Blocker | Fix |
|---------|---------|-----|
| 21 Keyboard Camera | `gl_key_pressed` / `gl_mouse_x/y` return stubs | Wire `keydown`/`keyup`/`mousemove` DOM events to a state map; `loftHost.gl_key_pressed(code)` reads the map |

### Phase 3: Asset loading via VIRT_FS (GL7.1–GL7.4 — 4 more examples)

**Design principle:** examples run unchanged.  The native functions are
implemented in WASM to work without filesystem access.  No example code
is modified — the gallery page provides asset files through VIRT_FS.

#### GL7.1 — `gl_load_texture` reads from VIRT_FS

`gl_load_texture("wall.jpg")` calls a native function that reads and
decodes an image file.  In WASM:

1. The gallery page bundles asset files (images) into the `files` JSON
   array alongside the `.loft` source, e.g.:
   ```js
   [{ name: "main.loft", content: source },
    { name: "wall.jpg", content: base64data }]
   ```
2. `compile_and_start` populates VIRT_FS with all files.
3. The WASM `wgl_load_texture` reads the image bytes from VIRT_FS via
   `virt_fs_get("wall.jpg")`, decodes PNG/JPG in Rust (the `png` crate
   is already compiled into the WASM binary), and uploads to WebGL via
   `loftHost.gl_upload_rgba_texture(pixels, w, h)`.

**Unlocks:** example 04 (textures).

#### GL7.2 — `save_png` writes to download / VIRT_FS

`canvas.save_png("file.png")` writes a PNG file.  In WASM:

1. The WASM `wgl_save_png` encodes the Canvas pixel data to PNG bytes
   (the `png` crate is already available).
2. Calls `loftHost.download_file("file.png", bytes)` which triggers a
   browser download via `URL.createObjectURL(new Blob([bytes]))`.
3. Alternative: write to VIRT_FS for inspection without download.

**Unlocks:** example 10 (2D canvas).

#### GL7.3 — `gl_load_font` + text rasterization

`gl_load_font("font.ttf")` loads a TrueType font for CPU rasterization.
The native implementation uses the `fontdue` crate.  In WASM:

**Option A (preferred):** Compile `fontdue` to WASM.  It is pure Rust
with no system dependencies — it should compile to `wasm32-unknown-unknown`
directly.  The font file is provided via VIRT_FS (same as textures).
All text rasterization functions (`gl_measure_text`, `gl_text_height`,
`rasterize_text_into`) work unchanged.

**Option B (fallback):** Delegate to the browser's Canvas 2D API.
`loftHost.gl_load_font(name)` maps to CSS `@font-face`.
`loftHost.rasterize_text(text, size)` uses `CanvasRenderingContext2D.fillText`
and returns pixel data.  This changes the rendering backend but keeps the
loft API identical.

**Unlocks:** example 20 (textured cube with text).

#### GL7.4 — Example 11 needs a render loop

Example 11 (scene graph) currently only exports GLB — it has no
interactive render loop.  In WASM `arguments()` returns `[]`, so
`--mode glb` is never selected, but there is no native render path
either.

**Fix:** Add an interactive render fallback to example 11, identical
to how examples 02–09 work: `if mode != "glb" { render_native(scene) }`.
This is a **one-line change to the example** — adding a render path
that was always intended but not yet written.  The scene construction
code is unchanged; only the entry point gains a `render_native` branch.

This is **not** a WASM workaround — it makes the example more complete
on native too.

### Phase 4: High-level renderer (GL8.1 — 1 more example)

Example 24 uses `render::create_renderer()` + `render_loop()`.
`render.loft` is pure loft code that calls `gl_*` functions internally.
With frame yield, `render_loop`'s internal `for` loop yields at
`gl_swap_buffers` naturally.  `create_renderer` compiles PBR + shadow
shaders via `gl_create_shader` — shader version patching (GL6.5) is
already done.

**No new WASM code needed.**  If frame yield works and the gl_* bridge
is complete, example 24 runs unchanged.  The only risk is that
`render.loft` uses gl_* functions not yet tested end-to-end in the
WebGL bridge (FBO setup, depth textures, multi-pass rendering).
These are all implemented in the bridge; they just need testing.

### Summary: path to 24/24

| Phase | Examples | Effort | Depends on |
|-------|----------|--------|------------|
| Phase 1 | 18 of 24 | M (frame yield) | FY.1–FY.3 |
| Phase 2 | +1 (19/24) | S (DOM events) | GL6.6 |
| Phase 3 | +4 (23/24) | M (VIRT_FS asset loading) | GL7.1–GL7.4 |
| Phase 4 | +1 (24/24) | S (testing only) | GL8.1, FY.1 |
