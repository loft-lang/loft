
# WASM Runtime — Virtual Filesystem, Host Bridges, and Node.js Testing
## Contents
- [Overview](#overview)
- [Which wasm build is this? (three distinct worlds)](#which-wasm-build-is-this-three-distinct-worlds)
- [Build Toolchain Dependencies](#build-toolchain-dependencies)
- [Library `[wasm.bridge]` packages — build, link, verify (@PLN84)](#library-wasmbridge-packages--build-link-verify-pln84)
- [Host Bridge API](#host-bridge-api)
- [Browser vs Node.js Host Comparison](#browser-vs-nodejs-host-comparison)
- [Implementation Notes](#implementation-notes)
- [Cargo Feature Gates](#cargo-feature-gates)
- [Split-out subjects](#split-out-subjects)
- [See also](#see-also)

---

## Overview

When loft runs as a WASM module (`wasm` Cargo feature), it cannot access
the real filesystem, system clock, or OS random source.  It calls out to
the JavaScript host through `wasm-bindgen` externs grouped under a
`loftHost` namespace: browser APIs in production, in-memory fakes in tests.

Three layers: a **JSON virtual filesystem** for file/directory behaviour
without disk, the **host bridge API** for random/time/env/storage, and a
**Node.js test harness** tying them together for automated testing.

## Which wasm build is this? (three distinct worlds)

loft produces **three different wasm builds with different host surfaces** —
conflating them is the #1 scoping mistake (a consumer read this doc's Host
Bridge API and concluded `--html` has a filesystem; it does not):

| Build | Target | Host surface | Use it for |
|---|---|---|---|
| **`--html`** | `wasm32-unknown-unknown` cdylib, raw `extern "C"` (no wasm-bindgen), driven by `doc/loft-gl-wasm.js` + `doc/loft-fs.js` | `loft_io.loft_host_print`, `loft_io` input queue + `host_output`, the `loft_io.loft_host_fs_*` filesystem (loft#851), `loft_gl.*`, plus any `[wasm.bridge]` routes a used library ships (e.g. `web`'s WebSocket).  **No args, no env.** | The browser.  Small: ~1.1 MB (~330 KB gz) for a real kernel |
| **`--native-wasm`** | `wasm32-wasip2`, full `std` + WASI + component adapter | WASI (a real fs, args, env via the runtime) | Headless/server wasm.  **Compile-only** — needs an external runtime (wasmtime).  ~4× heavier than `--html` (measured 5.4 MB / 1.5 MB gz on the same kernel) — do not ship it to a browser |
| **IDE `make wasm`** | wasm-bindgen build (`wasm` Cargo feature) | the `globalThis.loftHost` bridges **this document describes** | The in-browser IDE/playground |

Everything below — the virtual filesystem, the Host Bridge API, the Node
harness — applies to the **third** build only.  For `--html` see
[HTML_EXPORT.md](HTML_EXPORT.md) (its host-import list is complete);
for the browser-interaction design see [BROWSER_INTEROP.md](BROWSER_INTEROP.md).

### What each target binds

The table above says which world a build belongs to.  This one says what a
program can actually CALL in each, by host group — the question a consumer
arrives with, and the one that previously took a grep of an emitted page to
answer (loft#851).

| Host group | interpreter / `--native` | `--html` | `--native-wasm` | IDE `make wasm` |
|---|---|---|---|---|
| file I/O (`file`, `read_bytes`, `write_bytes`, `list_dir`, `exists`, …) | yes | yes (page FS, `doc/loft-fs.js`) | yes (WASI) | yes (VirtFS / `loftHost.fs_*`) |
| graphics + audio (`gl_*`, `audio_*`) | yes | yes | no | no |
| stdout / stdin (`println`, `host_input`) | yes | yes | yes | yes |
| structured messages (`host_output` → `loftPush`) | no | yes | no | no |
| http (`store_load_url`, range reads) | yes | yes | yes | yes |
| clock (`now`, `ticks`) | yes | yes | yes | yes |
| args / env | yes | **no** | yes | yes |
| key–value storage | no | no | no | yes (`loftHost.storage_*`) |

**Calling into a group this target does not bind is not an error.**  The loft
side compiles either way and the call answers as if the resource were absent —
a write reports failure, a read answers null, a size answers 0.  That is the
deliberate rule (loft#709: one source runs on every target, so a call this
target cannot serve answers at runtime rather than refusing the build).

### How deep a program can recurse (loft#1059)

The call-stack cap is loft's on two backends and the **host engine's** on wasm.
`--interpret` and `--native` halt at `State::MAX_CALL_DEPTH` (10 000 frames) with
loft's own diagnostic.  A wasm module has no say: recursion is bounded by the
stack the engine gives it, and running out is a **trap** — not a Rust panic, so
the hook that renders a panic to the page never runs.

Measured with `rec(n)` under `wasmtime`: 4 000 frames answer, 8 000 abort with
`wasm trap: call stack exhausted` (exit 134).  The module's own shadow stack is
**not** what runs out — linking at `-zstack-size=` 1, 2, 4 and 8 MiB changes
nothing.  Raising the host's limit is what moves it, and given a host stack that
can hold the cap the module then agrees with the other two backends exactly —
same stdout, same exit code, same message, same frames:

```
$ wasmtime -W max-wasm-stack=8388608 prog.wasm
error: call stack overflow — exceeded 10000 stack frames
```

So on `--native-wasm` this bound is **whoever runs the `.wasm`** to raise, and
loft's part is to say so.  Lowering the cap on wasm instead would halt programs
that work today, and the right number is a property of the host that the
compiler cannot know.

On `--html` loft owns the JS that calls in, so the trap is caught and reported to
the page and the console rather than lost — see
[HTML_EXPORT.md § When a page faults](HTML_EXPORT.md).

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

### The page filesystem (`--html`)

A page draws AND stores.  `--html` used to bind no filesystem at all: the file
calls compiled, each took an inert branch and answered "absent", and the build
reported success — so a rendering editor in a page could not save, and the
consumer found that out by grepping the emitted bundle (loft#851).

**What a page gets.**  The whole file surface — `file(p)` with `#size` /
`#next` / `#read(n)` / `+=`, `read_bytes` / `write_bytes`, `delete` / `move` /
`mkdir` / `mkdir_all` / `list_dir` / `is_dir` / `is_file` / `exists`, and
`set_file_size` — answering exactly what `--interpret` and `--native` answer for
the same program.  Both page shells bind it, so a program that only stores still
gets the minimal engine-less page rather than the WebGL2 one.

**Where the bytes live.**  `doc/loft-fs.js` is the host half: an immutable
**base tree** the page supplies, plus a **delta** holding every write, persisted
to `localStorage`.  Reads consult the delta and fall back to the base; writes
only ever land in the delta.  So closing the tab keeps the user's work and
`resetToBase()` throws it away — the shape `LayeredFS` (§ *Layered Filesystem*)
proved for the IDE, which is what a page actually needs.

Four page globals tune it, all optional:

| global | meaning |
|---|---|
| `loftBaseFS` | `{"/abs/path": string \| Uint8Array}` — the read-only base tree (default empty).  `--html` seeds it from each `[[embed]]` in `loft.toml` (@PLN146 F4), adding to whatever the page already set |
| `loftFSKey` | `localStorage` key for the delta (default `loft-fs-delta`) |
| `loftFSPersist` | `false` keeps the delta in memory for the tab's lifetime |
| `loftFSCwd` | what a relative path resolves against (default `/`) |

**It is the PAGE's filesystem, not the user's disk.**  A browser cannot read
`/home/you/data.csv`, and nothing here pretends otherwise: a path the page was
not given simply reads as absent.  Seed `loftBaseFS` with the data the program
needs, or fetch it (`store_load_url_trusted`).

**Two things a page still answers differently.**  A failed mutation reports
`Other` where native distinguishes `NotFound` / `PermissionDenied` /
`IsDirectory` — the host bridge carries success and failure, not an errno.  And
a `localStorage` quota failure is reported once to the console and then
tolerated: the run continues against the in-memory delta, because losing the
session is worse than losing the persistence.

**Why it could not reuse the `loftHost.fs_*` bridges below.**  Those are
`js_sys::Reflect` lookups compiled under the `wasm` (wasm-bindgen) Cargo
feature.  `--html` deliberately builds its runtime rlib
`--no-default-features --features random`, and it REFUSES to write a page whose
wasm imports anything beyond `loft_gl` and `loft_io` — a wasm-bindgen build
imports `__wbindgen_placeholder__` 35+ times and cannot instantiate against the
page's raw-extern glue (§ *The rlib-stomp hazard*).  So the file functions are
raw `loft_io` imports declared in `src/lib.rs`, and `src/wasm.rs` is the ONE
place that picks a transport: `js_sys` under the `wasm` feature, raw imports
otherwise.  Everything above it calls `host_fs_*` and never learns which it got,
which is why the two browsers cannot answer differently.

The build-side name for "this build reaches the filesystem through a JS host" is
the `host_fs` cfg (`build.rs`), set for the wasm-bindgen bundle and for `--html`
and deliberately NOT for `wasm32-wasip2`, which has a real WASI filesystem.

---

## Build Toolchain Dependencies

loft targets four backends and they do **not** share a toolchain.  A machine
that compiles the interpreter fine can still silently ship a broken browser
bundle if the WASM tools are missing.  Run **`make doctor`** for a live status
report; the table below is the reference.

| Tool | Needed for | If missing |
|---|---|---|
| `rustc` / `cargo` | everything | nothing builds |
| `node` (≥18) | WASM Node tests + `--html` bundle integrity check | tests skip; `make game` can't gate the bundle |
| **`wasm-opt`** (binaryen) | **`--html`** — the `--asyncify` pass that enables frame-yield | **bundle has no frame-yield → render loop HANGS the browser tab** ("page times out", @P337) |
| `wasmtime` | `wasm32-wasip2` (`--native-wasm`), @P334 | can't run/test the wasip2 backend locally |
| `wasm-pack` | `make wasm` (doc/pkg gallery + browser playground) | gallery/playground can't rebuild |

⚠ **Editing `default/*.loft` — even one doc-comment — reddens the two
`engine_host_connector` tests.** `doc/pkg` is a COMMITTED bundle, and
`scripts/wasm_bundle_stamp.sh` hashes the whole of `default/` along with
`src/{engine_host,wasm,native,compile}.rs`, `lib/engine_host/src/engine_host.loft`
and three `doc/` page files. The stamp is a content hash, so it does not care that
your change was a comment: the tests refuse to run against a bundle whose stamp
disagrees, with *"doc/pkg was built from a different tree"*. The fix is the one they
name — `make wasm`, then commit the rebuilt `doc/pkg` **and** `doc/pkg-src.stamp`
together. Budget ~30 s and a ~4.5 MB re-commit. Before assuming the drift is yours,
compare the stamp at HEAD against the committed one; the same failure appears when
someone else's bundle was already stale.
| `wasm-bindgen` CLI | *bundled by wasm-pack* | **not** needed for `--html` (it uses a raw `extern "C"` bridge, not wasm-bindgen) |
| a browser (Chromium/Firefox) | real `--html` verification | can only stub-instantiate, not render |
| rustup targets `wasm32-unknown-unknown` + `wasm32-wasip2` | `--html` / `--native-wasm` | cross-compile fails (E0463) |

Install on Debian/Ubuntu: `apt install binaryen wasmtime` (or download
releases); `cargo install wasm-pack`; `rustup target add
wasm32-unknown-unknown wasm32-wasip2`.

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

## Library `[wasm.bridge]` packages — build, link, verify (@PLN84)

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

### The build-extension (deps beyond `loft`)

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

### Headless verify loop (no browser)

```bash
# build a KAT .loft to a browser-wasm bundle against the LOCAL lib
loft --html /tmp/kat.html --lib /path/to/lib /path/to/kat.loft
# extract the wasm and run it under node (node installed user-local at ~/.local/bin)
python3 -c "import re,base64;h=open('/tmp/kat.html').read();\
m=re.search(r'wasmB64=\"([A-Za-z0-9+/=]+)\"',h);\
open('/tmp/kat.wasm','wb').write(base64.b64decode(m.group(1)))"
# pure-compute bridge: wasm_repro finds lib/*/wasm/host.js automatically
node tools/wasm_repro.mjs /tmp/kat.wasm           # exit 0 = asserts passed
# a loft-libs-core bridge's host.js (outside <repo>/lib): point at it explicitly
LOFT_WASM_HOST_JS=/path/to/lib/wasm/host.js node tools/wasm_repro.mjs /tmp/kat.wasm
```

A deterministic primitive's KAT passing on both `--interpret`/native AND this wasm
run proves `native == wasm`. **`wasm_repro.mjs` runs `loft_start()` synchronously
with no asyncify resume loop** — it is a trap+compute harness; it CANNOT drive an
interactive/event-driven client (WebSocket) — that needs a new asyncify-aware driver
(see [`plans/84-zt-c-web-ws-bridge.md`](plans/84-zt-c-web-ws-bridge.md) § 9).

### Gotcha — clear `~/.cache/loft` before `--html` if you just ran `--interpret`

The startup cache (#322) can serve a stale package entry to `--html` after an
`--interpret` run of the same program — the freshly-added `[wasm.bridge].routes`
don't apply and the build fails with `n_<x>(cell, …)` "unrouted native" / `can't be
dereferenced` errors. Workaround: `rm -rf ~/.cache/loft` immediately before the
`--html` build (the registry under `~/.loft` is separate, untouched). (Candidate
cache-invalidation bug — the route table isn't keyed on the lib's `loft.toml` change
across modes.)

---

## Host Bridge API

> **Scope: the wasm-bindgen (`wasm` feature / IDE) build ONLY.**  A `--html`
> bundle has none of these bridges — its complete host-import set is
> print + `host_input` + GL (+ library `[wasm.bridge]` routes); see
> [HTML_EXPORT.md](HTML_EXPORT.md) and the three-worlds table above.

The WASM module imports functions from a `loftHost` namespace. The host (browser or
Node.js) populates `globalThis.loftHost` before initialising the WASM module.

### Filesystem bridge

These functions map 1:1 to the loft `File` stdlib operations. On the Rust side,
`#[cfg(feature = "wasm")]` branches in `src/state/io.rs` call these instead of
`std::fs`.

```js
globalThis.loftHost = {
  // --- files ---
  fs_exists(path)                     -> boolean,
  fs_read_text(path)                  -> string | null,
  fs_read_binary(path, offset, len)   -> Uint8Array | null,
  fs_write_text(path, content)        -> i32,     // 0=ok, error code otherwise
  fs_write_binary(path, bytes)        -> i32,
  fs_file_size(path)                  -> number,  // bytes, -1 if not found
  fs_delete(path)                     -> i32,
  fs_move(from, to)                   -> i32,
  fs_mkdir(path)                      -> i32,
  fs_mkdir_all(path)                  -> i32,

  // --- directories ---
  fs_list_dir(path)                   -> string[], // entry names
  fs_is_dir(path)                     -> boolean,
  fs_is_file(path)                    -> boolean,

  // --- binary cursor ---
  fs_seek(path, pos),
  fs_read_bytes(path, n)              -> Uint8Array | null,
  fs_write_bytes(path, bytes)         -> i32,
  fs_get_cursor(path)                 -> number,

  // --- paths ---
  fs_cwd()                            -> string,
  fs_user_dir()                       -> string,
  fs_program_dir()                    -> string,
  // ...
};
```

**Return codes** (matching `FileResult` enum):

| Code | Meaning |
|------|---------|
| 0 | `Ok` |
| 1 | `NotFound` |
| 2 | `PermissionDenied` |
| 3 | `IsDirectory` |
| 5 | `Other` |

The current loft wasm bridge distinguishes only `0` (→ `Ok`) from any nonzero code
(→ `Other`); codes 1–3 / 5 are the host-protocol space (mirroring the native OS-error
classification) reserved for a future wasm classification pass. Code 4 was retired with
the `FileResult.NotDirectory` variant (@PLN102 H9 — it could not be produced portably).

### Random bridge

```js
globalThis.loftHost = {
  // ...
  random_int(lo, hi)                  -> number,   // integer in [lo, hi]
  random_seed(seed_hi, seed_lo),                    // 64-bit seed split into two i32
};
```

**Browser implementation:** uses `crypto.getRandomValues()` for proper randomness,
with a seedable PCG fallback for `rand_seed()`.

**Node.js test implementation:** uses a seeded PRNG for deterministic tests.

### Time bridge

```js
globalThis.loftHost = {
  // ...
  time_now()                          -> number,   // ms since epoch (like Date.now())
  time_ticks()                        -> number,   // us since program start
};
```

**Node.js:** `Date.now()` for `time_now()`; `process.hrtime.bigint() / 1000n` for
ticks (converted to a number, offset from start).

### Environment bridge

```js
globalThis.loftHost = {
  // ...
  env_variable(name)                  -> string | null,
};
```

**Node.js:** `process.env[name] ?? null`.
**Browser:** returns from a pre-configured `Map` (or null — browsers have no env vars).

### Arguments bridge

```js
globalThis.loftHost = {
  // ...
  arguments()                         -> string[],  // command-line arguments
};
```

**Browser:** returns `[]` or values injected by the IDE's "Program arguments" field.
**Node.js:** `process.argv.slice(2)` or a test-supplied array.

### Logging bridge

```js
globalThis.loftHost = {
  // ...
  log_write(level, message),          // level: "info"|"warn"|"error"|"fatal"
};
```

**Browser:** dispatches to `console.info()`, `console.warn()`, `console.error()`.
**Node.js:** same — `console` methods. No file rotation or directory creation.

See [Logging in WASM](WASM_HOST_MEDIA.md#logging-in-wasm) for the full design.

### Storage bridge (browser only)

For browser-side persistent storage beyond the virtual filesystem:

```js
globalThis.loftHost = {
  // ...
  storage_get(key)                    -> string | null,
  storage_set(key, value),
  storage_remove(key),
};
```

**Browser:** backed by `localStorage` for small string data or IndexedDB for binary/large
data (see [WEB_IDE.md](lib_plans/62-web-ide/README.md) § M4 for IndexedDB project storage).

**Node.js:** backed by a plain `Map` — no persistence needed for tests.

---

## Browser vs Node.js Host Comparison

| Capability | Browser host | Node.js test host |
|---|---|---|
| Filesystem | VirtFS backed by IndexedDB (persistent) | VirtFS in-memory (ephemeral) |
| Random | `crypto.getRandomValues()` + seedable PCG | Deterministic xoshiro128** |
| Time | `Date.now()` / `performance.now()` | Configurable fake values |
| Environment | Pre-configured `Map` (no real env) | `process.env` or fake `Map` |
| Storage | `localStorage` / IndexedDB | In-memory `Map` |
| Output | Thread-local buffer → console panel | Thread-local buffer → string |
| Binary data | Native `ArrayBuffer` in IndexedDB | base64 in JSON tree |
| Persistence | IndexedDB survives reload | None (per-test lifecycle) |
| Threading | Tier 2 (Web Workers) if COEP/COOP, else Tier 1 (sequential) | Sequential (Tier 1) |
| Logging | `console.*` methods, optional IDE log panel | `console.*` methods |
| PNG images | Decoded in WASM from VirtFS bytes | Decoded in WASM from VirtFS bytes |
| Arguments | IDE "arguments" field or `[]` | Test-supplied array or `[]` |

### When to use which

- **Node.js harness** — CI, automated regression tests, deterministic reproducibility.
  Every test starts from a known JSON tree and gets snapshot isolation.
- **Browser harness** — manual testing, the Web IDE itself. Uses the same `loftHost`
  interface but backed by real browser APIs. Can optionally swap in VirtFS for
  browser-side unit tests too (via `tests/runner.html`).

---

## Implementation Notes

### Rust-side `#[cfg(feature = "wasm")]` dispatch

Each native function that touches the OS gets a conditional branch:

```rust
// Example: file exists check in src/state/io.rs
#[cfg(feature = "wasm")]
fn host_exists(path: &str) -> bool {
    crate::wasm::call_host_bool("fs_exists", path)
}

#[cfg(not(feature = "wasm"))]
fn host_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}
```

The `crate::wasm` module provides typed wrappers around the raw `wasm-bindgen` extern
functions declared in `src/wasm.rs`. See [WEB_IDE.md](lib_plans/62-web-ide/README.md) § Rust Changes for
the full list of extern declarations.

### Binary data across the WASM boundary

Wasm linear memory and JS `ArrayBuffer` are separate. `wasm-bindgen` handles the
copy automatically for `&[u8]` ↔ `Uint8Array`. The VirtFS stores binary content as
base64 in the JSON tree (for serialisation) but decodes to `Uint8Array` at the API
boundary. In the browser with IndexedDB, binary data stays as `ArrayBuffer` natively
— no base64 overhead.

### Async operations

All `loftHost` functions are **synchronous**. The loft interpreter executes
sequentially and cannot yield to the JS event loop mid-execution. This means:

- **IndexedDB** (async) cannot be called directly from a host function. The browser
  host must pre-load project files into a VirtFS instance before calling
  `compileAndRun()`, and flush writes back to IndexedDB after execution completes.
- **`localStorage`** is synchronous and can be called directly for `storage_*`
  functions.
- For future async needs (HTTP fetch, etc.), the approach is: pre-fetch before
  execution, pass results via the virtual filesystem or a data bridge.

### Error mapping

JS host functions return integer error codes (see the table above). The Rust side
maps these to the `FileResult` enum. If a host function throws a JS exception,
`wasm-bindgen` converts it to a Rust panic — host functions must never throw; they
return error codes instead.

### Test isolation pattern

Every WASM integration test follows this lifecycle:

```
1. createHost(tree)          — fresh VirtFS + host from a fixture
2. globalThis.loftHost = host
3. compileAndRun(code)       — exercises the WASM module
4. assert on output, fs state, storage state
5. (no cleanup needed — next test creates a new host)
```

For tests that mutate the filesystem mid-test and need to verify intermediate states,
use `fs.snapshot()` / `fs.restore()`.

---

## Cargo Feature Gates

The WASM build disables OS-dependent features and enables the host bridge. Two WASM
build profiles exist — single-threaded and multi-threaded:

```toml
# Cargo.toml feature definitions
[features]
default    = ["png", "mmap", "random", "threading"]
wasm       = ["dep:wasm-bindgen", "dep:serde", "dep:serde-wasm-bindgen",
              "dep:js-sys", "dep:web-sys", "png"]
wasm-threads = ["wasm", "wasm-native-threads"]   # gallery bundle: wasm-bindgen + the pool
wasm-native-threads = ["threading", "rayon/web_spin_lock", "dep:wasm_sync"]
png        = ["dep:png"]
mmap       = ["dep:mmap-storage"]         # disabled for WASM — no file-backed mmap
random     = ["dep:rand_core", "dep:rand_pcg"]  # disabled for WASM — host provides RNG
threading  = []                            # enables par() — OS threads or Web Workers

[patch.crates-io]
# rayon's `web_spin_lock` is what keeps a `par` on the browser main thread from
# throwing "Atomics.wait cannot be called in this context" — that thread may not
# block, and rayon takes a lock there on every join.  The `wasm_sync` that
# feature pulls detects the main thread with `web_sys::window()`, i.e. needs
# wasm-bindgen, which the raw `--html` wasm must not contain — so loft supplies
# its own (@PLN117).
wasm_sync = { path = "wasm-sync" }
```

**Build commands:**

```sh
# Single-threaded WASM (works everywhere, including file://)
wasm-pack build --target web -- --features wasm --no-default-features

# Multi-threaded WASM (par() over real Web Workers) — use `make wasm-mt`.
# @PLN117: this toolchain's rustc does NOT auto-emit the wasm-threads linker
# flags from +atomics, so ALL of these are required.  Drop any one and the
# bundle silently builds a NON-shared memory → workers die at runtime with
# "Memory could not be cloned":
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals \
  -C link-arg=--shared-memory -C link-arg=--max-memory=1073741824 \
  -C link-arg=--import-memory -C link-arg=--export=__heap_base \
  -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size \
  -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base \
  -C link-arg=--export=__stack_pointer' \
  rustup run nightly \
  wasm-pack build --target web --out-dir tests/wasm/pkg-mt --release \
  -- --no-default-features --features wasm-threads -Z build-std=panic_abort,std
```

`--export=__stack_pointer` is loft's own (@PLN117): every `WebAssembly.Instance`
starts with the SAME stack pointer, so the worker bootstrap must move each worker
onto its own shadow stack before it runs any wasm.  The same list is in
`native_utils::WASM_THREAD_FLAGS`, which is what `loft --html` passes.

Requires the **nightly** toolchain with **rust-src** (`build-std` rebuilds std with
atomics). `--target web` is mandatory — the worker bootstrap imports the generated
glue as a module, and node has no Web Worker global, so the **browser** is the proof
environment.

**`loft --html` threads too, and needs no flags.** A program with a reachable
`par` gets the threaded runtime automatically; `--no-threads` opts out and
`--threads` forces it on.  It links the same pool through an atomics-std sysroot
assembled from the `build-std` output, so a page stays ONE self-contained file
with no wasm-bindgen in it.  Gate: `tests/wasm/html-thread-proof.sh`.

**Host-sized types are the wasm hazard class.** A pointer is 8 bytes natively and
**4 on wasm32**, so anything sized off a Rust type that contains one differs
between the two targets — `Str` (`ptr + len`) is 16 bytes natively and 8 on wasm,
`String` 24 and 12.  loft sizes `text` slots from exactly those types, which is
correct and self-consistent; what breaks is code that *hardcodes the 64-bit
number*.  Codegen did this for the fn-ref ops — `OpVarFnRef` / `OpPutFnRef` move a
20-byte fn-ref but are DECLARED taking a `text`, and the correction was written
`step(20) - step(16)`.  On wasm the declared slot is 8, not 16, so every
`f = some_fn` left the operand stack 8 bytes out: a bind faulted, a call through
it freed a garbage pointer.  Native was correct throughout, which is why it
survived so long.  The correction now reads its size from `Str`
(`Stack::fnref_signature_gap`), and a unit test pins the two homes of the fn-ref
size together.  **When touching stack arithmetic, size a slot from the type, never
from the number you measured on x86_64.**

A sweep of every literal size in the slot/stack code (`step(<lit>)`, `< 16`
padding thresholds, `args_size`, and size-keyed `match` arms) found two more of
the same class and confirmed the rest are target-independent (an i64/f64/DbRef=12
is the same width everywhere; only `Str`/`String`/pointer sizes move):
- **the text-yielding coroutine's dispatch arm** was keyed on the literal `16`,
  so on a 32-bit host it would fall through to the i64 arm and mis-transport the
  yielded string — latent (only the host runs codegen today), now
  `n == size_of::<&str>()`;
- **a `par` text-input worker's call frame** recorded `args_size: 16`, and the
  stack-trace / variable-snapshot readers scan that many bytes — 8 too many on
  wasm, so a browser `stack_trace()` inside such a worker walked past the
  argument.  Now `size_of::<Str>()`.
Both are guarded by compared scripts in the node wasm suite (`22c-par-sources`,
`35p-iterator-match`, `51-coroutines`), which run against the native reference.

**COOP/COEP hosting contract (arc D).** The threaded bundle needs
`crossOriginIsolated === true` (the precondition for `SharedArrayBuffer` + wasm
atomics), which a host grants only by sending **both**:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

`require-corp` means every cross-origin sub-resource must itself opt in
(`Cross-Origin-Resource-Policy` / CORS) or it won't load — plan for it when a page
pulls cross-origin assets. Before any `par`, JS must
```js
const wasm = await init();
if (crossOriginIsolated) {
  const { startLoftWorkers } = await import('./pkg/loft-thread.js');
  await startLoftWorkers(wasm, navigator.hardwareConcurrency,
                         { memory: wasm.memory, mainJS: new URL('./pkg/loft.js', import.meta.url).href });
}
```

A `loft --html` page does this itself (`loftInstantiate`, inlined).  When the host
is **not** isolated, `startLoftWorkers` returns 0 and the same `par` runs
sequentially (one worker) and **never crashes** (proven).  `globalThis.loftThreads`
is how many worker threads a page actually got, and `?loftTrace=1` makes loft
report how many workers each individual `par` dispatched across. `tests/wasm/coi-server.py`
is a reference COOP/COEP server; `tests/wasm/par-thread-proof.{html,sh}` is the
in-browser proof (a `par` dispatched across 4 Web Workers, value matching native).

**What each gate controls:**

| Feature | Native | WASM (single) | WASM (threaded) | Notes |
|---|---|---|---|---|
| `threading` | ON | OFF | **ON** | OS threads / Web Workers / sequential fallback |
| `wasm-threads` | — | — | ON | wasm-bindgen bundle + loft's Web Worker pool |
| `wasm-native-threads` | — | — | ON | loft's pool itself — also what `loft --html` links, without wasm-bindgen |
| `mmap` | ON | OFF | OFF | No file-backed mmap in browser |
| `random` | ON | OFF | OFF | Host provides RNG via bridge |
| `png` | ON | **ON** | **ON** | Pure Rust — compiles to WASM |
| `wasm` | OFF | ON | ON | Host bridge, virtual FS, output capture |

---

## Split-out subjects

Whole sections moved out to keep this file under 1000 lines; headings are unchanged.

- [WASM_FILESYSTEM.md](WASM_FILESYSTEM.md) — JSON Virtual Filesystem; Layered Filesystem — Base Tree + Delta Overlay
- [WASM_TESTING.md](WASM_TESTING.md) — Node.js Test Harness; Test Compatibility Matrix
- [WASM_THREADING.md](WASM_THREADING.md) — Threading in WASM — Two-Tier Design; W1.18 — Node.js Worker Threads
- [WASM_IMPLEMENTATION_PLAN.md](WASM_IMPLEMENTATION_PLAN.md) — Implementation Plan; Function References (`CallRef`) in WASM — W1.15; Store Locks in WASM — W1.17
- [WASM_FRAME_YIELD.md](WASM_FRAME_YIELD.md) — Frame Yield — Browser Game Loop via Interpreter Suspension
- [WASM_HOST_MEDIA.md](WASM_HOST_MEDIA.md) — PNG Image Support in WASM; Logging in WASM

---

## See also
- [WEB_IDE.md](lib_plans/62-web-ide/README.md) — Full Web IDE architecture, milestones, Rust changes
- [STDLIB.md](STDLIB.md) § File System — loft file I/O API
- [STDLIB_RUNTIME.md](STDLIB.md) § Random — `rand()`, `rand_seed()`, `rand_indices()`
- [INTERNALS.md](INTERNALS.md) — Native function registry and `src/state/io.rs`
- [TESTING.md](TESTING.md) — Rust-side test framework
- [THREADING.md](THREADING.md) — Parallel execution model (native only)
- [LOGGER.md](LOGGER.md) — Logging framework (file-based in native, console in WASM)
- [WASM file-I/O steps (FS-A…F), shipped](plans/finished/wasm-file-io-steps/README.md) — the step-by-step W1.16 implementation plan.
