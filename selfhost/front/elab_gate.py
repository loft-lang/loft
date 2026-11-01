#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The elaboration gate (@PLN91 stage 1, gate E): does the loft front end elaborate each program
as loft's own parser does?

For each corpus program the Rust front end writes the image as its parser left it
(`LOFT_EMIT_PARSED`, before the scope pass); `compare/defs.loft` reports that image's program
definitions and `front/elab.loft` elaborates the program over the image's standard library and
reports its own.  A program AGREES when the two reports are equal and the elaboration refused
nothing.  Both runners are built through the C backend once per run.  The refusals are counted
by reason: the constructs the port carries next.

  selfhost/front/elab_gate.py [--part NAME]… [--jobs N] [--show N] [FILE…]
"""
import argparse, collections, concurrent.futures as cf, glob, os, subprocess, sys, tempfile

PARTS = ["scripts", "docs", "suite", "lib"]


def corpus(root, suite):
    return {
        "scripts": sorted(glob.glob(os.path.join(root, "tests", "scripts", "*.loft"))),
        "docs": sorted(glob.glob(os.path.join(root, "tests", "docs", "*.loft"))),
        "suite": sorted(glob.glob(os.path.join(suite, "rules", "**", "*.loft"), recursive=True)),
        "lib": sorted(glob.glob(os.path.join(root, "lib", "**", "*.loft"), recursive=True)),
    }


def build(loft, src, out, env):
    os.makedirs(out, exist_ok=True)
    b = subprocess.run([loft, "--native-c", "--ship", "--out", out, src], env=env,
                       capture_output=True, text=True, timeout=1800)
    exe = os.path.join(out, os.path.splitext(os.path.basename(src))[0])
    if not os.path.exists(exe):
        sys.exit(f"{src} did not build:\n{b.stderr[-3000:]}")
    return exe


def one(loft, defs, elab, path, tmp, env, i):
    """(verdict, detail): `agree`, `noimage` (the Rust front end refused it), `differ` (the
    first differing line) or `refused` (the reasons)."""
    img = os.path.join(tmp, f"p{i}.store")
    try:
        r = subprocess.run([loft, path], env=dict(env, LOFT_EMIT_PARSED=img),
                           capture_output=True, timeout=180)
        if r.returncode != 0 or not os.path.exists(img):
            return "noimage", ""
        o = subprocess.run([defs, img, "program"], env=env, capture_output=True, text=True,
                           errors="replace", timeout=300)
        m = subprocess.run([elab, img, path], env=env, capture_output=True, text=True,
                           errors="replace", timeout=300)
    except subprocess.TimeoutExpired:
        return "differ", "timed out"
    finally:
        for f in (img, img + ".dschema"):
            if os.path.exists(f):
                os.remove(f)
    lines = m.stdout.splitlines()
    refused = [l[len("refused: "):] for l in lines if l.startswith("refused: ")]
    mine = [l for l in lines if not l.startswith("refused: ")]
    if m.returncode != 0 and not refused:
        return "differ", f"elab exited {m.returncode}: {m.stderr.strip()[-300:]}"
    if refused:
        return "refused", refused
    want = o.stdout.splitlines()
    if mine == want:
        return "agree", ""
    for k, (a, b) in enumerate(zip(want, mine)):
        if a != b:
            head = next((x for x in reversed(want[:k + 1]) if x.startswith("#")), "")
            return "differ", f"{head}\n    oracle: {a}\n    loft:   {b}"
    return "differ", f"oracle {len(want)} lines, loft {len(mine)}"


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("files", nargs="*", help="programs to compare (default: the corpus)")
    ap.add_argument("--part", action="append", choices=PARTS, help="corpus parts (default: all)")
    ap.add_argument("--loft", default="target/release/loft")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--show", type=int, default=20, help="differing programs to list")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft if os.path.isabs(o.loft) else os.path.join(root, o.loft)
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite-91"))
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_TIMEOUT="300")
    if o.files:
        todo = [("args", os.path.abspath(f)) for f in o.files]
        parts = ["args"]
    else:
        parts = o.part or PARTS
        files = corpus(root, suite)
        todo = [(part, os.path.abspath(f)) for part in parts for f in files[part]]
    tmp = tempfile.mkdtemp(prefix="loft-elab-", dir=os.environ.get("TMPDIR"))
    sh = os.path.join(root, "selfhost")
    defs = build(loft, os.path.join(sh, "compare", "defs.loft"), os.path.join(tmp, "defs"), env)
    elab = build(loft, os.path.join(sh, "front", "elab.loft"), os.path.join(tmp, "elab"), env)
    per = collections.defaultdict(collections.Counter)
    reasons = collections.Counter()
    differs = []
    with cf.ThreadPoolExecutor(o.jobs) as ex:
        futs = {ex.submit(one, loft, defs, elab, f, tmp, env, i): (part, f)
                for i, (part, f) in enumerate(todo)}
        for fut in cf.as_completed(futs):
            part, f = futs[fut]
            verdict, detail = fut.result()
            per[part][verdict] += 1
            if verdict == "refused":
                reasons.update(set(detail))
            elif verdict == "differ":
                differs.append((f, detail))
    tot = collections.Counter()
    for c in per.values():
        tot.update(c)
    n = sum(tot.values()) - tot["noimage"]
    print(f"{tot['agree']} of {n} programs agree; {tot['refused']} refused, {tot['differ']} differ"
          + (f"; {tot['noimage']} the Rust front end refuses" if tot["noimage"] else ""))
    for part in parts:
        c = per[part]
        print(f"  {part:8} {c['agree']} of {sum(c.values()) - c['noimage']}")
    print("refusals, by the programs that meet them:")
    for r, k in reasons.most_common(25):
        print(f"  {k:5}  {r}")
    for f, d in sorted(differs)[:o.show]:
        print(f"DIFFER {os.path.relpath(f, os.path.dirname(root))}\n  {d}")
    sys.exit(0 if tot["agree"] == n else 1)


if __name__ == "__main__":
    main()
