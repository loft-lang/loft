<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# WASM — history

The record behind [WASM.md](WASM.md): sections that described the wasm builds the rustc
writer made, retired with it (@PLN225, `@C170` in
[DESIGN_DECISIONS_PLATFORM.md](DESIGN_DECISIONS_PLATFORM.md)).  `--html` and `--native-wasm`
now build the program's C with clang for WASI; a library's native part is its C `[c] shim`.

## Retired with the rustc writer (@PLN225)

Each section below is kept as it stood when the writer built the wasm targets.

### A `[native] crate` package on `--native-wasm`

`--native-wasm` is the one wasm target that runs a package's own Rust: loft
cross-builds the `[native] crate` to `wasm32-wasip2` on demand and links its rlib
into the program, so `#native` functions execute for real (`--html` cannot — it
produces a standalone module with no crate to link, and routes through the
package's `[wasm.bridge]` instead).

Two shapes, and they have different requirements. A **scalar** `#native`
(`fn answer() -> integer`) lowers to a plain `extern "C"` call and needs nothing
from loft's runtime. One that **allocates into a loft store** — a `vector` return,
a struct `Reference` argument — is wrapped in `loft::native_call::enter` +
`build_store`, so the crate can claim records in the caller's store.

Those two helpers used to sit behind the `native-extensions` cargo feature, and
the wasm runtime rlib is built `--no-default-features --features random`
(`WasmRuntimeShape::features`) because that feature buys `dlopen`, which wasm does
not have. The result was that every store-touching native failed in `rustc`, inside
generated code, with `E0433: cannot find native_call in loft` — while the scalar
shape stayed green and reported the target as covered (loft#967). `native_call`
opens nothing, so it is no longer gated.

**The rule this leaves:** a `loft::<module>` path the generator writes into
generated code must exist in the lean feature set, or the construct that emits it
must be REFUSED before emission. `#c` bindings take the second route — `no_c_abi()`
rejects a reachable one with a diagnostic naming the package (@PLN24 arc E), which
is why `loft::c_call` may stay gated. `generated_loft_paths_survive_the_wasm_feature_set`
(`tests/html_wasm.rs`) reads both gate sites — the declaration in `src/lib.rs` and a
module file's own inner `#![cfg(…)]` — and is always-on, because the end-to-end
wasip2 test self-skips wherever wasmtime is not installed.

#### When the crate needs a device the target does not have

A `[native] crate` is cross-built as a whole, so ONE dependency that has no
wasm32 target takes the entire package off `--native-wasm` — including the parts
that never wanted a device.  `graphics` shipped that way for months: `winit` and
`glutin` were unconditional, cargo failed before reaching any of the package's
own code, and its software canvas, mesh maths and PNG encoder went down with the
window layer they have nothing to do with.

**The shape that works** (`loft-libs-graphics/graphics/native`):

1. Put only the crates that genuinely cannot target wasm32 under
   `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`, and cross-build
   each remaining one on its own to know which those are — the list is usually
   shorter than it looks.  For graphics it was two of nine: `gl` compiles
   anywhere (it is a table of function pointers), and so do `fontdue`, `png`,
   `image` and `rodio`.
2. Give a wasm32 twin only to the entry points that touch the absent device
   DIRECTLY.  Everything reached through a readiness flag needs none: graphics
   sets `GL_READY` inside window creation only, so on a target with no window
   the flag never opens and ~90 GL functions answer their documented defaults
   through the guard they already had.  Five needed a twin.
3. Make each twin answer what the SIGNATURE already documents for the missing
   device — `false` from a constructor, `0` from a handle, nothing from a setter
   — not a refusal and not a plausible-looking success.  This is loft#709's
   disposition: unavailability is a fact about this run on this target, not
   about whether the program is well-formed.
4. Re-export publicly every `n_*` entry point the generated code calls.  The
   backends reach them differently and only one notices: `--native` loads the
   cdylib and binds the `#[unsafe(no_mangle)]` SYMBOL, which a private `use`
   still exports, while `--native-wasm` links the rlib statically and generated
   code writes the Rust PATH.  A privately-`use`d one is `E0603` on wasm with
   the native build green beside it (graphics' `n_audio_play_raw`).

**What gates this.**  The shared library CI
(`.github/workflows/library-ci-reusable.yml`) cross-builds every package's
`[native] crate` for wasm32 in its **WASM cross-build** step, unless the package
declares `.wasm_exempt`, whose contents are the stated reason and are printed into
the run's summary.  `loft test` itself has no `--native-wasm` mode — it compiles a
program and a suite is not one — so the cross-build proves the crate BUILDS for
wasm, not that the suite passes there.  A package can hold a sharper guard of its
own: graphics' `native/tests/headless_safety.rs` holds an ALLOW-list of wasm-clean
unconditional dependencies (always runs, no toolchain needed, so a NEW dependency
fails until someone places it).  A deny-list of known-bad crates would be silent
about the next one.

### The rlib-stomp hazard — eliminated by per-shape isolation (@PLN100 Slice 1)

> **As of @PLN100 Slice 1 the stomp cannot happen and no manual step is needed.**
> `loft --html` now builds + links loft's own wasm runtime rlib in an **isolated
> per-shape directory** — `target/loft/html/wasm32-unknown-unknown/release/` — so it
> never shares a file with the `make wasm` (wasm-bindgen) build.  The rlib is
> **auto-built on stale/missing**, keyed on the running loft build's content
> fingerprint (`native_utils::ensure_loft_runtime_rlib`, gated by the same
> `.loft-build-fp` sidecar the native-package auto-build uses), so `loft --html`
> and `loft --native-wasm` need no prior `make` and never link a stale rlib.  Set
> `LOFT_NO_AUTO_REBUILD=1` to link the existing rlib instead of triggering a build.
> The historical hazard below is retained for context — it applies only to the
> *shared* `target/wasm32-unknown-unknown/release/libloft.rlib` that `make wasm`
> still writes and `loft --html` no longer reads.

`make wasm` (wasm-pack, **`feature=wasm`**) and `loft --html` historically wrote
**the same** `target/wasm32-unknown-unknown/release/libloft.rlib`, but with
**incompatible feature sets**:

- The **wasm-pack** rlib has `feature=wasm` → pulls in `wasm-bindgen`/`js_sys`,
  so the wasm imports `__wbindgen_placeholder__` (35+ entries) that must be
  resolved by the `wasm-bindgen` CLI + generated JS glue.
- The **`--html`** rlib is built `--no-default-features --features random`
  (**no** `wasm`).  Its host bridge is the raw `extern "C"` `loft_gl` / `loft_io`
  modules that the embedded `doc/loft-gl-wasm.js` provides — **zero** wbindgen
  imports.

If `loft --html` links the wasm-pack variant (i.e. you ran `make wasm` last),
the bundle imports `__wbindgen_placeholder__`, which the HTML glue does not
provide, and the page fails to instantiate
(`Import … __wbindgen_placeholder__: module is not an object`).

**Rule (pre-@PLN100, now automatic):** the `--html` shape's rlib is rebuilt for
you — `loft --html` auto-builds it into the isolated `target/loft/html/` dir.  The
manual rebuild that used to be required after `make wasm` is retired:

```bash
# No longer needed — loft --html does this itself into target/loft/html/:
cargo build --release --target wasm32-unknown-unknown --lib \
    --no-default-features --features random
```

`make game` also does this (step 3); `make wasm-html-test` rebuilds it for the
test gate.  The integrity
gate `node tools/check_html_bundle.mjs <bundle.html>` (run by `make game`
step 6) catches both this stomp and a missing-`wasm-opt` (no-asyncify) bundle
before either ships.

**@P350 — `loft --html` self-guards the stomp inline.**  As of 2026-05-26 a
bare `loft --html` no longer needs the external gate to catch the stomp: it
parses the emitted wasm's import section
(`native_utils::html_wasm_import_modules_ok`) and **hard-aborts** (exit 1, no
HTML written) if any import module is not `loft_gl`/`loft_io`, printing the
rebuild command above.  So a stomped rlib produces a clear error, never a
silently-broken bundle.  The companion `wasm-opt`/asyncify check stays a *loud
warning* (not an abort): whether a bundle needs asyncify depends on whether the
program frame-yields, so a compute-only `--html` program is valid without it.

---

### Library `[wasm.bridge]` packages — build, link, verify (@PLN84)

A registry/`--lib` library can run its `#native` functions in `--html` builds via a
`[wasm.bridge]` in its `loft.toml`. The `crypto` library (loft-libs-core) is the
worked reference — all 15 primitives bridge `native == wasm`. Two mechanisms:

1. **Routed pure-compute (`[wasm.bridge.routes]`).** `n_<x> = "<bridge_fn>"` maps a
   `#native` to a `pub fn` in the bridge crate (`wasm/src/lib.rs`); codegen calls it
   directly. Text/scalar/bool in+out, runs synchronously to completion. The cleanest
   way to keep `native == wasm` is to `#[path]`-SHARE the native crate's modules
   (byte-identical), e.g. crypto shares `sha256.rs`/`base64.rs`/`ed25519.rs`/… .
2. **Host imports** (the `random_fill` / WebSocket pattern). The bridge declares
   `#[cfg(target_arch="wasm32")] #[link(wasm_import_module="loft_<x>")] unsafe extern
   "C" { fn f(…); }` and `host.js` pushes a `LOFT_WASM_EXTENSIONS` callback
   `(imports, ctrl, getMem)` that adds `imports.loft_<x>.f`. For anything touching
   the live JS world (RNG, sockets, time). Bytes cross via the ptr/len ABI over
   `getMem().buffer` (always re-fetch — wasm memory can grow/detach).

#### The build-extension (deps beyond `loft`)

If the bridge crate's `Cargo.toml` declares dependencies beyond `loft` (e.g.
dalek/RustCrypto), `loft --html` builds those deps for `wasm32-unknown-unknown`,
`--extern`s each direct dep on the bridge rustc compile, and `-L`s the deps dir on
both the bridge compile and the main wasm link (`src/main.rs` — the `--html` bridge
loop). The bridge still links the SHARED prebuilt loft (no duplicate loft); the deps
are loft-independent. Deterministic crypto ops need no RNG → no getrandom/wasm-bindgen.
Dep-less bridges skip this (the `has-nonloft-deps` gate), so existing `--html` tests
are untouched. Import-module names must use a **`loft_` prefix** (the `--html`
stomp-guard in `src/native_utils.rs` allows `loft_*`, rejects `__wbindgen_*`).

The dep build does **not** `cargo build` the bridge's own `wasm/Cargo.toml` — that
manifest carries a `loft = { path = "../../../loft" }` dep that resolves only in a
dev checkout, never for a registry-installed package (where the relative path points
at the nonexistent `~/.loft/loft`, so cargo aborts before building any dep — this was
[#446](https://github.com/loft-lang/loft/issues/446)). That `loft` dep is redundant
here anyway: the bridge links the SHARED prebuilt loft via `--extern`, not through the
manifest. So the driver synthesizes a throwaway **deps-only crate** — an empty lib
whose `[dependencies]` are exactly the bridge's non-`loft` deps (`bridge_nonloft_deps`
/ `synth_bridge_deps_manifest` in `src/main.rs`) — and builds THAT. cargo compiles
every declared dep regardless of the empty lib, producing the identical wasm32 dep
rlibs without ever resolving the `loft` path. Works uniformly for dev-checkout and
registry consumption; guarded by unit tests in `src/main.rs` (the synth manifest must
never contain a `loft` line).

### Gotcha — clear `~/.cache/loft` before `--html` if you just ran `--interpret`

The startup cache (#322) can serve a stale package entry to `--html` after an
`--interpret` run of the same program — the freshly-added `[wasm.bridge].routes`
don't apply and the build fails with `n_<x>(cell, …)` "unrouted native" / `can't be
dereferenced` errors. Workaround: `rm -rf ~/.cache/loft` immediately before the
`--html` build (the registry under `~/.loft` is separate, untouched). (Candidate
cache-invalidation bug — the route table isn't keyed on the lib's `loft.toml` change
across modes.)

### Why the page filesystem could not reuse the `loftHost.fs_*` bridges

Those are `js_sys::Reflect` lookups compiled under the `wasm` (wasm-bindgen) Cargo
feature.  `--html` deliberately built its runtime rlib `--no-default-features --features
random`, and it refused to write a page whose wasm imported anything beyond `loft_gl` and
`loft_io`.  So the file functions were raw `loft_io` imports declared in `src/lib.rs`, and
`src/wasm.rs` picked the transport.  Until loft#851, `--html` bound no filesystem at all: the
file calls compiled, each took an inert branch and answered "absent", and the build reported
success.

### `loft --html` threads

A program with a reachable `par` got the threaded runtime automatically; `--no-threads`
opted out and `--threads` forced it on.  It linked loft's pool through an atomics-std sysroot
assembled from the `build-std` output, with `native_utils::WASM_THREAD_FLAGS` — the linker
list `make wasm-mt` passes — so a page stayed one self-contained file with no wasm-bindgen
in it.  A `loft --html` page started its workers itself (`loftInstantiate`, inlined).
