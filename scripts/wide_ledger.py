#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The wide ledger over the corpus: where 32-bit builds still hold or compute 64 bits (@PLN217).

Every corpus program is emitted as C under `--width 32` with `LOFT_WIDE_LEDGER` set, and the
ledgers are summed by class and by what (WIDTH32.md § The wide ledger).  Two classes are held
to a baseline, `scripts/wide_ledger.baseline`:

  undeclared   a value held in 64 bits that no declaration made wide — a compiler defect
  runtime      a value converted because a runtime function takes or answers 64 bits

A line that is not in the baseline, or a count above its baseline, fails.  A count that fell
is reported, to be blessed.  The other classes are what the programs themselves ask for and are
printed as totals only.

    scripts/wide_ledger.py              # the corpus against the baseline
    scripts/wide_ledger.py --bless      # record the current counts
    scripts/wide_ledger.py --top 30     # also the 30 largest lines of each held class
    scripts/wide_ledger.py FILE...      # these programs only (no baseline check)
    LOFT_BIN=<loft> scripts/wide_ledger.py   # another build (default target/release/loft)
"""
import argparse
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
BASELINE = ROOT / "scripts" / "wide_ledger.baseline"
HELD = ("undeclared", "runtime")


def corpus():
    out = subprocess.run(["bash", str(ROOT / "scripts" / "corpus_files.sh")], cwd=ROOT,
                         capture_output=True, text=True, check=True).stdout
    return [l for l in out.splitlines() if l.strip()]


def one(job):
    """The ledger lines of one program, or None when it does not compile under the regime."""
    k, path, tmp = job
    led = os.path.join(tmp, f"{k}.tsv")
    env = dict(os.environ, LOFT_WIDTH="32", LOFT_WIDE_LEDGER=led, LOFT_TIMEOUT="60", LOFT_NO_CACHE="1")
    c = os.path.join(tmp, f"{k}.c")
    subprocess.run([LOFT, "--native", "--no-warnings", "--native-emit", c, path], cwd=ROOT, env=env,
                   capture_output=True)
    if os.path.exists(c):
        os.remove(c)
    if not os.path.exists(led):
        return None
    with open(led, encoding="utf-8") as fh:
        rows = [l.rstrip("\n").split("\t") for l in fh if l.strip()]
    os.remove(led)
    return rows


def shape(what):
    """A compiler-made name without its number: `_len_12` and `_len_7` are one line."""
    return re.sub(r"\d+", "N", what)


def read_baseline():
    base = Counter()
    if BASELINE.exists():
        for line in BASELINE.read_text(encoding="utf-8").splitlines():
            if line and not line.startswith("#"):
                cls, what, n = line.split("\t")
                base[(cls, what)] = int(n)
    return base


def main():
    ap = argparse.ArgumentParser(description="The wide ledger over the corpus.")
    ap.add_argument("files", nargs="*")
    ap.add_argument("--bless", action="store_true")
    ap.add_argument("--top", type=int, default=0)
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) - 2))
    a = ap.parse_args()
    files = a.files or corpus()
    totals, held = Counter(), Counter()
    emitted = 0
    with tempfile.TemporaryDirectory(prefix="loft-wide-ledger-") as tmp:
        jobs = [(k, f, tmp) for k, f in enumerate(files)]
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
            for rows in pool.map(one, jobs):
                if rows is None:
                    continue
                emitted += 1
                for _fn, cls, what, n in rows:
                    totals[cls] += int(n)
                    if cls in HELD:
                        held[(cls, shape(what) if cls == "undeclared" else what)] += int(n)
    print(f"wide ledger: {emitted} of {len(files)} programs emitted at 32 bits — "
          + ", ".join(f"{totals[c]} {c}" for c in sorted(totals, key=lambda c: -totals[c])))
    if a.top:
        for cls in HELD:
            for (c, what), n in sorted(held.items(), key=lambda kv: -kv[1]):
                if c == cls:
                    print(f"  {n:>6}  {cls}  {what}")
                    a.top -= 1
                    if a.top <= 0:
                        break
    if a.files:
        return 0
    if a.bless:
        lines = ["# class\twhat\tcount — scripts/wide_ledger.py --bless; a count only goes down"]
        lines += [f"{c}\t{w}\t{n}" for (c, w), n in sorted(held.items())]
        BASELINE.write_text("\n".join(lines) + "\n", encoding="utf-8")
        print(f"blessed {len(held)} lines into {BASELINE.relative_to(ROOT)}")
        return 0
    base = read_baseline()
    rose = [(k, base.get(k, 0), n) for k, n in sorted(held.items()) if n > base.get(k, 0)]
    fell = [(k, base[k], held.get(k, 0)) for k in sorted(base) if held.get(k, 0) < base[k]]
    for (cls, what), was, now in rose:
        print(f"  ROSE  {cls}  {what}: {was} -> {now}")
    if fell:
        print(f"  {len(fell)} line(s) fell — record them: scripts/wide_ledger.py --bless")
    if rose:
        print("wide ledger: a 32-bit build does more 64-bit work than its baseline.  `undeclared` is a "
              "value the compiler made wide (type it by Parser::count_type); `runtime` is a value "
              "widened for a runtime function (give that function a form at the build's width).")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
