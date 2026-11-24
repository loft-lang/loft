#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Does every library run compiled?  The public functions each library's hosted C unit leaves
interpreted, against a baseline that only shrinks (@PLN206 coverage).

    scripts/hosted_coverage.py --lib ROOT... PKG...            # each PKG against the baseline
    scripts/hosted_coverage.py --strict --lib ROOT... PKG...   # none may stay interpreted
    scripts/hosted_coverage.py --update --lib ROOT... PKG...   # write what is measured
    scripts/hosted_coverage.py --baseline FILE ...             # default: scripts/hosted_coverage.baseline
    scripts/hosted_coverage.py --loft PATH ...                 # default: target/release/loft

Each package is built as a hosted C unit (`loft build-native`), which
names every public function the C bridge does not carry and why (`interpreted: <name>\\t<why>`):
such a function runs interpreted under a program that adopts the library, answering the same,
only slower.  A package whose native part is a Rust crate (`[native] crate`) has no unit at all until
it has a `[c] shim`; it is listed apart and never counted.

Red when a function is interpreted that the baseline does not list (coverage lost), and when the
baseline lists one that now runs compiled (coverage won: write it down, `--update`, so the
ratchet holds the gain).  `--strict` is the goal: red while anything stays interpreted.  The
packages not named keep their baseline lines.  Run on an exported copy of the libraries
(`git archive origin/main`): a build writes `native-auto/` in each package.
"""
import os, re, subprocess, sys

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BASELINE = os.path.join(HERE, "scripts", "hosted_coverage.baseline")


def package_name(pkg):
    """The `[package] name` of the manifest at `pkg`, else its directory's name."""
    try:
        text = open(os.path.join(pkg, "loft.toml"), encoding="utf-8").read()
    except OSError:
        return None
    m = re.search(r'^\s*name\s*=\s*"([^"]+)"', text, re.M)
    return m.group(1) if m else os.path.basename(pkg)


def measure(loft, libs, pkg):
    """`({function: why}, None)`, or `(None, why not measured)`."""
    args = [loft]
    for lib in libs:
        args += ["--lib", lib]
    env = dict(os.environ)
    r = subprocess.run(args + ["build-native", pkg], capture_output=True, text=True, env=env, timeout=900)
    out = {}
    for line in r.stdout.splitlines():
        if line.startswith("interpreted: ") and "\t" in line:
            name, why = line[len("interpreted: "):].split("\t", 1)
            out[name] = why
    if r.returncode != 0:
        tail = (r.stderr.strip().splitlines() or ["?"])[-1][:200]
        return None, f"build-native failed: {tail}"
    return out, None


def read_baseline(path):
    """`{package: {function: why}}` from `<package>\\t<function>\\t<why>` lines."""
    base = {}
    if not os.path.exists(path):
        return base
    for line in open(path, encoding="utf-8"):
        line = line.rstrip("\n")
        if not line or line.startswith("#"):
            continue
        pkg, name, why = (line.split("\t") + ["", ""])[:3]
        base.setdefault(pkg, {})[name] = why
    return base


def write_baseline(path, base):
    with open(path, "w", encoding="utf-8") as f:
        f.write("# The public library functions a hosted C unit leaves interpreted, and why\n")
        f.write("# (scripts/hosted_coverage.py; only shrinks).  <package>\\t<function>\\t<why>\n")
        for pkg in sorted(base):
            for name in sorted(base[pkg]):
                f.write(f"{pkg}\t{name}\t{base[pkg][name]}\n")


def main():
    args = sys.argv[1:]
    if not args or "-h" in args or "--help" in args:
        print(__doc__)
        return 2
    loft, baseline, libs, pkgs = os.path.join(HERE, "target/release/loft"), BASELINE, [], []
    strict = update = False
    it = iter(args)
    for a in it:
        if a == "--lib":
            libs.append(os.path.abspath(next(it)))
        elif a == "--loft":
            loft = os.path.abspath(next(it))
        elif a == "--baseline":
            baseline = os.path.abspath(next(it))
        elif a == "--strict":
            strict = True
        elif a == "--update":
            update = True
        else:
            pkgs.append(os.path.abspath(a))
    base = read_baseline(baseline)
    failed = 0
    totals = {}
    for pkg in pkgs:
        name = package_name(pkg)
        if name is None:
            continue
        now, why_not = measure(loft, libs, pkg)
        if now is None:
            print(f"{name}: not measured — {why_not}")
            failed += 1
            continue
        for why in now.values():
            totals[why] = totals.get(why, 0) + 1
        was = base.get(name, {})
        lost = sorted(set(now) - set(was))
        won = sorted(set(was) - set(now))
        if update:
            if now:
                base[name] = now
            else:
                base.pop(name, None)
        state = "compiled whole" if not now else f"{len(now)} interpreted"
        print(f"{name}: {state}")
        if lost and not update:
            failed += 1
            for f in lost:
                print(f"    now interpreted, not in the baseline: {f} ({now[f]})")
        if won and not update:
            failed += 1
            for f in won:
                print(f"    now compiled, still in the baseline (write it down: --update): {f}")
        if strict and now:
            failed += 1
            for f in sorted(now):
                print(f"    interpreted: {f} ({now[f]})")
    if update:
        write_baseline(baseline, base)
        print(f"baseline written: {baseline}")
    print()
    for why, n in sorted(totals.items(), key=lambda kv: -kv[1]):
        print(f"interpreted  {n:5}  {why}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
