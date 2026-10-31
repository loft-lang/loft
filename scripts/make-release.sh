#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# make-release — assemble a self-contained loft binary release zip per target.
# Implements the artifact half of RELEASE_PUBLISH.md § The bundles and the registry entry. The zip holds everything a
# programmer needs to run loft offline:
#   bin/loft(.exe)        the binary
#   default/*.loft        the standard library (resolved at <exe-dir>/../default)
#   lib/loft/components/  the prebuilt components (`libloft_net`: web/server/ssh natives)
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

# The bundle archive: Info-ZIP's `zip`, or 7-Zip where only it is installed (a Windows runner
# whose package feed is down).  Nothing reads the archive's own bytes — `repro-verify.sh`
# unpacks it and compares the contents' SHA256SUMS — so either writer serves.
if command -v zip >/dev/null; then
  zip_dir() { zip -qr "$1" "$2"; }
elif command -v 7z >/dev/null; then
  zip_dir() { 7z a -tzip -bso0 -bsp0 "$1" "$2"; }
else
  echo "make-release: 'zip' (or 7-Zip's '7z') is required" >&2; exit 1
fi

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

# The components (@PLN205): the prebuilt libraries behind a package's `[native] component`,
# such as `libloft_net` — web's, server's and ssh's natives.  `bin/loft` looks for them in
# `<bin>/../lib/loft/components/` (src/components.rs); a bundle without them refuses every
# program that uses one of those libraries (loft#2274).  Built by the target's own
# `loft-components`, into a scratch LOFT_HOME so no cached TLS archive of this machine — or
# of another architecture — is linked in.  The `.key` beside the library is that tool's
# rebuild cache and names the scratch path, so it is not shipped; `.loft-build-fp` is what
# the loader checks, and stays.
build_components() {
  local triple="$1" out="$2"
  local tool="target/install-lib/$triple/release/loft-components"
  case "$triple" in *windows*) tool="$tool.exe";; esac
  [ -f "$tool" ] || { echo "make-release: no loft-components at $tool" >&2; exit 1; }
  local home
  home="$(mktemp -d)"
  case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) home="$(cygpath -m "$home")";; esac
  local cc="" arch=""
  case "$triple" in
    aarch64-apple-darwin) arch=arm64 ;;
    x86_64-apple-darwin) arch=x86_64 ;;
  esac
  if [ "$triple" != "$HOST" ]; then
    # The one cross leg: x86_64 macOS on Apple Silicon.  The C is the target's because the
    # compiler is told so — not because the tool happens to run under Rosetta.
    [ -n "$arch" ] || {
      echo "make-release: cannot build loft's components for $triple on $HOST — build this bundle on a $triple host" >&2
      exit 1
    }
    cc="$home/cc-$arch"
    printf '#!/bin/sh\nexec cc -arch %s "$@"\n' "$arch" > "$cc"
    chmod +x "$cc"
  fi
  ( export LOFT_HOME="$home"
    [ -z "$cc" ] || export CC="$cc" LOFT_CC="$cc"
    "$tool" --out "$out" ) || { echo "make-release: the components did not build for $triple" >&2; exit 1; }
  rm -rf "$home"
  rm -f "$out"/.*.key
  local lib
  for lib in "$out"/*loft_*; do
    [ -f "$lib" ] || { echo "make-release: loft-components left no library in $out" >&2; exit 1; }
    # A library of the wrong architecture is found, fingerprinted and then refused by the
    # loader on the user's machine; say so here instead.
    if [ -n "$arch" ] && [ "$(lipo -archs "$lib")" != "$arch" ]; then
      echo "make-release: $lib is $(lipo -archs "$lib"), not $arch" >&2
      exit 1
    fi
  done
}

for TRIPLE in "${TARGETS[@]}"; do
  echo ">> building loft for $TRIPLE"
  # The production build (scripts/lean_build.sh): no development aids.  Always with
  # `--target`, the host's own triple included, because `repro-verify.sh` rebuilds that
  # way: without it the build-script output directory sits on another path, and a path
  # the binary embeds would differ from the rebuild's.
  CARGO_INCREMENTAL=0 scripts/lean_build.sh --bin loft --target "$TRIPLE"
  bin="target/install-lib/$TRIPLE/release/loft"
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

  # A second build, not a second `--bin` on the first: the shipped `loft` stays the binary
  # `repro-verify.sh` rebuilds with `--bin loft` alone.
  CARGO_INCREMENTAL=0 scripts/lean_build.sh --bin loft-components --target "$TRIPLE"
  build_components "$TRIPLE" "$stage/lib/loft/components"

  cat > "$stage/QUICKSTART.md" <<QS
# loft $VERSION — quick start ($TRIPLE)

Run a program with the bundled interpreter:

    bin/$exe --interpret examples/fibonacci.loft

The standard library lives in \`default/\` next to \`bin/\` — keep them together
(loft loads it from \`<binary-dir>/../default\`). Put \`bin/\` on your PATH to call
\`loft\` from anywhere.  \`lib/loft/components/\` holds the prebuilt native parts some
libraries call (web, server, ssh) — keep it beside \`bin/\` too.

> \`--interpret\` is the standalone runtime — it needs nothing but this bundle.
> The faster \`--native\` mode (loft's default) compiles via \`rustc\` against loft's
> runtime library, which a bundle deliberately does not carry: with a toolchain
> installed a default run still interprets, and an explicit \`--native\` reports a
> missing \`loft\` crate.  A source install (\`make install\` from a checkout) is what
> provides it.

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

  ( cd "$DIST" && rm -f "$name.zip" && zip_dir "$name.zip" "$name" )
  ( cd "$DIST" && $SHA256 "$name.zip" > "$name.zip.sha256" )
  echo ">> $DIST/$name.zip  ($(du -h "$DIST/$name.zip" | cut -f1))"
done

# @PLN78 1b — the SOURCE archive.  Emitted by `--source` (handled above, before any
# target is built), never as a side effect of a per-target build: in CI four legs run
# this script concurrently and all four would write the same `-src.zip`, so whichever
# uploaded last would silently win.  One producer, one artifact.
