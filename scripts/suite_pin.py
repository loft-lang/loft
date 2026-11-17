#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""A checkout of the language suite at the commit this tree pins (tests/loft-suite.pin).

Suite-backed checks — the receipts, index hygiene's decision guards, rule coverage — read the
cells of whatever `LOFT_SUITE_DIR` (default `../loft-suite`) holds.  A sibling checkout on
another commit makes a guard that lives only in the pinned suite invisible: index hygiene then
reports a decision with no guard although the pin carries one.  `ensure()` answers a directory
at the pin and exports it as LOFT_SUITE_DIR for the commands that follow:

  * an explicit LOFT_SUITE_DIR is kept as it is (a curator points it at a suite AHEAD of the
    pin on purpose); a note says when it is not at the pin;
  * else `../loft-suite` when its HEAD is the pin;
  * else a detached worktree at the pin, cached in .git/loft-suite-pin and re-pointed on each
    call — `../loft-suite`'s own checkout is never moved (it may be someone's work).  The pin is
    fetched into `../loft-suite` when it lacks it, or the suite is cloned when there is none.

  scripts/suite_pin.py          # print the directory (creating the worktree if needed)
"""
import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REMOTE = "https://github.com/loft-lang/loft-suite.git"


def _git(*args, cwd=None):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)


def _head(d):
    return _git("-C", str(d), "rev-parse", "HEAD").stdout.strip()


def pin():
    return (ROOT / "tests" / "loft-suite.pin").read_text(encoding="utf-8").strip()


def ensure(quiet=False):
    want = pin()
    explicit = os.environ.get("LOFT_SUITE_DIR")
    if explicit:
        d = Path(explicit) if Path(explicit).is_absolute() else ROOT / explicit
        if _head(d) != want and not quiet:
            print(f"  note  LOFT_SUITE_DIR={d} is {_head(d)[:9] or 'missing'}, the pin is {want[:9]}: "
                  "kept, as it was set on purpose", file=sys.stderr)
        return str(d)
    sibling = ROOT.parent / "loft-suite"
    if _head(sibling) == want:
        os.environ["LOFT_SUITE_DIR"] = str(sibling)
        return str(sibling)
    if not (sibling / ".git").exists():
        r = _git("clone", "-q", "--no-checkout", REMOTE, str(sibling))
        if r.returncode != 0:
            sys.exit(f"suite_pin: cannot clone {REMOTE}: {r.stderr.strip()}")
    if _git("-C", str(sibling), "cat-file", "-e", f"{want}^{{commit}}").returncode != 0:
        _git("-C", str(sibling), "fetch", "-q", "origin")
    cache = ROOT / ".git" / "loft-suite-pin"
    if not (cache / ".git").exists():
        _git("-C", str(sibling), "worktree", "prune")
        r = _git("-C", str(sibling), "worktree", "add", "-q", "--detach", str(cache), want)
    else:
        r = _git("-C", str(cache), "checkout", "-q", "--detach", want)
    if r.returncode != 0 or _head(cache) != want:
        sys.exit(f"suite_pin: no checkout of the pin {want[:9]}: {r.stderr.strip()}")
    os.environ["LOFT_SUITE_DIR"] = str(cache)
    if not quiet:
        print(f"  note  {sibling} is {_head(sibling)[:9]}; the checks read the pin {want[:9]} "
              f"from {cache}", file=sys.stderr)
    return str(cache)


if __name__ == "__main__":
    print(ensure())
