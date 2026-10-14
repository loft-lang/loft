#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# Fill this box's registry cache with every published package, so a doc build can
# render the tiers that need the package ITSELF (@PLN149).
#
# Two of the four documentation tiers read the extracted package under
# `~/.loft/registry/<name>-<version>/`, not the registry index:
#
#   Tier 1, the guide        — `docs/*.loft`, rendered to `doc/lib-<name>-guide.html`
#   Tier 3, the source       — every `.loft` the package ships, to `lib-<name>-src.html`
#
# Tiers 0 and 2 (the catalogue and the API reference) come from the index, which
# `gendoc` fetches over the network, so they render anywhere.  The other two do not,
# and a box with an empty cache publishes 42 "not on this build box" pages instead of
# the source browser — silently, because every page still generates.  That box is the
# release runner: `.github/workflows/release.yml` checks out, runs `gendoc` and
# publishes `./doc` on a fresh `ubuntu-latest`, which has no cache at all.
#
# Run before `gendoc` wherever the site is actually built.  On a developer box it is a
# no-op after the first run — `loft install` skips a version already extracted.
#
#   scripts/fetch-doc-packages.sh [path-to-loft-binary]
#
# Exit code: 0 when at least one package is in the cache afterwards, 1 when none is.
# A single package failing to download degrades one card and says so on its own page;
# fetching NOTHING removes two whole tiers, and that must not reach a deploy quietly.

set -uo pipefail

LOFT=${1:-target/release/loft}
if [ ! -x "$LOFT" ]; then
  echo "fetch-doc-packages: no loft binary at '$LOFT' — build it with \`cargo build --release --bin loft\`" >&2
  exit 1
fi
LOFT=$(cd "$(dirname "$LOFT")" && pwd)/$(basename "$LOFT")

# `loft install <name>` writes a `loft.toml` and a `.loft/api` stub into the working
# directory, which is how it records a dependency for the project you are standing in.
# Here there is no project — the target is the shared cache — so stand in a throwaway
# directory and let those land where they are deleted.
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
cd "$WORK" || exit 1

# `loft api --registry` prints two `#` header lines and then one `<name> <version> — …`
# line per package.  It also populates the cached index as a side effect, so this is
# both the package list and the fetch of the thing that lists them.
# A `while read`, not `mapfile`: macOS ships bash 3.2, which has no `mapfile`, and the
# reference pages are a derived artefact every checkout may re-derive.
PACKAGES=()
while IFS= read -r pkg; do PACKAGES+=("$pkg"); done < <("$LOFT" api --registry 2>/dev/null | awk '/^[a-z]/ { print $1 }')

if [ ${#PACKAGES[@]} -eq 0 ]; then
  echo "fetch-doc-packages: the registry index lists no packages — is the registry reachable?" >&2
  exit 1
fi

ok=0
failed=()
for pkg in "${PACKAGES[@]}"; do
  if "$LOFT" install "$pkg" >/dev/null 2>&1; then
    ok=$((ok + 1))
  else
    failed+=("$pkg")
  fi
done

echo "fetch-doc-packages: ${ok}/${#PACKAGES[@]} package(s) in the registry cache"
if [ ${#failed[@]} -gt 0 ]; then
  echo "fetch-doc-packages: could not fetch: ${failed[*]}" >&2
  echo "  their guide and source pages will say so rather than render." >&2
fi

if [ "$ok" -eq 0 ]; then
  echo "fetch-doc-packages: nothing was fetched — the guide and source tiers would be empty." >&2
  exit 1
fi
