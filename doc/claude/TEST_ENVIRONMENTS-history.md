<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# TEST_ENVIRONMENTS — history

## Cargo 1.100's build-dir layout (2026-11)

Cargo 1.100 became stable on 2026-11-09 with the per-unit build-dir layout.  The native
toolchain, `scripts/make-release.sh` and the library cdylib builds read only `deps/`, so a
`--native` build failed with `E0463`.  Until they read both layouts, `make` and every CI
workflow set cargo's temporary opt-out, `__CARGO_TEMPORARY_BUILD_DIR_NEW_LAYOUT_OPT_OUT=1`
(rust-lang/cargo#17182: removed one release later).  The readers moved to
`src/cargo_layout.rs` and `scripts/cargo_dep_files.sh` in loft#2474, and the opt-out was
retired on 2026-11-13, before the release.

## The build-dir readers and the runtime auto-rebuild, until the rustc writer went (2026-11, @PLN225)

`scripts/cargo_dep_files.sh` read both layouts for `make install` (which copied the runtime
rlib's dependencies into `share/loft/deps/`) and for `find_problems.sh`'s loft_ffi toolchain
guard; neither copies or prunes a dependency rlib any more, and the script went.  A `--native`
run whose runtime rlib was built by another rustc rebuilt the runtime with
`cargo build --release` from the current tree, replacing `target/release/loft` underneath any
gate that ran it, so a gate set `LOFT_NO_AUTO_REBUILD=1`; `--native` compiles through the C
now and links no runtime rlib.

## A private `HOME` hid the Rust toolchain — retired with the rustc writer (@PLN225, 2026-11-24)

A `use`d library's native build went through rustc while the rustc writer stood (`@C170`); the
section as it stood in TEST_ENVIRONMENTS.md:


A test that runs `loft` with `HOME` pointed at a scratch directory (the usual way to give it a
private registry cache) also moves rustup, which finds its toolchains through `HOME`.  With no
`rustc` reachable, every `use`d library interprets and prints *"has no native build and no
Rust toolchain"* — so the auto-native path is silently untested, and a defect that lives
only there shows up as an "intermittent" failure in whichever runner happens to expose
`rustc` (nextest does).  Point `LOFT_HOME` at the scratch directory and leave `HOME` alone,
or set `RUSTUP_HOME` / `CARGO_HOME` to the real ones, when the native half is part of what
the test claims.  `n3_use_native.rs::a_dependency_version_is_part_of_its_users_native_artifact`
is the worked example; the defect it guards read as a flake in `manifest_less_resolution.rs`.

## Debugging `loft --html` WASM traps through the generated Rust — retired with the rustc writer (@PLN225, 2026-11-24)

The page's module was the rustc writer's `wasm32-unknown-unknown` build; TEST_ENVIRONMENTS.md
now describes the C route.  The technique as it stood:


WASM compiled for `wasm32-unknown-unknown` with `-O` uses `panic = "abort"`: a panic becomes
a bare `unreachable` instruction with no message, no location and no panic handler, so the
engine reports only `RuntimeError: unreachable executed` and a WASM function index.  The
panic site is recovered by rebuilding the same generated Rust without `-O`.

#### The technique

1. **Write a minimal reproducer.** If `fn main() { }` or
   `fn main() { println("hi"); }` traps, the bug is in WASM init
   (Stores::new / stdlib load / host-import wiring), **not** in any
   user-code path.  Don't bisect user code yet.

2. **Keep the generated Rust.**  `LOFT_KEEP_NATIVE_RS=1` makes `loft --html` keep the
   `prog.rs` it compiled and print where (`browser-wasm source preserved at …`, a per-process
   directory under the temp dir):

   ```bash
   LOFT_KEEP_NATIVE_RS=1 ./target/release/loft --html /tmp/app.html app.loft
   cp <printed path> /tmp/loft_html_saved.rs
   ```

3. **Compile the saved Rust without `-O`** so Rust's panic machinery
   still emits symbols:

   ```bash
   rustc --edition=2024 --target wasm32-unknown-unknown \
     --crate-type cdylib \
     --extern loft=target/loft/html/wasm32-unknown-unknown/release/libloft.rlib \
     -L dependency=target/loft/html/wasm32-unknown-unknown/release/deps \
     -L dependency=target/release/deps \
     /tmp/loft_html_saved.rs -o /tmp/debug.wasm
   ```

4. **Run in Node with `tools/wasm_repro.mjs`** and print the stack:

   ```bash
   node tools/wasm_repro.mjs /tmp/debug.wasm
   ```

   The debug build's stack shows `_ZN...` mangled Rust symbols.  The
   first non-panic-machinery symbol is the function that panicked —
   often `std::...::now`, `Vec::index`, `Option::unwrap`,
   `core::panicking::panic_fmt`.
