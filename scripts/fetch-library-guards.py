#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN175 — the design-decision guards that live in the LIBRARY repos.
#
# A decision about a library (C84: `server` stays transport primitives) is kept by a test in
# that library, where the library is edited, and the loft index never saw it: `idx decisions`
# counts citations in loft's own tree.  This reads every `loft-lang/loft-libs-*` repo at its
# `origin/main` — never a local clone, which can lag it (@PLN112) — and records each file
# under a `tests/` directory that cites `@C<n>`, boundary-exact, with its line and the commit
# it was read at.  The result is committed as index/library_guards.json and read offline by
# `scripts/idx decisions`, `scripts/idx tag:@C<n>` and tests/index_hygiene.rs, the way
# `features-fetch` feeds index/features.json.
#
# Cheap on a re-run: a repo whose `main` commit is the one recorded is reused unread.
#
# Usage:  scripts/fetch-library-guards.py           # refresh index/library_guards.json
#         scripts/fetch-library-guards.py --check   # exit 1 when the GUARDS differ from the
#                                                   #   committed file: one gone, one new, or a
#                                                   #   repo added/removed (network; no write).
#                                                   #   A commit that moved no guard is not stale.
from __future__ import annotations

import base64
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "index" / "library_guards.json"
ORG = "loft-lang"
PREFIX = "loft-libs-"
# `@C<n>` or the one dotted id form (`@C54.D`), boundary-exact: `@C3` is not `@C38`.
TAG = re.compile(r"@C(\d+(?:\.[A-Z])?)(?![\w.])")
GUARD_PATH = re.compile(r"(^|/)tests/")


def gh(args: list[str]) -> str:
    r = subprocess.run(["gh", "api", *args], capture_output=True, text=True)
    if r.returncode != 0:
        sys.exit(f"gh api {' '.join(args)} failed: {r.stderr.strip()}")
    return r.stdout


def library_repos() -> list[str]:
    names = json.loads(gh([f"orgs/{ORG}/repos?per_page=100", "--jq", "[.[] | select(.archived | not) | .name]"]))
    return sorted(n for n in names if n.startswith(PREFIX))


def main_commit(repo: str) -> str:
    return gh([f"repos/{ORG}/{repo}/commits/main", "--jq", ".sha"]).strip()


def scan(repo: str, commit: str) -> list[dict]:
    tree = json.loads(gh([f"repos/{ORG}/{repo}/git/trees/{commit}?recursive=1"]))
    files = []
    for entry in tree.get("tree", []):
        path = entry["path"]
        if entry["type"] != "blob" or not GUARD_PATH.search(path):
            continue
        if not path.endswith((".loft", ".rs", ".sh", ".py")):
            continue
        blob = json.loads(gh([f"repos/{ORG}/{repo}/git/blobs/{entry['sha']}"]))
        text = base64.b64decode(blob["content"]).decode("utf-8", errors="replace")
        cites = [
            {"tag": "@C" + m.group(1), "line": n}
            for n, line in enumerate(text.splitlines(), start=1)
            for m in TAG.finditer(line)
        ]
        if cites:
            files.append({"path": path, "cites": cites})
    return files


def guard_set(files: list[dict]) -> set[tuple[str, str]]:
    """What a repo's guards ARE: which file cites which decision (lines move freely)."""
    return {(f["path"], c["tag"]) for f in files for c in f["cites"]}


def main() -> int:
    check = "--check" in sys.argv[1:]
    old = json.loads(OUT.read_text()) if OUT.exists() else {"repos": {}}
    names = library_repos()
    repos = {}
    drift = []
    for repo in names:
        commit = main_commit(repo)
        prior = old.get("repos", {}).get(repo)
        if prior and prior.get("commit") == commit:
            repos[repo] = prior
            continue
        files = scan(repo, commit)
        repos[repo] = {"commit": commit, "files": files}
        was = guard_set(prior["files"]) if prior else set()
        now = guard_set(files)
        if prior is None:
            drift.append(f"{repo} (not recorded)")
        drift += [f"{repo}:{p} {t} (gone)" for p, t in sorted(was - now)]
        drift += [f"{repo}:{p} {t} (new)" for p, t in sorted(now - was)]
    drift += [f"{g} (repo gone)" for g in sorted(set(old.get("repos", {})) - set(names))]
    if check:
        if drift:
            print("index/library_guards.json no longer matches the libraries' main:")
            for d in drift:
                print(f"  {d}")
            print("Run: make guards-fetch")
            return 1
        return 0
    OUT.write_text(json.dumps({"schema": 1, "repos": repos}, indent=1, sort_keys=True) + "\n")
    n = sum(len(r["files"]) for r in repos.values())
    print(f"index/library_guards.json: {n} guard file(s) across {len(repos)} library repo(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
