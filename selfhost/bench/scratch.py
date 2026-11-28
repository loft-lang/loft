# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The temp directory a bench or cover run works in, and the sweep of the ones killed runs left.

A run builds tools and writes every program's image and output under one directory, a few GB
for the larger surveys.  `run_dir(prefix)` makes it under `$TMPDIR` and removes it when the
run exits; `LOFT_KEEP_TMP=1` keeps it for a look.  A run that is killed (a stopped task, a
full disk, a lost session) never reaches that exit, so each new run first removes the
directories with its own prefix that nothing has written to for `STALE_SECONDS`.  A run in
progress keeps its directory fresh, since it creates entries in it as it goes, so a sweep
from another checkout never touches it.
"""
import atexit
import os
import shutil
import tempfile
import time

STALE_SECONDS = 12 * 3600


def sweep(base, prefix):
    """Remove the directories under `base` named `prefix…` that are older than STALE_SECONDS."""
    now = time.time()
    try:
        names = os.listdir(base)
    except OSError:
        return
    for name in names:
        if not name.startswith(prefix):
            continue
        path = os.path.join(base, name)
        try:
            if os.path.isdir(path) and not os.path.islink(path) \
                    and now - os.stat(path).st_mtime > STALE_SECONDS:
                shutil.rmtree(path, ignore_errors=True)
        except OSError:
            pass


def run_dir(prefix):
    """A fresh directory for this run, removed at exit unless LOFT_KEEP_TMP=1."""
    base = tempfile.gettempdir()
    sweep(base, prefix)
    tmp = tempfile.mkdtemp(prefix=prefix, dir=base)
    if not os.environ.get("LOFT_KEEP_TMP"):
        atexit.register(shutil.rmtree, tmp, True)
    return tmp
