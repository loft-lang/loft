#!/bin/sh
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# install.sh — get a working loft onto this machine, for someone who will not build it.
#
# @PLN78 step 6.  Deliberately the least clever piece of the distribution chain, and
# that is the design rather than a shortcut.  A shell script cannot verify a signed
# registry index, so instead of teaching it to try, its job is made small enough to
# audit in one sitting: pick the artifact for this host, check one sha256, unpack it,
# and hand off to loft, which verifies itself properly (`loft verify-self`).  Everything
# this script is trusted for is on this page.
#
# What the sha256 check does and does not prove: it catches a truncated or corrupted
# download, and it detects an artifact that does not match what the release recorded.
# It is not a signature — this script and the artifact arrive over the same transport,
# so anyone who could substitute one could substitute the other.  Real authenticity
# comes from the signed registry index, which the installed binary checks.
#
# Usage:
#   curl -fsSL <url>/install.sh | sh              # latest release, into ~/.local
#   sh install.sh --prefix /usr/local             # somewhere else
#   sh install.sh --version 2026.7.2              # a specific release
#   sh install.sh --list                          # what would be installed, no changes
#
# Prefer to read before running (the script is short on purpose):
#   curl -fsSLO <url>/install.sh && less install.sh && sh install.sh
set -eu

REPO="loft-lang/loft"
# Overridable so a mirror, an air-gapped copy, or a test can supply the artifacts.
BASE="${LOFT_INSTALL_BASE:-https://github.com/$REPO/releases/download}"
PREFIX="${LOFT_PREFIX:-$HOME/.local}"
VERSION=""
LIST=0

die() { echo "install.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
  case "$1" in
    --prefix)  PREFIX="${2:-}"; [ -n "$PREFIX" ] || die "--prefix needs a directory"; shift 2 ;;
    --version) VERSION="${2:-}"; [ -n "$VERSION" ] || die "--version needs a version"; shift 2 ;;
    --list)    LIST=1; shift ;;
    -h|--help) sed -n '5,26p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)         die "unknown option: $1" ;;
  esac
done

# The triple naming the artifact for this host.  Must agree with
# `self_update::host_triple` / `PUBLISHED_TRIPLES`.  Linux is gnu, dynamically linked so
# loft can load a library's native code, and built against glibc 2.28: a system on musl,
# or on an older glibc, cannot run it and is told so here rather than after the download.
arch=$(uname -m)
case "$arch" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  *) die "unsupported architecture: $arch (loft publishes x86_64 and aarch64)" ;;
esac
case "$(uname -s)" in
  Darwin) TRIPLE="$arch-apple-darwin" ;;
  Linux)  TRIPLE="$arch-unknown-linux-gnu" ;;
  *) die "unsupported system: $(uname -s) — on Windows use the .zip from the releases page" ;;
esac
if [ "$(uname -s)" = Linux ]; then
  glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | sed -n 's/^glibc //p')
  [ -n "$glibc" ] || die "this system has no glibc (musl?): the Linux release needs glibc 2.28 or newer — build loft from a source checkout instead (make install-user)"
  oldest=$(printf '%s\n%s\n' "$glibc" 2.28 | sort -V | head -1)
  [ "$oldest" = 2.28 ] || die "this system has glibc $glibc: the Linux release needs 2.28 or newer — build loft from a source checkout instead (make install-user)"
fi

# Whichever hashing tool this system has; both print `<hash>  <file>`.
if command -v sha256sum >/dev/null 2>&1; then SHA="sha256sum"
elif command -v shasum >/dev/null 2>&1;   then SHA="shasum -a 256"
else die "need sha256sum or shasum to check the download"; fi
command -v unzip >/dev/null 2>&1 || die "need unzip to unpack the release"
if command -v curl >/dev/null 2>&1; then FETCH="curl -fsSL -o"
elif command -v wget >/dev/null 2>&1; then FETCH="wget -qO"
else die "need curl or wget to download"; fi

# Resolve the version.  Asking GitHub for the latest tag keeps the script itself
# version-free, so it does not have to be reissued for every release.
if [ -z "$VERSION" ]; then
  if [ -n "${LOFT_INSTALL_VERSION:-}" ]; then
    VERSION="$LOFT_INSTALL_VERSION"
  else
    tag=$($FETCH - "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
          | sed -n 's/.*"tag_name" *: *"v\{0,1\}\([^"]*\)".*/\1/p' | head -1) || true
    [ -n "$tag" ] || die "cannot determine the latest version — pass --version"
    VERSION="$tag"
  fi
fi

NAME="loft-$VERSION-$TRIPLE"
URL="$BASE/v$VERSION/$NAME.zip"

if [ "$LIST" = 1 ]; then
  echo "$NAME -> $PREFIX"
  echo "$URL"
  exit 0
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM

$FETCH "$tmp/$NAME.zip" "$URL" || die "cannot download $URL"
# The sidecar is advisory (same transport as the artifact); its value is catching a
# truncated or corrupted download, which is the failure that actually happens.
if $FETCH "$tmp/$NAME.zip.sha256" "$URL.sha256" 2>/dev/null; then
  want=$(cut -d' ' -f1 < "$tmp/$NAME.zip.sha256")
  got=$($SHA "$tmp/$NAME.zip" | cut -d' ' -f1)
  [ "$want" = "$got" ] || die "sha256 mismatch — expected $want, got $got.  Download not installed."
fi

unzip -q "$tmp/$NAME.zip" -d "$tmp/x" || die "cannot unpack $NAME.zip"
src="$tmp/x/$NAME"
[ -d "$src" ] || src="$tmp/x"
[ -f "$src/bin/loft" ] || die "the archive does not contain bin/loft"

mkdir -p "$PREFIX/bin" "$PREFIX/default"
# Install the WHOLE bundle as the release laid it out — the same unit `loft self-update`
# installs: every file `SHA256SUMS` lists, plus the manifest itself.  `loft verify-self`
# holds an installation to that manifest and reports a missing file as a failure (the
# manifest says it should be there), so a runtime-only copy of bin/ and default/ can
# never verify.  bin/loft goes last, renamed into place (below).
for entry in "$src"/*; do
  name=$(basename "$entry")
  [ "$name" = bin ] && continue
  if [ -d "$entry" ]; then
    mkdir -p "$PREFIX/$name" && cp -R "$entry/." "$PREFIX/$name/"
  else
    cp "$entry" "$PREFIX/$name"
  fi || die "cannot copy $name into $PREFIX"
done
# Anything shipped beside the binary in bin/ (none today) lands before the binary does.
for entry in "$src"/bin/*; do
  [ "$(basename "$entry")" = loft ] && continue
  cp -R "$entry" "$PREFIX/bin/" || die "cannot copy bin/$(basename "$entry") into $PREFIX"
done
cp "$src/bin/loft" "$PREFIX/bin/loft.new"
chmod +x "$PREFIX/bin/loft.new"
# Rename into place: a running loft can be replaced this way, and an interrupted copy
# never leaves a half-written binary on the path.
mv -f "$PREFIX/bin/loft.new" "$PREFIX/bin/loft"

echo "loft $VERSION -> $PREFIX/bin/loft"
"$PREFIX/bin/loft" verify-self || die "the installation does not verify — see above"
case ":$PATH:" in
  *":$PREFIX/bin:"*) ;;
  *) echo "add to PATH: $PREFIX/bin" ;;
esac
