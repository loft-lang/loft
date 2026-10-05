#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Re-score the falsification patch receipts a change reaches — on a runner, once per PR.

A patch receipt (`tests/falsified/<guard>.patch`, GUARDS.md § The patch receipt) reintroduces
its guard's defect on top of HEAD.  Scoring one builds a whole control tree, so it runs here,
on `.github/workflows/receipts.yml`, instead of on every join.  A receipt is in reach when
its patch no longer applies to this tree: the code under the defect moved.  A receipt whose
patch or guard CHANGED was scored by its author when it was written (a new guard lands with
its receipt), so it is in reach only with `--changed` (a by-hand run).  A receipt stale since
before the apply check landed (`tests/falsified_patches.baseline`) is known and skipped.

A stale patch is first REFRESHED in a scratch worktree — a three-way merge, else an apply that
needs one line of context — and rewritten from the result.  A patch whose own lines changed
cannot be merged and needs a hand re-derivation.  Every receipt in reach is then scored with
`scripts/falsify.sh <guard> --patch <file>`.

Only a WILD SWING fails: a guard the patch no longer falsifies (falsify.sh exits non-zero —
INERT, an unclean tree, a patch that does not apply), or a stale patch that cannot be
refreshed.  A guard that still falsifies but moves a different channel than its recorded line
is reported, not failed: the channel is the reader's to re-record.

    python3 scripts/receipts_ci.py                   # refresh + score every stale receipt
    python3 scripts/receipts_ci.py --list            # only say what is in reach
    python3 scripts/receipts_ci.py --changed --base origin/main   # also what changed since BASE
"""

import argparse
import os
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
FALSIFIED = ROOT / "tests/falsified"


def git(*args, cwd=ROOT, check=True):
    r = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if check and r.returncode != 0:
        sys.exit(f"git {' '.join(args)}: {r.stderr.strip()}")
    return r


def applies(patch, cwd=ROOT):
    return git("apply", "--check", str(patch), cwd=cwd, check=False).returncode == 0


def citing_guards():
    """patch file name -> the guards whose `@falsified-by:` names it."""
    out = {}
    r = git("grep", "-l", "@falsified-by: tests/falsified/", "--", "tests/", check=False)
    for g in sorted(set(r.stdout.split())):
        if not g.endswith(".loft"):
            continue
        text = (ROOT / g).read_text(encoding="utf-8", errors="replace")
        for name in re.findall(r"@falsified-by: tests/falsified/([^\s]+\.patch)", text):
            out.setdefault(name, []).append(g)
    return out


def recorded(guard, name):
    """The channel text a guard records for patch `name` (its `@falsified-by:` line)."""
    text = (ROOT / guard).read_text(encoding="utf-8", errors="replace")
    m = re.search(r"@falsified-by: tests/falsified/" + re.escape(name) + r" — ([^\n]*)", text)
    return m.group(1).strip() if m else ""


def refresh(patch):
    """Rewrite a stale patch from a clean three-way merge onto HEAD; False when it conflicts."""
    with tempfile.TemporaryDirectory(prefix="loft-receipt-") as td:
        wt = pathlib.Path(td) / "wt"
        git("worktree", "add", "--detach", str(wt), "HEAD")
        try:
            # A three-way merge needs the patch's recorded pre-image blob, which a hand
            # re-derived patch's `index` line need not name; then one line of context is
            # enough — every removed line of the defect must still match exactly, and the
            # score that follows fails a patch that lands somewhere it does not belong.
            for how in (["--3way"], ["-C1"]):
                if git("apply", *how, str(patch), cwd=wt, check=False).returncode == 0:
                    break
                git("checkout", "--", ".", cwd=wt, check=False)
            else:
                return False
            diff = git("diff", "HEAD", "--", "src/", "default/", cwd=wt).stdout
            if not diff.strip():
                return False
            patch.write_text(diff, encoding="utf-8")
            return True
        finally:
            git("worktree", "remove", "--force", str(wt), check=False)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--list", action="store_true")
    ap.add_argument("--changed", action="store_true",
                    help="also score receipts whose patch or guard changed since --base")
    args = ap.parse_args()

    base = git("merge-base", args.base, "HEAD").stdout.strip()
    changed = set(git("diff", "--name-only", base, "HEAD").stdout.split())
    cites = citing_guards()
    # Stale already when the apply check landed (shrink-only, GUARDS.md): known, not in reach.
    known = {l.strip().removeprefix("tests/falsified/")
             for l in (ROOT / "tests/falsified_patches.baseline").read_text().splitlines()
             if l.strip() and not l.startswith("#")}

    reach, stale = [], []
    for patch in sorted(FALSIFIED.glob("*.patch")):
        name = patch.name
        guards = cites.get(name, [])
        moved = args.changed and (
            f"tests/falsified/{name}" in changed or any(g in changed for g in guards))
        if name in known:
            continue
        ok = applies(patch)
        if not ok:
            stale.append(patch)
        if (moved or not ok) and guards:
            reach.append(patch)

    if args.list:
        for p in reach:
            print(f"{p.relative_to(ROOT)}  {'STALE' if p in stale else 'changed'}  "
                  f"{', '.join(cites[p.name])}")
        return 0

    lines, swings, drift, refreshed = [], [], [], []
    for patch in reach:
        name = patch.name
        if patch in stale:
            if refresh(patch):
                refreshed.append(name)
            else:
                swings.append(f"{name}: no longer applies and a three-way merge conflicts — "
                              "re-derive it by hand (GUARDS.md § The patch receipt)")
                continue
        for guard in cites[name]:
            r = subprocess.run(["scripts/falsify.sh", guard, "--patch", str(patch)], cwd=ROOT,
                               capture_output=True, text=True)
            got = re.search(r"// @falsified-by: \S+ — (.*)", r.stdout)
            got = got.group(1).strip() if got else ""
            if r.returncode != 0:
                tail = "\n".join((r.stdout + r.stderr).strip().splitlines()[-6:])
                swings.append(f"{name} no longer falsifies {guard} (falsify.sh exit "
                              f"{r.returncode}):\n{tail}")
                lines.append(f"| `{guard}` | `{name}` | **SWING** |")
                continue
            was = recorded(guard, name)
            same = was and (was == got or was.startswith("the") or got.startswith(was[:40]))
            if not same:
                drift.append(f"{guard}: recorded `{was}`, now `{got}`")
            lines.append(f"| `{guard}` | `{name}` | {'falsified' if same else 'falsified, channels moved'} |")

    report = [f"## Patch receipts in reach since `{base[:9]}`: {len(reach)}", ""]
    if lines:
        report += ["| guard | patch | verdict |", "|---|---|---|", *lines, ""]
    if refreshed:
        report += ["Refreshed by a clean three-way merge: " + ", ".join(f"`{n}`" for n in refreshed), ""]
    if drift:
        report += ["Channels moved (re-record the line; not a failure):", *[f"- {d}" for d in drift], ""]
    if swings:
        report += ["**Wild swings — these fail:**", *[f"- {s}" for s in swings], ""]
    text = "\n".join(report)
    print(text)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as f:
            f.write(text + "\n")
    return 1 if swings else 0


if __name__ == "__main__":
    sys.exit(main())
