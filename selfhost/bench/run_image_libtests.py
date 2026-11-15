#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The published libraries' tests on the loft bytecode runner, against `loft --interpret`.

@PLN91, doc/claude/plans/91-self-host/1b-fill-port.md.  A run image starts at `main`, so each
test function is run as a program of its own: the test file with a `main` that calls that one
`fn test_*()`.  It is written INTO its package — a test may read the package's private fields,
which only a file of the package may — but into a mirror of the package in a scratch directory:
its `loft.toml`, a link to each other entry (`src`, `native`, data), a `tests/` of links beside
the written programs; it runs from the package's root, as `loft test` does, with every mirrored
repository on `--lib`.  A test file with a `main` of its own is run as the program it is.  The
verdicts are the survey's (`run_image_survey.py`): `agree`, `stores`, `leak`, `differ`,
`stopped` (the first operator or native not carried, ranked), `fault`, `noimage`.  `agree`
counts a test that fails the same way on both; how many of the agreeing tests pass on the
interpreter is printed beside it.

    python3 selfhost/bench/run_image_libtests.py [--libs DIR] [--repo loft-libs-core …] [--jobs N]
"""
import argparse, collections, concurrent.futures as cf, os, re, shutil, subprocess, sys, tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from run_image_survey import STORE_REPORT, proc  # noqa: E402  one verdict for both tools

# The repositories whose packages are published; a sibling checkout of a branch has a suffix.
REPOS = ["loft-libs-core", "loft-libs-game", "loft-libs-graphics", "loft-libs-docs",
         "loft-libs-net", "loft-libs-plugins", "loft-libs-assets", "loft-libs-world"]
TEST_FN = re.compile(r"^fn (test\w*)\s*\(\s*\)", re.M)


def tests_of(libs, repos, here=None):
    """Every (repo, package, test file, test name) of the repositories — and, with `here`, of
    this tree's own `lib/` (the in-tree libraries: `engine_host`, …), named `lib`."""
    out = []
    roots = [(r, os.path.join(libs, r)) for r in repos] + ([("lib", here)] if here else [])
    for repo, root in roots:
        if not os.path.isdir(root):
            continue
        for pkg in sorted(os.listdir(root)):
            tdir = os.path.join(root, pkg, "tests")
            if not os.path.isfile(os.path.join(root, pkg, "loft.toml")) or not os.path.isdir(tdir):
                continue
            for name in sorted(os.listdir(tdir)):
                f = os.path.join(tdir, name)
                if not name.endswith(".loft") or not os.path.isfile(f):
                    continue
                src = open(f, encoding="utf-8", errors="replace").read()
                if re.search(r"^fn main\s*\(", src, re.M):
                    out.append((repo, pkg, f, "main"))
                    continue
                for t in TEST_FN.findall(src):
                    out.append((repo, pkg, f, t))
    return out


def mirror(src_root, dst_root):
    """A mirror of every package under `src_root` in `dst_root`, and the package directories."""
    for pkg in sorted(os.listdir(src_root)):
        sp = os.path.join(src_root, pkg)
        if not os.path.isfile(os.path.join(sp, "loft.toml")):
            continue
        dp = os.path.join(dst_root, pkg)
        os.makedirs(dp, exist_ok=True)
        for e in os.listdir(sp):
            if e in ("native-auto", "target", ".loft", "tests"):
                continue
            if e == "loft.toml":
                shutil.copy(os.path.join(sp, e), os.path.join(dp, e))
            else:
                os.symlink(os.path.join(sp, e), os.path.join(dp, e))
        st = os.path.join(sp, "tests")
        if os.path.isdir(st):
            os.makedirs(os.path.join(dp, "tests"), exist_ok=True)
            for e in os.listdir(st):
                if e != ".loft":
                    os.symlink(os.path.join(st, e), os.path.join(dp, "tests", e))


def one(item, loft, runner, tmp, env, timeout, libargs, mirrors):
    repo, pkg, f, t = item
    src = open(f, encoding="utf-8", errors="replace").read()
    stem = re.sub(r"[^A-Za-z0-9]", "_", f"runimg_{os.path.basename(f)}_{t}")[:150]
    pkg_root = os.path.join(mirrors[repo], pkg)
    prog = os.path.join(pkg_root, "tests", stem + ".loft")
    with open(prog, "w", encoding="utf-8") as fh:
        fh.write(src if t == "main" else src + f"\n\nfn main() {{\n  {t}();\n}}\n")
    img = os.path.join(tmp, f"{repo}_{pkg}_{stem}.run")
    cwd = pkg_root
    # The runner carries no compiled library code (1b-fill-port.md difference 9): its image keeps
    # every library function as bytecode.  The interpreter it is compared with runs as a user's
    # does, its `use`d libraries compiled.
    rc, _, err = proc([loft, *libargs, "--emit-run-image", img, prog],
                      {**env, "LOFT_NO_NATIVE_LIBS": "1"}, timeout, cwd=cwd)
    if rc != 0:
        return item, "noimage", (err.strip().splitlines()[-1:] or [""])[0][:90], False
    rc, want, err = proc([loft, *libargs, "--interpret", prog], env, timeout, cwd=cwd)
    rc2, got, err2 = proc([runner, img], {**env, "LOFT_NATIVE_LEAK_CHECK": "1"}, timeout, cwd=cwd)
    passed = rc == 0
    m = re.search(r"stopped: not carried: ([^\n]+)", err2)
    if m:
        return item, "stopped", m.group(1), passed
    if rc2 == -9:
        return item, "fault", "timeout", passed
    if rc2 != rc:
        m = re.search(r"stopped: (.*)", err2)
        return item, "fault", (m.group(1) if m else f"exit {rc2}, interpreter {rc}")[:90], passed
    if got != want:
        if STORE_REPORT.sub("", got) == STORE_REPORT.sub("", want):
            return item, "stores", "", passed
        return item, "differ", "", passed
    leaked = [int(m.group(1)) if m else 0
              for m in (re.search(r"(\d+) stores not freed at program exit", e) for e in (err, err2))]
    if leaked[0] != leaked[1]:
        return item, "leak", f"interpreter {leaked[0]}, runner {leaked[1]}", passed
    return item, "agree", "", passed


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    ap.add_argument("--loft", default=os.path.join(root, "target", "release", "loft"))
    ap.add_argument("--libs", default=os.path.dirname(root), help="the directory holding the loft-libs-* checkouts")
    ap.add_argument("--repo", action="append", default=[], help="a repository (repeatable; default every published one)")
    ap.add_argument("--in-tree", action="store_true", help="also this tree's own lib/ packages")
    ap.add_argument("--in-tree-only", action="store_true", help="this tree's own lib/ packages alone")
    ap.add_argument("--runner", default=None, help="a runner built already (DIR/run_image_run)")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--top", type=int, default=30)
    ap.add_argument("--show", action="append", default=[], help="list the tests of a verdict (repeatable)")
    o = ap.parse_args()
    repos = [] if o.in_tree_only else (o.repo or REPOS)
    here = os.path.join(root, "lib") if (o.in_tree or o.in_tree_only) else None
    env = {**os.environ, "LOFT_NO_WARNINGS": "1"}
    tmp = tempfile.mkdtemp(prefix="loft-libtests-", dir=os.environ.get("TMPDIR"))
    try:
        # Every repository mirrored, so each test is written into its package without writing
        # into the repository; the in-tree lib/ too.
        mirrors = {}
        for r in REPOS + ["lib"]:
            src_root = os.path.join(root, "lib") if r == "lib" else os.path.join(o.libs, r)
            if os.path.isdir(src_root):
                mirrors[r] = os.path.join(tmp, "mirror", r)
                os.makedirs(mirrors[r], exist_ok=True)
                mirror(src_root, mirrors[r])
        libargs = []
        for r in mirrors.values():
            libargs += ["--lib", r]
        runner = os.path.join(o.runner, "run_image_run") if o.runner else None
        if not runner:
            out = os.path.join(tmp, "runner")
            os.makedirs(out, exist_ok=True)
            p = subprocess.run([o.loft, "--native-c", "--ship", "--out", out,
                                "--lib", os.path.join(root, "selfhost", "src"),
                                "--lib", os.path.join(root, "selfhost", "crt", "src"),
                                os.path.join(root, "selfhost", "bench", "run_image_run.loft")],
                               capture_output=True, text=True, env=env, timeout=900)
            runner = os.path.join(out, "run_image_run")
            if not os.path.exists(runner):
                sys.exit(f"the runner did not build:\n{p.stderr[-2000:]}")
        items = tests_of(o.libs, repos, here)
        counts = collections.Counter()
        per_repo = collections.defaultdict(collections.Counter)
        why = collections.Counter()
        passed = 0
        shown = []
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            for item, verdict, what, ok in ex.map(
                    lambda i: one(i, o.loft, runner, tmp, env, o.timeout, libargs, mirrors), items):
                counts[verdict] += 1
                per_repo[item[0]][verdict] += 1
                if verdict == "agree" and ok:
                    passed += 1
                if verdict in ("stopped", "fault", "noimage"):
                    why[f"{verdict}: {what}"] += 1
                if verdict in o.show:
                    shown.append(f"{verdict}\t{item[0]}/{item[1]}/tests/{os.path.basename(item[2])}::{item[3]}\t{what}")
        print(f"{counts['agree']} of {len(items)} library tests agree ({passed} of them pass on the interpreter); " +
              ", ".join(f"{counts[k]} {k}" for k in ("stores", "leak", "stopped", "differ", "fault", "noimage") if counts[k]))
        for repo in repos + (["lib"] if here else []):
            c = per_repo[repo]
            if c:
                print(f"  {repo}: " + ", ".join(f"{c[k]} {k}" for k in ("agree", "stores", "leak", "stopped", "differ", "fault", "noimage") if c[k]))
        for line in sorted(shown):
            print(line)
        if why:
            print("what stops the rest, by tests:")
            for w, n in why.most_common(o.top):
                print(f"  {n:5d}  {w}")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
