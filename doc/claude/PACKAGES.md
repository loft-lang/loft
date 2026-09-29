
# Library Package Format

Design for a unified packaging format that supports pure-loft libraries,
native Rust extensions, and WASM targets — with OpenGL as the driving
use case.

The timeline behind this doc (the original phase plan) is in [PACKAGES-history.md](PACKAGES-history.md).

---

## Contents
- [Goals](#goals)
- [Current state](#current-state)
- [Package layout](#package-layout)
- [Manifest: `loft.toml`](#manifest-lofttoml)
- [Package dependencies](#package-dependencies)
- [Function binding model](#function-binding-model)
- [Discovery and loading](#discovery-and-loading)
- [Auto-marshalling dispatch (interpreter, legacy path)](PACKAGES_NATIVE_API.md#auto-marshalling-dispatch-interpreter-legacy-path)
- [loft-ffi helper crate](PACKAGES_NATIVE_API.md#loft-ffi-helper-crate)
- [Store allocation from native code](PACKAGES_NATIVE_API.md#store-allocation-from-native-code)
- [Code generation: `loft generate`](PACKAGES_NATIVE_API.md#code-generation-loft-generate)
- [Key source files](PACKAGES_NATIVE_API.md#key-source-files)
- [Package test suite](PACKAGES_BUILD.md#package-test-suite)
- [Build pipeline](PACKAGES_BUILD.md#build-pipeline)
- [Target matrix](PACKAGES_BUILD.md#target-matrix)
- [OpenGL case study](PACKAGES_TARGETS.md#opengl-case-study)
- [Security model](PACKAGES_TARGETS.md#security-model)
- [Open work](#open-work)

---

## Goals

1. A single package can contain loft source, native Rust code, and
   pre-compiled WASM — consumers don't choose; the runtime picks the
   right variant for the target.
2. `use graphics;` in a loft program works identically whether running
   via the interpreter, `--native`, or `--native-wasm`.
3. OpenGL/WebGL bindings ship as a package, not as built-in stdlib.
4. Package authors write Rust once; the build system produces native
   and WASM artifacts from the same source.
5. Native code is Rust linking against `libloft.rlib`.  A C library is reached
   through a `#c` binding (@PLN24, below), never through C code in the package.

---

## Current state

| Layer | Status |
|---|---|
| Pure-loft packages (`use lib;`) | **Shipped** — directory layout, version check, `lib/` search |
| `loft.toml` manifest | **Shipped** — entry, version, native stem fields |
| `#native "symbol"` annotation | **Parsed** — bytecode dispatch NOT connected |
| `extensions.rs` cdylib loader | **Designed** — feature-gated, not integrated |
| WASM virtual filesystem | **JS side done** — Rust stubs return early |
| Native codegen (`--native`) | **Working** — generates Rust, compiles with rustc |
| WASM codegen (`--native-wasm`) | **Working** — targets wasm32-wasip2 |

The gap: no package can currently ship native code that works across
interpreter, native, and WASM targets from a single source.

---

## Package layout

```
graphics/
├── loft.toml                 # manifest
├── src/
│   ├── graphics.loft         # public loft API (types, wrappers)
│   ├── draw.loft             # loft-implemented rasterizer
│   └── math.loft             # loft-implemented matrix ops
├── tests/
│   ├── draw.loft             # test_* functions for draw module
│   ├── math.loft             # test_* functions for math module
│   └── integration.loft      # cross-module integration tests
├── native/
│   ├── Cargo.toml            # Rust crate for native functions
│   ├── src/
│   │   └── lib.rs            # implements #[loft_fn] functions
│   └── build.rs              # optional build script
└── prebuilt/                 # optional: pre-compiled artifacts (@PLN21)
    ├── x86_64-unknown-linux-gnu/
    │   ├── libloft_graphics_native.so   # cdylib — dlopen'd, NOT linked
    │   └── .loft-build-fp               # the loft-ffi fp it was built against
    ├── aarch64-apple-darwin/
    │   ├── libloft_graphics_native.dylib
    │   └── .loft-build-fp
    └── wasm32-wasip2/
        └── libgraphics.wasm             # wasm rlib (codegen-linked, not dlopen)
```

**Rules:**
- `src/` is mandatory — every package has at least one `.loft` file
- `native/` is optional — only if the package has Rust-implemented functions
- `prebuilt/` is optional — avoids requiring Rust toolchain on consumer machine.
  The NATIVE prebuilt is the **cdylib** (`.so`/`.dylib`/`.dll`), NOT an `.rlib`: it is
  `dlopen`'d over the loft-ffi C-ABI, so it is rustc-version-independent (an `.rlib` is
  SVH-locked to its rustc — E0514). Each per-triple dir carries a `.loft-build-fp`
  sidecar (the `loft-ffi` fingerprint it was built against); loft loads it only when
  that matches `cache::loft_ffi_fingerprint()` (@PLN21 Phase 1).
- `[native] runtime-libs` / `build-deps` (@PLN21 Phase 2) declare the package's system
  shared libs (validation + install hint) and dev packages (source-build diagnostics).
- The primary `.loft` file (`src/graphics.loft`) declares the public API
  including `#native` function signatures

---

## Manifest: `loft.toml`

```toml
[package]
name = "graphics"
version = "0.1.0"
loft = ">=0.9"
description = "2D canvas + 3D rendering for loft."  # one-line registry summary —
                                   # the OFFICIAL source for `loft search` /
                                   # `loft api --registry`.  Registry tooling
                                   # prefers this over scraping the README.
repository = "loft-libs-graphics"  # publishing repo — drives `loft package`'s
                                   # release URL/tag.  A monorepo value (several
                                   # packages share the repo) → `<name>-v<version>`
                                   # tags; a value with "/" is a full owner/repo;
                                   # omit → legacy loft-<name> repo + bare v<version>.

[library]
entry = "src/graphics.loft"
placement = "process"       # @PLN119 — run this library in a WORKER PROCESS, so a
                            # crash in it cannot corrupt its consumer's stores.
                            # Consumers are unchanged: same `use`, same typed
                            # `pub fn` calls.  "inproc" (the default, and what
                            # every library without this line gets) runs it here.
                            # Linux only; elsewhere it runs in-process — the same
                            # program, without the isolation.  See below.

[dependencies]
# Other loft packages this package needs.
# Keys are package names; values are version requirements.
math = ">=0.1"              # from registry or ~/.loft/lib/
utils = { path = "../utils" }  # local path (development)

[native]
# Rust crate in native/ — compiled to rlib (interpreter/native) or
# wasm (WASM target) at install time or first use.
crate = "native"

# Functions implemented in Rust.  Keys are loft function names;
# values are Rust symbol paths.  The loft compiler verifies signatures
# match between the .loft declaration and the Rust implementation.
[native.functions]
save_png = "graphics_native::save_png"
load_font = "graphics_native::load_font"
glyph_metrics = "graphics_native::glyph_metrics"
gl_create_window = "graphics_native::gl::create_window"
gl_swap_buffers = "graphics_native::gl::swap_buffers"

[native.wasm]
# WASM-specific overrides: some functions have different implementations
# in WASM (WebGL instead of OpenGL, Canvas2D instead of pixel buffer).
gl_create_window = "graphics_native::webgl::create_canvas"
gl_swap_buffers = "graphics_native::webgl::flush_canvas"

[native.dependencies]
# Additional crate dependencies the native code needs.
# These are added to the native/Cargo.toml [dependencies] section.
glutin = "0.32"
fontdue = "0.9"
png = "0.17"

# @PLN146 F5 — a font this package draws with, and where each target gets it.
# One entry per family; a library's entries reach a consumer's page too.
[[font]]
family = "PressStart2P"              # the CSS family AND the name the program
                                     # looks the font up by
native = "fonts/PressStart2P.ttf"    # what the program passes to gl_load_font;
                                     # its base name MUST equal `family`
url    = "fonts/PressStart2P.woff2"  # our own server -> an @font-face in the page
# stylesheet = "https://fonts.example/css2?family=…"   # a provider -> a <link>

# @PLN146 F4 — a file the --html page carries in its own filesystem.
# One entry per file; a library's entries reach a consumer's page too.
[[embed]]
path   = "assets/game.pack"   # what the PROGRAM passes; also the key in the page's FS
source = "build/game.pack"    # optional: where the bytes are on the build box.
                              # Defaults to `path`; both resolve against the
                              # PROGRAM's directory, the way loft resolves any
                              # path a program passes.
```

The `[[font]]` and `[[embed]]` tables and the `placement` key are in [PACKAGES_MANIFEST_TABLES.md](PACKAGES_MANIFEST_TABLES.md).

---

## Package dependencies

### Declaring dependencies

A package declares its dependencies in `loft.toml`:

```toml
[dependencies]
math = ">=0.2"                    # version requirement (newest satisfying)
utils = { path = "../utils" }     # local path (for development)
json = { version = ">=1.0" }      # explicit version field
glb = "=0.1.0"                    # EXACT pin — this version, never newer
```

**Version constraints.** `>=`, `>`, `<=`, `<`, `^` (caret), `~` (tilde), a
comma-list (`>=0.2, <0.3`), and `*` / empty (any) resolve to the **newest**
version that satisfies them.  `=X.Y.Z` — or a bare `X.Y.Z` — is an **exact pin**:
that version and no other.  Reach for an exact pin for reproducibility, or to
dodge a bad release without waiting for a fix.  Pinning is an *option*, not the
rule — omit it and you get the newest compatible release.

**A declared break holds an upgrade back.**  Once `loft.lock` holds a release,
`loft update` does not move it past a newer one whose `api_compatible_with` is above
it, and it names each release it held back.  To take one, say so in `loft.toml`: an
exact pin, or a floor the held release no longer satisfies (`>=0.1.3` over a held
0.1.0), is the deliberate step across the break.

The **root project's** declared constraints pin the **whole tree**, including
packages pulled in *transitively* by a `use` inside a dependency: a source-level
auto-install honours the root's pin, so `glb = "=0.1.0"` holds even when it's
`graphics` (not your code) that does `use glb;`.

In loft source, the dependency is imported with `use`:

```loft
// src/graphics.loft
use math;       // imports math package — types, functions available
use utils;      // imports utils package

pub fn transform(canvas: Canvas, mat: math.Mat4) -> Canvas {
  // math.Mat4 is a type from the math package
  // ...
}
```

### Resolution order

When the compiler encounters `use math;` it searches:

1. **Local `src/`** — sibling files in the same package
2. **`[dependencies]` paths** — `path = "..."` entries from `loft.toml`
3. **Package lib directories** — `~/.loft/lib/math/`, project `lib/math/`
4. **`--lib` CLI flag** — explicit search directories
5. **`LOFT_LIB` environment variable**

The first match wins, and a project `lib/` outranks `--lib`
(`advice[lib-flag-outranked]` says so when it happens).  If the dependency has its own
`loft.toml`, its version is checked against the requirement; which installed version a
lock file selects is [LIBRARY_AUTHORING.md § Troubleshooting](LIBRARY_AUTHORING.md).

⚠ **Step 3's project half needs a project.** The directory form `lib/math/` is read
through `lib_path_manifest`, which runs over the directories a program NAMED — a
`--lib` argument, a `LOFT_LIB` entry, `~/.loft/lib/`, a path dependency, a sibling
package.  A bare script's own `lib/` is not one of those: it is probed flat, so
`lib/math.loft` resolves there and `lib/math/src/math.loft` answers *"Library 'math'
not found"*.  Put a `loft.toml` above the script — make it a package — and the same
`lib/math/` resolves.  Measured across (root manifest × script in `src/` × package
manifest); the root manifest is the only axis that decides it.

### A `use` carries names ONE way

Step 1 above means a sibling file in your own `src/` is loaded exactly like any
other package — and that has a consequence worth stating plainly, because it
decides how a package can be split:

> `use helper;` imports **helper's** public names into the file that wrote the
> `use`.  It does **not** import that file's names into `helper`.

A used file is also parsed *before* the file that used it reaches its own
definitions (`switch_to_dep` suspends the importer — see the transitive section
below), so a name the importer declares is not merely out of scope in the used
file: it does not exist yet when the used file asks for it.

```loft
// src/pkg.loft — the entry
use helper;
pub struct Thing { t_n: integer }
pub fn make() -> Thing { Thing { t_n: 1 } }
```
```loft
// src/helper.loft
pub fn via_fn() -> integer { make().t_n }   // refused — `make` belongs to pkg.loft
```

**The cure** is a third file both `use`:

```loft
// src/shared.loft         — declarations both need
pub struct Thing { t_n: integer }
pub fn make() -> Thing { Thing { t_n: 1 } }

// src/helper.loft
use shared;
pub fn via_fn() -> integer { make().t_n }   // ok

// src/pkg.loft
use shared;
use helper;
```

Guard: `tests/package_layout.rs::shared_sibling_carries_types_and_functions`.

**Types and methods appear to cross; that is adoption, not a second rule.** A
used file naming a type its importer declares registers a `DefType::Unknown`
forward-reference stub, and the importer's later declaration ADOPTS that stub in
place (the three `Unknown` arms in `parser/definitions.rs`) — which is also what
makes a genuine cyclic `use` resolve.  Adoption reaches whichever used file's
stub the declaration lands on, so it is **not** a namespace: with two used files
naming the same type, one resolves and the other does not.  Do not build on it;
put the declaration in a shared file.

Two diagnostics exist because none of this is visible in the failure (loft#826):

- A name declared by the importer is refused with **where it is declared, which
  way a `use` carries names, and the cure** — for a function, a file-scope
  constant, or a type.  Without it the fuzzy same-name guess answered a call to
  `make()` with *"Unknown function make — did you mean 'move'?"*, pointing away
  from the cause while `make` sat one file away.
- An unresolved stub is **never reported as a rival declaration**.  Those stubs
  are pub-visible and import back into the entry like any other public name, so
  a second one landing on the first was recorded as an ambiguity — and a type
  declared *once* in the entry was reported as `declared by more than one
  package`, advising `helper::Thing` or `second::Thing` when neither file
  declares `Thing` and neither qualification can resolve.

### Transitive dependencies

If `graphics` depends on `math`, and `math` depends on `utils`, then
building `graphics` also loads `utils`.  The compiler resolves
transitively:

```
graphics/loft.toml  →  [dependencies] math = ">=0.2"
math/loft.toml      →  [dependencies] utils = ">=0.1"
```

Resolution:
1. Parse `graphics/src/graphics.loft` → encounters `use math;`
2. Find `math/` package → read `math/loft.toml` → version check
3. Parse `math/src/math.loft` → encounters `use utils;`
4. Find `utils/` package → read `utils/loft.toml` → version check
5. Parse `utils/src/utils.loft`
6. All types and functions from `utils` and `math` are now available

**A manifest dependency is pulled in from the file that named the package.**
Loading a library REPLACES the lexer's source; the file it switched away from
resumes later off `todo_files`. That is safe for a `use`, which always switches
away from the very file it appears in — but a `[dependencies]` entry is queued
when the manifest is read and drained by the same use-region loop, so it has to
wait until the lexer is back on that same file.

Draining it anywhere else is what loft#714 was: a dep got pulled in while the
lexer sat inside an unrelated library whose definitions had not been parsed yet.
That library was already marked loaded, so every later `use` of it was a no-op
against an empty library, and the failure landed **inside valid library code** —
`Unknown variable` on a tuple destructure, or `Expect token ;` on a tuple field,
because a tuple is the construct that needs the callee's return type at parse
time. Nothing in either message named resolution. It took two manifest
dependencies whose graphs meet; one alone never showed it.

`LOFT_LIB_ORDER=1` prints every library switch as it happens
(`[liborder] switch <from> -> <to>`) — the fastest way to see an order like
`hex_field.loft -> hex_draw.loft`, where the target is not a dependency of the
source at all. Guard: `tests/package_layout.rs::pkg_deps_resolve_before_the_dependent_is_parsed`.

### Diamond dependencies

When two packages depend on the same package:

```
graphics → math >=0.2
graphics → physics → math >=0.1
```

Loft loads `math` **once** at the highest compatible version.  Since
`>=0.2` satisfies `>=0.1`, version `0.2` is used.

If requirements conflict (e.g., `math =0.2` vs `math =0.3`), the
compiler emits:

```
Error: conflicting dependency versions for 'math':
  graphics requires =0.2
  physics requires =0.3
```

### Version syntax

| Pattern | Meaning |
|---|---|
| `">=0.2"` | Any version 0.2.0 or higher |
| `">=0.2.1"` | Any version 0.2.1 or higher |
| `"=0.2.0"` or `"0.2.0"` | Exactly 0.2.0 |
| `"^0.2"` / `"~0.2.1"` | 0.2.x (caret: at least 0.2.0; tilde: at least 0.2.1) |
| `">=0.2, <0.2.5"` | Every part at once |
| `{ path = "../math" }` | Local directory (no version check) |
| `{ version = ">=1.0" }` | Same as string form, explicit syntax |

### Cycle detection

Circular dependencies are rejected:

```
Error: circular dependency: graphics → math → graphics
```

The resolver tracks the dependency chain and panics on cycles before
any source is parsed.

### Native dependency propagation

When package A depends on package B, and both have `[native]` sections,
the build system must link both rlibs:

```
graphics/native/ depends on math/native/  (Rust crate dependency)
```

This is expressed in `graphics/native/Cargo.toml`:

```toml
[dependencies]
math_native = { path = "../../math/native" }
```

The loft build system passes both `--extern` flags to rustc:
```bash
rustc --extern math_native=.../libmath_native.rlib \
      --extern graphics_native=.../libgraphics_native.rlib \
      generated_program.rs
```

### Lock file

After resolving all dependencies, `loft install` writes `loft.lock`:

```toml
# Auto-generated — do not edit
[[package]]
name = "math"
version = "0.2.3"
source = "~/.loft/lib/math"

[[package]]
name = "utils"
version = "0.1.0"
source = "~/.loft/lib/utils"
```

Subsequent builds use `loft.lock` for reproducibility.  `loft update`
re-resolves and rewrites the lock file.

The lock is the RESOLVED form of the declarations, never a declaration of its own — cargo's
rule.  So:

- **A lock entry a declaration has since excluded is stale, and the declaration wins.**  Edit
  `glb = "=0.1.0"` into `loft.toml` over a lock recording 0.1.2 and the next run loads
  0.1.0; `loft install` then rewrites the entry.  The declarations asked are the project's
  `[dependencies]` and those of the package whose file says `use glb`.
- **One lock governs the whole program** — the entry file's.  A `use` inside a dependency
  resolves through it too, so the consumer's lock pins a transitive package.
- **`loft install` binds a transitive package to what the project declares for it**, on top
  of what the package pulling it requires, so the lock records the version the program
  loads whatever order `[dependencies]` lists.  A declaration no dependency can accept is
  refused at install, naming both sides.

---

## Function binding model

### Declaration in `.loft`

```loft
// src/graphics.loft

pub struct Canvas {
  width: integer not null,
  height: integer not null,
  data: vector<integer>     // RGBA pixel buffer
}

// Pure loft: implemented in draw.loft
pub fn clear(self: Canvas, color: integer) {
  for px_i in 0..self.width * self.height {
    self.data[px_i] = color;
  }
}

// Native: implemented in Rust, declared with #native
pub fn save_png(self: const Canvas, path: text);
#native "save_png"

pub fn load_font(path: text) -> integer;
#native "load_font"
```

### Implementation in Rust

A native function is a plain `extern "C"` fn using the `loft-ffi` ABI types,
annotated with **`#[loft_native]`**.  The macro reads the fn's *real Rust
signature* and generates a uniform marshal bridge (`<fn>__loft_bridge`) — you
write **no** marshalling code (plan-25 FFI generated-dispatch):

```rust
// native/src/lib.rs
use loft_ffi::{LoftRef, LoftStore};
use loft_ffi_macros::loft_native;

/// .loft decl:  fn save_png(self: const Image, path: text) -> boolean;
#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_save_png(
    store: LoftStore,                      // present when any arg is a ref/vector
    image: LoftRef,                        // a struct / vector arg
    path_ptr: *const u8, path_len: usize,  // a `text` arg → ptr + len
) -> bool {
    let path = unsafe { loft_ffi::text(path_ptr, path_len) };
    let w = unsafe { store.get_long(image.rec, image.pos, WIDTH) };
    // … encode the PNG; return success
    true
}
```

**Type mapping** — loft declaration → the impl's real Rust parameter:

| Loft type | Rust parameter | Notes |
|---|---|---|
| `integer` | `i64` | 64-bit; the bridge casts the cell to the impl width |
| `i32` / `u16` / `u8` / … | the same narrow Rust int | the **impl** picks the width — the bridge casts `as <type>` |
| `float` / `single` | `f64` / `f32` | |
| `boolean` | `bool` | |
| `text` | `*const u8, usize` | one loft arg → **two** Rust params (`loft_ffi::text(ptr,len)` → `&str`) |
| `vector<T>` / a struct | `LoftRef` | read via `store.vector_*` / field offsets; needs `store: LoftStore` first |

Returns map the same way: `text` → `LoftStr` (build with `loft_ffi::ret(s)`),
`vector`/struct → `LoftRef`, scalars by value.  A nullable `integer` return
uses the `i64::MIN` sentinel — the bridge preserves `i32::MIN → i64::MIN` for
narrow-int returns automatically.

`LoftStore` is the first parameter **only** when the fn touches a ref/vector
(the interpreter passes the store of the first ref arg, with allocation
callbacks for ref/vector returns).

The **direct C binding, `#c`** (@PLN24) — a C library reached without a Rust bridge — is in [PACKAGES_C_BINDING.md](PACKAGES_C_BINDING.md).

---

### Registration — zero boilerplate

You do **not** hand-maintain a register list.  A `build.rs` source-scans the
package's `.loft` for `#native` annotations and generates both the function
register and the bridge register:

```rust
// native/build.rs
fn main() {
    loft_ffi_build::generate_register_from_loft_with_bridges("../src");
}
```

```toml
# native/Cargo.toml
[lib]
crate-type = ["cdylib", "rlib"]   # cdylib → --interpret dlopen; rlib → --native link

[dependencies]
loft-ffi        = "0.1"
loft-ffi-macros = "0.1"           # the #[loft_native] proc-macro

[build-dependencies]
loft-ffi-build  = "0.2"           # the #native source-scanner
```

Adding a function is then a single edit: write the `.loft` declaration with a
bare `#native`, write the `#[loft_native]` Rust impl — the register + bridge
lists regenerate automatically.  No manifest, no drift.  (The legacy
`loft.toml [native.functions]` table and `generate_register_from_loft` — the
no-bridge variant — predate this and are retained only for un-migrated libs.)

**Write the CLEAN binding: let the `#native` symbol be the implementing fn's own
name.**  The generator only emits `"S" => S__loft_bridge`, so following it gives
you that for free.  A hand-written list can instead point `"S"` at a
differently-named fn, and then `S` is just a binding id with some *other* export
sitting under it — which is how loft#907 shipped: `graphics` kept an older raw
`(ptr, count)` fn under each `#native` name and loft's real
`(LoftStore, LoftRef)` entry point at `n_<x>`, so ten functions bound the wrong
one under `--native` and answered without complaining.  loft now resolves through
your registration on both backends (NATIVE_ARTIFACT_IDENTITY.md § Which symbol a `#native` binding
links), so a remap is no longer *wrong* — but it still costs you every consumer
whose cdylib loft cannot load, and it is free to avoid.

### Three execution paths

| Path | How native functions run |
|---|---|
| **Interpreter** | dlopen the cdylib → call the generated `<fn>__loft_bridge` (the uniform `LoftBridgeFn`) via the interpreter's bridge registry.  Libraries not yet built with `#[loft_native]` fall back to the legacy raw-ptr `dispatch_call` arms. |
| **`--native`** | Generated Rust calls the real typed fn directly (rlib linked via `--extern …`), **zero marshal** — the perf path; unaffected by the bridge layer |
| **WASM** | Generated Rust calls the WASM variant (`prebuilt/wasm32-wasip2/` or compiled in-situ) |

### Signature verification

The loft compiler checks the `.loft` declaration's parameter count + types are
compatible with the bound symbol.  The Rust impl's signature is **authoritative
for widths** — `#[loft_native]` reads it directly — so a loft `integer`
declared in `.loft` but impl'd as `i32` marshals correctly with no loft-core
change (this is the @P370 lesson the macro encodes).

### Complete example — a 3-function library

```loft
// src/mathx.loft
pub fn gcd(a: integer, b: integer) -> integer;   #native
pub fn hex(self: integer) -> text;               #native
pub fn rgb_lum(pixels: vector<integer>) -> integer;  #native
```

```rust
// native/src/lib.rs
use loft_ffi::{LoftRef, LoftStore, LoftStr};
use loft_ffi_macros::loft_native;

#[loft_native]
#[unsafe(no_mangle)]
pub extern "C" fn n_gcd(mut a: i64, mut b: i64) -> i64 {
    while b != 0 { (a, b) = (b, a % b); }
    a.abs()
}

#[loft_native]
#[unsafe(no_mangle)]
pub extern "C" fn n_hex(v: i64) -> LoftStr {
    loft_ffi::ret(format!("{v:#x}"))
}

#[loft_native]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_rgb_lum(store: LoftStore, pixels: LoftRef) -> i64 {
    let len = unsafe { store.vector_len(&pixels) };
    let p = unsafe { store.vector_data_ptr(&pixels) } as *const i64;
    let mut sum = 0i64;
    for i in 0..len { sum += unsafe { *p.add(i as usize) }; }
    if len == 0 { 0 } else { sum / len as i64 }
}
```

With the `build.rs` + `Cargo.toml` above, `loft --interpret prog.loft` (dlopen
+ bridge) and `loft --native prog.loft` (direct link) both just work — no
register list, no marshal glue.

---

## Discovery and loading

### Search chain

`lib_path()` in `src/parser/mod.rs` tries candidates in order:
1. `lib/<id>.loft` and `<id>.loft` relative to CWD
2. Each directory in `parser.lib_dirs` (`--lib` / `--project` flags)
3. Packaged layout: `<dir>/<id>/src/<id>.loft` for each search directory
4. Each directory in `LOFT_LIB` environment variable
5. Fallback: `<cur_dir>/<id>.loft` / `<base_dir>/<id>.loft`

When a directory `<id>/` is found, `lib_path_manifest()` reads `loft.toml`,
validates the version requirement, and resolves the entry path.

Registry packages sit at the end of that chain: the governing lockfile, then
auto-install, then the newest loadable copy already in the cache. Which lockfile
governs — and what a script with no manifest at all resolves to — is one
function, `resolution_scope(script)`, described in
[PKG_REGISTRY.md § Manifest-less resolution](PKG_REGISTRY_RESOLUTION.md#manifest-less-resolution--a-bare-script-takes-the-latest-release).
The cwd is not part of it, and a run writes no lockfile.

### Load-time sequencing

```
parse_dir(default)                           # load standard library
parse(user_script)                           # populates pending_native_libs
scopes::check(data)                          # scope analysis
State::new(database)                         # create runtime
compile::byte_code(state, data)              # bytecode gen; native::init() runs
extensions::load_all(state, pending_libs)    # dlopen cdylibs + auto-marshal
state.execute_argv("main", ...)              # run
```

### Auto-build

If a cdylib is not found but `native/Cargo.toml` exists, the interpreter
runs `cargo build --release` automatically via `auto_build_native()`.

#### An artifact is stale when any source it CONTAINS is newer (loft#777)

An auto-native cdylib is not just its own package: `emit_program` emits the
export set **and its transitive dependencies**, so `hex_editor`'s cdylib holds a
full copy of `hex_part`'s functions — and exports them under the same
`loft_shared_<name>` symbols `hex_part`'s own cdylib exports.  Whichever library
is dlopened first wins the lookup.

So the freshness question has to span every package that contributed code, not
just the one that owns the artifact (`source_newer_than` /
`source_content_hash`, `src/native_lib.rs`, take a *set* of package dirs — the
run path passes `pending_native`).  Asking only about the owner reported a
dependent as fresh after its **dependency** was edited: the edited library
rebuilt correctly, the dependent kept serving its stale inlined copy, and it won.
Permanently — nothing about the dependent's own sources ever changes again, so no
later run could clear it; only deleting `native-auto/` by hand did.

Two things made it expensive to see, and both are worth remembering:

- **It reads as a consumer-SIZE effect.**  It was filed as "a 5,900-line program
  is stale where an 8-line one tracks the edit", because you need a second
  library in the graph, loaded first, before anything can shadow the fresh one.
  A small consumer that `use`s the edited library directly was always right.
  Size was a proxy for *graph shape*.
- **The compile is fresh while the execution is stale.**  A syntax error in the
  edited library still fails startup, and the *generated* `.rs` beside the stale
  `.so` shows the NEW code — so every "is it being re-read?" check passes.  The
  discriminator is `LOFT_NO_NATIVE_LIBS=1`: same run, same program, and the
  interpreter answers differently from the cdylib.

The wider question costs one `stat` walk over the loaded packages (under a
millisecond for a ten-package tree, against a `rustc` invocation).  It does mean
a dependency edit makes every dependent stale — which is the honest answer, since
the dependent really does contain the edited code — and `dev-interpret-on-edit`
still keeps `rustc` out of the loop until editing settles.

**An auto-built cdylib links its OWN copy of libloft, so every `static` in loft exists once
per linkage unit.** A fact reached through a `#rust` body is compiled into whichever unit
calls it — the main binary, or `native-auto/libloft_auto_<pkg>.so` — so a process-global
table filled by the program is EMPTY from inside the package: `c_library_available` answered
true from the program and false from the package that declared the library, on one run, with
both backends green. "Agrees on `--interpret` and `--native`" is two configurations of three,
and a counter fed in one copy and drained in the other wraps (loft#862). Keep such facts in
the store or pass them across the boundary; never in a `static`.

### Agent discovery — generated API stubs (shipped)

Installed packages live outside the consumer project
(`~/.loft/registry/<name>-<version>/`, `~/.loft/lib/<name>/`).  Coding
agents explore the project tree, so a dependency's API is invisible to
them: `loft.toml` names the package, but no file in the project shows
what it exports.  Agents then guess signatures or re-implement what the
library already provides.

**Shipped — the dependency surface is materialized inside the project
and queryable from the shell:**

- Every command that writes or updates a lockfile (`loft install`,
  `loft update`, `loft pin`) also writes `.loft/api/<name>.api` — one
  stub per locked dependency (`write_api_stubs`, `src/main.rs`).  A
  stub holds a header (package name, resolved version, source path,
  `use <name>;` line), then per source file the `// --- Section ---`
  headers, doc-comment lines, and `pub` signatures with bodies
  stripped.
- **`loft api`** lists every library reachable from the cwd (project
  deps, installed registry packages, user libraries) with source
  paths; **`loft api <name>`** prints one library's public surface
  (newest installed version; also accepts a package path).
- The emitter is [`render_pkg_api_text`] in `src/documentation.rs` —
  the plain-text sibling of `generate_pkg_docs`, sharing
  `parse_pkg_api` + `strip_pub_body`.  Regression tests:
  `tests/api_discovery.rs`.
- Stubs are **committed**, not gitignored: small text files, API
  changes visible in PRs, and the surface stays readable in checkouts
  where `~/.loft` is not populated (CI, cloud agents).
- Staleness rides the lockfile: stubs regenerate on the same commands
  that rewrite `loft.lock`, and the header records the version, so a
  header/lock mismatch means "re-run `loft update`".
- The loft-write skill's Imports section walks agents through the
  discovery order: in-project stubs → `loft api` → `loft search` /
  `loft info`.

Known limits of the text-scan extractor, acceptable for the first cut:
`pub struct` / `pub enum` stubs keep only the declaration line (no
fields or variants), and a `pub fn` signature wrapped across lines
truncates at the first line.  The fix for both is a parser-based walk —
`Data` already records `pub_visible` per definition
(`src/data.rs:2149`) and full types — and that walker is the same one
[API_SURFACE.md](API_SURFACE.md)'s `api-lint` needs: build it once,
share it.

---

The native-side API — auto-marshalling dispatch, the `loft-ffi` crate, store allocation from native code, `loft generate` and the key source files — is in [PACKAGES_NATIVE_API.md](PACKAGES_NATIVE_API.md).

---

The package test suite, `loft build`, the build pipeline and the target matrix are in [PACKAGES_BUILD.md](PACKAGES_BUILD.md).

---

Wasm bridges, the OpenGL case study and the security model are in [PACKAGES_TARGETS.md](PACKAGES_TARGETS.md).

---

## Open work

The package **format, registry, and signing are SHIPPED**: `loft.toml`
manifests, `loft package`/`install`/`search`/`info`, `loft.lock`, Ed25519
index signing, and a bootstrapped 3-key trust root all work today —
**13 libraries are live in [loft-lang/registry](https://github.com/loft-lang/registry)**
and `loft install <name>` resolves + verifies + extracts them.  What remains is
the native-prebuilt **distribution** glue, a release to activate the trust root,
and the library-extraction arc.

| Item | Status |
|---|---|
| **PKG.REG** — registry MVP (`loft package`/`install`/`search`/`info`, resolve, sig-verify) | **SHIPPED 2026-05-24** — [PKG_REGISTRY.md](PKG_REGISTRY.md) R1–R9; 13 libs published. |
| **PKG.7** — `loft.lock` reproducible builds | **SHIPPED 2026-05-24** — `src/lockfile.rs` (= R2). |
| **PKG.SIGN** — Ed25519 trust root | **SHIPPED + MERGED 2026-06-14** — PR #371: three independent keys in `registry_keys.rs`, `scripts/registry-sign.sh` review-then-sign tool, live index signed ([REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md)).  Fully active once a loft **release** ships the embedded keys. |
| **PKG.PREBUILT** (@PLN21) — native prebuilts, no rustc to *use* a lib | **Producer SHIPPED, distribution glue OPEN.** `loft build-native` + the 4-OS `prebuild-native.yml` build cdylibs; consumer `fetch_prebuilt` loads a host-matching one.  Remaining: wire workflow artifacts → `index.json binaries[<triple>]`, the submit-CI gates, and a manylinux glibc baseline.  Scoped to **hand-written** native libs (auto-compiled libs are loft-build-locked — [plans/21](plans/21-prebuilt-native-libs/README.md)). |
| **PKG.EXTRACT** — move `lib/*/` to per-family GitHub repos | **In progress.** Libraries already live in `loft-lang/loft-libs-*` + published; the prerequisite arc (drain library `#native` code out of the compiler crate) is active — [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction). |
| **PKG.STUB** — generated API stubs + `loft api` | **SHIPPED** (stubs on install/update/pin, `loft api [name]`, `tests/api_discovery.rs`).  Remaining: parser-walk upgrade shared with [API_SURFACE.md](API_SURFACE.md) `api-lint`. |
| **PKG.FOREIGN-HOST** — a host hands a frame over as a FOREIGN store (@PLN174's tail) | **Open, a design pick.** The library side is built: a cdylib answers its buffer with no copy (`LoftStore::foreign_vector_from_owned`, loft-ffi 0.1.2) and cbor reads a frame's payloads as spans (−11 % on `check_request` over a foreign frame, `alloc-temp.md` § F6).  What is left is WHICH host produces pluginabi's frame that way — the engine's plugin channel in Rust through that bridge, or the browser through a typed array (`BROWSER_INTEROP.md`, the plan's F7) — and its cells are `174-foreign-bridge.loft`'s under the target's parity gate. |
| **PKG.CNAME** — a `[c]` library named ONCE, by identity | **Design only, not built** — [plans/24-c-abi-binding/LIBRARY_NAMING.md](plans/24-c-abi-binding/LIBRARY_NAMING.md).  Today a manifest names a library by its Linux ELF filename and every consumer recovers the identity by string surgery; four measured failures came out of that, each currently carrying its own local workaround (`-l:<file>` for the link stem, `host_lib_variants` for the probe, "at most one optional library per package" for symbol attribution).  **Trigger: the fifth one** — a new platform, or any consumer that has to re-derive a spelling from a filename. |

**Remaining, in order:**
1. **Cut a loft release** — activates the embedded trust root (PKG.SIGN); until then deployed loft has an empty trust root and ignores signatures.
2. **Prebuilt distribution glue** (PKG.PREBUILT) — on a library tag, the producer attaches per-triple cdylibs to the GitHub release (`gh release upload`), then the registry `index.json` gains a `binaries[<triple>] = {url, sha256, loft_ffi_fp}` entry (signed via `registry-sign.sh`); add the submit-CI gates + a manylinux glibc baseline.  See [plans/21 § Phase 4b / Open](plans/21-prebuilt-native-libs/README.md).
3. **PKG.EXTRACT** — continue draining the compiler crate + per-library moves via [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction).

---

## See also
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction) — moving `lib/*/` packages into external chunk repos (extraction/migration planning)
- [OPENGL.md](lib_plans/58-graphics/README.md) — OpenGL rendering design
- [OPENGL_IMPL.md](lib_plans/58-graphics/IMPLEMENTATION.md) — Step-by-step OpenGL implementation
- [WASM.md](WASM.md) — WASM architecture overview
- [WASM.md](WASM.md) — Virtual filesystem bridge steps

---


# Package Registry

The package registry has its own document: **[PKG_REGISTRY.md](PKG_REGISTRY.md)**
— the file-based `registry.json` MVP that backs `loft install <name>` (phases
R1–R9). For authoring and submitting a library see
[REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md); for governance and yanking see
[REGISTRY_GOVERNANCE.md](REGISTRY_GOVERNANCE.md).

---

# External Library Support

The package format, the native-extension binding model, discovery and
loading, the `loft-ffi` helper crate, store allocation from native code,
`loft generate`, and the per-target build pipeline are all documented in
the [Library Package Format](#library-package-format) section above.

The execution arc for moving the in-tree `lib/*/` packages out into
per-family external GitHub repos — the library inventory, the
stdlib-vs-library boundary, chunk topology + dependency graph, the
per-chunk extraction template, release workflow, current state, the
shipped-libraries catalog, and the open migration questions — lives in
[`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction)
(see its [REFERENCE.md](lib_plans/12-library-extraction/REFERENCE.md) for
the durable "how it works" reference and [README.md](lib_plans/12-library-extraction/README.md)
for current status).
