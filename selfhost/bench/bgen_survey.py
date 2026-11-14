#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""How much of the corpus the bytecode writer in loft writes, byte for byte.

@PLN91, doc/claude/plans/91-self-host/rust-dependencies.md step 1.  For every plain program of
the coverage census's corpus (`selfhost/cover/census.py`) the Rust compiler writes its program
image and its run image; `selfhost/bench/bgen_check.loft`, built `--native-c --ship`, writes
every function from the first and compares its bytes with the second.  Printed: the functions
identical, the functions that DIFFER (a defect, each named with its program) and the ones not
written yet, with what stops them by count — the order to port in.  The standard library's
functions are in every image, so a function is counted once, by name.  Then, per program
(step 2), the whole code stream and each definition's start in it written in loft against
the image's.

    python3 selfhost/bench/bgen_survey.py [--scripts-only] [--jobs N] [--top 20]
"""
import argparse, collections, concurrent.futures as cf, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "cover"))
import census  # noqa: E402


def one(f, loft, checker, tmp, env):
    stem = os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", f))
    for flag, ext in (("--emit-image", ".store"), ("--emit-run-image", ".run")):
        p = subprocess.run([loft, flag, stem + ext, f], capture_output=True, text=True, env=env, timeout=120)
        if p.returncode != 0:
            return f, None
    try:
        p = subprocess.run([checker, stem + ".store", stem + ".run", "--names"], capture_output=True,
                           text=True, encoding="utf-8", errors="replace", env=env, timeout=300)
    except subprocess.TimeoutExpired:
        return f, None
    try:
        q = subprocess.run([checker, stem + ".store", stem + ".run", "--stream"], capture_output=True,
                           text=True, encoding="utf-8", errors="replace", env=env, timeout=300)
    except subprocess.TimeoutExpired:
        return f, p.stdout
    return f, p.stdout + q.stdout


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--scripts-only", action="store_true")
    ap.add_argument("--top", type=int, default=20)
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = os.path.join(root, "target", "release", "loft")
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
    # The Rust writer's rewrites the loft writer does not make yet are switched off on the
    # Rust side, each by its own switch, so the bytes compared are one named form of the
    # writer: no text local is a view (`LOFT_NO_TEXT_BORROW`).
    env = {**os.environ, "LOFT_NO_WARNINGS": "1", "LOFT_NO_TEXT_BORROW": "1"}
    tmp = tempfile.mkdtemp(prefix="loft-bgen-", dir=os.environ.get("TMPDIR"))
    try:
        out = os.path.join(tmp, "checker")
        os.makedirs(out, exist_ok=True)
        p = subprocess.run([loft, "--native-c", "--ship", "--out", out,
                            "--lib", os.path.join(root, "selfhost", "src"),
                            "--lib", os.path.join(root, "selfhost", "crt", "src"),
                            os.path.join(root, "selfhost", "bench", "bgen_check.loft")],
                           capture_output=True, text=True, env=env, timeout=900)
        checker = os.path.join(out, "bgen_check")
        if not os.path.exists(checker):
            sys.exit(f"the checker did not build:\n{p.stderr[-2000:]}")
        files = [os.path.abspath(f) for f in census.corpus(root, suite if os.path.isdir(suite) else None,
                                                           [], o.scripts_only) if census.plain(f)]
        state = {}            # function name -> verdict, the first program's
        differ = []
        reasons = collections.Counter()
        programs = 0
        streams = collections.Counter()
        stream_differs = []
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for f, text in ex.map(lambda f: one(f, loft, checker, tmp, env), files):
                if text is None:
                    continue
                programs += 1
                for line in text.splitlines():
                    if line.startswith("stream\t"):
                        verdict = line.split("\t")[1]
                        streams[verdict] += 1
                        if verdict == "differs":
                            stream_differs.append(f"{os.path.relpath(f, root)}\t{line.split(chr(9), 2)[2]}")
                        continue
                    m = re.match(r"(same|differs|unwritten)\t([^\t]+)(?:\t(.*))?$", line)
                    if not m:
                        continue
                    # A name is counted once, but a difference in ANY program is one: two
                    # programs' functions of one name are not the same function.
                    if m.group(1) == "differs" and state.get(m.group(2)) != "differs":
                        state[m.group(2)] = "differs"
                        differ.append(f"{m.group(2)}\t{os.path.relpath(f, root)}")
                        continue
                    if m.group(2) in state:
                        continue
                    state[m.group(2)] = m.group(1)
                    if m.group(1) == "unwritten":
                        reasons[m.group(3) or ""] += 1
        n = collections.Counter(state.values())
        print(f"{programs} programs, {len(state)} functions: {n['same']} identical, "
              f"{n['differs']} differ, {n['unwritten']} not written")
        print(f"whole code stream and starts: {streams['same']} programs identical, "
              f"{streams['differs']} differ, {streams['unwritten']} hold a function not written, "
              f"{streams['natives']} with another natives table than loft's")
        for d in sorted(stream_differs)[:20]:
            print(f"stream differs\t{d}")
        for d in sorted(differ)[:40]:
            print(f"differs\t{d}")
        for why, k in reasons.most_common(o.top):
            print(f"  {k:5d}  {why}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
