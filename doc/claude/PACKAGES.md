
# Library Package Format

Design for a unified packaging format that supports pure-loft libraries,
libraries with a native part in C, and WASM targets — with OpenGL as the driving
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
- [Key source files](PACKAGES_NATIVE_API.md#key-source-files)
- [Package test suite](PACKAGES_BUILD.md#package-test-suite)
- [Build pipeline](PACKAGES_BUILD.md#build-pipeline)
- [Target matrix](PACKAGES_BUILD.md#target-matrix)
- [OpenGL case study](PACKAGES_TARGETS.md#opengl-case-study)
- [Security model](PACKAGES_TARGETS.md#security-model)
- [Open work](#open-work)

---

## Goals

1. A single package can contain loft source, a native part in C, and
   pre-compiled WASM — consumers don't choose; the runtime picks the
   right variant for the target.
2. `use graphics;` in a loft program works identically whether running
   via the interpreter, `--native`, or `--native-wasm`.
3. OpenGL/WebGL bindings ship as a package, not as built-in stdlib.
4. Package authors write the native part once, in C; loft builds it for every target
   from the same source.
5. A package's native part is C: the sources listed under `[c] shim`, bound by `#c`
   declarations ([PACKAGES_C_BINDING.md](PACKAGES_C_BINDING.md)).  A package's own
   native library in Rust (`[library] native`, `[native] crate`, `[native.functions]`,
   `[native.wasm]`) is refused where the manifest is read, naming that cure
   (`manifest::RUST_NATIVE_REFUSED`, @PLN225; guard `tests/native_loader.rs`).

---

## Current state

| Layer | Status |
|---|---|
| Pure-loft packages (`use lib;`) | **Shipped** — directory layout, version check, `lib/` search |
| `loft.toml` manifest | **Shipped** — entry, version, native stem fields |
| `#native "symbol"` annotation | **Parsed** — bytecode dispatch NOT connected |
| `extensions.rs` cdylib loader | **Designed** — feature-gated, not integrated |
| WASM virtual filesystem | **JS side done** — Rust stubs return early |
| Native codegen (`--native`) | **Working** — writes C, compiles with the host's C compiler |
| WASM codegen (`--native-wasm`) | **Working** — the same C, built by clang for WASI |

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
└── c/
    └── graphics.c            # the native part: C99 loft builds with `cc`
```

**Rules:**
- `src/` is mandatory — every package has at least one `.loft` file
- `c/` is optional — only if the package has a native part; its sources are listed under
  `[c] shim` and loft builds them (`loft new --native <name>` scaffolds one)
- `[native] runtime-libs` (@PLN21 Phase 2) declares the system shared libs a component's
  library needs (validation + install hint).
- The primary `.loft` file (`src/graphics.loft`) declares the public API
  including `#native` function signatures

---

## Releasing — libraries and applications alike

`loft package` is the release step of every project with a `[package]` version, a library or
an application.  Before it builds the tarball it records `layout.lock` beside `loft.toml`: the
layout of the types whose values outlive the program (those its store and `swap_world` calls
hand over, every `pub` type it declares, and what they hold), the release, and which stored fields each
computed field's default reads (DATABASE.md § A store that skipped releases).  Commit it with
the release: from then on every compile of the project warns (`layout-upgrade-refused`) at a
stored type a store of that release could not be upgraded to, and `loft package` refuses to
release while one stands, keeping the baseline it was measured against.  The registry steps that follow
(`loft publish`) are a library's only; `--tarball-only` builds the archive and records nothing.

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
data_compatible_with = "2.0"       # the oldest release whose STORES this one reads;
data_bridge = "2.3"                # the last release that read older ones — named
                                   # when an older store is refused (DATABASE.md §
                                   # A store that skipped releases).

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

[triggers]
enabled = true              # auto-use: a consumer that calls one of this
                            # package's METHODS (`line.matches(p)`) or names one
                            # of its TYPES (`c: Command = …`) loads it with no
                            # `use`.  Both halves are DERIVED from the `pub`
                            # declarations at publish time; nothing is listed.
                            # A consumer derives them from the copy a `use`
                            # would load (formal/dispatch.md `Disp-Trigger`):
                            # each DECLARED dependency's sibling, `path =`,
                            # `lib/<dep>` or `--lib` copy, else its extracted
                            # registry copy (the lock's pin); for a package
                            # nothing declares, a `--lib` directory's copy, then
                            # the catalogue (methods and builders only: the
                            # index carries no types).  So `--lib <worktree>`
                            # fires a method the published copy lacks.  A name
                            # two `--lib` packages derive fires neither.
builders = ["set"]          # the one declared half: the free `pub fn`s a consumer
                            # may call BARE (`set(words)`) and have this package
                            # loaded.  A builder takes a name in the program's own
                            # namespace, so the author lists the few worth it; the
                            # program's own `fn set` wins, and `<pkg>::set(…)`
                            # always reaches the library's.  A listed name the
                            # entry file does not declare as a free `pub fn` is
                            # dropped, with a warning at `loft publish`.

[c]
# The package's native part: C99 sources loft builds with `cc` (and `clang` for the wasm
# targets), bound by the `#c` declarations in src/ (PACKAGES_C_BINDING.md).
shim = "c/graphics.c"

[native]
# OR the natives are a COMPONENT loft ships beside its binary: a prebuilt C
# library behind loft-ffi's C ABI (`loft-ffi/include/loft_ffi.h`), found in
# <prefix>/lib/loft/components/ (`src/components.rs` has the search order;
# `make components` builds them in a loft tree).  Nothing is compiled when it
# is used, and a missing component is refused by name on every backend.
component = "net"           # libloft_net: the web and server libraries' HTTP,
                            # TLS and WebSockets, client and server
# A native library of the package's own — `[library] native`, `[native] crate`,
# `[native.functions]`, `[native.wasm]` — is refused: write the native part in C.

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

**A library's range stays in the API line it was built against** (@C166).  A range a
*published* library declares — `graphics = ">=0.9.0"` in a package from the registry —
resolves to the newest release that is still a drop-in for its floor, never one whose
`api_compatible_with` declares a break past it: that library compiled against graphics
0.9, so graphics 0.12 is not what it asked for.  Your own project's ranges are your choice
and resolve to the newest as before, and declaring the package yourself chooses for the
whole tree (below).  Where no release is left in the line, the range resolves to the newest,
so the rule never refuses a range that resolved.

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

When the compiler encounters `use math;` it takes the first copy it finds, by one rule
(`@C173`): **what this run says, then what the project declares, then what merely lies in a
directory.**

1. **The package's own name** — inside a package, its own `[package] name` binds its own
   entry, whatever its directory is called; a package is identified by its manifest, not its
   path (`tests/own_package_resolution.rs`).
2. **`--lib` on this command line** — the explicit override: a library can always be run from
   your own copy to debug or fix it.  It outranks everything below, the pin included, and
   `advice[lib-flag-override]` says which copy runs when it replaces a pinned or declared one.
   The one exception is a module beside the importing file (`helper.loft` next to the file
   that says `use helper`): that is the file's own source, and resolves as before.
3. **What the project declares** — a `[dependencies]` `path = "..."` entry (the importing
   package's, then the **root project's**, so a path the root declares answers a `use`
   anywhere in the program, however deep — `tests/transitive_binding.rs`), then a module
   beside the importing file, then the version the lock pins (`loft.lock`, or a script's
   `<script>.loft.lock` from `loft pin`).
4. **Everything else, for a name nothing pins** — a `lib/` in the working directory or beside
   the script, a flat `<name>.loft` there, sibling packages, `~/.loft/lib/`, the `LOFT_LIB`
   directories, the registry.  A **pinned** name skips these: a same-named file that merely
   lies in a directory never replaces the version the lock records.

The rule is `tests/lib_flag_override.rs`.  To work on a pinned library for longer than one
run, `loft develop` (below) declares your copy as a path dependency.

A package loads once, so the copy chosen is the program's; every package that `use`s it
must accept it.  The importing package's own range is checked against that copy wherever it
was chosen — at its first `use` or a later one — and a copy outside it is refused, naming
both declarations, rather than loaded anyway.  If the dependency has its own `loft.toml`, its
version is checked against the requirement; which installed version a lock file selects is
[LIBRARY_AUTHORING.md § Troubleshooting](LIBRARY_AUTHORING.md).

⚠ **A `lib/` directory's package form needs a project.** The directory form `lib/math/` is read
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

**The `loft` floor names a release or a DAY.**  A release answers its own version
(`2026.10.0`).  Every other build — the daily build, a development build of `main` —
answers `<last release's YYYY.M>.<yyyymmdd of its commit>`, e.g. `2026.10.20261005` <!-- doc-lint: ok -->
(`loft --version` prints it).  That orders after the release it builds on and any point
release of it, and before the next monthly release, so a library that needs what landed on
`main` floors on the day — `loft = ">=2026.10.20261005"` — and publishes now: a loft too old <!-- doc-lint: ok -->
for it skips that version when resolving (`registry_index.rs`), the daily build and the next
release take it.  `LOFT_VERSION=<v>` at build time stamps a build explicitly; release builds
take theirs from the tag.  Guard: `manifest::tests::a_dated_daily_version_orders_between_its_release_and_the_next`.

### Cycle detection

Circular dependencies are rejected:

```
Error: circular dependency: graphics → math → graphics
```

The resolver tracks the dependency chain and panics on cycles before
any source is parsed.

### Native dependency propagation

Each package's C part is built on its own.  A shim that includes a header of another
package names that package under `[dependencies]` and includes it as
`#include "<dep>/<header>.h"` ([PACKAGES_C_BINDING.md § Declaring the library](PACKAGES_C_BINDING.md)).

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
  resolves through it too, so the consumer's lock pins a transitive package.  A `use` that
  fetches a pinned package keeps those pins: what it pulls in installs at the locked version
  while that version satisfies every declaration (`install::kept_lock_pin`).
- **`loft install` binds a transitive package to what the project declares for it**, on top
  of what the package pulling it requires, so the lock records the version the program
  loads whatever order `[dependencies]` lists.  A declaration no dependency can accept is
  refused at install, naming both sides.

### Developing a library you use (`loft develop`)

A pinned library loads only from its pin, so working on one is an explicit step, and
`loft develop` is that step:

```sh
loft develop crypto                 # a copy of the version in use, in lib/crypto
loft develop crypto --path ../loft-libs-core/crypto   # or your own checkout of it
loft develop                        # what this project develops
loft develop --done crypto          # back to what the manifest said before
```

It rewrites the library's `[dependencies]` line to a path dependency that keeps its version
and records what the line was:

```toml
crypto = { path = "lib/crypto", version = "0.5.0" }  # loft develop: was "0.5.0"
```

A path dependency outranks the pin, so the copy runs, and its own `use`s of other libraries
resolve as before.  `--done` restores the line exactly (removing it when the library was only
used through another one) and leaves the copy where it is.  For one run, `--lib <dir>` does the
same without touching the project.

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

// Native: implemented in C (c/graphics.c), bound with #c
pub fn save_png(self: const Canvas, path: text) -> boolean;
#c "gr_save_png" "int(const void*, int64_t, int64_t, const char*, int64_t)"
```

### Implementation in C

The native part is C: the sources under `[c] shim`, each function bound by a `#c`
declaration that names its C symbol and C signature.  The binding rules, the type mapping
and the targets are in [PACKAGES_C_BINDING.md](PACKAGES_C_BINDING.md).  A `#native`
declaration is served by a component loft ships (`[native] component`) or by loft itself;
a package's own Rust implementation of one is refused.

**Mark a native `#pure` when its value is its only effect** — the answer depends on the
arguments alone, it writes nothing, reads no clock, file or entropy, always returns and reports
nothing: a hash, an encoder or decoder, a verifier.  The annotation goes on its own line after
`#native`.  loft then computes two calls on unchanged arguments once, also when two small
wrappers each make the call (a validate-then-use pair decodes once; `formal/rewrites.md`
§ `(R-PureReuse)`, the shared-call clause).  A wrong `#pure` is a wrong program: a call loft
merges or moves ahead of a test must not have been observable.  Leave it off a random source,
a clock, I/O, and anything that can fail loudly.

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

### An auto-native artifact is stale when any source it CONTAINS is newer

An auto-native cdylib is not just its own package: `emit_program` emits the
export set **and its transitive dependencies**, so `hex_editor`'s cdylib holds a
full copy of `hex_part`'s functions — and exports them under the same
`loft_shared_<name>` symbols `hex_part`'s own cdylib exports.  Whichever library
is dlopened first wins the lookup.

So the freshness question has to span every package that contributed code, not
just the one that owns the artifact (`source_newer_than` /
`source_content_hash`, `src/opt/native_key.rs`, take a *set* of package dirs — the
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
  edited library still fails startup, and the *generated* source beside the stale
  `.so` shows the NEW code — so every "is it being re-read?" check passes.  The
  discriminator is `LOFT_NO_NATIVE_LIBS=1`: same run, same program, and the
  interpreter answers differently from the cdylib.

The wider question costs one `stat` walk over the loaded packages (under a
millisecond for a ten-package tree, against a compile).  It does mean
a dependency edit makes every dependent stale — which is the honest answer, since
the dependent really does contain the edited code — and `dev-interpret-on-edit`
still keeps the compiler out of the loop until editing settles.

**An auto-built library links its OWN copy of loft's runtime, so every `static` in it exists
once per linkage unit** — the main binary, or `native-auto/libloft_auto_<pkg>.so` — so a
process-global table filled by the program is EMPTY from inside the package: `c_library_available` answered
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

The package **format, registry, and signing work today**: `loft.toml`
manifests, `loft package`/`install`/`search`/`info`, `loft.lock`, Ed25519
index signing, and a bootstrapped 3-key trust root —
**13 libraries are live in [loft-lang/registry](https://github.com/loft-lang/registry)**
and `loft install <name>` resolves + verifies + extracts them.  What remains is
a release to activate the trust root,
and the library-extraction arc.  The finished rows are in
[PACKAGES-history.md](PACKAGES-history.md).

| Item | Status |
|---|---|
| **PKG.PREBUILT** (@PLN21) — native prebuilts, no rustc to *use* a lib | **Superseded:** a library's compiled form is its hosted C unit, which each loft release builds for every published library (`scripts/release-lib-artifacts.sh`) and its registry row lists (@PLN206 P6).  A package's own prebuilt native library is not fetched or loaded (@PLN225); a version's `binaries` names only the loft toolchain's own builds (self-update). |
| **PKG.EXTRACT** — move `lib/*/` to per-family GitHub repos | **In progress.** Libraries already live in `loft-lang/loft-libs-*` + published; the prerequisite arc (drain library `#native` code out of the compiler crate) is active — [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction). |
| **PKG.STUB** — generated API stubs + `loft api` | **SHIPPED** (stubs on install/update/pin, `loft api [name]`, `tests/api_discovery.rs`).  Remaining: parser-walk upgrade shared with [API_SURFACE.md](API_SURFACE.md) `api-lint`. |
| **PKG.FOREIGN-HOST** — a host hands a frame over as a FOREIGN store (@PLN174's tail) | **Open, a design pick.** cbor reads a frame's payloads as spans (−11 % on `check_request` over a foreign frame, `alloc-temp.md` § F6).  What is left is WHICH host produces pluginabi's frame as a foreign store — the engine's plugin channel or the browser through a typed array (`BROWSER_INTEROP.md`, the plan's F7) — and its cells.  The Rust library bridge (`LoftStore::foreign_vector_from_owned`) and its fixture went with a package's own Rust native library (@PLN225). |
| **PKG.CNAME** — a `[c]` library named ONCE, by identity | **Design only, not built** — [plans/24-c-abi-binding/LIBRARY_NAMING.md](plans/24-c-abi-binding/LIBRARY_NAMING.md).  Today a manifest names a library by its Linux ELF filename and every consumer recovers the identity by string surgery; four measured failures came out of that, each currently carrying its own local workaround (`-l:<file>` for the link stem, `host_lib_variants` for the probe, "at most one optional library per package" for symbol attribution).  **Trigger: the fifth one** — a new platform, or any consumer that has to re-derive a spelling from a filename. |

**Remaining, in order:**
1. **Cut a loft release** — activates the embedded trust root (PKG.SIGN); until then deployed loft has an empty trust root and ignores signatures.
2. **PKG.EXTRACT** — continue draining the compiler crate + per-library moves via [`lib_plans/12-library-extraction/`](lib_plans/12-library-extraction).

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

The package format, the native binding model, discovery and
loading, the `loft-ffi` helper crate, store allocation from native code,
and the per-target build pipeline are all documented in
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
