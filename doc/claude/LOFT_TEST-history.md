<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# LOFT_TEST-history.md — what LOFT_TEST.md no longer describes

The record behind [LOFT_TEST.md](LOFT_TEST.md).

## `loft test --native` through the rustc writer — retired (@PLN225, 2026-11-24)

Until the rustc writer was retired (`@C170`), LOFT_TEST.md § Native test execution read:

When `--native` is combined with `--tests`, each file is compiled to a native
Rust binary via `output_native_reachable` + `rustc`, then executed:

1. Generate Rust source with all selected test functions called from a
   generated `main()`.  Files with `fn main()` use the loft main directly.
2. Compile with `rustc` (links against `libloft.rlib`).
3. Run the binary and check exit status.

**Binary cache:** each compiled test binary is kept in the temp directory as
`loft_test_native_<stem>_<key>_bin`, keyed by `native_cache_key` over the generated source and
the `libloft.rlib` it links; an unchanged file does not recompile.  The source is built in a
per-process directory and the binary published under its key by an atomic rename, so parallel
runs never share a half-written file.

**Stale rlib detection:** run from loft's own source root (a `Cargo.toml` in the working
directory), the runner compares `libloft.rlib` against the newest `src/` and `default/` file
and runs `cargo build --lib` when a source is newer.  Anywhere else it uses the installed rlib
as it is.
