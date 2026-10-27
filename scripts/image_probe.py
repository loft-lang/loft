#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN205 L0b — does the program IMAGE carry everything the C backend reads?

For every program `scripts/corpus_files.sh` lists, `loft --emit-c` writes the C twice: from the
program in memory, and from its image (`LOFT_EMIT_FROM_IMAGE=1`: the `Data` and type schema saved
as a bundle and read back into fresh stores, as a backend that is a separate part reads it).  The
two must be byte-identical, and must refuse alike.  Prints one line per program that differs and
a summary; exit 1 when any differs.

    scripts/image_probe.py [--jobs N] [--limit N] [--loft PATH] [--keep DIR]
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def programs() -> list[Path]:
    r = subprocess.run([str(ROOT / "scripts" / "corpus_files.sh")], capture_output=True,
                       text=True, cwd=ROOT, check=True)
    return [ROOT / line for line in r.stdout.splitlines() if line.strip()]


def emit(loft: str, src: Path, out: Path, image: bool) -> tuple[int, str]:
    env = dict(os.environ, LOFT_TIMEOUT="60")
    env.pop("LOFT_EMIT_FROM_IMAGE", None)
    if image:
        env["LOFT_EMIT_FROM_IMAGE"] = "1"
    try:
        r = subprocess.run([loft, "--emit-c", str(out), str(src)], capture_output=True,
                           text=True, cwd=src.parent, env=env, timeout=120)
    except subprocess.TimeoutExpired:
        return -1, "timeout"
    return r.returncode, r.stderr


def reason(err: str) -> str:
    """The line that says why loft stopped, past any warning the program draws."""
    lines = [l for l in err.splitlines() if l.startswith("loft:") or l.startswith("error")]
    return (lines[-1] if lines else err.strip()[-300:]).strip()


def one(loft: str, src: Path, tmp: Path) -> tuple[str, str]:
    stem = f"{abs(hash(str(src)))}"
    a, b = tmp / f"{stem}.mem.c", tmp / f"{stem}.img.c"
    ca, ea = emit(loft, src, a, False)
    cb, eb = emit(loft, src, b, True)
    if ca != cb:
        return "differ", f"exit {ca} from memory, {cb} from the image: {reason(eb if cb else ea)}"
    if ca != 0:
        if reason(ea) == reason(eb):
            return "same-refusal", ""
        return "differ", f"refusals differ: {reason(ea)} | {reason(eb)}"
    if a.read_bytes() != b.read_bytes():
        return "differ", "the C differs"
    a.unlink()
    b.unlink()
    return "same", ""


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--loft", default=str(ROOT / "target" / "debug" / "loft"))
    ap.add_argument("--keep", default="")
    args = ap.parse_args()
    # Each emit runs in its program's directory, so the binary is named absolutely.
    args.loft = str(Path(args.loft).resolve())
    progs = programs()
    if args.limit:
        progs = progs[: args.limit]
    tmp = Path(args.keep) if args.keep else Path(tempfile.mkdtemp(prefix="loft_image_probe_"))
    tmp.mkdir(parents=True, exist_ok=True)
    counts: dict[str, int] = {}
    with cf.ThreadPoolExecutor(args.jobs) as ex:
        futs = {ex.submit(one, args.loft, p, tmp): p for p in progs}
        for f in cf.as_completed(futs):
            verdict, why = f.result()
            counts[verdict] = counts.get(verdict, 0) + 1
            if verdict == "differ":
                print(f"DIFFER {futs[f].relative_to(ROOT) if futs[f].is_relative_to(ROOT) else futs[f]}: {why}")
    print(f"{len(progs)} programs: " + ", ".join(f"{v} {k}" for k, v in sorted(counts.items())))
    print(f"kept: {tmp}" if counts.get("differ") else "", end="")
    return 1 if counts.get("differ") else 0


if __name__ == "__main__":
    sys.exit(main())
