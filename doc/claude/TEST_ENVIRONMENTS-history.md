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
