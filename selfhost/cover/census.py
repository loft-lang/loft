#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN91 S2.1 — how much of the repository's programs the loft interpreter runs.

    python3 selfhost/cover/census.py [--loft target/release/loft] [--jobs N, default: every core]
                                     [--chapter operational --chapter calls …] [--scripts-only]

Each program (tests/scripts/*.loft, and the language suite's cells under LOFT_SUITE_DIR) is
emitted as a program image, run by `loft --interpret` for the expected stdout and exit, and by
`selfhost/cover/run_program.loft` (compiled once through the C backend) for the actual ones.
A program is `pass` (stdout and exit equal), `refused` (the interpreter named what it does not
carry), `leak` (it left records of the C runtime's stores live; the row says whether
`loft --interpret` leaked on it too), `wrong` (ran and answered differently) or `crash`.  What is not a plain run is
skipped: @EXPECT_ERROR / @EXPECT_FAIL / @ARGS / @EXPECT_WARNING files, and files with no main.
The rows go to target/selfhost-census.tsv; the summary ranks the refusals by programs blocked.
"""
import argparse, collections, concurrent.futures as cf, os, re, shutil, subprocess, sys, tempfile

SKIP = re.compile(r"@EXPECT_ERROR|@EXPECT_FAIL|@ARGS|@EXPECT_WARNING|@SCRIPT")

def corpus(root, suite, chapters, scripts_only):
    files = []
    if not chapters:
        files += sorted(os.path.join(root, "tests", "scripts", f)
                        for f in os.listdir(os.path.join(root, "tests", "scripts")) if f.endswith(".loft"))
    if suite and not scripts_only:
        rules = os.path.join(suite, "rules")
        for d, _, fs in sorted(os.walk(rules)):
            rel = os.path.relpath(d, rules).split(os.sep)[0]
            if chapters and rel not in chapters:
                continue
            files += sorted(os.path.join(d, f) for f in fs if f.endswith(".loft"))
    return files

def plain(path):
    try:
        src = open(path, encoding="utf-8").read()
    except OSError:
        return False
    return not SKIP.search(src) and re.search(r"^\s*fn main\s*\(", src, re.M) is not None

def run(cmd, env, timeout):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=env, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return -9, "", "timeout"

def one(path, loft, runner, tmp, env, root):
    # Each program's image is megabytes and a census runs a thousand: it goes once both
    # runs have read it.
    try:
        return judge(path, loft, runner, tmp, env, root)
    finally:
        img = path_image(path, tmp, root)
        if os.path.exists(img):
            os.remove(img)

def path_image(path, tmp, root):
    return os.path.join(tmp, re.sub(r"[^A-Za-z0-9]", "_", os.path.relpath(path, root))[-120:] + ".store")

def judge(path, loft, runner, tmp, env, root):
    img = path_image(path, tmp, root)
    libs = ["--lib", os.path.join(root, "tests", "lib")]
    code, _, err = run([loft, "--emit-image", img] + libs + [path], env, 300)
    if code != 0:
        return path, "skip", "does not compile as a plain program"
    ecode, eout, eerr = run([loft, "--interpret"] + libs + [path], env, 300)
    acode, aout, aerr = run([runner, img], env, 300)
    if acode == 3 and "refused: " in aerr:
        reason = aerr.split("refused: ", 1)[1].strip().splitlines()[0].split(" — in ")[0]
        return path, "refused", re.sub(r"\b\d+\b", "N", reason)[:160]
    if acode == 4 and "leak: " in aerr:
        also = "the Rust interpreter leaks too" if "stores not freed" in eerr else "the Rust interpreter does not"
        return path, "leak", aerr.split("leak: ", 1)[1].strip().splitlines()[0] + "; " + also
    if acode == ecode and aout == eout:
        return path, "pass", ""
    if acode < 0 or acode >= 100:
        return path, "crash", f"exit {acode}: {aerr.strip()[-160:]}"
    return path, "wrong", f"exit {acode} want {ecode}"

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--loft", default="target/release/loft")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 1)
    ap.add_argument("--chapter", action="append", default=[])
    ap.add_argument("--scripts-only", action="store_true")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft if os.path.isabs(o.loft) else os.path.join(root, o.loft)
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite"))
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_TIMEOUT="120")
    tmp = tempfile.mkdtemp(prefix="loft-census-", dir=os.environ.get("TMPDIR"))
    try:
        census(o, root, loft, suite, env, tmp)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

def census(o, root, loft, suite, env, tmp):
    out = os.path.join(tmp, "runner")
    os.makedirs(out, exist_ok=True)
    code, _, err = run([loft, "--native-c", "--ship", "--out", out,
                        os.path.join(root, "selfhost", "cover", "run_program.loft")], env, 900)
    runner = os.path.join(out, "run_program")
    if not os.path.exists(runner):
        sys.exit(f"the runner did not build:\n{err[-2000:]}")
    files = [f for f in corpus(root, suite, o.chapter, o.scripts_only) if plain(f)]
    rows = []
    with cf.ThreadPoolExecutor(o.jobs) as ex:
        for r in ex.map(lambda f: one(f, loft, runner, tmp, env, root), files):
            rows.append(r)
    tsv = os.path.join(root, "target", "selfhost-census.tsv")
    with open(tsv, "w", encoding="utf-8") as f:
        for path, kind, why in rows:
            f.write(f"{os.path.relpath(path, os.path.dirname(root))}\t{kind}\t{why}\n")
    counts = collections.Counter(k for _, k, _ in rows)
    print(f"{len(rows)} programs: " + ", ".join(f"{k} {counts[k]}" for k in ("pass", "refused", "leak", "wrong", "crash", "skip")))
    reasons = collections.Counter(w for _, k, w in rows if k == "refused")
    print("refusals by programs blocked:")
    for why, n in reasons.most_common(15):
        print(f"  {n:5}  {why}")
    for kind in ("leak", "wrong", "crash"):
        hits = [(p, w) for p, k, w in rows if k == kind]
        for path, why in hits[:20]:
            print(f"{kind}: {os.path.relpath(path, os.path.dirname(root))}  {why}")
        if len(hits) > 20:
            print(f"{kind}: … and {len(hits) - 20} more (the TSV has them)")
    print(f"rows: {tsv}")

if __name__ == "__main__":
    main()
