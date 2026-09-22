#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# checkout_libs.sh — the library checkouts the performance pass measures from (@PLN158).
#
#   bench/portal/checkout_libs.sh            # clone what is missing, fast-forward what is clean
#   bench/portal/checkout_libs.sh --status   # say where each checkout stands, change nothing
#
# Each repository in libs.tsv becomes a NORMAL clone (full history, every branch) under
# $LOFT_PERF_LIBS — by default the directory `loft-bench-libs` beside this repository — on the
# branch libs.tsv names.  The remote is this repository's own origin with the repository name
# swapped, so a clone made over ssh clones over ssh and one made over https over https.
#
# An existing checkout is only ever FAST-FORWARDED, and only when its tree is clean: build
# caches are ignored files and do not count, but an edit somebody made by hand is left alone
# and reported.  Nothing here ever resets, stashes or force-checks-out anything.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
DEST="${LOFT_PERF_LIBS:-$(dirname "$ROOT")/loft-bench-libs}"
STATUS=0
[[ "${1:-}" == "--status" ]] && STATUS=1

origin="$(git -C "$ROOT" remote get-url origin)"
base="${origin%/*}"          # git@github.com:loft-lang   or   https://github.com/loft-lang

[[ $STATUS -eq 1 ]] || mkdir -p "$DEST"
echo "library checkouts: $DEST"
while IFS=$'\t' read -r repo branch _packages; do
  [[ -z "$repo" || "$repo" == \#* ]] && continue
  dir="$DEST/$repo"
  if [[ ! -d "$dir/.git" ]]; then
    if [[ $STATUS -eq 1 ]]; then
      printf '  %-22s MISSING (run without --status to clone)\n' "$repo"
      continue
    fi
    git clone -q --branch "$branch" "$base/$repo.git" "$dir"
    printf '  %-22s cloned      %s @ %s\n' "$repo" "$branch" "$(git -C "$dir" rev-parse --short HEAD)"
    continue
  fi
  now="$(git -C "$dir" rev-parse --abbrev-ref HEAD)"
  dirty="$(git -C "$dir" status --porcelain --untracked-files=no)"
  if [[ $STATUS -eq 1 ]]; then
    behind="$(git -C "$dir" rev-list --count "HEAD..origin/$branch" 2>/dev/null || echo '?')"
    printf '  %-22s %s @ %s, %s behind origin/%s%s\n' "$repo" "$now" \
      "$(git -C "$dir" rev-parse --short HEAD)" "$behind" "$branch" "${dirty:+ — HAS LOCAL EDITS}"
    continue
  fi
  git -C "$dir" fetch -q origin
  if [[ -n "$dirty" || "$now" != "$branch" ]]; then
    printf '  %-22s left alone  on %s%s\n' "$repo" "$now" "${dirty:+, with local edits}"
    continue
  fi
  git -C "$dir" merge -q --ff-only "origin/$branch"
  printf '  %-22s up to date  %s @ %s\n' "$repo" "$branch" "$(git -C "$dir" rev-parse --short HEAD)"
done < "$HERE/libs.tsv"
