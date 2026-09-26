
# Library Package Format

Design for a unified packaging format that supports pure-loft libraries,
native Rust extensions, and WASM targets — with OpenGL as the driving
use case.

---

## Contents
- [Goals](#goals)
- [Current state](#current-state)
- [Package layout](#package-layout)
- [Manifest: `loft.toml`](#manifest-lofttoml)
- [Package dependencies](#package-dependencies)
- [Function binding model](#function-binding-model)
- [Discovery and loading](#discovery-and-loading)
- [Auto-marshalling dispatch (interpreter, legacy path)](#auto-marshalling-dispatch-interpreter-legacy-path)
- [loft-ffi helper crate](#loft-ffi-helper-crate)
- [Store allocation from native code](#store-allocation-from-native-code)
- [Code generation: `loft generate`](#code-generation-loft-generate)
- [Key source files](#key-source-files)
- [Package test suite](#package-test-suite)
- [Build pipeline](#build-pipeline)
- [Target matrix](#target-matrix)
- [OpenGL case study](#opengl-case-study)
- [Security model](#security-model)
- [Implementation phases](#implementation-phases)
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

### `[[font]]` — the font a browser page has to carry

Under `--html` a font path is never opened: `gl_load_font("fonts/Foo.ttf")` reaches the
page as the base name `Foo`, which the browser bridge resolves to a CSS family. So the
page must already carry the family, under exactly that name.

Declaring neither `url` nor `stylesheet` means the browser already has the family and
the page brings nothing. `url` names a font FILE we serve and becomes an `@font-face`;
`stylesheet` names a provider's stylesheet and becomes a `<link>`. `--html` also emits a
`document.fonts.load` await for every declared family ahead of `loft_start`, because a
webfont that arrives after the first frame draws is a fallback nobody was told about.

`family` and the base name of `native` must agree, and `--html` refuses the build when
they do not — before the wasm compile. The failure it replaces is silent: the page
registers one family, the program asks for another, text draws in a generic face and
nothing anywhere says so. Full reference:
[HTML_EXPORT.md](HTML_EXPORT.md).

### `[[embed]]` — the file a browser page carries

A `--html` page has no disk. `[[embed]]` puts a file in the page's own filesystem, under
the exact name the program reads it by, so `store_load(q, "assets/game.pack")` is the
same call on every target. `source` says where the bytes are on the build box — a pack
is usually generated by a `[[build.asset]]` step — and defaults to `path`.

Both resolve against **the program's directory**, not the manifest's, because that is
where loft resolves any path a program passes: `assets/game.pack` in `src/game.loft` is
`src/assets/game.pack`. So a `[[build.asset]]` step must write where the program reads,
and a step that exits zero without producing its declared `outputs` is reported as
failed rather than `✓`. A library's declarations resolve against the library.

`path` is the PROGRAM's path, so it must be relative and in normal form. `--html`
refuses `/abs/game.pack`, `./assets/game.pack`, `a/../b`, a `source` that is not there,
and one name declared from two files — all before the wasm compile. Each would
otherwise be carried under a key nothing asks for, and a page that cannot find its pack
says nothing at all.

This is the exception rather than the pipeline: assets normally travel as a store on a
dumb file server, read by HTTP range so only the bytes a lookup touches cross the wire.
Embedding is for what a page needs before its first fetch, and for a page that has to
be one self-contained file. The bytes go in base64, so the page grows by about 4/3 of
the file. Full reference:
[plans/146-content-delivery/F4.md](plans/146-content-delivery/F4.md).

### `placement` — where the library runs

`[library] placement = "process"` puts the library in a **worker process**.
Consumers do not change: the same `use`, the same typed `pub fn` calls, the same
values back. What changes is containment — a crash inside the library aborts the
call as a loft error instead of corrupting the caller's stores.

The **library** declares it, not the consumer, because the library is what knows
whether isolating it is safe and worth the crossing. The invariant is that you
cannot tell from the program:

> A call to a library is indistinguishable — in type, effect, ownership/lifetime,
> and error behaviour — from the same call in-process. Where it runs is
> deployment policy, not source.

`tests/placement_parity.rs` is that sentence as a test: one consumer, one
library, run under every placement, requiring identical stdout, stderr and exit
status.

**Writing a library that can be placed** — and the mechanism underneath — is
[PLACEMENT.md](PLACEMENT.md).  Four rules decide it, and each is better
in-process too: a public function must not BE a native, answer a value rather
than a cursor, closures do not cross, and a function that returns a VIEW of
something it did not create cannot be placed.

What crosses today: **every scalar** — integer at any declared width and with
its sign, `single`, boolean and text — and **structs and vectors**, in both
directions, to any nesting depth: a `text` field, a struct inside a struct,
`vector<struct>`, `vector<text>`, `vector<vector<T>>`, an empty vector, a null
struct. A `pub fn` outside that runs in-process, byte-identically; it is never
turned into a call that fails later. A polymorphic enum and a keyed collection
(`hash` / `index` / `sorted`) are outside for now — both are reference-shaped
and so look placeable, but their crossing has questions of its own.

A struct or a vector does not travel as bytes in the request. It is a graph of
records, and it crosses as itself, in a **shared store both processes map**.
That is why the crossing does not get proportionally more expensive as the value
gets bigger: it is a copy, not an encoding. Measured, a 16-element vector costs
about the same to cross as a two-field struct, and a 4096-element one adds
nothing measurable to a call that then loops over it.

**A compound argument is passed by reference, and stays that way.** In loft, a
`pub fn bump(p: Point)` that assigns `p.x` changes the CALLER's `p`, and a
vector parameter appended to grows the caller's vector. That holds across the
boundary too — a placed library that writes to a parameter is not quietly
writing to a copy. Passing the same value twice (`f(p, p)`) also stays one
value, as it is in-process.

**Mark what you only read `const`, and a placed call gets cheaper.** Carrying a
compound argument home again afterwards is the expensive half of a crossing, and
`const` is a compile-time promise that there is nothing to carry — loft already
rejects every mutation through such a parameter, so the crossing skips it. On a
call taking a 20 000-element vector that is about a tenth of its cost. It changes
nothing about what the program does, in-process or placed.

**A library whose public surface IS its natives cannot be placed**, and the fix
is one line per function. Placement works by giving a function a native symbol,
so a function that already has one is skipped — and a library made entirely of
`pub fn f(…); #native` would be marked nowhere and quietly run every call in the
caller. Make the native private and let the public name be a wrapper over it:

```loft
fn kernel_send(cid: integer, msg: text) -> boolean;
#native

pub fn send(cid: integer, msg: text) -> boolean { kernel_send(cid, msg) }
```

Consumers do not change — the public name and signature are the same — and the
wrapper is what gets placed.

**Shape the surface for a call, not for a cursor.** An API of "advance, then read
field, then read field" costs one crossing per read. One that answers a whole
value costs one crossing, whatever it contains — and it is usually the nicer API
anyway. `engine_host`'s `turn()` hands back a frame's entire event list for this
reason, where the underlying kernel offers a cursor.

**Closures do not cross.** A library whose entry point takes a function
(`run(port, on_event, on_tick)`) can only be driven in-process; give it a form
where the caller owns the loop as well, and both work.

**One shape is quietly not placed**, and it is worth knowing which: a function
that returns a *view* of something it did not create — `fn head(v: vector<P>) ->
P { v[0] }`, or a reference into the library's own long-lived state. In-process
the caller gets a borrow and frees nothing, which is right; across a process
boundary there is nothing to have a view OF, since the thing lives in the other
process. Such a function runs in-process. Returning a fresh value instead (`P {
x: v[0].x, … }`) makes it placeable.

One text return is worth knowing about because the rule is invisible: a function
whose text return is a **constant**, `fn version() -> text { "1.0" }`, is not
placed. The compiler gives such a function no result buffer, so there is nowhere
in the caller for the answer to live. It runs in-process — the same program —
but it is not isolated.

### `placement = "remote"` — the library runs on another machine

Same declaration, one word further. The library says it is meant to run as a
service; where it runs is deployment, so the consumer's environment says that:

```bash
# where it runs — started by whoever deploys it
loft --lib-server 127.0.0.1:9119 /srv/libs/pricing

# where it is called from
LOFT_REMOTE_PRICING=127.0.0.1:9119 loft report.loft
```

The consumer's source is unchanged, again — the same `use`, the same typed calls,
the same values back, structs and vectors included. What travels is the value's
own layout: a loft store is a self-contained image, so a `vector<Order>` goes on
the socket as bytes rather than as an encoding of itself, and comes back the same
way. Writes to a compound parameter still reach the caller.

A remote call costs around **25 µs** on a local network interface, against ~1 µs
for a worker on this machine — almost all of it the round trip, not the data, so
the size of what you pass barely moves it. That makes it a fit for calls that do
real work and a poor one for a getter.

**A library with nowhere to run refuses**, naming the variable to set, rather
than quietly running in-process: a library declared remote works on data that is
over there, so running it here is a different deployment, not a slower one.

`--lib-server` serves exactly the library you name and nothing else. It is not
authenticated, not encrypted, and not a sandbox: it runs that library's functions
for whoever connects. Bind it where only what should reach it can — `127.0.0.1`
for a local test, a private network or a tunnel otherwise.

Three limits worth knowing before reaching for it:

- **Calls to one placed library serialise** (the wire has a single request slot),
  so it is a poor fit for a hot `par` arm and a good fit for coarse calls. A
  placed call costs roughly **0.8 µs** for scalars and **1.4 µs** carrying a
  struct or a vector, against ~50 ns for a native in-process one — so it pays for
  itself on a call that does real work, and does not on a getter in a loop.
- **Linux only.** Elsewhere the library runs in-process — the same program,
  without the isolation. (A `remote` library is Linux-only on the CALLING side
  for the same reason; the server can be anywhere loft runs.)
- **Not under `--native`,** which compiles the library's own body into the
  program binary, so its calls never leave the process.

`LOFT_REQUIRE_PLACEMENT=1` turns either of the last two from a silent fallback
into an error that names which one applied.

A misspelled value is refused at load rather than treated as `inproc`: the two
placements are meant to behave identically, so a typo would otherwise yield a
program that runs correctly and isolates nothing, with no output ever showing it.

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
| `"=0.2.0"` | Exactly 0.2.0 |
| `{ path = "../math" }` | Local directory (no version check) |
| `{ version = ">=1.0" }` | Same as string form, explicit syntax |

No caret (`^`) or tilde (`~`) ranges — only `>=` and `=`.  This keeps
the resolver simple and predictable.

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

### Direct C binding — `#c` (@PLN24)

`#native` above is the **Rust** binding: a hand-written `extern "C"` fn compiled
by rustc into a cdylib.  A planned sibling annotation, **`#c "<symbol>"`**, binds
a loft function **straight to a C-library symbol — no Rust wrapper, no rustc, no
libffi**.  loft-core `dlopen`s the system library and calls the symbol through a
small **fixed per-arity C-ABI caller** (`extern "C" fn(u64, …) -> u64`, one per
arity — integer-class args collapse to a `u64` slot); the library is **pure loft**
(`#c` decls) plus, for signatures the caller can't express (float / struct-by-
value / varargs arguments), a `cc`-compiled **ANSI-C shim**.  Native-only — a
wasm module cannot `dlopen` a shared library, so both wasm targets refuse a
reachable `#c` call by name (see [below](#the-wasm-and-browser-targets--arc-e)).
It is the foundation for binding system C libraries
(databases, codecs, …) without the rustc toolchain, keeping loft-core minimal —
the linking tool lives in core, all complexity in the library + shim.

**Two things the architecture probe settled**, before anyone writes a binding.
The per-arity caller works for **arguments** — int, long, pointer, `char *`,
bool all cross correctly at every arity, including across the register/stack
boundary — but **not for returns**: a 32-bit C return read back as 64 bits turns
−1 into 4294967295, quietly.  So the declaration carries the C signature
(`#c "PQstatus" "int(void*)"`), and it is the **sole** authority: pointed at a
wrong arity or a variadic function, the caller returned the *right answer* by
luck, so there is no runtime signal to catch a mismatch — the check is at
compile time or nowhere.  Second: `#c` is the declared edge of loft's
no-runtime-errors rule.  Arguments cost nothing (non-null is already the default
and null-flow rejects a `τ?` at compile time), a NULL pointer return maps to
loft null, and a fault *inside* C is undefined — the same failure mode `#native`
already has, through the same crash handler.

#### The declaration (arc A — implemented, inert)

```loft
pub fn status(conn: integer) -> integer;      #c "PQstatus" "int(void*)"
pub fn error(conn: integer) -> text?;         #c "PQerrorMessage" "const char*(void*)"
pub fn sum(v: vector<integer>) -> integer;    #c "lc_sum" "long(const long*, long)"
```

Both strings are required.  The C signature is `<return>(<params>)` in ordinary
C spelling, and **widths resolve against the target**, exactly as a C compiler
reads the same header: `long` is 64 bits on Linux and macOS and 32 on Windows,
plain `char` follows the platform's signedness.  So one declaration stays
correct everywhere.  An unknown type is refused, never guessed.

**What a loft type looks like from C** — the mapping the arity check counts in:

| loft | C | notes |
|---|---|---|
| `integer`, narrow ints, `boolean`, `character` | one C integer | any width; the value is passed full-width |
| `text` argument | one `const char*` | **NUL-terminated**, unlike the `#native` path's `ptr, len` |
| `text` / `text?` RETURN | `char*` | the bytes are **copied** up to the first NUL; loft never frees the pointer |
| `vector<T>` | **two**: element pointer + count | C carries no length.  The pointer is valid *for the call only* — see the retention hazard below |
| `vector<float>` | `double*` + count | C may **write through it**; the writes are visible to loft |
| C-owned handle (`PGconn *`) | `void*` ↔ loft `integer` | the pointer value crosses as an integer |
| `float` / `single` SCALAR | — | **refused**: floats travel in SSE registers a fixed caller does not touch — pass it by pointer as a 1-element `vector<float>` |
| a loft record | — | **refused**: records live in a store that may move them |
| a nullable `τ?` ARGUMENT | — | **refused at compile time**: C has no null model |

The last two refusals are the design, not gaps.  A record's address is a
position in an arena the allocator can relocate, so handing it to C is the
store-lifetime bug class rather than a marshalling detail.  And `#c` is the
declared edge of loft's no-runtime-errors rule: a null crossing *into* C would be
an ordinary number or a fault, so it is rejected where loft still can — which
costs nothing, because non-null is already the default and null-flow already
requires a discharge (`?? 0`, `x?`, `match`).

**Two shape limits worth knowing before you write a binding.**  A `vector<T>`
becomes an element pointer **immediately followed by** its count, so `write(fd,
ptr, n)` binds directly while `memchr(ptr, ch, n)` and `fwrite(ptr, size, n, f)`
— which separate the pair — need a shim.  And a binding may declare at most **32**
C parameters: the interpreter calls through a fixed ladder of per-arity
trampolines, and that ladder is where the ceiling comes from.

**One ceiling, both backends** (@PLN128 arc C).  It used to be 12 and it was
enforced on the interpreter ONLY, which made `#c` two different languages: a
13-slot binding compiled under `--native`, shipped, and failed for whoever
interpreted it — including `loft debug`, which *is* the interpreter, so the
bindings you could not debug were exactly the ones with no other way in.  The
author never saw it either, because the refusal fired at the call site rather
than the declaration.

Unifying downward would have narrowed what already compiles, so the ladder was
extended to 32 instead and `--native` held to the same number.  32 was sized
against `dgemm_` when a by-reference scalar was still thought to cost two slots
(pointer + count); since a `vector` carries a count only where the C signature
has one, `dgemm_` costs **13** and LAPACK's 20-argument drivers fit with room to
spare.  Past 32 a shim is the answer — a ladder cannot be unbounded.

The check now runs on both backends, in two places: at the **declaration**, for
code you own (your program, your library, the stdlib), so you see it as you write
it; and at any **call site**, including one reaching into a dependency, because
that call genuinely cannot work.  Merely *loading* a dependency that declares an
over-ceiling binding you never call is fine — a consumer cannot edit someone
else's declaration, so it must not fail their build.

#### Numeric libraries — what the boundary already does (@PLN128)

BLAS/LAPACK, FFTW, HDF5 and GSL are the same three shapes over and over: an array
in, an array written back, and a scalar passed by reference.  Every cell below was
measured on **both backends** against a C oracle that computed the expected value
itself:

| shape | `--interpret` | `--native` |
|---|---|---|
| read a `double` array — `vector<float>` → `(const double*, int64_t)` | ✅ | ✅ |
| **C writes back through `double*`** (the `daxpy` shape) | ✅ | ✅ |
| 1-element `vector<integer>` as a Fortran scalar `const int64_t*` | ✅ | ✅ |
| a scalar `double` in and out through a pointer shim | ✅ | ✅ |
| **a full Fortran argument list — `dgemm_`'s thirteen bare pointers** | ✅ | ✅ |
| counted and bare vectors **mixed in one signature** | ✅ | ✅ |
| 14 C argument slots | ✅ | ✅ |
| **a `double` RETURNED by value** (`ddot_`, `dnrm2_`) | ✅ | ✅ |
| 33 C argument slots (past `MAX_C_ARITY`) | ❌ refused | ❌ refused |
| a `double` **argument** by value | ❌ refused | ❌ refused |
| a `vector` whose element width differs from the C pointee | ❌ refused | ❌ refused |

Every row above was re-measured against **real OpenBLAS** — `daxpy_`, `dgemm_`,
`dgesv_`, `ddot_`, `dnrm2_` as the library exports them — not only against a
fixture written for loft.

**The write-back cell is the one that matters.**  Every BLAS and LAPACK routine
returns its result by writing through a caller-supplied pointer, so a boundary
that could not carry those writes could not bind the numeric stack at all.  It
carries them.

**A scalar float crosses as a 1-element `vector<float>`** — this is the answer to
the most common question, and the cure the float refusal now names:

```loft
// C:  void shim_scale(double *out, int64_t n_out, const double *v, int64_t n_v)
pub fn shim_scale(out: vector<float>, v: vector<float>);
#c "shim_scale" "void(double*, int64_t, const double*, int64_t)"

v: vector<float> = [2.5];        // a COMPUTED double, not a hand-converted literal
out: vector<float> = [0.0];
shim_scale(out, v);              // out[0] == 5.0
```

There is deliberately no float→bits builtin, so a shim taking "the bit pattern as
an integer" is **not** a route a real program can take: `x as integer` is a value
cast (`2.5` → `2`).  Pointers are the whole answer.

##### A `vector` carries a count only where the C signature has one

C carries no length, so a `vector` normally crosses as **pointer then count** —
and a C library written for loft takes exactly that. Fortran does not. BLAS and
LAPACK pass **every** argument by reference, so each one is a bare pointer and
the routine learns the length from a separate `n`, itself a bare pointer.

**Both are supported, and the C signature is what chooses.** Write the signature
the header shows you and the count appears exactly where the header puts one:

```loft
// A C library written for loft: the count is in the signature, so loft sends it.
pub fn lc_i64_sum(v: vector<integer>) -> integer;
#c "lc_i64_sum" "int64_t(const int64_t*, int64_t)"

// Fortran: thirteen by-reference arguments, thirteen bare pointers, no counts.
pub fn dgemm(transa: text, transb: text, m: vector<integer>, n: vector<integer>,
             k: vector<integer>, alpha: vector<float>, a: vector<float>,
             lda: vector<integer>, b: vector<float>, ldb: vector<integer>,
             beta: vector<float>, c: vector<float>, ldc: vector<integer>);
#c "dgemm_" "void(const char*, const char*, const int64_t*, const int64_t*, const int64_t*, const double*, const double*, const int64_t*, const double*, const int64_t*, const double*, double*, const int64_t*)"
```

So **a Fortran routine costs one slot per argument**: `dgemm_` needs 13, not 26,
and even LAPACK's largest drivers (20+ arguments) fit under the 32-slot ceiling
without an argument-collapsing shim. Pointer-and-integer APIs (HDF5, GSL) spend
one slot per scalar and are nowhere near it either — though see the retention
hazard below before reaching for one that keeps your buffers.

A **scalar** by reference is a 1-element vector, because loft has no address-of:
`m: vector<integer> = [2]` is how you pass Fortran's `const int64_t *m`. A
`character*1` argument is `text` — `"N"` crosses as a NUL-terminated `const
char *`, which is what a C caller passes.

Two things to know before binding real Fortran:

- **Reading of the signature is unambiguous, but only the signature is checked.**
  Omit a count the C function really takes and the declaration is now *accepted*,
  and passes a bare pointer where the callee wants a length. Nothing at runtime
  can catch that — the same standing rule as any other wrong `#c` signature.
- **Hidden string lengths.** A Fortran compiler appends a hidden length argument
  per `character` argument. Reference BLAS and LAPACK do not read them for the
  length-1 flags, which is why C callers pass `"N"` and nothing else, but a
  routine that takes a real Fortran string needs a shim that supplies them.

##### The element type must be the one the C header spells

A `vector` reaches C as a pointer into **loft's own element bytes** — nothing is
converted on the way — so the loft element type and the C pointee are two
spellings of one layout, and a declaration where they disagree is refused:

| loft element | write the pointee as |
|---|---|
| `integer` | a 64-bit integer (`int64_t`, `long` on LP64) |
| `i32` / `u32` | a 32-bit integer (`int`) |
| `u16` | a 16-bit integer (`short`) |
| `u8` / `boolean` | an 8-bit integer (`char`) |
| `character` | a 32-bit integer — the codepoint |
| `float` | `double` |
| `single` | `float` |
| `i8` / `i16` | **refused** — see below |
| `vector<text>` | **refused** — the elements are loft's handles, not `char *` |
| anything above | `void *` — always accepted, and means "these are bytes" |

Signedness is not checked: it changes how a byte is read, never where the next
one starts, so `vector<u8>` against `const char *` is the ordinary byte-buffer
idiom.  `i8` and `i16` are refused because loft stores narrow **signed** elements
as `val - min`, so a loft `0` is the byte `128` — use `i32`, the unsigned
sibling, or `void *` if you really mean raw bytes.

**The width the C header uses is the one to write, and it is not always obvious.**
BLAS and LAPACK ship in two builds: LP64 (Fortran `INTEGER` is 32-bit, which is
what `libopenblas.so.0` on a typical Linux is) and ILP64 (64-bit).  The same
symbol names serve both, so *nothing but the header tells you which is
installed* — and the wrong choice is quiet:

```loft
// Against an LP64 build this reads the right answer for `n`, because the low
// four bytes of an 8-byte 3 are 3 — and then `info` and `ipiv`, which C WRITES,
// come back as -4294967296 and 8589934593.
pub fn dgesv(n: vector<integer>, …);   #c "dgesv_" "void(const int64_t*, …)"
```

Declare the widths your build uses and the check above holds you to them
consistently; it cannot tell you which build is on the machine.

##### A `double` return comes back; a `double` argument still does not

The level-1 BLAS *functions* answer by value — `ddot_`, `dnrm2_`, `dasum_`, and
LAPACK's `dlange_` / `dlamch_` — and that binds directly:

```loft
pub fn ddot(n: vector<i32>, x: vector<float>, incx: vector<i32>,
            y: vector<float>, incy: vector<i32>) -> float;
#c "ddot_" "double(const int*, const double*, const int*, const double*, const int*)"
```

A C `double` returns as loft `float` and a C `float` as loft `single`, and the
pairing is exact — a C `float` leaves a *single* in the return register, so
binding it to `float` would read those bits as a denormal.

A float **argument** is still refused, and that asymmetry is deliberate rather
than unfinished: the register file is chosen per argument position, so supporting
one anywhere would need a trampoline per subset of positions, while the return is
a single axis.  It costs nothing in practice — Fortran passes every argument by
reference, so a numeric binding has no by-value floats to pass.

##### The retention hazard — a pointer C keeps is a use-after-free

`vector<T>` hands C a pointer **valid for the duration of the call**.  loft cannot
see that C stored it, so the vector's lifetime still ends at its last *loft-visible*
use — and a later C read then hits memory loft has reused, silently:

```loft
a: vector<float> = [1.5, 2.25, 4.0];
retain(a);                       // C keeps the pointer; `a` has no later use
b: vector<float> = [100.0, 200.0, 400.0];
reread();                        // reads `b`, not `a` — measured, both backends
```

That reads another variable's data with no fault and no diagnostic.  Adding any
later use of `a` keeps it alive and the read is correct — which is precisely what
makes the bug dangerous: it appears and disappears with edits that look unrelated.

This is the FFTW plan API's exact shape (`fftw_plan_dft` retains the buffers,
`fftw_execute` reads them later), and it is not an FFTW quirk: zlib's `z_stream`
keeps `next_in` / `next_out`, `sqlite3_bind_text(…, SQLITE_STATIC)` keeps your
bytes, and every "context object" API is this.

##### Binding a retaining API: give C the buffer

**The cure is that C owns the memory.**  Allocate on the C side, hold the pointer
as an opaque `integer`, and copy in and out — then nothing loft owns is retained,
the buffer's lifetime is the handle's, and each copy lives only for its own call,
which is what `#c` already guarantees.  It needs no shim, because `memcpy` is
libc and needs no `[c] libs` entry:

```loft
pub fn fftw_malloc(nbytes: integer) -> integer;   #c "fftw_malloc" "void*(size_t)"
pub fn fftw_free(p: integer);                     #c "fftw_free" "void(void*)"
pub fn plan_dft_1d(n: integer, inp: integer, outp: integer, sign: integer, flags: integer) -> integer;
#c "fftw_plan_dft_1d" "void*(int, double*, double*, int, unsigned)"
pub fn execute(p: integer);                       #c "fftw_execute" "void(const void*)"

// The two crossings.  Same libc symbol; only the DIRECTION differs, which is
// what puts the loft vector on a different side of the boundary each time.
pub fn load(dst: integer, src: vector<float>, nbytes: integer) -> integer;
#c "memcpy" "void*(void*, const void*, size_t)"
pub fn store(dst: vector<float>, src: integer, nbytes: integer) -> integer;
#c "memcpy" "void*(void*, const void*, size_t)"
```

```loft
inp = fftw_malloc(n * 16);  outp = fftw_malloc(n * 16);
p = plan_dft_1d(n, inp, outp, -1, 64);   // FFTW_FORWARD, FFTW_ESTIMATE
load(inp, src, n * 16);
execute(p);
store(dst, outp, n * 16);
```

This is not a workaround for a gap — for FFTW it is what the library's own
documentation tells C callers to do, because `fftw_malloc` is what supplies the
SIMD alignment a caller array would forfeit.  Measured on both backends against
a C program computing the same transform.

**What it costs.** Two O(n) copies per call, against an FFT's O(n log n) of work.
And `nbytes` is **your** arithmetic: a `void *` pointee is the opaque escape
hatch, so the element-width check above does not apply and a `store` sized larger
than the destination writes past the end of the vector.  Size the copy off the
same expression that sized the allocation.
There is no annotation for "C keeps this pointer"; until there is, the rule is the
one at the top of this paragraph, and it is a rule about *your* code, not a
guarantee loft enforces.

**The `char *` return — three answers C's type system cannot give**, so the
binding gives them, the same way on every backend:

- **loft never frees it.**  `strerror` and `PQerrorMessage` hand back storage the
  caller must *not* free; `strdup` hands back storage it must.  `const` does not
  separate them — POSIX spells both `char *` — so a guess would free static
  memory, and that failure is not recoverable while a leak is.  A **caller-frees**
  function therefore goes through an ANSI-C shim, which is what shims are for.
- **The bytes end at the first NUL**, because that is what `char *` means.  A loft
  `text` carries a length and may hold an interior NUL; the crossing truncates
  there rather than inventing a length.
- **NULL is loft null, and invalid UTF-8 is replaced** (loft text is UTF-8; a
  locale-encoded byte from C must not take the program down).  Spell the return
  **`text?`** when NULL is a real answer — it does not add the null, it makes the
  null-flow analysis demand a discharge for it, which a bare `text` carries
  silently.

A pointer return that is *not* spelled `char *` is refused against a `text`
declaration: `void*` bound to `text` is either a mistake or a handle that wanted
`integer`, and nothing at runtime tells the two apart.

#### Declaring the library (arc D)

```toml
[c]
libs = "libpq.so.5"            # a soname the dynamic linker knows
# libs = "../../libmine.so"    # or a path, resolved against the package dir
optional-libs = "libduckdb.so" # bound, but not required to be installed
shim = "src/shim.c"            # ANSI-C loft compiles itself, with `cc`
```

The interpreter `dlopen`s each entry and keeps it loaded (a `#c` symbol is
looked up through it); `--native` links the same list.  One declaration, both
halves.  A binding to **libc needs no entry** — it is already in the process.

Distinct from `[native] runtime-libs`, which names what a Rust cdylib needs
present and only probes for it.

#### Optional libraries — `optional-libs` (arc G)

**`libs` means the package does not work without it.**  Absent, the failure is
early and actionable: the interpreter reports it, and `--native` will not even
link.  That is the right answer for a package's one reason to exist.

**`optional-libs` means the package binds it but works without it.**  It is not
linked and not opened at load; it is opened when a symbol from it is first
looked up, and `--native` resolves it at that moment too instead of putting it
on the link line.  So a program that never calls into it **builds and runs on a
machine where the library is not installed** — which is what lets one package
offer several backends without making a user install all of them to use one.

The cost is that presence becomes a question the program must ask:

```loft
if c_library_available("libduckdb.so") { … } else { /* fall back */ }
```

Ask it *before* the first call.  A `#c` symbol that cannot be resolved **faults**
— `#c` is the declared edge of loft's totality, not a null-returning
computation — so this query is what keeps an optional backend inside the
no-runtime-errors rule (C80).

`c_library_available` answers true when the library loads **and** every `#c`
symbol attributable to it resolves.  Both halves matter: a library of the wrong
vintage loads and exports only some of its symbols, so "the file is there" would
say yes where the call still faults.

**Declare at most one optional library per package.**  A `#c` annotation never
names the library it comes from, so symbols are attributed by package — and with
two optional libraries in one package nothing says which one exports what.  Such
a package still answers the load question correctly but gives up the skew half.
Required entries do not count: a package cannot load without them, and one of
them is usually its own `shim`, which loft just built.

**`shim`** names ANSI-C sources the package ships for the signatures the fixed
trampolines cannot express — a `double` argument, a struct by value, varargs, an
out-parameter, a caller-frees `char *` return.  loft compiles them with **`cc`,
never rustc** (the whole point of `#c`), and the result is then registered
*exactly like a `libs` entry*: nothing downstream can tell a shim from any other
C library, so the interpreter's `dlopen`, the `--native` link line and the symbol
resolver stay one code path.

The artifact lands in the package's `native-auto/` and is **content-addressed**
by the shim sources plus the compiler's identity — editing a shim produces a
different file rather than racing to overwrite one another process may be
reading, and a toolchain change rebuilds rather than reusing a stale ABI.  An
existing artifact IS the freshness check, so a warm run costs nothing.

`loft install` builds it at install time, so a package needing a C compiler says
so while the user is installing packages, not inside the first run of their
program.  A failure there is surfaced and the install still succeeds — the
parser reports it again, with the `use` site, if the package is actually used.

#### The wasm and browser targets — arc E

**A `#c` binding is refused on both wasm targets, at the call.**  `--native-wasm`
(wasip2) and `--html` (the browser) each report one message naming the loft
function, the C symbol, the declaring package and the target:

```
error: loft: `client_info` is bound to the C symbol 'mysql_get_client_info' with #c
       (package `mariadb`), and the wasm (wasip2) target has no C ABI to reach it —
       a wasm module cannot open a shared library. Give the library a wasm
       implementation, host it out of process (@PLN119), or drop the
       --native-wasm claim (@PLN24 arc E)
```

**Refused at the CALL, so a declaration is still portable.**  A library may
declare `#c` bindings and still build for wasm as long as the wasm program does
not reach one — the same rule `#native` follows for a routeless browser symbol.

**Why refusal rather than support**, in the order the measurements came in.
`wasm32-wasip2` links a libc, so a binding to `strlen` did *resolve* — and then
trapped, because wasm32 is a third data model (ILP32: `long`, `size_t` and every
pointer are 32 bits) while the extern carried the host's widths.  A symbol the
sysroot does *not* export gave a raw linker error naming neither package nor
library.  Neither is a capability a library can rely on: a `#c` library binds a
system library, and there is no `dlopen` in wasm to reach one with.

The two routes that do work are a **wasm implementation of the library** (the
same answer `#native` needs — a `[wasm.bridge]`), or **hosting the C out of
process** ([@PLN119](https://github.com/loft-lang/plans/issues/119)), where
"another process" and "another machine" are already one mechanism.  Compiling a package's
own `[c] shim` to wasm with a C cross-compiler is a third, and is not built: it
needs a `wasm32-wasi` C toolchain in the build environment, and it would cover
only a shim that is pure computation — never a database client, whose capability
does not exist in a browser at all.

`c_library_available` compiles on both wasm targets and answers **false** there,
so the guard an optional backend already writes keeps working when the same
source is built for wasm.

#### Under the sandbox

**A `#c` binding is gated by `native_ffi`, never by a `#cap` grant.**  A
capability says what data a script may touch; a C call runs machine code loft
cannot inspect, which is the line [`native_ffi`](SANDBOX.md) already draws for a
Rust cdylib bridge — and `#c` is the stronger case, having no marshalling layer
at all.  An allow-listed library (`allow_libs`) still admits its bindings: that
is the host vetting the library as a unit, exactly as for `#native`.

**Status: done on both backends and defined on all four targets** — `--native`
compiles the declaration into a typed `extern "C"` and calls it directly; the
interpreter resolves the symbol and calls it through a fixed ladder of per-arity
trampolines.  The two produce identical results, which is the bar.  Design in [plans/24-c-abi-binding](plans/24-c-abi-binding/README.md) /
[@PLN24](https://github.com/loft-lang/plans/issues/24) (first consumer: the
MariaDB/PostgreSQL clients, @PLN23), matrix + probe in
`tests/fixtures/c_abi/`.  `#native` remains today's path.

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
your registration on both backends (NATIVE.md § Which symbol a `#native` binding
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
[PKG_REGISTRY.md § Manifest-less resolution](PKG_REGISTRY.md#manifest-less-resolution--a-bare-script-takes-the-latest-release).
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

## Auto-marshalling dispatch (interpreter, legacy path)

The `#[loft_native]` bridge above (§ Function binding model) is the modern
zero-boilerplate path.  Libraries not yet migrated to it fall back to the
generic auto-marshaller in `src/extensions.rs`, which bridges loft stack
values to C-ABI calls without per-function glue code.

### How it works

1. **`compute_sig()`** reads the `#native` definition's types and produces a
   compact `NativeSig { params: Vec<ArgT>, ret: Option<ArgT> }`.

2. **`wire_native_fns()`** iterates all `#native` definitions, resolves symbols
   via dlsym, and replaces panic-stubs with the generic `native_auto_dispatch`.

3. **`native_auto_dispatch()`** pops arguments from the loft stack in reverse
   order, builds typed `ArgVal` values, and calls `dispatch_call()`.

4. **`dispatch_call()`** pattern-matches on the signature and calls the native
   function pointer with the correct C-ABI cast.

### Type mapping

| Loft type | ArgT | C-ABI type |
|-----------|------|-----------|
| `integer` (plain, no `size(N)`) | `I64` | `i64` |
| `character` / narrow int (`i8`/`u8`/`i16`/`u16`/`i32`) | `I32` | `i32` |
| `float` | `F64` | `f64` |
| `single` | `F32` | `f32` |
| `boolean` | `Bool` | `bool` |
| `text` | `Text` | `*const u8, usize` |
| struct / vector / collection | `Ref` | `LoftRef` (with `LoftStore` prepended) |

When any parameter or return type is `Ref`, a `LoftStore` handle is prepended
as the first C-ABI argument, giving the native function access to store memory.

For functions returning `Ref` with no `Ref` parameters (e.g. `rand_indices`),
the dispatcher allocates a fresh store for the result automatically.

### Thread-local state

During a native call, a thread-local `CURRENT_STORES` holds a raw pointer
to the interpreter's `Stores`. This enables the `LoftStore` allocation
callbacks to reach back into the interpreter for `claim()` and `resize()`
operations.

---

## loft-ffi helper crate

The `loft-ffi` crate (`/loft-ffi/`) provides safe building blocks for native
extension authors. No dependencies.

### Core types

**`LoftRef`** — Opaque reference to a store object (struct, vector, collection):
```rust
#[repr(C)]
pub struct LoftRef {
    pub store_nr: u16,
    pub rec: u32,
    pub pos: u32,
}
```

**`LoftStore`** — Direct memory access to a store buffer, with allocation callbacks:
```rust
#[repr(C)]
pub struct LoftStore {
    pub ptr: *mut u8,                    // base pointer (may move on alloc)
    pub size: u32,                       // capacity in 8-byte words
    pub ctx: LoftStoreCtx,              // opaque context for callbacks
    pub claim_fn: ...,                   // allocate words → rec
    pub reload_fn: ...,                  // refresh ptr/size after alloc
    pub resize_fn: ...,                  // resize record → new rec
}
```

**`LoftStr`** — `#[repr(C)]` text return type (borrowed pointer, valid until
next `ret()` call on the same thread).

### Text helpers

```rust
// Convert C-ABI text parameter to &str
let name = unsafe { loft_ffi::text(name_ptr, name_len) };

// Return a String as LoftStr (stored in thread-local buffer)
loft_ffi::ret(format!("Hello, {name}!"))

// Return a borrowed &str without copying
loft_ffi::ret_ref(some_str)
```

### Field access

`LoftStore` provides direct read/write methods for store memory:
- `get_int()` / `set_int()` — `i32` fields
- `get_long()` / `set_long()` — `i64` fields
- `get_float()` / `set_float()` — `f64` fields
- `get_byte()` / `set_byte()` — `u8` fields (boolean, simple enum)
- `get_text()` — read text field as `(*const u8, usize)`
- `get_ref()` — read sub-reference field as `LoftRef`

All take `(rec, pos, offset)` and compute byte address as `rec * 8 + pos + offset`.

### Null sentinels

```rust
pub const NULL_INT: i32 = i32::MIN;
pub const NULL_LONG: i64 = i64::MIN;
```

---

## Store allocation from native code

Native extensions can allocate records and build vectors directly in the store
via `LoftStore` methods. Each mutating operation automatically reloads the
store pointer, since allocation may trigger reallocation.

### Low-level allocation

```rust
// Allocate raw words (auto-reloads ptr)
let rec = unsafe { store.claim(words) };

// Resize a record (may relocate; auto-reloads ptr)
let new_rec = unsafe { store.resize(rec, new_words) };

// Manually refresh ptr/size
unsafe { store.reload() };
```

### Record allocation

```rust
// Allocate a struct record (store_nr derived from the LoftStore handle)
let r = unsafe { store.alloc_record(words) };
// r.rec = record number, r.pos = 8 (data start)
```

### Vector operations

```rust
// Create an empty vector with pre-allocated capacity
let mut v = unsafe { store.alloc_vector(elem_size, capacity) };

// Append elements (handles resize automatically)
unsafe { store.vector_push_int(&mut v, 42) };
unsafe { store.vector_push_long(&mut v, 123i64) };
unsafe { store.vector_push_float(&mut v, 3.14) };

// Read current length
let len = unsafe { store.vector_len(&v) };
```

The `vector_push_*` methods update `v.rec` in place if the vector record
moves during resize. The minimum allocation is 11 elements (matching the
interpreter's convention). The `store_nr` is derived automatically from
the `LoftStore` handle.

### Callback architecture

The allocation callbacks bridge native code back into the interpreter:

```
Native extension                    Interpreter (via thread-local)
─────────────────                   ──────────────────────────────
store.claim(words)
  → claim_fn(ctx, words)    ──→    Store::claim(words) → rec
  → reload_fn(ctx, &ptr, &size) →  read store.base_ptr(), capacity
  ← updated ptr, size
  ← rec
```

`LoftStoreCtx` encodes the `store_nr`; the thread-local `CURRENT_STORES`
holds a pointer to the interpreter's `Stores` for the duration of the call.

### Safety guarantees

The callback infrastructure provides two safety mechanisms:

1. **Panic containment**: All three callbacks (`ffi_claim`, `ffi_resize`,
   `ffi_reload`) wrap their bodies in `std::panic::catch_unwind` to prevent
   panics from propagating across the C-ABI boundary. On panic, `claim`
   returns 0, `resize` returns the original record unchanged, and `reload`
   is a no-op.

2. **RAII cleanup**: `dispatch_call` uses a guard struct whose `Drop` impl
   clears `CURRENT_STORES`, ensuring the thread-local is reset even if the
   native function or a callback panics.

---

## Code generation: `loft generate`

The `loft generate` command reads a package's `.loft` declarations and produces
a `native/src/generated.rs` file with correct C-ABI signatures and `todo!()`
bodies. (This is the no-bridge scaffold for the legacy path; libraries using
the `#[loft_native]` macro generate their bridges automatically and do not
need it.)

### Usage

```sh
cd lib/random
loft generate .          # writes native/src/generated.rs
```

### What it generates

For each `#native` declaration:

1. **C-ABI function signature** with proper type marshalling:
   - Scalars pass directly (`i32`, `i64`, `f64`, `f32`, `bool`)
   - `text` becomes `(name_ptr: *const u8, name_len: usize)` with a
     `let name = unsafe { loft_ffi::text(...) }` body line
   - Struct/vector/collection becomes `LoftRef`, with `LoftStore` prepended
   - Simple enums become `u8`

2. **Return type handling:**
   - Scalars return directly
   - `text` returns `LoftStr` with `loft_ffi::ret(result)` pattern
   - Struct/vector returns `LoftRef`

3. **Field offset modules** for struct types referenced as parameters:
   ```rust
   pub mod image_fields {
       pub const NAME: u16 = 0;   // text (record ref)
       pub const WIDTH: u16 = 4;  // integer
       pub const HEIGHT: u16 = 8; // integer
       pub const DATA: u16 = 12;  // vector ref
   }
   ```

4. **`todo!()` bodies** for the developer to fill in.

### Example output

For `fn rand_indices(n: integer) -> vector<integer>; #native "n_rand_indices"`:

```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn n_rand_indices(
    store: loft_ffi::LoftStore,
    n: i32,
) -> loft_ffi::LoftRef /* vector<integer> */ {
    let result: loft_ffi::LoftRef = todo!("implement n_rand_indices(n)");
    result
}
```

---

## Key source files

| File | Role |
|------|------|
| `src/extensions.rs` | cdylib loader, auto-marshalling dispatcher, allocation callbacks |
| `src/native.rs` | Built-in function registry (`FUNCTIONS` table, `init()`) |
| `src/manifest.rs` | `loft.toml` reader and version checker |
| `src/main.rs` | `generate_native_stubs()` for `loft generate` |
| `loft-ffi/src/lib.rs` | `LoftRef`, `LoftStore`, `LoftStr`, allocation helpers |

---

## Package test suite

Tests live in `tests/` alongside `src/` in the package root; each
`.loft` file is a test module.  Zero-parameter functions named
`test_*` (plus `main`) are discovered and run in isolation with a
fresh `State` — same rules as `loft --tests` on the main project.
The runner adds `src/` to the import path so `use graphics;` works,
sets cwd to the package root for fixture paths, and honours
`@EXPECT_FAIL` / `@EXPECT_ERROR` / `@EXPECT_WARNING` / `@IGNORE` /
`@ARGS` annotations.

**Every `@EXPECT_ERROR` substring must match an error**, not just one of them.
While any single matching error satisfied the whole set, a file with three
annotations and one live diagnostic passed — so an expectation could be reworded
out of existence and the run stayed green (loft#929).  `@EXPECT_WARNING` already
held itself to that bar; the two now agree.

### Layout

```
graphics/
├── src/
├── tests/
│   ├── draw.loft
│   ├── math.loft
│   ├── integration.loft
│   └── fixtures/           # PNG / text / binary test data
```

### Running

```bash
loft --tests graphics/tests                # interpreter
loft --tests graphics/tests/draw.loft      # single file
loft --tests graphics/tests/draw.loft::test_clear_canvas
loft --tests --native graphics/tests       # native backend
loft --tests --native-wasm graphics/tests  # WASM
```

Inside a package directory, `loft test` is shorthand for
`loft --tests tests/` with `src/` on the lib path; accepts optional
`<file>` or `<file>::<fn>` targets.  CI: `loft test` exits 0 on
all-pass, 1 on any unexpected failure.

### Manifest config

```toml
[test]
lib      = ["../other-package/src"]  # extra --lib dirs
skip     = ["tests/webgl.loft"]      # files or patterns
timeout  = 10                        # seconds per test (default 30)
fixtures = "tests/fixtures"          # copied by `loft install`
```

### Fixtures

Test data under `tests/fixtures/`.  Text via `file(...).lines()` or
`.content()`; binary via `f#format = LittleEndian; f#read(n) as T`.
Declare `fixtures = "tests/fixtures"` in `[test]` so `loft install`
copies it alongside the source.

**Update-or-compare pattern** for reference files:

```loft
fn update_or_compare(path: text, actual: text) {
  ref = file(path);
  if ref#format == NotExists {
    uc_f = file(path); uc_f += actual; uc_f += "\n";
  } else {
    expected = ref.content();
    assert(actual + "\n" == expected, "output differs from {path}");
  }
}
```

First run writes the reference; later runs compare.  Delete the
fixture to regenerate after an intentional change.

**Binary output** (PNGs, meshes): compare `f#size` bounds + re-load
and inspect structural fields rather than byte-identity, since
compression may vary across libraries.

### Test discovery rules

| Pattern | Discovered? |
|---------|-------------|
| `fn test_foo()` | yes (zero-param, `test_` prefix) |
| `fn main()` | yes (always an entry point) |
| `fn helper(x: integer)` | no — has parameters |
| `fn count() -> iterator<integer>` | no — returns an iterator |
| `fn _internal()` | no — leading `_` marks private helper |

---

## The build phase — `loft build` (@PLN100 Slice 2)

`loft build [target...]` builds a project's declared or default **targets** — a
target is a named (shape × triple × feature-set) with the toolchain it `requires`.
Built-in targets are **implicit**, so a zero-config project builds with no
`[build]` section:

| Target | Shape → action | Triple | Requires |
|---|---|---|---|
| `native` | `--native --check` (compile-check, no run) | host | rustc/cargo |
| `html` | `--html` (browser page) | `wasm32-unknown-unknown` | rustup `wasm32-unknown-unknown`; `wasm-opt` (soft) |
| `wasi` | `--native-wasm` (WASI `.wasm`) | `wasm32-wasip2` | rustup `wasm32-wasip2` |

```bash
loft build              # build [build] default-targets (or `native` if unset)
loft build html wasi    # build the named targets
loft build path.loft    # a .loft positional overrides the entry
```

The runtime wasm rlib each shape links is auto-built + isolated by Slice 1
(`target/loft/<shape>/`), so `loft build html` needs no prior `make`.

A `[build]` section **overrides** built-ins or **adds** targets. The
line-scanner manifest reader (`src/manifest.rs`) takes single-line arrays and a
`[build.target.<name>.requires]` **subtable** (not an inline table):

```toml
[build]
default-targets = ["native", "html"]   # what `loft build` makes with no args

[build.target.html]                      # overlay: keep the built-in shape/triple,
features = ["random", "png"]             # replace the features it builds with

[build.target.html.requires]
rust-targets = ["wasm32-unknown-unknown"]
tools = ["wasm-opt"]

[build.target.mobile]                    # a NEW named target
shape = "html"                           # required for a non-built-in name
triple = "wasm32-unknown-unknown"
```

Before compiling each target, `loft build` **doctor-checks** its `requires`: a
missing rustup target is a HARD failure (skip the target, print `rustup target
add …`); a missing tool is a SOFT warning (build proceeds). It exits non-zero if
any target fails. Resolution + requires logic lives in `src/build_phase.rs`
(unit-tested); the driver re-invokes this loft binary with the shape's flags per
target.

### Asset steps — `[[build.asset]]` (@PLN100 Slice 3)

A `[[build.asset]]` is a custom command that turns `inputs` into `outputs`, run
by `loft build` **before** the targets it feeds — but only when **stale**:

```toml
[[build.asset]]                 # built from local files
name    = "atlas"
run     = "scripts/pack_atlas.loft"   # a single .loft script, or any shell command
inputs  = ["art/**/*.png"]      # content-fingerprinted -> rebuild only on change
outputs = ["assets/atlas.bin"]  # a missing output forces a rebuild
targets = ["html"]              # runs only when `html` is being built (omit = always)

[[build.asset]]                 # fed by an EXTERNAL source
name     = "dataset"
run      = "scripts/fetch.loft"
outputs  = ["assets/dataset.pak"]
lifetime = "30d"                # freshness TTL — rebuild when older than this
```

**Staleness** = `output missing` **OR** `no prior build` **OR** `inputs content
changed` **OR** (`lifetime` set **AND** the output is older than it) **OR**
`--force`. A fingerprint of the inputs' content + a wall-clock build time are
stamped to `.loft/build/<name>.stamp`; the input fingerprint controls
*re-run-on-change*, the `lifetime` TTL controls *re-fetch-on-age* for
external-source outputs (no instrumentation of the source). `loft build --force`
(or `--fresh`) rebuilds every asset for a deterministic clean build (CI can pin
it). `lifetime` units: `s` `m` `h` `d` `w` `mo` (=30d) `y`.

`run` executes as a `.loft` script (with this loft binary) when it is a single
`.loft` path, else through the platform shell — trusted-by-declaration (it is the
project's own manifest; @PLN100 open question 3 / @PLN86). Input globs support
`*`, `?`, and `**`.

### Test phase — `[[test]]` + `loft check` (@PLN100 Slice 4)

`loft check` is the **build + test gate**: it builds the default targets, runs the
asset steps, then runs the declared `[[test]]` phase — one exit code for CI. A
`[[test]]` runs a `.loft` script over one or more **execution-backend** `targets`,
gated on the asset outputs it `needs`:

```toml
[[test]]
name    = "smoke"
run     = "tests/smoke.loft"
targets = ["interpret", "native"]  # run the SAME suite through each backend
needs   = ["atlas"]                # skip (and fail the gate) if `atlas` didn't build

[[test]]
name   = "atlas-integrity"
run    = "tests/check_atlas.loft"
inputs = ["assets/atlas.bin"]      # a test OVER a generated data file
```

- **Backends:** `interpret` (→ `loft <run>`) and `native` (→ `loft --native <run>`)
  — mirroring loft's own interpret+native harness. `html` / `wasi` have no headless
  runner yet and are reported as skipped (not silently passed).
- **`needs`** gates a test on named assets: if a needed asset's outputs are missing,
  the test is blocked and the gate fails.
- **Green-run caching (incremental `loft check`):** a passing test is cached by the
  `(run-script content + inputs content + target)` fingerprint in
  `.loft/test/<name>__<target>.stamp`; an unchanged test is skipped next run. The key
  includes the target, so a green `native` run does not vouch for `interpret`.
  `loft check --force` (or `--fresh`) reruns everything.

```bash
loft check            # build default-targets + assets, then run [[test]]
loft check --force    # rebuild + re-run every asset and test
loft check foo.loft   # (a .loft arg) compile-check that file instead — the old --check
```

The declared `[[test]]` phase is the *project-facing* surface; loft's own in-repo
`tests/scripts/*.loft` harness (and the `loft test` package-test runner) are
separate. Logic lives in `src/build_phase.rs` (unit-tested).

> **Minimal-scanner note:** the hand-rolled `loft.toml` reader takes single-line
> arrays and a `[build.target.<name>.requires]` subtable (no inline tables), and
> now strips `# …` comments (full-line and inline).

## Build pipeline

### Consumer's view

```bash
# Install a package (downloads or builds native code)
loft install graphics

# Use it — works on all targets
loft my_program.loft           # interpreter: dlopen libgraphics
loft --native my_program.loft  # native: link graphics_native.rlib
loft --native-wasm out.wasm my_program.loft  # wasm: link wasm variant
```

### The three spellings of `loft install`

| command | what it does |
|---|---|
| `loft install` | resolve every dependency this project's `loft.toml` declares |
| `loft install <pkg>[@<v>]` | install one package from the registry, and record it |
| `loft install .` / `loft install <dir>` | copy THAT package into `~/.loft/lib/<name>` for global use |

Bare install is the npm/cargo reading, and the one `loft api` names when it reports a
dependency unresolved. It used to do the third thing (loft#966), which left a copy in
`~/.loft/lib/<name>` shadowing the registry copy of the same name — loft#667, reached from
a command whose name reads like *install my dependencies*. A path dependency needs no
install: it resolves from the path it names, so bare install reports one only when the
path leads to no package.

The name an install is filed under is the manifest's `[package] name`, not the checkout
directory's — a package whose directory differs landed under a name no `use` can reach.

### What `loft install` does


```
1. Locate package (local path, or future: registry)
2. Read loft.toml
3. If [native] section exists:
   a. Check prebuilt/ for current target
   b. If missing or stale: cargo build native/ for current target
   c. Copy rlib to ~/.loft/lib/<package>/<target>/
4. Copy src/*.loft to ~/.loft/lib/<package>/src/
5. Register in ~/.loft/lib/<package>/loft.toml
```

### What `loft my_program.loft` does (enhanced)

```
1. parse_dir("default/")
2. For each `use <pkg>`:
   a. Find <pkg>/loft.toml in lib search path
   b. Parse src/<entry>.loft
   c. If [native] exists:
      - Interpreter: queue rlib for dlopen after byte_code()
      - Native: add --extern <pkg>_native=<rlib> to rustc
      - WASM: add --extern <pkg>_native=<wasm_rlib> to rustc
3. byte_code() — connects #native symbols to loaded functions
4. execute()
```

---

## Target matrix

| Feature | Interpreter | `--native` | `--native-wasm` | `--html` (browser) |
|---|---|---|---|---|
| Pure loft code | ✓ bytecode | ✓ compiled Rust | ✓ compiled WASM | ✓ compiled WASM |
| `#rust` inline | ✓ fill.rs dispatch | ✓ emitted inline | ✓ emitted inline | ✓ emitted inline |
| `#native` external | ✓ dlopen rlib | ✓ linked rlib | ✓ linked wasm rlib | ✓ wasm.bridge crate (see below) |
| File I/O | ✓ OS calls | ✓ OS calls | ✓ VirtFS bridge | ✗ embedded assets only |
| OpenGL | ✓ glutin/gl | ✓ glutin/gl | ✗ WebGL (different API) | ✓ WebGL2 (via loft-gl-wasm.js) |
| Threading | ✓ rayon | ✓ rayon | ✗ sequential | ✗ sequential |

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
top-level `target/wasm32-unknown-unknown/release/libloft.rlib` that
the standalone-binary link uses as `--extern loft=…`.  Two copies of
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

---

## Implementation phases

| Phase | Scope | Effort | Depends on |
|---|---|---|---|
| **P1** | Connect `#native` to interpreter dispatch | Medium | `extensions.rs` completion |
| **P2** | `loft install` for local packages | Medium | P1 |
| **P3** | Native codegen `--extern` for `#native` packages | Medium | P1 |
| **P4** | WASM codegen with native package wasm rlib | Medium | P3 |
| **P5** | OpenGL package: 2D canvas + PNG | Medium | P1 |
| **P6** | OpenGL package: font rendering | Small | P5 |
| **P7** | OpenGL package: GL window + shader | High | P5 + glutin |
| **P8** | WebGL variant + WASM integration | High | P4 + P7 |

P1 is the foundation — without interpreter dispatch of `#native` symbols,
nothing else works.  P5-P6 can proceed in parallel with P3-P4 since the
2D canvas and font rendering don't need GL.

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
[REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md); for governance and yanking see the
Registry Governance section below.

---


# Registry Governance

Procedures for adding third-party libraries to the central Loft package registry
and for responding when problems are discovered in listed packages.

> **What shipped**: the registry is the signed `index.json` in `loft-lang/registry`, and a
> yank sets the version's `yanked` field ([LIBRARY_AUTHORING.md § 5b](LIBRARY_AUTHORING.md)).
> The procedures below were written for the earlier `registry.txt` design; their roles and
> rules still hold, and where they name `registry.txt`, read `index.json`.

The registry is a plain text file (`registry.txt`) maintained in a GitHub
repository.  It starts as a personal repository (`loft-lang/registry`)
and can migrate to a shared GitHub organisation (`loft-lang/registry` or
similar) when the community grows to the point where one person cannot handle
the review load alone.  Both hosting models are described here.  The file
format is described in [PKG_REGISTRY.md](PKG_REGISTRY.md).  This document governs who
may add entries and what happens when an entry must be restricted or removed.

---

## Contents
- [Principles](#principles)
- [Shared Registry Hosting](#shared-registry-hosting)
- [Registry Format — Extended Fields](#registry-format--extended-fields)
- [Submission Requirements](#submission-requirements)
- [Review Checklist](#review-checklist)
- [Approval Workflow](#approval-workflow)
- [Native Package Track](#native-package-track)
- [Problem Reporting](#problem-reporting)
- [Severity Classification](#severity-classification)
- [Response Procedures](#response-procedures)
- [Yanking and Deprecation](#yanking-and-deprecation)
- [Author Appeals](#author-appeals)
- [Registry Maintainer Responsibilities](#registry-maintainer-responsibilities)

---

## Principles

1. **Source-visible** — every registered package must have a publicly readable
   source repository.  Binary-only packages are not accepted.
2. **Fast to restrict** — yanking a package is a one-line edit to `registry.txt`
   and takes effect immediately for new installs.  Security response must not
   be slowed by process.
3. **Proportionate** — minor bugs do not trigger yanks.  The response matches
   the severity.
4. **Stable URLs** — a registered URL for a specific version must never change.
   If the file moves, a new version entry is added.  Old entries are not edited.
5. **Scalable authority** — the process starts with one person and scales to a
   small team without changing the rules.  Any single Maintainer may approve a
   submission or act on a security report; consensus is not required for routine
   work.  Policy changes require team discussion.  See
   [Shared Registry Hosting](#shared-registry-hosting).

---

## Shared Registry Hosting

### Solo model (starting point)

The registry begins as a personal repository owned by the project author
(`loft-lang/registry`).  One person handles all submissions, yanks, and
deprecations.  The compiled-in `source:` URL in the interpreter points here.

This model works for a small package ecosystem.  When the submission queue
regularly takes more than one person can process within the response windows,
it is time to migrate to the team model.

### Team model — GitHub organisation

Create a GitHub organisation (e.g. `loft-lang`) and transfer the repository
to `loft-lang/registry`.  Update the compiled-in `source:` URL in
`src/registry.rs` and the official registry file header at the same time.
Users who run `loft registry sync` will pick up the new URL on their next sync;
no interpreter release is required.

#### Roles

| Role | Count | Permissions |
|------|-------|-------------|
| **Admin** | 1–2 | Add/remove Maintainers; change branch protection; modify this governance document |
| **Maintainer** | 2–5 | Approve submissions; yank/deprecate entries; merge PRs to `registry.txt` |
| **Reviewer** | optional | Review pull requests and issues; no merge permission |

**Reviewer** is an informal role — anyone with a GitHub account can comment on
submission issues.  The label is used in issue assignment to acknowledge people
who contribute reviews without holding Maintainer rights.

#### How decisions are made

- **Routine submissions** — any single Maintainer may approve after the review
  period.  No consensus or second approval is required.  First available
  Maintainer picks up the issue.
- **P0 yanks** — any single Maintainer may yank immediately without consulting
  others.  They notify the rest of the team via a comment on the yank commit or
  a GitHub team mention as soon as they act.
- **Rejections** — any single Maintainer may reject.  The author may re-open
  the issue and request that a different Maintainer review if they believe the
  rejection was incorrect.
- **Policy changes** (to this document) — a pull request, open for at least
  7 days, visible to all Maintainers.  No objection from any Maintainer within
  that window constitutes approval.  Objections must be resolved before merging.
- **Team membership** — Admin only.  A new member is added when nominated by
  any Maintainer and no existing Maintainer objects within 7 days.

#### Load balancing

Issues are self-assigned: any Maintainer picks up an unassigned submission.
If a submission sits unassigned for 4 days, GitHub's stale-issue bot pings
the team.  Maintainers are encouraged to claim issues they have domain knowledge
in (e.g. graphics Maintainer reviews graphics packages).

A rotating on-call schedule for P0/P1 security reports is optional but
recommended when the team reaches 3 or more members: one Maintainer per week is
designated as the primary responder for that week's urgent reports.

#### Joining the team

A person is eligible when they have:

1. Contributed at least **3 substantive reviews** on submission or problem
   issues in the registry repository (comments that check requirements, test
   the package, or identify concerns — not just "+1").
2. Been nominated by any existing Maintainer in a GitHub issue titled
   `Team nomination: <handle>`.
3. Received no objection from existing Maintainers within 7 days.

An Admin then adds the person to the GitHub team.  No vote is taken; silence
is consent.

#### Leaving the team

- **Voluntary** — open an issue or message an Admin.  Access is removed promptly.
- **Inactive** — a Maintainer with no review activity for **6 months** receives
  a 30-day notice issue.  If no activity follows, their Maintainer access is
  downgraded to Reviewer by an Admin.  They can rejoin the Maintainer role by
  resuming activity and requesting re-elevation from any Admin.

#### Branch protection settings (recommended)

```
Branch: main
  Require pull request before merging: ON
  Required approvals: 1
  Dismiss stale reviews: ON
  Allow specified actors to push directly: Maintainers (for P0 emergency yanks)
```

Allowing Maintainers to bypass the PR requirement exists solely for P0 yanks
where speed matters more than process.  Every direct push must include a
comment on the registry issue explaining the urgency.

#### Conflict resolution

If two Maintainers disagree on a submission decision:

1. Either may request a second Maintainer review by posting `@loft-lang/maintainers please review`.
2. If a third Maintainer agrees with one side, that side prevails.
3. If the team is evenly split and cannot resolve within 14 days, the submission
   is held and the author is notified.  The team writes up the specific concern
   in the issue so the author can address it directly.

For severity disputes on problem reports, the higher severity always wins
initially: it is safer to over-restrict and loosen later than the reverse.

---

## Registry Format — Extended Fields

The base format (`name version url`) is extended with an optional fourth field
to record governance status:

```
# name  version  url  [status[:detail]]
graphics  0.2.0  https://example.com/graphics-0.2.0.zip
graphics  0.1.0  https://example.com/graphics-0.1.0.zip  yanked:CVE-2026-001
opengl    0.1.0  https://example.com/opengl-0.1.0.zip    deprecated:use-graphics
math      1.0.0  https://example.com/math-1.0.0.zip      yanked:malicious
```

### Status values

| Status | Meaning |
|--------|---------|
| *(absent)* | Active — installable without warning |
| `deprecated:<reason>` | Installable but warns; excluded from "latest" selection |
| `yanked:<reason>` | Not installable; excluded from "latest"; existing installs unaffected |

The `reason` field is a short slug used in diagnostics.  It may reference a
CVE identifier, a GitHub issue number, or a brief human-readable label.

### Installer behaviour

| User action | Active | Deprecated | Yanked |
|-------------|--------|------------|--------|
| `install name` (latest) | installs | skipped — next active version is used | skipped |
| `install name@version` (exact) | installs | installs + warning | fails with reason |
| Existing install | works | works | works (no change to local files) |

When a deprecated version is the only available version:

```
warning: graphics 0.1.0 is deprecated (use-graphics).
  No other version is available.  Installing deprecated version.
```

---

## Submission Requirements

A library is eligible for submission if all of the following are true:

### Required for all packages

- **Public source repository** — hosted on GitHub, GitLab, Codeberg, or similar.
  The URL must be provided in the submission issue.
- **Open-source licence** — any OSI-approved licence is accepted.  The licence
  must appear in the repository root (`LICENSE`, `LICENSE.md`, or `COPYING`).
- **`loft.toml` with `name` and `version`** — both fields must be present and
  match the proposed registry entry.
- **Reproducible tests** — `loft --tests <pkg>/tests/` must pass cleanly on the
  submitter's platform.  Test output must be included in the submission.
- **Stable download URL** — the `.zip` URL must remain permanently accessible.
  GitHub release assets, tagged archives, or static file hosting are all
  acceptable.  Direct repository archive URLs (e.g. `github.com/.../archive/`)
  are *not* acceptable because their content can change silently.
- **No name collision** — the package name must not duplicate an existing
  registry entry (including deprecated entries).  If the intent is to supersede
  a deprecated package, contact the maintainer before submitting.

### Additional requirements for native packages

Native packages ship compiled shared libraries and execute arbitrary code inside
the interpreter process.  They require extra scrutiny:

- **Rust source only** — native extensions must be written in Rust.  Pre-compiled
  blobs with no corresponding source are rejected.
- **No `unsafe` outside the plugin boundary** — `unsafe` is permitted only in
  the `loft_register_v1` entry point and in direct FFI calls to platform APIs.
  All other Rust code must be safe.
- **Dependency audit** — the submission must list all crate dependencies and
  their versions.  Dependencies with known CVEs at submission time are a
  blocking issue.
- **Explicit capability declaration** — the submission must state clearly what
  system resources the native code accesses (network, filesystem, GPU, audio,
  etc.).  This is informational, not restrictive, but must be accurate.

---

## Review Checklist

The maintainer works through this checklist before approving:

### Pure-loft packages

- [ ] Source repository is public and readable
- [ ] Licence file is present and OSI-approved
- [ ] `loft.toml` fields `name` and `version` match the submission
- [ ] Download URL is stable (not a mutable archive URL)
- [ ] `loft --tests` passes (submitter-provided output reviewed)
- [ ] No name collision with existing registry entries
- [ ] Package description in the issue makes the purpose clear
- [ ] Package does not re-implement a core stdlib function
      (acceptable if it extends or specialises it)

### Native packages (all of the above, plus)

- [ ] Rust source is public and the entry point matches `loft_register_v1`
- [ ] `unsafe` is confined to the registration entry point and FFI calls
- [ ] Cargo.toml dependencies list reviewed; no known-vulnerable versions
- [ ] Capability declaration matches what the code actually does
- [ ] At least one reviewer other than the submitter has read the Rust source
      (the maintainer counts; community review is welcome but not required)

---

## Approval Workflow

### Step 1 — Open a submission issue

The package author opens a GitHub issue in the registry repository
(`loft-lang/registry` or `loft-lang/registry` if the team model is active)
using the **Package Submission** template.  Required fields:

- Package name and version
- Download URL (the exact `.zip` URL)
- Source repository URL
- Licence identifier (e.g. `MIT`, `Apache-2.0`, `LGPL-3.0-or-later`)
- Brief description (1–3 sentences)
- Test output paste or link to a CI run
- For native packages: capability declaration and dependency list

### Step 2 — Community review period

The issue remains open for **7 calendar days** before the maintainer makes a
decision.  Community members may:

- Report concerns (security, name confusion, licence issues)
- Confirm they tested the package successfully
- Suggest improvements to the submission

The 7-day period may be waived by any Maintainer for:
- A patch to an already-approved package (same name, new version)
- A dependency of an already-approved package

In the team model, any available Maintainer self-assigns the issue within
4 days of it being opened.  If no one self-assigns, GitHub's stale bot pings
the team.

### Step 3 — Maintainer decision

After the review period any Maintainer may act:

- **Approves** — adds the entry to `registry.txt` via a pull request, closes
  the issue with a link to the commit.
- **Requests changes** — lists specific blockers in the issue.  The author
  addresses them and re-requests review.  The same or a different Maintainer
  may handle the follow-up.  A new 7-day period does not restart unless the
  Maintainer judges the concerns were substantial.
- **Rejects** — closes the issue with a written reason.  Rejection reasons
  include: name collision, licence incompatibility, fails to build or test,
  native package fails the safety checklist, or the package duplicates
  existing stdlib functionality without adding value.  The author may ask a
  different Maintainer to re-review if they believe the rejection was wrong.

### Step 4 — Ongoing versions

Once a package is approved, the author may add new versions by opening a
**New Version** issue (lighter template: URL + test output only).  The 7-day
period applies unless waived.  The maintainer verifies the `loft.toml` version
field increments monotonically and the URL is stable, then appends the new line.

---

## Native Package Track

Native packages (those with `#native` annotations and compiled shared libraries)
follow the same workflow but with a **14-day** review period and a mandatory
Rust source review.  The checklist item "at least one reviewer other than the
submitter has read the Rust source" must be satisfied before any Maintainer
approves.

**Solo model** — if no community reviewer steps forward in 14 days, the single
maintainer performs the source review alone.  This is acceptable for small
packages but uncomfortable for large or complex ones; such packages may be held.

**Team model** — the approving Maintainer must not be the sole reviewer of the
Rust source.  A second Maintainer or a community Reviewer must have commented
confirming they read the native code.  This cross-review requirement is the
primary reason native packages exist as a separate track: with a team, it is
always satisfiable without holding packages indefinitely.

---

## Problem Reporting

Anyone — user, security researcher, or package author — may report a problem by
opening a GitHub issue in the registry repository with the **Problem Report**
label.

Required information:

- Package name and affected versions
- Description of the problem
- Reproduction steps or proof of concept (for security issues: report privately
  first — see below)
- Suggested severity (the maintainer makes the final call)

### Security vulnerabilities — private disclosure

For security issues (malicious code, data exfiltration, privilege escalation,
or any issue where publishing reproduction steps could cause immediate harm),
report privately:

- Use GitHub's **private security advisory** feature on the registry repository
  (works for both the solo and team models — all Maintainers see it).
- Email any individual Maintainer whose address is on their GitHub profile if
  the advisory feature is not available.

Any single Maintainer who receives a credible private report will yank the
affected versions within **24 hours**, before any public disclosure, and notify
the rest of the team immediately after acting.  In the team model, the on-call
Maintainer (if a rotation is in place) is the primary recipient.

---

## Severity Classification

| Severity | Examples | Target response |
|----------|----------|-----------------|
| **P0 — Critical** | Malicious code, data exfiltration, remote code execution, supply-chain attack | Yank within 24 h; no discussion required |
| **P1 — High** | Data loss, crash in common use path, security issue without active exploit | Deprecate within 48 h; yank if no fix in 14 days |
| **P2 — Medium** | Incorrect output, API incompatibility with a published version, failed tests | Notify author; deprecate if no fix in 30 days |
| **P3 — Low** | Documentation error, minor edge-case bug, cosmetic issue | Notify author; no forced action |

Severity is assigned by the maintainer after reviewing the report.  The reporter's
suggested severity is taken as input, not as binding.

---

## Response Procedures

### P0 — Critical

1. **Any single Maintainer** yanks all affected versions immediately — a
   direct push to `registry.txt` is allowed under branch protection for exactly
   this case.  No approval from other Maintainers is needed; speed is paramount.
2. The acting Maintainer posts a team notification (GitHub team mention or email)
   within 1 hour of the yank explaining what was done and why.
3. A public issue is opened describing the problem at a high level (no exploit
   details if not yet public).
4. If the author is reachable and acting in good faith, they are given
   opportunity to release a fixed version before the public issue is opened.
   This window is at most **24 hours**.
5. If the package was malicious or the author is unresponsive, the package is
   permanently removed from the registry (all versions yanked with
   `yanked:malicious` or `yanked:removed`).
6. The public issue references the yank commit and summarises the nature of the
   problem.

### P1 — High

1. Maintainer marks affected versions `deprecated:<issue-number>` within 48 h.
2. Maintainer notifies the package author via the GitHub issue and, if possible,
   via the source repository's issue tracker.
3. Author has **14 days** to release a patched version.
4. If a fix is released and passes the review checklist, the patch version is
   added to the registry and the deprecation reason updated to point to it.
5. If no fix appears in 14 days, the affected versions are yanked.

### P2 — Medium

1. A GitHub issue is opened in the registry repository referencing the problem.
2. The package author is tagged and has **30 days** to respond.
3. If a fix is released within 30 days, the new version is added normally and
   the issue is closed.
4. If no response or fix within 30 days, the affected versions are deprecated.
5. If 60 days pass with no fix, the affected versions are yanked.

### P3 — Low

1. The issue is opened and the author is notified.
2. No forced action.  The issue remains open until the author fixes it or
   closes it as "won't fix".
3. The maintainer may add a deprecation comment in the issue if the bug causes
   significant confusion, but registry entries are not changed.

---

## Yanking and Deprecation

### What yanking does

- The status field for the entry in `registry.txt` changes to `yanked:<reason>`.
- `loft install name` (latest) skips yanked entries.
- `loft install name@version` for a yanked version fails with the reason:
  ```
  error: graphics 0.1.0 has been yanked (CVE-2026-001).
    Install a different version or check the project repository for a fix.
  ```
- Existing local installations are **not removed**.  Yanking affects new installs only.
- A yanked entry is never removed from `registry.txt` entirely — the line
  remains so that users who already have that version can understand why it is
  flagged.

### What deprecation does

- The status field changes to `deprecated:<reason>`.
- `loft install name` (latest) skips deprecated entries and selects the next
  active version.  If no active version exists, the deprecated one is installed
  with a warning.
- `loft install name@version` installs the deprecated version with a warning:
  ```
  warning: graphics 0.1.0 is deprecated (outdated).
    Consider upgrading to graphics 0.2.0.
  ```
- Existing installations are unaffected.

### Permanent removal

In cases of confirmed malicious packages, the entry status is set to
`yanked:removed` and a note is added to the registry changelog.  The URL field
is replaced with a placeholder (`-`) so no download is possible even if a user
edits the status field manually.

---

## Author Appeals

If a package author believes a yank or deprecation was applied incorrectly:

1. Open a GitHub issue in the registry repository titled
   `Appeal: <package> <version>`.
2. Explain why the action was incorrect and provide evidence (fixed code,
   misattributed CVE, etc.).
3. **Solo model** — the maintainer reviews within **7 days**, taking the
   reporter's argument at face value since there is no second opinion available.
4. **Team model** — the appeal is reviewed by a Maintainer who was *not*
   involved in the original decision.  This separation is one of the concrete
   benefits of the team model: appeals are not judged by the person being
   challenged.  Resolution within **7 days**.
5. If the appeal is upheld, the status is removed or changed and a new version
   is added if appropriate.
6. P0 yanks (malicious code) are not subject to appeal.

---

## User-Side Verification

Users can check their installed packages against the latest registry at any time
using two commands (see [PKG_REGISTRY.md](PKG_REGISTRY.md)):

```sh
loft registry sync     # pull latest registry.txt from GitHub
loft registry check    # compare installed packages against registry
```

`loft registry check` exits with code 1 if any installed package is yanked,
making it usable as a CI gate:

```sh
# In a CI pipeline — fails if any yanked package is installed
loft registry sync && loft registry check
```

Typical output when a yank is relevant to the user:

```
  utils  0.3.0  YANKED  CVE-2026-001 — run: loft install utils
```

The staleness warning (registry older than 7 days) reminds users to sync
regularly without being an error.

### How yanks reach users

1. Maintainer edits `registry.txt` — adds `yanked:<reason>` to the affected line.
2. The change is committed and pushed to `loft-lang/registry` on GitHub.
3. Any user who runs `loft registry sync` gets the updated file immediately.
4. `loft registry check` then surfaces the yank in the terminal and in CI.

No action is required from package authors or the loft interpreter itself to
propagate the yank — the registry file is the single source of truth.

---

## Registry Maintainer Responsibilities

These apply to every Maintainer regardless of model.

### Response times (shared commitment)

| Action | Target |
|--------|--------|
| Self-assign an open submission | 4 days |
| Complete submission review after review period | 14 days |
| P0 yank after credible private report | 24 hours |
| P1 deprecation decision | 48 hours |
| Appeal review | 7 days |

Response times are per-team, not per-individual — if the assigned Maintainer
cannot meet a deadline, any other Maintainer may step in.  In the solo model
these are personal commitments; in the team model they are collective ones.

### Record-keeping (all Maintainers)

- `registry.txt` is kept in a git repository with a public commit history.
  Every addition, yank, and deprecation is a traceable commit with the acting
  Maintainer's identity visible in `git log`.
- `REGISTRY_CHANGELOG.md` in the same repository summarises all yanks and
  deprecations in human-readable form, updated with every status change.
- Entries are never removed from `registry.txt` — only the `status` field is
  added.  The file is a permanent auditable record.

### Additional responsibilities in the team model

- **On-call rotation** — when the team has 3 or more Maintainers, maintain a
  weekly on-call schedule for P0/P1 responses.  The schedule is published in
  the repository's `MAINTAINERS.md`.
- **Monthly async review** — post a brief summary to the repository's GitHub
  Discussions each month: open submissions, recent yanks/deprecations, team
  membership changes.  This keeps all Maintainers informed even if they were
  not the ones acting.
- **MAINTAINERS.md** — keep a `MAINTAINERS.md` file in the registry repository
  listing current Maintainers, their GitHub handles, and (if applicable) which
  week they are on call.  Update it when membership changes.

### Stepping down as primary owner (solo → team migration)

When the solo maintainer decides to migrate to the team model:

1. Create the GitHub organisation and transfer the repository.
2. Invite 2–4 people who have already been reviewing submissions as community
   members; they become the first Maintainers.
3. Update the `source:` URL in the registry file and the compiled-in default
   in `src/registry.rs` in a coordinated interpreter patch release.
4. Publish a `REGISTRY_CHANGELOG.md` entry and a GitHub release note explaining
   the transition.

The original owner retains an Admin role in the organisation indefinitely,
but may reduce their Maintainer workload to match the team capacity.

---

## See also

- [PKG_REGISTRY.md](PKG_REGISTRY.md) — file format, install flow, version resolution, implementation
- [Library Package Format](#library-package-format) — package format + native extension design
- [lib_plans/12-library-extraction/](lib_plans/12-library-extraction) — extraction/migration of `lib/*/` into external repos

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
