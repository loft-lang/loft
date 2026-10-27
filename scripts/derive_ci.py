#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Regenerate every derived artefact that is a pure function of the tree.

Runs the `regen` commands of each `scripts/derived_artefacts.json` entry marked `"ci": true`,
in the registry's order (the compiled stdlib first: what follows measures the build it makes),
then that entry's `check`.  Prints one line per command as it ends, with its seconds, and one per
artefact with the files that moved; on a runner the per-artefact seconds also go to the job
summary, so a slow derive names the step that made it slow.  A ratchet
(an entry with a `pin`, or one that needs a decision when it moves) never carries `ci`, so
nothing here can bless a count.

    python3 scripts/derive_ci.py           # regenerate; exit 1 if a regen or check failed
    python3 scripts/derive_ci.py --list    # the artefacts it would regenerate

Run by `.github/workflows/derive.yml`, which commits the result back to the branch; usable by
hand too.  JOINING.md § Derived artefacts on a runner.
"""

import json
import os
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent


def sh(cmd):
    t0 = time.monotonic()
    r = subprocess.run(cmd, shell=True, cwd=ROOT, capture_output=True, text=True)
    print(f"        {time.monotonic() - t0:6.1f}s  {cmd[:110]}", flush=True)
    if r.returncode != 0:
        out = (r.stdout + r.stderr).strip().splitlines()
        print("\n".join("      " + l for l in out[-15:]), flush=True)
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
    times = []
    for a in arts:
        t0 = time.monotonic()
        before = moved()
        ok = all(sh(cmd) for cmd in a["regen"])
        if ok and a.get("check"):
            ok = sh(a["check"])
            if not ok:
                print(f"  FAIL  {a['name']:24} still fails its check after regenerating")
        elif not ok:
            print(f"  FAIL  {a['name']:24} a regen command failed")
        secs = time.monotonic() - t0
        if not ok:
            failed.append(a["name"])
            times.append((a["name"], secs, "FAILED"))
            continue
        delta = moved() - before
        what = f"{len(delta)} file(s) regenerated" if delta else "unchanged"
        times.append((a["name"], secs, what))
        print(f"  ok    {a['name']:24} {secs:6.1f}s  {what}", flush=True)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as fh:
            fh.write("| artefact | seconds | result |\n|---|---:|---|\n")
            fh.writelines(f"| {n} | {t:.0f} | {w} |\n" for n, t, w in times)
    if failed:
        print(f"\n{len(failed)} artefact(s) could not be regenerated: {', '.join(failed)}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
