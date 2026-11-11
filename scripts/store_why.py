#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Why did this program fault on a store?  One command for the chain the store instruments
form: run under `LOFT_STRICT_STORES` with the fault log (`LOFT_FAULT_LOG`), take the first
fault's store, run again with that store's life traced (`LOFT_LOG=store_life:<N>`), and print
the store's story up to the fault — where it was made, each free with the variable and the
jumps that led there, and the access that found it freed.

  scripts/store_why.py prog.loft                  # the program's `main`, on the interpreter
  scripts/store_why.py cell.loft --tests          # a test file (`fn test_*`)
  scripts/store_why.py prog.loft --env LOFT_X=1   # with a switch set (repeatable)
  scripts/store_why.py prog.loft --lib DIR        # handed to loft as --lib DIR

Exit 0 when the program has no store fault, 1 when it has one (the story printed), 2 when
the program could not be run.  The trace runs on the interpreter: a compiled backend has no
bytecode positions to trace (DEBUG_STORES.md § Debugging store-ownership bugs).
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def run(loft, prog, extra, env_add, timeout):
    env = dict(os.environ)
    env.update(LOFT_STRICT_STORES="1", LOFT_LEAK_CHECK="1", LOFT_NO_CACHE="1", LOFT_TIMEOUT=str(timeout))
    env.update(env_add)
    cmd = [loft] + extra + [prog]
    try:
        p = subprocess.run(cmd, env=env, capture_output=True, text=True, errors="replace",
                           timeout=timeout + 10)
    except subprocess.TimeoutExpired:
        return None
    return p.stdout + p.stderr


def faults_of(path):
    out = []
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                try:
                    out.append(json.loads(line))
                except ValueError:
                    pass
    except OSError:
        pass
    return out


STORY = re.compile(r"^\s+\[(alloc|free |exit )")


def story(text, store):
    """The traced store's lines up to the first strict report, from its last allocation."""
    lines = text.splitlines()
    stop = next((i for i, ln in enumerate(lines)
                 if ln.startswith("[strict-store]") or "strict stores: the child-record" in ln),
                len(lines))
    head = lines[:stop]
    start = max((i for i, ln in enumerate(head) if ln.lstrip().startswith(f"[alloc #{store} ")),
                default=None)
    if start is None:
        return []
    keep = []
    for ln in head[start:]:
        # A story line, or the lines indented under it (site, source excerpt, path).
        if STORY.match(ln) or (keep and ln.startswith("        ")) or (keep and re.match(r"^\s+\d+ \|", ln)) \
                or (keep and re.match(r"^\s+\|", ln)):
            keep.append(ln)
    return keep


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("program")
    ap.add_argument("--loft", default=os.path.join(ROOT, "target", "release", "loft"))
    ap.add_argument("--tests", action="store_true", help="run the program's `fn test_*` (loft --tests)")
    ap.add_argument("--lib", action="append", default=[], help="a library directory (repeatable)")
    ap.add_argument("--env", action="append", default=[], help="NAME=VALUE for both runs (repeatable)")
    ap.add_argument("--timeout", type=int, default=120)
    a = ap.parse_args()
    loft = os.path.abspath(a.loft)
    prog = os.path.abspath(a.program)
    extra = ["--tests"] if a.tests else ["--interpret"]
    for d in a.lib:
        extra += ["--lib", d]
    env_add = dict(e.split("=", 1) for e in a.env if "=" in e)

    fd, log = tempfile.mkstemp(prefix="store_why_", suffix=".jsonl")
    os.close(fd)
    try:
        out = run(loft, prog, extra, {**env_add, "LOFT_FAULT_LOG": log}, a.timeout)
        if out is None:
            print(f"store_why: {a.program} did not finish in {a.timeout} s")
            return 2
        faults = faults_of(log)
    finally:
        os.unlink(log)
    if not faults:
        print("no store fault")
        return 0
    kinds = {}
    for f in faults:
        kinds[f.get("kind", "?")] = kinds.get(f.get("kind", "?"), 0) + 1
    print("faults: " + ", ".join(f"{k} ×{n}" for k, n in sorted(kinds.items())))
    first = next((f for f in faults if f.get("kind") in ("use-after-free", "re-free", "wrong-free")
                  and f.get("store")), faults[0])
    print(f"first:  {first.get('kind')}" + "".join(
        f"\n  {k:8}{v}" for k, v in first.items() if k not in ("kind", "backend")))
    store = first.get("store")
    if not store or first.get("kind") == "leak":
        return 1
    traced = run(loft, prog, extra, {**env_add, "LOFT_LOG": f"store_life:{store}"}, a.timeout)
    lines = story(traced or "", store)
    if lines:
        print(f"\nstore #{store}, from its last allocation before the fault:")
        print("\n".join(lines))
    else:
        print(f"\n(no allocation of store #{store} traced before the fault — "
              f"run LOFT_LOG=store_life:{store} by hand)")
    return 1


if __name__ == "__main__":
    sys.exit(main())
