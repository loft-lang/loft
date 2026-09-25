
# WASM Runtime — Virtual Filesystem, Host Bridges, and Node.js Testing

## Contents
- [Overview](#overview)
- [Build Toolchain Dependencies](#build-toolchain-dependencies)
- [JSON Virtual Filesystem](#json-virtual-filesystem)
- [Layered Filesystem — Base Tree + Delta Overlay](#layered-filesystem--base-tree--delta-overlay)
- [Host Bridge API](#host-bridge-api)
- [Node.js Test Harness](#nodejs-test-harness)
- [Browser vs Node.js Host Comparison](#browser-vs-nodejs-host-comparison)
- [Implementation Notes](#implementation-notes)
- [Cargo Feature Gates](#cargo-feature-gates)
- [Threading in WASM — Two-Tier Design](#threading-in-wasm--two-tier-design)
- [W1.18 — Node.js Worker Threads: Testing `par()` Outside the Browser](#w118--nodejs-worker-threads-testing-par-outside-the-browser)
- [PNG Image Support in WASM](#png-image-support-in-wasm)
- [Logging in WASM](#logging-in-wasm)
- [Test Compatibility Matrix](#test-compatibility-matrix)
- [Frame Yield — Browser Game Loop via Interpreter Suspension](#frame-yield--browser-game-loop-via-interpreter-suspension)
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

## JSON Virtual Filesystem

### Design goals

- Support the full loft `File` API: `file()`, `content()`, `lines()`, `write()`,
  `files()`, `exists()`, `delete()`, `move()`, `mkdir()`, `mkdir_all()`, binary
  read/write, `seek`, `f#size`, `f#exists`, `f#next`.
- Represent the entire filesystem as a single JSON tree — serialisable, inspectable,
  diffable.
- Allow snapshot/restore for test isolation.
- Run identically in browser (IndexedDB-backed) and Node.js (in-memory).

### Tree structure

```jsonc
{
  "/": {                          // root directory
    "home": {                     // directory node: value is an object
      "user": {
        "project": {
          "main.loft": {          // text file node
            "$type": "text",
            "$content": "fn main() { println(\"hello\") }"
          },
          "data.bin": {           // binary file node
            "$type": "binary",
            "$content": "AQID/w=="  // base64-encoded bytes
          },
          "src": {                // subdirectory — nested object
            "lib.loft": {
              "$type": "text",
              "$content": "pub fn add(a: integer, b: integer) -> integer { a + b }"
            }
          }
        }
      }
    },
    "tmp": {}                     // empty directory
  }
}
```

**Conventions:**
- A key whose value is a plain `{}` or contains nested keys (without `$type`) is a
  **directory**.
- A key whose value is `{ "$type": "text", "$content": "..." }` is a **text file**.
- A key whose value is `{ "$type": "binary", "$content": "<base64>" }` is a **binary
  file**.  Content is base64-encoded.
- Special keys always start with `$` — no loft filename may start with `$`.

### VirtFS class

```js
export class VirtFS {
  // --- construction ---
  constructor(tree = { "/": {} })     // initialise from a JSON tree
  static fromJSON(json)               // parse a JSON string into a VirtFS
  toJSON()                            // serialise the current state

  // --- snapshot / restore (test isolation) ---
  snapshot()                          // returns a deep-cloned tree
  restore(snapshot)                   // replaces the tree with a prior snapshot

  // --- path resolution ---
  // All paths are absolute. Relative paths are resolved against cwd.
  resolve(path)                       // normalise: remove //, resolve . and ..
  private _navigate(path)             // returns { parent, name, node } or null

  // --- read operations ---
  exists(path) -> boolean
  isFile(path) -> boolean
  isDirectory(path) -> boolean
  stat(path) -> { type, size } | null
  readText(path) -> string | null
  readBinary(path) -> Uint8Array | null
  readdir(path) -> string[]           // entry names (not full paths)

  // --- write operations ---
  writeText(path, content)            // creates parent dirs as needed
  writeBinary(path, bytes)            // bytes: Uint8Array; stored as base64
  mkdir(path)                         // single level, error if parent missing
  mkdirAll(path)                      // recursive
  delete(path)                        // file only
  deleteDir(path)                     // directory (must be empty)
  move(from, to)                      // rename/relocate

  // --- binary cursor (per-file state for seek/read) ---
  // Maintains a Map<path, { cursor: number }> for binary file positions.
  seek(path, pos)
  getCursor(path) -> number
  readBytes(path, n) -> Uint8Array    // reads n bytes from cursor, advances
  writeBytes(path, bytes)             // writes at cursor, advances

  // --- working directory ---
  cwd                                 // current working directory (string)
  chdir(path)
}
```

### Path resolution rules

1. Paths use forward slashes. Backslashes are converted on input.
2. `resolve("/home/user/../user/./project")` → `"/home/user/project"`.
3. Trailing slashes are stripped (except for `"/"`).
4. `resolve("relative/path")` prepends `this.cwd`.

### Internal storage

The tree is a plain JS object. File content is stored as a JS string (text) or
base64 string (binary).  Binary cursors are kept in a separate `Map<string, number>`
keyed by absolute path — cursors reset when the file is written or deleted.

### Example: test setup

```js
const fs = new VirtFS({
  "/": {
    "project": {
      "main.loft": { "$type": "text", "$content": "fn main() { println(\"hi\") }" },
      "data": {
        "names.txt": { "$type": "text", "$content": "alice\nbob\ncharlie" }
      }
    }
  }
});

fs.cwd = "/project";

assert(fs.exists("/project/main.loft"));
assert(fs.isDirectory("/project/data"));
assert.deepEqual(fs.readdir("/project/data"), ["names.txt"]);
assert(fs.readText("/project/data/names.txt") === "alice\nbob\ncharlie");
```

### Example: snapshot isolation in tests

```js
test('write does not leak between tests', () => {
  const snap = fs.snapshot();

  fs.writeText("/project/temp.loft", "fn temp() {}");
  assert(fs.exists("/project/temp.loft"));

  fs.restore(snap);
  assert(!fs.exists("/project/temp.loft"));
});
```

---

## Layered Filesystem — Base Tree + Delta Overlay

### Problem

The Web IDE ships with example programs and user documentation (from `tests/docs/`
and `doc/*.html`). These are **read-only defaults** that every user should see. When
a user edits an example or creates a new file, only the **changes** should be persisted
to localStorage/IndexedDB — not a full copy of the entire default tree.

### Design: two-layer VirtFS

```
┌─────────────────────────────────────────────┐
│              LayeredFS (read path)           │
│                                             │
│  read(path):                                │
│    if delta.exists(path)  → return delta    │
│    if delta.deleted(path) → return null     │
│    if base.exists(path)   → return base     │
│    → return null                            │
│                                             │
│  write(path, content):                      │
│    delta.write(path, content)               │
│    (base is never mutated)                  │
│                                             │
│  delete(path):                              │
│    delta.markDeleted(path)                  │
│    (base entry remains but is shadowed)     │
│                                             │
│  readdir(path):                             │
│    merge base entries + delta entries        │
│    minus delta-deleted entries              │
├─────────────────────────────────────────────┤
│  base (immutable)   │  delta (persisted)    │
│  ─────────────────  │  ──────────────────   │
│  Bundled at build   │  localStorage or      │
│  time from:         │  IndexedDB. Starts    │
│  • tests/docs/*.loft│  empty. Only grows    │
│  • doc/*.html       │  when the user edits  │
│  • default/*.loft   │  or creates files.    │
│  Shipped as a       │                       │
│  static JSON file   │  Stored as JSON:      │
│  in ide/assets/     │  { files: {...},      │
│    base-fs.json     │    deleted: [...] }   │
└─────────────────────┴───────────────────────┘
```

### Base tree — `base-fs.json`

Generated at build time by `ide/scripts/build-base-fs.js`:

```jsonc
{
  "/": {
    "examples": {
      "01-hello.loft":    { "$type": "text", "$content": "fn main() {\n  println(\"Hello, world!\")\n}" },
      "06-function.loft": { "$type": "text", "$content": "// Functions example\n..." },
      "07-vector.loft":   { "$type": "text", "$content": "// Vector example\n..." },
      "10-structs.loft":  { "$type": "text", "$content": "..." }
      // ... all tests/docs/*.loft files
    },
    "docs": {
      "language.html":    { "$type": "text", "$content": "<!DOCTYPE html>..." },
      "stdlib.html":      { "$type": "text", "$content": "..." }
      // ... generated doc/*.html files
    },
    "lib": {
      "01_code.loft":     { "$type": "text", "$content": "..." },
      "02_files.loft":   { "$type": "text", "$content": "..." },
      "03_text.loft":     { "$type": "text", "$content": "..." }
    }
  }
}
```

This file is loaded once on startup and never written to.

### Delta format

The delta is a small JSON object persisted to localStorage (or IndexedDB for large
projects):

```jsonc
{
  "files": {
    "/examples/06-function.loft": {        // modified base file
      "$type": "text",
      "$content": "// My edited version\nfn main() { ... }"
    },
    "/my-projects/game/main.loft": {       // new user file
      "$type": "text",
      "$content": "fn main() { ... }"
    }
  },
  "deleted": [
    "/examples/01-hello.loft"              // user deleted this example
  ],
  "dirs": [
    "/my-projects",
    "/my-projects/game"
  ]
}
```

**Storage key**: `loft-ide-delta` (single key for simple projects), or per-project
keys `loft-ide-delta:<project-id>` when multi-project support (M4) is active.

### LayeredFS class

```js
export class LayeredFS extends VirtFS {
  constructor(baseTree, delta = null) {
    super(baseTree);                    // base becomes the read-only layer
    this._base = baseTree;              // keep reference
    this._delta = delta ?? { files: {}, deleted: [], dirs: [] };
  }

  // --- read: delta wins, then base ---
  exists(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return false;
    if (path in this._delta.files) return true;
    if (this._delta.dirs.includes(path)) return true;
    return super.exists(path);           // check base
  }

  readText(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return null;
    const df = this._delta.files[path];
    if (df) return df.$content;
    return super.readText(path);         // fall through to base
  }

  readdir(path) {
    path = this.resolve(path);
    const baseEntries = new Set(super.readdir(path) ?? []);
    // add delta files in this directory
    for (const p of Object.keys(this._delta.files)) {
      const dir = p.substring(0, p.lastIndexOf('/')) || '/';
      if (dir === path) baseEntries.add(p.substring(p.lastIndexOf('/') + 1));
    }
    // add delta dirs
    for (const d of this._delta.dirs) {
      const parent = d.substring(0, d.lastIndexOf('/')) || '/';
      if (parent === path) baseEntries.add(d.substring(d.lastIndexOf('/') + 1));
    }
    // remove deleted entries
    for (const del of this._delta.deleted) {
      const dir = del.substring(0, del.lastIndexOf('/')) || '/';
      if (dir === path) baseEntries.delete(del.substring(del.lastIndexOf('/') + 1));
    }
    return [...baseEntries];
  }

  // --- write: always goes to delta ---
  writeText(path, content) {
    path = this.resolve(path);
    this._ensureParentDirs(path);
    this._delta.files[path] = { $type: 'text', $content: content };
    // remove from deleted if it was there
    this._delta.deleted = this._delta.deleted.filter(p => p !== path);
  }

  delete(path) {
    path = this.resolve(path);
    delete this._delta.files[path];
    if (!this._delta.deleted.includes(path)) {
      this._delta.deleted.push(path);
    }
  }

  // --- persistence ---
  getDelta()          { return this._delta; }
  setDelta(delta)     { this._delta = delta; }

  saveDelta(key = 'loft-ide-delta') {
    localStorage.setItem(key, JSON.stringify(this._delta));
  }

  static loadDelta(key = 'loft-ide-delta') {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : null;
  }

  // --- reset: discard all user changes ---
  resetToBase() {
    this._delta = { files: {}, deleted: [], dirs: [] };
  }

  // --- check if a file has been modified from base ---
  isModified(path) {
    path = this.resolve(path);
    return path in this._delta.files;
  }

  isDeleted(path) {
    return this._delta.deleted.includes(this.resolve(path));
  }

  // --- list all user-modified paths ---
  modifiedPaths() {
    return Object.keys(this._delta.files);
  }
}
```

### Size budget

| Component | Estimated size |
|---|---|
| `base-fs.json` (gzipped) | ~50-100 KB (loft examples + docs) |
| Delta in localStorage | Typically < 50 KB (just edited files) |
| localStorage limit | 5-10 MB — plenty for delta |
| IndexedDB fallback | Needed only if binary data or > 100 files |

Since the base tree is static and cacheable, the service worker (M6) caches it
alongside the WASM module. The delta is tiny and persists in localStorage — no
IndexedDB needed for typical usage.

### Build script — `ide/scripts/build-base-fs.js`

```js
// Reads tests/docs/*.loft, doc/*.html, default/*.loft
// Outputs ide/assets/base-fs.json
import { readFileSync, readdirSync, writeFileSync } from 'fs';

const tree = { "/": { examples: {}, docs: {}, lib: {} } };

// examples from tests/docs/
for (const f of readdirSync('tests/docs').filter(f => f.endsWith('.loft'))) {
  tree["/"].examples[f] = {
    $type: 'text',
    $content: readFileSync(`tests/docs/${f}`, 'utf8')
  };
}

// generated HTML docs
for (const f of readdirSync('doc').filter(f => f.endsWith('.html'))) {
  tree["/"].docs[f] = {
    $type: 'text',
    $content: readFileSync(`doc/${f}`, 'utf8')
  };
}

// default stdlib
for (const f of readdirSync('default').filter(f => f.endsWith('.loft'))) {
  tree["/"].lib[f] = {
    $type: 'text',
    $content: readFileSync(`default/${f}`, 'utf8')
  };
}

writeFileSync('ide/assets/base-fs.json', JSON.stringify(tree));
```

### Browser startup flow

```js
// app.js — startup
const baseTree = await fetch('assets/base-fs.json').then(r => r.json());
const delta = LayeredFS.loadDelta();  // from localStorage, may be null
const fs = new LayeredFS(baseTree, delta);

// Wire the host
globalThis.loftHost = createBrowserHost(fs);

// Auto-save delta on every write (debounced)
let saveTimer;
const originalWrite = fs.writeText.bind(fs);
fs.writeText = (path, content) => {
  originalWrite(path, content);
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => fs.saveDelta(), 2000);
};
```

### "Reset to default" button

```js
resetButton.onclick = () => {
  if (confirm('Discard all changes and restore examples to original?')) {
    fs.resetToBase();
    fs.saveDelta();    // clears localStorage
    reloadEditor();
  }
};
```

### Node.js testing with layers

```js
import { LayeredFS } from './layered-fs.mjs';

test('user edit shadows base file', () => {
  const base = { "/": { "examples": {
    "hello.loft": { "$type": "text", "$content": "fn main() { println(\"hi\") }" }
  }}};
  const fs = new LayeredFS(base);

  // unmodified — reads from base
  assert(fs.readText('/examples/hello.loft').includes('hi'));
  assert(!fs.isModified('/examples/hello.loft'));

  // user edits — goes to delta
  fs.writeText('/examples/hello.loft', 'fn main() { println("bye") }');
  assert(fs.readText('/examples/hello.loft').includes('bye'));
  assert(fs.isModified('/examples/hello.loft'));

  // delta is small
  const delta = fs.getDelta();
  assert(Object.keys(delta.files).length === 1);

  // reset brings back original
  fs.resetToBase();
  assert(fs.readText('/examples/hello.loft').includes('hi'));
});

test('delete base file is tracked in delta', () => {
  const base = { "/": { "examples": {
    "a.loft": { "$type": "text", "$content": "fn a() {}" },
    "b.loft": { "$type": "text", "$content": "fn b() {}" }
  }}};
  const fs = new LayeredFS(base);

  fs.delete('/examples/a.loft');
  assert(!fs.exists('/examples/a.loft'));
  assert(fs.exists('/examples/b.loft'));    // unaffected
  assert.deepEqual(fs.readdir('/examples'), ['b.loft']);

  // delta only stores the deletion marker, not a copy of b.loft
  const delta = fs.getDelta();
  assert(delta.deleted.includes('/examples/a.loft'));
  assert(Object.keys(delta.files).length === 0);
});

test('new user file coexists with base', () => {
  const base = { "/": { "examples": {
    "hello.loft": { "$type": "text", "$content": "fn main() {}" }
  }}};
  const fs = new LayeredFS(base);

  fs.writeText('/my-project/main.loft', 'fn main() { println("mine") }');
  assert(fs.exists('/my-project/main.loft'));
  assert(fs.exists('/examples/hello.loft'));  // base still visible
  assert.deepEqual(fs.readdir('/').sort(), ['examples', 'my-project']);
});

test('delta serialise and reload', () => {
  const base = { "/": { "examples": {
    "a.loft": { "$type": "text", "$content": "original" }
  }}};
  const fs = new LayeredFS(base);
  fs.writeText('/examples/a.loft', 'modified');
  fs.writeText('/new.loft', 'brand new');

  // simulate save/reload cycle
  const deltaJson = JSON.stringify(fs.getDelta());
  const fs2 = new LayeredFS(base, JSON.parse(deltaJson));

  assert(fs2.readText('/examples/a.loft') === 'modified');
  assert(fs2.readText('/new.loft') === 'brand new');
});
```

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

See [Logging in WASM](#logging-in-wasm) for the full design.

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

## Node.js Test Harness

### Architecture

```
tests/wasm/
├── harness.mjs            — test runner, VirtFS, host setup
├── virt-fs.mjs            — VirtFS class implementation
├── virt-fs.test.mjs       — unit tests for VirtFS itself
├── host.mjs               — loftHost factory for Node.js
├── bridge.test.mjs        — WASM bridge integration tests
├── file-io.test.mjs       — loft file I/O through the virtual FS
├── random.test.mjs        — rand / rand_seed determinism
└── fixtures/
    ├── hello.json          — minimal VirtFS tree
    ├── multi-file.json     — multi-file project tree
    └── binary-data.json    — tree with binary file entries
```

### Running

```sh
# Build WASM for Node.js target
wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm

# Run all WASM tests
node --experimental-vm-modules tests/wasm/harness.mjs

# Run a single test file
node --experimental-vm-modules tests/wasm/virt-fs.test.mjs
```

### Host factory (`host.mjs`)

Creates a `loftHost` object wired to a `VirtFS` instance:

```js
import { VirtFS } from './virt-fs.mjs';

export function createHost(tree, options = {}) {
  const fs = new VirtFS(tree);

  // deterministic PRNG (xoshiro128**)
  let rng_state = [1, 2, 3, 4];
  function nextRandom() { /* xoshiro128** body */ }

  const storage = new Map();

  const host = {
    // filesystem — delegates to VirtFS
    fs_exists:       (p) => fs.exists(p),
    fs_read_text:    (p) => fs.readText(p),
    fs_read_binary:  (p, o, n) => fs.readBinary(p)?.slice(o, o + n) ?? null,
    fs_write_text:   (p, c) => { try { fs.writeText(p, c); return 0; } catch { return 5; } },
    fs_write_binary: (p, b) => { try { fs.writeBinary(p, b); return 0; } catch { return 5; } },
    fs_file_size:    (p) => fs.stat(p)?.size ?? -1,
    fs_delete:       (p) => { try { fs.delete(p); return 0; } catch { return 1; } },
    fs_move:         (f, t) => { try { fs.move(f, t); return 0; } catch { return 5; } },
    fs_mkdir:        (p) => { try { fs.mkdir(p); return 0; } catch { return 5; } },
    fs_mkdir_all:    (p) => { try { fs.mkdirAll(p); return 0; } catch { return 5; } },
    fs_list_dir:     (p) => fs.readdir(p) ?? [],
    fs_is_dir:       (p) => fs.isDirectory(p),
    fs_is_file:      (p) => fs.isFile(p),
    fs_seek:         (p, pos) => fs.seek(p, pos),
    fs_read_bytes:   (p, n) => fs.readBytes(p, n),
    fs_write_bytes:  (p, b) => { try { fs.writeBytes(p, b); return 0; } catch { return 5; } },
    fs_get_cursor:   (p) => fs.getCursor(p),
    fs_cwd:          () => fs.cwd,
    fs_user_dir:     () => '/home/test',
    fs_program_dir:  () => '/usr/local/bin',

    // random — deterministic by default
    random_int: (lo, hi) => lo + (nextRandom() % (hi - lo + 1)),
    random_seed: (hi, lo) => { rng_state = [lo, hi, lo ^ hi, lo + hi]; },

    // time
    time_now:   () => options.fakeTime ?? Date.now(),
    time_ticks: () => options.fakeTicks ?? 0,

    // environment
    env_variable: (name) => options.env?.[name] ?? null,

    // arguments
    arguments: () => options.args ?? [],

    // logging — goes to console
    log_write: (level, msg) => {
      const fn_ = level === 'fatal' ? 'error' : level;
      console[fn_](`[loft] ${msg}`);
    },

    // storage
    storage_get:    (k) => storage.get(k) ?? null,
    storage_set:    (k, v) => storage.set(k, v),
    storage_remove: (k) => storage.delete(k),
  };

  return { host, fs, storage };
}
```

### VirtFS unit tests (`virt-fs.test.mjs`)

```js
import { test, assert } from './harness.mjs';
import { VirtFS } from './virt-fs.mjs';

test('empty filesystem has root', () => {
  const fs = new VirtFS();
  assert(fs.isDirectory('/'));
  assert.deepEqual(fs.readdir('/'), []);
});

test('writeText creates file and parent dirs', () => {
  const fs = new VirtFS();
  fs.writeText('/a/b/c.txt', 'hello');
  assert(fs.isDirectory('/a'));
  assert(fs.isDirectory('/a/b'));
  assert(fs.isFile('/a/b/c.txt'));
  assert(fs.readText('/a/b/c.txt') === 'hello');
});

test('readdir lists entries', () => {
  const fs = new VirtFS({
    "/": {
      "x.txt": { "$type": "text", "$content": "x" },
      "y.txt": { "$type": "text", "$content": "y" },
      "sub": {}
    }
  });
  const entries = fs.readdir('/').sort();
  assert.deepEqual(entries, ['sub', 'x.txt', 'y.txt']);
});

test('delete removes file', () => {
  const fs = new VirtFS();
  fs.writeText('/f.txt', 'data');
  fs.delete('/f.txt');
  assert(!fs.exists('/f.txt'));
});

test('move renames file', () => {
  const fs = new VirtFS();
  fs.writeText('/old.txt', 'content');
  fs.move('/old.txt', '/new.txt');
  assert(!fs.exists('/old.txt'));
  assert(fs.readText('/new.txt') === 'content');
});

test('mkdir fails without parent', () => {
  const fs = new VirtFS();
  assert.throws(() => fs.mkdir('/a/b/c'));
});

test('mkdirAll creates full path', () => {
  const fs = new VirtFS();
  fs.mkdirAll('/a/b/c');
  assert(fs.isDirectory('/a/b/c'));
});

test('stat returns size for text files', () => {
  const fs = new VirtFS();
  fs.writeText('/f.txt', 'hello');     // 5 bytes UTF-8
  assert(fs.stat('/f.txt').size === 5);
  assert(fs.stat('/f.txt').type === 'text');
});

test('binary roundtrip', () => {
  const fs = new VirtFS();
  const data = new Uint8Array([1, 2, 3, 255]);
  fs.writeBinary('/b.bin', data);
  const out = fs.readBinary('/b.bin');
  assert.deepEqual(out, data);
  assert(fs.stat('/b.bin').type === 'binary');
});

test('binary cursor seek and read', () => {
  const fs = new VirtFS();
  fs.writeBinary('/b.bin', new Uint8Array([10, 20, 30, 40, 50]));
  fs.seek('/b.bin', 2);
  const chunk = fs.readBytes('/b.bin', 2);
  assert.deepEqual(chunk, new Uint8Array([30, 40]));
  assert(fs.getCursor('/b.bin') === 4);
});

test('snapshot and restore isolate mutations', () => {
  const fs = new VirtFS();
  fs.writeText('/keep.txt', 'original');
  const snap = fs.snapshot();

  fs.writeText('/keep.txt', 'modified');
  fs.writeText('/extra.txt', 'leaked');

  fs.restore(snap);
  assert(fs.readText('/keep.txt') === 'original');
  assert(!fs.exists('/extra.txt'));
});

test('resolve normalises paths', () => {
  const fs = new VirtFS();
  assert(fs.resolve('/a/b/../c/./d') === '/a/c/d');
  assert(fs.resolve('/a//b') === '/a/b');
  assert(fs.resolve('/a/b/') === '/a/b');
});

test('toJSON and fromJSON roundtrip', () => {
  const fs = new VirtFS();
  fs.writeText('/a.txt', 'hello');
  fs.mkdirAll('/sub/dir');
  fs.writeBinary('/sub/b.bin', new Uint8Array([42]));

  const json = JSON.stringify(fs.toJSON());
  const fs2 = VirtFS.fromJSON(json);

  assert(fs2.readText('/a.txt') === 'hello');
  assert(fs2.isDirectory('/sub/dir'));
  assert.deepEqual(fs2.readBinary('/sub/b.bin'), new Uint8Array([42]));
});
```

### WASM bridge integration tests (`bridge.test.mjs`)

These load the actual WASM module with a VirtFS-backed host:

```js
import { test, assert } from './harness.mjs';
import { createHost } from './host.mjs';
import { initWasm, compileAndRun } from './pkg/loft_wasm.js';

const tree = {
  "/": {
    "project": {
      "main.loft": { "$type": "text", "$content": "" }  // overwritten per test
    }
  }
};

let host, fs;

function run(code) {
  ({ host, fs } = createHost(structuredClone(tree)));
  globalThis.loftHost = host;
  fs.writeText('/project/main.loft', code);
  return compileAndRun([{ name: 'main.loft', content: code }]);
}

test('file write and read back', () => {
  const r = run(`
    fn main() {
      f = file("/project/out.txt")
      f.write("hello world")
      g = file("/project/out.txt")
      println(g.content())
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'hello world');
  assert(fs.readText('/project/out.txt') === 'hello world');
});

test('exists and delete', () => {
  const r = run(`
    fn main() {
      f = file("/project/tmp.txt")
      f.write("x")
      println(exists("/project/tmp.txt"))
      delete("/project/tmp.txt")
      println(exists("/project/tmp.txt"))
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'true\nfalse');
});

test('directory listing', () => {
  ({ host, fs } = createHost(structuredClone(tree)));
  globalThis.loftHost = host;
  fs.writeText('/project/a.loft', 'fn a() {}');
  fs.writeText('/project/b.loft', 'fn b() {}');

  const r = compileAndRun([{
    name: 'main.loft',
    content: `
      fn main() {
        d = file("/project")
        for f in d.files() { println(f.path) }
      }
    `
  }]);
  assert(r.success);
  assert(r.output.includes('a.loft'));
  assert(r.output.includes('b.loft'));
});

test('rand with seed is deterministic', () => {
  const r1 = run(`
    fn main() {
      rand_seed(42)
      println(rand(1, 1000))
      println(rand(1, 1000))
    }
  `);
  const r2 = run(`
    fn main() {
      rand_seed(42)
      println(rand(1, 1000))
      println(rand(1, 1000))
    }
  `);
  assert(r1.success && r2.success);
  assert(r1.output === r2.output);
});

test('mkdir_all and nested write', () => {
  const r = run(`
    fn main() {
      mkdir_all("/project/a/b/c")
      f = file("/project/a/b/c/deep.txt")
      f.write("nested")
      println(file("/project/a/b/c/deep.txt").content())
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'nested');
});

test('binary write and read', () => {
  const r = run(`
    fn main() {
      f = file("/project/data.bin")
      f.little_endian()
      f += 42
      f += 256
      g = file("/project/data.bin")
      g.little_endian()
      a = g#read(4) as integer
      b = g#read(4) as integer
      println(a)
      println(b)
    }
  `);
  assert(r.success);
  assert(r.output.trim() === '42\n256');
});
```

### File I/O edge case tests (`file-io.test.mjs`)

```js
test('read nonexistent file returns null content', () => { /* ... */ });
test('write to nested path auto-creates dirs', () => { /* ... */ });
test('delete nonexistent returns NotFound code', () => { /* ... */ });
test('move across directories', () => { /* ... */ });
test('overwrite existing file', () => { /* ... */ });
test('f#size reflects write', () => { /* ... */ });
test('seek beyond end pads with zeros', () => { /* ... */ });
test('binary cursor resets on write', () => { /* ... */ });
```

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

## Threading in WASM — Two-Tier Design

> **⚠ Superseded implementation (@PLN117).** The hand-rolled Web-Worker dispatch
> described below (`worker_entry`, a manual `SharedArrayBuffer` control buffer,
> `LoftThreadPool`) was **never implemented** (`worker_entry` was a no-op stub) and
> has been **retired**. Browser `par()` now rides **loft's own rayon pool**
> (`src/wasm_threads.rs` + `doc/loft-thread.js`) — the same rayon scheduler the
> native backend uses — via `src/parallel.rs::with_pool`. rayon schedules; loft
> only supplies the threads, because a browser has no `thread::spawn`. The pool is
> plain wasm exports with no host imports, which is why the SAME runtime serves
> the wasm-bindgen gallery bundle and the raw `loft --html` one. Build with
> `make wasm-mt`; the COOP/COEP
> contract, the two-tier (isolated → threaded, else → sequential) fallback, and the
> runtime detection idea below all still hold. The current, proven picture is
> **§ Cargo Feature Gates → Build commands** above and
> `doc/claude/plans/117-browser-multithreading/`. The text below is kept for the
> two-tier design rationale only.

### Overview

Loft's `par()` loop attribute spawns OS threads with shared mutable access to the
Store heap. Browsers can replicate this using **Web Workers + SharedArrayBuffer**,
but only when specific HTTP headers are present. The design uses two tiers:

```
┌─────────────────────────────────────────────────────────────┐
│                    Runtime detection                         │
│                                                             │
│  if (crossOriginIsolated && SharedArrayBuffer) {            │
│      → Tier 2: Web Worker thread pool (real parallelism)    │
│  } else {                                                   │
│      → Tier 1: Sequential fallback (same results, no par)   │
│  }                                                          │
└─────────────────────────────────────────────────────────────┘
```

| Tier | Environment | `par()` behaviour | Build |
|---|---|---|---|
| **Tier 1** | `file://`, simple static hosts, Node.js | Sequential — loop body runs inline | `--features wasm` |
| **Tier 2** | Hosts with COOP/COEP headers, CDN | Real parallelism via Web Workers | `--features wasm-threads` |

### Tier 1 — Sequential fallback

When the `threading` feature is disabled, all parallel entry points execute the loop
body sequentially in the main thread. The loft program behaves identically — same
results, same side effects — just without parallelism.

**Rust changes in `src/parallel.rs`:**

```rust
// The public entry points used by fill.rs:

#[cfg(feature = "threading")]
pub fn run_parallel_int(/* ... */) { /* existing thread-pool or Web Worker impl */ }

#[cfg(not(feature = "threading"))]
pub fn run_parallel_int(
    state: &mut State,
    stores: &mut Stores,
    start: i32,
    end: i32,
    body_fn: usize,
) {
    // Sequential fallback: just run the loop body inline
    for i in start..end {
        state.push_int(i);
        state.call(body_fn, stores);
    }
}

// Same pattern for run_parallel_raw, run_parallel_text, etc.
```

**Bytecode generation (`src/state/codegen.rs`):**

No changes needed. `gen_for` already emits `OpParallelFor*` opcodes regardless of
target. The opcodes dispatch through `src/fill.rs` to the `parallel.rs` entry points,
which are swapped at compile time.

**Loft-side behaviour:**

- `par(threads: 4)` attribute is parsed and accepted but ignored — the loop runs
  sequentially.
- `fn <name>` references (used for parallel worker functions) still work — they are
  called inline.
- No runtime error or warning is emitted. Programs that use `par()` remain valid.

### Tier 2 — Web Worker parallelism

When `SharedArrayBuffer` is available, loft can use real parallel execution in the
browser. The WASM linear memory (which contains the Store heap) becomes a
`SharedArrayBuffer` that multiple Web Workers share.

**Why this maps directly to loft's `par()` model:**

| loft native (src/parallel.rs) | WASM + Web Workers |
|---|---|
| `std::thread::spawn(closure)` | Web Worker on shared WASM memory |
| `unsafe { copy_nonoverlapping }` into Store | Same pointers — Store is in shared linear memory |
| `mpsc::channel` for results | `Atomics.wait()` / `Atomics.notify()` |
| `thread_local! { RNG }` | Per-Worker TLS (each Worker gets its own) |
| `thread::available_parallelism()` | `navigator.hardwareConcurrency` |

**Required HTTP headers (server-side):**

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Without these headers, the browser blocks `SharedArrayBuffer` construction (Spectre
mitigation). This is why Tier 2 cannot work from `file://` URLs.

**Rust implementation** (as shipped — @PLN117; the sketch this section originally
carried used `wasm-bindgen-rayon`, which loft no longer depends on):

```rust
// src/wasm_threads.rs — under #[cfg(feature = "wasm-native-threads")]

// The page spawns the Web Workers and sequences start-up; rayon schedules.
loft_pool_new(n) -> handoff    // create the handoff, mark this thread non-blocking
loft_rayon_start_worker(h)     // each worker: claim one rayon thread and run it
loft_pool_build()              // install the global pool once the workers are up
```

Plain wasm exports with no host imports, driven from `doc/loft-thread.js`.  That is
what lets ONE pool serve both the wasm-bindgen gallery bundle and the raw
`loft --html` one, whose glue builds its own imports and would reject an extra
module.  `std::thread::spawn` does NOT become a Web Worker under the atomics target
feature — it is unsupported, which is exactly why the threads come from the host.

Alternatively, if loft's parallel model doesn't fit rayon's work-stealing pattern,
the worker pool can be managed directly:

```rust
// Direct Web Worker management via wasm-bindgen + js-sys
#[cfg(all(feature = "threading", feature = "wasm-threads"))]
mod wasm_workers {
    use js_sys::SharedArrayBuffer;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        type Worker;

        #[wasm_bindgen(constructor)]
        fn new(url: &str) -> Worker;

        #[wasm_bindgen(method, js_name = postMessage)]
        fn post_message(this: &Worker, msg: &JsValue);
    }

    /// Each worker receives:
    /// - The SharedArrayBuffer (WASM linear memory)
    /// - The function index to execute
    /// - The iteration range [start, end)
    /// - The base pointer into Store for writing results
    pub fn spawn_workers(
        worker_count: usize,
        fn_index: usize,
        start: i32,
        end: i32,
        store_base: *mut u8,
    ) {
        let chunk_size = (end - start) / worker_count as i32;
        // ... distribute work, wait for completion via Atomics
    }
}
```

**Worker script (`ide/src/loft-worker.js`):**

```js
// Loaded by each Web Worker
import init, { worker_entry } from '../pkg/loft_wasm.js';

self.onmessage = async ({ data }) => {
  if (data.type === 'init') {
    // Initialise WASM with shared memory
    await init(data.module, data.memory);
    self.postMessage({ type: 'ready' });
  } else if (data.type === 'run') {
    // Execute the parallel loop chunk
    worker_entry(data.fn_index, data.start, data.end);
    // Signal completion via Atomics (no postMessage needed for result —
    // results are written directly to shared Store memory)
    Atomics.store(data.signal, data.worker_id, 1);
    Atomics.notify(data.signal, data.worker_id);
  }
};
```

**Main thread coordination:**

```js
// wasm-bridge.js
async function initWasmThreaded(wasmUrl, workerCount) {
  // Compile module (shared across workers)
  const module = await WebAssembly.compileStreaming(fetch(wasmUrl));

  // Create shared memory (this is the WASM linear memory)
  const memory = new WebAssembly.Memory({
    initial: 256,       // 16 MB
    maximum: 16384,     // 1 GB
    shared: true        // ← SharedArrayBuffer under the hood
  });

  // Spawn worker pool
  const workers = [];
  for (let i = 0; i < workerCount; i++) {
    const w = new Worker('src/loft-worker.js', { type: 'module' });
    w.postMessage({ type: 'init', module, memory });
    workers.push(w);
  }

  // Wait for all workers to be ready
  await Promise.all(workers.map(w =>
    new Promise(resolve => {
      w.onmessage = (e) => { if (e.data.type === 'ready') resolve(); };
    })
  ));

  // Init main thread WASM with same shared memory
  await init(module, memory);

  return { workers, memory };
}
```

**Synchronisation pattern for `par()` loops:**

```
Main thread                          Workers
    │                                   │
    ├─ Prepare iteration ranges         │
    ├─ Write ranges to shared memory    │
    ├─ Atomics.store(signal, 0)  ──────→│ Workers wake, read range
    ├─ Atomics.wait(done, ...)          │ Execute loop body
    │                                   │ Write results to Store (shared memory)
    │                              ←────│ Atomics.store(done, 1)
    ├─ All workers done                 │
    ├─ Continue execution               │
    │                                   │
```

Results don't need to be "sent back" — they're written directly to the Store heap
in shared memory, exactly like the native `copy_nonoverlapping` pattern.

### Runtime detection and WASM loading

```js
// app.js — choose tier at startup
export async function initLoft() {
  const threaded = typeof SharedArrayBuffer !== 'undefined'
                && crossOriginIsolated;

  if (threaded) {
    console.info('[loft] Tier 2: parallel execution via Web Workers');
    const cores = navigator.hardwareConcurrency || 4;
    const { workers, memory } = await initWasmThreaded(
      'pkg/loft_wasm_bg.wasm', cores
    );
    return { threaded: true, workers, workerCount: cores };
  } else {
    console.info('[loft] Tier 1: sequential execution (no SharedArrayBuffer)');
    await initWasm('pkg/loft_wasm_bg.wasm');
    return { threaded: false, workerCount: 0 };
  }
}
```

**IDE status indicator:**

```js
// Show the user which tier is active
const badge = document.getElementById('threading-badge');
if (loftEnv.threaded) {
  badge.textContent = `⚡ ${loftEnv.workerCount} threads`;
  badge.title = 'Parallel execution enabled (SharedArrayBuffer)';
} else {
  badge.textContent = '1 thread';
  badge.title = 'Sequential mode — serve with COEP/COOP headers for parallelism';
}
```

### Deployment configurations

| Hosting method | Headers | Tier | `par()` |
|---|---|---|---|
| `file://index.html` | None | 1 | Sequential |
| `python -m http.server` | None | 1 | Sequential |
| Nginx / Apache with COOP+COEP | Yes | 2 | Parallel |
| Cloudflare Pages (custom headers) | Yes | 2 | Parallel |
| GitHub Pages | No (can't set headers) | 1 | Sequential |
| Vercel / Netlify (with `_headers`) | Yes | 2 | Parallel |

**Minimal dev server with headers** (`ide/serve.mjs`):

```js
import http from 'http';
import { readFileSync, existsSync } from 'fs';

const MIME = { '.html': 'text/html', '.js': 'text/javascript',
               '.wasm': 'application/wasm', '.css': 'text/css',
               '.json': 'application/json' };

http.createServer((req, res) => {
  const path = `ide${req.url === '/' ? '/index.html' : req.url}`;
  if (!existsSync(path)) { res.writeHead(404).end(); return; }

  const ext = path.substring(path.lastIndexOf('.'));
  res.writeHead(200, {
    'Content-Type': MIME[ext] || 'application/octet-stream',
    'Cross-Origin-Opener-Policy': 'same-origin',
    'Cross-Origin-Embedder-Policy': 'require-corp',
  });
  res.end(readFileSync(path));
}).listen(8080);

console.log('Loft IDE: http://localhost:8080 (threads enabled)');
```

### Build pipeline

```sh
# ide/build.sh — updated to produce both tiers

#!/bin/sh
set -e

echo "Building single-threaded WASM..."
wasm-pack build --target web --out-dir ide/pkg/st -- --features wasm --no-default-features

echo "Building multi-threaded WASM..."
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  wasm-pack build --target web --out-dir ide/pkg/mt -- --features wasm-threads --no-default-features

echo "Building base filesystem..."
node ide/scripts/build-base-fs.js

echo "Done. Serve with: node ide/serve.mjs"
```

The loader picks the right package at runtime:

```js
const pkg = loftEnv.threaded ? 'pkg/mt/loft_wasm.js' : 'pkg/st/loft_wasm.js';
```

### Memory considerations for shared WASM

- `WebAssembly.Memory({ shared: true })` requires a fixed `maximum` declared at
  compile time. For loft this should be generous (1 GB) since the Store heap can
  grow.
- Shared memory cannot be grown dynamically in some browsers — set initial size
  large enough for typical programs (~64 MB) to avoid frequent grows.
- Each Web Worker loads the full WASM module but shares the same linear memory.
  Per-worker overhead is ~1-2 MB for the WASM instance plus JS context.

### Test impact

| Test file | Tier 1 | Tier 2 |
|---|---|---|
| `tests/threading.rs` | **Skip** — tests Rust `std::thread` directly | **Skip** — same reason |
| `tests/scripts/22-threading.loft` | Runs (sequential) | Runs (parallel via Workers) |
| All other tests | Pass | Pass |

The `22-threading.loft` test verifies output correctness (sums, vector contents), not
execution speed or thread count. It passes under both tiers without changes.

---

## PNG Image Support in WASM

### Why it works

The `png` crate (v0.17) is pure Rust with no OS dependencies. It implements the full
PNG decode pipeline (inflate, filtering, interlacing) in safe Rust. This compiles to
WASM without modification.

### Adaptation: buffer-based decoding

The only blocker is that `png_store::read()` opens a file via `std::fs::File::open()`.
Under `#[cfg(feature = "wasm")]`, it reads from the VirtFS instead:

```rust
// src/png_store.rs

#[cfg(not(feature = "wasm"))]
pub fn read(path: &str, store: &mut Store) -> Result<(u32, u32, u32)> {
    let file = std::fs::File::open(path)?;
    let decoder = png::Decoder::new(file);
    decode_into_store(decoder, store)
}

#[cfg(feature = "wasm")]
pub fn read(path: &str, store: &mut Store) -> Result<(u32, u32, u32)> {
    let bytes = crate::wasm::host_read_binary(path)
        .ok_or_else(|| anyhow!("file not found: {path}"))?;
    let cursor = std::io::Cursor::new(bytes);
    let decoder = png::Decoder::new(cursor);
    decode_into_store(decoder, store)
}

// Shared decode logic — works with any std::io::Read source
fn decode_into_store<R: std::io::Read>(
    decoder: png::Decoder<R>,
    store: &mut Store,
) -> Result<(u32, u32, u32)> {
    let img = store.claim((reader.output_buffer_size() / 8) as u32 + 1);
    let info = reader.next_frame(store.buffer(img))?;
    Ok((img, info.width, info.height))
}
```

The key insight: `png::Decoder<R>` is generic over `R: Read`. Swapping
`File` for `Cursor<Vec<u8>>` requires no changes to the decode logic.

### Browser workflow

1. User drags a PNG file onto the IDE, or the PNG is in the VirtFS (base tree or
   delta).
2. The VirtFS stores it as a binary node (`"$type": "binary"`, base64 content).
3. When the loft program calls `file("image.png").png()`, the WASM bridge reads the
   binary bytes from VirtFS and passes them to `png::Decoder` via a `Cursor`.
4. Decoded pixels land in the Store heap as usual — the `Image` struct works
   identically.

### Browser-to-loft PNG import via drag-and-drop

```js
// In the IDE: handle dropped PNG files
dropZone.ondrop = async (e) => {
  for (const file of e.dataTransfer.files) {
    if (file.name.endsWith('.png')) {
      const bytes = new Uint8Array(await file.arrayBuffer());
      fs.writeBinary(`/project/${file.name}`, bytes);
    }
  }
};
```

### Displaying Image output in the browser

The reverse direction — loft `Image` → visible in the IDE — can be handled by a
host bridge that receives pixel data and renders to a `<canvas>`:

```js
globalThis.loftHost = {
  // ...
  display_image(width, height, pixels) {
    // pixels: Uint8Array of RGB triplets
    const canvas = document.getElementById('output-canvas');
    const ctx = canvas.getContext('2d');
    canvas.width = width;
    canvas.height = height;
    const imageData = ctx.createImageData(width, height);
    for (let i = 0, j = 0; i < pixels.length; i += 3, j += 4) {
      imageData.data[j]     = pixels[i];     // R
      imageData.data[j + 1] = pixels[i + 1]; // G
      imageData.data[j + 2] = pixels[i + 2]; // B
      imageData.data[j + 3] = 255;           // A
    }
    ctx.putImageData(imageData, 0, 0);
  }
};
```

This is optional — PNG decoding works without it. Display is an IDE convenience.

---

## Logging in WASM

### Problem

The loft logger (`src/logger.rs`) writes to files: it creates log directories,
rotates log files, and archives old entries. None of this makes sense in a browser.

### Design: console-only logging under `#[cfg(feature = "wasm")]`

All log output goes to the JavaScript console via the host bridge. No file I/O, no
rotation, no directories.

**Rust changes in `src/logger.rs`:**

```rust
#[cfg(feature = "wasm")]
fn write_log_entry(level: Level, message: &str) {
    // Call JS host to write to console
    crate::wasm::host_log_write(level.as_str(), message);
}

#[cfg(not(feature = "wasm"))]
fn write_log_entry(level: Level, message: &str) {
    // Existing file-based logging implementation
    // ...
}
```

**Conditional compilation gates:**

```rust
// Skip all file-based setup in WASM
#[cfg(not(feature = "wasm"))]
fn ensure_log_dir() { /* create directory, rotate files */ }

#[cfg(feature = "wasm")]
fn ensure_log_dir() { /* no-op */ }
```

**JS host implementation:**

```js
// Browser
globalThis.loftHost = {
  log_write(level, message) {
    switch (level) {
      case 'info':  console.info(`[loft] ${message}`);  break;
      case 'warn':  console.warn(`[loft] ${message}`);  break;
      case 'error': console.error(`[loft] ${message}`); break;
      case 'fatal': console.error(`[loft FATAL] ${message}`); break;
    }
  }
};

// Node.js — identical, console methods work the same
```

**Loft-side behaviour:**

- `log_info()`, `log_warn()`, `log_error()`, `log_fatal()` all work.
- `log_config()` is accepted but has no effect (no file to configure).
- Rate limiting still applies — implemented in Rust, not in the file layer.

### IDE integration (optional)

The IDE can capture log output in a dedicated "Log" panel instead of (or alongside)
the browser console:

```js
const logEntries = [];
globalThis.loftHost = {
  log_write(level, message) {
    logEntries.push({ level, message, time: Date.now() });
    renderLogPanel(logEntries);         // update UI
    console[level === 'fatal' ? 'error' : level](`[loft] ${message}`);
  }
};
```

---

## Test Compatibility Matrix

### Rust integration tests (`tests/*.rs`)

| Test file | Tier 1 (sequential) | Tier 2 (threaded) | Notes |
|---|---|---|---|
| `expressions.rs` | Yes | Yes | Pure computation, no OS deps |
| `enums.rs` | Yes | Yes | Pure computation |
| `strings.rs` | Yes | Yes | Pure computation |
| `objects.rs` | Yes | Yes | Pure computation |
| `vectors.rs` | Yes | Yes | Pure computation |
| `sizes.rs` | Yes | Yes | Pure computation |
| `data_structures.rs` | Yes | Yes | Pure computation |
| `parse_errors.rs` | Yes | Yes | Diagnostic checking, no runtime |
| `immutability.rs` | Yes | Yes | Diagnostic checking |
| `slot_assign.rs` | Yes | Yes | Compile-time analysis |
| `log_config.rs` | Yes | Yes | Unit tests for config parsing |
| `issues.rs` | Yes | Yes | Reproducers — most are pure computation |
| `expressions_auto_convert.rs` | Yes | Yes | Pure computation |
| `threading.rs` | **Skip** | **Skip** | Tests Rust `std::thread` APIs directly — not WASM-portable |
| `wrap.rs` | Partial | Partial | Runs `.loft` files — needs VirtFS for file-IO tests |

### Loft script tests (`tests/scripts/*.loft`)

| Test file | Tier 1 | Tier 2 | Bridge needed |
|---|---|---|---|
| `01-*` through `14-*` | Yes | Yes | Output capture only |
| `15-random.loft` | Yes | Yes | `random_int`, `random_seed` |
| `16-time.loft` | Yes | Yes | `time_now`, `time_ticks` |
| `19-files.loft` | Yes | Yes | Full VirtFS bridge |
| `22-threading.loft` | Yes (sequential) | Yes (parallel) | Sequential fallback or Web Workers |
| `42-file-result.loft` | Yes | Yes | VirtFS + `fs_delete`, `fs_move`, `fs_mkdir` |

### Loft doc tests (`tests/docs/*.loft`)

| Test file | Tier 1 | Tier 2 | Bridge needed |
|---|---|---|---|
| `13-file.loft` | Yes | Yes | Full VirtFS bridge |
| `21-random.loft` | Yes | Yes | `random_int`, `random_seed` |
| `22-time.loft` | Yes | Yes | `time_now`, `time_ticks` |
| All others | Yes | Yes | Output capture only |

### Summary

| Category | Total | Tier 1 | Tier 2 | Skip | Notes |
|---|---|---|---|---|---|
| Rust integration tests | ~15 | 14 | 14 | 1 | Only `threading.rs` skipped (both tiers) |
| Loft script tests | ~40+ | All | All | 0 | `22-threading` sequential in T1, parallel in T2 |
| Loft doc tests | ~25+ | All | All | 0 | |

The **only test file that must be skipped** is `tests/threading.rs`, which tests
Rust-level `std::thread` APIs directly. Every loft-level test — including those
using `par()` — runs under both tiers. Tier 2 gives real parallelism; Tier 1
gives identical results sequentially.

---

## Implementation Plan

### Principles

- Each step produces a testable result before moving to the next.
- Steps are ordered so that earlier steps unblock later ones.
- Each step works with `cargo test` (native) and/or `wasm-pack build` + Node.js.
- No step requires the Web IDE — all testing is CLI/Node.js until the final
  integration step.

---

### Step 1 — Cargo feature scaffolding

**Goal:** `cargo build --features wasm --no-default-features` compiles (with stubs).

**Changes:**
- `Cargo.toml`: add `wasm`, `wasm-threads`, `threading` features and optional deps
  (`wasm-bindgen`, `serde`, `serde-wasm-bindgen`, `js-sys`, `web-sys`,
  `wasm-bindgen-rayon`)
- `src/lib.rs` (new): `#[cfg(feature = "wasm")] mod wasm;` — expose crate as a
  library target alongside the binary
- `src/wasm.rs` (new): empty module with `// TODO` placeholders
- Add `[lib]` section to `Cargo.toml`: `crate-type = ["cdylib", "rlib"]`

**Test:**
```sh
cargo check --features wasm --no-default-features
cargo check                                          # default features still work
cargo test                                           # existing tests unchanged
```

**Verification:** both check commands succeed with zero warnings from new code.

---

### Step 2 — Output capture (`print` → buffer)

**Goal:** `println()` / `print()` output goes to a thread-local buffer under `wasm`.

**Changes:**
- `src/wasm.rs`: add `output_push()`, `output_take()` with thread-local `String`
- `src/fill.rs` line 1725: wrap `print!()` in `#[cfg(not(feature = "wasm"))]`,
  add `#[cfg(feature = "wasm")]` branch calling `crate::wasm::output_push()`

**Test:**
```sh
# Native — no change
cargo test

# WASM — write a Rust unit test in src/wasm.rs:
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_capture() {
        output_push("hello ");
        output_push("world");
        assert_eq!(output_take(), "hello world");
        assert_eq!(output_take(), "");  // cleared after take
    }
}

cargo test --features wasm --no-default-features -- wasm::tests
```

**Verification:** unit test passes; native `cargo test` still passes.

---

### Step 3 — Sequential `par()` fallback

**Goal:** `par()` loops run sequentially when `threading` feature is off.

**Changes:**
- `src/parallel.rs`: wrap each `run_parallel_*` function body in
  `#[cfg(feature = "threading")]`; add `#[cfg(not(feature = "threading"))]`
  sequential versions
- `src/native.rs`: the call sites (`run_parallel_int(...)` etc.) stay unchanged —
  the function signatures are identical

**Test:**
```sh
# Native with threading — existing tests pass as before
cargo test

# Without threading — 22-threading.loft produces correct results sequentially
cargo test --no-default-features --features png,random -- scripts::threading
cargo test --no-default-features --features png,random -- docs
```

**Verification:** `22-threading.loft` output matches expected values.
`tests/threading.rs` is expected to fail (or be `#[cfg]`-gated) without the
`threading` feature — add `#![cfg(feature = "threading")]` at the top of that file.

---

### Step 4 — Logging to console stub

**Goal:** logger compiles under `wasm` without file I/O.

**Changes:**
- `src/logger.rs`: gate file I/O functions (`ensure_log_dir`, file write, rotate)
  behind `#[cfg(not(feature = "wasm"))]`
- Add `#[cfg(feature = "wasm")]` versions that call `crate::wasm::host_log_write()`
- `src/wasm.rs`: add `host_log_write()` — for now, a no-op or `web_sys::console::log_1`
  behind `#[cfg(feature = "wasm")]`

**Test:**
```sh
# Compile check
cargo check --features wasm --no-default-features

# Native logging unchanged
cargo test -- log_config
```

**Verification:** compiles cleanly; `log_config.rs` tests still pass on native.

---

### Step 5 — Random bridge stub

**Goal:** `rand()` / `rand_seed()` compile under `wasm` without the `rand_pcg` crate.

**Changes:**
- `src/ops.rs`: gate the `thread_local! { RNG }` and PCG usage behind
  `#[cfg(feature = "random")]`
- Add `#[cfg(all(feature = "wasm", not(feature = "random")))]` versions that call
  `crate::wasm::host_random_int()` and `crate::wasm::host_random_seed()`
- `src/wasm.rs`: declare `#[wasm_bindgen]` extern functions for `random_int`,
  `random_seed`; for Rust-side testing, add `#[cfg(test)]` mock implementations

**Test:**
```sh
# Native — rand tests still pass
cargo test -- scripts::random
cargo test -- docs::random

# WASM compile check
cargo check --features wasm --no-default-features
```

**Verification:** native random tests pass; WASM target compiles.

---

### Step 6 — Time and environment bridge stubs

**Goal:** `now()`, `ticks()`, `env_variable()`, `arguments()`, `directory()`,
`user_directory()`, `program_directory()` compile under `wasm`.

**Changes:**
- `src/native.rs` / `src/database/format.rs`: gate `SystemTime`, `Instant`,
  `std::env::*`, `dirs::home_dir()` behind `#[cfg(not(feature = "wasm"))]`
- Add `#[cfg(feature = "wasm")]` versions calling host bridge functions
- `src/wasm.rs`: declare extern functions for `time_now`, `time_ticks`,
  `env_variable`, `arguments`, `fs_cwd`, `fs_user_dir`, `fs_program_dir`

**Test:**
```sh
# Native — time/env tests still pass
cargo test -- scripts::time
cargo test -- scripts::files

# WASM compile check
cargo check --features wasm --no-default-features
```

**Verification:** native tests pass; WASM target compiles cleanly.

---

### Step 7 — File I/O bridge stubs

**Goal:** all file operations compile under `wasm`, calling host bridge functions
instead of `std::fs`.

**Changes:**
- `src/state/io.rs` and `src/database/io.rs`: gate every `std::fs` call behind
  `#[cfg(not(feature = "wasm"))]`; add `#[cfg(feature = "wasm")]` versions calling
  host functions (`fs_exists`, `fs_read_text`, `fs_write_text`, `fs_read_binary`,
  `fs_write_binary`, `fs_delete`, `fs_move`, `fs_mkdir`, `fs_mkdir_all`,
  `fs_list_dir`, `fs_is_dir`, `fs_is_file`, `fs_file_size`, `fs_seek`,
  `fs_read_bytes`, `fs_write_bytes`, `fs_get_cursor`)
- `src/wasm.rs`: declare all `#[wasm_bindgen]` extern functions

**Test:**
```sh
# Native — file tests still pass
cargo test -- scripts::files
cargo test -- scripts::file_result
cargo test -- docs::file

# WASM compile check
cargo check --features wasm --no-default-features
```

**Verification:** native file I/O tests pass; WASM target compiles. This is the
largest single step — review the diff carefully for missed `std::fs` calls.

---

### Step 8 — PNG buffer-based decoding

**Goal:** `file("img.png").png()` works under `wasm` by reading bytes from the host
instead of `std::fs::File`.

**Changes:**
- `src/png_store.rs`: extract shared logic into `decode_into_store<R: Read>()`; add
  `#[cfg(feature = "wasm")]` version that reads via `crate::wasm::host_read_binary()`
  and wraps in `std::io::Cursor`

**Test:**
```sh
# Native — any PNG-using tests still pass
cargo test

# WASM compile check (png feature is ON for wasm)
cargo check --features wasm --no-default-features
```

**Verification:** compiles; native PNG tests unchanged.

---

### Step 9 — `compile_and_run()` WASM entry point

**Goal:** the full interpreter is callable from JS via a single function.

**Changes:**
- `src/wasm.rs`: implement `compile_and_run(files_js: JsValue) -> JsValue` as
  described in [WEB_IDE.md](lib_plans/62-web-ide/README.md) § src/wasm.rs
- Wire up virtual FS population, parser, scope check, bytecode gen, execution,
  output collection, diagnostic collection
- Return `{ output, diagnostics, success }`

**Test:**
```sh
# WASM build (first time producing a .wasm file)
wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm --no-default-features

# Smoke test from Node.js
node -e "
  const loft = require('./tests/wasm/pkg/loft_wasm.js');
  const r = loft.compile_and_run([{name:'main.loft', content:'fn main(){println(\"hi\")}'}]);
  console.log(r);
  process.exit(r.success ? 0 : 1);
"
```

**Verification:** prints `{ output: 'hi\n', diagnostics: [], success: true }` and
exits 0. This is the first time loft actually runs in WASM.

---

### Step 10 — VirtFS implementation (JavaScript)

**Goal:** the `VirtFS` class passes all unit tests in Node.js.

**Changes:**
- `tests/wasm/virt-fs.mjs`: implement the `VirtFS` class
- `tests/wasm/harness.mjs`: minimal test runner (`test()`, `assert()`,
  `assert.deepEqual()`, `assert.throws()`)
- `tests/wasm/virt-fs.test.mjs`: all unit tests from the [VirtFS unit tests]
  section of this document

**Test:**
```sh
node tests/wasm/virt-fs.test.mjs
```

**Verification:** all VirtFS tests pass (exists, read, write, delete, move, mkdir,
binary, cursor, snapshot/restore, toJSON/fromJSON, path resolution).

---

### Step 11 — Host factory and WASM bridge tests

**Goal:** loft programs that do file I/O, random, and time work end-to-end in Node.js
via WASM.

**Changes:**
- `tests/wasm/host.mjs`: implement `createHost()` wiring VirtFS to `loftHost`
- `tests/wasm/bridge.test.mjs`: integration tests from the [WASM bridge integration
  tests] section (file write/read, exists/delete, directory listing, rand
  determinism, mkdir_all, binary I/O)
- `tests/wasm/file-io.test.mjs`: edge case tests
- `tests/wasm/random.test.mjs`: rand/seed determinism tests

**Test:**
```sh
node --experimental-vm-modules tests/wasm/bridge.test.mjs
node --experimental-vm-modules tests/wasm/random.test.mjs
node --experimental-vm-modules tests/wasm/file-io.test.mjs
```

**Verification:** all bridge tests pass — loft programs produce correct output,
VirtFS reflects writes made by loft code, rand sequences are reproducible.

---

### Step 12 — LayeredFS and base tree

**Goal:** the overlay filesystem works; base tree is generated from project files.

**Changes:**
- `tests/wasm/layered-fs.mjs`: implement `LayeredFS` extending `VirtFS`
- `tests/wasm/layered-fs.test.mjs`: tests from the [Node.js testing with layers]
  section (shadow, delete tracking, coexistence, delta serialise/reload)
- `ide/scripts/build-base-fs.js`: build script that generates `base-fs.json` from
  `tests/docs/*.loft`, `doc/*.html`, `default/*.loft`

**Test:**
```sh
node tests/wasm/layered-fs.test.mjs
node ide/scripts/build-base-fs.js && ls -la ide/assets/base-fs.json
```

**Verification:** LayeredFS tests pass; `base-fs.json` is generated and contains
expected entries.

---

### Step 13 — Run existing loft test suite through WASM

**Goal:** the bulk of `tests/scripts/*.loft` and `tests/docs/*.loft` produce correct
output when executed via the WASM module in Node.js.

**Changes:**
- `tests/wasm/suite.mjs`: test runner that:
  1. Reads each `.loft` file
  2. Creates a VirtFS with the file and any supporting fixtures
  3. Calls `compile_and_run()` via the WASM module
  4. Compares output against the expected output (from the native test framework's
     result files, or by running native first and capturing)
- Skip list: `22-threading.loft` runs but is not compared for timing output;
  file-I/O tests need fixture trees

**Test:**
```sh
# Generate reference output from native
cargo test 2>&1 | tee tests/wasm/reference-output.txt

# Run through WASM
node --experimental-vm-modules tests/wasm/suite.mjs
```

**Verification:** WASM output matches native output for all non-skipped tests.
This is the major confidence gate — if this passes, the WASM port is functionally
correct.

---

### Step 14 — Tier 2: Web Worker threading (optional)

**Goal:** `par()` loops use real Web Workers when SharedArrayBuffer is available.

**Changes:**
- `Cargo.toml`: `wasm-threads` feature adds the Web Worker pool (@PLN117: loft's
  own, `wasm-native-threads`; this plan predates it and said `wasm-bindgen-rayon`)
- `src/parallel.rs`: route the `par` dispatch over the page's global pool
- `ide/src/loft-worker.js`: worker script
- `ide/src/wasm-bridge.js`: `initWasmThreaded()` with shared memory and worker pool
- `ide/serve.mjs`: dev server with COOP/COEP headers

**Test:**
```sh
# Build threaded WASM
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  wasm-pack build --target web --out-dir ide/pkg/mt -- --features wasm-threads --no-default-features

# Test with the dev server
node ide/serve.mjs &
# Open http://localhost:8080, run a par() program, verify output + thread badge
```

**Verification:** `22-threading.loft` produces correct output; browser console shows
worker pool initialisation; IDE badge shows thread count.

---

### Step summary

| Step | What | Test method | Depends on |
|---|---|---|---|
| 1 | Cargo features | `cargo check` | — |
| 2 | Output capture | Rust unit test | 1 |
| 3 | Sequential `par()` | `cargo test` (no threading) | 1 |
| 4 | Logging stub | `cargo check` | 1, 2 |
| 5 | Random bridge | `cargo check` + native tests | 1 |
| 6 | Time/env bridge | `cargo check` + native tests | 1 |
| 7 | File I/O bridge | `cargo check` + native tests | 1 |
| 8 | PNG buffer decode | `cargo check` + native tests | 7 |
| 9 | `compile_and_run` | `wasm-pack` + Node.js smoke | 2, 4, 5, 6, 7 |
| 10 | VirtFS (JS) | Node.js unit tests | — |
| 11 | Host + bridge tests | Node.js + WASM | 9, 10 |
| 12 | LayeredFS + base tree | Node.js unit tests | 10 |
| 13 | Full test suite via WASM | Node.js suite runner | 11 |
| 14 | Tier 2 threading | Browser manual test | 9, 3 |

```
Steps 1-8: Rust-side — all testable with cargo
Step 9:    First WASM build — smoke test in Node.js
Steps 10-12: JavaScript-side — testable with Node.js alone
Step 13:   Full validation — WASM matches native
Step 14:   Threading upgrade — browser-only, optional
```

Steps 1-8 can be done in rough parallel (they touch different files), but the
dependency arrows above show the minimum ordering. Steps 10-12 have no Rust
dependency and can be developed in parallel with steps 1-8.

---

## Function References (`CallRef`) in WASM — W1.15

**Skip list entry:** `06-function.loft` — `#77: CallRef not implemented`

### Current status

`Value::CallRef` is handled by `output_call_ref` in `src/generation/emit.rs`.  The
implementation enumerates all reachable definitions with a matching `Type::Function`
signature and emits a `match` dispatch on the runtime `u32` definition number:

```rust
match var_fn_ref {
    3 => n_double(stores, arg0),
    7 => n_triple(stores, arg0),
    _ => panic!("unknown fn-ref {}", var_fn_ref),
}
```

This covers `fn <name>` expressions (stored as a definition number).  Lambda
expressions compile to anonymous function definitions with the same mechanism.

### Investigation step

Before implementing anything, verify whether `06-function.loft` actually fails under
the current WASM backend by removing it from `WASM_SKIP` in `tests/wrap.rs` and
running `cargo test --test wrap wasm_docs`.  If it passes, the skip was stale and
should simply be removed.

### If still failing: likely root causes

1. **Lambda with closure capture** — `output_call_ref` only handles `fn <name>` and
   uncapturing lambdas.  A lambda that captures variables (A5.6) may produce a
   `Value::CallRef` variant whose closure record is not emitted correctly in the
   `--native-wasm` backend.  Fix: confirm captured-variable closures are excluded from
   CallRef dispatch and remain interpreter-only until A5.6 lands.

2. **Higher-order stdlib functions** — `map`, `filter`, `reduce` call through
   `Value::CallRef` at the call site.  If `output_call_ref` does not collect `map`'s
   lambda as a reachable definition, the `match` arm is missing and the panic fires.
   Fix: ensure `start_fn` / `reachable` tracking follows `fn <name>` constants through
   `Value::FnRef` assignments.

### Fix path

1. Remove `"06-function.loft"` from `WASM_SKIP` and run WASM tests.
2. If tests pass — remove the entry and close issue #77.
3. If tests fail — capture the panic message to identify which specific case fails (closure capture vs. reachability), then apply the targeted fix above.
4. Add a `tests/wasm/call-ref.test.mjs` that exercises `fn <name>`, lambdas, `map`, `filter`, and `reduce` via the host bridge.

**Effort:** S (investigation + targeted fix)
**Source:** `src/generation/emit.rs:output_call_ref`, `tests/docs/06-function.loft`, issue #77

---

## Store Locks in WASM — W1.17

**Skip list entry:** `18-locks.loft` — `todo!()`

### Current status

`n_get_store_lock` and `n_set_store_lock` are listed in `CODEGEN_RUNTIME_FNS` in
`src/generation/mod.rs`.  Functions in this list are **not** emitted as `todo!()` stubs
— they are silently skipped and resolved at link time from `loft::codegen_runtime`.
`codegen_runtime.rs` implements both functions using the standard `Store::locked` flag,
which is pure Rust with no OS dependency.  No host bridge is needed.

### Investigation step

Remove `"18-locks.loft"` from `WASM_SKIP` in `tests/wrap.rs` and run `cargo test
--test wrap wasm_docs`.  Because `n_get_store_lock` / `n_set_store_lock` are
feature-agnostic (no `#[cfg(feature = "wasm")]` needed), they should work without
modification.

### If still failing

The `todo!()` comment in the WASM skip list may refer to a different function in
`18-locks.loft` — inspect the panic message.  If `set_store_lock` panics, check that
the `Store::locked` flag is correctly maintained across the `clone_for_worker` path
used by WASM worker spawning (W1.18).

### Fix path

1. Remove `"18-locks.loft"` from `WASM_SKIP` and run WASM tests.
2. If tests pass — remove the entry; the skip was stale.
3. If tests fail — capture the panic, identify the specific failing function, and apply the targeted fix.
4. Add a lock assertion to `tests/wasm/bridge.test.mjs`.

**Effort:** XS (investigation; likely a stale skip)
**Source:** `src/generation/mod.rs:CODEGEN_RUNTIME_FNS`, `tests/docs/18-locks.loft`

---

## W1.18 — Node.js Worker Threads: Testing `par()` Outside the Browser

> **⚠ Retired (@PLN117).** This whole node-Worker approach (`worker_entry` +
> `LoftThreadPool` + `worker.mjs`/`parallel.mjs` + `initThreaded`, built with
> `--target nodejs`) has been **removed**. Its premise — "node avoids the browser's
> COOP/COEP obstacle" — does not survive contact with `wasm-bindgen-rayon`, which is
> `--target web` only and needs a **Web Worker** global that node lacks. Browser
> `par()` runs on loft's own pool (`src/wasm_threads.rs`), and the proof harness is the
> **browser**: `make wasm-mt` + `tests/wasm/par-thread-proof.{html,sh}` +
> `coi-server.py`. The COOP/COEP requirement is real and documented under
> **§ Cargo Feature Gates → Build commands** above. The section below is obsolete;
> kept only as a record of the abandoned node-worker attempt.

### Why Node.js, not the browser

The browser Tier 2 design requires `SharedArrayBuffer`, which demands `COOP`/`COEP` HTTP
headers (Spectre mitigation). That makes `file://` URLs and most dev servers ineligible,
and adds a server-side prerequisite to every CI run.

Node.js removes all of these obstacles:

| Capability | Browser | Node.js |
|---|---|---|
| `SharedArrayBuffer` | Requires COOP/COEP headers | Always available |
| `Atomics.wait()` on main thread | **Blocked** (main thread is not allowed) | **Allowed** |
| Worker spawning | `new Worker(url)` | `new Worker(__filename, { workerData })` |
| Shared WASM memory | `SharedArrayBuffer` via `.memory` | Same — identical API |
| CI without a browser | Not possible | Yes — just `node` |

The test for `19-threading.loft` is currently `#[ignore]` under WASM (WASM_SKIP) because
Tier 1 runs `par()` sequentially. W1.18 enables real parallel execution in Node.js by
implementing Tier 2 entirely within the existing `tests/wasm/` harness.

---

### Architecture

```
tests/wasm/
├── parallel.mjs           ← NEW: thread pool manager
├── worker.mjs             ← NEW: worker thread entry point
├── harness.mjs            — extended: detect CPU count, init pool
├── host.mjs               — extended: parallel_run / parallel_wait hooks
└── pkg/                   — wasm-pack output (wasm-threads feature build)
```

The WASM module is compiled **once** and shared across all workers via
`WebAssembly.Module`, which is structured-cloneable. The WASM linear memory is created
as a `SharedArrayBuffer`-backed `WebAssembly.Memory` (`{ shared: true }`) so every worker
operates on the same Store heap — exactly matching the native `thread::spawn` + shared
`Stores` model.

---

### Shared memory layout

One `Int32Array` on top of a separate small `SharedArrayBuffer` carries the control
signals (not inside the WASM heap itself, to avoid alignment issues):

```
Control buffer — Int32Array(N_WORKERS * 4 entries):

  offset 0..N:       per-worker command  (0=idle, 1=run, 2=exit)
  offset N..2N:      per-worker fn_index (bytecode entry point)
  offset 2N..3N:     per-worker start    (inclusive element index)
  offset 3N..4N:     per-worker end      (exclusive element index)
```

Results are written directly to the Store heap inside shared WASM memory — no transfer
needed, matching the native `copy_nonoverlapping` pattern.

A second `Int32Array(N_WORKERS)` — `doneSignal` — is used for completion notification:
each worker writes `1` and calls `Atomics.notify` when its chunk finishes.

---

### Worker entry point (`tests/wasm/worker.mjs`)

```js
import { receiveMessageOnPort, parentPort, workerData } from 'node:worker_threads';

const { module, memory, control, done, workerId } = workerData;

// Initialise WASM with the shared memory
const { instance } = await WebAssembly.instantiate(module, {
  env: { memory },
  // host bridge functions injected identically to harness.mjs
  loftHost: buildWorkerHost(),
});

const { worker_entry } = instance.exports;

// Signal ready
Atomics.store(done, workerId, 0);
parentPort.postMessage({ type: 'ready' });

// Work loop — park until signalled
while (true) {
  Atomics.wait(control, workerId, 0);           // sleep until cmd != 0
  const cmd = Atomics.load(control, workerId);
  if (cmd === 2) break;                          // exit

  const N = done.length;
  const fnIndex = Atomics.load(control, N      + workerId);
  const start   = Atomics.load(control, N * 2  + workerId);
  const end     = Atomics.load(control, N * 3  + workerId);

  worker_entry(fnIndex, start, end);             // write results to shared Store

  Atomics.store(done, workerId, 1);
  Atomics.notify(done, workerId);
  Atomics.store(control, workerId, 0);           // reset to idle
}
```

`buildWorkerHost()` supplies the same `loftHost` bridge as `host.mjs` but wired to a
per-worker `VirtFS` snapshot (read-only view of the main thread's virtual filesystem).

---

### Thread pool manager (`tests/wasm/parallel.mjs`)

```js
import { Worker } from 'node:worker_threads';
import { fileURLToPath } from 'node:url';

const WORKER_SCRIPT = fileURLToPath(new URL('./worker.mjs', import.meta.url));

export class LoftThreadPool {
  constructor(module, memory, nWorkers) {
    this.nWorkers = nWorkers;
    // Control buffer: 4 slots × N workers (command, fn_index, start, end)
    this.control  = new Int32Array(new SharedArrayBuffer(4 * nWorkers * 4));
    this.done     = new Int32Array(new SharedArrayBuffer(nWorkers * 4));

    this.workers = Array.from({ length: nWorkers }, (_, id) =>
      new Worker(WORKER_SCRIPT, {
        workerData: { module, memory, control: this.control,
                      done: this.done, workerId: id },
      })
    );
  }

  /** Wait for all workers to post { type: 'ready' }. */
  async waitReady() {
    await Promise.all(this.workers.map(w =>
      new Promise(resolve => {
        w.once('message', (msg) => { if (msg.type === 'ready') resolve(); });
      })
    ));
  }

  /**
   * Distribute a par() loop across all workers.
   * @param {number} fnIndex  — WASM function table index for the worker body
   * @param {number} total    — total number of elements
   */
  runParallel(fnIndex, total) {
    const chunkSize = Math.ceil(total / this.nWorkers);
    const N = this.nWorkers;

    for (let t = 0; t < N; t++) {
      const start = t * chunkSize;
      const end   = Math.min(start + chunkSize, total);
      Atomics.store(this.done,    t,         0);
      Atomics.store(this.control, N      + t, fnIndex);
      Atomics.store(this.control, N * 2  + t, start);
      Atomics.store(this.control, N * 3  + t, end);
      Atomics.store(this.control, t,          1);      // command = run
      Atomics.notify(this.control, t);
    }

    // Main thread waits for all workers
    for (let t = 0; t < N; t++) {
      Atomics.wait(this.done, t, 0);                   // wait until done[t] == 1
    }
  }

  /** Shut down all workers. */
  terminate() {
    for (let t = 0; t < this.nWorkers; t++) {
      Atomics.store(this.control, t, 2);               // command = exit
      Atomics.notify(this.control, t);
    }
    return Promise.all(this.workers.map(w => w.terminate()));
  }
}
```

---

### Harness integration (`tests/wasm/harness.mjs` — additions)

```js
import { LoftThreadPool } from './parallel.mjs';
import os from 'node:os';

// Build wasm-threads feature binary for threaded tests
// wasm-pack build --target nodejs --out-dir tests/wasm/pkg-mt \
//   -- --features wasm-threads --no-default-features

async function initThreaded(wasmPath, nWorkers = os.cpus().length) {
  // Fetch and compile the module once (structured-cloneable)
  const bytes  = readFileSync(wasmPath);
  const module = await WebAssembly.compile(bytes);

  // Shared memory — this becomes the Store heap
  const memory = new WebAssembly.Memory({
    initial:  256,    // 16 MB
    maximum:  16384,  // 1 GB
    shared:   true,
  });

  const pool = new LoftThreadPool(module, memory, nWorkers);
  await pool.waitReady();

  // Instantiate the main-thread copy
  const { instance } = await WebAssembly.instantiate(module, {
    env:      { memory },
    loftHost: createHost(new VirtFS(baseTree)),
  });

  // Register the pool so the host bridge can dispatch par() calls
  instance.exports.set_thread_pool_ptr(/* ptr to pool dispatch table */);

  return { instance, pool, memory };
}
```

The `parallel_run(fn_index, total)` and `parallel_wait()` calls from `fill.rs`
(via the WASM host bridge) route to `pool.runParallel(fnIndex, total)`.

---

### W1.18-6 — Build and test environment

#### Prerequisites

```bash
# 1. Install wasm-pack (if not already)
cargo install wasm-pack

# 2. Add the WASM target
rustup target add wasm32-unknown-unknown

# 3. Node.js v16+ (for Worker Threads + SharedArrayBuffer)
node --version   # must be >= 16
```

#### Building the threaded WASM module

The threaded build requires atomics, bulk-memory, and mutable-globals target
features.  These are passed via `RUSTFLAGS`:

```bash
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  wasm-pack build --target nodejs \
    --out-dir tests/wasm/pkg-mt \
    -- --features wasm-threads --no-default-features
```

This produces `tests/wasm/pkg-mt/loft_bg.wasm` with `SharedArrayBuffer`-backed
memory support.  The build is separate from the single-threaded `pkg/` build
(which uses `--features wasm` only).

A Makefile target is provided for convenience:

```bash
make wasm-mt     # builds pkg-mt/
```

#### Running the threaded test

Once `pkg-mt/` exists:

```bash
# Run the threading test through the WASM Worker Thread pool
node tests/wasm/suite.mjs --threaded 19-threading.loft
```

Or remove `19-threading.loft` from `WASM_SKIP` in `tests/wrap.rs` and run:

```bash
cargo test --test wrap wasm_dir
```

#### CI integration

Browser threading has its own workflow — **`.github/workflows/browser-threads.yml`**
(`Browser threading`), which runs the five headless gates nightly at 05:00 UTC and on
any PR touching the threading surface (`src/parallel.rs`, `src/wasm_threads.rs`,
`wasm-sync/`, `doc/loft-thread.js`, `tests/wasm/`, …).  It installs what the rest of
CI has no reason to carry: nightly + `rust-src` (the atomics std comes from
`-Z build-std`), `wasm-pack`, binaryen, and a headless Chrome.  Locally the same run
is `make par-gates`.

Two properties keep it honest:

- **A skipped gate is a CI failure.** Every gate exits 0 when a prerequisite is
  missing so a dev without a browser still gets a useful run; `scripts/par_gates.sh
  --ci` preflights the prerequisites and turns any `SKIP` into a red job, because a
  gate that skips itself green is the one way this can rot unnoticed.
- **The performance floors are calibrated, not disabled.** A runner has 4 vCPUs
  against the dev box's 24, so the workflow lowers the scaling and frame-rate floors
  (`POOLS`, `SPEEDUP_AT`, `MIN_SPEEDUP_PCT`, `THREADS`, `MIN_FRAME_RATIO_X10`) to what
  that hardware delivers — a `par` that stops parallelising still fails.

A red run is surfaced on every PR by ci.yml's `Nightly health (informational)` job.

#### Implementation status

| Step | Status | File |
|------|--------|------|
| W1.18-1 | ✓ Done | `src/parallel.rs` — `#[cfg(wasm+threading)]` branch |
| W1.18-2 | ✓ Done (stub) | `src/wasm.rs` — `worker_entry` export |
| W1.18-3 | ✓ Done | `tests/wasm/worker.mjs` — park/wake loop |
| W1.18-4 | ✓ Done | `tests/wasm/parallel.mjs` — `LoftThreadPool` |
| W1.18-5 | ✓ Done | `tests/wasm/harness.mjs` — `initThreaded()` |
| W1.18-6 | Pending | Remove from `WASM_SKIP` after `pkg-mt/` build verified |

---

### Rust-side WASM host bridge additions (W1.18)

Two new `extern "C"` imports are declared in `src/parallel.rs` under
`#[cfg(all(target_arch = "wasm32", feature = "threading"))]`:

```rust
#[wasm_bindgen]
extern "C" {
    /// Distribute fn_index over `total` elements using the JS worker pool.
    /// Blocks (via Atomics.wait in JS) until all workers complete.
    fn parallel_run(fn_index: u32, total: u32);
}

#[cfg(all(target_arch = "wasm32", feature = "threading"))]
pub fn run_parallel_raw(
    _stores: &mut Stores,
    _program: &[u8],
    fn_pos: u32,
    _input: &DbRef,
    _element_size: u32,
    _return_size: u32,
    n_elements: u32,
) -> Vec<u64> {
    // Dispatch to the JS worker pool; results are already in shared Store memory
    unsafe { parallel_run(fn_pos, n_elements); }
    // Return an empty Vec — caller reads results directly from shared Store
    vec![]
}
```

Results land in shared WASM linear memory (the Store heap), identical to the native
`copy_nonoverlapping` path — no serialisation, no transfer, no post-processing.

---

### `worker_entry` export (Rust)

A new `#[wasm_bindgen]` export gives workers their entry point:

```rust
/// Called by each JS worker to execute one chunk of a par() loop.
/// fn_pos:   bytecode position of the worker function
/// start:    first element index (inclusive)
/// end:      last element index (exclusive)
#[wasm_bindgen]
pub fn worker_entry(fn_pos: u32, start: u32, end: u32) {
    WORKER_STATE.with(|cell| {
        let mut state = cell.borrow_mut();
        for i in start..end {
            state.execute_at_raw(fn_pos, &DbRef::element(i), 4);
        }
    });
}
```

`WORKER_STATE` is a `thread_local!` — each Web Worker / Node.js worker thread gets its
own `State` instance backed by the shared `Memory`, matching `clone_for_worker()` in
native.

---

### Sequence diagram

```
Main thread                         Worker 0..N-1
    │                                    │
    ├─ compile module (once)             │
    ├─ create shared Memory              │
    ├─ spawn N workers via Worker()      │
    │    ←── { type: 'ready' } ─────────┤ workers init WASM + park on Atomics.wait
    │                                    │
    │  [par() loop begins]               │
    ├─ write fn_index, start, end        │
    ├─ Atomics.store(control[t], 1)      │
    ├─ Atomics.notify(control[t]) ──────→│ workers wake, call worker_entry()
    ├─ Atomics.wait(done[t], 0) (block)  │ results written to shared Store heap
    │                              ←─────│ Atomics.store(done[t], 1) + notify
    ├─ all done                          │
    ├─ read results from shared Store    │ workers park again on Atomics.wait
    ├─ continue execution                │
    │                                    │
    │  [test teardown]                   │
    ├─ Atomics.store(control[t], 2)      │
    ├─ Atomics.notify(control[t]) ──────→│ workers exit
    └─ pool.terminate()                  │
```

---

### Build target

A second wasm-pack build produces the threaded binary for W1.18 tests:

```sh
# Single-threaded (existing, all other WASM tests)
wasm-pack build --target nodejs --out-dir tests/wasm/pkg \
  -- --features wasm --no-default-features

# Multi-threaded (W1.18 tests only)
RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
  wasm-pack build --target nodejs --out-dir tests/wasm/pkg-mt \
  -- --features wasm-threads --no-default-features
```

`harness.mjs` selects `pkg-mt` for tests tagged `@threaded`; all other tests
continue using `pkg`.

---

### Test coverage

Once W1.18 lands, `19-threading.loft` is removed from `WASM_SKIP` in `tests/wrap.rs`:

```rust
// tests/wrap.rs — WASM_SKIP list (W1.18 removal)
// "19-threading.loft",   ← removed when W1.18 is complete
```

The threading test file exercises:
- `par()` with Form 1 worker (`double_score(a)`)
- `par()` with Form 2 method (`a.get_value()`)
- Result ordering (results must match sequential order)
- Multi-core count (`par(..., 4)` attribute)

---

### Implementation steps (W1.18)

| Step | File | Description |
|------|------|-------------|
| W1.18-1 | `src/parallel.rs` | Add `#[cfg(wasm+threading)]` branch: `parallel_run` import + `run_parallel_raw` stub |
| W1.18-2 | `src/lib.rs` | Export `worker_entry(fn_pos, start, end)` via `#[wasm_bindgen]` |
| W1.18-3 | `tests/wasm/worker.mjs` | Worker thread script: init WASM, park/wake loop, call `worker_entry` |
| W1.18-4 | `tests/wasm/parallel.mjs` | `LoftThreadPool` class: spawn, `runParallel`, `terminate` |
| W1.18-5 | `tests/wasm/harness.mjs` | `initThreaded()` helper; route `@threaded` tests to `pkg-mt` |
| W1.18-6 | `tests/wrap.rs` | Remove `19-threading.loft` from `WASM_SKIP`; add threaded build step |

**Effort:** H (as in ROADMAP.md — shared memory + Atomics protocol + WASM export plumbing)
**Design:** ✓ (this section)

---

## Frame Yield — Browser Game Loop via Interpreter Suspension

### Problem

The loft interpreter runs synchronously to completion (`execute_argv` is a tight
`while code_pos < len` loop).  In the browser, `requestAnimationFrame` is the only
way to render frames and receive input.  A loft game loop like:

```loft
for _ in 0..3600 {
  if !gl_poll_events() { break }
  gl_clear(bg);
  gl_draw(vao, count);
  gl_swap_buffers();
}
```

executes all 3 600 iterations in a single synchronous WASM call, blocking the
browser's main thread for the entire duration.  Only the last frame is visible.
No input is processed.

### Design: yield at `gl_swap_buffers`

The interpreter pauses mid-execution at frame boundaries and returns control to
JavaScript.  JavaScript calls `requestAnimationFrame` and resumes the interpreter
on the next frame.  Loft game code is **identical** on native and browser — no
API changes, no callback pattern, no user-visible generators.

> **Two asyncify suspend imports (@PLN84 ZT-C).**  The `--html` `wasm-opt
> --asyncify --pass-arg` now names **two** import functions (comma-separated):
> `loft_gl.loft_gl_swap_buffers` (the GL frame-yield) and `loft_web.ws_yield`
> (the WebSocket frame-yield).  The `web` library's `frame_yield()` lowers to
> `loft_web.ws_yield`, so a synchronous WS poll loop returns to the JS event loop
> between iterations — without it `WebSocket.onmessage` never fires (the @P337
> deadlock applied to sockets).  `frame_yield()` does NOT call swap_buffers, so it
> needs its own suspend point — a non-GL WS client has no GL window.  The headless
> verify driver `tools/wasm_ws_repro.mjs` drives this suspend/resume loop on a
> `setImmediate` (the Node rAF equivalent); see `plans/84-zt-c-web-ws-bridge.md`.

### Alternatives considered and rejected

| Approach | Why rejected |
|---|---|
| **JS-driven frame callback** (`fn frame(dt)` pattern) | Forces a different programming model; user must manage state between calls; native loop code doesn't work in browser |
| **Web Worker + SharedArrayBuffer + Atomics.wait** | Requires COOP/COEP headers; WebGL context can't be used from a Worker (all GL calls must proxy to main thread); Safari support unreliable; massive complexity |
| **Coroutine-based game loop** | Requires user to write `for frame in game_loop() { yield }` instead of a natural loop; awkward API that doesn't match native expectations |

### Architecture

```
                         ┌─── Browser main thread ───┐
                         │                           │
  compile_and_run()      │   requestAnimationFrame   │
         │               │          │                │
         ▼               │          ▼                │
  ┌─ execute_argv ─┐     │   ┌─ resume_frame ─┐     │
  │  ...opcodes... │     │   │  clear yield    │     │
  │  gl_clear()    │     │   │  ...opcodes...  │     │
  │  gl_draw()     │     │   │  gl_clear()     │     │
  │  gl_swap()     │     │   │  gl_draw()      │     │
  │  ► YIELD ◄     │─────│──►│  gl_swap()      │     │
  │  (return)      │     │   │  ► YIELD ◄      │─────│──► next rAF
  └────────────────┘     │   └─────────────────┘     │
                         └───────────────────────────┘
```

### Implementation steps

#### Step 1: State persistence across yields

`State` (bytecode stream, stack, code_pos, call frames) must survive between
calls.  Store the active `State` in a `thread_local!` in `wasm.rs` alongside
the existing `OUTPUT` and `VIRT_FS` thread-locals.

```rust
thread_local! {
    static GAME_STATE: RefCell<Option<GameSession>> = RefCell::new(None);
}

struct GameSession {
    state: State,
    data: Data,
    database: Stores,
}
```

#### Step 2: Yield flag on Stores

Add `pub frame_yield: bool` to `Stores` (or `State`).  Default `false`.

The WASM `wgl_swap_buffers` implementation sets `stores.frame_yield = true`
after calling the JavaScript `gl_swap_buffers`.

#### Step 3: Yield check in `execute_argv`

In the main interpreter loop (`src/state/mod.rs`, `execute_argv`), check the
flag after each opcode dispatch:

```rust
while self.code_pos < bytecode_len {
    let op = *self.code::<u8>();
    OPERATORS[op as usize](self);

    // Frame yield: return to JavaScript for requestAnimationFrame
    if self.database.frame_yield {
        self.database.frame_yield = false;
        return;  // code_pos, stack, call frames all preserved in self
    }
}
```

When `execute_argv` returns early, the loft program is **suspended** — its
stack, call frames, local variables, and code position are all intact inside
`State`.  The next call to `execute_argv` (or a thin `resume` wrapper) picks
up from the exact instruction after `gl_swap_buffers`.

#### Step 4: Resume export

Add a new WASM export that JavaScript calls on each `requestAnimationFrame`:

```rust
#[wasm_bindgen]
pub fn resume_frame() -> bool {
    GAME_STATE.with(|gs| {
        let mut session = gs.borrow_mut();
        let Some(s) = session.as_mut() else { return false };
        s.state.execute_continue(&s.data);
        !s.state.is_finished()
    })
}
```

Returns `true` while the game is still running, `false` when the loft program
completes (loop finished, window closed, etc.).

#### Step 5: JavaScript frame loop

```javascript
async function runGame(source) {
  const files = JSON.stringify([{ name: 'main.loft', content: source }]);
  compile_and_start(files);  // parses, compiles, starts execute_argv
  function frame() {
    if (resume_frame()) {
      requestAnimationFrame(frame);
    }
  }
  requestAnimationFrame(frame);
}
```

#### Step 6: Input integration

`gl_poll_events` reads the current input state from JavaScript (key map,
mouse position) on each call.  Since the interpreter yields between frames,
JavaScript has time to process DOM events between `resume_frame` calls.
The existing `loftHost.gl_key_pressed` / `gl_mouse_x` / `gl_mouse_y`
functions already return the current state — they just need DOM event
listeners to keep that state updated (see GL6.6 in GAME_INFRA.md).

### Scope of changes

| File | Change |
|---|---|
| `src/database/mod.rs` | Add `pub frame_yield: bool` to `Stores` |
| `src/state/mod.rs` | Check `frame_yield` in `execute_argv` loop; add `execute_continue` |
| `src/wasm.rs` | Add `GAME_STATE` thread-local; new exports `compile_and_start`, `resume_frame` |
| `src/wasm_gl.rs` | `wgl_swap_buffers` sets `stores.frame_yield = true` |
| `doc/gallery.html` | Use `requestAnimationFrame` loop with `resume_frame` |

### Native behaviour

On native, `frame_yield` is never set (the native `gl_swap_buffers` calls
glutin's swap which blocks until vsync).  `execute_argv` runs to completion
as before.  No change to native execution.

### Performance

The yield check is one `if` per opcode — a predictable branch that is almost
always not-taken.  Cost: ~1 ns per opcode on modern CPUs, invisible compared
to the opcode dispatch itself.

### Safety analysis and mitigations

#### M1. Data ownership — raw pointer to borrowed Data (HIGH)

**Risk:** `execute_argv` stores `data_ptr = std::ptr::from_ref::<Data>(data)` where
`data` is a borrow from the caller.  If the caller drops `Data` between yield and
resume, the pointer dangles.

**Mitigation:** `GameSession` **owns** `Data` (moved in, not borrowed).  The current
`run_pipeline` already owns `p.data` — `compile_and_start` moves it into the session.
`execute_argv` is replaced by a wrapper that borrows `&session.data` for each resume,
keeping the borrow live only while the interpreter runs.

```rust
struct GameSession {
    state: State,
    data: Data,         // owned, not borrowed
}
```

#### M2. Cleanup after natural completion (MEDIUM)

**Risk:** `execute_argv` lines 1333–1335 pop the synthetic call frame and clear
`parallel_ctx` after the main loop exits.  With frame-yield, the loop returns early
on each yield, so cleanup runs only when the program finishes its last frame.

**Mitigation:** Split the loop body into `execute_loop` (the `while` loop only) and
move cleanup into `finalize()`.  `resume_frame` calls `execute_loop`; when it returns
without the yield flag set, the program is done — call `finalize` and drop the session.

```rust
// In resume_frame:
session.state.execute_loop();
if session.state.is_finished() {
    session.state.finalize();
    *session_slot = None;  // drop the session
    return false;
}
true
```

#### M3. Re-run disposes old session (MEDIUM)

**Risk:** User clicks "Run" while a game session is still alive.  The old State
holds GL resource handles (VAOs, shaders, FBOs) that leak if not freed.

**Mitigation:** `compile_and_start` checks the thread-local for an existing session.
If one exists, it calls `loftHost.gl_destroy_window()` (which frees all GL resources
on the JS side) and drops the old session before creating the new one.

```rust
pub fn compile_and_start(files_json: &str) -> String {
    GAME_STATE.with(|gs| {
        let mut slot = gs.borrow_mut();
        if slot.is_some() {
            // Dispose previous session's GL resources
            host_call("gl_destroy_window", &js_sys::Array::new());
            *slot = None;
        }
        // ... parse, compile, create new session ...
    })
}
```

#### M4. Panic during resume (MEDIUM)

**Risk:** An opcode panics inside `resume_frame`.  Without cleanup, the session
remains in the thread-local in an inconsistent state; a subsequent resume would
access corrupt State.

**Mitigation:** Wrap the resume body in `std::panic::catch_unwind`.  On panic,
dispose the session, report the error to JavaScript, and return `false`.

```rust
pub fn resume_frame() -> String {
    GAME_STATE.with(|gs| {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut slot = gs.borrow_mut();
            let Some(session) = slot.as_mut() else { return false };
            session.state.execute_loop();
            if session.state.is_finished() {
                session.state.finalize();
                *slot = None;
                return false;
            }
            true
        }));
        match result {
            Ok(running) => /* return status */,
            Err(panic) => {
                // Dispose the broken session
                *gs.borrow_mut() = None;
                host_call("gl_destroy_window", &js_sys::Array::new());
                /* return error JSON */
            }
        }
    })
}
```

#### M5. Window close / tab navigation (LOW)

**Risk:** `gl_poll_events` always returns `true` in the browser.  The loft program
loops forever if it uses `if !gl_poll_events() { break }` as the exit condition.
Navigating away orphans the session.

**Mitigation:** Add a `should_close` flag on the JavaScript side, set by a "Stop"
button or `beforeunload` event.  `loftHost.gl_poll_events()` returns `false` when
the flag is set, causing the loft loop to break naturally.

```javascript
let shouldClose = false;
document.getElementById('stop-btn').onclick = () => { shouldClose = true; };
window.addEventListener('beforeunload', () => { shouldClose = true; });

loftHost.gl_poll_events = () => !shouldClose;
```

#### M6. Debug step counter overflow (LOW)

**Risk:** The `#[cfg(debug_assertions)]` step counter in `execute_argv` accumulates
across all frames.  At 60fps with ~1000 ops/frame, the 100M limit triggers after
~28 minutes of gameplay, killing the program.

**Mitigation:** Reset `step = 0` at the start of each `execute_loop` entry (i.e.,
each `resume_frame` call).  This counts ops-per-frame rather than ops-total, which
is the useful diagnostic anyway.

### Invariants preserved across yield

These are safe by construction and require no mitigation:

| Component | Why safe |
|---|---|
| **Interpreter stack** | Owned by State, heap-allocated, persists between calls |
| **Call frames** | `call_stack: Vec<CallFrame>` owned by State |
| **Store/heap records** | `Stores.allocations` owned by State.database |
| **Text buffers** | `Str` pointers reference store records; no reallocation during yield |
| **Coroutine frames** | `State.coroutines: Vec<Option<CoroutineFrame>>` persists |
| **Scope-based freeing** | Yield is inside a scope; `OpFreeRef`/`OpFreeText` emit at scope exit, which hasn't happened |
| **WebGL context** | GL state persists across JS event loop iterations |
| **For-loop state** | Counter, limit, jump target are bytecodes — pausing between iterations is safe |

### Limitations

- **One game session at a time.** The thread-local holds a single State.
  Multiple simultaneous games would need a session ID or separate Web Workers.
- **No input between opcodes.** Input is sampled at `gl_poll_events`, not
  continuously.  This is identical to native behaviour and standard for games.

### Effort

**S–M** — The interpreter change is ~20 lines.  The WASM exports are ~60 lines
(including catch_unwind and session lifecycle).  The gallery JavaScript is ~20 lines.
Mitigations M1–M4 are structural (ownership, error handling) and add minimal code.

---

## See also
- [WEB_IDE.md](lib_plans/62-web-ide/README.md) — Full Web IDE architecture, milestones, Rust changes
- [STDLIB.md](STDLIB.md) § File System — loft file I/O API
- [STDLIB.md](STDLIB.md) § Random — `rand()`, `rand_seed()`, `rand_indices()`
- [INTERNALS.md](INTERNALS.md) — Native function registry and `src/state/io.rs`
- [TESTING.md](TESTING.md) — Rust-side test framework
- [THREADING.md](THREADING.md) — Parallel execution model (native only)
- [LOGGER.md](LOGGER.md) — Logging framework (file-based in native, console in WASM)
- [WASM file-I/O steps (FS-A…F), shipped](plans/finished/wasm-file-io-steps/README.md) — the step-by-step W1.16 implementation plan.
