#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# make-release — assemble a self-contained loft binary release zip per target.
# Implements the artifact half of RELEASE.md § 10. The zip holds everything a
# programmer needs to run loft offline:
#   bin/loft(.exe)        the binary
#   default/*.loft        the standard library (resolved at <exe-dir>/../default)
#   examples/*.loft       runnable sample programs
#   loft-reference.pdf    the full reference (if built)
#   README / LICENSE / CHANGELOG / QUICKSTART
#   BUILD-INFO            version + target + the exact rustc that built it
#   SHA256SUMS            sha256 of every file in the bundle
# and emits dist/<name>.zip + <name>.zip.sha256 for the registry release entry.
#
# Usage:
#   scripts/make-release.sh                  # build for the host target
#   scripts/make-release.sh <triple> ...     # cross-build one or more targets
#                                            # (each target toolchain must be installed)
# Per-OS CI runners typically call this with no args (build their own host).
set -euo pipefail
cd "$(dirname "$0")/.."

command -v zip >/dev/null || { echo "make-release: 'zip' is required" >&2; exit 1; }

# Hashing command — the per-OS runners that call this each ship a different one:
# Linux + Git-Bash (Windows) have GNU `sha256sum`; macOS ships `shasum`.  Both
# print the same `<hash>  <file>` line and accept `-c` to verify, so the bundle's
# SHA256SUMS comes out identical whichever runner built it.
# Left unquoted at the call sites on purpose so `shasum -a 256` splits into
# command + args.
if command -v sha256sum >/dev/null; then SHA256="sha256sum"; else SHA256="shasum -a 256"; fi

VERSION=$(grep -m1 '^version = ' Cargo.toml | sed -E 's/.*"([^"]+)".*/\1/')
DIST="dist"
mkdir -p "$DIST"

# `--source` — emit the source archive and nothing else.
#
# A registry entry needs one artifact for the version ITSELF: `Version` requires
# `url`/`sha256`/`size`, and a release is four per-target zips, none of which is "the
# release".  The per-target binaries are the entry's `binaries` map; this is what the
# version-level fields name — so a toolchain entry means exactly what a package entry
# means: source, plus prebuilt binaries per target.
#
# `git archive` rather than a hand-rolled zip: it is byte-reproducible from the commit
# (it reads the object database, not the working tree, so a Windows checkout produces
# the same bytes), it cannot pick up a build artefact or a local file, and it honours
# `export-ignore`.  A sha256 we publish has to stay true, which also rules out GitHub's
# auto-generated source tarballs — those are not guaranteed byte-stable.
if [ "${1:-}" = "--source" ]; then
  name="loft-$VERSION-src"
  rm -f "$DIST/$name.zip"
  git archive --format=zip --prefix="loft-$VERSION/" -o "$DIST/$name.zip" HEAD
  ( cd "$DIST" && $SHA256 "$name.zip" > "$name.zip.sha256" )
  echo ">> $DIST/$name.zip  ($(du -h "$DIST/$name.zip" | cut -f1))"
  exit 0
fi

HOST=$(rustc -vV | sed -n 's/^host: //p')
TARGETS=("$@"); [ ${#TARGETS[@]} -eq 0 ] && TARGETS=("$HOST")

# Reproducibility: strip this machine's absolute paths out of the artifact, so
# the release is one anyone can rebuild rather than one that only matches here.
# The same file is sourced by `repro-verify.sh` — if the two ever used different
# flags, every release would be unverifiable and it would look like a source
# problem.  See scripts/repro-flags.sh.
. "$(dirname "$0")/repro-flags.sh"
# `repro_c_toolchain`, which BUILD-INFO records below — the same file repro-verify.sh asks.
. "$(dirname "$0")/repro-toolchain.sh"



for TRIPLE in "${TARGETS[@]}"; do
  echo ">> building loft for $TRIPLE"
  if [ "$TRIPLE" = "$HOST" ]; then
    CARGO_INCREMENTAL=0 cargo build --release --bin loft
    bin="target/release/loft"
  else
    CARGO_INCREMENTAL=0 cargo build --release --bin loft --target "$TRIPLE"
    bin="target/$TRIPLE/release/loft"
  fi
  exe="loft"
  case "$TRIPLE" in *windows*) exe="loft.exe"; bin="$bin.exe";; esac
  [ -f "$bin" ] || { echo "make-release: no binary at $bin" >&2; exit 1; }

  name="loft-$VERSION-$TRIPLE"
  stage="$DIST/$name"
  rm -rf "$stage"
  mkdir -p "$stage/bin" "$stage/default" "$stage/examples"

  cp "$bin" "$stage/bin/$exe"
  cp default/*.loft "$stage/default/"
  cp examples/*.loft "$stage/examples/" 2>/dev/null || true
  [ -f examples/README.md ] && cp examples/README.md "$stage/examples/"
  cp README.md LICENSE CHANGELOG.md "$stage/"
  [ -f doc/loft-reference.pdf ] && cp doc/loft-reference.pdf "$stage/loft-reference.pdf"

  cat > "$stage/QUICKSTART.md" <<QS
# loft $VERSION — quick start ($TRIPLE)

Run a program with the bundled interpreter:

    bin/$exe --interpret examples/fibonacci.loft

The standard library lives in \`default/\` next to \`bin/\` — keep them together
(loft loads it from \`<binary-dir>/../default\`). Put \`bin/\` on your PATH to call
\`loft\` from anywhere.

> \`--interpret\` is the standalone runtime — it needs nothing but this bundle.
> The faster \`--native\` mode (loft's default) compiles via \`rustc\` and so needs
> a Rust toolchain installed; use it once you have one.

- CLI help:    \`bin/$exe --help\`
- Reference:   \`loft-reference.pdf\`
- Examples:    \`examples/\` (run each with \`--interpret\`)

Verify this download: \`sha256sum -c SHA256SUMS\` (macOS: \`shasum -a 256 -c
SHA256SUMS\`), or all of it at once with \`loft verify-self\`.
QS

  # What it takes to rebuild this bundle, as a FILE rather than only a stamp inside
  # the binary.  A verifier reproducing a macOS bundle from Linux cannot run its
  # `bin/loft` to ask, and rustc's exact version is not optional here: an rlib and a
  # binary are only byte-identical when the compiler is.  Written before SHA256SUMS, so
  # the manifest covers it like every other file.
  {
    echo "version = $VERSION"
    echo "target = $TRIPLE"
    echo "rustc = $(RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-} rustc --version | sed 's/^rustc //')"
    # Says the build erased its machine's paths.  A release without this line was
    # built before that, so its bytes are tied to the machine that cut it and a
    # rebuild elsewhere CANNOT match — `repro-verify.sh` reports that as "cannot
    # verify" rather than as a difference, because calling it a difference would
    # blame the source for the toolchain's behaviour.
    echo "reproducible-paths = yes"
    # The commit `build.rs` baked into LOFT_BUILD_ID.  Without it a verifier cannot
    # reproduce the stamp: it rebuilds from the source ARCHIVE, which has no `.git`,
    # so `git rev-parse` there falls through to a timestamp and the binary can never
    # match.  A release that omits this line is unverifiable, not different.
    echo "commit = $(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
    # The platform C toolchain `ring`'s C code was compiled with (scripts/repro-toolchain.sh).
    # A rebuild with another one differs by construction, so a verifier that cannot match
    # it reports "cannot verify" instead of blaming the source for the bytes.
    echo "c-toolchain = $(repro_c_toolchain "$TRIPLE")"
  } > "$stage/BUILD-INFO"

  ( cd "$stage" && find . -type f ! -name SHA256SUMS | sort | sed 's|^\./||' | xargs $SHA256 > SHA256SUMS )

  ( cd "$DIST" && rm -f "$name.zip" && zip -qr "$name.zip" "$name" )
  ( cd "$DIST" && $SHA256 "$name.zip" > "$name.zip.sha256" )
  echo ">> $DIST/$name.zip  ($(du -h "$DIST/$name.zip" | cut -f1))"
done

# @PLN78 1b — the SOURCE archive.  Emitted by `--source` (handled above, before any
# target is built), never as a side effect of a per-target build: in CI four legs run
# this script concurrently and all four would write the same `-src.zip`, so whichever
# uploaded last would silently win.  One producer, one artifact.
