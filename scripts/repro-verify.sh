#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN78 step 7 — rebuild a published release on this platform and compare.
#
# The published sha256 says "this is the artifact the maintainer uploaded".  Rebuilding
# it from source with the compiler that release recorded upgrades that to "this is the
# artifact the source produces" — the claim a user actually wants, and the only one that
# survives a compromised build machine.
#
# ## Why this needs no pinned toolchain and no container
#
# rlibs and binaries are byte-identical only when the compiler is, and the repo pins
# `channel = "stable"` on purpose (@PLAN53 — track the newest rustc, never freeze).
# Those look contradictory and are not: the VERIFIER pins, per release, and throws the
# toolchain away afterwards.  `rustup toolchain install <exact> --profile minimal` into
# a private `RUSTUP_HOME` takes ~15s and ~600MB, and leaves the caller's default
# toolchain untouched.
#
# ## Why there is no fixed root any more
#
# A build embeds absolute paths (`$CARGO_HOME/registry/...`, `$RUSTUP_HOME/toolchains/...`
# — 193 strings for loft), so two machines agree only if those paths agree.  This script
# used to force RUSTUP_HOME, CARGO_HOME and the source under one FIXED root to make that
# happen.  It could not work, and did not: only the VERIFIER used that root, while the
# release was cut from the maintainer's own checkout and home directory.  Every
# verification failed, on every platform, and the failure read as "the source does not
# produce this binary".
#
# The paths are now erased on BOTH sides instead (`scripts/repro-flags.sh`, sourced by
# `make-release.sh` and by the rebuild below), so a rebuild matches from ANY directory,
# with no container and no magic path.
#
# Usage:
#   scripts/repro-verify.sh                 # newest release, this platform's target
#   scripts/repro-verify.sh --version 2026.7.3
#   scripts/repro-verify.sh --target x86_64-apple-darwin
#   scripts/repro-verify.sh --keep          # leave the work tree for inspection
#
# Exit: 0 identical · 1 differs · 3 cannot verify (says why; never a silent pass).
set -uo pipefail

REPO="loft-lang/loft"
# FIXED on purpose — see the canonicalisation note above.  Overridable only to run two
# verifications side by side, which costs byte-identity and says so.
ROOT="${LOFT_REPRO_ROOT:-/tmp/loft-repro}"
VERSION=""
TARGET_REQ=""
KEEP=0

die() { echo "repro-verify: $*" >&2; exit 3; }

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; [ -n "$VERSION" ] || die "--version needs a value"; shift 2 ;;
    --target)  TARGET_REQ="${2:-}"; [ -n "$TARGET_REQ" ] || die "--target needs a value"; shift 2 ;;
    --keep)    KEEP=1; shift ;;
    -h|--help) sed -n '5,30p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *)         die "unknown option: $1" ;;
  esac
done

command -v rustup >/dev/null || die "needs rustup to fetch the release's exact rustc"
command -v gh     >/dev/null || die "needs gh to download the release"
command -v unzip  >/dev/null || die "needs unzip"
if command -v sha256sum >/dev/null; then SHA="sha256sum"; else SHA="shasum -a 256"; fi
# `repro_c_toolchain` — the platform C toolchain, asked exactly as make-release.sh records it.
. "$(dirname "$0")/repro-toolchain.sh" 2>/dev/null || die "needs scripts/repro-toolchain.sh beside this script"

# The target this runner natively builds — matched to release.yml's matrix, because a
# cross-compiled binary is not expected to equal a natively built one.
#
# One triple is the exception BY CONSTRUCTION of the release: GitHub retired its Intel
# macOS image, so release.yml builds `x86_64-apple-darwin` by cross-compiling on an Apple
# Silicon runner.  The published bundle IS a cross-build, and the faithful reproduction is
# the same cross-build on the same runner class — so an aarch64 macOS host may verify it.
host=$(rustc -vV | sed -n 's/^host: //p')
CROSS_OK=""
case "$host" in
  x86_64-unknown-linux-gnu)  NATIVE="x86_64-unknown-linux-gnu" ;;
  x86_64-apple-darwin)       NATIVE="x86_64-apple-darwin" ;;
  aarch64-apple-darwin)      NATIVE="aarch64-apple-darwin"; CROSS_OK="x86_64-apple-darwin" ;;
  x86_64-pc-windows-msvc)    NATIVE="x86_64-pc-windows-msvc" ;;
  *) die "no published target for host $host" ;;
esac

# `--target` names WHICH published bundle to verify; without it, this host's own.
#
# It must be given when the caller has a target in mind, and it must be CHECKED:
# deriving the target from the host alone let a job labelled `x86_64-apple-darwin`
# run on an arm64 runner, download the AARCH64 bundle, rebuild aarch64, and report
# the result under the x86_64 name.  It could only ever fail, it said nothing about
# x86_64, and the x86_64 bundle went unverified while looking verified.  A verifier
# that reports on a target it did not build is the exact failure this script's "never
# a silent pass" rule exists to prevent — so a mismatch is `cannot verify` (3), not a
# difference (1).
TARGET="${TARGET_REQ:-$NATIVE}"
if [ "$TARGET" != "$NATIVE" ] && [ "$TARGET" != "$CROSS_OK" ]; then
  die "asked to verify $TARGET, but this host ($host) natively builds $NATIVE. A cross-compiled binary is not expected to equal a natively built one, so this runner cannot verify that target — give the job a $TARGET runner"
fi

if [ -z "$VERSION" ]; then
  VERSION=$(gh release view -R "$REPO" --json tagName -q '.tagName' 2>/dev/null | sed 's/^v//')
  [ -n "$VERSION" ] || die "cannot determine the latest release"
fi
NAME="loft-$VERSION-$TARGET"
echo "== verifying $NAME =="

rm -rf "$ROOT"; mkdir -p "$ROOT"
# The root RESOLVED, symlinks and all: on macOS `/tmp` is `/private/tmp`, and cargo records the
# resolved path of every crate it compiles under `$CARGO_HOME` — so a `--remap-path-prefix` for
# the unresolved `/tmp/loft-repro/cargo` (repro-flags.sh reads `$CARGO_HOME` as given) matched
# nothing, the registry paths stayed in the binary, and every macOS rebuild differed in size.
case "$(uname -s 2>/dev/null)" in
  MINGW*|MSYS*|CYGWIN*) ;;   # Git Bash's `pwd -P` answers /c/…, a form the native cargo does not see
  *) ROOT="$(cd "$ROOT" && pwd -P)" ;;
esac
[ "$KEEP" = 1 ] || trap 'rm -rf "$ROOT"' EXIT INT TERM

# 1. The published bundle, and what it says it was built with.
( cd "$ROOT" && gh release download "v$VERSION" -R "$REPO" -p "$NAME.zip" -O bundle.zip --clobber ) \
  || die "cannot download $NAME.zip"
unzip -q "$ROOT/bundle.zip" -d "$ROOT/published" || die "cannot unpack $NAME.zip"
pub_root="$ROOT/published/$NAME"; [ -d "$pub_root" ] || pub_root="$ROOT/published"

info="$pub_root/BUILD-INFO"
if [ ! -f "$info" ]; then
  # Deliberately exit 3, not 0.  A release with no BUILD-INFO records no compiler, so
  # nothing here can be compared — and reporting that as a pass would be the exact
  # failure this whole chain exists to prevent: a check that sounds like more than it did.
  die "v$VERSION ships no BUILD-INFO — it predates reproducible builds and cannot be verified"
fi
RUSTC_VER=$(sed -n 's/^rustc = //p' "$info" | sed 's/ .*//')
[ -n "$RUSTC_VER" ] || die "BUILD-INFO names no rustc"
# The commit the release baked into LOFT_BUILD_ID.  The rebuild has no `.git` (it is
# an unpacked source archive), so without this it would stamp a TIMESTAMP and differ
# from the release on every run — a difference the verifier caused, reported as one
# the source has.  Absent → exit 3, the same honest "cannot verify" a release without
# BUILD-INFO gets.
BUILD_COMMIT=$(sed -n 's/^commit = //p' "$info" | tr -d '\r')
[ -n "$BUILD_COMMIT" ] && [ "$BUILD_COMMIT" != "unknown" ] \
  || die "v$VERSION records no build commit — its LOFT_BUILD_ID cannot be reproduced"
# A release cut before the build stopped embedding its machine's absolute paths
# cannot be reproduced anywhere else — it carried ~193 of them
# (`/home/<user>/.cargo/registry/...`), and those strings differ, and differ in
# LENGTH, on every other machine.  Exit 3, not 1: reporting it as a DIFFERENCE
# would blame the source for the toolchain's behaviour, which is the same
# dishonesty as a silent pass, pointing the other way.
grep -q '^reproducible-paths = yes' "$info" \
  || die "v$VERSION was built before paths were made deterministic, so its bytes are tied to the machine that cut it — nothing here can be compared"
echo "   rustc $RUSTC_VER (from the bundle's BUILD-INFO)"

# 2. The source the release was cut from.  `git archive` of the tag, taken from the
#    release itself so the verifier trusts one artifact set, not a second checkout.
( cd "$ROOT" && gh release download "v$VERSION" -R "$REPO" -p "loft-$VERSION-src.zip" -O src.zip --clobber ) \
  || die "v$VERSION ships no source archive — nothing to rebuild from"
unzip -q "$ROOT/src.zip" -d "$ROOT/srcroot" || die "cannot unpack the source archive"
SRC="$ROOT/src"; mv "$ROOT/srcroot/loft-$VERSION" "$SRC" 2>/dev/null || mv "$ROOT/srcroot" "$SRC"

# 3. The exact compiler, in a private root that leaves the caller's toolchain alone.
#
# The root is no longer a canonicalisation lever, and never worked as one: only
# the VERIFIER used it, while the release was cut from the maintainer's own
# checkout and home directory, so the paths never had a chance to agree.  The
# build now erases them on BOTH sides (scripts/repro-flags.sh), which is why a
# rebuild matches from any directory.  The private root stays for the reason it
# is actually good: a throwaway toolchain that does not disturb the caller's.
export RUSTUP_HOME="$ROOT/rustup" CARGO_HOME="$ROOT/cargo"
echo "   installing rustc $RUSTC_VER (throwaway)"
rustup toolchain install "$RUSTC_VER" --profile minimal --target "$TARGET" >/dev/null 2>&1 \
  || die "cannot install rustc $RUSTC_VER"

# 4. Rebuild, exactly as make-release.sh does.
echo "   building $TARGET"
# A release whose source carries scripts/lean_build.sh was built by it (the production
# feature set, into target/install-lib); an older one by a plain default-feature build.
if [ -f "$SRC/scripts/lean_build.sh" ]; then
  build=(bash "$SRC/scripts/lean_build.sh" --bin loft --target "$TARGET"); out="target/install-lib"
else
  # That older make-release.sh passed `--target` only for a triple other than its host's,
  # and `--target` is not cosmetic: without it cargo builds the build scripts' and the
  # binary's dependencies as ONE set, with their features unified, so the code differs
  # (v2026.10.0's Windows bundle: .text 592 bytes apart).  Rebuild as it did.
  host=$(rustup run "$RUSTC_VER" rustc -vV | sed -n 's/^host: //p' | tr -d '\r')
  if [ "$TARGET" = "$host" ]; then
    build=(cargo build --release --bin loft); out="target"; host_build=1
  else
    build=(cargo build --release --bin loft --target "$TARGET"); out="target"
  fi
fi
( cd "$SRC" && . "$SRC/scripts/repro-flags.sh" \
    && LOFT_BUILD_ID="$BUILD_COMMIT" CARGO_INCREMENTAL=0 \
       rustup run "$RUSTC_VER" "${build[@]}" ) \
  || die "the rebuild failed"

exe="loft"; case "$TARGET" in *windows*) exe="loft.exe" ;; esac
rebuilt="$SRC/$out/$TARGET/release/$exe"
[ "${host_build:-0}" = 1 ] && rebuilt="$SRC/$out/release/$exe"
published="$pub_root/bin/$exe"
[ -f "$rebuilt" ]   || die "no rebuilt binary at $rebuilt"
[ -f "$published" ] || die "no published binary at $published"

a=$($SHA "$rebuilt"   | cut -d' ' -f1)
b=$($SHA "$published" | cut -d' ' -f1)
echo "   rebuilt   $a"
echo "   published $b"
if [ "$a" = "$b" ]; then
  echo "IDENTICAL — v$VERSION/$TARGET reproduces from source"
  exit 0
fi
# A Mach-O binary's LC_UUID is the one field no path remap reaches: the linker derives it
# from the debug map it writes before the strip, which names each object file by its
# absolute path on the machine that linked.  v2026.10.0's x86_64-apple-darwin rebuild
# matched in every byte but those 16, and the arm64 one also in the ad-hoc code signature's
# hash of the page holding them.  So a Mach-O that differs is compared again with the UUID
# and the signature blob zeroed on both sides, and the verdict says so.
case "$TARGET" in
  *apple-darwin)
    if python3 - "$rebuilt" "$published" <<'PY'
import struct, sys
def masked(path):
    b = bytearray(open(path, "rb").read())
    if struct.unpack_from("<I", b, 0)[0] != 0xFEEDFACF:
        sys.exit(2)
    ncmds = struct.unpack_from("<I", b, 16)[0]
    off = 32
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", b, off)
        if cmd == 0x1B:  # LC_UUID
            b[off + 8:off + 24] = bytes(16)
        if cmd == 0x1D:  # LC_CODE_SIGNATURE: an arm64 binary's ad-hoc signature hashes its
            # pages, the UUID's included; every byte it hashes is compared outside it.
            dataoff, datasize = struct.unpack_from("<II", b, off + 8)
            b[dataoff:dataoff + datasize] = bytes(datasize)
        off += size
    return bytes(b)
sys.exit(0 if masked(sys.argv[1]) == masked(sys.argv[2]) else 1)
PY
    then
      echo "IDENTICAL apart from the Mach-O LC_UUID and code signature — v$VERSION/$TARGET reproduces from source"
      echo "  (the linker derives the UUID from the absolute object paths of the machine that linked)"
      exit 0
    fi ;;
esac
# A PE binary's import HINTS are the Windows counterpart: each imported name carries a 2-byte
# guess at its index in the DLL's export table, copied from the SDK's import library, and the
# loader falls back to the name when the guess misses.  v2026.10.0's x86_64-pc-windows-msvc
# rebuild matched in every byte but its hints (all +5 — the runner's kernel32.lib numbers
# its exports apart under the same SDK version) and the /Brepro stamp, a hash over the image
# that therefore moved with them (the COFF header and the debug directory's copy).  So a
# differing PE is compared again with exactly those fields zeroed, and the verdict says so.
case "$TARGET" in
  *windows-msvc)
    if python3 - "$rebuilt" "$published" <<'PY'
import struct, sys
def masked(path):
    b = bytearray(open(path, "rb").read())
    pe = struct.unpack_from("<I", b, 0x3C)[0]
    if b[pe:pe + 4] != b"PE\0\0" or struct.unpack_from("<H", b, pe + 24)[0] != 0x20B:
        sys.exit(2)
    nsec = struct.unpack_from("<H", b, pe + 6)[0]
    optsz = struct.unpack_from("<H", b, pe + 20)[0]
    opt = pe + 24
    secs = []
    o = opt + optsz
    for _ in range(nsec):
        vs, va, _rs, rp = struct.unpack_from("<IIII", b, o + 8)
        secs.append((va, vs, rp))
        o += 40
    def off(rva):
        for va, vs, rp in secs:
            if va <= rva < va + vs:
                return rva - va + rp
        return None
    b[pe + 8:pe + 12] = bytes(4)                       # COFF TimeDateStamp (/Brepro hash)
    drva, dsize = struct.unpack_from("<II", b, opt + 112 + 8 * 6)
    p = off(drva) if drva else None
    for i in range(dsize // 28 if p is not None else 0):
        b[p + 28 * i + 4:p + 28 * i + 8] = bytes(4)     # each debug entry's TimeDateStamp
    irva = struct.unpack_from("<I", b, opt + 112 + 8)[0]
    p = off(irva)
    while p is not None:
        ilt, _ts, _fc, name, iat = struct.unpack_from("<IIIII", b, p)
        if name == 0:
            break
        q = off(ilt or iat)
        while q is not None:
            e = struct.unpack_from("<Q", b, q)[0]
            if e == 0:
                break
            if not e >> 63:
                h = off(e & 0x7FFFFFFF)
                if h is not None:
                    b[h:h + 2] = bytes(2)               # the import's hint
            q += 8
        p += 20
    return bytes(b)
sys.exit(0 if masked(sys.argv[1]) == masked(sys.argv[2]) else 1)
PY
    then
      echo "IDENTICAL apart from the PE import hints and the /Brepro stamp — v$VERSION/$TARGET reproduces from source"
      echo "  (a hint is the loader's guess at an export index, taken from the SDK's import library)"
      exit 0
    fi ;;
esac
# Size first: a size match with differing bytes points at embedded paths or timestamps,
# a size mismatch at a genuinely different build.  Saying which saves the next reader a
# bisect.
sa=$(wc -c < "$rebuilt"); sb=$(wc -c < "$published")
echo "DIFFERS — rebuilt $sa bytes, published $sb bytes"
# The rebuilt binary kept for a byte-level diff against the published one, where the caller
# names a place for it (repro-build.yml uploads it as an artifact).
if [ -n "${LOFT_REPRO_OUT:-}" ]; then
  mkdir -p "$LOFT_REPRO_OUT" && cp "$rebuilt" "$LOFT_REPRO_OUT/" && echo "   rebuilt binary kept in $LOFT_REPRO_OUT"
fi
[ "$sa" = "$sb" ] && echo "  (same size: look for embedded absolute paths or timestamps)"
# A difference is the SOURCE's only when the rebuild used the release's platform C
# toolchain (scripts/repro-toolchain.sh): `ring`'s C code is compiled by it, and two
# toolsets on one runner already produce different code.  Without that match the bytes
# say nothing about the source — exit 3, the same "cannot verify" a release without a
# compiler record gets.  Identical bytes above need no such record: they are the proof.
here_c=$(repro_c_toolchain "$TARGET")
recorded_c=$(sed -n 's/^c-toolchain = //p' "$info" | tr -d '\r')
echo "   C toolchain: release ${recorded_c:-not recorded}; this runner $here_c"
if [ "$recorded_c" != "$here_c" ]; then
  die "v$VERSION/$TARGET differs, but it was built with a platform C toolchain this runner does not have (release: ${recorded_c:-not recorded — the bundle predates the field}; here: $here_c). ring's C code is compiled by it, so the bytes cannot be compared"
fi
exit 1
