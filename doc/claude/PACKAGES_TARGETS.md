
# Wasm bridges, OpenGL case study, security model

Per-target extras of the package format.  Split out of [PACKAGES.md](PACKAGES.md).

---

## Wasm bridges (library-owned `--html` extensions)

Scope: `loft --html` produces a standalone browser-WASM binary +
HTML wrapper, separate from the `--native-wasm` (wasm32-wasip2)
target above.  A standalone-binary build has no `State` indirection
at runtime (`replace_native` doesn't apply), and the browser has no
dlopen, no filesystem, no native OS APIs — every host capability
must arrive through wasm imports the JS host provides.

Each library that needs browser-specific glue carries it inside
the library:

```
lib/<X>/
  src/                                  # pure loft (unchanged)
  native/                               # for --native (cdylib, unchanged)
  wasm/                                 # NEW — for --html
    src/lib.rs                          # Rust `pub fn` bridges
    host.js                             # JS host-imports
    Cargo.toml                          # crate name: loft-<x>-wasm
  loft.toml                             # declares the bridge
```

The `[wasm.bridge]` manifest section declares all three artefacts:

```toml
[wasm.bridge]
crate = "loft-imaging-wasm"   # Rust bridge crate name
host_js = "wasm/host.js"      # JS host-imports file

[wasm.bridge.routes]
n_load_png = "imaging_load_png"   # loft #native symbol → bridge fn name
n_save_png = "imaging_save_png"
```

What each part does:

| Part | Compiled / loaded by | Purpose |
|---|---|---|
| `wasm/src/lib.rs` (`pub fn`s) | `loft --html` invokes `rustc --crate-type rlib --extern loft=…` directly (NOT `cargo build` — see "Why rustc-direct" below) | Receives the loft store + arg references, calls back into JS via wasm extern imports |
| `wasm/host.js` (registers via `LOFT_WASM_EXTENSIONS`) | `loft --html` concatenates into the HTML preamble; harness `tools/wasm_repro.mjs` discovers + evals at startup | Implements the wasm extern imports (DOM access, Canvas, asset table lookup, etc.) |
| `[wasm.bridge.routes]` | `src/generation/mod.rs::output_native_direct_call` reads from `data.wasm_bridge_routes` | Routes a generated `n_<sym>` body to `<crate_ident>::<bridge_fn>` |

### Why rustc-direct (not `cargo build`)

The bridge crate depends on `loft` via a path dep (so it sees the
same `Stores` / `DbRef` types).  Running `cargo build` from
`lib/<X>/wasm/` would compile its OWN copy of `loft` into
`lib/<X>/wasm/target/`, with a different `StableCrateId` than the
`--html` runtime rlib (`target/loft/html/wasm32-unknown-unknown/release/libloft.rlib`)
that the standalone-binary link uses as `--extern loft=…`.  Two copies of
the same crate → rustc fails: "expected DbRef, found DbRef".

Workaround: the `--html` driver bypasses cargo and invokes `rustc`
directly on the bridge's `src/lib.rs`, threading the SAME
`--extern loft=…` + `-L dependency=…` flags through.  The bridge
rlib lands in `std::env::temp_dir()/lib<crate_ident>.rlib`; the
final link adds `--extern <crate_ident>=<that rlib>` to the main
rustc invocation.  One copy of `loft`, zero collisions.

The SAME reasoning extends to the bridge's *own* Cargo deps
(dalek/RustCrypto).  Those are loft-independent, so they CAN be
`cargo build`-ed — but NOT from the bridge's `wasm/Cargo.toml`,
which still declares `loft = { path = "../../../loft" }`.  That path
resolves in a dev checkout but not for a registry-installed package
(`~/.loft/registry/<pkg>/wasm/../../../loft` → `~/.loft/loft`, absent),
where cargo aborts before building a single dep — this was
[#446](https://github.com/loft-lang/loft/issues/446), the last blocker
for browser builds off the registry.  The fix: synthesize a
**deps-only crate** (an empty lib whose `[dependencies]` are exactly
the bridge's non-`loft` deps) and `cargo build` THAT.  cargo builds
every declared dep regardless of the empty lib, yielding the identical
wasm32 dep rlibs WITHOUT resolving the manifest's redundant `loft`
path.  See `bridge_nonloft_deps` / `synth_bridge_deps_manifest` in
`src/main.rs`.

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
and adds its functions via `Object.assign(imports.loft_gl, {...})`.
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

`lib/imaging` is the first library to use this pattern end-to-end.
See:
- `lib/imaging/wasm/src/lib.rs` — `imaging_load_png` /
  `imaging_save_png` bridges; field offsets in the `Image` struct;
  vector allocation via `loft::vector::alloc_vector_from_bytes`.
- `lib/imaging/wasm/host.js` — `imaging_query` / `imaging_copy_rgb`
  / `imaging_save` JS implementations.
- `lib/imaging/loft.toml::[wasm.bridge]` — the manifest declaration.

History + design rationale: [lib_plans/29-library-wasm-bridges](lib_plans/finished/29-library-wasm-bridges/README.md);
the @P321(c) browser-WASM dimension landing in [PROBLEMS.md](PROBLEMS.md)
is what surfaced the need.

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

### WASM variant

On `--native-wasm`, `loft.toml [native.wasm]` overrides:
- `gl_create_window` → `webgl::create_canvas` (creates `<canvas>` element)
- `gl_swap_buffers` → `webgl::flush_canvas` (requestAnimationFrame)
- `gl_create_shader` → `webgl::create_shader` (WebGL2 shader API)

The loft code is identical.  Only the native implementation changes.

### What stays in pure loft

| Component | Why loft, not Rust |
|---|---|
| 2D rasterizer (scanline fill, Bezier) | Performance contract — proves the interpreter is fast enough |
| Matrix math (Mat4, Vec3 ops) | Simple arithmetic — no benefit from native |
| Scene graph (transforms, camera) | Pure data manipulation |
| GLB binary writer | Byte-level file I/O — loft's `File` API handles it |
| Mesh generation | Vertex computation — pure math |

### What must be native

| Component | Why Rust, not loft |
|---|---|
| PNG encode/decode | Depends on `png` crate (zlib compression) |
| Font rasterization | Depends on `fontdue` crate (TrueType parsing) |
| GL context + window | Depends on `glutin`/`winit` (OS window management) |
| GL API calls | OpenGL is a C API; Rust FFI is the natural bridge |
| WebGL API calls | Browser DOM access via `web-sys` in WASM |

---

## Security model

### Interpreter mode

Native packages load shared libraries via `dlopen`.  A loaded library has
full process access — it can read files, open sockets, allocate memory.

**Mitigation:**
- `--no-native` flag: refuse to load any `#native` functions.  The program
  runs only pure-loft code; native calls produce a runtime error.
- Package signatures (Phase 3): SHA-256 hash in a lock file; refuse to
  load if the hash doesn't match.
- Origin tracking: `loft.toml` records the source URL; the runtime warns
  when loading a native package from an unknown origin.

### WASM mode

WASM is sandboxed by the runtime (wasmtime, browser).  Native functions
compiled to WASM can only access capabilities granted by the host:
- File I/O: only through the VirtFS bridge
- Network: only if the host provides a WASI socket capability
- GPU: only through WebGL (browser) or headless EGL (wasmtime)

No additional sandboxing needed — WASM's capability model is sufficient.

### Native mode (`--native`)

The generated Rust binary links the native package's rlib statically.
The binary has full OS access.  Same security as any compiled program.
No sandboxing — the user chose to compile and run native code.
