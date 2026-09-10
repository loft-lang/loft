#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# @PLN112 phase 2 — build doc/claude/unreleased-snapshot.json: for every registry
# library, its `origin/main` public API (the `unreleased` tier), extracted the SAME way
# `loft api` does (`pkg_api_items`).  The committed snapshot makes the catalogue's
# unreleased tier deterministic + CI-checkable (the generator renders from it; no network
# at --check time).
#
# CONTENT-ADDRESSED CACHE (the "check cheap, reuse the rest" invariant): each lib is keyed
# by its `origin/main` sub-path commit sha.  A cheap one-line sha check per lib; if the sha
# is unchanged since the committed snapshot, the entry is REUSED unchanged — no fetch, no
# extract.  A matching sha is a PROOF the source is identical (not a guess), because git
# shas are content-addressed.  There is no local clone to go stale — every read is `gh`
# against the authoritative ref.
#
# Usage:  scripts/refresh-unreleased.py            # refresh all libs
#         scripts/refresh-unreleased.py <name...>  # only these libs (still writes all)
#         scripts/refresh-unreleased.py --force     # re-extract even when the sha matches
#                                                   #   (use when the EXTRACTOR changed)
from __future__ import annotations

import base64
import json
import os
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
INDEX = REPO / "doc" / "claude" / "registry-index-snapshot.json"
OUT = REPO / "doc" / "claude" / "unreleased-snapshot.json"
LOFT = REPO / "target" / "release" / "loft"
# The catalogue is a LOCAL build (not committed), so the registry index snapshot may
# not exist yet — fetch it live in that case (self-bootstrapping, no committed seed).
LIVE_INDEX_URL = "https://raw.githubusercontent.com/loft-lang/registry/main/index.json"


def gh(args: list[str]) -> str:
    r = subprocess.run(["gh", "api", *args], capture_output=True, text=True)
    return r.stdout if r.returncode == 0 else ""


def repo_subpath(homepage: str) -> tuple[str, str]:
    """github.com/<owner>/<repo>/tree/main/<subpath> -> (owner/repo, subpath)."""
    rest = homepage.removeprefix("https://github.com/")
    owner_repo = rest.split("/tree/", 1)[0]
    subpath = rest.split("/tree/main/", 1)[1] if "/tree/main/" in rest else ""
    return owner_repo, subpath


def origin_main_sha(owner_repo: str, subpath: str) -> str:
    """The sha of the last commit touching this lib's sub-path on the default branch."""
    return gh([f"repos/{owner_repo}/commits?path={subpath}&per_page=1", "--jq", ".[0].sha"]).strip()


def extract_api(owner_repo: str, subpath: str) -> list[dict]:
    """Fetch the lib's src/*.loft from origin/main and run `loft api --json` on it."""
    srcdir = f"{subpath}/src" if subpath else "src"
    names = [n for n in gh([f"repos/{owner_repo}/contents/{srcdir}", "--jq", ".[].name"]).splitlines() if n.endswith(".loft")]
    if not names:
        return []
    with tempfile.TemporaryDirectory() as td:
        os.makedirs(f"{td}/src", exist_ok=True)
        for n in names:
            raw = gh([f"repos/{owner_repo}/contents/{srcdir}/{n}", "-H", "Accept: application/vnd.github.raw"])
            Path(f"{td}/src/{n}").write_text(raw, encoding="utf-8")
        Path(f"{td}/loft.toml").write_text('name = "probe"\nversion = "0.0.0"\n', encoding="utf-8")
        r = subprocess.run([str(LOFT), "api", td, "--json"], capture_output=True, text=True)
        try:
            return json.loads(r.stdout)
        except json.JSONDecodeError:
            return []


def has_guide(owner_repo: str, subpath: str) -> bool:
    """Does this lib ship a `docs/*.loft` getting-started guide on `origin/main`?

    Tier 1 of the four documentation tiers (@PLN149): one executed guide per library,
    living in the library and run by its own CI.  Recorded here rather than read from the
    registry index because the index carries no guide field — the guide travels inside the
    tarball, so the only cheap authoritative source is the repo tree.

    A directory that does not exist answers an empty listing, which is a real `false`; a
    `gh` call that FAILS answers the same empty string, and the caller cannot tell those
    apart from here.  That is why the snapshot records the flag only when the listing
    succeeded, and why the reader treats a MISSING key as "not measured" rather than as
    "no guide" — the whole point of the third state.
    """
    docsdir = f"{subpath}/docs" if subpath else "docs"
    listing = gh([f"repos/{owner_repo}/contents/{docsdir}", "--jq", ".[].name"])
    return any(n.endswith(".loft") for n in listing.splitlines())


def main() -> int:
    argv = sys.argv[1:]
    # The sha keys the SOURCE, and the source is only half of what the snapshot depends on:
    # the other half is the EXTRACTOR.  When `loft api`'s reading of a doc comment changes,
    # every sha is still identical and every entry is reused, so the snapshot silently keeps
    # answers the current binary would not give.  That is how the library review went on
    # reporting 334 undocumented public functions after the reader was fixed.  `--force`
    # re-extracts regardless of sha; reach for it whenever the extractor moved, not the libs.
    force = "--force" in argv
    only = {a for a in argv if not a.startswith("--")}
    if INDEX.exists():
        index = json.loads(INDEX.read_text(encoding="utf-8"))
    else:  # local build with no committed seed — fetch the live registry index
        with urllib.request.urlopen(LIVE_INDEX_URL, timeout=30) as resp:
            index = json.loads(resp.read().decode("utf-8"))
    prior = json.loads(OUT.read_text(encoding="utf-8")) if OUT.exists() else {}
    result: dict[str, dict] = {}
    for name, pkg in sorted(index.get("packages", {}).items()):
        owner_repo, subpath = repo_subpath((pkg.get("homepage") or "").strip())
        if not owner_repo:
            continue
        if only and name not in only:
            if name in prior:  # keep others untouched when refreshing a subset
                result[name] = prior[name]
            continue
        sha = origin_main_sha(owner_repo, subpath)
        if not sha:
            continue
        if not force and prior.get(name, {}).get("sha") == sha:
            result[name] = dict(prior[name])  # not stale — reuse (no fetch, no extract)
            # An entry written before `guide` existed carries no such key, and a sha match
            # would keep it that way for ever — the second instance of the extractor-moved
            # hazard the `--force` note above describes.  But the guide flag does NOT come
            # from the extractor: it is one directory listing, independent of `loft api`.
            # So backfill just that, cheaply, rather than making the reader run a full
            # re-extract of 42 libraries to learn one boolean per library.
            if "guide" not in result[name]:
                result[name]["guide"] = has_guide(owner_repo, subpath)
                sys.stderr.write(
                    f"  {name}: reuse ({sha[:7]}, guide flag backfilled: "
                    f"{'guide' if result[name]['guide'] else 'NO guide'})\n")
            else:
                sys.stderr.write(f"  {name}: reuse ({sha[:7]})\n")
            continue
        api = extract_api(owner_repo, subpath)
        guide = has_guide(owner_repo, subpath)
        result[name] = {"sha": sha, "api": api, "guide": guide}
        sys.stderr.write(
            f"  {name}: fetched ({sha[:7]}, {len(api)} sigs, "
            f"{'guide' if guide else 'NO guide'})\n")
    OUT.write_text(json.dumps(result, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"refresh-unreleased: wrote {OUT} ({len(result)} libs)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
