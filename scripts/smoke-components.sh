#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# smoke-components — does an unpacked release bundle's `loft` load its components?
#
#   scripts/smoke-components.sh <bundle>/bin/loft[.exe]
#
# Runs, under `--interpret`, a program whose natives are the `net` component's
# (tests/fixtures/web_natives declares them exactly as the web library does), so the binary
# has to find `lib/loft/components/` beside itself, accept its fingerprint and load it.  A
# bundle without the component fails here with the refusal a user would see (loft#2274).
# The natives used build a byte string in memory: no network, nothing the runner lacks.
set -euo pipefail
case "${1:-}" in
  -h|--help) echo "usage: $0 <bundle>/bin/loft[.exe] — does the bundle's loft load its \`net\` component?"; exit 0 ;;
esac
[ $# -eq 1 ] || { echo "usage: $0 <bundle>/bin/loft" >&2; exit 2; }
exe="$1"
fixtures="$(cd "$(dirname "$0")/../tests/fixtures" && pwd)"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cat > "$work/component.loft" <<'LOFT'
use web_natives;

fn main() {
  web_natives::pack_reset();
  web_natives::pack_u8(0);
  web_natives::pack_u8(255);
  web_natives::pack_u16_le(258);
  web_natives::pack_u32_le(4294967295);
  p = web_natives::pack_take();
  println("{p.len()} {web_natives::byte_at(0, p)} {web_natives::byte_at(1, p)} {web_natives::byte_at(2, p)} {web_natives::byte_at(3, p)} {web_natives::byte_at(7, p)}");
}
LOFT
# 0, 255, then 258 little-endian (2 1), then four 255s: eight bytes.
want="8 0 255 2 1 255"
got="$(LOFT_TIMEOUT="${LOFT_TIMEOUT:-60}" "$exe" --interpret --lib "$fixtures" "$work/component.loft")" || {
  echo "the bundle's loft could not run a program on its \`net\` component" >&2
  exit 1
}
got="$(printf '%s' "$got" | tr -d '\r')"
if [ "$got" != "$want" ]; then
  echo "the \`net\` component answered '$got', expected '$want'" >&2
  exit 1
fi
echo "  ok  the net component loads ($got)"
