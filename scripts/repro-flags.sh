#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# The rustc flags that make a loft build reproducible, in ONE place because two
# scripts need the identical set: `make-release.sh`, which cuts the artifact, and
# `repro-verify.sh`, which rebuilds it. If those two ever disagree, every release
# is unverifiable and the failure looks like a source problem.
#
# ## What it fixes
#
# A build embeds the absolute paths of everything it compiled — a released loft
# carried 193 of them (`/home/<user>/.cargo/registry/...`,
# `/home/<user>/.rustup/toolchains/...`). Those strings differ per machine, and
# differ in LENGTH, so the same source produced a different binary of a different
# size anywhere else.
#
# `repro-verify.sh` used to work around this by building under a fixed root. That
# cannot work and is now gone: only the VERIFIER used the root, while the release
# was cut from the maintainer's own checkout and home directory, so the two never
# had a chance to agree. Removing the paths is the fix; canonicalising one side
# of them is not.
#
# `--remap-path-prefix` is the stable way to do this (Cargo's `trim-paths` is not
# stabilised in the Cargo loft builds on). It rewrites the paths rustc records, so
# a rebuild matches from ANY directory, by any verifier, with no container.
#
# ## Usage
#   . scripts/repro-flags.sh          # exports RUSTFLAGS for the current build
#
# The three roots are remapped to fixed names. Order matters: the toolchain
# sysroot lives INSIDE $RUSTUP_HOME and is remapped first, because it also has to
# erase the toolchain's own directory name — a release built on `stable` and a
# verification pinned to `1.97.1` would otherwise still differ.

# `rustc --print sysroot` answers for the toolchain actually in use, which is the
# one whose paths get embedded — not whatever `rustup default` happens to say.
_repro_sysroot=$(rustc --print sysroot 2>/dev/null)
_repro_cargo_home="${CARGO_HOME:-$HOME/.cargo}"
_repro_src="$(pwd -P)"

RUSTFLAGS="${RUSTFLAGS:-}"
[ -n "$_repro_sysroot" ] && RUSTFLAGS="$RUSTFLAGS --remap-path-prefix=$_repro_sysroot=/rustc"
RUSTFLAGS="$RUSTFLAGS --remap-path-prefix=$_repro_cargo_home=/cargo"
RUSTFLAGS="$RUSTFLAGS --remap-path-prefix=$_repro_src=/src"

# On a Windows host the linker (`rust-lld`, per `.cargo/config.toml`) stamps every link with
# the wall clock: the COFF header's TimeDateStamp, the debug directory's copy of it, and the
# CodeView PDB GUID.  Measured 2026-09-15 on a windows-2025-vs2026 runner: two builds of one
# source from one root differed in exactly those 12 bytes and nothing else.  `/Brepro` makes
# the linker derive all three from a hash of the output.  It lives HERE rather than in
# `.cargo/config.toml` because this file exports RUSTFLAGS, which overrides a config's
# `target.<triple>.rustflags` — and here is where both the release and the verifier get it.
case "$(uname -s 2>/dev/null)" in
  MINGW*|MSYS*|CYGWIN*) RUSTFLAGS="$RUSTFLAGS -C link-arg=/Brepro" ;;
esac
export RUSTFLAGS

unset _repro_sysroot _repro_cargo_home _repro_src
