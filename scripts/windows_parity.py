#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN184 E2 — where Windows parity stands, every count in one report.

Each count is held by a census that only shrinks (the tests that read these baselines fail
on a rise); this report prints them together with every exemption and its reason, so the
release checklist row (`M-windows-parity`) reads one page.  `make windows-parity`.

Exit status: 0 always — a REPORT.  `--check` exits 1 while any count is above zero that is
not an exemption, the closing condition of @PLN184.
"""

import argparse
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def read(rel):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        return f.read()


def baseline(rel):
    """`<count> <path>` rows of a census baseline."""
    rows = []
    for line in read(rel).splitlines():
        if line.startswith("#") or not line.strip():
            continue
        n, path = line.split(" ", 1)
        rows.append((int(n), path))
    return rows


def opt_outs():
    """The files that opt out of the file-access lints, each with its stated reason."""
    out = []
    for line in read("src/file_access/clippy_allow.baseline").splitlines():
        if line.startswith("#") or not line.strip():
            continue
        text = read(line)
        reason = ""
        for l in text.splitlines():
            if "@PLN184 A" in l and ("exemption" in l or "not yet" in l):
                reason = l.strip().lstrip("/ ").strip()
                break
        out.append((line, reason))
    return out


def marked(pattern, roots, exts):
    """Every `pattern` comment under `roots`: (file, line, text)."""
    hits = []
    for root in roots:
        base = os.path.join(ROOT, root)
        for dirpath, dirnames, files in os.walk(base):
            dirnames[:] = [d for d in dirnames if d not in ("fixtures", "target", ".git")]
            for name in sorted(files):
                if not name.endswith(exts):
                    continue
                path = os.path.join(dirpath, name)
                rel = os.path.relpath(path, ROOT)
                with open(path, encoding="utf-8", errors="replace") as f:
                    for i, l in enumerate(f, 1):
                        m = re.search(pattern, l)
                        if m:
                            hits.append((rel, i, m.group(1).strip()))
    return hits


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--check", action="store_true", help="exit 1 while a count is not yet 0")
    args = ap.parse_args()

    gates = baseline("src/platform_census.baseline")
    src_gates = [(n, p) for n, p in gates if p.startswith("src/")]
    test_gates = [(n, p) for n, p in gates if p.startswith("tests/")]
    scripts = baseline("src/platform_census_scripts.baseline")
    ci = baseline("src/platform_census_ci.baseline")
    direct = baseline("src/file_access/direct.baseline")
    outs = opt_outs()
    migrating = [o for o in outs if "not yet" in o[1]]
    ci_exempt = marked(r"# @windows-exempt: (.*)", [".github/workflows"], (".yml",))
    approved = marked(r"approved exemption \(owner[^)]*\):? (.*)", ["src", "tests"], (".rs",))

    def total(rows):
        return sum(n for n, _ in rows)

    print("Windows parity (@PLN184) — every count, then every exemption\n")
    rows = [
        ("A  files opting out of the file-access lints, still migrating", len(migrating)),
        ("B  platform gates at a call site in src/", total(src_gates)),
        ("C  platform gates in tests/ (compiled out or skipped on Windows)", total(test_gates)),
        ("E  CI jobs with neither a Windows leg nor a stated exemption", total(ci)),
    ]
    for label, n in rows:
        print(f"  {n:6}  {label}")
    print(f"\n  report (not part of the close): {total(direct):5}  direct-access text matches "
          "(the ratchet for code Clippy cannot see)")
    print(f"  report (ports are @PLN179's):   {total(scripts):5}  Windows hazards in "
          f"{len(scripts)} scripts")
    smoke = [l for l in read("src/platform_census_smoke.baseline").splitlines()
             if l.strip() and not l.startswith("#")]
    print(f"  report (ports are @PLN179's):   {int(smoke[0]) if smoke else 0:5}  scripts with no "
          "Windows smoke invocation (scripts/windows_smoke.tsv)")

    print("\nApproved and candidate exemptions:")
    for path, reason in outs:
        if "not yet" not in reason:
            print(f"  lint opt-out  {path}: {reason or '(no reason stated)'}")
    for path, line, text in approved:
        print(f"  approved      {path}:{line}: {text}")
    kinds = {}
    for path, _, text in ci_exempt:
        key = re.split(r" \(| — |;", text)[0].strip()
        kinds.setdefault(key, []).append(os.path.basename(path))
    for key, files in sorted(kinds.items(), key=lambda kv: -len(kv[1])):
        print(f"  CI ({len(files):2} jobs)  {key}")

    # A gate file still counted is settled when it states an approved exemption; one with
    # gates and no stated reason is open.
    exempt_files = {path for path, _, _ in approved}
    unexplained = [(n, p) for n, p in src_gates + test_gates if p not in exempt_files]
    print(f"\n  {total(unexplained):6}  gates in files that state no approved exemption"
          + "".join(f"\n          {n:4} {p}" for n, p in unexplained))
    open_counts = len(migrating) + total(unexplained) + total(ci)
    if args.check and open_counts:
        print(f"\n{open_counts} still open — @PLN184 closes when every count is 0 or an approved exemption", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
