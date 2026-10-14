#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Regenerate every derived artefact that is a pure function of the tree.

Runs the `regen` commands of each `scripts/derived_artefacts.json` entry marked `"ci": true`,
in the registry's order (the compiled stdlib first: what follows measures the build it makes),
then that entry's `check`.  Prints one line per artefact and the files that moved.  A ratchet
(an entry with a `pin`, or one that needs a decision when it moves) never carries `ci`, so
nothing here can bless a count.

    python3 scripts/derive_ci.py           # regenerate; exit 1 if a regen or check failed
    python3 scripts/derive_ci.py --list    # the artefacts it would regenerate

Run by `.github/workflows/derive.yml`, which commits the result back to the branch; usable by
hand too.  JOINING.md § Derived artefacts on a runner.
"""

import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def sh(cmd):
    r = subprocess.run(cmd, shell=True, cwd=ROOT, capture_output=True, text=True)
    if r.returncode != 0:
        out = (r.stdout + r.stderr).strip().splitlines()
        print("\n".join("      " + l for l in out[-15:]))
    return r.returncode == 0


def moved():
    r = subprocess.run(["git", "status", "--porcelain"], cwd=ROOT, capture_output=True,
                       text=True, check=True)
    return {l[3:] for l in r.stdout.splitlines()}


def main():
    registry = json.loads((ROOT / "scripts/derived_artefacts.json").read_text(encoding="utf-8"))
    arts = [a for a in registry["artefacts"] if a.get("ci")]
    if "--list" in sys.argv[1:]:
        for a in arts:
            print(a["name"])
        return 0
    failed = []
    for a in arts:
        before = moved()
        ok = all(sh(cmd) for cmd in a["regen"])
        if ok and a.get("check"):
            ok = sh(a["check"])
            if not ok:
                print(f"  FAIL  {a['name']:24} still fails its check after regenerating")
        elif not ok:
            print(f"  FAIL  {a['name']:24} a regen command failed")
        if not ok:
            failed.append(a["name"])
            continue
        delta = moved() - before
        print(f"  ok    {a['name']:24} {f'{len(delta)} file(s) regenerated' if delta else 'unchanged'}")
    if failed:
        print(f"\n{len(failed)} artefact(s) could not be regenerated: {', '.join(failed)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
