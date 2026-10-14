#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Was src/compiled_stdlib_gen.rs compiled from default/*.loft as they are NOW?

    scripts/compiled_stdlib_fresh.py           # exit 0 fresh, 1 stale (and say what to run)
    scripts/compiled_stdlib_fresh.py --quiet   # the exit code alone

Why
---
The loft binary bakes in the standard library's compiled bodies (@PLN181), recorded against a
hash of the `default/*.loft` source.  When the source moves on and the file is not regenerated,
the binary declines the WHOLE compiled library at start-up and interprets those functions —
correct, and silently 5-23x slower on every text routine (measured 2026-10-02: a commit adding
operator markers to default/ made `join` +2213 %, `split` +907 %, and the bench harness
reported it as a regression of the change being measured).

Nothing failed, so nothing said so.  This is the one check, asked at every point that can act
on it: the build (`build.rs`, a cargo warning), the gate pre-flight (`gate_preflight.sh`,
refuses), `find_problems --changed` (a default/ edit runs `tests/compiled_stdlib.rs`), and the
bench harness (`bench/stats.py`, refuses to measure a stale tree).

The hash is `compiled_stdlib::source_hash` re-derived without building anything: SHA-256 over
each `default/*.loft` file's name and bytes, in name order, every part prefixed by its length
as 8 little-endian bytes; the first 8 bytes of the digest, little-endian.  It checks the SOURCE
half only — the type-table prefix the runtime also compares changes with the compiler, and
`tests/compiled_stdlib.rs::compiled_stdlib_up_to_date` (a full regeneration) owns that half.
"""

import hashlib
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
GENERATED = ROOT / "src" / "compiled_stdlib_gen.rs"
DEFAULT = ROOT / "default"


def source_hash(default_dir: Path) -> int:
    h = hashlib.sha256()
    # Files only: a program cache directory `default/.loft/` (ignored by git) also matches
    # `*.loft`, and reading it as a source stopped the gate's pre-flight.
    sources = (p for p in default_dir.glob("*.loft") if p.is_file())
    for path in sorted(sources, key=lambda p: p.name):
        for part in (path.name.encode(), path.read_bytes()):
            h.update(len(part).to_bytes(8, "little"))
            h.update(part)
    return int.from_bytes(h.digest()[:8], "little")


def recorded_hash(generated: Path):
    m = re.search(r"SOURCE_HASH: u64 = (\d+);", generated.read_text())
    return int(m.group(1)) if m else None


def main() -> int:
    quiet = "--quiet" in sys.argv[1:]
    recorded = recorded_hash(GENERATED) if GENERATED.is_file() else None
    current = source_hash(DEFAULT)
    if recorded == current:
        return 0
    if not quiet:
        print(
            "the compiled standard library is STALE: src/compiled_stdlib_gen.rs was compiled from "
            "a different default/*.loft, so a binary built now interprets every compiled stdlib "
            "function (correct, and 5-23x slower on text routines).  Run: make compiled-stdlib",
            file=sys.stderr,
        )
    return 1


if __name__ == "__main__":
    sys.exit(main())
