#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""A library's tests, interpreted and then with the library adopted as a hosted C unit
(@PLN206 P5a).

    scripts/hosted_suite.py --lib ROOT... PKG...   # each PKG's tests/*.loft, both ways
    scripts/hosted_suite.py --loft PATH ...        # the loft binary (default target/release/loft)
    scripts/hosted_suite.py --jobs N ...           # files run at once (default 4)
    scripts/hosted_suite.py --prebuilt ...         # adopted runs with no C compiler: only units
                                                   # `loft build-native` made beforehand

`loft test` interprets a library from its source, so a library's own suite never crosses the
compiled unit.  This runs each test file as a consumer program instead: a file with no `main`
gets one that calls its `test_*` functions in order.  The first run interprets everything; the
second sets `LOFT_LIB_BACKEND=c` with `LOFT_HOSTED_VERIFY` and `LOFT_POISON_CLAIM`, so every
call into the unit is also run interpreted on a copy of the world and compared (`P4d`).

A file is
  same        — the same output and exit both ways, and the verify compared its calls;
  differs     — another output or exit, or the verify stopped at a call (named);
  output only — the same output; the calls that crossed the unit all reached state outside
                the stores (a C function, a file), so none was compared call by call;
  not reached — the same output, but no call crossed the unit (nothing carried was called);
  skipped     — it did not run interpreted (refused, or failing on its own).
A program runs in a shadow of its package (links to its entries but the manifest and the
sources), so a fixture named from the test's directory resolves while the library is adopted
through `--lib`.  An adopted run builds `native-auto/` in each package: run this on an exported
copy of the libraries (`git archive origin/main`), never on a checkout.

The per-library line says how many of the library's public functions the unit carries, and
why the rest stay interpreted.  Exit 1 when any file differs.
"""
import os, re, subprocess, sys, tempfile
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
TEST_FN = re.compile(r"^fn (test_\w+)\(\s*\)", re.M)
VERIFY = "loft: LOFT_HOSTED_VERIFY: "


def program(src):
    """The file as a program: its own `main`, or one calling each `test_*` in order."""
    if re.search(r"^fn main\(", src, re.M):
        return src
    tests = TEST_FN.findall(src)
    if not tests:
        return None
    return src + "\nfn main() {\n" + "".join(f"  {t}();\n" for t in tests) + "}\n"


def run(loft, libs, path, env, timeout, cwd):
    args = [loft, "--interpret"]
    for lib in libs:
        args += ["--lib", lib]
    try:
        # From the package directory, as `loft test` runs a test: a fixture is named by its
        # path from there.
        r = subprocess.run(args + [path], capture_output=True, text=True, timeout=timeout,
                           env=env, cwd=cwd)
    except subprocess.TimeoutExpired:
        return None, "", "timeout"
    return r.returncode, r.stdout, r.stderr


def one(loft, libs, pkg, test, work, timeout):
    src = open(test, encoding="utf-8", errors="replace").read()
    prog = program(src)
    name = os.path.relpath(test, os.path.dirname(pkg))
    if prog is None:
        return name, "skipped", "no main and no test_* function", None
    # In the package's shadow: a test names its fixtures by their path from its own directory,
    # and from inside the package the library would be the program's own source, never
    # adopted (as under `loft test`).
    shadow = shadow_of(pkg, work)
    path = os.path.join(shadow, "tests", f"{os.getpid()}_{os.path.basename(test)}")
    with open(path, "w", encoding="utf-8") as f:
        f.write(prog)
    try:
        return judge(loft, libs, shadow, name, path, timeout)
    finally:
        os.remove(path)


_SHADOWS = {}
_SHADOW_LOCK = __import__("threading").Lock()


def shadow_of(pkg, work):
    """A directory beside no package that looks like `pkg` from its `tests/`: a link to each of
    its entries but its manifest, its sources and its caches, so a fixture path resolves and
    `use` finds the library through `--lib`."""
    with _SHADOW_LOCK:
        if pkg in _SHADOWS:
            return _SHADOWS[pkg]
        shadow = os.path.join(work, "shadow_" + str(len(_SHADOWS)), os.path.basename(pkg))
        os.makedirs(os.path.join(shadow, "tests"))
        for e in os.listdir(pkg):
            if e in ("loft.toml", "src", "tests", "native-auto", ".loft"):
                continue
            os.symlink(os.path.join(pkg, e), os.path.join(shadow, e))
        tdir = os.path.join(pkg, "tests")
        for e in os.listdir(tdir):
            if not e.startswith("."):
                os.symlink(os.path.join(tdir, e), os.path.join(shadow, "tests", e))
        _SHADOWS[pkg] = shadow
        return shadow


def judge(loft, libs, pkg, name, path, timeout):
    base = {**os.environ, "LOFT_NO_RUSTC": "1", "LOFT_HASH_SEED": "0x9E3779B97F4A7C15"}
    for k in ("LOFT_LIB_BACKEND", "LOFT_HOSTED_VERIFY", "LOFT_POISON_CLAIM"):
        base.pop(k, None)
    code_i, out_i, err_i = run(loft, libs, path, base, timeout, pkg)
    if code_i != 0:
        tail = (err_i.strip().splitlines() or ["?"])[-1][:160]
        return name, "skipped", f"interpreted exit {code_i}: {tail}", None
    adopted = {**base, "LOFT_LIB_BACKEND": "c", "LOFT_HOSTED_VERIFY": "1", "LOFT_POISON_CLAIM": "1"}
    if PREBUILT:
        # No C compiler: only a unit `loft build-native` made beforehand can be adopted.
        adopted["PATH"] = NO_CC
    code_c, out_c, err_c = run(loft, libs, path, adopted, timeout, pkg)
    lines = [l[len(VERIFY):] for l in err_c.splitlines() if l.startswith(VERIFY)]
    reach = [l for l in lines if l.startswith("library ")]
    calls = sum(int(m.group(1)) for l in lines for m in [re.match(r"(\d+) call\(s\)", l)] if m)
    stopped = next((l for l in lines if "leaves another world" in l), None)
    if stopped or code_c != code_i or out_c != out_i:
        why = stopped or (f"exit {code_c} against {code_i}" if code_c != code_i else "another output")
        if not stopped and code_c != 0:
            why += ": " + ((err_c.strip().splitlines() or ["?"])[-1][:200])
        return name, "differs", why, reach
    made = sum(int(m.group(1)) for l in lines for m in re.finditer(r"(\d+) (?:reaching|past|with)", l))
    if calls == 0 and made > 0:
        return name, "output only", f"{made} call(s) crossed the unit, none comparable (state outside the stores); the output agrees", reach
    if calls == 0:
        return name, "not reached", "no call crossed the unit", reach
    unverified = [l for l in lines if "without a comparison" in l]
    return name, "same", f"{calls} call(s)" + (f"; {unverified[0].split('; ', 1)[1]}" if unverified else ""), reach


PREBUILT = False
NO_CC = ""


def main():
    global PREBUILT, NO_CC
    args = sys.argv[1:]
    if not args or "-h" in args or "--help" in args:
        print(__doc__)
        return 2
    if "--prebuilt" in args:
        args.remove("--prebuilt")
        PREBUILT = True
        NO_CC = tempfile.mkdtemp(prefix="hosted_suite_nocc_")
    loft, jobs, timeout, libs, pkgs = os.path.join(HERE, "target/release/loft"), 4, 600, [], []
    it = iter(args)
    for a in it:
        if a == "--lib":
            libs.append(os.path.abspath(next(it)))
        elif a == "--loft":
            loft = os.path.abspath(next(it))
        elif a == "--jobs":
            jobs = int(next(it))
        elif a == "--timeout":
            timeout = int(next(it))
        else:
            pkgs.append(os.path.abspath(a))
    work = tempfile.mkdtemp(prefix="hosted_suite_")
    tasks = []
    for pkg in pkgs:
        tdir = os.path.join(pkg, "tests")
        if os.path.isdir(tdir):
            for f in sorted(os.listdir(tdir)):
                if f.endswith(".loft") and not f.startswith(".") and os.path.isfile(os.path.join(tdir, f)):
                    tasks.append((pkg, os.path.join(tdir, f)))
    with ThreadPoolExecutor(jobs) as ex:
        results = list(ex.map(lambda t: (t[0], one(loft, libs, t[0], t[1], work, timeout)), tasks))
    counts, failed, reach_by_pkg = {}, 0, {}
    for pkg, (name, verdict, why, reach) in results:
        counts[verdict] = counts.get(verdict, 0) + 1
        failed += verdict == "differs"
        for line in reach or []:
            m = re.match(r"library '([^']*)': (.*)", line)
            if m:
                reach_by_pkg.setdefault(m.group(1), m.group(2))
        print(f"{verdict:12} {name}: {why}")
    print()
    for lib, r in sorted(reach_by_pkg.items()):
        print(f"reach        {lib}: {r}")
    print("total        " + ", ".join(f"{v} {k}" for k, v in sorted(counts.items())))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
