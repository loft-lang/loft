#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The C backend against the interpreter, over the corpus (@PLN200 B5).

    scripts/c_sweep.py                  # every program scripts/corpus_files.sh lists
    scripts/c_sweep.py FILE...          # these programs
    scripts/c_sweep.py --verbose        # also every differing program's two outputs
    scripts/c_sweep.py --asan           # build with AddressSanitizer and UBSan: a report is a defect
    LOFT_BIN=<loft> scripts/c_sweep.py  # another build (default target/release/loft)

For each program: `loft --emit-c` writes it as C, `cc` builds it, and the C program's stdout
and exit status are compared with `loft --interpret`'s; a program without `main` is its tests,
and the C's exit status is compared with `loft --tests`'s.  Every program lands in one bucket:

  match     the backend carries it and the C answers what the interpreter answers
  differ    carried, and the answers differ — a defect of the C backend
  cc-error  carried, and the C does not compile — a defect of the C backend
  sanitizer (--asan) carried, and the C program trips a sanitizer — a defect of the C backend
  refused   not carried yet; the reason is counted, and the ranking is the next work

Exit 1 when any program differs or fails to compile, else 0.  Needs `cc` on PATH.
"""
import concurrent.futures
import os
import re
import subprocess
import sys
import tempfile
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOFT = os.environ.get("LOFT_BIN") or str(ROOT / "target" / "release" / "loft")
CC = ["cc", "-std=c99", "-O1", "-w"]
ASAN = ["-g", "-fsanitize=address,undefined", "-fno-omit-frame-pointer", "-fno-sanitize-recover=all"]
HAS_MAIN = re.compile(r"^\s*(pub\s+)?fn\s+main\s*\(", re.M)


def corpus() -> list[str]:
    r = subprocess.run([str(ROOT / "scripts" / "corpus_files.sh")], capture_output=True,
                       text=True, cwd=ROOT)
    return [l for l in r.stdout.split("\n") if l.strip()]


def run(cmd, timeout, cwd=None):
    try:
        r = subprocess.run(cmd, capture_output=True, timeout=timeout, cwd=cwd,
                           env={**os.environ, "LOFT_TIMEOUT": str(timeout)})
        return r.returncode, r.stdout, r.stderr
    except subprocess.TimeoutExpired:
        return "timeout", b"", b""


def reason(stderr: bytes) -> str:
    """The refusal without the function's name: `name: why` → `why`, numbers folded."""
    line = stderr.decode(errors="replace").strip().split("\n")[-1]
    why = line.split("yet: ", 1)[-1]
    why = why.split(": ", 1)[-1] if ": " in why else why
    return re.sub(r"\d+", "N", why)


def one(path: str, asan: bool = False):
    src = ROOT / path
    with tempfile.TemporaryDirectory(prefix="loft-c-sweep-") as tmp:
        c = os.path.join(tmp, "p.c")
        exe = os.path.join(tmp, "p")
        code, _, err = run([LOFT, "--emit-c", c, str(src)], 60, cwd=src.parent)
        if code == 2:
            return path, "refused", reason(err), None
        if code != 0:
            return path, "refused", "the program does not compile in loft", None
        code, _, err = run(CC + (ASAN if asan else []) + [c, "-o", exe], 120)
        if code != 0:
            return path, "cc-error", err.decode(errors="replace").strip().split("\n")[0], None
        c_out = run([exe], 60, cwd=src.parent)
        if asan and (b"Sanitizer" in c_out[2] or b"runtime error:" in c_out[2]):
            line = next((l for l in c_out[2].decode(errors="replace").split("\n")
                         if "Sanitizer" in l or "runtime error:" in l), "")
            return path, "sanitizer", line.strip()[:200], None
        if HAS_MAIN.search(src.read_text(encoding="utf-8", errors="replace")):
            i_out = run([LOFT, "--interpret", str(src)], 60, cwd=src.parent)
            same = (c_out[0], c_out[1]) == (i_out[0], i_out[1])
        else:
            # A program without `main` is its tests: the C runs them as `loft --tests` does,
            # and the two exit statuses say whether every test passed.
            i_out = run([LOFT, "--tests", str(src)], 60, cwd=src.parent)
            same = c_out[0] == i_out[0]
        if same:
            return path, "match", "", None
        return path, "differ", "", (i_out, c_out)


def main(argv: list[str]) -> int:
    if "-h" in argv or "--help" in argv:
        print(__doc__)
        return 0
    verbose = "--verbose" in argv
    files = [a for a in argv[1:] if not a.startswith("--")] or corpus()
    buckets = Counter()
    reasons = Counter()
    bad = []
    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as ex:
        asan = "--asan" in argv
        for path, bucket, why, outs in ex.map(lambda f: one(f, asan), files):
            buckets[bucket] += 1
            if bucket == "refused":
                reasons[why] += 1
            elif bucket in ("differ", "cc-error", "sanitizer"):
                bad.append((path, bucket, why, outs))
    total = len(files)
    print(f"C backend over {total} programs: " + ", ".join(
        f"{buckets[b]} {b}" for b in ("match", "differ", "cc-error", "sanitizer", "refused")))
    for path, bucket, why, outs in sorted(bad):
        print(f"  {bucket:8} {path}" + (f" — {why}" if why else ""))
        if verbose and outs:
            (ic, io, _), (cc, co, _) = outs
            print(f"      interpreter exit {ic}: {io[:300]!r}")
            print(f"      C           exit {cc}: {co[:300]!r}")
    if reasons:
        print("refused, by reason:")
        for why, n in reasons.most_common(25):
            print(f"  {n:5}  {why}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
