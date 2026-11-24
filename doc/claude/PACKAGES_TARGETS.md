
# Wasm bridges, OpenGL case study, security model

Per-target extras of the package format.  Split out of [PACKAGES.md](PACKAGES.md).

---

## Wasm bridges (library-owned `--html` extensions)

Scope: `loft --html` builds the program's C, with every used library's
`[c] shim`, into a browser-WASM module plus an HTML wrapper (a page build
defines `LOFT_BROWSER`).  The browser has no dlopen, no OS APIs and only the
page's filesystem — every other host capability arrives through wasm imports
the page's JavaScript provides.

A library that needs browser-specific glue carries both halves itself:

```
lib/<X>/
  src/                                  # pure loft
  c/<x>.c                               # the [c] shim; its LOFT_BROWSER arm declares the imports
  wasm/host.js                          # JS that provides those imports
  loft.toml                             # [c] shim = "c/<x>.c" + [wasm.bridge] host_js
```

```toml
[wasm.bridge]
host_js = "wasm/host.js"      # JS host-imports file
```

| Part | Compiled / loaded by | Purpose |
|---|---|---|
| the shim's `#if defined(LOFT_BROWSER)` arm | clang, with the program's C (`c_driver::build_wasm`) | declares each host function as `__attribute__((import_module("loft_<x>"), import_name("f")))` and calls it |
| `wasm/host.js` (registers via `LOFT_WASM_EXTENSIONS`) | `loft --html` concatenates into the HTML preamble; harness `tools/wasm_repro.mjs` discovers + evals at startup | Implements the wasm imports (DOM access, Canvas, asset table lookup, etc.) |

A `[wasm.bridge] crate` and `[wasm.bridge.routes]` (a Rust bridge crate the
rustc writer linked) are not built; how they worked is recorded in
[PACKAGES_TARGETS-history.md](PACKAGES_TARGETS-history.md).

### Self-registration via `LOFT_WASM_EXTENSIONS`

The HTML preamble's load order is:

1. `doc/loft-gl-wasm.js` (generic — defines `buildLoftImports`,
   `decodeLoftAssets`, asset preload, host_asset_exists)
2. Each library's `wasm/host.js` (pushes a callback onto
   `globalThis.LOFT_WASM_EXTENSIONS = (globalThis.LOFT_WASM_EXTENSIONS || [])`)
3. `const imports = buildLoftImports(canvas, output, () => mem, ctrl)`
4. Dispatch:
   ```js
   for (const reg of (globalThis.LOFT_WASM_EXTENSIONS || [])) {
     reg(imports, ctrl, () => mem);   // mutates imports.loft_gl
   }
   ```
5. `WebAssembly.instantiate(wasmBytes, imports).then(...)`

Each library's `host.js` callback receives `(imports, ctrl, getMem)`
and adds its functions to the import module its shim names
(`imports.loft_<x> = {...}`).
Test harnesses use the same dispatch — `tools/wasm_repro.mjs` scans
`lib/*/wasm/host.js` at startup and runs the dispatch before
`new WebAssembly.Instance(...)`.

### What stays generic (in the compiler / tooling crate)

- `src/wasm_assets.rs::asset_exists` — checks the JS-side asset
  table for a basename (used by `database::io::get_file` so PNG
  assets report as `TextFile` and `file().png()` reaches the
  bridge instead of short-circuiting to `null`).  Library-
  agnostic — every wasm-asset library uses it.
- `doc/loft-gl-wasm.js::decodeLoftAssets` — generic PNG asset
  preload decoder (`createImageBitmap` + Canvas `getImageData` +
  RGBA→RGB demux).  Runs before `loft_start` so wasm-side asset
  lookup is synchronous.
- `tools/wasm_repro.mjs`'s `node:zlib`-based PNG decoder + the
  asset-table builder.  Generic test infrastructure.

### Canonical example

The `web` library's browser half is the worked shape: `src/c/loft_web_page.h`
declares the `loft_web` imports (WebSocket, `fetch()`) under `LOFT_BROWSER`, and
`doc/loft-web.js` provides them.  Design rationale:
[lib_plans/29-library-wasm-bridges](lib_plans/finished/29-library-wasm-bridges/README.md).

---

## OpenGL case study

### Why OpenGL drives the package design

OpenGL is the first real-world use case that requires:
- **Native code** (GL context creation, shader compilation, buffer management)
- **Platform-specific variants** (OpenGL on desktop, WebGL in browser)
- **Large loft-side logic** (rasterizer, matrix math, scene graph)
- **Binary dependencies** (glutin, fontdue, png crate)

If the package format handles OpenGL cleanly, it handles everything.

### Package structure

```
graphics/
├── loft.toml
├── src/
│   ├── graphics.loft       # re-exports: pub use draw; pub use text;
│   ├── draw.loft            # Canvas, Rgba, Draw — pure loft rasterizer
│   ├── primitives.loft      # rect, ellipse, line, bezier — pure loft
│   ├── text.loft            # Font, TextStyle, draw_text — pure loft
│   ├── math.loft            # Mat4, Vec3, matrix ops — pure loft
│   ├── mesh.loft            # Vertex, Triangle, Mesh — pure loft
│   ├── scene.loft           # Transform, Camera, Light — pure loft
│   └── gl.loft              # OpenGL/WebGL API — #native bindings
├── native/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs           # re-exports
│       ├── png_io.rs        # save_png, load_png
│       ├── font.rs          # load_font, glyph_metrics, rasterize_glyph
│       ├── gl.rs            # create_window, swap_buffers, create_shader, ...
│       └── webgl.rs         # WASM variants of gl.rs functions
```

### `gl.loft` — the binding layer

```loft
// Types
pub struct Window { id: integer not null }
pub struct Shader { id: integer not null }
pub struct Buffer { id: integer not null }

// Window management
pub fn create_window(title: text, width: integer, height: integer) -> Window;
#native "gl_create_window"

pub fn swap_buffers(self: Window);
#native "gl_swap_buffers"

pub fn should_close(self: Window) -> boolean;
#native "gl_should_close"

pub fn poll_events(self: Window);
#native "gl_poll_events"

// Shader operations
pub fn create_shader(vertex_src: text, fragment_src: text) -> Shader;
#native "gl_create_shader"

pub fn use_shader(self: Shader);
#native "gl_use_shader"

// Buffer operations
pub fn create_buffer(data: vector<single>) -> Buffer;
#native "gl_create_buffer"

pub fn draw_triangles(self: Buffer, count: integer);
#native "gl_draw_triangles"
```

### User program

```loft
use graphics;

fn main() {
  // 2D software rendering — works everywhere (pure loft)
  canvas = Canvas { width: 800, height: 600 };
  canvas.clear(0xFF000000);     // black
  draw_rect(canvas, 100, 100, 200, 150, 0xFFFF0000);  // red rectangle
  save_png(canvas, "output.png");

  // 3D hardware rendering — requires native GL package
  win = create_window("My App", 800, 600);
  shader = create_shader(VERTEX_SRC, FRAGMENT_SRC);
  buf = create_buffer([0.0f, 0.5f, 0.0f, -0.5f, -0.5f, 0.0f, 0.5f, -0.5f, 0.0f]);
  while !win.should_close() {
    win.poll_events();
    shader.use_shader();
    buf.draw_triangles(3);
    win.swap_buffers();
  }
}
```

### Browser variant

On `--html`, each `gl_*` call is a `loft_gl` wasm import that the page's
`doc/loft-gl-wasm.js` implements over WebGL2: `gl_create_window` shows the
`<canvas>`, `gl_swap_buffers` is the frame yield (requestAnimationFrame), and
`gl_create_shader` translates to GLSL ES ([HTML_EXPORT.md](HTML_EXPORT.md)).
The loft code is identical.  Only the native implementation changes.

### What stays in pure loft

| Component | Why loft, not native |
|---|---|
| 2D rasterizer (scanline fill, Bezier) | Performance contract — proves the interpreter is fast enough |
| Matrix math (Mat4, Vec3 ops) | Simple arithmetic — no benefit from native |
| Scene graph (transforms, camera) | Pure data manipulation |
| GLB binary writer | Byte-level file I/O — loft's `File` API handles it |
| Mesh generation | Vertex computation — pure math |

### What must be native

| Component | Why native, not loft |
|---|---|
| PNG encode/decode | Depends on a codec library (zlib compression) |
| Font rasterization | Depends on a TrueType parser |
| GL context + window | Depends on OS window management |
| GL API calls | OpenGL is a C API; the C shim calls it directly |
| WebGL API calls | Browser DOM access, through the page's `loft_gl` imports |

---

## Security model

### Interpreter mode

A package's C shim is compiled to a shared library the interpreter loads via
`dlopen`.  A loaded library has full process access — it can read files, open
sockets, allocate memory.

**Mitigation:**
- Package signatures (Phase 3): SHA-256 hash in a lock file; refuse to
  load if the hash doesn't match.
- Origin tracking: `loft.toml` records the source URL; the runtime warns
  when loading a native package from an unknown origin.

### WASM mode

WASM is sandboxed by the runtime (wasmtime, browser).  Native functions
compiled to WASM can only access capabilities granted by the host:
- File I/O: only through WASI (the page's filesystem on `--html`)
- Network: only if the host provides a WASI socket capability
- GPU: only through WebGL (browser) or headless EGL (wasmtime)

No additional sandboxing needed — WASM's capability model is sufficient.

### Native mode (`--native`)

The binary links the package's C shim with the program's C.
The binary has full OS access.  Same security as any compiled program.
No sandboxing — the user chose to compile and run native code.
