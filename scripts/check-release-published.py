#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Refuse to start a new version while the previous release never reached the registry.

@PLN78.  Publishing a release is two halves: GitHub gets the binaries, and the signed
registry index gets the entry that makes them findable and verifiable.  Only the first
half fails loudly.  If the registry PR is forgotten, or its CI stays red, or it is
merged without a re-sign, everything keeps working -- `loft self-update` simply reports
"no releases published to compare against" forever, and `loft verify-self` can only say
a bundle is intact.  Nobody is paged by a feature quietly not existing; the usual way it
surfaces is a user asking why an update never arrived.

So the drift is caught at the one moment it is cheap to fix and expensive to skip: the
release-prep PR that bumps `Cargo.toml`.  Shipping N+1 while N never reached the
registry is how a gap becomes permanent -- N's assets are frozen at publish, so a
release that missed its entry can often never get a correct one afterwards.

**Only that PR is gated.**  A check that went red on every PR for the window between
"publish" and "registry PR merged" would be red for hours after each release, across
work that has nothing to do with it -- which teaches everyone to merge past a red
check, the opposite of what a gate is for.

Exits 0 when there is nothing to gate or the previous release is complete; 1 with a
GitHub-annotated error otherwise.
"""

import argparse
import json
import os
import re
import subprocess
import sys
import urllib.request

REPO = "loft-lang/loft"
INDEX_URL = os.environ.get(
    "LOFT_REGISTRY_INDEX",
    "https://raw.githubusercontent.com/loft-lang/registry/main/index.json",
)
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def fail(title: str, body: str) -> None:
    print(f"::error title={title}::{body}")
    sys.exit(1)


def cargo_version() -> str:
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as f:
        for line in f:
            m = re.match(r'^version\s*=\s*"([^"]+)"', line)
            if m:
                return m.group(1)
    fail("No version", "Cargo.toml has no top-level `version`.")
    raise AssertionError("unreachable")


def published_triples() -> list[str]:
    """Read from `self_update::PUBLISHED_TRIPLES` -- see gen-toolchain-entry.py."""
    src = open(
        os.path.join(ROOT, "src", "self_update.rs"), encoding="utf-8"
    ).read()
    m = re.search(r"PUBLISHED_TRIPLES: &\[&str\] = &\[(.*?)\];", src, re.S)
    if not m:
        fail("No PUBLISHED_TRIPLES", "src/self_update.rs: cannot find PUBLISHED_TRIPLES")
    return re.findall(r'"([^"]+)"', m.group(1))


def gh_json(path: str):
    """Query the GitHub API through `gh`, which CI already authenticates."""
    out = subprocess.run(
        ["gh", "api", path], capture_output=True, text=True, check=False
    )
    if out.returncode != 0:
        fail("GitHub API", f"`gh api {path}` failed: {out.stderr.strip()}")
    return json.loads(out.stdout)


def main() -> None:
    # The version the tree is MOVING TO.  Normally that is Cargo.toml's, because the
    # gate runs on the PR that bumps it.  `--version` lets the release checklist ask the
    # same question BEFORE the bump -- otherwise "has the previous release reached the
    # index?" reads as answered-and-fine during the whole window in which it is neither,
    # since the tree still carries the released version and the gate says "nothing to
    # gate".  One question, one implementation; only the moment it is asked differs.
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--version",
        help="the version being prepared (default: Cargo.toml's)",
    )
    version = ap.parse_args().version or cargo_version()

    releases = [r for r in gh_json(f"repos/{REPO}/releases?per_page=20") if not r["draft"]]
    if not releases:
        print("No published release yet — nothing to gate.")
        return
    latest = releases[0]
    tag = latest["tag_name"].lstrip("v")

    if tag == version:
        # Not a release-prep PR: the tree is still on the released version.
        print(f"Cargo.toml is {version}, the current release — nothing to gate.")
        return

    assets = {a["name"] for a in latest.get("assets", [])}
    if f"loft-{tag}-src.zip" not in assets:
        # Derived, not a hardcoded floor: a release with no source archive predates the
        # toolchain entry and can never gain one (published assets are immutable).  As
        # soon as release.yml attaches a source zip, every later release is covered
        # automatically -- there is no constant for anyone to forget to bump.
        print(
            f"v{tag} predates the toolchain registry entry (no source archive) — "
            f"nothing to gate."
        )
        return

    with urllib.request.urlopen(INDEX_URL, timeout=60) as resp:
        index = json.loads(resp.read())

    pkg = index.get("packages", {}).get("loft")
    fix = (
        f"Publish v{tag} to the registry before starting {version}: take "
        f"loft-{tag}-registry-entry.json from the release and\n"
        f"  scripts/gen-toolchain-entry.py --version {tag} --dir <assets> "
        f"--splice-into <registry>/index.json\n"
        f"then open the PR (doc/claude/REGISTRY_SUBMIT.md § The toolchain entry)."
    )
    if pkg is None:
        fail("Registry has no toolchain entry", f"The index carries no `loft` package. {fix}")
    ver = pkg.get("versions", {}).get(tag)
    if ver is None:
        fail(
            f"v{tag} is not in the registry",
            f"The last release was published to GitHub but never reached the signed "
            f"index, so `loft self-update` cannot see it. {fix}",
        )

    # Present is not the same as usable: an entry missing a platform, or missing the
    # anchor, is a release that half-exists -- which is harder to notice than one that
    # is absent, because `self-update` reports success for whoever it does cover.
    # The triples THAT release built — its own `loft-<v>-<triple>.zip` assets — not today's
    # PUBLISHED_TRIPLES: the list changes between releases (2026.11 moved Linux from musl to
    # glibc), and a release cannot be asked for a platform it never shipped.  Without
    # per-triple assets (an older layout) the current list stands in.
    built = sorted(
        a[len(f"loft-{tag}-") : -len(".zip")]
        for a in assets
        if a.startswith(f"loft-{tag}-") and a.endswith(".zip") and a != f"loft-{tag}-src.zip"
    )
    expected = built or published_triples()
    missing = [t for t in expected if t not in ver.get("binaries", {})]
    if missing:
        fail(
            f"v{tag}'s registry entry is incomplete",
            f"No binary for {', '.join(missing)} — users on those platforms are told "
            f"the release was not built for them. {fix}",
        )
    unanchored = [
        t for t, b in ver.get("binaries", {}).items() if not b.get("manifest_sha256")
    ]
    if unanchored:
        fail(
            f"v{tag}'s registry entry is not anchored",
            f"No manifest_sha256 for {', '.join(sorted(unanchored))} — `loft "
            f"verify-self` cannot trace those installations back to the signature. {fix}",
        )

    print(f"v{tag} is fully published: entry present, {len(ver['binaries'])} anchored binaries.")


if __name__ == "__main__":
    main()
