# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Shared sccache opt-in for the heavy-build scripts.  Source this near
# the top of any script that drives clean/release `cargo build|test`:
#
#     source "$(dirname "${BASH_SOURCE[0]}")/sccache_env.sh"
#
# sccache caches compiled crates across branches and runs, which is
# exactly what the batch builds here pay for repeatedly (the release
# binary, the wasm builds, the full `--release` suite).  It composes with
# the mold linker pinned in .cargo/config.toml — sccache caches the
# compile, mold still does the link.
#
# Guarded by `command -v` so this is a silent no-op where sccache is not
# installed (CI runners, other developers) — never a hard dependency.
#
# CARGO_INCREMENTAL=0 because sccache cannot cache incremental units (and
# warns when asked to).  These batch builds are not incremental, so there
# is no edit-loop cost here — the interactive `cargo run`/`build` loop is
# deliberately left untouched (no global RUSTC_WRAPPER) so it keeps its
# incremental cache.
if command -v sccache >/dev/null 2>&1; then
  export RUSTC_WRAPPER=sccache
  export CARGO_INCREMENTAL=0
fi
