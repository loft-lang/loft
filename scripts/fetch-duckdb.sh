#!/usr/bin/env bash
# Fetch the pinned libduckdb into ~/.local/lib, where the sqldb fixture expects it.
#
# duckdb is one of the four SQL backends loft binds through `#c`, and the only
# one no distribution ships: the library is ~70 MB, which is why it is declared
# `[c] optional-libs` and why its absence is a SKIP rather than a failure.  That
# also means nothing installs it for you, and nothing tells you it went missing —
# the duckdb cell simply stops running.  This script is how you get it back.
#
# It is NOT run by CI.  CI gates sqlite only (doc/claude/TEST_ENVIRONMENTS.md § Database
# backends); duckdb is part of the LOCAL four-backend bar.
#
# Usage:  scripts/fetch-duckdb.sh [--force]
set -euo pipefail

VERSION="v1.5.5"
# sha256 of the extracted libduckdb.so, not of the zip.  Recorded from the copy
# this project has been testing against since @PLN23; re-record deliberately
# when the pin moves, never to make a mismatch go away.
EXPECT_SHA="fc23f12e376c47be520f75221288281906e7942e8fd6f6ce4849198ba60d0405"
URL="https://github.com/duckdb/duckdb/releases/download/${VERSION}/libduckdb-linux-amd64.zip"

DEST="${DUCKDB_DEST:-$HOME/.local/lib}"
SO="$DEST/libduckdb.so"

if [ "${1:-}" != "--force" ] && [ -f "$SO" ]; then
    have=$(sha256sum "$SO" | cut -d' ' -f1)
    if [ "$have" = "$EXPECT_SHA" ]; then
        echo "libduckdb $VERSION already present and matching: $SO"
        exit 0
    fi
    echo "libduckdb at $SO does not match the pin."
    echo "  have:   $have"
    echo "  expect: $EXPECT_SHA"
    echo "Re-run with --force to replace it."
    exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "fetching duckdb $VERSION …"
curl -fsSL -o "$tmp/duckdb.zip" "$URL"
unzip -q -o "$tmp/duckdb.zip" -d "$tmp"

if [ ! -f "$tmp/libduckdb.so" ]; then
    echo "the archive did not contain libduckdb.so — upstream changed its layout." >&2
    ls -la "$tmp" >&2
    exit 1
fi

got=$(sha256sum "$tmp/libduckdb.so" | cut -d' ' -f1)
if [ "$got" != "$EXPECT_SHA" ]; then
    # A mismatch is information, not an obstacle: upstream re-cut the release, or
    # the pin is stale.  Decide which, then edit EXPECT_SHA on purpose.
    echo "sha256 mismatch for $VERSION — NOT installing." >&2
    echo "  got:    $got" >&2
    echo "  expect: $EXPECT_SHA" >&2
    exit 1
fi

mkdir -p "$DEST"
install -m 0755 "$tmp/libduckdb.so" "$SO"
echo "installed $SO ($VERSION)"
echo
echo "Use it with:  LD_LIBRARY_PATH=$DEST"
echo "Check it:     LD_LIBRARY_PATH=$DEST LOFT_SQLDB_MODE=duckdb \\"
echo "                target/release/loft --interpret --lib tests/fixtures/sqldb \\"
echo "                tests/fixtures/sqldb/uniform.loft"
