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
