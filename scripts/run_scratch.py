# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""run_scratch.py — one run's scratch directory, kept after the run, removed by the next.

    from run_scratch import run_root
    root = run_root("c-sweep")     # <temp>/loft-c-sweep-<checkout>/<pid>/, created empty

What a run leaves stays for inspection — the C of a slow program, a profile's perf.data — and
the tool says where.  The next run of the same tool removes it first: every run starts by
removing the directories under its root whose run is no longer alive, finished or killed
alike.  A directory is alive while its pid runs AND that process started when the
directory's `.owner` says (a reused pid is a different process), so a concurrent run's is
kept.  The root is per checkout, so a sibling checkout's is never touched; RUN_BOUNDS.md §
Scratch hygiene.
"""
import os
import shutil
import tempfile
import zlib

_CHECKOUT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def _started(pid: int) -> str:
    """The process's start time in clock ticks since boot, or "" when it does not run."""
    try:
        with open(f"/proc/{pid}/stat", encoding="utf-8") as f:
            # The command name (field 2) may hold spaces; the fields after its `)` are fixed.
            return f.read().rsplit(")", 1)[1].split()[19]
    except (OSError, IndexError):
        return ""


def _alive(d: str) -> bool:
    try:
        pid = int(os.path.basename(d))
        with open(os.path.join(d, ".owner"), encoding="utf-8") as f:
            stamp = f.read().strip()
    except (ValueError, OSError):
        return False
    return _started(pid) == stamp and stamp != ""


def run_root(tool: str) -> str:
    """This run's empty scratch directory under the checkout's root for `tool`."""
    tag = f"{os.path.basename(_CHECKOUT)}.{zlib.crc32(_CHECKOUT.encode()):08x}"
    root = os.path.join(tempfile.gettempdir(), f"loft-{tool}-{tag}")
    os.makedirs(root, exist_ok=True)
    for name in os.listdir(root):
        d = os.path.join(root, name)
        if os.path.isdir(d) and name != str(os.getpid()) and not _alive(d):
            shutil.rmtree(d, ignore_errors=True)
    mine = os.path.join(root, str(os.getpid()))
    shutil.rmtree(mine, ignore_errors=True)
    os.makedirs(mine)
    with open(os.path.join(mine, ".owner"), "w", encoding="utf-8") as f:
        f.write(_started(os.getpid()))
    return mine
