<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# Package targets — history

The record behind [PACKAGES_TARGETS.md](PACKAGES_TARGETS.md).

## Retired with the rustc writer (@PLN225)

A library's browser half was a Rust bridge crate the rustc writer linked into the page; the
writer is retired (`@C170` in [DESIGN_DECISIONS_PLATFORM.md](DESIGN_DECISIONS_PLATFORM.md)),
and a library's browser half is now its C shim's `LOFT_BROWSER` arm plus its `host.js`.
The sections below are kept as they stood.

### The Rust bridge crate (`[wasm.bridge] crate` + `routes`)

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

#### Why rustc-direct (not `cargo build`)

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

### The canonical example then

`lib/imaging` was the first library to use the Rust-bridge pattern end-to-end:
`lib/imaging/wasm/src/lib.rs` (`imaging_load_png` / `imaging_save_png` bridges; field offsets
in the `Image` struct; vector allocation via `loft::vector::alloc_vector_from_bytes`),
`lib/imaging/wasm/host.js` (`imaging_query` / `imaging_copy_rgb` / `imaging_save`), and
`lib/imaging/loft.toml::[wasm.bridge]`.  The @P321(c) browser-WASM dimension in
[PROBLEMS.md](PROBLEMS.md) is what surfaced the need.

### `[native.wasm]` overrides

On `--native-wasm`, `loft.toml [native.wasm]` overrode a native symbol per target
(`gl_create_window` → `webgl::create_canvas`, `gl_swap_buffers` → `webgl::flush_canvas`,
`gl_create_shader` → `webgl::create_shader`).  The manifest parse now refuses
`[native.wasm]` with the other Rust-native declarations.

### Security: the `--no-native` flag

The security model listed a `--no-native` flag (refuse to load any `#native` function); no
such flag exists in `src/main.rs`.
